#![cfg(feature = "motion")]

use valle_compiler::motion::compile_motion;
use valle_draw::{
    program::{DrawProgram, Node, ShaderUniformValue, TransitionKernel},
    transition::TransitionKind,
};
use valle_motion::{
    Fonts, LayoutOptions, SHADER_LAYER_CAPABILITY, TRANSITION_CAPABILITY, Viewport, build_tree,
    default_font_naming, emit, motion_context_at_frame, prepare_scene, resolve_props,
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
            assert!(
                matches!(transition.kernel, TransitionKernel::Builtin { kind, .. } if kind == effect)
            );
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
            r#"export default function T() {{ const ctx = useTime(); return <Transition kind="fade" progress={{0.5}} style={{{{width:64,height:64}}}}><View key="from" {attributes}/><View key="to"/></Transition>; }}"#
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
                "export default function T() {{ const ctx = useTime(); return {body}; }}"
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
                Node::Group(g) => match g.transition.as_ref().map(|t| &t.kernel) {
                    Some(TransitionKernel::Builtin { params, .. }) => Some(*params),
                    _ => None,
                },
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
    if let valle_motion::NodeKind::Transition {
        effect: valle_motion::TransitionEffect::Builtin { params, .. },
        ..
    } = &mut tampered
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

const WIPE: &str = "const wipe = `
uniform float softness;

float4 valle_main(float2 uv) {
  float edge = smoothstep(progress - softness, progress + softness, uv.x);
  return mix(sample_to(uv), sample_from(uv), edge);
}`;
";

fn wipe_scene(attributes: &str) -> String {
    format!(
        "{WIPE}{}",
        MOTION_TRANSITION.replace(r#"kind="circleOpen""#, attributes)
    )
}

fn messages(source: &str) -> Vec<String> {
    compile_motion(source)
        .map(|_| Vec::new())
        .unwrap_or_else(|diagnostics| {
            diagnostics
                .into_iter()
                .map(|diagnostic| diagnostic.message)
                .collect()
        })
}

#[test]
fn code_reads_both_children_and_the_progress() {
    let source = wipe_scene("code={wipe} uniforms={{ softness: 0.1 + ctx.progress / 10 }}");
    let artifact = compile_motion(&source).unwrap().artifact;
    artifact.validate().unwrap();
    assert_eq!(artifact.shaders.len(), 1);
    let names = &artifact.capability_set.names;
    assert!(names.iter().any(|name| name == TRANSITION_CAPABILITY));
    assert!(!names.iter().any(|name| name == SHADER_LAYER_CAPABILITY));
    let uri = artifact.shaders[0].uri().unwrap().to_string();
    for frame in [30, 0, 59] {
        let output = program(&source, frame).unwrap();
        let transition = output
            .nodes()
            .iter()
            .find_map(|node| match node {
                Node::Group(group) => group.transition.as_ref(),
                _ => None,
            })
            .unwrap();
        let TransitionKernel::Shader {
            shader,
            uniforms,
            textures,
        } = &transition.kernel
        else {
            panic!("{:?}", transition.kernel);
        };
        assert_eq!(shader.uri, uri);
        assert!(textures.is_empty());
        let progress = frame as f32 / 60.0;
        assert_eq!(transition.progress, progress);
        assert_eq!(uniforms.len(), 1);
        assert_eq!(uniforms[0].name, "softness");
        assert!(
            matches!(uniforms[0].value, ShaderUniformValue::Float(value)
                if (value - (0.1 + progress / 10.0)).abs() < 1e-6),
            "{:?}",
            uniforms[0].value
        );
        assert_eq!(output.requirements().runtime_shaders.len(), 1);
        output.validate().unwrap();
    }

    let mut unknown = artifact.clone();
    for node in &mut unknown.nodes {
        if let valle_motion::NodeKind::Transition {
            effect: valle_motion::TransitionEffect::Shader { program, .. },
            ..
        } = &mut node.kind
        {
            program.uri = "shader://unknown".into();
        }
    }
    assert!(unknown.validate().is_err());

    let declared = |code: &str| {
        format!(
            "const wipe = `{code}`;\n{}",
            MOTION_TRANSITION.replace(r#"kind="circleOpen""#, "code={wipe}")
        )
    };
    for (source, expected) in [
        (
            wipe_scene(r#"kind="fade" code={wipe} uniforms={{ softness: 0.1 }}"#),
            "either a built-in kind or code",
        ),
        (
            wipe_scene("code={wipe} uniforms={{ softness: 0.1 }} params={{ softness: 1 }}"),
            "params configure a built-in kind",
        ),
        (
            wipe_scene(r#"kind="fade" uniforms={{ softness: 0.1 }}"#),
            "inputs and uniforms bind its code",
        ),
        (
            wipe_scene("code={wipe}"),
            "Transition code requires uniform `softness`",
        ),
        (
            wipe_scene("code={wipe} uniforms={{ softness: 0.1, width: 1 }}"),
            "Transition code has no uniform `width`",
        ),
        (
            declared(
                "uniform float progress; float4 valle_main(float2 uv) { return sample_from(uv); }",
            ),
            "Transition code",
        ),
        (
            declared("float4 valle_main(float2 uv) { return sampleContent(uv); }"),
            "Transition code",
        ),
        (
            wipe_scene("code={wipe} uniforms={{ softness: 0.1 }}")
                .replace("return mix(", "return blend("),
            "Transition code, line 6, column ",
        ),
    ] {
        let messages = messages(&source);
        assert!(
            messages.iter().any(|message| message.contains(expected)),
            "{expected}: {messages:#?}"
        );
        assert_eq!(messages.len(), 1, "{expected}: {messages:#?}");
    }
}
