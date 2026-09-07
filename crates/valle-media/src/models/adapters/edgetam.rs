//! Product-side bridge for the shared EdgeTAM adapter.
//!
//! The local inference module owns every model-specific tensor, resize, prompt-scaling, mask
//! selection and memory-bank rule. This module only maps Valle's neutral [`RgbaFrame`] and
//! [`Gray8Frame`] values to that adapter and exposes one isolated, ordered job session. Candidate
//! selection, resize, sigmoid and Alpha8 quantization remain owned by the shared adapter.

use crate::models::inference::edgetam::{
    AdapterOptions, EdgeTamSession as RuntimeSession, OnnxBackend, OnnxEdgeTam, OnnxOptions,
    PointLabel, PointPrompt, PresentationTimestamp, PromptPoint, Rgba8Frame,
};
use anyhow::{Context, Result, ensure};

use crate::{
    frame::{Gray8Frame, RgbaFrame},
    models::manager::ResolvedModel,
};

pub(crate) const MODEL_ID: &str = "edgetam";

/// A model-neutral point label at the Valle media boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SegmentPointLabel {
    Negative,
    Positive,
}

/// One labeled point in display-oriented source-pixel coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct SegmentPoint {
    pub x: f32,
    pub y: f32,
    pub label: SegmentPointLabel,
}

/// EdgeTAM release v1's fixed first-frame prompt: exactly three points.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SegmentPrompt {
    points: [SegmentPoint; 3],
}

impl SegmentPrompt {
    pub(crate) fn new(points: [SegmentPoint; 3]) -> Result<Self> {
        ensure!(
            points
                .iter()
                .all(|point| point.x.is_finite() && point.y.is_finite()),
            "segment prompt coordinates must be finite"
        );
        ensure!(
            points
                .iter()
                .any(|point| point.label == SegmentPointLabel::Positive),
            "segment prompt requires at least one positive point"
        );
        Ok(Self { points })
    }

    pub(crate) const fn points(&self) -> &[SegmentPoint; 3] {
        &self.points
    }

    fn to_runtime(&self) -> Result<PointPrompt> {
        let points = self
            .points
            .map(|point| -> Result<PromptPoint> {
                PromptPoint::new(
                    point.x,
                    point.y,
                    match point.label {
                        SegmentPointLabel::Negative => PointLabel::Negative,
                        SegmentPointLabel::Positive => PointLabel::Positive,
                    },
                )
            })
            .into_iter()
            .collect::<Result<Vec<_>>>()?;
        let points: [PromptPoint; 3] = points
            .try_into()
            .map_err(|_| anyhow::anyhow!("EdgeTAM prompt point count drifted"))?;
        PointPrompt::new(points)
    }
}

/// Timing split reported by the shared adapter for one frame.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct SegmentFrameTiming {
    pub preprocess_seconds: f64,
    pub inference_seconds: f64,
    pub postprocess_seconds: f64,
}

/// A binary mask, model-owned soft alpha and timing split for one ordered frame.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SegmentFrameOutput {
    pub frame_index: u64,
    pub pts: i64,
    pub mask: Gray8Frame,
    pub alpha: Gray8Frame,
    pub timing: SegmentFrameTiming,
}

/// Backend-neutral session seam used by the complete-file tool and its mocks.
pub(crate) trait SegmentSession {
    fn initialize(
        &mut self,
        pts: i64,
        frame: &RgbaFrame,
        prompt: &SegmentPrompt,
    ) -> Result<SegmentFrameOutput>;

    fn track(&mut self, pts: i64, frame: &RgbaFrame) -> Result<SegmentFrameOutput>;
}

/// Load-once EdgeTAM model. Per-job memory lives only in values returned by
/// [`Self::start_session`].
pub(crate) struct EdgeTamModel {
    runtime: OnnxEdgeTam,
}

impl EdgeTamModel {
    pub(crate) fn open(
        model: &ResolvedModel,
        cpu_threads: usize,
        probability_threshold: f32,
        max_source_pixels: usize,
    ) -> Result<Self> {
        ensure!(
            model.resolved.id == MODEL_ID,
            "resolved model is not EdgeTAM"
        );
        ensure!(cpu_threads > 0, "EdgeTAM cpu_threads must be positive");
        ensure!(
            max_source_pixels > 0,
            "EdgeTAM max_source_pixels must be positive"
        );
        let mask_threshold = probability_threshold_to_logit(probability_threshold)?;
        let options = OnnxOptions {
            adapter: AdapterOptions {
                mask_threshold,
                max_source_pixels,
            },
            intra_threads: Some(cpu_threads),
            memory_pattern: true,
            prepacking: true,
        };
        let runtime = OnnxEdgeTam::load(
            &model.manifest,
            &model.route,
            &model.artifact,
            &model.resolved.root,
            &options,
        )
        .with_context(|| {
            format!(
                "open EdgeTAM artifact {} via route {}",
                model.resolved.artifact, model.resolved.route
            )
        })?;
        Ok(Self { runtime })
    }

    pub(crate) fn load_seconds(&self) -> f64 {
        self.runtime.load_ms() / 1_000.0
    }

    /// Start a clean job. Dropping the returned value discards its complete memory bank.
    pub(crate) fn start_session(&mut self) -> EdgeTamJob<'_> {
        EdgeTamJob {
            runtime: self.runtime.start_session(),
        }
    }
}

pub(crate) struct EdgeTamJob<'a> {
    runtime: RuntimeSession<'a, OnnxBackend>,
}

impl SegmentSession for EdgeTamJob<'_> {
    fn initialize(
        &mut self,
        pts: i64,
        frame: &RgbaFrame,
        prompt: &SegmentPrompt,
    ) -> Result<SegmentFrameOutput> {
        let prompt = prompt.to_runtime()?;
        let frame = runtime_frame(frame)?;
        let output = self
            .runtime
            .initialize(PresentationTimestamp::new(pts), frame, &prompt)?;
        convert_output(output)
    }

    fn track(&mut self, pts: i64, frame: &RgbaFrame) -> Result<SegmentFrameOutput> {
        let frame = runtime_frame(frame)?;
        let output = self.runtime.track(PresentationTimestamp::new(pts), frame)?;
        convert_output(output)
    }
}

fn runtime_frame(frame: &RgbaFrame) -> Result<Rgba8Frame<'_>> {
    // The upstream constructor enforces tightly packed straight-alpha RGBA8 storage. It ignores
    // alpha (rather than premultiplying/compositing), preserving Valle's neutral straight-alpha
    // contract at this boundary.
    Rgba8Frame::new(frame.width, frame.height, &frame.data)
}

fn convert_output(
    output: crate::models::inference::edgetam::SegmentationFrame,
) -> Result<SegmentFrameOutput> {
    let mask = Gray8Frame::from_data(
        output.mask.width(),
        output.mask.height(),
        output.mask.pixels().to_vec(),
    )
    .map_err(anyhow::Error::msg)?;
    ensure!(
        mask.is_binary_mask(),
        "EdgeTAM adapter returned a non-binary mask"
    );
    let alpha = Gray8Frame::from_data(
        output.alpha.width(),
        output.alpha.height(),
        output.alpha.pixels().to_vec(),
    )
    .map_err(anyhow::Error::msg)?;
    ensure!(
        alpha.width == mask.width && alpha.height == mask.height,
        "EdgeTAM adapter returned mask/alpha geometry drift"
    );
    let inference_ms = output.metrics.encode_ms
        + output.metrics.memory_attention_ms
        + output.metrics.decode_ms
        + output.metrics.memory_encode_ms;
    Ok(SegmentFrameOutput {
        frame_index: output.frame_index,
        pts: output.pts.value(),
        mask,
        alpha,
        timing: SegmentFrameTiming {
            preprocess_seconds: output.metrics.preprocess_ms / 1_000.0,
            inference_seconds: inference_ms / 1_000.0,
            postprocess_seconds: output.metrics.postprocess_ms / 1_000.0,
        },
    })
}

fn probability_threshold_to_logit(probability: f32) -> Result<f32> {
    ensure!(
        probability.is_finite() && probability > 0.0 && probability < 1.0,
        "segment probability threshold must be finite and strictly between 0 and 1"
    );
    Ok((probability / (1.0 - probability)).ln())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probability_threshold_maps_to_the_runtime_logit_domain() {
        assert_eq!(probability_threshold_to_logit(0.5).unwrap(), 0.0);
        assert!(probability_threshold_to_logit(0.75).unwrap() > 0.0);
        assert!(probability_threshold_to_logit(0.25).unwrap() < 0.0);
        assert!(probability_threshold_to_logit(0.0).is_err());
        assert!(probability_threshold_to_logit(1.0).is_err());
        assert!(probability_threshold_to_logit(f32::NAN).is_err());
    }

    #[test]
    fn neutral_prompt_requires_finite_coordinates_and_foreground() {
        let positive = SegmentPoint {
            x: 4.0,
            y: 5.0,
            label: SegmentPointLabel::Positive,
        };
        let negative = SegmentPoint {
            x: 0.0,
            y: 0.0,
            label: SegmentPointLabel::Negative,
        };
        assert!(SegmentPrompt::new([positive, positive, negative]).is_ok());
        assert!(SegmentPrompt::new([negative; 3]).is_err());
        assert!(
            SegmentPrompt::new([
                positive,
                SegmentPoint {
                    x: f32::NAN,
                    ..negative
                },
                negative,
            ])
            .is_err()
        );
    }

    #[test]
    fn rgba_bridge_keeps_straight_rgb_when_alpha_is_zero() {
        let frame = RgbaFrame {
            width: 1,
            height: 1,
            data: vec![10, 20, 30, 0],
        };
        let bridged = runtime_frame(&frame).unwrap();
        assert_eq!(bridged.pixels(), &[10, 20, 30, 0]);
    }
}
