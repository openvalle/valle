//! Product bridge for the shared LaMa image-inpainting adapter.

use crate::models::inference::lama::{
    BinaryMaskView, CanvasBucket, InpaintPlan, LamaSession, SessionOptions, plan_inpaint,
};
use anyhow::{Context, Result, ensure};

use crate::{
    frame::{Gray8Frame, RgbaFrame},
    models::manager::ResolvedModel,
};

pub(crate) const MODEL_ID: &str = "lama";

/// A resolved LaMa release. The fixed-shape ONNX session is opened only after
/// the tool has inspected a mask and selected the required canvas bucket.
pub(crate) struct InpaintModel {
    model: ResolvedModel,
    cpu_threads: usize,
}

impl InpaintModel {
    pub(crate) fn from_resolved(model: &ResolvedModel, cpu_threads: usize) -> Result<Self> {
        ensure!(cpu_threads > 0, "LaMa cpu_threads must be positive");
        ensure!(model.resolved.id == MODEL_ID, "resolved model is not LaMa");
        Ok(Self {
            model: model.clone(),
            cpu_threads,
        })
    }

    pub(crate) fn plan(mask: &Gray8Frame) -> Result<InpaintPlan> {
        let view = binary_mask(mask)?;
        plan_inpaint(view)
    }

    pub(crate) fn open_session(&self, bucket: CanvasBucket) -> Result<InpaintSession> {
        let entrypoint = self.model.resolved.root.join(bucket.onnx_filename());
        let inner = LamaSession::load_with_options(
            &entrypoint,
            bucket,
            SessionOptions {
                intra_threads: Some(self.cpu_threads),
                ..SessionOptions::default()
            },
        )
        .with_context(|| {
            format!(
                "load LaMa {}x{} artifact from {}",
                bucket.width(),
                bucket.height(),
                entrypoint.display()
            )
        })?;
        Ok(InpaintSession { inner })
    }
}

pub(crate) struct InpaintSession {
    inner: LamaSession,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct InpaintFrameOutput {
    pub(crate) frame: RgbaFrame,
    pub(crate) plan: InpaintPlan,
    pub(crate) inference_seconds: f64,
    pub(crate) total_seconds: f64,
    pub(crate) tasks: usize,
    pub(crate) scaled_tasks: usize,
}

impl InpaintSession {
    pub(crate) fn load_seconds(&self) -> f64 {
        self.inner.load_ms() / 1_000.0
    }

    pub(crate) fn bucket(&self) -> CanvasBucket {
        self.inner.bucket()
    }

    pub(crate) fn inpaint(
        &mut self,
        image: &RgbaFrame,
        mask: &Gray8Frame,
    ) -> Result<InpaintFrameOutput> {
        ensure!(
            image.width == mask.width && image.height == mask.height,
            "LaMa image {}x{} and mask {}x{} must match exactly",
            image.width,
            image.height,
            mask.width,
            mask.height
        );
        let image = crate::models::inference::lama::ImageView::rgba8(
            image.width,
            image.height,
            &image.data,
        )?;
        let mask = binary_mask(mask)?;
        let result = self.inner.inpaint(image, mask)?;
        let frame = RgbaFrame {
            width: result.frame.width(),
            height: result.frame.height(),
            data: result.frame.into_pixels(),
        };
        Ok(InpaintFrameOutput {
            frame,
            plan: result.plan,
            inference_seconds: result.metrics.inference_ms / 1_000.0,
            total_seconds: result.metrics.total_ms / 1_000.0,
            tasks: result.metrics.tasks,
            scaled_tasks: result.metrics.scaled_tasks,
        })
    }
}

fn binary_mask(mask: &Gray8Frame) -> Result<BinaryMaskView<'_>> {
    BinaryMaskView::gray8(mask.width, mask.height, &mask.data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planning_preserves_the_strict_gray8_contract() {
        let valid = Gray8Frame::from_data(2, 2, vec![0, 255, 0, 0]).unwrap();
        assert!(InpaintModel::plan(&valid).is_ok());
        let invalid = Gray8Frame {
            width: 2,
            height: 2,
            data: vec![0, 127, 0, 0],
        };
        assert!(InpaintModel::plan(&invalid).is_err());
    }
}
