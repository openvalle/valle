//! Guards for the private `--tw-*` utility-composition slots.
//!
//! These slots are internal implementation detail, not author-facing CSS variables. The style
//! convergence work deletes the author variable path while keeping every site below wired, so
//! this test is the checklist that must stay green before and after that deletion. It reads the
//! crate sources at run time so a migrated site reports as "missing" instead of failing to build.

use std::{fs, path::PathBuf};

const MANIFEST: &str = env!("CARGO_MANIFEST_DIR");

fn read(relative: &str) -> Option<String> {
    let path = PathBuf::from(MANIFEST).join(relative);
    fs::read_to_string(path).ok()
}

/// (file, needle, why the slot path must keep this)
const REQUIRED: &[(&str, &str, &str)] = &[
    (
        "src/tailwind/normalize.rs",
        "strip_prefix(\"--tw-\")",
        "important bank rename for visual utilities",
    ),
    (
        "src/tailwind/normalize.rs",
        "\"--tw-important-{slot}:{value};\"",
        "important bank write",
    ),
    (
        "src/tailwind/normalize.rs",
        "\"--tw-{}{slot}:{value};\"",
        "slot write carrying the important prefix",
    ),
    (
        "src/tailwind/normalize.rs",
        "\"var(--tw-important-{slot},var(--tw-{slot},{value}))\"",
        "normal/important fallback read",
    ),
    (
        "src/tailwind/normalize.rs",
        "\"--tw-{}{property}\"",
        "presence slot for arbitrary declarations",
    ),
    (
        "src/tailwind/normalize.rs",
        "name.replacen(\"--tw-\", \"--tw-important-\", 1)",
        "custom-property presence slot rename",
    ),
    (
        "src/tailwind/normalize.rs",
        "var(--tw-shadow-color,{})",
        "shadow colour chain",
    ),
    (
        "src/tailwind/visual.rs",
        "fn slot(name: &str, fallback: &str) -> String",
        "transform/scale/translate slot reader",
    ),
    (
        "src/tailwind/visual.rs",
        "\"--tw-translate-{axis}\"",
        "translate axis slot",
    ),
    (
        "src/tailwind/visual.rs",
        "\"--tw-scale-{axis}\"",
        "scale axis slot",
    ),
    (
        "src/tailwind/visual.rs",
        "format!(\"--tw-{prefix}\")",
        "filter/backdrop slot",
    ),
    (
        "src/style.rs",
        "marker.replacen(\"--tw-\", \"--tw-important-\", 1)",
        "transform state marker priority",
    ),
    (
        "src/style.rs",
        "--tw-motion-scale-state",
        "scale identity state constant",
    ),
    (
        "src/style.rs",
        "--tw-important-motion-scale-state",
        "scale identity state constant (important bank)",
    ),
];

#[test]
fn internal_composition_slots_stay_wired() {
    let mut missing = Vec::new();
    for (file, needle, why) in REQUIRED {
        match read(file) {
            Some(source) if source.contains(needle) => {}
            Some(_) => missing.push(format!("{file}: `{needle}` ({why})")),
            None => missing.push(format!("{file}: file missing ({why})")),
        }
    }
    assert!(
        missing.is_empty(),
        "internal --tw-* composition slots changed; verify the migration instead of deleting them:\n{}",
        missing.join("\n")
    );
}

#[test]
fn author_custom_property_rejection_stays_in_place() {
    // Author variables are gone; the rejection has to stay so the boundary keeps its message.
    let source = read("src/style/property.rs").expect("style/property.rs exists");
    assert!(
        source.contains("author CSS custom properties are not supported"),
        "author custom properties must keep being rejected with the boundary message"
    );
}
