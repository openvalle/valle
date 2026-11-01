//! Gradients as the GPU draws them: the gradient line in px, and the parts of it between color
//! stops, their colors in the space they mix in.

use crate::style::color::{self, Rgba};
use crate::style::gradient::{Gradient, Hue, Line, Space, Stop};

#[derive(Clone, Debug, PartialEq)]
pub struct Ramp {
    /// Where the gradient line starts and ends, px from the image's top left.
    pub line: [[f32; 2]; 2],
    pub repeating: bool,
    pub space: Space,
    /// The parts of the line between color stops, in order, 0 at its start and 1 at its end.
    /// Before the first is its first color, after the last its last, unless they repeat.
    pub segments: Vec<Segment>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    pub at: [f32; 2],
    /// The colors at either end, in the ramp's space, premultiplied but for a hue.
    pub colors: [[f32; 4]; 2],
}

/// The chroma below which a hue is powerless, as CSS Color 4 takes it.
const ACHROMATIC: f64 = 0.000004;

/// A linear gradient across an image `size` px: its line through the image's middle, as long as
/// the image's corners are far apart along it.
pub fn linear(g: &Gradient, size: [f32; 2]) -> Ramp {
    let [w, h] = size;
    let [dx, dy] = match g.line {
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
    let (segments, repeating) = segments(g, len);
    Ramp {
        line: [
            [middle[0] - half[0], middle[1] - half[1]],
            [middle[0] + half[0], middle[1] + half[1]],
        ],
        repeating,
        space: g.space,
        segments,
    }
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
