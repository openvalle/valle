//! Product bridge for RIFE's shared ordered-frame-pair runtime.

use crate::models::inference::rife::{InterpolationPosition, RifeSession, SessionOptions};
use anyhow::{Result, ensure};

use crate::{frame::RgbaFrame, models::ResolvedModel};

pub(crate) const MODEL_ID: &str = "rife";

pub(crate) struct InterpolationSession {
    inner: RifeSession,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct InterpolatedFrame {
    pub(crate) frame: RgbaFrame,
    pub(crate) inference_seconds: f64,
    pub(crate) total_seconds: f64,
}

impl InterpolationSession {
    pub(crate) fn open(resolved: &ResolvedModel, cpu_threads: usize) -> Result<Self> {
        ensure!(cpu_threads > 0, "RIFE cpu_threads must be positive");
        ensure!(
            resolved.resolved.id == MODEL_ID,
            "resolved model is not RIFE"
        );
        let inner = RifeSession::load(
            &resolved.manifest,
            &resolved.route,
            &resolved.artifact,
            &resolved.resolved.root,
            &SessionOptions {
                intra_threads: Some(cpu_threads),
                ..SessionOptions::default()
            },
        )?;
        Ok(Self { inner })
    }

    pub(crate) fn load_seconds(&self) -> f64 {
        self.inner.load_ms() / 1_000.0
    }

    pub(crate) fn interpolate(
        &mut self,
        left: &RgbaFrame,
        right: &RgbaFrame,
        position: f32,
    ) -> Result<InterpolatedFrame> {
        ensure!(
            left.width == right.width && left.height == right.height,
            "RIFE frame pair dimensions must match"
        );
        let left =
            crate::models::inference::rife::ImageView::rgba8(left.width, left.height, &left.data)?;
        let right = crate::models::inference::rife::ImageView::rgba8(
            right.width,
            right.height,
            &right.data,
        )?;
        let result = self
            .inner
            .interpolate(left, right, InterpolationPosition::new(position)?)?;
        Ok(InterpolatedFrame {
            frame: RgbaFrame {
                width: result.frame.width(),
                height: result.frame.height(),
                data: result.frame.into_pixels(),
            },
            inference_seconds: result.metrics.inference_ms / 1_000.0,
            total_seconds: result.metrics.total_ms / 1_000.0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_pair_boundary_rejects_mismatched_geometry() {
        let left = RgbaFrame::new(2, 2);
        let right = RgbaFrame::new(3, 2);
        assert_ne!((left.width, left.height), (right.width, right.height));
    }
}
