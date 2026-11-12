//! AP-3a wallet receive integration tests (plan 4.4): ownership-gated
//! persistence, expiry provenance, reconciliation - against a running
//! role with the mock backend seam that AP-3b replaces with live
//! mailbox/status RPCs.

#![cfg(feature = "ark")]

mod wallet_common;

use std::net::SocketAddr;

use bitcoin::hashes::{sha256, Hash as _};
use bitcoin::secp256k1::PublicKey;
use spc_agent::wallet::receive::{Expiry, ExpirySource, ObservedVtxo, ReceiveBackend, VtxoState};
use spc_agent::wallet::{Reconciliation, WalletRole};
use spc_crypto::Network;
use wallet_common::{
    a_send, provision_files, test_settings, MockReceiveBackend, RecordWriteExt as _,
    WalletCommonExt as _,
};

fn scenario_master(tag: u8) -> [u8; 64] {
    let mut seed = [0u8; 64];
    seed[..32].copy_from_slice(
        &sha256::Hash::hash(format!("spc-ap3a-receive-{tag}-left").as_bytes()).to_byte_array(),
    );
    seed[32..].copy_from_slice(
        &sha256::Hash::hash(format!("spc-ap3a-receive-{tag}-right").as_bytes()).to_byte_array(),
    );
    seed
}

async fn start_role(tag: u8, datadir: &std::path::Path) -> (WalletRole, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let (sources, _) = provision_files(dir.path(), &scenario_master(tag), Network::Regtest);
    let settings = test_settings(&sources, datadir, SocketAddr::from(([127, 0, 0, 1], 0)));
    let role = WalletRole::start(settings).await.unwrap();
    (role, dir)
}

fn observation(
    vtxo_id: [u8; 36],
    u_pubkey: PublicKey,
    amount_sats: u64,
    state: VtxoState,
    expiry: Expiry,
) -> ObservedVtxo {
    ObservedVtxo {
        vtxo_id,
        u_pubkey,
        amount_sats,
        expiry,
        state,
    }
}

fn vtxo_id(tag: u8) -> [u8; 36] {
    let mut id = [0u8; 36];
    id[0] = tag;
    id
}

/// The mailbox poll -> ledger record path: only VTXOs paying an owned
/// `U` are recorded, expiries carry provenance, and a re-poll does not
/// duplicate claims.
#[tokio::test]
async fn receive_records_owned_vtxos_from_the_backend() {
    let dir = tempfile::tempdir().unwrap();
    let (role, _keys_dir) = start_role(1, &dir.path().join("datadir")).await;

    let height = 800_000;
    let miner = a_send(0x11);
    let u = role.derive_u(height, &miner);

    // A VTXO paying a U we do not own: refused before any write.
    let mut backend = MockReceiveBackend {
        observations: vec![observation(
            vtxo_id(1),
            a_send(0x99),
            1_000,
            VtxoState::Spendable,
            Expiry::Unknown,
        )],
    };
    let stranger = backend.poll_mailbox(Some(height)).unwrap();
    let err = role.ledger().record(&stranger[0]).unwrap_err();
    assert!(
        format!("{err}").contains("does not own"),
        "unowned VTXO must be refused: {err}"
    );
    assert_eq!(role.ledger().records().unwrap().len(), 0);

    // The owned one: recorded, idempotent on re-poll.
    let obs = observation(vtxo_id(2), u, 12_345, VtxoState::Pending, Expiry::Unknown);
    let mut backend = MockReceiveBackend {
        observations: vec![obs.clone()],
    };
    let polled = backend.poll_mailbox(Some(height)).unwrap();
    assert!(role.ledger().record(&polled[0]).unwrap().is_inserted());
    let re_polled = backend.poll_mailbox(Some(height)).unwrap();
    assert!(role
        .ledger()
        .record(&re_polled[0])
        .unwrap()
        .is_already_present());
    assert_eq!(role.ledger().records().unwrap().len(), 1);

    // Unknown expiry: no deadline, no invented finding-height expiry,
    // balances honest.
    let snapshot = role.status().snapshot().unwrap();
    assert_eq!(snapshot.pending_sats, Some(12_345));
    assert_eq!(snapshot.spendable_sats, Some(0));
    assert_eq!(snapshot.min_expiry_height, None);
    assert_eq!(snapshot.next_refresh_height, None);
    assert_eq!(snapshot.needs_refresh_sats, None, "tip unknown: not zero");

    // Server validation: spendable with the actual (ancestry) expiry.
    let validated = observation(
        vtxo_id(2),
        u,
        12_345,
        VtxoState::Spendable,
        Expiry::At {
            height: 800_432,
            source: ExpirySource::VtxoAncestry,
        },
    );
    assert!(role.ledger().record(&validated).unwrap().is_updated());
    role.store().set_scan_height(800_000).unwrap();
    let snapshot = role.status().snapshot().unwrap();
    assert_eq!(snapshot.spendable_sats, Some(12_345));
    assert_eq!(snapshot.min_expiry_height, Some(800_432));
    assert_eq!(snapshot.next_refresh_height, Some(800_432 - 96));
    assert_eq!(snapshot.last_scan_height, Some(800_000));
}

/// Reconciliation against the pool-reported total: equal is ok, a
/// mismatch alarms in `last_error`, never a silent adjustment.
#[tokio::test]
async fn reconciliation_alarms_on_pool_mismatch() {
    let dir = tempfile::tempdir().unwrap();
    let (role, _keys_dir) = start_role(2, &dir.path().join("datadir")).await;
    let height = 800_000;
    let u = role.derive_u(height, &a_send(0x21));
    role.ledger()
        .record(&observation(
            vtxo_id(3),
            u,
            5_000,
            VtxoState::Spendable,
            Expiry::At {
                height: 800_900,
                source: ExpirySource::VtxoAncestry,
            },
        ))
        .unwrap();

    assert_eq!(role.ledger().reconcile(5_000).unwrap(), Reconciliation::Ok);
    let mismatch = role.ledger().reconcile(7_500).unwrap();
    assert_eq!(
        mismatch,
        Reconciliation::Mismatch {
            pool_reported_sats: 7_500,
            owned_sats: 5_000
        }
    );
    // The alarm is surfaced in the shareable status, not silently
    // adjusted.
    role.status().set_error(Some(format!("{mismatch:?}")));
    let body = role.http_status_json().await;
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["spendable_sats"], 5_000);
    assert!(
        json["last_error"].as_str().unwrap().contains("mismatch")
            || json["last_error"].as_str().unwrap().contains("7_500")
            || json["last_error"].as_str().unwrap().contains("7500")
    );
}

/// A VTXO whose U is owned but whose keystore secret does not derive
/// it (corrupt keystore) is refused - it is never treated as a payout.
#[tokio::test]
async fn corrupt_keystore_secret_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let datadir = dir.path().join("datadir");
    let (role, _keys_dir) = start_role(3, &datadir).await;
    let height = 800_000;
    let u = role.derive_u(height, &a_send(0x31));
    // Corrupt the stored secret for U via a direct connection (swap in
    // a different secret).
    {
        let db = rusqlite::Connection::open(datadir.join("state.sqlite")).unwrap();
        db.execute(
            "UPDATE sp_keystore SET u_secret = ?1 WHERE u_pubkey = ?2",
            rusqlite::params![
                bitcoin::secp256k1::SecretKey::from_slice(&[0x77u8; 32])
                    .unwrap()
                    .secret_bytes()
                    .to_vec(),
                u.serialize().to_vec()
            ],
        )
        .unwrap();
    }
    let err = role
        .ledger()
        .record(&observation(
            vtxo_id(4),
            u,
            1_000,
            VtxoState::Spendable,
            Expiry::Unknown,
        ))
        .unwrap_err();
    assert!(
        format!("{err}").contains("does not derive"),
        "corrupt keystore must fail closed: {err}"
    );
    assert_eq!(role.ledger().records().unwrap().len(), 0);
}
