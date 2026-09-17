//! Pure CSS filter, border, and transform syntax parsing.

use super::*;

pub(super) fn string_literal(expression: &Expression<'_>) -> Option<String> {
    match strip_parens(expression) {
        Expression::StringLiteral(value) => Some(value.value.to_string()),
        _ => None,
    }
}

#[derive(Clone, Copy)]
pub(super) enum Css3dArgument {
    Length,
    Angle,
    Number,
}

/// Parse the supported CSS 3D list. Ordinary 2D lists use shared CSS admission.
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
