//! Product bridge for Real-ESRGAN's shared load-once tiled runtime.

use crate::models::inference::realesrgan::{RealEsrganSession, SessionOptions, TileConfig};
use anyhow::{Result, ensure};

use crate::{frame::RgbaFrame, models::ResolvedModel};

pub(crate) const MODEL_ID: &str = "realesrgan";

pub(crate) struct UpscaleSession {
    inner: RealEsrganSession,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct UpscaleFrameOutput {
    pub(crate) frame: RgbaFrame,
    pub(crate) tiles: usize,
    pub(crate) inference_seconds: f64,
    pub(crate) total_seconds: f64,
}

impl UpscaleSession {
    pub(crate) fn open(
        resolved: &ResolvedModel,
        cpu_threads: usize,
        tiles: TileConfig,
    ) -> Result<Self> {
        ensure!(cpu_threads > 0, "Real-ESRGAN cpu_threads must be positive");
        ensure!(
            resolved.resolved.id == MODEL_ID,
            "resolved model is not Real-ESRGAN"
        );
        let inner = RealEsrganSession::load(
            &resolved.manifest,
            &resolved.route,
            &resolved.artifact,
            &resolved.resolved.root,
            &SessionOptions {
                tiles,
                intra_threads: Some(cpu_threads),
                ..SessionOptions::default()
            },
        )?;
        Ok(Self { inner })
    }

    pub(crate) fn load_seconds(&self) -> f64 {
        self.inner.load_ms() / 1_000.0
    }

    pub(crate) fn upscale(&mut self, frame: &RgbaFrame) -> Result<UpscaleFrameOutput> {
        let input = crate::models::inference::realesrgan::ImageView::rgba8(
            frame.width,
            frame.height,
            &frame.data,
        )?;
        let result = self.inner.upscale(input)?;
        Ok(UpscaleFrameOutput {
            frame: RgbaFrame {
                width: result.frame.width(),
                height: result.frame.height(),
                data: result.frame.into_pixels(),
            },
            tiles: result.metrics.tiles,
            inference_seconds: result.metrics.inference_ms / 1_000.0,
            total_seconds: result.metrics.total_ms / 1_000.0,
        })
    }
}
