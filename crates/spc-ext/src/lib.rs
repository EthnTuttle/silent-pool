//! Project-defined SV2 extensions:
//! - `0x0003` Coinbase Transaction Payouts (PR #203 @ 47dbedd)
//! - `0x5350` delivery capabilities (SPEC §11.4)
//! - `spc:ark:orders` order signing (SPEC §11.3)
//!
//! All encodings are byte-exact and reject trailing bytes. Consensus-serialized
//! tx outputs are carried as `B0_64K` blobs, as PR #203 defines them.

#![deny(unsafe_code)]

pub mod authorization;

use secp256k1::{schnorr, Keypair, Message, PublicKey, Secp256k1, XOnlyPublicKey};
use thiserror::Error;

pub const EXT_PAYOUTS: u16 = 0x0003;
pub const EXT_DELIVERY: u16 = 0x5350;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum Error {
    #[error("unexpected end of input")]
    Eof,
    #[error("trailing bytes")]
    Trailing,
    #[error("length overflow")]
    Length,
    #[error("invalid TNF")]
    InvalidTnf,
    #[error("bad signature")]
    Signature,
    #[error("empty distribution weight")]
    Weight,
    #[error("fixed outputs exceed revenue")]
    Fixed,
    #[error("length mismatch")]
    LengthMismatch,
    #[error("invalid distribution")]
    InvalidDistribution,
    #[error("unsupported delivery version")]
    Version,
    #[error("unsupported mailbox mode")]
    UnsupportedMailboxMode,
    #[error("request id is zero or exhausted")]
    RequestIdExhausted,
    #[error("stale distribution")]
    StaleDistribution,
}

// ---------------------------------------------------------------------------
// Small codec
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Writer(Vec<u8>);

impl Writer {
    fn u8(&mut self, v: u8) -> &mut Self {
        self.0.push(v);
        self
    }
    fn u16(&mut self, v: u16) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn u32(&mut self, v: u32) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn u64(&mut self, v: u64) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn bytes(&mut self, b: &[u8]) -> &mut Self {
        self.0.extend_from_slice(b);
        self
    }
    fn blob_u8(&mut self, b: &[u8]) -> Result<&mut Self, Error> {
        let n = u8::try_from(b.len()).map_err(|_| Error::Length)?;
        self.u8(n).bytes(b);
        Ok(self)
    }
    fn blob_u16(&mut self, b: &[u8]) -> Result<&mut Self, Error> {
        let n = u16::try_from(b.len()).map_err(|_| Error::Length)?;
        self.u16(n).bytes(b);
        Ok(self)
    }
    fn finish(self) -> Vec<u8> {
        self.0
    }
}

struct Reader<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> Reader<'a> {
    fn new(b: &'a [u8]) -> Self {
        Self { b, p: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let end = self.p.checked_add(n).ok_or(Error::Length)?;
        let out = self.b.get(self.p..end).ok_or(Error::Eof)?;
        self.p = end;
        Ok(out)
    }
    fn u8(&mut self) -> Result<u8, Error> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, Error> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> Result<u32, Error> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn u64(&mut self) -> Result<u64, Error> {
        let b = self.take(8)?;
        Ok(u64::from_le_bytes(b.try_into().map_err(|_| Error::Length)?))
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], Error> {
        self.take(N)?.try_into().map_err(|_| Error::Length)
    }
    fn blob_u8(&mut self) -> Result<&'a [u8], Error> {
        let n = self.u8()? as usize;
        self.take(n)
    }
    fn blob_u16(&mut self) -> Result<&'a [u8], Error> {
        let n = self.u16()? as usize;
        self.take(n)
    }
    fn finish(self) -> Result<(), Error> {
        if self.p == self.b.len() {
            Ok(())
        } else {
            Err(Error::Trailing)
        }
    }
}

// ---------------------------------------------------------------------------
// 0x0003: consensus tx outputs
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TxOut {
    pub script: Vec<u8>,
    pub value: u64,
}

impl TxOut {
    pub fn to_consensus(&self) -> Result<Vec<u8>, Error> {
        let mut w = Writer::default();
        w.u64(self.value);
        write_compact(&mut w.0, self.script.len() as u64);
        w.bytes(&self.script);
        Ok(w.finish())
    }

    pub fn from_consensus(bytes: &[u8]) -> Result<Self, Error> {
        let mut r = Reader::new(bytes);
        let value = r.u64()?;
        let n = read_compact(&mut r)?;
        let script = r.take(n)?;
        r.finish()?;
        Ok(Self {
            script: script.to_vec(),
            value,
        })
    }
}

fn write_compact(out: &mut Vec<u8>, n: u64) {
    match n {
        0..=0xfc => out.push(n as u8),
        0xfd..=0xffff => {
            out.push(0xfd);
            out.extend_from_slice(&(n as u16).to_le_bytes());
        }
        0x1_0000..=0xffff_ffff => {
            out.push(0xfe);
            out.extend_from_slice(&(n as u32).to_le_bytes());
        }
        _ => {
            out.push(0xff);
            out.extend_from_slice(&n.to_le_bytes());
        }
    }
}

fn read_compact(r: &mut Reader<'_>) -> Result<usize, Error> {
    match r.u8()? {
        n @ 0..=0xfc => Ok(n as usize),
        0xfd => Ok(r.u16()? as usize),
        0xfe => Ok(r.u32()? as usize),
        0xff => usize::try_from(r.u64()?).map_err(|_| Error::Length),
    }
}

/// `SetPayoutDistribution` (PR #203 §3.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetPayoutDistribution {
    pub distribution_id: u64,
    pub pool_payout: TxOut,
    pub payouts: Vec<TxOut>,
    pub dust_limits: Vec<u32>,
    pub additional_outputs: Vec<TxOut>,
}

impl SetPayoutDistribution {
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        let mut w = Writer::default();
        w.u64(self.distribution_id);
        w.blob_u16(&self.pool_payout.to_consensus()?)?;
        w.u16(u16::try_from(self.payouts.len()).map_err(|_| Error::Length)?);
        for p in &self.payouts {
            w.blob_u16(&p.to_consensus()?)?;
        }
        w.u16(u16::try_from(self.dust_limits.len()).map_err(|_| Error::Length)?);
        for d in &self.dust_limits {
            w.u32(*d);
        }
        w.u16(u16::try_from(self.additional_outputs.len()).map_err(|_| Error::Length)?);
        for a in &self.additional_outputs {
            w.blob_u16(&a.to_consensus()?)?;
        }
        Ok(w.finish())
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let mut r = Reader::new(bytes);
        let distribution_id = r.u64()?;
        let pool_payout = TxOut::from_consensus(r.blob_u16()?)?;
        let np = r.u16()? as usize;
        let mut payouts = Vec::with_capacity(np.min(4096));
        for _ in 0..np {
            payouts.push(TxOut::from_consensus(r.blob_u16()?)?);
        }
        let nd = r.u16()? as usize;
        let mut dust_limits = Vec::with_capacity(nd.min(4096));
        for _ in 0..nd {
            dust_limits.push(r.u32()?);
        }
        let na = r.u16()? as usize;
        let mut additional_outputs = Vec::with_capacity(na.min(4096));
        for _ in 0..na {
            additional_outputs.push(TxOut::from_consensus(r.blob_u16()?)?);
        }
        r.finish()?;
        if payouts.len() != dust_limits.len() {
            return Err(Error::LengthMismatch);
        }
        Ok(Self {
            distribution_id,
            pool_payout,
            payouts,
            dust_limits,
            additional_outputs,
        })
    }

    pub fn fixed_value(&self) -> Result<u64, Error> {
        self.additional_outputs
            .iter()
            .try_fold(0u64, |acc, o| acc.checked_add(o.value).ok_or(Error::Length))
    }
}

/// The `distribution_id` TLV (PR #203 §6): 13 bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DistributionIdTlv {
    pub id: u64,
}

impl DistributionIdTlv {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(13);
        out.extend_from_slice(&EXT_PAYOUTS.to_le_bytes());
        out.push(0x01);
        out.extend_from_slice(&8u16.to_le_bytes());
        out.extend_from_slice(&self.id.to_le_bytes());
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let mut r = Reader::new(bytes);
        if r.u16()? != EXT_PAYOUTS || r.u8()? != 0x01 || r.u16()? != 8 {
            return Err(Error::InvalidTnf);
        }
        let id = r.u64()?;
        r.finish()?;
        Ok(Self { id })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PayoutAmounts {
    pub pays: Vec<Option<u64>>,
    pub pay_p: u64,
    pub w: u64,
    pub r: u64,
    pub fixed: u64,
}

/// PR #203 §4.
pub fn compute_payout_amounts(
    template_revenue: u64,
    pool_weight: u64,
    weights: &[u64],
    dust_limits: &[u32],
    fixed_amounts: u64,
) -> Result<PayoutAmounts, Error> {
    if weights.len() != dust_limits.len() {
        return Err(Error::LengthMismatch);
    }
    let w: u128 = (pool_weight as u128) + weights.iter().map(|w| *w as u128).sum::<u128>();
    if w == 0 {
        return Err(Error::Weight);
    }
    if fixed_amounts >= template_revenue {
        return Err(Error::Fixed);
    }
    let r = template_revenue - fixed_amounts;
    let mut pays = Vec::with_capacity(weights.len());
    let mut paid_sum: u128 = 0;
    for (weight, dust) in weights.iter().zip(dust_limits) {
        let amount = ((*weight as u128) * (r as u128)) / w;
        if amount >= *dust as u128 {
            pays.push(Some(amount as u64));
            paid_sum += amount;
        } else {
            pays.push(None);
        }
    }
    let pay_p = (template_revenue as u128)
        .saturating_sub(paid_sum)
        .saturating_sub(fixed_amounts as u128);
    Ok(PayoutAmounts {
        pays,
        pay_p: u64::try_from(pay_p).unwrap_or(u64::MAX),
        w: u64::try_from(w).unwrap_or(u64::MAX),
        r,
        fixed: fixed_amounts,
    })
}

/// Recompute and compare the declared coinbase outputs.
pub fn validate_distribution(
    distribution: &SetPayoutDistribution,
    template_revenue: u64,
) -> Result<PayoutAmounts, Error> {
    let fixed = distribution.fixed_value()?;
    let weights: Vec<u64> = distribution.payouts.iter().map(|p| p.value).collect();
    let amounts = compute_payout_amounts(
        template_revenue,
        distribution.pool_payout.value,
        &weights,
        &distribution.dust_limits,
        fixed,
    )?;
    Ok(amounts)
}

// ---------------------------------------------------------------------------
// 0x5350 delivery capabilities (v1)
// ---------------------------------------------------------------------------

pub const DELIVERY_VERSION: u8 = 1;
pub const MSG_SET_ARK_IDENTITY: u8 = 0x01;
pub const MSG_SET_ARK_IDENTITY_SUCCESS: u8 = 0x02;
pub const MSG_SET_ARK_IDENTITY_ERROR: u8 = 0x03;
pub const MSG_REGISTER_EPOCH_MAILBOXES: u8 = 0x04;
pub const MSG_REGISTER_EPOCH_MAILBOXES_SUCCESS: u8 = 0x05;
pub const MSG_REGISTER_EPOCH_MAILBOXES_ERROR: u8 = 0x06;
pub const MSG_ARK_PAYOUT_NOTICE: u8 = 0x07;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MailboxMode {
    None = 0,
    Static = 1,
    EpochDerived = 2,
    EpochRegistered = 3,
}

impl MailboxMode {
    pub fn from_u8(v: u8) -> Result<Self, Error> {
        match v {
            0 => Ok(Self::None),
            1 => Ok(Self::Static),
            2 => Ok(Self::EpochDerived),
            3 => Ok(Self::EpochRegistered),
            _ => Err(Error::UnsupportedMailboxMode),
        }
    }
    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EpochFallback {
    Static = 0,
    Hold = 1,
}

impl EpochFallback {
    pub fn from_u8(v: u8) -> Result<Self, Error> {
        match v {
            0 => Ok(Self::Static),
            1 => Ok(Self::Hold),
            _ => Err(Error::InvalidTnf),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetArkIdentity {
    pub ark_id: [u8; 4],
    pub b_ark: [u8; 33],
    pub mailbox_mode: MailboxMode,
    pub mailbox_pubkey: Vec<u8>,
    pub mailbox_dh: Vec<u8>,
    pub epoch_fallback: EpochFallback,
    pub sig: [u8; 64],
    pub request_id: u32,
}

impl SetArkIdentity {
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        let mut w = Writer::default();
        w.u8(DELIVERY_VERSION)
            .bytes(&self.ark_id)
            .bytes(&self.b_ark)
            .u8(self.mailbox_mode.as_u8());
        w.blob_u8(&self.mailbox_pubkey)?;
        w.blob_u8(&self.mailbox_dh)?;
        w.u8(self.epoch_fallback as u8)
            .bytes(&self.sig)
            .u32(self.request_id);
        Ok(w.finish())
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let mut r = Reader::new(bytes);
        expect_version(&mut r)?;
        let out = Self {
            ark_id: r.array()?,
            b_ark: r.array()?,
            mailbox_mode: MailboxMode::from_u8(r.u8()?)?,
            mailbox_pubkey: r.blob_u8()?.to_vec(),
            mailbox_dh: r.blob_u8()?.to_vec(),
            epoch_fallback: EpochFallback::from_u8(r.u8()?)?,
            sig: r.array()?,
            request_id: r.u32()?,
        };
        r.finish()?;
        if out.mailbox_pubkey.len() > 33 || out.mailbox_dh.len() > 33 {
            return Err(Error::InvalidTnf);
        }
        if out.request_id == 0 {
            return Err(Error::RequestIdExhausted);
        }
        Ok(out)
    }

    /// `sp_id || version || ark_id || b_ark || mode || M || D || fallback ||
    /// request_id`.
    ///
    /// AP-2 contract amendment for Fable review: the reviewed plan left
    /// `request_id` outside this preimage. That allowed an old signature to be
    /// replayed with a greater counter and roll back current delivery. The
    /// counter therefore stays last on the wire but is signed here.
    pub fn signing_payload(&self, sp_id: &[u8; 32]) -> Result<Vec<u8>, Error> {
        let mut w = Writer::default();
        w.bytes(sp_id).u8(DELIVERY_VERSION);
        w.bytes(&self.ark_id)
            .bytes(&self.b_ark)
            .u8(self.mailbox_mode.as_u8());
        w.blob_u8(&self.mailbox_pubkey)?;
        w.blob_u8(&self.mailbox_dh)?;
        w.u8(self.epoch_fallback as u8).u32(self.request_id);
        Ok(w.finish())
    }

    pub fn signed_fields_digest(&self, sp_id: &[u8; 32]) -> Result<[u8; 32], Error> {
        Ok(tagged("SPc/ArkIdentity", &self.signing_payload(sp_id)?))
    }

    pub fn sign(&mut self, sp_id: &[u8; 32], key: &Keypair) -> Result<(), Error> {
        let digest = self.signed_fields_digest(sp_id)?;
        let msg = Message::from_digest(digest);
        let sig =
            Secp256k1::signing_only().sign_schnorr_with_rng(&msg, key, &mut rand::thread_rng());
        self.sig = *sig.as_ref();
        Ok(())
    }

    pub fn verify(&self, sp_id: &[u8; 32], b_scan: &XOnlyPublicKey) -> Result<(), Error> {
        let digest = self.signed_fields_digest(sp_id)?;
        let msg = Message::from_digest(digest);
        let sig = schnorr::Signature::from_slice(&self.sig).map_err(|_| Error::Signature)?;
        Secp256k1::verification_only()
            .verify_schnorr(&sig, &msg, b_scan)
            .map_err(|_| Error::Signature)
    }

    /// SPEC §11.4 error matrix. `b_spend` is the identity's spend key and
    /// `configured_ark` the pool's server id.
    pub fn error_code(
        &self,
        sp_id: &[u8; 32],
        b_scan: &XOnlyPublicKey,
        b_spend: &[u8; 33],
        configured_ark: &[u8; 4],
    ) -> Option<&'static str> {
        if self.request_id == 0 {
            return Some("request-id-exhausted");
        }
        if self.ark_id != *configured_ark {
            return Some("wrong-ark-server");
        }
        if self.b_ark == *b_spend {
            return Some("ark-key-is-spend-key");
        }
        if PublicKey::from_slice(&self.b_ark).is_err() {
            return Some("invalid-point");
        }
        if let Err(e) = validate_mailbox_fields(self) {
            return Some(e);
        }
        if self.verify(sp_id, b_scan).is_err() {
            return Some("bad-signature");
        }
        None
    }
}

fn validate_mailbox_fields(v: &SetArkIdentity) -> Result<(), &'static str> {
    if v.mailbox_mode != MailboxMode::EpochRegistered && v.epoch_fallback == EpochFallback::Hold {
        return Err("bad-mailbox-fields");
    }
    match v.mailbox_mode {
        MailboxMode::None => {
            if v.mailbox_pubkey.is_empty() && v.mailbox_dh.is_empty() {
                Ok(())
            } else {
                Err("bad-mailbox-fields")
            }
        }
        _ => {
            if v.mailbox_pubkey.len() != 33 || v.mailbox_dh.len() != 33 {
                Err("bad-mailbox-fields")
            } else if PublicKey::from_slice(&v.mailbox_pubkey).is_err()
                || PublicKey::from_slice(&v.mailbox_dh).is_err()
            {
                Err("invalid-point")
            } else {
                Ok(())
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegisterEpochMailboxes {
    pub entries: Vec<[u8; 37]>,
    pub sig: [u8; 64],
    pub request_id: u32,
}

impl RegisterEpochMailboxes {
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        let mut w = Writer::default();
        w.u8(DELIVERY_VERSION);
        w.u16(u16::try_from(self.entries.len()).map_err(|_| Error::Length)?);
        for e in &self.entries {
            w.bytes(e);
        }
        w.bytes(&self.sig).u32(self.request_id);
        Ok(w.finish())
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let mut r = Reader::new(bytes);
        expect_version(&mut r)?;
        let n = r.u16()? as usize;
        let mut entries = Vec::with_capacity(n.min(4096));
        for _ in 0..n {
            entries.push(r.array()?);
        }
        let sig = r.array()?;
        let request_id = r.u32()?;
        r.finish()?;
        if request_id == 0 {
            return Err(Error::RequestIdExhausted);
        }
        Ok(Self {
            entries,
            sig,
            request_id,
        })
    }

    pub fn signing_payload(&self, sp_id: &[u8; 32]) -> Result<Vec<u8>, Error> {
        let mut w = Writer::default();
        w.bytes(sp_id).u8(DELIVERY_VERSION);
        w.u16(u16::try_from(self.entries.len()).map_err(|_| Error::Length)?);
        for e in &self.entries {
            w.bytes(e);
        }
        w.u32(self.request_id);
        Ok(w.finish())
    }

    pub fn signed_fields_digest(&self, sp_id: &[u8; 32]) -> Result<[u8; 32], Error> {
        Ok(tagged("SPc/EpochMailboxes", &self.signing_payload(sp_id)?))
    }

    pub fn sign(&mut self, sp_id: &[u8; 32], key: &Keypair) -> Result<(), Error> {
        let msg = Message::from_digest(self.signed_fields_digest(sp_id)?);
        let sig =
            Secp256k1::signing_only().sign_schnorr_with_rng(&msg, key, &mut rand::thread_rng());
        self.sig = *sig.as_ref();
        Ok(())
    }

    pub fn verify(&self, sp_id: &[u8; 32], b_scan: &XOnlyPublicKey) -> Result<(), Error> {
        let msg = Message::from_digest(self.signed_fields_digest(sp_id)?);
        let sig = schnorr::Signature::from_slice(&self.sig).map_err(|_| Error::Signature)?;
        Secp256k1::verification_only()
            .verify_schnorr(&sig, &msg, b_scan)
            .map_err(|_| Error::Signature)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegistrationSuccessState {
    Applied = 0,
    AlreadyApplied = 1,
    Superseded = 2,
}

impl RegistrationSuccessState {
    fn from_u8(value: u8) -> Result<Self, Error> {
        match value {
            0 => Ok(Self::Applied),
            1 => Ok(Self::AlreadyApplied),
            2 => Ok(Self::Superseded),
            _ => Err(Error::InvalidTnf),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetArkIdentitySuccess {
    pub request_id: u32,
    pub registration_rev: u64,
    pub ark_id: [u8; 4],
    pub state: RegistrationSuccessState,
}

impl SetArkIdentitySuccess {
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        let mut w = Writer::default();
        w.u8(DELIVERY_VERSION)
            .u32(self.request_id)
            .u64(self.registration_rev)
            .bytes(&self.ark_id)
            .u8(self.state as u8);
        Ok(w.finish())
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let mut r = Reader::new(bytes);
        expect_version(&mut r)?;
        let out = Self {
            request_id: r.u32()?,
            registration_rev: r.u64()?,
            ark_id: r.array()?,
            state: RegistrationSuccessState::from_u8(r.u8()?)?,
        };
        r.finish()?;
        if out.request_id == 0 {
            return Err(Error::RequestIdExhausted);
        }
        Ok(out)
    }
}

pub type RegisterEpochMailboxesSuccess = SetArkIdentitySuccess;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetArkIdentityError {
    pub request_id: u32,
    pub retryable: bool,
    pub error_code: Vec<u8>,
    pub detail: Vec<u8>,
}

impl SetArkIdentityError {
    pub fn new(request_id: u32, retryable: bool, code: &str, detail: &str) -> Self {
        Self {
            request_id,
            retryable,
            error_code: code.as_bytes().to_vec(),
            detail: detail.as_bytes().to_vec(),
        }
    }

    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        if self.error_code.len() > u8::MAX as usize || self.detail.len() > u8::MAX as usize {
            return Err(Error::Length);
        }
        std::str::from_utf8(&self.error_code).map_err(|_| Error::InvalidTnf)?;
        std::str::from_utf8(&self.detail).map_err(|_| Error::InvalidTnf)?;
        let mut w = Writer::default();
        w.u8(DELIVERY_VERSION)
            .u32(self.request_id)
            .u8(u8::from(self.retryable));
        w.blob_u8(&self.error_code)?;
        w.blob_u8(&self.detail)?;
        Ok(w.finish())
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let mut r = Reader::new(bytes);
        expect_version(&mut r)?;
        let out = Self {
            request_id: r.u32()?,
            retryable: match r.u8()? {
                0 => false,
                1 => true,
                _ => return Err(Error::InvalidTnf),
            },
            error_code: r.blob_u8()?.to_vec(),
            detail: r.blob_u8()?.to_vec(),
        };
        r.finish()?;
        std::str::from_utf8(&out.error_code).map_err(|_| Error::InvalidTnf)?;
        std::str::from_utf8(&out.detail).map_err(|_| Error::InvalidTnf)?;
        Ok(out)
    }
}

pub type RegisterEpochMailboxesError = SetArkIdentityError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArkPayoutNotice {
    pub block_hash: [u8; 32],
    pub sats: u64,
    pub vtxo_id: Vec<u8>,
    pub expiry_height: u32,
}

impl ArkPayoutNotice {
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        if self.vtxo_id.len() > 64 {
            return Err(Error::Length);
        }
        let mut w = Writer::default();
        w.u8(DELIVERY_VERSION)
            .bytes(&self.block_hash)
            .u64(self.sats);
        w.blob_u8(&self.vtxo_id)?;
        w.u32(self.expiry_height);
        Ok(w.finish())
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let mut r = Reader::new(bytes);
        expect_version(&mut r)?;
        let out = Self {
            block_hash: r.array()?,
            sats: r.u64()?,
            vtxo_id: r.blob_u8()?.to_vec(),
            expiry_height: r.u32()?,
        };
        r.finish()?;
        if out.vtxo_id.len() > 64 {
            return Err(Error::Length);
        }
        Ok(out)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeliveryMessage {
    SetArkIdentity(SetArkIdentity),
    SetArkIdentitySuccess(SetArkIdentitySuccess),
    SetArkIdentityError(SetArkIdentityError),
    RegisterEpochMailboxes(RegisterEpochMailboxes),
    RegisterEpochMailboxesSuccess(RegisterEpochMailboxesSuccess),
    RegisterEpochMailboxesError(RegisterEpochMailboxesError),
    ArkPayoutNotice(ArkPayoutNotice),
}

impl DeliveryMessage {
    pub fn message_type(&self) -> u8 {
        match self {
            Self::SetArkIdentity(_) => MSG_SET_ARK_IDENTITY,
            Self::SetArkIdentitySuccess(_) => MSG_SET_ARK_IDENTITY_SUCCESS,
            Self::SetArkIdentityError(_) => MSG_SET_ARK_IDENTITY_ERROR,
            Self::RegisterEpochMailboxes(_) => MSG_REGISTER_EPOCH_MAILBOXES,
            Self::RegisterEpochMailboxesSuccess(_) => MSG_REGISTER_EPOCH_MAILBOXES_SUCCESS,
            Self::RegisterEpochMailboxesError(_) => MSG_REGISTER_EPOCH_MAILBOXES_ERROR,
            Self::ArkPayoutNotice(_) => MSG_ARK_PAYOUT_NOTICE,
        }
    }

    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        match self {
            Self::SetArkIdentity(v) => v.encode(),
            Self::SetArkIdentitySuccess(v) | Self::RegisterEpochMailboxesSuccess(v) => v.encode(),
            Self::SetArkIdentityError(v) | Self::RegisterEpochMailboxesError(v) => v.encode(),
            Self::RegisterEpochMailboxes(v) => v.encode(),
            Self::ArkPayoutNotice(v) => v.encode(),
        }
    }

    pub fn decode(msg_type: u8, bytes: &[u8]) -> Result<Self, Error> {
        match msg_type {
            MSG_SET_ARK_IDENTITY => SetArkIdentity::decode(bytes).map(Self::SetArkIdentity),
            MSG_SET_ARK_IDENTITY_SUCCESS => {
                SetArkIdentitySuccess::decode(bytes).map(Self::SetArkIdentitySuccess)
            }
            MSG_SET_ARK_IDENTITY_ERROR => {
                SetArkIdentityError::decode(bytes).map(Self::SetArkIdentityError)
            }
            MSG_REGISTER_EPOCH_MAILBOXES => {
                RegisterEpochMailboxes::decode(bytes).map(Self::RegisterEpochMailboxes)
            }
            MSG_REGISTER_EPOCH_MAILBOXES_SUCCESS => {
                SetArkIdentitySuccess::decode(bytes).map(Self::RegisterEpochMailboxesSuccess)
            }
            MSG_REGISTER_EPOCH_MAILBOXES_ERROR => {
                SetArkIdentityError::decode(bytes).map(Self::RegisterEpochMailboxesError)
            }
            MSG_ARK_PAYOUT_NOTICE => ArkPayoutNotice::decode(bytes).map(Self::ArkPayoutNotice),
            _ => Err(Error::InvalidTnf),
        }
    }
}

fn expect_version(r: &mut Reader<'_>) -> Result<(), Error> {
    if r.u8()? == DELIVERY_VERSION {
        Ok(())
    } else {
        Err(Error::Version)
    }
}

/// Best-effort correlation for a response to a malformed request. A zero id
/// means no trustworthy correlation could be recovered.
pub fn correlated_request_id(msg_type: u8, bytes: &[u8]) -> u32 {
    fn request(msg_type: u8, bytes: &[u8]) -> Result<u32, Error> {
        let mut r = Reader::new(bytes);
        expect_version(&mut r)?;
        match msg_type {
            MSG_SET_ARK_IDENTITY => {
                r.take(4 + 33 + 1)?;
                r.blob_u8()?;
                r.blob_u8()?;
                r.take(1 + 64)?;
                r.u32()
            }
            MSG_REGISTER_EPOCH_MAILBOXES => {
                let n = r.u16()? as usize;
                r.take(n.checked_mul(37).ok_or(Error::Length)? + 64)?;
                r.u32()
            }
            MSG_SET_ARK_IDENTITY_SUCCESS
            | MSG_SET_ARK_IDENTITY_ERROR
            | MSG_REGISTER_EPOCH_MAILBOXES_SUCCESS
            | MSG_REGISTER_EPOCH_MAILBOXES_ERROR => r.u32(),
            _ => Ok(0),
        }
    }
    request(msg_type, bytes).unwrap_or(0)
}

/// Map an undecodable client frame to the exact terminal v1 error code.
pub fn correspondent_error_code(msg_type: u8, bytes: &[u8]) -> &'static str {
    match DeliveryMessage::decode(msg_type, bytes) {
        Ok(_) => "invalid-message",
        Err(Error::UnsupportedMailboxMode) => "unsupported-mailbox-mode",
        Err(Error::RequestIdExhausted) => "request-id-exhausted",
        Err(_) => "invalid-message",
    }
}

// ---------------------------------------------------------------------------
// Ark send orders
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArkSendOrder {
    pub order_id: [u8; 16],
    pub ark_address: String,
    pub sats: u64,
    pub not_before_height: u32,
    pub sig: [u8; 64],
}

impl ArkSendOrder {
    /// WIRE §3.4: fixed fields plus a U8-length UTF-8 address (no envelope).
    pub const MAX_ENCODED_LEN: usize = 16 + 1 + 255 + 8 + 4 + 64;
    pub const MIN_ENCODED_LEN: usize = 16 + 1 + 8 + 4 + 64;
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        let mut w = Writer::default();
        w.bytes(&self.order_id);
        w.blob_u8(self.ark_address.as_bytes())?;
        w.u64(self.sats)
            .u32(self.not_before_height)
            .bytes(&self.sig);
        Ok(w.finish())
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let mut r = Reader::new(bytes);
        let order_id = r.array()?;
        let ark_address = std::str::from_utf8(r.blob_u8()?)
            .map_err(|_| Error::InvalidTnf)?
            .to_string();
        let sats = r.u64()?;
        let not_before_height = r.u32()?;
        let sig = r.array()?;
        r.finish()?;
        Ok(Self {
            order_id,
            ark_address,
            sats,
            not_before_height,
            sig,
        })
    }
    pub fn signing_payload(&self) -> Result<Vec<u8>, Error> {
        let mut w = Writer::default();
        w.bytes(&self.order_id);
        w.blob_u8(self.ark_address.as_bytes())?;
        w.u64(self.sats).u32(self.not_before_height);
        Ok(w.finish())
    }
    pub fn sign(&mut self, key: &Keypair) -> Result<(), Error> {
        let payload = self.signing_payload()?;
        let msg = Message::from_digest(tagged("SPc/ArkSendOrder", &payload));
        let sig =
            Secp256k1::signing_only().sign_schnorr_with_rng(&msg, key, &mut rand::thread_rng());
        self.sig = *sig.as_ref();
        Ok(())
    }
    pub fn verify(&self, pubkey: &XOnlyPublicKey) -> Result<(), Error> {
        let payload = self.signing_payload()?;
        let msg = Message::from_digest(tagged("SPc/ArkSendOrder", &payload));
        let sig = schnorr::Signature::from_slice(&self.sig).map_err(|_| Error::Signature)?;
        Secp256k1::verification_only()
            .verify_schnorr(&sig, &msg, pubkey)
            .map_err(|_| Error::Signature)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReceiptStatus {
    Sent = 0,
    Failed = 1,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArkSendReceipt {
    pub order_id: [u8; 16],
    pub status: ReceiptStatus,
    pub vtxo_id: Vec<u8>,
    pub expiry_height: u32,
}

impl ArkSendReceipt {
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        let mut w = Writer::default();
        w.bytes(&self.order_id).u8(self.status as u8);
        w.blob_u8(&self.vtxo_id)?;
        w.u32(self.expiry_height);
        Ok(w.finish())
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let mut r = Reader::new(bytes);
        let order_id = r.array()?;
        let status = match r.u8()? {
            0 => ReceiptStatus::Sent,
            1 => ReceiptStatus::Failed,
            _ => return Err(Error::InvalidTnf),
        };
        let out = Self {
            order_id,
            status,
            vtxo_id: r.blob_u8()?.to_vec(),
            expiry_height: r.u32()?,
        };
        r.finish()?;
        Ok(out)
    }
}

fn tagged(tag: &str, payload: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let tag_hash = Sha256::digest(tag.as_bytes());
    let mut h = Sha256::new();
    h.update(tag_hash);
    h.update(tag_hash);
    h.update(payload);
    h.finalize().into()
}

pub mod authorization;
#[cfg(test)]
mod tests;
