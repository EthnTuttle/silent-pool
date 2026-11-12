//! Small, allowlisted role/process inventory for the loopback lab hub.
//!
//! This is a projection, not a second control plane. It performs no network
//! requests and never serializes a manifest, process record, adapter config,
//! evidence payload, endpoint credential or artifact path. Observations come
//! from evidence that the runtime already collected; absence remains unknown.

use std::collections::BTreeMap;
use std::net::IpAddr;

use serde::Serialize;
use serde_json::Value;

use crate::engine::RunEngine;
use crate::evidence::{EvidenceEvent, EvidencePayload};
use crate::manifest::RunState;
use crate::resource::Endpoint;
use crate::scenario::FrontRole;
use crate::supervisor::ProcessState;

const RECENT_OBSERVATION_MS: u64 = 15_000;

#[derive(Debug, Serialize)]
pub struct LabInventory {
    pub schema_version: &'static str,
    pub run_id: String,
    pub lifecycle: String,
    pub view: &'static str,
    pub topology: &'static str,
    pub controls_href: &'static str,
    pub declared_actor_count: usize,
    pub recorded_actor_count: usize,
    pub recorded_live_actor_count: usize,
    pub latest_evidence: ObservationView,
    pub groups: Vec<InventoryGroup>,
    pub integrations: Vec<IntegrationView>,
}

#[derive(Debug, Serialize)]
pub struct InventoryGroup {
    pub id: String,
    pub title: String,
    pub purpose: &'static str,
    pub rows: Vec<InventoryRow>,
}

#[derive(Debug, Serialize)]
pub struct InventoryRow {
    pub id: String,
    pub process: String,
    pub purpose: String,
    pub declared_role: String,
    pub logical_roles: Vec<String>,
    pub recorded_state: String,
    pub observation: ObservationView,
    pub interfaces: Vec<InterfaceView>,
    pub dependencies: Vec<String>,
    pub details: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ObservationView {
    pub availability: &'static str,
    pub observed_at_ms: Option<u64>,
    pub age_ms: Option<u64>,
    pub freshness: &'static str,
    pub source: &'static str,
}

#[derive(Debug, Serialize)]
pub struct InterfaceView {
    pub label: String,
    pub kind: &'static str,
    pub url: Option<String>,
    pub note: &'static str,
}

#[derive(Debug, Serialize)]
pub struct IntegrationView {
    pub id: String,
    pub title: String,
    pub current_status: &'static str,
    pub required_next_profile: bool,
    pub ui: &'static str,
    pub details: Vec<String>,
}

#[derive(Clone)]
struct ObservationFact {
    availability: &'static str,
    at_ms: u64,
    source: &'static str,
}

impl ObservationFact {
    fn new(availability: &'static str, at_ms: u64, source: &'static str) -> Self {
        Self {
            availability,
            at_ms,
            source,
        }
    }

    fn view(&self, now_ms: u64, recorded: bool) -> ObservationView {
        ObservationView {
            availability: self.availability,
            observed_at_ms: Some(self.at_ms),
            age_ms: Some(now_ms.saturating_sub(self.at_ms)),
            freshness: if recorded {
                "recorded"
            } else if now_ms.saturating_sub(self.at_ms) <= RECENT_OBSERVATION_MS {
                "recent"
            } else {
                "older"
            },
            source: self.source,
        }
    }
}

#[derive(Default)]
struct EvidenceIndex {
    latest_at: Option<u64>,
    fronts: BTreeMap<String, ObservationFact>,
    nodes: BTreeMap<String, ObservationFact>,
    miners: BTreeMap<String, ObservationFact>,
    redis: BTreeMap<String, ObservationFact>,
    postgres: BTreeMap<String, ObservationFact>,
    scanner: Option<ObservationFact>,
    broker: Option<ObservationFact>,
    /// EXP-OP operator readiness facts, keyed by front id. The runtime
    /// records one after its bounded page probe; `page_served` is the only
    /// basis on which the inventory may expose an operator interface.
    operator: BTreeMap<String, OperatorFact>,
}

/// The operator readiness fact recorded by the runtime's start-time probe.
/// `page_served` means the page route answered HTTP 200 - a route proof,
/// never a funds-health claim.
#[derive(Clone)]
struct OperatorFact {
    page_served: bool,
}

impl EvidenceIndex {
    fn collect(events: &[std::sync::Arc<EvidenceEvent>]) -> Self {
        let mut index = Self::default();
        for event in events {
            let at = event.occurred_at_wall;
            index.latest_at = Some(at);
            match &event.payload {
                EvidencePayload::CanonicalAt(observation) => {
                    index.nodes.insert(
                        observation.node_id.to_string(),
                        ObservationFact::new("reachable_when_observed", at, "node tip sampler"),
                    );
                }
                EvidencePayload::TopologyPhase(phase) => {
                    for front in &phase.fronts {
                        index.fronts.insert(
                            front.front.to_string(),
                            ObservationFact::new(
                                if front.reachable {
                                    "reachable_when_observed"
                                } else {
                                    "unreachable_when_observed"
                                },
                                at,
                                "front API collector",
                            ),
                        );
                    }
                    for node in &phase.nodes {
                        index.nodes.insert(node.node.clone(), node_fact(node, at));
                    }
                    if let Some(broker) = &phase.broker_evidence {
                        let mut results = Vec::new();
                        results.extend(broker.counters.as_ref().map(|call| call.ok));
                        results.extend(broker.no_bypass.as_ref().map(|call| call.ok));
                        results.extend(
                            broker
                                .consensus_counters
                                .iter()
                                .map(|counter| counter.counters.ok),
                        );
                        if !results.is_empty() {
                            index.broker = Some(ObservationFact::new(
                                if results.iter().any(|ok| *ok) {
                                    "reachable_when_observed"
                                } else {
                                    "unreachable_when_observed"
                                },
                                at,
                                "namespace broker collector",
                            ));
                        }
                    }
                }
                EvidencePayload::TopologyConvergence(convergence) => {
                    for node in &convergence.views {
                        index.nodes.insert(node.node.clone(), node_fact(node, at));
                    }
                }
                EvidencePayload::SpcEvidence(record) => {
                    if let Some(front) = &record.front_id {
                        index.fronts.insert(
                            front.to_string(),
                            ObservationFact::new(
                                "evidence_received",
                                at,
                                "pool evidence collector",
                            ),
                        );
                    }
                }
                EvidencePayload::MinerEvent { miner_id, .. } => {
                    index.miners.insert(
                        miner_id.to_string(),
                        ObservationFact::new("evidence_received", at, "owned miner event stream"),
                    );
                }
                EvidencePayload::Unknown(unknown) => match unknown.topic.as_str() {
                    "node_tip_unknown" => {
                        if let Some(node) =
                            detail(&unknown.detail).get("node").and_then(Value::as_str)
                        {
                            index.nodes.insert(
                                node.to_string(),
                                ObservationFact::new(
                                    "unreachable_when_observed",
                                    at,
                                    "node tip sampler",
                                ),
                            );
                        }
                    }
                    "redis_observation" => {
                        if let Some(id) = event.source.as_str().strip_prefix("redis-") {
                            let reachable =
                                detail(&unknown.detail).get("ping").and_then(Value::as_str)
                                    == Some("PONG");
                            index.redis.insert(
                                id.to_string(),
                                ObservationFact::new(
                                    if reachable {
                                        "reachable_when_observed"
                                    } else {
                                        "unreachable_when_observed"
                                    },
                                    at,
                                    "authenticated Redis read collector",
                                ),
                            );
                        }
                    }
                    "postgres_observation" => {
                        if let Some(id) = event.source.as_str().strip_prefix("postgres-") {
                            let reachable = detail(&unknown.detail).get("observation").is_some();
                            index.postgres.insert(
                                id.to_string(),
                                ObservationFact::new(
                                    if reachable {
                                        "reachable_when_observed"
                                    } else {
                                        "unreachable_when_observed"
                                    },
                                    at,
                                    "authenticated Postgres read collector",
                                ),
                            );
                        }
                    }
                    "recipient_scan" => {
                        index.scanner = Some(ObservationFact::new(
                            "collector_observed",
                            at,
                            "in-process scan collector",
                        ));
                    }
                    "operator_observation" => {
                        let fields = detail(&unknown.detail);
                        if let (Some(front), Some(page_served)) = (
                            fields.get("front").and_then(Value::as_str),
                            fields.get("page_served").and_then(Value::as_bool),
                        ) {
                            index
                                .operator
                                .insert(front.to_string(), OperatorFact { page_served });
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        index
    }
}

fn node_fact(node: &crate::evidence::AtomicTipObservation, at: u64) -> ObservationFact {
    ObservationFact::new(
        if node.atomic && node.error.is_none() {
            "reachable_when_observed"
        } else if node.error.is_some() {
            "unreachable_when_observed"
        } else {
            "unknown"
        },
        at,
        "atomic node view collector",
    )
}

fn detail(value: &str) -> Value {
    serde_json::from_str(value).unwrap_or(Value::Null)
}

fn unknown_observation() -> ObservationView {
    ObservationView {
        availability: "unknown",
        observed_at_ms: None,
        age_ms: None,
        freshness: "unknown",
        source: "no collected availability observation",
    }
}

fn observation(fact: Option<&ObservationFact>, now_ms: u64, recorded: bool) -> ObservationView {
    fact.map(|fact| fact.view(now_ms, recorded))
        .unwrap_or_else(unknown_observation)
}

fn latest_observation(at: Option<u64>, now_ms: u64, recorded: bool) -> ObservationView {
    at.map(|at| {
        ObservationFact::new("evidence_retained", at, "latest retained evidence event")
            .view(now_ms, recorded)
    })
    .unwrap_or_else(unknown_observation)
}

/// Project the current engine into the public inventory schema.
///
/// Locks are taken only through the existing short snapshot methods. All source
/// values needed by the projection are cloned before any evidence processing;
/// no lock is held across file I/O, Chrome, HTTP or any other network call.
pub fn project(engine: &RunEngine, now_ms: u64) -> LabInventory {
    let recorded = engine.is_read_only();
    let manifest = engine.manifest_snapshot();
    let supervisor_records = engine.process_records();
    let events = engine.evidence().events();
    let index = EvidenceIndex::collect(&events);
    let namespaced = manifest.placement.is_some();

    let mut states: BTreeMap<(String, String), ProcessState> = BTreeMap::new();
    for record in manifest.processes.iter().chain(supervisor_records.iter()) {
        states.insert((record.role.clone(), record.entity.clone()), record.state);
    }
    let state_for = |role: &str, entity: &str| -> String {
        states
            .get(&(role.to_string(), entity.to_string()))
            .map(|state| state.as_str().to_string())
            .unwrap_or_else(|| {
                if manifest.state == RunState::Created {
                    "not_started".to_string()
                } else {
                    "not_recorded".to_string()
                }
            })
    };

    let mut groups = vec![controller_group(
        engine,
        &index,
        manifest.state,
        recorded,
        now_ms,
    )];
    groups.push(node_group(engine, &index, &state_for, recorded, now_ms));

    for pool in &engine.scenario.pools {
        groups.push(pool_group(
            engine,
            &manifest.resources.listeners,
            pool,
            &index,
            &state_for,
            namespaced,
            recorded,
            now_ms,
        ));
    }
    groups.push(miner_group(engine, &index, &state_for, recorded, now_ms));

    if !engine.scenario.ark_backends.is_empty() {
        groups.push(ark_group(engine, &index, &state_for, recorded, now_ms));
    }
    if namespaced {
        groups.push(namespace_group(&index, recorded, now_ms));
    }
    groups.retain(|group| !group.rows.is_empty());

    let declared_actor_count = engine.scenario.nodes.len()
        + engine.scenario.fronts.len()
        + engine.runtime_miner_specs().len()
        + engine.scenario.ark_backends.len()
        + manifest.resources.redis_instances.len()
        + manifest.resources.postgres_databases.len()
        + if engine
            .scenario
            .ark_backends
            .iter()
            .any(|b| b.local.is_some())
        {
            1 + engine.scenario.pools.len()
        } else {
            0
        };
    let recorded_actor_count = states.len();
    let recorded_live_actor_count = states
        .values()
        .filter(|state| {
            matches!(
                state,
                ProcessState::Running | ProcessState::Adopted | ProcessState::AdoptedControlLost
            )
        })
        .count();

    LabInventory {
        schema_version: "spc.lab.inventory.v1",
        run_id: engine.run_id.to_string(),
        lifecycle: manifest.state.as_str().to_string(),
        view: if recorded {
            "recorded_read_only"
        } else {
            "live"
        },
        topology: if namespaced { "namespaces" } else { "host" },
        controls_href: "/",
        declared_actor_count,
        recorded_actor_count,
        recorded_live_actor_count,
        latest_evidence: latest_observation(index.latest_at, now_ms, recorded),
        groups,
        integrations: integrations(engine),
    }
}

fn controller_group(
    engine: &RunEngine,
    index: &EvidenceIndex,
    lifecycle: RunState,
    recorded: bool,
    now_ms: u64,
) -> InventoryGroup {
    let scanner_dependencies = engine
        .scenario
        .nodes
        .first()
        .map(|node| vec![node.id.to_string()])
        .unwrap_or_default();
    InventoryGroup {
        id: "controller".to_string(),
        title: "Lab controller and collection".to_string(),
        purpose: "One process owns orchestration, durable commands, evidence and this HTTP server.",
        rows: vec![
            InventoryRow {
                id: "spc-lab".to_string(),
                process: "spc-lab controller".to_string(),
                purpose: "Orchestrates real regtest processes and serves the existing browser controls."
                    .to_string(),
                declared_role: "run controller".to_string(),
                logical_roles: vec![
                    "orchestration".to_string(),
                    "durable commands".to_string(),
                    "evidence".to_string(),
                    "loopback HTTP/SSE".to_string(),
                ],
                recorded_state: lifecycle.as_str().to_string(),
                observation: ObservationView {
                    availability: "serving_this_response",
                    observed_at_ms: Some(now_ms),
                    age_ms: Some(0),
                    freshness: if recorded { "recorded" } else { "recent" },
                    source: "this loopback HTTP response",
                },
                interfaces: vec![
                    interface(
                        "Open live controls",
                        "ui",
                        Some("/".to_string()),
                        "Existing advanced monitor and miner controls.",
                    ),
                    interface(
                        "Lab live hub",
                        "ui",
                        Some("/lab".to_string()),
                        "This role and process guide.",
                    ),
                ],
                dependencies: vec!["scenario + owned supervisor".to_string()],
                details: vec!["Closing a browser tab does not stop the run.".to_string()],
            },
            InventoryRow {
                id: "scanner".to_string(),
                process: "spc-lab collector thread".to_string(),
                purpose: "Uses the spc_scan library over canonical blocks fetched by the collector."
                    .to_string(),
                declared_role: "embedded scan collector".to_string(),
                logical_roles: vec!["in-process recipient scanning".to_string()],
                recorded_state: "embedded_no_standalone_process".to_string(),
                observation: observation(index.scanner.as_ref(), now_ms, recorded),
                interfaces: vec![interface(
                    "spc-scan CLI/library",
                    "cli_library",
                    None,
                    "The standalone CLI/library exists; the runtime does not launch a scanner daemon.",
                )],
                dependencies: scanner_dependencies,
                details: vec!["A separate spc-scan process is not invented for this view.".to_string()],
            },
        ],
    }
}

fn node_group(
    engine: &RunEngine,
    index: &EvidenceIndex,
    state_for: &dyn Fn(&str, &str) -> String,
    recorded: bool,
    now_ms: u64,
) -> InventoryGroup {
    let rows = engine
        .scenario
        .nodes
        .iter()
        .map(|node| InventoryRow {
            id: format!("node-{}", node.id),
            process: format!("{} {}", node.implementation, node.id),
            purpose: "Consensus, canonical chain and difficulty; templates, submission and P2P."
                .to_string(),
            declared_role: format!("node {}", node.id),
            logical_roles: vec![
                "consensus / canonical chain".to_string(),
                "getblocktemplate / submitblock".to_string(),
                "P2P".to_string(),
            ],
            recorded_state: state_for("node", node.id.as_str()),
            observation: observation(index.nodes.get(node.id.as_str()), now_ms, recorded),
            interfaces: vec![interface(
                "Authenticated JSON-RPC",
                "authenticated_json_rpc",
                None,
                "No browser UI and no browser credential handoff.",
            )],
            dependencies: Vec::new(),
            details: vec![
                "Native SV2 template transport is separate from the browser.".to_string(),
            ],
        })
        .collect();
    InventoryGroup {
        id: "nodes".to_string(),
        title: "Regtest nodes".to_string(),
        purpose: "Real node processes; process records are not health checks.",
        rows,
    }
}

#[allow(clippy::too_many_arguments)]
fn pool_group(
    engine: &RunEngine,
    listeners: &BTreeMap<String, Endpoint>,
    pool: &crate::scenario::PoolSpec,
    index: &EvidenceIndex,
    state_for: &dyn Fn(&str, &str) -> String,
    namespaced: bool,
    recorded: bool,
    now_ms: u64,
) -> InventoryGroup {
    let mut rows = Vec::new();
    for front in engine
        .scenario
        .fronts
        .iter()
        .filter(|front| front.pool_id == pool.id)
    {
        let roles: Vec<String> = front.roles.iter().map(|role| role_name(*role)).collect();
        let details = front
            .roles
            .iter()
            .map(|role| match role {
                FrontRole::Front => {
                    "front: job, share and block validation over native mining protocols."
                        .to_string()
                }
                FrontRole::Payout => {
                    "payout: accounting, persistence and confirmation-gated ledger.".to_string()
                }
                FrontRole::Api => {
                    "api: read-only JSON and Prometheus metrics, not an HTML operator UI."
                        .to_string()
                }
                FrontRole::Ark => {
                    "ark: declared delivery role; availability is not inferred from process state."
                        .to_string()
                }
            })
            .collect();
        let mut interfaces = vec![interface(
            "Native SV1/SV2 mining transport",
            "native_protocol",
            None,
            "Miner protocol endpoint; no browser UI.",
        )];
        if front.roles.contains(&FrontRole::Api) {
            let endpoint = listeners.get(&format!("front.{}.api", front.id));
            interfaces.push(interface(
                "Pool API JSON",
                "api_json",
                endpoint.and_then(|endpoint| browser_url(endpoint, "/api/pool", namespaced)),
                if namespaced {
                    "Internal namespace transport; use only the collected observation shown here."
                } else {
                    "Actual API route, explicitly JSON rather than a role-native UI."
                },
            ));
            interfaces.push(interface(
                "Pool metrics",
                "metrics",
                endpoint.and_then(|endpoint| browser_url(endpoint, "/metrics", namespaced)),
                if namespaced {
                    "Internal namespace transport; no host-browser link."
                } else {
                    "Actual Prometheus route, not an HTML dashboard."
                },
            ));
        }
        // EXP-OP operator page: the interface exists only when the run
        // planned the `front.<id>.operator` listener (`--operator-ui`),
        // the runtime's start-time probe recorded the page as served, and
        // the endpoint is host loopback. This projection performs no
        // network request; "served" comes only from the recorded fact.
        if let (Some(endpoint), Some(fact)) = (
            listeners.get(&format!("front.{}.operator", front.id)),
            index.operator.get(front.id.as_str()),
        ) {
            if fact.page_served {
                if let Some(url) = browser_url(endpoint, "/", namespaced) {
                    interfaces.push(interface(
                        "Pool operator page",
                        "operator_ui",
                        Some(url),
                        "Approved read-only operator page; served proof was recorded at start.",
                    ));
                }
            }
        }
        let mut dependencies: Vec<String> = pool.node_ids.iter().map(ToString::to_string).collect();
        dependencies.push(pool.storage.redis_instance_id.clone());
        dependencies.push(pool.storage.postgres_database_id.clone());
        rows.push(InventoryRow {
            id: format!("front-{}", front.id),
            process: format!("{} {}", pool.implementation, front.id),
            purpose: "One spc-pool process; the scenario's logical roles may be co-located here."
                .to_string(),
            declared_role: format!("front {} on {}", front.id, pool.id),
            logical_roles: roles,
            recorded_state: state_for("front", front.id.as_str()),
            observation: observation(index.fronts.get(front.id.as_str()), now_ms, recorded),
            interfaces,
            dependencies,
            details,
        });
    }

    for redis in manifest_resources(engine).0.into_iter().filter(|id| {
        engine
            .scenario
            .resources
            .redis_instances
            .iter()
            .any(|resource| resource.id == *id && resource.owner_pool_id == pool.id)
    }) {
        rows.push(InventoryRow {
            id: format!("redis-{redis}"),
            process: format!("redis-server {redis}"),
            purpose: format!("Per-pool stream and event bus for {}.", pool.id),
            declared_role: format!("Redis {redis}"),
            logical_roles: vec!["pool stream / event bus".to_string()],
            recorded_state: state_for("redis", &redis),
            observation: observation(index.redis.get(&redis), now_ms, recorded),
            interfaces: vec![interface(
                "Redis protocol",
                "native_protocol",
                None,
                "No browser UI and no direct authenticated connection link.",
            )],
            dependencies: vec![pool.id.to_string()],
            details: Vec::new(),
        });
    }
    for postgres in manifest_resources(engine).1.into_iter().filter(|id| {
        engine
            .scenario
            .resources
            .postgres_databases
            .iter()
            .any(|resource| resource.id == *id && resource.owner_pool_id == pool.id)
    }) {
        rows.push(InventoryRow {
            id: format!("postgres-{postgres}"),
            process: format!("postgres {postgres}"),
            purpose: format!("Per-pool ledger storage for {}.", pool.id),
            declared_role: format!("PostgreSQL {postgres}"),
            logical_roles: vec!["ledger persistence".to_string()],
            recorded_state: state_for("postgres", &postgres),
            observation: observation(index.postgres.get(&postgres), now_ms, recorded),
            interfaces: vec![interface(
                "PostgreSQL storage",
                "native_protocol",
                None,
                "No browser UI and no direct authenticated connection link.",
            )],
            dependencies: vec![pool.id.to_string()],
            details: Vec::new(),
        });
    }

    InventoryGroup {
        id: pool.id.to_string(),
        title: pool.id.to_string(),
        purpose: "Pool front, co-located logical roles, and its isolated storage resources.",
        rows,
    }
}

// Return only identifiers; URLs, database roles, paths and ports never enter
// the projection through this helper.
fn manifest_resources(engine: &RunEngine) -> (Vec<String>, Vec<String>) {
    (
        engine
            .scenario
            .resources
            .redis_instances
            .iter()
            .map(|resource| resource.id.clone())
            .collect(),
        engine
            .scenario
            .resources
            .postgres_databases
            .iter()
            .map(|resource| resource.id.clone())
            .collect(),
    )
}

fn miner_group(
    engine: &RunEngine,
    index: &EvidenceIndex,
    state_for: &dyn Fn(&str, &str) -> String,
    recorded: bool,
    now_ms: u64,
) -> InventoryGroup {
    let rows = engine
        .runtime_miner_specs()
        .into_iter()
        .map(|(miner, implementation, connections, _workers)| {
            let cpu_miner = implementation == "spc-cpu-miner";
            InventoryRow {
                id: format!("miner-{miner}"),
                process: format!("{implementation} {miner}"),
                purpose: if cpu_miner {
                    "Actual SHA256d worker budgets and native SV2 lab driver.".to_string()
                } else {
                    format!("Scenario miner process using {implementation}.")
                },
                declared_role: format!("miner {miner}"),
                logical_roles: if cpu_miner {
                    vec![
                        "SHA256d workers".to_string(),
                        "SV2 extended-channel driver".to_string(),
                        "owned JSONL control".to_string(),
                    ]
                } else {
                    vec![implementation.clone()]
                },
                recorded_state: state_for("miner", miner.as_str()),
                observation: observation(index.miners.get(miner.as_str()), now_ms, recorded),
                interfaces: vec![interface(
                    "Owned JSONL control through spc-lab",
                    "jsonl_control",
                    None,
                    "No standalone browser UI; this is not spc-agent.",
                )],
                dependencies: connections
                    .iter()
                    .map(|connection| connection.front_id.to_string())
                    .collect(),
                details: Vec::new(),
            }
        })
        .collect();
    InventoryGroup {
        id: "miners".to_string(),
        title: "Miners".to_string(),
        purpose: "Actual scenario driver processes and their declared front dependencies.",
        rows,
    }
}

fn ark_group(
    engine: &RunEngine,
    _index: &EvidenceIndex,
    state_for: &dyn Fn(&str, &str) -> String,
    recorded: bool,
    now_ms: u64,
) -> InventoryGroup {
    let mut rows: Vec<InventoryRow> = engine
        .scenario
        .ark_backends
        .iter()
        .map(|backend| InventoryRow {
            id: format!("ark-{}", backend.id),
            process: format!("{} {}", backend.implementation, backend.id),
            purpose: "Ark server backend declared explicitly by this scenario.".to_string(),
            declared_role: format!("Ark server {}", backend.id),
            logical_roles: vec![if backend.local.is_some() {
                "self-hosted stock Ark server"
            } else {
                "declared Ark server"
            }
            .to_string()],
            recorded_state: state_for("ark", backend.id.as_str()),
            observation: observation(None, now_ms, recorded),
            interfaces: vec![interface(
                "Ark service transport",
                "native_protocol",
                None,
                "No role-native browser UI; declaration does not imply validated Ark settlement.",
            )],
            dependencies: Vec::new(),
            details: if backend.local.is_some() {
                crate::ark_service::inventory_details(engine, backend.id.as_str())
            } else {
                vec!["Declaration alone does not prove an initialized or funded server.".into()]
            },
        })
        .collect();
    for backend in engine
        .scenario
        .ark_backends
        .iter()
        .filter(|b| b.local.is_some())
    {
        rows.push(InventoryRow {
            id: format!("ark-postgres-{}", backend.id),
            process: "postgres (Ark provider)".into(),
            purpose: "Private provider storage, separate from both pool databases.".into(),
            declared_role: "Ark provider database".into(),
            logical_roles: vec!["provider storage".into()],
            recorded_state: state_for("ark-postgres", backend.id.as_str()),
            observation: observation(None, now_ms, recorded),
            interfaces: vec![],
            dependencies: vec![],
            details: vec!["Dedicated run-local cluster with independent credentials.".into()],
        });
        for pool in &engine.scenario.pools {
            rows.push(InventoryRow {
                id:format!("pool-ark-{}",pool.id), process:format!("spc-pool --roles ark ({})",pool.id),
                purpose:"Pool-owned wallet and float; identity-free orders only.".into(),
                declared_role:"pool Ark wallet".into(), logical_roles:vec!["ark".into()],
                recorded_state:state_for("pool-ark",pool.id.as_str()),
                observation:observation(None,now_ms,recorded), interfaces:vec![],
                dependencies:vec![format!("ark-{}",backend.id)],
                details:vec!["Independent seed, wallet SQLite and restricted Redis ACL; no Postgres credentials.".into(),
                    "115000000-sat target is configuration, not a funded balance. Native funding and user-wallet validation remain outstanding.".into()],
            });
        }
    }
    InventoryGroup {
        id: "declared-ark".to_string(),
        title: "Ark explicitly declared by this scenario".to_string(),
        purpose: "Separate from the not-running required integrations below.",
        rows,
    }
}

fn namespace_group(index: &EvidenceIndex, recorded: bool, now_ms: u64) -> InventoryGroup {
    InventoryGroup {
        id: "namespace-broker".to_string(),
        title: "Namespace broker".to_string(),
        purpose: "Enabled by --topology; owns network faults, not a user control UI.",
        rows: vec![InventoryRow {
            id: "namespace-broker".to_string(),
            process: "namespace broker and holders".to_string(),
            purpose: "Owns endpoint network namespaces and declared network faults.".to_string(),
            declared_role: "optional --topology backend (enabled)".to_string(),
            logical_roles: vec![
                "namespace ownership".to_string(),
                "network faults".to_string(),
            ],
            recorded_state: "outside_scenario_process_records".to_string(),
            observation: observation(index.broker.as_ref(), now_ms, recorded),
            interfaces: vec![interface(
                "Controller-owned backend",
                "none",
                None,
                "No standalone user UI and no new browser control API.",
            )],
            dependencies: vec!["spc-lab controller".to_string()],
            details: vec![
                "Namespaced actor addresses are internal transport, not host links.".to_string(),
            ],
        }],
    }
}

fn integrations(engine: &RunEngine) -> Vec<IntegrationView> {
    let agent_declared = engine
        .runtime_miner_specs()
        .iter()
        .any(|(_, implementation, _, _)| implementation == "spc-agent");
    let mut values = vec![
        IntegrationView {
            id: "spc-agent".to_string(),
            title: "spc-agent translator / JDC".to_string(),
            current_status: if agent_declared {
                "declared_in_scenario"
            } else {
                "not_running"
            },
            required_next_profile: false,
            ui: "No role-native browser UI",
            details: vec![
                "The binary exists but the standard lab does not launch it; absence is not a down status."
                    .to_string(),
                "The standard scenario launches no agent wallet and wires no live registration or per-user wallet UI."
                    .to_string(),
            ],
        },
        IntegrationView {
            id: "independent-ark-server".to_string(),
            title: "Independent Ark server".to_string(),
            current_status: if engine.scenario.ark_backends.is_empty() {
                "not_running_planned_required"
            } else {
                "declared_in_scenario"
            },
            required_next_profile: true,
            ui: "No role-native browser UI",
            details: vec![
                "Required in the expanded live lab; not running in this experiment unless explicitly declared above."
                    .to_string(),
                "The expanded profile requires an independent server rather than a simulated pool-local substitute."
                    .to_string(),
            ],
        },
        IntegrationView {
            id: "spc-user-wallets".to_string(),
            title: "Every SPc user: registration, receive and refresh".to_string(),
            current_status: "not_running_planned_required",
            required_next_profile: true,
            ui: "Wallet desktop/mobile UIs are not implemented",
            details: vec![
                "Required per-user Ark wallet capability in the next lab profile.".to_string(),
                "Miner/SPc user wallets are distinct from each pool operator's Ark float wallet."
                    .to_string(),
                "No wallet or spc-agent wallet capability is claimed from this experiment.".to_string(),
            ],
        },
        IntegrationView {
            id: "namespace-broker".to_string(),
            title: "Namespace broker (--topology)".to_string(),
            current_status: "conditional_on_topology",
            required_next_profile: false,
            ui: "No standalone browser UI",
            details: vec![
                "Optional topology-owned network faults; this entry is not a process health status."
                    .to_string(),
            ],
        },
    ];
    for pool in &engine.scenario.pools {
        let declared = engine
            .scenario
            .fronts
            .iter()
            .filter(|front| front.pool_id == pool.id)
            .any(|front| front.roles.contains(&FrontRole::Ark))
            || engine
                .scenario
                .ark_backends
                .iter()
                .any(|b| b.local.is_some());
        values.push(IntegrationView {
            id: format!("ark-delivery-{}", pool.id),
            title: format!("Ark delivery and operator float: {}", pool.id),
            current_status: if declared {
                "declared_in_scenario"
            } else {
                "not_running_planned_required"
            },
            required_next_profile: true,
            ui: "No role-native browser UI",
            details: vec![
                "Each pool requires its own Ark delivery role and float in the expanded live lab."
                    .to_string(),
                "The pool operator Ark wallet is distinct from every SPc user's receive wallet."
                    .to_string(),
                "The lab's throwaway payout setup is not a real operator wallet or real funds control."
                    .to_string(),
            ],
        });
    }
    values
}

fn role_name(role: FrontRole) -> String {
    match role {
        FrontRole::Front => "front".to_string(),
        FrontRole::Payout => "payout".to_string(),
        FrontRole::Api => "api".to_string(),
        FrontRole::Ark => "ark".to_string(),
    }
}

fn interface(
    label: impl Into<String>,
    kind: &'static str,
    url: Option<String>,
    note: &'static str,
) -> InterfaceView {
    InterfaceView {
        label: label.into(),
        kind,
        url,
        note,
    }
}

fn browser_url(endpoint: &Endpoint, path: &str, namespaced: bool) -> Option<String> {
    if namespaced
        || !endpoint
            .host
            .parse::<IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
    {
        return None;
    }
    // Only the two known unauthenticated pool API routes reach this helper.
    Some(format!(
        "http://{}:{}{}",
        endpoint.host, endpoint.port, path
    ))
}
