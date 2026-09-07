use anyhow::{Context, Result, ensure};

use crate::models::inference::omnishotcut::{FRAME_BYTES, MODEL_HEIGHT, MODEL_WIDTH};

const RGBA_CHANNELS: usize = 4;
const RGB_CHANNELS: usize = 3;
const WEIGHT_ONE: u32 = 1 << 16;

#[derive(Debug, Clone, Copy)]
struct AxisSample {
    low: usize,
    high: usize,
    high_weight: u32,
}

/// Stateful raw-frame preprocessor for the model's packed u8 THWC boundary.
///
/// The source is tightly packed straight-alpha RGBA8 in display order. Alpha is
/// ignored, RGB channel order is preserved, and half-pixel bilinear sampling
/// produces one 128x96 packed RGB8 frame. The ONNX graph owns the remaining
/// numeric normalization, so the model boundary deliberately remains u8
/// `[0, 255]` rather than applying a second host-side normalization.
pub(crate) struct RgbaPreprocessor {
    source_dimensions: Option<(usize, usize)>,
    horizontal: Vec<AxisSample>,
    vertical: Vec<AxisSample>,
}

impl RgbaPreprocessor {
    pub(crate) fn new() -> Self {
        Self {
            source_dimensions: None,
            horizontal: Vec::new(),
            vertical: Vec::new(),
        }
    }

    /// Validate and resize one source frame into reusable model storage.
    pub(crate) fn resize_rgba_into(
        &mut self,
        width: usize,
        height: usize,
        source: &[u8],
        output: &mut Vec<u8>,
    ) -> Result<()> {
        ensure!(
            width > 0 && height > 0,
            "OmniShotCut source dimensions must be positive"
        );
        let expected = width
            .checked_mul(height)
            .and_then(|pixels| pixels.checked_mul(RGBA_CHANNELS))
            .context("OmniShotCut source frame dimensions overflow")?;
        ensure!(
            source.len() == expected,
            "OmniShotCut source RGBA frame has {} bytes; tightly packed {width}x{height}x4 requires {expected}",
            source.len()
        );

        match self.source_dimensions {
            None => {
                self.horizontal = axis_samples(width, MODEL_WIDTH);
                self.vertical = axis_samples(height, MODEL_HEIGHT);
                self.source_dimensions = Some((width, height));
            }
            Some(dimensions) => ensure!(
                dimensions == (width, height),
                "OmniShotCut source dimensions changed from {}x{} to {width}x{height} within one stream",
                dimensions.0,
                dimensions.1
            ),
        }

        output.resize(FRAME_BYTES, 0);
        if width == MODEL_WIDTH && height == MODEL_HEIGHT {
            for (rgba, rgb) in source
                .chunks_exact(RGBA_CHANNELS)
                .zip(output.chunks_exact_mut(RGB_CHANNELS))
            {
                rgb.copy_from_slice(&rgba[..RGB_CHANNELS]);
            }
            return Ok(());
        }

        let denominator = u64::from(WEIGHT_ONE) * u64::from(WEIGHT_ONE);
        for (target_y, y) in self.vertical.iter().copied().enumerate() {
            for (target_x, x) in self.horizontal.iter().copied().enumerate() {
                for channel in 0..RGB_CHANNELS {
                    let top_left = source[(y.low * width + x.low) * RGBA_CHANNELS + channel];
                    let top_right = source[(y.low * width + x.high) * RGBA_CHANNELS + channel];
                    let bottom_left = source[(y.high * width + x.low) * RGBA_CHANNELS + channel];
                    let bottom_right = source[(y.high * width + x.high) * RGBA_CHANNELS + channel];
                    let top = interpolate_axis(top_left, top_right, x.high_weight);
                    let bottom = interpolate_axis(bottom_left, bottom_right, x.high_weight);
                    let value = (top * u64::from(WEIGHT_ONE - y.high_weight)
                        + bottom * u64::from(y.high_weight)
                        + denominator / 2)
                        / denominator;
                    output[(target_y * MODEL_WIDTH + target_x) * RGB_CHANNELS + channel] =
                        value as u8;
                }
            }
        }
        Ok(())
    }

    pub(crate) fn reset_stream(&mut self) {
        self.source_dimensions = None;
        self.horizontal.clear();
        self.vertical.clear();
    }
}

fn interpolate_axis(low: u8, high: u8, high_weight: u32) -> u64 {
    u64::from(low) * u64::from(WEIGHT_ONE - high_weight) + u64::from(high) * u64::from(high_weight)
}

fn axis_samples(source: usize, target: usize) -> Vec<AxisSample> {
    debug_assert!(source > 0 && target > 0);
    (0..target)
        .map(|destination| {
            let position = ((destination as f64 + 0.5) * source as f64 / target as f64 - 0.5)
                .clamp(0.0, (source - 1) as f64);
            let low = position.floor() as usize;
            let high = (low + 1).min(source - 1);
            let high_weight = if high == low {
                0
            } else {
                ((position - low as f64) * f64::from(WEIGHT_ONE))
                    .floor()
                    .clamp(0.0, f64::from(WEIGHT_ONE - 1)) as u32
            };
            AxisSample {
                low,
                high,
                high_weight,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_geometry_preserves_rgb_order_ignores_alpha_and_reuses_storage() {
        let mut source = vec![0; MODEL_WIDTH * MODEL_HEIGHT * RGBA_CHANNELS];
        for (index, pixel) in source.chunks_exact_mut(RGBA_CHANNELS).enumerate() {
            pixel.copy_from_slice(&[
                (index % 251) as u8,
                (index % 239) as u8,
                (index % 233) as u8,
                (index % 227) as u8,
            ]);
        }
        let expected = source
            .chunks_exact(RGBA_CHANNELS)
            .flat_map(|pixel| [pixel[0], pixel[1], pixel[2]])
            .collect::<Vec<_>>();
        let mut preprocessor = RgbaPreprocessor::new();
        let mut output = Vec::new();
        preprocessor
            .resize_rgba_into(MODEL_WIDTH, MODEL_HEIGHT, &source, &mut output)
            .unwrap();
        assert_eq!(output, expected);
        let pointer = output.as_ptr();

        source.fill(0);
        for pixel in source.chunks_exact_mut(RGBA_CHANNELS) {
            pixel.copy_from_slice(&[5, 7, 11, 255]);
        }
        preprocessor
            .resize_rgba_into(MODEL_WIDTH, MODEL_HEIGHT, &source, &mut output)
            .unwrap();
        assert_eq!(output.as_ptr(), pointer);
        assert!(output.chunks_exact(3).all(|pixel| pixel == [5, 7, 11]));
    }

    #[test]
    fn source_layout_and_midstream_geometry_are_strict() {
        let mut preprocessor = RgbaPreprocessor::new();
        let mut output = Vec::new();
        assert!(
            preprocessor
                .resize_rgba_into(0, 2, &[], &mut output)
                .is_err()
        );
        assert!(
            preprocessor
                .resize_rgba_into(usize::MAX, 2, &[], &mut output)
                .is_err()
        );
        assert!(
            preprocessor
                .resize_rgba_into(2, 2, &[0; 15], &mut output)
                .is_err()
        );

        preprocessor
            .resize_rgba_into(2, 2, &[0; 16], &mut output)
            .unwrap();
        assert!(
            preprocessor
                .resize_rgba_into(3, 2, &[0; 24], &mut output)
                .is_err()
        );
    }

    #[test]
    fn reset_starts_a_new_geometry_contract() {
        let mut preprocessor = RgbaPreprocessor::new();
        let mut output = Vec::new();
        preprocessor
            .resize_rgba_into(2, 2, &[0; 16], &mut output)
            .unwrap();
        preprocessor.reset_stream();
        preprocessor
            .resize_rgba_into(3, 2, &[0; 24], &mut output)
            .unwrap();
        assert_eq!(output.len(), FRAME_BYTES);
    }

    #[test]
    fn resize_uses_half_pixel_bilinear_sampling() {
        let source = [0, 0, 0, 1, 100, 0, 0, 2, 0, 100, 0, 3, 100, 100, 0, 4];
        let mut preprocessor = RgbaPreprocessor::new();
        let mut output = Vec::new();
        preprocessor
            .resize_rgba_into(2, 2, &source, &mut output)
            .unwrap();
        let center = ((MODEL_HEIGHT / 2) * MODEL_WIDTH + MODEL_WIDTH / 2) * RGB_CHANNELS;
        assert!((49..=51).contains(&output[center]));
        assert!((49..=51).contains(&output[center + 1]));
        assert_eq!(output[center + 2], 0);
    }
}
