//! Offline public-proof checker. No keys, wallet, RPC client or signer.
use std::io::{Read, Write};
use std::str::FromStr;

use anyhow::{ensure, Result};
use ark::vtxo::{Full, Vtxo};
use ark::{ProtocolEncoding, VtxoPolicy};
use bitcoin::hashes::{sha256, Hash};
use bitcoin::secp256k1::PublicKey;
use serde::Deserialize;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Expected {
    user_pubkey: String,
    server_pubkey: String,
    amount_sats: u64,
    expiry_height: u32,
    exit_delta: u16,
    max_exit_depth: u16,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    schema: String,
    raw_vtxo_hex: String,
    anchor_tx_hex: String,
    expected: Expected,
}

fn main() -> Result<()> {
    let mut input = Vec::new();
    std::io::stdin()
        .take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut input)?;
    ensure!(
        input.len() <= 8 * 1024 * 1024,
        "proof request exceeds bound"
    );
    let request: Request = serde_json::from_slice(&input)?;
    let result = verify(request)?;
    serde_json::to_writer(std::io::stdout(), &result)?;
    std::io::stdout().write_all(b"\n")?;
    Ok(())
}

fn verify(request: Request) -> Result<serde_json::Value> {
    ensure!(
        request.schema == "spc.lab.ap5.vtxo-proof-request/v1",
        "proof schema"
    );
    let raw = hex::decode(&request.raw_vtxo_hex)?;
    let anchor_raw = hex::decode(&request.anchor_tx_hex)?;
    let vtxo = Vtxo::<Full>::deserialize(&raw)?;
    let anchor: bitcoin::Transaction = bitcoin::consensus::deserialize(&anchor_raw)?;
    ensure!(vtxo.serialize() == raw, "noncanonical VTXO encoding");
    ensure!(
        bitcoin::consensus::serialize(&anchor) == anchor_raw,
        "noncanonical anchor encoding"
    );
    ensure!(
        vtxo.chain_anchor().txid == anchor.compute_txid(),
        "anchor transaction identity mismatch"
    );
    let owner = PublicKey::from_str(&request.expected.user_pubkey)?;
    let server = PublicKey::from_str(&request.expected.server_pubkey)?;
    // Full policy equality, not a check that some policy mentions this key.
    ensure!(
        vtxo.policy() == &VtxoPolicy::new_pubkey(owner),
        "full VTXO policy mismatch"
    );
    ensure!(vtxo.server_pubkey() == server, "VTXO provider mismatch");
    ensure!(
        vtxo.amount().to_sat() == request.expected.amount_sats,
        "VTXO amount mismatch"
    );
    ensure!(
        vtxo.expiry_height().to_u32() == request.expected.expiry_height,
        "VTXO expiry mismatch"
    );
    ensure!(
        vtxo.exit_delta().to_u16() == request.expected.exit_delta,
        "VTXO exit delta mismatch"
    );
    ensure!(
        vtxo.exit_depth() <= request.expected.max_exit_depth,
        "VTXO exit depth exceeds bound"
    );
    // This checks the complete transaction chain INCLUDING signatures.
    // validate_unsigned and mere bark dev decoding are insufficient here.
    vtxo.validate(&anchor)?;
    let result = serde_json::json!({
        "schema": "spc.lab.ap5.vtxo-proof/v1", "verified": true,
        "validator": "stock-ark-lib-full-chain-and-signatures",
        "ark_revision": "c419159fa54085572f8ca72d2ba1716556a9d3eb",
        "raw_sha256": sha256::Hash::hash(&raw).to_string(),
        "anchor_raw_sha256": sha256::Hash::hash(&anchor_raw).to_string(),
        "id": vtxo.id().to_string(), "chain_anchor": vtxo.chain_anchor().to_string(),
        "policy_kind": "pubkey", "user_pubkey": owner.to_string(),
        "server_pubkey": server.to_string(), "amount_sats": vtxo.amount().to_sat(),
        "expiry_height": vtxo.expiry_height().to_u32(),
        "exit_delta": vtxo.exit_delta().to_u16(), "exit_depth": vtxo.exit_depth()
    });
    Ok(result)
}

#[cfg(test)]
mod vectors;
