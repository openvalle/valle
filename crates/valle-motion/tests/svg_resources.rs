use valle_motion::svg::{Op, compile, dimensions};

#[test]
fn intrinsic_size_and_vector_geometry_survive_compilation() {
    let source=br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 10"><defs><linearGradient id="g"><stop stop-color="red"/><stop offset="1" stop-color="blue"/></linearGradient></defs><path d="M0 0L20 0L0 10Z" fill="url(#g)"/></svg>"##;
    assert_eq!(dimensions(source).unwrap(), (20, 10));
    let doc = compile(source, &[]).unwrap();
    assert!(doc.ops.iter().any(|op| matches!(op, Op::Draw { .. })));
    assert!(!doc.ops.iter().any(|op| matches!(op, Op::Raster { .. })));
}

#[test]
fn text_uses_frozen_fonts_and_external_or_animated_content_is_rejected() {
    let text=br#"<svg xmlns="http://www.w3.org/2000/svg" width="60" height="30"><text x="0" y="24" font-family="Noto Sans" font-size="24">Valle</text></svg>"#;
    assert_eq!(dimensions(text).unwrap(), (60, 30));
    assert!(compile(text, &[]).unwrap_err().contains("font"));
    let font = include_bytes!("../../../assets/fonts/noto/NotoSans-Regular.ttf");
    let doc = compile(text, &[font.as_slice()]).unwrap();
    fn draws(ops: &[Op]) -> usize {
        ops.iter()
            .map(|op| match op {
                Op::Draw { .. } => 1,
                Op::Mask { ops, .. } => draws(ops),
                _ => 0,
            })
            .sum()
    }
    assert!(
        draws(&doc.ops) >= 1,
        "SVG text must become visible glyph outlines: {doc:?}"
    );
    let missing = std::str::from_utf8(text).unwrap().replace("Valle", "中文");
    assert!(
        compile(missing.as_bytes(), &[font.as_slice()])
            .unwrap_err()
            .contains("do not cover")
    );
    for child in [
        r#"<image href="file:///tmp/a.png"/>"#,
        r#"<image href="https://example.com/a.png"/>"#,
        r#"<animate attributeName="x"/>"#,
        r#"<foreignObject/>"#,
        r#"<script/>"#,
    ] {
        let source = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">{child}</svg>"#
        );
        assert!(compile(source.as_bytes(), &[]).is_err(), "{child}");
    }
}
