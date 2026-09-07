#![cfg(feature = "motion")]
//! Post-layout references require static, existing targets. Their values may affect paint only, preventing cycles back into layout.

#![cfg(feature = "motion")]

use valle_compiler::motion::compile_motion;
use valle_motion::Expr;

fn scene(body: &str) -> String {
    format!(
        "export default function Probe(ctx) {{\n  return (\n    <Scene key=\"scene\">\n\
         \x20     <View key=\"a\" style={{{{ width: 100, height: 40 }}}} />\n\
         \x20     <View key=\"b\" style={{{{ width: 100, height: 40 }}}} />\n{body}    </Scene>\n  );\n}}\n"
    )
}

fn reject(body: &str, needle: &str) {
    let diagnostics = compile_motion(&scene(body)).expect_err(body);
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains(needle)),
        "{body}: no diagnostic contains {needle:?}:\n{}",
        diagnostics
            .iter()
            .map(|diagnostic| diagnostic.message.clone())
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// Artifact admission errors use the same source-aware diagnostic channel as lowering errors.
fn reject_artifact(body: &str, needle: &str) {
    reject(body, needle);
}

const OVERLAY: &str = "style={{ position: \"absolute\", left: 0, top: 0 }}";

#[test]
fn anchor_and_connect_compose_from_one_new_ir_node() {
    let body = format!(
        "      <Path key=\"link\" {OVERLAY} fill=\"none\" stroke=\"#fff\" strokeWidth=\"2\"\n\
         \x20       d={{connect(anchor(\"a\", \"right\"), anchor(\"b\", \"left\"))}} />\n"
    );
    let compiled = compile_motion(&scene(&body)).expect("post-layout fixture compiles");
    let exprs = &compiled.artifact.exprs;

    // One bounds reference for each endpoint.
    let bounds = exprs
        .iter()
        .filter_map(|expr| match expr {
            Expr::NodeBounds { key } => Some(key.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        bounds,
        ["a", "b"],
        "anchor must read exactly its own target"
    );

    // anchor composes GeometryField, arithmetic and MakePoint.
    assert!(
        exprs
            .iter()
            .any(|expr| matches!(expr, Expr::GeometryField { .. }))
            && exprs.iter().any(|expr| matches!(expr, Expr::Add { .. }))
            && exprs
                .iter()
                .any(|expr| matches!(expr, Expr::MakePoint { .. })),
        "anchor must lower to existing primitives, not a bespoke node"
    );
    // The default straight route uses PathLine.
    assert!(
        exprs
            .iter()
            .any(|expr| matches!(expr, Expr::PathLine { .. })),
        "default connect route must be a plain line"
    );
    compiled.artifact.validate().expect("artifact validates");
}

#[test]
fn cubic_route_uses_the_existing_path_primitive() {
    let body = format!(
        "      <Path key=\"link\" {OVERLAY} fill=\"none\" stroke=\"#fff\" strokeWidth=\"2\"\n\
         \x20       d={{connect(anchor(\"a\", \"right\"), anchor(\"b\", \"left\"), {{ route: \"cubic\" }})}} />\n"
    );
    let compiled = compile_motion(&scene(&body)).expect("cubic route compiles");
    assert!(
        compiled
            .artifact
            .exprs
            .iter()
            .any(|expr| matches!(expr, Expr::PathCubic { .. }))
    );
    compiled.artifact.validate().expect("artifact validates");
}

#[test]
fn post_layout_values_may_not_flow_back_into_layout() {
    // Width affects layout and cannot depend on post-layout geometry.
    reject_artifact(
        "      <View key=\"c\" style={{ width: bounds(\"a\").width, height: 10 }} />\n",
        "may only feed paint",
    );
    // Text content also affects layout size.
    reject_artifact(
        "      <Text key=\"c\">{`${bounds(\"a\").width}`}</Text>\n",
        "may only feed paint",
    );
}

#[test]
fn post_layout_values_may_freely_feed_paint() {
    // Paint-only properties may use post-layout geometry.
    let compiled = compile_motion(&scene(
        "      <View key=\"c\" style={{ width: 10, height: 10, opacity: bounds(\"a\").width }} />\n",
    ))
    .expect("opacity is paint-only");
    compiled
        .artifact
        .validate()
        .expect("paint-only consumption must be allowed");
}

#[test]
fn bounds_targets_must_exist_and_be_static() {
    reject_artifact(
        "      <View key=\"c\" style={{ width: 10, height: 10, opacity: bounds(\"ghost\").width }} />\n",
        "is not a node in this scene",
    );
    reject(
        "      <View key=\"c\" style={{ width: 10, height: 10, opacity: bounds(ctx.enter.progress > 0 ? \"a\" : \"b\").width }} />\n",
        "statically known node key",
    );
}

#[test]
fn unknown_sides_and_routes_are_rejected() {
    reject(
        "      <View key=\"c\" style={{ width: 10, height: 10, opacity: anchor(\"a\", \"northwest\").x }} />\n",
        "unknown anchor side",
    );
    let body = format!(
        "      <Path key=\"link\" {OVERLAY} fill=\"none\" stroke=\"#fff\" strokeWidth=\"2\"\n\
         \x20       d={{connect(anchor(\"a\", \"right\"), anchor(\"b\", \"left\"), {{ route: \"spiral\" }})}} />\n"
    );
    reject(&body, "unknown connect route");
}

#[test]
fn every_anchor_side_lowers() {
    for side in [
        "left",
        "right",
        "top",
        "bottom",
        "center",
        "top-left",
        "top-right",
        "bottom-left",
        "bottom-right",
    ] {
        let body = format!(
            "      <View key=\"c\" style={{{{ width: 10, height: 10, opacity: anchor(\"a\", \"{side}\").x }}}} />\n"
        );
        let compiled = compile_motion(&scene(&body)).unwrap_or_else(|diagnostics| {
            panic!("anchor side `{side}` must lower: {diagnostics:#?}")
        });
        compiled
            .artifact
            .validate()
            .unwrap_or_else(|errors| panic!("anchor side `{side}`: {errors:#?}"));
    }
}

fn lowering_diagnostics(source: &str) -> Vec<String> {
    compile_motion(source)
        .expect_err("invalid post-layout input must fail closed")
        .into_iter()
        .map(|diagnostic| diagnostic.message)
        .collect()
}

#[test]
fn post_layout_bounds_cannot_be_reused_as_a_flip_feedback_loop() {
    let diagnostics = lowering_diagnostics(
        r#"export default function P(ctx){return <View key="a" style={{left:bounds("a").x * ctx.hold.progress}}/>;}"#,
    );
    assert!(
        diagnostics
            .iter()
            .any(|message| message.contains("post-layout"))
    );
}
