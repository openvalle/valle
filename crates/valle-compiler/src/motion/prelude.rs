//! Deterministic prepare-time JavaScript surface exposed to Motion authors.

pub(super) const STATIC_HELPERS: &str = include_str!("prelude.js");

/// Whether the helpers declare `name`. Module-level declarations run in the same script as the
/// helpers, so none of them can reuse such a name.
pub(super) fn declares(name: &str) -> bool {
    STATIC_HELPERS.lines().any(|line| {
        line.strip_prefix("const ")
            .and_then(|rest| rest.split_once(' '))
            .is_some_and(|(declared, _)| declared == name)
    })
}
