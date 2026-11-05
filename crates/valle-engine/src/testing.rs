//! Builders the unit tests share.

use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Mutex;

use crate::template::{Kind, Node, Piece, Prop, V};
use crate::text::{Blob, BuiltIn, Fonts, Text};

/// A node with constant inline styles, at `s.tsx:1:1`.
pub fn node(kind: Kind, style: &[(&str, &str)], children: &[u32]) -> Node {
    Node {
        kind,
        path: None,
        classes: Vec::new(),
        style: style
            .iter()
            .map(|(k, v)| (Rc::from(*k), Prop::Const(V::Str((*v).into()))))
            .collect(),
        children: children.to_vec(),
        site: Some("s.tsx:1:1".into()),
    }
}

pub fn view(style: &[(&str, &str)], children: &[u32]) -> Node {
    node(Kind::View, style, children)
}

/// A `<Mask>` whose source is node `source`.
pub fn mask(
    (source, luminance, invert): (u32, bool, bool),
    style: &[(&str, &str)],
    children: &[u32],
) -> Node {
    let kind = Kind::Mask {
        source,
        luminance,
        invert,
    };
    node(kind, style, children)
}

pub fn text_node(s: &str, style: &[(&str, &str)]) -> Node {
    let content = vec![Piece::Text(Prop::Const(V::Str(s.into())))];
    node(Kind::Text { content }, style, &[])
}

/// Text in the built-in fonts.
pub fn text() -> Text {
    Text::new(fonts())
}

/// The file of built-in font `i`: 0 is Noto Sans, 2 Noto Serif.
pub fn built_in(i: usize) -> Blob<u8> {
    fonts()(&crate::text::BUILT_IN[i]).unwrap()
}

/// The built-in fonts, from the global cache, downloaded the first time.
pub fn fonts() -> Fonts {
    static GIVEN: Mutex<BTreeMap<&str, Blob<u8>>> = Mutex::new(BTreeMap::new());
    Rc::new(|font: &BuiltIn| {
        let mut given = GIVEN.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(blob) = given.get(font.sha256) {
            return Ok(blob.clone());
        }
        let cache = valle_native::urls::UrlCache::global()?;
        let path = cache.fetch_pinned(font.url, font.sha256, font.file)?;
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let blob = Blob::from(bytes);
        given.insert(font.sha256, blob.clone());
        Ok(blob)
    })
}

/// Style declarations and computed styles on the shared test canvas.
pub(crate) mod styles {
    use crate::front::{Holds, Slot};
    use crate::style::{Sheet, Style};
    use crate::template::{Error, Kind, Node, Prop, Template, V};
    use std::rc::Rc;
    use taffy::{LengthPercentage, LengthPercentageAuto};

    pub(crate) const CANVAS: [f32; 2] = [200.0, 100.0];

    pub(crate) fn text(s: &str) -> Prop {
        Prop::Const(V::Str(s.into()))
    }

    pub(crate) fn num(x: f64) -> Prop {
        Prop::Const(V::Num(x))
    }

    pub(crate) fn view(style: &[(&str, Prop)], children: &[u32]) -> Node {
        Node {
            kind: Kind::View,
            path: None,
            classes: Vec::new(),
            style: style
                .iter()
                .map(|(k, p)| (Rc::from(*k), p.clone()))
                .collect(),
            children: children.to_vec(),
            site: Some("s.tsx:1:1".into()),
        }
    }

    pub(crate) fn sheet(nodes: Vec<Node>) -> Result<Sheet, Error> {
        Sheet::new(&Template {
            nodes,
            roots: vec![0],
            ..Default::default()
        })
    }

    /// The scene's resources: photo, then tile.
    pub(crate) fn resources() -> [Slot; 2] {
        ["photo", "tile"].map(|name| Slot {
            name: name.into(),
            kind: Holds::Image,
            default: Vec::new(),
        })
    }

    pub(crate) fn styles(nodes: Vec<Node>, values: &[V]) -> Result<Vec<Style>, Error> {
        sheet(nodes)?.compute(values, true, CANVAS, &resources())
    }

    pub(crate) fn one(style: &[(&str, Prop)]) -> Style {
        styles(vec![view(style, &[])], &[]).unwrap().remove(0)
    }

    pub(crate) fn error(style: &[(&str, Prop)]) -> String {
        styles(vec![view(style, &[])], &[]).unwrap_err()
    }

    pub(crate) fn px(v: f32) -> LengthPercentage {
        LengthPercentage::length(v)
    }

    pub(crate) fn auto_px(v: f32) -> LengthPercentageAuto {
        LengthPercentageAuto::length(v)
    }
}

/// Display lists and layouts built from test nodes.
pub(crate) mod paint {
    use super::text;
    use crate::layout::{Layout, layout};
    use crate::paint::{Filter, Item, paint, work};
    use crate::style::{Blend, Sheet, color};
    use crate::template::{Node, Template};

    /// The display list on a 200 × 100 canvas, and the layout it paints.
    pub(crate) fn paint_of(nodes: Vec<Node>, roots: &[u32]) -> (Vec<Item>, Layout) {
        let template = Template {
            nodes,
            roots: roots.to_vec(),
            ..Default::default()
        };
        let canvas = [200.0, 100.0];
        let styles = Sheet::new(&template)
            .unwrap()
            .compute(&[], true, canvas, &[])
            .unwrap();
        let laid = layout(&template, &[], &styles, canvas, &mut text(), &[]).unwrap();
        (paint(&template, &styles, &laid, &[]), laid)
    }

    /// Items by the named color they are painted in, with any opacity folded into it: `red`,
    /// `red@0.5`, `text blue`, `group 0.5 [...]`.
    pub(crate) fn names(items: &[Item]) -> Vec<String> {
        let name = |c: [f32; 4]| {
            for n in ["red", "green", "blue", "gray", "black", "white"] {
                let w = work(color::parse(n).unwrap());
                if w.map(|v| v * c[3]) == c {
                    return match c[3] {
                        1.0 => n.to_string(),
                        a => format!("{n}@{a}"),
                    };
                }
            }
            panic!("no name for {c:?}")
        };
        items
            .iter()
            .map(|item| match item {
                Item::Rect { color, .. } => name(*color),
                Item::Ring { colors, .. } => format!("ring {}", name(colors[0])),
                Item::Gradient { ramp, .. } => format!("gradient of {}", ramp.segments.len()),
                Item::Image { image, .. } => format!("image {image}"),
                Item::Glyphs { color, .. } => format!("text {}", name(*color)),
                Item::Path { color, .. } => format!("path {}", name(*color)),
                Item::Mask { mask, items } => format!("mask {:?} {:?}", names(mask), names(items)),
                Item::Group {
                    opacity,
                    blend: Blend::Normal,
                    items,
                } => format!("group {opacity} {:?}", names(items)),
                Item::Group {
                    opacity,
                    blend,
                    items,
                } => format!("group {opacity} {blend:?} {:?}", names(items)),
                Item::Clip { items, .. } => format!("clip {:?}", names(items)),
                Item::Transform { items, .. } => format!("transform {:?}", names(items)),
                Item::Cut { items, .. } => format!("cut {:?}", names(items)),
                Item::Backdrop { filters, .. } => format!("backdrop of {}", filters.len()),
                Item::Shader { layers, .. } => {
                    let layers: Vec<String> = (layers.iter())
                        .map(|(name, items)| format!("{name} {:?}", names(items)))
                        .collect();
                    format!("shader {layers:?}")
                }
                Item::Filter { filters, items } => {
                    let filters: Vec<&str> = (filters.iter())
                        .map(|f| match f {
                            Filter::Blur(_) => "blur",
                            Filter::Colors(_) => "colors",
                            Filter::Shadow { .. } => "shadow",
                            Filter::Luminance => "luminance",
                            Filter::Aberration(_) => "aberration",
                            Filter::Lens { .. } => "lens",
                            Filter::Grain { .. } => "grain",
                            Filter::Radial { .. } => "radial",
                            Filter::Motion(_) => "motion",
                            Filter::Displacement { .. } => "displacement",
                            Filter::Bloom { .. } => "bloom",
                        })
                        .collect();
                    format!("filter {filters:?} {:?}", names(items))
                }
                Item::Shadow { inside, items, .. } => {
                    let side = if *inside { "inner" } else { "outer" };
                    format!("{side} shadow {:?}", names(items))
                }
            })
            .collect()
    }

    pub(crate) fn painted(nodes: Vec<Node>, roots: &[u32]) -> Vec<String> {
        names(&paint_of(nodes, roots).0)
    }
}
