//! Stable monochrome film grain on premultiplied working-linear RGBA16F pixels.

use super::bloom::{BloomError, encode_output, fill_image_rows, quantize_half, read_base};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FilmGrainParams {
    pub seed: u32,
    /// Fraction of the available sRGB display-code headroom used by signed grain.
    pub amount: f32,
    /// Noise lattice spacing in device pixels.
    pub size: f32,
}

#[expect(
    clippy::too_many_arguments,
    reason = "Keep the rendering inputs, source and output geometry, and validation context explicit."
)]
pub fn apply_film_grain_f16(
    input: &[u8],
    input_width: u32,
    input_height: u32,
    output_width: u32,
    output_height: u32,
    input_offset: [i32; 2],
    output_origin: [i32; 2],
    params: FilmGrainParams,
) -> Result<Vec<u8>, BloomError> {
    if !params.amount.is_finite()
        || !(0.0..=1.0).contains(&params.amount)
        || !params.size.is_finite()
        || params.size < 1.0
    {
        return Err(BloomError::InvalidParameters);
    }
    let mut pixels = read_base(
        input,
        input_width,
        input_height,
        output_width,
        output_height,
        input_offset,
    )?;
    if params.amount == 0.0 {
        return Ok(encode_output(&pixels));
    }
    let width = output_width as usize;
    let invalid = std::sync::atomic::AtomicBool::new(false);
    fill_image_rows(&mut pixels, width, |first, rows| {
        let mut grained = Grained::new(params.amount);
        for (row_index, row) in rows.chunks_exact_mut(width).enumerate() {
            let y = i64::from(output_origin[1]) + (first + row_index) as i64;
            let mut lattice = LatticeRow::new(y, params.seed, f64::from(params.size));
            for column in 0..width {
                if row[column][3] == 0.0 {
                    continue;
                }
                let noise = lattice.at(i64::from(output_origin[0]) + column as i64);
                if grained.push(column, row[column], noise) && grained.apply(row).is_err() {
                    invalid.store(true, std::sync::atomic::Ordering::Relaxed);
                    return;
                }
            }
            if grained.apply(row).is_err() {
                invalid.store(true, std::sync::atomic::Ordering::Relaxed);
                return;
            }
        }
    });
    if invalid.into_inner() {
        return Err(BloomError::InvalidParameters);
    }
    Ok(encode_output(&pixels))
}

/// Up to 64 pixels of one row on their way through the grain. Their straight colors are encoded
/// together, the grain is added in display encoding, and the results are decoded together.
struct Grained {
    amount: f64,
    columns: [usize; 64],
    noise: [f64; 64],
    /// For each pixel, how many colors `fresh` held once it was pushed. The last of them is its
    /// color's encoding; with none, the memo's is.
    fresh_before: [u8; 64],
    fresh: [[f32; 3]; 64],
    grained: [[f32; 3]; 64],
    len: usize,
    fresh_len: usize,
    // This exact one-entry memo is local to the rows. Uniform spans share their straight
    // display conversion, while alpha and signed-zero bits remain part of the key.
    memo_key: Option<[u32; 4]>,
    memo_encoded: [f32; 3],
}

impl Grained {
    fn new(amount: f32) -> Self {
        Self {
            amount: f64::from(amount),
            columns: [0; 64],
            noise: [0.0; 64],
            fresh_before: [0; 64],
            fresh: [[0.0; 3]; 64],
            grained: [[0.0; 3]; 64],
            len: 0,
            fresh_len: 0,
            memo_key: None,
            memo_encoded: [0.0; 3],
        }
    }

    /// Adds the pixel at `column` with its noise and returns whether no more fit.
    fn push(&mut self, column: usize, pixel: [f32; 4], noise: f64) -> bool {
        let key = pixel.map(f32::to_bits);
        if self.memo_key != Some(key) {
            let alpha = pixel[3];
            self.fresh[self.fresh_len] = [pixel[0] / alpha, pixel[1] / alpha, pixel[2] / alpha];
            self.fresh_len += 1;
            self.memo_key = Some(key);
        }
        self.columns[self.len] = column;
        self.noise[self.len] = noise;
        self.fresh_before[self.len] = self.fresh_len as u8;
        self.len += 1;
        self.len == self.columns.len()
    }

    /// Grains the pixels and stores them in `row`, premultiplied by their alpha again.
    fn apply(&mut self, row: &mut [[f32; 4]]) -> Result<(), super::reference::ColorMathError> {
        let len = std::mem::take(&mut self.len);
        let fresh = &mut self.fresh[..std::mem::take(&mut self.fresh_len)];
        super::reference::working_to_extended_srgb_each(fresh)?;
        for ((grained, noise), fresh_before) in self
            .grained
            .iter_mut()
            .zip(&self.noise)
            .zip(&self.fresh_before)
            .take(len)
        {
            let mut encoded = match fresh_before {
                0 => self.memo_encoded,
                count => fresh[usize::from(*count) - 1],
            };
            // Construct symmetric noise in display encoding, where perceived gray
            // levels are reviewed. Limiting amplitude to both endpoint margins keeps
            // the distribution symmetric without clipping (including at amount=1).
            for channel in &mut encoded {
                let margin = channel.min(1.0 - *channel).max(0.0);
                *channel += (noise * self.amount * f64::from(margin)) as f32;
            }
            *grained = encoded;
        }
        if let Some(last) = fresh.last() {
            self.memo_encoded = *last;
        }
        let grained = &mut self.grained[..len];
        super::reference::extended_srgb_to_working_each(grained)?;
        for (&column, working) in self.columns.iter().zip(grained.iter()) {
            let pixel = &mut row[column];
            let alpha = pixel[3];
            for (channel, value) in pixel[..3].iter_mut().zip(working) {
                *channel = quantize_half(f64::from(value * alpha));
            }
        }
        Ok(())
    }
}

struct LatticeRow {
    seed: u32,
    size: f64,
    y0: i64,
    fy: f64,
    x0: Option<i64>,
    corners: [f64; 4],
}

impl LatticeRow {
    fn new(y: i64, seed: u32, size: f64) -> Self {
        let ny = y as f64 / size;
        let y0 = ny.floor() as i64;
        Self {
            seed,
            size,
            y0,
            fy: ny - y0 as f64,
            x0: None,
            corners: [0.0; 4],
        }
    }

    fn at(&mut self, x: i64) -> f64 {
        let nx = x as f64 / self.size;
        let x0 = nx.floor() as i64;
        let fx = nx - x0 as f64;
        if self.x0 != Some(x0) {
            self.corners = if self.x0 == Some(x0 - 1) {
                [
                    self.corners[1],
                    hash_noise(x0 + 1, self.y0, self.seed),
                    self.corners[3],
                    hash_noise(x0 + 1, self.y0 + 1, self.seed),
                ]
            } else {
                [
                    hash_noise(x0, self.y0, self.seed),
                    hash_noise(x0 + 1, self.y0, self.seed),
                    hash_noise(x0, self.y0 + 1, self.seed),
                    hash_noise(x0 + 1, self.y0 + 1, self.seed),
                ]
            };
            self.x0 = Some(x0);
        }
        let [a, b, c, d] = self.corners;
        (a * (1.0 - fx) + b * fx) * (1.0 - self.fy) + (c * (1.0 - fx) + d * fx) * self.fy
    }
}

/// Seeded lattice value shared with GPU texture preparation.
pub fn hash_noise(x: i64, y: i64, seed: u32) -> f64 {
    let value = hash_noise_bits(x, y, seed);
    f64::from(value) * (2.0 / f64::from(u32::MAX)) - 1.0
}

pub fn hash_noise_bits(x: i64, y: i64, seed: u32) -> u32 {
    let mut value =
        seed ^ (x as u32).wrapping_mul(0x9e37_79b9) ^ (y as u32).wrapping_mul(0x85eb_ca6b);
    value ^= value >> 16;
    value = value.wrapping_mul(0x7feb_352d);
    value ^= value >> 15;
    value = value.wrapping_mul(0x846c_a68b);
    value ^= value >> 16;
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compositor::bloom::{f32_to_half, half_to_f32};

    fn scalar_noise(x: i64, y: i64, seed: u32, size: f64) -> f64 {
        let nx = x as f64 / size;
        let ny = y as f64 / size;
        let x0 = nx.floor() as i64;
        let y0 = ny.floor() as i64;
        let fx = nx - x0 as f64;
        let fy = ny - y0 as f64;
        let a = hash_noise(x0, y0, seed);
        let b = hash_noise(x0 + 1, y0, seed);
        let c = hash_noise(x0, y0 + 1, seed);
        let d = hash_noise(x0 + 1, y0 + 1, seed);
        (a * (1.0 - fx) + b * fx) * (1.0 - fy) + (c * (1.0 - fx) + d * fx) * fy
    }

    // Keep the pre-optimization scalar traversal as the byte-level reference.
    fn scalar_film(
        input: &[u8],
        input_size: [u32; 2],
        output_size: [u32; 2],
        input_offset: [i32; 2],
        output_origin: [i32; 2],
        params: FilmGrainParams,
    ) -> Vec<u8> {
        let mut pixels = read_base(
            input,
            input_size[0],
            input_size[1],
            output_size[0],
            output_size[1],
            input_offset,
        )
        .unwrap();
        if params.amount == 0.0 {
            return encode_output(&pixels);
        }
        for (index, pixel) in pixels.iter_mut().enumerate() {
            let alpha = pixel[3];
            if alpha == 0.0 {
                continue;
            }
            let x = i64::from(output_origin[0]) + (index % output_size[0] as usize) as i64;
            let y = i64::from(output_origin[1]) + (index / output_size[0] as usize) as i64;
            let noise = scalar_noise(x, y, params.seed, f64::from(params.size));
            let mut encoded = super::super::reference::working_to_extended_srgb([
                pixel[0] / alpha,
                pixel[1] / alpha,
                pixel[2] / alpha,
            ])
            .unwrap();
            for channel in &mut encoded {
                let margin = channel.min(1.0 - *channel).max(0.0);
                *channel += (noise * f64::from(params.amount) * f64::from(margin)) as f32;
            }
            let working = super::super::reference::extended_srgb_to_working(encoded).unwrap();
            for (channel, value) in pixel[..3].iter_mut().zip(working) {
                *channel = quantize_half(f64::from(value * alpha));
            }
        }
        encode_output(&pixels)
    }

    #[test]
    fn lattice_row_reuse_preserves_scalar_bits_with_skips_and_negative_origins() {
        for seed in [0, 17, u32::MAX] {
            for size in [1.0_f32, 1.4, 2.0, 16.0, f32::MAX] {
                for origin in [i32::MIN, -19, 0, 47, i32::MAX] {
                    let y = i64::from(origin);
                    let mut row = LatticeRow::new(y, seed, f64::from(size));
                    for dx in [0, 1, 2, 3, 9, 10, 8, 31, 31, 32] {
                        let x = i64::from(origin) + dx;
                        assert_eq!(
                            row.at(x).to_bits(),
                            scalar_noise(x, y, seed, f64::from(size)).to_bits()
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn grain_reuse_matches_scalar_f16_for_alpha_hdr_signed_zero_and_crops() {
        let palette = [
            [0.0_f32; 4],
            [-0.0, 0.0, -0.0, 1.0],
            [0.25, 0.5, 0.125, 1.0],
            [0.25, 0.5, 0.125, 0.5],
            [-0.125, 1.5, 2.0, 0.25],
            [8.0, 3.0, -2.0, 1.0],
            [0.125, 0.25, 0.0625, 0.5],
            [0.25, 0.5, 0.125, 1.0],
        ];
        let input = (0..17 * 11)
            .flat_map(|index| {
                palette[(index / 5) % palette.len()]
                    .into_iter()
                    .flat_map(|c| f32_to_half(c).to_le_bytes())
            })
            .collect::<Vec<_>>();
        for seed in [0, 17, u32::MAX] {
            for size in [1.0, 1.4, 16.0] {
                for amount in [0.0, 0.105, 1.0] {
                    let params = FilmGrainParams { seed, amount, size };
                    for (output_size, offset, origin) in [
                        ([17, 11], [0, 0], [-19, -17]),
                        ([23, 17], [3, -2], [9, -5]),
                        ([80, 80], [3, -2], [9, -5]),
                        ([9, 7], [-5, -3], [i32::MAX, i32::MIN]),
                    ] {
                        assert_eq!(
                            apply_film_grain_f16(
                                &input,
                                17,
                                11,
                                output_size[0],
                                output_size[1],
                                offset,
                                origin,
                                params
                            )
                            .unwrap(),
                            scalar_film(&input, [17, 11], output_size, offset, origin, params),
                            "seed={seed} size={size} amount={amount} origin={origin:?} offset={offset:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn grain_matches_scalar_f16_over_random_premultiplied_pixels() {
        let mut state = 0x9e37_79b9_u32;
        let mut unit = move || {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 8) as f32 / 16_777_216.0
        };
        // Rows of 150 pixels fill whole batches of grained pixels and leave a partial one. Runs
        // of equal pixels, some transparent, carry the memo across batches and rows.
        for _ in 0..8 {
            let mut input = Vec::new();
            while input.len() < 150 * 7 * 8 {
                let alpha = match (unit() * 5.0) as u32 {
                    0 => 1.0,
                    1 => unit() * 1.0e-3,
                    2 => 0.0,
                    _ => unit(),
                };
                // Dark channels (on the linear part of the sRGB curve), the usual range, HDR
                // and negative ones.
                let straight: [f32; 3] = std::array::from_fn(|_| match (unit() * 4.0) as u32 {
                    0 => unit() * 0.005,
                    1 => unit(),
                    2 => unit() * 8.0,
                    _ => -unit() * 0.3,
                });
                let pixel = [
                    straight[0] * alpha,
                    straight[1] * alpha,
                    straight[2] * alpha,
                    alpha,
                ]
                .map(f32_to_half);
                for _ in 0..[1, 1, 2, 7, 70][(unit() * 5.0) as usize] {
                    input.extend(pixel.iter().flat_map(|c| c.to_le_bytes()));
                }
            }
            input.truncate(150 * 7 * 8);
            for (seed, amount, size) in [(7, 0.16, 2.0), (3, 1.0, 1.0), (11, 0.4, 1.5)] {
                let params = FilmGrainParams { seed, amount, size };
                for (output_size, offset, origin) in
                    [([150, 7], [0, 0], [0, 0]), ([170, 9], [17, 1], [-5, 9])]
                {
                    assert_eq!(
                        apply_film_grain_f16(
                            &input,
                            150,
                            7,
                            output_size[0],
                            output_size[1],
                            offset,
                            origin,
                            params
                        )
                        .unwrap(),
                        scalar_film(&input, [150, 7], output_size, offset, origin, params),
                    );
                }
            }
        }
    }

    #[test]
    fn grain_is_seeded_and_roi_stable() {
        let mut source = vec![0_u8; 32 * 24 * 8];
        for pixel in source.chunks_exact_mut(8) {
            for (channel, value) in [0.25, 0.25, 0.25, 1.0].into_iter().enumerate() {
                pixel[channel * 2..channel * 2 + 2]
                    .copy_from_slice(&f32_to_half(value).to_le_bytes());
            }
        }
        let params = FilmGrainParams {
            seed: 7,
            amount: 0.15,
            size: 2.0,
        };
        let full = apply_film_grain_f16(&source, 32, 24, 32, 24, [0, 0], [0, 0], params).unwrap();
        assert_eq!(
            full,
            apply_film_grain_f16(&source, 32, 24, 32, 24, [0, 0], [0, 0], params).unwrap()
        );
        assert_ne!(full, source);
        assert_ne!(
            full,
            apply_film_grain_f16(
                &source,
                32,
                24,
                32,
                24,
                [0, 0],
                [0, 0],
                FilmGrainParams { seed: 8, ..params }
            )
            .unwrap()
        );
        assert_eq!(
            source,
            apply_film_grain_f16(
                &source,
                32,
                24,
                32,
                24,
                [0, 0],
                [0, 0],
                FilmGrainParams {
                    amount: 0.0,
                    ..params
                }
            )
            .unwrap()
        );
        let crop = apply_film_grain_f16(&source, 32, 24, 12, 9, [-7, -5], [7, 5], params).unwrap();
        for y in 0..9 {
            for x in 0..12 {
                let full_at = ((y + 5) * 32 + x + 7) * 8;
                let crop_at = (y * 12 + x) * 8;
                assert_eq!(&crop[crop_at..crop_at + 8], &full[full_at..full_at + 8]);
            }
        }
        for pixel in full.chunks_exact(8) {
            assert_eq!(half_to_f32(u16::from_le_bytes([pixel[6], pixel[7]])), 1.0);
        }
    }
}

#[cfg(test)]
mod black_white_regressions {
    use super::*;
    use crate::compositor::bloom::f32_to_half;
    #[test]
    fn grain_preserves_black_white_and_transparency_at_large_device_scale() {
        for pixel in [[0.0; 4], [0.0, 0.0, 0.0, 1.0], [1.0; 4], [0.5; 4]] {
            let source = pixel
                .into_iter()
                .flat_map(|channel| f32_to_half(channel).to_le_bytes())
                .cycle()
                .take(8 * 64)
                .collect::<Vec<_>>();
            for seed in [7, 123] {
                assert_eq!(
                    apply_film_grain_f16(
                        &source,
                        8,
                        8,
                        8,
                        8,
                        [0, 0],
                        [0, 0],
                        FilmGrainParams {
                            seed,
                            amount: 0.5,
                            size: 320.0
                        }
                    )
                    .unwrap(),
                    source
                );
            }
        }
    }
}

#[cfg(test)]
mod display_mean_regressions {
    use super::*;
    use crate::compositor::{
        bloom::{f32_to_half, half_to_f32},
        reference::{extended_srgb_to_working, working_to_extended_srgb},
    };

    #[test]
    fn grain_keeps_display_gray_means_within_one_code_value_at_all_amounts() {
        const SIDE: u32 = 128;
        for gray in [0, 1, 8, 32, 64, 128, 192, 254, 255] {
            let working = extended_srgb_to_working([gray as f32 / 255.0; 3]).unwrap();
            let source = [working[0], working[1], working[2], 1.0]
                .into_iter()
                .flat_map(|c| f32_to_half(c).to_le_bytes())
                .cycle()
                .take((SIDE * SIDE * 8) as usize)
                .collect::<Vec<_>>();
            for amount in [0.1, 0.25, 0.5, 1.0] {
                let mut sum = 0.0_f64;
                for seed in [7, 38] {
                    let output = apply_film_grain_f16(
                        &source,
                        SIDE,
                        SIDE,
                        SIDE,
                        SIDE,
                        [0, 0],
                        [0, 0],
                        FilmGrainParams {
                            seed,
                            amount,
                            size: 2.0,
                        },
                    )
                    .unwrap();
                    for p in output.chunks_exact(8) {
                        let c =
                            |i: usize| half_to_f32(u16::from_le_bytes([p[i * 2], p[i * 2 + 1]]));
                        let display = working_to_extended_srgb([c(0), c(1), c(2)]).unwrap();
                        assert!(display.iter().all(|v| *v >= -0.001 && *v <= 1.001));
                        assert_eq!(c(3), 1.0);
                        sum += f64::from(display[0]) * 255.0;
                    }
                }
                let mean = sum / f64::from(2 * SIDE * SIDE);
                assert!(
                    (mean - f64::from(gray)).abs() < 1.0,
                    "gray={gray}, amount={amount}, mean={mean}"
                );
            }
        }
    }
}
