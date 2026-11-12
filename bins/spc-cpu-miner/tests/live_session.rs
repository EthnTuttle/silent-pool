use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bitcoin::consensus::{deserialize, serialize};
use bitcoin::hashes::Hash;
use bitcoin::{Block, BlockHash};
use secp256k1_legacy::{Keypair, Secp256k1};
use spc_cpu_miner::budget::HashBudget;
use spc_cpu_miner::config::{ConnectionConfig, ReconnectConfig, WorkerConfig};
use spc_cpu_miner::connection::SubmissionTracker;
use spc_cpu_miner::control::{Command, Operation, CONTROL_PROTOCOL, CONTROL_VERSION};
use spc_cpu_miner::hash::{assemble_coinbase, assemble_header, NtimePolicy};
use spc_cpu_miner::miner::Miner;
use spc_cpu_miner::report::{
    ConnectionSnapshot, EventSink, MinerEvent, OutputMessage, SubmissionSnapshot, SubmissionStatus,
};
use spc_cpu_miner::session::{JobTemplate, ProtocolState};
use spc_cpu_miner::target::{
    decode_compact_target, meets_target, target_from_le_bytes, target_le_hex,
};
use spc_cpu_miner::worker::WorkerHandle;
use stratum_apps::key_utils::{Secp256k1PublicKey, Secp256k1SecretKey};
use stratum_apps::network_helpers::accept_noise_connection;
use stratum_apps::network_helpers::noise_stream::NoiseTcpWriteHalf;
use stratum_core::binary_sv2::{self, Seq0255, Str0255, Sv2Option, B032, B064K, U256};
use stratum_core::common_messages_sv2::{
    SetupConnectionSuccess, MESSAGE_TYPE_SETUP_CONNECTION, MESSAGE_TYPE_SETUP_CONNECTION_SUCCESS,
};
use stratum_core::framing_sv2::framing::MessageFrame;
use stratum_core::mining_sv2::{
    NewExtendedMiningJob, OpenExtendedMiningChannel, OpenExtendedMiningChannelSuccess,
    SetNewPrevHash, SetTarget, SubmitSharesError, SubmitSharesExtended, SubmitSharesSuccess,
    CHANNEL_BIT_MINING_SET_NEW_PREV_HASH, CHANNEL_BIT_NEW_EXTENDED_MINING_JOB,
    CHANNEL_BIT_SET_TARGET, CHANNEL_BIT_SUBMIT_SHARES_ERROR, CHANNEL_BIT_SUBMIT_SHARES_SUCCESS,
    MESSAGE_TYPE_MINING_SET_NEW_PREV_HASH, MESSAGE_TYPE_NEW_EXTENDED_MINING_JOB,
    MESSAGE_TYPE_OPEN_EXTENDED_MINING_CHANNEL, MESSAGE_TYPE_OPEN_EXTENDED_MINING_CHANNEL_SUCCESS,
    MESSAGE_TYPE_SET_TARGET, MESSAGE_TYPE_SUBMIT_SHARES_ERROR, MESSAGE_TYPE_SUBMIT_SHARES_EXTENDED,
    MESSAGE_TYPE_SUBMIT_SHARES_SUCCESS,
};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

#[path = "native_gate/connection.rs"]
mod native_audit;

const BLOCK_100000: &str = "0200000035ab154183570282ce9afc0b494c9fc6a3cfea05aa8c1add2ecc56490000000038ba3d78e4500a5a7570dbe61960398add4410d278b21cd9708e6d9743f374d544fc055227f1001c29c1ea3b0101000000010000000000000000000000000000000000000000000000000000000000000000ffffffff3703a08601000427f1001c046a510100522cfabe6d6d0000000000000000000068692066726f6d20706f6f6c7365727665726aac1eeeed88ffffffff0100f2052a010000001976a914912e2b234f941f30b18afbb4fa46171214bf66c888ac00000000";

fn authority(secret: u8) -> (Secp256k1PublicKey, Secp256k1SecretKey) {
    let secp = Secp256k1::new();
    let mut bytes = [0u8; 32];
    bytes[31] = secret;
    let keypair = Keypair::from_seckey_slice(&secp, &bytes).unwrap();
    (
        Secp256k1PublicKey(keypair.x_only_public_key().0),
        Secp256k1SecretKey(keypair.secret_key()),
    )
}

fn connection(
    endpoint: String,
    authority: Secp256k1PublicKey,
    reconnect: bool,
) -> ConnectionConfig {
    ConnectionConfig {
        source_audit: spc_cpu_miner::config::SourceAuditConfig::Unaudited {},
        id: "conn-live".into(),
        endpoint,
        identity: "tsp1q-independent-test-fixture".into(),
        authority_public_key: hex::encode(authority.into_bytes()),
        min_extranonce_size: 4,
        reconnect: ReconnectConfig {
            enabled: reconnect,
            initial_backoff_ms: 10,
            max_backoff_ms: 40,
            handshake_timeout_ms: 2_000,
        },
        workers: (0..2)
            .map(|index| WorkerConfig {
                id: format!("worker-{index}").into(),
                budget: HashBudget {
                    hashes_per_second: Some(100),
                    total_attempts: Some(100_000),
                },
                ntime_policy: NtimePolicy::Hold,
                max_ntime_roll_seconds: 600,
                batch_size: 1,
            })
            .collect(),
    }
}

fn miner_with_events() -> (Miner, mpsc::Receiver<OutputMessage>) {
    let (tx, rx) = mpsc::channel(4096);
    let miner = Miner::new(
        "miner-test".into(),
        EventSink::new(tx, "miner-test".into(), Instant::now()),
    );
    (miner, rx)
}

fn miner() -> Miner {
    miner_with_events().0
}

fn command(id: &str, operation: Operation) -> Command {
    Command {
        protocol: CONTROL_PROTOCOL.into(),
        version: CONTROL_VERSION,
        command_id: id.into(),
        operation,
    }
}

async fn write<T>(
    writer: &mut NoiseTcpWriteHalf,
    message: T,
    message_type: u8,
    channel: bool,
) -> Result<(), String>
where
    T: binary_sv2::Serialize + binary_sv2::GetSize,
{
    let extension_type = 0;
    let frame = MessageFrame::from_message(message, message_type, extension_type, channel)
        .map_err(|error| error.to_string())?;
    writer
        .write_frame(frame)
        .await
        .map_err(|error| error.to_string())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wrong_pinned_authority_never_reaches_channel_open() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (server_pk, server_sk) = authority(1);
    let (wrong_pk, _) = authority(2);
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let noise = accept_noise_connection(socket, server_pk, server_sk, 3600).await;
        // A peer presenting a different pinned authority may fail on either
        // side of the handshake, but must never produce an application frame.
        if let Ok((mut read, _write)) = noise.map(|stream| stream.into_split()) {
            return matches!(
                tokio::time::timeout(Duration::from_millis(300), read.read_frame()).await,
                Ok(Ok(_))
            );
        }
        false
    });
    let mut miner = miner();
    let result = miner
        .execute(command(
            "wrong-authority",
            Operation::AddConnection {
                connection: connection(address.to_string(), wrong_pk, false),
            },
        ))
        .await;
    assert_eq!(result.outcome, "running", "{result:?}");
    tokio::time::sleep(Duration::from_millis(500)).await;
    let snapshot = miner.snapshot();
    assert_eq!(snapshot.connections.len(), 1);
    assert_eq!(snapshot.connections[0].channel_id, None);
    assert_eq!(snapshot.connections[0].counters.submitted, 0);
    assert!(!server.await.unwrap(), "application data crossed a bad pin");
    miner.stop_connections().await;
}

#[derive(Debug)]
struct SeenShare {
    sequence: u32,
    header: [u8; 80],
    hash: BlockHash,
    unique_extranonce: Vec<u8>,
}

async fn run_pool_with_reply_mode(
    listener: Arc<TcpListener>,
    batched_success: bool,
) -> Result<Vec<SeenShare>, String> {
    let (pk, sk) = authority(1);
    let (socket, _) = listener.accept().await.map_err(|error| error.to_string())?;
    let noise = accept_noise_connection(socket, pk, sk, 3600)
        .await
        .map_err(|error| error.to_string())?;
    let (mut read, mut writer) = noise.into_split();

    let frame = read.read_frame().await.map_err(|error| error.to_string())?;
    assert_eq!(frame.header().msg_type(), MESSAGE_TYPE_SETUP_CONNECTION);
    write(
        &mut writer,
        SetupConnectionSuccess {
            used_version: 2,
            flags: 0,
        },
        MESSAGE_TYPE_SETUP_CONNECTION_SUCCESS,
        false,
    )
    .await?;

    let mut open_frame = read.read_frame().await.map_err(|error| error.to_string())?;
    assert_eq!(
        open_frame.header().msg_type(),
        MESSAGE_TYPE_OPEN_EXTENDED_MINING_CHANNEL
    );
    let open: OpenExtendedMiningChannel =
        binary_sv2::from_bytes(open_frame.payload()).map_err(|error| error.to_string())?;
    assert_eq!(
        open.user_identity.as_bytes(),
        b"tsp1q-independent-test-fixture"
    );
    assert_eq!(open.min_extranonce_size, 4);

    let mut initial_target = [0u8; 32];
    initial_target[31] = 0x20;
    let prefix = [0xa5, 0x5a, 0xc3, 0x3c];
    write(
        &mut writer,
        OpenExtendedMiningChannelSuccess {
            request_id: open.request_id,
            channel_id: 41,
            target: U256::from(&initial_target),
            extranonce_size: 8,
            extranonce_prefix: B032::new(&prefix).map_err(|error| error.to_string())?,
            group_channel_id: 9,
        },
        MESSAGE_TYPE_OPEN_EXTENDED_MINING_CHANNEL_SUCCESS,
        false,
    )
    .await?;

    let block: Block = deserialize(&hex::decode(BLOCK_100000).unwrap()).unwrap();
    let coinbase = serialize(&block.txdata[0]);
    let split = 72;
    let job = NewExtendedMiningJob {
        channel_id: 41,
        job_id: 77,
        min_ntime: Sv2Option::new(None),
        version: block.header.version.to_consensus() as u32,
        version_rolling_allowed: false,
        merkle_path: Seq0255::new(Vec::<U256>::new()).unwrap(),
        coinbase_tx_prefix: B064K::try_from(&coinbase[..split])
            .map_err(|error| error.to_string())?,
        coinbase_tx_suffix: B064K::try_from(&coinbase[split + 12..])
            .map_err(|error| error.to_string())?,
    };
    write(
        &mut writer,
        job,
        MESSAGE_TYPE_NEW_EXTENDED_MINING_JOB,
        CHANNEL_BIT_NEW_EXTENDED_MINING_JOB,
    )
    .await?;

    // The future job must pick up this activation-time target, not the harder
    // target announced on open. The active job must ignore the later update.
    let mut activation_target = [0xffu8; 32];
    activation_target[31] = 0x7f;
    write(
        &mut writer,
        SetTarget {
            channel_id: 41,
            maximum_target: U256::from(&activation_target),
        },
        MESSAGE_TYPE_SET_TARGET,
        CHANNEL_BIT_SET_TARGET,
    )
    .await?;
    write(
        &mut writer,
        SetNewPrevHash {
            channel_id: 41,
            job_id: 77,
            prev_hash: U256::from(&[0x19; 32]),
            min_ntime: block.header.time,
            nbits: 0x200f_ffff,
        },
        MESSAGE_TYPE_MINING_SET_NEW_PREV_HASH,
        CHANNEL_BIT_MINING_SET_NEW_PREV_HASH,
    )
    .await?;
    // An additional future job receipt must not re-activate the current job.
    write(
        &mut writer,
        NewExtendedMiningJob {
            channel_id: 41,
            job_id: 78,
            min_ntime: Sv2Option::new(None),
            version: block.header.version.to_consensus() as u32,
            version_rolling_allowed: false,
            merkle_path: Seq0255::new(Vec::<U256>::new()).unwrap(),
            coinbase_tx_prefix: B064K::try_from(&coinbase[..split])
                .map_err(|error| error.to_string())?,
            coinbase_tx_suffix: B064K::try_from(&coinbase[split + 12..])
                .map_err(|error| error.to_string())?,
        },
        MESSAGE_TYPE_NEW_EXTENDED_MINING_JOB,
        CHANNEL_BIT_NEW_EXTENDED_MINING_JOB,
    )
    .await?;
    let mut later_target = [0u8; 32];
    later_target[31] = 0x0f;
    write(
        &mut writer,
        SetTarget {
            channel_id: 41,
            maximum_target: U256::from(&later_target),
        },
        MESSAGE_TYPE_SET_TARGET,
        CHANNEL_BIT_SET_TARGET,
    )
    .await?;

    let activation_target = target_from_le_bytes(&activation_target).unwrap();
    let network_target = decode_compact_target(0x200f_ffff).unwrap();
    let mut seen = Vec::new();
    let mut unique_headers = BTreeSet::new();
    while seen.len() < 3 {
        let mut frame = read.read_frame().await.map_err(|error| error.to_string())?;
        if frame.header().msg_type() != MESSAGE_TYPE_SUBMIT_SHARES_EXTENDED {
            continue;
        }
        let submit: SubmitSharesExtended =
            binary_sv2::from_bytes(frame.payload()).map_err(|error| error.to_string())?;
        assert_eq!(submit.channel_id, 41);
        assert_eq!(submit.job_id, 77);
        assert_eq!(submit.sequence_number as usize, seen.len());
        assert_eq!(submit.extranonce.as_bytes().len(), 8);
        let coinbase = assemble_coinbase(
            &coinbase[..split],
            &prefix,
            submit.extranonce.as_bytes(),
            &coinbase[split + 12..],
        );
        let header = assemble_header(
            submit.version,
            [0x19; 32],
            &coinbase,
            &[],
            submit.ntime,
            0x200f_ffff,
            submit.nonce,
        );
        let hash = BlockHash::from_byte_array(
            bitcoin::hashes::sha256d::Hash::hash(&header).to_byte_array(),
        );
        assert!(
            meets_target(&hash, activation_target),
            "submitted real hash must meet activation target"
        );
        assert!(unique_headers.insert(header), "duplicate header detected");
        seen.push(SeenShare {
            sequence: submit.sequence_number,
            header,
            hash,
            unique_extranonce: submit.extranonce.as_bytes().to_vec(),
        });
        let _ = network_target;
    }

    if batched_success {
        for (sequence, count, sum) in [(2, 3, 777), (2, 3, 777), (2, 0, 0)] {
            write(
                &mut writer,
                SubmitSharesSuccess {
                    channel_id: 41,
                    last_sequence_number: sequence,
                    new_submits_accepted_count: count,
                    new_shares_sum: sum,
                },
                MESSAGE_TYPE_SUBMIT_SHARES_SUCCESS,
                CHANNEL_BIT_SUBMIT_SHARES_SUCCESS,
            )
            .await?;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        return Ok(seen);
    }

    // Correlate several outstanding submissions independently. Success remains
    // ingress-only evidence; the sequence 1 rejection is an exact error reply.
    for (sequence, success) in [(2, true), (1, false), (0, true)] {
        if success {
            write(
                &mut writer,
                SubmitSharesSuccess {
                    channel_id: 41,
                    last_sequence_number: sequence,
                    new_submits_accepted_count: 1,
                    new_shares_sum: 1,
                },
                MESSAGE_TYPE_SUBMIT_SHARES_SUCCESS,
                CHANNEL_BIT_SUBMIT_SHARES_SUCCESS,
            )
            .await?;
        } else {
            let code = Str0255::try_from("duplicate-share").unwrap();
            write(
                &mut writer,
                SubmitSharesError {
                    channel_id: 41,
                    sequence_number: sequence,
                    error_code: code,
                },
                MESSAGE_TYPE_SUBMIT_SHARES_ERROR,
                CHANNEL_BIT_SUBMIT_SHARES_ERROR,
            )
            .await?;
        }
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    Ok(seen)
}

async fn run_independent_pool(listener: Arc<TcpListener>) -> Result<Vec<SeenShare>, String> {
    let result = run_pool_with_reply_mode(listener, false).await;
    if let Err(error) = &result {
        eprintln!("independent pool failed: {error}");
    }
    result
}

async fn run_batching_pool(listener: Arc<TcpListener>) -> Result<Vec<SeenShare>, String> {
    run_pool_with_reply_mode(listener, true).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_session_hashes_real_future_job_and_correlates_replies() {
    let listener = Arc::new(TcpListener::bind("127.0.0.1:0").await.unwrap());
    let address = listener.local_addr().unwrap();
    let (pk, _) = authority(1);
    let pool = tokio::spawn(run_independent_pool(Arc::clone(&listener)));
    let (mut miner, mut event_rx) = miner_with_events();
    let added = miner
        .execute(command(
            "add-live",
            Operation::AddConnection {
                connection: connection(address.to_string(), pk, true),
            },
        ))
        .await;
    assert_eq!(added.outcome, "running", "{added:?}");

    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let snapshot = miner.snapshot();
            if snapshot.connections[0].counters.ingress_success_replies == 2
                && snapshot.connections[0].counters.ingress_rejected == 1
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        while let Ok(message) = event_rx.try_recv() {
            eprintln!("event={message:?}");
        }
        panic!(
            "timed out waiting for real share ingress replies: {:?}",
            miner.snapshot()
        )
    });

    let snapshot = miner.snapshot();
    let connection = &snapshot.connections[0];
    assert!(connection.counters.submitted >= 3);
    let active = connection.active_job.as_ref().unwrap();
    let activation_target = {
        let mut bytes = [0xffu8; 32];
        bytes[31] = 0x7f;
        target_from_le_bytes(&bytes).unwrap()
    };
    assert_eq!(active.share_target_le, target_le_hex(activation_target));
    assert_ne!(
        connection.channel_target_le.as_deref(),
        Some(active.share_target_le.as_str())
    );
    for share in &connection.submissions {
        assert_eq!(share.accounted, "unknown");
        match share.sequence_number {
            0 | 2 => assert_eq!(share.status, SubmissionStatus::IngressAck),
            1 => {
                assert_eq!(share.status, SubmissionStatus::IngressRejected);
                assert_eq!(share.error_code.as_deref(), Some("duplicate-share"));
            }
            _ => assert!(matches!(
                share.status,
                SubmissionStatus::Submitted | SubmissionStatus::TransportUnknown
            )),
        }
    }
    assert!(snapshot
        .workers
        .iter()
        .all(|worker| worker.attempted.parse::<u64>().unwrap() > 0));
    let measured: Vec<_> = snapshot
        .workers
        .iter()
        .map(|worker| {
            (
                worker.id.clone(),
                worker.achieved_hps,
                worker.elapsed_ms,
                worker.attempted.clone(),
            )
        })
        .collect();
    assert!(
        snapshot.workers.iter().all(|worker| {
            worker
                .budget_hps
                .is_none_or(|budget| worker.achieved_hps <= budget)
        }),
        "measured={measured:?}"
    );

    let mut activations = 0;
    while let Ok(message) = event_rx.try_recv() {
        if let OutputMessage::Event(envelope) = message {
            if let MinerEvent::JobActivated { job, .. } = envelope.event {
                assert_eq!(job.job_id, 77);
                activations += 1;
            }
        }
    }
    assert_eq!(
        activations, 1,
        "future job receipt and SetTarget re-activated the active job"
    );
    assert!(snapshot
        .workers
        .iter()
        .all(|worker| worker.stale_cancelled == 0));

    let first_session = snapshot.connections[0].session_slot;
    let first = pool.await.unwrap().unwrap();
    assert_eq!(first.len(), 3);
    assert!(first.iter().any(|share| share.hash != first[0].hash));
    assert_eq!(
        first.iter().map(|share| share.sequence).collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
    assert!(first
        .iter()
        .all(|share| !share.unique_extranonce.is_empty()));
    assert!(first.iter().all(|share| share.header.len() == 80));

    // The same pool/job bytes are offered after a real reconnect. New session
    // state must not reuse the old header/extranonce or confuse sequence replies.
    let second_pool = tokio::spawn(run_independent_pool(Arc::clone(&listener)));
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let snapshot = miner.snapshot();
            let connection = &snapshot.connections[0];
            if connection.session_slot != first_session
                && connection.counters.ingress_success_replies >= 4
                && connection.counters.ingress_rejected >= 2
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("timed out waiting for reconnect shares");
    let second = second_pool.await.unwrap().unwrap();
    assert_ne!(first[0].unique_extranonce, second[0].unique_extranonce);
    assert_ne!(first[0].header, second[0].header);
    let reconnected = miner.snapshot();
    assert!(reconnected.connections[0].counters.reconnects >= 1);
    assert!(reconnected.connections[0]
        .submissions
        .iter()
        .any(|share| { Some(share.session_slot) != first_session && share.sequence_number == 0 }));
    miner.stop_connections().await;
}

fn activated_protocol_job(job_id: u32) -> Arc<JobTemplate> {
    let mut target = [0xffu8; 32];
    target[31] = 0x7f;
    let mut protocol =
        ProtocolState::new(41, 9, target_from_le_bytes(&target).unwrap(), vec![], 4).unwrap();
    protocol
        .on_new_extended(NewExtendedMiningJob {
            channel_id: 41,
            job_id,
            min_ntime: Sv2Option::new(None),
            version: 0x2000_0000,
            version_rolling_allowed: false,
            merkle_path: Seq0255::new(Vec::<U256>::new()).unwrap(),
            coinbase_tx_prefix: B064K::try_from(&[1u8; 50][..]).unwrap(),
            coinbase_tx_suffix: B064K::try_from(&[2u8; 50][..]).unwrap(),
        })
        .unwrap();
    protocol
        .on_set_new_prev_hash(SetNewPrevHash {
            channel_id: 41,
            job_id,
            prev_hash: U256::from(&[0x19; 32]),
            min_ntime: 1_700_000_000,
            nbits: 0x207f_ffff,
        })
        .unwrap();
    Arc::new(protocol.active_job().unwrap().clone())
}

fn submission_for(job: &JobTemplate, session: u16, sequence: u32) -> SubmissionSnapshot {
    let header = assemble_header(
        job.version,
        job.prev_hash,
        &[sequence.wrapping_add(1) as u8],
        &[],
        job.min_ntime,
        job.nbits,
        sequence,
    );
    let hash =
        BlockHash::from_byte_array(bitcoin::hashes::sha256d::Hash::hash(&header).to_byte_array());
    SubmissionSnapshot::new(
        &"cap-test".into(),
        &"worker-test".into(),
        session,
        sequence,
        job,
        header,
        hash,
        &session.to_be_bytes(),
        sequence,
        job.min_ntime,
        job.version,
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interruptible_pacing_stop_and_budget_rebase_are_bounded() {
    let job = activated_protocol_job(707);
    let (found_tx, mut found_rx) = mpsc::channel(16);
    let drain = tokio::spawn(async move { while found_rx.recv().await.is_some() {} });
    let (_events_tx, _events_rx) = mpsc::channel(16);
    let worker = WorkerHandle::start(
        0,
        0,
        1,
        WorkerConfig {
            id: "paced".into(),
            budget: HashBudget {
                hashes_per_second: Some(100_000),
                total_attempts: Some(100_000),
            },
            ntime_policy: NtimePolicy::Hold,
            max_ntime_roll_seconds: 600,
            batch_size: 1,
        },
        "conn-paced".into(),
        found_tx,
        EventSink::new(_events_tx, "miner-paced".into(), Instant::now()),
    );
    worker.publish(0, 4, job);
    tokio::time::timeout(Duration::from_secs(2), async {
        while worker.snapshot().attempted.parse::<u64>().unwrap() == 0 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("worker did not hash a real job: {:?}", worker.snapshot()));
    worker.set_budget(HashBudget {
        hashes_per_second: Some(1),
        total_attempts: Some(100_000),
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    let stop_started = Instant::now();
    worker.stop();
    worker.join();
    assert!(
        stop_started.elapsed() < Duration::from_millis(100),
        "stop took {:?}",
        stop_started.elapsed()
    );
    drain.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn batched_success_resolves_prefix_and_duplicate_ack_is_harmless() {
    let listener = Arc::new(TcpListener::bind("127.0.0.1:0").await.unwrap());
    let address = listener.local_addr().unwrap();
    let (pk, _) = authority(1);
    let pool = tokio::spawn(run_batching_pool(Arc::clone(&listener)));
    let mut config = connection(address.to_string(), pk, false);
    config.workers.truncate(1);
    config.workers[0].budget = HashBudget {
        hashes_per_second: Some(1),
        total_attempts: Some(32),
    };
    let mut miner = miner();
    let added = miner
        .execute(command(
            "add-batch",
            Operation::AddConnection { connection: config },
        ))
        .await;
    assert_eq!(added.outcome, "running");

    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if miner.snapshot().connections[0]
                .counters
                .ingress_success_replies
                >= 3
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("timed out waiting for batched and repeated ACKs");
    pool.await.unwrap().unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    let snapshot = miner.snapshot();
    let connection = &snapshot.connections[0];
    assert_eq!(connection.counters.ingress_accepted_reported, 3);
    assert_eq!(connection.counters.submitted, 3);
    for share in &connection.submissions {
        assert!(share.sequence_number <= 2);
        assert_eq!(
            share.status,
            SubmissionStatus::IngressAck,
            "share {} was not resolved",
            share.sequence_number
        );
        assert_ne!(share.status, SubmissionStatus::TransportUnknown);
    }
    miner.stop_connections().await;
}

#[test]
fn tracker_caps_pending_and_rejects_inconsistent_batches() {
    let job = activated_protocol_job(808);
    let mut tracker = SubmissionTracker::new(2);
    for sequence in 0..4 {
        let evicted = tracker.insert(submission_for(&job, 7, sequence)).unwrap();
        if sequence < 2 {
            assert!(evicted.is_none());
        } else {
            assert_eq!(evicted.unwrap().sequence_number, sequence - 2);
        }
    }
    assert_eq!(tracker.pending_sequences(), vec![2, 3]);
    let acknowledged = tracker.complete_success(3, 1).unwrap();
    assert_eq!(acknowledged.len(), 1);
    assert_eq!(tracker.complete_success(3, 1).unwrap().len(), 0);
    assert_eq!(tracker.complete_success(3, 0).unwrap().len(), 0);
    let pending: Vec<_> = tracker
        .finish_transport()
        .into_iter()
        .map(|share| share.sequence_number)
        .collect();
    assert_eq!(pending, vec![2], "sequence 3 was already acknowledged");

    let mut capped = SubmissionTracker::new(4);
    for sequence in 0..4 {
        capped.insert(submission_for(&job, 7, sequence)).unwrap();
    }
    assert!(capped.complete_success(2, 4).is_err());
    assert_eq!(capped.pending_sequences(), vec![0, 1, 2, 3]);
    assert_eq!(capped.complete_success(3, 4).unwrap().len(), 4);
    assert!(capped.pending_sequences().is_empty());

    let mut edge = SubmissionTracker::new(1);
    edge.insert(submission_for(&job, 9, u32::MAX)).unwrap();
    assert_eq!(edge.complete_success(u32::MAX, 1).unwrap().len(), 1);
    assert_eq!(edge.complete_success(u32::MAX, 1).unwrap().len(), 0);
    assert!(edge.finish_transport().is_empty());
    assert!(
        u32::MAX.checked_add(1).is_none(),
        "session sequence wrap is closed"
    );

    let mut late = SubmissionTracker::new(1);
    late.insert(submission_for(&job, 7, 0)).unwrap();
    late.insert(submission_for(&job, 7, 1)).unwrap();
    assert!(late.complete_success(0, 1).unwrap().is_empty());
    assert_eq!(late.drain_late_evicted(), vec![0]);
    assert!(late.complete_error(0).unwrap().is_none());
    assert_eq!(late.drain_late_evicted(), vec![0]);
    assert!(late.complete_error(999).is_err());

    let mut history = ConnectionSnapshot::new(&"cap-test".into(), "local", &"identity".into());
    for sequence in 0..=256 {
        let evicted = history.push_submission(submission_for(&job, 7, sequence));
        if sequence < 256 {
            assert!(evicted.is_none());
        } else {
            assert_eq!(evicted.unwrap().sequence_number, 0);
        }
    }
    assert_eq!(history.submissions.len(), 256);
    assert_eq!(history.submissions.front().unwrap().sequence_number, 1);

    let mut ambiguous = SubmissionTracker::new(4);
    for sequence in 0..4 {
        ambiguous.insert(submission_for(&job, 7, sequence)).unwrap();
    }
    // Only last-correct 3 is identifiable in this case; selecting 0 as its
    // predecessor would invent an acceptance under a smaller batch count.
    assert!(ambiguous.complete_success(3, 2).is_err());
    assert_eq!(ambiguous.pending_sequences(), vec![0, 1, 2, 3]);
    assert!(ambiguous
        .complete_success(2, 1)
        .unwrap()
        .iter()
        .all(|share| share.sequence_number == 2));

    let mut consistent = SubmissionTracker::new(5);
    for sequence in 0..5 {
        consistent
            .insert(submission_for(&job, 7, sequence))
            .unwrap();
    }
    // Sequence 2 has an explicit rejection. The remaining no-error prefix 0,1
    // is fully covered, last-correct 3 is exact, and later 4 stays pending.
    consistent.complete_error(2).unwrap();
    assert_eq!(consistent.complete_success(3, 3).unwrap().len(), 3);
    assert_eq!(consistent.pending_sequences(), vec![4]);

    let history_only = ConnectionSnapshot::new(&"cap-test".into(), "local", &"identity".into());
    let (events_tx, mut events_rx) = mpsc::channel(8);
    let events = EventSink::new(events_tx, "miner-test".into(), Instant::now());
    let shared = Arc::new(std::sync::Mutex::new(history_only.clone()));
    let first_pending = submission_for(&job, 7, 0);
    spc_cpu_miner::connection::mark_evicted(
        &shared,
        &events,
        &first_pending,
        "known-history-cap-evicted",
    );
    match events_rx.try_recv().unwrap() {
        OutputMessage::Event(envelope) => match envelope.event {
            MinerEvent::SubmissionsEvicted { .. } => {}
            other => panic!("history-only trim was mislabeled: {other:?}"),
        },
        other => panic!("unexpected output: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn control_operations_are_idempotent_and_switch_identity() {
    let mut miner = miner();
    let (pk, _) = authority(1);
    let (other_pk, _) = authority(2);
    let add = command(
        "control-add",
        Operation::AddConnection {
            connection: connection("127.0.0.1:9".into(), pk, false),
        },
    );
    let first = miner.execute(add.clone()).await;
    assert_eq!(first.outcome, "running");
    let replay = miner.execute(add.clone()).await;
    assert_eq!(replay, first);
    assert_eq!(miner.snapshot().connections.len(), 1);
    let conflict = miner
        .execute(command("control-add", Operation::Snapshot {}))
        .await;
    assert_eq!(conflict.outcome, "rejected_precondition");
    assert_eq!(conflict.error.unwrap().code, "command_id_conflict");

    let paused = miner
        .execute(command(
            "control-pause",
            Operation::PauseWorkers {
                worker_ids: vec!["worker-0".into(), "worker-1".into()],
            },
        ))
        .await;
    assert_eq!(paused.outcome, "succeeded");
    assert!(miner.snapshot().workers.iter().all(|worker| worker.paused));
    let resumed = miner
        .execute(command(
            "control-resume",
            Operation::ResumeWorkers {
                worker_ids: vec!["worker-0".into(), "worker-1".into()],
            },
        ))
        .await;
    assert_eq!(resumed.outcome, "succeeded");
    assert!(miner.snapshot().workers.iter().all(|worker| !worker.paused));

    let budgeted = miner
        .execute(command(
            "control-budget",
            Operation::SetWorkerBudget {
                worker_id: "worker-0".into(),
                budget: HashBudget {
                    hashes_per_second: Some(25),
                    total_attempts: Some(333),
                },
            },
        ))
        .await;
    assert_eq!(budgeted.outcome, "succeeded");
    let worker_zero = miner
        .snapshot()
        .workers
        .into_iter()
        .find(|worker| worker.id == "worker-0")
        .unwrap();
    assert_eq!(worker_zero.requested.hashes_per_second, Some(25));
    assert_eq!(worker_zero.requested.total_attempts, Some(333));

    let switched = miner
        .execute(command(
            "control-switch",
            Operation::SwitchPool {
                connection_id: "conn-live".into(),
                endpoint: "127.0.0.1:10".into(),
                identity: "tsp1q-new-identity".into(),
                authority_public_key: hex::encode(other_pk.into_bytes()),
                min_extranonce_size: Some(16),
            },
        ))
        .await;
    assert_eq!(switched.outcome, "running", "{switched:?}");
    let snapshot = miner.snapshot();
    assert_eq!(snapshot.connections.len(), 1);
    assert_eq!(snapshot.connections[0].endpoint, "127.0.0.1:10");
    assert_eq!(snapshot.connections[0].identity, "tsp1q-new-identity");
    assert_eq!(snapshot.workers.len(), 2);

    let removed = miner
        .execute(command(
            "control-remove",
            Operation::RemoveConnection {
                connection_id: "conn-live".into(),
            },
        ))
        .await;
    assert_eq!(removed.outcome, "succeeded");
    assert!(miner.snapshot().connections.is_empty());
    assert!(miner.snapshot().workers.is_empty());
    // Retrying the original add returns its recorded historical outcome; it
    // does not recreate the removed resources.
    let historical = miner.execute(add).await;
    assert_eq!(historical, first);
    assert!(miner.snapshot().connections.is_empty());
    miner.stop_connections().await;
}
