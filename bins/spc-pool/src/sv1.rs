//! Native Stratum V1 session state machine (SPEC §3.1, §3.4; decision
//! 2026-11-02-share-target-policy). No I/O here: the TCP listener feeds lines
//! into this type so the identity, replay, target and coinbase rules are
//! testable without a socket.
//!
//! ## Share target policy
//!
//! The share-difficulty policy is shared by both ingresses (`sv1_server`,
//! `sv2_server`) and by the front's accounting, so it lives here as four pure
//! functions (decision 2026-11-02-share-target-policy):
//!
//! - [`target_for_difficulty`] maps a pool share difficulty to a target using
//!   the Bitcoin difficulty-1 target (`0x00000000ffff0000...`) as the base.
//! - [`effective_share_target`] is the target a share is validated against:
//!   `max(target_for_difficulty(d), network_target)`, i.e. *never harder than
//!   the network target*.
//! - [`credited_difficulty`] is the `d` booked in the `ShareRec`:
//!   `min(d, max(1, round(network_difficulty)))`, so it is never zero and a
//!   share never counts for more than the network difficulty it could meet.
//! - [`network_target`] is the consensus target for the template's `nbits`;
//!   block detection and `SubmitShares` still key off it.

use std::collections::HashSet;

use bitcoin::hashes::Hash;
use bitcoin::pow::{CompactTarget, Target};
use bitcoin::BlockHash;
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
    #[error("ntime out of range")]
    StaleTime,
    #[error("duplicate share")]
    DuplicateShare,
    #[error("low difficulty share")]
    LowDifficulty,
    #[error("invalid extranonce2 length")]
    ExtranonceLength,
    #[error("no active job")]
    NoJob,
}

/// The Bitcoin difficulty-1 target (`0x00000000ffff0000...`), the base every
/// pool share difficulty is measured against.
const DIFFICULTY_1_TARGET: Target = Target::MAX;

/// Consensus target for a template's `nbits`.
pub fn network_target(bits: u32) -> Target {
    Target::from_compact(CompactTarget::from_consensus(bits))
}

/// Target for pool share difficulty `d` (clamped to at least 1):
/// `difficulty_1_target / d`.
pub fn target_for_difficulty(share_difficulty: u64) -> Target {
    let d = share_difficulty.max(1);
    Target::from_be_bytes(div_be_u256_by_u64(DIFFICULTY_1_TARGET.to_be_bytes(), d))
}

/// The target every submitted share must meet: the easier of the pool share
/// target and the network target. A share can never be required to be harder
/// than the block it might find.
pub fn effective_share_target(share_difficulty: u64, bits: u32) -> Target {
    let share = target_for_difficulty(share_difficulty);
    let network = network_target(bits);
    // Big-endian arrays compare in numeric order; a larger target is easier.
    if share.to_be_bytes() >= network.to_be_bytes() {
        share
    } else {
        network
    }
}

/// Network difficulty for `bits` as a rounded, never-zero `u64`.
///
/// Difficulty is measured against the difficulty-1 target, which is the
/// Bitcoin convention (and what `bitcoind` reports on mainnet). Regtest-style
/// bits are far easier than difficulty 1, so they round up to 1 rather than to
/// zero; a zero target saturates at `u64::MAX` and is never creditable.
pub fn network_difficulty(bits: u32) -> u64 {
    let d = network_target(bits).difficulty_float();
    if !d.is_finite() {
        u64::MAX
    } else if d < 1.0 {
        1
    } else if d >= u64::MAX as f64 {
        u64::MAX
    } else {
        d.round() as u64
    }
}

/// The `d` booked for one accepted share: `min(d, max(1, round(D_net)))`.
pub fn credited_difficulty(share_difficulty: u64, bits: u32) -> u64 {
    share_difficulty.max(1).min(network_difficulty(bits))
}

/// Does `hash` meet `target`? Never panics and never accepts a zero target.
pub fn meets_target(hash: [u8; 32], target: Target) -> bool {
    target.is_met_by(BlockHash::from_byte_array(hash))
}

/// Interpret the SV1 `mining.set_difficulty` value as a share difficulty.
/// A non-finite or sub-1 value clamps to 1 (fail closed: a pool never asks for
/// less than difficulty 1).
pub fn share_difficulty_from_float(difficulty: f64) -> u64 {
    if !difficulty.is_finite() || difficulty < 1.0 {
        return 1;
    }
    if difficulty >= u64::MAX as f64 {
        return u64::MAX;
    }
    difficulty.round() as u64
}

/// BIP 320 version-rolling mask: the header bits a miner may change without
/// changing the job's semantic version.
pub const VERSION_ROLLING_MASK: u32 = 0x1fff_e000;

/// How far `ntime` may differ from the job's base time (2 hours). Rolling
/// forward is normal; the lower tolerance keeps an honest miner whose job
/// predates by a few seconds from being rejected, while a header far outside
/// the window is not the template the pool can submit.
pub const MAX_NTIME_ROLL_SECS: u32 = 7_200;

/// Is `submitted` the job's version with only BIP 320 rollable bits changed?
pub fn version_allowed(job_version: u32, submitted: u32) -> bool {
    submitted & !VERSION_ROLLING_MASK == job_version & !VERSION_ROLLING_MASK
}

/// Is `submitted` a valid `ntime` for a job published at `job_ntime`
/// (within +/- `MAX_NTIME_ROLL_SECS`)?
pub fn ntime_allowed(job_ntime: u32, submitted: u32) -> bool {
    let low = job_ntime.saturating_sub(MAX_NTIME_ROLL_SECS);
    let high = job_ntime.saturating_add(MAX_NTIME_ROLL_SECS);
    submitted >= low && submitted <= high
}

/// Big-endian 256-bit division by a small integer, exact floors.
fn div_be_u256_by_u64(bytes: [u8; 32], divisor: u64) -> [u8; 32] {
    let divisor = u128::from(divisor.max(1));
    let mut out = [0u8; 32];
    let mut remainder: u128 = 0;
    for (i, byte) in bytes.iter().enumerate() {
        let cur = (remainder << 8) | u128::from(*byte);
        out[i] = (cur / divisor) as u8;
        remainder = cur % divisor;
    }
    out
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
    /// Ingress channel/connection id: part of the SPEC §3.4 replay key and
    /// never leaves the pool.
    pub channel_id: u32,
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
    /// Pool share difficulty announced with `mining.set_difficulty`; every
    /// `mining.submit` is validated against [`effective_share_target`] for it.
    share_difficulty: u64,
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
        share_difficulty: u64,
    ) -> Self {
        Self {
            network,
            channel_id,
            extranonce1,
            extranonce2_size,
            share_difficulty: share_difficulty.max(1),
            authorized: None,
            job: None,
            share_index: 0,
            seen: HashSet::new(),
            subscribed: false,
            last_share: None,
        }
    }

    /// The pool share difficulty this session announces and enforces.
    pub fn share_difficulty(&self) -> u64 {
        self.share_difficulty
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
        if !ntime_allowed(job.ntime, ntime) {
            return Err(Sv1Error::StaleTime);
        }

        let mut extranonce = self.extranonce1.clone();
        extranonce.extend_from_slice(&extranonce2);

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

        // SPEC §3.4 as amended by decision 2026-11-02-share-target-policy: a
        // share is accepted only if it meets the pool share target derived
        // from the session's difficulty (never harder than the network
        // target). Validation precedes replay marking so a rejected share does
        // not burn its nullifier.
        if !meets_target(
            share_hash,
            effective_share_target(self.share_difficulty, job.nbits),
        ) {
            return Err(Sv1Error::LowDifficulty);
        }

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

        let share_index = self.share_index;
        self.share_index = self.share_index.wrapping_add(1);

        Ok(Share {
            sp_id: auth.sp_id,
            b_scan: auth.b_scan,
            b_spend: auth.b_spend,
            channel_id: self.channel_id,
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

    /// A job whose network target is the regtest maximum, so an honest miner
    /// finds a share within a few nonces (used by the accept-path tests).
    fn job() -> Job {
        Job {
            job_id: "00000001".into(),
            prev_hash: [0x11; 32],
            height: 800_000,
            version: 0x2000_0000,
            nbits: 0x207f_ffff,
            ntime: 1_700_000_000,
            coinb1: vec![0x01, 0x00, 0x00, 0x00],
            coinb2: vec![0xff; 8],
            merkle_branches: vec![],
            clean_jobs: true,
        }
    }

    fn session() -> Session {
        Session::new(Network::Testnet, 7, vec![0xaa; 4], 4, 1)
    }

    /// Submit `mining.submit` lines with increasing nonces until one is
    /// accepted. Returns the reply value of the accepted submission.
    fn submit_until_accepted(s: &mut Session, start: u32) -> (Value, Share) {
        for nonce in start..start + 64 {
            let (_, line) = s.handle_request(json!({
                "id": 9, "method": "mining.submit",
                "params": ["w", "00000001", "00000000", "6553f100", format!("{nonce:08x}")]
            }));
            let v: Value = serde_json::from_str(&line.unwrap()).unwrap();
            if v["result"] == true {
                let share = s.take_share().expect("share");
                return (v, share);
            }
        }
        panic!("no share met the regtest target in 64 nonces");
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

        let (v, share) = submit_until_accepted(&mut s, 0);
        assert_eq!(v["result"], true);
        assert_eq!(share.extranonce.len(), 8);
        assert_eq!(share.share_index, 0);
        assert!(meets_target(
            share.share_hash,
            effective_share_target(1, job().nbits)
        ));

        // Replaying the exact same accepted submission is a duplicate: the
        // nullifier set is keyed on the share, not on the nonce counter.
        let replay_nonce = share.nonce;
        let (_, line) = s.handle_request(json!({
            "id": 4, "method": "mining.submit",
            "params": ["w", "00000001", "00000000", "6553f100", format!("{replay_nonce:08x}")]
        }));
        let v: Value = serde_json::from_str(&line.unwrap()).unwrap();
        assert_eq!(v["error"][1], "duplicate share");
    }

    #[test]
    fn submit_below_the_share_target_is_rejected() {
        // Diff-1 network target: a share at pool difficulty 1 must meet
        // 0x00000000ffff0000..., which a random header hash does not.
        let mut job = job();
        job.nbits = 0x1d00_ffff;
        let mut s = session();
        s.set_job(job);
        let good = address(Network::Testnet, 3);
        s.handle_request(json!({"id":1,"method":"mining.authorize","params":[good,"x"]}));
        let (_, line) = s.handle_request(json!({
            "id": 2, "method": "mining.submit",
            "params": ["w", "00000001", "00000000", "6553f100", "00000000"]
        }));
        let v: Value = serde_json::from_str(&line.unwrap()).unwrap();
        assert_eq!(v["error"][1], "low difficulty share");
        assert!(s.take_share().is_none());
    }

    #[test]
    fn target_for_difficulty_halves_the_target_each_step() {
        let one = target_for_difficulty(1).to_be_bytes();
        assert_eq!(one, DIFFICULTY_1_TARGET.to_be_bytes());
        assert_eq!(target_for_difficulty(0).to_be_bytes(), one, "0 clamps to 1");
        let two = target_for_difficulty(2).to_be_bytes();
        assert!(two < one && two > [0u8; 32]);
        assert!(target_for_difficulty(u64::MAX).to_be_bytes() < two);
    }

    #[test]
    fn effective_target_is_never_harder_than_the_network_target() {
        // Regtest bits: the network target is far easier than difficulty 1, so
        // it wins at every share difficulty.
        let regtest = network_target(0x207f_ffff);
        for d in [1u64, 2, 1_024, u64::MAX] {
            assert_eq!(
                effective_share_target(d, 0x207f_ffff).to_be_bytes(),
                regtest.to_be_bytes()
            );
        }
        // Network difficulty 256 (bits 0x1c00ffff): the share target wins for
        // d = 1 and d = 4, and is capped by the network target once the share
        // difficulty exceeds the network difficulty.
        let hard = 0x1c00_ffff;
        assert_eq!(
            effective_share_target(1, hard).to_be_bytes(),
            target_for_difficulty(1).to_be_bytes()
        );
        assert_eq!(
            effective_share_target(4, hard).to_be_bytes(),
            target_for_difficulty(4).to_be_bytes()
        );
        assert_eq!(
            effective_share_target(1_000, hard).to_be_bytes(),
            network_target(hard).to_be_bytes()
        );
    }

    #[test]
    fn accept_reject_boundary_is_the_share_target() {
        // Network difficulty 256: the pool share target for d = 4 is the easier
        // one, so it is exactly the accept/reject boundary.
        let bits = 0x1c00_ffff;
        let target = effective_share_target(4, bits);
        assert_eq!(target.to_be_bytes(), target_for_difficulty(4).to_be_bytes());
        let mut at = target.to_be_bytes();
        // `at == target` meets the target; `at + 1` does not.
        assert!(meets_target(at, target));
        increment_be(&mut at);
        assert!(!meets_target(at, target));
        assert_eq!(network_difficulty(bits), 256);
        assert_eq!(credited_difficulty(4, bits), 4);
        assert_eq!(credited_difficulty(u64::MAX, bits), 256);
        // Easier network bits (difficulty below 1 here) are capped at 1: never
        // zero, never more than the network difficulty.
        assert_eq!(network_difficulty(0x1e00_ffff), 1);
        assert_eq!(credited_difficulty(4, 0x1e00_ffff), 1);
        // On those bits the network target is the easier one, so it is the
        // boundary.
        assert_eq!(
            effective_share_target(4, 0x1e00_ffff).to_be_bytes(),
            network_target(0x1e00_ffff).to_be_bytes()
        );
    }

    #[test]
    fn regtest_bits_credit_exactly_one_never_zero() {
        // Regtest-style bits are easier than difficulty 1 (0x1d00ffff), so the
        // credit must be 1 and the window is never empty.
        for bits in [0x207f_ffff, 0x1d00_ffff, 0x1e00_ffff] {
            assert_eq!(network_difficulty(bits), 1, "{bits:#010x}");
            assert_eq!(credited_difficulty(1, bits), 1);
            assert_eq!(credited_difficulty(65_536, bits), 1);
            assert_ne!(credited_difficulty(0, bits), 0, "never zero");
        }
    }

    #[test]
    fn set_difficulty_float_clamps_to_at_least_one() {
        assert_eq!(share_difficulty_from_float(1.0), 1);
        assert_eq!(share_difficulty_from_float(7.4), 7);
        assert_eq!(share_difficulty_from_float(0.0), 1);
        assert_eq!(share_difficulty_from_float(-3.0), 1);
        assert_eq!(share_difficulty_from_float(f64::NAN), 1);
        assert_eq!(share_difficulty_from_float(f64::INFINITY), 1);
        assert_eq!(share_difficulty_from_float(1e30), u64::MAX);
    }

    /// Add one to a 32-byte big-endian integer (saturating).
    fn increment_be(bytes: &mut [u8; 32]) {
        for byte in bytes.iter_mut().rev() {
            let (next, carry) = byte.overflowing_add(1);
            *byte = next;
            if !carry {
                return;
            }
        }
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
