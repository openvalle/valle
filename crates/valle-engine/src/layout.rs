//! Layout: Taffy's algorithms run over a frame's nodes under the canvas, on a tree that borrows
//! their computed styles; text measured as Taffy asks and aligned on its first baseline; and
//! every node's boxes in canvas px. The canvas is a block container the size of the composition
//! that clips what overflows it, and the containing block of fixed boxes and of what no
//! positioned ancestor contains.

use std::borrow::Cow;

use taffy::{
    AvailableSpace, BlockContext, Cache, CacheTree, Dimension, Display, LayoutBlockContainer,
    LayoutContainingBlock, LayoutFlexboxContainer, LayoutInput, LayoutOutput, LayoutPartialTree,
    LengthPercentage, NodeId, Overflow, Point, Position, ResolveOrZero, RunMode, Size,
    TraversePartialTree, compute_block_layout, compute_cached_layout, compute_flexbox_layout,
    compute_hidden_layout, compute_leaf_layout, compute_oof_layout, compute_root_layout,
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

/// A container's children in the order it lays them out and paints them: a flex container's by
/// `order` (out-of-flow ones as 0), then as written; any other's as written.
pub fn ordered<'a>(
    children: &'a [u32],
    container: &Style,
    styles: &'a [Style],
) -> impl Iterator<Item = usize> + 'a {
    let mut list: Vec<usize> = children.iter().map(|&c| c as usize).collect();
    if container.layout.display == Display::Flex {
        list.sort_by_key(|&c| match styles[c].layout.position {
            Position::Absolute | Position::Fixed => 0,
            _ => styles[c].order,
        });
    }
    list.into_iter()
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
    let mut children: Vec<Vec<NodeId>> = (template.nodes.iter().zip(styles))
        .map(|(node, style)| {
            ordered(&node.children, style, styles)
                .map(NodeId::from)
                .collect()
        })
        .collect();
    children.push(
        template
            .roots
            .iter()
            .map(|&r| NodeId::from(r as usize))
            .collect(),
    );
    let mut tree = Tree {
        template,
        styles,
        canvas: taffy::Style {
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
        },
        children,
        strings: &strings,
        text,
        caches: vec![Cache::new(); n + 1],
        layouts: vec![taffy::Layout::with_order(0); n + 1],
        hoisted: vec![Vec::new(); n + 1],
        failed: None,
    };
    let available = Size {
        width: AvailableSpace::Definite(canvas[0]),
        height: AvailableSpace::Definite(canvas[1]),
    };
    compute_root_layout(&mut tree, NodeId::from(n), available);
    let Tree {
        children,
        layouts,
        hoisted,
        failed,
        ..
    } = tree;
    if let Some(e) = failed {
        return Err(e);
    }

    // Each container places its in-flow children and the out-of-flow boxes it is the containing
    // block of: their locations are from its border box. Containers come in preorder, the canvas
    // first, so a box's container is placed before it.
    let mut origins: Vec<Option<[f32; 2]>> = vec![None; n + 1];
    origins[n] = Some([0.0, 0.0]);
    let shown = |i: usize| styles[i].layout.display != Display::None;
    let in_flow = |i: &usize| !styles[*i].layout.position.is_out_of_flow();
    for id in std::iter::once(n).chain(0..n) {
        let Some(at) = origins[id] else {
            continue;
        };
        let flow = children[id].iter().map(|&c| usize::from(c)).filter(in_flow);
        for c in flow.chain(hoisted[id].iter().map(|&h| usize::from(h))) {
            if shown(c) {
                let l = layouts[c].location;
                origins[c] = Some([at[0] + l.x, at[1] + l.y]);
            }
        }
    }

    let mut boxes = vec![Boxes::default(); n];
    let mut lines = vec![None; n];
    for (i, origin) in origins[..n].iter().enumerate() {
        let Some([x, y]) = *origin else {
            continue;
        };
        let l = &layouts[i];
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

/// The frame as Taffy lays it out: a node's id is its template index, and the canvas's is the
/// number of nodes.
struct Tree<'a> {
    template: &'a Template,
    styles: &'a [Style],
    canvas: taffy::Style,
    /// Each node's children in the order it lays them out, the canvas's last.
    children: Vec<Vec<NodeId>>,
    /// Each text node's text.
    strings: &'a [Option<Cow<'a, str>>],
    text: &'a mut Text,
    caches: Vec<Cache>,
    layouts: Vec<taffy::Layout>,
    /// The out-of-flow boxes each node is the containing block of.
    hoisted: Vec<Vec<NodeId>>,
    /// The first error measuring text met.
    failed: Option<Error>,
}

impl Tree<'_> {
    fn style(&self, id: NodeId) -> &taffy::Style {
        match self.styles.get(usize::from(id)) {
            Some(s) => &s.layout,
            None => &self.canvas,
        }
    }

    fn compute(
        &mut self,
        id: NodeId,
        inputs: LayoutInput,
        block: Option<&mut BlockContext<'_>>,
    ) -> LayoutOutput {
        // Under `display: none`, whatever the node's own display.
        if inputs.run_mode == RunMode::PerformHiddenLayout {
            return compute_hidden_layout(self, id);
        }
        compute_cached_layout(self, id, inputs, |tree, id, inputs| {
            let parent = !tree.children[usize::from(id)].is_empty();
            let mut out = match (tree.style(id).display, parent) {
                (Display::None, _) => compute_hidden_layout(tree, id),
                (Display::Block, true) => compute_block_layout(tree, id, inputs, block),
                (Display::Flex, true) => compute_flexbox_layout(tree, id, inputs),
                _ => tree.leaf(usize::from(id), inputs),
            };
            // The out-of-flow boxes it contains, once their static positions are known.
            if inputs.run_mode == RunMode::PerformLayout {
                compute_oof_layout(tree, id, &mut out);
            }
            out
        })
    }

    /// A box without children: text measured as Taffy asks and aligned on its first baseline, or
    /// an empty box.
    fn leaf(&mut self, i: usize, inputs: LayoutInput) -> LayoutOutput {
        let (styles, strings) = (self.styles, self.strings);
        let (Some(style), Some(Some(s))) = (styles.get(i), strings.get(i)) else {
            let style = self.style(NodeId::from(i));
            return compute_leaf_layout(inputs, style, |_, _| 0.0, |_, _| Size::ZERO);
        };
        let (layout, font) = (&style.layout, &style.font);
        let Tree {
            template,
            text,
            failed,
            ..
        } = self;
        let mut fail = |e: Error| {
            failed.get_or_insert_with(|| format!("{}: {e}", template.nodes[i].place()));
        };
        let mut out = compute_leaf_layout(
            inputs,
            layout,
            |_, _| 0.0,
            |known, available| {
                let width = known.width.or(match available.width {
                    AvailableSpace::Definite(w) => Some(w),
                    AvailableSpace::MinContent => Some(0.0),
                    AvailableSpace::MaxContent => None,
                });
                match text.measure(s, font, width) {
                    Ok([w, h]) => Size {
                        width: known.width.unwrap_or(w),
                        height: known.height.unwrap_or(h),
                    },
                    Err(e) => {
                        fail(e);
                        Size::ZERO
                    }
                }
            },
        );
        // Flex items align on their first baselines, from the top of the border box; text has
        // one where its first line has, and empty text none.
        if inputs.run_mode == RunMode::PerformLayout {
            let side =
                |l: LengthPercentage| l.resolve_or_zero(inputs.parent_size.width, |_, _| 0.0);
            let (pad, border) = (layout.padding, layout.border);
            let inset = side(pad.left) + side(pad.right) + side(border.left) + side(border.right);
            match text.baseline(s, font, out.size.width - inset) {
                Ok(b) => {
                    let top = side(pad.top) + side(border.top);
                    out.baselines.first = b.map(|b| top + b);
                }
                Err(e) => fail(e),
            }
        }
        out
    }
}

impl TraversePartialTree for Tree<'_> {
    type ChildIter<'a>
        = std::iter::Copied<std::slice::Iter<'a, NodeId>>
    where
        Self: 'a;

    fn child_ids(&self, id: NodeId) -> Self::ChildIter<'_> {
        self.children[usize::from(id)].iter().copied()
    }

    fn child_count(&self, id: NodeId) -> usize {
        self.children[usize::from(id)].len()
    }

    fn get_child_id(&self, id: NodeId, index: usize) -> NodeId {
        self.children[usize::from(id)][index]
    }
}

impl LayoutPartialTree for Tree<'_> {
    type CoreContainerStyle<'a>
        = &'a taffy::Style
    where
        Self: 'a;
    type CustomIdent = String;

    fn get_core_container_style(&self, id: NodeId) -> &taffy::Style {
        self.style(id)
    }

    fn set_unrounded_layout(&mut self, id: NodeId, layout: &taffy::Layout) {
        self.layouts[usize::from(id)] = *layout;
    }

    fn compute_child_layout(&mut self, id: NodeId, inputs: LayoutInput) -> LayoutOutput {
        self.compute(id, inputs, None)
    }
}

impl LayoutContainingBlock for Tree<'_> {
    type OofItemStyle<'a>
        = &'a taffy::Style
    where
        Self: 'a;

    fn get_oof_item_style(&self, id: NodeId) -> &taffy::Style {
        self.style(id)
    }

    fn clear_hoisted_children(&mut self, id: NodeId) {
        self.hoisted[usize::from(id)].clear();
    }

    fn add_hoisted_children(&mut self, id: NodeId, hoisted: &[NodeId]) {
        self.hoisted[usize::from(id)].extend_from_slice(hoisted);
    }
}

impl CacheTree for Tree<'_> {
    fn cache_get(&mut self, id: NodeId, input: &LayoutInput) -> Option<LayoutOutput> {
        self.caches[usize::from(id)].get(input)
    }

    fn cache_store(&mut self, id: NodeId, input: &LayoutInput, output: LayoutOutput) {
        self.caches[usize::from(id)].store(input, output);
    }

    fn cache_clear(&mut self, id: NodeId) {
        self.caches[usize::from(id)].clear();
    }
}

impl LayoutBlockContainer for Tree<'_> {
    type BlockContainerStyle<'a>
        = &'a taffy::Style
    where
        Self: 'a;
    type BlockItemStyle<'a>
        = &'a taffy::Style
    where
        Self: 'a;

    fn get_block_container_style(&self, id: NodeId) -> &taffy::Style {
        self.style(id)
    }

    fn get_block_child_style(&self, id: NodeId) -> &taffy::Style {
        self.style(id)
    }

    fn compute_block_child_layout(
        &mut self,
        id: NodeId,
        inputs: LayoutInput,
        block: Option<&mut BlockContext<'_>>,
    ) -> LayoutOutput {
        self.compute(id, inputs, block)
    }
}

impl LayoutFlexboxContainer for Tree<'_> {
    type FlexboxContainerStyle<'a>
        = &'a taffy::Style
    where
        Self: 'a;
    type FlexboxItemStyle<'a>
        = &'a taffy::Style
    where
        Self: 'a;

    fn get_flexbox_container_style(&self, id: NodeId) -> &taffy::Style {
        self.style(id)
    }

    fn get_flexbox_child_style(&self, id: NodeId) -> &taffy::Style {
        self.style(id)
    }
}

#[cfg(test)]
mod tests;
