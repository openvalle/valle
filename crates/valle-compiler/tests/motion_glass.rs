#![cfg(feature = "motion")]

use valle_compiler::motion::compile_motion;
use valle_motion::glass::MOTION_GLASS_CAPABILITY;

#[test]
fn production_compiler_admits_glass_tags_and_declares_capability() {
    for tag in ["Glass", "GlassField"] {
        let source = if tag == "Glass" {
            r#"
export const component = "glass-open";
export default function Scene() {
  return <Glass surfaceId="hero-lens" shape={{ kind: "circle" }} />;
}
"#
            .to_string()
        } else {
            r#"
export const component = "glass-open";
export default function Scene() {
  return (
    <GlassField fieldId="orbit">
      <Glass surfaceId="a" shape={{ kind: "circle" }}
        style={{ width: 80, height: 80 }} />
    </GlassField>
  );
}
"#
            .to_string()
        };
        let compiled = compile_motion(&source).unwrap_or_else(|error| {
            panic!("{tag}: {error:?}");
        });
        assert!(
            compiled
                .artifact
                .capability_set
                .names
                .contains(&MOTION_GLASS_CAPABILITY.to_string()),
            "{tag}: capability must be declared"
        );
        compiled.artifact.validate().expect("artifact validates");
    }
}

#[test]
fn production_capability_registry_lists_motion_glass() {
    assert!(valle_motion::OPTIONAL_CAPABILITIES.contains(&MOTION_GLASS_CAPABILITY));
}

#[test]
fn artifact_requires_motion_glass_capability_exactly_when_used() {
    let glass_source = r#"
export const component = "glass";
export default function Scene() {
  return <Glass surfaceId="lens" shape={{ kind: "circle" }} />;
}
"#;
    let mut glass = compile_motion(glass_source).unwrap().artifact;
    glass
        .capability_set
        .names
        .retain(|name| name != MOTION_GLASS_CAPABILITY);
    let errors = glass.validate().unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.message.contains("does not declare `motion-glass`")),
        "{errors:?}"
    );

    let plain_source = r#"
export const component = "plain";
export default function Scene() { return <Group />; }
"#;
    let mut plain = compile_motion(plain_source).unwrap().artifact;
    plain
        .capability_set
        .names
        .push(MOTION_GLASS_CAPABILITY.to_owned());
    plain.capability_set.names.sort();
    let errors = plain.validate().unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.message.contains("has no Motion Glass node")),
        "{errors:?}"
    );
}
