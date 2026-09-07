//! Prepare-time theme validation and deterministic deep merge.

use super::*;

pub(super) fn validate_theme(theme: &serde_json::Value) -> Result<(), String> {
    if !theme.is_object() {
        return Err("ThemeProvider `value` must be an object of typed tokens".into());
    }
    let bytes = serde_json::to_vec(theme)
        .map_err(|error| format!("ThemeProvider value is not canonical JSON: {error}"))?
        .len();
    if bytes > MAX_THEME_BYTES {
        return Err(format!(
            "ThemeProvider value uses {bytes} bytes; the compile-time theme budget is {MAX_THEME_BYTES} bytes"
        ));
    }

    fn walk(
        value: &serde_json::Value,
        path: &str,
        depth: usize,
        tokens: &mut usize,
    ) -> Result<(), String> {
        if depth > MAX_THEME_DEPTH {
            return Err(format!(
                "ThemeProvider token `{path}` exceeds the maximum nesting depth of {MAX_THEME_DEPTH}"
            ));
        }
        match value {
            serde_json::Value::Null => Err(format!(
                "ThemeProvider token `{path}` is null; tokens need a stable string, number, boolean, array, object, or typed Motion value"
            )),
            serde_json::Value::Bool(_)
            | serde_json::Value::Number(_)
            | serde_json::Value::String(_) => {
                *tokens += 1;
                Ok(())
            }
            serde_json::Value::Array(items) => {
                *tokens += 1;
                for (index, item) in items.iter().enumerate() {
                    walk(item, &format!("{path}[{index}]"), depth + 1, tokens)?;
                }
                Ok(())
            }
            serde_json::Value::Object(object) => {
                if let Some(marker) = object.get("__valleType") {
                    let Some(kind) = marker.as_str() else {
                        return Err(format!(
                            "ThemeProvider token `{path}` has a non-string `__valleType` marker"
                        ));
                    };
                    if !THEME_TYPED_TOKEN_KINDS.contains(&kind) {
                        return Err(format!(
                            "ThemeProvider token `{path}` uses unknown typed Motion value `{kind}`"
                        ));
                    }
                    *tokens += 1;
                }
                for (key, item) in object {
                    if matches!(key.as_str(), "__proto__" | "prototype" | "constructor") {
                        return Err(format!(
                            "ThemeProvider token path `{path}.{key}` uses a reserved object key"
                        ));
                    }
                    walk(item, &format!("{path}.{key}"), depth + 1, tokens)?;
                }
                Ok(())
            }
        }
    }

    let mut tokens = 0;
    walk(theme, "theme", 0, &mut tokens)?;
    if tokens > MAX_THEME_TOKENS {
        return Err(format!(
            "ThemeProvider value contains {tokens} tokens; the compile-time theme budget is {MAX_THEME_TOKENS}"
        ));
    }
    Ok(())
}

pub(super) fn theme_kind(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "null".into(),
        serde_json::Value::Bool(_) => "boolean".into(),
        serde_json::Value::Number(_) => "number".into(),
        serde_json::Value::String(_) => "string".into(),
        serde_json::Value::Array(_) => "array".into(),
        serde_json::Value::Object(object) => object
            .get("__valleType")
            .and_then(serde_json::Value::as_str)
            .map_or_else(|| "object".into(), |kind| format!("typed {kind}")),
    }
}

/// Deep-merge plain token groups. Leaves, arrays, and typed Motion values replace atomically, but
/// an inherited token cannot silently change kind: that catches misshapen nested overrides at the
/// provider span instead of much later in style lowering.
pub(super) fn merge_theme(
    base: &mut serde_json::Value,
    overlay: serde_json::Value,
    path: &str,
) -> Result<(), String> {
    let base_is_plain =
        matches!(base, serde_json::Value::Object(object) if !object.contains_key("__valleType"));
    let overlay_is_plain = matches!(&overlay, serde_json::Value::Object(object) if !object.contains_key("__valleType"));
    if base_is_plain && overlay_is_plain {
        let serde_json::Value::Object(base_object) = base else {
            unreachable!("plain base checked above")
        };
        let serde_json::Value::Object(overlay_object) = overlay else {
            unreachable!("plain overlay checked above")
        };
        for (key, value) in overlay_object {
            let child_path = format!("{path}.{key}");
            if let Some(existing) = base_object.get_mut(&key) {
                merge_theme(existing, value, &child_path)?;
            } else {
                base_object.insert(key, value);
            }
        }
        return Ok(());
    }

    let inherited = theme_kind(base);
    let replacement = theme_kind(&overlay);
    if inherited != replacement {
        return Err(format!(
            "ThemeProvider override `{path}` changes token type from {inherited} to {replacement}; nested providers may replace values but not their inferred token types"
        ));
    }
    *base = overlay;
    Ok(())
}

pub(super) fn validation_span(
    path: &str,
    expr_spans: &[Span],
    node_spans: &[Span],
    controls_span: Option<Span>,
) -> Span {
    fn index(path: &str, prefix: &str) -> Option<usize> {
        path.strip_prefix(prefix)?.split('/').next()?.parse().ok()
    }
    if let Some(index) = index(path, "/exprs/") {
        expr_spans.get(index).copied().unwrap_or_default()
    } else if let Some(index) = index(path, "/nodes/") {
        node_spans.get(index).copied().unwrap_or_default()
    } else if path.starts_with("/controls/") || path.starts_with("/resourceRefs/") {
        controls_span.unwrap_or_default()
    } else {
        Span::new(0, 0)
    }
}
