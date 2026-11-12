//! Independent completion verification from raw canonical node evidence. T2
//! claims identify candidate transactions; they cannot manufacture a boundary.
use bitcoin::{OutPoint, ScriptBuf, Transaction, Txid};
use spc_lab_txd::actor_config::AllocationDescription;
use spc_lab_txd::bootstrap::BootstrapCompletion;
use spc_lab_txd::keys::{Chain, TrafficWallet, WalletRole};
use spc_lab_txd::observer::{CanonicalIndex, ChainPolicy};
use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

pub fn verify_distribution(
    completion: &BootstrapCompletion,
    indices: &[CanonicalIndex],
    allocation: &AllocationDescription,
    policy: &ChainPolicy,
    funding_blocks: u32,
    outputs_per_wallet: u32,
) -> Result<(), String> {
    allocation.reconstruct().map_err(|e| e.to_string())?;
    if indices.is_empty()
        || completion.distribution_txids.is_empty()
        || completion.distribution_txids.len() > 4096
        || outputs_per_wallet == 0
    {
        return Err("bootstrap_completion_shape_invalid".into());
    }
    let authority = &indices[0];
    authority.verify_policy(policy)?;
    if authority.anchor().block_hash.to_string() != completion.common_tip_hash
        || authority.anchor().height != completion.common_tip_height
    {
        return Err("bootstrap_common_anchor_mismatch".into());
    }
    for index in indices.iter().skip(1) {
        index.verify_policy(policy)?;
        if !authority.corroborates(index) {
            return Err("bootstrap_nodes_not_common".into());
        }
    }
    let mut ids = BTreeSet::new();
    for id in &completion.distribution_txids {
        if !ids.insert(Txid::from_str(id).map_err(|_| "bootstrap_distribution_txid_invalid")?) {
            return Err("bootstrap_distribution_duplicate".into());
        }
    }
    let mut scripts = BTreeMap::<ScriptBuf, WalletRole>::new();
    let mut counts = BTreeMap::<WalletRole, u32>::new();
    let mut network = 0u32;
    let mut user = 0u32;
    for actor in &allocation.actors {
        for (start, count, network_role) in [
            (network, actor.network_wallets, true),
            (user, actor.user_payment_wallets, false),
        ] {
            for index in start
                ..start
                    .checked_add(count)
                    .ok_or("bootstrap_wallet_count_overflow")?
            {
                let role = if network_role {
                    WalletRole::NetworkWallet { index }
                } else {
                    WalletRole::UserPayment { index }
                };
                counts.insert(role.clone(), 0);
                let wallet = TrafficWallet::new(
                    allocation.run_seed,
                    &allocation.run_id,
                    role.clone(),
                    allocation.network,
                    actor.lookahead,
                )
                .map_err(|e| e.to_string())?;
                for receive in 0..actor.lookahead.max(outputs_per_wallet) {
                    let script = wallet
                        .script_at(Chain::Receive, receive)
                        .map_err(|e| e.to_string())?
                        .script_pubkey;
                    if scripts.insert(script, role.clone()).is_some() {
                        return Err("bootstrap_script_allocation_ambiguous".into());
                    }
                }
            }
        }
        network = network
            .checked_add(actor.network_wallets)
            .ok_or("bootstrap_wallet_count_overflow")?;
        user = user
            .checked_add(actor.user_payment_wallets)
            .ok_or("bootstrap_wallet_count_overflow")?;
    }
    let funding = TrafficWallet::new(
        allocation.run_seed,
        &allocation.run_id,
        WalletRole::BootstrapFunding,
        allocation.network,
        1,
    )
    .map_err(|e| e.to_string())?;
    let mut funding_scripts = BTreeSet::new();
    let bound = funding_blocks
        .checked_add(ids.len() as u32)
        .and_then(|n| n.checked_add(1))
        .ok_or("bootstrap_derivation_overflow")?;
    for index in 0..bound {
        for chain in [Chain::Receive, Chain::Change] {
            funding_scripts.insert(
                funding
                    .script_at(chain, index)
                    .map_err(|e| e.to_string())?
                    .script_pubkey,
            );
        }
    }
    let mut checked = BTreeSet::new();
    for txid in &ids {
        let (height, tx) = authority
            .transactions
            .get(txid)
            .ok_or("bootstrap_distribution_not_canonical")?;
        if tx.is_coinbase()
            || tx.input.iter().any(|i| !i.witness.is_empty())
            || authority.fees.get(txid).is_none_or(|f| *f == 0)
        {
            return Err("bootstrap_distribution_not_fee_bearing_legacy".into());
        }
        prove_bootstrap_inputs(authority, tx, *height, &funding_scripts, &mut checked)?;
        for (vout, output) in tx.output.iter().enumerate() {
            if let Some(role) = scripts.get(&output.script_pubkey) {
                if output.value.to_sat() < 546
                    || !authority.unspent.contains_key(&OutPoint {
                        txid: *txid,
                        vout: vout as u32,
                    })
                {
                    return Err("bootstrap_distribution_output_unavailable".into());
                }
                let count = counts.get_mut(role).ok_or("bootstrap_role_missing")?;
                *count = count
                    .checked_add(1)
                    .ok_or("bootstrap_output_count_overflow")?;
            } else if !funding_scripts.contains(&output.script_pubkey) {
                return Err("bootstrap_distribution_foreign_destination".into());
            }
        }
    }
    if counts.values().any(|n| *n < outputs_per_wallet) {
        return Err("bootstrap_distribution_incomplete".into());
    }
    Ok(())
}

fn prove_bootstrap_inputs(
    index: &CanonicalIndex,
    tx: &Transaction,
    height: u64,
    scripts: &BTreeSet<ScriptBuf>,
    checked: &mut BTreeSet<Txid>,
) -> Result<(), String> {
    let mut pending = vec![(height, tx)];
    while let Some((height, tx)) = pending.pop() {
        if !checked.insert(tx.compute_txid()) {
            continue;
        }
        for input in &tx.input {
            let (created, creator) = index
                .transactions
                .get(&input.previous_output.txid)
                .ok_or("bootstrap_input_creation_unproven")?;
            let output = creator
                .output
                .get(input.previous_output.vout as usize)
                .ok_or("bootstrap_input_vout_missing")?;
            if !scripts.contains(&output.script_pubkey) {
                return Err("bootstrap_input_not_funding_wallet".into());
            }
            if creator.is_coinbase() {
                if *created == 0
                    || created
                        .checked_add(100)
                        .is_none_or(|mature| height < mature)
                {
                    return Err("bootstrap_coinbase_immature".into());
                }
            } else {
                pending.push((*created, creator));
            }
        }
    }
    Ok(())
}
