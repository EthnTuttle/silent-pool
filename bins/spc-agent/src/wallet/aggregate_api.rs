//! Owner-authenticated SPc-side API transport. A new single-use challenge is
//! obtained for EVERY read; no registration signature or cached header is reused.
use super::WalletKeys;
use anyhow::{ensure, Context};
use bitcoin::secp256k1::{Keypair, Message, Secp256k1, XOnlyPublicKey};
use reqwest::{Client, Url};

pub(super) const MAX_RESPONSE: usize = 4 * 1024 * 1024;
const MAX_ORDER_RESPONSE: usize = spc_ext::ArkSendOrder::MAX_ENCODED_LEN;
pub(super) struct PoolApi {
    client: Client,
    origin: Url,
    pub(super) order_key: XOnlyPublicKey,
    used_nonces: std::collections::VecDeque<String>,
}

impl PoolApi {
    pub(super) fn new(
        config: &crate::config::AggregateApi,
        network: spc_crypto::Network,
    ) -> anyhow::Result<Self> {
        let (origin, order_key) = config.validated(network)?;
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(std::time::Duration::from_secs(5))
            .timeout(std::time::Duration::from_secs(10))
            .build()?;
        Ok(Self {
            client,
            origin,
            order_key,
            used_nonces: Default::default(),
        })
    }

    async fn bounded(response: reqwest::Response, limit: usize) -> anyhow::Result<Vec<u8>> {
        ensure!(
            response.status().is_success(),
            "aggregate API response unavailable"
        );
        ensure!(
            response.content_length().is_none_or(|n| n <= limit as u64),
            "aggregate API body limit"
        );
        let mut response = response;
        let mut body = vec![];
        while let Some(chunk) = response.chunk().await? {
            ensure!(
                body.len()
                    .checked_add(chunk.len())
                    .is_some_and(|n| n <= limit),
                "aggregate API body limit"
            );
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }

    async fn read(
        &mut self,
        path: &str,
        keys: &WalletKeys,
        address: &str,
        limit: usize,
        octet_stream: bool,
    ) -> anyhow::Result<Vec<u8>> {
        ensure!(
            path == "/api/sp/me/ark-aggregates"
                || (path.starts_with("/api/sp/me/ark-aggregates/")
                    && (path.ends_with("/receipt") || path.ends_with("/order"))),
            "aggregate API path not allowed"
        );
        let parsed = spc_crypto::parse_sp_address(address, keys.network)?;
        ensure!(
            spc_crypto::sp_id(&parsed.b_scan, &parsed.b_spend) == keys.sp_id,
            "aggregate API owner mismatch"
        );
        let challenge = self
            .client
            .post(self.origin.join("/api/sp/challenge")?)
            .send()
            .await?;
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Challenge {
            nonce: String,
            expires_in: u64,
        }
        let challenge: Challenge = serde_json::from_slice(&Self::bounded(challenge, 4096).await?)?;
        ensure!(
            challenge.nonce.len() == 32
                && challenge.nonce.bytes().all(|b| b.is_ascii_hexdigit())
                && (1..=60).contains(&challenge.expires_in)
                && !self.used_nonces.contains(&challenge.nonce),
            "aggregate API challenge invalid or replayed"
        );
        self.used_nonces.push_back(challenge.nonce.clone());
        if self.used_nonces.len() > 256 {
            self.used_nonces.pop_front();
        }
        let signature = Secp256k1::new().sign_schnorr_no_aux_rand(
            &Message::from_digest(spc_pool::api::api_message(&challenge.nonce)),
            &Keypair::from_secret_key(&Secp256k1::new(), &keys.b_scan),
        );
        let mut header = reqwest::header::HeaderValue::from_str(&format!(
            "SPc {address} {} {signature}",
            challenge.nonce
        ))?;
        header.set_sensitive(true);
        let response = self
            .client
            .get(self.origin.join(path)?)
            .header(reqwest::header::AUTHORIZATION, header)
            .send()
            .await?;
        if octet_stream {
            ensure!(
                response.status() == reqwest::StatusCode::OK,
                "aggregate order unavailable; retain watch"
            );
            ensure!(
                response
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .is_some_and(|value| value.as_bytes() == b"application/octet-stream"),
                "aggregate order content type mismatch"
            );
            ensure!(
                response
                    .headers()
                    .get(reqwest::header::CACHE_CONTROL)
                    .is_some_and(|value| value.as_bytes() == b"no-store"),
                "aggregate order cache policy mismatch"
            );
        }
        Self::bounded(response, limit).await
    }

    pub(super) async fn status(
        &mut self,
        keys: &WalletKeys,
        address: &str,
    ) -> anyhow::Result<super::aggregate_watch::AggregateStatus> {
        let status: super::aggregate_watch::AggregateStatus = serde_json::from_slice(
            &self
                .read(
                    "/api/sp/me/ark-aggregates",
                    keys,
                    address,
                    MAX_RESPONSE,
                    false,
                )
                .await?,
        )
        .context("invalid aggregate status")?;
        status.validate()?;
        Ok(status)
    }
    pub(super) async fn receipt(
        &mut self,
        keys: &WalletKeys,
        address: &str,
        id: &[u8; 16],
    ) -> anyhow::Result<spc_ext::authorization::ProvedReceipt> {
        let bytes = self
            .read(
                &format!("/api/sp/me/ark-aggregates/{}/receipt", hex::encode(id)),
                keys,
                address,
                MAX_RESPONSE,
                false,
            )
            .await?;
        Ok(spc_ext::authorization::ProvedReceipt::decode(&bytes)?)
    }

    /// Source-bound owner endpoint (ladder 0515a6a8). A missing route, malformed response or
    /// unverified signature is an error; no order is reconstructed from status.
    pub(super) async fn order(
        &mut self,
        keys: &WalletKeys,
        address: &str,
        id: &[u8; 16],
    ) -> anyhow::Result<Vec<u8>> {
        let bytes = self
            .read(
                &format!("/api/sp/me/ark-aggregates/{}/order", hex::encode(id)),
                keys,
                address,
                MAX_ORDER_RESPONSE,
                true,
            )
            .await?;
        let order = spc_ext::ArkSendOrder::decode(&bytes)?;
        ensure!(
            order.encode()? == bytes && order.order_id == *id,
            "aggregate frozen order encoding/association mismatch"
        );
        order.verify(&self.order_key)?;
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        extract::{OriginalUri, State},
        http::{HeaderMap, StatusCode},
        routing::{get, post},
        Json, Router,
    };
    use bitcoin::secp256k1::{PublicKey, SecretKey};
    use std::{
        collections::BTreeSet,
        sync::{Arc, Mutex},
    };

    #[derive(Default)]
    struct Script {
        challenges: usize,
        consumed: BTreeSet<String>,
        headers: Vec<String>,
    }
    async fn challenge(State(script): State<Arc<Mutex<Script>>>) -> Json<serde_json::Value> {
        let mut script = script.lock().unwrap();
        script.challenges += 1;
        let n = if script.challenges == 4 {
            3
        } else {
            script.challenges
        };
        Json(serde_json::json!({"nonce":format!("{n:032x}"),"expires_in":60}))
    }
    async fn read(
        State(script): State<Arc<Mutex<Script>>>,
        OriginalUri(uri): OriginalUri,
        headers: HeaderMap,
    ) -> Result<Vec<u8>, StatusCode> {
        let header = headers
            .get("authorization")
            .and_then(|h| h.to_str().ok())
            .ok_or(StatusCode::UNAUTHORIZED)?;
        let parts: Vec<_> = header.split_whitespace().collect();
        if parts.len() != 4 || parts[0] != "SPc" {
            return Err(StatusCode::UNAUTHORIZED);
        }
        let address = spc_crypto::parse_sp_address(parts[1], spc_crypto::Network::Regtest)
            .map_err(|_| StatusCode::UNAUTHORIZED)?;
        let sig = parts[3]
            .parse::<bitcoin::secp256k1::schnorr::Signature>()
            .map_err(|_| StatusCode::UNAUTHORIZED)?;
        Secp256k1::verification_only()
            .verify_schnorr(
                &sig,
                &Message::from_digest(spc_pool::api::api_message(parts[2])),
                &address.b_scan.x_only_public_key().0,
            )
            .map_err(|_| StatusCode::UNAUTHORIZED)?;
        let mut script = script.lock().unwrap();
        if !script.consumed.insert(parts[2].into()) {
            return Err(StatusCode::UNAUTHORIZED);
        }
        script.headers.push(header.into());
        if uri.path().ends_with("/receipt") {
            return Ok(b"scripted malformed receipt".to_vec());
        }
        Ok(serde_json::to_vec(&serde_json::json!({"schema":"spc.aggregate-status/1","obligations":[],"orders":[],"wallet_received":null,"sent_means":"fetch_and_verify_owned_vtxo_before_closing_watch"})).unwrap())
    }
    fn keys(dir: &std::path::Path) -> (WalletKeys, String) {
        use std::os::unix::fs::PermissionsExt;
        let network = spc_crypto::Network::Regtest;
        let provisioned = spc_crypto::provision(&[41; 64], network).unwrap();
        let paths = [
            dir.join("scan.xprv"),
            dir.join("ark.xprv"),
            dir.join("child.seed"),
        ];
        let values = [
            provisioned.scan_branch.to_string(),
            provisioned.ark_branch.to_string(),
            hex::encode(provisioned.child.seed),
        ];
        for (path, value) in paths.iter().zip(values) {
            std::fs::write(path, value).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        let secp = Secp256k1::new();
        let scan = PublicKey::from_secret_key(
            &secp,
            &spc_crypto::hd::spc_branch_secret(&provisioned.scan_branch).unwrap(),
        );
        let spend = PublicKey::from_secret_key(
            &secp,
            &spc_crypto::hd::spc_branch_secret(&provisioned.spend_branch).unwrap(),
        );
        let address = spc_crypto::address::encode_sp_address(network, &scan, &spend).unwrap();
        let keys = WalletKeys::load(&super::super::KeySources {
            scan_xprv: paths[0].clone(),
            ark_xprv: paths[1].clone(),
            child_seed: paths[2].clone(),
            sp_address: address.clone(),
            network,
        })
        .unwrap();
        (keys, address)
    }
    fn config(origin: String) -> crate::config::AggregateApi {
        crate::config::AggregateApi {
            origin,
            order_pubkey: Keypair::from_secret_key(
                &Secp256k1::new(),
                &SecretKey::from_slice(&[8; 32]).unwrap(),
            )
            .x_only_public_key()
            .0
            .to_string(),
        }
    }

    #[tokio::test]
    async fn every_owner_read_gets_a_new_challenge_and_bad_receipt_does_not_reuse_auth() {
        let dir = tempfile::tempdir().unwrap();
        let (mut keys, address) = keys(dir.path());
        let script = Arc::new(Mutex::new(Script::default()));
        let app = Router::new()
            .route("/api/sp/challenge", post(challenge))
            .route("/api/sp/me/ark-aggregates", get(read))
            .route("/api/sp/me/ark-aggregates/{id}/receipt", get(read))
            .with_state(script.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut api = PoolApi::new(
            &config(format!("http://{}/", listener.local_addr().unwrap())),
            keys.network,
        )
        .unwrap();
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        api.status(&keys, &address).await.unwrap();
        assert!(api.receipt(&keys, &address, &[1; 16]).await.is_err());
        api.status(&keys, &address).await.unwrap();
        assert!(api
            .status(&keys, &address)
            .await
            .unwrap_err()
            .to_string()
            .contains("replayed"));
        assert_eq!(script.lock().unwrap().headers.len(), 3);
        let other = PublicKey::from_secret_key(
            &Secp256k1::new(),
            &SecretKey::from_slice(&[9; 32]).unwrap(),
        );
        let wrong_owner =
            spc_crypto::address::encode_sp_address(keys.network, &other, &keys.b_spend_public)
                .unwrap();
        assert!(api.status(&keys, &wrong_owner).await.is_err());
        assert_eq!(
            script.lock().unwrap().challenges,
            4,
            "wrong owner rejected before challenge request"
        );
        let real_scan = keys.b_scan;
        keys.b_scan = SecretKey::from_slice(&[9; 32]).unwrap();
        assert!(
            api.status(&keys, &address).await.is_err(),
            "wrong signer must not authenticate"
        );
        keys.b_scan = real_scan;
        api.status(&keys, &address).await.unwrap();
        let script = script.lock().unwrap();
        assert_eq!(script.challenges, 6);
        assert_eq!(script.headers.len(), 4);
        assert_eq!(script.headers.iter().collect::<BTreeSet<_>>().len(), 4);
        task.abort();
    }

    #[test]
    fn owner_api_refuses_ambiguous_or_insecure_origins() {
        for origin in [
            "http://example.com/",
            "https://user:pass@example.com/",
            "https://example.com/prefix/",
            "https://example.com/?q=1",
            "https://example.com/#fragment",
        ] {
            assert!(PoolApi::new(&config(origin.into()), spc_crypto::Network::Regtest).is_err());
        }
        assert!(PoolApi::new(
            &config("http://127.0.0.1/".into()),
            spc_crypto::Network::Mainnet
        )
        .is_err());
        assert!(PoolApi::new(
            &config("https://example.com/".into()),
            spc_crypto::Network::Regtest
        )
        .is_ok());
    }

    #[tokio::test]
    async fn assigned_order_endpoint_missing_or_invalid_never_yields_frozen_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let (keys, address) = keys(dir.path());
        // These are refusal fixtures only, not evidence of a successful pool
        // endpoint. Bound source/schema: ladder 0515a6a8 / ae1eaed4.
        for (code, mime, bytes) in [
            (StatusCode::NOT_FOUND, "application/octet-stream", vec![]),
            (
                StatusCode::BAD_REQUEST,
                "application/json",
                br#"{"error":"invalid_order_id","keep_watches":true}"#.to_vec(),
            ),
            (StatusCode::UNAUTHORIZED, "application/json", b"{}".to_vec()),
            (
                StatusCode::NOT_FOUND,
                "application/json",
                br#"{"error":"order_unknown","keep_watches":true}"#.to_vec(),
            ),
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "application/json",
                br#"{"error":"order_unavailable","keep_watches":true}"#.to_vec(),
            ),
            (StatusCode::OK, "application/json", b"{}".to_vec()),
            (
                StatusCode::OK,
                "application/octet-stream",
                vec![0; MAX_ORDER_RESPONSE + 1],
            ),
            (
                StatusCode::OK,
                "application/octet-stream",
                vec![0; MAX_ORDER_RESPONSE],
            ),
        ] {
            let script = Arc::new(Mutex::new(Script::default()));
            let app = Router::new()
                .route("/api/sp/challenge", post(challenge))
                .route(
                    "/api/sp/me/ark-aggregates/{id}/order",
                    get(move || async move {
                        (
                            code,
                            [
                                (axum::http::header::CONTENT_TYPE, mime),
                                (axum::http::header::CACHE_CONTROL, "no-store"),
                            ],
                            bytes,
                        )
                    }),
                )
                .with_state(script.clone());
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let mut api = PoolApi::new(
                &config(format!("http://{}/", listener.local_addr().unwrap())),
                keys.network,
            )
            .unwrap();
            let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            assert!(api.order(&keys, &address, &[3; 16]).await.is_err());
            assert_eq!(script.lock().unwrap().challenges, 1);
            assert_eq!(api.used_nonces.len(), 1);
            task.abort();
        }
    }
}
