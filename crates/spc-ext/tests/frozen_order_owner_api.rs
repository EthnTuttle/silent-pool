//! Raw codec compatibility with the source-bound producer storage fixture.
//! Not a valid Ark destination, successful HTTP response, signature-verification
//! claim (no fixture verification key is supplied), or funded delivery evidence.
use bitcoin::hashes::{sha256, Hash};
use spc_ext::{authorization::order_digest, ArkSendOrder};

const STORED_HEX: &str = "fb6282a0354c4453bccfcf336ee364c6157461726b31756e69742d64657374696e6174696f6e1027000000000000cf0000000393fe731a46613b373e1ab6f16d9ce6d027d4b8c692537e54c4f596edb645ba555ba25071c27dd8416798be05916a760403756671a490611638925c3d86765b";

fn decode_hex(text: &str) -> Vec<u8> {
    text.as_bytes()
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

#[test]
fn producer_stored_bytes_roundtrip_and_full_signed_order_digest_are_exact() {
    // /tmp/opencode/ladder-followup-source-r3/handoff.json:109-115.
    let bytes = decode_hex(STORED_HEX);
    assert_eq!(bytes.len(), 114);
    assert_eq!(
        sha256::Hash::hash(&bytes).to_string(),
        "3bec7f90d615dba1547bc52be602bb735267c3d6bb327411bd0e9b577b0315b5"
    );
    let order = ArkSendOrder::decode(&bytes).unwrap();
    assert_eq!(
        order.order_id.as_slice(),
        decode_hex("fb6282a0354c4453bccfcf336ee364c6")
    );
    assert_eq!(order.ark_address, "tark1unit-destination");
    assert_eq!(order.sats, 10_000);
    assert_eq!(order.not_before_height, 207);
    assert_eq!(order.encode().unwrap(), bytes);
    assert_eq!(
        order_digest(&order).unwrap().as_slice(),
        decode_hex("e4ce8b87d9bccbb7c9b57159db7b2f2b4053e43c940488a2c1a67620753cd710")
    );
    let mut changed = order;
    changed.sig[0] ^= 1;
    assert_ne!(
        order_digest(&changed).unwrap().as_slice(),
        decode_hex("e4ce8b87d9bccbb7c9b57159db7b2f2b4053e43c940488a2c1a67620753cd710")
    );
}

#[test]
fn producer_fixed_and_maximum_lengths_and_strict_framing_match() {
    let mut order = ArkSendOrder::decode(&decode_hex(STORED_HEX)).unwrap();
    order.ark_address.clear();
    assert_eq!(order.encode().unwrap().len(), ArkSendOrder::MIN_ENCODED_LEN);
    assert_eq!(ArkSendOrder::MIN_ENCODED_LEN, 93);
    order.ark_address = "x".repeat(255);
    let bytes = order.encode().unwrap();
    assert_eq!(bytes.len(), ArkSendOrder::MAX_ENCODED_LEN);
    assert_eq!(ArkSendOrder::MAX_ENCODED_LEN, 348);
    assert_eq!(ArkSendOrder::decode(&bytes).unwrap(), order);
    assert!(ArkSendOrder::decode(&bytes[..bytes.len() - 1]).is_err());
    let mut trailing = bytes;
    trailing.push(0);
    assert!(ArkSendOrder::decode(&trailing).is_err());
    order.ark_address.push('x');
    assert!(order.encode().is_err());
}
