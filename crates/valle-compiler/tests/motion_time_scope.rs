#![cfg(feature = "motion")]

use valle_compiler::motion::compile_motion;
use valle_motion::{
    EvalInputs, MotionValue, SceneArtifact, StyleValue, TIME_SCOPE_CAPABILITY, eval_all,
    motion_context_at_frame, resolve_props,
};
use valle_timeline::FrameRate;

const TIME_SCOPE: &str = include_str!("fixtures/motion/composition/time-scope.motion.tsx");

fn width(artifact: &SceneArtifact, key: &str, frame: u32) -> f64 {
    let context = motion_context_at_frame(frame, 60, FrameRate::new(30, 1).unwrap()).unwrap();
    let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
    let values = eval_all(
        artifact,
        EvalInputs {
            ctx: &context,
            props: &props,
            unit: None,
            viewport: Some((640.0, 360.0)),
        },
    )
    .unwrap();
    let node = artifact
        .nodes
        .iter()
        .find(|node| node.key.ends_with(key))
        .unwrap();
    let style = node
        .styles
        .iter()
        .find(|style| style.property == "width")
        .unwrap();
    match &style.value {
        StyleValue::Expr { expr } => match values[expr.0 as usize] {
            MotionValue::Number(value) => value,
            ref other => panic!("expected numeric width, got {other:?}"),
        },
        other => panic!("expected frame expression, got {other:?}"),
    }
}

#[test]
fn maps_component_time_and_restores_parent_scope() {
    let artifact = compile_motion(TIME_SCOPE).unwrap().artifact;
    assert!(
        artifact
            .capability_set
            .names
            .iter()
            .any(|name| name == TIME_SCOPE_CAPABILITY)
    );
    assert!((width(&artifact, "bar", 15) - 100.0).abs() < 1e-9);
    assert!((width(&artifact, "bar", 45) - 300.0).abs() < 1e-9);

    let frame_source = TIME_SCOPE.replace("ctx.seconds * 100", "ctx.localFrame * (100 / 30)");
    let frames = compile_motion(&frame_source).unwrap().artifact;
    assert!((width(&frames, "bar", 15) - 100.0).abs() < 1e-9);
    assert!((width(&frames, "bar", 45) - 300.0).abs() < 1e-9);

    let progress_source = TIME_SCOPE.replace("ctx.seconds * 100", "ctx.progress * 200");
    let progress = compile_motion(&progress_source).unwrap().artifact;
    assert!((width(&progress, "bar", 15) - 100.0).abs() < 1e-9);
    assert!((width(&progress, "bar", 45) - 300.0).abs() < 1e-9);

    let nested_source = TIME_SCOPE.replace(
        "<Bar key=\"bar\" />",
        "<TimeScope key=\"inner\" offset={0.25} speed={0.5}><Bar key=\"bar\" /></TimeScope>",
    );
    let nested = compile_motion(&nested_source).unwrap().artifact;
    assert!((width(&nested, "bar", 45) - 187.5).abs() < 1e-9);
    let with_sibling = TIME_SCOPE.replace(
        "</TimeScope>",
        "</TimeScope><View key=\"outside\" className=\"absolute\" style={{width:100+ctx.seconds*100,height:10}}/>",
    );
    let siblings = compile_motion(&with_sibling).unwrap().artifact;
    assert!((width(&siblings, "outside", 45) - 250.0).abs() < 1e-9);
}

#[test]
fn time_scope_rejects_missing_or_invalid_parameters() {
    assert!(compile_motion(&TIME_SCOPE.replace(" speed={2}", "")).is_err());
    assert!(compile_motion(&TIME_SCOPE.replace("offset={0.5}", "offset={ctx.seconds}")).is_err());
    assert!(
        compile_motion(&TIME_SCOPE.replace(
            "<TimeScope key=\"scope\"",
            "<TimeScope key=\"scope\" className=\"relative\""
        ))
        .is_err()
    );
    let mut invalid = compile_motion(TIME_SCOPE).unwrap().artifact;
    let scope = invalid
        .nodes
        .iter_mut()
        .find(|node| node.key == "scope")
        .unwrap();
    scope.kind = valle_motion::NodeKind::TimeScope {
        offset_seconds: f64::NAN,
        speed: 2.0,
    };
    assert!(invalid.validate().is_err());
}

#[test]
fn identity_scope_retains_every_integer_frame_at_non_binary_rates() {
    let source = r#"function Bar(ctx) { return <View key="bar" style={{width:ctx.localFrame,height:10}} />; }
    export default function Main() { return <Scene><TimeScope key="scope" offset={0} speed={1}><Bar /></TimeScope></Scene>; }"#;
    let artifact = compile_motion(source).unwrap().artifact;
    let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
    let expr = artifact
        .nodes
        .iter()
        .flat_map(|node| &node.styles)
        .find_map(|style| {
            if style.property == "width" {
                if let StyleValue::Expr { expr } = style.value {
                    return Some(expr);
                }
            }
            None
        })
        .unwrap();
    for fps in [25, 30, 60] {
        for frame in 0..1000 {
            let ctx =
                motion_context_at_frame(frame, 1000, FrameRate::new(fps, 1).unwrap()).unwrap();
            let values = eval_all(
                &artifact,
                EvalInputs {
                    ctx: &ctx,
                    props: &props,
                    unit: None,
                    viewport: None,
                },
            )
            .unwrap();
            assert_eq!(
                values[expr.0 as usize],
                MotionValue::Number(f64::from(frame)),
                "fps {fps}, frame {frame}"
            );
        }
    }
}
