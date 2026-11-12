//! Read-only HTTP API (SPEC §6.1).
//!
//! Public endpoints expose pool-wide data only. Anything per-identity needs a
//! BIP 340 signature by the miner's `b_scan` over a one-time nonce; otherwise
//! the API is an attribution oracle for anyone holding a public SP address.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use bitcoin::secp256k1::{schnorr, Message, Secp256k1};
use serde::Deserialize;
use serde_json::{json, Value};
use spc_crypto::{parse_sp_address, Network};
use thiserror::Error;
use tokio::sync::Mutex;

pub const CHALLENGE_TTL: Duration = Duration::from_secs(60);
#[derive(Debug)]
pub struct AggregateOrderUnavailable;

/// Content type of the Prometheus text exposition format 0.0.4.
pub const METRICS_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// The API's backing data. Implementations are read-only.
#[async_trait]
pub trait ApiBackend: Send + Sync + 'static {
    async fn pool(&self) -> Value;
    async fn window(&self) -> Value;
    async fn blocks(&self, limit: usize) -> Vec<Value>;
    async fn liability_sats(&self) -> u64;
    async fn capacity(&self) -> Value;
    async fn me(&self, sp_id: &[u8; 32]) -> Option<Value>;
    async fn audit(&self, sp_id: &[u8; 32], block_hash: &str) -> Option<Value>;
    async fn aggregates(&self, _sp_id: &[u8; 32]) -> Option<Value> {
        None
    }
    async fn aggregate_receipt(&self, _sp_id: &[u8; 32], _order_id: &[u8; 16]) -> Option<Vec<u8>> {
        None
    }
    async fn aggregate_order(
        &self,
        _sp_id: &[u8; 32],
        _order_id: &[u8; 16],
    ) -> Result<Option<Vec<u8>>, AggregateOrderUnavailable> {
        Err(AggregateOrderUnavailable)
    }
    /// Rendered Prometheus 0.0.4 text for `/metrics`. A failed source omits
    /// its family and is counted in `spc_metrics_scrape_errors_total`
    /// (SPEC §9); absent means unknown, never zero.
    async fn metrics(&self) -> String;
}

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("missing or malformed Authorization header")]
    Header,
    #[error("unknown or expired challenge")]
    Challenge,
    #[error("invalid identity")]
    Identity,
    #[error("bad signature")]
    Signature,
}

struct Challenge {
    issued: Instant,
    used: bool,
}

pub struct ApiState<B: ApiBackend> {
    backend: Arc<B>,
    network: Network,
    challenges: Mutex<HashMap<String, Challenge>>,
}

impl<B: ApiBackend> ApiState<B> {
    pub fn new(backend: Arc<B>, network: Network) -> Self {
        Self {
            backend,
            network,
            challenges: Mutex::new(HashMap::new()),
        }
    }

    pub async fn issue_challenge(&self) -> String {
        use bitcoin::hashes::{sha256, Hash};
        let mut bytes = [0u8; 16];
        // The nonce only needs to be unguessable and one-time; a fresh CSPRNG
        // draw is enough. If it fails, fall back to hashing the clock (the
        // challenge is still single-use and short-lived).
        if getrandom(&mut bytes).is_err() {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            bytes.copy_from_slice(&sha256::Hash::hash(&now.to_le_bytes()).to_byte_array()[..16]);
        }
        let nonce = hex(&bytes);
        let mut map = self.challenges.lock().await;
        map.retain(|_, c| !c.used && c.issued.elapsed() < CHALLENGE_TTL);
        map.insert(
            nonce.clone(),
            Challenge {
                issued: Instant::now(),
                used: false,
            },
        );
        nonce
    }

    pub async fn authenticate(&self, headers: &HeaderMap) -> Result<[u8; 32], AuthError> {
        let header = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .ok_or(AuthError::Header)?;
        let mut parts = header.split_whitespace();
        if parts.next() != Some("SPc") {
            return Err(AuthError::Header);
        }
        let addr = parts.next().ok_or(AuthError::Header)?;
        let nonce = parts.next().ok_or(AuthError::Header)?;
        let sig_hex = parts.next().ok_or(AuthError::Header)?;
        if parts.next().is_some() {
            return Err(AuthError::Header);
        }

        let address = parse_sp_address(addr, self.network).map_err(|_| AuthError::Identity)?;
        let sp_id = spc_crypto::sp_id(&address.b_scan, &address.b_spend);

        {
            let mut map = self.challenges.lock().await;
            let challenge = map.get_mut(nonce).ok_or(AuthError::Challenge)?;
            if challenge.used || challenge.issued.elapsed() >= CHALLENGE_TTL {
                return Err(AuthError::Challenge);
            }
            challenge.used = true;
        }

        let sig_bytes = decode_hex(sig_hex).ok_or(AuthError::Signature)?;
        let sig = schnorr::Signature::from_slice(&sig_bytes).map_err(|_| AuthError::Signature)?;
        let msg = api_message(nonce);
        let (xonly, _) = address.b_scan.x_only_public_key();
        Secp256k1::verification_only()
            .verify_schnorr(&sig, &Message::from_digest(msg), &xonly)
            .map_err(|_| AuthError::Signature)?;
        Ok(sp_id)
    }
}

bitcoin::hashes::sha256t_hash_newtype! {
    /// Tag for the API challenge signature.
    pub struct ApiTag = hash_str("SPc/Api");
    /// `hash_SPc/Api(nonce bytes)`.
    pub struct ApiHash(_);
}

/// BIP 340 message for the API challenge: `hash_SPc/Api(nonce bytes)`.
pub fn api_message(nonce: &str) -> [u8; 32] {
    use bitcoin::hashes::{Hash, HashEngine};
    let mut eng = ApiHash::engine();
    eng.input(nonce.as_bytes());
    ApiHash::from_engine(eng).to_byte_array()
}

/// Build the router. The caller supplies a backend and the pool network.
pub fn router<B: ApiBackend>(backend: Arc<B>, network: Network) -> Router {
    let state = Arc::new(ApiState::new(backend, network));
    Router::new()
        .route("/api/pool", get(pool::<B>))
        .route("/api/pool/window", get(window::<B>))
        .route("/api/pool/blocks", get(blocks::<B>))
        .route("/api/pool/liability", get(liability::<B>))
        .route("/api/pool/capacity", get(capacity::<B>))
        .route("/api/sp/challenge", post(challenge::<B>))
        .route("/api/sp/me", get(me::<B>))
        .route("/api/sp/me/audit/{block_hash}", get(audit::<B>))
        .route("/api/sp/me/ark-aggregates", get(aggregates::<B>))
        .route(
            "/api/sp/me/ark-aggregates/{order_id}/order",
            get(aggregate_order::<B>),
        )
        .route(
            "/api/sp/me/ark-aggregates/{order_id}/receipt",
            get(aggregate_receipt::<B>),
        )
        .route("/healthz", get(healthz))
        .route("/metrics", get(metrics))
        .with_state(state)
}

async fn pool<B: ApiBackend>(State(s): State<Arc<ApiState<B>>>) -> Json<Value> {
    Json(s.backend.pool().await)
}
async fn window<B: ApiBackend>(State(s): State<Arc<ApiState<B>>>) -> Json<Value> {
    Json(s.backend.window().await)
}
async fn capacity<B: ApiBackend>(State(s): State<Arc<ApiState<B>>>) -> Json<Value> {
    Json(s.backend.capacity().await)
}
async fn liability<B: ApiBackend>(State(s): State<Arc<ApiState<B>>>) -> Json<Value> {
    Json(json!({ "sats": s.backend.liability_sats().await }))
}
async fn blocks<B: ApiBackend>(
    State(s): State<Arc<ApiState<B>>>,
    Query(q): Query<BlocksQuery>,
) -> Json<Value> {
    let limit = q.limit.unwrap_or(50).min(500);
    Json(json!({ "blocks": s.backend.blocks(limit).await }))
}
async fn challenge<B: ApiBackend>(State(s): State<Arc<ApiState<B>>>) -> Json<Value> {
    Json(json!({ "nonce": s.issue_challenge().await, "expires_in": CHALLENGE_TTL.as_secs() }))
}
async fn me<B: ApiBackend>(State(s): State<Arc<ApiState<B>>>, headers: HeaderMap) -> Response {
    match s.authenticate(&headers).await {
        Ok(sp_id) => match s.backend.me(&sp_id).await {
            Some(v) => Json(v).into_response(),
            None => (
                StatusCode::NOT_FOUND,
                Json(json!({"error":"unknown identity"})),
            )
                .into_response(),
        },
        Err(e) => auth_failure(e),
    }
}
async fn audit<B: ApiBackend>(
    State(s): State<Arc<ApiState<B>>>,
    headers: HeaderMap,
    Path(block_hash): Path<String>,
) -> Response {
    match s.authenticate(&headers).await {
        Ok(sp_id) => match s.backend.audit(&sp_id, &block_hash).await {
            Some(v) => Json(v).into_response(),
            None => (
                StatusCode::NOT_FOUND,
                Json(json!({"error":"no such block"})),
            )
                .into_response(),
        },
        Err(e) => auth_failure(e),
    }
}
async fn aggregates<B: ApiBackend>(
    State(s): State<Arc<ApiState<B>>>,
    headers: HeaderMap,
) -> Response {
    match s.authenticate(&headers).await {
        Ok(id)=>match s.backend.aggregates(&id).await {
            Some(value)=>Json(value).into_response(),
            None=>(StatusCode::SERVICE_UNAVAILABLE,Json(json!({"schema":"spc.aggregate-status/1","error":"status_unknown","keep_watches":true}))).into_response(),
        },
        Err(e)=>auth_failure(e),
    }
}
async fn aggregate_receipt<B: ApiBackend>(
    State(s): State<Arc<ApiState<B>>>,
    headers: HeaderMap,
    Path(order_id): Path<String>,
) -> Response {
    let identity = match s.authenticate(&headers).await {
        Ok(id) => id,
        Err(e) => return auth_failure(e),
    };
    let Some(order) = decode_hex(&order_id).and_then(|v| v.try_into().ok()) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"invalid_order_id"})),
        )
            .into_response();
    };
    match s.backend.aggregate_receipt(&identity, &order).await {
        Some(bytes) if bytes.len() <= 4 * 1024 * 1024 => (
            [(axum::http::header::CONTENT_TYPE, "application/octet-stream")],
            bytes,
        )
            .into_response(),
        _ => (
            StatusCode::NOT_FOUND,
            Json(json!({"error":"delivery_proof_unknown","keep_watches":true})),
        )
            .into_response(),
    }
}
async fn aggregate_order<B: ApiBackend>(
    State(s): State<Arc<ApiState<B>>>,
    headers: HeaderMap,
    Path(order_id): Path<String>,
) -> Response {
    let identity = match s.authenticate(&headers).await {
        Ok(id) => id,
        Err(e) => return auth_failure(e),
    };
    let order: Option<[u8; 16]> = if order_id.len() == 32 {
        decode_hex(&order_id).and_then(|v| v.try_into().ok())
    } else {
        None
    };
    let Some(order) = order else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"invalid_order_id","keep_watches":true})),
        )
            .into_response();
    };
    match s.backend.aggregate_order(&identity, &order).await {
        Ok(Some(bytes))
            if (spc_ext::ArkSendOrder::MIN_ENCODED_LEN
                ..=spc_ext::ArkSendOrder::MAX_ENCODED_LEN)
                .contains(&bytes.len()) =>
        {
            (
                [
                    (axum::http::header::CONTENT_TYPE, "application/octet-stream"),
                    (axum::http::header::CACHE_CONTROL, "no-store"),
                ],
                bytes,
            )
                .into_response()
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error":"order_unknown","keep_watches":true})),
        )
            .into_response(),
        _ => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error":"order_unavailable","keep_watches":true})),
        )
            .into_response(),
    }
}
async fn healthz() -> Json<Value> {
    Json(json!({ "ok": true }))
}

/// Prometheus text format 0.0.4 (SPEC §6.1, §9). The backend renders from
/// live sources; there is no canned payload.
async fn metrics<B: ApiBackend>(State(s): State<Arc<ApiState<B>>>) -> Response {
    let text = s.backend.metrics().await;
    (
        [(axum::http::header::CONTENT_TYPE, METRICS_CONTENT_TYPE)],
        text,
    )
        .into_response()
}

fn auth_failure(e: AuthError) -> Response {
    let status = match e {
        AuthError::Signature | AuthError::Identity => StatusCode::UNAUTHORIZED,
        _ => StatusCode::UNAUTHORIZED,
    };
    (status, Json(json!({ "error": e.to_string() }))).into_response()
}

#[derive(Deserialize)]
struct BlocksQuery {
    limit: Option<usize>,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn decode_hex(s: &str) -> Option<Vec<u8>> {
    ::hex::decode(s).ok()
}

fn getrandom(buf: &mut [u8]) -> Result<(), ()> {
    use rand::RngCore;
    rand::rngs::OsRng.fill_bytes(buf);
    Ok(())
}

/// Parse a `host:port` listen address for the API listener.
pub fn parse_listen(s: &str) -> Result<SocketAddr, std::net::AddrParseError> {
    s.parse()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::secp256k1::{Keypair, SecretKey};
    use spc_crypto::address::encode_sp_address;

    struct Memory {
        pool: Value,
    }

    impl Memory {
        fn bare() -> Self {
            Self {
                pool: json!({"network":"regtest"}),
            }
        }
    }

    #[async_trait]
    impl ApiBackend for Memory {
        async fn pool(&self) -> Value {
            self.pool.clone()
        }
        async fn window(&self) -> Value {
            json!({"t":0})
        }
        async fn blocks(&self, _limit: usize) -> Vec<Value> {
            vec![]
        }
        async fn liability_sats(&self) -> u64 {
            42
        }
        async fn capacity(&self) -> Value {
            json!({"budget_wu":50000})
        }
        async fn me(&self, _sp_id: &[u8; 32]) -> Option<Value> {
            Some(json!({"pending":0}))
        }
        async fn audit(&self, _sp_id: &[u8; 32], _block: &str) -> Option<Value> {
            Some(json!({"slices":[]}))
        }
        async fn aggregates(&self, sp_id: &[u8; 32]) -> Option<Value> {
            Some(
                json!({"schema":"spc.aggregate-status/1","owner":hex(sp_id),"wallet_received":null}),
            )
        }
        async fn metrics(&self) -> String {
            crate::metrics::render(&crate::metrics::MetricsSnapshot::default())
        }
    }

    fn keys(seed: u8) -> (SecretKey, SecretKey) {
        (
            SecretKey::from_slice(&[seed; 32]).unwrap(),
            SecretKey::from_slice(&[seed.wrapping_add(1); 32]).unwrap(),
        )
    }

    fn address(seed: u8) -> String {
        let secp = Secp256k1::new();
        let (scan, spend) = keys(seed);
        encode_sp_address(
            Network::Testnet,
            &bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &scan),
            &bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &spend),
        )
        .unwrap()
    }

    fn sign(scan: &SecretKey, nonce: &str) -> String {
        let secp = Secp256k1::new();
        let kp = Keypair::from_secret_key(&secp, scan);
        let msg = Message::from_digest(api_message(nonce));
        let sig = secp.sign_schnorr_with_rng(&msg, &kp, &mut rand::thread_rng());
        hex(sig.as_ref())
    }

    #[tokio::test]
    async fn auth_requires_a_fresh_challenge_and_a_good_signature() {
        let state = ApiState::new(Arc::new(Memory::bare()), Network::Testnet);
        let (scan, _) = keys(3);
        let addr = address(3);
        let nonce = state.issue_challenge().await;
        let sig = sign(&scan, &nonce);

        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            format!("SPc {addr} {nonce} {sig}").parse().unwrap(),
        );
        let sp_id = state.authenticate(&headers).await.unwrap();
        assert_eq!(sp_id.len(), 32);

        // The nonce is one-time.
        assert!(matches!(
            state.authenticate(&headers).await,
            Err(AuthError::Challenge)
        ));

        // A new nonce with the old signature fails.
        let nonce2 = state.issue_challenge().await;
        headers.insert(
            axum::http::header::AUTHORIZATION,
            format!("SPc {addr} {nonce2} {sig}").parse().unwrap(),
        );
        assert!(matches!(
            state.authenticate(&headers).await,
            Err(AuthError::Signature)
        ));
    }

    #[tokio::test]
    async fn aggregate_status_requires_fresh_owner_proof_and_does_not_claim_wallet_receipt() {
        use axum::body::Body;
        use http_body_util::BodyExt;
        use tower::ServiceExt;
        let app = router(Arc::new(Memory::bare()), Network::Testnet);
        let get = || {
            axum::http::Request::builder()
                .uri("/api/sp/me/ark-aggregates")
                .body(Body::empty())
                .unwrap()
        };
        assert_eq!(
            app.clone().oneshot(get()).await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );
        let challenge = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/api/sp/challenge")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = challenge.into_body().collect().await.unwrap().to_bytes();
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        let nonce = value["nonce"].as_str().unwrap();
        let (scan, spend) = keys(3);
        let addr = address(3);
        let signature = sign(&scan, nonce);
        let auth = format!("SPc {addr} {nonce} {signature}");
        let request = || {
            axum::http::Request::builder()
                .uri("/api/sp/me/ark-aggregates")
                .header("Authorization", &auth)
                .body(Body::empty())
                .unwrap()
        };
        let response = app.clone().oneshot(request()).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        let secp = Secp256k1::new();
        let owner = spc_crypto::sp_id(
            &bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &scan),
            &bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &spend),
        );
        assert_eq!(body["owner"], hex(&owner));
        assert!(body["wallet_received"].is_null());
        assert_eq!(
            app.oneshot(request()).await.unwrap().status(),
            StatusCode::UNAUTHORIZED,
            "captured proof cannot authorize another private read"
        );
    }

    /// Router-level: `/api/pool` serves the backend's endpoints field
    /// verbatim - onion and clearnet alike (SPEC §3.8, §6.1).
    #[tokio::test]
    async fn pool_route_serves_endpoints_verbatim() {
        use axum::body::Body;
        use http_body_util::BodyExt;
        use tower::ServiceExt;

        let app = router(
            Arc::new(Memory {
                pool: json!({"endpoints": ["sv2://abc.onion:34254", "sv2://pool.example:34254"]}),
            }),
            Network::Testnet,
        );
        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/pool")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            body["endpoints"],
            json!(["sv2://abc.onion:34254", "sv2://pool.example:34254"])
        );
    }

    #[test]
    fn api_message_is_stable() {
        // A regression here would silently break every agent's signature.
        let msg = api_message("00112233445566778899aabbccddeeff");
        assert_eq!(
            hex(&msg),
            "59ffe830c5807ba676a5971c3abb2d78149ed04e9712b97d519a037229020dd7"
        );
    }

    /// Router-level: `/metrics` serves real Prometheus 0.0.4 text with the
    /// 0.0.4 content type, not a placeholder.
    #[tokio::test]
    async fn metrics_route_serves_prometheus_text() {
        use axum::body::Body;
        use http_body_util::BodyExt;
        use tower::ServiceExt;

        let app = router(Arc::new(Memory::bare()), Network::Testnet);
        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/metrics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(axum::http::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
            Some(METRICS_CONTENT_TYPE)
        );
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(text.contains("# TYPE spc_metrics_scrape_errors_total counter\n"));
        assert!(!text.contains("not yet wired"));
        assert!(!text.contains("placeholder"));
    }
}
