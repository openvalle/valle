//! Streaming TransNetV2 adapter for ONNX Runtime.
//!
//! The adapter owns one inference session and accepts tightly packed RGB or
//! RGBA frames one at a time. Frames are resized immediately to the model's
//! 48x27 RGB contract. It implements the upstream 100-frame window, 50-frame
//! step, replicated edge padding, and central 25..75 prediction policy without
//! retaining the complete video in memory.

use std::collections::VecDeque;
#[cfg(feature = "model-transnetv2-onnx")]
use std::path::Path;

#[cfg(feature = "model-transnetv2-onnx")]
use crate::models::runtime::onnx::{OnnxSession, OnnxSessionOptions, OnnxTensorInput};
use crate::models::spec::{Artifact, Backend, Dimension, ModelManifest, Route};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

pub const ADAPTER: &str = "transnetv2-shot-detection";
pub const CONTRACT_VERSION: u32 = 1;

pub const MODEL_WIDTH: usize = 48;
pub const MODEL_HEIGHT: usize = 27;
pub const MODEL_CHANNELS: usize = 3;
pub const WINDOW_FRAMES: usize = 100;
pub const STEP_FRAMES: usize = 50;
pub const RETAIN_START: usize = 25;
pub const RETAIN_END: usize = 75;
pub const MODEL_FRAME_BYTES: usize = MODEL_WIDTH * MODEL_HEIGHT * MODEL_CHANNELS;
pub const MODEL_WINDOW_BYTES: usize = WINDOW_FRAMES * MODEL_FRAME_BYTES;

/// The largest number of resized source frames resident in the adapter.
///
/// A temporary 100-frame tensor is additionally allocated only while a window
/// is being inferred. Neither allocation grows with the input duration.
pub const MAX_BUFFERED_FRAMES: usize = WINDOW_FRAMES;

/// Shot-boundary post-processing options.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DetectionConfig {
    pub threshold: f32,
}

impl Default for DetectionConfig {
    fn default() -> Self {
        Self { threshold: 0.5 }
    }
}

/// ONNX Runtime settings for the fixed-shape TransNetV2 graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrtSessionOptions {
    pub intra_threads: Option<usize>,
    pub memory_pattern: bool,
    pub prepacking: bool,
}

impl Default for OrtSessionOptions {
    fn default() -> Self {
        Self {
            intra_threads: None,
            memory_pattern: true,
            prepacking: true,
        }
    }
}

#[derive(Deserialize)]
struct PreprocessContract {
    color_space: String,
    resize: [usize; 2],
    window_frames: usize,
    step_frames: usize,
}

#[derive(Deserialize)]
struct PostprocessContract {
    retain_frame_range: [usize; 2],
    threshold: f32,
}

#[derive(Deserialize)]
struct OnnxMetadata {
    opset: u32,
    window_frames: usize,
    step_frames: usize,
}

/// Validate the complete release-to-adapter boundary without loading ONNX Runtime.
pub fn validate_release_contract(
    manifest: &ModelManifest,
    route: &Route,
    artifact: &Artifact,
) -> Result<()> {
    manifest.validate()?;
    ensure!(
        manifest.model.id == "transnetv2" && manifest.model.task == "shot-boundary-detection",
        "manifest is not the TransNetV2 shot-boundary release"
    );
    ensure!(
        manifest.contract.adapter == ADAPTER && manifest.contract.version == CONTRACT_VERSION,
        "unsupported TransNetV2 adapter contract {}@{}",
        manifest.contract.adapter,
        manifest.contract.version
    );
    ensure!(
        manifest.contract.inputs.len() == 1 && manifest.contract.outputs.len() == 2,
        "TransNetV2 contract must expose one input and two outputs"
    );
    let input = &manifest.contract.inputs[0];
    ensure!(
        input.name == "frames"
            && input.dtype == "u8"
            && input.layout == "nthwc"
            && input.shape
                == vec![
                    Dimension::Fixed(1),
                    Dimension::Fixed(WINDOW_FRAMES as u64),
                    Dimension::Fixed(MODEL_HEIGHT as u64),
                    Dimension::Fixed(MODEL_WIDTH as u64),
                    Dimension::Fixed(MODEL_CHANNELS as u64),
                ]
            && input.range == Some([0.0, 255.0]),
        "TransNetV2 input must be frames u8 NTHWC [1,100,27,48,3] in [0,255]"
    );
    for (output, name) in manifest
        .contract
        .outputs
        .iter()
        .zip(["single_frame_pred", "all_frames_pred"])
    {
        ensure!(
            output.name == name
                && output.dtype == "f32"
                && output.layout == "nt"
                && output.shape
                    == vec![Dimension::Fixed(1), Dimension::Fixed(WINDOW_FRAMES as u64),]
                && output.range == Some([0.0, 1.0]),
            "TransNetV2 output {name:?} must be f32 NT [1,100] in [0,1]"
        );
    }
    let preprocess: PreprocessContract =
        serde_json::from_value(manifest.contract.preprocess.clone())
            .context("TransNetV2 preprocess contract is invalid")?;
    ensure!(
        preprocess.color_space == "rgb"
            && preprocess.resize == [MODEL_WIDTH, MODEL_HEIGHT]
            && preprocess.window_frames == WINDOW_FRAMES
            && preprocess.step_frames == STEP_FRAMES,
        "TransNetV2 preprocess contract does not match the streaming adapter"
    );
    let postprocess: PostprocessContract =
        serde_json::from_value(manifest.contract.postprocess.clone())
            .context("TransNetV2 postprocess contract is invalid")?;
    ensure!(
        postprocess.retain_frame_range == [RETAIN_START, RETAIN_END]
            && postprocess.threshold == DetectionConfig::default().threshold,
        "TransNetV2 postprocess contract does not match the streaming adapter defaults"
    );
    ensure!(route.artifact == artifact.id, "route and artifact disagree");
    ensure!(
        route.backend == Backend::OnnxCpu,
        "TransNetV2 session requires an onnx-cpu route"
    );
    ensure!(
        artifact.format == "onnx" && artifact.precision == "fp32",
        "TransNetV2 requires an fp32 ONNX artifact"
    );
    let metadata: OnnxMetadata = serde_json::from_value(artifact.metadata.clone())
        .context("TransNetV2 ONNX metadata is invalid")?;
    ensure!(
        metadata.opset == 17
            && metadata.window_frames == WINDOW_FRAMES
            && metadata.step_frames == STEP_FRAMES,
        "TransNetV2 ONNX metadata does not match the streaming adapter"
    );
    Ok(())
}

/// Predictions returned by one 100-frame model invocation.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowPrediction {
    pub single_frame: Vec<f32>,
    pub all_frames: Vec<f32>,
}

impl WindowPrediction {
    pub fn new(single_frame: Vec<f32>, all_frames: Vec<f32>) -> Self {
        Self {
            single_frame,
            all_frames,
        }
    }
}

/// Backend boundary used by the streaming algorithm and its deterministic tests.
///
/// `frames` is one packed `[100, 27, 48, 3]` RGB byte tensor. It is borrowed so
/// the adapter can reuse the same fixed-capacity window buffer for every call.
pub trait WindowPredictor {
    fn predict_window(&mut self, frames: &[u8]) -> Result<WindowPrediction>;
}

/// A single ONNX Runtime session reused for every video window.
pub struct OrtWindowPredictor {
    #[cfg(feature = "model-transnetv2-onnx")]
    session: OnnxSession,
}

#[cfg(feature = "model-transnetv2-onnx")]
impl OrtWindowPredictor {
    pub fn open(model: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_options(model, OrtSessionOptions::default())
    }

    pub fn open_with_options(model: impl AsRef<Path>, options: OrtSessionOptions) -> Result<Self> {
        if let Some(threads) = options.intra_threads {
            ensure!(threads > 0, "ONNX intra_threads must be positive");
        }
        let model = model.as_ref();
        let session = OnnxSession::load_with_options(
            model,
            &OnnxSessionOptions {
                intra_threads: options.intra_threads,
                memory_pattern: options.memory_pattern,
                prepacking: options.prepacking,
            },
        )
        .with_context(|| format!("load TransNetV2 model {}", model.display()))?;
        Ok(Self { session })
    }
}

#[cfg(feature = "model-transnetv2-onnx")]
impl WindowPredictor for OrtWindowPredictor {
    fn predict_window(&mut self, frames: &[u8]) -> Result<WindowPrediction> {
        ensure!(
            frames.len() == MODEL_WINDOW_BYTES,
            "TransNetV2 window has {} bytes; expected {MODEL_WINDOW_BYTES}",
            frames.len()
        );

        let outputs = self
            .session
            .run_typed(
                &[OnnxTensorInput::borrowed_u8(
                    "frames",
                    [
                        1usize,
                        WINDOW_FRAMES,
                        MODEL_HEIGHT,
                        MODEL_WIDTH,
                        MODEL_CHANNELS,
                    ],
                    frames,
                )],
                &["single_frame_pred", "all_frames_pred"],
            )
            .context("run TransNetV2 ONNX inference")?;

        let single_output = outputs
            .first()
            .context("TransNetV2 graph did not return single_frame_pred")?;
        ensure!(
            single_output.shape == [1, WINDOW_FRAMES],
            "single_frame_pred shape is {:?}; expected [1, {WINDOW_FRAMES}]",
            single_output.shape
        );

        let all_output = outputs
            .get(1)
            .context("TransNetV2 graph did not return all_frames_pred")?;
        ensure!(
            all_output.shape == [1, WINDOW_FRAMES],
            "all_frames_pred shape is {:?}; expected [1, {WINDOW_FRAMES}]",
            all_output.shape
        );

        Ok(WindowPrediction::new(
            single_output.data.clone(),
            all_output.data.clone(),
        ))
    }
}

/// A contiguous run of frames whose transition probability exceeds the threshold.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransitionBoundary {
    pub start_frame: usize,
    pub end_frame_exclusive: usize,
    pub peak_frame: usize,
    pub peak_single_probability: f32,
    pub peak_all_frames_probability: f32,
}

/// A detected shot range using an inclusive start and exclusive end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShotRange {
    pub start_frame: usize,
    pub end_frame_exclusive: usize,
}

/// Final streaming result. Per-frame scores are intentionally not retained.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DetectionResult {
    pub frame_count: usize,
    pub window_count: usize,
    pub max_buffered_frames: usize,
    pub boundaries: Vec<TransitionBoundary>,
    pub shots: Vec<ShotRange>,
}

#[derive(Debug, Clone)]
struct ActiveBoundary {
    start_frame: usize,
    peak_frame: usize,
    peak_single_probability: f32,
    peak_all_frames_probability: f32,
}

/// Streaming TransNetV2 adapter backed by one reusable predictor session.
pub struct TransNetV2Session<P = OrtWindowPredictor> {
    predictor: P,
    config: DetectionConfig,
    frames: VecDeque<Vec<u8>>,
    window: Vec<u8>,
    buffer_start: usize,
    frame_count: usize,
    next_block_start: usize,
    window_count: usize,
    max_buffered_frames: usize,
    boundaries: Vec<TransitionBoundary>,
    active_boundary: Option<ActiveBoundary>,
}

#[cfg(feature = "model-transnetv2-onnx")]
impl TransNetV2Session<OrtWindowPredictor> {
    pub fn open(model: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_options(
            model,
            DetectionConfig::default(),
            OrtSessionOptions::default(),
        )
    }

    pub fn open_with_options(
        model: impl AsRef<Path>,
        config: DetectionConfig,
        options: OrtSessionOptions,
    ) -> Result<Self> {
        Self::with_predictor(
            OrtWindowPredictor::open_with_options(model, options)?,
            config,
        )
    }

    pub fn load(
        manifest: &ModelManifest,
        route: &Route,
        artifact: &Artifact,
        artifact_root: &Path,
        config: DetectionConfig,
        options: OrtSessionOptions,
    ) -> Result<Self> {
        validate_release_contract(manifest, route, artifact)?;
        let entrypoint = artifact_root.join(&artifact.entrypoint);
        ensure!(
            entrypoint.is_file(),
            "model artifact is missing: {}",
            entrypoint.display()
        );
        Self::open_with_options(entrypoint, config, options)
    }
}

impl<P: WindowPredictor> TransNetV2Session<P> {
    pub fn with_predictor(predictor: P, config: DetectionConfig) -> Result<Self> {
        ensure!(
            config.threshold.is_finite() && (0.0..=1.0).contains(&config.threshold),
            "transition threshold must be finite and between 0 and 1"
        );
        Ok(Self {
            predictor,
            config,
            frames: VecDeque::with_capacity(MAX_BUFFERED_FRAMES),
            window: Vec::with_capacity(MODEL_WINDOW_BYTES),
            buffer_start: 0,
            frame_count: 0,
            next_block_start: 0,
            window_count: 0,
            max_buffered_frames: 0,
            boundaries: Vec::new(),
            active_boundary: None,
        })
    }

    /// Push one tightly packed RGB frame. The frame is resized before returning.
    pub fn push_rgb(&mut self, width: usize, height: usize, frame: &[u8]) -> Result<()> {
        let resized = resize_to_model_rgb(width, height, 3, frame)?;
        self.push_resized(resized)
    }

    /// Push one tightly packed RGBA frame. Alpha is ignored during resize.
    pub fn push_rgba(&mut self, width: usize, height: usize, frame: &[u8]) -> Result<()> {
        let resized = resize_to_model_rgb(width, height, 4, frame)?;
        self.push_resized(resized)
    }

    pub fn frame_count(&self) -> usize {
        self.frame_count
    }

    pub fn processed_window_count(&self) -> usize {
        self.window_count
    }

    pub fn buffered_frame_count(&self) -> usize {
        self.frames.len()
    }

    /// Flush the final edge-padded windows and return boundaries and shot ranges.
    pub fn finish(mut self) -> Result<DetectionResult> {
        self.process_ready_windows(true)?;
        self.close_active_boundary(self.frame_count);
        let shots = boundaries_to_shots(self.frame_count, &self.boundaries);
        Ok(DetectionResult {
            frame_count: self.frame_count,
            window_count: self.window_count,
            max_buffered_frames: self.max_buffered_frames,
            boundaries: self.boundaries,
            shots,
        })
    }

    fn push_resized(&mut self, frame: Vec<u8>) -> Result<()> {
        ensure!(
            frame.len() == MODEL_FRAME_BYTES,
            "resized frame has {} bytes; expected {MODEL_FRAME_BYTES}",
            frame.len()
        );
        self.frame_count = self
            .frame_count
            .checked_add(1)
            .context("frame count overflow")?;
        self.frames.push_back(frame);
        self.max_buffered_frames = self.max_buffered_frames.max(self.frames.len());
        ensure!(
            self.frames.len() <= MAX_BUFFERED_FRAMES,
            "streaming frame buffer exceeded its fixed {MAX_BUFFERED_FRAMES}-frame bound"
        );
        self.process_ready_windows(false)
    }

    fn process_ready_windows(&mut self, finishing: bool) -> Result<()> {
        while self.next_block_start < self.frame_count {
            let needed = self
                .next_block_start
                .checked_add(RETAIN_END)
                .context("window position overflow")?;
            if !finishing && self.frame_count < needed {
                break;
            }

            self.build_window()?;
            let prediction = self.predictor.predict_window(&self.window)?;
            ensure!(
                prediction.single_frame.len() == WINDOW_FRAMES,
                "single_frame_pred has {} values; expected {WINDOW_FRAMES}",
                prediction.single_frame.len()
            );
            ensure!(
                prediction.all_frames.len() == WINDOW_FRAMES,
                "all_frames_pred has {} values; expected {WINDOW_FRAMES}",
                prediction.all_frames.len()
            );

            let retained = STEP_FRAMES.min(self.frame_count - self.next_block_start);
            for offset in 0..retained {
                let output_index = RETAIN_START + offset;
                let frame_index = self.next_block_start + offset;
                let single = prediction.single_frame[output_index];
                let all = prediction.all_frames[output_index];
                ensure!(
                    single.is_finite() && all.is_finite(),
                    "non-finite prediction at frame {frame_index}"
                );
                self.consume_prediction(frame_index, single, all);
            }

            self.window_count += 1;
            self.next_block_start = self
                .next_block_start
                .checked_add(STEP_FRAMES)
                .context("window position overflow")?;
            self.discard_before(self.next_block_start.saturating_sub(RETAIN_START));
        }
        Ok(())
    }

    fn build_window(&mut self) -> Result<()> {
        ensure!(
            self.frame_count > 0,
            "cannot build a window for an empty stream"
        );
        self.window.clear();
        for position in 0..WINDOW_FRAMES {
            let source_index = if position < RETAIN_START {
                self.next_block_start
                    .saturating_sub(RETAIN_START - position)
            } else {
                self.next_block_start
                    .checked_add(position - RETAIN_START)
                    .context("source frame position overflow")?
            }
            .min(self.frame_count - 1);
            ensure!(
                source_index >= self.buffer_start,
                "source frame {source_index} was discarded before window {}",
                self.next_block_start / STEP_FRAMES
            );
            let offset = source_index - self.buffer_start;
            let frame = self.frames.get(offset).with_context(|| {
                format!(
                    "source frame {source_index} is unavailable in buffer {}..{}",
                    self.buffer_start,
                    self.buffer_start + self.frames.len()
                )
            })?;
            self.window.extend_from_slice(frame);
        }
        debug_assert_eq!(self.window.len(), MODEL_WINDOW_BYTES);
        Ok(())
    }

    fn discard_before(&mut self, target: usize) {
        while self.buffer_start < target && self.frames.pop_front().is_some() {
            self.buffer_start += 1;
        }
    }

    fn consume_prediction(&mut self, frame_index: usize, single: f32, all: f32) {
        if single > self.config.threshold {
            match self.active_boundary.as_mut() {
                Some(boundary) if single > boundary.peak_single_probability => {
                    boundary.peak_frame = frame_index;
                    boundary.peak_single_probability = single;
                    boundary.peak_all_frames_probability = all;
                }
                Some(_) => {}
                None => {
                    self.active_boundary = Some(ActiveBoundary {
                        start_frame: frame_index,
                        peak_frame: frame_index,
                        peak_single_probability: single,
                        peak_all_frames_probability: all,
                    });
                }
            }
        } else {
            self.close_active_boundary(frame_index);
        }
    }

    fn close_active_boundary(&mut self, end_frame_exclusive: usize) {
        if let Some(boundary) = self.active_boundary.take() {
            self.boundaries.push(TransitionBoundary {
                start_frame: boundary.start_frame,
                end_frame_exclusive,
                peak_frame: boundary.peak_frame,
                peak_single_probability: boundary.peak_single_probability,
                peak_all_frames_probability: boundary.peak_all_frames_probability,
            });
        }
    }
}

fn boundaries_to_shots(frame_count: usize, boundaries: &[TransitionBoundary]) -> Vec<ShotRange> {
    if frame_count == 0 {
        return Vec::new();
    }

    let mut shots = Vec::new();
    let mut start = 0;
    for boundary in boundaries {
        if boundary.start_frame != 0 {
            let end = boundary.start_frame.saturating_add(1).min(frame_count);
            if start < end {
                shots.push(ShotRange {
                    start_frame: start,
                    end_frame_exclusive: end,
                });
            }
        }
        start = boundary.end_frame_exclusive.min(frame_count);
    }
    if start < frame_count {
        shots.push(ShotRange {
            start_frame: start,
            end_frame_exclusive: frame_count,
        });
    }
    if shots.is_empty() {
        shots.push(ShotRange {
            start_frame: 0,
            end_frame_exclusive: frame_count,
        });
    }
    shots
}

fn resize_to_model_rgb(
    width: usize,
    height: usize,
    channels: usize,
    source: &[u8],
) -> Result<Vec<u8>> {
    ensure!(width > 0 && height > 0, "frame dimensions must be positive");
    ensure!(
        channels == 3 || channels == 4,
        "only packed RGB and RGBA frames are supported"
    );
    let expected = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(channels))
        .context("frame dimensions overflow")?;
    ensure!(
        source.len() == expected,
        "frame has {} bytes; packed {width}x{height}x{channels} requires {expected}",
        source.len()
    );

    if width == MODEL_WIDTH && height == MODEL_HEIGHT {
        if channels == MODEL_CHANNELS {
            return Ok(source.to_vec());
        }
        let mut output = Vec::with_capacity(MODEL_FRAME_BYTES);
        for pixel in source.chunks_exact(4) {
            output.extend_from_slice(&pixel[..3]);
        }
        return Ok(output);
    }

    let mut output = vec![0_u8; MODEL_FRAME_BYTES];
    for target_y in 0..MODEL_HEIGHT {
        let source_y = ((target_y as f64 + 0.5) * height as f64 / MODEL_HEIGHT as f64 - 0.5)
            .clamp(0.0, (height - 1) as f64);
        let y0 = source_y.floor() as usize;
        let y1 = (y0 + 1).min(height - 1);
        let wy = source_y - y0 as f64;

        for target_x in 0..MODEL_WIDTH {
            let source_x = ((target_x as f64 + 0.5) * width as f64 / MODEL_WIDTH as f64 - 0.5)
                .clamp(0.0, (width - 1) as f64);
            let x0 = source_x.floor() as usize;
            let x1 = (x0 + 1).min(width - 1);
            let wx = source_x - x0 as f64;

            for channel in 0..MODEL_CHANNELS {
                let p00 = source[(y0 * width + x0) * channels + channel] as f64;
                let p01 = source[(y0 * width + x1) * channels + channel] as f64;
                let p10 = source[(y1 * width + x0) * channels + channel] as f64;
                let p11 = source[(y1 * width + x1) * channels + channel] as f64;
                let top = p00 + (p01 - p00) * wx;
                let bottom = p10 + (p11 - p10) * wx;
                let value = top + (bottom - top) * wy;
                output[(target_y * MODEL_WIDTH + target_x) * MODEL_CHANNELS + channel] =
                    value.round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    #[derive(Clone, Default)]
    struct RecordingPredictor {
        windows: Arc<Mutex<Vec<Vec<u8>>>>,
    }

    impl WindowPredictor for RecordingPredictor {
        fn predict_window(&mut self, frames: &[u8]) -> Result<WindowPrediction> {
            ensure!(frames.len() == MODEL_WINDOW_BYTES, "bad mock window");
            let mut single = vec![0.0; WINDOW_FRAMES];
            let mut all = vec![0.0; WINDOW_FRAMES];
            for position in 0..WINDOW_FRAMES {
                let identity = frames[position * MODEL_FRAME_BYTES];
                if identity == 20 || (50..=52).contains(&identity) || identity == 122 {
                    single[position] = 0.9;
                    all[position] = 0.8;
                }
            }
            self.windows.lock().unwrap().push(frames.to_vec());
            Ok(WindowPrediction::new(single, all))
        }
    }

    fn constant_frame(identity: u8) -> Vec<u8> {
        vec![identity; MODEL_FRAME_BYTES]
    }

    fn identities(window: &[u8]) -> Vec<u8> {
        (0..WINDOW_FRAMES)
            .map(|index| window[index * MODEL_FRAME_BYTES])
            .collect()
    }

    fn release() -> ModelManifest {
        serde_json::from_str(include_str!("../../catalog/release-transnetv2-1.0.0.json")).unwrap()
    }

    fn onnx_route_and_artifact(manifest: &ModelManifest) -> (&Route, &Artifact) {
        let route = manifest
            .routes
            .iter()
            .find(|route| route.id == "onnx-cpu-macos-arm64")
            .unwrap();
        let artifact = manifest.artifact(&route.artifact).unwrap();
        (route, artifact)
    }

    #[test]
    fn checked_in_release_matches_streaming_adapter_contract() {
        let manifest = release();
        let (route, artifact) = onnx_route_and_artifact(&manifest);
        validate_release_contract(&manifest, route, artifact).unwrap();
    }

    #[test]
    fn contract_validation_rejects_adapter_tensor_and_window_drift() {
        let mut manifest = release();
        manifest.contract.adapter = "wrong-adapter".into();
        let (route, artifact) = onnx_route_and_artifact(&manifest);
        assert!(validate_release_contract(&manifest, route, artifact).is_err());

        let mut manifest = release();
        manifest.contract.inputs[0].name = "input".into();
        let (route, artifact) = onnx_route_and_artifact(&manifest);
        assert!(validate_release_contract(&manifest, route, artifact).is_err());

        let mut manifest = release();
        manifest.contract.preprocess["window_frames"] = 50.into();
        let (route, artifact) = onnx_route_and_artifact(&manifest);
        assert!(validate_release_contract(&manifest, route, artifact).is_err());
    }

    #[test]
    fn streaming_windows_match_edge_padding_and_central_blocks() {
        let predictor = RecordingPredictor::default();
        let captured = Arc::clone(&predictor.windows);
        let mut session =
            TransNetV2Session::with_predictor(predictor, DetectionConfig::default()).unwrap();

        for index in 0..123_u8 {
            session
                .push_rgb(MODEL_WIDTH, MODEL_HEIGHT, &constant_frame(index))
                .unwrap();
            assert!(session.buffered_frame_count() <= MAX_BUFFERED_FRAMES);
        }
        let result = session.finish().unwrap();

        assert_eq!(result.frame_count, 123);
        assert_eq!(result.window_count, 3);
        assert!(result.max_buffered_frames <= MAX_BUFFERED_FRAMES);
        assert_eq!(
            result.boundaries,
            vec![
                TransitionBoundary {
                    start_frame: 20,
                    end_frame_exclusive: 21,
                    peak_frame: 20,
                    peak_single_probability: 0.9,
                    peak_all_frames_probability: 0.8,
                },
                TransitionBoundary {
                    start_frame: 50,
                    end_frame_exclusive: 53,
                    peak_frame: 50,
                    peak_single_probability: 0.9,
                    peak_all_frames_probability: 0.8,
                },
                TransitionBoundary {
                    start_frame: 122,
                    end_frame_exclusive: 123,
                    peak_frame: 122,
                    peak_single_probability: 0.9,
                    peak_all_frames_probability: 0.8,
                },
            ]
        );
        assert_eq!(
            result.shots,
            vec![
                ShotRange {
                    start_frame: 0,
                    end_frame_exclusive: 21,
                },
                ShotRange {
                    start_frame: 21,
                    end_frame_exclusive: 51,
                },
                ShotRange {
                    start_frame: 53,
                    end_frame_exclusive: 123,
                },
            ]
        );

        let windows = captured.lock().unwrap();
        assert_eq!(windows.len(), 3);
        let first = identities(&windows[0]);
        assert!(first[..26].iter().all(|&value| value == 0));
        assert_eq!(first[99], 74);

        let second = identities(&windows[1]);
        assert_eq!(second[0], 25);
        assert_eq!(second[97], 122);
        assert_eq!(&second[98..], &[122, 122]);

        let third = identities(&windows[2]);
        assert_eq!(third[0], 75);
        assert_eq!(third[47], 122);
        assert!(third[48..].iter().all(|&value| value == 122));
    }

    #[test]
    fn frame_storage_is_bounded_for_long_streams() {
        let predictor = RecordingPredictor::default();
        let mut session =
            TransNetV2Session::with_predictor(predictor, DetectionConfig::default()).unwrap();
        let frame = constant_frame(0);
        for _ in 0..1_000 {
            session.push_rgb(MODEL_WIDTH, MODEL_HEIGHT, &frame).unwrap();
            assert!(session.buffered_frame_count() <= MAX_BUFFERED_FRAMES);
        }
        let result = session.finish().unwrap();
        assert_eq!(result.window_count, 20);
        assert!(result.max_buffered_frames <= MAX_BUFFERED_FRAMES);
    }

    #[test]
    fn windows_run_as_soon_as_enough_lookahead_arrives() {
        let predictor = RecordingPredictor::default();
        let mut session =
            TransNetV2Session::with_predictor(predictor, DetectionConfig::default()).unwrap();
        let frame = constant_frame(0);
        for _ in 0..74 {
            session.push_rgb(MODEL_WIDTH, MODEL_HEIGHT, &frame).unwrap();
        }
        assert_eq!(session.processed_window_count(), 0);
        session.push_rgb(MODEL_WIDTH, MODEL_HEIGHT, &frame).unwrap();
        assert_eq!(session.processed_window_count(), 1);
        for _ in 0..50 {
            session.push_rgb(MODEL_WIDTH, MODEL_HEIGHT, &frame).unwrap();
        }
        assert_eq!(session.processed_window_count(), 2);
    }

    #[test]
    fn empty_stream_has_no_boundaries_or_shots() {
        let session = TransNetV2Session::with_predictor(
            RecordingPredictor::default(),
            DetectionConfig::default(),
        )
        .unwrap();
        let result = session.finish().unwrap();
        assert_eq!(result.frame_count, 0);
        assert_eq!(result.window_count, 0);
        assert!(result.boundaries.is_empty());
        assert!(result.shots.is_empty());
    }

    #[test]
    fn rgba_input_discards_alpha_without_changing_rgb() {
        let mut rgba = Vec::with_capacity(MODEL_WIDTH * MODEL_HEIGHT * 4);
        for _ in 0..MODEL_WIDTH * MODEL_HEIGHT {
            rgba.extend_from_slice(&[11, 22, 33, 255]);
        }
        let resized = resize_to_model_rgb(MODEL_WIDTH, MODEL_HEIGHT, 4, &rgba).unwrap();
        assert_eq!(&resized[..6], &[11, 22, 33, 11, 22, 33]);
        assert_eq!(resized.len(), MODEL_FRAME_BYTES);
    }

    #[test]
    fn resize_uses_bilinear_sampling_for_rgb() {
        let source = [0, 0, 0, 100, 0, 0, 0, 100, 0, 100, 100, 0];
        let resized = resize_to_model_rgb(2, 2, 3, &source).unwrap();
        let center = ((MODEL_HEIGHT / 2) * MODEL_WIDTH + MODEL_WIDTH / 2) * 3;
        assert!((48..=52).contains(&resized[center]));
        assert!((48..=52).contains(&resized[center + 1]));
        assert_eq!(resized[center + 2], 0);
    }

    #[test]
    fn invalid_threshold_and_frame_layout_are_rejected() {
        assert!(
            TransNetV2Session::with_predictor(
                RecordingPredictor::default(),
                DetectionConfig { threshold: 1.1 },
            )
            .is_err()
        );
        let mut session = TransNetV2Session::with_predictor(
            RecordingPredictor::default(),
            DetectionConfig::default(),
        )
        .unwrap();
        assert!(session.push_rgb(10, 10, &[0; 10]).is_err());
    }
}
