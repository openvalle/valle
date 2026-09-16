#![cfg(feature = "motion")]
//! Frame-addressed motion: physical time, velocity, FLIP overshoot, and random access.

use std::collections::BTreeMap;
use valle_compiler::motion::compile_motion;
use valle_motion::{EvalInputs, Expr, MotionValue, ResolvedSignals, SceneArtifact, StyleValue};
use valle_timeline::FrameRate;

fn values(artifact: &SceneArtifact, frame: u32, fps: FrameRate) -> Vec<MotionValue> {
    let props = valle_motion::resolve_props(&artifact.controls, &BTreeMap::new()).unwrap();
    let windows = valle_motion::phase_windows(&artifact.controls.phase_spec(), 600);
    let ctx = valle_motion::motion_context_at(frame, &windows, fps).unwrap();
    valle_motion::eval_all(
        artifact,
        EvalInputs {
            ctx: &ctx,
            props: &props,
            signals: &ResolvedSignals::default(),
            unit: None,
            viewport: Some((960.0, 640.0)),
        },
    )
    .unwrap()
}

fn style(artifact: &SceneArtifact, values: &[MotionValue], property: &str) -> MotionValue {
    let binding = artifact
        .nodes
        .iter()
        .flat_map(|n| &n.styles)
        .find(|s| s.property == property)
        .unwrap();
    match &binding.value {
        StyleValue::Expr { expr } => values[expr.0 as usize].clone(),
        StyleValue::Static { value } => value.clone(),
    }
}

const SPRING: &str = r#"
export default function Spring(ctx) {
  const p = spring({elapsedFrames: ctx.localFrame - 2, fps: ctx.fps, preset: "gentle", initialVelocity: -2});
  const v = springVelocity({elapsedFrames: ctx.localFrame - 2, fps: ctx.fps, preset: "gentle", initialVelocity: -2});
  return <View style={{width: 100, height: 20, translate: point(p * 100, 0),
    motionBlur: motionBlur(point(v * 100 * ctx.fps.den / ctx.fps.num, 0))}} />;
}
"#;

#[test]
fn seeking_repeating_reversing_and_parallel_sampling_produce_identical_values() {
    let artifact = compile_motion(SPRING).unwrap().artifact;
    // Serialization and preparation cannot bake a particular render frame rate into the spring.
    let bytes = serde_json::to_vec(&artifact).unwrap();
    let restored: SceneArtifact = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(artifact, restored);
    for rate in [
        FrameRate::new(24, 1).unwrap(),
        FrameRate::new(30, 1).unwrap(),
        FrameRate::new(60, 1).unwrap(),
        FrameRate::new(30000, 1001).unwrap(),
    ] {
        let sequential: Vec<_> = (0..180).map(|f| values(&artifact, f, rate)).collect();
        for frame in (0..180).rev().chain([90, 2, 179, 0, 90, 1, 2, 15]) {
            assert_eq!(values(&restored, frame, rate), sequential[frame as usize]);
        }
        std::thread::scope(|scope| {
            let workers: Vec<_> = [0, 13, 47, 179]
                .into_iter()
                .map(|frame| {
                    let artifact = &artifact;
                    scope.spawn(move || (frame, values(artifact, frame, rate)))
                })
                .collect();
            for worker in workers {
                let (frame, values) = worker.join().unwrap();
                assert_eq!(values, sequential[frame as usize]);
            }
        });
    }
}

#[test]
fn velocity_uses_seconds_and_blur_explicitly_converts_to_pixels_per_frame() {
    let artifact = compile_motion(&SPRING.replace("ctx.localFrame - 2", "ctx.localFrame"))
        .unwrap()
        .artifact;
    let fps30 = FrameRate::new(30, 1).unwrap();
    let fps60 = FrameRate::new(60, 1).unwrap();
    for frame in [0, 1, 5, 12, 25, 90] {
        let a = values(&artifact, frame, fps30);
        let b = values(&artifact, frame * 2, fps60);
        for (at, expr) in artifact.exprs.iter().enumerate() {
            if matches!(expr, Expr::Spring { .. }) {
                assert_eq!(a[at], b[at]);
            }
        }
        let MotionValue::Point(v30) = style(&artifact, &a, "motion-velocity-blur-velocity") else {
            panic!()
        };
        let MotionValue::Point(v60) = style(&artifact, &b, "motion-velocity-blur-velocity") else {
            panic!()
        };
        assert_eq!(v30.x, v60.x * 2.0);
    }
}

#[test]
fn constant_time_velocity_is_not_folded_and_initial_conditions_are_validated() {
    let source = SPRING.replace("ctx.localFrame - 2", "10");
    let artifact = compile_motion(&source).unwrap().artifact;
    assert_eq!(
        artifact
            .exprs
            .iter()
            .filter(|e| matches!(e, Expr::Spring { .. }))
            .count(),
        2
    );
    for invalid in ["ctx.progress", "1 / 0", "\"fast\""] {
        let diagnostics = compile_motion(&SPRING.replace(
            "initialVelocity: -2",
            &format!("initialVelocity: {invalid}"),
        ))
        .unwrap_err();
        assert!(
            diagnostics
                .iter()
                .any(|d| d.message.contains("initialVelocity")),
            "{diagnostics:?}"
        );
    }
    assert!(
        compile_motion(&SPRING.replace(
            "initialVelocity: -2",
            "initialVelocity: -2, initialVelocity: 1"
        ))
        .is_err()
    );
    let mut invalid = artifact;
    for expr in &mut invalid.exprs {
        if let Expr::Spring {
            initial_velocity, ..
        } = expr
        {
            *initial_velocity = f64::NAN;
        }
    }
    assert!(invalid.validate().is_err());
}

fn flip(progress: &str) -> SceneArtifact {
    compile_motion(&format!(r#"
const layouts = defineLayoutStates({{
  a: {{card: rect(0, 20, 100, 80)}}, b: {{card: rect(200, 40, 200, 40)}}
}});
export default function Flip(ctx) {{
  return <View layoutId="card" style={{{{layoutTransition: flip(layouts, "a", "b", {progress})}}}} />;
}}
"#)).unwrap().artifact
}

#[test]
fn flip_preserves_overshoot_but_does_not_reflect_a_collapsed_axis() {
    let rate = FrameRate::new(60, 1).unwrap();
    for (progress, x, y, sx, sy) in [
        ("0", -200.0, -20.0, 0.5, 2.0),
        ("1", 0.0, 0.0, 1.0, 1.0),
        ("1.2", 40.0, 4.0, 1.1, 0.8),
        ("clamp(1.2, 0, 1)", 0.0, 0.0, 1.0, 1.0),
        ("-0.2", -240.0, -24.0, 0.4, 2.2),
        ("3", 400.0, 40.0, 2.0, 0.0),
        ("-2", -600.0, -60.0, 0.0, 4.0),
    ] {
        let artifact = flip(progress);
        let v = values(&artifact, 0, rate);
        let MotionValue::Length2(translate) = style(&artifact, &v, "translate") else {
            panic!()
        };
        let MotionValue::Point(scale) = style(&artifact, &v, "scale") else {
            panic!()
        };
        for (actual, expected) in [
            (translate.x.value, x),
            (translate.y.value, y),
            (scale.x, sx),
            (scale.y, sy),
        ] {
            assert!(
                (actual - expected).abs() < 1e-12,
                "p={progress}: {actual} != {expected}"
            );
        }
    }
    let artifact = flip("spring({elapsedFrames: ctx.localFrame, fps: ctx.fps, preset: 'wobbly'})");
    let frames: Vec<_> = (0..120).map(|f| values(&artifact, f, rate)).collect();
    assert!(frames.iter().any(|v| matches!(style(&artifact, v, "translate"), MotionValue::Length2(t) if t.x.value > 0.0)),
        "spring overshoot must reach the actual FLIP transform");
    for f in (0..120).rev() {
        assert_eq!(values(&artifact, f, rate), frames[f as usize]);
    }
}
