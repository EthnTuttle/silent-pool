//! Standard delegated hArk round client with SP-aware input signatures. Output
//! keys and persistence remain the existing stock child wallet. All group,
//! nonce, leaf and forfeit operations use the pinned ark-lib implementations.
use anyhow::{ensure, Context};
use ark::attestations::DelegatedRoundParticipationAttestation;
use ark::forfeit::HashLockedForfeitBundle;
use ark::tree::signed::{LeafVtxoCosignContext, UnlockHash};
use ark::vtxo::{Full, TransitionKind, Vtxo, VtxoValidationError};
use ark::{ProtocolEncoding, VtxoRequest};
use bitcoin::hashes::Hash;
use bitcoin::secp256k1::{Keypair, Secp256k1};
use rusqlite::params;
use server_rpc::{protos, TryFromBytes};

use super::live_bark::LiveBark;
use super::maintenance::WalletCommand;
use super::{ReceiveLedger, WalletKeys, WalletStore};

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RoundCheckpoint {
    version: u32,
    input_ids: Vec<Vec<u8>>,
    outputs: Vec<VtxoRequest>,
    unlock_hash: Option<Vec<u8>>,
    released: Option<Vec<Vec<u8>>>,
    funding: Option<Vec<u8>>,
    locked: Option<Vec<Vec<u8>>>,
    proof_binding: Option<[u8; 32]>,
}

impl RoundCheckpoint {
    fn needs_leaf_cosign(&self) -> bool {
        self.locked.is_none()
    }
    fn binding(&self) -> anyhow::Result<[u8; 32]> {
        let bytes = serde_json::to_vec(&(
            self.version,
            &self.input_ids,
            &self.outputs,
            self.unlock_hash
                .as_ref()
                .context("missing round unlock binding")?,
            self.funding
                .as_ref()
                .context("missing round funding binding")?,
            self.locked
                .as_ref()
                .context("missing signed round outputs")?,
        ))?;
        Ok(bitcoin::hashes::sha256::Hash::hash(&bytes).to_byte_array())
    }

    fn validate_binding(&self) -> anyhow::Result<()> {
        ensure!(self.version == 2, "unsupported round checkpoint version");
        ensure!(
            self.proof_binding == Some(self.binding()?),
            "signed round proof binding mismatch"
        );
        ensure!(
            self.locked.as_ref().is_some_and(|v| !v.is_empty()
                && v.len() == self.outputs.len()
                && v.iter().all(|b| !b.is_empty())),
            "incomplete signed round proofs"
        );
        Ok(())
    }

    fn check_preimage(&self, preimage: &[u8]) -> anyhow::Result<[u8; 32]> {
        self.validate_binding()?;
        let preimage: [u8; 32] = preimage
            .try_into()
            .context("invalid round preimage length")?;
        ensure!(
            self.unlock_hash.as_deref()
                == Some(UnlockHash::hash(&preimage).as_byte_array().as_slice()),
            "round preimage mismatch"
        );
        Ok(preimage)
    }

    /// The closure is not invoked if validation or SQLite durability fails.
    /// The same boundary is exercised by crash/RPC-trace tests.
    async fn forfeit_after_commit<T, F, Fut>(
        &self,
        store: &WalletStore,
        id: &str,
        send: F,
    ) -> anyhow::Result<T>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = anyhow::Result<T>>,
    {
        self.validate_binding()?;
        store.set_operation(
            id,
            "round_forfeiting",
            Some(&serde_json::to_vec(self)?),
            None,
        )?;
        send().await
    }
}

impl LiveBark {
    pub(super) async fn refresh_sp_round(
        &self,
        keys: &WalletKeys,
        store: &WalletStore,
        ledger: &ReceiveLedger,
        command: &WalletCommand,
    ) -> anyhow::Result<String> {
        self.check_connection().await?;
        let stage = store.operation_stage(&command.operation_id)?;
        // A lost SubmitRoundParticipation response has no client operation-id
        // lookup in the standard protocol. Keep the reservation unknown rather
        // than superseding a possibly issued round with a blind new submission.
        if stage == "round_submission_unknown" {
            return Ok(stage);
        }
        let mut inputs = vec![];
        let mut signers = vec![];
        for id in &command.vtxo_ids {
            let raw: Vec<u8> = store.lock().query_row(
                "SELECT raw FROM live_receipts WHERE vtxo_id=?1",
                params![hex::decode(id)?],
                |r| r.get(0),
            )?;
            let input = Vtxo::<Full>::deserialize(&raw)?;
            let signer = Keypair::from_secret_key(
                &Secp256k1::new(),
                &store
                    .lookup_u(&input.user_pubkey())?
                    .context("SP round signer missing")?,
            );
            ensure!(
                signer.public_key() == input.user_pubkey()
                    && input.server_pubkey() == self.server_pubkey,
                "SP round ownership mismatch"
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
            info.server_pubkey == self.server_pubkey && info.mailbox_pubkey == self.mailbox_pubkey,
            "wrong-ark-server"
        );
        info.fees.validate()?;
        let mut checkpoint = if stage == "prepared" {
            self.revalidate_receipts(keys, store, ledger).await?;
            let tip = self.wallet.chain().tip().await?;
            let total = inputs.iter().map(|v| v.amount()).sum::<bitcoin::Amount>();
            let fee_and_amount = (|| -> anyhow::Result<_> {
                let fee = info
                    .fees
                    .refresh
                    .calculate(
                        inputs
                            .iter()
                            .map(|v| ark::fees::VtxoFeeInfo::from_vtxo_and_tip(v, tip)),
                    )
                    .context("refresh fee overflow")?;
                Ok(ark::fees::validate_and_subtract_fee_min_dust(
                    total,
                    fee,
                    ark::vtxo::VTXO_DUST,
                )?)
            })();
            let amount = store.local_fee_result(&command.operation_id, fee_and_amount)?;
            let (destination, _) = self.wallet.derive_store_next_keypair().await?;
            let checkpoint = RoundCheckpoint {
                version: 2,
                input_ids: command
                    .vtxo_ids
                    .iter()
                    .map(hex::decode)
                    .collect::<Result<_, _>>()?,
                outputs: vec![VtxoRequest {
                    policy: ark::VtxoPolicy::new_pubkey(destination.public_key()),
                    amount,
                }],
                unlock_hash: None,
                released: None,
                funding: None,
                locked: None,
                proof_binding: None,
            };
            store.set_operation(
                &command.operation_id,
                "round_prepared",
                Some(&serde_json::to_vec(&checkpoint)?),
                None,
            )?;
            checkpoint
        } else {
            let raw: Vec<u8> = store.lock().query_row(
                "SELECT outputs FROM live_operations WHERE id=?1",
                [&command.operation_id],
                |r| r.get(0),
            )?;
            serde_json::from_slice(&raw)?
        };
        ensure!(
            checkpoint.version == 2
                && checkpoint.input_ids
                    == command
                        .vtxo_ids
                        .iter()
                        .map(hex::decode)
                        .collect::<Result<Vec<_>, _>>()?,
            "round checkpoint request binding mismatch"
        );
        if checkpoint.locked.is_some() {
            checkpoint.validate_binding()?;
        }
        ensure!(
            checkpoint.released.is_none() || checkpoint.locked.is_some(),
            "released round lacks durable signed output binding"
        );
        ensure!(
            stage != "round_forfeiting" || checkpoint.locked.is_some(),
            "ambiguous forfeits lack durable signed outputs"
        );
        if checkpoint.unlock_hash.is_none() {
            server
                .client
                .register_vtxo_transactions(protos::RegisterVtxoTransactionsRequest {
                    vtxos: inputs.iter().map(|v| v.serialize()).collect(),
                })
                .await?;
            let request = protos::RoundParticipationRequest {
                input_vtxos: inputs
                    .iter()
                    .zip(&signers)
                    .map(|(v, k)| protos::InputVtxo {
                        vtxo_id: v.id().to_bytes().to_vec(),
                        attestation: DelegatedRoundParticipationAttestation::new(
                            v.id(),
                            &checkpoint.outputs,
                            k,
                        )
                        .serialize(),
                    })
                    .collect(),
                vtxo_requests: checkpoint
                    .outputs
                    .iter()
                    .map(|r| protos::VtxoRequest {
                        policy: r.policy.serialize(),
                        amount: r.amount.to_sat(),
                    })
                    .collect(),
                unblinded_mailbox_id: Some(self.wallet.mailbox_identifier().serialize()),
                scheduled_height: None,
            };
            store.set_operation(
                &command.operation_id,
                "round_submission_unknown",
                None,
                None,
            )?;
            let response = server
                .client
                .submit_round_participation(request)
                .await?
                .into_inner();
            UnlockHash::from_slice(&response.unlock_hash)?;
            checkpoint.unlock_hash = Some(response.unlock_hash);
            store.set_operation(
                &command.operation_id,
                "round_pending",
                Some(&serde_json::to_vec(&checkpoint)?),
                None,
            )?;
        }
        let unlock = UnlockHash::from_slice(checkpoint.unlock_hash.as_ref().expect("round hash"))?;
        let mut response_outputs = vec![];
        if checkpoint.released.is_none() && checkpoint.locked.is_none() {
            let response = server
                .client
                .round_participation_status(protos::RoundParticipationStatusRequest {
                    unlock_hash: unlock.to_byte_array().to_vec(),
                })
                .await?
                .into_inner();
            let status = protos::RoundParticipationStatus::try_from(response.status)?;
            if status == protos::RoundParticipationStatus::RoundPartPending {
                return Ok("round_pending".into());
            }
            checkpoint.funding = Some(
                response
                    .round_funding_tx
                    .context("round funding unavailable")?,
            );
            response_outputs = response.output_vtxos;
        }
        let funding = bitcoin::consensus::deserialize::<bitcoin::Transaction>(
            checkpoint
                .funding
                .as_ref()
                .context("round funding proof missing")?,
        )?;
        let height = self
            .wallet
            .chain()
            .tx_confirmed(funding.compute_txid())
            .await?
            .context("round funding not confirmed")?;
        let tip = self.wallet.chain().tip().await?;
        let required: u32 = self.wallet.config().round_tx_required_confirmations.into();
        let tip_u32: u32 = tip.into();
        let height_u32: u32 = height.into();
        ensure!(
            tip_u32
                >= height_u32
                    .checked_add(required.max(1) - 1)
                    .context("round confirmation overflow")?,
            "round funding not sufficiently confirmed"
        );
        let anchor = self.wallet.chain().block_ref(height).await?;
        let block = self
            .wallet
            .chain()
            .block(anchor.hash)
            .await?
            .context("round block unavailable")?;
        ensure!(
            block.block_hash() == anchor.hash
                && block.check_merkle_root()
                && block
                    .txdata
                    .iter()
                    .any(|t| t.compute_txid() == funding.compute_txid()),
            "round funding canonical proof mismatch"
        );
        let mut outputs = if let Some(raw) = &checkpoint.released {
            raw.iter()
                .map(|b| Vtxo::<Full>::deserialize(b))
                .collect::<Result<Vec<_>, _>>()?
        } else if let Some(raw) = &checkpoint.locked {
            raw.iter()
                .map(|b| Vtxo::<Full>::deserialize(b))
                .collect::<Result<Vec<_>, _>>()?
        } else {
            response_outputs
                .iter()
                .map(|b| Vtxo::<Full>::deserialize(b))
                .collect::<Result<Vec<_>, _>>()?
        };
        ensure!(
            outputs.len() == checkpoint.outputs.len(),
            "round output count mismatch"
        );
        let mut ids = std::collections::BTreeSet::new();
        let margin: u32 = self.wallet.config().vtxo_exit_margin.into();
        let delta: u32 = inputs
            .iter()
            .map(|v| v.exit_delta())
            .max()
            .context("round inputs missing")?
            .into();
        let minimum = tip_u32
            .checked_add(margin.checked_mul(2).context("exit margin overflow")?)
            .and_then(|h| h.checked_add(delta))
            .context("exit height overflow")?;
        for (v, r) in outputs.iter().zip(&checkpoint.outputs) {
            ensure!(
                v.unlock_hash() == Some(unlock),
                "round output unlock hash mismatch"
            );
            ensure!(
                ids.insert(v.id())
                    && v.amount() == r.amount
                    && v.policy() == &r.policy
                    && v.server_pubkey() == self.server_pubkey
                    && v.exit_delta() == info.vtxo_exit_delta,
                "round output policy mismatch"
            );
            let expiry: u32 = v.expiry_height().into();
            if stage != "round_forfeiting" && checkpoint.released.is_none() {
                ensure!(expiry >= minimum, "round output cannot be exited safely");
            }
            v.validate_unsigned(&funding)?;
            if checkpoint.released.is_none() {
                match v.validate(&funding) {
                    Err(VtxoValidationError::GenesisTransition {
                        genesis_idx,
                        genesis_len,
                        transition_kind,
                        ..
                    }) if genesis_idx + 1 == genesis_len
                        && (transition_kind == TransitionKind::HashLockedCosigned.as_str()
                            || transition_kind
                                == TransitionKind::HashLockedCosigned_v0.as_str()) => {}
                    _ => anyhow::bail!("round output must fail only at its terminal hash lock"),
                }
            } else {
                v.validate(&funding)?;
            }
        }
        if checkpoint.released.is_none() {
            if checkpoint.needs_leaf_cosign() {
                for v in &mut outputs {
                    let (_, key) = self
                        .wallet
                        .pubkey_keypair(&v.user_pubkey())
                        .await?
                        .context("stock round output signer missing")?;
                    let (ctx, request) = LeafVtxoCosignContext::new(v, &funding, &key)?;
                    let response = server
                        .client
                        .request_leaf_vtxo_cosign(protos::LeafVtxoCosignRequest::from(request))
                        .await?
                        .into_inner()
                        .try_into()?;
                    ensure!(ctx.finalize(v, response), "round leaf signature invalid");
                }
                // Preserve the FULL leaf-cosigned, still-locked genesis proofs.
                // Recovery must never depend on obtaining another leaf signature.
                checkpoint.locked = Some(outputs.iter().map(|v| v.serialize()).collect());
                checkpoint.proof_binding = Some(checkpoint.binding()?);
            }
            let mut nonces = vec![];
            for chunk in inputs.chunks(server_rpc::MAX_NB_FORFEIT_NONCE_IDS) {
                let response = server
                    .client
                    .request_forfeit_nonces(protos::ForfeitNoncesRequest {
                        unlock_hash: unlock.to_byte_array().to_vec(),
                        vtxo_ids: chunk.iter().map(|v| v.id().to_bytes().to_vec()).collect(),
                    })
                    .await?
                    .into_inner();
                ensure!(
                    response.public_nonces.len() == chunk.len(),
                    "round nonce count mismatch"
                );
                for bytes in response.public_nonces {
                    nonces.push(ark::musig::PublicNonce::from_bytes(bytes)?);
                }
            }
            ensure!(
                self.wallet.chain().block_ref(height).await?.hash == anchor.hash,
                "round displaced before forfeits"
            );
            let bundles = inputs
                .iter()
                .zip(&signers)
                .zip(&nonces)
                .map(|((input, key), nonce)| {
                    HashLockedForfeitBundle::new(input, unlock, key, nonce).serialize()
                })
                .collect();
            let response = checkpoint
                .forfeit_after_commit(store, &command.operation_id, || async {
                    Ok(server
                        .client
                        .forfeit_vtxos(protos::ForfeitVtxosRequest {
                            forfeit_bundles: bundles,
                        })
                        .await?
                        .into_inner())
                })
                .await?;
            let preimage = checkpoint.check_preimage(&response.unlock_preimage)?;
            for v in &mut outputs {
                ensure!(
                    v.provide_unlock_preimage(preimage),
                    "round preimage mismatch"
                );
                v.validate(&funding)?;
            }
            checkpoint.released = Some(outputs.iter().map(|v| v.serialize()).collect());
            store.set_operation(
                &command.operation_id,
                "round_released",
                Some(&serde_json::to_vec(&checkpoint)?),
                None,
            )?;
        }
        server
            .client
            .register_vtxo_transactions(protos::RegisterVtxoTransactionsRequest {
                vtxos: outputs.iter().map(|v| v.serialize()).collect(),
            })
            .await?;
        ensure!(
            self.wallet.chain().block_ref(height).await?.hash == anchor.hash,
            "round displaced before import"
        );
        self.wallet
            .import_vtxos(&outputs, Default::default())
            .await?;
        for id in &command.vtxo_ids {
            store.lock().execute(
                "UPDATE vtxo_records SET state='spent' WHERE vtxo_id=?1",
                params![hex::decode(id)?],
            )?;
        }
        store.set_operation(
            &command.operation_id,
            "round_confirmed",
            None,
            Some(&funding.compute_txid().to_string()),
        )?;
        Ok("round_confirmed".into())
    }
}

#[cfg(test)]
mod audit_tests {
    use super::super::{maintenance::MaintenanceAction, StoreIdentity};
    use super::*;

    fn identity() -> StoreIdentity {
        StoreIdentity {
            wallet_fingerprint: bitcoin::bip32::Fingerprint::from([1; 4]),
            sp_id: [2; 32],
            network: "regtest".into(),
        }
    }
    fn command() -> WalletCommand {
        WalletCommand {
            operation_id: hex::encode([3; 32]),
            vtxo_ids: vec![hex::encode([4; 36])],
            action: MaintenanceAction::Refresh,
        }
    }
    fn mock_leaf_reply() -> RoundCheckpoint {
        let key = Keypair::from_secret_key(
            &Secp256k1::new(),
            &bitcoin::secp256k1::SecretKey::from_slice(&[5; 32]).unwrap(),
        );
        // Transport-boundary mock bytes, NOT a funded VTXO/cryptographic proof.
        // Production supplies serialize() only after stock leaf finalization.
        let mut c = RoundCheckpoint {
            version: 2,
            input_ids: vec![vec![4; 36]],
            outputs: vec![VtxoRequest {
                policy: ark::VtxoPolicy::new_pubkey(key.public_key()),
                amount: bitcoin::Amount::from_sat(1000),
            }],
            unlock_hash: Some(UnlockHash::hash(&[9; 32]).to_byte_array().to_vec()),
            funding: Some(b"mock canonical funding bytes".to_vec()),
            released: None,
            locked: Some(vec![
                b"mock FULL genesis and finalized locked leaf signature".to_vec(),
            ]),
            proof_binding: None,
        };
        c.proof_binding = Some(c.binding().unwrap());
        c
    }

    #[tokio::test]
    async fn audit_r1_forfeit_reply_loss_retains_signed_outputs_without_leaf_rpc_on_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let cmd = command();
        let checkpoint = mock_leaf_reply();
        let mut trace = vec!["leaf_cosign"];
        {
            let store = WalletStore::open(dir.path(), &identity()).unwrap();
            store.reserve_maintenance(&cmd, Some(b"[]")).unwrap();
            let result: anyhow::Result<()> = checkpoint
                .forfeit_after_commit(&store, &cmd.operation_id, || async {
                    let raw: Vec<u8> = store
                        .lock()
                        .query_row(
                            "SELECT outputs FROM live_operations WHERE id=?1",
                            [&cmd.operation_id],
                            |r| r.get(0),
                        )
                        .unwrap();
                    let durable: RoundCheckpoint = serde_json::from_slice(&raw).unwrap();
                    durable.validate_binding().unwrap();
                    assert_eq!(durable.locked, checkpoint.locked);
                    assert_eq!(durable.funding, checkpoint.funding);
                    trace.push("durable_signed_outputs");
                    trace.push("forfeit_vtxos");
                    anyhow::bail!("mock: server accepted forfeits, response lost")
                })
                .await;
            assert!(result.is_err());
        }
        let store = WalletStore::open(dir.path(), &identity()).unwrap();
        let raw: Vec<u8> = store
            .lock()
            .query_row(
                "SELECT outputs FROM live_operations WHERE id=?1",
                [&cmd.operation_id],
                |r| r.get(0),
            )
            .unwrap();
        let recovered: RoundCheckpoint = serde_json::from_slice(&raw).unwrap();
        assert!(!recovered.needs_leaf_cosign());
        assert_eq!(recovered.locked, checkpoint.locked);
        assert!(recovered.check_preimage(&[8; 32]).is_err());
        assert_eq!(recovered.check_preimage(&[9; 32]).unwrap(), [9; 32]);
        assert!(store
            .reserved_inputs()
            .unwrap()
            .contains([4u8; 36].as_slice()));
        assert_eq!(
            trace,
            ["leaf_cosign", "durable_signed_outputs", "forfeit_vtxos"]
        );
        let mut tampered = recovered;
        tampered.locked.as_mut().unwrap()[0].push(0);
        assert!(tampered.check_preimage(&[9; 32]).is_err());
    }

    #[tokio::test]
    async fn audit_r1_failed_checkpoint_never_invokes_forfeit_and_legacy_shape_refuses() {
        let dir = tempfile::tempdir().unwrap();
        let store = WalletStore::open(dir.path(), &identity()).unwrap();
        let cmd = command();
        store.reserve_maintenance(&cmd, Some(b"[]")).unwrap();
        store.lock().execute_batch("CREATE TRIGGER reject_checkpoint BEFORE UPDATE ON live_operations BEGIN SELECT RAISE(ABORT,'mock durability failure'); END;").unwrap();
        let called = std::cell::Cell::new(false);
        assert!(mock_leaf_reply()
            .forfeit_after_commit(&store, &cmd.operation_id, || async {
                called.set(true);
                Ok(())
            })
            .await
            .is_err());
        assert!(!called.get());
        let mut old = serde_json::to_value(mock_leaf_reply()).unwrap();
        old.as_object_mut().unwrap().remove("version");
        old.as_object_mut().unwrap().remove("locked");
        assert!(serde_json::from_value::<RoundCheckpoint>(old).is_err());
    }
}
