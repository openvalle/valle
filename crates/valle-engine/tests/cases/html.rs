//! A frame as HTML for Chrome: every node an element with its style as written, printed as React
//! DOM prints a style object, on a canvas like the engine's: a Span as a `span`, an Image as an
//! `img` of a block that shows its file from the server, an SVG element as itself with its
//! attributes, an `<svg>` a block, and every other node as a `div`; a resource in a style's
//! `url()` is its file too. Elements carry their node's index, `data-n`, Texts `data-text`, and
//! their classes; a page with classes asks for the stylesheet the pinned Tailwind builds of them,
//! which `web/tests/chrome.ts` puts in.

use std::fmt::Write as _;

use valle_engine::front::{Composition, Slot};
use valle_engine::js;
use valle_engine::scene::Frame;
use valle_engine::svg;
use valle_engine::template::{Kind, Piece, Prop, Tag, V};
use valle_engine::text::{BUILT_IN, FontFile};

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

/// The built-in families: their `font-family` names, the names the pages give them, their upright
/// and italic fonts in `BUILT_IN`, which a page names by their files' names, and the Tailwind
/// class that names them through the theme `web/tests/tailwind.ts` gives.
const FAMILIES: [(&str, &str, usize, Option<usize>, &str); 3] = [
    ("sans-serif", "\"Valle Sans\"", 0, Some(1), "font-sans"),
    ("serif", "\"Valle Serif\"", 2, Some(3), "font-serif"),
    ("monospace", "\"Valle Mono\"", 4, None, "font-mono"),
];
const SANS_SERIF: &str = FAMILIES[0].1;

/// The frame's HTML, or what in it has none; `resources` are the scene's, their files paths from
/// the cases' folder, `fonts` the files of its font resources, and `tailwind` the Tailwind
/// version its classes are compiled by.
pub fn print(
    frame: &Frame,
    c: Composition,
    resources: &[Slot],
    fonts: &[FontFile],
    tailwind: &str,
) -> Result<String, String> {
    let mut body = "<div id=\"canvas\">".to_string();
    for &root in &frame.template.roots {
        node(&mut body, frame, resources, root)?;
    }
    body += "</div>\n";
    let mut html = "<!doctype html>\n<meta charset=\"utf-8\">\n<style>\n".to_string();
    // The sans-serif always, as the canvas's font; the others where the page names them.
    for (i, &(_, name, upright, italic, class)) in FAMILIES.iter().enumerate() {
        // The body escapes the quotes.
        if !(i == 0 || body.contains(name.trim_matches('"')) || body.contains(class)) {
            continue;
        }
        let faces = [(upright, "normal")].into_iter();
        for (font, style) in faces.chain(italic.map(|i| (i, "italic"))) {
            let _ = writeln!(
                html,
                "@font-face {{ font-family: {name}; src: url(\"{}\"); \
                 font-weight: 100 900; font-stretch: 62.5% 100%; font-style: {style}; }}",
                BUILT_IN[font].file
            );
        }
    }
    // Font resources, each a family by its name, a face per file as the file describes it.
    for f in fonts {
        let range = |[low, high]: [f32; 2], unit: &str| match low == high {
            true => format!("{low}{unit}"),
            false => format!("{low}{unit} {high}{unit}"),
        };
        let _ = writeln!(
            html,
            "@font-face {{ font-family: \"{}\"; src: url(\"{}\"); font-weight: {}; \
             font-stretch: {}; font-style: {}; }}",
            f.family,
            location(&f.path)?,
            range(f.weight, ""),
            range(f.width, "%"),
            if f.italic { "italic" } else { "normal" }
        );
    }
    let _ = write!(
        html,
        "*, ::before, ::after {{ box-sizing: border-box; }}\n\
         body {{ margin: 0; }}\n\
         #canvas {{ position: relative; overflow: hidden; width: {}px; height: {}px; \
         font-family: {SANS_SERIF}; }}\n\
         </style>\n",
        c.width, c.height
    );
    if frame.template.nodes.iter().any(|n| !n.classes.is_empty()) {
        let _ = writeln!(html, "<style data-tailwind=\"{tailwind}\"></style>");
    }
    html += &body;
    Ok(html)
}

fn node(html: &mut String, frame: &Frame, resources: &[Slot], i: u32) -> Result<(), String> {
    let n = &frame.template.nodes[i as usize];
    if let Kind::Mask { .. } = n.kind {
        return Err(format!(
            "{}: <Mask> is not CSS, so Chrome cannot draw it; its cases go outside css/ and svg/",
            n.place()
        ));
    }
    // A Span is an inline box.
    let tag = match &n.kind {
        Kind::Span { .. } => "span",
        Kind::Image { .. } => "img",
        Kind::Svg { tag, .. } => tag.name(),
        _ => "div",
    };
    let _ = write!(html, "<{tag} data-n=\"{i}\"");
    if !n.classes.is_empty() {
        let _ = write!(html, " class=\"{}\"", escape(&n.classes.join(" ")));
    }
    if let Kind::Svg { tag, attributes } = &n.kind {
        for (name, prop) in attributes {
            let v = value(frame, prop).text();
            let _ = write!(html, " {}=\"{}\"", attribute(*tag, name), escape(&v));
        }
    }
    if matches!(n.kind, Kind::Text { .. }) {
        html.push_str(" data-text");
    }
    let mut style = Vec::new();
    if let Kind::Image { resource } = n.kind {
        let src = file(&resources[resource as usize])?;
        let _ = write!(html, " src=\"{}\"", escape(&src));
        style.push("display: block;".to_string());
    }
    if let Kind::Svg { tag: Tag::Svg, .. } = n.kind {
        style.push("display: block;".to_string());
    }
    for (key, prop) in &n.style {
        if let Some(d) = declaration(key, value(frame, prop)) {
            style.push(served(&d, resources)?);
        }
    }
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

/// Where the server gives a resource's file.
fn file(slot: &Slot) -> Result<String, String> {
    match &slot.default[..] {
        [path] => location(path),
        _ => Err(format!("{}: not one file", slot.name)),
    }
}

/// Where the server gives the file at `path`; a pinned font's from the web is its file's name.
fn location(path: &str) -> Result<String, String> {
    if path.starts_with("https://") {
        let font = (super::pinned().find(|f| f.url == path))
            .ok_or_else(|| format!("{path}: cases read only the pinned fonts from the web"))?;
        return Ok(font.file.to_string());
    }
    Ok(format!("/cases/{path}"))
}

/// `text` with each `resource://` name in it the path of the resource's file.
fn served(text: &str, resources: &[Slot]) -> Result<String, String> {
    const SCHEME: &str = "resource://";
    let mut out = String::new();
    let mut rest = text;
    while let Some(i) = rest.find(SCHEME) {
        out.push_str(&rest[..i]);
        rest = &rest[i + SCHEME.len()..];
        let end = (rest.find(|c: char| !(c.is_ascii_alphanumeric() || "._-".contains(c))))
            .unwrap_or(rest.len());
        let slot = (resources.iter())
            .find(|s| s.name == rest[..end])
            .ok_or_else(|| format!("no resource {}", &rest[..end]))?;
        out += &file(slot)?;
        rest = &rest[end..];
    }
    out.push_str(rest);
    Ok(out)
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
            .map(|f| {
                let f = f.trim();
                (FAMILIES.iter())
                    .find(|(generic, ..)| f.eq_ignore_ascii_case(generic))
                    .map_or(f, |(_, name, ..)| name)
            })
            .collect::<Vec<_>>()
            .join(", "),
        V::Str(s) => s.trim().to_string(),
        V::Bool(_) => return None,
    };
    Some(format!("{}: {value};", hyphenate(key)))
}

/// An SVG attribute's name as React DOM writes it: a presentation attribute's as its property's,
/// `fillOpacity` as `fill-opacity`, `xlinkHref` as `xlink:href`, and others as they are, as
/// `viewBox` and `pathLength`.
fn attribute(tag: Tag, name: &str) -> String {
    match name {
        "xlinkHref" => "xlink:href".into(),
        _ if svg::presentational(tag, name) => hyphenate(name),
        _ => name.to_string(),
    }
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
