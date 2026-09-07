//! Static AST/value conversion shared by JSX, style, and expression lowering.

use super::*;

pub(super) fn static_property_name(key: &PropertyKey<'_>) -> Option<String> {
    match key {
        PropertyKey::StaticIdentifier(identifier) => Some(identifier.name.to_string()),
        PropertyKey::StringLiteral(literal) => Some(literal.value.to_string()),
        _ => None,
    }
}

pub(super) fn is_component_name(name: &str) -> bool {
    name.chars().next().is_some_and(char::is_uppercase)
}

pub(super) fn binding_local_name(pattern: &BindingPattern<'_>) -> Option<String> {
    match pattern {
        BindingPattern::BindingIdentifier(identifier) => Some(identifier.name.to_string()),
        BindingPattern::AssignmentPattern(pattern) => binding_local_name(&pattern.left),
        _ => None,
    }
}

pub(super) fn binding_default<'a>(pattern: &'a BindingPattern<'a>) -> Option<&'a Expression<'a>> {
    match pattern {
        BindingPattern::AssignmentPattern(pattern) => Some(&pattern.right),
        _ => None,
    }
}

/// Human-readable value type names for diagnostics only.
pub(super) fn value_kind_name(value: &MotionValue) -> &'static str {
    match value {
        MotionValue::Number(_) => "a number",
        MotionValue::Bool(_) => "a boolean",
        MotionValue::Str(_) => "a string",
        MotionValue::Enum(_) => "an enum keyword",
        MotionValue::Length(_) => "a length (it carries a CSS unit)",
        MotionValue::Length2(_) => "a length pair",
        MotionValue::Angle(_) => "an angle",
        MotionValue::Color(_) => "a color",
        MotionValue::Point(_) => "a point",
        MotionValue::Vec2(_) => "a vector",
        MotionValue::Rect(_) => "a rect",
        MotionValue::PathData(_) => "path data",
    }
}

pub(super) fn motion_value_from_json(value: &serde_json::Value) -> Option<MotionValue> {
    match value {
        serde_json::Value::Bool(value) => Some(MotionValue::Bool(*value)),
        serde_json::Value::Number(value) => value
            .as_f64()
            .filter(|value| value.is_finite())
            .map(MotionValue::Number),
        serde_json::Value::String(value) => Some(motion_value_from_string(value)),
        serde_json::Value::Object(object) => match object
            .get("__valleType")
            .and_then(serde_json::Value::as_str)?
        {
            "point" => Some(MotionValue::Point(Point::new(
                object.get("x")?.as_f64()?,
                object.get("y")?.as_f64()?,
            ))),
            "rect" => Some(MotionValue::Rect(Rect::new(
                object.get("x")?.as_f64()?,
                object.get("y")?.as_f64()?,
                object.get("width")?.as_f64()?,
                object.get("height")?.as_f64()?,
            ))),
            "pathData" => object
                .get("d")?
                .as_str()
                .and_then(parse_path_data)
                .map(MotionValue::PathData),
            "pathLine" => object
                .get("points")?
                .as_array()?
                .iter()
                .map(motion_value_from_json)
                .map(|value| match value {
                    Some(MotionValue::Point(point)) => Some(point),
                    _ => None,
                })
                .collect::<Option<Vec<_>>>()
                .and_then(|points| PathData::line(points).ok())
                .map(MotionValue::PathData),
            "pathCubic" => {
                let point = |name| match motion_value_from_json(object.get(name)?)? {
                    MotionValue::Point(point) => Some(point),
                    _ => None,
                };
                PathData::cubic(
                    point("from")?,
                    point("control1")?,
                    point("control2")?,
                    point("to")?,
                )
                .ok()
                .map(MotionValue::PathData)
            }
            "pathArc" => {
                let MotionValue::Point(center) = motion_value_from_json(object.get("center")?)?
                else {
                    return None;
                };
                PathData::arc(
                    center,
                    object.get("radius")?.as_f64()?,
                    object.get("startAngle")?.as_f64()?,
                    object.get("endAngle")?.as_f64()?,
                )
                .ok()
                .map(MotionValue::PathData)
            }
            "pathArea" => {
                let input = object.get("input")?;
                let path = if let Some(points) = input.as_array() {
                    let points = points
                        .iter()
                        .map(motion_value_from_json)
                        .map(|value| match value {
                            Some(MotionValue::Point(point)) => Some(point),
                            _ => None,
                        })
                        .collect::<Option<Vec<_>>>()?;
                    PathData::line(points).ok()?
                } else {
                    match motion_value_from_json(input)? {
                        MotionValue::PathData(path) => path,
                        _ => return None,
                    }
                };
                path.area(object.get("baseline")?.as_f64()?)
                    .ok()
                    .map(MotionValue::PathData)
            }
            "pathOffset" => {
                let MotionValue::PathData(path) = motion_value_from_json(object.get("input")?)?
                else {
                    return None;
                };
                path.offset_path(object.get("distance")?.as_f64()?)
                    .ok()
                    .map(MotionValue::PathData)
            }
            "pathBoolean" => {
                let MotionValue::PathData(left) = motion_value_from_json(object.get("left")?)?
                else {
                    return None;
                };
                let MotionValue::PathData(right) = motion_value_from_json(object.get("right")?)?
                else {
                    return None;
                };
                let op = match object.get("op")?.as_str()? {
                    "union" => PathBooleanOp::Union,
                    "intersection" => PathBooleanOp::Intersection,
                    "difference" => PathBooleanOp::Difference,
                    "xor" => PathBooleanOp::Xor,
                    _ => return None,
                };
                left.boolean(&right, op).ok().map(MotionValue::PathData)
            }
            _ => None,
        },
        _ => None,
    }
}

pub(super) fn static_paint_from_json(value: &serde_json::Value) -> Option<PaintValue> {
    let object = value.as_object()?;
    let kind = object.get("__valleType")?.as_str()?;
    let point = |name: &str| match motion_value_from_json(object.get(name)?)? {
        MotionValue::Point(value) => Some(PointValue::Static { value }),
        _ => None,
    };
    let number = |name: &str| {
        object
            .get(name)?
            .as_f64()
            .filter(|value| value.is_finite())
            .map(|value| NumberValue::Static { value })
    };
    let stops = || static_gradient_stops_from_json(object.get("stops")?);
    let spread = match object.get("spread").and_then(serde_json::Value::as_str) {
        Some("pad") => SpreadMode::Pad,
        Some("repeat") => SpreadMode::Repeat,
        Some("reflect") => SpreadMode::Reflect,
        _ => return None,
    };
    match kind {
        "linearGradient" => Some(PaintValue::Linear {
            start: point("start")?,
            end: point("end")?,
            stops: stops()?,
            spread,
        }),
        "radialGradient" => Some(PaintValue::Radial {
            center: point("center")?,
            radius: number("radius")?,
            stops: stops()?,
            spread,
        }),
        "conicGradient" => Some(PaintValue::Conic {
            center: point("center")?,
            start_angle: number("startAngle")?,
            stops: stops()?,
            spread,
        }),
        _ => None,
    }
}

pub(super) fn static_gradient_stops_from_json(
    value: &serde_json::Value,
) -> Option<Vec<GradientStopValue>> {
    let values = value.as_array()?;
    if !(2..=64).contains(&values.len()) {
        return None;
    }
    values.iter().map(static_gradient_stop_from_json).collect()
}

pub(super) fn static_gradient_stop_from_json(
    value: &serde_json::Value,
) -> Option<GradientStopValue> {
    let object = value.as_object()?;
    if object.get("__valleType")?.as_str()? != "gradientStop" {
        return None;
    }
    let offset = object
        .get("offset")?
        .as_f64()
        .filter(|value| value.is_finite())?;
    let color = object.get("color")?.as_str().and_then(Rgba::parse)?;
    Some(GradientStopValue {
        offset: NumberValue::Static { value: offset },
        color: ColorValue::Static { value: color },
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PathTopology {
    Open,
    Closed,
    Unknown,
}

pub(super) fn path_topology(
    svg_shape: Option<&str>,
    path: &PathValue,
    exprs: &[Expr],
) -> PathTopology {
    match svg_shape {
        Some("line" | "polyline") => return PathTopology::Open,
        Some("polygon" | "rect" | "circle" | "ellipse") => return PathTopology::Closed,
        _ => {}
    }
    match path {
        PathValue::Static { value } => verbs_topology(&value.verbs),
        PathValue::Expr { expr, .. } => expr_path_topology(*expr, exprs),
    }
}

pub(super) fn verbs_topology(verbs: &[PathVerb]) -> PathTopology {
    if verbs.iter().any(|verb| *verb == PathVerb::Close) {
        PathTopology::Closed
    } else {
        PathTopology::Open
    }
}

pub(super) fn expr_path_topology(expr: ExprId, exprs: &[Expr]) -> PathTopology {
    let Some(node) = exprs.get(expr.0 as usize) else {
        return PathTopology::Unknown;
    };
    match node {
        Expr::PathLine { .. } | Expr::PathCubic { .. } => PathTopology::Open,
        Expr::PathTemplate { verbs, .. } => verbs_topology(verbs),
        Expr::PathArc { .. } | Expr::PathArea { .. } => PathTopology::Closed,
        Expr::PathOffset { path, .. }
        | Expr::PathMorph { from: path, .. }
        | Expr::PathPointAt { path, .. } => expr_path_topology(*path, exprs),
        _ => PathTopology::Unknown,
    }
}
