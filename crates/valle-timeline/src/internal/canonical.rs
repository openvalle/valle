//! Validated Timeline canonical document state.
//!
//! Wire DTOs describe shape; this module is the only constructor for a
//! canonical/persistable Timeline.  A [`CanonicalTimeline`] therefore cannot
//! exist until every document-local invariant has been checked.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::{Value as JsonValue, json};

use crate::internal::{
    canonical_json::{CanonicalJsonError, parse_strict, to_canonical_bytes},
    document::{MAX_CAPTION_BLUR_SIGMA, MAX_CAPTION_FONT_SIZE, TimelineDocument},
    quantize::quantize_canonical_scalar,
    time::ExactRational,
    wire::document::*,
};

const MAX_CANONICAL_DOCUMENT_BYTES: usize = 16 * 1024 * 1024;
const MAX_TRACKS_PER_BAND: usize = 1_024;
const MAX_ITEMS_PER_TRACK: usize = 100_000;
const MAX_KEYFRAMES_PER_CURVE: usize = 100_000;
const MAX_MOTION_PROPS: usize = 4_096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DiagnosticPhase {
    Decode,
    LocalInvariant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DiagnosticSeverity {
    Error,
    Warning,
}

/// Stable structured diagnostic shared by decode and local construction.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContractDiagnostic {
    pub code: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_id: Option<String>,
    pub phase: DiagnosticPhase,
    pub severity: DiagnosticSeverity,
    pub details: JsonObject,
}

/// Complete local-invariant report. Construction is all-or-nothing.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalInvariantReport {
    pub diagnostics: Vec<ContractDiagnostic>,
}

impl LocalInvariantReport {
    pub fn is_empty(&self) -> bool {
        self.diagnostics.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CanonicalJsonIssue {
    Utf8Bom,
    InvalidUnicode,
    DuplicateObjectKey,
    UnsafeInteger,
    NonFiniteNumber,
    MalformedJson,
}

/// Public canonical-document decode failure. Parser implementation strings
/// never cross this boundary.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum CanonicalDecodeError {
    #[error("input violates valle-json/1: {issue:?}")]
    CanonicalJson {
        issue: CanonicalJsonIssue,
        path: String,
    },
    #[error("canonical Timeline document has an invalid wire shape")]
    InvalidShape,
    #[error("document violates local invariants")]
    InvalidDocument(LocalInvariantReport),
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CanonicalError {
    #[error("validated document could not be encoded as valle-json/1")]
    Encode,
}

/// The public persistence waist. Fields are intentionally private and this
/// type has no `Deserialize` implementation.
#[derive(Debug, Clone, PartialEq)]
pub struct CanonicalTimeline {
    document: TimelineDocument,
}

impl CanonicalTimeline {
    pub fn try_from_wire(
        mut wire: TimelineDocumentEnvelopeWire,
    ) -> Result<Self, LocalInvariantReport> {
        quantize_document_scalars(&mut wire.document);
        let mut validator = Validator::new(&wire);
        validator.validate();
        match to_canonical_bytes(&wire) {
            Ok(bytes) if bytes.len() > MAX_CANONICAL_DOCUMENT_BYTES => validator.error(
                "document_budget_exceeded",
                "",
                None,
                details([
                    ("limitBytes", json!(MAX_CANONICAL_DOCUMENT_BYTES)),
                    ("actualBytes", json!(bytes.len())),
                ]),
            ),
            Err(_) => validator.error("invalid_canonical_json_value", "", None, JsonObject::new()),
            Ok(_) => {}
        }
        if validator.diagnostics.is_empty() {
            let document = crate::internal::document::from_wire(wire.document)
                .expect("wire accepted by the canonical validator must convert to domain types");
            Ok(Self { document })
        } else {
            Err(LocalInvariantReport {
                diagnostics: validator.diagnostics,
            })
        }
    }

    pub fn document(&self) -> &TimelineDocument {
        &self.document
    }

    pub fn to_wire(&self) -> TimelineDocumentEnvelopeWire {
        TimelineDocumentEnvelopeWire {
            document: crate::internal::document::to_wire(&self.document),
        }
    }

    pub fn into_wire(self) -> TimelineDocumentEnvelopeWire {
        TimelineDocumentEnvelopeWire {
            document: crate::internal::document::to_wire(&self.document),
        }
    }
}

/// Normalize Timeline-owned scalars plus numeric leaves in Motion props. Other open JSON owned by
/// metadata or extension parameters remains byte-semantic and is canonicalized by JCS without
/// Timeline guessing at another namespace's precision.
fn quantize_document_scalars(document: &mut TimelineDocumentWire) {
    for track in &mut document.visual.tracks {
        for item in &mut track.items {
            let VisualItemWire::Clip(clip) = item else {
                continue;
            };
            quantize_visual_layer(&mut clip.layer);
            if let VisualSourceWire::Motion(motion) = &mut clip.source {
                for param in motion.props.values_mut() {
                    quantize_param(param, quantize_json_numbers);
                }
            }
        }
    }

    for track in &mut document.audio.tracks {
        for item in &mut track.items {
            if let AudioItemWire::Clip(clip) = item {
                quantize_param(&mut clip.gain, quantize_scalar);
                quantize_param(&mut clip.pan, quantize_scalar);
            }
        }
    }

    for timed in &mut document.adjustments {
        if let AdjustmentEffectWire::ColorGrade(grade) = &mut timed.effect {
            quantize_scalar(&mut grade.temperature);
        }
    }

    for track in &mut document.captions.tracks {
        for item in &mut track.items {
            let CaptionItemWire::Clip(caption) = item else {
                continue;
            };
            for run in &mut caption.runs {
                if let Some(font_size) = run
                    .style
                    .as_mut()
                    .and_then(|style| style.font_size.as_mut())
                {
                    quantize_scalar(font_size);
                }
            }
            quantize_scalar(&mut caption.style.font_size);
            if let Some(shadow) = &mut caption.style.shadow {
                quantize_components(&mut shadow.offset);
                quantize_scalar(&mut shadow.blur_sigma);
            }
            quantize_components(&mut caption.layout.region);
            quantize_param(&mut caption.presentation.opacity, quantize_scalar);
            quantize_param(&mut caption.presentation.translation, quantize_components);
            quantize_param(&mut caption.presentation.scale, quantize_scalar);
            quantize_param(&mut caption.presentation.rotation, quantize_scalar);
            quantize_param(&mut caption.presentation.clip_inset, quantize_components);
            quantize_param(&mut caption.presentation.blur_sigma, quantize_scalar);
            if let Some(CaptionBehaviorWire::Scroll { speed, .. }) = &mut caption.behavior {
                quantize_scalar(speed);
            }
        }
    }

    if let Some(camera) = &mut document.camera {
        quantize_param(&mut camera.center_x, quantize_scalar);
        quantize_param(&mut camera.center_y, quantize_scalar);
        quantize_param(&mut camera.zoom, quantize_scalar);
        quantize_param(&mut camera.rotation, quantize_scalar);
    }
}

fn quantize_visual_layer(layer: &mut VisualLayerWire) {
    quantize_param(&mut layer.transform.position, quantize_components);
    quantize_param(&mut layer.transform.scale, quantize_components);
    quantize_param(&mut layer.transform.rotation, quantize_scalar);
    quantize_components(&mut layer.transform.anchor);
    quantize_param(&mut layer.opacity, quantize_scalar);
    if let Some(mask) = &mut layer.mask {
        match mask {
            LayerMaskWire::Rect { rect, feather, .. }
            | LayerMaskWire::Ellipse { rect, feather, .. } => {
                quantize_param(rect, quantize_components);
                quantize_param(feather, quantize_scalar);
            }
        }
    }
}

fn quantize_param<T>(param: &mut ParamWire<T>, mut quantize_value: impl FnMut(&mut T)) {
    match param {
        ParamWire::Constant(constant) => quantize_value(&mut constant.value),
        ParamWire::Curve(curve) => {
            for keyframe in &mut curve.keyframes {
                quantize_value(&mut keyframe.value);
                if let Some(easing) = &mut keyframe.out_easing {
                    quantize_easing(easing);
                }
            }
        }
    }
}

fn quantize_easing(easing: &mut EasingWire) {
    if let EasingWire::CubicBezier(bezier) = easing {
        quantize_scalar(&mut bezier.x1);
        quantize_scalar(&mut bezier.y1);
        quantize_scalar(&mut bezier.x2);
        quantize_scalar(&mut bezier.y2);
    }
}

fn quantize_scalar(value: &mut f64) {
    *value = quantize_canonical_scalar(*value);
}

fn quantize_components<const N: usize>(values: &mut [f64; N]) {
    values.iter_mut().for_each(quantize_scalar);
}

fn quantize_json_numbers(value: &mut JsonValue) {
    match value {
        JsonValue::Number(number) if number.as_i64().is_none() && number.as_u64().is_none() => {
            if let Some(number) = number
                .as_f64()
                .map(quantize_canonical_scalar)
                .and_then(serde_json::Number::from_f64)
            {
                *value = JsonValue::Number(number);
            }
        }
        JsonValue::Array(values) => values.iter_mut().for_each(quantize_json_numbers),
        JsonValue::Object(values) => values.values_mut().for_each(quantize_json_numbers),
        _ => {}
    }
}

pub fn decode_canonical(json: &str) -> Result<CanonicalTimeline, CanonicalDecodeError> {
    let value = parse_strict(json.as_bytes()).map_err(map_canonical_json_error)?;
    let wire: TimelineDocumentEnvelopeWire =
        serde_json::from_value(value).map_err(|_| CanonicalDecodeError::InvalidShape)?;
    CanonicalTimeline::try_from_wire(wire).map_err(CanonicalDecodeError::InvalidDocument)
}

/// Strict `valle-json/1` decoding for immutable JSON resource payloads.
///
/// Unlike `serde_json::from_slice`, this rejects UTF-8 BOMs, malformed Unicode
/// escapes, duplicate object keys, non-finite numbers and integers outside the
/// interoperable safe-integer range before returning a semantic value.
pub fn decode_strict_json_value(input: &[u8]) -> Result<JsonValue, CanonicalDecodeError> {
    parse_strict(input).map_err(map_canonical_json_error)
}

pub fn canonical_bytes(timeline: &CanonicalTimeline) -> Result<Vec<u8>, CanonicalError> {
    to_canonical_bytes(&timeline.to_wire()).map_err(|_| CanonicalError::Encode)
}

pub fn encode_canonical(timeline: &CanonicalTimeline) -> Result<String, CanonicalError> {
    String::from_utf8(canonical_bytes(timeline)?).map_err(|_| CanonicalError::Encode)
}

fn map_canonical_json_error(error: CanonicalJsonError) -> CanonicalDecodeError {
    let (issue, path) = match error {
        CanonicalJsonError::Utf8Bom => (CanonicalJsonIssue::Utf8Bom, String::new()),
        CanonicalJsonError::InvalidUnicode => (CanonicalJsonIssue::InvalidUnicode, String::new()),
        CanonicalJsonError::DuplicateObjectKey { key } => (
            CanonicalJsonIssue::DuplicateObjectKey,
            format!("/<duplicate:{key}>"),
        ),
        CanonicalJsonError::UnsafeInteger { .. } => {
            (CanonicalJsonIssue::UnsafeInteger, String::new())
        }
        CanonicalJsonError::NonFiniteNumber => (CanonicalJsonIssue::NonFiniteNumber, String::new()),
        CanonicalJsonError::MalformedJson | CanonicalJsonError::Encode => {
            (CanonicalJsonIssue::MalformedJson, String::new())
        }
    };
    CanonicalDecodeError::CanonicalJson { issue, path }
}

struct Validator<'a> {
    wire: &'a TimelineDocumentEnvelopeWire,
    diagnostics: Vec<ContractDiagnostic>,
    ids: BTreeMap<String, (&'static str, String)>,
}

impl<'a> Validator<'a> {
    fn new(wire: &'a TimelineDocumentEnvelopeWire) -> Self {
        Self {
            wire,
            diagnostics: Vec::new(),
            ids: BTreeMap::new(),
        }
    }

    fn validate(&mut self) {
        let document = &self.wire.document;
        self.validate_canvas(&document.canvas);
        self.validate_document_budgets(document);
        if !crate::internal::Color(document.background.color.clone()).is_valid() {
            self.error(
                "invalid_color",
                "/document/background/color",
                None,
                JsonObject::new(),
            );
        }

        self.validate_visual(&document.visual, document.canvas.duration);
        self.validate_audio(&document.audio, document.canvas.duration);
        self.validate_adjustments(&document.adjustments, document.canvas.duration);
        self.validate_captions(&document.captions, document.canvas.duration);
        if let Some(camera) = &document.camera {
            self.validate_camera(camera, document.canvas.duration);
        }
    }

    fn validate_document_budgets(&mut self, document: &TimelineDocumentWire) {
        for (path, tracks, item_counts) in [
            (
                "/document/visual/tracks",
                document.visual.tracks.len(),
                document
                    .visual
                    .tracks
                    .iter()
                    .map(|track| track.items.len())
                    .collect::<Vec<_>>(),
            ),
            (
                "/document/audio/tracks",
                document.audio.tracks.len(),
                document
                    .audio
                    .tracks
                    .iter()
                    .map(|track| track.items.len())
                    .collect::<Vec<_>>(),
            ),
            (
                "/document/captions/tracks",
                document.captions.tracks.len(),
                document
                    .captions
                    .tracks
                    .iter()
                    .map(|track| track.items.len())
                    .collect::<Vec<_>>(),
            ),
        ] {
            if tracks > MAX_TRACKS_PER_BAND {
                self.error(
                    "track_budget_exceeded",
                    path,
                    None,
                    details([
                        ("limit", json!(MAX_TRACKS_PER_BAND)),
                        ("actual", json!(tracks)),
                    ]),
                );
            }
            for (index, items) in item_counts.into_iter().enumerate() {
                if items > MAX_ITEMS_PER_TRACK {
                    self.error(
                        "sequence_item_budget_exceeded",
                        &format!("{path}/{index}/items"),
                        None,
                        details([
                            ("limit", json!(MAX_ITEMS_PER_TRACK)),
                            ("actual", json!(items)),
                        ]),
                    );
                }
            }
        }
    }

    fn validate_canvas(&mut self, canvas: &CanvasWire) {
        if canvas.width == 0 || canvas.height == 0 {
            self.error(
                "canvas_extent_non_positive",
                "/document/canvas",
                None,
                details([
                    ("width", json!(canvas.width)),
                    ("height", json!(canvas.height)),
                ]),
            );
        }
        if !canvas.fps.is_positive() {
            self.error(
                "frame_rate_non_positive",
                "/document/canvas/fps",
                None,
                JsonObject::new(),
            );
        }
        if canvas.sample_rate == 0 {
            self.error(
                "sample_rate_non_positive",
                "/document/canvas/sampleRate",
                None,
                JsonObject::new(),
            );
        }
        self.require_positive(
            canvas.duration,
            "/document/canvas/duration",
            None,
            "duration_non_positive",
        );
    }

    fn validate_visual(&mut self, visual: &VisualCompositionWire, canvas: ExactRational) {
        for (track_index, track) in visual.tracks.iter().enumerate() {
            let track_path = format!("/document/visual/tracks/{track_index}");
            self.register_id(&track.id, "visual-track", &format!("{track_path}/id"));
            let mut cursor = ExactRational::ZERO;
            for (item_index, item) in track.items.iter().enumerate() {
                let path = format!("{track_path}/items/{item_index}");
                match item {
                    VisualItemWire::Clip(clip) => {
                        self.register_id(&clip.id, "visual-clip", &format!("{path}/id"));
                        self.require_positive(
                            clip.duration,
                            &format!("{path}/duration"),
                            Some(&clip.id),
                            "duration_non_positive",
                        );
                        self.validate_visual_clip(clip, &path);
                        cursor = self.advance(cursor, clip.duration, &path, Some(&clip.id));
                    }
                    VisualItemWire::Gap(gap) => {
                        self.register_id(&gap.id, "visual-gap", &format!("{path}/id"));
                        self.require_positive(
                            gap.duration,
                            &format!("{path}/duration"),
                            Some(&gap.id),
                            "duration_non_positive",
                        );
                        if item_index > 0
                            && matches!(track.items[item_index - 1], VisualItemWire::Gap(_))
                        {
                            self.error("adjacent_gap", &path, Some(&gap.id), JsonObject::new());
                        }
                        cursor = self.advance(cursor, gap.duration, &path, Some(&gap.id));
                    }
                    VisualItemWire::Transition(transition) => {
                        self.register_id(
                            &transition.id,
                            "visual-transition",
                            &format!("{path}/id"),
                        );
                        self.require_positive(
                            transition.duration,
                            &format!("{path}/duration"),
                            Some(&transition.id),
                            "duration_non_positive",
                        );
                        let valid_left = item_index.checked_sub(1).is_some_and(|index| {
                            matches!(track.items[index], VisualItemWire::Clip(_))
                        });
                        let valid_right = track
                            .items
                            .get(item_index + 1)
                            .is_some_and(|item| matches!(item, VisualItemWire::Clip(_)));
                        if !valid_left || !valid_right {
                            self.error(
                                "transition_adjacency",
                                &path,
                                Some(&transition.id),
                                JsonObject::new(),
                            );
                        }
                    }
                }
            }
            self.require_track_within_canvas(cursor, canvas, &track_path, Some(&track.id));
        }
    }

    fn validate_visual_clip(&mut self, clip: &VisualClipWire, path: &str) {
        self.validate_layer(&clip.layer, clip.duration, &format!("{path}/layer"));
        match &clip.source {
            VisualSourceWire::Video(source) => {
                self.validate_resource_id(&source.resource, &format!("{path}/source/resource"));
                self.require_non_negative(
                    source.source_start,
                    &format!("{path}/source/sourceStart"),
                    Some(&clip.id),
                );
                self.require_positive(
                    source.rate,
                    &format!("{path}/source/rate"),
                    Some(&clip.id),
                    "rate_non_positive",
                );
            }
            VisualSourceWire::Image(source) => {
                self.validate_resource_id(&source.resource, &format!("{path}/source/resource"));
            }
            VisualSourceWire::Lottie(source) => {
                self.validate_resource_id(&source.resource, &format!("{path}/source/resource"));
                self.require_non_negative(
                    source.source_start,
                    &format!("{path}/source/sourceStart"),
                    Some(&clip.id),
                );
                self.require_positive(
                    source.rate,
                    &format!("{path}/source/rate"),
                    Some(&clip.id),
                    "rate_non_positive",
                );
            }
            VisualSourceWire::Motion(source) => {
                if source.props.len() > MAX_MOTION_PROPS {
                    self.error(
                        "motion_prop_budget_exceeded",
                        &format!("{path}/source/props"),
                        Some(&clip.id),
                        details([
                            ("limit", json!(MAX_MOTION_PROPS)),
                            ("actual", json!(source.props.len())),
                        ]),
                    );
                }
                self.validate_resource_id(&source.component, &format!("{path}/source/component"));
                self.require_non_negative(
                    source.source_start,
                    &format!("{path}/source/sourceStart"),
                    Some(&clip.id),
                );
                self.require_positive(
                    source.source_duration,
                    &format!("{path}/source/sourceDuration"),
                    Some(&clip.id),
                    "duration_non_positive",
                );
                self.require_positive(
                    source.rate,
                    &format!("{path}/source/rate"),
                    Some(&clip.id),
                    "rate_non_positive",
                );
                for (name, param) in &source.props {
                    self.validate_json_param(
                        param,
                        source.source_duration,
                        &format!("{path}/source/props/{}", pointer_token(name)),
                    );
                }
                for (name, resource) in &source.resources {
                    self.validate_resource_id(
                        resource,
                        &format!("{path}/source/resources/{}", pointer_token(name)),
                    );
                }
                for (name, cue) in &source.cues {
                    let cue_path = format!("{path}/source/cues/{}", pointer_token(name));
                    if name.is_empty() || name.len() > 256 || name.chars().any(char::is_control) {
                        self.error(
                            "invalid_motion_cue_name",
                            &cue_path,
                            Some(&clip.id),
                            JsonObject::new(),
                        );
                    }
                    match cue {
                        MotionCueBindingWire::SourceRange {
                            start,
                            end,
                            enter_duration,
                            exit_duration,
                        } => {
                            self.require_non_negative(
                                *start,
                                &format!("{cue_path}/start"),
                                Some(&clip.id),
                            );
                            if *end <= *start || *end > source.source_duration {
                                self.error(
                                    "motion_cue_range_invalid",
                                    &cue_path,
                                    Some(&clip.id),
                                    JsonObject::new(),
                                );
                            }
                            self.validate_motion_phase_duration(
                                *enter_duration,
                                source.source_duration,
                                &format!("{cue_path}/enterDuration"),
                                &clip.id,
                            );
                            self.validate_motion_phase_duration(
                                *exit_duration,
                                source.source_duration,
                                &format!("{cue_path}/exitDuration"),
                                &clip.id,
                            );
                            if let (Ok(phase_total), Ok(cue_duration)) = (
                                enter_duration.checked_add(*exit_duration),
                                end.checked_sub(*start),
                            ) {
                                if phase_total > cue_duration {
                                    self.error(
                                        "motion_cue_phase_exceeds_range",
                                        &cue_path,
                                        Some(&clip.id),
                                        JsonObject::new(),
                                    );
                                }
                            } else {
                                self.error(
                                    "exact_time_overflow",
                                    &cue_path,
                                    Some(&clip.id),
                                    JsonObject::new(),
                                );
                            }
                        }
                    }
                }
                if let Some(enter) = source.phases.enter_duration {
                    self.validate_motion_phase_duration(
                        enter,
                        source.source_duration,
                        &format!("{path}/source/phases/enterDuration"),
                        &clip.id,
                    );
                }
                if let Some(exit) = source.phases.exit_duration {
                    self.validate_motion_phase_duration(
                        exit,
                        source.source_duration,
                        &format!("{path}/source/phases/exitDuration"),
                        &clip.id,
                    );
                }
                if let (Some(enter), Some(exit)) =
                    (source.phases.enter_duration, source.phases.exit_duration)
                {
                    match enter.checked_add(exit) {
                        Ok(total) if total <= source.source_duration => {}
                        Ok(_) => self.error(
                            "motion_phase_exceeds_source_duration",
                            &format!("{path}/source/phases"),
                            Some(&clip.id),
                            JsonObject::new(),
                        ),
                        Err(_) => self.error(
                            "exact_time_overflow",
                            &format!("{path}/source/phases"),
                            Some(&clip.id),
                            JsonObject::new(),
                        ),
                    }
                }
            }
            VisualSourceWire::Solid(source) => {
                if !crate::internal::Color(source.color.clone()).is_valid() {
                    self.error(
                        "invalid_color",
                        &format!("{path}/source/color"),
                        Some(&clip.id),
                        JsonObject::new(),
                    );
                }
            }
        }
    }

    fn validate_layer(&mut self, layer: &VisualLayerWire, owner: ExactRational, path: &str) {
        self.validate_vec2_param(
            &layer.transform.position,
            owner,
            &format!("{path}/transform/position"),
            |_| true,
            "invalid_position",
        );
        self.validate_scale_param(
            &layer.transform.scale,
            owner,
            &format!("{path}/transform/scale"),
        );
        self.validate_scalar_param(
            &layer.transform.rotation,
            owner,
            &format!("{path}/transform/rotation"),
            |_| true,
            "invalid_rotation",
        );
        self.validate_vec2(&layer.transform.anchor, &format!("{path}/transform/anchor"));
        self.validate_scalar_param(
            &layer.opacity,
            owner,
            &format!("{path}/opacity"),
            |value| (0.0..=1.0).contains(&value),
            "opacity_out_of_range",
        );
        if let Some(mask) = &layer.mask {
            match mask {
                LayerMaskWire::Rect { rect, feather, .. }
                | LayerMaskWire::Ellipse { rect, feather, .. } => {
                    self.validate_rect_param(rect, owner, &format!("{path}/mask/rect"));
                    self.validate_scalar_param(
                        feather,
                        owner,
                        &format!("{path}/mask/feather"),
                        |value| value >= 0.0,
                        "mask_feather_negative",
                    );
                }
            }
        }
        for (index, filter) in layer.filters.iter().enumerate() {
            self.register_id(
                &filter.id,
                "visual-filter",
                &format!("{path}/filters/{index}/id"),
            );
        }
    }

    fn validate_motion_phase_duration(
        &mut self,
        value: ExactRational,
        source_duration: ExactRational,
        path: &str,
        entity_id: &str,
    ) {
        if value.is_negative() || value > source_duration {
            self.error(
                "motion_phase_duration_out_of_range",
                path,
                Some(entity_id),
                JsonObject::new(),
            );
        }
    }

    fn validate_audio(&mut self, audio: &AudioCompositionWire, canvas: ExactRational) {
        for (track_index, track) in audio.tracks.iter().enumerate() {
            let track_path = format!("/document/audio/tracks/{track_index}");
            self.register_id(&track.id, "audio-track", &format!("{track_path}/id"));
            let mut cursor = ExactRational::ZERO;
            for (item_index, item) in track.items.iter().enumerate() {
                let path = format!("{track_path}/items/{item_index}");
                match item {
                    AudioItemWire::Clip(clip) => {
                        self.register_id(&clip.id, "audio-clip", &format!("{path}/id"));
                        self.require_positive(
                            clip.duration,
                            &format!("{path}/duration"),
                            Some(&clip.id),
                            "duration_non_positive",
                        );
                        match &clip.source {
                            AudioSourceWire::Media(source) => {
                                self.validate_resource_id(
                                    &source.resource,
                                    &format!("{path}/source/resource"),
                                );
                                self.require_non_negative(
                                    source.source_start,
                                    &format!("{path}/source/sourceStart"),
                                    Some(&clip.id),
                                );
                                self.require_positive(
                                    source.rate,
                                    &format!("{path}/source/rate"),
                                    Some(&clip.id),
                                    "rate_non_positive",
                                );
                            }
                        }
                        self.validate_scalar_param(
                            &clip.gain,
                            clip.duration,
                            &format!("{path}/gain"),
                            |value| value >= 0.0,
                            "gain_negative",
                        );
                        self.validate_scalar_param(
                            &clip.pan,
                            clip.duration,
                            &format!("{path}/pan"),
                            |value| (-1.0..=1.0).contains(&value),
                            "pan_out_of_range",
                        );
                        for (index, effect) in clip.effects.iter().enumerate() {
                            self.register_id(
                                &effect.id,
                                "audio-effect",
                                &format!("{path}/effects/{index}/id"),
                            );
                        }
                        cursor = self.advance(cursor, clip.duration, &path, Some(&clip.id));
                    }
                    AudioItemWire::Gap(gap) => {
                        self.register_id(&gap.id, "audio-gap", &format!("{path}/id"));
                        self.require_positive(
                            gap.duration,
                            &format!("{path}/duration"),
                            Some(&gap.id),
                            "duration_non_positive",
                        );
                        if item_index > 0
                            && matches!(track.items[item_index - 1], AudioItemWire::Gap(_))
                        {
                            self.error("adjacent_gap", &path, Some(&gap.id), JsonObject::new());
                        }
                        cursor = self.advance(cursor, gap.duration, &path, Some(&gap.id));
                    }
                    AudioItemWire::Crossfade(crossfade) => {
                        self.register_id(&crossfade.id, "audio-crossfade", &format!("{path}/id"));
                        self.require_positive(
                            crossfade.duration,
                            &format!("{path}/duration"),
                            Some(&crossfade.id),
                            "duration_non_positive",
                        );
                        let valid_left = item_index.checked_sub(1).is_some_and(|index| {
                            matches!(track.items[index], AudioItemWire::Clip(_))
                        });
                        let valid_right = track
                            .items
                            .get(item_index + 1)
                            .is_some_and(|item| matches!(item, AudioItemWire::Clip(_)));
                        if !valid_left || !valid_right {
                            self.error(
                                "crossfade_adjacency",
                                &path,
                                Some(&crossfade.id),
                                JsonObject::new(),
                            );
                        }
                    }
                }
            }
            self.require_track_within_canvas(cursor, canvas, &track_path, Some(&track.id));
        }
    }

    fn validate_adjustments(&mut self, effects: &[TimedAdjustmentWire], canvas: ExactRational) {
        for (index, effect) in effects.iter().enumerate() {
            let path = format!("/document/adjustments/{index}");
            self.register_id(&effect.id, "adjustment", &format!("{path}/id"));
            self.require_non_negative(effect.start, &format!("{path}/start"), Some(&effect.id));
            self.require_positive(
                effect.duration,
                &format!("{path}/duration"),
                Some(&effect.id),
                "duration_non_positive",
            );
            let end = self.advance(effect.start, effect.duration, &path, Some(&effect.id));
            if end > canvas {
                self.error(
                    "range_exceeds_canvas",
                    &path,
                    Some(&effect.id),
                    JsonObject::new(),
                );
            }
            if let AdjustmentEffectWire::ColorGrade(grade) = &effect.effect {
                self.validate_scalar(
                    grade.temperature,
                    &format!("{path}/effect/temperature"),
                    "non_finite_value",
                );
                if !(-1.0..=1.0).contains(&grade.temperature) {
                    self.error(
                        "temperature_out_of_range",
                        &format!("{path}/effect/temperature"),
                        Some(&effect.id),
                        JsonObject::new(),
                    );
                }
            }
        }
    }

    fn validate_captions(&mut self, captions: &CaptionCompositionWire, canvas: ExactRational) {
        for (track_index, track) in captions.tracks.iter().enumerate() {
            let track_path = format!("/document/captions/tracks/{track_index}");
            self.register_id(&track.id, "caption-track", &format!("{track_path}/id"));
            let mut cursor = ExactRational::ZERO;
            for (item_index, item) in track.items.iter().enumerate() {
                let path = format!("{track_path}/items/{item_index}");
                match item {
                    CaptionItemWire::Clip(caption) => {
                        self.register_id(&caption.id, "caption", &format!("{path}/id"));
                        self.require_positive(
                            caption.duration,
                            &format!("{path}/duration"),
                            Some(&caption.id),
                            "duration_non_positive",
                        );
                        if caption.runs.is_empty() {
                            self.error(
                                "caption_runs_empty",
                                &format!("{path}/runs"),
                                Some(&caption.id),
                                JsonObject::new(),
                            );
                        }
                        let karaoke = matches!(
                            caption.behavior.as_ref(),
                            Some(CaptionBehaviorWire::Karaoke { .. })
                        );
                        let mut previous_timing_end = ExactRational::ZERO;
                        for (run_index, run) in caption.runs.iter().enumerate() {
                            let run_path = format!("{path}/runs/{run_index}");
                            self.register_id(&run.id, "text-run", &format!("{run_path}/id"));
                            match &run.timing {
                                Some(timing) => {
                                    if !karaoke {
                                        self.error(
                                            "run_timing_requires_karaoke",
                                            &format!("{run_path}/timing"),
                                            Some(&run.id),
                                            JsonObject::new(),
                                        );
                                    }
                                    self.require_non_negative(
                                        timing.start,
                                        &format!("{run_path}/timing/start"),
                                        Some(&run.id),
                                    );
                                    if timing.end <= timing.start || timing.end > caption.duration {
                                        self.error(
                                            "caption_run_timing_invalid",
                                            &format!("{run_path}/timing"),
                                            Some(&run.id),
                                            JsonObject::new(),
                                        );
                                    }
                                    if karaoke
                                        && run_index > 0
                                        && timing.start < previous_timing_end
                                    {
                                        self.error(
                                            "caption_run_timing_overlap",
                                            &format!("{run_path}/timing/start"),
                                            Some(&run.id),
                                            JsonObject::new(),
                                        );
                                    }
                                    previous_timing_end = timing.end;
                                }
                                None if karaoke => self.error(
                                    "karaoke_run_timing_required",
                                    &format!("{run_path}/timing"),
                                    Some(&run.id),
                                    JsonObject::new(),
                                ),
                                None => {}
                            }
                            if let Some(style) = &run.style {
                                if style.font_size.is_none() && style.color.is_none() {
                                    self.error(
                                        "caption_run_style_empty",
                                        &format!("{run_path}/style"),
                                        Some(&run.id),
                                        JsonObject::new(),
                                    );
                                }
                                if let Some(font_size) = style.font_size
                                    && !(font_size.is_finite() && font_size > 0.0)
                                {
                                    self.error(
                                        "font_size_non_positive",
                                        &format!("{run_path}/style/fontSize"),
                                        Some(&run.id),
                                        JsonObject::new(),
                                    );
                                }
                                if style
                                    .font_size
                                    .is_some_and(|font_size| font_size > MAX_CAPTION_FONT_SIZE)
                                {
                                    self.error(
                                        "font_size_out_of_range",
                                        &format!("{run_path}/style/fontSize"),
                                        Some(&run.id),
                                        details([("maximum", json!(MAX_CAPTION_FONT_SIZE))]),
                                    );
                                }
                                if let Some(color) = &style.color
                                    && !crate::internal::Color(color.clone()).is_valid()
                                {
                                    self.error(
                                        "invalid_color",
                                        &format!("{run_path}/style/color"),
                                        Some(&run.id),
                                        JsonObject::new(),
                                    );
                                }
                            }
                        }
                        self.validate_resource_id(
                            &caption.style.font,
                            &format!("{path}/style/font"),
                        );
                        self.validate_scalar(
                            caption.style.font_size,
                            &format!("{path}/style/fontSize"),
                            "font_size_non_positive",
                        );
                        if !(caption.style.font_size.is_finite() && caption.style.font_size > 0.0) {
                            self.error(
                                "font_size_non_positive",
                                &format!("{path}/style/fontSize"),
                                Some(&caption.id),
                                JsonObject::new(),
                            );
                        }
                        if caption.style.font_size > MAX_CAPTION_FONT_SIZE {
                            self.error(
                                "font_size_out_of_range",
                                &format!("{path}/style/fontSize"),
                                Some(&caption.id),
                                details([("maximum", json!(MAX_CAPTION_FONT_SIZE))]),
                            );
                        }
                        if !crate::internal::Color(caption.style.color.clone()).is_valid() {
                            self.error(
                                "invalid_color",
                                &format!("{path}/style/color"),
                                Some(&caption.id),
                                JsonObject::new(),
                            );
                        }
                        if let Some(shadow) = &caption.style.shadow {
                            if !crate::internal::Color(shadow.color.clone()).is_valid() {
                                self.error(
                                    "invalid_color",
                                    &format!("{path}/style/shadow/color"),
                                    Some(&caption.id),
                                    JsonObject::new(),
                                );
                            }
                            self.validate_vec2(
                                &shadow.offset,
                                &format!("{path}/style/shadow/offset"),
                            );
                            if !(shadow.blur_sigma.is_finite()
                                && (0.0..=MAX_CAPTION_BLUR_SIGMA).contains(&shadow.blur_sigma))
                            {
                                self.error(
                                    "caption_shadow_blur_sigma_out_of_range",
                                    &format!("{path}/style/shadow/blurSigma"),
                                    Some(&caption.id),
                                    details([
                                        ("value", json!(shadow.blur_sigma)),
                                        ("maximum", json!(MAX_CAPTION_BLUR_SIGMA)),
                                    ]),
                                );
                            }
                        }
                        self.validate_caption_region(
                            &caption.layout.region,
                            &format!("{path}/layout/region"),
                        );
                        self.validate_scalar_param(
                            &caption.presentation.opacity,
                            caption.duration,
                            &format!("{path}/presentation/opacity"),
                            |value| (0.0..=1.0).contains(&value),
                            "opacity_out_of_range",
                        );
                        self.validate_vec2_param(
                            &caption.presentation.translation,
                            caption.duration,
                            &format!("{path}/presentation/translation"),
                            |_| true,
                            "invalid_caption_translation",
                        );
                        self.validate_scalar_param(
                            &caption.presentation.scale,
                            caption.duration,
                            &format!("{path}/presentation/scale"),
                            |value| value >= 0.0,
                            "caption_scale_negative",
                        );
                        self.validate_scalar_param(
                            &caption.presentation.rotation,
                            caption.duration,
                            &format!("{path}/presentation/rotation"),
                            |_| true,
                            "invalid_caption_rotation",
                        );
                        self.validate_param(
                            &caption.presentation.clip_inset,
                            caption.duration,
                            &format!("{path}/presentation/clipInset"),
                            |validator, value, value_path| {
                                if value
                                    .iter()
                                    .any(|part| !part.is_finite() || !(0.0..=1.0).contains(part))
                                {
                                    validator.error(
                                        "caption_clip_inset_out_of_range",
                                        value_path,
                                        None,
                                        JsonObject::new(),
                                    );
                                }
                            },
                        );
                        self.validate_scalar_param(
                            &caption.presentation.blur_sigma,
                            caption.duration,
                            &format!("{path}/presentation/blurSigma"),
                            |value| (0.0..=MAX_CAPTION_BLUR_SIGMA).contains(&value),
                            "caption_blur_sigma_out_of_range",
                        );
                        if let Some(CaptionBehaviorWire::Scroll { speed, .. }) = caption.behavior {
                            self.validate_scalar(
                                speed,
                                &format!("{path}/behavior/speed"),
                                "non_finite_value",
                            );
                            if speed == 0.0 {
                                self.error(
                                    "scroll_speed_zero",
                                    &format!("{path}/behavior/speed"),
                                    Some(&caption.id),
                                    JsonObject::new(),
                                );
                            }
                        }
                        cursor = self.advance(cursor, caption.duration, &path, Some(&caption.id));
                    }
                    CaptionItemWire::Gap(gap) => {
                        self.register_id(&gap.id, "caption-gap", &format!("{path}/id"));
                        self.require_positive(
                            gap.duration,
                            &format!("{path}/duration"),
                            Some(&gap.id),
                            "duration_non_positive",
                        );
                        if item_index > 0
                            && matches!(track.items[item_index - 1], CaptionItemWire::Gap(_))
                        {
                            self.error("adjacent_gap", &path, Some(&gap.id), JsonObject::new());
                        }
                        cursor = self.advance(cursor, gap.duration, &path, Some(&gap.id));
                    }
                }
            }
            self.require_track_within_canvas(cursor, canvas, &track_path, Some(&track.id));
        }
    }

    fn validate_camera(&mut self, camera: &CameraTrackWire, owner: ExactRational) {
        self.validate_scalar_param(
            &camera.center_x,
            owner,
            "/document/camera/centerX",
            |_| true,
            "invalid_camera_center",
        );
        self.validate_scalar_param(
            &camera.center_y,
            owner,
            "/document/camera/centerY",
            |_| true,
            "invalid_camera_center",
        );
        self.validate_scalar_param(
            &camera.zoom,
            owner,
            "/document/camera/zoom",
            |value| value > 0.0,
            "camera_zoom_non_positive",
        );
        self.validate_scalar_param(
            &camera.rotation,
            owner,
            "/document/camera/rotation",
            |_| true,
            "invalid_camera_rotation",
        );
    }

    fn validate_scalar_param<F>(
        &mut self,
        param: &ParamWire<f64>,
        owner: ExactRational,
        path: &str,
        predicate: F,
        code: &'static str,
    ) where
        F: Fn(f64) -> bool,
    {
        self.validate_param(param, owner, path, |validator, value, value_path| {
            if !value.is_finite() || !predicate(*value) {
                validator.error(code, value_path, None, details([("value", json!(value))]));
            }
        });
    }

    fn validate_vec2_param<F>(
        &mut self,
        param: &ParamWire<Vec2>,
        owner: ExactRational,
        path: &str,
        predicate: F,
        code: &'static str,
    ) where
        F: Fn(&Vec2) -> bool,
    {
        self.validate_param(param, owner, path, |validator, value, value_path| {
            if value.iter().any(|part| !part.is_finite()) || !predicate(value) {
                validator.error(code, value_path, None, JsonObject::new());
            }
        });
    }

    fn validate_scale_param(&mut self, param: &ParamWire<Vec2>, owner: ExactRational, path: &str) {
        self.validate_vec2_param(
            param,
            owner,
            path,
            |value| value[0] != 0.0 && value[1] != 0.0,
            "invalid_scale",
        );

        let ParamWire::Curve(curve) = param else {
            return;
        };
        if curve.interpolation != InterpolationWire::Linear {
            return;
        }
        for (segment, pair) in curve.keyframes.windows(2).enumerate() {
            for axis in 0..2 {
                let from = pair[0].value[axis];
                let to = pair[1].value[axis];
                if from.is_finite()
                    && to.is_finite()
                    && from != 0.0
                    && to != 0.0
                    && from.is_sign_positive() != to.is_sign_positive()
                {
                    self.error(
                        "scale_curve_crosses_zero",
                        &format!("{path}/keyframes/{}/value/{axis}", segment + 1),
                        Some(&pair[1].id),
                        details([
                            ("axis", json!(axis)),
                            ("from", json!(from)),
                            ("to", json!(to)),
                        ]),
                    );
                }
            }
        }
    }

    fn validate_rect_param(&mut self, param: &ParamWire<Rect>, owner: ExactRational, path: &str) {
        self.validate_param(param, owner, path, |validator, value, value_path| {
            validator.validate_mask_rect(value, value_path);
        });
    }

    fn validate_json_param(
        &mut self,
        param: &ParamWire<JsonValue>,
        owner: ExactRational,
        path: &str,
    ) {
        self.validate_param(param, owner, path, |_, _, _| {});
    }

    fn validate_param<T, F>(
        &mut self,
        param: &ParamWire<T>,
        owner: ExactRational,
        path: &str,
        mut validate_value: F,
    ) where
        F: FnMut(&mut Self, &T, &str),
    {
        match param {
            ParamWire::Constant(constant) => {
                validate_value(self, &constant.value, &format!("{path}/value"));
            }
            ParamWire::Curve(curve) => {
                self.register_id(&curve.id, "curve", &format!("{path}/id"));
                if curve.keyframes.len() > MAX_KEYFRAMES_PER_CURVE {
                    self.error(
                        "keyframe_budget_exceeded",
                        &format!("{path}/keyframes"),
                        Some(&curve.id),
                        details([
                            ("limit", json!(MAX_KEYFRAMES_PER_CURVE)),
                            ("actual", json!(curve.keyframes.len())),
                        ]),
                    );
                }
                if curve.keyframes.is_empty() {
                    self.error(
                        "curve_keyframes_empty",
                        &format!("{path}/keyframes"),
                        Some(&curve.id),
                        JsonObject::new(),
                    );
                    return;
                }
                let mut previous = None;
                let last = curve.keyframes.len() - 1;
                for (index, keyframe) in curve.keyframes.iter().enumerate() {
                    let keyframe_path = format!("{path}/keyframes/{index}");
                    self.register_id(&keyframe.id, "keyframe", &format!("{keyframe_path}/id"));
                    if keyframe.time.is_negative() || keyframe.time > owner {
                        self.error(
                            "keyframe_time_out_of_owner_range",
                            &format!("{keyframe_path}/time"),
                            Some(&keyframe.id),
                            JsonObject::new(),
                        );
                    }
                    if previous.is_some_and(|time| keyframe.time <= time) {
                        self.error(
                            "keyframe_time_not_strictly_increasing",
                            &format!("{keyframe_path}/time"),
                            Some(&keyframe.id),
                            JsonObject::new(),
                        );
                    }
                    if index == last && keyframe.out_easing.is_some() {
                        self.error(
                            "last_keyframe_out_easing_not_null",
                            &format!("{keyframe_path}/outEasing"),
                            Some(&keyframe.id),
                            JsonObject::new(),
                        );
                    }
                    validate_value(self, &keyframe.value, &format!("{keyframe_path}/value"));
                    previous = Some(keyframe.time);
                }
            }
        }
    }

    fn validate_vec2(&mut self, value: &Vec2, path: &str) {
        if value.iter().any(|part| !part.is_finite()) {
            self.error("non_finite_value", path, None, JsonObject::new());
        }
    }

    fn validate_mask_rect(&mut self, value: &Rect, path: &str) {
        if value.iter().any(|part| !part.is_finite()) {
            self.error("non_finite_value", path, None, JsonObject::new());
            return;
        }
        if value[2] <= 0.0 || value[3] <= 0.0 {
            self.error(
                "mask_rect_non_positive_extent",
                path,
                None,
                details([("width", json!(value[2])), ("height", json!(value[3]))]),
            );
        }
    }

    fn validate_caption_region(&mut self, value: &Rect, path: &str) {
        if value.iter().any(|part| !part.is_finite()) {
            self.error("non_finite_value", path, None, JsonObject::new());
            return;
        }
        let [x, y, width, height] = *value;
        if x < 0.0
            || y < 0.0
            || width <= 0.0
            || height <= 0.0
            || x + width > 1.0
            || y + height > 1.0
        {
            self.error(
                "caption_region_out_of_bounds",
                path,
                None,
                details([
                    ("x", json!(x)),
                    ("y", json!(y)),
                    ("width", json!(width)),
                    ("height", json!(height)),
                ]),
            );
        }
    }

    fn validate_scalar(&mut self, value: f64, path: &str, code: &'static str) {
        if !value.is_finite() {
            self.error(code, path, None, JsonObject::new());
        }
    }

    fn validate_resource_id(&mut self, id: &str, path: &str) {
        let valid = id.len() <= 256
            && id.split_once(':').is_some_and(|(namespace, name)| {
                !namespace.is_empty()
                    && !name.is_empty()
                    && !namespace.contains('/')
                    && !id.contains("://")
                    && !id.chars().any(|ch| ch.is_control() || ch.is_whitespace())
            });
        if !valid {
            self.error(
                "invalid_resource_id",
                path,
                None,
                details([("resourceId", json!(id))]),
            );
        }
    }

    fn register_id(&mut self, id: &str, kind: &'static str, path: &str) {
        if id.is_empty() || id.len() > 128 || id.chars().any(|character| character.is_control()) {
            self.error(
                "invalid_entity_id",
                path,
                Some(id),
                details([("kind", json!(kind))]),
            );
            return;
        }
        if let Some((first_kind, first_path)) = self.ids.get(id).cloned() {
            self.error(
                "duplicate_entity_id",
                path,
                Some(id),
                details([
                    ("kind", json!(kind)),
                    ("firstKind", json!(first_kind)),
                    ("firstPath", json!(first_path)),
                ]),
            );
        } else {
            self.ids.insert(id.to_owned(), (kind, path.to_owned()));
        }
    }

    fn require_positive(
        &mut self,
        value: ExactRational,
        path: &str,
        id: Option<&str>,
        code: &'static str,
    ) {
        if !value.is_positive() {
            self.error(
                code,
                path,
                id,
                details([("value", json!(value.to_string()))]),
            );
        }
    }

    fn require_non_negative(&mut self, value: ExactRational, path: &str, id: Option<&str>) {
        if value.is_negative() {
            self.error(
                "time_negative",
                path,
                id,
                details([("value", json!(value.to_string()))]),
            );
        }
    }

    fn advance(
        &mut self,
        cursor: ExactRational,
        duration: ExactRational,
        path: &str,
        id: Option<&str>,
    ) -> ExactRational {
        match cursor.checked_add(duration) {
            Ok(next) => next,
            Err(_) => {
                self.error("time_overflow", path, id, JsonObject::new());
                cursor
            }
        }
    }

    fn require_track_within_canvas(
        &mut self,
        duration: ExactRational,
        canvas: ExactRational,
        path: &str,
        id: Option<&str>,
    ) {
        if duration > canvas {
            self.error(
                "track_exceeds_canvas",
                path,
                id,
                details([
                    ("trackDuration", json!(duration.to_string())),
                    ("canvasDuration", json!(canvas.to_string())),
                ]),
            );
        }
    }

    fn error(
        &mut self,
        code: impl Into<String>,
        path: impl Into<String>,
        entity_id: Option<&str>,
        details: JsonObject,
    ) {
        self.diagnostics.push(ContractDiagnostic {
            code: code.into(),
            path: path.into(),
            entity_id: entity_id.map(str::to_owned),
            phase: DiagnosticPhase::LocalInvariant,
            severity: DiagnosticSeverity::Error,
            details,
        });
    }
}

fn details<const N: usize>(entries: [(&str, JsonValue); N]) -> JsonObject {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

fn pointer_token(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}
