#![cfg(feature = "motion")]
//! Supported math, operand types and static topology constraints.
use valle_compiler::motion::compile_motion;
use valle_motion::Expr;

fn diagnostics_of(source: &str) -> Vec<String> {
    compile_motion(source)
        .err()
        .unwrap_or_default()
        .into_iter()
        .map(|diagnostic| diagnostic.message)
        .collect()
}

#[test]
fn exactly_specified_math_members_work_at_frame_time_without_new_ir() {
    let compiled = compile_motion(
        r##"
export default function P(ctx) {
  const t = ctx.hold.progress;
  return (<Scene className="h-full w-full">
    <View key="a" style={{ position: "absolute", left: 0, top: 0, width: 10, height: 10,
                           opacity: clamp(Math.abs(t - 0.5) * Math.min(2, 3) + Math.max(0, t), 0, 1) }} />
  </Scene>);
}
"##,
    )
    .expect("min/max/abs/clamp compose at frame time with context-dependent inputs");
    // Compose expressions exclusively from existing IR variants.
    for expr in &compiled.artifact.exprs {
        assert!(
            matches!(
                expr,
                Expr::Const { .. }
                    | Expr::Context { .. }
                    | Expr::Add { .. }
                    | Expr::Sub { .. }
                    | Expr::Mul { .. }
                    | Expr::Neg { .. }
                    | Expr::Compare { .. }
                    | Expr::Select { .. }
            ),
            "min/max/abs/clamp must be composed from existing primitives, found {expr:?}"
        );
    }
    compiled.artifact.validate().expect("artifact validates");
}

#[test]
fn approximated_math_members_stay_rejected() {
    let messages = diagnostics_of(
        r##"
export default function P(ctx) {
  const t = ctx.hold.progress;
  return (<Scene className="h-full w-full">
    <View key="a" style={{ position: "absolute", left: 0, top: 0, width: 10, height: 10,
                           opacity: Math.log(t) }} />
  </Scene>);
}
"##,
    );
    assert!(
        messages
            .iter()
            .any(|message| message.contains("implementation-approximated")),
        "got {messages:#?}"
    );
}

#[test]
fn comparing_a_length_against_a_number_names_the_call_the_author_wrote() {
    let messages = diagnostics_of(
        r##"
export default function P(ctx) {
  const t = ctx.hold.progress;
  return (<Scene className="h-full w-full">
    <View key="a" style={{ position: "absolute", left: Math.min(t, "10px"), top: 0, width: 10, height: 10 }} />
  </Scene>);
}
"##,
    );
    assert!(
        messages
            .iter()
            .any(|message| message.contains("min") && message.contains("plain number")),
        "got {messages:#?}"
    );
}

#[test]
fn compile_time_exponentiation_is_rejected_like_math_pow() {
    let messages = diagnostics_of(
        r##"
export default function P(ctx) {
  return (<Scene className="h-full w-full">
    <View key="a" style={{ position: "absolute", left: 2 ** 5, top: 0, width: 10, height: 10 }} />
  </Scene>);
}
"##,
    );
    assert!(
        messages
            .iter()
            .any(|message| message.contains("**") && message.contains("Math.pow")),
        "got {messages:#?}"
    );
}

#[test]
fn a_literal_double_star_in_authored_text_is_not_mistaken_for_the_operator() {
    compile_motion(
        r##"
export default function P(ctx) {
  return (<Scene className="h-full w-full">
    <Text key="t" style={{ position: "absolute", left: 0, top: 0, fontSize: 20, color: "#fff" }}>a ** b</Text>
  </Scene>);
}
"##,
    )
    .expect("`**` inside authored text is just two characters");
}

#[test]
fn branching_on_a_context_value_points_at_the_two_shapes_that_keep_one_topology() {
    let messages = diagnostics_of(
        r##"
export default function P(ctx) {
  const t = ctx.hold.progress;
  if (t > 0.5) { return (<Scene className="h-full w-full" />); }
  return (<Scene className="h-full w-full" />);
}
"##,
    );
    assert!(
        messages
            .iter()
            .any(|message| message.contains("cond ? a : b") && message.contains("visibility")),
        "the diagnostic must name both fixes, got {messages:#?}"
    );
}

#[test]
fn a_loop_points_at_the_prepare_time_map_that_replaces_it() {
    let messages = diagnostics_of(
        r##"
export default function P(ctx) {
  for (let i = 0; i < 3; i++) {}
  return (<Scene className="h-full w-full" />);
}
"##,
    );
    assert!(
        messages
            .iter()
            .any(|message| message.contains("`.map`") || message.contains(".map")),
        "got {messages:#?}"
    );
}

#[test]
fn reversed_clamp_bounds_are_rejected_on_every_path() {
    // Constant arguments inside a component.
    let diagnostics = diagnostics_of(
        r##"
export default function P(ctx) {
  return (<Scene className="h-full w-full">
    <View key="a" style={{ position: "absolute", left: 0, top: 0, height: 10,
                           width: clamp(5, 10, 3) }} />
  </Scene>);
}
"##,
    );
    assert!(
        diagnostics
            .iter()
            .any(|message| message.contains("low <= high")),
        "all-constant clamp must be rejected too: {diagnostics:?}"
    );

    // Constant arguments at module scope.
    let diagnostics = diagnostics_of(
        r##"
const x = clamp(5, 10, 3);
export default function P(ctx) {
  return (<Scene className="h-full w-full">
    <View key="a" style={{ position: "absolute", left: 0, top: 0, height: 10, width: x }} />
  </Scene>);
}
"##,
    );
    assert!(
        diagnostics
            .iter()
            .any(|message| message.contains("low <= high")),
        "module-scope clamp must be rejected too: {diagnostics:?}"
    );

    // Frame-time value with static bounds.
    let diagnostics = diagnostics_of(
        r##"
export default function P(ctx) {
  return (<Scene className="h-full w-full">
    <View key="a" style={{ position: "absolute", left: 0, top: 0, height: 10,
                           width: clamp(ctx.hold.progress, 10, 3) }} />
  </Scene>);
}
"##,
    );
    assert!(
        diagnostics
            .iter()
            .any(|message| message.contains("low <= high")),
        "{diagnostics:?}"
    );
}

#[test]
fn well_formed_clamp_still_works_on_both_paths() {
    let folded = compile_motion(
        r##"
const x = clamp(5, 0, 3);
export default function P(ctx) {
  return (<Scene className="h-full w-full">
    <View key="a" style={{ position: "absolute", left: 0, top: 0, height: 10, width: x,
                           opacity: clamp(ctx.hold.progress, 0, 1) }} />
  </Scene>);
}
"##,
    )
    .expect("well-formed clamp compiles on both paths");
    let width = folded.artifact.nodes.iter().find_map(|node| {
        node.styles
            .iter()
            .find(|entry| entry.property == "width")
            .map(|entry| entry.value.clone())
    });
    assert_eq!(
        width,
        Some(valle_motion::StyleValue::Static {
            value: valle_motion::MotionValue::Number(3.0)
        }),
        "clamp(5, 0, 3) folds to 3 at compile time"
    );
}
