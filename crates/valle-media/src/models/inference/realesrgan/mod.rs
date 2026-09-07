//! Reusable Real-ESRGAN x4v3 host adapter and load-once ONNX session.
//!
//! The public image boundary is tightly packed RGB8/RGBA8. This crate owns the release contract's
//! RGB-to-NCHW `[0,1]` preprocessing, fixed-canvas symmetric padding, overlapping tile feathering,
//! 4x geometry, finite/shape checks, clamping and RGBA alpha preservation. It does not decode or
//! encode media, shell out, or retain frames between calls.

#[cfg(any(feature = "model-realesrgan-onnx", test))]
mod contract;
mod frame;
mod tile;

#[cfg(feature = "model-realesrgan-onnx")]
use std::path::Path;
#[cfg(feature = "model-realesrgan-onnx")]
use std::time::Instant;

#[cfg(feature = "model-realesrgan-onnx")]
use crate::models::runtime::TensorInput;
#[cfg(feature = "model-realesrgan-onnx")]
use crate::models::runtime::onnx::{OnnxSession, OnnxSessionOptions};
#[cfg(any(feature = "model-realesrgan-onnx", test))]
use crate::models::spec::{Artifact, ModelManifest, Route};
#[cfg(feature = "model-realesrgan-onnx")]
use anyhow::Context;
use anyhow::{Result, ensure};

pub use frame::{ImageFrame, ImageView, PixelFormat};
pub use tile::{TileBackend, TileOutput};

pub const ADAPTER: &str = "realesrgan-super-resolution";
pub const CONTRACT_VERSION: u32 = 1;
pub const SCALE: u32 = 4;
/// CoreML contract width. `release.v1.json` stores spatial axes as `[height, width]`.
pub const DEFAULT_TILE_WIDTH: u32 = 180;
/// CoreML contract height. `release.v1.json` stores spatial axes as `[height, width]`.
pub const DEFAULT_TILE_HEIGHT: u32 = 320;
pub const DEFAULT_OVERLAP: u32 = 16;
/// Hard ceiling for custom tile scratch memory (1024x1024 input pixels).
pub const MAX_TILE_PIXELS: u64 = 1_048_576;

/// Bounded per-frame tiling configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileConfig {
    pub width: u32,
    pub height: u32,
    pub overlap: u32,
}

impl TileConfig {
    pub fn new(width: u32, height: u32, overlap: u32) -> Result<Self> {
        let config = Self {
            width,
            height,
            overlap,
        };
        config.validate()?;
        Ok(config)
    }

    fn validate(self) -> Result<()> {
        ensure!(
            self.width > 0 && self.height > 0,
            "tile dimensions must be positive"
        );
        ensure!(
            self.overlap < self.width && self.overlap < self.height,
            "tile overlap must be smaller than both tile axes"
        );
        let pixels = u64::from(self.width) * u64::from(self.height);
        ensure!(
            pixels <= MAX_TILE_PIXELS,
            "tile area {pixels} exceeds the {MAX_TILE_PIXELS}-pixel scratch-memory limit"
        );
        Ok(())
    }
}

impl Default for TileConfig {
    fn default() -> Self {
        Self {
            width: DEFAULT_TILE_WIDTH,
            height: DEFAULT_TILE_HEIGHT,
            overlap: DEFAULT_OVERLAP,
        }
    }
}

/// Backend-neutral pre/post-processing adapter.
#[derive(Debug, Clone, Copy, Default)]
pub struct RealEsrganAdapter {
    tiles: TileConfig,
}

impl RealEsrganAdapter {
    pub fn new(tiles: TileConfig) -> Result<Self> {
        tiles.validate()?;
        Ok(Self { tiles })
    }

    pub const fn tile_config(&self) -> TileConfig {
        self.tiles
    }

    /// Upscale exactly one frame. Memory is bounded by one input frame, one output frame and the
    /// current tile/compositor; no state from this frame is retained after return.
    pub fn upscale(
        &self,
        backend: &mut impl TileBackend,
        input: ImageView<'_>,
    ) -> Result<(ImageFrame, usize)> {
        tile::upscale_tiled(input, self.tiles, backend)
    }
}

/// Load and inference options for the portable ONNX CPU route.
#[cfg(feature = "model-realesrgan-onnx")]
#[derive(Debug, Clone)]
pub struct SessionOptions {
    pub tiles: TileConfig,
    pub intra_threads: Option<usize>,
    pub memory_pattern: bool,
    pub prepacking: bool,
}

#[cfg(feature = "model-realesrgan-onnx")]
impl Default for SessionOptions {
    fn default() -> Self {
        Self {
            tiles: TileConfig::default(),
            intra_threads: None,
            memory_pattern: true,
            prepacking: true,
        }
    }
}

/// Timing and geometry from one frame call.
#[cfg(feature = "model-realesrgan-onnx")]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UpscaleMetrics {
    pub source_width: u32,
    pub source_height: u32,
    pub output_width: u32,
    pub output_height: u32,
    pub tiles: usize,
    pub inference_ms: f64,
    pub total_ms: f64,
}

#[cfg(feature = "model-realesrgan-onnx")]
#[derive(Debug, Clone, PartialEq)]
pub struct UpscaleResult {
    pub frame: ImageFrame,
    pub metrics: UpscaleMetrics,
}

/// Reusable load-once ONNX session.
///
/// Call [`Self::upscale`] once per decoded image/video frame. The mutable receiver serializes use
/// of the ORT session and ensures the graph is not reloaded between frames.
#[cfg(feature = "model-realesrgan-onnx")]
pub struct RealEsrganSession {
    session: OnnxSession,
    adapter: RealEsrganAdapter,
    input_name: String,
    output_name: String,
    load_ms: f64,
}

#[cfg(feature = "model-realesrgan-onnx")]
impl RealEsrganSession {
    pub fn load(
        manifest: &ModelManifest,
        route: &Route,
        artifact: &Artifact,
        artifact_root: &Path,
        options: &SessionOptions,
    ) -> Result<Self> {
        let names = contract::validate_contract(manifest, route, artifact)?;
        options.tiles.validate()?;
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
        .with_context(|| format!("failed to load Real-ESRGAN from {}", entrypoint.display()))?;
        Ok(Self {
            session,
            adapter: RealEsrganAdapter::new(options.tiles)?,
            input_name: names.input,
            output_name: names.output,
            load_ms: started.elapsed().as_secs_f64() * 1_000.0,
        })
    }

    pub const fn load_ms(&self) -> f64 {
        self.load_ms
    }

    pub const fn tile_config(&self) -> TileConfig {
        self.adapter.tile_config()
    }

    pub fn upscale(&mut self, input: ImageView<'_>) -> Result<UpscaleResult> {
        let source_width = input.width();
        let source_height = input.height();
        let input_name = self.input_name.clone();
        let output_name = self.output_name.clone();
        let started = Instant::now();
        let mut backend = OnnxBackend {
            session: &mut self.session,
            input_name: &input_name,
            output_name: &output_name,
            inference_ms: 0.0,
        };
        let (frame, tiles) = self.adapter.upscale(&mut backend, input)?;
        let metrics = UpscaleMetrics {
            source_width,
            source_height,
            output_width: frame.width(),
            output_height: frame.height(),
            tiles,
            inference_ms: backend.inference_ms,
            total_ms: started.elapsed().as_secs_f64() * 1_000.0,
        };
        Ok(UpscaleResult { frame, metrics })
    }
}

#[cfg(feature = "model-realesrgan-onnx")]
struct OnnxBackend<'a> {
    session: &'a mut OnnxSession,
    input_name: &'a str,
    output_name: &'a str,
    inference_ms: f64,
}

#[cfg(feature = "model-realesrgan-onnx")]
impl TileBackend for OnnxBackend<'_> {
    fn infer_tile(&mut self, input: &[f32], shape: [usize; 4]) -> Result<TileOutput> {
        let started = Instant::now();
        let output = self.session.run_f32(
            TensorInput::borrowed(self.input_name, shape, input),
            self.output_name,
        )?;
        self.inference_ms += started.elapsed().as_secs_f64() * 1_000.0;
        Ok(TileOutput {
            shape: output.shape,
            data: output.data,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release() -> ModelManifest {
        serde_json::from_str(include_str!("../../catalog/release-realesrgan-1.0.0.json")).unwrap()
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
        assert_eq!(names.input, "lr");
        assert_eq!(names.output, "sr");
        assert_eq!(
            TileConfig::default(),
            TileConfig::new(180, 320, 16).unwrap()
        );
    }

    #[test]
    fn contract_validation_rejects_adapter_or_tensor_drift() {
        let mut manifest = release();
        let (route, artifact) = onnx_route_and_artifact(&manifest);
        assert!(contract::validate_contract(&manifest, route, artifact).is_ok());

        manifest.contract.adapter = "wrong-adapter".into();
        let (route, artifact) = onnx_route_and_artifact(&manifest);
        assert!(contract::validate_contract(&manifest, route, artifact).is_err());

        let mut manifest = release();
        manifest.contract.outputs[0].name = "output".into();
        let (route, artifact) = onnx_route_and_artifact(&manifest);
        assert!(contract::validate_contract(&manifest, route, artifact).is_err());
    }

    #[test]
    fn tile_validation_is_bounded_and_unambiguous() {
        assert!(TileConfig::new(0, 320, 16).is_err());
        assert!(TileConfig::new(180, 320, 180).is_err());
        assert!(TileConfig::new(1025, 1025, 16).is_err());
        assert_eq!(TileConfig::new(180, 320, 0).unwrap().overlap, 0);
    }
}
