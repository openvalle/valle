//! The internal token table.
//!
//! These are the values behind familiar utility names (`bg-slate-950`, `text-2xl`, `rounded-2xl`,
//! `max-w-sm`), looked up while lowering a class and emitted as literals. This is not a theme
//! system: authors cannot declare, extend, or shadow it, and nothing here is exposed as a CSS
//! variable.
//!
//! Every namespace in the table must have a live utility consumer listed in
//! [`RETAINED_NAMESPACES`]. A family cannot appear without a consumer, and a consumer cannot
//! disappear while its tokens remain.

use std::collections::BTreeMap;

/// Namespaces the author surface keeps: each one has at least one live utility consumer.
#[cfg(test)]
const RETAINED_NAMESPACES: &[&str] = &[
    "color",
    "spacing",
    "font",
    "text",
    "leading",
    "tracking",
    "radius",
    "shadow",
    "drop",
    "blur",
    "container",
    "aspect",
];

/// Generated through the locked official compiler, embedded identically in Native and Wasm.
/// Runtime preparation never reads CSS files or discovers installed fonts.
pub(crate) fn defaults() -> &'static BTreeMap<String, String> {
    static DEFAULTS: std::sync::OnceLock<BTreeMap<String, String>> = std::sync::OnceLock::new();
    DEFAULTS.get_or_init(|| {
        serde_json::from_str(include_str!("../tailwind/defaults.json"))
            .expect("generated Tailwind defaults")
    })
}

/// Resolve `var(--token)` references that name a built-in token.
///
/// Utilities emit references such as `var(--spacing)` and the two font aliases reference each
/// other, so a bounded substitution over the frozen table is all the environment that is left.
/// Anything else was an author variable: report the boundary instead of leaving an unresolved
/// custom property for the backend, which would silently render nothing.
pub(crate) fn resolve(property: &str, raw: &str) -> Result<String, crate::style::StyleIssue> {
    const MAX_PASSES: usize = 8;
    let mut value = raw.to_owned();
    for _ in 0..MAX_PASSES {
        let Some((start, name)) = first_variable(&value) else {
            return Ok(value);
        };
        let Some(token) = defaults().get(&name) else {
            return Err(crate::style::StyleIssue::new(
                crate::style::StyleIssueKind::UnsupportedValue,
                property,
                raw,
                "CSS variable references are not supported",
            )
            .with_suggestion("a `const` at the top of the file, or a literal value"));
        };
        let end = matching_paren(&value, start).ok_or_else(|| {
            crate::style::StyleIssue::new(
                crate::style::StyleIssueKind::InvalidValue,
                property,
                raw,
                "unbalanced var() reference",
            )
        })?;
        value.replace_range(start..=end, token);
    }
    Err(crate::style::StyleIssue::new(
        crate::style::StyleIssueKind::InvalidValue,
        property,
        raw,
        "token aliases exceed the resolution depth",
    ))
}

/// Byte offset of the `var(` and the custom property name it names, ignoring nested functions.
fn first_variable(value: &str) -> Option<(usize, String)> {
    let bytes = value.as_bytes();
    let mut index = 0;
    while index + 4 <= bytes.len() {
        let window = value.get(index..index + 4)?;
        if window.eq_ignore_ascii_case("var(") {
            let rest = &value[index + 4..];
            let name = rest
                .trim_start()
                .split(|ch: char| ch == ',' || ch == ')' || ch.is_whitespace())
                .next()
                .unwrap_or_default()
                .to_owned();
            if name.starts_with("--") {
                return Some((index, name));
            }
        }
        index += 1;
    }
    None
}

/// Offset of the `)` closing the `var(` at `start`.
fn matching_paren(value: &str, start: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (offset, ch) in value[start..].char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(start + offset);
                }
            }
            _ => {}
        }
    }
    None
}

/// Namespace of a token name: `--color-red-500` → `color`.
#[cfg(test)]
fn namespace(name: &str) -> &str {
    name.trim_start_matches('-')
        .split('-')
        .next()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespaces_match_the_retained_list() {
        let mut seen = std::collections::BTreeSet::new();
        for name in defaults().keys() {
            seen.insert(namespace(name).to_string());
        }
        for name in &seen {
            assert!(
                RETAINED_NAMESPACES.contains(&name.as_str()),
                "`--{name}-*` has no declared consumer class; add a utility consumer or remove the \
                 entries"
            );
        }
        for retained in RETAINED_NAMESPACES {
            assert!(
                seen.contains(*retained),
                "`--{retained}-*` is listed as retained but missing from the table"
            );
        }
    }
}
