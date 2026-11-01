//! Paint: a frame's boxes and text as a display list in CSS painting order (CSS 2.1 Appendix E),
//! in canvas px, colors premultiplied linear Rec.2020 (the working space).
//!
//! A stacking context paints its own background, then its in-flow content: the backgrounds of
//! its blocks, then their text and their flex items, each flex item all at once, as an inline
//! block. Its positioned descendants and the stacking contexts in it come last, in tree order;
//! a positioned box paints all at once too, without the positioned boxes in it. Only opacity
//! makes a stacking context so far, and nothing has a z-index.

use crate::layout::{Layout, Rect};
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
}

/// Linear sRGB → linear Rec.2020 (ITU-R BT.2087).
const TO_REC2020: [[f64; 3]; 3] = [
    [
        0.627_403_895_934_699,
        0.329_283_038_377_884,
        0.043_313_065_687_417,
    ],
    [
        0.069_097_289_358_232,
        0.919_540_395_075_459,
        0.011_362_315_566_309,
    ],
    [
        0.016_391_438_875_150,
        0.088_013_307_877_226,
        0.895_595_253_247_624,
    ],
];

/// A computed color (straight linear sRGB) as a working color.
pub fn work(c: Rgba) -> [f32; 4] {
    let row = |r: [f64; 3]| ((r[0] * c[0] + r[1] * c[1] + r[2] * c[2]) * c[3]) as f32;
    [
        row(TO_REC2020[0]),
        row(TO_REC2020[1]),
        row(TO_REC2020[2]),
        c[3] as f32,
    ]
}

/// The frame's display list, from its template, computed styles and layout.
pub fn paint(template: &Template, styles: &[Style], layout: &Layout) -> Vec<Item> {
    let painter = Painter {
        template,
        styles,
        layout,
    };
    let mut out = Vec::new();
    painter.context(None, &mut out);
    out
}

/// Boxes are template indices; `None` is the canvas, a block container without a background.
struct Painter<'a> {
    template: &'a Template,
    styles: &'a [Style],
    layout: &'a Layout,
}

impl Painter<'_> {
    fn children(&self, i: Option<usize>) -> impl Iterator<Item = usize> {
        let list = match i {
            None => &self.template.roots,
            Some(i) => &self.template.nodes[i].children,
        };
        list.iter()
            .map(|&c| c as usize)
            .filter(|&c| self.styles[c].layout.display != taffy::Display::None)
    }

    fn positioned(&self, i: usize) -> bool {
        self.styles[i].layout.position != taffy::Position::Static
    }

    fn stacking(&self, i: usize) -> bool {
        self.styles[i].opacity < 1.0
    }

    /// Paints after the in-flow content of its stacking context.
    fn layered(&self, i: usize) -> bool {
        self.positioned(i) || self.stacking(i)
    }

    /// An in-flow child that paints all at once: a flex item.
    fn atomic(&self, parent: Option<usize>, child: usize) -> bool {
        parent.is_some_and(|p| self.styles[p].layout.display == taffy::Display::Flex)
            && !self.layered(child)
    }

    fn context(&self, i: Option<usize>, out: &mut Vec<Item>) {
        self.whole(i, out);
        let mut layered = Vec::new();
        self.collect(i, &mut layered);
        for d in layered {
            if self.stacking(d) {
                let mut items = Vec::new();
                self.context(Some(d), &mut items);
                group(self.styles[d].opacity as f32, items, out);
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
    fn whole(&self, i: Option<usize>, out: &mut Vec<Item>) {
        self.backgrounds(i, out);
        self.foreground(i, out);
    }

    fn backgrounds(&self, i: Option<usize>, out: &mut Vec<Item>) {
        if let Some(i) = i {
            self.background(i, out);
        }
        for c in self.children(i) {
            if !self.layered(c) && !self.atomic(i, c) {
                self.backgrounds(Some(c), out);
            }
        }
    }

    fn foreground(&self, i: Option<usize>, out: &mut Vec<Item>) {
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

    fn background(&self, i: usize, out: &mut Vec<Item>) {
        let s = &self.styles[i];
        let rect = self.layout.boxes[i].border;
        if s.background[3] <= 0.0 || rect.width <= 0.0 || rect.height <= 0.0 {
            return;
        }
        // Radii too large for the box shrink to fit it.
        let radius = (s.radius as f32)
            .min(rect.width / 2.0)
            .min(rect.height / 2.0);
        out.push(Item::Rect {
            rect,
            radius,
            color: work(s.background),
        });
    }

    fn text(&self, i: usize, out: &mut Vec<Item>) {
        let Some(lines) = &self.layout.lines[i] else {
            return;
        };
        let color = work(self.styles[i].color);
        if color[3] <= 0.0 {
            return;
        }
        let at = self.layout.boxes[i].content;
        for run in lines.glyphs.chunk_by(|a, b| a.face == b.face) {
            out.push(Item::Glyphs {
                face: run[0].face,
                size: lines.size,
                color,
                glyphs: run
                    .iter()
                    .map(|g| (g.id, [at.x + g.x, at.y + g.y]))
                    .collect(),
            });
        }
    }
}

/// `items` as one layer at `opacity`. A layer of one rectangle is that rectangle at the opacity,
/// and a layer of one layer is that layer at both opacities.
fn group(opacity: f32, items: Vec<Item>, out: &mut Vec<Item>) {
    if opacity <= 0.0 || items.is_empty() {
        return;
    }
    out.push(match <[Item; 1]>::try_from(items) {
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
    });
}

#[cfg(test)]
mod tests;
