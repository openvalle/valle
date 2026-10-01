#![cfg(feature = "motion")]

use valle_compiler::motion::compile_motion;
use valle_motion::layout::SceneReferenceKind;
use valle_motion::{NodeId, glass::MOTION_GLASS_CAPABILITY, prepare_scene};

#[test]
fn production_compiler_admits_glass_tags_and_declares_capability() {
    for tag in ["Glass", "GlassField"] {
        let source = if tag == "Glass" {
            r#"

export default function Scene() {
  return <Glass surfaceId="hero-lens" shape={{ kind: "circle" }} />;
}
"#
            .to_string()
        } else {
            r#"

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
fn scene_dependencies_mark_glass_field_and_backdrop_displacement_reads() {
    let source = r#"
export default function Scene() {
  return <Scene style={{width:320,height:180}}>
    <GlassField key="field" fieldId="orbit">
      <Glass key="glass" surfaceId="lens" shape={{kind:"circle"}}
        style={{width:80,height:80}} />
    </GlassField>
    <View key="displaced" style={{width:80,height:80,
      backdropDisplacement:displacement(17,point(0.01,0.02),4)}} />
  </Scene>;
}
"#;
    let artifact = compile_motion(source).unwrap().artifact;
    assert!(artifact.reads_destination());
    let prepared = prepare_scene(&artifact).unwrap();
    for key in ["field", "glass", "displaced"] {
        let at = artifact
            .nodes
            .iter()
            .position(|node| node.key == key)
            .unwrap();
        let facts = prepared.dependencies().node(NodeId(at as u32)).unwrap();
        assert!(facts.reads_backdrop, "{key}");
        assert!(facts.composition_boundary, "{key}");
    }
    let node_id = |key: &str| {
        NodeId(
            artifact
                .nodes
                .iter()
                .position(|node| node.key == key)
                .unwrap() as u32,
        )
    };
    assert!(prepared.dependencies().references().iter().any(|edge| {
        edge.consumer == node_id("glass")
            && edge.source == node_id("field")
            && edge.kind == SceneReferenceKind::GlassField
    }));
}

#[test]
fn artifact_requires_motion_glass_capability_exactly_when_used() {
    let glass_source = r#"

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
