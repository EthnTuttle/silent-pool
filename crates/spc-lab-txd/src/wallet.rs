//! The traffic wallet set: key provisioning, the payment flow, and the
//! crash-recovery contract on top of the journal (traffic plan sections
//! 5.1, 6.2, 6.3 and 6.7).
//!
//! One wallet set owns exactly one private key file and exactly one
//! journal; there is no second signer and no second ledger. The payment
//! flow is:
//!
//! 1. `begin_command` (durable dedup by command id + payload digest; the
//!    digest is computed by the journal over the full request record,
//!    payer role and category included);
//! 2. deterministic selection, pre-validated with an upper-bound fee so
//!    that a reservation never needs to be retried with different inputs;
//! 3. `reserve` (durable, exclusive, **before** signing);
//! 4. `build_and_sign` (deterministic convergence);
//! 5. `record_signed` (raw bytes + txid durable before any broadcast).
//!
//! `sign_payment` either always builds and records a transaction, or
//! fails closed with the reservation explicitly released (a construction
//! failure after reservation journals `InputsReleased` before returning);
//! there is no state in between.
//!
//! Crash recovery re-enters `sign_payment` with the same command id and
//! payload: a `Reserved`-but-not-signed command re-signs with the **same**
//! reserved outpoints and the **same** deterministic change index (never
//! reselects); a signed command returns the stored raw bytes (identical
//! bytes on retry); a conflicting payload digest is a journaled conflict
//! error. Nothing here talks to a node: broadcast and observation are
//! T1b/T3 orchestration over the same journal.
//!
//! Allocation (M3): a lab population may be split across several actors,
//! each owning an explicit, disjoint slice of the wallet index ranges.
//! [`ProvisionConfig::validate_allocation`] checks the whole allocation
//! (disjoint ranges, exactly one bootstrap-funding host, one shared run
//! seed + run id) **before** any actor provisions; each set's journal
//! header then binds its run id, driver id, network, run-seed
//! fingerprint, key mode and retry caps.

use std::collections::HashMap;
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};

use bitcoin::secp256k1::Secp256k1;
use bitcoin::{BlockHash, Network, OutPoint, ScriptBuf, Txid};

use crate::builder::{
    build_and_sign_with_construction, deterministic_utxo_order, selection_fee_upper_bound,
    DestinationSpec, FeePolicy, PaymentCategory, SignableUtxo, SignedTrafficTx,
};
use crate::error::{Error, WalletError};
use crate::journal::{
    request_record_digest, CommandBeginOutcome, CommandPhase, FundingObservation, JournalCaps,
    RequestRecord, StoredCommand, TrafficJournal, PROTOCOL, SUPPORTED_HISTORY_VERSION,
};
use crate::keys::{
    Chain, TrafficKeyStore, TrafficWallet, WalletRole, DEFAULT_SCRIPT_LOOKAHEAD,
    DERIVATION_VERSION, KEY_MODE,
};
use crate::money::checked_add_sats;

/// How many background network wallets the standard lab population has.
pub const STANDARD_NETWORK_WALLETS: u32 = 8;

/// How many SPc user payment wallets the standard lab population has.
pub const STANDARD_USER_PAYMENT_WALLETS: u32 = 4;

/// Provisioning shape of one wallet set (M3: explicit index ranges).
///
/// A multi-actor deployment provisions one `ProvisionConfig` per actor;
/// the whole allocation must pass [`ProvisionConfig::validate_allocation`]
/// first. The bootstrap funding wallet is hosted by exactly one config in
/// the allocation (the single coinbase-minting host; T2-L drives it).
#[derive(Clone, Debug)]
pub struct ProvisionConfig {
    /// The scenario `RunSpec.seed` the whole wallet set derives from.
    pub run_seed: u64,
    pub network: Network,
    /// The run id: HMAC derivation domain and journal-header binding
    /// (validated against the T0 id vocabulary).
    pub run_id: String,
    /// The driver id this set's journal is bound to (one journal, one
    /// owner; different actors in the same run may carry different
    /// driver ids).
    pub driver_id: String,
    /// The retry caps persisted in this set's journal header (H3).
    pub caps: JournalCaps,
    /// The explicit network-wallet index range this set owns
    /// (`0..=7` for the standard single-set population).
    pub network_wallets: RangeInclusive<u32>,
    /// The explicit user-payment-wallet index range this set owns
    /// (`0..=3` for the standard single-set population).
    pub user_payment_wallets: RangeInclusive<u32>,
    /// Whether this set hosts the (single) bootstrap funding wallet.
    pub hosts_bootstrap_funding: bool,
    pub lookahead: u32,
    pub fee_policy: FeePolicy,
}

impl ProvisionConfig {
    /// The standard lab population: one bootstrap funding wallet, eight
    /// network wallets, four user payment wallets (plan sections
    /// 5.1/6.2), hosted by a single set.
    pub fn standard(run_seed: u64, run_id: &str, driver_id: &str) -> Self {
        Self {
            run_seed,
            network: Network::Regtest,
            run_id: run_id.to_string(),
            driver_id: driver_id.to_string(),
            caps: JournalCaps::default(),
            network_wallets: 0..=(STANDARD_NETWORK_WALLETS - 1),
            user_payment_wallets: 0..=(STANDARD_USER_PAYMENT_WALLETS - 1),
            hosts_bootstrap_funding: true,
            lookahead: DEFAULT_SCRIPT_LOOKAHEAD,
            fee_policy: FeePolicy::default(),
        }
    }

    /// The full role list (the bootstrap funding wallet only when this
    /// config hosts it).
    pub fn roles(&self) -> Vec<WalletRole> {
        let mut roles = Vec::new();
        if self.hosts_bootstrap_funding {
            roles.push(WalletRole::BootstrapFunding);
        }
        for index in self.network_wallets.clone() {
            roles.push(WalletRole::NetworkWallet { index });
        }
        for index in self.user_payment_wallets.clone() {
            roles.push(WalletRole::UserPayment { index });
        }
        roles
    }

    /// Validate a whole wallet-set allocation **before** any actor
    /// provisions (M3), and return the opaque validated token (R6).
    /// Fail-closed rules:
    ///
    /// - the allocation is non-empty, and every config's ids are valid
    ///   under the T0 id vocabulary and its caps in range;
    /// - exactly one config hosts the bootstrap funding wallet (there is
    ///   exactly one coinbase-minting host per run);
    /// - every config shares the same run seed, run id, network and
    ///   retry caps (one derivation domain and one retry policy per
    ///   run; driver ids may differ but must be unique);
    /// - the network-wallet index ranges are pairwise disjoint, and so
    ///   are the user-payment-wallet index ranges (no role is ever
    ///   derived by two sets - a double signer is structurally
    ///   impossible, not merely prevented at runtime).
    ///
    /// The returned [`ValidatedAllocation`] is the only production
    /// admission to `provision`/`reopen`: an unvalidated config cannot
    /// reach a journal (the token is opaque - no public constructor, no
    /// `Deserialize`).
    pub fn validate_allocation(
        configs: &[ProvisionConfig],
    ) -> Result<ValidatedAllocation, WalletError> {
        let invalid = |reason: String| WalletError::AllocationConflict(reason);
        if configs.is_empty() {
            return Err(invalid("allocation is empty".to_string()));
        }
        let bootstrap_hosts = configs
            .iter()
            .filter(|config| config.hosts_bootstrap_funding)
            .count();
        if bootstrap_hosts != 1 {
            return Err(invalid(format!(
                "allocation has {bootstrap_hosts} bootstrap-funding hosts (exactly one is required)"
            )));
        }
        let first = &configs[0];
        for (index, config) in configs.iter().enumerate() {
            if config.network != first.network {
                return Err(invalid(format!(
                    "config {index} network {} does not match the allocation network {}",
                    network_string(config.network),
                    network_string(first.network)
                )));
            }
            if config.run_seed != first.run_seed {
                return Err(invalid(format!(
                    "config {index} run seed {} does not match the allocation run seed {}",
                    config.run_seed, first.run_seed
                )));
            }
            if config.run_id != first.run_id {
                return Err(invalid(format!(
                    "config {index} run id {:?} does not match the allocation run id {:?}",
                    config.run_id, first.run_id
                )));
            }
            if config.caps != first.caps {
                return Err(invalid(format!(
                    "config {index} retry caps {:?} do not match the allocation caps {:?} \
                     (one run, one retry policy)",
                    config.caps, first.caps
                )));
            }
            crate::keys::validate_run_id(&config.run_id)
                .map_err(|e| invalid(format!("config {index} run id: {e}")))?;
            crate::keys::validate_run_id(&config.driver_id)
                .map_err(|e| invalid(format!("config {index} driver id: {e}")))?;
            config
                .caps
                .validate()
                .map_err(|e| invalid(format!("config {index}: {e}")))?;
            if config.roles().is_empty() {
                return Err(invalid(format!(
                    "config {index} owns no wallet role (empty index ranges and no bootstrap host)"
                )));
            }
        }
        for index in 0..configs.len() {
            for other in (index + 1)..configs.len() {
                if configs[index].driver_id == configs[other].driver_id {
                    return Err(invalid(format!(
                        "configs {index} and {other} share the driver id {:?} (one journal, \
                         one owner)",
                        configs[index].driver_id
                    )));
                }
                if ranges_overlap(
                    &configs[index].network_wallets,
                    &configs[other].network_wallets,
                ) {
                    return Err(invalid(format!(
                        "network-wallet index ranges of configs {index} and {other} overlap"
                    )));
                }
                if ranges_overlap(
                    &configs[index].user_payment_wallets,
                    &configs[other].user_payment_wallets,
                ) {
                    return Err(invalid(format!(
                        "user-payment-wallet index ranges of configs {index} and {other} overlap"
                    )));
                }
            }
        }
        // Canonical order for the digest and for driver-id lookup.
        let mut sorted = configs.to_vec();
        sorted.sort_by(|a, b| a.driver_id.cmp(&b.driver_id));
        let digest = allocation_digest(&sorted);
        Ok(ValidatedAllocation {
            configs: sorted,
            digest,
        })
    }
}

/// A **validated whole-run wallet-set allocation** (M3/R6).
///
/// Opaque by construction: the fields are private, there is no
/// `Deserialize` implementation and no public constructor other than
/// [`ProvisionConfig::validate_allocation`], so the only way to
/// provision or reopen a production wallet set is to hand the core a
/// complete allocation that already passed every fail-closed check.
/// The canonical allocation digest inside is computed by the
/// constructor - never supplied by a caller - and is persisted in every
/// journal header of the run and verified on reopen (a drifted
/// allocation cannot reopen a provisioned journal, even with identical
/// role ranges).
#[derive(Clone, Debug)]
pub struct ValidatedAllocation {
    /// The actor configs, canonically sorted by driver id.
    configs: Vec<ProvisionConfig>,
    /// SHA-256 over the canonical sorted-key JSON of the complete
    /// allocation (see [`allocation_digest`]).
    digest: String,
}

impl ValidatedAllocation {
    /// The canonical digest of the complete allocation (hex SHA-256).
    /// Persisted in the v2 journal header and verified on reopen.
    pub fn digest(&self) -> &str {
        &self.digest
    }

    /// The config of the actor identified by `driver_id`.
    fn config_for(&self, driver_id: &str) -> Result<&ProvisionConfig, WalletError> {
        self.configs
            .iter()
            .find(|config| config.driver_id == driver_id)
            .ok_or_else(|| {
                WalletError::AllocationConflict(format!(
                    "driver id {driver_id:?} is not part of the allocation"
                ))
            })
    }

    /// The implicit single-config allocation of a conformance test
    /// (test-only path behind `provision_for_conformance_tests` /
    /// `reopen_for_conformance_tests`, whose public signatures predate
    /// the token): the config itself is validated, but the cross-actor
    /// rules that a lone config cannot satisfy (exactly one bootstrap
    /// host **per allocation**, disjointness against the other actors)
    /// do not apply - a conformance test may exercise a single
    /// non-hosting actor in isolation. The production paths always take
    /// the fully validated token.
    pub(crate) fn for_conformance_tests(config: &ProvisionConfig) -> Result<Self, WalletError> {
        let invalid = |reason: String| WalletError::AllocationConflict(reason);
        crate::keys::validate_run_id(&config.run_id)
            .map_err(|e| invalid(format!("run id: {e}")))?;
        crate::keys::validate_run_id(&config.driver_id)
            .map_err(|e| invalid(format!("driver id: {e}")))?;
        config
            .caps
            .validate()
            .map_err(|e| invalid(format!("{e}")))?;
        if config.roles().is_empty() {
            return Err(invalid("provision config owns no wallet role".to_string()));
        }
        Ok(Self {
            configs: vec![config.clone()],
            digest: allocation_digest(std::slice::from_ref(config)),
        })
    }
}

/// The canonical allocation digest (R6): SHA-256 over the JSON encoding
/// of the complete allocation with every object key sorted
/// (`serde_json` maps are BTreeMaps, so `to_vec` of a `Value` is
/// canonical). The digest covers everything that defines the run's
/// role space: the shared identity (run seed, run id, network, retry
/// caps) and, per actor (sorted by driver id), the driver id, the
/// bootstrap-host flag, both index ranges as `[start, end]`, the
/// lookahead and the fee policy. This exact shape is the T0 v2-header
/// handoff (`adapter.rs`).
fn allocation_digest(sorted_configs: &[ProvisionConfig]) -> String {
    use serde_json::json;
    let first = &sorted_configs[0];
    let actors: Vec<serde_json::Value> = sorted_configs
        .iter()
        .map(|config| {
            json!({
                "driver_id": config.driver_id,
                "hosts_bootstrap_funding": config.hosts_bootstrap_funding,
                "network_wallets": [
                    *config.network_wallets.start(),
                    *config.network_wallets.end()
                ],
                "user_payment_wallets": [
                    *config.user_payment_wallets.start(),
                    *config.user_payment_wallets.end()
                ],
                "lookahead": config.lookahead,
                "fee_policy": &config.fee_policy,
            })
        })
        .collect();
    let value = json!({
        "run_seed": first.run_seed,
        "run_id": &first.run_id,
        "network": network_string(first.network),
        "caps": &first.caps,
        "actors": actors,
    });
    let canonical =
        serde_json::to_vec(&value).expect("the allocation digest serialization is infallible");
    use sha2::Digest as _;
    hex::encode(sha2::Sha256::digest(canonical))
}

fn ranges_overlap(a: &RangeInclusive<u32>, b: &RangeInclusive<u32>) -> bool {
    a.start() <= b.end() && b.start() <= a.end()
}

/// One durable payment request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaymentRequest {
    /// The durable command id (caller-chosen, deduplicated by the journal).
    pub command_id: String,
    pub payer_role: WalletRole,
    pub category: PaymentCategory,
    pub destinations: Vec<DestinationSpec>,
    pub fee_rate_sats_vb: u64,
}

/// The shareable outcome of a payment command.
#[derive(Clone, Debug)]
pub struct SignedPaymentView {
    pub command_id: String,
    pub txid: Txid,
    pub raw_hex: String,
    pub vbytes: u64,
    pub weight_wu: u64,
    pub fee_sats: u64,
    pub fee_rate_sats_vb: u64,
    pub category: PaymentCategory,
    pub payer_role: WalletRole,
    pub spent_outpoints: Vec<OutPoint>,
    pub destinations: Vec<DestinationSpec>,
    pub change_script_hex: Option<String>,
    /// The payer's change chain index the change output was derived at
    /// (`None` when the transaction has no change output).
    pub change_index: Option<u32>,
}

impl SignedPaymentView {
    fn from_signed(
        command_id: &str,
        signed: &SignedTrafficTx,
        payer_role: WalletRole,
        fee_rate_sats_vb: u64,
    ) -> Self {
        Self {
            command_id: command_id.to_string(),
            txid: signed.txid,
            raw_hex: hex::encode(&signed.raw_bytes),
            vbytes: signed.vbytes,
            weight_wu: signed.weight_wu,
            fee_sats: signed.fee_sats,
            fee_rate_sats_vb,
            category: signed.category.clone(),
            payer_role,
            spent_outpoints: signed.spent_outpoints.clone(),
            destinations: signed.destinations.clone(),
            change_script_hex: signed
                .change
                .as_ref()
                .map(|change| change.script_pubkey.to_hex_string()),
            change_index: signed.change.as_ref().map(|change| change.index),
        }
    }
}

/// The result of `sign_payment`.
#[derive(Clone, Debug)]
pub enum PaymentOutcome {
    /// Freshly signed and durably recorded (not yet broadcast).
    Signed(SignedPaymentView),
    /// A durable retry: the stored, byte-identical signed transaction.
    ReplayedStored(SignedPaymentView),
}

/// A provisioned traffic wallet set (one key file, one journal).
pub struct TrafficWalletSet {
    run_seed: u64,
    network: Network,
    run_id: String,
    driver_id: String,
    wallets: HashMap<WalletRole, TrafficWallet>,
    journal: TrafficJournal,
    fee_policy: FeePolicy,
    private_dir: PathBuf,
}

impl std::fmt::Debug for TrafficWalletSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrafficWalletSet")
            .field("network", &self.network)
            .field("run_id", &self.run_id)
            .field("driver_id", &self.driver_id)
            .field("wallet_roles", &self.wallets.len())
            .field("journal", &self.journal)
            .finish_non_exhaustive()
    }
}

impl TrafficWalletSet {
    /// Provision the wallet set of the actor identified by `driver_id`
    /// from a **validated allocation** (R6): private keys under
    /// `<run_dir>/private` (mode 0600, provision-once, bound to the run
    /// id) and a fresh v2-header journal bound to the run id, driver id,
    /// network, run-seed fingerprint, retry caps and the canonical
    /// allocation digest. Synthetic fixtures are refused by this
    /// production path. There is deliberately no config-only overload:
    /// the allocation must have passed
    /// [`ProvisionConfig::validate_allocation`] first.
    pub fn provision(
        allocation: &ValidatedAllocation,
        driver_id: &str,
        run_dir: &Path,
    ) -> Result<Self, Error> {
        let config = allocation.config_for(driver_id)?;
        Self::provision_inner(config, allocation.digest(), run_dir, false)
    }

    /// Provision a conformance-test wallet set (synthetic fixture funding
    /// is admitted by the journal). Production paths never call this.
    pub fn provision_for_conformance_tests(
        config: &ProvisionConfig,
        run_dir: &Path,
    ) -> Result<Self, Error> {
        let allocation = ValidatedAllocation::for_conformance_tests(config)?;
        Self::provision_inner(config, allocation.digest(), run_dir, true)
    }

    fn provision_inner(
        config: &ProvisionConfig,
        allocation_digest: &str,
        run_dir: &Path,
        allow_fixtures: bool,
    ) -> Result<Self, Error> {
        config.fee_policy.validate()?;
        let roles = config.roles();
        if roles.is_empty() {
            return Err(WalletError::AllocationConflict(
                "provision config owns no wallet role".to_string(),
            )
            .into());
        }
        let private_dir = run_dir.join("private");
        let key_store = TrafficKeyStore::provision(
            config.run_seed,
            &config.run_id,
            config.network,
            &roles,
            &private_dir,
        )?;
        let fingerprint = key_store.run_seed_fingerprint().to_string();
        let journal = if allow_fixtures {
            TrafficJournal::create_for_conformance_tests(
                run_dir,
                &network_string(config.network),
                &fingerprint,
                &config.run_id,
                &config.driver_id,
                allocation_digest,
                &config.caps,
            )?
        } else {
            TrafficJournal::create(
                run_dir,
                &network_string(config.network),
                &fingerprint,
                &config.run_id,
                &config.driver_id,
                allocation_digest,
                &config.caps,
            )?
        };
        let wallets = key_store.load_wallets(config.run_seed, config.lookahead)?;
        Ok(Self {
            run_seed: config.run_seed,
            network: config.network,
            run_id: config.run_id.clone(),
            driver_id: config.driver_id.clone(),
            wallets,
            journal,
            fee_policy: config.fee_policy.clone(),
            private_dir,
        })
    }

    /// Reopen the wallet set of the actor identified by `driver_id` from
    /// a **validated allocation** (R6): the key file is verified against
    /// the run seed and run id (fail closed, never regenerated) and the
    /// journal is replayed with full integrity checks. The replayed history-3
    /// header must bind the same network, run-seed fingerprint, run id,
    /// driver id, key mode, derivation version, protocol, retry caps
    /// and **allocation digest** as the allocation - any drift is a hard
    /// error raised BEFORE the session record is appended, so a
    /// mismatching reopen never mutates the journal.
    ///
    /// **Reopen is a writer operation** (R2). For a read-only committed view
    /// of an existing owner use `journal().observer_snapshot()`. A durable
    /// `SessionStarted` record is appended, conservatively moving every
    /// still-in-flight `BroadcastSent` transaction to `Unknown`; the
    /// wallet-level identity checks below it only ever conserve (they
    /// refuse the handle, they never relax uncertainty).
    pub fn reopen(
        allocation: &ValidatedAllocation,
        driver_id: &str,
        run_dir: &Path,
    ) -> Result<Self, Error> {
        let config = allocation.config_for(driver_id)?;
        Self::reopen_inner(config, allocation.digest(), run_dir, false)
    }

    /// Reopen a conformance-test wallet set.
    pub fn reopen_for_conformance_tests(
        config: &ProvisionConfig,
        run_dir: &Path,
    ) -> Result<Self, Error> {
        let allocation = ValidatedAllocation::for_conformance_tests(config)?;
        Self::reopen_inner(config, allocation.digest(), run_dir, true)
    }

    fn reopen_inner(
        config: &ProvisionConfig,
        allocation_digest: &str,
        run_dir: &Path,
        allow_fixtures: bool,
    ) -> Result<Self, Error> {
        config.fee_policy.validate()?;
        let private_dir = run_dir.join("private");
        let key_store = TrafficKeyStore::reopen(
            config.run_seed,
            &config.run_id,
            config.network,
            &private_dir,
        )?;
        let wallets = key_store.load_wallets(config.run_seed, config.lookahead)?;
        let journal = TrafficJournal::reopen_bound(
            run_dir,
            &config.caps,
            allocation_digest,
            &config.driver_id,
            (
                &config.run_id,
                key_store.run_seed_fingerprint(),
                &network_string(config.network),
            ),
            allow_fixtures,
        )?;
        // The journal header must belong to the same run: network, seed
        // fingerprint, run id, driver id, protocol, key mode, derivation
        // version and history version all bind (the caps were compared
        // inside reopen against the header).
        let header = journal.header()?;
        if header.network != network_string(config.network) {
            return Err(crate::error::KeyError::WrongNetwork {
                stored: header.network,
                expected: network_string(config.network),
            }
            .into());
        }
        if header.run_seed_fingerprint != key_store.run_seed_fingerprint() {
            return Err(crate::error::KeyError::FingerprintMismatch {
                role: "journal run seed".to_string(),
                stored: header.run_seed_fingerprint,
                derived: key_store.run_seed_fingerprint().to_string(),
            }
            .into());
        }
        if header.run_id != config.run_id {
            return Err(crate::error::KeyError::BindingMismatch {
                field: "journal run_id".to_string(),
                stored: header.run_id,
                expected: config.run_id.clone(),
            }
            .into());
        }
        if header.driver_id != config.driver_id {
            return Err(crate::error::KeyError::BindingMismatch {
                field: "journal driver_id".to_string(),
                stored: header.driver_id,
                expected: config.driver_id.clone(),
            }
            .into());
        }
        if header.protocol != PROTOCOL {
            return Err(crate::error::JournalError::Integrity(format!(
                "journal protocol {:?} is not {PROTOCOL:?}",
                header.protocol
            ))
            .into());
        }
        if header.history_version != SUPPORTED_HISTORY_VERSION {
            return Err(crate::error::JournalError::UnsupportedHistoryVersion {
                found: header.history_version,
                supported: SUPPORTED_HISTORY_VERSION,
            }
            .into());
        }
        if header.key_mode != KEY_MODE {
            return Err(crate::error::KeyError::KeyModeMismatch {
                stored: header.key_mode,
                expected: KEY_MODE.to_string(),
            }
            .into());
        }
        if header.derivation_version != DERIVATION_VERSION {
            return Err(crate::error::JournalError::DerivationVersionMismatch {
                found: header.derivation_version,
                expected: DERIVATION_VERSION,
            }
            .into());
        }
        // Every role in the config must be present in the reopened set.
        for role in config.roles() {
            if !wallets.contains_key(&role) {
                return Err(crate::error::KeyError::WalletSetMismatch {
                    stored: key_store
                        .roles()
                        .iter()
                        .map(|role| role.name())
                        .collect::<Vec<_>>()
                        .join(","),
                    expected: config
                        .roles()
                        .iter()
                        .map(|role| role.name())
                        .collect::<Vec<_>>()
                        .join(","),
                }
                .into());
            }
        }
        Ok(Self {
            run_seed: config.run_seed,
            network: config.network,
            run_id: config.run_id.clone(),
            driver_id: config.driver_id.clone(),
            wallets,
            journal,
            fee_policy: config.fee_policy.clone(),
            private_dir,
        })
    }

    pub fn network(&self) -> Network {
        self.network
    }

    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    pub fn driver_id(&self) -> &str {
        &self.driver_id
    }

    #[allow(dead_code)]
    pub fn run_seed(&self) -> u64 {
        self.run_seed
    }

    pub fn roles(&self) -> Vec<WalletRole> {
        let mut roles: Vec<WalletRole> = self.wallets.keys().cloned().collect();
        roles.sort();
        roles
    }

    pub fn journal(&self) -> &TrafficJournal {
        &self.journal
    }

    pub fn fee_policy(&self) -> &FeePolicy {
        &self.fee_policy
    }

    pub fn private_dir(&self) -> &Path {
        &self.private_dir
    }

    fn wallet(&self, role: &WalletRole) -> Result<&TrafficWallet, Error> {
        self.wallets
            .get(role)
            .ok_or_else(|| WalletError::OwnerNotInSet(role.name()).into())
    }

    /// Record a funding observation with explicit provenance (H5). The
    /// owner must be part of this set. **The funding script is not a
    /// caller input**: this set derives it at the observation's recorded
    /// `(chain, index)` and hands the derived script to the journal, so
    /// a foreign script can never be claimed through this API. The
    /// spending path re-derives and re-compares the script a second time
    /// (M2), so both admission and spending are keyed to the same
    /// derivation locus.
    pub fn observe_funding(&self, observation: &FundingObservation) -> Result<(), Error> {
        let wallet = self.wallet(&observation.owner_role)?;
        let derived = wallet.script_at(observation.chain, observation.index)?;
        Ok(self
            .journal
            .observe_funding(observation, &derived.script_pubkey)?)
    }

    /// The next deterministic receive address of a role.
    ///
    /// Reservation semantics (T2-L): the index returned is this role's
    /// next not-yet-observed receive index. It is **reserved by the next
    /// funding observation journaled for this role** (each observation
    /// consumes exactly one index) - calling this method alone reserves
    /// nothing and writes nothing. The bootstrap mining loop derives the
    /// address here, pays the coinbase output to it, and then journals
    /// the observation (source `BootstrapCoinbase` with its block
    /// context); the two steps together make the index durably consumed.
    pub fn next_receive_address(
        &self,
        role: &WalletRole,
    ) -> Result<crate::keys::AddressScript, Error> {
        let index = self.journal.next_receive_index(role);
        Ok(self.wallet(role)?.script_at(Chain::Receive, index)?)
    }

    /// The available (unreserved, unspent, non-invalidated, mature)
    /// outpoints of a role, in deterministic order, each with its
    /// derivation locus `(chain, index)` and script (M2: the spending
    /// path resolves the key by direct derivation at that locus, never
    /// by scanning).
    pub fn available_outpoints(
        &self,
        role: &WalletRole,
    ) -> Vec<(OutPoint, u64, Chain, u32, ScriptBuf)> {
        self.journal.available_outpoints(role)
    }

    /// The available balance of a role (sats).
    pub fn balance_sats(&self, role: &WalletRole) -> u64 {
        self.journal.balance_sats(role)
    }

    /// Build a checked destination from an explicit address: the address
    /// must be valid for regtest, and its script must be standard P2PKH,
    /// P2WPKH or P2TR and at least dust - all verified BEFORE any
    /// journal record or signature exists. When the script belongs to one
    /// of this set's wallets (within the look-ahead), the destination is
    /// owner-tagged so a later confirmation credits it.
    ///
    /// This is the domain API the T0 `SendUserPayment` command maps onto
    /// (its explicit mandatory `destination_address`): no implicit
    /// destination rule exists anywhere in this core.
    ///
    /// Note: testnet and regtest share their base58 P2PKH address
    /// version bytes (so a base58 address structurally valid for one
    /// parses for the other), while their bech32 hrps differ (`tb`
    /// vs `bcrt`); the run's persisted network field carries the
    /// binding.
    pub fn checked_destination(
        &self,
        address: bitcoin::Address<bitcoin::address::NetworkUnchecked>,
        amount_sats: u64,
    ) -> Result<DestinationSpec, Error> {
        let address = address
            .require_network(bitcoin::Network::Regtest)
            .map_err(|_| WalletError::DestinationNetworkWrong {
                expected: network_string(bitcoin::Network::Regtest),
            })?;
        let script = address.script_pubkey();
        if !crate::builder::supported_destination_script(&script) {
            return Err(crate::error::BuilderError::UnsupportedDestination {
                script_hex: script.to_hex_string(),
            }
            .into());
        }
        if amount_sats < self.fee_policy.dust_sats {
            return Err(crate::error::BuilderError::DustDestination {
                amount: amount_sats,
                dust: self.fee_policy.dust_sats,
            }
            .into());
        }
        let owner = self
            .wallets
            .values()
            .find(|wallet| wallet.owns_script(&script))
            .map(|wallet| wallet.role().clone());
        Ok(DestinationSpec {
            script_pubkey: script,
            amount_sats,
            owner,
        })
    }

    /// The payload digest of a payment request: SHA-256 over the
    /// canonical `RequestRecord` encoding (the same digest the journal
    /// computes itself in `begin_command`). The same command id plus the
    /// same digest implies byte-identical intent - including the same
    /// payer role and the same category.
    pub fn payload_digest(request: &PaymentRequest) -> String {
        request_record_digest(&Self::request_record(request))
    }

    fn request_record(request: &PaymentRequest) -> RequestRecord {
        RequestRecord {
            construction_effect: None,
            payer_role: request.payer_role.clone(),
            category: request.category.clone(),
            destinations: request
                .destinations
                .iter()
                .map(|destination| crate::journal::DestinationRecord {
                    script_hex: destination.script_pubkey.to_hex_string(),
                    amount_sats: destination.amount_sats,
                    owner_role: destination.owner.clone(),
                })
                .collect(),
            fee_rate_sats_vb: request.fee_rate_sats_vb,
        }
    }

    /// Sign (or replay) one durable payment command. The command either
    /// builds and records a signed transaction, or fails closed with the
    /// reservation explicitly released; an `unknown` broadcast outcome
    /// never releases inputs (that is the journal's lifecycle contract,
    /// H1).
    pub fn sign_payment(&self, request: &PaymentRequest) -> Result<PaymentOutcome, Error> {
        self.sign_payment_with_construction(request, None)
    }

    /// Closed lab-acceptance case, selected before the existing signing path.
    /// The controller must authorize the operator command; the actor additionally
    /// requires its explicit lab_acceptance configuration. No version argument.
    pub fn sign_version_rejection_for_acceptance(
        &self,
        request: &PaymentRequest,
        enabled: bool,
    ) -> Result<PaymentOutcome, Error> {
        if !enabled {
            return Err(
                WalletError::InvalidCommandId("acceptance_construction_disabled".into()).into(),
            );
        }
        self.sign_payment_with_construction(
            request,
            Some(crate::construction::ConstructionEffect::VersionRejectionV1),
        )
    }

    pub(crate) fn sign_payment_with_construction(
        &self,
        request: &PaymentRequest,
        effect: Option<crate::construction::ConstructionEffect>,
    ) -> Result<PaymentOutcome, Error> {
        if effect.is_some()
            && (request.category != PaymentCategory::UserPayment
                || !matches!(request.payer_role, WalletRole::UserPayment { .. }))
        {
            return Err(WalletError::InvalidCommandId(
                "acceptance_construction_role_or_category".into(),
            )
            .into());
        }
        self.validate_request(request)?;
        let secp = Secp256k1::new();
        let mut record = Self::request_record(request);
        record.construction_effect = effect;

        let begin = self.journal.begin_command(&request.command_id, record)?;
        let stored = match begin {
            CommandBeginOutcome::New => None,
            CommandBeginOutcome::Stored(stored) => Some(*stored),
        };

        match stored {
            None => self.sign_fresh(&secp, request, effect),
            Some(stored) => self.sign_from_stored(&secp, request, stored),
        }
    }

    fn validate_request(&self, request: &PaymentRequest) -> Result<(), Error> {
        if request.command_id.trim().is_empty() {
            return Err(WalletError::InvalidCommandId(request.command_id.clone()).into());
        }
        if !self.wallets.contains_key(&request.payer_role) {
            return Err(WalletError::OwnerNotInSet(request.payer_role.name()).into());
        }
        if request.destinations.is_empty() {
            return Err(WalletError::InvalidCommandId(format!(
                "command {} has no destinations",
                request.command_id
            ))
            .into());
        }
        for destination in &request.destinations {
            // Script class, dust and ownership are all rejected here,
            // BEFORE any journal record or signature exists (fail
            // closed; the T0 `SendUserPayment.destination_address`
            // maps through `checked_destination`).
            crate::builder::validate_destination_script(destination)?;
            if destination.amount_sats < self.fee_policy.dust_sats {
                return Err(crate::error::BuilderError::DustDestination {
                    amount: destination.amount_sats,
                    dust: self.fee_policy.dust_sats,
                }
                .into());
            }
            if let Some(owner) = &destination.owner {
                if !self.wallets.contains_key(owner) {
                    return Err(WalletError::DestinationWalletNotInSet(owner.name()).into());
                }
            }
        }
        if request.fee_rate_sats_vb < self.fee_policy.min_rate_sats_vb
            || request.fee_rate_sats_vb > self.fee_policy.max_rate_sats_vb
        {
            return Err(crate::error::BuilderError::FeeRateOutsidePolicy {
                rate: request.fee_rate_sats_vb,
                min_rate: self.fee_policy.min_rate_sats_vb,
                max_rate: self.fee_policy.max_rate_sats_vb,
            }
            .into());
        }
        Ok(())
    }

    /// Fresh path: deterministic selection, reservation, single build.
    fn sign_fresh(
        &self,
        secp: &Secp256k1<bitcoin::secp256k1::All>,
        request: &PaymentRequest,
        effect: Option<crate::construction::ConstructionEffect>,
    ) -> Result<PaymentOutcome, Error> {
        let wallet = self.wallet(&request.payer_role)?;
        let available = self.available_outpoints(&request.payer_role);
        if available.is_empty() {
            return Err(crate::error::BuilderError::InsufficientFunds {
                needed: 0,
                available: 0,
            }
            .into());
        }
        let destinations_total =
            crate::money::checked_sum_sats(request.destinations.iter().map(|d| d.amount_sats))?;

        // Deterministic order, then the smallest prefix whose total covers
        // destinations + an upper-bound fee + dust. The upper bound makes
        // the subsequent build accept the first reservation, so inputs are
        // selected exactly once and reserved before any signature exists.
        // Keys resolve by direct derivation at each outpoint's recorded
        // (chain, index) - no scan (M2).
        let ordered = deterministic_utxo_order(
            available
                .iter()
                .map(|(outpoint, amount, chain, index, script)| {
                    SignableUtxo::resolve_indexed(
                        wallet, *outpoint, *chain, *index, script, *amount,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?,
        );
        let mut selected: Vec<SignableUtxo> = Vec::new();
        let mut selected_total: u64 = 0;
        let mut chosen = false;
        for take in 1..=ordered.len() {
            let candidate = &ordered[..take];
            let bound = selection_fee_upper_bound(
                &request.destinations,
                request.fee_rate_sats_vb,
                candidate.len(),
                true,
            );
            let needed = checked_add_sats(
                destinations_total,
                checked_add_sats(bound, self.fee_policy.dust_sats)?,
            )?;
            selected_total =
                crate::money::checked_sum_sats(candidate.iter().map(|u| u.amount_sats))?;
            if selected_total >= needed {
                selected = candidate.to_vec();
                chosen = true;
                break;
            }
        }
        if !chosen {
            // The whole wallet cannot fund the payment at this rate.
            return Err(crate::error::BuilderError::InsufficientFunds {
                needed: checked_add_sats(
                    destinations_total,
                    selection_fee_upper_bound(
                        &request.destinations,
                        request.fee_rate_sats_vb,
                        ordered.len(),
                        true,
                    ),
                )?,
                available: selected_total,
            }
            .into());
        }

        let change_index = self.journal.next_change_index(&request.payer_role);
        let outpoints: Vec<OutPoint> = selected.iter().map(|utxo| utxo.outpoint).collect();
        // Durable reservation BEFORE signing.
        self.journal
            .reserve(&request.command_id, &outpoints, change_index)?;
        let signed = match build_and_sign_with_construction(
            secp,
            wallet,
            &selected,
            &request.destinations,
            request.fee_rate_sats_vb,
            &self.fee_policy,
            change_index,
            request.category.clone(),
            effect,
        ) {
            Ok(signed) => signed,
            Err(error) => {
                // No signed bytes exist: releasing is safe and explicit
                // (an `unknown` broadcast result never takes this path).
                self.journal.release_failed_command(
                    &request.command_id,
                    &format!("construction failed: {error}"),
                )?;
                return Err(error.into());
            }
        };
        self.journal
            .record_signed(&request.command_id, &signed, request.fee_rate_sats_vb)?;
        Ok(PaymentOutcome::Signed(SignedPaymentView::from_signed(
            &request.command_id,
            &signed,
            request.payer_role.clone(),
            request.fee_rate_sats_vb,
        )))
    }

    /// Stored path: replay, resume, or reject, per the stored phase.
    fn sign_from_stored(
        &self,
        secp: &Secp256k1<bitcoin::secp256k1::All>,
        request: &PaymentRequest,
        stored: StoredCommand,
    ) -> Result<PaymentOutcome, Error> {
        match stored.phase {
            CommandPhase::Signed | CommandPhase::Finished => {
                let txid = stored.txid.ok_or_else(|| {
                    WalletError::InvalidCommandId(format!(
                        "command {} is in phase {} without a signed transaction",
                        stored.command_id,
                        stored.phase.as_str()
                    ))
                })?;
                let raw_hex = stored.raw_hex.clone().ok_or_else(|| {
                    WalletError::InvalidCommandId(format!(
                        "command {} has a txid but no stored raw bytes",
                        stored.command_id
                    ))
                })?;
                let facts = self
                    .journal
                    .tx_facts(&txid)
                    .ok_or_else(|| crate::error::JournalError::UnknownTx(txid.to_string()))?;
                Ok(PaymentOutcome::ReplayedStored(SignedPaymentView {
                    command_id: stored.command_id.clone(),
                    txid,
                    raw_hex,
                    vbytes: facts.vbytes,
                    weight_wu: facts.weight_wu,
                    fee_sats: facts.fee_sats,
                    fee_rate_sats_vb: facts.fee_rate_sats_vb,
                    category: facts.category,
                    payer_role: stored.wallet_role.clone(),
                    spent_outpoints: facts.spent_outpoints,
                    destinations: request.destinations.clone(),
                    change_script_hex: None,
                    change_index: facts.change_index,
                }))
            }
            // Crash between begin and reserve: nothing is locked yet, so
            // selection runs fresh (same request, deterministic order).
            CommandPhase::Began => {
                self.sign_fresh(secp, request, stored.request.construction_effect)
            }
            // Crash between reserve and signed: re-sign with the SAME
            // reserved outpoints and the SAME change index - never
            // reselect. Keys resolve by direct derivation at each
            // outpoint's recorded (chain, index) - no scan (M2).
            CommandPhase::Reserved => {
                let wallet = self.wallet(&request.payer_role)?;
                let change_index = stored.change_index.ok_or_else(|| {
                    WalletError::InvalidCommandId(format!(
                        "command {} is reserved without a change index",
                        stored.command_id
                    ))
                })?;
                let mut utxos = Vec::with_capacity(stored.reserved_outpoints.len());
                for outpoint in &stored.reserved_outpoints {
                    let (amount, script, chain, index, owner) =
                        self.journal.outpoint_facts(outpoint).ok_or_else(|| {
                            crate::error::JournalError::UnknownTx(outpoint.to_string())
                        })?;
                    if owner != request.payer_role {
                        return Err(crate::error::JournalError::OutpointOwnerMismatch {
                            outpoint: outpoint.to_string(),
                            owner: owner.name(),
                            wallet: request.payer_role.name(),
                        }
                        .into());
                    }
                    utxos.push(SignableUtxo::resolve_indexed(
                        wallet, *outpoint, chain, index, &script, amount,
                    )?);
                }
                let signed = match build_and_sign_with_construction(
                    secp,
                    wallet,
                    &utxos,
                    &request.destinations,
                    request.fee_rate_sats_vb,
                    &self.fee_policy,
                    change_index,
                    request.category.clone(),
                    stored.request.construction_effect,
                ) {
                    Ok(signed) => signed,
                    Err(error) => {
                        self.journal.release_failed_command(
                            &request.command_id,
                            &format!("resume construction failed: {error}"),
                        )?;
                        return Err(error.into());
                    }
                };
                self.journal.record_signed(
                    &request.command_id,
                    &signed,
                    request.fee_rate_sats_vb,
                )?;
                Ok(PaymentOutcome::Signed(SignedPaymentView::from_signed(
                    &request.command_id,
                    &signed,
                    request.payer_role.clone(),
                    request.fee_rate_sats_vb,
                )))
            }
        }
    }

    // ------------------------------------------------------------------
    // Lifecycle orchestration surface (T1b/T3 drive these; the journal
    // enforces the state machine, these are thin, typed wrappers).
    // ------------------------------------------------------------------

    /// Record that the stored bytes left the process. Returns the
    /// monotonic send sequence.
    pub fn note_broadcast_sent(&self, command_id: &str) -> Result<u64, Error> {
        Ok(self.journal.record_broadcast_sent(command_id)?)
    }

    pub fn note_accepted_by_rpc(&self, command_id: &str, node_id: &str) -> Result<(), Error> {
        Ok(self.journal.record_accepted_by_rpc(command_id, node_id)?)
    }

    pub fn note_observed_in_mempool(&self, command_id: &str, node_id: &str) -> Result<(), Error> {
        Ok(self
            .journal
            .record_observed_in_mempool(command_id, node_id)?)
    }

    /// Record canonical confirmation and finish the command (M7: the
    /// finish is guarded, the record is not - a late confirmation
    /// observation for an already-finished command, e.g. after a reorg
    /// cycle, is journaled without error and without a second finish;
    /// an illegal transition still fails closed).
    pub fn note_confirmed(
        &self,
        command_id: &str,
        block_hash: BlockHash,
        height: u64,
    ) -> Result<(), Error> {
        self.journal
            .record_confirmed(command_id, block_hash, height)?;
        if self.journal.command_phase(command_id) != Some(CommandPhase::Finished) {
            self.journal.finish_command(command_id, "confirmed")?;
        }
        Ok(())
    }

    /// Record a node rejection and finish the command (S4: the record
    /// is NOT guarded - only the finish is, mirroring
    /// `note_confirmed`/`note_conflicted`). A genuine rejection of an
    /// already-finished command - e.g. a
    /// `bad-txns-inputs-missingorspent` answer arriving after a reorg
    /// resubmission - is always journaled (the journal-level transition
    /// rules remain the sole legality gate; a resubmitted payment
    /// degrades its inputs to possibly-spent, never releases them) and
    /// the command keeps its original terminal outcome with no second
    /// finish. Classifying node answers is the driver's (T1b's)
    /// concern, not the core's: an "already known / already in block
    /// chain" answer is NOT a rejection of the funding claim - T1b
    /// maps it to a confirmation observation (`note_confirmed` with
    /// the canonical block); the core never parses the code string to
    /// infer canonicality.
    pub fn note_rejected(&self, command_id: &str, code: &str) -> Result<(), Error> {
        self.journal.record_rejected(command_id, code)?;
        if self.journal.command_phase(command_id) != Some(CommandPhase::Finished) {
            self.journal
                .finish_command(command_id, &format!("rejected:{code}"))?;
        }
        Ok(())
    }

    /// Record an unknown outcome (bounded same-bytes retries). Returns the
    /// attempt number.
    pub fn note_unknown(&self, command_id: &str, reason: &str) -> Result<u32, Error> {
        Ok(self.journal.record_unknown(command_id, reason)?)
    }

    /// Record a terminal conflict and finish the command (M7: the finish
    /// is guarded, the record is not - a late conflict observation for an
    /// already-finished command is journaled without error and without a
    /// second finish; an illegal transition still fails closed).
    pub fn note_conflicted(&self, command_id: &str, reason: &str) -> Result<(), Error> {
        self.journal.record_conflicted(command_id, reason)?;
        if self.journal.command_phase(command_id) != Some(CommandPhase::Finished) {
            self.journal.finish_command(command_id, "conflicted")?;
        }
        Ok(())
    }

    /// Record canonical displacement (possibly-spent claims preserved).
    pub fn note_reorged(
        &self,
        command_id: &str,
        from_block_hash: BlockHash,
        height: u64,
    ) -> Result<(), Error> {
        Ok(self
            .journal
            .record_reorged(command_id, from_block_hash, height)?)
    }

    /// Bounded same-bytes reorg resubmission (eligibility fail-closed).
    pub fn reorg_resubmit(
        &self,
        command_id: &str,
        raw_bytes: &[u8],
        proven_unspent: &[OutPoint],
    ) -> Result<u64, Error> {
        Ok(self
            .journal
            .record_reorg_resubmit(command_id, raw_bytes, proven_unspent)?)
    }

    /// Record a canonical chain observation (H5): the tip drives coinbase
    /// maturity, and blocks in `displaced_block_hashes` take every own
    /// confirmed transaction and canonical funding entry through the
    /// reorg transition (the derived cascade then invalidates
    /// descendants). An empty displaced list is a pure tip advance.
    pub fn note_chain_observed(
        &self,
        tip_block_hash: BlockHash,
        tip_height: u64,
        displaced_block_hashes: &[BlockHash],
    ) -> Result<(), Error> {
        Ok(self.journal.record_chain_observed(
            tip_block_hash,
            tip_height,
            displaced_block_hashes,
        )?)
    }

    /// Record the pause acknowledgment barrier (no broadcasts until
    /// `resume_sends`).
    pub fn pause_sends(&self, command_id: &str, monotonic_ms: u64) -> Result<(), Error> {
        Ok(self.journal.record_send_barrier(command_id, monotonic_ms)?)
    }

    /// Resume sends after a barrier.
    pub fn resume_sends(&self, command_id: &str) -> Result<(), Error> {
        Ok(self.journal.record_send_resumed(command_id)?)
    }
}

fn network_string(network: Network) -> String {
    match network {
        Network::Regtest => "regtest".to_string(),
        Network::Testnet => "testnet".to_string(),
        Network::Testnet4 => "testnet4".to_string(),
        Network::Signet => "signet".to_string(),
        Network::Bitcoin => "bitcoin".to_string(),
    }
}
