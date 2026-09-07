//! Deterministic Timeline boundary quantization.
//!
//! Closed-schema scalar values are snapped once to the Timeline
//! six-decimal grid before validation.

/// Number of decimal places retained by closed-schema Timeline scalar values.
pub const CANONICAL_SCALAR_DECIMAL_PLACES: u32 = 6;

/// The Timeline closed-schema scalar grid (`10^-6`).
pub const CANONICAL_SCALAR_QUANTUM: f64 = 0.000_001;

const CANONICAL_SCALAR_SCALE: f64 = 1_000_000.0;

// At 2^33 the binary64 ULP is 2^-19, already wider than the 10^-6 Timeline
// grid. Every finite value at or above this magnitude therefore already has a
// round-trippable decimal representation with at most six fractional digits.
const CANONICAL_SCALAR_GRID_LIMIT: f64 = 8_589_934_592.0;

/// Snaps a binary64 timeline scalar to the Timeline six-decimal grid.
///
/// The input binary64 value is treated as an exact binary rational while selecting the nearest
/// millionth, with exact ties rounded away from zero. This avoids a multiply/divide round-trip that
/// can drift by one ULP when canonicalizing the same large scalar again. Signed zero is normalized
/// to positive zero. Non-finite inputs are deliberately preserved so the canonical validator can
/// reject them with its existing field-specific diagnostic. Values whose binary64 ULP is already
/// wider than this grid are preserved.
pub fn quantize_canonical_scalar(value: f64) -> f64 {
    if !value.is_finite() {
        return value;
    }

    if value.abs() >= CANONICAL_SCALAR_GRID_LIMIT {
        return value;
    }

    let bits = value.to_bits();
    let exponent_bits = ((bits >> 52) & 0x7ff) as i32;
    let fraction = bits & ((1_u64 << 52) - 1);
    let (significand, binary_exponent) = if exponent_bits == 0 {
        (fraction, -1074)
    } else {
        ((1_u64 << 52) | fraction, exponent_bits - 1023 - 52)
    };
    debug_assert!(binary_exponent < 0);

    // value * 10^6 = scaled_numerator / 2^shift. The numerator is below 2^73,
    // so u128 covers every value inside CANONICAL_SCALAR_GRID_LIMIT exactly.
    let scaled_numerator = u128::from(significand) * 1_000_000;
    let shift = (-binary_exponent) as u32;
    let rounded_magnitude = if shift >= u128::BITS {
        0
    } else {
        let denominator = 1_u128 << shift;
        let quotient = scaled_numerator >> shift;
        let remainder = scaled_numerator & (denominator - 1);
        quotient + u128::from(remainder >= denominator - remainder)
    };
    debug_assert!(rounded_magnitude < (1_u128 << 53));

    let magnitude = rounded_magnitude as f64 / CANONICAL_SCALAR_SCALE;
    let quantized = if bits >> 63 == 0 {
        magnitude
    } else {
        -magnitude
    };
    if quantized == 0.0 { 0.0 } else { quantized }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_scalars_snap_to_six_decimals_and_normalize_zero() {
        assert_eq!(quantize_canonical_scalar(0.929_999_999_999_999_9), 0.93);
        assert_eq!(
            quantize_canonical_scalar(0.133_333_333_333_333_33),
            0.133_333
        );
        assert_eq!(
            quantize_canonical_scalar(0.266_666_666_666_666_66),
            0.266_667
        );
        assert_eq!(quantize_canonical_scalar(-0.000_000_4).to_bits(), 0);
    }

    #[test]
    fn canonical_scalar_quantization_is_idempotent_and_uses_half_away_from_zero() {
        for value in [
            -123.123_456_789,
            -0.007_812_5,
            0.007_812_5,
            123.123_456_789,
            f64::MAX,
        ] {
            let once = quantize_canonical_scalar(value);
            assert_eq!(quantize_canonical_scalar(once), once);
        }
        assert_eq!(quantize_canonical_scalar(0.007_812_5), 0.007_813);
        assert_eq!(quantize_canonical_scalar(-0.007_812_5), -0.007_813);

        let below_tie = f64::from_bits(0.007_812_5_f64.to_bits() - 1);
        let above_tie = f64::from_bits(0.007_812_5_f64.to_bits() + 1);
        assert_eq!(quantize_canonical_scalar(below_tie), 0.007_812);
        assert_eq!(quantize_canonical_scalar(above_tie), 0.007_813);
    }

    #[test]
    fn canonical_scalar_quantization_is_idempotent_across_the_binary64_domain() {
        let drift_regression = f64::from_bits(0x41f0_b16a_09c8_b75c);
        assert_eq!(
            quantize_canonical_scalar(drift_regression),
            4_480_999_580.544_765
        );

        let grid_limit = CANONICAL_SCALAR_GRID_LIMIT;
        for value in [
            f64::MIN_POSITIVE,
            f64::from_bits(1),
            f64::from_bits(grid_limit.to_bits() - 1),
            grid_limit,
            f64::from_bits(grid_limit.to_bits() + 1),
            f64::MAX,
        ] {
            for signed in [value, -value] {
                let once = quantize_canonical_scalar(signed);
                assert_eq!(
                    quantize_canonical_scalar(once).to_bits(),
                    once.to_bits(),
                    "q6 must be idempotent for {signed:?}"
                );
            }
        }

        let mut state = 0x9e37_79b9_7f4a_7c15_u64;
        for _ in 0..100_000 {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let sign = state & (1_u64 << 63);
            let exponent = ((state >> 52) % 1056) << 52;
            let fraction = state & ((1_u64 << 52) - 1);
            let sample = f64::from_bits(sign | exponent | fraction);
            let once = quantize_canonical_scalar(sample);
            assert_eq!(quantize_canonical_scalar(once).to_bits(), once.to_bits());
        }
    }

    #[test]
    fn canonical_scalar_quantization_leaves_invalid_values_for_validation() {
        assert!(quantize_canonical_scalar(f64::NAN).is_nan());
        assert_eq!(quantize_canonical_scalar(f64::INFINITY), f64::INFINITY);
        assert_eq!(
            quantize_canonical_scalar(f64::NEG_INFINITY),
            f64::NEG_INFINITY
        );
    }
}
