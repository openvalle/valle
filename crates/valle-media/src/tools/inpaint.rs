//! Complete-file LaMa image and video inpainting.
//!
//! PNG jobs decode one image/mask pair. Video jobs preflight the mask once to select exactly one
//! fixed LaMa bucket, then reopen it and process source/mask frames in strict PTS lockstep. Full
//! pairs compare absolute PTS; an explicit range selects/rebases the source against a zero-based
//! range mask. The live media window is two paired frames while a nominal encoder hint is derived
//! and one frame thereafter; source VFR PTS remain authoritative and no complete video is retained.

use std::{collections::BTreeMap, path::PathBuf, time::Instant};

use crate::models::inference::lama::CanvasBucket;
use serde::{Deserialize, Serialize};

use crate::{
    codec::{Muxer, VideoFrameTransport, probe_av, read_gray8_png, read_rgba_png, write_rgba_png},
    models::{
        ModelSelection,
        adapters::lama::{InpaintFrameOutput, InpaintModel},
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
        MatchedVideoFrame, MatchedVideoReader, SequentialGray8Decoder, SourceTimeline,
        VideoAudioBridge, decode_video_timing,
    },
};

const MODEL_ID: &str = "lama";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InpaintRequest {
    pub input: PathBuf,
    pub mask: PathBuf,
    pub output: PathBuf,
    pub model: ModelSelection,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<TimeRange>,
    pub overwrite: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InpaintResult {
    pub outputs: Vec<OutputArtifact>,
    pub frames: u64,
    pub width: u32,
    pub height: u32,
    pub duration_seconds: Option<f64>,
    pub tasks: u64,
    pub scaled_tasks: u64,
    pub milliseconds_per_frame: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pipeline: Option<PipelineUsage>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InputKind {
    Png,
    Video,
}

#[derive(Debug, Clone, Copy)]
struct MaskPreflight {
    frames: u64,
    bucket: Option<CanvasBucket>,
    decode_seconds: f64,
    preprocess_seconds: f64,
}

pub fn run(
    request: InpaintRequest,
    context: &mut RunContext<'_>,
) -> Result<ToolRun<InpaintResult>, ToolError> {
    context.validate()?;
    let kind = validate_request(&request)?;
    FileOutputTransaction::validate_target(&request.output, request.overwrite)?;
    let started = Instant::now();

    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Resolving));
    check_cancelled(context)?;
    let candidates = ModelSessionCandidates::resolve(context.models, request.model.clone())?;
    let transaction = FileOutputTransaction::new(&request.output, request.overwrite)?;

    match kind {
        InputKind::Png => run_png(request, transaction, candidates, started, context),
        InputKind::Video => run_video(request, transaction, candidates, started, context),
    }
}

fn run_png(
    request: InpaintRequest,
    transaction: FileOutputTransaction,
    candidates: ModelSessionCandidates,
    started: Instant,
    context: &mut RunContext<'_>,
) -> Result<ToolRun<InpaintResult>, ToolError> {
    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Decoding));
    let stage = Instant::now();
    let source = read_rgba_png(&request.input).map_err(|error| {
        ToolError::invalid_input(format!(
            "decode inpaint image {}: {error:#}",
            request.input.display()
        ))
    })?;
    let mask = read_gray8_png(&request.mask).map_err(|error| {
        ToolError::invalid_input(format!(
            "decode inpaint mask {}: {error:#}",
            request.mask.display()
        ))
    })?;
    let decode_seconds = stage.elapsed().as_secs_f64();
    validate_frame_pair(&source, &mask, 0)?;
    check_cancelled(context)?;

    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Preprocessing));
    let stage = Instant::now();
    let plan = InpaintModel::plan(&mask).map_err(|error| {
        ToolError::invalid_input(format!(
            "plan image mask {}: {error:#}",
            request.mask.display()
        ))
    })?;
    let preprocess_seconds = stage.elapsed().as_secs_f64();

    let mut provenance = ModelProvenance::from(&candidates.preferred_model().resolved);
    let mut route_warnings = Vec::new();
    let (output, load_seconds, inference_seconds, postprocess_seconds, tasks, scaled_tasks) =
        if let Some(bucket) = plan.bucket {
            context.progress.event(ToolEvent::phase(ToolPhase::Loading));
            let load_started = Instant::now();
            let opened = candidates.open("load LaMa", |model| {
                InpaintModel::from_resolved(model, context.resources.cpu_threads)?
                    .open_session(bucket)
            })?;
            let mut session = opened.session;
            provenance = ModelProvenance::from(&opened.model.resolved);
            route_warnings = opened.warnings;
            debug_assert_eq!(session.bucket(), bucket);
            let load_seconds = load_started
                .elapsed()
                .as_secs_f64()
                .max(session.load_seconds());
            check_cancelled(context)?;
            context
                .progress
                .event(ToolEvent::count(ToolPhase::Inferencing, 0, 1));
            let processed = session.inpaint(&source, &mask).map_err(|error| {
                ToolError::new(
                    ToolErrorCode::InferenceFailed,
                    format!("inpaint image: {error:#}"),
                )
            })?;
            validate_model_output(&processed, &plan, source.width, source.height, 0)?;
            (
                processed.frame,
                load_seconds,
                processed.inference_seconds,
                (processed.total_seconds - processed.inference_seconds).max(0.0),
                processed.tasks as u64,
                processed.scaled_tasks as u64,
            )
        } else {
            (source.clone(), 0.0, 0.0, 0.0, 0, 0)
        };
    check_cancelled(context)?;

    context
        .progress
        .event(ToolEvent::count(ToolPhase::Encoding, 0, 1));
    let stage = Instant::now();
    write_rgba_png(transaction.staging_path(), &output).map_err(|error| {
        output_error(
            "write staged inpaint PNG",
            anyhow::anyhow!("{}: {error:#}", transaction.staging_path().display()),
        )
    })?;
    let round_trip = read_rgba_png(transaction.staging_path())
        .map_err(|error| output_error("validate staged inpaint PNG", error))?;
    if round_trip != output {
        return Err(ToolError::new(
            ToolErrorCode::OutputValidationFailed,
            "staged inpaint PNG did not round-trip exactly as RGBA8",
        ));
    }
    let encode_seconds = stage.elapsed().as_secs_f64();
    check_cancelled(context)?;
    let output_path = transaction.commit()?;

    let output_artifact = OutputArtifact {
        role: "inpainted_image".to_owned(),
        path: output_path,
        media_type: "image/png".to_owned(),
        summary: Some(
            MediaSummary {
                width: Some(output.width),
                height: Some(output.height),
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
        result: InpaintResult {
            outputs: vec![output_artifact.clone()],
            frames: 1,
            width: output.width,
            height: output.height,
            duration_seconds: None,
            tasks,
            scaled_tasks,
            milliseconds_per_frame: total_seconds * 1_000.0,
            pipeline: None,
        },
        report: ProcessingReport {
            operation: "inpaint".to_owned(),
            parameters: normalized_parameters(&request, "png/rgba8"),
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
                preprocess_seconds,
                inference_seconds,
                postprocess_seconds,
                encode_seconds,
                total_seconds,
            },
        },
        warnings: route_warnings,
    })
}

#[allow(clippy::too_many_arguments)]
fn run_video(
    request: InpaintRequest,
    transaction: FileOutputTransaction,
    candidates: ModelSessionCandidates,
    started: Instant,
    context: &mut RunContext<'_>,
) -> Result<ToolRun<InpaintResult>, ToolError> {
    if context.resources.pipeline_capacity < 2 {
        return Err(ToolError::new(
            ToolErrorCode::ResourceBusy,
            "video inpaint requires pipeline_capacity >= 2 for bounded timestamp lookahead",
        ));
    }
    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Decoding));
    let preflight = preflight_video_mask(&request.mask, request.range.is_some(), context)?;
    check_cancelled(context)?;

    let stage = Instant::now();
    let mut reader = MatchedVideoReader::open(
        &request.input,
        &request.mask,
        request.range,
        context.resources.cpu_threads,
    )
    .map_err(|error| {
        ToolError::invalid_input(format!("open strict inpaint video pair: {error:#}"))
    })?;
    let source_metadata = reader.source_metadata();
    let mask_metadata = reader.mask_metadata();
    validate_video_range(request.range, source_metadata)?;
    if source_metadata.width % 2 != 0 || source_metadata.height % 2 != 0 {
        return Err(ToolError::invalid_input(format!(
            "inpaint MP4 output requires even display dimensions, got {}x{}",
            source_metadata.width, source_metadata.height
        )));
    }
    let first = reader
        .next_frame()
        .map_err(pair_decode_error)?
        .ok_or_else(|| ToolError::invalid_input("inpaint source and mask videos are empty"))?;
    let second = reader.next_frame().map_err(pair_decode_error)?;
    let decode_open_seconds = stage.elapsed().as_secs_f64();
    let mut clock = SourceTimeline::derive(
        first.pts,
        second.as_ref().map(|frame| frame.pts),
        first.time_base,
        source_metadata.nominal_fps,
    )
    .map_err(|error| ToolError::invalid_input(error.to_string()))?;
    let (fps_numerator, fps_denominator) = clock.fps();

    // Validate the pair origin and derive a nominal encoder hint before allocating a model
    // session. Actual source PTS remain authoritative, including VFR deltas.
    context.progress.event(ToolEvent::phase(ToolPhase::Loading));
    let mut provenance = ModelProvenance::from(&candidates.preferred_model().resolved);
    let (mut session, route_warnings, load_seconds) = if let Some(bucket) = preflight.bucket {
        let load_started = Instant::now();
        let opened = candidates.open("load LaMa", |model| {
            InpaintModel::from_resolved(model, context.resources.cpu_threads)?.open_session(bucket)
        })?;
        provenance = ModelProvenance::from(&opened.model.resolved);
        let load_seconds = load_started
            .elapsed()
            .as_secs_f64()
            .max(opened.session.load_seconds());
        (Some(opened.session), opened.warnings, load_seconds)
    } else {
        (None, Vec::new(), 0.0)
    };
    if let (Some(session), Some(bucket)) = (session.as_ref(), preflight.bucket) {
        debug_assert_eq!(session.bucket(), bucket);
    }
    check_cancelled(context)?;

    let first_video_seconds = first.absolute_source_seconds();
    let mut audio =
        VideoAudioBridge::open(&request.input, first_video_seconds).map_err(|error| {
            ToolError::invalid_input(format!("open source audio for inpaint: {error:#}"))
        })?;
    let source_had_audio = audio.source_had_audio();
    let mut muxer = Muxer::open_timestamped_media_with_transport(
        transaction.staging_path(),
        source_metadata.width,
        source_metadata.height,
        first.time_base,
        fps_numerator,
        fps_denominator,
        source_had_audio,
        None,
        false,
        Some(context.resources.cpu_threads),
        VideoFrameTransport::Cpu,
    )
    .map_err(|error| output_error("create staged inpaint MP4", error))?;
    audio.attach(&mut muxer);

    let mut pending = BoundedPipeline::new(context.resources.pipeline_capacity)?;
    pending.push_back(first)?;
    if let Some(second) = second {
        pending.push_back(second)?;
    }
    let mut frames = 0_u64;
    let mut tasks = 0_u64;
    let mut scaled_tasks = 0_u64;
    let mut decode_seconds = preflight.decode_seconds + decode_open_seconds;
    let mut preprocess_seconds = preflight.preprocess_seconds;
    let mut inference_seconds = 0.0;
    let mut postprocess_seconds = 0.0;
    let mut encode_seconds = 0.0;

    loop {
        check_cancelled(context)?;
        let frame = if let Some(frame) = pending.pop_front() {
            Some(frame)
        } else {
            context.progress.event(ToolEvent::count(
                ToolPhase::Decoding,
                frames,
                preflight.frames,
            ));
            let stage = Instant::now();
            let decoded = reader.next_frame().map_err(pair_decode_error)?;
            decode_seconds += stage.elapsed().as_secs_f64();
            decoded
        };
        let Some(frame) = frame else {
            break;
        };
        clock
            .observe(frames, frame.pts, frame.duration_ticks)
            .map_err(|error| ToolError::invalid_input(error.to_string()))?;
        validate_frame_pair(&frame.source, &frame.mask, frames)?;

        context.progress.event(ToolEvent::count(
            ToolPhase::Preprocessing,
            frames,
            preflight.frames,
        ));
        let stage = Instant::now();
        let plan = InpaintModel::plan(&frame.mask).map_err(|error| {
            ToolError::invalid_input(format!("plan video mask frame {frames}: {error:#}"))
        })?;
        preprocess_seconds += stage.elapsed().as_secs_f64();
        if let Some(required) = plan.bucket
            && preflight
                .bucket
                .is_none_or(|loaded| !bucket_contains(loaded, required))
        {
            return Err(ToolError::new(
                ToolErrorCode::Internal,
                format!(
                    "mask plan no longer fits the preflight bucket at frame {frames}: required={required:?}, loaded={:?}",
                    preflight.bucket
                ),
            ));
        }

        context.progress.event(ToolEvent::count(
            ToolPhase::Inferencing,
            frames,
            preflight.frames,
        ));
        let processed = if let Some(session) = session.as_mut() {
            session
                .inpaint(&frame.source, &frame.mask)
                .map_err(|error| {
                    ToolError::new(
                        ToolErrorCode::InferenceFailed,
                        format!("inpaint video frame {frames}: {error:#}"),
                    )
                })?
        } else {
            InpaintFrameOutput {
                frame: frame.source.clone(),
                plan: plan.clone(),
                inference_seconds: 0.0,
                total_seconds: 0.0,
                tasks: 0,
                scaled_tasks: 0,
            }
        };
        validate_model_output(
            &processed,
            &plan,
            source_metadata.width,
            source_metadata.height,
            frames,
        )?;
        inference_seconds += processed.inference_seconds;
        postprocess_seconds += (processed.total_seconds - processed.inference_seconds).max(0.0);
        tasks = tasks.checked_add(processed.tasks as u64).ok_or_else(|| {
            ToolError::new(ToolErrorCode::Internal, "inpaint task count overflowed")
        })?;
        scaled_tasks = scaled_tasks
            .checked_add(processed.scaled_tasks as u64)
            .ok_or_else(|| {
                ToolError::new(
                    ToolErrorCode::Internal,
                    "inpaint scaled-task count overflowed",
                )
            })?;

        check_cancelled(context)?;
        context.progress.event(ToolEvent::count(
            ToolPhase::Encoding,
            frames,
            preflight.frames,
        ));
        let stage = Instant::now();
        muxer
            .encode_video_at_pts_with_duration(&processed.frame, frame.pts, frame.duration_ticks)
            .map_err(|error| {
                output_error(&format!("encode inpaint video frame {frames}"), error)
            })?;
        let completed_frames = frames.checked_add(1).ok_or_else(|| {
            ToolError::new(ToolErrorCode::Internal, "inpaint frame count overflowed")
        })?;
        let completed_seconds = clock
            .pts_seconds(frame.pts)
            .map_err(|error| ToolError::invalid_input(error.to_string()))?;
        audio
            .pump_to_seconds(&mut muxer, completed_seconds)
            .map_err(|error| output_error("transcode source audio", error))?;
        encode_seconds += stage.elapsed().as_secs_f64();
        frames = completed_frames;
    }
    let pipeline = pending.usage();

    if frames != preflight.frames || reader.frames_read() != preflight.frames {
        return Err(ToolError::invalid_input(format!(
            "source/mask frame count changed after preflight: preflight={}, processed={}, matched={}",
            preflight.frames,
            frames,
            reader.frames_read()
        )));
    }
    let duration_seconds = clock
        .duration_seconds()
        .map_err(|error| ToolError::invalid_input(error.to_string()))?;
    let source_was_vfr = clock.variable_frame_rate();
    check_cancelled(context)?;
    let stage = Instant::now();
    audio
        .pump_to_seconds(&mut muxer, duration_seconds)
        .map_err(|error| output_error("finish source audio", error))?;
    muxer
        .finish()
        .map_err(|error| output_error("finish staged inpaint MP4", error))?;
    encode_seconds += stage.elapsed().as_secs_f64();
    drop(muxer);

    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Validating));
    validate_staged_video(
        transaction.staging_path(),
        source_metadata.width,
        source_metadata.height,
        frames,
        source_was_vfr,
        source_had_audio,
        duration_seconds,
        f64::from(fps_denominator) / f64::from(fps_numerator),
        context.resources.cpu_threads,
    )?;
    check_cancelled(context)?;
    let output_path = transaction.commit()?;

    let output_artifact = OutputArtifact {
        role: "inpainted_video".to_owned(),
        path: output_path,
        media_type: "video/mp4".to_owned(),
        summary: Some(
            MediaSummary {
                duration_seconds: Some(duration_seconds),
                width: Some(source_metadata.width),
                height: Some(source_metadata.height),
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
    let total_seconds = started.elapsed().as_secs_f64();
    let mut warnings = route_warnings;
    warnings.extend([
        ToolWarning {
            code: "video_reencoded".to_owned(),
            message: "inpainted video was encoded as H.264/yuv420p; rotation was applied and SAR was normalized to 1:1".to_owned(),
        },
        ToolWarning {
            code: "color_metadata_normalized".to_owned(),
            message: "the current MP4 output path normalizes color metadata to BT.709 limited range"
                .to_owned(),
        },
    ]);
    if source_had_audio {
        warnings.push(ToolWarning {
            code: "audio_reencoded".to_owned(),
            message: "source audio was decoded and re-encoded as 48 kHz stereo AAC; it was not packet-remuxed losslessly".to_owned(),
        });
    }
    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Completed));
    Ok(ToolRun {
        result: InpaintResult {
            outputs: vec![output_artifact.clone()],
            frames,
            width: source_metadata.width,
            height: source_metadata.height,
            duration_seconds: Some(duration_seconds),
            tasks,
            scaled_tasks,
            milliseconds_per_frame: total_seconds * 1_000.0 / frames as f64,
            pipeline: Some(pipeline),
        },
        report: ProcessingReport {
            operation: "inpaint".to_owned(),
            parameters: normalized_video_parameters(
                &request,
                fps_numerator,
                fps_denominator,
                source_metadata,
                mask_metadata,
                source_had_audio,
            ),
            input: request.input,
            input_media_type: "video/*".to_owned(),
            input_summary: MediaSummary {
                duration_seconds: source_metadata.duration_seconds(),
                width: Some(source_metadata.width),
                height: Some(source_metadata.height),
                frame_count: Some(frames),
                pixel_format: Some("rgba8-straight-decoded".to_owned()),
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

fn preflight_video_mask(
    path: &std::path::Path,
    require_zero_start: bool,
    context: &mut RunContext<'_>,
) -> Result<MaskPreflight, ToolError> {
    let stage = Instant::now();
    let mut decoder =
        SequentialGray8Decoder::open(path, context.resources.cpu_threads).map_err(|error| {
            ToolError::invalid_input(format!(
                "open lossless Gray8 mask video {}: {error:#}",
                path.display()
            ))
        })?;
    let mut decode_seconds = stage.elapsed().as_secs_f64();
    let mut preprocess_seconds = 0.0;
    let mut frames = 0_u64;
    let mut bucket = None;
    loop {
        check_cancelled(context)?;
        let stage = Instant::now();
        let frame = decoder.next_frame().map_err(|error| {
            ToolError::invalid_input(format!("decode mask video frame {frames}: {error:#}"))
        })?;
        decode_seconds += stage.elapsed().as_secs_f64();
        let Some(frame) = frame else {
            break;
        };
        if require_zero_start && frames == 0 && frame.pts != 0 {
            return Err(ToolError::invalid_input(format!(
                "range mask must start at absolute PTS zero, got {} {}/{}",
                frame.pts,
                decoder.metadata().time_base.numerator(),
                decoder.metadata().time_base.denominator(),
            )));
        }
        let stage = Instant::now();
        let plan = InpaintModel::plan(&frame.frame).map_err(|error| {
            ToolError::invalid_input(format!("plan mask video frame {frames}: {error:#}"))
        })?;
        preprocess_seconds += stage.elapsed().as_secs_f64();
        merge_video_bucket(&mut bucket, plan.bucket, frames)?;
        frames = frames.checked_add(1).ok_or_else(|| {
            ToolError::new(ToolErrorCode::Internal, "mask frame count overflowed")
        })?;
    }
    if frames == 0 {
        return Err(ToolError::invalid_input("mask video contains no frames"));
    }
    Ok(MaskPreflight {
        frames,
        bucket,
        decode_seconds,
        preprocess_seconds,
    })
}

fn merge_video_bucket(
    selected: &mut Option<CanvasBucket>,
    required: Option<CanvasBucket>,
    frame_index: u64,
) -> Result<(), ToolError> {
    let Some(required) = required else {
        return Ok(());
    };
    if let Some(current) = *selected {
        if bucket_contains(current, required) {
            return Ok(());
        }
        if bucket_contains(required, current) {
            *selected = Some(required);
        } else {
            return Err(ToolError::invalid_input(format!(
                "mask video requires incomparable LaMa canvas buckets at frame {frame_index}: {current:?} and {required:?}"
            )));
        }
    } else {
        *selected = Some(required);
    }
    Ok(())
}

fn bucket_contains(loaded: CanvasBucket, required: CanvasBucket) -> bool {
    required.width() <= loaded.width() && required.height() <= loaded.height()
}

fn validate_video_range(
    range: Option<TimeRange>,
    metadata: super::video_sequence::VideoMetadata,
) -> Result<(), ToolError> {
    let Some(range) = range else {
        return Ok(());
    };
    let Some(duration) = metadata.duration_seconds() else {
        return Ok(());
    };
    if range.start >= duration {
        return Err(ToolError::invalid_input(format!(
            "inpaint range starts at {:.6}s but the source duration is {:.6}s",
            range.start, duration
        )));
    }
    let source_tick =
        metadata.time_base.numerator() as f64 / metadata.time_base.denominator() as f64;
    if range
        .end
        .is_some_and(|end| end > duration + source_tick.abs())
    {
        return Err(ToolError::invalid_input(format!(
            "inpaint range end exceeds the source duration of {duration:.6}s"
        )));
    }
    Ok(())
}

fn validate_request(request: &InpaintRequest) -> Result<InputKind, ToolError> {
    for (role, path) in [("input", &request.input), ("mask", &request.mask)] {
        if !path.is_file() {
            return Err(ToolError::invalid_input(format!(
                "{role} does not exist or is not a file: {}",
                path.display()
            )));
        }
    }
    if request.model.id != MODEL_ID {
        return Err(ToolError::new(
            ToolErrorCode::UnsupportedAdapter,
            format!(
                "inpaint supports model {MODEL_ID:?}, got {:?}",
                request.model.id
            ),
        ));
    }
    if paths_refer_to_same_file(&request.input, &request.mask)
        || paths_refer_to_same_file(&request.input, &request.output)
        || paths_refer_to_same_file(&request.mask, &request.output)
    {
        return Err(ToolError::invalid_input(
            "inpaint input, mask and output must be three different files",
        ));
    }
    let input_png = has_extension(&request.input, "png");
    let mask_png = has_extension(&request.mask, "png");
    match (input_png, mask_png) {
        (true, true) => {
            if request.range.is_some() {
                return Err(ToolError::invalid_input(
                    "inpaint range is valid only for video input",
                ));
            }
            if !has_extension(&request.output, "png") {
                return Err(ToolError::invalid_input(
                    "image inpaint output must end in .png",
                ));
            }
            Ok(InputKind::Png)
        }
        (false, false) => {
            if !has_extension(&request.output, "mp4") {
                return Err(ToolError::invalid_input(
                    "video inpaint output must end in .mp4",
                ));
            }
            if let Some(range) = request.range {
                TimeRange::new(range.start, range.end)?;
            }
            Ok(InputKind::Video)
        }
        _ => Err(ToolError::invalid_input(
            "inpaint input and mask must both be PNG images or both be videos",
        )),
    }
}

fn validate_frame_pair(
    source: &crate::frame::RgbaFrame,
    mask: &crate::frame::Gray8Frame,
    frame_index: u64,
) -> Result<(), ToolError> {
    if source.width != mask.width || source.height != mask.height {
        return Err(ToolError::invalid_input(format!(
            "source/mask dimensions differ at frame {frame_index}: {}x{} != {}x{}",
            source.width, source.height, mask.width, mask.height
        )));
    }
    if !mask.is_binary_mask() {
        return Err(ToolError::invalid_input(format!(
            "mask frame {frame_index} is not strict binary Gray8 (only 0 and 255 are allowed)"
        )));
    }
    Ok(())
}

fn validate_model_output(
    output: &InpaintFrameOutput,
    expected_plan: &crate::models::inference::lama::InpaintPlan,
    width: u32,
    height: u32,
    frame_index: u64,
) -> Result<(), ToolError> {
    if output.frame.width != width || output.frame.height != height {
        return Err(ToolError::new(
            ToolErrorCode::InferenceFailed,
            format!(
                "LaMa output dimensions drifted at frame {frame_index}: {}x{} != {width}x{height}",
                output.frame.width, output.frame.height
            ),
        ));
    }
    if output.plan != *expected_plan {
        return Err(ToolError::new(
            ToolErrorCode::InferenceFailed,
            format!("LaMa plan drifted at frame {frame_index}"),
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_staged_video(
    path: &std::path::Path,
    width: u32,
    height: u32,
    expected_frames: u64,
    expected_vfr: bool,
    expect_audio: bool,
    expected_duration: f64,
    frame_seconds: f64,
    threads: usize,
) -> Result<(), ToolError> {
    let probe = probe_av(path).map_err(|error| output_error("probe staged inpaint MP4", error))?;
    let Some((actual_width, actual_height, actual_duration)) = probe.video else {
        return Err(ToolError::new(
            ToolErrorCode::OutputValidationFailed,
            "staged inpaint MP4 has no video stream",
        ));
    };
    if (actual_width, actual_height) != (width, height) {
        return Err(ToolError::new(
            ToolErrorCode::OutputValidationFailed,
            format!(
                "staged video dimensions drifted: {actual_width}x{actual_height} != {width}x{height}"
            ),
        ));
    }
    if actual_duration > 0.0 && (actual_duration - expected_duration).abs() > frame_seconds + 1e-3 {
        return Err(ToolError::new(
            ToolErrorCode::OutputValidationFailed,
            format!(
                "staged video duration drifted: {actual_duration:.6}s != {expected_duration:.6}s"
            ),
        ));
    }
    let decoded = decode_video_timing(path, threads)
        .map_err(|error| output_error("decode staged inpaint timing", error))?;
    if decoded.frame_count != expected_frames {
        return Err(ToolError::new(
            ToolErrorCode::OutputValidationFailed,
            format!(
                "staged inpaint video decoded {} frames, expected {expected_frames}",
                decoded.frame_count
            ),
        ));
    }
    if decoded.variable_frame_rate != expected_vfr {
        return Err(ToolError::new(
            ToolErrorCode::OutputValidationFailed,
            format!(
                "staged inpaint VFR mode drifted: decoded={}, expected={expected_vfr}",
                decoded.variable_frame_rate
            ),
        ));
    }
    if (decoded.duration_seconds - expected_duration).abs() > frame_seconds + 1e-3 {
        return Err(ToolError::new(
            ToolErrorCode::OutputValidationFailed,
            format!(
                "staged inpaint decoded timeline drifted: {:.6}s != {expected_duration:.6}s",
                decoded.duration_seconds
            ),
        ));
    }
    let audio_duration = match (expect_audio, probe.audio) {
        (true, Some(duration)) => Some(duration),
        (false, None) => None,
        (true, None) => {
            return Err(ToolError::new(
                ToolErrorCode::OutputValidationFailed,
                "staged inpaint MP4 lost the source audio stream",
            ));
        }
        (false, Some(_)) => {
            return Err(ToolError::new(
                ToolErrorCode::OutputValidationFailed,
                "staged inpaint MP4 invented an audio stream for a video-only source",
            ));
        }
    };
    let audio_tolerance = frame_seconds.max(1024.0 / 48_000.0) + 1e-3;
    if let Some(audio_duration) = audio_duration
        && (audio_duration - expected_duration).abs() > audio_tolerance
    {
        return Err(ToolError::new(
            ToolErrorCode::OutputValidationFailed,
            format!(
                "staged inpaint audio duration is {:.6}s, expected {expected_duration:.6}s",
                audio_duration
            ),
        ));
    }
    Ok(())
}

fn normalized_parameters(
    request: &InpaintRequest,
    output_contract: &str,
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
    parameters.insert(
        "maskPath".to_owned(),
        request.mask.display().to_string().into(),
    );
    parameters.insert("maskContract".to_owned(), "gray8/binary/0-255".into());
    parameters.insert(
        "range".to_owned(),
        serde_json::to_value(request.range).expect("range is serializable"),
    );
    parameters.insert("outputContract".to_owned(), output_contract.into());
    parameters
}

fn normalized_video_parameters(
    request: &InpaintRequest,
    fps_numerator: u32,
    fps_denominator: u32,
    source: super::video_sequence::VideoMetadata,
    mask: super::video_sequence::VideoMetadata,
    source_had_audio: bool,
) -> BTreeMap<String, serde_json::Value> {
    let mut parameters = normalized_parameters(
        request,
        if source_had_audio {
            "mp4/h264/yuv420p/aac"
        } else {
            "mp4/h264/yuv420p/video-only"
        },
    );
    parameters.insert(
        "nominalEncoderFrameRate".to_owned(),
        format!("{fps_numerator}/{fps_denominator}").into(),
    );
    parameters.insert(
        "ptsPolicy".to_owned(),
        if request.range.is_some() {
            "strict-vfr-capable/source-range-and-mask-rebased-to-zero"
        } else {
            "strict-vfr-capable/absolute-pair-match/output-rebased-to-zero"
        }
        .into(),
    );
    parameters.insert(
        "pairPtsContract".to_owned(),
        "source-to-mask-time-base/round-nearest/exact-integer".into(),
    );
    parameters.insert(
        "sourceTimeBase".to_owned(),
        format!(
            "{}/{}",
            source.time_base.numerator(),
            source.time_base.denominator()
        )
        .into(),
    );
    parameters.insert(
        "maskTimeBase".to_owned(),
        format!(
            "{}/{}",
            mask.time_base.numerator(),
            mask.time_base.denominator()
        )
        .into(),
    );
    parameters.insert(
        "sourceRotationApplied".to_owned(),
        source.rotation_degrees.into(),
    );
    parameters.insert(
        "maskRotationApplied".to_owned(),
        mask.rotation_degrees.into(),
    );
    parameters.insert(
        "sourceSampleAspectRatio".to_owned(),
        format!(
            "{}/{}",
            source.sample_aspect_ratio.numerator(),
            source.sample_aspect_ratio.denominator()
        )
        .into(),
    );
    parameters.insert(
        "maskSampleAspectRatio".to_owned(),
        format!(
            "{}/{}",
            mask.sample_aspect_ratio.numerator(),
            mask.sample_aspect_ratio.denominator()
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
    parameters
}

fn has_extension(path: &std::path::Path, extension: &str) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case(extension))
}

fn pair_decode_error(error: anyhow::Error) -> ToolError {
    ToolError::invalid_input(format!("decode strict source/mask pair: {error:#}"))
}

fn output_error(action: &str, error: anyhow::Error) -> ToolError {
    ToolError::new(
        ToolErrorCode::OutputValidationFailed,
        format!("{action}: {error:#}"),
    )
}

fn check_cancelled(context: &RunContext<'_>) -> Result<(), ToolError> {
    if context.cancellation.is_cancelled() {
        Err(ToolError::cancelled())
    } else {
        Ok(())
    }
}

impl MatchedVideoFrame {
    fn absolute_source_seconds(&self) -> f64 {
        self.source_absolute_pts as f64 * self.time_base.numerator() as f64
            / self.time_base.denominator() as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{Gray8Frame, RgbaFrame};

    fn request(root: &std::path::Path, image: bool) -> InpaintRequest {
        let input = root.join(if image { "input.png" } else { "input.mp4" });
        let mask = root.join(if image { "mask.png" } else { "mask.mkv" });
        std::fs::write(&input, b"fixture").unwrap();
        std::fs::write(&mask, b"fixture").unwrap();
        InpaintRequest {
            input,
            mask,
            output: root.join(if image { "output.png" } else { "output.mp4" }),
            model: ModelSelection::pinned_default(MODEL_ID),
            range: None,
            overwrite: false,
        }
    }

    #[test]
    fn request_requires_matching_media_kinds_and_distinct_paths() {
        let root = tempfile::tempdir().unwrap();
        let mut image = request(root.path(), true);
        assert_eq!(validate_request(&image).unwrap(), InputKind::Png);
        image.range = Some(TimeRange::new(0.0, Some(1.0)).unwrap());
        assert_eq!(
            validate_request(&image).unwrap_err().code,
            ToolErrorCode::InvalidInput
        );
        image.range = None;
        image.mask = root.path().join("mask.mkv");
        std::fs::write(&image.mask, b"fixture").unwrap();
        assert_eq!(
            validate_request(&image).unwrap_err().code,
            ToolErrorCode::InvalidInput
        );
        let mut video = request(root.path(), false);
        video.output = video.input.clone();
        assert_eq!(
            validate_request(&video).unwrap_err().code,
            ToolErrorCode::InvalidInput
        );
    }

    #[test]
    fn strict_mask_pair_rejects_dimensions_and_soft_samples() {
        let source = RgbaFrame::new(2, 2);
        let binary = Gray8Frame::from_data(2, 2, vec![0, 255, 0, 255]).unwrap();
        assert!(validate_frame_pair(&source, &binary, 0).is_ok());
        let wrong = Gray8Frame::from_data(1, 2, vec![0, 255]).unwrap();
        assert!(validate_frame_pair(&source, &wrong, 0).is_err());
        let soft = Gray8Frame::from_data(2, 2, vec![0, 127, 0, 255]).unwrap();
        assert!(validate_frame_pair(&source, &soft, 0).is_err());
    }

    #[test]
    fn video_bucket_policy_requires_one_loadable_shape() {
        let small = Gray8Frame::from_data(64, 64, {
            let mut mask = vec![0; 64 * 64];
            mask[32 * 64 + 32] = 255;
            mask
        })
        .unwrap();
        let large = Gray8Frame::from_data(640, 384, vec![255; 640 * 384]).unwrap();
        assert_eq!(
            InpaintModel::plan(&small).unwrap().bucket,
            Some(CanvasBucket::Square256)
        );
        assert_eq!(
            InpaintModel::plan(&large).unwrap().bucket,
            Some(CanvasBucket::Wide640x384)
        );

        let mut selected = None;
        merge_video_bucket(&mut selected, Some(CanvasBucket::Square256), 0).unwrap();
        merge_video_bucket(&mut selected, Some(CanvasBucket::Wide640x384), 1).unwrap();
        assert_eq!(selected, Some(CanvasBucket::Wide640x384));
        merge_video_bucket(&mut selected, Some(CanvasBucket::Square256), 2).unwrap();
        assert_eq!(selected, Some(CanvasBucket::Wide640x384));
    }

    #[test]
    fn range_audio_origin_uses_selected_source_absolute_pts() {
        let frame = MatchedVideoFrame {
            pts: 0,
            source_absolute_pts: 225_000,
            time_base: ffmpeg_next::Rational(1, 90_000),
            duration_ticks: Some(3_000),
            source: RgbaFrame::new(2, 2),
            mask: Gray8Frame::new(2, 2),
        };
        assert_eq!(frame.absolute_source_seconds(), 2.5);
    }
}
