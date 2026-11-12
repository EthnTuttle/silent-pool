//! Traffic-only startup and control joins. The default T2 executor is absent;
//! no bootstrap completion or funding is synthesized by this controller.
use super::*;
use crate::evidence::{
    ListenerObservation, NodeListenerEvidence, NodeNetworkReadinessNode, NodeNetworkReadinessReport,
    NodeProcessBinding,
};
use crate::ids::{ActorId, CommandId};
use crate::traffic::control::{RpcGate, TxDriverControl};
use crate::traffic::runtime::{NodeBinding, TrafficStage};
use spc_lab_txd::actor_config::{ActorConfig, AllocationActor, AllocationDescription};
use spc_lab_txd::bootstrap::{BootstrapExecutor, BootstrapRequest, OperationControl};
use spc_lab_txd::net_readiness as readiness;

#[derive(Default)]
pub(super) struct TrafficRuntime {
    allocation: Option<AllocationDescription>,
    drivers: BTreeMap<ActorId, Arc<TxDriverControl>>,
    executor: Option<Arc<dyn BootstrapExecutor>>,
    pub(super) operation: Option<(CommandId, OperationControl)>,
}

struct ControllerBootstrapHost {
    engine: Weak<RunEngine>,
    request: BootstrapRequest,
    control: OperationControl,
}
impl spc_lab_txd::bootstrap::BootstrapHost for ControllerBootstrapHost {
    fn actor_request(&self, operation: &str, payload: Value) -> Result<Value, String> {
        if !matches!(
            operation,
            "bootstrap_rpc"
                | "funding_address"
                | "observe_bootstrap_coinbase"
                | "distribute"
                | "bootstrap_status"
                | "reserve_hashes"
                | "reconcile_hashes"
                | "budget_status"
                | "snapshot"
                | "observe"
        ) {
            return Err("bootstrap_host_operation_not_allowed".into());
        }
        if self.control.cancelled() {
            return Err("bootstrap_cancelled".into());
        }
        if self.control.paused()
            && !matches!(
                operation,
                "snapshot" | "bootstrap_status" | "budget_status" | "reconcile_hashes"
            )
        {
            return Err("bootstrap_paused".into());
        }
        let engine = self.engine.upgrade().ok_or("bootstrap_controller_gone")?;
        engine.bootstrap_checkpoint(&self.request, &self.control)?;
        let driver = ActorId::new(&self.request.driver_id).map_err(|e| e.to_string())?;
        let command = CommandId::new(&self.request.command_id).map_err(|e| e.to_string())?;
        let actor = engine
            .traffic
            .lock()
            .unwrap()
            .drivers
            .get(&driver)
            .cloned()
            .ok_or("bootstrap_driver_missing")?;
        let value = actor.request(Some(&command), operation, payload)?;
        if value["status"].as_str() == Some("failed") {
            return Err("bootstrap_actor_operation_failed".into());
        }
        Ok(value)
    }
}

struct CompletionRpc {
    nodes: BTreeMap<NodeId, Arc<RpcNodeControl>>,
    control: OperationControl,
    deadline: u64,
    next: u64,
}
impl spc_lab_txd::observer::RawRpc for CompletionRpc {
    fn read(&mut self, node: &str, method: &str, params: Value) -> Result<Value, String> {
        self.checkpoint()?;
        let id = self.next;
        self.next = id
            .checked_add(1)
            .ok_or("completion_rpc_sequence_exhausted")?;
        let node = self
            .nodes
            .get(&NodeId::new(node).map_err(|e| e.to_string())?)
            .ok_or("completion_node_missing")?;
        match node.call_traffic(id, method, params) {
            crate::traffic::rpc::RpcOutcome::Ok { result } => Ok(result),
            _ => Err("completion_rpc_unavailable".into()),
        }
    }
    fn checkpoint(&mut self) -> Result<(), String> {
        if self.control.cancelled() {
            return Err("bootstrap_cancelled".into());
        }
        if spc_lab_txd::bootstrap::monotonic_ms().map_err(|e| e.to_string())? >= self.deadline {
            return Err("bootstrap_deadline".into());
        }
        Ok(())
    }
}

pub(super) fn prepare_traffic(scenario: &Scenario, config: &LabConfig) -> Result<(), EngineError> {
    if scenario.traffic.is_none() {
        return Ok(());
    }
    if scenario.traffic.as_ref().is_some_and(|t| t.lab_acceptance) && !scenario.expanded_access {
        return Err(EngineError::new(
            "traffic_acceptance_operator_required",
            "lab acceptance requires expanded operator authentication",
        ));
    }
    if config
        .traffic_probe
        .as_ref()
        .is_none_or(|a| a.fixture || a.program.as_os_str().is_empty())
    {
        return Err(EngineError::new(
            "traffic_probe_unconfigured",
            "a canonical native probe adapter is required",
        ));
    }
    if crate::traffic::AP1A_APPROVED_CONTRACT_SHA256S.is_empty() {
        return Err(EngineError::new(
            "traffic_capability_lineage_unapproved",
            "no approved canonical probe lineage",
        ));
    }
    if config
        .tx_driver
        .as_ref()
        .is_none_or(|a| a.fixture || a.program.as_os_str().is_empty())
    {
        return Err(EngineError::new(
            "traffic_driver_unconfigured",
            "native tx driver adapter required",
        ));
    }
    selected_bootstrap(scenario)?;
    Ok(())
}

fn selected_bootstrap(scenario: &Scenario) -> Result<(ActorId, NodeId), EngineError> {
    let mut selected = None;
    for raw in scenario.phases.iter().flat_map(|phase| &phase.commands) {
        if raw["kind"]["type"].as_str() != Some("bootstrap_fund") {
            continue;
        }
        let kind: CommandKind = serde_json::from_value(raw["kind"].clone())
            .map_err(|e| EngineError::new("traffic_bootstrap_route_invalid", e.to_string()))?;
        let CommandKind::BootstrapFund { driver_id, node_id } = kind else {
            unreachable!()
        };
        if selected.replace((driver_id, node_id)).is_some() {
            return Err(EngineError::new(
                "traffic_bootstrap_duplicate",
                "exactly one declared bootstrap command required",
            ));
        }
    }
    let (driver, node) = selected.ok_or_else(|| {
        EngineError::new(
            "traffic_bootstrap_route_missing",
            "declare the bootstrap driver and canonical node before provisioning",
        )
    })?;
    let spec = scenario
        .traffic
        .as_ref()
        .ok_or_else(|| EngineError::new("traffic_disabled", "traffic missing"))?;
    if !spec
        .driver
        .actors
        .iter()
        .any(|a| a.id == driver && a.hosts_bootstrap_funding)
        || !scenario.nodes.iter().any(|n| n.id == node)
    {
        return Err(EngineError::new(
            "traffic_bootstrap_route_invalid",
            "bootstrap target is not the declared host/node",
        ));
    }
    Ok((driver, node))
}

impl RunEngine {
    /// Scope checks precede actor I/O. The complete validated live allocation
    /// alone chooses the driver and global UserPayment index.
    pub fn t1_payment_wallet(
        &self,
        auth: &crate::access::AuthContext,
        user: &str,
    ) -> Result<crate::traffic::payment_view::PaymentWalletView, String> {
        use crate::traffic::payment_view::{self, PaymentWalletView};
        payment_view::authorize(auth, &self.scenario, user)?;
        let manifest = self.manifest.lock().unwrap();
        let active = manifest.traffic_stage == Some(TrafficStage::Active)
            && manifest.traffic_preflight_state.as_deref() == Some("recognized")
            && manifest.state == crate::manifest::RunState::Running;
        drop(manifest);
        let (allocation, drivers, executor) = {
            let traffic = self.traffic.lock().unwrap();
            (
                traffic.allocation.clone(),
                traffic.drivers.clone(),
                traffic.executor.is_some(),
            )
        };
        if !active || !executor {
            return Ok(PaymentWalletView::unavailable(
                user,
                "not_active",
                "native_funding_not_ready",
            ));
        }
        let Some(allocation) = allocation else {
            return Ok(PaymentWalletView::unavailable(
                user,
                "not_active",
                "allocation_unavailable",
            ));
        };
        let mut sampled = None;
        let view = payment_view::read_scoped(
            auth,
            &self.scenario,
            user,
            &allocation,
            |driver, request| {
                let id = ActorId::new(driver).map_err(|e| e.to_string())?;
                let actor = drivers.get(&id).ok_or("payment_driver_unavailable")?;
                sampled = Some((id, actor.incarnation().to_string()));
                actor.read_payment_wallet(request)
            },
        )?;
        // A slow poll cannot advertise the sampled incarnation after stop or
        // replacement. This is a read-only final readiness check, not a grant.
        let manifest = self.manifest.lock().unwrap();
        let still_active = manifest.traffic_stage == Some(TrafficStage::Active)
            && manifest.traffic_preflight_state.as_deref() == Some("recognized")
            && manifest.state == crate::manifest::RunState::Running;
        drop(manifest);
        let traffic = self.traffic.lock().unwrap();
        let same_actor = sampled.as_ref().is_some_and(|(id, incarnation)| {
            traffic.drivers.get(id).is_some_and(|actor| {
                actor.incarnation() == incarnation.as_str() && actor.payment_ready()
            })
        });
        if !still_active || !same_actor || traffic.executor.is_none() {
            return Ok(PaymentWalletView::unavailable(
                user,
                "not_active",
                "payment_provider_changed",
            ));
        }
        Ok(view)
    }

    pub(super) fn t1_control_catalog(&self) -> Value {
        let manifest = self.manifest.lock().unwrap();
        let active = manifest.traffic_stage == Some(TrafficStage::Active)
            && manifest.traffic_preflight_state.as_deref() == Some("recognized")
            && manifest.state == crate::manifest::RunState::Running;
        drop(manifest);
        let traffic = self.traffic.lock().unwrap();
        let ready = active
            && traffic.executor.is_some()
            && traffic.allocation.is_some()
            && !traffic.drivers.is_empty()
            && traffic.drivers.values().all(|d| d.payment_ready());
        serde_json::json!({"available":ready,"drivers":if ready {traffic.drivers.keys().map(|id|serde_json::json!({"driver_id":id})).collect::<Vec<_>>()}else{Vec::new()}})
    }
    pub(super) fn close_traffic_gates(&self) {
        let drivers = self.traffic.lock().unwrap().drivers.clone();
        for actor in drivers.values() {
            actor.close_gate();
            actor.close_bootstrap();
        }
    }
    /// Phase execution waits outside the command mutex. HTTP stop/cancel and
    /// actor RPC service remain live while the tracked worker owns completion.
    pub fn wait_traffic_bootstrap(&self, id: &CommandId) -> Result<CommandOutcome, EngineError> {
        let budget = self
            .command_log
            .hash_budget_snapshot(id)
            .map_err(|e| EngineError::new("bootstrap_budget_unavailable", e.to_string()))?;
        let final_deadline = budget
            .operation
            .deadline_monotonic_ms
            .checked_add(
                budget
                    .operation
                    .cancel_grace_secs
                    .checked_mul(1000)
                    .ok_or_else(|| {
                        EngineError::new("bootstrap_deadline_overflow", "cancel deadline overflow")
                    })?,
            )
            .ok_or_else(|| {
                EngineError::new("bootstrap_deadline_overflow", "cancel deadline overflow")
            })?;
        loop {
            if let Some(outcome) = self.command_log.outcome(id) {
                return Ok(outcome);
            }
            let now = spc_lab_txd::bootstrap::monotonic_ms()
                .map_err(|e| EngineError::new("bootstrap_clock", e.to_string()))?;
            if now >= budget.operation.deadline_monotonic_ms {
                if let Some((operation, control)) = &self.traffic.lock().unwrap().operation {
                    if operation == id {
                        control.cancel();
                    }
                }
            }
            if now >= final_deadline
                || spc_lab_txd::bootstrap::boot_id()
                    .map_err(|e| EngineError::new("bootstrap_clock", e.to_string()))?
                    != budget.operation.boot_id
            {
                let _ = self.stop();
                return Err(EngineError::new(
                    "bootstrap_cancel_unobserved",
                    "no durable terminal result within cancellation bound",
                ));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    pub(crate) fn validate_traffic_workflow(scenario: &Scenario) -> Result<(), EngineError> {
        if scenario.traffic.is_some() {
            selected_bootstrap(scenario)?;
        }
        Ok(())
    }
    pub fn set_bootstrap_executor(&self, executor: Arc<dyn BootstrapExecutor>) {
        self.traffic.lock().unwrap().executor = Some(executor);
    }

    pub(super) fn traffic_preflight(&self) -> Result<(), EngineError> {
        if self.scenario.traffic.is_none() {
            return Ok(());
        }
        let result = self.run_traffic_preflight();
        if let Err(error) = &result {
            let mut manifest = self.manifest.lock().unwrap();
            manifest.traffic_preflight_state = Some("failed".into());
            manifest.warnings.push(error.to_string());
            manifest
                .save(&self.manifest_path)
                .map_err(|e| EngineError::new("manifest_write", e.to_string()))?;
        }
        result
    }

    fn run_traffic_preflight(&self) -> Result<(), EngineError> {
        use crate::traffic::launch_profile::digest;
        use crate::traffic::rpc::RpcOutcome;
        let err = |detail: String| EngineError::new("traffic_preflight_failed", detail);
        let profiles = self
            .manifest
            .lock()
            .unwrap()
            .traffic_launch_profiles
            .clone();
        if profiles.len() != self.scenario.nodes.len() {
            return Err(err("missing actual node launch profile".into()));
        }
        let mut owned = BTreeSet::new();
        for node in &self.scenario.nodes {
            for name in ["rpc", "p2p"] {
                let listener = self.listener("node", node.id.as_str(), name)?;
                let host = self
                    .addresses
                    .get(&format!("node.{}.{name}_addr", node.id))
                    .cloned()
                    .unwrap_or(listener.host);
                owned.insert(format!("{host}:{}", listener.port));
            }
        }
        let mut actual = BTreeMap::new();
        let controls = self.adapters.rpc_nodes.lock().unwrap().clone();
        for (id, profile) in &profiles {
            if self
                .manifest
                .lock()
                .unwrap()
                .process("node", id.as_str())
                .is_none_or(|p| p.executable_sha256 != profile.binary_sha256)
            {
                return Err(err(
                    "traffic_launch_profile_mismatch:actual_node_binary".into()
                ));
            }
            let comparison = profile
                .normalize(&owned, |p| std::fs::read(p).map_err(|e| e.to_string()))
                .map_err(|e| err(e.into()))?;
            let node = controls
                .get(id)
                .ok_or_else(|| err("fixture or missing typed node control".into()))?;
            let genesis = match node.call_traffic(1, "getblockhash", serde_json::json!([0])) {
                RpcOutcome::Ok { result } => result
                    .as_str()
                    .map(str::to_string)
                    .ok_or_else(|| err("invalid genesis".into()))?,
                _ => return Err(err("genesis RPC unavailable".into())),
            };
            let consensus = match node.call_traffic(2, "getblockchaininfo", serde_json::json!([])) {
                RpcOutcome::Ok { result } => {
                    crate::traffic::runtime::consensus_facts(&result).map_err(|e| err(e.into()))?
                }
                _ => return Err(err("effective consensus unavailable".into())),
            };
            actual.insert(
                id.clone(),
                NodeBinding {
                    binary_sha256: profile.binary_sha256.clone(),
                    genesis,
                    comparison_sha256: digest(&comparison).map_err(|e| err(e.into()))?,
                    consensus,
                },
            );
        }
        let adapter = self
            .config
            .traffic_probe
            .as_ref()
            .ok_or_else(|| err("probe unconfigured".into()))?;
        let helper = self
            .lifetime_helper
            .as_ref()
            .ok_or_else(|| err("parent-death helper required".into()))?;
        let first = self
            .scenario
            .nodes
            .first()
            .ok_or_else(|| err("no actual node".into()))?;
        let binary = &self
            .config
            .node(&first.implementation)
            .ok_or_else(|| err("node adapter missing".into()))?
            .program;
        let root = self.run_dir.join("private/traffic");
        use std::os::unix::fs::DirBuilderExt;
        match std::fs::DirBuilder::new().mode(0o700).create(&root) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(err(e.to_string())),
        }
        self.artifacts
            .write_private("traffic/launch-profile.json", &profiles[&first.id])
            .map_err(|e| err(e.to_string()))?;
        self.artifacts
            .write_private(
                "traffic/owned-endpoints.json",
                &serde_json::json!({"endpoints":owned}),
            )
            .map_err(|e| err(e.to_string()))?;
        let output = root.join("probe");
        let pid = std::process::id();
        let ticks = crate::supervisor::process_start_time(pid)
            .ok_or_else(|| err("controller identity unavailable".into()))?;
        let mut args = adapter.args.clone();
        args.extend([
            "node-probe".into(),
            "--binary".into(),
            binary.display().to_string(),
            "--source-name".into(),
            format!(
                "spc-lab-bound-node-binary-{}",
                profiles[&first.id].binary_sha256
            ),
            "--launch-profile".into(),
            root.join("launch-profile.json").display().to_string(),
            "--owned-endpoints".into(),
            root.join("owned-endpoints.json").display().to_string(),
            "--actor-exec".into(),
            helper.display().to_string(),
            "--actor-exec-sha256".into(),
            crate::supervisor::hash_file(helper).map_err(|e| err(e.to_string()))?,
            "--controller-pid".into(),
            pid.to_string(),
            "--controller-start-ticks".into(),
            ticks.to_string(),
            "--label".into(),
            "traffic-preflight".into(),
            "--out".into(),
            output.display().to_string(),
        ]);
        self.artifacts
            .write_private(
                "traffic/probe-launch-intent.json",
                &serde_json::json!({"program":adapter.program,"args":args,"profiles":profiles}),
            )
            .map_err(|e| err(e.to_string()))?;
        let launched = self.launch_process(
            "traffic_probe",
            "preflight",
            adapter,
            args,
            adapter.env.clone(),
            Readiness::Immediate,
            &root.join("probe.log"),
            false,
        )?;
        let deadline = std::time::Instant::now() + Duration::from_secs(120);
        let success = loop {
            let status = self
                .supervisor
                .lock()
                .unwrap()
                .try_wait_child(launched.record.pid)
                .map_err(|e| err(e.to_string()))?;
            if let Some(status) = status {
                break status.success();
            }
            if std::time::Instant::now() >= deadline {
                break false;
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        if !success {
            let _ = self.supervisor.lock().unwrap().stop(
                "traffic_probe",
                "preflight",
                Duration::from_secs(1),
            );
            return Err(err("canonical probe failed or timed out".into()));
        }
        let report_path = output.join("report.json");
        if std::fs::metadata(&report_path)
            .map_err(|e| err(e.to_string()))?
            .len()
            > 16 * 1024 * 1024
        {
            return Err(err("canonical report exceeds bound".into()));
        }
        let bytes = std::fs::read(report_path).map_err(|e| err(e.to_string()))?;
        let report: Value =
            spc_lab_txd::control::strict_json(&bytes).map_err(|e| err(e.to_string()))?;
        let policy = &report["observations"]["traffic_fee_policy"];
        let relay = policy["minrelaytxfee_sats_kb"]
            .as_u64()
            .ok_or_else(|| err("traffic_fee_policy_unavailable".into()))?;
        if report["traffic_consumer"].as_str() != Some(crate::traffic::consumer::CONTRACT)
            || policy["errors"]
                .as_array()
                .is_none_or(|errors| !errors.is_empty())
            || u128::from(
                self.scenario
                    .traffic
                    .as_ref()
                    .unwrap()
                    .fees
                    .rate_sats_vb
                    .min,
            ) * 1000
                < u128::from(relay)
        {
            return Err(err("traffic_fee_policy_mismatch".into()));
        }
        if report["node"]["launch_profile"]["helper_sha256"].as_str()
            != Some(
                crate::supervisor::hash_file(helper)
                    .map_err(|e| err(e.to_string()))?
                    .as_str(),
            )
        {
            return Err(err("traffic_launch_profile_mismatch:helper_binary".into()));
        }
        crate::traffic::runtime::bind_report(&report, &actual).map_err(err)?;
        let record =
            crate::traffic::project_traffic_capability(&bytes).map_err(|e| err(e.to_string()))?;
        check_traffic_capability(&self.scenario, Some(&record)).map_err(|e| err(e.to_string()))?;
        for binding in actual.values() {
            if record.expected_genesis != binding.genesis {
                return Err(err("traffic_launch_profile_mismatch:genesis".into()));
            }
        }
        let mut manifest = self.manifest.lock().unwrap();
        manifest.traffic_capability = Some(record);
        manifest.traffic_preflight_state = Some("recognized".into());
        manifest
            .save(&self.manifest_path)
            .map_err(|e| err(e.to_string()))
    }

    pub(super) fn launch_tx_drivers(&self) -> Result<(), EngineError> {
        let Some(spec) = self.scenario.traffic.as_ref() else {
            return Ok(());
        };
        let (_, authority) = selected_bootstrap(&self.scenario)?;
        let mut actors: Vec<_> = spec
            .driver
            .actors
            .iter()
            .map(|a| AllocationActor {
                driver_id: a.id.to_string(),
                network_wallets: a.network_wallets,
                user_payment_wallets: a.user_payment_wallets,
                hosts_bootstrap_funding: a.hosts_bootstrap_funding,
                lookahead: spc_lab_txd::keys::DEFAULT_SCRIPT_LOOKAHEAD,
                fee_policy: spc_lab_txd::builder::FeePolicy {
                    min_rate_sats_vb: spec.fees.rate_sats_vb.min,
                    max_rate_sats_vb: spec.fees.rate_sats_vb.max,
                    max_fee_sats_per_tx: spec.fees.max_fee_sats_per_tx,
                    dust_sats: 546,
                },
            })
            .collect();
        actors.sort_by(|a, b| a.driver_id.cmp(&b.driver_id));
        let mut users: Vec<_> = self
            .scenario
            .identities
            .iter()
            .map(|i| i.id.to_string())
            .collect();
        users.sort();
        let allocation = AllocationDescription {
            run_id: self.run_id.to_string(),
            run_seed: self.scenario.run.seed,
            network: bitcoin::Network::Regtest,
            caps: spc_lab_txd::journal::JournalCaps {
                unknown_retry_sends: spec.caps.max_unknown_retries as u32,
                reorg_resubmits: spec.caps.reorg_resubmit_max as u32,
            },
            actors,
            users,
        };
        let allocation_digest = allocation
            .reconstruct()
            .map_err(|e| EngineError::new("traffic_allocation", e.to_string()))?
            .digest()
            .to_owned();
        let total: u32 = allocation.actors.iter().map(|a| a.network_wallets).sum();
        let mut nodes: Vec<_> = self
            .scenario
            .nodes
            .iter()
            .map(|n| n.id.to_string())
            .collect();
        nodes.sort();
        let chain_policy = self.traffic_chain_policy(&authority)?;
        for actor in &allocation.actors {
            let config = ActorConfig {
                lab_acceptance: spec.lab_acceptance,
                node_profile_binding: self.traffic_rejection_binding(&authority),
                chain_policy: chain_policy.clone(),
                size_classes: spec
                    .sizes
                    .classes_vb
                    .iter()
                    .map(|class| spc_lab_txd::actor_config::SizeClass {
                        vbytes: class.vb,
                        mix: class.mix,
                    })
                    .collect(),
                protocol: spc_lab_txd::control::PROTOCOL.into(),
                history_version: 3,
                run_dir: self.run_dir.clone(),
                driver_id: actor.driver_id.clone(),
                allocation: allocation.clone(),
                node_ids: nodes.clone(),
                authority_node: authority.to_string(),
                ordinary_interval_ms: (1000.0 * f64::from(total)
                    / (spec.rate.ordinary_tx_per_sec * f64::from(actor.network_wallets)))
                .ceil() as u64,
                max_vbytes: spec.sizes.max_vb,
                max_mempool_backlog_tx: spec.caps.max_mempool_backlog_tx,
            };
            let path = config
                .write()
                .map_err(|e| EngineError::new("traffic_config", e.to_string()))?;
            {
                let root = config
                    .root()
                    .map_err(|e| EngineError::new("traffic_config", e.to_string()))?;
                let mut manifest = self.manifest.lock().unwrap();
                manifest.traffic_driver_paths.insert(actor.driver_id.clone(),serde_json::json!({
                    "component":root.file_name().and_then(|s|s.to_str()),"key_path":root.join("private/traffic-wallets.json"),
                    "journal_path":root.join("private/traffic/history.jsonl"),"config_path":path,
                    "allocation_digest":allocation_digest,"authority_node":authority,
                    "config_sha256":crate::supervisor::hash_file(&path).map_err(|e|EngineError::new("traffic_config_hash",e.to_string()))?}));
                manifest
                    .save(&self.manifest_path)
                    .map_err(|e| EngineError::new("manifest_write", e.to_string()))?;
            }
            self.launch_tx_driver(&ActorId::trusted(&actor.driver_id), &path)?;
            if actor.hosts_bootstrap_funding && spec.bootstrap.enabled {
                let mut runtime = self.traffic.lock().unwrap();
                if runtime.executor.is_none() {
                    let incarnation = runtime.drivers[&ActorId::trusted(&actor.driver_id)]
                        .incarnation()
                        .to_owned();
                    let executor = spc_lab_txd::native_bootstrap::NativeBootstrapExecutor::new(
                        config.clone(),
                        incarnation,
                        spec.bootstrap.funding_blocks,
                        spec.bootstrap.distribution_outputs_per_wallet,
                    )
                    .map_err(|e| EngineError::new("native_bootstrap_configuration", e))?;
                    let manifest = self.manifest.lock().unwrap();
                    let executable = std::env::current_exe().map_err(|e| {
                        EngineError::new("native_bootstrap_provenance", e.to_string())
                    })?;
                    self.artifacts.write_private("traffic/native-bootstrap-provenance.json", &serde_json::json!({
                        "artifact_path": path.parent().ok_or_else(|| EngineError::new("native_bootstrap_provenance", "actor root missing"))?.join("native-bootstrap.jsonl"),
                        "artifact_authority": "observation_only",
                        "source_revision": manifest.base_revision,
                        "source_snapshot_claim": manifest.dirty_tree_hash,
                        "executor_source_sha256": spc_lab_txd::native_bootstrap::NativeBootstrapExecutor::source_sha256(),
                        "controller_executable_sha256": crate::supervisor::hash_file(&executable)
                            .map_err(|e| EngineError::new("native_bootstrap_provenance", e.to_string()))?,
                    })).map_err(|e| EngineError::new("native_bootstrap_provenance", e.to_string()))?;
                    runtime.executor = Some(Arc::new(executor));
                }
            }
        }
        self.set_traffic_stage(TrafficStage::Provisioned)
    }

    fn traffic_rejection_binding(
        &self,
        node: &NodeId,
    ) -> Option<spc_lab_txd::send_outcome::NodeProfileBinding> {
        let manifest = self.manifest.lock().unwrap();
        let binary = &manifest.traffic_launch_profiles.get(node)?.binary_sha256;
        let consensus = manifest
            .effective_profiles
            .get(node)?
            .profile_fingerprint
            .as_deref()?;
        let binding = spc_lab_txd::send_outcome::reviewed_binding(binary, consensus)?;
        if self
            .config
            .traffic_node_source_sha256
            .as_ref()
            .is_some_and(|s| s != &binding.source_sha256)
        {
            return None;
        }
        Some(binding)
    }

    fn launch_tx_driver(&self, id: &ActorId, path: &Path) -> Result<(), EngineError> {
        if self
            .manifest
            .lock()
            .unwrap()
            .traffic_preflight_state
            .as_deref()
            != Some("recognized")
        {
            return Err(EngineError::new(
                "traffic_capability_unrecorded",
                "tx driver launch requires successful bound preflight",
            ));
        }
        let adapter = self.config.tx_driver.as_ref().ok_or_else(|| {
            EngineError::new("traffic_driver_unconfigured", "native txd required")
        })?;
        let incarnation = CommandId::generate().to_string();
        let launched = self.launch_process(
            "tx_driver",
            id.as_str(),
            adapter,
            vec![
                "--config".into(),
                path.display().to_string(),
                "--incarnation".into(),
                incarnation.clone(),
            ],
            BTreeMap::new(),
            Readiness::Immediate,
            &path.with_file_name("actor.log"),
            true,
        )?;
        let gate = RpcGate::new(self.adapters.rpc_nodes.lock().unwrap().clone(), 0)
            .with_audit(
                self.command_log.clone(),
                crate::command::traffic_audit::Session {
                    run_id: self.run_id.to_string(),
                    driver_id: id.to_string(),
                    incarnation: incarnation.clone(),
                    controller_sha256: crate::supervisor::hash_file(
                        &std::env::current_exe()
                            .map_err(|e| EngineError::new("traffic_audit", e.to_string()))?,
                    )
                    .map_err(|e| EngineError::new("traffic_audit", e.to_string()))?,
                    actor_sha256: launched.record.executable_sha256.clone(),
                    acceptance_enabled: self
                        .scenario
                        .traffic
                        .as_ref()
                        .is_some_and(|t| t.lab_acceptance),
                },
            )
            .map_err(|e| EngineError::new("traffic_audit", e.to_string()))?;
        let control = TxDriverControl::spawn(
            id.clone(),
            incarnation,
            launched
                .stdin
                .ok_or_else(|| EngineError::new("traffic_pipe", "stdin missing"))?,
            launched
                .stdout
                .ok_or_else(|| EngineError::new("traffic_pipe", "stdout missing"))?,
            gate,
        )
        .map_err(|e| EngineError::new("traffic_control", e.to_string()))?;
        let hello = control
            .hello()
            .map_err(|e| EngineError::new("traffic_hello", e))?;
        if hello["status"].as_str() != Some("provisioned") {
            return Err(EngineError::new(
                "traffic_provision_failed",
                hello.to_string(),
            ));
        }
        let expected = ActorConfig::load(path)
            .map_err(|e| EngineError::new("traffic_config", e.to_string()))?;
        if expected.lab_acceptance
            != self
                .scenario
                .traffic
                .as_ref()
                .is_some_and(|t| t.lab_acceptance)
        {
            return Err(EngineError::new(
                "traffic_acceptance_binding",
                "actor acceptance flag differs from scenario",
            ));
        }
        let allocation = expected
            .validate()
            .map_err(|e| EngineError::new("traffic_config", e.to_string()))?;
        if hello["allocation_digest"].as_str() != Some(allocation.digest()) {
            return Err(EngineError::new(
                "traffic_allocation_mismatch",
                "actor header digest mismatch",
            ));
        }
        {
            let mut traffic = self.traffic.lock().unwrap();
            if let Some(old) = &traffic.allocation {
                if old
                    .reconstruct()
                    .map_err(|e| EngineError::new("traffic_allocation", e.to_string()))?
                    .digest()
                    != allocation.digest()
                {
                    return Err(EngineError::new(
                        "traffic_allocation_mismatch",
                        "provider allocation changed",
                    ));
                }
            }
            traffic.allocation = Some(expected.allocation.clone());
        }
        self.spawn_traffic_event_pump(id, &control);
        self.traffic
            .lock()
            .unwrap()
            .drivers
            .insert(id.clone(), Arc::new(control));
        Ok(())
    }

    fn traffic_chain_policy(
        &self,
        authority: &NodeId,
    ) -> Result<spc_lab_txd::observer::ChainPolicy, EngineError> {
        use std::str::FromStr;
        let manifest = self.manifest.lock().unwrap();
        let profile = manifest.effective_profiles.get(authority).ok_or_else(|| {
            EngineError::new(
                "traffic_consensus_unavailable",
                "canonical authority profile missing",
            )
        })?;
        if profile.coinbase_maturity_blocks != Some(100) {
            return Err(EngineError::new(
                "traffic_consensus_unsupported",
                "traffic v1 requires actual 100-block coinbase maturity",
            ));
        }
        let capability = manifest.traffic_capability.as_ref().ok_or_else(|| {
            EngineError::new("traffic_capability_unrecorded", "traffic profile missing")
        })?;
        let missing = || {
            EngineError::new(
                "traffic_consensus_unavailable",
                "required exact consensus fact missing",
            )
        };
        let bits = |value: &Option<String>| -> Result<u32, EngineError> {
            let value = value.as_deref().ok_or_else(missing)?;
            u32::from_str_radix(value.strip_prefix("0x").unwrap_or(value), 16)
                .map_err(|_| missing())
        };
        let policy = spc_lab_txd::observer::ChainPolicy {
            genesis: bitcoin::BlockHash::from_str(&capability.expected_genesis)
                .map_err(|_| missing())?,
            pow_limit_bits: bits(&profile.pow_limit_bits)?,
            initial_target_bits: bits(&profile.initial_target_bits)?,
            no_pow_retargeting: profile.no_pow_retargeting.ok_or_else(missing)?,
            allow_min_difficulty_blocks: profile.allow_min_difficulty_blocks.ok_or_else(missing)?,
            pow_target_spacing: profile.target_spacing_secs.ok_or_else(missing)?,
            difficulty_adjustment_interval: profile.retarget_interval_blocks.ok_or_else(missing)?,
            regtest_pow_bits_overlay: bits(&profile.regtest_pow_bits_overlay)?,
            subsidy_initial_sats: profile
                .subsidy_initial_sats
                .as_deref()
                .ok_or_else(missing)?
                .parse()
                .map_err(|_| missing())?,
            subsidy_halving_interval: profile
                .subsidy_halving_interval_blocks
                .ok_or_else(missing)?,
        };
        policy
            .pow_limit()
            .map_err(|e| EngineError::new("traffic_consensus_invalid", e))?;
        spc_lab_txd::contextual_pow::validate_policy(&policy)
            .map_err(|e| EngineError::new("traffic_consensus_unsupported", e))?;
        Ok(policy)
    }

    fn spawn_traffic_event_pump(&self, id: &ActorId, control: &TxDriverControl) {
        let weak = self.self_ref.lock().unwrap().clone();
        let receiver = control.event_receiver();
        let driver = id.clone();
        let incarnation = control.incarnation().to_string();
        let _ = std::thread::Builder::new()
            .name("txd-events".into())
            .spawn(move || loop {
                let message = receiver.lock().unwrap().recv();
                let Ok(spc_lab_txd::control::Message::Event {
                    event_id,
                    name,
                    payload,
                    ..
                }) = message
                else {
                    break;
                };
                let Some(engine) = weak.upgrade() else {
                    break;
                };
                if name != "tx_traffic" {
                    continue;
                }
                let Ok(observation) =
                    serde_json::from_value::<spc_lab_txd::control::TrafficObservation>(payload)
                else {
                    break;
                };
                engine.evidence.append(
                    SourceId::trusted("tx-traffic"),
                    format!("{incarnation}:{event_id}"),
                    EvidenceActor::harness(),
                    None,
                    EvidencePayload::TxTraffic {
                        driver_id: driver.clone(),
                        incarnation: incarnation.clone(),
                        observation,
                    },
                );
            });
    }

    fn set_traffic_stage(&self, stage: TrafficStage) -> Result<(), EngineError> {
        let mut manifest = self.manifest.lock().unwrap();
        manifest.traffic_stage = Some(stage);
        manifest
            .save(&self.manifest_path)
            .map_err(|e| EngineError::new("manifest_write", e.to_string()))?;
        drop(manifest);
        self.evidence.append(
            SourceId::trusted("traffic-stage"),
            format!("stage-{}", self.command_log.last_seq()),
            EvidenceActor::harness(),
            None,
            EvidencePayload::TrafficStage { stage },
        );
        Ok(())
    }

    /// Live-process half of the traffic node binding: the recorded process
    /// must still be the launched one (`Ownership::Owned`) with the recorded
    /// argv and binary. Split from [`Self::reverify_traffic_binding`] so the
    /// cleanup-path contrast is directly testable: a stopped run fails this
    /// live proof while [`Self::validate_persisted_traffic_bindings`]
    /// accepts the same on-disk state.
    fn reverify_node_record(
        &self,
        node: &crate::scenario::NodeSpec,
        profile: &crate::traffic::launch_profile::LaunchProfile,
        process: &crate::supervisor::ProcessRecord,
    ) -> Result<(), EngineError> {
        if self.supervisor.lock().unwrap().verify(process) != crate::supervisor::Ownership::Owned
            || process.args != profile.argv
            || process.executable_sha256 != profile.binary_sha256
        {
            return Err(EngineError::new(
                "traffic_launch_profile_mismatch",
                format!("node {} ownership/argv/binary changed", node.id),
            ));
        }
        Ok(())
    }

    pub(super) fn reverify_traffic_binding(&self) -> Result<(), EngineError> {
        if self.scenario.traffic.is_none() {
            return Ok(());
        }
        let manifest = self.manifest.lock().unwrap().clone();
        if manifest.traffic_lab_acceptance
            != self
                .scenario
                .traffic
                .as_ref()
                .is_some_and(|t| t.lab_acceptance)
        {
            return Err(EngineError::new(
                "traffic_acceptance_binding",
                "manifest acceptance flag differs from scenario",
            ));
        }
        check_traffic_capability(&self.scenario, manifest.traffic_capability.as_ref())
            .map_err(|e| EngineError::new(e.code(), e.detail()))?;
        let record = manifest
            .traffic_capability
            .as_ref()
            .ok_or_else(|| EngineError::new("traffic_capability_unrecorded", "record missing"))?;
        let report_path = self.run_dir.join("private/traffic/probe/report.json");
        if crate::supervisor::hash_file(&report_path)
            .map_err(|e| EngineError::new("traffic_probe_binding", e.to_string()))?
            != record.report_sha256
        {
            return Err(EngineError::new(
                "traffic_probe_binding",
                "recorded canonical report changed",
            ));
        }
        let report: Value = spc_lab_txd::control::strict_json(
            &std::fs::read(report_path)
                .map_err(|e| EngineError::new("traffic_probe_binding", e.to_string()))?,
        )
        .map_err(|e| EngineError::new("traffic_probe_binding", e.to_string()))?;
        let controls = self.adapters.rpc_nodes.lock().unwrap().clone();
        let mut actual = BTreeMap::new();
        let mut owned = BTreeSet::new();
        for node in &self.scenario.nodes {
            for name in ["rpc", "p2p"] {
                let endpoint = self.listener("node", node.id.as_str(), name)?;
                let host = self
                    .addresses
                    .get(&format!("node.{}.{name}_addr", node.id))
                    .cloned()
                    .unwrap_or(endpoint.host);
                owned.insert(format!("{host}:{}", endpoint.port));
            }
        }
        for node in &self.scenario.nodes {
            let profile = manifest
                .traffic_launch_profiles
                .get(&node.id)
                .ok_or_else(|| {
                    EngineError::new(
                        "traffic_launch_profile_mismatch",
                        "missing recorded profile",
                    )
                })?;
            let process = manifest.process("node", node.id.as_str()).ok_or_else(|| {
                EngineError::new(
                    "traffic_launch_profile_mismatch",
                    "missing node ownership record",
                )
            })?;
            self.reverify_node_record(node, profile, process)?;
            let bytes = std::fs::read(format!("/proc/{}/environ", process.pid))
                .map_err(|e| EngineError::new("traffic_launch_profile_mismatch", e.to_string()))?;
            let mut env = BTreeMap::new();
            for entry in bytes.split(|b| *b == 0).filter(|e| !e.is_empty()) {
                let text = std::str::from_utf8(entry).map_err(|_| {
                    EngineError::new(
                        "traffic_launch_profile_mismatch",
                        "non-UTF8 node environment",
                    )
                })?;
                let (key, value) = text.split_once('=').ok_or_else(|| {
                    EngineError::new(
                        "traffic_launch_profile_mismatch",
                        "invalid node environment",
                    )
                })?;
                if env.insert(key.to_string(), value.to_string()).is_some() {
                    return Err(EngineError::new(
                        "traffic_launch_profile_mismatch",
                        "duplicate node environment key",
                    ));
                }
            }
            if env != profile.env {
                return Err(EngineError::new(
                    "traffic_launch_profile_mismatch",
                    "finalized node environment changed",
                ));
            }
            let normalized = profile
                .normalize(&owned, |path| {
                    std::fs::read(path).map_err(|e| e.to_string())
                })
                .map_err(|e| EngineError::new(e, e))?;
            let rpc = controls.get(&node.id).ok_or_else(|| {
                EngineError::new(
                    "traffic_transport_unavailable",
                    "typed actual-node route unavailable",
                )
            })?;
            let genesis = match rpc.call_traffic(1, "getblockhash", serde_json::json!([0])) {
                crate::traffic::rpc::RpcOutcome::Ok { result } => {
                    result.as_str().map(str::to_string)
                }
                _ => None,
            }
            .ok_or_else(|| {
                EngineError::new(
                    "traffic_launch_profile_mismatch",
                    "actual genesis unavailable",
                )
            })?;
            if genesis != record.expected_genesis {
                return Err(EngineError::new(
                    "traffic_launch_profile_mismatch",
                    "actual genesis changed",
                ));
            }
            let consensus = match rpc.call_traffic(2, "getblockchaininfo", serde_json::json!([])) {
                crate::traffic::rpc::RpcOutcome::Ok { result } => {
                    crate::traffic::runtime::consensus_facts(&result)
                }
                _ => Err("traffic_consensus_unavailable"),
            }
            .map_err(|e| EngineError::new(e, e))?;
            actual.insert(
                node.id.clone(),
                NodeBinding {
                    binary_sha256: profile.binary_sha256.clone(),
                    genesis,
                    comparison_sha256: crate::traffic::launch_profile::digest(&normalized)
                        .map_err(|e| EngineError::new(e, e))?,
                    consensus,
                },
            );
        }
        crate::traffic::runtime::bind_report(&report, &actual)
            .map_err(|e| EngineError::new("traffic_launch_profile_mismatch", e))?;
        for mapping in manifest.traffic_driver_paths.values() {
            let path = mapping["config_path"]
                .as_str()
                .ok_or_else(|| EngineError::new("traffic_config_binding", "config path missing"))?;
            if Some(
                crate::supervisor::hash_file(Path::new(path))
                    .map_err(|e| EngineError::new("traffic_config_binding", e.to_string()))?
                    .as_str(),
            ) != mapping["config_sha256"].as_str()
            {
                return Err(EngineError::new(
                    "traffic_config_binding",
                    "actor config changed",
                ));
            }
        }
        if let Some(boundary) = manifest.traffic_measured_from {
            if boundary.boot_id
                != spc_lab_txd::bootstrap::boot_id()
                    .map_err(|e| EngineError::new("traffic_clock", e.to_string()))?
            {
                return Err(EngineError::new(
                    "traffic_clock",
                    "measured run belongs to another boot",
                ));
            }
        }
        Ok(())
    }

    /// Cleanup-purpose validation of the persisted traffic bindings. Unlike
    /// [`Self::reverify_traffic_binding`] this touches no live process: a
    /// missing node process record is not a failure here (the engine's
    /// `reap` reports it as expected-but-absent), while a persisted
    /// contradiction - a recorded process whose argv or binary no longer
    /// matches the recorded launch profile, or a changed driver config -
    /// rejects so cleanup never trusts rewritten history.
    pub(super) fn validate_persisted_traffic_bindings(&self) -> Result<(), EngineError> {
        if self.scenario.traffic.is_none() {
            return Ok(());
        }
        let manifest = self.manifest.lock().unwrap();
        for node in &self.scenario.nodes {
            let profile = manifest
                .traffic_launch_profiles
                .get(&node.id)
                .ok_or_else(|| {
                    EngineError::new(
                        "traffic_launch_profile_mismatch",
                        "missing recorded profile",
                    )
                })?;
            let Some(record) = manifest.process("node", node.id.as_str()) else {
                // Reported by reap as an expected-but-absent node.
                continue;
            };
            if record.args != profile.argv || record.executable_sha256 != profile.binary_sha256 {
                return Err(EngineError::new(
                    "traffic_launch_profile_mismatch",
                    "persisted node argv/binary binding changed",
                ));
            }
        }
        for mapping in manifest.traffic_driver_paths.values() {
            let path = mapping["config_path"]
                .as_str()
                .ok_or_else(|| EngineError::new("traffic_config_binding", "config path missing"))?;
            if Some(
                crate::supervisor::hash_file(Path::new(path))
                    .map_err(|e| EngineError::new("traffic_config_binding", e.to_string()))?
                    .as_str(),
            ) != mapping["config_sha256"].as_str()
            {
                return Err(EngineError::new(
                    "traffic_config_binding",
                    "actor config changed",
                ));
            }
        }
        Ok(())
    }

    /// Bounded observed-connectivity gate: wait for the planned node peer
    /// graph to be live before any bootstrap hash budget is reserved.
    /// Missing connectivity retries until a deadline (capped at
    /// [`NODE_NETWORK_READINESS_TIMEOUT_MS`]); an observation that
    /// contradicts the plan refuses immediately. The run is not stopped and
    /// zero hash debits occur: the gate runs before the checked budget
    /// start and
    /// a failure leaves `began` false, so the command resolves as a
    /// rejected precondition. On success, returns the never-reset
    /// acquisition deadline the caller must carry forward to its
    /// checked budget start (start_hash_budget_checked): the gate's own
    /// expiry when a P2P graph was observed, otherwise the original
    /// operation deadline.
    pub(super) fn await_node_network_readiness(
        &self,
        command_id: &CommandId,
        operation_deadline_ms: u64,
    ) -> Result<u64, String> {
        let plan = planned_node_network(&self.manifest.lock().unwrap(), &self.scenario)
            .map_err(|e| e.to_string())?;
        if plan.edges.is_empty() {
            // No planned P2P graph (fixture nodes or an unpeered run):
            // nothing to observe before bootstrap. The original operation
            // deadline is then the only acquisition bound, so it is the
            // bound carried forward to the pre-HashStart transition.
            return Ok(operation_deadline_ms);
        }
        // The whole-acquisition clock: one monotonic start before the
        // first read, one deadline that is never reset (the bootstrap
        // operation's own deadline capped by the source constant), and
        // the source-declared bounds serialized with every record.
        let started_at = spc_lab_txd::bootstrap::monotonic_ms().map_err(|e| e.to_string())?;
        let deadline = operation_deadline_ms.min(started_at + NODE_NETWORK_READINESS_TIMEOUT_MS);
        let clock = AcquisitionClock {
            started_at_mono_ms: started_at,
            deadline_mono_ms: deadline,
            bounds: readiness::ReadinessAcquisitionBounds {
                max_acquisition_ms: deadline
                    .checked_sub(started_at)
                    .ok_or("node_network_not_ready: bootstrap deadline already expired before the readiness gate")?,
                max_rpc_call_ms: readiness::READINESS_MAX_RPC_CALL_MS,
                max_peer_rows: readiness::READINESS_MAX_PEER_ROWS,
                max_tcp_rows: readiness::READINESS_MAX_TCP_ROWS,
                max_fd_entries: readiness::READINESS_MAX_FD_ENTRIES,
            },
        };
        // Every planned directed dial is an independent obligation: the
        // gate is ready only when each one carries a complete typed
        // proof (undirected report coverage alone proves nothing).
        let planned_dials: BTreeSet<(NodeId, NodeId)> = plan
            .connect
            .iter()
            .filter_map(|(node, target)| {
                plan.listen_endpoint_owner(target, node)
                    .map(|owner| (node.clone(), owner))
            })
            .collect();
        let mut request_id = 4u64;
        loop {
            // Cooperative: an operator stop or state change aborts the gate
            // instead of deadlocking behind it.
            if !self.state.lock().unwrap().started || self.manifest_state() != RunState::Running {
                return Err("node_network_readiness_aborted: run no longer active".into());
            }
            let pass = self.observe_node_network(&plan, &mut request_id, &clock);
            let ready = pass.covered_dials == planned_dials && pass.failure.is_none();
            if ready {
                // Deadline check on SUCCESS, before the ready record is
                // persisted and before the caller can reserve any hash
                // budget: a structurally valid observation is unready
                // unless BOTH its completed stamp and the fresh current
                // clock are inside the whole-acquisition deadline. The
                // Err return keeps `began` false, so no HashStart and no
                // new operation id or timeout is created.
                let now = spc_lab_txd::bootstrap::monotonic_ms().map_err(|e| e.to_string())?;
                success_within_deadline(
                    pass.completed_at_mono_ms,
                    now,
                    clock.deadline_mono_ms,
                )?;
                self.persist_node_network_readiness(
                    command_id, "ready", None, &pass, &plan, &clock,
                );
                // Carry the gate's never-reset acquisition expiry forward
                // to the caller's pre-HashStart transition check.
                return Ok(clock.deadline_mono_ms);
            }
            match &pass.failure {
                Some(GateFailure::Refuse { detail }) => {
                    self.persist_node_network_readiness(
                        command_id,
                        "refused",
                        Some(detail.clone()),
                        &pass,
                        &plan,
                        &clock,
                    );
                    return Err(format!("node_network_plan_violated: {detail}"));
                }
                failure => {
                    let now = spc_lab_txd::bootstrap::monotonic_ms().map_err(|e| e.to_string())?;
                    if now >= clock.deadline_mono_ms {
                        let detail = failure
                            .as_ref()
                            .map(|f| f.detail().to_string())
                            .unwrap_or_else(|| "planned peer edges not yet covered".to_string());
                        self.persist_node_network_readiness(
                            command_id,
                            "unready",
                            Some(detail.clone()),
                            &pass,
                            &plan,
                            &clock,
                        );
                        return Err(format!("node_network_not_ready: {detail}"));
                    }
                    std::thread::sleep(Duration::from_millis(NODE_NETWORK_POLL_MS));
                }
            }
        }
    }

    /// One polling pass over the planned node peer graph, proven with the
    /// shared `receiver_handshake_with_owned_reverse_tcp` evidence form.
    /// The observations collected before a failure are retained so the
    /// persisted readiness evidence shows what was actually seen.
    ///
    /// Contract version 3 flow: identity acquisition, then the before
    /// bracket (raw, bounded, stamped after its reads), its validation
    /// (host/node namespace identity, start ticks, listener ownership),
    /// then TWO peer samples (A and B, each stamped after its reads),
    /// then the after bracket and its validation, then the pure graph
    /// decision. The acquisition deadline is checked between phases:
    /// expiry during an otherwise-valid read makes the pass unready.
    fn observe_node_network(
        &self,
        plan: &PlannedNodeNetwork,
        request_id: &mut u64,
        clock: &AcquisitionClock,
    ) -> NodeNetworkPass {
        let refuse = |detail: String| GateFailure::Refuse { detail };
        let retry = |detail: String| GateFailure::Retry { detail };
        let mut pass = NodeNetworkPass {
            nodes: Vec::new(),
            covered: BTreeSet::new(),
            covered_dials: BTreeSet::new(),
            proofs: Vec::new(),
            failure: None,
            completed_at_mono_ms: None,
        };
        // Mid-acquisition expiry check: an otherwise-valid read that
        // lands after the whole-acquisition deadline cancels the pass
        // (unready; no retry, no deadline reset, no HashStart).
        let expired = |pass: &mut NodeNetworkPass| -> bool {
            match spc_lab_txd::bootstrap::monotonic_ms() {
                Ok(now) if now <= clock.deadline_mono_ms => false,
                Ok(_) => {
                    pass.failure = Some(retry(
                        "readiness acquisition deadline expired during the observation".into(),
                    ));
                    true
                }
                Err(error) => {
                    pass.failure = Some(refuse(format!("monotonic clock unreadable: {error}")));
                    true
                }
            }
        };
        // The shared proof layer revalidates the recorded plan (fail
        // closed on any half-wired or ambiguous shape).
        let planned_graph = match readiness::PlannedPeers::new(
            plan.listen
                .iter()
                .map(|(node, endpoint)| (node.to_string(), endpoint.clone()))
                .collect(),
            plan.connect
                .iter()
                .map(|(node, target)| (node.to_string(), target.clone()))
                .collect(),
        ) {
            Ok(peers) => peers,
            Err(detail) => {
                pass.failure = Some(refuse(format!(
                    "planned graph rejected by the proof layer: {detail}"
                )));
                return pass;
            }
        };
        let node_ids: BTreeMap<String, NodeId> = self
            .scenario
            .nodes
            .iter()
            .map(|spec| (spec.id.to_string(), spec.id.clone()))
            .collect();
        let controls = self.adapters.rpc_nodes.lock().unwrap().clone();
        // Identity acquisition: every planned node's process must still
        // be the supervisor-launched one, and both its listeners (the
        // bound RPC control listener and the planned P2P listener) must
        // be owned at their exact addresses before any peer is sampled.
        let mut identities = BTreeMap::new();
        let mut records = BTreeMap::new();
        let mut endpoints = BTreeMap::new();
        for spec in &self.scenario.nodes {
            let node = &spec.id;
            let Some(planned_listen) = plan.listen.get(node) else {
                continue;
            };
            let record = self
                .manifest
                .lock()
                .unwrap()
                .process("node", node.as_str())
                .cloned();
            let Some(record) = record else {
                pass.failure = Some(refuse(format!("node {node} has no process record")));
                return pass;
            };
            if self.supervisor.lock().unwrap().verify(&record)
                != crate::supervisor::Ownership::Owned
            {
                pass.failure = Some(refuse(format!("node {node} process is not owned")));
                return pass;
            }
            // Namespaced nodes: the host /proc view cannot prove
            // namespace-internal socket ownership, so the proof is
            // explicitly unavailable (fail closed). Skipping the check
            // is not evidence of a connection.
            if self.namespaces.contains_key(&format!("node/{node}")) {
                pass.failure = Some(refuse(format!(
                    "node {node} runs in a network namespace: the owned-socket readiness proof is unavailable there"
                )));
                return pass;
            }
            let Ok(rpc) = self.listener("node", node.as_str(), "rpc") else {
                pass.failure = Some(refuse(format!("node {node} rpc listener missing")));
                return pass;
            };
            let Ok(rpc_addr) = rpc.socket_addr() else {
                pass.failure = Some(refuse(format!(
                    "node {node} rpc listener endpoint {} is not a literal address",
                    rpc.host
                )));
                return pass;
            };
            let Ok(p2p_addr) = planned_listen.parse::<std::net::SocketAddr>() else {
                pass.failure = Some(refuse(format!(
                    "node {node} planned listen endpoint {planned_listen} is not a literal host:port"
                )));
                return pass;
            };
            if rpc_addr.ip().is_unspecified() || p2p_addr.ip().is_unspecified() {
                pass.failure = Some(refuse(format!(
                    "node {node} plans a wildcard listener: the exact-address ownership proof cannot apply"
                )));
                return pass;
            }
            // Listener ownership itself is proven from BOTH bracket
            // samples (contract version 3): the raw LISTEN rows and fd
            // membership before and after the peer samples, captured in
            // the brackets below.
            endpoints.insert(node.clone(), (rpc_addr, p2p_addr));
            identities.insert(
                node.to_string(),
                readiness::NodeProcessIdentity {
                    node: node.to_string(),
                    pid: record.pid,
                    start_time_ticks: record.start_time_ticks,
                    network_namespace: None,
                },
            );
            records.insert(node.clone(), record);
        }
        // The before bracket: raw identity/socket/namespace samples,
        // bounded by the source-declared cardinalities, with the
        // monotonic stamp taken AFTER its reads completed.
        let before = match readiness::capture_bracket(&identities, &clock.bounds) {
            Ok(before) => before,
            Err(detail) => {
                pass.failure = Some(refuse(format!(
                    "identity/socket bracket before sampling is unreadable: {detail}"
                )));
                return pass;
            }
        };
        if expired(&mut pass) {
            return pass;
        }
        // Validate the before bracket: the host controller's namespace
        // identity is an actual (non-None) value; every node's observed
        // namespace identity matches it; live start ticks still match
        // the records; both listeners (RPC and planned P2P) are owned
        // at their exact addresses from the RAW bracket rows.
        let Some(host_netns_before) = before.host_netns_identity else {
            pass.failure = Some(refuse(
                "host controller network namespace identity is unreadable".into(),
            ));
            return pass;
        };
        for (node, record) in &records {
            let Some(sample) = before.processes.get(node.as_str()) else {
                pass.failure = Some(refuse(format!(
                    "node {node} was not sampled in the before bracket"
                )));
                return pass;
            };
            match sample.netns_identity {
                Some(identity) if identity == host_netns_before => {}
                Some(identity) => {
                    pass.failure = Some(refuse(format!(
                        "node {node} lives in another network namespace ({identity:?}) than the host controller ({host_netns_before:?})"
                    )));
                    return pass;
                }
                None => {
                    pass.failure = Some(refuse(format!(
                        "network namespace identity of node {node} is unreadable"
                    )));
                    return pass;
                }
            }
            match sample.start_ticks {
                Some(live) if live == record.start_time_ticks => {}
                Some(live) => {
                    pass.failure = Some(refuse(format!(
                        "node {node} (pid {}) start time changed: recorded {}, live {live}",
                        record.pid, record.start_time_ticks
                    )));
                    return pass;
                }
                None => {
                    pass.failure = Some(refuse(format!(
                        "start time of node {node} (pid {}) is unreadable",
                        record.pid
                    )));
                    return pass;
                }
            }
            let (rpc_addr, p2p_addr) = endpoints[node];
            match readiness::listener_owned_in_sample(sample, rpc_addr) {
                readiness::ListenerOwnership::Owned => {}
                readiness::ListenerOwnership::NotOwned => {
                    pass.failure = Some(refuse(format!(
                        "node {node} rpc listener {rpc_addr} is not owned by the recorded process"
                    )));
                    return pass;
                }
                readiness::ListenerOwnership::Unreadable => {
                    pass.failure = Some(refuse(format!(
                        "node {node} rpc listener ownership is unreadable"
                    )));
                    return pass;
                }
            }
            match readiness::listener_owned_in_sample(sample, p2p_addr) {
                readiness::ListenerOwnership::Owned => {}
                readiness::ListenerOwnership::NotOwned => {
                    // The node may still be binding its P2P listener.
                    pass.failure = Some(retry(format!(
                        "node {node} p2p listener {p2p_addr} not yet owned by the recorded process"
                    )));
                    return pass;
                }
                readiness::ListenerOwnership::Unreadable => {
                    pass.failure = Some(refuse(format!(
                        "node {node} p2p listener ownership is unreadable"
                    )));
                    return pass;
                }
            }
        }
        // Strictly typed getpeerinfo rows; malformed rows fail closed
        // and the per-node row count is bounded by the source-declared
        // cardinality. Each sample is stamped AFTER its reads
        // completed.
        let peer_rows = |request_id: &mut u64,
                         stamp: &mut Option<u64>|
         -> Result<BTreeMap<String, Vec<readiness::PeerHandshakeRow>>, GateFailure> {
            let mut rows_by_node = BTreeMap::new();
            for spec in &self.scenario.nodes {
                let node = &spec.id;
                if !plan.listen.contains_key(node) {
                    continue;
                }
                let Some(control) = controls.get(node) else {
                    return Err(refuse(format!("node {node} has no typed RPC control")));
                };
                *request_id += 1;
                let peers =
                    match control.call_traffic(*request_id, "getpeerinfo", serde_json::json!([])) {
                        crate::traffic::rpc::RpcOutcome::Ok { result } => result,
                        _ => {
                            return Err(retry(format!("node {node} getpeerinfo unavailable")));
                        }
                    };
                let Some(entries) = peers.as_array() else {
                    return Err(refuse(format!("node {node} getpeerinfo is not an array")));
                };
                if entries.len() > clock.bounds.max_peer_rows {
                    return Err(refuse(format!(
                        "node {node} reports {} peer rows, over the source-declared bound {}",
                        entries.len(),
                        clock.bounds.max_peer_rows
                    )));
                }
                let mut rows = Vec::new();
                for entry in entries {
                    match readiness::PeerHandshakeRow::parse(entry) {
                        Ok(row) => rows.push(row),
                        Err(detail) => {
                            return Err(refuse(format!(
                                "node {node} getpeerinfo row is malformed: {detail}"
                            )));
                        }
                    }
                }
                rows_by_node.insert(node.to_string(), rows);
            }
            // The sample stamp is taken AFTER the reads completed.
            match spc_lab_txd::bootstrap::monotonic_ms() {
                Ok(now) => {
                    *stamp = Some(now);
                    Ok(rows_by_node)
                }
                Err(error) => Err(refuse(format!("monotonic clock unreadable: {error}"))),
            }
        };
        // Peer sample A (right after the before bracket).
        let mut stamp_a = None;
        let rows_a = match peer_rows(request_id, &mut stamp_a) {
            Ok(rows) => rows,
            Err(failure) => {
                pass.failure = Some(failure);
                return pass;
            }
        };
        if expired(&mut pass) {
            return pass;
        }
        // Peer sample B (right before the after bracket): the observed
        // peer set must be exactly stable across the acquisition.
        let mut stamp_b = None;
        let mut rows_b = match peer_rows(request_id, &mut stamp_b) {
            Ok(rows) => rows,
            Err(failure) => {
                pass.failure = Some(failure);
                return pass;
            }
        };
        if expired(&mut pass) {
            return pass;
        }
        let stamp_a = stamp_a.expect("peer sample A stamp");
        let stamp_b = stamp_b.expect("peer sample B stamp");
        let mut duals = BTreeMap::new();
        for (node, a_rows) in rows_a {
            let Some(b_rows) = rows_b.remove(&node) else {
                continue;
            };
            duals.insert(
                node,
                readiness::PeerDualSample {
                    a: readiness::PeerSample {
                        rows: a_rows,
                        sampled_at_mono_ms: stamp_a,
                    },
                    b: readiness::PeerSample {
                        rows: b_rows,
                        sampled_at_mono_ms: stamp_b,
                    },
                },
            );
        }
        // The after bracket (raw, bounded, stamped after its reads).
        let after = match readiness::capture_bracket(&identities, &clock.bounds) {
            Ok(after) => after,
            Err(detail) => {
                pass.failure = Some(refuse(format!(
                    "identity/socket bracket after sampling is unreadable: {detail}"
                )));
                return pass;
            }
        };
        if expired(&mut pass) {
            return pass;
        }
        // Validate the after bracket and its stability against the
        // before bracket: host namespace identity, per-node namespace
        // identity, start ticks, observed exe, listener ownership and
        // listener inode stability.
        match after.host_netns_identity {
            Some(identity) if identity == host_netns_before => {}
            Some(_) => {
                pass.failure = Some(refuse(
                    "host controller network namespace identity changed between bracket samples"
                        .into(),
                ));
                return pass;
            }
            None => {
                pass.failure = Some(refuse(
                    "host controller network namespace identity is unreadable".into(),
                ));
                return pass;
            }
        }
        for (node, record) in &records {
            let Some(sample_before) = before.processes.get(node.as_str()) else {
                continue;
            };
            let Some(sample_after) = after.processes.get(node.as_str()) else {
                pass.failure = Some(refuse(format!(
                    "node {node} was not sampled in the after bracket"
                )));
                return pass;
            };
            match (sample_before.netns_identity, sample_after.netns_identity) {
                (Some(left), Some(right)) if left == right => {}
                _ => {
                    pass.failure = Some(refuse(format!(
                        "network namespace identity of node {node} is unreadable or changed between bracket samples"
                    )));
                    return pass;
                }
            }
            match (sample_before.exe.as_deref(), sample_after.exe.as_deref()) {
                (Some(left), Some(right)) if left == right => {}
                _ => {
                    pass.failure = Some(refuse(format!(
                        "observed executable of node {node} is unreadable or changed between bracket samples"
                    )));
                    return pass;
                }
            }
            match sample_after.start_ticks {
                Some(live) if live == record.start_time_ticks => {}
                _ => {
                    pass.failure = Some(refuse(format!(
                        "node {node} process was replaced during the observation"
                    )));
                    return pass;
                }
            }
            let (rpc_addr, p2p_addr) = endpoints[node];
            let listener_inode = |sample: &readiness::ProcessSample,
                                  endpoint: std::net::SocketAddr| {
                sample
                    .tcp_rows
                    .iter()
                    .find(|row| row.state == readiness::TCP_STATE_LISTEN && row.local == endpoint)
                    .map(|row| row.inode)
            };
            for (endpoint, label) in [(rpc_addr, "rpc"), (p2p_addr, "p2p")] {
                match readiness::listener_owned_in_sample(sample_after, endpoint) {
                    readiness::ListenerOwnership::Owned => {}
                    readiness::ListenerOwnership::NotOwned if label == "p2p" => {
                        pass.failure = Some(retry(format!(
                            "node {node} p2p listener {endpoint} lost ownership during the observation"
                        )));
                        return pass;
                    }
                    readiness::ListenerOwnership::NotOwned => {
                        pass.failure = Some(refuse(format!(
                            "node {node} rpc listener {endpoint} lost ownership during the observation"
                        )));
                        return pass;
                    }
                    readiness::ListenerOwnership::Unreadable => {
                        pass.failure = Some(refuse(format!(
                            "node {node} {label} listener ownership is unreadable in the after bracket"
                        )));
                        return pass;
                    }
                }
                if let (Some(before_inode), Some(after_inode)) =
                    (listener_inode(sample_before, endpoint), listener_inode(sample_after, endpoint))
                {
                    if before_inode != after_inode {
                        pass.failure = Some(retry(format!(
                            "node {node} {label} listener {endpoint} was rebound during the observation"
                        )));
                        return pass;
                    }
                }
            }
        }
        // The sampled processes are still the supervisor-launched ones.
        for (node, record) in &records {
            if self.supervisor.lock().unwrap().verify(record) != crate::supervisor::Ownership::Owned
            {
                pass.failure = Some(refuse(format!(
                    "node {node} process was replaced during the observation"
                )));
                return pass;
            }
        }
        // The raw per-node evidence records: dual peer samples with
        // their stamps, raw listener observations from both brackets,
        // and the public nonsecret process identity bindings.
        for (node, record) in &records {
            let Some(dual) = duals.get(node.as_str()) else {
                continue;
            };
            let (rpc_addr, p2p_addr) = endpoints[node];
            let observation = |sample: Option<&readiness::ProcessSample>,
                               endpoint: std::net::SocketAddr| {
                sample.and_then(|sample| {
                    sample
                        .tcp_rows
                        .iter()
                        .find(|row| {
                            row.state == readiness::TCP_STATE_LISTEN && row.local == endpoint
                        })
                        .map(|row| ListenerObservation {
                            state: row.state.clone(),
                            inode: row.inode,
                            fd_membership: sample
                                .socket_inodes
                                .as_ref()
                                .is_some_and(|inodes| inodes.contains(&row.inode)),
                        })
                })
            };
            let sample_before = before.processes.get(node.as_str());
            let sample_after = after.processes.get(node.as_str());
            pass.nodes.push(NodeNetworkReadinessNode {
                node: node.clone(),
                outbound: dual
                    .b
                    .rows
                    .iter()
                    .filter(|row| !row.inbound)
                    .map(|row| row.addr.clone())
                    .collect(),
                inbound: dual
                    .b
                    .rows
                    .iter()
                    .filter(|row| row.inbound)
                    .map(|row| row.addr.clone())
                    .collect(),
                listener_owned: Some(true),
                peer_samples: dual.clone(),
                listener_evidence: NodeListenerEvidence {
                    rpc_listen: rpc_addr.to_string(),
                    p2p_listen: p2p_addr.to_string(),
                    rpc_before: observation(sample_before, rpc_addr),
                    rpc_after: observation(sample_after, rpc_addr),
                    p2p_before: observation(sample_before, p2p_addr),
                    p2p_after: observation(sample_after, p2p_addr),
                },
                process_binding: NodeProcessBinding {
                    run_id: self.run_id.to_string(),
                    pid: record.pid,
                    start_time_ticks: record.start_time_ticks,
                    executable_path: record.executable_path.display().to_string(),
                    executable_sha256: record.executable_sha256.clone(),
                    observed_exe: sample_before.and_then(|sample| sample.exe.clone()),
                    netns_identity: sample_before
                        .and_then(|sample| sample.netns_identity)
                        .map(|(dev, ino)| [dev, ino]),
                    host_netns_identity: before
                        .host_netns_identity
                        .map(|(dev, ino)| [dev, ino]),
                    boot_id: before.boot_id.clone(),
                },
            });
        }
        // The strict graph pass: every planned directed dial needs a
        // complete proof (a completed inbound handshake row stable
        // across both peer samples plus both process-owned
        // reverse-joined kernel sockets in both brackets).
        let graph =
            readiness::observe_peer_graph(&planned_graph, &duals, &identities, &before, &after);
        for proof in graph.proofs {
            if let (Some(dialer), Some(acceptor)) =
                (node_ids.get(&proof.dialer), node_ids.get(&proof.acceptor))
            {
                pass.covered_dials
                    .insert((dialer.clone(), acceptor.clone()));
                pass.covered.insert(edge_pair(dialer, acceptor));
            }
            pass.proofs.push(proof);
        }
        pass.failure = graph.failure.map(|failure| match failure {
            readiness::LinkProofFailure::Retry { detail } => GateFailure::Retry { detail },
            readiness::LinkProofFailure::Refuse { detail } => GateFailure::Refuse { detail },
        });
        // The completed stamp, taken after the LAST read of the pass
        // (the after bracket and the supervisor re-verification); the
        // caller checks it against the deadline before persisting
        // ready or reserving any hash budget.
        match spc_lab_txd::bootstrap::monotonic_ms() {
            Ok(now) => pass.completed_at_mono_ms = Some(now),
            Err(error) => {
                pass.failure = Some(refuse(format!("monotonic clock unreadable: {error}")))
            }
        }
        pass
    }

    /// Persist the before-hash peer/readiness evidence: the private
    /// readiness artifact and a journal evidence event. The
    /// whole-acquisition clock record (start, never-reset deadline,
    /// after-reads completion stamp, source-declared bounds) is part of
    /// every record (contract version 3).
    fn persist_node_network_readiness(
        &self,
        command_id: &CommandId,
        status: &str,
        detail: Option<String>,
        pass: &NodeNetworkPass,
        plan: &PlannedNodeNetwork,
        clock: &AcquisitionClock,
    ) {
        // Stamp the operation binding into every public proof record
        // (identity and clock bindings are already inside each proof).
        let mut proofs = pass.proofs.clone();
        for proof in &mut proofs {
            proof.command_id = Some(command_id.to_string());
        }
        let planned_dials: Vec<(NodeId, NodeId)> = plan
            .connect
            .iter()
            .filter_map(|(node, target)| {
                plan.listen_endpoint_owner(target, node)
                    .map(|owner| (node.clone(), owner))
            })
            .collect();
        // The completion stamp: the pass's after-last-read stamp, or
        // the current time for passes that never completed their reads
        // (conservative: a later stamp can only be past the deadline).
        let completed_at_mono_ms = pass.completed_at_mono_ms.unwrap_or_else(|| {
            spc_lab_txd::bootstrap::monotonic_ms()
                .unwrap_or(clock.deadline_mono_ms.saturating_add(1))
        });
        let report = NodeNetworkReadinessReport {
            status: status.to_string(),
            detail,
            contract_version: readiness::READINESS_CONTRACT_VERSION,
            evidence_form: Some(readiness::EVIDENCE_FORM.to_string()),
            acquisition: readiness::ReadinessAcquisitionRecord {
                started_at_mono_ms: clock.started_at_mono_ms,
                deadline_mono_ms: clock.deadline_mono_ms,
                completed_at_mono_ms,
                bounds: clock.bounds.clone(),
            },
            planned_edges: plan.edges.iter().cloned().collect(),
            covered_edges: pass.covered.iter().cloned().collect(),
            planned_dials,
            covered_dials: pass.covered_dials.iter().cloned().collect(),
            nodes: pass.nodes.clone(),
            proofs,
        };
        if let Err(error) = self
            .artifacts
            .write_private("traffic/node-network-readiness.json", &report)
        {
            self.evidence.append(
                SourceId::trusted("node-network-readiness"),
                format!("node-network-readiness-artifact-{command_id}"),
                EvidenceActor::harness(),
                Some(command_id.clone()),
                EvidencePayload::Inconclusive {
                    reason: format!("node network readiness artifact unwritable: {error}"),
                },
            );
        }
        self.evidence.append(
            SourceId::trusted("node-network-readiness"),
            format!("node-network-readiness-{command_id}"),
            EvidenceActor::harness(),
            Some(command_id.clone()),
            EvidencePayload::NodeNetworkReadiness {
                status: status.to_string(),
                report,
            },
        );
    }

    pub(super) fn begin_traffic_bootstrap(&self, command: &LabCommand) -> CommandOutcome {
        let id = command.command_id.clone();
        let mut began = false;
        let result = (|| -> Result<(), String> {
            let CommandKind::BootstrapFund { driver_id, node_id } = &command.kind else {
                return Err("not bootstrap".into());
            };
            if !self.state.lock().unwrap().started {
                return Err("traffic_not_active".into());
            }
            if self.manifest.lock().unwrap().traffic_stage != Some(TrafficStage::Provisioned) {
                return Err("traffic_bootstrap_duplicate".into());
            }
            if selected_bootstrap(&self.scenario).map_err(|e| e.to_string())?
                != (driver_id.clone(), node_id.clone())
            {
                return Err("traffic_bootstrap_route_invalid".into());
            }
            let executor = {
                let traffic = self.traffic.lock().unwrap();
                if traffic.operation.is_some() {
                    return Err("traffic_bootstrap_duplicate".into());
                }
                traffic
                    .executor
                    .clone()
                    .ok_or("traffic_bootstrap_executor_unavailable")?
            };
            let limits = &self.scenario.traffic.as_ref().unwrap().driver.bootstrap;
            let boot_id = spc_lab_txd::bootstrap::boot_id().map_err(|e| e.to_string())?;
            let deadline = spc_lab_txd::bootstrap::monotonic_ms()
                .map_err(|e| e.to_string())?
                .checked_add(
                    limits
                        .deadline_secs
                        .checked_mul(1000)
                        .ok_or("deadline overflow")?,
                )
                .ok_or("deadline overflow")?;
            // Observed-connectivity gate: the planned node peer graph must be
            // live BEFORE any hash budget is reserved. A run whose nodes have
            // zero peers must not debit hash budget waiting for funding that
            // can never propagate. The traffic lock is NOT held while the
            // gate polls, so an operator stop stays responsive.
            let readiness_deadline_ms = self.await_node_network_readiness(&id, deadline)?;
            let driver_control = {
                let traffic = self.traffic.lock().unwrap();
                if traffic.operation.is_some() {
                    return Err("traffic_bootstrap_duplicate".into());
                }
                traffic
                    .drivers
                    .get(driver_id)
                    .cloned()
                    .ok_or("bootstrap_driver_missing")?
            };
            // Checked budget start (contract version 3): the command log
            // acquires its state lock, samples a fresh CLOCK_MONOTONIC
            // reading and boot id INSIDE that guard, and validates them
            // immediately before the NEW HashStart write - the sampled
            // clock must be strictly before BOTH the never-reset
            // original operation deadline and the readiness acquisition
            // expiry carried out of the gate (equality expired,
            // consistent with reserve_hashes). The ready-record persist
            // above and any lock wait here consume real time; a stall
            // that carries the clock to either bound returns a rejected
            // precondition: `began` is still false, so no HashStart
            // exists, no hash budget is debited and no new operation id
            // or deadline is created. `began` becomes true only after
            // the HashStart write succeeds. An identical existing record
            // would replay idempotently (history, no new write).
            self.command_log
                .start_hash_budget_checked(
                    crate::command::traffic_budget::BudgetOperation {
                        operation_id: id.clone(),
                        driver_id: driver_id.to_string(),
                        boot_id: boot_id.clone(),
                        deadline_monotonic_ms: deadline,
                        total_attempts: limits.total_attempts,
                        hashes_per_second: limits.hashes_per_second,
                        cancel_grace_secs: limits.cancel_grace_secs,
                    },
                    readiness_deadline_ms,
                )
                .map_err(|e| e.to_string())?;
            began = true;
            let control = OperationControl::default();
            self.traffic.lock().unwrap().operation = Some((id.clone(), control.clone()));
            self.set_traffic_stage(TrafficStage::Bootstrapping)
                .map_err(|e| e.to_string())?;
            let request = BootstrapRequest {
                command_id: id.to_string(),
                driver_id: driver_id.to_string(),
                node_id: node_id.to_string(),
                boot_id,
                deadline_monotonic_ms: deadline,
                cancel_grace_secs: limits.cancel_grace_secs,
            };
            driver_control.open_bootstrap(Arc::clone(&self.command_log), &request)?;
            let engine = self
                .self_ref
                .lock()
                .unwrap()
                .upgrade()
                .ok_or("engine unavailable")?;
            let command_id = id.clone();
            std::thread::Builder::new()
                .name("traffic-bootstrap".into())
                .spawn(move || {
                    let host=Arc::new(ControllerBootstrapHost {engine:Arc::downgrade(&engine),request:request.clone(),control:control.clone()});
                    let result = executor.execute(request.clone(), control.clone(),host)
                        .and_then(|completion|engine.complete_traffic_bootstrap(&request,&control,&completion));
                    let _guard = engine.command_mutex.lock().unwrap();
                    let (state, detail) = if control.cancelled() {
                        (OutcomeState::Cancelled, "bootstrap cancelled".to_string())
                    } else {
                        match result {
                            Err(e) => (OutcomeState::Failed, e),
                            Ok(()) => (OutcomeState::Succeeded,"bootstrap distribution independently verified; measured phase active".into()),
                        }
                    };
                    let outcome = CommandOutcome::new(command_id.clone(), state, detail);
                    let durable=engine.command_log.finish(&command_id, &outcome);
                    engine.evidence.append(SourceId::trusted("command"),format!("bootstrap-finish-{command_id}"),EvidenceActor::harness(),Some(command_id.clone()),
                        EvidencePayload::Command(CommandObservation {command_id,state:outcome.state.as_str().into(),detail:outcome.detail.clone(),journal_seq:engine.command_log.last_seq()}));
                    if state!=OutcomeState::Succeeded || durable.is_err() {let _=engine.stop();let _=engine.set_manifest_state(RunState::Failed);}
                })
                .map_err(|e| e.to_string())?;
            Ok(())
        })();
        match result {
            Ok(()) => CommandOutcome::new(id, OutcomeState::Running, "bootstrap durably in flight"),
            Err(e) => {
                if began {
                    let _ = self.stop();
                    let _ = self.set_manifest_state(RunState::Failed);
                }
                CommandOutcome::new(
                    id,
                    if began {
                        OutcomeState::Failed
                    } else {
                        OutcomeState::RejectedPrecondition
                    },
                    e,
                )
            }
        }
    }

    fn bootstrap_checkpoint(
        &self,
        request: &BootstrapRequest,
        control: &OperationControl,
    ) -> Result<(), String> {
        if control.cancelled() {
            return Err("bootstrap_cancelled".into());
        }
        if request.boot_id != spc_lab_txd::bootstrap::boot_id().map_err(|e| e.to_string())?
            || spc_lab_txd::bootstrap::monotonic_ms().map_err(|e| e.to_string())?
                >= request.deadline_monotonic_ms
        {
            return Err("bootstrap_deadline_or_boot_changed".into());
        }
        if self.manifest_state() != RunState::Running {
            return Err("bootstrap_run_not_running".into());
        }
        Ok(())
    }

    fn complete_traffic_bootstrap(
        &self,
        request: &BootstrapRequest,
        control: &OperationControl,
        completion: &spc_lab_txd::bootstrap::BootstrapCompletion,
    ) -> Result<(), String> {
        self.bootstrap_checkpoint(request, control)?;
        let driver = ActorId::new(&request.driver_id).map_err(|e| e.to_string())?;
        let authority = NodeId::new(&request.node_id).map_err(|e| e.to_string())?;
        let path = spc_lab_txd::actor_config::driver_root(&self.run_dir, driver.as_str())
            .map_err(|e| e.to_string())?
            .join("actor.json");
        let config = ActorConfig::load(&path).map_err(|e| e.to_string())?;
        let mut rpc = CompletionRpc {
            nodes: self.adapters.rpc_nodes.lock().unwrap().clone(),
            control: control.clone(),
            deadline: request.deadline_monotonic_ms,
            next: 1,
        };
        let mut nodes: Vec<_> = self.scenario.nodes.iter().map(|n| n.id.clone()).collect();
        nodes.sort_by_key(|id| (*id != authority, id.clone()));
        let mut indices = Vec::new();
        for node in &nodes {
            self.bootstrap_checkpoint(request, control)?;
            indices.push(spc_lab_txd::observer::CanonicalIndex::scan(
                &mut rpc,
                node.as_str(),
                config.chain_policy.genesis,
                config.chain_policy.pow_limit()?,
            )?);
        }
        let spec = self.scenario.traffic.as_ref().ok_or("traffic_disabled")?;
        crate::traffic::completion::verify_distribution(
            completion,
            &indices,
            &config.allocation,
            &config.chain_policy,
            spec.bootstrap.funding_blocks,
            spec.bootstrap.distribution_outputs_per_wallet,
        )?;
        let drivers = self.traffic.lock().unwrap().drivers.clone();
        // Observation is asynchronous; snapshot reads are serviced during the
        // actor scan, and a committed tip is never treated as a scan cursor.
        for actor in drivers.values() {
            actor.request(None, "observe", serde_json::json!({}))?;
        }
        loop {
            self.bootstrap_checkpoint(request, control)?;
            let mut complete = true;
            for actor in drivers.values() {
                let snapshot = actor.request(None, "snapshot", serde_json::json!({}))?;
                complete &= snapshot["scan_complete"].as_bool() == Some(true)
                    && snapshot["tip"]["block_hash"].as_str()
                        == Some(completion.common_tip_hash.as_str())
                    && snapshot["tip"]["height"].as_u64() == Some(completion.common_tip_height);
            }
            if complete {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let host = drivers.get(&driver).ok_or("bootstrap_driver_missing")?;
        let command = CommandId::new(&request.command_id).map_err(|e| e.to_string())?;
        let status = host.request(Some(&command), "bootstrap_status", serde_json::json!({}))?;
        let recorded = status["confirmed_distribution_txids"]
            .as_array()
            .ok_or("bootstrap_journal_evidence_missing")?;
        if completion
            .distribution_txids
            .iter()
            .any(|id| !recorded.iter().any(|v| v.as_str() == Some(id.as_str())))
        {
            return Err("bootstrap_distribution_not_in_signer_journal".into());
        }
        let budget = self
            .command_log
            .hash_budget_snapshot(&command)
            .map_err(|e| e.to_string())?;
        if budget.outstanding.is_some() {
            return Err("bootstrap_hash_batch_unreconciled".into());
        }
        self.bootstrap_checkpoint(request, control)?;
        host.close_bootstrap();
        let boundary = crate::traffic::runtime::MeasuredBoundary {
            boot_id: request.boot_id.clone(),
            monotonic_ms: spc_lab_txd::bootstrap::monotonic_ms().map_err(|e| e.to_string())?,
            wall_ms: crate::command::now_ms(),
            tip_hash: completion.common_tip_hash.clone(),
            tip_height: completion.common_tip_height,
        };
        {
            let mut manifest = self.manifest.lock().unwrap();
            if manifest.traffic_measured_from.is_some() {
                return Err("traffic_measured_boundary_duplicate".into());
            }
            manifest.traffic_measured_from = Some(boundary.clone());
            manifest
                .save(&self.manifest_path)
                .map_err(|e| e.to_string())?;
        }
        // The boundary is durable before the FIRST AddConnection. This worker
        self.command_log
            .audit(crate::command::traffic_audit::Event::MeasuredBoundary {
                command_id: request.command_id.clone(),
                tip_hash: completion.common_tip_hash.clone(),
                tip_height: completion.common_tip_height,
            })
            .map_err(|e| e.to_string())?;
        self.evidence.append(
            SourceId::trusted("traffic-boundary"),
            format!("measured-{}", request.command_id),
            EvidenceActor::harness(),
            None,
            EvidencePayload::TrafficMeasured {
                boundary: boundary.clone(),
            },
        );
        // holds no global command mutex across bounded pipe/network waits.
        for miner in &self.scenario.miners {
            for connection in &miner.connections {
                self.bootstrap_checkpoint(request, control)?;
                let actor = self
                    .adapters
                    .miners
                    .lock()
                    .unwrap()
                    .get(&miner.id)
                    .cloned()
                    .ok_or("miner control missing")?;
                self.send_miner(
                    &actor,
                    MinerOperation::AddConnection,
                    self.connection_payload(connection, &miner.workers)
                        .map_err(|e| e.to_string())?,
                )
                .map_err(|(_, e)| e)?;
            }
        }
        for actor in drivers.values() {
            self.bootstrap_checkpoint(request, control)?;
            let result=actor.request(None,"activate",serde_json::json!({"boot_id":boundary.boot_id,"monotonic_ms":boundary.monotonic_ms}))?;
            if result["status"].as_str() != Some("active") {
                return Err("traffic_scheduler_activation_failed".into());
            }
        }
        self.bootstrap_checkpoint(request, control)?;
        self.set_traffic_stage(TrafficStage::Active)
            .map_err(|e| e.to_string())
    }

    pub(super) fn apply_traffic(&self, command: &LabCommand) -> CommandOutcome {
        let id = command.command_id.clone();
        let result = (|| -> Result<Value, String> {
            if self.scenario.traffic.is_none() {
                return Err("traffic_disabled".into());
            }
            match &command.kind {
                CommandKind::TrafficAcceptance { key, control } => {
                    if !self.scenario.expanded_access
                        || !self
                            .scenario
                            .traffic
                            .as_ref()
                            .is_some_and(|t| t.lab_acceptance)
                    {
                        return Err("traffic_acceptance_disabled".into());
                    }
                    let drivers = self.traffic.lock().unwrap();
                    let driver = drivers
                        .drivers
                        .get(&ActorId::new(&key.driver_id).map_err(|e| e.to_string())?)
                        .ok_or("acceptance_driver_missing")?;
                    if driver.incarnation() != key.incarnation {
                        return Err("acceptance_incarnation_mismatch".into());
                    }
                    let seq = driver.acceptance_control(key.clone(), control.clone())?;
                    Ok(serde_json::json!({"status":"recorded","audit_seq":seq}))
                }
                CommandKind::InjectTx {
                    driver_id,
                    adapter,
                    payload,
                } => {
                    if adapter != "rpc-signed" {
                        return Err("traffic_injection_adapter_unsupported".into());
                    }
                    let driver = driver_id
                        .as_ref()
                        .ok_or("traffic_injection_driver_required")?;
                    let (host, _) =
                        selected_bootstrap(&self.scenario).map_err(|e| e.to_string())?;
                    if driver != &host {
                        return Err("traffic_injection_not_bootstrap_host".into());
                    }
                    let request: crate::traffic::commands::RpcSignedTxPayload =
                        serde_json::from_value(payload.clone()).map_err(|e| e.to_string())?;
                    request.validate().map_err(|e| e.to_string())?;
                    let actor = self
                        .traffic
                        .lock()
                        .unwrap()
                        .drivers
                        .get(driver)
                        .cloned()
                        .ok_or("driver_unavailable")?;
                    actor.request(
                        Some(&id),
                        "inject",
                        serde_json::json!({"driver_id":driver,"raw_tx_hex":request.raw_tx_hex}),
                    )
                }
                CommandKind::BootstrapControl {
                    operation_id,
                    action,
                } => {
                    let traffic = self.traffic.lock().unwrap();
                    let (current, control) =
                        traffic.operation.as_ref().ok_or("bootstrap_not_running")?;
                    if current != operation_id {
                        return Err("bootstrap_operation_mismatch".into());
                    }
                    let control = control.clone();
                    let drivers = traffic.drivers.clone();
                    drop(traffic);
                    match action {
                        crate::command::BootstrapAction::Pause => control.pause(),
                        crate::command::BootstrapAction::Resume => control.resume(),
                        crate::command::BootstrapAction::Cancel => control.cancel(),
                    }
                    for actor in drivers.values() {
                        match action {
                            crate::command::BootstrapAction::Pause => actor.pause_bootstrap(true),
                            crate::command::BootstrapAction::Resume => actor.pause_bootstrap(false),
                            crate::command::BootstrapAction::Cancel => {
                                actor.close_gate();
                                actor.close_bootstrap();
                            }
                        }
                    }
                    Ok(serde_json::json!({"status":"control_recorded"}))
                }
                CommandKind::RestartTxDriver { driver_id } => {
                    self.reverify_traffic_binding().map_err(|e| e.to_string())?;
                    let path =
                        spc_lab_txd::actor_config::driver_root(&self.run_dir, driver_id.as_str())
                            .map_err(|e| e.to_string())?
                            .join("actor.json");
                    if !self
                        .scenario
                        .traffic
                        .as_ref()
                        .unwrap()
                        .driver
                        .actors
                        .iter()
                        .any(|a| &a.id == driver_id)
                    {
                        return Err("unknown_driver".into());
                    }
                    self.supervisor
                        .lock()
                        .unwrap()
                        .stop("tx_driver", driver_id.as_str(), Duration::from_secs(2))
                        .map_err(|e| e.to_string())?;
                    self.traffic.lock().unwrap().drivers.remove(driver_id);
                    self.launch_tx_driver(driver_id, &path)
                        .map_err(|e| e.to_string())?;
                    let actor = self
                        .traffic
                        .lock()
                        .unwrap()
                        .drivers
                        .get(driver_id)
                        .cloned()
                        .ok_or("driver_restart_missing")?;
                    let stage = self.manifest.lock().unwrap().traffic_stage;
                    if stage == Some(TrafficStage::Active) {
                        let boundary = self
                            .manifest
                            .lock()
                            .unwrap()
                            .traffic_measured_from
                            .clone()
                            .ok_or("traffic_measured_boundary_missing")?;
                        actor.request(None, "observe", serde_json::json!({}))?;
                        let restored=actor.request(None,"activate",serde_json::json!({"boot_id":boundary.boot_id,"monotonic_ms":boundary.monotonic_ms}))?;
                        if restored["status"].as_str() != Some("active") {
                            return Err("traffic_restart_activation_failed".into());
                        }
                    } else if stage == Some(TrafficStage::Bootstrapping) {
                        let operation = self
                            .traffic
                            .lock()
                            .unwrap()
                            .operation
                            .as_ref()
                            .map(|(id, _)| id.clone())
                            .ok_or("bootstrap_operation_unobservable")?;
                        let budget = self
                            .command_log
                            .hash_budget_snapshot(&operation)
                            .map_err(|e| e.to_string())?;
                        if budget.operation.driver_id == driver_id.as_str() {
                            let (_, node) =
                                selected_bootstrap(&self.scenario).map_err(|e| e.to_string())?;
                            actor.open_bootstrap(
                                Arc::clone(&self.command_log),
                                &BootstrapRequest {
                                    command_id: operation.to_string(),
                                    driver_id: driver_id.to_string(),
                                    node_id: node.to_string(),
                                    boot_id: budget.operation.boot_id,
                                    deadline_monotonic_ms: budget.operation.deadline_monotonic_ms,
                                    cancel_grace_secs: budget.operation.cancel_grace_secs,
                                },
                            )?;
                        }
                    }
                    Ok(serde_json::json!({"status":"restarted"}))
                }
                CommandKind::PauseTxDriver { driver_id }
                | CommandKind::ResumeTxDriver { driver_id } => {
                    let control = self
                        .traffic
                        .lock()
                        .unwrap()
                        .drivers
                        .get(driver_id)
                        .cloned()
                        .ok_or("unknown_driver")?;
                    if matches!(command.kind, CommandKind::PauseTxDriver { .. }) {
                        control.pause(&id)
                    } else {
                        control.resume(&id)
                    }
                }
                CommandKind::SendUserPayment {
                    user,
                    destination_address,
                    amount_sats,
                    fee_rate_sats_vb,
                }
                | CommandKind::AcceptanceVersionRejectionV1 {
                    user,
                    destination_address,
                    amount_sats,
                    fee_rate_sats_vb,
                } => {
                    let selected = matches!(
                        command.kind,
                        CommandKind::AcceptanceVersionRejectionV1 { .. }
                    );
                    if selected
                        && (!self
                            .scenario
                            .traffic
                            .as_ref()
                            .is_some_and(|t| t.lab_acceptance)
                            || !self.scenario.expanded_access
                            || !self.command_log.operator_owned(&id))
                    {
                        return Err(
                            "acceptance_construction_operator_or_enablement_required".into()
                        );
                    }
                    crate::traffic::commands::SendUserPayment {
                        user: user.clone(),
                        destination_address: destination_address.clone(),
                        amount_sats: *amount_sats,
                        fee_rate_sats_vb: *fee_rate_sats_vb,
                    }
                    .validate()
                    .map_err(|e| e.to_string())?;
                    let mut users: Vec<_> = self
                        .scenario
                        .identities
                        .iter()
                        .map(|u| u.id.clone())
                        .collect();
                    users.sort();
                    let index = users.binary_search(user).map_err(|_| "unknown_user")? as u32;
                    let mut actors = self
                        .scenario
                        .traffic
                        .as_ref()
                        .unwrap()
                        .driver
                        .actors
                        .clone();
                    actors.sort_by(|a, b| a.id.cmp(&b.id));
                    let mut end = 0;
                    let mut owner = None;
                    for actor in actors {
                        end += actor.user_payment_wallets;
                        if index < end {
                            owner = Some(actor.id);
                            break;
                        }
                    }
                    let owner = owner.ok_or("unallocated_user")?;
                    let control = self
                        .traffic
                        .lock()
                        .unwrap()
                        .drivers
                        .get(&owner)
                        .cloned()
                        .ok_or("driver_unavailable")?;
                    control.request(Some(&id),if selected {"acceptance_version_rejection_v1"} else {"send_user_payment"},serde_json::json!({"driver_id":owner,"user":user,"destination_address":destination_address,"amount_sats":amount_sats,"fee_rate_sats_vb":fee_rate_sats_vb}))
                }
                _ => Err("unsupported_traffic_operation".into()),
            }
        })();
        match result {
            Ok(v) => {
                let state = match v["status"].as_str() {
                    Some("recorded")
                        if matches!(command.kind, CommandKind::TrafficAcceptance { .. }) =>
                    {
                        OutcomeState::Succeeded
                    }
                    Some(
                        "accepted_by_rpc" | "paused" | "resumed" | "restarted" | "control_recorded",
                    ) => OutcomeState::Succeeded,
                    Some("failed" | "rejected") => OutcomeState::Failed,
                    Some("stored")
                        if matches!(
                            v["state"].as_str(),
                            Some("accepted_by_rpc" | "observed_in_mempool" | "confirmed")
                        ) =>
                    {
                        OutcomeState::Succeeded
                    }
                    _ => OutcomeState::Unknown,
                };
                CommandOutcome::new(id, state, v.to_string())
            }
            Err(e) => CommandOutcome::new(id, OutcomeState::Unknown, e),
        }
    }
}

/// Bounded wall-clock budget for the observed-connectivity gate.
const NODE_NETWORK_READINESS_TIMEOUT_MS: u64 = 90_000;
/// Poll interval while waiting for the planned peer graph to connect.
const NODE_NETWORK_POLL_MS: u64 = 500;

/// The whole-acquisition clock of one readiness gate run (contract
/// version 3): one monotonic start before the first read, one deadline
/// that is never reset (the bootstrap operation's own deadline capped
/// by [`NODE_NETWORK_READINESS_TIMEOUT_MS`]), and the source-declared
/// cardinality/RPC bounds serialized with every record.
#[derive(Debug, Clone)]
struct AcquisitionClock {
    started_at_mono_ms: u64,
    deadline_mono_ms: u64,
    bounds: readiness::ReadinessAcquisitionBounds,
}

/// The success-path deadline check (contract version 3), run BEFORE a
/// ready record is persisted and BEFORE the caller can reserve any
/// hash budget: a structurally valid observation is ready only when
/// BOTH its completed stamp and the fresh current clock are within the
/// never-reset acquisition deadline. `completed` is the pass's
/// after-last-read stamp; `now` is read fresh at the judgement point,
/// so a stall between the observation and this check can never turn an
/// expired acquisition ready, and a completed stamp later than the
/// current clock (a reversed stamp pair) is refused outright instead
/// of being substituted for the current time. The `Err` return keeps
/// the operation `began == false`, so no HashStart, no new operation
/// id and no timeout reset happen.
fn success_within_deadline(
    completed: Option<u64>,
    now: u64,
    deadline: u64,
) -> Result<(), String> {
    if let Some(stamp) = completed {
        if stamp > now {
            return Err(format!(
                "node_network_not_ready: readiness acquisition completed stamp {stamp} ms is later than the current clock {now} ms (reversed stamp pair)"
            ));
        }
        if stamp > deadline {
            return Err(format!(
                "node_network_not_ready: readiness acquisition completed at {stamp} ms, after the acquisition deadline {deadline} ms"
            ));
        }
    }
    if now > deadline {
        return Err(format!(
            "node_network_not_ready: readiness acquisition judged at {now} ms, after the acquisition deadline {deadline} ms"
        ));
    }
    Ok(())
}

/// The planned node peer graph, parsed from the recorded launch profiles.
/// Profiles stored in the manifest passed preflight `normalize`, so every
/// P2P flag is unambiguous. The per-endpoint dialer counts are recomputed
/// by the shared proof layer (`PlannedPeers::dialers_of`).
#[derive(Debug)]
struct PlannedNodeNetwork {
    /// node -> planned `--listen` endpoint.
    listen: BTreeMap<NodeId, String>,
    /// node -> planned `--connect` target.
    connect: BTreeMap<NodeId, String>,
    /// Undirected edges as sorted node-id pairs.
    edges: BTreeSet<(NodeId, NodeId)>,
}

impl PlannedNodeNetwork {
    /// The node (other than `excluding`) whose planned listener is `target`.
    fn listen_endpoint_owner(&self, target: &str, excluding: &NodeId) -> Option<NodeId> {
        self.listen
            .iter()
            .find(|(node, endpoint)| *endpoint == target && *node != excluding)
            .map(|(node, _)| node.clone())
    }
}

/// One polling pass over the planned node peer graph: what every node
/// reported, which planned directed dials are covered by complete typed
/// proofs, and why the pass is not ready (when it is not).
struct NodeNetworkPass {
    nodes: Vec<NodeNetworkReadinessNode>,
    covered: BTreeSet<(NodeId, NodeId)>,
    /// Planned directed dials (dialer, acceptor) proven by a complete
    /// `receiver_handshake_with_owned_reverse_tcp` proof.
    covered_dials: BTreeSet<(NodeId, NodeId)>,
    /// The public proof records collected this pass.
    proofs: Vec<readiness::DirectedDialProof>,
    failure: Option<GateFailure>,
    /// Monotonic stamp taken after the pass's LAST read completed;
    /// checked against the acquisition deadline on success before any
    /// ready persistence or hash budget reservation (contract version
    /// 3). `None` for passes that never completed their reads.
    completed_at_mono_ms: Option<u64>,
}

/// Why the readiness gate has not passed.
enum GateFailure {
    /// Transient: keep polling until the deadline.
    Retry { detail: String },
    /// Structural: the observation contradicts the plan; refuse.
    Refuse { detail: String },
}

impl GateFailure {
    fn detail(&self) -> &str {
        match self {
            Self::Retry { detail } | Self::Refuse { detail } => detail,
        }
    }
}

/// Sorted (undirected) node-id pair.
fn edge_pair(a: &NodeId, b: &NodeId) -> (NodeId, NodeId) {
    if a <= b {
        (a.clone(), b.clone())
    } else {
        (b.clone(), a.clone())
    }
}

/// Parse the planned node peer graph from the recorded launch profiles.
/// Fails closed (`node_network_plan_invalid`) on any half-wired or
/// ambiguous plan: a listen without a dial, an unknown dial target, or two
/// nodes sharing one listen endpoint.
fn planned_node_network(
    manifest: &RunManifest,
    scenario: &Scenario,
) -> Result<PlannedNodeNetwork, EngineError> {
    let invalid = |detail: String| EngineError::new("node_network_plan_invalid", detail);
    let mut listen = BTreeMap::new();
    let mut connect = BTreeMap::new();
    for node in &scenario.nodes {
        let Some(profile) = manifest.traffic_launch_profiles.get(&node.id) else {
            continue;
        };
        if let Some(value) = argv_flag_value(&profile.argv, "--listen") {
            listen.insert(node.id.clone(), value.to_string());
        }
        if let Some(value) = argv_flag_value(&profile.argv, "--connect") {
            connect.insert(node.id.clone(), value.to_string());
        }
    }
    if listen.keys().collect::<BTreeSet<_>>() != connect.keys().collect::<BTreeSet<_>>() {
        return Err(invalid("nodes mix --listen and --connect".into()));
    }
    if listen.values().collect::<BTreeSet<_>>().len() != listen.len() {
        return Err(invalid("two nodes plan the same listen endpoint".into()));
    }
    let mut edges = BTreeSet::new();
    for (node, target) in &connect {
        let Some(owner) = listen
            .iter()
            .find(|(listener, endpoint)| **endpoint == **target && *listener != node)
            .map(|(listener, _)| listener.clone())
        else {
            return Err(invalid(format!(
                "node {node} dials unplanned endpoint {target}"
            )));
        };
        edges.insert(edge_pair(node, &owner));
    }
    Ok(PlannedNodeNetwork {
        listen,
        connect,
        edges,
    })
}

/// Fixture seams for the Runtime phase regression (`runtime.rs` tests).
/// Production reaches both states only through `launch_tx_drivers` and
/// `start`; nothing outside `cfg(test)` can call these.
#[cfg(test)]
impl RunEngine {
    /// Install a scripted tx-driver control without launching the real
    /// actor stack. The control must still speak the real control protocol
    /// on its pipes; only the process behind it is a fixture.
    pub(crate) fn install_test_tx_driver(&self, driver: ActorId, control: Arc<TxDriverControl>) {
        self.traffic.lock().unwrap().drivers.insert(driver, control);
    }

    /// Mark the run started exactly as `start` does after `launch_all`
    /// succeeds, without launching any process.
    pub(crate) fn mark_test_started(&self) {
        self.state.lock().unwrap().started = true;
    }
}

#[cfg(test)]
#[path = "acceptance_followup_tests.rs"]
mod acceptance_followup_tests;

#[cfg(test)]
#[path = "reap_cleanup_tests.rs"]
mod reap_cleanup_tests;

#[cfg(test)]
mod readiness_gate_tests {
    use super::*;

    /// The success-path deadline check: a pass whose every obligation
    /// is proven is STILL unready when its completed stamp exceeds the
    /// never-reset acquisition deadline (slow valid success past the
    /// deadline). The Err return keeps the operation `began == false`,
    /// so no HashStart, no new operation id and no timeout reset.
    #[test]
    fn valid_pass_completed_after_the_deadline_is_unready() {
        let err = success_within_deadline(Some(10_500), 10_600, 10_000)
            .expect_err("a completed stamp past the deadline must be unready");
        assert!(err.starts_with("node_network_not_ready:"), "{err}");
        assert!(err.contains("after the acquisition deadline"), "{err}");
    }

    /// A valid pass completed within the deadline with a fresh current
    /// clock also inside it is ready (normal positive; both deadline
    /// boundaries are inclusive).
    #[test]
    fn valid_pass_within_the_deadline_is_ready() {
        assert!(success_within_deadline(Some(9_999), 10_000, 10_000).is_ok());
        assert!(success_within_deadline(Some(10_000), 10_000, 10_000).is_ok());
    }

    /// A late CURRENT clock refuses even when the completed stamp is on
    /// time: the gap between the observation and the judgement can
    /// never turn an expired acquisition ready, and the completed stamp
    /// is never substituted for the current time (late-now negative).
    #[test]
    fn late_current_clock_with_on_time_completed_is_unready() {
        let err = success_within_deadline(Some(10_000), 10_600, 10_000)
            .expect_err("a fresh clock past the deadline must be unready");
        assert!(err.starts_with("node_network_not_ready:"), "{err}");
        assert!(err.contains("judged at 10600 ms"), "{err}");
    }

    /// A completed stamp later than the current clock is a reversed
    /// stamp pair and is refused outright: no clock substitution in
    /// either direction.
    #[test]
    fn reversed_stamp_pair_is_refused() {
        let err = success_within_deadline(Some(10_600), 10_500, 11_000)
            .expect_err("a completed stamp after the current clock must be refused");
        assert!(err.starts_with("node_network_not_ready:"), "{err}");
        assert!(err.contains("later than the current clock"), "{err}");
    }

    /// A pass without a completed stamp is judged by the current time:
    /// the gap between validation and persist/start can never turn an
    /// expired acquisition ready (end-between-validation-and-start).
    #[test]
    fn missing_completed_stamp_is_judged_by_now() {
        assert!(success_within_deadline(Some(9_000), 9_500, 10_000).is_ok());
        let err = success_within_deadline(None, 10_001, 10_000)
            .expect_err("now past the deadline with no stamp must be unready");
        assert!(err.contains("judged at 10001 ms"), "{err}");
    }

    /// The pre-HashStart transition semantics are enforced by the
    /// command log's checked budget start (see
    /// traffic_budget::CommandLog::start_hash_budget_checked and its
    /// locked-clock tests): a slow ready-record persist or command-log
    /// lock wait that carries the fresh clock to either the original
    /// operation deadline or the carried acquisition expiry refuses
    // inside the guard, before any HashStart write or hash debit.

    /// The acquisition bounds are source-declared constants, not
    /// derived from any observed system: the serialized record a
    /// consumer cross-checks carries exactly these trusted numbers.
    #[test]
    fn source_declared_bounds_are_the_documented_constants() {
        assert_eq!(readiness::READINESS_MAX_RPC_CALL_MS, 10_000);
        assert_eq!(readiness::READINESS_MAX_PEER_ROWS, 256);
        assert_eq!(readiness::READINESS_MAX_TCP_ROWS, 8_192);
        assert_eq!(readiness::READINESS_MAX_FD_ENTRIES, 8_192);
    }
}
