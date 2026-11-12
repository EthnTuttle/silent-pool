//! External signature admission uses the existing frozen signer as its fixture
//! producer. This is offline conformance, not native funding evidence.
use bitcoin::hashes::Hash;
use bitcoin::{BlockHash, OutPoint, Txid};
use spc_lab_txd::builder::{build_and_sign, DestinationSpec, PaymentCategory, SignableUtxo};
use spc_lab_txd::journal::{FundingContext, FundingObservation, FundingSource};
use spc_lab_txd::keys::{Chain, TrafficWallet, WalletRole};
use spc_lab_txd::wallet::{ProvisionConfig, TrafficWalletSet};

#[test]
fn validated_external_bytes_enter_once_and_invalid_signatures_never_reserve() {
    let dir = tempfile::tempdir().unwrap();
    let config = ProvisionConfig::standard(42, "declared-fixture", "txd");
    let allocation = ProvisionConfig::validate_allocation(std::slice::from_ref(&config)).unwrap();
    let set = TrafficWalletSet::provision(&allocation, "txd", dir.path()).unwrap();
    let point = OutPoint::new(Txid::from_byte_array([9; 32]), 0);
    set.observe_funding(&FundingObservation {
        outpoint: point,
        amount_sats: 1_000_000,
        chain: Chain::Receive,
        index: 0,
        owner_role: WalletRole::BootstrapFunding,
        source: FundingSource::CanonicalBlock,
        context: FundingContext::CanonicalBlock {
            block_hash: BlockHash::from_byte_array([1; 32]).to_string(),
            height: 1,
            coinbase: false,
        },
    })
    .unwrap();
    let offline = TrafficWallet::new(
        config.run_seed,
        &config.run_id,
        WalletRole::BootstrapFunding,
        config.network,
        config.lookahead,
    )
    .unwrap();
    let script = offline.script_at(Chain::Receive, 0).unwrap().script_pubkey;
    let input =
        SignableUtxo::resolve_indexed(&offline, point, Chain::Receive, 0, &script, 1_000_000)
            .unwrap();
    let destination = DestinationSpec {
        script_pubkey: set
            .next_receive_address(&WalletRole::NetworkWallet { index: 0 })
            .unwrap()
            .script_pubkey,
        amount_sats: 100_000,
        owner: None,
    };
    let signed = build_and_sign(
        &bitcoin::secp256k1::Secp256k1::new(),
        &offline,
        &[input],
        &[destination],
        3,
        &config.fee_policy,
        0,
        PaymentCategory::Ordinary,
    )
    .unwrap();
    let mut invalid = signed.tx.clone();
    let mut sig = invalid.input[0].script_sig.to_bytes();
    sig[10] ^= 1;
    invalid.input[0].script_sig = bitcoin::ScriptBuf::from_bytes(sig);
    let before = std::fs::read(set.journal().path()).unwrap();
    assert!(spc_lab_txd::admit::admit_declared_tx(
        &set,
        "bad",
        &hex::encode(bitcoin::consensus::serialize(&invalid)),
        1000
    )
    .is_err());
    assert_eq!(std::fs::read(set.journal().path()).unwrap(), before);
    let raw = hex::encode(&signed.raw_bytes);
    let admitted = spc_lab_txd::admit::admit_declared_tx(&set, "inject", &raw, 1000).unwrap();
    assert_eq!(admitted.raw_hex, raw);
    assert_eq!(admitted.fee_sats, signed.fee_sats);
    assert_eq!(admitted.spent_outpoints, vec![point]);
    let committed = set.journal().last_seq();
    let replay = spc_lab_txd::admit::admit_declared_tx(&set, "inject", &raw, 1000).unwrap();
    assert_eq!(replay.raw_hex, raw);
    let alias = spc_lab_txd::admit::admit_declared_tx(&set, "same-bytes", &raw, 1000).unwrap();
    assert_eq!(alias.command_id, "inject");
    assert_eq!(set.journal().last_seq(), committed);
    assert_eq!(set.journal().next_send_sequence(), 1);
    assert_eq!(set.note_broadcast_sent("inject").unwrap(), 1);
    set.note_unknown("inject", "fixture uncertain").unwrap();
    assert!(spc_lab_txd::admit::admit_declared_tx(
        &set,
        "inject",
        &hex::encode(bitcoin::consensus::serialize(&invalid)),
        1000
    )
    .is_err());
    assert_eq!(set.journal().next_send_sequence(), 2);
}
