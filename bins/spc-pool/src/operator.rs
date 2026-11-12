//! EXP-OP: the read-only loopback operator page and its observation DTO
//! (contract `2026-11-07-live-lab-integration-contracts.md` §4).
//!
//! This module owns a dedicated, optional, off-by-default listener that
//! runs inside the same `spc-pool` API process as the public `[api]`
//! listener, on a validated loopback address only. Nothing is mounted on
//! the public router ([`crate::api`]) and nothing here writes: there are
//! no POST routes, no controls, no auth platform and no Ark daemon
//! startup. The public API stays byte-for-byte unchanged.
//!
//! Failure model (OP-1): every store-sourced DTO field is nullable. A
//! failed or inconsistent read yields `null` for that field plus one
//! closed-code `{field, code, source}` entry in `errors`; the full
//! underlying error is logged server-side only. The DTO never contains a
//! `StoreError` display string, a DSN, a private path, SQL text, a table
//! or column name, or a row value, and no `sp_id`, VTXO id, order id or
//! Ark address appears anywhere.
//!
//! Money model (OP-2): `pending_liability` comes from one
//! [`Store::liability_snapshot`] call, so `total_sats ==
//! pending_balance_sats + ark_due_sats` with a single `as_of`; on
//! invariant failure the whole object is `null` with
//! `snapshot_inconsistent`, never a partial number. Sats serialize as
//! exact decimal strings; the page formats them with `BigInt` and never
//! through `Number`.
//!
//! Blocks model (OP-3): `recent_blocks` is one existing
//! [`Store::recent_blocks`] call with a fixed bound of ten rows,
//! height-descending, exactly the four closed row fields. `rows: []` is a
//! legal empty table; a failed query nulls the whole object. The page
//! derives its latest-block fact from `rows[0]`; there is no competing
//! latest-block query and no cross-listener fetch.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;
use serde_json::Value;
use spc_store::postgres::{now_rfc3339, LiabilitySnapshot, StoreError, LIABILITY_TOTAL_OVERFLOW};
use spc_store::Store;

/// The fixed bound of recent block rows in the DTO (OP-3). The Details
/// disclosure states this bound explicitly; it is never advertised as an
/// unbounded "recent blocks".
pub const RECENT_BLOCKS_BOUND: i64 = 10;

/// The DTO schema identifier.
pub const OBSERVATION_SCHEMA: &str = "spc-pool-observation/v1";

/// Static assets, embedded at compile time and served only through their
/// relative operator-listener paths.
const INDEX_HTML: &str = include_str!("../static/operator/index.html");
const OPERATOR_CSS: &str = include_str!("../static/operator/operator.css");
const OPERATOR_JS: &str = include_str!("../static/operator/operator.js");

/// One recent block row as the operator surface sees it: the store's
/// four closed fields with `settled_at` already RFC3339-formatted, so no
/// consumer of [`ObservationSource`] needs to name the store's timestamp
/// type (spc-pool deliberately has no chrono dependency).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceBlockRow {
    pub block_hash: Vec<u8>,
    pub height: i32,
    pub status: String,
    pub settled_at: Option<String>,
}

/// The read surface the operator backend needs from the store. The real
/// implementation is [`Store`] itself; the trait exists so the DTO's
/// failure mapping is provable without a database (the same pattern as
/// [`crate::api::ApiBackend`] and its `StoreBackend`).
pub trait ObservationSource: Send + Sync + 'static {
    fn liability_snapshot(
        &self,
    ) -> impl std::future::Future<Output = spc_store::postgres::Result<LiabilitySnapshot>> + Send;
    fn ark_payouts_by_status(
        &self,
    ) -> impl std::future::Future<Output = spc_store::postgres::Result<Vec<(String, u64)>>> + Send;
    fn recent_blocks(
        &self,
        limit: i64,
    ) -> impl std::future::Future<Output = spc_store::postgres::Result<Vec<SourceBlockRow>>> + Send;
}

impl ObservationSource for Store {
    async fn liability_snapshot(&self) -> spc_store::postgres::Result<LiabilitySnapshot> {
        Store::liability_snapshot(self).await
    }
    async fn ark_payouts_by_status(&self) -> spc_store::postgres::Result<Vec<(String, u64)>> {
        Store::ark_payouts_by_status(self).await
    }
    async fn recent_blocks(&self, limit: i64) -> spc_store::postgres::Result<Vec<SourceBlockRow>> {
        Ok(Store::recent_blocks(self, limit)
            .await?
            .into_iter()
            .map(|(block_hash, height, status, settled_at)| SourceBlockRow {
                block_hash,
                height,
                status,
                settled_at: settled_at.map(|t| t.to_rfc3339()),
            })
            .collect())
    }
}

/// The closed error-code set (OP-1). Nothing else ever enters `errors`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObsErrorCode {
    StoreUnavailable,
    Timeout,
    SchemaMismatch,
    InvalidValue,
    SnapshotInconsistent,
    Unknown,
}

impl ObsErrorCode {
    /// Map a store error to its closed code. The underlying error never
    /// leaves the server log.
    fn from_store_error(e: &StoreError) -> Self {
        match e {
            StoreError::Timeout(_) => Self::Timeout,
            StoreError::Invalid(LIABILITY_TOTAL_OVERFLOW) => Self::SnapshotInconsistent,
            StoreError::Invalid(_) => Self::InvalidValue,
            e if e.is_schema_mismatch() => Self::SchemaMismatch,
            StoreError::Db(_) => Self::StoreUnavailable,
            _ => Self::Unknown,
        }
    }
}

/// One `errors` entry: exactly `{field, code, source}`, nothing else.
#[derive(Clone, Debug, Serialize)]
pub struct ObsError {
    pub field: &'static str,
    pub code: ObsErrorCode,
    pub source: &'static str,
}

/// The single-snapshot money facts (OP-2). All sats are decimal strings.
#[derive(Clone, Debug, Serialize)]
pub struct PendingLiability {
    pub as_of: String,
    pub pending_balance_sats: String,
    pub ark_due_sats: String,
    pub total_sats: String,
}

/// The ten most recent block rows (OP-3), from one
/// [`ObservationSource::recent_blocks`] call with its own `observed_at`,
/// explicitly distinct from the money snapshot's `as_of`.
#[derive(Clone, Debug, Serialize)]
pub struct RecentBlocks {
    pub observed_at: String,
    pub rows: Vec<BlockRow>,
}

/// The closed block row shape: exactly the four existing fields, in the
/// store's actual height-descending order.
#[derive(Clone, Debug, Serialize)]
pub struct BlockRow {
    pub block_hash: String,
    pub height: i32,
    pub status: String,
    pub settled_at: Option<String>,
}

/// Ark float state. `not_configured` until AP-4's real status source
/// exists; never hardcoded `ok`, and `/healthz` is never pool health.
#[derive(Clone, Debug, Serialize)]
pub struct FloatState {
    pub state: &'static str,
    pub reason: &'static str,
}

#[derive(Clone, Debug, Serialize)]
pub struct Sources {
    pub liability: &'static str,
    pub blocks: &'static str,
}

/// The `spc-pool-observation/v1` DTO.
#[derive(Clone, Debug, Serialize)]
pub struct Observation {
    pub schema: &'static str,
    pub generated_at: String,
    pub network: String,
    pub work_profile: Value,
    pub window_n: u32,
    pub recent_blocks: Option<RecentBlocks>,
    pub pending_liability: Option<PendingLiability>,
    pub ark_payouts_by_status: Option<serde_json::Map<String, Value>>,
    pub float: FloatState,
    pub sources: Sources,
    pub errors: Vec<ObsError>,
}

/// The work-profile field allowlist. `profile_json` is rebuilt through
/// this list; no other config string reaches the DTO, and `endpoints`
/// (the `[reach] advertise` republish) is omitted entirely. Every value
/// that does pass is inert text on the page (`textContent`), never an
/// `href`, `src` or fetch target.
pub const WORK_PROFILE_FIELDS: &[&str] = &[
    "work_unit_version",
    "work_reference_target",
    "share_work_multiplier",
    "share_difficulty",
    "work_unit_fingerprint",
    "max_work_multiplier",
    "max_window_target_units",
    "window_n",
    "window_rule",
    "pow_limit_bits",
    "max_slice_shares",
];

/// Copy only the allowlisted keys out of a work profile.
fn allowlist_work_profile(profile: &Value) -> Value {
    let mut allowed = serde_json::Map::new();
    for key in WORK_PROFILE_FIELDS {
        if let Some(v) = profile.get(*key) {
            allowed.insert((*key).to_string(), v.clone());
        }
    }
    Value::Object(allowed)
}

/// The config-derived, non-store facts the page shows.
#[derive(Clone, Debug)]
pub struct ObservationFacts {
    pub network: String,
    pub window_n: u32,
    pub work_profile: Value,
}

impl ObservationFacts {
    /// Build the allowlisted facts from a pool [`crate::config::Config`].
    pub fn from_config(config: &crate::config::Config) -> Self {
        Self {
            network: config.pool.network.clone(),
            window_n: config.accounting.window_n,
            work_profile: allowlist_work_profile(&config.accounting.profile_json()),
        }
    }
}

/// The operator backend: reads the store through [`ObservationSource`]
/// and renders the DTO with closed-code error capture.
pub struct OperatorBackend<S: ObservationSource = Store> {
    source: Arc<S>,
    facts: ObservationFacts,
}

impl<S: ObservationSource> OperatorBackend<S> {
    pub fn new(source: S, facts: ObservationFacts) -> Self {
        Self {
            source: Arc::new(source),
            facts,
        }
    }

    /// Gather one observation. Every store read is nullable; failures
    /// push closed-code entries and never fabricate a zero or an empty
    /// list. The full error is logged server-side only.
    pub async fn observation(&self) -> Observation {
        let mut errors = Vec::new();

        // The three store reads are independent; run them concurrently so
        // a store outage costs one pool timeout, not three (a sequential
        // version held the response ~90s and kept the page on stale data
        // long past the poll interval). Error order below stays fixed:
        // pending_liability, ark_payouts_by_status, recent_blocks.
        let (liability, payouts, blocks) = tokio::join!(
            self.source.liability_snapshot(),
            self.source.ark_payouts_by_status(),
            self.source.recent_blocks(RECENT_BLOCKS_BOUND),
        );

        // OP-2: one liability snapshot, one as_of, invariant re-verified
        // at the DTO boundary.
        let pending_liability = match liability {
            Ok(snap) => {
                let invariant = snap
                    .pending_balance_sats
                    .checked_add(snap.ark_due_sats)
                    .is_some_and(|total| total == snap.total_sats);
                if invariant {
                    Some(PendingLiability {
                        as_of: snap.as_of.to_rfc3339(),
                        pending_balance_sats: snap.pending_balance_sats.to_string(),
                        ark_due_sats: snap.ark_due_sats.to_string(),
                        total_sats: snap.total_sats.to_string(),
                    })
                } else {
                    errors.push(ObsError {
                        field: "pending_liability",
                        code: ObsErrorCode::SnapshotInconsistent,
                        source: "postgres",
                    });
                    None
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "operator: liability snapshot failed");
                errors.push(ObsError {
                    field: "pending_liability",
                    code: ObsErrorCode::from_store_error(&e),
                    source: "postgres",
                });
                None
            }
        };

        // Exactly the current schema statuses, zero-filled like the
        // metrics exporter; no `reorged` status is invented.
        let ark_payouts_by_status = match payouts {
            Ok(rows) => {
                let mut map = serde_json::Map::new();
                for status in ["scheduled", "sent", "failed_to_pending"] {
                    let n: u64 = rows
                        .iter()
                        .filter(|(s, _)| s == status)
                        .map(|(_, n)| *n)
                        .sum();
                    map.insert(status.to_string(), Value::from(n));
                }
                Some(map)
            }
            Err(e) => {
                tracing::warn!(error = %e, "operator: ark payouts by status failed");
                errors.push(ObsError {
                    field: "ark_payouts_by_status",
                    code: ObsErrorCode::from_store_error(&e),
                    source: "postgres",
                });
                None
            }
        };

        // OP-3: one recent_blocks call, fixed bound, own observed_at. The
        // bound and the height-descending order are enforced here too,
        // not just in the SQL: a source that misbehaves cannot push an
        // eleventh row or reorder the DTO.
        let recent_blocks = match blocks {
            Ok(rows) => {
                let mut rows = rows;
                rows.sort_by(|a, b| b.height.cmp(&a.height));
                rows.truncate(RECENT_BLOCKS_BOUND as usize);
                Some(RecentBlocks {
                    observed_at: now_rfc3339(),
                    rows: rows
                        .into_iter()
                        .map(|row| BlockRow {
                            block_hash: hex::encode(&row.block_hash),
                            height: row.height,
                            status: row.status,
                            settled_at: row.settled_at,
                        })
                        .collect(),
                })
            }
            Err(e) => {
                tracing::warn!(error = %e, "operator: recent blocks failed");
                errors.push(ObsError {
                    field: "recent_blocks",
                    code: ObsErrorCode::from_store_error(&e),
                    source: "postgres",
                });
                None
            }
        };

        Observation {
            schema: OBSERVATION_SCHEMA,
            generated_at: now_rfc3339(),
            network: self.facts.network.clone(),
            work_profile: self.facts.work_profile.clone(),
            window_n: self.facts.window_n,
            recent_blocks,
            pending_liability,
            ark_payouts_by_status,
            float: FloatState {
                state: "not_configured",
                reason: "ap-4 status source absent",
            },
            sources: Sources {
                liability: "postgres",
                blocks: "postgres",
            },
            errors,
        }
    }
}

/// Errors from the operator listener itself.
#[derive(Debug, thiserror::Error)]
pub enum OperatorError {
    #[error("operator listener address {0} is not loopback; the operator page is loopback-only")]
    NotLoopback(SocketAddr),
    #[error("operator listener bind failed: {0}")]
    Bind(#[from] std::io::Error),
}

/// The operator listener. Loopback-only by construction:
/// [`OperatorServer::bind`] refuses any non-loopback address
/// independently of config validation, so a programmatic caller cannot
/// bypass the loopback rule either.
pub struct OperatorServer<S: ObservationSource = Store> {
    listener: tokio::net::TcpListener,
    app: Router,
    _marker: std::marker::PhantomData<S>,
}

impl<S: ObservationSource> OperatorServer<S> {
    /// Refuse a non-loopback address (fail closed, contract §4 OP-1).
    pub fn validate_loopback(addr: &SocketAddr) -> Result<(), OperatorError> {
        if addr.ip().is_loopback() {
            Ok(())
        } else {
            Err(OperatorError::NotLoopback(*addr))
        }
    }

    /// Bind the dedicated operator listener. The address must be
    /// loopback; binding any public or otherwise non-loopback address is
    /// an error the caller turns into a process startup failure.
    pub async fn bind(
        addr: SocketAddr,
        backend: OperatorBackend<S>,
    ) -> Result<Self, OperatorError> {
        Self::validate_loopback(&addr)?;
        let listener = tokio::net::TcpListener::bind(addr).await?;
        let app = router(backend);
        Ok(Self {
            listener,
            app,
            _marker: std::marker::PhantomData,
        })
    }

    /// Serve until the process stops. Runs as a tracked task in the pool
    /// API process; there is no detached service and no orphan.
    pub async fn serve(self) -> std::io::Result<()> {
        axum::serve(self.listener, self.app).await
    }

    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.listener.local_addr()
    }
}

/// The operator router: exactly four GET routes, same-origin. Anything
/// else is 404; there are no POST routes; a request that carries a body
/// is rejected before routing.
pub fn router<S: ObservationSource>(backend: OperatorBackend<S>) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/operator.css", get(operator_css))
        .route("/operator.js", get(operator_js))
        .route("/api/pool/observation", get(observation::<S>))
        .fallback(not_found)
        .layer(middleware::from_fn(reject_request_bodies))
        .with_state(Arc::new(backend))
}

async fn index() -> Response {
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        [(header::CACHE_CONTROL, "no-store")],
        INDEX_HTML,
    )
        .into_response()
}

async fn operator_css() -> Response {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        [(header::CACHE_CONTROL, "no-store")],
        OPERATOR_CSS,
    )
        .into_response()
}

async fn operator_js() -> Response {
    (
        [(
            header::CONTENT_TYPE,
            "application/javascript; charset=utf-8",
        )],
        [(header::CACHE_CONTROL, "no-store")],
        OPERATOR_JS,
    )
        .into_response()
}

async fn observation<S: ObservationSource>(
    State(backend): State<Arc<OperatorBackend<S>>>,
) -> Response {
    // Same no-store policy as the three static routes (F6): the page
    // polls with cache: 'no-store', and the response itself must not be
    // cacheable either.
    (
        [(header::CACHE_CONTROL, "no-store")],
        Json(backend.observation().await),
    )
        .into_response()
}

/// Unknown path: 404 with no information beyond the status.
async fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "not found").into_response()
}

/// Reject any request that carries a body (unexpected for this surface)
/// before routing. Methods are constrained by the `get()` routes
/// themselves (non-GET on a known path is 405).
async fn reject_request_bodies(req: Request, next: Next) -> Response {
    if req.headers().contains_key(header::TRANSFER_ENCODING) {
        return (StatusCode::BAD_REQUEST, "request body not accepted").into_response();
    }
    if let Some(len) = req.headers().get(header::CONTENT_LENGTH) {
        if len != "0" {
            return (StatusCode::BAD_REQUEST, "request body not accepted").into_response();
        }
    }
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The allowlist is the only path config strings take into the DTO:
    /// keys outside it (endpoints, storage, anything else) never appear.
    #[test]
    fn work_profile_fields_are_the_closed_allowlist() {
        let profile = serde_json::json!({
            "work_unit_version": 1,
            "window_rule": "share_work_legacy",
            "window_n": 8,
            "endpoints": ["javascript:alert(1)"],
            "postgres": "postgres://user:secret@host/db",
        });
        let allowed = allowlist_work_profile(&profile);
        let text = serde_json::to_string(&allowed).unwrap();
        assert!(!text.contains("endpoints"));
        assert!(!text.contains("secret"));
        assert!(text.contains("share_work_legacy"));
        assert!(text.contains("\"window_n\":8"));
    }
}
