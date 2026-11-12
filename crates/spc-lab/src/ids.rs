//! Nominal identifiers for the lab scenario and evidence model.
//!
//! The lab deliberately does not use bare `String`s for identity-bearing
//! concepts. A [`NodeId`] cannot be passed where a [`PoolId`] is expected, and
//! an identifier is validated when it is constructed.

use std::fmt;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Error returned for an empty, oversized or control-character identifier.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid {kind} {value:?}: {reason}")]
pub struct IdError {
    pub kind: &'static str,
    pub value: String,
    pub reason: &'static str,
}

fn validate_id(kind: &'static str, value: &str) -> Result<(), IdError> {
    if value.is_empty() {
        return Err(IdError {
            kind,
            value: value.to_string(),
            reason: "identifier is empty",
        });
    }
    if value.len() > 128 {
        return Err(IdError {
            kind,
            value: value.to_string(),
            reason: "identifier exceeds 128 bytes",
        });
    }
    if value.chars().any(|c| c.is_control()) {
        return Err(IdError {
            kind,
            value: value.to_string(),
            reason: "identifier contains a control character",
        });
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':' | '/' | '@'))
    {
        return Err(IdError {
            kind,
            value: value.to_string(),
            reason: "identifier contains an unsupported character",
        });
    }
    Ok(())
}

macro_rules! id_type {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, IdError> {
                let value = value.into();
                validate_id(stringify!($name), &value)?;
                Ok(Self(value))
            }

            /// An identifier assembled from already-validated ASCII input.
            ///
            /// Keep this for tests and trusted internal composition; external
            /// input goes through [`Self::new`].
            pub fn trusted(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn into_string(self) -> String {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl FromStr for $name {
            type Err = IdError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::new(value)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }
    };
}

id_type!(
    /// Durable identity of one supervised experiment.
    RunId
);
id_type!(
    /// The nodes that validate the same consensus chain.
    NetworkId
);
id_type!(
    /// One node process and its RPC/TDP/P2P listeners.
    NodeId
);
id_type!(
    /// One accounting domain with its own Redis and Postgres resources.
    PoolId
);
id_type!(
    /// One front process/listener belonging to exactly one pool.
    FrontId
);
id_type!(
    /// A miner identity (BIP 352 address material lives in private storage).
    IdentityId
);
id_type!(
    /// A nonce/search partition under one identity and connection.
    WorkerId
);
id_type!(
    /// One miner driver process.
    MinerId
);
id_type!(
    /// One SV2 session binding an identity to a front.
    ConnectionId
);
id_type!(
    /// One job generation carrying a share target and work profile.
    JobId
);
id_type!(
    /// One share that reached pool accounting.
    ShareId
);
id_type!(
    /// One block-qualified submission to a node.
    CandidateId
);
id_type!(
    /// Idempotency key for one client command.
    CommandId
);
id_type!(
    /// One controlled link between two endpoints.
    LinkId
);
id_type!(
    /// One test Ark service.
    ArkBackendId
);
id_type!(
    /// The actor that issued a command or observation.
    ActorId
);
id_type!(
    /// A per-source cursor namespace (Redis stream, node poll, process counter).
    SourceId
);

impl CommandId {
    /// Generate an identifier unique within a process and extremely unlikely
    /// to collide across processes on the same host.
    pub fn generate() -> Self {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        Self(format!("cmd-{nanos:032x}-{seq:016x}"))
    }
}

impl RunId {
    pub fn generate() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        Self(format!("run-{nanos:032x}"))
    }
}

impl SourceId {
    pub fn fixture(name: impl Into<String>) -> Result<Self, IdError> {
        Self::new(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_validate_and_are_nominal() {
        assert!(RunId::new("run-a").is_ok());
        assert!(RunId::new("").is_err());
        assert!(RunId::new("bad\nvalue").is_err());
        assert!(CommandId::generate().as_str().starts_with("cmd-"));
        let a = NodeId::new("node-a").unwrap();
        assert_eq!(a.as_str(), "node-a");
    }
}
