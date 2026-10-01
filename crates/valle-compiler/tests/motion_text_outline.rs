#![cfg(feature = "motion")]

use valle_compiler::motion::{MeasureEnv, compile_motion, compile_motion_with_env};
use valle_motion::{NodeKind, PathValue, TextMeasure, TextOutlineAlign, text_outline};

const TEXT_OUTLINE: &str =
    include_str!("fixtures/motion/composition/text-outline-drawing.motion.tsx");

fn env() -> MeasureEnv {
    MeasureEnv::new(&[], (640, 360)).unwrap()
}

#[test]
fn bakes_real_valle_glyphs_into_a_static_path() {
    let artifact = compile_motion_with_env(TEXT_OUTLINE, &[], Some(&env()))
        .unwrap()
        .artifact;
    artifact.validate().unwrap();
    let path = artifact
        .nodes
        .iter()
        .find_map(|node| match &node.kind {
            NodeKind::Path {
                d: PathValue::Static { value },
                ..
            } => Some(value),
            _ => None,
        })
        .expect("textOutline bakes a static PathData");
    assert!(path.verbs.len() > 50, "real font curves should be retained");
    assert!(
        path.points
            .iter()
            .all(|point| point.x >= 40.0 && point.x < 640.0)
    );
    assert!(
        path.points
            .iter()
            .all(|point| point.y > 80.0 && point.y < 280.0)
    );
    assert_eq!(
        serde_json::to_vec(&artifact).unwrap(),
        serde_json::to_vec(
            &compile_motion_with_env(TEXT_OUTLINE, &[], Some(&env()))
                .unwrap()
                .artifact
        )
        .unwrap(),
    );
    let first_glyph = TEXT_OUTLINE.replace("d={WORD.path}", "d={WORD.glyphs[0].path}");
    let glyph_artifact = compile_motion_with_env(&first_glyph, &[], Some(&env()))
        .unwrap()
        .artifact;
    let glyph_path = glyph_artifact
        .nodes
        .iter()
        .find_map(|node| match &node.kind {
            NodeKind::Path {
                d: PathValue::Static { value },
                ..
            } => Some(value),
            _ => None,
        })
        .unwrap();
    assert!(glyph_path.verbs.len() < path.verbs.len());
}

#[test]
fn glyph_metadata_and_alignment_use_shaped_font_positions() {
    let mut fonts = valle_motion::Fonts::default();
    valle_motion::register_default_motion_fonts(&mut fonts).unwrap();
    let request = TextMeasure {
        text: "VALLE",
        font_size: 150.0,
        font_weight: Some(800.0),
        ..Default::default()
    };
    let viewport = valle_motion::Viewport::new((640, 360));
    let left = text_outline(
        &request,
        &fonts,
        viewport,
        valle_draw::Point::new(60.0, 240.0),
        TextOutlineAlign::Left,
    )
    .unwrap();
    assert_eq!(left.glyphs.len(), 5);
    assert_eq!(
        left.glyphs.iter().map(|g| g.cluster).collect::<Vec<_>>(),
        vec![0, 1, 2, 3, 4]
    );
    assert!(
        left.glyphs
            .iter()
            .all(|g| g.word == 0 && g.line == 0 && g.advance > 0.0)
    );
    let center = text_outline(
        &request,
        &fonts,
        viewport,
        valle_draw::Point::new(60.0, 240.0),
        TextOutlineAlign::Center,
    )
    .unwrap();
    let right = text_outline(
        &request,
        &fonts,
        viewport,
        valle_draw::Point::new(60.0, 240.0),
        TextOutlineAlign::Right,
    )
    .unwrap();
    assert!(right.bounds.x < center.bounds.x && center.bounds.x < left.bounds.x);
    assert!((right.bounds.y - left.bounds.y).abs() < 0.001);

    let wrapped = text_outline(
        &TextMeasure {
            text: "A B",
            font_size: 100.0,
            max_width: Some(100.0),
            ..Default::default()
        },
        &fonts,
        viewport,
        valle_draw::Point::new(0.0, 100.0),
        TextOutlineAlign::Left,
    )
    .unwrap();
    assert_eq!(
        wrapped
            .glyphs
            .iter()
            .map(|g| (g.word, g.line))
            .collect::<Vec<_>>(),
        vec![(0, 0), (1, 1)]
    );
}

#[test]
fn text_outline_rejects_missing_fonts_dynamic_inputs_and_bad_typography() {
    assert!(
        compile_motion(TEXT_OUTLINE)
            .unwrap_err()
            .iter()
            .any(|d| d.message.contains("--font"))
    );
    let dynamic = TEXT_OUTLINE.replace("fontSize: 150", "fontSize: 150 + ctx.progress");
    assert!(compile_motion_with_env(&dynamic, &[], Some(&env())).is_err());
    for replacement in ["fontSize: -1", "fontSize: 150, align: \"middle\""] {
        let source = TEXT_OUTLINE.replace("fontSize: 150", replacement);
        assert!(
            compile_motion_with_env(&source, &[], Some(&env())).is_err(),
            "{replacement}"
        );
    }
    let bad_origin = TEXT_OUTLINE.replace("point(60, 240)", "point(1 / 0, 0)");
    assert!(compile_motion_with_env(&bad_origin, &[], Some(&env())).is_err());
}
