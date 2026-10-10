//! Schlick coefficients with an exact, guarded multiplication fast path.
//!
//! For a clamped f32 cosine, `1 - cosine` lies on the 2^-24 grid. Exhaustively
//! checked over that domain, the f64 fifth power rounded to f32 is within one
//! ULP of libm::powf. Only skip powf when both ends of that interval produce
//! identical final f32 coefficients; otherwise retain the reference calculation.

#[inline]
fn fifth_power(value: f32) -> f32 {
    let value = f64::from(value);
    let squared = value * value;
    (squared * squared * value) as f32
}

#[inline]
fn reference(value: f32, f0: [f32; 3]) -> (f32, [f32; 3]) {
    let power = libm::powf(value, 5.0);
    (0.04 + 0.96 * power, f0.map(|f| f + (1.0 - f) * power))
}

#[inline]
fn guarded(value: f32, f0: [f32; 3]) -> Option<(f32, [f32; 3])> {
    let bits = fifth_power(value).to_bits();
    let lower = f32::from_bits(bits.saturating_sub(1));
    let upper = f32::from_bits(bits + 1);
    let dielectric_lower = 0.04 + 0.96 * lower;
    let dielectric_upper = 0.04 + 0.96 * upper;
    if dielectric_lower.to_bits() != dielectric_upper.to_bits() {
        return None;
    }
    // For a dielectric, all four expressions have the same operands:
    // 1.0f32 - 0.04f32 == 0.96f32, including its bit representation.
    if f0 == [0.04; 3] {
        return Some((dielectric_lower, [dielectric_lower; 3]));
    }
    let lower_terms = f0.map(|f| f + (1.0 - f) * lower);
    let upper_terms = f0.map(|f| f + (1.0 - f) * upper);
    if lower_terms.map(f32::to_bits) == upper_terms.map(f32::to_bits) {
        Some((dielectric_lower, lower_terms))
    } else {
        None
    }
}

#[inline]
pub(super) fn coefficients(cosine: f32, f0: [f32; 3]) -> (f32, [f32; 3]) {
    let value = 1.0 - cosine;
    if (0.0..=1.0).contains(&cosine)
        && let Some(terms) = guarded(value, f0)
    {
        return terms;
    }
    reference(value, f0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_reference(cosine: f32, f0: [f32; 3]) {
        let actual = coefficients(cosine, f0);
        let expected = reference(1.0 - cosine, f0);
        assert_eq!(
            (actual.0.to_bits(), actual.1.map(f32::to_bits)),
            (expected.0.to_bits(), expected.1.map(f32::to_bits)),
            "cosine={cosine:?}, f0={f0:?}",
        );
    }

    #[test]
    fn guarded_coefficients_match_reference_bits() {
        assert_eq!((1.0_f32 - 0.04).to_bits(), 0.96_f32.to_bits());
        for cosine in [0.0, f32::from_bits(1), 0.25, 0.5, 0.75, 1.0] {
            for f0 in [[0.0; 3], [0.04; 3], [1.0; 3], [0.0, 0.04, 1.0]] {
                assert_reference(cosine, f0);
            }
        }
        let mut seed = 41_u32;
        let mut next = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed & 0x00ff_ffff) as f32 / 16_777_216.0
        };
        for _ in 0..100_000 {
            let cosine = next();
            assert_reference(cosine, [0.04; 3]);
            assert_reference(cosine, [next(), next(), next()]);
        }
    }

    #[test]
    fn rounding_boundaries_fall_back_to_reference() {
        assert!(guarded(0.01, [0.04; 3]).is_some());
        assert!(guarded(1.0, [0.04; 3]).is_none());
        for cosine in [-1.0, 2.0, f32::INFINITY, f32::NEG_INFINITY, f32::NAN] {
            assert_reference(cosine, [0.04; 3]);
        }
    }

    // This checks every possible 1 - clamp(cosine, 0, 1), and must be rerun
    // whenever libm or the power calculation changes.
    #[test]
    #[ignore = "exhaustive 16,777,217-value libm rounding bound"]
    fn fifth_power_bound_covers_every_schlick_input() {
        for step in 0..=16_777_216_u32 {
            let value = step as f32 / 16_777_216.0;
            assert!(
                fifth_power(value)
                    .to_bits()
                    .abs_diff(libm::powf(value, 5.0).to_bits())
                    <= 1,
                "input bits {:08x}",
                value.to_bits(),
            );
        }
    }
}
