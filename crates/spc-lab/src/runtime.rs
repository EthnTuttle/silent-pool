//! Built-in real-process runtime.
//!
//! This module turns a versioned scenario into actual node, pool, miner,
//! Redis and PostgreSQL processes. It renders every config file itself, keeps
//! all secrets in mode-0600 private artifacts, and never falls back to a
//! fixture when a real adapter is requested. A fixture run stays explicit in
//! its own manifest; nothing here relabels fixture evidence as live.
//!
//! With `--topology`, the controller remains in the host user/network
//! namespaces with one engine, lock and listener. Isolated actors enter the
//! broker user namespace and their endpoint netns at launch. PostgreSQL stays
//! host-local on its private Unix socket; each Redis is isolated in its pool's
//! storage namespace and reached on its authenticated published address.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use rand::RngCore;
use secp256k1::{PublicKey, Secp256k1, SecretKey};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::adapters::RpcNodeControl;
use crate::command::{CommandKind, LabCommand};
use crate::config::{AdapterConfig, LabConfig};
use crate::convergence::{drive_convergence, shared_tip, ConvergenceDriver};
use crate::engine::{EngineOptions, MinerProvisioner, RunEngine, StartupHook};
use crate::evidence::{
    ActorKind, AtomicTipObservation, BrokerCallObservation, BrokerEvidenceObservation,
    CanonicalResult, ConsensusCounterObservation, ControllerObservation, EvidenceActor,
    EvidenceEvent, EvidencePayload, FrontApiObservation, SharedChainAnchor, TopologyPhaseEvidence,
    UnknownEvidence, TOPOLOGY_EVIDENCE_SCHEMA_VERSION,
};
use crate::ids::{ActorId, FrontId, IdentityId, MinerId, NodeId, SourceId};
use crate::links::netns::NetnsLinkBackend;
use crate::namespace::{BrokerOptions, NamespaceBroker, Placement};
use crate::resource::{Endpoint, PostgresRuntime, RedisRuntime};
use crate::scenario::{AddressSource, FrontRole, Scenario, WindowRule};
use crate::spc_evidence::{SpcEvidenceAdapter, SpcEvidenceMappings};
use crate::topology::{TopologyPlan, TopologyPublication};

/// The throwaway watch-only regtest descriptor used by the approved live lab.
pub const POOL_PAYOUT_DESCRIPTOR: &str =
    "tr(tpubD6NzVbkrYhZ4WUjXBVxZPvGhWxTRE5veAscTgokX65xcfCoDF5nyNdtaS4xGoJ7xnZuDTxCAEAnGpxJbWo6wtTAJsESmeJWpZzybZ6dvCia/0/*)";
/// Structurally valid P2TR output script matching the descriptor fixture above.
pub const NODE_CAPABILITIES: [&str; 3] = [
    "spc.node.regtest.pow_baseline",
    "spc.node.regtest.retarget",
    "spc.node.real",
];

pub const POOL_PAYOUT_SCRIPT: &str =
    "5120a60869f0dbcf1dc659c9cecbaf8050135ea9e8cdc487053f1dc6880949dc684c";

#[derive(Debug, Clone)]
pub struct RuntimeBinaries {
    pub txd: Option<PathBuf>,
    pub traffic_probe: Option<PathBuf>,
    pub traffic_probe_args: Vec<String>,
    pub node: PathBuf,
    pub pool: PathBuf,
    pub miner: PathBuf,
    pub scanner: Option<PathBuf>,
    /// Directory containing `initdb`, `postgres`, `createdb` and `psql`.
    pub postgres_bin_dir: PathBuf,
    pub redis_server: PathBuf,
    pub redis_cli: PathBuf,
}

impl RuntimeBinaries {
    pub fn postgres_tool(&self, name: &str) -> PathBuf {
        self.postgres_bin_dir.join(name)
    }

    pub fn verify(&self) -> Result<()> {
        for path in [&self.txd, &self.traffic_probe].into_iter().flatten() {
            if !path.is_file() {
                bail!("traffic runtime executable missing: {}", path.display());
            }
        }
        for (name, path) in [
            ("node", &self.node),
            ("pool", &self.pool),
            ("miner", &self.miner),
            ("redis-server", &self.redis_server),
            ("redis-cli", &self.redis_cli),
            ("initdb", &self.postgres_tool("initdb")),
            ("postgres", &self.postgres_tool("postgres")),
            ("createdb", &self.postgres_tool("createdb")),
            ("psql", &self.postgres_tool("psql")),
        ] {
            if !path.is_file() {
                bail!("runtime binary {name} does not exist: {}", path.display());
            }
        }
        if let Some(scanner) = &self.scanner {
            if !scanner.is_file() {
                bail!(
                    "runtime scanner binary does not exist: {}",
                    scanner.display()
                );
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeOptions {
    pub wallet_agent: Option<PathBuf>,
    pub ark: Option<crate::ark_service::ArkBinaries>,
    pub scenario_path: PathBuf,
    pub run_dir: PathBuf,
    pub binaries: RuntimeBinaries,
    pub pow_bits: String,
    pub base_revision: String,
    pub dirty_tree_hash: String,
    pub evidence_capacity: usize,
    /// Place the actors in network namespaces owned by the namespace broker.
    pub topology: Option<TopologyRuntimeOptions>,
    /// Render the approved EXP-OP read-only operator page into every pool
    /// config (one extra loopback listener per front, `front.<id>.operator`)
    /// and publish its URL only after a route-readiness proof. Off by
    /// default; host runs only - a namespace (topology) run rejects it
    /// before any effect until a reviewed controller relay exists.
    pub operator_ui: bool,
}

/// How the runtime reaches the namespace broker.
#[derive(Debug, Clone)]
pub struct TopologyRuntimeOptions {
    /// The broker implementation (the approved LP-4a namespace backend).
    pub broker_script: PathBuf,
    /// The `unshare` binary that creates the broker's namespaces.
    pub unshare: PathBuf,
    /// The `nsenter` binary that joins a namespace.
    pub nsenter: PathBuf,
}

/// The live topology: the broker, the plan and the published placement.
pub struct TopologyRuntime {
    pub broker: Arc<NamespaceBroker>,
    pub plan: TopologyPlan,
    pub placement: Placement,
    /// `role/entity` -> the namespace the actor lives in.
    pub namespaces: BTreeMap<String, String>,
    /// `role/entity` -> verified network-namespace holder identity.
    pub namespace_owners: BTreeMap<String, crate::supervisor::NamespaceOwner>,
    /// The broker user namespace every isolated actor enters before its netns.
    pub user_namespace: PathBuf,
    /// Start-time identity of the broker user-namespace owner.
    pub user_namespace_owner: crate::supervisor::NamespaceOwner,
    /// The broker's control socket, for the stop path.
    pub socket_path: PathBuf,
    /// The published addresses, keyed by `<kind>.<entity>.<name>`.
    pub addresses: BTreeMap<String, String>,
}

impl TopologyRuntime {
    /// The broker endpoint an actor lives in.
    pub fn endpoint_of(&self, actor: &str) -> Option<&str> {
        self.plan.endpoint_of(actor)
    }

    /// The published address of an endpoint on a link.
    pub fn address_of(&self, endpoint: &str, link: &str) -> Option<&str> {
        self.placement
            .address(endpoint, link)
            .map(|address| address.addr.as_str())
    }

    /// The link that joins two endpoints, if there is one.
    pub fn link_between(&self, a: &str, b: &str) -> Option<String> {
        self.plan
            .links
            .iter()
            .find(|link| (link.a == a && link.b == b) || (link.a == b && link.b == a))
            .map(|link| link.id.clone())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrivateIdentity {
    pub id: String,
    pub address: String,
    pub sp_id: String,
    pub b_scan: String,
    pub b_spend: String,
    pub scan_key: String,
    pub spend_key: String,
}

impl PrivateIdentity {
    pub fn scan_secret(&self) -> Result<SecretKey> {
        let bytes = hex::decode(&self.scan_key).context("scan key is not hex")?;
        SecretKey::from_slice(&bytes).context("scan key is not a scalar")
    }

    pub fn spend_public(&self) -> Result<PublicKey> {
        let bytes = hex::decode(&self.b_spend).context("b_spend is not hex")?;
        PublicKey::from_slice(&bytes).context("b_spend is not a point")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeSecrets {
    node_rpc_user: String,
    node_rpc_password: String,
    #[serde(default)]
    redis: BTreeMap<String, String>,
    #[serde(default)]
    postgres: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
struct NodeAccess {
    id: NodeId,
    endpoint: Endpoint,
    user: String,
    password: String,
    /// Private mode-0600 file used by all namespace-local helpers.
    cookie: PathBuf,
    /// The broker endpoint of the node, when the run is namespaced.
    broker_endpoint: Option<String>,
}

impl NodeAccess {
    fn control(&self, topology: Option<&Arc<TopologyRuntime>>) -> RpcNodeControl {
        let mut control = RpcNodeControl::new(
            self.id.clone(),
            self.endpoint.clone(),
            Some(format!("basic:{}:{}", self.user, self.password)),
        );
        if let (Some(topology), Some(endpoint)) = (topology, self.broker_endpoint.as_ref()) {
            control = control.with_auth_file(&self.cookie);
            control = control.in_namespace(crate::adapters::NamespaceClient {
                broker: Arc::clone(&topology.broker),
                endpoint: endpoint.clone(),
            });
        }
        control
    }
}

struct RuntimeConvergenceDriver<'a> {
    runtime: &'a Runtime,
}

impl ConvergenceDriver for RuntimeConvergenceDriver<'_> {
    fn observe(&mut self) -> Vec<AtomicTipObservation> {
        self.runtime.node_views()
    }

    fn worker_control(&mut self, resume: bool) -> Result<Vec<Value>, String> {
        self.runtime
            .convergence_workers(resume)
            .map_err(|error| error.to_string())
    }

    fn stop_requested(&self) -> bool {
        terminal(&self.runtime.engine)
    }

    fn wait(&mut self, duration: Duration) {
        thread::sleep(duration);
    }
}

#[derive(Debug, Clone)]
struct PoolRender {
    front: FrontId,
    numeric: u8,
    config_path: PathBuf,
}

/// A prepared but not yet started real run.
pub struct Runtime {
    pub engine: Arc<RunEngine>,
    run_dir: PathBuf,
    binaries: RuntimeBinaries,
    identities: Vec<PrivateIdentity>,
    nodes: Vec<NodeAccess>,
    pools: Vec<PoolRender>,
    observers: Option<Observers>,
    topology: Option<Arc<TopologyRuntime>>,
}

impl Runtime {
    pub fn prepare(options: RuntimeOptions) -> Result<Self> {
        // EXP-OP runtime wiring: the operator page is a host-run option in
        // this slice. A namespace run has no reviewed controller relay for a
        // host browser to reach an isolated front's loopback listener, so the
        // combination is rejected here - first statement of `prepare`, before
        // any binary check, filesystem effect or port lease exists.
        if options.operator_ui && options.topology.is_some() {
            bail!(
                "--operator-ui is host-runtime only in this slice: a namespace (--topology) \
                 run must not enable the operator page until a reviewed controller relay exists"
            );
        }
        options.binaries.verify()?;
        let scenario_text = std::fs::read_to_string(&options.scenario_path)
            .with_context(|| format!("read scenario {}", options.scenario_path.display()))?;
        let mut scenario =
            Scenario::from_json(&scenario_text).map_err(|error| anyhow::anyhow!("{error}"))?;
        crate::ark_service::validate_runtime(&scenario, &options)?;
        if !scenario.wallet_agents.is_empty() {
            let binary = options
                .wallet_agent
                .as_ref()
                .context("wallet agents require --wallet-agent-bin")?;
            if !binary.is_file() || !binary.canonicalize()?.starts_with("/nix/store") {
                bail!("wallet-agent binary must be a Nix store executable");
            }
        }
        if scenario.traffic.is_some() {
            if options.binaries.traffic_probe.is_none() {
                bail!("traffic_probe_unconfigured");
            }
            if options.binaries.txd.is_none() {
                bail!("traffic_driver_unconfigured");
            }
            RunEngine::validate_traffic_workflow(&scenario)?;
        }
        // The operator listener runs inside the pool API process (EXP-OP
        // decision, `OperatorRequiresApiRole`), so a front without the api
        // role cannot carry it. Rejecting here - before the run directory,
        // identities or port leases exist - keeps the failure a clear
        // preparation error instead of a pool process that dies on its own
        // config validation.
        if options.operator_ui {
            for front in &scenario.fronts {
                if !front.roles.contains(&FrontRole::Api) {
                    bail!(
                        "front {} has no api role; the EXP-OP operator listener runs inside \
                         the pool API process, so --operator-ui requires the api role on every \
                         front",
                        front.id
                    );
                }
            }
        }
        let run_dir = options.run_dir.clone();
        std::fs::create_dir_all(&run_dir)
            .with_context(|| format!("create run dir {}", run_dir.display()))?;
        let private = run_dir.join("private");
        std::fs::create_dir_all(&private)?;
        std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o700))?;

        if let Some(binary) = &options.wallet_agent {
            crate::wallet_bridge::provision(&scenario, &private, binary)?;
        }
        let identities = load_or_generate_identities(&scenario, &private)?;
        apply_identity_addresses(&mut scenario, &identities);
        let secrets = load_or_generate_secrets(&mut scenario, &private)?;

        // The topology is realised before anything is rendered: the placement
        // is the only source of an address, and it has to exist before a
        // config can name it.
        let topology = start_topology(&scenario, &options, &run_dir)?;
        let addresses = topology
            .as_ref()
            .map(|topology| topology.addresses.clone())
            .unwrap_or_default();
        let config = build_lab_config(&scenario, &options, &secrets)?;
        let engine = RunEngine::prepare(
            scenario,
            EngineOptions {
                run_dir: run_dir.clone(),
                config,
                base_revision: options.base_revision.clone(),
                dirty_tree_hash: options.dirty_tree_hash.clone(),
                owner_instance: format!("runtime-{}", std::process::id()),
                evidence_capacity: options.evidence_capacity.max(1_000),
                operator_ui: options.operator_ui,
                namespaces: topology
                    .as_ref()
                    .map(|topology| topology.namespaces.clone())
                    .unwrap_or_default(),
                addresses: addresses.clone(),
                nsenter: topology
                    .as_ref()
                    .and_then(|topology| topology.broker.tool_path("nsenter")),
                user_namespace: topology
                    .as_ref()
                    .map(|topology| topology.user_namespace.clone()),
                user_namespace_owner: topology
                    .as_ref()
                    .map(|topology| topology.user_namespace_owner.clone()),
                namespace_owners: topology
                    .as_ref()
                    .map(|topology| topology.namespace_owners.clone())
                    .unwrap_or_default(),
                namespace_broker: topology
                    .as_ref()
                    .map(|topology| Arc::clone(&topology.broker)),
                lifetime_helper: Some(
                    std::env::current_exe().context("the controller executable path is unknown")?,
                ),
                link_backend: topology.as_ref().map(|topology| {
                    Arc::new(NetnsLinkBackend::new(Some(Arc::clone(&topology.broker))))
                        as Arc<dyn crate::links::LinkBackend>
                }),
            },
        )
        .map_err(|error| anyhow::anyhow!("{error}"))?;

        // The placement is recorded before the configs are rendered, so the
        // manifest always carries the addresses the actors were given.
        if let Some(topology) = &topology {
            let publication = TopologyPublication::with_provenance(
                &topology.placement,
                &topology.plan,
                &topology.broker.run_dir().to_string_lossy(),
                json!({
                    "base_revision": options.base_revision,
                    "dirty_tree_hash": options.dirty_tree_hash,
                    "broker_snapshot": "approved LP-4a namespace broker (bce93151)",
                }),
            );
            engine.record_placement(publication.shareable.clone())?;
        }

        let manifest = engine.resources_manifest();
        render_node_files(&engine, &run_dir, &secrets, &options.pow_bits)?;
        render_redis_configs(
            &run_dir,
            &manifest.redis_instances,
            &secrets,
            topology.as_ref(),
        )?;
        initialise_postgres(
            &run_dir,
            &manifest.postgres_databases,
            &options.binaries,
            &secrets,
        )?;
        let pools = render_pool_configs(
            &engine,
            &run_dir,
            &manifest.redis_instances,
            &manifest.postgres_databases,
            &secrets,
            topology.as_ref(),
            options.operator_ui,
        )?;
        write_initial_miner_configs(&engine, &run_dir)?;

        let nodes = collect_node_access(&engine, &manifest.listeners, &secrets, topology.as_ref())?;
        engine.set_startup_hook(Arc::new(PostgresHook {
            binaries: options.binaries.clone(),
            databases: manifest.postgres_databases.clone(),
            secrets: secrets.clone(),
            ark: options.ark.clone(),
            wallet_agent: options.wallet_agent.clone(),
        }));
        engine.set_miner_provisioner(Arc::new(MinerConfigProvisioner {
            run_dir: run_dir.clone(),
        }));

        Ok(Self {
            engine,
            run_dir,
            binaries: options.binaries,
            identities,
            nodes,
            pools,
            observers: None,
            topology,
        })
    }

    /// Start every configured process and the live evidence collectors.
    pub fn start(&mut self) -> Result<()> {
        self.engine
            .start()
            .map_err(|error| anyhow::anyhow!("{error}"))?;
        self.observers = Some(Observers::start(
            self.engine.clone(),
            &self.run_dir,
            &self.binaries,
            &self.identities,
            &self.nodes,
            &self.pools,
            self.topology.as_ref(),
        )?);
        Ok(())
    }

    /// Live host runner. `listen` exposes the durable HTTP/SSE API and the
    /// browser controls; without it the run is still alive until `duration`
    /// elapses or a durable stop command is submitted.
    pub fn run(
        self,
        listen: Option<String>,
        duration: Option<Duration>,
        interactive: bool,
    ) -> Result<()> {
        self.run_with_assets(
            listen,
            duration,
            interactive,
            crate::http::WebAssets::fallback(),
        )
    }

    pub fn run_with_assets(
        mut self,
        listen: Option<String>,
        duration: Option<Duration>,
        interactive: bool,
        assets: crate::http::WebAssets,
    ) -> Result<()> {
        self.start()?;
        self.publish_operator_urls();
        let http = match listen {
            Some(bind) => {
                let server = crate::http::HttpServer::start(self.engine.clone(), &bind, assets)
                    .map_err(|error| anyhow::anyhow!("{error}"))?;
                println!("spc-lab-runtime: http={}", server.url());
                Some(server)
            }
            None => None,
        };
        let controller_url = http.as_ref().map(|server| server.url());
        let topology_report = match self.execute_phases(controller_url.as_deref()) {
            Ok(report) => report,
            Err(error) => {
                if let Some(observers) = self.observers.take() {
                    observers.stop();
                }
                let _ = self.engine.stop();
                return Err(error);
            }
        };
        let deadline = duration.map(|value| Instant::now() + value);
        if interactive {
            let engine = self.engine.clone();
            let run_id = engine.run_id().clone();
            std::thread::spawn(move || {
                let stdin = std::io::stdin();
                let mut line = String::new();
                while std::io::BufRead::read_line(&mut stdin.lock(), &mut line).unwrap_or(0) > 0 {
                    if line.trim() == "stop" {
                        let command = LabCommand::new(
                            run_id.clone(),
                            ActorId::trusted("runtime-stdin"),
                            CommandKind::StopRun {
                                reason: "interactive stdin stop".to_string(),
                            },
                        );
                        let _ = engine.submit(command);
                        return;
                    }
                    line.clear();
                }
            });
        }
        loop {
            if terminal(&self.engine) {
                break;
            }
            if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                let command = LabCommand::new(
                    self.engine.run_id().clone(),
                    ActorId::trusted("runtime"),
                    CommandKind::StopRun {
                        reason: "runtime duration elapsed".to_string(),
                    },
                );
                let outcome = self.engine.submit(command);
                if !matches!(
                    outcome.state,
                    crate::command::OutcomeState::Succeeded
                        | crate::command::OutcomeState::RejectedPrecondition
                ) {
                    eprintln!("runtime stop command: {outcome:?}");
                }
                break;
            }
            thread::sleep(Duration::from_millis(200));
        }
        if let Some(observers) = self.observers.take() {
            observers.stop();
        }
        if !terminal(&self.engine) {
            self.engine
                .stop()
                .map_err(|error| anyhow::anyhow!("{error}"))?;
        }
        let summary = self.emit_oracle_summary();
        let _ = self.engine.refresh_artifacts();
        self.write_oracle_result(&summary, &topology_report);
        if let Some(server) = http {
            server.shutdown();
        }
        Ok(())
    }

    /// Prove every rendered operator page is actually served, then publish
    /// its URL (EXP-OP). An HTTP 200 on `/` proves the page route exists;
    /// it is NOT a claim about funds health. The observation JSON probe is
    /// informational only and never gates publishing. A front whose page
    /// probe fails gets a warning and no URL; its evidence fact records
    /// `page_served: false` so the hub inventory stays silent for it.
    fn publish_operator_urls(&self) {
        let listeners = self.engine.resources_manifest().listeners;
        for front in &self.engine.scenario().fronts {
            let Some(endpoint) = listeners.get(&format!("front.{}.operator", front.id)) else {
                continue;
            };
            // Defense in depth: the port allocator only hands out loopback
            // endpoints and `--operator-ui` is host-only, but the published
            // URL is asserted, not assumed. A non-loopback listener is never
            // probed and never published.
            let loopback = endpoint.host == "127.0.0.1" || endpoint.host == "::1";
            let url = format!("http://{}/", endpoint.display());
            let page_served = loopback && operator_page_served(&url);
            let observation_route = if page_served {
                operator_observation_route_present(&url)
            } else {
                None
            };
            self.engine.evidence().append(
                SourceId::trusted("operator"),
                format!(
                    "operator-readiness-{}",
                    self.engine.evidence().last_event_id()
                ),
                EvidenceActor::new(ActorKind::Observer, "runtime-operator"),
                None,
                EvidencePayload::Unknown(UnknownEvidence {
                    topic: "operator_observation".to_string(),
                    detail: json!({
                        "front": front.id.to_string(),
                        "listen": endpoint.display(),
                        "loopback": loopback,
                        "page_served": page_served,
                        "observation_route": observation_route,
                    })
                    .to_string(),
                }),
            );
            if page_served {
                println!("spc-lab-runtime: operator front-{}={url}", front.id);
            } else if loopback {
                eprintln!(
                    "spc-lab-runtime: operator front-{} page not served at {url}; \
                     no URL published",
                    front.id
                );
            } else {
                eprintln!(
                    "spc-lab-runtime: operator front-{} listener {} is not loopback; \
                     no URL published",
                    front.id,
                    endpoint.display()
                );
            }
        }
    }

    /// Drive a heal recovery only through durable miner controls. The worker
    /// pause after an observed common tip prevents a later tied fork from being
    /// mistaken for a stable final converged view.
    fn convergence_workers(&self, resume: bool) -> Result<Vec<Value>> {
        let mut results = Vec::new();
        for (miner, _, _, workers) in self.engine.runtime_miner_specs() {
            if workers.is_empty() {
                continue;
            }
            let kind = if resume {
                CommandKind::ResumeWorkers {
                    miner_id: miner.clone(),
                    workers: workers.iter().map(|worker| worker.id.clone()).collect(),
                }
            } else {
                CommandKind::PauseWorkers {
                    miner_id: miner.clone(),
                    workers: workers.iter().map(|worker| worker.id.clone()).collect(),
                }
            };
            let outcome = self.engine.submit(LabCommand::new(
                self.engine.run_id().clone(),
                ActorId::trusted("runtime-convergence"),
                kind,
            ));
            if outcome.state != crate::command::OutcomeState::Succeeded {
                bail!("convergence worker control failed for {miner}: {outcome:?}");
            }
            results.push(json!({"miner": miner, "resume": resume, "outcome": outcome}));
        }
        Ok(results)
    }

    /// Execute real scenario phases through the same durable command journal
    /// used by browser/headless control. Every link step captures node views,
    /// front APIs, host controller health and (when present) broker evidence.
    fn execute_phases(&self, controller_url: Option<&str>) -> Result<Value> {
        let phases = self.engine.scenario.phases.clone();
        let mut reports = Vec::new();
        let mut convergence_phase: Option<(String, crate::scenario::ConvergenceSpec)> = None;
        let mut pre_partition_common_anchor: Option<SharedChainAnchor> = None;
        let mut partition_started = false;
        for phase in phases {
            let phase_id = phase.id.clone();
            if let Some(spec) = phase.convergence.clone() {
                if convergence_phase
                    .replace((phase_id.clone(), spec))
                    .is_some()
                {
                    bail!("more than one phase declares a convergence action");
                }
            }
            for raw in phase.commands {
                let mut value = raw;
                let object = value
                    .as_object_mut()
                    .context("phase command must be an object")?;
                object
                    .entry("command_id")
                    .or_insert_with(|| json!(crate::ids::CommandId::generate().to_string()));
                object
                    .entry("run_id")
                    .or_insert_with(|| json!(self.engine.run_id().to_string()));
                object
                    .entry("issued_at_wall")
                    .or_insert_with(|| json!(crate::command::now_ms()));
                object
                    .entry("actor")
                    .or_insert_with(|| json!(ActorId::trusted("runtime-phase").to_string()));
                let command: LabCommand = serde_json::from_value(value)
                    .context("phase command does not match the durable command contract")?;
                let link_id = match &command.kind {
                    CommandKind::PartitionLink { link_id } | CommandKind::HealLink { link_id } => {
                        Some(link_id.to_string())
                    }
                    _ => None,
                };
                let partition_step = matches!(command.kind, CommandKind::PartitionLink { .. });
                // A declared bootstrap is durably asynchronous: the single
                // submit below has already bound its controller-principal
                // Begin, sole hash budget and immutable deadline. `Running`
                // is that in-flight state, not a phase failure, so the only
                // honest observation is to wait on the SAME operation id to
                // its durable terminal - never a resubmit, a new id, or a
                // synthesized outcome. The wait holds no engine mutex, so
                // operator stop/cancel stays responsive while it blocks.
                let bootstrap = matches!(command.kind, CommandKind::BootstrapFund { .. });
                let operation_id = command.command_id.clone();
                let mut outcome = self.engine.submit(command);
                if bootstrap && outcome.state == crate::command::OutcomeState::Running {
                    outcome = self.engine.wait_traffic_bootstrap(&operation_id)?;
                }
                let controller = match controller_url {
                    Some(base) => {
                        let url = format!("{}/api/lab/v1/health", base.trim_end_matches('/'));
                        match crate::namespace::http_request(&url, "GET", None, None, None) {
                            Ok(_) => ControllerObservation {
                                reachable: Some(true),
                                url: Some(url),
                                error: None,
                            },
                            Err(error) => ControllerObservation {
                                reachable: Some(false),
                                url: Some(url),
                                error: Some(error),
                            },
                        }
                    }
                    None => ControllerObservation {
                        reachable: None,
                        url: None,
                        error: Some("controller HTTP is not configured".to_string()),
                    },
                };
                let broker_evidence = match (&link_id, self.topology.as_ref()) {
                    (Some(link), Some(topology)) => Some(BrokerEvidenceObservation {
                        counters: Some(match topology.broker.counters(link) {
                            Ok(value) => BrokerCallObservation::success(value),
                            Err(error) => BrokerCallObservation::failure(error.to_string()),
                        }),
                        // The full no-bypass probe is expensive and is required
                        // while the fault is active, not again after heal.
                        no_bypass: partition_step.then(|| {
                            match topology.broker.no_bypass_check() {
                                Ok(value) => BrokerCallObservation::success(value),
                                Err(error) => BrokerCallObservation::failure(error.to_string()),
                            }
                        }),
                        consensus_counters: Vec::new(),
                    }),
                    (None, Some(topology)) if !topology.plan.consensus_links.is_empty() => {
                        let counters = topology
                            .plan
                            .consensus_links
                            .iter()
                            .map(|link| ConsensusCounterObservation {
                                link: link.clone(),
                                counters: match topology.broker.counters(link) {
                                    Ok(value) => BrokerCallObservation::success(value),
                                    Err(error) => BrokerCallObservation::failure(error.to_string()),
                                },
                            })
                            .collect();
                        Some(BrokerEvidenceObservation {
                            counters: None,
                            no_bypass: None,
                            consensus_counters: counters,
                        })
                    }
                    _ => None,
                };
                if !partition_started {
                    pre_partition_common_anchor = shared_tip(&self.node_views())
                        .map(|(height, hash)| SharedChainAnchor { height, hash });
                }
                let detail = TopologyPhaseEvidence {
                    schema_version: TOPOLOGY_EVIDENCE_SCHEMA_VERSION,
                    phase: phase_id.clone(),
                    outcome: outcome.clone(),
                    link: link_id.clone(),
                    pre_partition_common_anchor: pre_partition_common_anchor.clone(),
                    controller,
                    nodes: self.node_views(),
                    fronts: self.front_views(),
                    broker_evidence,
                };
                self.engine.evidence().append(
                    SourceId::trusted("topology"),
                    format!("topology-phase-{}", self.engine.evidence().last_event_id()),
                    EvidenceActor::new(ActorKind::Observer, "runtime-topology"),
                    None,
                    EvidencePayload::TopologyPhase(detail.clone()),
                );
                let detail_value = serde_json::to_value(&detail)?;
                if outcome.state != crate::command::OutcomeState::Succeeded {
                    self.write_phase_report(&detail_value)?;
                    bail!("phase command failed: {detail_value}");
                }
                if partition_step {
                    partition_started = true;
                }
                reports.push(detail_value);
            }
        }

        let convergence = if let Some((phase_id, spec)) = convergence_phase {
            let mut driver = RuntimeConvergenceDriver { runtime: self };
            let mut evidence = drive_convergence(&phase_id, &spec, &mut driver)
                .map_err(|error| anyhow::anyhow!(error))?;
            evidence.pre_partition_common_anchor = pre_partition_common_anchor.clone();
            self.engine.evidence().append(
                SourceId::trusted("topology"),
                format!(
                    "topology-convergence-{}",
                    self.engine.evidence().last_event_id()
                ),
                EvidenceActor::new(ActorKind::Observer, "runtime-topology"),
                None,
                EvidencePayload::TopologyConvergence(evidence.clone()),
            );
            if evidence.status == "not_converged" {
                let detail = serde_json::to_value(&evidence)?;
                self.write_phase_report(&detail)?;
                bail!("explicit convergence failed: {detail}");
            }
            serde_json::to_value(&evidence)?
        } else {
            // Existing host/scenarios have no post-phase convergence action:
            // they never receive an implicit resume or hash operation.
            json!({"status": "not_requested"})
        };
        let report = json!({"phases": reports, "convergence": convergence});
        self.write_phase_report(&report)?;
        Ok(report)
    }

    fn write_phase_report(&self, report: &Value) -> Result<()> {
        let root = self.run_dir.join("shareable");
        std::fs::create_dir_all(&root)?;
        std::fs::write(
            root.join("topology.json"),
            serde_json::to_vec_pretty(report)?,
        )?;
        Ok(())
    }

    fn node_views(&self) -> Vec<AtomicTipObservation> {
        self.nodes
            .iter()
            .map(|node| {
                let control = node.control(self.topology.as_ref());
                match atomic_tip(&control) {
                    Ok((height, hash, parent_hash)) => {
                        let peers = control
                            .call("getpeerinfo", json!([]))
                            .ok()
                            .and_then(|value| value.as_array().map(Vec::len));
                        AtomicTipObservation {
                            node: node.id.to_string(),
                            height: Some(height),
                            hash: Some(hash),
                            parent_hash,
                            atomic: true,
                            peers,
                            error: None,
                        }
                    }
                    Err(error) => AtomicTipObservation {
                        node: node.id.to_string(),
                        height: None,
                        hash: None,
                        parent_hash: None,
                        atomic: false,
                        peers: None,
                        error: Some(error),
                    },
                }
            })
            .collect()
    }

    fn front_views(&self) -> Vec<FrontApiObservation> {
        let listeners = self.engine.resources_manifest().listeners;
        self.pools
            .iter()
            .map(|pool| {
                let Some(api) = listeners.get(&format!("front.{}.api", pool.front)) else {
                    return FrontApiObservation {
                        front: pool.front.clone(),
                        reachable: false,
                        response_bytes: None,
                        error: Some("no API listener".to_string()),
                    };
                };
                let host = self
                    .topology
                    .as_ref()
                    .and_then(|topology| {
                        topology
                            .addresses
                            .get(&format!("front.{}.api_addr", pool.front))
                            .cloned()
                    })
                    .unwrap_or_else(|| api.host.clone());
                let url = format!("http://{host}:{}/api/pool", api.port);
                let result = match (
                    self.topology.as_ref(),
                    self.topology.as_ref().and_then(|topology| {
                        topology.plan.endpoint_of(&format!("front/{}", pool.front))
                    }),
                ) {
                    (Some(topology), Some(endpoint)) => topology
                        .broker
                        .exec_http(endpoint, &url, "GET", None, None, None)
                        .map_err(|error| error.to_string()),
                    _ => crate::namespace::http_request(&url, "GET", None, None, None),
                };
                match result {
                    Ok(body) => FrontApiObservation {
                        front: pool.front.clone(),
                        reachable: true,
                        response_bytes: Some(body.len()),
                        error: None,
                    },
                    Err(error) => FrontApiObservation {
                        front: pool.front.clone(),
                        reachable: false,
                        response_bytes: None,
                        error: Some(error),
                    },
                }
            })
            .collect()
    }

    /// The actual control-plane split: the controller remains host-local while
    /// isolated actors have namespace-scoped node/front/storage controls.
    pub fn network_handoff(&self) -> Value {
        let topology = self.topology.is_some();
        json!({
            "backend": if topology { "netns" } else { "host" },
            "namespace_placement": if topology { "broker-owned user+network namespaces per actor" } else { "host" },
            "address_publication": if topology { "broker manifest before config rendering; recorded in run manifest" } else { "host resource plan" },
            "controller_transport": "host controller lifetime/lock/listener unchanged; node/front RPC through broker exec; miner controls on owned stdio pipes",
            "postgres": "host user and host network namespace; private Unix socket with planned port and credentials",
            "redis": if topology { "per-pool storage namespace on published address; authenticated URL/database" } else { "host" },
            "owned_paths": {
                "run_dir": self.run_dir,
                "private": self.run_dir.join("private"),
                "logs": self.run_dir.join("logs"),
            },
        })
    }

    fn emit_oracle_summary(&self) -> Value {
        let events = self.engine.evidence().events();
        let summary = accounted_target_summary(&events);
        self.engine.evidence().append(
            SourceId::trusted("oracle"),
            format!(
                "oracle-accounted-{}",
                self.engine.evidence().last_event_id()
            ),
            EvidenceActor::new(ActorKind::Observer, "runtime-accounted-target"),
            None,
            EvidencePayload::Unknown(UnknownEvidence {
                topic: "accounted_target_consistency".to_string(),
                detail: summary.to_string(),
            }),
        );
        summary
    }

    fn write_oracle_result(&self, summary: &Value, topology_report: &Value) {
        let path = self.run_dir.join("shareable").join("result.json");
        let Ok(text) = std::fs::read_to_string(&path) else {
            return;
        };
        let Ok(mut result) = serde_json::from_str::<Value>(&text) else {
            return;
        };
        if let Some(object) = result.as_object_mut() {
            object.insert("accounted_target_consistency".to_string(), summary.clone());
            object.insert("topology".to_string(), topology_report.clone());
        }
        if let Ok(body) = serde_json::to_vec_pretty(&result) {
            let _ = std::fs::write(&path, body);
        }
    }
}

/// Bounded operator page probe: one attempt every 500 ms, at most 20
/// attempts. `http_request` only returns `Ok` for a 2xx status, so `true`
/// means the page route answered HTTP 200 - the page is served. This is a
/// route-readiness proof, not a funds-health claim.
const OPERATOR_READY_ATTEMPTS: u32 = 20;
const OPERATOR_READY_RETRY: Duration = Duration::from_millis(500);

fn operator_page_served(url: &str) -> bool {
    for attempt in 0..OPERATOR_READY_ATTEMPTS {
        if attempt > 0 {
            thread::sleep(OPERATOR_READY_RETRY);
        }
        if crate::namespace::http_request(url, "GET", None, None, None).is_ok() {
            return true;
        }
    }
    false
}

/// Informational probe of the operator page's observation JSON route.
/// `None` means the route is not reachable; `Some(true)` means it served
/// the expected `spc-pool-observation/v1` schema; `Some(false)` means it
/// answered something else. Never gates URL publishing.
fn operator_observation_route_present(url: &str) -> Option<bool> {
    let body = crate::namespace::http_request(
        &format!("{url}api/pool/observation"),
        "GET",
        None,
        None,
        None,
    )
    .ok()?;
    let parsed = serde_json::from_str::<Value>(&body).ok();
    Some(
        parsed
            .as_ref()
            .and_then(|value| value.get("schema"))
            .and_then(Value::as_str)
            .is_some_and(|schema| schema == "spc-pool-observation/v1"),
    )
}

fn accounted_target_summary(events: &[Arc<EvidenceEvent>]) -> Value {
    #[derive(Default)]
    struct JobFacts {
        targets: BTreeSet<String>,
        credits: BTreeSet<u64>,
    }
    let mut jobs: BTreeMap<(String, String, u64), JobFacts> = BTreeMap::new();
    for event in events {
        if let EvidencePayload::JobIssued(job) = &event.payload {
            jobs.entry((
                job.front_id.to_string(),
                job.job_id.to_string(),
                job.generation,
            ))
            .or_default()
            .targets
            .insert(job.share_target.to_ascii_lowercase());
            jobs.entry((
                job.front_id.to_string(),
                job.job_id.to_string(),
                job.generation,
            ))
            .or_default()
            .credits
            .insert(job.m_issue);
        }
    }
    let mut accounted = 0u64;
    let mut checked = 0u64;
    let mut consistent = 0u64;
    let mut inconsistent = 0u64;
    let mut unknown = 0u64;
    let mut duplicate_conflicting_jobs = 0u64;
    let mut missing_jobs = 0u64;
    let mut target_mismatches = 0u64;
    let mut credit_mismatches = 0u64;
    let mut hash_above_target = 0u64;
    let mut malformed_hashes = 0u64;
    for event in events {
        let EvidencePayload::SpcEvidence(record) = &event.payload else {
            continue;
        };
        if record.event != "accounted" {
            continue;
        }
        accounted += 1;
        let (Some(front), Some(job_id), Some(generation)) = (
            record.front_id.as_ref(),
            record.job_id.as_ref(),
            record.generation,
        ) else {
            unknown += 1;
            continue;
        };
        let key = (front.to_string(), job_id.to_string(), generation);
        let Some(facts) = jobs.get(&key) else {
            missing_jobs += 1;
            unknown += 1;
            continue;
        };
        if facts.targets.len() != 1 || facts.credits.len() != 1 {
            duplicate_conflicting_jobs += 1;
            unknown += 1;
            continue;
        }
        let announced_target = facts.targets.iter().next().cloned().unwrap_or_default();
        let announced_credit = facts.credits.iter().next().copied().unwrap_or_default();
        let Some(recorded_target) = record
            .raw
            .get("share_target")
            .and_then(Value::as_str)
            .map(|value| value.to_ascii_lowercase())
        else {
            unknown += 1;
            continue;
        };
        let Some(recorded_credit) = record.raw.get("m_issue").and_then(value_u64) else {
            unknown += 1;
            continue;
        };
        let Some(booked_d) = record.raw.get("d").and_then(value_u64) else {
            unknown += 1;
            continue;
        };
        checked += 1;
        let mut record_consistent = true;
        if recorded_target != announced_target {
            target_mismatches += 1;
            record_consistent = false;
        }
        if recorded_credit != announced_credit {
            credit_mismatches += 1;
            record_consistent = false;
        }
        if booked_d != announced_credit {
            credit_mismatches += 1;
            record_consistent = false;
        }
        if let (Some(target), Some(hash_text)) = (
            num_bigint::BigUint::parse_bytes(announced_target.as_bytes(), 16),
            record.share_hash.as_deref(),
        ) {
            // LP-topology merge fix F1: the hash comparison fails closed.
            //
            // A share hash that is not hex, or that is not exactly the 32
            // raw SHA256d bytes, is malformed evidence.  It used to be skipped
            // silently, which let such a record count as `consistent`.  A
            // malformed hash is now counted as malformed and the record is
            // `inconsistent`; it can never be `consistent`.
            match hex::decode(hash_text) {
                Ok(hash) if hash.len() == 32 => {
                    // Raw SHA256d digest bytes, compared as a little-endian
                    // integer against the announced big-endian target.
                    let value = num_bigint::BigUint::from_bytes_le(&hash);
                    if value > target {
                        hash_above_target += 1;
                        record_consistent = false;
                    }
                }
                _ => {
                    malformed_hashes += 1;
                    inconsistent += 1;
                    continue;
                }
            }
        } else {
            // No announced target, or no recorded hash: the record cannot be
            // checked, so it stays unknown.
            unknown += 1;
            continue;
        }
        if record_consistent {
            consistent += 1;
        } else {
            inconsistent += 1;
        }
    }
    let status = if accounted == 0 {
        "unknown"
    } else if inconsistent > 0 {
        "inconsistent"
    } else if unknown > 0 {
        "unknown"
    } else {
        "consistent"
    };
    json!({
        "status": status,
        "accounted_records": accounted,
        "records_checked_against_job": checked,
        "consistent_records": consistent,
        "inconsistent_records": inconsistent,
        "unknown_records": unknown,
        "missing_job_records": missing_jobs,
        "duplicate_conflicting_job_keys": duplicate_conflicting_jobs,
        "target_mismatches": target_mismatches,
        "credit_mismatches": credit_mismatches,
        "hashes_above_announced_target": hash_above_target,
        "malformed_share_hashes": malformed_hashes,
        "scoped_job_keys": jobs.len(),
        "scope": {
            "checked": [
                "accounted hash <= its recorded/announced target",
                "accounted target, m_issue and booked d equal the announced job for the same front + job_id + generation",
                "the recorded share hash is exactly 32 raw bytes of hex; a malformed hash is inconsistent, never consistent"
            ],
            "not_checked": [
                "80-byte header recomputation",
                "independent family S(2^j) target derivation",
                "slice/database credit sums",
                "payout or settlement",
                "canonical block inclusion"
            ],
            "full_header_payout_oracle": "inconclusive"
        },
        "source": "pool accounted records and typed job_issued records; no miner ACK is used"
    })
}

fn value_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|text| text.parse::<u64>().ok()))
}

fn observer_active(engine: &RunEngine) -> bool {
    matches!(
        engine.manifest_snapshot().state,
        crate::manifest::RunState::Running | crate::manifest::RunState::Preparing
    )
}

fn terminal(engine: &RunEngine) -> bool {
    matches!(
        engine.manifest_snapshot().state,
        crate::manifest::RunState::Stopped
            | crate::manifest::RunState::CompletedWithErrors
            | crate::manifest::RunState::Failed
    )
}

fn load_or_generate_identities(
    scenario: &Scenario,
    private: &Path,
) -> Result<Vec<PrivateIdentity>> {
    let secp = Secp256k1::new();
    let mut rng = rand::thread_rng();
    let mut identities = Vec::new();
    for spec in &scenario.identities {
        if scenario
            .wallet_agents
            .iter()
            .any(|a| a.identity_id == spec.id)
        {
            identities.push(crate::wallet_bridge::public_identity(private, &spec.id)?);
            continue;
        }
        let path = private.join(format!("identity-{}.json", spec.id));
        if path.exists() {
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("read identity {}", path.display()))?;
            let identity: PrivateIdentity = serde_json::from_str(&text)
                .with_context(|| format!("identity artifact is not valid: {}", path.display()))?;
            if identity.id != spec.id.to_string() {
                bail!(
                    "identity artifact {} declares id {:?}, expected {:?}",
                    path.display(),
                    identity.id,
                    spec.id.to_string()
                );
            }
            // Never trust stored public fields on reload: re-derive them from
            // the private keys and reject parseable tampering.
            let scan_bytes = hex::decode(&identity.scan_key)
                .with_context(|| format!("scan key in {}", path.display()))?;
            let spend_bytes = hex::decode(&identity.spend_key)
                .with_context(|| format!("spend key in {}", path.display()))?;
            let scan = SecretKey::from_slice(&scan_bytes)
                .with_context(|| format!("scan key in {}", path.display()))?;
            let spend = SecretKey::from_slice(&spend_bytes)
                .with_context(|| format!("spend key in {}", path.display()))?;
            let b_scan = PublicKey::from_secret_key(&secp, &scan);
            let b_spend = PublicKey::from_secret_key(&secp, &spend);
            let address = spc_crypto::address::encode_sp_address(
                spc_crypto::Network::Regtest,
                &b_scan,
                &b_spend,
            )
            .map_err(|error| anyhow::anyhow!("identity address: {error}"))?;
            let expected_sp_id = hex::encode(spc_crypto::sp_id(&b_scan, &b_spend));
            if identity.b_scan != hex::encode(b_scan.serialize())
                || identity.b_spend != hex::encode(b_spend.serialize())
                || identity.address != address
                || identity.sp_id != expected_sp_id
            {
                bail!(
                    "identity artifact {} does not match its private keys; refusing tampered material",
                    path.display()
                );
            }
            identities.push(identity);
            continue;
        }
        let scan = SecretKey::new(&mut rng);
        let spend = SecretKey::new(&mut rng);
        let b_scan = PublicKey::from_secret_key(&secp, &scan);
        let b_spend = PublicKey::from_secret_key(&secp, &spend);
        let address =
            spc_crypto::address::encode_sp_address(spc_crypto::Network::Regtest, &b_scan, &b_spend)
                .map_err(|error| anyhow::anyhow!("identity address: {error}"))?;
        let identity = PrivateIdentity {
            id: spec.id.to_string(),
            address,
            sp_id: hex::encode(spc_crypto::sp_id(&b_scan, &b_spend)),
            b_scan: hex::encode(b_scan.serialize()),
            b_spend: hex::encode(b_spend.serialize()),
            scan_key: hex::encode(scan.secret_bytes()),
            spend_key: hex::encode(spend.secret_bytes()),
        };
        write_private_json(&path, &identity)?;
        identities.push(identity);
    }
    Ok(identities)
}

fn apply_identity_addresses(scenario: &mut Scenario, identities: &[PrivateIdentity]) {
    for spec in &mut scenario.identities {
        if let Some(identity) = identities
            .iter()
            .find(|item| item.id == spec.id.to_string())
        {
            spec.address_source = AddressSource::Explicit {
                address: identity.address.clone(),
            };
        }
    }
}

fn load_or_generate_secrets(scenario: &mut Scenario, private: &Path) -> Result<RuntimeSecrets> {
    let path = private.join("secrets.json");
    let mut secrets = if path.exists() {
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("read secrets {}", path.display()))?;
        serde_json::from_str(&text)
            .with_context(|| format!("secrets artifact is not valid: {}", path.display()))?
    } else {
        RuntimeSecrets {
            node_rpc_user: "spc".to_string(),
            node_rpc_password: random_hex(24),
            redis: BTreeMap::new(),
            postgres: BTreeMap::new(),
        }
    };
    let mut changed = !path.exists();
    for redis in &mut scenario.resources.redis_instances {
        if redis.password.is_none() {
            let password = secrets
                .redis
                .entry(redis.id.clone())
                .or_insert_with(|| random_hex(24))
                .clone();
            redis.password = Some(password);
            changed = true;
        }
    }
    for pg in &mut scenario.resources.postgres_databases {
        if pg.password.is_none() {
            let password = secrets
                .postgres
                .entry(pg.id.clone())
                .or_insert_with(|| random_hex(24))
                .clone();
            pg.password = Some(password);
            changed = true;
        }
    }
    if changed {
        write_private_json(&path, &secrets)?;
    }
    Ok(secrets)
}

/// The namespaces the actors are placed in, keyed by `role/entity`.
fn topology_namespaces(
    scenario: &Scenario,
    plan: &TopologyPlan,
    placement: &Placement,
) -> BTreeMap<String, String> {
    let mut namespaces = BTreeMap::new();
    for (actor, endpoint) in &plan.actor_endpoints {
        if let Some(netns) = placement.namespace(endpoint) {
            namespaces.insert(actor.clone(), netns.to_string());
        }
    }
    // The storage boundary (T8): the Redis of a pool lives in the storage
    // namespace of that pool; PostgreSQL stays in the host namespace because
    // it refuses to run as uid 0 inside a user namespace.
    for redis in &scenario.resources.redis_instances {
        let storage = format!("storage-{}", redis.owner_pool_id);
        if let Some(netns) = placement.namespace(&storage) {
            namespaces.insert(format!("redis/{}", redis.id), netns.to_string());
        }
    }
    namespaces
}

/// Start-time identities for every actor's network-namespace holder.
fn topology_namespace_owners(
    plan: &TopologyPlan,
    namespaces: &BTreeMap<String, String>,
    manifest: &crate::namespace::BrokerManifest,
) -> Result<BTreeMap<String, crate::supervisor::NamespaceOwner>> {
    let mut owners = BTreeMap::new();
    for actor in namespaces.keys() {
        let endpoint = plan
            .actor_endpoints
            .get(actor)
            .with_context(|| format!("no topology endpoint is planned for actor {actor}"))?;
        let published = manifest
            .endpoint(endpoint)
            .with_context(|| format!("the broker did not publish endpoint {endpoint}"))?;
        let start_time_ticks = published.namespace.start_time_token.clone();
        if start_time_ticks.is_empty() {
            bail!("namespace holder {endpoint} has no start-time identity");
        }
        owners.insert(
            actor.clone(),
            crate::supervisor::NamespaceOwner {
                pid: published.namespace.holder_pid,
                start_time_ticks,
            },
        );
    }
    Ok(owners)
}

/// The published addresses of the actors, keyed by `<kind>.<entity>.<name>`.
///
/// The map is what the engine and the config rendering use to name an
/// endpoint.  Without a topology it is empty and everything stays on the
/// loopback address of the host.
fn topology_addresses(
    scenario: &Scenario,
    plan: &TopologyPlan,
    placement: &Placement,
) -> BTreeMap<String, String> {
    let mut addresses = BTreeMap::new();
    for node in &scenario.nodes {
        let id = node.id.to_string();
        // The RPC listener has to be reachable from the pool that follows this
        // node, so it uses the address of the pool to node link.
        let rpc_link = scenario
            .pools
            .iter()
            .find(|pool| pool.node_ids.contains(&node.id))
            .and_then(|pool| {
                scenario
                    .fronts
                    .iter()
                    .find(|front| front.pool_id == pool.id)
                    .and_then(|front| link_between(plan, front.id.as_str(), &id))
            });
        if let Some(link) = rpc_link {
            if let Some(address) = placement.address(&id, &link) {
                addresses.insert(format!("node.{id}.rpc_addr"), address.addr.clone());
            }
        }
        // The P2P listener uses the address of the consensus link.
        // Every consensus link gives the node its P2P address and tells the
        // other end to connect to it.
        for link in &plan.consensus_links {
            let Some(item) = plan.links.iter().find(|item| &item.id == link) else {
                continue;
            };
            if item.a != id && item.b != id {
                continue;
            }
            if let Some(address) = placement.address(&id, link) {
                addresses.insert(format!("node.{id}.p2p_addr"), address.addr.clone());
            }
            let other = if item.a == id { &item.b } else { &item.a };
            if let Some(address) = placement.address(other, link) {
                addresses.insert(format!("node.{id}.p2p_connect_addr"), address.addr.clone());
            }
        }
    }
    for front in &scenario.fronts {
        let id = front.id.to_string();
        // The SV2 listener has to be reachable from the miner that follows this
        // front, so it uses the address of the miner to front link.
        let miner_link = scenario
            .miners
            .iter()
            .find(|miner| {
                miner
                    .connections
                    .iter()
                    .any(|connection| connection.front_id == front.id)
            })
            .and_then(|miner| link_between(plan, miner.id.as_str(), &id));
        if let Some(link) = miner_link {
            if let Some(address) = placement.address(&id, &link) {
                addresses.insert(format!("front.{id}.sv2_addr"), address.addr.clone());
                addresses.insert(format!("front.{id}.jd_addr"), address.addr.clone());
                addresses.insert(format!("front.{id}.api_addr"), address.addr.clone());
            }
        }
        // The pool reaches its node over the pool to node link.
        if let Some(pool) = scenario.pools.iter().find(|pool| pool.id == front.pool_id) {
            if let Some(node_id) = pool.node_ids.first() {
                if let Some(link) = link_between(plan, &id, node_id.as_str()) {
                    if let Some(address) = placement.address(node_id.as_str(), &link) {
                        addresses.insert(format!("front.{id}.node_rpc_addr"), address.addr.clone());
                    }
                }
            }
        }
        // The pool reaches its private storage over the front to storage link.
        let storage = format!("storage-{}", front.pool_id);
        if let Some(link) = link_between(plan, &id, &storage) {
            if let Some(address) = placement.address(&storage, &link) {
                addresses.insert(format!("front.{id}.redis_addr"), address.addr.clone());
                for redis in &scenario.resources.redis_instances {
                    if redis.owner_pool_id == front.pool_id {
                        addresses.insert(format!("redis.{}.addr", redis.id), address.addr.clone());
                    }
                }
            }
        }
    }
    addresses
}

/// The link that joins two endpoints, if the plan has one.
fn link_between(plan: &TopologyPlan, a: &str, b: &str) -> Option<String> {
    plan.links
        .iter()
        .find(|link| (link.a == a && link.b == b) || (link.a == b && link.b == a))
        .map(|link| link.id.clone())
}

/// Start the namespace broker and publish the placement.
fn start_topology(
    scenario: &Scenario,
    options: &RuntimeOptions,
    run_dir: &Path,
) -> Result<Option<Arc<TopologyRuntime>>> {
    let Some(topology_options) = options.topology.clone() else {
        return Ok(None);
    };
    let plan = TopologyPlan::from_scenario(scenario)
        .map_err(|error| anyhow::anyhow!("the topology plan is invalid: {error}"))?;
    let broker_run_dir = run_dir.join("broker");
    std::fs::create_dir_all(&broker_run_dir)
        .with_context(|| format!("create {}", broker_run_dir.display()))?;
    std::fs::set_permissions(&broker_run_dir, std::fs::Permissions::from_mode(0o700))?;
    // The broker requires its own run directory to be empty at startup.
    let spec_path = run_dir.join("broker-topology.json");
    std::fs::write(&spec_path, serde_json::to_vec_pretty(&plan.broker_spec())?)
        .with_context(|| format!("write {}", spec_path.display()))?;
    let mut broker_options = BrokerOptions::new(
        topology_options.broker_script.clone(),
        spec_path.clone(),
        broker_run_dir.clone(),
        runtime_run_token(),
    );
    broker_options.unshare = topology_options.unshare.clone();
    broker_options.nsenter = topology_options.nsenter.clone();
    broker_options.lifetime_helper =
        Some(std::env::current_exe().context("the controller executable path is unknown")?);
    let broker = Arc::new(
        NamespaceBroker::start(broker_options)
            .map_err(|error| anyhow::anyhow!("the namespace broker did not start: {error}"))?,
    );
    let placement = Placement::from_manifest(broker.manifest());
    let namespaces = topology_namespaces(scenario, &plan, &placement);
    let namespace_owners = topology_namespace_owners(&plan, &namespaces, broker.manifest())?;
    if namespace_owners.len() != namespaces.len() {
        bail!("not every namespaced actor has a verified namespace owner");
    }
    let broker_pid = broker
        .pid()
        .context("the namespace broker owner PID is unknown")?;
    let user_namespace_owner = crate::supervisor::NamespaceOwner {
        pid: broker_pid,
        start_time_ticks: crate::supervisor::process_start_time(broker_pid)
            .context("the namespace broker start-time token is unavailable")?
            .to_string(),
    };
    let user_namespace = PathBuf::from(format!("/proc/{broker_pid}/ns/user"));
    // The storage boundary (T8).
    //
    // The Redis of a pool lives in the storage namespace of that pool and is
    // reached over the published storage link, so two pools cannot reach each
    // other's Redis at all.  PostgreSQL refuses to run as uid 0 inside a user
    // namespace, so its instance stays in the host namespace and the front
    // reaches it over the private Unix socket of the run, which the shared
    // filesystem makes reachable from every namespace.  A link fault on the
    // consensus link therefore does not touch the storage, and the packet does
    // not claim that it does.
    if namespaces.is_empty() {
        bail!("the namespace broker published no namespace for the actors");
    }
    let addresses = topology_addresses(scenario, &plan, &placement);
    Ok(Some(Arc::new(TopologyRuntime {
        socket_path: broker.socket_path().to_path_buf(),
        broker,
        plan,
        placement,
        namespaces,
        namespace_owners,
        user_namespace,
        user_namespace_owner,
        addresses,
    })))
}

/// The run token of the broker: short hex, because the broker derives the
/// interface and nft table names from it.
fn runtime_run_token() -> String {
    let mut bytes = [0u8; 4];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn build_lab_config(
    scenario: &Scenario,
    options: &RuntimeOptions,
    secrets: &RuntimeSecrets,
) -> Result<LabConfig> {
    let mut config = LabConfig {
        link_backend: Some("fixture".to_string()),
        ..LabConfig::default()
    };
    if scenario.traffic.is_some() {
        let adapter = |program: &PathBuf, args: Vec<String>| AdapterConfig {
            program: program.clone(),
            args,
            capabilities: Vec::new(),
            fixture: false,
            effective_profile: None,
            rpc_auth: None,
            env: BTreeMap::new(),
        };
        config.tx_driver = options
            .binaries
            .txd
            .as_ref()
            .map(|program| adapter(program, Vec::new()));
        config.traffic_probe = options
            .binaries
            .traffic_probe
            .as_ref()
            .map(|program| adapter(program, options.binaries.traffic_probe_args.clone()));
    }
    let required_capability = scenario
        .network
        .consensus_profile
        .required_capability
        .clone();
    if !NODE_CAPABILITIES.contains(&required_capability.as_str()) {
        bail!(
            "scenario requires unsupported node capability {:?}; known real capabilities are {:?}",
            required_capability,
            NODE_CAPABILITIES
        );
    }
    let mut node_args = vec![
        "--network".to_string(),
        "regtest".to_string(),
        "--datadir".to_string(),
        "{run_dir}/private/node-{entity}".to_string(),
        "--rpc".to_string(),
        "--rpc-listen".to_string(),
        "{rpc_addr}:{rpc_port}".to_string(),
        "--rpc-cookie-file".to_string(),
        "{run_dir}/private/node-rpc-{entity}.cookie".to_string(),
        "--regtest-pow-bits".to_string(),
        options.pow_bits.clone(),
        "--no-listen".to_string(),
        "--no-seeds".to_string(),
        "--no-discover".to_string(),
        "--max-outbound".to_string(),
        "1".to_string(),
        "--api-log".to_string(),
        "{run_dir}/private/node-api-{entity}.jsonl".to_string(),
        "--log-level".to_string(),
        "info".to_string(),
    ];
    if scenario.ark_backends.iter().any(|ark| ark.local.is_some()) {
        node_args.extend(["--rpc-compat-profile".into(), "bark-v31".into()]);
        let tools = options.ark.as_ref().context("local Ark tools missing")?;
        config.ark.insert(
            "captaind".into(),
            AdapterConfig {
                program: tools.captaind.clone(),
                args: Vec::new(),
                capabilities: Vec::new(),
                fixture: false,
                effective_profile: None,
                rpc_auth: None,
                env: BTreeMap::new(),
            },
        );
    }
    config.nodes.insert(
        "rbitcoin-node".to_string(),
        AdapterConfig {
            program: options.binaries.node.clone(),
            args: node_args,
            capabilities: NODE_CAPABILITIES
                .iter()
                .map(|value| value.to_string())
                .collect(),
            fixture: false,
            effective_profile: None,
            rpc_auth: Some(format!(
                "basic:{}:{}",
                secrets.node_rpc_user, secrets.node_rpc_password
            )),
            env: BTreeMap::new(),
        },
    );
    if scenario.traffic.is_some() {
        config
            .nodes
            .get_mut("rbitcoin-node")
            .expect("inserted node adapter")
            .capabilities
            .push(crate::traffic::TRAFFIC_CAPABILITY.into());
    }
    config.pools.insert(
        "spc-pool".to_string(),
        AdapterConfig {
            program: options.binaries.pool.clone(),
            args: vec![
                "--config".to_string(),
                "{run_dir}/private/pool-{entity}.toml".to_string(),
            ],
            capabilities: vec![
                "spc.pool.work_unit_v2".to_string(),
                "spc.pool.network_block_work".to_string(),
            ],
            fixture: false,
            effective_profile: None,
            rpc_auth: None,
            env: BTreeMap::new(),
        },
    );
    config.miners.insert(
        "spc-cpu-miner".to_string(),
        AdapterConfig {
            program: options.binaries.miner.clone(),
            args: vec![
                "--config".to_string(),
                "{run_dir}/private/miner-{entity}.toml".to_string(),
            ],
            capabilities: vec!["spc.miner.sv2.cpu".to_string()],
            fixture: false,
            effective_profile: None,
            rpc_auth: None,
            env: BTreeMap::new(),
        },
    );
    config.redis = Some(AdapterConfig {
        program: options.binaries.redis_server.clone(),
        args: vec!["{run_dir}/private/redis-{entity}.conf".to_string()],
        capabilities: Vec::new(),
        fixture: false,
        effective_profile: None,
        rpc_auth: None,
        env: BTreeMap::new(),
    });
    // PostgreSQL always remains a host-user/host-namespace actor. Fronts in
    // topology runs connect through its planned private Unix socket.
    let postgres_args = vec![
        "-D".to_string(),
        "{data_dir}".to_string(),
        "-p".to_string(),
        "{port}".to_string(),
        "-h".to_string(),
        "127.0.0.1".to_string(),
        "-k".to_string(),
        "{data_dir}/socket".to_string(),
    ];
    config.postgres = Some(AdapterConfig {
        program: options.binaries.postgres_tool("postgres"),
        args: postgres_args,
        capabilities: Vec::new(),
        fixture: false,
        effective_profile: None,
        rpc_auth: None,
        env: BTreeMap::new(),
    });
    Ok(config)
}

fn render_node_files(
    engine: &RunEngine,
    run_dir: &Path,
    secrets: &RuntimeSecrets,
    _pow_bits: &str,
) -> Result<()> {
    for node in &engine.scenario().nodes {
        let datadir = run_dir.join("private").join(format!("node-{}", node.id));
        std::fs::create_dir_all(&datadir)?;
        std::fs::set_permissions(&datadir, std::fs::Permissions::from_mode(0o700))?;
        let cookie = run_dir
            .join("private")
            .join(format!("node-rpc-{}.cookie", node.id));
        let body = format!("{}:{}", secrets.node_rpc_user, secrets.node_rpc_password);
        write_private_bytes(&cookie, body.as_bytes())?;
    }
    Ok(())
}

fn render_redis_configs(
    run_dir: &Path,
    instances: &[RedisRuntime],
    secrets: &RuntimeSecrets,
    topology: Option<&Arc<TopologyRuntime>>,
) -> Result<()> {
    for redis in instances {
        std::fs::create_dir_all(&redis.dir)?;
        std::fs::set_permissions(&redis.dir, std::fs::Permissions::from_mode(0o700))?;
        let password = secrets
            .redis
            .get(&redis.id)
            .context("redis password is missing from the secret store")?;
        // A namespaced Redis binds the address the broker published for its
        // storage endpoint, so the pool reaches it over the storage link and
        // no other pool can reach it at all.
        let bind = topology
            .and_then(|topology| topology.addresses.get(&format!("redis.{}.addr", redis.id)))
            .cloned()
            .unwrap_or_else(|| "127.0.0.1".to_string());
        let body = format!(
            "bind {bind}\nport {}\nprotected-mode yes\ndaemonize no\nsave \"\"\nappendonly no\ndir {}\nrequirepass {}\nloglevel notice\n",
            redis.port,
            redis.dir.display(),
            password
        );
        let path = run_dir
            .join("private")
            .join(format!("redis-{}.conf", redis.id));
        write_private_bytes(&path, body.as_bytes())?;
    }
    Ok(())
}

fn initialise_postgres(
    run_dir: &Path,
    databases: &[PostgresRuntime],
    binaries: &RuntimeBinaries,
    secrets: &RuntimeSecrets,
) -> Result<()> {
    for pg in databases {
        let marker = pg.dir.join("PG_VERSION");
        if marker.exists() {
            let socket = pg.dir.join("socket");
            std::fs::create_dir_all(&socket)?;
            std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o700))?;
            continue;
        }
        std::fs::create_dir_all(&pg.dir)?;
        std::fs::set_permissions(&pg.dir, std::fs::Permissions::from_mode(0o700))?;
        let password = secrets
            .postgres
            .get(&pg.id)
            .context("postgres password is missing from the secret store")?;
        let password_file = run_dir
            .join("private")
            .join(format!("postgres-{}.password", pg.id));
        write_private_bytes(&password_file, format!("{password}\n").as_bytes())?;
        let output = Command::new(binaries.postgres_tool("initdb"))
            .arg("--pgdata")
            .arg(&pg.dir)
            .arg("--username")
            .arg(&pg.role)
            .arg("--pwfile")
            .arg(&password_file)
            .arg("--auth")
            .arg("scram-sha-256")
            .arg("--auth-host")
            .arg("scram-sha-256")
            .arg("--auth-local")
            .arg("trust")
            .arg("--encoding")
            .arg("UTF8")
            .arg("--locale")
            .arg("C")
            .arg("--no-instructions")
            .output()
            .context("run initdb")?;
        let _ = std::fs::remove_file(&password_file);
        if !output.status.success() {
            bail!(
                "initdb failed for {}: {}",
                pg.id,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        let socket = pg.dir.join("socket");
        std::fs::create_dir_all(&socket)?;
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn topology_postgres_url(pg: &PostgresRuntime, password: &str) -> String {
    // SQLx requires a nonempty URL authority. Its `host` query option replaces
    // the TCP host with the private Unix socket; port/credentials stay exact.
    format!(
        "postgres://{}:{}@localhost:{}/{}?host={}",
        pg.role,
        urlencoding(password),
        pg.port,
        pg.database,
        pg.dir.join("socket").display()
    )
}

fn topology_redis_url(redis: &RedisRuntime, address: &str, password: &str) -> String {
    format!(
        "redis://:{}@{}:{}/0",
        urlencoding(password),
        address,
        redis.port
    )
}

fn render_pool_configs(
    engine: &RunEngine,
    run_dir: &Path,
    redis: &[RedisRuntime],
    postgres: &[PostgresRuntime],
    secrets: &RuntimeSecrets,
    topology: Option<&Arc<TopologyRuntime>>,
    operator_ui: bool,
) -> Result<Vec<PoolRender>> {
    let mut rendered = Vec::new();
    let mut numeric = 0u8;
    for front in &engine.scenario().fronts {
        let pool = engine
            .scenario()
            .pools
            .iter()
            .find(|pool| pool.id == front.pool_id)
            .context("front names an unknown pool")?;
        numeric = numeric.saturating_add(1);
        let node = engine
            .scenario()
            .nodes
            .iter()
            .find(|node| pool.node_ids.contains(&node.id))
            .context("pool has no node")?;
        let listeners = engine.resources_manifest().listeners;
        let rpc = listeners
            .get(&format!("node.{}.rpc", node.id))
            .context("node RPC listener is missing")?;
        let sv2 = listeners
            .get(&format!("front.{}.sv2", front.id))
            .context("front SV2 listener is missing")?;
        let jd = listeners
            .get(&format!("front.{}.jd", front.id))
            .context("front JD listener is missing")?;
        let api = listeners
            .get(&format!("front.{}.api", front.id))
            .context("front API listener is missing")?;
        // The published addresses of the placement.  Without a topology every
        // endpoint stays on the loopback address of the host.
        let published = |key: &str| -> Option<String> {
            topology
                .and_then(|topology| topology.addresses.get(key))
                .cloned()
        };
        let sv2 = match published(&format!("front.{}.sv2_addr", front.id)) {
            Some(address) => format!("{address}:{}", sv2.port),
            None => sv2.display(),
        };
        let jd = match published(&format!("front.{}.jd_addr", front.id)) {
            Some(address) => format!("{address}:{}", jd.port),
            None => jd.display(),
        };
        let api = match published(&format!("front.{}.api_addr", front.id)) {
            Some(address) => format!("{address}:{}", api.port),
            None => api.display(),
        };
        let rpc = match published(&format!("front.{}.node_rpc_addr", front.id)) {
            Some(address) => format!("{address}:{}", rpc.port),
            None => rpc.display(),
        };
        let authority_path = run_dir
            .join("private")
            .join(format!("pool-authority-{}.json", pool.id));
        let authority: Value = serde_json::from_str(
            &std::fs::read_to_string(&authority_path)
                .with_context(|| format!("read {}", authority_path.display()))?,
        )?;
        let authority_public = authority
            .get("public_base58")
            .and_then(Value::as_str)
            .context("pool authority public key is missing")?;
        let authority_secret = authority
            .get("secret_base58")
            .and_then(Value::as_str)
            .context("pool authority secret key is missing")?;
        let redis_runtime = redis
            .iter()
            .find(|item| item.id == pool.storage.redis_instance_id)
            .context("pool Redis is missing")?;
        let pg_runtime = postgres
            .iter()
            .find(|item| item.id == pool.storage.postgres_database_id)
            .context("pool Postgres is missing")?;
        // The Redis of this pool is reached on its published address.  The
        // PostgreSQL instance stays in the host namespace and is reached over
        // its private Unix socket: PostgreSQL refuses to run as root inside a
        // user namespace.
        // The Postgres instance stays in the host namespace: it refuses to
        // run as uid 0 inside a user namespace.  The front reaches it over
        // the private Unix socket of the run, which every namespace shares
        // through the filesystem.
        let postgres_url = match topology {
            Some(_) => topology_postgres_url(
                pg_runtime,
                secrets
                    .postgres
                    .get(&pg_runtime.id)
                    .map(String::as_str)
                    .unwrap_or_default(),
            ),
            None => pg_runtime.url.clone(),
        };
        let redis_url = match published(&format!("front.{}.redis_addr", front.id)) {
            Some(address) => topology_redis_url(
                redis_runtime,
                &address,
                secrets
                    .redis
                    .get(&redis_runtime.id)
                    .context("pool Redis password is missing")?,
            ),
            None => redis_runtime.url.clone(),
        };
        let profile = &pool.work_profile;
        let reference = profile
            .reference_target
            .as_deref()
            .context("v2 work profile has no reference target")?;
        let multiplier = profile
            .share_work_multiplier
            .context("v2 work profile has no share work multiplier")?;
        let max_multiplier = profile.max_work_multiplier.unwrap_or(4_294_967_296);
        let max_window = profile
            .max_window_target_units
            .map(|value| value.to_string())
            .unwrap_or_else(|| "18446744073709551616".to_string());
        let window_rule = match profile.window_rule {
            WindowRule::NetworkBlockWork => "network_block_work",
            WindowRule::ShareWorkLegacy => "share_work_legacy",
        };
        let pow_bits = engine
            .scenario()
            .network
            .consensus_profile
            .requirements
            .pow_limit_bits
            .clone()
            .unwrap_or_else(|| "207fffff".to_string());
        let max_slice = profile.max_slice_shares.unwrap_or(65_000);
        let mut roles: Vec<&str> = front
            .roles
            .iter()
            .map(|role| match role {
                FrontRole::Front => "front",
                FrontRole::Payout => "payout",
                FrontRole::Api => "api",
                FrontRole::Ark => "ark",
            })
            .collect();
        roles.sort_unstable();
        roles.dedup();
        // EXP-OP runtime wiring: when opted in, the planned
        // `front.<id>.operator` listener - allocated through the same port
        // lease machinery as every other listener and therefore always a
        // host loopback endpoint - is rendered as the pool's `[operator]`
        // section. `Endpoint` cannot carry a malformed or non-loopback
        // address here, and the pool parser independently rejects one.
        let operator = if operator_ui {
            Some(
                listeners
                    .get(&format!("front.{}.operator", front.id))
                    .context("front operator listener is missing")?,
            )
        } else {
            None
        };
        let body = pool_config_body(&PoolConfigBody {
            roles: roles.iter().map(|role| role.to_string()).collect(),
            sv2,
            authority_public: authority_public.to_string(),
            authority_secret: authority_secret.to_string(),
            numeric,
            rpc,
            rpc_user: secrets.node_rpc_user.clone(),
            rpc_pass: secrets.node_rpc_password.clone(),
            reference: reference.to_string(),
            multiplier,
            max_multiplier,
            max_window,
            window_rule: window_rule.to_string(),
            window_n: profile.window_n,
            pow_bits,
            max_slice,
            jd,
            postgres_url,
            redis_url,
            api,
            operator: operator.cloned(),
        });
        let config_path = run_dir
            .join("private")
            .join(format!("pool-{}.toml", front.id));
        write_private_bytes(&config_path, body.as_bytes())?;
        rendered.push(PoolRender {
            front: front.id.clone(),
            numeric,
            config_path,
        });
    }
    Ok(rendered)
}

/// The exact field set the runtime renders into a pool TOML. Kept as a
/// struct so the rendered output can be validated against the real
/// `spc-pool` config parser in tests instead of string matching.
///
/// `operator` is `None` unless the run was started with `--operator-ui`;
/// then it is the planned `front.<id>.operator` loopback endpoint and is
/// rendered as the pool's optional `[operator]` section (EXP-OP).
#[derive(Debug, Clone)]
struct PoolConfigBody {
    roles: Vec<String>,
    sv2: String,
    authority_public: String,
    authority_secret: String,
    numeric: u8,
    rpc: String,
    rpc_user: String,
    rpc_pass: String,
    reference: String,
    multiplier: u64,
    max_multiplier: u64,
    max_window: String,
    window_rule: String,
    window_n: u64,
    pow_bits: String,
    max_slice: u64,
    jd: String,
    postgres_url: String,
    redis_url: String,
    api: String,
    operator: Option<Endpoint>,
}

/// Render one pool config body. The `[operator]` section is appended only
/// when an operator endpoint was planned; absent, the body is byte-for-byte
/// the pre-EXP-OP shape (the section is off by default in the pool).
fn pool_config_body(body: &PoolConfigBody) -> String {
    let operator_section = match &body.operator {
        Some(endpoint) => format!("\n[operator]\nlisten = \"{}\"\n", endpoint.display()),
        None => String::new(),
    };
    format!(
        r#"[pool]
network = "regtest"
roles = [{roles}]
listen = "{sv2}"
authority_public_key = "{authority_public}"
authority_secret_key = "{authority_secret}"
pool_payout_descriptor = "{POOL_PAYOUT_DESCRIPTOR}"
pool_payout_script = "{POOL_PAYOUT_SCRIPT}"
fee_bps = 100
front_id = {numeric}
outbox_max = 4096
tip_divergence_secs = 30

[template]
source = "rpc"
rpc_url = "http://{rpc}"
rpc_user = "{rpc_user}"
rpc_pass = "{rpc_pass}"
extranonce_bytes = 16
refresh_interval_ms = 250

[sv1]
enabled = false

[accounting]
work_unit_version = 2
work_reference_target = "{reference}"
share_work_multiplier = {multiplier}
max_work_multiplier = {max_multiplier}
max_window_target_units = "{max_window}"
window_rule = "{window_rule}"
window_n = {window_n}
pow_limit_bits = 0x{pow_bits}
slice_fee_delta_sats = 0
max_slice_shares = {max_slice}
dust_limit_sats = 330
coinbase_threshold_sats = 330
max_payout_slots = 256
retain_windows = 16
scrub_after_blocks = 100
confirmation_depth = 6
allow_unchecked_chain = false

[extensions]
share_accounting = false
window_hot_blocks = 6

[jd]
listen = "{jd}"
distribution_interval_secs = 60
distribution_rebalance_bps = 100
grace_same_height_only = true

[storage]
postgres = "{postgres_url}"
redis = "{redis_url}"
front_cursor_heartbeat_secs = 30
front_cursor_ttl_secs = 120

[api]
listen = "{api}"
{operator_section}"#,
        roles = body
            .roles
            .iter()
            .map(|role| format!("\"{role}\""))
            .collect::<Vec<_>>()
            .join(", "),
        sv2 = body.sv2,
        authority_public = body.authority_public,
        authority_secret = body.authority_secret,
        numeric = body.numeric,
        rpc = body.rpc,
        rpc_user = body.rpc_user,
        rpc_pass = body.rpc_pass,
        reference = body.reference,
        multiplier = body.multiplier,
        max_multiplier = body.max_multiplier,
        max_window = body.max_window,
        window_rule = body.window_rule,
        window_n = body.window_n,
        pow_bits = body.pow_bits,
        max_slice = body.max_slice,
        jd = body.jd,
        postgres_url = body.postgres_url,
        redis_url = body.redis_url,
        api = body.api,
        operator_section = operator_section,
    )
}

fn miner_config_body(miner: &MinerId) -> String {
    format!(
        "config_version = 1\nminer_id = \"{miner}\"\nevent_queue_capacity = 65536\nconnections = []\n"
    )
}

fn write_initial_miner_configs(engine: &RunEngine, run_dir: &Path) -> Result<()> {
    for miner in &engine.scenario().miners {
        let path = run_dir
            .join("private")
            .join(format!("miner-{}.toml", miner.id));
        if !path.exists() {
            write_private_bytes(&path, miner_config_body(&miner.id).as_bytes())?;
        }
    }
    Ok(())
}

struct MinerConfigProvisioner {
    run_dir: PathBuf,
}

impl MinerProvisioner for MinerConfigProvisioner {
    fn prepare_miner(
        &self,
        _engine: &RunEngine,
        miner: &MinerId,
        implementation: &str,
    ) -> Result<(), String> {
        if implementation != "spc-cpu-miner" {
            return Err(format!(
                "runtime only provisions spc-cpu-miner, not {implementation:?}"
            ));
        }
        let path = self
            .run_dir
            .join("private")
            .join(format!("miner-{miner}.toml"));
        if path.exists() {
            // Idempotent: never regenerate an identity/config after a crash.
            return Ok(());
        }
        write_private_bytes(&path, miner_config_body(miner).as_bytes())
            .map_err(|error| error.to_string())
    }
}

struct PostgresHook {
    wallet_agent: Option<PathBuf>,
    binaries: RuntimeBinaries,
    databases: Vec<PostgresRuntime>,
    secrets: RuntimeSecrets,
    ark: Option<crate::ark_service::ArkBinaries>,
}

impl StartupHook for PostgresHook {
    fn after_fronts(&self, engine: &RunEngine) -> Result<(), String> {
        if let Some(binary) = &self.wallet_agent {
            crate::wallet_bridge::launch(engine, binary).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    fn after_storage_and_nodes(&self, engine: &RunEngine) -> Result<(), String> {
        for pg in &self.databases {
            let password = self
                .secrets
                .postgres
                .get(&pg.id)
                .ok_or_else(|| format!("postgres password missing for {}", pg.id))?;
            let exists = Command::new(self.binaries.postgres_tool("psql"))
                .args([
                    "-h",
                    "127.0.0.1",
                    "-p",
                    &pg.port.to_string(),
                    "-U",
                    &pg.role,
                    "-d",
                    "postgres",
                    "-tAc",
                    &format!(
                        "SELECT 1 FROM pg_database WHERE datname = '{}'",
                        pg.database.replace('\'', "''")
                    ),
                ])
                .env("PGPASSWORD", password)
                .stdin(Stdio::null())
                .output()
                .map_err(|error| format!("psql database probe: {error}"))?;
            if !exists.status.success() {
                return Err(format!(
                    "database probe failed for {}: {}",
                    pg.id,
                    String::from_utf8_lossy(&exists.stderr).trim()
                ));
            }
            if String::from_utf8_lossy(&exists.stdout).trim() == "1" {
                continue;
            }
            let created = Command::new(self.binaries.postgres_tool("createdb"))
                .args([
                    "--host",
                    "127.0.0.1",
                    "--port",
                    &pg.port.to_string(),
                    "--username",
                    &pg.role,
                    "--no-password",
                    &pg.database,
                ])
                .env("PGPASSWORD", password)
                .stdin(Stdio::null())
                .output()
                .map_err(|error| format!("createdb: {error}"))?;
            if !created.status.success() {
                let stderr = String::from_utf8_lossy(&created.stderr);
                if !stderr.contains("already exists") {
                    return Err(format!("createdb failed for {}: {}", pg.id, stderr.trim()));
                }
            }
        }
        if let Some(tools) = &self.ark {
            crate::ark_service::start(engine, &self.binaries, tools)
                .map_err(|e| format!("local Ark startup: {e:#}"))?;
        }
        Ok(())
    }
}

fn collect_node_access(
    engine: &RunEngine,
    listeners: &BTreeMap<String, Endpoint>,
    secrets: &RuntimeSecrets,
    topology: Option<&Arc<TopologyRuntime>>,
) -> Result<Vec<NodeAccess>> {
    let mut nodes = Vec::new();
    for node in &engine.scenario().nodes {
        let endpoint = listeners
            .get(&format!("node.{}.rpc", node.id))
            .cloned()
            .context("node RPC listener is missing")?;
        // A namespaced node is reached on its published address, and the
        // observer has to ask the broker to run the call inside the namespace.
        let endpoint = match topology.and_then(|topology| {
            topology
                .addresses
                .get(&format!("node.{}.rpc_addr", node.id))
        }) {
            Some(address) => Endpoint {
                host: address.clone(),
                port: endpoint.port,
            },
            None => endpoint,
        };
        let broker_endpoint = topology.and_then(|topology| {
            topology
                .plan
                .endpoint_of(&format!("node/{}", node.id))
                .map(str::to_string)
        });
        nodes.push(NodeAccess {
            id: node.id.clone(),
            endpoint,
            user: secrets.node_rpc_user.clone(),
            password: secrets.node_rpc_password.clone(),
            cookie: engine
                .run_dir
                .join("private")
                .join(format!("node-rpc-{}.cookie", node.id)),
            broker_endpoint,
        });
    }
    Ok(nodes)
}

/// Percent encode a value for a connection URL.
fn urlencoding(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn random_hex(bytes: usize) -> String {
    let mut buffer = vec![0u8; bytes];
    rand::thread_rng().fill_bytes(&mut buffer);
    hex::encode(buffer)
}

fn write_private_bytes(path: &Path, body: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("write {}", path.display()))?;
    file.write_all(body)
        .and_then(|_| file.sync_all())
        .with_context(|| format!("write {}", path.display()))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

fn write_private_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let body = serde_json::to_vec_pretty(value)?;
    write_private_bytes(path, &body)
}

// ---------------------------------------------------------------------------
// Live collectors
// ---------------------------------------------------------------------------

struct Observers {
    stop: Arc<AtomicBool>,
    handles: Vec<JoinHandle<()>>,
}

impl Observers {
    fn start(
        engine: Arc<RunEngine>,
        run_dir: &Path,
        binaries: &RuntimeBinaries,
        identities: &[PrivateIdentity],
        nodes: &[NodeAccess],
        pools: &[PoolRender],
        topology: Option<&Arc<TopologyRuntime>>,
    ) -> Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let mut handles = Vec::new();
        let front_ids: BTreeMap<FrontId, u8> = pools
            .iter()
            .map(|pool| (pool.front.clone(), pool.numeric))
            .collect();
        let identity_sp_ids: BTreeMap<IdentityId, String> = identities
            .iter()
            .filter_map(|identity| {
                IdentityId::new(identity.id.clone())
                    .ok()
                    .map(|id| (id, identity.sp_id.clone()))
            })
            .collect();
        let mappings = SpcEvidenceMappings {
            front_ids,
            identity_sp_ids,
        };
        for pool in pools {
            let log = run_dir
                .join("logs")
                .join(format!("front-{}.log", pool.front));
            let private = run_dir
                .join("private")
                .join(format!("spc-evidence-{}.jsonl", pool.front));
            let adapter = SpcEvidenceAdapter::new(
                SourceId::trusted(format!("spc-evidence-{}", pool.front)),
                private,
                mappings.clone(),
            )
            .map_err(|error| anyhow::anyhow!("{error}"))?;
            handles.push(spawn_tail(engine.clone(), log, adapter, stop.clone()));
        }
        for node in nodes.iter().cloned() {
            handles.push(spawn_node_observer(
                engine.clone(),
                node,
                topology.cloned(),
                stop.clone(),
            ));
        }
        for pg in engine.resources_manifest().postgres_databases {
            let password = engine
                .scenario()
                .resources
                .postgres_databases
                .iter()
                .find(|spec| spec.id == pg.id)
                .and_then(|spec| spec.password.clone())
                .unwrap_or_default();
            handles.push(spawn_postgres_observer(
                engine.clone(),
                pg,
                binaries.postgres_tool("psql"),
                password,
                stop.clone(),
            ));
        }
        for redis in engine.resources_manifest().redis_instances {
            let password = engine
                .scenario()
                .resources
                .redis_instances
                .iter()
                .find(|spec| spec.id == redis.id)
                .and_then(|spec| spec.password.clone())
                .unwrap_or_default();
            handles.push(spawn_redis_observer(
                engine.clone(),
                redis,
                binaries.redis_cli.clone(),
                password,
                topology.cloned(),
                stop.clone(),
            ));
        }
        handles.push(spawn_scanner(
            engine.clone(),
            identities.to_vec(),
            nodes.to_vec(),
            topology.cloned(),
            stop.clone(),
        ));
        handles.push(spawn_metrics(engine.clone(), stop.clone()));
        Ok(Self { stop, handles })
    }

    fn stop(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        for handle in self.handles.drain(..) {
            let _ = handle.join();
        }
    }
}

fn sleep_unless_stopped(stop: &AtomicBool, duration: Duration) {
    let deadline = Instant::now() + duration;
    while !stop.load(Ordering::Relaxed) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(100));
    }
}

fn spawn_tail(
    engine: Arc<RunEngine>,
    path: PathBuf,
    mut adapter: SpcEvidenceAdapter,
    stop: Arc<AtomicBool>,
) -> JoinHandle<()> {
    thread::Builder::new()
        .name(format!("spc-lab-tail-{}", path.display()))
        .spawn(move || {
            let mut offset = 0u64;
            let mut pending = Vec::new();
            while !stop.load(Ordering::Relaxed) {
                if !observer_active(&engine) {
                    break;
                }
                match File::open(&path) {
                    Ok(mut file) => {
                        let length = file.metadata().map(|meta| meta.len()).unwrap_or(0);
                        if length < offset {
                            offset = 0;
                            pending.clear();
                        }
                        if file.seek(SeekFrom::Start(offset)).is_ok() {
                            let mut buffer = Vec::new();
                            if file.read_to_end(&mut buffer).is_ok() && !buffer.is_empty() {
                                offset += buffer.len() as u64;
                                pending.extend_from_slice(&buffer);
                                while let Some(index) =
                                    pending.iter().position(|byte| *byte == b'\n')
                                {
                                    let line: Vec<u8> = pending.drain(..=index).collect();
                                    let text = String::from_utf8_lossy(&line[..line.len() - 1]);
                                    ingest_pool_line(&engine, &mut adapter, &text);
                                }
                            }
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(_) => {}
                }
                thread::sleep(Duration::from_millis(50));
            }
        })
        .expect("spawn pool evidence tail")
}

fn ingest_pool_line(engine: &RunEngine, adapter: &mut SpcEvidenceAdapter, line: &str) {
    let Some((event, raw)) = parse_pool_evidence(line) else {
        return;
    };
    let envelope = json!({
        "target": "spc_evidence",
        "fields": { "event": event, "evidence": raw },
    })
    .to_string();
    let _ = adapter.ingest_line(engine.evidence(), &envelope);
}

fn parse_pool_evidence(line: &str) -> Option<(String, Value)> {
    let clean = strip_ansi(line);
    if !clean.contains("spc_evidence") {
        return None;
    }
    let event_at = clean.find("event=")?;
    let after = clean[event_at + "event=".len()..].trim_start();
    let (event, rest) = if let Some(quoted) = after.strip_prefix('"') {
        let end = quoted.find('"')?;
        (quoted[..end].to_string(), &quoted[end + 1..])
    } else {
        let end = after
            .find(|character: char| character.is_whitespace())
            .unwrap_or(after.len());
        (after[..end].to_string(), &after[end..])
    };
    let evidence_at = rest.find("evidence=")?;
    let payload = rest[evidence_at + "evidence=".len()..].trim();
    let mut deserializer = serde_json::Deserializer::from_str(payload);
    let raw = Value::deserialize(&mut deserializer).ok()?;
    Some((event, raw))
}

fn strip_ansi(line: &str) -> String {
    let bytes = line.as_bytes();
    let mut output = String::with_capacity(line.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == 0x1b && bytes.get(index + 1) == Some(&b'[') {
            index += 2;
            while index < bytes.len() && !bytes[index].is_ascii_alphabetic() {
                index += 1;
            }
            if index < bytes.len() {
                index += 1;
            }
            continue;
        }
        let character = match line[index..].chars().next() {
            Some(character) => character,
            None => break,
        };
        output.push(character);
        index += character.len_utf8();
    }
    output
}

fn displacement_result(previous: &(u64, String), current: Option<&str>) -> Option<CanonicalResult> {
    match current {
        Some(hash) if hash != previous.1 => Some(CanonicalResult::No),
        Some(_) => None,
        // A failed/missing RPC read is unknown; it must not invent `no`.
        None => None,
    }
}

/// Read one tip as a single validated (height, hash, parent) triple. A
/// mismatched pair is an error/unknown observation, never a displacement.
/// The parent's value is retained for independent chain inspection, never
/// used to claim a fork merely because heights differ.
fn atomic_tip(control: &RpcNodeControl) -> Result<(u64, String, Option<String>), String> {
    let hash = control
        .call("getbestblockhash", json!([]))?
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| "getbestblockhash did not return a hash".to_string())?;
    let header = control.call("getblockheader", json!([hash.clone()]))?;
    let height = header
        .get("height")
        .and_then(Value::as_u64)
        .ok_or_else(|| "getblockheader did not return the tip height".to_string())?;
    let canonical = control
        .call("getblockhash", json!([height]))?
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| "getblockhash did not return the confirmed tip hash".to_string())?;
    if canonical != hash {
        return Err(format!(
            "racy tip view: header {hash} is not the hash at height {height}"
        ));
    }
    let parent_hash = header
        .get("previousblockhash")
        .and_then(Value::as_str)
        .map(str::to_string);
    if height > 0 && parent_hash.is_none() {
        return Err(format!("tip at height {height} lacks previousblockhash"));
    }
    Ok((height, canonical, parent_hash))
}

fn spawn_node_observer(
    engine: Arc<RunEngine>,
    node: NodeAccess,
    topology: Option<Arc<TopologyRuntime>>,
    stop: Arc<AtomicBool>,
) -> JoinHandle<()> {
    thread::Builder::new()
        .name(format!("spc-lab-node-{}", node.id))
        .spawn(move || {
            let control = node.control(topology.as_ref());
            let mut observed: VecDeque<(u64, String)> = VecDeque::new();
            let mut tip_observations = 0u64;
            let mut distinct_tip_heights = BTreeSet::new();
            let mut displaced = 0u64;
            let mut max_height_seen = 0u64;
            let mut last = None;
            let mut passes = 0u64;
            while !stop.load(Ordering::Relaxed) {
                if !observer_active(&engine) {
                    break;
                }
                passes += 1;
                match atomic_tip(&control) {
                    Ok((height, hash, _parent_hash)) => {
                        max_height_seen = max_height_seen.max(height);
                        // Re-check the bounded last-K tip observations. A
                        // displaced observation becomes `no`; a failed read
                        // stays unknown and changes nothing.
                        let previous: Vec<_> = observed.iter().cloned().collect();
                        for (old_height, old_hash) in previous {
                            let current = control
                                .call("getblockhash", json!([old_height]))
                                .ok()
                                .and_then(|value| value.as_str().map(str::to_string));
                            if displacement_result(
                                &(old_height, old_hash.clone()),
                                current.as_deref(),
                            ) == Some(CanonicalResult::No)
                            {
                                displaced += 1;
                                engine.emit_canonical(
                                    &node.id,
                                    &old_hash,
                                    old_height,
                                    &hash,
                                    CanonicalResult::No,
                                    "tip displacement observed by bounded last-K recheck (historical observation retained)",
                                );
                                observed.retain(|(height, _)| *height != old_height);
                            }
                        }
                        if last.as_ref() != Some(&(height, hash.clone())) {
                            tip_observations += 1;
                            distinct_tip_heights.insert(height);
                            engine.emit_canonical(
                                &node.id,
                                &hash,
                                height,
                                &hash,
                                CanonicalResult::Yes,
                                "tip sampler: atomic getbestblockhash/getblockheader/getblockhash observation; not full canonical coverage or reorg tracking",
                            );
                            last = Some((height, hash.clone()));
                            observed.push_back((height, hash));
                            while observed.len() > 8 {
                                observed.pop_front();
                            }
                        }
                    }
                    Err(error) if passes.is_multiple_of(5) => {
                        engine.evidence().append(
                            SourceId::trusted(format!("node-{}", node.id)),
                            format!("node-tip-unknown-{}", engine.evidence().last_event_id()),
                            EvidenceActor::new(ActorKind::Node, node.id.to_string()),
                            None,
                            EvidencePayload::Unknown(UnknownEvidence {
                                topic: "node_tip_unknown".to_string(),
                                detail: json!({"node": node.id, "error": error}).to_string(),
                            }),
                        );
                    }
                    Err(_) => {}
                }
                if passes.is_multiple_of(5) {
                    engine.evidence().append(
                        SourceId::trusted(format!("node-{}", node.id)),
                        format!("node-scope-{}", engine.evidence().last_event_id()),
                        EvidenceActor::new(ActorKind::Node, node.id.to_string()),
                        None,
                        EvidencePayload::Unknown(UnknownEvidence {
                            topic: "node_canonical_scope".to_string(),
                            detail: json!({
                                "scope": "tip_sampler",
                                "tip_observations": tip_observations,
                                "distinct_tip_heights": distinct_tip_heights.len(),
                                "max_height_seen": max_height_seen,
                                "rechecked_window": 8,
                                "displaced_observations": displaced,
                                "note": "heights are not continuously observed; this is not full canonical coverage or reorg tracking",
                            })
                            .to_string(),
                        }),
                    );
                }
                sleep_unless_stopped(&stop, Duration::from_secs(2));
            }
        })
        .expect("spawn node observer")
}

fn spawn_postgres_observer(
    engine: Arc<RunEngine>,
    pg: PostgresRuntime,
    psql: PathBuf,
    password: String,
    stop: Arc<AtomicBool>,
) -> JoinHandle<()> {
    thread::Builder::new()
        .name(format!("spc-lab-pg-{}", pg.id))
        .spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                if !observer_active(&engine) {
                    break;
                }
                let sql = "SELECT (SELECT count(*) FROM slice), (SELECT count(*) FROM slice_credit), (SELECT count(*) FROM sp_identity);";
                let output = Command::new(&psql)
                    .args([
                        "-h",
                        "127.0.0.1",
                        "-p",
                        &pg.port.to_string(),
                        "-U",
                        &pg.role,
                        "-d",
                        &pg.database,
                        "-tAc",
                        sql,
                    ])
                    .env("PGPASSWORD", &password)
                    .stdin(Stdio::null())
                    .output();
                let detail = match output {
                    Ok(output) if output.status.success() => json!({
                        "database": pg.database,
                        "observation": "private read-only aggregate",
                        "slice_slice_credit_identity_counts": String::from_utf8_lossy(&output.stdout).trim(),
                        "source": "authenticated psql query",
                    }),
                    Ok(output) => json!({
                        "database": pg.database,
                        "error": String::from_utf8_lossy(&output.stderr).trim(),
                    }),
                    Err(error) => json!({ "database": pg.database, "error": error.to_string() }),
                };
                engine.evidence().append(
                    SourceId::trusted(format!("postgres-{}", pg.id)),
                    format!("pg-{}", engine.evidence().last_event_id()),
                    EvidenceActor::new(ActorKind::Harness, "runtime-postgres"),
                    None,
                    EvidencePayload::Unknown(UnknownEvidence {
                        topic: "postgres_observation".to_string(),
                        detail: detail.to_string(),
                    }),
                );
                sleep_unless_stopped(&stop, Duration::from_secs(5));
            }
        })
        .expect("spawn postgres observer")
}

fn spawn_redis_observer(
    engine: Arc<RunEngine>,
    redis: RedisRuntime,
    redis_cli: PathBuf,
    password: String,
    topology: Option<Arc<TopologyRuntime>>,
    stop: Arc<AtomicBool>,
) -> JoinHandle<()> {
    thread::Builder::new()
        .name(format!("spc-lab-redis-{}", redis.id))
        .spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                if !observer_active(&engine) {
                    break;
                }
                // The Redis of a namespaced run lives in the storage
                // namespace of its pool, so the read has to happen there.
                let storage_endpoint = topology.as_ref().and_then(|topology| {
                    topology
                        .plan
                        .endpoint_of(&format!("redis/{}", redis.id))
                        .map(str::to_string)
                });
                let redis_host = topology
                    .as_ref()
                    .and_then(|topology| {
                        topology
                            .addresses
                            .get(&format!("redis.{}.addr", redis.id))
                            .cloned()
                    })
                    .unwrap_or_else(|| "127.0.0.1".to_string());
                let run_redis = |arguments: &[&str]| -> Option<String> {
                    match (&storage_endpoint, topology.as_ref()) {
                        (Some(endpoint), Some(topology)) => {
                            let mut argv = vec![
                                redis_cli.display().to_string(),
                                "-a".to_string(),
                                password.clone(),
                            ];
                            argv.extend(arguments.iter().map(|argument| argument.to_string()));
                            match topology.broker.exec(endpoint, &argv, 15.0) {
                                Ok(value)
                                    if value.get("command_exit_code").and_then(Value::as_i64)
                                        == Some(0) =>
                                {
                                    Some(
                                        value
                                            .get("stdout")
                                            .and_then(Value::as_str)
                                            .unwrap_or_default()
                                            .trim()
                                            .to_string(),
                                    )
                                }
                                _ => None,
                            }
                        }
                        _ => Command::new(&redis_cli)
                            .args(arguments)
                            .env("REDISCLI_AUTH", &password)
                            .stdin(Stdio::null())
                            .output()
                            .ok()
                            .filter(|output| output.status.success())
                            .map(|output| {
                                String::from_utf8_lossy(&output.stdout).trim().to_string()
                            }),
                    }
                };
                let ping = run_redis(&[
                    "-h",
                    &redis_host,
                    "-p",
                    &redis.port.to_string(),
                    "--no-auth-warning",
                    "PING",
                ]);
                let length = run_redis(&[
                    "-h",
                    &redis_host,
                    "-p",
                    &redis.port.to_string(),
                    "--no-auth-warning",
                    "XLEN",
                    "spc:front:events",
                ]);
                let detail = json!({
                    "instance": redis.id,
                    "ping": ping,
                    "front_event_stream_len": length,
                    "source": "authenticated redis-cli reads",
                    "namespace": storage_endpoint,
                });
                engine.evidence().append(
                    SourceId::trusted(format!("redis-{}", redis.id)),
                    format!("redis-{}", engine.evidence().last_event_id()),
                    EvidenceActor::new(ActorKind::Harness, "runtime-redis"),
                    None,
                    EvidencePayload::Unknown(UnknownEvidence {
                        topic: "redis_observation".to_string(),
                        detail: detail.to_string(),
                    }),
                );
                sleep_unless_stopped(&stop, Duration::from_secs(5));
            }
        })
        .expect("spawn redis observer")
}

fn spawn_scanner(
    engine: Arc<RunEngine>,
    identities: Vec<PrivateIdentity>,
    nodes: Vec<NodeAccess>,
    topology: Option<Arc<TopologyRuntime>>,
    stop: Arc<AtomicBool>,
) -> JoinHandle<()> {
    thread::Builder::new()
        .name("spc-lab-scanner".to_string())
        .spawn(move || {
            let Some(node) = nodes.first() else {
                return;
            };
            let control = node.control(topology.as_ref());
            let mut scanned_through: Option<u64> = None;
            while !stop.load(Ordering::Relaxed) {
                if !observer_active(&engine) {
                    break;
                }
                let tip = control
                    .call("getblockcount", json!([]))
                    .ok()
                    .and_then(|value| value.as_u64());
                if let Some(tip) = tip {
                    let start = scanned_through.map(|height| height + 1).unwrap_or(0);
                    let mut found_total = 0u64;
                    let mut blocks = 0u64;
                    let mut skipped = Vec::new();
                    let mut next = start;
                    for height in start..=tip {
                        let block = control
                            .call("getblockhash", json!([height]))
                            .ok()
                            .and_then(|value| value.as_str().map(str::to_string))
                            .and_then(|hash| {
                                control
                                    .call("getblock", json!([hash, 0]))
                                    .ok()
                                    .and_then(|value| value.as_str().map(str::to_string))
                            })
                            .and_then(|hex| hex::decode(hex).ok())
                            .and_then(|bytes| bitcoin::consensus::deserialize(&bytes).ok());
                        let Some(block) = block else {
                            // Do not advance past a failed height: record it and
                            // retry from the same contiguous cursor next pass.
                            skipped.push(height);
                            break;
                        };
                        blocks += 1;
                        next = height + 1;
                        for identity in &identities {
                            let (Ok(scan), Ok(spend)) =
                                (identity.scan_secret(), identity.spend_public())
                            else {
                                continue;
                            };
                            if let Ok(outcome) = spc_scan::scan_block(&block, &scan, &spend) {
                                found_total += outcome.found.len() as u64;
                            }
                        }
                    }
                    let cursor = if next == 0 { None } else { Some(next - 1) };
                    engine.evidence().append(
                        SourceId::trusted("scanner"),
                        format!("scan-{}-{}", tip, engine.evidence().last_event_id()),
                        EvidenceActor::new(ActorKind::Harness, "runtime-scanner"),
                        None,
                        EvidencePayload::Unknown(UnknownEvidence {
                            topic: "recipient_scan".to_string(),
                            detail: json!({
                                "node": node.id,
                                "requested_tip": tip,
                                "scan_cursor": cursor,
                                "through_height": tip,
                                "blocks_scanned_this_pass": blocks,
                                "skipped_heights": skipped,
                                "retry_after_height": skipped.first(),
                                "recipients": identities.len(),
                                "outputs_found": found_total,
                                "source": "in-process spc_scan::scan_block over canonical blocks fetched by RPC",
                            })
                            .to_string(),
                        }),
                    );
                    scanned_through = cursor;
                }
                sleep_unless_stopped(&stop, Duration::from_secs(10));
            }
        })
        .expect("spawn scanner")
}

fn spawn_metrics(engine: Arc<RunEngine>, stop: Arc<AtomicBool>) -> JoinHandle<()> {
    thread::Builder::new()
        .name("spc-lab-metrics".to_string())
        .spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                // Do not compete with a stop for the miner control lock: the
                // snapshot request can wait on a process that is being torn
                // down. Metrics are only meaningful while the run is active.
                if !observer_active(&engine) {
                    break;
                }
                let snapshots = engine.runtime_miner_snapshots();
                let mut miners = Vec::new();
                for snapshot in snapshots {
                    let miner_id = snapshot.get("miner_id").cloned().unwrap_or(Value::Null);
                    let result = snapshot.get("result").cloned().unwrap_or(Value::Null);
                    let mut requested_total = 0u64;
                    let mut measured_total = 0u64;
                    let mut attempted_total = 0u64;
                    let mut stale_cancelled = 0u64;
                    if let Some(workers) = result.get("workers").and_then(Value::as_array) {
                        for worker in workers {
                            requested_total = requested_total.saturating_add(
                                worker
                                    .pointer("/requested/hashes_per_second")
                                    .and_then(Value::as_u64)
                                    .unwrap_or(0),
                            );
                            measured_total = measured_total.saturating_add(
                                worker
                                    .get("achieved_hps")
                                    .and_then(Value::as_u64)
                                    .unwrap_or(0),
                            );
                            attempted_total = attempted_total.saturating_add(
                                worker
                                    .get("attempted")
                                    .and_then(Value::as_str)
                                    .and_then(|value| value.parse::<u64>().ok())
                                    .unwrap_or(0),
                            );
                            stale_cancelled = stale_cancelled.saturating_add(
                                worker
                                    .get("stale_cancelled")
                                    .and_then(Value::as_u64)
                                    .unwrap_or(0),
                            );
                        }
                    }
                    miners.push(json!({
                        "miner_id": miner_id,
                        "requested_hps": requested_total,
                        "measured_hps": measured_total,
                        "attempted_total": attempted_total,
                        "stale_cancelled": stale_cancelled,
                    }));
                }
                let mut counters = BTreeMap::new();
                let mut connection_fronts = BTreeMap::new();
                for (miner, _, connections, _) in engine.runtime_miner_specs() {
                    for connection in connections {
                        connection_fronts.insert(
                            (miner.to_string(), connection.id.to_string()),
                            connection.front_id.to_string(),
                        );
                    }
                }
                let mut front_evidence: BTreeMap<String, BTreeMap<String, u64>> =
                    BTreeMap::new();
                for event in engine.evidence().events() {
                    match &event.payload {
                        EvidencePayload::SpcEvidence(record)
                            if record.event == "accounted" =>
                        {
                            if let Some(front) = record.front_id.as_ref() {
                                *front_evidence
                                    .entry(front.to_string())
                                    .or_default()
                                    .entry("credited".to_string())
                                    .or_insert(0) += 1;
                            }
                        }
                        EvidencePayload::MinerEvent { miner_id, name, payload, .. } => {
                            let connection_id = payload
                                .get("connection_id")
                                .or_else(|| payload.pointer("/share/connection_id"))
                                .and_then(Value::as_str);
                            let front = connection_id.and_then(|connection| {
                                connection_fronts
                                    .get(&(miner_id.to_string(), connection.to_string()))
                            });
                            if let Some(front) = front {
                                let metrics = front_evidence.entry(front.clone()).or_default();
                                if let Some(bucket) = match name.as_str() {
                                    "submitted" => Some("submitted"),
                                    "ingress_ack" => Some("acknowledged"),
                                    "ingress_rejected" => Some("rejected"),
                                    "submissions_unknown" => Some("transport_unknown"),
                                    "block_qualified" => Some("block_qualified"),
                                    _ => None,
                                } {
                                    *metrics.entry(bucket.to_string()).or_insert(0) += 1;
                                }
                                if name == "ingress_rejected"
                                    && payload.get("error_code").and_then(Value::as_str)
                                        == Some("stale-share")
                                {
                                    *metrics.entry("stale".to_string()).or_insert(0) += 1;
                                }
                                if name == "submitted"
                                    && payload.pointer("/share/status").and_then(Value::as_str)
                                        == Some("transport_unknown")
                                {
                                    *metrics.entry("transport_unknown".to_string()).or_insert(0) += 1;
                                }
                            }
                            let bucket = match name.as_str() {
                                "submitted" => Some("submitted"),
                                "ingress_ack" => Some("ingress_ack"),
                                "ingress_rejected" => Some("ingress_rejected"),
                                "submissions_unknown" => Some("transport_unknown"),
                                "block_qualified" => Some("block_qualified_shares"),
                                "jobs_invalidated" => Some("jobs_invalidated"),
                                _ => None,
                            };
                            if let Some(bucket) = bucket {
                                *counters.entry(bucket).or_insert(0u64) += 1;
                            }
                            if name == "submitted" {
                                if let Some(status) = payload.pointer("/share/status").and_then(Value::as_str) {
                                    if status == "transport_unknown" {
                                        *counters.entry("transport_unknown").or_insert(0u64) += 1;
                                    }
                                }
                            }
                        }
                        EvidencePayload::CanonicalAt(_) => {
                            *counters.entry("canonical_observations").or_insert(0u64) += 1;
                        }
                        _ => {}
                    }
                }
                engine.evidence().append(
                    SourceId::trusted("metrics"),
                    format!("metrics-{}", engine.evidence().last_event_id()),
                    EvidenceActor::new(ActorKind::Observer, "runtime-metrics"),
                    None,
                    EvidencePayload::Unknown(UnknownEvidence {
                        topic: "runtime_metrics".to_string(),
                        detail: json!({
                            "miners": miners,
                            "counters": counters,
                            "front_evidence": front_evidence,
                            "rate_basis": "requested from the durable worker budget; measured from the approved miner v1 Snapshot attempt counters",
                            "note": "block-qualified shares are not canonical blocks; no causal hashrate-response claim is made",
                        })
                        .to_string(),
                    }),
                );
                sleep_unless_stopped(&stop, Duration::from_secs(3));
            }
        })
        .expect("spawn metrics observer")
}

/// Catalog used by the browser's add-miner form. It exposes only public
/// scenario facts and never identity secret material.
pub fn catalog(engine: &RunEngine) -> Value {
    let mut fronts = Vec::new();
    for front in &engine.scenario().fronts {
        let endpoint = engine
            .resources_manifest()
            .listeners
            .get(&format!("front.{}.sv2", front.id))
            .map(|endpoint| endpoint.display());
        fronts.push(json!({
            "id": front.id,
            "pool_id": front.pool_id,
            "sv2": endpoint,
        }));
    }
    let specs = engine.runtime_miner_specs();
    let mut used = BTreeSet::new();
    for (_, _, connections, _) in &specs {
        for connection in connections {
            used.insert(connection.identity_id.to_string());
        }
    }
    let identities: Vec<Value> = engine
        .scenario()
        .identities
        .iter()
        .map(|identity| {
            let address = match &identity.address_source {
                AddressSource::Fixture { fixture } => fixture.clone(),
                AddressSource::Explicit { address } => address.clone(),
            };
            json!({
                "id": identity.id,
                "address": address,
                "used": used.contains(&identity.id.to_string()),
            })
        })
        .collect();
    let mut miners = Vec::new();
    for (id, implementation, connections, workers) in specs {
        miners.push(json!({
            "id": id,
            "implementation": implementation,
            "connections": connections,
            "workers": workers,
        }));
    }
    json!({
        "run_id": engine.run_id(),
        "fronts": fronts,
        "identities": identities,
        "miners": miners,
        "defaults": {
            "miner_implementation": "spc-cpu-miner",
            "hashes_per_second": "100",
        },
        "notes": [
            "an added miner reuses a declared persistent SP identity; identities are never regenerated at add time",
            "commands are durable; a retried command id does not duplicate the miner",
        ],
    })
}

/// Keep the compiler honest about the end-to-end registry type.
pub fn pool_config_paths(runtime: &Runtime) -> Vec<PathBuf> {
    runtime
        .pools
        .iter()
        .map(|pool| pool.config_path.clone())
        .collect()
}

/// A live node accessor is useful for the following integration gate and for
/// report generation. It does not start anything.
pub fn node_endpoints(runtime: &Runtime) -> BTreeMap<String, String> {
    runtime
        .nodes
        .iter()
        .map(|node| (node.id.to_string(), node.endpoint.display()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scoped_store(label: &str) -> (crate::evidence::EvidenceStore, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "spc-lab-runtime-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        (
            crate::evidence::EvidenceStore::new(
                crate::ids::RunId::new("runtime-test").unwrap(),
                10_000,
            ),
            dir,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn ingest(
        store: &crate::evidence::EvidenceStore,
        path: &Path,
        front: u8,
        job_id: &str,
        generation: u64,
        target: &str,
        m_issue: u64,
        accounted: bool,
        sp_id: &str,
    ) {
        ingest_with_hash(
            store,
            path,
            front,
            job_id,
            generation,
            target,
            m_issue,
            accounted,
            sp_id,
            RecordedHash::Default,
        );
    }

    /// The recorded share hash of an accounted record.
    enum RecordedHash {
        /// The valid 32 byte hash the other tests use.
        Default,
        /// No share hash at all: the summary has to treat it as unknown.
        Missing,
        /// An explicit hash text, valid or not.
        Text(String),
    }

    /// The same ingest with an explicit recorded share hash.
    #[allow(clippy::too_many_arguments)]
    fn ingest_with_hash(
        store: &crate::evidence::EvidenceStore,
        path: &Path,
        front: u8,
        job_id: &str,
        generation: u64,
        target: &str,
        m_issue: u64,
        accounted: bool,
        sp_id: &str,
        share_hash: RecordedHash,
    ) {
        let mut mappings = SpcEvidenceMappings::default();
        mappings
            .front_ids
            .insert(FrontId::new(format!("front-{front}")).unwrap(), front);
        mappings
            .identity_sp_ids
            .insert(IdentityId::new("id-a").unwrap(), sp_id.to_string());
        let mut adapter = SpcEvidenceAdapter::new(
            SourceId::trusted(format!("test-front-{front}")),
            path.join(format!("capture-{front}-{job_id}-{accounted}.jsonl")),
            mappings,
        )
        .unwrap();
        let event = if accounted { "accounted" } else { "job_issued" };
        let mut raw = json!({
            "activation": "future",
            "channel_id": 1,
            "front_id": front,
            "generation": generation.to_string(),
            "issued_at": 1_700_000_000_000u64 + generation,
            "job_id": job_id,
            "m_issue": m_issue,
            "nbits": 545259519,
            "share_target": target,
            "share_target_byte_order": "big-endian unsigned 256-bit",
            "work_unit_fingerprint": "7fe9ed1a4e2d4279b302cbb0bcf709758f03dc44db6dab82ac40c04eb0d77c6b",
            "work_unit_version": 2,
        });
        if accounted {
            raw["d"] = json!(m_issue);
            raw["sp_id"] = json!(sp_id);
            match share_hash {
                RecordedHash::Default => {
                    raw["share_hash"] = json!("00".repeat(32));
                }
                RecordedHash::Missing => {}
                RecordedHash::Text(hash) => {
                    raw["share_hash"] = json!(hash);
                }
            }
            raw["nonce"] = json!(1);
            raw["ntime"] = json!(1_700_000_000);
            raw["version"] = json!(0x2000_0000u32);
            raw["extranonce"] = json!("0000000000000000");
            raw["share_index"] = json!(0);
        }
        let envelope = json!({
            "target": "spc_evidence",
            "fields": {"event": event, "evidence": raw},
        })
        .to_string();
        adapter.ingest_line(store, &envelope).unwrap();
    }

    #[test]
    fn accounted_target_summary_joins_front_job_and_generation() {
        let (store, dir) = scoped_store("join");
        let sp = "aa".repeat(32);
        let target = "7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
        ingest(&store, &dir, 1, "00000001", 7, target, 2, false, &sp);
        ingest(&store, &dir, 2, "00000001", 7, target, 2, false, &sp);
        ingest(&store, &dir, 1, "00000001", 7, target, 2, true, &sp);
        ingest(&store, &dir, 2, "00000001", 7, target, 2, true, &sp);
        let summary = accounted_target_summary(&store.events());
        assert_eq!(summary["status"], "consistent");
        assert_eq!(summary["records_checked_against_job"], 2);
        assert_eq!(summary["scoped_job_keys"], 2);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn self_consistent_record_with_wrong_announced_target_is_inconsistent() {
        let (store, dir) = scoped_store("tamper");
        let sp = "bb".repeat(32);
        let announced = "7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
        let tampered = "3fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
        ingest(&store, &dir, 1, "00000002", 8, announced, 2, false, &sp);
        ingest(&store, &dir, 1, "00000002", 8, tampered, 2, true, &sp);
        let summary = accounted_target_summary(&store.events());
        assert_eq!(summary["status"], "inconsistent");
        assert_eq!(summary["target_mismatches"], 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn conflicting_duplicate_or_missing_jobs_never_pass() {
        let (store, dir) = scoped_store("unknown");
        let sp = "cc".repeat(32);
        let a = "7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
        let b = "3fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
        ingest(&store, &dir, 1, "00000003", 9, a, 2, false, &sp);
        ingest(&store, &dir, 1, "00000003", 9, b, 2, false, &sp);
        ingest(&store, &dir, 1, "00000003", 9, a, 2, true, &sp);
        ingest(&store, &dir, 1, "00000004", 10, a, 2, true, &sp);
        let summary = accounted_target_summary(&store.events());
        assert_eq!(summary["status"], "unknown");
        assert_eq!(summary["duplicate_conflicting_job_keys"], 1);
        assert_eq!(summary["missing_job_records"], 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// LP-topology merge fix F1: a malformed recorded share hash must never
    /// be able to count as consistent.
    ///
    /// A hash that is not hex, or that is not exactly the 32 raw SHA256d
    /// bytes, is malformed evidence.  Each case is injected on its own and
    /// the summary has to refuse it.
    #[test]
    fn malformed_share_hash_is_never_consistent() {
        let target = "7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
        let cases: Vec<(&str, String)> = vec![
            ("nonhex", "zz".repeat(32)),
            ("31 bytes", "00".repeat(31)),
            ("33 bytes", "00".repeat(33)),
            ("empty", String::new()),
        ];
        for (label, hash) in cases {
            let (store, dir) = scoped_store(&format!("f1-{}", label.replace(' ', "-")));
            let sp = "ee".repeat(32);
            ingest_with_hash(
                &store,
                &dir,
                1,
                "00000020",
                20,
                target,
                2,
                false,
                &sp,
                RecordedHash::Default,
            );
            ingest_with_hash(
                &store,
                &dir,
                1,
                "00000020",
                20,
                target,
                2,
                true,
                &sp,
                RecordedHash::Text(hash.clone()),
            );
            let summary = accounted_target_summary(&store.events());
            assert_eq!(
                summary["status"], "inconsistent",
                "a {label} share hash must never be consistent: {summary}"
            );
            assert_eq!(
                summary["malformed_share_hashes"], 1,
                "the {label} share hash is counted as malformed: {summary}"
            );
            assert_eq!(summary["consistent_records"], 0);
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    /// A valid 32 byte hash is still compared against the announced target.
    #[test]
    fn valid_share_hash_is_compared_against_the_announced_target() {
        // Below the announced target: consistent.
        let (store, dir) = scoped_store("f1-valid-below");
        let sp = "ef".repeat(32);
        let target = "7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
        ingest_with_hash(
            &store,
            &dir,
            1,
            "00000021",
            21,
            target,
            2,
            false,
            &sp,
            RecordedHash::Default,
        );
        ingest_with_hash(
            &store,
            &dir,
            1,
            "00000021",
            21,
            target,
            2,
            true,
            &sp,
            RecordedHash::Text("00".repeat(32)),
        );
        let summary = accounted_target_summary(&store.events());
        assert_eq!(summary["status"], "consistent");
        assert_eq!(summary["malformed_share_hashes"], 0);
        let _ = std::fs::remove_dir_all(dir);

        // Above the announced target: inconsistent.
        let (store, dir) = scoped_store("f1-valid-above");
        let sp = "f0".repeat(32);
        let small = "0000000000000000000000000000000000000000000000000000000000000001";
        ingest_with_hash(
            &store,
            &dir,
            1,
            "00000022",
            22,
            small,
            2,
            false,
            &sp,
            RecordedHash::Default,
        );
        ingest_with_hash(
            &store,
            &dir,
            1,
            "00000022",
            22,
            small,
            2,
            true,
            &sp,
            RecordedHash::Text("ff".repeat(32)),
        );
        let summary = accounted_target_summary(&store.events());
        assert_eq!(summary["status"], "inconsistent");
        assert_eq!(summary["hashes_above_announced_target"], 1);
        assert_eq!(summary["malformed_share_hashes"], 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A missing share hash stays unknown; it is not a defect and it is not
    /// a pass.
    #[test]
    fn missing_share_hash_stays_unknown() {
        let (store, dir) = scoped_store("f1-missing");
        let sp = "e0".repeat(32);
        let target = "7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
        ingest_with_hash(
            &store,
            &dir,
            1,
            "00000023",
            23,
            target,
            2,
            false,
            &sp,
            RecordedHash::Default,
        );
        ingest_with_hash(
            &store,
            &dir,
            1,
            "00000023",
            23,
            target,
            2,
            true,
            &sp,
            RecordedHash::Missing,
        );
        let summary = accounted_target_summary(&store.events());
        assert_eq!(summary["status"], "unknown");
        assert_eq!(summary["consistent_records"], 0);
        assert_eq!(summary["malformed_share_hashes"], 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn accounted_work_assertion_is_front_scoped() {
        let (store, dir) = scoped_store("assert");
        let sp = "dd".repeat(32);
        let target = "7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
        ingest(&store, &dir, 1, "00000005", 11, target, 2, false, &sp);
        ingest(&store, &dir, 1, "00000005", 11, target, 2, true, &sp);
        let pass = crate::scenario::AssertionSpec {
            id: "a".to_string(),
            kind: crate::scenario::AssertionKind::Evidence,
            predicate: "accounted_work(front=front-1,min=1)".to_string(),
            deadline_secs: 1,
        };
        let missing = crate::scenario::AssertionSpec {
            id: "b".to_string(),
            kind: crate::scenario::AssertionKind::Evidence,
            predicate: "accounted_work(front=front-2,min=1)".to_string(),
            deadline_secs: 1,
        };
        assert_eq!(
            store.evaluate_assertion(&pass),
            crate::evidence::AssertionEvaluation::Passed
        );
        assert!(matches!(
            store.evaluate_assertion(&missing),
            crate::evidence::AssertionEvaluation::Inconclusive(_)
        ));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn displacement_is_no_only_for_an_observed_different_hash() {
        let previous = (7u64, "aa".to_string());
        assert_eq!(
            displacement_result(&previous, Some("bb")),
            Some(CanonicalResult::No)
        );
        assert_eq!(displacement_result(&previous, Some("aa")), None);
        assert_eq!(displacement_result(&previous, None), None);
    }

    #[test]
    fn parses_ansi_coloured_pool_evidence_lines() {
        let line = "\u{1b}[2m2026-11-06T07:49:11.323396Z\u{1b}[0m \u{1b}[32m INFO\u{1b}[0m \u{1b}[2mspc_evidence\u{1b}[0m\u{1b}[2m:\u{1b}[0m \u{1b}[3mevent\u{1b}[0m\u{1b}[2m=\u{1b}[0m\"job_issued\" \u{1b}[3mevidence\u{1b}[0m\u{1b}[2m=\u{1b}[0m{\"activation\":\"future\",\"channel_id\":1,\"front_id\":1,\"generation\":\"1793951347188\",\"issued_at\":1793951351323,\"job_id\":\"afd531f4\",\"m_issue\":1,\"nbits\":545259519,\"share_target\":\"7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff\",\"work_unit_fingerprint\":\"7fe9ed1a4e2d4279b302cbb0bcf709758f03dc44db6dab82ac40c04eb0d77c6b\",\"work_unit_version\":2}";
        let (event, raw) = parse_pool_evidence(line).expect("line must parse");
        assert_eq!(event, "job_issued");
        assert_eq!(raw["job_id"], "afd531f4");
        assert_eq!(raw["channel_id"], 1);
    }

    #[test]
    fn namespaced_storage_urls_keep_published_address_auth_database_and_pg_port() {
        let pool = crate::ids::PoolId::new("pool-a").unwrap();
        let redis = RedisRuntime {
            id: "redis-pool-a".to_string(),
            owner_pool_id: pool.clone(),
            port: 41_234,
            dir: PathBuf::from("/run/redis"),
            url: "redis://127.0.0.1:41000/0".to_string(),
        };
        let redis_url = topology_redis_url(&redis, "10.250.0.2", "lab/secret");
        assert_eq!(redis_url, "redis://:lab%2Fsecret@10.250.0.2:41234/0");
        assert!(!redis_url.contains("127.0.0.1"));

        let pg = PostgresRuntime {
            id: "pg-pool-a".to_string(),
            owner_pool_id: pool,
            database: "spc_a".to_string(),
            role: "spc_a".to_string(),
            port: 43_210,
            dir: PathBuf::from("/run/pg"),
            url: "postgres://spc_a@127.0.0.1:1/spc_a".to_string(),
        };
        let pg_url = topology_postgres_url(&pg, "pg/secret");
        assert_eq!(
            pg_url,
            "postgres://spc_a:pg%2Fsecret@localhost:43210/spc_a?host=/run/pg/socket"
        );
        assert!(pg_url.contains(":43210/"));
        assert!(pg_url.ends_with("host=/run/pg/socket"));
    }

    #[test]
    fn p2p_publication_connects_each_node_to_the_other_and_storage_excludes_postgres() {
        let scenario = Scenario::from_json(include_str!(
            "../scenarios/lab-network-runtime-two-pools.json"
        ))
        .unwrap();
        let plan = TopologyPlan::from_scenario(&scenario).unwrap();
        let mut placement = Placement::default();
        for (node, address) in [("node-a", "10.250.0.25"), ("node-b", "10.250.0.26")] {
            placement.addresses.insert(
                node.to_string(),
                vec![crate::namespace::PublishedAddress {
                    link_id: "link-p2p-nodes".to_string(),
                    addr: address.to_string(),
                    prefix: 30,
                    iface: "test0".to_string(),
                    peer_addr: String::new(),
                    family: "inet".to_string(),
                }],
            );
        }
        let addresses = topology_addresses(&scenario, &plan, &placement);
        assert_eq!(
            addresses
                .get("node.node-a.p2p_connect_addr")
                .map(String::as_str),
            Some("10.250.0.26")
        );
        assert_eq!(
            addresses
                .get("node.node-b.p2p_connect_addr")
                .map(String::as_str),
            Some("10.250.0.25")
        );
        let storage = plan.endpoint("storage-pool-a").unwrap();
        assert_eq!(storage.actors, vec!["redis/redis-pool-a".to_string()]);
        assert!(!plan.actor_endpoints.contains_key("postgres/pg-pool-a"));
        assert_eq!(
            plan.links
                .iter()
                .filter(|link| link.a == "node-a" && link.b == "node-b")
                .count(),
            1,
            "the declared consensus link is the only P2P data route"
        );
    }

    // ---- EXP-OP operator runtime wiring ----

    fn operator_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "spc-lab-op-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A `PoolConfigBody` whose values mirror a real host render (the live
    /// lab fixture profile), so the rendered TOML is validated by the real
    /// `spc-pool` parser rather than by string matching.
    fn operator_pool_body(dir: &Path, operator: Option<Endpoint>) -> String {
        let pool_id = crate::ids::PoolId::new("op-render").unwrap();
        let authority = crate::secrets::PoolAuthority::generate(&pool_id, dir).unwrap();
        pool_config_body(&PoolConfigBody {
            roles: vec!["api".to_string(), "front".to_string(), "payout".to_string()],
            sv2: "127.0.0.1:18443".to_string(),
            authority_public: authority.public_base58().to_string(),
            authority_secret: authority.secret_base58(),
            numeric: 1,
            rpc: "127.0.0.1:18444".to_string(),
            rpc_user: "rpcuser".to_string(),
            rpc_pass: "rpcpass".to_string(),
            reference: "7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
                .to_string(),
            multiplier: 2,
            max_multiplier: 4_294_967_296,
            max_window: "18446744073709551616".to_string(),
            window_rule: "network_block_work".to_string(),
            window_n: 8,
            pow_bits: "207fffff".to_string(),
            max_slice: 65_000,
            jd: "127.0.0.1:18445".to_string(),
            postgres_url: "postgres://lab:lab@127.0.0.1:5432/pool".to_string(),
            redis_url: "redis://127.0.0.1:6379/0".to_string(),
            api: "127.0.0.1:18446".to_string(),
            operator,
        })
    }

    #[test]
    fn pool_config_body_without_operator_is_pre_op_shape_and_parses() {
        let dir = operator_dir("pool-body-plain");
        let body = operator_pool_body(&dir, None);
        assert!(!body.contains("[operator]"));
        let path = dir.join("pool-plain.toml");
        std::fs::write(&path, &body).unwrap();
        let config = spc_pool::config::Config::load(&path).unwrap();
        assert!(config.operator.listen.is_none());
    }

    #[test]
    fn pool_config_body_with_operator_parses_and_validates() {
        let dir = operator_dir("pool-body-operator");
        let body = operator_pool_body(&dir, Some(Endpoint::loopback(18481)));
        assert!(body.contains("\n[operator]\nlisten = \"127.0.0.1:18481\"\n"));
        let path = dir.join("pool-operator.toml");
        std::fs::write(&path, &body).unwrap();
        let config = spc_pool::config::Config::load(&path).unwrap();
        assert_eq!(config.operator.listen.as_deref(), Some("127.0.0.1:18481"));
    }

    #[test]
    fn pool_parser_rejects_operator_without_api_role_and_non_loopback() {
        let dir = operator_dir("pool-body-reject");
        // No api role: the operator listener runs inside the pool API
        // process, so the parser must reject the section.
        let no_api = operator_pool_body(&dir, Some(Endpoint::loopback(18481)))
            .replace("\"api\", \"front\", \"payout\"", "\"front\", \"payout\"");
        let path = dir.join("pool-no-api.toml");
        std::fs::write(&path, &no_api).unwrap();
        let error = spc_pool::config::Config::load(&path)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("requires the api role"),
            "unexpected error: {error}"
        );

        // Non-loopback listen: defense in depth on the pool side; the
        // runtime's allocator can never produce this value.
        let public = operator_pool_body(&dir, Some(Endpoint::loopback(18481)))
            .replace("127.0.0.1:18481", "10.0.0.8:18481");
        let path = dir.join("pool-public.toml");
        std::fs::write(&path, &public).unwrap();
        let error = spc_pool::config::Config::load(&path)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("not a loopback address"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn rendered_operator_section_fails_closed_on_a_pre_op_pool_config() {
        // The pre-OP pool Config (base b0ab4ca) used deny_unknown_fields
        // and had no `operator` field. A struct mirroring that top-level
        // shape proves the rendered [operator] section makes an old pool
        // binary reject the whole config (process exit, front readiness
        // never reached) instead of silently ignoring the section. The
        // fields exist only to give the mirror its shape.
        #[derive(Debug, Deserialize)]
        #[serde(deny_unknown_fields)]
        #[allow(dead_code)]
        struct PreOpConfig {
            pool: toml::Value,
            template: toml::Value,
            sv1: toml::Value,
            accounting: toml::Value,
            extensions: toml::Value,
            jd: toml::Value,
            storage: toml::Value,
            api: toml::Value,
        }
        let dir = operator_dir("pool-body-pre-op");
        let plain = operator_pool_body(&dir, None);
        toml::from_str::<PreOpConfig>(&plain)
            .expect("pre-OP shape accepts the body without the operator section");
        let with_operator = operator_pool_body(&dir, Some(Endpoint::loopback(18481)));
        let error = toml::from_str::<PreOpConfig>(&with_operator)
            .expect_err("pre-OP shape must reject the operator section");
        assert!(error.to_string().contains("operator"));
    }

    #[test]
    fn plan_resources_allocates_operator_listeners_only_when_opted_in() {
        let fixture = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/scenario_v1_one_pool_two_miners.json"
        );
        let scenario =
            crate::scenario::Scenario::from_json(&std::fs::read_to_string(fixture).unwrap())
                .unwrap();
        let run_id = crate::ids::RunId::new("op-plan-test").unwrap();
        let dir = operator_dir("plan-operator");
        let without = crate::resource::plan_resources(&scenario, &run_id, &dir, false).unwrap();
        assert!(!without.listeners.contains_key("front.front-a1.operator"));
        let with = crate::resource::plan_resources(&scenario, &run_id, &dir, true).unwrap();
        let operator = with.listener("front.front-a1.operator").unwrap();
        assert_eq!(operator.host, "127.0.0.1");
        assert_ne!(operator.port, 0);
        // One fresh loopback port per front, distinct from every other
        // planned listener of the same run.
        let mut ports: Vec<u16> = with.listeners.values().map(|e| e.port).collect();
        ports.sort_unstable();
        ports.dedup();
        assert_eq!(
            ports.len(),
            with.listeners.len(),
            "every planned listener holds a distinct port"
        );
        assert_eq!(with.listeners.len(), without.listeners.len() + 1);
    }

    fn dummy_binaries(dir: &Path) -> RuntimeBinaries {
        let file = |name: &str| {
            let path = dir.join(name);
            std::fs::write(&path, b"").unwrap();
            path
        };
        let pg_dir = dir.join("pg");
        std::fs::create_dir_all(&pg_dir).unwrap();
        for tool in ["initdb", "postgres", "createdb", "psql"] {
            std::fs::write(pg_dir.join(tool), b"").unwrap();
        }
        RuntimeBinaries {
            txd: None,
            traffic_probe: None,
            traffic_probe_args: Vec::new(),
            node: file("node"),
            pool: file("pool"),
            miner: file("miner"),
            scanner: None,
            postgres_bin_dir: pg_dir,
            redis_server: file("redis-server"),
            redis_cli: file("redis-cli"),
        }
    }

    #[test]
    fn runtime_prepare_rejects_operator_ui_with_topology_before_any_effect() {
        let dir = operator_dir("prepare-topology");
        let error = Runtime::prepare(RuntimeOptions {
            wallet_agent: None,
            ark: None,
            scenario_path: PathBuf::from("/nonexistent/scenario.json"),
            run_dir: dir.join("run"),
            binaries: RuntimeBinaries {
                txd: None,
                traffic_probe: None,
                traffic_probe_args: Vec::new(),
                node: PathBuf::from("/nonexistent/node"),
                pool: PathBuf::from("/nonexistent/pool"),
                miner: PathBuf::from("/nonexistent/miner"),
                scanner: None,
                postgres_bin_dir: PathBuf::from("/nonexistent/pg"),
                redis_server: PathBuf::from("/nonexistent/redis-server"),
                redis_cli: PathBuf::from("/nonexistent/redis-cli"),
            },
            pow_bits: "207fffff".to_string(),
            base_revision: "test".to_string(),
            dirty_tree_hash: "clean".to_string(),
            evidence_capacity: 1_000,
            topology: Some(TopologyRuntimeOptions {
                broker_script: PathBuf::from("/nonexistent/broker"),
                unshare: PathBuf::from("/nonexistent/unshare"),
                nsenter: PathBuf::from("/nonexistent/nsenter"),
            }),
            operator_ui: true,
        })
        .err()
        .expect("prepare must reject operator-ui with topology")
        .to_string();
        assert!(
            error.contains("host-runtime only"),
            "unexpected error: {error}"
        );
        assert!(
            !dir.join("run").exists(),
            "the rejection must happen before the run directory exists"
        );
    }

    #[test]
    fn runtime_prepare_rejects_operator_ui_without_api_role_before_run_dir() {
        let dir = operator_dir("prepare-no-api");
        let fixture = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/scenario_v1_one_pool_two_miners.json"
        );
        let no_api = std::fs::read_to_string(fixture).unwrap().replace(
            "\"roles\": [\"front\", \"payout\", \"api\"]",
            "\"roles\": [\"front\", \"payout\"]",
        );
        let scenario_path = dir.join("scenario-no-api.json");
        std::fs::write(&scenario_path, no_api).unwrap();
        let run_dir = dir.join("run");
        let error = Runtime::prepare(RuntimeOptions {
            wallet_agent: None,
            ark: None,
            scenario_path,
            run_dir: run_dir.clone(),
            binaries: dummy_binaries(&dir),
            pow_bits: "207fffff".to_string(),
            base_revision: "test".to_string(),
            dirty_tree_hash: "clean".to_string(),
            evidence_capacity: 1_000,
            topology: None,
            operator_ui: true,
        })
        .err()
        .expect("prepare must reject operator-ui without the api role")
        .to_string();
        assert!(error.contains("no api role"), "unexpected error: {error}");
        assert!(
            !run_dir.exists(),
            "the rejection must happen before the run directory exists"
        );
    }

    /// Serve like the pool operator page: 200 on `/`, the observation DTO
    /// with the expected schema on its route, 404 elsewhere. The thread is
    /// deliberately detached: it exits with the test process, and joining
    /// it would deadlock (it loops on `incoming()`).
    fn operator_page_stub(listener: std::net::TcpListener, observation_body: Option<&'static str>) {
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut buffer = [0u8; 2048];
                let _ = std::io::Read::read(&mut stream, &mut buffer);
                let request = String::from_utf8_lossy(&buffer).to_string();
                let (status, body) = if request.starts_with("GET / HTTP") {
                    ("HTTP/1.1 200 OK", "<html>operator page</html>")
                } else if request.starts_with("GET /api/pool/observation") {
                    match observation_body {
                        Some(body) => ("HTTP/1.1 200 OK", body),
                        None => ("HTTP/1.1 404 Not Found", ""),
                    }
                } else {
                    ("HTTP/1.1 404 Not Found", "")
                };
                let _ = stream.write_all(
                    format!(
                        "{status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                );
            }
        });
    }

    #[test]
    fn operator_page_probe_proves_route_and_observation_probe_is_informational() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        operator_page_stub(listener, Some("{\"schema\":\"spc-pool-observation/v1\"}"));
        let root = format!("http://{addr}/");
        assert!(operator_page_served(&root));
        assert_eq!(
            operator_observation_route_present(&root),
            Some(true),
            "the stub serves the expected schema"
        );
    }

    #[test]
    fn operator_observation_probe_reports_missing_route_and_wrong_schema() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        operator_page_stub(listener, None);
        let root = format!("http://{addr}/");
        assert!(operator_page_served(&root));
        assert_eq!(operator_observation_route_present(&root), None);

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        operator_page_stub(listener, Some("{\"schema\":\"other/v9\"}"));
        let root = format!("http://{addr}/");
        assert!(operator_page_served(&root));
        assert_eq!(operator_observation_route_present(&root), Some(false));
    }

    #[test]
    fn operator_page_probe_fails_closed_on_a_refused_connection() {
        // Bounded retry: the full 20 x 500 ms window must end in `false`
        // when nothing answers; no URL would be published for this front.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        assert!(!operator_page_served(&format!("http://{addr}/")));
    }

    // ---- Runtime declared-bootstrap phase regression ----
    //
    // RUNTIME-BOOTSTRAP-BLOCKER: the declared `bootstrap_fund` phase command
    // is durably asynchronous - the controller returns `Running` and the
    // bootstrap worker owns the durable terminal. These tests exercise the
    // ACTUAL `Runtime::execute_phases` path with a scripted actor peer (the
    // real control wire contract) and a test-only bootstrap executor; the
    // native terminal itself is proven by the next owned Nix run, not here.
    use std::sync::mpsc;
    use std::sync::Mutex as StdMutex;

    use crate::command::traffic_audit::Session;
    use crate::command::{CommandOutcome, OutcomeState};
    use crate::traffic::control::{RpcGate, TxDriverControl};
    use crate::traffic::runtime::TrafficStage;
    use spc_lab_txd::bootstrap::{
        BootstrapCompletion, BootstrapExecutor, BootstrapHost, BootstrapRequest, OperationControl,
    };

    const PHASE_BOOTSTRAP_ID: &str = "phase-bootstrap-original";
    const PHASE_AFTER_ID: &str = "phase-after-pause";

    /// What the fixture executor does once the real bootstrap worker hands it
    /// the durable request.
    enum FixtureTerminal {
        /// Write the test stand-in durable `Succeeded` terminal for the SAME
        /// operation id (the native worker writes this only after independent
        /// distribution verification), then block until released.
        SucceedThenBlock,
        /// Block until the operator cancels, then return through the real
        /// cancelled-worker path.
        BlockForCancel,
        /// Return an executor error immediately: the real worker records the
        /// durable `Failed` terminal.
        FailNow,
    }

    struct FixtureExecutor {
        log: Arc<crate::command::CommandLog>,
        terminal: FixtureTerminal,
        observed: StdMutex<mpsc::Sender<BootstrapRequest>>,
        release: StdMutex<mpsc::Receiver<()>>,
    }

    impl BootstrapExecutor for FixtureExecutor {
        fn execute(
            &self,
            request: BootstrapRequest,
            control: OperationControl,
            _host: Arc<dyn BootstrapHost>,
        ) -> Result<BootstrapCompletion, String> {
            self.observed.lock().unwrap().send(request.clone()).unwrap();
            match self.terminal {
                FixtureTerminal::FailNow => {
                    return Err("fixture bootstrap executor failed".into());
                }
                FixtureTerminal::SucceedThenBlock => {
                    // Test stand-in for the native worker's post-verification
                    // durable terminal: same operation id, no resubmit, no
                    // new id, no synthesized phase evidence.
                    let id = crate::ids::CommandId::trusted(request.command_id.clone());
                    let terminal = CommandOutcome::new(
                        id.clone(),
                        OutcomeState::Succeeded,
                        "bootstrap distribution independently verified; \
                         measured phase active",
                    );
                    self.log.finish(&id, &terminal).unwrap();
                }
                FixtureTerminal::BlockForCancel => {}
            }
            let _ = self.release.lock().unwrap().recv();
            if control.cancelled() {
                return Err("fixture bootstrap executor cancelled".into());
            }
            Err("fixture bootstrap executor released".into())
        }
    }

    /// A scripted actor peer answering the bootstrap window request over the
    /// real control protocol; killed when the fixture drops.
    struct ScriptedActor(std::process::Child);
    impl Drop for ScriptedActor {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn scripted_actor() -> ScriptedActor {
        let script = r#"
import json, sys
def send(value):
    sys.stdout.write(json.dumps(value) + "\n")
    sys.stdout.flush()
for line in sys.stdin:
    message = json.loads(line)
    if message.get("kind") == "request" and message.get("operation") == "bootstrap_window":
        send({"kind": "response", "protocol": message["protocol"],
              "incarnation": message["incarnation"], "driver_id": message["driver_id"],
              "request_id": message["request_id"], "command_id": message.get("command_id"),
              "result": {"status": "bootstrap_window_open"}})
"#;
        ScriptedActor(
            Command::new("/usr/bin/python3")
                .args(["-c", script])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap(),
        )
    }

    struct PhaseFixture {
        engine: Arc<RunEngine>,
        _actor: ScriptedActor,
        root: PathBuf,
        observed: mpsc::Receiver<BootstrapRequest>,
        release: mpsc::Sender<()>,
    }

    /// One declared `bootstrap_fund` phase (fixed original command id) plus
    /// one later phase command, exactly as the runtime contract requires.
    fn phase_fixture(terminal: FixtureTerminal) -> PhaseFixture {
        let mut scenario: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/scenario_v1_one_pool_two_miners.json"
        ))
        .unwrap();
        scenario["run"]["id"] = json!(crate::ids::CommandId::generate().to_string());
        scenario["expanded_access"] = json!(true);
        scenario["traffic"] = json!({
            "version": 1, "required_capability": "spc.node.regtest.mempool_rpc.v1",
            "protocol": {"control": "spc-lab-tx-control/v1", "history_version": 3},
            "bootstrap": {"enabled": true, "mining_mode": "native", "funding_blocks": 101,
                "distribution_outputs_per_wallet": 8, "excluded_from_statistics": true},
            "driver": {"actors": [{"id": "txd-0", "network_wallets": 8,
                "user_payment_wallets": 2, "hosts_bootstrap_funding": true}],
                "user_payment_wallets": 2, "history": "traffic/"},
            "rate": {"ordinary_tx_per_sec": 0.2, "pacing": "fixed_jitter"},
            "fees": {"rate_sats_vb": {"min": 1, "max": 20}, "max_fee_sats_per_tx": 100000},
            "sizes": {"classes_vb": [{"vb": 220, "mix": 1.0}], "max_vb": 1000},
            "spend_types": ["p2pkh"], "ark": {"enabled": false,
                "registration_before_mining": true, "chain_effect_budget_tx_per_sec": 0.0},
            "caps": {"max_inflight_broadcasts": 1, "max_unknown_retries": 3,
                "max_mempool_backlog_tx": 1000, "reorg_resubmit_max": 3},
            "pause_semantics": "no_broadcast_after_ack"
        });
        let node = scenario["nodes"][0]["id"].clone();
        scenario["phases"] = json!([
            {"id": "bootstrap", "commands": [{"command_id": PHASE_BOOTSTRAP_ID,
                "kind": {"type": "bootstrap_fund", "driver_id": "txd-0", "node_id": node}}]},
            {"id": "after", "commands": [{"command_id": PHASE_AFTER_ID,
                "kind": {"type": "bootstrap_control", "operation_id": PHASE_BOOTSTRAP_ID,
                    "action": "pause"}}]}
        ]);
        let scenario: Scenario = serde_json::from_value(scenario).unwrap();
        let adapter: AdapterConfig = serde_json::from_value(serde_json::json!({
            "program": "/bin/true", "args": [], "fixture": true,
            "capabilities": ["fixture.scripted", "spc.node.regtest.mempool_rpc.v1",
                "spc.pool.work_unit_v2", "spc.pool.network_block_work"],
            "effective_profile": {
                "implementation": "fixture-node", "network": "regtest",
                "consensus_profile_id": "fixture.regtest.retarget", "profile_fingerprint": null,
                "pow_limit_bits": "207fffff", "retarget_floor_bits": "1e7fffff",
                "initial_target_bits": "1e7fffff", "regtest_pow_bits_overlay": "1e7fffff",
                "target_spacing_secs": 10, "target_timespan_secs": 240,
                "retarget_interval_blocks": 24, "no_pow_retargeting": false,
                "allow_min_difficulty_blocks": false, "minimum_difficulty_time_gap_secs": null,
                "subsidy_initial_sats": "5000000000", "subsidy_halving_interval_blocks": 150,
                "coinbase_maturity_blocks": 100, "witness_commitment_supported": true,
                "tdp_session_cap": 8},
            "rpc_auth": null, "env": {}}))
        .unwrap();
        let mut config = LabConfig::default();
        config.nodes.insert("fixture-node".into(), adapter.clone());
        config.pools.insert("fixture-pool".into(), adapter.clone());
        config
            .miners
            .insert("fixture-miner".into(), adapter.clone());
        config.redis = Some(adapter.clone());
        config.postgres = Some(adapter.clone());
        let mut native = adapter;
        native.fixture = false;
        config.traffic_probe = Some(native.clone());
        config.tx_driver = Some(native);
        let root = PathBuf::from("/tmp/opencode").join(format!(
            "runtime-phase-{}",
            crate::ids::CommandId::generate()
        ));
        std::fs::create_dir(&root).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        let engine = RunEngine::prepare(
            scenario,
            EngineOptions {
                run_dir: root.clone(),
                config,
                base_revision: "runtime-phase-test".into(),
                dirty_tree_hash: "fixture".into(),
                owner_instance: "runtime-phase-test".into(),
                evidence_capacity: 1024,
                namespaces: BTreeMap::new(),
                nsenter: None,
                user_namespace: None,
                user_namespace_owner: None,
                namespace_owners: BTreeMap::new(),
                namespace_broker: None,
                lifetime_helper: None,
                addresses: BTreeMap::new(),
                link_backend: None,
                operator_ui: false,
            },
        )
        .unwrap();
        // The states `start` and `launch_tx_drivers` leave behind, without
        // launching any process: a started run and a provisioned traffic
        // stage (the stage the bootstrap declaration is admitted at).
        engine.mark_test_started();
        {
            let mut manifest = engine.manifest.lock().unwrap();
            manifest.traffic_stage = Some(TrafficStage::Provisioned);
            manifest.state = crate::manifest::RunState::Running;
            manifest.save(&engine.manifest_path).unwrap();
        }
        let mut actor = scripted_actor();
        let gate = RpcGate::new(BTreeMap::new(), 0)
            .with_audit(
                engine.command_log.clone(),
                Session {
                    run_id: engine.run_id.to_string(),
                    driver_id: "txd-0".into(),
                    incarnation: "phase-fixture".into(),
                    controller_sha256: crate::supervisor::hash_file(
                        &std::env::current_exe().unwrap(),
                    )
                    .unwrap(),
                    actor_sha256: crate::supervisor::hash_file(Path::new("/usr/bin/python3"))
                        .unwrap(),
                    acceptance_enabled: false,
                },
            )
            .unwrap();
        let control = TxDriverControl::spawn(
            ActorId::trusted("txd-0"),
            "phase-fixture".into(),
            actor.0.stdin.take().unwrap(),
            actor.0.stdout.take().unwrap(),
            gate,
        )
        .unwrap();
        engine.install_test_tx_driver(ActorId::trusted("txd-0"), Arc::new(control));
        let (observed_tx, observed_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        engine.set_bootstrap_executor(Arc::new(FixtureExecutor {
            log: engine.command_log.clone(),
            terminal,
            observed: StdMutex::new(observed_tx),
            release: StdMutex::new(release_rx),
        }));
        PhaseFixture {
            engine,
            _actor: actor,
            root,
            observed: observed_rx,
            release: release_tx,
        }
    }

    /// A `Runtime` whose engine is the phase fixture: no nodes, fronts or
    /// observers, so `execute_phases` observes only the phase outcomes.
    fn phase_runtime_shell(fixture: &PhaseFixture) -> Runtime {
        Runtime {
            engine: fixture.engine.clone(),
            run_dir: fixture.root.clone(),
            binaries: RuntimeBinaries {
                txd: None,
                traffic_probe: None,
                traffic_probe_args: Vec::new(),
                node: PathBuf::new(),
                pool: PathBuf::new(),
                miner: PathBuf::new(),
                scanner: None,
                postgres_bin_dir: PathBuf::new(),
                redis_server: PathBuf::new(),
                redis_cli: PathBuf::new(),
            },
            identities: Vec::new(),
            nodes: Vec::new(),
            pools: Vec::new(),
            observers: None,
            topology: None,
        }
    }

    fn command_journal(fixture: &PhaseFixture) -> Vec<Value> {
        std::fs::read_to_string(fixture.root.join("private").join("commands.jsonl"))
            .unwrap()
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn begins_for<'a>(journal: &'a [Value], command_id: &str) -> Vec<&'a Value> {
        journal
            .iter()
            .filter(|line| {
                line["record"].as_str() == Some("begin")
                    && line["command"]["command_id"].as_str() == Some(command_id)
            })
            .collect()
    }

    fn hash_rows_for<'a>(journal: &'a [Value], record: &str, operation_id: &str) -> Vec<&'a Value> {
        journal
            .iter()
            .filter(|line| {
                line["record"].as_str() == Some(record)
                    && (line["operation"]["operation_id"].as_str() == Some(operation_id)
                        || line["operation_id"].as_str() == Some(operation_id))
            })
            .collect()
    }

    fn phase_terminal_report(fixture: &PhaseFixture) -> Value {
        serde_json::from_str(
            &std::fs::read_to_string(fixture.root.join("shareable").join("topology.json")).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn declared_bootstrap_running_waits_for_its_terminal_and_phases_continue() {
        let fixture = phase_fixture(FixtureTerminal::SucceedThenBlock);
        let runtime = phase_runtime_shell(&fixture);
        // The actual Runtime phase path: without the wait on the SAME
        // operation id, this fails immediately on the `Running` outcome.
        let report = runtime.execute_phases(None).unwrap();

        let phases = report["phases"].as_array().unwrap();
        assert_eq!(phases.len(), 2, "both declared phases executed once");
        assert_eq!(phases[0]["phase"], "bootstrap");
        assert_eq!(phases[0]["outcome"]["state"], "succeeded");
        assert_eq!(phases[0]["outcome"]["command_id"], PHASE_BOOTSTRAP_ID);
        assert_eq!(
            phases[0]["outcome"]["detail"]
                .as_str()
                .unwrap()
                .replace(' ', ""),
            "bootstrapdistributionindependentlyverified;measuredphaseactive"
        );
        assert_eq!(phases[1]["phase"], "after");
        assert_eq!(phases[1]["outcome"]["state"], "succeeded");
        assert_eq!(phases[1]["outcome"]["command_id"], PHASE_AFTER_ID);

        let journal = command_journal(&fixture);
        // Exactly one controller-principal Begin for the original bootstrap:
        // the wait must not resubmit or bind another principal/id.
        let begins = begins_for(&journal, PHASE_BOOTSTRAP_ID);
        assert_eq!(begins.len(), 1, "exactly one Begin for the bootstrap");
        assert_eq!(begins[0]["principal"], json!("controller"));
        // Sole hash budget: one HashStart, no reservation and no deadline
        // reset added by the wait.
        assert_eq!(
            hash_rows_for(&journal, "hash_start", PHASE_BOOTSTRAP_ID).len(),
            1
        );
        assert!(hash_rows_for(&journal, "hash_reserve", PHASE_BOOTSTRAP_ID).is_empty());
        let request = fixture
            .observed
            .recv_timeout(Duration::from_secs(10))
            .unwrap();
        let budget = fixture
            .engine
            .command_log
            .hash_budget_snapshot(&crate::ids::CommandId::trusted(PHASE_BOOTSTRAP_ID))
            .unwrap();
        assert_eq!(
            budget.operation.deadline_monotonic_ms,
            request.deadline_monotonic_ms
        );
        assert_eq!(budget.operation.boot_id, request.boot_id);
        assert!(budget.outstanding.is_none());
        // The durable terminal belongs to the same operation id.
        let terminal = fixture
            .engine
            .command_log
            .outcome(&crate::ids::CommandId::trusted(PHASE_BOOTSTRAP_ID))
            .unwrap();
        assert_eq!(terminal.state, OutcomeState::Succeeded);
        assert_eq!(terminal.command_id.as_str(), PHASE_BOOTSTRAP_ID);
    }

    #[test]
    fn operator_stop_during_the_bootstrap_wait_cancels_and_does_not_advance_phases() {
        let fixture = phase_fixture(FixtureTerminal::BlockForCancel);
        let runtime = phase_runtime_shell(&fixture);
        let engine = fixture.engine.clone();
        let (done_tx, done_rx) = mpsc::channel();
        let phases = thread::spawn(move || {
            let _ = done_tx.send(runtime.execute_phases(None));
        });
        // The durable bootstrap request is in flight.
        fixture
            .observed
            .recv_timeout(Duration::from_secs(10))
            .unwrap();
        // Operator stop while the phase path waits: no engine mutex is held
        // across the wait, so the stop must stay responsive.
        let stop_engine = engine.clone();
        let stop = thread::spawn(move || {
            stop_engine.submit(LabCommand::new(
                stop_engine.run_id.clone(),
                ActorId::trusted("operator-stop"),
                CommandKind::StopRun {
                    reason: "operator stop during bootstrap".to_string(),
                },
            ))
        });
        let outcome = stop.join().unwrap();
        assert_ne!(outcome.state, OutcomeState::Unknown);
        // The stop cancelled the original operation: release the fixture
        // worker so the real bootstrap worker records its durable terminal.
        fixture.release.send(()).unwrap();
        let result = done_rx.recv_timeout(Duration::from_secs(30)).unwrap();
        assert!(
            result.is_err(),
            "a cancelled bootstrap is an honest phase error: {result:?}"
        );

        let journal = command_journal(&fixture);
        assert!(
            begins_for(&journal, PHASE_AFTER_ID).is_empty(),
            "no later phase command may begin after a cancelled bootstrap"
        );
        let terminal = fixture
            .engine
            .command_log
            .outcome(&crate::ids::CommandId::trusted(PHASE_BOOTSTRAP_ID))
            .unwrap();
        assert_eq!(terminal.state, OutcomeState::Cancelled);
        assert_eq!(terminal.command_id.as_str(), PHASE_BOOTSTRAP_ID);
        assert_eq!(terminal.detail, "bootstrap cancelled");
        // The terminal phase evidence records the SAME operation result.
        let report = phase_terminal_report(&fixture);
        assert_eq!(report["phase"], "bootstrap");
        assert_eq!(report["outcome"]["state"], "cancelled");
        assert_eq!(report["outcome"]["command_id"], PHASE_BOOTSTRAP_ID);
        let _ = phases.join();
    }

    #[test]
    fn failed_bootstrap_terminal_does_not_advance_phases() {
        let fixture = phase_fixture(FixtureTerminal::FailNow);
        let runtime = phase_runtime_shell(&fixture);
        assert!(runtime.execute_phases(None).is_err());

        let journal = command_journal(&fixture);
        assert!(
            begins_for(&journal, PHASE_AFTER_ID).is_empty(),
            "no later phase command may begin after a failed bootstrap"
        );
        let terminal = fixture
            .engine
            .command_log
            .outcome(&crate::ids::CommandId::trusted(PHASE_BOOTSTRAP_ID))
            .unwrap();
        assert_eq!(terminal.state, OutcomeState::Failed);
        assert_eq!(terminal.command_id.as_str(), PHASE_BOOTSTRAP_ID);
        let report = phase_terminal_report(&fixture);
        assert_eq!(report["phase"], "bootstrap");
        assert_eq!(report["outcome"]["state"], "failed");
        assert_eq!(report["outcome"]["command_id"], PHASE_BOOTSTRAP_ID);
    }
}
