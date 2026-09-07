#![cfg(feature = "motion")]
//! Line-height units, line boxes and glyph emission under height constraints.
use std::collections::BTreeMap;

use valle_compiler::motion::compile_motion;
use valle_motion::{
    EmitReport, Fonts, LayoutOptions, Viewport, build_tree, default_font_naming, emit,
    prepare_scene,
};
use valle_motion::{ResolvedSignals, motion_context_at, phase_windows, resolve_props};
use valle_timeline::FrameRate;

fn emit_source(source: &str) -> EmitReport {
    let artifact = compile_motion(source)
        .unwrap_or_else(|diagnostics| panic!("line-height source must compile: {diagnostics:#?}"))
        .artifact;
    let prepared = prepare_scene(&artifact).expect("prepare");
    let props = resolve_props(&artifact.controls, &BTreeMap::new()).expect("props");
    let windows = phase_windows(&artifact.controls.phase_spec(), 30);
    let ctx = motion_context_at(0, &windows, FrameRate::new(30, 1).unwrap()).expect("ctx");
    let mut fonts = Fonts::default();
    valle_motion::register_default_motion_fonts(&mut fonts).expect("font");
    let tree = build_tree(
        &prepared,
        &ctx,
        &props,
        &ResolvedSignals::default(),
        &LayoutOptions {
            viewport: Viewport::new((320, 180)),
            fonts: &fonts,
            styles: None,
        },
    )
    .expect("layout");
    emit(&tree, &default_font_naming).expect("emit")
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

fn glyph_count(body: &str) -> usize {
    emit_source(&scene(body))
        .program
        .nodes()
        .iter()
        .map(|node| match node {
            valle_draw::program::Node::GlyphRun(run) => run.glyphs.len(),
            _ => 0,
        })
        .sum()
}

#[test]
fn unitless_line_height_is_a_font_size_multiplier() {
    assert!(
        glyph_count(
            r##"<Text key="t" style={{ fontSize: 24, lineHeight: 1.2, color: "#ffffff" }}>Hello</Text>"##
        ) >= 5
    );
}

#[test]
fn px_line_height_with_sufficient_height_emits_glyphs() {
    assert!(
        glyph_count(
            r##"<Text key="t" style={{ fontSize: 26, height: 32, lineHeight: "32px", color: "#ffffff" }}>Hello</Text>"##
        ) >= 5
    );
    assert!(
        glyph_count(
            r##"<Text key="t" style={{ fontSize: 16, height: 80, lineHeight: "80px", color: "#ffffff" }}>Hello</Text>"##
        ) >= 5
    );
}

#[test]
fn glitch_cycle_cell_shape_emits_with_px_line_height() {
    assert!(
        glyph_count(
            r##"<View key="cell" className="relative" style={{ width: 20, height: 32 }}><Text key="t" className="absolute" style={{ left: 0, right: 0, textAlign: "center", fontSize: 26, lineHeight: "32px", color: "#ffffff" }}>I</Text></View>"##
        ) >= 1
    );
}

#[test]
fn brand_frame_label_emits_with_px_line_height() {
    assert!(
        glyph_count(
            r##"<Text key="edge-label" style={{ left: 70, lineHeight: "22px", fontSize: 22, fontWeight: 800, letterSpacing: 3, color: "#ffffff" }}>DESIGN</Text>"##
        ) >= 6
    );
}

#[test]
fn overflow_smaller_than_line_box_still_reaches_emit() {
    let report = emit_source(&scene(
        r##"<Text key="t" style={{ fontSize: 24, height: 12, overflow: "hidden", lineHeight: "32px", color: "#ffffff" }}>Hello</Text>"##,
    ));
    assert!(
        report
            .unsupported
            .iter()
            .all(|(_, what)| !what.contains("produced no glyphs")),
        "legal overflow clip must not take the silent empty-glyph path: {:?}",
        report.unsupported
    );
}

#[test]
fn direction_marks_preserve_authored_text_source_mapping() {
    for direction in ["ltr", "rtl"] {
        let report = emit_source(&scene(&format!(
            r##"<Text key="parent" style={{{{ direction: "{direction}", fontSize: 24, color: "#ffffff" }}}}>Hello <Span style={{{{ color: "#ffcc00" }}}}>world</Span>!</Text>"##,
        )));
        assert!(
            report.unsupported.is_empty(),
            "{direction}: {:?}",
            report.unsupported
        );
        let runs: Vec<_> = report
            .program
            .nodes()
            .iter()
            .filter_map(|node| match node {
                valle_draw::program::Node::GlyphRun(run) if !run.glyphs.is_empty() => Some(run),
                _ => None,
            })
            .collect();
        assert!(!runs.is_empty());
        for run in runs {
            assert!(
                run.source_node.is_some(),
                "{direction}: missing source node"
            );
            assert_eq!(run.source_ranges.len(), run.glyphs.len());
        }
    }
}

#[test]
fn explicit_newline_after_inline_image_starts_a_new_line() {
    let source = scene(
        r##"<Text key="code" className="line-clamp-12 overflow-hidden" style={{ whiteSpace: "pre-wrap", fontSize: 24, lineHeight: "32px", height: 128, textOverflow: "ellipsis" }}>{"A "}<Image key="dot" src="asset://dot" style={{ width: 16, height: 16 }} />{"\nB"}</Text>"##,
    );
    let report = emit_source(&format!(
        r#"export const controls = defineControls({{ assets: {{ dot: asset({{ kind: "image" }}) }} }}); {source}"#
    ));
    assert!(report.unsupported.is_empty(), "{:?}", report.unsupported);
    let glyphs: Vec<_> = report
        .program
        .nodes()
        .iter()
        .filter_map(|node| match node {
            valle_draw::program::Node::GlyphRun(run) => Some(run.glyphs.iter()),
            _ => None,
        })
        .flatten()
        .collect();
    assert!(glyphs.len() >= 2);
    assert!(
        glyphs.last().unwrap().y - glyphs[0].y >= 31.0,
        "newline must move B below A: {glyphs:?}"
    );
}

#[test]
fn multiline_code_preserves_newline_after_styled_spans_and_image() {
    let source = scene(
        r##"
<Text key="code" className="absolute whitespace-pre-wrap tabular-nums line-clamp-12 overflow-hidden text-white" style={{ left: 34, top: 88, width: 970, height: 430, fontSize: 26, lineHeight: "38px", tabSize: 2, textOverflow: "ellipsis" }}>
{"export default function Film(ctx) {\n"}
<Span style={{ color: "#67e8f9" }}>{"  const p = ctx.hold.progress;\n"}</Span>
{"  return (\n    <Scene className=\"h-full w-full\">\n"}
<Span style={{ color: "#f9a8d4" }}>{"      <GeometryBatch progress={p} /> "}</Span>
<Image key="status" src="asset://dot" style={{ width: 22, height: 22, verticalAlign: "middle" }} />
{"\n      <Text>Every frame is pure.</Text>\n    </Scene>\n  );\n}"}
</Text>"##,
    );
    let report = emit_source(&format!(
        r#"export const controls = defineControls({{ assets: {{ dot: asset({{ kind: "image" }}) }} }}); {source}"#
    ));
    assert!(report.unsupported.is_empty(), "{:?}", report.unsupported);
    let glyphs: Vec<_> = report
        .program
        .nodes()
        .iter()
        .filter_map(|node| match node {
            valle_draw::program::Node::GlyphRun(run) => Some(run.glyphs.iter()),
            _ => None,
        })
        .flatten()
        .collect();
    let first = glyphs.first().expect("first code glyph").y;
    let last = glyphs.last().expect("last code glyph").y;
    assert!(
        last - first >= 303.0,
        "nine code lines expected: first={first}, last={last}"
    );
}

#[test]
fn explicit_newlines_after_spaces_override_soft_break_rules() {
    for white_space in ["pre", "pre-wrap", "pre-line"] {
        for (text, lines) in [("A \nB", 2), ("A \n\nB", 3)] {
            let text = serde_json::to_string(text).unwrap();
            let report = emit_source(&scene(&format!(
                r##"<Text key="t" style={{{{ whiteSpace: "{white_space}", fontSize: 24, lineHeight: "32px" }}}}>{{{text}}}</Text>"##,
            )));
            assert!(report.unsupported.is_empty(), "{:?}", report.unsupported);
            let glyphs: Vec<_> = report
                .program
                .nodes()
                .iter()
                .filter_map(|node| match node {
                    valle_draw::program::Node::GlyphRun(run) => Some(run.glyphs.iter()),
                    _ => None,
                })
                .flatten()
                .collect();
            let first = glyphs.first().expect("A").y;
            let last = glyphs.last().expect("B").y;
            assert!(
                (last - first - f64::from((lines - 1) * 32)).abs() < 1.0,
                "{white_space}, {text}: first={first}, last={last}"
            );
        }
    }
}
