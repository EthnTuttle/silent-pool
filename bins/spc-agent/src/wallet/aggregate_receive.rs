//! Verify complete aggregate delivery before its single ledger/allocation commit.
//! The order-binding API dependency is explicit; SPRP signatures alone never
//! authenticate arbitrary later VTXO lists.
use super::aggregate_watch::AggregateOrder;
use super::live_bark::LiveBark;
use super::receive::{Expiry, ExpirySource, VtxoState};
use super::{VtxoRecord, WalletKeys, WalletStore};
use anyhow::{ensure, Context};
use ark::vtxo::{Full, Vtxo};
use ark::{ProtocolEncoding, VtxoPolicy};
use bitcoin::hashes::Hash;
use bitcoin::secp256k1::{Keypair, PublicKey, Secp256k1, XOnlyPublicKey};
use bitcoin::{Block, BlockHash};
use rusqlite::{params, OptionalExtension};
use spc_ext::{
    authorization::{order_digest, ProvedReceipt},
    ArkSendOrder,
};

pub(super) fn verify_order_envelope(
    status: &AggregateOrder,
    order: &ArkSendOrder,
    receipt: &ProvedReceipt,
    key: &XOnlyPublicKey,
) -> anyhow::Result<()> {
    order.verify(key)?;
    receipt.grant.verify_signature(key)?;
    ensure!(
        status.state == "sent"
            && status.order_id == order.order_id
            && receipt.grant.request.order_id == order.order_id,
        "aggregate receipt/order association mismatch"
    );
    ensure!(
        order_digest(order)? == receipt.grant.request.order_digest,
        "aggregate frozen order digest mismatch"
    );
    ensure!(
        order.sats == status.sats && receipt.sats == status.sats && receipt.sats > 0,
        "aggregate partial or excessive value"
    );
    ensure!(
        receipt.grant.request.nonce != [0; 16]
            && receipt.grant.revision > 0
            && receipt.grant.revision < status.revision
            && receipt.grant.expires_at > 0,
        "aggregate grant revision/expiry invalid"
    );
    // A historical receipt is not permission to send now. Its signed grant
    // may legitimately have expired while the wallet was offline.
    Ok(())
}

pub(super) fn verify_output_policy(
    vtxo: &Vtxo<Full>,
    policy: &VtxoPolicy,
    server: PublicKey,
) -> anyhow::Result<()> {
    ensure!(
        vtxo.policy() == policy,
        "aggregate full output policy mismatch"
    );
    ensure!(
        vtxo.server_pubkey() == server,
        "aggregate intended server mismatch"
    );
    Ok(())
}

/// Structural receipt-set checks precede all chain/server work. The caller
/// must still validate every genesis and canonical funding proof before credit.
pub(super) fn decode_receipt_outputs(
    receipt: &ProvedReceipt,
    policy: &VtxoPolicy,
    server: PublicKey,
) -> anyhow::Result<Vec<Vtxo<Full>>> {
    ensure!(
        !receipt.vtxos.is_empty() && receipt.vtxos.len() <= 2048,
        "aggregate VTXO count invalid"
    );
    let mut seen = std::collections::BTreeSet::new();
    let mut total = 0u64;
    let mut outputs = Vec::with_capacity(receipt.vtxos.len());
    for raw in &receipt.vtxos {
        let vtxo = Vtxo::<Full>::deserialize(raw)?;
        ensure!(vtxo.serialize() == *raw, "noncanonical aggregate VTXO");
        verify_output_policy(&vtxo, policy, server)?;
        ensure!(seen.insert(vtxo.id()), "duplicate aggregate VTXO");
        let amount = vtxo.amount().to_sat();
        ensure!(
            amount > 0 && i64::try_from(amount).is_ok(),
            "aggregate VTXO amount invalid"
        );
        total = total
            .checked_add(amount)
            .context("aggregate VTXO sum overflow")?;
        outputs.push(vtxo);
    }
    ensure!(
        total == receipt.sats,
        "aggregate delivery incomplete or excessive"
    );
    Ok(outputs)
}

impl LiveBark {
    pub(super) async fn receive_aggregate(
        &self,
        keys: &WalletKeys,
        store: &WalletStore,
        status: &AggregateOrder,
        receipt: &ProvedReceipt,
        order_key: &XOnlyPublicKey,
    ) -> anyhow::Result<()> {
        let order = store.frozen_aggregate_order(&status.order_id)?.context(
            "aggregate_frozen_order_unavailable: authenticated order endpoint unavailable; retain watch",
        )?;
        verify_order_envelope(status, &order, receipt, order_key)?;
        self.check_connection().await?;
        self.wallet.chain().invalidate_caches().await;
        let hash = BlockHash::from_byte_array(status.anchor_hash);
        // Sent/possibly-sent origin blocks can now be orphaned. Fetch the REAL
        // stored anchor by hash, verify its coinbase/Merkle/height, and derive
        // its owned key. Never replace it with a current-height destination.
        let block = self
            .wallet
            .chain()
            .block(hash)
            .await?
            .context("aggregate anchor block proof unavailable")?;
        let (owner, mailbox) = anchor_keys(keys, &block, status.anchor_height, hash)?;
        let expected = VtxoPolicy::new_pubkey(owner.public_key());
        let destination: ark::Address = order.ark_address.parse()?;
        ensure!(
            destination.is_testnet() != matches!(keys.network, spc_crypto::Network::Mainnet)
                && destination.is_for_server(self.server_pubkey)
                && destination.policy() == &expected,
            "frozen anchor destination mismatch"
        );
        let blinded = ark::mailbox::BlindedMailboxIdentifier::new(
            ark::mailbox::MailboxIdentifier::from_pubkey(mailbox),
            self.mailbox_pubkey,
            &owner,
        )?;
        ensure!(
            destination.delivery()
                == [ark::address::VtxoDelivery::ServerMailbox {
                    blinded_id: blinded
                }],
            "frozen aggregate mailbox mismatch"
        );
        let outputs = decode_receipt_outputs(receipt, &expected, self.server_pubkey)?;
        let mut records = vec![];
        let mut funding_anchors = std::collections::BTreeMap::new();
        let mut server = server_rpc::ServerConnection::builder()
            .address(&self.endpoint)
            .network(super::keys::bitcoin_network(keys.network))
            .connect()
            .await?;
        let info = server.ark_info().await;
        ensure!(
            info.server_pubkey == self.server_pubkey && info.mailbox_pubkey == self.mailbox_pubkey,
            "wrong aggregate Ark server"
        );
        for (vtxo, raw) in outputs.iter().zip(&receipt.vtxos) {
            self.wallet.validate_vtxo(vtxo).await?;
            let funding_height = self
                .wallet
                .chain()
                .tx_confirmed(vtxo.chain_anchor().txid)
                .await?
                .context("aggregate VTXO funding unconfirmed")?;
            let anchor = self.wallet.chain().block_ref(funding_height).await?;
            if let Some(prior) = funding_anchors.insert(funding_height, anchor.hash) {
                ensure!(
                    prior == anchor.hash,
                    "aggregate funding changed between proofs"
                );
            }
            let funding = self
                .wallet
                .chain()
                .block(anchor.hash)
                .await?
                .context("aggregate funding block unavailable")?;
            ensure!(
                funding.block_hash() == anchor.hash
                    && funding.check_merkle_root()
                    && funding
                        .txdata
                        .iter()
                        .any(|t| t.compute_txid() == vtxo.chain_anchor().txid),
                "aggregate canonical funding proof mismatch"
            );
            let observed = server
                .client
                .get_vtxo_status(server_rpc::protos::GetVtxoStatusRequest {
                    vtxo_id: vtxo.id().to_bytes().to_vec(),
                    attestation: ark::attestations::VtxoStatusAttestation::new(vtxo.id(), &owner)
                        .serialize(),
                })
                .await?
                .into_inner();
            ensure!(
                observed.spend_state == server_rpc::protos::VtxoSpendState::Spendable as i32,
                "aggregate VTXO spendability unknown"
            );
            ensure!(
                self.wallet.chain().block_ref(funding_height).await?.hash == anchor.hash,
                "aggregate funding displaced during validation"
            );
            records.push((
                VtxoRecord {
                    vtxo_id: vtxo.id().to_bytes(),
                    u_pubkey: owner.public_key(),
                    amount_sats: vtxo.amount().to_sat(),
                    state: VtxoState::Spendable,
                    expiry: Expiry::At {
                        height: vtxo.expiry_height().into(),
                        source: ExpirySource::VtxoAncestry,
                    },
                },
                raw.clone(),
            ));
        }
        if let Some(id) = &status.vtxo_id {
            let primary = records
                .iter()
                .find(|(r, _)| r.vtxo_id.as_slice() == id)
                .context("aggregate primary receipt missing")?;
            ensure!(
                status
                    .expiry_height
                    .is_none_or(|h| primary.0.expiry.height() == Some(h)),
                "aggregate expiry hint mismatch"
            );
        }
        // Recheck the complete funding set together, after the last network
        // proof/status read. A split delivery must not retain a stale first
        // anchor while later proofs are being fetched.
        self.wallet.chain().invalidate_caches().await;
        for (height, hash) in funding_anchors {
            ensure!(
                self.wallet.chain().block_ref(height).await?.hash == hash,
                "aggregate funding set displaced before commit"
            );
        }
        store.insert_u(
            &owner.public_key(),
            &owner.secret_key(),
            status.anchor_height,
        )?;
        store.commit_verified_aggregate(
            status,
            &receipt.encode()?,
            &records,
            &mailbox.serialize(),
            &bitcoin::consensus::serialize(&block),
        )?;
        Ok(())
    }
}

fn anchor_keys(
    keys: &WalletKeys,
    block: &Block,
    height: u32,
    hash: BlockHash,
) -> anyhow::Result<(Keypair, PublicKey)> {
    ensure!(
        block.block_hash() == hash && block.check_merkle_root(),
        "aggregate anchor Merkle/hash mismatch"
    );
    let coinbase = block
        .txdata
        .first()
        .context("aggregate anchor coinbase missing")?;
    ensure!(
        coinbase.is_coinbase()
            && spc_scan::parse_bip34_height(coinbase.input[0].script_sig.as_bytes())
                == Some(height),
        "aggregate anchor height mismatch"
    );
    let ax = spc_scan::carrier_from_tx(coinbase).context("aggregate anchor SPc carrier missing")?;
    let mut bytes = [0u8; 33];
    bytes[0] = 2;
    bytes[1..].copy_from_slice(&ax);
    let shared = spc_crypto::SharedSecret::miner_side(
        height,
        &keys.b_scan,
        &PublicKey::from_slice(&bytes)?,
    )?;
    let owner = Keypair::from_secret_key(
        &Secp256k1::new(),
        &spc_crypto::ark_user_secret(&keys.b_ark, &spc_crypto::tier_tweaks(&shared, 0)?.t_ark)?,
    );
    Ok((
        owner,
        spc_crypto::derived_epoch_mailbox_pubkey(&keys.mailbox_public(), &shared)?,
    ))
}

impl WalletStore {
    /// Only the fully validated receipt path can call this. One transaction
    /// attaches every covered member and credits each unique VTXO once using
    /// the existing authoritative ledger implementation.
    pub(super) fn commit_verified_aggregate(
        &self,
        order: &AggregateOrder,
        receipt: &[u8],
        records: &[(VtxoRecord, Vec<u8>)],
        mailbox: &[u8; 33],
        origin: &[u8],
    ) -> anyhow::Result<()> {
        self.initialize_live_receipts()?;
        self.init_aggregate_watches()?;
        let mut guard = self.lock();
        let tx = guard.transaction()?;
        let (stored, previous): (Vec<u8>, Option<Vec<u8>>) = tx.query_row(
            "SELECT status,verified_receipt FROM aggregate_client_orders WHERE order_id=?1",
            params![order.order_id.as_slice()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        ensure!(
            stored == serde_json::to_vec(order)?,
            "aggregate changed during receipt verification"
        );
        if let Some(previous) = previous {
            ensure!(previous == receipt, "aggregate receipt replay mismatch");
            return Ok(());
        }
        let mut seen = std::collections::BTreeSet::new();
        let mut total = 0u64;
        for (record, raw) in records {
            ensure!(seen.insert(record.vtxo_id), "duplicate aggregate credit");
            total = total
                .checked_add(record.amount_sats)
                .context("aggregate credit overflow")?;
            let allocated: Option<Vec<u8>> = tx
                .query_row(
                    "SELECT order_id FROM aggregate_vtxo_allocation WHERE vtxo_id=?1",
                    params![record.vtxo_id.as_slice()],
                    |r| r.get(0),
                )
                .optional()?;
            ensure!(
                allocated.is_none_or(|id| id == order.order_id),
                "VTXO already allocated to another aggregate"
            );
            let old_raw: Option<Vec<u8>> = tx
                .query_row(
                    "SELECT raw FROM live_receipts WHERE vtxo_id=?1",
                    params![record.vtxo_id.as_slice()],
                    |r| r.get(0),
                )
                .optional()?;
            ensure!(
                old_raw.is_none_or(|b| b == *raw),
                "aggregate/mailbox receipt bytes disagree"
            );
            tx.execute("INSERT OR IGNORE INTO live_receipts(vtxo_id,raw,epoch_height,epoch_hash,mailbox) VALUES(?1,?2,?3,?4,?5)",params![record.vtxo_id.as_slice(),raw,order.anchor_height,order.anchor_hash.as_slice(),mailbox.as_slice()])?;
            self.upsert_vtxo_in_transaction(&tx, record)?;
            tx.execute(
                "INSERT OR IGNORE INTO aggregate_vtxo_allocation(vtxo_id,order_id) VALUES(?1,?2)",
                params![record.vtxo_id.as_slice(), order.order_id.as_slice()],
            )?;
        }
        ensure!(
            total == order.sats && !records.is_empty(),
            "partial aggregate credit"
        );
        for member in &order.members {
            let previous: Option<Vec<u8>> = tx.query_row(
                "SELECT verified_order FROM aggregate_watches WHERE block_hash=?1",
                params![member.block_hash.as_slice()],
                |r| r.get(0),
            )?;
            ensure!(
                previous.is_none_or(|id| id == order.order_id),
                "member already delivered by another aggregate"
            );
            let n=tx.execute("UPDATE aggregate_watches SET verified_order=?2 WHERE block_hash=?1 AND current_order=?2 AND owed_sats=?3",params![member.block_hash.as_slice(),order.order_id.as_slice(),member.sats.to_string()])?;
            ensure!(n == 1, "aggregate member allocation changed");
            tx.execute(
                "UPDATE live_scan_blocks SET closed=1 WHERE height=?1 AND hash=?2",
                params![member.height, member.block_hash.as_slice()],
            )?;
        }
        tx.execute(
            "UPDATE aggregate_client_orders SET verified_receipt=?2 WHERE order_id=?1",
            params![order.order_id.as_slice(), receipt],
        )?;
        tx.execute(
            "INSERT OR REPLACE INTO aggregate_origin_proofs(block_hash,block) VALUES(?1,?2)",
            params![order.anchor_hash.as_slice(), origin],
        )?;
        tx.commit()?;
        Ok(())
    }
}
