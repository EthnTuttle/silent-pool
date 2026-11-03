//! Block assembly and merkle roots. Pure functions used by the front when a
//! share meets the network target.

/// A block header plus the transactions that follow it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssembledBlock {
    pub header: [u8; 80],
    pub block: Vec<u8>,
    pub merkle_root: [u8; 32],
}

/// `sha256d`.
pub fn txid(tx: &[u8]) -> [u8; 32] {
    use bitcoin::hashes::{sha256d, Hash};
    sha256d::Hash::hash(tx).to_byte_array()
}

/// `sha256d` of the concatenation, the inner merkle node.
fn hash_pair(a: &[u8; 32], b: &[u8; 32]) -> [u8; 32] {
    let mut buf = [0u8; 64];
    buf[..32].copy_from_slice(a);
    buf[32..].copy_from_slice(b);
    use bitcoin::hashes::{sha256d, Hash};
    sha256d::Hash::hash(&buf).to_byte_array()
}

/// Bitcoin-style merkle root; duplicate the last node on an odd level.
pub fn merkle_root(txids: &[[u8; 32]]) -> [u8; 32] {
    if txids.is_empty() {
        return [0u8; 32];
    }
    let mut level = txids.to_vec();
    while level.len() > 1 {
        if !level.len().is_multiple_of(2) {
            let last = *level.last().expect("non-empty");
            level.push(last);
        }
        level = level
            .chunks_exact(2)
            .map(|pair| hash_pair(&pair[0], &pair[1]))
            .collect();
    }
    level[0]
}

/// Merkle branch for the coinbase (leaf 0) over
/// `leaves = [coinbase] ++ txids`, in Stratum order: topmost sibling first, so
/// a miner reconstructs the root with
/// `root = H(coinbase_txid); for b in branch.rev() { root = H(root || b) }`.
///
/// The branch never depends on the coinbase txid, only on the non-coinbase
/// txs, which is why it can be built before the miner rolls the extranonce.
pub fn coinbase_merkle_branch(txids: &[[u8; 32]]) -> Vec<[u8; 32]> {
    if txids.is_empty() {
        return Vec::new();
    }
    let mut level = Vec::with_capacity(txids.len() + 1);
    // Placeholder for the coinbase: index 0 is never anyone's sibling, so its
    // value never reaches the path.
    level.push([0u8; 32]);
    level.extend_from_slice(txids);
    let mut branch = Vec::new();
    while level.len() > 1 {
        if !level.len().is_multiple_of(2) {
            let last = *level.last().expect("non-empty");
            level.push(last);
        }
        // The node we follow is always index 0, so its sibling is index 1.
        branch.push(level[1]);
        level = level
            .chunks_exact(2)
            .map(|pair| hash_pair(&pair[0], &pair[1]))
            .collect();
    }
    branch.reverse();
    branch
}

/// Assemble a block. `version` is the version the miner actually hashed (it may
/// be rolled within the BIP320 mask); `coinbase` is the fully serialized
/// coinbase (no witness); `transactions` are the non-coinbase txs in template
/// order (witness-free, see `template::tx_has_witness`).
pub fn assemble(
    version: u32,
    prev_hash: [u8; 32],
    time: u32,
    bits: u32,
    nonce: u32,
    coinbase: &[u8],
    transactions: &[Vec<u8>],
) -> AssembledBlock {
    let mut ids = Vec::with_capacity(transactions.len() + 1);
    ids.push(txid(coinbase));
    ids.extend(transactions.iter().map(|tx| txid(tx)));
    let merkle_root = merkle_root(&ids);

    let mut header = [0u8; 80];
    header[..4].copy_from_slice(&version.to_le_bytes());
    header[4..36].copy_from_slice(&prev_hash);
    header[36..68].copy_from_slice(&merkle_root);
    header[68..72].copy_from_slice(&time.to_le_bytes());
    header[72..76].copy_from_slice(&bits.to_le_bytes());
    header[76..80].copy_from_slice(&nonce.to_le_bytes());

    let mut block = Vec::with_capacity(
        80 + 9 + coinbase.len() + transactions.iter().map(Vec::len).sum::<usize>(),
    );
    block.extend_from_slice(&header);
    write_compact_size(&mut block, (transactions.len() + 1) as u64);
    // Bitcoin block serialization is a raw transaction list, not a list of
    // length-prefixed blobs.
    block.extend_from_slice(coinbase);
    for tx in transactions {
        block.extend_from_slice(tx);
    }

    AssembledBlock {
        header,
        block,
        merkle_root,
    }
}

fn write_compact_size(out: &mut Vec<u8>, n: u64) {
    match n {
        0..=0xfc => out.push(n as u8),
        0xfd..=0xffff => {
            out.push(0xfd);
            out.extend_from_slice(&(n as u16).to_le_bytes());
        }
        0x1_0000..=0xffff_ffff => {
            out.push(0xfe);
            out.extend_from_slice(&(n as u32).to_le_bytes());
        }
        _ => {
            out.push(0xff);
            out.extend_from_slice(&n.to_le_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merkle_root_matches_a_known_shape() {
        let a = [1u8; 32];
        let b = [2u8; 32];
        let c = [3u8; 32];
        // Two leaves: H(a || b).
        let ab = {
            let mut buf = [0u8; 64];
            buf[..32].copy_from_slice(&a);
            buf[32..].copy_from_slice(&b);
            use bitcoin::hashes::{sha256d, Hash};
            sha256d::Hash::hash(&buf).to_byte_array()
        };
        assert_eq!(merkle_root(&[a, b]), ab);
        // Three leaves duplicate c: H(H(a||b) || H(c||c)).
        let cc = {
            let mut buf = [0u8; 64];
            buf[..32].copy_from_slice(&c);
            buf[32..].copy_from_slice(&c);
            use bitcoin::hashes::{sha256d, Hash};
            sha256d::Hash::hash(&buf).to_byte_array()
        };
        let root = {
            let mut buf = [0u8; 64];
            buf[..32].copy_from_slice(&ab);
            buf[32..].copy_from_slice(&cc);
            use bitcoin::hashes::{sha256d, Hash};
            sha256d::Hash::hash(&buf).to_byte_array()
        };
        assert_eq!(merkle_root(&[a, b, c]), root);
    }

    #[test]
    fn assembles_a_header_and_transaction_count() {
        let coinbase = vec![0xaa; 100];
        let b = assemble(2, [0x11; 32], 1_700_000_000, 0x1d00_ffff, 7, &coinbase, &[]);
        assert_eq!(b.block.len(), 80 + 1 + 100);
        assert_eq!(&b.block[..4], &2u32.to_le_bytes());
        assert_eq!(&b.block[4..36], &[0x11; 32]);
        assert_eq!(&b.block[36..68], &b.merkle_root);
        assert_eq!(&b.block[68..72], &1_700_000_000u32.to_le_bytes());
        assert_eq!(&b.block[72..76], &0x1d00_ffffu32.to_le_bytes());
        assert_eq!(&b.block[76..80], &7u32.to_le_bytes());
        assert_eq!(b.block[80], 1); // one tx
        assert_eq!(&b.block[81..], &coinbase);
    }

    #[test]
    fn empty_merkle_root_is_zero() {
        assert_eq!(merkle_root(&[]), [0u8; 32]);
    }

    /// The branch a miner gets must rebuild the block's merkle root for any
    /// number of non-coinbase transactions, using the same fold the SV1/SV2
    /// listeners use.
    #[test]
    fn coinbase_branch_rebuilds_the_block_root() {
        let coinbase_txid = [0xc0u8; 32];
        for txids in [
            vec![[1u8; 32]],
            vec![[1u8; 32], [2u8; 32]],
            vec![[1u8; 32], [2u8; 32], [3u8; 32]],
            vec![[1u8; 32], [2u8; 32], [3u8; 32], [4u8; 32], [5u8; 32]],
        ] {
            let branch = coinbase_merkle_branch(&txids);
            let mut all = vec![coinbase_txid];
            all.extend_from_slice(&txids);
            let expected = merkle_root(&all);

            // Miner-side fold: coinbase txid, then the branch bottom-up.
            let mut root = coinbase_txid;
            for sibling in branch.iter().rev() {
                root = hash_pair(&root, sibling);
            }
            assert_eq!(root, expected, "{} txs", txids.len());
        }
    }

    #[test]
    fn assemble_uses_the_submitted_version() {
        let coinbase = vec![0xaa; 10];
        let rolled = 0x2000_0000u32 ^ 0x0000_2000;
        let b = assemble(rolled, [0x11; 32], 1, 0x207f_ffff, 3, &coinbase, &[]);
        assert_eq!(&b.block[..4], &rolled.to_le_bytes());
    }
}
