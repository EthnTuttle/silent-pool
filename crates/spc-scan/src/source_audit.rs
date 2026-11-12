//! Full source-share arithmetic verifier used by miner-side payout verification.
//! Credit attribution is authenticated pool-ingress evidence, not a BIP352
//! cryptographic identity proof. Raw header/work/fees/window math is rechecked.
use bitcoin::{
    block::Header,
    consensus::{deserialize, serialize},
    hashes::Hash,
    Transaction,
};
use serde::{Deserialize, Serialize};
use spc_accounting::{ShareRec, Slice, SliceAgg, SpId, WorkUnitProfile};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("source evidence is incomplete: {0}")]
    Unknown(&'static str),
    #[error("source evidence failed validation: {0}")]
    Invalid(&'static str),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobProof {
    pub job_id: u64,
    pub generation: u64,
    pub height: u32,
    pub prev_hash: [u8; 32],
    pub version: u32,
    pub ntime: u32,
    pub nbits: u32,
    pub coinbase_prefix: Vec<u8>,
    pub coinbase_suffix: Vec<u8>,
    pub extranonce_len: usize,
    pub merkle_path: Vec<[u8; 32]>,
    pub share_target_be: [u8; 32],
    pub credited_work: u64,
    pub transactions: Vec<Vec<u8>>,
    /// Real full prevout transactions, keyed by their raw internal txid bytes.
    pub prevout_transactions: Vec<Vec<u8>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptedShare {
    pub identity: SpId,
    pub job_id: u64,
    pub generation: u64,
    pub nonce: u32,
    pub ntime: u32,
    pub version: u32,
    pub extranonce: Vec<u8>,
    pub share_index: u32,
    pub share_hash: [u8; 32],
    pub credited_work: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SliceProof {
    pub slice_id: u64,
    pub height: u32,
    pub front_id: u8,
    pub work_unit_version: u8,
    pub work_unit_fingerprint: [u8; 32],
    pub difficulty: String,
    pub root: [u8; 32],
    pub shares: Vec<AcceptedShare>,
}

/// A separately validated history checkpoint is mandatory. Its digest must
/// come from the owner/operator's trusted source collector, never the bundle.
/// The collector replays source_accounting_history under its captured SQL
/// watermark and raw canonical settlement/reorg ancestry (WIRE §3.9).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryCheckpoint {
    pub watermark: u64,
    pub digest: [u8; 32],
    /// Available A at the issued job, after the independent collector has
    /// replayed the complete Rule-B reservation/booking/reorg history. Raw SQL
    /// booked B alone is not a valid checkpoint (WIRE §3.9).
    pub pending: BTreeMap<String, u64>,
}

pub struct TrustedSource {
    /// Independently captured complete proof-bundle bytes digest.
    pub bundle_digest: [u8; 32],
    /// Independently verified SQL/chain history checkpoint digest.
    pub history_digest: [u8; 32],
    pub history_watermark: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceBundle {
    pub schema: String,
    pub source_complete: bool,
    pub source_watermark: u64,
    pub job: JobProof,
    pub jobs: Vec<JobProof>,
    pub reference_target_be: [u8; 32],
    pub work_unit_fingerprint: [u8; 32],
    pub window_n: u32,
    pub window_target: String,
    pub high_water: BTreeMap<u8, u64>,
    pub finding_slice_id: u64,
    pub slices: Vec<SliceProof>,
    pub subsidy: u64,
    pub fees: u64,
    pub fee_bps: u16,
    pub prior: HistoryCheckpoint,
    pub owed: BTreeMap<String, u64>,
    /// Mandatory for audited underfill. Sealed by the independent collector;
    /// missing/pruned eligible history must never set this completeness claim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<WindowEvidence>,
    /// Reserved witness values from the authenticated immutable job records.
    /// Required for every current/referenced job containing a commitment.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub witness_reserved: BTreeMap<u64, [u8; 32]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowFacts {
    #[serde(with = "decimal_u128")]
    pub window_target: u128,
    #[serde(with = "decimal_u128")]
    pub actual_sum: u128,
    #[serde(with = "decimal_u128")]
    pub shortfall: u128,
    pub underfilled: bool,
}
impl WindowFacts {
    pub fn from_work(window_target: u128, actual_sum: u128) -> Self {
        Self {
            window_target,
            actual_sum,
            shortfall: window_target.saturating_sub(actual_sum),
            underfilled: actual_sum < window_target,
        }
    }
    pub fn is_consistent(&self) -> bool {
        self.window_target > 0 && *self == Self::from_work(self.window_target, self.actual_sum)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowEvidence {
    pub facts: WindowFacts,
    pub eligible_history_complete: bool,
}
mod decimal_u128 {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(n: &u128, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&n.to_string())
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u128, D::Error> {
        let s = String::deserialize(d)?;
        let n = s.parse::<u128>().map_err(serde::de::Error::custom)?;
        if s != n.to_string() {
            return Err(serde::de::Error::custom("noncanonical decimal work"));
        }
        Ok(n)
    }
}

/// Cannot be constructed from an owed scalar. Obtained only after complete
/// source/ancestry binding and independent share/window arithmetic verification.
pub struct ValidatedEntitlement {
    pub(crate) owed: u64,
    pub job: JobProof,
    window: WindowFacts,
}
impl ValidatedEntitlement {
    pub fn owed(&self) -> u64 {
        self.owed
    }
    pub fn window(&self) -> WindowFacts {
        self.window
    }
}

pub struct VerifiedOwnedSource {
    pub entitlement: ValidatedEntitlement,
    pub outputs: Vec<crate::Found>,
    pub remainder: u64,
}

/// The same full-owned-output check for warmup and full windows. No caller
/// supplied owed scalar, alternative scanner, or warmup absence bypass.
pub fn verify_owned(
    bytes: &[u8],
    trusted: &TrustedSource,
    issued: &JobProof,
    scan: &bitcoin::secp256k1::SecretKey,
    spend: &bitcoin::secp256k1::PublicKey,
) -> Result<VerifiedOwnedSource, Error> {
    let public =
        bitcoin::secp256k1::PublicKey::from_secret_key(&bitcoin::secp256k1::Secp256k1::new(), scan);
    let entitlement = verify(bytes, trusted, spc_crypto::sp_id(&public, spend), issued)?;
    let tx = job_coinbase(issued)?;
    let a = crate::carrier_from_tx(&tx).ok_or(Error::Invalid("current job carrier"))?;
    let outputs: Vec<_> = tx
        .output
        .iter()
        .map(|o| crate::OutputView {
            script_pubkey: o.script_pubkey.as_bytes(),
            value: o.value.to_sat(),
        })
        .collect();
    let found = crate::scan_outputs(issued.height, &a, &outputs, scan, spend)
        .map_err(|_| Error::Invalid("full owned output derivation/set"))?;
    let remainder = crate::audit_outputs(
        &found,
        entitlement.owed(),
        crate::PayoutFormat::FixedLadderV1,
    )
    .map_err(|_| Error::Invalid("full owned denomination entitlement"))?;
    Ok(VerifiedOwnedSource {
        entitlement,
        outputs: found,
        remainder,
    })
}

/// Public native/translator seam: bind the actual received bytes and nonce
/// layout, then run the identical pure source and ownership verifier.
pub fn verify_received_owned(
    bytes: &[u8],
    trusted: &TrustedSource,
    received: &crate::source_feed::ReceivedJob,
    scan: &bitcoin::secp256k1::SecretKey,
    spend: &bitcoin::secp256k1::PublicKey,
) -> Result<VerifiedOwnedSource, Error> {
    if bytes.len() > 4 * 1024 * 1024 {
        return Err(Error::Invalid("proof bundle bound"));
    }
    let bundle: SourceBundle =
        serde_json::from_slice(bytes).map_err(|_| Error::Invalid("proof bundle schema"))?;
    let len = received
        .pool_extranonce
        .len()
        .checked_add(received.miner_extranonce_len as usize)
        .ok_or(Error::Invalid("extranonce length"))?;
    if len > 63 {
        return Err(Error::Invalid("extranonce length"));
    }
    let mut raw = received.coinbase_prefix.clone();
    raw.extend(&received.pool_extranonce);
    raw.extend(std::iter::repeat_n(
        0,
        received.miner_extranonce_len as usize,
    ));
    raw.extend(&received.coinbase_suffix);
    let tx = transaction(&raw)?;
    let height = tx
        .input
        .first()
        .and_then(|i| crate::parse_bip34_height(i.script_sig.as_bytes()))
        .ok_or(Error::Invalid("received coinbase height"))?;
    let issued = JobProof {
        job_id: (u64::from(height) << 32) | u64::from(received.job_id),
        generation: bundle.job.generation,
        height,
        prev_hash: received.prev_hash,
        version: received.version,
        ntime: received.ntime,
        nbits: received.nbits,
        coinbase_prefix: received.coinbase_prefix.clone(),
        coinbase_suffix: received.coinbase_suffix.clone(),
        extranonce_len: len,
        merkle_path: received.merkle_path.clone(),
        share_target_be: received.share_target_be,
        credited_work: bundle.job.credited_work,
        transactions: vec![],
        prevout_transactions: vec![],
    };
    verify_owned(bytes, trusted, &issued, scan, spend)
}

fn job_coinbase(job: &JobProof) -> Result<Transaction, Error> {
    if job.extranonce_len > 63 {
        return Err(Error::Invalid("job extranonce bound"));
    }
    let mut raw = job.coinbase_prefix.clone();
    raw.extend(std::iter::repeat_n(0, job.extranonce_len));
    raw.extend(&job.coinbase_suffix);
    let tx = transaction(&raw)?;
    if !tx.is_coinbase()
        || crate::parse_bip34_height(tx.input[0].script_sig.as_bytes()) != Some(job.height)
    {
        return Err(Error::Invalid("job coinbase/height"));
    }
    if !(2..=100).contains(&tx.input[0].script_sig.len()) {
        return Err(Error::Invalid("coinbase scriptSig budget"));
    }
    if job.extranonce_len > 0 {
        // Canonical one-input coinbase, scriptSig <=100: both CompactSizes
        // occupy one byte. Nonces may not overlap height/carrier or any output.
        let script_start = 4 + usize::from(!tx.input[0].witness.is_empty()) * 2 + 1 + 36 + 1;
        let a = crate::carrier_from_tx(&tx).ok_or(Error::Invalid("job carrier"))?;
        let immutable = spc_crypto::emit_script_sig(job.height, &a);
        let start = job.coinbase_prefix.len();
        if !tx.input[0].script_sig.as_bytes().starts_with(&immutable)
            || start < script_start + immutable.len()
            || start
                .checked_add(job.extranonce_len)
                .is_none_or(|end| end > script_start + tx.input[0].script_sig.len())
        {
            return Err(Error::Invalid(
                "extranonce outside immutable coinbase scriptSig suffix",
            ));
        }
    }
    Ok(tx)
}

fn validate_job(job: &JobProof, reserved: Option<&[u8; 32]>) -> Result<Transaction, Error> {
    let cb = job_coinbase(job)?;
    let mut txids = vec![cb.compute_txid().to_byte_array()];
    let mut wtxids = vec![[0; 32]];
    let mut witness = false;
    for raw in &job.transactions {
        let tx = transaction(raw)?;
        witness |= tx.input.iter().any(|i| !i.witness.is_empty());
        txids.push(tx.compute_txid().to_byte_array());
        wtxids.push(tx.compute_wtxid().to_byte_array());
    }
    let mut branch = txids[0];
    for sibling in &job.merkle_path {
        let mut pair = branch.to_vec();
        pair.extend(sibling);
        branch = bitcoin::hashes::sha256d::Hash::hash(&pair).to_byte_array();
    }
    if branch != spc_accounting::merkle_root(&txids) {
        return Err(Error::Invalid("current job transaction/Merkle path"));
    }
    let commitment = cb.output.iter().rev().find_map(|o| {
        let s = o.script_pubkey.as_bytes();
        (s.len() >= 38 && s[..6] == [0x6a, 0x24, 0xaa, 0x21, 0xa9, 0xed]).then(|| &s[6..38])
    });
    if witness || commitment.is_some() || !cb.input[0].witness.is_empty() || reserved.is_some() {
        let reserved = reserved.ok_or(Error::Unknown("immutable witness reserved value"))?;
        if !cb.input[0].witness.is_empty()
            && (cb.input[0].witness.len() != 1
                || cb.input[0].witness.iter().next() != Some(reserved.as_slice()))
        {
            return Err(Error::Invalid("coinbase witness reserved value"));
        }
        let mut preimage = spc_accounting::merkle_root(&wtxids).to_vec();
        preimage.extend(reserved);
        if commitment
            != Some(
                bitcoin::hashes::sha256d::Hash::hash(&preimage)
                    .as_byte_array()
                    .as_slice(),
            )
        {
            return Err(Error::Invalid("job witness commitment"));
        }
    }
    Ok(cb)
}

fn transaction(bytes: &[u8]) -> Result<Transaction, Error> {
    let tx: Transaction = deserialize(bytes).map_err(|_| Error::Invalid("transaction decode"))?;
    if serialize(&tx) != bytes {
        return Err(Error::Invalid("noncanonical transaction"));
    }
    Ok(tx)
}
fn job_fees(job: &JobProof) -> Result<u64, Error> {
    let mut prevouts = BTreeMap::new();
    for raw in &job.prevout_transactions {
        let tx = transaction(raw)?;
        if prevouts.insert(tx.compute_txid(), tx).is_some() {
            return Err(Error::Invalid("duplicate prevout transaction"));
        }
    }
    let mut total = 0u64;
    for raw in &job.transactions {
        let tx = transaction(raw)?;
        let mut input = 0u64;
        if tx.is_coinbase() {
            return Err(Error::Invalid("coinbase in job transaction set"));
        }
        for vin in &tx.input {
            let parent = prevouts
                .get(&vin.previous_output.txid)
                .ok_or(Error::Unknown("fee prevout transaction"))?;
            let output = parent
                .output
                .get(vin.previous_output.vout as usize)
                .ok_or(Error::Invalid("prevout index"))?;
            input = input
                .checked_add(output.value.to_sat())
                .ok_or(Error::Invalid("input amount overflow"))?;
        }
        let output = tx
            .output
            .iter()
            .try_fold(0u64, |sum, o| sum.checked_add(o.value.to_sat()))
            .ok_or(Error::Invalid("output amount overflow"))?;
        total = total
            .checked_add(
                input
                    .checked_sub(output)
                    .ok_or(Error::Invalid("negative transaction fee"))?,
            )
            .ok_or(Error::Invalid("job fee overflow"))?;
    }
    Ok(total)
}

fn share_record(share: &AcceptedShare, job: &JobProof, fees: u64) -> Result<ShareRec, Error> {
    if share.job_id != job.job_id
        || share.generation != job.generation
        || share.credited_work != job.credited_work
        || share.extranonce.len() != job.extranonce_len
        || share.ntime < job.ntime
        || share.ntime > job.ntime.saturating_add(7200)
        || (share.version ^ job.version) & !0x1fff_e000 != 0
    {
        return Err(Error::Invalid("issued job/share binding"));
    }
    let mut bytes = job.coinbase_prefix.clone();
    bytes.extend_from_slice(&share.extranonce);
    bytes.extend_from_slice(&job.coinbase_suffix);
    let cb = transaction(&bytes)?;
    if !cb.is_coinbase()
        || crate::parse_bip34_height(cb.input[0].script_sig.as_bytes()) != Some(job.height)
    {
        return Err(Error::Invalid("job coinbase/height"));
    }
    let mut root = cb.compute_txid().to_byte_array();
    for sibling in &job.merkle_path {
        let mut pair = root.to_vec();
        pair.extend_from_slice(sibling);
        root = bitcoin::hashes::sha256d::Hash::hash(&pair).to_byte_array();
    }
    let mut txids = vec![cb.compute_txid().to_byte_array()];
    for raw in &job.transactions {
        txids.push(transaction(raw)?.compute_txid().to_byte_array());
    }
    if spc_accounting::merkle_root(&txids) != root {
        return Err(Error::Invalid("job transaction set/Merkle path"));
    }
    let header = Header {
        version: bitcoin::block::Version::from_consensus(share.version as i32),
        prev_blockhash: bitcoin::BlockHash::from_byte_array(job.prev_hash),
        merkle_root: bitcoin::TxMerkleNode::from_byte_array(root),
        time: share.ntime,
        bits: bitcoin::CompactTarget::from_consensus(job.nbits),
        nonce: share.nonce,
    };
    if header.block_hash().to_byte_array() != share.share_hash
        || !bitcoin::Target::from_be_bytes(job.share_target_be).is_met_by(header.block_hash())
    {
        return Err(Error::Invalid("raw share header/PoW"));
    }
    Ok(ShareRec {
        sp_id: share.identity,
        d: share.credited_work,
        job_fees: fees,
        job_id: share.job_id,
        nonce: share.nonce,
        ntime: share.ntime,
        version: share.version,
        extranonce: share.extranonce.clone(),
        share_index: share.share_index,
    })
}

pub fn verify(
    bytes: &[u8],
    trusted: &TrustedSource,
    identity: SpId,
    issued_job: &JobProof,
) -> Result<ValidatedEntitlement, Error> {
    if bytes.len() > 4 * 1024 * 1024 {
        return Err(Error::Invalid("proof bundle bound"));
    }
    if bitcoin::hashes::sha256::Hash::hash(bytes).to_byte_array() != trusted.bundle_digest {
        return Err(Error::Invalid("external source seal"));
    }
    let bundle: SourceBundle =
        serde_json::from_slice(bytes).map_err(|_| Error::Invalid("proof bundle schema"))?;
    if bundle.schema != "spc.payout-proof/1"
        || !bundle.source_complete
        || bundle.source_watermark == 0
    {
        return Err(Error::Unknown("complete source watermark"));
    }
    if bundle.prior.watermark == 0 || trusted.history_watermark == 0 {
        return Err(Error::Unknown("economic history checkpoint"));
    }
    if bundle.prior.digest != trusted.history_digest
        || bundle.prior.watermark != trusted.history_watermark
        || history_digest(bundle.prior.watermark, &bundle.prior.pending)? != trusted.history_digest
    {
        return Err(Error::Invalid("pending/settlement/reorg ancestry seal"));
    }
    if !same_issued_job(&bundle.job, issued_job) {
        return Err(Error::Invalid("actual issued job bytes"));
    }
    // v2 is explicitly regtest-only. Subsidy is consensus-derived from height;
    // fees come from real transactions/prevouts, never a supplied payout total.
    let halvings = bundle.job.height / 150;
    let subsidy = if halvings >= 64 {
        0
    } else {
        5_000_000_000u64 >> halvings
    };
    if bundle.subsidy != subsidy || job_fees(&bundle.job)? != bundle.fees {
        return Err(Error::Invalid("contextual subsidy/prevout fees"));
    }
    if bundle.fee_bps > 10_000 {
        return Err(Error::Invalid("fee basis points"));
    }
    let coinbase = validate_job(&bundle.job, bundle.witness_reserved.get(&bundle.job.job_id))?;
    let revenue = bundle
        .subsidy
        .checked_add(bundle.fees)
        .ok_or(Error::Invalid("revenue overflow"))?;
    let output_sum = coinbase
        .output
        .iter()
        .try_fold(0u64, |n, o| n.checked_add(o.value.to_sat()))
        .ok_or(Error::Invalid("coinbase value overflow"))?;
    if output_sum != revenue {
        return Err(Error::Invalid("coinbase revenue conservation"));
    }
    let profile = WorkUnitProfile::from_reference(
        bundle.reference_target_be,
        spc_accounting::work_unit::HARD_CAP_MAX_WORK_MULTIPLIER,
    )
    .map_err(|_| Error::Invalid("work reference profile"))?;
    if profile.fingerprint() != bundle.work_unit_fingerprint {
        return Err(Error::Invalid("work profile fingerprint"));
    }
    if profile
        .multiplier_for_share_target(&spc_accounting::work_unit::Target::from_be_bytes(
            bundle.job.share_target_be,
        ))
        .map_err(|_| Error::Invalid("issued work family"))?
        != bundle.job.credited_work
    {
        return Err(Error::Invalid("issued credited work"));
    }
    let network = spc_accounting::work_unit::decode_compact_target(bundle.job.nbits)
        .map_err(|_| Error::Invalid("network target"))?;
    let q = spc_accounting::work_unit::window_target_network_work(
        &profile,
        network,
        u64::from(bundle.window_n),
        spc_accounting::work_unit::HARD_CAP_MAX_WINDOW_TARGET_UNITS,
    )
    .map_err(|_| Error::Invalid("contextual window target"))?;
    let frozen = bundle
        .window_target
        .parse::<u128>()
        .map_err(|_| Error::Invalid("frozen window target"))?;
    if q != frozen {
        return Err(Error::Invalid("frozen window Q"));
    }
    if bundle.slices.len() > 4096 || bundle.jobs.len() > 4096 {
        return Err(Error::Unknown("bounded complete window"));
    }
    let finding_front = (bundle.finding_slice_id >> 56) as u8;
    if bundle.finding_slice_id == 0 || !bundle.high_water.contains_key(&finding_front) {
        return Err(Error::Unknown("complete front high-water inventory"));
    }
    for (front, mark) in &bundle.high_water {
        let newest = bundle
            .slices
            .iter()
            .find(|row| row.front_id == *front)
            .map_or(0, |row| row.slice_id);
        if *mark != newest {
            return Err(Error::Unknown("high-water slice missing"));
        }
    }
    let jobs: BTreeMap<_, _> = bundle.jobs.iter().map(|j| (j.job_id, j)).collect();
    if jobs.len() != bundle.jobs.len() {
        return Err(Error::Invalid("duplicate job generation"));
    }
    let fees_by_job = jobs
        .iter()
        .map(|(id, job)| {
            validate_job(job, bundle.witness_reserved.get(id))?;
            Ok((*id, job_fees(job)?))
        })
        .collect::<Result<BTreeMap<_, _>, Error>>()?;
    let mut aggregates = Vec::<SliceAgg>::new();
    let mut seen = BTreeSet::new();
    let mut previous = None;
    let mut fronts = BTreeSet::new();
    for row in &bundle.slices {
        if row.shares.is_empty()
            || row.height > bundle.job.height
            || row.height < bundle.job.height.saturating_sub(4096)
            || (row.slice_id >> 56) as u8 != row.front_id
        {
            return Err(Error::Invalid("eligible completed slice"));
        }
        if row.work_unit_version != 2 || row.work_unit_fingerprint != bundle.work_unit_fingerprint {
            return Err(Error::Invalid("mixed slice profile"));
        }
        if fronts.insert(row.front_id)
            && bundle.high_water.get(&row.front_id) != Some(&row.slice_id)
        {
            return Err(Error::Unknown("high-water slice missing"));
        }
        if row.slice_id == bundle.finding_slice_id
            || previous.is_some_and(|id| row.slice_id >= id)
            || bundle
                .high_water
                .get(&row.front_id)
                .is_none_or(|mark| row.slice_id > *mark)
        {
            return Err(Error::Invalid("frozen slice order/high-water"));
        }
        previous = Some(row.slice_id);
        let mut slice = Slice::new_v2(
            row.slice_id,
            row.height,
            [0; 32],
            row.front_id,
            0,
            0,
            bundle.work_unit_fingerprint,
        );
        for share in &row.shares {
            if !seen.insert(share.share_hash) {
                return Err(Error::Invalid("duplicate accepted share"));
            }
            let job = jobs
                .get(&share.job_id)
                .ok_or(Error::Unknown("referenced immutable job"))?;
            let target = spc_accounting::work_unit::Target::from_be_bytes(job.share_target_be);
            if profile
                .multiplier_for_share_target(&target)
                .map_err(|_| Error::Invalid("exact issued work family"))?
                != job.credited_work
            {
                return Err(Error::Invalid("credit differs from reference target ratio"));
            }
            let record = share_record(share, job, fees_by_job[&share.job_id])?;
            slice
                .push_v2(record)
                .map_err(|_| Error::Invalid("checked share work arithmetic"))?;
        }
        slice.seal();
        if slice.root != row.root || slice.difficulty.to_string() != row.difficulty {
            return Err(Error::Invalid("sealed share attribution/root/sum"));
        }
        aggregates.push(
            slice
                .agg_v2()
                .map_err(|_| Error::Invalid("slice aggregate"))?,
        );
    }
    let window = spc_accounting::select_window_v2(
        &aggregates,
        bundle.finding_slice_id,
        frozen,
        BTreeMap::new(),
        Some(bundle.work_unit_fingerprint),
    )
    .map_err(|_| Error::Invalid("frozen window profile"))?;
    let facts = WindowFacts::from_work(frozen, window.d_win);
    match &bundle.window {
        Some(evidence) => {
            if !evidence.eligible_history_complete {
                return Err(Error::Unknown("eligible history incomplete or pruned"));
            }
            if evidence.facts != facts {
                return Err(Error::Invalid("frozen window facts"));
            }
        }
        None if facts.underfilled => {
            return Err(Error::Unknown(
                "audited underfill requires complete eligible-history evidence",
            ))
        }
        None => {}
    }
    let raw = if window.slices.is_empty() {
        // Identical to the production front's empty-window raw adapter. Pending
        // is still added below; it is never erased to manufacture bootstrap.
        // Empty raw can only pay prior-backed rungs. This is a per-entitlement
        // bound, not a K/fee floor. Unknown-key/error holds can pay less; the
        // ordinary full-owned-key audit and independent complete source remain
        // required. Total prior itself is never capped at current T.
        let max_prior_rungs = bundle
            .prior
            .pending
            .values()
            .try_fold(0u128, |sum, p| {
                sum.checked_add(u128::from(spc_accounting::band_amount(*p).0))
            })
            .ok_or(Error::Invalid("prior rung sum overflow"))?;
        let mut actual_paid = 0u128;
        for output in coinbase.output.iter().skip(1) {
            let script = output.script_pubkey.as_bytes();
            if script.first() == Some(&0x6a) && output.value.to_sat() == 0 {
                continue;
            }
            if script.len() != 34
                || script[..2] != [0x51, 0x20]
                || !spc_accounting::LADDER_BASE_RUNGS_SATS.contains(&output.value.to_sat())
            {
                return Err(Error::Invalid("empty-window routing/revenue conservation"));
            }
            actual_paid = actual_paid
                .checked_add(u128::from(output.value.to_sat()))
                .ok_or(Error::Invalid("paid rung sum overflow"))?;
        }
        if actual_paid > max_prior_rungs
            || coinbase.output.first().is_none_or(|o| {
                o.script_pubkey.len() != 34 || o.script_pubkey.as_bytes()[..2] != [0x51, 0x20]
            })
        {
            return Err(Error::Invalid("empty-window routing/revenue conservation"));
        }
        spc_accounting::RawPayouts {
            raw: BTreeMap::new(),
            pool_sats: revenue,
            r_net: bundle.subsidy,
            f_net: bundle.fees,
        }
    } else {
        spc_accounting::raw_payouts_v2(&window, bundle.subsidy, bundle.fees, bundle.fee_bps)
            .map_err(|_| Error::Invalid("payout arithmetic"))?
    };
    let mut all_ids = BTreeSet::from([identity]);
    all_ids.extend(raw.raw.keys().copied());
    for id in bundle.prior.pending.keys() {
        let bytes = hex::decode(id).map_err(|_| Error::Invalid("pending identity encoding"))?;
        let parsed: SpId = bytes
            .try_into()
            .map_err(|_| Error::Invalid("pending identity length"))?;
        if hex::encode(parsed) != *id {
            return Err(Error::Invalid("pending identity canonical encoding"));
        }
        all_ids.insert(parsed);
    }
    let mut expected = BTreeMap::new();
    for id in all_ids {
        let key = hex::encode(id);
        let prior = bundle
            .prior
            .pending
            .get(&key)
            .ok_or(Error::Unknown("complete identity pending ancestry"))?;
        expected.insert(
            key,
            raw.raw
                .get(&id)
                .copied()
                .unwrap_or(0)
                .checked_add(*prior)
                .ok_or(Error::Invalid("owed overflow"))?,
        );
    }
    if expected != bundle.owed {
        return Err(Error::Invalid(
            "complete full owed differs from source proof",
        ));
    }
    let identity_hex = hex::encode(identity);
    let before = *bundle
        .prior
        .pending
        .get(&identity_hex)
        .ok_or(Error::Unknown("identity pending ancestry"))?;
    let owed = raw
        .raw
        .get(&identity)
        .copied()
        .unwrap_or(0)
        .checked_add(before)
        .ok_or(Error::Invalid("owed overflow"))?;
    if bundle.owed.get(&identity_hex) != Some(&owed) {
        return Err(Error::Invalid("full owed differs from source proof"));
    }
    Ok(ValidatedEntitlement {
        owed,
        job: bundle.job,
        window: facts,
    })
}
fn same_issued_job(a: &JobProof, b: &JobProof) -> bool {
    a.job_id == b.job_id
        && a.generation == b.generation
        && a.height == b.height
        && a.prev_hash == b.prev_hash
        && a.version == b.version
        && a.ntime == b.ntime
        && a.nbits == b.nbits
        && a.coinbase_prefix == b.coinbase_prefix
        && a.coinbase_suffix == b.coinbase_suffix
        && a.extranonce_len == b.extranonce_len
        && a.merkle_path == b.merkle_path
        && a.share_target_be == b.share_target_be
        && a.credited_work == b.credited_work
}
pub fn history_digest(watermark: u64, pending: &BTreeMap<String, u64>) -> Result<[u8; 32], Error> {
    use bitcoin::hashes::{sha256, HashEngine};
    let mut h = sha256::Hash::engine();
    h.input(b"SPc/VerifiedHistoryCheckpointV1");
    h.input(&watermark.to_le_bytes());
    h.input(
        &serde_json::to_vec(pending).map_err(|_| Error::Invalid("pending checkpoint encoding"))?,
    );
    Ok(sha256::Hash::from_engine(h).to_byte_array())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (SourceBundle, SpId) {
        // Explicit raw-header unit fixture. No mined/funded acceptance claim.
        use bitcoin::{Amount, OutPoint, ScriptBuf, TxIn, TxOut, Witness};
        let id = [1; 32];
        let profile = WorkUnitProfile::lab_default();
        let send = spc_crypto::SendKey::generate(1).unwrap();
        let tx = Transaction {
            version: bitcoin::transaction::Version::ONE,
            lock_time: bitcoin::absolute::LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::null(),
                script_sig: ScriptBuf::from_bytes(spc_crypto::emit_script_sig(1, send.x_only())),
                sequence: bitcoin::Sequence::MAX,
                witness: Witness::new(),
            }],
            output: vec![TxOut {
                value: Amount::from_sat(5_000_000_000),
                script_pubkey: ScriptBuf::from_bytes(spc_accounting::p2tr_script(&[2; 32])),
            }],
        };
        let job = JobProof {
            job_id: (1u64 << 32) | 1,
            generation: 1,
            height: 1,
            prev_hash: [0; 32],
            version: 0x2000_0000,
            ntime: 1,
            nbits: 0x207f_ffff,
            coinbase_prefix: serialize(&tx),
            coinbase_suffix: vec![],
            extranonce_len: 0,
            merkle_path: vec![],
            share_target_be: profile.reference_bytes(),
            credited_work: 1,
            transactions: vec![],
            prevout_transactions: vec![],
        };
        let mut shares = Vec::new();
        for nonce in 0..1000 {
            let header = Header {
                version: bitcoin::block::Version::from_consensus(job.version as i32),
                prev_blockhash: bitcoin::BlockHash::all_zeros(),
                merkle_root: bitcoin::TxMerkleNode::from_raw_hash(tx.compute_txid().to_raw_hash()),
                time: 1,
                bits: bitcoin::CompactTarget::from_consensus(job.nbits),
                nonce,
            };
            if bitcoin::Target::from_be_bytes(job.share_target_be).is_met_by(header.block_hash()) {
                shares.push(AcceptedShare {
                    identity: id,
                    job_id: job.job_id,
                    generation: 1,
                    nonce,
                    ntime: 1,
                    version: job.version,
                    extranonce: vec![],
                    share_index: shares.len() as u32,
                    share_hash: header.block_hash().to_byte_array(),
                    credited_work: 1,
                });
                if shares.len() == 2 {
                    break;
                }
            }
        }
        assert_eq!(shares.len(), 2);
        let mut slice = Slice::new_v2(1, 1, [0; 32], 0, job.job_id, 0, profile.fingerprint());
        for share in &shares {
            slice
                .push_v2(share_record(share, &job, 0).unwrap())
                .unwrap();
        }
        slice.seal();
        let pending = BTreeMap::from([(hex::encode(id), 0)]);
        let digest = history_digest(1, &pending).unwrap();
        (
            SourceBundle {
                window: None,
                witness_reserved: BTreeMap::new(),
                schema: "spc.payout-proof/1".into(),
                source_complete: true,
                source_watermark: 9,
                job: job.clone(),
                jobs: vec![job],
                reference_target_be: profile.reference_bytes(),
                work_unit_fingerprint: profile.fingerprint(),
                window_n: 1,
                window_target: "2".into(),
                high_water: BTreeMap::from([(0, 1)]),
                finding_slice_id: 2,
                slices: vec![SliceProof {
                    slice_id: 1,
                    height: 1,
                    front_id: 0,
                    work_unit_version: 2,
                    work_unit_fingerprint: profile.fingerprint(),
                    difficulty: "2".into(),
                    root: slice.root,
                    shares,
                }],
                subsidy: 5_000_000_000,
                fees: 0,
                fee_bps: 100,
                prior: HistoryCheckpoint {
                    watermark: 1,
                    digest,
                    pending,
                },
                owed: BTreeMap::from([(hex::encode(id), 4_950_000_000)]),
            },
            id,
        )
    }
    fn seal(bundle: &SourceBundle) -> (Vec<u8>, TrustedSource) {
        let bytes = serde_json::to_vec(bundle).unwrap();
        let trusted = TrustedSource {
            bundle_digest: bitcoin::hashes::sha256::Hash::hash(&bytes).to_byte_array(),
            history_digest: bundle.prior.digest,
            history_watermark: bundle.prior.watermark,
        };
        (bytes, trusted)
    }
    fn owned_warmup(
        work: usize,
        prior: u64,
    ) -> (
        SourceBundle,
        bitcoin::secp256k1::SecretKey,
        bitcoin::secp256k1::PublicKey,
    ) {
        // Raw-header conformance fixture, not synthetic live acceptance credit.
        use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
        let (mut bundle, _) = fixture();
        let secp = Secp256k1::new();
        let scan = SecretKey::from_slice(&[11; 32]).unwrap();
        let scan_public = PublicKey::from_secret_key(&secp, &scan);
        let spend = PublicKey::from_secret_key(&secp, &SecretKey::from_slice(&[12; 32]).unwrap());
        let id = spc_crypto::sp_id(&scan_public, &spend);
        if work == 0 {
            bundle.slices.clear();
            bundle.jobs.clear();
            bundle.high_water.insert(0, 0);
        } else {
            let row = &mut bundle.slices[0];
            row.shares.truncate(work);
            let mut slice = Slice::new_v2(
                row.slice_id,
                row.height,
                [0; 32],
                row.front_id,
                0,
                0,
                bundle.work_unit_fingerprint,
            );
            for share in &mut row.shares {
                share.identity = id;
                slice
                    .push_v2(share_record(share, &bundle.jobs[0], 0).unwrap())
                    .unwrap();
            }
            slice.seal();
            row.root = slice.root;
            row.difficulty = slice.difficulty.to_string();
        }
        let owed = if work == 0 {
            prior
        } else {
            4_950_000_000 + prior
        };
        bundle.prior.pending = BTreeMap::from([(hex::encode(id), prior)]);
        bundle.prior.digest =
            history_digest(bundle.prior.watermark, &bundle.prior.pending).unwrap();
        bundle.owed = BTreeMap::from([(hex::encode(id), owed)]);
        bundle.window = Some(WindowEvidence {
            facts: WindowFacts::from_work(2, work as u128),
            eligible_history_complete: true,
        });
        let send = spc_crypto::SendKey::generate(1).unwrap();
        let mut cb = job_coinbase(&bundle.job).unwrap();
        cb.input[0].script_sig =
            bitcoin::ScriptBuf::from_bytes(spc_crypto::emit_script_sig(1, send.x_only()));
        let ecdh = send.derive_pool_side(&scan_public).unwrap();
        let decomposition =
            spc_accounting::decompose(owed, spc_accounting::ladder::MAX_LADDER_OUTPUTS).unwrap();
        let mut owned = Vec::new();
        for (k, amount) in decomposition.outputs.iter().enumerate() {
            let tweaks = spc_crypto::tier_tweaks(&ecdh, k as u32).unwrap();
            let key = spc_crypto::spc_output_pubkey(&spend, &tweaks.t_onchain)
                .unwrap()
                .x_only_public_key()
                .0
                .serialize();
            owned.push(bitcoin::TxOut {
                value: bitcoin::Amount::from_sat(*amount),
                script_pubkey: bitcoin::ScriptBuf::from_bytes(spc_accounting::p2tr_script(&key)),
            });
        }
        owned.sort_by(|a, b| a.script_pubkey.cmp(&b.script_pubkey));
        let paid = decomposition.outputs.iter().sum::<u64>();
        cb.output[0].value = bitcoin::Amount::from_sat(5_000_000_000 - paid);
        cb.output.extend(owned);
        bundle.job.coinbase_prefix = serialize(&cb);
        bundle.job.job_id += 1;
        bundle.job.generation += 1;
        (bundle, scan, spend)
    }
    fn received(job: &JobProof) -> crate::source_feed::ReceivedJob {
        crate::source_feed::ReceivedJob {
            job_id: job.job_id as u32,
            prev_hash: job.prev_hash,
            version: job.version,
            nbits: job.nbits,
            ntime: job.ntime,
            coinbase_prefix: job.coinbase_prefix.clone(),
            coinbase_suffix: job.coinbase_suffix.clone(),
            merkle_path: job.merkle_path.clone(),
            share_target_be: job.share_target_be,
            pool_extranonce: vec![],
            miner_extranonce_len: job.extranonce_len as u32,
        }
    }
    #[test]
    fn audited_empty_partial_and_full_native_contexts_keep_q_and_full_per_block_math() {
        for work in [0, 1, 2] {
            let (bundle, scan, spend) = owned_warmup(work, 0);
            let (bytes, trusted) = seal(&bundle);
            let verified =
                verify_received_owned(&bytes, &trusted, &received(&bundle.job), &scan, &spend)
                    .unwrap();
            assert_eq!(
                verified.entitlement.window(),
                WindowFacts::from_work(2, work as u128)
            );
            assert_eq!(
                verified.entitlement.owed(),
                if work == 0 { 0 } else { 4_950_000_000 }
            );
            assert_eq!(verified.outputs.len(), if work == 0 { 0 } else { 990 });
            assert_eq!(verified.remainder, 0);
        }
    }
    #[test]
    fn empty_verified_prior_is_preserved_and_can_be_repaid() {
        let (bundle, scan, spend) = owned_warmup(0, 123);
        let (bytes, trusted) = seal(&bundle);
        let verified =
            verify_received_owned(&bytes, &trusted, &received(&bundle.job), &scan, &spend).unwrap();
        assert_eq!(verified.entitlement.owed(), 123);
        assert_eq!(verified.remainder, 123);
        let (bundle, scan, spend) = owned_warmup(0, 500_000);
        let (bytes, trusted) = seal(&bundle);
        let verified =
            verify_received_owned(&bytes, &trusted, &received(&bundle.job), &scan, &spend).unwrap();
        assert_eq!(verified.entitlement.owed(), 500_000);
        assert_eq!(verified.outputs.len(), 1);
        assert_eq!(verified.outputs[0].amount, 500_000);
        assert_eq!(verified.remainder, 0);
    }
    #[test]
    fn empty_partial_and_full_windows_repay_prior_without_a_gross_pool_floor() {
        for work in [0, 1, 2] {
            let (bundle, scan, spend) = owned_warmup(work, 1_000_123);
            let (bytes, trusted) = seal(&bundle);
            let verified =
                verify_received_owned(&bytes, &trusted, &received(&bundle.job), &scan, &spend)
                    .unwrap();
            let paid = verified.outputs.iter().map(|o| o.amount).sum::<u64>();
            assert_eq!(paid, if work == 0 { 1_000_000 } else { 4_951_000_000 });
            assert_eq!(verified.remainder, 123);
            assert_eq!(verified.entitlement.owed(), paid + 123);
            assert_eq!(i128::from(verified.remainder) - 1_000_123, -1_000_000);
            assert_eq!(
                job_coinbase(&bundle.job).unwrap().output[0].value.to_sat(),
                5_000_000_000 - paid
            );
            assert_eq!(
                verified.entitlement.window(),
                WindowFacts::from_work(2, work as u128)
            );
        }
    }
    #[test]
    fn truly_empty_zero_prior_still_requires_the_pool_only_coinbase() {
        let (mut bundle, scan, spend) = owned_warmup(0, 0);
        let mut cb = job_coinbase(&bundle.job).unwrap();
        cb.output[0].value = bitcoin::Amount::from_sat(4_999_500_000);
        let mut extra = cb.output[0].clone();
        extra.value = bitcoin::Amount::from_sat(500_000);
        cb.output.push(extra);
        bundle.job.coinbase_prefix = serialize(&cb);
        let (bytes, trusted) = seal(&bundle);
        assert!(matches!(
            verify_received_owned(&bytes, &trusted, &received(&bundle.job), &scan, &spend),
            Err(Error::Invalid("empty-window routing/revenue conservation"))
        ));
    }
    #[test]
    fn empty_and_partial_unknown_history_cannot_use_warmup_admission() {
        for work in [0, 1] {
            let (bundle, scan, spend) = owned_warmup(work, 0);
            for case in 0..6 {
                let mut changed = bundle.clone();
                match case {
                    0 => changed.source_complete = false,
                    1 => changed.window = None,
                    2 => changed.window.as_mut().unwrap().eligible_history_complete = false,
                    3 => changed.high_water.clear(),
                    4 => {
                        changed.prior.watermark = 0;
                        changed.prior.digest = history_digest(0, &changed.prior.pending).unwrap();
                    }
                    _ => {
                        changed.high_water.insert(1, (1u64 << 56) | 1);
                    }
                }
                let (bytes, trusted) = seal(&changed);
                assert!(
                    verify_received_owned(&bytes, &trusted, &received(&changed.job), &scan, &spend)
                        .is_err(),
                    "work={work}, case={case}"
                );
            }
        }
        let (bundle, id) = fixture();
        let mut omitted = bundle.clone();
        omitted.slices[0].shares.pop();
        let (bytes, trusted) = seal(&omitted);
        assert!(verify(&bytes, &trusted, id, &bundle.job).is_err());
        let (old_bytes, old_trusted) = seal(&bundle);
        assert!(verify(&old_bytes, &old_trusted, id, &bundle.job).is_ok());
        let bytes = serde_json::to_vec(&omitted).unwrap();
        assert!(verify(&bytes, &old_trusted, id, &bundle.job).is_err());
    }
    #[test]
    fn warmup_rejects_false_q_sum_shortfall_owned_values_and_current_witness() {
        let (bundle, scan, spend) = owned_warmup(1, 0);
        for case in 0..6 {
            let mut changed = bundle.clone();
            match case {
                0 => changed.window_target = "1".into(),
                1 => changed.window.as_mut().unwrap().facts.actual_sum = 2,
                2 => changed.window.as_mut().unwrap().facts.shortfall = 0,
                3 => changed.window.as_mut().unwrap().facts.underfilled = false,
                4 => {
                    let mut cb = job_coinbase(&changed.job).unwrap();
                    cb.output[1].value = bitcoin::Amount::from_sat(cb.output[1].value.to_sat() - 1);
                    cb.output[0].value += bitcoin::Amount::from_sat(1);
                    changed.job.coinbase_prefix = serialize(&cb);
                }
                _ => {
                    let mut cb = job_coinbase(&changed.job).unwrap();
                    cb.output.push(bitcoin::TxOut {
                        value: bitcoin::Amount::ZERO,
                        script_pubkey: bitcoin::ScriptBuf::from_bytes(
                            [vec![0x6a, 0x24, 0xaa, 0x21, 0xa9, 0xed], vec![0; 32]].concat(),
                        ),
                    });
                    changed.job.coinbase_prefix = serialize(&cb);
                }
            }
            let (bytes, trusted) = seal(&changed);
            assert!(
                verify_received_owned(&bytes, &trusted, &received(&changed.job), &scan, &spend)
                    .is_err(),
                "case={case}"
            );
        }
    }
    #[test]
    fn partially_filled_witness_job_requires_and_verifies_its_real_reserved_value() {
        let (mut bundle, scan, spend) = owned_warmup(1, 0);
        let reserved = [0x33; 32];
        let mut preimage = vec![0; 32];
        preimage.extend(reserved);
        let commitment = bitcoin::hashes::sha256d::Hash::hash(&preimage).to_byte_array();
        let mut script = vec![0x6a, 0x24, 0xaa, 0x21, 0xa9, 0xed];
        script.extend(commitment);
        let mut cb = job_coinbase(&bundle.job).unwrap();
        cb.output.push(bitcoin::TxOut {
            value: bitcoin::Amount::ZERO,
            script_pubkey: bitcoin::ScriptBuf::from_bytes(script),
        });
        bundle.job.coinbase_prefix = serialize(&cb);
        bundle.witness_reserved.insert(bundle.job.job_id, reserved);
        let (bytes, trusted) = seal(&bundle);
        assert!(
            verify_received_owned(&bytes, &trusted, &received(&bundle.job), &scan, &spend).is_ok()
        );
        bundle
            .witness_reserved
            .insert(bundle.job.job_id, [0x34; 32]);
        let (bytes, trusted) = seal(&bundle);
        assert!(matches!(
            verify_received_owned(&bytes, &trusted, &received(&bundle.job), &scan, &spend),
            Err(Error::Invalid("job witness commitment"))
        ));
    }
    #[test]
    fn audited_empty_does_not_allow_a_nonce_to_mutate_non_script_sig_fields() {
        let (mut bundle, scan, spend) = owned_warmup(0, 0);
        let raw = job_coinbase(&bundle.job).unwrap();
        let raw = serialize(&raw);
        let offset = raw.len() - 4;
        bundle.job.coinbase_prefix = raw[..offset].to_vec();
        bundle.job.coinbase_suffix = raw[offset + 1..].to_vec();
        bundle.job.extranonce_len = 1;
        let (bytes, trusted) = seal(&bundle);
        assert!(matches!(
            verify_received_owned(&bytes, &trusted, &received(&bundle.job), &scan, &spend),
            Err(Error::Invalid(
                "extranonce outside immutable coinbase scriptSig suffix"
            ))
        ));
    }
    #[test]
    fn complete_raw_share_proof_recomputes_full_owed_and_rejects_mutations() {
        let (bundle, id) = fixture();
        let (bytes, trusted) = seal(&bundle);
        assert_eq!(
            verify(&bytes, &trusted, id, &bundle.job).unwrap().owed(),
            4_950_000_000
        );
        let mut changed = bundle.clone();
        changed.owed.insert(hex::encode(id), 4_950_000_001);
        let (bytes, root) = seal(&changed);
        assert!(verify(&bytes, &root, id, &bundle.job).is_err());
        changed = bundle.clone();
        changed.window_target = "1".into();
        let (bytes, root) = seal(&changed);
        assert!(verify(&bytes, &root, id, &bundle.job).is_err());
        changed = bundle.clone();
        changed.prior.pending.insert(hex::encode(id), 1);
        let (bytes, root) = seal(&changed);
        assert!(verify(&bytes, &root, id, &bundle.job).is_err());
        changed = bundle.clone();
        changed.jobs[0].coinbase_prefix[0] ^= 1;
        let (bytes, root) = seal(&changed);
        assert!(verify(&bytes, &root, id, &bundle.job).is_err());
        changed = bundle.clone();
        changed.slices[0].shares[0].identity = [2; 32];
        let tampered = serde_json::to_vec(&changed).unwrap();
        assert!(
            verify(&tampered, &trusted, id, &bundle.job).is_err(),
            "attribution changes cannot replace externally captured source seal"
        );
        changed = bundle.clone();
        changed.source_complete = false;
        let (bytes, root) = seal(&changed);
        assert!(matches!(
            verify(&bytes, &root, id, &bundle.job),
            Err(Error::Unknown(_))
        ));
    }
}
