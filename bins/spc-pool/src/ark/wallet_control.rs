//! Local CLIENT control of the already-open stock pool wallet.
//!
//! A receipt journal binds an opaque operation to one signed funding proposal.
//! It is not a balance ledger. Stock wallet action checkpoints own boarding,
//! rebroadcast, registration, and recovery. No server-side order-id lookup exists.

use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::wallet_control_proof::terminal_context_bytes;
use super::wallet_control_proof::{
    digest, LineageState, OriginalBoardProof, ProofEnvelope, RoundImportFacts,
};
use anyhow::{bail, ensure, Context};
use bitcoin::hashes::{sha256, Hash};
use rustix::fs::{self, Mode, OFlags};
use serde::{Deserialize, Serialize};

pub const REQUEST_SCHEMA: &str = "spc.lab.ap5.pool-wallet-request/v1";
pub const RESULT_SCHEMA: &str = "spc.lab.ap5.pool-wallet-result/v1";
const MAX_FILE: u64 = 256 * 1024;
const MAX_OPERATIONS: usize = 4096;
const CONTROL_DIR: &str = "wallet-control";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub schema: String,
    pub operation_id: String,
    pub action: Action,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net_target_sats: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_fee_sats: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_board_operation_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Snapshot,
    BoardFloat,
    BoardProofV1,
}

impl Request {
    pub(super) fn validate(&self) -> anyhow::Result<()> {
        ensure!(self.schema == REQUEST_SCHEMA, "request schema");
        ensure!(
            !self.operation_id.is_empty()
                && self.operation_id.len() <= 128
                && self.operation_id.bytes().all(|b| b.is_ascii_graphic()),
            "operation id"
        );
        match self.action {
            Action::Snapshot => ensure!(
                self.net_target_sats.is_none()
                    && self.max_fee_sats.is_none()
                    && self.original_board_operation_id.is_none(),
                "snapshot fields"
            ),
            Action::BoardFloat => ensure!(
                self.net_target_sats
                    .is_some_and(|n| n > 0 && n <= bitcoin::Amount::MAX_MONEY.to_sat())
                    && self
                        .max_fee_sats
                        .is_some_and(|n| n <= bitcoin::Amount::MAX_MONEY.to_sat())
                    && self.original_board_operation_id.is_none(),
                "board amounts"
            ),
            Action::BoardProofV1 => ensure!(
                self.net_target_sats.is_none()
                    && self.max_fee_sats.is_none()
                    && self
                        .original_board_operation_id
                        .as_ref()
                        .is_some_and(|id| !id.is_empty()
                            && id.len() <= 128
                            && id.bytes().all(|b| b.is_ascii_graphic())
                            && id != &self.operation_id),
                "proof request fields"
            ),
        }
        Ok(())
    }

    fn key(&self) -> String {
        sha256::Hash::hash(self.operation_id.as_bytes()).to_string()
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Queued,
    Observed,
    Rejected,
    AlreadyAtTarget,
    Unknown,
    Submitted,
    Confirmed,
    ProofAvailable,
    ProofMissing,
    ProofUnavailable,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResultDto {
    pub schema: String,
    pub operation_id: String,
    pub state: State,
    pub onchain_trusted_sats: Option<u64>,
    pub float_spendable_sats: Option<u64>,
    pub observed_at_unix_ms: Option<u64>,
    pub observed_tip_height: Option<u32>,
    pub error_code: Option<String>,
    pub funding_txid: Option<String>,
    pub board: Option<BoardFacts>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proof: Option<ProofEnvelope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retained_proof_sha256: Option<String>,
}

impl ResultDto {
    fn new(request: &Request) -> Self {
        Self {
            schema: RESULT_SCHEMA.into(),
            operation_id: request.operation_id.clone(),
            state: State::Queued,
            onchain_trusted_sats: None,
            float_spendable_sats: None,
            observed_at_unix_ms: None,
            observed_tip_height: None,
            error_code: None,
            funding_txid: None,
            board: None,
            proof: None,
            retained_proof_sha256: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoardFacts {
    pub gross_board_sats: u64,
    pub net_board_sats: u64,
    pub board_fee_sats: u64,
    pub onchain_fee_sats: u64,
    pub actual_fee_sats: u64,
    pub expiry_height: u32,
    pub required_confirmations: u64,
    pub server_pubkey: String,
    pub vtxo_exit_delta: u32,
    pub min_board_sats: u64,
    pub max_vtxo_sats: Option<u64>,
    pub vtxo_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Prepared {
    // Persisted BEFORE board_psbt: binds the actual signed bytes, owned inputs,
    // policy, key and request. Secret key is rederived from the stock key index.
    psbt_hex: String,
    signed_tx_hex: String,
    key_index: u32,
    user_pubkey: String,
    funding_vout: u32,
    policy_json: String,
    inputs: Vec<String>,
    float_before_sats: u64,
    observed_tip_height: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    request: Request,
    result: ResultDto,
    prepared: Option<Prepared>,
    #[serde(default)]
    acceptance: Option<AcceptanceProof>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    proof_digest: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AcceptanceProof {
    full_vtxo_hex: String,
    checked_at_unix_ms: u64,
    checked_tip_height: u32,
    funding_confirmations: u64,
}

struct Resolution {
    state: State,
    acceptance: Option<AcceptanceProof>,
}
impl Resolution {
    fn pending(state: State) -> Self {
        Self {
            state,
            acceptance: None,
        }
    }
}

impl Record {
    fn validate(&self) -> anyhow::Result<()> {
        self.request.validate()?;
        let valid_state = match self.request.action {
            Action::BoardFloat => matches!(
                self.result.state,
                State::Rejected
                    | State::AlreadyAtTarget
                    | State::Unknown
                    | State::Submitted
                    | State::Confirmed
            ),
            Action::Snapshot => matches!(self.result.state, State::Observed | State::Rejected),
            Action::BoardProofV1 => matches!(
                self.result.state,
                State::ProofAvailable | State::ProofMissing | State::ProofUnavailable
            ),
        };
        ensure!(valid_state, "operation action/state binding");
        if let Some(proof) = &self.result.proof {
            ensure!(
                self.request.action == Action::BoardProofV1
                    && self.result.state == State::ProofAvailable,
                "proof response action/state"
            );
            proof.validate(
                self.request
                    .original_board_operation_id
                    .as_deref()
                    .context("original operation")?,
            )?;
        } else {
            ensure!(
                self.result.state != State::ProofAvailable,
                "proof available without proof"
            );
        }
        if let Some(digest) = &self.proof_digest {
            ensure!(
                self.result.state == State::Confirmed && valid_key(digest),
                "retained proof reference"
            );
        }
        ensure!(
            self.result.retained_proof_sha256 == self.proof_digest,
            "public/private proof reference mismatch"
        );
        ensure!(
            (self.result.state == State::Confirmed) == self.acceptance.is_some(),
            "confirmation evidence required"
        );
        ensure!(
            self.result.schema == RESULT_SCHEMA
                && self.result.operation_id == self.request.operation_id,
            "receipt binding"
        );
        let effect = matches!(
            self.result.state,
            State::Unknown | State::Submitted | State::Confirmed
        );
        ensure!(
            effect == self.prepared.is_some()
                && effect == self.result.board.is_some()
                && effect == self.result.funding_txid.is_some(),
            "receipt phase binding"
        );
        if let (Some(p), Some(f)) = (&self.prepared, &self.result.board) {
            ensure!(
                self.request.action == Action::BoardFloat,
                "snapshot cannot board"
            );
            let psbt = bitcoin::Psbt::deserialize(&hex::decode(&p.psbt_hex)?)?;
            let tx = psbt.clone().extract_tx()?;
            ensure!(
                psbt.inputs.iter().all(|i| i.final_script_witness.is_some())
                    && hex::encode(bitcoin::consensus::serialize(&tx)) == p.signed_tx_hex
                    && Some(tx.compute_txid().to_string()) == self.result.funding_txid
                    && tx
                        .input
                        .iter()
                        .map(|i| i.previous_output.to_string())
                        .collect::<Vec<_>>()
                        == p.inputs,
                "signed proposal binding"
            );
            let output = tx
                .output
                .get(p.funding_vout as usize)
                .context("funding output")?;
            let fees: ark::fees::BoardFees = serde_json::from_str(&p.policy_json)?;
            ensure!(
                output.value.to_sat() == f.gross_board_sats
                    && fees.calculate(output.value).map(|n| n.to_sat()) == Some(f.board_fee_sats)
                    && f.gross_board_sats.checked_sub(f.board_fee_sats) == Some(f.net_board_sats)
                    && psbt.fee()?.to_sat() == f.onchain_fee_sats
                    && f.onchain_fee_sats.checked_add(f.board_fee_sats) == Some(f.actual_fee_sats)
                    && f.actual_fee_sats <= self.request.max_fee_sats.context("fee cap")?
                    && p.float_before_sats.checked_add(f.net_board_sats)
                        == self.request.net_target_sats
                    && f.expiry_height > p.observed_tip_height,
                "funding fee/payload binding"
            );
            if let Some(proof) = &self.acceptance {
                use ark::ProtocolEncoding;
                let vtxo =
                    ark::Vtxo::<ark::vtxo::Full>::deserialize(&hex::decode(&proof.full_vtxo_hex)?)?;
                vtxo.validate(&tx)?;
                ensure!(
                    vtxo.chain_anchor()
                        == bitcoin::OutPoint::new(tx.compute_txid(), p.funding_vout)
                        && vtxo.user_pubkey().to_string() == p.user_pubkey
                        && vtxo.server_pubkey().to_string() == f.server_pubkey
                        && vtxo.amount().to_sat() == f.net_board_sats
                        && vtxo.expiry_height().to_u32() == f.expiry_height
                        && Some(vtxo.id().to_string()) == f.vtxo_id
                        && proof.checked_at_unix_ms > 0
                        && proof.checked_tip_height < f.expiry_height
                        && proof.funding_confirmations > 0
                        && proof.funding_confirmations >= f.required_confirmations,
                    "board acceptance proof binding"
                );
            }
        }
        Ok(())
    }
}

/// All names below are constants or digests. Operations use an owned directory
/// fd; every ancestor and leaf is opened NOFOLLOW. FIFOs cannot block opening.
struct PrivateDir(File);

/// Establish only the owned wallet directory. Existing ancestors may be
/// root-owned/0755 (the shipped /var/lib/spc/ark layout); never chmod them.
/// All traversal/creation is relative to no-follow directory descriptors.
pub fn prepare_datadir(path: &Path) -> anyhow::Result<()> {
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut fd = fs::open(
        "/",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )?;
    let parts: Vec<_> = absolute.components().collect();
    ensure!(parts.len() > 1, "wallet directory cannot be root");
    for part in parts {
        match part {
            Component::RootDir => (),
            Component::Normal(name) => {
                let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
                fd = match fs::openat(&fd, name, flags, Mode::empty()) {
                    Ok(next) => next,
                    Err(rustix::io::Errno::NOENT) => {
                        match fs::mkdirat(&fd, name, Mode::from_raw_mode(0o700)) {
                            Ok(()) | Err(rustix::io::Errno::EXIST) => (),
                            Err(e) => return Err(e.into()),
                        }
                        fs::fsync(&fd)?;
                        fs::openat(&fd, name, flags, Mode::empty())?
                    }
                    Err(e) => return Err(e.into()),
                };
            }
            _ => bail!("noncanonical wallet directory"),
        }
    }
    let file = File::from(fd);
    ensure!(
        file.metadata()?.uid() == rustix::process::getuid().as_raw(),
        "foreign wallet directory owner"
    );
    fs::fchmod(&file, Mode::from_raw_mode(0o700))?;
    file.sync_all()?;
    let dir = PrivateDir(file);
    // Metadata-only opens are important here: closing an ordinary I/O fd for
    // LOCK (or an active SQLite file) can release this process's POSIX locks.
    // O_PATH never opens the object for I/O and does not release those locks.
    // Queue children have their own strict per-entry validation.
    for entry in std::fs::read_dir(format!("/proc/self/fd/{}", dir.0.as_raw_fd()))? {
        let name = entry?.file_name();
        let child = File::from(fs::openat(
            &dir.0,
            &name,
            OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?);
        check_wallet_root_entry(&child, &name)?;
    }
    Ok(())
}

/// The pinned default lock manager's PID stamp is public metadata protected
/// by the already-private wallet root. Stock creates it as 0666 & !umask.
/// Only that source-identified metadata class tolerates non-private mode;
/// database, secret, control and unknown files keep the exact 0600 policy.
fn check_wallet_root_entry(file: &File, name: &std::ffi::OsStr) -> anyhow::Result<()> {
    let metadata = file.metadata()?;
    if name == std::ffi::OsStr::new(bark::lock_manager::pid_fcntl::LOCK_FILE) {
        PrivateDir::check_owned_type(&metadata, false)?;
        let mode = metadata.mode() & 0o7777;
        ensure!(
            mode & 0o600 == 0o600 && mode & !0o666 == 0,
            "unsafe stock PID metadata mode"
        );
        // No chmod, truncate, read, replacement, or unlink of a stock lock.
        Ok(())
    } else {
        PrivateDir::check(file, metadata.is_dir())
    }
}

pub(super) fn prepare_wallet_database(path: &Path) -> anyhow::Result<()> {
    prepare_datadir(path)?;
    let dir = PrivateDir::open(path)?;
    let file = File::from(fs::openat(
        &dir.0,
        "wallet.sqlite",
        OFlags::RDWR | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o600),
    )?);
    PrivateDir::check(&file, false)?;
    Ok(())
}

impl PrivateDir {
    fn present(&self, name: &str) -> anyhow::Result<bool> {
        match fs::statat(&self.0, name, fs::AtFlags::SYMLINK_NOFOLLOW) {
            Ok(_) => Ok(true),
            Err(rustix::io::Errno::NOENT) => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    fn entries(&self) -> anyhow::Result<Vec<String>> {
        use std::os::unix::ffi::OsStrExt;
        let mut names = Vec::new();
        for (index, entry) in
            std::fs::read_dir(format!("/proc/self/fd/{}", self.0.as_raw_fd()))?.enumerate()
        {
            ensure!(index < MAX_OPERATIONS * 3 + 16, "queue directory capacity");
            let name = entry?.file_name();
            names.push(name.to_str().map(str::to_owned).unwrap_or_else(|| {
                format!("invalid-utf8-{}.record", hex::encode(name.as_bytes()))
            }));
        }
        names.sort();
        Ok(names)
    }

    fn raw(&self, name: &str) -> anyhow::Result<Option<Vec<u8>>> {
        let fd = match fs::openat(
            &self.0,
            name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(fd) => fd,
            Err(rustix::io::Errno::NOENT) => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let file = File::from(fd);
        Self::check(&file, false)?;
        ensure!(file.metadata()?.len() <= MAX_FILE, "private file too large");
        let mut bytes = Vec::new();
        file.take(MAX_FILE + 1).read_to_end(&mut bytes)?;
        ensure!(bytes.len() as u64 <= MAX_FILE, "private file too large");
        Ok(Some(bytes))
    }
    fn open(path: &Path) -> anyhow::Result<Self> {
        let absolute = if path.is_absolute() {
            path.to_owned()
        } else {
            std::env::current_dir()?.join(path)
        };
        let mut fd = fs::open(
            "/",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        for part in absolute.components() {
            match part {
                Component::RootDir => (),
                Component::Normal(name) => {
                    fd = fs::openat(
                        &fd,
                        name,
                        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                        Mode::empty(),
                    )?
                }
                _ => bail!("noncanonical private directory"),
            }
        }
        let file = File::from(fd);
        Self::check(&file, true)?;
        Ok(Self(file))
    }

    fn check_owned_type(m: &std::fs::Metadata, directory: bool) -> anyhow::Result<()> {
        ensure!(
            m.uid() == rustix::process::getuid().as_raw()
                && if directory {
                    m.is_dir()
                } else {
                    m.is_file() && m.nlink() == 1
                },
            "private file ownership/type"
        );
        Ok(())
    }

    fn check(file: &File, directory: bool) -> anyhow::Result<()> {
        let metadata = file.metadata()?;
        Self::check_owned_type(&metadata, directory)?;
        ensure!(
            metadata.mode() & 0o7777 == if directory { 0o700 } else { 0o600 },
            "private file mode"
        );
        Ok(())
    }

    fn child(&self, name: &str) -> anyhow::Result<Self> {
        match fs::mkdirat(&self.0, name, Mode::from_raw_mode(0o700)) {
            Ok(()) => self.0.sync_all()?,
            Err(rustix::io::Errno::EXIST) => (),
            Err(e) => return Err(e.into()),
        }
        let f = File::from(fs::openat(
            &self.0,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?);
        Self::check(&f, true)?;
        Ok(Self(f))
    }

    fn read<T: serde::de::DeserializeOwned>(&self, name: &str) -> anyhow::Result<Option<T>> {
        let fd = match fs::openat(
            &self.0,
            name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(fd) => fd,
            Err(rustix::io::Errno::NOENT) => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let f = File::from(fd);
        Self::check(&f, false)?;
        ensure!(f.metadata()?.len() <= MAX_FILE, "private file too large");
        let mut bytes = Vec::new();
        f.take(MAX_FILE + 1).read_to_end(&mut bytes)?;
        ensure!(bytes.len() as u64 <= MAX_FILE, "private file too large");
        Ok(Some(serde_json::from_slice(&bytes)?))
    }

    fn write<T: Serialize>(&self, name: &str, value: &T) -> anyhow::Result<()> {
        let bytes = serde_json::to_vec(value)?;
        ensure!(bytes.len() as u64 <= MAX_FILE, "private file too large");
        let temp = format!(".tmp-{}", hex::encode(rand::random::<[u8; 16]>()));
        let mut f = File::from(fs::openat(
            &self.0,
            &temp,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )?);
        f.write_all(&bytes)?;
        f.sync_all()?;
        fs::renameat(&self.0, &temp, &self.0, name)?;
        self.0.sync_all()?;
        Ok(())
    }

    fn write_immutable<T: Serialize>(&self, name: &str, value: &T) -> anyhow::Result<()> {
        let bytes = serde_json::to_vec(value)?;
        ensure!(bytes.len() as u64 <= MAX_FILE, "immutable object capacity");
        if self.present(name)? {
            ensure!(
                self.raw(name)?.as_deref() == Some(bytes.as_slice()),
                "immutable object conflict"
            );
            return Ok(());
        }
        let temp = format!(".tmp-{}", hex::encode(rand::random::<[u8; 16]>()));
        let mut file = File::from(fs::openat(
            &self.0,
            &temp,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )?);
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::renameat_with(&self.0, &temp, &self.0, name, fs::RenameFlags::NOREPLACE)?;
        self.0.sync_all()?;
        Ok(())
    }

    fn lock(&self, name: &str) -> anyhow::Result<File> {
        let f = File::from(fs::openat(
            &self.0,
            name,
            OFlags::RDWR | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )?);
        Self::check(&f, false)?;
        fs::flock(&f, fs::FlockOperation::NonBlockingLockExclusive)?;
        Ok(f)
    }

    fn names(&self, suffix: &str) -> anyhow::Result<Vec<String>> {
        let names: Vec<_> = self
            .entries()?
            .into_iter()
            .filter(|n| n.ends_with(suffix))
            .collect();
        ensure!(names.len() <= MAX_OPERATIONS, "queue capacity");
        Ok(names)
    }
}

pub fn read_request(path: &Path) -> anyhow::Result<Request> {
    let dir = PrivateDir::open(path.parent().context("request parent")?)?;
    let request: Request = dir
        .read(
            path.file_name()
                .and_then(|s| s.to_str())
                .context("request filename")?,
        )?
        .context("request absent")?;
    request.validate()?;
    Ok(request)
}

/// Atomic enqueue and bounded wait. Timeout reports Queued, never cancellation.
/// Poll by rerunning with the EXACT same file/payload/operation id.
pub fn submit(datadir: &Path, request: &Request) -> anyhow::Result<ResultDto> {
    request.validate()?;
    let dir = PrivateDir::open(datadir)?.child(CONTROL_DIR)?;
    let key = request.key();
    {
        let _lock = dir.lock("enqueue.lock")?;
        if let Some(admitted) = dir.read::<Request>(&format!("{key}.admitted"))? {
            ensure!(admitted == *request, "operation_payload_conflict");
        }
        if let Some(record) = dir.read::<Record>(&format!("{key}.record"))? {
            record.validate()?;
            ensure!(record.request == *request, "operation_payload_conflict");
        }
        let name = format!("{key}.request");
        if let Some(existing) = dir.read::<Request>(&name)? {
            ensure!(existing == *request, "operation_payload_conflict");
        } else {
            ensure!(
                dir.names(".request")?.len() < MAX_OPERATIONS,
                "queue capacity"
            );
            dir.write(&name, request)?;
        }
    }
    for _ in 0..100 {
        if let Some(record) = dir.read::<Record>(&format!("{key}.record"))? {
            record.validate()?;
            ensure!(record.request == *request, "operation_payload_conflict");
            return Ok(record.result);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let mut result = ResultDto::new(request);
    if request.action == Action::BoardFloat && dir.present(&format!("{key}.admitted"))? {
        result.state = State::Unknown;
        result.error_code = Some("durable_admission_without_receipt".into());
    }
    Ok(result)
}

#[derive(Clone, Debug)]
struct Observation {
    onchain: u64,
    float: u64,
    tip: u32,
}

impl Observation {
    fn apply(&self, result: &mut ResultDto) -> anyhow::Result<()> {
        result.onchain_trusted_sats = Some(self.onchain);
        result.float_spendable_sats = Some(self.float);
        result.observed_tip_height = Some(self.tip);
        result.observed_at_unix_ms = Some(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)?
                .as_millis()
                .try_into()?,
        );
        Ok(())
    }
}

#[async_trait::async_trait]
trait WalletOwner: Send + Sync {
    async fn round_imports(&self) -> anyhow::Result<Vec<RoundImportFacts>> {
        Ok(vec![])
    }
    /// Absolutely no wallet sync, address derivation, or wallet writes here.
    async fn observe(&self) -> anyhow::Result<Observation>;
    async fn pending(&self) -> anyhow::Result<bool>;
    async fn prepare(
        &self,
        request: &Request,
        observation: &Observation,
    ) -> anyhow::Result<(Prepared, BoardFacts)>;
    async fn start(&self, prepared: &Prepared, facts: &BoardFacts) -> anyhow::Result<()>;
    async fn resolve(
        &self,
        prepared: &Prepared,
        facts: &mut BoardFacts,
    ) -> anyhow::Result<Resolution>;
    /// Stock per-VTXO reservation protects an unobserved board from refresh.
    async fn protect(&self, _prepared: &Prepared) -> anyhow::Result<()> {
        Ok(())
    }
    async fn release_confirmed(&self, _prepared: &Prepared) -> anyhow::Result<()> {
        Ok(())
    }
    async fn recover(&self) -> anyhow::Result<()> {
        Ok(())
    }
    async fn maintain(&self) -> anyhow::Result<()> {
        Ok(())
    }
}

/// Acquire before opening the stock wallet; retain for the role lifetime.
pub struct Controller {
    dir: PrivateDir,
    faults: PrivateDir,
    proofs: PrivateDir,
    lineage: PrivateDir,
    _lease: File,
    cursor: std::sync::Mutex<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct QueueFault {
    name_hex: String,
    fingerprint: String,
    reason: String,
    authority_may_exist: bool,
    safe_bytes_hex: Option<String>,
}

impl Controller {
    fn retain_proof(&self, record: &Record) -> anyhow::Result<String> {
        record.validate()?;
        ensure!(
            record.result.state == State::Confirmed,
            "unconfirmed proof retention"
        );
        let acceptance = record.acceptance.as_ref().context("acceptance absent")?;
        let prepared = record.prepared.as_ref().context("proposal absent")?;
        let proof = OriginalBoardProof::create(
            &record.request,
            &record.result,
            prepared.signed_tx_hex.clone(),
            acceptance.full_vtxo_hex.clone(),
            acceptance.checked_at_unix_ms,
            acceptance.checked_tip_height,
            acceptance.funding_confirmations,
        )?;
        let bytes = proof.canonical_bytes()?;
        let hash = digest(&bytes);
        let name = format!("{hash}.proof");
        if !self.proofs.present(&name)? {
            ensure!(
                self.proofs.names(".proof")?.len() < MAX_OPERATIONS,
                "proof retention capacity"
            );
        }
        self.proofs.write_immutable(&name, &proof)?;
        Ok(hash)
    }

    async fn capture_round_imports<W: WalletOwner>(&self, wallet: &W) -> anyhow::Result<()> {
        let imports = wallet.round_imports().await?;
        ensure!(imports.len() <= MAX_OPERATIONS, "round import capacity");
        for import in imports {
            import.validate()?;
            let hash = digest(&serde_json::to_vec(&import)?);
            let name = format!("{hash}.round");
            if !self.lineage.present(&name)? {
                ensure!(
                    self.lineage.names(".round")?.len() < MAX_OPERATIONS,
                    "lineage retention capacity"
                );
            }
            self.lineage.write_immutable(&name, &import)?;
        }
        Ok(())
    }

    fn lineage_for(&self, original_vtxo_id: &str) -> anyhow::Result<Vec<RoundImportFacts>> {
        let names = self.lineage.names(".round")?;
        ensure!(names.len() <= 256, "lineage read capacity");
        let mut all = Vec::new();
        let mut movement_hashes = std::collections::BTreeMap::new();
        for name in names {
            let raw = self.lineage.raw(&name)?.context("lineage missing")?;
            let hash = digest(&raw);
            ensure!(name == format!("{hash}.round"), "lineage digest mismatch");
            let import: RoundImportFacts = serde_json::from_slice(&raw)?;
            import.validate()?;
            ensure!(serde_json::to_vec(&import)? == raw, "lineage not canonical");
            if let Some(previous) = movement_hashes.insert(import.stock_movement_id, hash.clone()) {
                ensure!(previous == hash, "conflicting stock movement facts");
            }
            all.push(import);
        }
        all.sort_by_key(|m| m.stock_movement_id);
        let mut known = std::collections::BTreeSet::from([original_vtxo_id.to_owned()]);
        let mut selected = std::collections::BTreeMap::new();
        loop {
            let before = selected.len();
            for edge in &all {
                if edge.input_vtxo_ids.iter().any(|id| known.contains(id)) {
                    known.extend(edge.output_vtxo_ids.iter().cloned());
                    selected.insert(edge.stock_movement_id, edge.clone());
                }
            }
            if before == selected.len() {
                break;
            }
        }
        Ok(selected.into_values().collect())
    }

    fn load_original_proof(
        &self,
        request: &Request,
    ) -> Result<ProofEnvelope, (State, &'static str)> {
        let unavailable = (State::ProofUnavailable, "proof_unavailable");
        let original_id = request
            .original_board_operation_id
            .as_deref()
            .ok_or(unavailable)?;
        let key = digest(original_id.as_bytes());
        let name = format!("{key}.record");
        let record: Record = match self
            .dir
            .read(&name)
            .map_err(|_| (State::ProofUnavailable, "original_operation_unavailable"))?
        {
            Some(record) => record,
            None if self
                .dir
                .present(&format!("{key}.admitted"))
                .map_err(|_| unavailable)? =>
            {
                return Err((State::ProofUnavailable, "original_board_unresolved"))
            }
            None => return Err((State::ProofMissing, "original_operation_missing")),
        };
        record
            .validate()
            .map_err(|_| (State::ProofUnavailable, "original_operation_unavailable"))?;
        if record.request.operation_id != original_id || record.request.action != Action::BoardFloat
        {
            return Err((State::ProofUnavailable, "original_operation_mismatch"));
        }
        if record.result.state != State::Confirmed {
            return Err((State::ProofUnavailable, "original_board_not_confirmed"));
        }
        // Old receipts without a retained-object reference are not rewritten or
        // synthesized from a current stock/server lookup, even if amounts match.
        let hash = record
            .proof_digest
            .as_ref()
            .ok_or((State::ProofMissing, "proof_not_retained"))?;
        let bytes = self
            .proofs
            .raw(&format!("{hash}.proof"))
            .map_err(|_| unavailable)?
            .ok_or(unavailable)?;
        if digest(&bytes) != *hash {
            return Err((State::ProofUnavailable, "proof_digest_mismatch"));
        }
        let original: OriginalBoardProof =
            serde_json::from_slice(&bytes).map_err(|_| unavailable)?;
        original.validate().map_err(|_| unavailable)?;
        let acceptance = record.acceptance.as_ref().ok_or(unavailable)?;
        let prepared = record.prepared.as_ref().ok_or(unavailable)?;
        if original.board_request != record.request
            || original.original_full_vtxo_hex != acceptance.full_vtxo_hex
            || original.raw_anchor_tx_hex != prepared.signed_tx_hex
            || original.checked_at_unix_ms != acceptance.checked_at_unix_ms
            || original.checked_tip_height != acceptance.checked_tip_height
            || original.funding_confirmations != acceptance.funding_confirmations
            || original.terminal_receipt_context_sha256
                != digest(&terminal_context_bytes(&record.result).map_err(|_| unavailable)?)
            || original.canonical_bytes().map_err(|_| unavailable)? != bytes
        {
            return Err((State::ProofUnavailable, "proof_binding_mismatch"));
        }
        let (lineage_state, round_imports, lineage_error_code) =
            match self.lineage_for(&original.original_vtxo_id) {
                Ok(edges) if !edges.is_empty() => (LineageState::RecordedPartial, edges, None),
                Ok(_) => (
                    LineageState::Unavailable,
                    vec![],
                    Some("no_retained_round_imports".into()),
                ),
                Err(_) => (
                    LineageState::Unavailable,
                    vec![],
                    Some("round_import_evidence_unavailable".into()),
                ),
            };
        let envelope = ProofEnvelope {
            schema: super::wallet_control_proof::ENVELOPE_SCHEMA.into(),
            proof_sha256: hash.clone(),
            canonical_json_hex: hex::encode(bytes),
            original,
            lineage_state,
            lineage_complete: false,
            round_imports,
            lineage_error_code,
        };
        envelope.validate(original_id).map_err(|_| unavailable)?;
        Ok(envelope)
    }

    fn save_proof_reply(&self, record_name: &str, request: Request) -> anyhow::Result<()> {
        let mut result = ResultDto::new(&request);
        match self.load_original_proof(&request) {
            Ok(proof) => {
                result.state = State::ProofAvailable;
                result.proof = Some(proof);
            }
            Err((state, code)) => {
                result.state = state;
                result.error_code = Some(code.into());
            }
        }
        let record = Record {
            request,
            result,
            prepared: None,
            acceptance: None,
            proof_digest: None,
        };
        record.validate()?;
        self.dir.write(record_name, &record)?;
        Ok(())
    }

    async fn recovery_pass<W: WalletOwner>(&self, wallet: &W) -> anyhow::Result<()> {
        wallet.recover().await?;
        self.protect_receipts(wallet).await
    }

    async fn maintenance_pass<W: WalletOwner>(&self, wallet: &W) -> anyhow::Result<()> {
        // A damaged receipt cannot stop unrelated refresh. Existing valid
        // receipts are protected independently, and corruption still blocks
        // new funding through unresolved(). No whole maintenance cancellation.
        let protection = self.protect_receipts(wallet).await;
        let maintenance = wallet.maintain().await;
        let imports = self.capture_round_imports(wallet).await;
        protection.and(maintenance).and(imports)
    }
    async fn observe_read<W: WalletOwner>(wallet: &W) -> anyhow::Result<Observation> {
        tokio::time::timeout(super::wallet_control_io::READ_DEADLINE, wallet.observe())
            .await
            .context("read observation deadline")?
    }
    pub async fn startup(
        &self,
        client: &super::bark_stock::BarkWalletClient,
    ) -> anyhow::Result<()> {
        // No full scan/interactive refresh at startup. Recovery and bounded
        // read-sync run independently once StockOwner has the live handle.
        let _owner = client.operation_lock.lock().await;
        client.wallet().require_ark_info().await?;
        for name in self.dir.names(".record")? {
            if let Ok(Some(record)) = self.dir.read::<Record>(&name) {
                if record.validate().is_ok() && record.result.state != State::Confirmed {
                    if let Some(prepared) = &record.prepared {
                        StockOwner::protect_client(client, prepared).await?;
                    }
                }
            }
        }
        Ok(())
    }

    pub fn open(datadir: &Path) -> anyhow::Result<Self> {
        prepare_datadir(datadir)?;
        let dir = PrivateDir::open(datadir)?.child(CONTROL_DIR)?;
        let lease = dir.lock("owner.lock")?;
        let faults = dir.child("faults")?;
        let proofs = dir.child("proofs")?;
        let lineage = dir.child("lineage")?;
        let cursor_read = dir.read::<String>("cursor.json");
        let cursor = cursor_read
            .as_ref()
            .ok()
            .and_then(|v| v.clone())
            .unwrap_or_default();
        let controller = Self {
            dir,
            faults,
            proofs,
            lineage,
            _lease: lease,
            cursor: std::sync::Mutex::new(cursor),
        };
        if cursor_read.is_err() {
            let _ = controller.fault("cursor.json", "cursor_unavailable", false);
        }
        Ok(controller)
    }

    fn inventory(&self) -> anyhow::Result<Vec<String>> {
        let mut keys = std::collections::BTreeSet::new();
        for name in self.dir.entries()? {
            for suffix in [".request", ".record", ".admitted"] {
                if let Some(key) = name.strip_suffix(suffix) {
                    if valid_key(key) {
                        keys.insert(key.to_owned());
                    } else {
                        let _ = self.fault(&name, "invalid_filename", suffix != ".request");
                    }
                }
            }
        }
        ensure!(keys.len() <= MAX_OPERATIONS, "queue capacity");
        Ok(keys.into_iter().collect())
    }

    fn fault(&self, name: &str, reason: &str, authority: bool) -> anyhow::Result<()> {
        let stat = fs::statat(&self.dir.0, name, fs::AtFlags::SYMLINK_NOFOLLOW);
        let meta = match stat {
            Ok(s) => format!(
                "{}:{}:{}:{}:{}:{}",
                s.st_dev, s.st_ino, s.st_uid, s.st_mode, s.st_size, s.st_mtime
            ),
            Err(e) => format!("{e:?}"),
        };
        let bytes = self.dir.raw(name).ok().flatten();
        let mut binding = meta.into_bytes();
        if let Some(bytes) = &bytes {
            binding.extend_from_slice(bytes);
        }
        let fingerprint = sha256::Hash::hash(&binding).to_string();
        let key = sha256::Hash::hash(format!("{name}:{fingerprint}").as_bytes()).to_string();
        let path = format!("{key}.fault");
        if self.faults.present(&path)? {
            return Ok(());
        }
        ensure!(
            self.faults.names(".fault")?.len() < MAX_OPERATIONS,
            "fault evidence capacity"
        );
        self.faults.write(
            &path,
            &QueueFault {
                name_hex: hex::encode(name.as_bytes()),
                fingerprint,
                reason: reason.into(),
                authority_may_exist: authority,
                safe_bytes_hex: bytes.map(|b| hex::encode(&b[..b.len().min(65536)])),
            },
        )
    }

    /// Corruption is never evidence of no effect. Invalid never-admitted
    /// requests may be disposed, but any unreadable durable authority holds
    /// NEW funding globally; it does not prevent observations/recovery.
    fn unresolved(&self) -> anyhow::Result<bool> {
        for name in self.dir.entries()? {
            if name.ends_with(".record") {
                let record = self.dir.read::<Record>(&name)?.context("record absent")?;
                record.validate()?;
                ensure!(
                    name == format!("{}.record", record.request.key()),
                    "receipt filename binding"
                );
                if matches!(record.result.state, State::Unknown | State::Submitted) {
                    return Ok(true);
                }
            } else if let Some(key) = name.strip_suffix(".admitted") {
                let request = self
                    .dir
                    .read::<Request>(&name)?
                    .context("admission absent")?;
                request.validate()?;
                ensure!(
                    valid_key(key) && request.key() == key,
                    "admission filename binding"
                );
                let record = self.dir.read::<Record>(&format!("{key}.record"))?;
                if let Some(record) = record {
                    record.validate()?;
                    ensure!(record.request == request, "durable payload conflict");
                } else if request.action == Action::BoardFloat {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    fn next_keys(&self) -> anyhow::Result<Vec<String>> {
        let keys = self.inventory()?;
        let cursor = self
            .cursor
            .lock()
            .map_err(|_| anyhow::anyhow!("queue cursor"))?
            .clone();
        // Include terminal/corrupt entries in the budget and persist progress:
        // a poisoned low-sorting file cannot starve keys after it on restart.
        Ok(keys
            .iter()
            .filter(|k| **k > cursor)
            .chain(keys.iter().filter(|k| **k <= cursor))
            .take(8)
            .cloned()
            .collect())
    }

    fn advance(&self, key: &str) -> anyhow::Result<()> {
        self.dir.write("cursor.json", &key)?;
        *self
            .cursor
            .lock()
            .map_err(|_| anyhow::anyhow!("queue cursor"))? = key.into();
        Ok(())
    }

    #[cfg(test)]
    async fn step<W: WalletOwner>(&self, wallet: &W) -> anyhow::Result<()> {
        for key in self.next_keys()? {
            if self.process_key(wallet, &key).await.is_err() {
                self.fault(&format!("{key}.request"), "entry_unavailable", true)?;
            }
            self.advance(&key)?;
        }
        Ok(())
    }

    async fn reconcile<W: WalletOwner>(
        &self,
        wallet: &W,
        record: &mut Record,
    ) -> anyhow::Result<()> {
        if let (Some(prepared), Some(facts)) = (&record.prepared, &mut record.result.board) {
            wallet.protect(prepared).await?;
            match tokio::time::timeout(
                super::wallet_control_io::READ_DEADLINE,
                wallet.resolve(prepared, facts),
            )
            .await
            .context("read resolution deadline")
            .and_then(|r| r)
            {
                Ok(resolution) => {
                    if resolution.state != State::Unknown {
                        record.result.error_code = None;
                    }
                    record.result.state = resolution.state;
                    record.acceptance = resolution.acceptance;
                }
                Err(_) => {
                    record.result.state = State::Unknown;
                    if record.result.error_code.is_none() {
                        record.result.error_code = Some("board_resolution_unavailable".into());
                    }
                }
            }
            match Self::observe_read(wallet).await {
                Ok(obs) => obs.apply(&mut record.result)?,
                Err(_) => clear_observation(&mut record.result),
            }
            if record.result.state == State::Confirmed {
                let hash = self.retain_proof(record)?;
                record.proof_digest = Some(hash.clone());
                record.result.retained_proof_sha256 = Some(hash);
            }
            record.validate()?;
            self.dir
                .write(&format!("{}.record", record.request.key()), record)?;
            if record.result.state == State::Confirmed {
                wallet.release_confirmed(prepared).await?;
            } else {
                wallet.protect(prepared).await?;
            }
        }
        Ok(())
    }

    async fn process_key<W: WalletOwner>(&self, wallet: &W, key: &str) -> anyhow::Result<()> {
        let record_name = format!("{key}.record");
        let request_name = format!("{key}.request");
        let admitted_name = format!("{key}.admitted");
        if self.dir.present(&record_name)? {
            let mut record: Record =
                match self.dir.read(&record_name).and_then(|r: Option<Record>| {
                    let r = r.context("record absent")?;
                    r.validate()?;
                    ensure!(r.request.key() == key, "record binding");
                    Ok(r)
                }) {
                    Ok(record) => record,
                    Err(_) => {
                        self.fault(&record_name, "durable_record_unavailable", true)?;
                        return Ok(());
                    }
                };
            // The valid durable operation is authoritative even if its request
            // file is later unreadable or changed. Preserve evidence and recover.
            match self.dir.read::<Request>(&request_name) {
                Ok(Some(request)) if request == record.request => (),
                _ => {
                    let _ = self.fault(&request_name, "request_durable_mismatch", true);
                }
            }
            if self.dir.present(&admitted_name)? {
                match self.dir.read::<Request>(&admitted_name) {
                    Ok(Some(request)) if request == record.request => (),
                    _ => {
                        let _ = self.fault(&admitted_name, "admission_durable_mismatch", true);
                    }
                }
            }
            if matches!(record.result.state, State::Unknown | State::Submitted) {
                self.reconcile(wallet, &mut record).await?;
            }
            return Ok(());
        }
        if self.dir.present(&admitted_name)? {
            if let Ok(Some(request)) = self.dir.read::<Request>(&admitted_name) {
                if request.validate().is_ok()
                    && request.key() == key
                    && request.action == Action::BoardProofV1
                {
                    return self.save_proof_reply(&record_name, request);
                }
                if request.validate().is_ok()
                    && request.key() == key
                    && request.action == Action::Snapshot
                {
                    let mut result = ResultDto::new(&request);
                    match Self::observe_read(wallet).await {
                        Ok(obs) => {
                            obs.apply(&mut result)?;
                            result.state = State::Observed;
                        }
                        Err(_) => {
                            result.state = State::Rejected;
                            result.error_code = Some("fresh_observation_unavailable".into());
                        }
                    }
                    self.dir.write(
                        &record_name,
                        &Record {
                            request,
                            result,
                            prepared: None,
                            acceptance: None,
                            proof_digest: None,
                        },
                    )?;
                    return Ok(());
                }
            }
            // A crash/error after admission cannot be reinterpreted as a fresh
            // operation, even when no signed proposal can now be read.
            self.fault(&admitted_name, "admitted_without_receipt", true)?;
            return Ok(());
        }
        let request = match self.dir.read::<Request>(&request_name).and_then(|r| {
            let r = r.context("request absent")?;
            r.validate()?;
            ensure!(r.key() == key, "request filename mismatch");
            Ok(r)
        }) {
            Ok(request) => request,
            Err(_) => {
                self.fault(&request_name, "invalid_never_admitted_request", false)?;
                return Ok(());
            }
        };
        if request.action == Action::BoardProofV1 {
            self.dir.write(&admitted_name, &request)?;
            return self.save_proof_reply(&record_name, request);
        }
        let blocked = !matches!(self.unresolved(), Ok(false));
        // Durable admission precedes any key allocation. A later unexpected
        // preparation/validation failure never loops deriving fresh child keys.
        self.dir.write(&admitted_name, &request)?;
        let mut record = Record {
            result: ResultDto::new(&request),
            request,
            prepared: None,
            acceptance: None,
            proof_digest: None,
        };
        let observation = match Self::observe_read(wallet).await {
            Ok(obs) => {
                obs.apply(&mut record.result)?;
                obs
            }
            Err(_) => {
                record.result.state = State::Rejected;
                record.result.error_code = Some("fresh_observation_unavailable".into());
                self.dir.write(&record_name, &record)?;
                return Ok(());
            }
        };
        if record.request.action == Action::Snapshot {
            record.result.state = State::Observed;
        } else if observation.float >= record.request.net_target_sats.context("target")? {
            record.result.state = State::AlreadyAtTarget;
        } else if blocked || wallet.pending().await? {
            record.result.state = State::Rejected;
            record.result.error_code = Some("board_in_progress".into());
        } else {
            match wallet.prepare(&record.request, &observation).await {
                Err(_) => {
                    record.result.state = State::Rejected;
                    record.result.error_code = Some("funds_fee_or_policy_refused".into());
                }
                Ok((prepared, facts)) => {
                    let psbt = bitcoin::Psbt::deserialize(&hex::decode(&prepared.psbt_hex)?)?;
                    record.result.funding_txid = Some(psbt.unsigned_tx.compute_txid().to_string());
                    record.result.board = Some(facts);
                    record.prepared = Some(prepared);
                    record.result.state = State::Unknown;
                    record.validate()?;
                    self.dir.write(&record_name, &record)?;
                    let p = record.prepared.as_ref().context("proposal")?;
                    if wallet
                        .start(p, record.result.board.as_ref().context("facts")?)
                        .await
                        .is_err()
                    {
                        record.result.error_code = Some("stock_board_outcome_unknown".into());
                        self.dir.write(&record_name, &record)?;
                    }
                    self.reconcile(wallet, &mut record).await?;
                    return Ok(());
                }
            }
        }
        record.validate()?;
        self.dir.write(&record_name, &record)?;
        Ok(())
    }

    /// Protect only affected board VTXOs; unrelated refresh/recovery proceeds.
    async fn protect_receipts<W: WalletOwner>(&self, wallet: &W) -> anyhow::Result<()> {
        let mut failed = false;
        for name in self.dir.names(".record")? {
            let record = match self.dir.read::<Record>(&name) {
                Ok(Some(r)) if r.validate().is_ok() => r,
                _ => {
                    let _ = self.fault(&name, "durable_record_unavailable", true);
                    continue;
                }
            };
            if let Some(p) = &record.prepared {
                let result = if record.result.state == State::Confirmed {
                    wallet.release_confirmed(p).await
                } else {
                    wallet.protect(p).await
                };
                failed |= result.is_err();
            }
        }
        ensure!(!failed, "receipt protection incomplete");
        Ok(())
    }

    /// One production queue iteration: preserve the operation guard through
    /// outcome/fault persistence, then release it BEFORE cursor persistence.
    /// The cursor callback is synchronous work in production, but its future
    /// seam lets tests stop at this exact boundary without a wall-clock SLA.
    async fn process_key_with_capacity<W, A, F>(
        &self,
        wallet: &W,
        key: &str,
        operation_lock: &tokio::sync::Mutex<()>,
        advance: A,
    ) where
        W: WalletOwner,
        A: FnOnce() -> F,
        F: std::future::Future<Output = anyhow::Result<()>>,
    {
        {
            let _owner = operation_lock.lock().await;
            if self.process_key(wallet, key).await.is_err() {
                let _ = self.fault(&format!("{key}.request"), "entry_unavailable", true);
            }
        }
        if advance().await.is_err() {
            tracing::warn!("queue cursor unavailable");
        }
        tokio::task::yield_now().await;
    }

    pub async fn run<C, R>(
        self,
        wallet: StockOwner,
        boarding: super::board::BoardingLoop<C, super::bark_stock::BarkWalletClient, R>,
    ) where
        C: super::board::BoardChain,
        R: super::board::BoardRegistry,
    {
        use super::wallet_control_io::{MAINTENANCE_INTERVAL, SYNC_INTERVAL};
        let mut timer = tokio::time::interval(Duration::from_secs(2));
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut sync_due = tokio::time::Instant::now();
        let mut maintenance_due = tokio::time::Instant::now();
        loop {
            timer.tick().await;
            if tokio::time::Instant::now() >= sync_due {
                if wallet.sync_read_batch().await.is_err() {
                    tracing::warn!("bounded wallet read sync incomplete");
                }
                sync_due = tokio::time::Instant::now() + SYNC_INTERVAL;
                // Each independent span yields the wallet permit afterwards.
                {
                    let _owner = wallet.client.operation_lock.lock().await;
                    if self.recovery_pass(&wallet).await.is_err() {
                        tracing::warn!("stock board recovery or receipt protection unavailable");
                    }
                }
                {
                    let _owner = wallet.client.operation_lock.lock().await;
                    if boarding.scan_only().await.is_err() {
                        tracing::warn!("bounded pool output scan incomplete");
                    }
                }
            }
            match self.next_keys() {
                Ok(keys) => {
                    for key in keys {
                        self.process_key_with_capacity(
                            &wallet,
                            &key,
                            wallet.client.operation_lock.as_ref(),
                            || std::future::ready(self.advance(&key)),
                        )
                        .await;
                    }
                }
                Err(_) => tracing::warn!("queue inventory unavailable; observations deferred"),
            }
            if tokio::time::Instant::now() >= maintenance_due {
                // Protect unknown receipt inputs before refresh can change them.
                // No timeout cancels this effectful stock maintenance future.
                {
                    let _owner = wallet.client.operation_lock.lock().await;
                    if self.maintenance_pass(&wallet).await.is_err() {
                        tracing::warn!("stock maintenance or receipt protection incomplete");
                    }
                }
                maintenance_due = tokio::time::Instant::now() + MAINTENANCE_INTERVAL;
                if matches!(self.unresolved(), Ok(false)) {
                    let _owner = wallet.client.operation_lock.lock().await;
                    if matches!(wallet.pending().await, Ok(false))
                        && boarding.refill_only().await.is_err()
                    {
                        tracing::warn!("automatic board attempt unresolved");
                    }
                }
            }
        }
    }
}

fn valid_key(key: &str) -> bool {
    key.len() == 64
        && key
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
fn clear_observation(result: &mut ResultDto) {
    result.onchain_trusted_sats = None;
    result.float_spendable_sats = None;
    result.observed_at_unix_ms = None;
    result.observed_tip_height = None;
}

/// Adapter over the ONE wallet opened by the Ark role. No second seed/DB.
pub struct StockOwner {
    client: super::bark_stock::BarkWalletClient,
    rpc: std::sync::Arc<super::wallet_control_io::ReadRpc>,
    sync_rpc: std::sync::Arc<super::wallet_control_io::ReadRpc>,
    emitter: std::sync::Arc<std::sync::Mutex<Option<super::wallet_control_io::Emitter>>>,
    freshness: std::sync::Mutex<super::wallet_control_io::Freshness>,
    server: String,
    network: bitcoin::Network,
    ark_id: [u8; 4],
    min_remaining: u32,
    target_cap: u64,
}

impl StockOwner {
    async fn sync_read_batch(&self) -> anyhow::Result<()> {
        use bitcoincore_rpc::RpcApi;
        self.freshness
            .lock()
            .map_err(|_| anyhow::anyhow!("freshness mutex"))?
            .invalidate();
        let checkpoint = self.client.onchain.read().await.inner.latest_checkpoint();
        let emitter = self.emitter.clone();
        let rpc = self.sync_rpc.clone();
        // All RPC pagination is outside wallet send capacity, on a blocking
        // worker with real socket and total-batch limits. It cannot broadcast.
        let batch = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
            let mut slot = emitter
                .lock()
                .map_err(|_| anyhow::anyhow!("emitter mutex"))?;
            let emitter = slot.get_or_insert_with(|| {
                bdk_bitcoind_rpc::Emitter::new(
                    rpc.clone(),
                    checkpoint,
                    0,
                    bdk_bitcoind_rpc::NO_EXPECTED_MEMPOOL_TXS,
                )
            });
            Ok(super::wallet_control_io::read_batch(emitter, &rpc))
        })
        .await??;
        let _owner = self.client.operation_lock.lock().await;
        let mut onchain = self.client.onchain.write().await;
        for block in batch.blocks {
            if let Err(error) = onchain.inner.apply_block_connected_to(
                &block.block,
                block.block_height(),
                block.connected_to(),
            ) {
                *self
                    .emitter
                    .lock()
                    .map_err(|_| anyhow::anyhow!("emitter mutex"))? = None;
                return Err(error.into());
            }
        }
        // Exactly stock OnchainWallet::persist, using its public BDK stage and
        // the SAME stock persister. No independent onchain wallet/ledger.
        if let Some(stage) = onchain.inner.staged() {
            self.client.db.store_bdk_wallet_changeset(stage).await?;
            let _ = onchain.inner.take_staged();
        }
        if batch.complete {
            self.sync_rpc.reset();
            let hash = self.sync_rpc.get_best_block_hash()?;
            if onchain.inner.latest_checkpoint().hash() == hash {
                self.freshness
                    .lock()
                    .map_err(|_| anyhow::anyhow!("freshness mutex"))?
                    .complete(hash);
            }
        }
        Ok(())
    }

    async fn maintain_stock(&self) -> anyhow::Result<()> {
        let wallet = self.client.wallet();
        // Incremental stock round maintenance, not maintenance_refresh's
        // unbounded wait for the next interactive round. Delegated scheduling
        // uses the ordinary server API and stock locks/checkpoints. No whole
        // effectful future is timed out or cancelled.
        wallet.sync_pending_rounds().await?;
        wallet.progress_pending_rounds(None).await?;
        wallet
            .maybe_schedule_maintenance_refresh_delegated()
            .await?;
        // This last call is read-only fee estimation, so a deadline is safe.
        tokio::time::timeout(
            super::wallet_control_io::RPC_TIMEOUT,
            wallet
                .chain()
                .update_fee_rates(Some(bitcoin::FeeRate::from_sat_per_vb_u32(2))),
        )
        .await
        .context("fee observation timeout")??;
        Ok(())
    }

    fn protection_id(prepared: &Prepared) -> anyhow::Result<String> {
        let tx: bitcoin::Transaction =
            bitcoin::consensus::deserialize(&hex::decode(&prepared.signed_tx_hex)?)?;
        Ok(format!("wallet-control-board.{}", tx.compute_txid()))
    }

    async fn protect_client(
        client: &super::bark_stock::BarkWalletClient,
        prepared: &Prepared,
    ) -> anyhow::Result<()> {
        let tx: bitcoin::Transaction =
            bitcoin::consensus::deserialize(&hex::decode(&prepared.signed_tx_hex)?)?;
        let anchor = bitcoin::OutPoint::new(tx.compute_txid(), prepared.funding_vout);
        for vtxo in client.wallet().spendable_vtxos().await? {
            if vtxo.vtxo.chain_anchor() == anchor {
                ensure!(
                    vtxo.vtxo.user_pubkey().to_string() == prepared.user_pubkey,
                    "board protection key mismatch"
                );
                client
                    .wallet()
                    .lock_vtxos(
                        [&vtxo],
                        Some(bark::vtxo::VtxoLockHolder::Action {
                            id: Self::protection_id(prepared)?,
                        }),
                    )
                    .await?;
            }
        }
        Ok(())
    }

    pub fn new(
        client: super::bark_stock::BarkWalletClient,
        config: &crate::config::Config,
        network: bitcoin::Network,
        ark_id: [u8; 4],
    ) -> anyhow::Result<Self> {
        let cfg = config.ark.as_ref().context("ark configuration")?;
        let make_rpc = || {
            super::wallet_control_io::ReadRpc::new(
                &config.template.rpc_url,
                &config.template.rpc_user,
                &config.template.rpc_pass,
            )
            .map(std::sync::Arc::new)
        };
        let rpc = make_rpc()?;
        Ok(Self {
            client,
            rpc,
            sync_rpc: make_rpc()?,
            emitter: Default::default(),
            freshness: Default::default(),
            server: cfg.server.clone(),
            network,
            ark_id,
            min_remaining: cfg.min_remaining_blocks,
            target_cap: cfg.float_target_sats,
        })
    }

    async fn server(&self) -> anyhow::Result<(server_rpc::ServerConnection, ark::ArkInfo)> {
        let server = tokio::time::timeout(
            super::wallet_control_io::RPC_TIMEOUT,
            server_rpc::ServerConnection::builder()
                .address(self.server.clone())
                .network(self.network)
                .connect(),
        )
        .await
        .context("server read timeout")?
        .context("server observation unavailable")?;
        let info = server.ark_info().await;
        let props = self.client.wallet().properties().await?;
        ensure!(
            info.network == self.network
                && <[u8; 4]>::from(ark::address::ArkId::from_server_pubkey(info.server_pubkey))
                    == self.ark_id
                && props.server_pubkey == Some(info.server_pubkey)
                && props.server_mailbox_pubkey == Some(info.mailbox_pubkey),
            "foreign server"
        );
        info.fees.validate()?;
        Ok((server, info))
    }

    /// Only canonical, wallet-owned outputs with >=3 confirmations; coinbases
    /// additionally obey Bitcoin's 100-confirmation maturity. No balance cache.
    async fn owned_inputs(
        &self,
    ) -> anyhow::Result<(
        u32,
        bitcoin::BlockHash,
        Vec<(bitcoin::OutPoint, bitcoin::TxOut)>,
    )> {
        use bitcoincore_rpc::RpcApi;
        let tip = self.rpc.get_blockchain_info()?;
        ensure!(
            self.freshness
                .lock()
                .map_err(|_| anyhow::anyhow!("freshness mutex"))?
                .current(tip.best_block_hash),
            "wallet observation stale or partial"
        );
        let wallet = self.client.onchain.read().await;
        let checkpoint = wallet.inner.latest_checkpoint();
        ensure!(
            checkpoint.height() as u64 == tip.blocks && checkpoint.hash() == tip.best_block_hash,
            "wallet sync stale"
        );
        let unspent = wallet.list_unspent();
        ensure!(unspent.len() <= 1024, "onchain observation capacity");
        let mut outputs = Vec::new();
        for output in unspent {
            ensure!(
                wallet.inner.is_mine(output.txout.script_pubkey.clone()),
                "foreign onchain output"
            );
            if trusted_output(
                self.rpc.as_ref(),
                tip.best_block_hash,
                output.outpoint,
                &output.txout,
            )? {
                outputs.push((output.outpoint, output.txout));
            }
        }
        ensure!(
            self.rpc.get_best_block_hash()? == tip.best_block_hash,
            "chain moved during observation"
        );
        Ok((tip.blocks.try_into()?, tip.best_block_hash, outputs))
    }

    async fn proven_float(
        &self,
        server: &mut server_rpc::ServerConnection,
        info: &ark::ArkInfo,
        tip: u32,
    ) -> anyhow::Result<u64> {
        use ark::ProtocolEncoding;
        use bitcoincore_rpc::RpcApi;
        let wallet = self.client.wallet();
        let vtxos = wallet.spendable_vtxos().await?;
        ensure!(vtxos.len() <= 1024, "float observation capacity");
        let mut total = 0u64;
        for v in vtxos {
            let expiry = v.vtxo.expiry_height().to_u32();
            if expiry <= tip || expiry - tip < self.min_remaining {
                continue;
            }
            ensure!(
                v.vtxo.server_pubkey() == info.server_pubkey,
                "foreign float VTXO"
            );
            let full = wallet.get_full_vtxo(v.id()).await?;
            if full.exit_depth() >= info.max_vtxo_exit_depth {
                continue;
            }
            let key = wallet.get_vtxo_key(&full).await?;
            ensure!(
                *full.policy() == ark::VtxoPolicy::new_pubkey(key.public_key()),
                "unsupported float policy"
            );
            full.validate(
                &self
                    .rpc
                    .get_raw_transaction(&full.chain_anchor().txid, None)?,
            )?;
            let anchor = self
                .rpc
                .get_raw_transaction_info(&full.chain_anchor().txid, None)?;
            ensure!(
                anchor.txid == full.chain_anchor().txid,
                "float anchor txid mismatch"
            );
            ensure!(
                anchor.confirmations.is_some_and(
                    |n| n > 0 && u64::from(n) >= info.required_board_confirmations as u64
                ),
                "unconfirmed float anchor"
            );
            let attestation = ark::attestations::VtxoStatusAttestation::new(v.id(), &key);
            let status = tokio::time::timeout(
                super::wallet_control_io::RPC_TIMEOUT,
                server
                    .client
                    .get_vtxo_status(server_rpc::protos::GetVtxoStatusRequest {
                        vtxo_id: v.id().to_bytes().to_vec(),
                        attestation: ProtocolEncoding::serialize(&attestation),
                    }),
            )
            .await
            .context("VTXO status read timeout")??
            .into_inner();
            ensure!(
                status.spend_state == server_rpc::protos::VtxoSpendState::Spendable as i32,
                "float server state unavailable"
            );
            total = total
                .checked_add(v.vtxo.amount().to_sat())
                .context("float overflow")?;
        }
        Ok(total)
    }
}

/// A real node proof for an already wallet-owned output; never a scan/address
/// derivation or balance injection. A changed tip or altered prevout is an error.
fn trusted_output(
    rpc: &impl bitcoincore_rpc::RpcApi,
    tip: bitcoin::BlockHash,
    outpoint: bitcoin::OutPoint,
    expected: &bitcoin::TxOut,
) -> anyhow::Result<bool> {
    let Some(proof) = rpc.get_tx_out(&outpoint.txid, outpoint.vout, Some(true))? else {
        return Ok(false);
    };
    ensure!(proof.bestblock == tip, "chain moved during observation");
    ensure!(
        proof.value == expected.value
            && proof.script_pub_key.hex == expected.script_pubkey.as_bytes(),
        "onchain proof mismatch"
    );
    Ok(proof.confirmations >= 3 && (!proof.coinbase || proof.confirmations >= 100))
}

/// Exact inverse of the stock fee function. The fee schedule must be valid;
/// refuse unrepresentable exact net targets rather than silently overfill.
fn gross_for_net(net: u64, cap: u64, fees: &ark::fees::BoardFees) -> anyhow::Result<(u64, u64)> {
    let mut low = net;
    let mut high = net
        .checked_add(cap)
        .context("amount overflow")?
        .min(bitcoin::Amount::MAX_MONEY.to_sat());
    while low < high {
        let mid = low + (high - low) / 2;
        let fee = fees
            .calculate(bitcoin::Amount::from_sat(mid))
            .context("fee overflow")?
            .to_sat();
        if mid.saturating_sub(fee) < net {
            low = mid + 1;
        } else {
            high = mid;
        }
    }
    let fee = fees
        .calculate(bitcoin::Amount::from_sat(low))
        .context("fee overflow")?
        .to_sat();
    ensure!(
        low.checked_sub(fee) == Some(net) && fee <= cap,
        "fee cap or net target"
    );
    Ok((low, fee))
}

#[async_trait::async_trait]
impl WalletOwner for StockOwner {
    async fn round_imports(&self) -> anyhow::Result<Vec<RoundImportFacts>> {
        let movements = self.client.wallet().history().await?;
        ensure!(
            movements.len() <= MAX_OPERATIONS,
            "stock movement read capacity"
        );
        let mut result = Vec::new();
        for movement in movements {
            if movement.status != bark::movement::MovementStatus::Successful
                || movement.subsystem.name != "bark.round"
                || movement.subsystem.kind != "refresh"
                || movement.input_vtxos.is_empty()
                || movement.output_vtxos.is_empty()
            {
                continue;
            }
            let Some(completed) = movement.time.completed_at else {
                continue;
            };
            let Some(funding) = movement
                .metadata
                .get("funding_txid")
                .and_then(serde_json::Value::as_str)
            else {
                continue;
            };
            let txid: bitcoin::Txid = funding.parse()?;
            let mut inputs: Vec<_> = movement
                .input_vtxos
                .iter()
                .map(ToString::to_string)
                .collect();
            inputs.sort();
            let mut outputs: Vec<_> = movement
                .output_vtxos
                .iter()
                .map(ToString::to_string)
                .collect();
            outputs.sort();
            let facts = RoundImportFacts {
                schema: super::wallet_control_proof::ROUND_SCHEMA.into(),
                stock_movement_id: movement.id.0,
                input_vtxo_ids: inputs,
                output_vtxo_ids: outputs,
                round_funding_txid: txid.to_string(),
                offchain_fee_sats: movement.offchain_fee.to_sat(),
                completed_at_unix_ms: completed.timestamp_millis().try_into()?,
            };
            facts.validate()?;
            result.push(facts);
        }
        Ok(result)
    }
    async fn recover(&self) -> anyhow::Result<()> {
        self.client.wallet().sync_pending_boards().await
    }
    async fn maintain(&self) -> anyhow::Result<()> {
        self.maintain_stock().await
    }
    async fn observe(&self) -> anyhow::Result<Observation> {
        use bitcoincore_rpc::RpcApi;
        self.rpc.reset();
        let (mut server, info) = self.server().await?;
        let (tip, hash, outputs) = self.owned_inputs().await?;
        let onchain = outputs
            .iter()
            .try_fold(0u64, |n, (_, o)| n.checked_add(o.value.to_sat()))
            .context("onchain overflow")?;
        let float = self.proven_float(&mut server, &info, tip).await?;
        ensure!(
            self.rpc.get_best_block_hash()? == hash,
            "chain moved during observation"
        );
        Ok(Observation {
            onchain,
            float,
            tip,
        })
    }

    async fn pending(&self) -> anyhow::Result<bool> {
        Ok(!self.client.wallet().pending_boards().await?.is_empty()
            || !self
                .client
                .db
                .get_pending_round_state_ids()
                .await?
                .is_empty())
    }

    async fn prepare(
        &self,
        request: &Request,
        observation: &Observation,
    ) -> anyhow::Result<(Prepared, BoardFacts)> {
        use bark::onchain::OnchainWalletTrait;
        use bitcoincore_rpc::RpcApi;
        self.rpc.reset();
        let (_, info) = self.server().await?;
        let stock_info = self.client.wallet().require_ark_info().await?;
        ensure!(stock_info == info, "stock policy snapshot stale");
        let target = request.net_target_sats.context("target")?;
        ensure!(
            target <= self.target_cap,
            "target exceeds configured float cap"
        );
        let net = target
            .checked_sub(observation.float)
            .context("target already reached")?;
        let cap = request.max_fee_sats.context("fee cap")?;
        let (gross, board_fee) = gross_for_net(net, cap, &info.fees.board)?;
        ark::fees::validate_and_subtract_fee(
            bitcoin::Amount::from_sat(gross),
            bitcoin::Amount::from_sat(board_fee),
        )?;
        ensure!(
            gross >= info.min_board_amount.to_sat()
                && info.max_vtxo_amount.is_none_or(|m| net <= m.to_sat())
                && info.vtxo_lifetime.to_u32() >= self.min_remaining,
            "board server policy"
        );
        let (tip, hash, outputs) = self.owned_inputs().await?;
        ensure!(tip == observation.tip, "observation stale");
        let total = outputs
            .iter()
            .try_fold(0u64, |n, (_, o)| n.checked_add(o.value.to_sat()))
            .context("input overflow")?;
        ensure!(
            total > gross && !outputs.is_empty(),
            "insufficient confirmed wallet funds"
        );
        let (key, key_index) = self.client.wallet().peek_next_keypair().await?;
        let expiry = tip
            .checked_add(info.vtxo_lifetime.to_u32())
            .context("expiry overflow")?
            .into();
        let expected = ark::board::BoardBuilder::new(
            key.public_key(),
            expiry,
            info.server_pubkey,
            info.vtxo_exit_delta,
        );
        let address =
            bitcoin::Address::from_script(&expected.funding_script_pubkey(), self.network)?;
        let fee_rate = self.client.wallet().chain().fee_rates().await.regular;
        let psbt = {
            let mut onchain = self.client.onchain.write().await;
            let allowed: std::collections::HashSet<_> = outputs.iter().map(|(p, _)| *p).collect();
            let forbidden: Vec<_> = onchain
                .list_unspent()
                .iter()
                .map(|o| o.outpoint)
                .filter(|p| !allowed.contains(p))
                .collect();
            let mut builder = onchain.inner.build_tx();
            builder.add_recipient(address.script_pubkey(), bitcoin::Amount::from_sat(gross));
            builder.unspendable(forbidden);
            builder.fee_rate(fee_rate);
            let psbt = builder.finish()?;
            ensure!(
                psbt.inputs.len() <= server_rpc::MAX_NB_BOARD_FUNDING_INPUTS,
                "too many funding inputs"
            );
            ensure!(
                psbt.unsigned_tx
                    .input
                    .iter()
                    .all(|i| allowed.contains(&i.previous_output)),
                "unproven funding input"
            );
            onchain.sign_psbt(psbt).await?
        };
        let onchain_fee = psbt.fee()?.to_sat();
        let actual_fee = board_fee.checked_add(onchain_fee).context("fee overflow")?;
        ensure!(actual_fee <= cap, "total fee cap");
        ensure!(
            psbt.inputs.iter().all(|i| i.final_script_witness.is_some()),
            "funding not finalized"
        );
        let signed = psbt.clone().extract_tx()?;
        let funding_vout: u32 = signed
            .output
            .iter()
            .position(|o| o.script_pubkey == address.script_pubkey())
            .context("board output missing")?
            .try_into()?;
        // Last pre-effect check binds each selected prevout's script/value and
        // canonical maturity to the signed transaction, after fee calculation.
        for (input, data) in signed.input.iter().zip(psbt.inputs.iter()) {
            let proof = self
                .rpc
                .get_tx_out(
                    &input.previous_output.txid,
                    input.previous_output.vout,
                    Some(true),
                )?
                .context("funding input spent")?;
            let prev = data
                .witness_utxo
                .as_ref()
                .context("funding prevout missing")?;
            ensure!(
                proof.bestblock == hash
                    && proof.value == prev.value
                    && proof.script_pub_key.hex == prev.script_pubkey.as_bytes()
                    && proof.confirmations >= 3
                    && (!proof.coinbase || proof.confirmations >= 100),
                "funding proof stale"
            );
        }
        ensure!(
            self.rpc.get_best_block_hash()? == hash,
            "chain moved during preparation"
        );
        // Only definitely eligible input/fee proposals consume a stock index.
        self.client
            .db
            .store_vtxo_key(key_index, key.public_key())
            .await?;
        let facts = BoardFacts {
            gross_board_sats: gross,
            net_board_sats: net,
            board_fee_sats: board_fee,
            onchain_fee_sats: onchain_fee,
            actual_fee_sats: actual_fee,
            expiry_height: expiry.to_u32(),
            required_confirmations: info.required_board_confirmations.try_into()?,
            server_pubkey: info.server_pubkey.to_string(),
            vtxo_exit_delta: info.vtxo_exit_delta.to_u32(),
            min_board_sats: info.min_board_amount.to_sat(),
            max_vtxo_sats: info.max_vtxo_amount.map(|a| a.to_sat()),
            vtxo_id: None,
        };
        let prepared = Prepared {
            psbt_hex: hex::encode(psbt.serialize()),
            signed_tx_hex: hex::encode(bitcoin::consensus::serialize(&signed)),
            key_index,
            user_pubkey: key.public_key().to_string(),
            funding_vout,
            policy_json: serde_json::to_string(&info.fees.board)?,
            inputs: signed
                .input
                .iter()
                .map(|i| i.previous_output.to_string())
                .collect(),
            float_before_sats: observation.float,
            observed_tip_height: observation.tip,
        };
        Ok((prepared, facts))
    }

    async fn start(&self, prepared: &Prepared, facts: &BoardFacts) -> anyhow::Result<()> {
        // stock board_psbt persists Board::Broadcasting with the finalized PSBT
        // and signed VTXO before it calls the broadcaster. Its initial drive is
        // best-effort; an Ok here is NOT evidence of spendability/submission.
        let public = prepared.user_pubkey.parse()?;
        let (index, key) = self
            .client
            .wallet()
            .pubkey_keypair(&public)
            .await?
            .context("stock board key absent")?;
        ensure!(index == prepared.key_index, "board key index mismatch");
        ensure!(
            key.public_key().to_string() == prepared.user_pubkey,
            "board key mismatch"
        );
        let info = self.client.wallet().require_ark_info().await?;
        ensure!(
            info.server_pubkey.to_string() == facts.server_pubkey
                && serde_json::to_string(&info.fees.board)? == prepared.policy_json
                && info.vtxo_exit_delta.to_u32() == facts.vtxo_exit_delta
                && info.min_board_amount.to_sat() == facts.min_board_sats
                && info.max_vtxo_amount.map(|a| a.to_sat()) == facts.max_vtxo_sats
                && info.required_board_confirmations as u64 == facts.required_confirmations,
            "stock board policy changed"
        );
        let psbt = bitcoin::Psbt::deserialize(&hex::decode(&prepared.psbt_hex)?)?;
        ensure!(
            hex::encode(bitcoin::consensus::serialize(&psbt.clone().extract_tx()?))
                == prepared.signed_tx_hex,
            "signed funding mismatch"
        );
        self.client
            .wallet()
            .board_psbt(psbt, key, facts.expiry_height.into())
            .await?;
        Ok(())
    }

    async fn protect(&self, prepared: &Prepared) -> anyhow::Result<()> {
        Self::protect_client(&self.client, prepared).await
    }

    async fn release_confirmed(&self, prepared: &Prepared) -> anyhow::Result<()> {
        self.client
            .wallet()
            .release_action_locks(&Self::protection_id(prepared)?)
            .await?;
        Ok(())
    }

    async fn resolve(
        &self,
        prepared: &Prepared,
        facts: &mut BoardFacts,
    ) -> anyhow::Result<Resolution> {
        use ark::ProtocolEncoding;
        use bitcoincore_rpc::RpcApi;
        self.rpc.reset();
        let (mut server, info) = self.server().await?;
        ensure!(
            info.server_pubkey.to_string() == facts.server_pubkey,
            "foreign board server"
        );
        let tx: bitcoin::Transaction =
            bitcoin::consensus::deserialize(&hex::decode(&prepared.signed_tx_hex)?)?;
        let txid = tx.compute_txid();
        let checkpoints = self.client.wallet().pending_boards().await?;
        let pending = checkpoints
            .iter()
            .find(|b| b.funding_tx.compute_txid() == txid);
        let observation_hash = self.rpc.get_best_block_hash()?;
        let chain_tx = match self.rpc.get_raw_transaction_info(&txid, None) {
            Ok(tx) => tx,
            Err(_) => return Ok(Resolution::pending(State::Unknown)),
        };
        ensure!(
            chain_tx.txid == txid && chain_tx.hex == bitcoin::consensus::serialize(&tx),
            "funding transaction mismatch"
        );
        if let Some(board) = pending {
            ensure!(
                board.amount.to_sat() == facts.net_board_sats && board.vtxos.len() == 1,
                "stock pending board mismatch"
            );
            facts.vtxo_id = Some(board.vtxos[0].to_string());
            return Ok(Resolution::pending(State::Submitted));
        }
        // Resolve a lost reply even after the stock action has completed and
        // deleted its checkpoint: full wallet VTXO anchored at the bound txid.
        let wallet = self.client.wallet();
        let entries = if let Some(id) = &facts.vtxo_id {
            vec![wallet.get_vtxo_by_id(id.parse()?).await?]
        } else {
            wallet.all_vtxos().await?
        };
        ensure!(entries.len() <= 1024, "board observation capacity");
        for entry in entries {
            let vtxo = wallet.get_full_vtxo(entry.id()).await?;
            if vtxo.chain_anchor() != bitcoin::OutPoint::new(txid, prepared.funding_vout) {
                continue;
            }
            let protection = Self::protection_id(prepared)?;
            let available = matches!(&entry.state, bark::vtxo::VtxoState::Spendable)
                || matches!(&entry.state, bark::vtxo::VtxoState::Locked { holder: Some(bark::vtxo::VtxoLockHolder::Action { id }) } if id == &protection);
            if !available {
                return Ok(Resolution::pending(State::Unknown));
            }
            ensure!(
                vtxo.user_pubkey().to_string() == prepared.user_pubkey
                    && vtxo.amount().to_sat() == facts.net_board_sats
                    && vtxo.server_pubkey().to_string() == facts.server_pubkey
                    && vtxo.expiry_height().to_u32() == facts.expiry_height,
                "confirmed board mismatch"
            );
            ensure!(
                vtxo.exit_depth() < info.max_vtxo_exit_depth,
                "board exit depth insufficient"
            );
            ensure!(
                chain_tx
                    .confirmations
                    .is_some_and(|n| n > 0 && u64::from(n) >= facts.required_confirmations),
                "board not confirmed"
            );
            let tip: u32 = self.rpc.get_block_count()?.try_into()?;
            let tip_hash = self.rpc.get_best_block_hash()?;
            ensure!(
                tip_hash == observation_hash,
                "board observation chain changed"
            );
            vtxo.validate(&tx)?;
            let key = wallet.get_vtxo_key(&vtxo).await?;
            ensure!(
                *vtxo.policy() == ark::VtxoPolicy::new_pubkey(key.public_key()),
                "board policy mismatch"
            );
            let attestation = ark::attestations::VtxoStatusAttestation::new(vtxo.id(), &key);
            let status = tokio::time::timeout(
                super::wallet_control_io::RPC_TIMEOUT,
                server
                    .client
                    .get_vtxo_status(server_rpc::protos::GetVtxoStatusRequest {
                        vtxo_id: vtxo.id().to_bytes().to_vec(),
                        attestation: ProtocolEncoding::serialize(&attestation),
                    }),
            )
            .await
            .context("board status read timeout")??;
            ensure!(
                status.into_inner().spend_state
                    == server_rpc::protos::VtxoSpendState::Spendable as i32,
                "board server state unavailable"
            );
            ensure!(
                self.rpc.get_best_block_hash()? == tip_hash,
                "board observation chain changed"
            );
            ensure!(
                facts.expiry_height > tip && facts.expiry_height - tip >= self.min_remaining,
                "board lifetime insufficient"
            );
            facts.vtxo_id = Some(vtxo.id().to_string());
            return Ok(Resolution {
                state: State::Confirmed,
                acceptance: Some(AcceptanceProof {
                    full_vtxo_hex: hex::encode(ProtocolEncoding::serialize(&vtxo)),
                    checked_at_unix_ms: SystemTime::now()
                        .duration_since(UNIX_EPOCH)?
                        .as_millis()
                        .try_into()?,
                    checked_tip_height: tip,
                    funding_confirmations: chain_tx
                        .confirmations
                        .context("confirmation proof")?
                        .into(),
                }),
            });
        }
        Ok(Resolution::pending(State::Unknown))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::{Arc, Mutex};

    fn dir() -> std::path::PathBuf {
        let path = Path::new("/tmp/opencode").join(format!(
            "pool-wallet-control-test-{}",
            hex::encode(rand::random::<[u8; 8]>())
        ));
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    fn request(id: &str, action: Action) -> Request {
        Request {
            schema: REQUEST_SCHEMA.into(),
            operation_id: id.into(),
            action,
            net_target_sats: (action == Action::BoardFloat).then_some(115_000_000),
            max_fee_sats: (action == Action::BoardFloat).then_some(5_000_000),
            original_board_operation_id: None,
        }
    }

    fn proposal() -> (Prepared, BoardFacts) {
        use bitcoin::{Amount, OutPoint, Psbt, Transaction, TxIn, TxOut};
        let user = secp256k1::Keypair::from_secret_key(
            &secp256k1::Secp256k1::new(),
            &secp256k1::SecretKey::from_slice(&[2; 32]).unwrap(),
        );
        let server = secp256k1::Keypair::from_secret_key(
            &secp256k1::Secp256k1::new(),
            &secp256k1::SecretKey::from_slice(&[3; 32]).unwrap(),
        );
        let board = ark::board::BoardBuilder::new(
            user.public_key(),
            500u32.into(),
            server.public_key(),
            10u16.into(),
        );
        let tx = Transaction {
            version: bitcoin::transaction::Version::TWO,
            lock_time: bitcoin::absolute::LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::new(bitcoin::Txid::from_byte_array([1; 32]), 0),
                ..Default::default()
            }],
            output: vec![TxOut {
                value: Amount::from_sat(115_000_100),
                script_pubkey: board.funding_script_pubkey(),
            }],
        };
        let mut psbt = Psbt::from_unsigned_tx(tx).unwrap();
        psbt.inputs[0].witness_utxo = Some(TxOut {
            value: Amount::from_sat(115_000_200),
            script_pubkey: bitcoin::ScriptBuf::new(),
        });
        psbt.inputs[0].final_script_witness = Some(bitcoin::Witness::from_slice(&[[2u8; 64]]));
        let signed = psbt.clone().extract_tx().unwrap();
        (
            Prepared {
                psbt_hex: hex::encode(psbt.serialize()),
                signed_tx_hex: hex::encode(bitcoin::consensus::serialize(&signed)),
                key_index: 7,
                user_pubkey: user.public_key().to_string(),
                funding_vout: 0,
                policy_json: r#"{"min_fee_sat":100,"base_fee_sat":0,"ppm":0}"#.into(),
                inputs: vec![signed.input[0].previous_output.to_string()],
                float_before_sats: 0,
                observed_tip_height: 105,
            },
            BoardFacts {
                gross_board_sats: 115_000_100,
                net_board_sats: 115_000_000,
                board_fee_sats: 100,
                onchain_fee_sats: 100,
                actual_fee_sats: 200,
                expiry_height: 500,
                required_confirmations: 3,
                server_pubkey: server.public_key().to_string(),
                vtxo_exit_delta: 10,
                min_board_sats: 20_000,
                max_vtxo_sats: None,
                vtxo_id: None,
            },
        )
    }

    fn acceptance(prepared: &Prepared, facts: &mut BoardFacts) -> AcceptanceProof {
        use ark::ProtocolEncoding;
        let user = secp256k1::Keypair::from_secret_key(
            &secp256k1::Secp256k1::new(),
            &secp256k1::SecretKey::from_slice(&[2; 32]).unwrap(),
        );
        let server = secp256k1::Keypair::from_secret_key(
            &secp256k1::Secp256k1::new(),
            &secp256k1::SecretKey::from_slice(&[3; 32]).unwrap(),
        );
        let tx: bitcoin::Transaction =
            bitcoin::consensus::deserialize(&hex::decode(&prepared.signed_tx_hex).unwrap())
                .unwrap();
        let utxo = bitcoin::OutPoint::new(tx.compute_txid(), prepared.funding_vout);
        let builder = ark::board::BoardBuilder::new(
            user.public_key(),
            facts.expiry_height.into(),
            server.public_key(),
            10u16.into(),
        )
        .set_funding_details(
            bitcoin::Amount::from_sat(facts.gross_board_sats),
            bitcoin::Amount::from_sat(facts.board_fee_sats),
            utxo,
        )
        .unwrap()
        .generate_user_nonces();
        let cosigner = ark::board::BoardBuilder::new_for_cosign(
            user.public_key(),
            facts.expiry_height.into(),
            server.public_key(),
            10u16.into(),
            bitcoin::Amount::from_sat(facts.gross_board_sats),
            bitcoin::Amount::from_sat(facts.board_fee_sats),
            utxo,
            *builder.user_pub_nonce(),
        );
        let vtxo = builder
            .build_vtxo(&cosigner.server_cosign(&server), &user)
            .unwrap();
        vtxo.validate(&tx).unwrap();
        facts.vtxo_id = Some(vtxo.id().to_string());
        AcceptanceProof {
            full_vtxo_hex: hex::encode(ProtocolEncoding::serialize(&vtxo)),
            checked_at_unix_ms: 1000,
            checked_tip_height: 105,
            funding_confirmations: 3,
        }
    }

    #[derive(Default)]
    struct SimState {
        starts: usize,
        prepares: usize,
        observations: usize,
        pending: bool,
        round_pending: bool,
        confirmed: bool,
        fail_after_broadcast: bool,
        stale: bool,
        insufficient: bool,
        float: u64,
        must_find_checkpoint: Option<std::path::PathBuf>,
        protected: bool,
        maintenance: usize,
        recovery: usize,
        overdue_refreshed: usize,
        board_refreshed: bool,
        stall_observation: bool,
        round_imports: Vec<RoundImportFacts>,
        observation_probe: Option<Arc<ObservationProbe>>,
    }

    #[derive(Default)]
    struct ObservationProbe {
        entered: tokio::sync::Notify,
        attempts: std::sync::atomic::AtomicUsize,
        dropped: std::sync::atomic::AtomicBool,
    }

    struct ObservationDrop(Arc<ObservationProbe>);
    impl Drop for ObservationDrop {
        fn drop(&mut self) {
            self.0
                .dropped
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    struct Sim(Arc<Mutex<SimState>>);
    impl Sim {
        fn new() -> Self {
            Self(Arc::new(Mutex::new(SimState::default())))
        }
    }

    #[async_trait::async_trait]
    impl WalletOwner for Sim {
        async fn round_imports(&self) -> anyhow::Result<Vec<RoundImportFacts>> {
            Ok(self.0.lock().unwrap().round_imports.clone())
        }
        async fn observe(&self) -> anyhow::Result<Observation> {
            let (stall, probe) = {
                let state = self.0.lock().unwrap();
                (state.stall_observation, state.observation_probe.clone())
            };
            let _drop = probe.as_ref().map(|probe| ObservationDrop(probe.clone()));
            if let Some(probe) = &probe {
                probe
                    .attempts
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                probe.entered.notify_one();
            }
            if stall {
                std::future::pending::<()>().await;
            }
            let mut s = self.0.lock().unwrap();
            s.observations += 1;
            ensure!(!s.stale, "stale or foreign server");
            Ok(Observation {
                onchain: if s.insufficient { 0 } else { 120_000_000 },
                float: s.float,
                tip: 105,
            })
        }
        async fn pending(&self) -> anyhow::Result<bool> {
            let state = self.0.lock().unwrap();
            Ok(state.pending || state.round_pending)
        }
        async fn prepare(
            &self,
            _: &Request,
            _: &Observation,
        ) -> anyhow::Result<(Prepared, BoardFacts)> {
            let mut s = self.0.lock().unwrap();
            s.prepares += 1;
            ensure!(!s.insufficient, "insufficient funds");
            Ok(proposal())
        }
        async fn start(&self, prepared: &Prepared, _: &BoardFacts) -> anyhow::Result<()> {
            let mut s = self.0.lock().unwrap();
            if let Some(path) = &s.must_find_checkpoint {
                let dir = PrivateDir::open(path)?.child(CONTROL_DIR)?;
                let names = dir.names(".record")?;
                assert_eq!(
                    names.len(),
                    1,
                    "checkpoint must exist before simulated broadcast"
                );
                let record: Record = dir.read(&names[0])?.unwrap();
                record.validate()?;
                assert_eq!(
                    record.prepared.unwrap().signed_tx_hex,
                    prepared.signed_tx_hex
                );
            }
            s.starts += 1;
            s.pending = true;
            ensure!(!s.fail_after_broadcast, "RPC reply lost after broadcast");
            Ok(())
        }
        async fn resolve(
            &self,
            prepared: &Prepared,
            facts: &mut BoardFacts,
        ) -> anyhow::Result<Resolution> {
            let s = self.0.lock().unwrap();
            ensure!(!s.fail_after_broadcast, "resolution unavailable");
            Ok(if s.confirmed && !s.board_refreshed {
                Resolution {
                    state: State::Confirmed,
                    acceptance: Some(acceptance(prepared, facts)),
                }
            } else if s.pending {
                Resolution::pending(State::Submitted)
            } else {
                Resolution::pending(State::Unknown)
            })
        }
        async fn protect(&self, _: &Prepared) -> anyhow::Result<()> {
            self.0.lock().unwrap().protected = true;
            Ok(())
        }
        async fn release_confirmed(&self, _: &Prepared) -> anyhow::Result<()> {
            self.0.lock().unwrap().protected = false;
            Ok(())
        }
        async fn recover(&self) -> anyhow::Result<()> {
            self.0.lock().unwrap().recovery += 1;
            Ok(())
        }
        async fn maintain(&self) -> anyhow::Result<()> {
            let mut s = self.0.lock().unwrap();
            s.maintenance += 1;
            s.overdue_refreshed += 1;
            if s.confirmed && !s.protected {
                s.board_refreshed = true;
            }
            Ok(())
        }
    }

    fn enqueue(c: &Controller, request: &Request) {
        c.dir
            .write(&format!("{}.request", request.key()), request)
            .unwrap();
    }
    fn result(c: &Controller, request: &Request) -> Record {
        c.dir
            .read(&format!("{}.record", request.key()))
            .unwrap()
            .unwrap()
    }

    #[tokio::test]
    async fn real_queue_live_worker_client_and_immutable_replay() {
        let path = dir();
        let controller = Controller::open(&path).unwrap();
        let sim = Sim::new();
        let req = request("ap5-one", Action::BoardFloat);
        sim.0.lock().unwrap().must_find_checkpoint = Some(path.clone());
        enqueue(&controller, &req);
        controller.step(&sim).await.unwrap();
        assert_eq!(submit(&path, &req).unwrap().state, State::Submitted);
        controller.step(&sim).await.unwrap();
        assert_eq!(sim.0.lock().unwrap().starts, 1);
        let mut changed = req.clone();
        changed.max_fee_sats = Some(4);
        assert!(submit(&path, &changed)
            .unwrap_err()
            .to_string()
            .contains("conflict"));
        assert_eq!(sim.0.lock().unwrap().starts, 1);
        let raw = serde_json::to_value(submit(&path, &req).unwrap()).unwrap();
        assert!(raw["onchain_trusted_sats"].is_u64());
        assert_eq!(raw["schema"], RESULT_SCHEMA);
    }

    #[tokio::test]
    async fn after_broadcast_lost_reply_reopens_from_stock_state_without_resending() {
        let path = dir();
        let controller = Controller::open(&path).unwrap();
        let sim = Sim::new();
        sim.0.lock().unwrap().fail_after_broadcast = true;
        let req = request("lost", Action::BoardFloat);
        enqueue(&controller, &req);
        controller.step(&sim).await.unwrap();
        assert_eq!(result(&controller, &req).result.state, State::Unknown);
        drop(controller);
        let reopened = Controller::open(&path).unwrap();
        {
            let mut s = sim.0.lock().unwrap();
            s.pending = false;
            s.confirmed = true;
            s.fail_after_broadcast = false;
            s.float = 115_000_000;
        }
        reopened.step(&sim).await.unwrap();
        let r = result(&reopened, &req);
        assert_eq!(r.result.state, State::Confirmed);
        assert_eq!(r.result.float_spendable_sats, Some(115_000_000));
        assert_eq!(sim.0.lock().unwrap().starts, 1);
    }

    #[tokio::test]
    async fn before_broadcast_crash_retains_signed_proposal_and_blocks_new_board() {
        let path = dir();
        let controller = Controller::open(&path).unwrap();
        let req = request("before", Action::BoardFloat);
        enqueue(&controller, &req);
        let (prepared, facts) = proposal();
        let mut dto = ResultDto::new(&req);
        dto.state = State::Unknown;
        dto.board = Some(facts);
        dto.funding_txid = Some(
            bitcoin::Psbt::deserialize(&hex::decode(&prepared.psbt_hex).unwrap())
                .unwrap()
                .unsigned_tx
                .compute_txid()
                .to_string(),
        );
        controller
            .dir
            .write(
                &format!("{}.record", req.key()),
                &Record {
                    request: req.clone(),
                    result: dto,
                    prepared: Some(prepared.clone()),
                    acceptance: None,
                    proof_digest: None,
                },
            )
            .unwrap();
        drop(controller);
        let c = Controller::open(&path).unwrap();
        let sim = Sim::new();
        let next = request("fresh-id-must-not-resubmit", Action::BoardFloat);
        enqueue(&c, &next);
        c.step(&sim).await.unwrap();
        assert_eq!(
            result(&c, &req).prepared.unwrap().signed_tx_hex,
            prepared.signed_tx_hex
        );
        assert_eq!(
            result(&c, &next).result.error_code.as_deref(),
            Some("board_in_progress")
        );
        assert_eq!(sim.0.lock().unwrap().starts, 0);
        assert!(c.unresolved().unwrap());
    }

    #[tokio::test]
    async fn automatic_pending_capacity_and_two_manual_requests_never_reuse_inputs() {
        let path = dir();
        let c = Controller::open(&path).unwrap();
        let sim = Sim::new();
        sim.0.lock().unwrap().pending = true; // actual automatic board already accepted
        let first = request("auto-reserved", Action::BoardFloat);
        enqueue(&c, &first);
        c.step(&sim).await.unwrap();
        assert_eq!(sim.0.lock().unwrap().starts, 0);
        assert_eq!(
            result(&c, &first).result.error_code.as_deref(),
            Some("board_in_progress")
        );
        sim.0.lock().unwrap().pending = false;
        enqueue(&c, &request("manual-1", Action::BoardFloat));
        enqueue(&c, &request("manual-2", Action::BoardFloat));
        c.step(&sim).await.unwrap();
        assert_eq!(sim.0.lock().unwrap().starts, 1);
    }

    #[tokio::test]
    async fn fresh_snapshots_do_not_prepare_start_or_derive_and_noop_has_no_txid() {
        let path = dir();
        let c = Controller::open(&path).unwrap();
        let sim = Sim::new();
        let first = request("fresh1", Action::Snapshot);
        enqueue(&c, &first);
        c.step(&sim).await.unwrap();
        sim.0.lock().unwrap().float = 115_000_000;
        let second = request("fresh2", Action::Snapshot);
        enqueue(&c, &second);
        c.step(&sim).await.unwrap();
        assert_eq!(result(&c, &first).result.float_spendable_sats, Some(0));
        assert_eq!(
            result(&c, &second).result.float_spendable_sats,
            Some(115_000_000)
        );
        let noop = request("noop", Action::BoardFloat);
        enqueue(&c, &noop);
        c.step(&sim).await.unwrap();
        let r = result(&c, &noop).result;
        assert_eq!(r.state, State::AlreadyAtTarget);
        assert!(r.funding_txid.is_none());
        let s = sim.0.lock().unwrap();
        assert_eq!(s.prepares, 0);
        assert_eq!(s.starts, 0);
    }

    #[tokio::test]
    async fn stale_foreign_and_insufficient_refuse_without_wallet_side_effect() {
        let path = dir();
        let c = Controller::open(&path).unwrap();
        let sim = Sim::new();
        sim.0.lock().unwrap().stale = true;
        let req = request("stale", Action::Snapshot);
        enqueue(&c, &req);
        c.step(&sim).await.unwrap();
        let r = result(&c, &req).result;
        assert_eq!(r.state, State::Rejected);
        assert!(r.onchain_trusted_sats.is_none());
        {
            let mut s = sim.0.lock().unwrap();
            s.stale = false;
            s.insufficient = true;
        }
        let board = request("insufficient", Action::BoardFloat);
        enqueue(&c, &board);
        c.step(&sim).await.unwrap();
        assert_eq!(result(&c, &board).result.state, State::Rejected);
        assert_eq!(sim.0.lock().unwrap().starts, 0);
        // No mining handle or pause capability exists in this controller.
    }

    #[test]
    fn fee_inverse_exact_net_and_total_cap_boundary() {
        let fees: ark::fees::BoardFees =
            serde_json::from_str(r#"{"min_fee_sat":50,"base_fee_sat":10,"ppm":1000}"#).unwrap();
        let (gross, fee) = gross_for_net(115_000_000, 5_000_000, &fees).unwrap();
        assert_eq!(gross - fee, 115_000_000);
        assert_eq!(
            fees.calculate(bitcoin::Amount::from_sat(gross))
                .unwrap()
                .to_sat(),
            fee
        );
        assert!(gross_for_net(115_000_000, fee - 1, &fees).is_err());
        assert!(gross_for_net(u64::MAX, 1, &fees).is_err());
    }

    #[test]
    fn private_paths_refuse_symlink_fifo_hardlink_loose_mode_and_duplicate_owner() {
        let path = dir();
        let c = Controller::open(&path).unwrap();
        assert!(Controller::open(&path).is_err());
        let request_path = path.join("request.json");
        std::fs::write(
            &request_path,
            serde_json::to_vec(&request("../still-opaque", Action::Snapshot)).unwrap(),
        )
        .unwrap();
        std::fs::set_permissions(&request_path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(read_request(&request_path).is_ok());
        let symlink = path.join("link");
        std::os::unix::fs::symlink(&request_path, &symlink).unwrap();
        assert!(read_request(&symlink).is_err());
        let hard = path.join("hard");
        std::fs::hard_link(&request_path, &hard).unwrap();
        assert!(read_request(&hard).is_err());
        let fifo = path.join("fifo");
        fs::mknodat(
            fs::CWD,
            &fifo,
            fs::FileType::Fifo,
            Mode::from_raw_mode(0o600),
            0,
        )
        .unwrap();
        assert!(read_request(&fifo).is_err());
        let ancestor = dir();
        let nested = ancestor.join("link");
        std::os::unix::fs::symlink(&path, &nested).unwrap();
        assert!(PrivateDir::open(&nested).is_err());
        std::fs::set_permissions(&ancestor, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(PrivateDir::open(&ancestor).is_err());
        drop(c);
    }

    #[test]
    fn request_schema_has_no_identity_or_float_coercion() {
        let req = request("opaque/id", Action::BoardFloat);
        assert_eq!(req.key().len(), 64);
        let mut json = serde_json::to_value(&req).unwrap();
        json["block_hash"] = serde_json::json!("forbidden");
        assert!(serde_json::from_value::<Request>(json).is_err());
        let mut json = serde_json::to_value(&req).unwrap();
        json["max_fee_sats"] = serde_json::json!(1.5);
        assert!(serde_json::from_value::<Request>(json).is_err());
        let mut json = serde_json::to_value(&req).unwrap();
        json["max_fee_sats"] = serde_json::json!("5000000");
        assert!(serde_json::from_value::<Request>(json).is_err());
    }

    fn proof_request(id: &str, original: &str) -> Request {
        let mut req = request(id, Action::BoardProofV1);
        req.original_board_operation_id = Some(original.into());
        req
    }

    async fn confirmed_fixture(c: &Controller, wallet: &Sim, id: &str) -> Request {
        let req = request(id, Action::BoardFloat);
        enqueue(c, &req);
        c.process_key(wallet, &req.key()).await.unwrap();
        {
            let mut state = wallet.0.lock().unwrap();
            state.confirmed = true;
            state.pending = false;
        }
        c.process_key(wallet, &req.key()).await.unwrap();
        assert_eq!(result(c, &req).result.state, State::Confirmed);
        req
    }

    #[tokio::test]
    async fn original_proof_read_survives_refresh_and_stock_pruning_without_wallet_calls() {
        let path = dir();
        let controller = Controller::open(&path).unwrap();
        let wallet = Sim::new();
        let board = confirmed_fixture(&controller, &wallet, "proof-original").await;
        let original = result(&controller, &board);
        let receipt_bytes = controller
            .dir
            .raw(&format!("{}.record", board.key()))
            .unwrap()
            .unwrap();
        let proof_hash = original.proof_digest.clone().unwrap();
        let canonical = controller
            .proofs
            .raw(&format!("{proof_hash}.proof"))
            .unwrap()
            .unwrap();
        let original_id = original
            .result
            .board
            .as_ref()
            .unwrap()
            .vtxo_id
            .clone()
            .unwrap();
        let child = format!("{}:0", bitcoin::Txid::from_byte_array([4; 32]));
        let round = RoundImportFacts {
            schema: super::super::wallet_control_proof::ROUND_SCHEMA.into(),
            stock_movement_id: 12,
            input_vtxo_ids: vec![original_id],
            output_vtxo_ids: vec![child.clone()],
            round_funding_txid: bitcoin::Txid::from_byte_array([5; 32]).to_string(),
            offchain_fee_sats: 13,
            completed_at_unix_ms: 2000,
        };
        round.validate().unwrap();
        wallet.0.lock().unwrap().round_imports = vec![round];
        controller.maintenance_pass(&wallet).await.unwrap();
        {
            let mut state = wallet.0.lock().unwrap();
            state.round_imports.clear();
            state.stale = true;
            state.stall_observation = true;
            state.fail_after_broadcast = true;
            state.board_refreshed = true;
            state.confirmed = false;
        }
        let before = {
            let s = wallet.0.lock().unwrap();
            (
                s.observations,
                s.prepares,
                s.starts,
                s.maintenance,
                s.recovery,
                s.protected,
            )
        };
        for id in ["proof-read-after-refresh", "proof-read-after-prune"] {
            let req = proof_request(id, &board.operation_id);
            enqueue(&controller, &req);
            controller.process_key(&wallet, &req.key()).await.unwrap();
            let reply = submit(&path, &req).unwrap();
            assert_eq!(reply.state, State::ProofAvailable);
            assert!(reply.observed_at_unix_ms.is_none() && reply.float_spendable_sats.is_none());
            let proof = reply.proof.unwrap();
            proof.validate(&board.operation_id).unwrap();
            assert_eq!(proof.proof_sha256, proof_hash);
            assert_eq!(hex::decode(proof.canonical_json_hex).unwrap(), canonical);
            assert_eq!(
                proof.original.raw_anchor_tx_hex,
                original.prepared.as_ref().unwrap().signed_tx_hex
            );
            assert_eq!(
                proof.original.owner_pubkey,
                original.prepared.as_ref().unwrap().user_pubkey
            );
            assert_eq!(
                proof.original.original_full_vtxo_hex,
                original.acceptance.as_ref().unwrap().full_vtxo_hex
            );
            assert!(matches!(proof.lineage_state, LineageState::RecordedPartial));
            assert!(!proof.lineage_complete);
            assert_eq!(proof.round_imports[0].output_vtxo_ids, vec![child.clone()]);
            let value = serde_json::to_string(&proof.original).unwrap();
            for forbidden in ["seed", "mnemonic", "key_index", "psbt_hex", "secret_key"] {
                assert!(!value.contains(&format!("\"{forbidden}\":")));
            }
        }
        let after = {
            let s = wallet.0.lock().unwrap();
            (
                s.observations,
                s.prepares,
                s.starts,
                s.maintenance,
                s.recovery,
                s.protected,
            )
        };
        assert_eq!(before, after);
        assert_eq!(
            controller
                .dir
                .raw(&format!("{}.record", board.key()))
                .unwrap()
                .unwrap(),
            receipt_bytes
        );
    }

    #[tokio::test]
    async fn proof_missing_unknown_wrong_operation_and_legacy_missing_reference_are_typed() {
        let path = dir();
        let controller = Controller::open(&path).unwrap();
        let wallet = Sim::new();
        let missing = proof_request("missing-read", "no-such-board");
        enqueue(&controller, &missing);
        controller
            .process_key(&wallet, &missing.key())
            .await
            .unwrap();
        assert_eq!(
            result(&controller, &missing).result.state,
            State::ProofMissing
        );
        wallet.0.lock().unwrap().fail_after_broadcast = true;
        let unknown = request("unknown-proof-board", Action::BoardFloat);
        enqueue(&controller, &unknown);
        controller
            .process_key(&wallet, &unknown.key())
            .await
            .unwrap();
        assert_eq!(result(&controller, &unknown).result.state, State::Unknown);
        let read_unknown = proof_request("unknown-read", &unknown.operation_id);
        enqueue(&controller, &read_unknown);
        controller
            .process_key(&wallet, &read_unknown.key())
            .await
            .unwrap();
        assert_eq!(
            result(&controller, &read_unknown).result.state,
            State::ProofUnavailable
        );
        assert!(result(&controller, &read_unknown).result.proof.is_none());
        let snapshot = request("not-a-board", Action::Snapshot);
        enqueue(&controller, &snapshot);
        controller
            .process_key(&wallet, &snapshot.key())
            .await
            .unwrap();
        let wrong = proof_request("wrong-kind-read", &snapshot.operation_id);
        enqueue(&controller, &wrong);
        controller.process_key(&wallet, &wrong.key()).await.unwrap();
        assert_eq!(
            result(&controller, &wrong).result.error_code.as_deref(),
            Some("original_operation_mismatch")
        );
        {
            let mut s = wallet.0.lock().unwrap();
            s.confirmed = true;
            s.pending = false;
            s.fail_after_broadcast = false;
        }
        controller
            .process_key(&wallet, &unknown.key())
            .await
            .unwrap();
        let mut legacy = result(&controller, &unknown);
        legacy.proof_digest = None;
        legacy.result.retained_proof_sha256 = None;
        controller
            .dir
            .write(&format!("{}.record", unknown.key()), &legacy)
            .unwrap();
        let old_bytes = controller
            .dir
            .raw(&format!("{}.record", unknown.key()))
            .unwrap();
        let read_legacy = proof_request("legacy-read", &unknown.operation_id);
        enqueue(&controller, &read_legacy);
        controller
            .process_key(&wallet, &read_legacy.key())
            .await
            .unwrap();
        assert_eq!(
            result(&controller, &read_legacy)
                .result
                .error_code
                .as_deref(),
            Some("proof_not_retained")
        );
        assert_eq!(
            controller
                .dir
                .raw(&format!("{}.record", unknown.key()))
                .unwrap(),
            old_bytes
        );
        let mut invalid = proof_request("invalid-read", &unknown.operation_id);
        invalid.max_fee_sats = Some(1);
        assert!(invalid.validate().is_err());
        assert!(proof_request(&unknown.operation_id, &unknown.operation_id)
            .validate()
            .is_err());
    }

    #[tokio::test]
    async fn partial_or_symlinked_proof_is_unavailable_and_immutable_objects_cannot_be_replaced() {
        let path = dir();
        let controller = Controller::open(&path).unwrap();
        let wallet = Sim::new();
        let board = confirmed_fixture(&controller, &wallet, "damaged-proof-board").await;
        let record = result(&controller, &board);
        let name = format!("{}.proof", record.proof_digest.unwrap());
        let bytes = controller.proofs.raw(&name).unwrap().unwrap();
        assert!(controller
            .proofs
            .write_immutable(&name, &serde_json::json!({"replacement":true}))
            .is_err());
        assert_eq!(controller.proofs.raw(&name).unwrap().unwrap(), bytes);
        let file = path.join(CONTROL_DIR).join("proofs").join(&name);
        std::fs::write(&file, b"{").unwrap();
        let read = proof_request("partial-proof-read", &board.operation_id);
        enqueue(&controller, &read);
        controller.process_key(&wallet, &read.key()).await.unwrap();
        assert_eq!(
            result(&controller, &read).result.state,
            State::ProofUnavailable
        );
        let backup = file.with_extension("evidence");
        std::fs::rename(&file, &backup).unwrap();
        std::os::unix::fs::symlink(&backup, &file).unwrap();
        let read = proof_request("symlink-proof-read", &board.operation_id);
        enqueue(&controller, &read);
        controller.process_key(&wallet, &read.key()).await.unwrap();
        assert_eq!(
            result(&controller, &read).result.state,
            State::ProofUnavailable
        );
        assert_eq!(std::fs::read(&backup).unwrap(), b"{");
        assert_eq!(wallet.0.lock().unwrap().starts, 1);
    }

    #[test]
    fn default_service_datadir_under_public_parent_is_private_before_wallet_open() {
        let root = dir();
        let configured = crate::config::Ark::default().datadir;
        assert_eq!(configured, Path::new("/var/lib/spc/ark"));
        let wallet = root.join(configured.strip_prefix("/").unwrap());
        let parent = wallet.parent().unwrap();
        std::fs::create_dir_all(parent).unwrap();
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o755)).unwrap();
        let controller = Controller::open(&wallet).unwrap();
        assert_eq!(std::fs::metadata(parent).unwrap().mode() & 0o777, 0o755);
        assert_eq!(std::fs::metadata(&wallet).unwrap().mode() & 0o777, 0o700);
        assert!(Controller::open(&wallet).is_err()); // before any stock wallet open
        prepare_wallet_database(&wallet).unwrap();
        let _stock =
            bark::persist::sqlite::SqliteClient::open(wallet.join("wallet.sqlite")).unwrap();
        assert_eq!(
            std::fs::metadata(wallet.join("wallet.sqlite"))
                .unwrap()
                .mode()
                & 0o777,
            0o600
        );
        drop(controller);
    }

    // Executed in a separate process both to isolate umask and to test actual
    // F_SETLK exclusion. Same-process stock registry checks would hide an
    // accidental release caused by closing an ordinary metadata-reading fd.
    #[test]
    fn stock_pid_lock_subprocess() {
        use bark::lock_manager::pid_fcntl::{FcntlPidLockManager, LOCK_FILE};
        let Ok(action) = std::env::var("SPC_POOL_TEST_PID_LOCK_ACTION") else {
            return;
        };
        let path =
            std::path::PathBuf::from(std::env::var_os("SPC_POOL_TEST_PID_LOCK_DIR").unwrap());
        match action.as_str() {
            "create" => {
                let _previous_umask = rustix::process::umask(Mode::from_raw_mode(0o022));
                let manager = FcntlPidLockManager::new(&path).unwrap();
                drop(manager);
            }
            "held" => {
                let expected = std::env::var("SPC_POOL_TEST_PID_LOCK_HOLDER").unwrap();
                assert_eq!(
                    std::fs::read_to_string(path.join(LOCK_FILE)).unwrap(),
                    expected
                );
                assert!(matches!(
                    FcntlPidLockManager::new(&path),
                    Err(bark::lock_manager::PidLockError::AlreadyHeld { .. })
                ));
                assert_eq!(
                    std::fs::read_to_string(path.join(LOCK_FILE)).unwrap(),
                    expected
                );
            }
            _ => panic!("unknown stock lock test action"),
        }
    }

    fn stock_pid_lock_child(path: &Path, action: &str) {
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "ark::wallet_control::tests::stock_pid_lock_subprocess",
                "--nocapture",
            ])
            .env("SPC_POOL_TEST_PID_LOCK_ACTION", action)
            .env("SPC_POOL_TEST_PID_LOCK_DIR", path)
            .env(
                "SPC_POOL_TEST_PID_LOCK_HOLDER",
                std::process::id().to_string(),
            )
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
    }

    #[test]
    fn stock_0644_pid_stamp_survives_stop_reopen_without_replacing_or_unlocking_it() {
        use bark::lock_manager::pid_fcntl::{FcntlPidLockManager, LOCK_FILE};
        let path = dir();
        let first = Controller::open(&path).unwrap();
        prepare_wallet_database(&path).unwrap();
        // Actual pinned stock creator in an isolated 022-umask process.
        stock_pid_lock_child(&path, "create");
        let lock_path = path.join(LOCK_FILE);
        let initial = std::fs::metadata(&lock_path).unwrap();
        assert_eq!(initial.mode() & 0o7777, 0o644);
        let contents = std::fs::read(&lock_path).unwrap(); // stock creator has stopped
        drop(first);
        let reopened = Controller::open(&path).unwrap();
        assert_eq!(std::fs::metadata(&lock_path).unwrap().ino(), initial.ino());
        assert_eq!(std::fs::read(&lock_path).unwrap(), contents);
        // The stock manager now holds a real POSIX record lock in THIS process.
        let stock = FcntlPidLockManager::new(&path).unwrap();
        stock_pid_lock_child(&path, "held");
        prepare_datadir(&path).unwrap();
        stock_pid_lock_child(&path, "held");
        assert!(Controller::open(&path).is_err());
        stock_pid_lock_child(&path, "held");
        let alias = path.join("test-lock-alias");
        std::fs::hard_link(&lock_path, &alias).unwrap();
        assert!(prepare_datadir(&path).is_err());
        stock_pid_lock_child(&path, "held");
        std::fs::remove_file(&alias).unwrap(); // remove only this test-created alias
        let current = std::fs::metadata(&lock_path).unwrap();
        assert_eq!(
            (
                current.dev(),
                current.ino(),
                current.nlink(),
                current.mode()
            ),
            (
                initial.dev(),
                initial.ino(),
                initial.nlink(),
                initial.mode()
            )
        );
        drop(stock);
        drop(reopened);
        let _third = Controller::open(&path).unwrap();
        let _stock = FcntlPidLockManager::new(&path).unwrap();
        stock_pid_lock_child(&path, "held");
    }

    #[test]
    fn stock_metadata_compatibility_does_not_relax_secrets_aliases_or_control_files() {
        use bark::lock_manager::pid_fcntl::LOCK_FILE;
        let path = dir();
        let controller = Controller::open(&path).unwrap();
        let secret = path.join("mnemonic");
        std::fs::write(&secret, "private fixture").unwrap();
        std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(prepare_datadir(&path).is_err());
        std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o600)).unwrap();
        let lock_path = path.join(LOCK_FILE);
        std::fs::hard_link(&secret, &lock_path).unwrap();
        assert!(prepare_datadir(&path).is_err());
        std::fs::remove_file(&lock_path).unwrap(); // remove only the test's deliberate attack alias
        std::os::unix::fs::symlink(&secret, &lock_path).unwrap();
        assert!(prepare_datadir(&path).is_err());
        assert_eq!(std::fs::read_to_string(&secret).unwrap(), "private fixture");
        std::fs::remove_file(&lock_path).unwrap();
        fs::mknodat(
            fs::CWD,
            &lock_path,
            fs::FileType::Fifo,
            Mode::from_raw_mode(0o600),
            0,
        )
        .unwrap();
        assert!(prepare_datadir(&path).is_err());
        let req = request("private-mode-stays-strict", Action::Snapshot);
        enqueue(&controller, &req);
        let queued = path
            .join(CONTROL_DIR)
            .join(format!("{}.request", req.key()));
        std::fs::set_permissions(&queued, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(controller
            .dir
            .read::<Request>(&format!("{}.request", req.key()))
            .is_err());
        if rustix::process::getuid().as_raw() != 0 {
            let foreign = File::from(
                fs::open(
                    "/etc/passwd",
                    OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                    Mode::empty(),
                )
                .unwrap(),
            );
            assert_ne!(
                foreign.metadata().unwrap().uid(),
                rustix::process::getuid().as_raw()
            );
            let error =
                check_wallet_root_entry(&foreign, std::ffi::OsStr::new(LOCK_FILE)).unwrap_err();
            assert!(error.to_string().contains("ownership/type"));
        }
    }

    #[test]
    fn owned_loose_datadir_tightens_but_unsafe_private_files_and_links_are_refused() {
        let root = dir();
        let wallet = root.join("wallet");
        std::fs::create_dir(&wallet).unwrap();
        std::fs::set_permissions(&wallet, std::fs::Permissions::from_mode(0o750)).unwrap();
        let controller = Controller::open(&wallet).unwrap();
        assert_eq!(std::fs::metadata(&wallet).unwrap().mode() & 0o777, 0o700);
        drop(controller);
        let unsafe_file = wallet.join("wallet.sqlite");
        std::fs::write(&unsafe_file, []).unwrap();
        std::fs::set_permissions(&unsafe_file, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(Controller::open(&wallet).is_err());
        assert_eq!(
            std::fs::metadata(&unsafe_file).unwrap().mode() & 0o777,
            0o644
        );
        let link = root.join("alias");
        std::os::unix::fs::symlink(&wallet, &link).unwrap();
        assert!(Controller::open(&link).is_err());
        if rustix::process::getuid().as_raw() != 0 {
            let before = std::fs::metadata("/proc").unwrap().mode();
            assert!(prepare_datadir(Path::new("/proc")).is_err());
            assert_eq!(std::fs::metadata("/proc").unwrap().mode(), before);
        }
    }

    #[tokio::test]
    async fn pending_board_and_unknown_proposal_do_not_blackout_maintenance() {
        let path = dir();
        let c = Controller::open(&path).unwrap();
        let sim = Sim::new();
        sim.0.lock().unwrap().pending = true; // ordinary automatic stock board
        for _ in 0..3 {
            c.recovery_pass(&sim).await.unwrap();
            c.maintenance_pass(&sim).await.unwrap();
        }
        {
            let s = sim.0.lock().unwrap();
            assert_eq!(s.maintenance, 3);
            assert_eq!(s.recovery, 3);
            assert_eq!(s.overdue_refreshed, 3);
        }
        sim.0.lock().unwrap().pending = false;
        sim.0.lock().unwrap().fail_after_broadcast = true;
        let req = request("unknown-maintenance", Action::BoardFloat);
        enqueue(&c, &req);
        c.step(&sim).await.unwrap();
        assert_eq!(result(&c, &req).result.state, State::Unknown);
        assert!(c.unresolved().unwrap());
        c.recovery_pass(&sim).await.unwrap();
        c.maintenance_pass(&sim).await.unwrap();
        assert_eq!(sim.0.lock().unwrap().maintenance, 4);
        assert_eq!(sim.0.lock().unwrap().starts, 1);
        c.step(&sim).await.unwrap();
        assert_eq!(
            result(&c, &req).result.error_code.as_deref(),
            Some("stock_board_outcome_unknown")
        );
    }

    #[tokio::test]
    async fn pending_refresh_reserves_capacity_until_its_real_float_returns() {
        let path = dir();
        let controller = Controller::open(&path).unwrap();
        let wallet = Sim::new();
        wallet.0.lock().unwrap().round_pending = true;
        let while_refreshing = request("refresh-capacity", Action::BoardFloat);
        enqueue(&controller, &while_refreshing);
        controller.step(&wallet).await.unwrap();
        assert_eq!(
            result(&controller, &while_refreshing)
                .result
                .error_code
                .as_deref(),
            Some("board_in_progress")
        );
        {
            let mut state = wallet.0.lock().unwrap();
            state.round_pending = false;
            state.float = 115_000_000;
        }
        let after_refresh = request("refresh-completed", Action::BoardFloat);
        enqueue(&controller, &after_refresh);
        controller.step(&wallet).await.unwrap();
        assert_eq!(
            result(&controller, &after_refresh).result.state,
            State::AlreadyAtTarget
        );
        assert_eq!(wallet.0.lock().unwrap().starts, 0);
    }

    #[tokio::test]
    async fn acceptance_proof_is_durable_before_refresh_and_survives_original_becoming_spent() {
        let path = dir();
        let c = Controller::open(&path).unwrap();
        let sim = Sim::new();
        let req = request("refresh-proof", Action::BoardFloat);
        enqueue(&c, &req);
        c.step(&sim).await.unwrap();
        {
            let mut s = sim.0.lock().unwrap();
            s.confirmed = true;
            s.pending = false;
        }
        c.maintenance_pass(&sim).await.unwrap();
        assert!(!sim.0.lock().unwrap().board_refreshed); // only the unobserved board is reserved
        assert_eq!(sim.0.lock().unwrap().overdue_refreshed, 1); // other refresh still proceeds
        c.step(&sim).await.unwrap();
        let accepted = result(&c, &req);
        accepted.validate().unwrap();
        assert!(accepted.acceptance.is_some());
        assert_eq!(accepted.result.state, State::Confirmed);
        c.maintenance_pass(&sim).await.unwrap();
        assert!(sim.0.lock().unwrap().board_refreshed);
        drop(c);
        let reopened = Controller::open(&path).unwrap();
        reopened.step(&sim).await.unwrap();
        assert_eq!(result(&reopened, &req).result.state, State::Confirmed);
        assert_eq!(sim.0.lock().unwrap().starts, 1);
    }

    #[tokio::test]
    async fn spent_original_without_verified_acceptance_is_not_a_confirmed_board() {
        let path = dir();
        let c = Controller::open(&path).unwrap();
        let sim = Sim::new();
        let req = request("unproven-spent", Action::BoardFloat);
        enqueue(&c, &req);
        c.step(&sim).await.unwrap();
        {
            let mut s = sim.0.lock().unwrap();
            s.confirmed = true;
            s.pending = false;
            s.board_refreshed = true;
        }
        c.step(&sim).await.unwrap();
        assert_eq!(result(&c, &req).result.state, State::Unknown);
        assert!(result(&c, &req).acceptance.is_none());
    }

    #[tokio::test]
    async fn corrupt_first_entry_preserves_evidence_and_fair_snapshots_and_known_recovery_on_restart(
    ) {
        let path = dir();
        let mut c = Controller::open(&path).unwrap();
        let sim = Sim::new();
        let board = request("known-recovery", Action::BoardFloat);
        enqueue(&c, &board);
        c.step(&sim).await.unwrap();
        c.dir
            .write(
                &format!("{}.record", "0".repeat(64)),
                &serde_json::json!({"damaged":true}),
            )
            .unwrap();
        c.dir
            .write("bad.request", &serde_json::json!({"bad":true}))
            .unwrap();
        let requests: Vec<_> = (0..24)
            .map(|i| request(&format!("fair-{i}"), Action::Snapshot))
            .collect();
        for req in &requests {
            enqueue(&c, req);
        }
        {
            let mut s = sim.0.lock().unwrap();
            s.confirmed = true;
            s.pending = false;
        }
        for _ in 0..6 {
            c.step(&sim).await.unwrap();
            drop(c);
            c = Controller::open(&path).unwrap();
        }
        for req in &requests {
            assert_eq!(result(&c, req).result.state, State::Observed);
        }
        assert_eq!(result(&c, &board).result.state, State::Confirmed);
        assert!(c.unresolved().is_err()); // corrupt durable state is not no-effect evidence
        assert!(!c.faults.names(".fault").unwrap().is_empty());
        let next = request("blocked-by-corruption", Action::BoardFloat);
        enqueue(&c, &next);
        for _ in 0..5 {
            c.step(&sim).await.unwrap();
        }
        assert_eq!(
            result(&c, &next).result.error_code.as_deref(),
            Some("board_in_progress")
        );
        assert_eq!(sim.0.lock().unwrap().starts, 1);
    }

    #[tokio::test]
    async fn damaged_request_cannot_hide_valid_durable_recovery_or_restart_key_allocation() {
        let path = dir();
        let c = Controller::open(&path).unwrap();
        let sim = Sim::new();
        let req = request("durable-authority", Action::BoardFloat);
        enqueue(&c, &req);
        c.step(&sim).await.unwrap();
        c.dir
            .write(
                &format!("{}.request", req.key()),
                &serde_json::json!({"invalid":true}),
            )
            .unwrap();
        {
            let mut s = sim.0.lock().unwrap();
            s.pending = false;
            s.confirmed = true;
        }
        c.step(&sim).await.unwrap();
        assert_eq!(result(&c, &req).result.state, State::Confirmed);
        std::fs::remove_file(path.join(CONTROL_DIR).join(format!("{}.record", req.key()))).unwrap();
        for _ in 0..3 {
            c.step(&sim).await.unwrap();
        }
        assert!(c.unresolved().unwrap());
        assert_eq!(sim.0.lock().unwrap().prepares, 1);
    }

    fn stalled_owner() -> (Arc<Sim>, Arc<ObservationProbe>) {
        let owner = Arc::new(Sim::new());
        let probe = Arc::new(ObservationProbe::default());
        {
            let mut state = owner.0.lock().unwrap();
            state.stall_observation = true;
            state.observation_probe = Some(probe.clone());
        }
        (owner, probe)
    }

    fn assert_unavailable_snapshot(c: &Controller, req: &Request, owner: &Sim) {
        let record = result(c, req);
        record.validate().unwrap();
        assert_eq!(record.result.state, State::Rejected);
        assert_eq!(
            record.result.error_code.as_deref(),
            Some("fresh_observation_unavailable")
        );
        assert!(record.result.onchain_trusted_sats.is_none());
        assert!(record.result.float_spendable_sats.is_none());
        assert!(record.result.observed_at_unix_ms.is_none());
        assert!(record.result.observed_tip_height.is_none());
        let state = owner.0.lock().unwrap();
        assert_eq!(state.observations, 0);
        assert_eq!(state.prepares, 0);
        assert_eq!(state.starts, 0);
    }

    #[tokio::test(start_paused = true)]
    async fn stalled_snapshot_obeys_read_deadline_and_persists_unavailable() {
        use super::super::wallet_control_io::READ_DEADLINE;
        use std::sync::atomic::Ordering::SeqCst;
        assert_eq!(READ_DEADLINE, Duration::from_secs(5));
        let c = Arc::new(Controller::open(&dir()).unwrap());
        let (owner, probe) = stalled_owner();
        let req = request("controlled-read-deadline", Action::Snapshot);
        enqueue(&c, &req);
        let worker_controller = c.clone();
        let worker_owner = owner.clone();
        let key = req.key();
        let start = tokio::time::Instant::now();
        // This is the real process_key -> observe_read -> tokio::timeout path.
        // Files remain durable; only the runtime's clock is controlled.
        let worker = tokio::spawn(async move {
            worker_controller
                .process_key(worker_owner.as_ref(), &key)
                .await
        });
        tokio::time::timeout(Duration::from_secs(1), probe.entered.notified())
            .await
            .expect("snapshot never entered its observation");
        assert_eq!(probe.attempts.load(SeqCst), 1);
        tokio::time::advance(READ_DEADLINE - Duration::from_millis(1)).await;
        tokio::task::yield_now().await;
        assert!(!worker.is_finished(), "read timed out before READ_DEADLINE");
        assert!(!probe.dropped.load(SeqCst));
        tokio::time::advance(Duration::from_millis(1)).await;
        tokio::time::timeout(Duration::from_secs(1), worker)
            .await
            .expect("read deadline did not resolve")
            .unwrap()
            .unwrap();
        assert_eq!(tokio::time::Instant::now() - start, READ_DEADLINE);
        assert!(
            probe.dropped.load(SeqCst),
            "timed-out observation was not dropped"
        );
        assert_eq!(probe.attempts.load(SeqCst), 1);
        assert_unavailable_snapshot(&c, &req, &owner);
    }

    #[tokio::test(start_paused = true)]
    async fn production_key_span_releases_capacity_before_cursor_commit() {
        use super::super::wallet_control_io::READ_DEADLINE;
        use std::sync::atomic::Ordering::SeqCst;
        let c = Arc::new(Controller::open(&dir()).unwrap());
        let (owner, probe) = stalled_owner();
        let req = request("production-read-capacity", Action::Snapshot);
        enqueue(&c, &req);
        // The production helper acquires this mutex, just as run passes the
        // actual BarkWalletClient::operation_lock. The test never owns/drops
        // the controller's guard itself.
        let capacity = Arc::new(tokio::sync::Mutex::new(()));
        let cursor_entered = Arc::new(tokio::sync::Notify::new());
        let allow_cursor = Arc::new(tokio::sync::Notify::new());
        let competing_operation_done = Arc::new(tokio::sync::Notify::new());
        let worker_controller = c.clone();
        let worker_owner = owner.clone();
        let worker_capacity = capacity.clone();
        let cursor_signal = cursor_entered.clone();
        let cursor_release = allow_cursor.clone();
        let capacity_observed = competing_operation_done.clone();
        let key = req.key();
        let worker = tokio::spawn(async move {
            let cursor_controller = worker_controller.clone();
            let cursor_capacity = worker_capacity.clone();
            let cursor_key = key.clone();
            worker_controller
                .process_key_with_capacity(
                    worker_owner.as_ref(),
                    &key,
                    worker_capacity.as_ref(),
                    move || async move {
                        // A fair Tokio mutex may reserve the released permit for
                        // the queued waiter before that task is polled. Observe
                        // its completion instead of mistaking that reservation
                        // for the controller still holding the guard.
                        tokio::time::timeout(Duration::from_secs(1), capacity_observed.notified())
                            .await
                            .expect("cursor reached before controller released capacity");
                        // Moving guard release after cursor persistence must fail.
                        assert!(
                            cursor_capacity.try_lock().is_ok(),
                            "cursor entered while operation guard held"
                        );
                        cursor_signal.notify_one();
                        cursor_release.notified().await;
                        cursor_controller.advance(&cursor_key)
                    },
                )
                .await;
        });
        tokio::time::timeout(Duration::from_secs(1), probe.entered.notified())
            .await
            .expect("production helper never entered its observation");
        assert!(
            capacity.try_lock().is_err(),
            "production helper did not acquire capacity"
        );
        let waiting = capacity.clone();
        let waiting_controller = c.clone();
        let waiting_owner = owner.clone();
        let waiting_request = req.clone();
        let waiter = tokio::spawn(async move {
            {
                let _permit = waiting.lock().await;
                assert_unavailable_snapshot(&waiting_controller, &waiting_request, &waiting_owner);
                assert!(!waiting_controller.dir.present("cursor.json").unwrap());
            }
            competing_operation_done.notify_one();
        });
        tokio::time::advance(READ_DEADLINE - Duration::from_millis(1)).await;
        tokio::task::yield_now().await;
        assert!(
            !waiter.is_finished(),
            "competing operation entered before read outcome"
        );
        assert!(!probe.dropped.load(SeqCst));
        tokio::time::advance(Duration::from_millis(1)).await;
        // This virtual-time watchdog detects a stuck/mis-scoped guard. It is
        // not a new production latency limit on fsync or money operations.
        tokio::time::timeout(Duration::from_secs(1), cursor_entered.notified())
            .await
            .expect("production helper did not reach the unlocked cursor boundary");
        assert!(probe.dropped.load(SeqCst));
        assert_unavailable_snapshot(&c, &req, &owner);
        assert!(!c.dir.present("cursor.json").unwrap());
        assert!(!worker.is_finished(), "cursor barrier was bypassed");
        tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("send capacity unavailable before cursor commit")
            .unwrap();
        allow_cursor.notify_one();
        tokio::time::timeout(Duration::from_secs(1), worker)
            .await
            .expect("cursor completion did not finish")
            .unwrap();
        assert_eq!(
            c.dir.read::<String>("cursor.json").unwrap(),
            Some(req.key())
        );
        assert_eq!(probe.attempts.load(SeqCst), 1);
    }

    #[test]
    fn real_rpc_proofs_bind_prevout_tip_and_maturity_without_wallet_mutation() {
        use std::io::{BufRead, BufReader};
        use std::net::TcpListener;
        let tip = bitcoin::BlockHash::from_byte_array([3; 32]);
        let outpoint = bitcoin::OutPoint::new(bitcoin::Txid::from_byte_array([1; 32]), 7);
        let expected = bitcoin::TxOut {
            value: bitcoin::Amount::from_sat(120_000_000),
            script_pubkey: bitcoin::ScriptBuf::from_bytes(vec![0x51]),
        };
        // The real bitcoincore-rpc HTTP client parses these wire responses.
        // Each fixture permits ONLY gettxout: snapshot must not sync or derive.
        for (confirmations, coinbase, foreign_tip, foreign_value, wanted) in [
            (3, false, false, false, Some(true)),
            (2, false, false, false, Some(false)),
            (99, true, false, false, Some(false)),
            (100, true, false, false, Some(true)),
            (3, false, true, false, None),
            (3, false, false, true, None),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut content_length = None;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(n) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        content_length = Some(n.trim().parse::<usize>().unwrap());
                    }
                }
                let mut body = vec![0; content_length.unwrap()];
                reader.read_exact(&mut body).unwrap();
                let request: serde_json::Value = serde_json::from_slice(&body).unwrap();
                assert_eq!(request["method"], "gettxout");
                assert_eq!(
                    request["params"],
                    serde_json::json!([outpoint.txid.to_string(), 7, true])
                );
                let mut response: serde_json::Value = serde_json::from_str(r#"{"result":{"bestblock":"","confirmations":3,"value":1.20000000,"scriptPubKey":{"asm":"1","hex":"51","type":"nonstandard"},"coinbase":false},"error":null,"id":0}"#).unwrap();
                response["id"] = request["id"].clone();
                response["result"]["bestblock"] = serde_json::json!(if foreign_tip {
                    bitcoin::BlockHash::from_byte_array([4; 32])
                } else {
                    tip
                }
                .to_string());
                response["result"]["confirmations"] = serde_json::json!(confirmations);
                response["result"]["coinbase"] = serde_json::json!(coinbase);
                if foreign_value {
                    response["result"]["value"] = serde_json::json!(2);
                }
                let bytes = serde_json::to_vec(&response).unwrap();
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", bytes.len()).unwrap();
                stream.write_all(&bytes).unwrap();
            });
            let rpc = bitcoincore_rpc::Client::new(
                &format!("http://{addr}"),
                bitcoincore_rpc::Auth::None,
            )
            .unwrap();
            let result = trusted_output(&rpc, tip, outpoint, &expected);
            server.join().unwrap();
            match wanted {
                Some(want) => assert_eq!(result.unwrap(), want),
                None => assert!(result.is_err()),
            }
        }
    }
}
