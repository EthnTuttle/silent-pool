//! Public, immutable proof payloads. No wallet keys, signer, RPC, or ledger.
use anyhow::{ensure, Context};
use ark::ProtocolEncoding;
use bitcoin::hashes::{sha256, Hash};
use serde::{Deserialize, Serialize};

use super::wallet_control::{Action, BoardFacts, Request, ResultDto, State};

pub const PROOF_SCHEMA: &str = "spc.lab.ap5.pool-wallet-board-proof/v1";
pub const ENVELOPE_SCHEMA: &str = "spc.lab.ap5.pool-wallet-board-proof-envelope/v1";
pub const ROUND_SCHEMA: &str = "spc.lab.ap5.pool-wallet-round-import/v1";

/// JSON is canonical in this declared struct field order, compact UTF-8,
/// with integer money and no optional/reordered object-map fields.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OriginalBoardProof {
    pub schema: String,
    pub original_board_operation_id: String,
    pub board_request: Request,
    pub board_request_sha256: String,
    pub original_vtxo_id: String,
    pub original_full_vtxo_hex: String,
    pub original_full_vtxo_sha256: String,
    pub vtxo_encoding: String,
    pub raw_anchor_tx_hex: String,
    pub anchor_txid: String,
    pub anchor_vout: u32,
    pub owner_pubkey: String,
    pub owner_policy_hex: String,
    pub owner_policy_kind: String,
    pub server_pubkey: String,
    pub board: BoardFacts,
    pub checked_at_unix_ms: u64,
    pub checked_tip_height: u32,
    pub funding_confirmations: u64,
    pub stock_checkpoint_ref: String,
    pub terminal_receipt_context_schema: String,
    pub terminal_receipt_context_canonical_json_hex: String,
    pub terminal_receipt_context_sha256: String,
}

pub(super) fn digest(bytes: &[u8]) -> String {
    sha256::Hash::hash(bytes).to_string()
}

/// The terminal receipt with only its self-referential proof pointer omitted.
/// All amounts, operation id and observation context remain byte-bound.
pub(super) fn terminal_context_bytes(result: &ResultDto) -> anyhow::Result<Vec<u8>> {
    let mut context = result.clone();
    context.retained_proof_sha256 = None;
    ensure!(
        context.proof.is_none() && context.state == State::Confirmed,
        "not a board terminal context"
    );
    Ok(serde_json::to_vec(&context)?)
}

impl OriginalBoardProof {
    pub(super) fn create(
        request: &Request,
        result: &ResultDto,
        raw_anchor_tx_hex: String,
        full_vtxo_hex: String,
        checked_at_unix_ms: u64,
        checked_tip_height: u32,
        funding_confirmations: u64,
    ) -> anyhow::Result<Self> {
        let vtxo = ark::Vtxo::<ark::vtxo::Full>::deserialize(&hex::decode(&full_vtxo_hex)?)?;
        let receipt = terminal_context_bytes(result)?;
        let anchor = vtxo.chain_anchor();
        let proof = Self {
            schema: PROOF_SCHEMA.into(),
            original_board_operation_id: request.operation_id.clone(),
            board_request: request.clone(),
            board_request_sha256: digest(&serde_json::to_vec(request)?),
            original_vtxo_id: vtxo.id().to_string(),
            original_full_vtxo_sha256: digest(&hex::decode(&full_vtxo_hex)?),
            original_full_vtxo_hex: full_vtxo_hex,
            vtxo_encoding: "ark-lib/ProtocolEncoding/c419159".into(),
            raw_anchor_tx_hex,
            anchor_txid: anchor.txid.to_string(),
            anchor_vout: anchor.vout,
            owner_pubkey: vtxo.user_pubkey().to_string(),
            owner_policy_hex: hex::encode(ProtocolEncoding::serialize(vtxo.policy())),
            owner_policy_kind: "pubkey".into(),
            server_pubkey: vtxo.server_pubkey().to_string(),
            board: result.board.clone().context("board facts absent")?,
            checked_at_unix_ms,
            checked_tip_height,
            funding_confirmations,
            stock_checkpoint_ref: format!("board.{}.{}", anchor.txid, anchor.vout),
            terminal_receipt_context_schema: "spc.lab.ap5.pool-wallet-terminal-receipt-context/v1"
                .into(),
            terminal_receipt_context_canonical_json_hex: hex::encode(&receipt),
            terminal_receipt_context_sha256: digest(&receipt),
        };
        proof.validate()?;
        Ok(proof)
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        ensure!(
            self.schema == PROOF_SCHEMA && self.vtxo_encoding == "ark-lib/ProtocolEncoding/c419159",
            "proof schema/codec"
        );
        self.board_request.validate()?;
        ensure!(
            self.board_request.action == Action::BoardFloat
                && self.board_request.operation_id == self.original_board_operation_id
                && digest(&serde_json::to_vec(&self.board_request)?) == self.board_request_sha256,
            "original operation binding"
        );
        let raw = hex::decode(&self.raw_anchor_tx_hex)?;
        let tx: bitcoin::Transaction = bitcoin::consensus::deserialize(&raw)?;
        ensure!(
            bitcoin::consensus::serialize(&tx) == raw
                && tx.compute_txid().to_string() == self.anchor_txid,
            "anchor binding"
        );
        let bytes = hex::decode(&self.original_full_vtxo_hex)?;
        ensure!(
            digest(&bytes) == self.original_full_vtxo_sha256,
            "VTXO digest"
        );
        let vtxo = ark::Vtxo::<ark::vtxo::Full>::deserialize(&bytes)?;
        ensure!(
            ProtocolEncoding::serialize(&vtxo) == bytes,
            "noncanonical VTXO"
        );
        vtxo.validate(&tx)?;
        ensure!(
            vtxo.id().to_string() == self.original_vtxo_id
                && vtxo.chain_anchor()
                    == bitcoin::OutPoint::new(tx.compute_txid(), self.anchor_vout)
                && vtxo.user_pubkey().to_string() == self.owner_pubkey
                && vtxo.server_pubkey().to_string() == self.server_pubkey
                && self.owner_policy_kind == "pubkey"
                && *vtxo.policy() == ark::VtxoPolicy::new_pubkey(self.owner_pubkey.parse()?)
                && hex::encode(ProtocolEncoding::serialize(vtxo.policy())) == self.owner_policy_hex,
            "owner/policy/server binding"
        );
        ensure!(
            vtxo.amount().to_sat() == self.board.net_board_sats
                && vtxo.expiry_height().to_u32() == self.board.expiry_height
                && vtxo.exit_delta().to_u32() == self.board.vtxo_exit_delta
                && self.board.server_pubkey == self.server_pubkey
                && self.board.vtxo_id.as_deref() == Some(self.original_vtxo_id.as_str())
                && self
                    .board
                    .gross_board_sats
                    .checked_sub(self.board.board_fee_sats)
                    == Some(self.board.net_board_sats)
                && self
                    .board
                    .board_fee_sats
                    .checked_add(self.board.onchain_fee_sats)
                    == Some(self.board.actual_fee_sats)
                && tx
                    .output
                    .get(self.anchor_vout as usize)
                    .map(|o| o.value.to_sat())
                    == Some(self.board.gross_board_sats)
                && self.board.actual_fee_sats
                    <= self.board_request.max_fee_sats.context("fee cap absent")?
                && self.checked_at_unix_ms > 0
                && self.checked_tip_height < self.board.expiry_height
                && self.funding_confirmations > 0
                && self.funding_confirmations >= self.board.required_confirmations,
            "board facts binding"
        );
        ensure!(
            self.stock_checkpoint_ref == format!("board.{}.{}", self.anchor_txid, self.anchor_vout),
            "stock checkpoint binding"
        );
        ensure!(
            self.terminal_receipt_context_schema
                == "spc.lab.ap5.pool-wallet-terminal-receipt-context/v1",
            "receipt context schema"
        );
        let receipt_bytes = hex::decode(&self.terminal_receipt_context_canonical_json_hex)?;
        ensure!(
            digest(&receipt_bytes) == self.terminal_receipt_context_sha256,
            "receipt digest"
        );
        let receipt: ResultDto = serde_json::from_slice(&receipt_bytes)?;
        ensure!(
            receipt.proof.is_none()
                && receipt.retained_proof_sha256.is_none()
                && receipt.state == State::Confirmed
                && receipt.operation_id == self.original_board_operation_id
                && receipt.schema == super::wallet_control::RESULT_SCHEMA
                && receipt.funding_txid.as_deref() == Some(self.anchor_txid.as_str())
                && serde_json::to_vec(&receipt)? == receipt_bytes
                && serde_json::to_vec(&receipt.board)? == serde_json::to_vec(&Some(&self.board))?,
            "terminal receipt binding"
        );
        Ok(())
    }

    pub fn canonical_bytes(&self) -> anyhow::Result<Vec<u8>> {
        self.validate()?;
        Ok(serde_json::to_vec(self)?)
    }
}

/// Explicit stock movement links, not an amount-based claim that current
/// float descends from a board. Current outputs need their separate proofs.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoundImportFacts {
    pub schema: String,
    pub stock_movement_id: u32,
    pub input_vtxo_ids: Vec<String>,
    pub output_vtxo_ids: Vec<String>,
    pub round_funding_txid: String,
    pub offchain_fee_sats: u64,
    pub completed_at_unix_ms: u64,
}
impl RoundImportFacts {
    pub fn validate(&self) -> anyhow::Result<()> {
        ensure!(
            self.schema == ROUND_SCHEMA && self.completed_at_unix_ms > 0,
            "round fact schema"
        );
        let _: bitcoin::Txid = self.round_funding_txid.parse()?;
        for ids in [&self.input_vtxo_ids, &self.output_vtxo_ids] {
            ensure!(!ids.is_empty() && ids.len() <= 256, "round fact capacity");
            let mut previous: Option<&str> = None;
            for id in ids {
                ensure!(
                    previous.is_none_or(|p| p < id.as_str()),
                    "round ids not canonical"
                );
                ensure!(
                    id.parse::<ark::VtxoId>()?.to_string() == *id,
                    "round VTXO id"
                );
                previous = Some(id);
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineageState {
    RecordedPartial,
    Unavailable,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProofEnvelope {
    pub schema: String,
    pub proof_sha256: String,
    pub canonical_json_hex: String,
    pub original: OriginalBoardProof,
    pub lineage_state: LineageState,
    pub lineage_complete: bool,
    pub round_imports: Vec<RoundImportFacts>,
    pub lineage_error_code: Option<String>,
}
impl ProofEnvelope {
    pub fn validate(&self, original_id: &str) -> anyhow::Result<()> {
        let canonical = self.original.canonical_bytes()?;
        ensure!(
            self.schema == ENVELOPE_SCHEMA
                && self.original.original_board_operation_id == original_id
                && self.proof_sha256 == digest(&canonical)
                && self.canonical_json_hex == hex::encode(&canonical),
            "proof envelope binding"
        );
        ensure!(
            !self.lineage_complete && self.round_imports.len() <= 256,
            "lineage completeness unsupported"
        );
        for edge in &self.round_imports {
            edge.validate()?;
        }
        Ok(())
    }
}
