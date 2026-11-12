//! T2's bounded operations on the one actor-owned wallet. The PoW producer is
//! separately supplied; these handlers never create blocks or funding claims.
use super::*;
use crate::builder::DestinationSpec;
use crate::journal::{FundingContext, FundingObservation, FundingSource, TxState};
use crate::keys::{Chain, TrafficWallet};
use crate::observer::{CanonicalIndex, RawRpc};
use bitcoin::{BlockHash, OutPoint};
use sha2::{Digest, Sha256};

impl<W: Write> Actor<W> {
    pub(super) fn bootstrap_control(
        &mut self,
        operation: &str,
        command: Option<&str>,
        payload: Value,
    ) -> io::Result<Value> {
        let window = self
            .bootstrap_window
            .clone()
            .ok_or_else(|| invalid("bootstrap_window_closed"))?;
        if command != Some(window.command_id.as_str())
            || window.boot_id != crate::bootstrap::boot_id()?
            || crate::bootstrap::monotonic_ms()? >= window.deadline_monotonic_ms
        {
            return Err(invalid("bootstrap_operation_expired_or_mismatched"));
        }
        match operation {
            "bootstrap_rpc" => {
                #[derive(serde::Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Request {
                    node_id: String,
                    method: String,
                    params: Value,
                }
                let request: Request = serde_json::from_value(payload)?;
                if !self.config.node_ids.contains(&request.node_id) {
                    return Err(invalid("bootstrap_node_not_declared"));
                }
                if !matches!(
                    request.method.as_str(),
                    "getblocktemplate"
                        | "submitblock"
                        | "getbestblockhash"
                        | "getblockhash"
                        | "getblockheader"
                        | "getblock"
                        | "getrawmempool"
                        | "getrawtransaction"
                        | "getmempoolinfo"
                ) {
                    return Err(invalid("bootstrap_rpc_method_not_allowed"));
                }
                let result = self.rpc(RpcRequest {
                    node_id: request.node_id,
                    method: request.method,
                    params: request.params,
                    send_sequence: None,
                    txid: None,
                })?;
                Ok(json!({"status":"rpc_result","result":result}))
            }
            "funding_address" => {
                let wallet = self
                    .wallet
                    .as_ref()
                    .ok_or_else(|| invalid("wallet missing"))?;
                let address = wallet
                    .next_receive_address(&WalletRole::BootstrapFunding)
                    .map_err(|e| invalid(&e.to_string()))?;
                Ok(
                    json!({"status":"address","index":wallet.journal().next_receive_index(&WalletRole::BootstrapFunding),
                    "script_hex":address.script_pubkey.to_hex_string()}),
                )
            }
            "observe_bootstrap_coinbase" => {
                #[derive(serde::Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Claim {
                    block_hash: BlockHash,
                    index: u32,
                }
                let claim: Claim = serde_json::from_value(payload)?;
                let policy = self.config.chain_policy.clone();
                let limit = policy.pow_limit().map_err(|e| invalid(&e))?;
                let node = window.node_id.clone();
                let index = CanonicalIndex::scan(self, &node, policy.genesis, limit)
                    .map_err(|e| invalid(&e))?;
                index.verify_policy(&policy).map_err(|e| invalid(&e))?;
                let (height, block) = index
                    .blocks
                    .iter()
                    .enumerate()
                    .find(|(_, b)| b.block_hash() == claim.block_hash)
                    .ok_or_else(|| invalid("funding block not canonical"))?;
                // Every actual node must corroborate the exact canonical
                // funding block before its first credit is admitted.
                for other in self
                    .config
                    .node_ids
                    .clone()
                    .into_iter()
                    .filter(|n| n != &node)
                {
                    if self
                        .read(&other, "getblockhash", json!([height]))
                        .map_err(|e| invalid(&e))?
                        .as_str()
                        != Some(claim.block_hash.to_string().as_str())
                    {
                        return Err(invalid("funding canonical corroboration missing"));
                    }
                }
                let wallet = self
                    .wallet
                    .as_ref()
                    .ok_or_else(|| invalid("wallet missing"))?;
                let derived = TrafficWallet::new(
                    wallet.run_seed(),
                    wallet.run_id(),
                    WalletRole::BootstrapFunding,
                    wallet.network(),
                    1,
                )
                .map_err(|e| invalid(&e.to_string()))?
                .script_at(Chain::Receive, claim.index)
                .map_err(|e| invalid(&e.to_string()))?;
                let coinbase = &block.txdata[0];
                if height == 0
                    || coinbase.output.len() != 1
                    || coinbase.output[0].script_pubkey != derived.script_pubkey
                {
                    return Err(invalid("funding coinbase destination mismatch"));
                }
                let outpoint = OutPoint {
                    txid: coinbase.compute_txid(),
                    vout: 0,
                };
                let observation = FundingObservation {
                    outpoint,
                    amount_sats: coinbase.output[0].value.to_sat(),
                    chain: Chain::Receive,
                    index: claim.index,
                    owner_role: WalletRole::BootstrapFunding,
                    source: FundingSource::BootstrapCoinbase,
                    context: FundingContext::CanonicalBlock {
                        block_hash: claim.block_hash.to_string(),
                        height: height as u64,
                        coinbase: true,
                    },
                };
                let known = wallet.journal().observer_snapshot();
                if let Some(existing) = known.outpoints.get(&outpoint) {
                    if existing.amount_sats != observation.amount_sats
                        || existing.index != claim.index
                        || existing.script != derived.script_pubkey
                        || existing.source != FundingSource::BootstrapCoinbase
                    {
                        return Err(invalid("funding identity conflict"));
                    }
                    if !matches!(
                        existing.status,
                        crate::journal::OutpointStatus::Invalidated { .. }
                    ) {
                        return Ok(json!({"status":"funding_observed"}));
                    }
                } else if claim.index
                    != wallet
                        .journal()
                        .next_receive_index(&WalletRole::BootstrapFunding)
                {
                    return Err(invalid("funding receive index mismatch"));
                }
                // The authoritative observer admits tip/rollback first. A
                // fresh height is never fabricated to make a coin mature.
                let wallet = self.wallet.take().unwrap();
                let observed = index.observe(&wallet, self, &node, policy.genesis, limit, 1);
                self.wallet = Some(wallet);
                observed.map_err(|e| invalid(&e))?;
                self.wallet
                    .as_ref()
                    .unwrap()
                    .observe_funding(&observation)
                    .map_err(|e| invalid(&e.to_string()))?;
                self.index = Some(index);
                Ok(json!({"status":"funding_observed","height":height}))
            }
            "distribute" => {
                #[derive(serde::Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Output {
                    role: WalletRole,
                    receive_index: u32,
                    amount_sats: u64,
                }
                #[derive(serde::Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Distribution {
                    part: u32,
                    outputs: Vec<Output>,
                    fee_rate_sats_vb: u64,
                }
                let distribution: Distribution = serde_json::from_value(payload)?;
                if distribution.outputs.is_empty() || distribution.outputs.len() > 16_384 {
                    return Err(invalid("distribution output bound"));
                }
                self.observe_authority().map_err(|e| invalid(&e))?;
                let total_network: u32 = self
                    .config
                    .allocation
                    .actors
                    .iter()
                    .map(|a| a.network_wallets)
                    .sum();
                let total_users = self.config.allocation.users.len() as u32;
                let mut destinations = Vec::new();
                for output in distribution.outputs {
                    if !matches!(output.role,WalletRole::NetworkWallet {index} if index<total_network)
                        && !matches!(output.role,WalletRole::UserPayment {index} if index<total_users)
                    {
                        return Err(invalid("distribution role outside allocation"));
                    }
                    if output.receive_index >= 4096 {
                        return Err(invalid("distribution receive bound"));
                    }
                    let script = TrafficWallet::new(
                        self.config.allocation.run_seed,
                        &self.config.allocation.run_id,
                        output.role.clone(),
                        bitcoin::Network::Regtest,
                        1,
                    )
                    .map_err(|e| invalid(&e.to_string()))?
                    .script_at(Chain::Receive, output.receive_index)
                    .map_err(|e| invalid(&e.to_string()))?
                    .script_pubkey;
                    destinations.push(DestinationSpec {
                        script_pubkey: script,
                        amount_sats: output.amount_sats,
                        owner: None,
                    });
                }
                let id = format!(
                    "bootstrap/{}/{:08}",
                    hex::encode(Sha256::digest(window.command_id.as_bytes())),
                    distribution.part
                );
                let request = PaymentRequest {
                    command_id: id.clone(),
                    payer_role: WalletRole::BootstrapFunding,
                    category: PaymentCategory::BootstrapDistribution,
                    destinations,
                    fee_rate_sats_vb: distribution.fee_rate_sats_vb,
                };
                let signed = self
                    .wallet
                    .as_ref()
                    .unwrap()
                    .sign_payment(&request)
                    .map_err(|e| invalid(&e.to_string()))?;
                let view = match signed {
                    PaymentOutcome::Signed(v) | PaymentOutcome::ReplayedStored(v) => v,
                };
                let state = self.wallet.as_ref().unwrap().journal().tx_state(&view.txid);
                if matches!(state, Some(TxState::Signed | TxState::Unknown)) {
                    self.forward(&view)
                } else {
                    Ok(json!({"status":"stored","txid":view.txid.to_string()}))
                }
            }
            "bootstrap_status" => {
                let prefix = format!(
                    "bootstrap/{}/",
                    hex::encode(Sha256::digest(window.command_id.as_bytes()))
                );
                let snapshot = self
                    .wallet
                    .as_ref()
                    .ok_or_else(|| invalid("wallet missing"))?
                    .journal()
                    .recovery_snapshot();
                let ids: Vec<_> = snapshot
                    .transactions
                    .iter()
                    .filter(|(_, tx)| {
                        tx.facts.state == TxState::Confirmed
                            && tx.facts.category == PaymentCategory::BootstrapDistribution
                            && tx.facts.command_id.starts_with(&prefix)
                    })
                    .map(|(id, _)| id.to_string())
                    .collect();
                Ok(
                    json!({"status":"observed","confirmed_distribution_txids":ids,"scan_complete":self.index.is_some()}),
                )
            }
            _ => Err(invalid("unsupported_bootstrap_control")),
        }
    }
}
