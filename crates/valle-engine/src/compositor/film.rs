//! Stable monochrome film grain on premultiplied working-linear RGBA16F pixels.

use super::bloom::{BloomError, encode_output, quantize_half, read_base};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FilmGrainParams {
    pub seed: u32,
    /// Fraction of the available sRGB display-code headroom used by signed grain.
    pub amount: f32,
    /// Noise lattice spacing in device pixels.
    pub size: f32,
}

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
    for (index, pixel) in pixels.iter_mut().enumerate() {
        let alpha = pixel[3];
        if alpha == 0.0 {
            continue;
        }
        let x = i64::from(output_origin[0]) + (index % width) as i64;
        let y = i64::from(output_origin[1]) + (index / width) as i64;
        let noise = lattice_noise(x, y, params.seed, f64::from(params.size));
        // Construct symmetric noise in display encoding, where perceived gray
        // levels are reviewed. Limiting amplitude to both endpoint margins keeps
        // the distribution symmetric without clipping (including at amount=1).
        let mut encoded = super::reference::working_to_extended_srgb([
            pixel[0] / alpha,
            pixel[1] / alpha,
            pixel[2] / alpha,
        ])
        .map_err(|_| BloomError::InvalidParameters)?;
        for channel in &mut encoded {
            let margin = channel.min(1.0 - *channel).max(0.0);
            *channel += (noise * f64::from(params.amount) * f64::from(margin)) as f32;
        }
        let working = super::reference::extended_srgb_to_working(encoded)
            .map_err(|_| BloomError::InvalidParameters)?;
        for (channel, value) in pixel[..3].iter_mut().zip(working) {
            *channel = quantize_half(f64::from(value * alpha));
        }
    }
    Ok(encode_output(&pixels))
}

fn lattice_noise(x: i64, y: i64, seed: u32, size: f64) -> f64 {
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
