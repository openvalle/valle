use skia_safe::{
    AlphaType, ColorType, Image, ImageInfo, RuntimeEffect, Surface, image::CachingHint,
};
use valle_draw::program::BlendMode as DrawBlend;
use valle_engine::compositor::reference::{ReferenceBlendMode, blend_rgb};

use super::{draw::DrawError, effect::render_runtime_into, surface::working_color_space};

const BLEND_SOURCE: &str = include_str!("../../../../valle-draw/assets/shaders/creativeblend.sksl");

#[derive(Debug)]
pub(crate) struct BlendRuntime {
    effect: RuntimeEffect,
}

impl BlendRuntime {
    pub(crate) fn admit() -> Result<Self, DrawError> {
        let effect = RuntimeEffect::make_for_shader(BLEND_SOURCE, None).map_err(|message| {
            DrawError::ShaderCompile {
                uri: "builtin://creative-blend".into(),
                message,
            }
        })?;
        Ok(Self { effect })
    }

    pub(crate) fn effective_source_into(
        &self,
        output: &mut Surface,
        source: &Image,
        destination: &Image,
        mode: DrawBlend,
    ) -> Result<(), DrawError> {
        if raster_blend_into(output, source, destination, mode, 1.0, false)? {
            return Ok(());
        }
        render_runtime_into(
            output,
            &self.effect,
            &[source, destination],
            &[draw_code(mode), 1.0, 0.0],
        )
    }

    pub(crate) fn composite_into(
        &self,
        output: &mut Surface,
        source: &Image,
        destination: &Image,
        mode: DrawBlend,
        opacity: f32,
    ) -> Result<(), DrawError> {
        if raster_blend_into(output, source, destination, mode, opacity, true)? {
            return Ok(());
        }
        render_runtime_into(
            output,
            &self.effect,
            &[source, destination],
            &[draw_code(mode), opacity, 1.0],
        )
    }
}

/// CPU implementation of `creativeblend.sksl` for raster targets. On the CPU backend Skia
/// interprets runtime effects stage by stage, which made one full-frame blend cost hundreds of
/// milliseconds. This evaluates the same formula natively: the blend functions are the Engine
/// reference `blend_rgb`, and the extended-sRGB transfer uses a dense table inside [0, 2] (the
/// SkSL kernel already used an approximate `pow`). Returns `false` when the inputs cannot take
/// this path, so the caller falls back to the kernel.
fn raster_blend_into(
    output: &mut Surface,
    source: &Image,
    destination: &Image,
    mode: DrawBlend,
    opacity: f32,
    composite: bool,
) -> Result<bool, DrawError> {
    let extent = output.image_info().dimensions();
    if output.peek_pixels().is_none()
        || source.is_texture_backed()
        || destination.is_texture_backed()
        || source.dimensions() != extent
        || destination.dimensions() != extent
        || !(0.0..=1.0).contains(&opacity)
    {
        return Ok(false);
    }
    let width = usize::try_from(extent.width).map_err(|_| internal("negative blend width"))?;
    let height = usize::try_from(extent.height).map_err(|_| internal("negative blend height"))?;
    let row_bytes = width
        .checked_mul(16)
        .ok_or_else(|| internal("blend row is too large"))?;
    let byte_count = row_bytes
        .checked_mul(height)
        .ok_or_else(|| internal("blend image is too large"))?;
    let info = ImageInfo::new(
        extent,
        ColorType::RGBAF32,
        AlphaType::Premul,
        Some(working_color_space().map_err(|error| internal(&error.to_string()))?),
    );
    let reference_mode = ReferenceBlendMode::from(mode);
    let tables = transfer_tables();
    BLEND_SCRATCH.with_borrow_mut(|(pixels, backdrop)| {
        pixels.resize(byte_count, 0);
        backdrop.resize(byte_count, 0);
        // Both reads overwrite every byte, so capacity reused from a previous frame is never read.
        if !source.read_pixels(&info, pixels, row_bytes, (0, 0), CachingHint::Disallow)
            || !destination.read_pixels(&info, backdrop, row_bytes, (0, 0), CachingHint::Disallow)
        {
            return Err(internal("blend input readback failed"));
        }
        // Solid layers and flat regions repeat inputs, so each side remembers its last conversion.
        // The per-pixel helpers are forced inline; called out of line, this loop spilled its
        // pixel state around every call.
        let mut source_memo = ConversionMemo::default();
        let mut backdrop_memo = ConversionMemo::default();
        for (out, backdrop) in pixels.chunks_exact_mut(16).zip(backdrop.chunks_exact(16)) {
            let raw = read_rgba(out);
            let source = [
                raw[0] * opacity,
                raw[1] * opacity,
                raw[2] * opacity,
                raw[3] * opacity,
            ];
            let backdrop = read_rgba(backdrop);
            let source_alpha = source[3];
            let result = if source_alpha == 0.0 {
                if composite { backdrop } else { source }
            } else {
                let backdrop_alpha = backdrop[3];
                let source_srgb = source_memo.to_srgb(tables, source);
                let backdrop_srgb = if backdrop_alpha > 0.0 {
                    backdrop_memo.to_srgb(tables, backdrop)
                } else {
                    [0.0; 3]
                };
                let blended = blend_rgb(backdrop_srgb, source_srgb, reference_mode)
                    .map_err(|error| internal(&format!("creative blend: {error}")))?;
                let working = srgb_to_working(
                    tables,
                    [
                        (1.0 - backdrop_alpha) * source_srgb[0] + backdrop_alpha * blended[0],
                        (1.0 - backdrop_alpha) * source_srgb[1] + backdrop_alpha * blended[1],
                        (1.0 - backdrop_alpha) * source_srgb[2] + backdrop_alpha * blended[2],
                    ],
                );
                let keep = if composite { 1.0 - source_alpha } else { 0.0 };
                [
                    working[0] * source_alpha + backdrop[0] * keep,
                    working[1] * source_alpha + backdrop[1] * keep,
                    working[2] * source_alpha + backdrop[2] * keep,
                    source_alpha + backdrop[3] * keep,
                ]
            };
            write_rgba(out, result);
        }
        if !output
            .canvas()
            .write_pixels(&info, pixels, row_bytes, (0, 0))
        {
            return Err(internal("blend output write failed"));
        }
        Ok(true)
    })
}

thread_local! {
    /// Per-thread source/backdrop F32 buffers, reused across frames to avoid page-faulting two
    /// full-frame allocations per blend.
    static BLEND_SCRATCH: std::cell::RefCell<(Vec<u8>, Vec<u8>)> =
        const { std::cell::RefCell::new((Vec::new(), Vec::new())) };
}

/// The last premultiplied pixel converted to straight extended sRGB, keyed by exact bits.
#[derive(Default)]
struct ConversionMemo {
    key: Option<[u32; 4]>,
    value: [f32; 3],
}

impl ConversionMemo {
    #[inline(always)]
    fn to_srgb(&mut self, tables: &TransferTables, pixel: [f32; 4]) -> [f32; 3] {
        let key = [
            pixel[0].to_bits(),
            pixel[1].to_bits(),
            pixel[2].to_bits(),
            pixel[3].to_bits(),
        ];
        if self.key != Some(key) {
            let alpha = pixel[3];
            let linear = multiply(
                &WORKING_TO_LINEAR_SRGB,
                [pixel[0] / alpha, pixel[1] / alpha, pixel[2] / alpha],
            );
            self.value = [
                encode_with(tables, linear[0]),
                encode_with(tables, linear[1]),
                encode_with(tables, linear[2]),
            ];
            self.key = Some(key);
        }
        self.value
    }
}

// Spelled out rather than `[0, 1, 2, 3].map(..)`: the array closure stayed an out-of-line call
// per channel with a bounds check per byte, a tenth of full-frame blend time.
fn read_rgba(bytes: &[u8]) -> [f32; 4] {
    let bytes: &[u8; 16] = bytes.try_into().expect("RGBA F32 pixels are 16 bytes");
    let channel = |c: usize| {
        f32::from_ne_bytes([
            bytes[c * 4],
            bytes[c * 4 + 1],
            bytes[c * 4 + 2],
            bytes[c * 4 + 3],
        ])
    };
    [channel(0), channel(1), channel(2), channel(3)]
}

fn write_rgba(bytes: &mut [u8], pixel: [f32; 4]) {
    for (bytes, channel) in bytes.chunks_exact_mut(4).zip(pixel) {
        bytes.copy_from_slice(&channel.to_ne_bytes());
    }
}

// Linear Rec.2020 working space <-> linear sRGB/Rec.709, the constants of `creativeblend.sksl`.
// Products accumulate in f64 and round once, like the Engine reference: a gray input then stays
// exactly gray, which the non-separable modes need (`setSat` divides by max - min).
const WORKING_TO_LINEAR_SRGB: [[f64; 3]; 3] = [
    [
        1.6604910021084354,
        -0.5876411387885495,
        -0.07284986331988474,
    ],
    [
        -0.12455047452159074,
        1.1328998971259594,
        -0.008349422604369515,
    ],
    [
        -0.01815076335490526,
        -0.10057889800800739,
        1.118729661362913,
    ],
];
const LINEAR_SRGB_TO_WORKING: [[f64; 3]; 3] = [
    [0.627403895934699, 0.3292830383778836, 0.04331306568741723],
    [
        0.06909728935823205,
        0.9195403950754589,
        0.011362315566309159,
    ],
    [0.01639143887515025, 0.08801330787722578, 0.8955952532476238],
];

#[inline(always)]
fn multiply(matrix: &[[f64; 3]; 3], rgb: [f32; 3]) -> [f32; 3] {
    let (r, g, b) = (f64::from(rgb[0]), f64::from(rgb[1]), f64::from(rgb[2]));
    let row = |i: usize| (matrix[i][0] * r + matrix[i][1] * g + matrix[i][2] * b) as f32;
    [row(0), row(1), row(2)]
}

#[inline(always)]
fn srgb_to_working(tables: &TransferTables, rgb: [f32; 3]) -> [f32; 3] {
    multiply(
        &LINEAR_SRGB_TO_WORKING,
        [
            decode_with(tables, rgb[0]),
            decode_with(tables, rgb[1]),
            decode_with(tables, rgb[2]),
        ],
    )
}

/// The sRGB transfer tables cover [0, TRANSFER_TABLE_LIMIT], sampled at TRANSFER_TABLE_RESOLUTION
/// points per unit. Linear interpolation keeps the absolute error below 2e-6, far under one F16
/// step and the 0.004 reference tolerance. The range extends past 1 because near-white pixels
/// routinely land just above 1 after the gamut matrix or F16 unpremultiplication.
const TRANSFER_TABLE_LIMIT: f32 = 2.0;
const TRANSFER_TABLE_RESOLUTION: usize = 16_384;
const TRANSFER_TABLE_STEPS: usize = 2 * TRANSFER_TABLE_RESOLUTION;

struct TransferTables {
    encode: Vec<f32>,
    decode: Vec<f32>,
}

fn transfer_tables() -> &'static TransferTables {
    static TABLES: std::sync::OnceLock<TransferTables> = std::sync::OnceLock::new();
    TABLES.get_or_init(|| {
        let sample = |f: fn(f64) -> f64| {
            (0..=TRANSFER_TABLE_STEPS)
                .map(|i| f(i as f64 / TRANSFER_TABLE_RESOLUTION as f64) as f32)
                .collect()
        };
        TransferTables {
            encode: sample(srgb_encode_exact),
            decode: sample(srgb_decode_exact),
        }
    })
}

fn srgb_encode_exact(magnitude: f64) -> f64 {
    if magnitude <= 0.003_130_8 {
        magnitude * 12.92
    } else {
        1.055 * valle_draw::math::pow(magnitude, 1.0 / 2.4) - 0.055
    }
}

fn srgb_decode_exact(magnitude: f64) -> f64 {
    if magnitude <= 0.040_45 {
        magnitude / 12.92
    } else {
        valle_draw::math::pow((magnitude + 0.055) / 1.055, 2.4)
    }
}

#[inline(always)]
fn lookup(table: &[f32], magnitude: f32) -> f32 {
    let position = magnitude * TRANSFER_TABLE_RESOLUTION as f32;
    let index = (position as usize).min(TRANSFER_TABLE_STEPS - 1);
    table[index] + (table[index + 1] - table[index]) * (position - index as f32)
}

#[cfg(test)]
fn srgb_encode(value: f32) -> f32 {
    encode_with(transfer_tables(), value)
}

#[cfg(test)]
fn srgb_decode(value: f32) -> f32 {
    decode_with(transfer_tables(), value)
}

/// Extended sRGB encode: sign-symmetric, exact beyond the table range.
#[inline(always)]
fn encode_with(tables: &TransferTables, value: f32) -> f32 {
    let magnitude = value.abs();
    let encoded = if magnitude <= 0.003_130_8 {
        magnitude * 12.92
    } else if magnitude <= TRANSFER_TABLE_LIMIT {
        lookup(&tables.encode, magnitude)
    } else {
        srgb_encode_exact(f64::from(magnitude)) as f32
    };
    encoded.copysign(value)
}

#[inline(always)]
fn decode_with(tables: &TransferTables, value: f32) -> f32 {
    let magnitude = value.abs();
    let decoded = if magnitude <= 0.040_45 {
        magnitude / 12.92
    } else if magnitude <= TRANSFER_TABLE_LIMIT {
        lookup(&tables.decode, magnitude)
    } else {
        srgb_decode_exact(f64::from(magnitude)) as f32
    };
    decoded.copysign(value)
}

fn internal(message: &str) -> DrawError {
    DrawError::Internal(message.to_owned())
}

const fn draw_code(mode: DrawBlend) -> f32 {
    match mode {
        DrawBlend::Normal => 0.0,
        DrawBlend::Multiply => 1.0,
        DrawBlend::Screen => 2.0,
        DrawBlend::Overlay => 3.0,
        DrawBlend::Darken => 4.0,
        DrawBlend::Lighten => 5.0,
        DrawBlend::ColorDodge => 6.0,
        DrawBlend::ColorBurn => 7.0,
        DrawBlend::LinearBurn => 8.0,
        DrawBlend::HardLight => 9.0,
        DrawBlend::SoftLight => 10.0,
        DrawBlend::Difference => 11.0,
        DrawBlend::Exclusion => 12.0,
        DrawBlend::Hue => 13.0,
        DrawBlend::Saturation => 14.0,
        DrawBlend::Color => 15.0,
        DrawBlend::Luminosity => 16.0,
    }
}

#[cfg(test)]
mod tests {
    use skia_safe::{
        AlphaType, BlendMode, Color4f, ColorType, ImageInfo, Paint, image::CachingHint,
    };
    use valle_engine::{
        compositor::reference::{PremulRgba32, ReferenceBlendMode, blend_over},
        resource::Extent2d,
    };

    use super::*;
    use crate::executor::skia::surface::{raster_surface, working_color_space, working_info};

    #[test]
    fn admitted_kernel_matches_reference_linear_burn_effective_composite() {
        let extent = Extent2d::new(1, 1).unwrap();
        let info = working_info(extent).unwrap();
        let source_value = PremulRgba32::from_premultiplied([0.35, 0.12, 0.08, 0.7]).unwrap();
        let destination_value = PremulRgba32::from_premultiplied([0.18, 0.32, 0.09, 0.8]).unwrap();
        let source = solid(source_value, &info);
        let destination = solid(destination_value, &info);
        let opacity = 0.65;
        let mut output = raster_surface(&info).unwrap();
        BlendRuntime::admit()
            .unwrap()
            .composite_into(
                &mut output,
                &source,
                &destination,
                DrawBlend::LinearBurn,
                opacity,
            )
            .unwrap();
        let actual = output.image_snapshot();
        let expected = blend_over(
            source_value.scale_coverage(opacity).unwrap(),
            destination_value,
            ReferenceBlendMode::LinearBurn,
        )
        .unwrap();
        let actual = read(&actual);
        assert!(
            actual.approx_eq(expected, 0.004),
            "actual={actual:?} expected={expected:?}"
        );
    }

    #[test]
    fn raster_path_matches_kernel_and_reference_for_every_mode() {
        let modes = [
            DrawBlend::Multiply,
            DrawBlend::Screen,
            DrawBlend::Overlay,
            DrawBlend::Darken,
            DrawBlend::Lighten,
            DrawBlend::ColorDodge,
            DrawBlend::ColorBurn,
            DrawBlend::LinearBurn,
            DrawBlend::HardLight,
            DrawBlend::SoftLight,
            DrawBlend::Difference,
            DrawBlend::Exclusion,
            DrawBlend::Hue,
            DrawBlend::Saturation,
            DrawBlend::Color,
            DrawBlend::Luminosity,
        ];
        // Premultiplied working-space pairs: opaque, partial, transparent source/backdrop,
        // dark values near the linear transfer segment and extended (>1) components.
        let pairs = [
            ([0.35, 0.12, 0.08, 0.7], [0.18, 0.32, 0.09, 0.8]),
            ([0.9, 0.4, 0.1, 1.0], [0.05, 0.6, 0.95, 1.0]),
            ([0.001, 0.002, 0.0005, 0.5], [0.7, 0.7, 0.7, 1.0]),
            ([0.2, 0.3, 0.4, 0.4], [0.0, 0.0, 0.0, 0.0]),
            ([0.0, 0.0, 0.0, 0.0], [0.3, 0.2, 0.1, 0.6]),
            ([1.3, 0.2, 0.05, 1.0], [0.4, 1.1, 0.2, 1.0]),
        ];
        let extent = Extent2d::new(1, 1).unwrap();
        let info = working_info(extent).unwrap();
        let runtime = BlendRuntime::admit().unwrap();
        for mode in modes {
            for (source_value, destination_value) in pairs {
                for (opacity, composite) in [(1.0, true), (0.65, true), (1.0, false)] {
                    let source_value = PremulRgba32::from_premultiplied(source_value).unwrap();
                    let destination_value =
                        PremulRgba32::from_premultiplied(destination_value).unwrap();
                    let source = solid(source_value, &info);
                    let destination = solid(destination_value, &info);
                    let mut native = raster_surface(&info).unwrap();
                    assert!(
                        raster_blend_into(
                            &mut native,
                            &source,
                            &destination,
                            mode,
                            opacity,
                            composite,
                        )
                        .unwrap()
                    );
                    let mut kernel = raster_surface(&info).unwrap();
                    render_runtime_into(
                        &mut kernel,
                        &runtime.effect,
                        &[&source, &destination],
                        &[draw_code(mode), opacity, if composite { 1.0 } else { 0.0 }],
                    )
                    .unwrap();
                    let native = read(&native.image_snapshot());
                    let kernel = read(&kernel.image_snapshot());
                    let scaled = source_value.scale_coverage(opacity).unwrap();
                    let expected = if composite {
                        blend_over(scaled, destination_value, mode.into()).unwrap()
                    } else {
                        valle_engine::compositor::reference::effective_blend_source(
                            scaled,
                            destination_value,
                            mode.into(),
                        )
                        .unwrap()
                    };
                    let context = format!(
                        "{mode:?} opacity={opacity} composite={composite} \
                         native={native:?} kernel={kernel:?} reference={expected:?}"
                    );
                    // F16 storage quantizes extended (>1) values more coarsely, so scale the
                    // tolerance with magnitude; inside [0, 1] this is the usual 0.004.
                    let close = |a: PremulRgba32, b: PremulRgba32, tolerance: f32| {
                        a.channels()
                            .iter()
                            .zip(b.channels())
                            .all(|(a, b)| (a - b).abs() <= tolerance * b.abs().max(1.0))
                    };
                    // The reference is the contract. The kernel is a second approximation of
                    // it, so the two approximations may differ by up to both error budgets.
                    assert!(close(native, expected, 0.004), "{context}");
                    assert!(close(kernel, expected, 0.004), "{context}");
                    assert!(close(native, kernel, 0.008), "{context}");
                }
            }
        }
    }

    #[test]
    fn repeated_inputs_reuse_conversions_without_changing_results() {
        // A constant source (solid layer) over a backdrop that alternates and repeats exercises
        // both conversion memos; every pixel must still equal the reference.
        let first = [0.2, 0.45, 0.6, 1.0];
        let second = [0.7, 0.1, 0.05, 0.5];
        let backdrop = [first, first, second, second, first, second, second, first];
        let source = [0.3, 0.2, 0.1, 0.8];
        let image = |pixels: &[[f32; 4]]| {
            let info = ImageInfo::new(
                (pixels.len() as i32, 1),
                ColorType::RGBAF32,
                AlphaType::Premul,
                Some(working_color_space().unwrap()),
            );
            let bytes = pixels
                .iter()
                .flatten()
                .flat_map(|value| value.to_ne_bytes())
                .collect::<Vec<_>>();
            skia_safe::images::raster_from_data(
                &info,
                skia_safe::Data::new_copy(&bytes),
                pixels.len() * 16,
            )
            .unwrap()
        };
        let source_image = image(&[source; 8]);
        let backdrop_image = image(&backdrop);
        for mode in [DrawBlend::Multiply, DrawBlend::Overlay, DrawBlend::Hue] {
            let mut output =
                raster_surface(&working_info(Extent2d::new(8, 1).unwrap()).unwrap()).unwrap();
            assert!(
                raster_blend_into(&mut output, &source_image, &backdrop_image, mode, 0.9, true)
                    .unwrap()
            );
            let info = ImageInfo::new(
                (8, 1),
                ColorType::RGBAF32,
                AlphaType::Premul,
                Some(working_color_space().unwrap()),
            );
            let mut channels = [0.0_f32; 32];
            assert!(output.image_snapshot().read_pixels(
                &info,
                &mut channels,
                8 * 16,
                (0, 0),
                CachingHint::Disallow,
            ));
            for (index, backdrop) in backdrop.iter().enumerate() {
                let expected = blend_over(
                    PremulRgba32::from_premultiplied(source)
                        .unwrap()
                        .scale_coverage(0.9)
                        .unwrap(),
                    PremulRgba32::from_premultiplied(*backdrop).unwrap(),
                    mode.into(),
                )
                .unwrap();
                let actual = PremulRgba32::from_premultiplied(
                    channels[index * 4..index * 4 + 4].try_into().unwrap(),
                )
                .unwrap();
                assert!(actual.approx_eq(expected, 0.004), "{mode:?} pixel {index}");
            }
        }
    }

    #[test]
    fn transfer_tables_track_the_exact_curves() {
        for i in 0..=60_000 {
            let value = i as f32 / 10_000.0 - 3.0;
            let exact_encode = (srgb_encode_exact(f64::from(value.abs())) as f32).copysign(value);
            let exact_decode = (srgb_decode_exact(f64::from(value.abs())) as f32).copysign(value);
            assert!(
                (srgb_encode(value) - exact_encode).abs() < 2e-5,
                "encode {value}"
            );
            assert!(
                (srgb_decode(value) - exact_decode).abs() < 2e-5,
                "decode {value}"
            );
        }
    }

    fn solid(value: PremulRgba32, info: &ImageInfo) -> Image {
        let mut surface = raster_surface(info).unwrap();
        let channels = value.channels();
        let alpha = channels[3];
        let straight = if alpha == 0.0 {
            Color4f::new(0.0, 0.0, 0.0, 0.0)
        } else {
            Color4f::new(
                channels[0] / alpha,
                channels[1] / alpha,
                channels[2] / alpha,
                alpha,
            )
        };
        let mut paint = Paint::default();
        paint.set_blend_mode(BlendMode::Src);
        let space = working_color_space().unwrap();
        paint.set_color4f(straight, &space);
        surface.canvas().draw_paint(&paint);
        surface.image_snapshot()
    }

    fn read(image: &Image) -> PremulRgba32 {
        let info = ImageInfo::new(
            (1, 1),
            ColorType::RGBAF32,
            AlphaType::Premul,
            Some(working_color_space().unwrap()),
        );
        let mut channels = [0.0_f32; 4];
        assert!(image.read_pixels(&info, &mut channels, 16, (0, 0), CachingHint::Disallow,));
        PremulRgba32::from_premultiplied(channels).unwrap()
    }
}
