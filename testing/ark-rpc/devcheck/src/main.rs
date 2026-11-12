//! arkrpc devcheck: validates the Python contract golden vectors
//! against the REAL pinned Rust crate types bark c419159 deserializes
//! with, so the Python mini-language in arkrpc/contracts.py is not
//! merely self-consistent.
//!
//! Every vector below mirrors a test in
//! testing/ark-rpc/tests/test_contracts.py marked with a `#devcheck`
//! comment (same name after `v_`). If you change a vector here, change
//! the Python twin there, and vice versa.
//!
//! Crates under test (all pinned to bark c419159's Cargo.lock):
//! - bitcoincore-rpc-json 0.19.0: sync-client result types;
//!   ScriptPubkeyType WITHOUT Anchor (the gettxout consumer).
//! - corepc-types 0.13.0: v29 getblockchaininfo (async shape;
//!   bits/target/time required).
//! - bitcoin 0.32.9 / bitcoin-units 0.1.3: Amount + as_btc serde
//!   (u64 satoshi range, not the 21M policy cap).
//! - bark-bitcoin-ext 0.7.1 (pinned checkout, rpc feature):
//!   ScriptPubkeyType WITH Anchor, serde_hex, deserialize_hex_array_opt
//!   (the getrawtransaction-verbose consumer; see bark/src/chain.rs and
//!   server/src/bitcoind.rs).
//!
//! Dev-only: NOT part of the probe runtime, NOT part of the packet
//! manifest hash. Run with devcheck/run.sh (or `cargo run --offline`).

use bitcoincore_rpc_json::{
    GetBlockchainInfoResult, GetBlockHeaderResult, GetTxOutResult,
    GetRawTransactionResultVinScriptSig as V019ScriptSig,
    GetRawTransactionResultVoutScriptPubKey as V019Spk,
};
use bitcoin_ext::rpc as bext;
use corepc_types::{v24, v29, v30};
use serde::Deserialize;
use serde_json::{json, Value};

/// Mirror of bitcoin::amount::serde::as_btc for direct Amount vectors.
#[derive(Deserialize)]
struct BtcAmount(#[serde(with = "bitcoin::amount::serde::as_btc")] bitcoin::Amount);

const GENESIS_HASH: &str = "0f9188f13cb7b2c71f2a335e3a4fc328bf5beb436012afca590b1a11466e2206";
// D7: corrected against the r1 evidence capture (reports/2026-11-08-
// r1-*/raw_responses.json getblock verbosity 1 merkleroot and tx[0]);
// the previous constant had a typo (...618f76773e... vs the real
// ...618f76673e...).
const GENESIS_MERKLE: &str = "4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b";

fn de<T: serde::de::DeserializeOwned>(v: Value) -> Result<T, String> {
    serde_json::from_value(v).map_err(|e| format!("serde error: {}", e))
}

fn err_of<T: serde::de::DeserializeOwned>(v: Value) -> String {
    match de::<T>(v) {
        Ok(_) => "unexpectedly parsed".to_string(),
        Err(e) => e,
    }
}

// ---------------------------------------------------------------------------
// Amount: bitcoin::amount::serde::as_btc (bitcoin-units 0.1.3)
// ---------------------------------------------------------------------------

/// Python: AmountBtcTests::test_over_supply_accepted
/// 30M BTC is 3e15 sats, inside the u64 range: the 21M supply cap is
/// consensus policy (MoneyRange), never a serde bound.
fn v_amount_over_supply_accepted() -> Result<(), String> {
    let a: BtcAmount = de(json!(30000000.0))?;
    if a.0.to_sat() != 3_000_000_000_000_000 {
        return Err(format!("30M BTC -> {} sats", a.0.to_sat()));
    }
    Ok(())
}

/// Python: AmountBtcTests::test_u64_overflow_rejected
/// Amount::MAX is u64::MAX sats = 184467440737.09551615 BTC. 184467440737
/// BTC parses; one BTC more overflows and must be rejected. (as_btc
/// deserializes via f64 and from_float_in re-parses value.to_string(),
/// so these decimal boundaries survive the f64 round trip exactly.)
fn v_amount_u64_overflow_rejected() -> Result<(), String> {
    let ok: BtcAmount = de(json!(184467440737.0))?;
    if ok.0.to_sat() != 18_446_744_073_700_000_000 {
        return Err(format!("boundary -> {} sats", ok.0.to_sat()));
    }
    let e = err_of::<BtcAmount>(json!(184467440738.0));
    if !e.contains("range") && !e.to_lowercase().contains("too big") {
        return Err(format!("expected out-of-range, got: {}", e));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// serde_hex: empty string is a valid empty Vec<u8>
// ---------------------------------------------------------------------------

/// Python: HashAndHexTests::test_empty_hex_valid_where_serde_hex
/// (scriptSig twin: ScriptSigTests below). serde_hex deserializes via
/// FromHex::from_hex, which accepts "" as an empty Vec. Both the 0.19
/// module and bark's vendored copy share this helper shape.
fn v_empty_hex_serde_hex() -> Result<(), String> {
    let a: V019ScriptSig = de(json!({"asm": "", "hex": ""}))?;
    if !a.hex.is_empty() {
        return Err("0.19 scriptSig.hex not empty".to_string());
    }
    let b: bext::GetRawTransactionResultVinScriptSig = de(json!({"asm": "", "hex": ""}))?;
    if !b.hex.is_empty() {
        return Err("bitcoin-ext scriptSig.hex not empty".to_string());
    }
    Ok(())
}

/// Python: NullRejectTests::test_scriptsig_empty_hex_valid
fn v_scriptsig_empty_hex() -> Result<(), String> {
    let b: bext::GetRawTransactionResultVinScriptSig =
        de(json!({"asm": "", "hex": ""}))?;
    if !b.hex.is_empty() {
        return Err("hex not empty".to_string());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// The two ScriptPubkeyType enums (the load-bearing F4 split)
// ---------------------------------------------------------------------------

/// Python: ScriptPubkeyTypeTests::test_bark_enum_accepts_anchor
/// bark's vendored ScriptPubkeyType HAS an Anchor variant, and the vout
/// Amount uses as_btc (u64 range), so the anchor_verbose_tx scenario
/// vout (anchor type + 30M BTC) parses wholesale.
fn v_bark_enum_accepts_anchor() -> Result<(), String> {
    let spk: bext::GetRawTransactionResultVoutScriptPubKey =
        de(json!({"asm": "", "hex": "51024c53", "type": "anchor"}))?;
    match spk.type_ {
        Some(bext::ScriptPubkeyType::Anchor) => {}
        other => return Err(format!("wrong type: {:?}", other)),
    }
    let vout: bext::GetRawTransactionResultVout = de(json!({
        "value": 30000000.0, "n": 0,
        "scriptPubKey": {"asm": "", "hex": "51024c53", "type": "anchor"}
    }))?;
    if vout.value.to_sat() != 3_000_000_000_000_000 {
        return Err(format!("vout value -> {} sats", vout.value.to_sat()));
    }
    Ok(())
}

/// Python: ScriptPubkeyTypeTests::test_core_019_enum_rejects_anchor
/// bitcoincore-rpc-json 0.19's ScriptPubkeyType has NO Anchor variant:
/// an anchor output breaks the 0.19 gettxout consumer wholesale.
fn v_core_019_enum_rejects_anchor() -> Result<(), String> {
    let e = err_of::<V019Spk>(json!({"asm": "", "hex": "51024c53", "type": "anchor"}));
    if !e.contains("anchor") {
        return Err(format!("expected unknown-variant 'anchor', got: {}", e));
    }
    // The full GetTxOutResult path (the gettxout_anchor_rejected
    // scenario fixture):
    let e2 = err_of::<GetTxOutResult>(json!({
        "bestblock": GENESIS_HASH, "confirmations": 1, "value": 1.0,
        "scriptPubKey": {"asm": "", "hex": "51024c53", "type": "anchor"},
        "coinbase": false
    }));
    if !e2.contains("anchor") {
        return Err(format!("expected GetTxOutResult anchor failure, got: {}", e2));
    }
    // The same shape WITHOUT anchor parses fine:
    de::<GetTxOutResult>(json!({
        "bestblock": GENESIS_HASH, "confirmations": 1, "value": 1.0,
        "scriptPubKey": {"asm": "", "hex": "51024c53", "type": "pubkeyhash"},
        "coinbase": false
    }))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// txinwitness: deserialize_hex_array_opt accepts empty-string items
// ---------------------------------------------------------------------------

/// Python: NullRejectTests::test_txinwitness_empty_items_valid
fn v_txinwitness_empty_items() -> Result<(), String> {
    let vin: bext::GetRawTransactionResultVin =
        de(json!({"sequence": 0, "txinwitness": ["", "00"]}))?;
    match vin.txinwitness {
        Some(w) if w == vec![Vec::<u8>::new(), vec![0u8]] => Ok(()),
        other => Err(format!("txinwitness -> {:?}", other)),
    }
}

// ---------------------------------------------------------------------------
// Absent-vs-null: custom Option deserializers reject explicit null
// ---------------------------------------------------------------------------

/// Python: NullRejectTests::test_vin_coinbase_absent_ok_null_rejected
/// and test_txinwitness_absent_ok_null_rejected.
/// serde(default) + serde_hex::opt / deserialize_hex_array_opt call
/// the inner deserializer directly, so null is a hard error while
/// absence is fine.
fn v_null_reject_vin() -> Result<(), String> {
    let plain: bext::GetRawTransactionResultVin = de(json!({"sequence": 0}))?;
    if plain.coinbase.is_some() || plain.txinwitness.is_some() {
        return Err("absent fields should be None".to_string());
    }
    let e1 = err_of::<bext::GetRawTransactionResultVin>(
        json!({"sequence": 0, "coinbase": null}));
    if e1 == "unexpectedly parsed" {
        return Err("coinbase: null unexpectedly parsed".to_string());
    }
    let e2 = err_of::<bext::GetRawTransactionResultVin>(
        json!({"sequence": 0, "txinwitness": null}));
    if e2 == "unexpectedly parsed" {
        return Err("txinwitness: null unexpectedly parsed".to_string());
    }
    Ok(())
}

/// Python: NullRejectTests::test_versionhex_absent_ok_null_rejected
fn v_null_reject_versionhex() -> Result<(), String> {
    let base = json!({
        "hash": GENESIS_HASH, "confirmations": 1, "height": 0,
        "version": 1, "merkleroot": GENESIS_MERKLE,
        "time": 1231006505, "mediantime": 1231006505, "nonce": 2,
        "bits": "207fffff", "difficulty": 4.656542373906925e-05,
        "chainwork": "00".repeat(32), "nTx": 1
    });
    de::<GetBlockHeaderResult>(base.clone())?;
    let mut with_null = base.clone();
    with_null["versionHex"] = Value::Null;
    if err_of::<GetBlockHeaderResult>(with_null) == "unexpectedly parsed" {
        return Err("versionHex: null unexpectedly parsed".to_string());
    }
    Ok(())
}

/// Python: NullRejectTests::test_spk_addresses_null_rejected
/// (addresses is Vec<Address> with serde(default): null is not a
/// sequence, so it is a hard error while absence defaults to empty.)
fn v_null_reject_addresses() -> Result<(), String> {
    de::<bext::GetRawTransactionResultVoutScriptPubKey>(
        json!({"asm": "", "hex": "00"}))?;
    if err_of::<bext::GetRawTransactionResultVoutScriptPubKey>(
        json!({"asm": "", "hex": "00", "addresses": null})) == "unexpectedly parsed"
    {
        return Err("addresses: null unexpectedly parsed".to_string());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// getblockchaininfo: 0.19 sync shape vs corepc v29 async shape
// ---------------------------------------------------------------------------

fn blockchaininfo_full() -> Value {
    json!({
        "chain": "regtest", "blocks": 0, "headers": 0,
        "bestblockhash": GENESIS_HASH, "bits": "207fffff",
        "target": "7f".repeat(32), "difficulty": 4.656542373906925e-05,
        "time": 1296688602, "mediantime": 1296688602,
        "verificationprogress": 1.0, "initialblockdownload": false,
        "chainwork": "00".repeat(32), "size_on_disk": 296,
        "pruned": false, "warnings": []
    })
}

/// Python: BlockchainInfoShapeTests::test_async_requires_bits and
/// test_sync_ok_without_bits. The corepc v29 async shape REQUIRES
/// bits/target/time; the 0.19 sync shape has none of those fields and
/// still parses without them (and accepts a plain-string warnings).
fn v_blockchaininfo_shapes() -> Result<(), String> {
    de::<v29::GetBlockchainInfo>(blockchaininfo_full())?;
    let mut no_bits = blockchaininfo_full();
    no_bits["bits"].take();
    no_bits["target"].take();
    no_bits["time"].take();
    if err_of::<v29::GetBlockchainInfo>(no_bits) == "unexpectedly parsed" {
        return Err("corepc v29 parsed without bits/target/time".to_string());
    }
    // 0.19 sync shape: no bits/target/time, and string warnings are ok.
    let mut sync = blockchaininfo_full();
    sync["bits"].take();
    sync["target"].take();
    sync["time"].take();
    sync["warnings"] = json!("warning string");
    de::<GetBlockchainInfoResult>(sync)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// getmempoolinfo: corepc v30 shape (the traffic-slice contract)
// ---------------------------------------------------------------------------

fn getmempoolinfo_v30_full() -> Value {
    json!({
        "loaded": true, "size": 0, "bytes": 0, "usage": 0,
        "total_fee": 0.0, "maxmempool": 75000000,
        "mempoolminfee": 0.000001, "minrelaytxfee": 0.000001,
        "incrementalrelayfee": 0.000001, "unbroadcastcount": 0,
        "fullrbf": true, "permitbaremultisig": true,
        "maxdatacarriersize": 165,
        // rbitcoin extras observed in the r1 capture: unknown fields
        // must be ignored (corepc-types does not build with
        // serde-deny-unknown-fields).
        "limitclustercount": 64, "limitclustersize": 101000,
        "optimal": true, "orphanage": {"bytes": 0, "size": 0},
        "relay_enabled": false
    })
}

/// Python: GetMempoolInfoTests (test_ok, test_unknown_fields_ignored,
/// test_null_maxdatacarriersize_rejected,
/// test_v24_era_response_rejected). The pinned async client compiles
/// the v30 module ONLY (default features 30_2 + raw_rpc; the v29/v30
/// client modules are mutually exclusive), so v30::GetMempoolInfo is
/// THE deserializer for getmempoolinfo in bark's tree: all 13 fields
/// required, unknown fields ignored, and the exact rbitcoin r1
/// observation -- maxdatacarriersize JSON null where Core emits an
/// integer -- is a hard serde error (plain u64, no Option, no
/// default). The v24 shape (the v29 client module's) accepts the
/// v24-era response v30 rejects, proving the two differ.
fn v_getmempoolinfo_shapes() -> Result<(), String> {
    de::<v30::GetMempoolInfo>(getmempoolinfo_v30_full())?;
    // The real rbitcoin r1 capture: maxdatacarriersize is null.
    let mut null_mdc = getmempoolinfo_v30_full();
    null_mdc["maxdatacarriersize"] = Value::Null;
    if err_of::<v30::GetMempoolInfo>(null_mdc) == "unexpectedly parsed" {
        return Err("maxdatacarriersize null unexpectedly parsed".to_string());
    }
    // v24-era response (no permitbaremultisig/maxdatacarriersize):
    // v30 rejects it...
    let mut v24_era = getmempoolinfo_v30_full();
    v24_era["permitbaremultisig"].take();
    v24_era["maxdatacarriersize"].take();
    if err_of::<v30::GetMempoolInfo>(v24_era.clone()) == "unexpectedly parsed" {
        return Err("v24-era response unexpectedly parsed by v30".to_string());
    }
    // ...while v24::GetMempoolInfo (the v29 client module's shape)
    // accepts it: the version split is real, and bark's tree uses v30.
    de::<v24::GetMempoolInfo>(v24_era)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Runner
// ---------------------------------------------------------------------------

type Vector = (&'static str, fn() -> Result<(), String>);

const VECTORS: &[Vector] = &[
    ("amount_over_supply_accepted", v_amount_over_supply_accepted),
    ("amount_u64_overflow_rejected", v_amount_u64_overflow_rejected),
    ("empty_hex_serde_hex", v_empty_hex_serde_hex),
    ("scriptsig_empty_hex", v_scriptsig_empty_hex),
    ("bark_enum_accepts_anchor", v_bark_enum_accepts_anchor),
    ("core_019_enum_rejects_anchor", v_core_019_enum_rejects_anchor),
    ("txinwitness_empty_items", v_txinwitness_empty_items),
    ("null_reject_vin", v_null_reject_vin),
    ("null_reject_versionhex", v_null_reject_versionhex),
    ("null_reject_addresses", v_null_reject_addresses),
    ("blockchaininfo_shapes", v_blockchaininfo_shapes),
    ("getmempoolinfo_shapes", v_getmempoolinfo_shapes),
];

fn run_all() -> i32 {
    println!("arkrpc-devcheck: {} golden vectors vs pinned crate types",
             VECTORS.len());
    let mut failed = 0;
    for (name, f) in VECTORS {
        match f() {
            Ok(()) => println!("[PASS] {}", name),
            Err(e) => {
                println!("[FAIL] {}: {}", name, e);
                failed += 1;
            }
        }
    }
    if failed == 0 {
        println!("all {} vectors match the Python #devcheck twins",
                 VECTORS.len());
        0
    } else {
        println!("{} of {} vectors FAILED", failed, VECTORS.len());
        1
    }
}

fn main() {
    std::process::exit(run_all());
}

#[cfg(test)]
mod tests {
    #[test]
    fn all_vectors() {
        for (name, f) in super::VECTORS {
            if let Err(e) = f() {
                panic!("vector {}: {}", name, e);
            }
        }
    }
}
