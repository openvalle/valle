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
//! in it. Opacity, a blend, isolation, a transform, a filter, a clip path, a mask, a `<Mask>`, a
//! fixed position, and a z-index on a positioned box or a flex or grid item make a stacking
//! context. One with an opacity or a blend paints as a layer, and so does one with a box
//! blending in it: CSS isolates every stacking context, so a box blends only with what its
//! stacking context paints under it. A filtered one's items go in its filter, then over its
//! backdrop, then through its clip path, its mask and its `<Mask>` source, which take all it
//! paints, and a transformed one's in its transform around them, which takes its px, those of
//! the untransformed layout, to its parent's; a transform that flattens the plane hides it. A
//! `<Mask>` source paints alone, as the canvas does. Tree order puts a flex or grid container's
//! children in the order it lays them out. A hidden box (`visibility`) paints nothing of its
//! own, but what it contains still paints if visible. A box's background, its color and then its
//! images, the first on top, paints over its outer shadows, and its inner shadows and border over
//! the background; it is a layer of its own when an image blends, which blends with what is
//! under it in the background alone. Text paints run by run its underlines and overlines, its
//! glyphs and their stroke, then its line-throughs, each over its shadows.
//!
//! A box that clips what overflows it clips what it contains to its padding box, with its
//! corners rounded: its content and in-flow descendants, and the positioned ones whose
//! containing block is it or inside it. Layers group what they paint, but clips go on each item,
//! so a positioned box in a layer under a clip that does not contain it shows outside the clip. A
//! transformed or filtered box contains its positioned descendants, so the clips around it are
//! the same for all it paints, and go around its transform and filter.
//!
//! Boxes paint with their edges on whole pixels, as Chrome paints them, so a layout that lands
//! between pixels still has crisp edges.

mod borders;
mod cache;
mod dash;
mod gradient;
mod matrix;
mod offset;
mod scene;
mod shape;
pub use cache::Cache;
mod svg;

use std::rc::Rc;

use taffy::{Display, Overflow, Position};

pub use gradient::{Geometry, Ramp, Segment, Spread};
pub use matrix::{Matrix, Matrix4};

use crate::image::Picture;
use crate::layout::{Boxes, Layout, Natural, Rect, ordered};
use crate::path::Path;
use crate::shader::Program;
use crate::style::background::{Area, Composite, ImageSize, Layer, Repeat};
use crate::style::color::Rgba;
use crate::style::filter;
use crate::style::gradient::Image;
use crate::style::transform::Function;
use crate::style::{Blend, DecorationStyle, Fit, PaintPart, Shadow, Style};
use crate::svg::Element;
use crate::template::{Kind, Template};
use crate::text::{Glyph, Stripe};
use crate::transition;
use svg::Drawing;

use borders::{corner, fit, grown, inner, inner_shadow, lines, outer_shadow, outset, rings};

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
    /// The scene's image `image` filling a rectangle with its corners rounded by `radii`, in
    /// `tiles`; read at its nearest px if `pixelated`.
    Image {
        rect: Rect,
        radii: Radii,
        tiles: Tiles,
        image: u32,
        pixelated: bool,
    },
    /// Glyphs of `Text::faces()[face]` at `size` px: glyph ids at their baseline origins. With a
    /// `stroke` above 0, a stroke that many px wide along their edges instead.
    Glyphs {
        face: u32,
        size: f32,
        color: [f32; 4],
        stroke: f32,
        glyphs: Vec<(u32, [f32; 2])>,
    },
    /// A path filled in a color where it winds around, or with `even_odd` where it winds an odd
    /// number of times.
    Path {
        path: Path,
        even_odd: bool,
        color: [f32; 4],
    },
    /// Items composited onto what is under them as one layer, at `opacity`, their colors mixed
    /// with those under them by `blend`.
    Group {
        opacity: f32,
        blend: Blend,
        items: Vec<Item>,
    },
    /// Items in px that `matrix` takes to the px around them.
    Transform { matrix: Matrix, items: Vec<Item> },
    /// Items drawn as one image through `filters`, one after another, onto what is under them.
    Filter {
        filters: Vec<Filter>,
        items: Vec<Item>,
    },
    /// Layers of items that a shader authors wrote reads as the images they are named by, its
    /// `content`, or a transition's two children, and what is under the box as its `backdrop`,
    /// as a `Backdrop` has it: the shader paints the box `rect`, grown by `outset` px on each
    /// side, with its uniform `block`; and the frame's images of `inputs` by their names.
    Shader {
        program: Rc<Program>,
        block: Rc<[u8]>,
        inputs: Vec<(Rc<str>, u32)>,
        rect: Rect,
        outset: f32,
        layers: Vec<(&'static str, Vec<Item>)>,
    },
    /// What is under a box's layer, in its border box `rect`, its edges mirrored, through
    /// `filters`, drawn inside the rectangle with its corners rounded by `radii`. Under it is all
    /// drawn since the nearest layer that starts empty for what is in it, and those are the
    /// layers of a box with an opacity, a blend or a filter of it or of what is under it.
    Backdrop {
        filters: Vec<Filter>,
        rect: Rect,
        radii: Radii,
    },
    /// Items where a x + b y + c ≥ 0 for `line` [a, b, c], with a hard edge: the part of a
    /// plane of a 3D scene on one side of another plane, so that two parts share each pixel.
    Cut { line: [f32; 3], items: Vec<Item> },
    /// Items shown only inside a rectangle with its corners rounded by `radii`.
    Clip {
        rect: Rect,
        radii: Radii,
        items: Vec<Item>,
    },
    /// Items as one layer, each pixel of it as much as the alpha `mask` paints there: a box's
    /// clip path. Like a group's, the layer starts empty for what is in it.
    Mask { mask: Vec<Item>, items: Vec<Item> },
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

/// A filter on an image.
#[derive(Clone, Debug, PartialEq)]
pub enum Filter {
    /// A Gaussian blur of this standard deviation, px.
    Blur(f32),
    /// Unpremultiplied colors through 4 × 5 matrices, one after another, each result clamped to
    /// [0, 1]: the factors of red, green, blue and alpha in each of them, then their offsets.
    Colors(Vec<[f32; 20]>),
    /// The image's alpha in a working color, moved by `offset` px, blurred by a standard
    /// deviation of `sigma` px, under the image.
    Shadow {
        offset: [f32; 2],
        sigma: f32,
        color: [f32; 4],
    },
    /// The image's luminance, of its working values with Rec. 709's weights as Chrome takes it,
    /// times its alpha, as the alpha of black: what a mask layer shows by its luminance.
    Luminance,
    /// Its red moved on by half this many px along x and its blue back by half, its alpha the
    /// most of the three's.
    Aberration(f32),
    /// Each point of the box `frame` shows the one as much farther from its center as
    /// 1 + k1 r² + k2 r⁴, r its distance over the half diagonal's; nothing shows out of it.
    Lens { k: [f32; 2], frame: Rect },
    /// Noise of a pattern `seed` picks, cells `size` px across from `origin`, added to the
    /// colors' encoded values by up to `amount` of how far each is from 0 or 1, whichever is
    /// nearer.
    Grain {
        seed: u32,
        amount: f32,
        size: f32,
        origin: [f32; 2],
    },
    /// Blurred along each ray from `center` by up to `amount` px either way, and never more
    /// than half of the way to it.
    Radial { center: [f32; 2], amount: f32 },
    /// Blurred evenly along this trail, px, centered on each point.
    Motion([f32; 2]),
    /// Each point shows the one `scale` px times two noises less a half away along x and y:
    /// noises of a pattern `seed` picks, `frequency` cycles a px along x and y from `origin`, of
    /// `octaves`, fractal or `turbulence`, as SVG's feTurbulence makes them.
    Displacement {
        seed: u32,
        frequency: [f32; 2],
        scale: f32,
        octaves: u32,
        turbulence: bool,
        origin: [f32; 2],
    },
    /// The colors of what is brighter than `threshold`, eased in over `knee` either side of it,
    /// blurred by a Gaussian of half `radius` px, and added `intensity` times.
    Bloom {
        radius: f32,
        intensity: f32,
        threshold: f32,
        knee: f32,
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

/// A background image's tiles in `area`, if it has a size. A resource's image has a natural
/// size, its picture in `pictures` giving it: `auto` on both axes is that size, on one the size
/// the other gives in its proportions, and `contain` and `cover` scale it to fit the area or
/// cover it.
/// Gradients have no size of their own: `auto`, `contain` and `cover` are the area's. A rounded
/// axis fits a whole number of tiles, and the other, if `auto`, keeps the tile's proportions; a
/// spaced one fits as many whole tiles as it can from edge to edge, or a single one where the
/// position puts it.
fn tiles(layer: &Layer, area: Rect, pictures: &[Picture]) -> Option<Tiles> {
    let room = [area.width, area.height];
    let natural = match layer.image {
        Image::Url(r) => match pictures[r as usize].natural() {
            Natural::Size(size) => Some(size),
            Natural::Ratio(_) => None,
        },
        _ => None,
    };
    let (asked, auto) = match (&layer.size, natural) {
        (ImageSize::Sized(s), _) => {
            let given = [0, 1].map(|k| s[k].as_ref().map(|l| l.resolve(room[k])));
            let asked = match (given, natural) {
                ([Some(w), Some(h)], _) => [w, h],
                (_, None) => [0, 1].map(|k| given[k].unwrap_or(room[k])),
                ([None, None], Some(n)) => n,
                ([Some(w), None], Some([nw, nh])) => [w, w * nh / nw],
                ([None, Some(h)], Some([nw, nh])) => [h * nw / nh, h],
            };
            (asked, [s[0].is_none(), s[1].is_none()])
        }
        (_, None) => (room, [false; 2]),
        (fit, Some([w, h])) => {
            let [kw, kh] = [room[0] / w, room[1] / h];
            let k = match fit {
                ImageSize::Contain => kw.min(kh),
                _ => kw.max(kh),
            };
            ([w * k, h * k], [false; 2])
        }
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

/// A layer's image in `rect`, its corners rounded by `radii`, in `tiles`; nothing for `none`.
/// An SVG file is no background yet: a frame with one fails before it paints.
fn filled(
    image: &Image,
    rect: Rect,
    radii: Radii,
    tiles: Tiles,
    pictures: &[Picture],
    pixelated: bool,
) -> Option<Item> {
    Some(match image {
        Image::None => return None,
        Image::Gradient(g) => Item::Gradient {
            rect,
            radii,
            tiles,
            ramp: gradient::ramp(g, [tiles.first.width, tiles.first.height]),
        },
        Image::Url(r) => match pictures[*r as usize] {
            Picture::Image { image, .. } => Item::Image {
                rect,
                radii,
                tiles,
                image,
                pixelated,
            },
            Picture::Svg(_) | Picture::None => return None,
        },
    })
}

/// Items, each with the innermost box whose clip it is under.
type Drawn = Vec<(Option<usize>, Item)>;

/// A mask inverted: its alpha from 1, as the alpha of black.
const INVERTED: [f32; 20] = [
    0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0,
    1.0,
];

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

/// Filter functions of a box with the border box `frame` as filters, those of colors one after
/// another in one.
fn filters(functions: &[filter::Function], frame: Rect) -> Vec<Filter> {
    let mut out = Vec::new();
    for f in functions {
        let next = match f {
            filter::Function::Blur(sigma) => Filter::Blur(*sigma as f32),
            filter::Function::DropShadow {
                offset,
                blur,
                color,
            } => Filter::Shadow {
                offset: offset.map(|v| v as f32),
                sigma: *blur as f32,
                color: work(*color),
            },
            filter::Function::ChromaticAberration(offset) => Filter::Aberration(*offset as f32),
            filter::Function::LensDistortion(k) => Filter::Lens {
                k: k.map(|v| v as f32),
                frame,
            },
            filter::Function::FilmGrain { seed, amount, size } => Filter::Grain {
                seed: *seed,
                amount: *amount as f32,
                size: *size as f32,
                origin: [frame.x, frame.y],
            },
            filter::Function::MotionBlur(trail) => Filter::Motion(trail.map(|v| v as f32)),
            filter::Function::RadialBlur { center, amount } => Filter::Radial {
                center: [frame.x + center[0] as f32, frame.y + center[1] as f32],
                amount: *amount as f32,
            },
            filter::Function::Displacement {
                seed,
                frequency,
                scale,
                octaves,
                turbulence,
            } => Filter::Displacement {
                seed: *seed,
                frequency: frequency.map(|v| v as f32),
                scale: *scale as f32,
                octaves: *octaves,
                turbulence: *turbulence,
                origin: [frame.x, frame.y],
            },
            filter::Function::Bloom {
                radius,
                intensity,
                threshold,
                knee,
            } => Filter::Bloom {
                radius: *radius as f32,
                intensity: *intensity as f32,
                threshold: *threshold as f32,
                knee: *knee as f32,
            },
            f => {
                let Some(m) = colors(f) else { continue };
                if let Some(Filter::Colors(list)) = out.last_mut() {
                    list.push(m);
                    continue;
                }
                Filter::Colors(vec![m])
            }
        };
        out.push(next);
    }
    out
}

/// A filter function of colors as its matrix, as `Filter::Colors` has them (Filter Effects
/// Level 1); `None` for any other.
fn colors(f: &filter::Function) -> Option<[f32; 20]> {
    use filter::Function as F;
    // Factors of red, green and blue for each, then an offset added to each.
    let rgb = |m: [[f64; 3]; 3], add: f64| {
        let mut out = [0.0; 20];
        for (r, row) in m.iter().enumerate() {
            out[r * 4..r * 4 + 3].copy_from_slice(&row.map(|v| v as f32));
            out[16 + r] = add as f32;
        }
        out[15] = 1.0;
        out
    };
    let scaled = |k: f64| [[k, 0.0, 0.0], [0.0, k, 0.0], [0.0, 0.0, k]];
    // Toward gray by `k`, from the weights at `k` = 1 and those of each channel.
    let toward = |w: [[f64; 3]; 3], k: f64| -> [[f64; 3]; 3] {
        std::array::from_fn(|r| {
            std::array::from_fn(|c| w[r][c] + (f64::from(u8::from(r == c)) - w[r][c]) * (1.0 - k))
        })
    };
    let luma = [0.2126, 0.7152, 0.0722];
    let lum = [0.213, 0.715, 0.072];
    Some(match *f {
        F::Brightness(a) => rgb(scaled(a), 0.0),
        F::Contrast(a) => rgb(scaled(a), 0.5 - 0.5 * a),
        F::Invert(a) => rgb(scaled(1.0 - 2.0 * a), a),
        F::Opacity(a) => {
            let mut m = rgb(scaled(1.0), 0.0);
            m[15] = a as f32;
            m
        }
        F::Grayscale(a) => rgb(toward([luma; 3], a), 0.0),
        F::Sepia(a) => {
            let sepia = [
                [0.393, 0.769, 0.189],
                [0.349, 0.686, 0.168],
                [0.272, 0.534, 0.131],
            ];
            rgb(toward(sepia, a), 0.0)
        }
        F::Saturate(a) => rgb(toward([lum; 3], 1.0 - a), 0.0),
        F::HueRotate(degrees) => {
            let (sin, cos) = degrees.to_radians().sin_cos();
            let turned = [
                [
                    0.787 * cos - 0.213 * sin,
                    -0.715 * cos - 0.715 * sin,
                    -0.072 * cos + 0.928 * sin,
                ],
                [
                    -0.213 * cos + 0.143 * sin,
                    0.285 * cos + 0.140 * sin,
                    -0.072 * cos - 0.283 * sin,
                ],
                [
                    -0.213 * cos - 0.787 * sin,
                    -0.715 * cos + 0.715 * sin,
                    0.928 * cos + 0.072 * sin,
                ],
            ];
            rgb(
                std::array::from_fn(|r| std::array::from_fn(|c| lum[c] + turned[r][c])),
                0.0,
            )
        }
        F::Blur(_)
        | F::DropShadow { .. }
        | F::ChromaticAberration(_)
        | F::LensDistortion(_)
        | F::FilmGrain { .. }
        | F::RadialBlur { .. }
        | F::MotionBlur(_)
        | F::Displacement { .. }
        | F::Bloom { .. } => return None,
    })
}

/// A computed color as a working color.
pub fn work(c: Rgba) -> [f32; 4] {
    let a = c[3];
    [c[0] * a, c[1] * a, c[2] * a, a].map(|v| v as f32)
}

/// The frame's display list, from its template, computed styles and layout, its Images showing
/// the resources' `pictures`.
pub fn paint(
    template: &Template,
    styles: &[Style],
    layout: &Layout,
    pictures: &[Picture],
) -> Vec<Item> {
    Cache::default().paint(template, styles, layout, pictures)
}

pub fn drawn(
    template: &Template,
    styles: &[Style],
    layout: &Layout,
    pictures: &[Picture],
) -> Vec<(u32, Vec<Item>)> {
    Cache::default().drawn(template, styles, layout, pictures)
}

impl Cache {
    pub fn paint(
        &self,
        template: &Template,
        styles: &[Style],
        layout: &Layout,
        pictures: &[Picture],
    ) -> Vec<Item> {
        let painter = Painter::new(template, styles, layout, pictures, self);
        let mut out = Vec::new();
        painter.context(None, &mut out);
        painter.nest(out)
    }

    /// The pictures boxes of the frame draw, in the order to draw them in, as each reads only those
    /// before it: those in a box before it, and a box's before its later siblings'. Each is its
    /// image, and the display list of the canvas the box alone paints; one hidden, or in a hidden
    /// box, is left out, as nothing shown reads it.
    pub fn drawn(
        &self,
        template: &Template,
        styles: &[Style],
        layout: &Layout,
        pictures: &[Picture],
    ) -> Vec<(u32, Vec<Item>)> {
        if template.pictures.is_empty() {
            return Vec::new();
        }
        let painter = Painter::new(template, styles, layout, pictures, self);
        // The last box in each box, in preorder.
        let n = template.nodes.len();
        let mut last: Vec<usize> = (0..n).collect();
        for i in (0..n).rev() {
            for &c in &template.nodes[i].children {
                last[i] = last[i].max(last[c as usize]);
            }
        }
        let mut order: Vec<(u32, u32)> = template.pictures.clone();
        order.sort_by_key(|&(node, _)| (last[node as usize], std::cmp::Reverse(node)));
        let shown = |mut i: usize| loop {
            if styles[i].layout.display == Display::None {
                return false;
            }
            match painter.parents[i] {
                Some(p) => i = p,
                None => return true,
            }
        };
        (order.into_iter())
            .filter(|&(node, _)| shown(node as usize))
            .filter_map(|(node, slot)| {
                let Picture::Image { image, .. } = pictures[slot as usize] else {
                    return None;
                };
                let mut out = Vec::new();
                painter.layer(node as usize, &mut out);
                Some((image, painter.nest(out)))
            })
            .collect()
    }
}

impl<'a> Painter<'a> {
    fn new(
        template: &'a Template,
        styles: &'a [Style],
        layout: &'a Layout,
        pictures: &'a [Picture],
        cache: &'a Cache,
    ) -> Painter<'a> {
        let n = template.nodes.len();
        let parents = parents(template);
        let mut drawn = vec![false; n];
        for &(node, _) in &template.pictures {
            drawn[node as usize] = true;
        }
        let mut painter = Painter {
            cache,
            template,
            styles,
            layout,
            pictures,
            spaces: spaces(template, styles, layout, 0..template.nodes.len()),
            parents,
            clips: Vec::with_capacity(n),
            drawing: Drawing::new(template, styles, &[], cache),
            drawn,
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
        painter
    }
}

/// Each box's transform from its px to the canvas's: its ancestors' and its own.
pub fn transforms(template: &Template, styles: &[Style], layout: &Layout) -> Vec<Matrix> {
    transforms_of(template, styles, layout, 0..template.nodes.len())
}

/// The transforms of the boxes `of` alone, in preorder, each one's ancestors among them, a
/// `<Mask>`'s source's its mask; the others' are left the identity. They read only the styles
/// and boxes of those.
pub fn transforms_of(
    template: &Template,
    styles: &[Style],
    layout: &Layout,
    of: impl Iterator<Item = usize> + Clone,
) -> Vec<Matrix> {
    let spaces = spaces(template, styles, layout, of.clone());
    let mut out = vec![Matrix::IDENTITY; template.nodes.len()];
    // What the plane each box is flattened onto goes to on the canvas.
    let mut onto = out.clone();
    // In preorder, so a parent's comes first.
    for i in of {
        let node = &template.nodes[i];
        out[i] = onto[i].times(spaces[i].flatten());
        for &c in &node.children {
            onto[c as usize] = match styles[i].keeps_3d() {
                true => onto[i],
                false => out[i],
            };
        }
        // A mask's source is on its plane.
        if let Some(source) = styles[i].source {
            onto[source.root] = out[i];
        }
    }
    out
}

/// Each box's parent; `None` for a root.
fn parents(template: &Template) -> Vec<Option<usize>> {
    let mut parents = vec![None; template.nodes.len()];
    for (i, node) in template.nodes.iter().enumerate() {
        for &c in &node.children {
            parents[c as usize] = Some(i);
        }
    }
    parents
}

/// `i`'s containing block: a fixed box's is the nearest box around it that contains all, or the
/// canvas, an absolute box's the nearest such or positioned one, and any other's its parent.
/// `None` is the canvas.
fn container(parents: &[Option<usize>], styles: &[Style], i: usize) -> Option<usize> {
    let all = |p: usize| styles[p].contains_all();
    let positioned = |p: usize| styles[p].layout.position != Position::Static;
    let mut around = std::iter::successors(parents[i], |&p| parents[p]);
    match styles[i].layout.position {
        Position::Fixed => around.find(|&p| all(p)),
        Position::Absolute => around.find(|&p| positioned(p) || all(p)),
        _ => parents[i],
    }
}

/// The box `a` of a box with the style `s` and the boxes `b`, its border box snapped, and its
/// corners' radii.
fn area(s: &Style, b: &Boxes, a: Area) -> (Rect, Radii) {
    let border = snap(b.border);
    let corners = (s.radii.each_ref())
        .map(|[x, y]| corner([x.resolve(b.border.width), y.resolve(b.border.height)]));
    let radii = fit(corners, border);
    let widths = s.border.map(|w| w as f32);
    match a {
        Area::BorderBox | Area::Text => (border, radii),
        Area::PaddingBox => inner(border, radii, widths),
        Area::ContentBox => {
            let (p, c) = (b.padding, b.content);
            let paddings = [
                c.y - p.y,
                p.x + p.width - c.x - c.width,
                p.y + p.height - c.y - c.height,
                c.x - p.x,
            ];
            inner(
                border,
                radii,
                std::array::from_fn(|k| widths[k] + paddings[k]),
            )
        }
    }
}

/// Each box's transform in space, in its parent's perspective, onto the plane it is flattened
/// onto: its parent's, or in a 3D scene, the plane its root is flattened onto, through the
/// transforms of the boxes between; of the boxes `of` alone, as `transforms_of` takes them.
fn spaces(
    template: &Template,
    styles: &[Style],
    layout: &Layout,
    of: impl Iterator<Item = usize>,
) -> Vec<Matrix4> {
    let n = template.nodes.len();
    let parents = parents(template);
    let mut out = vec![Matrix4::IDENTITY; n];
    // What its parent puts before its own: the parent's perspective, and in a 3D scene the
    // parent's space.
    let mut seen = vec![None; n];
    let mut base = out.clone();
    // In preorder, so a parent's comes first.
    for i in of {
        let node = &template.nodes[i];
        let border = layout.boxes[i].border;
        // The box of its containing block a motion path is in.
        let within = (styles[i].motion.as_ref()).map(|m| match container(&parents, styles, i) {
            Some(c) => area(&styles[c], &layout.boxes[c], m.area),
            None => {
                let [width, height] = layout.canvas;
                let canvas = Rect {
                    x: 0.0,
                    y: 0.0,
                    width,
                    height,
                };
                (canvas, SQUARE)
            }
        });
        let own = transform(&styles[i], border, seen[i], within);
        out[i] = base[i].times(own.or(seen[i]).unwrap_or(Matrix4::IDENTITY));
        let perspective = perspective(&styles[i], border);
        let keeps = styles[i].keeps_3d();
        for &c in &node.children {
            seen[c as usize] = perspective;
            base[c as usize] = if keeps { out[i] } else { Matrix4::IDENTITY };
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
/// perspective `seen`: its functions, the last first, around its origin, with its motion path's
/// move, in the box `within`, after `translate`, `rotate` and `scale`. In space: flattened, it
/// is the transform onto its parent's plane. `None` for `none`.
fn transform(
    s: &Style,
    border: Rect,
    seen: Option<Matrix4>,
    within: Option<(Rect, Radii)>,
) -> Option<Matrix4> {
    if !s.transformed() {
        return None;
    }
    let b = snap(border);
    let size = [b.width, b.height];
    let origin = |r: Rect| {
        let size = [r.width, r.height];
        [0, 1].map(|k| [r.x, r.y][k] + s.transform_origin[k].resolve(size[k]))
    };
    let [x, y] = origin(b).map(f64::from);
    let z = s.transform_origin_z;
    let function = |m: Matrix4, f: &Function| {
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
    };
    let (individual, functions) = s.transform.split_at(s.individual);
    let mut m = individual.iter().fold(Matrix4::IDENTITY, function);
    // As Chrome moves it: from its border box as laid out, not snapped.
    if let (Some(motion), Some(within)) = (&s.motion, within) {
        let moved = offset::moved(motion, border, origin(border), within);
        m = (m.times(Matrix4::translate(-x, -y, 0.0)))
            .times(moved)
            .times(Matrix4::translate(x, y, 0.0));
    }
    let m = functions.iter().fold(m, function);
    let m = Matrix4::translate(x, y, z)
        .times(m)
        .times(Matrix4::translate(-x, -y, -z));
    Some(seen.unwrap_or(Matrix4::IDENTITY).times(m))
}

/// Boxes are template indices; `None` is the canvas, a block container without a background.
struct Painter<'a> {
    cache: &'a Cache,
    template: &'a Template,
    styles: &'a [Style],
    layout: &'a Layout,
    pictures: &'a [Picture],
    /// As `spaces` has them.
    spaces: Vec<Matrix4>,
    parents: Vec<Option<usize>>,
    /// The boxes whose clips each box is under, outermost first.
    clips: Vec<Vec<usize>>,
    /// What the frame's `<svg>`s draw.
    drawing: Drawing<'a>,
    /// Whether each box draws a picture, which it paints alone rather than where it is.
    drawn: Vec<bool>,
}

impl Painter<'_> {
    fn children(&self, i: Option<usize>) -> impl Iterator<Item = usize> {
        let list: Vec<usize> = match i {
            None => (self.template.roots.iter()).map(|&c| c as usize).collect(),
            // Its Spans paint with its text, and an <svg>'s elements in its drawing.
            Some(i)
                if matches!(
                    self.template.nodes[i].kind,
                    Kind::Text { .. } | Kind::Svg { .. }
                ) =>
            {
                Vec::new()
            }
            Some(i) => ordered(
                &self.template.nodes[i].children,
                &self.styles[i],
                self.styles,
            )
            .collect(),
        };
        (list.into_iter())
            .filter(|&c| self.styles[c].layout.display != Display::None && !self.drawn[c])
    }

    fn positioned(&self, i: usize) -> bool {
        self.styles[i].layout.position != Position::Static
    }

    /// The box whose clips `i` is under too: its containing block.
    fn container(&self, i: usize) -> Option<usize> {
        container(&self.parents, self.styles, i)
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
            || s.blend != Blend::Normal
            || s.isolate
            || s.clip.is_some()
            || !s.masks.is_empty()
            || s.source.is_some()
            || s.contains_all()
            || self.in_scene(i)
            || s.layout.position == Position::Fixed
            || self.drawn[i]
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
    /// context, in its transform if it has one and a layer if `grouped`, or a positioned box.
    fn layer(&self, d: usize, out: &mut Drawn) {
        let s = &self.styles[d];
        let mut blend = s.blend;
        if self.in_scene(d) {
            // It paints with its scene.
            return;
        } else if !self.stacking(d) {
            return self.whole(Some(d), out);
        } else if s.keeps_3d() {
            // Its planes group what they paint.
            return self.scene(d, out);
        }
        let mut drawn = Vec::new();
        if s.contains_all() {
            let space = self.spaces[d];
            let matrix = space.flatten();
            // Its back hides it and all in it, as they are on its plane.
            let back = !s.backface_visible && space.shows_back();
            if s.transformed() && (matrix.inverse().is_none() || back) {
                return;
            }
            let mut inside = Vec::new();
            match s.shader.as_ref().is_some_and(|s| s.transition) {
                // Its children paint into the shader, which paints over its background.
                true => self.background(d, &mut inside),
                false => self.context(Some(d), &mut inside),
            }
            // It contains all in it, so the clips around it go around all it paints.
            let mut items = self.nest_from(self.chained(inside), self.clips[d].len());
            items = self.shaded(d, items);
            if !s.filter.is_empty() && !items.is_empty() {
                let filters = filters(&s.filter, snap(self.layout.boxes[d].border));
                items = vec![Item::Filter { filters, items }];
            }
            (items, blend) = self.over_backdrop(d, items);
            items = self.masked(d, self.clipped(d, items));
            if s.transformed() && !items.is_empty() {
                items = vec![Item::Transform { matrix, items }];
            }
            let clip = self.clips[d].last().copied();
            drawn.extend(items.into_iter().map(|item| (clip, item)));
        } else {
            self.context(Some(d), &mut drawn);
            if s.clip.is_some() || !s.masks.is_empty() || s.source.is_some() {
                // With the clips in the mask, which takes all the box paints.
                let items = self.clipped(d, self.nest(std::mem::take(&mut drawn)));
                let items = self.masked(d, items);
                drawn = items.into_iter().map(|item| (None, item)).collect();
            }
        }
        if self.grouped(d) {
            // With the clips in the layer, so that it blends with all under it.
            let items = self.nest(drawn);
            out.extend(group(s.opacity as f32, blend, items).map(|g| (None, g)));
        } else {
            out.extend(drawn);
        }
    }

    /// Whether stacking context `d` paints as one layer: it has an opacity, blends, filters its
    /// backdrop or has a shader read it, or a box in it blends, which blends with what the
    /// context paints alone, as CSS isolates a stacking context.
    fn grouped(&self, d: usize) -> bool {
        let s = &self.styles[d];
        let reads = s
            .shader
            .as_ref()
            .is_some_and(|s| s.program.reads("backdrop"));
        s.opacity < 1.0 || s.blend != Blend::Normal || !s.backdrop.is_empty() || reads || {
            let mut layered = Vec::new();
            self.collect(Some(d), &mut layered);
            (layered.iter()).any(|&b| self.styles[b].blend != Blend::Normal)
        }
    }

    /// `items` as the content of `d`'s shader, if it is a `<ShaderLayer>`, which its corners
    /// round as they do a canvas, grown by its outset. A `<Transition>`'s shader reads its two
    /// children each as a layer of its own, and paints over `items`.
    fn shaded(&self, d: usize, items: Vec<Item>) -> Vec<Item> {
        let Some(shader) = &self.styles[d].shader else {
            return items;
        };
        let (layers, under) = match shader.transition {
            true => {
                let cells = (self.children(Some(d)).zip(transition::IMAGES)).map(|(c, name)| {
                    let mut drawn = Vec::new();
                    self.layer(c, &mut drawn);
                    (
                        name,
                        self.nest_from(self.chained(drawn), self.clips[d].len()),
                    )
                });
                (cells.collect(), items)
            }
            false => (vec![("content", items)], Vec::new()),
        };
        let rect = snap(self.layout.boxes[d].border);
        let by = shader.outset as f32;
        let inputs = (shader.inputs.iter())
            .filter_map(|(name, r)| match self.pictures[*r as usize] {
                Picture::Image { image, .. } => Some((name.clone(), image)),
                _ => None,
            })
            .collect();
        let shaded = Item::Shader {
            program: shader.program.clone(),
            block: shader.block.clone(),
            inputs,
            rect,
            outset: by,
            layers,
        };
        let radii = self.radii(d, rect);
        let shaded = match radii == SQUARE {
            true => shaded,
            false => Item::Clip {
                rect: outset(rect, by),
                radii: grown(radii, by),
                items: vec![shaded],
            },
        };
        under.into_iter().chain([shaded]).collect()
    }

    /// Blend `items` with `d`'s filtered backdrop, then composite the combined layer normally.
    /// Return its outer blend too, so the filtered backdrop is not blended with the page again.
    /// Even an empty filter region isolates the foreground from the page behind it.
    fn over_backdrop(&self, d: usize, items: Vec<Item>) -> (Vec<Item>, Blend) {
        let s = &self.styles[d];
        let rect = snap(self.layout.boxes[d].border);
        if s.backdrop.is_empty() {
            return (items, s.blend);
        }
        let backdrop = (rect.width > 0.0 && rect.height > 0.0).then(|| Item::Backdrop {
            filters: filters(&s.backdrop, rect),
            rect,
            radii: self.radii(d, rect),
        });
        let inside = (!items.is_empty()).then_some(Item::Group {
            opacity: 1.0,
            blend: s.blend,
            items,
        });
        (backdrop.into_iter().chain(inside).collect(), Blend::Normal)
    }

    /// `items` inside `d`'s clip path, if it has one: a shape in one of its boxes, in the px it
    /// paints in. Nothing shows where the shape has no area.
    fn clipped(&self, d: usize, items: Vec<Item>) -> Vec<Item> {
        let Some(clip) = &self.styles[d].clip else {
            return items;
        };
        if items.is_empty() {
            return items;
        }
        let (rect, radii) = area(&self.styles[d], &self.layout.boxes[d], clip.area);
        let mask = shape::mask(clip.shape.as_ref(), rect, radii);
        mask.map(|mask| Item::Mask {
            mask: vec![mask],
            items,
        })
        .into_iter()
        .collect()
    }

    /// `items` through `d`'s mask, if it has one: its layers' images, sized, placed and repeated
    /// as a background's are, each in its area rounded by the box's corners, and the first on
    /// top, going with those under it as its composite says. Each shows `items` by its alpha, or
    /// its luminance. Then through its `<Mask>` source,
    /// if it has one: what the source paints, by its alpha or its luminance, inverted or not, in
    /// the border box rounded by its corners.
    fn masked(&self, d: usize, items: Vec<Item>) -> Vec<Item> {
        let s = &self.styles[d];
        if (s.masks.is_empty() && s.source.is_none()) || items.is_empty() {
            return items;
        }
        let border = snap(self.layout.boxes[d].border);
        let radii = self.radii(d, border);
        let widths = self.widths(d);
        let paddings = self.paddings(d);
        let area = |a: Area| match a {
            Area::BorderBox | Area::Text => (border, radii),
            Area::PaddingBox => inner(border, radii, widths),
            Area::ContentBox => inner(
                border,
                radii,
                std::array::from_fn(|k| widths[k] + paddings[k]),
            ),
        };
        let image = |layer: &Layer| {
            let (rect, radii) = area(layer.clip);
            let rect = snap(rect);
            let tiles = tiles(layer, area(layer.origin).0, self.pictures)?;
            if rect.width <= 0.0 || rect.height <= 0.0 {
                return None;
            }
            let image = filled(&layer.image, rect, radii, tiles, self.pictures, s.pixelated)?;
            Some(match layer.luminance {
                true => Item::Filter {
                    filters: vec![Filter::Luminance],
                    items: vec![image],
                },
                false => image,
            })
        };
        // The box but where `items` show, which a filter cannot say outside what they paint.
        let outside = |items: Vec<Item>| Item::Mask {
            mask: vec![Item::Rect {
                rect: border,
                radii: Radii::default(),
                color: shape::SHOWN,
            }],
            items: vec![Item::Filter {
                filters: vec![Filter::Colors(vec![INVERTED])],
                items,
            }],
        };
        // The bottom layer has nothing to go with, whatever its composite.
        let mut layers = s.masks.iter().rev();
        let mut mask: Vec<Item> = layers.next().and_then(image).into_iter().collect();
        for layer in layers {
            let image = image(layer);
            match (layer.composite, image, mask.is_empty()) {
                (Composite::Add | Composite::Exclude, None, _) => {}
                (_, None, _) | (Composite::Intersect, _, true) => mask.clear(),
                (Composite::Add, Some(image), _) | (_, Some(image), true) => mask.push(image),
                (Composite::Subtract, Some(image), false) => {
                    mask = vec![Item::Mask {
                        mask: vec![outside(mask)],
                        items: vec![image],
                    }];
                }
                (Composite::Intersect, Some(image), false) => {
                    mask = vec![Item::Mask {
                        mask,
                        items: vec![image],
                    }];
                }
                (Composite::Exclude, Some(image), false) => {
                    let under = Item::Mask {
                        mask: vec![outside(vec![image.clone()])],
                        items: mask.clone(),
                    };
                    let over = Item::Mask {
                        mask: vec![outside(mask)],
                        items: vec![image],
                    };
                    // Their sum: what each lets through of a pixel leaves room for the other.
                    mask = vec![
                        under,
                        Item::Group {
                            opacity: 1.0,
                            blend: Blend::PlusLighter,
                            items: vec![over],
                        },
                    ];
                }
            }
        }
        let items = match (s.masks.is_empty(), mask.is_empty()) {
            (true, _) => items,
            (false, true) => return Vec::new(),
            (false, false) => vec![Item::Mask { mask, items }],
        };
        let Some(source) = s.source else {
            return items;
        };
        let mut drawn = Vec::new();
        self.context(Some(source.root), &mut drawn);
        let mut shown = self.nest(drawn);
        if source.luminance && !shown.is_empty() {
            shown = vec![Item::Filter {
                filters: vec![Filter::Luminance],
                items: shown,
            }];
        }
        let area = Item::Rect {
            rect: border,
            radii,
            color: shape::SHOWN,
        };
        let mask = match (source.invert, shown.is_empty()) {
            (false, true) => return Vec::new(),
            (false, false) => Item::Mask {
                mask: vec![area],
                items: shown,
            },
            (true, true) => area,
            (true, false) => Item::Mask {
                mask: vec![area],
                items: vec![Item::Filter {
                    filters: vec![Filter::Colors(vec![INVERTED])],
                    items: shown,
                }],
            },
        };
        vec![Item::Mask {
            mask: vec![mask],
            items,
        }]
    }

    /// Whether `i` is a plane of a 3D scene: its parent keeps its children in its space, and it
    /// has a transform, keeps its own children in its space too, or hides its back, which is
    /// where it is in the scene. As in Chrome, any other child paints on its parent's plane.
    fn in_scene(&self, i: usize) -> bool {
        let s = &self.styles[i];
        self.parents[i].is_some_and(|p| self.styles[p].keeps_3d())
            && (s.transformed() || s.keeps_3d() || !s.backface_visible)
    }

    /// A box that keeps its children in its space: the root of a 3D scene, whose planes paint
    /// back to front, each cut where another crosses it.
    fn scene(&self, d: usize, out: &mut Drawn) {
        let mut planes = Vec::new();
        self.planes(d, &mut planes);
        let rects: Vec<(Matrix4, Rect)> = planes.iter().map(|(m, r, _)| (*m, *r)).collect();
        let mut drawn = Vec::new();
        for (i, cuts) in scene::order(&rects) {
            let (space, _, items) = &planes[i];
            let items = (cuts.into_iter())
                .fold(items.clone(), |items, line| vec![Item::Cut { line, items }]);
            let item = Item::Transform {
                matrix: space.flatten(),
                items,
            };
            drawn.push(item);
        }
        // A 3D scene is a stacking context too: its blending planes see only what it paints.
        let drawn = if self.grouped(d) {
            group(1.0, Blend::Normal, drawn).into_iter().collect()
        } else {
            drawn
        };
        let clip = self.clips[d].last().copied();
        out.extend(drawn.into_iter().map(|item| (clip, item)));
    }

    /// The planes of a 3D scene from `e` on, each in space, with its border box and its items:
    /// its own but where it is edge on or shows its back and hides it, which leaves the planes
    /// in it, and those of its children in the scene.
    fn planes(&self, e: usize, out: &mut Vec<(Matrix4, Rect, Vec<Item>)>) {
        let s = &self.styles[e];
        let space = self.spaces[e];
        let back = !s.backface_visible && space.shows_back();
        if space.flatten().inverse().is_some() && !back {
            let mut drawn = Vec::new();
            self.context(Some(e), &mut drawn);
            let mut items = self.nest_from(self.chained(drawn), self.clips[e].len());
            items = self.shaded(e, items);
            if !s.filter.is_empty() && !items.is_empty() {
                let filters = filters(&s.filter, snap(self.layout.boxes[e].border));
                items = vec![Item::Filter { filters, items }];
            }
            let (items, blend) = self.over_backdrop(e, items);
            let mut items = self.masked(e, self.clipped(e, items));
            if self.grouped(e) {
                items = group(s.opacity as f32, blend, items).into_iter().collect();
            }
            if !items.is_empty() {
                out.push((space, self.layout.boxes[e].border, items));
            }
        }
        if s.keeps_3d() {
            for c in self.children(Some(e)) {
                if self.in_scene(c) {
                    self.planes(c, out);
                }
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
            self.image(i, out);
            self.svg(i, out);
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
        // opaque sides of lines the border box it shows in shrinks by half the widths of their
        // outer lines, as in Chrome, which shows it in the gap of a double line; dots and dashes
        // leave it whole.
        let lines = lines(widths, s.border_styles, s.border_colors);
        let half = lines
            .each_ref()
            .map(|l| l.first().map_or(0.0, |l| l.depth[1] / 2.0));
        let covered = |k: usize| widths[k] > 0.0 && colors[k][3] >= 1.0 && !lines[k].is_empty();
        let shrunk = match (0..4).all(covered) {
            true if radii != SQUARE => inner(rect, radii, half),
            _ => (rect, radii),
        };
        let shown = |a: Area| {
            let (r, radii) = match a {
                Area::BorderBox | Area::Text => shrunk,
                Area::PaddingBox => (inside, inner_radii),
                Area::ContentBox => content,
            };
            let r = snap(r);
            (r.width > 0.0 && r.height > 0.0).then_some((r, radii))
        };
        let placed = |a: Area| match a {
            Area::BorderBox | Area::Text => rect,
            Area::PaddingBox => inside,
            Area::ContentBox => content.0,
        };
        // A layer clipped to text shows through the box's text.
        let mut mask = None;
        let mut through = |area: Area, item: Item| match area {
            Area::Text => Item::Mask {
                mask: mask.get_or_insert_with(|| self.text_mask(i)).clone(),
                items: vec![item],
            },
            _ => item,
        };
        // The color shows where the last layer does.
        let last = s.layers.last().map_or(Area::BorderBox, |l| l.clip);
        let mut fill = Vec::new();
        if let (true, Some((rect, radii))) = (s.background[3] > 0.0, shown(last)) {
            let color = work(s.background);
            fill.push(through(last, Item::Rect { rect, radii, color }));
        }
        // Images over the color, the first on top, repeated out to the area they show in.
        let mut blends = false;
        for layer in s.layers.iter().rev() {
            let pictures = self.pictures;
            let Some(((rect, radii), tiles)) =
                shown(layer.clip).zip(tiles(layer, placed(layer.origin), pictures))
            else {
                continue;
            };
            let Some(image) = filled(&layer.image, rect, radii, tiles, pictures, s.pixelated)
            else {
                continue;
            };
            let image = through(layer.clip, image);
            fill.push(match layer.blend {
                Blend::Normal => image,
                blend => {
                    blends = true;
                    Item::Group {
                        opacity: 1.0,
                        blend,
                        items: vec![image],
                    }
                }
            });
        }
        // An image that blends blends with the color and the images under it alone.
        if blends {
            let items = std::mem::take(&mut fill);
            fill.push(Item::Group {
                opacity: 1.0,
                blend: Blend::Normal,
                items,
            });
        }
        out.extend(fill.into_iter().map(|item| (clip, item)));
        for shadow in inset {
            out.extend(inner_shadow(shadow, inside, inner_radii).map(|item| (clip, item)));
        }
        if (0..4).any(|k| widths[k] > 0.0 && colors[k][3] > 0.0) {
            let mut border = rings(rect, radii, widths, &lines);
            border.extend(dash::dashes(
                rect,
                radii,
                widths,
                s.border_styles,
                s.border_colors,
            ));
            out.extend(border.into_iter().map(|item| (clip, item)));
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
        let (styles, colors) = ([s.outline_style; 4], [s.outline_color; 4]);
        let mut outline = rings(rect, radii, [width; 4], &lines([width; 4], styles, colors));
        outline.extend(dash::dashes(rect, radii, [width; 4], styles, colors));
        let clip = self.clips[i].last().copied();
        out.extend(outline.into_iter().map(|item| (clip, item)));
    }

    /// Text as Chrome paints each run of it, in the style of the Text or Span it is in: its
    /// underlines and overlines, its glyphs and their stroke, and its line-throughs, each over
    /// its shadows.
    fn text(&self, i: usize, out: &mut Drawn) {
        let Some(lines) = &self.layout.lines[i] else {
            return;
        };
        let at = self.layout.boxes[i].content;
        // Text is content, which its own box clips.
        let clip = match self.clipping(i) {
            true => Some(i),
            false => self.clips[i].last().copied(),
        };
        for (k, run) in lines.runs.iter().enumerate() {
            let s = &self.styles[run.node as usize];
            if !s.visible {
                continue;
            }
            // Each part in its own colors, or all of it in a shadow's, moved by `by`.
            let stripes = |through: bool| {
                move |color: Option<[f32; 4]>, by: [f32; 2]| -> Vec<Item> {
                    (lines.decorations.iter())
                        .filter(|d| d.run == k as u32 && d.through == through)
                        .flat_map(|d| stripe(d, at, color.unwrap_or(work(d.color)), by))
                        .collect()
                }
            };
            let glyphs = &lines.glyphs[run.glyphs[0] as usize..run.glyphs[1] as usize];
            let size = s.font.size as f32;
            let drawn = |stroke: f32, color: [f32; 4], by: [f32; 2]| {
                faces(glyphs, at, size, color, stroke, by)
            };
            let fill = work(s.font.fill.unwrap_or(s.color));
            let stroke = match s.font.stroke > 0.0 && s.font.size > 0.0 {
                true => Some((
                    s.font.stroke as f32,
                    work(s.font.stroke_color.unwrap_or(s.color)),
                )),
                false => None,
            };
            let glyphs = |shadow: Option<[f32; 4]>, by: [f32; 2]| -> Vec<Item> {
                // The stroke and fill cast one shadow: both opaque, in one layer.
                let layer = shadow.filter(|c| stroke.is_some() && c[3] < 1.0);
                let shadow = match layer {
                    Some(c) => Some(c.map(|v| v / c[3])),
                    None => shadow,
                };
                let mut items = Vec::new();
                for part in s.paint_order {
                    match part {
                        PaintPart::Fill => items.extend(drawn(0.0, shadow.unwrap_or(fill), by)),
                        PaintPart::Stroke => {
                            if let Some((width, color)) = stroke {
                                items.extend(drawn(width, shadow.unwrap_or(color), by));
                            }
                        }
                        PaintPart::Markers => {}
                    }
                }
                match layer {
                    Some(c) => vec![Item::Group {
                        opacity: c[3],
                        blend: Blend::Normal,
                        items,
                    }],
                    None => items,
                }
            };
            let mut drawn = Vec::new();
            self.shadowed(s, stripes(false), clip, &mut drawn);
            self.shadowed(s, glyphs, clip, &mut drawn);
            self.shadowed(s, stripes(true), clip, &mut drawn);
            match self.template.nodes[run.node as usize].kind {
                Kind::Span { unit: true, .. } => {
                    let items = drawn.into_iter().map(|(_, item)| item).collect();
                    let items = self.unit(run.node as usize, items);
                    out.extend(items.into_iter().map(|item| (clip, item)));
                }
                _ => out.extend(drawn),
            }
        }
    }

    /// What a run of unit `u` of a split text paints, as an inline-block paints what is in it:
    /// through its filters, at its opacity, and through its transform around its box.
    fn unit(&self, u: usize, mut items: Vec<Item>) -> Vec<Item> {
        let s = &self.styles[u];
        if !s.filter.is_empty() && !items.is_empty() {
            let filters = filters(&s.filter, snap(self.layout.boxes[u].border));
            items = vec![Item::Filter { filters, items }];
        }
        if s.opacity < 1.0 {
            items = group(s.opacity as f32, Blend::Normal, items)
                .into_iter()
                .collect();
        }
        if s.transformed() && !items.is_empty() {
            let matrix = self.spaces[u].flatten();
            if matrix.inverse().is_none() {
                return Vec::new();
            }
            items = vec![Item::Transform { matrix, items }];
        }
        items
    }

    /// `i`'s image if it is an Image: in its content box as its object fit and position have
    /// it, the box clipping it with its corners. An SVG file without a natural size is as big as
    /// its ratio fits in the box, or the box without one, before it fits.
    fn image(&self, i: usize, out: &mut Drawn) {
        let Kind::Image { resource, .. } = self.template.nodes[i].kind else {
            return;
        };
        let picture = &self.pictures[resource as usize];
        let s = &self.styles[i];
        let (rect, radii) = area(s, &self.layout.boxes[i], Area::ContentBox);
        if !s.visible || rect.width <= 0.0 || rect.height <= 0.0 {
            return;
        }
        let clip = match self.clipping(i) {
            true => Some(i),
            false => self.clips[i].last().copied(),
        };
        let [cw, ch] = [rect.width, rect.height];
        let [w, h] = match picture.natural() {
            Natural::Size(size) => size,
            Natural::Ratio(Some(r)) if cw / ch > r => [ch * r, ch],
            Natural::Ratio(Some(r)) => [cw, cw / r],
            Natural::Ratio(None) => [cw, ch],
        };
        let scaled = |k: f32| [w * k, h * k];
        let ratio = w > 0.0 && h > 0.0;
        let contain = match ratio {
            true => scaled((cw / w).min(ch / h)),
            false => [cw, ch],
        };
        let [ow, oh] = match s.object_fit {
            Fit::Fill => [cw, ch],
            Fit::Contain => contain,
            Fit::Cover if ratio => scaled((cw / w).max(ch / h)),
            Fit::Cover => [cw, ch],
            Fit::None => [w, h],
            Fit::ScaleDown if contain[0] < w => contain,
            Fit::ScaleDown => [w, h],
        };
        // Percentages of the room left.
        let room = [cw - ow, ch - oh];
        let [x, y] = [0, 1].map(|k| s.object_position[k].resolve(room[k]));
        let first = Rect {
            x: rect.x + x,
            y: rect.y + y,
            width: ow,
            height: oh,
        };
        let tiles = Tiles {
            first,
            step: [None; 2],
        };
        let image = match picture {
            &Picture::Image { image, .. } => Item::Image {
                rect,
                radii,
                tiles,
                image,
                pixelated: s.pixelated,
            },
            Picture::Svg(document) => {
                let Some(drawn) = svg::document(document, first, self.cache) else {
                    return;
                };
                Item::Clip {
                    rect,
                    radii,
                    items: vec![drawn],
                }
            }
            Picture::None => return,
        };
        out.push((clip, image));
    }

    /// `i`'s drawing if it is an `<svg>`: its elements in its content box through its viewBox,
    /// which percentages of lengths measure, or else the box's size. The box clips them to its
    /// content box with its corners unless its overflow is visible. A viewBox without area draws
    /// nothing.
    fn svg(&self, i: usize, out: &mut Drawn) {
        let s = &self.styles[i];
        let Some(Element::Svg { view_box, fit }) = s.svg.as_deref().map(|c| &c.element) else {
            return;
        };
        let (rect, radii) = area(s, &self.layout.boxes[i], Area::ContentBox);
        if rect.width <= 0.0 || rect.height <= 0.0 {
            return;
        }
        let size = [rect.width, rect.height].map(f64::from);
        let (matrix, units) = match *view_box {
            Some([.., w, h]) if w <= 0.0 || h <= 0.0 => return,
            Some(b) => (crate::svg::viewport(b, *fit, size), [b[2], b[3]]),
            None => (Matrix::IDENTITY, size),
        };
        let matrix = Matrix::translate(f64::from(rect.x), f64::from(rect.y)).times(matrix);
        let mut items = Vec::new();
        for &c in &self.template.nodes[i].children {
            (self.drawing).element(c as usize, units, matrix, &mut items, None);
        }
        if items.is_empty() {
            return;
        }
        let mut item = Item::Transform { matrix, items };
        if self.clipping(i) {
            item = Item::Clip {
                rect,
                radii,
                items: vec![item],
            };
        }
        out.push((self.clips[i].last().copied(), item));
    }

    /// The glyphs and decorations of `i`'s text and its descendants', opaque, as Chrome clips a
    /// background to text: their fills, without strokes or shadows. Descendants with transforms
    /// of their own are left out.
    fn text_mask(&self, i: usize) -> Vec<Item> {
        const OPAQUE: [f32; 4] = [1.0; 4];
        let mut items = Vec::new();
        let mut nodes = vec![i];
        while let Some(n) = nodes.pop() {
            if n != i && self.styles[n].transformed() {
                continue;
            }
            nodes.extend(self.children(Some(n)));
            let Some(lines) = &self.layout.lines[n] else {
                continue;
            };
            let at = self.layout.boxes[n].content;
            for (k, run) in lines.runs.iter().enumerate() {
                let s = &self.styles[run.node as usize];
                if !s.visible {
                    continue;
                }
                let glyphs = &lines.glyphs[run.glyphs[0] as usize..run.glyphs[1] as usize];
                let size = s.font.size as f32;
                items.extend(faces(glyphs, at, size, OPAQUE, 0.0, [0.0; 2]));
                let decorations = lines.decorations.iter().filter(|d| d.run == k as u32);
                items.extend(decorations.flat_map(|d| stripe(d, at, OPAQUE, [0.0; 2])));
            }
        }
        items
    }

    /// What `part` paints in its colors over its shadows in `s`, the last one first: what it
    /// paints in a shadow's color, moved by its offset and blurred by half its blur, as Chrome
    /// blurs it. A shadow shows whatever alpha the text has.
    fn shadowed(
        &self,
        s: &Style,
        part: impl Fn(Option<[f32; 4]>, [f32; 2]) -> Vec<Item>,
        clip: Option<usize>,
        out: &mut Drawn,
    ) {
        let shown = |item: &Item| match item {
            Item::Rect { color, .. } | Item::Glyphs { color, .. } => color[3] > 0.0,
            _ => true,
        };
        for shadow in s.font.shadows.iter().rev() {
            let color = work(shadow.color.unwrap_or(s.color));
            if color[3] <= 0.0 {
                continue;
            }
            let by = shadow.offset.map(|v| v as f32);
            let items: Vec<Item> = part(Some(color), by).into_iter().filter(shown).collect();
            match shadow.blur > 0.0 && !items.is_empty() {
                true => out.push((
                    clip,
                    Item::Filter {
                        filters: vec![Filter::Blur(shadow.blur as f32 / 2.0)],
                        items,
                    },
                )),
                false => out.extend(items.into_iter().map(|item| (clip, item))),
            }
        }
        let items = part(None, [0.0, 0.0]).into_iter().filter(shown);
        out.extend(items.map(|item| (clip, item)));
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
                | Item::Gradient { rect: r, .. }
                | Item::Image { rect: r, .. } => within(*r, rect),
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

/// Items of `glyphs`, laid out in a box whose content box is `at`, moved by `by`: one for each
/// run of glyphs in a face.
fn faces(
    glyphs: &[Glyph],
    at: Rect,
    size: f32,
    color: [f32; 4],
    stroke: f32,
    [dx, dy]: [f32; 2],
) -> impl Iterator<Item = Item> + '_ {
    (glyphs.chunk_by(|a, b| a.face == b.face)).map(move |g| Item::Glyphs {
        face: g[0].face,
        size,
        color,
        stroke,
        glyphs: (g.iter())
            .map(|g| (g.id, [at.x + g.x + dx, at.y + g.y + dy]))
            .collect(),
    })
}

/// A decoration laid out in a box whose content box is `at`, in `color`, moved by `by`: its
/// piece of line, or of the dots, dashes or wave along its whole line.
fn stripe(d: &Stripe, at: Rect, color: [f32; 4], [dx, dy]: [f32; 2]) -> Vec<Item> {
    let [x0, y0, x1, y1] = d.rect;
    let rect = Rect {
        x: at.x + x0 + dx,
        y: at.y + y0 + dy,
        width: x1 - x0,
        height: y1 - y0,
    };
    let (start, length) = (at.x + d.along[0] + dx, d.along[1] - d.along[0]);
    match d.style {
        DecorationStyle::Solid | DecorationStyle::Double => vec![Item::Rect {
            rect,
            radii: SQUARE,
            color,
        }],
        DecorationStyle::Wavy => dash::wave(rect, start, d.thickness, color),
        style => dash::text(rect, start, length, d.thickness, style, color),
    }
}

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

/// `items` as one layer at `opacity`, blended by `blend`. Unblended, a layer of one rectangle or
/// ring is it at the opacity; and a layer of one unblended layer is that layer at both
/// opacities.
fn group(opacity: f32, blend: Blend, items: Vec<Item>) -> Option<Item> {
    if opacity <= 0.0 || items.is_empty() {
        return None;
    }
    let normal = blend == Blend::Normal;
    Some(match <[Item; 1]>::try_from(items) {
        Ok([Item::Rect { rect, radii, color }]) if normal => Item::Rect {
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
        ) if normal => Item::Ring {
            rect,
            radii,
            inner,
            inner_radii,
            colors: colors.map(|c| c.map(|v| v * opacity)),
        },
        Ok(
            [
                Item::Group {
                    opacity: o,
                    blend: Blend::Normal,
                    items,
                },
            ],
        ) => Item::Group {
            opacity: o * opacity,
            blend,
            items,
        },
        Ok([item]) => Item::Group {
            opacity,
            blend,
            items: vec![item],
        },
        Err(items) => Item::Group {
            opacity,
            blend,
            items,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::layout;
    use crate::style::{Sheet, color};
    use crate::template::{Prop, V};
    use crate::testing::{mask, node, text, text_node, view};

    use crate::testing::paint::{names, paint_of, painted};

    #[test]
    fn blocks_paint_backgrounds_then_text_then_positioned_boxes() {
        let order = painted(
            vec![
                view(&[("background", "red")], &[1, 2, 3]),
                text_node("Hi", &[("background", "green"), ("color", "black")]),
                view(
                    &[
                        ("position", "relative"),
                        ("height", "10px"),
                        ("background", "blue"),
                    ],
                    &[],
                ),
                view(&[("height", "10px"), ("background", "gray")], &[]),
            ],
            &[0],
        );
        // The positioned box comes after the text of the block before it, and after the block
        // after it.
        assert_eq!(order, ["red", "green", "gray", "text black", "blue"]);
    }

    #[test]
    fn flex_and_grid_items_paint_all_at_once() {
        let children = |display| {
            painted(
                vec![
                    view(&[("display", display), ("background", "red")], &[1, 2]),
                    text_node("Hi", &[("background", "green"), ("color", "black")]),
                    view(
                        &[
                            ("width", "10px"),
                            ("height", "10px"),
                            ("background", "blue"),
                        ],
                        &[],
                    ),
                ],
                &[0],
            )
        };
        assert_eq!(children("flex"), ["red", "green", "text black", "blue"]);
        assert_eq!(children("grid"), ["red", "green", "text black", "blue"]);
        assert_eq!(children("block"), ["red", "green", "blue", "text black"]);
    }

    #[test]
    fn flex_items_paint_in_order() {
        let item = |color, order| {
            let style = [("width", "10px"), ("background", color), ("order", order)];
            view(&style, &[])
        };
        let order = painted(
            vec![
                view(&[("display", "flex"), ("height", "10px")], &[1, 2, 3]),
                item("red", "1"),
                item("green", "0"),
                view(
                    &[
                        ("position", "absolute"),
                        ("width", "10px"),
                        ("height", "10px"),
                        ("order", "-1"),
                        ("background", "blue"),
                    ],
                    &[],
                ),
            ],
            &[0],
        );
        // The absolute box paints later, with the positioned boxes.
        assert_eq!(order, ["green", "red", "blue"]);
    }

    #[test]
    fn opacity_makes_a_layer_painted_with_the_positioned_boxes() {
        let half = [("opacity", "0.5"), ("height", "10px")];
        let order = painted(
            vec![
                // A layer of one rectangle is the rectangle.
                view(&[("background", "red"), half[0], half[1]], &[]),
                view(&[("background", "green"), half[0], half[1]], &[2]),
                view(&[("background", "blue"), ("height", "5px")], &[]),
                // Nested layers multiply.
                view(&[half[0]], &[4]),
                view(&[half[0]], &[5, 6]),
                view(&[("height", "5px"), ("background", "gray")], &[]),
                view(&[("height", "5px"), ("background", "black")], &[]),
                // Nothing shows through no opacity, or without a background.
                view(&[("opacity", "0"), ("background", "red"), half[1]], &[]),
                view(&[("height", "10px")], &[]),
            ],
            &[0, 1, 3, 7, 8],
        );
        assert_eq!(
            order,
            [
                "red@0.5",
                r#"group 0.5 ["green", "blue"]"#,
                r#"group 0.25 ["gray", "black"]"#,
            ]
        );
        // A layer paints after the in-flow content around it.
        let order = painted(
            vec![
                view(&[("background", "red"), half[0], half[1]], &[]),
                view(&[("background", "green"), ("height", "10px")], &[]),
            ],
            &[0, 1],
        );
        assert_eq!(order, ["green", "red@0.5"]);
    }

    #[test]
    fn positioned_boxes_leave_their_positioned_descendants_to_the_context() {
        let order = painted(
            vec![
                view(&[("position", "relative"), ("background", "red")], &[1, 2]),
                view(
                    &[
                        ("position", "absolute"),
                        ("width", "10px"),
                        ("height", "10px"),
                        ("background", "green"),
                    ],
                    &[],
                ),
                view(&[("height", "10px"), ("background", "blue")], &[]),
                view(&[("height", "10px"), ("background", "gray")], &[]),
            ],
            &[0, 3],
        );
        assert_eq!(order, ["gray", "red", "blue", "green"]);
    }

    #[test]
    fn z_index_orders_stacking_contexts() {
        let at = |z: &str, background| {
            let style = [
                ("position", "relative"),
                ("zIndex", z),
                ("height", "5px"),
                ("background", background),
            ];
            view(&style, &[])
        };
        let order = painted(
            vec![
                view(&[("height", "10px"), ("background", "red")], &[1, 2, 3, 4]),
                at("2", "green"),
                at("-1", "blue"),
                at("auto", "gray"),
                at("1", "black"),
            ],
            &[0],
        );
        // Below the in-flow content, then in tree order at 0, then up.
        assert_eq!(order, ["blue", "red", "gray", "black", "green"]);

        // A flex item with a z-index is a stacking context, painted after the in-flow content.
        let item = |z: &[(&'static str, &'static str)], background| {
            let mut style = vec![("width", "10px"), ("background", background)];
            style.extend_from_slice(z);
            view(&style, &[])
        };
        let flex = |z| {
            painted(
                vec![
                    view(&[("display", "flex"), ("height", "10px")], &[1, 2]),
                    item(z, "green"),
                    item(&[], "red"),
                ],
                &[0],
            )
        };
        assert_eq!(flex(&[("zIndex", "0")]), ["red", "green"]);
        assert_eq!(flex(&[]), ["green", "red"]);

        // A fixed box is a stacking context: what it has below 0 stays in it.
        let order = painted(
            vec![
                view(
                    &[
                        ("position", "fixed"),
                        ("width", "10px"),
                        ("height", "10px"),
                        ("background", "red"),
                    ],
                    &[1],
                ),
                at("-1", "blue"),
                view(&[("height", "10px"), ("background", "green")], &[]),
            ],
            &[0, 2],
        );
        assert_eq!(order, ["green", "red", "blue"]);

        // A block that is neither positioned nor an item takes no z-index, even as a stacking
        // context: at 0, after the in-flow content.
        let order = painted(
            vec![
                view(&[("height", "10px"), ("background", "red")], &[1, 2]),
                view(
                    &[
                        ("opacity", "0.5"),
                        ("zIndex", "-1"),
                        ("height", "5px"),
                        ("background", "blue"),
                    ],
                    &[],
                ),
                view(&[("height", "5px"), ("background", "green")], &[]),
            ],
            &[0],
        );
        assert_eq!(order, ["red", "green", "blue@0.5"]);
    }

    #[test]
    fn transformed_boxes_paint_in_their_transform_as_stacking_contexts() {
        // After the in-flow content, its items in its transform.
        let order = painted(
            vec![
                view(&[("height", "30px"), ("background", "red")], &[1, 2]),
                view(
                    &[
                        ("transform", "rotate(90deg)"),
                        ("height", "10px"),
                        ("background", "blue"),
                    ],
                    &[],
                ),
                view(&[("height", "10px"), ("background", "green")], &[]),
            ],
            &[0],
        );
        assert_eq!(order, ["red", "green", r#"transform ["blue"]"#]);

        // Around its origin, the functions last first.
        let matrix = |origin: &str| {
            let style = [
                ("position", "absolute"),
                ("left", "10px"),
                ("top", "20px"),
                ("width", "40px"),
                ("height", "20px"),
                ("background", "blue"),
                ("transform", "translate(5px, 10%) scale(2)"),
                ("transformOrigin", origin),
            ];
            match &paint_of(vec![view(&style, &[])], &[0]).0[..] {
                [Item::Transform { matrix, .. }] => *matrix,
                items => panic!("{items:?}"),
            }
        };
        assert_eq!(matrix("center").apply([10.0, 20.0]), [-5.0, 12.0]);
        assert_eq!(matrix("left top").apply([10.0, 20.0]), [15.0, 22.0]);
        assert_eq!(matrix("left top").apply([30.0, 30.0]), [55.0, 42.0]);

        // One that flattens the plane shows nothing.
        let flat = [
            ("transform", "scale(0, 1)"),
            ("height", "10px"),
            ("background", "blue"),
        ];
        assert_eq!(painted(vec![view(&flat, &[])], &[0]), Vec::<String>::new());
    }

    #[test]
    fn clips_around_a_transformed_box_go_around_its_transform() {
        let clipping = [("overflow", "hidden"), ("height", "40px")];
        let transformed = [
            ("transform", "rotate(10deg)"),
            ("height", "20px"),
            ("background", "blue"),
        ];
        let absolute = [
            ("position", "absolute"),
            ("width", "5px"),
            ("height", "5px"),
            ("background", "green"),
        ];
        let (items, _) = paint_of(
            vec![
                view(&[("position", "relative")], &[1]),
                view(&clipping, &[2]),
                view(&transformed, &[3]),
                view(&absolute, &[]),
            ],
            &[0],
        );
        // The positioned box's containing block is the transformed one, so the clip holds it too.
        let [Item::Clip { items, .. }] = &items[..] else {
            panic!("{items:?}")
        };
        assert_eq!(names(items), [r#"transform ["blue", "green"]"#]);

        // Its own clip is inside its transform.
        let clipped = [transformed.as_slice(), &[("overflow", "hidden")]].concat();
        let overflowing = [
            ("height", "50px"),
            ("borderRadius", "4px"),
            ("background", "green"),
        ];
        let (items, _) = paint_of(vec![view(&clipped, &[1]), view(&overflowing, &[])], &[0]);
        assert_eq!(names(&items), [r#"transform ["blue", "clip [\"green\"]"]"#]);
    }

    #[test]
    fn transforms_take_a_box_to_the_canvas_through_its_ancestors() {
        let template = Template {
            nodes: vec![
                view(&[("transform", "translate(10px, 0)")], &[1]),
                view(
                    &[
                        ("width", "20px"),
                        ("height", "20px"),
                        ("transform", "rotate(90deg)"),
                        ("transformOrigin", "0 0"),
                    ],
                    &[2],
                ),
                view(&[("height", "5px")], &[]),
            ],
            roots: vec![0],
            ..Default::default()
        };
        let canvas = [200.0, 100.0];
        let styles = Sheet::new(&template)
            .unwrap()
            .compute(&[], true, canvas, &[])
            .unwrap();
        let laid = layout(&template, &[], &styles, canvas, &mut text(), &[]).unwrap();
        let transforms = transforms(&template, &styles, &laid);
        let at = |i: usize, p| transforms[i].apply(p).map(rounded);
        assert_eq!(at(0, [0.0, 0.0]), [10.0, 0.0]);
        // A quarter turn clockwise about its top left, then moved by its parent.
        assert_eq!(at(1, [20.0, 0.0]), [10.0, 20.0]);
        assert_eq!(at(2, [0.0, 5.0]), [5.0, 0.0]);
    }

    #[test]
    fn motion_paths_move_a_box_along_and_turn_it_after_its_own_transforms() {
        let moving = |more: &[(&'static str, &'static str)]| {
            let place = [
                ("position", "absolute"),
                ("left", "50px"),
                ("top", "20px"),
                ("width", "40px"),
                ("height", "20px"),
            ];
            view(&[&place[..], more].concat(), &[])
        };
        let container = [
            ("position", "relative"),
            ("height", "100px"),
            ("padding", "10px"),
            ("border", "5px solid black"),
        ];
        let template = Template {
            nodes: vec![
                view(&container, &[1, 2, 3, 4, 5]),
                moving(&[
                    ("offsetPath", "path('M0 0 L100 0')"),
                    ("offsetDistance", "50%"),
                ]),
                moving(&[
                    ("offsetPath", "path('M0 0 H100 V100 Z')"),
                    ("offsetDistance", "150%"),
                ]),
                moving(&[
                    ("offsetPath", "ray(90deg)"),
                    ("offsetDistance", "30px"),
                    ("offsetAnchor", "0 0"),
                ]),
                moving(&[
                    ("offsetPath", "circle(20px at 100px 50px) content-box"),
                    ("offsetDistance", "25%"),
                ]),
                moving(&[
                    ("offsetPath", "path('M0 0')"),
                    ("translate", "10px 0"),
                    ("transform", "rotate(90deg)"),
                ]),
            ],
            roots: vec![0],
            ..Default::default()
        };
        let canvas = [200.0, 100.0];
        let styles = Sheet::new(&template)
            .unwrap()
            .compute(&[], true, canvas, &[])
            .unwrap();
        let laid = layout(&template, &[], &styles, canvas, &mut text(), &[]).unwrap();
        let transforms = transforms(&template, &styles, &laid);
        // Each box's center and the point 20 px right of it, from its border box at 55, 25.
        let at = |i: usize| [[75.0, 35.0], [95.0, 35.0]].map(|p| transforms[i].apply(p));
        let near = |i: usize, want: [[f32; 2]; 2]| {
            let got = at(i);
            let off = (0..4).map(|k| (got[k / 2][k % 2] - want[k / 2][k % 2]).abs());
            assert!(
                off.fold(0.0, f32::max) < 0.1,
                "box {i}: {got:?}, not {want:?}"
            );
        };
        // A path is from the box's own top left, its center on it.
        near(1, [[105.0, 25.0], [125.0, 25.0]]);
        // Closed, it goes on around, and the box turns down with it.
        near(2, [[155.0, 25.0 + 70.711], [155.0, 45.0 + 70.711]]);
        // A ray from the containing block's center, its anchor the box's top left on it.
        near(3, [[150.0, 60.0], [170.0, 60.0]]);
        // A circle in the containing block's content box, a quarter around, turned left.
        near(4, [[115.0, 85.0], [95.0, 85.0]]);
        // After `translate` and before `transform`.
        near(5, [[65.0, 25.0], [65.0, 45.0]]);
    }

    #[test]
    fn a_perspective_reaches_only_the_boxs_children() {
        let toward = [
            ("width", "40px"),
            ("height", "20px"),
            ("transform", "translateZ(50px)"),
        ];
        let template = Template {
            nodes: vec![
                view(&[("height", "100px"), ("perspective", "100px")], &[1, 2]),
                view(&toward, &[]),
                view(&[("height", "20px")], &[3]),
                view(&toward, &[]),
            ],
            roots: vec![0],
            ..Default::default()
        };
        let canvas = [200.0, 100.0];
        let styles = Sheet::new(&template)
            .unwrap()
            .compute(&[], true, canvas, &[])
            .unwrap();
        let laid = layout(&template, &[], &styles, canvas, &mut text(), &[]).unwrap();
        let transforms = transforms(&template, &styles, &laid);
        let at = |i: usize, p| transforms[i].apply(p).map(rounded);
        // Halfway to a viewer 100 px in front of the center: twice as far from it.
        assert_eq!(at(1, [0.0, 0.0]), [-100.0, -50.0]);
        // Its grandchild is flattened as if from infinitely far.
        assert_eq!(at(3, [0.0, 20.0]), [0.0, 20.0]);
    }

    #[test]
    fn a_box_showing_its_back_hides_with_all_in_it_when_asked() {
        let turned = |transform: &'static str, backface: &'static str| {
            let style = [
                ("height", "10px"),
                ("background", "blue"),
                ("transform", transform),
                ("backfaceVisibility", backface),
            ];
            painted(
                vec![
                    view(&style, &[1]),
                    view(&[("height", "5px"), ("background", "green")], &[]),
                ],
                &[0],
            )
        };
        assert_eq!(
            turned("rotateY(180deg)", "visible"),
            [r#"transform ["blue", "green"]"#]
        );
        assert_eq!(turned("rotateY(180deg)", "hidden"), [] as [&str; 0]);
        assert_eq!(turned("rotateX(-120deg)", "hidden"), [] as [&str; 0]);
        // A mirror does not show the back.
        assert_eq!(
            turned("scaleX(-1)", "hidden"),
            [r#"transform ["blue", "green"]"#]
        );
    }

    #[test]
    fn a_3d_scene_paints_its_planes_back_to_front_cut_where_they_cross() {
        let card = |color: &'static str, transform: &'static str| {
            let style = [
                ("position", "absolute"),
                ("width", "20px"),
                ("height", "20px"),
                ("background", color),
                ("transform", transform),
            ];
            view(&style, &[])
        };
        let scene = |style: &[(&'static str, &'static str)], cards| {
            let mut nodes = vec![view(style, &[1, 2])];
            nodes.extend(cards);
            painted(nodes, &[0])
        };
        let keeps = [("height", "20px"), ("transformStyle", "preserve-3d")];
        // The nearer one last, whatever the tree order.
        let order = scene(
            &keeps,
            [
                card("red", "translateZ(10px)"),
                card("blue", "translateZ(-10px)"),
            ],
        );
        assert_eq!(order, [r#"transform ["blue"]"#, r#"transform ["red"]"#]);
        // Two that cross: the second cut in two, its far half before the first and its near half
        // after.
        let order = scene(
            &keeps,
            [
                card("red", "rotateY(30deg)"),
                card("blue", "rotateY(-30deg)"),
            ],
        );
        assert_eq!(
            order,
            [
                r#"transform ["cut [\"blue\"]"]"#,
                r#"transform ["red"]"#,
                r#"transform ["cut [\"blue\"]"]"#,
            ]
        );
        // An opacity groups them, which flattens them in tree order.
        let flat = [keeps[0], keeps[1], ("opacity", "0.5")];
        let order = scene(
            &flat,
            [
                card("red", "translateZ(10px)"),
                card("blue", "translateZ(-10px)"),
            ],
        );
        assert_eq!(
            order,
            [r#"group 0.5 ["transform [\"red\"]", "transform [\"blue\"]"]"#]
        );
    }

    #[test]
    fn transforms_in_a_3d_scene_add_up_before_they_are_flattened() {
        let template = Template {
            nodes: vec![
                view(
                    &[
                        ("width", "20px"),
                        ("height", "20px"),
                        ("transform", "rotateY(45deg)"),
                        ("transformStyle", "preserve-3d"),
                    ],
                    &[1],
                ),
                view(&[("height", "20px"), ("transform", "rotateY(45deg)")], &[]),
            ],
            roots: vec![0],
            ..Default::default()
        };
        let canvas = [200.0, 100.0];
        let styles = Sheet::new(&template)
            .unwrap()
            .compute(&[], true, canvas, &[])
            .unwrap();
        let laid = layout(&template, &[], &styles, canvas, &mut text(), &[]).unwrap();
        let transforms = transforms(&template, &styles, &laid);
        // A quarter turn about y in all: edge on.
        assert_eq!(transforms[1].inverse(), None);
    }

    #[test]
    fn hidden_boxes_paint_only_what_is_visible_in_them() {
        let order = painted(
            vec![
                view(
                    &[("visibility", "hidden"), ("background", "red")],
                    &[1, 2, 3],
                ),
                view(&[("height", "5px"), ("background", "green")], &[]),
                text_node("Hi", &[("color", "black")]),
                view(
                    &[
                        ("visibility", "visible"),
                        ("height", "5px"),
                        ("background", "blue"),
                    ],
                    &[],
                ),
            ],
            &[0],
        );
        assert_eq!(order, ["blue"]);
    }

    #[test]
    fn overflow_clips_what_the_box_contains() {
        let hidden = [("overflow", "hidden"), ("height", "20px")];
        let at = |position, top: &str, background| {
            let style = [
                ("position", position),
                ("top", top),
                ("width", "10px"),
                ("height", "10px"),
                ("background", background),
            ];
            view(&style, &[])
        };
        let (items, _) = paint_of(
            vec![
                view(&[hidden[0], hidden[1], ("background", "red")], &[1, 2, 3]),
                view(&[("height", "30px"), ("background", "green")], &[]),
                // Its containing block is the canvas, outside the clip.
                at("absolute", "30px", "blue"),
                at("relative", "-15px", "gray"),
                view(&[hidden[0], hidden[1], ("position", "relative")], &[5, 6]),
                at("absolute", "15px", "black"),
                at("fixed", "50px", "red"),
            ],
            &[0, 4],
        );
        let rect = |x: f32, y: f32, width: f32, height: f32| Rect {
            x,
            y,
            width,
            height,
        };
        let rects: Vec<Rect> = (items.iter())
            .map(|item| match item {
                Item::Rect { rect, .. } => *rect,
                _ => panic!("{items:?}"),
            })
            .collect();
        // A square clip cuts a lone rectangle, or leaves one inside it as it is.
        assert_eq!(
            rects,
            [
                rect(0.0, 0.0, 200.0, 20.0),
                rect(0.0, 0.0, 200.0, 20.0),
                rect(0.0, 30.0, 10.0, 10.0),
                rect(0.0, 15.0, 10.0, 5.0),
                rect(0.0, 35.0, 10.0, 5.0),
                rect(0.0, 50.0, 10.0, 10.0),
            ]
        );
    }

    #[test]
    fn clips_round_and_go_on_each_item_in_a_layer() {
        let (items, _) = paint_of(
            vec![
                view(
                    &[
                        ("overflow", "clip"),
                        ("height", "20px"),
                        ("borderRadius", "4px"),
                    ],
                    &[1],
                ),
                view(
                    &[
                        ("opacity", "0.5"),
                        ("height", "30px"),
                        ("background", "green"),
                    ],
                    &[2],
                ),
                view(
                    &[
                        ("position", "absolute"),
                        ("top", "40px"),
                        ("width", "10px"),
                        ("height", "10px"),
                        ("background", "blue"),
                    ],
                    &[],
                ),
                view(&[("overflowX", "clip"), ("width", "10px")], &[4]),
                text_node("Clipped across", &[("color", "black")]),
            ],
            &[0, 3],
        );
        assert_eq!(
            names(&items),
            [
                r#"clip ["text black", "text black"]"#.to_string(),
                r#"group 0.5 ["clip [\"green\"]", "blue"]"#.into(),
            ]
        );
        let Item::Group { items: layer, .. } = &items[1] else {
            panic!("{items:?}")
        };
        let Item::Clip { rect, radii, .. } = &layer[0] else {
            panic!("{layer:?}")
        };
        assert_eq!((rect.height, *radii), (20.0, [[4.0; 2]; 4]));
    }

    #[test]
    fn a_filter_takes_all_a_box_paints_inside_its_transform_and_clips() {
        let (items, _) = paint_of(
            vec![
                view(&[("overflow", "hidden"), ("height", "50px")], &[1]),
                view(
                    &[
                        (
                            "filter",
                            "blur(2px) grayscale(1) invert(0.5) drop-shadow(1px 2px red)",
                        ),
                        ("transform", "rotate(10deg)"),
                        ("opacity", "0.5"),
                        ("height", "20px"),
                        ("background", "blue"),
                    ],
                    &[2],
                ),
                // A fixed box in a filtered one is in its filter.
                view(
                    &[
                        ("position", "fixed"),
                        ("width", "10px"),
                        ("height", "10px"),
                        ("background", "green"),
                    ],
                    &[],
                ),
            ],
            &[0],
        );
        let [Item::Group { items, .. }] = &items[..] else {
            panic!("{items:?}")
        };
        let [Item::Clip { items, .. }] = &items[..] else {
            panic!("{items:?}")
        };
        let [Item::Transform { items, .. }] = &items[..] else {
            panic!("{items:?}")
        };
        let [Item::Filter { filters, items }] = &items[..] else {
            panic!("{items:?}")
        };
        assert_eq!(names(items), ["blue", "green"]);
        let [
            Filter::Blur(2.0),
            Filter::Colors(m),
            Filter::Shadow {
                offset,
                sigma,
                color,
            },
        ] = &filters[..]
        else {
            panic!("{filters:?}")
        };
        assert_eq!(
            (m.len(), *offset, *sigma, *color),
            (2, [1.0, 2.0], 0.0, [1.0, 0.0, 0.0, 1.0])
        );
        assert_eq!(m[0][..3], [0.2126, 0.7152, 0.0722]);
        assert_eq!(m[1][..4], [0.0; 4]);
        assert_eq!(m[1][16..], [0.5, 0.5, 0.5, 0.0]);
    }

    #[test]
    fn a_backdrop_filter_paints_under_its_box_in_a_layer() {
        let order = painted(
            vec![
                view(&[("background", "gray")], &[1]),
                view(
                    &[
                        ("backdropFilter", "blur(4px) grayscale(1)"),
                        ("margin", "10px"),
                        ("height", "20px"),
                        ("borderRadius", "5px"),
                        ("background", "red"),
                    ],
                    &[2],
                ),
                // Fixed, in it.
                view(
                    &[
                        ("position", "fixed"),
                        ("width", "5px"),
                        ("height", "5px"),
                        ("background", "blue"),
                    ],
                    &[],
                ),
            ],
            &[0],
        );
        assert_eq!(
            order,
            [
                "gray",
                r#"group 1 ["backdrop of 2", "group 1 [\"red\", \"blue\"]"]"#
            ]
        );
    }

    #[test]
    fn backdrop_blending_composites_normally_in_layers_and_planes() {
        for plane in [false, true] {
            let mut parent = vec![("background", "gray")];
            let mut child = vec![
                ("width", "100px"),
                ("height", "60px"),
                ("backdropFilter", "invert(1)"),
                ("mixBlendMode", "multiply"),
                ("opacity", "0.5"),
                ("background", "red"),
            ];
            if plane {
                parent.push(("transformStyle", "preserve-3d"));
                child.push(("transform", "translateZ(1px)"));
            }
            let (items, _) = paint_of(vec![view(&parent, &[1]), view(&child, &[])], &[0]);
            let mut layer = items.last().unwrap();
            if plane {
                let Item::Group {
                    opacity,
                    blend,
                    items,
                } = layer
                else {
                    panic!("expected an isolated scene: {layer:?}")
                };
                assert_eq!((*opacity, *blend), (1.0, Blend::Normal));
                let Item::Transform { items, .. } = items.last().unwrap() else {
                    panic!("expected a scene plane: {layer:?}")
                };
                layer = &items[0];
            }
            let Item::Group {
                opacity,
                blend,
                items,
            } = layer
            else {
                panic!("expected a compositing group: {layer:?}")
            };
            assert_eq!((*opacity, *blend), (0.5, Blend::Normal));
            let [
                Item::Backdrop { .. },
                Item::Group {
                    opacity,
                    blend,
                    items,
                },
            ] = &items[..]
            else {
                panic!("expected backdrop then foreground: {items:?}")
            };
            assert_eq!((*opacity, *blend), (1.0, Blend::Multiply));
            assert_eq!(names(items), ["red"]);
        }
    }

    #[test]
    fn an_empty_backdrop_box_still_isolates_overflowing_content() {
        let (items, _) = paint_of(
            vec![
                view(&[("background", "gray")], &[1]),
                view(
                    &[
                        ("width", "0px"),
                        ("height", "60px"),
                        ("backdropFilter", "invert(1)"),
                        ("mixBlendMode", "multiply"),
                    ],
                    &[2],
                ),
                view(
                    &[("width", "80px"), ("height", "60px"), ("background", "red")],
                    &[],
                ),
            ],
            &[0],
        );
        let [_, Item::Group { blend, items, .. }] = &items[..] else {
            panic!("expected an overflowing foreground: {items:?}")
        };
        assert_eq!(*blend, Blend::Normal);
        assert_eq!(names(items), [r#"group 1 Multiply ["red"]"#]);
    }

    #[test]
    fn a_clip_path_masks_all_its_stacking_context_paints() {
        let order = painted(
            vec![
                view(&[("background", "gray")], &[1]),
                view(
                    &[
                        ("clipPath", "inset(2px round 4px)"),
                        ("opacity", "0.5"),
                        ("margin", "10px"),
                        ("height", "20px"),
                        ("background", "red"),
                    ],
                    &[2],
                ),
                // Absolute, its containing block the canvas: in the box's stacking context all the
                // same.
                view(
                    &[
                        ("position", "absolute"),
                        ("width", "5px"),
                        ("height", "5px"),
                        ("background", "blue"),
                    ],
                    &[],
                ),
            ],
            &[0],
        );
        assert_eq!(
            order,
            [
                "gray",
                r#"group 0.5 ["mask [\"white\"] [\"red\", \"blue\"]"]"#
            ]
        );
        // Around its backdrop and filter, inside its transform.
        let (items, _) = paint_of(
            vec![
                view(&[("background", "gray")], &[1]),
                view(
                    &[
                        ("clipPath", "circle()"),
                        ("filter", "blur(1px)"),
                        ("backdropFilter", "invert(1)"),
                        ("transform", "rotate(10deg)"),
                        ("height", "20px"),
                        ("background", "red"),
                    ],
                    &[],
                ),
            ],
            &[0],
        );
        let [_, Item::Group { items, .. }] = &items[..] else {
            panic!("{items:?}")
        };
        let [Item::Transform { items, .. }] = &items[..] else {
            panic!("{items:?}")
        };
        let [Item::Mask { mask, items }] = &items[..] else {
            panic!("{items:?}")
        };
        assert_eq!(names(mask), ["white"]);
        assert_eq!(
            names(items),
            [
                "backdrop of 1",
                r#"group 1 ["filter [\"blur\"] [\"red\"]"]"#
            ]
        );
    }

    #[test]
    fn a_mask_shows_what_its_box_paints_through_its_layers() {
        let (items, _) = paint_of(
            vec![
                view(&[("background", "gray")], &[1]),
                view(
                    &[
                        (
                            "maskImage",
                            "linear-gradient(black, transparent), radial-gradient(red, blue, green)",
                        ),
                        ("maskMode", "alpha, luminance"),
                        ("clipPath", "circle()"),
                        ("margin", "10px"),
                        ("height", "20px"),
                        ("borderRadius", "5px"),
                        ("background", "red"),
                    ],
                    &[2],
                ),
                view(
                    &[
                        ("position", "absolute"),
                        ("width", "5px"),
                        ("height", "5px"),
                        ("background", "blue"),
                    ],
                    &[],
                ),
            ],
            &[0],
        );
        // The last layer first, under the first; through the clip path first.
        let [_, Item::Mask { mask, items }] = &items[..] else {
            panic!("{items:?}")
        };
        assert_eq!(
            names(mask),
            [r#"filter ["luminance"] ["gradient of 2"]"#, "gradient of 1"]
        );
        assert_eq!(names(items), [r#"mask ["white"] ["red", "blue"]"#]);
        let Item::Gradient { rect, radii, .. } = &mask[1] else {
            panic!("{mask:?}")
        };
        // In the border box, rounded by its corners.
        assert_eq!(
            (*rect, *radii),
            (
                Rect {
                    x: 10.0,
                    y: 10.0,
                    width: 180.0,
                    height: 20.0
                },
                [[5.0; 2]; 4]
            )
        );
    }

    #[test]
    fn spans_paint_their_text_in_their_own_style() {
        use crate::template::{Kind, Piece, Prop, V};
        let piece = |s: &str| Piece::Text(Prop::Const(V::Str(s.into())));
        let mut text = text_node("", &[("color", "black")]);
        text.kind = Kind::Text {
            content: vec![piece("a "), Piece::Span(1), piece(" c")],
        };
        text.children = vec![1];
        let mut span = view(&[("color", "red"), ("fontSize", "20px")], &[]);
        span.kind = Kind::Span {
            content: vec![piece("b"), Piece::Span(2)],
            unit: false,
        };
        span.children = vec![2];
        let mut hidden = view(&[("visibility", "hidden")], &[]);
        hidden.kind = Kind::Span {
            content: vec![piece("d")],
            unit: false,
        };
        let (items, _) = paint_of(vec![text, span, hidden], &[0]);
        assert_eq!(names(&items), ["text black", "text red", "text black"]);
        let sizes: Vec<f32> = (items.iter())
            .map(|i| match i {
                Item::Glyphs { size, .. } => *size,
                _ => 0.0,
            })
            .collect();
        assert_eq!(sizes, [16.0, 20.0, 16.0]);
    }

    #[test]
    fn text_paints_line_throughs_over_its_glyphs_and_other_lines_under() {
        let style = [
            ("color", "black"),
            ("textDecoration", "underline overline line-through red"),
        ];
        let (items, _) = paint_of(vec![text_node("x", &style)], &[0]);
        assert_eq!(names(&items), ["red", "red", "text black", "red"]);
        let hidden = [("visibility", "hidden"), ("textDecoration", "underline")];
        assert!(paint_of(vec![text_node("x", &hidden)], &[0]).0.is_empty());
    }

    #[test]
    fn text_shadows_go_under_each_part_of_a_run_the_last_lowest() {
        let style = [
            ("color", "black"),
            ("textDecoration", "underline white"),
            ("textShadow", "1px 1px red, 2px 2px 4px blue"),
        ];
        let (items, _) = paint_of(vec![text_node("x", &style)], &[0]);
        let blurred = |what: &str| format!("filter [\"blur\"] [\"{what}\"]");
        assert_eq!(
            names(&items),
            [
                blurred("blue"),
                "red".into(),
                "white".into(),
                blurred("text blue"),
                "text red".into(),
                "text black".into(),
            ]
        );
        let Item::Filter { filters, .. } = &items[3] else {
            unreachable!()
        };
        assert!(matches!(filters[..], [Filter::Blur(2.0)]));
        // Text with no fill still casts its shadows.
        let unfilled = [
            ("WebkitTextFillColor", "transparent"),
            ("textShadow", "1px 1px red"),
        ];
        let (items, _) = paint_of(vec![text_node("x", &unfilled)], &[0]);
        assert_eq!(names(&items), ["text red"]);
    }

    #[test]
    fn text_strokes_go_over_the_fill_and_cast_one_shadow_with_it() {
        let style = [
            ("color", "black"),
            ("WebkitTextStroke", "2px blue"),
            ("textShadow", "1px 1px rgb(255 0 0 / 0.5), 2px 2px green"),
        ];
        let (items, _) = paint_of(vec![text_node("x", &style)], &[0]);
        // A translucent shadow is one layer, so that it is no darker where the two overlap.
        assert_eq!(
            names(&items),
            [
                "text green",
                "text green",
                "group 0.5 [\"text red\", \"text red\"]",
                "text black",
                "text blue",
            ]
        );
        let strokes: Vec<f32> = [&items[..2], &items[3..]]
            .concat()
            .iter()
            .map(|item| match item {
                Item::Glyphs { stroke, .. } => *stroke,
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(strokes, [0.0, 2.0, 0.0, 2.0]);
        // Its currentcolor is the color of the text, whatever its fill.
        let outlined = [
            ("color", "red"),
            ("WebkitTextFillColor", "transparent"),
            ("WebkitTextStrokeWidth", "1px"),
        ];
        assert_eq!(painted(vec![text_node("x", &outlined)], &[0]), ["text red"]);
    }

    #[test]
    fn stroke_first_text_keeps_its_fill_and_a_single_translucent_shadow() {
        let style = [
            ("color", "white"),
            ("WebkitTextStroke", "2px black"),
            ("paintOrder", "markers stroke"),
            ("textShadow", "1px 1px rgb(255 0 0 / 0.5)"),
        ];
        let (items, _) = paint_of(vec![text_node("字", &style)], &[0]);
        assert_eq!(
            names(&items),
            [
                "group 0.5 [\"text red\", \"text red\"]",
                "text black",
                "text white",
            ]
        );
        let Item::Group { items: shadow, .. } = &items[0] else {
            unreachable!()
        };
        for drawn in [&items[1..], shadow.as_slice()] {
            let strokes: Vec<_> = drawn
                .iter()
                .map(|item| match item {
                    Item::Glyphs { stroke, .. } => *stroke,
                    _ => unreachable!(),
                })
                .collect();
            assert_eq!(strokes, [2.0, 0.0]);
        }
    }

    #[test]
    fn a_background_clipped_to_text_shows_through_the_text_in_the_box() {
        let underlined = [("color", "black"), ("textDecoration", "underline red")];
        let painted = painted(
            vec![
                view(
                    &[("background", "linear-gradient(red, blue) text green")],
                    &[1, 2],
                ),
                text_node("x", &underlined),
                // Text in a transform of its own is not the box's.
                view(&[("transform", "translate(1px)")], &[3]),
                text_node("y", &[("color", "black")]),
            ],
            &[0],
        );
        // The glyphs and their decorations, opaque, for the color as for the image.
        let mask = r#"["text white", "white"]"#;
        assert_eq!(
            painted,
            [
                format!(r#"mask {mask} ["green"]"#),
                format!(r#"mask {mask} ["gradient of 1"]"#),
                "red".into(),
                "text black".into(),
                r#"transform ["text black"]"#.into(),
            ]
        );
    }

    #[test]
    fn a_mask_element_shows_what_its_box_paints_through_its_source() {
        let masked = |source: (u32, bool, bool), shown: &[u32]| {
            paint_of(
                vec![
                    mask(
                        source,
                        &[
                            ("margin", "10px"),
                            ("height", "20px"),
                            ("borderRadius", "5px"),
                            ("background", "red"),
                            ("color", "green"),
                        ],
                        &[1],
                    ),
                    view(&[("height", "10px"), ("background", "blue")], &[]),
                    view(&[], shown),
                    view(&[("height", "10px"), ("background", "green")], &[]),
                ],
                &[0],
            )
            .0
        };
        // What the source paints, inheriting from the mask, in the border box rounded.
        let items = masked((2, false, false), &[3]);
        let [Item::Mask { mask, items }] = &items[..] else {
            panic!("{items:?}")
        };
        assert_eq!(names(items), ["red", "blue"]);
        assert_eq!(names(mask), [r#"mask ["white"] ["green"]"#]);
        let [Item::Mask { mask: area, .. }] = &mask[..] else {
            panic!("{mask:?}")
        };
        let [Item::Rect { rect, radii, .. }] = &area[..] else {
            panic!("{area:?}")
        };
        assert_eq!((rect.y, rect.height, *radii), (10.0, 20.0, [[5.0; 2]; 4]));
        // Inverted, by luminance: inside the box, where the source's luminance is not.
        let items = masked((2, true, true), &[3]);
        let [Item::Mask { mask, .. }] = &items[..] else {
            panic!("{items:?}")
        };
        let [Item::Mask { mask: area, items }] = &mask[..] else {
            panic!("{mask:?}")
        };
        assert_eq!(names(area), ["white"]);
        let [Item::Filter { filters, items }] = &items[..] else {
            panic!("{items:?}")
        };
        assert_eq!(filters, &[Filter::Colors(vec![INVERTED])]);
        assert_eq!(names(items), [r#"filter ["luminance"] ["green"]"#]);
        // A source that paints nothing shows nothing, or inverted, all of the box.
        assert!(masked((2, false, false), &[]).is_empty());
        let items = masked((2, false, true), &[]);
        assert_eq!(names(&items), [r#"mask ["white"] ["red", "blue"]"#]);
    }

    #[test]
    fn clip_path_shapes_are_in_the_box_they_name() {
        use crate::path::Segment;
        // A box at 10, 10, 100 × 40, its border 2 px, its padding 3 px, its corners rounded 10 px.
        let mask = |clip: &str| {
            let style = [
                ("margin", "10px"),
                ("width", "100px"),
                ("height", "40px"),
                ("border", "2px solid black"),
                ("padding", "3px"),
                ("borderRadius", "10px"),
                ("background", "red"),
                ("clipPath", clip),
            ];
            let (items, _) = paint_of(vec![view(&style, &[])], &[0]);
            match <[Item; 1]>::try_from(items) {
                Ok([Item::Mask { mask, .. }]) => Some(mask),
                Ok(other) => panic!("{other:?}"),
                Err(none) if none.is_empty() => None,
                Err(items) => panic!("{items:?}"),
            }
        };
        let rect = |x, y, width, height, radii: [f32; 2]| Item::Rect {
            rect: Rect {
                x,
                y,
                width,
                height,
            },
            radii: [radii; 4],
            color: [1.0; 4],
        };
        assert_eq!(
            mask("padding-box"),
            Some(vec![rect(12.0, 12.0, 96.0, 36.0, [8.0; 2])])
        );
        // Radii of the box, fitting the rectangle.
        assert_eq!(
            mask("inset(5px 10% round 50%)"),
            Some(vec![rect(20.0, 15.0, 80.0, 30.0, [37.5, 15.0])])
        );
        assert_eq!(
            mask("circle(at 20px 50%)"),
            Some(vec![rect(10.0, 10.0, 40.0, 40.0, [20.0; 2])])
        );
        assert_eq!(
            mask("ellipse(farthest-side 25% at left)"),
            Some(vec![rect(-90.0, 20.0, 200.0, 20.0, [100.0, 10.0])])
        );
        assert_eq!(
            mask("content-box polygon(evenodd, 0 0, 100% 0, 50% 100%)"),
            Some(vec![Item::Path {
                path: Path(vec![
                    Segment::Move([15.0, 15.0]),
                    Segment::Line([105.0, 15.0]),
                    Segment::Line([60.0, 45.0]),
                ]),
                even_odd: true,
                color: [1.0; 4],
            }])
        );
        assert_eq!(
            mask("path('M0 0 h5')"),
            Some(vec![Item::Path {
                path: Path(vec![
                    Segment::Move([10.0, 10.0]),
                    Segment::Line([15.0, 10.0])
                ]),
                even_odd: false,
                color: [1.0; 4],
            }])
        );
        // Insets that meet leave nothing to show.
        assert_eq!(mask("inset(20px 0)"), None);
    }

    #[test]
    fn color_filters_at_no_effect_are_the_identity() {
        use crate::style::filter::Function as F;
        let none = [
            F::Brightness(1.0),
            F::Contrast(1.0),
            F::Grayscale(0.0),
            F::HueRotate(360.0),
            F::Invert(0.0),
            F::Opacity(1.0),
            F::Saturate(1.0),
            F::Sepia(0.0),
        ];
        for f in none {
            let m = colors(&f).unwrap();
            for (k, v) in m.iter().enumerate() {
                let one = f32::from(u8::from(k < 16 && k % 4 == k / 4));
                assert!((v - one).abs() < 1e-6, "{f:?}: {m:?}");
            }
        }
        assert_eq!(colors(&F::Blur(1.0)), None);
    }

    #[test]
    fn a_box_that_blends_is_a_layer_and_its_stacking_context_one_too() {
        let order = painted(
            vec![
                view(&[("background", "gray")], &[1, 3, 6]),
                view(&[("overflow", "hidden"), ("height", "20px")], &[2]),
                view(
                    &[
                        ("mixBlendMode", "multiply"),
                        ("height", "30px"),
                        ("background", "red"),
                    ],
                    &[],
                ),
                // A stacking context that is no layer of its own groups a box blending in it.
                view(
                    &[
                        ("position", "relative"),
                        ("zIndex", "1"),
                        ("background", "green"),
                    ],
                    &[4, 5],
                ),
                view(&[("height", "10px"), ("background", "blue")], &[]),
                view(
                    &[
                        ("mixBlendMode", "screen"),
                        ("height", "10px"),
                        ("background", "black"),
                    ],
                    &[],
                ),
                // Isolation alone groups nothing.
                view(
                    &[
                        ("isolation", "isolate"),
                        ("height", "5px"),
                        ("background", "red"),
                    ],
                    &[],
                ),
            ],
            &[0],
        );
        assert_eq!(
            order,
            [
                "gray",
                r#"group 1 Multiply ["red"]"#,
                "red",
                r#"group 1 ["green", "blue", "group 1 Screen [\"black\"]"]"#,
            ]
        );
        // The clips around a box that blends go in its layer, so it blends with all under them.
        let (items, _) = paint_of(
            vec![
                view(&[("background", "gray")], &[1]),
                view(&[("overflow", "hidden"), ("height", "20px")], &[2]),
                view(
                    &[
                        ("mixBlendMode", "difference"),
                        ("transform", "rotate(10deg)"),
                        ("height", "30px"),
                        ("background", "red"),
                    ],
                    &[],
                ),
            ],
            &[0],
        );
        let Item::Group { items: layer, .. } = &items[1] else {
            panic!("{items:?}")
        };
        assert!(matches!(&layer[..], [Item::Clip { .. }]), "{layer:?}");
    }

    #[test]
    fn boxes_paint_on_whole_pixels() {
        let at = |left: &str, top: &str, width: &str, height: &str| {
            let style = [
                ("position", "absolute"),
                ("left", left),
                ("top", top),
                ("width", width),
                ("height", height),
                ("background", "red"),
            ];
            view(&style, &[])
        };
        let (items, _) = paint_of(
            vec![
                at("10.5px", "3.4px", "20.25px", "0.5px"),
                at("1.25px", "3.7px", "0.75px", "0.5px"),
                at("0px", "0px", "8px", "0.05px"),
            ],
            &[0, 1, 2],
        );
        let rects: Vec<[f32; 4]> = (items.iter())
            .map(|item| match item {
                Item::Rect { rect: r, .. } => [r.x, r.y, r.width, r.height],
                _ => panic!("{items:?}"),
            })
            .collect();
        // Edges round, halves up; a side that rounds away keeps a pixel unless it is thinner than
        // 4/64 px, which then paints nothing.
        assert_eq!(rects, [[11.0, 3.0, 20.0, 1.0], [1.0, 4.0, 1.0, 1.0]]);
    }

    /// The gradients a box paints, 100 × 50 px and with a 10 px border, with its `background`.
    fn ramps(background: &str) -> Vec<(Rect, Rect, Ramp)> {
        let style = [
            ("width", "100px"),
            ("height", "50px"),
            ("border", "10px solid transparent"),
            ("boxSizing", "content-box"),
            ("background", background),
        ];
        let (items, _) = paint_of(vec![view(&style, &[])], &[0]);
        (items.into_iter())
            .filter_map(|item| match item {
                Item::Gradient {
                    rect, tiles, ramp, ..
                } => Some((rect, tiles.first, ramp)),
                _ => None,
            })
            .collect()
    }

    /// The first tile and the steps of the gradients a box like `ramps`'s paints, rounded.
    fn tiled(background: &str) -> Vec<([f32; 4], [Option<f32>; 2])> {
        let style = [
            ("width", "100px"),
            ("height", "50px"),
            ("border", "10px solid transparent"),
            ("boxSizing", "content-box"),
            ("background", background),
        ];
        let (items, _) = paint_of(vec![view(&style, &[])], &[0]);
        (items.into_iter())
            .filter_map(|item| match item {
                Item::Gradient { tiles, .. } => {
                    let r = tiles.first;
                    let first = [r.x, r.y, r.width, r.height].map(rounded);
                    Some((first, tiles.step.map(|s| s.map(rounded))))
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn layers_are_placed_in_and_clipped_to_their_boxes() {
        // The color's rect, or a gradient's and its first tile.
        let shown = |background: &str| {
            let style = [
                ("width", "100px"),
                ("height", "50px"),
                ("border", "10px solid transparent"),
                ("padding", "5px 10px"),
                ("boxSizing", "content-box"),
                ("background", background),
            ];
            let (items, _) = paint_of(vec![view(&style, &[])], &[0]);
            let xywh = |r: Rect| [r.x, r.y, r.width, r.height];
            (items.into_iter())
                .filter_map(|item| match item {
                    Item::Rect { rect, .. } => Some((xywh(rect), None)),
                    Item::Gradient { rect, tiles, .. } => {
                        Some((xywh(rect), Some(xywh(tiles.first))))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let border = [0.0, 0.0, 140.0, 80.0];
        let padding = [10.0, 10.0, 120.0, 60.0];
        let content = [20.0, 15.0, 100.0, 50.0];
        // Placed in the padding box and shown under the border.
        assert_eq!(
            shown("linear-gradient(red, blue), gray"),
            [(border, None), (border, Some(padding))]
        );
        assert_eq!(
            shown("linear-gradient(red, blue) content-box"),
            [(content, Some(content))]
        );
        // The color shows where the last layer does.
        assert_eq!(
            shown("linear-gradient(red, blue) border-box content-box, gray padding-box"),
            [(padding, None), (content, Some(border))]
        );
    }

    #[test]
    fn tiles_are_sized_placed_and_repeated_in_the_padding_box() {
        let tiles = |b: &str| tiled(&format!("linear-gradient(red, blue) {b}"))[0];
        // As big as the padding box, and repeated.
        assert_eq!(
            tiles(""),
            ([10.0, 10.0, 100.0, 50.0], [Some(100.0), Some(50.0)])
        );
        assert_eq!(tiles("0 0 / cover"), tiles(""));
        // Placed from a side, in the room the tile leaves.
        assert_eq!(
            tiles("right 10px bottom / 20px 50% no-repeat"),
            ([80.0, 35.0, 20.0, 25.0], [None, None])
        );
        assert_eq!(
            tiles("-5px 50% / 20px 10px repeat-y"),
            ([5.0, 30.0, 20.0, 10.0], [None, Some(10.0)])
        );
        // Spaced out from edge to edge, or one where the position puts it.
        assert_eq!(
            tiles("center / 30px 20px space"),
            ([10.0, 10.0, 30.0, 20.0], [Some(35.0), Some(30.0)])
        );
        assert_eq!(
            tiles("center / 60px space"),
            ([30.0, 10.0, 60.0, 50.0], [None, None])
        );
        // Rounded to whole tiles, an auto side kept in proportion.
        assert_eq!(
            tiles("0 0 / 30px 15px round"),
            ([10.0, 10.0, 33.333, 16.667], [Some(33.333), Some(16.667)])
        );
        assert_eq!(
            tiles("0 0 / 30px round no-repeat"),
            ([10.0, 10.0, 33.333, 55.556], [Some(33.333), None])
        );
        // Placed as if it were not rounded.
        assert_eq!(
            tiles("center / 28px 10px round"),
            ([46.0, 30.0, 25.0, 10.0], [Some(25.0), Some(10.0)])
        );
        // No tiles without a size.
        assert!(tiled("linear-gradient(red, blue) 0 0 / 0 10px").is_empty());
    }

    fn rounded(v: f32) -> f32 {
        (v * 1000.0).round() / 1000.0
    }

    /// Where a linear ramp's line starts and ends.
    fn line(ramp: &Ramp) -> [[f32; 2]; 2] {
        match ramp.geometry {
            Geometry::Line(ends) => ends.map(|p| p.map(rounded)),
            _ => panic!("{ramp:?}"),
        }
    }

    /// Where a ramp's segments start and end.
    fn spans(ramp: &Ramp) -> Vec<[f32; 2]> {
        ramp.segments.iter().map(|s| s.at.map(rounded)).collect()
    }

    #[test]
    fn gradients_paint_over_the_color_from_the_padding_box() {
        let (items, _) = paint_of(
            vec![view(
                &[
                    ("width", "40px"),
                    ("height", "20px"),
                    ("border", "2px solid black"),
                    (
                        "background",
                        "linear-gradient(red, blue), linear-gradient(red, green 50%, blue), gray",
                    ),
                ],
                &[],
            )],
            &[0],
        );
        assert_eq!(
            names(&items),
            ["gray", "gradient of 2", "gradient of 1", "ring black"]
        );
        // An image that blends makes the background a layer of its own.
        let (items, _) = paint_of(
            vec![view(
                &[
                    ("height", "20px"),
                    (
                        "background",
                        "linear-gradient(red, blue), linear-gradient(red, green 50%, blue), gray",
                    ),
                    ("backgroundBlendMode", "multiply, normal"),
                ],
                &[],
            )],
            &[0],
        );
        assert_eq!(
            names(&items),
            [r#"group 1 ["gray", "gradient of 2", "group 1 Multiply [\"gradient of 1\"]"]"#]
        );
        let ramps = ramps("linear-gradient(red, blue)");
        let [(rect, tile, ramp)] = &ramps[..] else {
            panic!("{ramps:?}")
        };
        // Under the border too, the image the padding box.
        assert_eq!((rect.width, rect.height), (120.0, 70.0));
        assert_eq!(
            (tile.x, tile.y, tile.width, tile.height),
            (10.0, 10.0, 100.0, 50.0)
        );
        // Top to bottom through the middle.
        assert_eq!(line(ramp), [[50.0, 0.0], [50.0, 50.0]]);
        assert_eq!(spans(ramp), [[0.0, 1.0]]);
    }

    #[test]
    fn gradient_lines_span_the_image_corner_to_corner() {
        let line = |g: &str| line(&ramps(g)[0].2);
        assert_eq!(
            line("linear-gradient(45deg, red, blue)"),
            [[12.5, 62.5], [87.5, -12.5]]
        );
        // Square to the diagonal from the top left to the bottom right, so both are halfway.
        assert_eq!(
            line("linear-gradient(to top right, red, blue)"),
            [[30.0, 65.0], [70.0, -15.0]]
        );
        assert_eq!(
            line("linear-gradient(to left, red, blue)"),
            [[100.0, 25.0], [0.0, 25.0]]
        );
    }

    #[test]
    fn radial_gradients_end_at_a_side_a_corner_or_their_radii() {
        // The center and radii in an image 100 × 50 px, and where the stops are.
        let ellipse = |g: &str| {
            let ramp = ramps(g).remove(0).2;
            match ramp.geometry {
                Geometry::Ellipse(c, r) => (c.map(rounded), r.map(rounded), spans(&ramp)),
                _ => panic!("{ramp:?}"),
            }
        };
        let shape = |g: &str| {
            let (c, r, _) = ellipse(g);
            (c, r)
        };
        // Through the farthest corner, as the farthest sides are in proportion.
        assert_eq!(
            shape("radial-gradient(red, blue)"),
            ([50.0, 25.0], [70.711, 35.355])
        );
        assert_eq!(
            shape("radial-gradient(circle closest-side at 20px 10px, red, blue)"),
            ([20.0, 10.0], [10.0, 10.0])
        );
        assert_eq!(
            shape("radial-gradient(circle farthest-side at 20px 10px, red, blue)").1,
            [80.0, 80.0]
        );
        assert_eq!(
            shape("radial-gradient(circle at 20px 10px, red, blue)").1,
            [89.443, 89.443]
        );
        assert_eq!(
            shape("radial-gradient(closest-corner at 20px 10px, red, blue)").1,
            [28.284, 14.142]
        );
        assert_eq!(
            shape("radial-gradient(farthest-side at right 10px bottom 20%, red, blue)"),
            ([90.0, 40.0], [90.0, 40.0])
        );
        // Sides measured from a center outside the image.
        assert_eq!(
            shape("radial-gradient(closest-side at -10px 25px, red, blue)").1,
            [10.0, 25.0]
        );
        // Stops along the horizontal radius; percentages of the image's sides.
        assert_eq!(
            ellipse("radial-gradient(10px 50%, red 5px, blue)"),
            ([50.0, 25.0], [10.0, 25.0], vec![[0.5, 1.0]])
        );
        // A radius of 0 is the last color all around.
        assert_eq!(
            shape("radial-gradient(circle 0px, red, blue)").1,
            [0.001, 0.001]
        );
    }

    #[test]
    fn conic_gradients_turn_around_their_center() {
        let ramp = ramps("conic-gradient(from 90deg at 20px 10px, red, blue 25%, green 180deg)")
            .remove(0)
            .2;
        assert_eq!(ramp.geometry, Geometry::Turn([20.0, 10.0], 0.25));
        // Stops are fractions of a turn.
        assert_eq!(spans(&ramp), [[0.0, 0.25], [0.25, 0.5]]);
        let ramp = ramps("repeating-conic-gradient(red 0 10deg, blue 10deg 20deg)")
            .remove(0)
            .2;
        assert_eq!(ramp.geometry, Geometry::Turn([50.0, 25.0], 0.0));
        assert_eq!(spans(&ramp), [[0.0, 0.028], [0.028, 0.028], [0.028, 0.056]]);
        assert_eq!(ramp.spread, Spread::Repeat);
    }

    #[test]
    fn stops_go_where_css_puts_them() {
        let ramp = |g: &str| ramps(g).remove(0).2;
        let (red, green, blue) = (
            work(color::parse("red").unwrap()),
            work(color::parse("green").unwrap()),
            work(color::parse("blue").unwrap()),
        );
        // Black is no earlier than the hint before it, green halfway between blue and black, and
        // the hint at black keeps the way to it green.
        let stops = ramp("linear-gradient(to right, red, blue 30%, green, 60%, black 50px, gray)");
        assert_eq!(
            spans(&stops),
            [[0.0, 0.3], [0.3, 0.45], [0.45, 0.6], [0.6, 0.6], [0.6, 1.0]]
        );
        assert_eq!(stops.segments[2].colors, [green; 2]);
        // Half and half a quarter of the way along, lines between nine points on the curve.
        let hinted = ramp("linear-gradient(to right, red, 25%, blue)");
        assert_eq!(
            spans(&hinted)[..4],
            [[0.0, 0.083], [0.083, 0.167], [0.167, 0.25], [0.25, 0.308]]
        );
        assert_eq!(spans(&hinted)[9], [0.596, 1.0]);
        assert_eq!(
            hinted.segments[3].colors[0].map(rounded),
            [0.5, 0.0, 0.5, 1.0]
        );
        // Points closer together toward the stop the hint is near.
        let late = ramp("linear-gradient(to right, red, 75%, blue)");
        assert_eq!(
            spans(&late)[7..],
            [[0.75, 0.833], [0.833, 0.917], [0.917, 1.0]]
        );
        // A hint at a stop changes colors there; one halfway changes nothing.
        let sudden = ramp("linear-gradient(to right, red, 0%, blue)");
        assert_eq!(spans(&sudden), [[0.0, 0.0], [0.0, 1.0]]);
        assert_eq!(sudden.segments[1].colors, [blue; 2]);
        let even = ramp("linear-gradient(to right, red, 50%, blue)");
        assert_eq!(even.segments[0].colors, [red, blue]);
        let one = ramp("linear-gradient(red 20%)");
        assert_eq!((spans(&one), one.spread), (vec![[0.2, 0.2]], Spread::Pad));
        let repeating = ramp("repeating-linear-gradient(to right, red 10px, blue 30px)");
        assert_eq!(spans(&repeating), [[0.1, 0.3]]);
        assert_eq!(repeating.spread, Spread::Repeat);
        // Stops 0 px apart that repeat are the last color, as in Chrome.
        let flat = ramp("repeating-linear-gradient(red 10px, blue 10px)");
        assert_eq!(flat.segments[0].colors, [blue; 2]);
        assert_eq!(flat.spread, Spread::Pad);
    }

    #[test]
    fn gradient_colors_mix_premultiplied_in_their_space() {
        let ends = |g: &str| ramps(g).remove(0).2.segments[0].colors;
        let both = |c: [[f32; 4]; 2]| c.map(|c| c.map(rounded));
        assert_eq!(
            ends("linear-gradient(rgb(255 0 0 / 50%), transparent)"),
            [[0.5, 0.0, 0.0, 0.5], [0.0; 4]]
        );
        assert_eq!(
            both(ends("linear-gradient(in srgb-linear, #808080, red)")),
            [[0.216, 0.216, 0.216, 1.0], [1.0, 0.0, 0.0, 1.0]]
        );
        // Oklab's red, as CSS Color 4 gives it.
        assert_eq!(
            both(ends("linear-gradient(in oklab, red, red)"))[0],
            [0.628, 0.225, 0.126, 1.0]
        );
        // Oklch: lightness and chroma premultiplied, hue in degrees. White's hue is powerless, so it
        // takes blue's; the shorter way from red to blue goes down through 0.
        let [white, blue] = ends("linear-gradient(in oklch, white, blue)");
        assert_eq!(white[2], blue[2]);
        let [red, blue] = ends("linear-gradient(in oklch, red, rgb(0 0 255 / 50%))");
        assert_eq!(
            [red, blue].map(|c| (c[2] * 10.0).round() / 10.0),
            [389.2, 264.1]
        );
        assert_eq!(rounded(blue[0]), 0.226);
        let [red, blue] = ends("linear-gradient(in oklch longer hue, red, blue)");
        assert_eq!([red, blue].map(|c| c[2].round()), [29.0, 264.0]);
        // CIE Lab's red, D50 white, as CSS Color 4 gives it; and in LCH a gray's hue is powerless.
        assert_eq!(
            both(ends("linear-gradient(in lab, red, red)"))[0],
            [54.291, 80.805, 69.891, 1.0]
        );
        // A wide color keeps its channels past sRGB until the gradient's pixels clip them.
        let [green, _] = ends("linear-gradient(in srgb, color(display-p3 0 1 0), red)");
        assert!(green[0] < 0.0 && green[1] > 1.0, "{green:?}");
        let [red, gray] = ends("linear-gradient(in lch, red, gray)");
        assert_eq!(
            [red, gray].map(|c| (c[2] * 10.0).round() / 10.0),
            [40.9, 40.9]
        );
    }

    #[test]
    fn glyphs_sit_in_the_content_box() {
        let (items, laid) = paint_of(
            vec![
                view(&[("paddingLeft", "30px")], &[1]),
                text_node("Hi there", &[("padding", "5px"), ("color", "blue")]),
            ],
            &[0],
        );
        let lines = laid.lines[1].as_ref().unwrap();
        let [
            Item::Glyphs {
                face,
                size,
                color: c,
                stroke: 0.0,
                glyphs,
            },
        ] = &items[..]
        else {
            panic!("{items:?}")
        };
        assert_eq!((*face, *size), (0, 16.0));
        assert_eq!(*c, work(color::parse("blue").unwrap()));
        let expected: Vec<(u32, [f32; 2])> = lines
            .glyphs
            .iter()
            .map(|g| (g.id, [35.0 + g.x, 5.0 + g.y]))
            .collect();
        assert_eq!(*glyphs, expected);
    }

    #[test]
    fn a_transition_paints_its_children_into_its_shader_over_its_background() {
        let code: Rc<str> =
            crate::transition::wrap(crate::transition::kind("fade").unwrap().0).into();
        let shader = Kind::Shader {
            code,
            uniforms: vec![("progress".into(), vec![Prop::Const(V::Num(0.5))])],
            inputs: Vec::new(),
            outset: 0.0,
            transition: true,
            kind: Some("fade".into()),
        };
        let cell = |c| view(&[("isolation", "isolate")], &[c]);
        let order = painted(
            vec![
                node(
                    shader,
                    &[
                        ("width", "20px"),
                        ("height", "10px"),
                        ("background", "gray"),
                    ],
                    &[1, 2],
                ),
                cell(3),
                cell(4),
                view(&[("height", "10px"), ("background", "red")], &[]),
                view(&[("height", "10px"), ("background", "blue")], &[]),
            ],
            &[0],
        );
        assert_eq!(
            order,
            [
                "gray",
                r#"shader ["valle_from [\"red\"]", "valle_to [\"blue\"]"]"#
            ]
        );
    }
}
