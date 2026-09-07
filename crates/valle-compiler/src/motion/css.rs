//! Pure CSS filter, border, and transform syntax parsing.

use super::*;

pub(super) fn normalize_css_filter(input: &str) -> Result<Option<String>, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() || trimmed == "none" {
        return Ok(None);
    }
    let functions = parse_css_filter_functions(trimmed)
        .ok_or_else(|| format!("filter `{input}` is not a supported CSS filter list"))?;
    let mut kept = Vec::new();
    for (name, args) in functions {
        if name == "blur" {
            let radius = parse_css_filter_length(&args)
                .ok_or_else(|| format!("filter blur() requires a finite length, got `{args}`"))?;
            if radius < 0.0 {
                return Err(
                    "static blur/sigma must be >= 0; clamp dynamic values in source".into(),
                );
            }
            if radius == 0.0 {
                continue;
            }
        }
        if name == "drop-shadow" {
            let parts: Vec<&str> = args.split_whitespace().collect();
            if let Some(blur) = parts.get(2).and_then(|part| parse_css_filter_length(part))
                && blur < 0.0
            {
                return Err(
                    "static drop-shadow blur must be >= 0; clamp dynamic values in source".into(),
                );
            }
        }
        kept.push(format!("{name}({args})"));
    }
    if kept.is_empty() {
        Ok(None)
    } else {
        Ok(Some(kept.join(" ")))
    }
}

pub(super) fn parse_css_filter_functions(input: &str) -> Option<Vec<(String, String)>> {
    let mut rest = input.trim();
    let mut out = Vec::new();
    while !rest.is_empty() {
        let name_end = rest.find('(')?;
        let name = rest[..name_end].trim();
        if name.is_empty() || !name.chars().all(|ch| ch.is_ascii_alphabetic() || ch == '-') {
            return None;
        }
        let after = &rest[name_end + 1..];
        let close = after.find(')')?;
        let args = after[..close].trim().to_string();
        out.push((name.to_ascii_lowercase(), args));
        rest = after[close + 1..].trim_start();
    }
    Some(out)
}

pub(super) fn parse_css_filter_length(input: &str) -> Option<f64> {
    let trimmed = input.trim();
    let number = trimmed
        .strip_suffix("px")
        .or_else(|| trimmed.strip_suffix("rem"))
        .or_else(|| trimmed.strip_suffix("em"))
        .unwrap_or(trimmed);
    number.parse::<f64>().ok().filter(|value| value.is_finite())
}

pub(super) fn string_literal(expression: &Expression<'_>) -> Option<String> {
    match strip_parens(expression) {
        Expression::StringLiteral(value) => Some(value.value.to_string()),
        _ => None,
    }
}

pub(super) fn is_border_width_property(name: &str) -> bool {
    name == "border-width"
        || name.ends_with("-width") && name.starts_with("border")
        || name == "outline-width"
}

pub(super) fn is_border_color_property(name: &str) -> bool {
    name == "border-color"
        || name.ends_with("-color") && name.starts_with("border")
        || name == "outline-color"
}

pub(super) fn is_border_style_property(name: &str) -> bool {
    name == "border-style"
        || name.ends_with("-style") && name.starts_with("border")
        || name == "outline-style"
}

pub(super) fn tailwind_sets_border_width(class_name: &str) -> bool {
    matches!(
        class_name,
        "border"
            | "outline"
            | "border-t"
            | "border-r"
            | "border-b"
            | "border-l"
            | "border-x"
            | "border-y"
    ) || [
        "border-",
        "border-t-",
        "border-r-",
        "border-b-",
        "border-l-",
        "border-x-",
        "border-y-",
        "outline-",
    ]
    .iter()
    .any(|prefix| {
        class_name
            .strip_prefix(prefix)
            .is_some_and(|value| value.parse::<u32>().is_ok())
    })
}

pub(super) fn tailwind_sets_border_color(class_name: &str) -> bool {
    class_name.strip_prefix("border-").is_some_and(|token| {
        !matches!(
            token,
            "solid" | "dashed" | "dotted" | "double" | "none" | "hidden"
        ) && token.parse::<u32>().is_err()
            && !token.starts_with(|ch: char| ch.is_ascii_digit())
    })
}

pub(super) fn tailwind_sets_border_style(class_name: &str) -> bool {
    matches!(
        class_name,
        "border-solid"
            | "border-dashed"
            | "border-dotted"
            | "border-double"
            | "border-none"
            | "outline-solid"
            | "outline-dashed"
            | "outline-dotted"
            | "outline-double"
            | "outline-none"
    )
}

pub(super) fn parse_core_transform(source: &str) -> Option<Vec<(String, String)>> {
    let bytes = source.as_bytes();
    let mut at = 0;
    let mut last_order = 0;
    let mut parts = Vec::new();
    while at < bytes.len() {
        while at < bytes.len() && bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        if at == bytes.len() {
            break;
        }
        let name_start = at;
        while at < bytes.len() && bytes[at].is_ascii_alphabetic() {
            at += 1;
        }
        let name = &source[name_start..at];
        let order = match name {
            "translate" => 1,
            "translateX" => 1,
            "translateY" => 2,
            "rotate" => 3,
            "scale" | "scaleX" | "scaleY" => 4,
            _ => return None,
        };
        let scale_axis_pair = order == 4
            && last_order == 4
            && name != "scale"
            && parts.iter().all(|(existing, _)| existing != "scale")
            && !parts.iter().any(|(existing, _)| existing == name);
        if (!scale_axis_pair && order <= last_order) || bytes.get(at) != Some(&b'(') {
            return None;
        }
        last_order = order;
        at += 1;
        let argument_start = at;
        while at < bytes.len() && bytes[at] != b')' {
            if bytes[at] == b'(' {
                return None;
            }
            at += 1;
        }
        if at == bytes.len() {
            return None;
        }
        let argument = source[argument_start..at].trim();
        if argument.is_empty() {
            return None;
        }
        if argument.contains(',') && name != "scale" {
            return None;
        }
        if name == "scale" && argument.split(',').count() > 2 {
            return None;
        }
        at += 1;
        if at < bytes.len() && !bytes[at].is_ascii_whitespace() {
            return None;
        }
        parts.push((name.to_string(), argument.to_string()));
    }
    (!parts.is_empty()).then_some(parts)
}

#[derive(Clone, Copy)]
pub(super) enum Css3dArgument {
    Length,
    Angle,
    Number,
}

/// Parse the deterministic ordered transform subset. Callers try the compact typed 2D path
/// first, then use this parser for every other legal authored order and all CSS 3D functions.
pub(super) fn parse_ordered_transform(source: &str) -> Option<Vec<(String, String)>> {
    let bytes = source.as_bytes();
    let mut at = 0;
    let mut parts = Vec::new();
    while at < bytes.len() {
        while at < bytes.len() && bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        if at == bytes.len() {
            break;
        }
        let name_start = at;
        while at < bytes.len() && bytes[at].is_ascii_alphanumeric() {
            at += 1;
        }
        let name = &source[name_start..at];
        if !matches!(
            name,
            "translate"
                | "translateX"
                | "translateY"
                | "translateZ"
                | "translate3d"
                | "rotate"
                | "rotateX"
                | "rotateY"
                | "rotateZ"
                | "rotate3d"
                | "scale"
                | "scaleX"
                | "scaleY"
                | "scaleZ"
                | "scale3d"
        ) || bytes.get(at) != Some(&b'(')
        {
            return None;
        }
        at += 1;
        let argument_start = at;
        while at < bytes.len() && bytes[at] != b')' {
            if bytes[at] == b'(' {
                return None;
            }
            at += 1;
        }
        if at == bytes.len() {
            return None;
        }
        let argument = source[argument_start..at].trim();
        if argument.is_empty() {
            return None;
        }
        at += 1;
        if at < bytes.len() && !bytes[at].is_ascii_whitespace() {
            return None;
        }
        parts.push((name.to_owned(), argument.to_owned()));
    }
    (!parts.is_empty()).then_some(parts)
}

pub(super) fn split_transform_arguments(argument: &str, expected: usize) -> Option<Vec<&str>> {
    let values = argument.split(',').map(str::trim).collect::<Vec<_>>();
    (values.len() == expected && values.iter().all(|value| !value.is_empty())).then_some(values)
}

/// Split a leading __VALLE_HOLE_N__ into its index and static suffix. Leave additional holes in the
/// suffix for property-specific validation.
pub(super) fn transform_hole_parts(argument: &str) -> Option<(usize, &str)> {
    let rest = argument.strip_prefix("__VALLE_HOLE_")?;
    let (digits, suffix) = rest.split_once("__")?;
    Some((digits.parse().ok()?, suffix))
}
