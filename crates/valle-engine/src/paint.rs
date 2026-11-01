//! Paint: a frame's boxes and text as a display list in CSS painting order (CSS 2.1 Appendix E),
//! in canvas px, colors premultiplied (the working space: sRGB-encoded values, which browsers
//! composite).
//!
//! A stacking context paints its own background, then its in-flow content: the backgrounds of
//! its blocks, then their text and their flex and grid items, each item all at once, as an
//! inline block. Its positioned descendants and the stacking contexts in it come last, in tree
//! order; a positioned box paints all at once too, without the positioned boxes in it. Only
//! opacity makes a stacking context so far, and nothing has a z-index. Tree order puts a flex or
//! grid container's children in the order it lays them out.
//!
//! A box that clips what overflows it clips what it contains to its padding box, with its
//! corners rounded: its content and in-flow descendants, and the positioned ones whose
//! containing block is it or inside it. Layers group what they paint, but clips go on each item,
//! so a positioned box in a layer under a clip that does not contain it shows outside the clip.
//!
//! Boxes paint with their edges on whole pixels, as Chrome paints them, so a layout that lands
//! between pixels still has crisp edges.

use taffy::{Display, Overflow, Position};

use crate::layout::{Layout, Rect, ordered};
use crate::style::Style;
use crate::style::color::Rgba;
use crate::template::Template;

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    /// A rectangle with its corners rounded by `radius` px, which fits in it.
    Rect {
        rect: Rect,
        radius: f32,
        color: [f32; 4],
    },
    /// Glyphs of `Text::faces()[face]` at `size` px: glyph ids at their baseline origins.
    Glyphs {
        face: u32,
        size: f32,
        color: [f32; 4],
        glyphs: Vec<(u32, [f32; 2])>,
    },
    /// Items composited onto what is under them as one layer, at `opacity`.
    Group { opacity: f32, items: Vec<Item> },
    /// Items shown only inside a rectangle with its corners rounded by `radius` px.
    Clip {
        rect: Rect,
        radius: f32,
        items: Vec<Item>,
    },
}

/// Items, each with the innermost box whose clip it is under.
type Drawn = Vec<(Option<usize>, Item)>;

/// Far past any canvas: the extent of a clip on an axis it leaves open.
const OPEN: f32 = 1.0e7;

/// A box as Chrome paints it (Blink's pixel snapping): each edge on the nearest pixel boundary,
/// halves rounding up, and a side longer than 4/64 px at least a pixel long.
fn snap(r: Rect) -> Rect {
    let round = |v: f32| (v + 0.5).floor();
    let side = |at: f32, size: f32| match round(at + size) - round(at) {
        0.0 if size.abs() > 4.0 / 64.0 => size.signum(),
        snapped => snapped,
    };
    Rect {
        x: round(r.x),
        y: round(r.y),
        width: side(r.x, r.width),
        height: side(r.y, r.height),
    }
}

/// A computed color as a working color.
pub fn work(c: Rgba) -> [f32; 4] {
    let a = c[3];
    [c[0] * a, c[1] * a, c[2] * a, a].map(|v| v as f32)
}

/// The frame's display list, from its template, computed styles and layout.
pub fn paint(template: &Template, styles: &[Style], layout: &Layout) -> Vec<Item> {
    let n = template.nodes.len();
    let mut parents = vec![None; n];
    for (i, node) in template.nodes.iter().enumerate() {
        for &c in &node.children {
            parents[c as usize] = Some(i);
        }
    }
    let mut painter = Painter {
        template,
        styles,
        layout,
        clips: Vec::with_capacity(n),
    };
    // In preorder, so the box whose clips a box takes has its own.
    for i in 0..n {
        let clips = match painter.container(i, &parents) {
            None => Vec::new(),
            Some(c) => {
                let mut clips = painter.clips[c].clone();
                clips.extend(painter.clipping(c).then_some(c));
                clips
            }
        };
        painter.clips.push(clips);
    }
    let mut out = Vec::new();
    painter.context(None, &mut out);
    painter.nest(out)
}

/// Boxes are template indices; `None` is the canvas, a block container without a background.
struct Painter<'a> {
    template: &'a Template,
    styles: &'a [Style],
    layout: &'a Layout,
    /// The boxes whose clips each box is under, outermost first.
    clips: Vec<Vec<usize>>,
}

impl Painter<'_> {
    fn children(&self, i: Option<usize>) -> impl Iterator<Item = usize> {
        let list: Vec<usize> = match i {
            None => (self.template.roots.iter()).map(|&c| c as usize).collect(),
            Some(i) => ordered(
                &self.template.nodes[i].children,
                &self.styles[i],
                self.styles,
            )
            .collect(),
        };
        (list.into_iter()).filter(|&c| self.styles[c].layout.display != Display::None)
    }

    fn positioned(&self, i: usize) -> bool {
        self.styles[i].layout.position != Position::Static
    }

    /// The box whose clips `i` is under too: a fixed box's is the canvas, an absolute box's its
    /// containing block, and any other's its parent.
    fn container(&self, i: usize, parents: &[Option<usize>]) -> Option<usize> {
        match self.styles[i].layout.position {
            Position::Fixed => None,
            Position::Absolute => {
                std::iter::successors(parents[i], |&p| parents[p]).find(|&p| self.positioned(p))
            }
            _ => parents[i],
        }
    }

    /// Whether `i` clips what overflows it.
    fn clipping(&self, i: usize) -> bool {
        let o = self.styles[i].layout.overflow;
        o.x != Overflow::Visible || o.y != Overflow::Visible
    }

    fn stacking(&self, i: usize) -> bool {
        self.styles[i].opacity < 1.0
    }

    /// Paints after the in-flow content of its stacking context.
    fn layered(&self, i: usize) -> bool {
        self.positioned(i) || self.stacking(i)
    }

    /// An in-flow child that paints all at once: a flex or grid item.
    fn atomic(&self, parent: Option<usize>, child: usize) -> bool {
        let display = |p: usize| self.styles[p].layout.display;
        parent.is_some_and(|p| matches!(display(p), Display::Flex | Display::Grid))
            && !self.layered(child)
    }

    fn context(&self, i: Option<usize>, out: &mut Drawn) {
        self.whole(i, out);
        let mut layered = Vec::new();
        self.collect(i, &mut layered);
        for d in layered {
            if self.stacking(d) {
                let mut drawn = Vec::new();
                self.context(Some(d), &mut drawn);
                let items = self.nest(drawn);
                out.extend(group(self.styles[d].opacity as f32, items).map(|g| (None, g)));
            } else {
                self.whole(Some(d), out);
            }
        }
    }

    /// The positioned boxes and stacking contexts that paint in `i`'s stacking context, in tree
    /// order.
    fn collect(&self, i: Option<usize>, out: &mut Vec<usize>) {
        for c in self.children(i) {
            if self.layered(c) {
                out.push(c);
            }
            if !self.stacking(c) {
                self.collect(Some(c), out);
            }
        }
    }

    /// `i` and its in-flow content.
    fn whole(&self, i: Option<usize>, out: &mut Drawn) {
        self.backgrounds(i, out);
        self.foreground(i, out);
    }

    fn backgrounds(&self, i: Option<usize>, out: &mut Drawn) {
        if let Some(i) = i {
            self.background(i, out);
        }
        for c in self.children(i) {
            if !self.layered(c) && !self.atomic(i, c) {
                self.backgrounds(Some(c), out);
            }
        }
    }

    fn foreground(&self, i: Option<usize>, out: &mut Drawn) {
        if let Some(i) = i {
            self.text(i, out);
        }
        for c in self.children(i) {
            if self.atomic(i, c) {
                self.whole(Some(c), out);
            } else if !self.layered(c) {
                self.foreground(Some(c), out);
            }
        }
    }

    fn background(&self, i: usize, out: &mut Drawn) {
        let s = &self.styles[i];
        let rect = snap(self.layout.boxes[i].border);
        if s.background[3] <= 0.0 || rect.width <= 0.0 || rect.height <= 0.0 {
            return;
        }
        let item = Item::Rect {
            rect,
            radius: self.radius(i, rect),
            color: work(s.background),
        };
        out.push((self.clips[i].last().copied(), item));
    }

    /// `i`'s corner radius in `rect`: radii too large for it shrink to fit.
    fn radius(&self, i: usize, rect: Rect) -> f32 {
        (self.styles[i].radius as f32)
            .min(rect.width / 2.0)
            .min(rect.height / 2.0)
    }

    fn text(&self, i: usize, out: &mut Drawn) {
        let Some(lines) = &self.layout.lines[i] else {
            return;
        };
        let color = work(self.styles[i].color);
        if color[3] <= 0.0 {
            return;
        }
        let at = self.layout.boxes[i].content;
        // Text is content, which its own box clips.
        let clip = match self.clipping(i) {
            true => Some(i),
            false => self.clips[i].last().copied(),
        };
        for run in lines.glyphs.chunk_by(|a, b| a.face == b.face) {
            let item = Item::Glyphs {
                face: run[0].face,
                size: lines.size,
                color,
                glyphs: run
                    .iter()
                    .map(|g| (g.id, [at.x + g.x, at.y + g.y]))
                    .collect(),
            };
            out.push((clip, item));
        }
    }

    /// The items inside the clips they are under: runs of items under a clip share it.
    fn nest(&self, drawn: Drawn) -> Vec<Item> {
        let chained = drawn.into_iter().map(|(clip, item)| match clip {
            None => (Vec::new(), item),
            Some(c) => ([&self.clips[c][..], &[c]].concat(), item),
        });
        self.nest_from(chained.collect(), 0)
    }

    /// Items with the boxes they are clipped by, the first `depth` of which clip them already.
    fn nest_from(&self, chained: Vec<(Vec<usize>, Item)>, depth: usize) -> Vec<Item> {
        let mut out = Vec::new();
        let mut chained = chained.into_iter().peekable();
        while let Some((clips, item)) = chained.next() {
            let Some(&c) = clips.get(depth) else {
                out.push(item);
                continue;
            };
            let mut run = vec![(clips, item)];
            while let Some((next, _)) = chained.peek()
                && next.get(depth) == Some(&c)
            {
                run.extend(chained.next());
            }
            let items = self.nest_from(run, depth + 1);
            self.clip(c, items, &mut out);
        }
        out
    }

    /// `items` inside `i`'s padding box on the axes it clips, its corners rounded if it clips
    /// both. Rectangles inside it need no clip, and a lone square one is cut to it.
    fn clip(&self, i: usize, items: Vec<Item>, out: &mut Vec<Item>) {
        let o = self.styles[i].layout.overflow;
        let b = snap(self.layout.boxes[i].padding);
        let axis = |clips: bool, at: f32, size: f32| match clips {
            true => (at, size),
            false => (-OPEN, 2.0 * OPEN),
        };
        let (x, width) = axis(o.x != Overflow::Visible, b.x, b.width);
        let (y, height) = axis(o.y != Overflow::Visible, b.y, b.height);
        let rect = Rect {
            x,
            y,
            width,
            height,
        };
        let radius = match o.x != Overflow::Visible && o.y != Overflow::Visible {
            true => self.radius(i, b),
            false => 0.0,
        };
        if radius == 0.0 {
            let inside =
                |item: &Item| matches!(item, Item::Rect { rect: r, .. } if within(*r, rect));
            if items.iter().all(inside) {
                out.extend(items);
                return;
            }
            if let [
                Item::Rect {
                    rect: r,
                    radius: 0.0,
                    color,
                },
            ] = items[..]
            {
                out.extend(overlap(r, rect).map(|rect| Item::Rect {
                    rect,
                    radius: 0.0,
                    color,
                }));
                return;
            }
        }
        out.push(Item::Clip {
            rect,
            radius,
            items,
        });
    }
}

/// Whether `inner` is inside `outer`.
fn within(inner: Rect, outer: Rect) -> bool {
    inner.x >= outer.x
        && inner.y >= outer.y
        && inner.x + inner.width <= outer.x + outer.width
        && inner.y + inner.height <= outer.y + outer.height
}

/// Where `a` and `b` overlap, if they do.
fn overlap(a: Rect, b: Rect) -> Option<Rect> {
    let (x, y) = (a.x.max(b.x), a.y.max(b.y));
    let width = (a.x + a.width).min(b.x + b.width) - x;
    let height = (a.y + a.height).min(b.y + b.height) - y;
    (width > 0.0 && height > 0.0).then_some(Rect {
        x,
        y,
        width,
        height,
    })
}

/// `items` as one layer at `opacity`. A layer of one rectangle is that rectangle at the opacity,
/// and a layer of one layer is that layer at both opacities.
fn group(opacity: f32, items: Vec<Item>) -> Option<Item> {
    if opacity <= 0.0 || items.is_empty() {
        return None;
    }
    Some(match <[Item; 1]>::try_from(items) {
        Ok(
            [
                Item::Rect {
                    rect,
                    radius,
                    color,
                },
            ],
        ) => Item::Rect {
            rect,
            radius,
            color: color.map(|c| c * opacity),
        },
        Ok([Item::Group { opacity: o, items }]) => Item::Group {
            opacity: o * opacity,
            items,
        },
        Ok([item]) => Item::Group {
            opacity,
            items: vec![item],
        },
        Err(items) => Item::Group { opacity, items },
    })
}

#[cfg(test)]
mod tests;
