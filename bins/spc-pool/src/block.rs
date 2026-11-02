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
            .map(|pair| {
                let mut buf = [0u8; 64];
                buf[..32].copy_from_slice(&pair[0]);
                buf[32..].copy_from_slice(&pair[1]);
                use bitcoin::hashes::{sha256d, Hash};
                sha256d::Hash::hash(&buf).to_byte_array()
            })
            .collect();
    }
    level[0]
}

/// Assemble a block. `coinbase` is the fully serialized coinbase (including
/// any witness appended by the caller); `transactions` are the non-coinbase
/// txs in template order.
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
}
