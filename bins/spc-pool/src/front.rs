//! Front-role state: one `a_send` per height, the open slice, the current
//! distribution and the job the miners are hashing (SPEC §2.2, §3.3, §4.7).

use std::collections::BTreeMap;
use std::sync::Arc;

use bitcoin::hashes::Hash;
use bitcoin::pow::{CompactTarget, Target};
use bitcoin::BlockHash;
use spc_accounting::{
    build_distribution, raw_payouts, route_tiers, select_window, ArkCapability, Distribution,
    MinerKeys, Routing, ShareRec, Slice, SliceAgg, SpId, TierConfig,
};
use spc_crypto::{Network, SendKey};
use spc_store::wire::SliceEvent;
use thiserror::Error;

use crate::block::{assemble, AssembledBlock};
use crate::coinbase::{build_pool_template, CoinbaseTemplate};
use crate::sv1::{Job, Share};
use crate::template::{Template, TemplateError, TemplateSource};

#[derive(Debug, Error)]
pub enum FrontError {
    #[error("template: {0}")]
    Template(#[from] TemplateError),
    #[error("accounting: {0}")]
    Accounting(#[from] spc_accounting::Error),
    #[error("crypto: {0}")]
    Crypto(#[from] spc_crypto::Error),
    #[error("no active template")]
    NoTemplate,
    #[error("no active slice")]
    NoSlice,
    #[error("no send key")]
    NoSendKey,
}

#[derive(Clone, Debug)]
pub struct FrontConfig {
    pub network: Network,
    pub fee_bps: u16,
    pub window_n: u32,
    pub slice_fee_delta_sats: u64,
    pub max_slice_shares: usize,
    pub dust_limit_sats: u64,
    pub coinbase_threshold_sats: u64,
    pub max_payout_slots: usize,
    pub budget_wu: u32,
    pub overhead_wu: u32,
    /// P2TR script of the pool output (43 bytes). The production source is
    /// `pool_payout_descriptor`; the dev front takes the derived script.
    pub pool_payout_script: Vec<u8>,
    pub extranonce_len: usize,
    pub front_id: u8,
    pub keep_slices: usize,
}

pub struct Front {
    cfg: FrontConfig,
    source: Arc<dyn TemplateSource>,
    send: Option<SendKey>,
    height: u32,
    template: Option<Template>,
    job: Option<Job>,
    distribution: Option<Distribution>,
    open: Option<Slice>,
    slices: Vec<SliceAgg>,
    slice_seq: u64,
    miners: BTreeMap<SpId, MinerKeys>,
    pending: BTreeMap<SpId, u64>,
    ark: BTreeMap<SpId, ArkCapability>,
    last_block: Option<AssembledBlock>,
}

impl FrontConfig {
    pub fn from_config(c: &crate::config::Config) -> Result<Self, String> {
        let network = c.network().map_err(|e| e.to_string())?;
        let script = match &c.pool.pool_payout_script {
            Some(h) => hex::decode(h).map_err(|e| format!("pool_payout_script: {e}"))?,
            None => crate::coinbase::p2tr_script(&[0u8; 32]).to_bytes().to_vec(),
        };
        Ok(Self {
            network,
            fee_bps: c.pool.fee_bps,
            window_n: c.accounting.window_n,
            slice_fee_delta_sats: c.accounting.slice_fee_delta_sats,
            max_slice_shares: c.accounting.max_slice_shares,
            dust_limit_sats: c.accounting.dust_limit_sats,
            coinbase_threshold_sats: c.accounting.coinbase_threshold_sats,
            max_payout_slots: c.accounting.max_payout_slots,
            budget_wu: c.budget.floor_wu,
            overhead_wu: 0,
            pool_payout_script: script,
            extranonce_len: c.sv1.extranonce1_bytes + c.sv1.extranonce2_bytes,
            front_id: c.pool.front_id,
            keep_slices: c.accounting.retain_windows as usize,
        })
    }
}

impl Front {
    pub fn new(cfg: FrontConfig, source: Arc<dyn TemplateSource>) -> Self {
        Self {
            cfg,
            source,
            send: None,
            height: 0,
            template: None,
            job: None,
            distribution: None,
            open: None,
            slices: Vec::new(),
            slice_seq: 0,
            miners: BTreeMap::new(),
            pending: BTreeMap::new(),
            ark: BTreeMap::new(),
            last_block: None,
        }
    }

    pub fn job(&self) -> Option<&Job> {
        self.job.as_ref()
    }

    pub fn distribution(&self) -> Option<&Distribution> {
        self.distribution.as_ref()
    }

    pub fn last_block(&self) -> Option<&AssembledBlock> {
        self.last_block.as_ref()
    }

    pub fn pending(&self) -> &BTreeMap<SpId, u64> {
        &self.pending
    }

    pub fn set_ark(&mut self, sp_id: SpId, cap: ArkCapability) {
        self.ark.insert(sp_id, cap);
    }

    /// Fetch a template and build the job miners will hash.
    pub async fn refresh(&mut self) -> Result<Job, FrontError> {
        let template = self.source.get_template().await?;
        if self.send.is_none() || self.height != template.height {
            self.roll_slice();
            self.send = Some(SendKey::generate(template.height)?);
            self.height = template.height;
            self.open = Some(Slice::new(
                self.slice_seq,
                self.height,
                template.prev_hash,
                self.cfg.front_id,
                0,
                0,
            ));
        }

        let difficulty = difficulty_from_bits(template.bits);
        let window = select_window(
            &self.slices,
            u64::MAX,
            (self.cfg.window_n as u128).saturating_mul(difficulty),
            BTreeMap::new(),
        );
        let raw = if window.t == 0 {
            spc_accounting::RawPayouts {
                raw: BTreeMap::new(),
                pool_sats: template.coinbase_value,
                r_net: template.subsidy,
                f_net: template.fees,
            }
        } else {
            raw_payouts(&window, template.subsidy, template.fees, self.cfg.fee_bps)?
        };
        let tier_cfg = TierConfig {
            fee_bps: self.cfg.fee_bps,
            dust_limit_sats: self.cfg.dust_limit_sats,
            coinbase_threshold_sats: self.cfg.coinbase_threshold_sats,
            max_payout_slots: self.cfg.max_payout_slots,
            budget_wu: self.cfg.budget_wu,
            overhead_wu: self.cfg.overhead_wu,
        };
        let routing = route_tiers(
            &raw,
            &self.pending,
            &self.ark,
            &tier_cfg,
            template.coinbase_value,
        )?;

        let send = self.send.as_ref().ok_or(FrontError::NoSendKey)?;
        let distribution = build_distribution(
            template.height,
            send,
            &routing,
            &self.miners,
            &self.cfg.pool_payout_script,
            &BTreeMap::new(),
        )?;

        let outputs: Vec<(bitcoin::script::ScriptBuf, u64)> = distribution
            .outputs
            .iter()
            .map(|(s, v)| (bitcoin::script::ScriptBuf::from_bytes(s.clone()), *v))
            .collect();
        let coinbase = build_pool_template(
            template.version,
            template.height,
            send.x_only(),
            self.cfg.extranonce_len,
            &outputs,
            0,
        );

        let job = Job {
            job_id: format!("{:08x}", template.height),
            prev_hash: template.prev_hash,
            height: template.height,
            version: template.version,
            nbits: template.bits,
            ntime: template.curtime,
            coinb1: coinbase.prefix,
            coinb2: coinbase.suffix,
            merkle_branches: template.merkle_branches.clone(),
            clean_jobs: true,
        };
        self.job = Some(job.clone());
        self.distribution = Some(distribution);
        self.template = Some(template);
        Ok(job)
    }

    /// Forward a validated share into the open slice and, if it meets the
    /// network target, assemble the block for submission.
    pub fn on_share(&mut self, share: Share) -> Result<Option<AssembledBlock>, FrontError> {
        let template = self
            .template
            .as_ref()
            .ok_or(FrontError::NoTemplate)?
            .clone();
        self.miners.insert(
            share.sp_id,
            MinerKeys {
                sp_id: share.sp_id,
                b_scan: bitcoin::secp256k1::PublicKey::from_slice(&share.b_scan)
                    .map_err(|_| FrontError::NoSendKey)?,
                b_spend: bitcoin::secp256k1::PublicKey::from_slice(&share.b_spend)
                    .map_err(|_| FrontError::NoSendKey)?,
            },
        );

        let rec = ShareRec {
            sp_id: share.sp_id,
            d: 1,
            job_fees: share.job_fees,
            job_id: parse_job_id(&share.job_id),
            nonce: share.nonce,
            ntime: share.ntime,
            version: share.version,
            extranonce: share.extranonce.clone(),
            share_index: share.share_index,
        };
        let slice = self.open.as_mut().ok_or(FrontError::NoSlice)?;
        match slice.seal_if_needed(
            &rec,
            self.cfg.slice_fee_delta_sats,
            self.cfg.max_slice_shares,
        ) {
            spc_accounting::Seal::Seal(_) => {
                self.roll_slice();
                let slice = self.open.as_mut().ok_or(FrontError::NoSlice)?;
                slice.push(rec);
            }
            spc_accounting::Seal::Keep => slice.push(rec),
        }

        let target = Target::from_compact(CompactTarget::from_consensus(template.bits));
        let hash = BlockHash::from_byte_array(share.share_hash);
        if !target.is_met_by(hash) {
            return Ok(None);
        }

        let block = assemble(
            template.version,
            template.prev_hash,
            share.ntime,
            template.bits,
            share.nonce,
            &share.coinbase,
            &template.transactions,
        );
        self.last_block = Some(block.clone());
        Ok(Some(block))
    }

    /// Sealed slices, newest first, for the window and type-32 service.
    pub fn sealed_slices(&self) -> &[SliceAgg] {
        &self.slices
    }

    /// Encode the open slice for `spc:slices`; call after sealing.
    pub fn take_sealed_event(&mut self) -> Option<SliceEvent> {
        let agg = self.slices.first()?;
        Some(SliceEvent {
            slice_id: agg.slice_id,
            height: agg.height,
            prev_hash: agg.prev_hash,
            ref_job_id: 0,
            ref_fees: 0,
            n_shares: 0,
            difficulty: agg.difficulty,
            root: agg.root,
            shares_blob: Vec::new(),
            credits: agg
                .per_id
                .iter()
                .map(|(id, (d, df))| spc_store::wire::SliceCredit {
                    sp_id: *id,
                    diff_sum: *d,
                    diff_fee_sum: *df,
                })
                .collect(),
        })
    }

    fn roll_slice(&mut self) {
        if let Some(mut open) = self.open.take() {
            if !open.shares.is_empty() {
                open.seal();
                self.slices.insert(0, open.agg());
                self.slices.truncate(self.cfg.keep_slices.max(1));
            }
        }
        if self.height != 0 {
            self.slice_seq = self.slice_seq.wrapping_add(1);
            self.open = Some(Slice::new(
                self.slice_seq,
                self.height,
                self.job.as_ref().map(|j| j.prev_hash).unwrap_or([0u8; 32]),
                self.cfg.front_id,
                0,
                0,
            ));
        }
    }

    pub fn coinbase_template(&self) -> Option<CoinbaseTemplate> {
        let _ = &self.template;
        None
    }

    /// Route helper for callers that need the canonical values.
    pub fn route_for_test(&self, raw: &spc_accounting::RawPayouts, total: u64) -> Routing {
        let tier = TierConfig {
            fee_bps: self.cfg.fee_bps,
            dust_limit_sats: self.cfg.dust_limit_sats,
            coinbase_threshold_sats: self.cfg.coinbase_threshold_sats,
            max_payout_slots: self.cfg.max_payout_slots,
            budget_wu: self.cfg.budget_wu,
            overhead_wu: self.cfg.overhead_wu,
        };
        route_tiers(raw, &self.pending, &self.ark, &tier, total).expect("valid tier config")
    }
}

fn parse_job_id(s: &str) -> u64 {
    u64::from_str_radix(s, 16).unwrap_or(0)
}

pub fn difficulty_from_bits(bits: u32) -> u128 {
    let target = Target::from_compact(CompactTarget::from_consensus(bits));
    let d = target.difficulty_float();
    if d < 1.0 {
        1
    } else if d > u128::MAX as f64 {
        u128::MAX
    } else {
        d as u128
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::template::MockTemplateSource;
    use spc_crypto::SendKey;

    fn template() -> Template {
        Template {
            height: 800_000,
            prev_hash: [0x11; 32],
            version: 0x2000_0000,
            bits: 0x207f_ffff, // easy regtest target
            curtime: 1_700_000_000,
            subsidy: 312_500_000,
            fees: 0,
            coinbase_value: 312_500_000,
            transactions: vec![],
            merkle_branches: vec![],
            coinbase_output_max_additional_size: 50_000,
        }
    }

    fn cfg() -> FrontConfig {
        FrontConfig {
            network: Network::Regtest,
            fee_bps: 100,
            window_n: 8,
            slice_fee_delta_sats: 0,
            max_slice_shares: 65_000,
            dust_limit_sats: 330,
            coinbase_threshold_sats: 330,
            max_payout_slots: 256,
            budget_wu: 100_000,
            overhead_wu: 0,
            pool_payout_script: crate::coinbase::p2tr_script(&[0x99; 32])
                .to_bytes()
                .to_vec(),
            extranonce_len: 8,
            front_id: 0,
            keep_slices: 16,
        }
    }

    #[tokio::test]
    async fn refresh_builds_a_job_with_the_carrier() {
        let source = Arc::new(MockTemplateSource::new(template()));
        let mut front = Front::new(cfg(), source);
        let job = front.refresh().await.unwrap();
        assert_eq!(job.height, 800_000);
        // coinb1 ends with the 32-byte key; coinb2 starts with the sequence.
        assert!(job.coinb1.len() > 33);
        assert!(job.coinb2.len() >= 4);
        let d = front.distribution().unwrap();
        assert_eq!(d.height, 800_000);
        assert_eq!(d.outputs.len(), 1); // pool only before any shares
    }

    #[tokio::test]
    async fn a_share_rolls_into_the_slice_and_can_find_a_block() {
        let source = Arc::new(MockTemplateSource::new(template()));
        let mut front = Front::new(cfg(), source.clone());
        front.refresh().await.unwrap();

        // A valid identity derived from a scan/spend key pair.
        let secp = bitcoin::secp256k1::Secp256k1::new();
        let scan_sk = bitcoin::secp256k1::SecretKey::from_slice(&[0x11; 32]).unwrap();
        let spend_sk = bitcoin::secp256k1::SecretKey::from_slice(&[0x12; 32]).unwrap();
        let scan = bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &scan_sk);
        let spend = bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &spend_sk);
        let id = spc_crypto::sp_id(&scan, &spend);
        let send = SendKey::generate(800_000).unwrap();
        let _ = send;

        let share = crate::sv1::Share {
            sp_id: id,
            b_scan: scan.serialize(),
            b_spend: spend.serialize(),
            job_id: "000c3500".into(),
            nonce: 1,
            ntime: 1_700_000_000,
            version: 0x2000_0000,
            extranonce: vec![0xaa; 8],
            share_index: 0,
            job_fees: 0,
            coinbase: front.job().unwrap().coinb1.clone(),
            merkle_root: [0u8; 32],
            share_hash: [0u8; 32], // easy target is met
        };
        let block = front.on_share(share).unwrap().expect("block");
        assert_eq!(block.header.len(), 80);
        assert_eq!(front.sealed_slices().len(), 0);
    }
}
