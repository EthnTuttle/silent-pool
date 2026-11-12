//! Exact money arithmetic for satoshi amounts (traffic plan section 6.3).
//!
//! BTC JSON amounts are parsed as exact decimals with at most eight
//! fractional digits and converted once to sats with integer arithmetic -
//! never through a binary-float satoshi cast. All sums use checked `u64`
//! with `u128` intermediates; underflow and overflow are named errors, never
//! saturating values.

use crate::error::MoneyError;

/// The number of satoshis in one BTC.
pub const SATS_PER_BTC: u64 = 100_000_000;

/// Checked satoshi addition with a wider intermediate.
pub fn checked_add_sats(a: u64, b: u64) -> Result<u64, MoneyError> {
    a.checked_add(b).ok_or(MoneyError::Overflow)
}

/// Checked satoshi subtraction with a named underflow error.
pub fn checked_sub_sats(a: u64, b: u64) -> Result<u64, MoneyError> {
    a.checked_sub(b).ok_or(MoneyError::Underflow)
}

/// Sum a slice of satoshi amounts with checked arithmetic.
pub fn checked_sum_sats(values: impl IntoIterator<Item = u64>) -> Result<u64, MoneyError> {
    let mut total: u128 = 0;
    for value in values {
        total += u128::from(value);
    }
    u64::try_from(total).map_err(|_| MoneyError::Overflow)
}

/// Ceiling division of non-negative integers.
pub fn ceil_div(numerator: u64, denominator: u64) -> u64 {
    debug_assert!(denominator > 0);
    numerator.div_ceil(denominator)
}

/// Parse an exact BTC decimal amount into satoshis.
///
/// Accepted shape: `digits [ '.' one-to-eight digits ]`. A leading `-` or
/// `+`, a second dot, more than eight fractional digits, empty integer or
/// fractional parts, or non-digit characters are all rejected (fail closed,
/// no rounding). The result is computed as
/// `integer * 10^8 + fraction * 10^(8 - len)` with checked multiplication
/// and addition, so `"0.00000001"` is exactly 1 sat and overflow is a named
/// error rather than a wrap.
pub fn parse_btc_to_sats_exact(value: &str) -> Result<u64, MoneyError> {
    let reject = |reason: &str| {
        Err(MoneyError::NotExactDecimal {
            value: format!("{value:?} ({reason})"),
        })
    };
    if value.is_empty() {
        return reject("empty");
    }
    let mut parts = value.split('.');
    let integer = parts.next().unwrap_or_default();
    let fraction = parts.next().unwrap_or_default();
    if parts.next().is_some() {
        return reject("more than one decimal point");
    }
    if integer.is_empty() {
        return reject("empty integer part");
    }
    if fraction.is_empty() && value.contains('.') {
        return reject("empty fractional part after the decimal point");
    }
    if fraction.len() > 8 {
        return reject("more than eight fractional digits");
    }
    if !integer.bytes().all(|b| b.is_ascii_digit()) || !fraction.bytes().all(|b| b.is_ascii_digit())
    {
        return reject("non-digit characters (signs are rejected)");
    }
    let integer: u128 = integer
        .bytes()
        .fold(0u128, |acc, b| acc * 10 + u128::from(b - b'0'));
    let fraction_padded = format!("{fraction:0<8}");
    let fraction: u128 = if fraction.is_empty() {
        0
    } else {
        fraction_padded
            .bytes()
            .fold(0u128, |acc, b| acc * 10 + u128::from(b - b'0'))
    };
    let sats = integer
        .checked_mul(SATS_PER_BTC as u128)
        .and_then(|whole| whole.checked_add(fraction))
        .ok_or(MoneyError::Overflow)?;
    u64::try_from(sats).map_err(|_| MoneyError::Overflow)
}

/// Compare the rational `fee_sats / vbytes` against a `[min, max]`
/// sats/vB band using exact integer products (no division, no floats).
///
/// Returns `Ok(())` when `min_rate * vbytes <= fee <= max_rate * vbytes`.
pub fn fee_within_band(
    fee_sats: u64,
    vbytes: u64,
    min_rate_sats_vb: u64,
    max_rate_sats_vb: u64,
) -> Result<(), MoneyError> {
    let fee = u128::from(fee_sats);
    let vbytes = u128::from(vbytes);
    if fee < u128::from(min_rate_sats_vb) * vbytes {
        return Err(MoneyError::Underflow);
    }
    if fee > u128::from(max_rate_sats_vb) * vbytes {
        return Err(MoneyError::Overflow);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_decimal_boundaries() {
        assert_eq!(parse_btc_to_sats_exact("0"), Ok(0));
        assert_eq!(parse_btc_to_sats_exact("1"), Ok(100_000_000));
        assert_eq!(parse_btc_to_sats_exact("0.00000001"), Ok(1));
        assert_eq!(parse_btc_to_sats_exact("1.5"), Ok(150_000_000));
        assert_eq!(parse_btc_to_sats_exact("50.00000000"), Ok(5_000_000_000));
        assert_eq!(parse_btc_to_sats_exact("0.12345678"), Ok(12_345_678));
        // 21M BTC cap fits u64 sats exactly.
        assert_eq!(
            parse_btc_to_sats_exact("21000000"),
            Ok(2_100_000_000_000_000)
        );
    }

    #[test]
    fn exact_decimal_rejects() {
        assert!(parse_btc_to_sats_exact("1.000000009").is_err());
        assert!(parse_btc_to_sats_exact("-1").is_err());
        assert!(parse_btc_to_sats_exact("+1").is_err());
        assert!(parse_btc_to_sats_exact("1.").is_err());
        assert!(parse_btc_to_sats_exact(".5").is_err());
        assert!(parse_btc_to_sats_exact("1.2.3").is_err());
        assert!(parse_btc_to_sats_exact("1e3").is_err());
        assert!(parse_btc_to_sats_exact("").is_err());
        assert!(parse_btc_to_sats_exact(" 1").is_err());
        assert!(parse_btc_to_sats_exact("0x10").is_err());
    }

    #[test]
    fn overflow_and_underflow_are_named() {
        assert_eq!(checked_add_sats(u64::MAX, 1), Err(MoneyError::Overflow));
        assert_eq!(checked_sub_sats(0, 1), Err(MoneyError::Underflow));
        assert_eq!(
            checked_sum_sats([u64::MAX, u64::MAX]),
            Err(MoneyError::Overflow)
        );
        assert_eq!(checked_add_sats(u64::MAX - 1, 1), Ok(u64::MAX));
    }

    #[test]
    fn band_comparison_is_exact() {
        // 3 sats/vB at 200 vB: fee must be in [600, 4000].
        assert!(fee_within_band(600, 200, 3, 20).is_ok());
        assert!(fee_within_band(4000, 200, 3, 20).is_ok());
        assert_eq!(fee_within_band(599, 200, 3, 20), Err(MoneyError::Underflow));
        assert_eq!(fee_within_band(4001, 200, 3, 20), Err(MoneyError::Overflow));
        // A nominal pre-signature size is never assumed: a size change of
        // one vB moves the boundary by exactly min_rate.
        assert_eq!(fee_within_band(600, 201, 3, 20), Err(MoneyError::Underflow));
    }

    #[test]
    fn ceil_div_edges() {
        assert_eq!(ceil_div(0, 4), 0);
        assert_eq!(ceil_div(1, 4), 1);
        assert_eq!(ceil_div(4, 4), 1);
        assert_eq!(ceil_div(5, 4), 2);
    }
}
