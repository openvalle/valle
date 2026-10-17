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

fn table(exponent: f32) -> Option<&'static PowerTable> {
    static DECODE: OnceLock<PowerTable> = OnceLock::new();
    static ENCODE: OnceLock<PowerTable> = OnceLock::new();
    let table = if exponent == 2.4_f32 {
        &DECODE
    } else if exponent == 1.0_f32 / 2.4 {
        &ENCODE
    } else {
        return None;
    };
    Some(table.get_or_init(|| PowerTable::new(exponent)))
}

/// [`powf`], given the table for `exponent` if it has one.
#[inline]
fn power(table: Option<&PowerTable>, base: f32, exponent: f32) -> f32 {
    let bits = base.to_bits();
    if let Some(table) = table
        && (0x0080_0000..0x7f80_0000).contains(&bits)
    {
        if base == 1.0 {
            return 1.0;
        }
        if let Some(value) = table.rounded(bits) {
            return value;
        }
    }
    math::pow(f64::from(base), f64::from(exponent)) as f32
}

pub(super) fn powf(base: f32, exponent: f32) -> f32 {
    power(table(exponent), base, exponent)
}

/// Replaces every base with [`powf`] of it, looking the table up once.
pub(super) fn powf_each(bases: &mut [f32], exponent: f32) {
    let table = table(exponent);
    #[cfg(all(target_arch = "aarch64", target_endian = "little"))]
    if let Some(table) = table {
        neon::powf_each(table, bases, exponent);
        return;
    }
    for base in bases {
        *base = power(table, *base, exponent);
    }
}

/// On aarch64, four bases at a time take the operations of [`PowerTable::rounded`] in f64 vectors
/// of two, in their order and without fused multiply-adds, so every power keeps its bits.
#[cfg(all(target_arch = "aarch64", target_endian = "little"))]
mod neon {
    use std::arch::aarch64::*;

    use super::{PowerTable, RELATIVE_ENVELOPE, STEPS, power};

    pub(super) fn powf_each(table: &PowerTable, bases: &mut [f32], exponent: f32) {
        let (groups, tail) = bases.as_chunks_mut::<4>();
        // Bases the table leaves to libm keep their places until the end of their block, so the
        // vector loop calls nothing.
        for block in groups.chunks_mut(16) {
            let mut unsettled = 0_u64;
            for (index, group) in block.iter_mut().enumerate() {
                // SAFETY: NEON is part of the aarch64 baseline; the group holds four floats.
                let settled = unsafe {
                    let bases = vld1q_f32(group.as_ptr());
                    let (powers, settled) = four(table, bases, group.map(f32::to_bits));
                    vst1q_f32(group.as_mut_ptr(), vbslq_f32(settled, powers, bases));
                    vaddvq_u32(vandq_u32(settled, vld1q_u32([1, 2, 4, 8].as_ptr())))
                };
                unsettled |= u64::from(settled ^ 0b1111) << (4 * index);
            }
            while unsettled != 0 {
                let lane = unsettled.trailing_zeros() as usize;
                let base = &mut block[lane / 4][lane % 4];
                *base = power(Some(table), *base, exponent);
                unsettled &= unsettled - 1;
            }
        }
        for base in tail {
            *base = power(Some(table), *base, exponent);
        }
    }

    /// The powers of four bases, also given as bits, and the lanes that `power` settles without
    /// libm: positive normal bases that are 1.0 or whose rounding the table settles.
    #[inline(always)]
    fn four(table: &PowerTable, bases: float32x4_t, lanes: [u32; 4]) -> (float32x4_t, uint32x4_t) {
        // SAFETY: NEON is part of the aarch64 baseline. A coefficient index is at most STEPS, and
        // the table holds STEPS + 1 rows; scale indices are clamped to the 254 scales.
        unsafe {
            let bits = vreinterpretq_u32_f32(bases);
            let normal = vcltq_u32(
                vsubq_u32(bits, vdupq_n_u32(0x0080_0000)),
                vdupq_n_u32(0x7f80_0000 - 0x0080_0000),
            );
            let one = vceqq_f32(bases, vdupq_n_f32(1.0));
            let fraction = vandq_u32(bits, vdupq_n_u32(0x007f_ffff));
            let index = vshrq_n_u32::<11>(vaddq_u32(fraction, vdupq_n_u32(1024)));
            let mantissa = vreinterpretq_f32_u32(vorrq_u32(fraction, vdupq_n_u32(0x3f80_0000)));
            // Rows and scales are found from each lane's bits as in `PowerTable::rounded`; lanes
            // that are not positive normal take some row and scale and are not settled.
            let row = |bits: u32| {
                let at = ((bits & 0x007f_ffff) as usize + 1024) >> 11;
                table.coefficients.as_ptr().add(at).cast::<f64>()
            };
            let scale = |bits: u32| {
                let at = ((bits >> 23) & 0xff).clamp(1, 254) as usize - 1;
                vld1_f64(table.scales.as_ptr().add(at))
            };
            let rounded = |mantissa: float64x2_t, index: uint64x2_t, lanes: [u32; 2]| {
                let (low, high) = (row(lanes[0]), row(lanes[1]));
                let (front, back) = (
                    [vld1q_f64(low), vld1q_f64(high)],
                    [vld1q_f64(low.add(2)), vld1q_f64(high.add(2))],
                );
                let value = vzip1q_f64(front[0], front[1]);
                let first = vzip2q_f64(front[0], front[1]);
                let second = vzip1q_f64(back[0], back[1]);
                let third = vzip2q_f64(back[0], back[1]);
                // Dividing by STEPS, a power of two, is exact, as is this product.
                let center = vaddq_f64(
                    vdupq_n_f64(1.0),
                    vmulq_f64(vcvtq_f64_u64(index), vdupq_n_f64(1.0 / STEPS as f64)),
                );
                let delta = vsubq_f64(mantissa, center);
                let estimate = vaddq_f64(vmulq_f64(third, delta), second);
                let estimate = vaddq_f64(vmulq_f64(estimate, delta), first);
                let estimate = vaddq_f64(vmulq_f64(estimate, delta), value);
                let estimate = vmulq_f64(estimate, vcombine_f64(scale(lanes[0]), scale(lanes[1])));
                let error = vmulq_f64(vabsq_f64(estimate), vdupq_n_f64(RELATIVE_ENVELOPE));
                let lower = vcvt_f32_f64(vsubq_f64(estimate, error));
                let upper = vcvt_f32_f64(vaddq_f64(estimate, error));
                (
                    lower,
                    vceq_u32(vreinterpret_u32_f32(lower), vreinterpret_u32_f32(upper)),
                )
            };
            let (low, low_settled) = rounded(
                vcvt_f64_f32(vget_low_f32(mantissa)),
                vmovl_u32(vget_low_u32(index)),
                [lanes[0], lanes[1]],
            );
            let (high, high_settled) = rounded(
                vcvt_high_f64_f32(mantissa),
                vmovl_high_u32(index),
                [lanes[2], lanes[3]],
            );
            let powers = vbslq_f32(one, vdupq_n_f32(1.0), vcombine_f32(low, high));
            let settled = vandq_u32(
                normal,
                vorrq_u32(one, vcombine_u32(low_settled, high_settled)),
            );
            (powers, settled)
        }
    }
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
                            let block = start..(start + BLOCK).min(END);
                            let mut powers = block.clone().map(f32::from_bits).collect::<Vec<_>>();
                            powf_each(&mut powers, exponent);
                            for (bits, each) in block.zip(powers) {
                                let base = f32::from_bits(bits);
                                let reference =
                                    math::pow(f64::from(base), f64::from(exponent)) as f32;
                                assert_eq!(
                                    powf(base, exponent).to_bits(),
                                    reference.to_bits(),
                                    "base={bits:08x} exponent={exponent}"
                                );
                                assert_eq!(
                                    each.to_bits(),
                                    reference.to_bits(),
                                    "each base={bits:08x} exponent={exponent}"
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
            let mut bases = Vec::new();
            for bits in specials {
                let base = f32::from_bits(bits);
                assert_eq!(
                    powf(base, exponent).to_bits(),
                    (math::pow(f64::from(base), f64::from(exponent)) as f32).to_bits()
                );
                bases.push(base);
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
                // Gamma bases with every exponent, and special values among them.
                bases.push(if state.is_multiple_of(7) {
                    f32::from_bits(specials[state as usize % specials.len()])
                } else {
                    f32::from_bits(0x3c00_0000 | (state & 0x03ff_ffff))
                });
            }
            // Each group of four mixes the powers the table settles with those it leaves to libm.
            let mut powers = bases.clone();
            powf_each(&mut powers, exponent);
            for (base, each) in bases.iter().zip(&powers) {
                assert_eq!(
                    each.to_bits(),
                    powf(*base, exponent).to_bits(),
                    "each base={:08x} exponent={exponent}",
                    base.to_bits()
                );
            }
        }
    }
}
