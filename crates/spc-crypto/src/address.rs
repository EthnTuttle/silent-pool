//! BIP 352 silent-payment address parsing (SPEC §2.1).
//!
//! BIP 352 SP addresses use bech32m with a custom (non-segwit) witness program
//! encoding: version byte `q` (= 0) followed by 66 bytes (33+33) of key data.
//! Because the payload is 66 bytes, the address is 116 chars for `sp` and 117
//! chars for `tsp` — longer than the 90-char segwit limit. We decode via
//! `CheckedHrpstring` (bech32m-verified, no witness-length constraint) and
//! extract the version and payload manually.

use bech32::primitives::decode::CheckedHrpstring;
use bech32::Bech32m;
use bitcoin::secp256k1::PublicKey;

use crate::Error;

/// Bitcoin network selector (SPEC §2.1).
/// The HRP encodes the network: `sp` for mainnet, `tsp` for others.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Network {
    Mainnet,
    Testnet,
    Signet,
    Regtest,
}

impl Network {
    /// The expected bech32m HRP for this network.
    pub fn hrp(self) -> &'static str {
        match self {
            Network::Mainnet => "sp",
            Network::Testnet | Network::Signet | Network::Regtest => "tsp",
        }
    }
}

/// A parsed BIP 352 silent-payment address (SPEC §2.1).
///
/// Payload layout (66 bytes): `ser_P(B_scan) || ser_P(B_spend)`.
/// Both are 33-byte compressed SEC1 points. Version MUST be 0.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpAddress {
    pub network: Network,
    pub b_scan: PublicKey,
    pub b_spend: PublicKey,
    /// The raw HRP string from the decoded address.
    pub hrp: String,
    /// Witness version (must be 0 in v0.1).
    pub version: u8,
}

/// Parse and validate a BIP 352 silent-payment address string.
///
/// Rules (SPEC §2.1):
/// - HRP MUST match `expected.hrp()`.
/// - Witness version MUST be 0.
/// - Payload MUST be exactly 66 bytes.
/// - Both 33-byte halves MUST be valid compressed secp256k1 points.
/// - Checksum MUST be valid bech32m.
///
/// Any failure returns `Err`. There is no donation fallback.
pub fn parse_sp_address(s: &str, expected: Network) -> Result<SpAddress, Error> {
    // Decode and validate bech32m checksum. This does NOT enforce segwit
    // length limits — SP addresses exceed them.
    let mut parsed = CheckedHrpstring::new::<Bech32m>(s)
        .map_err(|e| Error::InvalidAddress(format!("bech32m decode: {e}")))?;

    // Verify HRP.
    let expected_hrp = expected.hrp();
    let hrp_str = parsed.hrp().to_string();
    if hrp_str != expected_hrp {
        return Err(Error::InvalidAddress(format!(
            "wrong HRP: got '{hrp_str}', expected '{expected_hrp}'"
        )));
    }

    // The first data character is the 5-bit witness version, not a byte. Remove
    // it before converting the remaining 5-bit groups to the 66-byte payload.
    let version = parsed
        .remove_witness_version()
        .ok_or_else(|| Error::InvalidAddress("missing or invalid witness version".to_string()))?
        .to_u8();
    if version != 0 {
        return Err(Error::InvalidAddress(format!(
            "wrong witness version: got {version}, expected 0"
        )));
    }

    // 66 payload bytes occupy 106 five-bit groups (530 bits); the low two
    // padding bits MUST be zero (BIP 173).
    let data = parsed.data_part_ascii_no_checksum();
    if data.len() != 106 {
        return Err(Error::InvalidAddress(format!(
            "wrong data length: got {} five-bit groups, expected 106",
            data.len()
        )));
    }
    let last = bech32::Fe32::from_char(data[105] as char)
        .map_err(|e| Error::InvalidAddress(format!("bad data character: {e}")))?;
    if last.to_u8() & 0b11 != 0 {
        return Err(Error::InvalidAddress("non-zero padding bits".to_string()));
    }
    let payload: Vec<u8> = parsed.byte_iter().collect();
    if payload.len() != 66 {
        return Err(Error::InvalidAddress(format!(
            "wrong payload length: got {} bytes, expected 66",
            payload.len()
        )));
    }

    let b_scan = PublicKey::from_slice(&payload[..33])
        .map_err(|e| Error::InvalidAddress(format!("B_scan is not a valid point: {e}")))?;
    let b_spend = PublicKey::from_slice(&payload[33..])
        .map_err(|e| Error::InvalidAddress(format!("B_spend is not a valid point: {e}")))?;

    Ok(SpAddress {
        network: expected,
        b_scan,
        b_spend,
        hrp: hrp_str,
        version,
    })
}

/// Encode an SP address (for testing and CLI output).
///
/// `version` must be 0; `b_scan` and `b_spend` are 33-byte compressed points.
/// The HRP is taken from the network.
pub fn encode_sp_address(
    network: Network,
    b_scan: &PublicKey,
    b_spend: &PublicKey,
) -> Result<String, Error> {
    use bech32::{ByteIterExt, Fe32, Fe32IterExt};

    let hrp_str = network.hrp();
    let hrp =
        bech32::Hrp::parse(hrp_str).map_err(|e| Error::InvalidAddress(format!("HRP: {e}")))?;

    // The version is a single 5-bit field element, not a byte. Convert the
    // 66-byte key payload to 5-bit groups, prepend `q` (version 0), and
    // checksum the result.
    let mut payload = Vec::with_capacity(66);
    payload.extend_from_slice(&b_scan.serialize());
    payload.extend_from_slice(&b_spend.serialize());

    let encoded: String = payload
        .iter()
        .copied()
        .bytes_to_fes()
        .with_checksum::<Bech32m>(&hrp)
        .with_witness_version(Fe32::Q)
        .chars()
        .collect();

    Ok(encoded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};

    fn keys() -> (PublicKey, PublicKey) {
        let secp = Secp256k1::new();
        let k1 = SecretKey::from_slice(&[1u8; 32]).unwrap();
        let k2 = SecretKey::from_slice(&[2u8; 32]).unwrap();
        (
            PublicKey::from_secret_key(&secp, &k1),
            PublicKey::from_secret_key(&secp, &k2),
        )
    }

    #[test]
    fn roundtrip_mainnet() {
        let (b_scan, b_spend) = keys();
        let addr = encode_sp_address(Network::Mainnet, &b_scan, &b_spend).unwrap();
        assert!(addr.starts_with("sp1"), "mainnet prefix: {addr}");
        let parsed = parse_sp_address(&addr, Network::Mainnet).unwrap();
        assert_eq!(parsed.b_scan, b_scan);
        assert_eq!(parsed.b_spend, b_spend);
        assert_eq!(parsed.hrp, "sp");
        assert_eq!(parsed.version, 0);
    }

    #[test]
    fn roundtrip_testnet() {
        let (b_scan, b_spend) = keys();
        let addr = encode_sp_address(Network::Testnet, &b_scan, &b_spend).unwrap();
        assert!(addr.starts_with("tsp1"), "testnet prefix: {addr}");
        let parsed = parse_sp_address(&addr, Network::Testnet).unwrap();
        assert_eq!(parsed.b_scan, b_scan);
        assert_eq!(parsed.b_spend, b_spend);
        assert_eq!(parsed.hrp, "tsp");
    }

    #[test]
    fn reject_wrong_hrp() {
        let (b_scan, b_spend) = keys();
        let addr = encode_sp_address(Network::Mainnet, &b_scan, &b_spend).unwrap();
        // Parse as testnet should fail.
        assert!(parse_sp_address(&addr, Network::Testnet).is_err());
    }

    #[test]
    fn reject_wrong_hrp_bc1q() {
        // A native segwit address (not SP).
        let err = parse_sp_address(
            "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq",
            Network::Mainnet,
        );
        assert!(err.is_err());
    }

    #[test]
    fn reject_bad_checksum() {
        let (b_scan, b_spend) = keys();
        let mut addr = encode_sp_address(Network::Testnet, &b_scan, &b_spend).unwrap();
        // Flip the last character to corrupt the checksum.
        let last = addr.pop().unwrap();
        addr.push(if last == 'q' { 'p' } else { 'q' });
        assert!(parse_sp_address(&addr, Network::Testnet).is_err());
    }

    #[test]
    fn reject_truncated() {
        let (b_scan, b_spend) = keys();
        let addr = encode_sp_address(Network::Testnet, &b_scan, &b_spend).unwrap();
        let truncated = &addr[..addr.len() - 4];
        assert!(parse_sp_address(truncated, Network::Testnet).is_err());
    }

    #[test]
    fn reject_empty() {
        assert!(parse_sp_address("", Network::Mainnet).is_err());
    }

    #[test]
    fn reject_invalid_point() {
        // Build a payload with an all-zero first key (not on the curve).
        let secp = Secp256k1::new();
        let k = SecretKey::from_slice(&[2u8; 32]).unwrap();
        let b_spend = PublicKey::from_secret_key(&secp, &k);

        let hrp = bech32::Hrp::parse("tsp").unwrap();
        let mut payload = vec![0u8; 1 + 66]; // version + 66 zeros
        payload[0] = 0; // version 0
                        // Leave first 33 payload bytes as zeros (invalid point).
        payload[34..].copy_from_slice(&b_spend.serialize());

        let addr = bech32::encode::<Bech32m>(hrp, &payload).unwrap();
        assert!(parse_sp_address(&addr, Network::Testnet).is_err());
    }

    #[test]
    fn signet_and_regtest_use_tsp() {
        let (b_scan, b_spend) = keys();
        let addr = encode_sp_address(Network::Testnet, &b_scan, &b_spend).unwrap();
        assert!(parse_sp_address(&addr, Network::Signet).is_ok());
        assert!(parse_sp_address(&addr, Network::Regtest).is_ok());
    }
}
