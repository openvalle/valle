#![cfg(feature = "motion")]

use valle_compiler::motion::compile_motion;
use valle_draw::{
    program::{DrawProgram, Node},
    transition::TransitionKind,
};
use valle_motion::{
    Fonts, LayoutOptions, TRANSITION_CAPABILITY, Viewport, build_tree, default_font_naming, emit,
    motion_context_at_frame, prepare_scene, resolve_props,
};
use valle_timeline::FrameRate;

const MOTION_TRANSITION: &str =
    include_str!("fixtures/motion/composition/motion-transition.motion.tsx");

fn program(source: &str, frame: u32) -> Result<DrawProgram, String> {
    let artifact = compile_motion(source)
        .map_err(|e| format!("{e:?}"))?
        .artifact;
    let scene = prepare_scene(&artifact).map_err(|e| e.to_string())?;
    let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
    let fonts = Fonts::default();
    let ctx = motion_context_at_frame(frame, 60, FrameRate::new(30, 1).unwrap()).unwrap();
    let tree = build_tree(
        &scene,
        &ctx,
        &props,
        &LayoutOptions {
            viewport: Viewport::new((640, 360)),
            fonts: &fonts,
            styles: None,
        },
    )
    .map_err(|e| e.to_string())?;
    Ok(emit(&tree, &default_font_naming)
        .map_err(|e| e.to_string())?
        .program)
}

#[test]
fn all_kernels_keep_two_independent_inputs_and_frame_progress() {
    for effect in TransitionKind::ALL {
        let name = serde_json::to_value(effect)
            .unwrap()
            .as_str()
            .unwrap()
            .to_owned();
        let source = MOTION_TRANSITION.replace("circleOpen", &name);
        for frame in [30, 0, 59, 30] {
            let output = program(&source, frame).unwrap();
            let groups = output
                .nodes()
                .iter()
                .filter_map(|node| match node {
                    Node::Group(group) if group.transition.is_some() => Some(group),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(groups.len(), 1);
            assert_eq!(groups[0].children.len(), 2);
            assert!(groups[0].isolated);
            let transition = groups[0].transition.as_ref().unwrap();
            assert_eq!(transition.kind, effect);
            assert_eq!(transition.progress, frame as f32 / 60.0);
            assert_eq!(
                (transition.bounds.width, transition.bounds.height),
                (640.0, 360.0)
            );
            output.validate().unwrap();
        }
    }
}

#[test]
fn hidden_and_empty_endpoints_still_supply_two_inputs() {
    for attributes in ["visible={false}", r#"style={{display:"none"}}"#, ""] {
        let source = format!(
            r#"export default function T(ctx) {{ return <Transition kind="fade" progress={{0.5}} style={{{{width:64,height:64}}}}><View key="from" {attributes}/><View key="to"/></Transition>; }}"#
        );
        let output = program(&source, 0).unwrap();
        assert!(output.nodes().iter().any(
            |node| matches!(node, Node::Group(g) if g.transition.is_some() && g.children.len() == 2)
        ));
    }
}

#[test]
fn invalid_children_kind_progress_and_capability_are_rejected() {
    for body in [
        r#"<Transition kind="fade" progress={0.5}><View/></Transition>"#,
        r#"<Transition kind="fade" progress={0.5}><View/><View/><View/></Transition>"#,
        r#"<Transition kind="unknown" progress={0.5}><View/><View/></Transition>"#,
        r#"<Transition kind="fade" progress={true}><View/><View/></Transition>"#,
        r#"<Transition kind="fade"><View/><View/></Transition>"#,
        r#"<Transition progress={0.5}><View/><View/></Transition>"#,
        r#"<Transition kind="fade" progress={-0.1}><View/><View/></Transition>"#,
        r#"<Transition kind="fade" progress={1.1}><View/><View/></Transition>"#,
    ] {
        assert!(
            compile_motion(&format!(
                "export default function T(ctx) {{ return {body}; }}"
            ))
            .is_err(),
            "{body}"
        );
    }
    let mut artifact = compile_motion(MOTION_TRANSITION).unwrap().artifact;
    artifact
        .capability_set
        .names
        .retain(|name| name != TRANSITION_CAPABILITY);
    assert!(artifact.validate().is_err());
    let error = program(
        &MOTION_TRANSITION.replace("ctx.progress", "ctx.seconds"),
        59,
    )
    .unwrap_err();
    assert!(error.contains("[0, 1]"), "{error}");
}

#[test]
fn parameters_are_typed_animated_validated_and_emitted() {
    let source = MOTION_TRANSITION.replace(
        "progress={ctx.progress}",
        "progress={0.5} params={{centerX:ctx.seconds/2,centerY:0.25,softness:3}}",
    );
    let mut previous = None;
    for frame in [30, 0, 59, 30] {
        let output = program(&source, frame).unwrap();
        let params = output
            .nodes()
            .iter()
            .find_map(|node| match node {
                Node::Group(g) => g.transition.as_ref().map(|t| t.params),
                _ => None,
            })
            .unwrap();
        assert_eq!(params.0, [frame as f32 / 60.0, 0.25, 3.0, 0.0]);
        if frame == 30 {
            if let Some(first) = &previous {
                assert_eq!(&output.packed_bytes().unwrap(), first);
            }
            previous = Some(output.packed_bytes().unwrap());
        }
    }
    for params in [
        "{unknown:1}",
        "{centerX:2}",
        "{centerY:true}",
        "{softness:-1}",
        "{centerX:0,centerX:1}",
    ] {
        let bad = MOTION_TRANSITION.replace(
            "progress={ctx.progress}",
            &format!("progress={{0.5}} params={{{params}}}"),
        );
        assert!(compile_motion(&bad).is_err(), "{params}");
    }
    let nonfinite = MOTION_TRANSITION.replace(
        "progress={ctx.progress}",
        "progress={0.5} params={{softness:1/0}}",
    );
    assert!(program(&nonfinite, 0).is_err());
    let dynamic_bad = MOTION_TRANSITION.replace(
        "progress={ctx.progress}",
        "progress={0.5} params={{centerX:ctx.seconds}}",
    );
    assert!(program(&dynamic_bad, 59).unwrap_err().contains("centerX"));
    let zero_direction = MOTION_TRANSITION
        .replace("circleOpen", "directionalWarp")
        .replace(
            "progress={ctx.progress}",
            "progress={0.5} params={{directionX:0,directionY:0}}",
        );
    assert!(compile_motion(&zero_direction).is_err());
    let animated_direction = zero_direction.replace("directionX:0", "directionX:ctx.seconds/2");
    assert!(compile_motion(&animated_direction).is_ok());
    assert!(
        program(&animated_direction, 0)
            .unwrap_err()
            .contains("nonzero")
    );
    assert!(program(&animated_direction, 30).is_ok());

    let mut tampered = compile_motion(MOTION_TRANSITION).unwrap().artifact;
    if let valle_motion::NodeKind::Transition { params, .. } = &mut tampered
        .nodes
        .iter_mut()
        .find(|node| matches!(node.kind, valle_motion::NodeKind::Transition { .. }))
        .unwrap()
        .kind
    {
        params.insert(
            "unknown".into(),
            valle_motion::NumberValue::Static { value: 1.0 },
        );
    }
    assert!(tampered.validate().is_err());
}
