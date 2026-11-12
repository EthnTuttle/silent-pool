//! Byte-level Redis stream payloads (docs/WIRE-FORMATS.md §3).
//!
//! Every entry has exactly one field, `d`, whose value is the encoding produced
//! here. Decoders reject unknown discriminants and trailing bytes.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::codec::{CodecError, Reader, Result, Writer};
use crate::models::{
    ArkDeliveryRow, ArkEpochMailboxRow, EpochFallback, MailboxMode, SpIdentityRow,
};
use spc_accounting::work_unit::{check_version_rule_compatibility, WindowRule, WorkUnitVersion};

/// The rail a distribution entry was built for (SPEC §4.7).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Rail {
    Coinbase = 0,
    Ark = 1,
    Pending = 2,
    Forfeit = 3,
}

impl Rail {
    pub fn from_u8(v: u8) -> Result<Self> {
        match v {
            0 => Ok(Self::Coinbase),
            1 => Ok(Self::Ark),
            2 => Ok(Self::Pending),
            3 => Ok(Self::Forfeit),
            other => Err(CodecError::Discriminant(other)),
        }
    }

    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

/// One distribution entry (SPEC §4.7).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "EntryJson", into = "EntryJson")]
pub struct Entry {
    pub sp_id: [u8; 32],
    pub owed_sats: u64,
    pub rail: Rail,
    /// Keys in k order; empty when the id was not kept (entry format 2).
    pub p_x: Vec<[u8; 32]>,
    /// Ark destination (policy key + delivery); empty when not Ark-capable.
    pub dest_id: Vec<u8>,
    /// `pay(id)` for a kept id, `owed(id)` otherwise.
    pub sats: u64,
}

/// SQL/JSON carries its own mandatory version, independently of the binary
/// event envelope. Never reinterpret legacy exact-k0 snapshots as ladder rows.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EntryJson {
    format_version: u8,
    sp_id: [u8; 32],
    owed_sats: u64,
    rail: Rail,
    p_x: Vec<[u8; 32]>,
    dest_id: Vec<u8>,
    sats: u64,
}

impl From<Entry> for EntryJson {
    fn from(e: Entry) -> Self {
        Self {
            format_version: 2,
            sp_id: e.sp_id,
            owed_sats: e.owed_sats,
            rail: e.rail,
            p_x: e.p_x,
            dest_id: e.dest_id,
            sats: e.sats,
        }
    }
}

impl TryFrom<EntryJson> for Entry {
    type Error = String;
    fn try_from(e: EntryJson) -> std::result::Result<Self, String> {
        if e.format_version != 2 {
            return Err("unsupported distribution entry format".into());
        }
        let entry = Self {
            sp_id: e.sp_id,
            owed_sats: e.owed_sats,
            rail: e.rail,
            p_x: e.p_x,
            dest_id: e.dest_id,
            sats: e.sats,
        };
        entry.validate().map_err(|e| e.to_string())?;
        Ok(entry)
    }
}

impl Entry {
    fn encode(&self, w: &mut Writer) -> Result<()> {
        self.validate()?;
        w.bytes(&self.sp_id).u8(self.rail.as_u8());
        w.u16(self.p_x.len() as u16);
        for key in &self.p_x {
            w.bytes(key);
        }
        w.blob_u16(&self.dest_id)?;
        w.u64(self.sats);
        w.u64(self.owed_sats);
        Ok(())
    }

    fn decode(r: &mut Reader<'_>) -> Result<Self> {
        let sp_id = r.array()?;
        let rail = Rail::from_u8(r.u8()?)?;
        let count = usize::from(r.u16()?);
        if count > spc_accounting::ladder::MAX_LADDER_OUTPUTS {
            return Err(CodecError::Range("payout key count"));
        }
        let mut p_x = Vec::with_capacity(count);
        for _ in 0..count {
            p_x.push(r.array()?);
        }
        let entry = Self {
            sp_id,
            rail,
            p_x,
            dest_id: r.blob_u16()?.to_vec(),
            sats: r.u64()?,
            owed_sats: r.u64()?,
        };
        entry.validate()?;
        Ok(entry)
    }

    pub fn validate(&self) -> Result<()> {
        if self.p_x.len() > spc_accounting::ladder::MAX_LADDER_OUTPUTS
            || (self.rail == Rail::Coinbase) == self.p_x.is_empty()
            || self
                .p_x
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != self.p_x.len()
        {
            return Err(CodecError::Range("payout keys"));
        }
        for key in &self.p_x {
            bitcoin::secp256k1::XOnlyPublicKey::from_slice(key)
                .map_err(|_| CodecError::Range("payout key point"))?;
        }
        Ok(())
    }
}

/// `spc:blocks` payload (WIRE-FORMATS §3.1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockEvent {
    /// None is explicit historical SPCB/2 provenance, never a zero balance.
    pub economic: Option<spc_accounting::economic::FrozenJob>,
    pub block_hash: [u8; 32],
    pub height: u32,
    pub front_id: u8,
    pub fingerprint: [u8; 32],
    /// Public x-only key of the height's `a_send`; the secret is never stored.
    pub a_send_x: [u8; 32],
    pub high_water: BTreeMap<u8, String>,
    /// The exact window target (`N x d`, SPEC §4.3) this distribution was
    /// built against; `payout` freezes it in `distribution_snapshot` so
    /// settlement reconstructs the same window even across a retarget.
    pub window_target: u128,
    pub entries: Vec<Entry>,
    pub work_unit_version: WorkUnitVersion,
    pub work_unit_fingerprint: Option<[u8; 32]>,
    pub window_rule: WindowRule,
}

impl BlockEvent {
    /// SPc-side snapshot identity: coinbase fingerprints cannot distinguish
    /// registration-only or frozen-window-only generations.
    pub fn snapshot_id(&self) -> Result<[u8; 32]> {
        use bitcoin::hashes::{sha256, Hash, HashEngine};
        let mut engine = sha256::Hash::engine();
        engine.input(b"SPc/DistributionSnapshotV2");
        engine.input(&self.encode()?);
        Ok(sha256::Hash::from_engine(engine).to_byte_array())
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        validate_entries(&self.entries)?;
        let mut w = Writer::new();
        w.bytes(b"SPCB")
            .u8(if self.economic.is_some() { 3 } else { 2 });
        w.bytes(&self.block_hash);
        w.u32(self.height);
        w.u8(self.front_id);
        w.bytes(&self.fingerprint).bytes(&self.a_send_x);
        write_high_water(&mut w, &self.high_water)?;
        w.u128(self.window_target);
        w.u16(u16::try_from(self.entries.len()).map_err(|_| CodecError::Range("entries"))?);
        for e in &self.entries {
            e.encode(&mut w)?;
        }
        write_work_unit(&mut w, self.work_unit_version, self.work_unit_fingerprint)?;
        w.u8(self.window_rule.as_u8());
        if let Some(job) = &self.economic {
            let prior = job
                .prior()
                .map_err(|_| CodecError::Range("frozen economic job"))?;
            if prior.len() != self.entries.len()
                || self.entries.iter().any(|e| !prior.contains_key(&e.sp_id))
            {
                return Err(CodecError::Range("frozen prior coverage"));
            }
            let bytes =
                serde_json::to_vec(job).map_err(|_| CodecError::Range("economic encoding"))?;
            if bytes.len() > 4 * 1024 * 1024 {
                return Err(CodecError::Range("economic frame bound"));
            }
            w.u32(bytes.len() as u32).bytes(&bytes);
        }
        Ok(w.finish())
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut r = Reader::new(bytes);
        if r.array::<4>()? != *b"SPCB" {
            return Err(CodecError::Range("block event format"));
        }
        let format = r.u8()?;
        if format != 2 && format != 3 {
            return Err(CodecError::Range("block event format"));
        }
        let block_hash = r.array()?;
        let height = r.u32()?;
        let front_id = r.u8()?;
        let fingerprint = r.array()?;
        let a_send_x = r.array()?;
        let high_water = read_high_water(&mut r)?;
        let window_target = r.u128()?;
        let n = r.u16()? as usize;
        let mut entries = Vec::with_capacity(n.min(4096));
        let mut key_count = 0usize;
        for _ in 0..n {
            let entry = Entry::decode(&mut r)?;
            key_count += entry.p_x.len();
            if key_count > spc_accounting::ladder::MAX_LADDER_OUTPUTS {
                return Err(CodecError::Range("block payout key count"));
            }
            entries.push(entry);
        }
        validate_entries(&entries)?;
        let (work_unit_version, work_unit_fingerprint) = read_work_unit(&mut r)?;
        let window_rule =
            WindowRule::from_u8(r.u8()?).map_err(|_| CodecError::Discriminant(0xff))?;
        check_version_rule_compatibility(work_unit_version, window_rule)
            .map_err(|_| CodecError::Range("work unit/window rule"))?;
        let economic = if format == 3 {
            let bytes = r.blob_u32()?;
            if bytes.len() > 4 * 1024 * 1024 {
                return Err(CodecError::Range("economic frame bound"));
            }
            let job: spc_accounting::economic::FrozenJob =
                serde_json::from_slice(bytes).map_err(|_| CodecError::Range("economic job"))?;
            let prior = job
                .prior()
                .map_err(|_| CodecError::Range("economic prior"))?;
            if prior.len() != entries.len() || entries.iter().any(|e| !prior.contains_key(&e.sp_id))
            {
                return Err(CodecError::Range("economic prior coverage"));
            }
            Some(job)
        } else {
            None
        };
        r.finish()?;
        Ok(Self {
            economic,
            block_hash,
            height,
            front_id,
            fingerprint,
            a_send_x,
            high_water,
            window_target,
            entries,
            work_unit_version,
            work_unit_fingerprint,
            window_rule,
        })
    }
}

fn validate_entries(entries: &[Entry]) -> Result<()> {
    let mut ids = std::collections::BTreeSet::new();
    let mut keys = std::collections::BTreeSet::new();
    for entry in entries {
        entry.validate()?;
        if !ids.insert(entry.sp_id) {
            return Err(CodecError::Range("duplicate entry identity"));
        }
        for key in &entry.p_x {
            if !keys.insert(*key) {
                return Err(CodecError::Range("duplicate payout key"));
            }
        }
    }
    if keys.len() > spc_accounting::ladder::MAX_LADDER_OUTPUTS {
        return Err(CodecError::Range("block payout key count"));
    }
    Ok(())
}

fn write_work_unit(
    w: &mut Writer,
    version: WorkUnitVersion,
    fingerprint: Option<[u8; 32]>,
) -> Result<()> {
    if version == WorkUnitVersion::V2 && fingerprint == Some([0u8; 32]) {
        return Err(CodecError::Range("v2 zero work unit fingerprint"));
    }
    w.u8(version.as_u8());
    w.blob_u8(&fingerprint.unwrap_or([0u8; 32]))?;
    Ok(())
}

fn read_work_unit(r: &mut Reader<'_>) -> Result<(WorkUnitVersion, Option<[u8; 32]>)> {
    let code = r.u8()?;
    let version = WorkUnitVersion::from_u8(code).map_err(|_| CodecError::Discriminant(code))?;
    let bytes = r.blob_u8()?;
    if bytes.len() != 32 {
        return Err(CodecError::Range("work unit fingerprint length"));
    }
    let mut fingerprint = [0u8; 32];
    fingerprint.copy_from_slice(bytes);
    match version {
        WorkUnitVersion::V1 if fingerprint == [0u8; 32] => Ok((version, None)),
        WorkUnitVersion::V2 if fingerprint != [0u8; 32] => Ok((version, Some(fingerprint))),
        WorkUnitVersion::V1 => Err(CodecError::Range("v1 fingerprint must be zero sentinel")),
        WorkUnitVersion::V2 => Err(CodecError::Range("v2 zero work unit fingerprint")),
    }
}

fn write_high_water(w: &mut Writer, hw: &BTreeMap<u8, String>) -> Result<()> {
    w.u16(u16::try_from(hw.len()).map_err(|_| CodecError::Range("high_water"))?);
    for (front, id) in hw {
        w.u8(*front);
        w.blob_u8(id.as_bytes())?;
    }
    Ok(())
}

fn read_high_water(r: &mut Reader<'_>) -> Result<BTreeMap<u8, String>> {
    let n = r.u16()? as usize;
    let mut out = BTreeMap::new();
    for _ in 0..n {
        let front = r.u8()?;
        let id = std::str::from_utf8(r.blob_u8()?)
            .map_err(|_| CodecError::Range("high_water utf8"))?
            .to_string();
        out.insert(front, id);
    }
    Ok(out)
}

fn nonempty(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.is_empty() {
        None
    } else {
        Some(bytes.to_vec())
    }
}

/// `spc:front:events` payloads (WIRE-FORMATS §3.2). Every payload starts with
/// a type byte and a payout-assigned revision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FrontEvent {
    EconomicChanged {
        rev: u64,
        economic_revision: u64,
        anchor_hash: [u8; 32],
    },
    ArkRegApplied {
        rev: u64,
        sp_id: [u8; 32],
        request_id: u32,
        registration_rev: u64,
        state: spc_ext::RegistrationSuccessState,
        identity: SpIdentityRow,
        delivery: ArkDeliveryRow,
        epoch_mailboxes: Vec<ArkEpochMailboxRow>,
    },
    LedgerApplied {
        rev: u64,
        block_hash: [u8; 32],
        republish: bool,
        pending_deltas: Vec<([u8; 32], i64)>,
    },
    BlockFound {
        rev: u64,
        block_hash: [u8; 32],
        height: u32,
        front_id: u8,
        provisional_paids: Vec<([u8; 32], u64)>,
        high_water: BTreeMap<u8, String>,
    },
    TipHeartbeat {
        rev: u64,
        tip_hash: [u8; 32],
        height: u32,
        unix_ms: u64,
    },
}

impl FrontEvent {
    pub fn rev(&self) -> u64 {
        match self {
            Self::ArkRegApplied { rev, .. }
            | Self::EconomicChanged { rev, .. }
            | Self::LedgerApplied { rev, .. }
            | Self::BlockFound { rev, .. }
            | Self::TipHeartbeat { rev, .. } => *rev,
        }
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut w = Writer::new();
        match self {
            Self::EconomicChanged {
                rev,
                economic_revision,
                anchor_hash,
            } => {
                w.u8(0x05)
                    .u64(*rev)
                    .u64(*economic_revision)
                    .bytes(anchor_hash);
            }
            Self::ArkRegApplied {
                rev,
                sp_id,
                request_id,
                registration_rev,
                state,
                identity,
                delivery,
                epoch_mailboxes,
            } => {
                w.u8(0x01)
                    .u64(*rev)
                    .bytes(sp_id)
                    .u32(*request_id)
                    .u64(*registration_rev)
                    .u8(*state as u8);
                w.bytes(&identity.b_scan)
                    .bytes(&identity.b_spend)
                    .i64(i64::from(identity.network));
                w.bytes(&delivery.ark_id).bytes(&delivery.b_ark);
                let mode = match delivery.mailbox_mode {
                    MailboxMode::None => 0,
                    MailboxMode::Static => 1,
                    MailboxMode::EpochDerived => 2,
                    MailboxMode::EpochRegistered => 3,
                };
                w.u8(mode);
                w.blob_u8(delivery.mailbox_m.as_deref().unwrap_or_default())?;
                w.blob_u8(delivery.mailbox_d.as_deref().unwrap_or_default())?;
                w.u8(delivery.epoch_fallback as u8);
                w.u16(
                    u16::try_from(epoch_mailboxes.len())
                        .map_err(|_| CodecError::Range("epoch_mailboxes"))?,
                );
                for entry in epoch_mailboxes {
                    w.u32(entry.height).bytes(&entry.mailbox_mi);
                }
            }
            Self::LedgerApplied {
                rev,
                block_hash,
                republish,
                pending_deltas,
            } => {
                w.u8(0x02)
                    .u64(*rev)
                    .bytes(block_hash)
                    .u8(u8::from(*republish));
                w.u16(
                    u16::try_from(pending_deltas.len())
                        .map_err(|_| CodecError::Range("pending_deltas"))?,
                );
                for (id, delta) in pending_deltas {
                    w.bytes(id).i64(*delta);
                }
            }
            Self::BlockFound {
                rev,
                block_hash,
                height,
                front_id,
                provisional_paids,
                high_water,
            } => {
                w.u8(0x03)
                    .u64(*rev)
                    .bytes(block_hash)
                    .u32(*height)
                    .u8(*front_id);
                w.u16(
                    u16::try_from(provisional_paids.len())
                        .map_err(|_| CodecError::Range("provisional_paids"))?,
                );
                for (id, sats) in provisional_paids {
                    w.bytes(id).u64(*sats);
                }
                write_high_water(&mut w, high_water)?;
            }
            Self::TipHeartbeat {
                rev,
                tip_hash,
                height,
                unix_ms,
            } => {
                w.u8(0x04)
                    .u64(*rev)
                    .bytes(tip_hash)
                    .u32(*height)
                    .u64(*unix_ms);
            }
        }
        Ok(w.finish())
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut r = Reader::new(bytes);
        let ty = r.u8()?;
        let rev = r.u64()?;
        let event = match ty {
            0x05 => Self::EconomicChanged {
                rev,
                economic_revision: r.u64()?,
                anchor_hash: r.array()?,
            },
            0x01 => {
                let sp_id = r.array()?;
                let request_id = r.u32()?;
                let registration_rev = r.u64()?;
                let state = match r.u8()? {
                    0 => spc_ext::RegistrationSuccessState::Applied,
                    1 => spc_ext::RegistrationSuccessState::AlreadyApplied,
                    2 => spc_ext::RegistrationSuccessState::Superseded,
                    other => return Err(CodecError::Discriminant(other)),
                };
                let b_scan = r.array::<33>()?;
                let b_spend = r.array::<33>()?;
                let network =
                    i16::try_from(r.i64()?).map_err(|_| CodecError::Range("identity network"))?;
                let ark_id = r.array()?;
                let b_ark = r.array::<33>()?.to_vec();
                let mailbox_mode = match r.u8()? {
                    0 => MailboxMode::None,
                    1 => MailboxMode::Static,
                    2 => MailboxMode::EpochDerived,
                    3 => MailboxMode::EpochRegistered,
                    other => return Err(CodecError::Discriminant(other)),
                };
                let mailbox_m = nonempty(r.blob_u8()?);
                let mailbox_d = nonempty(r.blob_u8()?);
                let epoch_fallback = match r.u8()? {
                    0 => EpochFallback::Static,
                    1 => EpochFallback::Hold,
                    other => return Err(CodecError::Discriminant(other)),
                };
                let n = r.u16()? as usize;
                let mut epoch_mailboxes = Vec::with_capacity(n.min(4096));
                for _ in 0..n {
                    epoch_mailboxes.push(ArkEpochMailboxRow {
                        sp_id,
                        height: r.u32()?,
                        mailbox_mi: r.array()?,
                    });
                }
                Self::ArkRegApplied {
                    rev,
                    sp_id,
                    request_id,
                    registration_rev,
                    state,
                    identity: SpIdentityRow {
                        sp_id,
                        b_scan: b_scan.to_vec(),
                        b_spend: b_spend.to_vec(),
                        network,
                    },
                    delivery: ArkDeliveryRow {
                        sp_id,
                        ark_id,
                        b_ark,
                        mailbox_mode,
                        mailbox_m,
                        mailbox_d,
                        epoch_fallback,
                    },
                    epoch_mailboxes,
                }
            }
            0x02 => {
                let block_hash = r.array()?;
                let republish = r.u8()? != 0;
                let n = r.u16()? as usize;
                let mut pending_deltas = Vec::with_capacity(n.min(4096));
                for _ in 0..n {
                    pending_deltas.push((r.array()?, r.i64()?));
                }
                Self::LedgerApplied {
                    rev,
                    block_hash,
                    republish,
                    pending_deltas,
                }
            }
            0x03 => {
                let block_hash = r.array()?;
                let height = r.u32()?;
                let front_id = r.u8()?;
                let n = r.u16()? as usize;
                let mut provisional_paids = Vec::with_capacity(n.min(4096));
                for _ in 0..n {
                    provisional_paids.push((r.array()?, r.u64()?));
                }
                let high_water = read_high_water(&mut r)?;
                Self::BlockFound {
                    rev,
                    block_hash,
                    height,
                    front_id,
                    provisional_paids,
                    high_water,
                }
            }
            0x04 => Self::TipHeartbeat {
                rev,
                tip_hash: r.array()?,
                height: r.u32()?,
                unix_ms: r.u64()?,
            },
            other => return Err(CodecError::Discriminant(other)),
        };
        r.finish()?;
        Ok(event)
    }
}

/// `spc:arkreg` payload (WIRE-FORMATS §3.3), kind 0.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArkRegEvent {
    pub sp_id: [u8; 32],
    pub b_scan: [u8; 33],
    pub b_spend: [u8; 33],
    pub ark_id: [u8; 4],
    pub b_ark: [u8; 33],
    pub mailbox_mode: u8,
    pub mailbox_pubkey: Vec<u8>,
    pub mailbox_dh: Vec<u8>,
    pub epoch_fallback: u8,
    pub sig: [u8; 64],
    pub request_id: u32,
    /// Fresh front attempt nonce, cache-only and not covered by sig/digest.
    pub attempt: [u8; 16],
}

impl ArkRegEvent {
    pub const KIND: u8 = 0;

    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut w = Writer::new();
        w.u8(Self::KIND)
            .bytes(&self.sp_id)
            .bytes(&self.b_scan)
            .bytes(&self.b_spend);
        w.bytes(&self.ark_id)
            .bytes(&self.b_ark)
            .u8(self.mailbox_mode);
        w.blob_u8(&self.mailbox_pubkey)?;
        w.blob_u8(&self.mailbox_dh)?;
        w.u8(self.epoch_fallback)
            .bytes(&self.sig)
            .u32(self.request_id)
            .bytes(&self.attempt);
        Ok(w.finish())
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut r = Reader::new(bytes);
        let kind = r.u8()?;
        if kind != Self::KIND {
            return Err(CodecError::Discriminant(kind));
        }
        let out = Self {
            sp_id: r.array()?,
            b_scan: r.array()?,
            b_spend: r.array()?,
            ark_id: r.array()?,
            b_ark: r.array()?,
            mailbox_mode: r.u8()?,
            mailbox_pubkey: r.blob_u8()?.to_vec(),
            mailbox_dh: r.blob_u8()?.to_vec(),
            epoch_fallback: r.u8()?,
            sig: r.array()?,
            request_id: r.u32()?,
            attempt: r.array()?,
        };
        r.finish()?;
        Ok(out)
    }
}

/// `spc:arkreg` payload kind 1: registered variant-A epoch mailbox keys.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArkEpochRegEvent {
    pub sp_id: [u8; 32],
    pub entries: Vec<[u8; 37]>,
    pub sig: [u8; 64],
    pub request_id: u32,
    pub attempt: [u8; 16],
}

impl ArkEpochRegEvent {
    pub const KIND: u8 = 1;

    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut w = Writer::new();
        w.u8(Self::KIND).bytes(&self.sp_id);
        w.u16(u16::try_from(self.entries.len()).map_err(|_| CodecError::Range("entries"))?);
        for entry in &self.entries {
            w.bytes(entry);
        }
        w.bytes(&self.sig).u32(self.request_id).bytes(&self.attempt);
        Ok(w.finish())
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut r = Reader::new(bytes);
        let kind = r.u8()?;
        if kind != Self::KIND {
            return Err(CodecError::Discriminant(kind));
        }
        let sp_id = r.array()?;
        let n = r.u16()? as usize;
        let mut entries = Vec::with_capacity(n.min(4096));
        for _ in 0..n {
            entries.push(r.array()?);
        }
        let sig = r.array()?;
        let request_id = r.u32()?;
        let attempt = r.array()?;
        r.finish()?;
        Ok(Self {
            sp_id,
            entries,
            sig,
            request_id,
            attempt,
        })
    }
}

/// `spc:slices` payload. WIRE-FORMATS §3.6 (added with this implementation):
/// a sealed slice header, its identity-free share blob, and per-identity
/// aggregates. The share blob is opaque here; `spc-accounting` owns its
/// contents.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceEvent {
    pub slice_id: u64,
    pub height: u32,
    pub prev_hash: [u8; 32],
    pub ref_job_id: u64,
    pub ref_fees: u64,
    pub n_shares: u32,
    pub difficulty: u128,
    pub root: [u8; 32],
    pub shares_blob: Vec<u8>,
    pub credits: Vec<SliceCredit>,
    pub work_unit_version: WorkUnitVersion,
    pub work_unit_fingerprint: Option<[u8; 32]>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceCredit {
    pub sp_id: [u8; 32],
    pub diff_sum: u128,
    pub diff_fee_sum: u128,
}

impl SliceEvent {
    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut w = Writer::new();
        w.u64(self.slice_id).u32(self.height).bytes(&self.prev_hash);
        w.u64(self.ref_job_id).u64(self.ref_fees).u32(self.n_shares);
        w.u128(self.difficulty).bytes(&self.root);
        w.blob_u32(&self.shares_blob)?;
        w.u16(u16::try_from(self.credits.len()).map_err(|_| CodecError::Range("credits"))?);
        for c in &self.credits {
            w.bytes(&c.sp_id).u128(c.diff_sum).u128(c.diff_fee_sum);
        }
        write_work_unit(&mut w, self.work_unit_version, self.work_unit_fingerprint)?;
        Ok(w.finish())
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut r = Reader::new(bytes);
        let slice_id = r.u64()?;
        let height = r.u32()?;
        let prev_hash = r.array()?;
        let ref_job_id = r.u64()?;
        let ref_fees = r.u64()?;
        let n_shares = r.u32()?;
        let difficulty = r.u128()?;
        let root = r.array()?;
        let shares_blob = r.blob_u32()?.to_vec();
        let n = r.u16()? as usize;
        let mut credits = Vec::with_capacity(n.min(4096));
        for _ in 0..n {
            credits.push(SliceCredit {
                sp_id: r.array()?,
                diff_sum: r.u128()?,
                diff_fee_sum: r.u128()?,
            });
        }
        let (work_unit_version, work_unit_fingerprint) = read_work_unit(&mut r)?;
        r.finish()?;
        Ok(Self {
            slice_id,
            height,
            prev_hash,
            ref_job_id,
            ref_fees,
            n_shares,
            difficulty,
            root,
            shares_blob,
            credits,
            work_unit_version,
            work_unit_fingerprint,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: u8) -> Entry {
        use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
        let key = PublicKey::from_secret_key(
            &Secp256k1::new(),
            &SecretKey::from_slice(&[id; 32]).unwrap(),
        )
        .x_only_public_key()
        .0
        .serialize();
        Entry {
            sp_id: [id; 32],
            owed_sats: 1000 + id as u64,
            rail: Rail::Coinbase,
            p_x: vec![key],
            dest_id: vec![],
            sats: 1000 + id as u64,
        }
    }

    #[test]
    fn block_event_round_trip() {
        let e = BlockEvent {
            economic: None,
            block_hash: [1; 32],
            height: 800_000,
            front_id: 3,
            fingerprint: [2; 32],
            a_send_x: [3; 32],
            high_water: BTreeMap::from([(0u8, "1-1".to_string()), (3, "9-2".to_string())]),
            // u128::MAX: N x d can exceed i64 and must survive the wire.
            window_target: u128::MAX,
            entries: vec![entry(1), entry(2)],
            work_unit_version: WorkUnitVersion::V1,
            work_unit_fingerprint: None,
            window_rule: WindowRule::ShareWorkLegacy,
        };
        let bytes = e.encode().unwrap();
        assert_eq!(BlockEvent::decode(&bytes).unwrap(), e);
        let mut bad = bytes.clone();
        bad.push(0);
        assert_eq!(BlockEvent::decode(&bad), Err(CodecError::Trailing));
        assert_eq!(&bytes[..5], b"SPCB\x02");
        assert!(BlockEvent::decode(&bytes[5..]).is_err());
        let mut changed = e.clone();
        changed.entries[0].dest_id = vec![7];
        assert_eq!(changed.fingerprint, e.fingerprint);
        assert_ne!(changed.snapshot_id().unwrap(), e.snapshot_id().unwrap());
        changed.entries[0].owed_sats += 1;
        assert_ne!(changed.snapshot_id().unwrap(), e.snapshot_id().unwrap());
        let mut duplicate = e.clone();
        let key = duplicate.entries[0].p_x[0];
        duplicate.entries[0].p_x.push(key);
        assert!(duplicate.encode().is_err());
        let mut duplicate = e.clone();
        duplicate.entries.push(duplicate.entries[0].clone());
        assert!(duplicate.encode().is_err());
        let mut json = serde_json::to_value(&e.entries[0]).unwrap();
        assert_eq!(json["format_version"], 2);
        json.as_object_mut().unwrap().remove("format_version");
        assert!(serde_json::from_value::<Entry>(json).is_err());
    }

    #[test]
    fn front_events_round_trip() {
        let events = vec![
            FrontEvent::ArkRegApplied {
                rev: 1,
                sp_id: [1; 32],
                request_id: 1,
                registration_rev: 1,
                state: spc_ext::RegistrationSuccessState::Applied,
                identity: SpIdentityRow {
                    sp_id: [1; 32],
                    b_scan: hex::decode(
                        "024b19f0da60684ff1b6ee04c39c753a823c4bc758872931bdbef4ca14ff4b9c50",
                    )
                    .unwrap(),
                    b_spend: hex::decode(
                        "032cd165ae1dadd742f410eb06c56467feef432b218a14b216bada9573f9176907",
                    )
                    .unwrap(),
                    network: 3,
                },
                delivery: ArkDeliveryRow {
                    sp_id: [1; 32],
                    ark_id: [1, 2, 3, 4],
                    b_ark: hex::decode(
                        "02c6accb73e60944acb90d0eaade2d6ddb56cf91449cd11dcd42771fabdf222b18",
                    )
                    .unwrap(),
                    mailbox_mode: MailboxMode::EpochDerived,
                    mailbox_m: Some(
                        hex::decode(
                            "0235739eb757168f51be9a504da5b5cda61c37d95133ac677a76605d6971c5274f",
                        )
                        .unwrap(),
                    ),
                    mailbox_d: Some(
                        hex::decode(
                            "02d864313e3db31b98210d966a3a02589696d63491921b7a119bd6943be353820a",
                        )
                        .unwrap(),
                    ),
                    epoch_fallback: EpochFallback::Hold,
                },
                epoch_mailboxes: vec![ArkEpochMailboxRow {
                    sp_id: [1; 32],
                    height: 800_000,
                    mailbox_mi: hex::decode(
                        "0235739eb757168f51be9a504da5b5cda61c37d95133ac677a76605d6971c5274f",
                    )
                    .unwrap()
                    .try_into()
                    .unwrap(),
                }],
            },
            FrontEvent::LedgerApplied {
                rev: 2,
                block_hash: [2; 32],
                republish: true,
                pending_deltas: vec![([3; 32], -5), ([4; 32], 9)],
            },
            FrontEvent::BlockFound {
                rev: 3,
                block_hash: [5; 32],
                height: 10,
                front_id: 1,
                provisional_paids: vec![([6; 32], 7)],
                high_water: BTreeMap::from([(1u8, "3-4".to_string())]),
            },
            FrontEvent::TipHeartbeat {
                rev: 4,
                tip_hash: [7; 32],
                height: 11,
                unix_ms: 12,
            },
        ];
        for e in events {
            let bytes = e.encode().unwrap();
            assert_eq!(FrontEvent::decode(&bytes).unwrap(), e);
        }
    }

    #[test]
    fn v2_metadata_round_trips_and_old_or_ambiguous_events_fail() {
        let fp = [0x55; 32];
        let block = BlockEvent {
            economic: None,
            block_hash: [1; 32],
            height: 800_000,
            front_id: 3,
            fingerprint: [2; 32],
            a_send_x: [3; 32],
            high_water: BTreeMap::from([(3u8, "9-2".to_string())]),
            window_target: 65,
            entries: vec![entry(1)],
            work_unit_version: WorkUnitVersion::V2,
            work_unit_fingerprint: Some(fp),
            window_rule: WindowRule::NetworkBlockWork,
        };
        let block_bytes = block.encode().unwrap();
        assert_eq!(BlockEvent::decode(&block_bytes).unwrap(), block);
        assert!(matches!(
            BlockEvent::decode(&block_bytes[..block_bytes.len() - 35]),
            Err(CodecError::Eof)
        ));
        let mut zero = block_bytes.clone();
        let version_at = zero.len() - 35;
        zero[version_at] = 2;
        zero[version_at + 2..version_at + 34].fill(0);
        assert_eq!(
            BlockEvent::decode(&zero),
            Err(CodecError::Range("v2 zero work unit fingerprint"))
        );

        let mut unknown = block_bytes.clone();
        let unknown_at = unknown.len() - 35;
        unknown[unknown_at] = 9;
        assert!(matches!(
            BlockEvent::decode(&unknown),
            Err(CodecError::Discriminant(9))
        ));

        let slice = SliceEvent {
            slice_id: 42,
            height: 800_000,
            prev_hash: [1; 32],
            ref_job_id: 7,
            ref_fees: 50_000,
            n_shares: 1,
            difficulty: 2,
            root: [2; 32],
            shares_blob: vec![3; 10],
            credits: vec![SliceCredit {
                sp_id: [4; 32],
                diff_sum: 2,
                diff_fee_sum: 4,
            }],
            work_unit_version: WorkUnitVersion::V2,
            work_unit_fingerprint: Some(fp),
        };
        let slice_bytes = slice.encode().unwrap();
        assert_eq!(SliceEvent::decode(&slice_bytes).unwrap(), slice);
        assert!(matches!(
            SliceEvent::decode(&slice_bytes[..slice_bytes.len() - 34]),
            Err(CodecError::Eof)
        ));
        let mut zero_slice = slice_bytes;
        let zero_slice_at = zero_slice.len() - 34;
        zero_slice[zero_slice_at] = 2;
        zero_slice[zero_slice_at + 2..zero_slice_at + 34].fill(0);
        assert_eq!(
            SliceEvent::decode(&zero_slice),
            Err(CodecError::Range("v2 zero work unit fingerprint"))
        );
    }

    #[test]
    fn arkreg_round_trip() {
        let e = ArkRegEvent {
            sp_id: [1; 32],
            b_scan: [2; 33],
            b_spend: [3; 33],
            ark_id: [4; 4],
            b_ark: [5; 33],
            mailbox_mode: 2,
            mailbox_pubkey: vec![6; 33],
            mailbox_dh: vec![7; 33],
            epoch_fallback: 0,
            sig: [8; 64],
            request_id: 1,
            attempt: [9; 16],
        };
        let bytes = e.encode().unwrap();
        assert_eq!(ArkRegEvent::decode(&bytes).unwrap(), e);
    }

    #[test]
    fn slice_event_round_trip() {
        let e = SliceEvent {
            slice_id: 42,
            height: 800_000,
            prev_hash: [1; 32],
            ref_job_id: 7,
            ref_fees: 50_000,
            n_shares: 10,
            difficulty: 12345678901234567890,
            root: [2; 32],
            shares_blob: vec![3; 100],
            credits: vec![SliceCredit {
                sp_id: [4; 32],
                diff_sum: 5,
                diff_fee_sum: 6,
            }],
            work_unit_version: WorkUnitVersion::V1,
            work_unit_fingerprint: None,
        };
        let bytes = e.encode().unwrap();
        assert_eq!(SliceEvent::decode(&bytes).unwrap(), e);
    }
}
