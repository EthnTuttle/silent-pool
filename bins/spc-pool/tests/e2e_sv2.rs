//! End-to-end SV2 path: Noise handshake, SetupConnection, extended channel
//! open, job, share and block assembly.

use std::sync::Arc;
use std::time::Duration;

use bitcoin::hashes::Hash;
use spc_crypto::Network;
use spc_pool::front::{Front, FrontConfig};
use spc_pool::sv2_server::Sv2Server;
use spc_pool::template::{MockTemplateSource, Template, TemplateSource};
use stratum_apps::network_helpers::connect_with_noise;
use stratum_apps::network_helpers::noise_stream::NoiseTcpWriteHalf;
use stratum_core::binary_sv2::{self, Str0255, B032, U256};
use stratum_core::common_messages_sv2::{
    Protocol, SetupConnection, SetupConnectionSuccess, MESSAGE_TYPE_SETUP_CONNECTION,
};
use stratum_core::framing_sv2::framing::MessageFrame;
use stratum_core::mining_sv2::{
    OpenExtendedMiningChannel, OpenMiningChannelError, SubmitSharesExtended,
    MESSAGE_TYPE_NEW_EXTENDED_MINING_JOB, MESSAGE_TYPE_OPEN_EXTENDED_MINING_CHANNEL,
    MESSAGE_TYPE_OPEN_MINING_CHANNEL_ERROR, MESSAGE_TYPE_SUBMIT_SHARES_EXTENDED,
};
use tokio::net::TcpStream;
use tokio::sync::mpsc;

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

fn config() -> FrontConfig {
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
        pool_payout_script: spc_pool::coinbase::p2tr_script(&[0x99; 32])
            .to_bytes()
            .to_vec(),
        extranonce_len: 8,
        front_id: 0,
        keep_slices: 16,
    }
}

async fn write_msg<T>(w: &mut NoiseTcpWriteHalf, msg: T, msg_type: u8, channel_msg: bool)
where
    T: binary_sv2::Serialize + binary_sv2::GetSize,
{
    let frame = MessageFrame::from_message(msg, msg_type, 0, channel_msg).unwrap();
    w.write_frame(frame).await.unwrap();
}

#[tokio::test]
#[allow(unused_assignments)]
async fn sv2_miner_finds_a_block() {
    let source = Arc::new(MockTemplateSource::new(template()));
    let mut front = Front::new(
        config(),
        source.clone(),
        std::sync::Arc::new(std::sync::Mutex::new(
            spc_pool::type32::Type32Store::default(),
        )),
    );
    let job = front.refresh().await.unwrap();

    let (tx, mut rx) = mpsc::channel(8);
    // Review 2.1: the listener must offer exactly the extranonce total the
    // front's coinbase template is built for (`config().extranonce_len`).
    let expected_extranonce_len = config().extranonce_len;
    let server = Sv2Server::new(
        Network::Regtest,
        tx,
        std::sync::Arc::new(std::sync::Mutex::new(
            spc_pool::type32::Type32Store::default(),
        )),
        expected_extranonce_len,
    );
    let (addr, _h) = server.bind("127.0.0.1:0").await.unwrap();
    server.publish(job.clone()).await;

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
        vendor: Str0255::try_from("test").unwrap(),
        hardware_version: Str0255::try_from("1").unwrap(),
        firmware: Str0255::try_from("1").unwrap(),
        device_id: Str0255::try_from("1").unwrap(),
    };
    write_msg(&mut write, setup, MESSAGE_TYPE_SETUP_CONNECTION, false).await;
    let mut f = read.read_frame().await.unwrap();
    assert_eq!(f.header().msg_type(), 0x01);
    // Consume and validate the SetupConnection.Success payload.
    let mut payload = f.payload().to_vec();
    let _: SetupConnectionSuccess = binary_sv2::from_bytes(&mut payload).unwrap();

    let secp = bitcoin::secp256k1::Secp256k1::new();
    let scan_sk = bitcoin::secp256k1::SecretKey::from_slice(&[0x11; 32]).unwrap();
    let spend_sk = bitcoin::secp256k1::SecretKey::from_slice(&[0x12; 32]).unwrap();
    let scan = bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &scan_sk);
    let spend = bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &spend_sk);
    let address = spc_crypto::address::encode_sp_address(Network::Regtest, &scan, &spend).unwrap();

    let max_target_bytes = [0xffu8; 32];
    let open = OpenExtendedMiningChannel {
        request_id: 1,
        user_identity: Str0255::try_from(address.as_str()).unwrap(),
        nominal_hash_rate: 1.0,
        max_target: U256::from(&max_target_bytes),
        min_extranonce_size: 4,
    };
    write_msg(
        &mut write,
        open,
        MESSAGE_TYPE_OPEN_EXTENDED_MINING_CHANNEL,
        false,
    )
    .await;

    let mut extranonce_prefix = Vec::new();
    let mut job_id = 0u32;
    let mut prev_hash = [0u8; 32];
    let mut coinb1 = Vec::new();
    let mut coinb2 = Vec::new();
    let mut ntime = 0u32;
    let mut nbits = 0u32;
    let mut version = 0u32;
    let mut merkle_path: Vec<[u8; 32]> = Vec::new();
    loop {
        let mut f = read.read_frame().await.unwrap();
        let ty = f.header().msg_type();
        let payload = f.payload().to_vec();
        match ty {
            0x14 => {
                let mut p = payload.clone();
                let m: stratum_core::mining_sv2::OpenExtendedMiningChannelSuccess =
                    binary_sv2::from_bytes(&mut p).unwrap();
                extranonce_prefix = m.extranonce_prefix.as_bytes().to_vec();
            }
            0x20 => {
                let mut p = payload.clone();
                let m: stratum_core::mining_sv2::SetNewPrevHash =
                    binary_sv2::from_bytes(&mut p).unwrap();
                prev_hash.copy_from_slice(m.prev_hash.as_bytes());
                ntime = m.min_ntime;
                nbits = m.nbits;
            }
            MESSAGE_TYPE_NEW_EXTENDED_MINING_JOB => {
                let mut p = payload.clone();
                let m: stratum_core::mining_sv2::NewExtendedMiningJob =
                    binary_sv2::from_bytes(&mut p).unwrap();
                job_id = m.job_id;
                version = m.version;
                coinb1 = m.coinbase_tx_prefix.as_bytes().to_vec();
                coinb2 = m.coinbase_tx_suffix.as_bytes().to_vec();
                merkle_path = m
                    .merkle_path
                    .iter()
                    .map(|b| {
                        let mut a = [0u8; 32];
                        a.copy_from_slice(b.as_bytes());
                        a
                    })
                    .collect();
                break;
            }
            _ => {}
        }
    }
    assert_eq!(extranonce_prefix.len(), 4);

    let extranonce = vec![0xaa; 4];
    let mut coinbase = coinb1.clone();
    coinbase.extend_from_slice(&extranonce_prefix);
    coinbase.extend_from_slice(&extranonce);
    coinbase.extend_from_slice(&coinb2);
    let mut merkle_root = bitcoin::hashes::sha256d::Hash::hash(&coinbase).to_byte_array();
    for branch in merkle_path.iter().rev() {
        let mut buf = Vec::with_capacity(64);
        buf.extend_from_slice(&merkle_root);
        buf.extend_from_slice(branch);
        merkle_root = bitcoin::hashes::sha256d::Hash::hash(&buf).to_byte_array();
    }
    let target = spc_pool::sv2_server::target_from_bits(nbits);
    let mut nonce = 0u32;
    loop {
        let mut header = Vec::with_capacity(80);
        header.extend_from_slice(&version.to_le_bytes());
        header.extend_from_slice(&prev_hash);
        header.extend_from_slice(&merkle_root);
        header.extend_from_slice(&ntime.to_le_bytes());
        header.extend_from_slice(&nbits.to_le_bytes());
        header.extend_from_slice(&nonce.to_le_bytes());
        let h = bitcoin::hashes::sha256d::Hash::hash(&header).to_byte_array();
        if target.is_met_by(bitcoin::BlockHash::from_byte_array(h)) {
            break;
        }
        nonce = nonce.wrapping_add(1);
        if nonce > 2_000_000 {
            panic!("no nonce");
        }
    }

    let submit = SubmitSharesExtended {
        channel_id: 1,
        sequence_number: 0,
        job_id,
        nonce,
        ntime,
        version,
        extranonce: B032::new(&extranonce).unwrap(),
    };
    write_msg(
        &mut write,
        submit,
        MESSAGE_TYPE_SUBMIT_SHARES_EXTENDED,
        true,
    )
    .await;
    let f = read.read_frame().await.unwrap();
    assert_eq!(f.header().msg_type(), 0x1c);

    let share = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .unwrap()
        .unwrap();
    // Review 2.1: the pool must reconstruct exactly the coinbase the miner
    // rolled: coinbase_prefix ‖ extranonce_prefix ‖ miner extranonce ‖
    // coinbase_suffix — no silent length drift between ingress and template.
    let mut expected_extranonce = extranonce_prefix.clone();
    expected_extranonce.extend_from_slice(&extranonce);
    assert_eq!(share.extranonce, expected_extranonce);
    assert_eq!(share.coinbase, coinbase);
    let block = front.on_share(share).unwrap().expect("block");
    source.submit_block(&block.block).await.unwrap();
    assert_eq!(source.submitted().len(), 1);
}

/// Review 2.1: a listener whose configured extranonce total does not match
/// the 4+4 layout it offers MUST refuse the channel open — otherwise the
/// coinbase `coinb1 ‖ extranonce ‖ coinb2` reassembly silently mis-splits.
#[tokio::test]
async fn extranonce_layout_mismatch_opens_no_channel() {
    let (tx, _rx) = mpsc::channel(8);
    let server = Sv2Server::new(
        Network::Regtest,
        tx,
        std::sync::Arc::new(std::sync::Mutex::new(
            spc_pool::type32::Type32Store::default(),
        )),
        16,
    );
    let (addr, _h) = server.bind("127.0.0.1:0").await.unwrap();

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
        vendor: Str0255::try_from("test").unwrap(),
        hardware_version: Str0255::try_from("1").unwrap(),
        firmware: Str0255::try_from("1").unwrap(),
        device_id: Str0255::try_from("1").unwrap(),
    };
    write_msg(&mut write, setup, MESSAGE_TYPE_SETUP_CONNECTION, false).await;
    let _ = read.read_frame().await.unwrap();

    let secp = bitcoin::secp256k1::Secp256k1::new();
    let scan_sk = bitcoin::secp256k1::SecretKey::from_slice(&[0x11; 32]).unwrap();
    let spend_sk = bitcoin::secp256k1::SecretKey::from_slice(&[0x12; 32]).unwrap();
    let scan = bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &scan_sk);
    let spend = bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &spend_sk);
    let address = spc_crypto::address::encode_sp_address(Network::Regtest, &scan, &spend).unwrap();

    let max_target_bytes = [0xffu8; 32];
    let open = OpenExtendedMiningChannel {
        request_id: 1,
        user_identity: Str0255::try_from(address.as_str()).unwrap(),
        nominal_hash_rate: 1.0,
        max_target: U256::from(&max_target_bytes),
        min_extranonce_size: 4,
    };
    write_msg(
        &mut write,
        open,
        MESSAGE_TYPE_OPEN_EXTENDED_MINING_CHANNEL,
        false,
    )
    .await;

    let mut f = read.read_frame().await.unwrap();
    assert_eq!(
        f.header().msg_type(),
        MESSAGE_TYPE_OPEN_MINING_CHANNEL_ERROR
    );
    let mut payload = f.payload().to_vec();
    let err: OpenMiningChannelError = binary_sv2::from_bytes(&mut payload).unwrap();
    assert_eq!(err.error_code.as_bytes(), b"min-extranonce-size-too-large");
}
