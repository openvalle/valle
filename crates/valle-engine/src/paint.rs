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

mod dash;
mod gradient;
mod matrix;
mod offset;
mod scene;
mod shape;
mod svg;

use taffy::{Display, Overflow, Position};

pub use gradient::{Geometry, Ramp, Segment, Spread};
pub use matrix::{Matrix, Matrix4};

use crate::image::Picture;
use crate::layout::{Boxes, Layout, Natural, Rect, ordered};
use crate::path::Path;
use crate::style::background::{Area, Composite, ImageSize, Layer, Repeat};
use crate::style::color::{self, Rgba};
use crate::style::filter;
use crate::style::gradient::Image;
use crate::style::transform::Function;
use crate::style::{Blend, BorderStyle, DecorationStyle, Fit, Shadow, Style};
use crate::svg::Element;
use crate::template::{Kind, Template};
use crate::text::{Glyph, Stripe};
use svg::Drawing;

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

/// A line across a side of a border or an outline: from and to how deep in it, and its color.
struct Line {
    depth: [f32; 2],
    color: [f32; 4],
}

/// The lines across each side of a border or an outline `widths` wide in `styles` and `colors`, as
/// Chrome draws them. A double line is two lines and a gap, about a third of its width each (the
/// gap from (w + 1) / 3 to (2w + 1) / 3 deep, rounded down), or one under 3 px. An inset line is
/// in the darker shade of its color at the top and left and the lighter at the bottom and right,
/// an outset line the other way round; a groove is an inset line outside an outset one, a ridge
/// the other way round, the side split with the odd px at its top or left, or solid under 2 px.
/// An opaque outer half goes on under the inner one, which leaves no seam where their antialiased
/// edges meet; Chrome's translucent ones meet in a layer, which is left out. Dots and dashes are
/// no lines across a side, but marks along it.
fn lines(widths: [f32; 4], styles: [BorderStyle; 4], colors: [Rgba; 4]) -> [Vec<Line>; 4] {
    std::array::from_fn(|k| {
        let (w, c) = (widths[k], colors[k]);
        let top_left = k == 0 || k == 3;
        let shade = |inset: bool| work(color::shades(c)[usize::from(inset != top_left)]);
        let half = if top_left {
            (w / 2.0).ceil()
        } else {
            (w / 2.0).floor()
        };
        let line = |from, to, color| Line {
            depth: [from, to],
            color,
        };
        let under = if c[3] >= 1.0 { w } else { half };
        match styles[k] {
            BorderStyle::Double if w >= 3.0 => vec![
                line(0.0, ((w + 1.0) / 3.0).floor(), work(c)),
                line(((2.0 * w + 1.0) / 3.0).floor(), w, work(c)),
            ],
            BorderStyle::Groove if w >= 2.0 => {
                vec![line(0.0, under, shade(true)), line(half, w, shade(false))]
            }
            BorderStyle::Ridge if w >= 2.0 => {
                vec![line(0.0, under, shade(false)), line(half, w, shade(true))]
            }
            BorderStyle::Inset => vec![line(0.0, w, shade(true))],
            BorderStyle::Outset => vec![line(0.0, w, shade(false))],
            BorderStyle::Dotted | BorderStyle::Dashed => Vec::new(),
            _ => vec![line(0.0, w, work(c))],
        }
    })
}

/// The rings a border or an outline `widths` deep inside `rect` paints: its sides' `lines`.
fn rings(rect: Rect, radii: Radii, widths: [f32; 4], lines: &[Vec<Line>; 4]) -> Vec<Item> {
    let count = lines.iter().map(Vec::len).max().unwrap_or(0);
    (0..count)
        .map(|n| {
            // A side with fewer lines has none here. One of dots or dashes has none at all, but
            // keeps its part of the border, where the next sides' corners stop.
            let line = |k: usize| lines[k].get(n);
            let none = |k: usize| match lines[k].is_empty() {
                true => [0.0, widths[k]],
                false => [widths[k]; 2],
            };
            let depth = |k: usize| line(k).map_or(none(k), |l| l.depth);
            let (outer, outer_radii) = inner(rect, radii, std::array::from_fn(|k| depth(k)[0]));
            let (inside, inner_radii) = inner(rect, radii, std::array::from_fn(|k| depth(k)[1]));
            Item::Ring {
                rect: outer,
                radii: outer_radii,
                inner: inside,
                inner_radii,
                colors: std::array::from_fn(|k| line(k).map_or([0.0; 4], |l| l.color)),
            }
        })
        .collect()
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
    let n = template.nodes.len();
    let parents = parents(template);
    let mut painter = Painter {
        template,
        styles,
        layout,
        pictures,
        spaces: spaces(template, styles, layout),
        parents,
        clips: Vec::with_capacity(n),
        drawing: Drawing::new(template, styles, &[]),
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
    let spaces = spaces(template, styles, layout);
    let mut out = vec![Matrix::IDENTITY; template.nodes.len()];
    // What the plane each box is flattened onto goes to on the canvas.
    let mut onto = out.clone();
    // In preorder, so a parent's comes first.
    for (i, node) in template.nodes.iter().enumerate() {
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
/// transforms of the boxes between.
fn spaces(template: &Template, styles: &[Style], layout: &Layout) -> Vec<Matrix4> {
    let n = template.nodes.len();
    let parents = parents(template);
    let mut out = vec![Matrix4::IDENTITY; n];
    // What its parent puts before its own: the parent's perspective, and in a 3D scene the
    // parent's space.
    let mut seen = vec![None; n];
    let mut base = out.clone();
    // In preorder, so a parent's comes first.
    for (i, node) in template.nodes.iter().enumerate() {
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
        (list.into_iter()).filter(|&c| self.styles[c].layout.display != Display::None)
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
            self.context(Some(d), &mut inside);
            // It contains all in it, so the clips around it go around all it paints.
            let mut items = self.nest_from(self.chained(inside), self.clips[d].len());
            if !s.filter.is_empty() && !items.is_empty() {
                let filters = filters(&s.filter, snap(self.layout.boxes[d].border));
                items = vec![Item::Filter { filters, items }];
            }
            items = self.over_backdrop(d, items);
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
            out.extend(group(s.opacity as f32, s.blend, items).map(|g| (None, g)));
        } else {
            out.extend(drawn);
        }
    }

    /// Whether stacking context `d` paints as one layer: it has an opacity, blends or filters its
    /// backdrop, or a box in it blends, which blends with what the context paints alone, as CSS
    /// isolates a stacking context.
    fn grouped(&self, d: usize) -> bool {
        let s = &self.styles[d];
        s.opacity < 1.0 || s.blend != Blend::Normal || !s.backdrop.is_empty() || {
            let mut layered = Vec::new();
            self.collect(Some(d), &mut layered);
            (layered.iter()).any(|&b| self.styles[b].blend != Blend::Normal)
        }
    }

    /// `items` over what is under `d` through its backdrop filter, in its border box, if it has
    /// one. They are a layer of their own, so what is in the box sees what it paints, not its
    /// backdrop, as in Chrome.
    fn over_backdrop(&self, d: usize, items: Vec<Item>) -> Vec<Item> {
        let s = &self.styles[d];
        let rect = snap(self.layout.boxes[d].border);
        if s.backdrop.is_empty() || rect.width <= 0.0 || rect.height <= 0.0 {
            return items;
        }
        let backdrop = Item::Backdrop {
            filters: filters(&s.backdrop, rect),
            rect,
            radii: self.radii(d, rect),
        };
        let inside = (!items.is_empty()).then_some(Item::Group {
            opacity: 1.0,
            blend: Blend::Normal,
            items,
        });
        std::iter::once(backdrop).chain(inside).collect()
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
        for (i, cuts) in scene::order(&rects) {
            let (space, _, items) = &planes[i];
            let items = (cuts.into_iter())
                .fold(items.clone(), |items, line| vec![Item::Cut { line, items }]);
            let item = Item::Transform {
                matrix: space.flatten(),
                items,
            };
            out.push((self.clips[d].last().copied(), item));
        }
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
            if !s.filter.is_empty() && !items.is_empty() {
                let filters = filters(&s.filter, snap(self.layout.boxes[e].border));
                items = vec![Item::Filter { filters, items }];
            }
            items = self.over_backdrop(e, items);
            items = self.masked(e, self.clipped(e, items));
            if self.grouped(e) {
                items = group(s.opacity as f32, s.blend, items)
                    .into_iter()
                    .collect();
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
                // The stroke and the fill under it cast one shadow: both opaque, in one layer.
                let layer = shadow.filter(|c| stroke.is_some() && c[3] < 1.0);
                let shadow = match layer {
                    Some(c) => Some(c.map(|v| v / c[3])),
                    None => shadow,
                };
                let mut items: Vec<Item> = drawn(0.0, shadow.unwrap_or(fill), by).collect();
                if let Some((width, color)) = stroke {
                    items.extend(drawn(width, shadow.unwrap_or(color), by));
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
            self.shadowed(s, stripes(false), clip, out);
            self.shadowed(s, glyphs, clip, out);
            self.shadowed(s, stripes(true), clip, out);
        }
    }

    /// `i`'s image if it is an Image: in its content box as its object fit and position have
    /// it, the box clipping it with its corners. An SVG file without a natural size is as big as
    /// its ratio fits in the box, or the box without one, before it fits.
    fn image(&self, i: usize, out: &mut Drawn) {
        let Kind::Image { resource } = self.template.nodes[i].kind else {
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
                let Some(drawn) = svg::document(document, first) else {
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
mod tests;
