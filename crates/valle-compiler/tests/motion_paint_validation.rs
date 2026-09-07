#![cfg(feature = "motion")]
//! Explicit path paint, filter no-ops and merged border diagnostics.
use valle_compiler::motion::compile_motion;
use valle_draw::color::Rgba;
use valle_motion::{ColorValue, NodeKind, PaintValue};

fn diagnostics(source: &str) -> Vec<String> {
    compile_motion(source)
        .expect_err("invalid paint source must fail closed")
        .into_iter()
        .map(|diagnostic| diagnostic.message)
        .collect()
}

fn contains(messages: &[String], needles: &[&str]) -> bool {
    messages
        .iter()
        .any(|message| needles.iter().all(|needle| message.contains(needle)))
}

fn compile_ok(source: &str) -> valle_compiler::motion::CompiledMotion {
    compile_motion(source)
        .unwrap_or_else(|diagnostics| panic!("valid paint source must compile: {diagnostics:#?}"))
}

fn scene(body: &str) -> String {
    format!(
        r##"
export default function Card(ctx) {{
  return (
    <Scene style={{{{ width: 320, height: 180, backgroundColor: "#111111" }}}}>
      {body}
    </Scene>
  );
}}
"##
    )
}

#[test]
fn open_path_line_polyline_require_explicit_fill() {
    assert!(contains(
        &diagnostics(&scene(
            r##"<Path key="p" d={path("M0 0 L20 0")} stroke="#fff" />"##
        )),
        &["explicit fill", "<Path>"]
    ));
    assert!(contains(
        &diagnostics(&scene(
            r##"<Line key="l" x1={0} y1={0} x2={8} y2={8} stroke="#fff" />"##
        )),
        &["explicit fill", "<Line>"]
    ));
    assert!(contains(
        &diagnostics(&scene(
            r##"<Polyline key="pl" points={[point(0, 0), point(4, 4)]} stroke="#fff" />"##
        )),
        &["explicit fill", "<Polyline>"]
    ));
}

#[test]
fn stroke_only_fill_none_and_intentional_black_compile() {
    let none = compile_ok(&scene(
        r##"<Path key="p" d={path("M0 0 L20 0")} stroke="#fff" fill="none" />"##,
    ));
    let NodeKind::Path { fill: None, .. } = &none
        .artifact
        .nodes
        .iter()
        .find(|node| node.key == "p")
        .expect("path")
        .kind
    else {
        panic!("fill=\"none\" must drop the default black paint");
    };
    let ink = compile_ok(&scene(
        r##"<Path key="ink" d={path("M0 0 L20 0 L10 16 Z")} fill="#000" />"##,
    ));
    assert!(matches!(
        &ink.artifact
            .nodes
            .iter()
            .find(|node| node.key == "ink")
            .expect("ink")
            .kind,
        NodeKind::Path {
            fill: Some(PaintValue::Solid {
                color: ColorValue::Static { value },
            }),
            ..
        } if *value == Rgba::rgb(0, 0, 0)
    ));
}

#[test]
fn closed_path_keeps_svg_default_black_without_fill_attr() {
    let compiled = compile_ok(&scene(
        r##"<Path key="box" d={path("M0 0 L20 0 L20 20 Z")} />"##,
    ));
    assert!(matches!(
        &compiled
            .artifact
            .nodes
            .iter()
            .find(|node| node.key == "box")
            .expect("box")
            .kind,
        NodeKind::Path {
            fill: Some(PaintValue::Solid {
                color: ColorValue::Static { value },
            }),
            ..
        } if *value == Rgba::rgb(0, 0, 0)
    ));
}

#[test]
fn filter_none_and_blur_zero_are_noops_negative_blur_is_rejected() {
    let none = compile_ok(&scene(
        r##"<View key="n" style={{ width: 20, height: 20, backgroundColor: "#fff", filter: "none" }} />"##,
    ));
    assert!(
        !none
            .artifact
            .nodes
            .iter()
            .any(|node| { node.styles.iter().any(|style| style.property == "filter") })
    );
    let zero = compile_ok(&scene(
        r##"<View key="z" style={{ width: 20, height: 20, backgroundColor: "#fff", filter: "blur(0px)" }} />"##,
    ));
    assert!(
        !zero
            .artifact
            .nodes
            .iter()
            .any(|node| { node.styles.iter().any(|style| style.property == "filter") })
    );
    let mixed = compile_ok(&scene(
        r##"<View key="m" style={{ width: 20, height: 20, backgroundColor: "#fff", filter: "blur(0px) brightness(1.2)" }} />"##,
    ));
    let filter = mixed
        .artifact
        .nodes
        .iter()
        .flat_map(|node| &node.styles)
        .find(|style| style.property == "filter")
        .expect("kept brightness");
    match &filter.value {
        valle_motion::StyleValue::Static {
            value: valle_motion::MotionValue::Str(text),
        } => assert_eq!(text, "brightness(1.2)"),
        other => panic!("expected kept filter string, got {other:?}"),
    }
    assert!(contains(
        &diagnostics(&scene(
            r##"<View key="bad" style={{ width: 20, height: 20, backgroundColor: "#fff", filter: "blur(-2px)" }} />"##
        )),
        &["blur"]
    ));
}

#[test]
fn border_width_without_style_is_diagnosed_after_class_merge() {
    assert!(contains(
        &diagnostics(&scene(
            r##"<View key="a" style={{ width: 40, height: 20, borderWidth: 2, borderColor: "#fff" }} />"##
        )),
        &["borderStyle"]
    ));
    assert!(contains(
        &diagnostics(&scene(
            r##"<View key="b" className="border-2 border-white" style={{ width: 40, height: 20 }} />"##
        )),
        &["borderStyle"]
    ));
    compile_ok(&scene(
        r##"<View key="ok" className="border-2 border-solid border-white" style={{ width: 40, height: 20 }} />"##,
    ));
    compile_ok(&scene(
        r##"<View key="ok2" style={{ width: 40, height: 20, borderWidth: 2, borderColor: "#fff", borderStyle: "solid" }} />"##,
    ));
}
