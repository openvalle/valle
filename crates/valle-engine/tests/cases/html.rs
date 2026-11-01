//! A frame as HTML for Chrome: every node an element with its style as written, printed as React
//! DOM prints a style object, on a canvas like the engine's. Elements carry their node's index,
//! `data-n`, and text nodes `data-text`.

use std::fmt::Write as _;

use valle_engine::front::Composition;
use valle_engine::js;
use valle_engine::scene::Frame;
use valle_engine::template::{Kind, Prop, V};

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

pub fn print(frame: &Frame, c: Composition) -> String {
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
        node(&mut html, frame, root);
    }
    html += "</div>\n";
    html
}

fn node(html: &mut String, frame: &Frame, i: u32) {
    let n = &frame.template.nodes[i as usize];
    let _ = write!(html, "<div data-n=\"{i}\"");
    if matches!(n.kind, Kind::Text { .. }) {
        html.push_str(" data-text");
    }
    let style: Vec<String> = (n.style.iter())
        .filter_map(|(key, prop)| declaration(key, value(frame, prop)))
        .collect();
    if !style.is_empty() {
        let _ = write!(html, " style=\"{}\"", escape(&style.join(" ")));
    }
    html.push('>');
    match &n.kind {
        Kind::View => {
            for &child in &n.children {
                node(html, frame, child);
            }
        }
        Kind::Text { text } => html.push_str(&escape(&value(frame, text).text())),
    }
    html.push_str("</div>");
}

/// A prop's value at the frame.
pub fn value<'a>(frame: &'a Frame, prop: &'a Prop) -> &'a V {
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
                "sans-serif" => SANS_SERIF,
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
