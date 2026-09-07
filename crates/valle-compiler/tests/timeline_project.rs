//! Timeline project compilation accepts only the public sparse Timeline.

use valle_compiler::{compile_bundle, compile_project, parse_bundle};
use valle_timeline::internal::canonical_bytes;

const TIMELINE: &str = r##"{
  "canvas":{"width":320,"height":180,"fps":"30000/1001"},
  "tracks":{"visual":[{"clips":[{
    "start":0,"duration":5,"kind":"solid","color":"#000000ff"
  }]}]}
}"##;

#[test]
fn project_and_bundle_return_the_same_canonical_identity() {
    let entries = parse_bundle(TIMELINE);
    let project = compile_project(&entries).unwrap();
    let timeline = compile_bundle(&entries).unwrap();

    assert_eq!(
        canonical_bytes(&project.timeline).unwrap(),
        canonical_bytes(&timeline).unwrap()
    );
    assert_eq!(
        canonical_bytes(&project.timeline).unwrap(),
        canonical_bytes(&timeline).unwrap()
    );
}

#[test]
fn sidecars_cannot_change_document_identity() {
    let with_sidecar = parse_bundle(&format!(
        "-- timeline.json --\n{TIMELINE}\n-- components/ignored.tsx --\nthrow new Error('not a Timeline resource binding');\n"
    ));
    let plain = compile_bundle(&parse_bundle(TIMELINE)).unwrap();
    let sidecar = compile_bundle(&with_sidecar).unwrap();
    assert_eq!(
        canonical_bytes(&plain).unwrap(),
        canonical_bytes(&sidecar).unwrap()
    );
}

#[test]
fn malformed_or_unsupported_json_never_reaches_a_generic_serde_fallback() {
    for invalid in [
        r#"{"canvas":{"width":320,"height":180}}"#,
        r#"{"version":"4.0","canvas":{"width":320,"height":180}}"#,
        r#"{"document":{"unknown":true}}"#,
    ] {
        assert!(
            compile_project(&parse_bundle(invalid)).is_err(),
            "{invalid}"
        );
    }
}
