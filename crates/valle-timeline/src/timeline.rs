//! Validated public Timeline timeline boundary.

use std::collections::BTreeSet;

use serde::Serialize;
use serde_json::json;

use crate::{
    canonical_json::{CanonicalJsonError, parse_strict, to_canonical_bytes},
    quantize::quantize_canonical_scalar,
    wire::timeline::{
        EasingWire, JsonObject, TimelineAdjustmentWire, TimelineCaptionBehaviorWire,
        TimelineCaptionClipWire, TimelineCaptionLayoutWire, TimelineCaptionPresentationWire,
        TimelineParamWire, TimelineTextRunWire, TimelineVisualClipWire, TimelineVisualSourceWire,
        TimelineWire,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TimelineJsonIssue {
    Utf8Bom,
    InvalidUnicode,
    DuplicateObjectKey,
    UnsafeInteger,
    NonFiniteNumber,
    MalformedJson,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum TimelineDecodeError {
    #[error("input violates valle-json/1: {issue:?}")]
    CanonicalJson {
        issue: TimelineJsonIssue,
        path: String,
    },
    #[error("invalid Timeline: {message}")]
    InvalidShape { message: String },
    #[error(transparent)]
    InvalidTimeline(#[from] TimelineValidationReport),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TimelineEncodeError {
    #[error("Timeline timeline document cannot be encoded as valle-json/1")]
    InvalidJson,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineDiagnostic {
    pub code: String,
    pub path: String,
    pub details: JsonObject,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TimelineValidationReport {
    pub diagnostics: Vec<TimelineDiagnostic>,
}

impl std::fmt::Display for TimelineValidationReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Timeline validation failed")?;
        for diagnostic in &self.diagnostics {
            write!(
                f,
                "; {}: {} ({})",
                diagnostic.path,
                diagnostic.code,
                serde_json::json!(diagnostic.details)
            )?;
        }
        Ok(())
    }
}
impl std::error::Error for TimelineValidationReport {}

/// Shape-checked, normalized public Timeline state.
#[derive(Debug, Clone, PartialEq)]
pub struct Timeline {
    wire: TimelineWire,
}

impl Timeline {
    pub fn from_wire(mut wire: TimelineWire) -> Result<Self, TimelineValidationReport> {
        let mut validator = TimelineValidator::default();
        validator.normalize_and_validate(&mut wire);
        if validator.diagnostics.is_empty() {
            Ok(Self { wire })
        } else {
            Err(TimelineValidationReport {
                diagnostics: validator.diagnostics,
            })
        }
    }

    pub fn wire(&self) -> &TimelineWire {
        &self.wire
    }

    pub fn to_wire(&self) -> TimelineWire {
        self.wire.clone()
    }

    pub fn into_wire(self) -> TimelineWire {
        self.wire
    }
}

/// Strictly decodes and q6-normalizes one complete public Timeline document.
pub fn decode_timeline(json: &str) -> Result<Timeline, TimelineDecodeError> {
    parse_strict(json.as_bytes()).map_err(map_json_error)?;
    // Decode the original source so TimelineTimeWire sees the exact decimal token
    // before applying deterministic half-away-from-zero q6 normalization.
    let wire: TimelineWire = serde_path_to_error::deserialize(
        &mut serde_json::Deserializer::from_str(json),
    )
    .map_err(|error| TimelineDecodeError::InvalidShape {
        message: error.to_string(),
    })?;
    Timeline::from_wire(wire).map_err(TimelineDecodeError::InvalidTimeline)
}

/// Encodes the already validated sparse Timeline state as deterministic JCS.
pub fn timeline_bytes(timeline: &Timeline) -> Result<Vec<u8>, TimelineEncodeError> {
    to_canonical_bytes(timeline.wire()).map_err(|_| TimelineEncodeError::InvalidJson)
}

#[derive(Default)]
struct TimelineValidator {
    diagnostics: Vec<TimelineDiagnostic>,
}

impl TimelineValidator {
    fn normalize_and_validate(&mut self, timeline: &mut TimelineWire) {
        if timeline.canvas.width == 0 {
            self.error("invalid_canvas_width", "/canvas/width", json!({}));
        }
        if timeline.canvas.height == 0 {
            self.error("invalid_canvas_height", "/canvas/height", json!({}));
        }
        if timeline.canvas.fps.try_to_frame_rate().is_err() {
            self.error("invalid_frame_rate", "/canvas/fps", json!({}));
        }

        let mut valid_resources = BTreeSet::new();
        for (key, url) in &timeline.resources {
            let path = format!("/resources/{}", pointer_escape(key));
            if !valid_resource_key(key) {
                self.error("invalid_resource_key", &path, json!({ "key": key }));
            } else {
                valid_resources.insert(key.as_str());
            }
            if !valid_resource_url(url) {
                self.error("invalid_resource_url", &path, json!({ "url": url }));
            }
        }

        for (track_index, track) in timeline.tracks.visual.iter_mut().enumerate() {
            let track_path = format!("/tracks/visual/{track_index}");
            for (clip_index, clip) in track.clips.iter_mut().enumerate() {
                let path = format!("{track_path}/clips/{clip_index}");
                self.visual_clip(clip, &path, &valid_resources);
            }
        }

        for (track_index, track) in timeline.tracks.audio.iter_mut().enumerate() {
            let track_path = format!("/tracks/audio/{track_index}");
            for (clip_index, clip) in track.clips.iter_mut().enumerate() {
                let path = format!("{track_path}/clips/{clip_index}");
                self.positive_time(&clip.duration, &format!("{path}/duration"));
                self.resource_ref(&clip.src, &format!("{path}/src"), &valid_resources);
                if let Some(rate) = &clip.rate {
                    self.positive_time(rate, &format!("{path}/rate"));
                }
                normalize_param(&mut clip.gain, &format!("{path}/gain"), self);
                normalize_param(&mut clip.pan, &format!("{path}/pan"), self);
            }
        }

        for (track_index, track) in timeline.tracks.adjustment.iter_mut().enumerate() {
            let track_path = format!("/tracks/adjustment/{track_index}");
            for (clip_index, clip) in track.clips.iter_mut().enumerate() {
                let path = format!("{track_path}/clips/{clip_index}");
                self.positive_time(&clip.duration, &format!("{path}/duration"));
                match &mut clip.adjustment {
                    TimelineAdjustmentWire::ColorGrade { temperature } => {
                        normalize_f64(temperature, &format!("{path}/temperature"), self);
                        if !(-1.0..=1.0).contains(temperature) {
                            self.error(
                                "temperature_out_of_range",
                                &format!("{path}/temperature"),
                                json!({ "minimum": -1.0, "maximum": 1.0 }),
                            );
                        }
                    }
                }
            }
        }

        for (track_index, track) in timeline.tracks.caption.iter_mut().enumerate() {
            let track_path = format!("/tracks/caption/{track_index}");
            self.resource_ref(
                &track.style.font,
                &format!("{track_path}/style/font"),
                &valid_resources,
            );
            normalize_closed(
                &mut track.style.font_size,
                &format!("{track_path}/style/fontSize"),
                self,
            );
            if let Some(shadow) = &mut track.style.shadow {
                normalize_array(
                    &mut shadow.offset,
                    &format!("{track_path}/style/shadow/offset"),
                    self,
                );
                normalize_closed(
                    &mut shadow.blur,
                    &format!("{track_path}/style/shadow/blur"),
                    self,
                );
            }
            if let Some(layout) = &mut track.layout {
                normalize_layout(layout, &format!("{track_path}/layout"), self);
            }
            for (clip_index, clip) in track.clips.iter_mut().enumerate() {
                self.caption_clip(clip, &format!("{track_path}/clips/{clip_index}"));
            }
        }
    }

    fn visual_clip(
        &mut self,
        clip: &mut TimelineVisualClipWire,
        path: &str,
        resources: &BTreeSet<&str>,
    ) {
        self.positive_time(&clip.duration, &format!("{path}/duration"));
        match &mut clip.source {
            TimelineVisualSourceWire::Video { src, rate, .. }
            | TimelineVisualSourceWire::Lottie { src, rate, .. } => {
                self.resource_ref(src, &format!("{path}/src"), resources);
                if let Some(rate) = rate {
                    self.positive_time(rate, &format!("{path}/rate"));
                }
            }
            TimelineVisualSourceWire::Image { src, .. } => {
                self.resource_ref(src, &format!("{path}/src"), resources);
            }
            TimelineVisualSourceWire::Motion {
                component,
                rate,
                props,
                resources: bindings,
                ..
            } => {
                self.resource_ref(component, &format!("{path}/component"), resources);
                if let Some(rate) = rate {
                    self.positive_time(rate, &format!("{path}/rate"));
                }
                for (name, param) in props {
                    normalize_open_param_easing(
                        param,
                        &format!("{path}/props/{}", pointer_escape(name)),
                        self,
                    );
                }
                for (name, resource) in bindings {
                    self.resource_ref(
                        resource,
                        &format!("{path}/resources/{}", pointer_escape(name)),
                        resources,
                    );
                }
            }
            TimelineVisualSourceWire::Solid { .. } => {}
        }
        normalize_param(&mut clip.position, &format!("{path}/position"), self);
        normalize_param(&mut clip.scale, &format!("{path}/scale"), self);
        normalize_param(&mut clip.size, &format!("{path}/size"), self);
        if let TimelineVisualSourceWire::Video { gain, .. } = &mut clip.source {
            normalize_param(gain, &format!("{path}/gain"), self);
        }
        normalize_param(&mut clip.rotation, &format!("{path}/rotation"), self);
        if let Some(anchor) = &mut clip.anchor {
            normalize_array(anchor, &format!("{path}/anchor"), self);
        }
        normalize_param(&mut clip.opacity, &format!("{path}/opacity"), self);
    }

    fn caption_clip(&mut self, clip: &mut TimelineCaptionClipWire, path: &str) {
        self.positive_time(&clip.duration, &format!("{path}/duration"));
        match (&clip.text, &clip.runs) {
            (Some(_), None) => {}
            (None, Some(runs)) if !runs.is_empty() => {}
            _ => self.error(
                "caption_content_exactly_one",
                path,
                json!({ "required": "exactly one of text or non-empty runs" }),
            ),
        }
        if let Some(runs) = &mut clip.runs {
            for (index, run) in runs.iter_mut().enumerate() {
                normalize_text_run(run, &clip.duration, &format!("{path}/runs/{index}"), self);
            }
        }
        let karaoke = matches!(
            clip.behavior.as_ref(),
            Some(TimelineCaptionBehaviorWire::Karaoke { .. })
        );
        if !karaoke {
            if let Some(runs) = &clip.runs {
                for (index, run) in runs.iter().enumerate() {
                    if run.start.is_some() || run.end.is_some() {
                        self.error(
                            "run_timing_requires_karaoke",
                            &format!("{path}/runs/{index}"),
                            json!({ "requiredBehavior": "karaoke" }),
                        );
                    }
                }
            }
        }
        if let Some(layout) = &mut clip.layout {
            normalize_layout(layout, &format!("{path}/layout"), self);
        }
        if let Some(behavior) = &mut clip.behavior {
            match behavior {
                TimelineCaptionBehaviorWire::Scroll { speed, .. } => {
                    normalize_f64(speed, &format!("{path}/behavior/speed"), self);
                    if *speed == 0.0 {
                        self.error(
                            "scroll_speed_zero",
                            &format!("{path}/behavior/speed"),
                            json!({ "required": "non-zero canvas pixels per second" }),
                        );
                    }
                }
                TimelineCaptionBehaviorWire::Karaoke { .. } => {
                    if let Some(runs) = &clip.runs {
                        let mut previous_end = crate::time::ExactRational::ZERO;
                        for (index, run) in runs.iter().enumerate() {
                            let run_path = format!("{path}/runs/{index}");
                            let (Some(start), Some(end)) = (&run.start, &run.end) else {
                                self.error(
                                    "karaoke_run_timing_required",
                                    &run_path,
                                    json!({ "required": "start and end" }),
                                );
                                continue;
                            };
                            let start = start.to_exact();
                            let end = end.to_exact();
                            if index > 0 && start < previous_end {
                                self.error(
                                    "karaoke_run_timing_overlap",
                                    &format!("{run_path}/start"),
                                    json!({}),
                                );
                            }
                            previous_end = end;
                        }
                    } else {
                        self.error(
                            "karaoke_requires_timed_runs",
                            path,
                            json!({ "required": "non-empty runs with clip-local start and end" }),
                        );
                    }
                }
            }
        }
        if clip.presentation.is_some()
            && (clip.enter.is_some() || clip.display.is_some() || clip.exit.is_some())
        {
            self.error(
                "preset_presentation_conflict",
                path,
                json!({ "required": "presets or presentation, not both" }),
            );
        }
        if let Some(options) = &mut clip.display {
            normalize_closed(&mut options.rate, &format!("{path}/display/rate"), self);
        }
        if let Some(presentation) = &mut clip.presentation {
            normalize_presentation(presentation, &format!("{path}/presentation"), self);
        }
    }

    fn positive_time(&mut self, value: &crate::wire::timeline::TimelineTimeWire, path: &str) {
        if !value.to_exact().is_positive() {
            self.error("time_must_be_positive", path, json!({}));
        }
    }

    fn resource_ref(&mut self, key: &str, path: &str, resources: &BTreeSet<&str>) {
        if !resources.contains(key) {
            self.error("missing_resource", path, json!({ "resource": key }));
        }
    }

    fn error(&mut self, code: &str, path: &str, details: serde_json::Value) {
        self.diagnostics.push(TimelineDiagnostic {
            code: code.to_owned(),
            path: path.to_owned(),
            details: details
                .as_object()
                .map(|object| {
                    object
                        .iter()
                        .map(|(key, value)| (key.clone(), value.clone()))
                        .collect()
                })
                .unwrap_or_default(),
        });
    }
}

trait ClosedTimelineValue {
    fn normalize_q6(&mut self, path: &str, validator: &mut TimelineValidator);
}

impl ClosedTimelineValue for f64 {
    fn normalize_q6(&mut self, path: &str, validator: &mut TimelineValidator) {
        normalize_f64(self, path, validator);
    }
}

impl<const N: usize> ClosedTimelineValue for [f64; N] {
    fn normalize_q6(&mut self, path: &str, validator: &mut TimelineValidator) {
        normalize_array(self, path, validator);
    }
}

fn normalize_param<T: ClosedTimelineValue>(
    param: &mut Option<TimelineParamWire<T>>,
    path: &str,
    validator: &mut TimelineValidator,
) {
    let Some(param) = param else { return };
    match param {
        TimelineParamWire::Value(value) => value.normalize_q6(path, validator),
        TimelineParamWire::Curve(curve) => {
            for (index, keyframe) in curve.keyframes.iter_mut().enumerate() {
                let (value, easing) = match keyframe {
                    crate::wire::timeline::TimelineKeyframeWire::Plain((time, value)) => {
                        let _ = time;
                        (value, None)
                    }
                    crate::wire::timeline::TimelineKeyframeWire::Eased((time, value, easing)) => {
                        let _ = time;
                        (value, Some(easing))
                    }
                };
                value.normalize_q6(&format!("{path}/keyframes/{index}/1"), validator);
                if let Some(easing) = easing {
                    normalize_easing(easing, &format!("{path}/keyframes/{index}/2"), validator);
                }
            }
        }
    }
}

fn normalize_open_param_easing(
    param: &mut TimelineParamWire<serde_json::Value>,
    path: &str,
    validator: &mut TimelineValidator,
) {
    match param {
        TimelineParamWire::Value(value) => normalize_json_numbers(value, path, validator),
        TimelineParamWire::Curve(curve) => {
            for (index, keyframe) in curve.keyframes.iter_mut().enumerate() {
                let (value, easing) = match keyframe {
                    crate::wire::timeline::TimelineKeyframeWire::Plain((_, value)) => (value, None),
                    crate::wire::timeline::TimelineKeyframeWire::Eased((_, value, easing)) => {
                        (value, Some(easing))
                    }
                };
                normalize_json_numbers(value, &format!("{path}/keyframes/{index}/1"), validator);
                if let Some(easing) = easing {
                    normalize_easing(easing, &format!("{path}/keyframes/{index}/2"), validator);
                }
            }
        }
    }
}

fn normalize_json_numbers(
    value: &mut serde_json::Value,
    path: &str,
    validator: &mut TimelineValidator,
) {
    match value {
        serde_json::Value::Number(number)
            if number.as_i64().is_none() && number.as_u64().is_none() =>
        {
            let Some(float) = number.as_f64() else {
                validator.error("non_finite_number", path, json!({}));
                return;
            };
            let normalized = quantize_canonical_scalar(float);
            let Some(number) = serde_json::Number::from_f64(normalized) else {
                validator.error("non_finite_number", path, json!({}));
                return;
            };
            *value = serde_json::Value::Number(number);
        }
        serde_json::Value::Array(values) => {
            for (index, value) in values.iter_mut().enumerate() {
                normalize_json_numbers(value, &format!("{path}/{index}"), validator);
            }
        }
        serde_json::Value::Object(values) => {
            for (key, value) in values {
                normalize_json_numbers(
                    value,
                    &format!("{path}/{}", pointer_escape(key)),
                    validator,
                );
            }
        }
        _ => {}
    }
}

fn normalize_presentation(
    presentation: &mut TimelineCaptionPresentationWire,
    path: &str,
    validator: &mut TimelineValidator,
) {
    normalize_param(
        &mut presentation.opacity,
        &format!("{path}/opacity"),
        validator,
    );
    normalize_param(
        &mut presentation.translation,
        &format!("{path}/translation"),
        validator,
    );
    normalize_param(&mut presentation.scale, &format!("{path}/scale"), validator);
    normalize_param(
        &mut presentation.rotation,
        &format!("{path}/rotation"),
        validator,
    );
    normalize_param(
        &mut presentation.clip_inset,
        &format!("{path}/clipInset"),
        validator,
    );
    normalize_param(&mut presentation.blur, &format!("{path}/blur"), validator);
}

fn normalize_layout(
    layout: &mut TimelineCaptionLayoutWire,
    path: &str,
    validator: &mut TimelineValidator,
) {
    if let Some(region) = &mut layout.region {
        normalize_array(region, &format!("{path}/region"), validator);
    }
}

fn normalize_text_run(
    run: &mut TimelineTextRunWire,
    clip_duration: &crate::wire::timeline::TimelineTimeWire,
    path: &str,
    validator: &mut TimelineValidator,
) {
    normalize_closed(&mut run.font_size, &format!("{path}/fontSize"), validator);
    match (&run.start, &run.end) {
        (None, None) => {}
        (Some(start), Some(end)) => {
            let start = start.to_exact();
            let end = end.to_exact();
            if start.is_negative() {
                validator.error("run_start_before_clip", &format!("{path}/start"), json!({}));
            }
            if end <= start {
                validator.error(
                    "run_end_must_follow_start",
                    &format!("{path}/end"),
                    json!({}),
                );
            }
            if end > clip_duration.to_exact() {
                validator.error("run_end_after_clip", &format!("{path}/end"), json!({}));
            }
        }
        _ => validator.error(
            "run_timing_pair_required",
            path,
            json!({ "required": "start and end together" }),
        ),
    }
}

fn normalize_closed(value: &mut Option<f64>, path: &str, validator: &mut TimelineValidator) {
    if let Some(value) = value {
        normalize_f64(value, path, validator);
    }
}

fn normalize_array<const N: usize>(
    values: &mut [f64; N],
    path: &str,
    validator: &mut TimelineValidator,
) {
    for (index, value) in values.iter_mut().enumerate() {
        normalize_f64(value, &format!("{path}/{index}"), validator);
    }
}

fn normalize_f64(value: &mut f64, path: &str, validator: &mut TimelineValidator) {
    if !value.is_finite() {
        validator.error("non_finite_number", path, json!({}));
        return;
    }
    *value = quantize_canonical_scalar(*value);
}

fn normalize_easing(easing: &mut EasingWire, path: &str, validator: &mut TimelineValidator) {
    if let EasingWire::CubicBezier(bezier) = easing {
        normalize_f64(&mut bezier.x1, &format!("{path}/x1"), validator);
        normalize_f64(&mut bezier.y1, &format!("{path}/y1"), validator);
        normalize_f64(&mut bezier.x2, &format!("{path}/x2"), validator);
        normalize_f64(&mut bezier.y2, &format!("{path}/y2"), validator);
    }
}

fn valid_resource_key(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 64
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn valid_resource_url(value: &str) -> bool {
    if value.is_empty()
        || value.len() > 8192
        || value
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
    {
        return false;
    }
    if let Some((scheme, rest)) = value.split_once(':') {
        let valid_scheme = !scheme.is_empty()
            && scheme.as_bytes()[0].is_ascii_alphabetic()
            && scheme
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'));
        if !valid_scheme || rest.is_empty() {
            return false;
        }
        if matches!(scheme, "http" | "https") {
            return rest
                .strip_prefix("//")
                .is_some_and(|authority| !authority.is_empty() && !authority.starts_with('/'));
        }
    }
    true
}

fn pointer_escape(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}

fn map_json_error(error: CanonicalJsonError) -> TimelineDecodeError {
    let (issue, path) = match error {
        CanonicalJsonError::Utf8Bom => (TimelineJsonIssue::Utf8Bom, String::new()),
        CanonicalJsonError::InvalidUnicode => (TimelineJsonIssue::InvalidUnicode, String::new()),
        CanonicalJsonError::DuplicateObjectKey { key } => (
            TimelineJsonIssue::DuplicateObjectKey,
            format!("/<duplicate:{key}>"),
        ),
        CanonicalJsonError::UnsafeInteger { .. } => {
            (TimelineJsonIssue::UnsafeInteger, String::new())
        }
        CanonicalJsonError::NonFiniteNumber => (TimelineJsonIssue::NonFiniteNumber, String::new()),
        CanonicalJsonError::MalformedJson | CanonicalJsonError::Encode => {
            (TimelineJsonIssue::MalformedJson, String::new())
        }
    };
    TimelineDecodeError::CanonicalJson { issue, path }
}
