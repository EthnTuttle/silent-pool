//! Topology planning, publication and node scoped canonical observation.
//!
//! This module is deliberately isolated from `runtime.rs` and `engine.rs`: it
//! contains no line-position dependent hooks, so it can be merged into the
//! runtime after the runtime's own freeze.  The runtime supplies the probes
//! and the launch context; this module owns the topology, the publication and
//! the observation.
//!
//! Three things live here:
//!
//! * [`TopologyPlan`]: the scenario's actors and links turned into the
//!   endpoint/link set the namespace broker has to realise, plus the mapping
//!   from an actor to the endpoint it lives in.
//! * [`CanonicalObserver`]: node scoped canonical observation.  The runtime's
//!   own observer is a *tip sampler*: it re-reads the tip and can therefore
//!   never mark a displaced height as `no`.  A multi node partition needs
//!   more than that, so this observer keeps the last K observed
//!   `(height, hash)` pairs per node and re-checks them.  A height whose
//!   canonical hash changed is reported `no` (displaced), which is what a
//!   reorg looks like from the outside.
//! * [`TopologyPublication`]: the shareable and private projections of the
//!   placement.  Addresses and links are shareable; nothing identity bearing
//!   and no secret ever enters the shareable projection.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::scenario::Scenario;

/// The endpoint that carries the traffic of a scenario link endpoint.
///
/// A pool is not a namespace: its traffic comes from the front of the pool.
fn resolve_endpoint(
    scenario: &Scenario,
    kind: crate::scenario::EndpointKind,
    id: &str,
) -> Option<String> {
    use crate::scenario::EndpointKind;
    match kind {
        EndpointKind::Node => Some(id.to_string()),
        EndpointKind::Front => Some(id.to_string()),
        EndpointKind::Miner => Some(id.to_string()),
        EndpointKind::Pool => scenario
            .fronts
            .iter()
            .find(|front| {
                scenario
                    .pools
                    .iter()
                    .any(|pool| pool.id.to_string() == id && pool.id == front.pool_id)
            })
            .map(|front| front.id.to_string()),
        EndpointKind::Identity
        | EndpointKind::Ark
        | EndpointKind::Redis
        | EndpointKind::Postgres => None,
    }
}

/// One planned endpoint of the topology.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedEndpoint {
    /// The broker endpoint name.  It is the scenario entity id, so the
    /// placement never has to be guessed from a generated configuration.
    pub name: String,
    /// The role the endpoint plays.
    pub role: String,
    /// The actors that live in this endpoint.
    pub actors: Vec<String>,
}

/// One planned link of the topology.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedLink {
    pub id: String,
    pub a: String,
    pub b: String,
    /// The plane the link belongs to.
    pub plane: LinkPlane,
}

/// The plane a link belongs to.
///
/// The controller plane (the runtime's own API) is never a broker link: the
/// controller is not an actor of the network topology, and keeping it out of
/// the topology is what makes "no accidental bypass" measurable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkPlane {
    /// A link the scenario declared that is not node to node.
    Declared,
    /// Node to node consensus traffic.
    Consensus,
    /// Pool/front to node template and chain view traffic.
    PoolNode,
    /// Miner to front SV2 traffic.
    MinerFront,
    /// Pool/front to its private storage.
    Storage,
}

/// The plan of one topology.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopologyPlan {
    pub endpoints: Vec<PlannedEndpoint>,
    pub links: Vec<PlannedLink>,
    /// Actor role+entity -> the broker endpoint it lives in.
    pub actor_endpoints: BTreeMap<String, String>,
    /// The ids of the node to node links.  A scenario may declare more than
    /// one consensus link; all of them are kept.
    pub consensus_links: Vec<String>,
}

impl TopologyPlan {
    /// Derive the plan from a scenario.
    ///
    /// The rules are explicit and do not depend on the runtime:
    ///
    /// * every node is its own endpoint;
    /// * every front is its own endpoint (a front is a process with its own
    ///   listeners), and the front owns the storage endpoint of its pool;
    /// * every miner is its own endpoint, linked only to its front;
    /// * the node to node links are the consensus links, and the scenario's
    ///   link ids are used verbatim so a scenario can name a link.
    pub fn from_scenario(scenario: &Scenario) -> Result<Self, String> {
        let mut plan = TopologyPlan::default();
        let mut endpoints: Vec<PlannedEndpoint> = Vec::new();
        let mut links: Vec<PlannedLink> = Vec::new();
        let mut actor_endpoints: BTreeMap<String, String> = BTreeMap::new();

        for node in &scenario.nodes {
            endpoints.push(PlannedEndpoint {
                name: node.id.to_string(),
                role: "node".to_string(),
                actors: vec![format!("node/{}", node.id)],
            });
            actor_endpoints.insert(format!("node/{}", node.id), node.id.to_string());
        }
        let mut storage_ids: BTreeSet<String> = BTreeSet::new();
        for pool in &scenario.pools {
            let storage = format!("storage-{}", pool.id);
            if !storage_ids.insert(storage.clone()) {
                return Err(format!("the storage endpoint {storage} is generated twice"));
            }
            endpoints.push(PlannedEndpoint {
                name: storage.clone(),
                role: "storage".to_string(),
                // Redis alone inhabits this namespace. PostgreSQL is planned
                // as a host-user/host-namespace process and is reached by the
                // front through its private Unix socket.
                actors: vec![format!("redis/{}", pool.storage.redis_instance_id)],
            });
            actor_endpoints.insert(format!("redis/{}", pool.storage.redis_instance_id), storage);
        }
        for front in &scenario.fronts {
            let pool = scenario
                .pools
                .iter()
                .find(|pool| pool.id == front.pool_id)
                .ok_or_else(|| format!("front {} names an unknown pool", front.id))?;
            endpoints.push(PlannedEndpoint {
                name: front.id.to_string(),
                role: "front".to_string(),
                actors: vec![format!("front/{}", front.id)],
            });
            actor_endpoints.insert(format!("front/{}", front.id), front.id.to_string());
            // The front of a pool reaches that pool's node and that pool's
            // storage.  A front never reaches another pool's storage.
            for node_id in &pool.node_ids {
                links.push(PlannedLink {
                    id: format!("{}--{}--node", front.id, node_id),
                    a: front.id.to_string(),
                    b: node_id.to_string(),
                    plane: LinkPlane::PoolNode,
                });
            }
            links.push(PlannedLink {
                id: format!("{}--storage", front.id),
                a: front.id.to_string(),
                b: format!("storage-{}", pool.id),
                plane: LinkPlane::Storage,
            });
        }
        for miner in &scenario.miners {
            endpoints.push(PlannedEndpoint {
                name: miner.id.to_string(),
                role: "miner".to_string(),
                actors: vec![format!("miner/{}", miner.id)],
            });
            actor_endpoints.insert(format!("miner/{}", miner.id), miner.id.to_string());
            for connection in &miner.connections {
                links.push(PlannedLink {
                    id: format!("{}--{}--front", miner.id, connection.front_id),
                    a: miner.id.to_string(),
                    b: connection.front_id.to_string(),
                    plane: LinkPlane::MinerFront,
                });
            }
        }
        let generated_ids: BTreeSet<String> = links.iter().map(|link| link.id.clone()).collect();
        // Every link the scenario declares keeps its id. The plan never
        // renames a scenario link: fault commands and assertions use the ids.
        let mut seen_ids: BTreeSet<String> = BTreeSet::new();
        for link in &scenario.links {
            let id = link.id.to_string();
            if !seen_ids.insert(id.clone()) {
                return Err(format!("the scenario declares the link {id} twice"));
            }
            // A pool link is realised by the front of that pool: the front is
            // the process that talks to the node.
            let a = resolve_endpoint(scenario, link.from.kind, &link.from.id).ok_or_else(|| {
                format!(
                    "the link {id} names the endpoint {}, which the plan does not place",
                    link.from.id
                )
            })?;
            let b = resolve_endpoint(scenario, link.to.kind, &link.to.id).ok_or_else(|| {
                format!(
                    "the link {id} names the endpoint {}, which the plan does not place",
                    link.to.id
                )
            })?;
            if !endpoints.iter().any(|endpoint| endpoint.name == a) {
                return Err(format!(
                    "the link {id} names the endpoint {a}, which the plan does not place"
                ));
            }
            if !endpoints.iter().any(|endpoint| endpoint.name == b) {
                return Err(format!(
                    "the link {id} names the endpoint {b}, which the plan does not place"
                ));
            }
            let consensus = scenario.nodes.iter().any(|node| node.id.to_string() == a)
                && scenario.nodes.iter().any(|node| node.id.to_string() == b);
            if consensus {
                plan.consensus_links.push(id.clone());
            }
            links.push(PlannedLink {
                id,
                a,
                b,
                plane: if consensus {
                    LinkPlane::Consensus
                } else {
                    LinkPlane::Declared
                },
            });
        }
        // A declared miner/front or pool/node link is the one data-plane link
        // the runtime configures and faults. Never add a parallel generated
        // route for the same pair: that would let traffic bypass the scenario
        // command's named link.
        let declared_pairs: BTreeSet<(String, String)> = links
            .iter()
            .filter(|link| link.plane == LinkPlane::Declared || link.plane == LinkPlane::Consensus)
            .map(|link| {
                if link.a <= link.b {
                    (link.a.clone(), link.b.clone())
                } else {
                    (link.b.clone(), link.a.clone())
                }
            })
            .collect();
        links.retain(|link| {
            if link.plane != LinkPlane::PoolNode && link.plane != LinkPlane::MinerFront {
                return true;
            }
            let pair = if link.a <= link.b {
                (link.a.clone(), link.b.clone())
            } else {
                (link.b.clone(), link.a.clone())
            };
            !declared_pairs.contains(&pair)
        });
        // Preserve the useful plane labels on declared links as well.
        for link in &mut links {
            if link.plane != LinkPlane::Declared {
                continue;
            }
            let miner_link = scenario.miners.iter().any(|miner| {
                let id = miner.id.to_string();
                (link.a == id
                    && scenario
                        .fronts
                        .iter()
                        .any(|front| front.id.to_string() == link.b))
                    || (link.b == id
                        && scenario
                            .fronts
                            .iter()
                            .any(|front| front.id.to_string() == link.a))
            });
            if miner_link {
                link.plane = LinkPlane::MinerFront;
            } else {
                let node_link = scenario.nodes.iter().any(|node| {
                    let id = node.id.to_string();
                    (link.a == id
                        && scenario
                            .fronts
                            .iter()
                            .any(|front| front.id.to_string() == link.b))
                        || (link.b == id
                            && scenario
                                .fronts
                                .iter()
                                .any(|front| front.id.to_string() == link.a))
                });
                if node_link {
                    link.plane = LinkPlane::PoolNode;
                }
            }
        }
        // The generated links may not collide with a declared id.
        let declared_ids = seen_ids.clone();
        if let Some(id) = generated_ids.intersection(&declared_ids).next() {
            return Err(format!(
                "the generated link {id} collides with a declared link id"
            ));
        }
        plan.endpoints = endpoints;
        plan.links = links;
        plan.actor_endpoints = actor_endpoints;
        Ok(plan)
    }

    /// The broker topology document (the LP-4a spec schema).
    ///
    /// Every link the scenario declared is published with its own id, so a
    /// fault command can name it and the backend preflight finds it.
    pub fn broker_spec(&self) -> Value {
        json!({
            "schema": "spc.lab.link_topology.v1",
            "endpoints": self
                .endpoints
                .iter()
                .map(|endpoint| json!({"name": endpoint.name, "role": endpoint.role}))
                .collect::<Vec<_>>(),
            "links": self
                .links
                .iter()
                .map(|link| json!({"id": link.id, "a": link.a, "b": link.b}))
                .collect::<Vec<_>>(),
        })
    }

    /// The declared link ids of the scenario, in order.
    pub fn declared_link_ids(&self) -> Vec<String> {
        self.links
            .iter()
            .filter(|link| link.plane == LinkPlane::Declared || link.plane == LinkPlane::Consensus)
            .map(|link| link.id.clone())
            .collect()
    }

    pub fn endpoint(&self, name: &str) -> Option<&PlannedEndpoint> {
        self.endpoints.iter().find(|item| item.name == name)
    }

    /// The endpoint an actor lives in.
    pub fn endpoint_of(&self, actor: &str) -> Option<&str> {
        self.actor_endpoints.get(actor).map(String::as_str)
    }

    /// The links of one plane.
    pub fn links_of_plane(&self, plane: LinkPlane) -> Vec<&PlannedLink> {
        self.links
            .iter()
            .filter(|link| link.plane == plane)
            .collect()
    }
}

/// One observed canonical height of a node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedHeight {
    pub height: u64,
    pub hash: String,
    pub observed_at_ms: u64,
}

/// One canonical observation of a node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalObservation {
    pub node: String,
    pub height: u64,
    /// `yes` when the node still reports the observed hash at this height,
    /// `no` when it reports a different one (a **confirmed** displacement),
    /// `unknown` when the node could not be asked.
    ///
    /// A failed read is never a `no`: it must not invent a displacement.
    pub result: String,
    pub observed_hash: Option<String>,
    pub detail: Option<String>,
    /// The scope of the observation.  The runtime publishes `tip_sampler`:
    /// the heights are not continuously observed, so this is not full
    /// canonical coverage and not reorg tracking.
    pub scope: String,
}

/// The probe the observer needs from the runtime.
///
/// The runtime owns the node RPC client; this module only asks for hashes.
pub trait CanonicalProbe {
    /// The canonical hash of `node` at `height`, if the node still has it.
    fn hash_at(&mut self, node: &str, height: u64) -> Result<Option<String>, String>;
    /// An **atomic** tip view of `node` as `(height, hash)`.
    ///
    /// The two values have to describe the same block.  A height read from
    /// one call and a hash read from another call race with every new block,
    /// and the mismatch would look like a displacement that never happened.
    fn tip(&mut self, node: &str) -> Result<(u64, String), String>;
}

/// Node scoped canonical observation with a bounded history.
///
/// The observer keeps the last `depth` observed `(height, hash)` pairs per
/// node and re-checks them on every pass.  A height whose hash changed is
/// reported `no`, which is the only way a tip sampler can ever notice a
/// reorg.
#[derive(Debug, Clone)]
pub struct CanonicalObserver {
    depth: usize,
    observed: BTreeMap<String, Vec<ObservedHeight>>,
}

/// The bounded recheck window of the runtime's node observer.
pub const RECHECK_WINDOW: usize = 8;

impl CanonicalObserver {
    pub fn new(depth: usize) -> Self {
        Self {
            depth: depth.max(1),
            observed: BTreeMap::new(),
        }
    }

    /// An observer with the runtime's bounded recheck window.
    pub fn with_runtime_window() -> Self {
        Self::new(RECHECK_WINDOW)
    }

    pub fn depth(&self) -> usize {
        self.depth
    }

    /// The heights the observer currently tracks for a node.
    pub fn tracked(&self, node: &str) -> &[ObservedHeight] {
        self.observed.get(node).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Observe every node once and return the per node observations.
    ///
    /// The tip is observed first and appended to the tracked history; the
    /// tracked heights are then re-checked against the node.
    pub fn observe(
        &mut self,
        nodes: &[String],
        probe: &mut dyn CanonicalProbe,
        now_ms: u64,
    ) -> Vec<CanonicalObservation> {
        let mut results = Vec::new();
        for node in nodes {
            let tip = match probe.tip(node) {
                Ok(tip) => tip,
                Err(error) => {
                    results.push(CanonicalObservation {
                        node: node.clone(),
                        height: 0,
                        result: "unknown".to_string(),
                        observed_hash: None,
                        detail: Some(error),
                        scope: SCOPE_TIP_SAMPLER.to_string(),
                    });
                    continue;
                }
            };
            // The tip view has to be atomic.  If the hash at the reported
            // height is not the reported hash, the two reads raced with a new
            // block: the pass is skipped as unknown instead of recording a
            // displacement that never happened.
            let confirmed = match probe.hash_at(node, tip.0) {
                Ok(Some(hash)) => hash == tip.1,
                Ok(None) => false,
                Err(_) => false,
            };
            if !confirmed {
                results.push(CanonicalObservation {
                    node: node.clone(),
                    height: tip.0,
                    result: "unknown".to_string(),
                    observed_hash: None,
                    detail: Some(
                        "the tip view was not atomic: the hash at the reported height did not                          match the reported hash"
                            .to_string(),
                    ),
                    scope: SCOPE_TIP_SAMPLER.to_string(),
                });
                continue;
            }
            let history = self.observed.entry(node.clone()).or_default();
            if !history
                .iter()
                .any(|item| item.height == tip.0 && item.hash == tip.1)
            {
                history.push(ObservedHeight {
                    height: tip.0,
                    hash: tip.1.clone(),
                    observed_at_ms: now_ms,
                });
            }
            history.sort_by_key(|item| item.height);
            let overflow = history.len().saturating_sub(self.depth);
            if overflow > 0 {
                history.drain(0..overflow);
            }
            // Re-check the bounded window.  A confirmed displacement (the node
            // reports a different hash) is `no` and the observation is
            // dropped; a failed read stays `unknown` and changes nothing.
            let tracked: Vec<ObservedHeight> = history.clone();
            for item in tracked {
                let current = match probe.hash_at(node, item.height) {
                    Ok(Some(hash)) => Some(hash),
                    Ok(None) => None,
                    Err(_) => None,
                };
                let (result, detail) = match current.as_deref() {
                    Some(hash) if hash != item.hash => (
                        "no",
                        Some(
                            "tip displacement observed by bounded last-K recheck (historical observation retained)"
                                .to_string(),
                        ),
                    ),
                    Some(_) => ("yes", None),
                    None => ("unknown", None),
                };
                if result == "no" {
                    history.retain(|entry| entry.height != item.height);
                }
                results.push(CanonicalObservation {
                    node: node.clone(),
                    height: item.height,
                    result: result.to_string(),
                    observed_hash: current,
                    detail,
                    scope: SCOPE_TIP_SAMPLER.to_string(),
                });
            }
        }
        results
    }
}

/// The scope name the runtime publishes for the bounded tip sampler.
pub const SCOPE_TIP_SAMPLER: &str = "tip_sampler";

/// The shareable and private projection of a placement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopologyPublication {
    /// Addresses, links and the namespace of every actor.  No identity
    /// bearing field and no secret.
    pub shareable: Value,
    /// The private projection: the endpoint to actor mapping and the broker
    /// run directory, which stays inside the run's private area.
    pub private: Value,
}

impl TopologyPublication {
    pub fn new(
        placement: &crate::namespace::Placement,
        plan: &TopologyPlan,
        broker_run_dir: &str,
    ) -> Self {
        Self::with_provenance(placement, plan, broker_run_dir, serde_json::json!({}))
    }

    /// The publication with the provenance of the run it belongs to.
    ///
    /// The runtime records the base revision, the dirty tree hash, the code
    /// snapshot hash and the store paths of every binary.  A topology run has
    /// to carry the same fields, or a reader cannot bind the artifact to the
    /// source that produced it.
    pub fn with_provenance(
        placement: &crate::namespace::Placement,
        plan: &TopologyPlan,
        broker_run_dir: &str,
        provenance: Value,
    ) -> Self {
        let shareable = json!({
            "schema": "spc.lab.topology_placement.v1",
            "addresses": placement.addresses,
            "links": placement.links,
            "link_addresses": placement.link_addresses,
            "planes": plan
                .links
                .iter()
                .map(|link| json!({"id": link.id, "plane": link.plane, "from": link.a, "to": link.b}))
                .collect::<Vec<_>>(),
        });
        let private = json!({
            "schema": "spc.lab.topology_private.v1",
            "broker_run_dir": broker_run_dir,
            "actor_endpoints": plan.actor_endpoints,
            "namespaces": placement.namespaces,
            "provenance": provenance,
        });
        Self { shareable, private }
    }
}
