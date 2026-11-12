//! A pending front outcome must not delay delivery of a newer SV2 job.
//!
//! This is the wire-level counterpart to the pool's nested outcome/job select:
//! hold one accepted share's outcome unresolved, publish a new-prevhash job,
//! and require the job plus activation to arrive before the old outcome is
//! resolved. The old share then keeps its original job id, sequence and exact
//! credited work.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bitcoin::hashes::Hash as _;
use bitcoin::pow::Target;
use spc_crypto::Network;
use spc_pool::sv1::{Job, JobWork};
use spc_pool::sv2_server::{ShareOutcome, Sv2Server};
use spc_pool::type32::Type32Store;
use stratum_apps::network_helpers::connect_with_noise;
use stratum_apps::network_helpers::noise_stream::{NoiseTcpReadHalf, NoiseTcpWriteHalf};
use stratum_core::binary_sv2::{self, Str0255, B032, U256};
use stratum_core::common_messages_sv2::{
    Protocol, SetupConnection, SetupConnectionSuccess, MESSAGE_TYPE_SETUP_CONNECTION,
};
use stratum_core::framing_sv2::framing::MessageFrame;
use stratum_core::mining_sv2::{
    NewExtendedMiningJob, OpenExtendedMiningChannel, OpenExtendedMiningChannelSuccess,
    SetNewPrevHash, SubmitSharesExtendedOwned, SubmitSharesSuccess,
    MESSAGE_TYPE_MINING_SET_NEW_PREV_HASH, MESSAGE_TYPE_NEW_EXTENDED_MINING_JOB,
    MESSAGE_TYPE_OPEN_EXTENDED_MINING_CHANNEL, MESSAGE_TYPE_OPEN_MINING_CHANNEL_ERROR,
    MESSAGE_TYPE_SUBMIT_SHARES_EXTENDED,
};
use tokio::net::TcpStream;
use tokio::sync::mpsc;

const PUB_KEY: &str = "9auqWEzQDVyd2oe1JVGFLMLHZtCo2FFqZwtKA5gd9xbuEu7PH72";
const SECRET: &str = "mkDLTBBRxdBv998612qipDYoTK3YUrqLe8uWw7gu3iXbSrn2n";
const SUBMIT_SUCCESS: u8 = 0x1c;

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

fn test_job(id: &str, prev: u8, ntime: u32) -> Job {
    Job {
        job_id: id.to_string(),
        prev_hash: [prev; 32],
        height: 1,
        version: 0x2000_0000,
        nbits: 0x207f_ffff,
        ntime,
        coinb1: vec![0x01, 0x02],
        coinb2: vec![0x03, 0x04],
        merkle_branches: Vec::new(),
        clean_jobs: true,
        work: JobWork::legacy(1, 0x207f_ffff, 1),
    }
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

async fn open_peer(addr: SocketAddr) -> (NoiseTcpReadHalf, NoiseTcpWriteHalf, PeerJob) {
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
        vendor: Str0255::try_from("runtime-preemption").unwrap(),
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

fn candidate(job: &PeerJob, sequence: u32) -> SubmitSharesExtendedOwned {
    let extranonce = [0xaa; 4];
    for nonce in 0..4096u32 {
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
        if job
            .share_target
            .is_met_by(bitcoin::BlockHash::from_byte_array(hash))
        {
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
    panic!("no qualifying nonce found");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn new_job_is_delivered_while_an_old_share_outcome_is_pending() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let (share_tx, mut share_rx) = mpsc::channel(8);
    let type32 = Arc::new(Mutex::new(Type32Store::default()));
    let server = Sv2Server::new(Network::Regtest, share_tx, type32, 8)
        .with_authority_secret_key(SECRET.parse().unwrap())
        .with_share_difficulty(1)
        .with_type32_enabled(false)
        .with_front_id(7);
    let (bound, _handle) = server.bind(&addr.to_string()).await.unwrap();
    let first = test_job("00000001", 0x11, 1_700_000_000);
    server.publish(first.clone()).await;

    let (mut read, mut write, peer_job) = open_peer(bound).await;
    assert_eq!(peer_job.job_id, 1);

    let share = candidate(&peer_job, 0);
    send_message(&mut write, share, MESSAGE_TYPE_SUBMIT_SHARES_EXTENDED, true).await;
    let received = tokio::time::timeout(Duration::from_secs(2), share_rx.recv())
        .await
        .expect("front must receive the share")
        .expect("share channel");
    assert_eq!(received.job_id, first.job_id);
    assert_eq!(received.share_index, 0);

    // The outcome is deliberately unresolved here. A new-prevhash job must
    // still reach this connection before that outcome.
    let second = test_job("00000002", 0x22, 1_700_000_100);
    server.publish(second.clone()).await;
    let (ty, mut payload) = tokio::time::timeout(Duration::from_secs(2), next_frame(&mut read))
        .await
        .expect("new job must preempt the pending outcome");
    assert_eq!(ty, MESSAGE_TYPE_NEW_EXTENDED_MINING_JOB);
    let job_msg: NewExtendedMiningJob = binary_sv2::from_bytes(&mut payload).unwrap();
    assert_eq!(job_msg.job_id, 2);
    let (ty, mut payload) = tokio::time::timeout(Duration::from_secs(2), next_frame(&mut read))
        .await
        .expect("new prevhash must follow the future job");
    assert_eq!(ty, MESSAGE_TYPE_MINING_SET_NEW_PREV_HASH);
    let prev: SetNewPrevHash = binary_sv2::from_bytes(&mut payload).unwrap();
    assert_eq!(prev.job_id, 2);
    assert_eq!(prev.prev_hash.as_bytes(), &[0x22; 32]);
    assert_eq!(prev.nbits, second.nbits);

    // Resolve the old share *after* the new job was delivered. It must retain
    // its own job/sequence and exact credited work; no second success appears.
    server.resolve_share(
        peer_job.channel_id,
        0,
        ShareOutcome::Accepted { credited_work: 2 },
    );
    let (ty, mut payload) = tokio::time::timeout(Duration::from_secs(2), next_frame(&mut read))
        .await
        .expect("old outcome must still resolve");
    assert_eq!(ty, SUBMIT_SUCCESS);
    let ack: SubmitSharesSuccess = binary_sv2::from_bytes(&mut payload).unwrap();
    assert_eq!(ack.last_sequence_number, 0);
    assert_eq!(ack.new_shares_sum, 2);
    assert_eq!(ack.new_submits_accepted_count, 1);

    let extra = tokio::time::timeout(Duration::from_millis(300), next_frame(&mut read)).await;
    assert!(extra.is_err(), "a second success must not be fabricated");
}
