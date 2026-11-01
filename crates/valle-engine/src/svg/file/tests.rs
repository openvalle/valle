use super::*;
use crate::style::Paint;
use crate::svg::Element;

fn tags(d: &Document) -> Vec<&'static str> {
    (d.template.nodes.iter())
        .map(|n| match n.kind {
            Kind::Svg { tag, .. } => tag.name(),
            _ => "?",
        })
        .collect()
}

fn error(text: &str) -> String {
    Document::parse(text).unwrap_err()
}

#[test]
fn files_keep_the_svg_elements_and_drop_what_editors_add() {
    let text = r##"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd">
<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"
     xmlns:inkscape="http://www.inkscape.org/namespaces/inkscape"
     xmlns:sodipodi="http://sodipodi.sourceforge.net/DTD/sodipodi-0.dtd"
     version="1.1" x="0px" y="0px" width="24" height="12" viewBox="0 0 24 12"
     xml:space="preserve" enable-background="new 0 0 24 12" class="icon" data-name="Layer 1">
  <title>An icon</title>
  <sodipodi:namedview inkscape:zoom="2"><path d="M0 0" /></sodipodi:namedview>
  <defs>
    <linearGradient id="g"><stop offset="0" stop-color="red" /></linearGradient>
  </defs>
  <g inkscape:label="Layer" style="fill: blue; stroke-width: 3 !important; enable-background: new">
    <circle cx="6" cy="6" r="5" />
    <use xlink:href="#dot" x="12" />
  </g>
  <rect id="dot" width="4" height="4" fill="url(#g)" />
</svg>
"##;
    let d = Document::parse(text).unwrap();
    assert_eq!(
        tags(&d),
        [
            "svg",
            "defs",
            "linearGradient",
            "stop",
            "g",
            "circle",
            "use",
            "rect"
        ]
    );
    assert_eq!(d.template.nodes[0].children, [1, 4, 7]);
    assert_eq!(d.template.nodes[5].site.as_deref(), Some("14:5"));
    // The root's size is the file's natural size, not its box's.
    let Kind::Svg { attributes, .. } = &d.template.nodes[0].kind else {
        unreachable!()
    };
    let names: Vec<&str> = attributes.iter().map(|(k, _)| &**k).collect();
    assert_eq!(names, ["viewBox"]);
    assert_eq!(d.natural, Natural::Size([24.0, 12.0]));
    // The style attribute, as a style; the circle inherits its fill.
    let circle = &d.styles[5];
    assert_eq!(circle.fill, Paint::Color([0.0, 0.0, 1.0, 1.0]));
    assert_eq!(circle.stroke.width.resolve(0.0), 3.0);
    let Kind::Svg { attributes, .. } = &d.template.nodes[6].kind else {
        unreachable!()
    };
    assert_eq!(&*attributes[0].0, "xlinkHref");
    assert!(d.styles[6].instance.is_some());
    assert_eq!(d.styles[7].fill, Paint::Url("g".into()));
    let Some(Element::Svg { view_box, .. }) = d.styles[0].svg.as_deref().map(|c| &c.element) else {
        unreachable!()
    };
    assert_eq!(*view_box, Some([0.0, 0.0, 24.0, 12.0]));
}

#[test]
fn natural_sizes_come_from_absolute_sides_and_the_view_box() {
    let natural = |attributes: &str| {
        let text = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" {attributes}/>"#);
        Document::parse(&text).unwrap().natural
    };
    assert_eq!(
        natural(r#"width="40" height="20""#),
        Natural::Size([40.0, 20.0])
    );
    assert_eq!(
        natural(r#"width="3pt" height="1in""#),
        Natural::Size([4.0, 96.0])
    );
    let mm = natural(r#"width="25.4mm" height="2.54cm""#);
    assert_eq!(mm, Natural::Size([96.0, 96.0]));
    // A side from the other by the viewBox's ratio, else theirs.
    let sized = natural(r#"width="40" viewBox="0 0 4 1""#);
    assert_eq!(sized, Natural::Size([40.0, 10.0]));
    let sized = natural(r#"height="40" viewBox="0 0 4 1""#);
    assert_eq!(sized, Natural::Size([160.0, 40.0]));
    // A side without a ratio, the other as CSS sizes a replaced box.
    assert_eq!(natural(r#"width="40""#), Natural::Size([40.0, 150.0]));
    assert_eq!(natural(r#"height="40""#), Natural::Size([300.0, 40.0]));
    // Percentages give no size.
    let ratio = natural(r#"width="100%" height="50%" viewBox="0 0 4 1""#);
    assert_eq!(ratio, Natural::Ratio(Some(4.0)));
    assert_eq!(natural(""), Natural::Ratio(None));
    assert_eq!(natural(r#"viewBox="0 0 0 10""#), Natural::Ratio(None));
    let e = error(r#"<svg xmlns="http://www.w3.org/2000/svg" width="2em"/>"#);
    assert!(
        e.starts_with("1:1: <svg> width: 2em is not a length"),
        "{e}"
    );
}

#[test]
fn files_name_what_they_cannot_draw_by_line_and_column() {
    let svg = |inside: &str| {
        let text = format!("<svg xmlns=\"http://www.w3.org/2000/svg\">\n  {inside}\n</svg>");
        error(&text)
    };
    assert_eq!(svg("<text>Hi</text>"), "2:3: <text> is not supported yet");
    let e = svg("<foreignObject />");
    assert!(
        e.starts_with("2:3: <foreignObject> is not supported: "),
        "{e}"
    );
    let e = svg(r#"<circle r="2" vector-effect="non-scaling-stroke" />"#);
    assert_eq!(e, "2:3: <circle> vector-effect is not supported yet");
    let e = svg("<svg />");
    assert_eq!(e, "2:3: an <svg> inside an <svg> is not supported yet");
    let e = svg("<clipPath><g /></clipPath>");
    assert_eq!(e, "2:13: <g> cannot be inside <clipPath>: it holds shapes");
    assert_eq!(
        svg("<circle><g /></circle>"),
        "2:11: <circle> holds no elements"
    );
    // Ids name the file's own elements.
    let e = svg(r#"<rect clip-path="url(#nothing)" />"#);
    assert_eq!(e, "2:3: clip-path: url(#nothing) names no <clipPath>");
    let e = error(r#"<svg xmlns="http://www.w3.org/2000/svg" transform="scale(2)" />"#);
    assert_eq!(e, "1:1: <svg> transform is not supported yet");
    let e = error("<svg><rect /></svg>");
    assert!(e.contains("xmlns=\"http://www.w3.org/2000/svg\""), "{e}");
    let e = error(r#"<html xmlns="http://www.w3.org/1999/xhtml" />"#);
    assert_eq!(e, "not an SVG file: its root is <html>");
    assert!(error("<svg").starts_with("not an SVG file: "));
}

#[test]
fn files_look_like_markup() {
    assert!(looks(b"\xef\xbb\xbf\n  <svg/>"));
    assert!(looks(b"<?xml version=\"1.0\"?><svg/>"));
    assert!(!looks(b"\x89PNG\r\n\x1a\n"));
    assert!(!looks(b"// <svg>"));
}
