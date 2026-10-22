#![cfg(feature = "motion")]
//! Spring timing, frame-rate independence, keyframes and prepared staggered scheduling.

use std::collections::BTreeMap;

use valle_compiler::motion::compile_motion;
use valle_motion::{EvalInputs, Expr, MotionValue, motion_context_at_frame, resolve_props};
use valle_timeline::FrameRate;

fn diagnostics_of(source: &str) -> Vec<String> {
    compile_motion(source)
        .err()
        .unwrap_or_default()
        .into_iter()
        .map(|diagnostic| diagnostic.message)
        .collect()
}

const SPRING: &str = r##"
export default function P() { const time = useCurrentTime();
  const enter = spring(time.seconds, {
    damping: 18,
    stiffness: 140,
  });
  return (<Scene className="h-full w-full">
    <View key="a" style={{ position: "absolute", left: 0, top: 0, width: 10, height: 10, opacity: enter }} />
  </Scene>);
}
"##;

/// Evaluate a spring at a given frame, frame rate and clip duration.
fn spring_value(source: &str, fps_num: u32, duration_frames: u32, frame: u32) -> f64 {
    let compiled = compile_motion(source).expect("compiles");
    let artifact = &compiled.artifact;
    let props = resolve_props(&artifact.controls, &BTreeMap::new()).expect("props");
    let fps = FrameRate::new(i64::from(fps_num), 1).unwrap();
    let ctx = motion_context_at_frame(frame, duration_frames, fps).expect("frame in range");
    let values = valle_motion::eval_all(
        artifact,
        EvalInputs {
            ctx: &ctx,
            props: &props,
            unit: None,
            viewport: Some((1920.0, 1080.0)),
        },
    )
    .expect("evaluates");
    let spring_at = artifact
        .exprs
        .iter()
        .position(|expr| matches!(expr, Expr::Spring { .. }))
        .expect("a spring node exists");
    let MotionValue::Number(value) = values[spring_at] else {
        panic!("spring must evaluate to a number");
    };
    value
}

// A spring is a function of time in seconds.

/// A spring of a constant time is one value, whatever the frame and the frame rate: the compiler
/// folds it to the value the spring of the time takes then.
#[test]
fn a_spring_of_a_constant_time_is_the_same_at_every_frame_rate() {
    const CONSTANT_TIME: &str = r##"
export default function P() {
  return (<Scene className="h-full w-full">
    <View key="a" style={{ position: "absolute", left: 0, top: 0, height: 10,
                           width: spring(1 / 3, { preset: "gentle" }) }} />
  </Scene>);
}
"##;
    let artifact = compile_motion(CONSTANT_TIME).expect("compiles").artifact;
    assert!(
        !artifact
            .exprs
            .iter()
            .any(|expr| matches!(expr, Expr::Spring { .. }))
    );
    let width = artifact
        .nodes
        .iter()
        .flat_map(|node| &node.styles)
        .find(|style| style.property == "width")
        .unwrap();
    let folded = match &width.value {
        valle_motion::StyleValue::Static { value } => value.clone(),
        valle_motion::StyleValue::Expr { expr } => match &artifact.exprs[expr.0 as usize] {
            Expr::Const { value } => value.clone(),
            other => panic!("the spring folds to a constant, got {other:?}"),
        },
    };
    let MotionValue::Number(folded) = folded else {
        panic!("a width is a number, got {folded:?}");
    };
    let timed = CONSTANT_TIME
        .replace(
            "export default function P() {",
            "export default function P() { const time = useCurrentTime();",
        )
        .replace("spring(1 / 3,", "spring(time.seconds,");
    for (fps, frame) in [(30, 10), (60, 20)] {
        assert_eq!(
            spring_value(&timed, fps, 120, frame).to_bits(),
            folded.to_bits(),
            "{fps} fps frame {frame}"
        );
    }
    // Still on its way at a third of a second: past its target and not yet settled.
    assert!(folded > 1.0 && folded < 1.2, "{folded}");
}

// Frame-rate independence.

/// Frames n at 30fps and 2n at 60fps must sample the same closed-form spring value bit for bit.
#[test]
fn the_spring_is_frame_rate_invariant_bit_for_bit() {
    for n in [0u32, 1, 3, 7, 15, 29] {
        let slow = spring_value(SPRING, 30, 60, n);
        let fast = spring_value(SPRING, 60, 120, n * 2);
        assert_eq!(
            slow.to_bits(),
            fast.to_bits(),
            "30fps frame {n} = {slow} but 60fps frame {} = {fast}",
            n * 2
        );
    }
}

// Authored windows.

/// Continue spring evolution after an authored window instead of clamping elapsed time.
#[test]
fn the_spring_keeps_converging_after_its_authored_window_closes() {
    let inside = spring_value(SPRING, 30, 60, 5);
    let after = spring_value(SPRING, 30, 60, 10);
    let far = spring_value(SPRING, 30, 60, 40);
    assert!(
        after != inside,
        "the spring froze at the window edge: {inside} then {after}"
    );
    assert!(
        (far - 1.0).abs() < 1e-3,
        "the spring should be at rest far past the window, got {far}"
    );
}

/// Before its start time, the spring holds its initial value.
#[test]
fn the_spring_sits_at_its_start_value_before_its_authored_window_begins() {
    let source = r##"
export default function P() { const time = useCurrentTime();
  const out = spring(time.seconds - 1, { preset: "gentle" });
  return (<Scene className="h-full w-full">
    <View key="a" style={{ position: "absolute", left: 0, top: 0, width: 10, height: 10, opacity: out }} />
  </Scene>);
}
"##;
    for frame in [0u32, 5, 20] {
        assert_eq!(
            spring_value(source, 30, 60, frame),
            0.0,
            "frame {frame} is before the authored window, the spring must not have started"
        );
    }
    assert!(spring_value(source, 30, 60, 35) > 0.0);
}

// Reject mixed presets and explicit parameters with source locations.

#[test]
fn a_preset_and_bare_parameters_together_are_rejected_at_compile_time() {
    let messages = diagnostics_of(
        r##"
export default function P() { const time = useCurrentTime();
  const e = spring(time.seconds, { preset: "gentle", stiffness: 200 });
  return (<Scene className="h-full w-full">
    <View key="a" style={{ position: "absolute", left: 0, top: 0, width: 10, height: 10, opacity: e }} />
  </Scene>);
}
"##,
    );
    assert!(
        messages
            .iter()
            .any(|message| message.contains("mutually exclusive")),
        "got {messages:#?}"
    );
}

#[test]
fn an_unknown_preset_lists_the_ones_that_exist() {
    let messages = diagnostics_of(
        r##"
export default function P() { const time = useCurrentTime();
  const e = spring(time.seconds, { preset: "springy" });
  return (<Scene className="h-full w-full">
    <View key="a" style={{ position: "absolute", left: 0, top: 0, width: 10, height: 10, opacity: e }} />
  </Scene>);
}
"##,
    );
    assert!(
        messages
            .iter()
            .any(|message| message.contains("gentle") && message.contains("wobbly")),
        "got {messages:#?}"
    );
}

// The frame form is gone.

/// The old frame form names the form in seconds.
#[test]
fn the_old_frame_form_names_the_form_in_seconds() {
    for spring in [
        r#"spring({ elapsedFrames: 10, fps: 30, preset: "gentle" })"#,
        r#"spring(seconds, { fps: 30, preset: "gentle" })"#,
    ] {
        let messages = diagnostics_of(&format!(
            r##"
export default function P() {{ const {{ seconds }} = useCurrentTime();
  const e = {spring};
  return (<Scene className="h-full w-full">
    <View key="a" style={{{{ position: "absolute", left: 0, top: 0, width: 10, height: 10, opacity: e }}}} />
  </Scene>);
}}
"##
        ));
        assert!(
            messages.iter().any(|message| {
                message.contains("`elapsedFrames` and `fps` are gone")
                    && message.contains("spring(seconds - 0.4")
            }),
            "{spring}: {messages:#?}"
        );
    }
}

/// Physical parameters must be compile-time constants.
#[test]
fn frame_varying_physical_parameters_are_rejected() {
    let messages = diagnostics_of(
        r##"
export default function P() { const time = useCurrentTime();
  const e = spring(time.seconds, { stiffness: time.progress * 100 });
  return (<Scene className="h-full w-full">
    <View key="a" style={{ position: "absolute", left: 0, top: 0, width: 10, height: 10, opacity: e }} />
  </Scene>);
}
"##,
    );
    assert!(
        messages
            .iter()
            .any(|message| message.contains("do not vary per frame")),
        "got {messages:#?}"
    );
}

// Keyframes and prepared staggered timing.

/// Represent keyframes with multi-stop interpolation.
#[test]
fn keyframes_are_multi_stop_interpolate_with_per_segment_easing() {
    let compiled = compile_motion(
        r##"
export default function P() { const time = useCurrentTime();
  const t = time.progress;
  return (<Scene className="h-full w-full">
    <View key="a" style={{ position: "absolute", left: 0, top: 0, width: 10, height: 10,
                           opacity: interpolate(t, [0, 0.3, 0.7, 1], [0, 1, 1, 0], { easing: ["easeOut", "linear", "easeIn"] }) }} />
  </Scene>);
}
"##,
    )
    .expect("multi-stop interpolate compiles");
    let interpolate = compiled
        .artifact
        .exprs
        .iter()
        .find_map(|expr| match expr {
            Expr::Interpolate { stops, easings, .. } => Some((stops.len(), easings.len())),
            _ => None,
        })
        .expect("an interpolate node exists");
    assert_eq!(
        interpolate,
        (4, 3),
        "four stops and one easing per segment — that is what keyframes means"
    );
}

/// Prepare stagger offsets with compile-time helpers.
#[test]
fn staggered_sequences_are_computed_at_compile_time() {
    let compiled = compile_motion(
        r##"
const CARDS = [0, 1, 2, 3];
const stagger = (i, each, overlap) => i * (each - overlap);
export default function P() { const time = useCurrentTime();
  const t = time.progress;
  return (<Scene className="h-full w-full">
    {CARDS.map((c, i) => (
      <View key={`c-${i}`} style={{ position: "absolute", left: i * 100, top: 0, width: 80, height: 80,
                                    opacity: interpolate(t, [stagger(i, 0.3, 0.1), stagger(i, 0.3, 0.1) + 0.3], [0, 1]) }} />
    ))}
  </Scene>);
}
"##,
    )
    .expect("compile-time stagger compiles");
    // Each card's distinct interpolation start is baked into the artifact.
    let starts: Vec<f64> = compiled
        .artifact
        .exprs
        .iter()
        .filter_map(|expr| match expr {
            Expr::Interpolate { stops, .. } => Some(stops[0].input),
            _ => None,
        })
        .collect();
    assert_eq!(starts.len(), 4, "one interpolate per card, got {starts:?}");
    for pair in starts.windows(2) {
        assert!(
            pair[1] > pair[0],
            "each card must start later than the previous: {starts:?}"
        );
    }
}

#[test]
fn duration_and_bounce_describe_the_spring_as_swiftui_does() {
    let source = |options: &str| SPRING.replace("damping: 18,\n    stiffness: 140,", options);
    let physics = |options: &str| {
        let compiled = compile_motion(&source(options)).expect("compiles");
        compiled
            .artifact
            .exprs
            .iter()
            .find_map(|expr| match expr {
                Expr::Spring {
                    mass,
                    stiffness,
                    damping,
                    ..
                } => Some([*mass, *stiffness, *damping]),
                _ => None,
            })
            .expect("a spring node exists")
    };
    let close = |actual: f64, expected: f64| (actual - expected).abs() < 1e-9;
    // One undamped swing per half second.
    let frequency = std::f64::consts::TAU / 0.5;
    let [mass, stiffness, damping] = physics("duration: 0.5,");
    assert_eq!(mass, 1.0);
    assert!(close(stiffness, frequency * frequency), "{stiffness}");
    assert!(close(damping, 2.0 * frequency), "{damping}");
    let [_, _, damping] = physics("duration: 0.5, bounce: 0.5,");
    assert!(close(damping, frequency), "{damping}");
    // The default duration is half a second.
    let [_, _, damping] = physics("bounce: -0.5,");
    assert!(close(damping, 4.0 * frequency), "{damping}");

    // At 30 fps the half-second duration ends at frame 15.
    let positions = |options: &str| {
        (0..=60)
            .step_by(3)
            .map(|frame| spring_value(&source(options), 30, 120, frame))
            .collect::<Vec<_>>()
    };
    let critical = positions("duration: 0.5,");
    assert!(critical[5] > 0.98, "{critical:?}");
    assert!(
        critical.iter().all(|position| *position <= 1.0),
        "{critical:?}"
    );
    let bouncy = positions("duration: 0.5, bounce: 0.5,");
    assert!(bouncy.iter().any(|position| *position > 1.05), "{bouncy:?}");

    for options in [
        r#"duration: 0.5, preset: "gentle","#,
        "bounce: 0.2, damping: 10,",
    ] {
        let messages = diagnostics_of(&source(options));
        assert!(
            messages
                .iter()
                .any(|message| message.contains("three ways to describe one spring")),
            "{options}: {messages:?}"
        );
    }
    for (options, name) in [
        ("duration: 0,", "`duration`"),
        ("bounce: 1,", "`bounce`"),
        ("bounce: -1,", "`bounce`"),
    ] {
        let messages = diagnostics_of(&source(options));
        assert!(
            messages
                .iter()
                .any(|message| message.contains(name) && message.contains("compile time")),
            "{options}: {messages:?}"
        );
    }
}
