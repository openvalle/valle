use anyhow::{Context, Result, ensure};

use crate::models::inference::rife::frame::{ImageFrame, ImageView, PixelFormat, byte_len};
use crate::models::inference::rife::{InterpolationPosition, PAD_MULTIPLE};

/// Raw neural output returned by a backend implementation.
#[derive(Debug)]
pub struct ModelOutput {
    pub shape: Vec<usize>,
    pub data: Vec<f32>,
}

/// Tensor backend boundary. Media tools should use [`crate::models::inference::rife::RifeSession`] and never this tensor
/// interface directly; it exists so ONNX and future CoreML sessions share one exact adapter.
pub trait RifeBackend {
    fn infer(
        &mut self,
        frames: &[f32],
        frames_shape: [usize; 4],
        timestep: f32,
    ) -> Result<ModelOutput>;
}

#[derive(Debug, Clone, Copy)]
pub struct AdapterOptions {
    /// Reject a pair before allocation when the right/bottom padded canvas exceeds this area.
    pub max_padded_pixels: usize,
}

impl Default for AdapterOptions {
    fn default() -> Self {
        Self {
            // Fits a 3840x2160 frame after padding to 3840x2176.
            max_padded_pixels: 8_388_608,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RifeAdapter {
    options: AdapterOptions,
}

impl RifeAdapter {
    pub fn new(options: AdapterOptions) -> Result<Self> {
        ensure!(
            options.max_padded_pixels > 0,
            "max_padded_pixels must be positive"
        );
        Ok(Self { options })
    }

    pub const fn options(&self) -> AdapterOptions {
        self.options
    }

    /// Interpolate one pair and release every pair-specific tensor before returning.
    ///
    /// RGB is passed to the model as straight, unpremultiplied color. If either endpoint is RGBA,
    /// output is straight RGBA: missing RGB alpha is treated as 255 and endpoint alpha values are
    /// linearly interpolated at the same pixel using `position`. Alpha is deliberately not passed
    /// through the RGB-trained neural graph and is not motion-compensated.
    pub fn interpolate(
        &self,
        backend: &mut impl RifeBackend,
        frame0: ImageView<'_>,
        frame1: ImageView<'_>,
        position: InterpolationPosition,
    ) -> Result<(ImageFrame, PaddedGeometry)> {
        ensure!(
            frame0.width() == frame1.width() && frame0.height() == frame1.height(),
            "RIFE endpoint frames must have identical dimensions"
        );
        let geometry = PaddedGeometry::new(frame0.width(), frame0.height())?;
        ensure!(
            geometry.padded_pixels() <= self.options.max_padded_pixels,
            "RIFE padded canvas {}x{} has {} pixels, exceeding max_padded_pixels={}",
            geometry.padded_width,
            geometry.padded_height,
            geometry.padded_pixels(),
            self.options.max_padded_pixels
        );
        let input = concatenate_padded_nchw(frame0, frame1, geometry)?;
        let output = backend.infer(
            &input,
            [
                1,
                6,
                geometry.padded_height as usize,
                geometry.padded_width as usize,
            ],
            position.get(),
        )?;
        let expected_shape = vec![
            1,
            3,
            geometry.padded_height as usize,
            geometry.padded_width as usize,
        ];
        ensure!(
            output.shape == expected_shape,
            "RIFE backend returned shape {:?}, expected {:?}",
            output.shape,
            expected_shape
        );
        let expected_values = expected_shape.iter().product::<usize>();
        ensure!(
            output.data.len() == expected_values,
            "RIFE backend returned {} values, expected {expected_values}",
            output.data.len()
        );
        ensure!(
            output.data.iter().all(|value| value.is_finite()),
            "RIFE backend returned NaN or Inf"
        );
        let frame = crop_and_quantize(output.data, frame0, frame1, geometry, position)?;
        Ok((frame, geometry))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaddedGeometry {
    pub source_width: u32,
    pub source_height: u32,
    pub padded_width: u32,
    pub padded_height: u32,
}

impl PaddedGeometry {
    fn new(source_width: u32, source_height: u32) -> Result<Self> {
        Ok(Self {
            source_width,
            source_height,
            padded_width: round_up(source_width, PAD_MULTIPLE)?,
            padded_height: round_up(source_height, PAD_MULTIPLE)?,
        })
    }

    pub fn padded_pixels(self) -> usize {
        self.padded_width as usize * self.padded_height as usize
    }
}

fn round_up(value: u32, multiple: u32) -> Result<u32> {
    debug_assert!(value > 0 && multiple > 0);
    value
        .checked_add(multiple - 1)
        .map(|value| value / multiple * multiple)
        .context("padded image dimension exceeds u32")
}

fn concatenate_padded_nchw(
    frame0: ImageView<'_>,
    frame1: ImageView<'_>,
    geometry: PaddedGeometry,
) -> Result<Vec<f32>> {
    let plane = geometry.padded_pixels();
    let values = plane
        .checked_mul(6)
        .context("RIFE input tensor size overflows addressable memory")?;
    let mut input = vec![0.0_f32; values];
    let source_width = geometry.source_width as usize;
    let source_height = geometry.source_height as usize;
    let padded_width = geometry.padded_width as usize;
    for y in 0..source_height {
        for x in 0..source_width {
            let position = y * padded_width + x;
            for (frame_index, frame) in [frame0, frame1].into_iter().enumerate() {
                let rgb = frame.rgb_at(x, y);
                for channel in 0..3 {
                    input[(frame_index * 3 + channel) * plane + position] =
                        f32::from(rgb[channel]) / 255.0;
                }
            }
        }
    }
    Ok(input)
}

fn crop_and_quantize(
    output: Vec<f32>,
    frame0: ImageView<'_>,
    frame1: ImageView<'_>,
    geometry: PaddedGeometry,
    position: InterpolationPosition,
) -> Result<ImageFrame> {
    let format = if frame0.format() == PixelFormat::Rgba8 || frame1.format() == PixelFormat::Rgba8 {
        PixelFormat::Rgba8
    } else {
        PixelFormat::Rgb8
    };
    let mut pixels = vec![0_u8; byte_len(geometry.source_width, geometry.source_height, format)?];
    let plane = geometry.padded_pixels();
    let width = geometry.source_width as usize;
    let height = geometry.source_height as usize;
    let padded_width = geometry.padded_width as usize;
    let channels = format.channels();
    let timestep = position.get();
    for y in 0..height {
        for x in 0..width {
            let source_position = y * padded_width + x;
            let destination = (y * width + x) * channels;
            for channel in 0..3 {
                let value = output[channel * plane + source_position].clamp(0.0, 1.0);
                pixels[destination + channel] = (value * 255.0).round_ties_even() as u8;
            }
            if format == PixelFormat::Rgba8 {
                let alpha0 = f32::from(frame0.alpha_at(x, y));
                let alpha1 = f32::from(frame1.alpha_at(x, y));
                pixels[destination + 3] =
                    (alpha0 * (1.0 - timestep) + alpha1 * timestep).round_ties_even() as u8;
            }
        }
    }
    ImageFrame::new(
        geometry.source_width,
        geometry.source_height,
        format,
        pixels,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct BlendBackend {
        calls: usize,
    }

    impl RifeBackend for BlendBackend {
        fn infer(
            &mut self,
            frames: &[f32],
            shape: [usize; 4],
            timestep: f32,
        ) -> Result<ModelOutput> {
            self.calls += 1;
            assert_eq!(shape, [1, 6, 128, 128]);
            let plane = shape[2] * shape[3];
            let mut data = vec![0.0; plane * 3];
            for channel in 0..3 {
                for position in 0..plane {
                    data[channel * plane + position] = frames[channel * plane + position]
                        * (1.0 - timestep)
                        + frames[(channel + 3) * plane + position] * timestep;
                }
            }
            Ok(ModelOutput {
                shape: vec![1, 3, 128, 128],
                data,
            })
        }
    }

    #[test]
    fn pads_right_and_bottom_without_resizing_or_reordering_channels() {
        let frame0 = [10_u8, 20, 30, 40, 50, 60];
        let frame1 = [110_u8, 120, 130, 140, 150, 160];
        let first = ImageView::rgb8(2, 1, &frame0).unwrap();
        let second = ImageView::rgb8(2, 1, &frame1).unwrap();
        let position = InterpolationPosition::new(0.25).unwrap();
        let mut backend = BlendBackend::default();
        let (output, geometry) = RifeAdapter::default()
            .interpolate(&mut backend, first, second, position)
            .unwrap();
        assert_eq!(
            geometry,
            PaddedGeometry {
                source_width: 2,
                source_height: 1,
                padded_width: 128,
                padded_height: 128,
            }
        );
        assert_eq!(output.pixels(), &[35, 45, 55, 65, 75, 85]);
        assert_eq!(backend.calls, 1);
    }

    #[test]
    fn nchw_layout_and_zero_padding_are_exact() {
        let frame0 = ImageView::rgb8(2, 1, &[10, 20, 30, 40, 50, 60]).unwrap();
        let frame1 = ImageView::rgba8(2, 1, &[70, 80, 90, 1, 100, 110, 120, 254]).unwrap();
        let geometry = PaddedGeometry::new(2, 1).unwrap();
        let input = concatenate_padded_nchw(frame0, frame1, geometry).unwrap();
        let plane = 128 * 128;
        let close = |left: f32, right: f32| (left - right).abs() < 1e-7;
        assert!(close(input[0], 10.0 / 255.0));
        assert!(close(input[plane], 20.0 / 255.0));
        assert!(close(input[2 * plane], 30.0 / 255.0));
        assert!(close(input[3 * plane], 70.0 / 255.0));
        assert!(close(input[4 * plane], 80.0 / 255.0));
        assert!(close(input[5 * plane], 90.0 / 255.0));
        assert!(close(input[1], 40.0 / 255.0));
        for channel in 0..6 {
            assert_eq!(input[channel * plane + 2], 0.0, "right padding");
            assert_eq!(input[channel * plane + 128], 0.0, "bottom padding");
        }
    }

    #[test]
    fn straight_alpha_is_blended_independently_and_rgb_is_not_premultiplied() {
        let first = ImageView::rgba8(1, 1, &[200, 100, 50, 0]).unwrap();
        let second = ImageView::rgb8(1, 1, &[100, 50, 0]).unwrap();
        let mut backend = BlendBackend::default();
        let (output, _) = RifeAdapter::default()
            .interpolate(
                &mut backend,
                first,
                second,
                InterpolationPosition::new(0.5).unwrap(),
            )
            .unwrap();
        assert_eq!(output.format(), PixelFormat::Rgba8);
        assert_eq!(output.pixels(), &[150, 75, 25, 128]);
    }

    #[test]
    fn one_adapter_processes_ordered_pairs_without_retaining_sequence_state() {
        let frame0 = ImageView::rgb8(1, 1, &[0, 10, 20]).unwrap();
        let frame1 = ImageView::rgb8(1, 1, &[20, 30, 40]).unwrap();
        let frame2 = ImageView::rgb8(1, 1, &[40, 50, 60]).unwrap();
        let mut backend = BlendBackend::default();
        let adapter = RifeAdapter::default();
        let position = InterpolationPosition::new(0.5).unwrap();
        let first = adapter
            .interpolate(&mut backend, frame0, frame1, position)
            .unwrap()
            .0;
        let second = adapter
            .interpolate(&mut backend, frame1, frame2, position)
            .unwrap()
            .0;
        assert_eq!(first.pixels(), &[10, 20, 30]);
        assert_eq!(second.pixels(), &[30, 40, 50]);
        assert_eq!(backend.calls, 2);
    }

    #[test]
    fn mismatched_geometry_and_memory_budget_fail_before_inference() {
        let small = ImageView::rgb8(1, 1, &[0, 0, 0]).unwrap();
        let wide = ImageView::rgb8(2, 1, &[0; 6]).unwrap();
        let mut backend = BlendBackend::default();
        assert!(
            RifeAdapter::default()
                .interpolate(
                    &mut backend,
                    small,
                    wide,
                    InterpolationPosition::new(0.5).unwrap(),
                )
                .is_err()
        );
        let adapter = RifeAdapter::new(AdapterOptions {
            max_padded_pixels: 1,
        })
        .unwrap();
        assert!(
            adapter
                .interpolate(
                    &mut backend,
                    small,
                    small,
                    InterpolationPosition::new(0.5).unwrap(),
                )
                .is_err()
        );
        assert_eq!(backend.calls, 0);
    }

    #[test]
    fn invalid_model_output_is_rejected() {
        struct InvalidBackend;
        impl RifeBackend for InvalidBackend {
            fn infer(
                &mut self,
                _frames: &[f32],
                _shape: [usize; 4],
                _timestep: f32,
            ) -> Result<ModelOutput> {
                Ok(ModelOutput {
                    shape: vec![1, 3, 1, 1],
                    data: vec![f32::NAN; 3],
                })
            }
        }
        let frame = ImageView::rgb8(1, 1, &[0, 0, 0]).unwrap();
        assert!(
            RifeAdapter::default()
                .interpolate(
                    &mut InvalidBackend,
                    frame,
                    frame,
                    InterpolationPosition::new(0.5).unwrap(),
                )
                .is_err()
        );
    }
}
