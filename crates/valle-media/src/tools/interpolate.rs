//! Complete-file RIFE frame interpolation with a bounded ordered-pair pipeline.

use std::{collections::BTreeMap, path::PathBuf, time::Instant};

use serde::{Deserialize, Serialize};

use crate::{
    analysis::ShotList,
    codec::{Muxer, VideoFrameTransport, probe_av},
    frame::RgbaFrame,
    models::{
        ModelSelection,
        adapters::rife::{InterpolationSession, MODEL_ID},
    },
};

use super::{
    AlphaMode, MediaSummary, ModelProvenance, OutputArtifact, ProcessedMedia, ProcessingReport,
    RunContext, StageTiming, ToolError, ToolErrorCode, ToolEvent, ToolPhase, ToolRun, ToolWarning,
    model_session::ModelSessionCandidates,
    output::{FileOutputTransaction, paths_refer_to_same_file},
    video_sequence::{
        SequentialRgbaDecoder, SourceTimeline, VideoAudioBridge, decode_video_timing,
    },
};

const MAX_SHOTS_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InterpolateRequest {
    pub input: PathBuf,
    pub output: PathBuf,
    pub model: ModelSelection,
    pub fps: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shots: Option<PathBuf>,
    pub overwrite: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InterpolateResult {
    pub outputs: Vec<OutputArtifact>,
    pub source_frames: u64,
    pub output_frames: u64,
    pub generated_frames: u64,
    pub held_cross_shot_frames: u64,
    pub source_fps: f64,
    pub output_fps: u32,
    pub source_duration_seconds: f64,
    pub output_duration_seconds: f64,
    pub milliseconds_per_generated_frame: f64,
}

pub fn run(
    request: InterpolateRequest,
    context: &mut RunContext<'_>,
) -> Result<ToolRun<InterpolateResult>, ToolError> {
    context.validate()?;
    validate_request(&request)?;
    let shots = request.shots.as_deref().map(read_shot_list).transpose()?;
    FileOutputTransaction::validate_target(&request.output, request.overwrite)?;
    let started = Instant::now();
    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Resolving));
    let candidates = ModelSessionCandidates::resolve(context.models, request.model.clone())?;
    let transaction = FileOutputTransaction::new(&request.output, request.overwrite)?;

    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Decoding));
    let stage = Instant::now();
    let mut decoder = SequentialRgbaDecoder::open(&request.input, context.resources.cpu_threads)
        .map_err(|error| {
            ToolError::invalid_input(format!(
                "open interpolation input {}: {error:#}",
                request.input.display()
            ))
        })?;
    let metadata = decoder.metadata();
    if metadata.width % 2 != 0 || metadata.height % 2 != 0 {
        return Err(ToolError::invalid_input(format!(
            "interpolation video must have even display dimensions for H.264 output, got {}x{}",
            metadata.width, metadata.height
        )));
    }
    let first = decoder
        .next_frame()
        .map_err(decode_error)?
        .ok_or_else(|| ToolError::invalid_input("interpolation input contains no video frames"))?;
    let second = decoder.next_frame().map_err(decode_error)?.ok_or_else(|| {
        ToolError::invalid_input("interpolation requires at least two video frames")
    })?;
    let first_pts = first.pts;
    let first_absolute_seconds = first.absolute_seconds(metadata.time_base);
    let second_pts = second
        .pts
        .checked_sub(first_pts)
        .ok_or_else(|| ToolError::invalid_input("interpolation source PTS rebase overflowed"))?;
    let mut timeline = SourceTimeline::derive(
        0,
        Some(second_pts),
        metadata.time_base,
        metadata.nominal_fps,
    )
    .map_err(|error| ToolError::invalid_input(error.to_string()))?;
    timeline
        .observe(0, 0, first.duration_ticks)
        .map_err(|error| ToolError::invalid_input(error.to_string()))?;
    timeline
        .observe(1, second_pts, second.duration_ticks)
        .map_err(|error| ToolError::invalid_input(error.to_string()))?;
    let (source_fps_numerator, source_fps_denominator) = timeline.fps();
    let source_fps = f64::from(source_fps_numerator) / f64::from(source_fps_denominator);
    if f64::from(request.fps) <= source_fps {
        return Err(ToolError::invalid_input(format!(
            "target FPS {} must be greater than source FPS {:.6}",
            request.fps, source_fps
        )));
    }
    let mut decode_seconds = stage.elapsed().as_secs_f64();
    check_cancelled(context)?;

    context.progress.event(ToolEvent::phase(ToolPhase::Loading));
    let stage = Instant::now();
    let opened = candidates.open("load RIFE", |model| {
        InterpolationSession::open(model, context.resources.cpu_threads)
    })?;
    let mut session = opened.session;
    let load_seconds = stage.elapsed().as_secs_f64().max(session.load_seconds());
    let provenance = ModelProvenance::from(&opened.model.resolved);
    let route_warnings = opened.warnings;
    check_cancelled(context)?;

    let mut audio = VideoAudioBridge::open(&request.input, first_absolute_seconds)
        .map_err(|error| output_error("open source audio", error))?;
    let source_had_audio = audio.source_had_audio();
    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Encoding));
    let mut muxer = Muxer::open_timestamped_media_with_transport(
        transaction.staging_path(),
        metadata.width,
        metadata.height,
        crate::codec::TimeBase(1, request.fps as i32),
        request.fps,
        1,
        source_had_audio,
        None,
        false,
        Some(context.resources.cpu_threads),
        VideoFrameTransport::Cpu,
    )
    .map_err(|error| output_error("open interpolation output", error))?;
    audio.attach(&mut muxer);

    let mut left = first;
    let mut right = second;
    let mut source_frames = 2_u64;
    let mut output_frames = 0_u64;
    let mut generated_frames = 0_u64;
    let mut held_cross_shot_frames = 0_u64;
    let mut inference_seconds = 0.0;
    let mut postprocess_seconds = 0.0;
    let mut encode_seconds = 0.0;

    loop {
        check_cancelled(context)?;
        let left_pts = left.pts.checked_sub(first_pts).ok_or_else(|| {
            ToolError::invalid_input("interpolation source PTS rebase overflowed")
        })?;
        let right_pts = right.pts.checked_sub(first_pts).ok_or_else(|| {
            ToolError::invalid_input("interpolation source PTS rebase overflowed")
        })?;
        let left_seconds = pts_seconds(left_pts, metadata.time_base);
        let right_seconds = pts_seconds(right_pts, metadata.time_base);
        let crosses_shot = shots
            .as_ref()
            .is_some_and(|shots| pair_crosses_boundary(shots, left_seconds, right_seconds));

        while f64::from(request.fps) * right_seconds > output_frames as f64 + 1e-9 {
            let output_seconds = output_frames as f64 / f64::from(request.fps);
            let frame = if output_seconds <= left_seconds + 1e-9 {
                left.frame.clone()
            } else if crosses_shot {
                held_cross_shot_frames = held_cross_shot_frames.saturating_add(1);
                left.frame.clone()
            } else {
                let position = ((output_seconds - left_seconds) / (right_seconds - left_seconds))
                    .clamp(0.0, 1.0) as f32;
                context.progress.event(ToolEvent {
                    phase: ToolPhase::Inferencing,
                    completed: Some(generated_frames),
                    total: None,
                    message: None,
                });
                let generated = session
                    .interpolate(&left.frame, &right.frame, position)
                    .map_err(|error| {
                        ToolError::new(
                            ToolErrorCode::InferenceFailed,
                            format!("interpolate output frame {output_frames}: {error:#}"),
                        )
                    })?;
                inference_seconds += generated.inference_seconds;
                postprocess_seconds +=
                    (generated.total_seconds - generated.inference_seconds).max(0.0);
                generated_frames = generated_frames.saturating_add(1);
                generated.frame
            };
            encode_frame(
                &mut muxer,
                &mut audio,
                &frame,
                output_frames,
                request.fps,
                &mut encode_seconds,
            )?;
            output_frames = output_frames.saturating_add(1);
        }

        left = right;
        let stage = Instant::now();
        let next = decoder.next_frame().map_err(decode_error)?;
        decode_seconds += stage.elapsed().as_secs_f64();
        let Some(next) = next else {
            break;
        };
        let next_rebased_pts = next.pts.checked_sub(first_pts).ok_or_else(|| {
            ToolError::invalid_input("interpolation source PTS rebase overflowed")
        })?;
        timeline
            .observe(source_frames, next_rebased_pts, next.duration_ticks)
            .map_err(|error| ToolError::invalid_input(error.to_string()))?;
        right = next;
        source_frames = source_frames.checked_add(1).ok_or_else(|| {
            ToolError::new(ToolErrorCode::Internal, "source frame count overflowed")
        })?;
    }

    let source_duration_seconds = timeline
        .duration_seconds()
        .map_err(|error| ToolError::invalid_input(error.to_string()))?;
    if let Some(shots) = &shots {
        let tolerance = 1.0 / source_fps;
        if (shots.duration_seconds - source_duration_seconds).abs() > tolerance {
            return Err(ToolError::invalid_input(format!(
                "shots duration {:.6}s does not match source duration {:.6}s",
                shots.duration_seconds, source_duration_seconds
            )));
        }
    }
    let wanted_output_frames = (source_duration_seconds * f64::from(request.fps)).round() as u64;
    while output_frames < wanted_output_frames {
        encode_frame(
            &mut muxer,
            &mut audio,
            &left.frame,
            output_frames,
            request.fps,
            &mut encode_seconds,
        )?;
        output_frames += 1;
    }
    let output_duration_seconds = output_frames as f64 / f64::from(request.fps);
    let finish_started = Instant::now();
    muxer
        .finish()
        .map_err(|error| output_error("finish interpolation output", error))?;
    encode_seconds += finish_started.elapsed().as_secs_f64();
    drop(muxer);

    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Validating));
    let staged = probe_av(transaction.staging_path())
        .map_err(|error| output_error("probe staged interpolation output", error))?;
    match staged.video {
        Some((width, height, _)) if width == metadata.width && height == metadata.height => {}
        other => {
            return Err(ToolError::new(
                ToolErrorCode::OutputValidationFailed,
                format!("staged interpolation video geometry drifted: {other:?}"),
            ));
        }
    }
    let staged_audio_duration = match (source_had_audio, staged.audio) {
        (true, Some(duration)) => Some(duration),
        (false, None) => None,
        (true, None) => {
            return Err(ToolError::new(
                ToolErrorCode::OutputValidationFailed,
                "staged interpolation output lost the source audio stream",
            ));
        }
        (false, Some(_)) => {
            return Err(ToolError::new(
                ToolErrorCode::OutputValidationFailed,
                "staged interpolation output invented audio for a video-only source",
            ));
        }
    };
    let audio_tolerance = (1.0 / f64::from(request.fps)).max(1024.0 / 48_000.0) + 1e-3;
    if let Some(staged_audio_duration) = staged_audio_duration
        && (staged_audio_duration - output_duration_seconds).abs() > audio_tolerance
    {
        return Err(ToolError::new(
            ToolErrorCode::OutputValidationFailed,
            format!(
                "staged interpolation audio duration is {:.6}s, expected {output_duration_seconds:.6}s",
                staged_audio_duration
            ),
        ));
    }
    let decoded = decode_video_timing(transaction.staging_path(), context.resources.cpu_threads)
        .map_err(|error| output_error("decode staged interpolation timing", error))?;
    if decoded.frame_count != output_frames || decoded.variable_frame_rate {
        return Err(ToolError::new(
            ToolErrorCode::OutputValidationFailed,
            format!(
                "staged interpolation timeline mismatch: frames={}, expected={output_frames}, vfr={}",
                decoded.frame_count, decoded.variable_frame_rate
            ),
        ));
    }
    if (decoded.duration_seconds - output_duration_seconds).abs()
        > 1.0 / f64::from(request.fps) + 1e-3
    {
        return Err(ToolError::new(
            ToolErrorCode::OutputValidationFailed,
            format!(
                "staged interpolation duration is {:.6}s, expected {output_duration_seconds:.6}s",
                decoded.duration_seconds
            ),
        ));
    }
    check_cancelled(context)?;
    let output = transaction.commit()?;

    let output_artifact = OutputArtifact {
        role: "interpolated_video".to_owned(),
        path: output,
        media_type: "video/mp4".to_owned(),
        summary: Some(
            MediaSummary {
                duration_seconds: Some(output_duration_seconds),
                width: Some(metadata.width),
                height: Some(metadata.height),
                frame_count: Some(output_frames),
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
    let duration_delta = (output_duration_seconds - source_duration_seconds).abs();
    if duration_delta > 1.0 / f64::from(request.fps) / 1000.0 {
        warnings.push(ToolWarning {
            code: "duration_quantized_to_target_fps".to_owned(),
            message: format!(
                "source duration {:.6}s was quantized to the {} fps frame grid as {:.6}s",
                source_duration_seconds, request.fps, output_duration_seconds
            ),
        });
    }
    let total_seconds = started.elapsed().as_secs_f64();
    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Completed));
    Ok(ToolRun {
        result: InterpolateResult {
            outputs: vec![output_artifact.clone()],
            source_frames,
            output_frames,
            generated_frames,
            held_cross_shot_frames,
            source_fps,
            output_fps: request.fps,
            source_duration_seconds,
            output_duration_seconds,
            milliseconds_per_generated_frame: if generated_frames == 0 {
                0.0
            } else {
                inference_seconds * 1_000.0 / generated_frames as f64
            },
        },
        report: ProcessingReport {
            operation: "interpolate".to_owned(),
            parameters: normalized_parameters(&request, source_had_audio),
            input: request.input,
            input_media_type: "video/*".to_owned(),
            input_summary: MediaSummary {
                duration_seconds: Some(source_duration_seconds),
                width: Some(metadata.width),
                height: Some(metadata.height),
                frame_count: Some(source_frames),
                pixel_format: Some("rgba8-straight-display-oriented".to_owned()),
                ..MediaSummary::default()
            },
            outputs: vec![output_artifact],
            models: vec![provenance],
            processed: ProcessedMedia {
                frames: source_frames,
                audio_seconds: if source_had_audio {
                    output_duration_seconds
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

fn encode_frame(
    muxer: &mut Muxer,
    audio: &mut VideoAudioBridge,
    frame: &RgbaFrame,
    index: u64,
    fps: u32,
    encode_seconds: &mut f64,
) -> Result<(), ToolError> {
    let stage = Instant::now();
    let pts = i64::try_from(index).map_err(|_| {
        ToolError::new(
            ToolErrorCode::Internal,
            "interpolation output frame index exceeds the supported PTS range",
        )
    })?;
    muxer
        .encode_video_at_pts_with_duration(frame, pts, Some(1))
        .map_err(|error| output_error("encode interpolated frame", error))?;
    audio
        .pump_to_seconds(muxer, (index + 1) as f64 / f64::from(fps))
        .map_err(|error| output_error("encode source audio", error))?;
    *encode_seconds += stage.elapsed().as_secs_f64();
    Ok(())
}

fn read_shot_list(path: &std::path::Path) -> Result<ShotList, ToolError> {
    let metadata = std::fs::metadata(path).map_err(|error| {
        ToolError::invalid_input(format!("inspect shots JSON {}: {error}", path.display()))
    })?;
    if metadata.len() > MAX_SHOTS_BYTES {
        return Err(ToolError::invalid_input(format!(
            "shots JSON is {} bytes; maximum is {MAX_SHOTS_BYTES}",
            metadata.len()
        )));
    }
    let bytes = std::fs::read(path).map_err(|error| {
        ToolError::invalid_input(format!("read shots JSON {}: {error}", path.display()))
    })?;
    let shots: ShotList = serde_json::from_slice(&bytes).map_err(|error| {
        ToolError::invalid_input(format!("parse shots JSON {}: {error}", path.display()))
    })?;
    shots
        .validate()
        .map_err(|error| ToolError::invalid_input(format!("invalid shots JSON: {error}")))?;
    Ok(shots)
}

fn pair_crosses_boundary(shots: &ShotList, left: f64, right: f64) -> bool {
    shots
        .boundaries
        .iter()
        .any(|boundary| boundary.pts_seconds > left + 1e-6 && boundary.pts_seconds <= right + 1e-6)
}

fn validate_request(request: &InterpolateRequest) -> Result<(), ToolError> {
    if !request.input.is_file() {
        return Err(ToolError::invalid_input(format!(
            "interpolation input does not exist or is not a file: {}",
            request.input.display()
        )));
    }
    if request.fps == 0 || request.fps > 1_000 {
        return Err(ToolError::invalid_input(
            "target FPS must be between 1 and 1000",
        ));
    }
    if request.model.id != MODEL_ID {
        return Err(ToolError::new(
            ToolErrorCode::UnsupportedAdapter,
            format!(
                "interpolate supports model {MODEL_ID:?}, got {:?}",
                request.model.id
            ),
        ));
    }
    if !has_extension(&request.output, "mp4") {
        return Err(ToolError::invalid_input(
            "interpolation video output must end in .mp4",
        ));
    }
    if paths_refer_to_same_file(&request.input, &request.output) {
        return Err(ToolError::invalid_input(
            "interpolation input and output must be different files",
        ));
    }
    if let Some(shots) = &request.shots {
        if !shots.is_file() {
            return Err(ToolError::invalid_input(format!(
                "shots JSON does not exist: {}",
                shots.display()
            )));
        }
        if paths_refer_to_same_file(shots, &request.output)
            || paths_refer_to_same_file(shots, &request.input)
        {
            return Err(ToolError::invalid_input(
                "shots JSON, media input, and media output must be distinct files",
            ));
        }
    }
    Ok(())
}

fn video_warnings(source_had_audio: bool) -> Vec<ToolWarning> {
    let mut warnings = vec![ToolWarning {
        code: "video_color_normalized".to_owned(),
        message: "output video is normalized to display orientation, SAR 1:1, BT.709 limited-range YUV420P"
            .to_owned(),
    }];
    if source_had_audio {
        warnings.push(ToolWarning {
            code: "audio_reencoded_aac".to_owned(),
            message: "source audio was decoded and re-encoded as 48 kHz stereo AAC".to_owned(),
        });
    }
    warnings
}

fn normalized_parameters(
    request: &InterpolateRequest,
    source_had_audio: bool,
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
    parameters.insert("fps".to_owned(), request.fps.into());
    parameters.insert(
        "shots".to_owned(),
        request
            .shots
            .as_ref()
            .map_or(serde_json::Value::Null, |path| {
                path.display().to_string().into()
            }),
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

fn has_extension(path: &std::path::Path, wanted: &str) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case(wanted))
}

fn pts_seconds(pts: i64, time_base: crate::codec::TimeBase) -> f64 {
    pts as f64 * time_base.numerator() as f64 / time_base.denominator() as f64
}

fn decode_error(error: anyhow::Error) -> ToolError {
    ToolError::invalid_input(format!("decode interpolation video: {error:#}"))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shot_boundary_prevents_cross_shot_interpolation() {
        let shots = ShotList::from_boundaries(
            2.0,
            vec![crate::analysis::ShotBoundary {
                frame_index: 30,
                pts_seconds: 1.0,
                confidence: None,
            }],
        )
        .unwrap();
        assert!(pair_crosses_boundary(&shots, 0.9, 1.0));
        assert!(!pair_crosses_boundary(&shots, 1.0, 1.1));
    }

    #[test]
    fn request_rejects_zero_fps_and_in_place_output() {
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("input.mp4");
        std::fs::write(&input, b"fixture").unwrap();
        let request = InterpolateRequest {
            input: input.clone(),
            output: input,
            model: ModelSelection::pinned_default("rife"),
            fps: 0,
            shots: None,
            overwrite: false,
        };
        assert_eq!(
            validate_request(&request).unwrap_err().code,
            ToolErrorCode::InvalidInput
        );

        let request = InterpolateRequest {
            input: request.output,
            output: root.path().join("output.mp4"),
            model: ModelSelection::pinned_default("birefnet"),
            fps: 60,
            shots: None,
            overwrite: false,
        };
        assert_eq!(
            validate_request(&request).unwrap_err().code,
            ToolErrorCode::UnsupportedAdapter
        );
    }
}
