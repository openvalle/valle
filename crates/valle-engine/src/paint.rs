//! Paint: a frame's boxes and text as a display list in CSS painting order (CSS 2.1 Appendix E),
//! in canvas px, colors premultiplied (the working space: sRGB-encoded values, which browsers
//! composite).
//!
//! A stacking context paints its own background and border, the stacking contexts in it with a
//! negative z-index, then its in-flow content: the backgrounds and borders of its blocks, then
//! their text and their flex and grid items, each item all at once, as an inline block, then the
//! outlines of its blocks and its own. Its positioned descendants and the stacking contexts in it
//! with a z-index of 0 or `auto` come next, in tree order, and last those with a positive
//! z-index, lowest first. A positioned box paints all at once too, without the positioned boxes
//! in it. Opacity, a transform, a fixed position, and a z-index on a positioned box or a flex or
//! grid item make a stacking context; only opacity makes it a layer. A transformed one's items go
//! in its transform, which takes its px, those of the untransformed layout, to its parent's, and
//! a transform that flattens the plane hides it. Tree order puts a flex or grid container's
//! children in the order it lays them out. A hidden box (`visibility`) paints nothing of its
//! own, but what it contains still paints if visible. A box's background, its color and then its
//! images, the first on top, paints over its outer shadows, and its inner shadows and border over
//! the background.
//!
//! A box that clips what overflows it clips what it contains to its padding box, with its
//! corners rounded: its content and in-flow descendants, and the positioned ones whose
//! containing block is it or inside it. Layers group what they paint, but clips go on each item,
//! so a positioned box in a layer under a clip that does not contain it shows outside the clip. A
//! transformed box contains its positioned descendants, so the clips around it are the same for
//! all it paints, and go around its transform.
//!
//! Boxes paint with their edges on whole pixels, as Chrome paints them, so a layout that lands
//! between pixels still has crisp edges.

mod gradient;
mod matrix;

use taffy::{Display, Overflow, Position};

pub use gradient::{Geometry, Ramp, Segment};
pub use matrix::{Matrix, Matrix4};

use crate::layout::{Layout, Rect, ordered};
use crate::style::background::{Area, ImageSize, Layer, Repeat};
use crate::style::color::Rgba;
use crate::style::gradient::Image;
use crate::style::transform::Function;
use crate::style::{Shadow, Style};
use crate::template::Template;

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    /// A rectangle with its corners rounded by `radii`, which fit in it.
    Rect {
        rect: Rect,
        radii: Radii,
        color: [f32; 4],
    },
    /// The part of a rectangle with its corners rounded by `radii` outside a rectangle `inner`
    /// with its own: a border, an outline, or the shape of an inner shadow. Each side (top,
    /// right, bottom and left) is in its color, and sides meet on the lines from the outer
    /// corners to the inner ones.
    Ring {
        rect: Rect,
        radii: Radii,
        inner: Rect,
        inner_radii: Radii,
        colors: [[f32; 4]; 4],
    },
    /// A gradient filling a rectangle with its corners rounded by `radii`, in `tiles`.
    Gradient {
        rect: Rect,
        radii: Radii,
        tiles: Tiles,
        ramp: Ramp,
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
    /// Items in px that `matrix` takes to the px around them.
    Transform { matrix: Matrix, items: Vec<Item> },
    /// Items shown only inside a rectangle with its corners rounded by `radii`.
    Clip {
        rect: Rect,
        radii: Radii,
        items: Vec<Item>,
    },
    /// Items blurred by a Gaussian of `sigma` px, then shown only outside a rectangle with its
    /// corners rounded by `radii`, or only `inside` it: a box's shadow, which its border box
    /// hides when it is outer, and its padding box keeps in when it is inner.
    Shadow {
        sigma: f32,
        rect: Rect,
        radii: Radii,
        inside: bool,
        items: Vec<Item>,
    },
}

/// Where a background image is: one tile, and on each axis how far it is to the next one, if it
/// repeats.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tiles {
    pub first: Rect,
    pub step: [Option<f32>; 2],
}

/// Elliptical corners: their horizontal and vertical radii in px, from the top left clockwise.
/// A corner with a radius of 0 is square.
pub type Radii = [[f32; 2]; 4];

pub const SQUARE: Radii = [[0.0; 2]; 4];

/// A corner's radii: square if either is 0.
fn corner(r: [f32; 2]) -> [f32; 2] {
    if r.contains(&0.0) { [0.0; 2] } else { r }
}

/// `radii` grown by `by` px, or shrunk where it is negative; square corners stay square.
fn grown(radii: Radii, by: f32) -> Radii {
    radii.map(|r| corner(r.map(|v| if v > 0.0 { (v + by).max(0.0) } else { 0.0 })))
}

/// `radii` grown by a shadow's spread, or shrunk where it is negative. A radius smaller than the
/// spread grows less, as CSS has it, so a small one stays nearly square.
fn spread(radii: Radii, by: f32) -> Radii {
    if by <= 0.0 {
        return grown(radii, by);
    }
    radii.map(|r| {
        corner(r.map(|v| match v {
            v if v <= 0.0 => 0.0,
            v if v >= by => v + by,
            v => v + by * (1.0 + (v / by - 1.0).powi(3)),
        }))
    })
}

/// A background image's tiles in `area`, if it has a size. Gradients have no size of their own:
/// `auto`, `contain` and `cover` are the area's. A rounded axis fits a whole number of tiles,
/// and the other, if `auto`, keeps the tile's proportions; a spaced one fits as many whole
/// tiles as it can from edge to edge, or a single one where the position puts it.
fn tiles(layer: &Layer, area: Rect) -> Option<Tiles> {
    let room = [area.width, area.height];
    let (asked, auto) = match &layer.size {
        ImageSize::Contain | ImageSize::Cover => (room, [false; 2]),
        ImageSize::Sized(s) => (
            [0, 1].map(|k| s[k].as_ref().map_or(room[k], |l| l.resolve(room[k]))),
            [s[0].is_none(), s[1].is_none()],
        ),
    };
    let round = layer.repeat.map(|r| r == Repeat::Round);
    let mut size = asked;
    for k in 0..2 {
        if round[k] && asked[k] > 0.0 {
            size[k] = room[k] / (room[k] / asked[k]).round().max(1.0);
        }
    }
    for k in 0..2 {
        let other = 1 - k;
        if round[k] && !round[other] && auto[other] && asked[k] > 0.0 {
            size[other] = asked[other] * size[k] / asked[k];
        }
    }
    if size[0] <= 0.0 || size[1] <= 0.0 {
        return None;
    }
    // Placed in the room the tile leaves before it is rounded, as Chrome and Safari place it.
    let mut first = [0, 1].map(|k| layer.position[k].resolve(room[k] - asked[k]));
    let mut step = [None; 2];
    for k in 0..2 {
        step[k] = match layer.repeat[k] {
            Repeat::Repeat | Repeat::Round => Some(size[k]),
            Repeat::NoRepeat => None,
            Repeat::Space => {
                let n = (room[k] / size[k]).floor();
                (n >= 2.0).then(|| {
                    first[k] = 0.0;
                    size[k] + (room[k] - n * size[k]) / (n - 1.0)
                })
            }
        };
    }
    let first = Rect {
        x: area.x + first[0],
        y: area.y + first[1],
        width: size[0],
        height: size[1],
    };
    Some(Tiles { first, step })
}

/// `rect` grown by `by` px on each side.
fn outset(rect: Rect, by: f32) -> Rect {
    Rect {
        x: rect.x - by,
        y: rect.y - by,
        width: rect.width + 2.0 * by,
        height: rect.height + 2.0 * by,
    }
}

/// An outer shadow of a box whose border box is `rect` with its corners rounded by `radii`: its
/// shape moved by the offset and grown by the spread, blurred, outside the box. Under an
/// `opaque` background it goes a px into the box, as in Chrome, so that no gap shows at the
/// box's antialiased edge: the box shrinks by a px, its radii by as much as its shorter side.
fn outer_shadow(s: &Shadow, rect: Rect, radii: Radii, opaque: bool) -> Option<Item> {
    let [x, y] = s.offset.map(|v| v as f32);
    let by = s.spread as f32;
    let shape = outset(
        Rect {
            x: rect.x + x,
            y: rect.y + y,
            ..rect
        },
        by,
    );
    let color = work(s.color);
    if color[3] <= 0.0 || shape.width <= 0.0 || shape.height <= 0.0 {
        return None;
    }
    let radii_out = fit(spread(radii, by), shape);
    let (rect, radii) = match opaque {
        true => {
            let less = outset(rect, -1.0);
            let k = match less.width < less.height {
                true => less.width / rect.width,
                false => less.height / rect.height,
            };
            (less, radii.map(|r| r.map(|v| v * k.max(0.0))))
        }
        false => (rect, radii),
    };
    Some(Item::Shadow {
        sigma: s.blur as f32 / 2.0,
        rect,
        radii,
        inside: false,
        items: vec![Item::Rect {
            rect: shape,
            radii: radii_out,
            color,
        }],
    })
}

/// An inner shadow of a box whose padding box is `rect` with its corners rounded by `radii`: all
/// but the padding box's shape moved by the offset and shrunk by the spread, blurred, inside the
/// padding box.
fn inner_shadow(s: &Shadow, rect: Rect, radii: Radii) -> Option<Item> {
    let [x, y] = s.offset.map(|v| v as f32);
    let by = s.spread as f32;
    let hole = outset(
        Rect {
            x: rect.x + x,
            y: rect.y + y,
            ..rect
        },
        -by,
    );
    let color = work(s.color);
    if color[3] <= 0.0 || rect.width <= 0.0 || rect.height <= 0.0 {
        return None;
    }
    let sigma = s.blur as f32 / 2.0;
    // Out past what the blur reaches from the padding box, which sees only shadow there.
    let around = outset(rect, (3.0 * sigma).ceil() + 1.0);
    Some(Item::Shadow {
        sigma,
        rect,
        radii,
        inside: true,
        items: vec![Item::Ring {
            rect: around,
            radii: SQUARE,
            inner: hole,
            inner_radii: fit(spread(radii, -by), hole),
            colors: [color; 4],
        }],
    })
}

/// `radii` all shrunk by one factor until they fit in `rect`, as CSS has them. None fit in a
/// rectangle of no size.
fn fit(radii: Radii, rect: Rect) -> Radii {
    let [tl, tr, br, bl] = radii;
    // NaN where a side and its radii are 0, which `min` passes over.
    let fit = [
        rect.width / (tl[0] + tr[0]),
        rect.width / (bl[0] + br[0]),
        rect.height / (tl[1] + bl[1]),
        rect.height / (tr[1] + br[1]),
    ];
    let k = fit.into_iter().fold(1.0, f32::min).max(0.0);
    radii.map(|r| r.map(|v| v * k))
}

/// The inside of a border `widths` wide (top, right, bottom and left) on `rect` with its corners
/// rounded by `radii`: a rectangle, of a negative size where the border fills `rect`, and its
/// corners' radii, the outer ones less the widths.
fn inner(rect: Rect, radii: Radii, widths: [f32; 4]) -> (Rect, Radii) {
    let [t, r, b, l] = widths;
    let inside = Rect {
        x: rect.x + l,
        y: rect.y + t,
        width: rect.width - l - r,
        height: rect.height - t - b,
    };
    let less = [[l, t], [r, t], [r, b], [l, b]];
    let radii =
        std::array::from_fn(|c| corner([0, 1].map(|k| (radii[c][k] - less[c][k]).max(0.0))));
    (inside, fit(radii, inside))
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
        parents,
        clips: Vec::with_capacity(n),
    };
    // In preorder, so the box whose clips a box takes has its own.
    for i in 0..n {
        let clips = match painter.container(i) {
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

/// Each box's transform from its px to the canvas's: its ancestors' and its own.
pub fn transforms(template: &Template, styles: &[Style], layout: &Layout) -> Vec<Matrix> {
    let n = template.nodes.len();
    let mut out = vec![Matrix::IDENTITY; n];
    let mut seen = vec![None; n];
    // In preorder, so a parent's comes first.
    for (i, node) in template.nodes.iter().enumerate() {
        let m = match transform(&styles[i], layout.boxes[i].border, seen[i]) {
            Some(own) => out[i].times(own.flatten()),
            None => out[i],
        };
        out[i] = m;
        let children = perspective(&styles[i], layout.boxes[i].border);
        for &c in &node.children {
            out[c as usize] = m;
            seen[c as usize] = children;
        }
    }
    out
}

/// What a box's children are seen in: its perspective, from its origin, or `None` for `none`.
/// It reaches only its children, and only one with a transform shows it.
fn perspective(s: &Style, border: Rect) -> Option<Matrix4> {
    let d = s.perspective?;
    let b = snap(border);
    let size = [b.width, b.height];
    let [x, y] =
        [0, 1].map(|k| f64::from([b.x, b.y][k] + s.perspective_origin[k].resolve(size[k])));
    let m = Matrix4::translate(x, y, 0.0).times(Matrix4::perspective(d));
    Some(m.times(Matrix4::translate(-x, -y, 0.0)))
}

/// A box's transform with the border box `border`, in the px around it, seen in its parent's
/// perspective `seen`: its functions, the last first, around its origin. In space: flattened, it
/// is the transform onto its parent's plane. `None` for `none`.
fn transform(s: &Style, border: Rect, seen: Option<Matrix4>) -> Option<Matrix4> {
    if s.transform.is_empty() {
        return None;
    }
    let b = snap(border);
    let size = [b.width, b.height];
    let [x, y] = [0, 1].map(|k| f64::from([b.x, b.y][k] + s.transform_origin[k].resolve(size[k])));
    let z = s.transform_origin_z;
    let m = (s.transform.iter()).fold(Matrix4::IDENTITY, |m, f| {
        m.times(match f {
            Function::Translate([x, y, z]) => {
                let [x, y, z] = [(x, size[0]), (y, size[1]), (z, 0.0)].map(|(l, of)| l.resolve(of));
                Matrix4::translate(x.into(), y.into(), z.into())
            }
            Function::Scale([x, y, z]) => Matrix4::scale(*x, *y, *z),
            Function::Rotate(a) => Matrix4::rotate([0.0, 0.0, 1.0], *a),
            Function::Rotate3d(axis, a) => Matrix4::rotate(*axis, *a),
            Function::Skew([x, y]) => {
                let [x, y] = [x, y].map(|a| a.to_radians().tan());
                Matrix4::affine([1.0, y, x, 1.0, 0.0, 0.0])
            }
            Function::Matrix(m) => Matrix4::affine(*m),
            Function::Matrix3d(m) => Matrix4(std::array::from_fn(|c| {
                std::array::from_fn(|r| m[c * 4 + r])
            })),
            Function::Perspective(None) => Matrix4::IDENTITY,
            Function::Perspective(Some(d)) => Matrix4::perspective(d.resolve(0.0).into()),
        })
    });
    let m = Matrix4::translate(x, y, z)
        .times(m)
        .times(Matrix4::translate(-x, -y, -z));
    Some(seen.unwrap_or(Matrix4::IDENTITY).times(m))
}

/// Boxes are template indices; `None` is the canvas, a block container without a background.
struct Painter<'a> {
    template: &'a Template,
    styles: &'a [Style],
    layout: &'a Layout,
    parents: Vec<Option<usize>>,
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

    /// The box whose clips `i` is under too: a fixed box's is its containing block, the nearest
    /// box around it with a transform or a perspective or the canvas, an absolute box's the
    /// nearest such or positioned one, and any other's its parent.
    fn container(&self, i: usize) -> Option<usize> {
        let parents = &self.parents;
        let all = |p: usize| self.styles[p].contains_all();
        let mut around = std::iter::successors(parents[i], |&p| parents[p]);
        match self.styles[i].layout.position {
            Position::Fixed => around.find(|&p| all(p)),
            Position::Absolute => around.find(|&p| self.positioned(p) || all(p)),
            _ => parents[i],
        }
    }

    /// Whether `i` clips what overflows it.
    fn clipping(&self, i: usize) -> bool {
        let o = self.styles[i].layout.overflow;
        o.x != Overflow::Visible || o.y != Overflow::Visible
    }

    /// Whether `i` is a flex or grid item.
    fn item(&self, i: usize) -> bool {
        let display = |p: usize| self.styles[p].layout.display;
        self.parents[i].is_some_and(|p| matches!(display(p), Display::Flex | Display::Grid))
    }

    fn stacking(&self, i: usize) -> bool {
        let s = &self.styles[i];
        s.opacity < 1.0
            || s.contains_all()
            || s.layout.position == Position::Fixed
            || (s.z_index.is_some() && (self.positioned(i) || self.item(i)))
    }

    /// Where `i` paints among the layered boxes of its stacking context: only a positioned box
    /// or a flex or grid item takes a z-index.
    fn z(&self, i: usize) -> i32 {
        match self.positioned(i) || self.item(i) {
            true => self.styles[i].z_index.unwrap_or(0),
            false => 0,
        }
    }

    /// Paints after the in-flow content of its stacking context, or before it with a negative
    /// z-index.
    fn layered(&self, i: usize) -> bool {
        self.positioned(i) || self.stacking(i)
    }

    /// An in-flow child that paints all at once: a flex or grid item.
    fn atomic(&self, child: usize) -> bool {
        self.item(child) && !self.layered(child)
    }

    fn context(&self, i: Option<usize>, out: &mut Drawn) {
        let mut layered = Vec::new();
        self.collect(i, &mut layered);
        // A stable sort: ties stay in tree order.
        layered.sort_by_key(|&d| self.z(d));
        let below = layered.partition_point(|&d| self.z(d) < 0);
        if let Some(i) = i {
            self.background(i, out);
        }
        for &d in &layered[..below] {
            self.layer(d, out);
        }
        self.content(i, out);
        for &d in &layered[below..] {
            self.layer(d, out);
        }
    }

    /// A box that paints after its stacking context's in-flow content, or before it: a stacking
    /// context, in its transform if it has one and a layer if it has an opacity, or a positioned
    /// box.
    fn layer(&self, d: usize, out: &mut Drawn) {
        let opacity = self.styles[d].opacity as f32;
        let seen =
            self.parents[d].and_then(|p| perspective(&self.styles[p], self.layout.boxes[p].border));
        let transform = transform(&self.styles[d], self.layout.boxes[d].border, seen);
        if !self.stacking(d) {
            self.whole(Some(d), out);
        } else if let Some(space) = transform {
            let matrix = space.flatten();
            // Its back hides it and all in it, as they are on its plane.
            let back = !self.styles[d].backface_visible && space.shows_back();
            if matrix.inverse().is_none() || back {
                return;
            }
            let mut drawn = Vec::new();
            self.context(Some(d), &mut drawn);
            // The clips around it go around its transform.
            let mut items = self.nest_from(self.chained(drawn), self.clips[d].len());
            if opacity < 1.0 {
                items = group(opacity, items).into_iter().collect();
            }
            if !items.is_empty() {
                let item = Item::Transform { matrix, items };
                out.push((self.clips[d].last().copied(), item));
            }
        } else if opacity < 1.0 {
            let mut drawn = Vec::new();
            self.context(Some(d), &mut drawn);
            let items = self.nest(drawn);
            out.extend(group(opacity, items).map(|g| (None, g)));
        } else {
            self.context(Some(d), out);
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
        if let Some(i) = i {
            self.background(i, out);
        }
        self.content(i, out);
    }

    /// `i`'s in-flow content: the backgrounds of its blocks, then text and atomic items, then the
    /// outlines of its blocks and its own.
    fn content(&self, i: Option<usize>, out: &mut Drawn) {
        self.backgrounds(i, out);
        self.foreground(i, out);
        self.outlines(i, out);
        if let Some(i) = i {
            self.outline(i, out);
        }
    }

    /// The backgrounds of the blocks in `i`.
    fn backgrounds(&self, i: Option<usize>, out: &mut Drawn) {
        for c in self.children(i) {
            if !self.layered(c) && !self.atomic(c) {
                self.background(c, out);
                self.backgrounds(Some(c), out);
            }
        }
    }

    /// The outlines of the blocks in `i`.
    fn outlines(&self, i: Option<usize>, out: &mut Drawn) {
        for c in self.children(i) {
            if !self.layered(c) && !self.atomic(c) {
                self.outline(c, out);
                self.outlines(Some(c), out);
            }
        }
    }

    fn foreground(&self, i: Option<usize>, out: &mut Drawn) {
        if let Some(i) = i {
            self.text(i, out);
        }
        for c in self.children(i) {
            if self.atomic(c) {
                self.whole(Some(c), out);
            } else if !self.layered(c) {
                self.foreground(Some(c), out);
            }
        }
    }

    /// `i`'s outer shadows, background, inner shadows and border, each over the one before.
    fn background(&self, i: usize, out: &mut Drawn) {
        let s = &self.styles[i];
        let rect = snap(self.layout.boxes[i].border);
        if !s.visible || rect.width <= 0.0 || rect.height <= 0.0 {
            return;
        }
        let radii = self.radii(i, rect);
        let clip = self.clips[i].last().copied();
        let widths = self.widths(i);
        let colors = s.border_colors.map(work);
        let (inside, inner_radii) = inner(rect, radii, widths);
        // The first shadow is on top.
        let (inset, outer): (Vec<&Shadow>, _) = s.shadows.iter().rev().partition(|s| s.inset);
        let opaque = s.background[3] >= 1.0;
        for shadow in outer {
            out.extend(outer_shadow(shadow, rect, radii, opaque).map(|item| (clip, item)));
        }
        let paddings = self.paddings(i);
        let content = inner(
            rect,
            radii,
            std::array::from_fn(|k| widths[k] + paddings[k]),
        );
        // The background would show at the antialiased edge of a rounded border: under four
        // opaque sides the border box it shows in shrinks by half their widths, as in Chrome.
        let shrunk = match (0..4).all(|k| widths[k] > 0.0 && colors[k][3] >= 1.0) {
            true if radii != SQUARE => inner(rect, radii, widths.map(|w| w / 2.0)),
            _ => (rect, radii),
        };
        let shown = |a: Area| {
            let (r, radii) = match a {
                Area::BorderBox => shrunk,
                Area::PaddingBox => (inside, inner_radii),
                Area::ContentBox => content,
            };
            let r = snap(r);
            (r.width > 0.0 && r.height > 0.0).then_some((r, radii))
        };
        let placed = |a: Area| match a {
            Area::BorderBox => rect,
            Area::PaddingBox => inside,
            Area::ContentBox => content.0,
        };
        // The color shows where the last layer does.
        let last = s.layers.last().map_or(Area::BorderBox, |l| l.clip);
        if let (true, Some((rect, radii))) = (s.background[3] > 0.0, shown(last)) {
            let color = work(s.background);
            out.push((clip, Item::Rect { rect, radii, color }));
        }
        // Images over the color, the first on top, repeated out to the area they show in.
        for layer in s.layers.iter().rev() {
            let Image::Gradient(g) = &layer.image else {
                continue;
            };
            let Some(((rect, radii), tiles)) =
                shown(layer.clip).zip(tiles(layer, placed(layer.origin)))
            else {
                continue;
            };
            let ramp = gradient::ramp(g, [tiles.first.width, tiles.first.height]);
            out.push((
                clip,
                Item::Gradient {
                    rect,
                    radii,
                    tiles,
                    ramp,
                },
            ));
        }
        for shadow in inset {
            out.extend(inner_shadow(shadow, inside, inner_radii).map(|item| (clip, item)));
        }
        if (0..4).any(|k| widths[k] > 0.0 && colors[k][3] > 0.0) {
            let border = Item::Ring {
                rect,
                radii,
                inner: inside,
                inner_radii,
                colors,
            };
            out.push((clip, border));
        }
    }

    fn widths(&self, i: usize) -> [f32; 4] {
        self.styles[i].border.map(|w| w as f32)
    }

    /// `i`'s padding: top, right, bottom and left.
    fn paddings(&self, i: usize) -> [f32; 4] {
        let (p, c) = (self.layout.boxes[i].padding, self.layout.boxes[i].content);
        [
            c.y - p.y,
            p.x + p.width - c.x - c.width,
            p.y + p.height - c.y - c.height,
            c.x - p.x,
        ]
    }

    /// `i`'s corner radii in `rect`, fitting it.
    fn radii(&self, i: usize, rect: Rect) -> Radii {
        fit(self.corners(i), rect)
    }

    /// `i`'s corner radii, their percentages of its border box.
    fn corners(&self, i: usize) -> Radii {
        let b = self.layout.boxes[i].border;
        (self.styles[i].radii.each_ref())
            .map(|[x, y]| corner([x.resolve(b.width), y.resolve(b.height)]))
    }

    /// `i`'s outline: a ring `outline-offset` outside its border box, with its corners rounded
    /// as the border box's are, grown by as much.
    fn outline(&self, i: usize, out: &mut Drawn) {
        let s = &self.styles[i];
        let (width, color) = (s.outline as f32, work(s.outline_color));
        if !s.visible || width <= 0.0 || color[3] <= 0.0 {
            return;
        }
        let by = s.outline_offset as f32 + width;
        let rect = outset(snap(self.layout.boxes[i].border), by);
        if rect.width <= 0.0 || rect.height <= 0.0 {
            return;
        }
        let radii = fit(grown(self.corners(i), by), rect);
        let (inside, inner_radii) = inner(rect, radii, [width; 4]);
        let outline = Item::Ring {
            rect,
            radii,
            inner: inside,
            inner_radii,
            colors: [color; 4],
        };
        out.push((self.clips[i].last().copied(), outline));
    }

    fn text(&self, i: usize, out: &mut Drawn) {
        let Some(lines) = &self.layout.lines[i] else {
            return;
        };
        let color = work(self.styles[i].color);
        if !self.styles[i].visible || color[3] <= 0.0 {
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
        self.nest_from(self.chained(drawn), 0)
    }

    /// Items with all the boxes they are clipped by, outermost first.
    fn chained(&self, drawn: Drawn) -> Vec<(Vec<usize>, Item)> {
        (drawn.into_iter())
            .map(|(clip, item)| match clip {
                None => (Vec::new(), item),
                Some(c) => ([&self.clips[c][..], &[c]].concat(), item),
            })
            .collect()
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
        let border = snap(self.layout.boxes[i].border);
        let (b, inner_radii) = inner(border, self.radii(i, border), self.widths(i));
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
        let radii = match o.x != Overflow::Visible && o.y != Overflow::Visible {
            true => inner_radii,
            false => SQUARE,
        };
        if radii == SQUARE {
            let inside = |item: &Item| match item {
                Item::Rect { rect: r, .. }
                | Item::Ring { rect: r, .. }
                | Item::Gradient { rect: r, .. } => within(*r, rect),
                _ => false,
            };
            if items.iter().all(inside) {
                out.extend(items);
                return;
            }
            if let [
                Item::Rect {
                    rect: r,
                    radii: SQUARE,
                    color,
                },
            ] = items[..]
            {
                out.extend(overlap(r, rect).map(|rect| Item::Rect {
                    rect,
                    radii: SQUARE,
                    color,
                }));
                return;
            }
        }
        out.push(Item::Clip { rect, radii, items });
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

/// `items` as one layer at `opacity`. A layer of one rectangle or ring is it at the opacity,
/// and a layer of one layer is that layer at both opacities.
fn group(opacity: f32, items: Vec<Item>) -> Option<Item> {
    if opacity <= 0.0 || items.is_empty() {
        return None;
    }
    Some(match <[Item; 1]>::try_from(items) {
        Ok([Item::Rect { rect, radii, color }]) => Item::Rect {
            rect,
            radii,
            color: color.map(|c| c * opacity),
        },
        Ok(
            [
                Item::Ring {
                    rect,
                    radii,
                    inner,
                    inner_radii,
                    colors,
                },
            ],
        ) => Item::Ring {
            rect,
            radii,
            inner,
            inner_radii,
            colors: colors.map(|c| c.map(|v| v * opacity)),
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
