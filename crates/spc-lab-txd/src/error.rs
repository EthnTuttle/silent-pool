//! Fail-closed errors for the traffic wallet core.
//!
//! Every ambiguity in key material, funding provenance, money arithmetic,
//! journal integrity or lifecycle transitions is a named error here. There is
//! no donation fallback and no silent recovery: corrupt or wrong-network key
//! files fail closed without regenerating a seed, and an outpoint whose RPC
//! result is unknown is never released or reselected.

/// Top-level error for the `spc-lab-txd` core.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("key error: {0}")]
    Key(#[from] KeyError),

    #[error("money error: {0}")]
    Money(#[from] MoneyError),

    #[error("builder error: {0}")]
    Builder(#[from] BuilderError),

    #[error("journal error: {0}")]
    Journal(#[from] JournalError),

    #[error("wallet error: {0}")]
    Wallet(#[from] WalletError),
}

/// Key derivation and private key file errors.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KeyError {
    #[error("unsupported network for traffic keys: {0}")]
    UnsupportedNetwork(String),
    #[error("key file io: {0}")]
    Io(String),
    #[error("key file is corrupt or unreadable: {0}")]
    CorruptKeyFile(String),
    #[error("key file network {stored:?} does not match expected {expected:?}")]
    WrongNetwork { stored: String, expected: String },
    #[error("key file fingerprint mismatch for {role}: stored {stored}, re-derived {derived}; refusing to regenerate")]
    FingerprintMismatch {
        role: String,
        stored: String,
        derived: String,
    },
    #[error("key file wallet set mismatch: stored {stored}, expected {expected}")]
    WalletSetMismatch { stored: String, expected: String },
    #[error("key file binding mismatch: {field} is {stored:?}, expected {expected:?}")]
    BindingMismatch {
        field: String,
        stored: String,
        expected: String,
    },
    #[error("key file declares key mode {stored:?}, expected {expected:?}")]
    KeyModeMismatch { stored: String, expected: String },
    #[error(
        "invalid run id {0:?}: non-empty, at most 128 bytes, ASCII alphanumeric or -_.:/@ only"
    )]
    InvalidRunId(String),
    #[error("bip32 derivation failed: {0}")]
    Derivation(String),
    #[error(
        "script does not belong to this wallet (lookahead {lookahead} exceeded or foreign script)"
    )]
    ScriptNotOwned { lookahead: u32 },
    #[error("wallet role not present in this wallet set: {0}")]
    UnknownWallet(String),
    #[error("invalid wallet role: {0}")]
    InvalidRole(String),
}

/// Checked money arithmetic and exact decimal conversion errors.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MoneyError {
    #[error("satoshi sum overflows u64")]
    Overflow,
    #[error("satoshi subtraction underflows (inputs cannot fund outputs)")]
    Underflow,
    #[error("amount {value:?} is not an exact BTC amount (at most eight fractional digits, no sign, digits only)")]
    NotExactDecimal { value: String },
    #[error("zero or negative fee where a positive fee is required")]
    ZeroFee,
}

/// Transaction construction and signing errors.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BuilderError {
    #[error("insufficient funds: need {needed} sats, selected inputs hold {available}")]
    InsufficientFunds { needed: u64, available: u64 },
    #[error("change of {change} sats is below the dust limit {dust} sats; refusing to build")]
    DustChange { change: u64, dust: u64 },
    #[error(
        "destination amount {amount} sats is below the dust limit {dust} sats; refusing to build"
    )]
    DustDestination { amount: u64, dust: u64 },
    #[error("a destination script equals the change script for this wallet; refusing ambiguous classification")]
    AmbiguousChangeScript,
    #[error("fee {fee} sats is below the minimum rate band {min_rate} sat/vB at {vbytes} vB")]
    FeeBelowBand {
        fee: u64,
        min_rate: u64,
        vbytes: u64,
    },
    #[error("fee {fee} sats is above the maximum rate band {max_rate} sat/vB at {vbytes} vB")]
    FeeAboveBand {
        fee: u64,
        max_rate: u64,
        vbytes: u64,
    },
    #[error("fee {fee} sats exceeds the per-transaction ceiling {max_fee}")]
    FeeCeilingExceeded { fee: u64, max_fee: u64 },
    #[error(
        "requested fee rate {rate} is outside the policy band [{min_rate}, {max_rate}] sat/vB"
    )]
    FeeRateOutsidePolicy {
        rate: u64,
        min_rate: u64,
        max_rate: u64,
    },
    #[error("construction did not converge on a final signed size after {attempts} deterministic attempts")]
    NotConverged { attempts: u32 },
    #[error("prevout script is not P2PKH; the v1 traffic profile is legacy-only: {script_hex}")]
    NotP2pkhPrevout { script_hex: String },
    #[error("destination script must be standard P2PKH, P2WPKH or P2TR: {script_hex}")]
    UnsupportedDestination { script_hex: String },
    #[error("witness destination cannot be owned by a P2PKH traffic wallet")]
    WitnessDestinationNotOwned,
    #[error(
        "output script is neither a declared destination nor an owned change script: {script_hex}"
    )]
    UnknownOutputScript { script_hex: String },
    #[error("destination amount mismatch: requested {requested}, built {built}")]
    DestinationAmountMismatch { requested: u64, built: u64 },
    #[error("conservation violation: inputs {inputs} != outputs {outputs} + fee {fee}")]
    ConservationViolation { inputs: u64, outputs: u64, fee: u64 },
    #[error("transaction signing failed: {0}")]
    Signing(String),
    #[error("invalid fee policy: {0}")]
    InvalidFeePolicy(String),
}

/// Durable journal errors.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum JournalError {
    #[error("journal io: {0}")]
    Io(String),
    #[error("journal integrity: {0}")]
    Integrity(String),
    #[error("unsupported history version {found}; this core implements {supported}")]
    UnsupportedHistoryVersion { found: u32, supported: u32 },
    #[error("duplicate funding observation for outpoint {outpoint}")]
    DuplicateFunding { outpoint: String },
    #[error("outpoint {outpoint} cannot be reserved: {reason}")]
    NotReservable { outpoint: String, reason: String },
    #[error("outpoint {outpoint} is already reserved by command {command}")]
    AlreadyReserved { outpoint: String, command: String },
    #[error("outpoint {outpoint} belongs to {owner}, not to {wallet}")]
    OutpointOwnerMismatch {
        outpoint: String,
        owner: String,
        wallet: String,
    },
    #[error("command {command} is recorded with a different payload digest (existing {existing}, incoming {incoming})")]
    CommandConflict {
        command: String,
        existing: String,
        incoming: String,
    },
    #[error("illegal lifecycle transition for {txid}: {reason}")]
    IllegalTransition { txid: String, reason: String },
    #[error("unknown transaction {0} in the journal")]
    UnknownTx(String),
    #[error("unknown command {0} in the journal")]
    UnknownCommand(String),
    #[error("unknown-retry cap {cap} exceeded for {txid}")]
    UnknownRetryCapExceeded { cap: u32, txid: String },
    #[error("reorg-resubmit cap {cap} exceeded for {txid}")]
    ReorgResubmitCapExceeded { cap: u32, txid: String },
    #[error("reorg resubmission of {txid} is not eligible: {reason}")]
    ReorgRetryNotEligible { txid: String, reason: String },
    #[error(
        "fixture funding refused: this wallet set does not allow synthetic conformance fixtures"
    )]
    FixtureFundingRefused,
    #[error("raw bytes do not hash to the recorded txid for {txid}")]
    RawTxidMismatch { txid: String },
    #[error("journal tamper check failed: {0}")]
    TamperCheck(String),
    #[error("journal {path} is held by another owner (kernel flock); no mutation was performed")]
    HeldByAnotherOwner { path: String },
    #[error(
        "journal handle is poisoned: a write failed and the rollback could not be proven; no further mutation is possible"
    )]
    Poisoned,
    #[error("journal header caps {found:?} do not match the expected caps {expected:?}")]
    CapsMismatch {
        found: crate::journal::JournalCaps,
        expected: crate::journal::JournalCaps,
    },
    #[error(
        "journal header allocation digest {found:?} does not match the expected allocation digest {expected:?}"
    )]
    AllocationMismatch { found: String, expected: String },
    #[error(
        "journal header driver id {stored:?} does not match the expected driver id {expected:?}"
    )]
    DriverIdMismatch { stored: String, expected: String },
    #[error("journal header declares key mode {stored:?}, expected {expected:?}")]
    KeyModeMismatch { stored: String, expected: String },
    #[error("journal header declares derivation version {found}, expected {expected}")]
    DerivationVersionMismatch { found: u32, expected: u32 },
    #[error("funding observation for {outpoint} is not verifiable: {reason}")]
    FundingNotVerifiable { outpoint: String, reason: String },
}

/// Wallet-set level errors.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WalletError {
    #[error("fixture funding is only available to the explicit test constructor")]
    FixtureNotAllowed,
    #[error("funding observation owner {0} is not part of this wallet set")]
    OwnerNotInSet(String),
    #[error("invalid command id: {0}")]
    InvalidCommandId(String),
    #[error("destination wallet {0} is not part of this wallet set")]
    DestinationWalletNotInSet(String),
    #[error("destination address is not valid for this wallet set's network {expected}")]
    DestinationNetworkWrong { expected: String },
    #[error("wallet-set allocation is not valid: {0}")]
    AllocationConflict(String),
}
