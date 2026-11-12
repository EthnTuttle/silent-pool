//! Bounded loopback HTTP command API and SSE evidence stream.
//!
//! The browser is an observer. A disconnected or slow browser cannot stop the
//! run: publication uses a bounded per-subscriber queue and a full queue
//! records a gap instead of blocking the producer.
//!
//! Bounding is enforced before buffering: the request line and the whole
//! header block together are read through a 16 KiB byte budget (a line that
//! does not terminate within the budget is rejected without ever allocating
//! past it), the body is capped at 256 KiB and is only read after the header
//! bound has passed, and every read sits under the socket deadline.
//!
//! Ordinary evidence events are emitted with the generic SSE event type
//! `message` (the specific kind stays in the JSON payload), so an
//! `EventSource` client handles every evidence kind, including kinds added
//! later, with a single listener. `resync` remains a named event.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;

use crate::command::LabCommand;
use crate::engine::RunEngine;

const MAX_CONNECTIONS: usize = 64;
const MAX_HEADER_BYTES: usize = 16 * 1024;
const MAX_BODY_BYTES: usize = 256 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(5);
const ACCESS_JS: &str = include_str!("../../../bins/spc-lab/static/access.js");

#[derive(Clone, Copy)]
pub struct WebAssets {
    pub index_html: &'static str,
    pub app_js: &'static str,
    pub style_css: &'static str,
    pub lab_html: &'static str,
    pub lab_js: &'static str,
    pub lab_css: &'static str,
}

impl WebAssets {
    /// A minimal fallback for tests and embedders that supply their own UI.
    pub const fn fallback() -> Self {
        Self {
            index_html:
                "<!doctype html><title>spc-lab</title><body><h1>spc-lab</h1><p>API only</p></body>",
            app_js: "/* no embedded app */",
            style_css: "body{font-family:system-ui;margin:2rem}",
            lab_html:
                "<!doctype html><title>spc-lab roles</title><body><h1>spc-lab roles</h1><p>Inventory API only</p><a href='/'>Open live controls</a></body>",
            lab_js: "/* no embedded hub */",
            lab_css: "body{font-family:system-ui;margin:2rem}",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    #[error("HTTP listener must bind loopback, got {0}")]
    NotLoopback(String),
    #[error("HTTP io: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug)]
pub struct HttpServer {
    addr: SocketAddr,
    shutdown: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl HttpServer {
    pub fn start(
        engine: Arc<RunEngine>,
        bind: impl AsRef<str>,
        assets: WebAssets,
    ) -> Result<Self, HttpError> {
        let bind = bind.as_ref();
        let addrs: Vec<SocketAddr> = bind.to_socket_addrs().map_err(HttpError::Io)?.collect();
        let addr = *addrs.first().ok_or_else(|| {
            HttpError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "bind address resolved to nothing",
            ))
        })?;
        if !addr.ip().is_loopback() {
            return Err(HttpError::NotLoopback(bind.to_string()));
        }
        let listener = TcpListener::bind(addr)?;
        listener.set_nonblocking(false)?;
        let addr = listener.local_addr()?;
        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_thread = Arc::clone(&shutdown);
        let thread = std::thread::Builder::new()
            .name("spc-lab-http".to_string())
            .spawn(move || accept_loop(listener, engine, assets, shutdown_thread))
            .map_err(HttpError::Io)?;
        Ok(Self {
            addr,
            shutdown,
            thread: Some(thread),
        })
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn shutdown(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        // Wake the blocking accept by connecting once.
        let _ = TcpStream::connect_timeout(&self.addr, Duration::from_millis(100));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for HttpServer {
    fn drop(&mut self) {
        self.stop();
    }
}

fn accept_loop(
    listener: TcpListener,
    engine: Arc<RunEngine>,
    assets: WebAssets,
    shutdown: Arc<AtomicBool>,
) {
    let active = Arc::new(AtomicUsize::new(0));
    for stream in listener.incoming() {
        if shutdown.load(Ordering::SeqCst) {
            break;
        }
        let Ok(stream) = stream else { continue };
        let count = active.fetch_add(1, Ordering::SeqCst) + 1;
        if count > MAX_CONNECTIONS {
            active.fetch_sub(1, Ordering::SeqCst);
            let _ = respond_status(stream, 503, "too many connections");
            continue;
        }
        let engine = Arc::clone(&engine);
        let shutdown = Arc::clone(&shutdown);
        let active_thread = Arc::clone(&active);
        let _ = std::thread::Builder::new()
            .name("spc-lab-http-client".to_string())
            .spawn(move || {
                let _ = handle_client(stream, engine, assets, shutdown);
                active_thread.fetch_sub(1, Ordering::SeqCst);
            });
    }
}

fn handle_client(
    stream: TcpStream,
    engine: Arc<RunEngine>,
    assets: WebAssets,
    shutdown: Arc<AtomicBool>,
) -> std::io::Result<()> {
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    let request = match read_request(stream.try_clone()?) {
        Ok(request) => request,
        Err(error) => {
            return respond_status(stream, error.status, &error.message);
        }
    };
    let (path, query) = split_query(&request.path);
    let route = route(engine.run_id().as_str(), &path);
    // The expanded-access profile (scenario `expanded_access: true`) takes
    // a separate request path: claims, Bearer authentication, scope and the
    // filtered read projection. A legacy run never enters it.
    if engine.access().is_some() {
        return handle_expanded(stream, &engine, assets, request, query, route, &shutdown);
    }
    match (request.method.as_str(), route) {
        ("GET", Route::AccessJs) => respond_bytes(
            stream,
            "text/javascript; charset=utf-8",
            ACCESS_JS.as_bytes(),
        ),
        ("GET", Route::Index) => respond_bytes(
            stream,
            "text/html; charset=utf-8",
            assets.index_html.as_bytes(),
        ),
        ("GET", Route::AppJs) => respond_bytes(
            stream,
            "text/javascript; charset=utf-8",
            assets.app_js.as_bytes(),
        ),
        ("GET", Route::StyleCss) => respond_bytes(
            stream,
            "text/css; charset=utf-8",
            assets.style_css.as_bytes(),
        ),
        ("GET", Route::Lab) => respond_bytes(
            stream,
            "text/html; charset=utf-8",
            assets.lab_html.as_bytes(),
        ),
        ("GET", Route::LabJs) => respond_bytes(
            stream,
            "text/javascript; charset=utf-8",
            assets.lab_js.as_bytes(),
        ),
        ("GET", Route::LabCss) => {
            respond_bytes(stream, "text/css; charset=utf-8", assets.lab_css.as_bytes())
        }
        ("GET", Route::Favicon) => respond_status(stream, 204, ""),
        ("GET", Route::Health) => respond_json(
            stream,
            &serde_json::json!({"ok": true, "run_id": engine.run_id()}),
        ),
        ("GET", Route::Runs) => {
            respond_json(stream, &serde_json::json!([{"run_id": engine.run_id()}]))
        }
        ("GET", Route::Snapshot) => {
            // A cursor that does not parse is a client bug: reject it instead
            // of silently restarting from zero (which would replay duplicates).
            let cursor = match query.get("cursor") {
                Some(raw) => match parse_cursor(raw) {
                    Ok(cursor) => Some(cursor),
                    Err(message) => return respond_status(stream, 400, &message),
                },
                None => None,
            };
            respond_json(stream, &engine.snapshot(cursor))
        }
        ("POST", Route::Commands) => {
            if request.body.len() > MAX_BODY_BYTES {
                return respond_status(stream, 413, "request body too large");
            }
            let command: LabCommand = match serde_json::from_slice(&request.body) {
                Ok(command) => command,
                Err(error) => {
                    return respond_status(stream, 400, &format!("invalid command: {error}"))
                }
            };
            let outcome = engine.submit(command);
            respond_json(stream, &outcome)
        }
        ("GET", Route::Catalog) => respond_json(stream, &engine.runtime_catalog()),
        ("GET", Route::Inventory) => respond_json(stream, &engine.lab_inventory()),
        ("GET", Route::Events) => {
            // The SSE-standards reconnect path is Last-Event-ID: on a browser
            // reconnect it carries the id the client actually saw last, so it
            // takes precedence over the stale `after` in the original URL and
            // avoids duplicate replay.
            let after = match resolve_events_cursor(&request.headers, &query) {
                Ok(after) => after,
                Err(message) => return respond_status(stream, 400, &message),
            };
            serve_events(stream, &engine, after, &shutdown, None)
        }
        ("GET", Route::Artifact(path)) => serve_artifact(stream, &engine, &path),
        _ => respond_status(stream, 404, "not found"),
    }
}

/// The expanded-access request path (live-lab plan section 5).
///
/// This is a first foundation, not a production auth platform. Every route
/// requires `Authorization: Bearer <token>` with exactly one exception: the
/// closed public static bootstrap list - the controller's workbench
/// HTML/CSS/JS and the relayed role bootstrap assets, which contain no
/// private facts, no token and no status, so address-bar navigation works
/// without an authentication dead end. An unknown or out-of-scope path is a
/// closed 404/403; the static exception is a closed list, never a wildcard.
/// Failures never echo private detail.
#[allow(clippy::too_many_arguments)]
fn handle_expanded(
    stream: TcpStream,
    engine: &Arc<RunEngine>,
    assets: WebAssets,
    request: Request,
    query: std::collections::BTreeMap<String, String>,
    route: Route,
    shutdown: &Arc<AtomicBool>,
) -> std::io::Result<()> {
    let method = request.method.clone();
    // The closed public static bootstrap exception.
    if method == "GET" {
        match &route {
            Route::AccessJs => {
                return respond_static(
                    stream,
                    "text/javascript; charset=utf-8",
                    ACCESS_JS.as_bytes(),
                    false,
                )
            }
            Route::Index => {
                return respond_static(
                    stream,
                    "text/html; charset=utf-8",
                    assets.index_html.as_bytes(),
                    true,
                );
            }
            Route::AppJs => {
                return respond_static(
                    stream,
                    "text/javascript; charset=utf-8",
                    assets.app_js.as_bytes(),
                    false,
                );
            }
            Route::StyleCss => {
                return respond_static(
                    stream,
                    "text/css; charset=utf-8",
                    assets.style_css.as_bytes(),
                    false,
                );
            }
            // R4 merge condition: the hub lab pages join the closed
            // pre-auth static bootstrap list. Byte-verified gate: the
            // embedded assets are byte-exact to the Hub-approved set and
            // carry no inline private fact, status, address or token
            // (EXP-ACCESS-lab-asset-bytecheck.log). They render recorded
            // run data only through the authenticated inventory DTO.
            Route::Lab => {
                return respond_static(
                    stream,
                    "text/html; charset=utf-8",
                    assets.lab_html.as_bytes(),
                    true,
                );
            }
            Route::LabJs => {
                return respond_static(
                    stream,
                    "text/javascript; charset=utf-8",
                    assets.lab_js.as_bytes(),
                    false,
                );
            }
            Route::LabCss => {
                return respond_static(
                    stream,
                    "text/css; charset=utf-8",
                    assets.lab_css.as_bytes(),
                    false,
                );
            }
            Route::Favicon => return respond_status(stream, 204, ""),
            Route::UiRelay { role, asset }
                if crate::access::RELAY_STATIC_ASSETS
                    .iter()
                    .any(|(name, _)| name == asset) =>
            {
                return serve_relay(stream, engine, role, asset);
            }
            _ => {}
        }
    }
    // Everything else is private: authenticate before any private info,
    // effect or stored-outcome replay. Missing, unknown, duplicate,
    // malformed or revoked credentials all fail closed here.
    let Some((auth, token)) = authenticate_request(engine, &request.headers) else {
        return respond_status(stream, 401, "authentication required");
    };
    match (method.as_str(), route) {
        ("GET", Route::Health) => respond_json(
            stream,
            &serde_json::json!({"ok": true, "run_id": engine.run_id()}),
        ),
        ("GET", Route::Runs) => {
            respond_json(stream, &serde_json::json!([{"run_id": engine.run_id()}]))
        }
        ("GET", Route::Me) => respond_json(stream, &crate::access::me_view(&auth.claim)),
        ("GET", Route::Wallets) => {
            // Scope before provider I/O, including the operator-only all view.
            if auth.claim.is_user() && !auth.claim.may.iter().any(|m| m == "view_own_status") {
                return respond_status(stream, 403, "wallet view is outside this claim");
            }
            let bindings = engine.wallet_registry.lock().unwrap().scoped(&auth.claim);
            let wallets: Vec<_> = bindings
                .iter()
                .map(crate::wallet_projection::view)
                .collect();
            let state = if wallets.is_empty() {
                "not_configured"
            } else if wallets.iter().any(|wallet| wallet["status"].is_null()) {
                "unknown"
            } else {
                "available"
            };
            respond_json(
                stream,
                &serde_json::json!({"schema":"spc-lab-wallets/v1", "wallets":wallets, "state":state}),
            )
        }
        ("GET", Route::Projection) => {
            if !query.is_empty() {
                return respond_status(
                    stream,
                    400,
                    "projection uses the authenticated claim; selectors are not accepted",
                );
            }
            if auth.claim.is_user() {
                respond_json(stream, &engine.user_projection(&auth))
            } else {
                respond_json(stream, &engine.user_projections())
            }
        }
        ("GET", Route::Snapshot) => {
            let cursor = match snapshot_cursor(&query) {
                Ok(cursor) => cursor,
                Err(message) => return respond_status(stream, 400, &message),
            };
            if auth.claim.is_user() {
                respond_json(stream, &engine.user_snapshot(&auth, cursor))
            } else {
                respond_json(stream, &engine.snapshot(cursor))
            }
        }
        ("GET", Route::Catalog) => {
            if auth.claim.is_user() {
                respond_status(stream, 403, "catalog requires the operator claim")
            } else {
                respond_json(stream, &engine.runtime_catalog())
            }
        }
        ("GET", Route::Inventory) => {
            // R4 merge condition: the hub inventory DTO is operator-only
            // in the expanded profile. A user claim is a closed 403 BEFORE
            // any inventory build, serialization or file I/O; a tokenless
            // or invalid request never reaches here (401 at the Bearer
            // gate above). The operator gets the Hub-approved
            // `spc.lab.inventory.v1` DTO.
            if auth.claim.is_user() {
                respond_status(stream, 403, "inventory requires the operator claim")
            } else {
                respond_json(stream, &engine.lab_inventory())
            }
        }
        ("GET", Route::Events) => {
            let after = match resolve_events_cursor(&request.headers, &query) {
                Ok(after) => after,
                Err(message) => return respond_status(stream, 400, &message),
            };
            if auth.claim.is_user() {
                serve_events_filtered(stream, engine, &auth, &token, after, shutdown)
            } else {
                // The operator stream re-authenticates at every event and
                // heartbeat exactly like the user stream; revocation closes
                // it at the next defined boundary.
                serve_events(
                    stream,
                    engine,
                    after,
                    shutdown,
                    Some((token.as_str(), auth.principal.as_str())),
                )
            }
        }
        ("GET", Route::Artifact(path)) => {
            // Whole-run shareable artifacts aggregate every principal's
            // commands, evidence counts and resources: operator-only in the
            // expanded profile, mirroring the catalog. A user token is a
            // closed 403 before any file I/O.
            if auth.claim.is_user() {
                respond_status(stream, 403, "artifacts require the operator claim")
            } else {
                serve_artifact(stream, engine, &path)
            }
        }
        ("POST", Route::Commands) => {
            // Same-origin boundary for native controller POSTs: no CORS
            // headers exist anywhere.
            if let Err(message) = check_same_origin(&stream, &request.headers) {
                return respond_status(stream, 403, &message);
            }
            if request.body.len() > MAX_BODY_BYTES {
                return respond_status(stream, 413, "request body too large");
            }
            // Candidate kinds that exist only as typed T0 payloads are
            // detected pre-parse and answered with a closed outcome - never
            // journaled, never pretended to be T1.
            if let Some(outcome) = if engine.scenario.traffic.is_none() {
                unavailable_kind_outcome(&request.body)
            } else {
                None
            } {
                return respond_json(stream, &outcome);
            }
            let command: LabCommand = match serde_json::from_slice(&request.body) {
                Ok(command) => command,
                Err(error) => {
                    return respond_status(stream, 400, &format!("invalid command: {error}"))
                }
            };
            let outcome = engine.submit_authenticated(&auth, command);
            respond_json(stream, &outcome)
        }
        // The relay is GET-only: a POST on the relay path is 405.
        (_, Route::UiRelay { .. }) if method != "GET" => {
            respond_status(stream, 405, "relay is GET-only")
        }
        ("GET", Route::UiRelay { role, asset }) => {
            if asset == crate::access::RELAY_STATUS_ASSET {
                // Relayed role status is operator-only in this foundation:
                // the user-facing agent status relay is gated on AP-4's
                // agent endpoint mapping.
                if auth.claim.is_user() {
                    return respond_status(
                        stream,
                        403,
                        "role status relay requires the operator claim",
                    );
                }
                serve_relay(stream, engine, &role, &asset)
            } else {
                // The static assets were served (or 404'd) before the auth
                // gate; anything left is not in the closed list.
                respond_status(stream, 404, "not found")
            }
        }
        _ => respond_status(stream, 404, "not found"),
    }
}

/// Parse the snapshot `cursor` query parameter (closed: a cursor that does
/// not parse is a client bug, rejected instead of silently replaying from
/// zero).
fn snapshot_cursor(
    query: &std::collections::BTreeMap<String, String>,
) -> Result<Option<u64>, String> {
    match query.get("cursor") {
        Some(raw) => parse_cursor(raw).map(Some),
        None => Ok(None),
    }
}

/// Extract and verify the Bearer credential. Exactly one well-formed
/// `Authorization: Bearer <token>` header; duplicate, malformed, missing,
/// unknown or revoked credentials all fail closed the same way.
fn authenticate_request(
    engine: &RunEngine,
    headers: &[(String, String)],
) -> Option<(crate::access::AuthContext, String)> {
    let mut seen = 0usize;
    let mut token = None;
    for (name, value) in headers {
        if name == "authorization" {
            seen += 1;
            token = value.strip_prefix("Bearer ").map(str::to_string);
        }
    }
    if seen != 1 {
        return None;
    }
    let token = token?;
    engine.authenticate_bearer(&token).map(|auth| (auth, token))
}

/// Same-origin boundary for POSTs: `Origin`/`Sec-Fetch-Site` check. A
/// native controller POST carries no `Origin`; a browser cross-origin POST
/// is rejected. This packet adds no CORS headers anywhere.
fn check_same_origin(stream: &TcpStream, headers: &[(String, String)]) -> Result<(), String> {
    if let Some(site) = header_value(headers, "sec-fetch-site") {
        if site.eq_ignore_ascii_case("cross-site") {
            return Err("cross-site POSTs are rejected".to_string());
        }
    }
    if let Some(origin) = header_value(headers, "origin") {
        let Ok(local) = stream.local_addr() else {
            return Err("POST origin could not be validated".to_string());
        };
        let expected = format!("http://{local}");
        if origin != expected {
            return Err("cross-origin POSTs are rejected".to_string());
        }
    }
    Ok(())
}

/// A closed outcome for a candidate kind with no runtime variant. Detected
/// on the raw body before `LabCommand` parsing, so `deny_unknown_fields`
/// stays and the answer is a real `CommandOutcome` with no journal entry.
fn unavailable_kind_outcome(body: &[u8]) -> Option<crate::command::CommandOutcome> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    let tag = value.get("kind")?.get("type")?.as_str()?;
    if !crate::access::is_unavailable_kind(tag) {
        return None;
    }
    let command_id = value
        .get("command_id")
        .and_then(|value| value.as_str())
        .and_then(|id| crate::ids::CommandId::new(id).ok())
        .unwrap_or_else(crate::ids::CommandId::generate);
    Some(crate::access::closed_rejection(
        &command_id,
        "command kind is not available in this run",
    ))
}

/// Serve a static asset; `csp` adds the workbench
/// Content-Security-Policy (expanded-profile HTML only).
fn respond_static(
    mut stream: TcpStream,
    content_type: &str,
    body: &[u8],
    csp: bool,
) -> std::io::Result<()> {
    let mut head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n",
        body.len()
    );
    if csp {
        head.push_str(&format!(
            "Content-Security-Policy: {}\r\n",
            crate::access::WORKBENCH_CSP
        ));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}

/// Serve one relayed role asset: a fixed GET to the role's declared
/// listener (`<role>.api` in the runtime's declared addresses - never a
/// browser-supplied target), a closed asset/MIME list, bounded body, and
/// closed error mapping: an upstream failure becomes the relay's own error
/// with no body detail.
fn serve_relay(
    stream: TcpStream,
    engine: &RunEngine,
    role: &str,
    asset: &str,
) -> std::io::Result<()> {
    let Some(content_type) = crate::access::relay_asset_content_type(asset) else {
        return respond_status(stream, 404, "not found");
    };
    let Some(target) = engine.relay_target(role) else {
        return respond_status(stream, 404, "not found");
    };
    let url = format!("http://{target}/{asset}");
    let body = match crate::namespace::http_request(&url, "GET", None, None, None) {
        Ok(body) => body,
        Err(_) => return respond_status(stream, 502, "role upstream is unavailable"),
    };
    if body.len() > crate::access::RELAY_MAX_BODY_BYTES {
        return respond_status(stream, 502, "role upstream is unavailable");
    }
    let csp = content_type.starts_with("text/html");
    respond_static(stream, content_type, body.as_bytes(), csp)
}

/// Write one filtered resync snapshot event.
fn write_filtered_resync(
    stream: &mut TcpStream,
    engine: &RunEngine,
    auth: &crate::access::AuthContext,
    cursor: u64,
) -> std::io::Result<()> {
    let snapshot = engine.user_snapshot(auth, Some(cursor));
    let snapshot_cursor = snapshot
        .get("snapshot_cursor")
        .and_then(|value| value.as_u64())
        .unwrap_or(0);
    let data = serde_json::to_string(&snapshot)?;
    write!(
        stream,
        "event: resync\nid: {snapshot_cursor}\ndata: {data}\n\n"
    )?;
    stream.flush()
}

fn write_sse_filtered(
    stream: &mut TcpStream,
    event: &crate::access::FilteredEvent,
) -> std::io::Result<()> {
    let data = serde_json::to_string(event)?;
    // Same generic `message` framing as the operator stream; the payload
    // carries the filtered DTO with its own `kind`.
    write!(
        stream,
        "id: {}\nevent: message\ndata: {data}\n\n",
        event.event_id
    )?;
    stream.flush()
}

/// The filtered user SSE stream. The same filter governs the initial
/// replay, live events and the `resync` snapshot, applied server-side
/// before JSON serialization. Cursor and gap semantics are preserved, so a
/// filtered-empty output is never mistaken for healthy. Revocation applies
/// to new reads immediately; on an open stream the server re-authenticates
/// at every event and heartbeat boundary and closes the connection when the
/// claim is gone - nothing leaks while idle.
fn serve_events_filtered(
    mut stream: TcpStream,
    engine: &RunEngine,
    auth: &crate::access::AuthContext,
    token: &str,
    after: u64,
    shutdown: &Arc<AtomicBool>,
) -> std::io::Result<()> {
    let still_valid = |auth: &crate::access::AuthContext| -> bool {
        engine.authenticate_bearer(token).is_some_and(|current| {
            current.principal == auth.principal
                && crate::access::me_view(&current.claim) == crate::access::me_view(&auth.claim)
        })
    };
    match engine.evidence().subscribe(after) {
        Ok((_subscription, crate::evidence::SubscribeResult::ResyncRequired { .. })) => {
            if !still_valid(auth) {
                return Ok(());
            }
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n\r\n"
            )?;
            write_filtered_resync(&mut stream, engine, auth, after)
        }
        Ok((subscription, _)) => {
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\nX-Accel-Buffering: no\r\n\r\n"
            )?;
            for event in &subscription.replay {
                if !still_valid(auth) {
                    return Ok(());
                }
                if let Some(filtered) = crate::access::filter_event(&auth.claim, event) {
                    if write_sse_filtered(&mut stream, &filtered).is_err() {
                        return Ok(());
                    }
                }
            }
            loop {
                if shutdown.load(Ordering::SeqCst) {
                    return Ok(());
                }
                if !still_valid(auth) {
                    // Revoked: close at this defined boundary.
                    return Ok(());
                }
                match subscription.receiver.recv_timeout(Duration::from_secs(15)) {
                    Ok(event) => {
                        // Re-authenticate before the next private event,
                        // visible or not.
                        if !still_valid(auth) {
                            return Ok(());
                        }
                        if let Some(filtered) = crate::access::filter_event(&auth.claim, &event) {
                            if write_sse_filtered(&mut stream, &filtered).is_err() {
                                return Ok(());
                            }
                        }
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        // Revoked while idle: close at the heartbeat.
                        if !still_valid(auth) {
                            return Ok(());
                        }
                        if stream.write_all(b": keepalive\n\n").is_err() {
                            return Ok(());
                        }
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                        // An overflowed subscriber was intentionally closed.
                        // Answer with the filtered resync protocol instead
                        // of silently ending at a gap.
                        if !still_valid(auth) {
                            return Ok(());
                        }
                        return write_filtered_resync(&mut stream, engine, auth, 0);
                    }
                }
            }
        }
        Err(_) => {
            if !still_valid(auth) {
                return Ok(());
            }
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n\r\n"
            )?;
            write_filtered_resync(&mut stream, engine, auth, after)
        }
    }
}

/// Resolve the SSE replay cursor: `Last-Event-ID` wins over `after`, and both
/// must be well-formed decimal event ids. Stale and future cursors are not
/// decided here: `EvidenceStore::subscribe` returns `ResyncRequired` and the
/// stream answers with an `event: resync` snapshot, so there is no silent skip
/// in either direction.
fn resolve_events_cursor(
    headers: &[(String, String)],
    query: &std::collections::BTreeMap<String, String>,
) -> Result<u64, String> {
    if let Some((_, value)) = headers.iter().find(|(name, _)| name == "last-event-id") {
        return parse_cursor(value);
    }
    match query.get("after") {
        Some(raw) => parse_cursor(raw),
        None => Ok(0),
    }
}

fn parse_cursor(raw: &str) -> Result<u64, String> {
    raw.parse::<u64>()
        .map_err(|_| format!("invalid cursor {raw:?}: expected a decimal event id"))
}

#[derive(Debug)]
enum Route {
    AccessJs,
    Index,
    AppJs,
    StyleCss,
    Lab,
    LabJs,
    LabCss,
    Favicon,
    Health,
    Runs,
    Snapshot,
    Events,
    Commands,
    Catalog,
    Inventory,
    Artifact(PathBuf),
    Me,
    Wallets,
    Projection,
    UiRelay { role: String, asset: String },
}

fn route(run_id: &str, path: &str) -> Route {
    if path == "/access.js" {
        return Route::AccessJs;
    }
    if path == "/" || path == "/index.html" {
        return Route::Index;
    }
    if path == "/app.js" {
        return Route::AppJs;
    }
    if path == "/style.css" {
        return Route::StyleCss;
    }
    if path == "/lab" || path == "/lab/" || path == "/lab/index.html" {
        return Route::Lab;
    }
    if path == "/lab.js" {
        return Route::LabJs;
    }
    if path == "/lab.css" {
        return Route::LabCss;
    }
    if path == "/favicon.ico" {
        return Route::Favicon;
    }
    if path == "/api/lab/v1/health" {
        return Route::Health;
    }
    if path == "/api/lab/v1/runs" {
        return Route::Runs;
    }
    let prefix = format!("/api/lab/v1/runs/{run_id}");
    if let Some(rest) = path.strip_prefix(&prefix) {
        match rest {
            "/snapshot" => return Route::Snapshot,
            "/events" => return Route::Events,
            "/commands" => return Route::Commands,
            "/catalog" => return Route::Catalog,
            "/inventory" => return Route::Inventory,
            "/me" => return Route::Me,
            "/wallets" => return Route::Wallets,
            "/projection" => return Route::Projection,
            _ => {}
        }
        if let Some(rest) = rest.strip_prefix("/ui/") {
            if let Some((role, asset)) = rest.split_once('/') {
                return Route::UiRelay {
                    role: role.to_string(),
                    asset: asset.to_string(),
                };
            }
        }
        if let Some(artifact) = rest.strip_prefix("/artifacts/") {
            return Route::Artifact(PathBuf::from(artifact));
        }
    }
    Route::Artifact(PathBuf::new())
}

fn serve_events(
    mut stream: TcpStream,
    engine: &RunEngine,
    after: u64,
    shutdown: &Arc<AtomicBool>,
    revocation: Option<(&str, &str)>,
) -> std::io::Result<()> {
    // The legacy path passes `None`: no credential exists and the stream
    // never closes on revocation. The expanded operator path passes its
    // (token, principal) pair and is re-checked before every event and at
    // every heartbeat, exactly like the filtered user stream: a deleted or
    // rotated operator claim closes the open stream at the next defined
    // boundary with nothing further delivered.
    let still_valid = |revocation: Option<(&str, &str)>| -> bool {
        match revocation {
            None => true,
            Some((token, principal)) => engine
                .authenticate_bearer(token)
                .is_some_and(|current| current.principal == principal),
        }
    };
    // The resync decision is the evidence store's, never the HTTP layer's: a
    // cursor older than the retained tail OR beyond the last event comes back
    // as SubscribeResult::ResyncRequired and is answered with a snapshot.
    match engine.evidence().subscribe(after) {
        Ok((_subscription, crate::evidence::SubscribeResult::ResyncRequired { .. })) => {
            if !still_valid(revocation) {
                return Ok(());
            }
            let snapshot = engine.snapshot(Some(after));
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n\r\n"
            )?;
            let data = serde_json::to_string(&snapshot)?;
            write!(
                stream,
                "event: resync\nid: {}\ndata: {data}\n\n",
                snapshot.snapshot_cursor
            )?;
            stream.flush()
        }
        Ok((subscription, _)) => {
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\nX-Accel-Buffering: no\r\n\r\n"
            )?;
            for event in &subscription.replay {
                if !still_valid(revocation) {
                    return Ok(());
                }
                if write_sse(&mut stream, event).is_err() {
                    return Ok(());
                }
            }
            loop {
                if shutdown.load(Ordering::SeqCst) {
                    return Ok(());
                }
                if !still_valid(revocation) {
                    // Revoked: close at this defined boundary.
                    return Ok(());
                }
                match subscription.receiver.recv_timeout(Duration::from_secs(15)) {
                    Ok(event) => {
                        // Re-authenticate before the next event, visible or
                        // not.
                        if !still_valid(revocation) {
                            return Ok(());
                        }
                        if write_sse(&mut stream, &event).is_err() {
                            return Ok(());
                        }
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        // Revoked while idle: close at the heartbeat.
                        if !still_valid(revocation) {
                            return Ok(());
                        }
                        if stream.write_all(b": keepalive\n\n").is_err() {
                            return Ok(());
                        }
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                        // An overflowed subscriber was intentionally closed.
                        // Send the existing resync protocol instead of
                        // silently ending at a gap.
                        if !still_valid(revocation) {
                            return Ok(());
                        }
                        let snapshot = engine.snapshot(Some(0));
                        let data = serde_json::to_string(&snapshot)?;
                        write!(
                            stream,
                            "event: resync\nid: {}\ndata: {data}\n\n",
                            snapshot.snapshot_cursor
                        )?;
                        return stream.flush();
                    }
                }
            }
        }
        Err(_) => {
            if !still_valid(revocation) {
                return Ok(());
            }
            let snapshot = engine.snapshot(Some(after));
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n\r\n"
            )?;
            let data = serde_json::to_string(&snapshot)?;
            write!(
                stream,
                "event: resync\nid: {}\ndata: {data}\n\n",
                snapshot.snapshot_cursor
            )?;
            stream.flush()
        }
    }
}

fn write_sse(
    stream: &mut TcpStream,
    event: &crate::evidence::EvidenceEvent,
) -> std::io::Result<()> {
    let data = serde_json::to_string(event)?;
    // Generic `message` type: the payload carries `kind`, so an EventSource
    // needs one listener for every ordinary evidence kind, including kinds
    // added in later revisions (`miner_event`, `spc_evidence`, ...). The
    // `id:` line is what lets a reconnect resume via Last-Event-ID.
    write!(
        stream,
        "id: {}\nevent: message\ndata: {data}\n\n",
        event.event_id
    )?;
    stream.flush()
}

fn serve_artifact(
    mut stream: TcpStream,
    engine: &RunEngine,
    relative: &Path,
) -> std::io::Result<()> {
    let safe = sanitize_artifact(relative);
    let Some(safe) = safe else {
        return respond_status(stream, 403, "artifact path is not shareable");
    };
    let root = engine.manifest_snapshot().artifact_paths.root;
    let path = root.join(&safe);
    let Ok(bytes) = std::fs::read(&path) else {
        return respond_status(stream, 404, "artifact not found");
    };
    let content_type = if safe.extension().and_then(|e| e.to_str()) == Some("json") {
        "application/json"
    } else {
        "text/plain; charset=utf-8"
    };
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        bytes.len()
    )?;
    stream.write_all(&bytes)?;
    stream.flush()
}

fn sanitize_artifact(relative: &Path) -> Option<PathBuf> {
    if relative.as_os_str().is_empty() || relative.is_absolute() {
        return None;
    }
    let mut safe = PathBuf::new();
    let mut components = relative.components();
    match components.next()? {
        Component::Normal(first) if first == "shareable" => safe.push(first),
        _ => return None,
    }
    for component in components {
        match component {
            Component::Normal(part) => safe.push(part),
            _ => return None,
        }
    }
    Some(safe)
}

struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

/// A rejected request. The status is chosen before any response is written;
/// every variant answers with a plain text reason.
#[derive(Debug)]
struct RequestError {
    status: u16,
    message: String,
}

impl RequestError {
    fn malformed(message: impl Into<String>) -> Self {
        Self {
            status: 400,
            message: message.into(),
        }
    }

    fn too_large(message: impl Into<String>) -> Self {
        Self {
            status: 413,
            message: message.into(),
        }
    }
}

impl From<std::io::Error> for RequestError {
    fn from(error: std::io::Error) -> Self {
        Self::malformed(format!("request read failed: {error}"))
    }
}

/// Read one CRLF-terminated line under a shared byte budget. The budget is
/// decremented before the line is parsed, so the request line and the whole
/// header block together can never exceed `MAX_HEADER_BYTES`, and no line can
/// cause an allocation past what was actually read from the socket. A line
/// that does not terminate within the remaining budget (for example a request
/// line of any length sent without a CRLF) is rejected without waiting for
/// more data.
fn read_bounded_line(
    reader: &mut impl BufRead,
    budget: &mut usize,
) -> Result<String, RequestError> {
    if *budget == 0 {
        return Err(RequestError::malformed("headers exceed the 16 KiB limit"));
    }
    let mut take = reader.take(*budget as u64);
    let mut bytes = Vec::new();
    let read = take.read_until(b'\n', &mut bytes)?;
    if read == 0 {
        return Err(RequestError::malformed(
            "unexpected end of request before a complete header block",
        ));
    }
    *budget -= read;
    if bytes.last() != Some(&b'\n') {
        // The budget was exhausted (or the peer quit mid-line) before the
        // terminator arrived. Fail closed; do not wait for more bytes.
        return Err(RequestError::malformed(
            "header line exceeds the 16 KiB limit or lacks a terminating CRLF",
        ));
    }
    bytes.pop();
    if bytes.last() == Some(&b'\r') {
        bytes.pop();
    }
    String::from_utf8(bytes)
        .map_err(|_| RequestError::malformed("request headers are not valid UTF-8"))
}

fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

fn read_request(stream: TcpStream) -> Result<Request, RequestError> {
    // A small buffer keeps the pre-parse socket read itself bounded; the
    // budget below is what bounds acceptance.
    let mut reader = BufReader::with_capacity(2048, stream);
    let mut budget = MAX_HEADER_BYTES;
    let request_line = read_bounded_line(&mut reader, &mut budget)?;
    let mut parts = request_line.split_whitespace();
    let method = parts
        .next()
        .filter(|part| !part.is_empty())
        .ok_or_else(|| RequestError::malformed("request line has no method"))?
        .to_string();
    let path = parts
        .next()
        .filter(|part| !part.is_empty())
        .ok_or_else(|| RequestError::malformed("request line has no path"))?
        .to_string();

    let mut headers: Vec<(String, String)> = Vec::new();
    loop {
        let line = read_bounded_line(&mut reader, &mut budget)?;
        if line.is_empty() {
            break;
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(RequestError::malformed(format!(
                "malformed header line {line:?}"
            )));
        };
        let name = name.trim_ascii().to_ascii_lowercase();
        let value = value.trim_ascii().to_string();
        if name.is_empty() {
            return Err(RequestError::malformed("header line has no field name"));
        }
        headers.push((name, value));
    }

    // Unsupported framing is rejected clearly instead of misread: the server
    // only speaks identity framing with a byte length it can bound.
    if header_value(&headers, "transfer-encoding").is_some() {
        return Err(RequestError::malformed(
            "transfer-encoding is not supported; send identity framing with content-length",
        ));
    }
    let content_length = match header_value(&headers, "content-length") {
        Some(raw) => raw
            .parse::<usize>()
            .map_err(|_| RequestError::malformed(format!("invalid content-length {raw:?}")))?,
        None => 0,
    };
    if content_length > MAX_BODY_BYTES {
        return Err(RequestError::too_large(format!(
            "request body of {content_length} bytes exceeds the {MAX_BODY_BYTES} byte limit"
        )));
    }
    // Bounded by the check above and by the socket read deadline.
    let mut body = vec![0u8; content_length];
    reader.read_exact(&mut body)?;
    Ok(Request {
        method,
        path,
        headers,
        body,
    })
}

fn split_query(path: &str) -> (String, std::collections::BTreeMap<String, String>) {
    let (path, query) = path.split_once('?').unwrap_or((path, ""));
    let mut values = std::collections::BTreeMap::new();
    for pair in query.split('&').filter(|part| !part.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        values.insert(key.to_string(), percent_decode(value));
    }
    (percent_decode(path), values)
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&value[index + 1..index + 3], 16) {
                out.push(byte);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn respond_json<T: Serialize>(mut stream: TcpStream, value: &T) -> std::io::Result<()> {
    let body = serde_json::to_vec(value)?;
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n",
        body.len()
    )?;
    stream.write_all(&body)?;
    stream.flush()
}

fn respond_bytes(mut stream: TcpStream, content_type: &str, body: &[u8]) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)?;
    stream.flush()
}

fn respond_status(mut stream: TcpStream, status: u16, message: &str) -> std::io::Result<()> {
    let reason = match status {
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "Error",
    };
    let body = format!("{status} {reason}\n{message}\n");
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body.as_bytes())?;
    stream.flush()
}
