use std::f64::consts::PI;

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

use crate::models::inference::lama::frame::{BinaryMaskView, ImageFrame, ImageView};
use crate::models::inference::lama::plan::{
    CanvasBucket, InpaintPlan, InpaintTask, Rect, plan_inpaint,
};

/// Tensor backend boundary shared by ONNX and deterministic mock tests.
///
/// The adapter owns tensor names and layouts. Backends receive one NCHW
/// `[1,4,H,W]` f32 tensor and must fill the supplied `[1,3,H,W]` output buffer.
pub trait LamaBackend {
    fn infer(
        &mut self,
        input: &[f32],
        input_shape: [usize; 4],
        output: &mut [f32],
    ) -> Result<[usize; 4]>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterMetrics {
    pub tasks: usize,
    pub scaled_tasks: usize,
    pub planned_bucket: Option<CanvasBucket>,
    pub actual_bucket: CanvasBucket,
}

/// Backend-neutral LaMa crop, canvas, tensor, and composite adapter.
///
/// Scratch vectors are retained and reused across calls, but no source or
/// result frame is cached. Their capacity is bounded by the largest frame seen
/// by this session and the fixed selected canvas.
pub struct LamaAdapter {
    bucket: CanvasBucket,
    canvas_rgb: Vec<u8>,
    canvas_mask: Vec<u8>,
    input: Vec<f32>,
    model_output: Vec<f32>,
    crop_rgb: Vec<u8>,
    crop_mask: Vec<u8>,
    scaled_rgb: Vec<u8>,
    scaled_mask: Vec<u8>,
    restored_rgb: Vec<u8>,
}

impl LamaAdapter {
    pub fn new(bucket: CanvasBucket) -> Result<Self> {
        let plane = canvas_pixels(bucket)?;
        Ok(Self {
            bucket,
            canvas_rgb: vec![0; checked_mul(plane, 3, "canvas RGB")?],
            canvas_mask: vec![0; plane],
            input: vec![0.0; checked_mul(plane, 4, "model input")?],
            model_output: vec![0.0; checked_mul(plane, 3, "model output")?],
            crop_rgb: Vec::new(),
            crop_mask: Vec::new(),
            scaled_rgb: Vec::new(),
            scaled_mask: Vec::new(),
            restored_rgb: Vec::new(),
        })
    }

    pub const fn bucket(&self) -> CanvasBucket {
        self.bucket
    }

    pub fn inpaint(
        &mut self,
        backend: &mut impl LamaBackend,
        image: ImageView<'_>,
        mask: BinaryMaskView<'_>,
    ) -> Result<(ImageFrame, InpaintPlan, AdapterMetrics)> {
        let plan = plan_inpaint(mask)?;
        self.inpaint_planned(backend, image, mask, &plan)
    }

    /// Execute a previously inspected plan and reject stale input or a plan
    /// whose required bucket does not fit the loaded session canvas.
    pub fn inpaint_planned(
        &mut self,
        backend: &mut impl LamaBackend,
        image: ImageView<'_>,
        mask: BinaryMaskView<'_>,
        plan: &InpaintPlan,
    ) -> Result<(ImageFrame, InpaintPlan, AdapterMetrics)> {
        ensure!(
            image.width() == mask.width() && image.height() == mask.height(),
            "LaMa image {}x{} and mask {}x{} must have identical dimensions",
            image.width(),
            image.height(),
            mask.width(),
            mask.height()
        );
        let actual_plan = plan_inpaint(mask)?;
        ensure!(
            &actual_plan == plan,
            "LaMa inpaint plan is stale or belongs to a different mask"
        );
        if let Some(planned_bucket) = plan.bucket {
            ensure!(
                planned_bucket.width() <= self.bucket.width()
                    && planned_bucket.height() <= self.bucket.height(),
                "LaMa plan requires bucket {:?} ({}x{}), which does not fit the loaded session bucket {:?} ({}x{})",
                planned_bucket,
                planned_bucket.width(),
                planned_bucket.height(),
                self.bucket,
                self.bucket.width(),
                self.bucket.height()
            );
        }

        let mut pixels = image.pixels().to_vec();
        let mut scaled_tasks = 0;
        for task in &plan.tasks {
            if self.prepare_task(&pixels, image, mask, *task)? {
                scaled_tasks += 1;
            }
            let shape = [
                1,
                4,
                self.bucket.height() as usize,
                self.bucket.width() as usize,
            ];
            self.model_output.fill(f32::NAN);
            let output_shape = backend.infer(&self.input, shape, &mut self.model_output)?;
            let expected_shape = [shape[0], 3, shape[2], shape[3]];
            ensure!(
                output_shape == expected_shape,
                "LaMa backend returned shape {output_shape:?}, expected {expected_shape:?}"
            );
            ensure!(
                self.model_output.iter().all(|value| value.is_finite()),
                "LaMa backend returned NaN or Inf; refusing to composite corrupt output"
            );
            self.restore_task(*task)?;
            composite_task(&mut pixels, image, mask, *task, &self.restored_rgb);
        }

        let frame = ImageFrame::new(image.width(), image.height(), image.format(), pixels)?;
        let metrics = AdapterMetrics {
            tasks: plan.tasks.len(),
            scaled_tasks,
            planned_bucket: plan.bucket,
            actual_bucket: self.bucket,
        };
        Ok((frame, actual_plan, metrics))
    }

    /// Fixed allocations plus retained variable scratch capacities, in bytes.
    pub fn scratch_capacity_bytes(&self) -> usize {
        self.canvas_rgb.capacity()
            + self.canvas_mask.capacity()
            + self.input.capacity() * size_of::<f32>()
            + self.model_output.capacity() * size_of::<f32>()
            + self.crop_rgb.capacity()
            + self.crop_mask.capacity()
            + self.scaled_rgb.capacity()
            + self.scaled_mask.capacity()
            + self.restored_rgb.capacity()
    }

    fn prepare_task(
        &mut self,
        current: &[u8],
        image: ImageView<'_>,
        mask: BinaryMaskView<'_>,
        task: InpaintTask,
    ) -> Result<bool> {
        extract_task(
            current,
            image,
            mask,
            task,
            &mut self.crop_rgb,
            &mut self.crop_mask,
        )?;

        let (scaled_width, scaled_height, scaled) = scaled_geometry(task.crop, self.bucket);
        if scaled {
            resize_area_rgb(
                &self.crop_rgb,
                task.crop.width as usize,
                task.crop.height as usize,
                scaled_width,
                scaled_height,
                &mut self.scaled_rgb,
            )?;
            resize_nearest_mask(
                &self.crop_mask,
                task.crop.width as usize,
                task.crop.height as usize,
                scaled_width,
                scaled_height,
                &mut self.scaled_mask,
            )?;
        } else {
            self.scaled_rgb.clear();
            self.scaled_rgb.extend_from_slice(&self.crop_rgb);
            self.scaled_mask.clear();
            self.scaled_mask.extend_from_slice(&self.crop_mask);
        }

        fit_symmetric_canvas(
            &self.scaled_rgb,
            &self.scaled_mask,
            (scaled_width, scaled_height),
            (self.bucket.width() as usize, self.bucket.height() as usize),
            &mut self.canvas_rgb,
            &mut self.canvas_mask,
        );
        build_nchw_input(&self.canvas_rgb, &self.canvas_mask, &mut self.input);
        Ok(scaled)
    }

    fn restore_task(&mut self, task: InpaintTask) -> Result<()> {
        let canvas_width = self.bucket.width() as usize;
        let plane = canvas_pixels(self.bucket)?;
        let (scaled_width, scaled_height, scaled) = scaled_geometry(task.crop, self.bucket);
        let scaled_len = checked_mul(
            checked_mul(scaled_width, scaled_height, "scaled output pixels")?,
            3,
            "scaled output RGB",
        )?;
        self.scaled_rgb.resize(scaled_len, 0);
        for y in 0..scaled_height {
            for x in 0..scaled_width {
                let source_position = y * canvas_width + x;
                let destination = (y * scaled_width + x) * 3;
                for channel in 0..3 {
                    let value = self.model_output[channel * plane + source_position];
                    self.scaled_rgb[destination + channel] = (value.clamp(0.0, 1.0) * 255.0) as u8;
                }
            }
        }

        if scaled {
            resize_lanczos4_rgb(
                &self.scaled_rgb,
                scaled_width,
                scaled_height,
                task.crop.width as usize,
                task.crop.height as usize,
                &mut self.restored_rgb,
            )?;
        } else {
            self.restored_rgb.clear();
            self.restored_rgb.extend_from_slice(&self.scaled_rgb);
        }
        Ok(())
    }
}

fn canvas_pixels(bucket: CanvasBucket) -> Result<usize> {
    checked_mul(
        bucket.width() as usize,
        bucket.height() as usize,
        "canvas pixels",
    )
}

fn checked_mul(left: usize, right: usize, label: &str) -> Result<usize> {
    left.checked_mul(right)
        .with_context(|| format!("{label} size overflows addressable memory"))
}

fn extract_task(
    current: &[u8],
    image: ImageView<'_>,
    mask: BinaryMaskView<'_>,
    task: InpaintTask,
    crop_rgb: &mut Vec<u8>,
    crop_mask: &mut Vec<u8>,
) -> Result<()> {
    let pixels = checked_mul(
        task.crop.width as usize,
        task.crop.height as usize,
        "crop pixels",
    )?;
    crop_rgb.resize(checked_mul(pixels, 3, "crop RGB")?, 0);
    crop_mask.resize(pixels, 0);
    let image_width = image.width() as usize;
    let channels = image.format().channels();

    for local_y in 0..task.crop.height as usize {
        for local_x in 0..task.crop.width as usize {
            let x = task.crop.x as usize + local_x;
            let y = task.crop.y as usize + local_y;
            let source = (y * image_width + x) * channels;
            let destination = (local_y * task.crop.width as usize + local_x) * 3;
            crop_rgb[destination..destination + 3].copy_from_slice(&current[source..source + 3]);
            if contains(task.mask_region, x as u32, y as u32) {
                crop_mask[local_y * task.crop.width as usize + local_x] =
                    mask.pixels()[y * image_width + x];
            }
        }
    }
    Ok(())
}

fn contains(rect: Rect, x: u32, y: u32) -> bool {
    x >= rect.x && x < rect.right() && y >= rect.y && y < rect.bottom()
}

fn scaled_geometry(crop: Rect, bucket: CanvasBucket) -> (usize, usize, bool) {
    let scale = (bucket.width() as f64 / crop.width as f64)
        .min(bucket.height() as f64 / crop.height as f64)
        .min(1.0);
    if scale < 1.0 {
        (
            ((crop.width as f64 * scale) as usize).max(8),
            ((crop.height as f64 * scale) as usize).max(8),
            true,
        )
    } else {
        (crop.width as usize, crop.height as usize, false)
    }
}

fn fit_symmetric_canvas(
    rgb: &[u8],
    mask: &[u8],
    source_size: (usize, usize),
    canvas_size: (usize, usize),
    canvas_rgb: &mut [u8],
    canvas_mask: &mut [u8],
) {
    let (source_width, source_height) = source_size;
    let (canvas_width, canvas_height) = canvas_size;
    debug_assert!(source_width <= canvas_width && source_height <= canvas_height);
    for y in 0..canvas_height {
        let source_y = symmetric_index(y, source_height);
        for x in 0..canvas_width {
            let source_x = symmetric_index(x, source_width);
            let source = (source_y * source_width + source_x) * 3;
            let destination = (y * canvas_width + x) * 3;
            canvas_rgb[destination..destination + 3].copy_from_slice(&rgb[source..source + 3]);
            let position = y * canvas_width + x;
            canvas_mask[position] = if x < source_width && y < source_height {
                mask[y * source_width + x]
            } else {
                0
            };
        }
    }
}

fn symmetric_index(index: usize, length: usize) -> usize {
    debug_assert!(length > 0);
    let period = length * 2;
    let position = index % period;
    if position < length {
        position
    } else {
        period - 1 - position
    }
}

fn build_nchw_input(canvas_rgb: &[u8], canvas_mask: &[u8], input: &mut [f32]) {
    let plane = canvas_mask.len();
    debug_assert_eq!(canvas_rgb.len(), plane * 3);
    debug_assert_eq!(input.len(), plane * 4);
    for position in 0..plane {
        let masked = canvas_mask[position] == 255;
        for channel in 0..3 {
            input[channel * plane + position] = if masked {
                0.0
            } else {
                f32::from(canvas_rgb[position * 3 + channel]) / 255.0
            };
        }
        input[3 * plane + position] = f32::from(masked);
    }
}

fn composite_task(
    output: &mut [u8],
    image: ImageView<'_>,
    mask: BinaryMaskView<'_>,
    task: InpaintTask,
    restored_rgb: &[u8],
) {
    let width = image.width() as usize;
    let channels = image.format().channels();
    for local_y in 0..task.crop.height as usize {
        for local_x in 0..task.crop.width as usize {
            let x = task.crop.x as usize + local_x;
            let y = task.crop.y as usize + local_y;
            if contains(task.mask_region, x as u32, y as u32) && mask.pixels()[y * width + x] == 255
            {
                let source = (local_y * task.crop.width as usize + local_x) * 3;
                let destination = (y * width + x) * channels;
                output[destination..destination + 3]
                    .copy_from_slice(&restored_rgb[source..source + 3]);
            }
        }
    }
}

fn resize_area_rgb(
    source: &[u8],
    source_width: usize,
    source_height: usize,
    destination_width: usize,
    destination_height: usize,
    destination: &mut Vec<u8>,
) -> Result<()> {
    let destination_pixels =
        checked_mul(destination_width, destination_height, "area-resize pixels")?;
    destination.resize(checked_mul(destination_pixels, 3, "area-resize RGB")?, 0);
    let scale_x = source_width as f64 / destination_width as f64;
    let scale_y = source_height as f64 / destination_height as f64;
    for dy in 0..destination_height {
        let y0 = dy as f64 * scale_y;
        let y1 = (dy + 1) as f64 * scale_y;
        let sy_start = y0.floor() as usize;
        let sy_end = y1.ceil().min(source_height as f64) as usize;
        for dx in 0..destination_width {
            let x0 = dx as f64 * scale_x;
            let x1 = (dx + 1) as f64 * scale_x;
            let sx_start = x0.floor() as usize;
            let sx_end = x1.ceil().min(source_width as f64) as usize;
            let mut sums = [0.0_f64; 3];
            let mut total_weight = 0.0;
            for sy in sy_start..sy_end {
                let wy = (y1.min((sy + 1) as f64) - y0.max(sy as f64)).max(0.0);
                for sx in sx_start..sx_end {
                    let wx = (x1.min((sx + 1) as f64) - x0.max(sx as f64)).max(0.0);
                    let weight = wx * wy;
                    let offset = (sy * source_width + sx) * 3;
                    for channel in 0..3 {
                        sums[channel] += f64::from(source[offset + channel]) * weight;
                    }
                    total_weight += weight;
                }
            }
            let output = (dy * destination_width + dx) * 3;
            for channel in 0..3 {
                destination[output + channel] =
                    (sums[channel] / total_weight).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    Ok(())
}

fn resize_nearest_mask(
    source: &[u8],
    source_width: usize,
    source_height: usize,
    destination_width: usize,
    destination_height: usize,
    destination: &mut Vec<u8>,
) -> Result<()> {
    destination.resize(
        checked_mul(destination_width, destination_height, "nearest-mask pixels")?,
        0,
    );
    for y in 0..destination_height {
        let source_y = (y * source_height / destination_height).min(source_height - 1);
        for x in 0..destination_width {
            let source_x = (x * source_width / destination_width).min(source_width - 1);
            destination[y * destination_width + x] = source[source_y * source_width + source_x];
        }
    }
    Ok(())
}

fn resize_lanczos4_rgb(
    source: &[u8],
    source_width: usize,
    source_height: usize,
    destination_width: usize,
    destination_height: usize,
    destination: &mut Vec<u8>,
) -> Result<()> {
    let destination_pixels = checked_mul(
        destination_width,
        destination_height,
        "Lanczos-resize pixels",
    )?;
    destination.resize(checked_mul(destination_pixels, 3, "Lanczos-resize RGB")?, 0);

    for dy in 0..destination_height {
        let source_y = (dy as f64 + 0.5) * source_height as f64 / destination_height as f64 - 0.5;
        let base_y = source_y.floor() as isize;
        for dx in 0..destination_width {
            let source_x = (dx as f64 + 0.5) * source_width as f64 / destination_width as f64 - 0.5;
            let base_x = source_x.floor() as isize;
            let mut sums = [0.0_f64; 3];
            let mut total_weight = 0.0;
            for tap_y in -3..=4 {
                let sample_y = (base_y + tap_y).clamp(0, source_height as isize - 1) as usize;
                let wy = lanczos4(source_y - (base_y + tap_y) as f64);
                for tap_x in -3..=4 {
                    let sample_x = (base_x + tap_x).clamp(0, source_width as isize - 1) as usize;
                    let weight = wy * lanczos4(source_x - (base_x + tap_x) as f64);
                    let offset = (sample_y * source_width + sample_x) * 3;
                    for channel in 0..3 {
                        sums[channel] += f64::from(source[offset + channel]) * weight;
                    }
                    total_weight += weight;
                }
            }
            let output = (dy * destination_width + dx) * 3;
            for channel in 0..3 {
                destination[output + channel] =
                    (sums[channel] / total_weight).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    Ok(())
}

fn lanczos4(distance: f64) -> f64 {
    let distance = distance.abs();
    if distance < f64::EPSILON {
        1.0
    } else if distance >= 4.0 {
        0.0
    } else {
        let x = PI * distance;
        (x.sin() / x) * ((x / 4.0).sin() / (x / 4.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::inference::lama::frame::{BinaryMaskView, ImageView};
    use crate::models::inference::lama::plan::PlanStrategy;

    #[derive(Default)]
    struct SolidBackend {
        calls: usize,
        input_addresses: Vec<usize>,
        input_shapes: Vec<[usize; 4]>,
        non_finite: bool,
        wrong_shape: bool,
    }

    impl LamaBackend for SolidBackend {
        fn infer(
            &mut self,
            input: &[f32],
            input_shape: [usize; 4],
            output: &mut [f32],
        ) -> Result<[usize; 4]> {
            self.calls += 1;
            self.input_addresses.push(input.as_ptr() as usize);
            self.input_shapes.push(input_shape);
            let plane = input_shape[2] * input_shape[3];
            assert_eq!(input.len(), plane * 4);
            for position in 0..plane {
                output[position] = 1.0;
                output[plane + position] = 0.0;
                output[2 * plane + position] = 0.0;
            }
            if self.non_finite {
                output[0] = f32::NAN;
            }
            Ok(if self.wrong_shape {
                [1, 3, 1, 1]
            } else {
                [1, 3, input_shape[2], input_shape[3]]
            })
        }
    }

    #[test]
    fn tensor_layout_composite_and_rgba_alpha_policy_are_explicit() {
        let image_bytes = [10, 20, 30, 41, 40, 50, 60, 99];
        let mask_bytes = [0, 255];
        let image = ImageView::rgba8(2, 1, &image_bytes).unwrap();
        let mask = BinaryMaskView::gray8(2, 1, &mask_bytes).unwrap();
        let mut adapter = LamaAdapter::new(CanvasBucket::Square256).unwrap();

        struct InspectBackend;
        impl LamaBackend for InspectBackend {
            fn infer(
                &mut self,
                input: &[f32],
                shape: [usize; 4],
                output: &mut [f32],
            ) -> Result<[usize; 4]> {
                assert_eq!(shape, [1, 4, 256, 256]);
                let plane = 256 * 256;
                assert_eq!(input[0], 10.0 / 255.0);
                assert_eq!(input[plane], 20.0 / 255.0);
                assert_eq!(input[2 * plane], 30.0 / 255.0);
                assert_eq!(input[3 * plane], 0.0);
                assert_eq!(input[1], 0.0);
                assert_eq!(input[plane + 1], 0.0);
                assert_eq!(input[2 * plane + 1], 0.0);
                assert_eq!(input[3 * plane + 1], 1.0);
                // Symmetric RGB padding repeats the edge, while mask padding is zero.
                assert_eq!(input[2], 40.0 / 255.0);
                assert_eq!(input[plane + 2], 50.0 / 255.0);
                assert_eq!(input[3 * plane + 2], 0.0);
                for position in 0..plane {
                    output[position] = 1.0;
                    output[plane + position] = 0.0;
                    output[2 * plane + position] = 0.0;
                }
                Ok([1, 3, 256, 256])
            }
        }

        let (frame, plan, metrics) = adapter.inpaint(&mut InspectBackend, image, mask).unwrap();
        assert_eq!(plan.strategy, PlanStrategy::Single);
        assert_eq!(metrics.tasks, 1);
        assert_eq!(frame.pixels(), &[10, 20, 30, 41, 255, 0, 0, 99]);
        // Alpha is never passed to LaMa and is preserved byte-for-byte, including masked pixels.
        assert_eq!(frame.pixels()[3], 41);
        assert_eq!(frame.pixels()[7], 99);
    }

    #[test]
    fn empty_mask_is_exact_noop_without_backend_call() {
        let image_bytes = [1, 2, 3, 4, 5, 6];
        let mask_bytes = [0, 0];
        let image = ImageView::rgb8(2, 1, &image_bytes).unwrap();
        let mask = BinaryMaskView::gray8(2, 1, &mask_bytes).unwrap();
        let mut adapter = LamaAdapter::new(CanvasBucket::Square256).unwrap();
        let mut backend = SolidBackend::default();
        let (frame, plan, _) = adapter.inpaint(&mut backend, image, mask).unwrap();
        assert_eq!(plan.strategy, PlanStrategy::Noop);
        assert_eq!(backend.calls, 0);
        assert_eq!(frame.pixels(), image_bytes);
    }

    #[test]
    fn bucket_mismatch_reports_planned_and_loaded_shapes() {
        let image_bytes = vec![64; 640 * 384 * 3];
        let mut mask_bytes = vec![0; 640 * 384];
        for y in 40..340 {
            for x in 100..540 {
                mask_bytes[y * 640 + x] = 255;
            }
        }
        let image = ImageView::rgb8(640, 384, &image_bytes).unwrap();
        let mask = BinaryMaskView::gray8(640, 384, &mask_bytes).unwrap();
        let mut adapter = LamaAdapter::new(CanvasBucket::Square256).unwrap();
        let error = adapter
            .inpaint(&mut SolidBackend::default(), image, mask)
            .unwrap_err()
            .to_string();
        assert!(error.contains("640x384"));
        assert!(error.contains("256x256"));
    }

    #[test]
    fn wide_adapter_runs_smaller_plan_on_loaded_shape_without_growing() {
        let wide_image_bytes = vec![64; 640 * 384 * 3];
        let mut wide_mask_bytes = vec![0; 640 * 384];
        for y in 40..340 {
            for x in 100..540 {
                wide_mask_bytes[y * 640 + x] = 255;
            }
        }
        let wide_image = ImageView::rgb8(640, 384, &wide_image_bytes).unwrap();
        let wide_mask = BinaryMaskView::gray8(640, 384, &wide_mask_bytes).unwrap();
        let mut adapter = LamaAdapter::new(CanvasBucket::Wide640x384).unwrap();
        let mut backend = SolidBackend::default();
        let (_, first_plan, first_metrics) = adapter
            .inpaint(&mut backend, wide_image, wide_mask)
            .unwrap();
        assert_eq!(first_plan.bucket, Some(CanvasBucket::Wide640x384));
        assert_eq!(first_metrics.actual_bucket, CanvasBucket::Wide640x384);
        let first_capacity = adapter.scratch_capacity_bytes();

        let small_image_bytes = [10, 20, 30];
        let small_mask_bytes = [255];
        let small_image = ImageView::rgb8(1, 1, &small_image_bytes).unwrap();
        let small_mask = BinaryMaskView::gray8(1, 1, &small_mask_bytes).unwrap();
        let (frame, second_plan, second_metrics) = adapter
            .inpaint(&mut backend, small_image, small_mask)
            .unwrap();

        assert_eq!(second_plan.bucket, Some(CanvasBucket::Square256));
        assert_eq!(second_metrics.planned_bucket, Some(CanvasBucket::Square256));
        assert_eq!(second_metrics.actual_bucket, CanvasBucket::Wide640x384);
        assert_eq!(second_metrics.scaled_tasks, 0);
        assert_eq!(frame.pixels(), &[255, 0, 0]);
        assert_eq!(backend.calls, 2);
        assert_eq!(backend.input_shapes, [[1, 4, 384, 640], [1, 4, 384, 640]]);
        assert_eq!(backend.input_addresses[0], backend.input_addresses[1]);
        assert_eq!(adapter.scratch_capacity_bytes(), first_capacity);
    }

    #[test]
    fn non_finite_or_wrong_shape_output_is_rejected() {
        let image_bytes = [1, 2, 3];
        let mask_bytes = [255];
        let image = ImageView::rgb8(1, 1, &image_bytes).unwrap();
        let mask = BinaryMaskView::gray8(1, 1, &mask_bytes).unwrap();
        let mut adapter = LamaAdapter::new(CanvasBucket::Square256).unwrap();
        let mut non_finite = SolidBackend {
            non_finite: true,
            ..SolidBackend::default()
        };
        assert!(adapter.inpaint(&mut non_finite, image, mask).is_err());

        let mut wrong_shape = SolidBackend {
            wrong_shape: true,
            ..SolidBackend::default()
        };
        assert!(adapter.inpaint(&mut wrong_shape, image, mask).is_err());
    }

    #[test]
    fn sparse_tasks_are_sequential_and_preserve_every_unmasked_pixel() {
        let width = 640_u32;
        let height = 640_u32;
        let image_bytes = vec![17; width as usize * height as usize * 3];
        let mut mask_bytes = vec![0; width as usize * height as usize];
        for &(x, y) in &[(10, 10), (620, 10), (10, 620), (620, 620)] {
            mask_bytes[y * width as usize + x] = 255;
        }
        let image = ImageView::rgb8(width, height, &image_bytes).unwrap();
        let mask = BinaryMaskView::gray8(width, height, &mask_bytes).unwrap();
        let mut adapter = LamaAdapter::new(CanvasBucket::Square256).unwrap();
        let mut backend = SolidBackend::default();
        let (frame, plan, metrics) = adapter.inpaint(&mut backend, image, mask).unwrap();
        assert_eq!(plan.strategy, PlanStrategy::Partitioned);
        assert_eq!(backend.calls, 4);
        assert_eq!(metrics.tasks, 4);
        for (position, &masked) in mask_bytes.iter().enumerate() {
            let rgb = &frame.pixels()[position * 3..position * 3 + 3];
            if masked == 255 {
                assert_eq!(rgb, &[255, 0, 0]);
            } else {
                assert_eq!(rgb, &[17, 17, 17]);
            }
        }
    }

    #[test]
    fn fixed_tensor_buffers_are_reused_across_frames() {
        let image_bytes = [10, 20, 30];
        let mask_bytes = [255];
        let image = ImageView::rgb8(1, 1, &image_bytes).unwrap();
        let mask = BinaryMaskView::gray8(1, 1, &mask_bytes).unwrap();
        let mut adapter = LamaAdapter::new(CanvasBucket::Square256).unwrap();
        let mut backend = SolidBackend::default();
        adapter.inpaint(&mut backend, image, mask).unwrap();
        let first_capacity = adapter.scratch_capacity_bytes();
        adapter.inpaint(&mut backend, image, mask).unwrap();
        assert_eq!(backend.input_addresses.len(), 2);
        assert_eq!(backend.input_addresses[0], backend.input_addresses[1]);
        assert_eq!(adapter.scratch_capacity_bytes(), first_capacity);
    }
}
