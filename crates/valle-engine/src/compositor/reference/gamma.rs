//! F32 gamma powers with a guarded rounding interval and the pinned libm fallback.

use std::sync::OnceLock;

use valle_draw::math;

const STEPS: usize = 4096;
const RELATIVE_ENVELOPE: f64 = 1.0e-12;

struct PowerTable {
    coefficients: Box<[[f64; 4]]>,
    scales: [f64; 254],
}

impl PowerTable {
    fn new(exponent: f32) -> Self {
        let exponent = f64::from(exponent);
        let coefficients = (0..=STEPS)
            .map(|index| {
                let center = 1.0 + index as f64 / STEPS as f64;
                let value = math::pow(center, exponent);
                let first = exponent * value / center;
                let second = (exponent - 1.0) * first / (2.0 * center);
                let third = (exponent - 2.0) * second / (3.0 * center);
                [value, first, second, third]
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let scales =
            std::array::from_fn(|index| math::pow(2.0, (index as i32 - 126) as f64 * exponent));
        Self {
            coefficients,
            scales,
        }
    }

    fn rounded(&self, bits: u32) -> Option<f32> {
        let fraction = bits & 0x007f_ffff;
        let index = (fraction as usize + 1024) >> 11;
        let mantissa = f64::from(f32::from_bits(0x3f80_0000 | fraction));
        let delta = mantissa - (1.0 + index as f64 / STEPS as f64);
        let [value, first, second, third] = self.coefficients[index];
        let estimate = ((third * delta + second) * delta + first) * delta + value;
        let estimate = estimate * self.scales[((bits >> 23) & 0xff) as usize - 1];
        // |delta| <= 2^-13. Both supported powers have |fourth derivative| < 1
        // on [1,2], giving a Taylor remainder below 1e-17. The wider relative
        // envelope also covers coefficient, scaling and f64 evaluation rounding.
        // Ambiguous f32 rounding, including overflow boundaries, uses libm.
        let error = estimate.abs() * RELATIVE_ENVELOPE;
        let lower = (estimate - error) as f32;
        let upper = (estimate + error) as f32;
        (lower.to_bits() == upper.to_bits()).then_some(lower)
    }
}

pub(super) fn powf(base: f32, exponent: f32) -> f32 {
    static DECODE: OnceLock<PowerTable> = OnceLock::new();
    static ENCODE: OnceLock<PowerTable> = OnceLock::new();
    let bits = base.to_bits();
    if (0x0080_0000..0x7f80_0000).contains(&bits) {
        let table = if exponent == 2.4_f32 {
            Some(&DECODE)
        } else if exponent == 1.0_f32 / 2.4 {
            Some(&ENCODE)
        } else {
            None
        };
        if let Some(table) = table {
            if base == 1.0 {
                return 1.0;
            }
            if let Some(value) = table
                .get_or_init(|| PowerTable::new(exponent))
                .rounded(bits)
            {
                return value;
            }
        }
    }
    math::pow(f64::from(base), f64::from(exponent)) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run after changing libm, table construction, the interval or target math settings.
    #[test]
    #[ignore = "exhaustive F32 domain; run with --release --ignored"]
    fn all_positive_normal_f32_inputs_match_pinned_scalar() {
        use std::sync::atomic::{AtomicU32, Ordering};
        const START: u32 = 0x0080_0000;
        const END: u32 = 0x7f80_0000;
        const BLOCK: u32 = 1 << 20;
        for exponent in [2.4_f32, 1.0_f32 / 2.4] {
            let next = AtomicU32::new(START);
            let completed = AtomicU32::new(0);
            std::thread::scope(|scope| {
                for _ in 0..6 {
                    let next = &next;
                    let completed = &completed;
                    scope.spawn(move || {
                        loop {
                            let start = next.fetch_add(BLOCK, Ordering::Relaxed);
                            if start >= END {
                                break;
                            }
                            for bits in start..(start + BLOCK).min(END) {
                                let base = f32::from_bits(bits);
                                let reference =
                                    math::pow(f64::from(base), f64::from(exponent)) as f32;
                                assert_eq!(
                                    powf(base, exponent).to_bits(),
                                    reference.to_bits(),
                                    "base={bits:08x} exponent={exponent}"
                                );
                            }
                            completed.fetch_add(1, Ordering::Relaxed);
                        }
                    });
                }
            });
            assert_eq!(completed.load(Ordering::Relaxed), (END - START) / BLOCK);
        }
    }

    #[test]
    fn guarded_gamma_powers_match_pinned_scalar_bits_across_exponents_and_special_values() {
        let mut state = 0x9317_bef5_u32;
        let specials = [
            0_u32,
            0x8000_0000,
            1,
            0x007f_ffff,
            0x0080_0000,
            0x3f80_0000,
            0x7f7f_ffff,
            0x7f80_0000,
            0xff80_0000,
            0x7fc1_734a,
        ];
        for exponent in [2.4_f32, 1.0 / 2.4, 0.45, -2.4, f32::NAN] {
            for bits in specials {
                let base = f32::from_bits(bits);
                assert_eq!(
                    powf(base, exponent).to_bits(),
                    (math::pow(f64::from(base), f64::from(exponent)) as f32).to_bits()
                );
            }
            for _ in 0..65536 {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                let base = f32::from_bits(state);
                assert_eq!(
                    powf(base, exponent).to_bits(),
                    (math::pow(f64::from(base), f64::from(exponent)) as f32).to_bits(),
                    "base={state:08x} exponent={exponent}"
                );
            }
        }
    }
}
