use std::collections::VecDeque;
use std::time::Instant;

use anyhow::{Context, Result, ensure};

use crate::models::inference::edgetam::{
    AdapterOptions, Alpha8Matte, CANVAS_SIDE, DecodedFrame, EMBEDDING_DIM, EMBEDDING_GRID,
    EMPTY_SLOT_BIAS, EdgeTamBackend, EdgeTamConstants, FULL_MEMORY_FROM_FRAME, FrameMetrics,
    Gray8Mask, KEY_VALUE_TOKENS, LOW_SIDE, MEMORY_SLOTS, MEMORY_TOKENS_PER_FRAME, NO_OBJECT_LOGIT,
    POINTER_TOKENS, PointPrompt, PresentationTimestamp, Rgba8Frame, STABILITY_DELTA,
    STABILITY_THRESHOLD, SegmentationFrame, StateOccupancy,
};

const RECENT_MEMORY_FRAMES: usize = MEMORY_SLOTS - 1;
const RECENT_POINTER_FRAMES: usize = POINTER_TOKENS / 4 - 1;

/// A load-once backend and immutable artifact constants. Per-job state is never stored here.
pub struct EdgeTamAdapter<B> {
    backend: B,
    constants: EdgeTamConstants,
    options: AdapterOptions,
}

impl<B: EdgeTamBackend> EdgeTamAdapter<B> {
    pub fn new(backend: B, constants: EdgeTamConstants, options: AdapterOptions) -> Result<Self> {
        ensure!(
            options.mask_threshold.is_finite(),
            "mask threshold must be finite"
        );
        ensure!(
            options.max_source_pixels > 0,
            "max_source_pixels must be positive"
        );
        Ok(Self {
            backend,
            constants,
            options,
        })
    }

    pub const fn options(&self) -> AdapterOptions {
        self.options
    }

    /// Start an isolated tracking job while reusing the already-loaded model sessions.
    /// Dropping this value discards every memory-bank and PTS entry for the job.
    pub fn start_session(&mut self) -> EdgeTamSession<'_, B> {
        EdgeTamSession {
            backend: &mut self.backend,
            constants: &self.constants,
            options: self.options,
            state: SessionState::default(),
        }
    }
}

/// Strictly ordered state machine for one prompted object in one video.
pub struct EdgeTamSession<'a, B> {
    backend: &'a mut B,
    constants: &'a EdgeTamConstants,
    options: AdapterOptions,
    state: SessionState,
}

impl<B: EdgeTamBackend> EdgeTamSession<'_, B> {
    /// Initialize frame zero. Exactly one prompt is accepted for the lifetime of this session.
    pub fn initialize(
        &mut self,
        pts: PresentationTimestamp,
        frame: Rgba8Frame<'_>,
        prompt: &PointPrompt,
    ) -> Result<SegmentationFrame> {
        ensure!(
            self.state.next_frame == 0,
            "EdgeTAM session is already initialized"
        );
        self.process(pts, frame, Some(prompt))
    }

    /// Propagate the initialized object to the next frame. PTS must strictly increase.
    pub fn track(
        &mut self,
        pts: PresentationTimestamp,
        frame: Rgba8Frame<'_>,
    ) -> Result<SegmentationFrame> {
        ensure!(
            self.state.next_frame > 0,
            "EdgeTAM session must be initialized before tracking"
        );
        self.process(pts, frame, None)
    }

    pub fn occupancy(&self) -> StateOccupancy {
        self.state.bank.occupancy()
    }

    pub const fn next_frame_index(&self) -> u64 {
        self.state.next_frame
    }

    fn process(
        &mut self,
        pts: PresentationTimestamp,
        frame: Rgba8Frame<'_>,
        prompt: Option<&PointPrompt>,
    ) -> Result<SegmentationFrame> {
        let started = Instant::now();
        let frame_index = self.state.next_frame;
        validate_order_and_geometry(&self.state, pts, frame, self.options.max_source_pixels)?;
        let (coordinates, labels) = if frame_index == 0 {
            let prompt = prompt.context("first EdgeTAM frame requires a point prompt")?;
            scale_prompt(prompt, frame.width(), frame.height())?
        } else {
            ensure!(
                prompt.is_none(),
                "EdgeTAM prompt is allowed only on frame zero"
            );
            (vec![0.0, 0.0], vec![-1])
        };

        let preprocess_started = Instant::now();
        let image = rgba_to_model_nchw(frame)?;
        let preprocess_ms = elapsed_ms(preprocess_started);

        let encode_started = Instant::now();
        let encoded = self.backend.encode(&image)?;
        let encode_ms = elapsed_ms(encode_started);
        validate_encoded(&encoded)?;

        let mut memory_attention_ms = 0.0;
        let conditioned = if frame_index == 0 {
            let mut conditioned = encoded.pixel_features.clone();
            for channel in 0..EMBEDDING_DIM {
                let bias = self.constants.no_memory_embedding[channel];
                let start = channel * EMBEDDING_GRID * EMBEDDING_GRID;
                let end = start + EMBEDDING_GRID * EMBEDDING_GRID;
                for value in &mut conditioned[start..end] {
                    *value += bias;
                }
            }
            conditioned
        } else {
            let current = transpose(
                &encoded.pixel_features,
                EMBEDDING_DIM,
                EMBEDDING_GRID * EMBEDDING_GRID,
            )?;
            let attention_started = Instant::now();
            let attended = if frame_index >= FULL_MEMORY_FROM_FRAME {
                let memory = self.state.bank.assemble_memory(frame_index)?;
                self.backend.memory_attention_hot(&current, &memory)?
            } else {
                let (memory, positions, bias) = self.state.bank.assemble_warm(
                    frame_index,
                    &self.constants.memory_positions,
                    &self.constants.temporal_positions,
                )?;
                self.backend
                    .memory_attention_warm(&current, &memory, &positions, &bias)?
            };
            memory_attention_ms = elapsed_ms(attention_started);
            validate_f32(
                "memory-attention output",
                &attended,
                EMBEDDING_GRID * EMBEDDING_GRID * EMBEDDING_DIM,
            )?;
            transpose(&attended, EMBEDDING_GRID * EMBEDDING_GRID, EMBEDDING_DIM)?
        };

        let decode_started = Instant::now();
        let decoded = self.backend.decode(
            &conditioned,
            &encoded.high_resolution_0,
            &encoded.high_resolution_1,
            &coordinates,
            &labels,
        )?;
        let decode_ms = elapsed_ms(decode_started);
        validate_decoded(&decoded)?;
        let (selected_logits, pointer) = select_mask(
            &decoded,
            frame_index != 0,
            &self.constants.no_object_pointer,
        );

        let memory_logits = resize_bilinear(
            &selected_logits,
            LOW_SIDE,
            LOW_SIDE,
            CANVAS_SIDE,
            CANVAS_SIDE,
        )?;
        let memory_mask = if frame_index == 0 {
            memory_logits
                .iter()
                .map(|value| if *value > 0.0 { 10.0 } else { -10.0 })
                .collect::<Vec<_>>()
        } else {
            memory_logits
                .iter()
                .map(|value| 20.0 / (1.0 + (-value).exp()) - 10.0)
                .collect::<Vec<_>>()
        };
        ensure!(
            memory_mask.iter().all(|value| value.is_finite()),
            "EdgeTAM memory mask contains NaN or Inf"
        );

        let memory_encode_started = Instant::now();
        let encoded_memory = self
            .backend
            .encode_memory(&encoded.pixel_features, &memory_mask)?;
        let memory_encode_ms = elapsed_ms(memory_encode_started);
        validate_f32(
            "memory encoder output",
            &encoded_memory,
            MEMORY_TOKENS_PER_FRAME * 64,
        )?;

        let postprocess_started = Instant::now();
        let source_logits = resize_bilinear(
            &selected_logits,
            LOW_SIDE,
            LOW_SIDE,
            usize::try_from(frame.height())?,
            usize::try_from(frame.width())?,
        )?;
        let alpha = Alpha8Matte {
            width: frame.width(),
            height: frame.height(),
            pixels: source_logits
                .iter()
                .map(|value| logit_to_alpha8(*value))
                .collect(),
        };
        let pixels = source_logits
            .iter()
            .map(|value| {
                if *value > self.options.mask_threshold {
                    255
                } else {
                    0
                }
            })
            .collect::<Vec<_>>();
        debug_assert!(pixels.iter().all(|pixel| matches!(pixel, 0 | 255)));
        let mask = Gray8Mask {
            width: frame.width(),
            height: frame.height(),
            pixels,
        };
        let postprocess_ms = elapsed_ms(postprocess_started);

        // Mutate job state only after every graph output and host allocation was validated.
        self.state
            .bank
            .insert(frame_index, encoded_memory, pointer)?;
        self.state.width = Some(frame.width());
        self.state.height = Some(frame.height());
        self.state.last_pts = Some(pts);
        self.state.next_frame = frame_index
            .checked_add(1)
            .context("EdgeTAM frame index overflowed")?;
        let total_ms = elapsed_ms(started);
        Ok(SegmentationFrame {
            frame_index,
            pts,
            mask,
            alpha,
            metrics: FrameMetrics {
                frame_index,
                preprocess_ms,
                encode_ms,
                memory_attention_ms,
                decode_ms,
                memory_encode_ms,
                postprocess_ms,
                total_ms,
            },
        })
    }
}

fn logit_to_alpha8(logit: f32) -> u8 {
    debug_assert!(logit.is_finite());
    let probability = if logit >= 0.0 {
        1.0 / (1.0 + (-logit).exp())
    } else {
        let exp = logit.exp();
        exp / (1.0 + exp)
    };
    (probability * 255.0).round().clamp(0.0, 255.0) as u8
}

#[derive(Default)]
struct SessionState {
    next_frame: u64,
    last_pts: Option<PresentationTimestamp>,
    width: Option<u32>,
    height: Option<u32>,
    bank: MemoryBank,
}

#[derive(Default)]
struct MemoryBank {
    conditioning_memory: Option<Vec<f32>>,
    recent_memory: VecDeque<(u64, Vec<f32>)>,
    conditioning_pointer: Option<Vec<f32>>,
    recent_pointers: VecDeque<(u64, Vec<f32>)>,
}

impl MemoryBank {
    fn insert(&mut self, frame: u64, memory: Vec<f32>, pointer: Vec<f32>) -> Result<()> {
        validate_f32("stored memory", &memory, MEMORY_TOKENS_PER_FRAME * 64)?;
        validate_f32("stored object pointer", &pointer, EMBEDDING_DIM)?;
        if frame == 0 {
            ensure!(
                self.conditioning_memory.is_none()
                    && self.conditioning_pointer.is_none()
                    && self.recent_memory.is_empty()
                    && self.recent_pointers.is_empty(),
                "conditioning memory was already initialized"
            );
            self.conditioning_memory = Some(memory);
            self.conditioning_pointer = Some(pointer);
        } else {
            ensure!(
                self.conditioning_memory.is_some() && self.conditioning_pointer.is_some(),
                "tracking memory has no conditioning frame"
            );
            self.recent_memory.push_back((frame, memory));
            if self.recent_memory.len() > RECENT_MEMORY_FRAMES {
                self.recent_memory.pop_front();
            }
            self.recent_pointers.push_back((frame, pointer));
            if self.recent_pointers.len() > RECENT_POINTER_FRAMES {
                self.recent_pointers.pop_front();
            }
        }
        Ok(())
    }

    fn occupancy(&self) -> StateOccupancy {
        StateOccupancy {
            memory_frames: usize::from(self.conditioning_memory.is_some())
                + self.recent_memory.len(),
            pointer_frames: usize::from(self.conditioning_pointer.is_some())
                + self.recent_pointers.len(),
            max_memory_frames: MEMORY_SLOTS,
            max_pointer_frames: POINTER_TOKENS / 4,
        }
    }

    fn assemble_memory(&self, frame: u64) -> Result<Vec<f32>> {
        let mut memory = vec![0.0; KEY_VALUE_TOKENS * 64];
        self.fill_memory(frame, &mut memory, None)?;
        Ok(memory)
    }

    fn assemble_warm(
        &self,
        frame: u64,
        memory_positions: &[f32],
        temporal_positions: &[f32],
    ) -> Result<(Vec<f32>, Vec<f32>, Vec<f32>)> {
        validate_f32(
            "memory position constant",
            memory_positions,
            MEMORY_TOKENS_PER_FRAME * 64,
        )?;
        validate_f32(
            "temporal position constant",
            temporal_positions,
            MEMORY_SLOTS * 64,
        )?;
        let mut memory = vec![0.0; KEY_VALUE_TOKENS * 64];
        let mut positions = vec![0.0; KEY_VALUE_TOKENS * 64];
        let mut bias = vec![EMPTY_SLOT_BIAS; KEY_VALUE_TOKENS];
        let records = self.fill_memory(frame, &mut memory, Some(&mut bias))?;
        for (slot, record_frame) in records.into_iter().enumerate() {
            let temporal_position = if record_frame == 0 {
                0
            } else {
                let distance = usize::try_from(frame - record_frame)?;
                ensure!(
                    (1..MEMORY_SLOTS).contains(&distance),
                    "memory frame distance {distance} is outside the release window"
                );
                MEMORY_SLOTS - distance
            };
            let temporal_index = MEMORY_SLOTS - temporal_position - 1;
            let output_start = slot * MEMORY_TOKENS_PER_FRAME * 64;
            for token in 0..MEMORY_TOKENS_PER_FRAME {
                for dimension in 0..64 {
                    positions[output_start + token * 64 + dimension] = memory_positions
                        [token * 64 + dimension]
                        + temporal_positions[temporal_index * 64 + dimension];
                }
            }
        }
        Ok((memory, positions, bias))
    }

    fn fill_memory(
        &self,
        frame: u64,
        memory: &mut [f32],
        mut bias: Option<&mut [f32]>,
    ) -> Result<Vec<u64>> {
        ensure!(
            memory.len() == KEY_VALUE_TOKENS * 64,
            "memory assembly buffer has an invalid length"
        );
        if let Some(mask) = bias.as_deref() {
            ensure!(
                mask.len() == KEY_VALUE_TOKENS,
                "attention bias has an invalid length"
            );
        }
        let conditioning_memory = self
            .conditioning_memory
            .as_deref()
            .context("memory bank has no conditioning frame")?;
        let mut records = Vec::with_capacity(MEMORY_SLOTS);
        records.push((0, conditioning_memory));
        for (record_frame, values) in &self.recent_memory {
            ensure!(*record_frame < frame, "memory bank contains a future frame");
            records.push((*record_frame, values.as_slice()));
        }
        ensure!(
            records.len() <= MEMORY_SLOTS,
            "memory bank exceeded its fixed slot capacity"
        );
        for (slot, (record_frame, values)) in records.iter().enumerate() {
            validate_f32("memory-bank frame", values, MEMORY_TOKENS_PER_FRAME * 64)?;
            let start = slot * MEMORY_TOKENS_PER_FRAME * 64;
            memory[start..start + MEMORY_TOKENS_PER_FRAME * 64].copy_from_slice(values);
            if let Some(mask) = bias.as_deref_mut() {
                mask[slot * MEMORY_TOKENS_PER_FRAME..(slot + 1) * MEMORY_TOKENS_PER_FRAME]
                    .fill(0.0);
            }
            debug_assert!(*record_frame < frame || frame == 0);
        }

        let pointer_start = MEMORY_SLOTS * MEMORY_TOKENS_PER_FRAME;
        let conditioning_pointer = self
            .conditioning_pointer
            .as_deref()
            .context("pointer bank has no conditioning frame")?;
        let pointers = std::iter::once(conditioning_pointer).chain(
            self.recent_pointers
                .iter()
                .rev()
                .map(|(_, pointer)| pointer.as_slice()),
        );
        for (pointer_index, pointer) in pointers.take(POINTER_TOKENS / 4).enumerate() {
            validate_f32("object pointer", pointer, EMBEDDING_DIM)?;
            let token = pointer_index * 4;
            let start = (pointer_start + token) * 64;
            memory[start..start + EMBEDDING_DIM].copy_from_slice(pointer);
            if let Some(mask) = bias.as_deref_mut() {
                mask[pointer_start + token..pointer_start + token + 4].fill(0.0);
            }
        }
        Ok(records
            .into_iter()
            .map(|(record_frame, _)| record_frame)
            .collect())
    }
}

#[cfg(feature = "model-edgetam-onnx")]
pub(crate) fn full_memory_positions(
    memory_positions: &[f32],
    temporal_positions: &[f32],
) -> Result<Vec<f32>> {
    validate_f32(
        "memory position constant",
        memory_positions,
        MEMORY_TOKENS_PER_FRAME * 64,
    )?;
    validate_f32(
        "temporal position constant",
        temporal_positions,
        MEMORY_SLOTS * 64,
    )?;
    let mut positions = vec![0.0; KEY_VALUE_TOKENS * 64];
    for slot in 0..MEMORY_SLOTS {
        let temporal_index = MEMORY_SLOTS - slot - 1;
        let output_start = slot * MEMORY_TOKENS_PER_FRAME * 64;
        for token in 0..MEMORY_TOKENS_PER_FRAME {
            for dimension in 0..64 {
                positions[output_start + token * 64 + dimension] = memory_positions
                    [token * 64 + dimension]
                    + temporal_positions[temporal_index * 64 + dimension];
            }
        }
    }
    Ok(positions)
}

fn validate_order_and_geometry(
    state: &SessionState,
    pts: PresentationTimestamp,
    frame: Rgba8Frame<'_>,
    max_source_pixels: usize,
) -> Result<()> {
    let pixels = usize::try_from(frame.width())?
        .checked_mul(usize::try_from(frame.height())?)
        .context("source dimensions overflow")?;
    ensure!(
        pixels <= max_source_pixels,
        "source frame has {pixels} pixels, limit is {max_source_pixels}"
    );
    if let Some(last_pts) = state.last_pts {
        ensure!(
            pts > last_pts,
            "EdgeTAM PTS must strictly increase: {} <= {}",
            pts.value(),
            last_pts.value()
        );
    }
    if let (Some(width), Some(height)) = (state.width, state.height) {
        ensure!(
            frame.width() == width && frame.height() == height,
            "EdgeTAM frame dimensions changed from {width}x{height} to {}x{}",
            frame.width(),
            frame.height()
        );
    }
    Ok(())
}

fn scale_prompt(prompt: &PointPrompt, width: u32, height: u32) -> Result<(Vec<f32>, Vec<i32>)> {
    let mut coordinates = Vec::with_capacity(6);
    let mut labels = Vec::with_capacity(3);
    for point in prompt.points() {
        ensure!(
            point.x() >= 0.0
                && point.y() >= 0.0
                && point.x() < width as f32
                && point.y() < height as f32,
            "prompt point ({}, {}) is outside {width}x{height}",
            point.x(),
            point.y()
        );
        coordinates.push(point.x() / width as f32 * CANVAS_SIDE as f32);
        coordinates.push(point.y() / height as f32 * CANVAS_SIDE as f32);
        labels.push(point.label().as_i32());
    }
    Ok((coordinates, labels))
}

fn rgba_to_model_nchw(frame: Rgba8Frame<'_>) -> Result<Vec<f32>> {
    let source_width = usize::try_from(frame.width())?;
    let source_height = usize::try_from(frame.height())?;
    let (x0, x1, x_weight) = resize_axis(CANVAS_SIDE, source_width)?;
    let (y0, y1, y_weight) = resize_axis(CANVAS_SIDE, source_height)?;
    let plane = CANVAS_SIDE * CANVAS_SIDE;
    let mut output = vec![0.0; 3 * plane];
    for output_y in 0..CANVAS_SIDE {
        let top = y0[output_y] * source_width;
        let bottom = y1[output_y] * source_width;
        let vertical = y_weight[output_y];
        for output_x in 0..CANVAS_SIDE {
            let left = x0[output_x];
            let right = x1[output_x];
            let horizontal = x_weight[output_x];
            let output_index = output_y * CANVAS_SIDE + output_x;
            for channel in 0..3 {
                let top_left = f32::from(frame.pixels()[(top + left) * 4 + channel]);
                let top_right = f32::from(frame.pixels()[(top + right) * 4 + channel]);
                let bottom_left = f32::from(frame.pixels()[(bottom + left) * 4 + channel]);
                let bottom_right = f32::from(frame.pixels()[(bottom + right) * 4 + channel]);
                let top_value = top_left * (1.0 - horizontal) + top_right * horizontal;
                let bottom_value = bottom_left * (1.0 - horizontal) + bottom_right * horizontal;
                output[channel * plane + output_index] =
                    top_value * (1.0 - vertical) + bottom_value * vertical;
            }
        }
    }
    Ok(output)
}

fn resize_bilinear(
    source: &[f32],
    source_height: usize,
    source_width: usize,
    output_height: usize,
    output_width: usize,
) -> Result<Vec<f32>> {
    ensure!(
        source.len() == source_height * source_width,
        "resize source length does not match its dimensions"
    );
    let (x0, x1, x_weight) = resize_axis(output_width, source_width)?;
    let (y0, y1, y_weight) = resize_axis(output_height, source_height)?;
    let output_len = output_height
        .checked_mul(output_width)
        .context("resize output dimensions overflow")?;
    let mut output = vec![0.0; output_len];
    for output_y in 0..output_height {
        let top = y0[output_y] * source_width;
        let bottom = y1[output_y] * source_width;
        let vertical = y_weight[output_y];
        for output_x in 0..output_width {
            let horizontal = x_weight[output_x];
            let top_value = source[top + x0[output_x]] * (1.0 - horizontal)
                + source[top + x1[output_x]] * horizontal;
            let bottom_value = source[bottom + x0[output_x]] * (1.0 - horizontal)
                + source[bottom + x1[output_x]] * horizontal;
            output[output_y * output_width + output_x] =
                top_value * (1.0 - vertical) + bottom_value * vertical;
        }
    }
    Ok(output)
}

fn resize_axis(output: usize, source: usize) -> Result<(Vec<usize>, Vec<usize>, Vec<f32>)> {
    ensure!(
        source > 0 && output > 0,
        "resize dimensions must be positive"
    );
    let mut lower = vec![0; output];
    let mut upper = vec![0; output];
    let mut weight = vec![0.0; output];
    for index in 0..output {
        let coordinate = (index as f32 + 0.5) * source as f32 / output as f32 - 0.5;
        let floor = coordinate.floor();
        let first = (floor as i64).clamp(0, source as i64 - 1) as usize;
        lower[index] = first;
        upper[index] = (first + 1).min(source - 1);
        weight[index] = (coordinate - first as f32).clamp(0.0, 1.0);
    }
    Ok((lower, upper, weight))
}

fn transpose(source: &[f32], rows: usize, columns: usize) -> Result<Vec<f32>> {
    ensure!(
        source.len() == rows * columns,
        "transpose source length does not match its dimensions"
    );
    let mut output = vec![0.0; source.len()];
    for row in 0..rows {
        for column in 0..columns {
            output[column * rows + row] = source[row * columns + column];
        }
    }
    Ok(output)
}

fn select_mask(
    decoded: &DecodedFrame,
    tracking: bool,
    no_object_pointer: &[f32],
) -> (Vec<f32>, Vec<f32>) {
    let area = LOW_SIDE * LOW_SIDE;
    let object_present = decoded.object_score > 0.0;
    let (mut mask, mut pointer) = if tracking {
        let best = argmax(&decoded.predicted_ious[1..4]);
        (
            decoded.mask_logits[(1 + best) * area..(2 + best) * area].to_vec(),
            decoded.object_pointers[(1 + best) * EMBEDDING_DIM..(2 + best) * EMBEDDING_DIM]
                .to_vec(),
        )
    } else {
        let single = &decoded.mask_logits[..area];
        let inner = single
            .iter()
            .filter(|value| **value > STABILITY_DELTA)
            .count() as f32;
        let union = single
            .iter()
            .filter(|value| **value > -STABILITY_DELTA)
            .count() as f32;
        let stability = if union > 0.0 { inner / union } else { 1.0 };
        let mask = if stability >= STABILITY_THRESHOLD {
            single.to_vec()
        } else {
            let best = argmax(&decoded.predicted_ious[1..4]);
            decoded.mask_logits[(1 + best) * area..(2 + best) * area].to_vec()
        };
        (mask, decoded.object_pointers[..EMBEDDING_DIM].to_vec())
    };
    if !object_present {
        mask.fill(NO_OBJECT_LOGIT);
        pointer.copy_from_slice(no_object_pointer);
    }
    (mask, pointer)
}

fn argmax(values: &[f32]) -> usize {
    values
        .iter()
        .enumerate()
        .fold(
            (0, f32::NEG_INFINITY),
            |(best_index, best_value), (index, value)| {
                if *value > best_value {
                    (index, *value)
                } else {
                    (best_index, best_value)
                }
            },
        )
        .0
}

fn validate_encoded(encoded: &crate::models::inference::edgetam::EncodedFrame) -> Result<()> {
    validate_f32(
        "pixel features",
        &encoded.pixel_features,
        EMBEDDING_DIM * EMBEDDING_GRID * EMBEDDING_GRID,
    )?;
    validate_f32(
        "high-resolution feature 0",
        &encoded.high_resolution_0,
        32 * LOW_SIDE * LOW_SIDE,
    )?;
    validate_f32(
        "high-resolution feature 1",
        &encoded.high_resolution_1,
        64 * 128 * 128,
    )?;
    Ok(())
}

fn validate_decoded(decoded: &DecodedFrame) -> Result<()> {
    validate_f32(
        "decoder masks",
        &decoded.mask_logits,
        4 * LOW_SIDE * LOW_SIDE,
    )?;
    validate_f32("decoder IoUs", &decoded.predicted_ious, 4)?;
    validate_f32(
        "decoder object pointers",
        &decoded.object_pointers,
        4 * EMBEDDING_DIM,
    )?;
    ensure!(
        decoded.object_score.is_finite(),
        "decoder object score is NaN or Inf"
    );
    Ok(())
}

fn validate_f32(name: &str, values: &[f32], expected: usize) -> Result<()> {
    ensure!(
        values.len() == expected,
        "{name} has {} values, expected {expected}",
        values.len()
    );
    ensure!(
        values.iter().all(|value| value.is_finite()),
        "{name} contains NaN or Inf"
    );
    Ok(())
}

fn elapsed_ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1_000.0
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;
    use crate::models::inference::edgetam::{
        DecodedFrame, EdgeTamConstants, EncodedFrame, PointLabel, PromptPoint, Rgba8Frame,
    };

    #[derive(Debug, Default)]
    struct Calls {
        encodes: usize,
        memory_attention_warm: usize,
        memory_attention_hot: usize,
        decodes: Vec<(Vec<f32>, Vec<i32>)>,
        memory_encodes: usize,
        first_rgb: Option<[f32; 3]>,
        invalid_encoded_shape: bool,
    }

    struct MockBackend {
        calls: Rc<RefCell<Calls>>,
    }

    impl EdgeTamBackend for MockBackend {
        fn encode(
            &mut self,
            image: &[f32],
        ) -> Result<crate::models::inference::edgetam::EncodedFrame> {
            ensure!(
                image.len() == 3 * CANVAS_SIDE * CANVAS_SIDE,
                "mock received invalid image"
            );
            let plane = CANVAS_SIDE * CANVAS_SIDE;
            let mut calls = self.calls.borrow_mut();
            calls.encodes += 1;
            calls.first_rgb = Some([image[0], image[plane], image[2 * plane]]);
            let pixel_values = if calls.invalid_encoded_shape {
                1
            } else {
                EMBEDDING_DIM * EMBEDDING_GRID * EMBEDDING_GRID
            };
            Ok(EncodedFrame {
                pixel_features: vec![0.0; pixel_values],
                high_resolution_0: vec![0.0; 32 * LOW_SIDE * LOW_SIDE],
                high_resolution_1: vec![0.0; 64 * 128 * 128],
            })
        }

        fn memory_attention_warm(
            &mut self,
            current: &[f32],
            _memory: &[f32],
            _positions: &[f32],
            _bias: &[f32],
        ) -> Result<Vec<f32>> {
            self.calls.borrow_mut().memory_attention_warm += 1;
            Ok(current.to_vec())
        }

        fn memory_attention_hot(&mut self, current: &[f32], _memory: &[f32]) -> Result<Vec<f32>> {
            self.calls.borrow_mut().memory_attention_hot += 1;
            Ok(current.to_vec())
        }

        fn decode(
            &mut self,
            _pixel_features: &[f32],
            _high_resolution_0: &[f32],
            _high_resolution_1: &[f32],
            coordinates: &[f32],
            labels: &[i32],
        ) -> Result<DecodedFrame> {
            self.calls
                .borrow_mut()
                .decodes
                .push((coordinates.to_vec(), labels.to_vec()));
            let area = LOW_SIDE * LOW_SIDE;
            let mut masks = vec![-1.0; 4 * area];
            masks[..area / 2].fill(1.0);
            masks[2 * area..3 * area].fill(2.0);
            let mut pointers = vec![0.0; 4 * EMBEDDING_DIM];
            for candidate in 0..4 {
                pointers[candidate * EMBEDDING_DIM..(candidate + 1) * EMBEDDING_DIM]
                    .fill(candidate as f32);
            }
            Ok(DecodedFrame {
                mask_logits: masks,
                predicted_ious: vec![0.0, 0.1, 0.9, 0.2],
                object_pointers: pointers,
                object_score: 1.0,
            })
        }

        fn encode_memory(
            &mut self,
            _pixel_features: &[f32],
            _memory_mask: &[f32],
        ) -> Result<Vec<f32>> {
            self.calls.borrow_mut().memory_encodes += 1;
            Ok(vec![0.0; MEMORY_TOKENS_PER_FRAME * 64])
        }
    }

    fn adapter(calls: &Rc<RefCell<Calls>>) -> EdgeTamAdapter<MockBackend> {
        EdgeTamAdapter::new(
            MockBackend {
                calls: Rc::clone(calls),
            },
            EdgeTamConstants::zeros(),
            AdapterOptions::default(),
        )
        .unwrap()
    }

    fn frame(width: u32, height: u32) -> Vec<u8> {
        let mut pixels = vec![0; width as usize * height as usize * 4];
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.copy_from_slice(&[10, 20, 30, 0]);
        }
        pixels
    }

    fn prompt() -> PointPrompt {
        PointPrompt::new([
            PromptPoint::new(1.0, 1.0, PointLabel::Positive).unwrap(),
            PromptPoint::new(0.5, 0.5, PointLabel::Positive).unwrap(),
            PromptPoint::new(0.0, 0.0, PointLabel::Negative).unwrap(),
        ])
        .unwrap()
    }

    #[test]
    fn seed_then_tracking_use_strict_protocol_and_binary_gray8() {
        let calls = Rc::new(RefCell::new(Calls::default()));
        let mut adapter = adapter(&calls);
        let pixels = frame(2, 2);
        let view = Rgba8Frame::new(2, 2, &pixels).unwrap();
        let mut session = adapter.start_session();

        let seed = session
            .initialize(PresentationTimestamp::new(100), view, &prompt())
            .unwrap();
        assert_eq!(seed.frame_index, 0);
        assert_eq!(seed.mask.width(), 2);
        assert_eq!(seed.mask.height(), 2);
        assert!(
            seed.mask
                .pixels()
                .iter()
                .all(|pixel| matches!(pixel, 0 | 255))
        );
        assert!(seed.mask.pixels().contains(&0));
        assert!(seed.mask.pixels().contains(&255));
        assert_eq!((seed.alpha.width(), seed.alpha.height()), (2, 2));
        assert!(
            seed.alpha
                .pixels()
                .iter()
                .all(|pixel| *pixel > 0 && *pixel < 255)
        );
        assert_eq!(
            session.occupancy(),
            StateOccupancy {
                memory_frames: 1,
                pointer_frames: 1,
                max_memory_frames: 7,
                max_pointer_frames: 16,
            }
        );

        let tracked = session
            .track(PresentationTimestamp::new(101), view)
            .unwrap();
        assert_eq!(tracked.frame_index, 1);
        assert!(tracked.mask.pixels().iter().all(|pixel| *pixel == 255));
        assert!(tracked.alpha.pixels().iter().all(|pixel| *pixel == 225));
        assert_eq!(session.next_frame_index(), 2);
        drop(session);

        let calls = calls.borrow();
        assert_eq!(calls.encodes, 2);
        assert_eq!(calls.memory_attention_warm, 1);
        assert_eq!(calls.memory_attention_hot, 0);
        assert_eq!(calls.memory_encodes, 2);
        assert_eq!(calls.first_rgb, Some([10.0, 20.0, 30.0]));
        assert_eq!(calls.decodes[0].1, [1, 1, 0]);
        assert_eq!(calls.decodes[0].0, [512.0, 512.0, 256.0, 256.0, 0.0, 0.0]);
        assert_eq!(calls.decodes[1].1, [-1]);
        assert_eq!(calls.decodes[1].0, [0.0, 0.0]);
    }

    #[test]
    fn sigmoid_quantization_is_stable_for_extreme_logits() {
        assert_eq!(logit_to_alpha8(-100.0), 0);
        assert_eq!(logit_to_alpha8(0.0), 128);
        assert_eq!(logit_to_alpha8(100.0), 255);
    }

    #[test]
    fn initialization_dimensions_and_pts_fail_closed_before_inference() {
        let calls = Rc::new(RefCell::new(Calls::default()));
        let mut adapter = adapter(&calls);
        let pixels = frame(2, 2);
        let view = Rgba8Frame::new(2, 2, &pixels).unwrap();
        let other_pixels = frame(3, 2);
        let other = Rgba8Frame::new(3, 2, &other_pixels).unwrap();
        let mut session = adapter.start_session();

        assert!(session.track(PresentationTimestamp::new(0), view).is_err());
        let invalid_prompt = PointPrompt::new([
            PromptPoint::new(2.0, 1.0, PointLabel::Positive).unwrap(),
            PromptPoint::new(0.0, 0.0, PointLabel::Negative).unwrap(),
            PromptPoint::new(1.0, 0.0, PointLabel::Negative).unwrap(),
        ])
        .unwrap();
        assert!(
            session
                .initialize(PresentationTimestamp::new(10), view, &invalid_prompt)
                .is_err()
        );
        assert_eq!(calls.borrow().encodes, 0);

        session
            .initialize(PresentationTimestamp::new(10), view, &prompt())
            .unwrap();
        let calls_after_seed = calls.borrow().encodes;
        assert!(
            session
                .initialize(PresentationTimestamp::new(11), view, &prompt())
                .is_err()
        );
        assert!(session.track(PresentationTimestamp::new(10), view).is_err());
        assert!(
            session
                .track(PresentationTimestamp::new(11), other)
                .is_err()
        );
        assert_eq!(calls.borrow().encodes, calls_after_seed);
        assert_eq!(session.next_frame_index(), 1);
    }

    #[test]
    fn backend_failure_does_not_advance_or_seed_state() {
        let calls = Rc::new(RefCell::new(Calls {
            invalid_encoded_shape: true,
            ..Calls::default()
        }));
        let mut adapter = adapter(&calls);
        let pixels = frame(2, 2);
        let view = Rgba8Frame::new(2, 2, &pixels).unwrap();
        let mut session = adapter.start_session();
        assert!(
            session
                .initialize(PresentationTimestamp::new(0), view, &prompt())
                .is_err()
        );
        assert_eq!(session.next_frame_index(), 0);
        assert_eq!(session.occupancy().memory_frames, 0);
        assert_eq!(session.occupancy().pointer_frames, 0);
    }

    #[test]
    fn dropping_session_prevents_job_state_leakage() {
        let calls = Rc::new(RefCell::new(Calls::default()));
        let mut adapter = adapter(&calls);
        let pixels = frame(2, 2);
        let view = Rgba8Frame::new(2, 2, &pixels).unwrap();
        {
            let mut first = adapter.start_session();
            first
                .initialize(PresentationTimestamp::new(0), view, &prompt())
                .unwrap();
            first.track(PresentationTimestamp::new(1), view).unwrap();
            assert_eq!(first.occupancy().memory_frames, 2);
        }
        {
            let mut second = adapter.start_session();
            assert_eq!(second.next_frame_index(), 0);
            assert_eq!(second.occupancy().memory_frames, 0);
            second
                .initialize(PresentationTimestamp::new(-50), view, &prompt())
                .unwrap();
            assert_eq!(second.occupancy().memory_frames, 1);
        }
        let labels = calls
            .borrow()
            .decodes
            .iter()
            .map(|(_, labels)| labels.len())
            .collect::<Vec<_>>();
        assert_eq!(labels, [3, 1, 3]);
    }

    #[test]
    fn memory_and_pointer_history_have_fixed_capacity() {
        let mut bank = MemoryBank::default();
        for frame in 0..40 {
            bank.insert(
                frame,
                vec![frame as f32; MEMORY_TOKENS_PER_FRAME * 64],
                vec![frame as f32; EMBEDDING_DIM],
            )
            .unwrap();
            let occupancy = bank.occupancy();
            assert!(occupancy.memory_frames <= occupancy.max_memory_frames);
            assert!(occupancy.pointer_frames <= occupancy.max_pointer_frames);
        }
        assert_eq!(bank.occupancy().memory_frames, MEMORY_SLOTS);
        assert_eq!(bank.occupancy().pointer_frames, POINTER_TOKENS / 4);
        assert_eq!(
            bank.assemble_memory(40).unwrap().len(),
            KEY_VALUE_TOKENS * 64
        );
    }

    #[test]
    fn typed_inputs_and_resource_limits_are_validated() {
        assert!(Rgba8Frame::new(0, 1, &[]).is_err());
        assert!(Rgba8Frame::new(2, 2, &[0; 15]).is_err());
        assert!(PromptPoint::new(f32::NAN, 0.0, PointLabel::Positive).is_err());
        assert!(
            PointPrompt::new([
                PromptPoint::new(0.0, 0.0, PointLabel::Negative).unwrap(),
                PromptPoint::new(1.0, 0.0, PointLabel::Negative).unwrap(),
                PromptPoint::new(0.0, 1.0, PointLabel::Negative).unwrap(),
            ])
            .is_err()
        );

        let calls = Rc::new(RefCell::new(Calls::default()));
        let backend = MockBackend {
            calls: Rc::clone(&calls),
        };
        let mut adapter = EdgeTamAdapter::new(
            backend,
            EdgeTamConstants::zeros(),
            AdapterOptions {
                mask_threshold: 0.0,
                max_source_pixels: 3,
            },
        )
        .unwrap();
        let pixels = frame(2, 2);
        let mut session = adapter.start_session();
        assert!(
            session
                .initialize(
                    PresentationTimestamp::new(0),
                    Rgba8Frame::new(2, 2, &pixels).unwrap(),
                    &prompt(),
                )
                .is_err()
        );
        assert_eq!(calls.borrow().encodes, 0);
    }
}
