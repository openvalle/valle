//! Product bridge for the shared load-once shot-detection adapters.
//!
//! Complete media decoding and timestamp projection stay in `tools::shots`. This boundary accepts
//! one tightly packed, display-oriented RGBA8 frame at a time and delegates model-owned resize,
//! tensor packing, window and transition semantics to local inference modules. Detector selection is
//! explicit; this bridge never falls back across models.

use std::time::Instant;

use crate::models::inference::omnishotcut::{OmniShotCutSession, OrtWindowPredictor};
use crate::models::inference::transnetv2::{
    DetectionConfig as TransNetDetectionConfig, OrtSessionOptions as TransNetSessionOptions,
    TransNetV2Session,
};
use anyhow::{Context, Result, ensure};

use crate::{frame::RgbaFrame, models::manager::ResolvedModel};

pub(crate) const MODEL_ID: &str = "omnishotcut";
pub(crate) const TRANSNETV2_MODEL_ID: &str = "transnetv2";

pub(crate) fn supports_model(id: &str) -> bool {
    matches!(id, MODEL_ID | TRANSNETV2_MODEL_ID)
}

/// A resolved release. Opening the ORT session remains a distinct lifecycle step so media tools
/// can resolve before decoding and still report model-load time accurately.
pub(crate) struct ShotDetectionModel {
    model: ResolvedModel,
    cpu_threads: usize,
}

impl ShotDetectionModel {
    pub(crate) fn from_resolved(model: &ResolvedModel, cpu_threads: usize) -> Result<Self> {
        ensure!(
            cpu_threads > 0,
            "shot-detector cpu_threads must be positive"
        );
        ensure!(
            supports_model(&model.resolved.id),
            "unsupported shot detector {}; expected {MODEL_ID} or {TRANSNETV2_MODEL_ID}",
            model.resolved.id
        );
        Ok(Self {
            model: model.clone(),
            cpu_threads,
        })
    }

    pub(crate) fn open_session(&self) -> Result<ShotDetectionSession> {
        let started = Instant::now();
        let inner = match self.model.resolved.id.as_str() {
            MODEL_ID => {
                // Route options remain owned and validated by the factory adapter. The product
                // policy overrides only the per-job thread count, not artifact identity.
                let mut route = self.model.route.clone();
                let mut options = route_options(&route, "OmniShotCut")?;
                options.insert("intra_threads".to_owned(), self.cpu_threads.into());
                route.options = serde_json::Value::Object(options);
                ShotDetectorInner::OmniShotCut(OmniShotCutSession::<OrtWindowPredictor>::load(
                    &self.model.manifest,
                    &route,
                    &self.model.artifact,
                    &self.model.resolved.root,
                )?)
            }
            TRANSNETV2_MODEL_ID => {
                let options = route_options(&self.model.route, "TransNetV2")?;
                let bool_option = |name: &str, default: bool| -> Result<bool> {
                    options
                        .get(name)
                        .map(|value| {
                            value.as_bool().with_context(|| {
                                format!("TransNetV2 route option {name:?} must be boolean")
                            })
                        })
                        .transpose()
                        .map(|value| value.unwrap_or(default))
                };
                ShotDetectorInner::TransNetV2(TransNetV2Session::load(
                    &self.model.manifest,
                    &self.model.route,
                    &self.model.artifact,
                    &self.model.resolved.root,
                    TransNetDetectionConfig::default(),
                    TransNetSessionOptions {
                        intra_threads: Some(self.cpu_threads),
                        memory_pattern: bool_option("memory_pattern", true)?,
                        prepacking: bool_option("prepacking", true)?,
                    },
                )?)
            }
            _ => unreachable!("shot detector id was validated during resolution"),
        };
        Ok(ShotDetectionSession {
            inner,
            load_seconds: started.elapsed().as_secs_f64(),
            preprocess_seconds: 0.0,
            inference_seconds: 0.0,
        })
    }
}

pub(crate) struct ShotDetectionSession {
    inner: ShotDetectorInner,
    load_seconds: f64,
    preprocess_seconds: f64,
    inference_seconds: f64,
}

enum ShotDetectorInner {
    OmniShotCut(OmniShotCutSession<OrtWindowPredictor>),
    TransNetV2(TransNetV2Session),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ModelBoundary {
    /// Zero-based frame whose raw PTS becomes the canonical boundary timestamp.
    pub(crate) frame_index: u64,
    pub(crate) confidence: Option<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ModelTransition {
    OmniShotCut {
        start_frame: u64,
        end_frame_exclusive: u64,
        intra_index: u8,
        intra_label: &'static str,
        inter_index: u8,
        inter_label: &'static str,
    },
    TransNetV2 {
        start_frame: u64,
        end_frame_exclusive: u64,
        peak_frame: u64,
        peak_single_probability: f32,
        peak_all_frames_probability: f32,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ShotDetectionOutput {
    pub(crate) frame_count: u64,
    pub(crate) window_count: u64,
    pub(crate) max_buffered_frames: u64,
    pub(crate) boundaries: Vec<ModelBoundary>,
    pub(crate) transitions: Vec<ModelTransition>,
    pub(crate) preprocess_seconds: f64,
    pub(crate) inference_seconds: f64,
}

impl ShotDetectionSession {
    pub(crate) fn load_seconds(&self) -> f64 {
        self.load_seconds
    }

    /// Append one neutral media frame. Each factory adapter owns its complete preprocessing and
    /// copies only model-ready storage into its duration-independent rolling window.
    pub(crate) fn push_frame(&mut self, frame: &RgbaFrame) -> Result<()> {
        match &mut self.inner {
            ShotDetectorInner::OmniShotCut(inner) => {
                let timing = inner.push_rgba(
                    usize::try_from(frame.width).context("source width exceeds usize")?,
                    usize::try_from(frame.height).context("source height exceeds usize")?,
                    &frame.data,
                )?;
                self.preprocess_seconds += timing.preprocess_seconds;
                self.inference_seconds += timing.inference_seconds;
            }
            ShotDetectorInner::TransNetV2(inner) => {
                // TransNetV2 owns its 48x27 resize contract. Its public streaming call does not
                // expose separate resize/model timings, so the adapter wall time is conservatively
                // reported as inference rather than inventing a split.
                let inference_started = Instant::now();
                inner.push_rgba(
                    usize::try_from(frame.width).context("source width exceeds usize")?,
                    usize::try_from(frame.height).context("source height exceeds usize")?,
                    &frame.data,
                )?;
                self.inference_seconds += inference_started.elapsed().as_secs_f64();
            }
        }
        Ok(())
    }

    /// Flush the detector's final padded window and consume this one-stream product session.
    pub(crate) fn finish(mut self) -> Result<ShotDetectionOutput> {
        let inference_started = Instant::now();
        let output = match self.inner {
            ShotDetectorInner::OmniShotCut(mut inner) => {
                let result = inner.finish()?;
                let mut boundaries = Vec::new();
                let transitions = result
                    .shots
                    .into_iter()
                    .map(|shot| {
                        let start_frame = u64::try_from(shot.start_frame)
                            .context("OmniShotCut start frame exceeds u64")?;
                        let end_frame_exclusive = u64::try_from(shot.end_frame_exclusive)
                            .context("OmniShotCut end frame exceeds u64")?;
                        if end_frame_exclusive
                            < u64::try_from(result.frame_count)
                                .context("OmniShotCut frame count exceeds u64")?
                        {
                            // OmniShotCut exposes classes but no calibrated boundary probability.
                            boundaries.push(ModelBoundary {
                                frame_index: end_frame_exclusive,
                                confidence: None,
                            });
                        }
                        Ok(ModelTransition::OmniShotCut {
                            start_frame,
                            end_frame_exclusive,
                            intra_index: shot.intra.index(),
                            intra_label: shot.intra.label(),
                            inter_index: shot.inter.index(),
                            inter_label: shot.inter.label(),
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                ShotDetectionOutput {
                    frame_count: u64::try_from(result.frame_count)
                        .context("OmniShotCut frame count exceeds u64")?,
                    window_count: u64::try_from(result.window_count)
                        .context("OmniShotCut window count exceeds u64")?,
                    max_buffered_frames: u64::try_from(result.max_buffered_frames)
                        .context("OmniShotCut buffered-frame count exceeds u64")?,
                    boundaries,
                    transitions,
                    preprocess_seconds: self.preprocess_seconds,
                    inference_seconds: 0.0,
                }
            }
            ShotDetectorInner::TransNetV2(inner) => {
                let result = inner.finish()?;
                let boundaries = result
                    .boundaries
                    .iter()
                    .map(|boundary| {
                        Ok(ModelBoundary {
                            // A threshold run can span multiple frames. The peak probability has
                            // the least arbitrary single-frame identity for canonical ShotList.
                            frame_index: u64::try_from(boundary.peak_frame)
                                .context("TransNetV2 peak frame exceeds u64")?,
                            confidence: Some(boundary.peak_single_probability),
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                let transitions = result
                    .boundaries
                    .into_iter()
                    .map(|boundary| {
                        Ok(ModelTransition::TransNetV2 {
                            start_frame: u64::try_from(boundary.start_frame)
                                .context("TransNetV2 transition start exceeds u64")?,
                            end_frame_exclusive: u64::try_from(boundary.end_frame_exclusive)
                                .context("TransNetV2 transition end exceeds u64")?,
                            peak_frame: u64::try_from(boundary.peak_frame)
                                .context("TransNetV2 transition peak exceeds u64")?,
                            peak_single_probability: boundary.peak_single_probability,
                            peak_all_frames_probability: boundary.peak_all_frames_probability,
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                ShotDetectionOutput {
                    frame_count: u64::try_from(result.frame_count)
                        .context("TransNetV2 frame count exceeds u64")?,
                    window_count: u64::try_from(result.window_count)
                        .context("TransNetV2 window count exceeds u64")?,
                    max_buffered_frames: u64::try_from(result.max_buffered_frames)
                        .context("TransNetV2 buffered-frame count exceeds u64")?,
                    boundaries,
                    transitions,
                    preprocess_seconds: self.preprocess_seconds,
                    inference_seconds: 0.0,
                }
            }
        };
        self.inference_seconds += inference_started.elapsed().as_secs_f64();
        Ok(ShotDetectionOutput {
            inference_seconds: self.inference_seconds,
            ..output
        })
    }
}

fn route_options(
    route: &crate::models::spec::Route,
    model: &str,
) -> Result<serde_json::Map<String, serde_json::Value>> {
    if route.options.is_null() {
        Ok(serde_json::Map::new())
    } else {
        route
            .options
            .as_object()
            .cloned()
            .with_context(|| format!("{model} route options must be a JSON object"))
    }
}
