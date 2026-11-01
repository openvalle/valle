//! Layout: Taffy's algorithms run over a frame's nodes under the canvas, on a tree that borrows
//! their computed styles; text measured as Taffy asks and aligned on its first baseline; and
//! every node's boxes in canvas px. The canvas is a block container the size of the composition
//! that clips what overflows it, and the containing block of fixed boxes and of what no
//! positioned ancestor contains. A box with a transform or a perspective contains the absolute
//! and fixed boxes in it, as CSS has it. A mask's source is laid out after the mask, as the canvas
//! is but in the mask's border box.

use std::borrow::Cow;

use taffy::{
    AvailableSpace, BlockContext, BoxGenerationMode, BoxSizing, Cache, CacheTree, Contain,
    ContainingBlockClaims, CoreStyle, DetailedGridInfo, DetailedLayoutInfo, Dimension, Direction,
    Display, LayoutBlockContainer, LayoutContainingBlock, LayoutFlexboxContainer,
    LayoutGridContainer, LayoutInput, LayoutOutput, LayoutPartialTree, LengthPercentage,
    LengthPercentageAuto, NodeId, Overflow, Point, Position, Rect as Edges, ResolveOrZero, RunMode,
    Size, TraversePartialTree, compute_block_layout, compute_cached_layout, compute_flexbox_layout,
    compute_grid_layout, compute_hidden_layout, compute_leaf_layout, compute_oof_layout,
    compute_root_layout,
};

use crate::style::Style;
use crate::style::math::{self, Calc};
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

/// A node's border box, padding box and content box.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Boxes {
    pub border: Rect,
    pub padding: Rect,
    pub content: Rect,
}

/// A frame's layout, in template order. Nodes in a `display: none` subtree have empty boxes.
#[derive(Debug)]
pub struct Layout {
    pub boxes: Vec<Boxes>,
    /// Each shown text node's lines, from the top left of its content box.
    pub lines: Vec<Option<Lines>>,
}

/// A container's children in the order it lays them out and paints them: a flex or grid
/// container's by `order` (out-of-flow ones as 0), then as written; any other's as written.
pub fn ordered<'a>(
    children: &'a [u32],
    container: &Style,
    styles: &'a [Style],
) -> impl Iterator<Item = usize> + 'a {
    let mut list: Vec<usize> = children.iter().map(|&c| c as usize).collect();
    if matches!(container.layout.display, Display::Flex | Display::Grid) {
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
            Kind::View | Kind::Mask { .. } => None,
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
    // A root's style: a block container of this size.
    let root = |[width, height]: [f32; 2]| taffy::Style {
        display: Display::Block,
        position: Position::Relative,
        overflow: Point {
            x: Overflow::Hidden,
            y: Overflow::Hidden,
        },
        size: Size {
            width: Dimension::length(width),
            height: Dimension::length(height),
        },
        ..taffy::Style::DEFAULT
    };
    let mut roots = vec![None; n + 1];
    roots[n] = Some(root(canvas));
    let mut tree = Tree {
        template,
        styles,
        calcs: styles.iter().flat_map(|s| &s.calcs).collect(),
        roots,
        children,
        strings: &strings,
        text,
        caches: vec![Cache::new(); n + 1],
        layouts: vec![taffy::Layout::with_order(0); n + 1],
        hoisted: vec![Vec::new(); n + 1],
        grids: vec![DetailedLayoutInfo::None; n + 1],
        failed: None,
    };
    let available = |[width, height]: [f32; 2]| Size {
        width: AvailableSpace::Definite(width),
        height: AvailableSpace::Definite(height),
    };
    compute_root_layout(&mut tree, NodeId::from(n), available(canvas));
    // Each source once its mask is laid out: masks come before their sources, so in order.
    for (m, style) in styles.iter().enumerate() {
        if let Some(source) = style.source {
            let size = tree.layouts[m].size;
            let size = [size.width, size.height];
            tree.roots[source.root] = Some(root(size));
            compute_root_layout(&mut tree, NodeId::from(source.root), available(size));
        }
    }
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
    // block of: their locations are from its border box; a mask places its source on itself.
    // Containers come in preorder, the canvas first, so a box's container is placed before it.
    let mut origins: Vec<Option<[f32; 2]>> = vec![None; n + 1];
    origins[n] = Some([0.0, 0.0]);
    let shown = |i: usize| styles[i].layout.display != Display::None;
    let in_flow = |i: &usize| !styles[*i].layout.position.is_out_of_flow();
    for id in std::iter::once(n).chain(0..n) {
        let Some(at) = origins[id] else {
            continue;
        };
        if let Some(source) = styles.get(id).and_then(|s| s.source) {
            origins[source.root] = Some(at);
        }
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
        let border = Rect {
            x,
            y,
            width: l.size.width,
            height: l.size.height,
        };
        let padding = inside(border, l.border);
        let content = inside(padding, l.padding);
        boxes[i] = Boxes {
            border,
            padding,
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

/// `r` less `edges`.
fn inside(r: Rect, edges: taffy::Rect<f32>) -> Rect {
    Rect {
        x: r.x + edges.left,
        y: r.y + edges.top,
        width: r.width - edges.left - edges.right,
        height: r.height - edges.top - edges.bottom,
    }
}

/// The frame as Taffy lays it out: a node's id is its template index, and the canvas's is the
/// number of nodes.
struct Tree<'a> {
    template: &'a Template,
    styles: &'a [Style],
    /// The styles' calcs, which handles number in order.
    calcs: Vec<&'a Calc>,
    /// The style of each root, the canvas and the masks' sources, which takes the place of its
    /// own.
    roots: Vec<Option<taffy::Style>>,
    /// Each node's children in the order it lays them out, the canvas's last.
    children: Vec<Vec<NodeId>>,
    /// Each text node's text.
    strings: &'a [Option<Cow<'a, str>>],
    text: &'a mut Text,
    caches: Vec<Cache>,
    layouts: Vec<taffy::Layout>,
    /// The out-of-flow boxes each node is the containing block of.
    hoisted: Vec<Vec<NodeId>>,
    /// Each grid's tracks and lines, which place the out-of-flow boxes it contains.
    grids: Vec<DetailedLayoutInfo<String>>,
    /// The first error measuring text met.
    failed: Option<Error>,
}

impl Tree<'_> {
    fn style(&self, id: NodeId) -> &taffy::Style {
        let i = usize::from(id);
        match &self.roots[i] {
            Some(root) => root,
            None => &self.styles[i].layout,
        }
    }

    fn calc(&self, handle: *const (), basis: f32) -> f32 {
        self.calcs[math::index(handle)].resolve(basis)
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
            let i = usize::from(id);
            let parent = !tree.children[i].is_empty();
            let text = tree.strings.get(i).is_some_and(Option::is_some);
            let mut out = match (tree.style(id).display, parent) {
                (Display::None, _) => compute_hidden_layout(tree, id),
                (Display::Block, true) => compute_block_layout(tree, id, inputs, block),
                (Display::Flex, true) => compute_flexbox_layout(tree, id, inputs),
                // A grid's tracks size it even without items.
                (Display::Grid, _) if !text => compute_grid_layout(tree, id, inputs),
                _ => tree.leaf(i, inputs),
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
            let calc = |h, basis| self.calc(h, basis);
            return compute_leaf_layout(inputs, style, calc, |_, _| Size::ZERO);
        };
        let (layout, font) = (&style.layout, &style.font);
        let Tree {
            template,
            calcs,
            text,
            failed,
            ..
        } = self;
        let calc = |h, basis| calcs[math::index(h)].resolve(basis);
        let mut fail = |e: Error| {
            failed.get_or_insert_with(|| format!("{}: {e}", template.nodes[i].place()));
        };
        let mut out = compute_leaf_layout(inputs, layout, calc, |known, available| {
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
        });
        // Flex and grid items align on their first baselines, from the top of the border box; text has
        // one where its first line has, and empty text none.
        if inputs.run_mode == RunMode::PerformLayout {
            let side = |l: LengthPercentage| l.resolve_or_zero(inputs.parent_size.width, calc);
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
        = Container<'a>
    where
        Self: 'a;
    type CustomIdent = String;

    fn get_core_container_style(&self, id: NodeId) -> Container<'_> {
        let all = (self.styles.get(usize::from(id))).is_some_and(Style::contains_all);
        Container {
            style: self.style(id),
            all,
        }
    }

    fn set_unrounded_layout(&mut self, id: NodeId, layout: &taffy::Layout) {
        self.layouts[usize::from(id)] = *layout;
    }

    fn resolve_calc_value(&self, handle: *const (), basis: f32) -> f32 {
        self.calc(handle, basis)
    }

    fn compute_child_layout(&mut self, id: NodeId, inputs: LayoutInput) -> LayoutOutput {
        self.compute(id, inputs, None)
    }
}

/// A container's style as the boxes it lays out see it: one with a transform or a perspective,
/// `all`, is the containing block of every absolute and fixed box in it.
struct Container<'a> {
    style: &'a taffy::Style,
    all: bool,
}

impl CoreStyle for Container<'_> {
    type CustomIdent = String;

    fn is_containing_block(&self) -> ContainingBlockClaims {
        match self.all {
            true => ContainingBlockClaims::ALL,
            false => self.style.is_containing_block(),
        }
    }

    fn box_generation_mode(&self) -> BoxGenerationMode {
        self.style.box_generation_mode()
    }

    fn is_block(&self) -> bool {
        self.style.is_block()
    }

    fn is_compressible_replaced(&self) -> bool {
        self.style.is_compressible_replaced()
    }

    fn is_replaced(&self) -> bool {
        self.style.is_replaced()
    }

    fn box_sizing(&self) -> BoxSizing {
        self.style.box_sizing()
    }

    fn direction(&self) -> Direction {
        self.style.direction()
    }

    fn overflow(&self) -> Point<Overflow> {
        self.style.overflow()
    }

    fn scrollbar_width(&self) -> f32 {
        self.style.scrollbar_width()
    }

    fn position(&self) -> Position {
        self.style.position()
    }

    fn inset(&self) -> Edges<LengthPercentageAuto> {
        self.style.inset()
    }

    fn size(&self) -> Size<Dimension> {
        self.style.size()
    }

    fn min_size(&self) -> Size<LengthPercentageAuto> {
        self.style.min_size()
    }

    fn max_size(&self) -> Size<LengthPercentageAuto> {
        self.style.max_size()
    }

    fn aspect_ratio(&self) -> Option<f32> {
        self.style.aspect_ratio()
    }

    fn margin(&self) -> Edges<LengthPercentageAuto> {
        self.style.margin()
    }

    fn padding(&self) -> Edges<LengthPercentage> {
        self.style.padding()
    }

    fn border(&self) -> Edges<LengthPercentage> {
        self.style.border()
    }

    fn contain(&self) -> Contain {
        self.style.contain()
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

    fn get_detailed_layout_info(&self, id: NodeId) -> &DetailedLayoutInfo<String> {
        &self.grids[usize::from(id)]
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

impl LayoutGridContainer for Tree<'_> {
    type GridContainerStyle<'a>
        = &'a taffy::Style
    where
        Self: 'a;
    type GridItemStyle<'a>
        = &'a taffy::Style
    where
        Self: 'a;

    fn get_grid_container_style(&self, id: NodeId) -> &taffy::Style {
        self.style(id)
    }

    fn get_grid_child_style(&self, id: NodeId) -> &taffy::Style {
        self.style(id)
    }

    fn set_detailed_grid_info(&mut self, id: NodeId, info: DetailedGridInfo<String>) {
        self.grids[usize::from(id)] = DetailedLayoutInfo::Grid(Box::new(info));
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
