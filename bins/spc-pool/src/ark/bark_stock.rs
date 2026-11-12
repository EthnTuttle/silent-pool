//! The stock `bark-wallet` client behind the [`BarkClient`] seam.
//!
//! Everything bark-shaped lives here and nowhere else: the transport
//! ([`super::bark_transport`]) and the board loop
//! ([`super::board`]) see only plain data, so their rules are testable
//! without a wallet, a server or a chain (decision record
//! 2026-11-02-bark-adapter.md, clauses 2-3). This module is compiled only
//! behind the `ark` feature, at the UPSTREAM_DEPS.md pin; stock bark, no
//! patches, no forks.
//!
//! Startup is fail-closed (decision record, clause 7): the seed file is
//! hex, 64 bytes, grants nothing to group/other (bark `fs_perms`
//! convention: 0o600 file, 0o700 directory), and must not resolve under
//! the SPc-side secret store - a seed readable from the SPc side
//! collapses the role split (SPEC §11.3, §11.10).
//!
//! # The thin arkoor send
//!
//! Stock `Wallet::send_arkoor_payment` is unusable for this role for a
//! structural reason, not a quality one: its `finalize` step persists
//! only the *change* VTXOs of a send, the destination VTXOs never enter
//! the sender's wallet, and no public send API returns them - so a
//! caller cannot learn the VTXO ids a receipt must carry, and a
//! before/after wallet diff sees nothing. This module therefore drives
//! the same flow over fully-public bark APIs (validate, select, lock,
//! cosign, register, deliver, finalize) and returns the destination
//! VTXOs it created, which is exactly the receipt's raw material. The
//! flow mirrors `bark::actions::arkoor_send` step for step; only the
//! action-checkpoint persistence is replaced by this role's own
//! order-id dedupe, which is the restart-durable equivalent.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ark::arkoor::package::{ArkoorPackageBuilder, ArkoorPackageCosignResponse};
use ark::arkoor::ArkoorDestination;
use ark::vtxo::{Full, Vtxo};
use ark::{ProtocolEncoding, VtxoPolicy};
use bark::vtxo::{VtxoLockHolder, WalletVtxo};
use bitcoin::{Amount, Network};

use super::bark_transport::{ArkPayment, BarkClient, TrackedVtxo};
use super::send_fence::CapacityError;

/// The actual stock-wallet rows and supported fee quote, never a balance
/// target/config scalar. Kept shared with guarded send so preflight and send
/// apply the same state/expiry/exit-depth/fee eligibility and largest-first rule.
pub(crate) fn select_ready_inputs(
    mut candidates: Vec<WalletVtxo>,
    tip: u32,
    max_exit_depth: u16,
    sats: u64,
    quote: &bark::FeeEstimate,
) -> Result<Vec<WalletVtxo>, CapacityError> {
    let amount = Amount::from_sat(sats);
    if sats == 0
        || quote.net_amount != amount
        || amount.checked_add(quote.fee) != Some(quote.gross_amount)
    {
        return Err(CapacityError::ReadinessUnavailable);
    }
    candidates.retain(|v| {
        v.state.kind() == bark::vtxo::VtxoStateKind::Spendable
            && v.vtxo.expiry_height().to_u32() > tip
            && v.exit_depth < max_exit_depth
    });
    candidates.sort_by_key(|v| std::cmp::Reverse(v.vtxo.amount().to_sat()));
    let mut selected = Vec::new();
    let mut total = 0u64;
    let required = quote.gross_amount.to_sat();
    let mut ids = std::collections::BTreeSet::new();
    for v in candidates {
        if total >= required {
            break;
        }
        if !ids.insert(v.id()) {
            return Err(CapacityError::ReadinessUnavailable);
        }
        total = total
            .checked_add(v.vtxo.amount().to_sat())
            .ok_or(CapacityError::ReadinessUnavailable)?;
        selected.push(v);
    }
    if total < required {
        return Err(CapacityError::InsufficientFloat);
    }
    // The pinned stock estimate explicitly reports zero arkoor fee. This thin
    // package builder has no fee-output primitive: never guess or silently
    // waive a newly nonzero quote, even if the gross amount could be funded.
    if quote.fee != Amount::ZERO {
        return Err(CapacityError::UnsupportedFeePolicy);
    }
    Ok(selected)
}

/// The SPc-side secret-store root (order key, Noise keys, database
/// credentials). The ark seed must not be resolvable under it.
pub const SPC_SECRET_ROOT: &str = "/run/secrets/spc";

/// Whether `mode` grants any group or other access (bark's `is_loose`).
#[cfg(unix)]
fn is_loose(mode: u32) -> bool {
    mode & 0o077 != 0
}

/// Whether the seed at `path` resolves under the secret-store `root`.
/// Pure so the startup rule is testable without `/run/secrets/spc`
/// existing in the test environment.
fn seed_under_store(path: &Path, root: &Path) -> bool {
    // Both sides resolved: "resolvable under" follows symlinks. A root
    // that does not exist cannot contain the seed.
    match (std::fs::canonicalize(path), std::fs::canonicalize(root)) {
        (Ok(p), Ok(r)) => p.starts_with(r),
        _ => false,
    }
}

/// Load the ark seed (SPEC §11.3 "Independent key"): 64 bytes as hex.
/// Fails closed on anything ambiguous - unreadable, wrong length,
/// non-hex, group/other-readable file or store directory, or a location
/// that resolves under the SPc-side secret store.
pub fn load_seed(path: &Path) -> Result<[u8; 64], String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let meta =
            std::fs::metadata(path).map_err(|e| format!("ark seed {}: {e}", path.display()))?;
        if !meta.is_file() {
            return Err(format!("ark seed {} is not a regular file", path.display()));
        }
        let mode = meta.permissions().mode() & 0o777;
        if is_loose(mode) {
            return Err(format!(
                "ark seed {} is readable by group/other (mode {mode:03o}); expected 0o600 (bark fs_perms)",
                path.display()
            ));
        }
        // The containing directory is the secret store: it must be
        // traversable by the owner only.
        if let Some(dir) = path.parent() {
            if let Ok(dmeta) = std::fs::metadata(dir) {
                let dmode = dmeta.permissions().mode() & 0o777;
                if is_loose(dmode) {
                    return Err(format!(
                        "ark seed directory {} is accessible by group/other (mode {dmode:03o}); expected 0o700 (bark fs_perms)",
                        dir.display()
                    ));
                }
            }
        }
        if seed_under_store(path, Path::new(SPC_SECRET_ROOT)) {
            return Err(format!(
                "ark seed {} resolves under the SPc-side secret store {SPC_SECRET_ROOT}; the ark seed MUST NOT share the SPc-side secret store",
                path.display()
            ));
        }
        let hex = std::fs::read_to_string(path)
            .map_err(|e| format!("ark seed {}: {e}", path.display()))?;
        let bytes = hex::decode(hex.trim())
            .map_err(|e| format!("ark seed {} is not valid hex: {e}", path.display()))?;
        let seed: [u8; 64] = bytes.try_into().map_err(|v: Vec<u8>| {
            format!(
                "ark seed {} must be 64 bytes of hex (128 chars), got {} bytes",
                path.display(),
                v.len()
            )
        })?;
        Ok(seed)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err("the ark role requires unix file permissions".to_string())
    }
}

/// Everything [`open_bark_wallet`] needs. The bitcoind coordinates come
/// from `[template]` (the same node the pool templates coinbases
/// against); the wallet rides its RPC for chain data.
pub struct BarkWalletOptions {
    pub network: Network,
    pub seed: [u8; 64],
    /// Bark datadir (wallet sqlite, lock files); created 0o700.
    pub datadir: PathBuf,
    pub server_address: String,
    pub bitcoind_url: String,
    pub bitcoind_user: String,
    pub bitcoind_pass: String,
    /// Used when the chain backend has no estimate yet (bark
    /// `fallback_fee_rate`, sats/vb).
    pub fallback_fee_sat_vb: u32,
}

/// Open (or first-create) the stock bark wallet and wrap it in the
/// [`BarkClient`] seam. Mirrors the wallet-construction pattern of
/// bark's own ark-testing builders: sqlite persister, BDK on-chain
/// wallet over the same seed, `create_if_not_exists`.
pub async fn open_bark_wallet(opts: BarkWalletOptions) -> Result<BarkWalletClient, String> {
    super::wallet_control::prepare_wallet_database(&opts.datadir).map_err(|e| e.to_string())?;

    let seed = bark::WalletSeed::new_from_seed(opts.network, &opts.seed);
    let db: Arc<dyn bark::persist::BarkPersister> = Arc::new(
        bark::persist::sqlite::SqliteClient::open(opts.datadir.join("wallet.sqlite"))
            .map_err(|e| format!("ark wallet db: {e}"))?,
    );
    let onchain = Arc::new(tokio::sync::RwLock::new(
        bark::onchain::OnchainWallet::load_or_create(opts.network, opts.seed, db.clone())
            .await
            .map_err(|e| format!("ark onchain wallet: {e}"))?,
    ));

    let mut config = bark::Config::network_default(opts.network);
    config.server_address = opts.server_address.clone();
    config.bitcoind_address = Some(opts.bitcoind_url);
    config.bitcoind_user = Some(opts.bitcoind_user);
    config.bitcoind_pass = Some(bark::secret::Secret::new(opts.bitcoind_pass));
    config.fallback_fee_rate = Some(bitcoin::FeeRate::from_sat_per_vb_u32(
        opts.fallback_fee_sat_vb,
    ));

    // run_daemon stays off: this role drives sync and maintenance itself
    // (the board loop's tick), so liveness is observable rather than
    // spread across a background task we do not control.
    let wallet = bark::Wallet::open(
        opts.network,
        seed,
        config,
        bark::OpenWalletArgs {
            datadir: Some(opts.datadir.clone()),
            persister: Some(db.clone()),
            onchain: Some(onchain.clone()),
            create_if_not_exists: true,
            run_daemon: false,
            ..Default::default()
        },
    )
    .await
    .map_err(|e| format!("opening the ark bark wallet: {e}"))?;

    Ok(BarkWalletClient {
        wallet,
        db,
        onchain,
        network: opts.network,
        server_address: opts.server_address,
        fallback_fee_sat_vb: opts.fallback_fee_sat_vb,
        board_keypair: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
        operation_lock: Arc::new(tokio::sync::Mutex::new(())),
    })
}

/// [`BarkClient`] over a stock bark [`bark::Wallet`]. Clones share the
/// wallet and the board-keypair cache (the transport and the board loop
/// each hold one clone of the same client).
#[derive(Clone)]
pub struct BarkWalletClient {
    wallet: bark::Wallet,
    /// The wallet's persister: the thin send stores its change key index
    /// the same way stock bark's action does.
    pub(super) db: Arc<dyn bark::persist::BarkPersister>,
    pub(super) onchain: Arc<tokio::sync::RwLock<bark::onchain::OnchainWallet>>,
    network: Network,
    /// The ark server address from `[ark]` (a fresh [`server_rpc`]
    /// connection per send; the wallet's own RPCs reuse its config).
    server_address: String,
    fallback_fee_sat_vb: u32,
    /// The board keypair shared between `board_funding_address` and the
    /// next `board_psbt` (decision record, clause 3). Stock bark's
    /// `derive_store_next_keypair` ADVANCES the store index on every
    /// call: without the cache, the PSBT would pay a funding address
    /// derived from a different key than the one `board_psbt` registers,
    /// and bark would reject it ("PSBT output does not pay to the
    /// expected board funding address"). Consumed on a successful board;
    /// kept on failure so a retry reuses the same funding address.
    board_keypair: std::sync::Arc<tokio::sync::Mutex<Option<secp256k1::Keypair>>>,
    /// Held across an entire automatic tick, manual command, or float send.
    pub operation_lock: Arc<tokio::sync::Mutex<()>>,
}

impl BarkWalletClient {
    async fn bounded_plan_send(
        &self,
        payment: &ArkPayment,
    ) -> Result<Vec<WalletVtxo>, CapacityError> {
        tokio::time::timeout(std::time::Duration::from_secs(30), self.plan_send(payment))
            .await
            .map_err(|_| CapacityError::ReadinessDeadline)?
    }
    async fn plan_send(&self, payment: &ArkPayment) -> Result<Vec<WalletVtxo>, CapacityError> {
        self.wallet
            .validate_arkoor_address(&payment.address)
            .await
            .map_err(|_| CapacityError::ReadinessUnavailable)?;
        let info = self
            .wallet
            .ark_info()
            .await
            .map_err(|_| CapacityError::ReadinessUnavailable)?
            .ok_or(CapacityError::ReadinessUnavailable)?;
        if info.fees.validate().is_err()
            || info
                .max_vtxo_amount
                .is_some_and(|max| payment.sats > max.to_sat())
        {
            return Err(CapacityError::ReadinessUnavailable);
        }
        let quote = self
            .wallet
            .estimate_arkoor_payment_fee(Amount::from_sat(payment.sats))
            .await
            .map_err(|_| CapacityError::ReadinessUnavailable)?;
        let tip = self
            .wallet
            .chain()
            .tip()
            .await
            .map_err(|_| CapacityError::ReadinessUnavailable)?
            .to_u32();
        let candidates = self
            .wallet
            .spendable_vtxos()
            .await
            .map_err(|_| CapacityError::ReadinessUnavailable)?;
        let selected = select_ready_inputs(
            candidates,
            tip,
            info.max_vtxo_exit_depth,
            payment.sats,
            &quote,
        )?;
        let full = self
            .wallet
            .get_full_vtxos(selected.iter())
            .await
            .map_err(|_| CapacityError::ReadinessUnavailable)?;
        if full.len() != selected.len() {
            return Err(CapacityError::ReadinessUnavailable);
        }
        let mut anchors = std::collections::BTreeMap::new();
        let mut seen = std::collections::BTreeSet::new();
        for vtxo in &full {
            let row = selected
                .iter()
                .find(|row| row.id() == vtxo.id())
                .ok_or(CapacityError::ReadinessUnavailable)?;
            if !seen.insert(vtxo.id())
                || vtxo.to_bare() != row.vtxo
                || vtxo.exit_depth() != row.exit_depth
                || vtxo.server_pubkey() != info.server_pubkey
            {
                return Err(CapacityError::ReadinessUnavailable);
            }
            let id = vtxo.chain_anchor().txid;
            if let std::collections::btree_map::Entry::Vacant(entry) = anchors.entry(id) {
                let tx = self
                    .wallet
                    .chain()
                    .get_tx(&id)
                    .await
                    .map_err(|_| CapacityError::ReadinessUnavailable)?
                    .ok_or(CapacityError::ReadinessUnavailable)?;
                entry.insert(tx);
            }
            vtxo.validate(&anchors[&id])
                .map_err(|_| CapacityError::ReadinessUnavailable)?;
            let mut key = self
                .wallet
                .get_vtxo_key(vtxo)
                .await
                .map_err(|_| CapacityError::ReadinessUnavailable)?;
            let owned = key.public_key() == vtxo.user_pubkey();
            key.non_secure_erase();
            if !owned {
                return Err(CapacityError::ReadinessUnavailable);
            }
        }
        let (mut change, _) = self
            .wallet
            .peek_next_keypair()
            .await
            .map_err(|_| CapacityError::ReadinessUnavailable)?;
        let change_public = change.public_key();
        change.non_secure_erase();
        if payment.address.policy().user_pubkey() == change_public {
            return Err(CapacityError::ReadinessUnavailable);
        }
        // Pure package construction validates the pinned stock policy/input and
        // change shape. No input registration, lock, nonce or cosign occurs.
        let _preview = ArkoorPackageBuilder::new_single_output_with_checkpoints(
            full,
            ArkoorDestination {
                total_amount: Amount::from_sat(payment.sats),
                policy: payment.address.policy().clone(),
            },
            VtxoPolicy::new_pubkey(change_public),
        )
        .map_err(|_| CapacityError::ReadinessUnavailable)?;
        Ok(selected)
    }
    /// Shared with wallet-control startup/manual/automatic/recovery operations.
    pub fn operation_lock(&self) -> Arc<tokio::sync::Mutex<()>> {
        self.operation_lock.clone()
    }
    pub async fn acquire_send_capacity(
        &self,
    ) -> Result<super::send_fence::WalletOperationPermit, super::send_fence::CapacityError> {
        super::send_fence::WalletOperationPermit::acquire_for_send(self.operation_lock()).await
    }
    /// The wallet (for startup introspection such as the network
    /// cross-check).
    pub fn wallet(&self) -> &bark::Wallet {
        &self.wallet
    }

    /// The cached board keypair, deriving and caching one on first use.
    async fn board_keypair(&self) -> Result<secp256k1::Keypair, String> {
        let mut slot = self.board_keypair.lock().await;
        if let Some(kp) = *slot {
            return Ok(kp);
        }
        let (kp, index) = self
            .wallet
            .derive_store_next_keypair()
            .await
            .map_err(|e| format!("deriving a board keypair: {e}"))?;
        tracing::debug!(index, "derived a board funding keypair");
        *slot = Some(kp);
        Ok(kp)
    }

    /// Steps 2-7 of the send: hydrate and pre-register the inputs,
    /// cosign with the server, register the created VTXOs, deliver them
    /// to the destination's mailboxes, and finalize the wallet state.
    /// Returns the destination's VTXOs; the change is stored by the
    /// finalization itself.
    ///
    /// The failure split is the money-movement line: everything before
    /// the cosign RPC is undoable (locks released by the caller);
    /// everything after it is not (inputs spent server-side).
    async fn cosign_register_deliver(
        &self,
        payment: &ArkPayment,
        inputs: &[WalletVtxo],
        input_ids: &[ark::VtxoId],
        change_keypair: &secp256k1::Keypair,
        authorization: Option<&super::send_fence::SendAuthorization>,
    ) -> Result<Vec<TrackedVtxo>, SendFailure> {
        let wallet = &self.wallet;
        let amount = Amount::from_sat(payment.sats);

        // Hydrate the inputs to their full form: the arkoor builder
        // needs the genesis chain, and the server registration call
        // sends the full bytes over the wire. Pre-registering the input
        // chains anchors the post-cosign registration of the outputs
        // (bark does the same before cosigning).
        let inputs_full = wallet
            .get_full_vtxos(inputs.iter())
            .await
            .map_err(|e| SendFailure::BeforeCosign(format!("hydrating the send inputs: {e}")))?;
        if let Some(auth) = authorization {
            auth.validate(&payment.address.to_string(), payment.sats)
                .map_err(SendFailure::BeforeCosign)?;
        }
        wallet
            .register_vtxo_transactions_with_server(&inputs_full)
            .await
            .map_err(|e| {
                SendFailure::BeforeCosign(format!(
                    "registering the send inputs with the server: {e}"
                ))
            })?;
        let mut user_keypairs = Vec::with_capacity(inputs_full.len());
        for vtxo in &inputs_full {
            user_keypairs.push(wallet.get_vtxo_key(vtxo).await.map_err(|e| {
                SendFailure::BeforeCosign(format!("deriving a send input key: {e}"))
            })?);
        }

        // One destination output plus automatic change to the peeked key.
        let builder = ArkoorPackageBuilder::new_single_output_with_checkpoints(
            inputs_full,
            ArkoorDestination {
                total_amount: amount,
                policy: payment.address.policy().clone(),
            },
            VtxoPolicy::new_pubkey(change_keypair.public_key()),
        )
        .map_err(|e| SendFailure::BeforeCosign(format!("building the arkoor package: {e}")))?;
        let builder = builder
            .generate_user_nonces(&user_keypairs)
            .map_err(|e| SendFailure::BeforeCosign(format!("generating the send nonces: {e}")))?;
        let cosign_request =
            server_rpc::protos::ArkoorPackageCosignRequest::from(builder.cosign_request());

        // A fresh connection per send: one connect-and-handshake next to
        // a cosign round trip, and nothing long-lived to leak across
        // sends. The wallet's own RPCs (registration) reuse its config.
        let mut srv = server_rpc::ServerConnection::builder()
            .address(self.server_address.clone())
            .network(self.network)
            .connect()
            .await
            .map_err(|e| SendFailure::BeforeCosign(format!("connecting to the ark server: {e}")))?;

        // The point of no return: once the server accepts this request,
        // the inputs are spent server-side.
        if let Some(auth) = authorization {
            auth.persist_consumption(&payment.address.to_string(), payment.sats)
                .await
                .map_err(SendFailure::BeforeCosign)?;
            auth.enter_cosign(&payment.address.to_string(), payment.sats)
                .map_err(SendFailure::BeforeCosign)?;
        }
        let response = srv
            .client
            .request_arkoor_cosign(cosign_request)
            .await
            .map_err(|e| {
                SendFailure::AfterCosign(format!("the server refused the arkoor cosign: {e}"))
            })?
            .into_inner();
        let cosign_responses = ArkoorPackageCosignResponse::try_from(response)
            .map_err(|e| SendFailure::AfterCosign(format!("parsing the cosign response: {e:?}")))?;
        let signed = builder
            .user_cosign(&user_keypairs, cosign_responses)
            .map_err(|e| SendFailure::AfterCosign(format!("cosigning the arkoor vtxos: {e}")))?
            .build_signed_vtxos();

        // Divide the signed VTXOs between destination and change, by
        // policy - exactly how bark's own send splits them.
        let dest_policy = payment.address.policy().clone();
        let (dest_vtxos, change_vtxos): (Vec<Vtxo<Full>>, Vec<Vtxo<Full>>) =
            signed.into_iter().partition(|v| *v.policy() == dest_policy);

        // Register the signed chains of both sides so the receiver does
        // not have to re-register them and spends do not lazily retry.
        let mut to_register = dest_vtxos.clone();
        to_register.extend(change_vtxos.iter().cloned());
        wallet
            .register_vtxo_transactions_with_server(&to_register)
            .await
            .map_err(|e| {
                SendFailure::AfterCosign(format!("registering the sent VTXOs with the server: {e}"))
            })?;

        // Mailbox delivery, bark's any-success semantics: the signed
        // chains go to every ServerMailbox the destination lists; one
        // accepted post is enough. An empty delivery list is the
        // receiver's explicit choice and is honoured.
        let delivery = payment.address.delivery();
        if !delivery.is_empty() {
            let serialized: Vec<Vec<u8>> =
                dest_vtxos.iter().map(|v| v.serialize().to_vec()).collect();
            let mut any_succeeded = false;
            let mut failures: Vec<String> = Vec::new();
            for method in delivery {
                let ark::address::VtxoDelivery::ServerMailbox { blinded_id } = method else {
                    continue;
                };
                let req = server_rpc::protos::mailbox_server::PostArkoorMessageRequest {
                    blinded_id: blinded_id.as_ref().to_vec(),
                    vtxos: serialized.clone(),
                };
                match srv.mailbox_client.post_arkoor_message(req).await {
                    Ok(_) => any_succeeded = true,
                    Err(e) => failures.push(format!("{e}")),
                }
            }
            if !any_succeeded {
                return Err(SendFailure::AfterCosign(format!(
                    "no mailbox delivery mechanism accepted the arkoor vtxos: {}",
                    failures.join("; ")
                )));
            }
        }

        // Finalize: idempotent steps in bark's own order (mark inputs
        // spent, then store the change as spendable float).
        wallet
            .mark_vtxos_as_spent(input_ids.iter())
            .await
            .map_err(|e| SendFailure::AfterCosign(format!("marking the send inputs spent: {e}")))?;
        if !change_vtxos.is_empty() {
            wallet
                .store_spendable_vtxos(change_vtxos.iter())
                .await
                .map_err(|e| {
                    SendFailure::AfterCosign(format!("storing the send's change VTXOs: {e}"))
                })?;
        }

        Ok(dest_vtxos
            .iter()
            .map(|v| TrackedVtxo {
                proof: v.serialize(),
                id: v.id().to_bytes(),
                sats: v.amount().to_sat(),
                expiry_height: v.expiry_height().to_u32(),
                user_pubkey: v.user_pubkey().serialize(),
                // The destination's VTXO, never this wallet's float.
                spendable: false,
            })
            .collect())
    }
}

/// Which side of the money-movement line a send failure fell on.
enum SendFailure {
    /// Failed before the server cosigned: no money moved, the input
    /// locks can be released.
    BeforeCosign(String),
    /// Failed after the server cosigned: the inputs are spent
    /// server-side and the locks must stay.
    AfterCosign(String),
}

#[async_trait::async_trait]
impl BarkClient for BarkWalletClient {
    async fn board_capacity_available(&self) -> Result<bool, String> {
        if !self
            .db
            .get_pending_round_state_ids()
            .await
            .map_err(|e| e.to_string())?
            .is_empty()
        {
            return Ok(false);
        }
        self.wallet
            .pending_boards()
            .await
            .map(|b| b.is_empty())
            .map_err(|e| e.to_string())
    }
    async fn check_send_readiness(
        &self,
        payment: &ArkPayment,
        permit: &super::send_fence::WalletOperationPermit,
    ) -> Result<(), CapacityError> {
        permit
            .require_wallet(&self.operation_lock)
            .map_err(|_| CapacityError::ForeignPermit)?;
        self.bounded_plan_send(payment).await?;
        Ok(())
    }
    async fn acquire_send_capacity(
        &self,
    ) -> Result<super::send_fence::WalletOperationPermit, super::send_fence::CapacityError> {
        BarkWalletClient::acquire_send_capacity(self).await
    }
    async fn list_vtxos(&self) -> Result<Vec<TrackedVtxo>, String> {
        let wallet_vtxos = self
            .wallet
            .all_vtxos()
            .await
            .map_err(|e| format!("listing wallet vtxos: {e}"))?;
        Ok(wallet_vtxos
            .into_iter()
            .map(|w| TrackedVtxo {
                proof: vec![], // float enumeration is not a delivered VTXO proof
                id: w.vtxo.id().to_bytes(),
                sats: w.vtxo.amount().to_sat(),
                expiry_height: w.vtxo.expiry_height().to_u32(),
                user_pubkey: w.vtxo.user_pubkey().serialize(),
                spendable: matches!(w.state.kind(), bark::vtxo::VtxoStateKind::Spendable),
            })
            .collect())
    }

    async fn send_arkoor_payment(&self, payment: &ArkPayment) -> Result<Vec<TrackedVtxo>, String> {
        let permit = self
            .acquire_send_capacity()
            .await
            .map_err(|error| error.to_string())?;
        self.send_arkoor_guarded(payment, &permit, None).await
    }
    async fn send_arkoor_authorized(
        &self,
        payment: &ArkPayment,
        authorization: &super::send_fence::SendAuthorization,
    ) -> Result<Vec<TrackedVtxo>, String> {
        let permit = self
            .acquire_send_capacity()
            .await
            .map_err(|error| error.to_string())?;
        self.send_arkoor_guarded(payment, &permit, Some(authorization))
            .await
    }
    async fn send_arkoor_guarded(
        &self,
        payment: &ArkPayment,
        permit: &super::send_fence::WalletOperationPermit,
        authorization: Option<&super::send_fence::SendAuthorization>,
    ) -> Result<Vec<TrackedVtxo>, String> {
        permit.require_wallet(&self.operation_lock)?;
        if let Some(auth) = authorization {
            auth.validate(&payment.address.to_string(), payment.sats)?;
        }
        let wallet = &self.wallet;
        // Same actual planning rules as pre-request readiness. A later failure
        // here is AFTER a possible grant and stays on the unknown/pinned path.
        let selected = self
            .bounded_plan_send(payment)
            .await
            .map_err(|error| error.to_string())?;

        // The change key: peeked and stored so a retry that reuses it
        // keeps the implied spending txid stable (the server's cosign
        // idempotency accepts a retry, not a conflicting double-spend).
        let (change_keypair, change_key_index) = wallet
            .peek_next_keypair()
            .await
            .map_err(|e| format!("deriving the arkoor change keypair: {e}"))?;
        let change_pubkey = change_keypair.public_key();
        if payment.address.policy().user_pubkey() == change_pubkey {
            return Err(
                "the order's destination key equals the wallet's next change key".to_string(),
            );
        }
        self.db
            .store_vtxo_key(change_key_index, change_pubkey)
            .await
            .map_err(|e| format!("storing the arkoor change keypair: {e}"))?;

        // A unique holder for the input locks, mirroring bark's action
        // ids: `release_action_locks` targets exactly this send.
        let action_id = authorization.map_or_else(
            || format!("spc-{}", hex::encode(rand::random::<[u8; 16]>())),
            |auth| auth.action_id(),
        );
        wallet
            .lock_vtxos(
                selected.iter(),
                Some(VtxoLockHolder::Action {
                    id: action_id.clone(),
                }),
            )
            .await
            .map_err(|e| format!("locking the float VTXOs for the send: {e}"))?;
        let input_ids: Vec<ark::VtxoId> = selected.iter().map(|v| v.id()).collect();

        match self
            .cosign_register_deliver(
                payment,
                &selected,
                &input_ids,
                &change_keypair,
                authorization,
            )
            .await
        {
            Ok(delivered) => Ok(delivered),
            Err(SendFailure::BeforeCosign(e)) => {
                // No money moved: give the float back its locks. A failed
                // release is loud but not fatal - the next maintenance
                // pass reconciles states against the server.
                if let Err(rel) = wallet.release_action_locks(&action_id).await {
                    tracing::error!(
                        action = %action_id,
                        error = %rel,
                        "releasing the send's VTXO locks failed; maintenance will reconcile"
                    );
                }
                Err(e)
            }
            Err(SendFailure::AfterCosign(e)) => {
                // The server accepted the cosign: the inputs are spent
                // server-side and the destination VTXOs exist. The locks
                // stay, so a later sync cannot resurrect spent inputs as
                // spendable float. Fail closed: the order's outcome is
                // the transport error, never a fabricated receipt.
                tracing::error!(
                    action = %action_id,
                    error = %e,
                    "arkoor send failed after the server cosigned; VTXO locks are kept"
                );
                Err(e)
            }
        }
    }

    async fn board_funding_address(&self) -> Result<(bitcoin::Address, u32), String> {
        let kp = self.board_keypair().await?;
        let (address, expiry) = self
            .wallet
            .board_funding_address(&kp)
            .await
            .map_err(|e| format!("board funding address: {e}"))?;
        Ok((address, expiry.to_u32()))
    }

    async fn board_psbt(&self, psbt: &bitcoin::Psbt, expiry_height: u32) -> Result<(), String> {
        // The same cached keypair the funding address came from: bark
        // locates the board output by the key's funding script.
        let kp = self.board_keypair().await?;
        self.wallet
            .board_psbt(psbt.clone(), kp, expiry_height.into())
            .await
            .map_err(|e| format!("registering the board PSBT: {e}"))?;
        // Boarded: the next board derives a fresh keypair.
        self.board_keypair.lock().await.take();
        Ok(())
    }

    async fn onchain_address(&self) -> Result<bitcoin::Address, String> {
        use bark::onchain::OnchainWalletTrait as _;
        let mut onchain = self.onchain.write().await;
        onchain
            .address()
            .await
            .map_err(|e| format!("onchain address: {e}"))
    }

    async fn maintenance(&self) -> Result<(), String> {
        // Float hygiene (decision record, clause 4): a cheap sync (fees,
        // offchain state, force-exit detection), the on-chain wallet,
        // then bark's own refresh-before-expiry pass, so every send
        // candidate stays inside `min_remaining_blocks`.
        self.wallet.sync().await;
        self.wallet
            .sync_onchain()
            .await
            .map_err(|e| format!("onchain sync: {e}"))?;
        self.wallet
            .maintenance()
            .await
            .map_err(|e| format!("wallet maintenance: {e}"))
    }

    async fn feerate_sat_vb(&self) -> Result<u64, String> {
        // The cached estimate, never below the configured fallback (the
        // cache starts at the broadcast minimum before the first sync).
        let regular = self.wallet.chain().fee_rates().await.regular;
        Ok(regular
            .to_sat_per_vb_ceil()
            .max(u64::from(self.fallback_fee_sat_vb)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn readiness_covers_actual_fee_and_never_waives_an_unsupported_quote() {
        let full = &ark::test_util::VTXO_VECTORS.board_vtxo;
        full.validate(&ark::test_util::VTXO_VECTORS.anchor_tx)
            .unwrap();
        let row = WalletVtxo {
            vtxo: full.to_bare(),
            state: bark::vtxo::VtxoState::Spendable,
            exit_depth: full.exit_depth(),
            exit_tx_weight: bitcoin::Weight::from_wu(0),
            registered: true,
        };
        let value = full.amount().to_sat();
        assert!(value > 1);
        let quote = bark::FeeEstimate::new(full.amount(), Amount::ZERO, full.amount(), vec![]);
        assert_eq!(
            select_ready_inputs(vec![row.clone()], 0, u16::MAX, value, &quote)
                .unwrap()
                .len(),
            1
        );
        let short = bark::FeeEstimate::new(
            Amount::from_sat(value + 1),
            Amount::from_sat(1),
            full.amount(),
            vec![],
        );
        assert_eq!(
            select_ready_inputs(vec![row.clone()], 0, u16::MAX, value, &short).unwrap_err(),
            CapacityError::InsufficientFloat
        );
        let unsupported = bark::FeeEstimate::new(
            full.amount(),
            Amount::from_sat(1),
            Amount::from_sat(value - 1),
            vec![],
        );
        assert_eq!(
            select_ready_inputs(vec![row], 0, u16::MAX, value - 1, &unsupported).unwrap_err(),
            CapacityError::UnsupportedFeePolicy
        );
    }

    fn tempdir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join("spc-ark-bark-stock-tests")
            .join(format!("{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[cfg(unix)]
    fn write_seed(dir: &Path, name: &str, contents: &str, mode: u32) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(&path, contents).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        path
    }

    #[cfg(unix)]
    fn seed_hex() -> String {
        "11".repeat(64)
    }

    #[cfg(unix)]
    #[test]
    fn a_valid_seed_loads() {
        let dir = tempdir("valid");
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = write_seed(&dir, "seed", &seed_hex(), 0o600);
        let seed = load_seed(&path).unwrap();
        assert_eq!(seed, [0x11; 64]);
    }

    #[cfg(unix)]
    #[test]
    fn loose_file_or_directory_permissions_fail_closed() {
        let dir = tempdir("loose");
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        // Group-readable seed file.
        let loose = write_seed(&dir, "loose", &seed_hex(), 0o640);
        let err = load_seed(&loose).unwrap_err();
        assert!(err.contains("group/other"), "{err}");
        // World-readable store directory.
        let subdir = dir.join("store");
        std::fs::create_dir(&subdir).unwrap();
        std::fs::set_permissions(&subdir, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = write_seed(&subdir, "seed", &seed_hex(), 0o600);
        let err = load_seed(&path).unwrap_err();
        assert!(err.contains("ark seed directory"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn wrong_length_or_non_hex_seeds_fail_closed() {
        let dir = tempdir("length");
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let short = write_seed(&dir, "short", &"11".repeat(63), 0o600);
        assert!(load_seed(&short).unwrap_err().contains("64 bytes"));
        let garbage = write_seed(&dir, "garbage", "not hex at all!", 0o600);
        assert!(load_seed(&garbage).is_err());
    }

    #[test]
    fn a_seed_resolving_under_the_spc_store_is_refused() {
        // Pure rule test: no /run/secrets/spc needed.
        let dir = tempdir("spc-store");
        let root = dir.join("spc");
        let store = root.join("ark");
        std::fs::create_dir_all(&store).unwrap();
        let seed = store.join("seed");
        std::fs::write(&seed, "seed").unwrap();
        assert!(seed_under_store(&seed, &root));
        assert!(!seed_under_store(&dir.join("elsewhere"), &root));
        // A symlink into the store counts as "resolvable under".
        let link = dir.join("link");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&seed, &link).unwrap();
        #[cfg(not(unix))]
        std::fs::copy(&seed, &link).unwrap();
        assert!(seed_under_store(&link, &root));
        // A nonexistent root cannot contain anything.
        assert!(!seed_under_store(&seed, &dir.join("missing")));
    }
}
