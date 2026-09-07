use skia_safe::{
    AlphaType, ColorSpace, ColorType, Image, ImageInfo, image::CachingHint, named_primaries,
    named_transfer_fn,
};
use valle_engine::{
    compositor::delivery::{DeliveryStorage, transform_working_pixel},
    resource::{ColorPrimaries, OutputAlphaMode, OutputBitDepth, OutputSpec, TransferFunction},
};

use super::{draw::DrawError, surface::working_color_space};

pub(crate) struct StagedOutput {
    pixels: Vec<u8>,
    row_bytes: usize,
}

impl StagedOutput {
    pub(crate) fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub(crate) const fn row_bytes(&self) -> usize {
        self.row_bytes
    }
}

/// Materializes the terminal OutputSpec exactly. Skia color-space defaults are intentionally not
/// involved: tone/gamut/transfer/alpha/dither/quantization are plan semantics, not target policy.
pub(crate) fn stage_output(
    image: &Image,
    spec: OutputSpec,
    target_info: &ImageInfo,
) -> Result<StagedOutput, DrawError> {
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
    let mut working = vec![0.0_f32; sample_count];
    if !image.read_pixels(
        &read_info,
        &mut working,
        row_bytes,
        (0, 0),
        CachingHint::Disallow,
    ) {
        return Err(DrawError::Surface("working output readback failed".into()));
    }

    let mut pixels = Vec::with_capacity(target_info.compute_min_byte_size());
    for (index, channels) in working.chunks_exact(4).enumerate() {
        let channels = canonical_premul([channels[0], channels[1], channels[2], channels[3]])?;
        let sample = transform_working_pixel(
            channels,
            spec,
            [(index % width) as u32, (index / width) as u32],
        )
        .map_err(|error| DrawError::Surface(error.to_string()))?;
        append_storage(&mut pixels, sample.storage, target_info.color_type(), spec)?;
    }
    let target_row_bytes = width
        .checked_mul(target_info.color_type().bytes_per_pixel())
        .ok_or_else(|| DrawError::Surface("target row is too large".into()))?;
    Ok(StagedOutput {
        pixels,
        row_bytes: target_row_bytes,
    })
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

fn canonical_premul(mut value: [f32; 4]) -> Result<[f32; 4], DrawError> {
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
