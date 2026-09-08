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
        .unwrap_or_else(|diagnostics| panic!("emoji source must compile: {diagnostics:#?}"))
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
            viewport: Viewport::new((960, 540)),
            fonts: &fonts,
            styles: None,
        },
    )
    .expect("layout");
    emit(&tree, &default_font_naming).expect("emit")
}

#[test]
fn default_emoji_pack_emits_mixed_text_sequences_and_shadows() {
    for (text, extra) in [
        ("中文 Hello 😀 🚀 🔥 🎉 ❤️", ""),
        ("👍🏻 👍🏽 👍🏿 👩‍💻 👨‍👩‍👧‍👦", ""),
        ("🇨🇳 🇺🇸 1️⃣ 🏳️‍🌈", ""),
        ("🇨🇳 🇺🇸 1️⃣ 🏳️‍🌈", "fontFamily: \"Noto Color Emoji\","),
        (
            "中文 😀 1️⃣ 🇨🇳",
            "textShadow: \"4px 5px 3px #0008\", opacity: 0.7,",
        ),
    ] {
        let source = format!(
            r##"export default function Demo() {{ return <Scene style={{{{width:960,height:540}}}}><Text style={{{{fontSize:64,{extra}}}}}>{text}</Text></Scene>; }}"##
        );
        let report = emit_source(&source);
        assert!(
            report.unsupported.is_empty(),
            "{text}: {:?}",
            report.unsupported
        );
        assert!(
            !report.program.paths().is_empty(),
            "emoji must emit painted outlines: {text}"
        );
    }
}
