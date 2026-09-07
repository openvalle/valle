//! Reusable RIFE v4.25 host adapter and load-once ONNX session.
//!
//! The adapter owns every tensor detail: RGB conversion, frame concatenation, right/bottom
//! zero-padding to 128, NCHW layout, tensor names, timestep shape, output validation, crop and
//! quantization. Callers provide two ordinary RGB/RGBA buffers and a checked interpolation
//! position. No media decoding, shell process, shot detection or whole-video cache lives here.

mod adapter;
#[cfg(any(feature = "model-rife-onnx", test))]
mod contract;
mod frame;

#[cfg(feature = "model-rife-onnx")]
use std::path::Path;
#[cfg(feature = "model-rife-onnx")]
use std::time::Instant;

#[cfg(feature = "model-rife-onnx")]
use crate::models::runtime::TensorInput;
#[cfg(feature = "model-rife-onnx")]
use crate::models::runtime::onnx::{OnnxSession, OnnxSessionOptions};
#[cfg(any(feature = "model-rife-onnx", test))]
use crate::models::spec::{Artifact, ModelManifest, Route};
#[cfg(feature = "model-rife-onnx")]
use anyhow::Context;
use anyhow::{Result, ensure};

pub use adapter::{AdapterOptions, ModelOutput, PaddedGeometry, RifeAdapter, RifeBackend};
pub use frame::{ImageFrame, ImageView, PixelFormat};

pub const ADAPTER: &str = "rife-frame-interpolation";
pub const CONTRACT_VERSION: u32 = 1;
pub const PAD_MULTIPLE: u32 = 128;

/// Finite interpolation timestep in the inclusive `[0,1]` interval.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InterpolationPosition(f32);

impl InterpolationPosition {
    pub fn new(value: f32) -> Result<Self> {
        ensure!(
            value.is_finite() && (0.0..=1.0).contains(&value),
            "RIFE interpolation position must be finite and within [0,1]"
        );
        Ok(Self(value))
    }

    pub const fn get(self) -> f32 {
        self.0
    }
}

#[cfg(feature = "model-rife-onnx")]
#[derive(Debug, Clone)]
pub struct SessionOptions {
    pub adapter: AdapterOptions,
    pub intra_threads: Option<usize>,
    pub memory_pattern: bool,
    pub prepacking: bool,
}

#[cfg(feature = "model-rife-onnx")]
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

#[cfg(feature = "model-rife-onnx")]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InterpolationMetrics {
    pub source_width: u32,
    pub source_height: u32,
    pub padded_width: u32,
    pub padded_height: u32,
    pub position: f32,
    pub inference_ms: f64,
    pub total_ms: f64,
}

#[cfg(feature = "model-rife-onnx")]
#[derive(Debug, Clone, PartialEq)]
pub struct InterpolationResult {
    pub frame: ImageFrame,
    pub metrics: InterpolationMetrics,
}

/// One reusable ONNX session for an ordered stream of in-shot frame pairs.
///
/// The session retains model/runtime state only. Each call owns and drops its pair-specific input
/// and output tensors, so memory use is independent of video duration. The caller remains
/// responsible for never sending a pair across a shot boundary.
#[cfg(feature = "model-rife-onnx")]
pub struct RifeSession {
    session: OnnxSession,
    adapter: RifeAdapter,
    frames_name: String,
    timestep_name: String,
    output_name: String,
    load_ms: f64,
}

#[cfg(feature = "model-rife-onnx")]
impl RifeSession {
    pub fn load(
        manifest: &ModelManifest,
        route: &Route,
        artifact: &Artifact,
        artifact_root: &Path,
        options: &SessionOptions,
    ) -> Result<Self> {
        let names = contract::validate_contract(manifest, route, artifact)?;
        let adapter = RifeAdapter::new(options.adapter)?;
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
        .with_context(|| format!("failed to load RIFE from {}", entrypoint.display()))?;
        Ok(Self {
            session,
            adapter,
            frames_name: names.frames,
            timestep_name: names.timestep,
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

    pub fn interpolate(
        &mut self,
        frame0: ImageView<'_>,
        frame1: ImageView<'_>,
        position: InterpolationPosition,
    ) -> Result<InterpolationResult> {
        let started = Instant::now();
        let adapter = self.adapter;
        let mut backend = OnnxBackend {
            session: &mut self.session,
            frames_name: &self.frames_name,
            timestep_name: &self.timestep_name,
            output_name: &self.output_name,
            inference_ms: 0.0,
        };
        let (frame, geometry) = adapter.interpolate(&mut backend, frame0, frame1, position)?;
        let metrics = InterpolationMetrics {
            source_width: geometry.source_width,
            source_height: geometry.source_height,
            padded_width: geometry.padded_width,
            padded_height: geometry.padded_height,
            position: position.get(),
            inference_ms: backend.inference_ms,
            total_ms: started.elapsed().as_secs_f64() * 1_000.0,
        };
        Ok(InterpolationResult { frame, metrics })
    }
}

#[cfg(feature = "model-rife-onnx")]
struct OnnxBackend<'a> {
    session: &'a mut OnnxSession,
    frames_name: &'a str,
    timestep_name: &'a str,
    output_name: &'a str,
    inference_ms: f64,
}

#[cfg(feature = "model-rife-onnx")]
impl RifeBackend for OnnxBackend<'_> {
    fn infer(
        &mut self,
        frames: &[f32],
        frames_shape: [usize; 4],
        timestep: f32,
    ) -> Result<ModelOutput> {
        let timestep_data = [timestep];
        let inputs = [
            TensorInput::borrowed(self.frames_name, frames_shape, frames),
            TensorInput::borrowed(self.timestep_name, [1, 1, 1, 1], &timestep_data),
        ];
        let started = Instant::now();
        let mut outputs = self.session.run_f32_many(&inputs, &[self.output_name])?;
        self.inference_ms += started.elapsed().as_secs_f64() * 1_000.0;
        let output = outputs.pop().context("RIFE ONNX run returned no output")?;
        Ok(ModelOutput {
            shape: output.shape,
            data: output.data,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release() -> ModelManifest {
        serde_json::from_str(include_str!("../../catalog/release-rife-1.0.0.json")).unwrap()
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
        assert_eq!(names.frames, "x");
        assert_eq!(names.timestep, "t");
        assert_eq!(names.output, "mid");
    }

    #[test]
    fn contract_validation_rejects_adapter_tensor_and_padding_drift() {
        let mut manifest = release();
        manifest.contract.adapter = "wrong-adapter".into();
        let (route, artifact) = onnx_route_and_artifact(&manifest);
        assert!(contract::validate_contract(&manifest, route, artifact).is_err());

        let mut manifest = release();
        manifest.contract.inputs[0].name = "frames".into();
        let (route, artifact) = onnx_route_and_artifact(&manifest);
        assert!(contract::validate_contract(&manifest, route, artifact).is_err());

        let mut manifest = release();
        manifest.contract.preprocess["pad_dimension_multiple"] = 64.into();
        let (route, artifact) = onnx_route_and_artifact(&manifest);
        assert!(contract::validate_contract(&manifest, route, artifact).is_err());
    }

    #[test]
    fn interpolation_position_is_explicit_and_checked() {
        assert_eq!(InterpolationPosition::new(0.25).unwrap().get(), 0.25);
        assert!(InterpolationPosition::new(-0.01).is_err());
        assert!(InterpolationPosition::new(1.01).is_err());
        assert!(InterpolationPosition::new(f32::NAN).is_err());
    }
}
