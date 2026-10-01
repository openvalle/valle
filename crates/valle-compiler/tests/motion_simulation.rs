#![cfg(feature = "motion")]

use std::collections::BTreeMap;

use valle_compiler::motion::compile_motion;
use valle_motion::{
    EvalInputs, Expr, MotionValue, SceneArtifact, StyleValue, eval_all, motion_context_at_frame,
    resolve_props,
};
use valle_timeline::FrameRate;

const SPRING_SIMULATION: &str =
    include_str!("fixtures/motion/composition/spring-simulation.motion.tsx");

fn left(artifact: &SceneArtifact, frame: u32) -> f64 {
    let ctx = motion_context_at_frame(frame, 60, FrameRate::new(30, 1).unwrap()).unwrap();
    let props = resolve_props(&artifact.controls, &BTreeMap::new()).unwrap();
    let values = eval_all(
        artifact,
        EvalInputs {
            ctx: &ctx,
            props: &props,
            unit: None,
            viewport: Some((640.0, 360.0)),
        },
    )
    .unwrap();
    let dot = artifact
        .nodes
        .iter()
        .find(|node| node.key.ends_with("dot"))
        .unwrap();
    let left = dot
        .styles
        .iter()
        .find(|style| style.property == "left")
        .unwrap();
    let value = match &left.value {
        StyleValue::Expr { expr } => &values[expr.0 as usize],
        StyleValue::Static { value } => value,
    };
    let MotionValue::Number(value) = value else {
        panic!("simulation field must be numeric");
    };
    *value
}

#[test]
fn bakes_structured_spring_and_seeks_any_frame() {
    let artifact = compile_motion(SPRING_SIMULATION).unwrap().artifact;
    let samples = artifact
        .exprs
        .iter()
        .find_map(|expr| match expr {
            Expr::SimulationSample { samples, .. } => Some(samples),
            _ => None,
        })
        .expect("simulate.at generates a baked lookup");
    assert_eq!(samples.len(), 481);
    assert!((left(&artifact, 0) - 100.0).abs() < 1e-9);
    assert!((0..60).any(|frame| left(&artifact, frame) > 305.0));
    assert!((left(&artifact, 55) - 300.0).abs() <= 8.0);
    // Independent closed-form solution of x'' + 4x' + 40(x - 300) = 0.
    // The fixed-step semi-implicit Euler trajectory stays close to this reference.
    for frame in 0..60 {
        let t = frame as f64 / 30.0;
        let reference =
            300.0 + (-2.0 * t).exp() * (-200.0 * (6.0 * t).cos() - (200.0 / 3.0) * (6.0 * t).sin());
        assert!((left(&artifact, frame) - reference).abs() < 2.5);
    }
    let first = left(&artifact, 50);
    let earlier = left(&artifact, 10);
    assert_eq!(first.to_bits(), left(&artifact, 50).to_bits());
    assert_eq!(earlier.to_bits(), left(&artifact, 10).to_bits());
    assert_eq!(
        serde_json::to_vec(&artifact).unwrap(),
        serde_json::to_vec(&compile_motion(SPRING_SIMULATION).unwrap().artifact).unwrap()
    );
}

#[test]
fn simulation_exposes_each_state_field_and_interpolates_fractional_time() {
    let velocity = compile_motion(
        &SPRING_SIMULATION.replace("SPRING.at(ctx.seconds).x", "SPRING.at(ctx.seconds).v"),
    )
    .unwrap()
    .artifact;
    assert_eq!(left(&velocity, 0), 0.0);
    assert!(left(&velocity, 5) > 0.0);
    let half_speed =
        compile_motion(&SPRING_SIMULATION.replace("ctx.seconds).x", "ctx.seconds * 0.73).x"))
            .unwrap()
            .artifact;
    // Frame 15 at 0.73 speed lands between two fixed simulation steps.
    let x = left(&half_speed, 15);
    let full = compile_motion(SPRING_SIMULATION).unwrap().artifact;
    let Expr::SimulationSample { samples, dt, .. } = full
        .exprs
        .iter()
        .find(|expr| matches!(expr, Expr::SimulationSample { .. }))
        .unwrap()
    else {
        unreachable!()
    };
    let sample = 0.365 / dt;
    let index = sample.floor() as usize;
    let expected = samples[index] + (samples[index + 1] - samples[index]) * (sample - index as f64);
    assert!((x - expected).abs() < 1e-9);

    let static_at = compile_motion(
        &SPRING_SIMULATION.replace("SPRING.at(ctx.seconds).x", "SPRING.at(0.365).x"),
    )
    .unwrap()
    .artifact;
    assert!((left(&static_at, 0) - expected).abs() < 1e-9);
    let clamped_before =
        compile_motion(&SPRING_SIMULATION.replace("SPRING.at(ctx.seconds).x", "SPRING.at(-1).x"))
            .unwrap()
            .artifact;
    assert_eq!(left(&clamped_before, 0), 100.0);
    let clamped_after =
        compile_motion(&SPRING_SIMULATION.replace("SPRING.at(ctx.seconds).x", "SPRING.at(10).x"))
            .unwrap()
            .artifact;
    assert_eq!(left(&clamped_after, 0), *samples.last().unwrap());
}

#[test]
fn simulation_uses_scoped_time_without_mutating_the_baked_table() {
    let original = compile_motion(SPRING_SIMULATION).unwrap().artifact;
    let scoped = SPRING_SIMULATION
        .replace(
            "<View key=\"dot\" className=\"absolute\" style={{",
            "<TimeScope key=\"scope\" offset={0.5} speed={2}><View key=\"dot\" className=\"absolute\" style={{",
        )
        .replace("      }} />\n    </Scene>", "      }} /></TimeScope>\n    </Scene>");
    let scoped = compile_motion(&scoped).unwrap().artifact;
    assert_eq!(left(&scoped, 15).to_bits(), left(&original, 0).to_bits());
    assert_eq!(left(&scoped, 30).to_bits(), left(&original, 30).to_bits());
}

#[test]
fn simulation_rejects_invalid_input_and_tampered_table() {
    for source in [
        SPRING_SIMULATION.replace("dt: 1 / 240", "dt: 0"),
        SPRING_SIMULATION.replace("duration: 2,", "duration: 200,"),
        SPRING_SIMULATION.replace(
            "return { x: state.x + v * dt, v };",
            "return { x: state.x + v * dt };",
        ),
        SPRING_SIMULATION.replace(
            "return { x: state.x + v * dt, v };",
            "return { x: 0 / 0, v };",
        ),
        SPRING_SIMULATION.replace("SPRING.at(ctx.seconds).x", "SPRING.at(ctx.seconds).missing"),
    ] {
        assert!(compile_motion(&source).is_err());
    }
    let mut artifact = compile_motion(SPRING_SIMULATION).unwrap().artifact;
    let Expr::SimulationSample { samples, .. } = artifact
        .exprs
        .iter_mut()
        .find(|expr| matches!(expr, Expr::SimulationSample { .. }))
        .unwrap()
    else {
        unreachable!()
    };
    samples.pop();
    assert!(artifact.validate().is_err());
}

#[test]
fn repeated_reads_share_one_frozen_sample_table() {
    let source = SPRING_SIMULATION.replace("<View key=\"dot\"", "<View key=\"dot\"");
    let start = source.find("<View key=\"dot\"").unwrap();
    let end = source[start..].find("/>").unwrap() + start + 2;
    let node = &source[start..end];
    let many = (0..64)
        .map(|i| node.replace("key=\"dot\"", &format!("key=\"dot{i}\"")))
        .collect::<String>();
    let source = format!("{}{}{}", &source[..start], many, &source[end..]);
    let artifact = compile_motion(&source).unwrap().artifact;
    assert_eq!(
        artifact
            .exprs
            .iter()
            .filter(|expr| matches!(expr, Expr::SimulationSample { .. }))
            .count(),
        1
    );
}
