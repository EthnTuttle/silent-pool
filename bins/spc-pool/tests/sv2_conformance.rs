//! Native SV2 conformance against the independently implemented
//! `channels_sv2` client state machine (not the in-repo agent helpers).

use std::sync::Arc;
use std::time::Duration;

use bitcoin::pow::{CompactTarget, Target};
use channels_sv2::client::extended::ExtendedChannel;
use channels_sv2::extranonce_manager::ExtranoncePrefix;
use spc_crypto::Network;
use spc_pool::front::{Front, FrontConfig};
use spc_pool::sv1::Job;
use spc_pool::sv2_server::Sv2Server;
use spc_pool::template::{MockTemplateSource, Template};
use stratum_apps::network_helpers::connect_with_noise;
use stratum_apps::network_helpers::noise_stream::{NoiseTcpReadHalf, NoiseTcpWriteHalf};
use stratum_core::binary_sv2::{self, Str0255, Sv2OptionOwned, B032, U256};
use stratum_core::common_messages_sv2::{Protocol, SetupConnection};
use stratum_core::framing_sv2::framing::MessageFrame;
use stratum_core::mining_sv2::{
    NewExtendedMiningJob, NewExtendedMiningJobOwned, OpenExtendedMiningChannel,
    OpenExtendedMiningChannelSuccess, OpenExtendedMiningChannelSuccessOwned,
    OpenMiningChannelError, SetNewPrevHash, SetNewPrevHashOwned, SetTarget, SubmitSharesError,
    SubmitSharesExtended, SubmitSharesExtendedOwned, MESSAGE_TYPE_MINING_SET_NEW_PREV_HASH,
    MESSAGE_TYPE_NEW_EXTENDED_MINING_JOB, MESSAGE_TYPE_OPEN_EXTENDED_MINING_CHANNEL,
    MESSAGE_TYPE_OPEN_EXTENDED_MINING_CHANNEL_SUCCESS, MESSAGE_TYPE_OPEN_MINING_CHANNEL_ERROR,
    MESSAGE_TYPE_SET_TARGET, MESSAGE_TYPE_SUBMIT_SHARES_ERROR, MESSAGE_TYPE_SUBMIT_SHARES_EXTENDED,
};
use tokio::net::TcpStream;
use tokio::sync::mpsc;

const SETUP_SUCCESS: u8 = 0x01;

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

fn template() -> Template {
    Template {
        height: 800_000,
        prev_hash: [0x11; 32],
        version: 0x2000_0000,
        bits: 0x207f_ffff,
        curtime: 1_700_000_000,
        subsidy: 312_500_000,
        fees: 0,
        coinbase_value: 312_500_000,
        transactions: vec![],
        merkle_branches: vec![],
        coinbase_output_max_additional_size: 50_000,
    }
}

fn front_config() -> FrontConfig {
    let secp = bitcoin::secp256k1::Secp256k1::new();
    let xprv = bitcoin::bip32::Xpriv::new_master(bitcoin::Network::Testnet, &[0x5c; 32]).unwrap();
    let xpub = bitcoin::bip32::Xpub::from_priv(&secp, &xprv);
    FrontConfig {
        network: Network::Regtest,
        fee_bps: 100,
        window_n: 8,
        slice_fee_delta_sats: 0,
        max_slice_shares: 65_000,
        dust_limit_sats: 330,
        coinbase_threshold_sats: 330,
        max_payout_slots: 256,
        budget_wu: 100_000,
        overhead_wu: 0,
        pool_payout_descriptor: spc_pool::coinbase::PoolPayoutDescriptor::parse(
            &format!("tr({xpub}/0/*)"),
            Network::Regtest,
        )
        .unwrap(),
        pool_payout_script: Some(
            spc_pool::coinbase::p2tr_script(&[0x99; 32])
                .to_bytes()
                .to_vec(),
        ),
        extranonce_len: 8,
        front_id: 0,
        keep_slices: 16,
    }
}

async fn job_for(template: Template) -> Job {
    let mut front = Front::new(
        front_config(),
        Arc::new(MockTemplateSource::new(template)),
        Arc::new(std::sync::Mutex::new(
            spc_pool::type32::Type32Store::default(),
        )),
    );
    front.refresh().await.unwrap()
}

async fn initial_job() -> Job {
    job_for(template()).await
}

fn raw_legacy_tx(seed: u8) -> Vec<u8> {
    use bitcoin::absolute::LockTime;
    use bitcoin::hashes::Hash as _;
    use bitcoin::transaction::Version;
    use bitcoin::{Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness};
    let tx = Transaction {
        version: Version::TWO,
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint {
                txid: bitcoin::Txid::from_byte_array([seed; 32]),
                vout: u32::from(seed),
            },
            script_sig: ScriptBuf::new(),
            sequence: Sequence::MAX,
            witness: Witness::default(),
        }],
        output: vec![TxOut {
            value: Amount::from_sat(u64::from(seed) + 1),
            script_pubkey: ScriptBuf::new(),
        }],
    };
    bitcoin::consensus::serialize(&tx)
}

fn server(tx: mpsc::Sender<spc_pool::sv1::Share>) -> Sv2Server {
    Sv2Server::new(
        Network::Regtest,
        tx,
        Arc::new(std::sync::Mutex::new(
            spc_pool::type32::Type32Store::default(),
        )),
        8,
    )
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

async fn connect(addr: std::net::SocketAddr) -> (NoiseTcpReadHalf, NoiseTcpWriteHalf) {
    let stream = TcpStream::connect(addr).await.unwrap();
    let stream = connect_with_noise(stream, None).await.unwrap();
    let (mut read, mut write) = stream.into_split();
    let setup = SetupConnection {
        protocol: Protocol::MiningProtocol,
        min_version: 2,
        max_version: 2,
        flags: 0,
        endpoint_host: Str0255::try_from("127.0.0.1").unwrap(),
        endpoint_port: 0,
        vendor: Str0255::try_from("conformance").unwrap(),
        hardware_version: Str0255::try_from("1").unwrap(),
        firmware: Str0255::try_from("1").unwrap(),
        device_id: Str0255::try_from("1").unwrap(),
    };
    send_message(&mut write, setup, 0x00, false).await;
    let response = read.read_frame().await.unwrap();
    assert_eq!(response.header().msg_type(), SETUP_SUCCESS);
    (read, write)
}

async fn send_open(
    write: &mut NoiseTcpWriteHalf,
    request_id: u32,
    min_extranonce: u16,
    max: [u8; 32],
) {
    let identity = address();
    let open = OpenExtendedMiningChannel {
        request_id,
        user_identity: Str0255::try_from(identity.as_str()).unwrap(),
        nominal_hash_rate: 1.0,
        max_target: U256::from(&max),
        min_extranonce_size: min_extranonce,
    };
    send_message(
        write,
        open,
        MESSAGE_TYPE_OPEN_EXTENDED_MINING_CHANNEL,
        false,
    )
    .await;
}

async fn open_success(read: &mut NoiseTcpReadHalf) -> OpenExtendedMiningChannelSuccessOwned {
    loop {
        let mut frame = read.read_frame().await.unwrap();
        if frame.header().msg_type() == MESSAGE_TYPE_OPEN_EXTENDED_MINING_CHANNEL_SUCCESS {
            let mut payload = frame.payload().to_vec();
            let success: OpenExtendedMiningChannelSuccess =
                binary_sv2::from_bytes(&mut payload).unwrap();
            return OpenExtendedMiningChannelSuccessOwned {
                request_id: success.request_id,
                channel_id: success.channel_id,
                target: success.target.into_owned(),
                extranonce_size: success.extranonce_size,
                extranonce_prefix: success.extranonce_prefix.into_owned(),
                group_channel_id: success.group_channel_id,
            };
        }
        assert_ne!(
            frame.header().msg_type(),
            MESSAGE_TYPE_OPEN_MINING_CHANNEL_ERROR
        );
    }
}

fn owned_job(message: NewExtendedMiningJob) -> NewExtendedMiningJobOwned {
    NewExtendedMiningJobOwned {
        channel_id: message.channel_id,
        job_id: message.job_id,
        min_ntime: Sv2OptionOwned::new(message.min_ntime.into_inner()),
        version: message.version,
        version_rolling_allowed: message.version_rolling_allowed,
        merkle_path: message.merkle_path.into_owned(),
        coinbase_tx_prefix: message.coinbase_tx_prefix.into_owned(),
        coinbase_tx_suffix: message.coinbase_tx_suffix.into_owned(),
    }
}

fn owned_prev(message: SetNewPrevHash) -> SetNewPrevHashOwned {
    SetNewPrevHashOwned {
        channel_id: message.channel_id,
        job_id: message.job_id,
        prev_hash: message.prev_hash.into_owned(),
        min_ntime: message.min_ntime,
        nbits: message.nbits,
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Wire {
    Target,
    Future,
    Active,
    PrevHash,
    Other,
}

fn apply_upstream(
    frame: &mut stratum_core::codec_sv2::SerializedFrame,
    channel: &mut ExtendedChannel,
) -> Wire {
    let ty = frame.header().msg_type();
    let mut payload = frame.payload().to_vec();
    match ty {
        MESSAGE_TYPE_SET_TARGET => {
            let message: SetTarget = binary_sv2::from_bytes(&mut payload).unwrap();
            let target =
                Target::from_le_bytes(message.maximum_target.as_bytes().try_into().unwrap());
            channel.set_target(target).unwrap();
            Wire::Target
        }
        MESSAGE_TYPE_NEW_EXTENDED_MINING_JOB => {
            let message: NewExtendedMiningJob = binary_sv2::from_bytes(&mut payload).unwrap();
            let future = message.is_future();
            channel
                .on_new_extended_mining_job(owned_job(message))
                .unwrap();
            if future {
                Wire::Future
            } else {
                Wire::Active
            }
        }
        MESSAGE_TYPE_MINING_SET_NEW_PREV_HASH => {
            let message: SetNewPrevHash = binary_sv2::from_bytes(&mut payload).unwrap();
            channel.on_set_new_prev_hash(owned_prev(message)).unwrap();
            Wire::PrevHash
        }
        _ => Wire::Other,
    }
}

#[tokio::test]
async fn upstream_client_activates_future_jobs_and_captures_job_targets() {
    let (tx, _rx) = mpsc::channel(8);
    let server = server(tx);
    let (addr, _handle) = server.bind("127.0.0.1:0").await.unwrap();
    let (mut read, mut write) = connect(addr).await;
    send_open(&mut write, 1, 4, [0xff; 32]).await;
    let success = open_success(&mut read).await;

    // Early opens have no template target. The first job must update it before
    // the future job is queued.
    assert_eq!(success.target.as_bytes(), &[0xff; 32]);
    let open_target = Target::from_le_bytes([0xff; 32]);
    let mut upstream = ExtendedChannel::new(
        success.channel_id,
        address(),
        ExtranoncePrefix::from_wire(success.extranonce_prefix.as_bytes().to_vec()).unwrap(),
        open_target,
        1.0,
        true,
        success.extranonce_size,
        None,
    )
    .unwrap();

    let initial = initial_job().await;
    server.publish(initial.clone()).await;
    let mut frame = read.read_frame().await.unwrap();
    assert_eq!(apply_upstream(&mut frame, &mut upstream), Wire::Target);
    let expected_target = Target::from_compact(CompactTarget::from_consensus(initial.nbits));
    assert_eq!(upstream.get_target(), &expected_target);
    let mut frame = read.read_frame().await.unwrap();
    assert_eq!(apply_upstream(&mut frame, &mut upstream), Wire::Future);
    assert_eq!(upstream.get_future_jobs_count(), 1);
    assert!(upstream.get_active_job().is_none());
    let mut frame = read.read_frame().await.unwrap();
    assert_eq!(apply_upstream(&mut frame, &mut upstream), Wire::PrevHash);
    assert_eq!(upstream.get_future_jobs_count(), 0);
    assert_eq!(upstream.get_active_job().unwrap().target, expected_target);

    // Same-tip update: no SetNewPrevHash and no target update when nbits is
    // unchanged. The new nonfuture job becomes active without invalidating the
    // prevhash through an out-of-order activation.
    let mut same_tip = initial.clone();
    same_tip.job_id = "00000006".into();
    same_tip.ntime += 1;
    server.publish(same_tip.clone()).await;
    let mut frame = read.read_frame().await.unwrap();
    assert_eq!(apply_upstream(&mut frame, &mut upstream), Wire::Active);
    assert_eq!(upstream.get_past_jobs_count(), 1);

    // A same-prevhash nbits update also needs activation: future job then
    // SetNewPrevHash is the only message that can update upstream header bits.
    // The already active old job retains its old target until replacement.
    let mut retargeted = same_tip.clone();
    retargeted.job_id = "00000007".into();
    retargeted.nbits = 0x207e_ffff;
    retargeted.work =
        spc_pool::sv1::JobWork::legacy(1, retargeted.nbits, retargeted.work.generation + 1);
    let retargeted_bits = retargeted.nbits;
    let old_target = upstream.get_active_job().unwrap().target;
    server.publish(retargeted).await;
    let mut frame = read.read_frame().await.unwrap();
    assert_eq!(apply_upstream(&mut frame, &mut upstream), Wire::Target);
    assert_eq!(upstream.get_active_job().unwrap().target, old_target);
    let mut frame = read.read_frame().await.unwrap();
    assert_eq!(apply_upstream(&mut frame, &mut upstream), Wire::Future);
    assert_eq!(upstream.get_active_job().unwrap().target, old_target);
    let mut frame = read.read_frame().await.unwrap();
    assert_eq!(apply_upstream(&mut frame, &mut upstream), Wire::PrevHash);
    let new_target = Target::from_compact(CompactTarget::from_consensus(retargeted_bits));
    assert_eq!(upstream.get_active_job().unwrap().target, new_target);
    assert_eq!(upstream.get_chain_tip().unwrap().nbits(), retargeted_bits);
    assert_eq!(upstream.get_stale_jobs_count(), 2);
    assert_ne!(old_target, new_target);

    // A prevhash activation is also future job then exactly one
    // SetNewPrevHash. Ordinary same-(prevhash, bits) updates emitted none.
    let mut next_tip = initial;
    next_tip.job_id = "00000008".into();
    next_tip.prev_hash = [0x22; 32];
    next_tip.height += 1;
    next_tip.nbits = 0x207e_ffff;
    next_tip.work = spc_pool::sv1::JobWork::legacy(1, next_tip.nbits, next_tip.work.generation + 1);
    next_tip.ntime += 2;
    server.publish(next_tip).await;
    let mut frame = read.read_frame().await.unwrap();
    assert_eq!(apply_upstream(&mut frame, &mut upstream), Wire::Future);
    let mut frame = read.read_frame().await.unwrap();
    assert_eq!(apply_upstream(&mut frame, &mut upstream), Wire::PrevHash);
    assert_eq!(upstream.get_active_job().unwrap().target, new_target);
    assert_eq!(upstream.get_stale_jobs_count(), 1);
}

#[tokio::test]
async fn upstream_extended_channel_uses_wire_merkle_path_for_a_real_share() {
    use bitcoin::hashes::{sha256d, Hash};

    let transactions = vec![raw_legacy_tx(1), raw_legacy_tx(2)];
    let mut with_txs = template();
    with_txs.transactions = transactions.clone();
    let job = job_for(with_txs).await;
    assert_eq!(job.merkle_branches.len(), 2);

    let (tx, _rx) = mpsc::channel(8);
    let server = server(tx);
    server.publish(job.clone()).await;
    let (addr, _handle) = server.bind("127.0.0.1:0").await.unwrap();
    let (mut read, mut write) = connect(addr).await;
    send_open(&mut write, 1, 4, [0xff; 32]).await;
    let success = open_success(&mut read).await;
    let open_target = Target::from_le_bytes(success.target.as_bytes().try_into().unwrap());
    let mut upstream = ExtendedChannel::new(
        success.channel_id,
        address(),
        ExtranoncePrefix::from_wire(success.extranonce_prefix.as_bytes().to_vec()).unwrap(),
        open_target,
        1.0,
        true,
        success.extranonce_size,
        None,
    )
    .unwrap();
    let mut frame = read.read_frame().await.unwrap();
    assert_eq!(apply_upstream(&mut frame, &mut upstream), Wire::Future);
    let mut frame = read.read_frame().await.unwrap();
    assert_eq!(apply_upstream(&mut frame, &mut upstream), Wire::PrevHash);

    let active = upstream.get_active_job().unwrap();
    assert_eq!(active.job_message.merkle_path.len(), 2);
    let miner_extranonce = [0xaa; 4];
    let full_extranonce = [
        active.extranonce_prefix.as_slice(),
        miner_extranonce.as_slice(),
    ]
    .concat();
    let wire_root = channels_sv2::merkle_root::merkle_root_from_path(
        active.job_message.coinbase_tx_prefix.as_bytes(),
        active.job_message.coinbase_tx_suffix.as_bytes(),
        &full_extranonce,
        active.job_message.merkle_path.as_slice(),
    )
    .unwrap();

    // Independent three-leaf root: H(H(coinbase || tx1) || H(tx2 || tx2)).
    // The two wire siblings must be tx1, then H(tx2 || tx2), in that order.
    let pair = |a: &[u8; 32], b: &[u8; 32]| {
        let mut bytes = [0u8; 64];
        bytes[..32].copy_from_slice(a);
        bytes[32..].copy_from_slice(b);
        sha256d::Hash::hash(&bytes).to_byte_array()
    };
    let mut coinbase = active.job_message.coinbase_tx_prefix.as_bytes().to_vec();
    coinbase.extend_from_slice(&full_extranonce);
    coinbase.extend_from_slice(active.job_message.coinbase_tx_suffix.as_bytes());
    let coinbase_id = sha256d::Hash::hash(&coinbase).to_byte_array();
    let tx1 = sha256d::Hash::hash(&transactions[0]).to_byte_array();
    let tx2 = sha256d::Hash::hash(&transactions[1]).to_byte_array();
    assert_eq!(active.job_message.merkle_path[0].as_bytes(), tx1);
    assert_eq!(
        active.job_message.merkle_path[1].as_bytes(),
        pair(&tx2, &tx2)
    );
    assert_eq!(
        wire_root,
        pair(&pair(&coinbase_id, &tx1), &pair(&tx2, &tx2))
    );

    let job_id = u32::from_str_radix(&job.job_id, 16).unwrap();
    let mut accepted = false;
    for nonce in 0..64u32 {
        let share = SubmitSharesExtendedOwned {
            channel_id: success.channel_id,
            sequence_number: nonce,
            job_id,
            nonce,
            ntime: job.ntime,
            version: job.version,
            extranonce: B032::new(&miner_extranonce).unwrap().into_owned(),
        };
        if upstream.validate_share(share).is_ok() {
            accepted = true;
            break;
        }
    }
    assert!(accepted, "upstream must accept a real multi-level share");
}

#[tokio::test]
async fn open_rejects_unsupported_negotiation_and_a_second_channel() {
    let (tx, _rx) = mpsc::channel(8);
    let server = server(tx);
    let (addr, _handle) = server.bind("127.0.0.1:0").await.unwrap();

    for (request_id, min_extranonce, maximum, expected) in [
        (1, 8, [0xff; 32], "unsupported-min-extranonce-size"),
        (2, 4, Target::MAX.to_le_bytes(), "max-target-out-of-range"),
    ] {
        let (mut read, mut write) = connect(addr).await;
        send_open(&mut write, request_id, min_extranonce, maximum).await;
        let mut frame = read.read_frame().await.unwrap();
        assert_eq!(
            frame.header().msg_type(),
            MESSAGE_TYPE_OPEN_MINING_CHANNEL_ERROR
        );
        let mut payload = frame.payload().to_vec();
        let error: OpenMiningChannelError = binary_sv2::from_bytes(&mut payload).unwrap();
        assert_eq!(error.error_code.as_bytes(), expected.as_bytes());
    }

    let (mut read, mut write) = connect(addr).await;
    send_open(&mut write, 3, 4, [0xff; 32]).await;
    let first = open_success(&mut read).await;
    send_open(&mut write, 4, 4, [0xff; 32]).await;
    let mut frame = read.read_frame().await.unwrap();
    assert_eq!(
        frame.header().msg_type(),
        MESSAGE_TYPE_OPEN_MINING_CHANNEL_ERROR
    );
    let mut payload = frame.payload().to_vec();
    let error: OpenMiningChannelError = binary_sv2::from_bytes(&mut payload).unwrap();
    assert_eq!(error.error_code.as_bytes(), b"channel-capacity-exhausted");

    let (mut other_read, mut other_write) = connect(addr).await;
    send_open(&mut other_write, 5, 4, [0xff; 32]).await;
    let second = open_success(&mut other_read).await;
    assert_ne!(first.channel_id, second.channel_id);
    assert_ne!(
        first.extranonce_prefix.as_bytes(),
        second.extranonce_prefix.as_bytes()
    );
}

#[tokio::test]
async fn bad_extranonce_length_is_rejected_before_ingress() {
    let (tx, mut rx) = mpsc::channel(8);
    let server = server(tx);
    let job = initial_job().await;
    server.publish(job.clone()).await;
    let (addr, _handle) = server.bind("127.0.0.1:0").await.unwrap();
    let (mut read, mut write) = connect(addr).await;
    send_open(&mut write, 1, 4, [0xff; 32]).await;
    let success = open_success(&mut read).await;
    // Future job followed by prevhash activation.
    let _ = read.read_frame().await.unwrap();
    let _ = read.read_frame().await.unwrap();

    let submit = SubmitSharesExtended {
        channel_id: success.channel_id,
        sequence_number: 0,
        job_id: u32::from_str_radix(&job.job_id, 16).unwrap(),
        nonce: 0,
        ntime: 1_700_000_000,
        version: 0x2000_0000,
        extranonce: B032::new(&[0xaa; 3]).unwrap(),
    };
    send_message(
        &mut write,
        submit,
        MESSAGE_TYPE_SUBMIT_SHARES_EXTENDED,
        true,
    )
    .await;
    let mut frame = read.read_frame().await.unwrap();
    assert_eq!(frame.header().msg_type(), MESSAGE_TYPE_SUBMIT_SHARES_ERROR);
    let mut payload = frame.payload().to_vec();
    let error: SubmitSharesError = binary_sv2::from_bytes(&mut payload).unwrap();
    assert_eq!(error.error_code.as_bytes(), b"bad-extranonce-size");
    assert!(tokio::time::timeout(Duration::from_millis(100), rx.recv())
        .await
        .is_err());
}
