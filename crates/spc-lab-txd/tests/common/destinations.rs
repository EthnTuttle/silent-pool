//! Shared offline T1-DEST address corpus for controller, actor and core tests.
#![allow(dead_code)]

use bitcoin::{Address, Network, ScriptBuf};

/// Public onchain_funding_address from the stock Bark wallet provisioned in
/// AP3b-owned-unfunded-20261109T044603Z/run/private/pool-ark-pool-a/wallet/provision.json.
/// Only that public field was consumed. This is an address fixture, not funding evidence.
pub const BARK_FUNDING_ADDRESS: &str =
    "bcrt1pm96lcuz7whnewynm6xkv389pys2v7vjlh88w0dxfdyxka6rd2emqar8un5";

pub fn generated_pair(network: Network) -> [Address; 2] {
    let secp = bitcoin::secp256k1::Secp256k1::new();
    let secret = bitcoin::secp256k1::SecretKey::from_slice(&[7; 32]).unwrap();
    let public = bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &secret);
    [
        Address::p2wpkh(&bitcoin::CompressedPublicKey(public), network),
        Address::p2tr(&secp, public.x_only_public_key().0, None, network),
    ]
}

pub fn admitted_addresses() -> Vec<String> {
    let bark: Address<_> = BARK_FUNDING_ADDRESS.parse().unwrap();
    // Independently decoded bech32m witness program, fixed public bytes.
    assert_eq!(
        bark.require_network(Network::Regtest)
            .unwrap()
            .script_pubkey()
            .to_hex_string(),
        "5120d975fc705e75e797127bd1acc89ca12414cf325fb9cee7b4c9690d6ee86d5676"
    );
    let mut addresses = vec![BARK_FUNDING_ADDRESS.to_string()];
    addresses.extend(generated_pair(Network::Regtest).map(|a| a.to_string()));
    addresses
}

pub fn unsupported_scripts() -> Vec<ScriptBuf> {
    let program = |version, length: usize| {
        let mut bytes = vec![version, length as u8];
        bytes.extend(vec![7; length]);
        ScriptBuf::from_bytes(bytes)
    };
    vec![
        ScriptBuf::from_bytes(vec![0x6a]), // OP_RETURN
        ScriptBuf::from_bytes(vec![0x51]), // nonstandard output
        program(0x51, 31),                 // invalid P2TR length
        program(0x51, 33),
        program(0x52, 32), // future witness version
        program(0x00, 32), // P2WSH deliberately outside this packet
        ScriptBuf::from_bytes(
            hex::decode("a914000000000000000000000000000000000000000087").unwrap(),
        ),
    ]
}

pub fn rejected_addresses() -> Vec<String> {
    let mut addresses = vec!["not-an-address".into(), format!("{BARK_FUNDING_ADDRESS}x")];
    for network in [Network::Bitcoin, Network::Testnet, Network::Signet] {
        addresses.extend(generated_pair(network).map(|a| a.to_string()));
    }
    for script in unsupported_scripts() {
        if let Ok(address) = Address::from_script(&script, Network::Regtest) {
            addresses.push(address.to_string());
        }
    }
    addresses
}
