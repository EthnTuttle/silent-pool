//! Nominal lab identifiers with transparent serde representations.
//!
//! Run/network/pool/front IDs belong to LP-3 rather than this process. This
//! miner never invents an accepted `ShareId` or node `CandidateId`; the u32
//! submit sequence and local block-qualified flag are deliberately not those
//! evidence states.

use std::fmt;

use serde::{Deserialize, Serialize};

macro_rules! string_id {
    ($name:ident) => {
        #[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn into_inner(self) -> String {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl std::ops::Deref for $name {
            type Target = str;

            fn deref(&self) -> &str {
                &self.0
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl std::borrow::Borrow<str> for $name {
            fn borrow(&self) -> &str {
                &self.0
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.into())
            }
        }

        impl PartialEq<str> for $name {
            fn eq(&self, other: &str) -> bool {
                self.0 == other
            }
        }

        impl PartialEq<&str> for $name {
            fn eq(&self, other: &&str) -> bool {
                self.0 == *other
            }
        }

        impl PartialEq<String> for $name {
            fn eq(&self, other: &String) -> bool {
                &self.0 == other
            }
        }
    };
}

string_id!(CommandId);
string_id!(ConnectionId);
string_id!(IdentityId);
string_id!(MinerId);
string_id!(WorkerId);

/// SV2 wire job IDs remain u32 on serialization, but cannot accidentally be
/// confused with channel IDs, submit sequence numbers or worker IDs.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct JobId(pub u32);

impl fmt::Display for JobId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl From<u32> for JobId {
    fn from(value: u32) -> Self {
        Self(value)
    }
}

impl From<JobId> for u32 {
    fn from(value: JobId) -> Self {
        value.0
    }
}

impl PartialEq<u32> for JobId {
    fn eq(&self, other: &u32) -> bool {
        self.0 == *other
    }
}
