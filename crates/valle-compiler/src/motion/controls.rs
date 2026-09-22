//! Folded controls schema and structured prepare-data parsing.

use super::*;

pub(super) fn default_controls() -> ControlsSchema {
    ControlsSchema {
        props: BTreeMap::new(),
        data: BTreeMap::new(),
        assets: BTreeMap::new(),
    }
}

pub(super) fn controls_from_json(value: &serde_json::Value) -> Result<ControlsSchema, String> {
    let object = value
        .as_object()
        .ok_or("controls must evaluate to an object")?;
    ensure_keys(object, &["props", "data", "assets"]).map_err(str::to_string)?;
    let mut controls = default_controls();
    if let Some(props) = object.get("props") {
        controls.props = parse_named(props, parse_prop)?;
    }
    if let Some(data) = object.get("data") {
        controls.data = parse_named_data(data)?;
    }
    if let Some(assets) = object.get("assets") {
        controls.assets = parse_named(assets, |value| {
            let object = value
                .as_object()
                .ok_or("asset must be an asset() helper call")?;
            ensure_keys(object, &["kind", "assetKind", "required"])?;
            if object.get("kind").and_then(serde_json::Value::as_str) != Some("asset") {
                return Err("asset control must use asset()");
            }
            let kind = match object
                .get("kindName")
                .or_else(|| object.get("assetKind"))
                .and_then(serde_json::Value::as_str)
            {
                Some("image") => AssetKind::Image,
                Some("audio") => AssetKind::Audio,
                Some("video") => AssetKind::Video,
                Some("font") => AssetKind::Font,
                Some("model3d") => AssetKind::Model3d,
                Some("environment") => AssetKind::Environment,
                Some("shader") => AssetKind::Shader,
                _ => {
                    return Err(
                        "asset kind must be image/audio/video/font/model3d/environment/shader",
                    );
                }
            };
            Ok(AssetControl {
                kind,
                required: object
                    .get("required")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false),
            })
        })?;
    }
    Ok(controls)
}

pub(super) fn parse_named_data(
    value: &serde_json::Value,
) -> Result<BTreeMap<String, PrepareDataType>, String> {
    value
        .as_object()
        .ok_or_else(|| "controls.data must be an object".to_string())?
        .iter()
        .map(|(name, value)| {
            parse_prepare_data_type(value)
                .map(|value| (name.clone(), value))
                .map_err(|message| format!("data control `{name}`: {message}"))
        })
        .collect()
}

pub(super) fn parse_prepare_data_type(
    value: &serde_json::Value,
) -> Result<PrepareDataType, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "schema must be a data helper call".to_string())?;
    let kind = object
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "schema kind is missing".to_string())?;
    match kind {
        "number" => {
            ensure_keys(object, &["kind", "min", "max"]).map_err(str::to_string)?;
            Ok(PrepareDataType::Number {
                min: optional_f64(object, "min")?,
                max: optional_f64(object, "max")?,
            })
        }
        "string" => {
            ensure_keys(object, &["kind", "minBytes", "maxBytes"]).map_err(str::to_string)?;
            Ok(PrepareDataType::String {
                min_bytes: optional_usize(object, "minBytes")?,
                max_bytes: optional_usize(object, "maxBytes")?,
            })
        }
        "bool" => {
            ensure_keys(object, &["kind"]).map_err(str::to_string)?;
            Ok(PrepareDataType::Bool)
        }
        "color" => {
            ensure_keys(object, &["kind"]).map_err(str::to_string)?;
            Ok(PrepareDataType::Color)
        }
        "point" => {
            ensure_keys(object, &["kind"]).map_err(str::to_string)?;
            Ok(PrepareDataType::Point)
        }
        "array" => {
            ensure_keys(object, &["kind", "items", "minItems", "maxItems", "key"])
                .map_err(str::to_string)?;
            let items = object
                .get("items")
                .ok_or_else(|| "array() requires an item schema".to_string())?;
            let max_items = required_usize(object, "maxItems")?;
            let min_items = optional_usize(object, "minItems")?.unwrap_or(0);
            let key = match object.get("key") {
                Some(serde_json::Value::String(value)) if !value.is_empty() => Some(value.clone()),
                Some(_) => return Err("array key must be a non-empty string".into()),
                None => None,
            };
            Ok(PrepareDataType::Array {
                items: Box::new(parse_prepare_data_type(items)?),
                min_items,
                max_items,
                key,
            })
        }
        "record" => {
            ensure_keys(object, &["kind", "fields"]).map_err(str::to_string)?;
            let fields = object
                .get("fields")
                .and_then(serde_json::Value::as_object)
                .ok_or_else(|| "record() requires an object of field schemas".to_string())?
                .iter()
                .map(|(name, value)| {
                    parse_prepare_data_type(value)
                        .map(|value| (name.clone(), value))
                        .map_err(|message| format!("record field `{name}`: {message}"))
                })
                .collect::<Result<_, _>>()?;
            Ok(PrepareDataType::Record { fields })
        }
        "tuple" => {
            ensure_keys(object, &["kind", "items"]).map_err(str::to_string)?;
            let items = object
                .get("items")
                .and_then(serde_json::Value::as_array)
                .ok_or_else(|| "tuple() requires an array of item schemas".to_string())?
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    parse_prepare_data_type(value)
                        .map_err(|message| format!("tuple item {index}: {message}"))
                })
                .collect::<Result<_, _>>()?;
            Ok(PrepareDataType::Tuple { items })
        }
        _ => {
            Err("data schema must use number/string/boolean/color/point/array/record/tuple".into())
        }
    }
}

pub(super) fn optional_f64(
    object: &serde_json::Map<String, serde_json::Value>,
    name: &str,
) -> Result<Option<f64>, String> {
    object
        .get(name)
        .map(|value| {
            value
                .as_f64()
                .filter(|value| value.is_finite())
                .ok_or_else(|| format!("{name} must be a finite number"))
        })
        .transpose()
}

pub(super) fn optional_usize(
    object: &serde_json::Map<String, serde_json::Value>,
    name: &str,
) -> Result<Option<usize>, String> {
    object
        .get(name)
        .map(|value| {
            value
                .as_u64()
                .and_then(|value| usize::try_from(value).ok())
                .ok_or_else(|| format!("{name} must be a non-negative integer"))
        })
        .transpose()
}

pub(super) fn required_usize(
    object: &serde_json::Map<String, serde_json::Value>,
    name: &str,
) -> Result<usize, String> {
    optional_usize(object, name)?.ok_or_else(|| format!("{name} is required"))
}

pub(super) fn parse_named<T>(
    value: &serde_json::Value,
    parse: impl Fn(&serde_json::Value) -> Result<T, &'static str>,
) -> Result<BTreeMap<String, T>, String> {
    value
        .as_object()
        .ok_or_else(|| "control namespace must be an object".to_string())?
        .iter()
        .map(|(name, value)| {
            parse(value)
                .map(|value| (name.clone(), value))
                .map_err(|message| format!("control `{name}`: {message}"))
        })
        .collect()
}

pub(super) fn parse_prop(value: &serde_json::Value) -> Result<PropControl, &'static str> {
    let object = value
        .as_object()
        .ok_or("prop must be a control helper call")?;
    ensure_keys(
        object,
        &[
            "kind", "default", "required", "label", "min", "max", "step", "values",
        ],
    )?;
    let kind = object
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .ok_or("control kind is missing")?;
    let control = match kind {
        "number" => ControlType::Number {
            min: object.get("min").and_then(serde_json::Value::as_f64),
            max: object.get("max").and_then(serde_json::Value::as_f64),
            step: object.get("step").and_then(serde_json::Value::as_f64),
        },
        "string" => ControlType::String,
        "bool" => ControlType::Bool,
        "color" => ControlType::Color,
        "length" => ControlType::Length,
        "angle" => ControlType::Angle,
        "point" => ControlType::Point,
        "rect" => ControlType::Rect,
        "select" => ControlType::Select {
            values: object
                .get("values")
                .and_then(serde_json::Value::as_array)
                .ok_or("select needs values")?
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .map(str::to_string)
                        .ok_or("select values must be strings")
                })
                .collect::<Result<_, _>>()?,
        },
        _ => return Err("unknown prop control helper"),
    };
    let default = object
        .get("default")
        .filter(|value| !value.is_null())
        .map(|value| control_default(&control, value))
        .transpose()?;
    Ok(PropControl {
        control,
        default,
        required: object
            .get("required")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        label: object
            .get("label")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
    })
}

pub(super) fn control_default(
    control: &ControlType,
    value: &serde_json::Value,
) -> Result<MotionValue, &'static str> {
    match control {
        ControlType::Number { .. } => value
            .as_f64()
            .map(MotionValue::Number)
            .ok_or("number default must be numeric"),
        ControlType::String => value
            .as_str()
            .map(|value| MotionValue::Str(value.into()))
            .ok_or("string default must be text"),
        ControlType::Bool => value
            .as_bool()
            .map(MotionValue::Bool)
            .ok_or("bool default must be boolean"),
        ControlType::Color => value
            .as_str()
            .and_then(Rgba::parse)
            .map(MotionValue::Color)
            .ok_or("color default is invalid"),
        ControlType::Length => value
            .as_str()
            .and_then(Length::parse)
            .map(MotionValue::Length)
            .ok_or("length default is invalid"),
        ControlType::Angle => value
            .as_str()
            .and_then(Angle::parse)
            .map(MotionValue::Angle)
            .ok_or("angle default is invalid"),
        ControlType::Point => motion_value_from_json(value)
            .filter(|value| matches!(value, MotionValue::Point(_)))
            .ok_or("point default must use point(x, y)"),
        ControlType::Rect => motion_value_from_json(value)
            .filter(|value| matches!(value, MotionValue::Rect(_)))
            .ok_or("rect default must use rect(x, y, width, height)"),
        ControlType::Select { .. } => value
            .as_str()
            .map(|value| MotionValue::Enum(value.into()))
            .ok_or("select default must be text"),
    }
}

pub(super) fn ensure_keys(
    object: &serde_json::Map<String, serde_json::Value>,
    allowed: &[&str],
) -> Result<(), &'static str> {
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        Err("control object contains an unknown field")
    } else {
        Ok(())
    }
}
