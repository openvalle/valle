#![cfg(feature = "motion")]
//! Spring timing, frame-rate independence, keyframes and prepared staggered scheduling.

use std::collections::BTreeMap;

use valle_compiler::motion::compile_motion;
use valle_motion::{
    EvalInputs, Expr, MotionValue, ResolvedSignals, motion_context_at, phase_windows, resolve_props,
};
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
export default function P(ctx) {
  const enter = spring({
    elapsedFrames: ctx.enter.elapsedFrames,
    fps: ctx.fps,
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
    let layout = phase_windows(&artifact.controls.phase_spec(), duration_frames);
    let ctx = motion_context_at(
        frame,
        &layout,
        FrameRate::new(i64::from(fps_num), 1).unwrap(),
    )
    .expect("frame in range");
    let values = valle_motion::eval_all(
        artifact,
        EvalInputs {
            ctx: &ctx,
            props: &props,
            signals: &ResolvedSignals::default(),
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

// Springs must never fold to constants.

/// Spring reads render-time fps directly, even when elapsedFrames is constant. Preserve its IR node rather than evaluating with a placeholder context.
#[test]
fn a_spring_with_a_constant_time_input_is_never_folded_away() {
    const CONSTANT_TIME: &str = r##"
export default function P(ctx) {
  return (<Scene className="h-full w-full">
    <View key="a" style={{ position: "absolute", left: 0, top: 0, height: 10,
                           width: spring({ elapsedFrames: 10, fps: ctx.fps, preset: "gentle" }) }} />
  </Scene>);
}
"##;
    let compiled = compile_motion(CONSTANT_TIME).expect("compiles");
    assert!(
        compiled
            .artifact
            .exprs
            .iter()
            .any(|expr| matches!(expr, Expr::Spring { .. })),
        "a spring must survive constant folding even when its time input is a literal; \
         folding it bakes the dummy fps=1 answer into the artifact"
    );

    // The same artifact produces different values at different frame rates for a fixed frame index.
    let slow = spring_value(CONSTANT_TIME, 30, 60, 0);
    let fast = spring_value(CONSTANT_TIME, 60, 120, 0);
    assert_ne!(
        slow, fast,
        "a constant elapsedFrames still means different wall-clock times at different fps"
    );
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

// Behavior before and after the phase window.

/// Continue spring evolution after the enter window using elapsedFrames rather than clamped frame.
#[test]
fn the_spring_keeps_converging_after_its_phase_window_closes() {
    let source = SPRING.replace(
        "export default function P(ctx) {",
        "export const controls = defineControls({ timing: { enterFrames: frames({ default: 6, min: 0, max: 120 }) } });\nexport default function P(ctx) {",
    );
    // Samples beyond the enter window must continue changing toward rest.
    let inside = spring_value(&source, 30, 60, 5);
    let after = spring_value(&source, 30, 60, 10);
    let far = spring_value(&source, 30, 60, 40);
    assert!(
        after != inside,
        "the spring froze at the window edge: {inside} then {after}"
    );
    assert!(
        (far - 1.0).abs() < 1e-3,
        "the spring should be at rest far past the window, got {far}"
    );
}

/// Before a phase starts, return the initial value.
#[test]
fn the_spring_sits_at_its_start_value_before_its_phase_begins() {
    // Sampling the exit spring at clip start precedes its phase window.
    let source = r##"
export const controls = defineControls({ timing: { exitFrames: frames({ default: 10, min: 0, max: 120 }) } });
export default function P(ctx) {
  const out = spring({ elapsedFrames: ctx.exit.elapsedFrames, fps: ctx.fps, preset: "gentle" });
  return (<Scene className="h-full w-full">
    <View key="a" style={{ position: "absolute", left: 0, top: 0, width: 10, height: 10, opacity: out }} />
  </Scene>);
}
"##;
    for frame in [0u32, 5, 20] {
        assert_eq!(
            spring_value(source, 30, 60, frame),
            0.0,
            "frame {frame} is before the exit window, the spring must not have started"
        );
    }
}

// Reject mixed presets and explicit parameters with source locations.

#[test]
fn a_preset_and_bare_parameters_together_are_rejected_at_compile_time() {
    let messages = diagnostics_of(
        r##"
export default function P(ctx) {
  const e = spring({ elapsedFrames: ctx.enter.elapsedFrames, fps: ctx.fps, preset: "gentle", stiffness: 200 });
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
export default function P(ctx) {
  const e = spring({ elapsedFrames: ctx.enter.elapsedFrames, fps: ctx.fps, preset: "springy" });
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

// Require ctx.fps as the frame-rate input.

/// The spring frame rate must match the render context to preserve sampling invariance.
#[test]
fn a_hand_written_frame_rate_is_rejected() {
    let messages = diagnostics_of(
        r##"
export default function P(ctx) {
  const e = spring({ elapsedFrames: ctx.enter.elapsedFrames, fps: 60, preset: "gentle" });
  return (<Scene className="h-full w-full">
    <View key="a" style={{ position: "absolute", left: 0, top: 0, width: 10, height: 10, opacity: e }} />
  </Scene>);
}
"##,
    );
    assert!(
        messages
            .iter()
            .any(|message| message.contains("exactly `ctx.fps`")),
        "got {messages:#?}"
    );
}

/// Physical parameters must be compile-time constants.
#[test]
fn frame_varying_physical_parameters_are_rejected() {
    let messages = diagnostics_of(
        r##"
export default function P(ctx) {
  const e = spring({ elapsedFrames: ctx.enter.elapsedFrames, fps: ctx.fps, stiffness: ctx.hold.progress * 100 });
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
export default function P(ctx) {
  const t = ctx.hold.progress;
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
export default function P(ctx) {
  const t = ctx.hold.progress;
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
