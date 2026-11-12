//! SP-aware receive adapter over stock ReadMailbox and GetVtxoStatus. The
//! pinned protocol carries arkoor proof bytes, not a custom encrypted envelope.
//! Owner context is independently derived from a canonical coinbase before
//! any proof is accepted. No pool identity is sent to the Ark provider.

use anyhow::{ensure, Context};
use ark::attestations::VtxoStatusAttestation;
use ark::mailbox::MailboxAuthorization;
use ark::vtxo::{Full, Vtxo};
use ark::ProtocolEncoding;
use bitcoin::hashes::Hash;
use bitcoin::secp256k1::{Keypair, PublicKey, Secp256k1};
use bitcoin::BlockHash;
use server_rpc::protos;

use super::live_bark::LiveBark;
use super::receive::{Expiry, ExpirySource, ObservedVtxo, VtxoState};
use super::{ReceiveLedger, WalletKeys, WalletStore};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EpochReadResult {
    pub received: usize,
    pub continuation: bool,
}

/// Bounded pagination state shared by the live reader and scripted transport
/// regressions. A page counts only after all its verified message commits.
#[derive(Default)]
pub(super) struct MailboxBurst {
    pages: usize,
    received: usize,
}

impl MailboxBurst {
    pub(super) fn committed_page(
        &mut self,
        received: usize,
        have_more: bool,
    ) -> anyhow::Result<Option<EpochReadResult>> {
        ensure!(self.pages < 10, "mailbox burst exhausted");
        self.pages += 1;
        self.received = self
            .received
            .checked_add(received)
            .context("mailbox count overflow")?;
        Ok((!have_more || self.pages == 10).then_some(EpochReadResult {
            received: self.received,
            continuation: have_more,
        }))
    }
}

impl LiveBark {
    pub async fn revalidate_receipts(
        &self,
        keys: &WalletKeys,
        store: &WalletStore,
        ledger: &ReceiveLedger,
    ) -> anyhow::Result<()> {
        self.ui_observations.invalidate_derived();
        let mut ui_capture = super::ui_inputs::Capture::new();
        let receipts = {
            let guard = store.lock();
            let mut stmt = guard.prepare("SELECT raw,epoch_height,epoch_hash,mailbox FROM live_receipts WHERE vtxo_id IN (SELECT vtxo_id FROM vtxo_records WHERE state != 'spent')")?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, Vec<u8>>(0)?,
                    r.get::<_, u32>(1)?,
                    r.get::<_, Vec<u8>>(2)?,
                    r.get::<_, Vec<u8>>(3)?,
                ))
            })?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        for (raw, height, hash, mailbox) in receipts {
            let hash = BlockHash::from_slice(&hash)?;
            let preserved = store.verified_aggregate_origin(&hash.to_byte_array())?;
            let aggregate_origin = preserved.is_some();
            ensure!(
                aggregate_origin
                    || self.wallet.chain().block_ref(height.into()).await?.hash == hash,
                "receipt epoch displaced"
            );
            let block = if let Some(block) = preserved {
                block
            } else {
                self.wallet
                    .chain()
                    .block(hash)
                    .await?
                    .context("receipt epoch unavailable")?
            };
            ensure!(
                block.block_hash() == hash && block.check_merkle_root(),
                "receipt epoch proof mismatch"
            );
            let coinbase = block.txdata.first().context("receipt coinbase missing")?;
            let a_x = spc_scan::carrier_from_tx(coinbase).context("receipt carrier missing")?;
            let mut compressed = [0u8; 33];
            compressed[0] = 2;
            compressed[1..].copy_from_slice(&a_x);
            let shared = spc_crypto::SharedSecret::miner_side(
                height,
                &keys.b_scan,
                &PublicKey::from_slice(&compressed)?,
            )?;
            let tweaks = spc_crypto::tier_tweaks(&shared, 0)?;
            let owner = Keypair::from_secret_key(
                &Secp256k1::new(),
                &spc_crypto::ark_user_secret(&keys.b_ark, &tweaks.t_ark)?,
            );
            ensure!(
                mailbox
                    == spc_crypto::derived_epoch_mailbox_pubkey(&keys.mailbox_public(), &shared)?
                        .serialize(),
                "receipt mailbox mismatch"
            );
            let vtxo = Vtxo::<Full>::deserialize(&raw)?;
            ensure!(
                vtxo.serialize() == raw
                    && vtxo.policy() == &ark::VtxoPolicy::new_pubkey(owner.public_key())
                    && vtxo.server_pubkey() == self.server_pubkey,
                "receipt ownership mismatch"
            );
            self.wallet.validate_vtxo(&vtxo).await?;
            let anchor_height = self
                .wallet
                .chain()
                .tx_confirmed(vtxo.chain_anchor().txid)
                .await?
                .context("receipt anchor unconfirmed")?;
            let anchor = self.wallet.chain().block_ref(anchor_height).await?;
            let anchor_block = self
                .wallet
                .chain()
                .block(anchor.hash)
                .await?
                .context("receipt anchor block unavailable")?;
            ensure!(
                anchor_block.block_hash() == anchor.hash
                    && anchor_block.check_merkle_root()
                    && anchor_block
                        .txdata
                        .iter()
                        .any(|t| t.compute_txid() == vtxo.chain_anchor().txid),
                "receipt anchor proof mismatch"
            );
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
            let status = server
                .client
                .get_vtxo_status(protos::GetVtxoStatusRequest {
                    vtxo_id: vtxo.id().to_bytes().to_vec(),
                    attestation: VtxoStatusAttestation::new(vtxo.id(), &owner).serialize(),
                })
                .await?
                .into_inner();
            let state = match protos::VtxoSpendState::try_from(status.spend_state)? {
                protos::VtxoSpendState::Spendable => VtxoState::Spendable,
                protos::VtxoSpendState::Spent => VtxoState::Spent,
                _ => anyhow::bail!("receipt spendability unknown"),
            };
            ensure!(
                (aggregate_origin
                    || self.wallet.chain().block_ref(height.into()).await?.hash == hash)
                    && self.wallet.chain().block_ref(anchor_height).await?.hash == anchor.hash,
                "chain changed during receipt validation"
            );
            let observed = ObservedVtxo {
                vtxo_id: vtxo.id().to_bytes(),
                u_pubkey: owner.public_key(),
                amount_sats: vtxo.amount().to_sat(),
                state,
                expiry: Expiry::At {
                    height: vtxo.expiry_height().into(),
                    source: ExpirySource::VtxoAncestry,
                },
            };
            ledger.record(&observed)?;
            ui_capture.derived(&observed);
        }
        self.ui_observations.derived_validated(ui_capture);
        Ok(())
    }
    /// Retain proofs for a pool epoch using the default derived mailbox. The
    /// caller supplies a locator, never a trusted A_send or expiry. A missing
    /// block/anchor/status is an error/unknown, never a zero balance.
    pub async fn receive_epoch(
        &self,
        keys: &WalletKeys,
        store: &WalletStore,
        _ledger: &ReceiveLedger,
        height: u32,
        expected_hash: BlockHash,
    ) -> anyhow::Result<EpochReadResult> {
        self.check_connection().await?;
        self.wallet.chain().invalidate_caches().await;
        let canonical = self.wallet.chain().block_ref(height.into()).await?;
        ensure!(canonical.hash == expected_hash, "epoch is not canonical");
        let block = self
            .wallet
            .chain()
            .block(canonical.hash)
            .await?
            .context("epoch block unavailable")?;
        ensure!(
            block.block_hash() == expected_hash && block.check_merkle_root(),
            "invalid epoch block proof"
        );
        let coinbase = block.txdata.first().context("epoch has no coinbase")?;
        ensure!(
            coinbase.is_coinbase()
                && spc_scan::parse_bip34_height(coinbase.input[0].script_sig.as_bytes())
                    == Some(height),
            "epoch coinbase height mismatch"
        );
        let a_x = spc_scan::carrier_from_tx(coinbase).context("epoch has no SPc carrier")?;
        let mut compressed = [0u8; 33];
        compressed[0] = 2;
        compressed[1..].copy_from_slice(&a_x);
        let shared = spc_crypto::SharedSecret::miner_side(
            height,
            &keys.b_scan,
            &PublicKey::from_slice(&compressed)?,
        )?;
        let tweaks = spc_crypto::tier_tweaks(&shared, 0)?;
        let secret = spc_crypto::ark_user_secret(&keys.b_ark, &tweaks.t_ark)?;
        let owner = Keypair::from_secret_key(&Secp256k1::new(), &secret);
        ensure!(
            owner.public_key() == spc_crypto::ark_user_pubkey(&keys.b_ark_public(), &tweaks.t_ark)?,
            "SPc owner key parity mismatch"
        );
        let mailbox_secret = keys
            .mailbox
            .secret_key()
            .add_tweak(&spc_crypto::derived_mailbox_tweak(&shared)?)?;
        let mailbox = Keypair::from_secret_key(&Secp256k1::new(), &mailbox_secret);
        ensure!(
            mailbox.public_key()
                == spc_crypto::derived_epoch_mailbox_pubkey(&keys.mailbox_public(), &shared)?,
            "derived mailbox key parity mismatch"
        );
        let mailbox_bytes = mailbox.public_key().serialize();
        store.initialize_live_receipts()?;
        store.insert_u(&owner.public_key(), &secret, height)?;
        // Fresh direct server connection per epoch (MAILBOX section 3).
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
        let auth = MailboxAuthorization::new(
            &mailbox,
            chrono::Local::now() + std::time::Duration::from_secs(60),
        );
        let mut burst = MailboxBurst::default();
        for _ in 0..10 {
            let received = 0;
            let before = u64::try_from(store.mailbox_checkpoint(&mailbox_bytes)?)?;
            let response = server
                .mailbox_client
                .read_mailbox(protos::mailbox_server::MailboxRequest {
                    mailbox_id: auth.mailbox().serialize(),
                    authorization: Some(auth.serialize()),
                    checkpoint: before,
                })
                .await?
                .into_inner();
            let mut checkpoint = before;
            for message in response.messages {
                ensure!(
                    message.checkpoint > checkpoint,
                    "non-monotonic mailbox checkpoint"
                );
                match message.message {
                    Some(protos::mailbox_server::mailbox_message::Message::Arkoor(package)) => {
                        for raw in package.vtxos {
                            let vtxo = Vtxo::<Full>::deserialize(&raw)?;
                            ensure!(vtxo.serialize() == raw, "noncanonical VTXO encoding");
                            super::aggregate_receive::verify_output_policy(
                                &vtxo,
                                &ark::VtxoPolicy::new_pubkey(owner.public_key()),
                                self.server_pubkey,
                            )?;
                            self.wallet.validate_vtxo(&vtxo).await?;
                            let anchor_height = self
                                .wallet
                                .chain()
                                .tx_confirmed(vtxo.chain_anchor().txid)
                                .await?
                                .context("VTXO anchor not canonically confirmed")?;
                            let anchor_ref = self.wallet.chain().block_ref(anchor_height).await?;
                            let anchor_block = self
                                .wallet
                                .chain()
                                .block(anchor_ref.hash)
                                .await?
                                .context("anchor block unavailable")?;
                            ensure!(
                                anchor_block.block_hash() == anchor_ref.hash
                                    && anchor_block.check_merkle_root()
                                    && anchor_block
                                        .txdata
                                        .iter()
                                        .any(|tx| tx.compute_txid() == vtxo.chain_anchor().txid),
                                "VTXO canonical anchor proof mismatch"
                            );
                            let status = server
                                .client
                                .get_vtxo_status(protos::GetVtxoStatusRequest {
                                    vtxo_id: vtxo.id().to_bytes().to_vec(),
                                    attestation: VtxoStatusAttestation::new(vtxo.id(), &owner)
                                        .serialize(),
                                })
                                .await?
                                .into_inner();
                            ensure!(
                                status.spend_state == protos::VtxoSpendState::Spendable as i32,
                                "VTXO spendability not proven"
                            );
                            ensure!(
                                self.wallet.chain().block_ref(height.into()).await?.hash
                                    == expected_hash
                                    && self.wallet.chain().block_ref(anchor_height).await?.hash
                                        == anchor_ref.hash,
                                "chain changed during receive verification"
                            );
                            let id = vtxo.id().to_bytes();
                            store.persist_live_receipt(
                                &id,
                                &raw,
                                height,
                                &expected_hash.to_byte_array(),
                                &mailbox_bytes,
                            )?;
                            // An individually owned proof can still be only a
                            // partial aggregate. Retain it before advancing the
                            // checkpoint; only the complete signed order/receipt
                            // join may credit and allocate the covered members.
                        }
                    }
                    _ => anyhow::bail!("unexpected message in SPc epoch mailbox"),
                }
                // Raw candidates committed first. A crash before this line
                // replays idempotently from the old cursor, without a credit.
                checkpoint = message.checkpoint;
                store.advance_mailbox_checkpoint(&mailbox_bytes, i64::try_from(checkpoint)?)?;
            }
            ensure!(
                !response.have_more || checkpoint > before,
                "mailbox pagination made no progress"
            );
            if let Some(result) = burst.committed_page(received, response.have_more)? {
                return Ok(result);
            }
        }
        unreachable!("the tenth committed page returns a continuation")
    }
}
