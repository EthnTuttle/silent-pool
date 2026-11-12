//! Native, legacy-only bootstrap. The actor owns every key and spend; its
//! controller owns every hash debit. This module holds neither RPC credentials
//! nor a signing wallet. Private evidence is observation, never authorization.
use crate::actor_config::ActorConfig;
use crate::bootstrap::{
    boot_id, monotonic_ms, BatchConsumption, BootstrapCompletion, BootstrapExecutor, BootstrapHost,
    BootstrapRequest, HashAuthorization, HashReservationStatus, OperationControl,
};
use crate::control::RpcOutcome;
use crate::keys::{Chain, TrafficWallet, WalletRole};
use crate::observer::{CanonicalIndex, ChainPolicy, RawRpc};
use bitcoin::consensus::{deserialize, serialize};
use bitcoin::hashes::Hash;
use bitcoin::{Block, BlockHash, OutPoint, ScriptBuf, Target, Transaction, TxIn, TxOut, Txid};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use spc_cpu_miner::budget::{HashBudget, Pacer};
use spc_cpu_miner::hash::{sha256d, HeaderPreimage};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const BATCH_MAX: u64 = 16;
const POLL: Duration = Duration::from_millis(25);
const MAX_RAW_BYTES: usize = 4_000_000;
const MAX_EVIDENCE_BYTES: u64 = 256 * 1024 * 1024;

/// Values come from the validated scenario and the actual launched actor.
/// Rates and the absolute deadline are read back from the durable budget.
pub struct NativeBootstrapExecutor {
    config: ActorConfig,
    incarnation: String,
    funding_blocks: u32,
    outputs_per_wallet: u32,
    started: Mutex<bool>,
}

impl NativeBootstrapExecutor {
    /// Compile-time source bytes, recorded privately with the executing binary.
    pub fn source_sha256() -> String {
        use sha2::Digest;
        hex::encode(sha2::Sha256::digest(include_bytes!("native_bootstrap.rs")))
    }
    pub fn new(
        config: ActorConfig,
        incarnation: String,
        funding_blocks: u32,
        outputs_per_wallet: u32,
    ) -> Result<Self, String> {
        config.validate().map_err(|e| e.to_string())?;
        validate_policy(&config.chain_policy)?;
        if incarnation.is_empty()
            || !(101..=1000).contains(&funding_blocks)
            || !(1..=64).contains(&outputs_per_wallet)
            || !config
                .allocation
                .actors
                .iter()
                .any(|a| a.driver_id == config.driver_id && a.hosts_bootstrap_funding)
        {
            return Err("native_bootstrap_config_invalid".into());
        }
        Ok(Self {
            config,
            incarnation,
            funding_blocks,
            outputs_per_wallet,
            started: Mutex::new(false),
        })
    }
}

impl BootstrapExecutor for NativeBootstrapExecutor {
    fn execute(
        &self,
        request: BootstrapRequest,
        control: OperationControl,
        host: Arc<dyn BootstrapHost>,
    ) -> Result<BootstrapCompletion, String> {
        if request.driver_id != self.config.driver_id
            || request.node_id != self.config.authority_node
        {
            return Err("native_bootstrap_request_binding".into());
        }
        // Re-execution is not a resume mechanism. A lost/old permit can never
        // restart this producer. The controller keeps unresolved debit charged.
        {
            let mut started = self.started.lock().map_err(|_| "bootstrap_poisoned")?;
            if *started {
                return Err("native_bootstrap_already_started".into());
            }
            *started = true;
        }
        let mut rpc = HostRpc {
            host,
            request,
            control,
        };
        rpc.checkpoint()?;
        let snapshot: BudgetSnapshot = decode(rpc.call("budget_status", json!({}))?)?;
        snapshot.validate(&rpc.request)?;
        // Outstanding authorization belongs to its original execution. No
        // inference of zero hashes, refund, or redemption after a restart.
        if snapshot.outstanding.is_some() {
            return Err("native_bootstrap_unresolved_reservation".into());
        }
        let root = self.config.root().map_err(|e| e.to_string())?;
        validate_evidence_root(&self.config.run_dir, &root)?;
        let evidence = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(root.join("native-bootstrap.jsonl"))
            .map_err(|e| format!("bootstrap_evidence_create: {e}"))?;
        let mut run = Run {
            rpc,
            config: &self.config,
            evidence,
            budget: HashBudget {
                hashes_per_second: Some(snapshot.operation.hashes_per_second),
                total_attempts: Some(snapshot.operation.total_attempts),
            },
            cumulative_reserved: snapshot.cumulative_reserved,
            next_batch: snapshot.next_batch_id,
            consumption: BatchConsumption::new(
                snapshot.operation.operation_id,
                self.incarnation.clone(),
            ),
            pacer: Pacer::new(Instant::now()),
            entropy: 0,
            parent_context: None,
        };
        run.budget.validate().map_err(|e| e.to_string())?;
        run.pacer.available_attempts(&run.budget, Instant::now());
        run.pacer.pause(Instant::now());
        run.record(json!({"event":"begin", "request":run.rpc.request,
            "incarnation":self.incarnation,"budget":run.budget,
            "prior_reserved":run.cumulative_reserved,"policy":self.config.chain_policy,
            "funding_blocks":self.funding_blocks,"outputs_per_wallet":self.outputs_per_wallet}))?;
        let result = run.fund(self.funding_blocks, self.outputs_per_wallet);
        run.record(json!({"event":"terminal","success":result.is_ok(),
            "error":result.as_ref().err(),"measured_attempts":run.pacer.attempted(),
            "acknowledged_cumulative_reserved":run.cumulative_reserved,
            "debit_authority":"controller_command_log"}))?;
        result
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BudgetOperation {
    operation_id: String,
    driver_id: String,
    boot_id: String,
    deadline_monotonic_ms: u64,
    total_attempts: u64,
    hashes_per_second: u64,
    cancel_grace_secs: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BudgetSnapshot {
    operation: BudgetOperation,
    cumulative_reserved: u64,
    next_batch_id: u64,
    outstanding: Option<HashAuthorization>,
}
impl BudgetSnapshot {
    fn validate(&self, request: &BootstrapRequest) -> Result<(), String> {
        let op = &self.operation;
        let now = monotonic_ms().map_err(|e| e.to_string())?;
        if op.operation_id != request.command_id
            || op.driver_id != request.driver_id
            || op.boot_id != request.boot_id
            || op.deadline_monotonic_ms != request.deadline_monotonic_ms
            || op.cancel_grace_secs != request.cancel_grace_secs
            || op.cancel_grace_secs == 0
            || op.cancel_grace_secs > 30
            || op.hashes_per_second == 0
            || op.total_attempts == 0
            || op.total_attempts > 4_194_304
            || op.deadline_monotonic_ms <= now
            || op.deadline_monotonic_ms - now > 3_600_000
            || self.cumulative_reserved > op.total_attempts
            || self.next_batch_id == 0
        {
            return Err("native_bootstrap_budget_binding".into());
        }
        Ok(())
    }
}

struct HostRpc {
    host: Arc<dyn BootstrapHost>,
    request: BootstrapRequest,
    control: OperationControl,
}
impl HostRpc {
    fn live(&self) -> Result<(), String> {
        if self.control.cancelled() {
            return Err("bootstrap_cancelled".into());
        }
        if boot_id().map_err(|e| e.to_string())? != self.request.boot_id
            || monotonic_ms().map_err(|e| e.to_string())? >= self.request.deadline_monotonic_ms
        {
            return Err("bootstrap_deadline_or_boot_changed".into());
        }
        Ok(())
    }
    fn call(&mut self, operation: &str, payload: Value) -> Result<Value, String> {
        self.checkpoint()?;
        // The T1 pipe and node RPC each have bounded waits. Never retry an
        // uncertain mutating request here: retain its journal/evidence outcome.
        let value = self.host.actor_request(operation, payload)?;
        if value["status"].as_str() == Some("failed") {
            return Err(format!("bootstrap_actor_failed: {}", value["code"]));
        }
        Ok(value)
    }
    fn outcome(&mut self, node: &str, method: &str, params: Value) -> Result<RpcOutcome, String> {
        let value = self.call(
            "bootstrap_rpc",
            json!({"node_id":node,"method":method,"params":params}),
        )?;
        if value["status"].as_str() != Some("rpc_result") {
            return Err("bootstrap_rpc_envelope".into());
        }
        decode(value["result"].clone())
    }
}
impl RawRpc for HostRpc {
    fn read(&mut self, node: &str, method: &str, params: Value) -> Result<Value, String> {
        match self.outcome(node, method, params)? {
            RpcOutcome::Ok { result } => Ok(result),
            other => Err(format!(
                "bootstrap_rpc_unavailable: {}",
                serde_json::to_string(&other).map_err(|e| e.to_string())?
            )),
        }
    }
    fn checkpoint(&mut self) -> Result<(), String> {
        loop {
            self.live()?;
            if !self.control.paused() {
                return Ok(());
            }
            std::thread::sleep(POLL);
        }
    }
}

struct Run<'a> {
    rpc: HostRpc,
    config: &'a ActorConfig,
    evidence: File,
    budget: HashBudget,
    cumulative_reserved: u64,
    next_batch: u64,
    consumption: BatchConsumption,
    pacer: Pacer,
    entropy: u64,
    parent_context: Option<crate::contextual_pow::VerifiedHeaders>,
}
impl Run<'_> {
    fn record(&mut self, mut value: Value) -> Result<(), String> {
        value["source"] = json!("bootstrap");
        value["exclude_from_statistics"] = json!(true);
        value["monotonic_ms"] = json!(monotonic_ms().map_err(|e| e.to_string())?);
        let mut bytes = serde_json::to_vec(&value).map_err(|e| e.to_string())?;
        bytes.push(b'\n');
        if self
            .evidence
            .metadata()
            .map_err(|e| e.to_string())?
            .len()
            .checked_add(bytes.len() as u64)
            .is_none_or(|total| total > MAX_EVIDENCE_BYTES)
        {
            return Err("bootstrap_evidence_byte_bound".into());
        }
        self.evidence
            .write_all(&bytes)
            .and_then(|()| self.evidence.flush())
            .and_then(|()| self.evidence.sync_data())
            .map_err(|e| format!("bootstrap_evidence_write: {e}"))
    }
    fn checkpoint(&mut self) -> Result<(), String> {
        // Also pause the shared pacer while RPCs/observer work run, so waiting
        // cannot bank a burst. Absolute boot-bound deadline always advances.
        if self.rpc.control.paused() {
            self.pacer.pause(Instant::now());
            self.rpc.checkpoint()?;
        }
        self.rpc.live()
    }
    fn common_index(&mut self) -> Result<CanonicalIndex, String> {
        loop {
            match self.common_index_once() {
                Err(error)
                    if matches!(
                        error.as_str(),
                        "bootstrap_common_chain_inconclusive"
                            | "bootstrap_common_anchor_changed"
                            | "canonical anchor changed mid-scan"
                    ) =>
                {
                    self.checkpoint()?;
                    std::thread::sleep(POLL);
                }
                result => return result,
            }
        }
    }
    fn common_index_once(&mut self) -> Result<CanonicalIndex, String> {
        self.pacer.pause(Instant::now());
        self.checkpoint()?;
        let policy = &self.config.chain_policy;
        let index = CanonicalIndex::scan(
            &mut self.rpc,
            &self.config.authority_node,
            policy.genesis,
            policy.pow_limit()?,
        )?;
        index.verify_policy(policy)?;
        for node in &self.config.node_ids {
            if node == &self.config.authority_node {
                continue;
            }
            let other =
                CanonicalIndex::scan(&mut self.rpc, node, policy.genesis, policy.pow_limit()?)?;
            other.verify_policy(policy)?;
            if !index.corroborates(&other) {
                return Err("bootstrap_common_chain_inconclusive".into());
            }
        }
        // Reject an authority advance during corroboration, rather than
        // alternating node views into the one journal tip.
        if self
            .rpc
            .read(&self.config.authority_node, "getbestblockhash", json!([]))?
            .as_str()
            != Some(index.anchor().block_hash.to_string().as_str())
        {
            return Err("bootstrap_common_anchor_changed".into());
        }
        Ok(index)
    }
    fn fund(
        &mut self,
        funding_blocks: u32,
        outputs_per_wallet: u32,
    ) -> Result<BootstrapCompletion, String> {
        let initial = self.common_index()?;
        if initial.anchor().height != 0 {
            return Err("native_bootstrap_requires_fresh_chain".into());
        }
        let mut mined = BTreeMap::new();
        for _ in 0..funding_blocks {
            let (block, height, index) = self.mine_block()?;
            mined.insert(
                block.txdata[0].compute_txid(),
                (height, index, block.txdata[0].output[0].clone()),
            );
        }
        let canonical = self.common_index()?;
        let tip = canonical.anchor();
        let mut mature = Vec::new();
        for (txid, (height, _, output)) in &mined {
            if canonical
                .transactions
                .get(txid)
                .is_none_or(|(actual_height, tx)| {
                    actual_height != height
                        || !tx.is_coinbase()
                        || tx.output.len() != 1
                        || tx.output.first() != Some(output)
                })
            {
                return Err("bootstrap_funding_displaced_before_distribution".into());
            }
            if is_mature(*height, tip.height)?
                && canonical.unspent.get(&OutPoint {
                    txid: *txid,
                    vout: 0,
                }) == Some(output)
            {
                mature.push(output.value.to_sat());
            }
        }
        // Use a conservative ONE mature input's value to budget the entire
        // declared sequence before its first send. Real change and surplus
        // remain in the same funding wallet; they are never rounded into fees.
        let available = mature
            .into_iter()
            .min()
            .ok_or("bootstrap_no_mature_funding")?;
        let plan = DistributionPlan::new(self.config, outputs_per_wallet, available)?;
        if u64::from(funding_blocks)
            .checked_add(plan.parts.len() as u64)
            .and_then(|n| n.checked_add(1))
            .is_none_or(|n| n > 4096)
        {
            return Err("bootstrap_distribution_chain_bound".into());
        }
        self.record(
            json!({"event":"distribution_plan","tip":tip,"parts":plan.parts,
            "output_count":plan.count,"amount_per_output":plan.amount,
            "fee_reserve":plan.fee_reserve,"mature_budget":available}),
        )?;
        let mut txids = Vec::new();
        for (part, outputs) in plan.parts.iter().enumerate() {
            self.checkpoint()?;
            self.pacer.pause(Instant::now());
            let result = loop {
                let result = self.rpc.call(
                    "distribute",
                    json!({"part":part as u32,
                    "outputs":outputs,"fee_rate_sats_vb":plan.rate}),
                )?;
                self.record(json!({"event":"distribution_result","part":part,"result":result}))?;
                if result["status"].as_str() != Some("deferred") {
                    break result;
                }
                // T1's explicit deferred result precedes the send intent.
                // Replay the identical part/command, never an unknown send.
                self.checkpoint()?;
                std::thread::sleep(POLL);
            };
            let txid = parse_txid(&result["txid"])?;
            if !matches!(
                result["status"].as_str(),
                Some("accepted_by_rpc" | "unknown" | "stored")
            ) {
                return Err("bootstrap_distribution_not_admitted".into());
            }
            // Unknown remains unknown until actual raw canonical inclusion.
            // Do not rebroadcast or create a replacement payment here.
            let mut found = None;
            while found.is_none() {
                let (block, height, receive_index) = self.mine_block()?;
                mined.insert(
                    block.txdata[0].compute_txid(),
                    (height, receive_index, block.txdata[0].output[0].clone()),
                );
                let index = self.common_index()?;
                if let Some((height, tx)) = index.transactions.get(&txid) {
                    verify_distribution_part(
                        &index,
                        tx,
                        *height,
                        outputs,
                        self.config,
                        &mined,
                        part as u32 + 1,
                        &mut self.rpc,
                    )?;
                    found = Some(index.anchor());
                }
            }
            txids.push(txid.to_string());
            // Ask the same signer observer to discover its confirmed change
            // before the next part. The next distribute also observes freshly.
            self.rpc.call("observe", json!({}))?;
        }
        let common = self.common_index()?;
        for (part, id) in txids.iter().enumerate() {
            let id = Txid::from_str(id).map_err(|e| e.to_string())?;
            let (height, tx) = common
                .transactions
                .get(&id)
                .ok_or("bootstrap_distribution_displaced")?;
            verify_distribution_part(
                &common,
                tx,
                *height,
                &plan.parts[part],
                self.config,
                &mined,
                txids.len() as u32,
                &mut self.rpc,
            )?;
            for (vout, output) in tx.output.iter().enumerate().take(plan.parts[part].len()) {
                if common.unspent.get(&OutPoint {
                    txid: id,
                    vout: vout as u32,
                }) != Some(output)
                {
                    return Err("bootstrap_distribution_already_spent".into());
                }
            }
        }
        let tip = common.anchor();
        self.record(json!({"event":"distribution_canonical","txids":txids,"tip":tip}))?;
        Ok(BootstrapCompletion {
            distribution_txids: txids,
            common_tip_hash: tip.block_hash.to_string(),
            common_tip_height: tip.height,
        })
    }

    fn mine_block(&mut self) -> Result<(Block, u64, u32), String> {
        self.pacer.pause(Instant::now());
        let address = self.rpc.call("funding_address", json!({}))?;
        if address["status"].as_str() != Some("address") {
            return Err("bootstrap_address_shape".into());
        }
        let index = integer_u32(&address, "index")?;
        let script = ScriptBuf::from_bytes(hex_bytes(&address["script_hex"], None)?);
        if !script.is_p2pkh() {
            return Err("bootstrap_funding_not_p2pkh".into());
        }
        self.entropy = self
            .entropy
            .checked_add(1)
            .ok_or("bootstrap_entropy_overflow")?;
        let mut current: Option<Template> = None;
        let mut nonce = 0u32;
        loop {
            self.pacer.pause(Instant::now());
            self.checkpoint()?;
            let node = self.config.authority_node.clone();
            let tip = self.rpc.read(&node, "getbestblockhash", json!([]))?;
            let tip_hash = parse_blockhash(&tip)?;
            let header = self
                .rpc
                .read(&node, "getblockheader", json!([tip_hash.to_string()]))?;
            let height = integer(&header, "height")?
                .checked_add(1)
                .ok_or("bootstrap_height_overflow")?;
            if self
                .parent_context
                .as_ref()
                .is_none_or(|context| context.tip() != tip_hash)
            {
                let policy = &self.config.chain_policy;
                let index = CanonicalIndex::scan(
                    &mut self.rpc,
                    &node,
                    policy.genesis,
                    policy.pow_limit()?,
                )?;
                index.verify_policy(policy)?;
                self.parent_context = Some(crate::contextual_pow::VerifiedHeaders::new(
                    policy,
                    index.blocks.iter().map(|block| block.header).collect(),
                )?);
            }
            let context = self
                .parent_context
                .as_ref()
                .ok_or("bootstrap_parent_context_missing")?;
            if context.tip() != tip_hash || context.next_height() != height {
                return Err("bootstrap_raw_parent_anchor_changed".into());
            }
            let value = self.rpc.read(
                &node,
                "getblocktemplate",
                json!([{"mode":"template","rules":["segwit"]}]),
            )?;
            let template = match Template::parse(&value, &self.config.chain_policy) {
                Ok(template) => template,
                Err(error) => {
                    self.record(serde_json::json!({"event":"template_refused","error":error,"template":value}))?;
                    return Err(error);
                }
            };
            if template.prev != tip_hash || template.height != height {
                self.record(
                    json!({"event":"template_stale_before_work","height":template.height}),
                )?;
                continue;
            }
            if let Err(error) = template.verify_context(
                self.parent_context
                    .as_ref()
                    .ok_or("bootstrap_parent_context_missing")?,
            ) {
                self.record(json!({"event":"template_refused","error":error,"template":value}))?;
                return Err(error);
            }
            if current.as_ref() != Some(&template) {
                self.entropy = self
                    .entropy
                    .checked_add(1)
                    .ok_or("bootstrap_entropy_overflow")?;
                nonce = 0;
                self.record(json!({"event":"template","height":height,"bits":template.bits,
                    "target":hex::encode(template.target.to_be_bytes()),"previous":template.prev,
                    "coinbasevalue":template.revenue,"txids":template.transactions.iter().map(Transaction::compute_txid).collect::<Vec<_>>()}))?;
            }
            let coinbase = coinbase(template.height, self.entropy, &script, template.revenue)?;
            let cb = serialize(&coinbase);
            let path = coinbase_path(&template.transactions);
            let preimage = HeaderPreimage::new(
                template.version,
                template.prev.to_byte_array(),
                &cb,
                &path,
                template.time,
                template.bits,
            );
            let target = template.target;
            current = Some(template);
            self.pacer.resume(Instant::now());
            let attempts = loop {
                self.checkpoint()?;
                self.pacer.resume(Instant::now());
                let remaining = self.budget.remaining(self.cumulative_reserved);
                if remaining == 0 {
                    return Err("bootstrap_hash_budget_exhausted".into());
                }
                let credit = self.pacer.available_attempts(&self.budget, Instant::now());
                let attempts = credit
                    .min(BATCH_MAX)
                    .min(remaining)
                    .min(u64::from(u32::MAX) - u64::from(nonce) + 1);
                if attempts > 0 {
                    break attempts;
                }
                let delay = self
                    .pacer
                    .required_delay(&self.budget, Instant::now())
                    .min(POLL);
                self.pacer.note_pace_sleep();
                std::thread::sleep(delay);
            };
            self.pacer.pause(Instant::now());
            let reserved = self.rpc.call(
                "reserve_hashes",
                json!({"operation_id":self.rpc.request.command_id,
                "batch_id":self.next_batch,"attempts":attempts}),
            )?;
            let status = validate_authorization(
                &reserved,
                self.next_batch,
                attempts,
                self.cumulative_reserved,
                &mut self.consumption,
            )?;
            self.cumulative_reserved = status.authorization.cumulative_reserved;
            self.next_batch = self
                .next_batch
                .checked_add(1)
                .ok_or("bootstrap_batch_overflow")?;
            self.pacer.resume(Instant::now());
            let mut headers = Vec::new();
            let mut solution = None;
            // Hash calls occur ONLY in this bounded loop, after both the actor
            // and local BatchConsumption accepted the full bound permit.
            for offset in 0..attempts {
                if self.rpc.control.cancelled()
                    || self.rpc.control.paused()
                    || self.rpc.live().is_err()
                {
                    break;
                }
                let n = nonce
                    .checked_add(offset as u32)
                    .ok_or("bootstrap_nonce_overflow")?;
                let (raw, hash) = preimage.hash_nonce(n);
                self.pacer.note_attempts(1);
                headers.push(hex::encode(raw));
                if target.is_met_by(hash) {
                    solution = Some(raw);
                    break;
                }
            }
            let measured = headers.len() as u64;
            self.pacer.pause(Instant::now());
            self.record(
                json!({"event":"hash_batch","authorization":status.authorization,
                "measured_attempts":measured,"raw_headers":headers,
                "target":hex::encode(target.to_be_bytes()),"height":height}),
            )?;
            // On loss/cancel the full reservation remains charged. Persisted
            // actual headers remain distinct from an unresolved debit.
            let reconciled = self.rpc.host.actor_request(
                "reconcile_hashes",
                json!({
                "authorization":status.authorization,"measured_attempts":measured}),
            )?;
            if reconciled["status"].as_str() != Some("reconciled") {
                return Err("bootstrap_reconcile_unacknowledged".into());
            }
            self.checkpoint()?;
            let template = current.as_ref().ok_or("bootstrap_template_missing")?;
            if let Some(raw) = solution {
                let mut txdata = vec![coinbase];
                txdata.extend(template.transactions.clone());
                let block = Block {
                    header: deserialize(&raw).map_err(|e| e.to_string())?,
                    txdata,
                };
                verify_raw_block(
                    &serialize(&block),
                    block.block_hash(),
                    &self.config.chain_policy,
                )?;
                self.pacer.pause(Instant::now());
                let outcome = self.rpc.outcome(
                    &node,
                    "submitblock",
                    json!([hex::encode(serialize(&block))]),
                );
                self.record(json!({"event":"submitblock","block_hash":block.block_hash(),
                    "raw_block":hex::encode(serialize(&block)),"outcome":outcome.as_ref().ok(),"transport_error":outcome.as_ref().err()}))?;
                // A null answer is not evidence; errors likewise do not prove
                // absence. Observe every outcome before crediting anything.
                let canonical = self.common_index()?;
                if canonical
                    .blocks
                    .get(height as usize)
                    .is_some_and(|b| serialize(b) == serialize(&block))
                {
                    self.verify_and_observe(&block, height, index, &script)?;
                    return Ok((block, height, index));
                }
                self.record(json!({"event":"submission_not_canonical","block_hash":block.block_hash(),"tip":canonical.anchor()}))?;
                // Never reinterpret unknown as rejection or refund its work.
                if !matches!(outcome,Ok(RpcOutcome::Ok {result}) if result.is_null() || result.is_string())
                {
                    return Err("bootstrap_submission_uncertain".into());
                }
                current = None;
            } else {
                match nonce.checked_add(measured as u32) {
                    Some(next) => nonce = next,
                    None => {
                        current = None;
                        self.entropy = self
                            .entropy
                            .checked_add(1)
                            .ok_or("bootstrap_entropy_overflow")?;
                    }
                }
            }
        }
    }

    fn verify_and_observe(
        &mut self,
        block: &Block,
        height: u64,
        index: u32,
        script: &ScriptBuf,
    ) -> Result<(), String> {
        let hash = block.block_hash();
        for node in &self.config.node_ids {
            let value = self
                .rpc
                .read(node, "getblock", json!([hash.to_string(), 0]))?;
            let bytes = hex_bytes(&value, None)?;
            let fetched = verify_raw_block(&bytes, hash, &self.config.chain_policy)?;
            let header = self
                .rpc
                .read(node, "getblockheader", json!([hash.to_string()]))?;
            if serialize(&fetched) != serialize(block)
                || fetched.txdata[0].output.len() != 1
                || fetched.txdata[0].output[0].script_pubkey != *script
                || integer(&header, "height")? != height
                || integer(&header, "confirmations")? == 0
                || self
                    .rpc
                    .read(node, "getblockhash", json!([height]))?
                    .as_str()
                    != Some(hash.to_string().as_str())
            {
                return Err("bootstrap_funding_canonical_inconclusive".into());
            }
        }
        let observed = self.rpc.call(
            "observe_bootstrap_coinbase",
            json!({"block_hash":hash,"index":index}),
        )?;
        if observed["status"].as_str() != Some("funding_observed") {
            return Err("bootstrap_funding_observation_failed".into());
        }
        self.record(json!({"event":"coinbase_canonical","block_hash":hash,"height":height,
            "txid":block.txdata[0].compute_txid(),"vout":0,"index":index,"amount_sats":block.txdata[0].output[0].value.to_sat()}))
    }
}

fn validate_evidence_root(run: &std::path::Path, root: &std::path::Path) -> Result<(), String> {
    let uid = std::fs::metadata("/proc/self")
        .map_err(|e| e.to_string())?
        .uid();
    for path in [
        run.join("private"),
        run.join("private/traffic"),
        root.to_path_buf(),
    ] {
        let meta = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        if !meta.is_dir() || meta.mode() & 0o777 != 0o700 || meta.uid() != uid {
            return Err("bootstrap_evidence_root_unsafe".into());
        }
    }
    Ok(())
}

fn validate_authorization(
    value: &Value,
    batch: u64,
    attempts: u64,
    cumulative: u64,
    consumption: &mut BatchConsumption,
) -> Result<HashReservationStatus, String> {
    let status: HashReservationStatus = decode(value["reservation"].clone())?;
    if value["status"].as_str() != Some("authorized")
        || value["execute"].as_bool() != Some(true)
        || status.authorization.batch_id != batch
        || status.authorization.reserved_attempts != attempts
        || status.authorization.cumulative_reserved
            != cumulative
                .checked_add(attempts)
                .ok_or("bootstrap_debit_overflow")?
        || status.measured_attempts.is_some()
        || !consumption.consume(&status)?
    {
        return Err("bootstrap_hash_authorization_refused".into());
    }
    Ok(status)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Template {
    version: u32,
    height: u64,
    prev: BlockHash,
    bits: u32,
    target: Target,
    time: u32,
    revenue: u64,
    transactions: Vec<Transaction>,
}
impl Template {
    fn parse(value: &Value, policy: &ChainPolicy) -> Result<Self, String> {
        validate_policy(policy)?;
        let version = integer_u32(value, "version")?;
        let height = integer(value, "height")?;
        if height == 0 || height > u64::from(u32::MAX) {
            return Err("bootstrap_height_invalid".into());
        }
        let bits_bytes = hex_bytes(&value["bits"], Some(4))?;
        let bits = u32::from_be_bytes(bits_bytes.try_into().map_err(|_| "bootstrap_bits_shape")?);
        let target = compact_target(bits, policy.pow_limit()?)?;
        let target_bytes: [u8; 32] = hex_bytes(&value["target"], Some(32))?
            .try_into()
            .map_err(|_| "bootstrap_target_shape")?;
        if target != Target::from_be_bytes(target_bytes) {
            return Err("bootstrap_template_target_mismatch".into());
        }
        let prev = parse_blockhash(&value["previousblockhash"])?;
        let minimum = integer_u32(value, "mintime")?;
        let time = integer_u32(value, "curtime")?;
        if minimum > time {
            return Err("bootstrap_time_range".into());
        }
        let rows = value["transactions"]
            .as_array()
            .ok_or("bootstrap_transactions_shape")?;
        if rows.len() > 16_384 {
            return Err("bootstrap_transaction_count_bound".into());
        }
        let mut transactions = Vec::new();
        let mut fees = 0u64;
        let mut bytes = 0usize;
        let mut ids = BTreeSet::new();
        for row in rows {
            let raw = hex_bytes(&row["data"], None)?;
            bytes = bytes
                .checked_add(raw.len())
                .ok_or("bootstrap_template_size_overflow")?;
            if bytes > MAX_RAW_BYTES {
                return Err("bootstrap_template_size_bound".into());
            }
            let tx = strict_transaction(&raw, parse_txid(&row["txid"])?)?;
            if tx.is_coinbase() || !ids.insert(tx.compute_txid()) {
                return Err("bootstrap_template_transaction_invalid".into());
            }
            if tx.input.iter().any(|i| !i.witness.is_empty()) {
                return Err("bootstrap_witness_template_unsupported".into());
            }
            fees = fees
                .checked_add(integer(row, "fee")?)
                .ok_or("bootstrap_fee_overflow")?;
            transactions.push(tx);
        }
        let revenue = subsidy(policy, height)?
            .checked_add(fees)
            .ok_or("bootstrap_revenue_overflow")?;
        if revenue != integer(value, "coinbasevalue")? {
            return Err("bootstrap_coinbasevalue_mismatch".into());
        }
        Ok(Self {
            version,
            height,
            prev,
            bits,
            target,
            time,
            revenue,
            transactions,
        })
    }
    fn verify_context(
        &self,
        context: &crate::contextual_pow::VerifiedHeaders,
    ) -> Result<(), String> {
        let expected = context.expected_next_bits(self.height, self.prev, self.time)?;
        if self.bits != expected {
            return Err(format!(
                "bootstrap_target_profile_mismatch: gbt_bits={:08x} expected={expected:08x}",
                self.bits
            ));
        }
        Ok(())
    }
}

fn validate_policy(policy: &ChainPolicy) -> Result<(), String> {
    crate::contextual_pow::validate_policy(policy)
}
fn compact_target(bits: u32, limit: Target) -> Result<Target, String> {
    let size = bits >> 24;
    let word = bits & 0x007f_ffff;
    let target = Target::from_compact(bitcoin::CompactTarget::from_consensus(bits));
    if bits & 0x0080_0000 != 0
        || word == 0
        || size > 34
        || (word > 0xff && size > 33)
        || (word > 0xffff && size > 32)
        || target == Target::ZERO
        || target > limit
    {
        return Err("bootstrap_compact_target_invalid".into());
    }
    Ok(target)
}
fn subsidy(policy: &ChainPolicy, height: u64) -> Result<u64, String> {
    if policy.subsidy_halving_interval == 0 {
        return Err("bootstrap_halving_interval_zero".into());
    }
    let halvings = height / policy.subsidy_halving_interval;
    Ok(if halvings >= 64 {
        0
    } else {
        policy.subsidy_initial_sats >> halvings
    })
}
fn is_mature(created: u64, tip: u64) -> Result<bool, String> {
    Ok(created != 0
        && tip
            >= created
                .checked_add(100)
                .ok_or("bootstrap_maturity_overflow")?)
}
fn strict_transaction(raw: &[u8], txid: Txid) -> Result<Transaction, String> {
    let tx: Transaction = deserialize(raw).map_err(|e| e.to_string())?;
    if tx.input.is_empty() || serialize(&tx) != raw || tx.compute_txid() != txid {
        return Err("bootstrap_raw_transaction_mismatch".into());
    }
    Ok(tx)
}
fn coinbase(
    height: u64,
    nonce: u64,
    script: &ScriptBuf,
    value: u64,
) -> Result<Transaction, String> {
    if height == 0 || height > u64::from(u32::MAX) || !script.is_p2pkh() {
        return Err("bootstrap_coinbase_arguments".into());
    }
    let script_sig = bitcoin::script::Builder::new()
        .push_int(height as i64)
        .push_slice(nonce.to_le_bytes())
        .into_script();
    Ok(Transaction {
        version: bitcoin::transaction::Version::ONE,
        lock_time: bitcoin::absolute::LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint::null(),
            script_sig,
            sequence: bitcoin::Sequence::MAX,
            witness: bitcoin::Witness::new(),
        }],
        output: vec![TxOut {
            value: bitcoin::Amount::from_sat(value),
            script_pubkey: script.clone(),
        }],
    })
}
fn coinbase_path(transactions: &[Transaction]) -> Vec<[u8; 32]> {
    // A placeholder for the coinbase leaf: siblings of the leftmost path do
    // not depend on its value. Every other leaf is a txid, never a wtxid.
    let mut level = vec![[0; 32]];
    level.extend(
        transactions
            .iter()
            .map(|t| t.compute_txid().to_byte_array()),
    );
    let mut path = Vec::new();
    while level.len() > 1 {
        if level.len() % 2 == 1 {
            level.push(*level.last().expect("nonempty"));
        }
        path.push(level[1]);
        level = level
            .chunks_exact(2)
            .map(|p| {
                let mut pair = [0; 64];
                pair[..32].copy_from_slice(&p[0]);
                pair[32..].copy_from_slice(&p[1]);
                sha256d(&pair)
            })
            .collect();
    }
    path
}
fn verify_raw_block(raw: &[u8], hash: BlockHash, policy: &ChainPolicy) -> Result<Block, String> {
    if raw.len() > MAX_RAW_BYTES {
        return Err("bootstrap_raw_block_bound".into());
    }
    let block: Block = deserialize(raw).map_err(|e| e.to_string())?;
    let target = compact_target(block.header.bits.to_consensus(), policy.pow_limit()?)?;
    if serialize(&block) != raw
        || block.block_hash() != hash
        || !block.check_merkle_root()
        || block.header.validate_pow(target).is_err()
        || block.txdata.is_empty()
        || !block.txdata[0].is_coinbase()
        || block.txdata.iter().skip(1).any(Transaction::is_coinbase)
        || block
            .txdata
            .iter()
            .any(|t| t.input.iter().any(|i| !i.witness.is_empty()))
    {
        return Err("bootstrap_raw_block_invalid".into());
    }
    Ok(block)
}

#[derive(Clone, Serialize)]
struct DistributionOutput {
    role: WalletRole,
    receive_index: u32,
    amount_sats: u64,
}
struct DistributionPlan {
    parts: Vec<Vec<DistributionOutput>>,
    count: u64,
    amount: u64,
    fee_reserve: u64,
    rate: u64,
}
impl DistributionPlan {
    fn new(config: &ActorConfig, per_wallet: u32, available: u64) -> Result<Self, String> {
        let host = config
            .allocation
            .actors
            .iter()
            .find(|a| a.driver_id == config.driver_id)
            .ok_or("bootstrap_host_missing")?;
        let policy = &host.fee_policy;
        // One P2PKH input (<=149 bytes), change and CompactSize overhead.
        // Leave additional room for the core's conservative selection bound.
        let capacity = config
            .max_vbytes
            .min(policy.max_fee_sats_per_tx / policy.min_rate_sats_vb)
            .checked_sub(256)
            .ok_or("bootstrap_distribution_size_cap")?
            / 34;
        let capacity = capacity.min(1024) as usize;
        if capacity == 0 {
            return Err("bootstrap_distribution_size_cap".into());
        }
        let mut roles = Vec::new();
        let mut n = 0u32;
        let mut u = 0u32;
        for actor in &config.allocation.actors {
            for _ in 0..actor.network_wallets {
                roles.push(WalletRole::NetworkWallet { index: n });
                n = n.checked_add(1).ok_or("bootstrap_count_overflow")?;
            }
            for _ in 0..actor.user_payment_wallets {
                roles.push(WalletRole::UserPayment { index: u });
                u = u.checked_add(1).ok_or("bootstrap_count_overflow")?;
            }
        }
        let count = (roles.len() as u64)
            .checked_mul(u64::from(per_wallet))
            .ok_or("bootstrap_count_overflow")?;
        if count == 0 {
            return Err("bootstrap_distribution_empty".into());
        }
        let parts = count.div_ceil(capacity as u64);
        if parts > 4096 {
            return Err("bootstrap_distribution_part_bound".into());
        }
        let fee_reserve = parts
            .checked_mul(policy.max_fee_sats_per_tx)
            .ok_or("bootstrap_distribution_fee_overflow")?;
        let amount = available
            .checked_sub(fee_reserve)
            .and_then(|n| n.checked_sub(policy.dust_sats))
            .ok_or("bootstrap_insufficient_mature_funding")?
            / count;
        if amount < policy.dust_sats {
            return Err("bootstrap_insufficient_mature_funding".into());
        }
        let outputs: Vec<_> = roles
            .into_iter()
            .flat_map(|role| {
                (0..per_wallet).map(move |receive_index| DistributionOutput {
                    role: role.clone(),
                    receive_index,
                    amount_sats: amount,
                })
            })
            .collect();
        Ok(Self {
            parts: outputs.chunks(capacity).map(<[_]>::to_vec).collect(),
            count,
            amount,
            fee_reserve,
            rate: policy.min_rate_sats_vb,
        })
    }
}

#[allow(clippy::too_many_arguments)]
fn verify_distribution_part(
    index: &CanonicalIndex,
    tx: &Transaction,
    height: u64,
    expected: &[DistributionOutput],
    config: &ActorConfig,
    mined: &BTreeMap<Txid, (u64, u32, TxOut)>,
    change_count: u32,
    rpc: &mut HostRpc,
) -> Result<(), String> {
    if tx.is_coinbase()
        || tx.input.iter().any(|i| !i.witness.is_empty())
        || tx.output.len() != expected.len() + 1
    {
        return Err("bootstrap_distribution_shape".into());
    }
    for (actual, wanted) in tx.output.iter().zip(expected) {
        rpc.checkpoint()?;
        let wallet = TrafficWallet::new(
            config.allocation.run_seed,
            &config.allocation.run_id,
            wanted.role.clone(),
            config.allocation.network,
            1,
        )
        .map_err(|e| e.to_string())?;
        let script = wallet
            .script_at(Chain::Receive, wanted.receive_index)
            .map_err(|e| e.to_string())?
            .script_pubkey;
        if actual.script_pubkey != script || actual.value.to_sat() != wanted.amount_sats {
            return Err("bootstrap_distribution_destination_mismatch".into());
        }
    }
    let policy = &config
        .allocation
        .actors
        .iter()
        .find(|a| a.driver_id == config.driver_id)
        .ok_or("bootstrap_host_missing")?
        .fee_policy;
    let fee = *index
        .fees
        .get(&tx.compute_txid())
        .ok_or("bootstrap_distribution_fee_unproven")?;
    if fee == 0 || fee > policy.max_fee_sats_per_tx || tx.vsize() as u64 > config.max_vbytes {
        return Err("bootstrap_distribution_fee_or_size".into());
    }
    crate::money::fee_within_band(
        fee,
        tx.vsize() as u64,
        policy.min_rate_sats_vb,
        policy.max_rate_sats_vb,
    )
    .map_err(|_| "bootstrap_distribution_fee_band")?;
    // Trace the input chain to an actually produced, mature native coinbase.
    // Every intermediate transaction is the host's funding change, verified
    // by a derived Change script, not merely by P2PKH script class.
    let wallet = TrafficWallet::new(
        config.allocation.run_seed,
        &config.allocation.run_id,
        WalletRole::BootstrapFunding,
        config.allocation.network,
        1,
    )
    .map_err(|e| e.to_string())?;
    let mut change_scripts = BTreeSet::new();
    for i in 0..change_count {
        if i % 16 == 0 {
            rpc.checkpoint()?;
        }
        change_scripts.insert(
            wallet
                .script_at(Chain::Change, i)
                .map_err(|e| e.to_string())?
                .script_pubkey,
        );
    }
    if !change_scripts.contains(
        &tx.output
            .last()
            .ok_or("bootstrap_change_missing")?
            .script_pubkey,
    ) {
        return Err("bootstrap_foreign_change".into());
    }
    let mut pending = vec![(height, tx)];
    let mut checked = BTreeSet::new();
    while let Some((height, tx)) = pending.pop() {
        rpc.checkpoint()?;
        if !checked.insert(tx.compute_txid()) {
            continue;
        }
        for input in &tx.input {
            let (created, parent) = index
                .transactions
                .get(&input.previous_output.txid)
                .ok_or("bootstrap_input_missing")?;
            let output = parent
                .output
                .get(input.previous_output.vout as usize)
                .ok_or("bootstrap_input_vout")?;
            if parent.is_coinbase() {
                if input.previous_output.vout != 0
                    || !is_mature(*created, height)?
                    || mined
                        .get(&parent.compute_txid())
                        .is_none_or(|(h, _, o)| h != created || o != output)
                {
                    return Err("bootstrap_native_input_unproven".into());
                }
            } else {
                if !change_scripts.contains(&output.script_pubkey) {
                    return Err("bootstrap_input_not_host_change".into());
                }
                pending.push((*created, parent));
            }
        }
    }
    Ok(())
}
fn decode<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|e| format!("bootstrap_shape: {e}"))
}
fn integer(value: &Value, key: &str) -> Result<u64, String> {
    value[key]
        .as_u64()
        .ok_or_else(|| format!("bootstrap_integer_required: {key}"))
}
fn integer_u32(value: &Value, key: &str) -> Result<u32, String> {
    u32::try_from(integer(value, key)?).map_err(|_| format!("bootstrap_u32_range: {key}"))
}
fn hex_bytes(value: &Value, length: Option<usize>) -> Result<Vec<u8>, String> {
    let s = value.as_str().ok_or("bootstrap_hex_string_required")?;
    if s.len() > MAX_RAW_BYTES * 2 || s.len() % 2 != 0 || length.is_some_and(|n| s.len() != n * 2) {
        return Err("bootstrap_hex_length".into());
    }
    hex::decode(s).map_err(|_| "bootstrap_hex_invalid".into())
}
fn parse_txid(value: &Value) -> Result<Txid, String> {
    hex_bytes(value, Some(32))?;
    Txid::from_str(value.as_str().ok_or("bootstrap_txid_shape")?).map_err(|e| e.to_string())
}
fn parse_blockhash(value: &Value) -> Result<BlockHash, String> {
    hex_bytes(value, Some(32))?;
    BlockHash::from_str(value.as_str().ok_or("bootstrap_hash_shape")?).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests;
