#![cfg(feature = "motion")]

use valle_compiler::motion::compile_motion;
use valle_draw::program::{DrawProgram, Group, Node};
use valle_motion::layout::SceneReferenceKind;
use valle_motion::{
    Fonts, LayoutOptions, NodeId, Viewport, build_tree, default_font_naming, emit,
    glass::MOTION_GLASS_CAPABILITY, motion_context_at_frame, prepare_scene, resolve_props,
};
use valle_timeline::FrameRate;

/// The program of the first frame of a `width`×`height` scene holding `element`, or the error
/// emitting it fails with.
fn emit_program(element: &str, width: u32, height: u32) -> Result<DrawProgram, String> {
    let source = format!(
        "export const composition = {{ width: {width}, height: {height}, fps: 30, duration: 1 }};\n\
         export default function Test() {{\n  return <Scene style={{{{ width: {width}, height: {height} }}}}>{element}</Scene>;\n}}\n"
    );
    let artifact = compile_motion(&source).unwrap().artifact;
    let prepared = prepare_scene(&artifact).unwrap();
    let context = motion_context_at_frame(0, 30, FrameRate::new(30, 1).unwrap()).unwrap();
    let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
    let fonts = Fonts::default();
    let tree = build_tree(
        &prepared,
        &context,
        &props,
        &LayoutOptions {
            viewport: Viewport::new((width, height)),
            fonts: &fonts,
            styles: None,
        },
    )
    .unwrap();
    emit(&tree, &default_font_naming)
        .map(|output| output.program)
        .map_err(|error| error.to_string())
}

/// The error emitting the first frame of a `width`×`height` scene holding `element` fails with.
fn emit_error(element: &str, width: u32, height: u32) -> String {
    emit_program(element, width, height)
        .err()
        .unwrap_or_default()
}

#[test]
fn production_compiler_admits_glass_tags_and_declares_capability() {
    for tag in ["Glass", "GlassField"] {
        let source = if tag == "Glass" {
            r#"

export default function Scene() {
  return <Glass style={{ width: 80, height: 80, borderRadius: "50%" }} />;
}
"#
            .to_string()
        } else {
            r#"

export default function Scene() {
  return (
    <GlassField>
      <Glass style={{ width: 80, height: 80, borderRadius: "50%" }} />
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
    <GlassField key="field">
      <Glass key="glass" style={{width:80,height:80,borderRadius:40}} />
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
  return <Glass style={{ width: 80, height: 80 }} />;
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

#[test]
fn a_glass_blurs_on_the_plane_its_own_projection_maps_it_onto() {
    let program = emit_program(
        r##"<Glass material={{ frost: 4 }}
      style={{ width: 120, height: 80, backgroundColor: "#ffffff20",
        transform: "perspective(300px) rotateY(30deg)" }} />"##,
        240,
        160,
    )
    .unwrap();
    let projective = |group: &Group| group.transform.0[6] != 0.0 || group.transform.0[7] != 0.0;
    let (mut frosts, mut surfaces, mut projections) = (0, 0, 0);
    let mut stack: Vec<_> = program.roots().iter().map(|id| (*id, false)).collect();
    while let Some((id, projected)) = stack.pop() {
        let Node::Group(group) = &program.nodes()[id.raw() as usize] else {
            continue;
        };
        let projected = projected || projective(group);
        // No blur runs under a projection, where it would have no single radius.
        assert!(
            !projected || (group.backdrop.is_none() && group.filters.is_empty()),
            "a blur under a projection: {group:?}"
        );
        frosts += usize::from(group.backdrop.is_some());
        surfaces += usize::from(group.shader.is_some());
        projections += usize::from(projective(group));
        stack.extend(group.children.iter().map(|child| (*child, projected)));
    }
    assert_eq!((frosts, surfaces), (1, 1));
    assert!(
        projections > 0,
        "the background still draws through the projection"
    );
}

#[test]
fn a_glass_inside_another_elements_projection_is_refused() {
    let refusal = "a Glass inside another element's CSS 3D transform or perspective cannot blur";
    let error = emit_error(
        r#"<View style={{ width: 200, height: 160, transform: "perspective(300px) rotateY(30deg)" }}>
      <Glass style={{ width: 100, height: 80 }} />
    </View>"#,
        240,
        160,
    );
    assert!(error.contains(refusal), "{error}");
    let error = emit_error(
        r#"<View style={{ width: 200, height: 160, transform: "perspective(300px) rotateX(20deg)" }}>
      <GlassField><Glass style={{ width: 80, height: 80 }} /></GlassField>
    </View>"#,
        240,
        160,
    );
    assert!(error.contains(refusal), "{error}");
}

#[test]
fn a_glass_beyond_a_shader_layers_area_is_refused_as_glass() {
    let error = emit_error(
        r#"<Glass style={{ width: 2000, height: 1100 }} />"#,
        2048,
        1152,
    );
    assert!(
        error.contains(
            "Glass draws 2000x1100 local pixels; one Glass or GlassField draws at most 2073600 (1920x1080)"
        ),
        "{error}"
    );
    let error = emit_error(
        r#"<GlassField>
      <Glass style={{ position: "absolute", left: 0, top: 0, width: 1000, height: 1100 }} />
      <Glass style={{ position: "absolute", left: 1000, top: 0, width: 1000, height: 1100 }} />
    </GlassField>"#,
        2048,
        1152,
    );
    assert!(
        error.contains("Glass draws 2004x1104 local pixels"),
        "{error}"
    );
}
