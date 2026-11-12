//! Real native Noise connection and real SHA256d workers; ONLY the wallet,
//! collector and pool signing authorities below are explicit test fixtures.
use super::*;
use bitcoin::secp256k1::{Keypair as WalletKey, Secp256k1 as WalletSecp, SecretKey};
use spc_cpu_miner::config::SourceAuditConfig;
use spc_scan::native_activation::{
    self as wire, ActivationReceipt, CandidateRequest, ProcessIdentity, Response,
};
use spc_scan::source_feed::{
    self, Checkpoint, CommittedEvidence, Consumed, TraceSeal, VerifiedSourceEvidence,
};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;
use tokio::sync::oneshot;

fn key(n: u8) -> WalletKey {
    WalletKey::from_secret_key(
        &WalletSecp::new(),
        &SecretKey::from_slice(&[n; 32]).unwrap(),
    )
}
#[derive(Clone, Copy, Debug)]
enum Reply {
    Valid,
    ShortLived,
    WrongWallet,
    WrongPool,
    Expired,
    AlteredJob,
}
struct Pending {
    request: CandidateRequest,
    reply: oneshot::Sender<Reply>,
}
enum Change {
    Job(u32),
    Prev(u32),
    Target,
    Extranonce,
    Disconnect,
}

fn reply(request: &CandidateRequest, mode: Reply) -> Response {
    let (wallet, pool, collector) = (key(7), key(8), key(9));
    let mut request = request.clone();
    if matches!(mode, Reply::AlteredJob) {
        request.job.version ^= 1;
    }
    let pool_signer = if matches!(mode, Reply::WrongPool) {
        key(10)
    } else {
        pool
    };
    let mut seal = TraceSeal {
        session: [1; 16],
        seq: 2,
        digest: [3; 32],
        signature: String::new(),
    };
    seal.signature = source_feed::sign(seal.signing_digest(), &pool_signer);
    let mut checkpoint = Checkpoint {
        attempt: request.nonce,
        wire_job_digest: request.job.digest(),
        job_id: request.full_job_id().unwrap(),
        generation: u64::from(request.job.job_id),
        source_watermark: 2,
        history_watermark: 3,
        bundle_digest: [4; 32],
        history_digest: [5; 32],
        expires_at_unix_ms: if matches!(mode, Reply::Expired) {
            1
        } else if matches!(mode, Reply::ShortLived) {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64;
            (now + 500).min(request.expires_at_unix_ms)
        } else {
            request.expires_at_unix_ms
        },
        complete: true,
        pool: seal,
        signature: String::new(),
    };
    checkpoint.signature = source_feed::sign(checkpoint.signing_digest(), &collector);
    let mut consumed = Consumed {
        attempt: request.nonce,
        job_id: checkpoint.job_id,
        generation: checkpoint.generation,
        bundle_digest: checkpoint.bundle_digest,
        history_digest: checkpoint.history_digest,
        checkpoint_digest: checkpoint.signing_digest(),
        window: spc_scan::source_audit::WindowFacts::from_work(9, 0),
        signature: String::new(),
    };
    consumed.signature = source_feed::sign(consumed.signing_digest(), &wallet);
    let committed = CommittedEvidence {
        attempt: request.nonce,
        ack_digest: consumed.signing_digest(),
        signature: source_feed::sign(
            source_feed::committed_digest(request.nonce, consumed.signing_digest()),
            &collector,
        ),
    };
    let evidence = VerifiedSourceEvidence {
        schema: source_feed::SCHEMA.into(),
        checkpoint,
        consumed,
        committed,
    };
    let mut receipt = ActivationReceipt {
        request_digest: request.signing_digest().unwrap(),
        candidate_epoch: request.candidate_epoch,
        nonce: request.nonce,
        evidence,
        wallet_signature: String::new(),
    };
    let signer = if matches!(mode, Reply::WrongWallet) {
        key(11)
    } else {
        wallet
    };
    receipt.wallet_signature = source_feed::sign(
        wire::activation_digest(&request, &receipt).unwrap(),
        &signer,
    );
    Response::Approved {
        schema: wire::SCHEMA.into(),
        capability: wire::CAPABILITY.into(),
        receipt,
    }
}

async fn job(writer: &mut NoiseTcpWriteHalf, id: u32, future: bool) {
    let b: Block = deserialize(&hex::decode(BLOCK_100000).unwrap()).unwrap();
    let cb = serialize(&b.txdata[0]);
    write(
        writer,
        NewExtendedMiningJob {
            channel_id: 41,
            job_id: id,
            min_ntime: Sv2Option::new(if future { None } else { Some(b.header.time) }),
            version: b.header.version.to_consensus() as u32,
            version_rolling_allowed: false,
            merkle_path: Seq0255::new(Vec::<U256>::new()).unwrap(),
            coinbase_tx_prefix: B064K::try_from(&cb[..72]).unwrap(),
            coinbase_tx_suffix: B064K::try_from(&cb[84..]).unwrap(),
        },
        MESSAGE_TYPE_NEW_EXTENDED_MINING_JOB,
        CHANNEL_BIT_NEW_EXTENDED_MINING_JOB,
    )
    .await
    .unwrap();
}
async fn prev(writer: &mut NoiseTcpWriteHalf, id: u32) {
    let b: Block = deserialize(&hex::decode(BLOCK_100000).unwrap()).unwrap();
    write(
        writer,
        SetNewPrevHash {
            channel_id: 41,
            job_id: id,
            prev_hash: U256::from(&[id as u8; 32]),
            min_ntime: b.header.time,
            nbits: 0x207fffff,
        },
        MESSAGE_TYPE_MINING_SET_NEW_PREV_HASH,
        CHANNEL_BIT_MINING_SET_NEW_PREV_HASH,
    )
    .await
    .unwrap();
}

struct Fixture {
    _dir: tempfile::TempDir,
    miner: Miner,
    events: mpsc::Receiver<OutputMessage>,
    requests: mpsc::Receiver<Pending>,
    changes: mpsc::Sender<Change>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}
impl Fixture {
    async fn start() -> Self {
        Self::start_reconnect(false).await
    }
    async fn start_reconnect(reconnect: bool) -> Self {
        let dir = tempfile::Builder::new()
            .prefix("native-gate-")
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir_in("/tmp/opencode")
            .unwrap();
        let socket = dir.path().join("native-audit.sock");
        let wallet = UnixListener::bind(&socket).unwrap();
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
        let meta = std::fs::metadata(&socket).unwrap();
        let (requests_tx, requests) = mpsc::channel(16);
        let wallet_task = tokio::spawn(async move {
            let mut sessions = tokio::task::JoinSet::new();
            loop {
                let (mut stream, _) = wallet.accept().await.unwrap();
                let requests = requests_tx.clone();
                sessions.spawn(async move {
                    let Ok(len) = stream.read_u32_le().await else {
                        return;
                    };
                    let len = len as usize;
                    assert!(len <= wire::MAX_REQUEST_BYTES);
                    let mut bytes = vec![0; len];
                    if stream.read_exact(&mut bytes).await.is_err() {
                        return;
                    }
                    let request = wire::decode_request(&bytes).unwrap();
                    let (tx, rx) = oneshot::channel();
                    if requests
                        .send(Pending {
                            request: request.clone(),
                            reply: tx,
                        })
                        .await
                        .is_err()
                    {
                        return;
                    }
                    let Ok(mode) = rx.await else {
                        return;
                    };
                    let response = wire::encode_response(&reply(&request, mode)).unwrap();
                    // Superseded native sockets close while this slow fixture
                    // may still try to deliver its old, valid signed response.
                    if stream.write_u32_le(response.len() as u32).await.is_ok() {
                        let _ = stream.write_all(&response).await;
                    }
                });
            }
        });
        let tcp = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = tcp.local_addr().unwrap();
        let (changes, mut change_rx) = mpsc::channel(16);
        let pool_task = tokio::spawn(async move {
            loop {
                let (pk, sk) = authority(1);
                let (stream, _) = tcp.accept().await.unwrap();
                let noise = accept_noise_connection(stream, pk, sk, 3600).await.unwrap();
                let (mut reader, mut writer) = noise.into_split();
                reader.read_frame().await.unwrap();
                write(
                    &mut writer,
                    SetupConnectionSuccess {
                        used_version: 2,
                        flags: 0,
                    },
                    MESSAGE_TYPE_SETUP_CONNECTION_SUCCESS,
                    false,
                )
                .await
                .unwrap();
                let mut frame = reader.read_frame().await.unwrap();
                let open: OpenExtendedMiningChannel =
                    binary_sv2::from_bytes(frame.payload()).unwrap();
                write(
                    &mut writer,
                    OpenExtendedMiningChannelSuccess {
                        request_id: open.request_id,
                        channel_id: 41,
                        target: U256::from(&[255; 32]),
                        extranonce_size: 8,
                        extranonce_prefix: B032::new(&[1; 4]).unwrap(),
                        group_channel_id: 9,
                    },
                    MESSAGE_TYPE_OPEN_EXTENDED_MINING_CHANNEL_SUCCESS,
                    false,
                )
                .await
                .unwrap();
                job(&mut writer, 1, true).await;
                prev(&mut writer, 1).await;
                loop {
                    tokio::select! {
                        change=change_rx.recv()=>match change {
                            Some(Change::Job(id))=>job(&mut writer,id,false).await,
                            Some(Change::Prev(id))=>{job(&mut writer,id,true).await;prev(&mut writer,id).await;},
                            Some(Change::Target)=>{write(&mut writer,SetTarget{channel_id:41,maximum_target:U256::from(&[0x7f;32])},MESSAGE_TYPE_SET_TARGET,CHANNEL_BIT_SET_TARGET).await.unwrap();},
                            Some(Change::Extranonce)=>{write(&mut writer,stratum_core::mining_sv2::SetExtranoncePrefix{channel_id:41,extranonce_prefix:B032::new(&[2;4]).unwrap()},
                                stratum_core::mining_sv2::MESSAGE_TYPE_SET_EXTRANONCE_PREFIX,true).await.unwrap();},
                            Some(Change::Disconnect)|None=>break,
                        },
                        frame=reader.read_frame()=>if frame.is_err(){break;},
                    }
                }
            }
        });
        let (mut miner, events) = miner_with_events();
        let (authority, _) = authority(1);
        let mut config = connection(address.to_string(), authority, reconnect);
        config.workers.truncate(1);
        config.workers[0].budget.total_attempts = Some(1000);
        config.identity = spc_crypto::address::encode_sp_address(
            spc_crypto::Network::Regtest,
            &key(7).public_key(),
            &key(12).public_key(),
        )
        .unwrap()
        .into();
        let stat = std::fs::read_to_string("/proc/self/stat").unwrap();
        let ticks = stat
            .rsplit_once(')')
            .unwrap()
            .1
            .split_whitespace()
            .nth(19)
            .unwrap()
            .parse()
            .unwrap();
        config.source_audit = SourceAuditConfig::Required {
            run_id: "fixture-run".into(),
            miner_id: "miner-test".into(),
            wallet_socket: socket,
            wallet_instance: [13; 16],
            wallet_process: ProcessIdentity {
                pid: std::process::id(),
                uid: meta.uid(),
                start_time_ticks: ticks,
                boot_id: std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
                    .unwrap()
                    .trim()
                    .into(),
                executable_sha256: [1; 32],
                run_token_sha256: [2; 32],
                config_sha256: [3; 32],
            },
            wallet_socket_device: meta.dev(),
            wallet_socket_inode: meta.ino(),
            peer_generation: 1,
            connection_config_digest: [0; 32],
            collector_pubkey: key(9).x_only_public_key().0.to_string(),
            pool_pubkey: key(8).x_only_public_key().0.to_string(),
        };
        let digest =
            wire::connection_config_digest(&serde_json::to_value(&config).unwrap()).unwrap();
        if let SourceAuditConfig::Required {
            connection_config_digest,
            ..
        } = &mut config.source_audit
        {
            *connection_config_digest = digest;
        }
        assert_eq!(
            miner
                .execute(command(
                    "audit-start",
                    Operation::AddConnection { connection: config }
                ))
                .await
                .outcome,
            "running"
        );
        Self {
            _dir: dir,
            miner,
            events,
            requests,
            changes,
            tasks: vec![pool_task, wallet_task],
        }
    }
    async fn request(&mut self) -> Pending {
        match tokio::time::timeout(Duration::from_secs(3), self.requests.recv()).await {
            Ok(Some(request)) => request,
            other => {
                let mut events = Vec::new();
                while let Ok(event) = self.events.try_recv() {
                    if let OutputMessage::Event(event) = event {
                        events.push(serde_json::to_value(event).unwrap());
                    }
                }
                panic!(
                    "wallet request missing (timeout={}): snapshot={} events={}",
                    other.is_err(),
                    serde_json::to_value(self.miner.snapshot()).unwrap(),
                    serde_json::json!(events)
                );
            }
        }
    }
    fn attempts(&self) -> u64 {
        self.miner
            .snapshot()
            .workers
            .iter()
            .map(|w| w.attempted.parse::<u64>().unwrap())
            .sum()
    }
    async fn stop(&mut self) {
        tokio::time::timeout(Duration::from_secs(1), self.miner.stop_connections())
            .await
            .unwrap();
        for t in &self.tasks {
            t.abort();
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_commit_is_required_and_events_precede_first_real_hash() {
    let mut f = Fixture::start().await;
    let request = f.request().await;
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert_eq!(f.attempts(), 0);
    assert!(f.miner.snapshot().connections[0].active_job.is_none());
    assert!(f
        .miner
        .snapshot()
        .workers
        .iter()
        .all(|w| w.active_job_id.is_none()));
    while let Ok(message) = f.events.try_recv() {
        if let OutputMessage::Event(event) = message {
            assert!(!matches!(
                event.event,
                MinerEvent::JobActivated { .. }
                    | MinerEvent::SourceAuditCommitted { .. }
                    | MinerEvent::AuditFirstHash { .. }
            ));
        }
    }
    request.reply.send(Reply::Valid).unwrap();
    let mut committed = None;
    let mut activated = None;
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let OutputMessage::Event(e) = f.events.recv().await.unwrap() else {
                continue;
            };
            match e.event {
                MinerEvent::SourceAuditCommitted {
                    candidate_epoch,
                    window,
                    ..
                } => {
                    assert_eq!(candidate_epoch, request.request.candidate_epoch);
                    assert!(window.underfilled);
                    committed = Some(e.event_id);
                }
                MinerEvent::JobActivated { .. } => activated = Some(e.event_id),
                MinerEvent::AuditFirstHash {
                    candidate_epoch, ..
                } => {
                    assert_eq!(candidate_epoch, request.request.candidate_epoch);
                    assert!(
                        committed.unwrap() < activated.unwrap() && activated.unwrap() < e.event_id
                    );
                    break;
                }
                _ => {}
            }
        }
    })
    .await
    .unwrap();
    f.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bad_or_expired_authorities_never_publish_or_hash() {
    for mode in [
        Reply::WrongWallet,
        Reply::WrongPool,
        Reply::Expired,
        Reply::AlteredJob,
    ] {
        let mut f = Fixture::start().await;
        f.request().await.reply.send(mode).unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if f.miner.snapshot().connections[0]
                    .source_audit_state
                    .as_deref()
                    == Some("unknown")
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(f.attempts(), 0);
        assert!(f.miner.snapshot().connections[0].active_job.is_none());
        f.stop().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_native_candidate_change_cancels_slow_approval() {
    for kind in 0..4 {
        let mut f = Fixture::start().await;
        let old = f.request().await;
        match kind {
            0 => f.changes.send(Change::Job(2)).await.unwrap(),
            1 => f.changes.send(Change::Prev(2)).await.unwrap(),
            2 => {
                f.changes.send(Change::Target).await.unwrap();
                f.changes.send(Change::Job(2)).await.unwrap();
            }
            _ => {
                f.changes.send(Change::Extranonce).await.unwrap();
                f.changes.send(Change::Job(2)).await.unwrap();
            }
        }
        let new = f.request().await;
        assert!(new.request.candidate_epoch > old.request.candidate_epoch);
        assert_ne!(new.request.nonce, old.request.nonce);
        let _ = old.reply.send(Reply::Valid);
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(f.attempts(), 0);
        assert!(f.miner.snapshot().connections[0].active_job.is_none());
        if kind == 2 {
            assert_ne!(
                new.request.job.share_target_be,
                old.request.job.share_target_be
            );
        }
        if kind == 3 {
            assert_eq!(new.request.job.pool_extranonce, vec![2; 4]);
        }
        new.reply.send(Reply::Valid).unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            while f.attempts() == 0 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        f.stop().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replacement_quiesces_old_work_and_stop_remains_responsive() {
    let mut f = Fixture::start().await;
    f.request().await.reply.send(Reply::Valid).unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while f.attempts() == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    f.changes.send(Change::Job(2)).await.unwrap();
    let _pending = f.request().await;
    tokio::time::sleep(Duration::from_millis(30)).await;
    let attempts = f.attempts();
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert_eq!(f.attempts(), attempts);
    assert!(f.miner.snapshot().connections[0].active_job.is_none());
    f.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn disconnect_during_wallet_wait_cannot_activate() {
    let mut f = Fixture::start().await;
    let old = f.request().await;
    f.changes.send(Change::Disconnect).await.unwrap();
    tokio::time::sleep(Duration::from_millis(40)).await;
    let _ = old.reply.send(Reply::Valid);
    tokio::time::sleep(Duration::from_millis(40)).await;
    assert_eq!(f.attempts(), 0);
    f.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn expired_active_work_stops_until_a_fresh_receipt_arrives() {
    let mut f = Fixture::start().await;
    let first = f.request().await;
    first.reply.send(Reply::ShortLived).unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while f.attempts() == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let renewal = f.request().await;
    assert!(renewal.request.candidate_epoch > first.request.candidate_epoch);
    assert_ne!(renewal.request.nonce, first.request.nonce);
    assert_eq!(renewal.request.job.digest(), first.request.job.digest());
    tokio::time::sleep(Duration::from_millis(30)).await;
    let stopped = f.attempts();
    assert!(stopped > 0);
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert_eq!(f.attempts(), stopped);
    assert!(f.miner.snapshot().connections[0].active_job.is_none());
    renewal.reply.send(Reply::Valid).unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while f.attempts() == stopped {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    f.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_evidence_retries_current_job_with_a_new_attempt() {
    let mut f = Fixture::start().await;
    let first = f.request().await;
    first.reply.send(Reply::WrongPool).unwrap();
    let retry = f.request().await;
    assert_eq!(f.attempts(), 0);
    assert!(retry.request.candidate_epoch > first.request.candidate_epoch);
    assert_ne!(retry.request.nonce, first.request.nonce);
    assert_eq!(retry.request.job.digest(), first.request.job.digest());
    retry.reply.send(Reply::Valid).unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while f.attempts() == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    f.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn target_and_extranonce_alone_revoke_active_work() {
    for change in [Change::Target, Change::Extranonce] {
        let mut f = Fixture::start().await;
        f.request().await.reply.send(Reply::Valid).unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            while f.attempts() == 0 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        f.changes.send(change).await.unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            while f.miner.snapshot().connections[0].active_job.is_some() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;
        let attempts = f.attempts();
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert_eq!(f.attempts(), attempts);
        assert!(f
            .miner
            .snapshot()
            .workers
            .iter()
            .all(|w| w.active_job_id.is_none()));
        assert!(f.requests.try_recv().is_err());
        f.changes.send(Change::Job(2)).await.unwrap();
        let next = f.request().await;
        assert_eq!(f.attempts(), attempts);
        next.reply.send(Reply::Valid).unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            while f.attempts() == attempts {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        f.stop().await;
    }
}

#[test]
fn materialized_connection_digest_rejects_config_tampering() {
    let (authority, _) = authority(1);
    let mut config = connection("127.0.0.1:34254".into(), authority, false);
    config.identity = spc_crypto::address::encode_sp_address(
        spc_crypto::Network::Regtest,
        &key(7).public_key(),
        &key(12).public_key(),
    )
    .unwrap()
    .into();
    config.source_audit = SourceAuditConfig::Required {
        run_id: "fixture-run".into(),
        miner_id: "miner-test".into(),
        wallet_socket: "/private/wallet/native-audit.sock".into(),
        wallet_instance: [13; 16],
        wallet_process: ProcessIdentity {
            pid: 123,
            uid: 1000,
            start_time_ticks: 456,
            boot_id: "00000000-0000-0000-0000-000000000001".into(),
            executable_sha256: [1; 32],
            run_token_sha256: [2; 32],
            config_sha256: [3; 32],
        },
        wallet_socket_device: 1,
        wallet_socket_inode: 2,
        peer_generation: 1,
        connection_config_digest: [0; 32],
        collector_pubkey: key(9).x_only_public_key().0.to_string(),
        pool_pubkey: key(8).x_only_public_key().0.to_string(),
    };
    let digest = wire::connection_config_digest(&serde_json::to_value(&config).unwrap()).unwrap();
    if let SourceAuditConfig::Required {
        connection_config_digest,
        ..
    } = &mut config.source_audit
    {
        *connection_config_digest = digest;
    }
    config.validate().unwrap();
    for mutation in 0..5 {
        let mut changed = config.clone();
        match mutation {
            0 => changed.endpoint.push('1'),
            1 => changed.workers[0].budget.hashes_per_second = Some(99),
            2 => changed.reconnect.handshake_timeout_ms += 1,
            3 => {
                if let SourceAuditConfig::Required {
                    wallet_instance, ..
                } = &mut changed.source_audit
                {
                    wallet_instance[0] ^= 1;
                }
            }
            _ => {
                if let SourceAuditConfig::Required { wallet_process, .. } =
                    &mut changed.source_audit
                {
                    wallet_process.config_sha256[0] ^= 1;
                }
            }
        }
        assert!(changed.validate().is_err(), "mutation {mutation}");
    }
    let mut missing = serde_json::to_value(&config).unwrap();
    missing.as_object_mut().unwrap().remove("source_audit");
    assert!(serde_json::from_value::<ConnectionConfig>(missing).is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reconnect_rejects_the_previous_session_receipt() {
    let mut f = Fixture::start_reconnect(true).await;
    let old = f.request().await;
    f.changes.send(Change::Disconnect).await.unwrap();
    let new = f.request().await;
    assert!(new.request.session_epoch > old.request.session_epoch);
    assert_ne!(new.request.nonce, old.request.nonce);
    let _ = old.reply.send(Reply::Valid);
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(f.attempts(), 0);
    new.reply.send(Reply::Valid).unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while f.attempts() == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    f.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pause_is_responsive_while_proof_waits_and_prevents_hashing() {
    let mut f = Fixture::start().await;
    let pending = f.request().await;
    let ids = f
        .miner
        .snapshot()
        .workers
        .iter()
        .map(|w| w.id.clone())
        .collect();
    let result = tokio::time::timeout(
        Duration::from_millis(200),
        f.miner.execute(command(
            "audit-pause",
            Operation::PauseWorkers { worker_ids: ids },
        )),
    )
    .await
    .unwrap();
    assert_eq!(result.outcome, "succeeded");
    pending.reply.send(Reply::Valid).unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(f.attempts(), 0);
    let ids = f
        .miner
        .snapshot()
        .workers
        .iter()
        .map(|w| w.id.clone())
        .collect();
    assert_eq!(
        f.miner
            .execute(command(
                "audit-resume",
                Operation::ResumeWorkers { worker_ids: ids }
            ))
            .await
            .outcome,
        "succeeded"
    );
    tokio::time::timeout(Duration::from_secs(3), async {
        while f.attempts() == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    f.stop().await;
}
