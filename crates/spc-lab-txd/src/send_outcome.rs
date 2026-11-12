//! Exact pre-publication rejection rows. Numeric errors alone never release
//! inputs. One source-reviewed, actual-node-demonstrated version guard is
//! enabled for one exact release/source/profile tuple. All other rows remain
//! Unknown. See the 2026-11-10 acceptance follow-up decision and receipts.
use crate::control::RpcOutcome;

pub const REVIEWED_SOURCE: &str =
    "f1592c935e29b0b1fdf6429ad1b84e9ae925e9a9fcccd66b04f3d090680ff20f";
pub const REVIEWED_BINARY: &str =
    "bba4752298aff258adb6cce99c6db849b02f50736a8ec32766ed1710045051aa";
pub const REVIEWED_CONSENSUS: &str =
    "4634ab3e2ca66aa9d319b35d89131f7ad1fc48c3b315435652282fe9e20b79d3";
const REVIEWED_PROFILES: &[(&str, &str, &str)] =
    &[(REVIEWED_SOURCE, REVIEWED_BINARY, REVIEWED_CONSENSUS)];

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeProfileBinding {
    pub source_sha256: String,
    pub binary_sha256: String,
    pub consensus_fingerprint: String,
}
pub struct ReviewedProfile(());
/// Source identity is recovered only from a reviewed exact binary/profile
/// mapping, never from a caller's version string or an unbound RPC claim.
pub fn reviewed_binding(binary: &str, consensus: &str) -> Option<NodeProfileBinding> {
    REVIEWED_PROFILES
        .iter()
        .find(|(_, b, p)| *b == binary && *p == consensus)
        .map(|(s, b, p)| NodeProfileBinding {
            source_sha256: (*s).into(),
            binary_sha256: (*b).into(),
            consensus_fingerprint: (*p).into(),
        })
}
pub fn bind(binding: &NodeProfileBinding) -> Option<ReviewedProfile> {
    REVIEWED_PROFILES
        .iter()
        .any(|(source, binary, profile)| {
            *source == binding.source_sha256
                && *binary == binding.binary_sha256
                && *profile == binding.consensus_fingerprint
        })
        .then_some(ReviewedProfile(()))
}
pub fn definitive_rejection(
    outcome: &RpcOutcome,
    profile: Option<&ReviewedProfile>,
    raw_hex: &str,
) -> Option<&'static str> {
    profile?;
    let RpcOutcome::NodeRejected { code, message } = outcome else {
        return None;
    };
    if (*code, message.as_str()) != (-26, "version") {
        return None;
    }
    let bytes = hex::decode(raw_hex).ok()?;
    let tx: bitcoin::Transaction = bitcoin::consensus::deserialize(&bytes).ok()?;
    if bitcoin::consensus::serialize(&tx) != bytes || tx.version.0 != 3 {
        return None;
    }
    Some("rpc_tx_version_nonstandard")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_exact_review_bound_rows_are_definitive() {
        let mut tx = bitcoin::blockdata::constants::genesis_block(bitcoin::Network::Regtest)
            .txdata
            .remove(0);
        tx.version = bitcoin::transaction::Version(3);
        let raw = hex::encode(bitcoin::consensus::serialize(&tx));
        let version = RpcOutcome::NodeRejected {
            code: -26,
            message: "version".into(),
        };
        assert_eq!(definitive_rejection(&version, None, &raw), None);
        let binding = reviewed_binding(REVIEWED_BINARY, REVIEWED_CONSENSUS).unwrap();
        let fixture = bind(&binding).unwrap();
        assert_eq!(
            definitive_rejection(&version, Some(&fixture), &raw),
            Some("rpc_tx_version_nonstandard")
        );
        for message in [
            "version ",
            " version",
            "mempool not available",
            "already in block chain",
            "bad-txns-inputs-missingorspent",
        ] {
            assert_eq!(
                definitive_rejection(
                    &RpcOutcome::NodeRejected {
                        code: -26,
                        message: message.into()
                    },
                    Some(&fixture),
                    &raw
                ),
                None
            );
        }
        for code in [-8, -25, -26, -27, -1] {
            assert_eq!(
                definitive_rejection(
                    &RpcOutcome::NodeRejected {
                        code,
                        message:
                            "Fee exceeds maximum configured by user (e.g. -maxtxfee, maxfeerate)"
                                .into()
                    },
                    Some(&fixture),
                    &raw
                ),
                None
            );
        }
        tx.version = bitcoin::transaction::Version::TWO;
        assert_eq!(
            definitive_rejection(
                &version,
                Some(&fixture),
                &hex::encode(bitcoin::consensus::serialize(&tx))
            ),
            None
        );
        assert!(reviewed_binding(&"aa".repeat(32), REVIEWED_CONSENSUS).is_none());
        assert!(reviewed_binding(
            "36e2387ada164e2c89b3d20f5d515bf19c33f30fb927e8ca11621eba3658d7d9",
            REVIEWED_CONSENSUS
        )
        .is_none());
        assert!(reviewed_binding(REVIEWED_BINARY, &"aa".repeat(32)).is_none());
        let mut wrong = binding;
        wrong.source_sha256 = "aa".repeat(32);
        assert!(bind(&wrong).is_none());
        assert_eq!(
            definitive_rejection(
                &RpcOutcome::unknown(crate::control::UnknownClass::Transport),
                Some(&fixture),
                &raw
            ),
            None
        );
    }
}
