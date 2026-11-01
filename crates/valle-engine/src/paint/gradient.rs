//! Gradients as the GPU draws them: the line, ellipse or turn in px their colors go along, and the
//! parts of it between color stops, their colors in the space they mix in.

use crate::style::color::{self, Rgba};
use crate::style::gradient::{Gradient, Hue, Line, Shape, Size, Space, Stop};
use crate::style::position::Offset;

#[derive(Clone, Debug, PartialEq)]
pub struct Ramp {
    pub geometry: Geometry,
    pub spread: Spread,
    pub space: Space,
    /// Whether colors mix premultiplied, as CSS mixes them, or not, as SVG does.
    pub premultiplied: bool,
    /// The parts of the line between color stops, in order, 0 at its start and 1 at its end.
    pub segments: Vec<Segment>,
}

/// What is past the segments: before the first its first color and after the last its last,
/// the segments repeated, or repeated each the other way round.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Spread {
    Pad,
    Repeat,
    Reflect,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    pub at: [f32; 2],
    /// The colors at either end, in the ramp's space, premultiplied but for a hue if the ramp
    /// is.
    pub colors: [[f32; 4]; 2],
}

/// Where a ramp's colors go, px from the image's top left.
#[derive(Clone, Debug, PartialEq)]
pub enum Geometry {
    /// From the start of a line to its end.
    Line([[f32; 2]; 2]),
    /// Out from a center to an ellipse with these horizontal and vertical radii.
    Ellipse([f32; 2], [f32; 2]),
    /// Clockwise around a center, from a fraction of a turn clockwise from up.
    Turn([f32; 2], f32),
    /// From a circle around a focus, with a radius, to another around a center: SVG's radial
    /// gradient with its focus away from its center, a cone where one circle is not in the
    /// other.
    Focal {
        focus: [f32; 2],
        focus_radius: f32,
        center: [f32; 2],
        radius: f32,
    },
}

impl Geometry {
    /// The geometry moved by `by`.
    pub fn moved(&self, by: [f32; 2]) -> Geometry {
        let p = |q: [f32; 2]| [q[0] + by[0], q[1] + by[1]];
        match *self {
            Geometry::Line([a, b]) => Geometry::Line([p(a), p(b)]),
            Geometry::Ellipse(center, radii) => Geometry::Ellipse(p(center), radii),
            Geometry::Turn(center, from) => Geometry::Turn(p(center), from),
            Geometry::Focal {
                focus,
                focus_radius,
                center,
                radius,
            } => Geometry::Focal {
                focus: p(focus),
                focus_radius,
                center: p(center),
                radius,
            },
        }
    }
}

/// The chroma below which a hue is powerless, as CSS Color 4 takes it.
const ACHROMATIC: f64 = 0.000004;

/// The smallest radius an ending shape has: one of 0 is the last color all around, as CSS has it.
const POINT: f32 = 0.001;

/// A gradient in an image `size` px.
pub fn ramp(g: &Gradient, size: [f32; 2]) -> Ramp {
    let center = |at: &[Offset; 2]| [at[0].resolve(size[0]), at[1].resolve(size[1])];
    let (geometry, len) = match &g.shape {
        Shape::Linear(line) => linear(*line, size),
        Shape::Radial {
            circle,
            size: s,
            at,
        } => radial(*circle, s, center(at), size),
        // Stops are degrees, as px of a line 360 long.
        Shape::Conic { from, at } => (Geometry::Turn(center(at), (from / 360.0) as f32), 360.0),
    };
    let (segments, repeating) = segments(g, len);
    Ramp {
        geometry,
        spread: match repeating {
            true => Spread::Repeat,
            false => Spread::Pad,
        },
        space: g.space,
        premultiplied: true,
        segments,
    }
}

/// A linear gradient's line, through the middle of the image, as long as the image's corners
/// are far apart along it, and its length.
fn linear(line: Line, size: [f32; 2]) -> (Geometry, f32) {
    let [w, h] = size;
    let [dx, dy] = match line {
        Line::Angle(degrees) => {
            let a = degrees.to_radians() as f32;
            [a.sin(), -a.cos()]
        }
        Line::Corner([x, y]) => {
            let d = [x as f32 * h, y as f32 * w];
            let n = d[0].hypot(d[1]);
            [d[0] / n, d[1] / n]
        }
    };
    let len = (w * dx).abs() + (h * dy).abs();
    let half = [dx * len / 2.0, dy * len / 2.0];
    let middle = [w / 2.0, h / 2.0];
    let ends = [
        [middle[0] - half[0], middle[1] - half[1]],
        [middle[0] + half[0], middle[1] + half[1]],
    ];
    (Geometry::Line(ends), len)
}

/// A radial gradient's ending shape around `center`, and its horizontal radius, along which its
/// stops are. Sized by a side or a corner, it meets the sides nearest the center or farthest
/// from it, or passes through the corner where they meet: an ellipse through a corner keeps
/// the proportions it has at the sides.
fn radial(circle: bool, size: &Size, center: [f32; 2], image: [f32; 2]) -> (Geometry, f32) {
    let [x, y] = [0, 1].map(|k| [center[k].abs(), (image[k] - center[k]).abs()]);
    let near = [x[0].min(x[1]), y[0].min(y[1])];
    let far = [x[0].max(x[1]), y[0].max(y[1])];
    let radii = match size {
        Size::ClosestSide | Size::FarthestSide => {
            let sides = if *size == Size::ClosestSide {
                near
            } else {
                far
            };
            match circle {
                true if *size == Size::ClosestSide => [sides[0].min(sides[1]); 2],
                true => [sides[0].max(sides[1]); 2],
                false => sides,
            }
        }
        Size::ClosestCorner | Size::FarthestCorner => {
            let sides = if *size == Size::ClosestCorner {
                near
            } else {
                far
            };
            match circle {
                true => [sides[0].hypot(sides[1]); 2],
                false => sides.map(|s| s * std::f32::consts::SQRT_2),
            }
        }
        Size::Radii([rx, ry]) => [rx.resolve(image[0]), ry.resolve(image[1])],
    };
    let radii = radii.map(|r| r.max(POINT));
    (Geometry::Ellipse(center, radii), radii[0])
}

/// The parts between the color stops of a gradient whose line is `len` px, and whether they
/// repeat. Stops go where CSS puts them: the first and last at the ends of the line if they do
/// not say, none before one ahead of it, and those without positions spread evenly between the
/// ones with. Repeating stops 0 px apart are the last color, as in Chrome. A hint splits the part
/// it is in as `hinted` has it.
fn segments(g: &Gradient, len: f32) -> (Vec<Segment>, bool) {
    let mut at: Vec<Option<f32>> = (g.stops.iter())
        .map(|s| match s {
            Stop::Color(_, p) => p.as_ref().map(|p| p.resolve(len) / len),
            Stop::Hint(p) => Some(p.resolve(len) / len),
        })
        .collect();
    let colors: Vec<usize> = (0..g.stops.len())
        .filter(|&i| matches!(g.stops[i], Stop::Color(..)))
        .collect();
    let (first, last) = (colors[0], colors[colors.len() - 1]);
    at[first] = at[first].or(Some(0.0));
    at[last] = at[last].or(Some(1.0));
    let mut most = f32::MIN;
    for p in at.iter_mut().flatten() {
        most = most.max(*p);
        *p = most;
    }
    let mut from = 0;
    for (k, &i) in colors.iter().enumerate().skip(1) {
        if let Some(to) = at[i] {
            let (a, b) = (at[colors[from]].unwrap_or(0.0), to);
            for j in from + 1..k {
                at[colors[j]] = Some(a + (b - a) * (j - from) as f32 / (k - from) as f32);
            }
            from = k;
        }
    }
    let position = |i: usize| at[i].unwrap_or(0.0);
    let color = |i: usize| match g.stops[i] {
        Stop::Color(c, _) => c,
        Stop::Hint(_) => unreachable!("a color stop"),
    };
    let period = position(last) - position(first);
    if g.repeating && period <= 0.0 {
        let c = ends([mixed(color(last), g.space); 2], g.space);
        return (
            vec![Segment {
                at: [0.0; 2],
                colors: c,
            }],
            false,
        );
    }
    let mut out = Vec::new();
    for pair in colors.windows(2) {
        let (i, j) = (pair[0], pair[1]);
        let c = ends(
            [mixed(color(i), g.space), mixed(color(j), g.space)],
            g.space,
        );
        let hint = (i + 1..j).find(|&h| matches!(g.stops[h], Stop::Hint(_)));
        let points = hinted(position(i), position(j), hint.map(position));
        let mix = |k: f32| std::array::from_fn(|n| c[0][n] + (c[1][n] - c[0][n]) * k);
        out.extend(points.windows(2).map(|w| Segment {
            at: [w[0].0, w[1].0],
            colors: [mix(w[0].1), mix(w[1].1)],
        }));
    }
    if out.is_empty() {
        let p = position(first);
        let c = ends([mixed(color(first), g.space); 2], g.space);
        out.push(Segment {
            at: [p; 2],
            colors: c,
        });
    }
    (out, g.repeating)
}

/// Points between color stops at `a` and `b`, and how much of the second color there is at
/// each: at the stops, and with a hint at `h`, where half of each is, at nine more points on
/// the curve CSS gives, which browsers draw lines between (WebKit's approximation, which Blink
/// and Gecko share). A hint at either stop makes the change there sudden.
fn hinted(a: f32, b: f32, h: Option<f32>) -> Vec<(f32, f32)> {
    let Some(h) = h else {
        return vec![(a, 0.0), (b, 1.0)];
    };
    let (left, right) = (h - a, b - h);
    if (left - right).abs() <= f32::EPSILON {
        return vec![(a, 0.0), (b, 1.0)];
    }
    if left <= f32::EPSILON {
        return vec![(a, 0.0), (a, 1.0), (b, 1.0)];
    }
    if right <= f32::EPSILON {
        return vec![(a, 0.0), (b, 0.0), (b, 1.0)];
    }
    let half = left / (b - a);
    // More points on the side of the hint where the curve bends most.
    let at: [f32; 9] = std::array::from_fn(|n| match half > 0.5 {
        true if n < 7 => a + left * (7 + n) as f32 / 13.0,
        true => h + right * (n - 6) as f32 / 3.0,
        false if n < 2 => a + left * (n + 1) as f32 / 3.0,
        false => h + right * (n - 2) as f32 / 13.0,
    });
    let power = 0.5f32.ln() / half.ln();
    let curve = at.map(|p| (p, ((p - a) / (b - a)).powf(power)));
    [(a, 0.0)]
        .into_iter()
        .chain(curve)
        .chain([(b, 1.0)])
        .collect()
}

/// A color in `space`, straight, and for Oklch its hue in degrees, `None` where it is powerless.
fn mixed(c: Rgba, space: Space) -> ([f64; 4], Option<f64>) {
    let rgb = [c[0], c[1], c[2]];
    let ([x, y, z], hue) = match space {
        Space::Srgb => (rgb, None),
        Space::SrgbLinear => (rgb.map(color::linear), None),
        Space::Oklab => (color::oklab(rgb), None),
        Space::Oklch(_) => {
            let [l, a, b] = color::oklab(rgb);
            let chroma = a.hypot(b);
            let hue = (chroma > ACHROMATIC).then(|| b.atan2(a).to_degrees().rem_euclid(360.0));
            ([l, chroma, 0.0], hue)
        }
    };
    ([x, y, z, c[3]], hue)
}

/// Two colors as `mixed` gives them, premultiplied, their hues going around the circle as
/// `space` has them: a powerless hue takes the other's.
fn ends(colors: [([f64; 4], Option<f64>); 2], space: Space) -> [[f32; 4]; 2] {
    let [(from, a), (to, b)] = colors;
    let hues = match space {
        Space::Oklch(way) => {
            let [a, b] = [a.or(b).unwrap_or(0.0), b.or(a).unwrap_or(0.0)];
            let d = b - a;
            match way {
                Hue::Shorter if d > 180.0 => [a + 360.0, b],
                Hue::Shorter if d < -180.0 => [a, b + 360.0],
                Hue::Longer if d > 0.0 && d < 180.0 => [a + 360.0, b],
                Hue::Longer if d > -180.0 && d <= 0.0 => [a, b + 360.0],
                Hue::Increasing if b < a => [a, b + 360.0],
                Hue::Decreasing if a < b => [a + 360.0, b],
                _ => [a, b],
            }
        }
        _ => [0.0; 2],
    };
    let premultiplied = |c: [f64; 4], hue: f64| {
        let k = c[3];
        let third = match space {
            Space::Oklch(_) => hue,
            _ => c[2] * k,
        };
        [c[0] * k, c[1] * k, third, k].map(|v| v as f32)
    };
    [premultiplied(from, hues[0]), premultiplied(to, hues[1])]
}
