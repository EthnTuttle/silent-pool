//! SP-owned maintenance via a client-side arkoor into the SAME stock child
//! wallet, followed by stock refresh/offboard. No provider changes or extra
//! signer. Ambiguous sends stay locked/unknown; a restart never blindly sends
//! again. Stock actions retain their own authoritative checkpoints.
use anyhow::{ensure, Context};
use ark::arkoor::package::{ArkoorPackageBuilder, ArkoorPackageCosignResponse};
use ark::vtxo::{Full, Vtxo};
use ark::ProtocolEncoding;
use bitcoin::secp256k1::{Keypair, Secp256k1};
use rusqlite::{params, OptionalExtension};
use server_rpc::{protos, TryFromBytes};
use std::collections::BTreeSet;
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use super::live_bark::LiveBark;
use super::{ReceiveLedger, WalletKeys, WalletStore};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WalletCommand {
    pub operation_id: String,
    pub vtxo_ids: Vec<String>,
    pub action: MaintenanceAction,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MaintenanceAction {
    Refresh,
    Offboard { address: String },
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct DirectOffboardCheckpoint {
    request: ark::offboard::OffboardRequest,
    unsigned: Vec<u8>,
    nonces: Vec<Vec<u8>>,
    signed: Option<Vec<u8>>,
}

const TERMINAL_STAGES: [&str; 5] = [
    "round_confirmed",
    "offboard_confirmed",
    "stock_submitted",
    "rejected_no_effect",
    "noop",
];

/// Only these call-site-specific proofs authorize reservation release. An
/// arbitrary transport/storage error is deliberately not representable here.
#[derive(Clone, Copy)]
pub(super) enum NoEffectEvidence {
    LocalFeeOrDust,
    StockRefreshReturnedNone,
}

impl WalletCommand {
    pub fn validate(&self) -> anyhow::Result<()> {
        ensure!(
            self.operation_id.len() == 64
                && self.operation_id.bytes().all(|c| c.is_ascii_hexdigit()),
            "invalid wallet operation id"
        );
        ensure!(
            !self.vtxo_ids.is_empty() && self.vtxo_ids.len() <= 64,
            "wallet command needs 1..64 explicit owned inputs"
        );
        let mut unique = std::collections::BTreeSet::new();
        for id in &self.vtxo_ids {
            let bytes = hex::decode(id)?;
            ensure!(
                bytes.len() == 36 && unique.insert(bytes),
                "invalid or duplicate VTXO outpoint"
            );
        }
        if let MaintenanceAction::Offboard { address } = &self.action {
            address.parse::<bitcoin::Address<bitcoin::address::NetworkUnchecked>>()?;
        }
        Ok(())
    }
}

impl WalletStore {
    pub fn has_uncertain_operations(&self) -> anyhow::Result<bool> {
        self.init_operations()?;
        Ok(self
            .lock()
            .query_row("SELECT count(*) FROM live_operations o JOIN live_operation_safety s ON s.id=o.id WHERE o.stage NOT IN ('stock_submitted','offboard_confirmed','round_confirmed','rejected_no_effect','noop') AND s.phase != 'local'", [], |r| {
                r.get::<_, i64>(0)
            })?
            > 0)
    }
    pub(super) fn init_operations(&self) -> anyhow::Result<()> {
        self.lock().execute_batch(
            "CREATE TABLE IF NOT EXISTS live_operations (
            id TEXT PRIMARY KEY, payload BLOB NOT NULL, stage TEXT NOT NULL,
            outputs BLOB, result TEXT);
            CREATE TABLE IF NOT EXISTS live_operation_inputs (
            vtxo_id BLOB PRIMARY KEY, operation_id TEXT NOT NULL REFERENCES live_operations(id));
            CREATE TABLE IF NOT EXISTS live_operation_safety (
                id TEXT PRIMARY KEY REFERENCES live_operations(id), input_class TEXT NOT NULL,
                phase TEXT NOT NULL CHECK(phase IN ('local','may_have_effect','stock_refresh_call','legacy_unknown','terminal')));
            INSERT OR IGNORE INTO live_operation_safety(id,input_class,phase)
                SELECT id,'legacy','legacy_unknown' FROM live_operations;
            CREATE TABLE IF NOT EXISTS live_command_schedule (
                id INTEGER PRIMARY KEY CHECK(id=1), last_name BLOB NOT NULL);
            INSERT OR IGNORE INTO live_command_schedule(id,last_name) VALUES(1,X'');",
        )?;
        Ok(())
    }

    pub(super) fn operation_stage(&self, id: &str) -> anyhow::Result<String> {
        Ok(self
            .lock()
            .query_row("SELECT stage FROM live_operations WHERE id=?1", [id], |r| {
                r.get(0)
            })?)
    }

    pub(super) fn set_operation(
        &self,
        id: &str,
        stage: &str,
        outputs: Option<&[u8]>,
        result: Option<&str>,
    ) -> anyhow::Result<()> {
        let mut guard = self.lock();
        let tx = guard.transaction()?;
        let changed=tx.execute("UPDATE live_operations SET stage=?2,outputs=COALESCE(?3,outputs),result=COALESCE(?4,result) WHERE id=?1",params![id,stage,outputs,result])?;
        ensure!(changed == 1, "operation checkpoint missing");
        if !matches!(stage, "prepared" | "stock_ready" | "round_prepared") {
            tx.execute("UPDATE live_operation_safety SET phase='may_have_effect' WHERE id=?1 AND phase='local'",[id])?;
        }
        tx.commit()?;
        Ok(())
    }

    pub(super) fn finish_no_effect(
        &self,
        id: &str,
        evidence: NoEffectEvidence,
    ) -> anyhow::Result<String> {
        let mut guard = self.lock();
        let tx = guard.transaction()?;
        let (stage,class,phase):(String,String,String)=tx.query_row("SELECT o.stage,s.input_class,s.phase FROM live_operations o JOIN live_operation_safety s ON s.id=o.id WHERE o.id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
        let (terminal, reason) = match evidence {
            NoEffectEvidence::LocalFeeOrDust => {
                ensure!(
                    stage == "prepared" && phase == "local",
                    "no local no-effect provenance; reservation retained"
                );
                ("rejected_no_effect", "local_fee_or_dust")
            }
            NoEffectEvidence::StockRefreshReturnedNone => {
                ensure!(
                    stage == "stock_unknown" && class == "stock" && phase == "stock_refresh_call",
                    "no stock no-op provenance; reservation retained"
                );
                ("noop", "stock_refresh_returned_none")
            }
        };
        tx.execute(
            "UPDATE live_operations SET stage=?2,result=?3 WHERE id=?1",
            params![id, terminal, reason],
        )?;
        tx.execute(
            "UPDATE live_operation_safety SET phase='terminal' WHERE id=?1",
            [id],
        )?;
        tx.execute(
            "DELETE FROM live_operation_inputs WHERE operation_id=?1",
            [id],
        )?;
        tx.commit()?;
        Ok(terminal.into())
    }

    pub(super) fn local_fee_result<T>(
        &self,
        id: &str,
        result: anyhow::Result<T>,
    ) -> anyhow::Result<T> {
        match result {
            Ok(value) => Ok(value),
            Err(error) => {
                self.finish_no_effect(id, NoEffectEvidence::LocalFeeOrDust)?;
                Err(error)
            }
        }
    }

    pub(super) fn mark_stock_refresh_invocation(&self, id: &str) -> anyhow::Result<()> {
        // Legacy normalization inputs cannot acquire a no-effect proof for an
        // earlier transfer merely because the later refresh returned None.
        self.lock().execute("UPDATE live_operation_safety SET phase='stock_refresh_call' WHERE id=?1 AND input_class='stock' AND phase='may_have_effect' AND EXISTS(SELECT 1 FROM live_operations WHERE id=?1 AND stage='stock_unknown')",[id])?;
        Ok(())
    }

    pub(super) fn finish_stock_refresh(
        &self,
        id: &str,
        submitted: Option<String>,
    ) -> anyhow::Result<String> {
        if let Some(result) = submitted {
            self.set_operation(id, "stock_submitted", None, Some(&result))?;
            Ok("stock_submitted".into())
        } else {
            self.finish_no_effect(id, NoEffectEvidence::StockRefreshReturnedNone)
        }
    }

    fn terminal_outcome(&self, command: &WalletCommand) -> anyhow::Result<Option<String>> {
        let expected = serde_json::to_vec(command)?;
        let row: Option<(Vec<u8>, String)> = self
            .lock()
            .query_row(
                "SELECT payload,stage FROM live_operations WHERE id=?1",
                [&command.operation_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        Ok(row.and_then(|(payload, stage)| {
            (payload == expected && TERMINAL_STAGES.contains(&stage.as_str())).then_some(stage)
        }))
    }

    /// Advance the durable cursor BEFORE file parsing or network work, so a
    /// malformed file, timeout or restart cannot monopolize the bounded batch.
    pub(super) fn next_command_file(
        &self,
        files: &[PathBuf],
        visited: &BTreeSet<PathBuf>,
    ) -> anyhow::Result<Option<PathBuf>> {
        self.init_operations()?;
        let mut guard = self.lock();
        let tx = guard.transaction()?;
        let cursor: Vec<u8> = tx.query_row(
            "SELECT last_name FROM live_command_schedule WHERE id=1",
            [],
            |r| r.get(0),
        )?;
        let mut candidates = vec![];
        for path in files.iter().filter(|p| !visited.contains(*p)) {
            let name = path.file_name().context("command filename missing")?;
            let id = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            let stage: Option<String> = tx
                .query_row("SELECT stage FROM live_operations WHERE id=?1", [id], |r| {
                    r.get(0)
                })
                .optional()?;
            if stage
                .as_deref()
                .is_some_and(|s| TERMINAL_STAGES.contains(&s))
            {
                continue;
            }
            candidates.push((name.as_bytes().to_vec(), path.clone()));
        }
        candidates.sort_by(|a, b| {
            (a.0.as_slice() <= cursor.as_slice(), &a.0)
                .cmp(&(b.0.as_slice() <= cursor.as_slice(), &b.0))
        });
        let Some((name, path)) = candidates.into_iter().next() else {
            return Ok(None);
        };
        tx.execute(
            "UPDATE live_command_schedule SET last_name=?1 WHERE id=1",
            params![name],
        )?;
        tx.commit()?;
        Ok(Some(path))
    }

    pub(super) fn reserved_inputs(&self) -> anyhow::Result<BTreeSet<Vec<u8>>> {
        self.init_operations()?;
        let guard = self.lock();
        let mut stmt = guard.prepare("SELECT vtxo_id FROM live_operation_inputs")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Caller validates stock ownership/proofs before supplying ordinary bytes.
    /// The atomic reservation, immutable payload and provenance are shared by
    /// manual/automatic commands and their restart regressions.
    pub(super) fn reserve_maintenance(
        &self,
        command: &WalletCommand,
        ordinary_outputs: Option<&[u8]>,
    ) -> anyhow::Result<()> {
        command.validate()?;
        self.init_operations()?;
        let payload = serde_json::to_vec(command)?;
        let mut guard = self.lock();
        let tx = guard.transaction()?;
        let prior: Option<Vec<u8>> = tx
            .query_row(
                "SELECT payload FROM live_operations WHERE id=?1",
                [&command.operation_id],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(prior) = prior {
            ensure!(prior == payload, "wallet operation payload mismatch");
        } else {
            for id in command
                .vtxo_ids
                .iter()
                .filter(|_| ordinary_outputs.is_none())
            {
                let count:i64=tx.query_row("SELECT count(*) FROM vtxo_records WHERE vtxo_id=?1 AND state='spendable' AND expiry_height IS NOT NULL",params![hex::decode(id)?],|r|r.get(0))?;
                ensure!(count == 1, "maintenance input not proven spendable");
            }
            tx.execute(
                "INSERT INTO live_operations(id,payload,stage,outputs) VALUES(?1,?2,?3,?4)",
                params![
                    command.operation_id,
                    payload,
                    if ordinary_outputs.is_some() {
                        "stock_ready"
                    } else {
                        "prepared"
                    },
                    ordinary_outputs
                ],
            )?;
            for id in &command.vtxo_ids {
                tx.execute(
                    "INSERT INTO live_operation_inputs(vtxo_id,operation_id) VALUES(?1,?2)",
                    params![hex::decode(id)?, command.operation_id],
                )?;
            }
            tx.execute(
                "INSERT INTO live_operation_safety(id,input_class,phase) VALUES(?1,?2,'local')",
                params![
                    command.operation_id,
                    if ordinary_outputs.is_some() {
                        "stock"
                    } else {
                        "sp"
                    }
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}

/// Use the stock selection policy, then feed renewal through the SAME private
/// command/reservation path as manual work. The callback seam allows a second
/// generation's renewal to be exercised without inventing funded VTXOs.
pub(super) async fn queue_stock_renewal<F, Fut>(
    store: &WalletStore,
    dir: &Path,
    tip: u32,
    select: F,
) -> anyhow::Result<usize>
where
    F: FnOnce(Vec<ark::VtxoId>) -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<Vec<ark::VtxoId>>>,
{
    let mut excluded = store.reserved_inputs()?;
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.extension().is_none_or(|s| s != "json") {
            continue;
        }
        let Ok(command) = read_command(&path) else {
            continue;
        };
        let stage: Option<String> = store
            .lock()
            .query_row(
                "SELECT stage FROM live_operations WHERE id=?1",
                [&command.operation_id],
                |r| r.get(0),
            )
            .optional()?;
        if stage
            .as_deref()
            .is_some_and(|s| TERMINAL_STAGES.contains(&s))
        {
            continue;
        }
        for id in command.vtxo_ids {
            excluded.insert(hex::decode(id)?);
        }
    }
    let ids = excluded
        .iter()
        .map(|b| ark::VtxoId::from_slice(b))
        .collect::<Result<Vec<_>, _>>()?;
    let selected = select(ids).await?;
    let mut queued = 0;
    for id in selected
        .into_iter()
        .filter(|id| !excluded.contains(id.to_bytes().as_slice()))
    {
        use bitcoin::hashes::{sha256, Hash};
        let mut material = b"SPc/StockAutoRefresh/v1".to_vec();
        material.extend_from_slice(&id.to_bytes());
        material.extend_from_slice(&tip.to_be_bytes());
        let command = WalletCommand {
            operation_id: sha256::Hash::hash(&material).to_string(),
            vtxo_ids: vec![hex::encode(id.to_bytes())],
            action: MaintenanceAction::Refresh,
        };
        let stage: Option<String> = store
            .lock()
            .query_row(
                "SELECT stage FROM live_operations WHERE id=?1",
                [&command.operation_id],
                |r| r.get(0),
            )
            .optional()?;
        if stage
            .as_deref()
            .is_some_and(|s| TERMINAL_STAGES.contains(&s))
        {
            continue;
        }
        submit_command(dir.parent().context("wallet command parent")?, &command)?;
        queued += 1;
        if queued == 8 {
            break;
        }
    }
    Ok(queued)
}

fn read_command(path: &Path) -> anyhow::Result<WalletCommand> {
    let meta = std::fs::symlink_metadata(path)?;
    ensure!(
        meta.is_file() && meta.permissions().mode() & 0o777 == 0o600 && meta.len() <= 16384,
        "unsafe wallet command file"
    );
    let command: WalletCommand = serde_json::from_slice(&std::fs::read(path)?)?;
    command.validate()?;
    ensure!(
        path.file_name()
            .context("command filename")?
            .to_string_lossy()
            == format!("{}.json", command.operation_id),
        "wallet command filename mismatch"
    );
    Ok(command)
}

/// Read-model exclusion only: queued owner intent is not a funds authority.
/// Execution continues to perform its existing proof and atomic reservation
/// checks. Invalid queue data makes the observation unavailable.
pub(super) fn queued_ui_inputs(
    store: &WalletStore,
    dir: &Path,
) -> anyhow::Result<BTreeSet<Vec<u8>>> {
    if !dir.exists() {
        return Ok(BTreeSet::new());
    }
    let meta = std::fs::symlink_metadata(dir)?;
    ensure!(
        meta.is_dir() && meta.permissions().mode() & 0o777 == 0o700,
        "unsafe UI command queue"
    );
    let mut excluded = BTreeSet::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.extension().is_none_or(|s| s != "json") {
            continue;
        }
        let command = read_command(&path)?;
        let stage: Option<String> = store
            .lock()
            .query_row(
                "SELECT stage FROM live_operations WHERE id=?1",
                [&command.operation_id],
                |r| r.get(0),
            )
            .optional()?;
        if stage
            .as_deref()
            .is_some_and(|s| TERMINAL_STAGES.contains(&s))
        {
            continue;
        }
        for id in command.vtxo_ids {
            excluded.insert(hex::decode(id)?);
        }
        ensure!(excluded.len() <= 4096, "UI command input limit");
    }
    Ok(excluded)
}

impl LiveBark {
    async fn direct_sp_offboard(
        &self,
        keys: &WalletKeys,
        store: &WalletStore,
        ledger: &ReceiveLedger,
        command: &WalletCommand,
        address: &str,
    ) -> anyhow::Result<String> {
        use ark::offboard::{OffboardForfeitContext, OffboardRequest};
        use bitcoin::hashes::Hash;
        self.check_connection().await?;
        let mut input_ids = command
            .vtxo_ids
            .iter()
            .map(|id| Ok(ark::VtxoId::from_slice(&hex::decode(id)?)?))
            .collect::<anyhow::Result<Vec<_>>>()?;
        input_ids.sort_unstable();
        let mut inputs = vec![];
        let mut signers = vec![];
        for id in &input_ids {
            let raw: Vec<u8> = store.lock().query_row(
                "SELECT raw FROM live_receipts WHERE vtxo_id=?1",
                params![id.to_bytes().as_slice()],
                |r| r.get(0),
            )?;
            let input = Vtxo::<Full>::deserialize(&raw)?;
            let secret = store
                .lookup_u(&input.user_pubkey())?
                .context("SP offboard signer missing")?;
            let signer = Keypair::from_secret_key(&Secp256k1::new(), &secret);
            ensure!(
                signer.public_key() == input.user_pubkey()
                    && input.server_pubkey() == self.server_pubkey,
                "SP offboard ownership mismatch"
            );
            inputs.push(input);
            signers.push(signer);
        }
        let mut server = server_rpc::ServerConnection::builder()
            .address(&self.endpoint)
            .network(super::keys::bitcoin_network(keys.network))
            .connect()
            .await?;
        let info = server.ark_info().await;
        ensure!(
            info.server_pubkey == self.server_pubkey
                && info.mailbox_pubkey == self.mailbox_pubkey
                && inputs.len() <= info.max_offboard_inputs,
            "offboard server or input policy mismatch"
        );
        info.fees.validate()?;
        let stage = store.operation_stage(&command.operation_id)?;
        let mut checkpoint = if stage == "prepared" {
            self.revalidate_receipts(keys, store, ledger).await?;
            let destination = address
                .parse::<bitcoin::Address<_>>()?
                .require_network(super::keys::bitcoin_network(keys.network))?
                .script_pubkey();
            let fee_rate = server.offboard_feerate().await?;
            let tip = self.wallet.chain().tip().await?;
            let gross = inputs.iter().map(|v| v.amount()).sum::<bitcoin::Amount>();
            let fee_and_net = (|| -> anyhow::Result<_> {
                let fee = info
                    .fees
                    .offboard
                    .calculate(
                        &destination,
                        gross,
                        fee_rate,
                        inputs
                            .iter()
                            .map(|v| ark::fees::VtxoFeeInfo::from_vtxo_and_tip(v, tip)),
                    )
                    .context("offboard fee overflow")?;
                Ok(ark::fees::validate_and_subtract_fee_min_dust(
                    gross,
                    fee,
                    destination.minimal_non_dust(),
                )?)
            })();
            let net = store.local_fee_result(&command.operation_id, fee_and_net)?;
            let checkpoint = DirectOffboardCheckpoint {
                request: OffboardRequest {
                    script_pubkey: destination,
                    net_amount: net,
                    deduct_fees_from_gross_amount: true,
                    fee_rate,
                },
                unsigned: vec![],
                nonces: vec![],
                signed: None,
            };
            store.set_operation(
                &command.operation_id,
                "offboard_requested",
                Some(&serde_json::to_vec(&checkpoint)?),
                None,
            )?;
            checkpoint
        } else {
            let bytes: Vec<u8> = store.lock().query_row(
                "SELECT outputs FROM live_operations WHERE id=?1",
                [&command.operation_id],
                |r| r.get(0),
            )?;
            serde_json::from_slice(&bytes)?
        };
        if checkpoint.unsigned.is_empty() {
            server
                .client
                .register_vtxo_transactions(protos::RegisterVtxoTransactionsRequest {
                    vtxos: inputs.iter().map(|v| v.serialize()).collect(),
                })
                .await?;
            let response = server
                .client
                .prepare_offboard(protos::PrepareOffboardRequest {
                    offboard: Some(checkpoint.request.clone().into()),
                    input_vtxo_ids: input_ids.iter().map(|id| id.to_bytes().to_vec()).collect(),
                    attestation: signers
                        .iter()
                        .map(|key| {
                            ark::attestations::OffboardRequestAttestation::new(
                                &checkpoint.request,
                                &input_ids,
                                key,
                            )
                            .serialize()
                        })
                        .collect(),
                })
                .await?
                .into_inner();
            let tx =
                bitcoin::consensus::deserialize::<bitcoin::Transaction>(&response.offboard_tx)?;
            ensure!(
                bitcoin::consensus::serialize(&tx) == response.offboard_tx,
                "noncanonical offboard transaction"
            );
            OffboardForfeitContext::new(&inputs, &tx)?.validate_offboard_tx(&checkpoint.request)?;
            ensure!(
                response.forfeit_cosign_nonces.len() == inputs.len(),
                "offboard nonce count mismatch"
            );
            for n in &response.forfeit_cosign_nonces {
                ark::musig::PublicNonce::from_bytes(n)?;
            }
            checkpoint.unsigned = response.offboard_tx;
            checkpoint.nonces = response.forfeit_cosign_nonces;
            store.set_operation(
                &command.operation_id,
                "offboard_prepared",
                Some(&serde_json::to_vec(&checkpoint)?),
                None,
            )?;
        }
        let unsigned =
            bitcoin::consensus::deserialize::<bitcoin::Transaction>(&checkpoint.unsigned)?;
        let txid = unsigned.compute_txid();
        if checkpoint.signed.is_none() {
            let nonces = checkpoint
                .nonces
                .iter()
                .map(ark::musig::PublicNonce::from_bytes)
                .collect::<Result<Vec<_>, _>>()?;
            let context = OffboardForfeitContext::new(&inputs, &unsigned)?;
            context.validate_offboard_tx(&checkpoint.request)?;
            let sigs = context.user_sign_forfeits(&signers, &nonces)?;
            store.set_operation(&command.operation_id, "offboard_finishing", None, None)?;
            let signed = if let Some(tx) = self.wallet.chain().get_tx(&txid).await? {
                bitcoin::consensus::serialize(&tx)
            } else {
                server
                    .client
                    .finish_offboard(protos::FinishOffboardRequest {
                        offboard_txid: txid.to_byte_array().to_vec(),
                        user_nonces: sigs
                            .public_nonces
                            .iter()
                            .map(|n| n.serialize().to_vec())
                            .collect(),
                        partial_signatures: sigs
                            .partial_signatures
                            .iter()
                            .map(|s| s.serialize().to_vec())
                            .collect(),
                    })
                    .await?
                    .into_inner()
                    .signed_offboard_tx
            };
            let tx = bitcoin::consensus::deserialize::<bitcoin::Transaction>(&signed)?;
            ensure!(
                tx.compute_txid() == txid
                    && bitcoin::consensus::serialize(&tx) == signed
                    && tx
                        .input
                        .iter()
                        .all(|i| !i.witness.is_empty() || !i.script_sig.is_empty()),
                "invalid signed offboard response"
            );
            checkpoint.signed = Some(signed);
            store.set_operation(
                &command.operation_id,
                "offboard_signed",
                Some(&serde_json::to_vec(&checkpoint)?),
                None,
            )?;
        }
        let signed = bitcoin::consensus::deserialize::<bitcoin::Transaction>(
            checkpoint.signed.as_ref().expect("signed checkpoint"),
        )?;
        let confirmation = self.wallet.chain().tx_confirmed(txid).await?;
        let result = if let Some(height) = confirmation {
            let anchor = self.wallet.chain().block_ref(height).await?;
            let block = self
                .wallet
                .chain()
                .block(anchor.hash)
                .await?
                .context("offboard confirmation proof unavailable")?;
            ensure!(
                block.block_hash() == anchor.hash
                    && block.check_merkle_root()
                    && block.txdata.iter().any(|t| t.compute_txid() == txid)
                    && self.wallet.chain().block_ref(height).await?.hash == anchor.hash,
                "offboard confirmation displaced"
            );
            "offboard_confirmed"
        } else {
            self.wallet.chain().broadcast_tx(&signed).await?;
            "offboard_broadcast"
        };
        for id in &input_ids {
            store.lock().execute(
                "UPDATE vtxo_records SET state='spent' WHERE vtxo_id=?1",
                params![id.to_bytes().as_slice()],
            )?;
        }
        store.set_operation(&command.operation_id, result, None, Some(&txid.to_string()))?;
        Ok(result.into())
    }
    async fn validate_stock_input(&self, id: ark::VtxoId) -> anyhow::Result<Vtxo<Full>> {
        self.check_connection().await?;
        let stored = self.wallet.get_vtxo_by_id(id).await?;
        ensure!(
            matches!(stored.state, bark::vtxo::VtxoState::Spendable),
            "stock input is locked or spent"
        );
        let vtxo = self.wallet.get_full_vtxo(id).await?;
        let signer = self.wallet.get_vtxo_key(id).await?;
        ensure!(
            signer.public_key() == vtxo.user_pubkey() && vtxo.server_pubkey() == self.server_pubkey,
            "stock input ownership mismatch"
        );
        self.wallet.validate_vtxo(&vtxo).await?;
        let height = self
            .wallet
            .chain()
            .tx_confirmed(vtxo.chain_anchor().txid)
            .await?
            .context("stock anchor unconfirmed")?;
        let anchor = self.wallet.chain().block_ref(height).await?;
        let block = self
            .wallet
            .chain()
            .block(anchor.hash)
            .await?
            .context("stock anchor unavailable")?;
        ensure!(
            block.block_hash() == anchor.hash
                && block.check_merkle_root()
                && block
                    .txdata
                    .iter()
                    .any(|tx| tx.compute_txid() == vtxo.chain_anchor().txid),
            "stock anchor proof mismatch"
        );
        let mut server = server_rpc::ServerConnection::builder()
            .address(&self.endpoint)
            .network(self.wallet.network().await?)
            .connect()
            .await?;
        let info = server.ark_info().await;
        ensure!(
            info.server_pubkey == self.server_pubkey && info.mailbox_pubkey == self.mailbox_pubkey,
            "wrong-ark-server"
        );
        let response = server
            .client
            .get_vtxo_status(protos::GetVtxoStatusRequest {
                vtxo_id: id.to_bytes().to_vec(),
                attestation: ark::attestations::VtxoStatusAttestation::new(id, &signer).serialize(),
            })
            .await?
            .into_inner();
        ensure!(
            response.spend_state == protos::VtxoSpendState::Spendable as i32
                && self.wallet.chain().block_ref(height).await?.hash == anchor.hash,
            "stock spendability unknown"
        );
        Ok(vtxo)
    }

    /// Stock balance is read only after proof/status validation of every
    /// available VTXO. Pending stock actions remain explicitly pending.
    pub async fn proven_stock_balance(&self) -> anyhow::Result<(u64, u64, u64)> {
        self.ui_observations.invalidate_normal();
        let mut ui_capture = super::ui_inputs::Capture::new();
        self.wallet.sync_pending_rounds().await?;
        self.wallet.sync_pending_offboards().await?;
        let available = self.wallet.spendable_vtxos().await?;
        let mut sum = 0u64;
        for v in available {
            let validated = self.validate_stock_input(v.id()).await?;
            sum = sum
                .checked_add(validated.amount().to_sat())
                .context("stock balance overflow")?;
            ui_capture.normal(
                validated.id().to_bytes(),
                validated.amount().to_sat(),
                validated.expiry_height().into(),
            );
        }
        let balance = self.wallet.balance().await?;
        ensure!(
            sum == balance
                .spendable
                .to_sat()
                .checked_add(balance.needs_refresh.to_sat())
                .context("stock balance overflow")?,
            "stock balance changed while validating"
        );
        self.ui_observations.normal_validated(ui_capture);
        Ok((
            balance.spendable.to_sat(),
            balance.pending().to_sat(),
            balance.needs_refresh.to_sat(),
        ))
    }
    pub async fn process_commands(
        &self,
        keys: &WalletKeys,
        store: &WalletStore,
        ledger: &ReceiveLedger,
        dir: &Path,
    ) -> anyhow::Result<()> {
        private_queue(dir)?;
        if let Some(tip) = store.last_scan_height()? {
            use bitcoin::hashes::{sha256, Hash};
            for record in store
                .vtxo_records()?
                .into_iter()
                .filter(|r| {
                    r.state == super::receive::VtxoState::Spendable
                        && r.expiry.height().is_some_and(|h| {
                            h <= tip.saturating_add(super::REFRESH_THRESHOLD_BLOCKS)
                        })
                })
                .take(8)
            {
                let mut preimage = b"SPc/AutoRefresh/v1".to_vec();
                preimage.extend_from_slice(&record.vtxo_id);
                let command = WalletCommand {
                    operation_id: sha256::Hash::hash(&preimage).to_string(),
                    vtxo_ids: vec![hex::encode(record.vtxo_id)],
                    action: MaintenanceAction::Refresh,
                };
                submit_command(dir.parent().context("wallet command parent")?, &command)?;
            }
        }
        let files = std::fs::read_dir(dir)?
            .map(|e| e.map(|e| e.path()))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .collect::<Vec<_>>();
        let mut visited = BTreeSet::new();
        let mut work_failed = false;
        for _ in 0..8 {
            let Some(path) = store.next_command_file(&files, &visited)? else {
                break;
            };
            visited.insert(path.clone());
            let parsed = read_command(&path);
            let Ok(command) = parsed else {
                work_failed = true;
                continue;
            };
            let outcome = tokio::time::timeout(
                std::time::Duration::from_secs(10),
                self.maintain(keys, store, ledger, &command),
            )
            .await;
            let state = match outcome {
                Ok(Ok(stage)) => stage,
                _ => match store.terminal_outcome(&command)? {
                    Some(terminal) => terminal,
                    None => {
                        work_failed = true;
                        "unknown".into()
                    }
                },
            };
            // Public outcome only, kept in the owner's private directory.
            let result = serde_json::to_vec(
                &serde_json::json!({"operation_id":command.operation_id,"state":state}),
            )?;
            let temp = dir.join(format!("{}.result.tmp", command.operation_id));
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&temp)?;
            f.write_all(&result)?;
            f.sync_all()?;
            std::fs::rename(temp, dir.join(format!("{}.result", command.operation_id)))?;
        }
        // run_daemon=false: this loop owns NEW ordinary-wallet renewal as well
        // as progressing existing rounds. Pending rounds (including imported
        // stock state) and manual reservations are excluded before selection.
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            self.wallet.sync_pending_rounds().await?;
            let tip: u32 = self.wallet.chain().tip().await?.into();
            queue_stock_renewal(store, dir, tip, |mut excluded| async move {
                excluded.extend(
                    self.wallet
                        .pending_round_input_vtxos()
                        .await?
                        .iter()
                        .map(|v| v.id()),
                );
                Ok(self
                    .wallet
                    .get_vtxos_to_refresh_with_excluded(excluded)
                    .await?
                    .iter()
                    .map(|v| v.id())
                    .collect())
            })
            .await
        })
        .await??;
        ensure!(
            !work_failed,
            "wallet command work remains unknown or invalid"
        );
        Ok(())
    }
    /// The returned stage is an operation state, NOT evidence of settled funds.
    /// Repeated identical IDs replay it; changed payloads fail closed.
    pub async fn maintain(
        &self,
        keys: &WalletKeys,
        store: &WalletStore,
        ledger: &ReceiveLedger,
        command: &WalletCommand,
    ) -> anyhow::Result<String> {
        command.validate()?;
        if let MaintenanceAction::Offboard { address } = &command.action {
            address
                .parse::<bitcoin::Address<_>>()?
                .require_network(super::keys::bitcoin_network(keys.network))?;
        }
        store.init_operations()?;
        let exists: bool = store.lock().query_row(
            "SELECT EXISTS(SELECT 1 FROM live_operations WHERE id=?1)",
            [&command.operation_id],
            |r| r.get(0),
        )?;
        let mut ordinary_outputs = None;
        if !exists {
            let mut sp_count = 0;
            for id in &command.vtxo_ids {
                sp_count += store.lock().query_row(
                    "SELECT count(*) FROM vtxo_records WHERE vtxo_id=?1",
                    params![hex::decode(id)?],
                    |r| r.get::<_, usize>(0),
                )?;
            }
            if sp_count == 0 {
                let mut full = vec![];
                for id in &command.vtxo_ids {
                    let vtxo = self
                        .validate_stock_input(ark::VtxoId::from_slice(&hex::decode(id)?)?)
                        .await?;
                    full.push(vtxo.serialize());
                }
                ordinary_outputs = Some(serde_json::to_vec(&full)?);
            } else {
                ensure!(
                    sp_count == command.vtxo_ids.len(),
                    "mixed input classes must be maintained separately"
                );
            }
        }
        store.reserve_maintenance(command, ordinary_outputs.as_deref())?;
        let stage = store.operation_stage(&command.operation_id)?;
        if TERMINAL_STAGES.contains(&stage.as_str()) {
            return Ok(stage);
        }
        if (stage == "prepared" || stage.starts_with("round_"))
            && matches!(command.action, MaintenanceAction::Refresh)
        {
            return self.refresh_sp_round(keys, store, ledger, command).await;
        }
        if (stage == "prepared" || stage.starts_with("offboard_"))
            && matches!(command.action, MaintenanceAction::Offboard { .. })
        {
            if let MaintenanceAction::Offboard { address } = &command.action {
                return self
                    .direct_sp_offboard(keys, store, ledger, command, address)
                    .await;
            }
        }
        if matches!(
            stage.as_str(),
            "normalization_unknown" | "stock_unknown" | "stock_submitted"
        ) {
            // Stock journals resume their own operations. Never start another
            // arkoor/round/offboard solely because an RPC reply was lost.
            self.wallet.sync_pending_rounds().await?;
            self.wallet.sync_pending_offboards().await?;
            return Ok(stage);
        }
        self.check_connection().await?;
        if stage == "prepared" {
            self.revalidate_receipts(keys, store, ledger).await?;
            let mut inputs = Vec::new();
            let mut signers = Vec::new();
            for id in &command.vtxo_ids {
                let raw: Vec<u8> = store.lock().query_row(
                    "SELECT raw FROM live_receipts WHERE vtxo_id=?1",
                    params![hex::decode(id)?],
                    |r| r.get(0),
                )?;
                let input = Vtxo::<Full>::deserialize(&raw)?;
                let secret = store
                    .lookup_u(&input.user_pubkey())?
                    .context("SP-owned signer missing")?;
                let signer = Keypair::from_secret_key(&Secp256k1::new(), &secret);
                ensure!(
                    signer.public_key() == input.user_pubkey(),
                    "SP-owned key mismatch"
                );
                inputs.push(input);
                signers.push(signer);
            }
            let (destination, _index) = self.wallet.derive_store_next_keypair().await?;
            let builder = ArkoorPackageBuilder::new_claim_all_with_checkpoints(
                inputs.clone(),
                ark::VtxoPolicy::new_pubkey(destination.public_key()),
            )?
            .generate_user_nonces(&signers)?;
            let mut server = server_rpc::ServerConnection::builder()
                .address(&self.endpoint)
                .network(super::keys::bitcoin_network(keys.network))
                .connect()
                .await?;
            let info = server.ark_info().await;
            ensure!(
                info.server_pubkey == self.server_pubkey
                    && info.mailbox_pubkey == self.mailbox_pubkey,
                "wrong-ark-server"
            );
            info.fees.validate()?;
            server
                .client
                .register_vtxo_transactions(protos::RegisterVtxoTransactionsRequest {
                    vtxos: inputs.iter().map(|v| v.serialize()).collect(),
                })
                .await?;
            store.set_operation(&command.operation_id, "normalization_unknown", None, None)?;
            let response = server
                .client
                .request_arkoor_cosign(protos::ArkoorPackageCosignRequest::from(
                    builder.cosign_request(),
                ))
                .await?
                .into_inner();
            let signed = builder
                .user_cosign(&signers, ArkoorPackageCosignResponse::try_from(response)?)?
                .build_signed_vtxos();
            ensure!(
                signed
                    .iter()
                    .all(|v| v.user_pubkey() == destination.public_key()),
                "normalization output mismatch"
            );
            let bytes =
                serde_json::to_vec(&signed.iter().map(|v| v.serialize()).collect::<Vec<_>>())?;
            store.set_operation(
                &command.operation_id,
                "normalization_signed",
                Some(&bytes),
                None,
            )?;
        }
        let raw: Vec<u8> = store.lock().query_row(
            "SELECT outputs FROM live_operations WHERE id=?1",
            [&command.operation_id],
            |r| r.get(0),
        )?;
        let outputs: Vec<Vec<u8>> = serde_json::from_slice(&raw)?;
        let vtxos = outputs
            .iter()
            .map(|v| Vtxo::<Full>::deserialize(v))
            .collect::<Result<Vec<_>, _>>()?;
        if stage == "stock_ready" {
            for v in &vtxos {
                self.validate_stock_input(v.id()).await?;
            }
        }
        let mut server = server_rpc::ServerConnection::builder()
            .address(&self.endpoint)
            .network(super::keys::bitcoin_network(keys.network))
            .connect()
            .await?;
        let info = server.ark_info().await;
        ensure!(
            info.server_pubkey == self.server_pubkey && info.mailbox_pubkey == self.mailbox_pubkey,
            "wrong-ark-server"
        );
        server
            .client
            .register_vtxo_transactions(protos::RegisterVtxoTransactionsRequest { vtxos: outputs })
            .await?;
        self.wallet.import_vtxos(&vtxos, Default::default()).await?;
        // Import is idempotent. Old SP-owned value is consumed exactly once;
        // the new ordinary VTXOs live only in the stock wallet persister.
        for id in &command.vtxo_ids {
            store.lock().execute(
                "UPDATE vtxo_records SET state='spent' WHERE vtxo_id=?1",
                params![hex::decode(id)?],
            )?;
        }
        store.set_operation(&command.operation_id, "stock_unknown", None, None)?;
        let ids: Vec<_> = vtxos.iter().map(|v| v.id()).collect();
        let result = match &command.action {
            MaintenanceAction::Refresh => {
                store.mark_stock_refresh_invocation(&command.operation_id)?;
                let outcome = self.wallet.refresh_vtxos_delegated(ids).await?;
                return store.finish_stock_refresh(
                    &command.operation_id,
                    outcome.map(|round| format!("round_submitted:{}", round.id())),
                );
            }
            MaintenanceAction::Offboard { address } => {
                let address = address
                    .parse::<bitcoin::Address<_>>()?
                    .require_network(super::keys::bitcoin_network(keys.network))?;
                format!(
                    "offboard_broadcast:{}",
                    self.wallet.offboard_vtxos(ids, address).await?
                )
            }
        };
        store.set_operation(
            &command.operation_id,
            "stock_submitted",
            None,
            Some(&result),
        )?;
        Ok("stock_submitted".into())
    }
}

fn private_queue(dir: &Path) -> anyhow::Result<()> {
    if !dir.exists() {
        std::fs::DirBuilder::new().mode(0o700).create(dir)?;
    }
    let meta = std::fs::symlink_metadata(dir)?;
    ensure!(
        meta.is_dir() && meta.permissions().mode() & 0o777 == 0o700,
        "wallet command queue must be private"
    );
    Ok(())
}

pub fn submit_command(datadir: &Path, command: &WalletCommand) -> anyhow::Result<()> {
    command.validate()?;
    let meta = std::fs::symlink_metadata(datadir)?;
    ensure!(
        meta.is_dir() && meta.permissions().mode() & 0o777 == 0o700,
        "wallet datadir must be private"
    );
    let dir = datadir.join("commands");
    private_queue(&dir)?;
    let dest = dir.join(format!("{}.json", command.operation_id));
    let bytes = serde_json::to_vec(command)?;
    if dest.exists() {
        ensure!(
            std::fs::read(dest)? == bytes,
            "wallet operation payload mismatch"
        );
        return Ok(());
    }
    let temp = dir.join(format!(
        "{}.{}.tmp",
        command.operation_id,
        std::process::id()
    ));
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temp)?;
    f.write_all(&bytes)?;
    f.sync_all()?;
    std::fs::hard_link(&temp, &dest)?;
    std::fs::remove_file(temp)?;
    std::fs::File::open(&dir)?.sync_all()?;
    Ok(())
}
