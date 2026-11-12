//! Legacy P2PKH transaction construction and signing (traffic plan sections
//! 6.3 and 7).
//!
//! Properties implemented here, all with checked integer arithmetic:
//!
//! - **Legacy only**: every prevout script must be P2PKH; any other spend
//!   type is rejected (the witness gate is a separate pool packet, T4).
//! - **Exact destinations**: each manual destination output value equals the
//!   requested amount exactly, and change goes only to a script owned by the
//!   spending wallet.
//! - **Final signed size**: weight and `vbytes = ceil(weight/4)` come from
//!   the final signed transaction, never a nominal pre-signature size.
//!   The fee is set from an upper-bound estimate of the signed size, the
//!   transaction is signed **once** (RFC 6979 deterministic, so the same
//!   inputs always produce the same bytes), and the rational
//!   `fee_sats / vbytes` is compared against the sats/vB band using the
//!   **measured** final vbytes. If (and only if) that comparison fails,
//!   the fee is re-targeted to `rate * measured_vbytes` and the bounded
//!   loop re-signs; a loop that cannot produce an in-band fee is a clear
//!   construction failure, never a guess.
//! - **Exact rational fee bounds**: the final `fee_sats / vbytes` is
//!   compared against the sats/vB band with checked `u128` products.
//! - **Dust and fee ceilings are explicit**; a change output below the dust
//!   limit rejects construction instead of being silently absorbed.
//! - **Conservation**: `sum(inputs) = sum(outputs) + fee` is asserted on the
//!   final signed transaction.

use bitcoin::absolute::LockTime;
use bitcoin::consensus::encode::serialize as consensus_serialize;
use bitcoin::hashes::sha256d::Hash as Sha256d;
use bitcoin::hashes::Hash as _;
use bitcoin::secp256k1::{Message, Secp256k1, SecretKey};
use bitcoin::sighash::SighashCache;
use bitcoin::transaction::Version;
use bitcoin::{OutPoint, PublicKey, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Txid, Witness};
use serde::{Deserialize, Serialize};

use crate::error::BuilderError;
use crate::error::MoneyError;
use crate::keys::{Chain, TrafficWallet};
use crate::money::{checked_add_sats, checked_sub_sats, fee_within_band};

/// The maximum signature-witness size a P2PKH input can add when signed:
/// varint + (DER <= 72 + 1 sighash byte) + varint + 33-byte pubkey.
const MAX_P2PKH_SCRIPT_SIG_OVERHEAD: u64 = 1 + 73 + 1 + 33;

/// The bound on the deterministic sign-and-remeasure convergence loop.
pub const MAX_SIGN_ITERATIONS: u32 = 8;

/// The single sighash type used by the v1 traffic profile: `SIGHASH_ALL`.
const SIGHASH_ALL: u32 = 0x01;

/// Fee and dust policy for traffic construction (domain arguments; the T0
/// `TrafficSpec` maps onto this, see `adapter.rs`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeePolicy {
    /// Minimum admissible fee rate in sats/vB.
    pub min_rate_sats_vb: u64,
    /// Maximum admissible fee rate in sats/vB.
    pub max_rate_sats_vb: u64,
    /// Hard per-transaction fee ceiling in sats.
    pub max_fee_sats_per_tx: u64,
    /// Outputs strictly below this value are dust and not admissible; an
    /// output at exactly this value is admissible (the checks in
    /// `build_and_sign` are strict inequalities).
    pub dust_sats: u64,
}

impl FeePolicy {
    /// Validate the policy itself (fail closed on an unusable band).
    pub fn validate(&self) -> Result<(), BuilderError> {
        let invalid = |reason: &str| Err(BuilderError::InvalidFeePolicy(reason.to_string()));
        if self.min_rate_sats_vb == 0 {
            return invalid("min_rate_sats_vb must be nonzero (positive fees)");
        }
        if self.max_rate_sats_vb < self.min_rate_sats_vb {
            return invalid("max_rate_sats_vb must be >= min_rate_sats_vb");
        }
        if self.max_fee_sats_per_tx == 0 {
            return invalid("max_fee_sats_per_tx must be nonzero");
        }
        if self.dust_sats == 0 {
            return invalid("dust_sats must be nonzero");
        }
        Ok(())
    }
}

impl Default for FeePolicy {
    fn default() -> Self {
        Self {
            min_rate_sats_vb: 1,
            max_rate_sats_vb: 20,
            max_fee_sats_per_tx: 100_000,
            dust_sats: 546,
        }
    }
}

/// Traffic payment category. The `ark_*` categories are declared by the Ark
/// agent's interface (plan section 9.4) and carried as an opaque label.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum PaymentCategory {
    Ordinary,
    UserPayment,
    BootstrapDistribution,
    ArkEffect { label: String },
}

impl PaymentCategory {
    /// The canonical journal string for this category.
    pub fn as_str(&self) -> String {
        match self {
            Self::Ordinary => "ordinary".to_string(),
            Self::UserPayment => "user_payment".to_string(),
            Self::BootstrapDistribution => "bootstrap_distribution".to_string(),
            Self::ArkEffect { label } => format!("ark_{label}"),
        }
    }

    /// Parse a canonical journal category string, failing closed on
    /// anything else.
    pub fn parse(value: &str) -> Result<Self, BuilderError> {
        match value {
            "ordinary" => Ok(Self::Ordinary),
            "user_payment" => Ok(Self::UserPayment),
            "bootstrap_distribution" => Ok(Self::BootstrapDistribution),
            other => {
                if let Some(label) = other.strip_prefix("ark_") {
                    if label.is_empty()
                        || !label
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                    {
                        return Err(BuilderError::Signing(format!(
                            "invalid ark category label: {other:?}"
                        )));
                    }
                    Ok(Self::ArkEffect {
                        label: label.to_string(),
                    })
                } else {
                    Err(BuilderError::Signing(format!(
                        "unknown payment category: {other:?}"
                    )))
                }
            }
        }
    }
}

/// One payment destination: an exact amount to an exact script.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DestinationSpec {
    pub script_pubkey: ScriptBuf,
    pub amount_sats: u64,
    /// The owning wallet when the destination is another traffic wallet
    /// (creditable on confirmation); `None` for external recipients.
    pub owner: Option<crate::keys::WalletRole>,
}

/// T1-DEST output policy. This grants no input-signing or wallet ownership
/// authority: owned receive/change scripts and all prevouts remain P2PKH.
/// Exact standard templates exclude future witness versions and bad lengths.
pub fn supported_destination_script(script: &bitcoin::Script) -> bool {
    script.is_p2pkh() || script.is_p2wpkh() || script.is_p2tr()
}

/// Validate output class and refuse witness outputs falsely tagged as owned.
pub(crate) fn validate_destination_script(
    destination: &DestinationSpec,
) -> Result<(), BuilderError> {
    if !supported_destination_script(&destination.script_pubkey) {
        return Err(BuilderError::UnsupportedDestination {
            script_hex: destination.script_pubkey.to_hex_string(),
        });
    }
    if destination.owner.is_some() && !destination.script_pubkey.is_p2pkh() {
        return Err(BuilderError::WitnessDestinationNotOwned);
    }
    Ok(())
}

/// A UTXO selected for spending, with its owning key resolved.
#[derive(Clone, Debug)]
pub struct SignableUtxo {
    pub outpoint: OutPoint,
    pub amount_sats: u64,
    pub script_pubkey: ScriptBuf,
    pub secret_key: SecretKey,
    pub public_key: PublicKey,
}

impl SignableUtxo {
    /// Resolve an owned outpoint for spending by **direct derivation** at
    /// its recorded `(chain, index)` (M2): the wallet derives the script
    /// and key at exactly that index, and the derived script must equal
    /// the recorded `expected_script` byte-for-byte - a foreign script, or
    /// a script that has drifted from the recorded index, fails closed
    /// here, before any signature exists. No chain scan is involved, so
    /// the spending path never depends on the lookahead bound (the scan
    /// in `key_for_script` serves destination tagging only).
    pub fn resolve_indexed(
        wallet: &TrafficWallet,
        outpoint: OutPoint,
        chain: Chain,
        index: u32,
        expected_script: &ScriptBuf,
        amount_sats: u64,
    ) -> Result<Self, BuilderError> {
        if !expected_script.is_p2pkh() {
            return Err(BuilderError::NotP2pkhPrevout {
                script_hex: expected_script.to_hex_string(),
            });
        }
        let derived = wallet
            .script_at(chain, index)
            .map_err(|e| BuilderError::Signing(format!("derive input script: {e}")))?;
        if &derived.script_pubkey != expected_script {
            return Err(BuilderError::Signing(format!(
                "outpoint {outpoint}: recorded script {} does not equal this wallet's \
                 derived script at {chain:?}/{index}",
                expected_script.to_hex_string()
            )));
        }
        let key = wallet
            .resolved_key_at(chain, index)
            .map_err(|e| BuilderError::Signing(format!("resolve input key: {e}")))?;
        Ok(Self {
            outpoint,
            amount_sats,
            script_pubkey: expected_script.clone(),
            secret_key: key.secret_key,
            public_key: key.public_key,
        })
    }
}

/// Deterministic eligible-UTXO ordering: descending amount, then ascending
/// outpoint (txid, then vout). Wallet, fee and size-class selection is
/// deterministic per the plan (section 6.3): the same ledger state always
/// yields the same selection.
pub fn deterministic_utxo_order(mut utxos: Vec<SignableUtxo>) -> Vec<SignableUtxo> {
    utxos.sort_by(|a, b| {
        b.amount_sats
            .cmp(&a.amount_sats)
            .then(a.outpoint.txid.cmp(&b.outpoint.txid))
            .then(a.outpoint.vout.cmp(&b.outpoint.vout))
    });
    utxos
}

/// A fully signed traffic transaction with its measured final properties.
#[derive(Clone, Debug)]
pub struct SignedTrafficTx {
    pub tx: Transaction,
    pub txid: Txid,
    /// Exact consensus serialization of the signed transaction.
    pub raw_bytes: Vec<u8>,
    pub weight_wu: u64,
    pub vbytes: u64,
    pub fee_sats: u64,
    pub spent_outpoints: Vec<OutPoint>,
    pub destinations: Vec<DestinationSpec>,
    pub change: Option<ChangeInfo>,
    pub category: PaymentCategory,
}

/// Where the change output went (always an owned wallet script).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangeInfo {
    pub script_pubkey: ScriptBuf,
    pub amount_sats: u64,
    pub chain: Chain,
    pub index: u32,
}

/// Upper bound on the signed vbytes for a given unsigned transaction body
/// and input count. Legacy transactions have `vbytes == size`.
fn signed_vbytes_upper_bound(unsigned: &Transaction, inputs: usize) -> u64 {
    let base = consensus_serialize(unsigned).len() as u64;
    base + inputs as u64 * MAX_P2PKH_SCRIPT_SIG_OVERHEAD
}

/// Upper bound on the total fee (at `fee_rate_sats_vb`) of a transaction
/// spending `input_count` P2PKH inputs to `destinations` plus (optionally)
/// one P2PKH change output.
///
/// Selection uses this bound **before** reserving: any input set whose
/// total satisfies `destinations_total + bound + dust` is guaranteed to
/// admit a valid signed build, so the reservation-to-signing step never
/// has to be retried with different inputs (reservations happen exactly
/// once per command, before any signature exists).
pub fn selection_fee_upper_bound(
    destinations: &[DestinationSpec],
    fee_rate_sats_vb: u64,
    input_count: usize,
    include_change: bool,
) -> u64 {
    let dummy_outpoint = OutPoint::new(Txid::from_byte_array([0u8; 32]), 0);
    let input: Vec<TxIn> = (0..input_count)
        .map(|_| TxIn {
            previous_output: dummy_outpoint,
            script_sig: ScriptBuf::new(),
            sequence: Sequence::MAX,
            witness: Witness::default(),
        })
        .collect();
    let mut output: Vec<TxOut> = destinations
        .iter()
        .map(|destination| TxOut {
            value: bitcoin::Amount::from_sat(destination.amount_sats),
            script_pubkey: destination.script_pubkey.clone(),
        })
        .collect();
    if include_change {
        // A P2PKH change output is 34 bytes regardless of key.
        let dummy_change = bitcoin::Address::p2pkh(
            bitcoin::PubkeyHash::from_byte_array([0u8; 20]),
            bitcoin::Network::Regtest,
        );
        output.push(TxOut {
            value: bitcoin::Amount::from_sat(0),
            script_pubkey: dummy_change.script_pubkey(),
        });
    }
    let unsigned = Transaction {
        version: Version(2),
        lock_time: LockTime::ZERO,
        input,
        output,
    };
    fee_rate_sats_vb * signed_vbytes_upper_bound(&unsigned, input_count)
}

/// Build and sign one legacy P2PKH transaction.
///
/// The fee starts from an upper-bound estimate of the signed size, the
/// transaction is signed once, and the fee band is checked against the
/// **measured** final `vbytes` with exact integer products. Only an
/// out-of-band result re-targets the fee (`rate * measured vbytes`) and
/// re-signs, under the bounded loop below; since the estimate is an upper
/// bound, the fee is monotonically non-increasing across re-targets, so
/// the dust and sufficiency checks cannot be invalidated by a later
/// iteration. RFC 6979 makes every step deterministic: the same inputs
/// always produce the same signed bytes.
#[allow(clippy::too_many_arguments)]
pub fn build_and_sign(
    secp: &Secp256k1<bitcoin::secp256k1::All>,
    wallet: &TrafficWallet,
    utxos: &[SignableUtxo],
    destinations: &[DestinationSpec],
    fee_rate_sats_vb: u64,
    policy: &FeePolicy,
    change_index: u32,
    category: PaymentCategory,
) -> Result<SignedTrafficTx, BuilderError> {
    build_and_sign_with_construction(
        secp,
        wallet,
        utxos,
        destinations,
        fee_rate_sats_vb,
        policy,
        change_index,
        category,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_and_sign_with_construction(
    secp: &Secp256k1<bitcoin::secp256k1::All>,
    wallet: &TrafficWallet,
    utxos: &[SignableUtxo],
    destinations: &[DestinationSpec],
    fee_rate_sats_vb: u64,
    policy: &FeePolicy,
    change_index: u32,
    category: PaymentCategory,
    effect: Option<crate::construction::ConstructionEffect>,
) -> Result<SignedTrafficTx, BuilderError> {
    let version = crate::construction::ConstructionEffect::version(effect);
    policy.validate()?;
    if fee_rate_sats_vb < policy.min_rate_sats_vb || fee_rate_sats_vb > policy.max_rate_sats_vb {
        return Err(BuilderError::FeeRateOutsidePolicy {
            rate: fee_rate_sats_vb,
            min_rate: policy.min_rate_sats_vb,
            max_rate: policy.max_rate_sats_vb,
        });
    }
    if utxos.is_empty() {
        return Err(BuilderError::InsufficientFunds {
            needed: 0,
            available: 0,
        });
    }
    for utxo in utxos {
        if !utxo.script_pubkey.is_p2pkh() {
            return Err(BuilderError::NotP2pkhPrevout {
                script_hex: utxo.script_pubkey.to_hex_string(),
            });
        }
    }
    // Defense in depth: re-assert the bounded OUTPUT policy independently
    // of wallet/address admission. The input guard above remains P2PKH-only.
    for destination in destinations {
        validate_destination_script(destination)?;
    }

    let inputs_total: u64 = crate::money::checked_sum_sats(utxos.iter().map(|u| u.amount_sats))
        .map_err(|_| BuilderError::InsufficientFunds {
            needed: 0,
            available: u64::MAX,
        })?;
    let destinations_total: u64 = crate::money::checked_sum_sats(
        destinations.iter().map(|d| d.amount_sats),
    )
    .map_err(|_| BuilderError::InsufficientFunds {
        needed: u64::MAX,
        available: inputs_total,
    })?;

    let change_script = wallet
        .script_at(Chain::Change, change_index)
        .map_err(|e| BuilderError::Signing(format!("derive change script: {e}")))?;
    let change_script_pubkey = change_script.script_pubkey;

    // Exact destination match, known-script-only change, and dust are all
    // checked before any signing work below; a destination that equals the
    // change script would make output classification ambiguous, so it is
    // rejected outright (fail closed, no reclassification).
    for destination in destinations {
        if destination.amount_sats < policy.dust_sats {
            return Err(BuilderError::DustDestination {
                amount: destination.amount_sats,
                dust: policy.dust_sats,
            });
        }
        if destination.script_pubkey == change_script_pubkey {
            return Err(BuilderError::AmbiguousChangeScript);
        }
    }

    // Initial fee from an upper-bound size so the first build is safe; the
    // band is then checked against the measured final signed size (never a
    // nominal pre-signature size), and only an out-of-band fee re-targets.
    // The probe includes the change output (the common shape), so the
    // estimate dominates the actual signed size and the realized rate is
    // never below the requested rate.
    let probe = assemble_transaction(
        utxos,
        destinations,
        Some((change_script_pubkey.clone(), 0)),
        version,
    );
    let mut fee_sats = fee_rate_sats_vb
        .checked_mul(signed_vbytes_upper_bound(&probe, utxos.len()))
        .ok_or_else(|| BuilderError::Signing("fee overflow".into()))?;

    let mut converged_signed: Option<(Transaction, u64)> = None;
    for _attempt in 0..MAX_SIGN_ITERATIONS {
        let needed = checked_add_sats(destinations_total, fee_sats).map_err(|_| {
            BuilderError::InsufficientFunds {
                needed: u64::MAX,
                available: inputs_total,
            }
        })?;
        let remainder = checked_sub_sats(inputs_total, needed).map_err(|_| {
            BuilderError::InsufficientFunds {
                needed,
                available: inputs_total,
            }
        })?;
        if remainder > 0 && remainder < policy.dust_sats {
            return Err(BuilderError::DustChange {
                change: remainder,
                dust: policy.dust_sats,
            });
        }
        let change_output = if remainder == 0 {
            None
        } else {
            Some((change_script_pubkey.clone(), remainder))
        };
        let unsigned = assemble_transaction(utxos, destinations, change_output, version);
        let signed = sign_legacy_p2pkh(secp, unsigned, utxos)?;
        let weight = signed.weight().to_wu();
        let vbytes = signed.vsize() as u64;
        // The band comparison uses the measured final size. Passing here
        // accepts the fee as built (change absorbs it exactly); failing
        // re-targets the fee once and re-signs, deterministically.
        let band = fee_within_band(
            fee_sats,
            vbytes,
            policy.min_rate_sats_vb,
            policy.max_rate_sats_vb,
        );
        if band.is_ok() {
            converged_signed = Some((signed, weight));
            break;
        }
        let target_fee = fee_rate_sats_vb
            .checked_mul(vbytes)
            .ok_or_else(|| BuilderError::Signing("fee overflow".into()))?;
        if target_fee >= fee_sats {
            // The fee cannot decrease further (the estimate is an upper
            // bound). Classify by the true band direction (L2): a fee
            // *below* the band (underflow) is a FeeBelowBand failure,
            // and an above-band fee at the target rate is a FeeAboveBand
            // construction failure.
            return Err(match band.unwrap_err() {
                MoneyError::Underflow => BuilderError::FeeBelowBand {
                    fee: fee_sats,
                    min_rate: policy.min_rate_sats_vb,
                    vbytes,
                },
                _ => BuilderError::FeeAboveBand {
                    fee: fee_sats,
                    max_rate: policy.max_rate_sats_vb,
                    vbytes,
                },
            });
        }
        fee_sats = target_fee;
    }
    let (signed, weight_wu) = converged_signed.ok_or(BuilderError::NotConverged {
        attempts: MAX_SIGN_ITERATIONS,
    })?;
    let vbytes = signed.vsize() as u64;

    // Recompute the exact change from the final transaction and confirm the
    // fee actually landed on the converged value.
    let outputs_total: u64 =
        crate::money::checked_sum_sats(signed.output.iter().map(|o| o.value.to_sat()))
            .map_err(|e| BuilderError::Signing(format!("output sum: {e}")))?;
    let final_fee = checked_sub_sats(inputs_total, outputs_total).map_err(|_| {
        BuilderError::ConservationViolation {
            inputs: inputs_total,
            outputs: outputs_total,
            fee: 0,
        }
    })?;
    if final_fee != fee_sats {
        return Err(BuilderError::NotConverged {
            attempts: MAX_SIGN_ITERATIONS,
        });
    }
    if fee_sats == 0 {
        return Err(BuilderError::FeeBelowBand {
            fee: 0,
            min_rate: policy.min_rate_sats_vb,
            vbytes,
        });
    }
    fee_within_band(
        fee_sats,
        vbytes,
        policy.min_rate_sats_vb,
        policy.max_rate_sats_vb,
    )
    .map_err(|e| match e {
        MoneyError::Underflow => BuilderError::FeeBelowBand {
            fee: fee_sats,
            min_rate: policy.min_rate_sats_vb,
            vbytes,
        },
        _ => BuilderError::FeeAboveBand {
            fee: fee_sats,
            max_rate: policy.max_rate_sats_vb,
            vbytes,
        },
    })?;
    if fee_sats > policy.max_fee_sats_per_tx {
        return Err(BuilderError::FeeCeilingExceeded {
            fee: fee_sats,
            max_fee: policy.max_fee_sats_per_tx,
        });
    }

    // Exact destination match, known-script-only change, and conservation
    // on the final signed bytes. Classification is fail-closed and counts
    // occurrences: every output is either a declared destination (consumed
    // exactly once, with its exact requested amount) or the single change
    // output; every destination is consumed exactly once. Anything else
    // rejects construction.
    let mut consumed = vec![false; destinations.len()];
    let mut change_amount: Option<u64> = None;
    for output in &signed.output {
        let script = &output.script_pubkey;
        let mut matched_destination = false;
        for (index, destination) in destinations.iter().enumerate() {
            if !consumed[index] && destination.script_pubkey == *script {
                consumed[index] = true;
                matched_destination = true;
                if output.value.to_sat() != destination.amount_sats {
                    return Err(BuilderError::DestinationAmountMismatch {
                        requested: destination.amount_sats,
                        built: output.value.to_sat(),
                    });
                }
                break;
            }
        }
        if matched_destination {
            continue;
        }
        if *script == change_script_pubkey {
            if change_amount.is_some() {
                return Err(BuilderError::UnknownOutputScript {
                    script_hex: script.to_hex_string(),
                });
            }
            change_amount = Some(output.value.to_sat());
            continue;
        }
        return Err(BuilderError::UnknownOutputScript {
            script_hex: script.to_hex_string(),
        });
    }
    for (index, destination) in destinations.iter().enumerate() {
        if !consumed[index] {
            return Err(BuilderError::DestinationAmountMismatch {
                requested: destination.amount_sats,
                built: 0,
            });
        }
    }
    let checked_outputs = outputs_total;
    let checked_inputs = inputs_total;
    let outputs_plus_fee = checked_add_sats(checked_outputs, fee_sats).map_err(|_| {
        BuilderError::ConservationViolation {
            inputs: checked_inputs,
            outputs: checked_outputs,
            fee: fee_sats,
        }
    })?;
    if checked_inputs != outputs_plus_fee {
        return Err(BuilderError::ConservationViolation {
            inputs: checked_inputs,
            outputs: checked_outputs,
            fee: fee_sats,
        });
    }

    let txid = signed.compute_txid();
    let raw_bytes = consensus_serialize(&signed);
    Ok(SignedTrafficTx {
        tx: signed,
        txid,
        raw_bytes,
        weight_wu,
        vbytes,
        fee_sats,
        spent_outpoints: utxos.iter().map(|u| u.outpoint).collect(),
        destinations: destinations.to_vec(),
        change: change_amount.map(|amount| ChangeInfo {
            script_pubkey: change_script_pubkey.clone(),
            amount_sats: amount,
            chain: Chain::Change,
            index: change_index,
        }),
        category,
    })
}

fn assemble_transaction(
    utxos: &[SignableUtxo],
    destinations: &[DestinationSpec],
    change: Option<(ScriptBuf, u64)>,
    version: Version,
) -> Transaction {
    let input = utxos
        .iter()
        .map(|utxo| TxIn {
            previous_output: utxo.outpoint,
            script_sig: ScriptBuf::new(),
            sequence: Sequence::MAX,
            witness: Witness::default(),
        })
        .collect();
    let mut output: Vec<TxOut> = destinations
        .iter()
        .map(|destination| TxOut {
            value: bitcoin::Amount::from_sat(destination.amount_sats),
            script_pubkey: destination.script_pubkey.clone(),
        })
        .collect();
    if let Some((script, amount)) = change {
        output.push(TxOut {
            value: bitcoin::Amount::from_sat(amount),
            script_pubkey: script,
        });
    }
    Transaction {
        version,
        lock_time: LockTime::ZERO,
        input,
        output,
    }
}

/// Sign every input of a legacy P2PKH transaction in place with
/// `SIGHASH_ALL`, using low-S normalized RFC 6979 ECDSA signatures.
fn sign_legacy_p2pkh(
    secp: &Secp256k1<bitcoin::secp256k1::All>,
    mut tx: Transaction,
    utxos: &[SignableUtxo],
) -> Result<Transaction, BuilderError> {
    // Compute every sighash first (immutable borrow), then fill scriptSigs.
    let mut sighashes = Vec::with_capacity(utxos.len());
    {
        let cache = SighashCache::new(&tx);
        for (index, utxo) in utxos.iter().enumerate() {
            let sighash = cache
                .legacy_signature_hash(index, &utxo.script_pubkey, SIGHASH_ALL)
                .map_err(|e| BuilderError::Signing(format!("sighash: {e}")))?;
            sighashes.push(Message::from(sighash));
        }
    }
    for (index, utxo) in utxos.iter().enumerate() {
        let mut signature = secp.sign_ecdsa(&sighashes[index], &utxo.secret_key);
        signature.normalize_s();
        let mut signature_with_type = signature.serialize_der().as_ref().to_vec();
        signature_with_type.push(SIGHASH_ALL as u8);
        let mut script_sig = ScriptBuf::new();
        let sig_push = bitcoin::blockdata::script::PushBytesBuf::try_from(signature_with_type)
            .map_err(|e| BuilderError::Signing(format!("push sig: {e}")))?;
        script_sig.push_slice(&sig_push);
        let key_push = bitcoin::blockdata::script::PushBytesBuf::try_from(
            utxo.public_key.inner.serialize().to_vec(),
        )
        .map_err(|e| BuilderError::Signing(format!("push pubkey: {e}")))?;
        script_sig.push_slice(&key_push);
        tx.input[index].script_sig = script_sig;
    }
    Ok(tx)
}

/// Double-SHA256 helper used by the independent verification path in tests.
pub fn sha256d(data: &[u8]) -> [u8; 32] {
    let hash = Sha256d::hash(data);
    hash.to_byte_array()
}
