//! One supervised run path.
//!
//! The engine turns a validated scenario into isolated processes, routes typed
//! commands through the durable log, records evidence, and writes the same
//! snapshot/SSE view used by the CLI, tests and the thin browser.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::access::{AccessStore, AuthContext, UserProjection, PRINCIPAL_CONTROLLER};
use crate::adapters::{
    FixtureNodeControl, MinerControl, MinerOperation, MinerWireResponse, NodeControl,
    RpcNodeControl,
};
use crate::artifacts::{
    decimate_timeline, ArtifactPaths, ArtifactWriter, AssertionSummary, CommandSummary,
    ResourceSummary, ScenarioSummary, ShareableManifest, WorkProfileSummary,
};
use crate::command::{
    BeginOutcome, CommandKind, CommandLog, CommandOutcome, LabCommand, OutcomeState,
};
use crate::config::{
    expand_template, preflight, validate_effective_profiles, AdapterConfig, LabConfig,
    PreflightReport,
};
use crate::error::EngineError;
use crate::evidence::{
    now_ms as evidence_now_ms, ActorKind, AssertionEvaluation, CanonicalAt, CanonicalResult,
    CommandObservation, EntitySnapshot, EvidenceActor, EvidencePayload, EvidenceStore, Gap,
    GapScope, JobActivation, ProcessObservation, Snapshot,
};
use crate::ids::{FrontId, IdentityId, LinkId, MinerId, NodeId, PoolId, RunId, SourceId, WorkerId};
use crate::links::{backend_for, LinkBackend};
use crate::manifest::{now_ms as manifest_now_ms, RunLock, RunManifest, RunState};
use crate::resource::{plan_resources, Endpoint, PlannedResources};
use crate::scenario::{
    BudgetSpec, ConnectionSpec, LinkPolicy, NodeSpec, Scenario, WorkProfileSpec, WorkerSpec,
};
use crate::secrets::{PoolAuthority, PublicAuthority};
use crate::supervisor::{
    hash_file, ControlTransport, Launched, ProcessRecord, ProcessSpec, ProcessState, Readiness,
    Supervisor,
};
use crate::traffic::check_traffic_capability;
use crate::wide::{ReferenceProfile, WideU128};

/// A runtime integration hook invoked after storage and node processes are
/// ready, before fronts/pools start. The built-in real runtime uses it to
/// create Postgres databases and render the actual pool TOML.
pub trait StartupHook: Send + Sync {
    fn after_storage_and_nodes(&self, engine: &RunEngine) -> Result<(), String>;
    fn after_fronts(&self, _engine: &RunEngine) -> Result<(), String> {
        Ok(())
    }
}

/// Called before a miner process is spawned so the built-in runtime can write
/// the strict per-miner TOML (including for miners added after start). The call
/// must be idempotent: an existing identity/config is never regenerated.
pub trait MinerProvisioner: Send + Sync {
    fn prepare_miner(
        &self,
        engine: &RunEngine,
        miner: &MinerId,
        implementation: &str,
    ) -> Result<(), String>;
}

#[derive(Debug, Clone)]
pub struct EngineOptions {
    pub run_dir: PathBuf,
    pub config: LabConfig,
    pub base_revision: String,
    pub dirty_tree_hash: String,
    pub owner_instance: String,
    pub evidence_capacity: usize,
    /// `role/entity` -> the network namespace the actor is placed in.
    pub namespaces: BTreeMap<String, String>,
    /// The `nsenter` binary used to join a namespace.
    pub nsenter: Option<PathBuf>,
    /// The broker user namespace shared by all isolated actors.
    pub user_namespace: Option<PathBuf>,
    /// Start-time identity of the broker user-namespace owner.
    pub user_namespace_owner: Option<crate::supervisor::NamespaceOwner>,
    /// Start-time identities of the per-endpoint network-namespace holders.
    pub namespace_owners: BTreeMap<String, crate::supervisor::NamespaceOwner>,
    /// Broker used for namespace-scoped node RPC during actor startup.
    pub namespace_broker: Option<Arc<crate::namespace::NamespaceBroker>>,
    /// Safe parent-death exec helper used by the real runtime for every actor.
    pub lifetime_helper: Option<PathBuf>,
    /// The published addresses, keyed by `<kind>.<entity>.<name>`.
    pub addresses: BTreeMap<String, String>,
    /// The link backend, when the run has one the runtime built.
    pub link_backend: Option<Arc<dyn LinkBackend>>,
    /// Plan one extra loopback listener per front (`front.<id>.operator`)
    /// for the EXP-OP operator page. Set only by the host runtime's
    /// `--operator-ui`; the listener is recorded like any other so
    /// attach/adopt keeps it.
    pub operator_ui: bool,
}

#[derive(Debug, Clone)]
struct ExtraMiner {
    implementation: String,
    connections: Vec<ConnectionSpec>,
    workers: Vec<WorkerSpec>,
}

#[derive(Default)]
struct EngineState {
    started: bool,
    paused_miners: BTreeSet<MinerId>,
    paused_workers: BTreeSet<WorkerId>,
    removed: BTreeSet<String>,
    extra_miners: BTreeMap<MinerId, ExtraMiner>,
    worker_budgets: BTreeMap<WorkerId, BudgetSpec>,
    driver_dropped: BTreeMap<String, u64>,
}

struct RuntimeAdapters {
    rpc_nodes: Mutex<BTreeMap<NodeId, Arc<RpcNodeControl>>>,
    nodes: Mutex<BTreeMap<NodeId, Box<dyn NodeControl>>>,
    miners: Mutex<BTreeMap<MinerId, Arc<MinerControl>>>,
}

pub struct RunEngine {
    traffic: Mutex<traffic_runtime::TrafficRuntime>,
    pub run_id: RunId,
    pub scenario: Scenario,
    pub run_dir: PathBuf,
    pub manifest_path: PathBuf,
    pub manifest: Mutex<RunManifest>,
    pub command_log: Arc<CommandLog>,
    pub evidence: Arc<EvidenceStore>,
    pub artifacts: ArtifactWriter,
    pub supervisor: Mutex<Supervisor>,
    adapters: RuntimeAdapters,
    state: Mutex<EngineState>,
    link_backend: Arc<dyn LinkBackend>,
    namespaces: BTreeMap<String, String>,
    nsenter: Option<PathBuf>,
    user_namespace: Option<PathBuf>,
    user_namespace_owner: Option<crate::supervisor::NamespaceOwner>,
    namespace_owners: BTreeMap<String, crate::supervisor::NamespaceOwner>,
    namespace_broker: Option<Arc<crate::namespace::NamespaceBroker>>,
    lifetime_helper: Option<PathBuf>,
    addresses: BTreeMap<String, String>,
    config: LabConfig,
    preflight: PreflightReport,
    lock: Mutex<Option<RunLock>>,
    resources: PlannedResources,
    authorities: BTreeMap<PoolId, PoolAuthority>,
    read_only: bool,
    /// Expanded-access claim store. `None` when the scenario did not opt in:
    /// the unauthenticated loopback surface is then byte-for-byte unchanged.
    access: Option<AccessStore>,
    self_ref: Mutex<Weak<RunEngine>>,
    command_mutex: Mutex<()>,
    startup_hook: Mutex<Option<Arc<dyn StartupHook>>>,
    miner_provisioner: Mutex<Option<Arc<dyn MinerProvisioner>>>,
    pub wallet_registry: Mutex<crate::wallet_projection::WalletRegistry>,
}

mod native_audit;
mod traffic_runtime;

/// Why a run directory is reopened. Live adoption must prove every
/// recorded process is still the launched one; cleanup only needs the
/// persisted records to be consistent with the launch profiles.
#[derive(Clone, Copy, PartialEq, Eq)]
enum AttachMode {
    /// Adopt live processes: strict traffic reverify, process adoption,
    /// command reconciliation, state rebuild.
    Live,
    /// Read-only review: no lock, no adoption, no effects.
    ReadOnly,
    /// Cleanup: lock, persisted-binding validation, census loaded
    /// verbatim so stopped or missing processes can still be reaped.
    Cleanup,
}

impl RunEngine {
    pub fn prepare(scenario: Scenario, options: EngineOptions) -> Result<Arc<Self>, EngineError> {
        let preflight = preflight(&scenario, &options.config)
            .map_err(|e| EngineError::new(e.code(), e.detail()))?;
        // Declared traffic requires configured native adapters before any
        // filesystem effect. Its full capability/profile gate runs after
        // nodes and storage, before traffic-bearing processes are launched.
        traffic_runtime::prepare_traffic(&scenario, &options.config)?;
        // Expanded-access binding derivation happens before any filesystem,
        // lock or authority effect: a scenario whose identities do not map
        // one-to-one onto miners (with workers and a single pool) is rejected
        // closed here. The per-miner single-identity rule is load-bearing -
        // a PauseWorkers with an empty worker list pauses the whole miner.
        if scenario.expanded_access {
            crate::access::derive_user_bindings(&scenario)
                .map_err(|detail| EngineError::new("access_bindings", detail))?;
        }
        let run_id = scenario.run.id.clone();
        let run_dir = options.run_dir.clone();
        if run_dir.join("run-manifest.json").exists() {
            return Err(EngineError::new(
                "run_dir_in_use",
                format!(
                    "{} already contains a run manifest; use attach/adopt instead of preparing a duplicate",
                    run_dir.display()
                ),
            ));
        }
        std::fs::create_dir_all(&run_dir)
            .map_err(|e| EngineError::new("run_dir_create", e.to_string()))?;
        let artifacts = ArtifactWriter::new(ArtifactPaths::under(&run_dir))
            .map_err(|e| EngineError::new("artifacts", e.to_string()))?;
        let resources = plan_resources(&scenario, &run_id, &run_dir, options.operator_ui)
            .map_err(|e| EngineError::new("resource_allocation", e.to_string()))?;
        let mut authorities = BTreeMap::new();
        for pool in &scenario.pools {
            let authority = PoolAuthority::generate(&pool.id, &run_dir)
                .map_err(|e| EngineError::new("authority_generation", e.to_string()))?;
            authorities.insert(pool.id.clone(), authority);
        }
        let lock = RunLock::acquire(
            run_dir.join("locks").join("owner.lock"),
            &run_id,
            &options.owner_instance,
        )
        .map_err(|e| EngineError::new("run_lock", e.to_string()))?;
        let scenario_json = serde_json::to_vec_pretty(&scenario)
            .map_err(|e| EngineError::new("scenario_encode", e.to_string()))?;
        artifacts
            .write_private("scenario.json", &scenario)
            .map_err(|e| EngineError::new("scenario_write", e.to_string()))?;
        artifacts
            .write_private("runtime.json", &options.config)
            .map_err(|e| EngineError::new("runtime_write", e.to_string()))?;

        let mut binary_hashes = BTreeMap::new();
        collect_binaries(&options.config, &mut binary_hashes);
        let manifest = RunManifest {
            traffic_preflight_state: scenario.traffic.as_ref().map(|_| "pending".into()),
            traffic_stage: None,
            traffic_launch_profiles: BTreeMap::new(),
            traffic_driver_paths: BTreeMap::new(),
            traffic_measured_from: None,
            traffic_lab_acceptance: scenario.traffic.as_ref().is_some_and(|t| t.lab_acceptance),
            placement: None,
            traffic_capability: None,
            schema_version: crate::manifest::MANIFEST_SCHEMA_VERSION,
            run_id: run_id.clone(),
            owner_instance: options.owner_instance.clone(),
            created_at: manifest_now_ms(),
            updated_at: manifest_now_ms(),
            base_revision: options.base_revision,
            dirty_tree_hash: options.dirty_tree_hash,
            scenario_sha256: hex::encode(<sha2::Sha256 as sha2::Digest>::digest(&scenario_json)),
            binary_paths_and_hashes: binary_hashes,
            resources: crate::manifest::ResourceManifest {
                run_dir: run_dir.clone(),
                listeners: resources.listeners.clone(),
                redis_instances: resources.redis_instances.clone(),
                postgres_databases: resources.postgres_databases.clone(),
                temp_dirs: resources.temp_dirs.clone(),
            },
            effective_profiles: preflight.effective_profiles.clone(),
            processes: Vec::new(),
            locks: lock.record(),
            state: RunState::Created,
            artifact_paths: artifacts.paths().clone(),
            warnings: {
                let mut warnings = preflight.warnings.clone();
                warnings.push(
                    "allocated ports are held by kernel-backed per-port locks for the run lifetime; the run manifest records the requested/actual endpoints, while an independent run must still allocate its own ports"
                        .to_string(),
                );
                warnings
            },
        };
        manifest
            .save(run_dir.join("run-manifest.json"))
            .map_err(|e| EngineError::new("manifest_write", e.to_string()))?;
        let evidence = Arc::new(
            EvidenceStore::open(
                run_id.clone(),
                &artifacts.paths().private_evidence,
                options.evidence_capacity.max(1),
            )
            .map_err(|e| EngineError::new("evidence_open", e.to_string()))?,
        );
        let command_log = Arc::new(
            (if scenario.traffic.is_some() {
                CommandLog::open_traffic(&artifacts.paths().private_commands)
            } else {
                CommandLog::open(&artifacts.paths().private_commands)
            })
            .map_err(|e| EngineError::new("command_log", e.to_string()))?,
        );
        let access = if scenario.expanded_access {
            Some(
                AccessStore::prepare(&run_id, &run_dir, &scenario)
                    .map_err(|detail| EngineError::new("access_claims", detail))?,
            )
        } else {
            None
        };
        let run_token = format!("{}:{}", run_id, options.owner_instance);
        let run_token = if scenario.traffic.is_some() {
            hex::encode(<sha2::Sha256 as sha2::Digest>::digest(run_token.as_bytes()))
        } else {
            run_token
        };
        let engine = Arc::new(Self {
            run_id,
            scenario,
            run_dir: run_dir.clone(),
            manifest_path: run_dir.join("run-manifest.json"),
            manifest: Mutex::new(manifest),
            command_log,
            evidence,
            artifacts,
            supervisor: Mutex::new(Supervisor::new(run_token)),
            traffic: Mutex::new(traffic_runtime::TrafficRuntime::default()),
            adapters: RuntimeAdapters {
                nodes: Mutex::new(BTreeMap::new()),
                rpc_nodes: Mutex::new(BTreeMap::new()),
                miners: Mutex::new(BTreeMap::new()),
            },
            state: Mutex::new(EngineState::default()),
            link_backend: options
                .link_backend
                .clone()
                .unwrap_or_else(|| Arc::from(backend_for(options.config.link_backend.as_deref()))),
            namespaces: options.namespaces.clone(),
            nsenter: options.nsenter.clone(),
            user_namespace: options.user_namespace.clone(),
            user_namespace_owner: options.user_namespace_owner.clone(),
            namespace_owners: options.namespace_owners.clone(),
            namespace_broker: options.namespace_broker.clone(),
            lifetime_helper: options.lifetime_helper.clone(),
            addresses: options.addresses.clone(),
            config: options.config,
            preflight,
            lock: Mutex::new(Some(lock)),
            resources,
            authorities,
            read_only: false,
            access,
            self_ref: Mutex::new(Weak::new()),
            command_mutex: Mutex::new(()),
            startup_hook: Mutex::new(None),
            miner_provisioner: Mutex::new(None),
            wallet_registry: Mutex::new(crate::wallet_projection::WalletRegistry::default()),
        });
        *engine.self_ref.lock().unwrap() = Arc::downgrade(&engine);
        Ok(engine)
    }

    /// Reopen a run directory for adoption or serve.
    pub fn attach(
        run_dir: impl AsRef<Path>,
        owner_instance: &str,
    ) -> Result<Arc<Self>, EngineError> {
        Self::attach_internal(run_dir, owner_instance, AttachMode::Live)
    }

    /// Reopen a run directory without adopting processes or reconciling
    /// commands. Use this for snapshot/event/manifest review.
    pub fn attach_read_only(
        run_dir: impl AsRef<Path>,
        owner_instance: &str,
    ) -> Result<Arc<Self>, EngineError> {
        Self::attach_internal(run_dir, owner_instance, AttachMode::ReadOnly)
    }

    /// Reopen a run directory for cleanup (`reap`). Takes the owner lock and
    /// validates that the persisted node argv/binary bindings still match
    /// the recorded launch profiles, then loads the recorded process census
    /// verbatim: `reap` stops what is actually ours and reports
    /// expected-but-absent nodes as exited, without the live-process proof a
    /// live adopt demands. A run stopped before this call attaches cleanly.
    /// A traffic run whose capability projection was never recorded (the
    /// probe stopped before the grant) attaches too: cleanup never grants or
    /// exercises a capability, while a recorded-but-invalid projection still
    /// rejects. A native-audit run whose wallet failed before any lease was
    /// published attaches too: cleanup never requires a live wallet,
    /// instance or lease, while a present-but-foreign or rewritten lease
    /// projection still rejects
    /// (`validate_persisted_native_audit_bindings`).
    pub fn attach_for_cleanup(
        run_dir: impl AsRef<Path>,
        owner_instance: &str,
    ) -> Result<Arc<Self>, EngineError> {
        Self::attach_internal(run_dir, owner_instance, AttachMode::Cleanup)
    }

    fn attach_internal(
        run_dir: impl AsRef<Path>,
        owner_instance: &str,
        mode: AttachMode,
    ) -> Result<Arc<Self>, EngineError> {
        let run_dir = run_dir.as_ref().to_path_buf();
        let manifest_path = run_dir.join("run-manifest.json");
        let manifest = RunManifest::load(&manifest_path)
            .map_err(|e| EngineError::new("manifest_load", e.to_string()))?;
        let scenario_path = manifest.artifact_paths.private_scenario.clone();
        let scenario_json = std::fs::read_to_string(scenario_path)
            .map_err(|e| EngineError::new("scenario_load", e.to_string()))?;
        let scenario = Scenario::from_json(&scenario_json)
            .map_err(|e| EngineError::new(e.code(), e.detail()))?;
        let runtime_path = manifest.artifact_paths.private_runtime.clone();
        let config_json = std::fs::read_to_string(runtime_path)
            .map_err(|e| EngineError::new("runtime_load", e.to_string()))?;
        let config = LabConfig::from_json(&config_json)
            .map_err(|e| EngineError::new(e.code(), e.detail()))?;
        let preflight = match mode {
            AttachMode::ReadOnly => PreflightReport::default(),
            AttachMode::Live => {
                let preflight = preflight(&scenario, &config)
                    .map_err(|e| EngineError::new(e.code(), e.detail()))?;
                // Traffic gate (plan Section 14, T0) on the adopt path,
                // BEFORE the owner lock is taken: a traffic-bearing scenario
                // whose recorded projection is missing, unapproved,
                // tampered or unrecognized rejects here without any lock,
                // filesystem or authority effect - exactly as at prepare.
                check_traffic_capability(&scenario, manifest.traffic_capability.as_ref())
                    .map_err(|e| EngineError::new(e.code(), e.detail()))?;
                preflight
            }
            // Cleanup never grants or exercises a traffic capability, and it
            // never enables commands, hashing or resume: it validates the
            // persisted manifest/config and only stops what the recorded
            // census actually identifies as ours. A run that stopped before
            // the probe granted the capability (FAILED/INCOMPLETE
            // provisioning) has no record to verify - that absence is not a
            // persisted contradiction and must not block resource cleanup.
            // A record that IS present is verified exactly as strictly as
            // Live, so cleanup still refuses rewritten history.
            AttachMode::Cleanup => {
                let preflight = preflight(&scenario, &config)
                    .map_err(|e| EngineError::new(e.code(), e.detail()))?;
                if let Some(record) = manifest.traffic_capability.as_ref() {
                    check_traffic_capability(&scenario, Some(record))
                        .map_err(|e| EngineError::new(e.code(), e.detail()))?;
                }
                preflight
            }
        };
        let attached_lock = if mode != AttachMode::ReadOnly {
            Some(
                RunLock::acquire(
                    run_dir.join("locks").join("owner.lock"),
                    &manifest.run_id,
                    owner_instance,
                )
                .map_err(|e| EngineError::new("run_lock", e.to_string()))?,
            )
        } else {
            None
        };
        let artifacts = ArtifactWriter::new(manifest.artifact_paths.clone())
            .map_err(|e| EngineError::new("artifacts", e.to_string()))?;
        let evidence = Arc::new(
            EvidenceStore::load(
                manifest.run_id.clone(),
                &manifest.artifact_paths.private_evidence,
                100_000,
            )
            .map_err(|e| EngineError::new("evidence_load", e.to_string()))?,
        );
        let command_log = Arc::new(
            (if scenario.traffic.is_some() {
                CommandLog::open_traffic(&manifest.artifact_paths.private_commands)
            } else {
                CommandLog::open(&manifest.artifact_paths.private_commands)
            })
            .map_err(|e| EngineError::new("command_log", e.to_string()))?,
        );
        let access = if scenario.expanded_access {
            Some(
                AccessStore::attach(run_dir.join("private").join("access"))
                    .map_err(|detail| EngineError::new("access_claims", detail))?,
            )
        } else {
            None
        };
        let run_token = manifest
            .processes
            .first()
            .map(|record| record.run_token.clone())
            .unwrap_or_else(|| format!("{}:{owner_instance}", manifest.run_id));
        let resources = PlannedResources {
            listeners: manifest.resources.listeners.clone(),
            redis_instances: manifest.resources.redis_instances.clone(),
            postgres_databases: manifest.resources.postgres_databases.clone(),
            temp_dirs: manifest.resources.temp_dirs.clone(),
            port_locks: Vec::new(),
        };
        let mut node_controls: BTreeMap<NodeId, Box<dyn NodeControl>> = BTreeMap::new();
        let mut rpc_nodes = BTreeMap::new();
        for node in &scenario.nodes {
            let profile = manifest.effective_profiles.get(&node.id).cloned();
            let maybe_adapter = config.node(&node.implementation);
            let fixture = maybe_adapter.map(|a| a.fixture).unwrap_or(true);
            let control: Box<dyn NodeControl> = if fixture {
                Box::new(FixtureNodeControl::new(
                    node.id.clone(),
                    profile.unwrap_or_default(),
                    run_dir.join(format!("height-{}.txt", node.id)),
                ))
            } else if let Some(endpoint) = manifest
                .resources
                .listeners
                .get(&format!("node.{}.rpc", node.id))
            {
                let rpc = Arc::new(RpcNodeControl::new(
                    node.id.clone(),
                    endpoint.clone(),
                    maybe_adapter.and_then(|a| a.rpc_auth.clone()),
                ));
                rpc_nodes.insert(node.id.clone(), Arc::clone(&rpc));
                Box::new(rpc)
            } else {
                continue;
            };
            node_controls.insert(node.id.clone(), control);
        }
        let adapters = RuntimeAdapters {
            // Adoption must re-bind topology/profile before enabling this lane.
            rpc_nodes: Mutex::new(rpc_nodes),
            nodes: Mutex::new(node_controls),
            miners: Mutex::new(BTreeMap::new()),
        };
        let engine = Arc::new(Self {
            run_id: manifest.run_id.clone(),
            scenario,
            run_dir,
            manifest_path,
            manifest: Mutex::new(manifest),
            command_log,
            evidence,
            artifacts,
            supervisor: Mutex::new(Supervisor::new(run_token)),
            traffic: Mutex::new(traffic_runtime::TrafficRuntime::default()),
            adapters,
            state: Mutex::new(EngineState {
                started: false,
                ..EngineState::default()
            }),
            link_backend: Arc::from(backend_for(config.link_backend.as_deref())),
            namespaces: BTreeMap::new(),
            nsenter: None,
            user_namespace: None,
            user_namespace_owner: None,
            namespace_owners: BTreeMap::new(),
            namespace_broker: None,
            lifetime_helper: None,
            addresses: BTreeMap::new(),
            config,
            preflight,
            lock: Mutex::new(attached_lock),
            resources,
            authorities: BTreeMap::new(),
            read_only: mode == AttachMode::ReadOnly,
            access,
            self_ref: Mutex::new(Weak::new()),
            command_mutex: Mutex::new(()),
            startup_hook: Mutex::new(None),
            miner_provisioner: Mutex::new(None),
            wallet_registry: Mutex::new(crate::wallet_projection::WalletRegistry::default()),
        });
        *engine.self_ref.lock().unwrap() = Arc::downgrade(&engine);
        match mode {
            AttachMode::Live => {
                engine.reverify_traffic_binding()?;
                engine.adopt_processes()?;
                engine.validate_native_audit_attach()?;
                engine.reconcile_commands()?;
                engine.rebuild_state_from_log();
            }
            AttachMode::Cleanup => {
                engine.validate_persisted_traffic_bindings()?;
                engine.validate_persisted_native_audit_bindings()?;
                let records = engine.manifest.lock().unwrap().processes.clone();
                engine.supervisor.lock().unwrap().load_census(records);
            }
            AttachMode::ReadOnly => {}
        }
        // Native-audit attach validation is factored by the same purpose:
        // the Live arm above demands the live wallet socket, instance and
        // ownership proof (`validate_native_audit_attach`), while Cleanup
        // validates only the persisted lease projection
        // (`validate_persisted_native_audit_bindings`) and never requires
        // a wallet, lease or instance that a partial startup never
        // created. Read-only never validates (unchanged).
        // The wallet-service direction removed the UI-source
        // reconstruction on attach (`rebind_ui_registry`): attach never
        // fabricates wallet bindings; the strict persisted-binding
        // validation runs in the launch path
        // (`wallet_aggregate_binding::validate_persisted`).
        Ok(engine)
    }

    pub fn set_startup_hook(&self, hook: Arc<dyn StartupHook>) {
        *self.startup_hook.lock().unwrap() = Some(hook);
    }

    pub fn set_miner_provisioner(&self, provisioner: Arc<dyn MinerProvisioner>) {
        *self.miner_provisioner.lock().unwrap() = Some(provisioner);
    }

    pub fn scenario(&self) -> &Scenario {
        &self.scenario
    }

    /// Public catalog for the browser add-miner form. It never includes
    /// identity secret material.
    pub fn runtime_catalog(&self) -> serde_json::Value {
        let mut catalog = crate::runtime::catalog(self);
        if self.scenario.expanded_access {
            catalog["traffic_controls"] = self.t1_control_catalog();
        }
        catalog
    }

    /// Every live miner (declared or added) with its current connection and
    /// worker specs. Used only by the public runtime catalog.
    pub fn runtime_miner_specs(
        &self,
    ) -> Vec<(
        MinerId,
        String,
        Vec<crate::scenario::ConnectionSpec>,
        Vec<crate::scenario::WorkerSpec>,
    )> {
        let mut specs: Vec<_> = self
            .scenario
            .miners
            .iter()
            .map(|miner| {
                (
                    miner.id.clone(),
                    miner.implementation.clone(),
                    miner.connections.clone(),
                    miner.workers.clone(),
                )
            })
            .collect();
        for (id, extra) in &self.state.lock().unwrap().extra_miners {
            specs.push((
                id.clone(),
                extra.implementation.clone(),
                extra.connections.clone(),
                extra.workers.clone(),
            ));
        }
        specs
    }

    /// Live miner snapshots for the runtime metrics observer. Snapshots are
    /// requested from the approved v1 control surface; no rate is inferred
    /// from command completion.
    pub fn runtime_miner_snapshots(&self) -> Vec<serde_json::Value> {
        // Clone the handles out before any blocking pipe request: snapshot
        // rendering takes the evidence lock, and a slow miner must not stall it.
        let miners: Vec<(MinerId, Arc<MinerControl>)> = self
            .adapters
            .miners
            .lock()
            .unwrap()
            .iter()
            .map(|(miner, control)| (miner.clone(), Arc::clone(control)))
            .collect();
        let mut snapshots = Vec::new();
        for (miner, control) in miners {
            match control.request(MinerOperation::Snapshot, serde_json::json!({})) {
                Ok(response) => snapshots.push(serde_json::json!({
                    "miner_id": miner,
                    "outcome": response.outcome,
                    "result": response.result,
                })),
                Err(error) => snapshots.push(serde_json::json!({
                    "miner_id": miner,
                    "error": error,
                })),
            }
        }
        snapshots
    }

    pub fn resources_manifest(&self) -> crate::manifest::ResourceManifest {
        self.manifest.lock().unwrap().resources.clone()
    }

    pub fn start(&self) -> Result<(), EngineError> {
        {
            let state = self.state.lock().unwrap();
            if state.started {
                return Err(EngineError::new(
                    "already_started",
                    "run is already started",
                ));
            }
        }
        let current = self.manifest_state();
        if !current.can_start() {
            return Err(EngineError::new(
                "start_precondition",
                format!("run state {current:?} cannot be started"),
            ));
        }
        self.set_manifest_state(RunState::Preparing)?;
        self.emit_run_state("preparing");
        let result = self.launch_all();
        match result {
            Ok(()) => {
                self.set_manifest_state(RunState::Running)?;
                self.emit_run_state("running");
                self.state.lock().unwrap().started = true;
                Ok(())
            }
            Err(error) => {
                self.supervisor
                    .lock()
                    .unwrap()
                    .stop_all(Duration::from_secs(1));
                self.sync_processes()?;
                self.set_manifest_state(RunState::Failed)?;
                self.emit_run_state("failed");
                Err(error)
            }
        }
    }

    fn launch_all(&self) -> Result<(), EngineError> {
        if self.scenario.ark_backends.iter().any(|b| b.local.is_some())
            && self.startup_hook.lock().unwrap().is_none()
        {
            return Err(EngineError::new(
                "local_ark_hook_missing",
                "use the built-in runtime for self-hosted Ark",
            ));
        }
        self.launch_storage()?;
        self.launch_nodes()?;
        if let Some(hook) = self.startup_hook.lock().unwrap().clone() {
            hook.after_storage_and_nodes(self)
                .map_err(|detail| EngineError::new("startup_hook", detail))?;
        }
        self.traffic_preflight()?;
        self.launch_fronts()?;
        self.launch_ark()?;
        if let Some(hook) = self.startup_hook.lock().unwrap().clone() {
            hook.after_fronts(self)
                .map_err(|detail| EngineError::new("wallet_startup", detail))?;
        }
        self.launch_miners()?;
        self.launch_tx_drivers()?;
        Ok(())
    }

    fn launch_storage(&self) -> Result<(), EngineError> {
        for redis in &self.resources.redis_instances {
            let adapter = self.config.redis.as_ref().ok_or_else(|| {
                EngineError::new(
                    "adapter_missing",
                    "Redis adapter disappeared after preflight",
                )
            })?;
            let context = self.context([
                ("role", "redis".to_string()),
                ("entity", redis.id.clone()),
                ("port", redis.port.to_string()),
                ("data_dir", redis.dir.display().to_string()),
                ("redis_url", redis.url.clone()),
            ]);
            let args = expand_args(adapter, &[], &context)?;
            let env = adapter.env.clone();
            let guard = self
                .artifacts
                .paths()
                .logs
                .join(format!("redis-{}.log", redis.id));
            let _ = self.launch_process(
                "redis",
                &redis.id,
                adapter,
                args,
                env,
                Readiness::Tcp {
                    host: self
                        .addresses
                        .get(&format!("redis.{}.addr", redis.id))
                        .cloned()
                        .unwrap_or_else(|| "127.0.0.1".to_string()),
                    port: redis.port,
                },
                &guard,
                false,
            )?;
        }
        for pg in &self.resources.postgres_databases {
            let adapter = self.config.postgres.as_ref().ok_or_else(|| {
                EngineError::new(
                    "adapter_missing",
                    "Postgres adapter disappeared after preflight",
                )
            })?;
            let context = self.context([
                ("role", "postgres".to_string()),
                ("entity", pg.id.clone()),
                ("port", pg.port.to_string()),
                ("data_dir", pg.dir.display().to_string()),
                ("database", pg.database.clone()),
                ("owner_role", pg.role.clone()),
                ("postgres_url", pg.url.clone()),
            ]);
            let args = expand_args(adapter, &[], &context)?;
            let env = adapter.env.clone();
            let guard = self
                .artifacts
                .paths()
                .logs
                .join(format!("postgres-{}.log", pg.id));
            let _ = self.launch_process(
                "postgres",
                &pg.id,
                adapter,
                args,
                env,
                Readiness::Tcp {
                    host: "127.0.0.1".to_string(),
                    port: pg.port,
                },
                &guard,
                false,
            )?;
        }
        Ok(())
    }

    fn launch_nodes(&self) -> Result<(), EngineError> {
        let mut profiles = BTreeMap::new();
        let mut controls: BTreeMap<NodeId, Box<dyn NodeControl>> = BTreeMap::new();
        // Pre-pass: render and validate every node's argv BEFORE any process
        // is launched. A caller-supplied P2P flag or a duplicate flag fails
        // closed here with zero process, manifest or profile effects.
        let mut expanded: BTreeMap<NodeId, Vec<String>> = BTreeMap::new();
        for node in &self.scenario.nodes {
            let adapter = self.config.node(&node.implementation).ok_or_else(|| {
                EngineError::new(
                    "adapter_missing",
                    format!("node {} adapter is not configured", node.id),
                )
            })?;
            let context = self.node_launch_context(node)?;
            let args = expand_args(adapter, &node.args, &context)?;
            validate_node_p2p_argv(&node.id, &args)?;
            expanded.insert(node.id.clone(), args);
        }
        // Host-mode peering plan derived from the validated argv.
        let host_ring = self.host_node_ring(&expanded)?;
        for node in &self.scenario.nodes {
            let adapter = self.config.node(&node.implementation).ok_or_else(|| {
                EngineError::new(
                    "adapter_missing",
                    format!("node {} adapter is not configured", node.id),
                )
            })?;
            let p2p = self.listener("node", node.id.as_ref(), "p2p")?;
            let rpc = self.listener("node", node.id.as_ref(), "rpc")?;
            let height_file = self.run_dir.join(format!("height-{}.txt", node.id));
            if !height_file.exists() {
                std::fs::write(&height_file, b"0")
                    .map_err(|e| EngineError::new("height_file", e.to_string()))?;
            }
            let profile_file = self.run_dir.join(format!("profile-{}.json", node.id));
            let profile = self
                .config
                .node_profiles
                .get(&node.id)
                .cloned()
                .or_else(|| adapter.effective_profile.clone());
            if let Some(profile) = &profile {
                std::fs::write(
                    &profile_file,
                    serde_json::to_vec_pretty(profile)
                        .map_err(|e| EngineError::new("profile_encode", e.to_string()))?,
                )
                .map_err(|e| EngineError::new("profile_file", e.to_string()))?;
            }
            let mut args = expanded
                .remove(&node.id)
                .expect("pre-pass rendered every node argv");
            // In a namespaced run the node listens on its published P2P
            // address and connects to its peer over the consensus link.  The
            // host run keeps the loopback default.
            if let Some(p2p_addr) = self.addresses.get(&format!("node.{}.p2p_addr", node.id)) {
                args.retain(|argument| argument != "--no-listen");
                args.push("--listen".to_string());
                args.push(format!("{p2p_addr}:{}", p2p.port));
                if let Some(peer) = self
                    .addresses
                    .get(&format!("node.{}.p2p_connect_addr", node.id))
                {
                    let peer_port = self
                        .scenario
                        .nodes
                        .iter()
                        .filter(|candidate| candidate.id != node.id)
                        .find(|candidate| {
                            self.addresses
                                .get(&format!("node.{}.p2p_addr", candidate.id))
                                == Some(peer)
                        })
                        .map(|candidate| self.listener("node", candidate.id.as_str(), "p2p"))
                        .transpose()?
                        .ok_or_else(|| {
                            EngineError::new(
                                "topology_peer_missing",
                                format!("node {} has no planned owner for peer {peer}", node.id),
                            )
                        })?
                        .port;
                    args.push("--connect".to_string());
                    args.push(format!("{peer}:{peer_port}"));
                }
            } else if let Some((listen, connect)) = host_ring.get(&node.id) {
                // Host run without namespace topology: every node was
                // rendered in the isolated listener shape, so peer them in
                // scenario order over the leased loopback P2P ports. The
                // readiness gate observes this planned graph before any
                // bootstrap hash budget is reserved.
                args.retain(|argument| argument != "--no-listen");
                args.push("--listen".to_string());
                args.push(listen.clone());
                args.push("--connect".to_string());
                args.push(connect.clone());
            }
            let log = self
                .artifacts
                .paths()
                .logs
                .join(format!("node-{}.log", node.id));
            let _ = self.launch_process(
                "node",
                node.id.as_str(),
                adapter,
                args,
                node.env.clone(),
                Readiness::Tcp {
                    host: self
                        .addresses
                        .get(&format!("node.{}.rpc_addr", node.id))
                        .cloned()
                        .unwrap_or_else(|| "127.0.0.1".to_string()),
                    port: rpc.port,
                },
                &log,
                false,
            )?;
            let control: Box<dyn NodeControl> = if adapter.fixture {
                Box::new(FixtureNodeControl::new(
                    node.id.clone(),
                    profile.unwrap_or_default(),
                    height_file,
                ))
            } else {
                let rpc_endpoint = self
                    .addresses
                    .get(&format!("node.{}.rpc_addr", node.id))
                    .map(|host| Endpoint {
                        host: host.clone(),
                        port: rpc.port,
                    })
                    .unwrap_or_else(|| rpc.clone());
                let mut control =
                    RpcNodeControl::new(node.id.clone(), rpc_endpoint, adapter.rpc_auth.clone());
                if let (Some(broker), Some(_)) = (
                    self.namespace_broker.as_ref(),
                    self.namespaces.get(&format!("node/{}", node.id)),
                ) {
                    control = control.with_auth_file(
                        self.run_dir
                            .join("private")
                            .join(format!("node-rpc-{}.cookie", node.id)),
                    );
                    control = control.in_namespace(crate::adapters::NamespaceClient {
                        broker: std::sync::Arc::clone(broker),
                        endpoint: node.id.to_string(),
                    });
                }
                let control = Arc::new(control);
                self.adapters
                    .rpc_nodes
                    .lock()
                    .unwrap()
                    .insert(node.id.clone(), Arc::clone(&control));
                Box::new(control)
            };
            profiles.insert(
                node.id.clone(),
                control.effective_profile().map_err(|e| {
                    EngineError::new("profile_unavailable", format!("node {}: {e}", node.id))
                })?,
            );
            controls.insert(node.id.clone(), control);
        }
        let warnings = validate_effective_profiles(&self.scenario, &profiles)
            .map_err(|e| EngineError::new(e.code(), e.detail()))?;
        {
            let mut manifest = self.manifest.lock().unwrap();
            manifest.effective_profiles = profiles;
            manifest.warnings.extend(warnings);
            manifest.updated_at = manifest_now_ms();
            manifest
                .save(&self.manifest_path)
                .map_err(|e| EngineError::new("manifest_write", e.to_string()))?;
        }
        *self.adapters.nodes.lock().unwrap() = controls;
        Ok(())
    }

    fn launch_fronts(&self) -> Result<(), EngineError> {
        for front in &self.scenario.fronts {
            let pool = self
                .scenario
                .pools
                .iter()
                .find(|pool| pool.id == front.pool_id)
                .ok_or_else(|| EngineError::new("front_pool_missing", front.pool_id.to_string()))?;
            let adapter = self.config.pool(&pool.implementation).ok_or_else(|| {
                EngineError::new(
                    "adapter_missing",
                    format!("pool {} adapter is not configured", pool.id),
                )
            })?;
            let sv2 = self.listener("front", front.id.as_str(), "sv2")?;
            let sv1 = self
                .resources
                .listeners
                .get(&format!("front.{}.sv1", front.id))
                .cloned();
            let jd = self
                .resources
                .listeners
                .get(&format!("front.{}.jd", front.id))
                .cloned();
            let api = self
                .resources
                .listeners
                .get(&format!("front.{}.api", front.id))
                .cloned();
            let redis = self
                .resources
                .redis_for_pool(&pool.id)
                .ok_or_else(|| EngineError::new("redis_missing", pool.id.to_string()))?;
            let pg = self
                .resources
                .postgres_for_pool(&pool.id)
                .ok_or_else(|| EngineError::new("postgres_missing", pool.id.to_string()))?;
            let authority = self.authorities.get(&pool.id);
            let context = self.context([
                ("role", "front".to_string()),
                ("entity", front.id.to_string()),
                ("sv2_port", sv2.port.to_string()),
                (
                    "sv1_port",
                    sv1.map(|e| e.port.to_string()).unwrap_or_default(),
                ),
                (
                    "jd_port",
                    jd.map(|e| e.port.to_string()).unwrap_or_default(),
                ),
                (
                    "api_port",
                    api.map(|e| e.port.to_string()).unwrap_or_default(),
                ),
                ("redis_url", redis.url.clone()),
                ("postgres_url", pg.url.clone()),
                (
                    "authority_public_key",
                    authority
                        .map(|value| value.public_base58().to_string())
                        .unwrap_or_default(),
                ),
                (
                    "authority_secret_file",
                    authority
                        .map(|value| value.secret_file().display().to_string())
                        .unwrap_or_default(),
                ),
            ]);
            let args = expand_args(adapter, &front.args, &context)?;
            let log = self
                .artifacts
                .paths()
                .logs
                .join(format!("front-{}.log", front.id));
            let _ = self.launch_process(
                "front",
                front.id.as_str(),
                adapter,
                args,
                front.env.clone(),
                Readiness::Tcp {
                    host: self
                        .addresses
                        .get(&format!("front.{}.sv2_addr", front.id))
                        .cloned()
                        .unwrap_or_else(|| "127.0.0.1".to_string()),
                    port: sv2.port,
                },
                &log,
                false,
            )?;
        }
        Ok(())
    }

    fn launch_ark(&self) -> Result<(), EngineError> {
        for ark in &self.scenario.ark_backends {
            // The local stock backend is provisioned by the startup hook,
            // before front metadata is rendered and before any miner starts.
            if ark.local.is_some() {
                continue;
            }
            let adapter = self.config.ark.get(&ark.implementation).ok_or_else(|| {
                EngineError::new(
                    "adapter_missing",
                    format!("Ark backend {} adapter is not configured", ark.id),
                )
            })?;
            let endpoint = self
                .resources
                .listeners
                .get(&format!("ark.{}", ark.id))
                .cloned()
                .ok_or_else(|| EngineError::new("ark_listener_missing", ark.id.to_string()))?;
            let context = self.context([
                ("role", "ark".to_string()),
                ("entity", ark.id.to_string()),
                ("port", endpoint.port.to_string()),
            ]);
            let args = expand_args(adapter, &[], &context)?;
            let log = self
                .artifacts
                .paths()
                .logs
                .join(format!("ark-{}.log", ark.id));
            let _ = self.launch_process(
                "ark",
                ark.id.as_str(),
                adapter,
                args,
                adapter.env.clone(),
                Readiness::Tcp {
                    host: "127.0.0.1".to_string(),
                    port: endpoint.port,
                },
                &log,
                false,
            )?;
        }
        Ok(())
    }

    fn launch_miners(&self) -> Result<(), EngineError> {
        for miner in &self.scenario.miners {
            self.launch_miner(
                &miner.id,
                &miner.implementation,
                &miner.connections,
                &miner.workers,
            )?;
        }
        Ok(())
    }

    fn launch_miner(
        &self,
        miner_id: &MinerId,
        implementation: &str,
        connections: &[ConnectionSpec],
        workers: &[WorkerSpec],
    ) -> Result<(), EngineError> {
        crate::wallet_bridge::require_ready(self, connections)
            .map_err(|detail| EngineError::new("ark_wallet_not_ready", detail))?;
        if let Some(provisioner) = self.miner_provisioner.lock().unwrap().clone() {
            provisioner
                .prepare_miner(self, miner_id, implementation)
                .map_err(|detail| EngineError::new("miner_provision", detail))?;
        }
        let adapter = self.config.miner(implementation).ok_or_else(|| {
            EngineError::new(
                "adapter_missing",
                format!("miner {miner_id} adapter {implementation:?} is not configured"),
            )
        })?;
        let context = self.context([
            ("role", "miner".to_string()),
            ("entity", miner_id.to_string()),
            ("miner_id", miner_id.to_string()),
        ]);
        let args = expand_args(adapter, &[], &context)?;
        let log = self
            .artifacts
            .paths()
            .logs
            .join(format!("miner-{miner_id}.log"));
        let launched = self.launch_process(
            "miner",
            miner_id.as_str(),
            adapter,
            args,
            adapter.env.clone(),
            Readiness::Immediate,
            &log,
            true,
        )?;
        let stdin = launched
            .stdin
            .ok_or_else(|| EngineError::new("miner_stdin", "miner control stdin missing"))?;
        let stdout = launched
            .stdout
            .ok_or_else(|| EngineError::new("miner_stdout", "miner control stdout missing"))?;
        let incarnation = format!(
            "{}:{}",
            launched.record.run_token, launched.record.start_time_ticks
        );
        let control = MinerControl::spawn(miner_id.clone(), incarnation, stdin, stdout, log)
            .map_err(|e| EngineError::new("miner_control", e))?;
        for connection in connections.iter().filter(|_| {
            self.scenario.traffic.is_none()
                || self.manifest.lock().unwrap().traffic_stage
                    == Some(crate::traffic::runtime::TrafficStage::Active)
        }) {
            let payload = self.connection_payload(connection, workers)?;
            self.send_miner(&control, MinerOperation::AddConnection, payload)
                .map_err(|(_, detail)| EngineError::new("miner_rejected", detail))?;
        }
        // Workers are part of add_connection in protocol version 1. A later
        // set_worker_budget only changes an already declared worker.
        let _ = workers;
        self.spawn_miner_event_pump(miner_id, &control);
        self.adapters
            .miners
            .lock()
            .unwrap()
            .insert(miner_id.clone(), Arc::new(control));
        Ok(())
    }

    fn identity_address(&self, identity: &IdentityId) -> Result<String, EngineError> {
        let spec = self
            .scenario
            .identities
            .iter()
            .find(|item| &item.id == identity)
            .ok_or_else(|| {
                EngineError::new(
                    "identity_unknown",
                    format!("identity {identity} is not declared"),
                )
            })?;
        Ok(match &spec.address_source {
            crate::scenario::AddressSource::Fixture { fixture } => fixture.clone(),
            crate::scenario::AddressSource::Explicit { address } => address.clone(),
        })
    }

    fn front_endpoint(&self, front: &FrontId) -> Result<String, EngineError> {
        let endpoint = self
            .resources
            .listeners
            .get(&format!("front.{front}.sv2"))
            .ok_or_else(|| {
                EngineError::new(
                    "front_listener_missing",
                    format!("front {front} has no allocated SV2 endpoint"),
                )
            })?;
        // A namespaced front is reachable on its published address, not on the
        // loopback address of the host.
        match self.addresses.get(&format!("front.{front}.sv2_addr")) {
            Some(address) => Ok(format!("{address}:{}", endpoint.port)),
            None => Ok(endpoint.display()),
        }
    }

    pub(crate) fn authority_public(&self, front: &FrontId) -> Result<String, EngineError> {
        let pool = self.scenario.pool_for_front(front).ok_or_else(|| {
            EngineError::new("front_pool_missing", format!("front {front} has no pool"))
        })?;
        self.authorities
            .get(&pool.id)
            .map(|authority| authority.public_base58().to_string())
            .ok_or_else(|| {
                EngineError::new(
                    "authority_missing",
                    format!("pool {} has no generated authority pair", pool.id),
                )
            })
    }

    fn worker_payload(&self, worker: &WorkerSpec) -> Value {
        let budget = serde_json::json!({
            "hashes_per_second": wide_json(worker.budget.hashes_per_second),
            "total_attempts": wide_json(worker.budget.total_hashes),
        });
        let ntime_policy = match self.scenario.run.time_policy.miner_ntime {
            crate::scenario::MinerNtime::Hold => "hold",
            crate::scenario::MinerNtime::Roll => "roll",
        };
        serde_json::json!({
            "id": worker.id,
            "budget": budget,
            "ntime_policy": ntime_policy,
            "max_ntime_roll_seconds": 600,
            "batch_size": 256,
        })
    }

    fn connection_payload(
        &self,
        connection: &ConnectionSpec,
        workers: &[WorkerSpec],
    ) -> Result<Value, EngineError> {
        let connection_workers: Vec<Value> = workers
            .iter()
            .filter(|worker| worker.connection_id == connection.id)
            .map(|worker| self.worker_payload(worker))
            .collect();
        Ok(serde_json::json!({
            "connection": {
                "id": connection.id,
                "endpoint": self.front_endpoint(&connection.front_id)?,
                "identity": self.identity_address(&connection.identity_id)?,
                "authority_public_key": self.authority_public(&connection.front_id)?,
                "min_extranonce_size": 4,
                "reconnect": {
                    "enabled": true,
                    "initial_backoff_ms": 100,
                    "max_backoff_ms": 5000,
                    "handshake_timeout_ms": 10000,
                },
                "workers": connection_workers,
            }
        }))
    }

    fn send_miner(
        &self,
        control: &MinerControl,
        operation: MinerOperation,
        mut payload: Value,
    ) -> Result<MinerWireResponse, (OutcomeState, String)> {
        if matches!(operation, MinerOperation::AddConnection) {
            let ready = control
                .request(MinerOperation::Snapshot, serde_json::json!({}))
                .map_err(|e| (OutcomeState::RejectedPrecondition, e))?;
            if !ready.is_success() {
                return Err((
                    OutcomeState::RejectedPrecondition,
                    "native capability snapshot unavailable".into(),
                ));
            }
            let advertised = ready.result.as_ref().and_then(|r| r.get("native_evidence"));
            self.bind_native_audit(control.miner_id(), &mut payload, advertised)
                .map_err(|e| (OutcomeState::RejectedPrecondition, e.to_string()))?;
        }
        let mut attempt = None;
        let trace =
            self.scenario.traffic.is_some() && matches!(operation, MinerOperation::AddConnection);
        let response = control.request_traced(operation, payload, |id, bytes| {
            if trace {
                let manifest = self.manifest.lock().unwrap();
                let actor = manifest
                    .process("miner", control.miner_id().as_str())
                    .ok_or("miner audit identity missing")?;
                let controller = crate::supervisor::hash_file(
                    &std::env::current_exe().map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?;
                attempt = Some(
                    self.command_log
                        .audit(crate::command::traffic_audit::Event::MinerAttempt {
                            run_id: self.run_id.to_string(),
                            miner_id: control.miner_id().to_string(),
                            incarnation: control.incarnation().into(),
                            request_id: id.to_string(),
                            payload_sha256: crate::command::traffic_audit::hash(bytes),
                            controller_sha256: controller,
                            actor_sha256: actor.executable_sha256.clone(),
                        })
                        .map_err(|e| e.to_string())?,
                );
            }
            Ok(())
        });
        if let Some(attempt_seq) = attempt {
            self.command_log
                .audit(crate::command::traffic_audit::Event::MinerResult {
                    attempt_seq,
                    succeeded: response.as_ref().is_ok_and(|r| r.is_success()),
                })
                .map_err(|e| (OutcomeState::Failed, e.to_string()))?;
        }
        let response = response.map_err(|e| (OutcomeState::Failed, e))?;
        if response.is_rejected() {
            return Err((OutcomeState::RejectedPrecondition, response.error_text()));
        }
        if !response.is_success() {
            return Err((OutcomeState::Failed, response.error_text()));
        }
        Ok(response)
    }

    fn spawn_miner_event_pump(&self, miner_id: &MinerId, control: &MinerControl) {
        let Some(engine) = self.self_ref.lock().unwrap().upgrade() else {
            return;
        };
        // The pump must not keep the engine alive after the supervisor is
        // dropped: a crash closes the control pipe and ends this loop.
        let weak = Arc::downgrade(&engine);
        let receiver = control.event_receiver();
        let miner_id = miner_id.clone();
        let _ = std::thread::Builder::new()
            .name(format!("miner-{miner_id}-events"))
            .spawn(move || loop {
                let event = {
                    let receiver = receiver.lock().unwrap();
                    receiver.recv()
                };
                let Ok(event) = event else { break };
                let Some(engine) = weak.upgrade() else { break };
                engine.record_miner_event(&miner_id, event);
            });
    }

    fn record_miner_event(&self, miner_id: &MinerId, event: crate::adapters::MinerWireEvent) {
        let is_parse_error = event.event.name == "driver_parse_error";
        let dropped_before = event.dropped_before;
        let incarnation = self
            .adapters
            .miners
            .lock()
            .unwrap()
            .get(miner_id)
            .map(|control| control.incarnation().to_string())
            .unwrap_or_else(|| "unknown".to_string());
        self.evidence.append_at(
            SourceId::trusted(format!("miner-{miner_id}")),
            format!("{incarnation}:{}", event.source_cursor),
            EvidenceActor::new(ActorKind::Miner, miner_id.to_string()),
            None,
            EvidencePayload::MinerEvent {
                miner_id: miner_id.clone(),
                incarnation: incarnation.clone(),
                driver_event_id: event.event_id,
                name: event.event.name,
                payload: event.event.payload,
                dropped_before: event.dropped_before,
            },
            event.occurred_at_wall_ms,
        );
        let loss_delta = {
            let mut state = self.state.lock().unwrap();
            let previous = state.driver_dropped.get(&incarnation).copied().unwrap_or(0);
            let delta = if is_parse_error {
                1
            } else {
                dropped_before.saturating_sub(previous)
            };
            state
                .driver_dropped
                .insert(incarnation.clone(), dropped_before.max(previous));
            delta
        };
        if loss_delta > 0 {
            self.evidence.record_gap(Gap {
                scope: GapScope::Store,
                subscriber: 0,
                from_event_id: event.event_id,
                to_event_id: event.event_id,
                reason: if is_parse_error {
                    "miner driver parse failure".to_string()
                } else {
                    format!(
                        "miner driver dropped events (delta={loss_delta}, cumulative={dropped_before})"
                    )
                },
            });
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn launch_process(
        &self,
        role: &str,
        entity: &str,
        adapter: &AdapterConfig,
        args: Vec<String>,
        env: BTreeMap<String, String>,
        readiness: Readiness,
        log_path: &Path,
        pipes: bool,
    ) -> Result<Launched, EngineError> {
        let mut spec = ProcessSpec::new(role, entity, &adapter.program, log_path);
        spec.args = args;
        spec.env = env;
        if role == "tx_driver" {
            spec.clear_env = true;
        }
        if role == "node" && self.scenario.traffic.is_some() {
            spec.clear_env = true;
            spec.env.insert(
                "SPC_LAB_RUN_TOKEN".into(),
                self.supervisor.lock().unwrap().run_token().into(),
            );
            let profile = crate::traffic::launch_profile::LaunchProfile::capture(
                crate::supervisor::hash_file(&adapter.program)
                    .map_err(|e| EngineError::new("node_hash", e.to_string()))?,
                spec.args.clone(),
                spec.env.clone(),
            )
            .map_err(|e| EngineError::new(e, e))?;
            let mut manifest = self.manifest.lock().unwrap();
            manifest
                .traffic_launch_profiles
                .insert(NodeId::trusted(entity), profile);
            manifest
                .save(&self.manifest_path)
                .map_err(|e| EngineError::new("manifest_write", e.to_string()))?;
        }
        spec.readiness = readiness;
        spec.pipe_stdin = pipes;
        spec.pipe_stdout = pipes;
        if pipes {
            spec.control = ControlTransport::Pipe;
        }
        spec.ready_timeout_ms = 10_000;
        // The real runtime gives every actor a safe parent-death helper. The
        // controller itself never re-execs or changes namespace.
        spec.lifetime_helper = self.lifetime_helper.clone();
        let namespace_key = format!("{role}/{entity}");
        if let Some(netns) = self.namespaces.get(&namespace_key) {
            let nsenter = self.nsenter.clone().ok_or_else(|| {
                EngineError::new(
                    "namespace_launcher_missing",
                    format!("{namespace_key} has no nsenter binary"),
                )
            })?;
            let userns = self.user_namespace.clone().ok_or_else(|| {
                EngineError::new(
                    "namespace_launcher_missing",
                    format!("{namespace_key} has no broker user namespace"),
                )
            })?;
            spec.in_namespaces(userns, netns.clone(), nsenter);
            spec.namespace_owner = self.namespace_owners.get(&namespace_key).cloned();
            spec.user_namespace_owner = self.user_namespace_owner.clone();
            if spec.namespace_owner.is_none() || spec.user_namespace_owner.is_none() {
                return Err(EngineError::new(
                    "namespace_owner_missing",
                    format!("{namespace_key} lacks verified owner identities"),
                ));
            }
            // The readiness probe enters the same user/net namespaces.
            spec.readiness_helper = std::env::current_exe().ok();
        }
        let launched = self
            .supervisor
            .lock()
            .unwrap()
            .launch_with_pipes(spec)
            .map_err(|e| EngineError::new("process_launch", format!("{role}/{entity}: {e}")))?;
        {
            let mut manifest = self.manifest.lock().unwrap();
            manifest.upsert_process(launched.record.clone());
            manifest.updated_at = manifest_now_ms();
            manifest
                .save(&self.manifest_path)
                .map_err(|e| EngineError::new("manifest_write", e.to_string()))?;
        }
        Ok(launched)
    }

    fn listener(&self, kind: &str, entity: &str, name: &str) -> Result<Endpoint, EngineError> {
        self.resources
            .listeners
            .get(&format!("{kind}.{entity}.{name}"))
            .cloned()
            .ok_or_else(|| {
                EngineError::new(
                    "listener_missing",
                    format!("no recorded {kind}.{entity}.{name} listener"),
                )
            })
    }

    fn context<const N: usize>(
        &self,
        pairs: [(&'static str, String); N],
    ) -> BTreeMap<&'static str, String> {
        let mut context = BTreeMap::new();
        context.insert("run_dir", self.run_dir.display().to_string());
        context.insert("run_id", self.run_id.to_string());
        context.insert(
            "scenario",
            self.artifacts
                .paths()
                .private_scenario
                .display()
                .to_string(),
        );
        for (key, value) in pairs {
            context.insert(key, value);
        }
        context
    }

    /// Render the template context for one node's argv. Pure: recomputes the
    /// same listener and address values the launch loop uses, so the
    /// pre-launch validation pass and the loop agree on every rendered
    /// argument.
    fn node_launch_context(
        &self,
        node: &NodeSpec,
    ) -> Result<BTreeMap<&'static str, String>, EngineError> {
        let p2p = self.listener("node", node.id.as_ref(), "p2p")?;
        let rpc = self.listener("node", node.id.as_ref(), "rpc")?;
        let tdp = self.listener("node", node.id.as_ref(), "tdp")?;
        let height_file = self.run_dir.join(format!("height-{}.txt", node.id));
        let profile_file = self.run_dir.join(format!("profile-{}.json", node.id));
        Ok(self.context([
            ("role", "node".to_string()),
            ("entity", node.id.to_string()),
            ("rpc_port", rpc.port.to_string()),
            ("p2p_port", p2p.port.to_string()),
            ("tdp_port", tdp.port.to_string()),
            ("rpc_endpoint", rpc.display()),
            ("height_file", height_file.display().to_string()),
            ("profile_file", profile_file.display().to_string()),
            (
                "rpc_addr",
                self.addresses
                    .get(&format!("node.{}.rpc_addr", node.id))
                    .cloned()
                    .unwrap_or_else(|| "127.0.0.1".to_string()),
            ),
            (
                "p2p_addr",
                self.addresses
                    .get(&format!("node.{}.p2p_addr", node.id))
                    .cloned()
                    .unwrap_or_else(|| "127.0.0.1".to_string()),
            ),
            (
                "p2p_connect_addr",
                self.addresses
                    .get(&format!("node.{}.p2p_connect_addr", node.id))
                    .cloned()
                    .unwrap_or_else(|| "127.0.0.1".to_string()),
            ),
        ]))
    }

    /// Ring peering plan for host runs (no namespace topology). Applies only
    /// when there is more than one node and every node was rendered in the
    /// full isolated shape (`--no-listen --no-seeds --no-discover
    /// --max-outbound 1`): each node gets `--listen` on its own leased
    /// loopback P2P port and `--connect` to its scenario-order ring
    /// successor, so a host run still forms the planned peer graph. Any
    /// other shape is left untouched: a single node stays isolated, and runs
    /// with no isolated listener flag anywhere are the caller's own wiring.
    /// Mixed or ambiguous shapes fail closed before any launch.
    fn host_node_ring(
        &self,
        expanded: &BTreeMap<NodeId, Vec<String>>,
    ) -> Result<BTreeMap<NodeId, (String, String)>, EngineError> {
        // A topology run wires P2P through published addresses in the launch
        // loop; the ring never applies there.
        if self
            .addresses
            .keys()
            .any(|key| key.starts_with("node.") && key.ends_with(".p2p_addr"))
        {
            return Ok(BTreeMap::new());
        }
        if expanded.len() < 2 {
            return Ok(BTreeMap::new());
        }
        let full_isolated = |args: &[String]| {
            args.iter().any(|argument| argument == "--no-listen")
                && args.iter().any(|argument| argument == "--no-seeds")
                && args.iter().any(|argument| argument == "--no-discover")
                && argv_flag_value(args, "--max-outbound") == Some("1")
        };
        if !expanded.values().all(|args| full_isolated(args)) {
            if expanded
                .values()
                .all(|args| !args.iter().any(|argument| argument == "--no-listen"))
            {
                return Ok(BTreeMap::new());
            }
            return Err(EngineError::new(
                "node_p2p_plan_invalid",
                "nodes mix isolated and non-isolated P2P shapes; refusing to plan host ring peering",
            ));
        }
        let mut endpoints = BTreeMap::new();
        let mut ports = BTreeSet::new();
        for node in &self.scenario.nodes {
            // Host runs have no published addresses, so every node peers on
            // loopback at its own leased P2P port.
            let host = self
                .addresses
                .get(&format!("node.{}.p2p_addr", node.id))
                .cloned()
                .unwrap_or_else(|| "127.0.0.1".to_string());
            let port = self.listener("node", node.id.as_ref(), "p2p")?.port;
            if !ports.insert(port) {
                return Err(EngineError::new(
                    "node_p2p_plan_invalid",
                    "two nodes share one leased P2P port",
                ));
            }
            endpoints.insert(node.id.clone(), (host, port));
        }
        if endpoints
            .values()
            .map(|(host, _)| host.as_str())
            .collect::<BTreeSet<_>>()
            .len()
            > 1
        {
            return Err(EngineError::new(
                "node_p2p_plan_invalid",
                "host node P2P addresses span more than one host",
            ));
        }
        let order: Vec<NodeId> = self.scenario.nodes.iter().map(|n| n.id.clone()).collect();
        let mut ring = BTreeMap::new();
        for (index, node) in order.iter().enumerate() {
            let successor = &order[(index + 1) % order.len()];
            let (listen_host, listen_port) = &endpoints[node];
            let (connect_host, connect_port) = &endpoints[successor];
            ring.insert(
                node.clone(),
                (
                    format!("{listen_host}:{listen_port}"),
                    format!("{connect_host}:{connect_port}"),
                ),
            );
        }
        Ok(ring)
    }

    /// Record the published placement in the run manifest.
    ///
    /// The placement is recorded before the configurations are rendered, so a
    /// reader of the manifest always sees the addresses the actors were
    /// actually given.
    pub fn record_placement(&self, placement: serde_json::Value) -> Result<(), EngineError> {
        let mut manifest = self.manifest.lock().unwrap();
        manifest.placement = Some(placement);
        manifest.updated_at = manifest_now_ms();
        manifest
            .save(&self.manifest_path)
            .map_err(|e| EngineError::new("manifest_write", e.to_string()))?;
        Ok(())
    }

    pub fn run_id(&self) -> &RunId {
        &self.run_id
    }

    pub fn snapshot(&self, cursor: Option<u64>) -> Snapshot {
        self.evidence.snapshot_with(cursor, || self.entity_states())
    }

    /// True when the scenario opted into the expanded-access profile.
    pub fn expanded_access(&self) -> bool {
        self.scenario.expanded_access
    }

    /// The expanded-access claim store, when the run opted in.
    pub fn access(&self) -> Option<&AccessStore> {
        self.access.as_ref()
    }

    /// Authenticate a bearer token against the stored claims. Missing,
    /// unknown, malformed and revoked tokens all return `None`.
    pub fn authenticate_bearer(&self, token: &str) -> Option<AuthContext> {
        self.access.as_ref()?.authenticate(token)
    }

    /// The filtered user snapshot (`spc-user-snapshot/v1`): the whole-run
    /// snapshot is filtered server-side before serialization.
    pub fn user_snapshot(&self, auth: &AuthContext, cursor: Option<u64>) -> serde_json::Value {
        let snapshot = self.snapshot(cursor);
        crate::access::user_snapshot(&auth.claim, &snapshot)
    }

    /// The miner-poll worker projection for one user
    /// (`spc-user-projection/v1`). Worker state comes from the existing
    /// single miner v1 control poll - there is no second pipe. A failed,
    /// rejected or absent poll yields `null` fields with a closed reason,
    /// never zeros.
    pub fn user_projection(&self, auth: &AuthContext) -> UserProjection {
        let mut projection = self.miner_projection(auth);
        let user = auth.claim.user_id.as_deref().unwrap_or("");
        projection.payment_wallet = Some(self.t1_payment_wallet(auth, user).unwrap_or_else(|_| {
            crate::traffic::payment_view::PaymentWalletView::unavailable(
                user,
                "not_active",
                "payment_provider_unavailable",
            )
        }));
        projection
    }

    fn miner_projection(&self, auth: &AuthContext) -> UserProjection {
        let observed_at = crate::command::now_ms();
        let Some(miner_id) = auth.claim.miner_id.clone() else {
            return UserProjection::from_miner_snapshot(
                &auth.claim,
                None,
                Some("claim has no miner binding"),
                observed_at,
            );
        };
        // Clone the handle out before the blocking pipe request so a slow
        // miner never holds the adapter lock.
        let control = self.adapters.miners.lock().unwrap().get(&miner_id).cloned();
        match control {
            Some(control) => {
                match control.request(MinerOperation::Snapshot, serde_json::json!({})) {
                    Ok(response) if response.is_success() => {
                        let rows = response
                            .result
                            .as_ref()
                            .and_then(|result| result.get("workers"))
                            .and_then(|workers| workers.as_array());
                        UserProjection::from_miner_snapshot(
                            &auth.claim,
                            rows.map(|rows| rows.as_slice()),
                            None,
                            observed_at,
                        )
                    }
                    // A closed outcome string only; the wire error text is
                    // never forwarded to a user response.
                    Ok(response) => UserProjection::from_miner_snapshot(
                        &auth.claim,
                        None,
                        Some(&format!(
                            "miner snapshot poll returned {}",
                            response.outcome
                        )),
                        observed_at,
                    ),
                    Err(_) => UserProjection::from_miner_snapshot(
                        &auth.claim,
                        None,
                        Some("miner snapshot poll failed"),
                        observed_at,
                    ),
                }
            }
            None => UserProjection::from_miner_snapshot(
                &auth.claim,
                None,
                Some("miner control pipe is unavailable (not started, adopted or stopped)"),
                observed_at,
            ),
        }
    }

    /// The worker projections of every user claim (the operator view).
    pub fn user_projections(&self) -> Vec<UserProjection> {
        let Some(store) = &self.access else {
            return Vec::new();
        };
        store
            .claims()
            .iter()
            .filter(|claim| claim.is_user())
            .map(|claim| {
                self.user_projection(&AuthContext {
                    principal: claim.principal.clone(),
                    claim: claim.clone(),
                })
            })
            .collect()
    }

    /// Resolve a relay role to its declared listener address
    /// (`<role>.api` in the runtime's declared addresses). A browser-supplied
    /// target is never resolved; an undeclared role is `None`.
    pub fn relay_target(&self, role: &str) -> Option<String> {
        self.addresses.get(&format!("{role}.api")).cloned()
    }

    pub fn entity_states(&self) -> Vec<EntitySnapshot> {
        let supervisor = self.supervisor.lock().unwrap();
        let controls: BTreeSet<MinerId> = self
            .adapters
            .miners
            .lock()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        let manifest = self.manifest.lock().unwrap().clone();
        let state = self.state.lock().unwrap();
        let mut entities = Vec::new();
        for node in &self.scenario.nodes {
            entities.push(entity_for_process(
                &supervisor,
                &manifest,
                "node",
                node.id.as_str(),
            ));
        }
        for pool in &self.scenario.pools {
            entities.push(EntitySnapshot {
                kind: "pool".to_string(),
                id: pool.id.to_string(),
                status: "declared".to_string(),
                detail: format!("network {}", pool.network_id),
                miner_id: None,
            });
        }
        for front in &self.scenario.fronts {
            entities.push(entity_for_process(
                &supervisor,
                &manifest,
                "front",
                front.id.as_str(),
            ));
        }
        for miner in &self.scenario.miners {
            let paused = state.paused_miners.contains(&miner.id)
                || (!miner.workers.is_empty()
                    && miner
                        .workers
                        .iter()
                        .all(|worker| state.paused_workers.contains(&worker.id)));
            let status = if state.removed.contains(miner.id.as_str()) {
                "removed".to_string()
            } else if let Some(record) = supervisor.record("miner", miner.id.as_str()) {
                if record.state == ProcessState::AdoptedControlLost {
                    "control_lost".to_string()
                } else if !controls.contains(&miner.id) {
                    "unknown".to_string()
                } else if paused {
                    "paused".to_string()
                } else {
                    record.state.as_str().to_string()
                }
            } else if !controls.contains(&miner.id) {
                "unknown".to_string()
            } else if paused {
                "paused".to_string()
            } else {
                "running".to_string()
            };
            entities.push(EntitySnapshot {
                kind: "miner".to_string(),
                id: miner.id.to_string(),
                status,
                detail: format!("{} connection(s)", miner.connections.len()),
                miner_id: None,
            });
            for worker in &miner.workers {
                let paused = state.paused_miners.contains(&miner.id)
                    || state.paused_workers.contains(&worker.id);
                let status = if !controls.contains(&miner.id) {
                    "unknown"
                } else if paused {
                    "paused"
                } else {
                    "running"
                };
                entities.push(EntitySnapshot {
                    kind: "worker".to_string(),
                    id: worker.id.to_string(),
                    status: status.to_string(),
                    detail: format!("miner {}", miner.id),
                    miner_id: Some(miner.id.clone()),
                });
            }
        }
        for (miner_id, extra) in &state.extra_miners {
            entities.push(EntitySnapshot {
                kind: "miner".to_string(),
                id: miner_id.to_string(),
                status: if !controls.contains(miner_id) {
                    "unknown".to_string()
                } else {
                    supervisor
                        .record("miner", miner_id.as_str())
                        .map(|record| record.state.as_str().to_string())
                        .unwrap_or_else(|| "declared".to_string())
                },
                detail: format!("dynamic {}", extra.implementation),
                miner_id: None,
            });
        }
        for identity in &self.scenario.identities {
            entities.push(EntitySnapshot {
                kind: "identity".to_string(),
                id: identity.id.to_string(),
                status: "private".to_string(),
                detail: "identity material is in the private artifact store".to_string(),
                miner_id: None,
            });
        }
        for redis in &self.resources.redis_instances {
            entities.push(EntitySnapshot {
                kind: "redis".to_string(),
                id: redis.id.clone(),
                status: "allocated".to_string(),
                detail: format!("port {}", redis.port),
                miner_id: None,
            });
        }
        for pg in &self.resources.postgres_databases {
            entities.push(EntitySnapshot {
                kind: "postgres".to_string(),
                id: pg.id.clone(),
                status: "allocated".to_string(),
                detail: format!("database {} role {}", pg.database, pg.role),
                miner_id: None,
            });
        }
        entities
    }

    pub fn submit(&self, command: LabCommand) -> CommandOutcome {
        // In-process submissions (CLI, runtime phases) are the harness
        // principal: bound in the expanded journal like every other
        // principal, but exempt from the HTTP claim scope because the claim
        // store is the browser-facing authority, not the harness. Legacy
        // runs keep the kind-only hash path unchanged.
        let principal = if self.scenario.expanded_access {
            Some(PRINCIPAL_CONTROLLER)
        } else {
            None
        };
        self.submit_as(command, principal)
    }

    /// Submit a command authenticated by an access claim (the HTTP path).
    ///
    /// The claim, not the payload, is the authority: the effective actor,
    /// miner and worker ids are bound server-side from the stored claim and
    /// checked before any stored-outcome lookup, journal entry or effect.
    /// A scope failure is a real closed outcome with no journal effect.
    pub fn submit_authenticated(&self, auth: &AuthContext, command: LabCommand) -> CommandOutcome {
        if let Err(detail) = crate::access::check_command_scope(&auth.claim, &command) {
            return crate::access::closed_rejection(&command.command_id, &detail);
        }
        if let Some(identity) = crate::wallet_control::identity(&command.kind) {
            let binding = self.wallet_registry.lock().unwrap().get(identity);
            if !auth.claim.is_operator()
                && (binding
                    .as_ref()
                    .is_some_and(|b| auth.claim.pool_id.as_ref() != Some(&b.pool_id))
                    || !crate::access::derive_user_bindings(&self.scenario).is_ok_and(|bindings| {
                        bindings.iter().any(|b| {
                            &b.identity_id == identity
                                && auth.claim.pool_id.as_ref() == Some(&b.pool_id)
                        })
                    }))
            {
                return crate::access::closed_rejection(
                    &command.command_id,
                    "wallet is outside this claim",
                );
            }
        }
        self.submit_as(command, Some(auth.principal.as_str()))
    }

    fn submit_as(&self, command: LabCommand, principal: Option<&str>) -> CommandOutcome {
        // Serialize begin/apply/finish for the whole run. This is the
        // log-ordered precedence boundary; check-then-act races cannot create
        // two processes for one entity.
        // Only the closed acceptance-control vocabulary bypasses a held
        // payment's effect mutex. Its transitions serialize in CommandLog.
        let mut command_guard = if matches!(command.kind, CommandKind::TrafficAcceptance { .. }) {
            None
        } else {
            Some(self.command_mutex.lock().unwrap())
        };
        // A recorded read-only run may serve an already durable wallet result,
        // but cannot turn a denied observation into cancellation of old funds.
        if self.read_only
            && command.run_id == self.run_id
            && crate::wallet_control::identity(&command.kind).is_some()
        {
            match self.command_log.lookup_scoped(&command, principal) {
                Some(BeginOutcome::Existing(outcome) | BeginOutcome::InFlight(outcome)) => {
                    return self.resolve_wallet_replay(&command, principal, outcome);
                }
                Some(BeginOutcome::Denied(outcome) | BeginOutcome::Conflict(outcome)) => {
                    return outcome
                }
                _ => {}
            }
        }
        if self.read_only {
            return CommandOutcome::new(
                command.command_id,
                OutcomeState::RejectedPrecondition,
                "attached run is read-only; use the live run or attach for adoption to control it",
            );
        }
        if command.run_id != self.run_id {
            return CommandOutcome::new(
                command.command_id,
                OutcomeState::RejectedPrecondition,
                format!(
                    "command run_id {} does not match {}",
                    command.run_id, self.run_id
                ),
            );
        }
        if crate::wallet_control::identity(&command.kind).is_some()
            && !self.command_log.wallet_resolutions_available()
        {
            return match self.command_log.lookup_scoped(&command, principal) {
                Some(BeginOutcome::Existing(outcome))
                    if !matches!(outcome.state, OutcomeState::Running | OutcomeState::Unknown) =>
                {
                    outcome
                }
                Some(BeginOutcome::Denied(outcome) | BeginOutcome::Conflict(outcome)) => outcome,
                _ => CommandOutcome::new(
                    command.command_id.clone(),
                    OutcomeState::Unknown,
                    "wallet journal unavailable; retain original command",
                ),
            };
        }
        let begin = match principal {
            // Expanded access: the journal hash is bound to the
            // authenticated principal and the run before any stored outcome
            // can be returned.
            Some(principal) => self.command_log.begin_scoped(&command, principal),
            None => self.command_log.begin(&command),
        };
        match begin {
            Ok(BeginOutcome::Existing(mut outcome)) => {
                if crate::wallet_control::identity(&command.kind).is_some() {
                    return self.resolve_wallet_replay(&command, principal, outcome);
                }
                if outcome.journal_seq == 0 {
                    outcome.journal_seq = self.command_log.last_seq();
                }
                return outcome;
            }
            Ok(BeginOutcome::InFlight(mut outcome)) => {
                if crate::wallet_control::identity(&command.kind).is_some() {
                    return self.resolve_wallet_replay(&command, principal, outcome);
                }
                outcome.journal_seq = self.command_log.last_seq();
                return outcome;
            }
            Ok(BeginOutcome::Conflict(mut outcome)) => {
                outcome.journal_seq = self.command_log.last_seq();
                return outcome;
            }
            // Closed denial: a different principal presented the id, or the
            // stored entry's hash scheme is missing or unknown. No journal
            // write, no evidence event, no outcome leak.
            Ok(BeginOutcome::Denied(outcome)) => return outcome,
            Ok(BeginOutcome::New) => {}
            Err(error) => {
                if crate::wallet_control::identity(&command.kind).is_some() {
                    return CommandOutcome::new(
                        command.command_id,
                        OutcomeState::Unknown,
                        "wallet intent durability unknown; retain original command",
                    );
                }
                return CommandOutcome::new(
                    command.command_id,
                    OutcomeState::Failed,
                    format!("command log write failed: {error}"),
                );
            }
        }
        let journal_seq = self.command_log.last_seq();
        if matches!(command.kind, CommandKind::BootstrapFund { .. }) {
            let outcome = self.begin_traffic_bootstrap(&command);
            if outcome.state == OutcomeState::Running {
                command_guard.take();
                return outcome;
            }
            let _ = self.command_log.finish(&command.command_id, &outcome);
            return outcome;
        }
        // A height wait is an observation, not a mutating effect. Release the
        // effect lock after the durable begin so a stop can cancel it without
        // waiting for the declared timeout. Mutating commands remain fully
        // serialized until finish.
        if matches!(command.kind, CommandKind::WaitHeight { .. }) {
            command_guard.take();
        }
        let mut outcome = self.apply(&command);
        if crate::wallet_control::identity(&command.kind).is_some() {
            return self.persist_wallet_resolution(&command, principal, outcome);
        }
        if outcome.journal_seq == 0 {
            outcome.journal_seq = journal_seq;
        }
        let _ = self.command_log.finish(&command.command_id, &outcome);
        self.evidence.append(
            SourceId::trusted("command"),
            format!("command-{}", command.command_id),
            EvidenceActor::harness(),
            Some(command.command_id.clone()),
            EvidencePayload::Command(CommandObservation {
                command_id: command.command_id.clone(),
                state: outcome.state.as_str().to_string(),
                detail: outcome.detail.clone(),
                journal_seq: outcome.journal_seq,
            }),
        );
        // `stop` writes the shareable artifacts before this command outcome is
        // journaled. Refresh them once so the recorded stop result is the one
        // that actually resolved, not the pre-finish "unknown" placeholder.
        if matches!(command.kind, CommandKind::StopRun { .. }) {
            let _ = self.write_artifacts();
        }
        outcome
    }

    fn resolve_wallet_replay(
        &self,
        command: &LabCommand,
        principal: Option<&str>,
        previous: CommandOutcome,
    ) -> CommandOutcome {
        if !matches!(
            previous.state,
            OutcomeState::Running | OutcomeState::Unknown
        ) {
            return previous;
        }
        if self.read_only {
            return CommandOutcome::new(command.command_id.clone(), OutcomeState::Unknown,
                "read-only observation cannot resolve this wallet operation; retain original command");
        }
        let binding = crate::wallet_control::identity(&command.kind)
            .and_then(|id| self.wallet_registry.lock().unwrap().get(id));
        let mut outcome = crate::wallet_control::apply(binding, command, true);
        if matches!(outcome.state, OutcomeState::Running | OutcomeState::Unknown) {
            // Observation failure/progress is not new terminal agent evidence.
            // Keep the durable initial intent/outcome unchanged on every read.
            outcome.journal_seq = previous.journal_seq;
            return outcome;
        }
        self.persist_wallet_resolution(command, principal, outcome)
    }

    fn persist_wallet_resolution(
        &self,
        command: &LabCommand,
        principal: Option<&str>,
        outcome: CommandOutcome,
    ) -> CommandOutcome {
        match self
            .command_log
            .supersede_nonterminal(command, principal, &outcome)
        {
            Ok((durable, changed)) => {
                if changed {
                    self.evidence.append(
                        SourceId::trusted("command"),
                        format!(
                            "wallet-command-{}-resolution-{}",
                            command.command_id, durable.journal_seq
                        ),
                        EvidenceActor::harness(),
                        Some(command.command_id.clone()),
                        EvidencePayload::Command(CommandObservation {
                            command_id: command.command_id.clone(),
                            state: durable.state.as_str().to_string(),
                            detail: durable.detail.clone(),
                            journal_seq: durable.journal_seq,
                        }),
                    );
                    let _ = self.write_artifacts();
                }
                durable
            }
            Err(_) => CommandOutcome::new(
                command.command_id.clone(),
                OutcomeState::Unknown,
                "wallet result durability unknown; retain original command",
            ),
        }
    }

    fn apply(&self, command: &LabCommand) -> CommandOutcome {
        let id = command.command_id.clone();
        let running = self.state.lock().unwrap().started;
        if !running
            && !matches!(
                command.kind,
                CommandKind::StartRun { .. } | CommandKind::StopRun { .. }
            )
        {
            return CommandOutcome::new(
                id,
                OutcomeState::RejectedPrecondition,
                "run is not running",
            );
        }
        if let Some(stage) = self.manifest.lock().unwrap().traffic_stage {
            if let Err(reason) = crate::traffic::runtime::admitted(stage, &command.kind) {
                return CommandOutcome::new(id, OutcomeState::RejectedPrecondition, reason);
            }
        }
        match &command.kind {
            CommandKind::AcceptanceVersionRejectionV1 { .. } | CommandKind::TrafficAcceptance { .. } |
            CommandKind::BootstrapFund { .. } | CommandKind::BootstrapControl { .. }
            | CommandKind::PauseTxDriver { .. } | CommandKind::ResumeTxDriver { .. }
            | CommandKind::RestartTxDriver { .. } | CommandKind::SendUserPayment { .. } => self.apply_traffic(command),
            CommandKind::WalletRefresh { identity_id, .. }
            | CommandKind::WalletOffboard { identity_id, .. }
            | CommandKind::WalletReadInputs { identity_id } => {
                if !running { return CommandOutcome::new(id, OutcomeState::RejectedPrecondition, "run is not running"); }
                crate::wallet_control::apply(self.wallet_registry.lock().unwrap().get(identity_id), command, false)
            }
            CommandKind::StartRun { .. } => match self.start() {
                Ok(()) => CommandOutcome::new(id, OutcomeState::Succeeded, "run started"),
                Err(error) => CommandOutcome::new(id, OutcomeState::RejectedPrecondition, error.to_string()),
            },
            CommandKind::StopRun { .. } => match self.stop() {
                Ok(()) => CommandOutcome::new(
                    id,
                    OutcomeState::Succeeded,
                    format!("run stopped (final state {})", self.manifest_state().as_str()),
                ),
                Err(error) if error.code() == "stop_precondition" => {
                    CommandOutcome::new(id, OutcomeState::RejectedPrecondition, error.to_string())
                }
                Err(error) => CommandOutcome::new(id, OutcomeState::Failed, error.to_string()),
            },
            _ if !running => CommandOutcome::new(
                id,
                OutcomeState::RejectedPrecondition,
                "run is not running",
            ),
            CommandKind::AddMiner {
                miner_id,
                implementation,
                connections,
                workers,
            } => {
                if self.scenario.miners.iter().any(|m| &m.id == miner_id)
                    || self.state.lock().unwrap().extra_miners.contains_key(miner_id)
                    || self.state.lock().unwrap().removed.contains(miner_id.as_str())
                {
                    return CommandOutcome::new(
                        id,
                        OutcomeState::RejectedPrecondition,
                        format!("miner {miner_id} already exists or was removed"),
                    );
                }
                if let Err(error) = self.launch_miner(miner_id, implementation, connections, workers)
                {
                    return CommandOutcome::new(id, OutcomeState::Failed, error.to_string());
                }
                self.state.lock().unwrap().extra_miners.insert(
                    miner_id.clone(),
                    ExtraMiner {
                        implementation: implementation.clone(),
                        connections: connections.clone(),
                        workers: workers.clone(),
                    },
                );
                CommandOutcome::new(id, OutcomeState::Succeeded, format!("added miner {miner_id}"))
            }
            CommandKind::RemoveMiner { miner_id } => {
                let exists = self.scenario.miners.iter().any(|m| &m.id == miner_id)
                    || self.state.lock().unwrap().extra_miners.contains_key(miner_id);
                if !exists {
                    return CommandOutcome::new(
                        id,
                        OutcomeState::RejectedPrecondition,
                        format!("miner {miner_id} does not exist"),
                    );
                }
                if let Some(control) = self.adapters.miners.lock().unwrap().get(miner_id) {
                    let _ = control.stop();
                }
                let _ = self
                    .supervisor
                    .lock()
                    .unwrap()
                    .stop("miner", miner_id.as_str(), Duration::from_secs(2));
                self.adapters.miners.lock().unwrap().remove(miner_id);
                self.state.lock().unwrap().extra_miners.remove(miner_id);
                self.state
                    .lock()
                    .unwrap()
                    .removed
                    .insert(miner_id.to_string());
                CommandOutcome::new(id, OutcomeState::Succeeded, format!("removed miner {miner_id}"))
            }
            CommandKind::AddConnection {
                miner_id,
                connection,
            } => {
                if let Err(detail) = crate::wallet_bridge::require_ready(self, std::slice::from_ref(connection)) {
                    return CommandOutcome::new(id, OutcomeState::RejectedPrecondition, detail);
                }
                let Some(spec) = self.miner_connections(miner_id) else {
                    return CommandOutcome::new(id, OutcomeState::RejectedPrecondition, "unknown miner");
                };
                if spec.iter().any(|c| c.id == connection.id) {
                    return CommandOutcome::new(id, OutcomeState::RejectedPrecondition, "connection exists");
                }
                let workers = self.miner_workers(miner_id).unwrap_or_default();
                let payload = match self.connection_payload(connection, &workers) {
                    Ok(payload) => payload,
                    Err(error) => {
                        return CommandOutcome::new(
                            id,
                            OutcomeState::RejectedPrecondition,
                            error.to_string(),
                        );
                    }
                };
                let miners = self.adapters.miners.lock().unwrap();
                let Some(control) = miners.get(miner_id) else {
                    return CommandOutcome::new(
                        id,
                        OutcomeState::RejectedPrecondition,
                        format!("miner {miner_id} control pipe is unavailable (adopted or stopped)"),
                    );
                };
                if let Err((state, detail)) =
                    self.send_miner(control, MinerOperation::AddConnection, payload)
                {
                    return CommandOutcome::new(id, state, detail);
                }
                if let Some(extra) = self.state.lock().unwrap().extra_miners.get_mut(miner_id) {
                    extra.connections.push(connection.clone());
                }
                CommandOutcome::new(id, OutcomeState::Succeeded, "connection added")
            }
            CommandKind::RemoveConnection {
                miner_id,
                connection_id,
            } => {
                let Some(spec) = self.miner_connections(miner_id) else {
                    return CommandOutcome::new(id, OutcomeState::RejectedPrecondition, "unknown miner");
                };
                if !spec.iter().any(|c| &c.id == connection_id) {
                    return CommandOutcome::new(id, OutcomeState::RejectedPrecondition, "connection missing");
                }
                let miners = self.adapters.miners.lock().unwrap();
                let Some(control) = miners.get(miner_id) else {
                    return CommandOutcome::new(
                        id,
                        OutcomeState::RejectedPrecondition,
                        format!("miner {miner_id} control pipe is unavailable (adopted or stopped)"),
                    );
                };
                let payload = serde_json::json!({ "connection_id": connection_id });
                if let Err((state, detail)) =
                    self.send_miner(control, MinerOperation::RemoveConnection, payload)
                {
                    return CommandOutcome::new(id, state, detail);
                }
                if let Some(extra) = self.state.lock().unwrap().extra_miners.get_mut(miner_id) {
                    extra.connections.retain(|c| &c.id != connection_id);
                }
                CommandOutcome::new(id, OutcomeState::Succeeded, "connection removed")
            }
            CommandKind::AddWorker { miner_id, worker } => {
                let Some(workers) = self.miner_workers(miner_id) else {
                    return CommandOutcome::new(id, OutcomeState::RejectedPrecondition, "unknown miner");
                };
                if workers.iter().any(|w| w.id == worker.id) {
                    return CommandOutcome::new(id, OutcomeState::RejectedPrecondition, "worker exists");
                }
                // Protocol version 1 defines workers inside add_connection;
                // there is no independent add_worker operation. Adding a new
                // worker to a live connection is not silently faked.
                CommandOutcome::new(
                    id,
                    OutcomeState::RejectedPrecondition,
                    "spc-cpu-miner-control v1 has no add_worker operation; recreate the connection through add_connection",
                )
            }
            CommandKind::SetWorkerBudget {
                miner_id,
                worker_id,
                hashes_per_second,
                total_hashes,
            } => {
                let Some(workers) = self.miner_workers(miner_id) else {
                    return CommandOutcome::new(id, OutcomeState::RejectedPrecondition, "unknown miner");
                };
                if !workers.iter().any(|w| &w.id == worker_id) {
                    return CommandOutcome::new(id, OutcomeState::RejectedPrecondition, "unknown worker");
                }
                let miners = self.adapters.miners.lock().unwrap();
                let Some(control) = miners.get(miner_id) else {
                    return CommandOutcome::new(
                        id,
                        OutcomeState::RejectedPrecondition,
                        format!("miner {miner_id} control pipe is unavailable (adopted or stopped)"),
                    );
                };
                let payload = serde_json::json!({
                    "worker_id": worker_id,
                    "budget": {
                        "hashes_per_second": wide_json(*hashes_per_second),
                        "total_attempts": wide_json(*total_hashes),
                    }
                });
                if let Err((state, detail)) =
                    self.send_miner(control, MinerOperation::SetWorkerBudget, payload)
                {
                    return CommandOutcome::new(id, state, detail);
                }
                self.state.lock().unwrap().worker_budgets.insert(
                    worker_id.clone(),
                    BudgetSpec {
                        hashes_per_second: *hashes_per_second,
                        total_hashes: *total_hashes,
                    },
                );
                CommandOutcome::new(id, OutcomeState::Succeeded, "worker budget updated")
            }
            CommandKind::PauseWorkers { miner_id, workers } => {
                let Some(spec) = self.miner_workers(miner_id) else {
                    return CommandOutcome::new(id, OutcomeState::RejectedPrecondition, "unknown miner");
                };
                let targets: Vec<WorkerId> = if workers.is_empty() {
                    spec.iter().map(|worker| worker.id.clone()).collect()
                } else {
                    workers.clone()
                };
                if targets.iter().any(|worker| !spec.iter().any(|w| &w.id == worker)) {
                    return CommandOutcome::new(id, OutcomeState::RejectedPrecondition, "unknown worker");
                }
                if targets.is_empty() {
                    return CommandOutcome::new(
                        id,
                        OutcomeState::RejectedPrecondition,
                        "pause_workers requires at least one worker id",
                    );
                }
                let miners = self.adapters.miners.lock().unwrap();
                let Some(control) = miners.get(miner_id) else {
                    return CommandOutcome::new(
                        id,
                        OutcomeState::RejectedPrecondition,
                        format!("miner {miner_id} control pipe is unavailable (adopted or stopped)"),
                    );
                };
                let payload = serde_json::json!({ "worker_ids": &targets });
                if let Err((state, detail)) =
                    self.send_miner(control, MinerOperation::PauseWorkers, payload)
                {
                    return CommandOutcome::new(id, state, detail);
                }
                let mut state = self.state.lock().unwrap();
                if workers.is_empty() {
                    state.paused_miners.insert(miner_id.clone());
                }
                state.paused_workers.extend(targets);
                CommandOutcome::new(id, OutcomeState::Succeeded, "workers paused")
            }
            CommandKind::ResumeWorkers { miner_id, workers } => {
                let Some(spec) = self.miner_workers(miner_id) else {
                    return CommandOutcome::new(id, OutcomeState::RejectedPrecondition, "unknown miner");
                };
                let targets: Vec<WorkerId> = if workers.is_empty() {
                    spec.iter().map(|worker| worker.id.clone()).collect()
                } else {
                    workers.clone()
                };
                if targets.is_empty() {
                    return CommandOutcome::new(
                        id,
                        OutcomeState::RejectedPrecondition,
                        "resume_workers requires at least one worker id",
                    );
                }
                let miners = self.adapters.miners.lock().unwrap();
                let Some(control) = miners.get(miner_id) else {
                    return CommandOutcome::new(
                        id,
                        OutcomeState::RejectedPrecondition,
                        format!("miner {miner_id} control pipe is unavailable (adopted or stopped)"),
                    );
                };
                let payload = serde_json::json!({ "worker_ids": &targets });
                if let Err((state, detail)) =
                    self.send_miner(control, MinerOperation::ResumeWorkers, payload)
                {
                    return CommandOutcome::new(id, state, detail);
                }
                let mut state = self.state.lock().unwrap();
                state.paused_miners.remove(miner_id);
                for worker in targets {
                    state.paused_workers.remove(&worker);
                }
                CommandOutcome::new(id, OutcomeState::Succeeded, "workers resumed")
            }
            CommandKind::SwitchPool {
                miner_id,
                connection_id,
                front_id,
                identity_id,
            } => {
                if !self.scenario.fronts.iter().any(|f| &f.id == front_id) {
                    return CommandOutcome::new(id, OutcomeState::RejectedPrecondition, "unknown front");
                }
                let endpoint = match self.front_endpoint(front_id) {
                    Ok(endpoint) => endpoint,
                    Err(error) => {
                        return CommandOutcome::new(id, OutcomeState::RejectedPrecondition, error.to_string());
                    }
                };
                let identity = match self.identity_address(identity_id) {
                    Ok(identity) => identity,
                    Err(error) => {
                        return CommandOutcome::new(id, OutcomeState::RejectedPrecondition, error.to_string());
                    }
                };
                let authority = match self.authority_public(front_id) {
                    Ok(authority) => authority,
                    Err(error) => {
                        return CommandOutcome::new(id, OutcomeState::RejectedPrecondition, error.to_string());
                    }
                };
                let payload = serde_json::json!({
                    "connection_id": connection_id,
                    "endpoint": endpoint,
                    "identity": identity,
                    "authority_public_key": authority,
                    "min_extranonce_size": 4,
                });
                let miners = self.adapters.miners.lock().unwrap();
                let Some(control) = miners.get(miner_id) else {
                    return CommandOutcome::new(
                        id,
                        OutcomeState::RejectedPrecondition,
                        format!("miner {miner_id} control pipe is unavailable (adopted or stopped)"),
                    );
                };
                if let Err((state, detail)) =
                    self.send_miner(control, MinerOperation::SwitchPool, payload)
                {
                    return CommandOutcome::new(id, state, detail);
                }
                CommandOutcome::new(id, OutcomeState::Succeeded, "connection switched")
            }
            CommandKind::WaitHeight {
                node_id,
                height,
                timeout_secs,
            } => self.wait_for_height(id, node_id, *height, *timeout_secs),
            CommandKind::Assert { assertion_id } => {
                let Some(spec) = self
                    .scenario
                    .assertions
                    .iter()
                    .find(|assertion| &assertion.id == assertion_id)
                else {
                    return CommandOutcome::new(id, OutcomeState::RejectedPrecondition, "unknown assertion");
                };
                match self.evidence.evaluate_assertion(spec) {
                    AssertionEvaluation::Passed => {
                        CommandOutcome::new(id, OutcomeState::Succeeded, "assertion passed")
                    }
                    AssertionEvaluation::Failed(detail) => {
                        CommandOutcome::new(id, OutcomeState::Failed, detail)
                    }
                    AssertionEvaluation::Inconclusive(detail) => {
                        CommandOutcome::new(id, OutcomeState::Unknown, format!("inconclusive: {detail}"))
                    }
                    AssertionEvaluation::BlockedOnHeight(detail) => {
                        CommandOutcome::new(id, OutcomeState::BlockedOnHeight, detail)
                    }
                    AssertionEvaluation::TimedOut(detail) => {
                        CommandOutcome::new(id, OutcomeState::Failed, format!("timed out: {detail}"))
                    }
                }
            }
            CommandKind::KillRole { role, entity_id } => {
                match self.supervisor.lock().unwrap().stop(role, entity_id, Duration::from_secs(2)) {
                    Ok(true) => CommandOutcome::new(id, OutcomeState::Succeeded, format!("killed {role}/{entity_id}")),
                    Ok(false) => CommandOutcome::new(id, OutcomeState::RejectedPrecondition, format!("{role}/{entity_id} is not running")),
                    Err(error) => CommandOutcome::new(id, OutcomeState::Failed, error.to_string()),
                }
            }
            CommandKind::RestartRole { .. } => CommandOutcome::new(
                id,
                OutcomeState::Failed,
                "restart is not implemented in LP-3; start a new run so the manifest and process identity are fresh",
            ),
            CommandKind::PartitionLink { link_id } => self.link_command(link_id, true, id),
            CommandKind::HealLink { link_id } => self.link_command(link_id, false, id),
            CommandKind::InjectTx { .. } if self.scenario.traffic.is_some() => self.apply_traffic(command),
            CommandKind::InjectTx { adapter, .. } => CommandOutcome::new(
                id,
                OutcomeState::RejectedPrecondition,
                format!("funding/mempool adapter {adapter:?} is not configured in LP-3"),
            ),
        }
    }

    fn link_command(
        &self,
        link_id: &LinkId,
        partition: bool,
        id: crate::ids::CommandId,
    ) -> CommandOutcome {
        let Some(link) = self.scenario.links.iter().find(|link| &link.id == link_id) else {
            return CommandOutcome::new(id, OutcomeState::RejectedPrecondition, "unknown link");
        };
        let policy = if partition {
            LinkPolicy::Partition
        } else {
            LinkPolicy::Normal
        };
        if partition && !self.link_backend.supports_sustained_fault() {
            return CommandOutcome::new(
                id,
                OutcomeState::RejectedPrecondition,
                format!(
                    "link backend {} cannot sustain a partition; LP-4 netns backend is required",
                    self.link_backend.name()
                ),
            );
        }
        if let Err(error) = self.link_backend.apply(link, &policy) {
            return CommandOutcome::new(id, OutcomeState::Failed, error.to_string());
        }
        self.evidence.append(
            SourceId::trusted("link"),
            format!("link-{link_id}"),
            EvidenceActor::new(ActorKind::Observer, "link-backend"),
            None,
            EvidencePayload::LinkPolicy {
                link_id: link_id.to_string(),
                policy: policy.to_string(),
            },
        );
        CommandOutcome::new(
            id,
            OutcomeState::Succeeded,
            format!("link {link_id} -> {policy}"),
        )
    }

    fn wait_for_height(
        &self,
        id: crate::ids::CommandId,
        node: &NodeId,
        height: u64,
        timeout_secs: u64,
    ) -> CommandOutcome {
        let nodes = self.adapters.nodes.lock().unwrap();
        let control = match nodes.get(node) {
            Some(control) => control,
            None => {
                return CommandOutcome::new(
                    id,
                    OutcomeState::Failed,
                    format!("node {node} has no control adapter"),
                );
            }
        };
        let producers = self.chain_producers(node);
        if producers.is_empty() {
            return CommandOutcome::new(
                id,
                OutcomeState::BlockedOnHeight,
                format!("no miner can advance node {node}; height {height} is blocked"),
            );
        }
        if producers.iter().all(|miner| self.miner_paused(miner)) {
            return CommandOutcome::new(
                id,
                OutcomeState::BlockedOnHeight,
                format!("all miners that can advance node {node} are paused"),
            );
        }
        let timeout = Duration::from_secs(timeout_secs.max(1));
        let deadline = Instant::now() + timeout;
        let mut last_detail = format!("node {node} has not been probed");
        loop {
            if !matches!(
                self.manifest_state(),
                RunState::Running | RunState::Preparing
            ) {
                return CommandOutcome::new(
                    id,
                    OutcomeState::Cancelled,
                    "height wait cancelled because the run left the active state",
                );
            }
            match control.height() {
                Ok(actual) if actual >= height => {
                    return CommandOutcome::new(
                        id,
                        OutcomeState::Succeeded,
                        format!("node {node} reached height {actual} (target {height})"),
                    );
                }
                Ok(actual) => {
                    last_detail = format!("node {node} is at height {actual}, target {height}")
                }
                Err(error) => last_detail = format!("node {node} height probe failed: {error}"),
            }
            if Instant::now() >= deadline {
                return CommandOutcome::new(
                    id,
                    OutcomeState::Failed,
                    format!("height wait timed out: {last_detail}"),
                );
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn chain_producers(&self, node: &NodeId) -> Vec<MinerId> {
        let mut producers = Vec::new();
        let removed = self.state.lock().unwrap().removed.clone();
        let controls: BTreeSet<MinerId> = self
            .adapters
            .miners
            .lock()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        for miner in &self.scenario.miners {
            if removed.contains(miner.id.as_str()) || !controls.contains(&miner.id) {
                continue;
            }
            if miner.connections.iter().any(|connection| {
                self.scenario
                    .fronts
                    .iter()
                    .find(|front| front.id == connection.front_id)
                    .and_then(|front| {
                        self.scenario
                            .pools
                            .iter()
                            .find(|pool| pool.id == front.pool_id)
                    })
                    .map(|pool| pool.node_ids.contains(node))
                    .unwrap_or(false)
            }) {
                producers.push(miner.id.clone());
            }
        }
        let extras = self.state.lock().unwrap().extra_miners.clone();
        for (miner_id, extra) in &extras {
            if removed.contains(miner_id.as_str()) || !controls.contains(miner_id) {
                continue;
            }
            if extra.connections.iter().any(|connection| {
                self.scenario
                    .fronts
                    .iter()
                    .find(|front| front.id == connection.front_id)
                    .and_then(|front| {
                        self.scenario
                            .pools
                            .iter()
                            .find(|pool| pool.id == front.pool_id)
                    })
                    .map(|pool| pool.node_ids.contains(node))
                    .unwrap_or(false)
            }) {
                producers.push(miner_id.clone());
            }
        }
        producers
    }

    fn miner_paused(&self, miner: &MinerId) -> bool {
        let (paused_miner, removed) = {
            let state = self.state.lock().unwrap();
            (
                state.paused_miners.contains(miner),
                state.removed.contains(miner.as_str()),
            )
        };
        if paused_miner || removed {
            return true;
        }
        let workers = self.miner_workers(miner).unwrap_or_default();
        let state = self.state.lock().unwrap();
        !workers.is_empty()
            && workers
                .iter()
                .all(|worker| state.paused_workers.contains(&worker.id))
    }

    fn miner_connections(&self, miner: &MinerId) -> Option<Vec<ConnectionSpec>> {
        if let Some(spec) = self.scenario.miners.iter().find(|m| &m.id == miner) {
            return Some(spec.connections.clone());
        }
        self.state
            .lock()
            .unwrap()
            .extra_miners
            .get(miner)
            .map(|extra| extra.connections.clone())
    }

    fn miner_workers(&self, miner: &MinerId) -> Option<Vec<WorkerSpec>> {
        if let Some(spec) = self.scenario.miners.iter().find(|m| &m.id == miner) {
            return Some(spec.workers.clone());
        }
        self.state
            .lock()
            .unwrap()
            .extra_miners
            .get(miner)
            .map(|extra| extra.workers.clone())
    }

    pub fn stop(&self) -> Result<(), EngineError> {
        if let Some((_, control)) = self.traffic.lock().unwrap().operation.as_ref() {
            control.cancel();
        }
        self.close_traffic_gates();
        let state_before = self.manifest_state();
        let has_records = !self.supervisor.lock().unwrap().records().is_empty();
        if !has_records {
            return match state_before {
                RunState::Stopped | RunState::CompletedWithErrors => Ok(()),
                RunState::Created => Err(EngineError::new(
                    "stop_precondition",
                    "run was created but never started; there is nothing to stop",
                )),
                RunState::Failed => Err(EngineError::new(
                    "stop_precondition",
                    "run failed before any owned process was recorded; stop is not a success postcondition",
                )),
                _ => Err(EngineError::new(
                    "stop_precondition",
                    format!("run state {} has no owned process records", state_before.as_str()),
                )),
            };
        }
        self.set_manifest_state(RunState::Stopping)?;
        self.emit_run_state("stopping");
        self.adapters.miners.lock().unwrap().clear();
        self.supervisor
            .lock()
            .unwrap()
            .stop_all(Duration::from_secs(2));
        {
            let mut state = self.state.lock().unwrap();
            state.started = false;
        }
        self.sync_processes()?;
        let all_verified = {
            let supervisor = self.supervisor.lock().unwrap();
            supervisor
                .records()
                .iter()
                .all(|record| matches!(record.state, ProcessState::Stopped | ProcessState::Exited))
        };
        let final_state = if all_verified {
            RunState::Stopped
        } else {
            RunState::CompletedWithErrors
        };
        self.set_manifest_state(final_state)?;
        self.emit_run_state(final_state.as_str());
        let artifacts = self.write_artifacts();
        if let Some(lock) = self.lock.lock().unwrap().take() {
            lock.release();
        }
        artifacts
    }

    fn sync_processes(&self) -> Result<(), EngineError> {
        let records = self.supervisor.lock().unwrap().records();
        let mut manifest = self.manifest.lock().unwrap();
        manifest.processes = records;
        manifest.updated_at = manifest_now_ms();
        manifest
            .save(&self.manifest_path)
            .map_err(|e| EngineError::new("manifest_write", e.to_string()))
    }

    /// Refresh the shareable projection after a late observer record. Callers
    /// must not use this to publish while the run is still mutating.
    pub fn refresh_artifacts(&self) -> Result<(), EngineError> {
        self.write_artifacts()
    }

    fn write_artifacts(&self) -> Result<(), EngineError> {
        let events = self.evidence.events();
        let event_values: Vec<_> = events.iter().map(|event| (**event).clone()).collect();
        let snapshot = self.evidence.snapshot(None, Vec::new());
        let commands = self
            .command_log
            .entries()
            .into_iter()
            .map(|(command, outcome)| CommandSummary {
                command_id: command.command_id,
                kind: command_kind_name(&command.kind),
                state: outcome
                    .map(|outcome| outcome.state.as_str().to_string())
                    .unwrap_or_else(|| "unknown".to_string()),
            })
            .collect();
        let manifest = self.manifest.lock().unwrap().clone();
        let assertions: Vec<AssertionSummary> = self
            .scenario
            .assertions
            .iter()
            .map(|assertion| {
                let evaluation = self.evidence.evaluate_assertion(assertion);
                AssertionSummary {
                    id: assertion.id.clone(),
                    state: evaluation_name(&evaluation).to_string(),
                    detail: evaluation_detail(&evaluation),
                }
            })
            .collect();
        let work_profiles = self
            .scenario
            .pools
            .iter()
            .map(|pool| work_summary(&pool.id, &pool.work_profile))
            .collect();
        let resources = resource_summaries(&manifest);
        let effective_profiles = manifest
            .effective_profiles
            .iter()
            .filter_map(|(id, profile)| {
                serde_json::to_value(profile)
                    .ok()
                    .map(|value| (id.clone(), value))
            })
            .collect();
        let non_claims = non_claims(&self.preflight, &self.config);
        let shareable = ShareableManifest {
            schema_version: 1,
            run_id: self.run_id.clone(),
            base_revision: manifest.base_revision.clone(),
            dirty_tree_hash: manifest.dirty_tree_hash.clone(),
            scenario_sha256: manifest.scenario_sha256.clone(),
            generated_at: evidence_now_ms(),
            state: manifest.state.as_str().to_string(),
            scenario: scenario_summary(&self.scenario),
            binary_hashes: manifest.binary_paths_and_hashes.clone(),
            resources,
            effective_profiles,
            pool_authorities: self.authorities.iter().map(PublicAuthority::from).collect(),
            work_profiles,
            commands,
            evidence: snapshot.evidence.clone(),
            gaps: snapshot.gaps.clone(),
            assertions,
            warnings: manifest.warnings.clone(),
            non_claims,
        };
        self.artifacts
            .write_shareable(&shareable)
            .map_err(|e| EngineError::new("artifact_shareable", e.to_string()))?;
        let charts = decimate_timeline(&event_values, 1_000);
        self.artifacts
            .write_charts(&charts)
            .map_err(|e| EngineError::new("artifact_charts", e.to_string()))?;
        self.artifacts
            .copy_private_evidence(&event_values)
            .map_err(|e| EngineError::new("artifact_evidence", e.to_string()))?;
        self.artifacts
            .write_result(&serde_json::json!({
                "run_id": self.run_id,
                "state": shareable.state,
                "assertions": shareable.assertions,
                "gaps": shareable.gaps.len(),
                "non_claims": shareable.non_claims,
            }))
            .map_err(|e| EngineError::new("artifact_result", e.to_string()))?;
        self.artifacts
            .write_private("scenario.json", &self.scenario)
            .map_err(|e| EngineError::new("artifact_scenario", e.to_string()))?;
        self.artifacts
            .write_private("runtime.json", &self.config)
            .map_err(|e| EngineError::new("artifact_runtime", e.to_string()))?;
        Ok(())
    }

    fn rebuild_state_from_log(&self) {
        let mut rebuilt = EngineState::default();
        for (command, outcome) in self.command_log.entries() {
            if !outcome
                .as_ref()
                .map(|outcome| outcome.state == OutcomeState::Succeeded)
                .unwrap_or(false)
            {
                continue;
            }
            match command.kind {
                CommandKind::AddMiner {
                    miner_id,
                    implementation,
                    connections,
                    workers,
                } => {
                    rebuilt.removed.remove(miner_id.as_str());
                    rebuilt.extra_miners.insert(
                        miner_id,
                        ExtraMiner {
                            implementation,
                            connections,
                            workers,
                        },
                    );
                }
                CommandKind::RemoveMiner { miner_id } => {
                    rebuilt.extra_miners.remove(&miner_id);
                    rebuilt.removed.insert(miner_id.to_string());
                }
                CommandKind::AddConnection {
                    miner_id,
                    connection,
                } => {
                    if let Some(extra) = rebuilt.extra_miners.get_mut(&miner_id) {
                        extra.connections.push(connection);
                    }
                }
                CommandKind::RemoveConnection {
                    miner_id,
                    connection_id,
                } => {
                    if let Some(extra) = rebuilt.extra_miners.get_mut(&miner_id) {
                        extra.connections.retain(|item| item.id != connection_id);
                    }
                }
                CommandKind::AddWorker { miner_id, worker } => {
                    if let Some(extra) = rebuilt.extra_miners.get_mut(&miner_id) {
                        extra.workers.push(worker);
                    }
                }
                CommandKind::SetWorkerBudget {
                    worker_id,
                    hashes_per_second,
                    total_hashes,
                    ..
                } => {
                    rebuilt.worker_budgets.insert(
                        worker_id,
                        BudgetSpec {
                            hashes_per_second,
                            total_hashes,
                        },
                    );
                }
                CommandKind::PauseWorkers { miner_id, workers } => {
                    if workers.is_empty() {
                        rebuilt.paused_miners.insert(miner_id);
                    } else {
                        rebuilt.paused_workers.extend(workers);
                    }
                }
                CommandKind::ResumeWorkers { miner_id, workers } => {
                    rebuilt.paused_miners.remove(&miner_id);
                    for worker in workers {
                        rebuilt.paused_workers.remove(&worker);
                    }
                }
                _ => {}
            }
        }
        rebuilt.started = false;
        *self.state.lock().unwrap() = rebuilt;
    }

    pub fn reconcile_commands(&self) -> Result<Vec<CommandOutcome>, EngineError> {
        let manifest = self.manifest.lock().unwrap().clone();
        let outcomes = self
            .command_log
            .reconcile(&manifest)
            .map_err(|e| EngineError::new("command_reconcile", e.to_string()))?;
        if !outcomes.is_empty() {
            let mut manifest = self.manifest.lock().unwrap();
            manifest.updated_at = manifest_now_ms();
            manifest
                .save(&self.manifest_path)
                .map_err(|e| EngineError::new("manifest_write", e.to_string()))?;
        }
        Ok(outcomes)
    }

    pub fn adopt_processes(&self) -> Result<Vec<(String, String, ProcessState)>, EngineError> {
        let records = self.manifest.lock().unwrap().processes.clone();
        let mut results = Vec::new();
        for record in records {
            let entity = record.entity.clone();
            let role = record.role.clone();
            let adopted_result = { self.supervisor.lock().unwrap().adopt(record.clone()) };
            let state = match adopted_result {
                Ok(mut adopted) => {
                    if adopted.control == ControlTransport::Pipe {
                        adopted.state = ProcessState::AdoptedControlLost;
                        self.supervisor.lock().unwrap().set_state(
                            &adopted.role,
                            &adopted.entity,
                            ProcessState::AdoptedControlLost,
                        );
                    }
                    let mut manifest = self.manifest.lock().unwrap();
                    manifest.upsert_process(adopted.clone());
                    adopted.state
                }
                Err(_) => {
                    let mut manifest = self.manifest.lock().unwrap();
                    let ownership = self.supervisor.lock().unwrap().verify(&record);
                    let state = if ownership.is_owned() {
                        if record.control == ControlTransport::Pipe {
                            ProcessState::AdoptedControlLost
                        } else {
                            ProcessState::Adopted
                        }
                    } else {
                        ProcessState::OrphanUnverified
                    };
                    if state == ProcessState::AdoptedControlLost {
                        self.supervisor.lock().unwrap().set_state(
                            &record.role,
                            &record.entity,
                            state,
                        );
                    }
                    manifest.upsert_process(crate::supervisor::ProcessRecord {
                        state,
                        ..record.clone()
                    });
                    state
                }
            };
            results.push((role, entity, state));
        }
        let mut manifest = self.manifest.lock().unwrap();
        manifest.processes = self.supervisor.lock().unwrap().records();
        if results
            .iter()
            .any(|(_, _, state)| !matches!(state, ProcessState::Adopted))
            && manifest.state == RunState::Running
        {
            manifest.state = RunState::CompletedWithErrors;
            manifest.warnings.push(
                "stale_resources: one or more recorded processes were missing, unverified, or had a pipe control channel that cannot be reopened; operator review required"
                    .to_string(),
            );
        }
        manifest.updated_at = manifest_now_ms();
        manifest
            .save(&self.manifest_path)
            .map_err(|e| EngineError::new("manifest_write", e.to_string()))?;
        Ok(results)
    }

    pub fn reap(&self) -> Result<Vec<(String, String, ProcessState)>, EngineError> {
        // Snapshot what a traffic run promised to launch and what the
        // manifest actually recorded, BEFORE reap_owned mutates records:
        // an expected node with no process record at all is reported as
        // exited rather than silently vanishing from the cleanup report.
        // The expectation is the scenario's node census itself: a node with
        // a launch profile but no process record stopped before launch,
        // and a node with neither was never provisioned (FAILED/INCOMPLETE
        // provisioning) - both are reported as expected-not-launched.
        // Non-traffic runs keep reporting only their recorded processes.
        let expected_nodes: BTreeSet<String> = if self.scenario.traffic.is_some() {
            let profiled: Vec<String> = {
                let manifest = self.manifest.lock().unwrap();
                manifest
                    .traffic_launch_profiles
                    .keys()
                    .map(|id| id.as_str().to_string())
                    .collect()
            };
            self.scenario
                .nodes
                .iter()
                .map(|node| node.id.as_str().to_string())
                .chain(profiled)
                .collect()
        } else {
            BTreeSet::new()
        };
        let recorded_nodes: BTreeSet<String> = {
            let manifest = self.manifest.lock().unwrap();
            manifest
                .processes
                .iter()
                .filter(|record| record.role == "node")
                .map(|record| record.entity.clone())
                .collect()
        };
        let mut results = self
            .supervisor
            .lock()
            .unwrap()
            .reap_owned(Duration::from_secs(2));
        for node in &expected_nodes {
            if !recorded_nodes.contains(node.as_str()) {
                results.push(("node".to_string(), node.clone(), ProcessState::Exited));
            }
        }
        let mut manifest = self.manifest.lock().unwrap();
        manifest.processes = self.supervisor.lock().unwrap().records();
        if results
            .iter()
            .any(|(_, _, state)| !matches!(state, ProcessState::Adopted))
            && manifest.state == RunState::Running
        {
            manifest.state = RunState::CompletedWithErrors;
            manifest.warnings.push(
                "stale_resources: one or more recorded processes were missing or unverified; operator review required"
                    .to_string(),
            );
        }
        manifest.updated_at = manifest_now_ms();
        manifest
            .save(&self.manifest_path)
            .map_err(|e| EngineError::new("manifest_write", e.to_string()))?;
        Ok(results)
    }

    pub fn assertions(&self) -> Vec<(String, AssertionEvaluation)> {
        self.scenario
            .assertions
            .iter()
            .map(|assertion| {
                (
                    assertion.id.clone(),
                    self.evidence.evaluate_assertion(assertion),
                )
            })
            .collect()
    }

    pub fn manifest_snapshot(&self) -> RunManifest {
        self.manifest.lock().unwrap().clone()
    }

    /// A short, read-only snapshot for projections that must not expose the
    /// supervisor lock or any of the process record's sensitive fields.
    pub fn process_records(&self) -> Vec<ProcessRecord> {
        self.supervisor.lock().unwrap().records()
    }

    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// Allowlisted, read-only role/process view for the existing HTTP server.
    pub fn lab_inventory(&self) -> crate::inventory::LabInventory {
        crate::inventory::project(self, manifest_now_ms())
    }

    pub fn evidence(&self) -> &EvidenceStore {
        &self.evidence
    }

    pub fn command_log(&self) -> &CommandLog {
        &self.command_log
    }

    pub fn config(&self) -> &LabConfig {
        &self.config
    }

    pub fn public_authorities(&self) -> BTreeMap<PoolId, PublicAuthority> {
        self.authorities
            .iter()
            .map(|(pool, authority)| (pool.clone(), PublicAuthority::from((pool, authority))))
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn emit_job_issued(
        &self,
        front: &FrontId,
        channel_id: u64,
        job_id: crate::ids::JobId,
        generation: u64,
        nbits: u32,
        share_target: String,
        m_issue: u64,
        work_unit_version: u16,
        fingerprint: String,
        activation: JobActivation,
    ) -> u64 {
        self.evidence.append(
            SourceId::trusted(format!("front-{front}")),
            format!("job-{job_id}"),
            EvidenceActor::new(ActorKind::Front, front.to_string()),
            None,
            EvidencePayload::JobIssued(crate::evidence::JobIssued {
                front_id: front.clone(),
                channel_id,
                job_id,
                generation,
                nbits,
                share_target,
                m_issue,
                work_unit_version,
                work_unit_fingerprint: fingerprint,
                issued_at: evidence_now_ms(),
                activation,
            }),
        )
    }

    pub fn emit_canonical(
        &self,
        node: &NodeId,
        block_hash: &str,
        height: u64,
        anchored_tip: &str,
        result: CanonicalResult,
        detail: impl Into<String>,
    ) -> u64 {
        self.evidence.append(
            SourceId::trusted(format!("node-{node}")),
            format!("canonical-{height}-{block_hash}"),
            EvidenceActor::new(ActorKind::Node, node.to_string()),
            None,
            EvidencePayload::CanonicalAt(CanonicalAt {
                node_id: node.clone(),
                block_hash: block_hash.to_string(),
                height,
                anchored_tip: anchored_tip.to_string(),
                observed_at: evidence_now_ms(),
                source: SourceId::trusted(format!("node-{node}")),
                result,
                detail: detail.into(),
            }),
        )
    }

    pub fn pools(&self) -> &[crate::scenario::PoolSpec] {
        &self.scenario.pools
    }

    fn manifest_state(&self) -> RunState {
        self.manifest.lock().unwrap().state
    }

    fn set_manifest_state(&self, state: RunState) -> Result<(), EngineError> {
        let mut manifest = self.manifest.lock().unwrap();
        manifest.state = state;
        manifest.updated_at = manifest_now_ms();
        manifest
            .save(&self.manifest_path)
            .map_err(|e| EngineError::new("manifest_write", e.to_string()))
    }

    fn emit_run_state(&self, state: &str) {
        self.evidence.append(
            SourceId::trusted("harness"),
            format!("run-state-{state}-{}", evidence_now_ms()),
            EvidenceActor::harness(),
            None,
            EvidencePayload::RunState {
                state: state.to_string(),
            },
        );
        let records = self.supervisor.lock().unwrap().records();
        for record in records {
            self.evidence.append(
                SourceId::trusted(format!("process-{}", record.role)),
                format!("process-{}-{}", record.pid, record.start_time_ticks),
                EvidenceActor::harness(),
                None,
                EvidencePayload::Process(ProcessObservation {
                    role: record.role.clone(),
                    entity: record.entity.clone(),
                    state: record.state.as_str().to_string(),
                    pid: record.pid,
                }),
            );
        }
    }
}

fn entity_for_process(
    supervisor: &Supervisor,
    manifest: &RunManifest,
    role: &str,
    entity: &str,
) -> EntitySnapshot {
    let record = supervisor
        .record(role, entity)
        .cloned()
        .or_else(|| manifest.process(role, entity).cloned());
    match record {
        Some(record) => EntitySnapshot {
            kind: role.to_string(),
            id: entity.to_string(),
            status: record.state.as_str().to_string(),
            detail: format!("pid {} pid-start {}", record.pid, record.start_time_ticks),
            miner_id: None,
        },
        None => EntitySnapshot {
            kind: role.to_string(),
            id: entity.to_string(),
            status: "declared".to_string(),
            detail: "not launched in this run (or attached without a process record)".to_string(),
            miner_id: None,
        },
    }
}

fn expand_args(
    adapter: &AdapterConfig,
    extra: &[String],
    context: &BTreeMap<&str, String>,
) -> Result<Vec<String>, EngineError> {
    let mut args = Vec::new();
    for template in adapter.args.iter().chain(extra.iter()) {
        args.push(
            expand_template(template, context)
                .map_err(|e| EngineError::new(e.code(), e.detail()))?,
        );
    }
    Ok(args)
}

/// Value of a two-argument flag in a rendered argv. `None` when the flag is
/// absent, and `None` when it is ambiguous (repeated, or trailing with no
/// value): callers fail closed rather than guessing which occurrence was
/// meant.
pub(crate) fn argv_flag_value<'a>(argv: &'a [String], flag: &str) -> Option<&'a str> {
    let mut value: Option<&str> = None;
    let mut index = 0;
    while index < argv.len() {
        if argv[index] == flag {
            if value.is_some() || index + 1 >= argv.len() {
                return None;
            }
            value = Some(argv[index + 1].as_str());
        }
        index += 1;
    }
    value
}

/// Reject caller-controlled node P2P wiring before any launch effect. The
/// engine owns node peering (namespace topology or host ring), so a scenario
/// or adapter argument that pre-empts it (`--listen`, `--connect`,
/// `--addnode`) or duplicates an engine-managed isolation flag would make
/// the planned graph unobservable. Fails closed with zero process, manifest
/// or profile effects.
fn validate_node_p2p_argv(node: &NodeId, args: &[String]) -> Result<(), EngineError> {
    let count = |flag: &str| args.iter().filter(|a| *a == flag).count();
    for flag in ["--listen", "--connect", "--addnode"] {
        if count(flag) > 0 {
            return Err(EngineError::new(
                "node_p2p_argv_invalid",
                format!("node {node} argv carries caller P2P flag {flag}"),
            ));
        }
    }
    for flag in [
        "--no-listen",
        "--no-seeds",
        "--no-discover",
        "--max-outbound",
    ] {
        if count(flag) > 1 {
            return Err(EngineError::new(
                "node_p2p_argv_invalid",
                format!("node {node} argv repeats {flag}"),
            ));
        }
    }
    Ok(())
}

fn collect_binaries(
    config: &LabConfig,
    out: &mut BTreeMap<String, crate::manifest::BinaryIdentity>,
) {
    let adapters = config
        .nodes
        .values()
        .chain(config.pools.values())
        .chain(config.miners.values())
        .chain(config.ark.values())
        .chain(config.redis.iter())
        .chain(config.traffic_probe.iter())
        .chain(config.tx_driver.iter())
        .chain(config.postgres.iter());
    for adapter in adapters {
        let key = adapter.program.display().to_string();
        let hash = hash_file(&adapter.program).unwrap_or_else(|_| "unreadable".to_string());
        out.insert(
            key.clone(),
            crate::manifest::BinaryIdentity {
                path: adapter.program.clone(),
                sha256: hash,
            },
        );
    }
}

fn command_kind_name(kind: &CommandKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|value| {
            value
                .get("type")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "unknown".to_string())
}

fn work_summary(pool_id: &PoolId, profile: &WorkProfileSpec) -> WorkProfileSummary {
    let fingerprint = profile
        .reference_target
        .as_deref()
        .and_then(|reference| ReferenceProfile::parse_hex(reference).ok())
        .map(|reference| reference.fingerprint());
    WorkProfileSummary {
        pool_id: pool_id.to_string(),
        work_unit_version: profile.work_unit_version,
        window_rule: match profile.window_rule {
            crate::scenario::WindowRule::ShareWorkLegacy => "share_work_legacy".to_string(),
            crate::scenario::WindowRule::NetworkBlockWork => "network_block_work".to_string(),
        },
        reference_fingerprint: fingerprint,
        share_work_multiplier: profile.share_work_multiplier,
        window_n: profile.window_n,
    }
}

fn scenario_summary(scenario: &Scenario) -> ScenarioSummary {
    ScenarioSummary {
        scenario_version: scenario.scenario_version,
        node_count: scenario.nodes.len() as u64,
        pool_count: scenario.pools.len() as u64,
        front_count: scenario.fronts.len() as u64,
        miner_count: scenario.miners.len() as u64,
        identity_count: scenario.identities.len() as u64,
        worker_count: scenario.miners.iter().map(|m| m.workers.len() as u64).sum(),
        link_count: scenario.links.len() as u64,
    }
}

fn resource_summaries(manifest: &RunManifest) -> Vec<ResourceSummary> {
    let mut values = Vec::new();
    for redis in &manifest.resources.redis_instances {
        values.push(ResourceSummary {
            kind: "redis".to_string(),
            id: redis.id.clone(),
            owner: redis.owner_pool_id.to_string(),
            port: redis.port,
            database: None,
        });
    }
    for pg in &manifest.resources.postgres_databases {
        values.push(ResourceSummary {
            kind: "postgres".to_string(),
            id: pg.id.clone(),
            owner: pg.owner_pool_id.to_string(),
            port: pg.port,
            database: Some(pg.database.clone()),
        });
    }
    values
}

fn non_claims(preflight: &PreflightReport, config: &LabConfig) -> Vec<String> {
    let mut values = vec![
        "LP-3 fixture runs are not real mining and are not an SV2 interoperability test".to_string(),
        "A block candidate observation is not node acceptance or canonical inclusion".to_string(),
        "LP-2's native CPU miner and L1's conformance evidence are required before a real vertical slice is claimed".to_string(),
        "L2's effective consensus profile is compared by field value; a fingerprint alone is not trusted".to_string(),
    ];
    if config
        .nodes
        .values()
        .chain(config.pools.values())
        .chain(config.miners.values())
        .any(|adapter| adapter.fixture)
    {
        values.push(
            "one or more adapters are fixture processes: no simulated share or hash is counted as live PoW"
                .to_string(),
        );
    }
    if let Some(warning) = preflight
        .warnings
        .iter()
        .find(|warning| warning.contains("fixture"))
    {
        values.push(warning.clone());
    }
    values
}

fn wide_json(value: Option<WideU128>) -> Value {
    match value {
        Some(value) if value.get() <= u64::MAX as u128 => Value::from(value.get() as u64),
        Some(value) => Value::String(value.to_string()),
        None => Value::Null,
    }
}

fn evaluation_name(evaluation: &AssertionEvaluation) -> &'static str {
    match evaluation {
        AssertionEvaluation::Passed => "passed",
        AssertionEvaluation::Failed(_) => "failed",
        AssertionEvaluation::Inconclusive(_) => "inconclusive",
        AssertionEvaluation::BlockedOnHeight(_) => "blocked_on_height",
        AssertionEvaluation::TimedOut(_) => "timed_out",
    }
}

fn evaluation_detail(evaluation: &AssertionEvaluation) -> String {
    match evaluation {
        AssertionEvaluation::Passed => String::new(),
        AssertionEvaluation::Failed(detail)
        | AssertionEvaluation::Inconclusive(detail)
        | AssertionEvaluation::BlockedOnHeight(detail)
        | AssertionEvaluation::TimedOut(detail) => detail.clone(),
    }
}

impl std::fmt::Debug for RunEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunEngine")
            .field("run_id", &self.run_id)
            .field("run_dir", &self.run_dir)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for RuntimeAdapters {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeAdapters")
            .field("nodes", &self.nodes.lock().unwrap().len())
            .finish()
    }
}
