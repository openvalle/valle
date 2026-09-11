//! Complete-file Real-ESRGAN super-resolution workflow.
//!
//! This tool owns media I/O, the ordered one-session-per-job loop, atomic publication,
//! cancellation, progress and provenance. Model-specific tiling and pixel pre/post-processing stay
//! behind `models::adapters::realesrgan` and the shared model runtime.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Instant,
};

use crate::codec::TimeBase as Rational;
use crate::models::inference::realesrgan::TileConfig;
use serde::{Deserialize, Serialize};

use crate::{
    codec::{Muxer, VideoFrameTransport, probe_av, read_rgba_png, write_rgba_png},
    frame::RgbaFrame,
    models::{
        ModelSelection,
        adapters::realesrgan::{MODEL_ID, UpscaleSession},
    },
};

use super::{
    AlphaMode, MediaSummary, ModelProvenance, OutputArtifact, PipelineUsage, ProcessedMedia,
    ProcessingReport, RunContext, StageTiming, TimeRange, ToolError, ToolErrorCode, ToolEvent,
    ToolPhase, ToolRun, ToolWarning,
    bounded_pipeline::BoundedPipeline,
    model_session::ModelSessionCandidates,
    output::{FileOutputTransaction, paths_refer_to_same_file},
    video_sequence::{
        SequentialRgbaDecoder, SourceTimeline, TimedVideoFrame, VideoAudioBridge,
        decode_video_timing,
    },
};

/// The published Real-ESRGAN release is the native x4v3 model. Lower scales must be implemented as
/// an explicit, reported post-resize rather than by silently changing the model contract.
pub const PUBLISHED_SCALE: u32 = crate::models::inference::realesrgan::SCALE;

/// Bound the unavoidable full output/compositor allocations. At x4 this admits a 1920x1080 input
/// (7680x4320 output) while rejecting dimensions that would otherwise allocate several GiB.
const MAX_OUTPUT_PIXELS: u64 = 33_554_432;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpscaleRequest {
    pub input: PathBuf,
    pub output: PathBuf,
    pub model: ModelSelection,
    /// Requested output scale. Only accepts the published model's native x4 contract.
    pub scale: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<TimeRange>,
    pub overwrite: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpscaleResult {
    pub outputs: Vec<OutputArtifact>,
    pub frames: u64,
    pub source_width: u32,
    pub source_height: u32,
    pub output_width: u32,
    pub output_height: u32,
    pub scale: u32,
    pub tiles: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<f64>,
    pub milliseconds_per_frame: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pipeline: Option<PipelineUsage>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InputKind {
    Png,
    Video,
}

pub fn run(
    request: UpscaleRequest,
    context: &mut RunContext<'_>,
) -> Result<ToolRun<UpscaleResult>, ToolError> {
    context.validate()?;
    let input_kind = validate_request(&request)?;
    FileOutputTransaction::validate_target(&request.output, request.overwrite)?;
    let started = Instant::now();
    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Resolving));
    let candidates = ModelSessionCandidates::resolve(context.models, request.model.clone())?;
    let transaction = FileOutputTransaction::new(&request.output, request.overwrite)?;

    match input_kind {
        InputKind::Png => run_png(request, transaction, candidates, started, context),
        InputKind::Video => run_video(request, transaction, candidates, started, context),
    }
}

fn run_png(
    request: UpscaleRequest,
    transaction: FileOutputTransaction,
    candidates: ModelSessionCandidates,
    started: Instant,
    context: &mut RunContext<'_>,
) -> Result<ToolRun<UpscaleResult>, ToolError> {
    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Decoding));
    let stage = Instant::now();
    let source = read_rgba_png(&request.input).map_err(|error| {
        ToolError::invalid_input(format!(
            "decode upscale input PNG {}: {error:#}",
            request.input.display()
        ))
    })?;
    let decode_seconds = stage.elapsed().as_secs_f64();
    let (output_width, output_height) = validate_geometry(source.width, source.height)?;
    check_cancelled(context)?;

    context.progress.event(ToolEvent::phase(ToolPhase::Loading));
    let load_started = Instant::now();
    let opened = candidates.open("load Real-ESRGAN", |model| {
        UpscaleSession::open(model, context.resources.cpu_threads, TileConfig::default())
    })?;
    let mut session = opened.session;
    let load_seconds = load_started
        .elapsed()
        .as_secs_f64()
        .max(session.load_seconds());
    let provenance = ModelProvenance::from(&opened.model.resolved);
    let warnings = opened.warnings;
    check_cancelled(context)?;

    context
        .progress
        .event(ToolEvent::count(ToolPhase::Inferencing, 0, 1));
    let upscaled = session.upscale(&source).map_err(|error| {
        ToolError::new(
            ToolErrorCode::InferenceFailed,
            format!("upscale PNG with Real-ESRGAN: {error:#}"),
        )
    })?;
    validate_output_geometry(
        &upscaled.frame,
        source.width,
        source.height,
        output_width,
        output_height,
    )?;
    check_cancelled(context)?;

    context
        .progress
        .event(ToolEvent::count(ToolPhase::Encoding, 0, 1));
    let stage = Instant::now();
    write_rgba_png(transaction.staging_path(), &upscaled.frame).map_err(|error| {
        ToolError::new(
            ToolErrorCode::OutputValidationFailed,
            format!(
                "write upscaled PNG {}: {error:#}",
                transaction.staging_path().display()
            ),
        )
    })?;
    let encode_seconds = stage.elapsed().as_secs_f64();

    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Validating));
    check_cancelled(context)?;
    let staged = read_rgba_png(transaction.staging_path()).map_err(|error| {
        ToolError::new(
            ToolErrorCode::OutputValidationFailed,
            format!("validate staged upscaled PNG: {error:#}"),
        )
    })?;
    if staged != upscaled.frame {
        return Err(ToolError::new(
            ToolErrorCode::OutputValidationFailed,
            "staged upscaled PNG did not round-trip exactly",
        ));
    }
    let output = transaction.commit()?;
    let output_artifact = OutputArtifact {
        role: "upscaled_media".to_owned(),
        path: output,
        media_type: "image/png".to_owned(),
        summary: Some(
            MediaSummary {
                width: Some(output_width),
                height: Some(output_height),
                frame_count: Some(1),
                ..MediaSummary::default()
            }
            .with_image_encoding("png", "png", "rgba", AlphaMode::Straight),
        ),
    };
    let total_seconds = started.elapsed().as_secs_f64();
    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Completed));

    Ok(ToolRun {
        result: UpscaleResult {
            outputs: vec![output_artifact.clone()],
            frames: 1,
            source_width: source.width,
            source_height: source.height,
            output_width,
            output_height,
            scale: PUBLISHED_SCALE,
            tiles: upscaled.tiles as u64,
            duration_seconds: None,
            milliseconds_per_frame: total_seconds * 1_000.0,
            pipeline: None,
        },
        report: ProcessingReport {
            operation: "upscale".to_owned(),
            parameters: normalized_parameters(&request, "not_applicable"),
            input: request.input,
            input_media_type: "image/png".to_owned(),
            input_summary: MediaSummary {
                width: Some(source.width),
                height: Some(source.height),
                frame_count: Some(1),
                pixel_format: Some("rgba8-straight".to_owned()),
                ..MediaSummary::default()
            },
            outputs: vec![output_artifact],
            models: vec![provenance],
            processed: ProcessedMedia {
                frames: 1,
                audio_seconds: 0.0,
            },
            timing: StageTiming {
                load_seconds,
                decode_seconds,
                preprocess_seconds: 0.0,
                inference_seconds: upscaled.inference_seconds,
                postprocess_seconds: (upscaled.total_seconds - upscaled.inference_seconds).max(0.0),
                encode_seconds,
                total_seconds,
            },
        },
        warnings,
    })
}

fn run_video(
    request: UpscaleRequest,
    transaction: FileOutputTransaction,
    candidates: ModelSessionCandidates,
    started: Instant,
    context: &mut RunContext<'_>,
) -> Result<ToolRun<UpscaleResult>, ToolError> {
    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Decoding));
    let stage = Instant::now();
    let mut decoder = SequentialRgbaDecoder::open(&request.input, context.resources.cpu_threads)
        .map_err(|error| {
            ToolError::invalid_input(format!(
                "open upscale input video {}: {error:#}",
                request.input.display()
            ))
        })?;
    let metadata = decoder.metadata();
    let mut decode_seconds = stage.elapsed().as_secs_f64();
    let (output_width, output_height) = validate_geometry(metadata.width, metadata.height)?;
    validate_video_range(request.range, metadata.duration_ticks, metadata.time_base)?;
    let estimated_frames = estimated_selected_frames(
        request.range,
        metadata.duration_seconds(),
        metadata.nominal_fps,
    );
    check_cancelled(context)?;

    // Two frames provide a nominal encoder-rate hint and a final-frame duration fallback. The
    // actual source PTS remain authoritative and may be VFR.
    let mut pending = BoundedPipeline::new(context.resources.pipeline_capacity)?;
    let mut source_ended = false;
    while pending.len() < pending.capacity().min(2) {
        match next_selected_frame(
            &mut decoder,
            request.range,
            metadata.start_pts,
            metadata.time_base,
            &mut decode_seconds,
            context,
        )? {
            Some(frame) => pending.push_back(frame)?,
            None => {
                source_ended = true;
                break;
            }
        }
    }
    let first = pending
        .front()
        .ok_or_else(|| ToolError::invalid_input("upscale range selected no video frames"))?;
    let first_source_pts = first.pts;
    let first_video_seconds = first.absolute_seconds(metadata.time_base);
    let second_rebased_pts = pending
        .get(1)
        .map(|frame| {
            frame.pts.checked_sub(first_source_pts).ok_or_else(|| {
                ToolError::invalid_input("upscale source PTS rebase overflowed the i64 time base")
            })
        })
        .transpose()?;
    let mut timeline = SourceTimeline::derive(
        0,
        second_rebased_pts,
        metadata.time_base,
        metadata.nominal_fps,
    )
    .map_err(|error| ToolError::invalid_input(format!("derive source video clock: {error:#}")))?;
    let (fps_numerator, fps_denominator) = timeline.fps();
    context.progress.event(ToolEvent::phase(ToolPhase::Loading));
    let load_started = Instant::now();
    let opened = candidates.open("load Real-ESRGAN", |model| {
        UpscaleSession::open(model, context.resources.cpu_threads, TileConfig::default())
    })?;
    let mut session = opened.session;
    let load_seconds = load_started
        .elapsed()
        .as_secs_f64()
        .max(session.load_seconds());
    let provenance = ModelProvenance::from(&opened.model.resolved);
    let route_warnings = opened.warnings;
    check_cancelled(context)?;

    let stage = Instant::now();
    let mut audio =
        VideoAudioBridge::open(&request.input, first_video_seconds).map_err(|error| {
            ToolError::new(
                ToolErrorCode::OutputValidationFailed,
                format!("open source audio bridge: {error:#}"),
            )
        })?;
    let source_had_audio = audio.source_had_audio();
    let mut muxer = Muxer::open_timestamped_media_with_transport(
        transaction.staging_path(),
        output_width,
        output_height,
        metadata.time_base,
        fps_numerator,
        fps_denominator,
        source_had_audio,
        None,
        false,
        Some(context.resources.cpu_threads),
        VideoFrameTransport::Cpu,
    )
    .map_err(|error| {
        ToolError::new(
            ToolErrorCode::OutputValidationFailed,
            format!(
                "create upscaled video {}: {error:#}",
                transaction.staging_path().display()
            ),
        )
    })?;
    audio.attach(&mut muxer);
    let mut encode_seconds = stage.elapsed().as_secs_f64();

    let mut frames = 0_u64;
    let mut tiles = 0_u64;
    let mut inference_seconds = 0.0;
    let mut postprocess_seconds = 0.0;
    loop {
        let timed = if let Some(frame) = pending.pop_front() {
            Some(frame)
        } else if source_ended {
            None
        } else {
            let next = next_selected_frame(
                &mut decoder,
                request.range,
                metadata.start_pts,
                metadata.time_base,
                &mut decode_seconds,
                context,
            )?;
            if next.is_none() {
                source_ended = true;
            }
            next
        };
        let Some(timed) = timed else {
            break;
        };
        check_cancelled(context)?;
        let rebased_pts = timed.pts.checked_sub(first_source_pts).ok_or_else(|| {
            ToolError::invalid_input("upscale source PTS rebase overflowed the i64 time base")
        })?;
        timeline
            .observe(frames, rebased_pts, timed.duration_ticks)
            .map_err(|error| {
                ToolError::invalid_input(format!("observe upscale source PTS: {error:#}"))
            })?;

        emit_progress(context, ToolPhase::Inferencing, frames, estimated_frames);
        let upscaled = session.upscale(&timed.frame).map_err(|error| {
            ToolError::new(
                ToolErrorCode::InferenceFailed,
                format!("upscale video frame {frames}: {error:#}"),
            )
        })?;
        validate_output_geometry(
            &upscaled.frame,
            metadata.width,
            metadata.height,
            output_width,
            output_height,
        )?;
        tiles = tiles.checked_add(upscaled.tiles as u64).ok_or_else(|| {
            ToolError::new(ToolErrorCode::Internal, "upscale tile count overflowed")
        })?;
        inference_seconds += upscaled.inference_seconds;
        postprocess_seconds += (upscaled.total_seconds - upscaled.inference_seconds).max(0.0);
        check_cancelled(context)?;

        emit_progress(context, ToolPhase::Encoding, frames, estimated_frames);
        let stage = Instant::now();
        muxer
            .encode_video_at_pts_with_duration(&upscaled.frame, rebased_pts, timed.duration_ticks)
            .map_err(|error| {
                ToolError::new(
                    ToolErrorCode::OutputValidationFailed,
                    format!("encode upscaled video frame {frames}: {error:#}"),
                )
            })?;
        let next_frames = frames.checked_add(1).ok_or_else(|| {
            ToolError::new(ToolErrorCode::Internal, "upscale frame count overflowed")
        })?;
        let output_seconds = timeline.pts_seconds(rebased_pts).map_err(|error| {
            ToolError::new(
                ToolErrorCode::OutputValidationFailed,
                format!("derive output audio position: {error:#}"),
            )
        })?;
        audio
            .pump_to_seconds(&mut muxer, output_seconds)
            .map_err(|error| {
                ToolError::new(
                    ToolErrorCode::OutputValidationFailed,
                    format!("encode source audio through {output_seconds:.6}s: {error:#}"),
                )
            })?;
        encode_seconds += stage.elapsed().as_secs_f64();
        frames = next_frames;
    }
    let pipeline = pending.usage();

    let duration_seconds = timeline.duration_seconds().map_err(|error| {
        ToolError::new(
            ToolErrorCode::OutputValidationFailed,
            format!("derive upscaled video duration: {error:#}"),
        )
    })?;
    let source_was_vfr = timeline.variable_frame_rate();
    let stage = Instant::now();
    audio
        .pump_to_seconds(&mut muxer, duration_seconds)
        .map_err(|error| {
            ToolError::new(
                ToolErrorCode::OutputValidationFailed,
                format!("finish source audio through {duration_seconds:.6}s: {error:#}"),
            )
        })?;
    muxer.finish().map_err(|error| {
        ToolError::new(
            ToolErrorCode::OutputValidationFailed,
            format!("finish upscaled video: {error:#}"),
        )
    })?;
    encode_seconds += stage.elapsed().as_secs_f64();
    drop(muxer);
    drop(session);

    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Validating));
    check_cancelled(context)?;
    validate_staged_video(
        transaction.staging_path(),
        output_width,
        output_height,
        frames,
        source_was_vfr,
        source_had_audio,
        duration_seconds,
        f64::from(fps_denominator) / f64::from(fps_numerator),
        context.resources.cpu_threads,
    )?;
    let output = transaction.commit()?;
    let output_artifact = OutputArtifact {
        role: "upscaled_media".to_owned(),
        path: output,
        media_type: "video/mp4".to_owned(),
        summary: Some(
            MediaSummary {
                duration_seconds: Some(duration_seconds),
                width: Some(output_width),
                height: Some(output_height),
                frame_count: Some(frames),
                sample_rate_hz: source_had_audio.then_some(48_000),
                channels: source_had_audio.then_some(2),
                sample_format: source_had_audio.then(|| "fltp".to_owned()),
                ..MediaSummary::default()
            }
            .with_video_encoding(
                "mp4",
                "h264",
                source_had_audio.then_some("aac"),
                "yuv420p",
                AlphaMode::Opaque,
            ),
        ),
    };
    let mut warnings = route_warnings;
    warnings.extend(video_warnings(source_had_audio));
    let total_seconds = started.elapsed().as_secs_f64();
    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Completed));

    Ok(ToolRun {
        result: UpscaleResult {
            outputs: vec![output_artifact.clone()],
            frames,
            source_width: metadata.width,
            source_height: metadata.height,
            output_width,
            output_height,
            scale: PUBLISHED_SCALE,
            tiles,
            duration_seconds: Some(duration_seconds),
            milliseconds_per_frame: total_seconds * 1_000.0 / frames as f64,
            pipeline: Some(pipeline),
        },
        report: ProcessingReport {
            operation: "upscale".to_owned(),
            parameters: normalized_video_parameters(
                &request,
                metadata.rotation_degrees,
                metadata.sample_aspect_ratio,
                fps_numerator,
                fps_denominator,
                first_source_pts,
                metadata.time_base,
                source_had_audio,
            ),
            input: request.input,
            input_media_type: "video/*".to_owned(),
            input_summary: MediaSummary {
                duration_seconds: metadata.duration_seconds(),
                width: Some(metadata.width),
                height: Some(metadata.height),
                ..MediaSummary::default()
            },
            outputs: vec![output_artifact],
            models: vec![provenance],
            processed: ProcessedMedia {
                frames,
                audio_seconds: if source_had_audio {
                    duration_seconds
                } else {
                    0.0
                },
            },
            timing: StageTiming {
                load_seconds,
                decode_seconds,
                preprocess_seconds: 0.0,
                inference_seconds,
                postprocess_seconds,
                encode_seconds,
                total_seconds,
            },
        },
        warnings,
    })
}

fn validate_request(request: &UpscaleRequest) -> Result<InputKind, ToolError> {
    if !request.input.is_file() {
        return Err(ToolError::invalid_input(format!(
            "upscale input does not exist or is not a file: {}",
            request.input.display()
        )));
    }
    if request.model.id != MODEL_ID {
        return Err(ToolError::new(
            ToolErrorCode::UnsupportedAdapter,
            format!(
                "upscale supports model {MODEL_ID:?}, got {:?}",
                request.model.id
            ),
        ));
    }
    if request.scale != PUBLISHED_SCALE {
        return Err(ToolError::invalid_input(format!(
            "the published Real-ESRGAN model supports only native x{PUBLISHED_SCALE}; got x{}",
            request.scale
        )));
    }
    if paths_refer_to_same_file(&request.input, &request.output) {
        return Err(ToolError::invalid_input(
            "upscale input and output must be different files",
        ));
    }

    let kind = input_kind(&request.input);
    match kind {
        InputKind::Png if !has_extension(&request.output, "png") => {
            return Err(ToolError::invalid_input(
                "PNG upscale input requires a .png output",
            ));
        }
        InputKind::Png if request.range.is_some() => {
            return Err(ToolError::invalid_input(
                "upscale range is valid only for video input",
            ));
        }
        InputKind::Video if !has_extension(&request.output, "mp4") => {
            return Err(ToolError::invalid_input(
                "video upscale output must end in .mp4",
            ));
        }
        _ => {}
    }
    if let Some(range) = request.range {
        TimeRange::new(range.start, range.end)?;
    }
    Ok(kind)
}

fn next_selected_frame(
    decoder: &mut SequentialRgbaDecoder,
    range: Option<TimeRange>,
    source_start_pts: i64,
    time_base: Rational,
    decode_seconds: &mut f64,
    context: &mut RunContext<'_>,
) -> Result<Option<TimedVideoFrame<RgbaFrame>>, ToolError> {
    loop {
        check_cancelled(context)?;
        let stage = Instant::now();
        let next = decoder.next_frame().map_err(|error| {
            ToolError::invalid_input(format!("decode sequential upscale frame: {error:#}"))
        })?;
        *decode_seconds += stage.elapsed().as_secs_f64();
        let Some(frame) = next else {
            return Ok(None);
        };
        let local_pts = frame.pts.checked_sub(source_start_pts).ok_or_else(|| {
            ToolError::invalid_input("upscale source-local PTS conversion overflowed")
        })?;
        let local_seconds = local_pts as f64 * rational_seconds(time_base);
        let start = range.map_or(0.0, |range| range.start);
        if local_seconds < start {
            continue;
        }
        if range
            .and_then(|range| range.end)
            .is_some_and(|end| local_seconds >= end)
        {
            return Ok(None);
        }
        return Ok(Some(frame));
    }
}

fn validate_video_range(
    range: Option<TimeRange>,
    duration_ticks: Option<i64>,
    time_base: Rational,
) -> Result<(), ToolError> {
    let Some(range) = range else {
        return Ok(());
    };
    let Some(duration_ticks) = duration_ticks else {
        return Ok(());
    };
    let duration = duration_ticks as f64 * rational_seconds(time_base);
    if range.start >= duration {
        return Err(ToolError::invalid_input(format!(
            "upscale range starts at {:.6}s but the video duration is {:.6}s",
            range.start, duration
        )));
    }
    if range
        .end
        .is_some_and(|end| end > duration + rational_seconds(time_base).abs())
    {
        return Err(ToolError::invalid_input(format!(
            "upscale range end exceeds the video duration of {duration:.6}s"
        )));
    }
    Ok(())
}

fn estimated_selected_frames(
    range: Option<TimeRange>,
    source_duration: Option<f64>,
    nominal_fps: f64,
) -> Option<u64> {
    if !nominal_fps.is_finite() || nominal_fps <= 0.0 {
        return None;
    }
    let start = range.map_or(0.0, |range| range.start);
    let end = range.and_then(|range| range.end).or(source_duration)?;
    (end > start).then(|| ((end - start) * nominal_fps).ceil() as u64)
}

fn emit_progress(
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

#[allow(clippy::too_many_arguments)]
fn validate_staged_video(
    path: &Path,
    width: u32,
    height: u32,
    expected_frames: u64,
    expected_vfr: bool,
    expect_audio: bool,
    duration_seconds: f64,
    frame_seconds: f64,
    threads: usize,
) -> Result<(), ToolError> {
    let probe = probe_av(path).map_err(|error| {
        ToolError::new(
            ToolErrorCode::OutputValidationFailed,
            format!("probe staged upscaled video {}: {error:#}", path.display()),
        )
    })?;
    let (staged_width, staged_height, video_duration) = probe.video.ok_or_else(|| {
        ToolError::new(
            ToolErrorCode::OutputValidationFailed,
            "upscaled video has no video stream",
        )
    })?;
    if (staged_width, staged_height) != (width, height) {
        return Err(ToolError::new(
            ToolErrorCode::OutputValidationFailed,
            format!(
                "upscaled video dimensions are {staged_width}x{staged_height}, expected {width}x{height}"
            ),
        ));
    }
    let audio_duration = match (expect_audio, probe.audio) {
        (true, Some(duration)) => Some(duration),
        (true, None) => {
            return Err(ToolError::new(
                ToolErrorCode::OutputValidationFailed,
                "upscaled MP4 lost the source audio stream",
            ));
        }
        (false, None) => None,
        (false, Some(_)) => {
            return Err(ToolError::new(
                ToolErrorCode::OutputValidationFailed,
                "upscaled MP4 invented an audio stream for a video-only source",
            ));
        }
    };
    // H.264/AAC stream durations are allowed one video frame or one AAC frame of container
    // rounding. Larger drift indicates the audio bridge or muxer did not preserve the output clock.
    let tolerance = frame_seconds.max(1024.0 / 48_000.0);
    let decoded = decode_video_timing(path, threads).map_err(|error| {
        ToolError::new(
            ToolErrorCode::OutputValidationFailed,
            format!("decode staged upscaled video timing: {error:#}"),
        )
    })?;
    if decoded.frame_count != expected_frames {
        return Err(ToolError::new(
            ToolErrorCode::OutputValidationFailed,
            format!(
                "upscaled video decoded {} frames, expected {expected_frames}",
                decoded.frame_count
            ),
        ));
    }
    if decoded.variable_frame_rate != expected_vfr {
        return Err(ToolError::new(
            ToolErrorCode::OutputValidationFailed,
            format!(
                "upscaled video VFR mode drifted: decoded={}, expected={expected_vfr}",
                decoded.variable_frame_rate
            ),
        ));
    }
    if (decoded.duration_seconds - duration_seconds).abs() > tolerance {
        return Err(ToolError::new(
            ToolErrorCode::OutputValidationFailed,
            format!(
                "upscaled decoded timeline is {:.6}s, expected {duration_seconds:.6}s",
                decoded.duration_seconds
            ),
        ));
    }
    if (video_duration - duration_seconds).abs() > tolerance {
        return Err(ToolError::new(
            ToolErrorCode::OutputValidationFailed,
            format!(
                "upscaled video duration is {video_duration:.6}s, expected {duration_seconds:.6}s"
            ),
        ));
    }
    if audio_duration.is_some_and(|audio| (audio - duration_seconds).abs() > tolerance) {
        return Err(ToolError::new(
            ToolErrorCode::OutputValidationFailed,
            format!(
                "upscaled audio duration is {:.6}s, expected {duration_seconds:.6}s",
                audio_duration.unwrap()
            ),
        ));
    }
    Ok(())
}

fn video_warnings(source_had_audio: bool) -> Vec<ToolWarning> {
    let mut warnings = vec![ToolWarning {
        code: "color_metadata_normalized".to_owned(),
        message: "output video is normalized to BT.709 limited range; source color metadata is not retained by the current output muxer"
            .to_owned(),
    }];
    if source_had_audio {
        warnings.push(ToolWarning {
            code: "audio_reencoded".to_owned(),
            message: "the best source audio stream was decoded to 48 kHz stereo and re-encoded as AAC; secondary audio streams are not copied because the current in-process muxer has no packet-remux path"
                .to_owned(),
        });
    }
    warnings
}

fn validate_geometry(width: u32, height: u32) -> Result<(u32, u32), ToolError> {
    if width == 0 || height == 0 {
        return Err(ToolError::invalid_input(
            "upscale source dimensions must be positive",
        ));
    }
    let output_width = width
        .checked_mul(PUBLISHED_SCALE)
        .ok_or_else(|| ToolError::invalid_input("upscaled width exceeds u32"))?;
    let output_height = height
        .checked_mul(PUBLISHED_SCALE)
        .ok_or_else(|| ToolError::invalid_input("upscaled height exceeds u32"))?;
    let output_pixels = u64::from(output_width) * u64::from(output_height);
    if output_pixels > MAX_OUTPUT_PIXELS {
        return Err(ToolError::invalid_input(format!(
            "x{PUBLISHED_SCALE} output has {output_pixels} pixels; maximum is {MAX_OUTPUT_PIXELS}"
        )));
    }
    Ok((output_width, output_height))
}

fn validate_output_geometry(
    output: &crate::frame::RgbaFrame,
    source_width: u32,
    source_height: u32,
    expected_width: u32,
    expected_height: u32,
) -> Result<(), ToolError> {
    if output.width != expected_width || output.height != expected_height {
        return Err(ToolError::new(
            ToolErrorCode::InferenceFailed,
            format!(
                "Real-ESRGAN x{PUBLISHED_SCALE} geometry mismatch: {source_width}x{source_height} became {}x{} instead of {expected_width}x{expected_height}",
                output.width, output.height
            ),
        ));
    }
    let expected_bytes = usize::try_from(expected_width)
        .ok()
        .and_then(|width| {
            usize::try_from(expected_height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| {
            ToolError::new(
                ToolErrorCode::InferenceFailed,
                "Real-ESRGAN output storage exceeds addressable memory",
            )
        })?;
    if output.data.len() != expected_bytes {
        return Err(ToolError::new(
            ToolErrorCode::InferenceFailed,
            "Real-ESRGAN output is not tightly packed RGBA8",
        ));
    }
    Ok(())
}

fn normalized_parameters(
    request: &UpscaleRequest,
    timeline_policy: &str,
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
    parameters.insert("scale".to_owned(), PUBLISHED_SCALE.into());
    parameters.insert("modelScale".to_owned(), PUBLISHED_SCALE.into());
    parameters.insert("tileWidth".to_owned(), TileConfig::default().width.into());
    parameters.insert("tileHeight".to_owned(), TileConfig::default().height.into());
    parameters.insert(
        "tileOverlap".to_owned(),
        TileConfig::default().overlap.into(),
    );
    parameters.insert("timelinePolicy".to_owned(), timeline_policy.into());
    parameters.insert(
        "range".to_owned(),
        serde_json::to_value(request.range).expect("range is serializable"),
    );
    parameters
}

#[allow(clippy::too_many_arguments)]
fn normalized_video_parameters(
    request: &UpscaleRequest,
    source_rotation_degrees: u32,
    source_sample_aspect_ratio: Rational,
    fps_numerator: u32,
    fps_denominator: u32,
    first_source_pts: i64,
    source_time_base: Rational,
    source_had_audio: bool,
) -> BTreeMap<String, serde_json::Value> {
    let mut parameters = normalized_parameters(request, "preserve_source_pts_vfr_capable");
    parameters.insert(
        "sourceRotationDegrees".to_owned(),
        source_rotation_degrees.into(),
    );
    parameters.insert("outputRotationDegrees".to_owned(), 0.into());
    parameters.insert(
        "sourceSampleAspectRatio".to_owned(),
        format!(
            "{}:{}",
            source_sample_aspect_ratio.numerator(),
            source_sample_aspect_ratio.denominator()
        )
        .into(),
    );
    parameters.insert("outputSampleAspectRatio".to_owned(), "1:1".into());
    parameters.insert(
        "nominalEncoderFrameRate".to_owned(),
        format!("{fps_numerator}/{fps_denominator}").into(),
    );
    parameters.insert("firstSourcePts".to_owned(), first_source_pts.into());
    parameters.insert(
        "sourceTimeBase".to_owned(),
        format!(
            "{}/{}",
            source_time_base.numerator(),
            source_time_base.denominator()
        )
        .into(),
    );
    parameters.insert(
        "audioPolicy".to_owned(),
        if source_had_audio {
            "decode_48khz_stereo_and_reencode_aac"
        } else {
            "preserve_absent_audio_stream"
        }
        .into(),
    );
    parameters.insert("colorPolicy".to_owned(), "normalize_bt709_limited".into());
    parameters
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

fn rational_seconds(value: Rational) -> f64 {
    value.numerator() as f64 / value.denominator() as f64
}

fn check_cancelled(context: &RunContext<'_>) -> Result<(), ToolError> {
    if context.cancellation.is_cancelled() {
        Err(ToolError::cancelled())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(input: PathBuf, output: PathBuf) -> UpscaleRequest {
        UpscaleRequest {
            input,
            output,
            model: ModelSelection::pinned_default(MODEL_ID),
            scale: PUBLISHED_SCALE,
            range: None,
            overwrite: false,
        }
    }

    #[test]
    fn published_x4_geometry_is_checked_without_allocating_the_output() {
        assert_eq!(validate_geometry(1920, 1080).unwrap(), (7680, 4320));
        assert!(validate_geometry(u32::MAX, 1).is_err());
        assert!(validate_geometry(3840, 2160).is_err());
    }

    #[test]
    fn non_native_scale_is_rejected_before_model_work() {
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("input.png");
        std::fs::write(&input, b"fixture").unwrap();
        let mut request = request(input, root.path().join("output.png"));
        request.scale = 2;
        assert_eq!(
            validate_request(&request).unwrap_err().code,
            ToolErrorCode::InvalidInput
        );
    }

    #[test]
    fn png_range_and_mismatched_output_extension_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("input.png");
        std::fs::write(&input, b"fixture").unwrap();
        let mut request = request(input, root.path().join("output.mp4"));
        assert!(validate_request(&request).is_err());
        request.output = root.path().join("output.png");
        request.range = Some(TimeRange::new(0.0, Some(1.0)).unwrap());
        assert!(validate_request(&request).is_err());
    }
}
