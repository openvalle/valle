//! Transfer functions at the raster's sRGB half-float boundary. Opaque pixels can
//! share quantized results; fractional coverage still uses the original math.

use std::sync::LazyLock;

use super::{f16_bits_to_f32, f32_to_f16_bits};
use crate::scene3d::asset::{linear_to_srgb, srgb_to_linear};

const START: u32 = 0x3b00_0000; // 2^-9, below the nonlinear sRGB segment.
const END: u32 = 1.0_f32.to_bits();
const SHIFT: u32 = 11;
const BINS: usize = ((END - START) >> SHIFT) as usize;

// 72 KiB, initialized once. Zero marks a bin that crosses a half-float rounding
// boundary and must use the reference function. Widen both endpoints by 16 f32
// ULPs so libm rounding near a boundary cannot make an ambiguous bin look constant.
// The exhaustive test checks every representable input in the accelerated range,
// including bin interiors, against the actual reference math.
static ENCODE: LazyLock<Box<[u16]>> = LazyLock::new(|| {
    (0..BINS)
        .map(|index| {
            let start = START + ((index as u32) << SHIFT);
            let end = start + (1 << SHIFT) - 1;
            let low = linear_to_srgb(f32::from_bits(start)).clamp(0.0, 1.0);
            let high = linear_to_srgb(f32::from_bits(end)).clamp(0.0, 1.0);
            let low = f32_to_f16_bits(f32::from_bits(low.to_bits() - 16));
            let high = f32_to_f16_bits(f32::from_bits(high.to_bits() + 16));
            if low == high { low } else { 0 }
        })
        .collect()
});

#[inline]
pub(super) fn encode_opaque(value: f32) -> u16 {
    if value > 0.003_130_8 && value < 1.0 {
        let result = ENCODE[((value.to_bits() - START) >> SHIFT) as usize];
        if result != 0 {
            return result;
        }
    }
    f32_to_f16_bits(linear_to_srgb(value).clamp(0.0, 1.0))
}

// The half-float domain in [0, 1] is small enough to cache the inverse exactly.
// This saves three powf calls per opaque source pixel in CPU depth of field.
static DECODE: LazyLock<Box<[f32]>> = LazyLock::new(|| {
    (0..=0x3c00)
        .map(|bits| srgb_to_linear(f16_bits_to_f32(bits)))
        .collect()
});

#[inline]
pub(super) fn decode_opaque(bits: u16) -> f32 {
    DECODE[usize::from(bits.min(0x3c00))]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opaque_transfer_matches_reference_at_boundaries() {
        for index in 0..BINS {
            let start = START + ((index as u32) << SHIFT);
            for offset in [0, 1, (1 << (SHIFT - 1)), (1 << SHIFT) - 2, (1 << SHIFT) - 1] {
                let value = f32::from_bits(start + offset);
                assert_eq!(
                    encode_opaque(value),
                    f32_to_f16_bits(linear_to_srgb(value).clamp(0.0, 1.0)),
                    "input bits {:08x}",
                    value.to_bits(),
                );
            }
        }
        for bits in 0..=0x3c00 {
            let value = f16_bits_to_f32(bits);
            assert_eq!(
                encode_opaque(value),
                f32_to_f16_bits(linear_to_srgb(value).clamp(0.0, 1.0)),
            );
            assert_eq!(
                decode_opaque(bits).to_bits(),
                srgb_to_linear(value).to_bits()
            );
        }
        assert_eq!(decode_opaque(0x3c01), 1.0);
        // Avoid silently losing the optimization after changes to binning or guards.
        assert!(ENCODE.iter().filter(|&&value| value != 0).count() > BINS * 3 / 4);
    }

    #[test]
    #[ignore = "exhaustive 75-million-input transfer check; run explicitly after math changes"]
    fn opaque_encode_matches_reference_for_every_accelerated_f32() {
        for bits in START..END {
            let value = f32::from_bits(bits);
            assert_eq!(
                encode_opaque(value),
                f32_to_f16_bits(linear_to_srgb(value).clamp(0.0, 1.0)),
                "input bits {bits:08x}",
            );
        }
    }
}
