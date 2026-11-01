//! Layout: Taffy's algorithms run over a frame's nodes under the canvas, on a tree that borrows
//! their computed styles; text measured as Taffy asks and aligned on its first baseline; and
//! every node's boxes in canvas px. The canvas is a block container the size of the composition
//! that clips what overflows it, and the containing block of fixed boxes and of what no
//! positioned ancestor contains. A box with a transform or a perspective contains the absolute
//! and fixed boxes in it, as CSS has it. A mask's source is laid out after the mask, as the canvas
//! is but in the mask's border box.

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
use crate::template::{self, Error, Kind, Prop, Template, V};
use crate::text::{Content, Lines, Piece, Stripe, Text};

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

/// A frame's layout, in template order. Nodes in a `display: none` subtree have empty boxes. A
/// Span's boxes are all the border box of its text, as Chrome measures an inline box's.
#[derive(Debug)]
pub struct Layout {
    pub boxes: Vec<Boxes>,
    /// Each shown Text's lines, from the top left of its content box.
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
    let contents: Vec<Option<Content>> = (template.nodes.iter().enumerate())
        .map(|(i, node)| match node.kind {
            Kind::Text { .. } => {
                let mut pieces = Vec::new();
                pieces_of(template, values, styles, i, &mut pieces);
                let font = &styles[i].font;
                Some(Content { font, pieces })
            }
            _ => None,
        })
        .collect();
    let mut children: Vec<Vec<NodeId>> = (template.nodes.iter().zip(styles))
        .map(|(node, style)| match node.kind {
            // Its Spans are in its text, not boxes.
            Kind::Text { .. } => Vec::new(),
            _ => (ordered(&node.children, style, styles))
                .map(NodeId::from)
                .collect(),
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
        contents: &contents,
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

    // The Text or Span each Span is in.
    let mut parents = vec![None; n];
    for (i, node) in template.nodes.iter().enumerate() {
        if let Kind::Text { content } | Kind::Span { content } = &node.kind {
            for piece in content {
                if let template::Piece::Span(s) = piece {
                    parents[*s as usize] = Some(i);
                }
            }
        }
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
        if let Some(c) = &contents[i] {
            let place = |e| format!("{}: {e}", template.nodes[i].place());
            let mut laid = text.lines(c, content.width).map_err(place)?;
            let at = [content.x, content.y];
            laid.decorations = decorations(&laid, at, i, template, styles, text).map_err(place)?;
            spans(
                &laid,
                [content.x, content.y],
                styles,
                &parents,
                text,
                &mut boxes,
            )
            .map_err(place)?;
            lines[i] = Some(laid);
        }
    }
    Ok(Layout { boxes, lines })
}

/// The pieces of text of node `i`, a Text or a Span, and of the Spans in it, in order.
fn pieces_of<'a>(
    template: &'a Template,
    values: &'a [V],
    styles: &'a [Style],
    i: usize,
    out: &mut Vec<Piece<'a>>,
) {
    let (Kind::Text { content } | Kind::Span { content }) = &template.nodes[i].kind else {
        return;
    };
    for piece in content {
        match piece {
            template::Piece::Text(prop) => out.push(Piece {
                text: match prop {
                    Prop::Const(v) => v.text(),
                    Prop::Expr(e) => values[*e as usize].text(),
                },
                node: i as u32,
                font: &styles[i].font,
            }),
            template::Piece::Span(s) => pieces_of(template, values, styles, *s as usize, out),
        }
    }
}

/// The boxes of the Spans in a Text laid out in `lines` from `at`, as Chrome has an inline box's:
/// on each line, from the left of its first run of glyphs to the right of its last, and from its
/// font's ascent above the baseline to its descent below, rounded; its lines together.
fn spans(
    lines: &Lines,
    at: [f32; 2],
    styles: &[Style],
    parents: &[Option<usize>],
    text: &mut Text,
    boxes: &mut [Boxes],
) -> Result<(), Error> {
    let mut extents: Vec<(usize, [f32; 4])> = Vec::new();
    for run in &lines.runs {
        let baseline = at[1] + lines.baselines[run.line as usize];
        // The run's own Span and those around it, up to the Text.
        let mut span = Some(run.node as usize);
        while let Some(s) = span.filter(|&s| parents[s].is_some()) {
            let [ascent, descent, _] = text.metrics(&styles[s].font)?;
            let r = [
                at[0] + run.x[0],
                baseline - ascent.round(),
                at[0] + run.x[1],
                baseline + descent.round(),
            ];
            match extents.iter_mut().find(|(e, _)| *e == s) {
                Some((_, e)) => {
                    *e = [
                        e[0].min(r[0]),
                        e[1].min(r[1]),
                        e[2].max(r[2]),
                        e[3].max(r[3]),
                    ]
                }
                None => extents.push((s, r)),
            }
            span = parents[s];
        }
    }
    for (s, [x0, y0, x1, y1]) in extents {
        let r = Rect {
            x: x0,
            y: y0,
            width: x1 - x0,
            height: y1 - y0,
        };
        boxes[s] = Boxes {
            border: r,
            padding: r,
            content: r,
        };
    }
    Ok(())
}

/// The text decorations of Text `i` laid out in `lines` from `at`, as Chrome draws them: on each
/// run, those its node takes, in the color, thickness and underline offset of the box that
/// declares each, as the font of its decorating box has them, the Span that declares it or else
/// the Text. A line is a tenth of that font size thick, at least 1 px, or as thick as set, rounded
/// to whole px; it is drawn whole px thick, at least 1, its top on a whole px of the canvas. An
/// underline is under the baseline by its offset, or by half its thickness rounded up to whole
/// px, at least 1; an overline is on the run's ascent; a line-through is centered two thirds of
/// the decorating box's ascent under the run's ascent. Underlines and overlines leave gaps where
/// the run's glyphs have ink across them but for their outer half px, wider by their thickness on
/// each side, up to 13 px, with edges rounded to whole px.
fn decorations(
    lines: &Lines,
    at: [f32; 2],
    i: usize,
    template: &Template,
    styles: &[Style],
    text: &mut Text,
) -> Result<Vec<Stripe>, Error> {
    let mut out = Vec::new();
    for (k, run) in lines.runs.iter().enumerate() {
        let style = &styles[run.node as usize];
        if style.decorations.is_empty() {
            continue;
        }
        let baseline = at[1] + lines.baselines[run.line as usize];
        let ascent = text.metrics(&style.font)?[0].round();
        let glyphs = &lines.glyphs[run.glyphs[0] as usize..run.glyphs[1] as usize];
        for d in &style.decorations {
            let font = match template.nodes[d.by].kind {
                Kind::Span { .. } => &styles[d.by].font,
                _ => &styles[i].font,
            };
            let size = font.size as f32;
            let thickness = match &d.thickness {
                None => (size / 10.0).max(1.0),
                Some(t) => t.resolve(size).round().max(1.0),
            };
            let height = thickness.trunc().max(1.0);
            let mut tops = Vec::new();
            if d.lines[0] {
                let offset = match &d.offset {
                    None => (thickness / 2.0).ceil().max(1.0),
                    Some(o) => o.resolve(size),
                };
                tops.push((baseline + offset, false));
            }
            if d.lines[1] {
                tops.push((baseline - ascent - height, false));
            }
            if d.lines[2] {
                let decorating = text.metrics(font)?[0].round();
                let top = baseline - ascent + decorating * 2.0 / 3.0 - thickness / 2.0;
                tops.push((top, true));
            }
            for (top, through) in tops {
                let y = top.round();
                let mut gaps = Vec::new();
                if !through && style.font.skip_ink {
                    let dilation = thickness.min(13.0);
                    for g in glyphs {
                        let below = top - at[1] - g.y;
                        let band = [below + 0.5, below + thickness - 0.5];
                        let size = style.font.size as f32;
                        if let Some([x0, x1]) = text.ink(g.face, g.id, size, band)? {
                            let x = at[0] + g.x;
                            gaps.push([(x + x0 - dilation).round(), (x + x1 + dilation).round()]);
                        }
                    }
                    gaps.sort_by(|a, b| a[0].total_cmp(&b[0]));
                }
                let mut x = at[0] + run.x[0];
                let end = at[0] + run.x[1];
                for [g0, g1] in gaps.into_iter().chain([[end, end]]) {
                    if g0.min(end) > x {
                        out.push(Stripe {
                            rect: [
                                x - at[0],
                                y - at[1],
                                g0.min(end) - at[0],
                                y + height - at[1],
                            ],
                            color: d.color,
                            run: k as u32,
                            through,
                        });
                    }
                    x = x.max(g1);
                }
            }
        }
    }
    Ok(out)
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
    /// Each Text's paragraph.
    contents: &'a [Option<Content<'a>>],
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
            let text = tree.contents.get(i).is_some_and(Option::is_some);
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
        let (styles, contents) = (self.styles, self.contents);
        let (Some(style), Some(Some(c))) = (styles.get(i), contents.get(i)) else {
            let style = self.style(NodeId::from(i));
            let calc = |h, basis| self.calc(h, basis);
            return compute_leaf_layout(inputs, style, calc, |_, _| Size::ZERO);
        };
        let layout = &style.layout;
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
            let measured = match (known.width, available.width) {
                (Some(w), _) | (None, AvailableSpace::Definite(w)) => text.measure(c, Some(w)),
                (None, AvailableSpace::MinContent) => text.min_content(c),
                (None, AvailableSpace::MaxContent) => text.measure(c, None),
            };
            match measured {
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
            match text.baseline(c, out.size.width - inset) {
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
