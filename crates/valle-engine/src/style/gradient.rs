//! Background images: gradients so far. A gradient is a line through its image, the color space
//! its colors mix in, and color stops along the line, with hints between them.

use super::color::{self, Rgba};
use super::math::{self, Calc};
use super::{Accept, Len, Units, css, current_color, length_text};
use crate::template::V;

/// A `background-image` layer, its stop positions `P` (declared lengths, then px and fractions
/// of the gradient line) and colors `C` (declared, `None` is `currentcolor`).
#[derive(Clone, Debug, PartialEq)]
pub enum Image<P = Calc, C = Rgba> {
    None,
    Gradient(Gradient<P, C>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Gradient<P = Calc, C = Rgba> {
    pub line: Line,
    pub repeating: bool,
    pub space: Space,
    pub stops: Vec<Stop<P, C>>,
}

/// Which way a linear gradient's line goes, through the middle of its image.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Line {
    /// Degrees clockwise from up.
    Angle(f64),
    /// Toward a corner, -1 or 1 on each axis: the line is then square to the diagonal between
    /// the two corners beside it.
    Corner([f64; 2]),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Stop<P, C> {
    /// A color at a position, or spread evenly between the positions of the stops around it.
    Color(C, Option<P>),
    /// Where the colors of the stops on either side mix half and half.
    Hint(P),
}

/// The color space a gradient's colors mix in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Space {
    Srgb,
    SrgbLinear,
    Oklab,
    Oklch(Hue),
}

/// Which way hues go around the circle from one stop to the next.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hue {
    Shorter,
    Longer,
    Increasing,
    Decreasing,
}

/// An image as declared.
pub(super) type Declared = Image<Len, Option<Rgba>>;

/// Stop positions and hints: px or percentages of the gradient line, either side of it.
const POSITION: Accept = Accept {
    keyword: None,
    negative: true,
    percent: true,
    sizing: &[],
};

/// `none` or a gradient.
pub(super) fn image(s: &str) -> Result<Declared, String> {
    if s == "none" {
        return Ok(Image::None);
    }
    let Some((name, args)) = css::function(s) else {
        return Err(format!("{s} is not an image"));
    };
    let at = |e: String| format!("{name}(): {e}");
    match name {
        "linear-gradient" => linear(args, false).map_err(at),
        "repeating-linear-gradient" => linear(args, true).map_err(at),
        "radial-gradient"
        | "repeating-radial-gradient"
        | "conic-gradient"
        | "repeating-conic-gradient" => Err(format!("{name}() is not supported yet")),
        "url" => Err("url() images are not supported yet".into()),
        _ => Err(format!("{s} is not an image")),
    }
    .map(Image::Gradient)
}

/// A linear gradient's arguments: which way it goes, its color space, and its stops.
fn linear(args: &str, repeating: bool) -> Result<Gradient<Len, Option<Rgba>>, String> {
    let parts = css::commas(args);
    let (mut line, mut space) = (None, None);
    let mut stops = &parts[..];
    if let Some(first) = parts.first() {
        let words = css::words(first);
        if words
            .first()
            .is_some_and(|w| ["to", "in"].contains(w) || angle(w).is_some())
        {
            let mut i = 0;
            while i < words.len() {
                let used = match words[i] {
                    "in" if space.is_none() => interpolation(&words[i + 1..], &mut space)?,
                    "to" if line.is_none() => to(&words[i + 1..], &mut line)?,
                    w if line.is_none() && angle(w).is_some() => {
                        line = angle(w).map(Line::Angle);
                        0
                    }
                    w => {
                        return Err(format!(
                            "{w}: a gradient starts with an angle or a side, and a color space"
                        ));
                    }
                };
                i += 1 + used;
            }
            stops = &parts[1..];
        }
    }
    Ok(Gradient {
        line: line.unwrap_or(Line::Angle(180.0)),
        repeating,
        // Colors are all sRGB as CSS wrote them before color spaces, which mix in sRGB.
        space: space.unwrap_or(Space::Srgb),
        stops: color_stops(stops)?,
    })
}

/// Degrees, from `deg`, `rad`, `grad` or `turn`, or a 0.
fn angle(s: &str) -> Option<f64> {
    let n = css::num(s)?;
    Some(match n.unit {
        "deg" => n.value,
        "rad" => n.value.to_degrees(),
        "grad" => n.value * 0.9,
        "turn" => n.value * 360.0,
        "" if n.value == 0.0 => 0.0,
        _ => return None,
    })
}

/// The side or corner after `to`, into `line`; how many words it takes.
fn to(words: &[&str], line: &mut Option<Line>) -> Result<usize, String> {
    // The axis each side is on, and which end of it.
    let side = |w: Option<&&str>| match w.copied() {
        Some("left") => Some((0, -1.0)),
        Some("right") => Some((0, 1.0)),
        Some("top") => Some((1, -1.0)),
        Some("bottom") => Some((1, 1.0)),
        _ => None,
    };
    let Some((axis, end)) = side(words.first()) else {
        return Err("to takes a side or a corner, as to right or to top left".into());
    };
    let mut corner = [0.0; 2];
    corner[axis] = end;
    let used = match side(words.get(1)) {
        Some((other, end)) if other != axis => {
            corner[other] = end;
            *line = Some(Line::Corner(corner));
            2
        }
        Some(_) => return Err(format!("to {} {} is not a corner", words[0], words[1])),
        None => {
            let degrees = match corner {
                [0.0, -1.0] => 0.0,
                [1.0, _] => 90.0,
                [0.0, _] => 180.0,
                _ => 270.0,
            };
            *line = Some(Line::Angle(degrees));
            1
        }
    };
    Ok(used)
}

/// The color space after `in`, into `space`, and for `oklch` the way hues go; how many words it
/// takes.
fn interpolation(words: &[&str], space: &mut Option<Space>) -> Result<usize, String> {
    const SPACES: &str = "srgb, srgb-linear, oklab or oklch";
    let (s, used) = match words {
        ["srgb", ..] => (Space::Srgb, 1),
        ["srgb-linear", ..] => (Space::SrgbLinear, 1),
        ["oklab", ..] => (Space::Oklab, 1),
        ["oklch", way, "hue", ..] => {
            let hue = match *way {
                "shorter" => Hue::Shorter,
                "longer" => Hue::Longer,
                "increasing" => Hue::Increasing,
                "decreasing" => Hue::Decreasing,
                w => return Err(format!("{w} hue is not a way around the hue circle")),
            };
            (Space::Oklch(hue), 3)
        }
        ["oklch", ..] => (Space::Oklch(Hue::Shorter), 1),
        [s, ..] if OTHER_SPACES.contains(s) => {
            return Err(format!("in {s} is not supported yet; use {SPACES}"));
        }
        _ => return Err(format!("in takes a color space: {SPACES}")),
    };
    *space = Some(s);
    Ok(used)
}

/// The color spaces CSS mixes in that the engine does not take yet.
const OTHER_SPACES: &[&str] = &[
    "hsl",
    "hwb",
    "lab",
    "lch",
    "display-p3",
    "a98-rgb",
    "prophoto-rgb",
    "rec2020",
    "xyz",
    "xyz-d50",
    "xyz-d65",
];

/// Color stops, each a color and up to two positions, and hints, each a lone position between
/// two stops.
fn color_stops(parts: &[&str]) -> Result<Vec<Stop<Len, Option<Rgba>>>, String> {
    let mut stops = Vec::new();
    let positioned = |w: &str| css::num(w).is_some() || math::is_math(w);
    for (n, part) in parts.iter().enumerate() {
        match css::words(part)[..] {
            [at] if positioned(at) => {
                let between = n > 0 && n + 1 < parts.len();
                if !between || matches!(stops.last(), Some(Stop::Hint(_))) {
                    return Err(format!(
                        "{at} is a hint, which goes between two color stops"
                    ));
                }
                stops.push(Stop::Hint(length_text(at, POSITION)?));
            }
            [c, ref at @ ..] if at.len() <= 2 && !positioned(c) => {
                let color = current_color(&V::Str(c.into()))?;
                if at.is_empty() {
                    stops.push(Stop::Color(color, None));
                }
                for a in at {
                    stops.push(Stop::Color(color, Some(length_text(a, POSITION)?)));
                }
            }
            _ => {
                return Err(format!(
                    "{part:?} is not a color stop: a color, then up to two positions"
                ));
            }
        }
    }
    if stops.is_empty() {
        return Err("a gradient needs a color stop".into());
    }
    Ok(stops)
}

impl Declared {
    /// The image computed: lengths in px and fractions of the gradient line, and `currentcolor`
    /// the `current` color.
    pub(super) fn computed(&self, u: Units, current: Rgba) -> Image {
        match self {
            Image::None => Image::None,
            Image::Gradient(g) => Image::Gradient(Gradient {
                line: g.line,
                repeating: g.repeating,
                space: g.space,
                stops: (g.stops.iter())
                    .map(|s| match s {
                        Stop::Color(c, at) => {
                            Stop::Color(c.unwrap_or(current), at.as_ref().map(|l| l.calc(u)))
                        }
                        Stop::Hint(at) => Stop::Hint(at.calc(u)),
                    })
                    .collect(),
            }),
        }
    }
}

/// `background-image`: images separated by commas, the first on top.
pub(super) fn images(s: &str) -> Result<Vec<Declared>, String> {
    match s {
        "" => Err("\"\" is not an image; use none".into()),
        _ => css::commas(s).into_iter().map(image).collect(),
    }
}

/// `background`: layers separated by commas, each an image, and the last one a color too.
pub(super) fn background(s: &str) -> Result<(Vec<Declared>, Rgba), String> {
    let layers = css::commas(s);
    if layers.is_empty() {
        return Err("\"\" is not a background; use none".into());
    }
    let mut images = Vec::new();
    let mut color = None;
    for (n, layer) in layers.iter().enumerate() {
        let mut image = None;
        for w in css::words(layer) {
            if let Ok(c) = color::parse(w) {
                if n + 1 < layers.len() || color.is_some() {
                    return Err(format!(
                        "{layer:?}: only the last layer takes a color, and only one"
                    ));
                }
                color = Some(c);
            } else if w != "none" && css::function(w).is_none() {
                return Err(format!("{w} is not a color or an image"));
            } else if image.is_none() {
                image = Some(self::image(w)?);
            } else {
                return Err(format!(
                    "{layer:?} is not a background layer: an image, and last a color, so far"
                ));
            }
        }
        images.push(image.unwrap_or(Image::None));
    }
    Ok((images, color.unwrap_or(color::TRANSPARENT)))
}
