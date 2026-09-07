//! Public Timeline WASM construction helpers.
//!
//! TypeScript may hold generated DTOs, but it never implements Valle JSON,
//! exact compilation, or document hashing. These functions call the same Rust
//! constructors used by Native persistence and Engine admission.

use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use valle_timeline::internal::{
    quantize::{quantize_frame_boundary, quantize_sample_boundary},
    wire::document::{
        AudioItemWire, AudioSourceWire, CaptionItemWire, MotionCueBindingWire, TimedAdjustmentWire,
        TimelineDocumentWire, VisualItemWire, VisualSourceWire,
    },
};
use valle_timeline::{
    ExactRational, FrameRate, RationalTime,
    wire::timeline::{TimelineFrameRateWire, TimelineTimeWire},
};
use wasm_bindgen::prelude::*;

/// Lossy, read-only values for browser transport and screen geometry. The
/// public Timeline remains the sole working-copy truth.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TimelineDocumentView {
    canvas: TimelineCanvasView,
    sequences: Vec<TimelineSequenceView>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TimelineCanvasView {
    width: u32,
    height: u32,
    duration_seconds: f64,
    frames_per_second: f64,
    frame_count: i64,
    sample_rate: u32,
    sample_count: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TimelineSequenceView {
    band: &'static str,
    track_id: String,
    track_index: usize,
    duration_seconds: f64,
    items: Vec<TimelineSequenceItemView>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TimelineSequenceItemView {
    item_id: String,
    timeline_path: Option<String>,
    item_index: usize,
    start_seconds: f64,
    duration_seconds: f64,
    end_seconds: f64,
    start_frame: i64,
    duration_frames: i64,
    end_frame: i64,
    advances_cursor: bool,
    source_start_seconds: Option<f64>,
    source_start_frame: Option<i64>,
    source_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    motion_frames: Option<MotionAuthoringFramesView>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MotionAuthoringFramesView {
    source_duration_frames: i64,
    enter_frames: Option<i64>,
    exit_frames: Option<i64>,
    cues: BTreeMap<String, MotionCueFramesView>,
}

#[derive(Debug, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum MotionCueFramesView {
    SourceRange {
        start_frame: i64,
        end_frame: i64,
        enter_frames: i64,
        exit_frames: i64,
    },
}

/// Compile one sparse public Timeline into the expanded canonical document
/// consumed by the browser renderer. The returned document is a derived view;
/// Studio must keep and save the public Timeline as its working copy.
#[wasm_bindgen]
pub fn compile_timeline(json: &str) -> Result<String, JsError> {
    compile_timeline_native(json).map_err(|error| JsError::new(&error))
}

/// Normalize one sparse public Timeline with the exact same Rust admission
/// path used by Project persistence, then return canonical valle-json bytes.
#[wasm_bindgen]
pub fn normalize_timeline(json: &str) -> Result<String, JsError> {
    normalize_timeline_native(json).map_err(|error| JsError::new(&error))
}

/// Convert an integral frame count to public Timeline seconds using the exact
/// frame-rate rational and the canonical q6 half-away rule.
#[wasm_bindgen]
pub fn timeline_time_from_frames(frames: f64, fps_json: &str) -> Result<f64, JsError> {
    timeline_time_from_frames_native(frames, fps_json).map_err(|error| JsError::new(&error))
}

/// Convert an integral composition-frame delta to source seconds after applying
/// the clip's exact playback rate, then canonicalize the result with q6
/// round-half-away semantics.
#[wasm_bindgen]
pub fn timeline_source_time_delta_from_frames(
    frames: f64,
    fps_json: &str,
    rate_json: &str,
) -> Result<f64, JsError> {
    timeline_source_time_delta_from_frames_native(frames, fps_json, rate_json)
        .map_err(|error| JsError::new(&error))
}

/// Strictly decode a complete `document@2.0` envelope and return its JCS
/// canonical spelling. Invalid shape, local invariants, duplicate keys, or
/// unsupported contracts fail before any value is returned.
#[wasm_bindgen]
pub fn canonicalize_timeline_document(json: &str) -> Result<String, JsError> {
    canonicalize_timeline_document_native(json).map_err(|error| JsError::new(&error))
}

/// Decode a canonical document and derive browser display/playback geometry
/// with the same exact Sequence and quantization rules used by Native. This is
/// a view only: it cannot be submitted or hashed as a Timeline document.
#[wasm_bindgen]
pub fn timeline_document_view(json: &str) -> Result<String, JsError> {
    timeline_document_view_native(json).map_err(|error| JsError::new(&error))
}

fn canonicalize_timeline_document_native(json: &str) -> Result<String, String> {
    let timeline = valle_timeline::internal::decode_canonical(json)
        .map_err(|error| timeline_error("timeline_document_decode", error))?;
    valle_timeline::internal::encode_canonical(&timeline)
        .map_err(|error| timeline_error("timeline_document_encode", error))
}

fn compile_timeline_native(json: &str) -> Result<String, String> {
    let timeline = valle_timeline::decode_timeline(json)
        .map_err(|error| timeline_error("timeline_decode", error))?;
    let timeline = valle_compiler::compile_timeline(timeline)
        .map_err(|error| timeline_error("timeline_compile", error))?;
    valle_timeline::internal::encode_canonical(&timeline)
        .map_err(|error| timeline_error("timeline_document_encode", error))
}

fn normalize_timeline_native(json: &str) -> Result<String, String> {
    let timeline = valle_timeline::decode_timeline(json)
        .map_err(|error| timeline_error("timeline_decode", error))?;
    let bytes = valle_timeline::timeline_bytes(&timeline)
        .map_err(|error| timeline_error("timeline_encode", error))?;
    String::from_utf8(bytes).map_err(|error| timeline_error("timeline_encode", error))
}

fn timeline_time_from_frames_native(frames: f64, fps_json: &str) -> Result<f64, String> {
    timeline_delta_from_frames_native(
        "timeline_time_from_frames",
        frames,
        fps_json,
        ExactRational::ONE,
    )
}

fn timeline_source_time_delta_from_frames_native(
    frames: f64,
    fps_json: &str,
    rate_json: &str,
) -> Result<f64, String> {
    let operation = "timeline_source_time_delta_from_frames";
    let rate: TimelineTimeWire =
        serde_json::from_str(rate_json).map_err(|error| timeline_error(operation, error))?;
    let rate = rate.to_exact();
    if !rate.is_positive() {
        return Err(format!("[{operation}] rate must be positive"));
    }
    timeline_delta_from_frames_native(operation, frames, fps_json, rate)
}

fn timeline_delta_from_frames_native(
    operation: &str,
    frames: f64,
    fps_json: &str,
    rate: ExactRational,
) -> Result<f64, String> {
    const Q6_SCALE: i128 = 1_000_000;
    const MAX_SAFE_INTEGER: i128 = 9_007_199_254_740_991;

    if !frames.is_finite() || frames.fract() != 0.0 || frames.abs() > MAX_SAFE_INTEGER as f64 {
        return Err(format!("[{operation}] frames must be a safe integer"));
    }
    let fps_wire: TimelineFrameRateWire =
        serde_json::from_str(fps_json).map_err(|error| timeline_error(operation, error))?;
    let fps = fps_wire
        .try_to_frame_rate()
        .map_err(|error| timeline_error(operation, error))?;
    let scaled = (frames as i128)
        .checked_mul(i128::from(fps.denominator()))
        .and_then(|value| value.checked_mul(i128::from(rate.numerator())))
        .and_then(|value| value.checked_mul(Q6_SCALE))
        .ok_or_else(|| format!("[{operation}] time overflow"))?;
    let divisor = u128::try_from(fps.numerator())
        .ok()
        .and_then(|value| value.checked_mul(u128::from(rate.denominator())))
        .ok_or_else(|| format!("[{operation}] time overflow"))?;
    let magnitude = scaled.unsigned_abs();
    let quotient = magnitude / divisor;
    let remainder = magnitude % divisor;
    let rounded = quotient
        .checked_add(u128::from(remainder.saturating_mul(2) >= divisor))
        .ok_or_else(|| format!("[{operation}] time overflow"))?;
    if rounded > MAX_SAFE_INTEGER as u128 {
        return Err(format!(
            "[{operation}] q6 result exceeds JavaScript safe range"
        ));
    }
    let micros = if scaled < 0 {
        -(rounded as i128)
    } else {
        rounded as i128
    };
    if micros == 0 {
        Ok(0.0)
    } else {
        Ok(micros as f64 / Q6_SCALE as f64)
    }
}

fn timeline_document_view_native(json: &str) -> Result<String, String> {
    let timeline = valle_timeline::internal::decode_canonical(json)
        .map_err(|error| timeline_error("timeline_document_decode", error))?;
    // The view is a transport projection of the generated wire shape. Domain types remain the
    // canonical in-memory truth; this owned conversion avoids exposing or mutating them in JS.
    let wire = timeline.to_wire();
    let document = &wire.document;
    let duration = RationalTime::from_exact(document.canvas.duration);
    let frame_rate = valle_timeline::FrameRate::from_exact(document.canvas.fps)
        .map_err(|error| timeline_error("timeline_document_view", error))?;
    let frame_count = quantize_frame_boundary(duration, frame_rate)
        .map_err(|error| timeline_error("timeline_document_view", error))?;
    let sample_count = quantize_sample_boundary(duration, document.canvas.sample_rate)
        .map_err(|error| timeline_error("timeline_document_view", error))?;

    let mut sequences = Vec::new();
    for (track_index, track) in document.visual.tracks.iter().enumerate() {
        let compiler_track = compiler_track_id("visual", track_index);
        let mut sequence = project_sequence(
            "visual",
            track.id.clone(),
            track_index,
            frame_rate,
            track.items.iter().enumerate().map(|(item_index, item)| {
                let (id, duration, advances, source_clock, timeline_path) = match item {
                    VisualItemWire::Clip(value) => {
                        let source_clock = match &value.source {
                            VisualSourceWire::Video(source) => {
                                Some((source.source_start, source.rate))
                            }
                            VisualSourceWire::Lottie(source) => {
                                Some((source.source_start, source.rate))
                            }
                            VisualSourceWire::Motion(source) => {
                                Some((source.source_start, source.rate))
                            }
                            VisualSourceWire::Image(_) | VisualSourceWire::Solid(_) => None,
                        };
                        let timeline_path = (track.id == compiler_track)
                            .then(|| compiler_timeline_path(&value.id, "visual", track_index))
                            .flatten();
                        (&value.id, value.duration, true, source_clock, timeline_path)
                    }
                    VisualItemWire::Gap(value) => (&value.id, value.duration, true, None, None),
                    VisualItemWire::Transition(value) => {
                        (&value.id, value.duration, false, None, None)
                    }
                };
                (
                    item_index,
                    id.clone(),
                    timeline_path,
                    duration,
                    advances,
                    source_clock,
                )
            }),
        )?;
        for (item, view) in track.items.iter().zip(&mut sequence.items) {
            if let VisualItemWire::Clip(clip) = item
                && let VisualSourceWire::Motion(source) = &clip.source
            {
                view.motion_frames = Some(project_motion_frames(source, frame_rate)?);
            }
        }
        sequences.push(sequence);
    }
    for (track_index, track) in document.audio.tracks.iter().enumerate() {
        let compiler_track = compiler_track_id("audio", track_index);
        sequences.push(project_sequence(
            "audio",
            track.id.clone(),
            track_index,
            frame_rate,
            track.items.iter().enumerate().map(|(item_index, item)| {
                let (id, duration, advances, source_clock, timeline_path) = match item {
                    AudioItemWire::Clip(value) => {
                        let AudioSourceWire::Media(source) = &value.source;
                        let timeline_path = (track.id == compiler_track)
                            .then(|| compiler_timeline_path(&value.id, "audio", track_index))
                            .flatten();
                        (
                            &value.id,
                            value.duration,
                            true,
                            Some((source.source_start, source.rate)),
                            timeline_path,
                        )
                    }
                    AudioItemWire::Gap(value) => (&value.id, value.duration, true, None, None),
                    AudioItemWire::Crossfade(value) => {
                        (&value.id, value.duration, false, None, None)
                    }
                };
                (
                    item_index,
                    id.clone(),
                    timeline_path,
                    duration,
                    advances,
                    source_clock,
                )
            }),
        )?);
    }
    for (track_index, track) in document.captions.tracks.iter().enumerate() {
        let compiler_track = compiler_track_id("caption", track_index);
        sequences.push(project_sequence(
            "caption",
            track.id.clone(),
            track_index,
            frame_rate,
            track.items.iter().enumerate().map(|(item_index, item)| {
                let (id, duration, timeline_path) = match item {
                    CaptionItemWire::Clip(value) => {
                        let timeline_path = (track.id == compiler_track)
                            .then(|| compiler_timeline_path(&value.id, "caption", track_index))
                            .flatten();
                        (&value.id, value.duration, timeline_path)
                    }
                    CaptionItemWire::Gap(value) => (&value.id, value.duration, None),
                };
                (item_index, id.clone(), timeline_path, duration, true, None)
            }),
        )?);
    }
    sequences.extend(project_adjustment_sequences(document, frame_rate)?);

    serde_json::to_string(&TimelineDocumentView {
        canvas: TimelineCanvasView {
            width: document.canvas.width,
            height: document.canvas.height,
            duration_seconds: duration.as_f64(),
            frames_per_second: document.canvas.fps.as_f64(),
            frame_count,
            sample_rate: document.canvas.sample_rate,
            sample_count,
        },
        sequences,
    })
    .map_err(|error| timeline_error("timeline_document_view", error))
}

fn project_sequence<I>(
    band: &'static str,
    track_id: String,
    track_index: usize,
    frame_rate: FrameRate,
    items: I,
) -> Result<TimelineSequenceView, String>
where
    I: IntoIterator<
        Item = (
            usize,
            String,
            Option<String>,
            valle_timeline::ExactRational,
            bool,
            Option<(ExactRational, ExactRational)>,
        ),
    >,
{
    let mut cursor = RationalTime::ZERO;
    let mut projected = Vec::new();
    for (item_index, item_id, timeline_path, duration, advances_cursor, source_clock) in items {
        let duration = RationalTime::from_exact(duration);
        let start = cursor;
        let end = if advances_cursor {
            cursor
                .checked_add(duration)
                .map_err(|error| timeline_error("timeline_document_view", error))?
        } else {
            cursor
        };
        projected.push(TimelineSequenceItemView {
            item_id,
            timeline_path,
            item_index,
            start_seconds: start.as_f64(),
            duration_seconds: duration.as_f64(),
            end_seconds: end.as_f64(),
            start_frame: quantize_frame_boundary(start, frame_rate)
                .map_err(|error| timeline_error("timeline_document_view", error))?,
            duration_frames: quantize_frame_boundary(duration, frame_rate)
                .map_err(|error| timeline_error("timeline_document_view", error))?,
            end_frame: quantize_frame_boundary(end, frame_rate)
                .map_err(|error| timeline_error("timeline_document_view", error))?,
            advances_cursor,
            source_start_seconds: source_clock.map(|value| value.0.as_f64()),
            source_start_frame: source_clock
                .map(|value| {
                    // Source clocks are exact seconds and use the same Motion/Studio frame UI rate.
                    // Quantization stays in Rust even though this projection is display-only.
                    let source_start = RationalTime::from_exact(value.0);
                    quantize_frame_boundary(source_start, frame_rate)
                })
                .transpose()
                .map_err(|error| timeline_error("timeline_document_view", error))?,
            source_rate: source_clock.map(|value| value.1.as_f64()),
            motion_frames: None,
        });
        cursor = end;
    }
    Ok(TimelineSequenceView {
        band,
        track_id,
        track_index,
        duration_seconds: cursor.as_f64(),
        items: projected,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CompilerClipOrigin {
    track_index: usize,
    clip_index: usize,
}

fn compiler_track_id(band: &str, track_index: usize) -> String {
    format!("timeline:{band}-track:{track_index}")
}

fn compiler_clip_origin(id: &str, band: &str) -> Option<CompilerClipOrigin> {
    let rest = id.strip_prefix(&format!("timeline:{band}-track:"))?;
    let (track, clip) = rest.split_once(":clip:")?;
    let track_index = track.parse::<usize>().ok()?;
    let clip_index = clip.parse::<usize>().ok()?;
    (id == format!("timeline:{band}-track:{track_index}:clip:{clip_index}")).then_some(
        CompilerClipOrigin {
            track_index,
            clip_index,
        },
    )
}

fn compiler_timeline_path(id: &str, band: &str, track_index: usize) -> Option<String> {
    let origin = compiler_clip_origin(id, band)?;
    (origin.track_index == track_index).then(|| {
        format!(
            "/tracks/{band}/{}/clips/{}",
            origin.track_index, origin.clip_index
        )
    })
}

fn project_adjustment_sequences(
    document: &TimelineDocumentWire,
    frame_rate: FrameRate,
) -> Result<Vec<TimelineSequenceView>, String> {
    let mut compiler_tracks: BTreeMap<usize, Vec<(usize, &TimedAdjustmentWire)>> = BTreeMap::new();
    let mut canonical = Vec::new();
    let mut used_track_indices = BTreeSet::new();

    for (canonical_index, adjustment) in document.adjustments.iter().enumerate() {
        if let Some(origin) = compiler_clip_origin(&adjustment.id, "adjustment") {
            used_track_indices.insert(origin.track_index);
            compiler_tracks
                .entry(origin.track_index)
                .or_default()
                .push((origin.clip_index, adjustment));
        } else {
            canonical.push((canonical_index, adjustment));
        }
    }

    let mut projected =
        Vec::with_capacity(compiler_tracks.len() + usize::from(!canonical.is_empty()));
    for (track_index, mut adjustments) in compiler_tracks {
        adjustments.sort_by_key(|(clip_index, _)| *clip_index);
        projected.push(project_absolute_adjustment_sequence(
            compiler_track_id("adjustment", track_index),
            track_index,
            frame_rate,
            adjustments.into_iter().map(|(clip_index, adjustment)| {
                (
                    clip_index,
                    adjustment,
                    compiler_timeline_path(&adjustment.id, "adjustment", track_index),
                )
            }),
        )?);
    }

    if !canonical.is_empty() {
        let mut track_index = 0;
        while used_track_indices.contains(&track_index) {
            track_index = track_index
                .checked_add(1)
                .ok_or_else(|| "no synthetic adjustment track index remains".to_owned())?;
        }
        projected.push(project_absolute_adjustment_sequence(
            "canonical:adjustments".to_owned(),
            track_index,
            frame_rate,
            canonical
                .into_iter()
                .map(|(item_index, adjustment)| (item_index, adjustment, None)),
        )?);
    }

    Ok(projected)
}

fn project_absolute_adjustment_sequence<'a, I>(
    track_id: String,
    track_index: usize,
    frame_rate: FrameRate,
    adjustments: I,
) -> Result<TimelineSequenceView, String>
where
    I: IntoIterator<Item = (usize, &'a TimedAdjustmentWire, Option<String>)>,
{
    let mut duration = RationalTime::ZERO;
    let mut items = Vec::new();
    for (item_index, adjustment, timeline_path) in adjustments {
        let start = RationalTime::from_exact(adjustment.start);
        let item_duration = RationalTime::from_exact(adjustment.duration);
        let end = start
            .checked_add(item_duration)
            .map_err(|error| timeline_error("timeline_document_view", error))?;
        if end > duration {
            duration = end;
        }
        items.push(TimelineSequenceItemView {
            item_id: adjustment.id.clone(),
            timeline_path,
            item_index,
            start_seconds: start.as_f64(),
            duration_seconds: item_duration.as_f64(),
            end_seconds: end.as_f64(),
            start_frame: quantize_frame_boundary(start, frame_rate)
                .map_err(|error| timeline_error("timeline_document_view", error))?,
            duration_frames: quantize_frame_boundary(item_duration, frame_rate)
                .map_err(|error| timeline_error("timeline_document_view", error))?,
            end_frame: quantize_frame_boundary(end, frame_rate)
                .map_err(|error| timeline_error("timeline_document_view", error))?,
            advances_cursor: false,
            source_start_seconds: None,
            source_start_frame: None,
            source_rate: None,
            motion_frames: None,
        });
    }
    Ok(TimelineSequenceView {
        band: "adjustment",
        track_id,
        track_index,
        duration_seconds: duration.as_f64(),
        items,
    })
}

fn project_motion_frames(
    source: &valle_timeline::internal::wire::document::MotionInstanceWire,
    frame_rate: FrameRate,
) -> Result<MotionAuthoringFramesView, String> {
    let frame = |value: ExactRational| {
        quantize_frame_boundary(RationalTime::from_exact(value), frame_rate)
            .map_err(|error| timeline_error("timeline_document_view", error))
    };
    let cues = source
        .cues
        .iter()
        .map(|(name, cue)| {
            let view = match cue {
                MotionCueBindingWire::SourceRange {
                    start,
                    end,
                    enter_duration,
                    exit_duration,
                } => MotionCueFramesView::SourceRange {
                    start_frame: frame(*start)?,
                    end_frame: frame(*end)?,
                    enter_frames: frame(*enter_duration)?,
                    exit_frames: frame(*exit_duration)?,
                },
            };
            Ok((name.clone(), view))
        })
        .collect::<Result<_, String>>()?;
    Ok(MotionAuthoringFramesView {
        source_duration_frames: frame(source.source_duration)?,
        enter_frames: source.phases.enter_duration.map(frame).transpose()?,
        exit_frames: source.phases.exit_duration.map(frame).transpose()?,
        cues,
    })
}

fn timeline_error(code: &str, error: impl std::fmt::Display) -> String {
    format!("[{code}] {error}")
}

#[cfg(test)]
mod tests {
    use super::{
        canonicalize_timeline_document_native, compile_timeline_native, compiler_clip_origin,
        normalize_timeline_native, timeline_document_view_native,
        timeline_source_time_delta_from_frames_native, timeline_time_from_frames_native,
    };

    const TIMELINE: &str = r##"{
      "canvas": {"width": 640, "height": 360, "fps": 30, "background": "#112233ff"},
      "resources": {
        "audio": "https://example.test/audio.wav",
        "font": "https://example.test/font.ttf"
      },
      "tracks": {
        "visual": [{"clips": [
          {"start": 0, "duration": 1, "kind": "solid", "color": "#ff0000ff"},
          {"start": 2, "duration": 1, "kind": "solid", "color": "#00ff00ff"}
        ]}],
        "audio": [{"clips": [
          {"start": 0, "duration": 1, "src": "audio"}
        ]}],
        "caption": [{
          "style": {"font": "font"},
          "clips": [{"start": 0, "duration": 1, "text": "hello"}]
        }],
        "adjustment": [{"clips": [
          {"start": 0.25, "duration": 0.5, "kind": "color-grade", "temperature": 0.1}
        ]}]
      }
    }"##;

    #[test]
    fn compile_timeline_uses_the_reserved_timeline_identity_namespace() {
        let compiled = compile_timeline_native(TIMELINE).unwrap();
        let timeline = valle_timeline::internal::decode_canonical(&compiled).unwrap();
        let wire = timeline.to_wire();
        let valle_timeline::internal::wire::document::VisualItemWire::Clip(clip) =
            &wire.document.visual.tracks[0].items[0]
        else {
            panic!("expected compiled visual clip");
        };
        assert_eq!(clip.id, "timeline:visual-track:0:clip:0");

        let error = compile_timeline_native(&TIMELINE.replacen(
            "\"canvas\"",
            "\"version\": 2, \"canvas\"",
            1,
        ))
        .unwrap_err();
        assert!(error.starts_with("[timeline_decode]"), "{error}");
    }

    #[test]
    fn sparse_timeline_normalization_and_frame_conversion_stay_in_rust() {
        let noisy = TIMELINE.replace("0.1}", "0.13333333333333333}");
        let normalized = normalize_timeline_native(&noisy).unwrap();
        assert!(normalized.contains("\"temperature\":0.133333"));
        assert_eq!(normalize_timeline_native(&normalized).unwrap(), normalized);

        assert_eq!(
            timeline_time_from_frames_native(4.0, "30").unwrap(),
            0.133333
        );
        assert_eq!(
            timeline_time_from_frames_native(1.0, "128").unwrap(),
            0.007813
        );
        assert_eq!(
            timeline_time_from_frames_native(-1.0, "128").unwrap(),
            -0.007813
        );
        assert_eq!(
            timeline_time_from_frames_native(1.0, "\"30000/1001\"").unwrap(),
            0.033367
        );
        assert!(timeline_time_from_frames_native(0.5, "30").is_err());

        assert_eq!(
            timeline_source_time_delta_from_frames_native(1.0, "\"30000/1001\"", "2").unwrap(),
            0.066733
        );
        assert_eq!(
            timeline_source_time_delta_from_frames_native(1.0, "\"30000/1001\"", "0.5").unwrap(),
            0.016683
        );
        assert_eq!(
            timeline_source_time_delta_from_frames_native(-1.0, "128", "2").unwrap(),
            -0.015625
        );
        assert!(timeline_source_time_delta_from_frames_native(1.0, "30", "0").is_err());
    }

    #[test]
    fn canonical_document_helpers_share_one_fail_closed_contract() {
        let compiled = compile_timeline_native(TIMELINE).unwrap();
        let canonical = canonicalize_timeline_document_native(&compiled).unwrap();
        assert_eq!(
            canonicalize_timeline_document_native(&canonical).unwrap(),
            canonical
        );
        let duplicate =
            canonical.replacen("\"metadata\":{}", "\"metadata\":{\"x\":1,\"\\u0078\":2}", 1);
        let error = canonicalize_timeline_document_native(&duplicate).unwrap_err();
        assert!(error.starts_with("[timeline_document_decode]"));
    }

    #[test]
    fn document_view_projects_four_bands_and_public_timeline_paths() {
        let compiled = compile_timeline_native(TIMELINE).unwrap();
        let view: serde_json::Value =
            serde_json::from_str(&timeline_document_view_native(&compiled).unwrap()).unwrap();
        let sequences = view["sequences"].as_array().unwrap();
        assert_eq!(
            sequences
                .iter()
                .map(|sequence| sequence["band"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["visual", "audio", "caption", "adjustment"]
        );

        let visual = &sequences[0]["items"];
        assert_eq!(visual[0]["timelinePath"], "/tracks/visual/0/clips/0");
        assert!(visual[1]["timelinePath"].is_null());
        assert_eq!(visual[2]["timelinePath"], "/tracks/visual/0/clips/1");
        assert_eq!(
            sequences[1]["items"][0]["timelinePath"],
            "/tracks/audio/0/clips/0"
        );
        assert_eq!(
            sequences[2]["items"][0]["timelinePath"],
            "/tracks/caption/0/clips/0"
        );

        let adjustment = &sequences[3];
        assert_eq!(adjustment["trackId"], "timeline:adjustment-track:0");
        assert_eq!(
            adjustment["items"][0]["timelinePath"],
            "/tracks/adjustment/0/clips/0"
        );
        assert_eq!(adjustment["items"][0]["startSeconds"], 0.25);
        assert_eq!(adjustment["items"][0]["durationSeconds"], 0.5);
        assert_eq!(adjustment["items"][0]["endSeconds"], 0.75);
        assert_eq!(adjustment["items"][0]["advancesCursor"], false);
    }

    #[test]
    fn arbitrary_canonical_ids_are_not_reported_as_public_timeline_paths() {
        let compiled = compile_timeline_native(TIMELINE).unwrap();
        let mut value: serde_json::Value = serde_json::from_str(&compiled).unwrap();
        value["document"]["visual"]["tracks"][0]["items"][0]["id"] =
            serde_json::json!("custom:visual");
        value["document"]["adjustments"][0]["id"] = serde_json::json!("custom:adjustment");
        let json = serde_json::to_string(&value).unwrap();
        let view: serde_json::Value =
            serde_json::from_str(&timeline_document_view_native(&json).unwrap()).unwrap();
        assert!(view["sequences"][0]["items"][0]["timelinePath"].is_null());
        let adjustment = view["sequences"]
            .as_array()
            .unwrap()
            .iter()
            .find(|sequence| sequence["band"] == "adjustment")
            .unwrap();
        assert_eq!(adjustment["trackId"], "canonical:adjustments");
        assert!(adjustment["items"][0]["timelinePath"].is_null());

        assert!(compiler_clip_origin("timeline:visual-track:0:gap:1", "visual").is_none());
        assert!(compiler_clip_origin("timeline:visual-track:0:transition:1", "visual").is_none());
    }

    #[test]
    fn adjustment_view_rebuilds_multiple_public_tracks_and_keeps_a_synthetic_fallback() {
        let mut timeline: serde_json::Value = serde_json::from_str(TIMELINE).unwrap();
        timeline["tracks"]["adjustment"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"clips": [{
                "start": 1,
                "duration": 0.25,
                "kind": "color-grade",
                "temperature": -0.2
            }]}));
        let compiled = compile_timeline_native(&serde_json::to_string(&timeline).unwrap()).unwrap();
        let mut canonical: serde_json::Value = serde_json::from_str(&compiled).unwrap();
        let adjustments = canonical["document"]["adjustments"].as_array_mut().unwrap();
        let mut arbitrary = adjustments[0].clone();
        arbitrary["id"] = serde_json::json!("custom:adjustment");
        arbitrary["start"] = serde_json::json!("3/2");
        adjustments.push(arbitrary);

        let view: serde_json::Value = serde_json::from_str(
            &timeline_document_view_native(&serde_json::to_string(&canonical).unwrap()).unwrap(),
        )
        .unwrap();
        let adjustment_sequences = view["sequences"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|sequence| sequence["band"] == "adjustment")
            .collect::<Vec<_>>();
        assert_eq!(adjustment_sequences.len(), 3);
        assert_eq!(
            adjustment_sequences[0]["items"][0]["timelinePath"],
            "/tracks/adjustment/0/clips/0"
        );
        assert_eq!(
            adjustment_sequences[1]["items"][0]["timelinePath"],
            "/tracks/adjustment/1/clips/0"
        );
        assert_eq!(adjustment_sequences[2]["trackId"], "canonical:adjustments");
        assert!(adjustment_sequences[2]["items"][0]["timelinePath"].is_null());
    }
}
