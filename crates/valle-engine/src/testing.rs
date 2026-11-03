//! Builders the unit tests share.

use std::rc::Rc;

use crate::template::{Kind, Node, Piece, Prop, V};
use crate::text::Text;

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
    Text::new().unwrap()
}
