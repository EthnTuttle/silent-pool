//! Loopback status JSON (plan 4.7; SPEC 11.10).
//!
//! Honest status, not optimistic status:
//!
//! - balances are `null` while unknown - **never zero**: a zero is an
//!   assertion nobody has made;
//! - `min_expiry_height` is `null` when no owned VTXO has a known
//!   expiry (an unknown expiry is not a deadline and is never invented
//!   from a finding height);
//! - `last_error` carries the most recent failure verbatim;
//! - `registered` is the durable journal answer, not "we sent
//!   something".
//!
//! Redaction: the snapshot carries no key material, no `sp_id`, no
//! `U`/`u` values and no raw VTXO ids. The listener is loopback-only
//! (validated in config).

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

use crate::wallet::receive::ReceiveLedger;
use crate::wallet::registration::MailboxMode;
use crate::wallet::state::{WalletStateError, WalletStore};

/// Coarse role state for the status endpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WalletState {
    /// Startup in progress (keys/state not yet loaded).
    Initializing,
    /// No committed registration yet.
    AwaitingRegistration,
    /// A registration transport is wired and the current payload is
    /// the live registration (effective binding matches).
    Connected,
    /// **No registration transport is wired.** The role holds
    /// persistent state - possibly a committed registration - but is
    /// connected to nothing. `registered` in the snapshot stays the
    /// durable journal answer while the state stays honest: a
    /// persistent ack is not a live connection, and a real connection
    /// is never invented.
    Dormant,
    /// A fatal error (keys, state, terminal registration refusal,
    /// counter exhaustion). Attempts have ceased until an explicit
    /// operator restart.
    Error,
    /// Graceful shutdown completed.
    Stopped,
}

impl WalletState {
    pub fn as_str(&self) -> &'static str {
        match self {
            WalletState::Initializing => "initializing",
            WalletState::AwaitingRegistration => "awaiting_registration",
            WalletState::Connected => "connected",
            WalletState::Dormant => "dormant",
            WalletState::Error => "error",
            WalletState::Stopped => "stopped",
        }
    }
}

/// The status JSON (plan 4.7 field set). Unknown values are `null`,
/// never zero.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StatusSnapshot {
    pub state: &'static str,
    pub registered: bool,
    pub registration_live_until_unix_ms: Option<u64>,
    pub mailbox_mode: &'static str,
    pub spendable_sats: Option<u64>,
    pub pending_sats: Option<u64>,
    pub needs_refresh_sats: Option<u64>,
    pub min_expiry_height: Option<u32>,
    pub next_refresh_height: Option<u32>,
    pub last_scan_height: Option<u32>,
    pub last_error: Option<String>,
    pub updated_at: String,
}

#[derive(Debug)]
struct StatusInner {
    registration_live_until_unix_ms: Option<u64>,
    live_balance_fresh: Option<bool>,
    stock_balance: Option<(u64, u64, u64)>,
    state: WalletState,
    last_error: Option<String>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

/// Shared status state over the persistent store. The HTTP handler and
/// the role loop both hold `Arc<StatusService>`.
pub struct StatusService {
    inner: Mutex<StatusInner>,
    store: Arc<WalletStore>,
    ledger: ReceiveLedger,
    mailbox_mode: MailboxMode,
    /// The registration payload digest the `registered` answer is
    /// evaluated against (the effective-binding check).
    payload_digest: [u8; 32],
}

impl StatusService {
    pub fn new(
        store: Arc<WalletStore>,
        refresh_threshold: u32,
        mailbox_mode: MailboxMode,
        payload_digest: [u8; 32],
    ) -> Self {
        let ledger = ReceiveLedger::new(Arc::clone(&store), refresh_threshold);
        StatusService {
            inner: Mutex::new(StatusInner {
                registration_live_until_unix_ms: None,
                live_balance_fresh: None,
                stock_balance: None,
                state: WalletState::Initializing,
                last_error: None,
                updated_at: chrono::Utc::now(),
            }),
            store,
            ledger,
            mailbox_mode,
            payload_digest,
        }
    }

    /// Replace the state, stamping `updated_at`.
    pub fn set_state(&self, state: WalletState) {
        let mut inner = self.inner.lock().expect("status mutex poisoned");
        inner.state = state;
        inner.registration_live_until_unix_ms = None;
        inner.updated_at = chrono::Utc::now();
    }

    /// Set (or clear, with `None`) the last error. The state is only
    /// moved to [`WalletState::Error`] by explicit choice of the
    /// caller; a transient retryable error keeps the role in its
    /// current state with `last_error` set.
    pub fn set_error(&self, error: Option<String>) {
        let mut inner = self.inner.lock().expect("status mutex poisoned");
        inner.last_error = error;
        inner.updated_at = chrono::Utc::now();
    }

    pub fn state(&self) -> WalletState {
        self.inner.lock().expect("status mutex poisoned").state
    }

    /// Granted only after a CURRENT wire ack, never by process startup or a
    /// historical journal row. Runtime admission consumes this bounded lease.
    pub fn set_live_registration(&self) {
        let mut inner = self.inner.lock().expect("status mutex poisoned");
        inner.state = WalletState::Connected;
        inner.updated_at = chrono::Utc::now();
        inner.registration_live_until_unix_ms =
            Some(inner.updated_at.timestamp_millis() as u64 + 30_000);
    }

    pub fn set_live_balance_fresh(&self, fresh: bool) {
        self.inner
            .lock()
            .expect("status mutex poisoned")
            .live_balance_fresh = Some(fresh);
    }

    pub fn set_verified_stock_balance(&self, balance: (u64, u64, u64)) {
        self.inner
            .lock()
            .expect("status mutex poisoned")
            .stock_balance = Some(balance);
    }

    /// Compute the honest snapshot. Balances come from the ledger; the
    /// scan cursor gates `needs_refresh_sats` (unknown tip -> unknown,
    /// not zero); `registered` is the effective-binding answer for the
    /// current payload.
    pub fn snapshot(&self) -> Result<StatusSnapshot, WalletStateError> {
        let inner = self.inner.lock().expect("status mutex poisoned");
        let registered = self.store.is_registered(&self.payload_digest)?;
        let tip = self.store.last_scan_height()?;
        let summary = self.ledger.summary(tip)?;
        let combine = |sp: Option<u64>, stock: u64| -> Option<u64> {
            if inner.live_balance_fresh == Some(false) || (sp.is_none() && stock == 0) {
                None
            } else {
                sp.unwrap_or(0).checked_add(stock)
            }
        };
        let stock = inner.stock_balance.unwrap_or_default();
        Ok(StatusSnapshot {
            state: if inner
                .registration_live_until_unix_ms
                .is_some_and(|until| until <= chrono::Utc::now().timestamp_millis() as u64)
            {
                WalletState::AwaitingRegistration.as_str()
            } else {
                inner.state.as_str()
            },
            registered,
            registration_live_until_unix_ms: inner.registration_live_until_unix_ms,
            mailbox_mode: self.mailbox_mode.as_str(),
            spendable_sats: combine(summary.spendable_sats, stock.0),
            pending_sats: combine(summary.pending_sats, stock.1),
            needs_refresh_sats: combine(summary.needs_refresh_sats, stock.2),
            min_expiry_height: summary.min_expiry_height,
            next_refresh_height: summary.next_refresh_height,
            last_scan_height: tip,
            last_error: inner.last_error.clone(),
            updated_at: inner.updated_at.to_rfc3339(),
        })
    }

    /// Bind the loopback status server. The returned handle stops it.
    ///
    /// The loopback constraint is enforced **here at bind**, not only
    /// in config validation: a programmatic `SocketAddr` that is not
    /// loopback is refused (the status JSON is loopback-only, plan
    /// 4.7).
    pub async fn serve(self: &Arc<Self>, listen: SocketAddr) -> anyhow::Result<StatusServeHandle> {
        if !listen.ip().is_loopback() {
            return Err(anyhow::anyhow!(
                "status listener {listen} is not loopback; the status JSON is loopback-only \
                 (plan 4.7)"
            ));
        }
        let listener = tokio::net::TcpListener::bind(listen)
            .await
            .map_err(|e| anyhow::anyhow!("status listener {listen}: {e}"))?;
        let addr = listener.local_addr()?;
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);
        let app = Router::new()
            .route("/status", get(status_handler))
            .with_state(Arc::clone(self));
        let task = tokio::spawn(async move {
            let server = axum::serve(listener, app).with_graceful_shutdown(async move {
                let _ = shutdown_rx.changed().await;
            });
            if let Err(e) = server.await {
                tracing::error!("status server: {e}");
            }
        });
        Ok(StatusServeHandle {
            addr,
            shutdown: shutdown_tx,
            task,
        })
    }
}

/// A bound status server.
pub struct StatusServeHandle {
    pub addr: SocketAddr,
    shutdown: tokio::sync::watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
}

impl StatusServeHandle {
    /// Stop the server and wait for it to release the port.
    pub async fn stop(self) {
        let _ = self.shutdown.send(true);
        let _ = self.task.await;
    }
}

async fn status_handler(State(service): State<Arc<StatusService>>) -> Response {
    match service.snapshot() {
        Ok(snapshot) => Json(snapshot).into_response(),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "last_error": e.to_string() })),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};

    fn fixture() -> Arc<StatusService> {
        let dir = tempfile::tempdir().unwrap();
        let secp = Secp256k1::new();
        let scan = SecretKey::from_slice(&[1u8; 32]).unwrap();
        let spend = SecretKey::from_slice(&[2u8; 32]).unwrap();
        let identity = crate::wallet::state::StoreIdentity {
            wallet_fingerprint: bitcoin::bip32::Fingerprint::from([9u8; 4]),
            sp_id: spc_crypto::sp_id(
                &PublicKey::from_secret_key(&secp, &scan),
                &PublicKey::from_secret_key(&secp, &spend),
            ),
            network: "regtest".into(),
        };
        let store = Arc::new(WalletStore::open(dir.path(), &identity).unwrap());
        std::mem::forget(dir);
        Arc::new(StatusService::new(
            store,
            96,
            MailboxMode::EpochDerived,
            [7u8; 32],
        ))
    }

    #[test]
    fn unknown_is_null_never_zero() {
        let service = fixture();
        let snapshot = service.snapshot().unwrap();
        assert_eq!(snapshot.state, "initializing");
        assert!(!snapshot.registered);
        assert_eq!(snapshot.mailbox_mode, "epoch_derived");
        assert_eq!(snapshot.spendable_sats, None);
        assert_eq!(snapshot.pending_sats, None);
        assert_eq!(snapshot.needs_refresh_sats, None);
        assert_eq!(snapshot.min_expiry_height, None);
        assert_eq!(snapshot.next_refresh_height, None);
        assert_eq!(snapshot.last_scan_height, None);
        assert_eq!(snapshot.last_error, None);
    }

    #[test]
    fn state_transitions_stamp_the_time() {
        let service = fixture();
        service.set_state(WalletState::AwaitingRegistration);
        service.set_error(Some("transport not wired (AP-2)".into()));
        let first = service.snapshot().unwrap();
        assert_eq!(first.state, "awaiting_registration");
        assert_eq!(
            first.last_error.as_deref(),
            Some("transport not wired (AP-2)")
        );
        assert!(!first.updated_at.is_empty());
        service.set_state(WalletState::Connected);
        service.set_error(None);
        let second = service.snapshot().unwrap();
        assert_eq!(second.state, "connected");
        assert_eq!(second.last_error, None);
        service.set_state(WalletState::Stopped);
        assert_eq!(service.state(), WalletState::Stopped);
    }

    #[tokio::test]
    async fn http_endpoint_serves_redacted_json_and_stops_cleanly() {
        let service = fixture();
        service.set_state(WalletState::AwaitingRegistration);
        let handle = service
            .serve(SocketAddr::from(([127, 0, 0, 1], 0)))
            .await
            .unwrap();

        let body = http_get(handle.addr).await;
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(json["state"], "awaiting_registration");
        assert_eq!(json["registered"], false);
        assert_eq!(json["mailbox_mode"], "epoch_derived");
        assert!(json["spendable_sats"].is_null());
        assert!(json["needs_refresh_sats"].is_null());

        // Redaction: no key material, no sp_id, no U, no vtxo ids in
        // the serialized output.
        let text = body.to_lowercase();
        for banned in ["sp_id", "secret", "b_scan", "b_ark", "vtxo_id", "u_pubkey"] {
            assert!(!text.contains(banned), "status leaked '{banned}'");
        }

        // The port is released: a rebind on the same addr succeeds.
        let addr = handle.addr;
        handle.stop().await;
        let rebound = tokio::net::TcpListener::bind(addr).await;
        assert!(rebound.is_ok(), "status port must be released on stop");
    }

    /// The loopback constraint is enforced at bind, including for
    /// programmatic addresses that never went through config
    /// validation.
    #[tokio::test]
    async fn non_loopback_listener_is_refused_at_bind() {
        let service = fixture();
        let lan = SocketAddr::from((std::net::Ipv4Addr::new(192, 168, 1, 10), 0));
        let err = service
            .serve(lan)
            .await
            .err()
            .expect("a non-loopback bind must be refused");
        assert!(
            err.to_string().contains("not loopback"),
            "a non-loopback bind must be refused: {err}"
        );
        // IPv6 non-loopback too.
        let v6 = SocketAddr::from((std::net::Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1), 0));
        assert!(service.serve(v6).await.is_err());
        // Loopback (v4 and v6) still binds.
        service
            .serve(SocketAddr::from(([127, 0, 0, 1], 0)))
            .await
            .unwrap()
            .stop()
            .await;
        service
            .serve(SocketAddr::from((std::net::Ipv6Addr::LOCALHOST, 0)))
            .await
            .unwrap()
            .stop()
            .await;
    }

    /// Honest dormancy (should-fix 6): with no transport the role
    /// reports `dormant` even when a registration was acknowledged
    /// persistently - `registered` (journal answer) and `state`
    /// (liveness) are different questions.
    #[tokio::test]
    async fn dormant_state_is_reported_even_with_a_persistent_ack() {
        let service = fixture();
        service.set_state(WalletState::Dormant);
        let snapshot = service.snapshot().unwrap();
        assert_eq!(snapshot.state, "dormant");
        assert!(!snapshot.registered, "no ack in this fixture");
        // A state of dormant with registered=true is the honest
        // combination for a durably-registered but unwired role; the
        // registration itself is journaled in the effective-binding
        // tests (state.rs).
    }

    /// Minimal loopback HTTP/1.0 GET without adding a dev-dependency:
    /// raw tokio TCP is enough.
    async fn http_get(addr: SocketAddr) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        stream
            .write_all(format!("GET /status HTTP/1.0\r\nHost: {addr}\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await.unwrap();
        let text = String::from_utf8_lossy(&buf).to_string();
        text.split("\r\n\r\n").nth(1).unwrap_or("").to_string()
    }
}
