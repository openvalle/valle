//! Velocity blur: copies of the input along its trail, averaged. Skia's image filter defines it
//! on every backend. Raster uses a byte-exact NEON kernel; GPU uses one shader draw where
//! the filter's sampling coverage is reproducible without its intermediate surfaces.

use skia_safe::{
    AlphaType, BlendMode, ColorType, Data, IRect, Image, Paint, RuntimeEffect, SamplingOptions,
    Surface, TileMode, runtime_effect::ChildPtr,
};
use valle_draw::program::Filter;

use super::draw::DrawError;

/// The offset of each copy along the trail from `from` to `to`, or `None` where the blur leaves
/// its input unchanged.
pub(super) fn offsets(from: [f32; 2], to: [f32; 2]) -> Option<Vec<(f32, f32)>> {
    Some(
        Filter::velocity_blur_offsets(from, to)?
            .into_iter()
            .map(|[x, y]| (x, y))
            .collect(),
    )
}

/// The row-major color matrix that weights each of `samples` positions: its alpha by one share.
pub(super) fn weight(samples: usize) -> [f32; 20] {
    let weight = 1.0 / samples as f32;
    [
        1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        weight, 0.0,
    ]
}

/// A single GPU draw over the scheduled device-space pass. Raster keeps its byte-exact NEON
/// kernel; the original filter tree remains available for unsupported surfaces and comparisons.
pub(super) fn draw_gpu(
    surface: &mut Surface,
    input: &Image,
    source: IRect,
    origin: [i32; 2],
    from: [f32; 2],
    to: [f32; 2],
) -> Result<bool, DrawError> {
    let info = surface.image_info();
    if surface.canvas().peek_pixels().is_some()
        || !surface.canvas().local_to_device_as_3x3().is_identity()
        || info.color_type() != ColorType::RGBAF16
        || info.alpha_type() != AlphaType::Premul
        || input.color_type() != ColorType::RGBAF16
        || input.alpha_type() != AlphaType::Premul
        || input.color_space() != info.color_space()
        || source.is_empty()
        || source.left != 0
        || source.top != 0
        || source.right > input.width()
        || source.bottom > input.height()
        || std::env::var_os("VALLE_VELOCITY_BLUR_FILTER_REFERENCE").is_some()
    {
        return Ok(false);
    }
    let Some(offsets) = offsets(from, to) else {
        return Ok(false);
    };
    let Some(clip) = surface.canvas().device_clip_bounds() else {
        return Ok(false);
    };
    // The filter tree can rescale oversized inputs. GPU texture normalization also makes an
    // exact half-texel tie depend on its intermediate surface's coordinates. Keep that path
    // where a single nearest lookup cannot reproduce its coverage reliably.
    let largest = (2 * clip.width().max(clip.height())).max(2048);
    let coordinate = (origin[0] as f32).abs() + source.width() as f32;
    let coordinate = coordinate
        .max((origin[1] as f32).abs() + source.height() as f32)
        .max(
            clip.left
                .abs()
                .max(clip.right.abs())
                .max(clip.top.abs())
                .max(clip.bottom.abs()) as f32,
        );
    let tie_margin = (coordinate * (4.0 * f32::EPSILON)).max(0.001);
    if source.width() > largest
        || source.height() > largest
        || origin.iter().any(|&v| v.unsigned_abs() >= 1_048_576)
        || offsets
            .iter()
            .flat_map(|&(x, y)| [x, y])
            .any(|v| (v - v.floor() - 0.5).abs() <= tie_margin)
    {
        return Ok(false);
    }
    thread_local! {
        static EFFECT: Result<RuntimeEffect, String> = RuntimeEffect::make_for_shader(
            include_str!("../../../../valle-draw/assets/shaders/velocityblur.sksl"), None,
        );
    }
    EFFECT.with(|effect| {
        let effect = effect
            .as_ref()
            .map_err(|message| DrawError::ShaderCompile {
                uri: "builtin://velocity-blur".into(),
                message: message.clone(),
            })?;
        let mut uniforms = vec![
            source.width() as f32,
            source.height() as f32,
            origin[0] as f32,
            origin[1] as f32,
            offsets.len() as f32,
            0.0,
        ];
        let matrix = weight(offsets.len());
        uniforms.extend((0..4).flat_map(|column| (0..4).map(move |row| matrix[row * 5 + column])));
        uniforms.extend([0.0; 4]);
        uniforms.extend(offsets.iter().flat_map(|&(x, y)| [x, y]));
        uniforms.resize(26 + 32 * 2, 0.0);
        let bytes = uniforms
            .iter()
            .flat_map(|v| v.to_ne_bytes())
            .collect::<Vec<_>>();
        if bytes.len() != effect.uniform_size() {
            return Err(DrawError::Internal(
                "GPU velocity blur shader ABI mismatch".into(),
            ));
        }
        let image = input
            .to_raw_shader(
                (TileMode::Clamp, TileMode::Clamp),
                SamplingOptions::default(),
                None,
            )
            .ok_or_else(|| DrawError::Unsupported("GPU velocity blur input".into()))?;
        let shader = effect
            .make_shader(Data::new_copy(&bytes), &[ChildPtr::Shader(image)], None)
            .ok_or_else(|| DrawError::Unsupported("GPU velocity blur shader".into()))?;
        let mut paint = Paint::default();
        paint.set_blend_mode(BlendMode::Src);
        paint.set_shader(shader);
        surface.canvas().draw_paint(&paint);
        if std::env::var_os("VALLE_TRACE_F16_STAGES").is_some() {
            eprintln!(
                "[valle f16] gpu-velocity-blur {} taps (no intermediate readback)",
                offsets.len()
            );
        }
        Ok(true)
    })
}

/// Draws the velocity blur directly into the scheduled clip, retaining intermediate F16
/// rounding. Returns false without drawing where Skia's original filter tree is needed.
pub(super) fn draw(
    surface: &mut Surface,
    input: &Image,
    source: IRect,
    origin: [i32; 2],
    from: [f32; 2],
    to: [f32; 2],
) -> Result<bool, DrawError> {
    if draw_gpu(surface, input, source, origin, from, to)? {
        return Ok(true);
    }
    #[cfg(all(target_arch = "aarch64", target_endian = "little"))]
    {
        fused::draw(surface, input, source, origin, from, to)
    }
    #[cfg(not(all(target_arch = "aarch64", target_endian = "little")))]
    {
        Ok(false)
    }
}

#[cfg(all(target_arch = "aarch64", target_endian = "little"))]
mod fused {
    use std::arch::aarch64::*;

    use skia_safe::{
        AlphaType, ColorType, IRect, Image, ImageInfo, Surface, surface::ContentChangeMode,
    };
    use valle_engine::compositor::bloom::fill_image_rows;

    use super::{
        super::draw::{DrawError, rows_clip},
        offsets, weight,
    };

    /// Pixels per vector.
    const LANES: usize = 4;

    pub(super) fn draw(
        surface: &mut Surface,
        input: &Image,
        source: IRect,
        origin: [i32; 2],
        from: [f32; 2],
        to: [f32; 2],
    ) -> Result<bool, DrawError> {
        let Some(offsets) = offsets(from, to) else {
            return Ok(false);
        };
        let (Some(clip), Some(pixels)) = (rows_clip(surface), input.peek_pixels()) else {
            return Ok(false);
        };
        let target = surface.image_info();
        let half = |info: &ImageInfo| {
            info.color_type() == ColorType::RGBAF16 && info.alpha_type() == AlphaType::Premul
        };
        if !half(&target)
            || !half(pixels.info())
            || pixels.info().color_space() != target.color_space()
            || source.is_empty()
            || source.left != 0
            || source.top != 0
            || source.right > pixels.width()
            || source.bottom > pixels.height()
        {
            return Ok(false);
        }
        let placed = IRect::from_xywh(origin[0], origin[1], source.width(), source.height());
        let Some(taps) = Taps::new(clip, placed, &offsets) else {
            return Ok(false);
        };
        let input = Input {
            bytes: pixels
                .bytes()
                .ok_or_else(|| DrawError::Surface("Raster image has no pixels".into()))?,
            row_bytes: pixels.row_bytes(),
            width: source.width(),
        };
        // As for a Src draw: pixels a snapshot still shares are replaced before they are written,
        // by a copy unless the clip is the whole surface.
        let whole = clip == IRect::from_wh(surface.width(), surface.height());
        surface.notify_content_will_change(if whole {
            ContentChangeMode::Discard
        } else {
            ContentChangeMode::Retain
        });
        let mut out = surface
            .peek_pixels()
            .ok_or_else(|| DrawError::Surface("Raster surface has no pixels".into()))?;
        let row_bytes = out.row_bytes();
        let bytes = out
            .bytes_mut()
            .ok_or_else(|| DrawError::Surface("Raster surface pixels are not writable".into()))?;
        let span =
            clip.top as usize * row_bytes..(clip.bottom as usize * row_bytes).min(bytes.len());
        // The pipeline reads the coefficients from memory and applies each by a fused
        // multiply-add, including the ones and zeros; keep them data here too.
        let matrix = std::hint::black_box(weight(offsets.len()));
        fill_image_rows(&mut bytes[span], row_bytes, |first, rows| {
            blur_rows(&taps, &input, &matrix, first, rows, row_bytes);
        });
        Ok(true)
    }

    /// Where each shutter position samples the input for the pixels of a clip.
    struct Taps {
        clip: IRect,
        /// Clip column `x` of position `j` reads input column `x + columns[j]`, transparent
        /// outside the input.
        columns: Vec<i32>,
        /// For each clip row in turn, the input row each position reads, or -1.
        rows: Vec<i32>,
        /// The input columns the positions read across the clip, past the input's edges too.
        reach: [i32; 2],
    }

    impl Taps {
        /// The samples of an input placed at `placed`, its copies translated by `offsets`, for
        /// the pixels of `clip`, or `None` where the filter's draw does not take them this way.
        fn new(clip: IRect, placed: IRect, offsets: &[(f32, f32)]) -> Option<Self> {
            let samples = offsets.len();
            // Skia scales a filter's layer down past this size.
            let largest = (2 * clip.width().max(clip.height())).max(2048);
            // Keep whole pixels and pixel centers exact in f32.
            let exact = |value: f32| value.abs() < 1_048_576.0;
            if placed.width() > largest
                || placed.height() > largest
                || ![placed.left, placed.top, placed.right, placed.bottom]
                    .iter()
                    .all(|&edge| exact(edge as f32))
                || !offsets.iter().all(|&(x, y)| exact(x) && exact(y))
            {
                return None;
            }
            let size = (placed.width(), placed.height());
            // The filter maps each copy's bounds in double precision, and places its image by the
            // copy's translation composed with the input's in f32.
            let bounds = offsets
                .iter()
                .map(|&offset| map_bounds(placed, offset))
                .collect::<Vec<_>>();
            let translations = offsets
                .iter()
                .map(|&(x, y)| (placed.left as f32 + x, placed.top as f32 + y))
                .collect::<Vec<_>>();
            for tap in 0..samples {
                let image = map_bounds(IRect::from_wh(size.0, size.1), translations[tap]);
                // It crops a copy whose bounds miss part of its image, and filters a copy
                // bilinearly where its offset is whole pixels within a thousandth but its
                // translation is not.
                if !contains(bounds[tap], image)
                    || (nearly_integer(offsets[tap]) && !nearly_integer(translations[tap]))
                {
                    return None;
                }
            }
            // Partial sum `i` adds copy `i` to the sum before it, over the union of the bounds
            // so far within the clip; the first adds copies 0 and 1.
            let mut areas = vec![None; samples];
            let mut union = bounds[0];
            for tap in 1..samples {
                union = join(union, bounds[tap]);
                areas[tap] = IRect::intersect(&union, &clip);
            }
            let mut columns = vec![0; samples];
            let mut rows = vec![-1; samples * clip.height() as usize];
            let mut reach = None::<[i32; 2]>;
            let mut read = vec![false; samples];
            for tap in 0..samples {
                let Some(area) = areas[tap.max(1)] else {
                    continue;
                };
                let (x, y) = translations[tap];
                let sampled = |column: i32| {
                    if (area.left..area.right).contains(&column) {
                        sample(column, area.left, x, size.0)
                    } else {
                        None
                    }
                };
                let Some(offset) =
                    (clip.left..clip.right).find_map(|column| Some(sampled(column)? - column))
                else {
                    continue;
                };
                // Every column reads the input column at that offset, and only those inside it.
                if (clip.left..clip.right).any(|column| {
                    sampled(column) != Some(column + offset).filter(|at| (0..size.0).contains(at))
                }) {
                    return None;
                }
                columns[tap] = offset;
                read[tap] = true;
                let [low, high] = reach.get_or_insert([clip.left + offset, clip.right + offset]);
                *low = (*low).min(clip.left + offset);
                *high = (*high).max(clip.right + offset);
                for (row, sources) in (clip.top..clip.bottom).zip(rows.chunks_mut(samples)) {
                    if (area.top..area.bottom).contains(&row)
                        && let Some(at) = sample(row, area.top, y, size.1)
                    {
                        sources[tap] = at;
                    }
                }
            }
            let reach = reach.unwrap_or([clip.left, clip.right]);
            // Bound the weighed rows, which span the copies' spread past the clip.
            if reach[1] - reach[0] > 1 << 14 {
                return None;
            }
            // Positions that read nothing read transparent planes from their start.
            for tap in 0..samples {
                if !read[tap] {
                    columns[tap] = reach[0] - clip.left;
                }
            }
            Some(Self {
                clip,
                columns,
                rows,
                reach,
            })
        }

        /// Floats in each channel plane of a weighed row: the reach and a vector past it.
        fn plane(&self) -> usize {
            (self.reach[1] - self.reach[0]) as usize + LANES
        }
    }

    /// The input pixel Skia's nearest-neighbor sampling reads at the center of `pixel`, for an
    /// image translated by `translation` and drawn into a surface whose first pixel is the one
    /// before `start`, or `None` where the decal reads transparent. The canvas translation and
    /// the image's compose and invert in f32 before they meet the pixel center.
    fn sample(pixel: i32, start: i32, translation: f32, extent: i32) -> Option<i32> {
        let canvas = (1 - start) as f32;
        let at = ((pixel + 1 - start) as f32 + 0.5) + -(translation + canvas);
        let limit = extent as f32;
        if !((0.0 < at && at < limit) || at == limit) {
            return None;
        }
        // Clamped inside the image and then one ulp down, so a center on a pixel edge reads the
        // pixel before it.
        let inside = at
            .max(f32::MIN_POSITIVE)
            .min(f32::from_bits(limit.to_bits() - 1));
        Some(f32::from_bits(inside.to_bits() - 1) as i32)
    }

    /// Skia maps whole-pixel bounds through a translation in double precision and rounds them
    /// out, ignoring a thousandth of a pixel.
    fn map_bounds(rect: IRect, (x, y): (f32, f32)) -> IRect {
        let epsilon = f64::from(1.0e-3_f32);
        let low = |edge: i32, by: f32| (f64::from(edge) + f64::from(by) + epsilon).floor() as i32;
        let high = |edge: i32, by: f32| (f64::from(edge) + f64::from(by) - epsilon).ceil() as i32;
        IRect::from_ltrb(
            low(rect.left, x),
            low(rect.top, y),
            high(rect.right, x),
            high(rect.bottom, y),
        )
    }

    /// Skia's translation test: each axis whole pixels within a thousandth.
    fn nearly_integer((x, y): (f32, f32)) -> bool {
        let near = |value: f32| ((f64::from(value) + 0.5).floor() as f32 - value).abs() <= 1.0e-3;
        near(x) && near(y)
    }

    /// `SkIRect::contains`, which an empty rect fails.
    fn contains(outer: IRect, inner: IRect) -> bool {
        !inner.is_empty()
            && !outer.is_empty()
            && outer.left <= inner.left
            && outer.top <= inner.top
            && outer.right >= inner.right
            && outer.bottom >= inner.bottom
    }

    /// `SkIRect::join`, which skips an empty rect.
    fn join(a: IRect, b: IRect) -> IRect {
        if b.is_empty() {
            return a;
        }
        if a.is_empty() {
            return b;
        }
        IRect::from_ltrb(
            a.left.min(b.left),
            a.top.min(b.top),
            a.right.max(b.right),
            a.bottom.max(b.bottom),
        )
    }

    /// The input's half-float pixels.
    struct Input<'a> {
        bytes: &'a [u8],
        row_bytes: usize,
        width: i32,
    }

    /// Writes clip rows `first..` of the blur into `rows`, the target's bytes from the first of
    /// them. Each input row a position reads is weighed once into a slot of channel planes and
    /// kept while the next clip rows read it; a last slot stays transparent.
    fn blur_rows(
        taps: &Taps,
        input: &Input<'_>,
        matrix: &[f32; 20],
        first: usize,
        rows: &mut [u8],
        row_bytes: usize,
    ) {
        let samples = taps.columns.len();
        let plane = taps.plane();
        let size = 4 * plane;
        let [low, high] = taps.reach;
        // Columns past the input's edges stay transparent in every slot.
        let weighed = [low.max(0), high.min(input.width)];
        // SAFETY: NEON is part of the aarch64 baseline.
        let matrix = matrix.map(|value| unsafe { vdupq_n_f32(value) });
        let mut slots = vec![0.0; (samples + 1) * size];
        let mut held = vec![-1; samples];
        let mut starts = vec![0; samples];
        let offsets = taps
            .columns
            .iter()
            .map(|offset| (taps.clip.left + offset - low) as usize)
            .collect::<Vec<_>>();
        let columns = taps.clip.left as usize * 8..taps.clip.right as usize * 8;
        for (sources, row) in taps.rows[first * samples..]
            .chunks(samples)
            .zip(rows.chunks_mut(row_bytes))
        {
            for &source in sources.iter().filter(|&&source| source >= 0) {
                if !held.contains(&source) {
                    // A slot no position of this row reads.
                    let slot = held
                        .iter()
                        .position(|held| *held < 0 || !sources.contains(held))
                        .expect("a slot for each position");
                    held[slot] = source;
                    weigh(
                        input,
                        source,
                        weighed,
                        low,
                        &matrix,
                        &mut slots[slot * size..][..size],
                        plane,
                    );
                }
            }
            for (start, &source) in starts.iter_mut().zip(sources) {
                let slot = held
                    .iter()
                    .position(|&slot| source >= 0 && slot == source)
                    .unwrap_or(samples);
                *start = slot * size;
            }
            blur_row(&slots, &starts, &offsets, plane, &mut row[columns.clone()]);
        }
    }

    /// Weighs input columns `[start, end)` of input row `row` into channel planes `plane` floats
    /// apart, indexed from input column `low`.
    fn weigh(
        input: &Input<'_>,
        row: i32,
        [start, end]: [i32; 2],
        low: i32,
        matrix: &[float32x4_t; 20],
        planes: &mut [f32],
        plane: usize,
    ) {
        let pixels = &input.bytes[row as usize * input.row_bytes..];
        let mut column = start;
        while column < end {
            let count = (end - column).min(LANES as i32) as usize;
            let block = &pixels[column as usize * 8..][..count * 8];
            // SAFETY: NEON is part of the aarch64 baseline; a whole block loads its four pixels'
            // 32 bytes, and a short one is copied out first.
            let channels = unsafe {
                let halves = if count == LANES {
                    vld4_u16(block.as_ptr().cast())
                } else {
                    let mut padded = [0_u16; 4 * LANES];
                    for (half, bytes) in padded.iter_mut().zip(block.as_chunks::<2>().0) {
                        *half = u16::from_le_bytes(*bytes);
                    }
                    vld4_u16(padded.as_ptr())
                };
                weighted(
                    [halves.0, halves.1, halves.2, halves.3]
                        .map(|channel| vcvt_f32_f16(vreinterpret_f16_u16(channel))),
                    matrix,
                )
            };
            // A short block's missing pixels weigh to transparent, which the columns after it
            // are.
            let at = (column - low) as usize;
            for (channel, value) in channels.into_iter().enumerate() {
                let out = &mut planes[channel * plane + at..][..LANES];
                // SAFETY: the store writes the four floats of `out`.
                unsafe { vst1q_f32(out.as_mut_ptr(), value) };
            }
            column += LANES as i32;
        }
    }

    /// The color filter on four pixels, stage by stage as Skia's raster pipeline runs it:
    /// unpremultiply, the 4 x 5 matrix by fused multiply-adds from each row's constant inward,
    /// clamp to [0, 1], premultiply.
    #[inline(always)]
    fn weighted(
        [red, green, blue, alpha]: [float32x4_t; 4],
        matrix: &[float32x4_t; 20],
    ) -> [float32x4_t; 4] {
        // SAFETY: NEON is part of the aarch64 baseline.
        unsafe {
            let zero = vdupq_n_f32(0.0);
            let one = vdupq_n_f32(1.0);
            let inverse = vdivq_f32(one, alpha);
            let scale = vbslq_f32(
                vcltq_f32(inverse, vdupq_n_f32(f32::INFINITY)),
                inverse,
                zero,
            );
            let [red, green, blue] = [red, green, blue].map(|channel| vmulq_f32(channel, scale));
            let row = |at: usize| {
                let sum = vfmaq_f32(matrix[at + 4], alpha, matrix[at + 3]);
                let sum = vfmaq_f32(sum, blue, matrix[at + 2]);
                let sum = vfmaq_f32(sum, green, matrix[at + 1]);
                vminq_f32(vmaxq_f32(zero, vfmaq_f32(sum, red, matrix[at])), one)
            };
            let alpha = row(15);
            [
                vmulq_f32(row(0), alpha),
                vmulq_f32(row(5), alpha),
                vmulq_f32(row(10), alpha),
                alpha,
            ]
        }
    }

    /// Sums the positions of one clip row into its pixels `out`: position `j` reads the channel
    /// planes from `slots[starts[j]..]`, index `offsets[j] + i` for clip column `i`. Each partial
    /// sum is the Plus blend of the next copy over the sum before it, clamped to one and rounded
    /// to half floats as the surface that held it stored it.
    fn blur_row(slots: &[f32], starts: &[usize], offsets: &[usize], plane: usize, out: &mut [u8]) {
        let samples = starts.len();
        let width = out.len() / 8;
        let blocks = width.div_ceil(LANES);
        assert!(samples >= 2 && offsets.len() == samples);
        for (start, offset) in starts.iter().zip(offsets) {
            assert!(offset + blocks * LANES <= plane && start + 4 * plane <= slots.len());
        }
        // SAFETY: NEON is part of the aarch64 baseline.
        let one = unsafe { vdupq_n_f32(1.0) };
        for block in 0..blocks {
            let column = block * LANES;
            // SAFETY: the assertions above keep every block of every plane inside `slots`.
            let copy = |tap: usize| unsafe {
                load(
                    slots.as_ptr().add(starts[tap] + offsets[tap] + column),
                    plane,
                )
            };
            let first = copy(0);
            let mut sum = plus(copy(1), first, one).map(round);
            for tap in 2..samples - 1 {
                sum = plus(copy(tap), sum, one).map(round);
            }
            // SAFETY: NEON is part of the aarch64 baseline.
            let [red, green, blue, alpha] = plus(copy(samples - 1), sum, one)
                .map(|channel| unsafe { vreinterpret_u16_f16(vcvt_f16_f32(channel)) });
            let pixels = uint16x4x4_t(red, green, blue, alpha);
            let out = &mut out[column * 8..];
            if out.len() >= LANES * 8 {
                // SAFETY: the store writes the 32 bytes of four pixels of `out`.
                unsafe { vst4_u16(out.as_mut_ptr().cast(), pixels) };
            } else {
                let mut padded = [0_u16; 4 * LANES];
                // SAFETY: the store writes the sixteen halves of `padded`.
                unsafe { vst4_u16(padded.as_mut_ptr(), pixels) };
                for (bytes, half) in out.as_chunks_mut::<2>().0.iter_mut().zip(padded) {
                    *bytes = half.to_le_bytes();
                }
            }
        }
    }

    /// The four channels of four columns from planes `plane` floats apart, from `at`.
    ///
    /// # Safety
    ///
    /// The four floats from `at` and from each of the next three planes are readable.
    #[inline(always)]
    unsafe fn load(at: *const f32, plane: usize) -> [float32x4_t; 4] {
        // SAFETY: the caller keeps the four loads readable.
        unsafe {
            [
                vld1q_f32(at),
                vld1q_f32(at.add(plane)),
                vld1q_f32(at.add(2 * plane)),
                vld1q_f32(at.add(3 * plane)),
            ]
        }
    }

    /// The Plus blend of a copy over the sum before it, clamped to one, channel by channel.
    #[inline(always)]
    fn plus(copy: [float32x4_t; 4], sum: [float32x4_t; 4], one: float32x4_t) -> [float32x4_t; 4] {
        // SAFETY: NEON is part of the aarch64 baseline.
        std::array::from_fn(|channel| unsafe {
            vminq_f32(vaddq_f32(copy[channel], sum[channel]), one)
        })
    }

    /// `value` stored as half floats and read back.
    #[inline(always)]
    fn round(value: float32x4_t) -> float32x4_t {
        // SAFETY: NEON is part of the aarch64 baseline.
        unsafe { vcvt_f32_f16(vcvt_f16_f32(value)) }
    }
}
