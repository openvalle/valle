use thiserror::Error;

use super::{
    ColorMathError, PremulRgba32, ReferenceImage, WORKING_REFERENCE_WHITE_NITS, decode_author_srgb,
    encode_output_rgb, working_to_primaries,
};
use crate::resource::{
    ColorPrimaries, Dither, GamutMap, OutputAlphaMode, OutputBitDepth, OutputSpec, ToneMap,
    TransferFunction,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputStorage {
    Integer { bit_depth: u8, codes: [u16; 4] },
    Float16 { bits: [u16; 4] },
}

/// `encoded` is the exact target-transfer/alpha result before dither and quantization. `storage`
/// is the terminal representation that a sink must receive.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReferenceOutputSample {
    pub encoded: [f32; 4],
    pub storage: OutputStorage,
}

pub fn output_root_pixel(spec: OutputSpec) -> Result<PremulRgba32, OutputMathError> {
    decode_author_srgb(spec.background().author_color()).map_err(Into::into)
}

pub fn transform_output_pixel(
    pixel: PremulRgba32,
    spec: OutputSpec,
    position: [u32; 2],
) -> Result<ReferenceOutputSample, OutputMathError> {
    if spec.alpha() == OutputAlphaMode::Opaque && pixel.alpha() != 1.0 {
        return Err(OutputMathError::OpaquePixelHasTransparency {
            alpha: pixel.alpha(),
        });
    }

    let working = tone_map(pixel.straight_rgb()?, spec.tone_map())?;
    let target = working_to_primaries(working, spec.target().primaries)?;
    let maximum = target_linear_maximum(spec);
    let mapped = gamut_map(target, spec.target().primaries, maximum, spec.gamut_map())?;
    let encoded_straight = encode_output_rgb(
        mapped,
        spec.target().transfer,
        spec.target().primaries,
        spec.luminance(),
    )?;
    if !encoded_straight.iter().all(|channel| channel.is_finite()) {
        return Err(OutputMathError::NonFiniteOutput);
    }

    let alpha = match spec.alpha() {
        OutputAlphaMode::Opaque => 1.0,
        OutputAlphaMode::StraightCoverage | OutputAlphaMode::PremultipliedCoverage => pixel.alpha(),
    };
    let encoded_rgb = match spec.alpha() {
        OutputAlphaMode::PremultipliedCoverage => encoded_straight.map(|channel| channel * alpha),
        OutputAlphaMode::Opaque | OutputAlphaMode::StraightCoverage => encoded_straight,
    };
    let encoded = [encoded_rgb[0], encoded_rgb[1], encoded_rgb[2], alpha];
    let storage = quantize(encoded, spec, position)?;
    Ok(ReferenceOutputSample { encoded, storage })
}

pub fn transform_output_image(
    image: &ReferenceImage,
    spec: OutputSpec,
) -> Result<Vec<ReferenceOutputSample>, OutputMathError> {
    let width = image.extent().width() as usize;
    image
        .pixels()
        .iter()
        .enumerate()
        .map(|(index, pixel)| {
            transform_output_pixel(
                *pixel,
                spec,
                [(index % width) as u32, (index / width) as u32],
            )
        })
        .collect()
}

pub fn tone_map(rgb: [f32; 3], method: ToneMap) -> Result<[f32; 3], OutputMathError> {
    if !rgb.iter().all(|channel| channel.is_finite()) {
        return Err(OutputMathError::NonFiniteOutput);
    }
    match method {
        ToneMap::None => Ok(rgb),
        ToneMap::ReinhardLuminance => {
            let luminance = rec2020_luminance(rgb);
            if luminance <= 0.0 {
                Ok(rgb)
            } else {
                let mapped = luminance / (1.0 + luminance);
                let scale = mapped / luminance;
                Ok(rgb.map(|channel| channel * scale))
            }
        }
    }
}

pub fn gamut_map(
    rgb: [f32; 3],
    primaries: ColorPrimaries,
    maximum: f32,
    method: GamutMap,
) -> Result<[f32; 3], OutputMathError> {
    if maximum <= 0.0 || !maximum.is_finite() || !rgb.iter().all(|channel| channel.is_finite()) {
        return Err(OutputMathError::NonFiniteOutput);
    }
    match method {
        GamutMap::Clip => Ok(rgb.map(|channel| channel.clamp(0.0, maximum))),
        GamutMap::ChromaCompress => {
            if rgb.iter().all(|channel| (0.0..=maximum).contains(channel)) {
                return Ok(rgb);
            }
            let luminance = target_luminance(rgb, primaries).clamp(0.0, maximum);
            let mut scale = 1.0f32;
            for channel in rgb {
                let delta = channel - luminance;
                if delta > 0.0 {
                    scale = scale.min((maximum - luminance) / delta);
                } else if delta < 0.0 {
                    scale = scale.min(luminance / -delta);
                }
            }
            Ok(rgb.map(|channel| (luminance + (channel - luminance) * scale).clamp(0.0, maximum)))
        }
    }
}

fn quantize(
    encoded: [f32; 4],
    spec: OutputSpec,
    position: [u32; 2],
) -> Result<OutputStorage, OutputMathError> {
    if !encoded.iter().all(|channel| channel.is_finite()) {
        return Err(OutputMathError::NonFiniteOutput);
    }
    match spec.bit_depth() {
        OutputBitDepth::Float16 => Ok(OutputStorage::Float16 {
            bits: encoded.map(f32_to_f16_bits),
        }),
        depth => {
            let bit_depth = integer_bit_depth(depth);
            let levels = ((1u32 << bit_depth) - 1) as f32;
            let mut codes = [0u16; 4];
            for channel in 0..4 {
                let noise = if channel == 3 {
                    0.0
                } else {
                    dither(spec.dither(), position, channel as u32) / levels
                };
                codes[channel] =
                    ((encoded[channel] + noise).clamp(0.0, 1.0) * levels).round() as u16;
            }
            Ok(OutputStorage::Integer { bit_depth, codes })
        }
    }
}

fn dither(method: Dither, position: [u32; 2], channel: u32) -> f32 {
    match method {
        Dither::None => 0.0,
        Dither::Triangular { seed } => {
            let key = seed
                ^ u64::from(position[0]).wrapping_mul(0x9e37_79b9_7f4a_7c15)
                ^ u64::from(position[1]).wrapping_mul(0xbf58_476d_1ce4_e5b9)
                ^ u64::from(channel).wrapping_mul(0x94d0_49bb_1331_11eb);
            unit_hash(key) - unit_hash(key ^ 0xd6e8_feb8_6659_fd93)
        }
    }
}

fn unit_hash(mut value: u64) -> f32 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^= value >> 31;
    ((value >> 40) as f32) / 16_777_216.0
}

fn target_linear_maximum(spec: OutputSpec) -> f32 {
    match spec.target().transfer {
        TransferFunction::Linear | TransferFunction::Srgb | TransferFunction::Rec709 => {
            f32::from(spec.reference_white().get()) / WORKING_REFERENCE_WHITE_NITS
        }
        TransferFunction::Pq | TransferFunction::Hlg => {
            f32::from(spec.peak_luminance().get()) / WORKING_REFERENCE_WHITE_NITS
        }
    }
}

fn rec2020_luminance(rgb: [f32; 3]) -> f32 {
    0.2627 * rgb[0] + 0.6780 * rgb[1] + 0.0593 * rgb[2]
}

fn target_luminance(rgb: [f32; 3], primaries: ColorPrimaries) -> f32 {
    let weights = match primaries {
        ColorPrimaries::Rec709 => [0.212_639, 0.715_169, 0.072_192],
        ColorPrimaries::DisplayP3 => [0.228_975, 0.691_739, 0.079_287],
        ColorPrimaries::Rec2020 => [0.2627, 0.6780, 0.0593],
    };
    weights[0] * rgb[0] + weights[1] * rgb[1] + weights[2] * rgb[2]
}

const fn integer_bit_depth(depth: OutputBitDepth) -> u8 {
    match depth {
        OutputBitDepth::Eight => 8,
        OutputBitDepth::Ten => 10,
        OutputBitDepth::Twelve => 12,
        OutputBitDepth::Sixteen => 16,
        OutputBitDepth::Float16 => unreachable!(),
    }
}

/// IEEE-754 round-to-nearest-even binary32 → binary16 conversion.
fn f32_to_f16_bits(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xff) as i32;
    let mantissa = bits & 0x7f_ff_ff;

    if exponent == 0xff {
        return sign | if mantissa == 0 { 0x7c00 } else { 0x7e00 };
    }
    let half_exponent = exponent - 127 + 15;
    if half_exponent >= 0x1f {
        return sign | 0x7c00;
    }
    if half_exponent <= 0 {
        if half_exponent < -10 {
            return sign;
        }
        let mantissa = mantissa | 0x80_00_00;
        let shift = (14 - half_exponent) as u32;
        let rounded = round_shift_even(mantissa, shift);
        return sign | rounded as u16;
    }
    let rounded = round_shift_even(mantissa, 13);
    if rounded == 0x400 {
        let exponent = half_exponent + 1;
        if exponent >= 0x1f {
            sign | 0x7c00
        } else {
            sign | ((exponent as u16) << 10)
        }
    } else {
        sign | ((half_exponent as u16) << 10) | rounded as u16
    }
}

fn round_shift_even(value: u32, shift: u32) -> u32 {
    let truncated = value >> shift;
    let remainder_mask = (1u32 << shift) - 1;
    let remainder = value & remainder_mask;
    let halfway = 1u32 << (shift - 1);
    truncated + u32::from(remainder > halfway || (remainder == halfway && truncated & 1 == 1))
}

#[derive(Debug, Error, Clone, PartialEq)]
pub enum OutputMathError {
    #[error(transparent)]
    Pixel(#[from] super::PixelError),
    #[error(transparent)]
    Color(#[from] ColorMathError),
    #[error("opaque output received a pixel with alpha {alpha}; root composition is incomplete")]
    OpaquePixelHasTransparency { alpha: f32 },
    #[error("output transform produced a non-finite channel")]
    NonFiniteOutput,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resource::{
        AuthorSrgbStraight, Dither, GamutMap, LuminanceNits, OutputBackground, OutputColorEncoding,
        SignalLuminance,
    };

    fn spec(
        target: OutputColorEncoding,
        alpha: OutputAlphaMode,
        tone_map: ToneMap,
        gamut_map: GamutMap,
        dither: Dither,
        depth: OutputBitDepth,
        luminance: SignalLuminance,
    ) -> OutputSpec {
        OutputSpec::new(
            target,
            alpha,
            if alpha == OutputAlphaMode::Opaque {
                OutputBackground::opaque_srgb([0, 0, 0])
            } else {
                OutputBackground::Transparent
            },
            tone_map,
            gamut_map,
            dither,
            depth,
            luminance,
        )
        .unwrap()
    }

    #[test]
    fn sdr_neutral_black_and_white_have_exact_integer_codes() {
        let output = spec(
            OutputColorEncoding::SRGB,
            OutputAlphaMode::Opaque,
            ToneMap::None,
            GamutMap::Clip,
            Dither::None,
            OutputBitDepth::Eight,
            SignalLuminance::SDR_100,
        );
        for (value, expected) in [(0.0, [0, 0, 0, 255]), (1.0, [255; 4])] {
            let sample = transform_output_pixel(
                PremulRgba32::from_straight([value; 3], 1.0).unwrap(),
                output,
                [0, 0],
            )
            .unwrap();
            assert_eq!(
                sample.storage,
                OutputStorage::Integer {
                    bit_depth: 8,
                    codes: expected,
                }
            );
        }
    }

    #[test]
    fn output_alpha_form_is_terminal_and_explicit() {
        let pixel = PremulRgba32::from_straight([0.5; 3], 0.5).unwrap();
        let straight = transform_output_pixel(
            pixel,
            spec(
                OutputColorEncoding::SRGB,
                OutputAlphaMode::StraightCoverage,
                ToneMap::None,
                GamutMap::Clip,
                Dither::None,
                OutputBitDepth::Sixteen,
                SignalLuminance::SDR_100,
            ),
            [0, 0],
        )
        .unwrap();
        let premul = transform_output_pixel(
            pixel,
            spec(
                OutputColorEncoding::SRGB,
                OutputAlphaMode::PremultipliedCoverage,
                ToneMap::None,
                GamutMap::Clip,
                Dither::None,
                OutputBitDepth::Sixteen,
                SignalLuminance::SDR_100,
            ),
            [0, 0],
        )
        .unwrap();
        assert!((premul.encoded[0] - straight.encoded[0] * 0.5).abs() < 1e-7);
        assert!(
            transform_output_pixel(
                pixel,
                spec(
                    OutputColorEncoding::SRGB,
                    OutputAlphaMode::Opaque,
                    ToneMap::None,
                    GamutMap::Clip,
                    Dither::None,
                    OutputBitDepth::Eight,
                    SignalLuminance::SDR_100,
                ),
                [0, 0]
            )
            .is_err()
        );
    }

    #[test]
    fn reinhard_is_a_declared_luminance_curve() {
        let mapped = tone_map([3.0; 3], ToneMap::ReinhardLuminance).unwrap();
        for channel in mapped {
            assert!((channel - 0.75).abs() < 1e-7);
        }
    }

    #[test]
    fn pq_encodes_working_white_as_absolute_one_hundred_nits() {
        let hdr = SignalLuminance::new(
            LuminanceNits::new(203).unwrap(),
            LuminanceNits::new(1000).unwrap(),
        )
        .unwrap();
        let output = spec(
            OutputColorEncoding::REC2020_PQ,
            OutputAlphaMode::Opaque,
            ToneMap::None,
            GamutMap::Clip,
            Dither::None,
            OutputBitDepth::Ten,
            hdr,
        );
        let sample = transform_output_pixel(
            PremulRgba32::from_straight([1.0; 3], 1.0).unwrap(),
            output,
            [0, 0],
        )
        .unwrap();
        for channel in &sample.encoded[..3] {
            assert!((*channel - 0.508_078).abs() < 2e-5, "{channel}");
        }
    }

    #[test]
    fn triangular_dither_is_seeded_and_position_stable() {
        let output = spec(
            OutputColorEncoding::SRGB,
            OutputAlphaMode::Opaque,
            ToneMap::None,
            GamutMap::Clip,
            Dither::Triangular { seed: 7 },
            OutputBitDepth::Eight,
            SignalLuminance::SDR_100,
        );
        let image = ReferenceImage::solid(
            crate::resource::Extent2d::new(8, 8).unwrap(),
            PremulRgba32::from_straight([0.18; 3], 1.0).unwrap(),
        )
        .unwrap();
        let first = transform_output_image(&image, output).unwrap();
        let second = transform_output_image(&image, output).unwrap();
        assert_eq!(first, second);
        let codes: Vec<_> = first
            .iter()
            .map(|sample| match sample.storage {
                OutputStorage::Integer { codes, .. } => codes[0],
                OutputStorage::Float16 { .. } => unreachable!(),
            })
            .collect();
        assert!(codes.windows(2).any(|pair| pair[0] != pair[1]));
    }

    #[test]
    fn float16_storage_uses_ieee_bits_without_dither() {
        assert_eq!(f32_to_f16_bits(0.0), 0x0000);
        assert_eq!(f32_to_f16_bits(1.0), 0x3c00);
        assert_eq!(f32_to_f16_bits(-2.0), 0xc000);
    }

    #[test]
    fn output_root_is_derived_only_from_explicit_background() {
        let transparent = OutputSpec::srgb_preview(OutputBackground::Transparent).unwrap();
        assert_eq!(
            output_root_pixel(transparent).unwrap(),
            PremulRgba32::TRANSPARENT
        );
        let opaque = OutputSpec::srgb_preview(OutputBackground::AuthorSrgbStraight {
            color: AuthorSrgbStraight([255, 0, 0, 255]),
        })
        .unwrap();
        assert_eq!(output_root_pixel(opaque).unwrap().alpha(), 1.0);
    }
}
