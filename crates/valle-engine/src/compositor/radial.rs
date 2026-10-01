//! Deterministic radial blur over premultiplied working-linear RGBA16F pixels.

use super::bloom::{BloomError, encode_output, pixel_count, quantize_half, read_base};

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
    let original = read_base(
        input,
        input_width,
        input_height,
        input_width,
        input_height,
        [0, 0],
    )?;
    let mut source = std::borrow::Cow::Borrowed(original.as_slice());
    let input_origin = [
        output_origin[0]
            .checked_add(input_offset[0])
            .ok_or(BloomError::InvalidParameters)?,
        output_origin[1]
            .checked_add(input_offset[1])
            .ok_or(BloomError::InvalidParameters)?,
    ];
    let plan = radial_blur_plan(
        params.amount,
        input_origin,
        [input_width, input_height],
        output_origin,
        [output_width, output_height],
    )?;
    let mut origin = input_origin;
    let mut size = [input_width, input_height];
    for (index, pass) in plan.iter().enumerate() {
        let mut output = vec![[0.0; 4]; pixel_count(pass.size[0], pass.size[1])?];
        for y in 0..pass.size[1] as usize {
            for x in 0..pass.size[0] as usize {
                let ax = i64::from(pass.origin[0]) + x as i64;
                let ay = i64::from(pass.origin[1]) + y as i64;
                let sx = (ax - i64::from(origin[0])) as f64;
                let sy = (ay - i64::from(origin[1])) as f64;
                let dx = ax as f64 + 0.5 - f64::from(params.center[0]);
                let dy = ay as f64 + 0.5 - f64::from(params.center[1]);
                let distance = dx.hypot(dy);
                let reach = f64::from(pass.fraction) * f64::from(params.amount).min(distance * 0.5);
                let center = source_at(
                    &source,
                    size[0] as usize,
                    size[1] as usize,
                    sx as i64,
                    sy as i64,
                );
                let pixel = if index + 1 == plan.len()
                    && f64::from(params.amount).min(distance * 0.5) <= 8.0
                {
                    exact_pixel(
                        &original,
                        input_width as usize,
                        input_height as usize,
                        (ax - i64::from(input_origin[0])) as f64,
                        (ay - i64::from(input_origin[1])) as f64,
                        dx,
                        dy,
                        params.amount,
                    )
                } else if reach <= 0.0 || distance < 1e-12 {
                    center
                } else {
                    let tx = dx / distance * reach;
                    let ty = dy / distance * reach;
                    let before = bilinear_decal(
                        &source,
                        size[0] as usize,
                        size[1] as usize,
                        sx - tx,
                        sy - ty,
                    );
                    let after = bilinear_decal(
                        &source,
                        size[0] as usize,
                        size[1] as usize,
                        sx + tx,
                        sy + ty,
                    );
                    std::array::from_fn(|c| {
                        quantize_half((before[c] + after[c]) * 0.25 + f64::from(center[c]) * 0.5)
                    })
                };
                output[y * pass.size[0] as usize + x] = pixel;
            }
        }
        source = std::borrow::Cow::Owned(output);
        origin = pass.origin;
        size = pass.size;
    }
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
    if !amount.is_finite() || amount < 0.0 {
        return Err(BloomError::InvalidParameters);
    }
    pixel_count(input_size[0], input_size[1])?;
    pixel_count(output_size[0], output_size[1])?;
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
    let pads = fractions
        .iter()
        .map(|f| ((f64::from(amount) * f64::from(*f)).ceil() as i64).saturating_add(1))
        .collect::<Vec<_>>();
    let mut support = rect(input_origin, input_size);
    let supports = pads
        .iter()
        .map(|pad| {
            support = inflate(support, *pad);
            support
        })
        .collect::<Vec<_>>();
    let mut needed = rect(output_origin, output_size);
    let mut passes = Vec::with_capacity(count as usize);
    for i in (0..count as usize).rev() {
        let bounds = if i + 1 == count as usize {
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

// A small central neighborhood uses the original source directly. This avoids
// accumulating bilinear interpolation diffusion at the radial singularity;
// the number of taps here is bounded independently of output scale and amount.
fn exact_pixel(
    source: &[[f32; 4]],
    width: usize,
    height: usize,
    sx: f64,
    sy: f64,
    dx: f64,
    dy: f64,
    amount: f32,
) -> [f32; 4] {
    let distance = dx.hypot(dy);
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

fn source_at(pixels: &[[f32; 4]], width: usize, height: usize, x: i64, y: i64) -> [f32; 4] {
    if x >= 0 && x < width as i64 && y >= 0 && y < height as i64 {
        pixels[y as usize * width + x as usize]
    } else {
        [0.0; 4]
    }
}

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
    let mut result = [0.0_f64; 4];
    for (sx, sy, weight) in [
        (x0, y0, (1.0 - fx) * (1.0 - fy)),
        (x0 + 1, y0, fx * (1.0 - fy)),
        (x0, y0 + 1, (1.0 - fx) * fy),
        (x0 + 1, y0 + 1, fx * fy),
    ] {
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
