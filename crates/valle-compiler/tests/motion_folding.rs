#![cfg(feature = "motion")]
//! Constant folding must accept static author expressions and produce the same artifact as literal values, without frame-time expression nodes.

use valle_compiler::motion::compile_motion;
use valle_motion::{Expr, StyleValue};

/// Data-driven source with arrays, static helper arguments and computed interpolation outputs.
const COMPUTED: &str = r##"
const VALUES = [42, 68, 91];
const BASE = 120;
const scaled = (v) => BASE - v;

export default function Bars(ctx) {
  const t = ctx.hold.progress;
  return (
    <Scene className="h-full w-full">
      {VALUES.map((v, i) => (
        <View
          key={`bar-${i}`}
          style={{
            position: "absolute",
            left: 100 + i * 80,
            top: scaled(v),
            width: 60,
            height: v,
            opacity: interpolate(t, [0, 1], [0, v / 100]),
          }}
        />
      ))}
    </Scene>
  );
}
"##;

/// Equivalent scene using literal values.
const LITERAL: &str = r##"
export default function Bars(ctx) {
  const t = ctx.hold.progress;
  return (
    <Scene className="h-full w-full">
      <View key="bar-0" style={{ position: "absolute", left: 100, top: 78, width: 60, height: 42, opacity: interpolate(t, [0, 1], [0, 0.42]) }} />
      <View key="bar-1" style={{ position: "absolute", left: 180, top: 52, width: 60, height: 68, opacity: interpolate(t, [0, 1], [0, 0.68]) }} />
      <View key="bar-2" style={{ position: "absolute", left: 260, top: 29, width: 60, height: 91, opacity: interpolate(t, [0, 1], [0, 0.91]) }} />
    </Scene>
  );
}
"##;

#[test]
fn computed_stops_and_styles_produce_the_same_artifact_as_hand_written_literals() {
    let computed = compile_motion(COMPUTED).expect("data-driven form must compile");
    let literal = compile_motion(LITERAL).expect("literal form compiles");

    // Compare artifact bytes without buildFingerprint, whose normalizedAstDigest depends on the source.
    let a = valle_motion::canonical_bytes(&computed.artifact).expect("canonical");
    let b = valle_motion::canonical_bytes(&literal.artifact).expect("canonical");
    assert_eq!(
        String::from_utf8(a).unwrap(),
        String::from_utf8(b).unwrap(),
        "folded constants must produce the same artifact as literals"
    );
}

#[test]
fn folded_values_land_as_static_styles_not_constant_expressions() {
    // Fold styles to Static, avoiding a frame-time Const expression.
    let compiled = compile_motion(COMPUTED).expect("compiles");
    let bar = compiled
        .artifact
        .nodes
        .iter()
        .find(|node| node.key == "bar-1")
        .expect("bar-1 exists");
    let left = bar
        .styles
        .iter()
        .find(|binding| binding.property == "left")
        .expect("left is bound");
    assert!(
        matches!(left.value, StyleValue::Static { .. }),
        "`100 + i * 80` must fold into a static style, got {:?}",
        left.value
    );
}

#[test]
fn context_dependent_values_are_not_folded() {
    // Preserve ctx-dependent expressions at frame time.
    let compiled = compile_motion(
        r##"
export default function Probe(ctx) {
  return (
    <Scene className="h-full w-full">
      <View key="b" style={{ position: "absolute", left: 0, top: 0, width: 10, height: 10,
                             opacity: ctx.hold.progress * 0.5 }} />
    </Scene>
  );
}
"##,
    )
    .expect("compiles");
    let bar = compiled
        .artifact
        .nodes
        .iter()
        .find(|node| node.key == "b")
        .expect("node exists");
    let opacity = bar
        .styles
        .iter()
        .find(|binding| binding.property == "opacity")
        .expect("opacity is bound");
    let StyleValue::Expr { .. } = opacity.value else {
        panic!("a ctx-dependent value must stay a frame-time expression, got {opacity:?}");
    };
    assert!(
        compiled
            .artifact
            .exprs
            .iter()
            .any(|expr| matches!(expr, Expr::Context { .. })),
        "the context leaf must survive folding"
    );
}

#[test]
fn a_stop_that_depends_on_context_still_fails_with_a_pointed_diagnostic() {
    // Interpolation stops must remain static; diagnostics explain that boundary.
    let diagnostics = compile_motion(
        r##"
export default function Probe(ctx) {
  const t = ctx.hold.progress;
  return (
    <Scene className="h-full w-full">
      <View key="b" style={{ position: "absolute", left: 0, top: 0, width: 10, height: 10,
                             opacity: interpolate(t, [0, 1], [0, t * 2]) }} />
    </Scene>
  );
}
"##,
    )
    .expect_err("a ctx-dependent output stop must be rejected");
    assert!(
        diagnostics
            .iter()
            .any(|d| d.message.contains("compile time") && d.message.contains("ctx")),
        "diagnostic should name the boundary, got {diagnostics:#?}"
    );
}

// QuickJS static evaluation.

/// Typed values must reject implicit coercion through every static-evaluation path.
#[test]
fn a_typed_value_cannot_be_coerced_into_a_string_by_concatenation() {
    let diagnostics = compile_motion(
        r##"
export default function P(ctx) {
  return (<Scene className="h-full w-full">
    <Text key="a" style={{ position: "absolute", left: 0, top: 0, fontSize: 40 }}>
      {"score: " + point(1, 2)}
    </Text>
  </Scene>);
}
"##,
    )
    .expect_err("string concatenation of a typed value must fail closed");
    assert!(
        diagnostics.iter().any(
            |diagnostic| diagnostic.message.contains("cannot be coerced")
                && diagnostic.message.contains("point")
        ),
        "the author must be told which typed value it was, not get `[object Object]` \
         baked into the artifact: {diagnostics:?}"
    );
}

/// Valid typed construction and nesting must still fold successfully.
#[test]
fn typed_values_still_fold_when_they_are_not_being_coerced() {
    let compiled = compile_motion(
        r##"
export default function GoodGradient() {
  return <Path
    d="M 0 0 L 20 0 L 20 20 Z"
    fill={linearGradient(point(0, 0), point(20, 0), [
      gradientStop(0.2, "#000"),
      gradientStop(0.8, "#fff"),
    ])}
  />;
}
"##,
    )
    .expect("a well-formed gradient still compiles");
    assert!(
        compiled.artifact.exprs.is_empty(),
        "the whole gradient must still fold at compile time, leaving no frame-time expression"
    );
}

/// Emit each diagnostic once when static text evaluation and lowering both inspect a JSX child.
#[test]
fn one_mistake_is_reported_once() {
    let diagnostics = compile_motion(
        r##"
export default function P(ctx) {
  return (<Scene className="h-full w-full">
    <Text key="a" style={{ position: "absolute", left: 0, top: 0, fontSize: 40 }}>
      {"score: " + point(1, 2)}
    </Text>
  </Scene>);
}
"##,
    )
    .expect_err("fails");
    let coercion = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.message.contains("cannot be coerced"))
        .count();
    assert_eq!(coercion, 1, "one mistake, one line: {diagnostics:?}");
}

/// JSON.stringify bypasses Symbol.toPrimitive; reject leaked typed representations at the output boundary.
#[test]
fn stringified_typed_internals_are_rejected_at_the_fold_boundary() {
    let diagnostics = compile_motion(
        r##"
export default function P(ctx) {
  return (<Scene className="h-full w-full">
    <Text key="a" style={{ position: "absolute", left: 0, top: 0, fontSize: 40 }}>
      {JSON.stringify(point(1, 2)) + "px"}
    </Text>
  </Scene>);
}
"##,
    )
    .expect_err("serialized typed internals must not reach the artifact")
    .into_iter()
    .map(|diagnostic| diagnostic.message)
    .collect::<Vec<_>>();
    assert!(
        diagnostics
            .iter()
            .any(|message| message.contains("internal representation leaked")),
        "{diagnostics:?}"
    );
}

/// Inject bindings with JSON.parse so __proto__ remains an own property and cannot inject inherited values.
#[test]
fn a_proto_key_does_not_become_a_prototype_through_the_binding_round_trip() {
    let diagnostics = compile_motion(
        r##"
export default function P(ctx) {
  const o = { ["__proto__"]: { hidden: 77 } };
  const v = o.hidden;
  return (<Scene className="h-full w-full">
    <View key="a" style={{ position: "absolute", left: 0, top: 0, height: 10, width: v }} />
  </Scene>);
}
"##,
    );
    match diagnostics {
        // Rejecting an undefined binding is valid.
        Err(diagnostics) => {
            assert!(!diagnostics.is_empty());
        }
        // If undefined becomes representable, width must still never resolve to 77.
        Ok(compiled) => {
            let leaked = compiled.artifact.nodes.iter().any(|node| {
                node.styles.iter().any(|entry| {
                    matches!(
                        &entry.value,
                        StyleValue::Static { value: valle_motion::MotionValue::Number(n) } if *n == 77.0
                    )
                })
            });
            assert!(
                !leaked,
                "__proto__ must not smuggle a prototype through rebind"
            );
        }
    }
}
