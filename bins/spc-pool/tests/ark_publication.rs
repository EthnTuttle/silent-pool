//! Live `0x5350` -> payout -> front-event -> frozen-destination integration.
//! Uses real Noise/SV2, PostgreSQL and Redis; no node or Ark server is claimed.

use std::sync::Arc;
use std::time::Duration;

use bitcoin::hashes::{sha256d, Hash};
use bitcoin::pow::Target;
use bitcoin::secp256k1::{PublicKey as BitcoinPub, Secp256k1 as BitcoinSecp, SecretKey};
use secp256k1::{Keypair, PublicKey, Secp256k1};
use spc_crypto::Network;
use spc_ext::{
    DeliveryMessage, EpochFallback, MailboxMode, RegistrationSuccessState, SetArkIdentity,
};
use spc_pool::front::{Front, FrontConfig};
use spc_pool::identity::ChannelIdentity;
use spc_pool::payout::{AlwaysConfirm, PayoutService};
use spc_pool::registration::{ArkRegistrationHandler, ArkServerMetadata, RegistrationService};
use spc_pool::sv1::Share;
use spc_pool::sv2_server::Sv2Server;
use spc_pool::template::{MockTemplateSource, Template};
use spc_pool::type32::Type32Store;
use spc_store::models::BootstrapSnapshot;
use spc_store::redis::Streams;
use spc_store::wire::{FrontEvent, Rail};
use spc_store::Store;
use stratum_apps::network_helpers::connect_with_noise;
use stratum_apps::network_helpers::noise_stream::{NoiseTcpReadHalf, NoiseTcpWriteHalf};
use stratum_core::binary_sv2::{self, Seq064K, Str0255, U256};
use stratum_core::common_messages_sv2::{Protocol, SetupConnection};
use stratum_core::extensions_sv2::{
    RequestExtensions, RequestExtensionsSuccess, EXTENSION_TYPE_EXTENSIONS_NEGOTIATION,
    MESSAGE_TYPE_REQUEST_EXTENSIONS, MESSAGE_TYPE_REQUEST_EXTENSIONS_SUCCESS,
};
use stratum_core::framing_sv2::framing::MessageFrame;
use stratum_core::mining_sv2::OpenExtendedMiningChannel;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::time::timeout;
use uuid::Uuid;

fn key(seed: u8) -> Keypair {
    Keypair::from_secret_key(
        &Secp256k1::new(),
        &secp256k1::SecretKey::from_slice(&[seed; 32]).unwrap(),
    )
}

fn public(seed: u8) -> [u8; 33] {
    PublicKey::from_keypair(&key(seed)).serialize()
}

fn metadata() -> ArkServerMetadata {
    let server = PublicKey::from_keypair(&key(9));
    ArkServerMetadata {
        ark_id: <[u8; 4]>::from(ark::address::ArkId::from_server_pubkey(server)),
        server_pubkey: server,
        mailbox_pubkey: PublicKey::from_keypair(&key(10)),
        testnet: true,
    }
}

fn identity(seed: u8) -> ChannelIdentity {
    let secp = BitcoinSecp::new();
    let scan = BitcoinPub::from_secret_key(&secp, &SecretKey::from_slice(&[seed; 32]).unwrap());
    let spend =
        BitcoinPub::from_secret_key(&secp, &SecretKey::from_slice(&[seed + 1; 32]).unwrap());
    ChannelIdentity {
        sp_id: spc_crypto::sp_id(&scan, &spend),
        b_scan: scan.serialize(),
        b_spend: spend.serialize(),
        worker: None,
    }
}

fn request(identity: &ChannelIdentity, seed: u8, id: u32, mode: MailboxMode) -> SetArkIdentity {
    let mut request = SetArkIdentity {
        ark_id: metadata().ark_id,
        b_ark: public(seed + 10),
        mailbox_mode: mode,
        mailbox_pubkey: if mode == MailboxMode::None {
            vec![]
        } else {
            public(seed + 20).to_vec()
        },
        mailbox_dh: if mode == MailboxMode::None {
            vec![]
        } else {
            public(seed + 30).to_vec()
        },
        epoch_fallback: if mode == MailboxMode::EpochRegistered {
            EpochFallback::Hold
        } else {
            EpochFallback::Static
        },
        sig: [0; 64],
        request_id: id,
    };
    request.sign(&identity.sp_id, &key(seed)).unwrap();
    request
}

async fn isolated_store(tag: &str) -> Option<(Store, String)> {
    let url = std::env::var("SPC_TEST_DATABASE_URL").ok()?;
    let base = url.split('?').next().unwrap_or(&url);
    let (head, _) = base.rsplit_once('/').expect("database path");
    let admin = sqlx::PgPool::connect(&format!("{head}/postgres"))
        .await
        .unwrap();
    let name = format!("{tag}{}", Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(&admin)
        .await
        .unwrap();
    let store = Store::connect(&format!("{head}/{name}")).await.unwrap();
    store.migrate().await.unwrap();
    Some((store, name))
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
    let secp = BitcoinSecp::new();
    let root = bitcoin::bip32::Xpriv::new_master(bitcoin::Network::Testnet, &[0x5c; 32]).unwrap();
    let xpub = bitcoin::bip32::Xpub::from_priv(&secp, &root);
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

fn new_front(store_pending: &[ChannelIdentity]) -> Front {
    let mut front = Front::new(
        front_config(),
        Arc::new(MockTemplateSource::new(template())),
        Arc::new(std::sync::Mutex::new(Type32Store::default())),
    );
    let snapshot = BootstrapSnapshot {
        economic: None,
        rev: 0,
        applied_blocks: vec![],
        pending_balance: store_pending
            .iter()
            .map(|identity| (identity.sp_id, 100))
            .collect(),
        ark_delivery: vec![],
        ark_identities: vec![],
        ark_epoch_mailboxes: vec![],
        window_tail: vec![],
        type32_blobs: vec![],
        slices_cursor: None,
        front_events_cursor: None,
    };
    front.apply_bootstrap(&snapshot).unwrap();
    front.set_ark_server(metadata());
    front
}

fn share(front: &Front, identity: &ChannelIdentity) -> Share {
    let job = front.job().unwrap().clone();
    let extranonce = vec![0xaau8; 8];
    let mut coinbase = job.coinb1.clone();
    coinbase.extend_from_slice(&extranonce);
    coinbase.extend_from_slice(&job.coinb2);
    let mut merkle_root = sha256d::Hash::hash(&coinbase).to_byte_array();
    for branch in &job.merkle_branches {
        let mut pair = Vec::with_capacity(64);
        pair.extend_from_slice(&merkle_root);
        pair.extend_from_slice(branch);
        merkle_root = sha256d::Hash::hash(&pair).to_byte_array();
    }
    let target: Target = spc_pool::sv2_server::target_from_bits(job.nbits);
    let mut nonce = 0u32;
    let share_hash = loop {
        let mut header = Vec::with_capacity(80);
        header.extend_from_slice(&job.version.to_le_bytes());
        header.extend_from_slice(&job.prev_hash);
        header.extend_from_slice(&merkle_root);
        header.extend_from_slice(&job.ntime.to_le_bytes());
        header.extend_from_slice(&job.nbits.to_le_bytes());
        header.extend_from_slice(&nonce.to_le_bytes());
        let hash = sha256d::Hash::hash(&header).to_byte_array();
        if target.is_met_by(bitcoin::BlockHash::from_byte_array(hash)) {
            break hash;
        }
        nonce = nonce.wrapping_add(1);
        assert!(nonce < 2_000_000);
    };
    Share {
        sp_id: identity.sp_id,
        b_scan: identity.b_scan,
        b_spend: identity.b_spend,
        channel_id: 1,
        job_id: job.job_id,
        nonce,
        ntime: job.ntime,
        version: job.version,
        extranonce,
        share_index: 0,
        job_fees: 0,
        coinbase,
        merkle_root,
        share_hash,
    }
}

async fn write_typed<T>(write: &mut NoiseTcpWriteHalf, message: T, msg_type: u8, ext_type: u16)
where
    T: binary_sv2::Serialize + binary_sv2::GetSize,
{
    write
        .write_frame(MessageFrame::from_message(message, msg_type, ext_type, false).unwrap())
        .await
        .unwrap();
}

async fn write_raw(write: &mut NoiseTcpWriteHalf, msg_type: u8, payload: &[u8]) {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&spc_ext::EXT_DELIVERY.to_le_bytes());
    bytes.push(msg_type);
    bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes()[..3]);
    bytes.extend_from_slice(payload);
    write
        .write_frame(stratum_core::codec_sv2::SerializedFrame::from_bytes(bytes).unwrap())
        .await
        .unwrap();
}

async fn setup(write: &mut NoiseTcpWriteHalf, read: &mut NoiseTcpReadHalf) {
    write_typed(
        write,
        SetupConnection {
            protocol: Protocol::MiningProtocol,
            min_version: 2,
            max_version: 2,
            flags: 0,
            endpoint_host: Str0255::try_from("127.0.0.1").unwrap(),
            endpoint_port: 0,
            vendor: Str0255::try_from("ap2").unwrap(),
            hardware_version: Str0255::try_from("1").unwrap(),
            firmware: Str0255::try_from("1").unwrap(),
            device_id: Str0255::try_from("1").unwrap(),
        },
        0,
        0,
    )
    .await;
    assert_eq!(read.read_frame().await.unwrap().header().msg_type(), 1);
}

async fn negotiate(write: &mut NoiseTcpWriteHalf, read: &mut NoiseTcpReadHalf) -> Vec<u16> {
    write_typed(
        write,
        RequestExtensions {
            request_id: 41,
            requested_extensions: Seq064K::new(vec![spc_ext::EXT_DELIVERY]).unwrap(),
        },
        MESSAGE_TYPE_REQUEST_EXTENSIONS,
        EXTENSION_TYPE_EXTENSIONS_NEGOTIATION,
    )
    .await;
    let mut frame = read.read_frame().await.unwrap();
    assert_eq!(
        frame.header().msg_type(),
        MESSAGE_TYPE_REQUEST_EXTENSIONS_SUCCESS
    );
    let mut response_payload = frame.payload().to_vec();
    let response: RequestExtensionsSuccess = binary_sv2::from_bytes(&mut response_payload).unwrap();
    response.supported_extensions.iter().copied().collect()
}

async fn open(
    write: &mut NoiseTcpWriteHalf,
    read: &mut NoiseTcpReadHalf,
    identity: &ChannelIdentity,
) {
    let secp = BitcoinSecp::new();
    let address = spc_crypto::address::encode_sp_address(
        Network::Regtest,
        &BitcoinPub::from_slice(&identity.b_scan).unwrap(),
        &BitcoinPub::from_slice(&identity.b_spend).unwrap(),
    )
    .unwrap();
    let _ = secp;
    write_typed(
        write,
        OpenExtendedMiningChannel {
            request_id: 42,
            user_identity: Str0255::try_from(address.as_str()).unwrap(),
            nominal_hash_rate: 1.0,
            max_target: U256::from(&[0xffu8; 32]),
            min_extranonce_size: 4,
        },
        stratum_core::mining_sv2::MESSAGE_TYPE_OPEN_EXTENDED_MINING_CHANNEL,
        0,
    )
    .await;
    assert_eq!(
        read.read_frame().await.unwrap().header().msg_type(),
        stratum_core::mining_sv2::MESSAGE_TYPE_OPEN_EXTENDED_MINING_CHANNEL_SUCCESS
    );
}

#[tokio::test]
async fn live_wire_event_and_bootstrap_freeze_destination_and_hold_underived_pending() {
    let Some((store, _)) = isolated_store("ap2pub_").await else {
        eprintln!("skip: storage URLs absent");
        return;
    };
    let streams = Streams::connect(&std::env::var("SPC_TEST_REDIS_URL").unwrap())
        .await
        .unwrap();
    let suffix = Uuid::new_v4().simple().to_string();
    let register_stream = format!("ap2:pub:register:{suffix}");
    let ack_key = format!("ap2:pub:ack:{suffix}");
    let front_stream = format!("ap2:pub:front:{suffix}");
    let payout = PayoutService::new(
        store.clone(),
        streams.clone(),
        6,
        330,
        Arc::new(AlwaysConfirm),
        None,
        100_000,
    )
    .with_registration_ark_id(metadata().ark_id)
    .with_registration_streams(register_stream.clone(), ack_key.clone())
    .with_front_events_stream(front_stream.clone())
    .with_network(3);
    let service = RegistrationService::new(streams.clone(), metadata().ark_id)
        .with_names(register_stream, ack_key)
        .with_ack_timeout(Duration::from_secs(2));

    let (share_tx, _) = mpsc::channel(4);
    let server = Sv2Server::new(
        Network::Regtest,
        share_tx,
        Arc::new(std::sync::Mutex::new(Type32Store::default())),
        8,
    )
    .with_ark_registration(Arc::new(service.clone()));
    let (addr, _server_task) = server.bind("127.0.0.1:0").await.unwrap();
    let socket = TcpStream::connect(addr).await.unwrap();
    let socket = connect_with_noise(socket, None).await.unwrap();
    let (mut read, mut write) = socket.into_split();
    setup(&mut write, &mut read).await;
    assert_eq!(
        negotiate(&mut write, &mut read).await,
        vec![spc_ext::EXT_DELIVERY]
    );

    let alice = identity(3);
    open(&mut write, &mut read, &alice).await;
    let first_request = request(&alice, 3, 1, MailboxMode::Static);
    write_raw(
        &mut write,
        spc_ext::MSG_SET_ARK_IDENTITY,
        &first_request.encode().unwrap(),
    )
    .await;
    let response = tokio::spawn(async move { read.read_frame().await.unwrap() });
    for _ in 0..30 {
        if response.is_finished() {
            break;
        }
        let _ = payout.process_registrations_once().await;
    }
    let mut response = timeout(Duration::from_secs(3), response)
        .await
        .expect("SV2 response timeout")
        .unwrap();
    let response =
        DeliveryMessage::decode(spc_ext::MSG_SET_ARK_IDENTITY_SUCCESS, response.payload()).unwrap();
    let DeliveryMessage::SetArkIdentitySuccess(success) = response else {
        panic!("expected success");
    };
    assert_eq!(success.state, RegistrationSuccessState::Applied);
    assert_eq!(success.ark_id, metadata().ark_id);
    assert!(store.ark_delivery(&alice.sp_id).await.unwrap().is_some());

    // Second real registration has an explicit hold fallback. It is durable
    // but has no epoch key, so no destination can be derived.
    let bob = identity(11);
    let second = request(&bob, 11, 1, MailboxMode::EpochRegistered);
    let second_task = tokio::spawn({
        let service = service.clone();
        let bob = bob.clone();
        async move {
            service
                .handle(
                    &bob,
                    spc_ext::MSG_SET_ARK_IDENTITY,
                    &second.encode().unwrap(),
                )
                .await
        }
    });
    for _ in 0..30 {
        if second_task.is_finished() {
            break;
        }
        let _ = payout.process_registrations_once().await;
    }
    assert!(matches!(
        second_task.await.unwrap(),
        DeliveryMessage::SetArkIdentitySuccess(_)
    ));

    // Consume the real payout events and apply them to a front with low carried
    // balances. One destination is derived; Bob's explicit hold stays pending.
    streams
        .ensure_group_from(&front_stream, "publication-proof", "0")
        .await
        .unwrap();
    let entries = streams
        .read_group(&front_stream, "publication-proof", "reader", 16)
        .await
        .unwrap();
    let mut front = new_front(&[alice.clone(), bob.clone()]);
    for (_, payload) in entries {
        let event = FrontEvent::decode(&payload).unwrap();
        assert!(matches!(event, FrontEvent::ArkRegApplied { .. }));
        front.apply_front_event(&event);
    }
    front.refresh().await.unwrap();
    let finding = share(&front, &alice);
    let _ = front.on_share(finding).unwrap().expect("block");
    let block = front
        .take_events()
        .into_iter()
        .find_map(|event| match event {
            spc_pool::front::FrontEvent::Block(block) => Some(block),
            _ => None,
        })
        .expect("frozen block event");
    let alice_entry = block
        .entries
        .iter()
        .find(|entry| entry.sp_id == alice.sp_id)
        .unwrap();
    let bob_entry = block
        .entries
        .iter()
        .find(|entry| entry.sp_id == bob.sp_id)
        .unwrap();
    assert_eq!(alice_entry.rail, Rail::Ark);
    assert!(!alice_entry.dest_id.is_empty());
    let address = String::from_utf8(alice_entry.dest_id.clone()).unwrap();
    let parsed = address.parse::<ark::Address>().unwrap();
    assert_eq!(&<[u8; 4]>::from(parsed.ark_id()), &metadata().ark_id);
    assert_eq!(bob_entry.rail, Rail::Pending);
    assert!(bob_entry.dest_id.is_empty());
    assert_eq!(block.height, 800_000);
    assert_eq!(block.a_send_x, front.distribution().unwrap().a_send_x);

    // A new front restores full identity/delivery/epoch state from the actual
    // database, not just a `registered` bit, then applies the same routing.
    let restored_snapshot = store.bootstrap(16).await.unwrap();
    assert!(!restored_snapshot.ark_identities.is_empty());
    assert_eq!(restored_snapshot.ark_delivery.len(), 2);
    let mut restored = new_front(&[alice.clone(), bob.clone()]);
    restored
        .apply_bootstrap(&BootstrapSnapshot {
            economic: None,
            pending_balance: restored.pending().clone().into_iter().collect(),
            ..restored_snapshot
        })
        .unwrap();
    restored.refresh().await.unwrap();
    let distribution = restored.distribution().unwrap().clone();
    let alice = distribution
        .entries
        .iter()
        .find(|entry| entry.sp_id == alice.sp_id)
        .unwrap();
    let bob = distribution
        .entries
        .iter()
        .find(|entry| entry.sp_id == bob.sp_id)
        .unwrap();
    assert_eq!(alice.rail, spc_accounting::Rail::Ark);
    assert!(alice.dest_id.is_some());
    assert_eq!(bob.rail, spc_accounting::Rail::Pending);
    assert!(bob.dest_id.is_none());
}

#[tokio::test]
async fn disabled_handler_neither_advertises_nor_pretends_success() {
    let (share_tx, _) = mpsc::channel(4);
    let server = Sv2Server::new(
        Network::Regtest,
        share_tx,
        Arc::new(std::sync::Mutex::new(Type32Store::default())),
        8,
    );
    let (addr, _server_task) = server.bind("127.0.0.1:0").await.unwrap();
    let socket = TcpStream::connect(addr).await.unwrap();
    let socket = connect_with_noise(socket, None).await.unwrap();
    let (mut read, mut write) = socket.into_split();
    setup(&mut write, &mut read).await;
    assert!(negotiate(&mut write, &mut read).await.is_empty());
    let alice = identity(3);
    open(&mut write, &mut read, &alice).await;
    // Even an invalid request cannot negotiate into a void or get success; the
    // disabled router returns its own explicit code before touching storage.
    write_raw(&mut write, spc_ext::MSG_SET_ARK_IDENTITY, &[1]).await;
    let mut response = read.read_frame().await.unwrap();
    let response =
        DeliveryMessage::decode(spc_ext::MSG_SET_ARK_IDENTITY_ERROR, response.payload()).unwrap();
    let DeliveryMessage::SetArkIdentityError(error) = response else {
        panic!("expected error");
    };
    assert_eq!(error.error_code, b"unsupported-extension");
    assert!(!error.retryable);
}

#[test]
fn ark_role_wire_payloads_omit_every_identity_field() {
    let sp_id = [0x51; 32];
    let block_hash = [0x52; 32];
    let b_scan = public(53);
    let b_spend = public(54);
    let p_i = [0x55; 32];
    let address = "ark1pw4zjupfzqqpgwa8ay9vc42v5pq6gs7ydgty86dztz9v25kjydkrv39agl6njdezqyprdqnynftx9g0p6lj4jqpnykkjlczlfxl8526956r88l8tn5qfggqlrm0es";
    let mut order = spc_ext::ArkSendOrder {
        order_id: spc_pool::payout::ark_order_id(&block_hash, &sp_id),
        ark_address: address.into(),
        sats: 100,
        not_before_height: 800_006,
        sig: [0; 64],
    };
    order.sign(&key(5)).unwrap();
    let order_bytes = order.encode().unwrap();
    let receipt = spc_ext::ArkSendReceipt {
        order_id: order.order_id,
        status: spc_ext::ReceiptStatus::Sent,
        vtxo_id: vec![0x61; 34],
        expiry_height: 900_000,
    };
    let receipt_bytes = receipt.encode().unwrap();
    for forbidden in [
        &sp_id[..],
        &block_hash[..],
        &b_scan[..],
        &b_spend[..],
        &p_i[..],
    ] {
        assert!(!order_bytes
            .windows(forbidden.len())
            .any(|window| window == forbidden));
        assert!(!receipt_bytes
            .windows(forbidden.len())
            .any(|window| window == forbidden));
    }
    assert_eq!(spc_ext::ArkSendOrder::decode(&order_bytes).unwrap(), order);
    assert_eq!(
        spc_ext::ArkSendReceipt::decode(&receipt_bytes).unwrap(),
        receipt
    );
}
