#![cfg(feature = "motion")]
//! Function and arrow helpers preserve closure scope and expand JSX at call sites.
use valle_compiler::motion::compile_motion;
use valle_motion::StyleValue;

fn diagnostics_of(source: &str) -> Vec<String> {
    compile_motion(source)
        .err()
        .unwrap_or_default()
        .into_iter()
        .map(|diagnostic| diagnostic.message)
        .collect()
}

#[test]
fn arrow_const_helpers_take_context_dependent_arguments_like_declared_ones() {
    let body = |declaration: &str| {
        format!(
            r##"
{declaration}
export default function P(ctx) {{
  const t = ctx.hold.progress;
  return (<Scene className="h-full w-full">
    <View key="a" style={{{{ position: "absolute", left: 0, top: 0, width: 10, height: 10,
                           opacity: fade(t, 0.5) }}}} />
  </Scene>);
}}
"##
        )
    };
    let declared = compile_motion(&body("function fade(t, v) { return t * v; }"))
        .expect("declared helper already worked");
    let arrow = compile_motion(&body("const fade = (t, v) => t * v;"))
        .expect("arrow-const helper must work the same way with context-dependent inputs");
    assert_eq!(
        valle_motion::canonical_bytes(&declared.artifact).expect("canonical"),
        valle_motion::canonical_bytes(&arrow.artifact).expect("canonical"),
        "the two spellings of one helper must produce the same artifact"
    );
}

#[test]
fn helpers_that_return_jsx_expand_at_the_call_site() {
    let compiled = compile_motion(
        r##"
const bar = (i, v, o) => {
  const x = 100 + i * 80;
  return (<View key={`bar-${i}`} style={{ position: "absolute", left: x, top: 200 - v,
                                          width: 60, height: v, opacity: o }} />);
};
export default function P(ctx) {
  const t = ctx.hold.progress;
  return (<Scene className="h-full w-full">{[42, 68].map((v, i) => bar(i, v, t))}</Scene>);
}
"##,
    )
    .expect("JSX-returning helpers compile with context-dependent inputs");
    let keys = compiled
        .artifact
        .nodes
        .iter()
        .map(|node| node.key.as_str())
        .collect::<Vec<_>>();
    assert!(
        keys.contains(&"bar-0") && keys.contains(&"bar-1"),
        "the helper is inlined once per list item, keys come from the returned JSX: {keys:?}"
    );
    // Fold static arguments while retaining ctx-dependent expressions.
    let bar_1 = compiled
        .artifact
        .nodes
        .iter()
        .find(|node| node.key == "bar-1")
        .expect("bar-1 exists");
    let style = |property: &str| {
        bar_1
            .styles
            .iter()
            .find(|binding| binding.property == property)
            .map(|binding| binding.value.clone())
            .unwrap_or_else(|| panic!("{property} is bound"))
    };
    assert!(
        matches!(style("left"), StyleValue::Static { .. }),
        "`100 + i * 80` folds, got {:?}",
        style("left")
    );
    assert!(
        matches!(style("opacity"), StyleValue::Expr { .. }),
        "the ctx-dependent argument stays a frame-time expression"
    );
}

#[test]
fn helpers_declared_inside_a_component_close_over_the_enclosing_scope() {
    let compiled = compile_motion(
        r##"
export default function P(ctx) {
  const t = ctx.hold.progress;
  const span = 200;
  const place = (i) => span * i + t * 10;
  return (<Scene className="h-full w-full">
    {[0, 1].map((i) => (
      <View key={`b-${i}`} style={{ position: "absolute", left: place(i), top: 0, width: 10, height: 10 }} />
    ))}
  </Scene>);
}
"##,
    )
    .expect("component-local helpers compile and capture their scope");
    let left = |key: &str| {
        compiled
            .artifact
            .nodes
            .iter()
            .find(|node| node.key == key)
            .expect("node exists")
            .styles
            .iter()
            .find(|binding| binding.property == "left")
            .expect("left is bound")
            .value
            .clone()
    };
    // Lower both instances independently and retain their frame-time expressions.
    assert!(matches!(left("b-0"), StyleValue::Expr { .. }));
    assert!(matches!(left("b-1"), StyleValue::Expr { .. }));
}

#[test]
fn a_component_local_helper_does_not_leak_into_another_component() {
    let diagnostics = diagnostics_of(
        r##"
function Other(ctx) {
  return (<View key="x" style={{ position: "absolute", left: place(1), top: 0, width: 10, height: 10 }} />);
}
export default function P(ctx) {
  const place = (i) => i * 10;
  return (<Scene className="h-full w-full"><Other key="o" /></Scene>);
}
"##,
    );
    assert!(
        !diagnostics.is_empty(),
        "a helper declared in one component must not be visible in another"
    );
}
