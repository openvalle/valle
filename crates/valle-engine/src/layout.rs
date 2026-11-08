//! Layout: Taffy's algorithms run over a frame's nodes under the canvas, on a tree that borrows
//! their computed styles; text measured as Taffy asks and aligned on its first baseline; and
//! every node's boxes in canvas px. The canvas is a block container the size of the composition
//! that clips what overflows it, and the containing block of fixed boxes and of what no
//! positioned ancestor contains. A box with a transform or a perspective contains the absolute
//! and fixed boxes in it, as CSS has it. A mask's source is laid out after the mask, as the canvas
//! is but in the mask's border box.

use std::collections::HashMap;

use taffy::{
    AvailableSpace, BlockContext, BoxGenerationMode, BoxSizing, Cache, CacheTree, Contain,
    ContainingBlockClaims, CoreStyle, DetailedGridInfo, DetailedLayoutInfo, Dimension, Direction,
    Display, LayoutBlockContainer, LayoutContainingBlock, LayoutFlexboxContainer,
    LayoutGridContainer, LayoutInput, LayoutOutput, LayoutPartialTree, LengthPercentage,
    LengthPercentageAuto, MaybeResolve, NodeId, Overflow, Point, Position, Rect as Edges,
    ResolveOrZero, RunMode, Size, TraversePartialTree, compute_block_layout, compute_cached_layout,
    compute_flexbox_layout, compute_grid_layout, compute_hidden_layout, compute_leaf_layout,
    compute_oof_layout, compute_root_layout,
};

use crate::image::Picture;
use crate::style::math::{self, Calc};
use crate::style::{DecorationStyle, SkipInk, Style};
use crate::svg::Element;
use crate::template::{self, Error, Kind, Prop, Tag, Template, V};
use crate::text::{Content, Lines, Piece, Stripe, Text, cjk_ranges, wave};

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
    /// The canvas's size: the roots' containing block.
    pub canvas: [f32; 2],
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
/// on a canvas of `canvas` px, its Images showing the resources' `pictures`.
pub fn layout(
    template: &Template,
    values: &[V],
    styles: &[Style],
    canvas: [f32; 2],
    text: &mut Text,
    pictures: &[Picture],
) -> Result<Layout, Error> {
    let n = template.nodes.len();
    let contents: Vec<Option<Content>> = (template.nodes.iter().enumerate())
        .map(|(i, node)| match node.kind {
            Kind::Text { .. } => {
                let mut pieces = Vec::new();
                pieces_of(template, values, styles, i, &mut pieces);
                let font = &styles[i].font;
                let clamp = styles[i].clamp.map(|n| n as usize);
                Some(Content {
                    font,
                    pieces,
                    clamp,
                })
            }
            _ => None,
        })
        .collect();
    let mut children: Vec<Vec<NodeId>> = (template.nodes.iter().zip(styles))
        .map(|(node, style)| match node.kind {
            // Its Spans are in its text, and an <svg>'s elements in its drawing, not boxes.
            Kind::Text { .. } | Kind::Svg { .. } => Vec::new(),
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
    // Images and <svg>s, boxes of replaced content.
    let mut replaced = vec![None; n];
    for (i, node) in template.nodes.iter().enumerate() {
        replaced[i] = match &node.kind {
            Kind::Image { resource, .. } => Some(pictures[*resource as usize].natural()),
            Kind::Svg { tag: Tag::Svg, .. } => Some(Natural::Ratio(
                styles[i].svg.as_deref().and_then(|c| match c.element {
                    Element::Svg {
                        view_box: Some([_, _, w, h]),
                        ..
                    } if w > 0.0 && h > 0.0 => Some((w / h) as f32),
                    _ => None,
                }),
            )),
            _ => None,
        };
        if replaced[i].is_some() {
            roots[i] = Some(taffy::Style {
                item_is_replaced: true,
                ..styles[i].layout.clone()
            });
        }
    }
    let mut tree = Tree {
        template,
        styles,
        calcs: styles.iter().flat_map(|s| &s.calcs).collect(),
        roots,
        replaced,
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
        if let Kind::Text { content } | Kind::Span { content, .. } = &node.kind {
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
            let style = &styles[i];
            let overflow = style.ellipsis && style.layout.overflow.x != Overflow::Visible;
            if overflow || c.clamp.is_some_and(|n| laid.boxes.len() > n) {
                let (font, node) = (&style.font, i as u32);
                (text.truncate(&mut laid, font, node, content.width, overflow, c.clamp))
                    .map_err(place)?;
            }
            lines[i] = Some(laid);
        }
    }
    Ok(Layout {
        boxes,
        lines,
        canvas,
    })
}

/// The pieces of text of node `i`, a Text or a Span, and of the Spans in it, in order.
fn pieces_of<'a>(
    template: &'a Template,
    values: &'a [V],
    styles: &'a [Style],
    i: usize,
    out: &mut Vec<Piece<'a>>,
) {
    let (Kind::Text { content } | Kind::Span { content, .. }) = &template.nodes[i].kind else {
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
/// font's ascent above the baseline to its descent below, rounded; its lines together. A run that
/// is only white space hanging at the end of its line has no part in a box, as in Chrome a Span
/// that starts with the space a line breaks at starts on the next line; unless the Span has
/// nothing else.
fn spans(
    lines: &Lines,
    at: [f32; 2],
    styles: &[Style],
    parents: &[Option<usize>],
    text: &mut Text,
    boxes: &mut [Boxes],
) -> Result<(), Error> {
    let mut extents: Vec<(usize, [f32; 4])> = Vec::new();
    let mut hanging: Vec<(usize, [f32; 4])> = Vec::new();
    for (i, run) in lines.runs.iter().enumerate() {
        let hangs = (lines.runs[i..].iter())
            .take_while(|r| r.line == run.line)
            .all(|r| r.x[1] <= r.x[0]);
        let extents = if hangs { &mut hanging } else { &mut extents };
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
    for (s, e) in hanging {
        if !extents.iter().any(|(t, _)| *t == s) {
            extents.push((s, e));
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
/// the decorating box's ascent under the run's ascent. A double line has a second line a px past
/// its thickness further out, under a line-through; a wavy one's wave is half a px under its top,
/// that much further out. Underlines and overlines leave gaps where the run's glyphs have ink
/// across them, all lines of them, but for their outer half px, wider by their thickness on each
/// side, up to 13 px, with edges rounded to whole px.
fn decorations(
    lines: &Lines,
    at: [f32; 2],
    i: usize,
    template: &Template,
    styles: &[Style],
    text: &mut Text,
) -> Result<Vec<Stripe>, Error> {
    let mut out = Vec::new();
    // A shaping/font boundary does not restart a decorating box's dots, dashes or waves.
    // Collect its full interval on each line once; glyph runs still clip their own pieces.
    let mut spans = HashMap::new();
    for run in &lines.runs {
        for d in &styles[run.node as usize].decorations {
            spans
                .entry((run.line, d.by))
                .and_modify(|x: &mut [f32; 2]| {
                    x[0] = x[0].min(run.x[0]);
                    x[1] = x[1].max(run.x[1]);
                })
                .or_insert(run.x);
        }
    }
    let cjk = if lines.runs.iter().any(|run| {
        let s = &styles[run.node as usize];
        s.font.skip_ink == SkipInk::Auto && !s.decorations.is_empty()
    }) {
        cjk_ranges(&lines.string)
    } else {
        Vec::new()
    };
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
            // Each line's top, and which way from it a second line or a wave goes.
            let mut tops = Vec::new();
            if d.lines[0] {
                let offset = match &d.offset {
                    None => (thickness / 2.0).ceil().max(1.0),
                    Some(o) => o.resolve(size),
                };
                tops.push((baseline + offset, 1.0));
            }
            if d.lines[1] {
                tops.push((baseline - ascent - height, -1.0));
            }
            if d.lines[2] {
                let decorating = text.metrics(font)?[0].round();
                let top = baseline - ascent + decorating * 2.0 / 3.0 - thickness / 2.0;
                tops.push((top, 0.0));
            }
            for (top, way) in tops {
                let (y, through) = (top.round(), way == 0.0);
                // The rows it is drawn in, and the band of them its gaps are for.
                let (rows, band) = match d.style {
                    DecorationStyle::Double => {
                        let apart = match through {
                            true => (thickness + 1.0).floor(),
                            false => way * (thickness + 1.0),
                        };
                        let second = (top + apart).round();
                        let rows = vec![[y, y + height], [second, second + height]];
                        let band = [top.min(top + apart), top.max(top + apart) + thickness];
                        (rows, band)
                    }
                    DecorationStyle::Wavy => {
                        let axis = y + 0.5 + way * (thickness + 1.0);
                        let [_, control] = wave(thickness);
                        let (reach, ink) = (control + thickness, control * 0.29 + thickness / 2.0);
                        (vec![[axis - reach, axis + reach]], [axis - ink, axis + ink])
                    }
                    // Stroked as thick as it is, its dots and dashes as wide as that in whole px,
                    // centered on a whole px, or a half one for an odd width, whose stroke's band
                    // its gaps are for.
                    DecorationStyle::Dotted | DecorationStyle::Dashed => {
                        let w = thickness.round().max(1.0);
                        let odd = if w % 2.0 == 1.0 { 0.5 } else { 0.0 };
                        let middle = (top + (thickness / 2.0).max(0.5)).floor() + odd;
                        let rows = vec![[middle - thickness / 2.0, middle + thickness / 2.0]];
                        (rows, [middle - w / 2.0, middle + w / 2.0])
                    }
                    _ => (vec![[y, y + height]], [top, top + thickness]),
                };
                let mut gaps = Vec::new();
                if !through && style.font.skip_ink != SkipInk::None {
                    let dilation = thickness.min(13.0);
                    for g in glyphs {
                        if style.font.skip_ink == SkipInk::Auto {
                            let at = g.cluster as usize;
                            let k = cjk.partition_point(|r| r.end <= at);
                            if cjk.get(k).is_some_and(|r| r.contains(&at)) {
                                continue;
                            }
                        }
                        let below = |y: f32| y - at[1] - g.y;
                        let band = [below(band[0]) + 0.5, below(band[1]) - 0.5];
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
                let style = match d.style {
                    DecorationStyle::Double => DecorationStyle::Solid,
                    style => style,
                };
                for [g0, g1] in gaps.into_iter().chain([[end, end]]) {
                    if g0.min(end) > x {
                        out.extend(rows.iter().map(|&[y0, y1]| Stripe {
                            rect: [x - at[0], y0 - at[1], g0.min(end) - at[0], y1 - at[1]],
                            along: spans[&(run.line, d.by)],
                            style,
                            thickness,
                            color: d.color,
                            run: k as u32,
                            through,
                        }));
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
    /// The style that takes the place of a node's own: each root's, the canvas's and the masks'
    /// sources', and each image's, as replaced content.
    roots: Vec<Option<taffy::Style>>,
    /// What each image or `<svg>` sizes its box by.
    replaced: Vec<Option<Natural>>,
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
            let image = tree.replaced.get(i).is_some_and(Option::is_some);
            let mut out = match (tree.style(id).display, parent) {
                (Display::None, _) => compute_hidden_layout(tree, id),
                (Display::Block, true) => compute_block_layout(tree, id, inputs, block),
                (Display::Flex, true) => compute_flexbox_layout(tree, id, inputs),
                // A grid's tracks size it even without items.
                (Display::Grid, _) if !text && !image => compute_grid_layout(tree, id, inputs),
                _ => tree.leaf(i, inputs),
            };
            // The out-of-flow boxes it contains, once their static positions are known.
            if inputs.run_mode == RunMode::PerformLayout {
                compute_oof_layout(tree, id, &mut out);
            }
            out
        })
    }

    /// A box without children: text measured as Taffy asks and aligned on its first baseline, an
    /// image, or an empty box.
    fn leaf(&mut self, i: usize, inputs: LayoutInput) -> LayoutOutput {
        if let Some(r) = self.replaced.get(i).copied().flatten() {
            return self.replaced(i, inputs, r);
        }
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

impl Tree<'_> {
    /// An image's or an `<svg>`'s box: its content box the size given on an axis, by its
    /// container or its style, and on the other as its natural ratio has it, or its natural size.
    /// One with a ratio but no size takes the width there is, and without a ratio 300 × 150 px.
    fn replaced(&self, i: usize, inputs: LayoutInput, r: Natural) -> LayoutOutput {
        let style = self.style(NodeId::from(i));
        let calc = |handle, basis| self.calc(handle, basis);
        let width = inputs.parent_size.width;
        let side = |l: LengthPercentage| l.resolve_or_zero(width, calc);
        let (p, b) = (style.padding, style.border);
        let inset = [
            side(p.left) + side(p.right) + side(b.left) + side(b.right),
            side(p.top) + side(p.bottom) + side(b.top) + side(b.bottom),
        ];
        let sized = style.size.maybe_resolve(inputs.parent_size, calc);
        let border_box = style.box_sizing == BoxSizing::BorderBox;
        compute_leaf_layout(inputs, style, calc, |known, available| {
            let given = |known: Option<f32>, sized: Option<f32>, inset: f32| {
                let content = match known {
                    Some(k) => Some(k - inset),
                    None => sized.map(|s| if border_box { s - inset } else { s }),
                };
                content.map(|c| c.max(0.0))
            };
            let given = [
                given(known.width, sized.width, inset[0]),
                given(known.height, sized.height, inset[1]),
            ];
            let [w, h] = match r {
                Natural::Size(natural) => natural,
                Natural::Ratio(Some(ratio)) => {
                    let w = match available.width {
                        AvailableSpace::Definite(w) => (w - inset[0]).max(0.0),
                        _ => 300.0,
                    };
                    [w, w / ratio]
                }
                Natural::Ratio(None) => [300.0, 150.0],
            };
            let ratio = match r {
                Natural::Ratio(None) => false,
                _ => w > 0.0 && h > 0.0,
            };
            let [width, height] = match given {
                [Some(x), Some(y)] => [x, y],
                [Some(x), None] if ratio => [x, x * h / w],
                [None, Some(y)] if ratio => [y * w / h, y],
                [x, y] => [x.unwrap_or(w), y.unwrap_or(h)],
            };
            Size { width, height }
        })
    }
}

/// What sizes a box of replaced content: its natural size, or without one its natural ratio, if
/// it has one, as an `<svg>` has its viewBox's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Natural {
    Size([f32; 2]),
    Ratio(Option<f32>),
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
mod tests {
    use super::*;
    use crate::style::Sheet;
    use crate::template::Node;
    use crate::testing::{mask, text, text_node, view};

    /// Laid out on a 200 × 100 canvas.
    fn lay(nodes: Vec<Node>, roots: &[u32]) -> Result<Layout, Error> {
        let template = Template {
            nodes,
            roots: roots.to_vec(),
            ..Default::default()
        };
        let styles = Sheet::new(&template)?.compute(&[], true, [200.0, 100.0], &[])?;
        layout(&template, &[], &styles, [200.0, 100.0], &mut text(), &[])
    }

    fn rect(x: f32, y: f32, width: f32, height: f32) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn blocks_stack_and_fill_their_container() {
        let l = lay(
            vec![
                view(&[("height", "20px"), ("marginTop", "5px")], &[]),
                view(
                    &[("width", "50%"), ("height", "30px"), ("marginLeft", "10px")],
                    &[],
                ),
            ],
            &[0, 1],
        )
        .unwrap();
        // The canvas clips, so its first child's margin stays inside it.
        assert_eq!(l.boxes[0].border, rect(0.0, 5.0, 200.0, 20.0));
        assert_eq!(l.boxes[1].border, rect(10.0, 25.0, 100.0, 30.0));
    }

    #[test]
    fn flex_rows_place_and_grow() {
        let row = [
            ("display", "flex"),
            ("padding", "10px"),
            ("gap", "5px"),
            ("height", "50px"),
        ];
        let l = lay(
            vec![
                view(&row, &[1, 2]),
                view(&[("width", "20px")], &[]),
                view(&[("flexGrow", "1")], &[]),
            ],
            &[0],
        )
        .unwrap();
        assert_eq!(l.boxes[0].content, rect(10.0, 10.0, 180.0, 30.0));
        assert_eq!(l.boxes[1].border, rect(10.0, 10.0, 20.0, 30.0));
        assert_eq!(l.boxes[2].border, rect(35.0, 10.0, 155.0, 30.0));
    }

    #[test]
    fn flex_items_go_in_order_then_as_written() {
        let l = lay(
            vec![
                view(&[("display", "flex")], &[1, 2, 3, 4]),
                view(&[("width", "10px"), ("order", "1")], &[]),
                view(&[("width", "20px")], &[]),
                view(&[("width", "30px"), ("order", "-1")], &[]),
                view(&[("width", "40px")], &[]),
            ],
            &[0],
        )
        .unwrap();
        let x: Vec<f32> = (1..5).map(|i| l.boxes[i].border.x).collect();
        assert_eq!(x, [90.0, 30.0, 0.0, 50.0]);
    }

    #[test]
    fn grids_place_items_on_tracks_and_lines() {
        let grid = [
            ("display", "grid"),
            ("position", "relative"),
            ("gridTemplateColumns", "[a] 50px [b] 1fr [c]"),
            ("gridTemplateRows", "20px"),
            ("gridAutoRows", "10px"),
            ("columnGap", "10px"),
        ];
        let l = lay(
            vec![
                view(&grid, &[1, 2, 3, 4]),
                view(&[("gridColumn", "span 2")], &[]),
                view(&[], &[]),
                view(&[("gridColumn", "b")], &[]),
                view(
                    &[
                        ("position", "absolute"),
                        ("inset", "0"),
                        ("gridColumn", "b / c"),
                        ("gridRow", "1 / 2"),
                    ],
                    &[],
                ),
                view(&[("display", "flex")], &[6]),
                view(
                    &[
                        ("display", "grid"),
                        ("gridTemplateColumns", "30px 20px"),
                        ("gridTemplateRows", "15px"),
                    ],
                    &[],
                ),
            ],
            &[0, 5],
        )
        .unwrap();
        assert_eq!(l.boxes[1].border, rect(0.0, 0.0, 200.0, 20.0));
        assert_eq!(l.boxes[2].border, rect(0.0, 20.0, 50.0, 10.0));
        assert_eq!(l.boxes[3].border, rect(60.0, 20.0, 140.0, 10.0));
        // An absolute box in a grid takes its grid area.
        assert_eq!(l.boxes[4].border, rect(60.0, 0.0, 140.0, 20.0));
        // A grid's tracks size it without items.
        assert_eq!(l.boxes[6].border, rect(0.0, 30.0, 50.0, 15.0));
    }

    #[test]
    fn grid_areas_place_items_by_name() {
        let l = lay(
            vec![
                view(
                    &[
                        ("display", "grid"),
                        (
                            "gridTemplate",
                            "'head head' 10px 'side main' 30px / 40px 1fr",
                        ),
                    ],
                    &[1, 2, 3],
                ),
                view(&[("gridArea", "main")], &[]),
                view(&[("gridArea", "head")], &[]),
                // An area names the lines around it too.
                view(&[("gridColumn", "side")], &[]),
            ],
            &[0],
        )
        .unwrap();
        assert_eq!(l.boxes[1].border, rect(40.0, 10.0, 160.0, 30.0));
        assert_eq!(l.boxes[2].border, rect(0.0, 0.0, 200.0, 10.0));
        assert_eq!(l.boxes[3].border, rect(0.0, 10.0, 40.0, 30.0));
    }

    #[test]
    fn sizes_take_box_sizing_and_aspect_ratio() {
        let l = lay(
            vec![
                view(&[("width", "50px"), ("padding", "5px")], &[]),
                view(
                    &[
                        ("width", "50px"),
                        ("padding", "5px"),
                        ("boxSizing", "content-box"),
                    ],
                    &[],
                ),
                view(&[("width", "50px"), ("aspectRatio", "5 / 2")], &[]),
            ],
            &[0, 1, 2],
        )
        .unwrap();
        assert_eq!(l.boxes[0].border, rect(0.0, 0.0, 50.0, 10.0));
        assert_eq!(l.boxes[1].border, rect(0.0, 10.0, 60.0, 10.0));
        assert_eq!(l.boxes[2].border, rect(0.0, 20.0, 50.0, 20.0));
    }

    #[test]
    fn math_functions_resolve_against_what_their_percentages_are_of() {
        let l = lay(
            vec![
                view(
                    &[
                        ("width", "calc(50% - 10px)"),
                        ("height", "min(10px, 50%)"),
                        ("paddingLeft", "calc(10% + 5px)"),
                    ],
                    &[],
                ),
                text_node(
                    "Hi",
                    &[
                        ("width", "max(20%, 10px)"),
                        ("paddingLeft", "calc(5% - 2px)"),
                    ],
                ),
            ],
            &[0, 1],
        )
        .unwrap();
        assert_eq!(l.boxes[0].border, rect(0.0, 0.0, 90.0, 10.0));
        assert_eq!(l.boxes[0].content.x, 25.0);
        assert_eq!(l.boxes[1].border.width, 40.0);
        assert_eq!(l.boxes[1].content.x, 8.0);
    }

    #[test]
    fn widths_from_content_break_text_or_not() {
        let l = lay(
            vec![
                view(&[("width", "min-content")], &[1]),
                text_node("Hi there", &[]),
                view(&[("width", "max-content")], &[3]),
                text_node("Hi there", &[]),
            ],
            &[0, 2],
        )
        .unwrap();
        let lines = |i: usize| l.lines[i].as_ref().map_or(0, |lines| lines.text.len());
        assert_eq!((lines(1), lines(3)), (2, 1));
        assert!(l.boxes[0].border.width < l.boxes[2].border.width);
        assert!(l.boxes[2].border.width < 100.0);
    }

    #[test]
    fn absolute_boxes_take_the_nearest_positioned_ancestor() {
        let l = lay(
            vec![
                view(
                    &[
                        ("position", "relative"),
                        ("left", "50px"),
                        ("top", "20px"),
                        ("width", "100px"),
                        ("height", "50px"),
                    ],
                    &[1],
                ),
                view(&[("padding", "10px")], &[2]),
                view(
                    &[
                        ("position", "absolute"),
                        ("top", "5px"),
                        ("left", "5px"),
                        ("width", "10px"),
                        ("height", "10px"),
                    ],
                    &[],
                ),
                view(&[], &[4]),
                view(
                    &[
                        ("position", "absolute"),
                        ("right", "0"),
                        ("bottom", "0"),
                        ("width", "10px"),
                        ("height", "10px"),
                    ],
                    &[],
                ),
            ],
            &[0, 3],
        )
        .unwrap();
        assert_eq!(l.boxes[0].border, rect(50.0, 20.0, 100.0, 50.0));
        assert_eq!(l.boxes[1].content, rect(60.0, 30.0, 80.0, 0.0));
        assert_eq!(l.boxes[2].border, rect(55.0, 25.0, 10.0, 10.0));
        // No positioned ancestor: the canvas contains it.
        assert_eq!(l.boxes[4].border, rect(190.0, 90.0, 10.0, 10.0));
    }

    #[test]
    fn fixed_boxes_take_the_canvas() {
        let fixed = [("position", "fixed"), ("width", "10px"), ("height", "10px")];
        let l = lay(
            vec![
                view(
                    &[
                        ("position", "relative"),
                        ("left", "50px"),
                        ("top", "20px"),
                        ("padding", "10px"),
                        ("height", "50px"),
                    ],
                    &[1, 2],
                ),
                view(
                    &[
                        ("right", "0"),
                        ("bottom", "0"),
                        fixed[0],
                        fixed[1],
                        fixed[2],
                    ],
                    &[],
                ),
                view(&fixed, &[3]),
                view(
                    &[
                        ("position", "absolute"),
                        ("left", "50%"),
                        ("width", "10px"),
                        ("height", "10px"),
                    ],
                    &[],
                ),
            ],
            &[0],
        )
        .unwrap();
        assert_eq!(l.boxes[1].border, rect(190.0, 90.0, 10.0, 10.0));
        // Without insets, where it would be in flow.
        assert_eq!(l.boxes[2].border, rect(60.0, 30.0, 10.0, 10.0));
        // A fixed box is positioned, and contains absolute boxes.
        assert_eq!(l.boxes[3].border, rect(65.0, 30.0, 10.0, 10.0));
    }

    #[test]
    fn transformed_boxes_contain_absolute_and_fixed_boxes() {
        let at = |inset: [(&'static str, &'static str); 3]| {
            let mut style = vec![inset[0], inset[1], inset[2]];
            style.extend([("width", "10px"), ("height", "10px")]);
            view(&style, &[])
        };
        let l = lay(
            vec![
                view(
                    &[
                        ("marginLeft", "20px"),
                        ("marginTop", "10px"),
                        ("width", "100px"),
                        ("height", "50px"),
                        ("transform", "rotate(10deg)"),
                    ],
                    &[1, 2],
                ),
                at([("position", "absolute"), ("right", "0"), ("bottom", "0")]),
                at([("position", "fixed"), ("left", "5px"), ("top", "5px")]),
            ],
            &[0],
        )
        .unwrap();
        // From its padding box, as laid out before it is transformed.
        assert_eq!(l.boxes[1].border, rect(110.0, 50.0, 10.0, 10.0));
        assert_eq!(l.boxes[2].border, rect(25.0, 15.0, 10.0, 10.0));
    }

    #[test]
    fn a_mask_lays_out_its_source_in_its_border_box_as_the_canvas_is() {
        let l = lay(
            vec![
                mask(
                    (2, false, false),
                    &[
                        ("margin", "10px"),
                        ("width", "100px"),
                        ("height", "40px"),
                        ("border", "5px solid red"),
                        ("padding", "5px"),
                    ],
                    &[1],
                ),
                view(&[("height", "10px")], &[]),
                view(&[], &[3, 4]),
                view(&[("height", "50%")], &[]),
                view(
                    &[
                        ("position", "absolute"),
                        ("right", "0"),
                        ("bottom", "0"),
                        ("width", "10px"),
                        ("height", "10px"),
                    ],
                    &[],
                ),
            ],
            &[0],
        )
        .unwrap();
        assert_eq!(l.boxes[1].border, rect(20.0, 20.0, 80.0, 10.0));
        // Its root is the mask's border box, which holds what is absolute in it.
        assert_eq!(l.boxes[2].border, rect(10.0, 10.0, 100.0, 40.0));
        assert_eq!(l.boxes[3].border, rect(10.0, 10.0, 100.0, 20.0));
        assert_eq!(l.boxes[4].border, rect(100.0, 40.0, 10.0, 10.0));
    }

    #[test]
    fn text_is_measured_and_set_in_its_box() {
        let l = lay(
            vec![
                view(&[("width", "60px")], &[1]),
                text_node("one two three", &[("padding", "2px")]),
                view(&[("display", "flex")], &[3]),
                text_node("Hi", &[]),
            ],
            &[0, 2],
        )
        .unwrap();
        let wrapped = l.lines[1].as_ref().unwrap();
        assert!(wrapped.boxes.len() >= 2);
        let height = 22.0 * wrapped.boxes.len() as f32;
        assert_eq!(l.boxes[1].border, rect(0.0, 0.0, 60.0, height + 4.0));
        assert_eq!(l.boxes[1].content, rect(2.0, 2.0, 56.0, height));
        // A flex item shrinks to fit its text.
        let hi = l.boxes[3].border;
        assert!(hi.width > 0.0 && hi.width < 30.0, "{hi:?}");
        assert_eq!([hi.x, hi.y, hi.height], [0.0, height + 4.0, 22.0]);
        assert!(l.lines[0].is_none());
    }

    #[test]
    fn spans_are_boxes_of_their_text_on_its_lines() {
        let piece = |s: &str| template::Piece::Text(Prop::Const(V::Str(s.into())));
        let mut text = text_node("", &[("padding", "2px")]);
        text.kind = Kind::Text {
            content: vec![piece("Hi "), template::Piece::Span(1), piece(" there")],
        };
        text.children = vec![1];
        let mut span = view(&[("fontSize", "32px")], &[]);
        span.kind = Kind::Span {
            content: vec![piece("big")],
            unit: false,
        };
        let l = lay(vec![text, span], &[0]).unwrap();
        let lines = l.lines[0].as_ref().unwrap();
        // As high as the big text: at 32px its ascent is 34 and its descent 9.
        assert_eq!(lines.boxes, [[0.0, 43.0]]);
        let big = lines.runs[1];
        assert_eq!(big.node, 1);
        let width = big.x[1] - big.x[0];
        assert_eq!(l.boxes[1].border, rect(2.0 + big.x[0], 2.0, width, 43.0));
    }

    #[test]
    fn decorations_are_lines_on_runs_with_gaps_at_ink() {
        let style = |skip: &str| {
            let style = [
                ("fontSize", "20px"),
                ("lineHeight", "30px"),
                ("textDecoration", "underline line-through"),
                ("textDecorationSkipInk", skip),
            ];
            let l = lay(vec![text_node("Hugs", &style)], &[0]).unwrap();
            l.lines[0].clone().unwrap()
        };
        let lines = style("auto");
        let b = lines.baselines[0];
        let run = lines.runs[0];
        let (under, through): (Vec<&Stripe>, Vec<_>) =
            lines.decorations.iter().partition(|d| !d.through);
        // 2 px thick, 1 px under the baseline, and broken where the g goes through it, in whole px.
        let top = (b + 1.0).round();
        assert_eq!(under.len(), 2);
        for d in &under {
            assert_eq!([d.rect[1], d.rect[3]], [top, top + 2.0]);
        }
        assert_eq!([under[0].rect[0], under[1].rect[2]], run.x);
        let gap = [under[0].rect[2], under[1].rect[0]];
        let g = lines.glyphs[2].x;
        assert!(gap[0].fract() == 0.0 && gap[1].fract() == 0.0, "{gap:?}");
        assert!(
            gap[0] < g + 5.0 && g + 5.0 < gap[1],
            "{gap:?} for the g at {g}"
        );
        // A third of the ascent, 21 px, over the baseline, centered.
        assert_eq!(through.len(), 1);
        assert_eq!(through[0].rect[1], (b - 7.0 - 1.0).round());
        assert_eq!(style("none").decorations.len(), 2);
    }

    #[test]
    fn cjk_decorations_stay_continuous_in_auto_but_all_still_skips_ink() {
        for kind in ["solid", "double", "dotted", "dashed", "wavy"] {
            let decoration = format!("underline {kind} 2px");
            let lines = |words: &str, skip: &str| {
                let l = lay(
                    vec![text_node(
                        words,
                        &[
                            ("fontSize", "72px"),
                            ("fontWeight", "800"),
                            ("whiteSpace", "pre"),
                            ("textDecoration", &decoration),
                            ("textDecorationSkipInk", skip),
                        ],
                    )],
                    &[0],
                )
                .unwrap();
                l.lines[0].clone().unwrap()
            };
            let cjk = lines("强调随行，", "none");
            assert_eq!(
                lines("强调随行，", "auto").decorations,
                cjk.decorations,
                "{kind}"
            );
            assert_ne!(
                lines("强调随行，", "all").decorations,
                cjk.decorations,
                "{kind}"
            );
            let latin = lines("gyp", "auto");
            assert_eq!(lines("gyp", "all").decorations, latin.decorations, "{kind}");
            assert_ne!(
                lines("gyp", "none").decorations,
                latin.decorations,
                "{kind}"
            );
            // Latin descenders still make gaps when CJK shares their text box and font.
            let mixed = lines("强调 gyp", "auto");
            let along = [
                mixed.runs.first().unwrap().x[0],
                mixed.runs.last().unwrap().x[1],
            ];
            assert!(
                mixed.decorations.iter().all(|d| d.along == along),
                "{kind}: shaping must not reset the pattern"
            );
            let first = mixed.decorations.first().unwrap();
            assert_eq!(
                [first.rect[0], first.rect[2]],
                mixed.runs[0].x,
                "{kind}: CJK ink must not cut the first shaping run"
            );
            assert_ne!(
                mixed.decorations,
                lines("强调 gyp", "none").decorations,
                "{kind}"
            );
        }
    }

    #[test]
    fn decoration_styles_draw_two_lines_or_a_wave_or_marks_along_the_run() {
        let lines = |decoration: &str| {
            let style = [
                ("fontSize", "20px"),
                ("lineHeight", "30px"),
                ("textDecoration", decoration),
                ("textDecorationSkipInk", "none"),
            ];
            let l = lay(vec![text_node("mmm", &style)], &[0]).unwrap();
            l.lines[0].clone().unwrap()
        };
        let rows = |l: &Lines| -> Vec<[f32; 2]> {
            l.decorations
                .iter()
                .map(|d| [d.rect[1], d.rect[3]])
                .collect()
        };
        // A double line is two solid ones, a px past the thickness apart: under, and over upward.
        let solid = lines("underline 3px");
        let [top, bottom] = [solid.decorations[0].rect[1], solid.decorations[0].rect[3]];
        let double = lines("underline double 3px");
        assert_eq!(rows(&double), [[top, bottom], [top + 4.0, bottom + 4.0]]);
        assert!(
            double
                .decorations
                .iter()
                .all(|d| d.style == DecorationStyle::Solid)
        );
        let over = lines("overline double 3px");
        let [first, second] = [over.decorations[0].rect[1], over.decorations[1].rect[1]];
        assert_eq!(second, first - 4.0);
        // A wave is around an axis half a px under the line's top and as far again further out, in
        // a band its curve stays in; dots are spaced along the whole run.
        let wavy = lines("underline wavy 3px");
        let [y0, y1] = [wavy.decorations[0].rect[1], wavy.decorations[0].rect[3]];
        assert_eq!((y0 + y1) / 2.0, top + 0.5 + 4.0);
        assert!(y1 - y0 > 2.0 * 10.5);
        let dotted = lines("underline dotted");
        let d = dotted.decorations[0];
        assert_eq!(
            (d.style, d.along),
            (DecorationStyle::Dotted, dotted.runs[0].x)
        );
    }

    #[test]
    fn a_decoration_takes_the_font_of_its_decorating_box() {
        let piece = |s: &str| template::Piece::Text(Prop::Const(V::Str(s.into())));
        let mut text = text_node("", &[("textDecoration", "line-through")]);
        text.kind = Kind::Text {
            content: vec![piece("aa"), template::Piece::Span(1)],
        };
        text.children = vec![1];
        let mut span = view(&[("fontSize", "32px"), ("textDecoration", "overline")], &[]);
        span.kind = Kind::Span {
            content: vec![piece("bb")],
            unit: false,
        };
        let l = lay(vec![text, span], &[0]).unwrap();
        let lines = l.lines[0].as_ref().unwrap();
        let b = lines.baselines[0];
        let on = |node: u32| -> Vec<[f32; 2]> {
            (lines.decorations.iter())
                .filter(|d| lines.runs[d.run as usize].node == node)
                .map(|d| [d.rect[1], d.rect[3]])
                .collect()
        };
        // The Text's line-through, 1.6 px drawn as 1, is two thirds of the Text's ascent, 17 px,
        // under each run's ascent: the Text's 17 px, and the big Span's 34.
        let through = |ascent: f32| {
            let top = (b - ascent + 17.0 * 2.0 / 3.0 - 0.8).round();
            [top, top + 1.0]
        };
        assert_eq!(on(0), [through(17.0)]);
        // The Span's overline, 3.2 px drawn as 3, is on its ascent.
        let over = (b - 34.0 - 3.0).round();
        assert_eq!(on(1), [through(34.0), [over, over + 3.0]]);
    }

    #[test]
    fn flex_items_align_on_the_first_baseline_of_text() {
        let l = lay(
            vec![
                view(
                    &[("display", "flex"), ("alignItems", "baseline")],
                    &[1, 2, 3],
                ),
                view(&[("width", "20px"), ("height", "6px")], &[]),
                text_node("Ag", &[("paddingTop", "3px")]),
                view(&[("width", "20px")], &[]),
            ],
            &[0],
        )
        .unwrap();
        let baseline = l.boxes[2].content.y + l.lines[2].as_ref().unwrap().glyphs[0].y;
        assert_eq!(l.boxes[2].border.y, 0.0);
        // A box without text aligns its bottom edge, an empty one its top.
        assert_eq!(l.boxes[1].border.y + 6.0, baseline);
        assert_eq!(l.boxes[3].border.y, baseline);
    }

    #[test]
    fn hidden_subtrees_take_no_room() {
        let l = lay(
            vec![
                view(&[("display", "none"), ("height", "50px")], &[1]),
                text_node("hidden", &[]),
                view(&[("height", "10px")], &[]),
            ],
            &[0, 2],
        )
        .unwrap();
        assert_eq!(l.boxes[0], Boxes::default());
        assert_eq!(l.boxes[1], Boxes::default());
        assert!(l.lines[1].is_none());
        assert_eq!(l.boxes[2].border, rect(0.0, 0.0, 200.0, 10.0));
    }

    #[test]
    fn text_errors_point_at_the_node() {
        let e = lay(vec![text_node("a", &[("fontFamily", "Brand")])], &[0]);
        assert_eq!(
            e.err().unwrap(),
            "s.tsx:1:1: font family Brand: the scene declares no font resource Brand in controls.resources"
        );
    }
}
