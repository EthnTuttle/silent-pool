//! Inclusive, tip-anchored raw canonical index. RPC reads are health evidence;
//! only complete verified scans may mutate the wallet's existing journal.
use crate::journal::{
    ChainAnchor, FundingContext, FundingObservation, FundingSource, OutpointStatus,
    RollbackEvidence, TxState,
};
use crate::keys::{Chain, TrafficWallet};
use crate::wallet::TrafficWalletSet;
use bitcoin::consensus::{deserialize, serialize};
use bitcoin::{Block, BlockHash, OutPoint, Target, Transaction, Txid};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

pub trait RawRpc {
    fn read(&mut self, node: &str, method: &str, params: Value) -> Result<Value, String>;
    /// Called between bounded CPU/scan steps so a pipe actor can yield to
    /// queued controls without dropping the current bounded RPC response.
    fn checkpoint(&mut self) -> Result<(), String> {
        Ok(())
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChainPolicy {
    pub genesis: BlockHash,
    pub pow_limit_bits: u32,
    pub initial_target_bits: u32,
    pub no_pow_retargeting: bool,
    pub allow_min_difficulty_blocks: bool,
    pub pow_target_spacing: u64,
    pub difficulty_adjustment_interval: u64,
    pub regtest_pow_bits_overlay: u32,
    pub subsidy_initial_sats: u64,
    pub subsidy_halving_interval: u64,
}
impl ChainPolicy {
    pub fn pow_limit(&self) -> Result<Target, String> {
        let bits = self.pow_limit_bits;
        let size = bits >> 24;
        let word = bits & 0x007fffff;
        let target = Target::from_compact(bitcoin::CompactTarget::from_consensus(bits));
        if bits & 0x00800000 != 0
            || size > 34
            || (word > 0xff && size > 33)
            || (word > 0xffff && size > 32)
            || target == Target::ZERO
            || target > Target::MAX_ATTAINABLE_REGTEST
            || self.subsidy_halving_interval == 0
        {
            return Err("traffic_chain_policy_invalid".into());
        }
        Ok(target)
    }
}

pub struct CanonicalIndex {
    /// Genesis first, inclusive through the rechecked tip.
    pub blocks: Vec<Block>,
    pub transactions: BTreeMap<Txid, (u64, Transaction)>,
    pub unspent: BTreeMap<OutPoint, bitcoin::TxOut>,
    pub fees: BTreeMap<Txid, u64>,
}

pub struct MempoolView {
    pub transactions: BTreeMap<Txid, Transaction>,
    pub membership_sha256: String,
    pub observed_monotonic_ms: u64,
}
impl MempoolView {
    pub fn capture(
        rpc: &mut impl RawRpc,
        node: &str,
        anchor: BlockHash,
        cap: usize,
    ) -> Result<Self, String> {
        fn membership(value: Value, cap: usize) -> Result<BTreeSet<Txid>, String> {
            let entries = value.as_array().ok_or("mempool list shape")?;
            if entries.len() > cap {
                return Err("mempool backlog bound".into());
            }
            let mut ids = BTreeSet::new();
            for entry in entries {
                let id = Txid::from_str(entry.as_str().ok_or("mempool txid shape")?)
                    .map_err(|e| e.to_string())?;
                if !ids.insert(id) {
                    return Err("duplicate mempool txid".into());
                }
            }
            Ok(ids)
        }
        let first = membership(rpc.read(node, "getrawmempool", json!([]))?, cap)?;
        let mut transactions = BTreeMap::new();
        for txid in &first {
            rpc.checkpoint()?;
            let raw = rpc.read(node, "getrawtransaction", json!([txid.to_string(), 0]))?;
            let bytes =
                hex::decode(raw.as_str().ok_or("mempool raw shape")?).map_err(|e| e.to_string())?;
            let tx: Transaction = deserialize(&bytes).map_err(|e| e.to_string())?;
            if tx.compute_txid() != *txid || serialize(&tx) != bytes {
                return Err("mempool transaction identity mismatch".into());
            }
            transactions.insert(*txid, tx);
        }
        if first != membership(rpc.read(node, "getrawmempool", json!([]))?, cap)?
            || read_tip(rpc, node)? != anchor
        {
            return Err("mempool/anchor changed during scan".into());
        }
        use sha2::Digest;
        let mut digest = sha2::Sha256::new();
        for txid in &first {
            digest.update(txid.to_string().as_bytes());
        }
        Ok(Self {
            transactions,
            membership_sha256: hex::encode(digest.finalize()),
            observed_monotonic_ms: crate::bootstrap::monotonic_ms().map_err(|e| e.to_string())?,
        })
    }
    pub fn conflict(&self, own: Txid, inputs: &[OutPoint]) -> Option<Txid> {
        self.transactions
            .iter()
            .find(|(id, tx)| {
                **id != own && tx.input.iter().any(|i| inputs.contains(&i.previous_output))
            })
            .map(|(id, _)| *id)
    }
}

fn raw_block(value: Value, expected: BlockHash, limit: Target) -> Result<Block, String> {
    let bytes =
        hex::decode(value.as_str().ok_or("raw block must be hex")?).map_err(|e| e.to_string())?;
    let block: Block = deserialize(&bytes).map_err(|e| e.to_string())?;
    let bits = block.header.bits.to_consensus();
    let size = bits >> 24;
    let word = bits & 0x007fffff;
    if serialize(&block) != bytes
        || block.block_hash() != expected
        || !block.check_merkle_root()
        || bits & 0x00800000 != 0
        || size > 34
        || (word > 0xff && size > 33)
        || (word > 0xffff && size > 32)
        || block.header.target() == Target::ZERO
        || block.header.target() > limit
        || block.header.validate_pow(block.header.target()).is_err()
        || block.txdata.is_empty()
        || !block.txdata[0].is_coinbase()
        || block.txdata.iter().skip(1).any(Transaction::is_coinbase)
    {
        return Err("invalid raw canonical block".into());
    }
    Ok(block)
}

impl CanonicalIndex {
    fn from_anchor(
        rpc: &mut impl RawRpc,
        node: &str,
        tip: BlockHash,
        genesis: BlockHash,
        limit: Target,
    ) -> Result<Self, String> {
        let mut reverse = Vec::new();
        let mut next = tip;
        let mut seen = BTreeSet::new();
        let mut bytes = 0usize;
        loop {
            rpc.checkpoint()?;
            if reverse.len() >= 4096 || !seen.insert(next) {
                return Err("canonical scan bound/cycle".into());
            }
            let raw = rpc.read(node, "getblock", json!([next.to_string(), 0]))?;
            bytes = bytes
                .checked_add(raw.as_str().ok_or("raw block shape")?.len())
                .ok_or("scan overflow")?;
            if bytes > 128 * 1024 * 1024 {
                return Err("canonical scan byte bound".into());
            }
            let block = raw_block(raw, next, limit)?;
            let at_genesis = next == genesis;
            next = block.header.prev_blockhash;
            reverse.push(block);
            if at_genesis {
                break;
            }
        }
        reverse.reverse();
        let mut transactions = BTreeMap::new();
        let mut unspent = BTreeMap::new();
        let mut fees = BTreeMap::new();
        for (height, block) in reverse.iter().enumerate() {
            for tx in &block.txdata {
                rpc.checkpoint()?;
                let id = tx.compute_txid();
                if transactions
                    .insert(id, (height as u64, tx.clone()))
                    .is_some()
                {
                    return Err("duplicate canonical txid".into());
                }
                if !tx.is_coinbase() {
                    let mut input_sats = 0u64;
                    for input in &tx.input {
                        let previous: bitcoin::TxOut = unspent
                            .remove(&input.previous_output)
                            .ok_or("canonical prevout coverage missing")?;
                        input_sats = input_sats
                            .checked_add(previous.value.to_sat())
                            .ok_or("canonical input amount overflow")?;
                    }
                    let output_sats = tx
                        .output
                        .iter()
                        .try_fold(0u64, |sum, o| sum.checked_add(o.value.to_sat()))
                        .ok_or("canonical output amount overflow")?;
                    fees.insert(
                        id,
                        input_sats
                            .checked_sub(output_sats)
                            .ok_or("canonical outputs exceed proven inputs")?,
                    );
                }
                for (vout, output) in tx.output.iter().enumerate() {
                    unspent.insert(
                        OutPoint {
                            txid: id,
                            vout: u32::try_from(vout).map_err(|_| "vout overflow")?,
                        },
                        output.clone(),
                    );
                }
            }
        }
        Ok(Self {
            blocks: reverse,
            transactions,
            unspent,
            fees,
        })
    }

    pub fn scan(
        rpc: &mut impl RawRpc,
        node: &str,
        genesis: BlockHash,
        limit: Target,
    ) -> Result<Self, String> {
        let tip = read_tip(rpc, node)?;
        let index = Self::from_anchor(rpc, node, tip, genesis, limit)?;
        if read_tip(rpc, node)? != tip {
            return Err("canonical anchor changed mid-scan".into());
        }
        Ok(index)
    }
    pub fn anchor(&self) -> ChainAnchor {
        ChainAnchor {
            block_hash: self
                .blocks
                .last()
                .expect("genesis-inclusive index")
                .block_hash(),
            height: self.blocks.len() as u64 - 1,
        }
    }

    pub fn verify_policy(&self, policy: &ChainPolicy) -> Result<(), String> {
        crate::contextual_pow::VerifiedHeaders::new(
            policy,
            self.blocks.iter().map(|b| b.header).collect(),
        )?;
        if self
            .blocks
            .first()
            .is_none_or(|b| b.block_hash() != policy.genesis)
        {
            return Err("canonical genesis mismatch".into());
        }
        for (height, block) in self.blocks.iter().enumerate().skip(1) {
            let halvings = height as u64 / policy.subsidy_halving_interval;
            let subsidy = if halvings >= 64 {
                0
            } else {
                policy.subsidy_initial_sats >> halvings
            };
            let fees = block
                .txdata
                .iter()
                .skip(1)
                .try_fold(0u64, |sum, tx| {
                    sum.checked_add(*self.fees.get(&tx.compute_txid())?)
                })
                .ok_or("canonical fee coverage/overflow")?;
            let revenue = block.txdata[0]
                .output
                .iter()
                .try_fold(0u64, |sum, o| sum.checked_add(o.value.to_sat()))
                .ok_or("coinbase amount overflow")?;
            if revenue > subsidy.checked_add(fees).ok_or("subsidy/fee overflow")? {
                return Err("coinbase exceeds bound subsidy and raw fees".into());
            }
        }
        Ok(())
    }

    pub fn corroborates(&self, other: &Self) -> bool {
        self.anchor() == other.anchor()
            && self
                .blocks
                .iter()
                .zip(&other.blocks)
                .all(|(a, b)| a.block_hash() == b.block_hash())
    }

    /// Complete mempool decoding plus stable membership and tip recheck. Own
    /// presence is deliberately excluded from conflicting-spend detection.
    pub fn prove_unspent(
        &self,
        rpc: &mut impl RawRpc,
        node: &str,
        own: Txid,
        inputs: &[OutPoint],
        backlog: usize,
    ) -> Result<bool, String> {
        let membership = |value: Value| -> Result<BTreeSet<Txid>, String> {
            let entries = value.as_array().ok_or("mempool list shape")?;
            if entries.len() > backlog {
                return Err("mempool backlog bound".into());
            }
            let mut out = BTreeSet::new();
            for entry in entries {
                let txid = Txid::from_str(entry.as_str().ok_or("txid shape")?)
                    .map_err(|e| e.to_string())?;
                if !out.insert(txid) {
                    return Err("duplicate mempool txid".into());
                }
            }
            Ok(out)
        };
        if inputs.iter().any(|p| !self.unspent.contains_key(p)) {
            return Ok(false);
        }
        let first = membership(rpc.read(node, "getrawmempool", json!([]))?)?;
        let mut conflict = false;
        for txid in &first {
            let raw = rpc.read(node, "getrawtransaction", json!([txid.to_string(), 0]))?;
            let bytes = hex::decode(raw.as_str().ok_or("raw mempool tx shape")?)
                .map_err(|e| e.to_string())?;
            let tx: Transaction = deserialize(&bytes).map_err(|e| e.to_string())?;
            if serialize(&tx) != bytes || tx.compute_txid() != *txid {
                return Err("mempool raw txid mismatch".into());
            }
            if *txid != own && tx.input.iter().any(|i| inputs.contains(&i.previous_output)) {
                conflict = true;
            }
        }
        if first != membership(rpc.read(node, "getrawmempool", json!([]))?)?
            || read_tip(rpc, node)? != self.anchor().block_hash
        {
            return Err("mempool/chain view changed".into());
        }
        Ok(!conflict)
    }

    /// Caller holds its actor-local serialization boundary for this entire
    /// method AND every send admission/recheck/forward. Snapshot tip is never
    /// a scan cursor: every invocation covers genesis through the live tip.
    pub fn observe(
        &self,
        wallet: &TrafficWalletSet,
        rpc: &mut impl RawRpc,
        node: &str,
        genesis: BlockHash,
        limit: Target,
        lookahead: u32,
    ) -> Result<(), String> {
        let snapshot = wallet.journal().observer_snapshot();
        let new = self.anchor();
        if read_tip(rpc, node)? != new.block_hash {
            return Err("observer anchor changed".into());
        }
        if snapshot.tip.as_ref() != Some(&new) {
            if let Some(old) = snapshot.tip {
                let old_index = Self::from_anchor(rpc, node, old.block_hash, genesis, limit)?;
                if old_index.anchor() != old {
                    return Err("old journal anchor height mismatch".into());
                }
                let common = old_index
                    .blocks
                    .iter()
                    .zip(&self.blocks)
                    .take_while(|(a, b)| a.block_hash() == b.block_hash())
                    .count()
                    .checked_sub(1)
                    .ok_or("no common genesis")?;
                let displaced: Vec<_> = old_index.blocks[common + 1..]
                    .iter()
                    .rev()
                    .map(Block::block_hash)
                    .collect();
                if new.height < old.height {
                    wallet
                        .journal()
                        .record_chain_rollback(RollbackEvidence {
                            version: 1,
                            old_anchor: old,
                            new_anchor: new.clone(),
                            common_ancestor: ChainAnchor {
                                block_hash: self.blocks[common].block_hash(),
                                height: common as u64,
                            },
                            old_headers: old_index.blocks[common..]
                                .iter()
                                .rev()
                                .map(|b| hex::encode(serialize(&b.header)))
                                .collect(),
                            new_headers: self.blocks[common..]
                                .iter()
                                .rev()
                                .map(|b| hex::encode(serialize(&b.header)))
                                .collect(),
                            displaced_block_hashes: displaced,
                        })
                        .map_err(|e| e.to_string())?;
                } else {
                    wallet
                        .note_chain_observed(new.block_hash, new.height, &displaced)
                        .map_err(|e| e.to_string())?;
                }
            } else {
                wallet
                    .note_chain_observed(new.block_hash, new.height, &[])
                    .map_err(|e| e.to_string())?;
            }
        }
        let mut scripts = BTreeMap::new();
        for role in wallet.roles() {
            rpc.checkpoint()?;
            let derivation = TrafficWallet::new(
                wallet.run_seed(),
                wallet.run_id(),
                role.clone(),
                wallet.network(),
                lookahead,
            )
            .map_err(|e| e.to_string())?;
            for chain in [Chain::Receive, Chain::Change] {
                let next = match chain {
                    Chain::Receive => wallet.journal().next_receive_index(&role),
                    Chain::Change => wallet.journal().next_change_index(&role),
                };
                let end = next
                    .checked_add(lookahead)
                    .ok_or("observer derivation overflow")?;
                if end > 100_000 {
                    return Err("observer derivation bound".into());
                }
                for index in 0..end {
                    if index % 16 == 0 {
                        rpc.checkpoint()?;
                    }
                    let script = derivation
                        .script_at(chain, index)
                        .map_err(|e| e.to_string())?
                        .script_pubkey;
                    if scripts
                        .insert(script, (role.clone(), chain, index))
                        .is_some()
                    {
                        return Err("ambiguous owned script".into());
                    }
                }
            }
        }
        let known = wallet.journal().observer_snapshot().outpoints;
        for (height, block) in self.blocks.iter().enumerate() {
            for tx in &block.txdata {
                rpc.checkpoint()?;
                let txid = tx.compute_txid();
                let own = wallet.journal().tx_facts(&txid);
                if let Some(facts) = &own {
                    if facts.raw_hex != hex::encode(serialize(tx)) {
                        return Err("own canonical bytes mismatch".into());
                    }
                    if matches!(
                        facts.state,
                        TxState::Signed | TxState::BroadcastSent | TxState::Invalidated
                    ) {
                        continue;
                    }
                    if facts.state != TxState::Confirmed {
                        wallet
                            .note_confirmed(&facts.command_id, block.block_hash(), height as u64)
                            .map_err(|e| e.to_string())?;
                    }
                }
                for (vout, output) in tx.output.iter().enumerate() {
                    let Some((owner, chain, index)) = scripts.get(&output.script_pubkey) else {
                        continue;
                    };
                    let outpoint = OutPoint {
                        txid,
                        vout: vout as u32,
                    };
                    if let Some(existing) = known.get(&outpoint) {
                        if existing.amount_sats != output.value.to_sat()
                            || existing.script != output.script_pubkey
                            || existing.owner != *owner
                            || existing.chain != *chain
                            || existing.index != *index
                        {
                            return Err("funding identity drift".into());
                        }
                        if existing.source == FundingSource::OwnTransaction
                            || !matches!(existing.status, OutpointStatus::Invalidated { .. })
                        {
                            continue;
                        }
                    }
                    // New coinbase credits are T2-only. A previously verified
                    // T2 credit may be re-observed after canonical restoration;
                    // identity/value/locus were compared above, with no mint.
                    if tx.is_coinbase() {
                        if let Some(existing) = known.get(&outpoint).filter(|f| {
                            f.source == FundingSource::BootstrapCoinbase
                                && matches!(f.status, OutpointStatus::Invalidated { .. })
                        }) {
                            wallet
                                .observe_funding(&FundingObservation {
                                    outpoint,
                                    amount_sats: existing.amount_sats,
                                    chain: existing.chain,
                                    index: existing.index,
                                    owner_role: existing.owner.clone(),
                                    source: FundingSource::BootstrapCoinbase,
                                    context: FundingContext::CanonicalBlock {
                                        block_hash: block.block_hash().to_string(),
                                        height: height as u64,
                                        coinbase: true,
                                    },
                                })
                                .map_err(|e| e.to_string())?;
                        }
                        continue;
                    }
                    let (source, context) = if own.is_some() {
                        (
                            FundingSource::OwnTransaction,
                            FundingContext::OwnTransaction,
                        )
                    } else {
                        (
                            FundingSource::CanonicalBlock,
                            FundingContext::CanonicalBlock {
                                block_hash: block.block_hash().to_string(),
                                height: height as u64,
                                coinbase: false,
                            },
                        )
                    };
                    wallet
                        .observe_funding(&FundingObservation {
                            outpoint,
                            amount_sats: output.value.to_sat(),
                            chain: *chain,
                            index: *index,
                            owner_role: owner.clone(),
                            source,
                            context,
                        })
                        .map_err(|e| e.to_string())?;
                }
            }
        }
        Ok(())
    }
}
fn read_tip(rpc: &mut impl RawRpc, node: &str) -> Result<BlockHash, String> {
    BlockHash::from_str(
        rpc.read(node, "getbestblockhash", json!([]))?
            .as_str()
            .ok_or("tip shape")?,
    )
    .map_err(|e| e.to_string())
}
