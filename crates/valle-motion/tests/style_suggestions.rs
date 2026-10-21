//! Permanent rejections must name a replacement the current version accepts.
//!
//! "Failure is loud" only helps if the message says what to write instead, and a replacement that
//! points at a removed or unbuilt feature is worse than none. This test walks the property
//! rejections that survive the style convergence and checks that each one carries a suggestion, is
//! rendered in the message, and does not reference the CSS syntax the project deliberately does not
//! implement.

use valle_motion::style::{StyleIssue, StyleIssueKind, property_spec};

/// Properties and values the project rejects on purpose, with the surface the suggestion must name.
const REJECTIONS: &[(&str, &str, &str)] = &[
    ("animation", "spin 1s linear infinite", "interpolate"),
    ("animation-name", "spin", "interpolate"),
    ("transition", "opacity 0.2s", "interpolate"),
    ("clip-path", "circle(40%)", "Clip"),
    ("mask-image", "url(a.png)", "Mask"),
    ("offset-path", "path('M0 0')", "motionPath"),
    ("border-image", "url(a.png) 30", "border"),
    ("width", "fit-content", "measureText"),
    ("width", "min(10px, 5%)", "calc"),
    ("grid-template-columns", "subgrid", "grid-cols"),
    ("overflow", "auto", "overflow-hidden"),
];

fn issue(property: &str, value: &str) -> StyleIssue {
    let spec = property_spec(property);
    if let Err(limit) = spec.check_value(value) {
        return limit;
    }
    match spec.admit() {
        Err(error) => StyleIssue::admission(error, value),
        Ok(()) => panic!("`{property}: {value}` is admitted; move it out of REJECTIONS"),
    }
}

#[test]
fn every_permanent_rejection_names_a_working_replacement() {
    for (property, value, expected) in REJECTIONS {
        let issue = issue(property, value);
        let suggestion = issue
            .suggestion
            .as_deref()
            .unwrap_or_else(|| panic!("`{property}: {value}` has no suggestion: {issue}"));
        assert!(
            suggestion.contains(expected),
            "`{property}: {value}` should point at `{expected}`, got `{suggestion}`"
        );
        assert!(
            issue.to_string().contains("; use "),
            "the replacement must appear in the message: {issue}"
        );
        assert!(
            matches!(
                issue.kind,
                StyleIssueKind::UnsupportedProperty | StyleIssueKind::UnsupportedValue
            ),
            "`{property}: {value}` should be reported as unsupported, got {:?}",
            issue.kind
        );
    }
}

#[test]
fn suggestions_never_ask_for_css_the_project_removed() {
    // The convergence deleted author-facing CSS theme, variables, and responsive variants, and it never
    // implemented keyframes;
    // a suggestion must not send an author back to any of them.
    let forbidden = [
        "@theme",
        "var(--",
        "md:",
        "sm:",
        "@keyframes",
        "animation-",
        "transition:",
    ];
    for (property, value, _) in REJECTIONS {
        let issue = issue(property, value);
        let suggestion = issue.suggestion.clone().unwrap_or_default();
        for token in forbidden {
            assert!(
                !suggestion.contains(token),
                "`{property}: {value}` suggests removed syntax `{token}`: {suggestion}"
            );
        }
    }
}

/// The guide's Unsupported CSS tables come from the same records as the diagnostics. A stale table
/// fails; `VALLE_UPDATE_DOCS=1` rewrites it in place.
#[test]
fn the_guide_lists_the_admission_records() {
    const START: &str = "<!-- Generated from the admission records; VALLE_UPDATE_DOCS=1 cargo test \
                         -p valle-motion --test style_suggestions rewrites it. -->\n";
    const END: &str = "<!-- End of the generated tables. -->";
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/motion.md");
    let guide = std::fs::read_to_string(path).expect("docs/motion.md");
    let (head, rest) = guide
        .split_once(START)
        .expect("the guide marks the generated tables");
    let (current, tail) = rest
        .split_once(END)
        .expect("the guide ends the generated tables");
    let expected = format!("\n{}\n", valle_motion::style::unsupported_css_markdown());
    if current == expected {
        return;
    }
    if std::env::var_os("VALLE_UPDATE_DOCS").is_some() {
        std::fs::write(path, format!("{head}{START}{expected}{END}{tail}"))
            .expect("rewrite docs/motion.md");
        return;
    }
    panic!(
        "the Unsupported CSS tables in docs/motion.md differ from the admission records; rerun \
         with VALLE_UPDATE_DOCS=1"
    );
}
