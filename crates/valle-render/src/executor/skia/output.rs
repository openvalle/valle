use skia_safe::{
    AlphaType, ColorSpace, ColorType, Image, ImageInfo, image::CachingHint, named_primaries,
    named_transfer_fn,
};
use valle_engine::{
    compositor::delivery::{DeliveryStorage, transform_working_pixel},
    resource::{
        ColorPrimaries, Dither, OutputAlphaMode, OutputBitDepth, OutputSpec, TransferFunction,
    },
};

use super::{draw::DrawError, surface::working_color_space};

#[cfg(test)]
pub(crate) struct StagedOutput {
    pixels: Vec<u8>,
}

#[cfg(test)]
impl StagedOutput {
    pub(crate) fn pixels(&self) -> &[u8] {
        &self.pixels
    }
}

/// Largest staged target pixel: four 16-bit channels.
const MAX_STAGED_PIXEL_BYTES: usize = 8;
type EncodedPixel = [u8; MAX_STAGED_PIXEL_BYTES];

/// Reusable CPU buffers for exact output staging. Capacity persists across frames; contents never
/// do, because every use fully overwrites the working readback and rebuilds the pixel bytes.
#[derive(Debug, Default)]
pub(crate) struct OutputStaging {
    working: Vec<f32>,
    pixels: Vec<u8>,
}

impl OutputStaging {
    pub(crate) fn pixels(&self) -> &[u8] {
        &self.pixels
    }
}

// Sample the existing raster pixels without readback. Continuous-tone frames (gradients/video)
// favor Skia's shader; flat-color graphics favor exact memoized conversion. This selects cost,
// never a different color contract.
pub(crate) fn prefers_cached_output(image: &Image) -> bool {
    let Some(pixmap) = image.peek_pixels() else {
        return false;
    };
    let Some(bytes) = pixmap.bytes() else {
        return false;
    };
    let width = pixmap.width() as usize;
    let height = pixmap.height() as usize;
    let columns = width.min(32);
    let rows = height.min(32);
    let pixel_bytes = pixmap.info().bytes_per_pixel();
    if columns == 0 || rows == 0 || pixel_bytes == 0 {
        return false;
    }
    let mut colors = std::collections::HashSet::new();
    for y in 0..rows {
        for x in 0..columns {
            let offset = ((2 * y + 1) * height / (2 * rows)) * pixmap.row_bytes()
                + ((2 * x + 1) * width / (2 * columns)) * pixel_bytes;
            colors.insert(&bytes[offset..offset + pixel_bytes]);
            if colors.len() > columns * rows / 4 {
                return false;
            }
        }
    }
    true
}

/// Materializes the terminal OutputSpec exactly. Skia color-space defaults are intentionally not
/// involved: tone/gamut/transfer/alpha/dither/quantization are plan semantics, not target policy.
#[cfg(test)]
pub(crate) fn stage_output(
    image: &Image,
    spec: OutputSpec,
    target_info: &ImageInfo,
) -> Result<StagedOutput, DrawError> {
    let mut staging = OutputStaging::default();
    stage_output_into(image, spec, target_info, &mut staging)?;
    Ok(StagedOutput {
        pixels: staging.pixels,
    })
}

/// Stage exact output into caller-owned buffers, returning the target row bytes. The pixels are
/// available through [`OutputStaging::pixels`] until the next call.
pub(crate) fn stage_output_into(
    image: &Image,
    spec: OutputSpec,
    target_info: &ImageInfo,
    staging: &mut OutputStaging,
) -> Result<usize, DrawError> {
    validate_target(spec, target_info)?;
    let width = usize::try_from(image.width())
        .map_err(|_| DrawError::Surface("negative output width".into()))?;
    let height = usize::try_from(image.height())
        .map_err(|_| DrawError::Surface("negative output height".into()))?;
    if image.width() != target_info.width() || image.height() != target_info.height() {
        return Err(DrawError::Surface("output extent mismatch".into()));
    }
    let read_info = ImageInfo::new(
        image.dimensions(),
        ColorType::RGBAF32,
        AlphaType::Premul,
        Some(working_color_space().map_err(|error| DrawError::Surface(error.to_string()))?),
    );
    let row_bytes = width
        .checked_mul(4)
        .and_then(|value| value.checked_mul(core::mem::size_of::<f32>()))
        .ok_or_else(|| DrawError::Surface("output row is too large".into()))?;
    let sample_count = width
        .checked_mul(height)
        .and_then(|value| value.checked_mul(4))
        .ok_or_else(|| DrawError::Surface("output image is too large".into()))?;
    // read_pixels overwrites every sample, so stale contents from a prior frame are never read.
    let working = &mut staging.working;
    working.resize(sample_count, 0.0);
    if !image.read_pixels(
        &read_info,
        working.as_mut_slice(),
        row_bytes,
        (0, 0),
        CachingHint::Disallow,
    ) {
        return Err(DrawError::Surface("working output readback failed".into()));
    }

    let color_type = target_info.color_type();
    let pixel_bytes = color_type.bytes_per_pixel();
    if pixel_bytes == 0 || pixel_bytes > MAX_STAGED_PIXEL_BYTES {
        return Err(DrawError::Surface(format!(
            "unsupported staged output pixel size for {color_type:?}"
        )));
    }
    let pixels = &mut staging.pixels;
    pixels.clear();
    pixels.resize(sample_count / 4 * pixel_bytes, 0);
    // Non-dithered conversion is position-independent. A small, frame-local cache avoids
    // repeating transfer/gamut math for flat fills and text without approximating any pixels.
    // Entries hold the final encoded bytes, so hits are a single fixed-size copy.
    let cacheable = spec.dither() == Dither::None;
    // Sized for a full row of a horizontal gradient (1920 distinct inputs at 1080p), which a
    // smaller direct-mapped cache would evict every row.
    let mut cache = vec![None::<([u32; 4], EncodedPixel)>; SDR_CACHE_SLOTS];
    // Flat regions repeat the previous pixel; check it before hashing.
    let mut previous = None::<([u32; 4], EncodedPixel)>;
    let mut encode_buffer = Vec::with_capacity(MAX_STAGED_PIXEL_BYTES);
    for (index, (channels, output)) in working
        .chunks_exact(4)
        .zip(pixels.chunks_exact_mut(pixel_bytes))
        .enumerate()
    {
        let channels = [channels[0], channels[1], channels[2], channels[3]];
        let key = channels.map(f32::to_bits);
        if cacheable {
            if let Some((prior, encoded)) = &previous {
                if *prior == key {
                    output.copy_from_slice(&encoded[..pixel_bytes]);
                    continue;
                }
            }
        }
        let hash = key
            .iter()
            .fold(0_u32, |hash, bits| hash.rotate_left(5) ^ bits)
            .wrapping_mul(0x9e3779b9);
        let slot = (hash >> (32 - SDR_CACHE_BITS)) as usize;
        let cached = if cacheable {
            cache[slot].filter(|(prior, _)| *prior == key)
        } else {
            None
        };
        let encoded = if let Some((_, encoded)) = cached {
            encoded
        } else {
            let sample = transform_working_pixel(
                canonical_rgba(channels)?,
                spec,
                [(index % width) as u32, (index / width) as u32],
            )
            .map_err(|error| DrawError::Surface(error.to_string()))?;
            encode_buffer.clear();
            append_storage(&mut encode_buffer, sample.storage, color_type, spec)?;
            if encode_buffer.len() != pixel_bytes {
                return Err(DrawError::Surface(format!(
                    "{color_type:?} staging produced {} bytes per pixel",
                    encode_buffer.len()
                )));
            }
            let mut encoded = [0_u8; MAX_STAGED_PIXEL_BYTES];
            encoded[..pixel_bytes].copy_from_slice(&encode_buffer);
            if cacheable {
                cache[slot] = Some((key, encoded));
            }
            encoded
        };
        output.copy_from_slice(&encoded[..pixel_bytes]);
        if cacheable {
            previous = Some((key, encoded));
        }
    }
    width
        .checked_mul(color_type.bytes_per_pixel())
        .ok_or_else(|| DrawError::Surface("target row is too large".into()))
}

/// Batch SDR output: Skia converts the linear primaries, shared delivery math maps the gamut,
/// then a bounded curve lookup replaces per-pixel transfer powers. No platform gamma defaults
/// are involved; Skia's named Rec.709 transfer is a display EOTF, not our output OETF.
#[cfg(test)]
pub(crate) fn stage_sdr_output(
    image: &Image,
    spec: OutputSpec,
    target_info: &ImageInfo,
) -> Result<Option<StagedOutput>, DrawError> {
    let mut staging = OutputStaging::default();
    Ok(
        stage_sdr_output_into(image, spec, target_info, &mut staging)?.map(|_| StagedOutput {
            pixels: staging.pixels,
        }),
    )
}

/// Stage SDR output into caller-owned buffers, returning the target row bytes when the SDR
/// path applies. The pixels are available through [`OutputStaging::pixels`] until the next call.
pub(crate) fn stage_sdr_output_into(
    image: &Image,
    spec: OutputSpec,
    target_info: &ImageInfo,
    staging: &mut OutputStaging,
) -> Result<Option<usize>, DrawError> {
    use valle_engine::resource::ToneMap;
    if spec.bit_depth() != OutputBitDepth::Eight
        || spec.dither() != Dither::None
        || spec.tone_map() != ToneMap::None
        || !matches!(
            spec.target().transfer,
            TransferFunction::Linear | TransferFunction::Srgb | TransferFunction::Rec709
        )
    {
        return Ok(None);
    }
    validate_target(spec, target_info)?;
    if image.dimensions() != target_info.dimensions() {
        return Err(DrawError::Surface("output extent mismatch".into()));
    }
    let primaries = match spec.target().primaries {
        ColorPrimaries::Rec709 => named_primaries::CicpId::Rec709,
        ColorPrimaries::DisplayP3 => named_primaries::CicpId::SMPTE_EG_432_1,
        ColorPrimaries::Rec2020 => named_primaries::CicpId::Rec2020,
    };
    let linear = ColorSpace::new_cicp(primaries, named_transfer_fn::CicpId::Linear)
        .ok_or_else(|| DrawError::Surface("unsupported linear output color space".into()))?;
    let info = ImageInfo::new(
        image.dimensions(),
        ColorType::RGBAF32,
        AlphaType::Unpremul,
        Some(linear),
    );
    let width = image.width() as usize;
    let pixel_count = width * image.height() as usize;
    // read_pixels overwrites every sample, so stale contents from a prior frame are never read.
    let samples = &mut staging.working;
    samples.resize(pixel_count * 4, 0.0);
    if !image.read_pixels(
        &info,
        samples.as_mut_slice(),
        width * 16,
        (0, 0),
        CachingHint::Disallow,
    ) {
        return Err(DrawError::Surface("linear output conversion failed".into()));
    }
    // Loop invariants, queried once rather than per pixel.
    let maximum = f32::from(spec.reference_white().get()) / 100.0;
    let curve = transfer_lut(spec.target().transfer);
    let primaries = spec.target().primaries;
    let gamut_map = spec.gamut_map();
    let alpha_mode = spec.alpha();
    let swap_red_blue = target_info.color_type() == ColorType::BGRA8888;
    let row_bytes = target_info.min_row_bytes();
    let pixels = &mut staging.pixels;
    pixels.clear();
    pixels.resize(pixel_count * 4, 0);
    // Continuous-tone frames still repeat neighbors in flat regions; an identical input reuses
    // the previous pixel's already validated codes.
    let mut previous: Option<([u32; 4], [u8; 4])> = None;
    // Gradients repeat a color down a column rather than along a row, so a small exact-key cache
    // catches what the previous-pixel check misses. Keys are the input bits, so hits are exact.
    let mut cache = vec![None::<([u32; 4], [u8; 4])>; SDR_CACHE_SLOTS];
    for (pixel, output) in samples.chunks_exact(4).zip(pixels.chunks_exact_mut(4)) {
        let key = [
            pixel[0].to_bits(),
            pixel[1].to_bits(),
            pixel[2].to_bits(),
            pixel[3].to_bits(),
        ];
        if let Some((prior, codes)) = previous
            && prior == key
        {
            output.copy_from_slice(&codes);
            continue;
        }
        let slot = (key
            .iter()
            .fold(0_u32, |hash, bits| hash.rotate_left(5) ^ bits)
            .wrapping_mul(0x9e37_79b9)
            >> (32 - SDR_CACHE_BITS)) as usize;
        if let Some((prior, codes)) = cache[slot]
            && prior == key
        {
            output.copy_from_slice(&codes);
            previous = Some((key, codes));
            continue;
        }
        let pixel = canonical_rgba([pixel[0], pixel[1], pixel[2], pixel[3]])?;
        if alpha_mode == OutputAlphaMode::Opaque && pixel[3] != 1.0 {
            return Err(DrawError::Surface("opaque output has transparency".into()));
        }
        let mapped = valle_engine::compositor::delivery::map_output_gamut(
            [pixel[0], pixel[1], pixel[2]],
            primaries,
            maximum,
            gamut_map,
        )
        .map_err(|error| DrawError::Surface(error.to_string()))?;
        let coverage = if alpha_mode == OutputAlphaMode::PremultipliedCoverage {
            pixel[3]
        } else {
            1.0
        };
        let code = |value: f32| round_code(transfer_code(value / maximum, curve) * coverage);
        let (red, blue) = if swap_red_blue { (2, 0) } else { (0, 2) };
        let codes = [
            code(mapped[red]),
            code(mapped[1]),
            code(mapped[blue]),
            round_code(pixel[3] * 255.0),
        ];
        output.copy_from_slice(&codes);
        previous = Some((key, codes));
        cache[slot] = Some((key, codes));
    }
    Ok(Some(row_bytes))
}

const SDR_CACHE_BITS: u32 = 12;
const SDR_CACHE_SLOTS: usize = 1 << SDR_CACHE_BITS;

/// `value.round() as u8` for the non-negative code range, without a libm `roundf` call: the
/// truncation and its remainder are exact in f32 here, so ties still round away from zero.
fn round_code(value: f32) -> u8 {
    let truncated = value as u32;
    let rounded = truncated + u32::from(value - truncated as f32 >= 0.5);
    rounded.min(255) as u8
}

// The bounded [0,1] SDR curve is shared across frames and output primaries/white levels.
// Linear interpolation is within 0.07 of one 8-bit code (including Rec.709's join).
// HDR, dither and higher bit depths retain the exact delivery implementation.
fn transfer_lut(transfer: TransferFunction) -> Option<&'static [f32]> {
    use std::sync::OnceLock;
    static SRGB: OnceLock<Vec<f32>> = OnceLock::new();
    static REC709: OnceLock<Vec<f32>> = OnceLock::new();
    let cache = match transfer {
        TransferFunction::Linear => return None,
        TransferFunction::Srgb => &SRGB,
        TransferFunction::Rec709 => &REC709,
        _ => unreachable!("SDR output admitted a non-SDR transfer"),
    };
    Some(
        cache
            .get_or_init(|| {
                (0..=4096)
                    .map(|i| {
                        let v = i as f64 / 4096.0;
                        let encoded = if transfer == TransferFunction::Srgb {
                            if v <= 0.0031308 {
                                v * 12.92
                            } else {
                                1.055 * v.powf(1.0 / 2.4) - 0.055
                            }
                        } else if v <= 0.018 {
                            v * 4.5
                        } else {
                            1.099 * v.powf(0.45) - 0.099
                        };
                        (encoded * 255.0) as f32
                    })
                    .collect()
            })
            .as_slice(),
    )
}

fn transfer_code(value: f32, curve: Option<&[f32]>) -> f32 {
    let Some(curve) = curve else {
        return value * 255.0;
    };
    let position = value * 4096.0;
    let index = (position as usize).min(4095);
    curve[index] + (curve[index + 1] - curve[index]) * (position - index as f32)
}

pub(crate) fn validate_target(spec: OutputSpec, info: &ImageInfo) -> Result<(), DrawError> {
    let alpha_ok = match spec.alpha() {
        OutputAlphaMode::Opaque => {
            matches!(info.alpha_type(), AlphaType::Opaque | AlphaType::Premul)
        }
        OutputAlphaMode::StraightCoverage => info.alpha_type() == AlphaType::Unpremul,
        OutputAlphaMode::PremultipliedCoverage => info.alpha_type() == AlphaType::Premul,
    };
    if !alpha_ok {
        return Err(DrawError::Surface(
            "target alpha type does not match OutputSpec".into(),
        ));
    }
    let color_ok = match spec.bit_depth() {
        OutputBitDepth::Eight => matches!(
            info.color_type(),
            ColorType::RGBA8888 | ColorType::BGRA8888 | ColorType::SRGBA8888
        ),
        OutputBitDepth::Ten => matches!(
            info.color_type(),
            ColorType::RGBA10x6 | ColorType::RGBA1010102 | ColorType::RGB101010x
        ),
        OutputBitDepth::Twelve | OutputBitDepth::Sixteen => {
            info.color_type() == ColorType::R16G16B16A16UNorm
        }
        OutputBitDepth::Float16 => info.color_type() == ColorType::RGBAF16,
    };
    if !color_ok {
        return Err(DrawError::Surface(
            "target color type does not match OutputSpec bit depth".into(),
        ));
    }
    let expected_space = output_color_space(spec)?;
    if info.color_space().as_ref() != Some(&expected_space) {
        return Err(DrawError::Surface(
            "target color space does not match OutputSpec".into(),
        ));
    }
    if matches!(
        info.color_type(),
        ColorType::RGBA1010102 | ColorType::RGB101010x
    ) && spec.alpha() != OutputAlphaMode::Opaque
    {
        return Err(DrawError::Surface(
            "10-bit packed target cannot preserve coverage alpha".into(),
        ));
    }
    Ok(())
}

pub(crate) fn output_color_space(spec: OutputSpec) -> Result<ColorSpace, DrawError> {
    let primaries = match spec.target().primaries {
        ColorPrimaries::Rec709 => named_primaries::CicpId::Rec709,
        ColorPrimaries::DisplayP3 => named_primaries::CicpId::SMPTE_EG_432_1,
        ColorPrimaries::Rec2020 => named_primaries::CicpId::Rec2020,
    };
    let transfer = match spec.target().transfer {
        TransferFunction::Linear => named_transfer_fn::CicpId::Linear,
        TransferFunction::Srgb => named_transfer_fn::CicpId::IEC61966_2_1,
        TransferFunction::Rec709 => named_transfer_fn::CicpId::Rec709,
        TransferFunction::Pq => named_transfer_fn::CicpId::PQ,
        TransferFunction::Hlg => named_transfer_fn::CicpId::HLG,
    };
    ColorSpace::new_cicp(primaries, transfer)
        .ok_or_else(|| DrawError::Surface("unsupported OutputSpec color space".into()))
}

pub(crate) fn rgba8_target_info(
    spec: OutputSpec,
    extent: valle_engine::resource::Extent2d,
) -> Result<ImageInfo, DrawError> {
    if spec.bit_depth() != OutputBitDepth::Eight {
        return Err(DrawError::Surface(
            "RGBA8 host target requires an eight-bit OutputSpec".into(),
        ));
    }
    let alpha = match spec.alpha() {
        OutputAlphaMode::Opaque => AlphaType::Opaque,
        OutputAlphaMode::StraightCoverage => AlphaType::Unpremul,
        OutputAlphaMode::PremultipliedCoverage => AlphaType::Premul,
    };
    let info = ImageInfo::new(
        (
            i32::try_from(extent.width())
                .map_err(|_| DrawError::Surface("target width exceeds Skia range".into()))?,
            i32::try_from(extent.height())
                .map_err(|_| DrawError::Surface("target height exceeds Skia range".into()))?,
        ),
        ColorType::RGBA8888,
        alpha,
        Some(output_color_space(spec)?),
    );
    validate_target(spec, &info)?;
    Ok(info)
}

fn canonical_rgba(mut value: [f32; 4]) -> Result<[f32; 4], DrawError> {
    if !value.iter().all(|channel| channel.is_finite()) {
        return Err(DrawError::Surface("non-finite working output".into()));
    }
    if value[3].abs() <= f32::EPSILON {
        return Ok([0.0; 4]);
    }
    if (value[3] - 1.0).abs() <= f32::EPSILON {
        value[3] = 1.0;
    }
    if !(0.0..=1.0).contains(&value[3]) {
        return Err(DrawError::Surface("working alpha is outside [0, 1]".into()));
    }
    Ok(value)
}

fn append_storage(
    output: &mut Vec<u8>,
    storage: DeliveryStorage,
    color_type: ColorType,
    spec: OutputSpec,
) -> Result<(), DrawError> {
    match storage {
        DeliveryStorage::Float16 { bits } => {
            if color_type != ColorType::RGBAF16 {
                return Err(DrawError::Surface("Float16 storage target mismatch".into()));
            }
            append_u16s(output, bits);
        }
        DeliveryStorage::Integer { bit_depth, codes } => match (bit_depth, color_type) {
            (8, ColorType::RGBA8888 | ColorType::SRGBA8888) => {
                output.extend(codes.map(|value| value as u8));
            }
            (8, ColorType::BGRA8888) => {
                output.extend([
                    codes[2] as u8,
                    codes[1] as u8,
                    codes[0] as u8,
                    codes[3] as u8,
                ]);
            }
            (10, ColorType::RGBA10x6) => append_u16s(output, codes.map(|value| value << 6)),
            (10, ColorType::RGBA1010102) => {
                let packed = u32::from(codes[0])
                    | (u32::from(codes[1]) << 10)
                    | (u32::from(codes[2]) << 20)
                    | (3 << 30);
                output.extend_from_slice(&packed.to_le_bytes());
            }
            (10, ColorType::RGB101010x) => {
                let packed =
                    u32::from(codes[0]) | (u32::from(codes[1]) << 10) | (u32::from(codes[2]) << 20);
                output.extend_from_slice(&packed.to_le_bytes());
            }
            (12 | 16, ColorType::R16G16B16A16UNorm) => {
                let maximum = (1_u32 << bit_depth) - 1;
                append_u16s(
                    output,
                    codes.map(|value| ((u32::from(value) * 65_535 + maximum / 2) / maximum) as u16),
                );
            }
            _ => {
                return Err(DrawError::Surface(format!(
                    "{}-bit output cannot be stored in {color_type:?}",
                    match spec.bit_depth() {
                        OutputBitDepth::Eight => 8,
                        OutputBitDepth::Ten => 10,
                        OutputBitDepth::Twelve => 12,
                        OutputBitDepth::Sixteen => 16,
                        OutputBitDepth::Float16 => 16,
                    }
                )));
            }
        },
    }
    Ok(())
}

fn append_u16s(output: &mut Vec<u8>, values: [u16; 4]) {
    for value in values {
        output.extend_from_slice(&value.to_le_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use skia_safe::{Data, images};
    use valle_engine::resource::{
        GamutMap, OutputBackground, OutputColorEncoding, SignalLuminance, ToneMap,
    };

    #[test]
    fn sdr_curve_interpolation_stays_below_a_tenth_of_one_code() {
        for transfer in [TransferFunction::Srgb, TransferFunction::Rec709] {
            let lut = transfer_lut(transfer);
            for i in 0..=262144 {
                let value = i as f32 / 262144.0;
                let v = f64::from(value);
                let exact = if transfer == TransferFunction::Srgb {
                    if v <= 0.0031308 {
                        v * 12.92
                    } else {
                        1.055 * v.powf(1.0 / 2.4) - 0.055
                    }
                } else if v <= 0.018 {
                    v * 4.5
                } else {
                    1.099 * v.powf(0.45) - 0.099
                };
                assert!((f64::from(transfer_code(value, lut)) - exact * 255.0).abs() < 0.07);
            }
        }
    }

    #[test]
    fn native_sdr_output_matches_reference_for_gamut_alpha_and_luminance() {
        use valle_engine::resource::LuminanceNits;
        for primaries in [
            ColorPrimaries::Rec709,
            ColorPrimaries::DisplayP3,
            ColorPrimaries::Rec2020,
        ] {
            for transfer in [
                TransferFunction::Linear,
                TransferFunction::Srgb,
                TransferFunction::Rec709,
            ] {
                for alpha in [
                    OutputAlphaMode::Opaque,
                    OutputAlphaMode::StraightCoverage,
                    OutputAlphaMode::PremultipliedCoverage,
                ] {
                    for gamut in [GamutMap::Clip, GamutMap::ChromaCompress] {
                        for white in [100, 203] {
                            let background = if alpha == OutputAlphaMode::Opaque {
                                OutputBackground::opaque_srgb([0, 0, 0])
                            } else {
                                OutputBackground::Transparent
                            };
                            let nits = LuminanceNits::new(white).unwrap();
                            let spec = OutputSpec::new(
                                OutputColorEncoding {
                                    primaries,
                                    transfer,
                                },
                                alpha,
                                background,
                                ToneMap::None,
                                gamut,
                                Dither::None,
                                OutputBitDepth::Eight,
                                SignalLuminance::new(nits, nits).unwrap(),
                            )
                            .unwrap();
                            let samples = (0..1024)
                                .flat_map(|i| {
                                    let a = if alpha == OutputAlphaMode::Opaque {
                                        1.0
                                    } else {
                                        (i % 31) as f32 / 30.0
                                    };
                                    [
                                        ((i % 257) as f32 / 100.0 - 0.3) * a,
                                        ((i % 71) as f32 / 40.0) * a,
                                        ((i % 19) as f32 / 10.0 - 0.1) * a,
                                        a,
                                    ]
                                })
                                .collect::<Vec<_>>();
                            let bytes = samples
                                .iter()
                                .flat_map(|value| value.to_ne_bytes())
                                .collect::<Vec<_>>();
                            let info = ImageInfo::new(
                                (32, 32),
                                ColorType::RGBAF32,
                                AlphaType::Premul,
                                Some(working_color_space().unwrap()),
                            );
                            let image =
                                images::raster_from_data(&info, Data::new_copy(&bytes), 32 * 16)
                                    .unwrap();
                            let target = rgba8_target_info(
                                spec,
                                valle_engine::resource::Extent2d::new(32, 32).unwrap(),
                            )
                            .unwrap();
                            let actual = stage_sdr_output(&image, spec, &target).unwrap().unwrap();
                            let expected = stage_output(&image, spec, &target).unwrap();
                            let maximum = actual
                                .pixels()
                                .iter()
                                .zip(expected.pixels())
                                .map(|(a, b)| a.abs_diff(*b))
                                .max()
                                .unwrap();
                            let mse = actual
                                .pixels()
                                .iter()
                                .zip(expected.pixels())
                                .map(|(a, b)| (f64::from(*a) - f64::from(*b)).powi(2))
                                .sum::<f64>()
                                / actual.pixels().len() as f64;
                            assert!(
                                maximum <= 2 && mse <= 0.65,
                                "{spec:?}: max={maximum}, mse={mse}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn continuous_color_frames_keep_the_shader_path() {
        let info = ImageInfo::new((64, 64), ColorType::RGBA8888, AlphaType::Premul, None);
        let flat = [32, 64, 128, 255].repeat(64 * 64);
        let ramp = (0..64 * 64)
            .flat_map(|i| [(i % 256) as u8, (i / 256) as u8, 0, 255])
            .collect::<Vec<_>>();
        for (bytes, expected) in [(flat, true), (ramp, false)] {
            let image = images::raster_from_data(&info, Data::new_copy(&bytes), 64 * 4).unwrap();
            assert_eq!(prefers_cached_output(&image), expected);
        }
    }

    #[test]
    fn cached_output_matches_per_pixel_math_including_collisions_and_dither() {
        for dither in [Dither::None, Dither::Triangular { seed: 71 }] {
            let spec = OutputSpec::new(
                OutputColorEncoding::SRGB,
                OutputAlphaMode::Opaque,
                OutputBackground::opaque_srgb([0, 0, 0]),
                ToneMap::None,
                GamutMap::ChromaCompress,
                dither,
                OutputBitDepth::Eight,
                SignalLuminance::SDR_100,
            )
            .unwrap();
            let samples = (0..1024)
                .map(|i| {
                    if i % 3 == 0 {
                        [0.18, 0.18, 0.18, 1.0]
                    } else {
                        [
                            (i % 257) as f32 / 200.0,
                            (i % 71) as f32 / 70.0,
                            (i % 19) as f32 / 18.0,
                            1.0,
                        ]
                    }
                })
                .collect::<Vec<_>>();
            let data = samples
                .iter()
                .flatten()
                .flat_map(|value| value.to_ne_bytes())
                .collect::<Vec<_>>();
            let info = ImageInfo::new(
                (32, 32),
                ColorType::RGBAF32,
                AlphaType::Premul,
                Some(working_color_space().unwrap()),
            );
            let image = images::raster_from_data(&info, Data::new_copy(&data), 32 * 16).unwrap();
            let target =
                rgba8_target_info(spec, valle_engine::resource::Extent2d::new(32, 32).unwrap())
                    .unwrap();
            let actual = stage_output(&image, spec, &target).unwrap();
            let mut expected = Vec::new();
            for (index, pixel) in samples.iter().enumerate() {
                let sample = transform_working_pixel(
                    *pixel,
                    spec,
                    [(index % 32) as u32, (index / 32) as u32],
                )
                .unwrap();
                append_storage(&mut expected, sample.storage, target.color_type(), spec).unwrap();
            }
            assert_eq!(actual.pixels(), expected, "dither={dither:?}");
        }
    }

    #[test]
    fn repeated_pixels_match_per_pixel_math_for_every_staged_format() {
        let formats = [
            (OutputBitDepth::Eight, ColorType::RGBA8888),
            (OutputBitDepth::Eight, ColorType::BGRA8888),
            (OutputBitDepth::Ten, ColorType::RGBA1010102),
            (OutputBitDepth::Sixteen, ColorType::R16G16B16A16UNorm),
            (OutputBitDepth::Float16, ColorType::RGBAF16),
        ];
        // Runs of identical pixels exercise the previous-pixel path; the varied segments between
        // them exercise the hashed cache and misses.
        let palette = [
            [0.18, 0.18, 0.18, 1.0],
            [0.9, 0.1, 0.05, 1.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let samples = (0..1024)
            .map(|i| {
                let run = i / 7;
                if run % 2 == 0 {
                    palette[run % palette.len()]
                } else {
                    [(i % 13) as f32 / 12.0, (i % 5) as f32 / 4.0, 0.5, 1.0]
                }
            })
            .collect::<Vec<_>>();
        let data = samples
            .iter()
            .flatten()
            .flat_map(|value| value.to_ne_bytes())
            .collect::<Vec<_>>();
        let source = ImageInfo::new(
            (32, 32),
            ColorType::RGBAF32,
            AlphaType::Premul,
            Some(working_color_space().unwrap()),
        );
        let image = images::raster_from_data(&source, Data::new_copy(&data), 32 * 16).unwrap();
        let mut checked = 0;
        for (bit_depth, color_type) in formats {
            for dither in [Dither::None, Dither::Triangular { seed: 5 }] {
                let Ok(spec) = OutputSpec::new(
                    OutputColorEncoding::SRGB,
                    OutputAlphaMode::Opaque,
                    OutputBackground::opaque_srgb([0, 0, 0]),
                    ToneMap::None,
                    GamutMap::ChromaCompress,
                    dither,
                    bit_depth,
                    SignalLuminance::SDR_100,
                ) else {
                    continue;
                };
                let target = ImageInfo::new(
                    (32, 32),
                    color_type,
                    AlphaType::Opaque,
                    Some(output_color_space(spec).unwrap()),
                );
                let actual = stage_output(&image, spec, &target).unwrap();
                let mut expected = Vec::new();
                for (index, pixel) in samples.iter().enumerate() {
                    let sample = transform_working_pixel(
                        *pixel,
                        spec,
                        [(index % 32) as u32, (index / 32) as u32],
                    )
                    .unwrap();
                    append_storage(&mut expected, sample.storage, color_type, spec).unwrap();
                }
                assert_eq!(
                    actual.pixels(),
                    expected,
                    "{bit_depth:?} {color_type:?} dither={dither:?}"
                );
                checked += 1;
            }
        }
        // Float16 cannot dither; every other format runs both modes.
        assert_eq!(checked, formats.len() * 2 - 1);
    }

    #[test]
    fn staging_reuse_overwrites_pixels_across_sizes_formats_and_output_paths() {
        let mut staging = OutputStaging::default();
        // Grow and shrink the same buffers, switch both delivery paths, then reuse a prior size.
        for (width, height, bit_depth, color_type, transfer) in [
            (
                32,
                8,
                OutputBitDepth::Eight,
                ColorType::RGBA8888,
                TransferFunction::Srgb,
            ),
            (
                1,
                1,
                OutputBitDepth::Sixteen,
                ColorType::R16G16B16A16UNorm,
                TransferFunction::Linear,
            ),
            (
                17,
                3,
                OutputBitDepth::Eight,
                ColorType::BGRA8888,
                TransferFunction::Rec709,
            ),
            (
                64,
                9,
                OutputBitDepth::Float16,
                ColorType::RGBAF16,
                TransferFunction::Linear,
            ),
            (
                32,
                8,
                OutputBitDepth::Eight,
                ColorType::RGBA8888,
                TransferFunction::Srgb,
            ),
        ] {
            let spec = OutputSpec::new(
                OutputColorEncoding {
                    primaries: ColorPrimaries::Rec709,
                    transfer,
                },
                OutputAlphaMode::Opaque,
                OutputBackground::opaque_srgb([0, 0, 0]),
                ToneMap::None,
                GamutMap::ChromaCompress,
                Dither::None,
                bit_depth,
                SignalLuminance::SDR_100,
            )
            .unwrap();
            let samples = (0..width * height)
                .map(|i| {
                    [
                        (i % 7) as f32 / 8.0,
                        (i % 5) as f32 / 6.0,
                        (i % 3) as f32 / 4.0,
                        1.0,
                    ]
                })
                .collect::<Vec<_>>();
            let data = samples
                .iter()
                .flatten()
                .flat_map(|value| value.to_ne_bytes())
                .collect::<Vec<_>>();
            let source = ImageInfo::new(
                (width, height),
                ColorType::RGBAF32,
                AlphaType::Premul,
                Some(working_color_space().unwrap()),
            );
            let image =
                images::raster_from_data(&source, Data::new_copy(&data), width as usize * 16)
                    .unwrap();
            let target = ImageInfo::new(
                (width, height),
                color_type,
                AlphaType::Opaque,
                Some(output_color_space(spec).unwrap()),
            );
            let mut expected = Vec::new();
            for pixel in &samples {
                let sample = transform_working_pixel(*pixel, spec, [0, 0]).unwrap();
                append_storage(&mut expected, sample.storage, color_type, spec).unwrap();
            }
            // Poison both buffers so an incomplete overwrite is visible in this frame.
            staging.working.fill(f32::NAN);
            staging.pixels.fill(0xa5);
            assert_eq!(
                stage_output_into(&image, spec, &target, &mut staging).unwrap(),
                target.min_row_bytes()
            );
            assert_eq!(staging.pixels(), expected);
            if bit_depth == OutputBitDepth::Eight {
                let fresh = stage_sdr_output(&image, spec, &target).unwrap().unwrap();
                staging.working.fill(f32::NAN);
                staging.pixels.fill(0xa5);
                assert_eq!(
                    stage_sdr_output_into(&image, spec, &target, &mut staging).unwrap(),
                    Some(target.min_row_bytes())
                );
                assert_eq!(staging.pixels(), fresh.pixels());
            }
        }
    }
}
