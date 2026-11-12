//! Declared, externally signed legacy transactions enter the existing wallet
//! journal. No signer, funding import, release path or shadow ledger exists.
use std::collections::BTreeSet;

use bitcoin::consensus::{deserialize, serialize};
use bitcoin::hashes::Hash;
use bitcoin::script::Instruction;
use bitcoin::sighash::{EcdsaSighashType, SighashCache};
use bitcoin::{OutPoint, Transaction};

use crate::builder::{DestinationSpec, PaymentCategory, SignedTrafficTx};
use crate::journal::{
    CommandBeginOutcome, DestinationRecord, OutpointStatus, RequestRecord, SignedTxFacts,
};
use crate::wallet::TrafficWalletSet;

/// Returns the journal's stored identity, including on a byte-identical txid
/// duplicate under a different outer command id. A partially committed old
/// admission without SignedTx bytes is uncertain and retains its reservation;
/// it is never completed using a newly supplied unbound raw transaction.
pub fn admit_declared_tx(
    wallet: &TrafficWalletSet,
    command_id: &str,
    raw_hex: &str,
    max_vbytes: u64,
) -> Result<SignedTxFacts, String> {
    if command_id.is_empty()
        || command_id.len() > 128
        || raw_hex.len() > crate::control::MAX_BODY
        || raw_hex.is_empty()
        || !raw_hex.len().is_multiple_of(2)
    {
        return Err("traffic_declared_shape_invalid".into());
    }
    let bytes = hex::decode(raw_hex).map_err(|_| "traffic_declared_hex_invalid")?;
    let tx: Transaction = deserialize(&bytes).map_err(|_| "traffic_declared_consensus_invalid")?;
    if serialize(&tx) != bytes
        || tx.input.is_empty()
        || tx.is_coinbase()
        || tx.output.is_empty()
        || tx.input.iter().any(|i| !i.witness.is_empty())
        || tx.vsize() as u64 > max_vbytes
    {
        return Err("traffic_declared_profile_invalid".into());
    }
    let txid = tx.compute_txid();
    let recovery = wallet.journal().recovery_snapshot();
    // Nonstandard version 3 is never admitted as a new external transaction.
    // Only exact replay of the explicit, already signed construction case is
    // allowed; its original command/effect binding cannot be invented here.
    let standard_version = matches!(tx.version.0, 1 | 2);
    let selected_replay = tx.version.0 == 3
        && recovery.commands.get(command_id).is_some_and(|c| {
            c.request.construction_effect
                == Some(crate::construction::ConstructionEffect::VersionRejectionV1)
                && c.txid == Some(txid)
                && c.raw_hex.as_deref() == Some(raw_hex)
        });
    if !(standard_version || selected_replay) {
        return Err("traffic_declared_version_not_selected".into());
    }
    if recovery.poisoned || recovery.barrier_active {
        return Err("traffic_send_gate_closed".into());
    }
    if let Some(command) = recovery.commands.get(command_id) {
        if command.txid == Some(txid)
            && command.raw_hex.as_deref() == Some(hex::encode(&bytes).as_str())
        {
            return wallet
                .journal()
                .tx_facts(&txid)
                .ok_or_else(|| "traffic_declared_history_missing".into());
        }
        return Err("traffic_declared_command_conflict_or_incomplete".into());
    }
    if let Some(existing) = recovery.transactions.get(&txid) {
        if hex::decode(&existing.facts.raw_hex).map_err(|_| "traffic_declared_history_corrupt")?
            != bytes
        {
            return Err("traffic_declared_txid_collision".into());
        }
        return Ok(existing.facts.clone());
    }
    let funding = wallet.journal().observer_snapshot();
    let policy = wallet.fee_policy();
    policy.validate().map_err(|e| e.to_string())?;
    let mut input_sum = 0u64;
    let mut seen = BTreeSet::<OutPoint>::new();
    let mut payer = None;
    let secp = bitcoin::secp256k1::Secp256k1::verification_only();
    for (index, input) in tx.input.iter().enumerate() {
        if !seen.insert(input.previous_output) {
            return Err("traffic_declared_duplicate_input".into());
        }
        let previous = funding
            .outpoints
            .get(&input.previous_output)
            .ok_or("traffic_declared_prevout_unproven")?;
        if previous.coinbase_immature
            || previous.status != OutpointStatus::Available
            || !previous.script.is_p2pkh()
            || !wallet.roles().contains(&previous.owner)
        {
            return Err("traffic_declared_input_unavailable".into());
        }
        if payer.as_ref().is_some_and(|p| p != &previous.owner) {
            return Err("traffic_declared_mixed_owner".into());
        }
        payer = Some(previous.owner.clone());
        input_sum = input_sum
            .checked_add(previous.amount_sats)
            .ok_or("traffic_declared_amount_overflow")?;
        let instructions: Vec<_> = input
            .script_sig
            .instructions_minimal()
            .collect::<Result<_, _>>()
            .map_err(|_| "traffic_declared_script_sig_invalid")?;
        let [Instruction::PushBytes(signature), Instruction::PushBytes(public)] =
            instructions.as_slice()
        else {
            return Err("traffic_declared_script_sig_invalid".into());
        };
        let signature = bitcoin::ecdsa::Signature::from_slice(signature.as_bytes())
            .map_err(|_| "traffic_declared_signature_invalid")?;
        let public = bitcoin::PublicKey::from_slice(public.as_bytes())
            .map_err(|_| "traffic_declared_pubkey_invalid")?;
        let mut normalized = signature.signature;
        normalized.normalize_s();
        if signature.sighash_type != EcdsaSighashType::All
            || normalized != signature.signature
            || !public.compressed
            || bitcoin::ScriptBuf::new_p2pkh(&public.pubkey_hash()) != previous.script
        {
            return Err("traffic_declared_signature_policy".into());
        }
        let sighash = SighashCache::new(&tx)
            .legacy_signature_hash(index, &previous.script, EcdsaSighashType::All.to_u32())
            .map_err(|_| "traffic_declared_sighash_invalid")?;
        let message = bitcoin::secp256k1::Message::from_digest(sighash.to_byte_array());
        secp.verify_ecdsa(&message, &signature.signature, &public.inner)
            .map_err(|_| "traffic_declared_signature_invalid")?;
    }
    let mut output_sum = 0u64;
    let mut destinations = Vec::new();
    for output in &tx.output {
        if !crate::builder::supported_destination_script(&output.script_pubkey)
            || output.value.to_sat() < policy.dust_sats
        {
            return Err("traffic_declared_destination_invalid".into());
        }
        output_sum = output_sum
            .checked_add(output.value.to_sat())
            .ok_or("traffic_declared_amount_overflow")?;
        destinations.push(DestinationSpec {
            script_pubkey: output.script_pubkey.clone(),
            amount_sats: output.value.to_sat(),
            owner: None,
        });
    }
    let fee = input_sum
        .checked_sub(output_sum)
        .ok_or("traffic_declared_outputs_exceed_inputs")?;
    let vbytes = tx.vsize() as u64;
    if fee == 0
        || fee > policy.max_fee_sats_per_tx
        || u128::from(fee) < u128::from(policy.min_rate_sats_vb) * u128::from(vbytes)
        || u128::from(fee) > u128::from(policy.max_rate_sats_vb) * u128::from(vbytes)
    {
        return Err("traffic_declared_fee_outside_policy".into());
    }
    let rate = fee / vbytes;
    let payer = payer.ok_or("traffic_declared_owner_missing")?;
    let request = RequestRecord {
        construction_effect: None,
        payer_role: payer.clone(),
        category: PaymentCategory::Ordinary,
        destinations: destinations
            .iter()
            .map(|d| DestinationRecord {
                script_hex: d.script_pubkey.to_hex_string(),
                amount_sats: d.amount_sats,
                owner_role: None,
            })
            .collect(),
        fee_rate_sats_vb: rate,
    };
    match wallet
        .journal()
        .begin_command(command_id, request)
        .map_err(|e| e.to_string())?
    {
        CommandBeginOutcome::New => {}
        CommandBeginOutcome::Stored(_) => {
            return Err("traffic_declared_concurrent_admission".into())
        }
    }
    let outpoints: Vec<_> = tx.input.iter().map(|i| i.previous_output).collect();
    wallet
        .journal()
        .reserve(
            command_id,
            &outpoints,
            wallet.journal().next_change_index(&payer),
        )
        .map_err(|e| e.to_string())?;
    let signed = SignedTrafficTx {
        weight_wu: tx.weight().to_wu(),
        tx,
        txid,
        raw_bytes: bytes,
        vbytes,
        fee_sats: fee,
        spent_outpoints: outpoints,
        destinations,
        change: None,
        category: PaymentCategory::Ordinary,
    };
    wallet
        .journal()
        .record_signed(command_id, &signed, rate)
        .map_err(|e| e.to_string())?;
    wallet
        .journal()
        .tx_facts(&txid)
        .ok_or_else(|| "traffic_declared_commit_missing".into())
}
