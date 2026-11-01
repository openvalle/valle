//! A frame as HTML for Chrome: every node an element with its style as written, printed as React
//! DOM prints a style object, on a canvas like the engine's: a Span as a `span`, an Image as an
//! `img` of a block that shows its file from the server, and every other node as a `div`.
//! Elements carry their node's index, `data-n`, and Texts `data-text`.

use std::fmt::Write as _;

use valle_engine::front::{Composition, Slot};
use valle_engine::js;
use valle_engine::scene::Frame;
use valle_engine::template::{Kind, Piece, Prop, V};

/// Style properties React DOM prints as bare numbers; it gives other numbers but 0 `px`.
const UNITLESS: &[&str] = &[
    "animationIterationCount",
    "aspectRatio",
    "borderImageOutset",
    "borderImageSlice",
    "borderImageWidth",
    "columnCount",
    "columns",
    "flex",
    "flexGrow",
    "flexShrink",
    "fontWeight",
    "gridArea",
    "gridColumn",
    "gridColumnEnd",
    "gridColumnStart",
    "gridRow",
    "gridRowEnd",
    "gridRowStart",
    "lineClamp",
    "WebkitLineClamp",
    "lineHeight",
    "opacity",
    "order",
    "scale",
    "tabSize",
    "zIndex",
    "zoom",
    "fillOpacity",
    "floodOpacity",
    "stopOpacity",
    "strokeDasharray",
    "strokeDashoffset",
    "strokeMiterlimit",
    "strokeOpacity",
    "strokeWidth",
];

/// The built-in sans-serif as Chrome loads it, from the server `web/tests/chrome.ts` runs.
const SANS_SERIF: &str = "\"Valle Sans\"";

/// The frame's HTML, or what in it has none; `resources` are the scene's, their files paths from
/// the cases' folder.
pub fn print(frame: &Frame, c: Composition, resources: &[Slot]) -> Result<String, String> {
    let mut html = format!(
        "<!doctype html>\n<meta charset=\"utf-8\">\n<style>\n\
         @font-face {{ font-family: {SANS_SERIF}; src: url(\"/fonts/NotoSans-Variable.ttf\"); \
         font-weight: 100 900; font-stretch: 62.5% 100%; }}\n\
         *, ::before, ::after {{ box-sizing: border-box; }}\n\
         body {{ margin: 0; }}\n\
         #canvas {{ position: relative; overflow: hidden; width: {}px; height: {}px; \
         font-family: {SANS_SERIF}; }}\n\
         </style>\n<div id=\"canvas\">",
        c.width, c.height
    );
    for &root in &frame.template.roots {
        node(&mut html, frame, resources, root)?;
    }
    html += "</div>\n";
    Ok(html)
}

fn node(html: &mut String, frame: &Frame, resources: &[Slot], i: u32) -> Result<(), String> {
    let n = &frame.template.nodes[i as usize];
    if let Kind::Mask { .. } = n.kind {
        return Err(format!(
            "{}: <Mask> is not CSS, so Chrome cannot draw it; its cases go outside css/",
            n.place()
        ));
    }
    // A Span is an inline box.
    let tag = match n.kind {
        Kind::Span { .. } => "span",
        Kind::Image { .. } => "img",
        _ => "div",
    };
    let _ = write!(html, "<{tag} data-n=\"{i}\"");
    if matches!(n.kind, Kind::Text { .. }) {
        html.push_str(" data-text");
    }
    let mut style = Vec::new();
    if let Kind::Image { resource } = n.kind {
        let slot = &resources[resource as usize];
        let path = slot.default.as_deref().ok_or("an image without a file")?;
        let _ = write!(html, " src=\"/cases/{}\"", escape(path));
        style.push("display: block;".to_string());
    }
    style.extend((n.style.iter()).filter_map(|(key, prop)| declaration(key, value(frame, prop))));
    // Chrome draws a decoration in `currentcolor` in the color of the text's stroke or fill,
    // which hide with the glyphs for the runner; its color as a value keeps it.
    let declared = frame.styles[i as usize].decorations.last();
    if let Some(d) = declared.filter(|d| d.by == i as usize) {
        let [r, g, b] = [0, 1, 2].map(|k| js::number_to_string(d.color[k] * 255.0));
        let a = js::number_to_string(d.color[3]);
        style.push(format!("text-decoration-color: rgb({r} {g} {b} / {a});"));
    }
    if !style.is_empty() {
        let _ = write!(html, " style=\"{}\"", escape(&style.join(" ")));
    }
    html.push('>');
    match &n.kind {
        Kind::Text { content } | Kind::Span { content } => {
            for piece in content {
                match piece {
                    Piece::Text(text) => html.push_str(&escape(&value(frame, text).text())),
                    Piece::Span(span) => node(html, frame, resources, *span)?,
                }
            }
        }
        _ => {
            for &child in &n.children {
                node(html, frame, resources, child)?;
            }
        }
    }
    // An img has no end tag.
    if tag != "img" {
        let _ = write!(html, "</{tag}>");
    }
    Ok(())
}

/// A prop's value at the frame.
fn value<'a>(frame: &'a Frame, prop: &'a Prop) -> &'a V {
    match prop {
        Prop::Const(v) => v,
        Prop::Expr(e) => &frame.values[*e as usize],
    }
}

/// `key: value;` as React DOM writes it, or nothing for what it leaves out.
fn declaration(key: &str, v: &V) -> Option<String> {
    let value = match v {
        V::Num(x) if *x == 0.0 || UNITLESS.contains(&key) => js::number_to_string(*x),
        V::Num(x) => format!("{}px", js::number_to_string(*x)),
        V::Str(s) if key == "fontFamily" => (s.split(','))
            .map(|f| match f.trim() {
                name if name.eq_ignore_ascii_case("sans-serif") => SANS_SERIF,
                name => name,
            })
            .collect::<Vec<_>>()
            .join(", "),
        V::Str(s) => s.trim().to_string(),
        V::Bool(_) => return None,
    };
    Some(format!("{}: {value};", hyphenate(key)))
}

/// `backgroundColor` to `background-color`, `WebkitTextStroke` to `-webkit-text-stroke`.
fn hyphenate(key: &str) -> String {
    let mut out = String::with_capacity(key.len() + 4);
    for c in key.chars() {
        if c.is_ascii_uppercase() {
            out.push('-');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    match out.strip_prefix("ms-") {
        Some(rest) => format!("-ms-{rest}"),
        None => out,
    }
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
    out
}
