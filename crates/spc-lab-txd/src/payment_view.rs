//! Read-only T1 test-payment funds. No journal append, address allocation,
//! pending-funds inference, second wallet, or SP/Ark aggregation lives here.
use crate::{
    actor_config::AllocationDescription, control::invalid, keys::WalletRole,
    observer::CanonicalIndex, wallet::TrafficWalletSet,
};
use serde::{Deserialize, Serialize};
use std::io;

pub const SCHEMA: &str = "spc-t1-payment-wallet/v1";
pub const MAX_AGE_MS: u64 = 15_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaymentWalletView {
    pub schema: String,
    pub user_id: String,
    pub state: String,
    pub source: String,
    pub stage: String,
    pub spendable_sats: Option<String>,
    pub unconfirmed_sats: Option<String>,
    pub pending_sats: Option<String>,
    pub receive_address: Option<String>,
    pub source_observed_at_ms: Option<u64>,
    pub source_age_ms: Option<u64>,
    pub reason: Option<String>,
}
impl PaymentWalletView {
    pub fn unavailable(user: &str, stage: &str, reason: &str) -> Self {
        Self {
            schema: SCHEMA.into(),
            user_id: user.into(),
            state: "not_available".into(),
            source: "T1 test-payment funds".into(),
            stage: stage.into(),
            spendable_sats: None,
            unconfirmed_sats: None,
            pending_sats: None,
            receive_address: None,
            source_observed_at_ms: None,
            source_age_ms: None,
            reason: Some(reason.into()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaymentRead {
    pub user_id: String,
    pub wallet_index: u32,
    pub allocation_digest: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaymentReply {
    pub driver_id: String,
    pub wallet_index: u32,
    pub allocation_digest: String,
    pub observation_boot_id: Option<String>,
    pub observation_monotonic_ms: Option<u64>,
    pub journal_seq: u64,
    pub view: PaymentWalletView,
}
#[derive(Clone)]
pub(crate) struct ObservationStamp {
    pub boot_id: String,
    pub monotonic_ms: u64,
    pub wall_ms: u64,
}

pub(crate) fn read_wallet(
    request: PaymentRead,
    allocation: &AllocationDescription,
    driver: &str,
    wallet: &TrafficWalletSet,
    index: Option<&CanonicalIndex>,
    observed: Option<&ObservationStamp>,
    active: bool,
) -> io::Result<PaymentReply> {
    let validated = allocation.reconstruct()?;
    let (owner, global_index) = allocation.owner_of_user(&request.user_id)?;
    if owner != driver
        || global_index != request.wallet_index
        || request.allocation_digest != validated.digest()
        || wallet
            .journal()
            .header()
            .map_err(|e| invalid(&e.to_string()))?
            .allocation_digest
            != validated.digest()
    {
        return Err(invalid("payment_read_scope_mismatch"));
    }
    let snapshot = wallet.journal().recovery_snapshot();
    let stage = if active { "active" } else { "not_active" };
    let mut view =
        PaymentWalletView::unavailable(&request.user_id, stage, "observation_unavailable");
    let now = crate::bootstrap::monotonic_ms()?;
    let age = observed.and_then(|o| now.checked_sub(o.monotonic_ms));
    view.source_age_ms = age;
    view.source_observed_at_ms = observed.map(|o| o.wall_ms);
    let fresh = age.is_some_and(|n| n <= MAX_AGE_MS)
        && observed.is_some_and(|o| crate::bootstrap::boot_id().is_ok_and(|b| b == o.boot_id));
    if !fresh && observed.is_some() {
        view.state = "stale".into();
        view.reason = Some("observation_stale".into());
    }
    if let Some(index) = index.filter(|_| fresh && active && !snapshot.poisoned) {
        if snapshot.tip.as_ref() != Some(&index.anchor()) {
            return Err(invalid("payment_view_anchor_mismatch"));
        }
        let role = WalletRole::UserPayment {
            index: global_index,
        };
        let mut total = 0u64;
        for (outpoint, amount, _, _, script) in wallet.available_outpoints(&role) {
            let actual = index
                .unspent
                .get(&outpoint)
                .ok_or_else(|| invalid("payment_view_unproven_outpoint"))?;
            if actual.value.to_sat() != amount
                || actual.script_pubkey != script
                || !index.transactions.contains_key(&outpoint.txid)
            {
                return Err(invalid("payment_view_canonical_mismatch"));
            }
            total = total
                .checked_add(amount)
                .ok_or_else(|| invalid("payment_view_balance_overflow"))?;
        }
        // This existing method derives at the current receive cursor without
        // consuming it. The actor is the only writer throughout this read.
        let address = wallet
            .next_receive_address(&role)
            .map_err(|e| invalid(&e.to_string()))?;
        view.spendable_sats = Some(total.to_string());
        view.receive_address = Some(address.address.to_string());
        view.state = if snapshot.barrier_active {
            "not_available"
        } else {
            "available"
        }
        .into();
        view.reason = if snapshot.barrier_active {
            Some("traffic_paused".into())
        } else {
            None
        };
    }
    Ok(PaymentReply {
        driver_id: driver.into(),
        wallet_index: global_index,
        allocation_digest: validated.digest().into(),
        observation_boot_id: observed.map(|o| o.boot_id.clone()),
        observation_monotonic_ms: observed.map(|o| o.monotonic_ms),
        journal_seq: snapshot.committed_seq,
        view,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        actor_config::AllocationActor,
        builder::{DestinationSpec, PaymentCategory},
        journal::{FundingContext, FundingObservation, FundingSource},
        keys::Chain,
        wallet::PaymentRequest,
    };
    use bitcoin::{Amount, OutPoint, TxOut};
    use std::collections::BTreeMap;

    fn allocation() -> AllocationDescription {
        AllocationDescription {
            run_id: "read-model".into(),
            run_seed: 42,
            network: bitcoin::Network::Regtest,
            caps: Default::default(),
            users: vec!["alice".into(), "bob".into()],
            actors: vec![
                AllocationActor {
                    driver_id: "a".into(),
                    network_wallets: 1,
                    user_payment_wallets: 1,
                    hosts_bootstrap_funding: true,
                    lookahead: 2,
                    fee_policy: Default::default(),
                },
                AllocationActor {
                    driver_id: "b".into(),
                    network_wallets: 1,
                    user_payment_wallets: 1,
                    hosts_bootstrap_funding: false,
                    lookahead: 2,
                    fee_policy: Default::default(),
                },
            ],
        }
    }
    #[test]
    fn exact_large_balance_scope_unknown_reservations_and_polling_are_read_only() {
        // Explicit offline canonical-index fixture for the read model. This
        // test is not a node/source proof of its intentionally >2^53 amount.
        let dir = tempfile::tempdir().unwrap();
        let allocation = allocation();
        let token = allocation.reconstruct().unwrap();
        let wallet = TrafficWalletSet::provision(&token, "a", dir.path()).unwrap();
        let role = WalletRole::UserPayment { index: 0 };
        let script = wallet.next_receive_address(&role).unwrap().script_pubkey;
        let genesis = bitcoin::blockdata::constants::genesis_block(bitcoin::Network::Regtest);
        let tx = genesis.txdata[0].clone();
        let outpoint = OutPoint::new(tx.compute_txid(), 0);
        let amount = (1u64 << 53) + 123;
        wallet
            .note_chain_observed(genesis.block_hash(), 0, &[])
            .unwrap();
        wallet
            .observe_funding(&FundingObservation {
                outpoint,
                amount_sats: amount,
                chain: Chain::Receive,
                index: 0,
                owner_role: role.clone(),
                source: FundingSource::CanonicalBlock,
                context: FundingContext::CanonicalBlock {
                    block_hash: genesis.block_hash().to_string(),
                    height: 0,
                    coinbase: false,
                },
            })
            .unwrap();
        let index = CanonicalIndex {
            blocks: vec![genesis],
            transactions: BTreeMap::from([(tx.compute_txid(), (0, tx))]),
            unspent: BTreeMap::from([(
                outpoint,
                TxOut {
                    value: Amount::from_sat(amount),
                    script_pubkey: script,
                },
            )]),
            fees: BTreeMap::new(),
        };
        let request = PaymentRead {
            user_id: "alice".into(),
            wallet_index: 0,
            allocation_digest: token.digest().into(),
        };
        let stamp = ObservationStamp {
            boot_id: crate::bootstrap::boot_id().unwrap(),
            monotonic_ms: crate::bootstrap::monotonic_ms().unwrap(),
            wall_ms: crate::now_unix_ms(),
        };
        let before = std::fs::read(wallet.journal().path()).unwrap();
        let cursor = wallet.journal().next_receive_index(&role);
        let mut address = None;
        for _ in 0..2 {
            let reply = read_wallet(
                request.clone(),
                &allocation,
                "a",
                &wallet,
                Some(&index),
                Some(&stamp),
                true,
            )
            .unwrap();
            assert_eq!(reply.view.state, "available");
            assert_eq!(
                reply.view.spendable_sats.as_deref(),
                Some("9007199254741115")
            );
            assert!(reply.view.pending_sats.is_none());
            assert!(reply.view.unconfirmed_sats.is_none());
            if let Some(previous) = &address {
                assert_eq!(&reply.view.receive_address, previous);
            }
            address = Some(reply.view.receive_address.clone());
            let public = serde_json::to_value(&reply.view).unwrap();
            assert!(public["spendable_sats"].is_string());
            assert!(public.get("wallet_index").is_none());
        }
        assert_eq!(std::fs::read(wallet.journal().path()).unwrap(), before);
        assert_eq!(wallet.journal().next_receive_index(&role), cursor);
        let mut wrong = request.clone();
        wrong.user_id = "bob".into();
        wrong.wallet_index = 1;
        assert!(read_wallet(
            wrong,
            &allocation,
            "a",
            &wallet,
            Some(&index),
            Some(&stamp),
            true
        )
        .is_err());
        let mut wrong = request.clone();
        wrong.wallet_index = 1;
        assert!(read_wallet(
            wrong,
            &allocation,
            "a",
            &wallet,
            Some(&index),
            Some(&stamp),
            true
        )
        .is_err());
        assert_eq!(std::fs::read(wallet.journal().path()).unwrap(), before);
        let destination = wallet
            .next_receive_address(&WalletRole::NetworkWallet { index: 0 })
            .unwrap();
        wallet
            .sign_payment(&PaymentRequest {
                command_id: "outgoing".into(),
                payer_role: role,
                category: PaymentCategory::UserPayment,
                destinations: vec![DestinationSpec {
                    script_pubkey: destination.script_pubkey,
                    amount_sats: 100_000,
                    owner: Some(WalletRole::NetworkWallet { index: 0 }),
                }],
                fee_rate_sats_vb: 1,
            })
            .unwrap();
        wallet.note_broadcast_sent("outgoing").unwrap();
        wallet
            .note_unknown("outgoing", "offline response loss")
            .unwrap();
        let reply = read_wallet(
            request.clone(),
            &allocation,
            "a",
            &wallet,
            Some(&index),
            Some(&stamp),
            true,
        )
        .unwrap();
        assert_eq!(reply.view.spendable_sats.as_deref(), Some("0"));
        let stale = ObservationStamp {
            boot_id: "another-boot".into(),
            ..stamp
        };
        let reply = read_wallet(
            request,
            &allocation,
            "a",
            &wallet,
            Some(&index),
            Some(&stale),
            true,
        )
        .unwrap();
        assert_eq!(reply.view.state, "stale");
        assert!(reply.view.spendable_sats.is_none());
    }
}
