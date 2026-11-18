//! Consensus chain identity, deliberately separate from address/key encoding.
//!
//! `Network` is the historical SP encoding selector. A `tsp`, `tb` or tpub
//! value cannot distinguish Testnet3, Testnet4, Testnet5 or signet. Never
//! pass `address_encoding_network()` to a chain backend or a stock Ark wallet.

use bitcoin::{hashes::Hash, BlockHash};

use crate::Network;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChainNetwork {
    Mainnet,
    Testnet3,
    Testnet4,
    Signet,
    Regtest,
    Testnet5,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ChainError {
    #[error("unknown or ambiguous chain name")]
    Unknown,
    #[error("Bitcoin endpoint genesis does not match configured chain")]
    WrongGenesis,
}

impl ChainNetwork {
    /// Canonical names only. Historical context-specific `testnet` aliases
    /// belong at their old config boundary, never in this parser.
    pub fn parse(name: &str) -> Result<Self, ChainError> {
        match name {
            "mainnet" => Ok(Self::Mainnet),
            "testnet3" => Ok(Self::Testnet3),
            "testnet4" => Ok(Self::Testnet4),
            "signet" => Ok(Self::Signet),
            "regtest" => Ok(Self::Regtest),
            "testnet5" => Ok(Self::Testnet5),
            _ => Err(ChainError::Unknown),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Mainnet => "mainnet",
            Self::Testnet3 => "testnet3",
            Self::Testnet4 => "testnet4",
            Self::Signet => "signet",
            Self::Regtest => "regtest",
            Self::Testnet5 => "testnet5",
        }
    }

    /// Existing SQL/wire codes 0..3 retain their specified meanings.
    pub fn code(self) -> i16 {
        match self {
            Self::Mainnet => 0,
            Self::Testnet4 => 1,
            Self::Signet => 2,
            Self::Regtest => 3,
            Self::Testnet5 => 4,
            Self::Testnet3 => 5,
        }
    }

    pub fn from_code(code: i16) -> Result<Self, ChainError> {
        match code {
            0 => Ok(Self::Mainnet),
            1 => Ok(Self::Testnet4),
            2 => Ok(Self::Signet),
            3 => Ok(Self::Regtest),
            4 => Ok(Self::Testnet5),
            5 => Ok(Self::Testnet3),
            _ => Err(ChainError::Unknown),
        }
    }

    /// Only SP HRP/BIP32 encoding compatibility; not a consensus conversion.
    pub fn sp_encoding(self) -> Network {
        match self {
            Self::Mainnet => Network::Mainnet,
            Self::Testnet3 | Self::Testnet4 | Self::Testnet5 => Network::Testnet,
            Self::Signet => Network::Signet,
            Self::Regtest => Network::Regtest,
        }
    }

    /// Address/descriptor serialization adapter ONLY. Testnet5 uses `tb`
    /// and test-family Base58/BIP32 prefixes; this does not select its chain.
    pub fn address_encoding_network(self) -> bitcoin::Network {
        match self {
            Self::Mainnet => bitcoin::Network::Bitcoin,
            Self::Testnet3 | Self::Testnet4 | Self::Testnet5 => bitcoin::Network::Testnet,
            Self::Signet => bitcoin::Network::Signet,
            Self::Regtest => bitcoin::Network::Regtest,
        }
    }

    /// Actual stock-backend selector. Testnet5 requires the adopted
    /// rust-bitcoin/bark forks (UPSTREAM_DEPS.md "Adopted forks
    /// (2026-11-17)"; decision 2026-11-17-bark-testnet5-fork.md).
    pub fn stock_bitcoin_network(self) -> Result<bitcoin::Network, ChainError> {
        match self {
            Self::Mainnet => Ok(bitcoin::Network::Bitcoin),
            Self::Testnet3 => Ok(bitcoin::Network::Testnet),
            Self::Testnet4 => Ok(bitcoin::Network::Testnet4),
            Self::Signet => Ok(bitcoin::Network::Signet),
            Self::Regtest => Ok(bitcoin::Network::Regtest),
            Self::Testnet5 => Ok(bitcoin::Network::Testnet5),
        }
    }

    pub fn genesis(self) -> BlockHash {
        if self == Self::Testnet5 {
            // Mined BIP95 genesis, Core ee505609 / BIP PR f82c4612.
            "00000000000008c1637e7d3252dc1c333d4546fd6fff6a474b200d674a91d497"
                .parse()
                .expect("fixed BIP95 genesis hash")
        } else {
            bitcoin::blockdata::constants::genesis_block(
                self.stock_bitcoin_network()
                    .expect("supported historical chain"),
            )
            .block_hash()
        }
    }

    /// Compare typed hashes (internal byte order), not HRP or an RPC label.
    pub fn validate_genesis(self, observed: BlockHash) -> Result<(), ChainError> {
        if observed == self.genesis() {
            Ok(())
        } else {
            Err(ChainError::WrongGenesis)
        }
    }

    pub fn genesis_bytes(self) -> [u8; 32] {
        self.genesis().to_byte_array()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_address_prefix_does_not_authorize_another_chain() {
        let a = ChainNetwork::Testnet4;
        let b = ChainNetwork::Testnet5;
        let keys = crate::provision(&[0x42; 64], a.sp_encoding()).unwrap();
        // Same source keys and derivation bytes on both test chains.
        let other = crate::provision(&[0x42; 64], b.sp_encoding()).unwrap();
        assert_eq!(keys.scan_branch, other.scan_branch);
        assert_eq!(keys.spend_branch, other.spend_branch);
        let secp = bitcoin::secp256k1::Secp256k1::new();
        let scan = bitcoin::secp256k1::PublicKey::from_secret_key(
            &secp,
            &crate::spc_branch_secret(&keys.scan_branch).unwrap(),
        );
        let spend = bitcoin::secp256k1::PublicKey::from_secret_key(
            &secp,
            &crate::spc_branch_secret(&keys.spend_branch).unwrap(),
        );
        let address = crate::address::encode_sp_address(a.sp_encoding(), &scan, &spend).unwrap();
        assert_eq!(
            address,
            crate::address::encode_sp_address(b.sp_encoding(), &scan, &spend).unwrap()
        );
        assert!(crate::parse_sp_address(&address, b.sp_encoding()).is_ok());
        assert_eq!(a.address_encoding_network(), b.address_encoding_network());
        assert_eq!(
            b.validate_genesis(a.genesis()),
            Err(ChainError::WrongGenesis)
        );
        assert_eq!(
            a.validate_genesis(b.genesis()),
            Err(ChainError::WrongGenesis)
        );
        assert!(b.validate_genesis(b.genesis()).is_ok());
        assert_eq!(
            b.stock_bitcoin_network().unwrap(),
            bitcoin::Network::Testnet5
        );
        // The adopted fork's own genesis must match the pinned BIP95
        // constant; identity never comes from the dependency alone.
        assert_eq!(
            bitcoin::blockdata::constants::genesis_block(bitcoin::Network::Testnet5).block_hash(),
            b.genesis()
        );
        assert_eq!(
            a.stock_bitcoin_network().unwrap(),
            bitcoin::Network::Testnet4
        );
    }

    #[test]
    fn all_chain_pairs_and_legacy_codes_are_distinct() {
        let chains = [
            ChainNetwork::Mainnet,
            ChainNetwork::Testnet4,
            ChainNetwork::Signet,
            ChainNetwork::Regtest,
            ChainNetwork::Testnet5,
            ChainNetwork::Testnet3,
        ];
        for (code, chain) in chains.iter().copied().enumerate() {
            assert_eq!(chain.code(), code as i16);
            assert_eq!(ChainNetwork::from_code(code as i16), Ok(chain));
            assert_eq!(ChainNetwork::parse(chain.name()), Ok(chain));
            for other in chains {
                assert_eq!(
                    chain.validate_genesis(other.genesis()).is_ok(),
                    chain == other
                );
            }
        }
        assert!(ChainNetwork::from_code(6).is_err());
        for name in ["testnet", "Testnet5", "testnet5 ", "unknown"] {
            assert!(ChainNetwork::parse(name).is_err());
        }
    }
}
