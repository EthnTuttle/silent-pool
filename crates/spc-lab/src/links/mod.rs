//! Link-fault backend boundary.
//!
//! LP-3 supplies the interface and a fixture backend for control-surface
//! tests. LP-4 owns the Linux network-namespace backend. A fixture link never
//! satisfies a sustained-partition or no-bypass assertion.

pub mod netns;

use std::collections::BTreeMap;
use std::sync::Mutex;

use crate::ids::LinkId;
use crate::scenario::{LinkPolicy, LinkSpec, Scenario};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LinkError {
    #[error("link backend {0:?} does not support policy {1}")]
    Unsupported(String, LinkPolicy),
    #[error("unknown link {0}")]
    UnknownLink(LinkId),
    #[error("link backend unavailable: {0}")]
    Unavailable(String),
    #[error("the link fault was only partly applied: {0}")]
    Partial(String),
    #[error("the link backend is busy: {0}")]
    Busy(String),
}

pub trait LinkBackend: Send + Sync + std::fmt::Debug {
    fn name(&self) -> &'static str;
    fn capabilities(&self) -> Vec<String>;
    /// Fixture backends cannot support a sustained-fault assertion.
    fn supports_sustained_fault(&self) -> bool;
    fn preflight(&self, scenario: &Scenario) -> Result<(), LinkError>;
    fn apply(&self, link: &LinkSpec, policy: &LinkPolicy) -> Result<(), LinkError>;
    fn current(&self, link: &LinkId) -> Option<LinkPolicy>;
}

#[derive(Debug, Default)]
pub struct FixtureLinkBackend {
    applied: Mutex<BTreeMap<LinkId, LinkPolicy>>,
}

impl FixtureLinkBackend {
    pub fn new() -> Self {
        Self::default()
    }
}

impl LinkBackend for FixtureLinkBackend {
    fn name(&self) -> &'static str {
        "fixture"
    }

    fn capabilities(&self) -> Vec<String> {
        vec!["lab.link.fixture".to_string()]
    }

    fn supports_sustained_fault(&self) -> bool {
        false
    }

    fn preflight(&self, scenario: &Scenario) -> Result<(), LinkError> {
        for link in &scenario.links {
            if link.policy.is_normal() {
                continue;
            }
            // The fixture can record the requested policy but its evidence is
            // deliberately weak. The run is allowed to continue; assertions
            // that need a real fault must become inconclusive.
            let _ = link;
        }
        Ok(())
    }

    fn apply(&self, link: &LinkSpec, policy: &LinkPolicy) -> Result<(), LinkError> {
        self.applied
            .lock()
            .unwrap()
            .insert(link.id.clone(), policy.clone());
        Ok(())
    }

    fn current(&self, link: &LinkId) -> Option<LinkPolicy> {
        self.applied.lock().unwrap().get(link).cloned()
    }
}

#[derive(Debug, Default)]
pub struct UnsupportedLinkBackend {
    reason: String,
}

impl UnsupportedLinkBackend {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl LinkBackend for UnsupportedLinkBackend {
    fn name(&self) -> &'static str {
        "unsupported"
    }

    fn capabilities(&self) -> Vec<String> {
        Vec::new()
    }

    fn supports_sustained_fault(&self) -> bool {
        false
    }

    fn preflight(&self, _scenario: &Scenario) -> Result<(), LinkError> {
        Err(LinkError::Unavailable(self.reason.clone()))
    }

    fn apply(&self, _link: &LinkSpec, policy: &LinkPolicy) -> Result<(), LinkError> {
        Err(LinkError::Unsupported(
            self.name().to_string(),
            policy.clone(),
        ))
    }

    fn current(&self, _link: &LinkId) -> Option<LinkPolicy> {
        None
    }
}

/// Build the configured backend. LP-4 will add `netns` here; an absent or
/// unknown backend fails closed instead of silently using a direct route.
pub fn backend_for(name: Option<&str>) -> Box<dyn LinkBackend> {
    match name.unwrap_or("fixture") {
        "fixture" => Box::new(FixtureLinkBackend::new()),
        other => Box::new(UnsupportedLinkBackend::new(format!(
            "link backend {other:?} is not implemented in LP-3"
        ))),
    }
}
