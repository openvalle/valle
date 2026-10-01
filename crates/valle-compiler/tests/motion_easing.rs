#![cfg(feature = "motion")]

use valle_compiler::motion::compile_motion;
use valle_motion::{
    EvalInputs, Expr, MotionValue, eval_all, motion_context_at_frame, resolve_props,
};
use valle_timeline::FrameRate;

fn source(easing: &str) -> String {
    format!(
        r#"export default function E(ctx) {{ return <View style={{{{
        left: interpolate(ctx.seconds, [0, 1], [40, 440], {{easing:{easing}}})
    }}}}/>; }}"#
    )
}

fn positions(easing: &str, frames: &[u32]) -> Vec<f64> {
    let compiled = compile_motion(&source(easing)).unwrap();
    let artifact = &compiled.artifact;
    let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
    let index = artifact
        .exprs
        .iter()
        .position(|expr| matches!(expr, Expr::Interpolate { .. }))
        .unwrap();
    frames
        .iter()
        .map(|frame| {
            let ctx = motion_context_at_frame(*frame, 60, FrameRate::new(30, 1).unwrap()).unwrap();
            let values = eval_all(
                artifact,
                EvalInputs {
                    ctx: &ctx,
                    props: &props,
                    unit: None,
                    viewport: None,
                },
            )
            .unwrap();
            let MotionValue::Number(value) = values[index] else {
                panic!("number");
            };
            value
        })
        .collect()
}

#[test]
fn back_overshoots_and_steps_visit_only_the_requested_positions() {
    let frames: Vec<_> = (0..=42).step_by(3).collect();
    let back = positions(r#""easeOutBack""#, &frames);
    assert!(back.iter().any(|x| *x > 445.0));
    assert_eq!(back[0], 40.0);
    assert_eq!(back.last(), Some(&440.0));
    let steps = positions(r#""steps(4)""#, &frames);
    assert!(
        steps
            .iter()
            .all(|x| [40.0, 140.0, 240.0, 340.0, 440.0].contains(x))
    );
    assert_eq!(
        steps
            .iter()
            .map(|x| *x as i64)
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        5
    );
    // Random access evaluates the same expression without cached history.
    assert_eq!(
        positions(r#""easeOutBack""#, &[42, 12, 0, 12]),
        vec![440.0, back[4], 40.0, back[4]]
    );
}

#[test]
fn step_start_jumps_at_zero_and_at_each_segment_start() {
    assert_eq!(
        positions(r#""steps(4, start)""#, &[0, 3, 15, 30]),
        vec![140.0, 140.0, 340.0, 440.0]
    );
    assert_eq!(
        positions(r#""steps(1, end)""#, &[0, 3, 15, 30]),
        vec![40.0, 40.0, 40.0, 440.0]
    );
    let input = source(r#"["steps(4, end)", "steps(4, start)"]"#)
        .replace("[0, 1], [40, 440]", "[0, 0.5, 1], [40, 240, 440]");
    let artifact = compile_motion(&input).unwrap().artifact;
    let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
    let ctx = motion_context_at_frame(15, 60, FrameRate::new(30, 1).unwrap()).unwrap();
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
    let index = artifact
        .exprs
        .iter()
        .position(|expr| matches!(expr, Expr::Interpolate { .. }))
        .unwrap();
    assert_eq!(values[index], MotionValue::Number(290.0));
}

#[test]
fn presets_accept_immutable_names_and_round_trip_artifacts() {
    for name in [
        "easeInBack",
        "easeOutBack",
        "easeInOutBack",
        "elastic",
        "bounce",
        "steps(4)",
        "steps(4, start)",
    ] {
        let direct = compile_motion(&source(&format!("{name:?}")))
            .unwrap()
            .artifact;
        let named = compile_motion(&format!("const CURVE = {name:?}; {}", source("CURVE")))
            .unwrap()
            .artifact;
        assert_eq!(direct.exprs, named.exprs);
        let decoded: valle_motion::SceneArtifact =
            serde_json::from_slice(&serde_json::to_vec(&direct).unwrap()).unwrap();
        decoded.validate().unwrap();
        assert_eq!(decoded.exprs, direct.exprs);
    }
}

#[test]
fn invalid_step_counts_positions_and_beziers_report_the_authored_value() {
    for name in [
        "steps(0)",
        "steps(-1)",
        "steps(1.5)",
        "steps(1e2)",
        "steps(4294967296)",
        "steps(4, middle)",
        "steps(4,)",
        "steps(4, end, start)",
        "steps()",
        "cubic-bezier(2,0,1,1)",
    ] {
        let diagnostics = compile_motion(&source(&format!("{name:?}"))).unwrap_err();
        assert!(
            diagnostics
                .iter()
                .any(|d| d.message.contains(name) && d.message.contains("positive integer")),
            "{diagnostics:?}"
        );
    }
}
