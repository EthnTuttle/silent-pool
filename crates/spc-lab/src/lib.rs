//! Headless regtest lab foundation.
//!
//! The crate owns the versioned scenario vocabulary, exact target helpers,
//! durable command log, append-only evidence, independent credit oracle,
//! process supervisor, resource allocation and the HTTP/SSE view used by the
//! thin browser. It does not contain pool accounting or SV2 business logic.

#![deny(unsafe_code)]

pub mod access;
pub mod adapters;
pub mod ark_service;
pub mod artifacts;
pub mod command;
pub mod config;
pub mod convergence;
pub mod engine;
pub mod error;
pub mod evidence;
pub mod http;
pub mod ids;
pub mod inventory;
pub mod links;
pub mod manifest;
pub mod namespace;
pub mod oracle;
pub mod resource;
pub mod runtime;
pub mod scenario;
pub mod secrets;
pub mod spc_evidence;
pub mod supervisor;
pub mod topology;
pub mod traffic;
pub mod wallet_bridge;
pub mod wallet_control;
pub mod wallet_projection;
pub mod wide;

pub use command::{CommandKind, CommandLog, CommandOutcome, LabCommand, OutcomeState};
pub use config::LabConfig;
pub use engine::{EngineOptions, RunEngine};
pub use evidence::{CanonicalResult, EvidenceEvent, EvidencePayload, EvidenceStore};
pub use ids::{
    ArkBackendId, CandidateId, CommandId, ConnectionId, FrontId, IdentityId, JobId, LinkId,
    MinerId, NetworkId, NodeId, PoolId, RunId, ShareId, SourceId, WorkerId,
};
pub use manifest::{RunManifest, RunState};
pub use namespace::{BrokerManifest, NamespaceBroker, Placement, PublishedAddress};
pub use scenario::{parse_scenario, Scenario, ScenarioError, SCENARIO_VERSION};
pub use supervisor::{ProcessRecord, ProcessSpec, Supervisor};
pub use traffic::{
    check_traffic_capability, project_traffic_capability, TrafficCapabilityRecord, TrafficSpec,
};
pub use wide::{ReferenceProfile, Target256, WideU128};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
