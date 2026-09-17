#![cfg(feature = "motion")]

use valle_compiler::motion::compile_motion;
use valle_motion::diag::{DiagClass, DiagCode};
use valle_motion::style::{StyleIssueKind, parse_declarations, parse_property};
use valle_motion::{MotionValue, StyleBinding, StyleValue};

fn source(attributes: &str) -> String {
    format!(
        "export default function Demo(ctx) {{return <Scene>\n  <View key=\"bad\" {attributes}/>\n</Scene>;}}"
    )
}

#[test]
fn css_and_arbitrary_properties_share_error_classification_and_value_details() {
    for (property, value, kind, class) in [
        (
            "made-up",
            "2px",
            StyleIssueKind::UnknownProperty,
            DiagClass::Illegal,
        ),
        (
            "mask-size",
            "cover",
            StyleIssueKind::UnsupportedProperty,
            DiagClass::Unsupported,
        ),
        (
            "width",
            "min-content",
            StyleIssueKind::UnsupportedValue,
            DiagClass::Unsupported,
        ),
        (
            "width",
            "clamp(10px,50%,100px)",
            StyleIssueKind::UnsupportedValue,
            DiagClass::Unsupported,
        ),
        (
            "grid-template-columns",
            "subgrid",
            StyleIssueKind::UnsupportedValue,
            DiagClass::Unsupported,
        ),
        (
            "overflow",
            "scroll",
            StyleIssueKind::UnsupportedValue,
            DiagClass::Unsupported,
        ),
        (
            "width",
            "12px垃圾",
            StyleIssueKind::InvalidValue,
            DiagClass::Illegal,
        ),
        (
            "filter",
            "blur(-2px)",
            StyleIssueKind::InvalidValue,
            DiagClass::Illegal,
        ),
    ] {
        let candidate = format!("[{property}:{value}]");
        let utility = source(&format!(
            "className={{{}}}",
            serde_json::to_string(&candidate).unwrap()
        ));
        let inline = source(&format!(
            "style={{{{{}: {}}}}}",
            serde_json::to_string(property).unwrap(),
            serde_json::to_string(value).unwrap()
        ));
        for authored in [&utility, &inline] {
            let diagnostics = compile_motion(authored).unwrap_err();
            let diagnostic = diagnostics
                .iter()
                .find(|d| d.style.as_ref().is_some_and(|d| d.property == property))
                .unwrap_or_else(|| panic!("{authored}: {diagnostics:?}"));
            let issue = diagnostic.style.as_ref().unwrap();
            assert_eq!(issue.kind, kind, "{diagnostics:?}");
            assert_eq!(diagnostic.class, class);
            assert_eq!(diagnostic.code, issue.code());
            assert!(!issue.reason.is_empty());
            assert!(diagnostic.node_path.is_some());
            assert_eq!(diagnostic.span.line, 2);
            let json = serde_json::to_value(diagnostic).unwrap();
            assert_eq!(json["style"]["property"], property);
            if authored == &utility {
                // Arbitrary math normalizes whitespace; utility preserves the authored token.
                let lowered = if value == "clamp(10px,50%,100px)" {
                    "clamp(10px, 50%, 100px)"
                } else {
                    value
                };
                assert_eq!(issue.value, lowered);
                assert_eq!(diagnostic.utility.as_deref(), Some(candidate.as_str()));
                assert_eq!(json["utility"], candidate);
            } else {
                assert_eq!(issue.value, value);
            }
        }
    }
}

#[test]
fn unselected_class_branch_points_to_the_bad_candidate() {
    let authored = source("className={ctx.localFrame<30?'w-20':'[mask-size:cover]'}");
    let diagnostics = compile_motion(&authored).unwrap_err();
    let diagnostic = diagnostics.iter().find(|d| d.utility.is_some()).unwrap();
    assert_eq!(diagnostic.code, DiagCode::StyleUnsupportedProperty);
    assert_eq!(diagnostic.style.as_ref().unwrap().value, "cover");
    assert_eq!(
        &authored[diagnostic.span.start as usize..diagnostic.span.end as usize],
        "'[mask-size:cover]'"
    );
    assert!(diagnostic.message.contains("all branches"));
}

#[test]
fn finite_inline_css_and_direct_artifacts_retain_the_shared_issue() {
    let authored = source("style={{width:ctx.localFrame<30?'calc(80px)':'min-content'}}");
    let diagnostics = compile_motion(&authored).unwrap_err();
    let diagnostic = diagnostics
        .iter()
        .find(|d| d.style.is_some())
        .unwrap_or_else(|| panic!("{diagnostics:?}"));
    assert_eq!(diagnostic.code, DiagCode::StyleUnsupportedValue);
    assert_eq!(diagnostic.style.as_ref().unwrap().value, "min-content");
    assert!(
        authored[diagnostic.span.start as usize..diagnostic.span.end as usize]
            .contains("ctx.localFrame")
    );

    let mut artifact = compile_motion(&source("")).unwrap().artifact;
    artifact.nodes[0].styles.push(StyleBinding {
        property: "width".into(),
        value: StyleValue::Static {
            value: MotionValue::Str("min-content".into()),
        },
    });
    let errors = artifact.validate().unwrap_err();
    let issue = errors
        .iter()
        .find_map(|error| error.style.as_ref())
        .unwrap();
    assert_eq!(issue, &parse_property("width", "min-content").unwrap_err());
    artifact.nodes[0].styles.clear();
    artifact.nodes[0]
        .class_names
        .push("[mask-size:cover]".into());
    let errors = artifact.validate().unwrap_err();
    let error = errors
        .iter()
        .find(|error| error.path.contains("classNames"))
        .unwrap();
    assert_eq!(
        error.style.as_ref().unwrap().kind,
        StyleIssueKind::UnsupportedProperty
    );
    assert!(error.message.contains("CSS mask lowering"));
}

#[test]
fn syntax_and_visual_utility_failures_keep_actionable_causes() {
    let syntax = compile_motion(&source("className='[width:calc(20px]' ")).unwrap_err();
    assert!(
        syntax
            .iter()
            .any(|d| d.code == DiagCode::SyntaxError && d.message.contains("balanced"))
    );
    let blur = compile_motion(&source("className='blur-[-2px]' ")).unwrap_err();
    let issue = blur.iter().find_map(|d| d.style.as_ref()).unwrap();
    assert_eq!(issue.property, "filter");
    assert_eq!(issue.value, "blur(-2px)");
    assert_eq!(issue.kind, StyleIssueKind::InvalidValue);
    // A utility that consumes a variable is a permanent boundary with a replacement.
    let variable = compile_motion(&source("className='w-[var(--size)]' ")).unwrap_err();
    let issue = variable
        .iter()
        .find_map(|d| d.style.as_ref())
        .expect("the boundary carries a structured style issue");
    assert_eq!(issue.kind, StyleIssueKind::UnsupportedValue);
    assert_eq!(issue.property, "w-[var(--size)]");
    assert!(
        issue
            .suggestion
            .as_deref()
            .is_some_and(|text| text.contains("literal value")),
        "{issue}"
    );
    assert!(
        variable
            .iter()
            .any(|d| d.utility.as_deref() == Some("w-[var(--size)]"))
    );
    let important = compile_motion(&source("className='!p-4' ")).unwrap_err();
    assert!(important.iter().any(|d| d.message.contains("p-4!")));
}

#[test]
fn value_limits_parse_tokens_and_apply_to_concrete_frame_declarations() {
    for (property, value) in [
        ("width", "m\\69n-content"),
        ("width", "MAX(20px,30px)"),
        ("grid-template-columns", "subgrid"),
    ] {
        assert_eq!(
            parse_property(property, value).unwrap_err().kind,
            StyleIssueKind::UnsupportedValue
        );
        assert!(parse_declarations(&format!("{property}:{value}")).is_err());
    }
    // Words inside a font name, identifier or comment are not CSS sizing functions/keywords.
    for (property, value) in [
        ("font-family", "'min-content'"),
        ("width", "20px /* min-content */"),
        ("width", "calc(100% - 20px)"),
        ("overflow", "hidden"),
    ] {
        assert!(
            parse_property(property, value).is_ok(),
            "{property}: {value}"
        );
    }
}
