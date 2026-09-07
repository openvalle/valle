use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use valle_draw::Rgba;

use crate::phases::PhaseSpec;
use crate::value::MotionValue;

use super::artifact::ValidationError;

/// Hard budgets for data that is frozen and expanded during Motion prepare.
pub const MAX_PREPARE_DATA_DEPTH: usize = 16;
pub const MAX_PREPARE_DATA_ARRAY_ITEMS: usize = 10_000;
pub const MAX_PREPARE_DATA_TOTAL_ITEMS: usize = 50_000;
pub const MAX_PREPARE_DATA_BYTES: usize = 1024 * 1024;

/// Schema for JSON values admitted through `controls.data`.
///
/// This deliberately stays separate from [`ControlType`]: data shapes disappear during prepare,
/// while prop controls remain addressable frame-time values in the Artifact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind",
    deny_unknown_fields
)]
pub enum PrepareDataType {
    Number {
        min: Option<f64>,
        max: Option<f64>,
    },
    String {
        min_bytes: Option<usize>,
        max_bytes: Option<usize>,
    },
    Bool,
    Color,
    /// JSON representation is exactly `[x, y]`, with two finite numbers.
    Point,
    Array {
        items: Box<PrepareDataType>,
        min_items: usize,
        max_items: usize,
        /// Optional record field whose scalar value must be unique in the array.
        key: Option<String>,
    },
    Record {
        fields: BTreeMap<String, PrepareDataType>,
    },
    Tuple {
        items: Vec<PrepareDataType>,
    },
}

impl PrepareDataType {
    fn validate_schema(&self, path: &str, depth: usize, errors: &mut Vec<ValidationError>) {
        if depth > MAX_PREPARE_DATA_DEPTH {
            errors.push(ValidationError::new(
                path,
                format!("data schema nesting exceeds {MAX_PREPARE_DATA_DEPTH}"),
            ));
            return;
        }
        match self {
            PrepareDataType::Number { min, max } => {
                for (name, value) in [("min", min), ("max", max)] {
                    if value.is_some_and(|value| !value.is_finite()) {
                        errors.push(ValidationError::new(
                            format!("{path}/{name}"),
                            "number bound must be finite",
                        ));
                    }
                }
                if min.zip(*max).is_some_and(|(min, max)| min > max) {
                    errors.push(ValidationError::new(path, "number min must not exceed max"));
                }
            }
            PrepareDataType::String {
                min_bytes,
                max_bytes,
            } => {
                if min_bytes
                    .zip(*max_bytes)
                    .is_some_and(|(min, max)| min > max)
                {
                    errors.push(ValidationError::new(
                        path,
                        "string minBytes must not exceed maxBytes",
                    ));
                }
                if max_bytes.is_some_and(|max| max > MAX_PREPARE_DATA_BYTES) {
                    errors.push(ValidationError::new(
                        format!("{path}/maxBytes"),
                        format!("string maxBytes must not exceed {MAX_PREPARE_DATA_BYTES}"),
                    ));
                }
            }
            PrepareDataType::Array {
                items,
                min_items,
                max_items,
                key,
            } => {
                if min_items > max_items {
                    errors.push(ValidationError::new(
                        path,
                        "array minItems must not exceed maxItems",
                    ));
                }
                if *max_items > MAX_PREPARE_DATA_ARRAY_ITEMS {
                    errors.push(ValidationError::new(
                        format!("{path}/maxItems"),
                        format!("array maxItems must not exceed {MAX_PREPARE_DATA_ARRAY_ITEMS}"),
                    ));
                }
                if let Some(key) = key {
                    let valid_key = matches!(
                        items.as_ref(),
                        PrepareDataType::Record { fields }
                            if matches!(
                                fields.get(key),
                                Some(
                                    PrepareDataType::String { .. }
                                        | PrepareDataType::Number { .. }
                                        | PrepareDataType::Bool
                                )
                            )
                    );
                    if !valid_key {
                        errors.push(ValidationError::new(
                            format!("{path}/key"),
                            "array key must name a string, number, or bool field on its record item",
                        ));
                    }
                }
                items.validate_schema(&format!("{path}/items"), depth + 1, errors);
            }
            PrepareDataType::Record { fields } => {
                validate_names(&format!("{path}/fields"), fields.keys(), errors);
                if fields.is_empty() {
                    errors.push(ValidationError::new(
                        format!("{path}/fields"),
                        "record fields must not be empty",
                    ));
                }
                for (name, field) in fields {
                    field.validate_schema(&format!("{path}/fields/{name}"), depth + 1, errors);
                }
            }
            PrepareDataType::Tuple { items } => {
                if items.is_empty() {
                    errors.push(ValidationError::new(
                        format!("{path}/items"),
                        "tuple items must not be empty",
                    ));
                }
                for (index, item) in items.iter().enumerate() {
                    item.validate_schema(&format!("{path}/items/{index}"), depth + 1, errors);
                }
            }
            PrepareDataType::Bool | PrepareDataType::Color | PrepareDataType::Point => {}
        }
    }

    fn validate_value(
        &self,
        value: &Value,
        path: &str,
        depth: usize,
        total_items: &mut usize,
        errors: &mut Vec<ValidationError>,
    ) {
        if depth > MAX_PREPARE_DATA_DEPTH {
            errors.push(ValidationError::new(
                path,
                format!("data nesting exceeds {MAX_PREPARE_DATA_DEPTH}"),
            ));
            return;
        }
        *total_items = total_items.saturating_add(1);
        if *total_items > MAX_PREPARE_DATA_TOTAL_ITEMS {
            if *total_items == MAX_PREPARE_DATA_TOTAL_ITEMS + 1 {
                errors.push(ValidationError::new(
                    path,
                    format!("data contains more than {MAX_PREPARE_DATA_TOTAL_ITEMS} values"),
                ));
            }
            return;
        }
        match self {
            PrepareDataType::Number { min, max } => match value.as_f64() {
                Some(number) if number.is_finite() => {
                    if min.is_some_and(|min| number < min) || max.is_some_and(|max| number > max) {
                        errors.push(ValidationError::new(path, "number is outside its bounds"));
                    }
                }
                _ => errors.push(ValidationError::new(path, "expected a finite number")),
            },
            PrepareDataType::String {
                min_bytes,
                max_bytes,
            } => match value.as_str() {
                Some(text) => {
                    let len = text.len();
                    if min_bytes.is_some_and(|min| len < min)
                        || max_bytes.is_some_and(|max| len > max)
                    {
                        errors.push(ValidationError::new(
                            path,
                            "string byte length is outside its bounds",
                        ));
                    }
                }
                None => errors.push(ValidationError::new(path, "expected a string")),
            },
            PrepareDataType::Bool => {
                if !value.is_boolean() {
                    errors.push(ValidationError::new(path, "expected a boolean"));
                }
            }
            PrepareDataType::Color => match value.as_str() {
                Some(color) if Rgba::parse(color).is_some() => {}
                _ => errors.push(ValidationError::new(path, "expected a valid color string")),
            },
            PrepareDataType::Point => match value.as_array() {
                Some(point)
                    if point.len() == 2
                        && point
                            .iter()
                            .all(|value| value.as_f64().is_some_and(f64::is_finite)) => {}
                _ => errors.push(ValidationError::new(
                    path,
                    "expected a point encoded as [x, y] with finite numbers",
                )),
            },
            PrepareDataType::Array {
                items,
                min_items,
                max_items,
                key,
            } => match value.as_array() {
                Some(values) => {
                    if values.len() < *min_items || values.len() > *max_items {
                        errors.push(ValidationError::new(
                            path,
                            format!(
                                "array length {} is outside {min_items}..={max_items}",
                                values.len()
                            ),
                        ));
                    }
                    if let Some(key) = key {
                        let mut seen = BTreeMap::<String, usize>::new();
                        for (index, value) in values.iter().enumerate() {
                            if let Some(key_value) =
                                value.as_object().and_then(|item| item.get(key))
                            {
                                let identity = match key_value {
                                    Value::String(value) => Some(format!("s:{value}")),
                                    Value::Number(value) => Some(format!("n:{value}")),
                                    Value::Bool(value) => Some(format!("b:{value}")),
                                    _ => None,
                                };
                                if let Some(identity) = identity
                                    && let Some(previous) = seen.insert(identity, index)
                                {
                                    errors.push(ValidationError::new(
                                        format!("{path}/{index}/{key}"),
                                        format!("array key `{key}` duplicates item {previous}"),
                                    ));
                                }
                            }
                        }
                    }
                    for (index, value) in values.iter().enumerate() {
                        items.validate_value(
                            value,
                            &format!("{path}/{index}"),
                            depth + 1,
                            total_items,
                            errors,
                        );
                    }
                }
                None => errors.push(ValidationError::new(path, "expected an array")),
            },
            PrepareDataType::Record { fields } => match value.as_object() {
                Some(object) => {
                    for name in object.keys() {
                        if !fields.contains_key(name) {
                            errors.push(ValidationError::new(
                                format!("{path}/{name}"),
                                "unknown record field",
                            ));
                        }
                    }
                    for (name, field) in fields {
                        match object.get(name) {
                            Some(value) => field.validate_value(
                                value,
                                &format!("{path}/{name}"),
                                depth + 1,
                                total_items,
                                errors,
                            ),
                            None => errors.push(ValidationError::new(
                                format!("{path}/{name}"),
                                "required record field is missing",
                            )),
                        }
                    }
                }
                None => errors.push(ValidationError::new(path, "expected an object")),
            },
            PrepareDataType::Tuple { items } => match value.as_array() {
                Some(values) if values.len() == items.len() => {
                    for (index, (schema, value)) in items.iter().zip(values).enumerate() {
                        schema.validate_value(
                            value,
                            &format!("{path}/{index}"),
                            depth + 1,
                            total_items,
                            errors,
                        );
                    }
                }
                Some(values) => errors.push(ValidationError::new(
                    path,
                    format!(
                        "tuple length {} does not match schema length {}",
                        values.len(),
                        items.len()
                    ),
                )),
                None => errors.push(ValidationError::new(path, "expected a tuple array")),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind",
    deny_unknown_fields
)]
pub enum ControlType {
    Number {
        min: Option<f64>,
        max: Option<f64>,
        step: Option<f64>,
    },
    String,
    Bool,
    Color,
    Length,
    Angle,
    Point,
    Rect,
    PathData,
    /// Stable authored node address for scene-camera controls.
    NodeTarget,
    Select {
        values: Vec<String>,
    },
}

impl ControlType {
    pub(crate) fn accepts(&self, value: &MotionValue) -> bool {
        matches!(
            (self, value),
            (ControlType::Number { .. }, MotionValue::Number(_))
                | (ControlType::String, MotionValue::Str(_))
                | (ControlType::Bool, MotionValue::Bool(_))
                | (ControlType::Color, MotionValue::Color(_))
                | (ControlType::Length, MotionValue::Length(_))
                | (ControlType::Angle, MotionValue::Angle(_))
                | (ControlType::Point, MotionValue::Point(_))
                | (ControlType::Rect, MotionValue::Rect(_))
                | (ControlType::NodeTarget, MotionValue::Str(_))
        ) || matches!((self, value), (ControlType::Select { values }, MotionValue::Enum(v)) if values.contains(v))
            || matches!(
                (self, value),
                (ControlType::PathData, MotionValue::PathData(path))
                    if path.points.len() <= crate::geometry::MAX_FRAME_GEOMETRY_POINTS
            )
    }

    pub(crate) fn validate(&self, path: &str, errors: &mut Vec<ValidationError>) {
        match self {
            ControlType::Number { min, max, step } => {
                for (name, value) in [("min", min), ("max", max), ("step", step)] {
                    if value.is_some_and(|value| !value.is_finite()) {
                        errors.push(ValidationError::new(
                            format!("{path}/{name}"),
                            "number bound must be finite",
                        ));
                    }
                }
                if (*min).zip(*max).is_some_and(|(min, max)| min > max) {
                    errors.push(ValidationError::new(path, "number min must not exceed max"));
                }
                if step.is_some_and(|step| step <= 0.0) {
                    errors.push(ValidationError::new(path, "number step must be positive"));
                }
            }
            ControlType::Select { values } => {
                if values.is_empty() {
                    errors.push(ValidationError::new(
                        path,
                        "select values must not be empty",
                    ));
                }
                let mut sorted = values.clone();
                sorted.sort();
                sorted.dedup();
                if sorted.len() != values.len() || values.iter().any(String::is_empty) {
                    errors.push(ValidationError::new(
                        path,
                        "select values must be non-empty and unique",
                    ));
                }
            }
            _ => {}
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PropControl {
    pub control: ControlType,
    pub default: Option<MotionValue>,
    #[serde(default)]
    pub required: bool,
    pub label: Option<String>,
}

impl PropControl {
    fn validate(&self, path: &str, errors: &mut Vec<ValidationError>) {
        self.control.validate(&format!("{path}/control"), errors);
        if let Some(default) = &self.default {
            if !default.is_finite() {
                errors.push(ValidationError::new(
                    format!("{path}/default"),
                    "default value must be finite",
                ));
            } else if !self.control.accepts(default) {
                errors.push(ValidationError::new(
                    format!("{path}/default"),
                    "default value does not match the control type",
                ));
            }
            if let (ControlType::Number { min, max, .. }, MotionValue::Number(value)) =
                (&self.control, default)
                && (min.is_some_and(|min| *value < min) || max.is_some_and(|max| *value > max))
            {
                errors.push(ValidationError::new(
                    format!("{path}/default"),
                    "number default is outside its bounds",
                ));
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FrameControl {
    pub default: u32,
    pub min: u32,
    pub max: Option<u32>,
}

impl FrameControl {
    fn validate(&self, path: &str, errors: &mut Vec<ValidationError>) {
        if self.max.is_some_and(|max| self.min > max)
            || self.default < self.min
            || self.max.is_some_and(|max| self.default > max)
        {
            errors.push(ValidationError::new(
                path,
                "frame default must lie inside min/max",
            ));
        }
    }

    fn resolve(self, field: &'static str, override_value: Option<u32>) -> Result<u32, TimingError> {
        let value = override_value.unwrap_or(self.default);
        if value < self.min || self.max.is_some_and(|max| value > max) {
            return Err(TimingError {
                field,
                value,
                min: self.min,
                max: self.max,
            });
        }
        Ok(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimingError {
    pub field: &'static str,
    pub value: u32,
    pub min: u32,
    pub max: Option<u32>,
}

impl core::fmt::Display for TimingError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.max {
            Some(max) => write!(
                f,
                "{} override {} is outside {}..={}",
                self.field, self.value, self.min, max
            ),
            None => write!(
                f,
                "{} override {} is below minimum {}",
                self.field, self.value, self.min
            ),
        }
    }
}

impl std::error::Error for TimingError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OptionalFrameControl {
    pub default: Option<u32>,
    pub min: u32,
    pub max: Option<u32>,
}

impl OptionalFrameControl {
    fn validate(&self, path: &str, errors: &mut Vec<ValidationError>) {
        if self.min == 0 {
            errors.push(ValidationError::new(
                format!("{path}/min"),
                "optional frame minimum must be positive",
            ));
        }
        if self.max.is_some_and(|max| self.min > max)
            || self.default.is_some_and(|value| value < self.min)
            || self
                .default
                .zip(self.max)
                .is_some_and(|(value, max)| value > max)
        {
            errors.push(ValidationError::new(
                path,
                "optional frame default must lie inside min/max",
            ));
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TimingControls {
    pub enter_frames: FrameControl,
    pub hold_cycle_frames: OptionalFrameControl,
    pub exit_frames: FrameControl,
}

impl TimingControls {
    pub fn phase_spec(&self) -> PhaseSpec {
        PhaseSpec {
            enter_frames: self.enter_frames.default,
            exit_frames: self.exit_frames.default,
            hold_cycle_frames: self.hold_cycle_frames.default,
        }
    }

    pub fn resolve_phase_spec(
        &self,
        enter_frames: Option<u32>,
        exit_frames: Option<u32>,
    ) -> Result<PhaseSpec, TimingError> {
        Ok(PhaseSpec {
            enter_frames: self.enter_frames.resolve("enterFrames", enter_frames)?,
            exit_frames: self.exit_frames.resolve("exitFrames", exit_frames)?,
            hold_cycle_frames: self.hold_cycle_frames.default,
        })
    }

    fn validate(&self, errors: &mut Vec<ValidationError>) {
        self.enter_frames
            .validate("/controls/timing/enterFrames", errors);
        self.hold_cycle_frames
            .validate("/controls/timing/holdCycleFrames", errors);
        self.exit_frames
            .validate("/controls/timing/exitFrames", errors);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CueKind {
    Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CueControl {
    pub kind: CueKind,
    pub required: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AssetKind {
    Image,
    Audio,
    Video,
    Font,
    Model3d,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssetControl {
    pub kind: AssetKind,
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CameraControls {
    pub values: BTreeMap<String, PropControl>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ControlsSchema {
    pub props: BTreeMap<String, PropControl>,
    /// Structured JSON frozen and expanded at prepare time. It never becomes frame-time IR.
    pub data: BTreeMap<String, PrepareDataType>,
    pub timing: TimingControls,
    pub cues: BTreeMap<String, CueControl>,
    pub assets: BTreeMap<String, AssetControl>,
    pub camera: CameraControls,
}

impl ControlsSchema {
    pub fn phase_spec(&self) -> PhaseSpec {
        self.timing.phase_spec()
    }

    pub fn phase_spec_with_overrides(
        &self,
        enter_frames: Option<u32>,
        exit_frames: Option<u32>,
    ) -> Result<PhaseSpec, TimingError> {
        self.timing.resolve_phase_spec(enter_frames, exit_frames)
    }

    pub(crate) fn validate(&self, errors: &mut Vec<ValidationError>) {
        self.timing.validate(errors);
        validate_names("/controls/props", self.props.keys(), errors);
        validate_names("/controls/data", self.data.keys(), errors);
        validate_names("/controls/cues", self.cues.keys(), errors);
        validate_names("/controls/assets", self.assets.keys(), errors);
        validate_names("/controls/camera", self.camera.values.keys(), errors);
        for (name, control) in &self.props {
            control.validate(&format!("/controls/props/{name}"), errors);
        }
        for (name, schema) in &self.data {
            schema.validate_schema(&format!("/controls/data/{name}"), 1, errors);
        }
        for (name, control) in &self.camera.values {
            control.validate(&format!("/controls/camera/{name}"), errors);
        }
    }

    /// Validate a complete external binding against `controls.data` and all prepare budgets.
    pub fn validate_data(&self, value: &Value) -> Result<(), Vec<ValidationError>> {
        let mut errors = Vec::new();
        let Some(object) = value.as_object() else {
            return Err(vec![ValidationError::new(
                "/data",
                "controls.data binding must be a JSON object",
            )]);
        };
        let bytes = crate::canonical_bytes(value).map_or(usize::MAX, |bytes| bytes.len());
        if bytes > MAX_PREPARE_DATA_BYTES {
            errors.push(ValidationError::new(
                "/data",
                format!("canonical data size {bytes} exceeds {MAX_PREPARE_DATA_BYTES} bytes"),
            ));
        }
        for name in object.keys() {
            if !self.data.contains_key(name) {
                errors.push(ValidationError::new(
                    format!("/data/{name}"),
                    "binding is not declared in controls.data",
                ));
            }
        }
        let mut total_items = 0;
        for (name, schema) in &self.data {
            match object.get(name) {
                Some(value) => schema.validate_value(
                    value,
                    &format!("/data/{name}"),
                    1,
                    &mut total_items,
                    &mut errors,
                ),
                None => errors.push(ValidationError::new(
                    format!("/data/{name}"),
                    "required data binding is missing",
                )),
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

fn validate_names<'a>(
    path: &str,
    names: impl Iterator<Item = &'a String>,
    errors: &mut Vec<ValidationError>,
) {
    for name in names {
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            errors.push(ValidationError::new(
                format!("{path}/{name}"),
                "control name must be a non-empty ASCII identifier",
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn data_controls() -> ControlsSchema {
        ControlsSchema {
            props: BTreeMap::new(),
            data: BTreeMap::from([(
                "rows".into(),
                PrepareDataType::Array {
                    items: Box::new(PrepareDataType::Record {
                        fields: BTreeMap::from([
                            (
                                "id".into(),
                                PrepareDataType::String {
                                    min_bytes: Some(1),
                                    max_bytes: Some(32),
                                },
                            ),
                            (
                                "value".into(),
                                PrepareDataType::Number {
                                    min: Some(0.0),
                                    max: Some(100.0),
                                },
                            ),
                        ]),
                    }),
                    min_items: 1,
                    max_items: 8,
                    key: Some("id".into()),
                },
            )]),
            timing: TimingControls {
                enter_frames: FrameControl {
                    default: 0,
                    min: 0,
                    max: None,
                },
                hold_cycle_frames: OptionalFrameControl {
                    default: None,
                    min: 1,
                    max: None,
                },
                exit_frames: FrameControl {
                    default: 0,
                    min: 0,
                    max: None,
                },
            },
            cues: BTreeMap::new(),
            assets: BTreeMap::new(),
            camera: CameraControls::default(),
        }
    }

    #[test]
    fn structured_data_validation_is_closed_bounded_and_keyed() {
        let controls = data_controls();
        controls
            .validate_data(&json!({"rows":[{"id":"a","value":42}]}))
            .unwrap();

        let errors = controls
            .validate_data(&json!({
                "rows": [
                    {"id":"same","value":-1},
                    {"id":"same","value":2,"extra":true}
                ],
                "unknown": 1
            }))
            .unwrap_err();
        assert!(errors.iter().any(|error| error.path == "/data/unknown"));
        assert!(
            errors
                .iter()
                .any(|error| error.path == "/data/rows/0/value")
        );
        assert!(errors.iter().any(|error| error.path == "/data/rows/1/id"));
        assert!(
            errors
                .iter()
                .any(|error| error.path == "/data/rows/1/extra")
        );
    }

    #[test]
    fn array_schema_requires_a_real_static_upper_bound() {
        let mut controls = data_controls();
        let PrepareDataType::Array { max_items, .. } = controls.data.get_mut("rows").unwrap()
        else {
            unreachable!()
        };
        *max_items = MAX_PREPARE_DATA_ARRAY_ITEMS + 1;
        let mut errors = Vec::new();
        controls.validate(&mut errors);
        assert!(errors.iter().any(|error| error.path.ends_with("/maxItems")));
    }
}
