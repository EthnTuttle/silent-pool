//! Thin wallet/shared-auditor boundary tests. Mock collectors below never claim
//! native activation or funded acceptance; positive source verification and key
//! cleanup regressions are also imported unchanged with the actual helper.
use super::*;
use crate::source_feed::{read_frame, write_frame};
use crate::wallet::{KeySources, WalletKeys};
use bitcoin::secp256k1::{Keypair, PublicKey, Secp256k1, SecretKey};
use spc_scan::source_feed::{Checkpoint, Frame, TraceSeal, SCHEMA};
use std::os::unix::fs::PermissionsExt;
use std::time::Duration;
use tokio::io::AsyncReadExt;

fn key(n: u8) -> Keypair {
    Keypair::from_secret_key(&Secp256k1::new(), &SecretKey::from_slice(&[n; 32]).unwrap())
}

fn wallet() -> (tempfile::TempDir, WalletKeys, String) {
    let root = tempfile::Builder::new()
        .prefix("wallet-source-audit-")
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir_in("/tmp/opencode")
        .unwrap();
    let online = root.path().join("online");
    let public = crate::wallet::provision::provision(&online, &root.path().join("cold")).unwrap();
    let keys = WalletKeys::load(&KeySources {
        scan_xprv: online.join("scan.xprv"),
        ark_xprv: online.join("ark.xprv"),
        child_seed: online.join("child.seed"),
        sp_address: public.sp_address.clone(),
        network: spc_crypto::Network::Regtest,
    })
    .unwrap();
    (root, keys, public.sp_address)
}

fn binding(socket: PathBuf) -> SourceBinding {
    SourceBinding {
        socket,
        pool: key(9).x_only_public_key().0,
        collector: key(10).x_only_public_key().0,
    }
}

fn received() -> ReceivedJob {
    // Only used before a valid proof is available. It cannot pass owned payout
    // verification and is never returned as an activated native job.
    ReceivedJob {
        job_id: 17,
        prev_hash: [1; 32],
        version: 0x2000_0000,
        nbits: 0x207f_ffff,
        ntime: 123,
        coinbase_prefix: vec![1],
        coinbase_suffix: vec![2],
        merkle_path: vec![],
        share_target_be: [3; 32],
        pool_extranonce: vec![4; 4],
        miner_extranonce_len: 8,
    }
}

fn listener(root: &std::path::Path) -> (PathBuf, tokio::net::UnixListener) {
    let path = root.join("collector.sock");
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    (path, listener)
}

#[test]
fn wallet_bridge_borrows_only_existing_key_and_checks_the_public_identity() {
    let (root, keys, address) = wallet();
    let source = binding(root.path().join("not-opened.sock"));
    let request = RequestAudit::from_wallet(&keys, &address, &source).unwrap();
    let wrong = spc_crypto::address::encode_sp_address(
        keys.network,
        &key(12).public_key(),
        &keys.b_spend_public,
    )
    .unwrap();
    assert!(RequestAudit::from_wallet(&keys, &wrong, &source).is_err());
    let mut same_authority = source.clone();
    same_authority.collector = source.pool;
    assert!(RequestAudit::from_wallet(&keys, &address, &same_authority).is_err());
    // No borrowed wallet reference is retained by the request-owned verifier.
    drop(keys);
    drop(request);
    assert_eq!(SCHEMA, "spc.source-feed/2");
}

#[tokio::test]
async fn wallet_bridge_preserves_outer_attempt_and_cancellation_closes_without_ack() {
    let (root, keys, address) = wallet();
    let (path, server) = listener(root.path());
    let request = RequestAudit::from_wallet(&keys, &address, &binding(path)).unwrap();
    let job = received();
    let digest = job.digest();
    let original_public = PublicKey::from_secret_key(&Secp256k1::new(), &keys.b_scan);
    let (entered, ready) = tokio::sync::oneshot::channel();
    let collector = tokio::spawn(async move {
        let (mut stream, _) = server.accept().await.unwrap();
        let Frame::Job {
            schema,
            attempt,
            job,
        } = read_frame(&mut stream).await.unwrap()
        else {
            panic!("expected shared source job")
        };
        assert_eq!(schema, SCHEMA);
        assert_eq!(attempt, [7; 16]);
        assert_eq!(job.digest(), digest);
        entered.send(()).unwrap();
        assert_eq!(
            stream.read(&mut [0]).await.unwrap(),
            0,
            "cancellation must not send a consumption ACK"
        );
    });
    let task = tokio::spawn(async move {
        request
            .verify_received(&job, [7; 16], Instant::now() + Duration::from_secs(2))
            .await
    });
    tokio::time::timeout(Duration::from_secs(1), ready)
        .await
        .unwrap()
        .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    tokio::time::timeout(Duration::from_secs(1), collector)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        PublicKey::from_secret_key(&Secp256k1::new(), &keys.b_scan),
        original_public
    );
}

#[tokio::test]
async fn wallet_bridge_outer_deadline_drops_the_same_consuming_auditor() {
    let (root, keys, address) = wallet();
    let (path, server) = listener(root.path());
    let request = RequestAudit::from_wallet(&keys, &address, &binding(path)).unwrap();
    let collector = tokio::spawn(async move {
        let (mut stream, _) = server.accept().await.unwrap();
        assert!(matches!(
            read_frame(&mut stream).await.unwrap(),
            Frame::Job { .. }
        ));
        assert_eq!(stream.read(&mut [0]).await.unwrap(), 0);
    });
    let error = request
        .verify_received(
            &received(),
            [8; 16],
            Instant::now() + Duration::from_millis(200),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("total deadline"));
    tokio::time::timeout(Duration::from_secs(1), collector)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn wallet_bridge_rejects_v1_or_incomplete_history_instead_of_warming_success() {
    for schema in ["spc.source-feed/1", SCHEMA] {
        let (root, keys, address) = wallet();
        let (path, server) = listener(root.path());
        let request = RequestAudit::from_wallet(&keys, &address, &binding(path)).unwrap();
        let collector = tokio::spawn(async move {
            let (mut stream, _) = server.accept().await.unwrap();
            let Frame::Job { attempt, job, .. } = read_frame(&mut stream).await.unwrap() else {
                panic!("job")
            };
            let checkpoint = Checkpoint {
                attempt,
                wire_job_digest: job.digest(),
                job_id: 17,
                generation: 1,
                source_watermark: 1,
                history_watermark: 1,
                bundle_digest: [0; 32],
                history_digest: [0; 32],
                expires_at_unix_ms: u64::MAX,
                complete: false,
                pool: TraceSeal {
                    session: [1; 16],
                    seq: 1,
                    digest: [0; 32],
                    signature: String::new(),
                },
                signature: String::new(),
            };
            // Explicit invalid source: no cryptographic success is simulated.
            write_frame(
                &mut stream,
                &Frame::Proof {
                    schema: schema.into(),
                    bundle_hex: String::new(),
                    checkpoint,
                },
            )
            .await
            .unwrap();
            assert_eq!(
                stream.read(&mut [0]).await.unwrap(),
                0,
                "no consumed ACK for missing history/v1"
            );
        });
        assert!(request
            .verify_received(
                &received(),
                [9; 16],
                Instant::now() + Duration::from_secs(1)
            )
            .await
            .is_err());
        tokio::time::timeout(Duration::from_secs(1), collector)
            .await
            .unwrap()
            .unwrap();
    }
}

#[tokio::test]
async fn wallet_bridge_expired_or_zero_attempt_never_needs_a_collector() {
    let (root, keys, address) = wallet();
    let source = binding(root.path().join("absent.sock"));
    let request = RequestAudit::from_wallet(&keys, &address, &source).unwrap();
    assert!(request
        .verify_received(&received(), [1; 16], Instant::now())
        .await
        .unwrap_err()
        .to_string()
        .contains("admission expired"));
    let request = RequestAudit::from_wallet(&keys, &address, &source).unwrap();
    assert!(request
        .verify_received(
            &received(),
            [0; 16],
            Instant::now() + Duration::from_secs(1)
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("attempt must be nonzero"));
}

#[tokio::test]
async fn wallet_bridge_factory_transfers_only_one_request_auditor_without_borrowed_wallet_state() {
    let (root, keys, address) = wallet();
    let source = binding(root.path().join("absent.sock"));
    let (requests, factory) = auditor_channel(1).unwrap();
    let ask = async move {
        let audit = requests
            .request(Instant::now() + Duration::from_secs(1))
            .await
            .unwrap();
        drop(requests);
        audit
    };
    let ((), audit) = tokio::join!(factory.serve(&keys, &address, &source), ask);
    drop(keys);
    // The helper still owns exactly its request-scoped scan copy; it cannot
    // be reused after this consuming call and never reads a scan-key file.
    assert!(audit
        .verify_received(
            &received(),
            [0; 16],
            Instant::now() + Duration::from_secs(1)
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("attempt must be nonzero"));
}

#[tokio::test]
async fn wallet_bridge_unserviced_factory_is_bounded_and_times_out_without_key_material() {
    assert!(auditor_channel(0).is_err());
    assert!(auditor_channel(5).is_err());
    let (requests, _factory) = auditor_channel(1).unwrap();
    assert!(requests
        .request(Instant::now() + Duration::from_millis(10))
        .await
        .err()
        .unwrap()
        .to_string()
        .contains("construction deadline"));
    // The expired queue entry has no key; a second producer cannot create an
    // unbounded backlog while the owner's factory is not being polled.
    assert!(requests
        .request(Instant::now() + Duration::from_secs(1))
        .await
        .err()
        .unwrap()
        .to_string()
        .contains("unavailable or full"));
}
