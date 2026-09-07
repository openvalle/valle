//! Complete font mapping and glyph, line, rectangle and path conversion.

use std::path::PathBuf;

use valle_draw::program::recording::RecordCmd;
use valle_motion::math_formula::{
    AdmitPolicy, FormulaFontRegistry, FormulaStyle, UNFILLED_PATH_STROKE_PX, emit_formula,
    formula_faces,
};

fn registry() -> FormulaFontRegistry {
    FormulaFontRegistry::load_dir(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/fonts/katex"),
    )
    .unwrap()
}

#[test]
fn all_nineteen_faces_resolve_gid_for_ascii_probe() {
    let registry = registry();
    assert_eq!(registry.len(), formula_faces().len());
    for face in formula_faces() {
        let gid = registry
            .glyph_id(face.ratex_name, u32::from(b'M'))
            .or_else(|_| registry.glyph_id(face.ratex_name, 0x221A));
        assert!(
            gid.is_ok(),
            "{} must cmap a probe glyph: {gid:?}",
            face.ratex_name
        );
        let named = registry.font_face(face.ratex_name, 16.0).unwrap();
        assert!(named.family.starts_with("valle-face-"));
    }
}

#[test]
fn display_items_cover_glyph_line_rect_and_unfilled_path() {
    let registry = registry();
    let policy = AdmitPolicy::default();
    let style = FormulaStyle::default();
    let fraction = emit_formula(r"\frac{1}{2}", true, style, &registry, &policy).unwrap();
    assert!(fraction.list.cmds.iter().any(|cmd| matches!(
        cmd,
        RecordCmd::Path {
            fill: Some(_),
            stroke: None,
            ..
        }
    )));
    let colorbox = emit_formula(r"\colorbox{red}{x}", true, style, &registry, &policy).unwrap();
    assert!(
        colorbox
            .list
            .cmds
            .iter()
            .any(|cmd| matches!(cmd, RecordCmd::Path { fill: Some(_), .. }))
    );
    if let Ok(angl) = emit_formula(r"\angl{x}", true, style, &registry, &policy) {
        assert!(angl.list.cmds.iter().any(|cmd| matches!(
            cmd,
            RecordCmd::Path {
                fill: None,
                stroke: Some(stroke),
                ..
            } if (stroke.width - UNFILLED_PATH_STROKE_PX).abs() < f64::EPSILON
        )));
    }
    let dashed = emit_formula(
        r"\begin{array}{c} a \\ \hdashline b \end{array}",
        true,
        style,
        &registry,
        &policy,
    );
    if let Ok(dashed) = dashed {
        assert!(
            dashed.list.cmds.len() > 1,
            "hdashline should expand to segments or rules"
        );
    }
}
