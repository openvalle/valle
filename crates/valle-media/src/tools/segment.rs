//! Complete-file prompted object segmentation.
//!
//! This tool owns prompt JSON, shared sequential libav decoding with source PTS, lossless
//! binary-mask encoding, optional transparent-foreground encoding, staged publication, progress,
//! cancellation and run provenance. EdgeTAM tensor, soft-alpha and state semantics stay behind
//! `models::adapters::edgetam`. It never shells out and never touches Project or Timeline state.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Instant,
};

use anyhow::{Context as _, Result as AnyResult, anyhow, ensure};
use ff::{Rational, codec, encoder, format, frame};
use ffmpeg_next as ff;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    codec::{
        TransparentVideoMuxer,
        ffi::{drain_video_encoder, ffmpeg_init, fill_plane0},
        probe_av, read_gray8_png, read_rgba_png, write_gray8_png, write_rgba_png,
    },
    frame::{Gray8Frame, RgbaFrame},
    models::{
        ModelSelection,
        adapters::edgetam::{
            EdgeTamModel, SegmentFrameOutput, SegmentPoint, SegmentPointLabel, SegmentPrompt,
            SegmentSession,
        },
    },
};

use super::{
    AlphaMode, MediaSummary, ModelProvenance, OutputArtifact, ProcessedMedia, ProcessingReport,
    RunContext, StageTiming, TimeRange, ToolError, ToolErrorCode, ToolEvent, ToolPhase, ToolRun,
    model_session::ModelSessionCandidates,
    output::{FileOutputTransaction, paths_refer_to_same_file},
    video_sequence::{
        SequentialGray8Decoder, SequentialRgbaDecoder, VideoMetadata, rescale_timestamp_nearest,
    },
};

const MODEL_ID: &str = "edgetam";
const DEFAULT_THRESHOLD: f32 = 0.5;
const MAX_PROMPT_BYTES: u64 = 64 * 1024;
const MAX_SOURCE_PIXELS: usize = 33_554_432;
const MASK_VIDEO_ENCODER: &str = "ffv1";
const MASK_VIDEO_MUXER: &str = "matroska";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SegmentRequest {
    pub input: PathBuf,
    pub output: PathBuf,
    /// Optional transparent foreground. Its alpha is the shared adapter's soft Alpha8 output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foreground_output: Option<PathBuf>,
    pub prompt: PathBuf,
    pub model: ModelSelection,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<TimeRange>,
    /// Foreground probability threshold. `None` uses 0.5.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold: Option<f32>,
    pub overwrite: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SegmentTimeBase {
    pub numerator: i32,
    pub denominator: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SegmentResult {
    pub outputs: Vec<OutputArtifact>,
    pub frames: u64,
    pub width: u32,
    pub height: u32,
    pub duration_seconds: Option<f64>,
    pub source_time_base: Option<SegmentTimeBase>,
    pub probability_threshold: f32,
    pub milliseconds_per_frame: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PromptDocument {
    format: String,
    format_version: u32,
    coordinate_space: String,
    points: [PromptDocumentPoint; 3],
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PromptDocumentPoint {
    x: f32,
    y: f32,
    label: PromptDocumentLabel,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum PromptDocumentLabel {
    Negative,
    Positive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InputKind {
    Png,
    Video,
}

pub fn run(
    request: SegmentRequest,
    context: &mut RunContext<'_>,
) -> Result<ToolRun<SegmentResult>, ToolError> {
    context.validate()?;
    let threshold = validate_request(&request)?;
    let prompt_document = read_prompt_document(&request.prompt)?;
    let prompt = prompt_from_document(&prompt_document)?;
    FileOutputTransaction::validate_target(&request.output, request.overwrite)?;
    if let Some(output) = request.foreground_output.as_deref() {
        FileOutputTransaction::validate_target(output, false)?;
    }
    let started = Instant::now();

    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Resolving));
    check_cancelled(context)?;
    let candidates = ModelSessionCandidates::resolve(context.models, request.model.clone())?;
    let transaction = FileOutputTransaction::new(&request.output, request.overwrite)?;
    let foreground_transaction = request
        .foreground_output
        .as_deref()
        .map(|path| FileOutputTransaction::new(path, false))
        .transpose()?;

    match input_kind(&request.input) {
        InputKind::Png => run_png(
            request,
            threshold,
            prompt_document,
            prompt,
            transaction,
            foreground_transaction,
            candidates,
            started,
            context,
        ),
        InputKind::Video => run_video(
            request,
            threshold,
            prompt_document,
            prompt,
            transaction,
            foreground_transaction,
            candidates,
            started,
            context,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn run_png(
    request: SegmentRequest,
    threshold: f32,
    prompt_document: PromptDocument,
    prompt: SegmentPrompt,
    transaction: FileOutputTransaction,
    foreground_transaction: Option<FileOutputTransaction>,
    candidates: ModelSessionCandidates,
    started: Instant,
    context: &mut RunContext<'_>,
) -> Result<ToolRun<SegmentResult>, ToolError> {
    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Decoding));
    let stage = Instant::now();
    let source = read_rgba_png(&request.input).map_err(|error| {
        ToolError::invalid_input(format!(
            "decode segment input {}: {error:#}",
            request.input.display()
        ))
    })?;
    let decode_seconds = stage.elapsed().as_secs_f64();
    validate_prompt_dimensions(&prompt, source.width, source.height)?;
    check_cancelled(context)?;

    context.progress.event(ToolEvent::phase(ToolPhase::Loading));
    let load_started = Instant::now();
    let opened = candidates.open("load EdgeTAM", |model| {
        EdgeTamModel::open(
            model,
            context.resources.cpu_threads,
            threshold,
            MAX_SOURCE_PIXELS,
        )
    })?;
    let mut model = opened.session;
    let load_seconds = load_started
        .elapsed()
        .as_secs_f64()
        .max(model.load_seconds());
    let provenance = ModelProvenance::from(&opened.model.resolved);
    let warnings = opened.warnings;
    check_cancelled(context)?;

    context
        .progress
        .event(ToolEvent::count(ToolPhase::Inferencing, 0, 1));
    let mut session = model.start_session();
    let segmented = session.initialize(0, &source, &prompt).map_err(|error| {
        ToolError::new(
            ToolErrorCode::InferenceFailed,
            format!("segment image: {error:#}"),
        )
    })?;
    validate_segmented_frame(&segmented, 0, 0, source.width, source.height)?;
    check_cancelled(context)?;

    context
        .progress
        .event(ToolEvent::count(ToolPhase::Encoding, 0, 1));
    let encode_started = Instant::now();
    write_gray8_png(transaction.staging_path(), &segmented.mask).map_err(|error| {
        ToolError::new(
            ToolErrorCode::OutputValidationFailed,
            format!(
                "write binary mask PNG {}: {error:#}",
                transaction.staging_path().display()
            ),
        )
    })?;
    let staged = read_gray8_png(transaction.staging_path()).map_err(|error| {
        ToolError::new(
            ToolErrorCode::OutputValidationFailed,
            format!("validate staged binary mask PNG: {error:#}"),
        )
    })?;
    if staged.width != source.width
        || staged.height != source.height
        || !staged.is_binary_mask()
        || staged.data != segmented.mask.data
    {
        return Err(ToolError::new(
            ToolErrorCode::OutputValidationFailed,
            "staged PNG did not round-trip as the exact binary Gray8 mask",
        ));
    }
    let mut encode_seconds = encode_started.elapsed().as_secs_f64();
    let mut foreground_postprocess_seconds = 0.0;
    let mut foreground_written = false;
    if let Some(foreground_transaction) = foreground_transaction.as_ref() {
        let alpha_started = Instant::now();
        let rendered = apply_soft_alpha(&source, &segmented.alpha)?;
        foreground_postprocess_seconds += alpha_started.elapsed().as_secs_f64();
        let encode_started = Instant::now();
        write_rgba_png(foreground_transaction.staging_path(), &rendered).map_err(|error| {
            ToolError::new(
                ToolErrorCode::OutputValidationFailed,
                format!(
                    "write transparent foreground PNG {}: {error:#}",
                    foreground_transaction.staging_path().display()
                ),
            )
        })?;
        let round_trip = read_rgba_png(foreground_transaction.staging_path()).map_err(|error| {
            ToolError::new(
                ToolErrorCode::OutputValidationFailed,
                format!("validate staged transparent foreground PNG: {error:#}"),
            )
        })?;
        if round_trip != rendered {
            return Err(ToolError::new(
                ToolErrorCode::OutputValidationFailed,
                "staged transparent foreground PNG did not round-trip exactly",
            ));
        }
        encode_seconds += encode_started.elapsed().as_secs_f64();
        foreground_written = true;
    }
    check_cancelled(context)?;
    let (output, foreground_output) = commit_segment_outputs(transaction, foreground_transaction)?;

    let output_artifact = OutputArtifact {
        role: "binary_mask".to_owned(),
        path: output,
        media_type: "image/png".to_owned(),
        summary: Some(
            MediaSummary {
                width: Some(source.width),
                height: Some(source.height),
                frame_count: Some(1),
                ..MediaSummary::default()
            }
            .with_image_encoding("png", "png", "gray", AlphaMode::NotApplicable),
        ),
    };
    let foreground_artifact = foreground_output.map(|path| OutputArtifact {
        role: "transparent_foreground".to_owned(),
        path,
        media_type: "image/png".to_owned(),
        summary: Some(
            MediaSummary {
                width: Some(source.width),
                height: Some(source.height),
                frame_count: Some(1),
                ..MediaSummary::default()
            }
            .with_image_encoding("png", "png", "rgba", AlphaMode::Straight),
        ),
    });
    debug_assert_eq!(foreground_written, foreground_artifact.is_some());
    let mut outputs = vec![output_artifact.clone()];
    if let Some(artifact) = foreground_artifact {
        outputs.push(artifact);
    }
    let total_seconds = started.elapsed().as_secs_f64();
    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Completed));
    Ok(ToolRun {
        result: SegmentResult {
            outputs: outputs.clone(),
            frames: 1,
            width: source.width,
            height: source.height,
            duration_seconds: None,
            source_time_base: None,
            probability_threshold: threshold,
            milliseconds_per_frame: total_seconds * 1_000.0,
        },
        report: ProcessingReport {
            operation: "segment".to_owned(),
            parameters: normalized_parameters(&request, threshold, &prompt_document),
            input: request.input,
            input_media_type: "image/png".to_owned(),
            input_summary: MediaSummary {
                width: Some(source.width),
                height: Some(source.height),
                frame_count: Some(1),
                pixel_format: Some("rgba8-straight".to_owned()),
                ..MediaSummary::default()
            },
            outputs,
            models: vec![provenance],
            processed: ProcessedMedia {
                frames: 1,
                audio_seconds: 0.0,
            },
            timing: StageTiming {
                load_seconds,
                decode_seconds,
                preprocess_seconds: segmented.timing.preprocess_seconds,
                inference_seconds: segmented.timing.inference_seconds,
                postprocess_seconds: segmented.timing.postprocess_seconds
                    + foreground_postprocess_seconds,
                encode_seconds,
                total_seconds,
            },
        },
        warnings,
    })
}

#[allow(clippy::too_many_arguments)]
fn run_video(
    request: SegmentRequest,
    threshold: f32,
    prompt_document: PromptDocument,
    prompt: SegmentPrompt,
    transaction: FileOutputTransaction,
    foreground_transaction: Option<FileOutputTransaction>,
    candidates: ModelSessionCandidates,
    started: Instant,
    context: &mut RunContext<'_>,
) -> Result<ToolRun<SegmentResult>, ToolError> {
    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Decoding));
    let decoder_started = Instant::now();
    let mut decoder = SequentialRgbaDecoder::open(&request.input, context.resources.cpu_threads)
        .map_err(|error| {
            ToolError::invalid_input(format!(
                "open segment input {}: {error:#}",
                request.input.display()
            ))
        })?;
    let mut decode_seconds = decoder_started.elapsed().as_secs_f64();
    let metadata = decoder.metadata();
    let source_duration = metadata.duration_seconds();
    validate_prompt_dimensions(&prompt, metadata.width, metadata.height)?;
    check_cancelled(context)?;

    context.progress.event(ToolEvent::phase(ToolPhase::Loading));
    let load_started = Instant::now();
    let opened = candidates.open("load EdgeTAM", |model| {
        EdgeTamModel::open(
            model,
            context.resources.cpu_threads,
            threshold,
            MAX_SOURCE_PIXELS,
        )
    })?;
    let mut model = opened.session;
    let load_seconds = load_started
        .elapsed()
        .as_secs_f64()
        .max(model.load_seconds());
    let provenance = ModelProvenance::from(&opened.model.resolved);
    let warnings = opened.warnings;
    check_cancelled(context)?;

    let mut muxer = BinaryMaskVideoMuxer::open(
        transaction.staging_path(),
        metadata.width,
        metadata.height,
        metadata.time_base,
        metadata.nominal_fps,
    )
    .map_err(|error| {
        ToolError::new(
            ToolErrorCode::OutputValidationFailed,
            format!("create staged binary-mask video: {error:#}"),
        )
    })?;
    let mut foreground_muxer = foreground_transaction
        .as_ref()
        .map(|transaction| {
            TransparentVideoMuxer::open_timed(
                transaction.staging_path(),
                metadata.width,
                metadata.height,
                metadata.time_base,
                fps_rational(metadata.nominal_fps),
            )
            .map_err(|error| {
                ToolError::new(
                    ToolErrorCode::OutputValidationFailed,
                    format!("create staged transparent foreground MOV: {error:#}"),
                )
            })
        })
        .transpose()?;
    let range_start = request.range.map(|range| range.start).unwrap_or(0.0);
    let range_end = request.range.and_then(|range| range.end);
    let total_hint = estimated_frames(metadata, range_start, range_end);
    let mut session = model.start_session();
    // Full-file masks preserve the source stream's absolute PTS so a later consumer can prove
    // that source and mask really share an origin. A range artifact is a new, trimmed timeline
    // and therefore starts at zero.
    let mut pts_mapper = OutputPtsMapper::new(request.range.is_some());
    let mut frames = 0_u64;
    let mut preprocess_seconds = 0.0;
    let mut inference_seconds = 0.0;
    let mut postprocess_seconds = 0.0;
    let mut encode_seconds = 0.0;
    let mut last_output_pts = None;
    let mut mask_digest = Sha256::new();
    let mut foreground_digest = foreground_transaction.as_ref().map(|_| Sha256::new());

    loop {
        check_cancelled(context)?;
        progress_frame(context, ToolPhase::Decoding, frames, total_hint);
        let stage = Instant::now();
        let decoded = decoder.next_frame().map_err(|error| {
            ToolError::invalid_input(format!("decode segment video frame: {error:#}"))
        })?;
        decode_seconds += stage.elapsed().as_secs_f64();
        let Some(decoded) = decoded else {
            break;
        };
        let local_pts = decoded.pts.checked_sub(metadata.start_pts).ok_or_else(|| {
            ToolError::invalid_input("segment source-local PTS conversion overflowed")
        })?;
        let time_seconds = local_pts as f64 * rational_seconds(metadata.time_base);
        if time_seconds < range_start {
            continue;
        }
        if range_end.is_some_and(|end| time_seconds >= end) {
            break;
        }

        progress_frame(context, ToolPhase::Inferencing, frames, total_hint);
        let output = if frames == 0 {
            session.initialize(decoded.pts, &decoded.frame, &prompt)
        } else {
            session.track(decoded.pts, &decoded.frame)
        }
        .map_err(|error| {
            ToolError::new(
                ToolErrorCode::InferenceFailed,
                format!("segment video frame {frames}: {error:#}"),
            )
        })?;
        validate_segmented_frame(
            &output,
            frames,
            decoded.pts,
            metadata.width,
            metadata.height,
        )?;
        mask_digest.update(&output.mask.data);
        preprocess_seconds += output.timing.preprocess_seconds;
        inference_seconds += output.timing.inference_seconds;
        postprocess_seconds += output.timing.postprocess_seconds;
        let output_pts = pts_mapper.push(decoded.pts)?;

        check_cancelled(context)?;
        progress_frame(context, ToolPhase::Encoding, frames, total_hint);
        let stage = Instant::now();
        muxer.encode(&output.mask, output_pts).map_err(|error| {
            ToolError::new(
                ToolErrorCode::OutputValidationFailed,
                format!("encode binary mask frame {frames}: {error:#}"),
            )
        })?;
        encode_seconds += stage.elapsed().as_secs_f64();
        if let Some(foreground_muxer) = foreground_muxer.as_mut() {
            let stage = Instant::now();
            let foreground = apply_soft_alpha(&decoded.frame, &output.alpha)?;
            postprocess_seconds += stage.elapsed().as_secs_f64();
            foreground_digest
                .as_mut()
                .expect("foreground muxer requires a digest")
                .update(&foreground.data);
            let stage = Instant::now();
            foreground_muxer
                .encode_video_at(&foreground, output_pts)
                .map_err(|error| {
                    ToolError::new(
                        ToolErrorCode::OutputValidationFailed,
                        format!("encode transparent foreground frame {frames}: {error:#}"),
                    )
                })?;
            encode_seconds += stage.elapsed().as_secs_f64();
        }
        frames = frames.checked_add(1).ok_or_else(|| {
            ToolError::new(ToolErrorCode::Internal, "segment frame count overflowed")
        })?;
        last_output_pts = Some(output_pts);
    }

    if frames == 0 {
        return Err(ToolError::invalid_input(
            "segment range contains no decodable video frames",
        ));
    }
    check_cancelled(context)?;
    let finish_started = Instant::now();
    muxer.finish().map_err(|error| {
        ToolError::new(
            ToolErrorCode::OutputValidationFailed,
            format!("finish binary-mask video: {error:#}"),
        )
    })?;
    if let Some(foreground_muxer) = foreground_muxer.as_mut() {
        foreground_muxer.finish().map_err(|error| {
            ToolError::new(
                ToolErrorCode::OutputValidationFailed,
                format!("finish transparent foreground MOV: {error:#}"),
            )
        })?;
    }
    encode_seconds += finish_started.elapsed().as_secs_f64();
    drop(muxer);
    drop(foreground_muxer);

    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Validating));
    let first_output_pts = pts_mapper
        .first_source_pts()
        .map(|first| if request.range.is_some() { 0 } else { first });
    let expectations = VideoOutputExpectations {
        frames,
        first_pts: first_output_pts.expect("non-empty segment output has a first PTS"),
        last_pts: last_output_pts.expect("non-empty segment output has a last PTS"),
        mask_sha256: mask_digest.finalize().into(),
        foreground_sha256: foreground_digest.map(|digest| digest.finalize().into()),
    };
    validate_staged_video_outputs(
        transaction.staging_path(),
        foreground_transaction
            .as_ref()
            .map(FileOutputTransaction::staging_path),
        metadata,
        expectations,
        context.resources.cpu_threads,
    )?;
    let frame_duration = nominal_frame_seconds(metadata);
    let duration_seconds = match (last_output_pts, pts_mapper.first_source_pts()) {
        (Some(last), Some(first)) => {
            let span = last
                .checked_sub(if request.range.is_some() { 0 } else { first })
                .ok_or_else(|| {
                    ToolError::invalid_input("segment output PTS span overflowed the i64 time base")
                })?;
            Some(
                (span as f64 * rational_seconds(metadata.time_base)).max(0.0)
                    + frame_duration.unwrap_or(0.0),
            )
        }
        _ => None,
    };
    check_cancelled(context)?;
    let (output, foreground_output) = commit_segment_outputs(transaction, foreground_transaction)?;

    let output_artifact = OutputArtifact {
        role: "binary_mask".to_owned(),
        path: output,
        media_type: "video/x-matroska".to_owned(),
        summary: Some(
            MediaSummary {
                duration_seconds,
                width: Some(metadata.width),
                height: Some(metadata.height),
                frame_count: Some(frames),
                ..MediaSummary::default()
            }
            .with_video_encoding(
                "matroska",
                "ffv1",
                None,
                "gray",
                AlphaMode::NotApplicable,
            ),
        ),
    };
    let foreground_artifact = foreground_output.map(|path| OutputArtifact {
        role: "transparent_foreground".to_owned(),
        path,
        media_type: "video/quicktime".to_owned(),
        summary: Some(
            MediaSummary {
                duration_seconds,
                width: Some(metadata.width),
                height: Some(metadata.height),
                frame_count: Some(frames),
                ..MediaSummary::default()
            }
            .with_video_encoding("mov", "qtrle", None, "argb", AlphaMode::Straight),
        ),
    });
    let mut outputs = vec![output_artifact.clone()];
    if let Some(artifact) = foreground_artifact {
        outputs.push(artifact);
    }
    let total_seconds = started.elapsed().as_secs_f64();
    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Completed));
    Ok(ToolRun {
        result: SegmentResult {
            outputs: outputs.clone(),
            frames,
            width: metadata.width,
            height: metadata.height,
            duration_seconds,
            source_time_base: Some(SegmentTimeBase {
                numerator: metadata.time_base.numerator(),
                denominator: metadata.time_base.denominator(),
            }),
            probability_threshold: threshold,
            milliseconds_per_frame: total_seconds * 1_000.0 / frames as f64,
        },
        report: ProcessingReport {
            operation: "segment".to_owned(),
            parameters: normalized_parameters(&request, threshold, &prompt_document),
            input: request.input,
            input_media_type: "video/*".to_owned(),
            input_summary: MediaSummary {
                duration_seconds: source_duration,
                width: Some(metadata.width),
                height: Some(metadata.height),
                frame_count: None,
                pixel_format: Some("rgba8-straight".to_owned()),
                ..MediaSummary::default()
            },
            outputs,
            models: vec![provenance],
            processed: ProcessedMedia {
                frames,
                audio_seconds: 0.0,
            },
            timing: StageTiming {
                load_seconds,
                decode_seconds,
                preprocess_seconds,
                inference_seconds,
                postprocess_seconds,
                encode_seconds,
                total_seconds,
            },
        },
        warnings,
    })
}

fn apply_soft_alpha(source: &RgbaFrame, alpha: &Gray8Frame) -> Result<RgbaFrame, ToolError> {
    if alpha.width != source.width
        || alpha.height != source.height
        || alpha.data.len() != source.pixel_count()
    {
        return Err(ToolError::new(
            ToolErrorCode::InferenceFailed,
            format!(
                "EdgeTAM alpha geometry drifted: source={}x{}, alpha={}x{} ({} bytes)",
                source.width,
                source.height,
                alpha.width,
                alpha.height,
                alpha.data.len()
            ),
        ));
    }
    let mut foreground = source.clone();
    for (pixel, model_alpha) in foreground.data.chunks_exact_mut(4).zip(&alpha.data) {
        pixel[3] = ((u16::from(pixel[3]) * u16::from(*model_alpha) + 127) / 255) as u8;
    }
    Ok(foreground)
}

/// Publish the optional companion before the main mask so a successful return can never expose a
/// mask without its requested foreground. Two arbitrary destination paths cannot be committed as
/// one filesystem operation. If a concurrent writer wins the main path after the companion was
/// published, retain the fully validated companion and report its exact path: deleting by pathname
/// would risk removing a third party's replacement.
fn commit_segment_outputs(
    mask: FileOutputTransaction,
    foreground: Option<FileOutputTransaction>,
) -> Result<(PathBuf, Option<PathBuf>), ToolError> {
    let foreground = foreground.map(FileOutputTransaction::commit).transpose()?;
    match mask.commit() {
        Ok(mask) => Ok((mask, foreground)),
        Err(mut error) => {
            if let Some(path) = foreground {
                error.message = format!(
                    "{}; the fully validated transparent foreground was already published at {} and was retained because two destination paths cannot be committed atomically",
                    error.message,
                    path.display()
                );
                error.context.insert(
                    "publishedForegroundOutput".to_owned(),
                    path.display().to_string(),
                );
            }
            Err(error)
        }
    }
}

fn validate_staged_video_outputs(
    mask_path: &Path,
    foreground_path: Option<&Path>,
    source: VideoMetadata,
    expected: VideoOutputExpectations,
    threads: usize,
) -> Result<(), ToolError> {
    validate_staged_video_outputs_inner(mask_path, foreground_path, source, expected, threads)
        .map_err(|error| {
            ToolError::new(
                ToolErrorCode::OutputValidationFailed,
                format!("validate complete staged segment output: {error:#}"),
            )
        })
}

#[derive(Debug, Clone, Copy)]
struct VideoOutputExpectations {
    frames: u64,
    first_pts: i64,
    last_pts: i64,
    mask_sha256: [u8; 32],
    foreground_sha256: Option<[u8; 32]>,
}

fn validate_staged_video_outputs_inner(
    mask_path: &Path,
    foreground_path: Option<&Path>,
    source: VideoMetadata,
    expected: VideoOutputExpectations,
    threads: usize,
) -> AnyResult<()> {
    let mask_probe = probe_av(mask_path)?;
    ensure!(
        mask_probe.audio.is_none(),
        "binary-mask video contains audio"
    );
    ensure!(
        mask_probe
            .video
            .is_some_and(|(width, height, _)| (width, height) == (source.width, source.height)),
        "binary-mask video dimensions differ from {}x{}",
        source.width,
        source.height
    );

    let mut mask = SequentialGray8Decoder::open(mask_path, threads)?;
    let mask_metadata = mask.metadata();
    let mut foreground = if let Some(path) = foreground_path {
        let probe = probe_av(path)?;
        ensure!(
            probe.audio.is_none(),
            "transparent foreground contains audio"
        );
        ensure!(
            probe
                .video
                .is_some_and(|(width, height, _)| (width, height) == (source.width, source.height)),
            "transparent foreground dimensions differ from {}x{}",
            source.width,
            source.height
        );
        let input = format::input(path)?;
        let video = input
            .streams()
            .best(ff::media::Type::Video)
            .context("transparent foreground has no video stream")?;
        ensure!(
            video.parameters().id() == codec::Id::QTRLE,
            "transparent foreground must use lossless qtrle, got {:?}",
            video.parameters().id()
        );
        Some(SequentialRgbaDecoder::open(path, threads)?)
    } else {
        None
    };

    ensure!(
        foreground_path.is_some() == expected.foreground_sha256.is_some(),
        "foreground validation expectation does not match the staged outputs"
    );
    let expected_first_mask = rescale_timestamp_nearest(
        expected.first_pts,
        source.time_base,
        mask_metadata.time_base,
    )?;
    let expected_last_mask =
        rescale_timestamp_nearest(expected.last_pts, source.time_base, mask_metadata.time_base)?;
    let mut count = 0_u64;
    let mut actual_first_mask = None;
    let mut actual_last_mask = None;
    let mut actual_mask_digest = Sha256::new();
    let mut actual_foreground_digest = foreground.as_ref().map(|_| Sha256::new());
    loop {
        let mask_frame = mask.next_frame()?;
        let foreground_frame = if let Some(decoder) = foreground.as_mut() {
            decoder.next_frame()?
        } else {
            None
        };
        match mask_frame {
            None => {
                ensure!(
                    foreground_frame.is_none(),
                    "transparent foreground has extra frame {count}"
                );
                break;
            }
            Some(mask_frame) => {
                actual_mask_digest.update(&mask_frame.frame.data);
                actual_first_mask.get_or_insert(mask_frame.pts);
                actual_last_mask = Some(mask_frame.pts);
                if let Some(foreground_frame) = foreground_frame {
                    let foreground_metadata = foreground
                        .as_ref()
                        .expect("foreground frame requires its decoder")
                        .metadata();
                    let expected_foreground = rescale_timestamp_nearest(
                        mask_frame.pts,
                        mask_metadata.time_base,
                        foreground_metadata.time_base,
                    )?;
                    ensure!(
                        foreground_frame.pts == expected_foreground,
                        "mask/foreground PTS mismatch at frame {count}: expected {expected_foreground}, got {}",
                        foreground_frame.pts
                    );
                    ensure!(
                        (foreground_frame.frame.width, foreground_frame.frame.height)
                            == (source.width, source.height),
                        "transparent foreground geometry drifted at frame {count}"
                    );
                    actual_foreground_digest
                        .as_mut()
                        .expect("foreground frame requires its digest")
                        .update(&foreground_frame.frame.data);
                } else {
                    ensure!(
                        foreground.is_none(),
                        "transparent foreground ended before mask frame {count}"
                    );
                }
                count = count
                    .checked_add(1)
                    .context("validated segment frame count overflowed")?;
            }
        }
    }
    ensure!(
        count == expected.frames,
        "staged segment output has {count} frames, expected {}",
        expected.frames
    );
    ensure!(
        actual_first_mask == Some(expected_first_mask),
        "staged mask first PTS {:?}, expected {expected_first_mask}",
        actual_first_mask
    );
    ensure!(
        actual_last_mask == Some(expected_last_mask),
        "staged mask last PTS {:?}, expected {expected_last_mask}",
        actual_last_mask
    );
    let actual_mask_sha256: [u8; 32] = actual_mask_digest.finalize().into();
    ensure!(
        actual_mask_sha256 == expected.mask_sha256,
        "staged mask pixels differ from the frames submitted to the lossless encoder"
    );
    let actual_foreground_sha256 =
        actual_foreground_digest.map(|digest| <[u8; 32]>::from(digest.finalize()));
    ensure!(
        actual_foreground_sha256 == expected.foreground_sha256,
        "staged foreground RGBA pixels differ from the frames submitted to qtrle"
    );
    Ok(())
}

fn estimated_frames(metadata: VideoMetadata, start: f64, end: Option<f64>) -> Option<u64> {
    let fps = metadata.nominal_fps;
    let end = end.or(metadata.duration_seconds())?;
    (fps > 0.0 && end > start).then(|| ((end - start) * fps).ceil() as u64)
}

fn nominal_frame_seconds(metadata: VideoMetadata) -> Option<f64> {
    (metadata.nominal_fps > 0.0).then_some(1.0 / metadata.nominal_fps)
}

fn validate_request(request: &SegmentRequest) -> Result<f32, ToolError> {
    if !request.input.is_file() {
        return Err(ToolError::invalid_input(format!(
            "segment input does not exist or is not a file: {}",
            request.input.display()
        )));
    }
    if !request.prompt.is_file() {
        return Err(ToolError::invalid_input(format!(
            "segment prompt does not exist or is not a file: {}",
            request.prompt.display()
        )));
    }
    if request.model.id != MODEL_ID {
        return Err(ToolError::new(
            ToolErrorCode::UnsupportedAdapter,
            format!(
                "segment supports model {MODEL_ID:?}, got {:?}",
                request.model.id
            ),
        ));
    }
    if request.foreground_output.is_some() && request.overwrite {
        return Err(ToolError::invalid_input(
            "segment --foreground-output cannot be combined with --overwrite: the first multi-output release only publishes to two new destinations",
        ));
    }
    if paths_refer_to_same_file(&request.input, &request.output)
        || paths_refer_to_same_file(&request.prompt, &request.output)
    {
        return Err(ToolError::invalid_input(
            "segment output must differ from both media input and prompt JSON",
        ));
    }
    if paths_refer_to_same_file(&request.input, &request.prompt) {
        return Err(ToolError::invalid_input(
            "segment media input and prompt JSON must be different files",
        ));
    }
    if let Some(foreground) = request.foreground_output.as_deref()
        && (paths_refer_to_same_file(&request.input, foreground)
            || paths_refer_to_same_file(&request.prompt, foreground)
            || paths_refer_to_same_file(&request.output, foreground))
    {
        return Err(ToolError::invalid_input(
            "segment foreground output must differ from the media input, prompt JSON and binary-mask output",
        ));
    }
    match input_kind(&request.input) {
        InputKind::Png if !has_extension(&request.output, "png") => {
            return Err(ToolError::invalid_input(
                "PNG segment input requires a .png Gray8 mask output",
            ));
        }
        InputKind::Png if request.range.is_some() => {
            return Err(ToolError::invalid_input(
                "segment range is valid only for video input",
            ));
        }
        InputKind::Png
            if request
                .foreground_output
                .as_deref()
                .is_some_and(|path| !has_extension(path, "png")) =>
        {
            return Err(ToolError::invalid_input(
                "PNG segment input requires a .png transparent foreground output",
            ));
        }
        InputKind::Video if !has_extension(&request.output, "mkv") => {
            return Err(ToolError::invalid_input(
                "video segment output must end in .mkv (lossless FFV1 Gray8, no audio)",
            ));
        }
        InputKind::Video
            if request
                .foreground_output
                .as_deref()
                .is_some_and(|path| !has_extension(path, "mov")) =>
        {
            return Err(ToolError::invalid_input(
                "video segment foreground output must end in .mov (lossless qtrle RGBA, no audio)",
            ));
        }
        _ => {}
    }
    if let Some(range) = request.range {
        if !range.start.is_finite() || range.start < 0.0 {
            return Err(ToolError::invalid_input(
                "segment range start must be finite and non-negative",
            ));
        }
        if range
            .end
            .is_some_and(|end| !end.is_finite() || end <= range.start)
        {
            return Err(ToolError::invalid_input(
                "segment range end must be finite and greater than start",
            ));
        }
    }
    let threshold = request.threshold.unwrap_or(DEFAULT_THRESHOLD);
    if !threshold.is_finite() || threshold <= 0.0 || threshold >= 1.0 {
        return Err(ToolError::invalid_input(
            "segment threshold must be finite and strictly between 0 and 1",
        ));
    }
    Ok(threshold)
}

fn input_kind(path: &Path) -> InputKind {
    if has_extension(path, "png") {
        InputKind::Png
    } else {
        InputKind::Video
    }
}

fn has_extension(path: &Path, wanted: &str) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case(wanted))
}

fn read_prompt_document(path: &Path) -> Result<PromptDocument, ToolError> {
    let metadata = std::fs::metadata(path).map_err(|error| {
        ToolError::invalid_input(format!("inspect prompt JSON {}: {error}", path.display()))
    })?;
    if metadata.len() > MAX_PROMPT_BYTES {
        return Err(ToolError::invalid_input(format!(
            "prompt JSON is {} bytes; maximum is {MAX_PROMPT_BYTES}",
            metadata.len()
        )));
    }
    let bytes = std::fs::read(path).map_err(|error| {
        ToolError::invalid_input(format!("read prompt JSON {}: {error}", path.display()))
    })?;
    let document: PromptDocument = serde_json::from_slice(&bytes).map_err(|error| {
        ToolError::invalid_input(format!("parse prompt JSON {}: {error}", path.display()))
    })?;
    if document.format != "valle.segment-prompt"
        || document.format_version != 1
        || document.coordinate_space != "source_pixels_xy"
    {
        return Err(ToolError::invalid_input(
            "prompt JSON must be valle.segment-prompt@1 in source_pixels_xy coordinates",
        ));
    }
    Ok(document)
}

fn prompt_from_document(document: &PromptDocument) -> Result<SegmentPrompt, ToolError> {
    let points = document.points.map(|point| SegmentPoint {
        x: point.x,
        y: point.y,
        label: match point.label {
            PromptDocumentLabel::Negative => SegmentPointLabel::Negative,
            PromptDocumentLabel::Positive => SegmentPointLabel::Positive,
        },
    });
    SegmentPrompt::new(points).map_err(|error| ToolError::invalid_input(error.to_string()))
}

fn validate_prompt_dimensions(
    prompt: &SegmentPrompt,
    width: u32,
    height: u32,
) -> Result<(), ToolError> {
    if width == 0 || height == 0 {
        return Err(ToolError::invalid_input(
            "segment source dimensions must be positive",
        ));
    }
    let pixels = usize::try_from(width)
        .ok()
        .and_then(|width| {
            usize::try_from(height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .ok_or_else(|| ToolError::invalid_input("segment source dimensions overflow"))?;
    if pixels > MAX_SOURCE_PIXELS {
        return Err(ToolError::invalid_input(format!(
            "segment source has {pixels} pixels; maximum is {MAX_SOURCE_PIXELS}"
        )));
    }
    for point in prompt.points() {
        if point.x < 0.0 || point.y < 0.0 || point.x >= width as f32 || point.y >= height as f32 {
            return Err(ToolError::invalid_input(format!(
                "prompt point ({}, {}) is outside display frame {width}x{height}",
                point.x, point.y
            )));
        }
    }
    Ok(())
}

fn validate_segmented_frame(
    output: &SegmentFrameOutput,
    expected_index: u64,
    expected_pts: i64,
    width: u32,
    height: u32,
) -> Result<(), ToolError> {
    if output.frame_index != expected_index
        || output.pts != expected_pts
        || output.mask.width != width
        || output.mask.height != height
        || !output.mask.is_binary_mask()
        || output.alpha.width != width
        || output.alpha.height != height
    {
        return Err(ToolError::new(
            ToolErrorCode::InferenceFailed,
            format!(
                "EdgeTAM output contract drifted at frame {expected_index}: index={}, pts={}, mask={}x{}, binary={}, alpha={}x{}",
                output.frame_index,
                output.pts,
                output.mask.width,
                output.mask.height,
                output.mask.is_binary_mask(),
                output.alpha.width,
                output.alpha.height
            ),
        ));
    }
    Ok(())
}

fn check_cancelled(context: &RunContext<'_>) -> Result<(), ToolError> {
    if context.cancellation.is_cancelled() {
        Err(ToolError::cancelled())
    } else {
        Ok(())
    }
}

fn progress_frame(
    context: &mut RunContext<'_>,
    phase: ToolPhase,
    completed: u64,
    total: Option<u64>,
) {
    context.progress.event(ToolEvent {
        phase,
        completed: Some(completed),
        total,
        message: None,
    });
}

fn normalized_parameters(
    request: &SegmentRequest,
    threshold: f32,
    prompt: &PromptDocument,
) -> BTreeMap<String, serde_json::Value> {
    let mut parameters = BTreeMap::new();
    parameters.insert("model".to_owned(), request.model.id.clone().into());
    parameters.insert(
        "modelVersion".to_owned(),
        request
            .model
            .version
            .as_ref()
            .map_or(serde_json::Value::Null, |version| version.clone().into()),
    );
    parameters.insert(
        "backend".to_owned(),
        serde_json::to_value(request.model.backend).expect("backend is serializable"),
    );
    parameters.insert("threshold".to_owned(), threshold.into());
    parameters.insert(
        "range".to_owned(),
        request.range.map_or(serde_json::Value::Null, |range| {
            serde_json::to_value(range).expect("time range is serializable")
        }),
    );
    parameters.insert(
        "promptPath".to_owned(),
        request.prompt.display().to_string().into(),
    );
    parameters.insert(
        "foregroundOutput".to_owned(),
        request
            .foreground_output
            .as_ref()
            .map_or(serde_json::Value::Null, |path| {
                path.display().to_string().into()
            }),
    );
    parameters.insert(
        "prompt".to_owned(),
        serde_json::to_value(prompt).expect("validated prompt is serializable"),
    );
    parameters.insert(
        "outputContract".to_owned(),
        match input_kind(&request.input) {
            InputKind::Png => "png/gray8/binary".into(),
            InputKind::Video => "matroska/ffv1/gray8/binary/no-audio".into(),
        },
    );
    if input_kind(&request.input) == InputKind::Video {
        parameters.insert(
            "ptsPolicy".to_owned(),
            if request.range.is_some() {
                "source-range/rebased-to-zero"
            } else {
                "source-absolute/preserved"
            }
            .into(),
        );
    }
    if request.foreground_output.is_some() {
        parameters.insert(
            "foregroundOutputContract".to_owned(),
            match input_kind(&request.input) {
                InputKind::Png => "png/rgba8/straight-alpha/no-audio".into(),
                InputKind::Video => "mov/qtrle/rgba8/straight-alpha/no-audio".into(),
            },
        );
        parameters.insert(
            "multiOutputCommitPolicy".to_owned(),
            "new-destinations-only/foreground-first/main-mask-last".into(),
        );
    }
    parameters
}

struct BinaryMaskVideoMuxer {
    output: format::context::Output,
    encoder: encoder::Video,
    stream_index: usize,
    encoder_time_base: Rational,
    output_time_base: Rational,
    width: u32,
    height: u32,
    last_pts: Option<i64>,
    finished: bool,
}

impl BinaryMaskVideoMuxer {
    fn open(
        path: &Path,
        width: u32,
        height: u32,
        time_base: Rational,
        nominal_fps: f64,
    ) -> AnyResult<Self> {
        ffmpeg_init();
        ensure!(
            width > 0 && height > 0,
            "mask video dimensions must be positive"
        );
        ensure!(
            time_base.numerator() > 0 && time_base.denominator() > 0,
            "mask video time base must be positive"
        );
        let mut output = format::output_as(path, MASK_VIDEO_MUXER)
            .with_context(|| format!("open mask video {}", path.display()))?;
        let global_header = output
            .format()
            .flags()
            .contains(format::Flags::GLOBAL_HEADER);
        let codec = encoder::find_by_name(MASK_VIDEO_ENCODER)
            .ok_or_else(|| anyhow!("lossless encoder {MASK_VIDEO_ENCODER:?} is unavailable"))?;
        let mut video = codec::context::Context::new_with_codec(codec)
            .encoder()
            .video()?;
        video.set_width(width);
        video.set_height(height);
        video.set_format(format::Pixel::GRAY8);
        video.set_time_base(time_base);
        video.set_aspect_ratio(Rational(1, 1));
        if let Some(rate) = fps_rational(nominal_fps) {
            video.set_frame_rate(Some(rate));
        }
        if global_header {
            video.set_flags(codec::Flags::GLOBAL_HEADER);
        }
        let encoder = video.open_as(codec)?;
        let stream_index = {
            let mut stream = output.add_stream(codec)?;
            let index = stream.index();
            stream.set_parameters(&encoder);
            stream.set_time_base(time_base);
            index
        };
        output.write_header()?;
        let output_time_base = output
            .stream(stream_index)
            .context("mask video stream missing after header")?
            .time_base();
        Ok(Self {
            output,
            encoder,
            stream_index,
            encoder_time_base: time_base,
            output_time_base,
            width,
            height,
            last_pts: None,
            finished: false,
        })
    }

    fn encode(&mut self, mask: &Gray8Frame, pts: i64) -> AnyResult<()> {
        ensure!(
            mask.width == self.width && mask.height == self.height,
            "mask frame {}x{} does not match muxer {}x{}",
            mask.width,
            mask.height,
            self.width,
            self.height
        );
        ensure!(
            mask.is_binary_mask(),
            "mask video input is not binary Gray8"
        );
        if let Some(last) = self.last_pts {
            ensure!(
                pts > last,
                "output PTS is not strictly increasing: {pts} <= {last}"
            );
        }
        let mut encoded = frame::Video::new(format::Pixel::GRAY8, self.width, self.height);
        fill_plane0(&mut encoded, &mask.data, self.width as usize, self.height);
        encoded.set_pts(Some(pts));
        self.encoder.send_frame(&encoded)?;
        drain_video_encoder(
            &mut self.encoder,
            &mut self.output,
            self.stream_index,
            self.encoder_time_base,
            self.output_time_base,
        )?;
        self.last_pts = Some(pts);
        Ok(())
    }

    fn finish(&mut self) -> AnyResult<()> {
        if self.finished {
            return Ok(());
        }
        self.encoder.send_eof()?;
        drain_video_encoder(
            &mut self.encoder,
            &mut self.output,
            self.stream_index,
            self.encoder_time_base,
            self.output_time_base,
        )?;
        self.output.write_trailer()?;
        self.finished = true;
        Ok(())
    }
}

struct OutputPtsMapper {
    rebase: bool,
    first: Option<i64>,
    last: Option<i64>,
}

impl OutputPtsMapper {
    const fn new(rebase: bool) -> Self {
        Self {
            rebase,
            first: None,
            last: None,
        }
    }

    fn push(&mut self, source_pts: i64) -> Result<i64, ToolError> {
        if let Some(last) = self.last
            && source_pts <= last
        {
            return Err(ToolError::invalid_input(format!(
                "segment source PTS must strictly increase: {source_pts} <= {last}"
            )));
        }
        let first = *self.first.get_or_insert(source_pts);
        let output = if self.rebase {
            source_pts.checked_sub(first).ok_or_else(|| {
                ToolError::invalid_input("segment PTS rebase overflowed the i64 time base")
            })?
        } else {
            source_pts
        };
        self.last = Some(source_pts);
        Ok(output)
    }

    const fn first_source_pts(&self) -> Option<i64> {
        self.first
    }
}

fn rational_seconds(value: Rational) -> f64 {
    value.numerator() as f64 / value.denominator() as f64
}

fn fps_rational(fps: f64) -> Option<Rational> {
    const SCALE: f64 = 1_000_000.0;
    if !fps.is_finite() || fps <= 0.0 {
        return None;
    }
    let numerator = (fps * SCALE).round();
    if numerator <= 0.0 || numerator > i32::MAX as f64 {
        return None;
    }
    let numerator = numerator as i32;
    let denominator = SCALE as i32;
    let divisor = gcd_i32(numerator, denominator);
    Some(Rational(numerator / divisor, denominator / divisor))
}

fn gcd_i32(mut left: i32, mut right: i32) -> i32 {
    while right != 0 {
        (left, right) = (right, left % right);
    }
    left.abs().max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prompt_document() -> PromptDocument {
        PromptDocument {
            format: "valle.segment-prompt".to_owned(),
            format_version: 1,
            coordinate_space: "source_pixels_xy".to_owned(),
            points: [
                PromptDocumentPoint {
                    x: 4.0,
                    y: 3.0,
                    label: PromptDocumentLabel::Positive,
                },
                PromptDocumentPoint {
                    x: 5.0,
                    y: 3.0,
                    label: PromptDocumentLabel::Positive,
                },
                PromptDocumentPoint {
                    x: 0.0,
                    y: 0.0,
                    label: PromptDocumentLabel::Negative,
                },
            ],
        }
    }

    #[test]
    fn prompt_json_is_strict_and_maps_to_neutral_points() {
        let json = serde_json::to_vec(&prompt_document()).unwrap();
        let decoded: PromptDocument = serde_json::from_slice(&json).unwrap();
        let prompt = prompt_from_document(&decoded).unwrap();
        assert_eq!(prompt.points()[0].label, SegmentPointLabel::Positive);

        let mut value = serde_json::to_value(decoded).unwrap();
        value["unexpected"] = true.into();
        assert!(serde_json::from_value::<PromptDocument>(value).is_err());
    }

    #[test]
    fn prompt_bounds_use_display_dimensions() {
        let prompt = prompt_from_document(&prompt_document()).unwrap();
        assert!(validate_prompt_dimensions(&prompt, 6, 4).is_ok());
        assert!(validate_prompt_dimensions(&prompt, 4, 4).is_err());
    }

    #[test]
    fn full_pts_are_absolute_but_range_pts_are_rebased_once() {
        let mut full = OutputPtsMapper::new(false);
        assert_eq!(full.push(900).unwrap(), 900);
        assert_eq!(full.push(903).unwrap(), 903);

        let mut range = OutputPtsMapper::new(true);
        assert_eq!(range.push(900).unwrap(), 0);
        assert_eq!(range.push(903).unwrap(), 3);
        assert!(range.push(903).is_err());
        assert!(range.push(899).is_err());
    }

    #[test]
    fn binary_mask_contract_rejects_soft_values() {
        let output = SegmentFrameOutput {
            frame_index: 0,
            pts: 7,
            mask: Gray8Frame::from_data(2, 1, vec![0, 254]).unwrap(),
            alpha: Gray8Frame::from_data(2, 1, vec![64, 192]).unwrap(),
            timing: Default::default(),
        };
        assert!(validate_segmented_frame(&output, 0, 7, 2, 1).is_err());
    }

    #[test]
    fn soft_alpha_multiplies_existing_straight_alpha_without_touching_rgb() {
        let source = RgbaFrame {
            width: 2,
            height: 1,
            data: vec![10, 20, 30, 255, 40, 50, 60, 128],
        };
        let alpha = Gray8Frame::from_data(2, 1, vec![128, 128]).unwrap();
        let foreground = apply_soft_alpha(&source, &alpha).unwrap();
        assert_eq!(foreground.data, vec![10, 20, 30, 128, 40, 50, 60, 64]);
    }

    #[test]
    fn foreground_race_prevents_main_publish_and_cleans_staging() {
        let root = tempfile::tempdir().unwrap();
        let mask_path = root.path().join("mask.mkv");
        let foreground_path = root.path().join("foreground.mov");
        let mask = FileOutputTransaction::new(&mask_path, false).unwrap();
        let foreground = FileOutputTransaction::new(&foreground_path, false).unwrap();
        let mask_stage = mask.staging_path().to_owned();
        let foreground_stage = foreground.staging_path().to_owned();
        std::fs::write(&mask_stage, b"mask").unwrap();
        std::fs::write(&foreground_stage, b"foreground").unwrap();
        std::fs::write(&foreground_path, b"competitor").unwrap();

        let error = commit_segment_outputs(mask, Some(foreground)).unwrap_err();
        assert_eq!(error.code, ToolErrorCode::OutputValidationFailed);
        assert!(!mask_path.exists());
        assert_eq!(std::fs::read(&foreground_path).unwrap(), b"competitor");
        assert!(!mask_stage.exists());
        assert!(!foreground_stage.exists());
    }

    #[test]
    fn main_race_retains_published_companion_and_reports_its_path() {
        let root = tempfile::tempdir().unwrap();
        let mask_path = root.path().join("mask.mkv");
        let foreground_path = root.path().join("foreground.mov");
        let mask = FileOutputTransaction::new(&mask_path, false).unwrap();
        let foreground = FileOutputTransaction::new(&foreground_path, false).unwrap();
        let mask_stage = mask.staging_path().to_owned();
        let foreground_stage = foreground.staging_path().to_owned();
        std::fs::write(&mask_stage, b"mask").unwrap();
        std::fs::write(&foreground_stage, b"foreground").unwrap();
        std::fs::write(&mask_path, b"competitor").unwrap();

        let error = commit_segment_outputs(mask, Some(foreground)).unwrap_err();
        assert_eq!(error.code, ToolErrorCode::OutputValidationFailed);
        assert_eq!(std::fs::read(&mask_path).unwrap(), b"competitor");
        assert_eq!(std::fs::read(&foreground_path).unwrap(), b"foreground");
        assert_eq!(
            error.context.get("publishedForegroundOutput"),
            Some(&foreground_path.display().to_string())
        );
        assert!(!mask_stage.exists());
        assert!(!foreground_stage.exists());
    }

    #[test]
    fn multi_output_first_release_rejects_overwrite() {
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("source.png");
        let prompt = root.path().join("prompt.json");
        std::fs::write(&input, b"input").unwrap();
        std::fs::write(&prompt, b"prompt").unwrap();
        let request = SegmentRequest {
            input,
            output: root.path().join("mask.png"),
            foreground_output: Some(root.path().join("foreground.png")),
            prompt,
            model: ModelSelection::pinned_default(MODEL_ID),
            range: None,
            threshold: None,
            overwrite: true,
        };
        let error = validate_request(&request).unwrap_err();
        assert_eq!(error.code, ToolErrorCode::InvalidInput);
        assert!(
            error
                .message
                .contains("cannot be combined with --overwrite")
        );
    }

    #[test]
    fn video_outputs_roundtrip_every_frame_with_matching_pts_and_no_audio() {
        let root = tempfile::tempdir().unwrap();
        let mask_path = root.path().join("mask.mkv");
        let foreground_path = root.path().join("foreground.mov");
        let time_base = Rational(1, 1_000);
        let metadata = VideoMetadata {
            width: 2,
            height: 2,
            duration_ticks: Some(66),
            nominal_fps: 30.0,
            time_base,
            start_pts: 900,
            rotation_degrees: 0,
            sample_aspect_ratio: Rational(1, 1),
        };
        let mut mask_muxer = BinaryMaskVideoMuxer::open(&mask_path, 2, 2, time_base, 30.0).unwrap();
        let mut foreground_muxer = TransparentVideoMuxer::open_timed(
            &foreground_path,
            2,
            2,
            time_base,
            Some(Rational(30, 1)),
        )
        .unwrap();
        let mask0 = Gray8Frame::from_data(2, 2, vec![0, 255, 255, 0]).unwrap();
        let mask1 = Gray8Frame::from_data(2, 2, vec![255, 0, 0, 255]).unwrap();
        let foreground0 = RgbaFrame {
            width: 2,
            height: 2,
            data: vec![
                10, 20, 30, 64, 40, 50, 60, 128, 10, 20, 30, 64, 40, 50, 60, 128,
            ],
        };
        let foreground1 = RgbaFrame {
            width: 2,
            height: 2,
            data: vec![
                70, 80, 90, 192, 100, 110, 120, 255, 70, 80, 90, 192, 100, 110, 120, 255,
            ],
        };
        for (pts, mask, foreground) in [(900, &mask0, &foreground0), (933, &mask1, &foreground1)] {
            mask_muxer.encode(mask, pts).unwrap();
            foreground_muxer.encode_video_at(foreground, pts).unwrap();
        }
        mask_muxer.finish().unwrap();
        foreground_muxer.finish().unwrap();
        drop(mask_muxer);
        drop(foreground_muxer);

        let mut mask_digest = Sha256::new();
        mask_digest.update(&mask0.data);
        mask_digest.update(&mask1.data);
        let mut foreground_digest = Sha256::new();
        foreground_digest.update(&foreground0.data);
        foreground_digest.update(&foreground1.data);
        validate_staged_video_outputs(
            &mask_path,
            Some(&foreground_path),
            metadata,
            VideoOutputExpectations {
                frames: 2,
                first_pts: 900,
                last_pts: 933,
                mask_sha256: mask_digest.finalize().into(),
                foreground_sha256: Some(foreground_digest.finalize().into()),
            },
            1,
        )
        .unwrap();

        let mut decoded = SequentialRgbaDecoder::open(&foreground_path, 1).unwrap();
        assert_eq!(decoded.next_frame().unwrap().unwrap().frame, foreground0);
        assert_eq!(decoded.next_frame().unwrap().unwrap().frame, foreground1);
        assert!(decoded.next_frame().unwrap().is_none());
    }

    #[test]
    fn single_frame_video_outputs_keep_nonzero_absolute_pts() {
        let root = tempfile::tempdir().unwrap();
        let mask_path = root.path().join("mask.mkv");
        let foreground_path = root.path().join("foreground.mov");
        let time_base = Rational(1, 1_000);
        let metadata = VideoMetadata {
            width: 2,
            height: 2,
            duration_ticks: Some(33),
            nominal_fps: 30.0,
            time_base,
            start_pts: 900,
            rotation_degrees: 0,
            sample_aspect_ratio: Rational(1, 1),
        };
        let mask = Gray8Frame::from_data(2, 2, vec![0, 255, 255, 0]).unwrap();
        let foreground = RgbaFrame {
            width: 2,
            height: 2,
            data: vec![
                10, 20, 30, 64, 40, 50, 60, 128, 10, 20, 30, 64, 40, 50, 60, 128,
            ],
        };
        let mut mask_muxer = BinaryMaskVideoMuxer::open(&mask_path, 2, 2, time_base, 30.0).unwrap();
        let mut foreground_muxer = TransparentVideoMuxer::open_timed(
            &foreground_path,
            2,
            2,
            time_base,
            Some(Rational(30, 1)),
        )
        .unwrap();
        mask_muxer.encode(&mask, 900).unwrap();
        foreground_muxer.encode_video_at(&foreground, 900).unwrap();
        mask_muxer.finish().unwrap();
        foreground_muxer.finish().unwrap();
        drop(mask_muxer);
        drop(foreground_muxer);

        let mut mask_digest = Sha256::new();
        mask_digest.update(&mask.data);
        let mut foreground_digest = Sha256::new();
        foreground_digest.update(&foreground.data);
        validate_staged_video_outputs(
            &mask_path,
            Some(&foreground_path),
            metadata,
            VideoOutputExpectations {
                frames: 1,
                first_pts: 900,
                last_pts: 900,
                mask_sha256: mask_digest.finalize().into(),
                foreground_sha256: Some(foreground_digest.finalize().into()),
            },
            1,
        )
        .unwrap();
    }
}
