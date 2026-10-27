use std::arch::aarch64::*;

use super::{Codes, Converter, GamutMap, Kernel, Parameters};

pub(super) struct Neon;

#[target_feature(enable = "neon")]
pub(super) unsafe fn row<const CACHED: bool>(
    converter: &mut Converter,
    input: &[u8],
    y: usize,
    swap: bool,
    output: &mut [u8],
) -> Result<(), &'static str> {
    // SAFETY: dispatch checked NEON. Inlining the loop here keeps per-batch ISA calls inlineable.
    unsafe { converter.run::<Neon, CACHED>(input, y, swap, output) }
}

impl Kernel for Neon {
    #[inline]
    #[target_feature(enable = "neon")]
    unsafe fn colors(p: &Parameters, keys: [u64; 4]) -> Codes {
        // SAFETY: the caller checked NEON and validated four finite, opaque F16 pixels. Loads
        // are unaligned and contained within the batch; every LUT index is clamped to 0..4095.
        unsafe {
            let uint16x4x4_t(r, g, b, _) = vld4_u16(keys.as_ptr().cast());
            let source = [r, g, b].map(|x| vcvt_f32_f16(vreinterpret_f16_u16(x)));
            let low = source.map(|x| vcvt_f64_f32(vget_low_f32(x)));
            let high = source.map(|x| vcvt_f64_f32(vget_high_f32(x)));
            // Keep the reference f64 coefficients, addition order and single f32 rounding.
            let target = p.matrix.map(|m| {
                let apply = |v: [float64x2_t; 3]| {
                    vaddq_f64(
                        vaddq_f64(vmulq_n_f64(v[0], m[0]), vmulq_n_f64(v[1], m[1])),
                        vmulq_n_f64(v[2], m[2]),
                    )
                };
                vcombine_f32(vcvt_f32_f64(apply(low)), vcvt_f32_f64(apply(high)))
            });
            gamut(p, target).map(|v| {
                let position = vmulq_n_f32(v, 4096.0);
                let index = vminq_u32(vcvtq_u32_f32(position), vdupq_n_u32(4095));
                let pair = |i| vld1_f32(p.curve.as_ptr().add(i as usize));
                let a = vcombine_f32(
                    pair(vgetq_lane_u32::<0>(index)),
                    pair(vgetq_lane_u32::<1>(index)),
                );
                let b = vcombine_f32(
                    pair(vgetq_lane_u32::<2>(index)),
                    pair(vgetq_lane_u32::<3>(index)),
                );
                let (lower, upper) = (vuzp1q_f32(a, b), vuzp2q_f32(a, b));
                let code = vaddq_f32(
                    lower,
                    vmulq_f32(
                        vsubq_f32(upper, lower),
                        vsubq_f32(position, vcvtq_f32_u32(index)),
                    ),
                );
                let mut lanes = [0.0; 4];
                vst1q_f32(lanes.as_mut_ptr(), code);
                lanes
            })
        }
    }

    #[inline]
    #[target_feature(enable = "neon")]
    unsafe fn pack(codes: Codes, noise: Option<&Codes>, swap: bool) -> [u8; 16] {
        // SAFETY: each array holds exactly four lanes and output holds exactly sixteen bytes.
        // FCVTA is round-to-nearest, ties away from zero, independent of the FP rounding mode.
        unsafe {
            let mut codes = std::array::from_fn::<_, 3, _>(|c| {
                let mut v = vld1q_f32(codes[c].as_ptr());
                if let Some(n) = noise {
                    v = vaddq_f32(v, vld1q_f32(n[c].as_ptr()));
                }
                vcvtaq_u32_f32(vminq_f32(
                    vmaxq_f32(v, vdupq_n_f32(0.0)),
                    vdupq_n_f32(255.0),
                ))
            });
            if swap {
                codes.swap(0, 2);
            }
            let packed = vorrq_u32(
                vorrq_u32(codes[0], vshlq_n_u32::<8>(codes[1])),
                vorrq_u32(vshlq_n_u32::<16>(codes[2]), vdupq_n_u32(0xff00_0000)),
            );
            let mut result = [0; 16];
            vst1q_u8(result.as_mut_ptr(), vreinterpretq_u8_u32(packed));
            result
        }
    }
}

#[inline]
#[target_feature(enable = "neon")]
unsafe fn gamut(p: &Parameters, rgb: [float32x4_t; 3]) -> [float32x4_t; 3] {
    let zero = vdupq_n_f32(0.0);
    let one = vdupq_n_f32(1.0);
    // Comparisons preserve signed zero, as f32::clamp does in the reference.
    let clamp = |v| {
        vbslq_f32(
            vcgtq_f32(v, one),
            one,
            vbslq_f32(vcltq_f32(v, zero), zero, v),
        )
    };
    if p.gamut == GamutMap::Clip {
        return rgb.map(clamp);
    }
    let inside = |v| vandq_u32(vcgeq_f32(v, zero), vcleq_f32(v, one));
    let unchanged = vandq_u32(inside(rgb[0]), vandq_u32(inside(rgb[1]), inside(rgb[2])));
    if vminvq_u32(unchanged) == u32::MAX {
        return rgb;
    }
    let y = clamp(vaddq_f32(
        vaddq_f32(
            vmulq_n_f32(rgb[0], p.weights[0]),
            vmulq_n_f32(rgb[1], p.weights[1]),
        ),
        vmulq_n_f32(rgb[2], p.weights[2]),
    ));
    let mut scale = one;
    for channel in rgb {
        let delta = vsubq_f32(channel, y);
        let above = vminnmq_f32(scale, vdivq_f32(vsubq_f32(one, y), delta));
        let below = vminnmq_f32(scale, vdivq_f32(y, vnegq_f32(delta)));
        scale = vbslq_f32(
            vcgtq_f32(delta, zero),
            above,
            vbslq_f32(vcltq_f32(delta, zero), below, scale),
        );
    }
    rgb.map(|v| {
        vbslq_f32(
            unchanged,
            v,
            clamp(vaddq_f32(y, vmulq_f32(vsubq_f32(v, y), scale))),
        )
    })
}
