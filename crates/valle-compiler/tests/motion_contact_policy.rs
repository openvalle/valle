#![cfg(feature = "motion")]

use valle_compiler::motion::compile_motion;
use valle_motion::{DiagClass, DiagCode};

const SOURCE: &str = include_str!("fixtures/motion/composition/contact-warning.motion.tsx");

#[test]
fn tangent_contact_warns_by_default_and_can_be_an_error() {
    let warned = compile_motion(SOURCE).expect("a tangent contact remains a valid morph");
    assert_eq!(warned.warnings.len(), 1, "{:?}", warned.warnings);
    assert_eq!(warned.warnings[0].class, DiagClass::Warning);
    assert_eq!(warned.warnings[0].code, DiagCode::MorphContact);
    assert!(warned.warnings[0].message.contains("0.500000000000"));

    let default = SOURCE.replace(", contactPolicy: \"warn\"", "");
    let default_warnings = compile_motion(&default).unwrap().warnings;
    assert_eq!(default_warnings.len(), 1);
    assert_eq!(default_warnings[0].code, DiagCode::MorphContact);
    let sequence = SOURCE.replace(
        "morph(A, B, ctx.progress,",
        "morphSequence([A, B], [0, 1], ctx.progress,",
    );
    assert_eq!(compile_motion(&sequence).unwrap().warnings.len(), 1);

    let strict = SOURCE.replace("contactPolicy: \"warn\"", "contactPolicy: \"error\"");
    let diagnostics = compile_motion(&strict).unwrap_err();
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == DiagCode::BuiltinRejected && diagnostic.message.contains("Contact")
    }));

    let bypass = SOURCE.replace(
        "contactPolicy: \"warn\"",
        "allowSelfIntersection: true, contactPolicy: \"warn\"",
    );
    assert!(compile_motion(&bypass).unwrap().warnings.is_empty());
    let invalid = SOURCE.replace("contactPolicy: \"warn\"", "contactPolicy: \"ignore\"");
    assert!(compile_motion(&invalid).is_err());
}
