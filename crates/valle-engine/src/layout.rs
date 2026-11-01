//! Layout: a frame's computed styles as a Taffy tree under the canvas, text measured as Taffy
//! asks, and every node's boxes in canvas px. The canvas is a block container the size of the
//! composition that clips what overflows it, and the containing block of what no positioned
//! ancestor contains.

use std::borrow::Cow;
use std::collections::HashMap;

use taffy::{
    AvailableSpace, Dimension, Display, NodeId, Overflow, Point, Position, Size, TaffyTree,
};

use crate::style::Style;
use crate::template::{Error, Kind, Prop, Template, V};
use crate::text::{Lines, Text};

/// A rectangle in canvas px, y down.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// A node's border box and content box.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Boxes {
    pub border: Rect,
    pub content: Rect,
}

/// A frame's layout, in template order. Nodes in a `display: none` subtree have empty boxes.
#[derive(Debug)]
pub struct Layout {
    pub boxes: Vec<Boxes>,
    /// Each shown text node's lines, from the top left of its content box.
    pub lines: Vec<Option<Lines>>,
}

fn bug(e: taffy::TaffyError) -> Error {
    format!("engine bug: layout: {e}")
}

/// Lays out a frame: the template's nodes with their computed styles and the program's values,
/// on a canvas of `canvas` px.
pub fn layout(
    template: &Template,
    values: &[V],
    styles: &[Style],
    canvas: [f32; 2],
    text: &mut Text,
) -> Result<Layout, Error> {
    let n = template.nodes.len();
    let strings: Vec<Option<Cow<str>>> = template
        .nodes
        .iter()
        .map(|node| match &node.kind {
            Kind::View => None,
            Kind::Text {
                text: Prop::Const(v),
            } => Some(v.text()),
            Kind::Text {
                text: Prop::Expr(e),
            } => Some(values[*e as usize].text()),
        })
        .collect();

    // A text node's context is its index.
    let mut tree: TaffyTree<u32> = TaffyTree::with_capacity(n + 1);
    tree.disable_rounding();
    let mut ids = Vec::with_capacity(n);
    for (i, (node, style)) in template.nodes.iter().zip(styles).enumerate() {
        let id = match node.kind {
            Kind::View => tree.new_leaf(style.layout.clone()),
            Kind::Text { .. } => tree.new_leaf_with_context(style.layout.clone(), i as u32),
        };
        ids.push(id.map_err(bug)?);
    }
    for (node, &id) in template.nodes.iter().zip(&ids) {
        if !node.children.is_empty() {
            let children: Vec<NodeId> = node.children.iter().map(|&c| ids[c as usize]).collect();
            tree.set_children(id, &children).map_err(bug)?;
        }
    }
    let roots: Vec<NodeId> = template.roots.iter().map(|&r| ids[r as usize]).collect();
    let canvas_style = taffy::Style {
        display: Display::Block,
        position: Position::Relative,
        overflow: Point {
            x: Overflow::Hidden,
            y: Overflow::Hidden,
        },
        size: Size {
            width: Dimension::length(canvas[0]),
            height: Dimension::length(canvas[1]),
        },
        ..taffy::Style::DEFAULT
    };
    let root = tree.new_with_children(canvas_style, &roots).map_err(bug)?;

    let mut failed = None;
    let available = Size {
        width: AvailableSpace::Definite(canvas[0]),
        height: AvailableSpace::Definite(canvas[1]),
    };
    tree.compute_layout_with_measure(root, available, |inputs, _, context, style| {
        taffy::compute_leaf_layout(
            inputs,
            style,
            |_, _| 0.0,
            |known, available| {
                let Some(&mut i) = context else {
                    return Size::ZERO;
                };
                let i = i as usize;
                let width = known.width.or(match available.width {
                    AvailableSpace::Definite(w) => Some(w),
                    AvailableSpace::MinContent => Some(0.0),
                    AvailableSpace::MaxContent => None,
                });
                let s = strings[i].as_deref().unwrap_or_default();
                match text.measure(s, &styles[i].font, width) {
                    Ok([w, h]) => Size {
                        width: known.width.unwrap_or(w),
                        height: known.height.unwrap_or(h),
                    },
                    Err(e) => {
                        failed.get_or_insert_with(|| format!("{}: {e}", template.nodes[i].place()));
                        Size::ZERO
                    }
                }
            },
        )
    })
    .map_err(bug)?;
    if let Some(e) = failed {
        return Err(e);
    }

    // Each container places its in-flow children and the out-of-flow boxes it is the containing
    // block of (Taffy's hoisted children): their locations are from its border box. Containers
    // come in preorder, the canvas first, so a box's container is placed before it.
    let index: HashMap<NodeId, usize> = ids.iter().enumerate().map(|(i, &id)| (id, i)).collect();
    let mut origins: Vec<Option<[f32; 2]>> = vec![None; n];
    let shown = |i: usize| styles[i].layout.display != Display::None;
    let out_of_flow = |i: usize| {
        matches!(
            styles[i].layout.position,
            Position::Absolute | Position::Fixed
        )
    };
    let containers = std::iter::once((root, Some([0.0, 0.0]), &template.roots[..])).chain(
        template
            .nodes
            .iter()
            .enumerate()
            .map(|(i, node)| (ids[i], None, &node.children[..])),
    );
    for (k, (id, at, children)) in containers.enumerate() {
        let Some(at) = at.or_else(|| origins[k - 1]) else {
            continue;
        };
        let from = |id: NodeId| -> Result<[f32; 2], Error> {
            let l = tree.layout(id).map_err(bug)?;
            Ok([at[0] + l.location.x, at[1] + l.location.y])
        };
        for &c in children {
            let c = c as usize;
            if shown(c) && !out_of_flow(c) {
                origins[c] = Some(from(ids[c])?);
            }
        }
        for h in tree.hoisted_children(id).map_err(bug)? {
            let i = index[h];
            if shown(i) {
                origins[i] = Some(from(*h)?);
            }
        }
    }

    let mut boxes = vec![Boxes::default(); n];
    let mut lines = vec![None; n];
    for (i, origin) in origins.iter().enumerate() {
        let Some([x, y]) = *origin else {
            continue;
        };
        let l = tree.layout(ids[i]).map_err(bug)?;
        let (inset, pad) = (l.border, l.padding);
        let content = Rect {
            x: x + inset.left + pad.left,
            y: y + inset.top + pad.top,
            width: l.size.width - inset.left - inset.right - pad.left - pad.right,
            height: l.size.height - inset.top - inset.bottom - pad.top - pad.bottom,
        };
        boxes[i] = Boxes {
            border: Rect {
                x,
                y,
                width: l.size.width,
                height: l.size.height,
            },
            content,
        };
        if let Some(s) = &strings[i] {
            let laid = text
                .lines(s, &styles[i].font, content.width)
                .map_err(|e| format!("{}: {e}", template.nodes[i].place()))?;
            lines[i] = Some(laid);
        }
    }
    Ok(Layout { boxes, lines })
}

#[cfg(test)]
mod tests;
