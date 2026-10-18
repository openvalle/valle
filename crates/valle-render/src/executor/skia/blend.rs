use skia_safe::{
    AlphaType, ColorType, IRect, Image, ImageInfo, RuntimeEffect, Surface, image::CachingHint,
};
use valle_draw::program::{BlendMode as DrawBlend, BlendSpace};
use valle_engine::compositor::{
    bloom::fill_image_rows,
    reference::{ReferenceBlendMode, blend_rgb},
};

use super::{draw::DrawError, effect::render_runtime_into, surface::working_color_space};

const BLEND_SOURCE: &str = include_str!("../../../../valle-draw/assets/shaders/creativeblend.sksl");

#[derive(Debug)]
pub(crate) struct BlendRuntime {
    effect: RuntimeEffect,
}

/// A blend operand: the top-left `rect.size()` pixels of `image`, placed at `rect` in output
/// coordinates.
pub(crate) struct Placed<'a> {
    pub(crate) image: &'a Image,
    pub(crate) rect: IRect,
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
        space: BlendSpace,
    ) -> Result<(), DrawError> {
        if raster_blend_into(output, source, destination, mode, space, 1.0, false)? {
            return Ok(());
        }
        render_runtime_into(
            output,
            &self.effect,
            &[source, destination],
            &[draw_code(mode), 1.0, 0.0, space_code(space)],
        )
    }

    /// `effective_source_into` with operands that would first be drawn under the Src blend into
    /// transparent images the size of `output`. On Raster their half floats are read in place
    /// instead, where the drawn images would hold the same values. Returns `false` where only
    /// the drawn images can take the blend.
    pub(crate) fn effective_placed_into(
        &self,
        output: &mut Surface,
        source: Placed<'_>,
        destination: Placed<'_>,
        mode: DrawBlend,
        space: BlendSpace,
    ) -> Result<bool, DrawError> {
        let target = output.image_info();
        let stored = |operand: &Placed<'_>| {
            let image = operand.image.image_info();
            image.color_type() == ColorType::RGBAF16
                && image.alpha_type() == AlphaType::Premul
                && image.color_space() == target.color_space()
                && operand.rect.width() <= image.width()
                && operand.rect.height() <= image.height()
        };
        if target.color_type() != ColorType::RGBAF16
            || target.alpha_type() != AlphaType::Premul
            || !stored(&source)
            || !stored(&destination)
        {
            return Ok(false);
        }
        raster_blend_placed_into(output, source, destination, mode, space, 1.0, false)
    }

    pub(crate) fn composite_into(
        &self,
        output: &mut Surface,
        source: &Image,
        destination: &Image,
        mode: DrawBlend,
        space: BlendSpace,
        opacity: f32,
    ) -> Result<(), DrawError> {
        if raster_blend_into(output, source, destination, mode, space, opacity, true)? {
            return Ok(());
        }
        render_runtime_into(
            output,
            &self.effect,
            &[source, destination],
            &[draw_code(mode), opacity, 1.0, space_code(space)],
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
    space: BlendSpace,
    opacity: f32,
    composite: bool,
) -> Result<bool, DrawError> {
    let extent = output.image_info().dimensions();
    if source.dimensions() != extent || destination.dimensions() != extent {
        return Ok(false);
    }
    let rect = IRect::from_size(extent);
    raster_blend_placed_into(
        output,
        Placed {
            image: source,
            rect,
        },
        Placed {
            image: destination,
            rect,
        },
        mode,
        space,
        opacity,
        composite,
    )
}

fn raster_blend_placed_into(
    output: &mut Surface,
    source: Placed<'_>,
    destination: Placed<'_>,
    mode: DrawBlend,
    space: BlendSpace,
    opacity: f32,
    composite: bool,
) -> Result<bool, DrawError> {
    if output.peek_pixels().is_none()
        || source.image.is_texture_backed()
        || destination.image.is_texture_backed()
        || !(0.0..=1.0).contains(&opacity)
    {
        return Ok(false);
    }
    let info = ImageInfo::new(
        output.image_info().dimensions(),
        ColorType::RGBAF32,
        AlphaType::Premul,
        Some(working_color_space().map_err(|error| internal(&error.to_string()))?),
    );
    let blend = PixelBlend {
        mode,
        reference: ReferenceBlendMode::from(mode),
        space,
        opacity,
        composite,
    };
    let tables = transfer_tables();
    #[cfg(all(target_arch = "aarch64", target_endian = "little"))]
    if fused::blend_halves(output, &source, &destination, blend, tables, &info)? {
        return Ok(true);
    }
    blend_floats(output, &source, &destination, blend, tables, &info)?;
    Ok(true)
}

/// The blend of `source` over `destination` into `output`, through copies of the operands in
/// the F32 pixels of `info`.
fn blend_floats(
    output: &mut Surface,
    source: &Placed<'_>,
    destination: &Placed<'_>,
    blend: PixelBlend,
    tables: &TransferTables,
    info: &ImageInfo,
) -> Result<(), DrawError> {
    let width = usize::try_from(info.width()).map_err(|_| internal("negative blend width"))?;
    let height = usize::try_from(info.height()).map_err(|_| internal("negative blend height"))?;
    let row_bytes = width
        .checked_mul(16)
        .ok_or_else(|| internal("blend row is too large"))?;
    let byte_count = row_bytes
        .checked_mul(height)
        .ok_or_else(|| internal("blend image is too large"))?;
    BLEND_SCRATCH.with_borrow_mut(|(pixels, backdrop)| {
        pixels.resize(byte_count, 0);
        backdrop.resize(byte_count, 0);
        if !read_placed(source, info, pixels, row_bytes)
            || !read_placed(destination, info, backdrop, row_bytes)
        {
            return Err(internal("blend input readback failed"));
        }
        // Rows are independent; the earliest failing pixel of all row spans is reported.
        let failure = std::sync::Mutex::new(None::<(usize, String)>);
        let backdrop = &*backdrop;
        fill_image_rows(pixels.as_mut_slice(), row_bytes, |first, rows| {
            let backdrop = &backdrop[first * row_bytes..][..rows.len()];
            if let Err((index, message)) = blend_rows(tables, blend, rows, backdrop) {
                let index = first * width + index;
                let mut failure = failure.lock().unwrap_or_else(|e| e.into_inner());
                if failure.as_ref().is_none_or(|(at, _)| index < *at) {
                    *failure = Some((index, message));
                }
            }
        });
        if let Some((_, message)) = failure.into_inner().unwrap_or_else(|e| e.into_inner()) {
            return Err(internal(&message));
        }
        if !output
            .canvas()
            .write_pixels(info, pixels, row_bytes, (0, 0))
        {
            return Err(internal("blend output write failed"));
        }
        Ok(())
    })
}

/// What a creative blend does to every pixel.
#[derive(Clone, Copy)]
struct PixelBlend {
    mode: DrawBlend,
    reference: ReferenceBlendMode,
    space: BlendSpace,
    opacity: f32,
    composite: bool,
}

/// Blends the F32 RGBA source pixels in `out` with the `backdrop` pixels of the same span, in
/// place. `Err` holds the index of the first pixel that fails and the reason.
#[cfg(not(all(target_arch = "aarch64", target_endian = "little")))]
fn blend_rows(
    tables: &TransferTables,
    blend: PixelBlend,
    out: &mut [u8],
    backdrop: &[u8],
) -> Result<(), (usize, String)> {
    blend_pixels(tables, blend, out, backdrop)
}

#[cfg(all(target_arch = "aarch64", target_endian = "little"))]
use fused::blend_rows;

/// [`blend_rows`] one pixel at a time. Solid layers and flat regions repeat inputs, so each side
/// remembers its last conversion. The per-pixel helpers are forced inline; called out of line,
/// this loop spilled its pixel state around every call.
#[inline(always)]
fn blend_pixels(
    tables: &TransferTables,
    blend: PixelBlend,
    out: &mut [u8],
    backdrop: &[u8],
) -> Result<(), (usize, String)> {
    let PixelBlend {
        mode,
        reference,
        space,
        opacity,
        composite,
    } = blend;
    let mut source_memo = ConversionMemo::default();
    let mut backdrop_memo = ConversionMemo::default();
    for (index, (out, backdrop)) in out
        .chunks_exact_mut(16)
        .zip(backdrop.chunks_exact(16))
        .enumerate()
    {
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
        } else if mode == DrawBlend::Plus {
            let alpha = if backdrop[3] == 1.0 {
                source_alpha
            } else {
                (source_alpha / (1.0 - backdrop[3])).min(1.0)
            };
            let keep = if composite { 0.0 } else { 1.0 - alpha };
            [
                (source[0] + backdrop[0]).clamp(0.0, 1.0) - backdrop[0] * keep,
                (source[1] + backdrop[1]).clamp(0.0, 1.0) - backdrop[1] * keep,
                (source[2] + backdrop[2]).clamp(0.0, 1.0) - backdrop[2] * keep,
                if composite {
                    (source_alpha + backdrop[3]).min(1.0)
                } else {
                    alpha
                },
            ]
        } else {
            let backdrop_alpha = backdrop[3];
            let source_rgb = source_memo.to_blend(tables, source, space);
            let backdrop_rgb = if backdrop_alpha > 0.0 {
                backdrop_memo.to_blend(tables, backdrop, space)
            } else {
                [0.0; 3]
            };
            let blended = blend_rgb(backdrop_rgb, source_rgb, reference)
                .map_err(|error| (index, format!("creative blend: {error}")))?;
            let working = blend_to_working(
                tables,
                space,
                [
                    (1.0 - backdrop_alpha) * source_rgb[0] + backdrop_alpha * blended[0],
                    (1.0 - backdrop_alpha) * source_rgb[1] + backdrop_alpha * blended[1],
                    (1.0 - backdrop_alpha) * source_rgb[2] + backdrop_alpha * blended[2],
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
    Ok(())
}

/// Reads `operand` into F32 `pixels` the size of `info`, transparent outside its rect.
fn read_placed(
    operand: &Placed<'_>,
    info: &ImageInfo,
    pixels: &mut [u8],
    row_bytes: usize,
) -> bool {
    let extent = IRect::from_size(info.dimensions());
    if operand.rect == extent {
        // The read overwrites every byte, so capacity reused from a previous frame is never read.
        return operand
            .image
            .read_pixels(info, pixels, row_bytes, (0, 0), CachingHint::Disallow);
    }
    pixels.fill(0);
    let Some(visible) = IRect::intersect(&operand.rect, &extent) else {
        return true;
    };
    let start = visible.top as usize * row_bytes + visible.left as usize * 16;
    operand.image.read_pixels(
        &info.with_dimensions(visible.size()),
        &mut pixels[start..],
        row_bytes,
        (
            visible.left - operand.rect.left,
            visible.top - operand.rect.top,
        ),
        CachingHint::Disallow,
    )
}

thread_local! {
    /// Per-thread source/backdrop F32 buffers, reused across frames to avoid page-faulting two
    /// full-frame allocations per blend.
    static BLEND_SCRATCH: std::cell::RefCell<(Vec<u8>, Vec<u8>)> =
        const { std::cell::RefCell::new((Vec::new(), Vec::new())) };
}

/// One pass has a fixed space; remember its last straight RGB conversion by exact bits.
#[derive(Default)]
struct ConversionMemo {
    key: Option<[u32; 4]>,
    value: [f32; 3],
}

impl ConversionMemo {
    #[inline(always)]
    fn to_blend(
        &mut self,
        tables: &TransferTables,
        pixel: [f32; 4],
        space: BlendSpace,
    ) -> [f32; 3] {
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
            self.value = if space == BlendSpace::Linear {
                linear
            } else {
                [
                    encode_with(tables, linear[0]),
                    encode_with(tables, linear[1]),
                    encode_with(tables, linear[2]),
                ]
            };
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
fn blend_to_working(tables: &TransferTables, space: BlendSpace, rgb: [f32; 3]) -> [f32; 3] {
    multiply(
        &LINEAR_SRGB_TO_WORKING,
        if space == BlendSpace::Linear {
            rgb
        } else {
            [
                decode_with(tables, rgb[0]),
                decode_with(tables, rgb[1]),
                decode_with(tables, rgb[2]),
            ]
        },
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

/// On aarch64, Multiply and Screen blend four pixels at a time. Every lane takes the operations
/// of [`blend_pixels`] in their order, without fused multiply-adds, so the bytes do not change.
#[cfg(all(target_arch = "aarch64", target_endian = "little"))]
mod fused {
    use std::{arch::aarch64::*, sync::Mutex};

    use skia_safe::{
        AlphaType, ColorType, IRect, ImageInfo, Pixmap, Surface, surface::ContentChangeMode,
    };
    use valle_draw::program::{BlendMode as DrawBlend, BlendSpace};
    use valle_engine::compositor::bloom::fill_image_rows;

    use super::{
        DrawError, LINEAR_SRGB_TO_WORKING, PixelBlend, Placed, TRANSFER_TABLE_LIMIT,
        TRANSFER_TABLE_RESOLUTION, TRANSFER_TABLE_STEPS, TransferTables, WORKING_TO_LINEAR_SRGB,
        blend_pixels, internal,
    };

    /// Pixels per vector.
    const LANES: usize = 4;
    /// Bytes of four F32 RGBA pixels.
    const GROUP: usize = LANES * 16;
    /// Bytes of four F16 RGBA pixels.
    const HALF_GROUP: usize = LANES * 8;

    /// Whether the mode of `blend` takes four pixels at a time.
    fn fused(blend: PixelBlend) -> bool {
        matches!(blend.mode, DrawBlend::Multiply | DrawBlend::Screen)
    }

    /// [`blend_pixels`] four pixels at a time, where the mode allows it.
    pub(super) fn blend_rows(
        tables: &TransferTables,
        blend: PixelBlend,
        out: &mut [u8],
        backdrop: &[u8],
    ) -> Result<(), (usize, String)> {
        if !fused(blend)
            || !out.as_ptr().cast::<f32>().is_aligned()
            || !backdrop.as_ptr().cast::<f32>().is_aligned()
        {
            return blend_pixels(tables, blend, out, backdrop);
        }
        let mut memos = Memos::default();
        let (groups, _) = out.as_chunks_mut::<GROUP>();
        let count = groups.len();
        for (index, (out, backdrop)) in groups
            .iter_mut()
            .zip(backdrop.as_chunks::<GROUP>().0)
            .enumerate()
        {
            // SAFETY: the groups hold four pixels of four floats, aligned for floats.
            let pixels = unsafe { (load(out), load(backdrop)) };
            match blend_four(tables, blend, pixels.0, pixels.1, &mut memos) {
                // SAFETY: as above.
                Some(result) => unsafe { store(out, result) },
                None => one_by_one(tables, blend, out, backdrop)
                    .map_err(|(pixel, message)| (index * LANES + pixel, message))?,
            }
        }
        let tail = count * GROUP;
        one_by_one(tables, blend, &mut out[tail..], &backdrop[tail..])
            .map_err(|(pixel, message)| (count * LANES + pixel, message))
    }

    /// The blend of every pixel of `output`, reading the half floats of the operands and writing
    /// those of `output` in place, with the conversions that read them into the F32 pixels of
    /// `info` and write them back. Takes raster operands whose rects cover `output` and whose
    /// images, like `output`, hold F16 premultiplied pixels in the color space of `info`. Returns
    /// `false` where the blend goes through F32 copies.
    pub(super) fn blend_halves(
        output: &mut Surface,
        source: &Placed<'_>,
        destination: &Placed<'_>,
        blend: PixelBlend,
        tables: &TransferTables,
        info: &ImageInfo,
    ) -> Result<bool, DrawError> {
        if !fused(blend) || !half(&output.image_info(), info) {
            return Ok(false);
        }
        let (Some(source_pixels), Some(backdrop_pixels)) =
            (source.image.peek_pixels(), destination.image.peek_pixels())
        else {
            return Ok(false);
        };
        let (Some(source_rows), Some(backdrop_rows)) = (
            Rows::new(&source_pixels, source.rect, info),
            Rows::new(&backdrop_pixels, destination.rect, info),
        ) else {
            return Ok(false);
        };
        let width = info.width() as usize;
        // As for a write of every pixel: pixels a snapshot still shares are replaced, not copied.
        output.notify_content_will_change(ContentChangeMode::Discard);
        let mut pixels = output
            .peek_pixels()
            .ok_or_else(|| internal("blend output has no pixels"))?;
        let row_bytes = pixels.row_bytes();
        let bytes = pixels
            .bytes_mut()
            .ok_or_else(|| internal("blend output pixels are not writable"))?;
        // The F32 copies write every pixel again.
        if !bytes.as_ptr().cast::<u16>().is_aligned() || !row_bytes.is_multiple_of(8) {
            return Ok(false);
        }
        // Rows are independent; the earliest failing pixel of all row spans is reported.
        let failure = Mutex::new(None::<(usize, String)>);
        fill_image_rows(bytes, row_bytes, |first, rows| {
            let mut memos = Memos::default();
            for (index, row) in rows.chunks_mut(row_bytes).enumerate() {
                let y = first + index;
                let source = source_rows.row(y, width);
                let backdrop = backdrop_rows.row(y, width);
                if let Err((x, message)) = half_row(
                    tables,
                    blend,
                    &mut row[..width * 8],
                    source,
                    backdrop,
                    &mut memos,
                ) {
                    let index = y * width + x;
                    let mut failure = failure.lock().unwrap_or_else(|e| e.into_inner());
                    if failure.as_ref().is_none_or(|(at, _)| index < *at) {
                        *failure = Some((index, message));
                    }
                    return;
                }
            }
        });
        if let Some((_, message)) = failure.into_inner().unwrap_or_else(|e| e.into_inner()) {
            return Err(internal(&message));
        }
        Ok(true)
    }

    /// Whether `stored` holds F16 premultiplied pixels in the color space of `info`.
    fn half(stored: &ImageInfo, info: &ImageInfo) -> bool {
        stored.color_type() == ColorType::RGBAF16
            && stored.alpha_type() == AlphaType::Premul
            && stored.color_space() == info.color_space()
    }

    /// The half floats of an operand at the output's pixels, row by row.
    struct Rows<'a> {
        /// From the operand's pixel at the output's top-left corner.
        bytes: &'a [u8],
        row_bytes: usize,
    }

    impl<'a> Rows<'a> {
        /// The rows of an operand placed at `rect`, where its rect covers the output of `info`
        /// and `pixels` hold its pixels as `half` takes them.
        fn new(pixels: &Pixmap<'a>, rect: IRect, info: &ImageInfo) -> Option<Self> {
            let stored = pixels.info();
            let row_bytes = pixels.row_bytes();
            let bytes = pixels.bytes()?;
            let covers = rect.left <= 0
                && rect.top <= 0
                && rect.right >= info.width()
                && rect.bottom >= info.height()
                && rect.width() <= stored.width()
                && rect.height() <= stored.height();
            // The reads load the halves of four pixels together.
            (covers
                && half(stored, info)
                && bytes.as_ptr().cast::<u16>().is_aligned()
                && row_bytes.is_multiple_of(8))
            .then(|| Self {
                bytes: &bytes[rect.top.unsigned_abs() as usize * row_bytes
                    + rect.left.unsigned_abs() as usize * 8..],
                row_bytes,
            })
        }

        /// The `width` pixels of output row `y`.
        fn row(&self, y: usize, width: usize) -> &'a [u8] {
            &self.bytes[y * self.row_bytes..][..width * 8]
        }
    }

    /// One row of [`blend_halves`]: the halves of `source` over those of `backdrop` into `out`.
    fn half_row(
        tables: &TransferTables,
        blend: PixelBlend,
        out: &mut [u8],
        source: &[u8],
        backdrop: &[u8],
        memos: &mut Memos,
    ) -> Result<(), (usize, String)> {
        let (groups, tail) = out.as_chunks_mut::<HALF_GROUP>();
        let (sources, source_tail) = source.as_chunks::<HALF_GROUP>();
        let (backdrops, backdrop_tail) = backdrop.as_chunks::<HALF_GROUP>();
        let count = groups.len();
        for (index, ((out, source), backdrop)) in
            groups.iter_mut().zip(sources).zip(backdrops).enumerate()
        {
            // SAFETY: the groups hold four pixels of four halves, aligned for the halves by the
            // alignment of the rows.
            unsafe {
                half_group(
                    tables,
                    blend,
                    out.as_mut_ptr().cast(),
                    source.as_ptr().cast(),
                    backdrop.as_ptr().cast(),
                    memos,
                )
            }
            .map_err(|(pixel, message)| (index * LANES + pixel, message))?;
        }
        if tail.is_empty() {
            return Ok(());
        }
        // The last pixels go with transparent ones, which the blend copies.
        let padded = |bytes: &[u8]| {
            let mut halves = [0_u16; LANES * 4];
            for (half, bytes) in halves.iter_mut().zip(bytes.as_chunks::<2>().0) {
                *half = u16::from_le_bytes(*bytes);
            }
            halves
        };
        let (source, backdrop) = (padded(source_tail), padded(backdrop_tail));
        let mut halves = [0_u16; LANES * 4];
        // SAFETY: the buffers hold four pixels of four halves.
        unsafe {
            half_group(
                tables,
                blend,
                halves.as_mut_ptr(),
                source.as_ptr(),
                backdrop.as_ptr(),
                memos,
            )
        }
        .map_err(|(pixel, message)| (count * LANES + pixel, message))?;
        for (bytes, half) in tail.as_chunks_mut::<2>().0.iter_mut().zip(&halves) {
            *bytes = half.to_le_bytes();
        }
        Ok(())
    }

    /// Blends the four pixels of halves at `source` over the four at `backdrop` into `out`.
    ///
    /// # Safety
    ///
    /// Each pointer points to sixteen halves.
    #[inline(always)]
    unsafe fn half_group(
        tables: &TransferTables,
        blend: PixelBlend,
        out: *mut u16,
        source: *const u16,
        backdrop: *const u16,
        memos: &mut Memos,
    ) -> Result<(), (usize, String)> {
        // SAFETY: NEON is part of the aarch64 baseline; the caller provides the halves.
        unsafe {
            let widen = |value: uint16x4_t| vcvt_f32_f16(vreinterpret_f16_u16(value));
            let pixels = |halves: uint16x4x4_t| {
                [
                    widen(halves.0),
                    widen(halves.1),
                    widen(halves.2),
                    widen(halves.3),
                ]
            };
            let raw = pixels(vld4_u16(source));
            let under = pixels(vld4_u16(backdrop));
            let result = match blend_four(tables, blend, raw, under, memos) {
                Some(result) => result,
                None => {
                    let (mut floats, mut backdrop) = (Floats::default(), Floats::default());
                    store(&mut floats.0, raw);
                    store(&mut backdrop.0, under);
                    one_by_one(tables, blend, &mut floats.0, &backdrop.0)?;
                    load(&floats.0)
                }
            };
            let narrow = |value: float32x4_t| vreinterpret_u16_f16(vcvt_f16_f32(value));
            vst4_u16(
                out,
                uint16x4x4_t(
                    narrow(result[0]),
                    narrow(result[1]),
                    narrow(result[2]),
                    narrow(result[3]),
                ),
            );
            Ok(())
        }
    }

    /// Four F32 RGBA pixels, aligned for floats.
    #[repr(align(16))]
    struct Floats([u8; GROUP]);

    impl Default for Floats {
        fn default() -> Self {
            Self([0; GROUP])
        }
    }

    /// The four F32 pixels at `bytes`, by channel.
    ///
    /// # Safety
    ///
    /// `bytes` is aligned for floats.
    #[inline(always)]
    unsafe fn load(bytes: &[u8; GROUP]) -> [float32x4_t; 4] {
        // SAFETY: NEON is part of the aarch64 baseline; the caller aligns the floats.
        unsafe {
            let pixels = vld4q_f32(bytes.as_ptr().cast());
            [pixels.0, pixels.1, pixels.2, pixels.3]
        }
    }

    /// Stores four F32 pixels, by channel, at `bytes`.
    ///
    /// # Safety
    ///
    /// `bytes` is aligned for floats.
    #[inline(always)]
    unsafe fn store(bytes: &mut [u8; GROUP], pixels: [float32x4_t; 4]) {
        // SAFETY: NEON is part of the aarch64 baseline; the caller aligns the floats.
        unsafe {
            vst4q_f32(
                bytes.as_mut_ptr().cast(),
                float32x4x4_t(pixels[0], pixels[1], pixels[2], pixels[3]),
            );
        }
    }

    /// [`blend_pixels`] out of line, for the few pixels the groups leave.
    #[cold]
    #[inline(never)]
    fn one_by_one(
        tables: &TransferTables,
        blend: PixelBlend,
        out: &mut [u8],
        backdrop: &[u8],
    ) -> Result<(), (usize, String)> {
        blend_pixels(tables, blend, out, backdrop)
    }

    /// The blend of four pixels, by channel, from `raw` source pixels before the opacity and
    /// `backdrop` pixels. `None` where a pixel the blend computes needs the one-pixel path: a
    /// value that is not finite, which the blend reports, or a magnitude beyond the transfer
    /// tables.
    #[inline(always)]
    fn blend_four(
        tables: &TransferTables,
        blend: PixelBlend,
        raw: [float32x4_t; 4],
        backdrop: [float32x4_t; 4],
        memos: &mut Memos,
    ) -> Option<[float32x4_t; 4]> {
        // SAFETY: NEON is part of the aarch64 baseline.
        unsafe {
            let opacity = vdupq_n_f32(blend.opacity);
            let source = [
                vmulq_f32(raw[0], opacity),
                vmulq_f32(raw[1], opacity),
                vmulq_f32(raw[2], opacity),
                vmulq_f32(raw[3], opacity),
            ];
            let zero = vdupq_n_f32(0.0);
            let one = vdupq_n_f32(1.0);
            let source_alpha = source[3];
            let backdrop_alpha = backdrop[3];
            // A transparent source is copied, or the backdrop when compositing.
            let copied = if blend.composite { backdrop } else { source };
            let blended = vmvnq_u32(vceqq_f32(source_alpha, zero));
            if vmaxvq_u32(blended) == 0 {
                return Some(copied);
            }
            // A group of one source pixel over one backdrop pixel blends as the last one did.
            let keys = (uniform(&source), uniform(&backdrop));
            let pair = keys.0.zip(keys.1);
            if let (Some(pair), Some(last)) = (pair, memos.result)
                && pair == last.pixels
            {
                return Some([
                    vdupq_n_f32(last.result[0]),
                    vdupq_n_f32(last.result[1]),
                    vdupq_n_f32(last.result[2]),
                    vdupq_n_f32(last.result[3]),
                ]);
            }
            let present = vcgtq_f32(backdrop_alpha, zero);
            let source_rgb = memos
                .source
                .convert(tables, &source, keys.0, blend.space, blended)?;
            let backdrop_rgb = memos.backdrop.convert(
                tables,
                &backdrop,
                keys.1,
                blend.space,
                vandq_u32(blended, present),
            )?;
            let backdrop_rgb = [
                vbslq_f32(present, backdrop_rgb[0], zero),
                vbslq_f32(present, backdrop_rgb[1], zero),
                vbslq_f32(present, backdrop_rgb[2], zero),
            ];
            let mixed = if blend.mode == DrawBlend::Multiply {
                [
                    vmulq_f32(backdrop_rgb[0], source_rgb[0]),
                    vmulq_f32(backdrop_rgb[1], source_rgb[1]),
                    vmulq_f32(backdrop_rgb[2], source_rgb[2]),
                ]
            } else {
                let screen =
                    |b: float32x4_t, s: float32x4_t| vsubq_f32(vaddq_f32(b, s), vmulq_f32(b, s));
                [
                    screen(backdrop_rgb[0], source_rgb[0]),
                    screen(backdrop_rgb[1], source_rgb[1]),
                    screen(backdrop_rgb[2], source_rgb[2]),
                ]
            };
            // `blend_rgb` fails where its operands or its result are not finite. The tables bound
            // the operands of sRGB blends, whose products and sums stay finite.
            if blend.space == BlendSpace::Linear {
                let mut failing = vdupq_n_u32(0);
                for rgb in [&source_rgb, &backdrop_rgb, &mixed] {
                    for value in rgb {
                        let finite = vcltq_f32(vabsq_f32(*value), vdupq_n_f32(f32::INFINITY));
                        failing = vorrq_u32(failing, vbicq_u32(blended, finite));
                    }
                }
                if vmaxvq_u32(failing) != 0 {
                    return None;
                }
            }
            let over = |s: float32x4_t, b: float32x4_t| {
                vaddq_f32(
                    vmulq_f32(vsubq_f32(one, backdrop_alpha), s),
                    vmulq_f32(backdrop_alpha, b),
                )
            };
            let working = to_working(
                tables,
                blend.space,
                [
                    over(source_rgb[0], mixed[0]),
                    over(source_rgb[1], mixed[1]),
                    over(source_rgb[2], mixed[2]),
                ],
                blended,
            )?;
            let keep = if blend.composite {
                vsubq_f32(one, source_alpha)
            } else {
                zero
            };
            let channel = |working: float32x4_t, backdrop: float32x4_t, copied: float32x4_t| {
                vbslq_f32(
                    blended,
                    vaddq_f32(vmulq_f32(working, source_alpha), vmulq_f32(backdrop, keep)),
                    copied,
                )
            };
            let result = [
                channel(working[0], backdrop[0], copied[0]),
                channel(working[1], backdrop[1], copied[1]),
                channel(working[2], backdrop[2], copied[2]),
                vbslq_f32(
                    blended,
                    vaddq_f32(source_alpha, vmulq_f32(backdrop[3], keep)),
                    copied[3],
                ),
            ];
            // Every lane of such a group holds the same pixel.
            if let Some(pixels) = pair {
                memos.result = Some(Repeated {
                    pixels,
                    result: [
                        vgetq_lane_f32::<0>(result[0]),
                        vgetq_lane_f32::<0>(result[1]),
                        vgetq_lane_f32::<0>(result[2]),
                        vgetq_lane_f32::<0>(result[3]),
                    ],
                });
            }
            Some(result)
        }
    }

    /// What the groups of a row span remember: the conversions of each side, as the one-pixel
    /// memos keep them, and the last group of one pixel over one pixel with its result.
    #[derive(Default)]
    struct Memos {
        source: Memo,
        backdrop: Memo,
        result: Option<Repeated>,
    }

    /// A group of one source pixel over one backdrop pixel, by bits, and the pixel it blends to.
    #[derive(Clone, Copy)]
    struct Repeated {
        pixels: ([u32; 4], [u32; 4]),
        result: [f32; 4],
    }

    /// The last group of one pixel and its conversion, as `ConversionMemo` keeps them.
    #[derive(Default)]
    struct Memo {
        key: Option<[u32; 4]>,
        value: [f32; 3],
    }

    impl Memo {
        /// `ConversionMemo::to_blend` of each lane of `pixel`, whose lanes hold the one pixel
        /// `key` where it is known, or `None` where a lane in `needed` has a magnitude beyond the
        /// transfer tables.
        #[inline(always)]
        fn convert(
            &mut self,
            tables: &TransferTables,
            pixel: &[float32x4_t; 4],
            key: Option<[u32; 4]>,
            space: BlendSpace,
            needed: uint32x4_t,
        ) -> Option<[float32x4_t; 3]> {
            // SAFETY: NEON is part of the aarch64 baseline.
            unsafe {
                if key.is_some() && key == self.key {
                    return Some([
                        vdupq_n_f32(self.value[0]),
                        vdupq_n_f32(self.value[1]),
                        vdupq_n_f32(self.value[2]),
                    ]);
                }
                let alpha = pixel[3];
                let linear = multiply(
                    &WORKING_TO_LINEAR_SRGB,
                    [
                        vdivq_f32(pixel[0], alpha),
                        vdivq_f32(pixel[1], alpha),
                        vdivq_f32(pixel[2], alpha),
                    ],
                );
                let (rgb, beyond) = if space == BlendSpace::Linear {
                    (linear, vdupq_n_u32(0))
                } else {
                    let (r, beyond_r) = encode(&tables.encode, linear[0]);
                    let (g, beyond_g) = encode(&tables.encode, linear[1]);
                    let (b, beyond_b) = encode(&tables.encode, linear[2]);
                    let beyond = vorrq_u32(vorrq_u32(beyond_r, beyond_g), beyond_b);
                    ([r, g, b], beyond)
                };
                if vmaxvq_u32(vandq_u32(beyond, needed)) != 0 {
                    return None;
                }
                // A lane beyond the tables holds no conversion, needed or not.
                if key.is_some() && vmaxvq_u32(beyond) == 0 {
                    self.key = key;
                    self.value = [
                        vgetq_lane_f32::<0>(rgb[0]),
                        vgetq_lane_f32::<0>(rgb[1]),
                        vgetq_lane_f32::<0>(rgb[2]),
                    ];
                }
                Some(rgb)
            }
        }
    }

    /// The bits of the one pixel every lane of `pixel` holds, if they hold one.
    #[inline(always)]
    fn uniform(pixel: &[float32x4_t; 4]) -> Option<[u32; 4]> {
        // SAFETY: NEON is part of the aarch64 baseline.
        unsafe {
            let bits = [
                vreinterpretq_u32_f32(pixel[0]),
                vreinterpretq_u32_f32(pixel[1]),
                vreinterpretq_u32_f32(pixel[2]),
                vreinterpretq_u32_f32(pixel[3]),
            ];
            let same = |value: uint32x4_t| vceqq_u32(value, vdupq_laneq_u32::<0>(value));
            let all = vandq_u32(
                vandq_u32(same(bits[0]), same(bits[1])),
                vandq_u32(same(bits[2]), same(bits[3])),
            );
            (vminvq_u32(all) == u32::MAX).then(|| {
                [
                    vgetq_lane_u32::<0>(bits[0]),
                    vgetq_lane_u32::<0>(bits[1]),
                    vgetq_lane_u32::<0>(bits[2]),
                    vgetq_lane_u32::<0>(bits[3]),
                ]
            })
        }
    }

    /// `blend_to_working` of each lane, or `None` where a lane in `needed` has a magnitude
    /// beyond the transfer tables.
    #[inline(always)]
    fn to_working(
        tables: &TransferTables,
        space: BlendSpace,
        rgb: [float32x4_t; 3],
        needed: uint32x4_t,
    ) -> Option<[float32x4_t; 3]> {
        // SAFETY: NEON is part of the aarch64 baseline.
        unsafe {
            let rgb = if space == BlendSpace::Linear {
                rgb
            } else {
                let (r, beyond_r) = decode(&tables.decode, rgb[0]);
                let (g, beyond_g) = decode(&tables.decode, rgb[1]);
                let (b, beyond_b) = decode(&tables.decode, rgb[2]);
                let beyond = vorrq_u32(vorrq_u32(beyond_r, beyond_g), beyond_b);
                if vmaxvq_u32(vandq_u32(beyond, needed)) != 0 {
                    return None;
                }
                [r, g, b]
            };
            Some(multiply(&LINEAR_SRGB_TO_WORKING, rgb))
        }
    }

    /// `multiply` of each lane: products and sums in f64, in its order, rounded once.
    #[inline(always)]
    fn multiply(matrix: &[[f64; 3]; 3], rgb: [float32x4_t; 3]) -> [float32x4_t; 3] {
        // SAFETY: NEON is part of the aarch64 baseline.
        unsafe {
            let low = [
                vcvt_f64_f32(vget_low_f32(rgb[0])),
                vcvt_f64_f32(vget_low_f32(rgb[1])),
                vcvt_f64_f32(vget_low_f32(rgb[2])),
            ];
            let high = [
                vcvt_high_f64_f32(rgb[0]),
                vcvt_high_f64_f32(rgb[1]),
                vcvt_high_f64_f32(rgb[2]),
            ];
            let row = |m: &[f64; 3]| {
                let sum = |c: &[float64x2_t; 3]| {
                    vaddq_f64(
                        vaddq_f64(
                            vmulq_f64(vdupq_n_f64(m[0]), c[0]),
                            vmulq_f64(vdupq_n_f64(m[1]), c[1]),
                        ),
                        vmulq_f64(vdupq_n_f64(m[2]), c[2]),
                    )
                };
                vcvt_high_f32_f64(vcvt_f32_f64(sum(&low)), sum(&high))
            };
            [row(&matrix[0]), row(&matrix[1]), row(&matrix[2])]
        }
    }

    /// `encode_with` of each lane, and the lanes past the table, which take the exact curve.
    #[inline(always)]
    fn encode(table: &[f32], value: float32x4_t) -> (float32x4_t, uint32x4_t) {
        // SAFETY: NEON is part of the aarch64 baseline.
        unsafe {
            let magnitude = vabsq_f32(value);
            let (looked_up, beyond) = lookup(table, magnitude);
            let encoded = vbslq_f32(
                vcleq_f32(magnitude, vdupq_n_f32(0.003_130_8)),
                vmulq_f32(magnitude, vdupq_n_f32(12.92)),
                looked_up,
            );
            (copysign(encoded, value), beyond)
        }
    }

    /// `decode_with` of each lane, and the lanes past the table, which take the exact curve.
    #[inline(always)]
    fn decode(table: &[f32], value: float32x4_t) -> (float32x4_t, uint32x4_t) {
        // SAFETY: NEON is part of the aarch64 baseline.
        unsafe {
            let magnitude = vabsq_f32(value);
            let (looked_up, beyond) = lookup(table, magnitude);
            let decoded = vbslq_f32(
                vcleq_f32(magnitude, vdupq_n_f32(0.040_45)),
                vdivq_f32(magnitude, vdupq_n_f32(12.92)),
                looked_up,
            );
            (copysign(decoded, value), beyond)
        }
    }

    /// `lookup` of each lane, and the lanes past the table's limit (NaN included), where the
    /// interpolation means nothing.
    #[inline(always)]
    fn lookup(table: &[f32], magnitude: float32x4_t) -> (float32x4_t, uint32x4_t) {
        debug_assert_eq!(table.len(), TRANSFER_TABLE_STEPS + 1);
        // SAFETY: NEON is part of the aarch64 baseline. An index is at most
        // `TRANSFER_TABLE_STEPS - 1`, and the tables hold `TRANSFER_TABLE_STEPS + 1` entries.
        unsafe {
            let position = vmulq_f32(magnitude, vdupq_n_f32(TRANSFER_TABLE_RESOLUTION as f32));
            // Saturating like `as usize`, NaN to zero.
            let index = vminq_u32(
                vcvtq_u32_f32(position),
                vdupq_n_u32((TRANSFER_TABLE_STEPS - 1) as u32),
            );
            let pair = |lane: u32| vld1_f32(table.as_ptr().add(lane as usize));
            let low = vcombine_f32(
                pair(vgetq_lane_u32::<0>(index)),
                pair(vgetq_lane_u32::<1>(index)),
            );
            let high = vcombine_f32(
                pair(vgetq_lane_u32::<2>(index)),
                pair(vgetq_lane_u32::<3>(index)),
            );
            let start = vuzp1q_f32(low, high);
            let end = vuzp2q_f32(low, high);
            let value = vaddq_f32(
                start,
                vmulq_f32(
                    vsubq_f32(end, start),
                    vsubq_f32(position, vcvtq_f32_u32(index)),
                ),
            );
            let inside = vcleq_f32(magnitude, vdupq_n_f32(TRANSFER_TABLE_LIMIT));
            (value, vmvnq_u32(inside))
        }
    }

    /// `magnitude` with the sign of `sign`, as `f32::copysign`.
    #[inline(always)]
    fn copysign(magnitude: float32x4_t, sign: float32x4_t) -> float32x4_t {
        // SAFETY: NEON is part of the aarch64 baseline.
        unsafe { vbslq_f32(vdupq_n_u32(0x8000_0000), sign, magnitude) }
    }
}

fn internal(message: &str) -> DrawError {
    DrawError::Internal(message.to_owned())
}

const fn space_code(space: BlendSpace) -> f32 {
    match space {
        BlendSpace::Linear => 1.0,
        BlendSpace::Srgb => 0.0,
    }
}

const fn draw_code(mode: DrawBlend) -> f32 {
    match mode {
        DrawBlend::Normal => 0.0,
        DrawBlend::Plus => 17.0,
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
                BlendSpace::Srgb,
                opacity,
            )
            .unwrap();
        let actual = output.image_snapshot();
        let expected = blend_over(
            source_value.scale_coverage(opacity).unwrap(),
            destination_value,
            ReferenceBlendMode::LinearBurn,
            BlendSpace::Srgb,
        )
        .unwrap();
        let actual = read(&actual);
        assert!(
            actual.approx_eq(expected, 0.004),
            "actual={actual:?} expected={expected:?}"
        );
    }

    #[test]
    fn f16_storage_error_is_bounded_separately_from_blending() {
        let info = working_info(Extent2d::new(1, 1).unwrap()).unwrap();
        for channels in [
            [0.9, 0.4, 0.1, 1.0],
            [0.05, 0.6, 0.95, 1.0],
            [0.35, 0.12, 0.08, 0.7],
            [1.3, 0.2, 0.05, 1.0],
        ] {
            let intended = PremulRgba32::from_premultiplied(channels).unwrap();
            let stored = read(&solid(intended, &info));
            for (before, after) in channels.into_iter().zip(stored.channels()) {
                // One F16 ULP, including the premultiply/unpremultiply paint round trip.
                assert!(
                    (before - after).abs() <= 0.001 * before.abs().max(1.0),
                    "intended={intended:?}, stored={stored:?}"
                );
            }
        }
    }

    #[test]
    fn raster_path_matches_kernel_and_reference_for_every_mode() {
        let modes = [
            DrawBlend::Plus,
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
        for (mode, space) in modes
            .into_iter()
            .flat_map(|mode| [BlendSpace::Srgb, BlendSpace::Linear].map(|space| (mode, space)))
        {
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
                            space,
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
                        &[
                            draw_code(mode),
                            opacity,
                            if composite { 1.0 } else { 0.0 },
                            space_code(space),
                        ],
                    )
                    .unwrap();
                    let native = read(&native.image_snapshot());
                    let kernel = read(&kernel.image_snapshot());
                    // Compare the blend algorithm using the actual F16 inputs. ColorBurn
                    // amplifies input rounding near its kink; pre-storage values are not
                    // the operands seen by either the raster or shader implementation.
                    let source_value = read(&source);
                    let destination_value = read(&destination);
                    let scaled = source_value.scale_coverage(opacity).unwrap();
                    let expected = if composite {
                        blend_over(scaled, destination_value, mode.into(), space).unwrap()
                    } else {
                        valle_engine::compositor::reference::effective_blend_source(
                            scaled,
                            destination_value,
                            mode.into(),
                            space,
                        )
                        .unwrap()
                    };
                    let context = format!(
                        "{mode:?}/{space:?} opacity={opacity} composite={composite} \
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
    fn placed_operands_keep_the_bytes_of_drawn_operands() {
        let info = working_info(Extent2d::new(37, 29).unwrap()).unwrap();
        let runtime = BlendRuntime::admit().unwrap();
        // Larger than the output, as slot snapshots are, with opaque, partial and clear pixels.
        let operand = |colors: [Color4f; 3]| {
            let mut surface =
                raster_surface(&working_info(Extent2d::new(41, 33).unwrap()).unwrap()).unwrap();
            surface.canvas().clear(colors[0]);
            for (index, color) in colors[1..].iter().enumerate() {
                let mut paint = Paint::new(*color, None);
                paint.set_blend_mode(BlendMode::Src);
                let at = 4.0 + 9.0 * index as f32;
                surface
                    .canvas()
                    .draw_rect(skia_safe::Rect::from_xywh(at, at / 2.0, 19.0, 13.0), &paint);
            }
            surface.image_snapshot()
        };
        let source_image = operand([
            Color4f::new(0.8, 0.3, 0.1, 1.0),
            Color4f::new(0.2, 0.6, 0.9, 0.5),
            Color4f::new(0.0, 0.0, 0.0, 0.0),
        ]);
        let destination_image = operand([
            Color4f::new(0.1, 0.5, 0.3, 0.7),
            Color4f::new(0.9, 0.9, 0.2, 1.0),
            Color4f::new(0.4, 0.1, 0.6, 0.25),
        ]);
        // As `blend_source_into` aligns operands: drawn under Src into transparent images.
        let drawn = |placed: &Placed<'_>| {
            let mut surface = raster_surface(&info).unwrap();
            surface.canvas().clear(Color4f::new(0.0, 0.0, 0.0, 0.0));
            if !placed.rect.is_empty() {
                let mut paint = Paint::default();
                paint.set_blend_mode(BlendMode::Src);
                surface.canvas().draw_image_rect_with_sampling_options(
                    placed.image,
                    Some((
                        &skia_safe::Rect::from_xywh(
                            0.0,
                            0.0,
                            placed.rect.width() as f32,
                            placed.rect.height() as f32,
                        ),
                        skia_safe::canvas::SrcRectConstraint::Strict,
                    )),
                    skia_safe::Rect::from(placed.rect),
                    skia_safe::SamplingOptions::default(),
                    &paint,
                );
            }
            surface.image_snapshot()
        };
        let bytes = |surface: &mut Surface| {
            let mut bytes = vec![0_u8; 37 * 29 * 8];
            assert!(surface.read_pixels(&info, &mut bytes, 37 * 8, (0, 0)));
            bytes
        };
        let rects = [
            IRect::from_xywh(0, 0, 37, 29),
            IRect::from_xywh(5, 3, 20, 17),
            IRect::from_xywh(-6, -4, 30, 25),
            IRect::from_xywh(20, 15, 21, 30),
            IRect::from_xywh(3, 2, 0, 0),
            IRect::from_xywh(50, 1, 10, 10),
        ];
        for mode in [DrawBlend::Multiply, DrawBlend::Plus, DrawBlend::Hue] {
            for space in [BlendSpace::Srgb, BlendSpace::Linear] {
                for (source_rect, destination_rect) in rects
                    .into_iter()
                    .flat_map(|source| [rects[0], rects[2], rects[3]].map(|dest| (source, dest)))
                {
                    let source = Placed {
                        image: &source_image,
                        rect: source_rect,
                    };
                    let destination = Placed {
                        image: &destination_image,
                        rect: destination_rect,
                    };
                    let mut expected = raster_surface(&info).unwrap();
                    runtime
                        .effective_source_into(
                            &mut expected,
                            &drawn(&source),
                            &drawn(&destination),
                            mode,
                            space,
                        )
                        .unwrap();
                    let mut actual = raster_surface(&info).unwrap();
                    assert!(
                        runtime
                            .effective_placed_into(&mut actual, source, destination, mode, space)
                            .unwrap()
                    );
                    assert_eq!(
                        bytes(&mut actual),
                        bytes(&mut expected),
                        "{mode:?}/{space:?} source {source_rect:?} destination {destination_rect:?}"
                    );
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
                raster_blend_into(
                    &mut output,
                    &source_image,
                    &backdrop_image,
                    mode,
                    BlendSpace::Srgb,
                    0.9,
                    true
                )
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
                    BlendSpace::Srgb,
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

    /// Operand bytes for the four-pixel blends: premultiplied colors with every kind of alpha (NaN
    /// only with `nan`), grays at the edges of the transfer curves, which the matrices keep gray,
    /// colors past the tables, and runs of one pixel: each pixel repeats the last one with
    /// probability `repeat`.
    #[cfg(all(target_arch = "aarch64", target_endian = "little"))]
    fn operands(count: usize, repeat: f32, nan: bool, state: &mut u64) -> Vec<u8> {
        let mut unit = || {
            *state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (*state >> 40) as f32 / (1 << 24) as f32
        };
        let mut pixel = [0.0_f32; 4];
        let mut bytes = Vec::with_capacity(count * 16);
        for index in 0..count {
            if index == 0 || unit() >= repeat {
                let alpha = match (unit() * 8.0) as u32 {
                    0 => 0.0,
                    1 => -0.0,
                    2 => 1e-3,
                    3 => -0.25,
                    4 if nan => f32::NAN,
                    4 | 5 => 1.0,
                    _ => unit(),
                };
                let kind = unit();
                let rgb = if kind < 0.2 {
                    let edge = [0.003_130_8_f32, 0.040_45, 1.0, 2.0][(unit() * 4.0) as usize];
                    [edge * (1.0 + (unit() - 0.5) * 4e-6); 3]
                } else if kind < 0.3 {
                    [1.5 + 2.0 * unit(), 1.5 + 2.0 * unit(), unit()]
                } else {
                    [unit() * 1.4 - 0.2, unit() * 1.4 - 0.2, unit() * 1.4 - 0.2]
                };
                pixel = [rgb[0] * alpha, rgb[1] * alpha, rgb[2] * alpha, alpha];
            }
            bytes.extend(pixel.iter().flat_map(|channel| channel.to_ne_bytes()));
        }
        bytes
    }

    #[cfg(all(target_arch = "aarch64", target_endian = "little"))]
    #[test]
    fn four_pixel_blends_keep_the_bytes_and_failures_of_one_pixel_blends() {
        let tables = transfer_tables();
        // Groups of four pixels, then a tail of three.
        let count = 4 * 1024 + 3;
        let compare = |blend: PixelBlend, source: &[u8], backdrop: &[u8]| {
            let mut fused_out = source.to_vec();
            let mut one_out = source.to_vec();
            assert!(fused_out.as_ptr().cast::<f32>().is_aligned());
            assert!(backdrop.as_ptr().cast::<f32>().is_aligned());
            let fused_result = fused::blend_rows(tables, blend, &mut fused_out, backdrop);
            let one_result = blend_pixels(tables, blend, &mut one_out, backdrop);
            let what = format!(
                "{:?} {:?} composite {} opacity {}",
                blend.mode, blend.space, blend.composite, blend.opacity
            );
            assert_eq!(fused_result, one_result, "{what}");
            let differing = fused_out
                .as_chunks::<16>()
                .0
                .iter()
                .zip(one_out.as_chunks::<16>().0)
                .position(|(fused, one)| fused != one);
            assert_eq!(differing, None, "{what}");
            one_result
        };
        let mut state = 0x5eed_u64;
        for round in 0..3 {
            let source = operands(count, 0.6, false, &mut state);
            let backdrop = operands(count, 0.9, true, &mut state);
            for mode in [DrawBlend::Multiply, DrawBlend::Screen, DrawBlend::Overlay] {
                for space in [BlendSpace::Srgb, BlendSpace::Linear] {
                    for (composite, opacity) in
                        [(false, 1.0), (true, 1.0), (false, 0.7), (true, 0.7)]
                    {
                        let blend = PixelBlend {
                            mode,
                            reference: mode.into(),
                            space,
                            opacity,
                            composite,
                        };
                        assert_eq!(compare(blend, &source, &backdrop), Ok(()));
                        if round > 0 {
                            continue;
                        }
                        // Both report the first pixel they cannot blend, in a group or the tail.
                        let failing = [
                            ([f32::INFINITY, 0.5, 0.5, 1.0], None),
                            ([0.5, 0.5, 0.5, f32::NAN], None),
                            ([0.3, 0.3, 0.3, 1.0], Some([0.5, f32::NAN, 0.5, 1.0])),
                        ];
                        for at in [0, 6, count - 2] {
                            for (source_pixel, backdrop_pixel) in failing {
                                let mut source = source.clone();
                                let mut backdrop = backdrop.clone();
                                write_rgba(&mut source[at * 16..][..16], source_pixel);
                                if let Some(pixel) = backdrop_pixel {
                                    write_rgba(&mut backdrop[at * 16..][..16], pixel);
                                }
                                let result = compare(blend, &source, &backdrop);
                                assert!(matches!(result, Err((index, _)) if index == at));
                            }
                        }
                    }
                }
            }
        }
    }

    #[cfg(all(target_arch = "aarch64", target_endian = "little"))]
    #[test]
    fn half_blends_keep_the_bytes_and_failures_of_float_blends() {
        let tables = transfer_tables();
        let working = working_color_space().unwrap();
        let mut state = 0xb1e2d_u64;
        // Rows with a tail of one pixel; the larger output spreads its rows over workers.
        for (width, height) in [(37, 9), (203, 170)] {
            let info = ImageInfo::new(
                (width, height),
                ColorType::RGBAF32,
                AlphaType::Premul,
                Some(working.clone()),
            );
            let output_info =
                working_info(Extent2d::new(width as u32, height as u32).unwrap()).unwrap();
            // Operands larger than the output, in halves.
            let (operand_width, operand_height) = (width + 3, height + 2);
            let halves = |floats: &[u8]| {
                let float_info = info.with_dimensions((operand_width, operand_height));
                let image = skia_safe::images::raster_from_data(
                    &float_info,
                    skia_safe::Data::new_copy(floats),
                    operand_width as usize * 16,
                )
                .unwrap();
                let half_info = output_info.with_dimensions((operand_width, operand_height));
                let mut bytes = vec![0_u8; (operand_width * operand_height) as usize * 8];
                assert!(image.read_pixels(
                    &half_info,
                    &mut bytes,
                    operand_width as usize * 8,
                    (0, 0),
                    CachingHint::Disallow,
                ));
                skia_safe::images::raster_from_data(
                    &half_info,
                    skia_safe::Data::new_copy(&bytes),
                    operand_width as usize * 8,
                )
                .unwrap()
            };
            let count = (operand_width * operand_height) as usize;
            let source_floats = operands(count, 0.6, false, &mut state);
            let source = halves(&source_floats);
            let backdrop = halves(&operands(count, 0.9, true, &mut state));
            let mut failing = source_floats.clone();
            write_rgba(
                &mut failing[(operand_width as usize * 5 + 7) * 16..][..16],
                [f32::INFINITY, 0.5, 0.5, 1.0],
            );
            let failing = halves(&failing);
            let bytes =
                |surface: &mut Surface| surface.peek_pixels().unwrap().bytes().unwrap().to_vec();
            for (rect, covers) in [
                (IRect::from_wh(width, height), true),
                (
                    IRect::from_xywh(-2, -1, operand_width, operand_height),
                    true,
                ),
                (IRect::from_xywh(1, 0, width, height), false),
            ] {
                let placed = |image| Placed { image, rect };
                for mode in [DrawBlend::Multiply, DrawBlend::Screen, DrawBlend::Overlay] {
                    for space in [BlendSpace::Srgb, BlendSpace::Linear] {
                        for (composite, opacity) in [(false, 1.0), (true, 0.7)] {
                            let blend = PixelBlend {
                                mode,
                                reference: mode.into(),
                                space,
                                opacity,
                                composite,
                            };
                            let what = format!("{mode:?} {space:?} {composite} {opacity} {rect:?}");
                            let mut halves_out = raster_surface(&output_info).unwrap();
                            let took = fused::blend_halves(
                                &mut halves_out,
                                &placed(&source),
                                &placed(&backdrop),
                                blend,
                                tables,
                                &info,
                            )
                            .unwrap();
                            assert_eq!(took, covers && mode != DrawBlend::Overlay, "{what}");
                            if !took {
                                continue;
                            }
                            let mut floats_out = raster_surface(&output_info).unwrap();
                            blend_floats(
                                &mut floats_out,
                                &placed(&source),
                                &placed(&backdrop),
                                blend,
                                tables,
                                &info,
                            )
                            .unwrap();
                            assert!(bytes(&mut halves_out) == bytes(&mut floats_out), "{what}");
                            // Both report a pixel the blend cannot take.
                            let halves_error = fused::blend_halves(
                                &mut halves_out,
                                &placed(&failing),
                                &placed(&backdrop),
                                blend,
                                tables,
                                &info,
                            )
                            .unwrap_err();
                            let floats_error = blend_floats(
                                &mut floats_out,
                                &placed(&failing),
                                &placed(&backdrop),
                                blend,
                                tables,
                                &info,
                            )
                            .unwrap_err();
                            assert_eq!(
                                format!("{halves_error:?}"),
                                format!("{floats_error:?}"),
                                "{what}"
                            );
                        }
                    }
                }
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
