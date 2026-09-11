//! Complete-file shot detection with bounded sequential decoding.
//!
//! The primary artifact is a direct, canonical [`ShotList`] JSON document so downstream tools can
//! consume it without a run-report envelope. Model-specific transition details remain an extension
//! of [`ShotsResult`]. Raw source PTS values are spooled as eight-byte records: frame memory and
//! timestamp memory therefore remain independent of video duration.

use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    mem::size_of,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};

use crate::codec::TimeBase as Rational;
use serde::{Deserialize, Serialize};

use crate::{
    analysis::{ShotBoundary, ShotList},
    models::{
        ModelSelection,
        adapters::omnishotcut::{
            MODEL_ID, ModelTransition, ShotDetectionModel, ShotDetectionOutput,
            TRANSNETV2_MODEL_ID, supports_model,
        },
    },
};

use super::{
    MediaSummary, ModelProvenance, OutputArtifact, ProcessedMedia, ProcessingReport, RunContext,
    StageTiming, ToolError, ToolErrorCode, ToolEvent, ToolPhase, ToolRun, ToolWarning,
    model_session::ModelSessionCandidates,
    output::{FileOutputTransaction, paths_refer_to_same_file},
    video_sequence::{SequentialRgbaDecoder, VideoMetadata},
};

const PTS_RECORD_BYTES: u64 = size_of::<i64>() as u64;
static PTS_SPOOL_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShotsRequest {
    pub input: PathBuf,
    pub output: PathBuf,
    pub model: ModelSelection,
    pub overwrite: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TransitionClass {
    pub index: u8,
    pub label: String,
}

/// Detector-specific metadata retained outside the canonical `ShotList` artifact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "model", rename_all_fields = "camelCase")]
pub enum ShotTransition {
    #[serde(rename = "omnishotcut")]
    OmniShotCut {
        start_frame: u64,
        end_frame_exclusive: u64,
        start_seconds: f64,
        end_seconds: f64,
        intra: TransitionClass,
        inter: TransitionClass,
    },
    #[serde(rename = "transnetv2")]
    TransNetV2 {
        start_frame: u64,
        end_frame_exclusive: u64,
        peak_frame: u64,
        start_seconds: f64,
        end_seconds: f64,
        peak_seconds: f64,
        peak_single_probability: f32,
        peak_all_frames_probability: f32,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShotsResult {
    /// Canonical model-neutral payload. The primary output file serializes this value directly.
    pub shots: ShotList,
    /// Model-specific transition classes or probability runs, excluded from the primary file.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transitions: Vec<ShotTransition>,
    pub outputs: Vec<OutputArtifact>,
    pub frames: u64,
    pub width: u32,
    pub height: u32,
    pub duration_seconds: f64,
    pub model_windows: u64,
    pub max_buffered_model_frames: u64,
    pub milliseconds_per_frame: f64,
}

pub fn run(
    request: ShotsRequest,
    context: &mut RunContext<'_>,
) -> Result<ToolRun<ShotsResult>, ToolError> {
    context.validate()?;
    validate_request(&request)?;
    FileOutputTransaction::validate_target(&request.output, request.overwrite)?;
    let started = Instant::now();

    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Resolving));
    check_cancelled(context)?;
    let candidates = ModelSessionCandidates::resolve(context.models, request.model.clone())?;
    let transaction = FileOutputTransaction::new(&request.output, request.overwrite)?;

    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Decoding));
    let decode_started = Instant::now();
    let mut decoder = SequentialRgbaDecoder::open(&request.input, context.resources.cpu_threads)
        .map_err(|error| {
            ToolError::invalid_input(format!(
                "open shot-detection input {}: {error:#}",
                request.input.display()
            ))
        })?;
    let metadata = decoder.metadata();
    let mut decode_seconds = decode_started.elapsed().as_secs_f64();
    let mut pts = PtsSpool::create(
        &context.resources.temporary_directory,
        context.resources.temporary_disk_budget_bytes,
    )?;
    check_cancelled(context)?;

    context.progress.event(ToolEvent::phase(ToolPhase::Loading));
    let load_started = Instant::now();
    let opened = candidates.open("load shot detector", |model| {
        ShotDetectionModel::from_resolved(model, context.resources.cpu_threads)?.open_session()
    })?;
    let session = opened.session;
    let provenance = ModelProvenance::from(&opened.model.resolved);
    let route_warnings = opened.warnings;
    let load_seconds = load_started
        .elapsed()
        .as_secs_f64()
        .max(session.load_seconds());
    let mut session = session;
    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Preprocessing));

    let mut frames = 0_u64;
    loop {
        check_cancelled(context)?;
        let decode_started = Instant::now();
        let frame = decoder.next_frame().map_err(|error| {
            ToolError::invalid_input(format!("decode sequential shot frame: {error:#}"))
        })?;
        decode_seconds += decode_started.elapsed().as_secs_f64();
        let Some(frame) = frame else {
            break;
        };

        pts.push(frame.pts)?;
        session.push_frame(&frame.frame).map_err(|error| {
            ToolError::new(
                ToolErrorCode::InferenceFailed,
                format!("run {} on frame {frames}: {error:#}", request.model.id),
            )
        })?;
        frames = frames
            .checked_add(1)
            .ok_or_else(|| ToolError::new(ToolErrorCode::Internal, "frame count overflow"))?;
        context.progress.event(ToolEvent {
            phase: ToolPhase::Inferencing,
            completed: Some(frames),
            total: None,
            message: None,
        });
    }
    if frames == 0 {
        return Err(ToolError::invalid_input(
            "shot-detection input contains no decodable video frames",
        ));
    }
    check_cancelled(context)?;

    // `finish` may execute the final padded model window; keep it in inference timing.
    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Inferencing));
    let detected = session.finish().map_err(|error| {
        ToolError::new(
            ToolErrorCode::InferenceFailed,
            format!("finish {} shot detection: {error:#}", request.model.id),
        )
    })?;
    if detected.frame_count != frames {
        return Err(ToolError::new(
            ToolErrorCode::InferenceFailed,
            format!(
                "{} consumed {} frames but the decoder produced {frames}",
                request.model.id, detected.frame_count
            ),
        ));
    }
    pts.finish_writes()?;
    check_cancelled(context)?;

    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Postprocessing));
    let postprocess_started = Instant::now();
    let derived = derive_analysis(&detected, &mut pts, metadata)?;
    let postprocess_seconds = postprocess_started.elapsed().as_secs_f64();

    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Encoding));
    let encode_started = Instant::now();
    write_staged_shots(transaction.staging_path(), &derived.shots)?;
    let encode_seconds = encode_started.elapsed().as_secs_f64();

    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Validating));
    validate_staged_shots(transaction.staging_path(), &derived.shots)?;
    check_cancelled(context)?;
    let output_path = transaction.commit()?;

    let output = OutputArtifact {
        role: "shots".to_owned(),
        path: output_path,
        media_type: "application/json".to_owned(),
        summary: Some(MediaSummary {
            duration_seconds: Some(derived.shots.duration_seconds),
            frame_count: Some(frames),
            ..MediaSummary::default()
        }),
    };
    let total_seconds = started.elapsed().as_secs_f64();
    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Completed));

    let mut warnings = route_warnings;
    warnings.extend(detector_warnings(&request.model.id));
    Ok(ToolRun {
        result: ShotsResult {
            shots: derived.shots.clone(),
            transitions: derived.transitions,
            outputs: vec![output.clone()],
            frames,
            width: metadata.width,
            height: metadata.height,
            duration_seconds: derived.shots.duration_seconds,
            model_windows: detected.window_count,
            max_buffered_model_frames: detected.max_buffered_frames,
            milliseconds_per_frame: total_seconds * 1_000.0 / frames as f64,
        },
        report: ProcessingReport {
            operation: "shots".to_owned(),
            parameters: normalized_parameters(&request, metadata),
            input: request.input,
            input_media_type: "video/*".to_owned(),
            input_summary: MediaSummary {
                duration_seconds: Some(derived.shots.duration_seconds),
                width: Some(metadata.width),
                height: Some(metadata.height),
                frame_count: Some(frames),
                pixel_format: Some("rgba8-straight-display-oriented".to_owned()),
                ..MediaSummary::default()
            },
            outputs: vec![output],
            models: vec![provenance],
            processed: ProcessedMedia {
                frames,
                audio_seconds: 0.0,
            },
            timing: StageTiming {
                load_seconds,
                decode_seconds,
                preprocess_seconds: detected.preprocess_seconds,
                inference_seconds: detected.inference_seconds,
                postprocess_seconds,
                encode_seconds,
                total_seconds,
            },
        },
        warnings,
    })
}

struct DerivedAnalysis {
    shots: ShotList,
    transitions: Vec<ShotTransition>,
}

fn derive_analysis(
    detected: &ShotDetectionOutput,
    pts: &mut PtsSpool,
    metadata: VideoMetadata,
) -> Result<DerivedAnalysis, ToolError> {
    if detected.frame_count != pts.count {
        return Err(ToolError::new(
            ToolErrorCode::Internal,
            "model frame count and timestamp spool length differ",
        ));
    }
    let duration_seconds = source_duration_seconds(pts, metadata)?;
    let mut boundaries = Vec::with_capacity(detected.boundaries.len());
    for boundary in &detected.boundaries {
        if let Some(confidence) = boundary.confidence
            && (!confidence.is_finite() || !(0.0..=1.0).contains(&confidence))
        {
            return Err(ToolError::new(
                ToolErrorCode::InferenceFailed,
                format!(
                    "shot boundary at frame {} has invalid confidence {confidence}",
                    boundary.frame_index
                ),
            ));
        }
        boundaries.push(ShotBoundary {
            frame_index: boundary.frame_index,
            pts_seconds: frame_boundary_seconds(
                pts,
                boundary.frame_index,
                metadata,
                duration_seconds,
            )?,
            confidence: boundary.confidence,
        });
    }
    let shots = ShotList::from_boundaries(duration_seconds, boundaries).map_err(|error| {
        ToolError::new(
            ToolErrorCode::InferenceFailed,
            format!("build canonical shot list: {error}"),
        )
    })?;

    let mut transitions = Vec::with_capacity(detected.transitions.len());
    for transition in &detected.transitions {
        transitions.push(match transition {
            ModelTransition::OmniShotCut {
                start_frame,
                end_frame_exclusive,
                intra_index,
                intra_label,
                inter_index,
                inter_label,
            } => ShotTransition::OmniShotCut {
                start_frame: *start_frame,
                end_frame_exclusive: *end_frame_exclusive,
                start_seconds: frame_boundary_seconds(
                    pts,
                    *start_frame,
                    metadata,
                    duration_seconds,
                )?,
                end_seconds: frame_boundary_seconds(
                    pts,
                    *end_frame_exclusive,
                    metadata,
                    duration_seconds,
                )?,
                intra: TransitionClass {
                    index: *intra_index,
                    label: (*intra_label).to_owned(),
                },
                inter: TransitionClass {
                    index: *inter_index,
                    label: (*inter_label).to_owned(),
                },
            },
            ModelTransition::TransNetV2 {
                start_frame,
                end_frame_exclusive,
                peak_frame,
                peak_single_probability,
                peak_all_frames_probability,
            } => {
                for (name, probability) in [
                    ("single-frame", *peak_single_probability),
                    ("all-frames", *peak_all_frames_probability),
                ] {
                    if !probability.is_finite() || !(0.0..=1.0).contains(&probability) {
                        return Err(ToolError::new(
                            ToolErrorCode::InferenceFailed,
                            format!(
                                "TransNetV2 {name} probability at frame {peak_frame} is {probability}"
                            ),
                        ));
                    }
                }
                ShotTransition::TransNetV2 {
                    start_frame: *start_frame,
                    end_frame_exclusive: *end_frame_exclusive,
                    peak_frame: *peak_frame,
                    start_seconds: frame_boundary_seconds(
                        pts,
                        *start_frame,
                        metadata,
                        duration_seconds,
                    )?,
                    end_seconds: frame_boundary_seconds(
                        pts,
                        *end_frame_exclusive,
                        metadata,
                        duration_seconds,
                    )?,
                    peak_seconds: frame_boundary_seconds(
                        pts,
                        *peak_frame,
                        metadata,
                        duration_seconds,
                    )?,
                    peak_single_probability: *peak_single_probability,
                    peak_all_frames_probability: *peak_all_frames_probability,
                }
            }
        });
    }

    Ok(DerivedAnalysis { shots, transitions })
}

fn source_duration_seconds(pts: &PtsSpool, metadata: VideoMetadata) -> Result<f64, ToolError> {
    let last_pts = pts.last_pts.ok_or_else(|| {
        ToolError::invalid_input("cannot derive shot duration from an empty video")
    })?;
    let tick_seconds = rational_seconds(metadata.time_base)?;
    let last_local_ticks = last_pts.checked_sub(metadata.start_pts).ok_or_else(|| {
        ToolError::invalid_input("source-local video timestamp conversion overflowed")
    })?;
    if last_local_ticks < 0 {
        return Err(ToolError::invalid_input(format!(
            "first/last video PTS precedes declared stream start {}",
            metadata.start_pts
        )));
    }
    let last_local_seconds = last_local_ticks as f64 * tick_seconds;

    if let Some(duration_ticks) = metadata.duration_ticks {
        let declared = duration_ticks as f64 * tick_seconds;
        if declared.is_finite() && declared > last_local_seconds {
            return Ok(declared);
        }
    }

    let inferred_step = pts
        .last_delta_ticks
        .filter(|delta| *delta > 0)
        .map(|delta| delta as f64 * tick_seconds)
        .or_else(|| {
            (metadata.nominal_fps.is_finite() && metadata.nominal_fps > 0.0)
                .then(|| 1.0 / metadata.nominal_fps)
        })
        .unwrap_or(tick_seconds);
    let duration = last_local_seconds + inferred_step;
    if !duration.is_finite() || duration <= 0.0 {
        return Err(ToolError::invalid_input(
            "video duration cannot be derived from stream PTS",
        ));
    }
    Ok(duration)
}

fn frame_boundary_seconds(
    pts: &mut PtsSpool,
    frame_index: u64,
    metadata: VideoMetadata,
    duration_seconds: f64,
) -> Result<f64, ToolError> {
    if frame_index > pts.count {
        return Err(ToolError::new(
            ToolErrorCode::InferenceFailed,
            format!(
                "model boundary frame {frame_index} exceeds decoded frame count {}",
                pts.count
            ),
        ));
    }
    if frame_index == 0 {
        return Ok(0.0);
    }
    if frame_index == pts.count {
        return Ok(duration_seconds);
    }
    let raw_pts = pts.read(frame_index)?;
    let local_ticks = raw_pts.checked_sub(metadata.start_pts).ok_or_else(|| {
        ToolError::invalid_input("source-local video timestamp conversion overflowed")
    })?;
    if local_ticks < 0 {
        return Err(ToolError::invalid_input(format!(
            "video PTS {raw_pts} precedes declared stream start {}",
            metadata.start_pts
        )));
    }
    let seconds = local_ticks as f64 * rational_seconds(metadata.time_base)?;
    if !seconds.is_finite() || seconds < 0.0 || seconds > duration_seconds {
        return Err(ToolError::new(
            ToolErrorCode::InferenceFailed,
            format!("model boundary frame {frame_index} maps to invalid source time {seconds}"),
        ));
    }
    Ok(seconds)
}

fn rational_seconds(value: Rational) -> Result<f64, ToolError> {
    if value.numerator() <= 0 || value.denominator() <= 0 {
        return Err(ToolError::invalid_input(format!(
            "video stream has invalid time base {}/{}",
            value.numerator(),
            value.denominator()
        )));
    }
    Ok(value.numerator() as f64 / value.denominator() as f64)
}

fn write_staged_shots(path: &Path, shots: &ShotList) -> Result<(), ToolError> {
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|error| output_error("create staged shots JSON", error))?;
    serde_json::to_writer_pretty(&mut file, shots)
        .map_err(|error| output_error("serialize shots JSON", error))?;
    file.write_all(b"\n")
        .map_err(|error| output_error("write shots JSON", error))?;
    file.sync_all()
        .map_err(|error| output_error("flush shots JSON", error))?;
    Ok(())
}

fn validate_staged_shots(path: &Path, expected: &ShotList) -> Result<(), ToolError> {
    let file = File::open(path).map_err(|error| output_error("open staged shots JSON", error))?;
    let decoded: ShotList = serde_json::from_reader(file)
        .map_err(|error| output_error("parse staged shots JSON", error))?;
    decoded
        .validate()
        .map_err(|error| output_error("validate staged shots JSON", error))?;
    if decoded != *expected {
        return Err(ToolError::new(
            ToolErrorCode::OutputValidationFailed,
            "staged shots JSON did not round-trip exactly",
        ));
    }
    Ok(())
}

fn normalized_parameters(
    request: &ShotsRequest,
    metadata: VideoMetadata,
) -> BTreeMap<String, serde_json::Value> {
    let mut parameters = BTreeMap::new();
    parameters.insert("model".to_owned(), request.model.id.clone().into());
    parameters.insert(
        "modelVersion".to_owned(),
        request
            .model
            .version
            .clone()
            .map_or(serde_json::Value::Null, Into::into),
    );
    parameters.insert(
        "backend".to_owned(),
        serde_json::to_value(request.model.backend).expect("backend is serializable"),
    );
    parameters.insert(
        "timestampPolicy".to_owned(),
        "raw_pts_rebased_to_stream_start".into(),
    );
    parameters.insert(
        "sourceTimeBase".to_owned(),
        format!(
            "{}/{}",
            metadata.time_base.numerator(),
            metadata.time_base.denominator()
        )
        .into(),
    );
    parameters.insert("sourceStartPts".to_owned(), metadata.start_pts.into());
    parameters.insert(
        "sourceRotationDegrees".to_owned(),
        metadata.rotation_degrees.into(),
    );
    parameters.insert(
        "sourceSampleAspectRatio".to_owned(),
        format!(
            "{}/{}",
            metadata.sample_aspect_ratio.numerator(),
            metadata.sample_aspect_ratio.denominator()
        )
        .into(),
    );
    parameters
}

fn detector_warnings(model_id: &str) -> Vec<ToolWarning> {
    if model_id == MODEL_ID {
        vec![ToolWarning {
            code: "boundary_confidence_unavailable".to_owned(),
            message: "OmniShotCut emits discrete ranges and transition classes, not calibrated boundary probabilities; canonical boundaries therefore omit confidence"
                .to_owned(),
        }]
    } else {
        Vec::new()
    }
}

fn validate_request(request: &ShotsRequest) -> Result<(), ToolError> {
    if !request.input.is_file() {
        return Err(ToolError::invalid_input(format!(
            "shot-detection input does not exist or is not a file: {}",
            request.input.display()
        )));
    }
    if !has_extension(&request.output, "json") {
        return Err(ToolError::invalid_input(
            "shot-detection output must end in .json",
        ));
    }
    if paths_refer_to_same_file(&request.input, &request.output) {
        return Err(ToolError::invalid_input(
            "shot-detection input and output must be different files",
        ));
    }
    if !supports_model(&request.model.id) {
        return Err(ToolError::new(
            ToolErrorCode::UnsupportedAdapter,
            format!(
                "unsupported shot detector {:?}; choose {MODEL_ID} or {TRANSNETV2_MODEL_ID}",
                request.model.id
            ),
        ));
    }
    Ok(())
}

fn has_extension(path: &Path, wanted: &str) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case(wanted))
}

fn output_error(action: &str, error: impl std::fmt::Display) -> ToolError {
    ToolError::new(
        ToolErrorCode::OutputValidationFailed,
        format!("{action}: {error}"),
    )
}

fn check_cancelled(context: &RunContext<'_>) -> Result<(), ToolError> {
    if context.cancellation.is_cancelled() {
        Err(ToolError::cancelled())
    } else {
        Ok(())
    }
}

struct PtsSpool {
    file: Option<File>,
    path: PathBuf,
    budget_bytes: u64,
    count: u64,
    last_pts: Option<i64>,
    last_delta_ticks: Option<i64>,
}

impl PtsSpool {
    fn create(directory: &Path, budget_bytes: u64) -> Result<Self, ToolError> {
        let (file, path) = (0..16)
            .find_map(|_| {
                let sequence = PTS_SPOOL_SEQUENCE.fetch_add(1, Ordering::Relaxed);
                let path = directory.join(format!(
                    ".valle-shots-pts-{}-{sequence}.bin",
                    std::process::id()
                ));
                let mut options = OpenOptions::new();
                options.create_new(true).read(true).write(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt as _;
                    options.mode(0o600);
                }
                match options.open(&path) {
                    Ok(file) => Some(Ok((file, path))),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => None,
                    Err(error) => Some(Err(error)),
                }
            })
            .unwrap_or_else(|| {
                Err(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    "too many temporary shot timestamp spool collisions",
                ))
            })
            .map_err(|error| {
                ToolError::new(
                    ToolErrorCode::ResourceBusy,
                    format!("create temporary shot timestamp spool: {error}"),
                )
            })?;
        Ok(Self {
            file: Some(file),
            path,
            budget_bytes,
            count: 0,
            last_pts: None,
            last_delta_ticks: None,
        })
    }

    fn push(&mut self, pts: i64) -> Result<(), ToolError> {
        if let Some(previous) = self.last_pts {
            let delta = pts.checked_sub(previous).ok_or_else(|| {
                ToolError::invalid_input("video PTS delta overflowed while spooling")
            })?;
            if delta <= 0 {
                return Err(ToolError::invalid_input(format!(
                    "video PTS is not strictly increasing: {pts} <= {previous}"
                )));
            }
            self.last_delta_ticks = Some(delta);
        }
        let next_count = self
            .count
            .checked_add(1)
            .ok_or_else(|| ToolError::new(ToolErrorCode::Internal, "PTS count overflow"))?;
        let required = next_count.checked_mul(PTS_RECORD_BYTES).ok_or_else(|| {
            ToolError::new(ToolErrorCode::ResourceBusy, "PTS spool size overflow")
        })?;
        if required > self.budget_bytes {
            return Err(ToolError::new(
                ToolErrorCode::ResourceBusy,
                format!(
                    "shot timestamp spool requires {required} bytes, exceeding the temporary-disk budget of {} bytes",
                    self.budget_bytes
                ),
            ));
        }
        self.file_mut()?
            .write_all(&pts.to_le_bytes())
            .map_err(|error| {
                ToolError::new(
                    ToolErrorCode::ResourceBusy,
                    format!("write temporary shot timestamp spool: {error}"),
                )
            })?;
        self.count = next_count;
        self.last_pts = Some(pts);
        Ok(())
    }

    fn finish_writes(&mut self) -> Result<(), ToolError> {
        self.file_mut()?.sync_data().map_err(|error| {
            ToolError::new(
                ToolErrorCode::ResourceBusy,
                format!("flush temporary shot timestamp spool: {error}"),
            )
        })
    }

    fn read(&mut self, index: u64) -> Result<i64, ToolError> {
        if index >= self.count {
            return Err(ToolError::new(
                ToolErrorCode::Internal,
                format!("PTS spool index {index} exceeds {} records", self.count),
            ));
        }
        let offset = index
            .checked_mul(PTS_RECORD_BYTES)
            .ok_or_else(|| ToolError::new(ToolErrorCode::Internal, "PTS spool offset overflow"))?;
        let file = self.file_mut()?;
        file.seek(SeekFrom::Start(offset)).map_err(|error| {
            ToolError::new(
                ToolErrorCode::Internal,
                format!("seek temporary shot timestamp spool: {error}"),
            )
        })?;
        let mut bytes = [0_u8; size_of::<i64>()];
        file.read_exact(&mut bytes).map_err(|error| {
            ToolError::new(
                ToolErrorCode::Internal,
                format!("read temporary shot timestamp spool: {error}"),
            )
        })?;
        Ok(i64::from_le_bytes(bytes))
    }

    fn file_mut(&mut self) -> Result<&mut File, ToolError> {
        self.file.as_mut().ok_or_else(|| {
            ToolError::new(
                ToolErrorCode::Internal,
                "temporary shot timestamp spool is already closed",
            )
        })
    }
}

impl Drop for PtsSpool {
    fn drop(&mut self) {
        drop(self.file.take());
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::RunBackendPreference;

    fn metadata(duration_ticks: Option<i64>) -> VideoMetadata {
        VideoMetadata {
            width: 1920,
            height: 1080,
            duration_ticks,
            nominal_fps: 25.0,
            time_base: Rational(1, 1_000),
            start_pts: 100,
            rotation_degrees: 0,
            sample_aspect_ratio: Rational(1, 1),
        }
    }

    #[test]
    fn pts_spool_enforces_budget_supports_random_reads_and_cleans_up() {
        let root = tempfile::tempdir().unwrap();
        let path = {
            let mut spool = PtsSpool::create(root.path(), 2 * PTS_RECORD_BYTES).unwrap();
            let path = spool.path.clone();
            spool.push(100).unwrap();
            spool.push(140).unwrap();
            assert_eq!(
                spool.push(180).unwrap_err().code,
                ToolErrorCode::ResourceBusy
            );
            spool.finish_writes().unwrap();
            assert_eq!(spool.read(1).unwrap(), 140);
            assert_eq!(spool.read(0).unwrap(), 100);
            assert!(path.is_file());
            path
        };
        assert!(!path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn pts_spool_is_private_to_the_current_user() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = tempfile::tempdir().unwrap();
        let spool = PtsSpool::create(root.path(), PTS_RECORD_BYTES).unwrap();
        let mode = std::fs::metadata(&spool.path).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0);
    }

    #[test]
    fn raw_vfr_pts_form_the_canonical_shot_list() {
        let root = tempfile::tempdir().unwrap();
        let mut spool = PtsSpool::create(root.path(), 1_024).unwrap();
        for pts in [100, 140, 215, 260] {
            spool.push(pts).unwrap();
        }
        spool.finish_writes().unwrap();
        let detected = ShotDetectionOutput {
            frame_count: 4,
            window_count: 1,
            max_buffered_frames: 4,
            boundaries: vec![crate::models::adapters::omnishotcut::ModelBoundary {
                frame_index: 2,
                confidence: Some(0.75),
            }],
            transitions: vec![ModelTransition::TransNetV2 {
                start_frame: 1,
                end_frame_exclusive: 3,
                peak_frame: 2,
                peak_single_probability: 0.75,
                peak_all_frames_probability: 0.6,
            }],
            preprocess_seconds: 0.0,
            inference_seconds: 0.0,
        };
        let derived = derive_analysis(&detected, &mut spool, metadata(Some(220))).unwrap();
        assert_eq!(derived.shots.duration_seconds, 0.22);
        assert_eq!(derived.shots.boundaries[0].frame_index, 2);
        assert_eq!(derived.shots.boundaries[0].pts_seconds, 0.115);
        assert_eq!(derived.shots.shots.len(), 2);
        let ShotTransition::TransNetV2 {
            start_seconds,
            end_seconds,
            peak_seconds,
            ..
        } = &derived.transitions[0]
        else {
            panic!("wrong transition variant")
        };
        assert_eq!(
            (*start_seconds, *end_seconds, *peak_seconds),
            (0.04, 0.16, 0.115)
        );
    }

    #[test]
    fn missing_declared_duration_uses_the_final_pts_step() {
        let root = tempfile::tempdir().unwrap();
        let mut spool = PtsSpool::create(root.path(), 1_024).unwrap();
        for pts in [100, 140, 180] {
            spool.push(pts).unwrap();
        }
        assert_eq!(
            source_duration_seconds(&spool, metadata(None)).unwrap(),
            0.12
        );
    }

    #[test]
    fn staged_artifact_is_a_direct_shot_list_compatible_with_consumers() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("clip.shots.json");
        let transaction = FileOutputTransaction::new(&target, false).unwrap();
        let shots = ShotList::from_boundaries(
            2.0,
            vec![ShotBoundary {
                frame_index: 25,
                pts_seconds: 1.0,
                confidence: Some(0.8),
            }],
        )
        .unwrap();
        write_staged_shots(transaction.staging_path(), &shots).unwrap();
        validate_staged_shots(transaction.staging_path(), &shots).unwrap();
        transaction.commit().unwrap();

        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&target).unwrap()).unwrap();
        assert_eq!(value["durationSeconds"], 2.0);
        assert!(value.get("format").is_none());
        assert!(value.get("transitions").is_none());
        let decoded: ShotList = serde_json::from_value(value).unwrap();
        assert_eq!(decoded, shots);
    }

    #[test]
    fn explicit_detector_selection_never_falls_back_to_another_model() {
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("input.mp4");
        std::fs::write(&input, b"fixture").unwrap();
        let make = |id: &str| ShotsRequest {
            input: input.clone(),
            output: root.path().join(format!("{id}.json")),
            model: ModelSelection {
                id: id.to_owned(),
                version: None,
                backend: RunBackendPreference::Auto,
            },
            overwrite: false,
        };
        assert!(validate_request(&make(MODEL_ID)).is_ok());
        assert!(validate_request(&make(TRANSNETV2_MODEL_ID)).is_ok());
        assert_eq!(
            validate_request(&make("unsupported-histogram"))
                .unwrap_err()
                .code,
            ToolErrorCode::UnsupportedAdapter
        );
    }
}
