//! Rust consumer of the canonical probe's frozen launch-profile/1 grammar.
//! r4 policy selectors remain ordered literals, never transport substitutions.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub const CONTRACT_CANDIDATE: &str =
    "754e1e6131955f9e5bbce7ef81b6f9aabaa2f1379ec6409f1c3a7a9e88be91f9";
const UNSUPPORTED: &str = "traffic_launch_profile_unsupported";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchProfile {
    pub profile_version: u32,
    pub binary_sha256: String,
    pub argv: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub config_files: Vec<ConfigFile>,
    pub substitutions: Vec<Substitution>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigFile {
    pub argv_index: u32,
    pub sha256: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Substitution {
    pub class: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub argv_index: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<[u32; 2]>,
}

fn hash(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn path(s: &str) -> bool {
    s.starts_with('/')
        && s.split('/')
            .skip(1)
            .all(|p| !p.is_empty() && p != "." && p != "..")
}
fn endpoint(s: &str) -> Option<std::net::SocketAddrV4> {
    s.parse().ok()
}
fn width(flag: &str) -> Option<usize> {
    match flag {
        "--rpc"
        | "--no-listen"
        | "--no-seeds"
        | "--no-discover"
        | "--rpc-compat-profile=bark-v31" => Some(1),
        "--network"
        | "--datadir"
        | "--rpc-listen"
        | "--rpc-cookie-file"
        | "--regtest-pow-bits"
        | "--api-log"
        | "--log-level"
        | "--max-run-secs"
        | "--config"
        | "--max-outbound"
        | "--listen"
        | "--connect"
        | "--rpc-compat-profile" => Some(2),
        _ => None,
    }
}
const P2P: &[&str] = &[
    "--no-listen",
    "--no-seeds",
    "--no-discover",
    "--max-outbound",
    "--listen",
    "--connect",
];
const VALUES: &[(&str, &str)] = &[
    ("datadir", "--datadir"),
    ("rpc-listen", "--rpc-listen"),
    ("cookie-file", "--rpc-cookie-file"),
    ("max-run-secs", "--max-run-secs"),
];

impl LaunchProfile {
    /// Render substitutions from the ACTUAL finalized argv, preserving every
    /// policy token. Validation below refuses unknown/duplicate flags.
    pub fn capture(
        binary_sha256: String,
        argv: Vec<String>,
        env: BTreeMap<String, String>,
    ) -> Result<Self, &'static str> {
        let mut substitutions = Vec::new();
        let mut config_files = Vec::new();
        let mut i = 0;
        while i < argv.len() {
            let w = width(&argv[i]).ok_or(UNSUPPORTED)?;
            if i + w > argv.len() {
                return Err(UNSUPPORTED);
            }
            if P2P.contains(&argv[i].as_str()) {
                substitutions.push(Substitution {
                    class: "p2p-isolation".into(),
                    argv_index: None,
                    span: Some([i as u32, (i + w) as u32]),
                });
            } else if let Some((class, _)) = VALUES.iter().find(|(_, flag)| *flag == argv[i]) {
                substitutions.push(Substitution {
                    class: (*class).into(),
                    argv_index: Some((i + 1) as u32),
                    span: None,
                });
            } else if argv[i] == "--api-log" {
                if argv.get(i + 2).map(String::as_str) != Some("--log-level") || i + 4 > argv.len()
                {
                    return Err(UNSUPPORTED);
                }
                substitutions.push(Substitution {
                    class: "api-log".into(),
                    argv_index: None,
                    span: Some([i as u32, (i + 4) as u32]),
                });
            } else if argv[i] == "--config" {
                let bytes = std::fs::read(&argv[i + 1]).map_err(|_| UNSUPPORTED)?;
                config_files.push(ConfigFile {
                    argv_index: (i + 1) as u32,
                    sha256: hex::encode(Sha256::digest(bytes)),
                });
            }
            i += w;
        }
        Ok(Self {
            profile_version: 1,
            binary_sha256,
            argv,
            env,
            config_files,
            substitutions,
        })
    }

    pub fn normalize(
        &self,
        owned: &BTreeSet<String>,
        read: impl Fn(&Path) -> Result<Vec<u8>, String>,
    ) -> Result<Value, &'static str> {
        canonical_bytes(&serde_json::to_value(self).map_err(|_| UNSUPPORTED)?)?;
        if self.profile_version != 1
            || !hash(&self.binary_sha256)
            || self.argv.is_empty()
            || self.argv.len() > u32::MAX as usize
        {
            return Err(UNSUPPORTED);
        }
        let token = self.env.get("SPC_LAB_RUN_TOKEN").ok_or(UNSUPPORTED)?;
        if !(16..=128).contains(&token.len())
            || !token
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err(UNSUPPORTED);
        }
        for key in self.env.keys() {
            if key.is_empty()
                || !key.bytes().enumerate().all(|(i, b)| {
                    b.is_ascii_alphabetic() || b == b'_' || (i > 0 && b.is_ascii_digit())
                })
            {
                return Err(UNSUPPORTED);
            }
        }
        let mut flags = BTreeMap::new();
        let mut i = 0;
        while i < self.argv.len() {
            let flag = self.argv[i].as_str();
            let w = width(flag).ok_or(UNSUPPORTED)?;
            let name = if flag == "--rpc-compat-profile=bark-v31" {
                "--rpc-compat-profile"
            } else {
                flag
            };
            if i + w > self.argv.len() || flags.insert(name, (i, w)).is_some() {
                return Err(UNSUPPORTED);
            }
            i += w;
        }
        let value = |flag: &str| -> Option<&str> {
            let (i, w) = *flags.get(flag)?;
            if w == 2 {
                Some(&self.argv[i + 1])
            } else if flag == "--rpc-compat-profile" {
                Some("bark-v31")
            } else {
                None
            }
        };
        if value("--network") != Some("regtest")
            || !["--rpc", "--datadir", "--rpc-listen", "--rpc-cookie-file"]
                .iter()
                .all(|f| flags.contains_key(f))
        {
            return Err(UNSUPPORTED);
        }
        if flags.contains_key("--rpc-compat-profile") {
            let dialect = self
                .env
                .get("RBITCOIN_RPC_PACKAGE_DIALECT")
                .map(String::as_str)
                .unwrap_or("");
            if value("--rpc-compat-profile") != Some("bark-v31")
                || !(dialect.is_empty() || dialect == "1" || dialect.eq_ignore_ascii_case("true"))
            {
                return Err(UNSUPPORTED);
            }
        }
        for f in ["--datadir", "--rpc-cookie-file", "--api-log", "--config"] {
            if value(f).is_some_and(|v| !path(v)) {
                return Err(UNSUPPORTED);
            }
        }
        let rpc = value("--rpc-listen").ok_or(UNSUPPORTED)?;
        if !endpoint(rpc)
            .is_some_and(|a| a.port() > 0 && (a.ip().is_loopback() || owned.contains(rpc)))
        {
            return Err(UNSUPPORTED);
        }
        if let Some(bits) = value("--regtest-pow-bits") {
            let bits = bits.strip_prefix("0x").unwrap_or(bits);
            if bits.len() != 8
                || !bits
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return Err(UNSUPPORTED);
            }
        }
        if let Some(seconds) = value("--max-run-secs") {
            if seconds.starts_with('0')
                || !seconds.bytes().all(|b| b.is_ascii_digit())
                || !seconds
                    .parse::<u64>()
                    .is_ok_and(|n| (1..=86400).contains(&n))
            {
                return Err(UNSUPPORTED);
            }
        }
        if value("--log-level")
            .is_some_and(|s| !["error", "warn", "info", "debug", "trace"].contains(&s))
        {
            return Err(UNSUPPORTED);
        }
        let present: BTreeSet<&str> = flags.keys().copied().filter(|f| P2P.contains(f)).collect();
        let isolated: BTreeSet<&str> = [
            "--no-seeds",
            "--no-discover",
            "--max-outbound",
            "--no-listen",
        ]
        .into_iter()
        .collect();
        if present == isolated {
            if !matches!(value("--max-outbound"), Some("0" | "1")) {
                return Err(UNSUPPORTED);
            }
        } else {
            let namespace: BTreeSet<&str> = [
                "--no-seeds",
                "--no-discover",
                "--max-outbound",
                "--listen",
                "--connect",
            ]
            .into_iter()
            .collect();
            if present != namespace
                || value("--max-outbound") != Some("1")
                || value("--listen") == value("--connect")
            {
                return Err(UNSUPPORTED);
            }
            for f in ["--listen", "--connect"] {
                let e = value(f).ok_or(UNSUPPORTED)?;
                if !owned.contains(e) || endpoint(e).is_none_or(|e| e.port() == 0) {
                    return Err(UNSUPPORTED);
                }
            }
        }
        let p2p: BTreeSet<usize> = present
            .iter()
            .flat_map(|f| {
                let (i, w) = flags[f];
                i..i + w
            })
            .collect();
        let (mut claimed, mut p2p_claimed, mut classes) =
            (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
        let mut subs = BTreeMap::new();
        for sub in &self.substitutions {
            let (start, end) = match (sub.argv_index, sub.span) {
                (Some(i), None) => (i as usize, i as usize + 1),
                (None, Some([s, e])) => (s as usize, e as usize),
                _ => return Err(UNSUPPORTED),
            };
            if start >= end || end > self.argv.len() || (start..end).any(|i| !claimed.insert(i)) {
                return Err(UNSUPPORTED);
            }
            if sub.class == "p2p-isolation" {
                if sub.span.is_none() || !(start..end).all(|i| p2p.contains(&i)) {
                    return Err(UNSUPPORTED);
                }
                p2p_claimed.extend(start..end);
                continue;
            }
            if !classes.insert(sub.class.as_str()) {
                return Err(UNSUPPORTED);
            }
            if let Some((_, flag)) = VALUES.iter().find(|(c, _)| *c == sub.class) {
                if sub.argv_index.is_none() || flags.get(flag).map(|(i, _)| i + 1) != Some(start) {
                    return Err(UNSUPPORTED);
                }
            } else if sub.class == "api-log" {
                if sub.span.is_none()
                    || end - start != 4
                    || self.argv[start] != "--api-log"
                    || self.argv[start + 2] != "--log-level"
                {
                    return Err(UNSUPPORTED);
                }
            } else {
                return Err(UNSUPPORTED);
            }
            subs.insert(start, (end, sub.class.as_str()));
        }
        let mut required: BTreeSet<&str> = VALUES
            .iter()
            .filter(|(_, f)| flags.contains_key(f))
            .map(|(c, _)| *c)
            .collect();
        if flags.contains_key("--api-log") || flags.contains_key("--log-level") {
            required.insert("api-log");
        }
        if classes != required || p2p_claimed != p2p {
            return Err(UNSUPPORTED);
        }
        let (mut argv, mut mapping, mut i) = (Vec::new(), BTreeMap::new(), 0);
        while i < self.argv.len() {
            if p2p.contains(&i) {
                i += 1;
                continue;
            }
            mapping.insert(i, argv.len());
            if let Some((end, class)) = subs.get(&i) {
                argv.push(json!({"class":class}));
                i = *end;
            } else {
                argv.push(json!({"literal":self.argv[i]}));
                i += 1;
            }
        }
        let mut configs = Vec::new();
        for config in &self.config_files {
            let index = config.argv_index as usize;
            if !hash(&config.sha256)
                || flags.get("--config").map(|(i, _)| i + 1) != Some(index)
                || !configs.is_empty()
            {
                return Err(UNSUPPORTED);
            }
            let bytes = read(Path::new(self.argv.get(index).ok_or(UNSUPPORTED)?))
                .map_err(|_| UNSUPPORTED)?;
            if hex::encode(Sha256::digest(bytes)) != config.sha256 {
                return Err(UNSUPPORTED);
            }
            configs.push(json!({"argv_index":mapping.get(&index).ok_or(UNSUPPORTED)?,"sha256":config.sha256}));
        }
        if configs.is_empty() == flags.contains_key("--config") {
            return Err(UNSUPPORTED);
        }
        Ok(
            json!({"profile_version":1,"binary_sha256":self.binary_sha256,"argv":argv,"env":self.env,
            "config_files":configs,"p2p_isolation":"validated_lab_transport"}),
        )
    }
}

/// Restricted RFC 8785 encoder: safe integers, strings, arrays and objects,
/// with UTF-16 key ordering. No floats, controls or token normalization.
pub fn canonical_bytes(value: &Value) -> Result<Vec<u8>, &'static str> {
    fn encode(v: &Value, out: &mut Vec<u8>) -> Result<(), &'static str> {
        match v {
            Value::String(s) => {
                if s.chars().any(|c| c.is_control()) {
                    return Err(UNSUPPORTED);
                }
                out.extend(serde_json::to_vec(s).map_err(|_| UNSUPPORTED)?);
            }
            Value::Number(n) => {
                if n.as_i64()
                    .is_none_or(|n| n.unsigned_abs() > 9_007_199_254_740_991)
                {
                    return Err(UNSUPPORTED);
                }
                out.extend(n.to_string().bytes());
            }
            Value::Array(a) => {
                out.push(b'[');
                for (i, v) in a.iter().enumerate() {
                    if i > 0 {
                        out.push(b',');
                    }
                    encode(v, out)?;
                }
                out.push(b']');
            }
            Value::Object(m) => {
                let mut entries: Vec<_> = m.iter().collect();
                entries.sort_by_cached_key(|(k, _)| k.encode_utf16().collect::<Vec<_>>());
                out.push(b'{');
                for (i, (k, v)) in entries.into_iter().enumerate() {
                    if i > 0 {
                        out.push(b',');
                    }
                    encode(&Value::String(k.clone()), out)?;
                    out.push(b':');
                    encode(v, out)?;
                }
                out.push(b'}');
            }
            _ => return Err(UNSUPPORTED),
        }
        Ok(())
    }
    let mut out = Vec::new();
    encode(value, &mut out)?;
    Ok(out)
}
pub fn digest(value: &Value) -> Result<String, &'static str> {
    Ok(hex::encode(Sha256::digest(canonical_bytes(value)?)))
}
