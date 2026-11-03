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
        let path = cache.fetch_pinned(font.url, font.sha256)?;
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let blob = Blob::from(bytes);
        given.insert(font.sha256, blob.clone());
        Ok(blob)
    })
}
