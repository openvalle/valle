#![cfg(feature = "motion")]
//! Video times/multipliers must not inherit normalized path-trim bounds.
use std::collections::BTreeMap;
use valle_compiler::motion::compile_motion;
use valle_motion::{
    EvalInputs, MotionValue, NodeKind, NumberValue, motion_context_at_frame, resolve_props,
};
use valle_timeline::FrameRate;

fn source(start: &str, speed: &str) -> String {
    format!(
        r#"
const START = 6;
export const controls = ({{ assets: {{ clip: asset({{ kind: "video" }}) }} }});
export default function VideoProbe(ctx) {{
    return <Scene><Video src="asset://clip" sourceStart={start} speed={speed} /></Scene>;
}}
"#
    )
}

#[test]
fn video_static_seconds_and_multipliers_are_finite_not_normalized() {
    for (start, speed, expected_start, expected_speed) in [
        ("{6}", "{2}", 6.0, 2.0),
        ("{START}", "{1 + 1}", 6.0, 2.0),
        ("\"9\"", "\"0.5\"", 9.0, 0.5),
        ("{3}", "{0}", 3.0, 0.0),
        ("{-2}", "{-1}", -2.0, -1.0),
    ] {
        let artifact = compile_motion(&source(start, speed)).unwrap().artifact;
        artifact.validate().unwrap();
        let (actual_start, actual_speed) = artifact
            .nodes
            .iter()
            .find_map(|node| match &node.kind {
                NodeKind::Video {
                    source_start,
                    speed,
                    ..
                } => Some((source_start, speed)),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            *actual_start,
            NumberValue::Static {
                value: expected_start
            }
        );
        assert_eq!(
            *actual_speed,
            NumberValue::Static {
                value: expected_speed
            }
        );
    }
}

#[test]
fn equivalent_video_expressions_have_the_same_values_in_any_frame_order() {
    let artifact = compile_motion(&source("{ctx.seconds * 0 + 6}", "{ctx.seconds * 0 + 2}"))
        .unwrap()
        .artifact;
    let props = resolve_props(&artifact.controls, &BTreeMap::new()).unwrap();
    for frame in [60, 0, 30, 60, 1] {
        let ctx = motion_context_at_frame(frame, 90, FrameRate::new(30, 1).unwrap()).unwrap();
        let values = valle_motion::eval_all(
            &artifact,
            EvalInputs {
                ctx: &ctx,
                props: &props,
                unit: None,
                viewport: Some((64.0, 64.0)),
            },
        )
        .unwrap();
        let value = |number: &NumberValue| match number {
            NumberValue::Static { value } => *value,
            NumberValue::Expr { expr } => match values[expr.0 as usize] {
                MotionValue::Number(value) => value,
                ref other => panic!("expected scalar: {other:?}"),
            },
        };
        let (start, speed) = artifact
            .nodes
            .iter()
            .find_map(|node| match &node.kind {
                NodeKind::Video {
                    source_start,
                    speed,
                    ..
                } => Some((value(source_start), value(speed))),
                _ => None,
            })
            .unwrap();
        assert_eq!((start, speed), (6.0, 2.0));
    }
}

#[test]
fn video_nonfinite_constants_fail_and_path_trim_bounds_are_unchanged() {
    for bad in ["\"NaN\"", "\"inf\"", "\"-inf\""] {
        for input in [source(bad, "{1}"), source("{0}", bad)] {
            let errors = compile_motion(&input).unwrap_err();
            assert!(
                errors
                    .iter()
                    .any(|e| e.message.contains("must be a finite number")),
                "{errors:?}"
            );
        }
    }
    let errors = compile_motion(
        r#"export default function P() {
        return <Path d="M0 0 L10 10" trimStart={2} />;
    }"#,
    )
    .unwrap_err();
    assert!(errors.iter().any(|e| {
        e.message
            .contains("trimStart must be a finite number in 0..=1")
    }));
}
