//! BIP54 vectors pinned to bitcoin/bips
//! f82c4612768827ea26e7b08cc895696db61bf073 (CC0-1.0).
//! Fixtures are unmodified bip-0054/test_vectors/{coinbases,txsize}.json.
//! These check the field/size rules, not full chain or script validation.

use bitcoin::consensus::{deserialize, serialize};
use spc_pool::coinbase::{build_pool_template, p2tr_script, try_build_pool_template_with_fields};
use spc_pool::template::{validate_bip54_fields, CoinbaseFields};

fn cleanup(height: u32) -> CoinbaseFields {
    CoinbaseFields::from_gbt_rules(height, &["!consensuscleanup".into()]).unwrap()
}

#[test]
fn official_coinbase_vectors() {
    let vectors: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/bip54-coinbases.json")).unwrap();
    for vector in vectors.as_array().unwrap() {
        let chain = vector["block_chain"].as_array().unwrap();
        let block: bitcoin::Block =
            deserialize(&hex::decode(chain.last().unwrap().as_str().unwrap()).unwrap()).unwrap();
        let tx = &block.txdata[0];
        let result = validate_bip54_fields(
            (chain.len() - 1) as u32,
            tx.lock_time.to_consensus_u32(),
            tx.input[0].sequence.0,
        );
        assert_eq!(
            result.is_ok(),
            vector["valid"].as_bool().unwrap(),
            "{}",
            vector["comment"]
        );
    }
}

#[test]
fn official_stripped_size_vectors() {
    let vectors: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/bip54-txsize.json")).unwrap();
    for vector in vectors.as_array().unwrap() {
        let tx: bitcoin::Transaction =
            deserialize(&hex::decode(vector["tx"].as_str().unwrap()).unwrap()).unwrap();
        assert_eq!(
            cleanup(1).validate_transaction(&tx).is_ok(),
            vector["valid"].as_bool().unwrap(),
            "{}",
            vector["comment"]
        );
        assert!(CoinbaseFields::legacy(0).validate_transaction(&tx).is_ok());
    }
}

#[test]
fn builder_preserves_legacy_bytes_and_changes_only_bip54_fields() {
    let outputs = [(p2tr_script(&[0x11; 32]), 1000)];
    let old = build_pool_template(2, 800_000, &[0x42; 32], 8, &outputs, 0);
    let explicit = try_build_pool_template_with_fields(
        2,
        800_000,
        &[0x42; 32],
        8,
        &outputs,
        CoinbaseFields::legacy(0),
    )
    .unwrap();
    let bip54 =
        try_build_pool_template_with_fields(2, 800_000, &[0x42; 32], 8, &outputs, cleanup(800_000))
            .unwrap();
    let raw = old.assemble(&[0x77; 8]);
    // Pre-change legacy wire vector: height 800000, 8-byte extranonce,
    // one 1000-sat P2TR output. No builder supplies the expected bytes.
    let expected = concat!(
        "02000000010000000000000000000000000000000000000000000000000000000000000000ffffffff2d0300350c20",
        "4242424242424242424242424242424242424242424242424242424242424242",
        "7777777777777777ffffffff01e803000000000000225120",
        "1111111111111111111111111111111111111111111111111111111111111111",
        "00000000"
    );
    assert_eq!(hex::encode(&raw), expected);
    assert_eq!(explicit, old);
    let mut tx: bitcoin::Transaction = deserialize(&raw).unwrap();
    tx.lock_time = bitcoin::absolute::LockTime::from_height(799_999).unwrap();
    tx.input[0].sequence = bitcoin::Sequence(0xffff_fffe);
    assert_eq!(bip54.assemble(&[0x77; 8]), serialize(&tx));
    cleanup(800_000)
        .validate_coinbase(800_000, &serialize(&tx))
        .unwrap();
    tx.input[0].sequence = bitcoin::Sequence::MAX;
    assert!(cleanup(800_000)
        .validate_coinbase(800_000, &serialize(&tx))
        .is_err());
    assert!(cleanup(800_001)
        .validate_coinbase(800_001, &bip54.assemble(&[0x77; 8]))
        .is_err());
}

#[test]
fn height_one_and_height_locktime_boundaries() {
    assert_eq!(cleanup(1).locktime, 0);
    assert_eq!(cleanup(1).sequence, 0xffff_fffe);
    assert_eq!(cleanup(500_000_000).locktime, 499_999_999);
    for height in [0, 500_000_001, u32::MAX] {
        assert!(CoinbaseFields::from_gbt_rules(height, &["!consensuscleanup".into()]).is_err());
    }
    let mut fields = cleanup(2);
    fields.sequence = u32::MAX;
    assert!(try_build_pool_template_with_fields(2, 2, &[0x42; 32], 8, &[], fields).is_err());
    fields = cleanup(2);
    fields.locktime = 2;
    assert!(try_build_pool_template_with_fields(2, 2, &[0x42; 32], 8, &[], fields).is_err());
}
