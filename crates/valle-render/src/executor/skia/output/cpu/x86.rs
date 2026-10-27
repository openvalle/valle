use std::arch::x86_64::*;

use super::{Codes, Converter, GamutMap, Kernel, Parameters};

pub(super) struct Avx2;

#[target_feature(enable = "avx2,f16c")]
pub(super) unsafe fn row<const CACHED: bool>(
    converter: &mut Converter,
    input: &[u8],
    y: usize,
    swap: bool,
    output: &mut [u8],
) -> Result<(), &'static str> {
    // SAFETY: dispatch checked both features. The loop can inline kernels in this ISA context.
    unsafe { converter.run::<Avx2, CACHED>(input, y, swap, output) }
}

impl Kernel for Avx2 {
    #[inline]
    #[target_feature(enable = "avx2,f16c")]
    unsafe fn colors(p: &Parameters, keys: [u64; 4]) -> Codes {
        // SAFETY: runtime dispatch checked both AVX2 (including OS AVX state) and F16C. The two
        // loads read exactly 32 bytes. F16C widens subnormals exactly, like the portable kernel.
        unsafe {
            let source = keys.as_ptr().cast::<__m128i>();
            let a = _mm256_cvtph_ps(_mm_loadu_si128(source));
            let b = _mm256_cvtph_ps(_mm_loadu_si128(source.add(1)));
            let mut r = _mm256_castps256_ps128(a);
            let mut g = _mm256_extractf128_ps::<1>(a);
            let mut b0 = _mm256_castps256_ps128(b);
            let mut alpha = _mm256_extractf128_ps::<1>(b);
            _MM_TRANSPOSE4_PS(&mut r, &mut g, &mut b0, &mut alpha);
            let rgb = [r, g, b0].map(|v| _mm256_cvtps_pd(v));
            let target = p.matrix.map(|m| {
                _mm256_cvtpd_ps(_mm256_add_pd(
                    _mm256_add_pd(
                        _mm256_mul_pd(rgb[0], _mm256_set1_pd(m[0])),
                        _mm256_mul_pd(rgb[1], _mm256_set1_pd(m[1])),
                    ),
                    _mm256_mul_pd(rgb[2], _mm256_set1_pd(m[2])),
                ))
            });
            gamut(p, target).map(|v| {
                let position = _mm_mul_ps(v, _mm_set1_ps(4096.0));
                let index = _mm_min_epi32(_mm_cvttps_epi32(position), _mm_set1_epi32(4095));
                let lower = _mm_i32gather_ps::<4>(p.curve.as_ptr(), index);
                let upper = _mm_i32gather_ps::<4>(
                    p.curve.as_ptr(),
                    _mm_add_epi32(index, _mm_set1_epi32(1)),
                );
                let code = _mm_add_ps(
                    lower,
                    _mm_mul_ps(
                        _mm_sub_ps(upper, lower),
                        _mm_sub_ps(position, _mm_cvtepi32_ps(index)),
                    ),
                );
                let mut lanes = [0.0; 4];
                _mm_storeu_ps(lanes.as_mut_ptr(), code);
                lanes
            })
        }
    }

    #[inline]
    #[target_feature(enable = "avx2,f16c")]
    unsafe fn pack(codes: Codes, noise: Option<&Codes>, swap: bool) -> [u8; 16] {
        // SAFETY: all vector loads/stores fit their four-lane arrays. Do not use CVTPS2DQ's
        // ties-to-even, or add(0.5)+truncate: the latter misrounds floats just below a half.
        unsafe {
            let mut codes = std::array::from_fn::<_, 3, _>(|c| {
                let mut v = _mm_loadu_ps(codes[c].as_ptr());
                if let Some(n) = noise {
                    v = _mm_add_ps(v, _mm_loadu_ps(n[c].as_ptr()));
                }
                v = _mm_min_ps(_mm_max_ps(v, _mm_setzero_ps()), _mm_set1_ps(255.0));
                let integer = _mm_cvttps_epi32(v);
                let fraction = _mm_sub_ps(v, _mm_cvtepi32_ps(integer));
                let up = _mm_castps_si128(_mm_cmpge_ps(fraction, _mm_set1_ps(0.5)));
                _mm_sub_epi32(integer, up)
            });
            if swap {
                codes.swap(0, 2);
            }
            let packed = _mm_or_si128(
                _mm_or_si128(codes[0], _mm_slli_epi32::<8>(codes[1])),
                _mm_or_si128(
                    _mm_slli_epi32::<16>(codes[2]),
                    _mm_set1_epi32(0xff00_0000_u32 as i32),
                ),
            );
            let mut result = [0; 16];
            _mm_storeu_si128(result.as_mut_ptr().cast(), packed);
            result
        }
    }
}

#[inline]
#[target_feature(enable = "avx2,f16c")]
unsafe fn gamut(p: &Parameters, rgb: [__m128; 3]) -> [__m128; 3] {
    let zero = _mm_setzero_ps();
    let one = _mm_set1_ps(1.0);
    let clamp = |v| {
        _mm_blendv_ps(
            _mm_blendv_ps(v, zero, _mm_cmplt_ps(v, zero)),
            one,
            _mm_cmpgt_ps(v, one),
        )
    };
    if p.gamut == GamutMap::Clip {
        return rgb.map(clamp);
    }
    let inside = |v| _mm_and_ps(_mm_cmpge_ps(v, zero), _mm_cmple_ps(v, one));
    let unchanged = _mm_and_ps(inside(rgb[0]), _mm_and_ps(inside(rgb[1]), inside(rgb[2])));
    if _mm_movemask_ps(unchanged) == 15 {
        return rgb;
    }
    let y = clamp(_mm_add_ps(
        _mm_add_ps(
            _mm_mul_ps(rgb[0], _mm_set1_ps(p.weights[0])),
            _mm_mul_ps(rgb[1], _mm_set1_ps(p.weights[1])),
        ),
        _mm_mul_ps(rgb[2], _mm_set1_ps(p.weights[2])),
    ));
    let mut scale = one;
    for channel in rgb {
        let delta = _mm_sub_ps(channel, y);
        // Put scale second so MINPS returns it on an unused NaN lane (zero/zero).
        let above = _mm_min_ps(_mm_div_ps(_mm_sub_ps(one, y), delta), scale);
        let below = _mm_min_ps(_mm_div_ps(y, _mm_sub_ps(zero, delta)), scale);
        scale = _mm_blendv_ps(
            _mm_blendv_ps(scale, below, _mm_cmplt_ps(delta, zero)),
            above,
            _mm_cmpgt_ps(delta, zero),
        );
    }
    rgb.map(|v| {
        _mm_blendv_ps(
            clamp(_mm_add_ps(y, _mm_mul_ps(_mm_sub_ps(v, y), scale))),
            v,
            unchanged,
        )
    })
}
