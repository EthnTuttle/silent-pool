//! The agent wallet role (SPEC 11.5, 11.8; plan 3.3, 4.1.1, 4.4, 4.7).
//!
//! AP-3a scope - the persistent foundation:
//!
//! - load exactly the provisioned key material (scan/ark branch xprvs
//!   + BIP 85 child seed; never the master or the spend branch);
//! - open (or first-bind) the persistent `U_i -> u_i` keystore, VTXO
//!   ledger, registration counter/journal and scan cursor, plus the
//!   stock bark persister foundation (`bark_store`);
//! - drive the registration state machine against a transport seam
//!   (AP-2 owns the `0x5350` codec; without it the role is honestly
//!   **dormant** - a persistent registration ack is not a live
//!   connection - and it never mines);
//! - expose the honest loopback status JSON.
//!
//! Explicitly **not** in AP-3a (AP-3b): opening the full stock bark
//! `Wallet` (it needs a live chain source; the persister foundation
//! here is the stock sqlite + `init_wallet` + reopen validation only),
//! live balances, the chain scan, live mailbox/status RPCs, delegated
//! refresh and offboard, and the `0x5350` wire transport.

mod aggregate_api;
mod aggregate_receive;
#[cfg(test)]
mod aggregate_tests;
#[cfg(all(test, feature = "postgres-integration"))]
mod aggregate_join_proof_tests;
mod aggregate_watch;
#[cfg(test)]
mod audit_regressions;
pub mod bark_store;
pub mod keys;
pub mod live_bark;
pub mod live_receive;
pub mod live_registration;
pub mod maintenance;
mod native_audit;
mod native_audit_io;
pub mod native_audit_service;
pub mod native_source_audit;
pub mod provision;
pub mod receive;
pub mod registration;
pub mod round;
pub mod scan;
pub mod state;
pub mod status;
mod ui_inputs;

pub use bark_store::{BarkStore, BarkStoreError};
pub use keys::{KeyError, KeySources, WalletKeys};
pub use receive::{ReceiveBackend, ReceiveError, ReceiveLedger, Reconciliation, VtxoRecord};
pub use registration::{
    Attempt, Backoff, EpochFallback, MailboxMode, RegistrationError, RegistrationFatal,
    RegistrationManager, RegistrationOutcome, RegistrationPayload, RegistrationTransport,
};
pub use state::{StoreIdentity, WalletStateError, WalletStore};
pub use status::{StatusServeHandle, StatusService, StatusSnapshot, WalletState};

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use spc_crypto::Network;

use crate::config::AgentConfig;

/// Blocks before expiry at which a VTXO needs a refresh (the lab
/// profile; bark's default is 144).
pub const REFRESH_THRESHOLD_BLOCKS: u32 = 96;

/// Everything the wallet role needs, resolved from the agent config.
/// Built by [`WalletSettings::from_config`]; validated fail-closed.
#[derive(Clone, Debug)]
pub struct WalletSettings {
    pub scan_xprv: PathBuf,
    pub ark_xprv: PathBuf,
    pub child_seed: PathBuf,
    pub datadir: PathBuf,
    pub status_listen: SocketAddr,
    pub mailbox_mode: MailboxMode,
    pub epoch_fallback: EpochFallback,
    /// The Ark server id the registration targets (hex, 4 bytes).
    pub ark_id: [u8; 4],
    pub network: Network,
    pub sp_address: String,
}

impl WalletSettings {
    /// Resolve the wallet role's settings from the agent config. Fails
    /// closed when the `[ark]` section does not fully describe the
    /// provisioned material.
    pub fn from_config(config: &AgentConfig) -> anyhow::Result<Self> {
        let ark = config
            .ark
            .as_ref()
            .filter(|ark| ark.enabled)
            .ok_or_else(|| anyhow::anyhow!("the wallet role requires [ark] enabled"))?;

        let mailbox_mode = MailboxMode::parse_str(&ark.mailbox_mode)
            .ok_or_else(|| anyhow::anyhow!("unknown [ark] mailbox_mode '{}'", ark.mailbox_mode))?;
        let epoch_fallback = EpochFallback::parse_str(&ark.epoch_fallback).ok_or_else(|| {
            anyhow::anyhow!("unknown [ark] epoch_fallback '{}'", ark.epoch_fallback)
        })?;
        match mailbox_mode {
            // AP-3a registers M for the derived modes.
            MailboxMode::EpochDerived | MailboxMode::None | MailboxMode::Static => {}
            // Static needs the Ark server's mailbox pubkey S_mb for the
            // blinding hint; epoch_registered needs RegisterEpochMailboxes.
            // Both land with the live transport (AP-3b/AP-2).
            other => {
                anyhow::bail!(
                    "mailbox_mode '{}' registration needs live Ark server fields; \
                     not wired until AP-3b (use 'epoch_derived')",
                    other.as_str()
                )
            }
        }

        let scan_xprv = required_path(ark.scan_xprv.as_deref(), "ark.scan_xprv")?;
        let ark_xprv = required_path(ark.ark_xprv.as_deref(), "ark.ark_xprv")?;
        let child_seed = required_path(ark.seed_child.as_deref(), "ark.seed_child")?;
        let datadir = required_path(ark.datadir.as_deref(), "ark.datadir")?;
        let ark_id = parse_ark_id(ark.ark_id.as_deref())?;

        Ok(WalletSettings {
            scan_xprv,
            ark_xprv,
            child_seed,
            datadir,
            status_listen: ark.status_socket_addr()?,
            mailbox_mode,
            epoch_fallback,
            ark_id,
            network: config.ark_network(),
            sp_address: config.upstream.sp_address.clone(),
        })
    }
}

fn required_path(path: Option<&std::path::Path>, name: &str) -> anyhow::Result<PathBuf> {
    path.map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("the wallet role requires {name}"))
}

fn parse_ark_id(text: Option<&str>) -> anyhow::Result<[u8; 4]> {
    let text =
        text.ok_or_else(|| anyhow::anyhow!("the wallet role requires ark.ark_id (8 hex chars)"))?;
    let bytes = hex::decode(text.trim())
        .map_err(|_| anyhow::anyhow!("ark.ark_id must be 8 hex chars, got '{text}'"))?;
    bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("ark.ark_id must be exactly 8 hex chars, got '{text}'"))
}

/// The running wallet role.
pub struct WalletRole {
    _writer: std::fs::File,
    keys: WalletKeys,
    store: Arc<WalletStore>,
    /// The stock bark persister foundation (AP-3b builds the full
    /// `Wallet` on top of it).
    bark: BarkStore,
    ledger: ReceiveLedger,
    registration: Mutex<RegistrationManager>,
    status: Arc<StatusService>,
    server: Option<StatusServeHandle>,
    shutdown_tx: tokio::sync::watch::Sender<bool>,
    /// The initial receiver, kept alive so a `shutdown()` before `run`
    /// still stores the value (a `watch` send with no receiver is a
    /// no-op error, which would silently drop the request).
    shutdown_rx: tokio::sync::watch::Receiver<bool>,
}

impl WalletRole {
    /// Load keys, open (or first-bind) persistent state - the agent
    /// store and the stock bark persister foundation - and bind the
    /// loopback status server. Fails closed on any validation error -
    /// a failed start regenerates nothing and never wipes state.
    pub async fn start(settings: WalletSettings) -> anyhow::Result<Self> {
        Self::start_with_mailbox_key(settings, None).await
    }

    async fn start_with_mailbox_key(
        settings: WalletSettings,
        server_mailbox: Option<bitcoin::secp256k1::PublicKey>,
    ) -> anyhow::Result<Self> {
        let sources = KeySources {
            scan_xprv: settings.scan_xprv.clone(),
            ark_xprv: settings.ark_xprv.clone(),
            child_seed: settings.child_seed.clone(),
            sp_address: settings.sp_address.clone(),
            network: settings.network,
        };
        let keys = WalletKeys::load(&sources)?;
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
        if !settings.datadir.exists() {
            std::fs::DirBuilder::new()
                .mode(0o700)
                .recursive(true)
                .create(&settings.datadir)?;
        }
        let meta = std::fs::symlink_metadata(&settings.datadir)?;
        anyhow::ensure!(
            meta.is_dir() && meta.permissions().mode() & 0o777 == 0o700,
            "wallet datadir must be a private regular directory"
        );
        let writer = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .mode(0o600)
            .open(settings.datadir.join("writer.lock"))?;
        writer
            .try_lock()
            .map_err(|_| anyhow::anyhow!("wallet already has a writer"))?;
        tracing::info!(
            network = keys::network_name(keys.network),
            wallet_fingerprint = %keys.wallet_fingerprint,
            "wallet role: provisioned keys loaded and validated"
        );

        if settings.datadir.join("bark/spc-initialized").exists()
            && !settings.datadir.join("state.sqlite").is_file()
        {
            anyhow::bail!("Bark initialization exists but SP wallet state is missing; refusing partial wallet reset");
        }
        let store = Arc::new(WalletStore::open(
            &settings.datadir,
            &keys.store_identity(),
        )?);
        store.vtxo_records()?; // Refuse AP-3a synthetic IDs before opening Bark.
                               // Cross-binding (fix-r2, Fable A): a `state.sqlite` that
                               // records a completed bark initialization together with a
                               // wholly absent `bark/` directory is partial datadir damage,
                               // not a deliberate reset (the reset removes the whole datadir,
                               // both databases together). Refuse before the persister would
                               // silently re-initialize a fresh wallet.
        if store.bark_initialized()? && !settings.datadir.join("bark/db.sqlite").is_file() {
            anyhow::bail!(
                "agent state at {} records an initialized bark wallet but {} is \
                 missing: partial datadir damage, refusing to re-initialize (the \
                 deliberate reset removes the whole datadir)",
                settings.datadir.display(),
                settings.datadir.join("bark").display()
            );
        }
        // The stock bark persister foundation: same datadir, separate
        // `<datadir>/bark` database owned by the child seed. Nothing
        // here opens a full bark `Wallet` or talks to a chain/server.
        let bark = BarkStore::open(
            &settings.datadir,
            keys::bitcoin_network(keys.network),
            keys.wallet_fingerprint,
        )
        .await?;
        // Record (or heal) the cross-binding after a validated open:
        // fresh or reopen, the persister under this datadir is
        // initialized.
        store.set_bark_initialized()?;

        let mut payload = Self::registration_payload(&keys, &settings)?;
        if settings.mailbox_mode != MailboxMode::None {
            if let Some(server_mailbox) = server_mailbox {
                payload.mailbox_dh = Some(spc_crypto::mailbox_blinding_hint(
                    &keys.b_ark,
                    &server_mailbox,
                )?);
            }
        }
        let digest = payload.local_digest();
        let registered = store.is_registered(&digest)?;
        tracing::info!(
            datadir = %settings.datadir.display(),
            registered,
            last_request_id = store.last_request_id().unwrap_or(0),
            "wallet role: persistent state open"
        );

        let ledger = ReceiveLedger::new(Arc::clone(&store), REFRESH_THRESHOLD_BLOCKS);
        let registration = RegistrationManager::new(Arc::clone(&store), payload);

        let status = Arc::new(StatusService::new(
            Arc::clone(&store),
            REFRESH_THRESHOLD_BLOCKS,
            settings.mailbox_mode,
            digest,
        ));
        let server = status.serve(settings.status_listen).await?;
        tracing::info!(addr = %server.addr, "wallet role: status endpoint bound (loopback)");

        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        // Honest initial state (registered != connected): a persistent
        // ack does not make the role connected - only a wired transport
        // with the payload live does, which `run` determines.
        status.set_state(if registered {
            WalletState::Dormant
        } else {
            WalletState::AwaitingRegistration
        });
        Ok(WalletRole {
            _writer: writer,
            keys,
            store,
            bark,
            ledger,
            registration: Mutex::new(registration),
            status,
            server: Some(server),
            shutdown_tx,
            shutdown_rx,
        })
    }

    fn registration_payload(
        keys: &WalletKeys,
        settings: &WalletSettings,
    ) -> anyhow::Result<RegistrationPayload> {
        Ok(match settings.mailbox_mode {
            MailboxMode::EpochDerived => RegistrationPayload::epoch_derived(
                keys.sp_id,
                settings.ark_id,
                keys.b_ark_public(),
                keys.mailbox_public(),
                settings.epoch_fallback,
            ),
            MailboxMode::Static => RegistrationPayload {
                sp_id: keys.sp_id,
                version: registration::REGISTRATION_VERSION,
                ark_id: settings.ark_id,
                b_ark: keys.b_ark_public(),
                mailbox_mode: MailboxMode::Static,
                mailbox_pubkey: Some(keys.mailbox_public()),
                mailbox_dh: None,
                epoch_fallback: settings.epoch_fallback,
            },
            MailboxMode::None => RegistrationPayload {
                sp_id: keys.sp_id,
                version: registration::REGISTRATION_VERSION,
                ark_id: settings.ark_id,
                b_ark: keys.b_ark_public(),
                mailbox_mode: MailboxMode::None,
                mailbox_pubkey: None,
                mailbox_dh: None,
                epoch_fallback: settings.epoch_fallback,
            },
            // Filtered out by WalletSettings::from_config; unreachable.
            other => anyhow::bail!("mailbox mode {} is not wired in AP-3a", other.as_str()),
        })
    }

    /// The status service (snapshot/state access for tests and
    /// embedders).
    pub fn status(&self) -> &Arc<StatusService> {
        &self.status
    }

    /// The loaded keys (public accessors only in practice; recovery
    /// checks use the fingerprints and public derivation parity).
    pub fn keys(&self) -> &WalletKeys {
        &self.keys
    }

    /// The persistent store handle.
    pub fn store(&self) -> &Arc<WalletStore> {
        &self.store
    }

    /// The stock bark persister foundation (AP-3b builds the full
    /// `Wallet` on top of it).
    pub fn bark(&self) -> &BarkStore {
        &self.bark
    }

    /// The registration payload digest the effective-binding answers
    /// are evaluated against (status `registered`, store checks).
    pub fn registration_digest(&self) -> [u8; 32] {
        self.with_registration(|r| r.payload().local_digest())
    }

    /// The receive ledger (validation/persistence over the store).
    pub fn ledger(&self) -> &ReceiveLedger {
        &self.ledger
    }

    /// The bound loopback status address, if the server is running.
    pub fn status_addr(&self) -> Option<std::net::SocketAddr> {
        self.server.as_ref().map(|s| s.addr)
    }

    /// Access the registration state machine.
    pub fn with_registration<R>(&self, f: impl FnOnce(&mut RegistrationManager) -> R) -> R {
        let mut guard = self
            .registration
            .lock()
            .expect("registration mutex poisoned");
        f(&mut guard)
    }

    /// Request a graceful shutdown. `run` observes it, moves the status
    /// to `stopped`, releases the status port and returns.
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
    }

    /// A cloneable shutdown signal for embedders that spawn [`run`]
    /// onto a task and must be able to stop it without owning the
    /// role (the role is consumed by `run`).
    pub fn shutdown_handle(&self) -> tokio::sync::watch::Sender<bool> {
        self.shutdown_tx.clone()
    }

    /// Run until shutdown, a fatal registration outcome, or counter
    /// exhaustion.
    ///
    /// **Dormancy (honest state)**: with `None` transport the role
    /// reports `dormant` even when a registration was acknowledged
    /// persistently - `registered` is the durable journal answer, not a
    /// live connection, and a real connection is never invented. The
    /// role never mines in this state.
    ///
    /// **Fatal stops (the terminal-loop fix)**: a hard terminal
    /// (`bad-signature`, `wrong-ark-server`, ...), counter exhaustion,
    /// or a second consecutive terminal for the same payload stops the
    /// attempts, moves the status to `error` with the exact code, and
    /// makes `run` return `Err` - retries resume only after an
    /// explicit operator restart. The wedge kinds
    /// (`mismatch`/`stale`/`superseded`) get exactly one fresh
    /// same-payload retry first.
    ///
    /// **Backoff**: every non-success outcome participates
    /// (1, 2, 4, 8, 16, 30 s, cap 30 s); after six failed attempts
    /// local `registration_unavailable` is surfaced while the 30 s
    /// retries continue.
    pub async fn run(
        mut self,
        transport: Option<Box<dyn RegistrationTransport>>,
    ) -> anyhow::Result<()> {
        let mut shutdown_rx = self.shutdown_rx.clone();
        let digest = self.registration_digest();

        let already_registered = self.store.is_registered(&digest)?;
        let mut fatal: Option<RegistrationFatal> = None;

        match transport {
            None => {
                // Honest dormancy: the 0x5350 client codec is AP-2's
                // (lab-ark-registration); until it lands there is
                // nothing to send, a persistent ack is not a live
                // connection, and the role never mines.
                self.status.set_state(WalletState::Dormant);
                self.status.set_error(Some(
                    "registration transport not wired in this build (AP-2); \
                     dormant - holding persistent state, never mining"
                        .into(),
                ));
                tracing::warn!(
                    registered = already_registered,
                    "wallet role: no registration transport wired (AP-2); \
                     dormant and never mining"
                );
                // A shutdown requested before `run` subscribed is caught
                // by the borrow; one after, by `changed`.
                while !*shutdown_rx.borrow() {
                    if shutdown_rx.changed().await.is_err() {
                        break;
                    }
                }
            }
            Some(mut transport) => {
                tracing::info!("wallet role: registration loop starting");
                loop {
                    if *shutdown_rx.borrow() {
                        break;
                    }
                    if self
                        .with_registration(|r| r.last_outcome().is_some_and(|o| o.is_registered()))
                        && self.store.is_registered(&digest)?
                    {
                        self.status.set_state(WalletState::Connected);
                        self.status.set_error(None);
                        while !*shutdown_rx.borrow() {
                            if shutdown_rx.changed().await.is_err() {
                                break;
                            }
                        }
                        break;
                    }
                    let attempt = {
                        let mut guard = self
                            .registration
                            .lock()
                            .expect("registration mutex poisoned");
                        guard.attempt(transport.as_mut())
                    };
                    let attempt = match attempt {
                        Ok(attempt) => attempt,
                        Err(RegistrationError::State(e)) => return Err(e.into()),
                        Err(RegistrationError::Fatal {
                            request_id,
                            error_code,
                        }) => {
                            // The manager refused to continue (a second
                            // fatal attempt after the one that tripped
                            // it): surface and stop.
                            fatal = Some(RegistrationFatal {
                                error_code,
                                request_id,
                            });
                            break;
                        }
                    };
                    match &attempt.outcome {
                        RegistrationOutcome::Applied | RegistrationOutcome::AlreadyApplied => {
                            tracing::info!(
                                request_id = attempt.request_id,
                                outcome = attempt.outcome.as_str(),
                                "wallet role: registration committed"
                            );
                        }
                        outcome => {
                            tracing::warn!(
                                request_id = attempt.request_id,
                                outcome = outcome.as_str(),
                                error_code = ?outcome.error_code(),
                                "wallet role: registration attempt not applied"
                            );
                        }
                    }
                    // A fatal outcome stops the loop after this
                    // iteration (the attempt itself is journaled).
                    if let Some(f) = self.with_registration(|r| r.fatal().cloned()) {
                        fatal = Some(f);
                        break;
                    }
                    // After six failed attempts surface local
                    // registration_unavailable; the 30 s retries
                    // continue regardless. Before that, per-attempt
                    // errors surface with their own codes (a transport
                    // failure is reported as such, not prematurely as
                    // exhaustion).
                    if self.with_registration(|r| r.backoff().exhausted()) {
                        self.status.set_error(Some(
                            "registration_unavailable (6+ failed attempts); \
                             retrying at the 30s cap"
                                .into(),
                        ));
                    } else {
                        match &attempt.outcome {
                            RegistrationOutcome::Unavailable => {
                                self.status.set_error(Some(format!(
                                    "registration transport unavailable (attempt {}); retrying",
                                    attempt.request_id
                                )));
                            }
                            outcome => {
                                if let Some(code) = outcome.error_code() {
                                    self.status.set_error(Some(code));
                                }
                            }
                        }
                    }
                    // Backoff (plan 4.1.1): 1, 2, 4, 8, 16, 30 s, cap
                    // 30 s. Retries continue; never mining.
                    let delay = self.with_registration(|r| r.backoff().delay_secs()).max(1);
                    tokio::select! {
                        _ = tokio::time::sleep(std::time::Duration::from_secs(delay)) => {}
                        _ = shutdown_rx.changed() => break,
                    }
                }
            }
        }

        if let Some(fatal) = fatal {
            // Fatal: error state with the exact code, attempts stopped,
            // status port released, and an explicit Err to the caller.
            // Recovery requires an operator restart by design.
            self.status.set_error(Some(fatal.error_code.clone()));
            self.status.set_state(WalletState::Error);
            if let Some(server) = self.server.take() {
                server.stop().await;
            }
            tracing::error!(
                error_code = %fatal.error_code,
                request_id = fatal.request_id,
                "wallet role: registration fatally stopped; operator restart required"
            );
            return Err(anyhow::anyhow!(
                "wallet role: registration fatally stopped after request {}: {} \
                 (attempts cease until an explicit operator restart)",
                fatal.request_id,
                fatal.error_code
            ));
        }

        self.status.set_state(WalletState::Stopped);
        if let Some(server) = self.server.take() {
            server.stop().await;
        }
        tracing::info!("wallet role: stopped");
        Ok(())
    }
}

/// Entry point for `--roles wallet` (feature `ark`): resolve settings
/// from the config and run the role. AP-3a has no live `0x5350`
/// transport, so this runs dormant (see [`WalletRole::run`]).
pub async fn run_from_config(config: AgentConfig) -> anyhow::Result<()> {
    let settings = WalletSettings::from_config(&config)?;
    anyhow::ensure!(settings.mailbox_mode == MailboxMode::EpochDerived,
        "live wallet receive currently requires epoch_derived; refusing an unconsumed delivery mode");
    let ark = config.ark.as_ref().expect("validated wallet settings");
    let mailbox = ark
        .mailbox_pubkey
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("ark.mailbox_pubkey required"))?
        .parse()?;
    anyhow::ensure!(
        settings.epoch_fallback == EpochFallback::Static,
        "hold fallback requires epoch_registered"
    );
    let expected_id = settings.ark_id;
    let mut aggregate_api = ark
        .aggregate_api
        .as_ref()
        .map(|cfg| aggregate_api::PoolApi::new(cfg, settings.network))
        .transpose()?;
    let ui_datadir = settings.datadir.clone();
    let mut role = WalletRole::start_with_mailbox_key(settings, Some(mailbox)).await?;
    let mut native = native_audit::NativeRuntime::start(&config).await?;
    let work = async {
        if let (Some(client), Some(cfg)) = (&aggregate_api, &ark.aggregate_api) {
            role.store
                .bind_aggregate_api(&role.keys.sp_id, &cfg.origin, &client.order_key)?;
        }
        let ui_inputs = ui_inputs::Publisher::open(&ui_datadir).ok();
        if let Some(publisher) = &ui_inputs {
            let _ = publisher.unavailable();
        }
        let live = live_bark::LiveBark::open(&role.keys, &role.bark, ark, expected_id).await?;
        role.status.set_live_balance_fresh(false);
        let mut connection = None;
        loop {
            // Invalidate before any new validation or command/reservation
            // mutation. Publication below only follows this tick's proofs.
            if let Some(publisher) = &ui_inputs {
                let _ = publisher.unavailable();
            }
            role.status.set_state(WalletState::AwaitingRegistration);
            let digest = role.registration_digest();
            role.store.reserve_registration_revalidation(&digest)?;
            let id = role.store.reserve_registration_request_id(&digest)?;
            let payload = role.with_registration(|r| r.payload().clone());
            let exchange = tokio::time::timeout(std::time::Duration::from_secs(15), async {
                live.check_connection().await?;
                if connection.is_none() {
                    connection = Some(live_registration::LiveRegistration::connect(&config).await?);
                }
                connection
                    .as_mut()
                    .expect("connected")
                    .exchange(&payload, id, &role.keys.b_scan)
                    .await
            })
            .await;
            let outcome = match exchange {
                Ok(Ok(outcome)) => outcome,
                _ => {
                    connection = None;
                    RegistrationOutcome::Unavailable
                }
            };
            // The async exchange used the already durable reservation. Feed its
            // correlated result through the SAME AP-3a journal/terminal policy.
            struct Received(Option<RegistrationOutcome>, u32);
            impl RegistrationTransport for Received {
                fn send(
                    &mut self,
                    _: &RegistrationPayload,
                    id: u32,
                ) -> anyhow::Result<RegistrationOutcome> {
                    anyhow::ensure!(
                        self.1 == id,
                        "reservation changed during registration exchange"
                    );
                    self.0
                        .take()
                        .ok_or_else(|| anyhow::anyhow!("reply already consumed"))
                }
            }
            let attempt =
                role.with_registration(|r| r.attempt(&mut Received(Some(outcome), id)))?;
            if let Some(fatal) = role.with_registration(|r| r.fatal().cloned()) {
                anyhow::bail!("registration stopped: {}", fatal.error_code);
            }
            let mut balances_fresh = false;
            let delay = if attempt.outcome.is_registered() {
                role.status.set_live_registration();
                role.status.set_error(None);
                role.status.set_live_balance_fresh(false);
                if let Some(client) = &mut aggregate_api {
                    let aggregate_result =
                        tokio::time::timeout(std::time::Duration::from_secs(10), async {
                            let mut failed = match client
                                .status(&role.keys, &config.upstream.sp_address)
                                .await
                            {
                                Ok(status) => role
                                    .store
                                    .ingest_aggregate_status(&role.keys.sp_id, &status)
                                    .is_err(),
                                Err(_) => true,
                            };
                            for order in role.store.aggregate_fetch_work()? {
                                role.store.begin_aggregate_fetch(&order.order_id)?;
                                let attempt = async {
                                    let frozen = client
                                        .order(
                                            &role.keys,
                                            &config.upstream.sp_address,
                                            &order.order_id,
                                        )
                                        .await?;
                                    role.store
                                        .retain_frozen_aggregate_order(&order.order_id, &frozen)?;
                                    let receipt = client
                                        .receipt(
                                            &role.keys,
                                            &config.upstream.sp_address,
                                            &order.order_id,
                                        )
                                        .await?;
                                    receipt.grant.verify_signature(&client.order_key)?;
                                    role.store
                                        .retain_candidate_receipt(&order.order_id, &receipt)?;
                                    live.receive_aggregate(
                                        &role.keys,
                                        &role.store,
                                        &order,
                                        &receipt,
                                        &client.order_key,
                                    )
                                    .await?;
                                    Ok::<_, anyhow::Error>(())
                                }
                                .await;
                                failed |= attempt.is_err();
                            }
                            anyhow::ensure!(!failed, "aggregate status or receipt unavailable");
                            Ok::<_, anyhow::Error>(())
                        })
                        .await;
                    if !matches!(aggregate_result, Ok(Ok(()))) {
                        role.status.set_error(Some(
                            "aggregate_status_or_verified_receipt_unavailable".into(),
                        ));
                    }
                } else {
                    role.status
                        .set_error(Some("aggregate_api_unconfigured_watches_retained".into()));
                }
                match tokio::time::timeout(
                    std::time::Duration::from_secs(10),
                    live.scan_tick(&role.keys, &role.store, &role.ledger),
                )
                .await
                {
                    Ok(Ok(())) => balances_fresh = true,
                    _ => role
                        .status
                        .set_error(Some("receive_or_chain_proof_unavailable".into())),
                }
                15
            } else {
                role.status.set_error(attempt.outcome.error_code());
                role.with_registration(|r| {
                    let index = r.backoff().failures().saturating_sub(1) as usize;
                    registration::BACKOFF_SCHEDULE_SECS[index.min(5)]
                })
            };
            role.status.set_live_balance_fresh(false);
            if live
                .process_commands(
                    &role.keys,
                    &role.store,
                    &role.ledger,
                    &role
                        .bark
                        .db_path()
                        .parent()
                        .and_then(|p| p.parent())
                        .expect("wallet datadir")
                        .join("commands"),
                )
                .await
                .is_err()
            {
                role.status
                    .set_error(Some("wallet_operation_unavailable".into()));
            } else if !role.store.has_uncertain_operations()? {
                if let Ok(Ok(balance)) = tokio::time::timeout(
                    std::time::Duration::from_secs(10),
                    live.proven_stock_balance(),
                )
                .await
                {
                    role.status.set_verified_stock_balance(balance);
                    role.status.set_live_balance_fresh(balances_fresh);
                    if balances_fresh {
                        if let Some(publisher) = &ui_inputs {
                            let publication = async {
                                let pending = live.wallet.pending_round_input_vtxos().await?;
                                publisher.publish(
                                    &live.ui_observations,
                                    &role.store,
                                    pending.iter().map(|v| v.id().to_bytes().to_vec()),
                                )
                            }
                            .await;
                            if publication.is_err() {
                                let _ = publisher.unavailable();
                                role.status
                                    .set_error(Some("wallet_ui_inputs_unavailable".into()));
                            }
                        }
                    }
                }
            }
            tokio::select! {
                _ = tokio::signal::ctrl_c() => break,
                _ = tokio::time::sleep(std::time::Duration::from_secs(delay)) => {},
            }
        }
        if let Some(publisher) = &ui_inputs {
            let _ = publisher.unavailable();
        }
        Ok::<_, anyhow::Error>(())
    };
    let result = if let Some(runtime) = &mut native {
        tokio::select! {
            result = work => result,
            result = runtime.serve_owner(&role.keys) => result,
        }
    } else {
        work.await
    };
    let result = if let Some(runtime) = native {
        let stopped = runtime.stop().await;
        result.and(stopped)
    } else {
        result
    };
    role.status.set_state(if result.is_ok() {
        WalletState::Stopped
    } else {
        WalletState::Error
    });
    if let Some(server) = role.server.take() {
        server.stop().await;
    }
    result
}
