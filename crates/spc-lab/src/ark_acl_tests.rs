//! Installed production ACL, real restricted credentials, real worker/PEL.
//! The transport/issuer fixtures isolate ACL behavior; no funded send is claimed.
use super::install_ark_acl;
use secp256k1::{Keypair, Secp256k1, SecretKey};
use spc_ext::authorization::{
    order_digest, AuthorizationGrant, AuthorizationRequest, GRANTS_KEY, REQUESTS_STREAM,
};
use spc_ext::ArkSendOrder;
use spc_pool::ark::send_fence::{CapacityError, SendAuthorization, WalletOperationPermit};
use spc_pool::ark::{
    ArkService, ArkTransport, OrderVerifier, SeenOrders, CONSUMER, GROUP, ORDERS_STREAM,
    RECEIPTS_STREAM,
};
use spc_store::redis::Streams;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

struct AdmissionFixture {
    ready: Arc<AtomicBool>,
    escapes: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl ArkTransport for AdmissionFixture {
    async fn reserve_send_capacity(&self) -> Result<Option<WalletOperationPermit>, CapacityError> {
        // This regression tests installed role permissions. The production
        // permit/deadline branches have separate pool-owned boundary tests.
        Ok(None)
    }

    async fn check_send_readiness(
        &self,
        _: &str,
        _: u64,
        _: Option<&WalletOperationPermit>,
    ) -> Result<(), CapacityError> {
        if self.ready.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(CapacityError::InsufficientFloat)
        }
    }

    async fn send(&self, _: &str, _: u64) -> Result<(Vec<u8>, u32), String> {
        panic!("authorized worker must not use the legacy transport")
    }

    async fn send_authorized(
        &self,
        address: &str,
        sats: u64,
        authorization: &SendAuthorization,
        _: Option<&WalletOperationPermit>,
    ) -> Result<Vec<Vec<u8>>, String> {
        authorization.enter_cosign(address, sats)?;
        self.escapes.fetch_add(1, Ordering::SeqCst);
        Err("ACL fixture: simulated lost post-grant response".into())
    }
}

fn assert_noperm<T: std::fmt::Debug>(result: Result<T, impl std::fmt::Display>) {
    match result {
        Ok(value) => panic!("restricted Ark connection unexpectedly permitted {value:?}"),
        Err(error) => assert!(error.to_string().contains("NOPERM"), "{error}"),
    }
}

/// Run explicitly with --ignored and a freshly started, dedicated Redis URL:
/// SPC_LADDER_ACL_TEST_REDIS_URL=redis://:password@127.0.0.1:port/0.
/// Missing setup fails; shared/nonempty Redis is rejected without flushing it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires an owned empty Redis; installs the actual production Ark ACL"]
async fn installed_ark_acl_allows_cursor_and_held_retry_without_identity_access() {
    let admin_url =
        std::env::var("SPC_LADDER_ACL_TEST_REDIS_URL").expect("dedicated owned Redis URL required");
    let admin_client = redis::Client::open(admin_url.as_str()).unwrap();
    let mut admin_conn = admin_client
        .get_multiplexed_async_connection()
        .await
        .unwrap();
    let size: u64 = redis::cmd("DBSIZE")
        .query_async(&mut admin_conn)
        .await
        .unwrap();
    assert_eq!(
        size, 0,
        "use an empty owned Redis, never clear another fixture"
    );
    let role_password = hex::encode(rand::random::<[u8; 32]>());
    install_ark_acl(&admin_url, &role_password).unwrap();
    let (_, host) = admin_url.split_once('@').unwrap();
    let role_url = format!("redis://ark:{role_password}@{host}");
    let role = Streams::connect(&role_url).await.unwrap();
    let admin = Streams::connect(&admin_url).await.unwrap();
    let mut restricted = redis::Client::open(role_url.as_str())
        .unwrap()
        .get_multiplexed_async_connection()
        .await
        .unwrap();

    let cursor = format!("spc:ark:pending-cursor:{ORDERS_STREAM}:{GROUP}:{CONSUMER}");
    assert_eq!(role.kv_get(&cursor).await.unwrap(), None);
    role.kv_set(&cursor, "0").await.unwrap();
    assert_eq!(role.kv_get(&cursor).await.unwrap().as_deref(), Some("0"));
    // The new selector is exactly GET/SET, not the wider operational selector.
    assert_noperm(role.hset(&format!("{cursor}:hash"), "field", "value").await);
    assert_noperm(role.add(&format!("{cursor}:stream"), b"forbidden").await);
    assert_noperm(
        redis::cmd("DEL")
            .arg(&cursor)
            .query_async::<i64>(&mut restricted)
            .await,
    );
    assert_noperm(
        redis::cmd("ACL")
            .arg("GETUSER")
            .arg("ark")
            .query_async::<redis::Value>(&mut restricted)
            .await,
    );

    for key in [
        "arbitrary:key",
        "spc:ark:pending-cursor",
        "spc:slices",
        "spc:blocks",
        "spc:arkreg",
        "spc:arkreg:requests",
        "spc:arkreg:events",
    ] {
        assert_noperm(role.kv_get(key).await);
        assert_noperm(role.kv_set(key, "forbidden").await);
    }
    for stream in [
        "spc:slices",
        "spc:blocks",
        "spc:arkreg",
        "spc:arkreg:requests",
    ] {
        admin.ensure_group_from(stream, GROUP, "0").await.unwrap();
        admin
            .add(stream, b"identity-bearing fixture")
            .await
            .unwrap();
        assert_noperm(role.read_group(stream, GROUP, CONSUMER, 1).await);
        assert_noperm(role.add(stream, b"forbidden").await);
    }
    assert_noperm(role.add(ORDERS_STREAM, b"forbidden publication").await);

    let key =
        Keypair::from_secret_key(&Secp256k1::new(), &SecretKey::from_slice(&[7; 32]).unwrap());
    let server = key.public_key();
    let address = ark::Address::builder()
        .testnet(true)
        .server_pubkey(server)
        .pubkey_policy(server)
        .delivery(ark::address::VtxoDelivery::ServerMailbox {
            blinded_id: ark::mailbox::BlindedMailboxIdentifier::from_pubkey(server),
        })
        .into_address()
        .unwrap();
    let ark_id = <[u8; 4]>::from(address.ark_id());
    let mut order = ArkSendOrder {
        order_id: [0x42; 16],
        ark_address: address.to_string(),
        sats: 500_000,
        not_before_height: 1,
        sig: [0; 64],
    };
    order.sign(&key).unwrap();
    let frozen = order.encode().unwrap();
    let ready = Arc::new(AtomicBool::new(false));
    let escapes = Arc::new(AtomicUsize::new(0));
    let make_worker = |streams: Streams| {
        ArkService::new(
            OrderVerifier::new(key.x_only_public_key().0, ark_id, u64::MAX, 0),
            SeenOrders::new(streams.clone(), 6),
            AdmissionFixture {
                ready: ready.clone(),
                escapes: escapes.clone(),
            },
            streams,
            1,
        )
        .require_fresh_authorization()
    };
    role.ensure_group_from(ORDERS_STREAM, GROUP, "0")
        .await
        .unwrap();
    let mut worker = make_worker(role.independent().await.unwrap());
    // The first run-loop retry must succeed even when the PEL is empty.
    worker.retry_pending().await.unwrap();
    assert_eq!(role.kv_get(&cursor).await.unwrap().as_deref(), Some("0"));

    let stream_id = admin.add(ORDERS_STREAM, &frozen).await.unwrap();
    worker.pump_once().await.unwrap(); // actual restricted XREADGROUP -> HANDLE
    worker.retry_pending().await.unwrap(); // Held, persists the actual cursor
    assert_eq!(
        role.kv_get(&cursor).await.unwrap().as_deref(),
        Some(stream_id.as_str())
    );
    assert!(!admin
        .sismember(SeenOrders::AMBIGUOUS_KEY, &order.order_id)
        .await
        .unwrap());
    assert_eq!(admin.len(REQUESTS_STREAM).await.unwrap(), 0);
    assert_eq!(admin.len(RECEIPTS_STREAM).await.unwrap(), 0);
    assert_eq!(escapes.load(Ordering::SeqCst), 0);
    let pel: redis::Value = redis::cmd("XPENDING")
        .arg(ORDERS_STREAM)
        .arg(GROUP)
        .query_async(&mut admin_conn)
        .await
        .unwrap();
    let redis::Value::Array(summary) = pel else {
        panic!("XPENDING summary expected")
    };
    assert!(matches!(summary.first(), Some(redis::Value::Int(1))));
    assert_eq!(
        admin
            .read_pending(ORDERS_STREAM, GROUP, CONSUMER, 8)
            .await
            .unwrap(),
        vec![(stream_id.clone(), frozen.clone())]
    );
    drop(worker);

    // A separate restricted connection resumes and wraps the persisted cursor.
    let mut restarted = make_worker(role.independent().await.unwrap());
    restarted.retry_pending().await.unwrap();
    assert_eq!(role.kv_get(&cursor).await.unwrap().as_deref(), Some("0"));
    ready.store(true, Ordering::SeqCst);
    admin
        .ensure_group_from(REQUESTS_STREAM, "acl-issuer", "0")
        .await
        .unwrap();
    let issue_one = async {
        let requests = admin
            .read_group(REQUESTS_STREAM, "acl-issuer", "issuer", 1)
            .await
            .unwrap();
        assert_eq!(requests.len(), 1);
        let request = AuthorizationRequest::decode(&requests[0].1).unwrap();
        assert_eq!(request.order_id, order.order_id);
        assert_eq!(request.order_digest, order_digest(&order).unwrap());
        let field = format!(
            "{}:{}",
            hex::encode(request.order_id),
            hex::encode(request.nonce)
        );
        let mut grant = AuthorizationGrant {
            request,
            revision: 1,
            expires_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs()
                + 10,
            signature: [0; 64],
        };
        grant.sign(&key);
        admin
            .hset(GRANTS_KEY, &field, &hex::encode(grant.encode()))
            .await
            .unwrap();
    };
    tokio::time::timeout(Duration::from_secs(5), async {
        let (retry, ()) = tokio::join!(restarted.retry_pending(), issue_one);
        retry.unwrap();
    })
    .await
    .unwrap();
    assert_eq!(admin.len(REQUESTS_STREAM).await.unwrap(), 1);
    assert_eq!(escapes.load(Ordering::SeqCst), 1);
    assert!(admin
        .sismember(SeenOrders::AMBIGUOUS_KEY, &order.order_id)
        .await
        .unwrap());
    restarted.retry_pending().await.unwrap(); // wrap
    restarted.retry_pending().await.unwrap(); // pinned, never another request
    assert_eq!(admin.len(REQUESTS_STREAM).await.unwrap(), 1);
    assert_eq!(admin.len(RECEIPTS_STREAM).await.unwrap(), 0);
    assert_eq!(escapes.load(Ordering::SeqCst), 1);
    assert_eq!(
        admin
            .read_pending(ORDERS_STREAM, GROUP, CONSUMER, 8)
            .await
            .unwrap(),
        vec![(stream_id, frozen)]
    );
    assert_noperm(role.add(ORDERS_STREAM, b"still forbidden").await);
    assert_noperm(role.kv_get("spc:slices").await);
}
