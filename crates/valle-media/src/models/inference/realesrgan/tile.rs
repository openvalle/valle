use anyhow::{Context, Result, ensure};

use crate::models::inference::realesrgan::frame::{ImageFrame, ImageView, PixelFormat, byte_len};
use crate::models::inference::realesrgan::{SCALE, TileConfig};

/// One neural graph output in NCHW layout.
#[derive(Debug)]
pub struct TileOutput {
    pub shape: Vec<usize>,
    pub data: Vec<f32>,
}

/// Backend boundary used by the shared pre/post-processing adapter.
///
/// Implementations receive one RGB `f32` NCHW tile in `[0,1]`. Calls are sequential, allowing a
/// backend to reuse one loaded session and fixed scratch buffers without retaining video frames.
pub trait TileBackend {
    fn infer_tile(&mut self, input: &[f32], shape: [usize; 4]) -> Result<TileOutput>;
}

pub(crate) fn upscale_tiled(
    input: ImageView<'_>,
    config: TileConfig,
    backend: &mut impl TileBackend,
) -> Result<(ImageFrame, usize)> {
    config.validate()?;
    let source_width = input.width() as usize;
    let source_height = input.height() as usize;
    let tile_width = config.width as usize;
    let tile_height = config.height as usize;
    let scale = SCALE as usize;
    let scaled_width = input
        .width()
        .checked_mul(SCALE)
        .context("scaled image width exceeds u32")?;
    let scaled_height = input
        .height()
        .checked_mul(SCALE)
        .context("scaled image height exceeds u32")?;
    let output_width = scaled_width as usize;
    let output_height = scaled_height as usize;
    let output_pixels = output_width
        .checked_mul(output_height)
        .context("scaled image dimensions overflow addressable memory")?;
    let sum_values = output_pixels
        .checked_mul(3)
        .context("scaled RGB accumulator is too large")?;

    let xs = tile_starts(source_width, tile_width, config.overlap as usize)?;
    let ys = tile_starts(source_height, tile_height, config.overlap as usize)?;
    let x_weights = axis_weights(&xs, tile_width, scale);
    let y_weights = axis_weights(&ys, tile_height, scale);
    let mut sums = vec![0.0_f32; sum_values];
    let mut weight_sums = vec![0.0_f32; output_pixels];
    let mut tile_count = 0;

    for (yi, &y) in ys.iter().enumerate() {
        let valid_height = tile_height.min(source_height - y);
        for (xi, &x) in xs.iter().enumerate() {
            let valid_width = tile_width.min(source_width - x);
            let (model_input, left, top) = fixed_canvas_nchw(
                input,
                x,
                y,
                valid_width,
                valid_height,
                tile_width,
                tile_height,
            );
            let prediction = backend.infer_tile(&model_input, [1, 3, tile_height, tile_width])?;
            let expected_shape = vec![1, 3, tile_height * scale, tile_width * scale];
            ensure!(
                prediction.shape == expected_shape,
                "tile backend returned shape {:?}, expected {:?}",
                prediction.shape,
                expected_shape
            );
            let expected_values = expected_shape.iter().product::<usize>();
            ensure!(
                prediction.data.len() == expected_values,
                "tile backend returned {} values, expected {expected_values}",
                prediction.data.len()
            );
            ensure!(
                prediction.data.iter().all(|value| value.is_finite()),
                "tile backend returned NaN or Inf"
            );

            composite_tile(
                &prediction.data,
                tile_width,
                tile_height,
                valid_width,
                valid_height,
                left,
                top,
                x,
                y,
                output_width,
                &x_weights[xi],
                &y_weights[yi],
                &mut sums,
                &mut weight_sums,
            );
            tile_count += 1;
        }
    }

    ensure!(
        weight_sums.iter().all(|weight| *weight > 0.0),
        "tile compositor left uncovered pixels"
    );
    let format = input.format();
    let mut pixels = vec![0_u8; byte_len(scaled_width, scaled_height, format)?];
    let channels = format.channels();
    for output_y in 0..output_height {
        for output_x in 0..output_width {
            let pixel = output_y * output_width + output_x;
            let weight = weight_sums[pixel];
            let destination = pixel * channels;
            for channel in 0..3 {
                let value = (sums[pixel * 3 + channel] / weight).clamp(0.0, 1.0);
                pixels[destination + channel] = (value * 255.0).round_ties_even() as u8;
            }
            if format == PixelFormat::Rgba8 {
                pixels[destination + 3] = input.alpha_at(output_x / scale, output_y / scale);
            }
        }
    }
    Ok((
        ImageFrame::new(scaled_width, scaled_height, format, pixels)?,
        tile_count,
    ))
}

fn tile_starts(length: usize, tile: usize, overlap: usize) -> Result<Vec<usize>> {
    ensure!(length > 0 && tile > 0, "length and tile must be positive");
    ensure!(
        overlap < tile,
        "overlap must be smaller than each tile axis"
    );
    if length <= tile {
        return Ok(vec![0]);
    }
    let stride = tile - overlap;
    let last = length - tile;
    let mut starts = (0..=last).step_by(stride).collect::<Vec<_>>();
    if starts.last().copied() != Some(last) {
        starts.push(last);
    }
    Ok(starts)
}

fn axis_weights(starts: &[usize], tile: usize, scale: usize) -> Vec<Vec<f32>> {
    starts
        .iter()
        .enumerate()
        .map(|(index, &start)| {
            let mut weights = vec![1.0_f32; tile * scale];
            if index > 0 {
                let overlap = (starts[index - 1] + tile - start) * scale;
                for (position, weight) in weights[..overlap].iter_mut().enumerate() {
                    *weight = (position + 1) as f32 / (overlap + 1) as f32;
                }
            }
            if index + 1 < starts.len() {
                let overlap = (start + tile - starts[index + 1]) * scale;
                let offset = weights.len() - overlap;
                for (position, weight) in weights[offset..].iter_mut().enumerate() {
                    *weight = 1.0 - (position + 1) as f32 / (overlap + 1) as f32;
                }
            }
            weights
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn fixed_canvas_nchw(
    input: ImageView<'_>,
    source_x: usize,
    source_y: usize,
    valid_width: usize,
    valid_height: usize,
    tile_width: usize,
    tile_height: usize,
) -> (Vec<f32>, usize, usize) {
    let left = (tile_width - valid_width) / 2;
    let top = (tile_height - valid_height) / 2;
    let plane = tile_width * tile_height;
    let mut result = vec![0.0_f32; plane * 3];
    for canvas_y in 0..tile_height {
        let local_y = symmetric_index(canvas_y as isize - top as isize, valid_height);
        for canvas_x in 0..tile_width {
            let local_x = symmetric_index(canvas_x as isize - left as isize, valid_width);
            let rgb = input.rgb_at(source_x + local_x, source_y + local_y);
            let position = canvas_y * tile_width + canvas_x;
            for channel in 0..3 {
                result[channel * plane + position] = f32::from(rgb[channel]) / 255.0;
            }
        }
    }
    (result, left, top)
}

fn symmetric_index(index: isize, length: usize) -> usize {
    debug_assert!(length > 0);
    if length == 1 {
        return 0;
    }
    let period = (length * 2) as isize;
    let position = index.rem_euclid(period) as usize;
    if position < length {
        position
    } else {
        length * 2 - 1 - position
    }
}

#[allow(clippy::too_many_arguments)]
fn composite_tile(
    prediction: &[f32],
    tile_width: usize,
    tile_height: usize,
    valid_width: usize,
    valid_height: usize,
    left: usize,
    top: usize,
    source_x: usize,
    source_y: usize,
    output_width: usize,
    x_weights: &[f32],
    y_weights: &[f32],
    sums: &mut [f32],
    weight_sums: &mut [f32],
) {
    let scale = SCALE as usize;
    let model_width = tile_width * scale;
    let model_height = tile_height * scale;
    let plane = model_width * model_height;
    let crop_x = left * scale;
    let crop_y = top * scale;
    for local_y in 0..valid_height * scale {
        let model_y = crop_y + local_y;
        let output_y = source_y * scale + local_y;
        for local_x in 0..valid_width * scale {
            let model_x = crop_x + local_x;
            let output_x = source_x * scale + local_x;
            let output_pixel = output_y * output_width + output_x;
            let weight = y_weights[model_y] * x_weights[model_x];
            let model_pixel = model_y * model_width + model_x;
            for channel in 0..3 {
                sums[output_pixel * 3 + channel] +=
                    prediction[channel * plane + model_pixel] * weight;
            }
            weight_sums[output_pixel] += weight;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NearestX4;

    impl TileBackend for NearestX4 {
        fn infer_tile(&mut self, input: &[f32], shape: [usize; 4]) -> Result<TileOutput> {
            let [batch, channels, height, width] = shape;
            ensure!(batch == 1 && channels == 3, "unexpected test shape");
            let output_width = width * SCALE as usize;
            let output_height = height * SCALE as usize;
            let input_plane = width * height;
            let output_plane = output_width * output_height;
            let mut data = vec![0.0; output_plane * 3];
            for channel in 0..3 {
                for y in 0..output_height {
                    for x in 0..output_width {
                        data[channel * output_plane + y * output_width + x] = input[channel
                            * input_plane
                            + (y / SCALE as usize) * width
                            + x / SCALE as usize];
                    }
                }
            }
            Ok(TileOutput {
                shape: vec![1, 3, output_height, output_width],
                data,
            })
        }
    }

    #[test]
    fn starts_cover_axis_and_edge_align_last_tile() {
        assert_eq!(tile_starts(29, 11, 3).unwrap(), [0, 8, 16, 18]);
        assert!(tile_starts(10, 10, 10).is_err());
    }

    #[test]
    fn overlapping_tiles_have_no_rgb_seams_or_geometry_drift() {
        let width = 29_u32;
        let height = 17_u32;
        let input = (0..width as usize * height as usize * 3)
            .map(|index| ((index * 37 + 11) % 256) as u8)
            .collect::<Vec<_>>();
        let view = ImageView::rgb8(width, height, &input).unwrap();
        let config = TileConfig::new(11, 7, 3).unwrap();
        let (output, tiles) = upscale_tiled(view, config, &mut NearestX4).unwrap();
        assert_eq!(
            (output.width(), output.height()),
            (width * SCALE, height * SCALE)
        );
        assert!(tiles > 1);
        for y in 0..output.height() as usize {
            for x in 0..output.width() as usize {
                let source = ((y / SCALE as usize) * width as usize + x / SCALE as usize) * 3;
                let destination = (y * output.width() as usize + x) * 3;
                assert_eq!(
                    &output.pixels()[destination..destination + 3],
                    &input[source..source + 3]
                );
            }
        }
    }

    #[test]
    fn short_axes_are_symmetrically_padded_then_cropped() {
        let input = [10_u8, 20, 30, 40, 50, 60];
        let view = ImageView::rgb8(2, 1, &input).unwrap();
        let config = TileConfig::new(5, 4, 1).unwrap();
        let (output, tiles) = upscale_tiled(view, config, &mut NearestX4).unwrap();
        assert_eq!(tiles, 1);
        for y in 0..SCALE as usize {
            for x in 0..(2 * SCALE) as usize {
                let source = (x / SCALE as usize) * 3;
                let destination = (y * output.width() as usize + x) * 3;
                assert_eq!(
                    &output.pixels()[destination..destination + 3],
                    &input[source..source + 3]
                );
            }
        }
    }

    #[test]
    fn rgba_alpha_is_nearest_scaled_and_never_sent_to_model() {
        let input = [1_u8, 2, 3, 0, 4, 5, 6, 64, 7, 8, 9, 128, 10, 11, 12, 255];
        let view = ImageView::rgba8(2, 2, &input).unwrap();
        let config = TileConfig::new(2, 2, 0).unwrap();
        let (output, _) = upscale_tiled(view, config, &mut NearestX4).unwrap();
        assert_eq!(output.format(), PixelFormat::Rgba8);
        for y in 0..output.height() as usize {
            for x in 0..output.width() as usize {
                let expected = input[((y / 4) * 2 + x / 4) * 4 + 3];
                assert_eq!(output.pixels()[(y * 8 + x) * 4 + 3], expected);
            }
        }
    }

    #[test]
    fn output_is_clamped_after_compositing() {
        struct OutOfRange;
        impl TileBackend for OutOfRange {
            fn infer_tile(&mut self, _input: &[f32], shape: [usize; 4]) -> Result<TileOutput> {
                let values = 3 * shape[2] * SCALE as usize * shape[3] * SCALE as usize;
                let mut data = vec![2.0_f32; values];
                data[0] = -1.0;
                Ok(TileOutput {
                    shape: vec![1, 3, shape[2] * SCALE as usize, shape[3] * SCALE as usize],
                    data,
                })
            }
        }
        let input = [0_u8, 0, 0];
        let view = ImageView::rgb8(1, 1, &input).unwrap();
        let config = TileConfig::new(1, 1, 0).unwrap();
        let (output, _) = upscale_tiled(view, config, &mut OutOfRange).unwrap();
        assert_eq!(output.pixels()[0], 0);
        assert!(output.pixels()[1..].iter().all(|value| *value == 255));
    }
}
