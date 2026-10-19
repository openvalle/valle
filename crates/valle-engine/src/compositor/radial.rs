//! Deterministic radial blur over premultiplied working-linear RGBA16F pixels.

use super::bloom::{
    BloomError, encode_output, fill_image_rows, pixel_count, quantize_halves, read_base,
    source_pixel_count,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RadialBlurParams {
    /// Center in device coordinates, before subtracting the output ROI origin.
    pub center: [f32; 2],
    /// Maximum displacement along a ray, in device pixels.
    pub amount: f32,
}

pub fn apply_radial_blur_f16(
    input: &[u8],
    input_width: u32,
    input_height: u32,
    output_width: u32,
    output_height: u32,
    input_offset: [i32; 2],
    output_origin: [i32; 2],
    params: RadialBlurParams,
) -> Result<Vec<u8>, BloomError> {
    if !params.center.iter().all(|value| value.is_finite())
        || !params.amount.is_finite()
        || params.amount < 0.0
    {
        return Err(BloomError::InvalidParameters);
    }
    let input_origin = [
        output_origin[0]
            .checked_add(input_offset[0])
            .ok_or(BloomError::InvalidParameters)?,
        output_origin[1]
            .checked_add(input_offset[1])
            .ok_or(BloomError::InvalidParameters)?,
    ];
    let input_size = [input_width, input_height];
    // Decode only the input the passes read: the rest of a larger input never reaches the output.
    let Some((used_origin, used_size)) = radial_blur_input(
        params.amount,
        input_origin,
        input_size,
        output_origin,
        [output_width, output_height],
    )?
    else {
        // No input pixel reaches the output, which stays transparent. Every input pixel is still
        // checked, as when some do.
        read_base(input, input_width, input_height, 1, 1, [0, 0])?;
        return Ok(vec![0; pixel_count(output_width, output_height)? * 8]);
    };
    let original = read_base(
        input,
        input_width,
        input_height,
        used_size[0],
        used_size[1],
        [
            input_origin[0] - used_origin[0],
            input_origin[1] - used_origin[1],
        ],
    )?;
    // Taps keep the coordinates of the whole input, so decoding only a part of it cannot change
    // their rounding. Shifting them by whole pixels onto that part is exact.
    let shift = [
        i64::from(used_origin[0]) - i64::from(input_origin[0]),
        i64::from(used_origin[1]) - i64::from(input_origin[1]),
    ];
    let mut source = std::borrow::Cow::Borrowed(original.as_slice());
    let plan = radial_blur_plan(
        params.amount,
        input_origin,
        input_size,
        output_origin,
        [output_width, output_height],
    )?;
    let unmoved = Unmoved::new(params, plan.len(), input_size);
    let mut origin = input_origin;
    let mut size = used_size;
    // From `origin` to the first pixel of `source`.
    let mut offset = shift;
    // Every pass writes each of its pixels, so the two buffers alternate without clearing.
    let mut spare = Vec::new();
    for (index, pass) in plan.iter().enumerate() {
        let mut output = std::mem::take(&mut spare);
        output.resize(pixel_count(pass.size[0], pass.size[1])?, [0.0; 4]);
        let pixel_at = |ax: i64, ay: i64| -> [f32; 4] {
            let sx = (ax - i64::from(origin[0])) as f64;
            let sy = (ay - i64::from(origin[1])) as f64;
            let dx = ax as f64 + 0.5 - f64::from(params.center[0]);
            let dy = ay as f64 + 0.5 - f64::from(params.center[1]);
            let distance = dx.hypot(dy);
            let limited = f64::from(params.amount).min(distance * 0.5);
            let reach = f64::from(pass.fraction) * limited;
            if index + 1 == plan.len() && limited <= 8.0 {
                return exact_pixel(
                    &original,
                    used_size[0] as usize,
                    used_size[1] as usize,
                    shift,
                    (ax - i64::from(input_origin[0])) as f64,
                    (ay - i64::from(input_origin[1])) as f64,
                    dx,
                    dy,
                    distance,
                    params.amount,
                );
            }
            let center = source_at(
                &source,
                size[0] as usize,
                size[1] as usize,
                sx as i64 - offset[0],
                sy as i64 - offset[1],
            );
            if reach <= 0.0 || distance < 1e-12 {
                center
            } else {
                let tx = dx / distance * reach;
                let ty = dy / distance * reach;
                let [ox, oy] = offset.map(|offset| offset as f64);
                let [before, after] = bilinear_pair(
                    &source,
                    size[0] as usize,
                    size[1] as usize,
                    [sx - tx - ox, sx + tx - ox],
                    [sy - ty - oy, sy + ty - oy],
                );
                quantize_halves(std::array::from_fn(|c| {
                    (before[c] + after[c]) * 0.25 + f64::from(center[c]) * 0.5
                }))
            }
        };
        let width = pass.size[0] as usize;
        fill_image_rows(&mut output, width, |first, rows| {
            for (row, pixels) in rows.chunks_mut(width).enumerate() {
                let ay = i64::from(pass.origin[1]) + (first + row) as i64;
                let unmoved_row = unmoved
                    .as_ref()
                    .and_then(|unmoved| unmoved.row(&original, used_size, used_origin, ay));
                for (x, pixel) in pixels.iter_mut().enumerate() {
                    let ax = i64::from(pass.origin[0]) + x as i64;
                    *pixel = match unmoved_row.as_ref().and_then(|row| row.keep(ax)) {
                        Some(kept) => kept,
                        None => pixel_at(ax, ay),
                    };
                }
            }
        });
        if let std::borrow::Cow::Owned(previous) =
            std::mem::replace(&mut source, std::borrow::Cow::Owned(output))
        {
            spare = previous;
        }
        origin = pass.origin;
        size = pass.size;
        offset = [0, 0];
    }
    drop(spare);
    Ok(encode_output(&source))
}

/// Three-tap binomial passes at geometric spacings approximate the triangular
/// kernel with O(log(radius)) samples. Fractions sum to one. Near the center,
/// every pass receives its fraction of the same distance-limited radius, so
/// iterating cannot send a ray across the center or turn a point into a ring.
#[derive(Debug, Clone, Copy)]
pub struct RadialBlurPass {
    pub origin: [i32; 2],
    pub size: [u32; 2],
    pub fraction: f32,
}

pub fn radial_blur_plan(
    amount: f32,
    input_origin: [i32; 2],
    input_size: [u32; 2],
    output_origin: [i32; 2],
    output_size: [u32; 2],
) -> Result<Vec<RadialBlurPass>, BloomError> {
    // Only the input the passes read is admitted, however far a larger input extends.
    if let Some((_, used)) =
        radial_blur_input(amount, input_origin, input_size, output_origin, output_size)?
    {
        pixel_count(used[0], used[1])?;
    }
    pixel_count(output_size[0], output_size[1])?;
    let (fractions, pads) = radial_blur_steps(amount);
    let count = fractions.len();
    let rect = |origin: [i32; 2], size: [u32; 2]| {
        [
            i64::from(origin[0]),
            i64::from(origin[1]),
            i64::from(origin[0]) + i64::from(size[0]),
            i64::from(origin[1]) + i64::from(size[1]),
        ]
    };
    let inflate = |r: [i64; 4], pad: i64| {
        [
            r[0].saturating_sub(pad),
            r[1].saturating_sub(pad),
            r[2].saturating_add(pad),
            r[3].saturating_add(pad),
        ]
    };
    let mut support = rect(input_origin, input_size);
    let supports = pads
        .iter()
        .map(|pad| {
            support = inflate(support, *pad);
            support
        })
        .collect::<Vec<_>>();
    let mut needed = rect(output_origin, output_size);
    let mut passes = Vec::with_capacity(count);
    for i in (0..count).rev() {
        let bounds = if i + 1 == count {
            needed
        } else {
            let s = supports[i];
            [
                needed[0].max(s[0]),
                needed[1].max(s[1]),
                needed[2].min(s[2]),
                needed[3].min(s[3]),
            ]
        };
        // An empty support still needs a zero image for the following pass. A
        // single transparent pixel outside the input is sufficient.
        let size = [
            u32::try_from(bounds[2].saturating_sub(bounds[0]).max(1))
                .map_err(|_| BloomError::InvalidParameters)?,
            u32::try_from(bounds[3].saturating_sub(bounds[1]).max(1))
                .map_err(|_| BloomError::InvalidParameters)?,
        ];
        pixel_count(size[0], size[1])?;
        let origin = [
            i32::try_from(bounds[0]).map_err(|_| BloomError::InvalidParameters)?,
            i32::try_from(bounds[1]).map_err(|_| BloomError::InvalidParameters)?,
        ];
        passes.push(RadialBlurPass {
            origin,
            size,
            fraction: if amount == 0.0 { 0.0 } else { fractions[i] },
        });
        needed = inflate(bounds, pads[i]);
    }
    passes.reverse();
    Ok(passes)
}

/// The input pixels a radial blur of `amount` reads to write `output`: those within the passes'
/// combined reach of it, as an origin and a size. `None` when no input pixel is that close.
pub fn radial_blur_input(
    amount: f32,
    input_origin: [i32; 2],
    input_size: [u32; 2],
    output_origin: [i32; 2],
    output_size: [u32; 2],
) -> Result<Option<([i32; 2], [u32; 2])>, BloomError> {
    if !amount.is_finite() || amount < 0.0 {
        return Err(BloomError::InvalidParameters);
    }
    source_pixel_count(input_size[0], input_size[1])?;
    let reach = radial_blur_steps(amount)
        .1
        .into_iter()
        .fold(0_i64, i64::saturating_add);
    let mut origin = [0; 2];
    let mut size = [0; 2];
    for axis in 0..2 {
        let input = i64::from(input_origin[axis]);
        let output = i64::from(output_origin[axis]);
        let start = input.max(output.saturating_sub(reach));
        let end = (input + i64::from(input_size[axis]))
            .min((output + i64::from(output_size[axis])).saturating_add(reach));
        if start >= end {
            return Ok(None);
        }
        origin[axis] = i32::try_from(start).map_err(|_| BloomError::InvalidParameters)?;
        size[axis] = u32::try_from(end - start).map_err(|_| BloomError::InvalidParameters)?;
    }
    Ok(Some((origin, size)))
}

/// The fraction of the amount each pass travels, and how far each pass reads beyond the pixels
/// it writes, in whole pixels.
fn radial_blur_steps(amount: f32) -> (Vec<f32>, Vec<i64>) {
    let mut count = 1_u32;
    let mut denominator = 1.0_f64;
    while denominator < f64::from(amount) {
        denominator = denominator * 2.0 + 1.0;
        count += 1;
    }
    if amount <= 8.0 {
        count = 1;
        denominator = 1.0;
    }
    let mut step = 1.0;
    let fractions = (0..count)
        .map(|_| {
            let f = (step / denominator) as f32;
            step *= 2.0;
            f
        })
        .collect::<Vec<_>>();
    let pads = fractions
        .iter()
        .map(|f| ((f64::from(amount) * f64::from(*f)).ceil() as i64).saturating_add(1))
        .collect();
    (fractions, pads)
}

// A small central neighborhood uses the original source directly. This avoids
// accumulating bilinear interpolation diffusion at the radial singularity;
// the number of taps here is bounded independently of output scale and amount.
//
// `sx` and `sy` count from the whole input's origin; `source` holds its pixels from `shift` on.
fn exact_pixel(
    source: &[[f32; 4]],
    width: usize,
    height: usize,
    shift: [i64; 2],
    sx: f64,
    sy: f64,
    dx: f64,
    dy: f64,
    distance: f64,
    amount: f32,
) -> [f32; 4] {
    let pixel_at = || {
        source_at(
            source,
            width,
            height,
            sx as i64 - shift[0],
            sy as i64 - shift[1],
        )
    };
    let [ox, oy] = shift.map(|shift| shift as f64);
    if distance < 1e-12 || amount == 0.0 {
        return pixel_at();
    }
    let reach = f64::from(amount).min(distance * 0.5);
    let steps = reach.ceil().max(1.0) as i32;
    if steps == 1 {
        // The loop below with one step, whose travel is `reach` itself rather than divided by
        // one: taps -1 and 1 travel it in opposite directions, and negating a travel negates its
        // products exactly, so the two are sampled together.
        let travel = [dx / distance * reach, dy / distance * reach];
        let [before, after] = bilinear_pair(
            source,
            width,
            height,
            [sx - travel[0] - ox, sx + travel[0] - ox],
            [sy - travel[1] - oy, sy + travel[1] - oy],
        );
        let pixel = pixel_at();
        // Weights 1, 2 and 1, then the quarter of their square.
        return quantize_halves(std::array::from_fn(|c| {
            (0.0 + before[c] * 1.0 + (0.0 + f64::from(pixel[c])) * 2.0 + after[c] * 1.0) * 0.25
        }));
    }
    let mut sum = [0.0; 4];
    for tap in -steps..=steps {
        let weight = f64::from(steps + 1 - tap.abs());
        let sample = if tap == 0 {
            // The middle tap lands on the pixel itself: bilinear weights 1, 0, 0, 0 over
            // finite pixels give the pixel, with a negative zero made positive.
            let pixel = pixel_at();
            std::array::from_fn(|c| 0.0 + f64::from(pixel[c]))
        } else {
            let travel = f64::from(tap) * reach / f64::from(steps);
            bilinear_decal(
                source,
                width,
                height,
                sx + dx / distance * travel - ox,
                sy + dy / distance * travel - oy,
            )
        };
        for c in 0..4 {
            sum[c] += sample[c] * weight;
        }
    }
    let divisor = f64::from((steps + 1) * (steps + 1));
    if (steps + 1).count_ones() == 1 {
        // Dividing by a power of two rounds like multiplying by its exact reciprocal.
        let scale = divisor.recip();
        for v in &mut sum {
            *v *= scale;
        }
    } else {
        for v in &mut sum {
            *v /= divisor;
        }
    }
    quantize_halves(sum)
}

/// The pixels that a blur of one pass under a quarter pixel leaves as they are.
///
/// Such a blur takes the `steps == 1` branch of [`exact_pixel`] at every pixel `p` but beside the
/// center. Its two taps lean at most `s` toward a neighboring column and row, `s` being the amount
/// plus the rounding of the pixel's coordinate with the travel added, so away from the input's
/// border they read `p`'s 3×3 neighborhood and give its eight neighbors at most `2s` of their
/// weight. With `D` bounding how far a neighbor's channel lies from `p`'s, each tap lies within
/// `2s·D` of `p`, plus `8·2^-53·(|p| + D)` for the rounding of the weights and the sums; weighing
/// the taps 1, 2 and 1 halves that and adds `2·2^-53·|p|`, and rounding to f32 adds `2^-24` of the
/// result. The f32 thus lies within `slope·D + 2^-22·|p|` of `p`. Below half the gap between `p`
/// and the half beside it on either side, which is at least `2^-12` of the power of two at or
/// below `|p|` and `2^-25` among half subnormals, it rounds back to `p`. A zero is kept only
/// beside neighbors that are not negative, whose blur cannot round to −0; a −0 is kept as +0, as
/// the blur's sums leave it.
#[derive(Debug, Clone, Copy)]
struct Unmoved {
    /// `1.25·amount + 2^-52·(size + 2)`, which covers `s + 4·2^-53` with room for the rounding of
    /// the travel's direction and of the test's own arithmetic.
    slope: f32,
    /// The column and row of the center, beside which `exact_pixel` may copy a pixel as it is
    /// rather than blur it.
    center: [i64; 2],
}

impl Unmoved {
    fn new(params: RadialBlurParams, passes: usize, input_size: [u32; 2]) -> Option<Self> {
        (passes == 1 && params.amount > 0.0 && params.amount < 0.25).then(|| Self {
            slope: (1.25 * f64::from(params.amount)
                + (f64::from(input_size[0].max(input_size[1])) + 2.0) * f64::EPSILON)
                as f32,
            center: params.center.map(|value| f64::from(value).floor() as i64),
        })
    }

    /// The three input rows around device row `y`, if they lie inside the input.
    fn row<'a>(
        &self,
        pixels: &'a [[f32; 4]],
        [width, height]: [u32; 2],
        origin: [i32; 2],
        y: i64,
    ) -> Option<UnmovedRow<'a>> {
        let row = y - i64::from(origin[1]);
        if row < 1 || row + 1 >= i64::from(height) || width < 3 {
            return None;
        }
        let width = width as usize;
        let top = (row as usize - 1) * width;
        let rows = &pixels[top..top + 3 * width];
        Some(UnmovedRow {
            rows: [&rows[..width], &rows[width..2 * width], &rows[2 * width..]],
            left: i64::from(origin[0]),
            slope: self.slope,
            center: (y.abs_diff(self.center[1]) <= 1).then_some(self.center[0]),
        })
    }
}

struct UnmovedRow<'a> {
    rows: [&'a [[f32; 4]]; 3],
    /// The device column of the rows' first pixel.
    left: i64,
    slope: f32,
    /// The center's column, on rows beside the center's.
    center: Option<i64>,
}

impl UnmovedRow<'_> {
    /// The blurred pixel at device column `x`, if the blur leaves it as it is.
    #[inline(always)]
    fn keep(&self, x: i64) -> Option<[f32; 4]> {
        let column = x - self.left;
        if column < 1
            || column + 1 >= self.rows[1].len() as i64
            || self.center.is_some_and(|center| x.abs_diff(center) <= 1)
        {
            return None;
        }
        #[cfg(all(target_arch = "aarch64", target_endian = "little"))]
        {
            neon::keep(self.rows, column as usize, self.slope)
        }
        #[cfg(not(all(target_arch = "aarch64", target_endian = "little")))]
        {
            keep(self.rows, column as usize, self.slope)
        }
    }
}

/// `p + 0.0` for the pixel `p` at `x` of the middle row if [`Unmoved`] bounds its blur within
/// half a gap of it, given the `slope` of the bound.
#[cfg(any(test, not(all(target_arch = "aarch64", target_endian = "little"))))]
fn keep(rows: [&[[f32; 4]]; 3], x: usize, slope: f32) -> Option<[f32; 4]> {
    let pixel = rows[1][x];
    let mut kept = true;
    for (channel, value) in pixel.into_iter().enumerate() {
        let (mut high, mut low) = (value, value);
        for row in rows {
            for neighbor in &row[x - 1..=x + 1] {
                high = high.max(neighbor[channel]);
                low = low.min(neighbor[channel]);
            }
        }
        let bound = (high - value).max(value - low) * slope + value.abs() * (1.0 / 4_194_304.0);
        let gap = if value == 0.0 && low < 0.0 {
            0.0
        } else {
            (f32::from_bits(value.to_bits() & 0x7f80_0000) * (1.0 / 4096.0)).max(1.0 / 33_554_432.0)
        };
        kept &= bound < gap;
    }
    kept.then(|| pixel.map(|value| value + 0.0))
}

/// [`bilinear_decal`] at two points.
#[inline(always)]
fn bilinear_pair(
    pixels: &[[f32; 4]],
    width: usize,
    height: usize,
    x: [f64; 2],
    y: [f64; 2],
) -> [[f64; 4]; 2] {
    #[cfg(all(target_arch = "aarch64", target_endian = "little"))]
    if let Some(samples) = neon::bilinear_pair(pixels, width, height, x, y) {
        return samples;
    }
    [
        bilinear_decal(pixels, width, height, x[0], y[0]),
        bilinear_decal(pixels, width, height, x[1], y[1]),
    ]
}

#[cfg(all(target_arch = "aarch64", target_endian = "little"))]
mod neon {
    use std::arch::aarch64::*;

    /// [`super::bilinear_decal`] at two points whose taps all lie inside the image, lane by lane
    /// its operations, or `None` where a tap of either lies outside.
    #[inline(always)]
    pub(super) fn bilinear_pair(
        pixels: &[[f32; 4]],
        width: usize,
        height: usize,
        x: [f64; 2],
        y: [f64; 2],
    ) -> Option<[[f64; 4]; 2]> {
        // SAFETY: NEON is part of the aarch64 baseline; the loads read the two coordinates and
        // pixels of `pixels`, and the stores write the channels of the two samples.
        unsafe {
            let (x, y) = (vld1q_f64(x.as_ptr()), vld1q_f64(y.as_ptr()));
            // `floor() as i64` saturates as the rounding conversion does.
            let (x0, y0) = (vcvtmq_s64_f64(x), vcvtmq_s64_f64(y));
            let inside = vandq_u64(
                vandq_u64(vcgezq_s64(x0), vcltq_s64(x0, vdupq_n_s64(width as i64 - 1))),
                vandq_u64(
                    vcgezq_s64(y0),
                    vcltq_s64(y0, vdupq_n_s64(height as i64 - 1)),
                ),
            );
            if vminvq_u32(vreinterpretq_u32_u64(inside)) == 0 {
                return None;
            }
            let fx = vsubq_f64(x, vcvtq_f64_s64(x0));
            let fy = vsubq_f64(y, vcvtq_f64_s64(y0));
            let one = vdupq_n_f64(1.0);
            let (gx, gy) = (vsubq_f64(one, fx), vsubq_f64(one, fy));
            let weights = [
                vmulq_f64(gx, gy),
                vmulq_f64(fx, gy),
                vmulq_f64(gx, fy),
                vmulq_f64(fx, fy),
            ];
            Some([
                sample::<0>(pixels, width, x0, y0, &weights),
                sample::<1>(pixels, width, x0, y0, &weights),
            ])
        }
    }

    /// The sample of lane `LANE`, whose four taps lie inside the image.
    #[inline(always)]
    fn sample<const LANE: i32>(
        pixels: &[[f32; 4]],
        width: usize,
        x0: int64x2_t,
        y0: int64x2_t,
        weights: &[float64x2_t; 4],
    ) -> [f64; 4] {
        // SAFETY: NEON is part of the aarch64 baseline; the loads read pixels of `pixels` and the
        // stores write the four channels of `sample`.
        unsafe {
            let top =
                vgetq_lane_s64::<LANE>(y0) as usize * width + vgetq_lane_s64::<LANE>(x0) as usize;
            let rows = &pixels[top..top + width + 2];
            let (mut low, mut high) = (vdupq_n_f64(0.0), vdupq_n_f64(0.0));
            for (pixel, weight) in [rows[0], rows[1], rows[width], rows[width + 1]]
                .iter()
                .zip(weights)
            {
                let value = vld1q_f32(pixel.as_ptr());
                low = vaddq_f64(
                    low,
                    vmulq_laneq_f64::<LANE>(vcvt_f64_f32(vget_low_f32(value)), *weight),
                );
                high = vaddq_f64(
                    high,
                    vmulq_laneq_f64::<LANE>(vcvt_high_f64_f32(value), *weight),
                );
            }
            let mut sample = [0.0; 4];
            vst1q_f64(sample.as_mut_ptr(), low);
            vst1q_f64(sample[2..].as_mut_ptr(), high);
            sample
        }
    }

    /// [`super::keep`], lane by lane its operations.
    #[inline(always)]
    pub(super) fn keep(rows: [&[[f32; 4]]; 3], x: usize, slope: f32) -> Option<[f32; 4]> {
        // SAFETY: NEON is part of the aarch64 baseline; the loads read pixels of `rows` and the
        // store writes the four channels of `kept`.
        unsafe {
            let pixel = vld1q_f32(rows[1][x].as_ptr());
            let (mut high, mut low) = (pixel, pixel);
            for row in rows {
                for neighbor in &row[x - 1..=x + 1] {
                    let neighbor = vld1q_f32(neighbor.as_ptr());
                    high = vmaxq_f32(high, neighbor);
                    low = vminq_f32(low, neighbor);
                }
            }
            let bound = vaddq_f32(
                vmulq_n_f32(
                    vmaxq_f32(vsubq_f32(high, pixel), vsubq_f32(pixel, low)),
                    slope,
                ),
                vmulq_n_f32(vabsq_f32(pixel), 1.0 / 4_194_304.0),
            );
            let unit = vreinterpretq_f32_u32(vandq_u32(
                vreinterpretq_u32_f32(pixel),
                vdupq_n_u32(0x7f80_0000),
            ));
            let gap = vmaxq_f32(
                vmulq_n_f32(unit, 1.0 / 4096.0),
                vdupq_n_f32(1.0 / 33_554_432.0),
            );
            let gap = vreinterpretq_f32_u32(vbicq_u32(
                vreinterpretq_u32_f32(gap),
                vandq_u32(vceqzq_f32(pixel), vcltzq_f32(low)),
            ));
            if vminvq_u32(vcltq_f32(bound, gap)) != u32::MAX {
                return None;
            }
            let mut kept = [0.0; 4];
            vst1q_f32(kept.as_mut_ptr(), vaddq_f32(pixel, vdupq_n_f32(0.0)));
            Some(kept)
        }
    }
}

fn source_at(pixels: &[[f32; 4]], width: usize, height: usize, x: i64, y: i64) -> [f32; 4] {
    if x >= 0 && x < width as i64 && y >= 0 && y < height as i64 {
        pixels[y as usize * width + x as usize]
    } else {
        [0.0; 4]
    }
}

#[inline(always)]
pub(crate) fn bilinear_decal(
    pixels: &[[f32; 4]],
    width: usize,
    height: usize,
    x: f64,
    y: f64,
) -> [f64; 4] {
    let x0 = x.floor() as i64;
    let y0 = y.floor() as i64;
    let fx = x - x0 as f64;
    let fy = y - y0 as f64;
    let taps = [
        (x0, y0, (1.0 - fx) * (1.0 - fy)),
        (x0 + 1, y0, fx * (1.0 - fy)),
        (x0, y0 + 1, (1.0 - fx) * fy),
        (x0 + 1, y0 + 1, fx * fy),
    ];
    let mut result = [0.0_f64; 4];
    // Inside the image all four taps exist; read them from two adjacent rows, in the same order.
    if x0 >= 0 && x0 < width as i64 - 1 && y0 >= 0 && y0 < height as i64 - 1 {
        let top = y0 as usize * width + x0 as usize;
        let rows = &pixels[top..top + width + 2];
        for (pixel, (_, _, weight)) in [rows[0], rows[1], rows[width], rows[width + 1]]
            .into_iter()
            .zip(taps)
        {
            for (channel, value) in result.iter_mut().zip(pixel) {
                *channel += weight * f64::from(value);
            }
        }
        return result;
    }
    for (sx, sy, weight) in taps {
        if sx >= 0 && sx < width as i64 && sy >= 0 && sy < height as i64 {
            for (channel, value) in result
                .iter_mut()
                .zip(pixels[sy as usize * width + sx as usize])
            {
                *channel += weight * f64::from(value);
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compositor::bloom::{f32_to_half, half_to_f32};

    #[test]
    fn radial_blur_spreads_a_bar_along_rays() {
        let (width, height) = (64_u32, 64_u32);
        let mut source = vec![0_u8; width as usize * height as usize * 8];
        for y in 28..36_usize {
            for x in 20..44_usize {
                let at = (y * width as usize + x) * 8;
                for channel in 0..4 {
                    source[at + channel * 2..at + channel * 2 + 2]
                        .copy_from_slice(&f32_to_half(1.0).to_le_bytes());
                }
            }
        }
        let params = RadialBlurParams {
            center: [32.5, 32.5],
            amount: 16.0,
        };
        let output = apply_radial_blur_f16(
            &source,
            width,
            height,
            width,
            height,
            [0, 0],
            [0, 0],
            params,
        )
        .unwrap();
        let sample = |x: usize, y: usize| {
            let at = (y * width as usize + x) * 8;
            half_to_f32(u16::from_le_bytes([output[at], output[at + 1]]))
        };
        assert!(sample(46, 32) > 0.1);
        assert!(sample(32, 38) > 0.0);
        assert!(sample(60, 32) < 0.01);
        assert_eq!(sample(32, 32), 1.0);
        assert_eq!(
            output,
            apply_radial_blur_f16(
                &source,
                width,
                height,
                width,
                height,
                [0, 0],
                [0, 0],
                params
            )
            .unwrap()
        );
        assert_eq!(
            source,
            apply_radial_blur_f16(
                &source,
                width,
                height,
                width,
                height,
                [0, 0],
                [0, 0],
                RadialBlurParams {
                    amount: 0.0,
                    ..params
                }
            )
            .unwrap()
        );
    }

    #[test]
    fn cropped_output_reads_source_outside_its_roi() {
        let mut source = vec![0_u8; 64 * 64 * 8];
        let at = (32 * 64 + 32) * 8;
        for channel in 0..4 {
            source[at + channel * 2..at + channel * 2 + 2]
                .copy_from_slice(&f32_to_half(1.0).to_le_bytes());
        }
        let params = RadialBlurParams {
            center: [0.5, 32.5],
            amount: 16.0,
        };
        let full = apply_radial_blur_f16(&source, 64, 64, 64, 64, [0, 0], [0, 0], params).unwrap();
        let cropped =
            apply_radial_blur_f16(&source, 64, 64, 16, 16, [-40, -24], [40, 24], params).unwrap();
        for y in 0..16 {
            for x in 0..16 {
                let full_at = ((y + 24) * 64 + x + 40) * 8;
                let cropped_at = (y * 16 + x) * 8;
                assert_eq!(
                    &cropped[cropped_at..cropped_at + 8],
                    &full[full_at..full_at + 8]
                );
            }
        }
        let glow_at = (8 * 16) * 8;
        assert!(half_to_f32(u16::from_le_bytes([cropped[glow_at], cropped[glow_at + 1]])) > 0.0);
    }

    #[test]
    fn large_inputs_are_admitted_by_the_pixels_the_passes_read() {
        // A 3600-pixel square turned 45 degrees behind a 1920x1080 output: about 26M pixels.
        let (origin, size) = ([-1586, -2006], [5092, 5092]);
        let reach = radial_blur_steps(32.0).1.iter().sum::<i64>() as i32;
        let used = radial_blur_input(32.0, origin, size, [0, 0], [1920, 1080])
            .unwrap()
            .unwrap();
        assert_eq!(
            used,
            (
                [-reach, -reach],
                [1920 + 2 * reach as u32, 1080 + 2 * reach as u32]
            )
        );
        // The used part has the same passes as the whole input.
        let whole = radial_blur_plan(32.0, origin, size, [0, 0], [1920, 1080]).unwrap();
        let part = radial_blur_plan(32.0, used.0, used.1, [0, 0], [1920, 1080]).unwrap();
        assert_eq!(whole.len(), part.len());
        for (whole, part) in whole.iter().zip(&part) {
            assert_eq!(
                (whole.origin, whole.size, whole.fraction),
                (part.origin, part.size, part.fraction)
            );
        }
        assert!(
            radial_blur_input(32.0, [1920 + reach, 0], [64, 64], [0, 0], [1920, 1080])
                .unwrap()
                .is_none()
        );
        // An input beyond the reach leaves the output transparent, and is still checked whole.
        let params = RadialBlurParams {
            center: [8.0, 8.0],
            amount: 32.0,
        };
        let blur = |input: &[u8]| {
            apply_radial_blur_f16(input, 8, 8, 16, 16, [16 + reach, 0], [0, 0], params)
        };
        let mut far = vec![0_u8; 8 * 8 * 8];
        far[..2].copy_from_slice(&f32_to_half(1.0).to_le_bytes());
        assert_eq!(blur(&far).unwrap(), vec![0; 16 * 16 * 8]);
        far[..2].copy_from_slice(&0x7e00_u16.to_le_bytes());
        assert!(matches!(blur(&far), Err(BloomError::InvalidImage)));
    }
}

#[cfg(test)]
mod point_regressions {
    use super::*;
    use crate::compositor::bloom::{f32_to_half, half_to_f32};
    #[test]
    fn center_does_not_form_rings_and_far_point_has_a_continuous_tail() {
        let mut source = vec![0; 129 * 9 * 8];
        let set = |source: &mut [u8], x: usize| {
            for c in 0..4 {
                let at = (4 * 129 + x) * 8 + c * 2;
                source[at..at + 2].copy_from_slice(&f32_to_half(1.0).to_le_bytes());
            }
        };
        set(&mut source, 4);
        let p = RadialBlurParams {
            center: [4.5, 4.5],
            amount: 32.0,
        };
        let out = apply_radial_blur_f16(&source, 129, 9, 129, 9, [0, 0], [0, 0], p).unwrap();
        for y in 0usize..9 {
            for x in 0usize..129 {
                if x.abs_diff(4) > 2 || y.abs_diff(4) > 2 {
                    let at = (y * 129 + x) * 8 + 6;
                    assert_eq!(&out[at..at + 2], &[0, 0], "ring at {x},{y}");
                }
            }
        }
        source.fill(0);
        set(&mut source, 80);
        let out = apply_radial_blur_f16(&source, 129, 9, 129, 9, [0, 0], [0, 0], p).unwrap();
        for x in 55..107 {
            let at = (4 * 129 + x) * 8 + 6;
            assert!(
                half_to_f32(u16::from_le_bytes([out[at], out[at + 1]])) > 0.0,
                "gap at {x}"
            );
        }
    }
}

#[cfg(test)]
mod scale_regressions {
    use super::*;
    use crate::compositor::bloom::{f32_to_half, half_to_f32};

    fn scene(scale: u32, amount: f32) -> Vec<f32> {
        let (w, h) = (128 * scale, 72 * scale);
        let mut source = vec![0; (w * h * 8) as usize];
        for (x0, y0, x1, y1) in [(60, 32, 68, 40), (108, 32, 112, 36)] {
            for y in y0 * scale..y1 * scale {
                for x in x0 * scale..x1 * scale {
                    for c in 0..4 {
                        let at = ((y * w + x) * 8 + c * 2) as usize;
                        source[at..at + 2].copy_from_slice(&f32_to_half(1.0).to_le_bytes());
                    }
                }
            }
        }
        let result = apply_radial_blur_f16(
            &source,
            w,
            h,
            w,
            h,
            [0, 0],
            [0, 0],
            RadialBlurParams {
                center: [64.0 * scale as f32, 36.0 * scale as f32],
                amount: amount * scale as f32,
            },
        )
        .unwrap();
        let mut reduced = vec![0.0; 128 * 72];
        for y in 0..h {
            for x in 0..w {
                let at = ((y * w + x) * 8) as usize;
                reduced[((y / scale) * 128 + x / scale) as usize] +=
                    half_to_f32(u16::from_le_bytes([result[at], result[at + 1]]))
                        / (scale * scale) as f32;
            }
        }
        reduced
    }

    #[test]
    fn geometric_sampling_keeps_output_scale_and_cost_bounded() {
        for amount in [8.0, 32.0, 128.0] {
            let base = scene(1, amount);
            for scale in [2, 4] {
                let scaled = scene(scale, amount);
                let (mut peak, mut mean) = (0.0_f32, 0.0_f64);
                for (a, b) in base.iter().zip(&scaled) {
                    let error = (a - b).abs();
                    peak = peak.max(error);
                    mean += f64::from(error);
                }
                mean /= base.len() as f64;
                assert!(
                    mean < 0.003 && peak < 0.08,
                    "amount {amount} scale {scale}: mean={mean}, peak={peak}"
                );
                let passes = radial_blur_plan(
                    amount * scale as f32,
                    [0, 0],
                    [128 * scale, 72 * scale],
                    [0, 0],
                    [128 * scale, 72 * scale],
                )
                .unwrap();
                assert!(passes.len() <= 10);
            }
        }
    }
}

#[cfg(test)]
mod reference_bits {
    use super::*;
    use crate::compositor::bloom::{f32_to_half, half_to_f32, quantize_half};

    // The blur before its middle-tap, division and buffer shortcuts, kept as the byte reference.
    fn reference_blur(
        input: &[u8],
        input_size: [u32; 2],
        output_size: [u32; 2],
        input_offset: [i32; 2],
        output_origin: [i32; 2],
        params: RadialBlurParams,
    ) -> Vec<u8> {
        let [input_width, input_height] = input_size;
        let original = read_base(
            input,
            input_width,
            input_height,
            input_width,
            input_height,
            [0, 0],
        )
        .unwrap();
        let mut source = original.clone();
        let input_origin = [
            output_origin[0] + input_offset[0],
            output_origin[1] + input_offset[1],
        ];
        let plan = radial_blur_plan(
            params.amount,
            input_origin,
            input_size,
            output_origin,
            output_size,
        )
        .unwrap();
        let mut origin = input_origin;
        let mut size = input_size;
        for (index, pass) in plan.iter().enumerate() {
            let mut output = vec![[0.0; 4]; pixel_count(pass.size[0], pass.size[1]).unwrap()];
            for (i, pixel) in output.iter_mut().enumerate() {
                let ax = i64::from(pass.origin[0]) + (i % pass.size[0] as usize) as i64;
                let ay = i64::from(pass.origin[1]) + (i / pass.size[0] as usize) as i64;
                let sx = (ax - i64::from(origin[0])) as f64;
                let sy = (ay - i64::from(origin[1])) as f64;
                let dx = ax as f64 + 0.5 - f64::from(params.center[0]);
                let dy = ay as f64 + 0.5 - f64::from(params.center[1]);
                let distance = dx.hypot(dy);
                let reach = f64::from(pass.fraction) * f64::from(params.amount).min(distance * 0.5);
                *pixel = if index + 1 == plan.len()
                    && f64::from(params.amount).min(distance * 0.5) <= 8.0
                {
                    reference_exact_pixel(
                        &original,
                        input_width as usize,
                        input_height as usize,
                        (ax - i64::from(input_origin[0])) as f64,
                        (ay - i64::from(input_origin[1])) as f64,
                        dx,
                        dy,
                        distance,
                        params.amount,
                    )
                } else {
                    let (w, h) = (size[0] as usize, size[1] as usize);
                    let center = source_at(&source, w, h, sx as i64, sy as i64);
                    if reach <= 0.0 || distance < 1e-12 {
                        center
                    } else {
                        let tx = dx / distance * reach;
                        let ty = dy / distance * reach;
                        let before = bilinear_decal(&source, w, h, sx - tx, sy - ty);
                        let after = bilinear_decal(&source, w, h, sx + tx, sy + ty);
                        std::array::from_fn(|c| {
                            quantize_half(
                                (before[c] + after[c]) * 0.25 + f64::from(center[c]) * 0.5,
                            )
                        })
                    }
                };
            }
            source = output;
            origin = pass.origin;
            size = pass.size;
        }
        encode_output(&source)
    }

    #[allow(clippy::too_many_arguments)]
    fn reference_exact_pixel(
        source: &[[f32; 4]],
        width: usize,
        height: usize,
        sx: f64,
        sy: f64,
        dx: f64,
        dy: f64,
        distance: f64,
        amount: f32,
    ) -> [f32; 4] {
        if distance < 1e-12 || amount == 0.0 {
            return source_at(source, width, height, sx as i64, sy as i64);
        }
        let reach = f64::from(amount).min(distance * 0.5);
        let steps = reach.ceil().max(1.0) as i32;
        let mut sum = [0.0; 4];
        for tap in -steps..=steps {
            let weight = f64::from(steps + 1 - tap.abs());
            let travel = f64::from(tap) * reach / f64::from(steps);
            let sample = bilinear_decal(
                source,
                width,
                height,
                sx + dx / distance * travel,
                sy + dy / distance * travel,
            );
            for c in 0..4 {
                sum[c] += sample[c] * weight;
            }
        }
        sum.map(|v| quantize_half(v / f64::from((steps + 1) * (steps + 1))))
    }

    #[test]
    fn shortcuts_keep_every_output_bit() {
        let mut state = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let (width, height) = (61_u32, 37_u32);
        // Smooth ramps, hard edges, signed zeros, negative and large channels.
        let palette = [
            0.0, -0.0, 1.0, 0.5, 0.25, 1.0e-4, -0.125, 3.0, 60_000.0, 6.0e-8,
        ];
        let input = (0..width * height)
            .flat_map(|index| {
                let ramp = (index % width) as f32 / width as f32;
                let pick = next();
                (0..4)
                    .map(|channel| match (pick >> (channel * 8)) % 4 {
                        0 => ramp,
                        1 => palette[((pick >> (32 + channel * 4)) % 10) as usize],
                        _ => ramp * (channel + 1) as f32 * 0.3,
                    })
                    .collect::<Vec<_>>()
            })
            .flat_map(|channel| f32_to_half(channel).to_le_bytes())
            .collect::<Vec<_>>();
        let amounts = [
            1.0e-12, 3.0e-7, 1.0e-5, 0.01, 0.4, 0.75, 1.0, 1.0001, 2.5, 3.0, 4.0, 7.0, 7.9, 8.0,
            8.5, 21.0, 67.0,
        ];
        let centers = [[30.5, 18.5], [30.0, 18.0], [0.25, 36.75], [-20.0, 70.0]];
        let crops = [
            ([width, height], [0, 0], [0, 0]),
            ([23, 17], [-9, -5], [9, 5]),
            ([70, 45], [4, 3], [-4, -3]),
            // Far enough from the input's left and top that the passes read only a part of it.
            ([23, 17], [-30, -18], [30, 18]),
        ];
        for amount in amounts {
            for center in centers {
                for (output_size, input_offset, output_origin) in crops {
                    let params = RadialBlurParams { center, amount };
                    let actual = apply_radial_blur_f16(
                        &input,
                        width,
                        height,
                        output_size[0],
                        output_size[1],
                        input_offset,
                        output_origin,
                        params,
                    )
                    .unwrap();
                    let expected = reference_blur(
                        &input,
                        [width, height],
                        output_size,
                        input_offset,
                        output_origin,
                        params,
                    );
                    assert!(
                        actual == expected,
                        "amount {amount} center {center:?} crop {output_size:?}"
                    );
                }
            }
        }
    }

    /// Channels of every sign and binade beside neighbors from a hair to many binades away: the
    /// middle pixel of every 3×3 tile holds powers of two, zeros of either sign or other halves,
    /// and most of the tile's other channels lie 2^m from them for m from -30 to 8, so that for
    /// every amount some pixels lie just inside the bound of [`Unmoved`] and some just outside.
    fn rounding_edges(width: u32, height: u32) -> Vec<f32> {
        let mut state = 0x2545_f491_4f6c_dd1d_u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let mut centers = vec![0.0_f32; (width * height * 4) as usize];
        for value in &mut centers {
            let pick = next();
            let sign = if pick & 1 == 0 { 1.0 } else { -1.0 };
            *value = match (pick >> 1) % 5 {
                0 => 0.0 * sign,
                1 | 2 => sign * 2.0_f32.powi(((pick >> 8) % 31) as i32 - 24),
                _ => sign * half_to_f32(((pick >> 8) % 0x7c00) as u16),
            };
        }
        let mut channels = Vec::with_capacity(centers.len());
        for y in 0..height as usize {
            for x in 0..width as usize {
                let middle = x % 3 == 1 && y % 3 == 1;
                let center = (y / 3 * 3 + 1).min(height as usize - 1) * width as usize
                    + (x / 3 * 3 + 1).min(width as usize - 1);
                for value in centers[center * 4..center * 4 + 4].iter().copied() {
                    let pick = next();
                    let step = 2.0_f32.powi(((pick >> 2) % 39) as i32 - 30)
                        * (1.0 + ((pick >> 16) % 1024) as f32 / 1024.0);
                    let neighbor = half_to_f32(f32_to_half(if pick & (1 << 40) == 0 {
                        value + step
                    } else {
                        value - step
                    }));
                    channels.push(if middle || pick % 4 == 0 || !neighbor.is_finite() {
                        value
                    } else {
                        neighbor
                    });
                }
            }
        }
        channels
    }

    #[test]
    fn blurs_under_a_quarter_pixel_keep_pixels_only_where_they_round_back() {
        let (width, height) = (61_u32, 40_u32);
        let channels = rounding_edges(width, height);
        let input = channels
            .iter()
            .flat_map(|&channel| f32_to_half(channel).to_le_bytes())
            .collect::<Vec<_>>();
        let original = read_base(&input, width, height, width, height, [0, 0]).unwrap();
        let amounts = [
            1.0e-14, 1.0e-12, 1.0e-10, 3.0e-9, 1.0e-8, 3.0e-8, 1.0e-7, 3.0e-7, 1.0e-6, 3.0e-6,
            1.0e-5, 3.0e-5, 1.0e-4, 3.0e-4, 1.0e-3, 3.0e-3, 0.01, 0.03, 0.1, 0.2, 0.2499,
        ];
        // At pixel corners and centers inside, and far off along an axis and a diagonal.
        let centers = [
            [30.0, 20.0],
            [12.5, 31.5],
            [-4000.0, 20.25],
            [-900.0, -700.0],
        ];
        let crops = [
            ([width, height], [0, 0], [0, 0]),
            ([29, 21], [-11, -7], [11, 7]),
        ];
        let mut kept = 0;
        for amount in amounts {
            for center in centers {
                let params = RadialBlurParams { center, amount };
                let unmoved = Unmoved::new(params, 1, [width, height]).unwrap();
                kept += (0..i64::from(height))
                    .filter_map(|y| unmoved.row(&original, [width, height], [0, 0], y))
                    .map(|row| {
                        (0..i64::from(width))
                            .filter(|&x| row.keep(x).is_some())
                            .count()
                    })
                    .sum::<usize>();
                for (output_size, input_offset, output_origin) in crops {
                    let actual = apply_radial_blur_f16(
                        &input,
                        width,
                        height,
                        output_size[0],
                        output_size[1],
                        input_offset,
                        output_origin,
                        params,
                    )
                    .unwrap();
                    let expected = reference_blur(
                        &input,
                        [width, height],
                        output_size,
                        input_offset,
                        output_origin,
                        params,
                    );
                    assert!(
                        actual == expected,
                        "amount {amount} center {center:?} crop {output_size:?}"
                    );
                }
            }
        }
        let interior = (amounts.len() * centers.len()) as u32 * (width - 2) * (height - 2);
        assert!(
            kept as u32 * 10 > interior && (kept as u32) < interior,
            "kept {kept} of {interior}"
        );
    }

    #[cfg(all(target_arch = "aarch64", target_endian = "little"))]
    #[test]
    fn unmoved_pixels_are_the_same_lane_by_lane() {
        let (width, height) = (61_u32, 40_u32);
        let channels = rounding_edges(width, height);
        let pixels = channels.as_chunks::<4>().0;
        let mut kept = 0;
        for slope in [1.0e-14, 3.0e-9, 1.0e-6, 2.0e-4, 0.01, 0.3] {
            for y in 1..height as usize - 1 {
                let rows = [y - 1, y, y + 1]
                    .map(|row| &pixels[row * width as usize..(row + 1) * width as usize]);
                for x in 1..width as usize - 1 {
                    let lanes = neon::keep(rows, x, slope);
                    assert_eq!(
                        lanes.map(|pixel| pixel.map(f32::to_bits)),
                        keep(rows, x, slope).map(|pixel| pixel.map(f32::to_bits)),
                        "slope {slope} at {x},{y}"
                    );
                    kept += usize::from(lanes.is_some());
                }
            }
        }
        assert!(kept > 0);
    }
}
