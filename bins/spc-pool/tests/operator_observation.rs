//! EXP-OP targeted tests (contract §4.4, OP-1/OP-2/OP-3).
//!
//! Router- and DTO-level coverage runs fully without a database through
//! the `ObservationSource` trait; the store-boundary cases run against a
//! real database under the existing `SPC_TEST_DATABASE_URL` convention
//! (the e2e harness sets it; the EXP-OP run points it at a dedicated
//! throwaway Postgres, so in that mode they always execute). Every
//! fixture value is a tiny TESTFIXTURES label, not earnings.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use spc_pool::api::{router as public_router, ApiBackend};
use spc_pool::config::{Config, ConfigError};
use spc_pool::operator::{
    router as operator_router, ObsErrorCode, ObservationFacts, ObservationSource, OperatorBackend,
    OperatorServer, SourceBlockRow, RECENT_BLOCKS_BOUND,
};
use spc_store::postgres::{LiabilitySnapshot, StoreError};
use spc_store::Store;
use tower::ServiceExt;

// ---------------------------------------------------------------- helpers

fn facts() -> ObservationFacts {
    ObservationFacts {
        network: "regtest".into(),
        window_n: 16,
        work_profile: serde_json::json!({
            "work_unit_version": 1,
            "window_rule": "share_work_legacy",
            "window_n": 16,
        }),
    }
}

/// A cloneable failure recipe: builds a fresh `StoreError` per read so a
/// scripted source can serve repeated polls (a real `StoreError` wraps a
/// sqlx error and is not `Clone`).
#[derive(Clone)]
enum Fail {
    /// A database error whose display carries `text`, the way a real
    /// sqlx error can carry a DSN, secret or table name.
    Db(String),
    /// A missing-column error: the canonical `schema_mismatch` shape.
    ColumnNotFound,
    Timeout,
    Invalid(&'static str),
}

impl Fail {
    fn build(&self) -> StoreError {
        match self {
            Fail::Db(text) => StoreError::Db(sqlx::Error::Io(std::io::Error::other(text.clone()))),
            Fail::ColumnNotFound => {
                StoreError::Db(sqlx::Error::ColumnNotFound("pending_balance".into()))
            }
            Fail::Timeout => StoreError::Timeout(std::time::Duration::from_secs(3)),
            Fail::Invalid(msg) => StoreError::Invalid(msg),
        }
    }
}

/// A scriptable source: returns fixed results or the given failure
/// recipes.
struct FixedSource {
    liability: Result<LiabilitySnapshot, Fail>,
    payouts: Result<Vec<(String, u64)>, Fail>,
    blocks: Result<Vec<SourceBlockRow>, Fail>,
}

impl Default for FixedSource {
    fn default() -> Self {
        Self {
            liability: Ok(LiabilitySnapshot {
                pending_balance_sats: 7,
                ark_due_sats: 3,
                total_sats: 10,
                as_of: chrono::Utc::now(),
            }),
            payouts: Ok(vec![("scheduled".into(), 1)]),
            blocks: Ok(vec![SourceBlockRow {
                block_hash: vec![0xaa; 32],
                height: 42,
                status: "confirmed".into(),
                settled_at: None,
            }]),
        }
    }
}

impl ObservationSource for FixedSource {
    async fn liability_snapshot(&self) -> spc_store::postgres::Result<LiabilitySnapshot> {
        self.liability.clone().map_err(|f| f.build())
    }
    async fn ark_payouts_by_status(&self) -> spc_store::postgres::Result<Vec<(String, u64)>> {
        self.payouts.clone().map_err(|f| f.build())
    }
    async fn recent_blocks(&self, _limit: i64) -> spc_store::postgres::Result<Vec<SourceBlockRow>> {
        // Deliberately ignores `limit`: the DTO's own bound must hold
        // even against a source that misbehaves.
        self.blocks.clone().map_err(|f| f.build())
    }
}

async fn get(app: &axum::Router, path: &str) -> StatusCode {
    let response = app
        .clone()
        .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    response.status()
}

/// A minimal `ApiBackend` for the public-router scope test (same shape as
/// the in-crate `api.rs` tests).
struct Memory;

#[async_trait::async_trait]
impl ApiBackend for Memory {
    async fn pool(&self) -> Value {
        serde_json::json!({"network": "regtest"})
    }
    async fn window(&self) -> Value {
        serde_json::json!({"slices": []})
    }
    async fn blocks(&self, _limit: usize) -> Vec<Value> {
        Vec::new()
    }
    async fn liability_sats(&self) -> u64 {
        0
    }
    async fn capacity(&self) -> Value {
        serde_json::json!({})
    }
    async fn me(&self, _sp_id: &[u8; 32]) -> Option<Value> {
        None
    }
    async fn audit(&self, _sp_id: &[u8; 32], _block: &str) -> Option<Value> {
        None
    }
    async fn metrics(&self) -> String {
        String::new()
    }
}

// ------------------------------------------------------------- config

const SAMPLE: &str = r#"
[pool]
network = "regtest"
roles = ["front", "payout", "api"]
listen = "127.0.0.1:34254"
authority_public_key = "9auqWEzQDVyd2oe1JVGFLMLHZtCo2FFqZwtKA5gd9xbuEu7PH72"
authority_secret_key = "mkDLTBBRxdBv998612qipDYoTK3YUrqLe8uWw7gu3iXbSrn2n"
pool_payout_descriptor = "tr(tpubD6NzVbkrYhZ4YTest/0/*)"

[template]
source = "mock"

[accounting]
allow_unchecked_chain = true

[sv1]
enabled = false
"#;

/// Absent `[operator]` section: the listener stays off and nothing else
/// changes.
#[test]
fn operator_section_defaults_off() {
    let cfg: Config = toml::from_str(SAMPLE).unwrap();
    cfg.validate().unwrap();
    assert!(cfg.operator.listen.is_none());
}

/// A configured loopback address validates; any non-loopback address or
/// unparseable value is a configuration error.
#[test]
fn operator_listen_must_be_loopback() {
    let parse = |extra: &str| -> Config { toml::from_str(&format!("{SAMPLE}\n{extra}")).unwrap() };
    for good in ["127.0.0.1:18086", "[::1]:18086", "127.8.8.8:18086"] {
        let cfg = parse(&format!("[operator]\nlisten = \"{good}\"\n"));
        cfg.validate().unwrap_or_else(|e| panic!("{good}: {e:?}"));
    }
    for bad in [
        "0.0.0.0:18086",
        "192.168.1.5:18086",
        "[::]:18086",
        "localhost:18086",
    ] {
        let cfg = parse(&format!("[operator]\nlisten = \"{bad}\"\n"));
        let err = cfg.validate().unwrap_err();
        assert!(
            matches!(
                err,
                ConfigError::OperatorNotLoopback(_) | ConfigError::OperatorListen(_)
            ),
            "{bad}: {err:?}"
        );
    }
}

/// `[operator] listen` without the api role fails closed.
#[test]
fn operator_listen_requires_the_api_role() {
    // A standalone sample (no `[api]` default role list overlap with
    // SAMPLE): payout-only roles plus an operator listener.
    let cfg: Config = toml::from_str(
        r#"
[pool]
network = "regtest"
roles = ["payout"]
listen = "127.0.0.1:34254"
authority_public_key = "9auqWEzQDVyd2oe1JVGFLMLHZtCo2FFqZwtKA5gd9xbuEu7PH72"
authority_secret_key = "mkDLTBBRxdBv998612qipDYoTK3YUrqLe8uWw7gu3iXbSrn2n"
pool_payout_descriptor = "tr(tpubD6NzVbkrYhZ4YTest/0/*)"

[template]
source = "mock"

[accounting]
allow_unchecked_chain = true

[operator]
listen = "127.0.0.1:18086"
"#,
    )
    .unwrap();
    assert!(matches!(
        cfg.validate(),
        Err(ConfigError::OperatorRequiresApiRole)
    ));
}

// --------------------------------------------------------------- scope

/// A request to the public listener's router cannot reach any operator
/// route: 404 on all four (and the public surface itself still works).
#[tokio::test]
async fn public_router_has_no_operator_routes() {
    let app = public_router(Arc::new(Memory), spc_crypto::Network::Regtest);
    for path in [
        "/",
        "/operator.css",
        "/operator.js",
        "/api/pool/observation",
    ] {
        assert_eq!(get(&app, path).await, StatusCode::NOT_FOUND, "{path}");
    }
    assert_eq!(get(&app, "/api/pool").await, StatusCode::OK);
    assert_eq!(get(&app, "/healthz").await, StatusCode::OK);
}

/// The operator router serves exactly its four GET routes; unknown paths
/// are 404, POST is refused (405), and a request carrying a body is
/// rejected before routing (400).
#[tokio::test]
async fn operator_router_serves_exactly_four_get_routes() {
    let app: axum::Router = operator_router(OperatorBackend::new(FixedSource::default(), facts()));

    for (path, contains) in [
        ("/", "<!doctype html>"),
        ("/operator.css", "table-wrap"),
        ("/operator.js", "BigInt"),
        ("/api/pool/observation", "spc-pool-observation/v1"),
    ] {
        let response = app
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        // F6: every route, including the observation DTO, is no-store.
        assert_eq!(
            response
                .headers()
                .get(axum::http::header::CACHE_CONTROL)
                .and_then(|v| v.to_str().ok()),
            Some("no-store"),
            "{path}"
        );
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(text.contains(contains), "{path}");
        assert!(!text.contains("\r\nSet-Cookie"));
    }

    assert_eq!(get(&app, "/nope").await, StatusCode::NOT_FOUND);
    assert_eq!(get(&app, "/api/pool").await, StatusCode::NOT_FOUND);
    assert_eq!(get(&app, "/metrics").await, StatusCode::NOT_FOUND);
    assert_eq!(get(&app, "/healthz").await, StatusCode::NOT_FOUND);

    let post = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/pool/observation")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(post.status(), StatusCode::METHOD_NOT_ALLOWED);

    let post_root = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(post_root.status(), StatusCode::METHOD_NOT_ALLOWED);

    let with_body = app
        .oneshot(
            Request::builder()
                .uri("/api/pool/observation")
                .header("content-length", "7")
                .body(Body::from("surprise"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(with_body.status(), StatusCode::BAD_REQUEST);
}

/// The server constructor refuses a non-loopback address independently of
/// config validation, so a programmatic caller cannot bypass the rule.
#[tokio::test]
async fn operator_server_refuses_non_loopback_bind() {
    let backend = OperatorBackend::new(FixedSource::default(), facts());
    for bad in ["0.0.0.0:18086", "192.168.1.5:18086"] {
        let addr: std::net::SocketAddr = bad.parse().unwrap();
        let err = OperatorServer::bind(addr, OperatorBackend::new(FixedSource::default(), facts()))
            .await
            .err()
            .expect("non-loopback bind refused");
        assert!(matches!(
            err,
            spc_pool::operator::OperatorError::NotLoopback(_)
        ));
        // `bind` refuses before touching the socket, so it cannot have
        // bound anything; no connection attempt is made here because a
        // SYN to an arbitrary LAN address may hang.
    }
    // The same backend still binds loopback fine.
    let server = OperatorServer::bind("127.0.0.1:0".parse().unwrap(), backend)
        .await
        .unwrap();
    assert!(server.local_addr().unwrap().ip().is_loopback());
    drop(server);
}

// ----------------------------------------------------------------- DTO

/// OP-2 single snapshot: pending 7, Ark 3 -> exact decimal strings "7",
/// "3", "10", one `as_of`, no errors, and a total strictly below the
/// naive sum of the displayed facts (the page must show "of which",
/// never 13).
#[tokio::test]
async fn liability_serializes_as_exact_decimal_strings() {
    let backend = OperatorBackend::new(FixedSource::default(), facts());
    let dto = backend.observation().await;
    let liab = dto.pending_liability.clone().expect("liability present");
    assert_eq!(liab.pending_balance_sats, "7");
    assert_eq!(liab.ark_due_sats, "3");
    assert_eq!(liab.total_sats, "10");
    assert!(!liab.as_of.is_empty());
    assert!(dto.errors.is_empty());
    assert_ne!(liab.as_of, "");
    let text = serde_json::to_string(&dto).unwrap();
    assert!(text.contains("\"total_sats\":\"10\""));
    assert!(text.contains("\"ark_due_sats\":\"3\""));
}

/// An amount above the JS safe-integer range survives as its exact
/// decimal string.
#[tokio::test]
async fn amounts_beyond_js_safe_integers_survive_as_strings() {
    let source = FixedSource {
        liability: Ok(LiabilitySnapshot {
            pending_balance_sats: 9_007_199_254_740_990,
            ark_due_sats: 3,
            total_sats: 9_007_199_254_740_993,
            as_of: chrono::Utc::now(),
        }),
        ..FixedSource::default()
    };
    let backend = OperatorBackend::new(source, facts());
    let dto = backend.observation().await;
    let liab = dto.pending_liability.unwrap();
    assert_eq!(liab.total_sats, "9007199254740993");
    assert_eq!(
        liab.total_sats.parse::<u128>().unwrap(),
        9_007_199_254_740_993u128
    );
}

/// A failed liability read nulls the whole object and reports only the
/// closed code and source: no DSN, secret, path, table, column, SQL text
/// or row value from the underlying error.
#[tokio::test]
async fn liability_failure_nulls_whole_object_with_closed_code_only() {
    for (fail, code) in [
        (
            Fail::Db("postgres://user:secret@host/db: connection refused".into()),
            ObsErrorCode::StoreUnavailable,
        ),
        (Fail::ColumnNotFound, ObsErrorCode::SchemaMismatch),
        (Fail::Timeout, ObsErrorCode::Timeout),
        (
            Fail::Invalid("negative pending sum"),
            ObsErrorCode::InvalidValue,
        ),
    ] {
        let source = FixedSource {
            liability: Err(fail),
            ..FixedSource::default()
        };
        let backend = OperatorBackend::new(source, facts());
        let dto = backend.observation().await;
        assert!(dto.pending_liability.is_none(), "whole object null");
        assert_eq!(dto.errors.len(), 1);
        assert_eq!(dto.errors[0].field, "pending_liability");
        assert_eq!(dto.errors[0].code, code);
        assert_eq!(dto.errors[0].source, "postgres");
        let text = serde_json::to_string(&dto).unwrap();
        assert!(!text.contains("secret"), "{text}");
        assert!(!text.contains("postgres://"), "{text}");
        assert!(!text.contains("pending_balance"), "{text}");
        assert!(!text.contains("connection refused"), "{text}");
        // The error entry is exactly {field, code, source}.
        let entry = serde_json::to_value(&dto.errors[0]).unwrap();
        let obj = entry.as_object().unwrap();
        assert_eq!(obj.len(), 3);
        assert!(
            obj.contains_key("field") && obj.contains_key("code") && obj.contains_key("source")
        );
    }
}

/// OP-2 invariant failures: a snapshot whose total does not equal its
/// parts, and a total that cannot be represented, both yield `null` plus
/// `snapshot_inconsistent` - never a partial number.
#[tokio::test]
async fn liability_invariant_failure_is_snapshot_inconsistent() {
    // Torn snapshot from the source side.
    let source = FixedSource {
        liability: Ok(LiabilitySnapshot {
            pending_balance_sats: 7,
            ark_due_sats: 3,
            total_sats: 13,
            as_of: chrono::Utc::now(),
        }),
        ..FixedSource::default()
    };
    let backend = OperatorBackend::new(source, facts());
    let dto = backend.observation().await;
    assert!(dto.pending_liability.is_none());
    assert_eq!(dto.errors[0].code, ObsErrorCode::SnapshotInconsistent);

    // Unrepresentable total (pending + Ark overflows u64).
    let source = FixedSource {
        liability: Err(Fail::Invalid(spc_store::postgres::LIABILITY_TOTAL_OVERFLOW)),
        ..FixedSource::default()
    };
    let backend = OperatorBackend::new(source, facts());
    let dto = backend.observation().await;
    assert!(dto.pending_liability.is_none());
    assert_eq!(dto.errors[0].code, ObsErrorCode::SnapshotInconsistent);
}

/// OP-3: at most ten rows, height-descending, exactly the four closed
/// row fields; an empty table is `rows: []`, a failed query nulls the
/// whole object with its code, and `observed_at` is its own timestamp.
#[tokio::test]
async fn recent_blocks_are_bounded_ordered_and_closed() {
    // Twelve rows, ascending: the DTO serves only ten, newest first,
    // even though this source ignores the limit it is given.
    let rows: Vec<SourceBlockRow> = (0..12)
        .map(|i| SourceBlockRow {
            block_hash: vec![i as u8; 32],
            height: 100 + i,
            status: "confirmed".into(),
            settled_at: None,
        })
        .collect();
    let source = FixedSource {
        blocks: Ok(rows),
        ..FixedSource::default()
    };
    let backend = OperatorBackend::new(source, facts());
    let dto = backend.observation().await;
    let blocks = dto.recent_blocks.expect("blocks present");
    assert_eq!(blocks.rows.len(), RECENT_BLOCKS_BOUND as usize);
    let heights: Vec<i32> = blocks.rows.iter().map(|r| r.height).collect();
    let mut sorted = heights.clone();
    sorted.sort_by(|a, b| b.cmp(a));
    assert_eq!(heights, sorted, "height DESC");
    assert_eq!(heights[0], 111);
    for row in &blocks.rows {
        let v = serde_json::to_value(row).unwrap();
        let mut keys: Vec<&str> = v.as_object().unwrap().keys().map(|k| k.as_str()).collect();
        keys.sort_unstable();
        // serde_json orders object keys, so compare as a set: exactly the
        // four closed fields, no fifth.
        assert_eq!(keys, ["block_hash", "height", "settled_at", "status"]);
    }
    assert!(!blocks.observed_at.is_empty());

    // Empty table is a legal successful read.
    let source = FixedSource {
        blocks: Ok(Vec::new()),
        ..FixedSource::default()
    };
    let dto = OperatorBackend::new(source, facts()).observation().await;
    assert_eq!(dto.recent_blocks.as_ref().unwrap().rows.len(), 0);
    assert!(dto.errors.is_empty(), "an empty table is not an error");

    // Failed query (missing column) nulls the whole object.
    let source = FixedSource {
        blocks: Err(Fail::ColumnNotFound),
        ..FixedSource::default()
    };
    let dto = OperatorBackend::new(source, facts()).observation().await;
    assert!(dto.recent_blocks.is_none());
    assert_eq!(dto.errors[0].field, "recent_blocks");
    assert_eq!(dto.errors[0].code, ObsErrorCode::SchemaMismatch);

    // A raw database error carrying table names reports only its closed
    // code; nothing from the error text reaches the DTO.
    let source = FixedSource {
        blocks: Err(Fail::Db("relation \"block_found\" does not exist".into())),
        ..FixedSource::default()
    };
    let dto = OperatorBackend::new(source, facts()).observation().await;
    assert!(dto.recent_blocks.is_none());
    assert_eq!(dto.errors[0].code, ObsErrorCode::StoreUnavailable);
    let text = serde_json::to_string(&dto).unwrap();
    assert!(!text.contains("block_found"), "{text}");
}

/// Payout counts use exactly the three schema statuses, zero-filled; no
/// `reorged` or other status is invented.
#[tokio::test]
async fn ark_payout_counts_are_the_three_statuses_zero_filled() {
    let backend = OperatorBackend::new(FixedSource::default(), facts());
    let dto = backend.observation().await;
    let counts = dto.ark_payouts_by_status.clone().expect("counts present");
    let v = serde_json::to_value(&counts).unwrap();
    assert_eq!(v["scheduled"], 1);
    assert_eq!(v["sent"], 0);
    assert_eq!(v["failed_to_pending"], 0);
    assert_eq!(v.as_object().unwrap().len(), 3);
    let text = serde_json::to_string(&dto).unwrap();
    assert!(!text.contains("reorged"));

    // Failure nulls the object with its closed code.
    let source = FixedSource {
        payouts: Err(Fail::Db("postgres://user:secret@host/db".into())),
        ..FixedSource::default()
    };
    let dto = OperatorBackend::new(source, facts()).observation().await;
    assert!(dto.ark_payouts_by_status.is_none());
    assert_eq!(dto.errors[0].field, "ark_payouts_by_status");
    let text = serde_json::to_string(&dto).unwrap();
    assert!(!text.contains("secret"), "{text}");
}

/// The float stays `not_configured` until AP-4's real source exists.
#[tokio::test]
async fn float_is_not_configured_until_ap4() {
    let backend = OperatorBackend::new(FixedSource::default(), facts());
    let dto = backend.observation().await;
    assert_eq!(dto.float.state, "not_configured");
    assert!(!dto.float.reason.is_empty());
}

/// The DTO shape is closed: the schema name, the config-allowlisted work
/// profile (never `endpoints`), and no identity, order or VTXO field.
#[tokio::test]
async fn dto_shape_is_closed_and_carries_no_identity_fields() {
    let backend = OperatorBackend::new(FixedSource::default(), facts());
    let dto = backend.observation().await;
    assert_eq!(dto.schema, "spc-pool-observation/v1");
    let v = serde_json::to_value(&dto).unwrap();
    let text = serde_json::to_string(&v).unwrap();
    for forbidden in ["endpoints", "sp_id", "vtxo", "order_id", "ark_address"] {
        assert!(!text.contains(forbidden), "{forbidden} leaked: {text}");
    }
    let profile = v["work_profile"].as_object().unwrap();
    for key in profile.keys() {
        assert!(
            spc_pool::operator::WORK_PROFILE_FIELDS.contains(&key.as_str()),
            "non-allowlisted key {key}"
        );
    }
}

// ------------------------------------------------- store-backed (real DB)

/// Isolated fixture database (TESTFIXTURES label in the name).
async fn isolated_fixture() -> Option<(Store, sqlx::PgPool, String)> {
    let url = std::env::var("SPC_TEST_DATABASE_URL").ok()?;
    let base = url.split('?').next().unwrap_or(&url);
    let (head, _) = base.rsplit_once('/').expect("database path in URL");
    let name = format!(
        "expop_testfixtures_{}",
        hex::encode(uuid::Uuid::new_v4().as_bytes())
    );
    let admin = sqlx::PgPool::connect(&format!("{head}/postgres"))
        .await
        .unwrap();
    if sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(&admin)
        .await
        .is_err()
    {
        eprintln!("skip: no CREATE DATABASE privilege on the test server");
        return None;
    }
    let store = Store::connect(&format!("{head}/{name}"))
        .await
        .expect("connect isolated database");
    store.migrate().await.expect("migrate isolated database");
    Some((store, admin, name))
}

fn id() -> [u8; 32] {
    let mut out = [0u8; 32];
    out[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    out[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    out
}

/// The full DTO path against the real store on the seeded contract
/// fixture: pending 7 / Ark 3 / twelve blocks -> ten rows, decimal
/// strings, zero-filled counts, no errors.
#[tokio::test]
async fn real_store_observation_serves_the_contract_fixture() {
    let Some((store, admin, name)) = isolated_fixture().await else {
        eprintln!("skip: SPC_TEST_DATABASE_URL not set");
        return;
    };
    use spc_store::models::*;
    let fingerprint = id();
    store
        .insert_snapshot(&SnapshotRow {
            fingerprint,
            height: 800_000,
            a_send_x: [4; 32],
            distribution_id: None,
            entries: serde_json::json!([]),
            high_water: serde_json::json!({}),
            work_unit_version: WorkUnitVersion::V1,
            work_unit_fingerprint: None,
            window_rule: WindowRule::ShareWorkLegacy,
            window_target: Some("16".into()),
        })
        .await
        .unwrap();
    for (delta,) in [(4i64,), (3,)] {
        let sp_id = id();
        store
            .upsert_identity(&SpIdentityRow {
                sp_id,
                b_scan: vec![2; 33],
                b_spend: vec![3; 33],
                network: 3,
            })
            .await
            .unwrap();
        store
            .settle(
                &Settlement {
                    block_hash: id(),
                    height: 800_000,
                    fingerprint: Some(fingerprint),
                    status: BlockStatus::Confirmed,
                    ledger: vec![LedgerRow {
                        sp_id,
                        rail: LedgerRail::Pending,
                        paid_sats: 0,
                        ark_due_sats: 0,
                        forfeited_sats: 0,
                        pending_delta: delta,
                    }],
                },
                6,
            )
            .await
            .unwrap();
    }
    let sp_id = id();
    store
        .upsert_identity(&SpIdentityRow {
            sp_id,
            b_scan: vec![2; 33],
            b_spend: vec![3; 33],
            network: 3,
        })
        .await
        .unwrap();
    store
        .settle(
            &Settlement {
                block_hash: id(),
                height: 800_001,
                fingerprint: Some(fingerprint),
                status: BlockStatus::Confirmed,
                ledger: vec![LedgerRow {
                    sp_id,
                    rail: LedgerRail::Ark,
                    paid_sats: 0,
                    ark_due_sats: 3,
                    forfeited_sats: 0,
                    pending_delta: 0,
                }],
            },
            6,
        )
        .await
        .unwrap();
    for i in 0..12u32 {
        store
            .settle(
                &Settlement {
                    block_hash: id(),
                    height: 700_000 + i,
                    fingerprint: Some(fingerprint),
                    status: BlockStatus::Confirmed,
                    ledger: vec![],
                },
                6,
            )
            .await
            .unwrap();
    }

    let backend = OperatorBackend::new(store, facts());
    let dto = backend.observation().await;
    assert!(dto.errors.is_empty(), "{:?}", dto.errors);
    let liab = dto.pending_liability.unwrap();
    assert_eq!(liab.pending_balance_sats, "7");
    assert_eq!(liab.ark_due_sats, "3");
    assert_eq!(liab.total_sats, "10");
    let blocks = dto.recent_blocks.unwrap();
    assert_eq!(blocks.rows.len(), 10);
    let heights: Vec<i32> = blocks.rows.iter().map(|r| r.height).collect();
    let mut sorted = heights.clone();
    sorted.sort_by(|a, b| b.cmp(a));
    assert_eq!(heights, sorted);
    // Fifteen blocks total: the ark settle at 800_001, two pending
    // settles both at 800_000, and twelve plain blocks at 700_000..700_011.
    // The bound keeps the ten highest, so 700_000..700_004 fall off.
    assert_eq!(heights[0], 800_001);
    assert_eq!(heights[1], 800_000);
    assert_eq!(heights[2], 800_000);
    assert_eq!(heights[3], 700_011);
    assert_eq!(heights[9], 700_005);
    let counts = dto.ark_payouts_by_status.unwrap();
    let v = serde_json::to_value(counts).unwrap();
    assert_eq!(v["scheduled"], 1);
    assert_eq!(v["sent"], 0);
    assert_eq!(v["failed_to_pending"], 0);
    sqlx::query(&format!("DROP DATABASE {name} WITH (FORCE)"))
        .execute(&admin)
        .await
        .unwrap();
}

/// Secret redaction against a real broken schema: the store connects with
/// a DSN that contains `postgres://user:secret@...`, the schema is
/// broken (tables dropped), and the DTO carries only the closed code and
/// source - no secret, no object names.
///
/// Fable R2 rework: the earlier version obtained its admin pool from
/// `isolated_fixture()` and discarded the created database name, leaking
/// one fixture database per run (only a later manual sweep removed past
/// leaks). This version connects the admin pool directly - no fixture
/// database is created on the way in - records every assertion as a
/// failure string, and runs cleanup on every path (including assertion
/// failure), then verifies by name that neither the database nor the
/// login role it created remains.
#[tokio::test]
async fn broken_schema_with_secret_dsn_leaks_nothing() {
    let Some(url) = std::env::var("SPC_TEST_DATABASE_URL").ok() else {
        eprintln!("skip: SPC_TEST_DATABASE_URL not set");
        return;
    };
    let base = url.split('?').next().unwrap_or(&url);
    let (head, _) = base.rsplit_once('/').expect("database path in URL");
    let admin = sqlx::PgPool::connect(&format!("{head}/postgres"))
        .await
        .unwrap();
    let name = format!(
        "expop_testfixtures_{}",
        hex::encode(uuid::Uuid::new_v4().as_bytes())
    );
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(&admin)
        .await
        .unwrap();
    // A user-named role so the DSN genuinely carries a password-style
    // secret. With trust auth the password is unused but still part of
    // the DSN the store error could otherwise echo.
    let role = format!("expop_testuser_{}", &name[name.len() - 8..]);
    if let Err(e) = sqlx::query(&format!("CREATE ROLE \"{role}\" LOGIN PASSWORD 'secret'"))
        .execute(&admin)
        .await
    {
        // Role creation failed: still drop the database before failing.
        let _ = sqlx::query(&format!("DROP DATABASE {name} WITH (FORCE)"))
            .execute(&admin)
            .await;
        panic!("create role: {e}");
    }
    // Rebuild the DSN from the actual host:port of the test server so the
    // secret genuinely rides in the DSN string.
    let hostport = base
        .trim_start_matches("postgres://")
        .split('/')
        .next()
        .unwrap_or("127.0.0.1:5432")
        .to_string();
    let dsn = format!("postgres://{role}:secret@{hostport}/{name}");
    // Assertions land in `bad` instead of panicking so the cleanup below
    // always runs; they are reported after cleanup.
    let mut bad: Vec<String> = Vec::new();
    match Store::connect(&dsn).await {
        Ok(store) => {
            let backend = OperatorBackend::new(store, facts());
            let dto = backend.observation().await;
            let text = serde_json::to_string(&dto).unwrap();
            if text.contains("secret") {
                bad.push(format!("leaked the DSN secret: {text}"));
            }
            if text.contains(&role) {
                bad.push(format!("leaked the login role: {text}"));
            }
            if text.contains("postgres://") {
                bad.push(format!("leaked a DSN: {text}"));
            }
            if text.contains("pending_balance") {
                bad.push(format!("leaked a table name: {text}"));
            }
            // No `ark_payout` substring check here: the DTO's own contract
            // field name `ark_payouts_by_status` contains it. The redaction
            // property is that the underlying error text (e.g. `relation
            // "ark_payout" does not exist`) never reaches the DTO - covered
            // by the closed {field, code, source} shape asserted below.
            if text.contains("block_found") {
                bad.push(format!("leaked a table name: {text}"));
            }
            if text.contains("sp_identity") {
                bad.push(format!("leaked a table name: {text}"));
            }
            if dto.pending_liability.is_some() {
                bad.push("liability must be null on a broken schema".into());
            }
            if dto.recent_blocks.is_some() {
                bad.push("recent_blocks must be null on a broken schema".into());
            }
            if dto.ark_payouts_by_status.is_some() {
                bad.push("payout counts must be null on a broken schema".into());
            }
            let fields: Vec<&str> = dto.errors.iter().map(|e| e.field).collect();
            if fields
                != [
                    "pending_liability",
                    "ark_payouts_by_status",
                    "recent_blocks",
                ]
            {
                bad.push(format!("unexpected error fields: {fields:?}"));
            }
            for err in &dto.errors {
                if err.source != "postgres" {
                    bad.push(format!("unexpected error source: {}", err.source));
                }
                if !matches!(
                    err.code,
                    ObsErrorCode::SchemaMismatch | ObsErrorCode::StoreUnavailable
                ) {
                    bad.push(format!("unexpected error code: {:?}", err.code));
                }
            }
        }
        Err(e) => bad.push(format!("connect with role dsn: {e}")),
    }
    // Cleanup runs on every path and drops both names this test created.
    let drop_db = sqlx::query(&format!("DROP DATABASE {name} WITH (FORCE)"))
        .execute(&admin)
        .await;
    let drop_role = sqlx::query(&format!("DROP ROLE \"{role}\""))
        .execute(&admin)
        .await;
    // Verify by name that nothing this test created remains (R2).
    let db_left: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_database WHERE datname = $1)")
            .bind(&name)
            .fetch_one(&admin)
            .await
            .unwrap();
    let role_left: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = $1)")
            .bind(&role)
            .fetch_one(&admin)
            .await
            .unwrap();
    assert!(
        bad.is_empty(),
        "redaction assertion failures (cleanup already ran): {bad:?}"
    );
    drop_db.expect("drop fixture database");
    drop_role.expect("drop fixture role");
    assert!(!db_left, "fixture database {name} still exists after drop");
    assert!(!role_left, "fixture role {role} still exists after drop");
}
