//! Background images: gradients, and resources' images. A gradient is a line through its image,
//! or ellipses or angles around a point in it, the color space its colors mix in, and color stops
//! along the line, out from the point or around it, with hints between them.

use std::rc::Rc;

use super::color::{self, Rgba};
use super::css::{self, angle};
use super::math::{self, Calc};
use super::position::{Offset, position};
use super::{Accept, Len, Units, length_text};
use crate::front::{Holds, Slot};

/// A `background-image` or `mask-image` layer, its stop positions `P` (declared lengths, then px
/// and fractions of the gradient line), colors `C` (declared, `None` is `currentcolor`) and
/// resources `R` (declared by name, then by their number in the scene).
#[derive(Clone, Debug, PartialEq)]
pub enum Image<P = Calc, C = Rgba, R = u32> {
    None,
    Gradient(Gradient<P, C>),
    /// A resource's image.
    Url(R),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Gradient<P = Calc, C = Rgba> {
    pub shape: Shape<P>,
    pub repeating: bool,
    pub space: Space,
    pub stops: Vec<Stop<P, C>>,
}

/// What a gradient's colors go along.
#[derive(Clone, Debug, PartialEq)]
pub enum Shape<P = Calc> {
    Linear(Line),
    /// Circles or ellipses around a point, out to an ending shape `size` big; stops are along
    /// its horizontal radius.
    Radial {
        circle: bool,
        size: Size<P>,
        at: [Offset<P>; 2],
    },
    /// Around a point clockwise, from an angle in degrees clockwise from up; stops are degrees,
    /// as px of a line 360 long.
    Conic {
        from: f64,
        at: [Offset<P>; 2],
    },
}

/// How big a radial gradient's ending shape is: out to the side or the corner of the image
/// nearest its center or farthest from it, or its horizontal and vertical radii, percentages of
/// the image's width and height.
#[derive(Clone, Debug, PartialEq)]
pub enum Size<P = Calc> {
    ClosestSide,
    FarthestSide,
    ClosestCorner,
    FarthestCorner,
    Radii([P; 2]),
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
    Lab,
    Oklab,
    Lch(Hue),
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
pub(super) type Declared = Image<Len, Option<Rgba>, Rc<str>>;

/// Stop positions and hints: px or percentages of the gradient line, either side of it.
const POSITION: Accept = Accept {
    keyword: None,
    negative: true,
    percent: true,
    sizing: &[],
};
/// A circle's radius, and an ellipse's.
const CIRCLE: Accept = Accept {
    keyword: None,
    negative: false,
    percent: false,
    sizing: &[],
};
const ELLIPSE: Accept = Accept {
    keyword: None,
    negative: false,
    percent: true,
    sizing: &[],
};

const SIZES: [(&str, Size<Len>); 4] = [
    ("closest-side", Size::ClosestSide),
    ("farthest-side", Size::FarthestSide),
    ("closest-corner", Size::ClosestCorner),
    ("farthest-corner", Size::FarthestCorner),
];

/// `none`, a gradient, or `url()` of a resource.
pub(super) fn image(s: &str) -> Result<Declared, String> {
    if s == "none" {
        return Ok(Image::None);
    }
    let Some((name, args)) = css::function(s) else {
        return Err(format!("{s} is not an image"));
    };
    let gradient = match name {
        "linear-gradient" => linear(args, false),
        "repeating-linear-gradient" => linear(args, true),
        "radial-gradient" => radial(args, false),
        "repeating-radial-gradient" => radial(args, true),
        "conic-gradient" => conic(args, false),
        "repeating-conic-gradient" => conic(args, true),
        "url" => return url(args).map(Image::Url),
        _ => return Err(format!("{s} is not an image")),
    };
    gradient
        .map(Image::Gradient)
        .map_err(|e| format!("{name}(): {e}"))
}

/// The name of the resource `url()` gives, quoted or not.
fn url(args: &str) -> Result<Rc<str>, String> {
    let s = args.trim();
    let bare = (['"', '\''])
        .iter()
        .find_map(|&q| s.strip_prefix(q)?.strip_suffix(q));
    match bare.unwrap_or(s).strip_prefix("resource://") {
        Some(name) if !name.is_empty() => Ok(name.into()),
        _ => Err(format!(
            "url({s}): an image is a resource declared in controls.resources, such as \
             url(\"resource://logo\")"
        )),
    }
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
            .is_some_and(|w| ["to", "in"].contains(w) || turned(w))
        {
            let mut i = 0;
            while i < words.len() {
                let used = match words[i] {
                    "in" if space.is_none() => interpolation(&words[i + 1..], &mut space)?,
                    "to" if line.is_none() => to(&words[i + 1..], &mut line)?,
                    w if line.is_none() && turned(w) => {
                        line = Some(Line::Angle(math::degrees(w)?));
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
    let shape = Shape::Linear(line.unwrap_or(Line::Angle(180.0)));
    gradient(shape, repeating, space, color_stops(stops, along)?)
}

fn gradient(
    shape: Shape<Len>,
    repeating: bool,
    space: Option<Space>,
    (stops, legacy): (Stops, bool),
) -> Result<Gradient<Len, Option<Rgba>>, String> {
    // Colors all written as sRGB was before color spaces mix in sRGB, as then; others, as CSS
    // Color 4 has it, in Oklab.
    let otherwise = if legacy { Space::Srgb } else { Space::Oklab };
    Ok(Gradient {
        shape,
        repeating,
        space: space.unwrap_or(otherwise),
        stops,
    })
}

/// A stop position along a line: a length or a percentage of it.
fn along(w: &str) -> Result<Len, String> {
    length_text(w, POSITION)
}

/// A conic gradient's stop position: an angle, as degrees, or a percentage of a turn.
fn angular(w: &str) -> Result<Len, String> {
    match css::num(w) {
        Some(n) if n.unit == "%" => Ok(Len::Percent(n.value)),
        _ if turned(w) => Ok(Len::Px(math::degrees(w)?)),
        _ => Err(format!("{w} is not an angle or a percentage")),
    }
}

/// Whether `w` is an angle, or a math function, which may be one.
fn turned(w: &str) -> bool {
    angle(w).is_some() || math::is_math(w)
}

/// The center after `at` from word `i` on, up to a color space, and where it ends; the middle
/// without one.
fn center(words: &[&str], i: usize) -> Result<([Offset<Len>; 2], usize), String> {
    if words.get(i) != Some(&"at") {
        let middle = || Offset {
            end: false,
            by: Len::Percent(50.0),
        };
        return Ok(([middle(), middle()], i));
    }
    let end = (words[i + 1..].iter())
        .position(|w| *w == "in")
        .map_or(words.len(), |n| i + 1 + n);
    Ok((position(&words[i + 1..end], false)?, end))
}

/// A conic gradient's arguments: a color space, then the angle it starts from and its center,
/// or the color space last, and its stops.
fn conic(args: &str, repeating: bool) -> Result<Gradient<Len, Option<Rgba>>, String> {
    let parts = css::commas(args);
    let mut space = None;
    let (mut from, (mut at, _)) = (0.0, center(&[], 0)?);
    let mut stops = &parts[..];
    let words = parts.first().map(|p| css::words(p)).unwrap_or_default();
    if words
        .first()
        .is_some_and(|w| ["from", "at", "in"].contains(w))
    {
        let mut i = 0;
        if words[0] == "in" {
            i += 1 + interpolation(&words[1..], &mut space)?;
        }
        if words.get(i) == Some(&"from") {
            from = match words.get(i + 1) {
                Some(w) if turned(w) => math::degrees(w)?,
                _ => return Err("from takes an angle".into()),
            };
            i += 2;
        }
        (at, i) = center(&words, i)?;
        if words.get(i) == Some(&"in") && space.is_none() {
            i += 1 + interpolation(&words[i + 1..], &mut space)?;
        }
        if let Some(w) = words.get(i) {
            return Err(format!(
                "{w}: a conic gradient starts from an angle and at its center, and a color space"
            ));
        }
        stops = &parts[1..];
    }
    let shape = Shape::Conic { from, at };
    gradient(shape, repeating, space, color_stops(stops, angular)?)
}

/// A radial gradient's arguments: a color space, then its shape, its size and `at` its center,
/// or the color space last, and its stops.
fn radial(args: &str, repeating: bool) -> Result<Gradient<Len, Option<Rgba>>, String> {
    let parts = css::commas(args);
    let mut space = None;
    let (mut circle, mut size, mut radii) = (None, None, Vec::new());
    let (mut at, _) = center(&[], 0)?;
    let sized = |w: &str| {
        SIZES
            .iter()
            .find(|(name, _)| *name == w)
            .map(|(_, s)| s.clone())
    };
    let length = |w: &str| css::num(w).is_some() || math::is_math(w);
    let mut stops = &parts[..];
    let words = parts.first().map(|p| css::words(p)).unwrap_or_default();
    let shaped =
        |w: &&str| ["circle", "ellipse", "at", "in"].contains(w) || sized(w).is_some() || length(w);
    if words.first().is_some_and(shaped) {
        let mut i = 0;
        if words[0] == "in" {
            i += 1 + interpolation(&words[1..], &mut space)?;
        }
        while let Some(&w) = words.get(i) {
            match w {
                "circle" | "ellipse" if circle.is_none() => circle = Some(w == "circle"),
                w if size.is_none() && radii.is_empty() && sized(w).is_some() => size = sized(w),
                w if size.is_none() && length(w) && radii.len() < 2 => {
                    if !radii.is_empty() && !length(words[i - 1]) {
                        break;
                    }
                    radii.push(w);
                }
                _ => break,
            }
            i += 1;
        }
        (at, i) = center(&words, i)?;
        if words.get(i) == Some(&"in") && space.is_none() {
            i += 1 + interpolation(&words[i + 1..], &mut space)?;
        }
        if let Some(w) = words.get(i) {
            return Err(format!(
                "{w}: a radial gradient starts with its shape, its size and at its center, and \
                 a color space"
            ));
        }
        stops = &parts[1..];
    }
    let circle = circle.unwrap_or(radii.len() == 1);
    let size = match (size, &radii[..], circle) {
        (Some(size), ..) => size,
        (None, [], _) => Size::FarthestCorner,
        (None, [r], true) => {
            let r = length_text(r, CIRCLE).map_err(|e| format!("a circle's radius: {e}"))?;
            Size::Radii([r.clone(), r])
        }
        (None, [x, y], false) => Size::Radii([length_text(x, ELLIPSE)?, length_text(y, ELLIPSE)?]),
        (None, _, true) => return Err("a circle takes one radius".into()),
        (None, _, false) => return Err("an ellipse takes two radii".into()),
    };
    let shape = Shape::Radial { circle, size, at };
    gradient(shape, repeating, space, color_stops(stops, along)?)
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

/// The color space after `in`, into `space`, and for `lch` or `oklch` the way hues go; how many
/// words it takes.
fn interpolation(words: &[&str], space: &mut Option<Space>) -> Result<usize, String> {
    const SPACES: &str = "srgb, srgb-linear, lab, oklab, lch or oklch";
    let (s, used) = match words {
        ["srgb", ..] => (Space::Srgb, 1),
        ["srgb-linear", ..] => (Space::SrgbLinear, 1),
        ["lab", ..] => (Space::Lab, 1),
        ["oklab", ..] => (Space::Oklab, 1),
        [polar @ ("lch" | "oklch"), rest @ ..] => {
            let (hue, used) = match rest {
                [way, "hue", ..] => match *way {
                    "shorter" => (Hue::Shorter, 3),
                    "longer" => (Hue::Longer, 3),
                    "increasing" => (Hue::Increasing, 3),
                    "decreasing" => (Hue::Decreasing, 3),
                    w => return Err(format!("{w} hue is not a way around the hue circle")),
                },
                _ => (Hue::Shorter, 1),
            };
            match *polar {
                "lch" => (Space::Lch(hue), used),
                _ => (Space::Oklch(hue), used),
            }
        }
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
    "display-p3",
    "a98-rgb",
    "prophoto-rgb",
    "rec2020",
    "xyz",
    "xyz-d50",
    "xyz-d65",
];

/// A gradient's color stops and hints, as declared; a color `None` is `currentcolor`.
type Stops = Vec<Stop<Len, Option<Rgba>>>;

/// Color stops, each a color and up to two positions, and hints, each a lone position between
/// two stops; and whether the colors are all legacy sRGB ones, `currentcolor` too.
fn color_stops(
    parts: &[&str],
    position: fn(&str) -> Result<Len, String>,
) -> Result<(Stops, bool), String> {
    let (mut stops, mut legacy) = (Vec::new(), true);
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
                stops.push(Stop::Hint(position(at)?));
            }
            [c, ref at @ ..] if at.len() <= 2 && !positioned(c) => {
                let color = match c.to_ascii_lowercase().as_str() {
                    "currentcolor" => None,
                    c => {
                        legacy &= color::legacy(c);
                        Some(color::wide(c)?)
                    }
                };
                if at.is_empty() {
                    stops.push(Stop::Color(color, None));
                }
                for a in at {
                    stops.push(Stop::Color(color, Some(position(a)?)));
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
    Ok((stops, legacy))
}

impl Declared {
    /// The image computed: lengths in px and fractions of the gradient line, `currentcolor` the
    /// `current` color, and a resource its number among the scene's `resources`.
    pub(super) fn computed(
        &self,
        u: Units,
        current: Rgba,
        resources: &[Slot],
    ) -> Result<Image, String> {
        Ok(match self {
            Image::None => Image::None,
            Image::Url(name) => match resources.iter().position(|r| *r.name == **name) {
                Some(i) if resources[i].kind == Holds::Font => {
                    return Err(format!(
                        "url(resource://{name}): {name} is a font, not an image"
                    ));
                }
                Some(i) => Image::Url(i as u32),
                None => {
                    return Err(format!(
                        "url(resource://{name}): the scene declares no resource {name} in \
                         controls.resources"
                    ));
                }
            },
            Image::Gradient(g) => Image::Gradient(Gradient {
                shape: match &g.shape {
                    Shape::Linear(line) => Shape::Linear(*line),
                    Shape::Radial { circle, size, at } => Shape::Radial {
                        circle: *circle,
                        size: match size {
                            Size::ClosestSide => Size::ClosestSide,
                            Size::FarthestSide => Size::FarthestSide,
                            Size::ClosestCorner => Size::ClosestCorner,
                            Size::FarthestCorner => Size::FarthestCorner,
                            Size::Radii(r) => Size::Radii(r.each_ref().map(|l| l.calc(u))),
                        },
                        at: at.each_ref().map(|o| o.computed(u)),
                    },
                    Shape::Conic { from, at } => Shape::Conic {
                        from: *from,
                        at: at.each_ref().map(|o| o.computed(u)),
                    },
                },
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
        })
    }
}
