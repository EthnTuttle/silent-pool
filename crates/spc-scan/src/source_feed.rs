//! Private collector/agent feed v2. Explicit binary signing preimages;
//! JSON is transport only. No field belongs on an Ark-role interface.
use bitcoin::{
    hashes::{sha256, Hash, HashEngine},
    secp256k1::{schnorr::Signature, Keypair, Message, Secp256k1, XOnlyPublicKey},
};
use serde::{Deserialize, Serialize};
pub const SCHEMA: &str = "spc.source-feed/2";
pub const MAX_FRAME: usize = 9 * 1024 * 1024;
pub const MAX_BUNDLE: usize = 4 * 1024 * 1024;
pub const DEADLINE_SECS: u64 = 10;
pub const MAX_LIFETIME_MS: u64 = 30_000;
pub fn tagged(tag: &str, bytes: &[u8]) -> [u8; 32] {
    let tag = sha256::Hash::hash(tag.as_bytes());
    let mut e = sha256::Hash::engine();
    e.input(tag.as_byte_array());
    e.input(tag.as_byte_array());
    e.input(bytes);
    sha256::Hash::from_engine(e).to_byte_array()
}
pub fn sign(digest: [u8; 32], key: &Keypair) -> String {
    Secp256k1::new()
        .sign_schnorr_no_aux_rand(&Message::from_digest(digest), key)
        .to_string()
}
pub fn verify(digest: [u8; 32], signature: &str, key: &XOnlyPublicKey) -> Result<(), String> {
    let signature: Signature = signature.parse().map_err(|_| "source signature encoding")?;
    Secp256k1::verification_only()
        .verify_schnorr(&signature, &Message::from_digest(digest), key)
        .map_err(|_| "source signature invalid".into())
}
fn blob(out: &mut Vec<u8>, v: &[u8]) {
    out.extend_from_slice(&(v.len() as u32).to_le_bytes());
    out.extend_from_slice(v);
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReceivedJob {
    pub job_id: u32,
    pub prev_hash: [u8; 32],
    pub version: u32,
    pub nbits: u32,
    pub ntime: u32,
    pub coinbase_prefix: Vec<u8>,
    pub coinbase_suffix: Vec<u8>,
    pub merkle_path: Vec<[u8; 32]>,
    pub share_target_be: [u8; 32],
    pub pool_extranonce: Vec<u8>,
    pub miner_extranonce_len: u32,
}
impl ReceivedJob {
    pub fn digest(&self) -> [u8; 32] {
        let mut b = Vec::new();
        b.extend(self.job_id.to_le_bytes());
        b.extend(self.prev_hash);
        for v in [self.version, self.nbits, self.ntime] {
            b.extend(v.to_le_bytes());
        }
        blob(&mut b, &self.coinbase_prefix);
        blob(&mut b, &self.coinbase_suffix);
        b.extend((self.merkle_path.len() as u32).to_le_bytes());
        for h in &self.merkle_path {
            b.extend(h);
        }
        b.extend(self.share_target_be);
        blob(&mut b, &self.pool_extranonce);
        b.extend(self.miner_extranonce_len.to_le_bytes());
        tagged("SPc/ReceivedJobV1", &b)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TraceSeal {
    pub session: [u8; 16],
    pub seq: u64,
    pub digest: [u8; 32],
    pub signature: String,
}
impl TraceSeal {
    pub fn signing_digest(&self) -> [u8; 32] {
        let mut b = self.session.to_vec();
        b.extend(self.seq.to_le_bytes());
        b.extend(self.digest);
        tagged("SPc/SourceTraceSealV1", &b)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Checkpoint {
    pub attempt: [u8; 16],
    pub wire_job_digest: [u8; 32],
    pub job_id: u64,
    pub generation: u64,
    pub source_watermark: u64,
    pub history_watermark: u64,
    pub bundle_digest: [u8; 32],
    pub history_digest: [u8; 32],
    pub expires_at_unix_ms: u64,
    pub complete: bool,
    pub pool: TraceSeal,
    pub signature: String,
}
impl Checkpoint {
    pub fn signing_digest(&self) -> [u8; 32] {
        let mut b = self.attempt.to_vec();
        b.extend(self.wire_job_digest);
        for v in [
            self.job_id,
            self.generation,
            self.source_watermark,
            self.history_watermark,
            self.expires_at_unix_ms,
        ] {
            b.extend(v.to_le_bytes());
        }
        b.extend(self.bundle_digest);
        b.extend(self.history_digest);
        b.push(u8::from(self.complete));
        b.extend(self.pool.signing_digest());
        tagged("SPc/SourceCheckpointV1", &b)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Consumed {
    pub attempt: [u8; 16],
    pub job_id: u64,
    pub generation: u64,
    pub bundle_digest: [u8; 32],
    pub history_digest: [u8; 32],
    pub checkpoint_digest: [u8; 32],
    pub window: crate::source_audit::WindowFacts,
    pub signature: String,
}
impl Consumed {
    pub fn signing_digest(&self) -> [u8; 32] {
        let mut b = self.attempt.to_vec();
        b.extend(self.job_id.to_le_bytes());
        b.extend(self.generation.to_le_bytes());
        b.extend(self.bundle_digest);
        b.extend(self.history_digest);
        b.extend(self.checkpoint_digest);
        b.extend(self.window.window_target.to_le_bytes());
        b.extend(self.window.actual_sum.to_le_bytes());
        b.extend(self.window.shortfall.to_le_bytes());
        b.push(u8::from(self.window.underfilled));
        tagged("SPc/SourceConsumedV2", &b)
    }
}
pub fn committed_digest(attempt: [u8; 16], ack_digest: [u8; 32]) -> [u8; 32] {
    let mut b = attempt.to_vec();
    b.extend(ack_digest);
    tagged("SPc/SourceCommittedV1", &b)
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommittedEvidence {
    pub attempt: [u8; 16],
    pub ack_digest: [u8; 32],
    pub signature: String,
}
/// Public statements only: no scan/spend secret, tweak, source attribution
/// bundle or mutable activation flag is returned to the native actor.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifiedSourceEvidence {
    pub schema: String,
    pub checkpoint: Checkpoint,
    pub consumed: Consumed,
    pub committed: CommittedEvidence,
}
/// Native/public-codec owners reuse this signature/correlation verifier after
/// binding expected_attempt and expected_job_digest to their current candidate.
/// The wallet signature attests the private full-source/owned-output checks.
pub fn verify_public_evidence(
    evidence: &VerifiedSourceEvidence,
    expected_attempt: [u8; 16],
    expected_job_digest: [u8; 32],
    pool: &XOnlyPublicKey,
    collector: &XOnlyPublicKey,
    wallet_scan: &XOnlyPublicKey,
    now_ms: u64,
) -> Result<(), String> {
    let c = &evidence.checkpoint;
    let a = &evidence.consumed;
    let r = &evidence.committed;
    if evidence.schema != SCHEMA
        || pool == collector
        || expected_attempt == [0; 16]
        || c.attempt != expected_attempt
        || a.attempt != expected_attempt
        || r.attempt != expected_attempt
        || c.wire_job_digest != expected_job_digest
        || !c.complete
        || c.source_watermark == 0
        || c.history_watermark == 0
        || c.pool.seq != c.source_watermark
        || c.expires_at_unix_ms <= now_ms
        || c.expires_at_unix_ms - now_ms > MAX_LIFETIME_MS
        || a.job_id != c.job_id
        || a.generation != c.generation
        || a.bundle_digest != c.bundle_digest
        || a.history_digest != c.history_digest
        || a.checkpoint_digest != c.signing_digest()
        || !a.window.is_consistent()
        || r.ack_digest != a.signing_digest()
    {
        return Err("source evidence correlation/completeness/lifetime".into());
    }
    verify(c.pool.signing_digest(), &c.pool.signature, pool)?;
    verify(c.signing_digest(), &c.signature, collector)?;
    verify(a.signing_digest(), &a.signature, wallet_scan)?;
    verify(
        committed_digest(r.attempt, r.ack_digest),
        &r.signature,
        collector,
    )
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Frame {
    Job {
        schema: String,
        attempt: [u8; 16],
        job: ReceivedJob,
    },
    Proof {
        schema: String,
        bundle_hex: String,
        checkpoint: Checkpoint,
    },
    Consumed {
        schema: String,
        ack: Consumed,
    },
    Committed {
        schema: String,
        attempt: [u8; 16],
        ack_digest: [u8; 32],
        signature: String,
    },
    Unavailable {
        schema: String,
        attempt: [u8; 16],
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn collector_and_pool_signatures_bind_distinct_authorities_and_every_activation_context() {
        use bitcoin::secp256k1::SecretKey;
        let key = |seed| {
            Keypair::from_secret_key(
                &Secp256k1::new(),
                &SecretKey::from_slice(&[seed; 32]).unwrap(),
            )
        };
        let pool = key(7);
        let collector = key(8);
        let mut trace = TraceSeal {
            session: [1; 16],
            seq: 9,
            digest: [2; 32],
            signature: String::new(),
        };
        trace.signature = sign(trace.signing_digest(), &pool);
        let mut checkpoint = Checkpoint {
            attempt: [3; 16],
            wire_job_digest: [4; 32],
            job_id: 5,
            generation: 6,
            source_watermark: 9,
            history_watermark: 10,
            bundle_digest: [7; 32],
            history_digest: [8; 32],
            expires_at_unix_ms: 10000,
            complete: true,
            pool: trace,
            signature: String::new(),
        };
        checkpoint.signature = sign(checkpoint.signing_digest(), &collector);
        verify(
            checkpoint.pool.signing_digest(),
            &checkpoint.pool.signature,
            &pool.x_only_public_key().0,
        )
        .unwrap();
        verify(
            checkpoint.signing_digest(),
            &checkpoint.signature,
            &collector.x_only_public_key().0,
        )
        .unwrap();
        assert!(verify(
            checkpoint.signing_digest(),
            &checkpoint.signature,
            &pool.x_only_public_key().0
        )
        .is_err());
        for field in 0..9 {
            let mut changed = checkpoint.clone();
            match field {
                0 => changed.attempt[0] ^= 1,
                1 => changed.wire_job_digest[0] ^= 1,
                2 => changed.job_id += 1,
                3 => changed.generation += 1,
                4 => changed.source_watermark += 1,
                5 => changed.history_watermark += 1,
                6 => changed.bundle_digest[0] ^= 1,
                7 => changed.history_digest[0] ^= 1,
                _ => changed.pool.digest[0] ^= 1,
            }
            assert!(verify(
                changed.signing_digest(),
                &changed.signature,
                &collector.x_only_public_key().0
            )
            .is_err());
        }
    }
    #[test]
    fn consumption_commit_cannot_ack_a_different_job_or_bundle() {
        let ack = Consumed {
            window: crate::source_audit::WindowFacts::from_work(9, 0),
            attempt: [1; 16],
            job_id: 3,
            generation: 4,
            bundle_digest: [5; 32],
            history_digest: [6; 32],
            checkpoint_digest: [7; 32],
            signature: String::new(),
        };
        let mut other = ack.clone();
        other.generation += 1;
        assert_ne!(ack.signing_digest(), other.signing_digest());
        other = ack.clone();
        other.bundle_digest[0] ^= 1;
        assert_ne!(ack.signing_digest(), other.signing_digest());
        assert_ne!(
            committed_digest(ack.attempt, ack.signing_digest()),
            committed_digest([2; 16], ack.signing_digest())
        );
    }
    #[test]
    fn public_evidence_cannot_relabel_warmup_or_change_native_candidate() {
        use bitcoin::secp256k1::SecretKey;
        let key = |seed| {
            Keypair::from_secret_key(
                &Secp256k1::new(),
                &SecretKey::from_slice(&[seed; 32]).unwrap(),
            )
        };
        let pool = key(7);
        let collector = key(8);
        let wallet = key(9);
        let mut trace = TraceSeal {
            session: [1; 16],
            seq: 9,
            digest: [2; 32],
            signature: String::new(),
        };
        trace.signature = sign(trace.signing_digest(), &pool);
        let mut c = Checkpoint {
            attempt: [3; 16],
            wire_job_digest: [4; 32],
            job_id: 5,
            generation: 6,
            source_watermark: 9,
            history_watermark: 10,
            bundle_digest: [7; 32],
            history_digest: [8; 32],
            expires_at_unix_ms: 10000,
            complete: true,
            pool: trace,
            signature: String::new(),
        };
        c.signature = sign(c.signing_digest(), &collector);
        let mut a = Consumed {
            attempt: c.attempt,
            job_id: c.job_id,
            generation: c.generation,
            bundle_digest: c.bundle_digest,
            history_digest: c.history_digest,
            checkpoint_digest: c.signing_digest(),
            window: crate::source_audit::WindowFacts::from_work(9, 0),
            signature: String::new(),
        };
        a.signature = sign(a.signing_digest(), &wallet);
        let r = CommittedEvidence {
            attempt: c.attempt,
            ack_digest: a.signing_digest(),
            signature: sign(committed_digest(c.attempt, a.signing_digest()), &collector),
        };
        let evidence = VerifiedSourceEvidence {
            schema: SCHEMA.into(),
            checkpoint: c,
            consumed: a,
            committed: r,
        };
        let check = |e: &VerifiedSourceEvidence| {
            verify_public_evidence(
                e,
                [3; 16],
                [4; 32],
                &pool.x_only_public_key().0,
                &collector.x_only_public_key().0,
                &wallet.x_only_public_key().0,
                1000,
            )
        };
        check(&evidence).unwrap();
        let mut changed = evidence.clone();
        changed.consumed.window = crate::source_audit::WindowFacts::from_work(9, 9);
        assert!(check(&changed).is_err());
        changed = evidence.clone();
        changed.checkpoint.wire_job_digest[0] ^= 1;
        assert!(check(&changed).is_err());
        changed = evidence.clone();
        changed.consumed.attempt[0] ^= 1;
        assert!(check(&changed).is_err());
        changed = evidence.clone();
        changed.committed.signature = evidence.checkpoint.signature.clone();
        assert!(check(&changed).is_err());
    }
}
