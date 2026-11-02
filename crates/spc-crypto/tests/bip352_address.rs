use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
use spc_crypto::{parse_sp_address, Network};

#[test]
fn parses_bip352_reference_address() {
    // BIP 352 send_and_receive_test_vectors.json, "Simple send: two inputs".
    let addr = "sp1qqgste7k9hx0qftg6qmwlkqtwuy6cycyavzmzj85c6qdfhjdpdjtdgqjuexzk6murw56suy3e0rd2cgqvycxttddwsvgxe2usfpxumr70xc9pkqwv";
    let scan_sk = SecretKey::from_slice(
        &hex::decode("0f694e068028a717f8af6b9411f9a133dd3565258714cc226594b34db90c1f2c").unwrap(),
    )
    .unwrap();
    let spend_sk = SecretKey::from_slice(
        &hex::decode("9d6ad855ce3417ef84e836892e5a56392bfba05fa5d97ccea30e266f540e08b3").unwrap(),
    )
    .unwrap();
    let secp = Secp256k1::new();
    let scan_pk = PublicKey::from_secret_key(&secp, &scan_sk);
    let spend_pk = PublicKey::from_secret_key(&secp, &spend_sk);
    let parsed = parse_sp_address(addr, Network::Mainnet).expect("BIP352 address must parse");
    assert_eq!(parsed.b_scan, scan_pk);
    assert_eq!(parsed.b_spend, spend_pk);
    assert_eq!(
        spc_crypto::address::encode_sp_address(Network::Mainnet, &scan_pk, &spend_pk).unwrap(),
        addr
    );
}
