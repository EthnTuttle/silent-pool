//! Canonical node-driven scan cursor and replayable mailbox schedule. Rewinds
//! are journaled in the existing wallet SQLite transaction before replay.
use anyhow::{ensure, Context};
use bitcoin::hashes::Hash;
use bitcoin::BlockHash;
use rusqlite::params;

use super::live_bark::LiveBark;
use super::{ReceiveLedger, WalletKeys, WalletStore};

impl WalletStore {
    pub fn initialize_live_scan(&self) -> anyhow::Result<()> {
        self.initialize_live_receipts()?;
        self.init_aggregate_watches()?;
        self.lock().execute_batch(
            "CREATE TABLE IF NOT EXISTS live_scan_blocks (
            height INTEGER PRIMARY KEY, hash BLOB NOT NULL CHECK(length(hash)=32),
            epoch INTEGER NOT NULL, reads INTEGER NOT NULL DEFAULT 0,
            closed INTEGER NOT NULL DEFAULT 0);
            CREATE TABLE IF NOT EXISTS live_scan_rewinds (
            seq INTEGER PRIMARY KEY AUTOINCREMENT, old_height INTEGER NOT NULL,
            old_hash BLOB NOT NULL, fork_height INTEGER NOT NULL, fork_hash BLOB NOT NULL);
            CREATE TABLE IF NOT EXISTS live_epoch_continuations (
                height INTEGER PRIMARY KEY REFERENCES live_scan_blocks(height) ON DELETE CASCADE);
            CREATE TABLE IF NOT EXISTS live_scan_schedule (
                id INTEGER PRIMARY KEY CHECK(id=1), last_epoch INTEGER NOT NULL);
            INSERT OR IGNORE INTO live_scan_schedule(id,last_epoch) VALUES(1,0);",
        )?;
        if self.last_scan_height()?.is_some() && self.live_scan_blocks()?.is_empty() {
            anyhow::bail!("scan cursor lacks canonical hash history; refusing guessed rescan");
        }
        self.lock().execute("UPDATE live_scan_blocks SET closed=0 WHERE epoch=1 AND closed=1 AND NOT EXISTS
            (SELECT 1 FROM aggregate_watches w WHERE w.block_hash=live_scan_blocks.hash AND w.verified_order IS NOT NULL)",[])?;
        Ok(())
    }

    pub fn live_scan_blocks(&self) -> anyhow::Result<Vec<(u32, BlockHash)>> {
        let guard = self.lock();
        let mut stmt =
            guard.prepare("SELECT height, hash FROM live_scan_blocks ORDER BY height")?;
        let mut rows = stmt.query([])?;
        let mut out = vec![];
        while let Some(row) = rows.next()? {
            let height: u32 = row.get(0)?;
            let bytes: Vec<u8> = row.get(1)?;
            out.push((height, BlockHash::from_slice(&bytes)?));
        }
        Ok(out)
    }

    pub fn commit_scan_block(
        &self,
        height: u32,
        hash: BlockHash,
        epoch: bool,
    ) -> anyhow::Result<()> {
        self.commit_scan_block_with_outputs(height, hash, epoch, &[])
    }

    pub(super) fn commit_scan_block_with_outputs(
        &self,
        height: u32,
        hash: BlockHash,
        epoch: bool,
        outputs: &[spc_scan::Found],
    ) -> anyhow::Result<()> {
        let mut guard = self.lock();
        let tx = guard.transaction()?;
        let previous: Option<u32> =
            tx.query_row("SELECT MAX(height) FROM live_scan_blocks", [], |r| r.get(0))?;
        ensure!(
            height
                == previous
                    .unwrap_or(0)
                    .checked_add(1)
                    .context("scan height exhausted")?,
            "noncontiguous scan commit"
        );
        tx.execute(
            "INSERT INTO live_scan_blocks(height, hash, epoch) VALUES (?1, ?2, ?3)",
            params![height, hash.to_byte_array().as_slice(), epoch],
        )?;
        if epoch {
            tx.execute(
                "INSERT OR IGNORE INTO aggregate_watches(block_hash,height) VALUES(?1,?2)",
                params![hash.to_byte_array().as_slice(), height],
            )?;
        }
        for output in outputs {
            ensure!(
                epoch && output.height <= height && height - output.height <= 1,
                "coinbase output derivation height mismatch"
            );
            let changed=tx.execute("INSERT INTO owned_coinbase_outputs(block_hash,height,derivation_height,k,vout,sats,tweak) VALUES(?1,?2,?3,?4,?5,?6,?7)
                ON CONFLICT(block_hash,derivation_height,k) DO UPDATE SET canonical=1
                WHERE height=excluded.height AND vout=excluded.vout AND sats=excluded.sats AND tweak=excluded.tweak",
                params![hash.to_byte_array().as_slice(),height,output.height,output.k,output.vout,output.amount.to_string(),output.tweak.as_slice()])?;
            ensure!(changed == 1, "replayed owned output changed");
        }
        let stored_outputs: usize = tx.query_row(
            "SELECT count(*) FROM owned_coinbase_outputs WHERE block_hash=?1",
            params![hash.to_byte_array().as_slice()],
            |r| r.get(0),
        )?;
        ensure!(
            stored_outputs == outputs.len(),
            "replayed owned output set changed"
        );
        tx.execute("UPDATE live_scan_blocks SET closed=1 WHERE hash=?1 AND EXISTS
            (SELECT 1 FROM aggregate_watches w WHERE w.block_hash=?1 AND w.verified_order IS NOT NULL)",params![hash.to_byte_array().as_slice()])?;
        tx.execute(
            "INSERT INTO scan_cursor(id,last_height) VALUES (1,?1)
            ON CONFLICT(id) DO UPDATE SET last_height=excluded.last_height",
            params![height],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn rewind_live_scan(&self, fork_height: u32, fork_hash: BlockHash) -> anyhow::Result<()> {
        let mut guard = self.lock();
        let tx = guard.transaction()?;
        let (old_height, old_hash): (u32, Vec<u8>) = tx.query_row(
            "SELECT height,hash FROM live_scan_blocks ORDER BY height DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        ensure!(fork_height < old_height, "rewind must move backwards");
        if fork_height > 0 {
            let stored: Vec<u8> = tx.query_row(
                "SELECT hash FROM live_scan_blocks WHERE height=?1",
                [fork_height],
                |r| r.get(0),
            )?;
            ensure!(
                stored == fork_hash.to_byte_array(),
                "fork point not in scan history"
            );
        }
        tx.execute("INSERT INTO live_scan_rewinds(old_height,old_hash,fork_height,fork_hash) VALUES (?1,?2,?3,?4)", params![old_height, old_hash, fork_height, fork_hash.to_byte_array().as_slice()])?;
        tx.execute(
            "DELETE FROM live_scan_blocks WHERE height>?1",
            [fork_height],
        )?;
        tx.execute(
            "UPDATE owned_coinbase_outputs SET canonical=0 WHERE height>?1",
            [fork_height],
        )?;
        // Aggregate watches and frozen covered-set associations survive reorgs.
        // Retain receipts and keys. A reorg invalidates proof freshness, not
        // ownership or amounts. Revalidation is required to restore spendability.
        tx.execute("UPDATE vtxo_records SET state='pending' WHERE state='spendable' AND vtxo_id IN (SELECT vtxo_id FROM live_receipts)", [])?;
        if fork_height == 0 {
            tx.execute("DELETE FROM scan_cursor WHERE id=1", [])?;
        } else {
            tx.execute(
                "UPDATE scan_cursor SET last_height=?1 WHERE id=1",
                params![fork_height],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub(super) fn due_epochs(&self, tip: u32) -> anyhow::Result<Vec<(u32, BlockHash, usize)>> {
        let guard = self.lock();
        // Validate arithmetic even for rows that the due predicate would hide.
        let invalid: bool = guard.query_row("SELECT EXISTS(SELECT 1 FROM live_scan_blocks
            WHERE epoch=1 AND closed=0 AND (reads NOT BETWEEN 0 AND 4 OR height NOT BETWEEN 1 AND 4294967295
            OR height+6+CASE reads WHEN 0 THEN 0 WHEN 1 THEN 6 WHEN 2 THEN 36 WHEN 3 THEN 144 WHEN 4 THEN 1008 END > 4294967295))",[],|r|r.get(0))?;
        ensure!(!invalid, "invalid mailbox schedule or height overflow");
        let mut stmt = guard.prepare("SELECT height,hash,reads FROM live_scan_blocks
            WHERE epoch=1 AND closed=0 AND NOT EXISTS(
              SELECT 1 FROM aggregate_watches w JOIN aggregate_client_orders o ON o.order_id=w.current_order
              WHERE w.block_hash=live_scan_blocks.hash AND o.state!='cancelled' AND o.anchor_hash!=w.block_hash)
            AND (
              height IN (SELECT height FROM live_epoch_continuations)
              OR height+6+CASE reads WHEN 0 THEN 0 WHEN 1 THEN 6 WHEN 2 THEN 36 WHEN 3 THEN 144 WHEN 4 THEN 1008 END <= ?1)
            ORDER BY height <= (SELECT last_epoch FROM live_scan_schedule WHERE id=1),height LIMIT 128")?;
        let mut rows = stmt.query([tip])?;
        let mut out = vec![];
        while let Some(row) = rows.next()? {
            let height: u32 = row.get(0)?;
            let bytes: Vec<u8> = row.get(1)?;
            let reads: usize = row.get(2)?;
            out.push((height, BlockHash::from_slice(&bytes)?, reads));
        }
        Ok(out)
    }

    pub(super) fn begin_epoch_read(&self, height: u32, reads: usize) -> anyhow::Result<()> {
        let mut guard = self.lock();
        let tx = guard.transaction()?;
        let present:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM live_scan_blocks WHERE height=?1 AND reads=?2 AND closed=0)",params![height,reads],|r|r.get(0))?;
        ensure!(present, "mailbox schedule changed concurrently");
        tx.execute(
            "INSERT OR IGNORE INTO live_epoch_continuations(height) VALUES(?1)",
            [height],
        )?;
        tx.execute(
            "UPDATE live_scan_schedule SET last_epoch=?1 WHERE id=1",
            [height],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub(super) fn record_epoch_read(
        &self,
        height: u32,
        reads: usize,
        result: super::live_receive::EpochReadResult,
    ) -> anyhow::Result<()> {
        if result.continuation {
            return Ok(());
        }
        let mut guard = self.lock();
        let tx = guard.transaction()?;
        // Only atomic aggregate allocation closes a covered watch. A mailbox
        // proof or a completed pagination pass alone does not settle a debt.
        let arrived:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM aggregate_watches w
            JOIN live_scan_blocks b ON b.hash=w.block_hash WHERE b.height=?1 AND w.verified_order IS NOT NULL)",[height],|r|r.get(0))?;
        let changed = tx.execute(
            "UPDATE live_scan_blocks SET reads=MIN(reads+1,4),closed=?1 WHERE height=?2 AND reads=?3 AND closed=0",
            params![arrived, height, reads])?;
        ensure!(changed == 1, "mailbox schedule changed concurrently");
        tx.execute(
            "DELETE FROM live_epoch_continuations WHERE height=?1",
            [height],
        )?;
        tx.commit()?;
        Ok(())
    }
}

impl LiveBark {
    pub async fn scan_tick(
        &self,
        keys: &WalletKeys,
        store: &WalletStore,
        ledger: &ReceiveLedger,
    ) -> anyhow::Result<()> {
        store.initialize_live_scan()?;
        self.wallet.chain().invalidate_caches().await;
        let tip = self.wallet.chain().tip_ref().await?;
        let tip_height: u32 = tip.height.into();
        let history = store.live_scan_blocks()?;
        if let Some(&(height, hash)) = history.last() {
            if height > tip_height
                || self.wallet.chain().block_ref(height.into()).await?.hash != hash
            {
                let mut fork = (0, self.wallet.chain().block_ref(0u32.into()).await?.hash);
                for &(h, old) in history.iter().rev() {
                    if h <= tip_height && self.wallet.chain().block_ref(h.into()).await?.hash == old
                    {
                        fork = (h, old);
                        break;
                    }
                }
                store.rewind_live_scan(fork.0, fork.1)?;
            }
        }
        let mut height = store.last_scan_height()?.unwrap_or(0);
        for _ in 0..128 {
            if height >= tip_height {
                break;
            }
            let next = height.checked_add(1).context("scan exhausted")?;
            let block_ref = self.wallet.chain().block_ref(next.into()).await?;
            let block = self
                .wallet
                .chain()
                .block(block_ref.hash)
                .await?
                .context("scan block unavailable")?;
            ensure!(
                block.block_hash() == block_ref.hash && block.check_merkle_root(),
                "invalid canonical scan block"
            );
            let prev = self.wallet.chain().block_ref(height.into()).await?;
            if height > 0 {
                let stored: Vec<u8> = store.lock().query_row(
                    "SELECT hash FROM live_scan_blocks WHERE height=?1",
                    [height],
                    |r| r.get(0),
                )?;
                ensure!(
                    stored == prev.hash.to_byte_array(),
                    "scan history displaced during replay"
                );
            }
            ensure!(
                block.header.prev_blockhash == prev.hash
                    && self.wallet.chain().block_ref(next.into()).await?.hash == block_ref.hash,
                "chain changed during scan"
            );
            let scan = spc_scan::scan_block(&block, &keys.b_scan, &keys.b_spend_public)?;
            store.commit_scan_block_with_outputs(next, block_ref.hash, scan.is_spc, &scan.found)?;
            height = next;
        }
        for (epoch, hash, reads) in store.due_epochs(tip_height)? {
            store.begin_epoch_read(epoch, reads)?;
            let received = self.receive_epoch(keys, store, ledger, epoch, hash).await?;
            store.record_epoch_read(epoch, reads, received)?;
        }
        self.revalidate_receipts(keys, store, ledger).await?;
        Ok(())
    }
}
