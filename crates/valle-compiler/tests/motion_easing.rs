#![cfg(feature = "motion")]

use valle_compiler::motion::compile_motion;
use valle_motion::{
    Easing, EvalInputs, Expr, MotionValue, eval_all, motion_context_at_frame, resolve_props,
};
use valle_timeline::FrameRate;

fn source(easing: &str) -> String {
    with_options(&format!("{{easing:{easing}}}"))
}

fn with_options(options: &str) -> String {
    format!(
        r#"export default function E() {{ const time = useCurrentTime(); return <View style={{{{
        left: interpolate(time.seconds, [0, 1], [40, 440], {options})
    }}}}/>; }}"#
    )
}

fn positions(easing: &str, frames: &[u32]) -> Vec<f64> {
    positions_of(&source(easing), frames)
}

fn positions_of(source: &str, frames: &[u32]) -> Vec<f64> {
    let compiled = compile_motion(source).unwrap();
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
fn steps_of_the_time_change_on_their_frame() {
    let at_frame_9 = |input: &str, easing: &str| {
        let source = with_options(&format!("{{easing:{easing:?}}}")).replace(
            "time.seconds, [0, 1], [40, 440]",
            &format!("{input}, [0.1, 0.4], [0, 3]"),
        );
        assert!(source.contains("[0.1, 0.4]"), "{source}");
        positions_of(&source, &[9])[0]
    };
    // Frame 9 at 30 fps is 0.3 s, two thirds of the way from 0.1 to 0.4, where the second step
    // begins; `(0.3 - 0.1) / (0.4 - 0.1) * 3` is 1.9999999999999996.
    assert_eq!(at_frame_9("time.seconds", "steps(3)"), 2.0);
    // `0.3 - 0.2` is 0.09999999999999998, just before the stop at 0.1, where the first step of
    // `start` shows.
    assert_eq!(at_frame_9("time.seconds - 0.2", "steps(3, start)"), 1.0);
}

#[test]
fn css_and_easings_net_spellings_name_the_shared_curves() {
    // easeOutCubic at half time: 1 - 0.5³ of the way.
    assert_eq!(positions(r#""easeOutCubic""#, &[15]), vec![390.0]);
    for (css, camel) in [
        ("ease-in", "easeIn"),
        ("ease-out", "easeOut"),
        ("ease-in-out", "easeInOut"),
        ("step-start", "steps(1, start)"),
        ("step-end", "steps(1)"),
        ("steps(4, jump-end)", "steps(4, end)"),
    ] {
        let artifact = |name: &str| {
            compile_motion(&source(&format!("{name:?}")))
                .unwrap()
                .artifact
        };
        assert_eq!(artifact(css).exprs, artifact(camel).exprs, "{css}");
    }
    // jump-none holds both ends for a step each; jump-both jumps at both ends.
    assert_eq!(
        positions(r#""steps(5, jump-none)""#, &[0, 3, 9, 15, 21, 27, 30]),
        vec![40.0, 40.0, 140.0, 240.0, 340.0, 440.0, 440.0]
    );
    assert_eq!(
        positions(r#""steps(3, jump-both)""#, &[0, 5, 15, 25, 30]),
        vec![140.0, 140.0, 240.0, 340.0, 440.0]
    );
}

#[test]
fn presets_accept_immutable_names_and_round_trip_artifacts() {
    for name in [
        "easeInBack",
        "easeOutBack",
        "easeInOutBack",
        "easeOutElastic",
        "easeOutBounce",
        "easeInOutSine",
        "easeOutExpo",
        "ease-in-out",
        "step-start",
        "steps(4)",
        "steps(4, start)",
        "steps(4, jump-both)",
        "cubic-bezier(0.34, 1.56, 0.64, 1)",
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
        "steps(1, jump-none)",
        "cubic-bezier(2,0,1,1)",
        "exp",
        "elastic",
        "bounce",
        "easeOutcubic",
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

#[test]
fn extrapolation_clamps_by_default_or_extends_the_end_segments() {
    // At 30 fps frame 45 is 1.5 s, half a segment past the last stop.
    let clamp = with_options(r#"{extrapolateRight: "clamp"}"#);
    assert_eq!(positions_of(&clamp, &[0, 45]), vec![40.0, 440.0]);
    // Past the end the segment goes on as a straight line; its easing applies only inside it.
    let extend = with_options(r#"{extrapolateRight: "extend", easing: "easeOutCubic"}"#);
    assert_eq!(
        positions_of(&extend, &[15, 30, 45]),
        vec![390.0, 440.0, 640.0]
    );
    let left = with_options(r#"{extrapolateLeft: "extend"}"#)
        .replace("[0, 1], [40, 440]", "[0.5, 1], [40, 440]");
    assert_eq!(positions_of(&left, &[0, 15]), vec![-360.0, 40.0]);
    for value in ["identity", "wrap"] {
        let options = format!(r#"{{extrapolateRight: "{value}"}}"#);
        let diagnostics = compile_motion(&with_options(&options)).unwrap_err();
        assert!(
            diagnostics
                .iter()
                .any(|d| d.message.contains(&format!("`{value}`"))
                    && d.message.contains(r#""extend""#)),
            "{diagnostics:?}"
        );
    }
}

#[test]
fn remotion_easing_objects_name_the_string_that_draws_the_same_curve() {
    for (remotion, spelling) in [
        ("Easing.out(Easing.cubic)", "easeOutCubic"),
        ("Easing.inOut(Easing.quad)", "easeInOutQuad"),
        ("Easing.in(Easing.poly(4))", "easeInQuart"),
        ("Easing.ease", "easeIn"),
        ("Easing.out(Easing.back())", "easeOutBack"),
        ("Easing.bounce", "easeOutBounce"),
        (
            "Easing.bezier(0.33, 1, 0.68, 1)",
            "cubic-bezier(0.33, 1, 0.68, 1)",
        ),
        (
            "Easing.out(Easing.bezier(0.5, 0, 0.75, 0))",
            "cubic-bezier(0.25, 1, 0.5, 1)",
        ),
    ] {
        let diagnostics = compile_motion(&source(remotion)).unwrap_err();
        assert!(
            diagnostics.iter().any(|d| d
                .message
                .contains(&format!("write this curve as \"{spelling}\""))),
            "{remotion}: {diagnostics:?}"
        );
        assert_eq!(
            Easing::parse(spelling)
                .map(|easing| easing.to_string())
                .as_deref(),
            Some(spelling)
        );
    }
    for remotion in ["Easing.elastic(2)", "Easing.inOut(Easing.back(2))"] {
        let diagnostics = compile_motion(&source(remotion)).unwrap_err();
        assert!(
            diagnostics
                .iter()
                .any(|d| d.message.contains("no `Easing` object")
                    && d.message.contains("such as easeOutCubic")),
            "{remotion}: {diagnostics:?}"
        );
    }
}
