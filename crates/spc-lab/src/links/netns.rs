//! The network-namespace link backend.
//!
//! The backend does not implement faults itself: it drives the approved LP-4a
//! namespace broker, which owns the namespaces, the veth pairs and the
//! nftables rules.  A sustained partition is therefore the same object the
//! LP-4a precursor proved: an interface scoped, family agnostic drop rule in
//! both directions, installed as one atomic nftables transaction.
//!
//! Two properties are deliberately *not* claimed here:
//!
//! * a fixture backend can never satisfy a sustained partition, and
//! * the broker dying loses the links.  The backend reports that state
//!   instead of pretending the fault survived.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use crate::ids::LinkId;
use crate::namespace::{NamespaceBroker, NamespaceError};
use crate::scenario::{LinkPolicy, LinkSpec, Scenario};

use super::{LinkBackend, LinkError};

/// The netns backend over the approved namespace broker.
#[derive(Debug)]
pub struct NetnsLinkBackend {
    broker: Option<Arc<NamespaceBroker>>,
    applied: Mutex<BTreeMap<LinkId, LinkPolicy>>,
}

impl NetnsLinkBackend {
    /// A backend over the broker.
    ///
    /// Without a broker the backend can still describe itself, but it cannot
    /// install a fault: the preflight uses that to tell "the netns backend is
    /// not wired" from "the netns backend failed".
    pub fn new(broker: Option<Arc<NamespaceBroker>>) -> Self {
        Self {
            broker,
            applied: Mutex::new(BTreeMap::new()),
        }
    }

    /// True when a broker is attached.
    pub fn is_wired(&self) -> bool {
        self.broker.is_some()
    }

    pub fn broker(&self) -> Option<&Arc<NamespaceBroker>> {
        self.broker.as_ref()
    }

    /// The raw per-link counters the broker collected.
    pub fn counters(&self, link: &str) -> Result<Value, LinkError> {
        self.broker
            .as_ref()
            .ok_or_else(|| LinkError::Unavailable("no namespace broker is attached".to_string()))?
            .counters(link)
            .map_err(|error| LinkError::Unavailable(error.to_string()))
    }

    /// The broker's no-bypass proof for the current topology.
    pub fn no_bypass_check(&self) -> Result<Value, LinkError> {
        self.broker
            .as_ref()
            .ok_or_else(|| LinkError::Unavailable("no namespace broker is attached".to_string()))?
            .no_bypass_check()
            .map_err(|error| LinkError::Unavailable(error.to_string()))
    }

    /// Remove a fault from a link.
    pub fn heal(&self, link: &LinkSpec) -> Result<(), LinkError> {
        self.broker
            .as_ref()
            .ok_or_else(|| LinkError::Unavailable("no namespace broker is attached".to_string()))?
            .heal(link.id.as_str())
            .map_err(|error| LinkError::Unavailable(error.to_string()))?;
        self.applied
            .lock()
            .expect("link policy lock")
            .insert(link.id.clone(), LinkPolicy::Normal);
        Ok(())
    }

    /// The published addresses of the topology, for the manifests.
    pub fn placement(&self) -> Option<crate::namespace::Placement> {
        self.broker
            .as_ref()
            .map(|broker| crate::namespace::Placement::from_manifest(broker.manifest()))
    }
}

fn policy_json(policy: &LinkPolicy) -> Value {
    match policy {
        LinkPolicy::Normal => json!({"type": "normal"}),
        LinkPolicy::Partition => json!({"type": "partition"}),
        LinkPolicy::Latency { delay_ms } => json!({"type": "latency", "delay_ms": delay_ms}),
        LinkPolicy::Loss { loss_percent } => json!({"type": "loss", "loss_percent": loss_percent}),
    }
}

fn policy_from_json(value: &Value) -> Option<LinkPolicy> {
    match value.get("type").and_then(Value::as_str)? {
        "normal" => Some(LinkPolicy::Normal),
        "partition" => Some(LinkPolicy::Partition),
        "latency" => Some(LinkPolicy::Latency {
            delay_ms: value.get("delay_ms").and_then(Value::as_u64)?,
        }),
        "loss" => Some(LinkPolicy::Loss {
            loss_percent: value.get("loss_percent").and_then(Value::as_u64)? as u8,
        }),
        _ => None,
    }
}

impl LinkBackend for NetnsLinkBackend {
    fn name(&self) -> &'static str {
        "netns"
    }

    fn capabilities(&self) -> Vec<String> {
        vec![
            "lab.link.netns".to_string(),
            "lab.link.sustained_partition".to_string(),
            "lab.link.no_bypass".to_string(),
            "lab.link.ipv6".to_string(),
        ]
    }

    fn supports_sustained_fault(&self) -> bool {
        true
    }

    fn preflight(&self, scenario: &Scenario) -> Result<(), LinkError> {
        let Some(broker) = self.broker.as_ref() else {
            return Err(LinkError::Unavailable(
                "no namespace broker is attached".to_string(),
            ));
        };
        let manifest = broker.manifest();
        for link in &scenario.links {
            let published = manifest.link(link.id.as_str()).ok_or_else(|| {
                LinkError::Unavailable(format!(
                    "the namespace broker did not publish link {}",
                    link.id
                ))
            })?;
            for endpoint in [published.from.as_str(), published.to.as_str()] {
                if manifest.endpoint(endpoint).is_none() {
                    return Err(LinkError::Unavailable(format!(
                        "the namespace broker did not publish endpoint {endpoint}"
                    )));
                }
            }
        }
        Ok(())
    }

    fn apply(&self, link: &LinkSpec, policy: &LinkPolicy) -> Result<(), LinkError> {
        let broker = self
            .broker
            .as_ref()
            .ok_or_else(|| LinkError::Unavailable("no namespace broker is attached".to_string()))?;
        let result = match policy {
            LinkPolicy::Normal => broker.heal(link.id.as_str()),
            other => broker.apply(link.id.as_str(), policy_json(other)),
        };
        match result {
            Ok(value) => {
                let Some(applied) = policy_from_json(value.get("policy").unwrap_or(&Value::Null))
                else {
                    self.applied
                        .lock()
                        .expect("link policy lock")
                        .remove(&link.id);
                    return Err(LinkError::Partial(
                        "broker returned a missing or malformed policy".to_string(),
                    ));
                };
                if &applied != policy {
                    self.applied
                        .lock()
                        .expect("link policy lock")
                        .remove(&link.id);
                    return Err(LinkError::Partial(format!(
                        "broker applied {applied}, not the requested {policy}"
                    )));
                }
                self.applied
                    .lock()
                    .expect("link policy lock")
                    .insert(link.id.clone(), applied);
                Ok(())
            }
            Err(error) => {
                // A rejected or failed application never leaves a policy
                // behind: the cache would claim a fault that is not installed.
                self.applied
                    .lock()
                    .expect("link policy lock")
                    .remove(&link.id);
                Err(map_error(error, policy, &link.id))
            }
        }
    }

    fn current(&self, link: &LinkId) -> Option<LinkPolicy> {
        // The broker is the source of truth.  The cache is only a fallback for
        // a broker that cannot answer, and `None` means "unknown", never
        // "normal".
        let broker = self.broker.as_ref()?;
        if let Ok(status) = broker.status() {
            if let Some(link_status) =
                status
                    .get("links")
                    .and_then(Value::as_array)
                    .and_then(|links| {
                        links.iter().find(|item| {
                            item.get("id").and_then(Value::as_str) == Some(link.as_str())
                        })
                    })
            {
                if let Some(policy) = link_status.get("policy").and_then(policy_from_json) {
                    self.applied
                        .lock()
                        .expect("link policy lock")
                        .insert(link.clone(), policy.clone());
                    return Some(policy);
                }
                // partial/malformed policy means unknown, never Normal.
                self.applied.lock().ok()?.remove(link);
                return None;
            }
        }
        // The broker did not answer: the state of the link is unknown.
        self.applied.lock().ok()?.remove(link);
        None
    }
}

/// Map a broker error onto the link error vocabulary.
///
/// The mapping is exact: a policy the broker rejects is not the same thing as
/// a backend that cannot express a policy, and a half applied fault is
/// neither.
fn map_error(error: NamespaceError, policy: &LinkPolicy, link: &LinkId) -> LinkError {
    let NamespaceError::Rejected { code, detail, .. } = &error else {
        return LinkError::Unavailable(error.to_string());
    };
    match code.as_str() {
        "unknown_link" => LinkError::UnknownLink(link.clone()),
        "invalid_policy" => LinkError::Unsupported("netns".to_string(), policy.clone()),
        "busy" => LinkError::Busy(detail.clone()),
        "partial" | "policy_apply_failed" => LinkError::Partial(detail.clone()),
        other => LinkError::Unavailable(format!("broker error {other}: {detail}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes_not_message_substrings_drive_classification() {
        let link = LinkId::new("link-a").unwrap();
        let policy = LinkPolicy::Partition;
        for (code, expected) in [
            ("invalid_policy", "unsupported"),
            ("busy", "busy"),
            ("partial", "partial"),
            ("policy_apply_failed", "partial"),
            ("busy_but_not_a_real_code", "unavailable"),
        ] {
            let error = map_error(
                NamespaceError::Rejected {
                    op: "apply".to_string(),
                    code: code.to_string(),
                    detail: "the message intentionally uses other words".to_string(),
                },
                &policy,
                &link,
            );
            let actual = match error {
                LinkError::Unsupported(..) => "unsupported",
                LinkError::Busy(_) => "busy",
                LinkError::Partial(_) => "partial",
                LinkError::Unavailable(_) => "unavailable",
                LinkError::UnknownLink(_) => "unknown_link",
            };
            assert_eq!(actual, expected, "code {code}");
        }
    }

    #[test]
    fn partial_or_malformed_policy_is_unknown_not_normal() {
        for status in [
            json!({"type": "partial"}),
            json!({"type": "bogus"}),
            json!({}),
        ] {
            assert_eq!(policy_from_json(&status), None);
        }
        assert_eq!(
            policy_from_json(&json!({"type": "normal"})),
            Some(LinkPolicy::Normal)
        );
    }
}
