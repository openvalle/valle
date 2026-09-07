#![cfg(feature = "motion")]
//! Inner bindings must shadow outer bindings across static evaluation and frame-time lowering. Assert artifact expression structure to detect silently frozen dynamic inputs.

use valle_compiler::motion::compile_motion;
use valle_motion::{Expr, StyleValue};

/// Read a node's style value by key.
fn style_of(
    compiled: &valle_compiler::motion::CompiledMotion,
    key: &str,
    property: &str,
) -> StyleValue {
    compiled
        .artifact
        .nodes
        .iter()
        .find(|node| node.key == key)
        .unwrap_or_else(|| panic!("node `{key}` missing"))
        .styles
        .iter()
        .find(|entry| entry.property == property)
        .unwrap_or_else(|| panic!("style `{property}` missing on `{key}`"))
        .value
        .clone()
}

/// Return the arena index of a frame-time style expression.
fn expr_id(value: &StyleValue) -> u32 {
    match value {
        StyleValue::Expr { expr } => expr.0,
        other => panic!("expected a frame-time expression, got {other:?}"),
    }
}

/// Expect phase progress multiplied by a constant.
fn assert_progress_times(compiled: &valle_compiler::motion::CompiledMotion, at: u32, k: f64) {
    let exprs = &compiled.artifact.exprs;
    let Expr::Mul { lhs, rhs } = exprs[at as usize] else {
        panic!("expected Mul at {at}, got {:?}", exprs[at as usize]);
    };
    assert!(
        matches!(exprs[lhs.0 as usize], Expr::Context { .. }),
        "left operand must stay a context leaf, got {:?}",
        exprs[lhs.0 as usize]
    );
    let Expr::Const { value } = &exprs[rhs.0 as usize] else {
        panic!("expected Const, got {:?}", exprs[rhs.0 as usize]);
    };
    assert_eq!(
        *value,
        valle_motion::MotionValue::Number(k),
        "the surviving constant must be the one the author wrote"
    );
}

// Helper parameters shadow outer constants.

/// A frame-time helper argument must shadow a captured static binding with the same name.
#[test]
fn a_helper_parameter_shadows_the_outer_constant_it_is_named_after() {
    let compiled = compile_motion(
        r##"
export default function P(ctx) {
  const gap = 12;
  const f = (gap) => gap * 2;
  return (<Scene className="h-full w-full">
    <View key="a" style={{ position: "absolute", left: 0, top: 0, height: 10,
                           width: f(ctx.hold.progress) }} />
  </Scene>);
}
"##,
    )
    .expect("compiles");
    let width = style_of(&compiled, "a", "width");
    assert_progress_times(&compiled, expr_id(&width), 2.0);
}

// Component bindings shadow module constants.

/// Detect dynamic shadowing before static evaluation can read the module prelude.
#[test]
fn a_component_local_binding_shadows_the_module_constant_it_is_named_after() {
    let compiled = compile_motion(
        r##"
const size = 40;
export default function P(ctx) {
  const size = ctx.hold.progress * 100;
  return (<Scene className="h-full w-full">
    <View key="a" style={{ position: "absolute", left: 0, top: 0, height: 10, width: size }} />
  </Scene>);
}
"##,
    )
    .expect("compiles");
    let width = style_of(&compiled, "a", "width");
    assert_progress_times(&compiled, expr_id(&width), 100.0);
}

/// Unshadowed module constants must continue to fold within the same component.
#[test]
fn an_unshadowed_module_constant_still_folds_in_the_same_component() {
    let compiled = compile_motion(
        r##"
const size = 40;
const other = 7;
export default function P(ctx) {
  const size = ctx.hold.progress * 100;
  return (<Scene className="h-full w-full">
    <View key="a" style={{ position: "absolute", left: 0, top: 0, height: other, width: size }} />
  </Scene>);
}
"##,
    )
    .expect("compiles");
    assert_eq!(
        style_of(&compiled, "a", "height"),
        StyleValue::Static {
            value: valle_motion::MotionValue::Number(7.0)
        },
        "`other` is not shadowed, so it must still fold at compile time"
    );
    let width = style_of(&compiled, "a", "width");
    assert_progress_times(&compiled, expr_id(&width), 100.0);
}

// Static bindings shadow dynamic bindings.

/// Clear conflicting bindings in both directions so inner static constants also shadow outer expressions.
#[test]
fn an_inner_constant_shadows_an_outer_frame_time_binding_of_the_same_name() {
    let compiled = compile_motion(
        r##"
export default function P(ctx) {
  const v = ctx.hold.progress * 100;
  const box = (i) => {
    const v = 33;
    return v + i;
  };
  return (<Scene className="h-full w-full">
    <View key="a" style={{ position: "absolute", left: 0, top: 0, height: 10, width: box(4) }} />
  </Scene>);
}
"##,
    )
    .expect("compiles");
    assert_eq!(
        style_of(&compiled, "a", "width"),
        StyleValue::Static {
            value: valle_motion::MotionValue::Number(37.0)
        },
        "the inner `const v = 33` must win over the outer frame-time `v`"
    );
}

// Local helpers capture the complete enclosing scope.

/// Local helpers capture sibling functions as well as values; helper_stack still rejects recursion.
#[test]
fn a_local_helper_can_call_its_sibling() {
    let compiled = compile_motion(
        r##"
export default function P(ctx) {
  const h1 = (v) => v * 2;
  const h2 = (v) => h1(v) + 1;
  return (<Scene className="h-full w-full">
    <View key="a" style={{ position: "absolute", left: 0, top: 0, height: 10,
                           width: h2(ctx.hold.progress) }} />
  </Scene>);
}
"##,
    )
    .expect("a local helper must see its siblings");
    let width = style_of(&compiled, "a", "width");
    let exprs = &compiled.artifact.exprs;
    let Expr::Add { lhs, rhs } = exprs[expr_id(&width) as usize] else {
        panic!(
            "expected `h1(v) + 1`, got {:?}",
            exprs[expr_id(&width) as usize]
        );
    };
    assert_progress_times(&compiled, lhs.0, 2.0);
    assert_eq!(
        exprs[rhs.0 as usize],
        Expr::Const {
            value: valle_motion::MotionValue::Number(1.0)
        }
    );
}

/// Mutually recursive helpers must fail because inline expansion cannot terminate.
#[test]
fn mutually_recursive_helpers_are_still_rejected() {
    let diagnostics = compile_motion(
        r##"
export default function P(ctx) {
  const f = (v) => g(v) + 1;
  const g = (v) => f(v) * 2;
  return (<Scene className="h-full w-full">
    <View key="a" style={{ position: "absolute", left: 0, top: 0, height: 10,
                           width: f(ctx.hold.progress) }} />
  </Scene>);
}
"##,
    )
    .expect_err("mutual recursion cannot be expanded")
    .into_iter()
    .map(|diagnostic| diagnostic.message)
    .collect::<Vec<_>>();
    assert!(
        diagnostics
            .iter()
            .any(|message| message.contains("recursive")),
        "{diagnostics:?}"
    );
}

// Static map parameters share the binding namespace.

/// Prepared map item/index bindings must replace outer frame-time bindings with the same name.
#[test]
fn a_static_map_parameter_shadows_the_outer_frame_time_binding() {
    let compiled = compile_motion(
        r##"
export default function P(ctx) {
  const v = ctx.hold.progress * 100;
  return (<Scene className="h-full w-full">
    {[1, 2].map((v, i) => <View key={`k-${i}`} style={{ position: "absolute", left: 0, top: 0,
                                                        height: 10, width: v * 2 }} />)}
  </Scene>);
}
"##,
    )
    .expect("compiles");
    for (key, expected) in [("k-0", 2.0), ("k-1", 4.0)] {
        assert_eq!(
            style_of(&compiled, key, "width"),
            StyleValue::Static {
                value: valle_motion::MotionValue::Number(expected)
            },
            "the map item must win over the outer frame-time `v` on `{key}`"
        );
    }
}

/// Restore outer bindings after each map iteration and preserve unrelated dynamic names inside the map.
#[test]
fn the_outer_frame_time_binding_survives_the_map_when_not_shadowed() {
    let compiled = compile_motion(
        r##"
export default function P(ctx) {
  const v = ctx.hold.progress * 100;
  return (<Scene className="h-full w-full">
    {[1, 2].map((k, i) => <View key={`k-${i}`} style={{ position: "absolute", left: 0, top: 0,
                                                        height: 10, width: v + k }} />)}
    <View key="after" style={{ position: "absolute", left: 0, top: 20, height: 10, width: v }} />
  </Scene>);
}
"##,
    )
    .expect("compiles");
    let exprs = &compiled.artifact.exprs;
    // The map body combines an outer expression with the prepared item constant.
    for (key, expected) in [("k-0", 1.0), ("k-1", 2.0)] {
        let width = style_of(&compiled, key, "width");
        let Expr::Add { lhs, rhs } = exprs[expr_id(&width) as usize] else {
            panic!("expected `v + k` on `{key}`");
        };
        assert!(matches!(exprs[lhs.0 as usize], Expr::Mul { .. }));
        assert_eq!(
            exprs[rhs.0 as usize],
            Expr::Const {
                value: valle_motion::MotionValue::Number(expected)
            }
        );
    }
    // After the map, the outer binding still points to the same subtree.
    let after = style_of(&compiled, "after", "width");
    assert!(matches!(exprs[expr_id(&after) as usize], Expr::Mul { .. }));
}

// Nested function parameters introduce their own scope.

/// Static map parameters may reuse a dynamic outer name without preventing constant folding.
#[test]
fn an_arrow_parameter_sharing_a_frame_time_name_does_not_block_static_folding() {
    let compiled = compile_motion(
        r##"
export default function P(ctx) {
  const t = ctx.hold.progress;
  const DOUBLED = [1, 2, 3].map((t) => t * 2);
  return (<Scene className="h-full w-full">
    <View key="a" style={{ position: "absolute", left: 0, top: 0, height: 10,
                           width: DOUBLED[2] }} />
  </Scene>);
}
"##,
    )
    .expect("a shadowing arrow parameter must not poison a fully static map");
    assert_eq!(
        style_of(&compiled, "a", "width"),
        StyleValue::Static {
            value: valle_motion::MotionValue::Number(6.0)
        },
    );
}

/// Shadowing exemptions are lexical: references outside a nested function remain dynamic.
#[test]
fn a_free_use_outside_the_arrow_still_blocks_static_folding() {
    let compiled = compile_motion(
        r##"
export default function P(ctx) {
  const t = ctx.hold.progress * 50;
  return (<Scene className="h-full w-full">
    <View key="a" style={{ position: "absolute", left: 0, top: 0, height: 10,
                           width: t + [1].map((t) => t)[0] }} />
  </Scene>);
}
"##,
    )
    .expect("compiles");
    let width = style_of(&compiled, "a", "width");
    let exprs = &compiled.artifact.exprs;
    assert!(
        matches!(exprs[expr_id(&width) as usize], Expr::Add { .. }),
        "the outer `t` must stay frame-time; folding it would use a stale value"
    );
}
