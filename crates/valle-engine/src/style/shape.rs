//! `clip-path`: the part of a box that shows, a basic shape in one of its boxes, or the box
//! itself with its corners rounded.

use super::background::{Area, area};
use super::css;
use super::declaration::{CENTER, SPREAD, ZERO, radii};
use super::math::Calc;
use super::position::{Offset, position};
use super::units::{Accept, Len, RADIUS, Units, length_text};
use crate::path::{self, Path};
use crate::template::V;

/// A shape in a box of the element, its border box unless it says, or the box alone.
#[derive(Clone, Debug, PartialEq)]
pub struct Clip<P = Calc> {
    pub shape: Option<Shape<P>>,
    pub area: Area,
}

/// A basic shape, its lengths `P` (declared, then px and fractions of what they are of).
#[derive(Clone, Debug, PartialEq)]
pub enum Shape<P = Calc> {
    /// The box less lengths in from its top, right, bottom and left, percentages of its height or
    /// width, with corners rounded as `border-radius` has them, percentages of the box.
    Inset { sides: [P; 4], radii: [[P; 2]; 4] },
    /// Around a point; a radius a percentage of the box's diagonal over √2.
    Circle {
        radius: Radius<P>,
        at: [Offset<P>; 2],
    },
    /// Around a point; radii percentages of the box's width and height.
    Ellipse {
        radii: [Radius<P>; 2],
        at: [Offset<P>; 2],
    },
    /// Points from the box's top left, percentages of its width and height, filled where they
    /// wind around, or with `even_odd` where they wind an odd number of times.
    Polygon { even_odd: bool, points: Vec<[P; 2]> },
    /// A path in px from the box's top left, filled as a polygon is.
    Path { even_odd: bool, path: Path },
}

/// Out to the side of the box nearest the center or farthest from it, or a length.
#[derive(Clone, Debug, PartialEq)]
pub enum Radius<P = Calc> {
    ClosestSide,
    FarthestSide,
    Len(P),
}

impl Clip<Len> {
    pub(super) fn computed(&self, u: Units) -> Clip {
        Clip {
            shape: self.shape.as_ref().map(|s| s.computed(u)),
            area: self.area,
        }
    }
}

impl Shape<Len> {
    pub(super) fn computed(&self, u: Units) -> Shape {
        let calc = |l: &Len| l.calc(u);
        let radius = |r: &Radius<Len>| match r {
            Radius::ClosestSide => Radius::ClosestSide,
            Radius::FarthestSide => Radius::FarthestSide,
            Radius::Len(l) => Radius::Len(calc(l)),
        };
        let at = |at: &[Offset<Len>; 2]| at.each_ref().map(|o| o.computed(u));
        match self {
            Shape::Inset { sides, radii } => Shape::Inset {
                sides: sides.each_ref().map(calc),
                radii: radii.each_ref().map(|r| r.each_ref().map(calc)),
            },
            Shape::Circle { radius: r, at: a } => Shape::Circle {
                radius: radius(r),
                at: at(a),
            },
            Shape::Ellipse { radii, at: a } => Shape::Ellipse {
                radii: radii.each_ref().map(radius),
                at: at(a),
            },
            Shape::Polygon { even_odd, points } => Shape::Polygon {
                even_odd: *even_odd,
                points: points.iter().map(|p| p.each_ref().map(calc)).collect(),
            },
            Shape::Path { even_odd, path } => Shape::Path {
                even_odd: *even_odd,
                path: path.clone(),
            },
        }
    }
}

/// Lengths in from a side, and a polygon's points: either side of it.
const LENGTH: Accept = Accept {
    keyword: None,
    negative: true,
    percent: true,
    sizing: &[],
};

const SHAPES: &str = "inset(), circle(), ellipse(), polygon() or path()";

/// `clip-path`: `none`, or a basic shape, a box (`border-box`, `padding-box` or `content-box`)
/// or both in either order. Path data is case-sensitive, so `s` is as written.
pub(super) fn clip(s: &str) -> Result<Option<Clip<Len>>, String> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("none") {
        return Ok(None);
    } else if s.is_empty() {
        return Err("\"\" is not a clip path; use none".into());
    }
    let (mut shape, mut boxed) = (None, None);
    for w in css::words(s) {
        let lower = w.to_ascii_lowercase();
        if let Some(a) = area(&lower) {
            if boxed.replace(a).is_some() {
                return Err(format!("{s:?} takes one box"));
            }
            continue;
        }
        let Some((name, args)) = css::function(w) else {
            return Err(format!(
                "{w} is not a basic shape, {SHAPES}, or a box, border-box, padding-box or \
                 content-box"
            ));
        };
        let name = name.to_ascii_lowercase();
        let lower = args.to_ascii_lowercase();
        let found = match name.as_str() {
            "inset" => inset(&lower),
            "circle" => round(&lower, false),
            "ellipse" => round(&lower, true),
            "polygon" => polygon(&lower),
            "path" => path_of(args).map(|(even_odd, path)| Shape::Path { even_odd, path }),
            _ => return Err(format!("{name}() is not a basic shape: {SHAPES}")),
        };
        if shape
            .replace(found.map_err(|e| format!("{name}(): {e}"))?)
            .is_some()
        {
            return Err(format!("{s:?} takes one shape"));
        }
    }
    Ok(Some(Clip {
        shape,
        area: boxed.unwrap_or(Area::BorderBox),
    }))
}

/// `inset()`: one to four lengths for the sides as `margin` takes them, then `round` and radii as
/// `border-radius` takes them, if its corners are rounded.
pub(super) fn inset(s: &str) -> Result<Shape<Len>, String> {
    let words = css::words(s);
    let (sides, corners) = match words.iter().position(|w| *w == "round") {
        Some(i) => (&words[..i], Some(words[i + 1..].join(" "))),
        None => (&words[..], None),
    };
    if !(1..=4).contains(&sides.len()) {
        return Err("takes one to four lengths, and round and radii if its corners are".into());
    }
    let lengths = (sides.iter())
        .map(|w| length_text(w, LENGTH))
        .collect::<Result<Vec<_>, _>>()?;
    let radii = match corners {
        None => [const { [ZERO; 2] }; 4],
        Some(c) if c.is_empty() => return Err("round needs radii".into()),
        Some(c) => radii(&V::Str(c.into()))?,
    };
    Ok(Shape::Inset {
        sides: SPREAD[lengths.len() - 1].map(|i| lengths[i].clone()),
        radii,
    })
}

/// `circle()` and `ellipse()`: a radius, or two, then `at` and a position, the box's center if
/// it says none.
pub(super) fn round(s: &str, ellipse: bool) -> Result<Shape<Len>, String> {
    let words = css::words(s);
    let (radii, at) = match words.iter().position(|w| *w == "at") {
        Some(i) => (&words[..i], position(&words[i + 1..], false)?),
        None => (&words[..], [CENTER, CENTER]),
    };
    let radius = |w: &str| {
        Ok::<_, String>(match w {
            "closest-side" => Radius::ClosestSide,
            "farthest-side" => Radius::FarthestSide,
            w => Radius::Len(length_text(w, RADIUS)?),
        })
    };
    let side = Radius::ClosestSide;
    Ok(match (ellipse, radii) {
        (false, []) => Shape::Circle { radius: side, at },
        (false, [r]) => Shape::Circle {
            radius: radius(r)?,
            at,
        },
        (true, []) => Shape::Ellipse {
            radii: [side.clone(), side],
            at,
        },
        (true, [x, y]) => Shape::Ellipse {
            radii: [radius(x)?, radius(y)?],
            at,
        },
        (false, _) => {
            return Err(
                "takes a radius, a length, closest-side or farthest-side, then at and a position"
                    .into(),
            );
        }
        (true, _) => {
            return Err(
                "takes two radii or none, each a length, closest-side or farthest-side, \
                        then at and a position"
                    .into(),
            );
        }
    })
}

/// `polygon()`: a fill rule or not, then points, each two lengths, all after commas.
pub(super) fn polygon(s: &str) -> Result<Shape<Len>, String> {
    let mut parts = css::commas(s);
    let even_odd = rule(&mut parts);
    if parts.is_empty() {
        return Err("takes points, each two lengths after a comma".into());
    }
    let points = (parts.iter())
        .map(|p| match css::words(p)[..] {
            [x, y] => Ok([length_text(x, LENGTH)?, length_text(y, LENGTH)?]),
            _ => Err(format!("{p:?} is not a point: two lengths, x and y")),
        })
        .collect::<Result<_, String>>()?;
    Ok(Shape::Polygon { even_odd, points })
}

/// `path()`: a fill rule or not, then a comma and path data in quotes; whether the rule is
/// `evenodd`, and the path.
pub(super) fn path_of(s: &str) -> Result<(bool, Path), String> {
    let mut parts = css::commas(s);
    let even_odd = rule(&mut parts);
    let data = match parts[..] {
        [d] => ['"', '\'']
            .into_iter()
            .find_map(|q| d.strip_prefix(q)?.strip_suffix(q)),
        _ => None,
    };
    let Some(data) = data else {
        return Err("takes a fill rule or not, then path data in quotes".into());
    };
    Ok((even_odd, path::parse(data)?))
}

/// Whether the fill rule that comes first, if one does, is `evenodd` rather than `nonzero`.
fn rule(parts: &mut Vec<&str>) -> bool {
    let first = parts.first().map(|p| p.to_ascii_lowercase());
    match first.as_deref() {
        Some("nonzero") | Some("evenodd") => parts.remove(0).eq_ignore_ascii_case("evenodd"),
        _ => false,
    }
}
