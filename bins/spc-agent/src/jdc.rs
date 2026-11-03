//! Job Declarator Client (SPEC §3.2, §3.5, §3.6).
//!
//! The JDC connects to the JDS, negotiates extension `0x0003` (payout
//! distributions), receives the current `SetPayoutDistribution`, builds the
//! coinbase it will declare, verifies the miner's own payout and declares the
//! job. There is no base-protocol fallback: a JDS that declines `0x0003` or
//! does not answer within [`JD_NEGOTIATION_TIMEOUT`] makes the role fail
//! closed instead.
//!
//! ## Fallback (SPEC §3.5; decision 2026-11-03)
//!
//! After declaring, the JDC waits up to `declare_timeout` for the matching
//! `DeclareMiningJob.Success`/`.Error`. A coded decline, silence past the
//! window, or a refuse-mode own-payout failure latch the fallback: the JDC
//! stops declaring, alarms once per episode, and retries once per *changed*
//! declaration key `(distribution_id, template)` - never on a timer, so a
//! dead JDS yields exactly one alarm and no re-sent declarations. Reading
//! happens in a dedicated pump task ([`read_pump`]) so waiting with a
//! deadline never cancels a half-read frame (the noise stream is not
//! cancellation-safe).

use std::time::{Duration, Instant};

use bitcoin::secp256k1::{PublicKey, SecretKey};
use spc_ext::{DistributionIdTlv, SetPayoutDistribution};
use spc_pool::net::{Egress, Target};
use stratum_apps::network_helpers::connect_with_noise;
use stratum_apps::network_helpers::noise_stream::{NoiseTcpReadHalf, NoiseTcpWriteHalf};
use stratum_core::binary_sv2::{self, Seq064K, Str0255, B0255, B064K, U256};
use stratum_core::codec_sv2::SerializedFrame;
use stratum_core::common_messages_sv2::{
    Protocol, SetupConnection, SetupConnectionSuccess, MESSAGE_TYPE_SETUP_CONNECTION,
    MESSAGE_TYPE_SETUP_CONNECTION_SUCCESS,
};
use stratum_core::extensions_sv2::{
    RequestExtensions, RequestExtensionsError, RequestExtensionsSuccess,
    EXTENSION_TYPE_EXTENSIONS_NEGOTIATION, MESSAGE_TYPE_REQUEST_EXTENSIONS,
    MESSAGE_TYPE_REQUEST_EXTENSIONS_ERROR, MESSAGE_TYPE_REQUEST_EXTENSIONS_SUCCESS,
};
use stratum_core::framing_sv2::framing::MessageFrame;
use stratum_core::job_declaration_sv2::{
    AllocateMiningJobToken, AllocateMiningJobTokenSuccess, DeclareMiningJob, DeclareMiningJobError,
    DeclareMiningJobSuccess, MESSAGE_TYPE_ALLOCATE_MINING_JOB_TOKEN,
    MESSAGE_TYPE_ALLOCATE_MINING_JOB_TOKEN_SUCCESS, MESSAGE_TYPE_DECLARE_MINING_JOB,
    MESSAGE_TYPE_DECLARE_MINING_JOB_ERROR, MESSAGE_TYPE_DECLARE_MINING_JOB_SUCCESS,
};
use thiserror::Error;
use tokio::sync::{mpsc, watch};
use tokio::time::timeout;

use crate::config::{AgentConfig, OwnPayoutPolicy};
use crate::verify::{self, Policy};

const EXT_PAYOUTS: u16 = 0x0003;
const EXT_SHARE_ACCOUNTING: u16 = 0x0020;
const EXT_NEGOTIATION: u16 = 0x0001;

/// How long the JDS may stay silent during connect/negotiation before the JDC
/// refuses JD mode (SPEC §3.2: a client whose `RequestExtensions.Error` lists
/// `0x0003` MUST NOT run JD mode; silence is treated the same way rather than
/// falling back to the base protocol).
pub const JD_NEGOTIATION_TIMEOUT: Duration = Duration::from_secs(10);

/// Default bounded wait for the declaration answer (SPEC §3.5; the roadmap
/// names the same 10 s). Configured per deployment as `[jd]
/// declare_timeout_secs`; [`MIN_DECLARE_TIMEOUT`] is the fail-closed floor.
pub const DEFAULT_DECLARE_TIMEOUT: Duration = Duration::from_secs(10);

/// The fail-closed minimum for the declaration window: a shorter configured
/// value is a startup error, never a clamp (SPEC §3.5).
pub const MIN_DECLARE_TIMEOUT: Duration = Duration::from_secs(1);

/// Miner-side key material for own-payout verification (SPEC §3.6): the
/// BIP 352 scan secret key and spend public key.
#[derive(Clone, Debug)]
pub struct OwnKeys {
    pub b_scan: SecretKey,
    pub b_spend: PublicKey,
}

impl OwnKeys {
    pub fn new(b_scan: SecretKey, b_spend: PublicKey) -> Self {
        Self { b_scan, b_spend }
    }
}

#[derive(Debug, Error)]
pub enum JdcError {
    #[error("network: {0}")]
    Network(String),
    #[error("codec: {0}")]
    Codec(String),
    #[error("no distribution")]
    Empty,
    #[error("own payout derivation failed")]
    OwnDerivation,
    #[error("own payout absent from the coinbase outputs")]
    OwnAbsent,
    #[error("coinbase build: {0}")]
    Coinbase(String),
    #[error("payout distribution has no SPc OP_RETURN carrier")]
    NoCarrier,
    #[error("JDS declined extension 0x0003; refusing JD mode (no base-protocol fallback)")]
    JdUnavailable,
    #[error("JDS was silent for {0:?}; refusing JD mode (no base-protocol fallback)")]
    Timeout(Duration),
    #[error("extension negotiation: {0}")]
    Negotiation(String),
    #[error("JDC configuration: {0}")]
    Config(String),
}

/// The template fields the JDC needs to build and declare a job. A live
/// deployment fills this from its own Template Provider; until that hook is
/// wired the role takes it from `[jd]` (see [`JdPlan::from_config`]).
///
/// `txs` is the template's transaction set as the wtxid list that goes into
/// `DeclareMiningJob.wtxid_list`: on a new prev-hash the JDC rebuilds the
/// *whole* template - coinbase and transaction list - never one without the
/// other (SPEC §3.5).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JdTemplate {
    pub height: u32,
    pub coinbase_value: u64,
    pub extranonce_len: usize,
    /// Wtxids of the template's non-coinbase transactions, in template order.
    pub txs: Vec<[u8; 32]>,
}

/// Everything `run` needs, validated once at startup.
#[derive(Clone, Debug)]
pub struct JdPlan {
    pub jds: Target,
    pub egress: Egress,
    pub own_keys: OwnKeys,
    pub template: JdTemplate,
    pub policy: Policy,
    pub user_identifier: String,
    /// Bounded wait for the declaration answer (SPEC §3.5).
    pub declare_timeout: Duration,
}

impl JdPlan {
    pub fn from_config(config: &AgentConfig) -> Result<Self, JdcError> {
        let jd = config
            .jd
            .as_ref()
            .filter(|jd| jd.enabled)
            .ok_or_else(|| JdcError::Config("[jd] is not enabled".into()))?;
        let jds = jd
            .jds_target()
            .map_err(|e| JdcError::Config(e.to_string()))?;
        let egress = config
            .net
            .egress()
            .map_err(|e| JdcError::Config(e.to_string()))?;
        let b_scan = jd
            .scan_secret()
            .map_err(|e| JdcError::Config(e.to_string()))?;
        let parsed = spc_crypto::parse_sp_address(&config.upstream.sp_address, config.network())
            .map_err(|e| JdcError::Config(e.to_string()))?;
        let (height, coinbase_value, extranonce_len) =
            jd.template().map_err(|e| JdcError::Config(e.to_string()))?;
        let declare_timeout = Duration::from_secs(jd.declare_timeout_secs);
        if declare_timeout < MIN_DECLARE_TIMEOUT {
            return Err(JdcError::Config(format!(
                "[jd] declare_timeout_secs must be at least {}s (fail-closed minimum, SPEC §3.5)",
                MIN_DECLARE_TIMEOUT.as_secs()
            )));
        }
        let policy = match config
            .own_payout_policy()
            .map_err(|e| JdcError::Config(e.to_string()))?
        {
            OwnPayoutPolicy::Warn => Policy::Warn,
            OwnPayoutPolicy::Refuse => Policy::Refuse,
        };
        Ok(Self {
            jds,
            egress,
            own_keys: OwnKeys::new(b_scan, parsed.b_spend),
            template: JdTemplate {
                height,
                coinbase_value,
                extranonce_len,
                txs: Vec::new(),
            },
            policy,
            user_identifier: jd.user_identifier.clone(),
            declare_timeout,
        })
    }
}

/// Run the JDC role. The template channel is driven by the caller; `run`
/// supplies the static `[jd]` template for the current build.
pub async fn run(config: AgentConfig) -> anyhow::Result<()> {
    let plan = JdPlan::from_config(&config)?;
    run_static_template(plan).await
}

/// Production path for the current build: a static `[jd]` template.
///
/// The sender is moved into [`run_with_templates`] and held there for the
/// whole run, so `templates.changed()` stays pending instead of resolving to
/// `Err` when there is no live Template Provider yet.
async fn run_static_template(plan: JdPlan) -> anyhow::Result<()> {
    let (tx, rx) = watch::channel(plan.template.clone());
    run_with_templates(plan, rx, tx).await
}

/// Drive the JDC from a template stream (SPEC §3.5 "JDC fallback").
///
/// One loop iteration: if the outstanding declared job's key moved, abandon
/// it with a reason; if the current declaration key was not attempted yet,
/// declare against the newest retained distribution and await the bounded
/// answer. Then wait for the next distribution or template update.
///
/// Decline outcomes latch the fallback (alarm once per episode); recovery is
/// a single fresh attempt triggered by a changed key - a republished
/// distribution (owed on every `SetNewPrevHash`, rebalance and ledger apply)
/// or a template move. There is no timer-based retry, so a silent JDS is
/// declared to once and only once per key.
///
/// `keepalive` MUST stay alive for the duration of this function. It is the
/// `watch::Sender` for `templates`; dropping it makes `templates.changed()`
/// resolve to `Err`, which this loop treats as a fatal "undeclared job" error.
///
/// Integration hook: a live Template Provider must send each new
/// [`JdTemplate`] (height, `coinbase_tx_value_remaining`, extranonce length,
/// transaction list) through the watch channel. There is intentionally no
/// reuse of a stale template across heights: the template keys every
/// declaration.
pub async fn run_with_templates(
    plan: JdPlan,
    mut templates: watch::Receiver<JdTemplate>,
    _keepalive: watch::Sender<JdTemplate>,
) -> anyhow::Result<()> {
    let mut jdc = Jdc::connect(&plan.jds, &plan.egress).await?;
    let token = jdc.allocate_token(&plan.user_identifier).await?;
    tracing::info!(jds = %plan.jds, "JDC connected; JD mode negotiated");
    let mut latch = FallbackLatch::default();
    let mut attempted: Option<(u64, JdTemplate)> = None;

    loop {
        let template = templates.borrow_and_update().clone();
        if let Some(outstanding) = jdc.outstanding() {
            if let Some(reason) = abandon_reason(
                &outstanding,
                &template,
                jdc.last().map(|d| d.distribution_id),
            ) {
                jdc.abandon_outstanding(reason);
            }
        }
        if let Some(id) = jdc.last().map(|d| d.distribution_id) {
            let key = (id, template.clone());
            if attempted.as_ref() != Some(&key) {
                attempted = Some(key);
                match jdc
                    .declare_with_distribution(
                        &token,
                        &template,
                        &plan.own_keys,
                        plan.policy,
                        plan.declare_timeout,
                    )
                    .await
                {
                    Ok(DeclareOutcome::Accepted(id)) => {
                        latch.clear();
                        tracing::info!(
                            distribution_id = id,
                            height = template.height,
                            "JDC declared its coinbase"
                        );
                    }
                    Ok(DeclareOutcome::Declined(decline)) => latch.enter_decline(&decline),
                    Ok(DeclareOutcome::Silent) => latch.enter_silence(plan.declare_timeout),
                    Err(error) => match fallback_kind_for(&error) {
                        Some(kind) => latch.enter(kind, error.to_string()),
                        None => return Err(error.into()),
                    },
                }
            }
        }

        tokio::select! {
            distribution = jdc.next_distribution() => {
                let distribution = distribution?;
                tracing::debug!(
                    distribution_id = distribution.distribution_id,
                    "JDS republished the payout distribution"
                );
            }
            changed = templates.changed() => {
                if changed.is_err() {
                    anyhow::bail!(
                        "template channel closed; refusing to keep an undeclared job"
                    );
                }
                tracing::debug!("JDC template updated");
            }
        }
    }
}

/// The declaration this JDC considers live at the pool: answered
/// `.Success` and not yet superseded (SPEC §3.5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutstandingJob {
    pub distribution_id: u64,
    pub height: u32,
}

/// Why an outstanding declaration was dropped. JDP has no cancel message;
/// the JDS grace rule makes the abandoned job unacceptable to the pool, so
/// this is client-side bookkeeping plus a log line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AbandonReason {
    /// The template height moved (`SetNewPrevHash` at the miner's own TP).
    NewPrevHash,
    /// A new distribution replaced the one the job was declared under.
    DistributionReplaced,
}

impl std::fmt::Display for AbandonReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AbandonReason::NewPrevHash => write!(f, "new-prev-hash"),
            AbandonReason::DistributionReplaced => write!(f, "distribution-replaced"),
        }
    }
}

/// Decide whether the outstanding declaration is stale after the latest
/// template/distribution state, and why. Height wins over distribution: a
/// height move is the prev-hash signal.
fn abandon_reason(
    outstanding: &OutstandingJob,
    template: &JdTemplate,
    last_distribution_id: Option<u64>,
) -> Option<AbandonReason> {
    if outstanding.height != template.height {
        Some(AbandonReason::NewPrevHash)
    } else if last_distribution_id != Some(outstanding.distribution_id) {
        Some(AbandonReason::DistributionReplaced)
    } else {
        None
    }
}

/// A coded decline answer from the JDS (SPEC §3.5 "Declaration answers").
/// This is a protocol outcome, not an I/O failure - [`JdcError::Network`] is
/// reserved for transport errors, so the two are always distinguishable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decline {
    pub code: String,
}

impl Decline {
    /// Transient codes are expected at height races; anything else (including
    /// an unknown code) is hard.
    pub fn transient(&self) -> bool {
        matches!(
            self.code.as_str(),
            "stale-payout-distribution" | "no-payout-distribution"
        )
    }
}

impl std::fmt::Display for Decline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.code)
    }
}

/// How a sent declaration resolved inside the bounded window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeclareOutcome {
    /// `DeclareMiningJobSuccess`; carries the distribution id declared under.
    Accepted(u64),
    /// `DeclareMiningJobError` with the server's coded reason.
    Declined(Decline),
    /// No matching answer arrived inside `declare_timeout`.
    Silent,
}

/// Why the fallback latched (drives the alarm level).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FallbackKind {
    /// No answer inside the declaration window.
    Silence,
    /// A hard coded decline.
    HardDecline,
    /// A transient coded decline (height race, no distribution yet).
    TransientDecline,
    /// The own-payout check refused the job (or made it unverifiable).
    OwnPayoutRefused,
}

/// The latched fallback state: alarms once per episode (error level) for
/// hard causes, warn level for transient ones; cleared on a successful
/// declaration.
#[derive(Default, Debug)]
struct FallbackLatch {
    active: Option<FallbackKind>,
    episodes: u64,
}

impl FallbackLatch {
    fn enter(&mut self, kind: FallbackKind, detail: String) {
        if self.active == Some(kind) {
            // Same episode, same cause: hold quietly. Changing cause while
            // latched logs again at debug level only.
            tracing::debug!(?kind, %detail, "JD fallback already latched");
            return;
        }
        self.active = Some(kind);
        self.episodes += 1;
        match kind {
            FallbackKind::TransientDecline => {
                tracing::warn!(%detail, episode = self.episodes, "JDS declined (transient); holding for the next distribution or template");
            }
            _ => {
                tracing::error!(%detail, episode = self.episodes, "JD fallback latched: not declaring until the declaration key changes");
            }
        }
    }

    fn enter_decline(&mut self, decline: &Decline) {
        let kind = if decline.transient() {
            FallbackKind::TransientDecline
        } else {
            FallbackKind::HardDecline
        };
        self.enter(kind, decline.to_string());
    }

    fn enter_silence(&mut self, window: Duration) {
        self.enter(
            FallbackKind::Silence,
            format!("no DeclareMiningJob answer within {window:?}"),
        );
    }

    fn clear(&mut self) {
        if self.active.take().is_some() {
            tracing::info!(
                episodes = self.episodes,
                "JD fallback cleared; declaring again"
            );
        }
    }
}

/// Which declaration-path failures latch the fallback instead of tearing the
/// role down (SPEC §3.5/§3.6: refuse to mine an unpayable job; keep the
/// failure scoped to JD). Transport and codec failures stay fatal.
fn fallback_kind_for(error: &JdcError) -> Option<FallbackKind> {
    match error {
        JdcError::OwnAbsent | JdcError::OwnDerivation | JdcError::NoCarrier => {
            Some(FallbackKind::OwnPayoutRefused)
        }
        _ => None,
    }
}

pub struct Jdc {
    rx: mpsc::Receiver<Result<SerializedFrame, String>>,
    write: NoiseTcpWriteHalf,
    last: Option<SetPayoutDistribution>,
    last_declared: Option<u64>,
    outstanding: Option<OutstandingJob>,
    last_abandonment: Option<(u64, AbandonReason)>,
    request_seq: u32,
    _pump: tokio::task::JoinHandle<()>,
}

/// Pump frames off the noise stream into a channel for the connection's
/// whole life. `read_frame` is documented as not cancellation-safe, so no
/// bounded wait in the JDC may wrap it directly; `mpsc::Receiver::recv` is
/// cancellation-safe, which makes `declare_timeout` safe to apply anywhere
/// without corrupting a half-read frame.
async fn read_pump(mut read: NoiseTcpReadHalf, tx: mpsc::Sender<Result<SerializedFrame, String>>) {
    loop {
        match read.read_frame().await {
            Ok(frame) => {
                if tx.send(Ok(frame)).await.is_err() {
                    return;
                }
            }
            Err(e) => {
                let _ = tx.send(Err(e.to_string())).await;
                return;
            }
        }
    }
}

impl Jdc {
    pub async fn connect(target: &Target, egress: &Egress) -> Result<Self, JdcError> {
        let stream = egress
            .connect(target)
            .await
            .map_err(|e| JdcError::Network(e.to_string()))?;
        let stream = connect_with_noise(stream, None)
            .await
            .map_err(|e| JdcError::Network(e.to_string()))?;
        let (read, mut write) = stream.into_split();
        let (tx, rx) = mpsc::channel(64);
        let pump = tokio::spawn(read_pump(read, tx));
        let s = |x: &'static str| Str0255::try_from(x).unwrap();
        let setup = SetupConnection {
            protocol: Protocol::JobDeclarationProtocol,
            min_version: 2,
            max_version: 2,
            flags: 0,
            endpoint_host: s("127.0.0.1"),
            endpoint_port: 0,
            vendor: s("spc-agent"),
            hardware_version: s("1"),
            firmware: s("1"),
            device_id: s("1"),
        };
        let frame = MessageFrame::from_message(setup, MESSAGE_TYPE_SETUP_CONNECTION, 0, false)
            .map_err(|e| JdcError::Codec(e.to_string()))?;
        write
            .write_frame(frame)
            .await
            .map_err(|e| JdcError::Network(e.to_string()))?;
        let mut jdc = Self {
            rx,
            write,
            last: None,
            last_declared: None,
            outstanding: None,
            last_abandonment: None,
            request_seq: 2, // 1 is AllocateMiningJobToken's
            _pump: pump,
        };
        let mut f = timeout(JD_NEGOTIATION_TIMEOUT, jdc.recv_frame())
            .await
            .map_err(|_| JdcError::Timeout(JD_NEGOTIATION_TIMEOUT))??;
        if f.header().msg_type() != MESSAGE_TYPE_SETUP_CONNECTION_SUCCESS {
            return Err(JdcError::Codec("expected SetupConnection.Success".into()));
        }
        let mut p = f.payload().to_vec();
        let _: SetupConnectionSuccess =
            binary_sv2::from_bytes(&mut p).map_err(|e| JdcError::Codec(e.to_string()))?;
        jdc.negotiate().await?;
        if jdc.last.is_none() {
            jdc.receive_distribution_within(JD_NEGOTIATION_TIMEOUT)
                .await?;
        }
        Ok(jdc)
    }

    /// Receive the next frame from the pump. Channel closure or a pump-side
    /// error is a transport failure and tears the role down.
    async fn recv_frame(&mut self) -> Result<SerializedFrame, JdcError> {
        match self.rx.recv().await {
            Some(Ok(frame)) => Ok(frame),
            Some(Err(e)) => Err(JdcError::Network(e)),
            None => Err(JdcError::Network("JDS connection closed".into())),
        }
    }

    /// Send `RequestExtensions{0x0001, 0x0003, 0x0020}` and require `0x0003`
    /// to be accepted. The JDS may publish the distribution before its
    /// negotiation answer; either order is accepted, and a distribution
    /// received here is retained.
    async fn negotiate(&mut self) -> Result<(), JdcError> {
        let requested = vec![EXT_NEGOTIATION, EXT_PAYOUTS, EXT_SHARE_ACCOUNTING];
        let msg = RequestExtensions {
            request_id: 1,
            requested_extensions: Seq064K::new(requested)
                .map_err(|e| JdcError::Codec(e.to_string()))?,
        };
        let frame = MessageFrame::from_message(
            msg,
            MESSAGE_TYPE_REQUEST_EXTENSIONS,
            EXTENSION_TYPE_EXTENSIONS_NEGOTIATION,
            false,
        )
        .map_err(|e| JdcError::Codec(e.to_string()))?;
        self.write
            .write_frame(frame)
            .await
            .map_err(|e| JdcError::Network(e.to_string()))?;

        let deadline = Instant::now() + JD_NEGOTIATION_TIMEOUT;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(JdcError::Timeout(JD_NEGOTIATION_TIMEOUT));
            }
            let mut f = timeout(remaining, self.recv_frame())
                .await
                .map_err(|_| JdcError::Timeout(JD_NEGOTIATION_TIMEOUT))??;
            let ext = f.header().ext_type_without_channel_msg();
            let msg_type = f.header().msg_type();

            if ext == EXT_PAYOUTS && msg_type == 0 {
                self.retain_distribution(&mut f)?;
                continue;
            }
            if ext != EXTENSION_TYPE_EXTENSIONS_NEGOTIATION {
                continue;
            }
            let mut p = f.payload().to_vec();
            match msg_type {
                MESSAGE_TYPE_REQUEST_EXTENSIONS_SUCCESS => {
                    let success: RequestExtensionsSuccess = binary_sv2::from_bytes(&mut p)
                        .map_err(|e| JdcError::Codec(e.to_string()))?;
                    if success.request_id != 1 {
                        return Err(JdcError::Negotiation(format!(
                            "request_id {} does not match 1",
                            success.request_id
                        )));
                    }
                    let supported: Vec<u16> =
                        success.supported_extensions.iter().copied().collect();
                    return require_payout_extension(&supported);
                }
                MESSAGE_TYPE_REQUEST_EXTENSIONS_ERROR => {
                    let error: RequestExtensionsError = binary_sv2::from_bytes(&mut p)
                        .map_err(|e| JdcError::Codec(e.to_string()))?;
                    let unsupported: Vec<u16> =
                        error.unsupported_extensions.iter().copied().collect();
                    let required: Vec<u16> = error.required_extensions.iter().copied().collect();
                    return check_payout_rejection(&unsupported, &required);
                }
                _ => {}
            }
        }
    }

    pub async fn allocate_token(&mut self, user_identifier: &str) -> Result<Vec<u8>, JdcError> {
        let msg = AllocateMiningJobToken {
            user_identifier: Str0255::try_from(user_identifier)
                .map_err(|e| JdcError::Codec(e.to_string()))?,
            request_id: 1,
        };
        let frame =
            MessageFrame::from_message(msg, MESSAGE_TYPE_ALLOCATE_MINING_JOB_TOKEN, 0, false)
                .map_err(|e| JdcError::Codec(e.to_string()))?;
        self.write
            .write_frame(frame)
            .await
            .map_err(|e| JdcError::Network(e.to_string()))?;
        loop {
            let mut f = timeout(JD_NEGOTIATION_TIMEOUT, self.recv_frame())
                .await
                .map_err(|_| JdcError::Timeout(JD_NEGOTIATION_TIMEOUT))??;
            if f.header().msg_type() == MESSAGE_TYPE_ALLOCATE_MINING_JOB_TOKEN_SUCCESS {
                let mut p = f.payload().to_vec();
                let success: AllocateMiningJobTokenSuccess =
                    binary_sv2::from_bytes(&mut p).map_err(|e| JdcError::Codec(e.to_string()))?;
                return Ok(success.mining_job_token.as_bytes().to_vec());
            }
        }
    }

    /// Build the coinbase the JDC will declare. `a_send_x` and the height come
    /// from the template; the carrier is the JD-mode `OP_RETURN` output (the
    /// scriptSig carries only the BIP34 height push and the extranonce, SPEC
    /// §2.4.2). Before building, the miner's own `P` is derived for
    /// `(height, A_send)` and checked against the outputs (SPEC §3.6):
    /// `Policy::Refuse` fails the build when the payout is absent.
    ///
    /// The distribution's `additional_outputs` normally already contains the
    /// carrier. It is stripped and re-emitted exactly once at the end, so a
    /// duplicated carrier in the distribution cannot produce two `OP_RETURN`
    /// outputs.
    pub fn build_coinbase(
        distribution: &SetPayoutDistribution,
        template_revenue: u64,
        height: u32,
        a_send_x: &[u8; 32],
        extranonce_len: usize,
        own_keys: &OwnKeys,
        policy: Policy,
    ) -> Result<(Vec<u8>, Vec<u8>), JdcError> {
        let fixed = distribution
            .fixed_value()
            .map_err(|e| JdcError::Codec(e.to_string()))?;
        let weights: Vec<u64> = distribution.payouts.iter().map(|p| p.value).collect();
        let amounts = spc_ext::compute_payout_amounts(
            template_revenue,
            distribution.pool_payout.value,
            &weights,
            &distribution.dust_limits,
            fixed,
        )
        .map_err(|e| JdcError::Codec(e.to_string()))?;
        let carrier = spc_crypto::emit_op_return(a_send_x);

        let mut outputs: Vec<(bitcoin::script::ScriptBuf, u64)> = Vec::new();
        outputs.push((
            bitcoin::script::ScriptBuf::from_bytes(distribution.pool_payout.script.clone()),
            amounts.pay_p,
        ));
        for (out, pay) in distribution.payouts.iter().zip(&amounts.pays) {
            if let Some(v) = pay {
                outputs.push((
                    bitcoin::script::ScriptBuf::from_bytes(out.script.clone()),
                    *v,
                ));
            }
        }
        for out in &distribution.additional_outputs {
            // Every SPc-shaped carrier is dropped, including one carrying a
            // different `A_send`; the expected carrier is added once, below.
            if is_spc_carrier(&out.script) {
                continue;
            }
            outputs.push((
                bitcoin::script::ScriptBuf::from_bytes(out.script.clone()),
                out.value,
            ));
        }
        outputs.push((carrier, 0));

        Self::verify_own_payout(height, a_send_x, &outputs, own_keys, policy)?;

        let template =
            spc_pool::coinbase::build_jd_template(2, height, extranonce_len, &outputs, 0)
                .map_err(|e| JdcError::Coinbase(e.to_string()))?;
        Ok((template.prefix, template.suffix))
    }

    /// Derive the miner's own `P` for `(height, A_send)` and check that it
    /// appears among `outputs` (SPEC §3.6). `Policy::Refuse` fails when the
    /// payout is absent; `Policy::Warn` allows a missing payout (a small
    /// miner is legitimately below the threshold or trimmed).
    pub fn verify_own_payout(
        height: u32,
        a_send_x: &[u8; 32],
        outputs: &[(bitcoin::script::ScriptBuf, u64)],
        own_keys: &OwnKeys,
        policy: Policy,
    ) -> Result<(), JdcError> {
        let own = verify::derive_own(height, a_send_x, &own_keys.b_scan, &own_keys.b_spend)
            .map_err(|_| JdcError::OwnDerivation)?;
        let views: Vec<verify::OutputView<'_>> = outputs
            .iter()
            .map(|(script, value)| verify::OutputView {
                script_pubkey: script.as_bytes(),
                value: *value,
            })
            .collect();
        let found = verify::find_own(&views, &own).is_some();
        verify::check_policy(policy, found).map_err(|_| JdcError::OwnAbsent)
    }

    /// Recover `A_send` from the distribution's SPc `OP_RETURN` carrier
    /// (SPEC §3.6 allows either the job prefix or the distribution).
    pub fn send_key_from_distribution(
        distribution: &SetPayoutDistribution,
    ) -> Result<[u8; 32], JdcError> {
        let views: Vec<spc_crypto::carriers::TxOut<'_>> = distribution
            .additional_outputs
            .iter()
            .map(|out| spc_crypto::carriers::TxOut {
                value: out.value,
                script_pubkey: &out.script,
            })
            .collect();
        spc_crypto::parse_op_return(&views).ok_or(JdcError::NoCarrier)
    }

    /// Rebuild the coinbase for the most recent distribution and declare it,
    /// then wait up to `window` for the matching `.Success`/`.Error`
    /// (SPEC §3.5). Distributions published during the wait are retained.
    pub async fn declare_with_distribution(
        &mut self,
        token: &[u8],
        template: &JdTemplate,
        own_keys: &OwnKeys,
        policy: Policy,
        window: Duration,
    ) -> Result<DeclareOutcome, JdcError> {
        let distribution = self.last.clone().ok_or(JdcError::Empty)?;
        let a_send_x = Self::send_key_from_distribution(&distribution)?;
        let (prefix, suffix) = Self::build_coinbase(
            &distribution,
            template.coinbase_value,
            template.height,
            &a_send_x,
            template.extranonce_len,
            own_keys,
            policy,
        )?;
        let request_id = self.next_request_id();
        self.declare_job(
            token,
            distribution.distribution_id,
            &prefix,
            &suffix,
            &template.txs,
            request_id,
        )
        .await?;
        let outcome = self
            .await_declaration(request_id, distribution.distribution_id, window)
            .await?;
        if let DeclareOutcome::Accepted(id) = outcome {
            self.last_declared = Some(id);
            self.outstanding = Some(OutstandingJob {
                distribution_id: id,
                height: template.height,
            });
        }
        Ok(outcome)
    }

    /// Declare a job with the current distribution id. The caller supplies the
    /// token from `allocate_token`, the prefix/suffix from `build_coinbase`
    /// and the template's transaction list (`wtxids`).
    pub async fn declare_job(
        &mut self,
        token: &[u8],
        id: u64,
        coinbase_prefix: &[u8],
        coinbase_suffix: &[u8],
        wtxids: &[[u8; 32]],
        request_id: u32,
    ) -> Result<(), JdcError> {
        let tlv = DistributionIdTlv { id }.encode();
        let wtxid_list = Seq064K::try_from(wtxids.iter().map(U256::from).collect::<Vec<U256>>())
            .map_err(|e| JdcError::Codec(e.to_string()))?;
        let msg = DeclareMiningJob {
            request_id,
            mining_job_token: B0255::new(token).map_err(|e| JdcError::Codec(e.to_string()))?,
            version: 0x2000_0000,
            coinbase_tx_prefix: B064K::new(coinbase_prefix)
                .map_err(|e| JdcError::Codec(e.to_string()))?,
            coinbase_tx_suffix: B064K::new(coinbase_suffix)
                .map_err(|e| JdcError::Codec(e.to_string()))?,
            wtxid_list,
            excess_data: B064K::new(&tlv).map_err(|e| JdcError::Codec(e.to_string()))?,
        };
        let frame = MessageFrame::from_message(msg, MESSAGE_TYPE_DECLARE_MINING_JOB, 0, false)
            .map_err(|e| JdcError::Codec(e.to_string()))?;
        self.write
            .write_frame(frame)
            .await
            .map_err(|e| JdcError::Network(e.to_string()))
    }

    /// Wait for the answer to the declaration sent with `request_id`, at most
    /// `window`. Frames that are not the matching answer are handled inline
    /// (distributions retained) or skipped; a late answer for a different
    /// request is ignored, not mistaken for ours.
    async fn await_declaration(
        &mut self,
        request_id: u32,
        id: u64,
        window: Duration,
    ) -> Result<DeclareOutcome, JdcError> {
        let deadline = Instant::now() + window;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(DeclareOutcome::Silent);
            }
            let mut f = match timeout(remaining, self.recv_frame()).await {
                Ok(f) => f?,
                Err(_) => return Ok(DeclareOutcome::Silent),
            };
            if self.retain_distribution(&mut f)? {
                continue;
            }
            let msg_type = f.header().msg_type();
            if !matches!(
                msg_type,
                MESSAGE_TYPE_DECLARE_MINING_JOB_SUCCESS | MESSAGE_TYPE_DECLARE_MINING_JOB_ERROR
            ) {
                continue;
            }
            let mut p = f.payload().to_vec();
            match msg_type {
                MESSAGE_TYPE_DECLARE_MINING_JOB_SUCCESS => {
                    let m: DeclareMiningJobSuccess = binary_sv2::from_bytes(&mut p)
                        .map_err(|e| JdcError::Codec(e.to_string()))?;
                    if m.request_id == request_id {
                        return Ok(DeclareOutcome::Accepted(id));
                    }
                    tracing::debug!(
                        got = m.request_id,
                        want = request_id,
                        "ignoring a declaration answer for another request"
                    );
                }
                _ => {
                    let m: DeclareMiningJobError = binary_sv2::from_bytes(&mut p)
                        .map_err(|e| JdcError::Codec(e.to_string()))?;
                    if m.request_id == request_id {
                        return Ok(DeclareOutcome::Declined(Decline {
                            code: m.error_code.as_utf8_or_hex().to_string(),
                        }));
                    }
                    tracing::debug!(
                        got = m.request_id,
                        want = request_id,
                        "ignoring a declaration answer for another request"
                    );
                }
            }
        }
    }

    pub async fn next_distribution(&mut self) -> Result<SetPayoutDistribution, JdcError> {
        loop {
            let mut f = self.recv_frame().await?;
            if self.retain_distribution(&mut f)? {
                return self.last.clone().ok_or(JdcError::Empty);
            }
        }
    }

    /// Bounded first-distribution read. The pump makes the timeout
    /// cancellation-safe: expiry no longer drops a half-read frame.
    pub async fn receive_distribution_within(
        &mut self,
        window: Duration,
    ) -> Result<SetPayoutDistribution, JdcError> {
        let deadline = Instant::now() + window;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(JdcError::Timeout(window));
            }
            let mut f = timeout(remaining, self.recv_frame())
                .await
                .map_err(|_| JdcError::Timeout(window))??;
            if self.retain_distribution(&mut f)? {
                return self.last.clone().ok_or(JdcError::Empty);
            }
        }
    }

    fn retain_distribution(&mut self, frame: &mut SerializedFrame) -> Result<bool, JdcError> {
        if frame.header().ext_type_without_channel_msg() == EXT_PAYOUTS
            && frame.header().msg_type() == 0
        {
            let d = SetPayoutDistribution::decode(frame.payload())
                .map_err(|e| JdcError::Codec(e.to_string()))?;
            self.last = Some(d);
            return Ok(true);
        }
        Ok(false)
    }

    fn next_request_id(&mut self) -> u32 {
        let id = self.request_seq;
        self.request_seq = self.request_seq.wrapping_add(1);
        id
    }

    pub fn last(&self) -> Option<&SetPayoutDistribution> {
        self.last.as_ref()
    }

    pub fn last_declared(&self) -> Option<u64> {
        self.last_declared
    }

    /// The declaration the JDC considers live at the pool, if any.
    pub fn outstanding(&self) -> Option<OutstandingJob> {
        self.outstanding
    }

    /// The most recent abandonment, for observability and tests.
    pub fn last_abandonment(&self) -> Option<(u64, AbandonReason)> {
        self.last_abandonment
    }

    /// Abandon the outstanding declaration with a reason (SPEC §3.5: a
    /// template move invalidates it; the JDS grace rule makes it
    /// unacceptable server-side).
    pub fn abandon_outstanding(&mut self, reason: AbandonReason) -> Option<u64> {
        let job = self.outstanding.take()?;
        tracing::info!(
            distribution_id = job.distribution_id,
            height = job.height,
            %reason,
            "JDC abandoning declared job"
        );
        self.last_abandonment = Some((job.distribution_id, reason));
        Some(job.distribution_id)
    }

    /// Reserved for the declaration layer; the write half is kept so the JDC
    /// can send the TLV without reconnecting.
    pub fn write_half(&mut self) -> &mut NoiseTcpWriteHalf {
        &mut self.write
    }
}

fn is_spc_carrier(script: &[u8]) -> bool {
    script.len() == 37 && script[0] == 0x6a && script[1] == 0x23 && &script[2..5] == b"SPc"
}

fn require_payout_extension(supported: &[u16]) -> Result<(), JdcError> {
    if supported.contains(&EXT_PAYOUTS) {
        Ok(())
    } else {
        Err(JdcError::JdUnavailable)
    }
}

fn check_payout_rejection(unsupported: &[u16], required: &[u16]) -> Result<(), JdcError> {
    if unsupported.contains(&EXT_PAYOUTS) || required.contains(&EXT_PAYOUTS) {
        Err(JdcError::JdUnavailable)
    } else {
        // The error is about other extensions (for example `0x0020`); JD mode
        // itself was not declined.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::secp256k1::Secp256k1;
    use spc_ext::TxOut;

    /// Non-negotiation declaration windows used by the tests: long enough
    /// that a live JDS answers; short enough that a scripted silence fails
    /// fast.
    const SHORT: Duration = Duration::from_millis(300);

    fn keys() -> OwnKeys {
        let secp = Secp256k1::new();
        let b_scan = SecretKey::from_slice(&[0x11; 32]).unwrap();
        let b_spend = SecretKey::from_slice(&[0x12; 32]).unwrap();
        OwnKeys::new(b_scan, PublicKey::from_secret_key(&secp, &b_spend))
    }

    fn template_at(height: u32) -> JdTemplate {
        JdTemplate {
            height,
            coinbase_value: 1_000,
            extranonce_len: 8,
            txs: Vec::new(),
        }
    }

    fn dist_with_payout(script: Vec<u8>) -> SetPayoutDistribution {
        SetPayoutDistribution {
            distribution_id: 0,
            pool_payout: TxOut {
                script: vec![0x51, 0x20, 1, 1],
                value: 100,
            },
            payouts: vec![TxOut { script, value: 300 }],
            dust_limits: vec![0],
            additional_outputs: vec![],
        }
    }

    fn dist_with_payout_script() -> SetPayoutDistribution {
        dist_with_payout(vec![0x51, 0x20, 2, 2])
    }

    /// Decode the JD coinbase suffix into `(script, value)` outputs.
    fn suffix_outputs(suffix: &[u8]) -> Vec<(Vec<u8>, u64)> {
        let mut off = 4usize; // input sequence
        let (count, next) = spc_pool::coinbase::read_compact_size(suffix, off).unwrap();
        off = next;
        let mut out = Vec::new();
        for _ in 0..count {
            let value = u64::from_le_bytes(suffix[off..off + 8].try_into().unwrap());
            off += 8;
            let (len, next) = spc_pool::coinbase::read_compact_size(suffix, off).unwrap();
            off = next;
            let script = suffix[off..off + len as usize].to_vec();
            off += len as usize;
            out.push((script, value));
        }
        out
    }

    fn is_carrier(script: &[u8], a_send_x: &[u8; 32]) -> bool {
        script == spc_crypto::emit_op_return(a_send_x).as_bytes()
    }

    #[test]
    fn missing_own_payout_under_refuse_fails() {
        let own_keys = keys();
        let send = spc_crypto::SendKey::generate(800_000).unwrap();
        // The only miner output is not our P.
        let dist = dist_with_payout_script();
        let err = Jdc::build_coinbase(
            &dist,
            1_000,
            800_000,
            send.x_only(),
            8,
            &own_keys,
            Policy::Refuse,
        )
        .unwrap_err();
        assert!(matches!(err, JdcError::OwnAbsent));
    }

    #[test]
    fn missing_own_payout_under_warn_succeeds() {
        let own_keys = keys();
        let send = spc_crypto::SendKey::generate(800_000).unwrap();
        let dist = dist_with_payout_script();
        let (prefix, suffix) = Jdc::build_coinbase(
            &dist,
            1_000,
            800_000,
            send.x_only(),
            8,
            &own_keys,
            Policy::Warn,
        )
        .unwrap();
        // JD coinbase: the scriptSig is the height push + extranonce only.
        assert_eq!(prefix[41], 12); // 4-byte height push + 8-byte extranonce
        assert_eq!(&prefix[42..], &[0x03, 0x00, 0x35, 0x0c]);
        // A_send rides in the OP_RETURN output, never in the scriptSig.
        let a_x = send.x_only();
        assert!(!prefix.windows(32).any(|w| w == a_x));
        assert!(suffix
            .windows(37)
            .any(|w| w == spc_crypto::emit_op_return(a_x).as_bytes()));
    }

    #[test]
    fn present_own_payout_under_refuse_succeeds() {
        let own_keys = keys();
        let send = spc_crypto::SendKey::generate(800_000).unwrap();
        let own = verify::derive_own(800_000, send.x_only(), &own_keys.b_scan, &own_keys.b_spend)
            .unwrap();
        let mut script = vec![0x51, 0x20];
        script.extend_from_slice(&own.p_x);
        let dist = dist_with_payout(script);
        assert!(Jdc::build_coinbase(
            &dist,
            1_000,
            800_000,
            send.x_only(),
            8,
            &own_keys,
            Policy::Refuse,
        )
        .is_ok());
    }

    #[test]
    fn distribution_carrier_yields_exactly_one_op_return() {
        let own_keys = keys();
        let send = spc_crypto::SendKey::generate(800_000).unwrap();
        let carrier = spc_crypto::emit_op_return(send.x_only()).to_bytes();
        let mut dist = dist_with_payout_script();
        // The normal distribution already carries the carrier; this one has it
        // twice, plus one pointing at a different `A_send`. The JDC must still
        // emit exactly one, with its own key.
        dist.additional_outputs = vec![
            TxOut {
                script: carrier.clone(),
                value: 0,
            },
            TxOut {
                script: carrier.clone(),
                value: 0,
            },
            TxOut {
                script: spc_crypto::emit_op_return(&[0x55; 32]).to_bytes(),
                value: 0,
            },
        ];
        let (_prefix, suffix) = Jdc::build_coinbase(
            &dist,
            1_000,
            800_000,
            send.x_only(),
            8,
            &own_keys,
            Policy::Warn,
        )
        .unwrap();
        let outputs = suffix_outputs(&suffix);
        let carriers: Vec<&(Vec<u8>, u64)> = outputs
            .iter()
            .filter(|(script, _)| is_carrier(script, send.x_only()))
            .collect();
        assert_eq!(carriers.len(), 1, "exactly one SPc carrier");
        assert_eq!(carriers[0].1, 0, "the carrier output is 0-valued");
        assert_eq!(
            outputs
                .iter()
                .filter(|(script, _)| is_spc_carrier(script))
                .count(),
            1,
            "no carrier-shaped output survives besides ours"
        );
    }

    #[test]
    fn carrier_is_emitted_when_the_distribution_omits_it() {
        let own_keys = keys();
        let send = spc_crypto::SendKey::generate(800_000).unwrap();
        let dist = dist_with_payout_script();
        assert!(dist.additional_outputs.is_empty());
        let (_prefix, suffix) = Jdc::build_coinbase(
            &dist,
            1_000,
            800_000,
            send.x_only(),
            8,
            &own_keys,
            Policy::Warn,
        )
        .unwrap();
        let outputs = suffix_outputs(&suffix);
        assert_eq!(
            outputs
                .iter()
                .filter(|(script, _)| is_carrier(script, send.x_only()))
                .count(),
            1
        );
    }

    #[test]
    fn send_key_is_read_from_the_distribution_carrier() {
        let send = spc_crypto::SendKey::generate(800_000).unwrap();
        let mut dist = dist_with_payout_script();
        dist.additional_outputs = vec![TxOut {
            script: spc_crypto::emit_op_return(send.x_only()).to_bytes(),
            value: 0,
        }];
        assert_eq!(
            Jdc::send_key_from_distribution(&dist).unwrap(),
            *send.x_only()
        );
        dist.additional_outputs.clear();
        assert!(matches!(
            Jdc::send_key_from_distribution(&dist),
            Err(JdcError::NoCarrier)
        ));
    }

    #[test]
    fn declining_0x0003_refuses_jd_mode() {
        assert!(matches!(
            require_payout_extension(&[EXT_NEGOTIATION, EXT_SHARE_ACCOUNTING]),
            Err(JdcError::JdUnavailable)
        ));
        assert!(matches!(
            check_payout_rejection(&[EXT_PAYOUTS], &[]),
            Err(JdcError::JdUnavailable)
        ));
        assert!(matches!(
            check_payout_rejection(&[], &[EXT_PAYOUTS]),
            Err(JdcError::JdUnavailable)
        ));
        // An error about an optional extension is not a JD refusal.
        assert!(check_payout_rejection(&[EXT_SHARE_ACCOUNTING], &[]).is_ok());
        assert!(require_payout_extension(&[EXT_NEGOTIATION, EXT_PAYOUTS]).is_ok());
    }

    /// SPEC §3.5: only the race-expected codes are transient; anything else,
    /// including a code this build does not know, is hard (fail closed).
    #[test]
    fn decline_codes_classify_by_action() {
        assert!(Decline {
            code: "stale-payout-distribution".into()
        }
        .transient());
        assert!(Decline {
            code: "no-payout-distribution".into()
        }
        .transient());
        assert!(!Decline {
            code: "invalid-payout-distribution".into()
        }
        .transient());
        assert!(!Decline {
            code: "invalid-job".into()
        }
        .transient());
        assert!(!Decline {
            code: "some-future-code".into()
        }
        .transient());
    }

    /// SPEC §3.5: a height move abandons with `new-prev-hash`; a same-height
    /// distribution move with `distribution-replaced`.
    #[test]
    fn abandon_reason_prefers_the_height_move() {
        let job = OutstandingJob {
            distribution_id: 5,
            height: 1,
        };
        let t1 = template_at(1);
        let mut t2 = template_at(2);
        assert_eq!(abandon_reason(&job, &t1, Some(5)), None);
        assert_eq!(
            abandon_reason(&job, &t1, Some(6)),
            Some(AbandonReason::DistributionReplaced)
        );
        assert_eq!(
            abandon_reason(&job, &t2, Some(5)),
            Some(AbandonReason::NewPrevHash)
        );
        // Height wins when both moved.
        assert_eq!(
            abandon_reason(&job, &t2, Some(6)),
            Some(AbandonReason::NewPrevHash)
        );
        // A same-height template whose tx set changed is not an abandonment.
        t2.height = 1;
        t2.txs.push([0xaa; 32]);
        assert_eq!(abandon_reason(&job, &t2, Some(5)), None);
    }

    #[test]
    fn own_payout_failures_latch_the_fallback_transport_errors_do_not() {
        for e in [
            JdcError::OwnAbsent,
            JdcError::OwnDerivation,
            JdcError::NoCarrier,
        ] {
            assert_eq!(
                fallback_kind_for(&e),
                Some(FallbackKind::OwnPayoutRefused),
                "{e}"
            );
        }
        assert_eq!(fallback_kind_for(&JdcError::Network("x".into())), None);
        assert_eq!(fallback_kind_for(&JdcError::Codec("x".into())), None);
        assert_eq!(fallback_kind_for(&JdcError::Empty), None);
    }

    fn own_distribution(
        height: u32,
        a_send_x: &[u8; 32],
        own_keys: &OwnKeys,
        pool_weight: u64,
    ) -> SetPayoutDistribution {
        let own =
            verify::derive_own(height, a_send_x, &own_keys.b_scan, &own_keys.b_spend).unwrap();
        let mut script = vec![0x51, 0x20];
        script.extend_from_slice(&own.p_x);
        SetPayoutDistribution {
            distribution_id: 0,
            pool_payout: TxOut {
                script: vec![0x51, 0x20, 1, 1],
                value: pool_weight,
            },
            payouts: vec![TxOut { script, value: 300 }],
            dust_limits: vec![0],
            additional_outputs: vec![TxOut {
                script: spc_crypto::emit_op_return(a_send_x).to_bytes(),
                value: 0,
            }],
        }
    }

    async fn wait_accepted(server: &spc_pool::jds_server::JdsServer, id: u64) {
        for _ in 0..100 {
            if server.accepted().await == Some(id) {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("JDS did not accept distribution {id}");
    }

    async fn wait_declare_count(server: &spc_pool::jds_server::JdsServer, n: usize) {
        for _ in 0..100 {
            if server.declare_count().await >= n {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("JDS saw fewer than {n} declarations");
    }

    fn plan_for(addr: std::net::SocketAddr, own_keys: &OwnKeys, template: JdTemplate) -> JdPlan {
        JdPlan {
            jds: addr.into(),
            egress: Egress::DIRECT,
            own_keys: own_keys.clone(),
            template,
            policy: Policy::Refuse,
            user_identifier: "jdc-test".into(),
            declare_timeout: SHORT,
        }
    }

    #[tokio::test]
    async fn jdc_declares_current_and_rebuilds_on_a_new_distribution() {
        use spc_pool::jds_server::JdsServer;

        let server = JdsServer::new(1);
        let (addr, _handle) = server.bind("127.0.0.1:0").await.unwrap();
        let own_keys = keys();
        let send = spc_crypto::SendKey::generate(1).unwrap();
        let a_x = *send.x_only();
        let dist = own_distribution(1, &a_x, &own_keys, 100);
        let id = server.publish(1, 1_000, dist).await.unwrap();

        let mut jdc = Jdc::connect(&addr.into(), &Egress::DIRECT).await.unwrap();
        let token = jdc.allocate_token("jdc-test").await.unwrap();
        let template = template_at(1);
        let outcome = jdc
            .declare_with_distribution(&token, &template, &own_keys, Policy::Refuse, SHORT)
            .await
            .unwrap();
        assert_eq!(outcome, DeclareOutcome::Accepted(id));
        assert_eq!(jdc.last_declared(), Some(id));
        assert_eq!(
            jdc.outstanding(),
            Some(OutstandingJob {
                distribution_id: id,
                height: 1
            })
        );
        wait_accepted(&server, id).await;

        // The JDS republishes at a new revenue; the JDC must rebuild the
        // coinbase and redeclare under the new id, not reuse the old one.
        let id2 = server
            .publish(1, 2_000, own_distribution(1, &a_x, &own_keys, 100))
            .await
            .unwrap();
        assert_ne!(id, id2);
        let received = jdc.next_distribution().await.unwrap();
        assert_eq!(received.distribution_id, id2);
        let template2 = JdTemplate {
            coinbase_value: 2_000,
            ..template.clone()
        };
        let outcome2 = jdc
            .declare_with_distribution(&token, &template2, &own_keys, Policy::Refuse, SHORT)
            .await
            .unwrap();
        assert_eq!(outcome2, DeclareOutcome::Accepted(id2));
        wait_accepted(&server, id2).await;
    }

    /// A coded decline is a protocol outcome (SPEC §3.5), not a transport
    /// failure: the same client then sees a real network error behave
    /// differently when the server goes away.
    #[tokio::test]
    async fn decline_codes_are_not_transport_errors() {
        use spc_pool::jds_server::{AnswerPolicy, JdsServer};

        let server = JdsServer::new(4);
        let (addr, _handle) = server.bind("127.0.0.1:0").await.unwrap();
        let own_keys = keys();
        let send = spc_crypto::SendKey::generate(1).unwrap();
        let a_x = *send.x_only();
        server
            .publish(1, 1_000, own_distribution(1, &a_x, &own_keys, 100))
            .await
            .unwrap();
        server
            .set_answer_policy(AnswerPolicy::ForceDecline(
                "invalid-payout-distribution".into(),
            ))
            .await;

        let mut jdc = Jdc::connect(&addr.into(), &Egress::DIRECT).await.unwrap();
        let token = jdc.allocate_token("jdc-test").await.unwrap();
        let outcome = jdc
            .declare_with_distribution(&token, &template_at(1), &own_keys, Policy::Refuse, SHORT)
            .await
            .unwrap();
        assert_eq!(
            outcome,
            DeclareOutcome::Declined(Decline {
                code: "invalid-payout-distribution".into()
            }),
            "a coded decline is a protocol outcome, not an Err"
        );
        assert_eq!(jdc.last_declared(), None, "declined jobs are not live");
        assert_eq!(jdc.outstanding(), None);
        assert_eq!(server.declare_count().await, 1);

        // A transport failure is a different beast: the connection drops
        // (crashed-JDS stand-in) and the same call path returns Err(Network).
        server.set_answer_policy(AnswerPolicy::HangUp).await;
        let err = jdc
            .declare_with_distribution(&token, &template_at(1), &own_keys, Policy::Refuse, SHORT)
            .await
            .unwrap_err();
        assert!(
            matches!(err, JdcError::Network(_)),
            "expected a transport error, got {err}"
        );
    }

    /// Decline -> fallback, once: the JDC declares once per declaration key,
    /// does not loop against the decline, and recovers on the next key (a
    /// republished distribution).
    #[tokio::test]
    async fn decline_latches_the_fallback_and_recovery_redeclares_once() {
        use spc_pool::jds_server::{AnswerPolicy, JdsServer};

        let server = JdsServer::new(5);
        let (addr, _handle) = server.bind("127.0.0.1:0").await.unwrap();
        let own_keys = keys();
        let send = spc_crypto::SendKey::generate(1).unwrap();
        let a_x = *send.x_only();
        server
            .publish(1, 1_000, own_distribution(1, &a_x, &own_keys, 100))
            .await
            .unwrap();
        server
            .set_answer_policy(AnswerPolicy::ForceDecline(
                "invalid-payout-distribution".into(),
            ))
            .await;

        let plan = plan_for(addr, &own_keys, template_at(1));
        let template = plan.template.clone();
        let (tx, rx) = watch::channel(template);
        let handle = tokio::spawn(run_with_templates(plan, rx, tx));
        wait_declare_count(&server, 1).await;
        // Latched: no second declaration while nothing moved.
        tokio::time::sleep(std::time::Duration::from_millis(600)).await;
        assert_eq!(server.declare_count().await, 1);
        assert!(!handle.is_finished(), "a declined JDC must not exit");

        // Recovery: clear the decline and republish; exactly one fresh
        // declaration under the new key is accepted.
        server.set_answer_policy(AnswerPolicy::Natural).await;
        let id2 = server
            .publish(1, 1_000, own_distribution(1, &a_x, &own_keys, 100))
            .await
            .unwrap();
        wait_accepted(&server, id2).await;
        assert_eq!(server.declare_count().await, 2);
        handle.abort();
    }

    /// 10 s style silence (here 300 ms) -> fallback once, not repeatedly: the
    /// muted server receives exactly one declaration per key, and the role
    /// stays alive waiting for inputs.
    #[tokio::test]
    async fn silence_latches_exactly_once_until_an_input_moves() {
        use spc_pool::jds_server::{AnswerPolicy, JdsServer};

        let server = JdsServer::new(6);
        let (addr, _handle) = server.bind("127.0.0.1:0").await.unwrap();
        let own_keys = keys();
        let send = spc_crypto::SendKey::generate(1).unwrap();
        let a_x = *send.x_only();
        server
            .publish(1, 1_000, own_distribution(1, &a_x, &own_keys, 100))
            .await
            .unwrap();
        server.set_answer_policy(AnswerPolicy::Mute).await;

        let plan = plan_for(addr, &own_keys, template_at(1));
        let template = plan.template.clone();
        let (tx, rx) = watch::channel(template);
        let handle = tokio::spawn(run_with_templates(plan, rx, tx));
        wait_declare_count(&server, 1).await;
        // Well beyond the SHORT window: no retry, no busy loop, no exit.
        tokio::time::sleep(std::time::Duration::from_millis(1_200)).await;
        assert_eq!(server.declare_count().await, 1);
        assert!(!handle.is_finished(), "a silenced JDC must not exit");

        // Un-mute and republish: one fresh declaration, accepted.
        server.set_answer_policy(AnswerPolicy::Natural).await;
        let id2 = server
            .publish(1, 1_000, own_distribution(1, &a_x, &own_keys, 100))
            .await
            .unwrap();
        wait_accepted(&server, id2).await;
        assert_eq!(server.declare_count().await, 2);
        handle.abort();
    }

    /// A refuse-mode own-payout failure falls back *before any declaration is
    /// sent*: the miner never hashes a job that cannot pay it. Recovery is the
    /// next key as usual.
    #[tokio::test]
    async fn own_payout_refusal_declares_nothing_until_fixed() {
        use spc_pool::jds_server::JdsServer;

        let server = JdsServer::new(7);
        let (addr, _handle) = server.bind("127.0.0.1:0").await.unwrap();
        let own_keys = keys();
        let send = spc_crypto::SendKey::generate(1).unwrap();
        let a_x = *send.x_only();
        // This distribution pays a different miner only (dist_with_payout has
        // someone else's script), and carries the carrier with a valid key.
        let mut without_us = dist_with_payout(vec![0x51, 0x20, 2, 2]);
        without_us.additional_outputs = vec![TxOut {
            script: spc_crypto::emit_op_return(&a_x).to_bytes(),
            value: 0,
        }];
        server.publish(1, 1_000, without_us).await.unwrap();

        let plan = plan_for(addr, &own_keys, template_at(1));
        let template = plan.template.clone();
        let (tx, rx) = watch::channel(template);
        let handle = tokio::spawn(run_with_templates(plan, rx, tx));
        tokio::time::sleep(std::time::Duration::from_millis(600)).await;
        assert_eq!(
            server.declare_count().await,
            0,
            "an unpayable distribution must never be declared"
        );
        assert!(!handle.is_finished(), "a refused JDC must not exit");

        // Recovery: the next distribution pays us again.
        let id2 = server
            .publish(1, 1_000, own_distribution(1, &a_x, &own_keys, 100))
            .await
            .unwrap();
        wait_accepted(&server, id2).await;
        assert_eq!(server.declare_count().await, 1);
        handle.abort();
    }

    /// A new prev-hash while a declared job is outstanding: the old job is
    /// abandoned (`new-prev-hash`), and the redeclaration carries the *whole*
    /// rebuilt template - fresh coinbase and the fresh transaction list.
    #[tokio::test]
    async fn new_prev_hash_abandons_and_rebuilds_the_full_template() {
        use spc_pool::jds_server::JdsServer;

        let server = JdsServer::new(8);
        let (addr, _handle) = server.bind("127.0.0.1:0").await.unwrap();
        let own_keys = keys();
        let send1 = spc_crypto::SendKey::generate(1).unwrap();
        let a_x1 = *send1.x_only();
        let id1 = server
            .publish(1, 1_000, own_distribution(1, &a_x1, &own_keys, 100))
            .await
            .unwrap();

        let mut jdc = Jdc::connect(&addr.into(), &Egress::DIRECT).await.unwrap();
        let token = jdc.allocate_token("jdc-test").await.unwrap();
        let outcome = jdc
            .declare_with_distribution(&token, &template_at(1), &own_keys, Policy::Refuse, SHORT)
            .await
            .unwrap();
        assert_eq!(outcome, DeclareOutcome::Accepted(id1));

        // The prev-hash moves: a new distribution for height 2 arrives.
        let send2 = spc_crypto::SendKey::generate(2).unwrap();
        let a_x2 = *send2.x_only();
        let id2 = server
            .publish(2, 1_000, own_distribution(2, &a_x2, &own_keys, 100))
            .await
            .unwrap();
        let received = jdc.next_distribution().await.unwrap();
        assert_eq!(received.distribution_id, id2);

        // The abandoning decision the run loop applies.
        let mut moved = template_at(2);
        moved.txs = vec![[0xaa; 32], [0xbb; 32]];
        let outstanding = jdc.outstanding().unwrap();
        assert_eq!(
            abandon_reason(&outstanding, &moved, Some(id2)),
            Some(AbandonReason::NewPrevHash)
        );
        assert_eq!(
            jdc.abandon_outstanding(AbandonReason::NewPrevHash),
            Some(id1)
        );
        assert_eq!(
            jdc.last_abandonment(),
            Some((id1, AbandonReason::NewPrevHash))
        );
        assert_eq!(jdc.outstanding(), None);

        // Redeclare with the whole new template (coinbase + tx set).
        let outcome2 = jdc
            .declare_with_distribution(&token, &moved, &own_keys, Policy::Refuse, SHORT)
            .await
            .unwrap();
        assert_eq!(outcome2, DeclareOutcome::Accepted(id2));
        let declares = server.declares().await;
        assert_eq!(declares.len(), 2);
        assert_eq!(declares[1].wtxids, 2, "the fresh tx set rides the wire");
        wait_accepted(&server, id2).await;
    }

    #[tokio::test]
    async fn run_static_template_path_survives_an_empty_template_tick() {
        use spc_pool::jds_server::JdsServer;
        use tokio::time::Duration;

        let server = JdsServer::new(2);
        let (addr, _handle) = server.bind("127.0.0.1:0").await.unwrap();
        let own_keys = keys();
        let send = spc_crypto::SendKey::generate(1).unwrap();
        let a_x = *send.x_only();
        let id = server
            .publish(1, 1_000, own_distribution(1, &a_x, &own_keys, 100))
            .await
            .unwrap();
        let plan = plan_for(addr, &own_keys, template_at(1));

        let handle = tokio::spawn(run_static_template(plan));
        wait_accepted(&server, id).await;
        // With no live Template Provider the watch sender is the only thing
        // keeping `changed()` pending. If it were dropped, the loop would bail
        // immediately and this task would finish.
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            !handle.is_finished(),
            "JDC exited because the static template sender was dropped"
        );
        handle.abort();
    }

    #[tokio::test]
    async fn run_with_templates_handles_a_tick_and_a_new_distribution() {
        use spc_pool::jds_server::JdsServer;
        use tokio::time::Duration;

        let server = JdsServer::new(3);
        let (addr, _handle) = server.bind("127.0.0.1:0").await.unwrap();
        let own_keys = keys();
        let send = spc_crypto::SendKey::generate(1).unwrap();
        let a_x = *send.x_only();
        let template = template_at(1);
        let id = server
            .publish(1, 1_000, own_distribution(1, &a_x, &own_keys, 100))
            .await
            .unwrap();
        let plan = plan_for(addr, &own_keys, template.clone());
        let (tx, rx) = watch::channel(template);
        let handle = tokio::spawn(run_with_templates(plan, rx, tx.clone()));
        wait_accepted(&server, id).await;

        // A template tick is consumed without tearing the loop down.
        tx.send(template_at(1)).unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!handle.is_finished(), "JDC exited on a template tick");
        // An unchanged declaration key is not re-declared.
        assert_eq!(server.declare_count().await, 1);

        // A republished distribution triggers a rebuild/redeclare.
        let id2 = server
            .publish(1, 1_000, own_distribution(1, &a_x, &own_keys, 100))
            .await
            .unwrap();
        wait_accepted(&server, id2).await;
        assert_eq!(server.declare_count().await, 2);
        handle.abort();
    }

    #[test]
    fn template_without_height_or_value_is_rejected() {
        let config = config_without_jd_template();
        assert!(matches!(
            JdPlan::from_config(&config),
            Err(JdcError::Config(_))
        ));
    }

    fn config_without_jd_template() -> AgentConfig {
        use crate::config::*;
        let secp = Secp256k1::new();
        let b_scan = SecretKey::from_slice(&[0x11; 32]).unwrap();
        let b_spend = SecretKey::from_slice(&[0x12; 32]).unwrap();
        let sp_address = spc_crypto::address::encode_sp_address(
            spc_crypto::Network::Testnet,
            &PublicKey::from_secret_key(&secp, &b_scan),
            &PublicKey::from_secret_key(&secp, &b_spend),
        )
        .unwrap();
        AgentConfig {
            upstream: Upstream {
                address: "127.0.0.1:34254".into(),
                authority_public_key:
                    "02aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
                sp_address,
                require_own_payout: "warn".into(),
                request_share_accounting: true,
            },
            net: spc_pool::net::Net::default(),
            downstream: Downstream::default(),
            jd: Some(Jd {
                enabled: true,
                tp_address: String::new(),
                jds_address: "127.0.0.1:34264".into(),
                scan_secret_key: Some(
                    "1111111111111111111111111111111111111111111111111111111111111111".into(),
                ),
                height: None,
                coinbase_value: None,
                extranonce_len: 8,
                user_identifier: "spc-agent".into(),
                declare_timeout_secs: 10,
            }),
            ark: None,
        }
    }
}
