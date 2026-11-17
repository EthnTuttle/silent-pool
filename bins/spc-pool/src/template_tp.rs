//! SV2 Template Distribution Protocol (TDP) template source (SPEC §7.1).
//!
//! A long-lived client connection to the node's Template Provider: Noise_NX
//! with the configured authority public key, `SetupConnection` for the
//! TemplateDistribution protocol, then `CoinbaseOutputConstraints`. From then
//! on the TP *pushes* `NewTemplate` / `SetNewPrevHash`; this module caches the
//! newest complete pair, fetches its transaction set with
//! `RequestTransactionData` on first use and maps the result into the pool's
//! [`Template`]. `getblocktemplate` polling disappears entirely.
//!
//! Field mapping (sv2-spec 07 / rbitcoin `sv2-template-provider.md`):
//!
//! - height: parsed from the BIP34 push at the start of `coinbase_prefix`.
//!   A missing or non-canonical push is rejected: the height drives the pool
//!   coinbase, the per-height payout key and the distribution, so it must
//!   never be guessed.
//! - `prev_hash`, `n_bits`, `curtime`: `SetNewPrevHash`. The push's
//!   `header_timestamp` is the earliest time the TP allows, so it is also
//!   the template's `min_time`; TDP carries no `maxtime`.
//! - `version`: `NewTemplate.version`.
//! - coinbase consensus fields: `coinbase_tx_input_sequence` and
//!   `coinbase_tx_locktime` from `NewTemplate`, read through
//!   [`crate::template::CoinbaseFields::from_template_provider`]. A final
//!   sequence is the legacy shape; a non-final sequence is BIP54
//!   consensuscleanup and MUST carry the exact height-1 locktime - anything
//!   else rejects the template (fail closed, decision
//!   2026-11-15-template-bip54).
//! - `coinbase_value`: `coinbase_tx_value_remaining` (subsidy + fees).
//! - subsidy: the configured network's halving schedule at that height
//!   (regtest halves every 150 blocks, every other network 210_000).
//!   `fees = coinbase_value - subsidy`, failing closed on underflow.
//! - `transactions`: `RequestTransactionData.Success`, in template order.
//! - `merkle_branches`: `NewTemplate.merkle_path`, whose TDP order is
//!   deepest-first; the pool's `Template` keeps the Stratum order (topmost
//!   first), so the path is reversed.
//!
//! Witness transactions are allowed, exactly like the RPC source: the pool
//! coinbase carries the BIP141 witness commitment (`crate::witness`) and the
//! front assembles witness blocks. Every transaction must still strictly
//! decode (`crate::template::analyze_transactions`); an undecodable or
//! ambiguous transaction poisons the whole template, because per-transaction
//! exclusion is impossible over TDP - `SubmitSolution` reuses the TP's
//! *retained* transaction set for the template id, so locally dropping a
//! transaction would not drop it from the mined block.

use std::collections::{BTreeMap, VecDeque};
use std::net::SocketAddr;
use std::time::Duration;

use async_trait::async_trait;
use stratum_apps::key_utils::Secp256k1PublicKey;
use stratum_apps::network_helpers::connect_with_noise;
use stratum_apps::network_helpers::noise_stream::NoiseTcpWriteHalf;
use stratum_core::binary_sv2::{self, Str0255};
use stratum_core::common_messages_sv2::{
    Protocol, SetupConnection, SetupConnectionError, SetupConnectionSuccess,
    MESSAGE_TYPE_SETUP_CONNECTION, MESSAGE_TYPE_SETUP_CONNECTION_ERROR,
    MESSAGE_TYPE_SETUP_CONNECTION_SUCCESS,
};
use stratum_core::framing_sv2::framing::MessageFrame;
use stratum_core::template_distribution_sv2::{
    NewTemplate, RequestTransactionData, RequestTransactionDataError,
    RequestTransactionDataSuccess, SetNewPrevHash, SubmitSolution, MESSAGE_TYPE_NEW_TEMPLATE,
    MESSAGE_TYPE_REQUEST_TRANSACTION_DATA, MESSAGE_TYPE_REQUEST_TRANSACTION_DATA_ERROR,
    MESSAGE_TYPE_REQUEST_TRANSACTION_DATA_SUCCESS, MESSAGE_TYPE_SET_NEW_PREV_HASH,
    MESSAGE_TYPE_SUBMIT_SOLUTION,
};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};
use tokio::time::{timeout, Instant};

use crate::template::{BlockSubmitter, CoinbaseFields, Template, TemplateError, TemplateSource};

/// Bounds one template/transaction fetch: without it a wedged TP would wedge
/// the front's refresh path forever instead of erroring into a retry.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Reconnect backoff bounds. The TP is a local service (an rbitcoin node on
/// loopback, typically), so the first retry is quick; long outages back off.
const RECONNECT_MIN: Duration = Duration::from_millis(500);
const RECONNECT_MAX: Duration = Duration::from_secs(30);
/// Templates are retained per id because a fee push or tip push can land
/// while a `RequestTransactionData` for the previous id is still in flight.
const MAX_CACHED_TEMPLATES: usize = 8;
/// How many served templates a `SubmitSolution` can still be matched to.
/// The TP itself retains more (64 per session on rbitcoin, plus its stale
/// grace on replaced tips).
const MAX_SERVED_TEMPLATES: usize = 64;

/// Connection parameters for [`Sv2TpTemplateSource`].
#[derive(Clone, Debug)]
pub struct Sv2TpConfig {
    /// TCP address of the node's `--sv2-tp-listen` endpoint (IP:port).
    pub address: SocketAddr,
    /// The authority public key the node logs at startup.
    pub authority: Secp256k1PublicKey,
    /// Declared `coinbase_output_max_additional_size`, mirrored into every
    /// served [`Template`].
    pub coinbase_output_max_additional_size: u32,
    /// Declared `coinbase_output_max_additional_sigops`.
    pub coinbase_output_max_additional_sigops: u16,
    /// Blocks between halvings: 150 on regtest, 210_000 elsewhere.
    pub halving_interval: u32,
    /// The configured chain. Every served template is checked against it
    /// (`crate::chain_identity::validate_template_chain`): a Testnet5 TP
    /// whose templates omit the active BIP54 facts is refused, never mined.
    pub chain: spc_crypto::ChainNetwork,
}

/// `CoinbaseOutputConstraints.coinbase_output_max_additional_size` from the
/// configured coinbase budget. The TP sizes its template transaction set to
/// `MAX_BLOCK_WEIGHT - max(1168 + 4*size, 2000)` weight units, so the declared
/// size MUST cover the largest coinbase the pool can build: under-declaring
/// can assemble an overweight block that the node then rejects. The §4.8
/// ceiling is the hard cap on the pool's coinbase weight (coinbase <= 25% of
/// the block), so the ceiling in bytes is the honest bound. Reserving for the
/// moving autoscaler value instead would let a late upscaling invalidate a
/// found block.
pub fn coinbase_budget_additional_size(ceiling_wu: u32) -> u32 {
    ceiling_wu.saturating_add(3) / 4
}

/// `CoinbaseOutputConstraints.coinbase_output_max_additional_sigops`. Every
/// output the pool builds is P2TR (key path) or an OP_RETURN carrier: neither
/// contributes sigops under legacy counting, so no reserve is needed. The
/// client's value replaces the TP's default 400-sigop reserve.
pub fn coinbase_additional_sigops() -> u16 {
    0
}

/// Parse `[template] sv2_tp_authority_pubkey`: the SRI `key-utils`
/// `Secp256k1PublicKey` base58check form (what rbitcoin logs), or 64 hex
/// characters of the x-only key. Fails closed on any other shape.
pub fn parse_tp_authority_key(text: &str) -> Result<Secp256k1PublicKey, String> {
    let trimmed = text.trim();
    if trimmed.len() == 64 && trimmed.bytes().all(|b| b.is_ascii_hexdigit()) {
        let bytes = hex::decode(trimmed).map_err(|e| format!("authority key hex: {e}"))?;
        let key = secp256k1_legacy::XOnlyPublicKey::from_slice(&bytes)
            .map_err(|e| format!("authority key is not a valid x-only public key: {e}"))?;
        return Ok(Secp256k1PublicKey(key));
    }
    trimmed
        .parse::<Secp256k1PublicKey>()
        .map_err(|e| format!("authority key key-utils base58check: {e}"))
}

/// Block subsidy in satoshis at `height`, from the network's halving
/// schedule: 50 BTC at genesis, halved every `halving_interval` blocks
/// (150 on regtest, 210_000 on mainnet/testnet/signet). The TP reports only
/// subsidy + fees together, so the schedule is how the pool recovers the
/// split it accounts against. Zero once 64 halvings have passed.
pub fn subsidy_at(height: u32, halving_interval: u32) -> u64 {
    // The interval is fixed per network at construction; max(1) only keeps
    // this pure function total.
    let halvings = height / halving_interval.max(1);
    if halvings >= 64 {
        return 0;
    }
    5_000_000_000u64 >> halvings
}

/// Decode the BIP34 height push at the start of a TP `coinbase_prefix`.
/// Accepts exactly the canonical CScriptNum form rbitcoin's
/// `bip34_height_script` emits (and spc-crypto's `height_push_bytes` for the
/// pool's own coinbase): OP_0 for 0, OP_1..OP_16 for 1..=16, else a minimal
/// little-endian sign-magnitude data push. Anything else - truncated,
/// non-minimal, negative, OP_PUSHDATA*, wider than a u32 - is `None`, because
/// a height derived any other way would be a guess.
pub fn parse_bip34_height(prefix: &[u8]) -> Option<u32> {
    let opcode = *prefix.first()?;
    match opcode {
        0x00 => Some(0),
        0x51..=0x60 => Some(u32::from(opcode - 0x50)),
        0x01..=0x05 => {
            let n = usize::from(opcode);
            let body = prefix.get(1..1 + n)?;
            let last = body[n - 1];
            // Minimal-encoding rule, per CScriptNum: a trailing 0x00 byte is
            // only the sign protector, required exactly when the magnitude's
            // top byte has its high bit set.
            let value = if last == 0x00 {
                if n == 1 || body[n - 2] & 0x80 == 0 {
                    return None; // zero as a data push, or redundant sign byte
                }
                // Strip the sign protector.
                let mut value = 0u64;
                for (i, b) in body[..n - 1].iter().enumerate() {
                    value |= u64::from(*b) << (8 * i);
                }
                value
            } else {
                if last & 0x80 != 0 {
                    return None; // negative height, or a negative sign byte
                }
                let mut value = 0u64;
                for (i, b) in body.iter().enumerate() {
                    value |= u64::from(*b) << (8 * i);
                }
                value
            };
            u32::try_from(value).ok()
        }
        _ => None,
    }
}

/// One `NewTemplate` whose `SetNewPrevHash` has not arrived yet.
#[derive(Debug)]
struct PendingNew {
    template_id: u64,
    height: u32,
    version: u32,
    coinbase: CoinbaseFields,
    coinbase_value: u64,
    merkle_path: Vec<[u8; 32]>,
}

/// A complete template: `NewTemplate` joined with the prev-hash state.
#[derive(Clone, Debug)]
struct CachedTemplate {
    template_id: u64,
    height: u32,
    prev_hash: [u8; 32],
    bits: u32,
    curtime: u32,
    version: u32,
    coinbase: CoinbaseFields,
    coinbase_value: u64,
    merkle_path: Vec<[u8; 32]>,
    transactions: TxFetch,
}

#[derive(Clone, Debug)]
enum TxFetch {
    /// Not requested yet.
    Pending,
    /// Fetched and validated: every transaction strictly decodes.
    Ready(Vec<Vec<u8>>),
    /// Fetched and refused: remembered so the next `get_template` fails
    /// immediately instead of re-requesting a template we can never mine.
    Rejected(String),
}

enum TpCommand {
    GetTemplate {
        reply: oneshot::Sender<Result<Template, TemplateError>>,
    },
    Submit {
        solution: WireSolution,
        /// Merkle identity of the served transaction set (`block::merkle_root`
        /// over the non-coinbase txids), the link back to the template id the
        /// found block was mined against.
        txset_key: [u8; 32],
        reply: oneshot::Sender<Result<(), TemplateError>>,
    },
}

/// The fields `SubmitSolution` carries on the wire, plus the two local facts
/// the acceptance verdict needs (TDP has no per-solution acknowledgement).
struct WireSolution {
    template_id: u64,
    version: u32,
    header_timestamp: u32,
    header_nonce: u32,
    coinbase_tx: Vec<u8>,
    block_hash: [u8; 32],
    height: u32,
}

/// TDP template source. A thin handle onto a supervisor task that owns the
/// connection, so callers (`Front::refresh`, block submission) never share
/// socket state. Construction spawns the supervisor; a lost connection
/// reconnects with backoff and clears cached templates, because template ids
/// are per-session.
pub struct Sv2TpTemplateSource {
    cmd: mpsc::Sender<TpCommand>,
}

impl Sv2TpTemplateSource {
    pub fn spawn(config: Sv2TpConfig) -> Self {
        let (cmd, rx) = mpsc::channel(16);
        tokio::spawn(async move { supervisor(config, rx).await });
        Self { cmd }
    }
}

#[async_trait]
impl TemplateSource for Sv2TpTemplateSource {
    async fn get_template(&self) -> Result<Template, TemplateError> {
        let (reply, rx) = oneshot::channel();
        self.cmd
            .send(TpCommand::GetTemplate { reply })
            .await
            .map_err(|_| TemplateError::Unavailable("sv2-tp supervisor stopped".into()))?;
        timeout(REQUEST_TIMEOUT, rx)
            .await
            .map_err(|_| TemplateError::Unavailable("sv2-tp get_template timed out".into()))?
            .map_err(|_| TemplateError::Unavailable("sv2-tp reply dropped".into()))?
    }

    async fn submit_block(&self, block: &[u8]) -> Result<(), TemplateError> {
        self.submit_raw_block(block).await
    }
}

#[async_trait]
impl BlockSubmitter for Sv2TpTemplateSource {
    async fn submit_assembled(
        &self,
        block: &crate::block::AssembledBlock,
    ) -> Result<(), TemplateError> {
        self.submit_raw_block(&block.block).await
    }
}

impl Sv2TpTemplateSource {
    /// Send `SubmitSolution` for an assembled block and wait for the TP's
    /// acceptance verdict. TDP has no solution ack: acceptance is the
    /// `NewTemplate` + `SetNewPrevHash` pushed *on top of our block*. A tip
    /// that advances past our height WITHOUT our block is a rejection (the
    /// template went stale under us); a timeout is reported rather than
    /// swallowed, because rbitcoin logs-and-drops undecodable, unknown-id or
    /// consensus-invalid solutions without a wire error.
    async fn submit_raw_block(&self, block: &[u8]) -> Result<(), TemplateError> {
        use bitcoin::hashes::Hash as _;
        let decoded: bitcoin::Block = bitcoin::consensus::deserialize(block)
            .map_err(|e| TemplateError::Rejected(format!("solution decode: {e}")))?;
        // Block serialization starts with the 80-byte header. The header's
        // version is signed in rust-bitcoin but an opaque u32 in TDP, so take
        // the raw LE bytes rather than a conversion.
        let header = block.get(..80).ok_or_else(|| {
            TemplateError::Rejected("submit: block is shorter than its header".into())
        })?;
        let version_bytes: [u8; 4] = header[0..4]
            .try_into()
            .map_err(|e| TemplateError::Rejected(format!("submit: header version: {e}")))?;
        let version = u32::from_le_bytes(version_bytes);
        let Some(coinbase) = decoded.txdata.first() else {
            return Err(TemplateError::Rejected(
                "submit: block has no coinbase".into(),
            ));
        };
        // Full serialization: a witness block's coinbase keeps its BIP141
        // reserved-value witness here, which the TP needs to rebuild the
        // block it validates.
        let coinbase_tx = bitcoin::consensus::serialize(coinbase);
        // The scriptSig starts with the BIP34 height (the pool emits exactly
        // the canonical push parsed by `parse_bip34_height`), needed to
        // distinguish "another block replaced ours".
        let height =
            parse_bip34_height(coinbase.input[0].script_sig.as_bytes()).ok_or_else(|| {
                TemplateError::Rejected("submit: coinbase has no canonical BIP34 height".into())
            })?;
        let solution = WireSolution {
            template_id: 0, // filled by the supervisor from the txset map
            version,
            header_timestamp: decoded.header.time,
            header_nonce: decoded.header.nonce,
            coinbase_tx,
            block_hash: decoded.header.block_hash().to_byte_array(),
            height,
        };
        let txids = decoded.txdata[1..]
            .iter()
            .map(|tx| tx.compute_txid().to_byte_array())
            .collect::<Vec<_>>();
        let txset_key = crate::block::merkle_root(&txids);
        let (reply, rx) = oneshot::channel();
        self.cmd
            .send(TpCommand::Submit {
                solution,
                txset_key,
                reply,
            })
            .await
            .map_err(|_| TemplateError::Unavailable("sv2-tp supervisor stopped".into()))?;
        timeout(REQUEST_TIMEOUT, rx)
            .await
            .map_err(|_| {
                TemplateError::Rejected(
                    "sv2-tp: SubmitSolution unconfirmed within 10s (TDP has no ack; a silent TP-side drop cannot be distinguished until a new tip arrives)"
                        .into(),
                )
            })?
            .map_err(|_| TemplateError::Unavailable("sv2-tp submit reply dropped".into()))?
    }
}

fn unavailable(what: impl Into<String>) -> TemplateError {
    TemplateError::Unavailable(what.into())
}

async fn supervisor(config: Sv2TpConfig, mut rx: mpsc::Receiver<TpCommand>) {
    let mut backoff = RECONNECT_MIN;
    loop {
        match run_session(&config, &mut rx).await {
            // Every handle is gone: this source is dropped, not disconnected.
            Ok(()) => return,
            Err(e) => {
                tracing::warn!(error = %e, addr = %config.address, "sv2-tp session failed; reconnecting")
            }
        }
        drain_commands(&mut rx);
        if rx.is_closed() {
            return;
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(RECONNECT_MAX);
    }
}

/// Fail every queued request during a reconnect: their template ids belonged
/// to the dead session.
fn drain_commands(rx: &mut mpsc::Receiver<TpCommand>) {
    while let Ok(cmd) = rx.try_recv() {
        match cmd {
            TpCommand::GetTemplate { reply } => {
                let _ = reply.send(Err(unavailable(
                    "sv2-tp connection lost; template ids from the old session are unusable",
                )));
            }
            TpCommand::Submit { reply, .. } => {
                let _ = reply.send(Err(unavailable(
                    "sv2-tp connection lost before SubmitSolution could be confirmed",
                )));
            }
        }
    }
}

struct Session {
    /// Complete templates by id. `current` names the freshest one.
    templates: BTreeMap<u64, CachedTemplate>,
    current: Option<u64>,
    pending_new: Option<PendingNew>,
    in_flight: Option<(u64, oneshot::Sender<Result<Template, TemplateError>>)>,
    /// (transaction-set merkle identity, template id) for every template this
    /// session has served, bounded. `SubmitSolution` is matched back through
    /// this map: TDP's only template reference is the id.
    served: VecDeque<([u8; 32], u64)>,
    /// Solutions awaiting their acceptance verdict (TDP has no ack).
    watches: Vec<(WireSolution, oneshot::Sender<Result<(), TemplateError>>)>,
}

impl Session {
    fn new() -> Self {
        Self {
            templates: BTreeMap::new(),
            current: None,
            pending_new: None,
            in_flight: None,
            served: VecDeque::new(),
            watches: Vec::new(),
        }
    }

    /// Remember which template id a served transaction set belongs to, so a
    /// later solution on exactly those transactions can be submitted.
    fn record_served(&mut self, template_id: u64, transactions: &[Vec<u8>]) {
        let txids = transactions
            .iter()
            .filter_map(|raw| crate::template::txid_of(raw))
            .collect::<Vec<_>>();
        if txids.len() != transactions.len() {
            tracing::error!(
                template_id,
                "sv2-tp: not recording served transactions (undecodable tx passed validation?)"
            );
            return;
        }
        let key = crate::block::merkle_root(&txids);
        if self
            .served
            .iter()
            .any(|(k, id)| *k == key && *id == template_id)
        {
            return;
        }
        self.served.push_back((key, template_id));
        while self.served.len() > MAX_SERVED_TEMPLATES {
            self.served.pop_front();
        }
    }

    /// Fail every waiter with the same reason (connection loss): no solution
    /// or request may silently vanish.
    fn fail_all(&mut self, reason: &str) {
        if let Some((_, reply)) = self.in_flight.take() {
            let _ = reply.send(Err(unavailable(reason.to_string())));
        }
        for (_, reply) in std::mem::take(&mut self.watches) {
            let _ = reply.send(Err(unavailable(reason.to_string())));
        }
    }

    /// Map a cached template into the pool `Template` including the
    /// subsidy/fee split, and check it against the configured chain
    /// (BIP54 facts included). Witness-decode failures were already recorded
    /// in `TxFetch`; value-underflow and chain mismatches fail here, once,
    /// per call.
    fn build(
        &self,
        cached: &CachedTemplate,
        config: &Sv2TpConfig,
        transactions: Vec<Vec<u8>>,
    ) -> Result<Template, TemplateError> {
        let subsidy = subsidy_at(cached.height, config.halving_interval);
        let fees = cached.coinbase_value.checked_sub(subsidy).ok_or_else(|| {
            unavailable(format!(
                "template {}: coinbase value {} below subsidy {} at height {}",
                cached.template_id, cached.coinbase_value, subsidy, cached.height
            ))
        })?;
        // TDP merkle_path is deepest-first; the pool Template keeps the
        // Stratum (topmost-first) order. The front recomputes the branch from
        // the transaction list regardless, but the mapped value must be
        // correct on its own.
        let merkle_branches = cached.merkle_path.iter().rev().cloned().collect();
        let template = Template {
            height: cached.height,
            prev_hash: cached.prev_hash,
            version: cached.version,
            bits: cached.bits,
            curtime: cached.curtime,
            // SetNewPrevHash.header_timestamp is the earliest time the TP
            // allows; TDP has no maxtime.
            min_time: cached.curtime,
            max_time: None,
            coinbase: cached.coinbase,
            subsidy,
            fees,
            coinbase_value: cached.coinbase_value,
            transactions,
            merkle_branches,
            coinbase_output_max_additional_size: config.coinbase_output_max_additional_size,
        };
        crate::chain_identity::validate_template_chain(&template, config.chain)?;
        Ok(template)
    }

    fn get_template(
        &mut self,
        config: &Sv2TpConfig,
        reply: oneshot::Sender<Result<Template, TemplateError>>,
    ) -> Option<RequestTransactionData> {
        let Some(current_id) = self.current else {
            let _ = reply.send(Err(unavailable(
                "no template from the TP yet (node in IBD, or reconnecting)",
            )));
            return None;
        };
        let Some(cached) = self.templates.get(&current_id) else {
            let _ = reply.send(Err(unavailable("sv2-tp cache inconsistency")));
            return None;
        };
        let fetch = cached.transactions.clone();
        match fetch {
            TxFetch::Rejected(reason) => {
                let _ = reply.send(Err(unavailable(reason)));
                None
            }
            TxFetch::Ready(txs) => {
                self.record_served(current_id, &txs);
                let Some(cached) = self.templates.get(&current_id) else {
                    let _ = reply.send(Err(unavailable("sv2-tp cache inconsistency")));
                    return None;
                };
                let built = self.build(cached, config, txs);
                let _ = reply.send(built);
                None
            }
            TxFetch::Pending => {
                if self.in_flight.is_some() {
                    let _ = reply.send(Err(unavailable(
                        "transaction-data request already in flight; retry on the next poll",
                    )));
                    return None;
                }
                self.in_flight = Some((current_id, reply));
                Some(RequestTransactionData {
                    template_id: current_id,
                })
            }
        }
    }

    fn on_new_template(&mut self, msg: &NewTemplate<'_>) {
        let Some(height) = parse_bip34_height(msg.coinbase_prefix.as_bytes()) else {
            tracing::error!(
                template_id = msg.template_id,
                "sv2-tp: ignoring NewTemplate with missing or ambiguous BIP34 height push"
            );
            self.pending_new = None;
            return;
        };
        // The TP declares the coinbase consensus fields it validated the
        // template under. A non-final sequence is BIP54 consensuscleanup and
        // must carry the exact height-1 locktime; anything else is refused
        // here so the template is never served.
        let coinbase = match CoinbaseFields::from_template_provider(
            height,
            msg.coinbase_tx_locktime,
            msg.coinbase_tx_input_sequence,
        ) {
            Ok(fields) => fields,
            Err(e) => {
                tracing::error!(
                    template_id = msg.template_id,
                    height,
                    error = %e,
                    "sv2-tp: ignoring NewTemplate with invalid coinbase consensus fields"
                );
                self.pending_new = None;
                return;
            }
        };
        let merkle_path = msg
            .merkle_path
            .iter()
            .filter_map(|h| h.as_ref().try_into().ok())
            .collect::<Vec<[u8; 32]>>();
        if merkle_path.len() != msg.merkle_path.len() {
            tracing::error!(
                template_id = msg.template_id,
                "sv2-tp: ignoring NewTemplate with a non-32-byte merkle path entry"
            );
            self.pending_new = None;
            return;
        }
        let pending = PendingNew {
            template_id: msg.template_id,
            height,
            version: msg.version,
            coinbase,
            coinbase_value: msg.coinbase_tx_value_remaining,
            merkle_path,
        };
        if msg.future_template {
            // SetNewPrevHash completes it; nothing is servable until then.
            self.pending_new = Some(pending);
            return;
        }
        // Same-tip refresh (fee push): no SetNewPrevHash follows, so the
        // prev-hash state carries over from the current template.
        let Some(current) = self.current.and_then(|id| self.templates.get(&id)) else {
            tracing::warn!(
                template_id = msg.template_id,
                "sv2-tp: non-future NewTemplate before any complete template; ignored"
            );
            return;
        };
        let (prev_hash, bits, curtime) = (current.prev_hash, current.bits, current.curtime);
        self.insert(CachedTemplate {
            template_id: pending.template_id,
            height: pending.height,
            prev_hash,
            bits,
            curtime,
            version: pending.version,
            coinbase: pending.coinbase,
            coinbase_value: pending.coinbase_value,
            merkle_path: pending.merkle_path,
            transactions: TxFetch::Pending,
        });
    }

    fn on_set_new_prev_hash(&mut self, msg: &SetNewPrevHash<'_>) {
        let prev_hash: [u8; 32] = match msg.prev_hash.as_ref().try_into() {
            Ok(h) => h,
            Err(_) => {
                tracing::error!("sv2-tp: ignoring SetNewPrevHash with a non-32-byte prev hash");
                return;
            }
        };
        let Some(pending) = self.pending_new.take() else {
            tracing::warn!(
                template_id = msg.template_id,
                "sv2-tp: SetNewPrevHash without a preceding future NewTemplate; ignored"
            );
            return;
        };
        if pending.template_id != msg.template_id {
            tracing::error!(
                expected = pending.template_id,
                got = msg.template_id,
                "sv2-tp: SetNewPrevHash does not match the pending NewTemplate; dropping both"
            );
            return;
        }
        tracing::debug!(
            template_id = pending.template_id,
            height = pending.height,
            "sv2-tp: new template on a new tip"
        );
        // The push confirms a tip at height `pending.height - 1`: a template
        // at height H comes after the block at H-1.
        let accepted_height = pending.height.saturating_sub(1);
        self.insert(CachedTemplate {
            template_id: pending.template_id,
            height: pending.height,
            prev_hash,
            bits: msg.n_bits,
            curtime: msg.header_timestamp,
            version: pending.version,
            coinbase: pending.coinbase,
            coinbase_value: pending.coinbase_value,
            merkle_path: pending.merkle_path,
            transactions: TxFetch::Pending,
        });
        self.resolve_solutions(prev_hash, accepted_height);
    }

    /// Resolve the `SubmitSolution` watches at a tip push. The TP has no ack:
    /// our block being the push's prev-hash is acceptance; another block at
    /// our height or beyond means the template went stale and the solution
    /// lost the race (or was dropped).
    fn resolve_solutions(&mut self, tip_hash: [u8; 32], tip_height: u32) {
        let mut remaining = Vec::new();
        for (solution, reply) in std::mem::take(&mut self.watches) {
            if tip_hash == solution.block_hash {
                tracing::info!(
                    template_id = solution.template_id,
                    height = solution.height,
                    "sv2-tp: SubmitSolution accepted by the TP"
                );
                let _ = reply.send(Ok(()));
            } else if tip_height >= solution.height {
                let _ = reply.send(Err(TemplateError::Rejected(format!(
                    "stale-template-id: tip advanced to height {tip_height} without our block at height {}",
                    solution.height
                ))));
            } else {
                remaining.push((solution, reply));
            }
        }
        self.watches = remaining;
    }

    fn insert(&mut self, template: CachedTemplate) {
        let id = template.template_id;
        self.templates.insert(id, template);
        self.current = Some(id);
        while self.templates.len() > MAX_CACHED_TEMPLATES {
            // Keep `current`; drop the oldest id.
            let Some(oldest) = self.templates.keys().next().copied() else {
                break;
            };
            self.templates.remove(&oldest);
        }
    }

    fn on_tx_success(&mut self, config: &Sv2TpConfig, msg: &RequestTransactionDataSuccess<'_>) {
        let template_id = msg.template_id;
        let raw: Vec<Vec<u8>> = msg
            .transaction_list
            .iter()
            .map(|t| t.as_bytes().to_vec())
            .collect();
        // Strict decode of every transaction (witness allowed; the pool
        // coinbase carries the BIP141 commitment). Per-transaction exclusion
        // is impossible over TDP - SubmitSolution reuses the TP's retained
        // transaction set - so an undecodable or ambiguous transaction
        // poisons the whole template (fail closed), and a BIP54-forbidden
        // 64-byte transaction does too.
        let verdict = match crate::template::analyze_transactions(&raw) {
            Ok(_) => {
                let bip54 = raw.iter().enumerate().find_map(|(i, bytes)| {
                    let tx = crate::template::decode_transaction_strict(bytes).ok()?;
                    self.templates
                        .get(&template_id)
                        .and_then(|c| c.coinbase.validate_transaction(&tx).err())
                        .map(|e| (i, e))
                });
                match bip54 {
                    None => TxFetch::Ready(raw),
                    Some((i, e)) => {
                        TxFetch::Rejected(format!("template {template_id}: transaction {i}: {e}"))
                    }
                }
            }
            Err(e) => TxFetch::Rejected(format!("template {template_id}: {e}")),
        };
        if let TxFetch::Rejected(reason) = &verdict {
            tracing::warn!("sv2-tp: {reason}");
        }
        let ready_txs = match &verdict {
            TxFetch::Ready(txs) => Some(txs.clone()),
            _ => None,
        };
        if let Some(cached) = self.templates.get_mut(&template_id) {
            cached.transactions = verdict;
        }
        if let Some(txs) = ready_txs {
            self.record_served(template_id, &txs);
        }
        let Some((in_flight_id, reply)) = self.in_flight.take() else {
            return;
        };
        if in_flight_id != template_id {
            let _ = reply.send(Err(unavailable(format!(
                "sv2-tp: transaction data for template {template_id} while {in_flight_id} was requested"
            ))));
            return;
        }
        let Some(cached) = self.templates.get(&template_id) else {
            let _ = reply.send(Err(unavailable(format!(
                "template {template_id} aged out before its transactions arrived"
            ))));
            return;
        };
        let built = match &cached.transactions {
            TxFetch::Ready(txs) => self.build(cached, config, txs.clone()),
            TxFetch::Rejected(reason) => Err(unavailable(reason.clone())),
            TxFetch::Pending => Err(unavailable("sv2-tp cache inconsistency")),
        };
        let _ = reply.send(built);
    }

    fn on_tx_error(&mut self, msg: &RequestTransactionDataError<'_>) {
        let code = String::from_utf8_lossy(msg.error_code.as_bytes()).into_owned();
        let Some((in_flight_id, reply)) = self.in_flight.take() else {
            return;
        };
        // The TP's own error name (e.g. stale-template-id) is surfaced
        // verbatim: it is the only rejection channel TDP has.
        let _ = reply.send(Err(unavailable(format!(
            "RequestTransactionData for template {in_flight_id} failed: {code}"
        ))));
    }
}

async fn write_message<T>(
    w: &mut NoiseTcpWriteHalf,
    msg: T,
    msg_type: u8,
) -> Result<(), TemplateError>
where
    T: binary_sv2::Serialize + binary_sv2::GetSize,
{
    let frame = MessageFrame::from_message(msg, msg_type, 0, false)
        .map_err(|e| unavailable(format!("sv2-tp frame: {e}")))?;
    w.write_frame(frame)
        .await
        .map_err(|e| unavailable(format!("sv2-tp write: {e}")))
}

async fn run_session(
    config: &Sv2TpConfig,
    rx: &mut mpsc::Receiver<TpCommand>,
) -> Result<(), TemplateError> {
    let stream = TcpStream::connect(config.address)
        .await
        .map_err(|e| unavailable(format!("sv2-tp connect {}: {e}", config.address)))?;
    let stream = connect_with_noise(stream, Some(config.authority))
        .await
        .map_err(|e| unavailable(format!("sv2-tp noise handshake: {e}")))?;
    tracing::info!(addr = %config.address, "sv2-tp Noise_NX session established");
    let (mut read, mut write) = stream.into_split();

    let s = |x: &'static str| -> Result<Str0255<'static>, TemplateError> {
        Str0255::try_from(x).map_err(|e| unavailable(format!("setup string: {e}")))
    };
    let setup = SetupConnection {
        protocol: Protocol::TemplateDistributionProtocol,
        min_version: 2,
        max_version: 2,
        flags: 0,
        endpoint_host: s("127.0.0.1")?,
        endpoint_port: 0,
        vendor: s("silent-pool")?,
        hardware_version: s("1")?,
        firmware: s("1")?,
        device_id: s("1")?,
    };
    write_message(&mut write, setup, MESSAGE_TYPE_SETUP_CONNECTION).await?;

    let mut frame = timeout(REQUEST_TIMEOUT, read.read_frame())
        .await
        .map_err(|_| unavailable("sv2-tp: SetupConnection response timed out"))?
        .map_err(|e| unavailable(format!("sv2-tp read: {e}")))?;
    match frame.header().msg_type() {
        MESSAGE_TYPE_SETUP_CONNECTION_SUCCESS => {
            let mut payload = frame.payload().to_vec();
            let _: SetupConnectionSuccess = binary_sv2::from_bytes(&mut payload)
                .map_err(|e| unavailable(format!("sv2-tp setup decode: {e}")))?;
        }
        MESSAGE_TYPE_SETUP_CONNECTION_ERROR => {
            let mut payload = frame.payload().to_vec();
            let err: SetupConnectionError = binary_sv2::from_bytes(&mut payload)
                .map_err(|e| unavailable(format!("sv2-tp setup decode: {e}")))?;
            let code = String::from_utf8_lossy(err.error_code.as_bytes()).into_owned();
            return Err(unavailable(format!(
                "sv2-tp SetupConnection rejected: {code}"
            )));
        }
        other => {
            return Err(unavailable(format!(
                "sv2-tp: unexpected message 0x{other:02x} during setup"
            )))
        }
    }

    write_message(
        &mut write,
        stratum_core::template_distribution_sv2::CoinbaseOutputConstraints {
            coinbase_output_max_additional_size: config.coinbase_output_max_additional_size,
            coinbase_output_max_additional_sigops: config.coinbase_output_max_additional_sigops,
        },
        stratum_core::template_distribution_sv2::MESSAGE_TYPE_COINBASE_OUTPUT_CONSTRAINTS,
    )
    .await?;
    tracing::info!(
        size = config.coinbase_output_max_additional_size,
        sigops = config.coinbase_output_max_additional_sigops,
        "sv2-tp constraints sent"
    );

    let mut session = Session::new();
    let mut request_deadline: Option<Instant> = None;
    loop {
        // Clear finished request timers; arm a new one when a request starts.
        let deadline = request_deadline.unwrap_or_else(far_future);
        tokio::select! {
            frame = read.read_frame() => {
                let mut frame = match frame {
                    Ok(f) => f,
                    Err(e) => {
                        let reason = format!("sv2-tp connection lost: {e}");
                        session.fail_all(&reason);
                        return Err(unavailable(reason));
                    }
                };
                let ty = frame.header().msg_type();
                let payload = frame.payload();
                match ty {
                    MESSAGE_TYPE_NEW_TEMPLATE => {
                        let msg: NewTemplate = binary_sv2::from_bytes(payload)
                            .map_err(|e| unavailable(format!("sv2-tp NewTemplate decode: {e}")))?;
                        session.on_new_template(&msg);
                    }
                    MESSAGE_TYPE_SET_NEW_PREV_HASH => {
                        let msg: SetNewPrevHash = binary_sv2::from_bytes(payload)
                            .map_err(|e| unavailable(format!("sv2-tp SetNewPrevHash decode: {e}")))?;
                        session.on_set_new_prev_hash(&msg);
                    }
                    MESSAGE_TYPE_REQUEST_TRANSACTION_DATA_SUCCESS => {
                        let msg: RequestTransactionDataSuccess = binary_sv2::from_bytes(payload)
                            .map_err(|e| unavailable(format!("sv2-tp tx-success decode: {e}")))?;
                        session.on_tx_success(config, &msg);
                        request_deadline = None;
                    }
                    MESSAGE_TYPE_REQUEST_TRANSACTION_DATA_ERROR => {
                        let msg: RequestTransactionDataError = binary_sv2::from_bytes(payload)
                            .map_err(|e| unavailable(format!("sv2-tp tx-error decode: {e}")))?;
                        session.on_tx_error(&msg);
                        request_deadline = None;
                    }
                    other => {
                        tracing::debug!(msg_type = other, "sv2-tp: unexpected frame ignored");
                    }
                }
            }
            cmd = rx.recv() => {
                match cmd {
                    Some(TpCommand::GetTemplate { reply }) => {
                        if let Some(request) = session.get_template(config, reply) {
                            write_message(&mut write, request, MESSAGE_TYPE_REQUEST_TRANSACTION_DATA).await?;
                            request_deadline = Some(Instant::now() + REQUEST_TIMEOUT);
                        }
                    }
                    Some(TpCommand::Submit { mut solution, txset_key, reply }) => {
                        // Match the served transaction set back to its
                        // template id, or fail closed without touching the
                        // wire (the TP would drop an unknown id silently).
                        let Some(template_id) = session
                            .served
                            .iter()
                            .rev()
                            .find(|(key, _)| *key == txset_key)
                            .map(|(_, id)| *id)
                        else {
                            let _ = reply.send(Err(TemplateError::Rejected(
                                "stale-template-id: the found block's transaction set matches no template served by this TDP session".into(),
                            )));
                            continue;
                        };
                        if solution.coinbase_tx.len() > usize::from(u16::MAX) {
                            let _ = reply.send(Err(TemplateError::Rejected(format!(
                                "sv2-tp: coinbase is {} bytes, above the B064K SubmitSolution limit",
                                solution.coinbase_tx.len()
                            ))));
                            continue;
                        }
                        solution.template_id = template_id;
                        let send_result = {
                            let coinbase = match stratum_core::binary_sv2::B064K::new(
                                &solution.coinbase_tx,
                            ) {
                                Ok(c) => c,
                                Err(e) => {
                                    let _ = reply.send(Err(TemplateError::Rejected(format!(
                                        "sv2-tp coinbase encode: {e}"
                                    ))));
                                    continue;
                                }
                            };
                            let frame = SubmitSolution {
                                template_id,
                                version: solution.version,
                                header_timestamp: solution.header_timestamp,
                                header_nonce: solution.header_nonce,
                                coinbase_tx: coinbase,
                            };
                            write_message(&mut write, frame, MESSAGE_TYPE_SUBMIT_SOLUTION).await
                        };
                        if let Err(e) = send_result {
                            let reason = format!("sv2-tp connection lost on SubmitSolution: {e}");
                            session.fail_all(&reason);
                            return Err(e);
                        }
                        session.watches.push((solution, reply));
                    }
                    None => return Ok(()),
                }
            }
            _ = tokio::time::sleep_until(deadline), if request_deadline.is_some() => {
                if let Some((id, reply)) = session.in_flight.take() {
                    let _ = reply.send(Err(unavailable(format!(
                        "RequestTransactionData for template {id} timed out"
                    ))));
                }
                request_deadline = None;
            }
        }
    }
}

/// A select-arm deadline placeholder used only when no request is in flight
/// (the arm is also disabled by its guard).
fn far_future() -> Instant {
    Instant::now() + Duration::from_secs(365 * 24 * 3600)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::absolute::LockTime;
    use bitcoin::transaction::Version;
    use bitcoin::{Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness};
    use std::sync::{Arc, Mutex};
    use stratum_apps::key_utils::Secp256k1SecretKey;
    use stratum_apps::network_helpers::noise_stream::NoiseTcpReadHalf;
    use stratum_core::binary_sv2::{Seq0255, Seq064K, B016M, B0255, B064K, U256};
    use stratum_core::template_distribution_sv2::{
        CoinbaseOutputConstraints, RequestTransactionData,
        MESSAGE_TYPE_COINBASE_OUTPUT_CONSTRAINTS, MESSAGE_TYPE_REQUEST_TRANSACTION_DATA,
    };
    use tokio::task::JoinHandle;

    /// key-utils base58check vectors derived from the codec_sv2 example key
    /// (`9auqWEz…`), payload `01 00 || 24ee…6ce7`.
    const KEY_GOOD: &str = "9auqWEzQDVyd2oe1JVGFLMLHZtCo2FFqZwtKA5gd9xbuEu7PH72";
    const KEY_BAD_CHECKSUM: &str = "9auqWEzQDVyd2oe1JVGFLMLHZtCo2FFqZwtKA5gd9xbuEu7PH74";
    const KEY_WRONG_VERSION: &str = "JAYQfbtaNR7bYpQZNtA3WSpY15rdSWz11a1scRn7diKBa6MRbxT";
    const KEY_WRONG_LENGTH: &str = "2worxZd7cunTHiqG2F7B3a9uEo3DhA7zDuJyDw5Ss4Y1s5fyKt";

    #[test]
    fn authority_key_forms_are_fail_closed() {
        let key = parse_tp_authority_key(KEY_GOOD).expect("valid base58check");
        let expected = "24ee3c3804a1aaa4c03b80ea19f7a5863c916e8994b7db94a3bad7ee092b6ce7";
        assert_eq!(hex::encode(key.0.serialize()), expected);
        assert!(parse_tp_authority_key(KEY_BAD_CHECKSUM).is_err());
        assert!(parse_tp_authority_key(KEY_WRONG_VERSION).is_err());
        assert!(parse_tp_authority_key(KEY_WRONG_LENGTH).is_err());

        // Hex: the secp256k1 generator's x coordinate is a valid x-only key.
        let hex_key = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
        let key = parse_tp_authority_key(hex_key).expect("valid hex");
        assert_eq!(
            hex::decode(hex_key).expect("hex").as_slice(),
            &key.0.serialize()[..]
        );
        assert!(parse_tp_authority_key("79be667e").is_err(), "short");
        let bad_hex = format!("{}zz", &hex_key[..62]);
        assert!(parse_tp_authority_key(&bad_hex).is_err(), "non-hex");
        assert!(parse_tp_authority_key("").is_err());
    }

    #[test]
    fn regtest_subsidy_halves_every_150_blocks() {
        assert_eq!(subsidy_at(0, 150), 5_000_000_000);
        assert_eq!(subsidy_at(149, 150), 5_000_000_000);
        assert_eq!(subsidy_at(150, 150), 2_500_000_000);
        assert_eq!(subsidy_at(151, 150), 2_500_000_000);
        assert_eq!(subsidy_at(300, 150), 1_250_000_000);
        assert_eq!(subsidy_at(450, 150), 625_000_000);
        assert_eq!(subsidy_at(150 * 64, 150), 0);
    }

    #[test]
    fn other_networks_halve_every_210_000_blocks() {
        assert_eq!(subsidy_at(209_999, 210_000), 5_000_000_000);
        assert_eq!(subsidy_at(210_000, 210_000), 2_500_000_000);
        assert_eq!(subsidy_at(420_000, 210_000), 1_250_000_000);
        let far: u32 = 210_000 * 64;
        assert_eq!(subsidy_at(far, 210_000), 0);
    }

    #[test]
    fn bip34_height_push_is_strict() {
        assert_eq!(parse_bip34_height(&[0x00]), Some(0));
        assert_eq!(parse_bip34_height(&[0x51]), Some(1));
        assert_eq!(parse_bip34_height(&[0x60]), Some(16));
        assert_eq!(parse_bip34_height(&[0x01, 0x11]), Some(17));
        assert_eq!(parse_bip34_height(&[0x01, 0x66]), Some(102));
        assert_eq!(parse_bip34_height(&[0x02, 0x80, 0x00]), Some(128));
        assert_eq!(parse_bip34_height(&[0x02, 0x96, 0x00]), Some(150));
        assert_eq!(parse_bip34_height(&[0x02, 0xe8, 0x03]), Some(1000));
        assert_eq!(
            parse_bip34_height(&[0x05, 0xff, 0xff, 0xff, 0xff, 0x00]),
            Some(u32::MAX)
        );
        assert_eq!(parse_bip34_height(&[]), None);
        assert_eq!(parse_bip34_height(&[0x01, 0x00]), None); // zero as a data push
        assert_eq!(parse_bip34_height(&[0x02, 0x64, 0x00]), None); // redundant sign byte
        assert_eq!(parse_bip34_height(&[0x01, 0x80]), None); // negative zero
        assert_eq!(parse_bip34_height(&[0x02, 0x11, 0x80]), None); // negative
        assert_eq!(parse_bip34_height(&[0x02, 0x11]), None); // truncated
        assert_eq!(parse_bip34_height(&[0x4c, 0x01, 0x11]), None); // OP_PUSHDATA1
        assert_eq!(parse_bip34_height(&[0x06, 1, 0, 0, 0, 0, 0]), None);

        // The encoding in the pool's own coinbase parses back to the same
        // height: spc-crypto and rbitcoin emit the identical push.
        for h in [
            0u32,
            1,
            16,
            17,
            100,
            127,
            128,
            150,
            1024,
            65535,
            1_000_000,
            u32::MAX,
        ] {
            assert_eq!(
                parse_bip34_height(&spc_crypto::height_push_bytes(h)),
                Some(h),
                "height {h}"
            );
        }
    }

    #[test]
    fn constraint_sizes_track_the_coinbase_ceiling() {
        assert_eq!(coinbase_budget_additional_size(1_000_000), 250_000);
        assert_eq!(coinbase_budget_additional_size(50_000), 12_500);
        assert_eq!(coinbase_additional_sigops(), 0);
    }

    fn raw_tx(witness: bool, marker: u64) -> Vec<u8> {
        let tx = Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::null(),
                script_sig: ScriptBuf::new(),
                sequence: Sequence::MAX,
                witness: if witness {
                    Witness::from_slice(&[vec![0u8; 4]])
                } else {
                    Witness::default()
                },
            }],
            output: vec![TxOut {
                value: Amount::from_sat(marker),
                script_pubkey: ScriptBuf::from_bytes(vec![0x51]),
            }],
        };
        bitcoin::consensus::serialize(&tx)
    }

    fn authority_keypair() -> (Secp256k1PublicKey, Secp256k1SecretKey) {
        let secp = secp256k1_legacy::Secp256k1::new();
        let mut rng = rand::thread_rng();
        let kp = secp256k1_legacy::Keypair::new(&secp, &mut rng);
        (
            Secp256k1PublicKey(kp.x_only_public_key().0),
            Secp256k1SecretKey(kp.secret_key()),
        )
    }

    /// Bind a mock TP listener on a random loopback port. `serve` gets the
    /// Noise-responder halves of the first accepted connection.
    fn start_mock<F, Fut>(
        serve: F,
    ) -> (
        SocketAddr,
        Arc<Mutex<Vec<String>>>,
        JoinHandle<()>,
        Secp256k1PublicKey,
    )
    where
        F: FnOnce(NoiseTcpReadHalf, NoiseTcpWriteHalf, Arc<Mutex<Vec<String>>>) -> Fut
            + Send
            + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let (pk, sk) = authority_keypair();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        listener.set_nonblocking(true).expect("nonblocking");
        let addr = listener.local_addr().expect("addr");
        let events: Arc<Mutex<Vec<String>>> = Arc::default();
        let mock_events = events.clone();
        let task = tokio::spawn(async move {
            let listener = tokio::net::TcpListener::from_std(listener).expect("listener");
            let (socket, _) = listener.accept().await.expect("accept");
            let stream =
                stratum_apps::network_helpers::accept_noise_connection(socket, pk, sk, 3600)
                    .await
                    .expect("responder handshake");
            let (read, write) = stream.into_split();
            serve(read, write, mock_events).await;
        });
        (addr, events, task, pk)
    }

    fn test_config(addr: SocketAddr, authority: Secp256k1PublicKey) -> Sv2TpConfig {
        Sv2TpConfig {
            address: addr,
            authority,
            coinbase_output_max_additional_size: 250_000,
            coinbase_output_max_additional_sigops: 0,
            halving_interval: 150,
            chain: spc_crypto::ChainNetwork::Regtest,
        }
    }

    async fn read_one(read: &mut NoiseTcpReadHalf) -> (u8, Vec<u8>) {
        let mut frame = timeout(Duration::from_secs(10), read.read_frame())
            .await
            .expect("frame in time")
            .expect("frame");
        let ty = frame.header().msg_type();
        (ty, frame.payload().to_vec())
    }

    async fn send<T>(write: &mut NoiseTcpWriteHalf, msg: T, msg_type: u8)
    where
        T: binary_sv2::Serialize + binary_sv2::GetSize,
    {
        let frame = MessageFrame::from_message(msg, msg_type, 0, false).expect("frame");
        timeout(Duration::from_secs(10), write.write_frame(frame))
            .await
            .expect("write in time")
            .expect("write");
    }

    async fn negotiate(
        read: &mut NoiseTcpReadHalf,
        write: &mut NoiseTcpWriteHalf,
        events: &Arc<Mutex<Vec<String>>>,
    ) -> CoinbaseOutputConstraints {
        let (ty, mut payload) = read_one(read).await;
        assert_eq!(ty, MESSAGE_TYPE_SETUP_CONNECTION, "first frame is setup");
        let setup: SetupConnection = binary_sv2::from_bytes(&mut payload).expect("setup decode");
        assert_eq!(setup.protocol, Protocol::TemplateDistributionProtocol);
        assert_eq!((setup.min_version, setup.max_version), (2, 2));
        assert_eq!(setup.flags, 0);
        events.lock().expect("poisoned").push("setup".into());
        send(
            write,
            SetupConnectionSuccess {
                used_version: 2,
                flags: 0,
            },
            MESSAGE_TYPE_SETUP_CONNECTION_SUCCESS,
        )
        .await;
        let (ty, mut payload) = read_one(read).await;
        assert_eq!(
            ty, MESSAGE_TYPE_COINBASE_OUTPUT_CONSTRAINTS,
            "second frame is constraints"
        );
        let constraints: CoinbaseOutputConstraints =
            binary_sv2::from_bytes(&mut payload).expect("constraints decode");
        events.lock().expect("poisoned").push("constraints".into());
        constraints
    }

    fn template_msg<'a>(
        template_id: u64,
        future: bool,
        coinbase_value: u64,
        prefix: &'a [u8],
        merkle_path: &'a [[u8; 32]],
    ) -> NewTemplate<'a> {
        template_msg_fields(
            template_id,
            future,
            coinbase_value,
            prefix,
            merkle_path,
            u32::MAX,
            0,
        )
    }

    /// Like [`template_msg`], with explicit coinbase consensus fields
    /// (`coinbase_tx_input_sequence`, `coinbase_tx_locktime`) for the BIP54
    /// negotiation tests.
    fn template_msg_fields<'a>(
        template_id: u64,
        future: bool,
        coinbase_value: u64,
        prefix: &'a [u8],
        merkle_path: &'a [[u8; 32]],
        sequence: u32,
        locktime: u32,
    ) -> NewTemplate<'a> {
        let path = merkle_path.iter().map(U256::from).collect::<Vec<_>>();
        NewTemplate {
            template_id,
            future_template: future,
            version: 0x2000_0000,
            coinbase_tx_version: 2,
            coinbase_prefix: B0255::new(prefix).expect("prefix"),
            coinbase_tx_input_sequence: sequence,
            coinbase_tx_value_remaining: coinbase_value,
            coinbase_tx_outputs_count: 0,
            coinbase_tx_outputs: B064K::new(&[]).expect("empty"),
            coinbase_tx_locktime: locktime,
            merkle_path: Seq0255::new(path).expect("path"),
        }
    }

    fn tx_success_msg(template_id: u64, txs: &[Vec<u8>]) -> RequestTransactionDataSuccess<'_> {
        let list = txs.iter().map(|t| B016M::new(t).expect("tx"));
        RequestTransactionDataSuccess {
            template_id,
            excess_data: B064K::new(&[]).expect("empty"),
            transaction_list: Seq064K::new(list.collect()).expect("tx list"),
        }
    }

    /// Retry `get_template` like the front's ticker would, until `check`
    /// accepts the outcome or the deadline passes.
    async fn poll_outcome<F>(
        source: &Sv2TpTemplateSource,
        check: F,
    ) -> Result<Template, TemplateError>
    where
        F: Fn(&Result<Template, TemplateError>) -> bool,
    {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let outcome = source.get_template().await;
            if check(&outcome) {
                return outcome;
            }
            if Instant::now() > deadline {
                panic!("timed out waiting for the expected outcome: {outcome:?}");
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    /// Full happy path: handshake, constraints, NewTemplate+SetNewPrevHash,
    /// RequestTransactionData, and the complete field mapping including the
    /// regtest subsidy/fees split, the witness transaction and the
    /// merkle-path order.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn template_mapping_handshake_constraints_and_txdata() {
        let tx1 = raw_tx(false, 10_000);
        let tx2 = raw_tx(true, 20_000); // witness transactions are allowed
        let txid1 = crate::template::txid_of(&tx1).expect("txid");
        let txid2 = crate::template::txid_of(&tx2).expect("txid");
        // The pool Template keeps the Stratum branch order (topmost first);
        // TDP supplies the same path deepest-first.
        let stratum_branch = crate::block::coinbase_merkle_branch(&[txid1, txid2]);
        let tdp_path: Vec<[u8; 32]> = stratum_branch.iter().rev().cloned().collect();
        let subsidy = subsidy_at(151, 150);
        let fees = 123_456u64;
        let prev_hash = [0x22; 32];
        let time = 1_700_000_000u32;
        let bits = 0x207f_ffffu32;

        let (addr, events, task, pk) = start_mock(move |mut read, mut write, events| {
            let txs = vec![tx1.clone(), tx2.clone()];
            let tdp_path = tdp_path.clone();
            async move {
                let constraints = negotiate(&mut read, &mut write, &events).await;
                assert_eq!(constraints.coinbase_output_max_additional_size, 250_000);
                assert_eq!(constraints.coinbase_output_max_additional_sigops, 0);

                let prefix = spc_crypto::height_push_bytes(151);
                send(
                    &mut write,
                    template_msg(1, true, subsidy + fees, &prefix, &tdp_path),
                    MESSAGE_TYPE_NEW_TEMPLATE,
                )
                .await;
                send(
                    &mut write,
                    SetNewPrevHash {
                        template_id: 1,
                        prev_hash: U256::from(&prev_hash),
                        header_timestamp: time,
                        n_bits: bits,
                        target: U256::from(&[0xff; 32]),
                    },
                    MESSAGE_TYPE_SET_NEW_PREV_HASH,
                )
                .await;

                let (ty, mut payload) = read_one(&mut read).await;
                assert_eq!(ty, MESSAGE_TYPE_REQUEST_TRANSACTION_DATA);
                let req: RequestTransactionData =
                    binary_sv2::from_bytes(&mut payload).expect("request decode");
                assert_eq!(req.template_id, 1);
                events.lock().expect("poisoned").push("txdata:1".into());
                send(
                    &mut write,
                    tx_success_msg(1, &txs),
                    MESSAGE_TYPE_REQUEST_TRANSACTION_DATA_SUCCESS,
                )
                .await;
                // Hold the session open while the client maps and re-reads.
                std::future::pending::<()>().await;
            }
        });

        let source = Sv2TpTemplateSource::spawn(test_config(addr, pk));
        let template = poll_outcome(&source, |r| r.is_ok())
            .await
            .expect("template");

        assert_eq!(template.height, 151);
        assert_eq!(template.prev_hash, prev_hash);
        assert_eq!(template.version, 0x2000_0000);
        assert_eq!(template.bits, bits);
        assert_eq!(template.curtime, time);
        assert_eq!(template.min_time, time);
        assert_eq!(template.max_time, None);
        assert_eq!(template.coinbase, CoinbaseFields::legacy(0));
        assert_eq!(template.subsidy, 2_500_000_000); // height 151 on regtest
        assert_eq!(template.fees, fees);
        assert_eq!(template.coinbase_value, subsidy + fees);
        assert_eq!(template.transactions.len(), 2);
        assert_eq!(template.merkle_branches, stratum_branch);
        assert_eq!(template.coinbase_output_max_additional_size, 250_000);

        // A second poll reuses the fetched transaction list: no second
        // RequestTransactionData is sent for the same template id.
        let again = source.get_template().await.expect("cached template");
        assert_eq!(again, template);
        let events = events.lock().expect("poisoned").clone();
        assert_eq!(events, vec!["setup", "constraints", "txdata:1"]);

        drop(source);
        task.abort();
    }

    /// The TP's declared coinbase consensus fields reach the mapped template:
    /// a non-final sequence with the exact height-1 locktime is BIP54
    /// consensuscleanup.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn bip54_coinbase_fields_are_negotiated_from_the_tp() {
        let subsidy = subsidy_at(151, 150);
        let (addr, _events, task, pk) = start_mock(move |mut read, mut write, events| async move {
            let _ = negotiate(&mut read, &mut write, &events).await;
            let prefix = spc_crypto::height_push_bytes(151);
            send(
                &mut write,
                template_msg_fields(1, true, subsidy, &prefix, &[], 0xffff_fffe, 150),
                MESSAGE_TYPE_NEW_TEMPLATE,
            )
            .await;
            send(
                &mut write,
                SetNewPrevHash {
                    template_id: 1,
                    prev_hash: U256::from(&[0x22; 32]),
                    header_timestamp: 1_700_000_000,
                    n_bits: 0x207f_ffff,
                    target: U256::from(&[0xff; 32]),
                },
                MESSAGE_TYPE_SET_NEW_PREV_HASH,
            )
            .await;
            let (ty, _payload) = read_one(&mut read).await;
            assert_eq!(ty, MESSAGE_TYPE_REQUEST_TRANSACTION_DATA);
            send(
                &mut write,
                tx_success_msg(1, &[]),
                MESSAGE_TYPE_REQUEST_TRANSACTION_DATA_SUCCESS,
            )
            .await;
            std::future::pending::<()>().await;
        });
        let source = Sv2TpTemplateSource::spawn(test_config(addr, pk));
        let template = poll_outcome(&source, |r| r.is_ok())
            .await
            .expect("template");
        assert!(template.coinbase.consensus_cleanup());
        assert_eq!(template.coinbase.locktime, 150);
        assert_eq!(template.coinbase.sequence, 0xffff_fffe);
        task.abort();
    }

    /// A non-final sequence with the WRONG locktime is not interpretable as
    /// BIP54 or legacy: the template is never served (fail closed).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn invalid_bip54_fields_reject_the_template() {
        let subsidy = subsidy_at(151, 150);
        let (addr, _events, task, pk) = start_mock(move |mut read, mut write, events| async move {
            let _ = negotiate(&mut read, &mut write, &events).await;
            let prefix = spc_crypto::height_push_bytes(151);
            // locktime 149 at height 151: BIP54 requires exactly 150.
            send(
                &mut write,
                template_msg_fields(1, true, subsidy, &prefix, &[], 0xffff_fffe, 149),
                MESSAGE_TYPE_NEW_TEMPLATE,
            )
            .await;
            send(
                &mut write,
                SetNewPrevHash {
                    template_id: 1,
                    prev_hash: U256::from(&[0x22; 32]),
                    header_timestamp: 1_700_000_000,
                    n_bits: 0x207f_ffff,
                    target: U256::from(&[0xff; 32]),
                },
                MESSAGE_TYPE_SET_NEW_PREV_HASH,
            )
            .await;
            std::future::pending::<()>().await;
        });
        let source = Sv2TpTemplateSource::spawn(test_config(addr, pk));
        let err = poll_outcome(&source, |r| match r {
            Err(TemplateError::Unavailable(e)) => e.contains("no template from the TP yet"),
            _ => false,
        })
        .await
        .expect_err("no template is servable");
        assert!(err.to_string().contains("no template from the TP yet"));
        task.abort();
    }

    /// A template carrying an undecodable transaction is refused (per-tx
    /// exclusion is impossible over TDP) and the refusal is cached: no
    /// re-request for the poisoned template.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn undecodable_transaction_poisons_the_template() {
        let good = raw_tx(false, 10_000);
        let garbage = vec![0x00, 0x01, 0x02];
        let subsidy = subsidy_at(151, 150);
        let (addr, events, task, pk) = start_mock(move |mut read, mut write, events| {
            let txs = vec![good.clone(), garbage.clone()];
            async move {
                let _ = negotiate(&mut read, &mut write, &events).await;
                let prefix = spc_crypto::height_push_bytes(151);
                send(
                    &mut write,
                    template_msg(1, true, subsidy, &prefix, &[]),
                    MESSAGE_TYPE_NEW_TEMPLATE,
                )
                .await;
                send(
                    &mut write,
                    SetNewPrevHash {
                        template_id: 1,
                        prev_hash: U256::from(&[0x22; 32]),
                        header_timestamp: 1_700_000_000,
                        n_bits: 0x207f_ffff,
                        target: U256::from(&[0xff; 32]),
                    },
                    MESSAGE_TYPE_SET_NEW_PREV_HASH,
                )
                .await;
                let (ty, _payload) = read_one(&mut read).await;
                assert_eq!(ty, MESSAGE_TYPE_REQUEST_TRANSACTION_DATA);
                events.lock().expect("poisoned").push("txdata:1".into());
                send(
                    &mut write,
                    tx_success_msg(1, &txs),
                    MESSAGE_TYPE_REQUEST_TRANSACTION_DATA_SUCCESS,
                )
                .await;
                std::future::pending::<()>().await;
            }
        });

        let source = Sv2TpTemplateSource::spawn(test_config(addr, pk));
        let _ = poll_outcome(&source, |r| match r {
            Err(TemplateError::Unavailable(e)) => e.contains("transaction 1"),
            _ => false,
        })
        .await;

        // The refusal is cached: no further transaction-data request.
        let err = source.get_template().await.expect_err("still refused");
        assert!(err.to_string().contains("transaction 1"), "{err}");
        let events = events.lock().expect("poisoned").clone();
        assert_eq!(events, vec!["setup", "constraints", "txdata:1"]);
        task.abort();
    }

    /// `coinbase_value` below the schedule's subsidy is an underflow: fail
    /// closed instead of wrapping into a fabricated fee count.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn fee_underflow_fails_closed() {
        let subsidy = subsidy_at(151, 150);
        let (addr, _events, task, pk) = start_mock(move |mut read, mut write, events| async move {
            let _ = negotiate(&mut read, &mut write, &events).await;
            let prefix = spc_crypto::height_push_bytes(151);
            // One satoshi below the subsidy for height 151.
            send(
                &mut write,
                template_msg(1, true, subsidy - 1, &prefix, &[]),
                MESSAGE_TYPE_NEW_TEMPLATE,
            )
            .await;
            send(
                &mut write,
                SetNewPrevHash {
                    template_id: 1,
                    prev_hash: U256::from(&[0x22; 32]),
                    header_timestamp: 1_700_000_000,
                    n_bits: 0x207f_ffff,
                    target: U256::from(&[0xff; 32]),
                },
                MESSAGE_TYPE_SET_NEW_PREV_HASH,
            )
            .await;
            let (ty, _payload) = read_one(&mut read).await;
            assert_eq!(ty, MESSAGE_TYPE_REQUEST_TRANSACTION_DATA);
            send(
                &mut write,
                tx_success_msg(1, &[]),
                MESSAGE_TYPE_REQUEST_TRANSACTION_DATA_SUCCESS,
            )
            .await;
            std::future::pending::<()>().await;
        });
        let source = Sv2TpTemplateSource::spawn(test_config(addr, pk));
        let _ = poll_outcome(&source, |r| match r {
            Err(TemplateError::Unavailable(e)) => e.contains("below subsidy"),
            _ => false,
        })
        .await;
        task.abort();
    }

    /// A NewTemplate whose `coinbase_prefix` has no canonical BIP34 push is
    /// never served; and a TP that has pushed nothing (IBD) yields no
    /// template at all rather than a placeholder.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ambiguous_height_and_ibd_serve_no_template() {
        let subsidy = subsidy_at(151, 150);
        let (addr, _events, task, pk) = start_mock(move |mut read, mut write, events| async move {
            let _ = negotiate(&mut read, &mut write, &events).await;
            // OP_PUSHDATA1 encoding: not a canonical height push.
            let prefix = [0x4c, 0x01, 0x96];
            send(
                &mut write,
                template_msg(1, true, subsidy, &prefix, &[]),
                MESSAGE_TYPE_NEW_TEMPLATE,
            )
            .await;
            send(
                &mut write,
                SetNewPrevHash {
                    template_id: 1,
                    prev_hash: U256::from(&[0x22; 32]),
                    header_timestamp: 1_700_000_000,
                    n_bits: 0x207f_ffff,
                    target: U256::from(&[0xff; 32]),
                },
                MESSAGE_TYPE_SET_NEW_PREV_HASH,
            )
            .await;
            std::future::pending::<()>().await;
        });
        let source = Sv2TpTemplateSource::spawn(test_config(addr, pk));
        let err = poll_outcome(&source, |r| match r {
            Err(TemplateError::Unavailable(e)) => e.contains("no template from the TP yet"),
            _ => false,
        })
        .await
        .expect_err("no template is servable");
        assert!(err.to_string().contains("no template from the TP yet"));
        task.abort();
    }

    /// RequestTransactionData.Error surfaces the TP's own error name.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn transaction_data_error_names_the_tp_error() {
        let subsidy = subsidy_at(151, 150);
        let (addr, _events, task, pk) = start_mock(move |mut read, mut write, events| async move {
            let _ = negotiate(&mut read, &mut write, &events).await;
            let prefix = spc_crypto::height_push_bytes(151);
            send(
                &mut write,
                template_msg(1, true, subsidy, &prefix, &[]),
                MESSAGE_TYPE_NEW_TEMPLATE,
            )
            .await;
            send(
                &mut write,
                SetNewPrevHash {
                    template_id: 1,
                    prev_hash: U256::from(&[0x22; 32]),
                    header_timestamp: 1_700_000_000,
                    n_bits: 0x207f_ffff,
                    target: U256::from(&[0xff; 32]),
                },
                MESSAGE_TYPE_SET_NEW_PREV_HASH,
            )
            .await;
            let (ty, _payload) = read_one(&mut read).await;
            assert_eq!(ty, MESSAGE_TYPE_REQUEST_TRANSACTION_DATA);
            send(
                &mut write,
                RequestTransactionDataError {
                    template_id: 1,
                    error_code: Str0255::try_from("stale-template-id").expect("code"),
                },
                MESSAGE_TYPE_REQUEST_TRANSACTION_DATA_ERROR,
            )
            .await;
            std::future::pending::<()>().await;
        });
        let source = Sv2TpTemplateSource::spawn(test_config(addr, pk));
        let err = poll_outcome(&source, |r| match r {
            Err(TemplateError::Unavailable(e)) => e.contains("stale-template-id"),
            _ => false,
        })
        .await
        .expect_err("the TP error name is surfaced");
        assert!(err.to_string().contains("stale-template-id"));
        task.abort();
    }

    /// A structurally valid witness-free coinbase whose scriptSig starts with
    /// the canonical BIP34 push, plus 16 bytes of extranonce.
    fn test_coinbase(height: u32) -> Vec<u8> {
        let mut script_sig = spc_crypto::emit_script_sig(height, &[7u8; 32]);
        script_sig.extend_from_slice(&[0xaa; 16]);
        let tx = Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::null(),
                script_sig: ScriptBuf::from_bytes(script_sig),
                sequence: Sequence::MAX,
                witness: Witness::default(),
            }],
            output: vec![TxOut {
                value: Amount::from_sat(2_500_000_000),
                script_pubkey: ScriptBuf::from_bytes(
                    std::iter::once(0x51)
                        .chain(std::iter::once(0x20))
                        .chain([9u8; 32])
                        .collect::<Vec<u8>>(),
                ),
            }],
        };
        bitcoin::consensus::serialize(&tx)
    }

    struct Submission {
        template_id: u64,
        version: u32,
        header_timestamp: u32,
        header_nonce: u32,
        coinbase_tx: Vec<u8>,
    }

    /// Read and decode the next `SubmitSolution` on the mock server.
    async fn expect_solution(read: &mut NoiseTcpReadHalf) -> Submission {
        let (ty, mut payload) = read_one(read).await;
        assert_eq!(ty, MESSAGE_TYPE_SUBMIT_SOLUTION);
        let sol: SubmitSolution = binary_sv2::from_bytes(&mut payload).expect("solution decode");
        Submission {
            template_id: sol.template_id,
            version: sol.version,
            header_timestamp: sol.header_timestamp,
            header_nonce: sol.header_nonce,
            coinbase_tx: sol.coinbase_tx.as_bytes().to_vec(),
        }
    }

    /// Serve template 1 at height 151 whose tx set is `txs`.
    async fn serve_one_template(
        read: &mut NoiseTcpReadHalf,
        write: &mut NoiseTcpWriteHalf,
        events: &Arc<Mutex<Vec<String>>>,
        txs: Vec<Vec<u8>>,
    ) {
        let _ = negotiate(read, write, events).await;
        let prefix = spc_crypto::height_push_bytes(151);
        send(
            write,
            template_msg(1, true, subsidy_at(151, 150), &prefix, &[]),
            MESSAGE_TYPE_NEW_TEMPLATE,
        )
        .await;
        send(
            write,
            SetNewPrevHash {
                template_id: 1,
                prev_hash: U256::from(&[0x22; 32]),
                header_timestamp: 1_700_000_000,
                n_bits: 0x207f_ffff,
                target: U256::from(&[0xff; 32]),
            },
            MESSAGE_TYPE_SET_NEW_PREV_HASH,
        )
        .await;
        let (ty, _payload) = read_one(read).await;
        assert_eq!(ty, MESSAGE_TYPE_REQUEST_TRANSACTION_DATA);
        send(
            write,
            tx_success_msg(1, &txs),
            MESSAGE_TYPE_REQUEST_TRANSACTION_DATA_SUCCESS,
        )
        .await;
    }

    /// A found block goes out as `SubmitSolution` with the template id it was
    /// served, and acceptance is confirmed by the TP's next tip push.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn submit_solution_carries_the_served_template_and_header() {
        use bitcoin::hashes::{sha256d, Hash as _};
        let tx1 = raw_tx(false, 10_000);
        let coinbase = test_coinbase(151);
        let version = 0x2000_0000u32;
        let time = 1_700_000_000u32;
        let bits = 0x207f_ffffu32;
        let nonce = 42u32;
        let assembled = crate::block::assemble(
            version,
            [0x22; 32],
            time,
            bits,
            nonce,
            &coinbase,
            std::slice::from_ref(&tx1),
        );
        let block_hash = sha256d::Hash::hash(&assembled.header).to_byte_array();

        let (addr, _events, task, pk) = start_mock(move |mut read, mut write, events| {
            let cb = coinbase.clone();
            async move {
                serve_one_template(&mut read, &mut write, &events, vec![tx1.clone()]).await;
                let sol = expect_solution(&mut read).await;
                assert_eq!(sol.template_id, 1);
                assert_eq!(sol.version, 0x2000_0000);
                assert_eq!(sol.header_timestamp, time);
                assert_eq!(sol.header_nonce, nonce);
                assert_eq!(sol.coinbase_tx, cb);
                events.lock().expect("poisoned").push("solution:1".into());

                // Acceptance: the new template's SetNewPrevHash points at OUR
                // block's hash, at height 151 (next template height 152).
                let ours = block_hash;
                let prefix = spc_crypto::height_push_bytes(152);
                send(
                    &mut write,
                    template_msg(2, true, subsidy_at(152, 150), &prefix, &[]),
                    MESSAGE_TYPE_NEW_TEMPLATE,
                )
                .await;
                send(
                    &mut write,
                    SetNewPrevHash {
                        template_id: 2,
                        prev_hash: U256::from(&ours),
                        header_timestamp: time + 1,
                        n_bits: bits,
                        target: U256::from(&[0xff; 32]),
                    },
                    MESSAGE_TYPE_SET_NEW_PREV_HASH,
                )
                .await;
                std::future::pending::<()>().await;
            }
        });

        let source = Sv2TpTemplateSource::spawn(test_config(addr, pk));
        let _ = poll_outcome(&source, |r| r.is_ok()).await;
        crate::template::BlockSubmitter::submit_assembled(&source, &assembled)
            .await
            .expect("TP accepted the solution");
        assert_eq!(block_hash.len(), 32);
        task.abort();
    }

    /// A block whose transaction set no served template produced is refused
    /// locally with `stale-template-id`, and nothing reaches the wire.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn submit_on_an_unserved_txset_fails_closed_off_wire() {
        let tx1 = raw_tx(false, 10_000);
        let different = raw_tx(false, 99_999);
        let assembled = crate::block::assemble(
            0x2000_0000,
            [0x22; 32],
            1_700_000_000,
            0x207f_ffff,
            7,
            &test_coinbase(151),
            &[different],
        );

        let (addr, _events, task, pk) = start_mock(move |mut read, mut write, events| async move {
            serve_one_template(&mut read, &mut write, &events, vec![tx1.clone()]).await;
            // No solution must arrive while everything matches nothing.
            let got = timeout(Duration::from_millis(400), read.read_frame()).await;
            assert!(
                got.is_err(),
                "no wire frame expected after a local rejection"
            );
        });

        let source = Sv2TpTemplateSource::spawn(test_config(addr, pk));
        let _ = poll_outcome(&source, |r| r.is_ok()).await;
        let err = crate::template::BlockSubmitter::submit_assembled(&source, &assembled)
            .await
            .expect_err("stale template");
        assert!(err.to_string().contains("stale-template-id"), "{err}");
        task.abort();
    }

    /// When the TP's tip moves on a different block at our height, the
    /// solution is reported as rejected (stale template), not accepted and
    /// not dropped.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_competing_tip_rejects_the_solution() {
        let tx1 = raw_tx(false, 10_000);
        let coinbase = test_coinbase(151);
        let time = 1_700_000_000u32;
        let bits = 0x207f_ffffu32;
        let nonce = 55u32;
        let assembled = crate::block::assemble(
            0x2000_0000,
            [0x22; 32],
            time,
            bits,
            nonce,
            &coinbase,
            std::slice::from_ref(&tx1),
        );

        let (addr, _events, task, pk) = start_mock(move |mut read, mut write, events| async move {
            serve_one_template(&mut read, &mut write, &events, vec![tx1.clone()]).await;
            let _ = expect_solution(&mut read).await;
            // Another block at height 151 wins: prev-hash is not our hash.
            let prefix = spc_crypto::height_push_bytes(152);
            send(
                &mut write,
                template_msg(2, true, subsidy_at(152, 150), &prefix, &[]),
                MESSAGE_TYPE_NEW_TEMPLATE,
            )
            .await;
            send(
                &mut write,
                SetNewPrevHash {
                    template_id: 2,
                    prev_hash: U256::from(&[0x99; 32]),
                    header_timestamp: time + 1,
                    n_bits: bits,
                    target: U256::from(&[0xff; 32]),
                },
                MESSAGE_TYPE_SET_NEW_PREV_HASH,
            )
            .await;
            std::future::pending::<()>().await;
        });

        let source = Sv2TpTemplateSource::spawn(test_config(addr, pk));
        let _ = poll_outcome(&source, |r| r.is_ok()).await;
        let err = crate::template::BlockSubmitter::submit_assembled(&source, &assembled)
            .await
            .expect_err("competing tip");
        assert!(err.to_string().contains("stale-template-id"), "{err}");
        task.abort();
    }
}
