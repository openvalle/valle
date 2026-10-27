use skia_safe::{
    AlphaType, ColorSpace, ColorType, Image, ImageInfo, image::CachingHint, named_primaries,
    named_transfer_fn,
};
#[cfg(target_arch = "aarch64")]
use valle_engine::compositor::bloom::half_to_f32;
use valle_engine::{
    compositor::delivery::{DeliveryStorage, transform_working_pixel},
    resource::{
        ColorPrimaries, Dither, GamutMap, OutputAlphaMode, OutputBitDepth, OutputSpec,
        TransferFunction,
    },
};

use super::{draw::DrawError, surface::working_color_space};

mod cpu;

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

// The encoded destination needs a full frame; the float readback does not. Processing stripes
// bounds this scratch to 3.75 MiB at 4K instead of retaining 126.6 MiB in every CPU worker.
const OUTPUT_STRIPE_ROWS: usize = 64;

/// Reusable CPU output buffers. Each frame rewrites the pixels; cached SDR colors are keyed by
/// all input bits and the complete output contract, before position-dependent dither.
#[derive(Debug, Default)]
pub(crate) struct OutputStaging {
    working: Vec<f32>,
    pixels: Vec<u8>,
    sdr: Option<CachedSdrOutput>,
}

#[derive(Debug)]
struct CachedSdrOutput {
    _memory: Option<valle_engine::motion::memory::MemoryLease>,
    spec: OutputSpec,
    converter: cpu::Converter,
}

/// Opaque SDR delivery directly from resident F16 working pixels. This is an RGBA8/BGRA8
/// terminal, retaining the normal encoder path. HDR, coverage alpha, tone mapping and other
/// source formats keep their existing conversion. The interpolated sRGB curve is bounded to
/// less than 0.07 of one 8-bit code; final bytes can differ from the exact reference by one.
pub(crate) fn stage_cpu_sdr_output_into(
    image: &Image,
    spec: OutputSpec,
    target_info: &ImageInfo,
    staging: &mut OutputStaging,
) -> Result<Option<usize>, DrawError> {
    use valle_engine::{
        motion::memory::{self, MemoryKind},
        resource::ToneMap,
    };
    if spec.alpha() != OutputAlphaMode::Opaque
        || spec.bit_depth() != OutputBitDepth::Eight
        || spec.target().primaries != ColorPrimaries::Rec709
        || spec.target().transfer != TransferFunction::Srgb
        || spec.tone_map() != ToneMap::None
        || spec.reference_white().get() != 100
    {
        staging.sdr = None;
        return Ok(None);
    }
    validate_target(spec, target_info)?;
    if image.dimensions() != target_info.dimensions() {
        return Err(DrawError::Surface("output extent mismatch".into()));
    }
    let Some(pixmap) = image.peek_pixels() else {
        return Ok(None);
    };
    if pixmap.color_type() != ColorType::RGBAF16
        || pixmap.alpha_type() != AlphaType::Premul
        || pixmap.color_space().as_ref()
            != Some(&working_color_space().map_err(|e| DrawError::Surface(e.to_string()))?)
    {
        return Ok(None);
    }
    let Some(bytes) = pixmap.bytes() else {
        return Ok(None);
    };
    if staging.sdr.as_ref().is_none_or(|state| state.spec != spec) {
        staging.sdr = None;
        // This optional cache survives frames, so it must not spend a frame's working credit.
        // Under pressure the existing converter remains available without this allocation.
        let Ok(memory) = memory::reserve_retained(
            MemoryKind::Delivery,
            cpu::Converter::retained_bytes(spec.dither()) as u64,
        ) else {
            return Ok(None);
        };
        let Ok(_temporary) = memory::reserve_retained(
            MemoryKind::Temporary,
            cpu::Converter::construction_bytes(spec.dither()) as u64,
        ) else {
            return Ok(None);
        };
        staging.sdr = Some(CachedSdrOutput {
            _memory: memory,
            spec,
            converter: cpu::Converter::new(spec),
        });
    }
    let cached = prefers_cached_output(image);
    let state = staging.sdr.as_mut().unwrap();
    let width = image.width() as usize;
    let height = image.height() as usize;
    staging.pixels.resize(width * height * 4, 0);
    let swap = target_info.color_type() == ColorType::BGRA8888;
    for (y, output) in staging.pixels.chunks_exact_mut(width * 4).enumerate() {
        state
            .converter
            .row(
                &bytes[y * pixmap.row_bytes()..][..width * 8],
                y,
                swap,
                cached,
                output,
            )
            .map_err(|error| DrawError::Surface(error.into()))?;
    }
    Ok(Some(width * 4))
}

impl OutputStaging {
    pub(crate) fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub(crate) fn clear_cache(&mut self) {
        self.sdr = None;
    }
}

// Sample existing raster pixels without readback. Continuous-tone frames (gradients/video)
// favor uncached batches; flat-color graphics favor exact-key memoization. This selects cost,
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
    let color_type = target_info.color_type();
    let pixel_bytes = color_type.bytes_per_pixel();
    // Every admitted target stores four 8-bit or four 16-bit channels. Fixing the size per
    // instantiation makes each pixel's code one fixed-size store, not a memmove call.
    let stage = match pixel_bytes {
        4 => stage_working::<4>,
        8 => stage_working::<8>,
        _ => {
            return Err(DrawError::Surface(format!(
                "unsupported staged output pixel size for {color_type:?}"
            )));
        }
    };
    // The stripes below rewrite every pixel, so only a change of size needs new storage.
    staging.pixels.resize(sample_count / 4 * pixel_bytes, 0);
    stage(
        image,
        spec,
        color_type,
        &read_info,
        row_bytes,
        &mut staging.working,
        &mut staging.pixels,
    )?;
    width
        .checked_mul(pixel_bytes)
        .ok_or_else(|| DrawError::Surface("target row is too large".into()))
}

fn stage_working<const N: usize>(
    image: &Image,
    spec: OutputSpec,
    color_type: ColorType,
    read_info: &ImageInfo,
    row_bytes: usize,
    working: &mut Vec<f32>,
    pixels: &mut [u8],
) -> Result<(), DrawError> {
    let width = image.width() as usize;
    let height = image.height() as usize;
    let (pixels, _) = pixels.as_chunks_mut::<N>();
    // Skia's aarch64 readback widens halves with the same exact hardware conversion, so a Raster
    // frame in the working format can be keyed by its stored halves, widening only cache misses,
    // with no readback at all. Other targets keep the readback: without F16C, Skia flushes half
    // subnormals to zero.
    #[cfg(target_arch = "aarch64")]
    if let Some(pixmap) = image.peek_pixels()
        && pixmap.color_type() == ColorType::RGBAF16
        && pixmap.alpha_type() == AlphaType::Premul
        && pixmap.color_space() == read_info.color_space()
        && let Some(bytes) = pixmap.bytes()
    {
        let mut encoder = SampleEncoder::<u64, N>::new(spec, color_type, width);
        for (y, output) in pixels.chunks_exact_mut(width).enumerate() {
            let (halves, _) = bytes[y * pixmap.row_bytes()..][..width * 8].as_chunks::<8>();
            for (x, (half, output)) in halves.iter().zip(output).enumerate() {
                *output = encoder.encode(u64::from_ne_bytes(*half), y * width + x, || {
                    std::array::from_fn(|channel| {
                        half_to_f32(u16::from_ne_bytes([
                            half[2 * channel],
                            half[2 * channel + 1],
                        ]))
                    })
                })?;
            }
        }
        return Ok(());
    }
    let mut encoder = SampleEncoder::<[u32; 4], N>::new(spec, color_type, width);
    for top in (0..height).step_by(OUTPUT_STRIPE_ROWS) {
        let rows = OUTPUT_STRIPE_ROWS.min(height - top);
        // read_pixels overwrites every sample, so stale contents from a prior frame are never
        // read.
        working.resize(width * rows * 4, 0.0);
        if !image.read_pixels(
            &read_info.with_dimensions((image.width(), rows as i32)),
            working.as_mut_slice(),
            row_bytes,
            (0, top as i32),
            CachingHint::Disallow,
        ) {
            return Err(DrawError::Surface("working output readback failed".into()));
        }
        let start_pixel = top * width;
        let (samples, _) = working.as_chunks::<4>();
        for (local_index, (channels, output)) in samples
            .iter()
            .zip(&mut pixels[start_pixel..start_pixel + rows * width])
            .enumerate()
        {
            *output = encoder.encode(
                channels.map(f32::to_bits),
                start_pixel + local_index,
                || *channels,
            )?;
        }
    }
    Ok(())
}

/// The exact bits of one working sample, as read.
trait SampleKey: Copy + Eq {
    fn slot(self) -> usize;
}

impl SampleKey for [u32; 4] {
    fn slot(self) -> usize {
        let hash = self
            .iter()
            .fold(0_u32, |hash, bits| hash.rotate_left(5) ^ bits)
            .wrapping_mul(0x9e3779b9);
        (hash >> (32 - SDR_CACHE_BITS)) as usize
    }
}

/// Four stored binary16 channels.
impl SampleKey for u64 {
    fn slot(self) -> usize {
        (self.wrapping_mul(0x9e37_79b9_7f4a_7c15) >> (64 - SDR_CACHE_BITS)) as usize
    }
}

/// Encodes working samples into target pixels. Non-dithered conversion is position-independent,
/// so a small, frame-local cache avoids repeating transfer/gamut math for flat fills and text
/// without approximating any pixels. Entries hold the final encoded bytes, so hits are a single
/// fixed-size copy.
struct SampleEncoder<K, const N: usize> {
    spec: OutputSpec,
    color_type: ColorType,
    width: usize,
    cacheable: bool,
    // Sized for a full row of a horizontal gradient (1920 distinct inputs at 1080p), which a
    // smaller direct-mapped cache would evict every row. At about 100 KiB it stays on the heap.
    cache: Vec<Option<(K, [u8; N])>>,
    // Flat regions repeat the previous pixel; it is checked before hashing.
    previous: Option<(K, [u8; N])>,
    buffer: Vec<u8>,
}

impl<K: SampleKey, const N: usize> SampleEncoder<K, N> {
    fn new(spec: OutputSpec, color_type: ColorType, width: usize) -> Self {
        Self {
            spec,
            color_type,
            width,
            cacheable: spec.dither() == Dither::None,
            cache: vec![None; SDR_CACHE_SLOTS],
            previous: None,
            buffer: Vec::with_capacity(N),
        }
    }

    /// `index` is the pixel's position in the frame; `channels` gives the sample on a miss.
    #[inline(always)]
    fn encode(
        &mut self,
        key: K,
        index: usize,
        channels: impl FnOnce() -> [f32; 4],
    ) -> Result<[u8; N], DrawError> {
        if self.cacheable
            && let Some((prior, encoded)) = self.previous
            && prior == key
        {
            return Ok(encoded);
        }
        let slot = key.slot();
        let encoded = match self.cache[slot] {
            Some((prior, encoded)) if self.cacheable && prior == key => encoded,
            _ => {
                // Dither remains anchored to the original frame, including the last short stripe.
                let sample = transform_working_pixel(
                    canonical_rgba(channels())?,
                    self.spec,
                    [(index % self.width) as u32, (index / self.width) as u32],
                )
                .map_err(|error| DrawError::Surface(error.to_string()))?;
                self.buffer.clear();
                append_storage(&mut self.buffer, sample.storage, self.color_type, self.spec)?;
                let encoded: [u8; N] = self.buffer.as_slice().try_into().map_err(|_| {
                    DrawError::Surface(format!(
                        "{:?} staging produced {} bytes per pixel",
                        self.color_type,
                        self.buffer.len()
                    ))
                })?;
                if self.cacheable {
                    self.cache[slot] = Some((key, encoded));
                }
                encoded
            }
        };
        if self.cacheable {
            self.previous = Some((key, encoded));
        }
        Ok(encoded)
    }
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
    let height = image.height() as usize;
    let pixel_count = width * height;
    // read_pixels overwrites every sample, so stale contents from a prior frame are never read.
    let samples = &mut staging.working;
    // Loop invariants, queried once rather than per pixel.
    let codes = SdrCodes::new(spec, target_info.color_type());
    let row_bytes = target_info.min_row_bytes();
    let pixels = &mut staging.pixels;
    // The stripes below rewrite every pixel, so only a change of size needs new storage.
    pixels.resize(pixel_count * 4, 0);
    // Continuous-tone frames still repeat neighbors in flat regions; an identical input reuses
    // the previous pixel's already validated codes.
    #[cfg(not(all(target_arch = "aarch64", target_endian = "little")))]
    let mut previous: Option<([u32; 4], [u8; 4])> = None;
    // Gradients repeat a color down a column rather than along a row, so a small exact-key cache
    // catches what the previous-pixel check misses. Keys are the input bits, so hits are exact.
    #[cfg(not(all(target_arch = "aarch64", target_endian = "little")))]
    let mut cache = vec![None::<([u32; 4], [u8; 4])>; SDR_CACHE_SLOTS];
    for top in (0..height).step_by(OUTPUT_STRIPE_ROWS) {
        let rows = OUTPUT_STRIPE_ROWS.min(height - top);
        samples.resize(width * rows * 4, 0.0);
        if !image.read_pixels(
            &info.with_dimensions((image.width(), rows as i32)),
            samples.as_mut_slice(),
            width * 16,
            (0, top as i32),
            CachingHint::Disallow,
        ) {
            return Err(DrawError::Surface("linear output conversion failed".into()));
        }
        let (stripe_samples, _) = samples.as_chunks::<4>();
        let (stripe_output, _) = pixels[top * width * 4..(top + rows) * width * 4].as_chunks_mut();
        // Four pixels at a time compute what one does, lane by lane; a group that leaves the
        // common path, and the stripe's last pixels, go one by one and fail where a scan would.
        // Flat regions repeat the previous pixel and gradients across a row repeat the row
        // above, so a group with exactly their inputs reuses their already validated codes.
        #[cfg(all(target_arch = "aarch64", target_endian = "little"))]
        {
            let same = |pixel: &[f32; 4], other: &[f32; 4]| {
                pixel.map(f32::to_bits) == other.map(f32::to_bits)
            };
            let (groups, samples_tail) = stripe_samples.as_chunks::<4>();
            for (index, group) in groups.iter().enumerate() {
                let first = index * 4;
                if first > 0
                    && group
                        .iter()
                        .all(|pixel| same(pixel, &stripe_samples[first - 1]))
                {
                    let previous = stripe_output[first - 1];
                    stripe_output[first..][..4].fill(previous);
                    continue;
                }
                if first >= width
                    && group
                        .iter()
                        .zip(&stripe_samples[first - width..])
                        .all(|(pixel, above)| same(pixel, above))
                {
                    stripe_output.copy_within(first - width..first - width + 4, first);
                    continue;
                }
                let output = &mut stripe_output[first..][..4];
                if let Some(four) = codes.four(group) {
                    output.copy_from_slice(&four);
                    continue;
                }
                for (pixel, output) in group.iter().zip(output) {
                    *output = codes.pixel(*pixel)?;
                }
            }
            let tail = groups.len() * 4;
            for (pixel, output) in samples_tail.iter().zip(&mut stripe_output[tail..]) {
                *output = codes.pixel(*pixel)?;
            }
        }
        #[cfg(not(all(target_arch = "aarch64", target_endian = "little")))]
        for (pixel, output) in stripe_samples.iter().zip(stripe_output) {
            let key = pixel.map(f32::to_bits);
            if let Some((prior, codes)) = previous
                && prior == key
            {
                *output = codes;
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
                *output = codes;
                previous = Some((key, codes));
                continue;
            }
            let pixel_codes = codes.pixel(*pixel)?;
            *output = pixel_codes;
            previous = Some((key, pixel_codes));
            cache[slot] = Some((key, pixel_codes));
        }
    }
    Ok(Some(row_bytes))
}

/// How the SDR path turns a pixel of its float readback, unpremultiplied in linear output
/// primaries, into 8-bit codes.
struct SdrCodes {
    maximum: f32,
    curve: Option<&'static [f32]>,
    primaries: ColorPrimaries,
    gamut_map: GamutMap,
    alpha_mode: OutputAlphaMode,
    swap_red_blue: bool,
}

impl SdrCodes {
    fn new(spec: OutputSpec, color_type: ColorType) -> Self {
        Self {
            maximum: f32::from(spec.reference_white().get()) / 100.0,
            curve: transfer_lut(spec.target().transfer),
            primaries: spec.target().primaries,
            gamut_map: spec.gamut_map(),
            alpha_mode: spec.alpha(),
            swap_red_blue: color_type == ColorType::BGRA8888,
        }
    }

    /// The codes of `pixel`, or why the output cannot hold it.
    fn pixel(&self, pixel: [f32; 4]) -> Result<[u8; 4], DrawError> {
        let pixel = canonical_rgba(pixel)?;
        if self.alpha_mode == OutputAlphaMode::Opaque && pixel[3] != 1.0 {
            return Err(DrawError::Surface("opaque output has transparency".into()));
        }
        let mapped = valle_engine::compositor::delivery::map_output_gamut(
            [pixel[0], pixel[1], pixel[2]],
            self.primaries,
            self.maximum,
            self.gamut_map,
        )
        .map_err(|error| DrawError::Surface(error.to_string()))?;
        let coverage = if self.alpha_mode == OutputAlphaMode::PremultipliedCoverage {
            pixel[3]
        } else {
            1.0
        };
        let code =
            |value: f32| round_code(transfer_code(value / self.maximum, self.curve) * coverage);
        let (red, blue) = if self.swap_red_blue { (2, 0) } else { (0, 2) };
        Ok([
            code(mapped[red]),
            code(mapped[1]),
            code(mapped[blue]),
            round_code(pixel[3] * 255.0),
        ])
    }

    /// The codes of four pixels, lane by lane the operations of [`SdrCodes::pixel`], or `None`
    /// where one of them leaves its common path: a channel that is not finite, alpha outside
    /// [0, 1] or transparency in opaque output, or a peak the gamut map rejects.
    #[cfg(all(target_arch = "aarch64", target_endian = "little"))]
    #[inline(always)]
    fn four(&self, pixels: &[[f32; 4]; 4]) -> Option<[[u8; 4]; 4]> {
        use std::arch::aarch64::*;
        if !(self.maximum > 0.0 && self.maximum.is_finite()) {
            return None;
        }
        // SAFETY: NEON is part of the aarch64 baseline; the loads read the sixteen channels of the
        // four pixels and two adjacent entries of the curve, and the store writes sixteen codes.
        unsafe {
            let float32x4x4_t(red, green, blue, alpha) = vld4q_f32(pixels.as_ptr().cast::<f32>());
            let zero = vdupq_n_f32(0.0);
            let one = vdupq_n_f32(1.0);
            let all = |mask: uint32x4_t| vminvq_u32(mask) != 0;
            // `canonical_rgba`: every channel finite, then alpha within an epsilon of 0 clears the
            // pixel and within one of 1 becomes 1, and must then lie in [0, 1].
            let infinity = vdupq_n_f32(f32::INFINITY);
            let finite = |value: float32x4_t| vcltq_f32(vabsq_f32(value), infinity);
            if !all(vandq_u32(
                vandq_u32(finite(red), finite(green)),
                vandq_u32(finite(blue), finite(alpha)),
            )) {
                return None;
            }
            let epsilon = vdupq_n_f32(f32::EPSILON);
            let clear = vcleq_f32(vabsq_f32(alpha), epsilon);
            let alpha = vbslq_f32(
                vcleq_f32(vabsq_f32(vsubq_f32(alpha, one)), epsilon),
                one,
                alpha,
            );
            let keep = |value: float32x4_t| vbslq_f32(clear, zero, value);
            let (red, green, blue, alpha) = (keep(red), keep(green), keep(blue), keep(alpha));
            if !all(vandq_u32(vcleq_f32(zero, alpha), vcleq_f32(alpha, one)))
                || (self.alpha_mode == OutputAlphaMode::Opaque && !all(vceqq_f32(alpha, one)))
            {
                return None;
            }
            // `map_output_gamut`. The clamps keep a negative zero, as `f32::clamp` does.
            let maximum = vdupq_n_f32(self.maximum);
            let clamp = |value: float32x4_t| {
                let value = vbslq_f32(vcltq_f32(value, zero), zero, value);
                vbslq_f32(vcgtq_f32(value, maximum), maximum, value)
            };
            let (red, green, blue) = match self.gamut_map {
                GamutMap::Clip => (clamp(red), clamp(green), clamp(blue)),
                GamutMap::ChromaCompress => {
                    let inside = |value: float32x4_t| {
                        vandq_u32(vcleq_f32(zero, value), vcleq_f32(value, maximum))
                    };
                    let unchanged = vandq_u32(vandq_u32(inside(red), inside(green)), inside(blue));
                    if all(unchanged) {
                        (red, green, blue)
                    } else {
                        let [r, g, b] =
                            valle_engine::compositor::delivery::output_luminance_weights(
                                self.primaries,
                            );
                        let luminance = clamp(vaddq_f32(
                            vaddq_f32(vmulq_n_f32(red, r), vmulq_n_f32(green, g)),
                            vmulq_n_f32(blue, b),
                        ));
                        let narrow = |scale: float32x4_t, channel: float32x4_t| {
                            let delta = vsubq_f32(channel, luminance);
                            let above =
                                vminnmq_f32(scale, vdivq_f32(vsubq_f32(maximum, luminance), delta));
                            let below = vminnmq_f32(scale, vdivq_f32(luminance, vnegq_f32(delta)));
                            vbslq_f32(
                                vcgtq_f32(delta, zero),
                                above,
                                vbslq_f32(vcltq_f32(delta, zero), below, scale),
                            )
                        };
                        let scale = narrow(narrow(narrow(one, red), green), blue);
                        let compress = |channel: float32x4_t| {
                            let compressed = vaddq_f32(
                                luminance,
                                vmulq_f32(vsubq_f32(channel, luminance), scale),
                            );
                            vbslq_f32(unchanged, channel, clamp(compressed))
                        };
                        (compress(red), compress(green), compress(blue))
                    }
                }
            };
            // `round_code` of `transfer_code` scaled by the coverage. Dividing by a peak of exactly
            // one, the default 100-nit white, and scaling by a coverage of one return the value.
            let unit = self.maximum == 1.0;
            let coverage =
                (self.alpha_mode == OutputAlphaMode::PremultipliedCoverage).then_some(alpha);
            let round = |value: float32x4_t| {
                let truncated = vcvtq_u32_f32(value);
                let up = vcgeq_f32(vsubq_f32(value, vcvtq_f32_u32(truncated)), vdupq_n_f32(0.5));
                vminq_u32(vsubq_u32(truncated, up), vdupq_n_u32(255))
            };
            let code = |value: float32x4_t| {
                let value = if unit {
                    value
                } else {
                    vdivq_f32(value, maximum)
                };
                let encoded = match self.curve {
                    None => vmulq_n_f32(value, 255.0),
                    Some(curve) => {
                        let position = vmulq_n_f32(value, 4096.0);
                        let index = vminq_u32(vcvtq_u32_f32(position), vdupq_n_u32(4095));
                        // Each lane's entry and the next one, gathered in pairs.
                        let pair = |index: u32| vld1_f32(curve[index as usize..][..2].as_ptr());
                        let front = vcombine_f32(
                            pair(vgetq_lane_u32::<0>(index)),
                            pair(vgetq_lane_u32::<1>(index)),
                        );
                        let back = vcombine_f32(
                            pair(vgetq_lane_u32::<2>(index)),
                            pair(vgetq_lane_u32::<3>(index)),
                        );
                        let (low, high) = (vuzp1q_f32(front, back), vuzp2q_f32(front, back));
                        vaddq_f32(
                            low,
                            vmulq_f32(
                                vsubq_f32(high, low),
                                vsubq_f32(position, vcvtq_f32_u32(index)),
                            ),
                        )
                    }
                };
                round(match coverage {
                    Some(alpha) => vmulq_f32(encoded, alpha),
                    None => encoded,
                })
            };
            let (first, third) = if self.swap_red_blue {
                (blue, red)
            } else {
                (red, blue)
            };
            let word = vorrq_u32(
                vorrq_u32(code(first), vshlq_n_u32::<8>(code(green))),
                vorrq_u32(
                    vshlq_n_u32::<16>(code(third)),
                    vshlq_n_u32::<24>(round(vmulq_n_f32(alpha, 255.0))),
                ),
            );
            let mut codes = [[0_u8; 4]; 4];
            vst1q_u8(codes.as_mut_ptr().cast::<u8>(), vreinterpretq_u8_u32(word));
            Some(codes)
        }
    }
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
// HDR and higher bit depths retain the exact delivery implementation. CPU SDR dither is applied
// after this shared curve, at the original canvas coordinates.
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

    fn sdr_spec(
        primaries: ColorPrimaries,
        transfer: TransferFunction,
        alpha: OutputAlphaMode,
        gamut: GamutMap,
        white: u16,
    ) -> OutputSpec {
        use valle_engine::resource::LuminanceNits;
        let nits = LuminanceNits::new(white).unwrap();
        OutputSpec::new(
            OutputColorEncoding {
                primaries,
                transfer,
            },
            alpha,
            if alpha == OutputAlphaMode::Opaque {
                OutputBackground::opaque_srgb([0, 0, 0])
            } else {
                OutputBackground::Transparent
            },
            ToneMap::None,
            gamut,
            Dither::None,
            OutputBitDepth::Eight,
            SignalLuminance::new(nits, nits).unwrap(),
        )
        .unwrap()
    }

    #[cfg(all(target_arch = "aarch64", target_endian = "little"))]
    #[test]
    fn sdr_codes_of_four_pixels_match_one_at_a_time() {
        // Channels inside and past [0, peak], signed zeros, a subnormal, values on the curve's
        // knots, near its joins and on a halfway code; alpha that clears, snaps to one, lies
        // inside or outside [0, 1]; channels that are not finite.
        let channels = [
            0.0,
            -0.0,
            1.0e-40,
            0.25,
            0.5,
            1.0,
            1.5,
            2.03,
            3.0,
            -0.2,
            -1.0e-3,
            0.999_999,
            1.0 / 4096.0,
            0.003_130_8,
            0.018,
            127.5 / 255.0,
            60.0,
        ];
        let alphas = [
            1.0,
            1.0 - f32::EPSILON / 2.0,
            1.0 + f32::EPSILON,
            0.0,
            1.0e-8,
            f32::EPSILON,
            0.5,
            0.25,
            0.75,
            1.0 - 2.0 * f32::EPSILON,
        ];
        let broken = [
            [f32::NAN, 0.5, 0.5, 1.0],
            [0.5, f32::INFINITY, 0.5, 1.0],
            [0.5, 0.5, f32::NEG_INFINITY, 0.5],
            [0.5, 0.5, 0.5, f32::NAN],
            [0.5, 0.5, 0.5, 1.5],
            [0.5, 0.5, 0.5, -0.1],
        ];
        let mut state = 0x9e37_79b9_u32;
        let mut next = move |count: usize| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state as usize % count
        };
        let mut groups = 0;
        let mut four = 0;
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
                            for color_type in [ColorType::RGBA8888, ColorType::BGRA8888] {
                                let spec = sdr_spec(primaries, transfer, alpha, gamut, white);
                                let codes = SdrCodes::new(spec, color_type);
                                // Opaque output keeps only alpha that becomes one.
                                let kept = if alpha == OutputAlphaMode::Opaque {
                                    3
                                } else {
                                    alphas.len()
                                };
                                for _ in 0..512 {
                                    let group: [[f32; 4]; 4] = std::array::from_fn(|_| {
                                        if next(16) == 0 {
                                            broken[next(broken.len())]
                                        } else {
                                            [
                                                channels[next(channels.len())],
                                                channels[next(channels.len())],
                                                channels[next(channels.len())],
                                                alphas[next(kept)],
                                            ]
                                        }
                                    });
                                    let one_at_a_time = group.map(|pixel| codes.pixel(pixel).ok());
                                    let expected = one_at_a_time
                                        .iter()
                                        .all(Option::is_some)
                                        .then(|| one_at_a_time.map(Option::unwrap));
                                    assert_eq!(
                                        codes.four(&group),
                                        expected,
                                        "{spec:?} {color_type:?} {group:?}"
                                    );
                                    groups += 1;
                                    four += usize::from(expected.is_some());
                                }
                            }
                        }
                    }
                }
            }
        }
        // Most groups take the four-pixel path.
        assert!(four * 2 > groups, "{four} of {groups}");
    }

    #[test]
    fn sdr_output_stages_the_codes_of_each_pixel_of_its_readback() {
        // A width that is not a multiple of four, so that groups straddle rows and the stripe
        // ends in a partial group.
        let (width, height) = (203, 37);
        let frame = |broken: Option<(usize, [f32; 4])>| {
            let mut samples = (0..width * height)
                .map(|i| {
                    // Pixels that differ from their neighbors, then runs of equal pixels along
                    // each row, then rows that repeat the one above.
                    let i = match i / width {
                        0..12 => i,
                        12..24 => i / 7 * 7,
                        _ => 24 * width + i % width,
                    };
                    let a = [1.0, 0.5, 0.0, 0.75][i % 4 / 3 + i / width % 2 * 2];
                    [
                        ((i % 41) as f32 / 20.0 - 0.4) * a,
                        (i % 23) as f32 / 15.0 * a,
                        ((i / width) as f32 / 14.0 - 0.2) * a,
                        a,
                    ]
                })
                .collect::<Vec<_>>();
            if let Some((index, pixel)) = broken {
                samples[index] = pixel;
            }
            let bytes = samples
                .iter()
                .flatten()
                .flat_map(|value| value.to_ne_bytes())
                .collect::<Vec<_>>();
            let info = ImageInfo::new(
                (width as i32, height as i32),
                ColorType::RGBAF32,
                AlphaType::Premul,
                Some(working_color_space().unwrap()),
            );
            images::raster_from_data(&info, Data::new_copy(&bytes), width * 16).unwrap()
        };
        for (alpha, gamut, transfer, white) in [
            (
                OutputAlphaMode::StraightCoverage,
                GamutMap::ChromaCompress,
                TransferFunction::Srgb,
                100,
            ),
            (
                OutputAlphaMode::PremultipliedCoverage,
                GamutMap::Clip,
                TransferFunction::Rec709,
                203,
            ),
            (
                OutputAlphaMode::StraightCoverage,
                GamutMap::ChromaCompress,
                TransferFunction::Linear,
                203,
            ),
        ] {
            let spec = sdr_spec(ColorPrimaries::DisplayP3, transfer, alpha, gamut, white);
            let target = rgba8_target_info(
                spec,
                valle_engine::resource::Extent2d::new(width as u32, height as u32).unwrap(),
            )
            .unwrap();
            let codes = SdrCodes::new(spec, target.color_type());
            // The float readback the staging converts, read in one piece.
            let readback = |image: &Image| {
                let linear = ColorSpace::new_cicp(
                    named_primaries::CicpId::SMPTE_EG_432_1,
                    named_transfer_fn::CicpId::Linear,
                )
                .unwrap();
                let info = ImageInfo::new(
                    image.dimensions(),
                    ColorType::RGBAF32,
                    AlphaType::Unpremul,
                    Some(linear),
                );
                let mut samples = vec![0.0_f32; width * height * 4];
                assert!(image.read_pixels(
                    &info,
                    samples.as_mut_slice(),
                    width * 16,
                    (0, 0),
                    CachingHint::Disallow,
                ));
                samples
            };
            let image = frame(None);
            let expected = readback(&image)
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|pixel| codes.pixel(*pixel).unwrap())
                .collect::<Vec<_>>();
            let mut staging = OutputStaging::default();
            assert_eq!(
                stage_sdr_output_into(&image, spec, &target, &mut staging).unwrap(),
                Some(target.min_row_bytes())
            );
            assert!(staging.pixels() == expected, "{spec:?}");
            // A failing pixel among differing neighbors, inside a run, in a row that otherwise
            // repeats the one above, and in the stripe's last partial group.
            for index in [
                5 * width + 9,
                15 * width + 15,
                30 * width + 101,
                width * height - 2,
            ] {
                let image = frame(Some((index, [0.5, f32::NAN, 0.5, 1.0])));
                let expected = readback(&image)
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .find_map(|pixel| codes.pixel(*pixel).err())
                    .unwrap()
                    .to_string();
                let actual = stage_sdr_output_into(&image, spec, &target, &mut staging)
                    .unwrap_err()
                    .to_string();
                assert_eq!(actual, expected);
            }
        }
    }

    #[test]
    fn continuous_color_frames_bypass_color_memoization() {
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
        // Cross both stripe boundaries and a final one-row stripe, including position-dependent
        // dithering. Every pixel is compared with the independent per-pixel delivery reference.
        let samples = (0..32 * 129)
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
            (32, 129),
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
                    (32, 129),
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
    fn half_float_frames_stage_like_their_float_readback() {
        use valle_engine::compositor::bloom::f32_to_half as half;
        // Coverage output keeps partial and zero alpha; 10-bit packed targets are opaque only.
        let formats = [
            (OutputBitDepth::Eight, ColorType::RGBA8888),
            (OutputBitDepth::Sixteen, ColorType::R16G16B16A16UNorm),
            (OutputBitDepth::Float16, ColorType::RGBAF16),
        ];
        // Runs and varied values for both caches, plus half subnormals, signed zero, values above
        // one, partial alpha and transparency.
        let halves = (0..32 * 67)
            .map(|i| match i % 11 {
                0 => [0x0001, 0x03ff, 0x8001, 0x3c00],
                1 => [0x8000, 0x0000, 0x0200, 0x3c00],
                2 => [0; 4],
                3 => [half(0.2), half(0.1), half(0.05), half(0.5)],
                4 => [half(1.5), half(-0.25), half(0.75), 0x3c00],
                _ if i / 7 % 2 == 0 => [half(0.18), half(0.18), half(0.18), 0x3c00],
                _ => [
                    half((i % 13) as f32 / 12.0),
                    half((i % 5) as f32 / 4.0),
                    half(0.5),
                    0x3c00,
                ],
            })
            .collect::<Vec<[u16; 4]>>();
        // Padded rows check that staging follows the frame's row stride.
        let row_bytes = 32 * 8 + 24;
        let mut data = vec![0_u8; row_bytes * 67];
        for (index, pixel) in halves.iter().enumerate() {
            let offset = index / 32 * row_bytes + index % 32 * 8;
            for (channel, bits) in pixel.iter().enumerate() {
                data[offset + channel * 2..offset + channel * 2 + 2]
                    .copy_from_slice(&bits.to_ne_bytes());
            }
        }
        let source = ImageInfo::new(
            (32, 67),
            ColorType::RGBAF16,
            AlphaType::Premul,
            Some(working_color_space().unwrap()),
        );
        let image = images::raster_from_data(&source, Data::new_copy(&data), row_bytes).unwrap();
        let mut floats = vec![0.0_f32; 32 * 67 * 4];
        assert!(image.read_pixels(
            &source.with_color_type(ColorType::RGBAF32),
            floats.as_mut_slice(),
            32 * 16,
            (0, 0),
            CachingHint::Disallow,
        ));
        // The direct path relies on this: Skia's aarch64 readback widens halves exactly.
        #[cfg(target_arch = "aarch64")]
        assert!(
            floats
                .iter()
                .zip(halves.iter().flatten())
                .all(|(value, bits)| {
                    value.to_bits() == valle_engine::compositor::bloom::half_to_f32(*bits).to_bits()
                })
        );
        let mut checked = 0;
        for (bit_depth, color_type) in formats {
            for dither in [Dither::None, Dither::Triangular { seed: 9 }] {
                let Ok(spec) = OutputSpec::new(
                    OutputColorEncoding::SRGB,
                    OutputAlphaMode::PremultipliedCoverage,
                    OutputBackground::Transparent,
                    ToneMap::None,
                    GamutMap::ChromaCompress,
                    dither,
                    bit_depth,
                    SignalLuminance::SDR_100,
                ) else {
                    continue;
                };
                let target = ImageInfo::new(
                    (32, 67),
                    color_type,
                    AlphaType::Premul,
                    Some(output_color_space(spec).unwrap()),
                );
                let actual = stage_output(&image, spec, &target).unwrap();
                let mut expected = Vec::new();
                for (index, pixel) in floats.as_chunks::<4>().0.iter().enumerate() {
                    let sample = transform_working_pixel(
                        canonical_rgba(*pixel).unwrap(),
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
        assert_eq!(checked, formats.len() * 2 - 1);
    }

    #[test]
    fn staging_reuse_overwrites_pixels_across_sizes_formats_and_output_paths() {
        let mut staging = OutputStaging::default();
        // Grow and shrink the same buffers, switch both delivery paths, then reuse a prior size.
        for (width, height, bit_depth, color_type, transfer) in [
            (
                32,
                133,
                OutputBitDepth::Eight,
                ColorType::RGBA8888,
                TransferFunction::Srgb,
            ),
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
            // A tall frame must not retain its full float plane. Later cases poison and reuse
            // this same storage across changing sizes and output formats.
            assert!(staging.working.capacity() <= 64 * OUTPUT_STRIPE_ROWS * 4);
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
