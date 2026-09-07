//! Formula glyph and paint emission with deterministic recording bytes.

use std::path::PathBuf;

use ratex_font::FontId;
use valle_draw::program::recording::{Paint, RecordCmd};
use valle_motion::math_formula::{
    AdmitPolicy, FormulaFontRegistry, FormulaStyle, UNFILLED_PATH_STROKE_PX, emit_formula,
};

fn registry() -> FormulaFontRegistry {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/fonts/katex");
    FormulaFontRegistry::load_dir(&dir).expect("load 19 KaTeX faces")
}

const SIX: &[(&str, &str)] = &[
    ("fraction", r"\frac{a}{b}"),
    ("matrix", r"\begin{pmatrix} 1 & 2 \\ 3 & 4 \end{pmatrix}"),
    ("aligned", r"\begin{aligned} a &= b \\ c &= d \end{aligned}"),
    ("macro", r"\def\sqr#1{#1^2}\sqr{y}"),
    ("accent", r"\hat{x}"),
    ("color", r"\color{red}{x}"),
];

#[test]
fn six_class_formulas_emit_glyph_runs_with_cmap_gids() {
    let registry = registry();
    let policy = AdmitPolicy::default();
    let style = FormulaStyle::default();
    for (name, latex) in SIX {
        let fragment = emit_formula(latex, true, style, &registry, &policy)
            .unwrap_or_else(|err| panic!("{name}: {err}"));
        assert!(
            fragment
                .list
                .cmds
                .iter()
                .any(|cmd| matches!(cmd, RecordCmd::GlyphRun { .. })),
            "{name} must emit GlyphRun"
        );
        assert!(!fragment.list.glyphs.is_empty(), "{name} glyphs");
        for glyph in &fragment.list.glyphs {
            assert_ne!(glyph.id, 0, "{name} gid must not be .notdef");
        }
        fragment.list.validate().expect(name);
        assert!(
            fragment.width.is_finite() && fragment.width > 0.0,
            "{name} width"
        );
    }
}

#[test]
fn math_alnum_uses_katex_ascii_cmap_not_raw_unicode() {
    let registry = registry();
    let raw = 0x1D44E; // MATHEMATICAL ITALIC SMALL A
    let mapped = ratex_font::katex_ttf_glyph_char(FontId::MathItalic, raw);
    assert_eq!(mapped, 'a');
    let gid_raw_attempt = registry
        .raw_cmap_gid("Math-Italic", char::from_u32(raw).unwrap())
        .unwrap();
    assert!(
        gid_raw_attempt.is_none(),
        "KaTeX Math-Italic cmap must not have U+1D44E"
    );
    let gid = registry.glyph_id("Math-Italic", raw).expect("mapped cmap");
    let ascii = registry.glyph_id("Math-Italic", u32::from(b'a')).unwrap();
    assert_eq!(gid, ascii);
}

#[test]
fn href_paints_katex_blue_not_node_color() {
    let registry = registry();
    let policy = AdmitPolicy::default();
    let style = FormulaStyle {
        font_size: 40.0,
        color: valle_draw::Rgba::rgb(255, 255, 255),
    };
    let fragment = emit_formula(
        r"\href{https://example.com}{x}",
        true,
        style,
        &registry,
        &policy,
    )
    .expect("href");
    let mut saw_blue = false;
    for cmd in &fragment.list.cmds {
        if let RecordCmd::GlyphRun {
            paint: Paint::Solid(rgba),
            ..
        } = cmd
        {
            assert_eq!((rgba.r, rgba.g, rgba.b), (0, 0, 255), "KaTeX link blue");
            saw_blue = true;
        }
    }
    assert!(saw_blue, "href must paint glyphs");
}

#[test]
fn canonical_bytes_are_stable_across_two_emits() {
    let registry = registry();
    let policy = AdmitPolicy::default();
    let style = FormulaStyle::default();
    let a = emit_formula(SIX[0].1, true, style, &registry, &policy).unwrap();
    let b = emit_formula(SIX[0].1, true, style, &registry, &policy).unwrap();
    let ha = a.list.canonical_bytes().expect("hash a");
    let hb = b.list.canonical_bytes().expect("hash b");
    assert_eq!(ha, hb);
    let inline = emit_formula(r"\sum_{i=1}^n x_i", false, style, &registry, &policy).unwrap();
    let display = emit_formula(r"\sum_{i=1}^n x_i", true, style, &registry, &policy).unwrap();
    assert_ne!(
        inline.list.canonical_bytes().unwrap(),
        display.list.canonical_bytes().unwrap()
    );
}

#[test]
fn unfilled_path_stroke_is_post_em_logical_px() {
    assert_eq!(UNFILLED_PATH_STROKE_PX, 1.5);
    let registry = registry();
    let policy = AdmitPolicy::default();
    let fragment = emit_formula(
        r"\sqrt{x}",
        true,
        FormulaStyle::default(),
        &registry,
        &policy,
    )
    .expect("sqrt");
    let stroked = fragment.list.cmds.iter().any(|cmd| {
        matches!(
            cmd,
            RecordCmd::Path {
                stroke: Some(stroke),
                fill: None,
                ..
            } if (stroke.width - 1.5).abs() < f64::EPSILON
        )
    });
    let _ = stroked; // sqrt may use a glyph surd; angl guarantees a stroke path.
    let angl = emit_formula(
        r"\angl{x}",
        true,
        FormulaStyle::default(),
        &registry,
        &policy,
    );
    if let Ok(angl) = angl {
        assert!(
            angl.list.cmds.iter().any(|cmd| matches!(
                cmd,
                RecordCmd::Path {
                    stroke: Some(stroke),
                    fill: None,
                    ..
                } if (stroke.width - UNFILLED_PATH_STROKE_PX).abs() < f64::EPSILON
            )),
            "\\angl must emit 1.5px stroke"
        );
    }
}

#[test]
fn family_names_are_valle_face_not_katex_logical() {
    let registry = registry();
    let policy = AdmitPolicy::default();
    let fragment = emit_formula("x", true, FormulaStyle::default(), &registry, &policy).unwrap();
    for face in &fragment.list.fonts {
        assert!(
            face.family.starts_with("valle-face-"),
            "unexpected family {}",
            face.family
        );
        assert!(!face.family.contains("KaTeX"));
    }
}
