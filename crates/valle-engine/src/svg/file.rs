//! SVG files, as an Image shows them: parsed into the elements inline SVG has, checked by the same
//! tables, and styled once as a document of their own, which inherits nothing and whose ids are
//! its own, as Chrome draws an SVG image. Elements of other namespaces, as editors add, draw
//! nothing, and so do attributes of other namespaces but `xlink:href`. An `<image>` shows a PNG,
//! JPEG or WebP image the file embeds as a data URI; it reads no other files.

use std::rc::Rc;

use roxmltree::{Node as Xml, ParsingOptions};

use super::{Holds, INERT, NEVER, SILENT};
use crate::image::Image;
use crate::layout::Natural;
use crate::style::{Sheet, Style};
use crate::template::{Kind, Node, Prop, Tag, Template, V};

const SVG: &str = "http://www.w3.org/2000/svg";
const XLINK: &str = "http://www.w3.org/1999/xlink";

/// An SVG file's elements, in preorder from its root `<svg>`, and their styles.
#[derive(Debug)]
pub struct Document {
    pub template: Template,
    pub styles: Vec<Style>,
    /// Its natural size, or ratio, from its root's width, height and viewBox.
    pub natural: Natural,
    /// Its `<image>`s, in order.
    pub embedded: Vec<Embedded>,
}

/// An `<image>` of an SVG file: its node, and the scene's image it shows, `size` px big.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Embedded {
    pub node: u32,
    pub image: u32,
    pub size: [u32; 2],
}

/// Whether `bytes` may be an SVG file: text that starts with markup.
pub fn looks(bytes: &[u8]) -> bool {
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    bytes.iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'<')
}

impl Document {
    /// The SVG file `text`, styled with its natural size as the viewport `vw` and `vh` measure,
    /// or 300 × 150 px without one; the images it embeds go onto `images`. Errors point at
    /// `line:column`.
    pub fn parse(text: &str, images: &mut Vec<Image>) -> Result<Document, String> {
        let options = ParsingOptions {
            allow_dtd: true,
            ..ParsingOptions::default()
        };
        let xml = roxmltree::Document::parse_with_options(text, options)
            .map_err(|e| format!("not an SVG file: {e}"))?;
        let root = xml.root_element();
        let name = root.tag_name();
        if name.name() != "svg" {
            return Err(format!("not an SVG file: its root is <{}>", name.name()));
        }
        if name.namespace() != Some(SVG) {
            return Err(format!(
                "its <svg> needs xmlns=\"{SVG}\", without which browsers do not draw it"
            ));
        }
        let mut out = Out {
            nodes: Vec::new(),
            embedded: Vec::new(),
            images,
        };
        element(&xml, root, None, &mut out)?;
        let Out {
            nodes, embedded, ..
        } = out;
        let natural = natural(sides(&xml, root)?, root.attribute("viewBox"));
        let template = Template {
            nodes,
            roots: vec![0],
            ..Template::default()
        };
        let viewport = match natural {
            Natural::Size(size) => size,
            Natural::Ratio(_) => [300.0, 150.0],
        };
        let styles = Sheet::new(&template)?.compute(&[], viewport, &[])?;
        Ok(Document {
            template,
            styles,
            natural,
            embedded,
        })
    }
}

/// Where elements go as they are read: their nodes in preorder, the `<image>`s among them, and
/// the images those embed.
struct Out<'a> {
    nodes: Vec<Node>,
    embedded: Vec<Embedded>,
    images: &'a mut Vec<Image>,
}

/// Element `x` of `xml`, in `parent`, and those in it, onto `out`: its node's index, or `None`
/// for one that draws nothing, as one of another namespace.
fn element(
    xml: &roxmltree::Document,
    x: Xml,
    parent: Option<Tag>,
    out: &mut Out<'_>,
) -> Result<Option<u32>, String> {
    let name = x.tag_name().name();
    let fail = |what: String| Err(format!("{}: {what}", place(xml, x)));
    if x.tag_name().namespace() != Some(SVG) || SILENT.contains(&name) {
        return Ok(None);
    }
    if let Some((_, instead)) = NEVER.iter().find(|(n, _)| *n == name) {
        return fail(format!("<{name}> is not supported: {instead}"));
    }
    let Some(tag) = Tag::by_name(name) else {
        return fail(format!("<{name}> is not supported yet"));
    };
    if let Some(p) = parent {
        let only = match super::holds(p) {
            Holds::Any => None,
            Holds::Nothing => return fail(format!("<{}> holds no elements", p.name())),
            Holds::Only(tags, what) => Some((tags, what)),
        };
        match only {
            Some((tags, what)) if !tags.contains(&tag) => {
                return fail(format!(
                    "<{name}> cannot be inside <{}>: it holds {what}",
                    p.name()
                ));
            }
            None if tag == Tag::Stop => {
                return fail("<stop> must be inside a <linearGradient> or <radialGradient>".into());
            }
            None if tag == Tag::Svg => {
                return fail("an <svg> inside an <svg> is not supported yet".into());
            }
            _ => {}
        }
    }
    let mut attributes = Vec::new();
    let mut style = Vec::new();
    for a in x.attributes() {
        let key = match a.namespace() {
            None => camel(a.name()),
            Some(XLINK) if a.name() == "href" => "xlinkHref".into(),
            Some(_) => continue,
        };
        let root = parent.is_none() && matches!(&*key, "x" | "y" | "width" | "height");
        let inert = INERT.contains(&&*key)
            || ["data-", "aria-"].iter().any(|p| a.name().starts_with(p))
            || key == "class";
        if root || inert {
            continue;
        }
        if parent.is_none() && key == "transform" {
            return fail("<svg> transform is not supported yet".into());
        }
        if key == "style" {
            style.extend(declarations(a.value()));
            continue;
        }
        // An image's data is read below, and kept once.
        if tag == Tag::Image && matches!(&*key, "href" | "xlinkHref") {
            continue;
        }
        if !super::takes(tag, &key) {
            return fail(format!("<{name}> {} is not supported yet", a.name()));
        }
        attributes.push((Rc::from(key), Prop::Const(V::Str(a.value().into()))));
    }
    let index = out.nodes.len();
    if tag == Tag::Image {
        let href = (x.attribute("href")).or_else(|| x.attribute((XLINK, "href")));
        let image = embedded(href).map_err(|e| format!("{}: <image> {e}", place(xml, x)))?;
        out.embedded.push(Embedded {
            node: index as u32,
            image: out.images.len() as u32,
            size: image.size,
        });
        out.images.push(image);
    }
    out.nodes.push(Node {
        kind: Kind::Svg { tag, attributes },
        path: None,
        classes: Vec::new(),
        style,
        children: Vec::new(),
        site: Some(place(xml, x).into()),
    });
    let mut children = Vec::new();
    for c in x.children().filter(Xml::is_element) {
        children.extend(element(xml, c, Some(tag), out)?);
    }
    out.nodes[index].children = children;
    Ok(Some(index as u32))
}

/// The image an `<image>`'s href holds: a PNG, JPEG or WebP file as a base64 data URI.
fn embedded(href: Option<&str>) -> Result<Image, String> {
    let href = href
        .ok_or("needs an href: a PNG, JPEG or WebP image as a data URI")?
        .trim();
    let Some(data) = href.strip_prefix("data:") else {
        return Err(format!(
            "href: an SVG file reads no other files: embed {href} as a data URI"
        ));
    };
    let (_, data) = (data.split_once(','))
        .filter(|(kind, _)| kind.ends_with(";base64"))
        .ok_or("href: an image's data URI is base64: data:image/png;base64,…")?;
    let bytes = base64(data).ok_or("href: its data is not base64")?;
    if looks(&bytes) {
        return Err("href: an SVG file in an SVG file is not supported yet".into());
    }
    Image::decode(&bytes).map_err(|e| format!("href: {e}"))
}

/// The bytes base64 `text` holds, white space left out; `None` if it is not base64.
fn base64(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let (mut bits, mut held) = (0u32, 0);
    let text = text.trim_end_matches(|c: char| c == '=' || c.is_ascii_whitespace());
    for c in text.bytes().filter(|c| !c.is_ascii_whitespace()) {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        bits = bits << 6 | u32::from(v);
        held += 6;
        if held >= 8 {
            held -= 8;
            out.push((bits >> held) as u8);
        }
    }
    Some(out)
}

/// Where `x` starts in `xml`: `line:column`.
fn place(xml: &roxmltree::Document, x: Xml) -> String {
    let p = xml.text_pos_at(x.range().start);
    format!("{}:{}", p.row, p.col)
}

/// An attribute's name as JSX writes it: `stroke-width` as `strokeWidth`.
fn camel(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut upper = false;
    for c in name.chars() {
        match c {
            '-' => upper = true,
            _ if upper => {
                out.push(c.to_ascii_uppercase());
                upper = false;
            }
            _ => out.push(c),
        }
    }
    out
}

/// The declarations of a `style` attribute, as React names their properties, without those that
/// change nothing.
fn declarations(css: &str) -> Vec<(Rc<str>, Prop)> {
    let declared = css.split(';').filter_map(|d| {
        let (key, value) = d.split_once(':')?;
        let key = camel(&key.trim().to_ascii_lowercase());
        let value = value.trim();
        let value = value.strip_suffix("!important").unwrap_or(value).trim();
        let shown = !key.is_empty() && !INERT.contains(&&*key);
        shown.then(|| (Rc::from(key), Prop::Const(V::Str(value.into()))))
    });
    declared.collect()
}

/// The width and height of an SVG file with root `x` in px, those in absolute units.
fn sides(xml: &roxmltree::Document, x: Xml) -> Result<[Option<f32>; 2], String> {
    let side = |name: &str| match x.attribute(name) {
        Some(v) => absolute(v).map_err(|e| format!("{}: <svg> {name}: {e}", place(xml, x))),
        None => Ok(None),
    };
    Ok([side("width")?, side("height")?])
}

/// The natural size of an SVG file with absolute `sides` and `view_box`: those sides, one it
/// lacks from the other by its ratio (its viewBox's, else theirs), or 300 by 150 px as CSS sizes
/// a replaced box; with neither, its ratio alone, if it has one.
fn natural([w, h]: [Option<f32>; 2], view_box: Option<&str>) -> Natural {
    let boxed = view_box
        .and_then(|v| super::view_box(v).ok())
        .filter(|&[.., w, h]| w > 0.0 && h > 0.0);
    let ratio = match (boxed, w, h) {
        (Some([.., bw, bh]), ..) => Some((bw / bh) as f32),
        (None, Some(w), Some(h)) if w > 0.0 && h > 0.0 => Some(w / h),
        _ => None,
    };
    match (w, h, ratio) {
        (Some(w), Some(h), _) => Natural::Size([w, h]),
        (Some(w), None, Some(r)) => Natural::Size([w, w / r]),
        (None, Some(h), Some(r)) => Natural::Size([h * r, h]),
        (Some(w), None, None) => Natural::Size([w, 150.0]),
        (None, Some(h), None) => Natural::Size([300.0, h]),
        (None, None, r) => Natural::Ratio(r),
    }
}

/// A width or height in px: a number, or one in px, pt, pc, in, cm, mm or Q; `None` for a
/// percentage or `auto`, which give no natural size.
fn absolute(v: &str) -> Result<Option<f32>, String> {
    const UNITS: [(&str, f32); 7] = [
        ("px", 1.0),
        ("pt", 4.0 / 3.0),
        ("pc", 16.0),
        ("in", 96.0),
        ("cm", 96.0 / 2.54),
        ("mm", 96.0 / 25.4),
        ("Q", 96.0 / 101.6),
    ];
    let v = v.trim();
    if v == "auto" || v.ends_with('%') {
        return Ok(None);
    }
    let (number, k) = (UNITS.iter())
        .find_map(|&(unit, k)| Some((v.strip_suffix(unit)?, k)))
        .unwrap_or((v, 1.0));
    match number.trim().parse::<f32>() {
        Ok(x) if x >= 0.0 && x.is_finite() => Ok(Some(x * k)),
        _ => Err(format!(
            "{v} is not a length in px, pt, pc, in, cm, mm or Q, a percentage or auto"
        )),
    }
}

#[cfg(test)]
mod tests;
