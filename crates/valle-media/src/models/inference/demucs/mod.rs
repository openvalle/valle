//! Streaming two-stem Demucs source-separation adapter and load-once ONNX session.
//!
//! Media decoding, resampling, spooling and output encoding stay outside this crate. The adapter
//! owns the model contract: 44.1 kHz stereo, decoded-whole-track mono-reference statistics,
//! normalization, 343980-sample segments, 257985 stride, final right-zero-padding, triangular
//! weighted overlap-add, `[vocals, instrumental]` ordering, stem-specific denormalization and exact
//! source-length cropping. Its pull-source/push-sink interface keeps memory independent of track
//! duration and never retains complete stems.

#[cfg(any(feature = "model-demucs-onnx", test))]
mod contract;
mod stream;

#[cfg(feature = "model-demucs-onnx")]
use std::path::Path;
#[cfg(feature = "model-demucs-onnx")]
use std::time::Instant;

#[cfg(feature = "model-demucs-onnx")]
use crate::models::runtime::TensorInput;
#[cfg(feature = "model-demucs-onnx")]
use crate::models::runtime::onnx::{OnnxSession, OnnxSessionOptions};
#[cfg(any(feature = "model-demucs-onnx", test))]
use crate::models::spec::{Artifact, ModelManifest, Route};
use anyhow::{Context, Result, ensure};

pub use stream::{
    AdapterOptions, DemucsAdapter, SegmentBackend, SegmentOutput, SeparationSummary, StemChunk,
    StemSink, StereoSamples, StereoSource,
};

pub const ADAPTER: &str = "demucs-source-separation";
pub const CONTRACT_VERSION: u32 = 2;
pub const SAMPLE_RATE: u32 = 44_100;
pub const CHANNELS: u16 = 2;
pub const STEMS: usize = 2;
pub const SEGMENT_SAMPLES: usize = 343_980;
pub const STRIDE_SAMPLES: usize = 257_985;
pub const OVERLAP_SAMPLES: usize = SEGMENT_SAMPLES - STRIDE_SAMPLES;
pub const WINDOW_HALF: usize = SEGMENT_SAMPLES / 2;

/// First-pass decoded-whole-track normalization statistics.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackNormalization {
    samples: u64,
    mean: f32,
    std: f32,
}

impl TrackNormalization {
    pub const fn samples(self) -> u64 {
        self.samples
    }

    pub const fn mean(self) -> f32 {
        self.mean
    }

    pub const fn std(self) -> f32 {
        self.std
    }

    fn normalize(self, sample: f32) -> f32 {
        (sample - self.mean) / self.std
    }

    fn denormalize(self, stem: usize, sample: f32) -> f32 {
        let mean_multiplier = if stem == 0 { 1.0 } else { 3.0 };
        sample * self.std + mean_multiplier * self.mean
    }
}

/// Streaming first-pass accumulator over decoded 44.1 kHz stereo samples.
#[derive(Debug, Clone)]
pub struct TrackStatsBuilder {
    count: u64,
    mean: f64,
    squared_deviation: f64,
}

impl TrackStatsBuilder {
    pub fn new(sample_rate: u32, channels: u16) -> Result<Self> {
        ensure!(
            sample_rate == SAMPLE_RATE && channels == CHANNELS,
            "Demucs requires {SAMPLE_RATE} Hz stereo, got {sample_rate} Hz/{channels} channels"
        );
        Ok(Self {
            count: 0,
            mean: 0.0,
            squared_deviation: 0.0,
        })
    }

    pub fn push(&mut self, left: &[f32], right: &[f32]) -> Result<()> {
        ensure!(
            left.len() == right.len(),
            "stereo statistics slices differ in length"
        );
        for (&left, &right) in left.iter().zip(right) {
            ensure!(
                left.is_finite() && right.is_finite(),
                "track contains NaN or Inf"
            );
            self.push_reference(f64::from((left + right) * 0.5))?;
        }
        Ok(())
    }

    pub fn push_interleaved(&mut self, stereo: &[f32]) -> Result<()> {
        ensure!(
            stereo.len().is_multiple_of(CHANNELS as usize),
            "interleaved stereo buffer has an incomplete frame"
        );
        for frame in stereo.chunks_exact(CHANNELS as usize) {
            let left = frame[0];
            let right = frame[1];
            ensure!(
                left.is_finite() && right.is_finite(),
                "track contains NaN or Inf"
            );
            self.push_reference(f64::from((left + right) * 0.5))?;
        }
        Ok(())
    }

    fn push_reference(&mut self, reference: f64) -> Result<()> {
        self.count = self
            .count
            .checked_add(1)
            .context("track sample count overflowed u64")?;
        let delta = reference - self.mean;
        self.mean += delta / self.count as f64;
        let delta_after = reference - self.mean;
        self.squared_deviation += delta * delta_after;
        Ok(())
    }

    pub fn finish(self) -> Result<TrackNormalization> {
        ensure!(
            self.count >= 2,
            "Demucs track must contain at least two samples"
        );
        let variance = (self.squared_deviation / (self.count - 1) as f64).max(0.0);
        let std = variance.sqrt() + 1e-8;
        ensure!(
            std.is_finite() && std > 0.0,
            "Demucs normalization std is invalid"
        );
        Ok(TrackNormalization {
            samples: self.count,
            mean: self.mean as f32,
            std: std as f32,
        })
    }
}

pub(crate) fn triangular_weight(index: usize) -> f32 {
    debug_assert!(index < SEGMENT_SAMPLES);
    if index < WINDOW_HALF {
        (index + 1) as f32 / WINDOW_HALF as f32
    } else {
        (SEGMENT_SAMPLES - index) as f32 / WINDOW_HALF as f32
    }
}

#[cfg(feature = "model-demucs-onnx")]
#[derive(Debug, Clone)]
pub struct SessionOptions {
    pub adapter: AdapterOptions,
    pub intra_threads: Option<usize>,
    pub memory_pattern: bool,
    pub prepacking: bool,
}

#[cfg(feature = "model-demucs-onnx")]
impl Default for SessionOptions {
    fn default() -> Self {
        Self {
            adapter: AdapterOptions::default(),
            intra_threads: None,
            memory_pattern: true,
            prepacking: true,
        }
    }
}

#[cfg(feature = "model-demucs-onnx")]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SeparationReport {
    pub samples: u64,
    pub segments: usize,
    pub load_ms: f64,
    pub inference_ms: f64,
    pub total_ms: f64,
    pub max_input_samples: usize,
    pub max_output_samples: usize,
    pub max_finalized_samples: usize,
}

/// Reusable load-once ONNX session. It contains no per-track media or output buffers.
#[cfg(feature = "model-demucs-onnx")]
pub struct DemucsSession {
    session: OnnxSession,
    adapter: DemucsAdapter,
    input_name: String,
    output_name: String,
    load_ms: f64,
}

#[cfg(feature = "model-demucs-onnx")]
impl DemucsSession {
    pub fn load(
        manifest: &ModelManifest,
        route: &Route,
        artifact: &Artifact,
        artifact_root: &Path,
        options: &SessionOptions,
    ) -> Result<Self> {
        let names = contract::validate_contract(manifest, route, artifact)?;
        let adapter = DemucsAdapter::new(options.adapter)?;
        if let Some(threads) = options.intra_threads {
            ensure!(threads > 0, "ONNX intra_threads must be positive");
        }
        let entrypoint = artifact_root.join(&artifact.entrypoint);
        ensure!(
            entrypoint.is_file(),
            "model artifact is missing: {}",
            entrypoint.display()
        );
        let started = Instant::now();
        let session = OnnxSession::load_with_options(
            &entrypoint,
            &OnnxSessionOptions {
                intra_threads: options.intra_threads,
                memory_pattern: options.memory_pattern,
                prepacking: options.prepacking,
            },
        )
        .with_context(|| format!("failed to load Demucs from {}", entrypoint.display()))?;
        Ok(Self {
            session,
            adapter,
            input_name: names.input,
            output_name: names.output,
            load_ms: started.elapsed().as_secs_f64() * 1_000.0,
        })
    }

    pub const fn load_ms(&self) -> f64 {
        self.load_ms
    }

    pub const fn adapter_options(&self) -> AdapterOptions {
        self.adapter.options()
    }

    pub fn separate_track(
        &mut self,
        normalization: TrackNormalization,
        source: &mut impl StereoSource,
        sink: &mut impl StemSink,
    ) -> Result<SeparationReport> {
        let started = Instant::now();
        let adapter = self.adapter;
        let mut backend = OnnxBackend {
            session: &mut self.session,
            input_name: &self.input_name,
            output_name: &self.output_name,
            inference_ms: 0.0,
        };
        let summary = adapter.separate(&mut backend, normalization, source, sink)?;
        Ok(SeparationReport {
            samples: summary.samples,
            segments: summary.segments,
            load_ms: self.load_ms,
            inference_ms: backend.inference_ms,
            total_ms: started.elapsed().as_secs_f64() * 1_000.0,
            max_input_samples: summary.max_input_samples,
            max_output_samples: summary.max_output_samples,
            max_finalized_samples: summary.max_finalized_samples,
        })
    }
}

#[cfg(feature = "model-demucs-onnx")]
struct OnnxBackend<'a> {
    session: &'a mut OnnxSession,
    input_name: &'a str,
    output_name: &'a str,
    inference_ms: f64,
}

#[cfg(feature = "model-demucs-onnx")]
impl SegmentBackend for OnnxBackend<'_> {
    fn infer_segment(&mut self, mix: &[f32], shape: [usize; 3]) -> Result<SegmentOutput> {
        let started = Instant::now();
        let output = self.session.run_f32(
            TensorInput::borrowed(self.input_name, shape, mix),
            self.output_name,
        )?;
        self.inference_ms += started.elapsed().as_secs_f64() * 1_000.0;
        Ok(SegmentOutput {
            shape: output.shape,
            data: output.data,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release() -> ModelManifest {
        serde_json::from_str(include_str!("../../catalog/release-demucs-1.0.0.json")).unwrap()
    }

    fn onnx_route_and_artifact(manifest: &ModelManifest) -> (&Route, &Artifact) {
        let route = manifest
            .routes
            .iter()
            .find(|route| route.id == "onnx-cpu-macos-arm64")
            .unwrap();
        let artifact = manifest
            .artifacts
            .iter()
            .find(|artifact| artifact.id == route.artifact)
            .unwrap();
        (route, artifact)
    }

    #[test]
    fn checked_in_release_matches_adapter_contract() {
        let manifest = release();
        let (route, artifact) = onnx_route_and_artifact(&manifest);
        let names = contract::validate_contract(&manifest, route, artifact).unwrap();
        assert_eq!(names.input, "mix");
        assert_eq!(names.output, "stems");
    }

    #[test]
    fn contract_validation_rejects_adapter_geometry_and_stem_drift() {
        let mut manifest = release();
        manifest.contract.adapter = "wrong-adapter".into();
        let (route, artifact) = onnx_route_and_artifact(&manifest);
        assert!(contract::validate_contract(&manifest, route, artifact).is_err());

        let mut manifest = release();
        manifest.contract.inputs[0].shape[2] = crate::models::spec::Dimension::Fixed(1);
        let (route, artifact) = onnx_route_and_artifact(&manifest);
        assert!(contract::validate_contract(&manifest, route, artifact).is_err());

        let mut manifest = release();
        manifest.contract.postprocess["stem_order"] = serde_json::json!(["instrumental", "vocals"]);
        let (route, artifact) = onnx_route_and_artifact(&manifest);
        assert!(contract::validate_contract(&manifest, route, artifact).is_err());
    }

    #[test]
    fn whole_track_mono_reference_uses_sample_standard_deviation() {
        let mut stats = TrackStatsBuilder::new(SAMPLE_RATE, CHANNELS).unwrap();
        stats.push(&[0.0, 2.0, 4.0], &[2.0, 4.0, 6.0]).unwrap();
        let normalization = stats.finish().unwrap();
        assert_eq!(normalization.samples(), 3);
        assert!((normalization.mean() - 3.0).abs() < 1e-6);
        assert!((normalization.std() - 2.0).abs() < 1e-6);
    }

    #[test]
    fn format_and_statistics_inputs_are_validated() {
        assert!(TrackStatsBuilder::new(48_000, 2).is_err());
        assert!(TrackStatsBuilder::new(SAMPLE_RATE, 1).is_err());
        let mut stats = TrackStatsBuilder::new(SAMPLE_RATE, CHANNELS).unwrap();
        assert!(stats.push(&[0.0], &[]).is_err());
        assert!(stats.push_interleaved(&[0.0]).is_err());
        assert!(stats.push(&[f32::NAN], &[0.0]).is_err());
        assert!(stats.finish().is_err());
    }

    #[test]
    fn triangular_window_matches_the_release_formula() {
        assert_eq!(triangular_weight(0), 1.0 / WINDOW_HALF as f32);
        assert_eq!(triangular_weight(WINDOW_HALF - 1), 1.0);
        assert_eq!(triangular_weight(WINDOW_HALF), 1.0);
        assert_eq!(
            triangular_weight(SEGMENT_SAMPLES - 1),
            1.0 / WINDOW_HALF as f32
        );
    }
}
