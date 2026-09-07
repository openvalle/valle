//! Complete-file portrait matting for PNG images and sampled video.
//!
//! Valle owns media I/O and publication. BiRefNet/MODNet preprocessing, graph I/O and
//! postprocessing remain in the local model inference modules. One selected session is loaded
//! once and reused for every frame in the job. Video frames are decoded in display orientation
//! with sample-aspect-ratio normalized to square pixels before model inference.

use std::{collections::BTreeMap, path::PathBuf, time::Instant};

use serde::{Deserialize, Serialize};

use crate::{
    codec::{
        LibavVideoSource, TransparentVideoMuxer, VideoSource, probe_av, read_rgba_png,
        write_rgba_png,
    },
    frame::RgbaFrame,
    models::{
        ModelSelection,
        adapters::birefnet::{BIREFNET_MODEL_ID, MODNET_MODEL_ID, MatteSession},
    },
};

use super::{
    AlphaMode, CancellationToken, MediaSummary, ModelProvenance, OutputArtifact, ProcessedMedia,
    ProcessingReport, RunContext, StageTiming, TimeRange, ToolError, ToolErrorCode, ToolEvent,
    ToolPhase, ToolRun, ToolWarning,
    model_session::ModelSessionCandidates,
    output::{FileOutputTransaction, paths_refer_to_same_file},
    video_sequence::SequentialRgbaDecoder,
};

const DEFAULT_SAMPLE_FPS: f64 = 5.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MatteRequest {
    pub input: PathBuf,
    pub output: PathBuf,
    pub model: ModelSelection,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<TimeRange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample_fps: Option<f64>,
    pub overwrite: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MatteResult {
    pub outputs: Vec<OutputArtifact>,
    pub frames: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fps: Option<f64>,
    pub width: u32,
    pub height: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range_start: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range_end: Option<f64>,
    pub milliseconds_per_frame: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InputKind {
    Png,
    Video,
}

pub fn run(
    request: MatteRequest,
    context: &mut RunContext<'_>,
) -> Result<ToolRun<MatteResult>, ToolError> {
    context.validate()?;
    let (kind, fps) = validate_request(&request)?;
    FileOutputTransaction::validate_target(&request.output, request.overwrite)?;
    let started = Instant::now();

    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Resolving));
    check_cancelled(context)?;
    let candidates = ModelSessionCandidates::resolve(context.models, request.model.clone())?;
    let transaction = FileOutputTransaction::new(&request.output, request.overwrite)?;
    context.progress.event(ToolEvent::phase(ToolPhase::Loading));
    let load_started = Instant::now();
    let opened = candidates.open("load matte model", |model| {
        MatteSession::open(
            model,
            &context.models.compiled_coreml_root(),
            context.resources.cpu_threads,
        )
    })?;
    let mut session = opened.session;
    let load_seconds = load_started
        .elapsed()
        .as_secs_f64()
        .max(session.load_seconds());
    let provenance = ModelProvenance::from(&opened.model.resolved);
    let warnings = opened.warnings;
    check_cancelled(context)?;

    match kind {
        InputKind::Png => run_png(
            request,
            transaction,
            started,
            load_seconds,
            provenance,
            warnings,
            &mut session,
            context,
        ),
        InputKind::Video => run_video(
            request,
            transaction,
            started,
            load_seconds,
            provenance,
            warnings,
            fps,
            &mut session,
            context,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn run_png(
    request: MatteRequest,
    transaction: FileOutputTransaction,
    started: Instant,
    load_seconds: f64,
    provenance: ModelProvenance,
    warnings: Vec<ToolWarning>,
    session: &mut MatteSession,
    context: &mut RunContext<'_>,
) -> Result<ToolRun<MatteResult>, ToolError> {
    context
        .progress
        .event(ToolEvent::count(ToolPhase::Decoding, 0, 1));
    let stage = Instant::now();
    let source = read_rgba_png(&request.input).map_err(|error| {
        ToolError::invalid_input(format!(
            "decode matte input PNG {}: {error:#}",
            request.input.display()
        ))
    })?;
    let decode_seconds = stage.elapsed().as_secs_f64();
    check_cancelled(context)?;

    context
        .progress
        .event(ToolEvent::count(ToolPhase::Inferencing, 0, 1));
    let matte = session
        .alpha(source.width, source.height, &source.data)
        .map_err(|error| inference_error("matte PNG", error))?;
    let stage = Instant::now();
    let foreground = apply_alpha(&source, &matte.alpha)?;
    let alpha_seconds = stage.elapsed().as_secs_f64();
    check_cancelled(context)?;

    context
        .progress
        .event(ToolEvent::count(ToolPhase::Encoding, 0, 1));
    let stage = Instant::now();
    write_rgba_png(transaction.staging_path(), &foreground).map_err(|error| {
        output_error(format!(
            "write staged matte PNG {}: {error:#}",
            transaction.staging_path().display()
        ))
    })?;
    let round_trip = read_rgba_png(transaction.staging_path())
        .map_err(|error| output_error(format!("validate staged matte PNG: {error:#}")))?;
    if round_trip != foreground {
        return Err(output_error(
            "staged matte PNG did not round-trip exactly".to_owned(),
        ));
    }
    let encode_seconds = stage.elapsed().as_secs_f64();

    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Validating));
    check_cancelled(context)?;
    let output = transaction.commit()?;
    let artifact = OutputArtifact {
        role: "transparent_foreground".to_owned(),
        path: output,
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
    };
    let total_seconds = started.elapsed().as_secs_f64();
    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Completed));
    Ok(ToolRun {
        result: MatteResult {
            outputs: vec![artifact.clone()],
            frames: 1,
            fps: None,
            width: source.width,
            height: source.height,
            range_start: None,
            range_end: None,
            milliseconds_per_frame: total_seconds * 1_000.0,
        },
        report: ProcessingReport {
            operation: "matte".to_owned(),
            parameters: normalized_parameters(&request, None),
            input: request.input,
            input_media_type: "image/png".to_owned(),
            input_summary: MediaSummary {
                width: Some(source.width),
                height: Some(source.height),
                frame_count: Some(1),
                pixel_format: Some("rgba8-straight".to_owned()),
                ..MediaSummary::default()
            },
            outputs: vec![artifact],
            models: vec![provenance],
            processed: ProcessedMedia {
                frames: 1,
                audio_seconds: 0.0,
            },
            timing: StageTiming {
                load_seconds,
                decode_seconds,
                preprocess_seconds: matte.preprocess_seconds,
                inference_seconds: matte.inference_seconds,
                postprocess_seconds: matte.postprocess_seconds + alpha_seconds,
                encode_seconds,
                total_seconds,
            },
        },
        warnings,
    })
}

#[allow(clippy::too_many_arguments)]
fn run_video(
    request: MatteRequest,
    transaction: FileOutputTransaction,
    started: Instant,
    load_seconds: f64,
    provenance: ModelProvenance,
    warnings: Vec<ToolWarning>,
    fps: f64,
    session: &mut MatteSession,
    context: &mut RunContext<'_>,
) -> Result<ToolRun<MatteResult>, ToolError> {
    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Decoding));
    let stage = Instant::now();
    let mut source =
        LibavVideoSource::open_with_threads(&request.input, context.resources.cpu_threads)
            .map_err(|error| {
                ToolError::invalid_input(format!("open {}: {error:#}", request.input.display()))
            })?;
    let (width, height, source_duration) = {
        let metadata = source.meta();
        (metadata.width, metadata.height, metadata.duration_s)
    };
    let mut decode_seconds = stage.elapsed().as_secs_f64();
    if width == 0 || height == 0 {
        return Err(ToolError::invalid_input("source has no video dimensions"));
    }
    let range_start = request.range.map(|range| range.start).unwrap_or(0.0);
    let range_end = requested_range_end(request.range, source_duration)?;
    if let Some(duration) = source_duration
        && range_end > duration + 1.0e-6
    {
        return Err(ToolError::invalid_input(format!(
            "matte range end {range_end:.6}s exceeds source duration {duration:.6}s"
        )));
    }
    let frames = sample_count(range_end - range_start, fps)?;
    let (fps_numerator, fps_denominator) = fps_rational(fps)?;
    let mut muxer = TransparentVideoMuxer::open(
        transaction.staging_path(),
        width,
        height,
        fps_numerator,
        fps_denominator,
    )
    .map_err(|error| output_error(format!("create staged matte MOV: {error:#}")))?;

    let mut preprocess_seconds = 0.0;
    let mut inference_seconds = 0.0;
    let mut postprocess_seconds = 0.0;
    let mut encode_seconds = 0.0;
    for index in 0..frames {
        check_cancelled(context)?;
        let time = range_start + index as f64 / fps;
        context
            .progress
            .event(ToolEvent::count(ToolPhase::Decoding, index, frames));
        let stage = Instant::now();
        let frame = source.frame_at(time).map_err(|error| {
            ToolError::invalid_input(format!("decode matte frame at {time:.6}s: {error:#}"))
        })?;
        decode_seconds += stage.elapsed().as_secs_f64();

        context
            .progress
            .event(ToolEvent::count(ToolPhase::Inferencing, index, frames));
        let matte = session
            .alpha(frame.width, frame.height, &frame.data)
            .map_err(|error| inference_error(&format!("matte video frame {index}"), error))?;
        preprocess_seconds += matte.preprocess_seconds;
        inference_seconds += matte.inference_seconds;
        postprocess_seconds += matte.postprocess_seconds;

        let stage = Instant::now();
        let foreground = apply_alpha(&frame, &matte.alpha)?;
        postprocess_seconds += stage.elapsed().as_secs_f64();
        context
            .progress
            .event(ToolEvent::count(ToolPhase::Encoding, index, frames));
        let stage = Instant::now();
        muxer.encode_video(&foreground).map_err(|error| {
            output_error(format!("encode matte video frame {index}: {error:#}"))
        })?;
        encode_seconds += stage.elapsed().as_secs_f64();
    }

    let stage = Instant::now();
    muxer
        .finish()
        .map_err(|error| output_error(format!("finish staged matte MOV: {error:#}")))?;
    encode_seconds += stage.elapsed().as_secs_f64();
    drop(muxer);

    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Validating));
    check_cancelled(context)?;
    let output_duration = frames as f64 / fps;
    validate_video_output(
        transaction.staging_path(),
        VideoOutputContract {
            width,
            height,
            frames,
            duration: output_duration,
            fps,
        },
        context.resources.cpu_threads,
        &context.cancellation,
    )?;
    let output = transaction.commit()?;
    let artifact = OutputArtifact {
        role: "transparent_foreground".to_owned(),
        path: output,
        media_type: "video/quicktime".to_owned(),
        summary: Some(
            MediaSummary {
                duration_seconds: Some(output_duration),
                width: Some(width),
                height: Some(height),
                frame_count: Some(frames),
                ..MediaSummary::default()
            }
            .with_video_encoding("mov", "qtrle", None, "argb", AlphaMode::Straight),
        ),
    };
    let total_seconds = started.elapsed().as_secs_f64();
    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Completed));
    Ok(ToolRun {
        result: MatteResult {
            outputs: vec![artifact.clone()],
            frames,
            fps: Some(fps),
            width,
            height,
            range_start: Some(range_start),
            range_end: Some(range_end),
            milliseconds_per_frame: total_seconds * 1_000.0 / frames as f64,
        },
        report: ProcessingReport {
            operation: "matte".to_owned(),
            parameters: normalized_parameters(&request, Some(fps)),
            input: request.input,
            input_media_type: "video/*".to_owned(),
            input_summary: MediaSummary {
                duration_seconds: source_duration,
                width: Some(width),
                height: Some(height),
                ..MediaSummary::default()
            },
            outputs: vec![artifact],
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

fn validate_request(request: &MatteRequest) -> Result<(InputKind, f64), ToolError> {
    if !request.input.is_file() {
        return Err(ToolError::invalid_input(format!(
            "input does not exist or is not a file: {}",
            request.input.display()
        )));
    }
    if !matches!(
        request.model.id.as_str(),
        BIREFNET_MODEL_ID | MODNET_MODEL_ID
    ) {
        return Err(ToolError::new(
            ToolErrorCode::UnsupportedAdapter,
            format!(
                "matte supports model {BIREFNET_MODEL_ID:?} or {MODNET_MODEL_ID:?}, got {:?}",
                request.model.id
            ),
        ));
    }
    if paths_refer_to_same_file(&request.input, &request.output) {
        return Err(ToolError::invalid_input(
            "matte input and output must be different files",
        ));
    }
    let kind = if has_extension(&request.input, "png") {
        InputKind::Png
    } else {
        InputKind::Video
    };
    match kind {
        InputKind::Png if !has_extension(&request.output, "png") => {
            return Err(ToolError::invalid_input(
                "PNG matte input requires a .png transparent foreground output",
            ));
        }
        InputKind::Png if request.range.is_some() => {
            return Err(ToolError::invalid_input(
                "--range is only valid for video matte input",
            ));
        }
        InputKind::Video if !has_extension(&request.output, "mov") => {
            return Err(ToolError::invalid_input(
                "video matte output must end in .mov (lossless qtrle RGBA)",
            ));
        }
        _ => {}
    }
    let fps = request.sample_fps.unwrap_or(DEFAULT_SAMPLE_FPS);
    if !fps.is_finite() || fps <= 0.0 {
        return Err(ToolError::invalid_input(
            "sample_fps must be finite and greater than zero",
        ));
    }
    Ok((kind, fps))
}

fn requested_range_end(
    range: Option<TimeRange>,
    source_duration: Option<f64>,
) -> Result<f64, ToolError> {
    let start = range.map(|range| range.start).unwrap_or(0.0);
    let end = range
        .and_then(|range| range.end)
        .or(source_duration)
        .ok_or_else(|| {
            ToolError::invalid_input(
                "video duration is unavailable; provide an explicit --range start,end",
            )
        })?;
    if end <= start {
        return Err(ToolError::invalid_input(format!(
            "range end must be greater than start (got {start}..{end})"
        )));
    }
    Ok(end)
}

fn normalized_parameters(
    request: &MatteRequest,
    fps: Option<f64>,
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
        "sampleFps".to_owned(),
        fps.map_or(serde_json::Value::Null, serde_json::Value::from),
    );
    parameters.insert(
        "range".to_owned(),
        request.range.map_or(serde_json::Value::Null, |range| {
            serde_json::to_value(range).expect("time range is serializable")
        }),
    );
    parameters
}

fn apply_alpha(source: &RgbaFrame, alpha: &[u8]) -> Result<RgbaFrame, ToolError> {
    if alpha.len() != source.pixel_count() {
        return Err(ToolError::new(
            ToolErrorCode::InferenceFailed,
            format!(
                "matte returned {} alpha samples for {} pixels",
                alpha.len(),
                source.pixel_count()
            ),
        ));
    }
    let mut output = source.clone();
    for (pixel, matte) in output.data.chunks_exact_mut(4).zip(alpha) {
        pixel[3] = ((u16::from(pixel[3]) * u16::from(*matte) + 127) / 255) as u8;
    }
    Ok(output)
}

#[derive(Debug, Clone, Copy)]
struct VideoOutputContract {
    width: u32,
    height: u32,
    frames: u64,
    duration: f64,
    fps: f64,
}

fn validate_video_output(
    path: &std::path::Path,
    expected: VideoOutputContract,
    cpu_threads: usize,
    cancellation: &CancellationToken,
) -> Result<(), ToolError> {
    let probe = probe_av(path)
        .map_err(|error| output_error(format!("probe staged matte MOV: {error:#}")))?;
    let Some((actual_width, actual_height, duration)) = probe.video else {
        return Err(output_error(
            "staged matte MOV has no video stream".to_owned(),
        ));
    };
    if probe.audio.is_some() || (actual_width, actual_height) != (expected.width, expected.height) {
        return Err(output_error(format!(
            "staged matte MOV contract mismatch: video={actual_width}x{actual_height}, audio={}",
            probe.audio.is_some()
        )));
    }
    let tolerance = (1.0 / expected.fps).max(1.0e-3);
    if duration > 0.0 && (duration - expected.duration).abs() > tolerance {
        return Err(output_error(format!(
            "staged matte MOV duration {duration:.6}s differs from expected {:.6}s",
            expected.duration
        )));
    }
    let mut decoder = SequentialRgbaDecoder::open(path, cpu_threads)
        .map_err(|error| output_error(format!("open staged matte MOV decoder: {error:#}")))?;
    let time_base = decoder.metadata().time_base;
    let timestamp_tolerance =
        (f64::from(time_base.numerator()) / f64::from(time_base.denominator())).abs() + 1.0e-9;
    let mut decoded_frames = 0_u64;
    loop {
        if cancellation.is_cancelled() {
            return Err(ToolError::cancelled());
        }
        let Some(frame) = decoder
            .next_frame()
            .map_err(|error| output_error(format!("decode staged matte MOV: {error:#}")))?
        else {
            break;
        };
        let actual_seconds = frame.absolute_seconds(time_base);
        let expected_seconds = decoded_frames as f64 / expected.fps;
        if (actual_seconds - expected_seconds).abs() > timestamp_tolerance {
            return Err(output_error(format!(
                "staged matte MOV frame {} starts at {actual_seconds:.6}s, expected {expected_seconds:.6}s",
                decoded_frames + 1
            )));
        }
        decoded_frames = decoded_frames.checked_add(1).ok_or_else(|| {
            output_error("staged matte MOV decoded frame count overflow".to_owned())
        })?;
        if (frame.frame.width, frame.frame.height) != (expected.width, expected.height) {
            return Err(output_error(format!(
                "staged matte MOV frame {decoded_frames} has dimensions {}x{}, expected {}x{}",
                frame.frame.width, frame.frame.height, expected.width, expected.height
            )));
        }
        if decoded_frames > expected.frames {
            return Err(output_error(format!(
                "staged matte MOV contains more than the expected {} frames",
                expected.frames
            )));
        }
    }
    if decoded_frames != expected.frames {
        return Err(output_error(format!(
            "staged matte MOV decoded {decoded_frames} frames, expected {}",
            expected.frames
        )));
    }
    Ok(())
}

fn fps_rational(fps: f64) -> Result<(u32, u32), ToolError> {
    const SCALE: u32 = 1_000_000;
    const MAX_FPS: f64 = i32::MAX as f64 / SCALE as f64;
    if fps > MAX_FPS {
        return Err(ToolError::invalid_input(format!(
            "sample_fps is too large (maximum {MAX_FPS:.6})"
        )));
    }
    let numerator = (fps * f64::from(SCALE)).round();
    if numerator < 1.0 || numerator > i32::MAX as f64 {
        return Err(ToolError::invalid_input(
            "sample_fps is outside the supported six-decimal rational range",
        ));
    }
    let numerator = numerator as u32;
    let divisor = gcd(numerator, SCALE);
    Ok((numerator / divisor, SCALE / divisor))
}

fn gcd(mut left: u32, mut right: u32) -> u32 {
    while right != 0 {
        (left, right) = (right, left % right);
    }
    left
}

fn sample_count(duration_seconds: f64, fps: f64) -> Result<u64, ToolError> {
    let count = (duration_seconds * fps).ceil();
    if !count.is_finite() || count < 1.0 || count > u64::MAX as f64 {
        return Err(ToolError::invalid_input(
            "matte sample count is outside the supported range",
        ));
    }
    Ok(count as u64)
}

fn inference_error(operation: &str, error: anyhow::Error) -> ToolError {
    ToolError::new(
        ToolErrorCode::InferenceFailed,
        format!("{operation}: {error:#}"),
    )
}

fn output_error(message: String) -> ToolError {
    ToolError::new(ToolErrorCode::OutputValidationFailed, message)
}

fn has_extension(path: &std::path::Path, expected: &str) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case(expected))
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
    use crate::models::RunBackendPreference;

    fn request(root: &std::path::Path, input_name: &str, output_name: &str) -> MatteRequest {
        let input = root.join(input_name);
        std::fs::write(&input, b"validation fixture").unwrap();
        MatteRequest {
            input,
            output: root.join(output_name),
            model: ModelSelection {
                id: BIREFNET_MODEL_ID.to_owned(),
                version: None,
                backend: RunBackendPreference::Auto,
            },
            range: None,
            sample_fps: None,
            overwrite: false,
        }
    }

    #[test]
    fn alpha_is_multiplied_without_replacing_rgb() {
        let source = RgbaFrame {
            width: 2,
            height: 1,
            data: vec![10, 20, 30, 255, 40, 50, 60, 128],
        };
        let output = apply_alpha(&source, &[0, 128]).unwrap();
        assert_eq!(output.data, vec![10, 20, 30, 0, 40, 50, 60, 64]);
    }

    #[test]
    fn fractional_fps_is_preserved() {
        assert_eq!(fps_rational(5.0).unwrap(), (5, 1));
        assert_eq!(fps_rational(2.5).unwrap(), (5, 2));
        assert_eq!(fps_rational(29.97).unwrap(), (2997, 100));
    }

    #[test]
    fn sample_count_covers_a_half_open_interval() {
        assert_eq!(sample_count(1.0, 2.0).unwrap(), 2);
        assert_eq!(sample_count(1.0, 2.1).unwrap(), 3);
        assert_eq!(sample_count(0.1, 2.0).unwrap(), 1);
    }

    #[test]
    fn image_and_video_require_their_declared_output_types() {
        let root = tempfile::tempdir().unwrap();
        assert!(validate_request(&request(root.path(), "input.png", "output.png")).is_ok());
        assert!(validate_request(&request(root.path(), "input.png", "output.mov")).is_err());
        assert!(validate_request(&request(root.path(), "input.mp4", "output.mov")).is_ok());
        assert!(validate_request(&request(root.path(), "input.mp4", "output.png")).is_err());
    }

    #[test]
    fn explicit_alternative_model_is_supported_without_cross_model_fallback() {
        let root = tempfile::tempdir().unwrap();
        let mut request = request(root.path(), "input.png", "output.png");
        request.model.id = MODNET_MODEL_ID.to_owned();
        assert!(validate_request(&request).is_ok());
        request.model.id = "unknown".to_owned();
        assert_eq!(
            validate_request(&request).unwrap_err().code,
            ToolErrorCode::UnsupportedAdapter
        );
    }

    #[test]
    fn request_rejects_output_that_aliases_input() {
        let root = tempfile::tempdir().unwrap();
        let mut request = request(root.path(), "input.mov", "output.mov");
        request.output = request.input.clone();
        assert_eq!(
            validate_request(&request).unwrap_err().code,
            ToolErrorCode::InvalidInput
        );
    }

    #[test]
    fn staged_video_validation_decodes_the_complete_stream() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("foreground.mov");
        let mut muxer = TransparentVideoMuxer::open(&path, 4, 2, 1, 1).unwrap();
        for value in [16, 96, 224] {
            let mut frame = RgbaFrame::new(4, 2);
            frame.fill([value, value / 2, 255 - value, value]);
            muxer.encode_video(&frame).unwrap();
        }
        muxer.finish().unwrap();
        drop(muxer);

        let cancellation = CancellationToken::new();
        let expected = VideoOutputContract {
            width: 4,
            height: 2,
            frames: 3,
            duration: 3.0,
            fps: 1.0,
        };
        validate_video_output(&path, expected, 1, &cancellation).unwrap();

        let error = validate_video_output(
            &path,
            VideoOutputContract {
                frames: 4,
                ..expected
            },
            1,
            &cancellation,
        )
        .expect_err("a truncated staged stream must not be committed");
        assert_eq!(error.code, ToolErrorCode::OutputValidationFailed);
        assert!(error.message.contains("decoded 3 frames, expected 4"));
    }
}
