#![cfg(feature = "motion")]

use valle_compiler::motion::compile_motion;
use valle_draw::program::{DrawProgram, Group, MaskMode, Node, Paint, ShaderLayer};
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

/// Every node of `program`, a mask's source included, with the groups above it, outermost
/// first.
fn nodes_with_groups(program: &DrawProgram) -> Vec<(Vec<&Group>, &Node)> {
    let mut out = Vec::new();
    let mut stack: Vec<_> = program.roots().iter().map(|id| (*id, Vec::new())).collect();
    while let Some((id, above)) = stack.pop() {
        let node = &program.nodes()[id.raw() as usize];
        if let Node::Group(group) = node {
            let mut inner = above.clone();
            inner.push(group);
            stack.extend(group.children.iter().map(|child| (*child, inner.clone())));
            if let Some(mask) = &group.mask {
                stack.push((mask.source, inner));
            }
        }
        out.push((above, node));
    }
    out
}

/// The shader layer of a group, if it is the Glass shader with the uniform `name`.
fn glass_shader<'a>(group: &'a Group, name: &str) -> Option<&'a ShaderLayer> {
    group
        .shader
        .as_ref()
        .filter(|shader| shader.uniforms.iter().any(|uniform| uniform.name == name))
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
    // The optics layer and the rim layer.
    assert_eq!((frosts, surfaces), (1, 2));
    assert!(
        projections > 0,
        "the background still draws through the projection"
    );
}

#[test]
fn a_glass_inside_another_elements_projection_uses_viewport_optics() {
    for transform in [
        "perspective(300px) rotateY(30deg)",
        "perspective(300px) rotateX(20deg)",
    ] {
        for child in [
            r#"<Glass material={{ frost: 4 }} style={{ width: 100, height: 80 }} />"#,
            r#"<GlassField material={{ frost: 4 }}><Glass style={{ width: 100, height: 80 }} /><Glass style={{ width: 70, height: 60, left: 40 }} /></GlassField>"#,
        ] {
            let source = format!(
                r#"<View style={{{{ width: 200, height: 160, transform: "{transform}" }}}}>
          {child}
        </View>"#
            );
            let program = emit_program(&source, 240, 160).unwrap();
            let mut frosts = 0;
            let mut optics = 0;
            for (above, node) in nodes_with_groups(&program) {
                let Node::Group(group) = node else {
                    continue;
                };
                if group.backdrop.is_some() || !group.filters.is_empty() {
                    let world = above.iter().fold(
                        valle_draw::program::recording::Homography::IDENTITY,
                        |world, group| {
                            valle_draw::program::recording::Homography(group.transform.0)
                                .then(world)
                        },
                    );
                    assert!(
                        world.0[6].abs() < 1e-12 && world.0[7].abs() < 1e-12,
                        "blur still projected: {:?}",
                        world.0
                    );
                }
                frosts += usize::from(group.backdrop.is_some());
                optics += usize::from(group.shader.is_some());
            }
            assert_eq!((frosts, optics), (1, 2));
        }
    }
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

#[test]
fn a_glass_draws_its_presence_as_the_gray_its_shape_is_drawn_in() {
    let program = emit_program(
        r#"<Glass presence={0.5} style={{ width: 120, height: 80, borderRadius: 20 }} />"#,
        160,
        120,
    )
    .unwrap();
    let nodes = nodes_with_groups(&program);
    let mut fills = 0;
    for (above, node) in &nodes {
        let Node::Path(path) = node else {
            continue;
        };
        if !above.iter().any(|group| group.shader.is_some()) {
            continue;
        }
        let paint = &program.paints()[path.fill.unwrap().raw() as usize];
        let Paint::Solid(color) = paint else {
            panic!("a shape drawn in {paint:?}");
        };
        // Opaque, so the lens keeps its height; gray by the presence the shaders read.
        assert_eq!(
            [color.red, color.green, color.blue, color.alpha],
            [0.5, 0.5, 0.5, 1.0]
        );
        fills += 1;
    }
    // The optics layer's height field and the rim layer's shape.
    assert_eq!(fills, 2);
    assert!(
        nodes
            .iter()
            .all(|(_, node)| !matches!(node, Node::Group(group) if group.opacity < 1.0)),
        "presence does not fade the glass"
    );
}

#[test]
fn a_glass_draws_nothing_without_presence() {
    let program = emit_program(
        r##"<Glass presence={0} material={{ frost: 4, tint: "#ffffff40" }}
      style={{ width: 120, height: 80, borderRadius: 20 }} />"##,
        160,
        120,
    )
    .unwrap();
    assert!(nodes_with_groups(&program).iter().all(|(_, node)| {
        !matches!(node, Node::Group(group) if group.shader.is_some() || group.backdrop.is_some())
    }));
}

#[test]
fn the_rim_reaches_past_the_clip_the_optics_draws_in() {
    let program = emit_program(
        r#"<Glass style={{ position: "absolute", left: 20, top: 20, width: 120, height: 80,
      borderRadius: 20 }} />"#,
        160,
        120,
    )
    .unwrap();
    let nodes = nodes_with_groups(&program);
    let layer = |name: &str| {
        nodes
            .iter()
            .find_map(|(above, node)| match node {
                Node::Group(group) => glass_shader(group, name).map(|shader| (above, shader)),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no Glass shader with `{name}`"))
    };
    let (optics_above, optics) = layer("refraction");
    let (rim_above, rim) = layer("contour");
    let clips = |above: &[&Group]| above.iter().filter(|group| group.clip.is_some()).count();
    assert_eq!(clips(optics_above), clips(rim_above) + 1);
    // The dark edge lies just outside the shape, inside the rim's box.
    assert!(rim.bounds.width > optics.bounds.width + 2.0, "{rim:?}");
    assert!(rim.bounds.height > optics.bounds.height + 2.0, "{rim:?}");
}

#[test]
fn a_field_member_coming_in_brings_its_frost_in_by_its_gray() {
    let field = |presence: &str| {
        let program = emit_program(
            &format!(
                r#"<GlassField material={{{{ frost: 6 }}}}>
      <Glass presence={{{presence}}} style={{{{ position: "absolute", left: 10, top: 10,
        width: 80, height: 80, borderRadius: 40 }}}} />
      <Glass style={{{{ position: "absolute", left: 100, top: 10, width: 80, height: 80,
        borderRadius: 40 }}}} />
    </GlassField>"#
            ),
            200,
            100,
        )
        .unwrap();
        // Each mask, with whether its group frosts and whether it bends.
        let nodes = nodes_with_groups(&program);
        let mut masks = Vec::new();
        for (_, node) in &nodes {
            let Node::Group(group) = node else {
                continue;
            };
            let Some(mask) = &group.mask else {
                continue;
            };
            let under = |test: &dyn Fn(&Group) -> bool| {
                nodes.iter().any(|(above, node)| {
                    above.iter().any(|outer| std::ptr::eq(*outer, group))
                        && matches!(node, Node::Group(inner) if test(inner))
                })
            };
            masks.push((
                mask.mode,
                under(&|group| group.backdrop.is_some()),
                under(&|group| glass_shader(group, "refraction").is_some()),
            ));
        }
        masks.sort_by_key(|(mode, ..)| format!("{mode:?}"));
        masks
    };
    // Both in: one mask of the shapes' coverage holds the frost and the surface.
    assert_eq!(field("1"), [(MaskMode::Alpha, true, true)]);
    // One coming in: the frost is masked by the shapes' gray, the surface by their coverage.
    assert_eq!(
        field("0.5"),
        [
            (MaskMode::Alpha, false, true),
            (MaskMode::Luminance, true, false)
        ]
    );
}
