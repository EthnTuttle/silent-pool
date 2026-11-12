//! A1/A2/A5: real isolated PG/Redis, real pinned cryptographic VTXO vectors.
//! These are boundary tests, not funded stock-captaind acceptance.
use super::*;
use ark::ProtocolEncoding;
use bitcoin::hashes::Hash;
use spc_ext::authorization::{
    order_digest, AuthorizationGrant, AuthorizationRequest, ProvedReceipt, REQUESTS_STREAM,
};
use spc_store::aggregate_store::AggregateMember;
use std::sync::atomic::{AtomicUsize, Ordering};
// Fixed production stream, separate owned fixture databases.
static AUTHORIZATION_FIXTURE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct ProofChain {
    blocks: BTreeMap<u32, [u8; 32]>,
    anchor: bitcoin::Transaction,
}
#[async_trait::async_trait]
impl ChainView for ProofChain {
    async fn tip_height(&self) -> Result<u32, ChainError> {
        Ok(110)
    }
    async fn block_hash_at(&self, height: u32) -> Result<Option<[u8; 32]>, ChainError> {
        Ok(Some(
            self.blocks.get(&height).copied().unwrap_or([0xfe; 32]),
        ))
    }
    async fn transaction(&self, id: &[u8; 32]) -> Result<Option<bitcoin::Transaction>, ChainError> {
        Ok((self.anchor.compute_txid().to_byte_array() == *id).then(|| self.anchor.clone()))
    }
    fn authoritative(&self) -> bool {
        true
    }
}
fn key() -> secp256k1::Keypair {
    secp256k1::Keypair::from_secret_key(
        &secp256k1::Secp256k1::new(),
        &secp256k1::SecretKey::from_slice(&[7; 32]).unwrap(),
    )
}
async fn resources() -> (Store, Streams) {
    let url = std::env::var("SPC_LADDER_TEST_DATABASE_URL").expect("owned PG required; no skip");
    assert!(url.contains("ladder_completion"));
    let admin = Store::connect(&url).await.unwrap();
    let name = format!(
        "ladder_completion_boundary_{}",
        uuid::Uuid::new_v4().simple()
    );
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(admin.pool())
        .await
        .unwrap();
    let store = Store::connect(&url.replace("/ladder_completion?", &format!("/{name}?")))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    let redis = std::env::var("SPC_LADDER_TEST_REDIS_URL").expect("owned Redis required; no skip");
    (store, Streams::connect(&redis).await.unwrap())
}
async fn fixture(
    vtxo: &ark::Vtxo<ark::vtxo::Full>,
    server: secp256k1::PublicKey,
) -> (PayoutService, spc_ext::ArkSendOrder, AuthorizationRequest) {
    fixture_members(vtxo, server, 1).await
}
async fn fixture_members(
    vtxo: &ark::Vtxo<ark::vtxo::Full>,
    server: secp256k1::PublicKey,
    count: u32,
) -> (PayoutService, spc_ext::ArkSendOrder, AuthorizationRequest) {
    let (store, streams) = resources().await;
    let mut blocks = BTreeMap::new();
    let mut members = Vec::new();
    let id = spc_crypto::sp_id(&server, &vtxo.user_pubkey());
    let sats = vtxo.amount().to_sat();
    let address = ark::Address::builder()
        .testnet(true)
        .server_pubkey(server)
        .pubkey_policy(vtxo.user_pubkey())
        .delivery(ark::address::VtxoDelivery::ServerMailbox {
            blinded_id: ark::mailbox::BlindedMailboxIdentifier::from_pubkey(vtxo.user_pubkey()),
        })
        .into_address()
        .unwrap()
        .to_string();
    store
        .upsert_identity(&SpIdentityRow {
            sp_id: id,
            b_scan: server.serialize().to_vec(),
            b_spend: vtxo.user_pubkey().serialize().to_vec(),
            network: 3,
        })
        .await
        .unwrap();
    for ordinal in 0..count {
        let height = 100 + ordinal;
        let block = rand::random();
        blocks.insert(height, block);
        let sats = sats / u64::from(count)
            + if ordinal == 0 {
                sats % u64::from(count)
            } else {
                0
            };
        let fp = rand::random();
        store
            .insert_snapshot(&SnapshotRow {
                fingerprint: fp,
                height,
                a_send_x: [4; 32],
                distribution_id: None,
                entries: json!([Entry {
                    sp_id: id,
                    owed_sats: sats,
                    rail: Rail::Ark,
                    p_x: vec![],
                    dest_id: address.as_bytes().to_vec(),
                    sats
                }]),
                high_water: json!({}),
                window_target: Some("1".into()),
                work_unit_version: WorkUnitVersion::V1,
                work_unit_fingerprint: None,
                window_rule: WindowRule::ShareWorkLegacy,
            })
            .await
            .unwrap();
        store
            .settle(
                &Settlement {
                    block_hash: block,
                    height,
                    fingerprint: Some(fp),
                    status: BlockStatus::Confirmed,
                    ledger: vec![LedgerRow {
                        sp_id: id,
                        rail: LedgerRail::Ark,
                        paid_sats: 0,
                        ark_due_sats: sats,
                        forfeited_sats: 0,
                        pending_delta: 0,
                    }],
                },
                6,
            )
            .await
            .unwrap();
        let member = AggregateMember {
            block_hash: block,
            sp_id: id,
            block_height: height,
            due_height: height + 6,
            sats,
            destination: address.as_bytes().to_vec(),
            is_remainder: false,
        };
        members.push(member);
    }
    let order = store
        .freeze_aggregate_bound(&members, 110, [0xfe; 32], &key(), server.serialize())
        .await
        .unwrap();
    store.publish_aggregate(&order.order_id).await.unwrap();
    let request = AuthorizationRequest {
        order_id: order.order_id,
        order_digest: order_digest(&order).unwrap(),
        nonce: rand::random(),
    };
    let receipts = format!("spc:test:receipts:{}", uuid::Uuid::new_v4());
    let svc = PayoutService::new(
        store,
        streams,
        6,
        330,
        Arc::new(ProofChain {
            blocks,
            anchor: ark::test_util::VTXO_VECTORS.anchor_tx.clone(),
        }),
        Some(key()),
        u64::MAX,
    )
    .with_ark_streams(
        format!("spc:test:orders:{}", uuid::Uuid::new_v4()),
        receipts,
    );
    svc.streams
        .ensure_group_from(&svc.receipts_stream, GROUP, "0")
        .await
        .unwrap();
    (svc, order, request)
}
async fn proved(
    svc: &PayoutService,
    order: &spc_ext::ArkSendOrder,
    request: AuthorizationRequest,
    vtxo: &ark::Vtxo<ark::vtxo::Full>,
) -> ProvedReceipt {
    let frozen = svc.store.aggregate(&order.order_id).await.unwrap().unwrap();
    let hashes = frozen.members.iter().map(|m| m.block_hash).collect();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let grant = svc
        .store
        .authorize_aggregate(&request, 110, [0xfe; 32], &hashes, now, &key())
        .await
        .unwrap()
        .unwrap();
    ProvedReceipt {
        grant,
        sats: order.sats,
        vtxos: vec![vtxo.serialize()],
    }
}

#[tokio::test]
async fn same_user_valid_htlc_or_foreign_server_proof_cannot_discharge_pubkey_order() {
    let vectors = &*ark::test_util::VTXO_VECTORS;
    for (vtxo, server) in [
        (
            &vectors.arkoor_htlc_out_vtxo,
            vectors.arkoor_htlc_out_vtxo.server_pubkey(),
        ),
        (&vectors.board_vtxo, key().public_key()),
    ] {
        vtxo.validate(&vectors.anchor_tx).unwrap(); // real valid signature/ancestry, not a decode-error test
        let (svc, order, request) = fixture(vtxo, server).await;
        let receipt = proved(&svc, &order, request, vtxo).await;
        assert!(svc
            .apply_proved_receipt(&receipt.encode().unwrap())
            .await
            .is_err());
        let frozen = svc.store.aggregate(&order.order_id).await.unwrap().unwrap();
        assert_eq!(frozen.state, "authorized");
        assert_eq!(svc.store.scheduled_ark_payouts().await.unwrap().len(), 1);
    }
}

struct ObserveTransport(Arc<AtomicUsize>);
struct CapacityTransport {
    lock: Arc<tokio::sync::Mutex<()>>,
    entered: std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    escaped: Arc<AtomicUsize>,
    deadline: Duration,
}

#[cfg(feature = "ark")]
struct WalletStateTransport {
    capacity: CapacityTransport,
    rows: Arc<std::sync::Mutex<Vec<bark::vtxo::WalletVtxo>>>,
    tip: Arc<std::sync::atomic::AtomicU32>,
    quote: Arc<std::sync::Mutex<bark::FeeEstimate>>,
}
#[cfg(feature = "ark")]
#[async_trait::async_trait]
impl crate::ark::ArkTransport for WalletStateTransport {
    async fn send(&self, _: &str, _: u64) -> Result<(Vec<u8>, u32), String> {
        panic!("legacy bypass")
    }
    async fn reserve_send_capacity(
        &self,
    ) -> Result<
        Option<crate::ark::send_fence::WalletOperationPermit>,
        crate::ark::send_fence::CapacityError,
    > {
        self.capacity.reserve_send_capacity().await
    }
    async fn check_send_readiness(
        &self,
        address: &str,
        sats: u64,
        permit: Option<&crate::ark::send_fence::WalletOperationPermit>,
    ) -> Result<(), crate::ark::send_fence::CapacityError> {
        self.capacity
            .check_send_readiness(address, sats, permit)
            .await?;
        crate::ark::bark_stock::select_ready_inputs(
            self.rows.lock().unwrap().clone(),
            self.tip.load(Ordering::SeqCst),
            64,
            sats,
            &self.quote.lock().unwrap(),
        )?;
        Ok(())
    }
    async fn send_authorized(
        &self,
        address: &str,
        sats: u64,
        authorization: &crate::ark::send_fence::SendAuthorization,
        permit: Option<&crate::ark::send_fence::WalletOperationPermit>,
    ) -> Result<Vec<Vec<u8>>, String> {
        self.check_send_readiness(address, sats, permit)
            .await
            .map_err(|e| e.to_string())?;
        self.capacity
            .send_authorized(address, sats, authorization, permit)
            .await
    }
}
#[async_trait::async_trait]
impl crate::ark::ArkTransport for CapacityTransport {
    async fn check_send_readiness(
        &self,
        _: &str,
        _: u64,
        permit: Option<&crate::ark::send_fence::WalletOperationPermit>,
    ) -> Result<(), crate::ark::send_fence::CapacityError> {
        permit
            .ok_or(crate::ark::send_fence::CapacityError::ForeignPermit)?
            .require_wallet(&self.lock)
            .map_err(|_| crate::ark::send_fence::CapacityError::ForeignPermit)
    }
    async fn send(&self, _: &str, _: u64) -> Result<(Vec<u8>, u32), String> {
        panic!("legacy bypass")
    }
    async fn reserve_send_capacity(
        &self,
    ) -> Result<
        Option<crate::ark::send_fence::WalletOperationPermit>,
        crate::ark::send_fence::CapacityError,
    > {
        if let Some(tx) = self.entered.lock().unwrap().take() {
            let _ = tx.send(());
        }
        crate::ark::send_fence::WalletOperationPermit::acquire_with_deadline(
            self.lock.clone(),
            self.deadline,
        )
        .await
        .map(Some)
    }
    async fn send_authorized(
        &self,
        address: &str,
        sats: u64,
        authorization: &crate::ark::send_fence::SendAuthorization,
        permit: Option<&crate::ark::send_fence::WalletOperationPermit>,
    ) -> Result<Vec<Vec<u8>>, String> {
        permit.ok_or("missing permit")?.require_wallet(&self.lock)?;
        authorization.validate(address, sats)?;
        authorization.enter_cosign(address, sats)?;
        self.escaped.fetch_add(1, Ordering::SeqCst);
        Err("response lost after this attempt's cosign escape".into())
    }
}
#[async_trait::async_trait]
impl crate::ark::ArkTransport for ObserveTransport {
    async fn check_send_readiness(
        &self,
        _: &str,
        _: u64,
        _: Option<&crate::ark::send_fence::WalletOperationPermit>,
    ) -> Result<(), crate::ark::send_fence::CapacityError> {
        Ok(())
    }
    async fn reserve_send_capacity(
        &self,
    ) -> Result<
        Option<crate::ark::send_fence::WalletOperationPermit>,
        crate::ark::send_fence::CapacityError,
    > {
        Ok(None)
    }
    async fn send_authorized(
        &self,
        address: &str,
        sats: u64,
        authorization: &crate::ark::send_fence::SendAuthorization,
        _permit: Option<&crate::ark::send_fence::WalletOperationPermit>,
    ) -> Result<Vec<Vec<u8>>, String> {
        authorization.validate(address, sats)?;
        self.0.fetch_add(1, Ordering::SeqCst);
        Err("deliberate transport unknown after authorization".into())
    }
    async fn send(&self, _: &str, _: u64) -> Result<(Vec<u8>, u32), String> {
        panic!("legacy bypass")
    }
    async fn send_proven(&self, _: &str, _: u64) -> Result<Vec<Vec<u8>>, String> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err("deliberate transport unknown after authorization".into())
    }
}
#[tokio::test]
async fn quiet_unrelated_redis_reads_do_not_starve_the_fresh_grant() {
    let _fixture = AUTHORIZATION_FIXTURE.lock().await;
    let vtxo = &ark::test_util::VTXO_VECTORS.board_vtxo;
    let (svc, order, _) = fixture(vtxo, vtxo.server_pubkey()).await;
    svc.streams
        .ensure_group_from(REQUESTS_STREAM, GROUP, "0")
        .await
        .unwrap();
    let _dispatcher = svc.authorization_dispatcher().await.unwrap();
    let mut names = Vec::new();
    for _ in 0..3 {
        let stream = format!("spc:test:quiet:{}", uuid::Uuid::new_v4());
        svc.streams.ensure_group(&stream, "quiet").await.unwrap();
        names.push(stream);
    }
    let blocking = svc.streams.clone();
    let quiet = tokio::spawn(async move {
        for name in names {
            blocking
                .read_group(&name, "quiet", "reader", 1)
                .await
                .unwrap();
        }
    });
    tokio::time::sleep(Duration::from_millis(30)).await;
    let streams = svc.streams.independent().await.unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let verifier = crate::ark::OrderVerifier::new(
        key().x_only_public_key().0,
        <[u8; 4]>::from(ark::address::ArkId::from_server_pubkey(
            vtxo.server_pubkey(),
        )),
        u64::MAX,
        0,
    );
    let mut worker = crate::ark::ArkService::new(
        verifier,
        crate::ark::SeenOrders::new(streams.clone(), 6),
        ObserveTransport(calls.clone()),
        streams,
        110,
    )
    .require_fresh_authorization();
    let start = Instant::now();
    assert!(worker.process(&order).await.is_err());
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "healthy quiet streams must not lose the grant"
    );
    assert!(start.elapsed() < Duration::from_secs(2));
    quiet.await.unwrap();
}

#[tokio::test]
async fn malformed_and_more_than_64_unknown_pending_receipts_do_not_starve_a_valid_receipt_after_restart(
) {
    let vtxo = &ark::test_util::VTXO_VECTORS.board_vtxo;
    let (svc, order, request) = fixture(vtxo, vtxo.server_pubkey()).await;
    let good = proved(&svc, &order, request, vtxo).await;
    svc.streams
        .add(&svc.receipts_stream, b"SPRP\x01bad")
        .await
        .unwrap();
    for _ in 0..65 {
        let mut grant = AuthorizationGrant {
            request: AuthorizationRequest {
                order_id: rand::random(),
                order_digest: [3; 32],
                nonce: rand::random(),
            },
            revision: 1,
            expires_at: 2000,
            signature: [0; 64],
        };
        grant.sign(&key());
        let unknown = ProvedReceipt {
            grant,
            sats: order.sats,
            vtxos: vec![vtxo.serialize()],
        };
        svc.streams
            .add(&svc.receipts_stream, &unknown.encode().unwrap())
            .await
            .unwrap();
    }
    svc.streams
        .add(&svc.receipts_stream, &good.encode().unwrap())
        .await
        .unwrap();
    svc.streams
        .read_group(&svc.receipts_stream, GROUP, CONSUMER, 128)
        .await
        .unwrap(); // all are PEL; no new messages
    svc.process_receipts_once().await.unwrap();
    let saved_cursor = svc
        .store
        .receipt_cursor(&svc.receipts_stream)
        .await
        .unwrap();
    assert_ne!(saved_cursor, "0");
    let mut restarted = svc.clone();
    restarted.streams = svc.streams.independent().await.unwrap();
    assert_eq!(
        restarted
            .store
            .receipt_cursor(&svc.receipts_stream)
            .await
            .unwrap(),
        saved_cursor
    );
    restarted.process_receipts_once().await.unwrap();
    assert_eq!(
        svc.store
            .aggregate(&order.order_id)
            .await
            .unwrap()
            .unwrap()
            .state,
        "sent"
    );
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ark_receipt_disposition WHERE status='terminal_invalid'",
    )
    .fetch_one(svc.store.pool())
    .await
    .unwrap();
    assert_eq!(count, 1);
    let retries: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ark_receipt_disposition WHERE status='retry_unknown'",
    )
    .fetch_one(svc.store.pool())
    .await
    .unwrap();
    assert_eq!(retries, 65);
}

#[tokio::test]
async fn real_authorization_deadline_expiry_pins_the_order_and_late_grant_cannot_retry_send() {
    let _fixture = AUTHORIZATION_FIXTURE.lock().await;
    let vtxo = &ark::test_util::VTXO_VECTORS.board_vtxo;
    let (svc, order, _) = fixture(vtxo, vtxo.server_pubkey()).await;
    svc.streams
        .ensure_group_from(REQUESTS_STREAM, GROUP, "0")
        .await
        .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let streams = svc.streams.independent().await.unwrap();
    let verifier = crate::ark::OrderVerifier::new(
        key().x_only_public_key().0,
        <[u8; 4]>::from(ark::address::ArkId::from_server_pubkey(
            vtxo.server_pubkey(),
        )),
        u64::MAX,
        0,
    );
    let mut worker = crate::ark::ArkService::new(
        verifier,
        crate::ark::SeenOrders::new(streams.clone(), 6),
        ObserveTransport(calls.clone()),
        streams,
        110,
    )
    .require_fresh_authorization();
    let start = Instant::now();
    assert!(!worker.handle(&order).await.unwrap());
    assert!(start.elapsed() >= Duration::from_secs(2));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    svc.process_authorizations().await.unwrap(); // an actual late nonce grant, after the worker timed out
    assert_eq!(
        svc.store
            .aggregate(&order.order_id)
            .await
            .unwrap()
            .unwrap()
            .state,
        "authorized"
    );
    assert!(svc
        .streams
        .sismember(crate::ark::SeenOrders::AMBIGUOUS_KEY, &order.order_id)
        .await
        .unwrap());
    let _ = worker.process(&order).await;
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "durable unknown intent forbids a new attempt"
    );
    assert_eq!(svc.store.scheduled_ark_payouts().await.unwrap().len(), 1);
}

#[tokio::test]
async fn wallet_capacity_precedes_grant_and_guarded_escape_never_reacquires_or_reissues() {
    let _fixture = AUTHORIZATION_FIXTURE.lock().await;
    let vtxo = &ark::test_util::VTXO_VECTORS.board_vtxo;
    let (svc, order, _) = fixture(vtxo, vtxo.server_pubkey()).await;
    svc.streams
        .ensure_group_from(REQUESTS_STREAM, GROUP, "0")
        .await
        .unwrap();
    let _dispatcher = svc.authorization_dispatcher().await.unwrap();
    let lock = Arc::new(tokio::sync::Mutex::new(()));
    let board = lock.clone().lock_owned().await;
    let (tx, rx) = tokio::sync::oneshot::channel();
    let escaped = Arc::new(AtomicUsize::new(0));
    let streams = svc.streams.independent().await.unwrap();
    let verifier = crate::ark::OrderVerifier::new(
        key().x_only_public_key().0,
        <[u8; 4]>::from(ark::address::ArkId::from_server_pubkey(
            vtxo.server_pubkey(),
        )),
        u64::MAX,
        0,
    );
    let mut worker = crate::ark::ArkService::new(
        verifier,
        crate::ark::SeenOrders::new(streams.clone(), 6),
        CapacityTransport {
            lock,
            entered: std::sync::Mutex::new(Some(tx)),
            escaped: escaped.clone(),
            deadline: Duration::from_secs(30),
        },
        streams,
        110,
    )
    .require_fresh_authorization();
    let sending_order = order.clone();
    let task = tokio::spawn(async move {
        let result = worker.process(&sending_order).await;
        (worker, result)
    });
    rx.await.unwrap();
    let grants: i64 = sqlx::query_scalar("SELECT count(*) FROM ark_send_authorization")
        .fetch_one(svc.store.pool())
        .await
        .unwrap();
    assert_eq!(grants, 0, "no grant while board owns capacity");
    drop(board);
    let (mut worker, result) = tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap();
    assert!(result.is_err());
    assert_eq!(escaped.load(Ordering::SeqCst), 1);
    assert!(worker.process(&order).await.is_err());
    assert_eq!(escaped.load(Ordering::SeqCst), 1);
    let grants: i64 = sqlx::query_scalar("SELECT count(*) FROM ark_send_authorization")
        .fetch_one(svc.store.pool())
        .await
        .unwrap();
    assert_eq!(grants, 1, "ambiguity never obtains a replacement grant");
}

#[tokio::test]
async fn handle_capacity_deadline_keeps_held_order_unpinned_and_restart_pel_retry_reaches_it() {
    let _fixture = AUTHORIZATION_FIXTURE.lock().await;
    let vtxo = &ark::test_util::VTXO_VECTORS.board_vtxo;
    let (svc, order, _) = fixture(vtxo, vtxo.server_pubkey()).await;
    let group = format!("ark-held-{}", uuid::Uuid::new_v4());
    let consumer = "same-worker";
    svc.streams
        .ensure_group_from(&svc.orders_stream, &group, "0")
        .await
        .unwrap();
    svc.streams
        .ensure_group_from(REQUESTS_STREAM, GROUP, "0")
        .await
        .unwrap();
    let requests_before = svc.streams.len(REQUESTS_STREAM).await.unwrap();
    let lock = Arc::new(tokio::sync::Mutex::new(()));
    let board = lock.clone().lock_owned().await;
    let escaped = Arc::new(AtomicUsize::new(0));
    let make = |streams: Streams| {
        crate::ark::ArkService::new(
            crate::ark::OrderVerifier::new(
                key().x_only_public_key().0,
                <[u8; 4]>::from(ark::address::ArkId::from_server_pubkey(
                    vtxo.server_pubkey(),
                )),
                u64::MAX,
                0,
            ),
            crate::ark::SeenOrders::new(streams.clone(), 6),
            CapacityTransport {
                lock: lock.clone(),
                entered: std::sync::Mutex::new(None),
                escaped: escaped.clone(),
                deadline: Duration::from_millis(20),
            },
            streams,
            110,
        )
        .require_fresh_authorization()
        .with_names(
            svc.orders_stream.clone(),
            svc.receipts_stream.clone(),
            group.clone(),
            consumer,
        )
    };
    let mut worker = make(svc.streams.independent().await.unwrap());
    // An unresolved prefix cannot starve the later Held obligation.
    let mut preexisting = None;
    for _ in 0..65 {
        let mut pinned = order.clone();
        pinned.order_id = rand::random();
        pinned.sign(&key()).unwrap();
        svc.streams
            .sadd(crate::ark::SeenOrders::AMBIGUOUS_KEY, &pinned.order_id)
            .await
            .unwrap();
        svc.streams
            .add(&svc.orders_stream, &pinned.encode().unwrap())
            .await
            .unwrap();
        preexisting = Some(pinned.order_id);
    }
    svc.streams
        .add(&svc.orders_stream, &order.encode().unwrap())
        .await
        .unwrap();
    for _ in 0..5 {
        worker.pump_once().await.unwrap();
    } // drives HANDLE, including the actual timeout helper
    assert!(!svc
        .streams
        .sismember(crate::ark::SeenOrders::AMBIGUOUS_KEY, &order.order_id)
        .await
        .unwrap());
    assert!(svc
        .streams
        .sismember(crate::ark::SeenOrders::AMBIGUOUS_KEY, &preexisting.unwrap())
        .await
        .unwrap());
    assert_eq!(
        svc.streams.len(REQUESTS_STREAM).await.unwrap(),
        requests_before
    );
    assert_eq!(escaped.load(Ordering::SeqCst), 0);
    let grants: i64 = sqlx::query_scalar("SELECT count(*) FROM ark_send_authorization")
        .fetch_one(svc.store.pool())
        .await
        .unwrap();
    assert_eq!(grants, 0);
    assert_eq!(
        svc.store
            .aggregate(&order.order_id)
            .await
            .unwrap()
            .unwrap()
            .state,
        "published"
    );
    let reserved: bool =
        sqlx::query_scalar("SELECT bool_and(reserved) FROM ark_aggregate_member WHERE order_id=$1")
            .bind(&order.order_id[..])
            .fetch_one(svc.store.pool())
            .await
            .unwrap();
    assert!(reserved);
    worker.retry_pending().await.unwrap(); // first 64 pinned, cursor persisted
    drop(worker);
    drop(board);
    let _dispatcher = svc.authorization_dispatcher().await.unwrap();
    let mut restarted = make(svc.streams.independent().await.unwrap());
    restarted.retry_pending().await.unwrap(); // no new stream message: reaches SAME Held order
    assert_eq!(escaped.load(Ordering::SeqCst), 1);
    assert_eq!(
        svc.streams.len(REQUESTS_STREAM).await.unwrap(),
        requests_before + 1
    );
    assert!(
        svc.streams
            .sismember(crate::ark::SeenOrders::AMBIGUOUS_KEY, &order.order_id)
            .await
            .unwrap(),
        "lost cosign response is now genuinely ambiguous"
    );
    assert!(!restarted.handle(&order).await.unwrap());
    assert_eq!(escaped.load(Ordering::SeqCst), 1);
    let grants: i64 = sqlx::query_scalar("SELECT count(*) FROM ark_send_authorization")
        .fetch_one(svc.store.pool())
        .await
        .unwrap();
    assert_eq!(grants, 1);
}

#[cfg(feature = "ark")]
#[tokio::test]
async fn handle_actual_wallet_state_shortages_wait_before_request_then_same_order_gets_first_grant()
{
    let _fixture = AUTHORIZATION_FIXTURE.lock().await;
    let vtxo = &ark::test_util::VTXO_VECTORS.board_vtxo;
    vtxo.validate(&ark::test_util::VTXO_VECTORS.anchor_tx)
        .unwrap();
    let (svc, order, _) = fixture(vtxo, vtxo.server_pubkey()).await;
    let sats = order.sats;
    svc.streams
        .ensure_group_from(REQUESTS_STREAM, GROUP, "0")
        .await
        .unwrap();
    let before = svc.streams.len(REQUESTS_STREAM).await.unwrap();
    let lock = Arc::new(tokio::sync::Mutex::new(()));
    let escaped = Arc::new(AtomicUsize::new(0));
    let rows = Arc::new(std::sync::Mutex::new(vec![]));
    let tip = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let amount = bitcoin::Amount::from_sat(sats);
    let quote = Arc::new(std::sync::Mutex::new(bark::FeeEstimate::new(
        amount,
        bitcoin::Amount::ZERO,
        amount,
        vec![],
    )));
    let streams = svc.streams.independent().await.unwrap();
    let mut worker = crate::ark::ArkService::new(
        crate::ark::OrderVerifier::new(
            key().x_only_public_key().0,
            <[u8; 4]>::from(ark::address::ArkId::from_server_pubkey(
                vtxo.server_pubkey(),
            )),
            u64::MAX,
            0,
        ),
        crate::ark::SeenOrders::new(streams.clone(), 6),
        WalletStateTransport {
            capacity: CapacityTransport {
                lock,
                entered: std::sync::Mutex::new(None),
                escaped: escaped.clone(),
                deadline: Duration::from_millis(20),
            },
            rows: rows.clone(),
            tip: tip.clone(),
            quote: quote.clone(),
        },
        streams,
        110,
    )
    .require_fresh_authorization();
    let good = bark::vtxo::WalletVtxo {
        vtxo: vtxo.to_bare(),
        state: bark::vtxo::VtxoState::Spendable,
        exit_depth: vtxo.exit_depth(),
        exit_tx_weight: bitcoin::Weight::from_wu(0),
        registered: true,
    };
    for phase in 0..5 {
        match phase {
            0 => {} // zero float
            1 => {
                let mut row = good.clone();
                row.state = bark::vtxo::VtxoState::Locked { holder: None };
                *rows.lock().unwrap() = vec![row];
            }
            2 => {
                *rows.lock().unwrap() = vec![good.clone()];
                tip.store(vtxo.expiry_height().to_u32(), Ordering::SeqCst);
            }
            3 => {
                tip.store(0, Ordering::SeqCst);
                let mut row = good.clone();
                row.exit_depth = 64;
                *rows.lock().unwrap() = vec![row];
            }
            _ => {
                *rows.lock().unwrap() = vec![good.clone()];
                *quote.lock().unwrap() = bark::FeeEstimate::new(
                    bitcoin::Amount::from_sat(sats + 1),
                    bitcoin::Amount::from_sat(1),
                    amount,
                    vec![],
                );
            }
        }
        assert!(!worker.handle(&order).await.unwrap());
        assert_eq!(svc.streams.len(REQUESTS_STREAM).await.unwrap(), before);
        assert!(!svc
            .streams
            .sismember(crate::ark::SeenOrders::AMBIGUOUS_KEY, &order.order_id)
            .await
            .unwrap());
        assert_eq!(escaped.load(Ordering::SeqCst), 0);
        let grants: i64 = sqlx::query_scalar("SELECT count(*) FROM ark_send_authorization")
            .fetch_one(svc.store.pool())
            .await
            .unwrap();
        assert_eq!(grants, 0);
    }
    *quote.lock().unwrap() = bark::FeeEstimate::new(amount, bitcoin::Amount::ZERO, amount, vec![]);
    *rows.lock().unwrap() = vec![good];
    tip.store(0, Ordering::SeqCst);
    let _dispatcher = svc.authorization_dispatcher().await.unwrap();
    assert!(!worker.handle(&order).await.unwrap()); // first actual grant, then simulated escaped/lost cosign response
    assert_eq!(escaped.load(Ordering::SeqCst), 1);
    assert_eq!(svc.streams.len(REQUESTS_STREAM).await.unwrap(), before + 1);
    assert!(svc
        .streams
        .sismember(crate::ark::SeenOrders::AMBIGUOUS_KEY, &order.order_id)
        .await
        .unwrap());
    assert!(!worker.handle(&order).await.unwrap());
    assert_eq!(escaped.load(Ordering::SeqCst), 1);
    assert_eq!(svc.streams.len(REQUESTS_STREAM).await.unwrap(), before + 1);
    let reserved: bool =
        sqlx::query_scalar("SELECT bool_and(reserved) FROM ark_aggregate_member WHERE order_id=$1")
            .bind(&order.order_id[..])
            .fetch_one(svc.store.pool())
            .await
            .unwrap();
    assert!(reserved);
}

#[tokio::test]
async fn legacy_sent_cannot_split_two_members_then_complete_proof_applies_once() {
    let vtxo = &ark::test_util::VTXO_VECTORS.board_vtxo;
    let (mut svc, order, request) = fixture_members(vtxo, vtxo.server_pubkey(), 2).await;
    svc.aggregate_server = Some(("http://unused.invalid".into(), bitcoin::Network::Regtest));
    let members = svc
        .store
        .aggregate(&order.order_id)
        .await
        .unwrap()
        .unwrap()
        .members;
    let legacy = ArkSendReceipt {
        order_id: ark_order_id(&members[0].block_hash, &members[0].sp_id),
        status: ReceiptStatus::Sent,
        vtxo_id: vec![1; 36],
        expiry_height: 9000,
    };
    svc.streams
        .add(&svc.receipts_stream, &legacy.encode().unwrap())
        .await
        .unwrap();
    svc.process_receipts_once().await.unwrap();
    assert_eq!(svc.store.scheduled_ark_payouts().await.unwrap().len(), 2);
    let receipt = proved(&svc, &order, request, vtxo).await;
    svc.apply_proved_receipt(&receipt.encode().unwrap())
        .await
        .unwrap();
    svc.apply_proved_receipt(&receipt.encode().unwrap())
        .await
        .unwrap();
    assert!(svc.store.scheduled_ark_payouts().await.unwrap().is_empty());
    let sent: i64 = sqlx::query_scalar("SELECT count(*) FROM ark_payout WHERE status='sent'")
        .fetch_one(svc.store.pool())
        .await
        .unwrap();
    assert_eq!(sent, 2);
}

#[tokio::test]
async fn frozen_order_private_api_requires_owner_and_returns_exact_signed_bytes_in_unknown_state() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let vtxo = &ark::test_util::VTXO_VECTORS.board_vtxo;
    let (svc, order, request) = fixture(vtxo, key().public_key()).await;
    let receipt = proved(&svc, &order, request, vtxo).await;
    svc.store
        .block_aggregate_unknown(&order.order_id)
        .await
        .unwrap();
    let owner = spc_crypto::sp_id(&key().public_key(), &vtxo.user_pubkey());
    let bytes = svc
        .store
        .aggregate_order_for(&owner, &order.order_id, Some(&key().x_only_public_key().0))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(bytes, order.encode().unwrap());
    let decoded = spc_ext::ArkSendOrder::decode(&bytes).unwrap();
    decoded.verify(&key().x_only_public_key().0).unwrap();
    assert_eq!(
        order_digest(&decoded).unwrap(),
        receipt.grant.request.order_digest
    );
    assert!(
        svc.store
            .aggregate_order_for(&[0xff; 32], &order.order_id, None)
            .await
            .unwrap()
            .is_none(),
        "ownership denial precedes decoding/key lookup"
    );
    let backend = crate::api_backend::StoreBackend::new(svc.store.clone(), json!({}), json!({}))
        .with_order_pubkey(Some(key().x_only_public_key().0));
    let app = crate::api::router(Arc::new(backend), spc_crypto::Network::Regtest);
    let path = format!(
        "/api/sp/me/ark-aggregates/{}/order",
        hex::encode(order.order_id)
    );
    assert_eq!(
        app.clone()
            .oneshot(Request::builder().uri(&path).body(Body::empty()).unwrap())
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    for authorized in [false, true] {
        let scan = if authorized {
            key()
        } else {
            secp256k1::Keypair::from_secret_key(
                &secp256k1::Secp256k1::new(),
                &secp256k1::SecretKey::from_slice(&[8; 32]).unwrap(),
            )
        };
        let addr = spc_crypto::address::encode_sp_address(
            spc_crypto::Network::Regtest,
            &scan.public_key(),
            &vtxo.user_pubkey(),
        )
        .unwrap();
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/sp/challenge")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let value: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        let nonce = value["nonce"].as_str().unwrap();
        let sig = secp256k1::Secp256k1::new().sign_schnorr_no_aux_rand(
            &secp256k1::Message::from_digest(crate::api::api_message(nonce)),
            &scan,
        );
        let auth = format!("SPc {addr} {nonce} {sig}");
        let get = || {
            Request::builder()
                .uri(&path)
                .header("Authorization", &auth)
                .body(Body::empty())
                .unwrap()
        };
        let response = app.clone().oneshot(get()).await.unwrap();
        assert_eq!(
            response.status(),
            if authorized {
                StatusCode::OK
            } else {
                StatusCode::NOT_FOUND
            }
        );
        if authorized {
            assert_eq!(
                response.headers()["content-type"],
                "application/octet-stream"
            );
            assert_eq!(
                response
                    .into_body()
                    .collect()
                    .await
                    .unwrap()
                    .to_bytes()
                    .as_ref(),
                bytes
            );
        }
        assert_eq!(
            app.clone().oneshot(get()).await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );
    }
    for mode in 0..4 {
        // Insert a separate corrupt storage fixture, never disable immutable triggers.
        let id: [u8; 16] = rand::random();
        let mut bad = order.clone();
        bad.order_id = if mode == 3 { order.order_id } else { id };
        bad.sign(&key()).unwrap();
        if mode == 2 {
            bad.sig = [0; 64];
        }
        let digest = order_digest(&bad).unwrap();
        let mut malformed = bad.encode().unwrap();
        if mode == 0 {
            malformed.pop();
        } else if mode == 1 {
            malformed = vec![0; spc_ext::ArkSendOrder::MAX_ENCODED_LEN + 1];
        }
        sqlx::query("INSERT INTO ark_aggregate(order_id,sp_id,anchor_hash,anchor_height,destination,sats,not_before_height,selected_tip,selected_tip_hash,order_bytes,order_digest,state)
            SELECT $1,sp_id,anchor_hash,anchor_height,destination,sats,not_before_height,selected_tip,selected_tip_hash,$2,$4,'frozen' FROM ark_aggregate WHERE order_id=$3")
            .bind(&id[..]).bind(malformed).bind(&order.order_id[..]).bind(&digest[..]).execute(svc.store.pool()).await.unwrap();
        assert!(svc
            .store
            .aggregate_order_for(&[0xff; 32], &id, None)
            .await
            .unwrap()
            .is_none());
        assert!(svc
            .store
            .aggregate_order_for(&owner, &id, Some(&key().x_only_public_key().0))
            .await
            .is_err());
    }
}
