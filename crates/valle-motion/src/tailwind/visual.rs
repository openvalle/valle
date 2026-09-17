//! Visual utilities lower to the same CSS properties as inline style. Only trusted
//! composition slots use variables; authored values pass shared strict CSS admission.
use super::{
    TailwindClassError,
    normalize::{ValueResolver, decode_arbitrary, default_theme},
};
use takumi_core::style::StyleDeclaration;

pub(super) struct VisualUtility {
    pub declarations: Vec<(String, String)>,
    /// Retained for the lowering's own bookkeeping; conflict order is derived from declarations.
    #[allow(dead_code)]
    pub representative: String,
}

// Longest names first. This also supplies the property identity used by package admission
// and geometry reuse, so adding a utility cannot bypass their corresponding style checks.
fn family(candidate: &str) -> Option<(&'static str, &str, bool)> {
    let candidate = candidate.strip_suffix('!').unwrap_or(candidate);
    let (negative, candidate) = candidate
        .strip_prefix('-')
        .map_or((false, candidate), |v| (true, v));
    for prefix in [
        "backdrop-hue-rotate",
        "backdrop-brightness",
        "backdrop-grayscale",
        "backdrop-contrast",
        "backdrop-saturate",
        "backdrop-opacity",
        "backdrop-filter",
        "backdrop-invert",
        "backdrop-sepia",
        "backdrop-blur",
        "drop-shadow",
        "hue-rotate",
        "translate-x",
        "translate-y",
        "translate",
        "scale-x",
        "scale-y",
        "scale",
        "rotate",
        "origin",
        "brightness",
        "contrast",
        "grayscale",
        "saturate",
        "invert",
        "sepia",
        "blur",
        "filter",
    ] {
        if candidate == prefix {
            return Some((prefix, "", negative));
        }
        if let Some(value) = candidate
            .strip_prefix(prefix)
            .and_then(|v| v.strip_prefix('-'))
        {
            return Some((prefix, value, negative));
        }
    }
    None
}

pub(crate) fn property(candidate: &str) -> Option<&'static str> {
    let (prefix, _, _) = family(candidate)?;
    Some(match prefix {
        "translate" | "translate-x" | "translate-y" => "translate",
        "scale" | "scale-x" | "scale-y" => "scale",
        "rotate" => "rotate",
        "origin" => "transform-origin",
        value if value.starts_with("backdrop-") => "backdrop-filter",
        _ => "filter",
    })
}

fn checked(property: &str, value: &str) -> Result<(), TailwindClassError> {
    if crate::style::contains_variable(value) {
        return Err(TailwindClassError::Css(crate::style::StyleIssue::new(
            crate::style::StyleIssueKind::UnsupportedValue,
            property,
            value,
            "authored variables need the shared variable resolver; use a resolved CSS value",
        )));
    }
    crate::style::parse_property(property, value)
        .map(|_| ())
        .map_err(TailwindClassError::Css)
}

fn arbitrary(
    value: &str,
    property: &str,
    resolve: &ValueResolver<'_>,
) -> Result<Option<String>, TailwindClassError> {
    if let Some(value) = value.strip_prefix('[') {
        Ok(Some(resolve(
            property,
            &decode_arbitrary(
                value
                    .strip_suffix(']')
                    .ok_or(TailwindClassError::Unsupported)?,
            )?,
        )?))
    } else {
        Ok(None)
    }
}

fn scale_number(value: &str) -> Result<f32, TailwindClassError> {
    let mut tokens = cssparser::ParserInput::new(value);
    let mut input = cssparser::Parser::new(&mut tokens);
    let number = match input.next().map_err(|_| TailwindClassError::Unsupported)? {
        cssparser::Token::Number { value, .. } => *value,
        cssparser::Token::Percentage { unit_value, .. } => *unit_value,
        _ => return Err(TailwindClassError::Unsupported),
    };
    input
        .expect_exhausted()
        .map_err(|_| TailwindClassError::Unsupported)?;
    if !number.is_finite() {
        return Err(TailwindClassError::Unsupported);
    }
    Ok(number)
}

fn angle(
    value: &str,
    negative: bool,
    resolve: &ValueResolver<'_>,
) -> Result<String, TailwindClassError> {
    let value = if let Some(value) = arbitrary(value, "rotate", resolve)? {
        value
    } else if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) {
        format!("{value}deg")
    } else {
        return Err(TailwindClassError::Unsupported);
    };
    checked("rotate", &value)?;
    let parsed = crate::style::parse_property("rotate", &value)
        .map_err(|_| TailwindClassError::Unsupported)?;
    let Some(StyleDeclaration::Rotate(Some(angle))) = parsed.declarations.iter().next() else {
        return Err(TailwindClassError::Unsupported);
    };
    let degrees = **angle * if negative { -1.0 } else { 1.0 };
    if !degrees.is_finite() {
        return Err(TailwindClassError::Unsupported);
    }
    Ok(format!("{degrees}deg"))
}

fn chain(backdrop: bool) -> String {
    let names: &[&str] = if backdrop {
        &[
            "blur",
            "brightness",
            "contrast",
            "grayscale",
            "hue-rotate",
            "invert",
            "opacity",
            "saturate",
            "sepia",
        ]
    } else {
        &[
            "blur",
            "brightness",
            "contrast",
            "grayscale",
            "hue-rotate",
            "invert",
            "saturate",
            "sepia",
            "drop-shadow",
        ]
    };
    names
        .iter()
        .map(|name| {
            slot(
                &format!("{}{name}", if backdrop { "backdrop-" } else { "" }),
                "",
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn slot(name: &str, fallback: &str) -> String {
    format!("var(--tw-important-{name},var(--tw-{name},{fallback}))")
}

pub(super) fn utility(
    class: &str,
    resolve: &ValueResolver<'_>,
) -> Result<Option<VisualUtility>, TailwindClassError> {
    let Some((prefix, token, negative)) = family(class) else {
        return Ok(None);
    };
    let fail = TailwindClassError::Unsupported;
    let property = property(class).ok_or_else(|| fail.clone())?;
    let mut declarations = Vec::new();
    let representative;
    match property {
        "translate" => {
            if token == "none" && prefix == "translate" && !negative {
                // The backend's default pair represents CSS none.
                declarations.push((property.into(), "initial".into()));
                representative = "translate-none".into();
            } else {
                let value = if let Some(value) = arbitrary(token, property, resolve)? {
                    value
                } else if token == "full" {
                    "100%".into()
                } else if token == "px" {
                    "1px".into()
                } else if let Some((a, b)) = token.split_once('/') {
                    let a = a.parse::<u32>().map_err(|_| fail.clone())?;
                    let b = b.parse::<u32>().map_err(|_| fail.clone())?;
                    if b == 0 {
                        return Err(fail.clone());
                    }
                    format!("{}%", f64::from(a) / f64::from(b) * 100.0)
                } else if super::non_negative_number(token) {
                    format!("calc(0.25rem * {token})")
                } else {
                    return Err(fail.clone());
                };
                let value = if negative {
                    format!("calc({value} * -1)")
                } else {
                    value
                };
                // Validate a single length, not a whole translate pair or a CSS-wide keyword.
                checked("translate", &format!("{value} 0px"))?;
                for axis in ["x", "y"] {
                    if prefix == "translate" || prefix.ends_with(axis) {
                        declarations.push((format!("--tw-translate-{axis}"), value.clone()));
                    }
                }
                declarations.push((
                    property.into(),
                    format!(
                        "{} {}",
                        slot("translate-x", "0px"),
                        slot("translate-y", "0px")
                    ),
                ));
                representative = format!("{prefix}-4");
            }
        }
        "scale" => {
            if token == "none" && prefix == "scale" && !negative {
                declarations.push((property.into(), "initial".into()));
                representative = "scale-none".into();
            } else if prefix == "scale" && token.starts_with('[') {
                let mut value = arbitrary(token, property, resolve)?.ok_or_else(|| fail.clone())?;
                if negative {
                    value = (-scale_number(&value)?).to_string();
                }
                checked(property, &value)?;
                declarations.push((property.into(), value));
                representative = "scale-[1.25]".into();
            } else {
                let value = if let Some(value) = arbitrary(token, property, resolve)? {
                    value
                } else if !token.is_empty() && token.bytes().all(|b| b.is_ascii_digit()) {
                    format!("{token}%")
                } else {
                    return Err(fail.clone());
                };
                checked("scale", &format!("{value} 1"))?;
                let number = scale_number(&value)? * if negative { -1.0 } else { 1.0 };
                let value = number.to_string();
                for axis in ["x", "y"] {
                    if prefix == "scale" || prefix.ends_with(axis) {
                        declarations.push((format!("--tw-scale-{axis}"), value.clone()));
                    }
                }
                declarations.push((
                    property.into(),
                    format!("{} {}", slot("scale-x", "100%"), slot("scale-y", "100%")),
                ));
                representative = format!("{prefix}-4");
            }
        }
        "rotate" => {
            let value = if token == "none" && !negative {
                "none".into()
            } else {
                angle(token, negative, resolve)?
            };
            checked(property, &value)?;
            declarations.push((property.into(), value));
            representative = "rotate-4".into();
        }
        "transform-origin" => {
            if negative {
                return Err(fail.clone());
            }
            let value = if let Some(value) = arbitrary(token, property, resolve)? {
                value
            } else {
                match token {
                    "center" | "top" | "right" | "bottom" | "left" | "top-left" | "top-right"
                    | "bottom-left" | "bottom-right" => token.replace('-', " "),
                    _ => return Err(fail.clone()),
                }
            };
            checked(property, &value)?;
            declarations.push((property.into(), value));
            representative = "origin-center".into();
        }
        "filter" | "backdrop-filter" => {
            let backdrop = property == "backdrop-filter";
            let function = prefix.strip_prefix("backdrop-").unwrap_or(prefix);
            if negative && function != "hue-rotate" {
                return Err(fail.clone());
            }
            if function == "filter" {
                let value = if token.is_empty() {
                    chain(backdrop)
                } else if token == "none" {
                    "none".into()
                } else {
                    let value = arbitrary(token, property, resolve)?.ok_or_else(|| fail.clone())?;
                    checked(property, &value)?;
                    value
                };
                declarations.push((property.into(), value));
                representative = property.into();
            } else {
                let (value, sample) = match function {
                    "blur" | "drop-shadow" if token == "none" => {
                        (" ".into(), format!("{prefix}-none"))
                    }
                    "blur" => {
                        let length = if token.is_empty() {
                            "8px".into()
                        } else if let Some(value) = arbitrary(token, property, resolve)? {
                            value
                        } else {
                            default_theme(&format!("--blur-{token}"))
                                .ok_or_else(|| fail.clone())?
                                .into()
                        };
                        (format!("blur({length})"), format!("{prefix}-sm"))
                    }
                    "drop-shadow" => {
                        let value = if token.is_empty() {
                            "drop-shadow(0 1px 2px rgb(0 0 0 / 0.1)) drop-shadow(0 1px 1px rgb(0 0 0 / 0.06))".into()
                        } else {
                            let shadow = if let Some(value) = arbitrary(token, property, resolve)? {
                                value
                            } else {
                                default_theme(&format!("--drop-shadow-{token}"))
                                    .ok_or_else(|| fail.clone())?
                                    .into()
                            };
                            format!("drop-shadow({shadow})")
                        };
                        (value, "drop-shadow-sm".into())
                    }
                    "hue-rotate" => (
                        format!("hue-rotate({})", angle(token, negative, resolve)?),
                        format!("{prefix}-30"),
                    ),
                    "brightness" | "contrast" | "grayscale" | "invert" | "saturate" | "sepia"
                    | "opacity" => {
                        let amount = if let Some(value) = arbitrary(token, property, resolve)? {
                            value
                        } else if token.is_empty()
                            && matches!(function, "grayscale" | "invert" | "sepia")
                        {
                            "100%".into()
                        } else if !token.is_empty() && token.bytes().all(|b| b.is_ascii_digit()) {
                            format!("{token}%")
                        } else {
                            return Err(fail.clone());
                        };
                        let sample = match function {
                            "grayscale" | "invert" | "sepia" => prefix.into(),
                            "opacity" => format!("{prefix}-50"),
                            "saturate" => format!("{prefix}-150"),
                            _ => format!("{prefix}-125"),
                        };
                        (format!("{function}({amount})"), sample)
                    }
                    _ => return Err(fail.clone()),
                };
                if !value.trim().is_empty() {
                    checked(property, &value)?;
                }
                declarations.push((format!("--tw-{prefix}"), value));
                declarations.push((property.into(), chain(backdrop)));
                representative = sample;
            }
        }
        _ => return Err(fail.clone()),
    }
    if property == "scale"
        && let Some((_, value)) = declarations.iter().find(|(name, _)| name == property)
    {
        let (marker, state) = crate::style::scale_presence(value);
        declarations.push((marker.into(), state.into()));
    }
    Ok(Some(VisualUtility {
        declarations,
        representative,
    }))
}
