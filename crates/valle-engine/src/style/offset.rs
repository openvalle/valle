//! Motion paths: `offset-path`, a path, a ray or a basic shape that moves a box's
//! `offset-anchor` to the point `offset-distance` along it, turned as `offset-rotate` says; and
//! `offset-position`, where a ray starts.

use super::background::{Area, area};
use super::css;
use super::math::{self, Calc};
use super::position::{Offset, position};
use super::shape::{self, Shape};
use super::units::{Accept, Len, Units, length_text};
use crate::path::Path;

/// A box's motion path, its lengths `P` (declared, then px and fractions of what they are of).
#[derive(Clone, Debug, PartialEq)]
pub struct Motion<P = Calc> {
    pub path: Route<P>,
    /// The box of its containing block a ray or a shape is in.
    pub area: Area,
    /// How far along it, a percentage of its length: a closed path goes on around, an open one
    /// stops at its ends, and a ray goes on either way.
    pub distance: P,
    /// `offset-rotate`: whether it turns to the path's direction (`auto`), and degrees clockwise
    /// more; `reverse` is `auto` and 180.
    pub along: bool,
    pub angle: f64,
    /// `offset-anchor`: the point in the border box on the path, or with `None` its transform
    /// origin.
    pub anchor: Option<[Offset<P>; 2]>,
    pub position: Start<P>,
}

/// A path to move along.
#[derive(Clone, Debug, PartialEq)]
pub enum Route<P = Calc> {
    /// `path()`, in px from the box's own border box's top left.
    Path(Path),
    /// `ray()`: degrees clockwise from up, how long it is, which its percentages are of, and
    /// where it starts, or with `None` the box's `offset-position`.
    Ray {
        angle: f64,
        size: Size,
        at: Option<[Offset<P>; 2]>,
    },
    /// A basic shape but a path, or without one the box itself with its corners rounded. A
    /// circle or an ellipse with no `at` is around the box's `offset-position` unless that is
    /// `normal`.
    Shape { shape: Option<Shape<P>>, at: bool },
}

/// How long a ray is: to the side of its box nearest its start or the farthest, to the corner
/// nearest or the farthest, or to where it leaves the box (`sides`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Size {
    ClosestSide,
    ClosestCorner,
    FarthestSide,
    FarthestCorner,
    Sides,
}

/// `offset-position`: where a ray starts.
#[derive(Clone, Debug, PartialEq)]
pub enum Start<P = Calc> {
    /// The center of the box the ray is in.
    Normal,
    /// The box's own border box's top left.
    Auto,
    /// A point in the box the ray is in.
    At([Offset<P>; 2]),
}

impl Route<Len> {
    pub(super) fn computed(&self, u: Units) -> Route {
        let at = |at: &[Offset<Len>; 2]| at.each_ref().map(|o| o.computed(u));
        match self {
            Route::Path(p) => Route::Path(p.clone()),
            Route::Ray { angle, size, at: a } => Route::Ray {
                angle: *angle,
                size: *size,
                at: a.as_ref().map(at),
            },
            Route::Shape { shape, at } => Route::Shape {
                shape: shape.as_ref().map(|s| s.computed(u)),
                at: *at,
            },
        }
    }
}

impl Start<Len> {
    pub(super) fn computed(&self, u: Units) -> Start {
        match self {
            Start::Normal => Start::Normal,
            Start::Auto => Start::Auto,
            Start::At(at) => Start::At(at.each_ref().map(|o| o.computed(u))),
        }
    }
}

/// `offset-distance`: any length.
const DISTANCE: Accept = Accept {
    keyword: None,
    negative: true,
    percent: true,
    sizing: &[],
};

const ROUTES: &str = "path(), ray(), inset(), circle(), ellipse() or polygon()";

/// `offset-path`: `none`; `path()`; or `ray()`, a basic shape, a box (`border-box`,
/// `padding-box` or `content-box`) or a ray or shape and a box in either order. Path data is
/// case-sensitive, so `s` is as written.
pub(super) fn path(s: &str) -> Result<Option<(Route<Len>, Area)>, String> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("none") {
        return Ok(None);
    } else if s.is_empty() {
        return Err("\"\" is not an offset path; use none".into());
    }
    let (mut route, mut boxed) = (None, None);
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
                "{w} is not an offset path, {ROUTES}, or a box, border-box, padding-box or \
                 content-box"
            ));
        };
        let name = name.to_ascii_lowercase();
        let lower = args.to_ascii_lowercase();
        let found = match name.as_str() {
            "path" => shape::path_of(args).map(|(_, path)| Route::Path(path)),
            "ray" => ray(&lower),
            "inset" => shape::inset(&lower).map(|s| Route::Shape {
                shape: Some(s),
                at: true,
            }),
            "circle" | "ellipse" => shape::round(&lower, name == "ellipse").map(|s| Route::Shape {
                shape: Some(s),
                at: css::words(&lower).contains(&"at"),
            }),
            "polygon" => shape::polygon(&lower).map(|s| Route::Shape {
                shape: Some(s),
                at: true,
            }),
            _ => return Err(format!("{name}() is not an offset path: {ROUTES}")),
        };
        if route
            .replace(found.map_err(|e| format!("{name}(): {e}"))?)
            .is_some()
        {
            return Err(format!("{s:?} takes one path"));
        }
    }
    let route = route.unwrap_or(Route::Shape {
        shape: None,
        at: true,
    });
    if matches!(route, Route::Path(_)) && boxed.is_some() {
        return Err(format!(
            "{s:?}: path() is in the box's own px and takes no box"
        ));
    }
    Ok(Some((route, boxed.unwrap_or(Area::BorderBox))))
}

/// `ray()`: an angle, a size or not, then `at` and a position or not.
fn ray(s: &str) -> Result<Route<Len>, String> {
    let words = css::words(s);
    let (words, at) = match words.iter().position(|w| *w == "at") {
        Some(i) => (&words[..i], Some(position(&words[i + 1..], false)?)),
        None => (&words[..], None),
    };
    let (mut angle, mut size) = (None, None);
    for &w in words {
        let found = match w {
            "closest-side" => size.replace(Size::ClosestSide).is_some(),
            "closest-corner" => size.replace(Size::ClosestCorner).is_some(),
            "farthest-side" => size.replace(Size::FarthestSide).is_some(),
            "farthest-corner" => size.replace(Size::FarthestCorner).is_some(),
            "sides" => size.replace(Size::Sides).is_some(),
            "contain" => return Err("contain is not supported".into()),
            w if css::angle(w).is_some() || math::is_math(w) => {
                angle.replace(math::degrees(w)?).is_some()
            }
            w => {
                return Err(format!(
                    "{w} is not an angle or a size, closest-side, closest-corner, \
                     farthest-side, farthest-corner or sides"
                ));
            }
        };
        if found {
            return Err("takes one angle and one size".into());
        }
    }
    let Some(angle) = angle else {
        return Err("takes an angle".into());
    };
    Ok(Route::Ray {
        angle,
        size: size.unwrap_or(Size::ClosestSide),
        at,
    })
}

/// `offset-rotate`: `auto` or `reverse`, an angle, or both in either order.
pub(super) fn rotate(s: &str) -> Result<(bool, f64), String> {
    let bad = || format!("{s:?} is not an offset rotate: auto or reverse, an angle, or both");
    let (mut along, mut angle) = (None, None);
    let words = css::words(s);
    for w in &words {
        let twice = match *w {
            "auto" => along.replace(0.0).is_some(),
            "reverse" => along.replace(180.0).is_some(),
            w => angle
                .replace(math::degrees(w).map_err(|_| bad())?)
                .is_some(),
        };
        if twice {
            return Err(bad());
        }
    }
    if words.is_empty() {
        return Err(bad());
    }
    Ok((along.is_some(), along.unwrap_or(0.0) + angle.unwrap_or(0.0)))
}

/// `offset-anchor`: `auto`, or a position of one, two or four words.
pub(super) fn anchor(s: &str) -> Result<Option<[Offset<Len>; 2]>, String> {
    match s {
        "auto" => Ok(None),
        _ => Ok(Some(at(s, "an offset anchor: auto")?)),
    }
}

/// `offset-position`: `normal`, `auto`, or a position of one, two or four words.
pub(super) fn start(s: &str) -> Result<Start<Len>, String> {
    match s {
        "normal" => Ok(Start::Normal),
        "auto" => Ok(Start::Auto),
        _ => Ok(Start::At(at(s, "an offset position: normal, auto")?)),
    }
}

fn at(s: &str, what: &str) -> Result<[Offset<Len>; 2], String> {
    position(&css::words(s), false).map_err(|_| {
        format!(
            "{s:?} is not {what} or one, two or four of left, center, right, top, bottom and \
             lengths"
        )
    })
}

/// `offset-distance`.
pub(super) fn distance(s: &str) -> Result<Len, String> {
    length_text(s, DISTANCE)
}
