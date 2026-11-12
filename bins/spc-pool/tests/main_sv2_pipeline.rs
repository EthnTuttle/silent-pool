//! Production `main.rs` SV2 outcome path with a small real protocol peer.

use std::io::Read;
use std::net::{SocketAddr, TcpListener};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use spc_share_ext::{Message as Type32Message, EXTENSION_TYPE};

const TYPE32_ERROR: u8 = 0x08;

use bitcoin::hashes::Hash as _;
use bitcoin::pow::{CompactTarget, Target};
use spc_crypto::Network;
use stratum_apps::network_helpers::connect_with_noise;
use stratum_apps::network_helpers::noise_stream::{NoiseTcpReadHalf, NoiseTcpWriteHalf};
use stratum_core::binary_sv2::{self, Str0255, B032, U256};
use stratum_core::common_messages_sv2::{
    Protocol, SetupConnection, SetupConnectionSuccess, MESSAGE_TYPE_SETUP_CONNECTION,
};
use stratum_core::framing_sv2::framing::MessageFrame;
use stratum_core::mining_sv2::{
    NewExtendedMiningJob, OpenExtendedMiningChannel, OpenExtendedMiningChannelSuccess,
    SetNewPrevHash, SetTarget, SubmitSharesExtendedOwned, SubmitSharesSuccess,
    MESSAGE_TYPE_MINING_SET_NEW_PREV_HASH, MESSAGE_TYPE_NEW_EXTENDED_MINING_JOB,
    MESSAGE_TYPE_OPEN_EXTENDED_MINING_CHANNEL, MESSAGE_TYPE_OPEN_MINING_CHANNEL_ERROR,
    MESSAGE_TYPE_SET_TARGET, MESSAGE_TYPE_SUBMIT_SHARES_EXTENDED,
};
use tokio::net::TcpStream;
use tokio::time::sleep;

const PUB_KEY: &str = "9auqWEzQDVyd2oe1JVGFLMLHZtCo2FFqZwtKA5gd9xbuEu7PH72";
const SECRET: &str = "mkDLTBBRxdBv998612qipDYoTK3YUrqLe8uWw7gu3iXbSrn2n";
const SUBMIT_SUCCESS: u8 = 0x1c;

struct MainProcess {
    child: Child,
    _log: Arc<Mutex<String>>,
}

impl Drop for MainProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn free_addr() -> SocketAddr {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
}

fn address() -> String {
    let secp = bitcoin::secp256k1::Secp256k1::new();
    let scan = bitcoin::secp256k1::PublicKey::from_secret_key(
        &secp,
        &bitcoin::secp256k1::SecretKey::from_slice(&[0x11; 32]).unwrap(),
    );
    let spend = bitcoin::secp256k1::PublicKey::from_secret_key(
        &secp,
        &bitcoin::secp256k1::SecretKey::from_slice(&[0x12; 32]).unwrap(),
    );
    spc_crypto::address::encode_sp_address(Network::Regtest, &scan, &spend).unwrap()
}

async fn send_message<T>(write: &mut NoiseTcpWriteHalf, message: T, ty: u8, channel: bool)
where
    T: binary_sv2::Serialize + binary_sv2::GetSize,
{
    write
        .write_frame(MessageFrame::from_message(message, ty, 0, channel).unwrap())
        .await
        .unwrap();
}

async fn next_frame(read: &mut NoiseTcpReadHalf) -> (u8, Vec<u8>) {
    let mut frame = read.read_frame().await.unwrap();
    (frame.header().msg_type(), frame.payload().to_vec())
}

#[derive(Debug)]
struct PeerJob {
    channel_id: u32,
    prefix: Vec<u8>,
    coinbase_prefix: Vec<u8>,
    coinbase_suffix: Vec<u8>,
    prev_hash: [u8; 32],
    version: u32,
    nbits: u32,
    ntime: u32,
    job_id: u32,
    share_target: Target,
}

async fn connect_peer(addr: SocketAddr) -> (NoiseTcpReadHalf, NoiseTcpWriteHalf, PeerJob) {
    let authority: stratum_apps::key_utils::Secp256k1PublicKey = PUB_KEY.parse().unwrap();
    let stream = connect_with_noise(TcpStream::connect(addr).await.unwrap(), Some(authority))
        .await
        .unwrap();
    let (mut read, mut write) = stream.into_split();
    let setup = SetupConnection {
        protocol: Protocol::MiningProtocol,
        min_version: 2,
        max_version: 2,
        flags: 0,
        endpoint_host: Str0255::try_from("127.0.0.1").unwrap(),
        endpoint_port: 0,
        vendor: Str0255::try_from("lp1-peer").unwrap(),
        hardware_version: Str0255::try_from("1").unwrap(),
        firmware: Str0255::try_from("1").unwrap(),
        device_id: Str0255::try_from("1").unwrap(),
    };
    send_message(&mut write, setup, MESSAGE_TYPE_SETUP_CONNECTION, false).await;
    let (ty, mut payload) = next_frame(&mut read).await;
    assert_eq!(ty, 0x01);
    let _: SetupConnectionSuccess = binary_sv2::from_bytes(&mut payload).unwrap();
    let identity = address();
    let open = OpenExtendedMiningChannel {
        request_id: 7,
        user_identity: Str0255::try_from(identity.as_str()).unwrap(),
        nominal_hash_rate: 1.0,
        max_target: U256::from(&[0xff; 32]),
        min_extranonce_size: 4,
    };
    send_message(
        &mut write,
        open,
        MESSAGE_TYPE_OPEN_EXTENDED_MINING_CHANNEL,
        false,
    )
    .await;
    let mut channel_id = None;
    let mut prefix = None;
    let mut share_target = None;
    let mut coinbase_prefix = None;
    let mut coinbase_suffix = None;
    let mut version = None;
    let mut job_id = None;
    let mut prev_hash = None;
    let mut nbits = None;
    let mut ntime = None;
    while job_id.is_none() || nbits.is_none() {
        let (ty, mut payload) = next_frame(&mut read).await;
        match ty {
            MESSAGE_TYPE_OPEN_MINING_CHANNEL_ERROR => panic!("channel open rejected"),
            0x14 => {
                let success: OpenExtendedMiningChannelSuccess =
                    binary_sv2::from_bytes(&mut payload).unwrap();
                channel_id = Some(success.channel_id);
                prefix = Some(success.extranonce_prefix.as_bytes().to_vec());
                share_target = Some(Target::from_le_bytes(
                    success.target.as_bytes().try_into().unwrap(),
                ));
            }
            MESSAGE_TYPE_SET_TARGET => {
                let target: SetTarget = binary_sv2::from_bytes(&mut payload).unwrap();
                share_target = Some(Target::from_le_bytes(
                    target.maximum_target.as_bytes().try_into().unwrap(),
                ));
            }
            MESSAGE_TYPE_NEW_EXTENDED_MINING_JOB => {
                let job: NewExtendedMiningJob = binary_sv2::from_bytes(&mut payload).unwrap();
                coinbase_prefix = Some(job.coinbase_tx_prefix.as_bytes().to_vec());
                coinbase_suffix = Some(job.coinbase_tx_suffix.as_bytes().to_vec());
                version = Some(job.version);
                job_id = Some(job.job_id);
            }
            MESSAGE_TYPE_MINING_SET_NEW_PREV_HASH => {
                let prev: SetNewPrevHash = binary_sv2::from_bytes(&mut payload).unwrap();
                prev_hash = Some(prev.prev_hash.as_bytes().try_into().unwrap());
                nbits = Some(prev.nbits);
                ntime = Some(prev.min_ntime);
            }
            _ => {}
        }
    }
    (
        read,
        write,
        PeerJob {
            channel_id: channel_id.unwrap(),
            prefix: prefix.unwrap(),
            coinbase_prefix: coinbase_prefix.unwrap(),
            coinbase_suffix: coinbase_suffix.unwrap(),
            prev_hash: prev_hash.unwrap(),
            version: version.unwrap(),
            nbits: nbits.unwrap(),
            ntime: ntime.unwrap(),
            job_id: job_id.unwrap(),
            share_target: share_target.unwrap(),
        },
    )
}

fn candidate(job: &PeerJob, sequence: u32, require_block: bool) -> SubmitSharesExtendedOwned {
    let extranonce = if require_block { [0xbb; 4] } else { [0xaa; 4] };
    let network = Target::from_compact(CompactTarget::from_consensus(job.nbits));
    for nonce in 0..4_096u32 {
        let mut coinbase = job.coinbase_prefix.clone();
        coinbase.extend_from_slice(&job.prefix);
        coinbase.extend_from_slice(&extranonce);
        coinbase.extend_from_slice(&job.coinbase_suffix);
        let root = bitcoin::hashes::sha256d::Hash::hash(&coinbase).to_byte_array();
        let mut header = Vec::with_capacity(80);
        header.extend_from_slice(&job.version.to_le_bytes());
        header.extend_from_slice(&job.prev_hash);
        header.extend_from_slice(&root);
        header.extend_from_slice(&job.ntime.to_le_bytes());
        header.extend_from_slice(&job.nbits.to_le_bytes());
        header.extend_from_slice(&nonce.to_le_bytes());
        let hash = bitcoin::hashes::sha256d::Hash::hash(&header).to_byte_array();
        let qualifies = job
            .share_target
            .is_met_by(bitcoin::BlockHash::from_byte_array(hash));
        let is_block = network.is_met_by(bitcoin::BlockHash::from_byte_array(hash));
        if qualifies && is_block == require_block {
            return SubmitSharesExtendedOwned {
                channel_id: job.channel_id,
                sequence_number: sequence,
                job_id: job.job_id,
                nonce,
                ntime: job.ntime,
                version: job.version,
                extranonce: B032::new(&extranonce).unwrap().into_owned(),
            };
        }
    }
    panic!(
        "no {} nonce found",
        if require_block { "block" } else { "share" }
    );
}

fn append_log<R: Read + Send + 'static>(mut reader: R, log: Arc<Mutex<String>>) {
    std::thread::spawn(move || {
        let mut bytes = [0u8; 1024];
        loop {
            match reader.read(&mut bytes) {
                Ok(0) | Err(_) => break,
                Ok(n) => log
                    .lock()
                    .unwrap()
                    .push_str(&String::from_utf8_lossy(&bytes[..n])),
            }
        }
    });
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn main_sv2_real_front_outcome_distinguishes_non_block_and_block() {
    let addr = free_addr();
    let config = format!(
        r#"
[pool]
network = "regtest"
roles = ["front"]
listen = "{addr}"
authority_public_key = "{PUB_KEY}"
authority_secret_key = "{SECRET}"
pool_payout_descriptor = "tr(tpubD6NzVbkrYhZ4WUjXBVxZPvGhWxTRE5veAscTgokX65xcfCoDF5nyNdtaS4xGoJ7xnZuDTxCAEAnGpxJbWo6wtTAJsESmeJWpZzybZ6dvCia/0/*)"
front_id = 7

[template]
source = "mock"

[sv1]
enabled = false

[accounting]
allow_unchecked_chain = true
work_unit_version = 2
work_reference_target = "{}"
share_work_multiplier = 2
window_n = 8

[extensions]
share_accounting = false
window_hot_blocks = 6

[jd]
listen = "127.0.0.1:0"
"#,
        "7f".to_string() + &"ff".repeat(31),
    );
    let path = std::env::temp_dir().join(format!(
        "LP1-main-{}-{}.toml",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::write(&path, config).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_spc-pool"))
        .args(["--config", path.to_str().unwrap()])
        // Nix's check derivation exports RUST_LOG as the empty string, whose
        // filter disables the evidence this test observes. Set an explicit
        // filter so the spawned main process emits the same info records in
        // both dev-shell and sandboxed runs.
        .env("RUST_LOG", "info,spc_pool=debug,spc_evidence=info")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let log = Arc::new(Mutex::new(String::new()));
    append_log(child.stdout.take().unwrap(), log.clone());
    append_log(child.stderr.take().unwrap(), log.clone());
    let _process = MainProcess {
        child,
        _log: log.clone(),
    };
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            match TcpStream::connect(addr).await {
                Ok(stream) => {
                    drop(stream);
                    break;
                }
                Err(_) => sleep(Duration::from_millis(50)).await,
            }
        }
    })
    .await
    .unwrap();

    let (mut read, mut write, job) = connect_peer(addr).await;
    assert_eq!(job.share_target.to_be_bytes()[0], 0x3f);
    assert!(job.share_target.to_be_bytes()[1..]
        .iter()
        .all(|b| *b == 0xff));

    let non_block = candidate(&job, 0, false);
    send_message(
        &mut write,
        non_block,
        MESSAGE_TYPE_SUBMIT_SHARES_EXTENDED,
        true,
    )
    .await;
    let (ty, mut payload) = next_frame(&mut read).await;
    assert_eq!(ty, SUBMIT_SUCCESS);
    let ack: SubmitSharesSuccess = binary_sv2::from_bytes(&mut payload).unwrap();
    assert_eq!(ack.last_sequence_number, 0);
    assert_eq!(
        ack.new_shares_sum, 2,
        "v2 m_issue=2, not legacy/recomputed d"
    );

    for message in [
        Type32Message::GetWindow {
            request_id: 40,
            block_hash: [0; 32],
        },
        Type32Message::GetShares {
            request_id: 41,
            slice_id: 0,
            indices: vec![0],
        },
        Type32Message::GetTransactionsInJob {
            request_id: 42,
            job_id: 0,
        },
    ] {
        let payload = message.encode().unwrap();
        let mut frame = Vec::with_capacity(6 + payload.len());
        frame.extend_from_slice(&EXTENSION_TYPE.to_le_bytes());
        frame.push(message.message_type());
        frame.extend_from_slice(&(payload.len() as u32).to_le_bytes()[..3]);
        frame.extend_from_slice(&payload);
        let request_id = match message {
            Type32Message::GetWindow { request_id, .. }
            | Type32Message::GetShares { request_id, .. }
            | Type32Message::GetTransactionsInJob { request_id, .. } => request_id,
            _ => unreachable!(),
        };
        write
            .write_frame(stratum_core::codec_sv2::SerializedFrame::from_bytes(frame).unwrap())
            .await
            .unwrap();
        let (ty, payload) = next_frame(&mut read).await;
        assert_eq!(
            ty, TYPE32_ERROR,
            "type-32 must be denied at the SV2 ingress, not as {message:?}"
        );
        let error = Type32Message::decode(TYPE32_ERROR, &payload).unwrap();
        assert_eq!(
            error,
            Type32Message::Error {
                request_id,
                code: "unsupported-extension".into(),
            }
        );
    }

    let block = candidate(&job, 1, true);
    send_message(&mut write, block, MESSAGE_TYPE_SUBMIT_SHARES_EXTENDED, true).await;
    // A found block makes the front refresh and publish a new job while this
    // share's outcome is still pending. New jobs are allowed to arrive before
    // the share response; the response must still be exactly one success for
    // sequence 1 with its original credited work.
    let (mut ty, mut payload) = next_frame(&mut read).await;
    let ack: SubmitSharesSuccess = loop {
        match ty {
            SUBMIT_SUCCESS => break binary_sv2::from_bytes(&mut payload).unwrap(),
            MESSAGE_TYPE_NEW_EXTENDED_MINING_JOB | MESSAGE_TYPE_MINING_SET_NEW_PREV_HASH => {
                (ty, payload) = next_frame(&mut read).await;
            }
            other => panic!("unexpected frame {other} before submit success"),
        }
    };
    assert_eq!(ack.last_sequence_number, 1);
    assert_eq!(ack.new_shares_sum, 2);

    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let text = log.lock().unwrap().clone();
        if text.matches("accounted").count() >= 2 && text.contains("job_issued") {
            assert!(text.contains("work_unit_version"));
            assert!(text.contains("m_issue"));
            assert!(text.contains("share_hash"));
            assert!(text.contains("extranonce"));
            break;
        }
        assert!(
            Instant::now() < deadline,
            "missing accounted/job_issued evidence: {text}"
        );
        sleep(Duration::from_millis(25)).await;
    }
    let _ = std::fs::remove_file(path);
}
