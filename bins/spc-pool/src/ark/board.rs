//! Boarding: refill the float from matured pool outputs (SPEC §11.3
//! "Refilling (H_f + 100)"; decision record 2026-11-02-bark-adapter.md,
//! clause 3).
//!
//! This is a second surface, deliberately not part of [`ArkTransport`]:
//! the board loop runs on its own cadence, reads only the chain and the
//! float, and cannot be driven by order-stream content at all. What it
//! does per matured pool output:
//!
//! 1. re-check the outpoint is still on the active chain (two pool blocks
//!    at the same height share a script but not a txid, SPEC §11.3);
//! 2. claim the one boarding decision for that `(txid, vout)` - boarding
//!    is idempotent per outpoint, across restarts, via a durable
//!    registry;
//! 3. board `float_target_sats - float_balance` (if positive), never
//!    more: the remainder stays on-chain under `ark` as operator surplus;
//! 4. build and sign the funding PSBT with the pool output keychain
//!    (account `m/86'/coin'/1'`, index = block height, key path) and
//!    register it via `board_psbt`.
//!
//! The float counts only spendable VTXOs whose remaining lifetime covers
//! `min_remaining_blocks` (SPEC §11.3's pay window); keeping the float
//! inside that window is the maintenance loop's job (decision record,
//! clause 4), which runs on every tick.
//!
//! Fail-closed posture: an outpoint that left the active chain is dropped
//! (logged), never boarded blind. A generic board error retains its decision:
//! it is not evidence that nothing was submitted. Stock pending checkpoints
//! own recovery; the automatic loop never releases a possibly submitted input
//! merely because an RPC or persistence call returned an error.

use std::str::FromStr;

use bitcoin::bip32::{ChildNumber, Xpriv, Xpub};
use bitcoin::hashes::Hash as _;
use bitcoin::locktime::absolute::LockTime;
use bitcoin::secp256k1::{Secp256k1, XOnlyPublicKey};
use bitcoin::sighash::{Prevouts, SighashCache, TapSighashType};
use bitcoin::{Amount, Network, OutPoint, Psbt, Sequence, Transaction, TxIn, TxOut, Witness};
use secp256k1::Message;

use super::bark_transport::BarkClient;

/// Coinbase maturity: pool outputs board at `H_f + 100` (SPEC §11.3).
pub const MATURITY_BLOCKS: u32 = 100;
/// How many heights below the scan cursor are re-scanned every tick, so a
/// shallow reorg that replaces a scanned coinbase is still seen. Deeper
/// events are operator territory (SPEC §11.3 "Reorg and float edge cases").
pub const REORG_WINDOW: u32 = 12;
/// Below the P2TR dust floor nothing is boarded; the output stays
/// on-chain as surplus.
pub const DUST_SATS: u64 = 546;

/// A pool output recorded from a coinbase: paid to
/// `pool_output_script(H)` at height `H`, value `sats`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PoolOutput {
    pub outpoint: OutPoint,
    pub height: u32,
    pub sats: u64,
}

/// The coinbase of the active-chain block at one height.
#[derive(Clone, Debug, Default)]
pub struct Coinbase {
    pub txid: [u8; 32],
    /// `(script_pubkey, sats)` per output.
    pub outputs: Vec<(Vec<u8>, u64)>,
}

/// The chain slice boarding needs. `Ok(None)` means "height not reached
/// yet"; `Err` holds the tick.
#[async_trait::async_trait]
pub trait BoardChain: Send + Sync {
    async fn tip_height(&self) -> Result<u32, String>;
    /// The coinbase of the active-chain block at `height`.
    async fn coinbase(&self, height: u32) -> Result<Option<Coinbase>, String>;
    /// True when `outpoint` is an unspent output of the active chain.
    async fn outpoint_active(&self, outpoint: OutPoint) -> Result<bool, String>;
}

/// Durable boarding state (Redis in production, in-memory in tests).
/// MUST survive restarts: the claim is what makes boarding idempotent per
/// `(txid, vout)` across a reboot.
#[async_trait::async_trait]
pub trait BoardRegistry: Send + Sync {
    /// The last height whose coinbase was scanned.
    async fn scanned_height(&self) -> Result<Option<u32>, String>;
    async fn set_scanned_height(&self, height: u32) -> Result<(), String>;
    /// Record a pool output found in a coinbase. Idempotent per outpoint
    /// (the same txid can only carry the same height and value).
    async fn record_pool_output(&self, output: PoolOutput) -> Result<(), String>;
    async fn pool_outputs(&self) -> Result<Vec<PoolOutput>, String>;
    /// Drop a recorded output that left the active chain.
    async fn drop_pool_output(&self, outpoint: OutPoint) -> Result<(), String>;
    /// Claim the one boarding decision for `outpoint`: `true` the first
    /// time, `false` once it is decided (boarded or deliberately skipped).
    async fn claim_decision(&self, outpoint: OutPoint) -> Result<bool, String>;
    /// Release only with definite pre-effect evidence. A generic board error
    /// is not such evidence; the automatic loop retains that claim.
    async fn release_decision(&self, outpoint: OutPoint) -> Result<(), String>;
}

/// Tuning for the board loop. Defaults per SPEC §11.3/§11.10.
#[derive(Clone, Copy, Debug)]
pub struct BoardingPolicy {
    pub float_target_sats: u64,
    /// Only VTXOs with at least this many blocks left count as float
    /// (SPEC §11.3 `min_remaining_blocks`, default 3024).
    pub min_remaining_blocks: u32,
    /// Coinbase maturity (default 100, SPEC §11.3).
    pub maturity_blocks: u32,
    /// Heights scanned below the tip at first start (default 200: one
    /// maturity period plus margin).
    pub backfill_blocks: u32,
}

impl BoardingPolicy {
    pub fn new(float_target_sats: u64, min_remaining_blocks: u32) -> Self {
        Self {
            float_target_sats,
            min_remaining_blocks,
            maturity_blocks: MATURITY_BLOCKS,
            backfill_blocks: 2 * MATURITY_BLOCKS,
        }
    }
}

/// The pool output keychain (SPEC §11.3): the `pool_output_account` under
/// the ark seed, a **separate account** from bark's BIP86 account 0, with
/// the derivation index equal to the block height. `front` holds only the
/// account xpub (`pool_payout_descriptor`); this side holds the privkeys
/// and signs key-path spends of matured outputs.
#[derive(Clone)]
pub struct PoolOutputKeychain {
    account: Xpriv,
}

/// Parse `pool_output_account` strictly: `m/86'/coin'/N'`, three hardened
/// children, coin type matching the network (0' mainnet, 1' otherwise).
/// Anything else fails closed.
pub fn parse_pool_account(path: &str, network: Network) -> Result<Vec<ChildNumber>, String> {
    let expected_coin = if network == Network::Bitcoin { 0 } else { 1 };
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() != 4 || parts[0] != "m" {
        return Err(format!(
            "pool_output_account must be m/86'/{expected_coin}'/N' (three hardened children), got {path}"
        ));
    }
    let parsed: Vec<ChildNumber> = parts[1..]
        .iter()
        .map(|p| ChildNumber::from_str(p).map_err(|e| format!("bad derivation step {p}: {e}")))
        .collect::<Result<_, _>>()?;
    if !parsed.iter().all(|c| c.is_hardened()) {
        return Err(format!(
            "pool_output_account steps must be hardened, got {path}"
        ));
    }
    let purpose = ChildNumber::from_hardened_idx(86).expect("static index is hardened");
    let coin = ChildNumber::from_hardened_idx(expected_coin).expect("static index is hardened");
    if parsed[0] != purpose {
        return Err(format!(
            "pool_output_account must start at m/86', got {path}"
        ));
    }
    if parsed[1] != coin {
        return Err(format!(
            "pool_output_account coin type {}' does not match network {network} (expected {expected_coin}')",
            u32::from(parsed[1]) & 0x7FFF_FFFF
        ));
    }
    Ok(parsed)
}

/// Extract the account xpub from `[pool] pool_payout_descriptor`
/// (`tr(<xpub>/0/*)`), for the startup cross-check that the ark seed's
/// pool account is the descriptor's key (decision record, clause 7).
pub fn descriptor_account_xpub(descriptor: &str) -> Result<Xpub, String> {
    let inner = descriptor
        .strip_prefix("tr(")
        .and_then(|d| d.strip_suffix(')'))
        .ok_or_else(|| format!("not a tr(...) descriptor: {descriptor}"))?;
    let first = inner.split('/').next().unwrap_or_default();
    Xpub::from_str(first).map_err(|e| format!("bad account xpub in descriptor: {e}"))
}

impl PoolOutputKeychain {
    pub fn from_seed(
        network: Network,
        seed: &[u8; 64],
        account_path: &str,
    ) -> Result<Self, String> {
        let secp = Secp256k1::new();
        let root = Xpriv::new_master(network, seed).map_err(|e| format!("bad ark seed: {e}"))?;
        let path = parse_pool_account(account_path, network)?;
        let account = root
            .derive_priv(&secp, &path)
            .map_err(|e| format!("bad pool_output_account derivation: {e}"))?;
        Ok(Self { account })
    }

    /// The account xpub `pool_payout_descriptor` commits to.
    pub fn account_xpub(&self) -> Xpub {
        Xpub::from_priv(&Secp256k1::new(), &self.account)
    }

    /// The signing key for the pool output of block `height`:
    /// `<account>/0/<height>`, key path.
    fn output_keypair(&self, height: u32) -> bitcoin::secp256k1::Keypair {
        let secp = Secp256k1::new();
        let path = [
            ChildNumber::from_normal_idx(0).expect("constant is normal"),
            ChildNumber::from_normal_idx(height).expect("height is normal"),
        ];
        self.account
            .derive_priv(&secp, &path)
            .expect("unhardened derivation cannot fail")
            .to_keypair(&secp)
    }

    /// The pool output script for `height`: P2TR of the x-only output
    /// key, matching `pool_payout_descriptor.at_derivation_index(height)`
    /// on the SPc side.
    pub fn script_pubkey(&self, height: u32) -> bitcoin::ScriptBuf {
        let secp = Secp256k1::new();
        let xonly = XOnlyPublicKey::from_keypair(&self.output_keypair(height)).0;
        bitcoin::ScriptBuf::new_p2tr(&secp, xonly, None)
    }

    /// The fee (sats) of a board transaction with `with_change` outputs
    /// spending one pool output, at `feerate_sat_vb`.
    pub fn board_tx_fee(with_change: bool, feerate_sat_vb: u64) -> Result<u64, String> {
        // Transaction shape: 1 P2TR key-path input, 1 or 2 P2TR outputs.
        // Non-witness bytes: 4 (version) + 2 (segwit marker/flag) + 1
        // (input count) + 1 (output count) + 4 (locktime) = 12; input:
        // 36 (outpoint) + 1 (script len) + 4 (sequence) = 41; output:
        // 8 + 1 + 34 = 43. Witness: 1 (stack count) + 1 (item len) + 64
        // (schnorr sig) = 66.
        let outputs = if with_change { 2 } else { 1 };
        let non_witness = 12 + 41 + outputs * 43;
        let weight = bitcoin::Weight::from_wu(non_witness as u64 * 4 + 66);
        let fee = bitcoin::FeeRate::from_sat_per_vb(feerate_sat_vb)
            .ok_or_else(|| "fee rate over 6.8M sat/vb".to_string())?
            .checked_mul_by_weight(weight)
            .ok_or_else(|| "fee overflowed".to_string())?;
        Ok(fee.to_sat())
    }

    /// Build and fully sign the board funding PSBT spending the pool
    /// output of `height` at `outpoint` (value `sats`): the board output
    /// pays `funding_addr` exactly `board_sats`, plus an optional change
    /// output to an `ark`-controlled on-chain address. The fee is the
    /// caller's arithmetic: `sats - board_sats - change_sats`.
    pub fn build_board_psbt(
        &self,
        outpoint: OutPoint,
        height: u32,
        sats: u64,
        funding_addr: &bitcoin::Address,
        board_sats: u64,
        change: Option<(&bitcoin::Address, u64)>,
    ) -> Result<Psbt, String> {
        let mut outputs = vec![TxOut {
            value: Amount::from_sat(board_sats),
            script_pubkey: funding_addr.script_pubkey(),
        }];
        if let Some((addr, change_sats)) = change {
            outputs.push(TxOut {
                value: Amount::from_sat(change_sats),
                script_pubkey: addr.script_pubkey(),
            });
        }
        let tx = Transaction {
            version: bitcoin::transaction::Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: outpoint,
                script_sig: bitcoin::ScriptBuf::new(),
                sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
                witness: Witness::new(),
            }],
            output: outputs,
        };
        let mut psbt = Psbt::from_unsigned_tx(tx).map_err(|e| format!("psbt: {e}"))?;
        psbt.inputs[0].witness_utxo = Some(TxOut {
            value: Amount::from_sat(sats),
            script_pubkey: self.script_pubkey(height),
        });
        self.sign_psbt(&mut psbt, height)?;
        Ok(psbt)
    }

    /// Sign input 0 as a taproot key-path spend with the height's pool
    /// output key (SIGHASH_DEFAULT: no hash byte in the witness). The
    /// script is BIP86 `tr(key)` - no script tree - so the output key
    /// is the *tweaked* key `q = p + H_tag(p)G`; a key-path spend must
    /// therefore sign with the tweaked keypair, never the internal key.
    fn sign_psbt(&self, psbt: &mut Psbt, height: u32) -> Result<(), String> {
        let prevouts = [psbt
            .inputs
            .first()
            .and_then(|i| i.witness_utxo.clone())
            .ok_or_else(|| "psbt input 0 has no witness_utxo".to_string())?];
        let sighash = {
            let mut cache = SighashCache::new(&psbt.unsigned_tx);
            cache
                .taproot_signature_hash(
                    0,
                    &Prevouts::All(&prevouts),
                    None,
                    None,
                    TapSighashType::Default,
                )
                .map_err(|e| format!("taproot sighash: {e}"))?
        };
        let secp = Secp256k1::new();
        let keypair = self.output_keypair(height);
        let tweaked = bitcoin::key::TapTweak::tap_tweak(keypair, &secp, None);
        let sig = secp.sign_schnorr_no_aux_rand(
            &Message::from_digest(sighash.to_byte_array()),
            &tweaked.to_keypair(),
        );
        let mut witness = Witness::new();
        witness.push(sig.serialize());
        // A finalised PSBT is what bark broadcasts itself (bark's
        // `board_psbt` finds the board output by script, not by index).
        psbt.inputs[0].final_script_witness = Some(witness);
        Ok(())
    }
}

/// The board loop: scan coinbases for pool outputs, board shortfalls at
/// maturity. Generic over the chain, the bark client and the registry so
/// every rule is mock-testable; `main.rs` wires the RPC chain and the
/// Redis registry.
pub struct BoardingLoop<C: BoardChain, W: BarkClient, R: BoardRegistry> {
    chain: C,
    client: W,
    registry: R,
    keychain: PoolOutputKeychain,
    policy: BoardingPolicy,
}

impl<C: BoardChain, W: BarkClient, R: BoardRegistry> BoardingLoop<C, W, R> {
    pub fn new(
        chain: C,
        client: W,
        registry: R,
        keychain: PoolOutputKeychain,
        policy: BoardingPolicy,
    ) -> Self {
        Self {
            chain,
            client,
            registry,
            keychain,
            policy,
        }
    }

    /// Run forever on `interval`. A tick error is logged and retried on
    /// the next interval: boarding liveness is operational, never a
    /// silent skip of a decision.
    pub async fn run(&self, interval: std::time::Duration) {
        let mut ticker = tokio::time::interval(interval);
        loop {
            ticker.tick().await;
            if let Err(e) = self.tick().await {
                tracing::error!(error = %e, "ark board tick failed; retrying next interval");
            }
        }
    }

    pub async fn tick(&self) -> Result<(), String> {
        // Float hygiene first (decision record, clause 4): keep every send
        // candidate inside the min_remaining_blocks window.
        if let Err(e) = self.client.maintenance().await {
            tracing::warn!(error = %e, "ark wallet maintenance failed");
        }
        let tip = self.chain.tip_height().await?;
        self.scan(tip, u32::MAX).await?;
        self.board_matured(tip).await
    }

    /// Safe chain observation is independent of new-board admission.
    pub async fn scan_only(&self) -> Result<(), String> {
        self.scan(self.chain.tip_height().await?, 32).await
    }

    pub async fn refill_only(&self) -> Result<(), String> {
        self.board_matured(self.chain.tip_height().await?).await
    }

    /// Scan every unscanned height up to `tip` for a pool output, then
    /// re-scan the reorg window so a replaced coinbase is still seen.
    async fn scan(&self, tip: u32, limit: u32) -> Result<(), String> {
        let cursor = self.registry.scanned_height().await?;
        let start = match cursor {
            Some(h) => h.saturating_add(1),
            // First start: one maturity period plus margin below the tip.
            None => tip.saturating_sub(self.policy.backfill_blocks),
        };
        for height in start..=tip.min(start.saturating_add(limit - 1)) {
            self.scan_height(height).await?;
            self.registry.set_scanned_height(height).await?;
        }
        // Shallow-reorg tolerance: re-check the last scanned heights.
        if let Some(h) = cursor {
            for height in h.saturating_sub(REORG_WINDOW - 1)..=h {
                self.scan_height(height).await?;
            }
        }
        Ok(())
    }

    async fn scan_height(&self, height: u32) -> Result<(), String> {
        let Some(coinbase) = self.chain.coinbase(height).await? else {
            return Ok(());
        };
        let script = self.keychain.script_pubkey(height);
        let txid = bitcoin::Txid::from_byte_array(coinbase.txid);
        for (vout, (script_pubkey, sats)) in coinbase.outputs.iter().enumerate() {
            if script_pubkey.as_slice() == script.as_bytes() {
                let outpoint = OutPoint {
                    txid,
                    vout: vout as u32,
                };
                tracing::info!(
                    txid = %hex::encode(&coinbase.txid[..8]),
                    vout,
                    height,
                    sats,
                    "pool output found in coinbase"
                );
                self.registry
                    .record_pool_output(PoolOutput {
                        outpoint,
                        height,
                        sats: *sats,
                    })
                    .await?;
            }
        }
        Ok(())
    }

    /// Board every matured pool output that needs boarding.
    async fn board_matured(&self, tip: u32) -> Result<(), String> {
        for output in self.registry.pool_outputs().await? {
            if !self.client.board_capacity_available().await? {
                break;
            }
            if tip < output.height.saturating_add(self.policy.maturity_blocks) {
                continue;
            }
            if !self.chain.outpoint_active(output.outpoint).await? {
                tracing::warn!(
                    txid = %hex::encode(&output.outpoint.txid.as_byte_array()[..8]),
                    vout = output.outpoint.vout,
                    "pool output left the active chain; dropping it"
                );
                self.registry.drop_pool_output(output.outpoint).await?;
                continue;
            }
            // Claim before boarding: the claim is the idempotency fence.
            // A duplicate claim means the decision was already made.
            if !self.registry.claim_decision(output.outpoint).await? {
                continue;
            }
            if let Err(e) = self.board_one(&output, tip).await {
                tracing::error!(
                    txid = %hex::encode(&output.outpoint.txid.as_byte_array()[..8]),
                    vout = output.outpoint.vout,
                    error = %e,
                    "boarding outcome unresolved; retaining the outpoint decision"
                );
                return Err(e);
            }
        }
        Ok(())
    }

    /// The one boarding decision for a matured output. Returns Ok(()) for
    /// "boarded" and for "deliberately boarded nothing" alike: both are
    /// decisions, and neither is retried.
    async fn board_one(&self, output: &PoolOutput, tip: u32) -> Result<(), String> {
        let float = self.float_balance(tip).await?;
        let shortfall = self.policy.float_target_sats.saturating_sub(float);
        if shortfall == 0 {
            tracing::info!(
                txid = %hex::encode(&output.outpoint.txid.as_byte_array()[..8]),
                "float at target; nothing boarded (operator surplus stays on-chain)"
            );
            return Ok(());
        }
        let feerate = self.client.feerate_sat_vb().await?;
        // Try with a change output first; fall back to boarding the whole
        // output minus fee when the change would be dust.
        let fee_with_change = PoolOutputKeychain::board_tx_fee(true, feerate)?;
        let available = output.sats.saturating_sub(fee_with_change);
        if available <= DUST_SATS {
            tracing::info!(
                txid = %hex::encode(&output.outpoint.txid.as_byte_array()[..8]),
                sats = output.sats,
                "matured pool output too small to board; leaving it on-chain"
            );
            return Ok(());
        }
        let board_amount = shortfall.min(available);
        let change = available - board_amount;
        let (funding_addr, expiry_height) = self.client.board_funding_address().await?;
        let psbt = if change >= DUST_SATS {
            let change_addr = self.client.onchain_address().await?;
            self.keychain.build_board_psbt(
                output.outpoint,
                output.height,
                output.sats,
                &funding_addr,
                board_amount,
                Some((&change_addr, change)),
            )?
        } else {
            // Dust remainder: fold it into the fee rather than emit a
            // dust output; the board still never exceeds the shortfall
            // by more than that dust.
            self.keychain.build_board_psbt(
                output.outpoint,
                output.height,
                output.sats,
                &funding_addr,
                board_amount,
                None,
            )?
        };
        self.client.board_psbt(&psbt, expiry_height).await?;
        tracing::info!(
            txid = %hex::encode(&output.outpoint.txid.as_byte_array()[..8]),
            vout = output.outpoint.vout,
            board_sats = board_amount,
            change_sats = if change >= DUST_SATS { change } else { 0 },
            "matured pool output boarded"
        );
        Ok(())
    }

    /// The float: spendable VTXOs with at least `min_remaining_blocks`
    /// left (SPEC §11.3's pay window; decision record, clause 4).
    async fn float_balance(&self, tip: u32) -> Result<u64, String> {
        Ok(self
            .client
            .list_vtxos()
            .await?
            .into_iter()
            .filter(|v| {
                v.spendable
                    && v.expiry_height.saturating_sub(tip) >= self.policy.min_remaining_blocks
            })
            .map(|v| v.sats)
            .sum())
    }
}

/// Redis-backed [`BoardRegistry`] on role-local keys (no identity data:
/// outpoints and heights only, SPEC §11.3).
#[derive(Clone)]
pub struct RedisBoardRegistry {
    streams: spc_store::redis::Streams,
}

impl RedisBoardRegistry {
    pub const CURSOR_KEY: &'static str = "spc:ark:board:cursor";
    pub const OUTPUTS_KEY: &'static str = "spc:ark:board:outputs";
    pub const DECIDED_KEY: &'static str = "spc:ark:board:decided";

    pub fn new(streams: spc_store::redis::Streams) -> Self {
        Self { streams }
    }

    fn encode_output(o: &PoolOutput) -> Vec<u8> {
        let mut buf = Vec::with_capacity(48);
        buf.extend_from_slice(o.outpoint.txid.as_byte_array());
        buf.extend_from_slice(&o.outpoint.vout.to_be_bytes());
        buf.extend_from_slice(&o.height.to_be_bytes());
        buf.extend_from_slice(&o.sats.to_be_bytes());
        buf
    }

    fn decode_output(bytes: &[u8]) -> Option<PoolOutput> {
        if bytes.len() != 48 {
            return None;
        }
        let mut txid = [0u8; 32];
        txid.copy_from_slice(&bytes[..32]);
        let vout = u32::from_be_bytes(bytes[32..36].try_into().ok()?);
        let height = u32::from_be_bytes(bytes[36..40].try_into().ok()?);
        let sats = u64::from_be_bytes(bytes[40..48].try_into().ok()?);
        Some(PoolOutput {
            outpoint: OutPoint {
                txid: bitcoin::Txid::from_byte_array(txid),
                vout,
            },
            height,
            sats,
        })
    }

    fn encode_outpoint(o: OutPoint) -> Vec<u8> {
        let mut buf = Vec::with_capacity(36);
        buf.extend_from_slice(o.txid.as_byte_array());
        buf.extend_from_slice(&o.vout.to_be_bytes());
        buf
    }
}

#[async_trait::async_trait]
impl BoardRegistry for RedisBoardRegistry {
    async fn scanned_height(&self) -> Result<Option<u32>, String> {
        let raw = self
            .streams
            .kv_get(Self::CURSOR_KEY)
            .await
            .map_err(|e| e.to_string())?;
        raw.and_then(|v| v.parse().ok())
            .map_or(Ok(None), |h| Ok(Some(h)))
    }

    async fn set_scanned_height(&self, height: u32) -> Result<(), String> {
        self.streams
            .kv_set(Self::CURSOR_KEY, &height.to_string())
            .await
            .map_err(|e| e.to_string())
    }

    async fn record_pool_output(&self, output: PoolOutput) -> Result<(), String> {
        self.streams
            .sadd(Self::OUTPUTS_KEY, &Self::encode_output(&output))
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn pool_outputs(&self) -> Result<Vec<PoolOutput>, String> {
        Ok(self
            .streams
            .smembers(Self::OUTPUTS_KEY)
            .await
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter_map(|bytes| Self::decode_output(&bytes))
            .collect())
    }

    async fn drop_pool_output(&self, outpoint: OutPoint) -> Result<(), String> {
        self.streams
            .srem(Self::OUTPUTS_KEY, &Self::encode_outpoint(outpoint))
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    async fn claim_decision(&self, outpoint: OutPoint) -> Result<bool, String> {
        self.streams
            .sadd(Self::DECIDED_KEY, &Self::encode_outpoint(outpoint))
            .await
            .map_err(|e| e.to_string())
    }

    async fn release_decision(&self, outpoint: OutPoint) -> Result<(), String> {
        self.streams
            .srem(Self::DECIDED_KEY, &Self::encode_outpoint(outpoint))
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

/// bitcoin-core-RPC [`BoardChain`].
#[cfg(feature = "rpc")]
pub struct RpcBoardChain {
    client: bitcoincore_rpc::Client,
    read_deadline: std::sync::Mutex<std::time::Instant>,
}

#[cfg(feature = "rpc")]
impl RpcBoardChain {
    pub fn new(url: &str, user: &str, pass: &str) -> Result<Self, bitcoincore_rpc::Error> {
        let mut http = bitcoincore_rpc::jsonrpc::simple_http::Builder::new()
            .url(url)
            .map_err(|e| bitcoincore_rpc::Error::ReturnedError(e.to_string()))?
            .timeout(std::time::Duration::from_secs(1));
        if !user.is_empty() {
            http = http.auth(user, Some(pass));
        }
        Ok(Self {
            client: bitcoincore_rpc::Client::from_jsonrpc(
                bitcoincore_rpc::jsonrpc::Client::with_transport(http.build()),
            ),
            read_deadline: std::sync::Mutex::new(std::time::Instant::now()),
        })
    }
}

#[cfg(feature = "rpc")]
#[async_trait::async_trait]
impl BoardChain for RpcBoardChain {
    async fn tip_height(&self) -> Result<u32, String> {
        use bitcoincore_rpc::RpcApi;
        *self
            .read_deadline
            .lock()
            .map_err(|_| "chain budget mutex")? =
            std::time::Instant::now() + std::time::Duration::from_secs(5);
        let count = self.client.get_block_count().map_err(|e| e.to_string())?;
        u32::try_from(count).map_err(|_| "tip exceeds u32".to_string())
    }

    async fn coinbase(&self, height: u32) -> Result<Option<Coinbase>, String> {
        use bitcoincore_rpc::RpcApi;
        if std::time::Instant::now()
            >= *self
                .read_deadline
                .lock()
                .map_err(|_| "chain budget mutex")?
        {
            return Err("chain scan budget exhausted".into());
        }
        let hash = match self.client.get_block_hash(u64::from(height)) {
            Ok(hash) => hash,
            // RPC_INVALID_PARAMETER: the chain has not reached `height`.
            Err(bitcoincore_rpc::Error::JsonRpc(bitcoincore_rpc::jsonrpc::error::Error::Rpc(
                e,
            ))) if e.code == -8 => return Ok(None),
            Err(e) => return Err(e.to_string()),
        };
        let block = self
            .client
            .get_block(&hash)
            .map_err(|e| format!("getblock {height}: {e}"))?;
        let Some(coinbase) = block.txdata.first() else {
            return Err(format!("block at {height} has no coinbase"));
        };
        Ok(Some(Coinbase {
            txid: coinbase.compute_txid().to_byte_array(),
            outputs: coinbase
                .output
                .iter()
                .map(|o| (o.script_pubkey.as_bytes().to_vec(), o.value.to_sat()))
                .collect(),
        }))
    }

    async fn outpoint_active(&self, outpoint: OutPoint) -> Result<bool, String> {
        use bitcoincore_rpc::RpcApi;
        if std::time::Instant::now()
            >= *self
                .read_deadline
                .lock()
                .map_err(|_| "chain budget mutex")?
        {
            return Err("chain read budget exhausted".into());
        }
        let txout = self
            .client
            .get_tx_out(&outpoint.txid, outpoint.vout, Some(false))
            .map_err(|e| format!("gettxout {}: {e}", outpoint.txid))?;
        Ok(txout.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::secp256k1::Keypair;
    use bitcoin::Address;
    use secp256k1::{Secp256k1, SecretKey};
    use std::collections::{HashMap, HashSet};
    use std::sync::Arc;

    fn seed() -> [u8; 64] {
        [7; 64]
    }

    fn keychain() -> PoolOutputKeychain {
        PoolOutputKeychain::from_seed(Network::Regtest, &seed(), "m/86'/1'/1'").unwrap()
    }

    #[test]
    fn pool_account_parsing_is_strict() {
        assert!(parse_pool_account("m/86'/1'/1'", Network::Regtest).is_ok());
        assert!(parse_pool_account("m/86'/0'/2'", Network::Bitcoin).is_ok());
        // Fail closed: wrong purpose, wrong coin type for the network,
        // unhardened steps, wrong depth, garbage.
        assert!(parse_pool_account("m/84'/1'/1'", Network::Regtest).is_err());
        assert!(parse_pool_account("m/86'/0'/1'", Network::Regtest).is_err());
        assert!(parse_pool_account("m/86'/1'/1", Network::Regtest).is_err());
        assert!(parse_pool_account("m/86'/1'", Network::Regtest).is_err());
        assert!(parse_pool_account("m/86'/1'/1'/2'", Network::Regtest).is_err());
        assert!(parse_pool_account("", Network::Regtest).is_err());
    }

    /// The keychain's script must equal what the SPc side derives from
    /// the descriptor xpub at the same index, and the account xpub must
    /// round-trip into a descriptor the front would accept.
    #[test]
    fn script_matches_the_descriptor_derivation() {
        let kc = keychain();
        let xpub = kc.account_xpub();
        // Independent derivation straight from the xpub.
        let secp = Secp256k1::new();
        let child = xpub
            .derive_pub(
                &secp,
                &[
                    ChildNumber::from_normal_idx(0).unwrap(),
                    ChildNumber::from_normal_idx(201).unwrap(),
                ],
            )
            .unwrap();
        let xonly = XOnlyPublicKey::from(child.to_pub());
        assert_eq!(
            kc.script_pubkey(201).as_bytes(),
            bitcoin::ScriptBuf::new_p2tr(&secp, xonly, None).as_bytes()
        );
        // The descriptor cross-check finds the same key.
        let descriptor = format!("tr({}/0/*)", xpub);
        assert_eq!(descriptor_account_xpub(&descriptor).unwrap(), xpub);
    }

    /// The board PSBT is signed: the schnorr signature verifies against
    /// the script's *output* key (the BIP86 tweak of the height's key)
    /// over the taproot key-path sighash. Verifying against the internal
    /// key instead would hide an untweaked-signature bug from bitcoind.
    #[test]
    fn board_psbt_is_fully_signed() {
        use secp256k1::VerifyOnly;
        let kc = keychain();
        let (funding, _) = p2tr_addresses();
        let outpoint = OutPoint::new(bitcoin::Txid::from_byte_array([1; 32]), 0);
        let psbt = kc
            .build_board_psbt(outpoint, 201, 1_000_000, &funding, 400_000, None)
            .unwrap();
        // Finalised: bark broadcasts a finalised PSBT itself.
        assert!(psbt.inputs[0].final_script_witness.is_some());
        // Extractable: the fee check needs witness_utxo, and the
        // signature must verify.
        let tx = psbt.clone().extract_tx().unwrap();
        assert_eq!(tx.input.len(), 1);
        assert_eq!(tx.output[0].value.to_sat(), 400_000);
        let secp: Secp256k1<VerifyOnly> = Secp256k1::verification_only();
        let script = kc.script_pubkey(201);
        let prevouts = [TxOut {
            value: Amount::from_sat(1_000_000),
            script_pubkey: script.clone(),
        }];
        let mut cache = SighashCache::new(&tx);
        let sighash = cache
            .taproot_signature_hash(
                0,
                &Prevouts::All(&prevouts),
                None,
                None,
                TapSighashType::Default,
            )
            .unwrap();
        let sig_bytes = tx.input[0].witness.to_vec()[0].clone();
        assert_eq!(sig_bytes.len(), 64, "SIGHASH_DEFAULT has no hash byte");
        let sig = secp256k1::schnorr::Signature::from_slice(&sig_bytes).unwrap();
        // The x-only output key is the 32-byte payload of the P2TR script.
        let xonly = XOnlyPublicKey::from_slice(&script.as_bytes()[2..34]).unwrap();
        assert!(secp
            .verify_schnorr(&sig, &Message::from_digest(sighash.to_byte_array()), &xonly)
            .is_ok());
    }

    fn p2tr_addresses() -> (Address, Address) {
        let secp = Secp256k1::new();
        let a = Keypair::from_secret_key(&secp, &SecretKey::from_slice(&[1; 32]).unwrap());
        let b = Keypair::from_secret_key(&secp, &SecretKey::from_slice(&[2; 32]).unwrap());
        (
            Address::p2tr(
                &secp,
                XOnlyPublicKey::from_keypair(&a).0,
                None,
                Network::Regtest,
            ),
            Address::p2tr(
                &secp,
                XOnlyPublicKey::from_keypair(&b).0,
                None,
                Network::Regtest,
            ),
        )
    }

    // ---- BoardingLoop with mocks ----

    struct MockChain {
        tip: u32,
        blocks: HashMap<u32, Coinbase>,
        inactive: HashSet<OutPoint>,
    }

    #[async_trait::async_trait]
    impl BoardChain for MockChain {
        async fn tip_height(&self) -> Result<u32, String> {
            Ok(self.tip)
        }
        async fn coinbase(&self, height: u32) -> Result<Option<Coinbase>, String> {
            Ok(self.blocks.get(&height).cloned())
        }
        async fn outpoint_active(&self, outpoint: OutPoint) -> Result<bool, String> {
            Ok(!self.inactive.contains(&outpoint))
        }
    }

    /// Clone shares the underlying state: two clones stand in for one
    /// durable registry across a process restart.
    type RegistryState = (Option<u32>, Vec<PoolOutput>, HashSet<OutPoint>);

    #[derive(Clone, Default)]
    struct InMemoryRegistry {
        state: Arc<std::sync::Mutex<RegistryState>>,
    }

    #[async_trait::async_trait]
    impl BoardRegistry for InMemoryRegistry {
        async fn scanned_height(&self) -> Result<Option<u32>, String> {
            Ok(self.state.lock().unwrap().0)
        }
        async fn set_scanned_height(&self, height: u32) -> Result<(), String> {
            self.state.lock().unwrap().0 = Some(height);
            Ok(())
        }
        async fn record_pool_output(&self, output: PoolOutput) -> Result<(), String> {
            let mut s = self.state.lock().unwrap();
            if !s.1.iter().any(|o| o.outpoint == output.outpoint) {
                s.1.push(output);
            }
            Ok(())
        }
        async fn pool_outputs(&self) -> Result<Vec<PoolOutput>, String> {
            Ok(self.state.lock().unwrap().1.clone())
        }
        async fn drop_pool_output(&self, outpoint: OutPoint) -> Result<(), String> {
            self.state
                .lock()
                .unwrap()
                .1
                .retain(|o| o.outpoint != outpoint);
            Ok(())
        }
        async fn claim_decision(&self, outpoint: OutPoint) -> Result<bool, String> {
            Ok(self.state.lock().unwrap().2.insert(outpoint))
        }
        async fn release_decision(&self, outpoint: OutPoint) -> Result<(), String> {
            self.state.lock().unwrap().2.remove(&outpoint);
            Ok(())
        }
    }

    #[derive(Clone)]
    struct MockWallet {
        vtxos: Vec<super::super::bark_transport::TrackedVtxo>,
        boards: Arc<std::sync::Mutex<Vec<(Psbt, u32)>>>,
        funding: Address,
        change: Address,
    }

    #[async_trait::async_trait]
    impl BarkClient for MockWallet {
        async fn list_vtxos(
            &self,
        ) -> Result<Vec<super::super::bark_transport::TrackedVtxo>, String> {
            Ok(self.vtxos.clone())
        }
        async fn send_arkoor_payment(
            &self,
            _: &super::super::bark_transport::ArkPayment,
        ) -> Result<Vec<super::super::bark_transport::TrackedVtxo>, String> {
            unimplemented!("the send path is exercised in bark_transport.rs")
        }
        async fn board_funding_address(&self) -> Result<(Address, u32), String> {
            Ok((self.funding.clone(), 950_000))
        }
        async fn board_psbt(&self, psbt: &Psbt, expiry_height: u32) -> Result<(), String> {
            self.boards
                .lock()
                .unwrap()
                .push((psbt.clone(), expiry_height));
            Ok(())
        }
        async fn onchain_address(&self) -> Result<Address, String> {
            Ok(self.change.clone())
        }
        async fn maintenance(&self) -> Result<(), String> {
            Ok(())
        }
        async fn feerate_sat_vb(&self) -> Result<u64, String> {
            Ok(1)
        }
    }

    fn mock_vtxo(sats: u64, expiry: u32) -> super::super::bark_transport::TrackedVtxo {
        super::super::bark_transport::TrackedVtxo {
            proof: vec![],
            id: [3; 36],
            sats,
            expiry_height: expiry,
            user_pubkey: [9; 33],
            spendable: true,
        }
    }

    fn setup() -> (PoolOutputKeychain, PoolOutput, Coinbase) {
        let kc = keychain();
        let txid = [0xAB; 32];
        let script = kc.script_pubkey(100);
        let coinbase = Coinbase {
            txid,
            outputs: vec![
                (vec![0x51, 0x00], 1_000), // carrier noise
                (script.as_bytes().to_vec(), 1_000_000),
            ],
        };
        let output = PoolOutput {
            outpoint: OutPoint {
                txid: bitcoin::Txid::from_byte_array(txid),
                vout: 1,
            },
            height: 100,
            sats: 1_000_000,
        };
        (kc, output, coinbase)
    }

    fn make_loop(
        chain: MockChain,
        wallet: MockWallet,
        registry: InMemoryRegistry,
        kc: PoolOutputKeychain,
        policy: BoardingPolicy,
    ) -> BoardingLoop<MockChain, MockWallet, InMemoryRegistry> {
        BoardingLoop::new(chain, wallet, registry, kc, policy)
    }

    /// A matured output boards exactly the shortfall; the rest is change
    /// to an ark-controlled on-chain address (SPEC §11.3 "Refilling").
    #[tokio::test]
    async fn matured_output_boards_only_the_shortfall() {
        let (kc, output, coinbase) = setup();
        let mut blocks = HashMap::new();
        blocks.insert(100u32, coinbase);
        let (funding, change_addr) = p2tr_addresses();
        let wallet = MockWallet {
            vtxos: vec![mock_vtxo(100_000, 900_000)],
            boards: Default::default(),
            funding: funding.clone(),
            change: change_addr.clone(),
        };
        let policy = BoardingPolicy::new(500_000, 3024);
        let registry = InMemoryRegistry::default();
        let lp = make_loop(
            MockChain {
                tip: 200,
                blocks,
                inactive: HashSet::new(),
            },
            wallet,
            registry,
            kc,
            policy,
        );
        lp.tick().await.unwrap();

        let boards = lp.client.boards.lock().unwrap();
        assert_eq!(boards.len(), 1, "exactly one board");
        let (psbt, expiry) = &boards[0];
        assert_eq!(*expiry, 950_000);
        let tx = &psbt.unsigned_tx;
        assert_eq!(tx.input[0].previous_output, output.outpoint);
        assert_eq!(tx.output.len(), 2);
        assert_eq!(tx.output[0].value.to_sat(), 400_000, "shortfall only");
        assert_eq!(tx.output[0].script_pubkey, funding.script_pubkey());
        let fee = 1_000_000 - 400_000 - tx.output[1].value.to_sat();
        assert_eq!(fee, PoolOutputKeychain::board_tx_fee(true, 1).unwrap());
        assert_eq!(tx.output[1].script_pubkey, change_addr.script_pubkey());
    }

    /// SPEC §11.9: with the float already at target nothing is boarded,
    /// and the decision is made once - a later tick boards nothing new.
    #[tokio::test]
    async fn float_at_target_boards_nothing() {
        let (kc, _, coinbase) = setup();
        let mut blocks = HashMap::new();
        blocks.insert(100u32, coinbase);
        let (funding, change_addr) = p2tr_addresses();
        let wallet = MockWallet {
            vtxos: vec![mock_vtxo(500_000, 900_000)],
            boards: Default::default(),
            funding,
            change: change_addr,
        };
        let lp = make_loop(
            MockChain {
                tip: 200,
                blocks,
                inactive: HashSet::new(),
            },
            wallet,
            InMemoryRegistry::default(),
            kc,
            BoardingPolicy::new(500_000, 3024),
        );
        lp.tick().await.unwrap();
        lp.tick().await.unwrap();
        assert!(
            lp.client.boards.lock().unwrap().is_empty(),
            "float at target: nothing boarded"
        );
    }

    /// SPEC §11.9: boarding is idempotent per (txid, vout) - across
    /// ticks *and* across a restart (a fresh loop over the same durable
    /// registry, a fresh tip, a fresh wallet recorder).
    #[tokio::test]
    async fn boarding_is_idempotent_per_outpoint_across_restarts() {
        let (kc, _, coinbase) = setup();
        let mut blocks = HashMap::new();
        blocks.insert(100u32, coinbase);
        let (funding, change_addr) = p2tr_addresses();
        let registry = InMemoryRegistry::default();
        {
            let wallet = MockWallet {
                vtxos: vec![],
                boards: Default::default(),
                funding: funding.clone(),
                change: change_addr.clone(),
            };
            let lp = make_loop(
                MockChain {
                    tip: 200,
                    blocks: blocks.clone(),
                    inactive: HashSet::new(),
                },
                wallet,
                registry.clone(),
                kc.clone(),
                BoardingPolicy::new(500_000, 3024),
            );
            lp.tick().await.unwrap();
            assert_eq!(lp.client.boards.lock().unwrap().len(), 1);
        }
        // "Restart": the same durable registry, a new process. The
        // already-decided outpoint is never boarded again, even though
        // the float is still empty and the tip has advanced.
        let wallet = MockWallet {
            vtxos: vec![],
            boards: Default::default(),
            funding,
            change: change_addr,
        };
        let lp = make_loop(
            MockChain {
                tip: 250,
                blocks,
                inactive: HashSet::new(),
            },
            wallet,
            registry,
            kc,
            BoardingPolicy::new(500_000, 3024),
        );
        lp.tick().await.unwrap();
        assert!(
            lp.client.boards.lock().unwrap().is_empty(),
            "a restart must not re-board a decided outpoint"
        );
    }

    /// `min_remaining_blocks`: VTXOs too close to expiry do not count as
    /// float, so the shortfall (and the board) is larger.
    #[tokio::test]
    async fn expiring_vtxos_do_not_count_as_float() {
        let (kc, _, coinbase) = setup();
        let mut blocks = HashMap::new();
        blocks.insert(100u32, coinbase);
        let (funding, change_addr) = p2tr_addresses();
        // 400_000 sats of float, but expiring in 1000 blocks < 3024.
        let wallet = MockWallet {
            vtxos: vec![mock_vtxo(400_000, 1200)],
            boards: Default::default(),
            funding,
            change: change_addr,
        };
        let lp = make_loop(
            MockChain {
                tip: 200,
                blocks,
                inactive: HashSet::new(),
            },
            wallet,
            InMemoryRegistry::default(),
            kc,
            BoardingPolicy::new(500_000, 3024),
        );
        lp.tick().await.unwrap();
        let boards = lp.client.boards.lock().unwrap();
        assert_eq!(boards.len(), 1);
        assert_eq!(
            boards[0].0.unsigned_tx.output[0].value.to_sat(),
            500_000,
            "full target boarded: the expiring float does not count"
        );
    }

    /// SPEC §11.9 / §11.3: an output that left the active chain (reorg
    /// or spend) is dropped, never boarded.
    #[tokio::test]
    async fn an_output_that_left_the_chain_is_dropped() {
        let (kc, output, coinbase) = setup();
        let mut blocks = HashMap::new();
        blocks.insert(100u32, coinbase);
        let (funding, change_addr) = p2tr_addresses();
        let registry = InMemoryRegistry::default();
        let lp = make_loop(
            MockChain {
                tip: 200,
                blocks,
                inactive: HashSet::from([output.outpoint]),
            },
            MockWallet {
                vtxos: vec![],
                boards: Default::default(),
                funding,
                change: change_addr,
            },
            registry,
            kc,
            BoardingPolicy::new(500_000, 3024),
        );
        lp.tick().await.unwrap();
        assert!(lp.client.boards.lock().unwrap().is_empty());
        assert!(
            lp.registry.state.lock().unwrap().1.is_empty(),
            "the dropped output is not tracked anymore"
        );
    }

    #[tokio::test]
    async fn pending_stock_board_does_not_stop_bounded_pool_output_scanning() {
        struct PendingWallet;
        #[async_trait::async_trait]
        impl BarkClient for PendingWallet {
            async fn board_capacity_available(&self) -> Result<bool, String> {
                Ok(false)
            }
            async fn list_vtxos(
                &self,
            ) -> Result<Vec<super::super::bark_transport::TrackedVtxo>, String> {
                Ok(vec![])
            }
            async fn send_arkoor_payment(
                &self,
                _: &super::super::bark_transport::ArkPayment,
            ) -> Result<Vec<super::super::bark_transport::TrackedVtxo>, String> {
                Err("unused".into())
            }
            async fn board_funding_address(&self) -> Result<(Address, u32), String> {
                panic!("pending capacity must prevent a new board")
            }
            async fn board_psbt(&self, _: &Psbt, _: u32) -> Result<(), String> {
                panic!("pending capacity must prevent a new board")
            }
            async fn onchain_address(&self) -> Result<Address, String> {
                Err("unused".into())
            }
            async fn maintenance(&self) -> Result<(), String> {
                Ok(())
            }
            async fn feerate_sat_vb(&self) -> Result<u64, String> {
                Ok(1)
            }
        }
        let (kc, output, coinbase) = setup();
        let mut blocks = HashMap::new();
        blocks.insert(100u32, coinbase);
        let lp = BoardingLoop::new(
            MockChain {
                tip: 200,
                blocks,
                inactive: HashSet::new(),
            },
            PendingWallet,
            InMemoryRegistry::default(),
            kc,
            BoardingPolicy::new(500_000, 3024),
        );
        lp.scan_only().await.unwrap();
        assert_eq!(lp.registry.scanned_height().await.unwrap(), Some(31));
        for _ in 0..5 {
            lp.scan_only().await.unwrap();
            lp.refill_only().await.unwrap();
        }
        assert_eq!(lp.registry.scanned_height().await.unwrap(), Some(191));
        assert!(lp
            .registry
            .pool_outputs()
            .await
            .unwrap()
            .iter()
            .any(|o| o.outpoint == output.outpoint));
        assert!(
            lp.registry.claim_decision(output.outpoint).await.unwrap(),
            "pending board prevented every new claim"
        );
    }

    /// A generic board failure cannot release a possibly submitted input.
    #[tokio::test]
    async fn a_failed_board_retains_the_decision() {
        let (kc, output, coinbase) = setup();
        let mut blocks = HashMap::new();
        blocks.insert(100u32, coinbase);
        let (funding, change_addr) = p2tr_addresses();

        #[derive(Clone)]
        struct FailingWallet {
            inner: MockWallet,
        }
        #[async_trait::async_trait]
        impl BarkClient for FailingWallet {
            async fn list_vtxos(
                &self,
            ) -> Result<Vec<super::super::bark_transport::TrackedVtxo>, String> {
                self.inner.list_vtxos().await
            }
            async fn send_arkoor_payment(
                &self,
                p: &super::super::bark_transport::ArkPayment,
            ) -> Result<Vec<super::super::bark_transport::TrackedVtxo>, String> {
                self.inner.send_arkoor_payment(p).await
            }
            async fn board_funding_address(&self) -> Result<(Address, u32), String> {
                self.inner.board_funding_address().await
            }
            async fn board_psbt(&self, _psbt: &Psbt, _e: u32) -> Result<(), String> {
                Err("server down".to_string())
            }
            async fn onchain_address(&self) -> Result<Address, String> {
                self.inner.onchain_address().await
            }
            async fn maintenance(&self) -> Result<(), String> {
                Ok(())
            }
            async fn feerate_sat_vb(&self) -> Result<u64, String> {
                Ok(1)
            }
        }

        let registry = InMemoryRegistry::default();
        let lp = BoardingLoop::new(
            MockChain {
                tip: 200,
                blocks,
                inactive: HashSet::new(),
            },
            FailingWallet {
                inner: MockWallet {
                    vtxos: vec![],
                    boards: Default::default(),
                    funding,
                    change: change_addr,
                },
            },
            registry,
            kc,
            BoardingPolicy::new(500_000, 3024),
        );
        assert!(lp.tick().await.is_err());
        // No generic error can authorize another attempt with this input.
        assert!(
            !lp.registry.claim_decision(output.outpoint).await.unwrap(),
            "an ambiguous board retains its decision"
        );
    }
}
