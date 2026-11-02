//! Native Stratum V1 session state machine (SPEC §3.1, §3.4; decision
//! 2026-11-02). No I/O here: the TCP listener feeds lines into this type so
//! the identity, replay and coinbase rules are testable without a socket.

use std::collections::HashSet;

use serde_json::{json, Value};
use spc_crypto::{parse_sp_address, Network};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum Sv1Error {
    #[error("request is not a JSON object")]
    NotObject,
    #[error("missing method")]
    MissingMethod,
    #[error("invalid params for {0}")]
    Params(&'static str),
    #[error("not authorized")]
    Unauthorized,
    #[error("invalid-user-identity")]
    InvalidIdentity,
    #[error("stale job")]
    StaleJob,
    #[error("duplicate share")]
    DuplicateShare,
    #[error("invalid extranonce2 length")]
    ExtranonceLength,
    #[error("no active job")]
    NoJob,
}

/// A job as the pool sends it to an SV1 miner. `coinb1` ends just after the
/// SPc `push32`, so the miner's extranonce (extranonce1 ‖ extranonce2) follows
/// it exactly as required by SPEC §2.4.1.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job {
    pub job_id: String,
    pub prev_hash: [u8; 32],
    pub height: u32,
    pub version: u32,
    pub nbits: u32,
    pub ntime: u32,
    pub coinb1: Vec<u8>,
    pub coinb2: Vec<u8>,
    pub merkle_branches: Vec<[u8; 32]>,
    pub clean_jobs: bool,
}

/// One accepted share, in the shape the slice engine consumes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Share {
    pub sp_id: [u8; 32],
    pub b_scan: [u8; 33],
    pub b_spend: [u8; 33],
    pub job_id: String,
    pub nonce: u32,
    pub ntime: u32,
    pub version: u32,
    pub extranonce: Vec<u8>,
    pub share_index: u32,
    pub job_fees: u64,
    pub coinbase: Vec<u8>,
    pub merkle_root: [u8; 32],
    pub share_hash: [u8; 32],
}

#[derive(Clone, Debug)]
struct Authorized {
    sp_id: [u8; 32],
    b_scan: [u8; 33],
    b_spend: [u8; 33],
    worker: Option<String>,
}

/// One SV1 connection.
pub struct Session {
    network: Network,
    channel_id: u32,
    extranonce1: Vec<u8>,
    extranonce2_size: usize,
    authorized: Option<Authorized>,
    job: Option<Job>,
    share_index: u32,
    seen: HashSet<[u8; 32]>,
    subscribed: bool,
    last_share: Option<Share>,
}

impl Session {
    pub fn new(
        network: Network,
        channel_id: u32,
        extranonce1: Vec<u8>,
        extranonce2_size: usize,
    ) -> Self {
        Self {
            network,
            channel_id,
            extranonce1,
            extranonce2_size,
            authorized: None,
            job: None,
            share_index: 0,
            seen: HashSet::new(),
            subscribed: false,
            last_share: None,
        }
    }

    pub fn sp_id(&self) -> Option<[u8; 32]> {
        self.authorized.as_ref().map(|a| a.sp_id)
    }

    pub fn worker(&self) -> Option<&str> {
        self.authorized.as_ref().and_then(|a| a.worker.as_deref())
    }

    pub fn set_job(&mut self, job: Job) {
        if job.clean_jobs {
            self.seen.clear();
        }
        self.job = Some(job);
    }

    fn subscription_id(&self) -> String {
        format!("spc{:08x}", self.channel_id)
    }

    /// Handle one newline-delimited JSON request. Returns the JSON response
    /// line (if any) and any notifications to send first.
    pub fn handle_line(&mut self, line: &str) -> (Vec<String>, Option<String>) {
        let value: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => return (vec![], Some(error_line(Value::Null, -32700, "parse error"))),
        };
        let (notifications, response) = self.handle_request(value);
        (notifications, response)
    }

    pub fn handle_request(&mut self, req: Value) -> (Vec<String>, Option<String>) {
        let obj = match req.as_object() {
            Some(o) => o,
            None => {
                return (
                    vec![],
                    Some(error_line(Value::Null, -32600, "invalid request")),
                )
            }
        };
        let id = obj.get("id").cloned().unwrap_or(Value::Null);
        let method = match obj.get("method").and_then(|m| m.as_str()) {
            Some(m) => m,
            None => return (vec![], Some(error_line(id, -32600, "missing method"))),
        };
        let params = obj.get("params").cloned().unwrap_or(Value::Array(vec![]));

        match method {
            "mining.subscribe" => {
                self.subscribed = true;
                let sub = self.subscription_id();
                let result = json!([
                    [["mining.set_difficulty", sub], ["mining.notify", sub]],
                    hex(&self.extranonce1),
                    self.extranonce2_size
                ]);
                (vec![], Some(ok_line(id, result)))
            }
            "mining.authorize" => match self.authorize(&params) {
                Ok(()) => (vec![], Some(ok_line(id, json!(true)))),
                Err(e) => (vec![], Some(error_line(id, 20, &e.to_string()))),
            },
            "mining.submit" => match self.submit(&params) {
                Ok(share) => {
                    let line = ok_line(id, json!(true));
                    // The caller consumes the share through `last_share`.
                    self.last_share = Some(share);
                    (vec![], Some(line))
                }
                Err(e) => (vec![], Some(error_line(id, 21, &e.to_string()))),
            },
            "mining.suggest_difficulty" | "mining.extranonce.subscribe" | "mining.configure" => {
                (vec![], Some(ok_line(id, json!(true))))
            }
            other => (
                vec![],
                Some(error_line(id, -32601, &format!("unknown method '{other}'"))),
            ),
        }
    }

    /// Set by a successful `mining.submit`; the listener takes it and forwards
    /// it to the share pipeline.
    pub fn take_share(&mut self) -> Option<Share> {
        self.last_share.take()
    }

    fn authorize(&mut self, params: &Value) -> Result<(), Sv1Error> {
        let username = params
            .get(0)
            .and_then(|v| v.as_str())
            .ok_or(Sv1Error::Params("mining.authorize"))?;
        let stripped = spc_crypto::strip_worker(username);
        let address =
            parse_sp_address(stripped, self.network).map_err(|_| Sv1Error::InvalidIdentity)?;
        let sp_id = spc_crypto::sp_id(&address.b_scan, &address.b_spend);
        let worker = if stripped.len() == username.len() {
            None
        } else {
            Some(username[stripped.len() + 1..].to_string())
        };

        // One connection is one identity. A second authorize with a different
        // address is rejected (SPEC §3.1).
        if let Some(existing) = &self.authorized {
            if existing.sp_id != sp_id {
                return Err(Sv1Error::InvalidIdentity);
            }
        } else {
            self.authorized = Some(Authorized {
                sp_id,
                b_scan: address.b_scan.serialize(),
                b_spend: address.b_spend.serialize(),
                worker,
            });
        }
        Ok(())
    }

    fn submit(&mut self, params: &Value) -> Result<Share, Sv1Error> {
        let auth = self.authorized.as_ref().ok_or(Sv1Error::Unauthorized)?;
        let job = self.job.as_ref().ok_or(Sv1Error::NoJob)?;

        let job_id = params
            .get(1)
            .and_then(|v| v.as_str())
            .ok_or(Sv1Error::Params("mining.submit"))?;
        if job_id != job.job_id {
            return Err(Sv1Error::StaleJob);
        }
        let extranonce2 = params
            .get(2)
            .and_then(|v| v.as_str())
            .ok_or(Sv1Error::Params("mining.submit"))?;
        let extranonce2 = decode_hex(extranonce2).ok_or(Sv1Error::Params("mining.submit"))?;
        if extranonce2.len() != self.extranonce2_size {
            return Err(Sv1Error::ExtranonceLength);
        }
        let ntime = params
            .get(3)
            .and_then(|v| v.as_str())
            .and_then(|s| u32::from_str_radix(s, 16).ok())
            .ok_or(Sv1Error::Params("mining.submit"))?;
        let nonce = params
            .get(4)
            .and_then(|v| v.as_str())
            .and_then(|s| u32::from_str_radix(s, 16).ok())
            .ok_or(Sv1Error::Params("mining.submit"))?;

        let mut extranonce = self.extranonce1.clone();
        extranonce.extend_from_slice(&extranonce2);

        let nullifier = nullifier(
            self.channel_id,
            job_id,
            nonce,
            ntime,
            job.version,
            &extranonce,
        );
        if !self.seen.insert(nullifier) {
            return Err(Sv1Error::DuplicateShare);
        }

        let mut coinbase = job.coinb1.clone();
        coinbase.extend_from_slice(&extranonce);
        coinbase.extend_from_slice(&job.coinb2);

        let mut merkle_root = double_sha256(&coinbase);
        for branch in job.merkle_branches.iter().rev() {
            let mut buf = Vec::with_capacity(64);
            buf.extend_from_slice(&merkle_root);
            buf.extend_from_slice(branch);
            merkle_root = double_sha256(&buf);
        }
        let mut header = Vec::with_capacity(80);
        header.extend_from_slice(&job.version.to_le_bytes());
        header.extend_from_slice(&job.prev_hash);
        header.extend_from_slice(&merkle_root);
        header.extend_from_slice(&ntime.to_le_bytes());
        header.extend_from_slice(&job.nbits.to_le_bytes());
        header.extend_from_slice(&nonce.to_le_bytes());
        let share_hash = double_sha256(&header);

        let share_index = self.share_index;
        self.share_index = self.share_index.wrapping_add(1);

        Ok(Share {
            sp_id: auth.sp_id,
            b_scan: auth.b_scan,
            b_spend: auth.b_spend,
            job_id: job_id.to_string(),
            nonce,
            ntime,
            version: job.version,
            extranonce,
            share_index,
            job_fees: 0,
            coinbase,
            merkle_root,
            share_hash,
        })
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn decode_hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

fn double_sha256(data: &[u8]) -> [u8; 32] {
    use bitcoin::hashes::{sha256d, Hash};
    sha256d::Hash::hash(data).to_byte_array()
}

/// SPEC §3.4 nullifier, widened with the connection's channel id.
pub fn nullifier(
    channel_id: u32,
    job_id: &str,
    nonce: u32,
    ntime: u32,
    version: u32,
    extranonce: &[u8],
) -> [u8; 32] {
    use bitcoin::hashes::{sha256, Hash, HashEngine};
    let mut eng = sha256::Hash::engine();
    eng.input(&channel_id.to_le_bytes());
    eng.input(job_id.as_bytes());
    eng.input(&nonce.to_le_bytes());
    eng.input(&ntime.to_le_bytes());
    eng.input(&version.to_le_bytes());
    eng.input(extranonce);
    sha256::Hash::from_engine(eng).to_byte_array()
}

/// `mining.notify` params. SV1 sends the previous hash in display order.
pub fn notify(job: &Job) -> Value {
    json!([
        job.job_id,
        hex(&job.prev_hash.iter().rev().copied().collect::<Vec<_>>()),
        hex(&job.coinb1),
        hex(&job.coinb2),
        job.merkle_branches
            .iter()
            .map(|b| hex(b))
            .collect::<Vec<_>>(),
        format!("{:08x}", job.version),
        format!("{:08x}", job.nbits),
        format!("{:08x}", job.ntime),
        job.clean_jobs
    ])
}

fn ok_line(id: Value, result: Value) -> String {
    json!({ "id": id, "result": result, "error": null }).to_string()
}

fn error_line(id: Value, code: i64, message: &str) -> String {
    json!({ "id": id, "result": null, "error": [code, message, null] }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};

    fn address(network: Network, seed: u8) -> String {
        let secp = Secp256k1::new();
        let scan = SecretKey::from_slice(&[seed; 32]).unwrap();
        let spend = SecretKey::from_slice(&[seed.wrapping_add(1); 32]).unwrap();
        spc_crypto::address::encode_sp_address(
            network,
            &PublicKey::from_secret_key(&secp, &scan),
            &PublicKey::from_secret_key(&secp, &spend),
        )
        .unwrap()
    }

    fn job() -> Job {
        Job {
            job_id: "00000001".into(),
            prev_hash: [0x11; 32],
            height: 800_000,
            version: 0x2000_0000,
            nbits: 0x1d00_ffff,
            ntime: 1_700_000_000,
            coinb1: vec![0x01, 0x00, 0x00, 0x00],
            coinb2: vec![0xff; 8],
            merkle_branches: vec![],
            clean_jobs: true,
        }
    }

    fn session() -> Session {
        Session::new(Network::Testnet, 7, vec![0xaa; 4], 4)
    }

    #[test]
    fn subscribe_returns_extranonce() {
        let mut s = session();
        let (_, line) = s.handle_request(json!({
            "id": 1, "method": "mining.subscribe", "params": []
        }));
        let v: Value = serde_json::from_str(&line.unwrap()).unwrap();
        assert_eq!(v["result"][1], "aaaaaaaa");
        assert_eq!(v["result"][2], 4);
    }

    #[test]
    fn authorize_accepts_valid_identity_and_rejects_bare_address() {
        let mut s = session();
        let good = address(Network::Testnet, 3);
        let (_, line) = s.handle_request(json!({
            "id": 1, "method": "mining.authorize", "params": [format!("{good}.rig1"), "x"]
        }));
        let v: Value = serde_json::from_str(&line.unwrap()).unwrap();
        assert_eq!(v["result"], true);
        assert!(s.sp_id().is_some());
        assert_eq!(s.worker(), Some("rig1"));

        let mut s2 = session();
        let (_, line) = s2.handle_request(json!({
            "id": 2, "method": "mining.authorize",
            "params": ["bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq", "x"]
        }));
        let v: Value = serde_json::from_str(&line.unwrap()).unwrap();
        assert_eq!(v["error"][1], "invalid-user-identity");
        assert!(s2.sp_id().is_none());
    }

    #[test]
    fn one_connection_is_one_identity() {
        let mut s = session();
        let a = address(Network::Testnet, 3);
        let b = address(Network::Testnet, 9);
        s.handle_request(json!({"id":1,"method":"mining.authorize","params":[a,"x"]}));
        let (_, line) =
            s.handle_request(json!({"id":2,"method":"mining.authorize","params":[b,"x"]}));
        let v: Value = serde_json::from_str(&line.unwrap()).unwrap();
        assert_eq!(v["error"][1], "invalid-user-identity");
    }

    #[test]
    fn submit_requires_authorization_and_rejects_replay() {
        let mut s = session();
        s.set_job(job());
        let (_, line) = s.handle_request(json!({
            "id": 1, "method": "mining.submit",
            "params": ["w", "00000001", "00000000", "6553f100", "00000000"]
        }));
        let v: Value = serde_json::from_str(&line.unwrap()).unwrap();
        assert_eq!(v["error"][1], "not authorized");

        let good = address(Network::Testnet, 3);
        s.handle_request(json!({"id":2,"method":"mining.authorize","params":[good,"x"]}));
        let (_, line) = s.handle_request(json!({
            "id": 3, "method": "mining.submit",
            "params": ["w", "00000001", "00000000", "6553f100", "00000000"]
        }));
        let v: Value = serde_json::from_str(&line.unwrap()).unwrap();
        assert_eq!(v["result"], true);
        let share = s.take_share().expect("share");
        assert_eq!(share.extranonce.len(), 8);
        assert_eq!(share.share_index, 0);

        let (_, line) = s.handle_request(json!({
            "id": 4, "method": "mining.submit",
            "params": ["w", "00000001", "00000000", "6553f100", "00000000"]
        }));
        let v: Value = serde_json::from_str(&line.unwrap()).unwrap();
        assert_eq!(v["error"][1], "duplicate share");
    }

    #[test]
    fn stale_job_rejected() {
        let mut s = session();
        let good = address(Network::Testnet, 3);
        s.handle_request(json!({"id":1,"method":"mining.authorize","params":[good,"x"]}));
        s.set_job(job());
        let (_, line) = s.handle_request(json!({
            "id": 2, "method": "mining.submit",
            "params": ["w", "00000002", "00000000", "6553f100", "00000000"]
        }));
        let v: Value = serde_json::from_str(&line.unwrap()).unwrap();
        assert_eq!(v["error"][1], "stale job");
    }

    #[test]
    fn notify_reverses_prev_hash() {
        let j = job();
        let v = notify(&j);
        assert_eq!(v[1].as_str().unwrap(), "11".repeat(32));
        assert_eq!(v[8], true);
    }
}
