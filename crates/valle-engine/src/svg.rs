//! Inline SVG: what an element's attributes say it draws, in its user units, where an `<svg>`'s
//! viewBox puts them in its box, and the outlines of strokes.

use std::rc::Rc;

use kurbo::{BezPath, ParamCurveArclen, PathEl, Shape as _, StrokeOpts};

use crate::paint::{self, Geometry, Matrix, Ramp, Spread};
use crate::path::{self, Path, Point, Segment};
use crate::style::color::Rgba;
use crate::style::gradient::Space;
use crate::style::{Cap, Join};
use crate::template::{Tag, V};

/// An SVG length: user units, and a percentage of the viewport's width, height or normalized
/// diagonal, as the attribute measures.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Length {
    pub units: f64,
    pub percent: f64,
}

impl Length {
    pub fn resolve(self, basis: f64) -> f64 {
        self.units + self.percent / 100.0 * basis
    }
}

/// How a viewBox fits its viewport (`preserveAspectRatio`): stretched to it (`align` `None`), or
/// scaled evenly to fit in it or, with `slice`, to cover it, aligned on each axis at 0 (min), 0.5
/// (mid) or 1 (max).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fit {
    pub align: Option<[f64; 2]>,
    pub slice: bool,
}

/// A basic shape, or a path.
#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    Path(Path),
    Circle {
        center: [Length; 2],
        r: Length,
    },
    /// Its radii; `None` for `auto`, the other one.
    Ellipse {
        center: [Length; 2],
        r: [Option<Length>; 2],
    },
    /// Its corner radii; `None` for `auto`, the other one, or 0 if both are.
    Rect {
        at: [Length; 2],
        size: [Length; 2],
        r: [Option<Length>; 2],
    },
    Line([Length; 4]),
    /// A polyline, or with `closed` a polygon.
    Points {
        points: Vec<Point>,
        closed: bool,
    },
}

/// What an element draws: an `<svg>` its viewBox through its fit, a group its children, a shape
/// itself, a `<use>` a copy. Definitions, gradients and their stops, clip paths, masks and symbols
/// draw nothing themselves.
#[derive(Clone, Debug, PartialEq)]
pub enum Element {
    Svg {
        view_box: Option<[f64; 4]>,
        fit: Fit,
    },
    G,
    Shape(Shape),
    Defs,
    Gradient(Gradient),
    /// A gradient's stop, at an offset from 0 to 1.
    Stop(f64),
    /// A clip path: the shapes in it, in user units of the element it clips, or without
    /// `user_space` in fractions of that element's bounding box.
    ClipPath {
        user_space: bool,
    },
    Mask(Mask),
    /// A copy of the element `href` names, moved by `at`; a `<symbol>` in a viewport `size` big,
    /// as big as the one it is in on an axis without a size.
    Use {
        href: Rc<str>,
        at: [Length; 2],
        size: [Option<Length>; 2],
    },
    /// What a `<use>` shows of it: its children, through its viewBox by its fit in the viewport
    /// the `<use>` gives it, and clipped to it.
    Symbol {
        view_box: Option<[f64; 4]>,
        fit: Fit,
    },
}

/// A mask: what its children draw, by their luminance or alpha, inside `region` (x, y, width and
/// height), which is in user units of the element it masks or without `user_space` in fractions
/// of that element's bounding box; its children are in those user units, or without
/// `content_user_space` in those fractions.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mask {
    pub user_space: bool,
    pub content_user_space: bool,
    pub region: [Length; 4],
}

/// A gradient as its own attributes have it; those it leaves out come from the gradient its
/// `href` names, then their defaults.
#[derive(Clone, Debug, PartialEq)]
pub struct Gradient {
    pub radial: bool,
    pub href: Option<Rc<str>>,
    /// `x1`, `y1`, `x2` and `y2`, or `cx`, `cy`, `r`, `fx`, `fy` and `fr`.
    pub lengths: [Option<Length>; 6],
    /// `gradientUnits`: user units, not fractions of the shape's bounding box.
    pub user_space: Option<bool>,
    pub transform: Option<Matrix>,
    pub spread: Option<Spread>,
}

/// An element as its attributes have it at a frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Computed {
    pub element: Element,
    /// Its `transform` attribute: from its user units to its parent's.
    pub transform: Option<Matrix>,
    /// A shape's `pathLength`: the length its dashes take its path to have.
    pub path_length: Option<f64>,
    pub id: Option<Rc<str>>,
}

/// Whether an attribute is a presentation attribute: a style property of the same name, which
/// the element's style overrides. An `<svg>`'s `width` and `height` are its box's.
pub fn presentational(tag: Tag, name: &str) -> bool {
    matches!(
        name,
        "fill"
            | "fillOpacity"
            | "fillRule"
            | "stroke"
            | "strokeOpacity"
            | "strokeWidth"
            | "strokeLinecap"
            | "strokeLinejoin"
            | "strokeMiterlimit"
            | "strokeDasharray"
            | "strokeDashoffset"
            | "stopColor"
            | "stopOpacity"
            | "clipPath"
            | "clipRule"
            | "mask"
            | "maskType"
            | "opacity"
            | "visibility"
            | "color"
    ) || (tag == Tag::Svg && matches!(name, "width" | "height"))
}

/// The element `tag` as these attribute values have it; presentation attributes are left to the
/// style.
pub fn computed(tag: Tag, attributes: &[(&str, &V)]) -> Result<Computed, String> {
    let get = |name: &str| (attributes.iter()).find_map(|(n, v)| (*n == name).then_some(*v));
    let length = |name: &str| -> Result<Length, String> {
        get(name).map_or(Ok(Length::default()), |v| self::length(v).map_err(at(name)))
    };
    let auto = |name: &str| -> Result<Option<Length>, String> {
        match get(name) {
            None => Ok(None),
            Some(V::Str(s)) if s.trim() == "auto" => Ok(None),
            Some(v) => self::length(v).map(Some).map_err(at(name)),
        }
    };
    let text = |name: &str| get(name).map(|v| v.text().into_owned());
    let href = || match text("href").or_else(|| text("xlinkHref")) {
        Some(h) => match h.trim().strip_prefix('#') {
            Some(id) if !id.is_empty() => Ok(Some(Rc::from(id))),
            _ => Err(format!("href: {h} is not #id, an element in the <svg>")),
        },
        None => Ok(None),
    };
    let fitting = || -> Result<(Option<[f64; 4]>, Fit), String> {
        let view_box = text("viewBox").map(|v| view_box(&v).map_err(at("viewBox")));
        let fit = match text("preserveAspectRatio") {
            Some(f) => fit(&f).map_err(at("preserveAspectRatio"))?,
            None => MEET,
        };
        Ok((view_box.transpose()?, fit))
    };
    let transform =
        (text("transform").map(|t| transform(&t).map_err(at("transform")))).transpose()?;
    let element = match tag {
        Tag::Svg => {
            let (view_box, fit) = fitting()?;
            Element::Svg { view_box, fit }
        }
        Tag::G => Element::G,
        Tag::Path => Element::Shape(Shape::Path(match text("d") {
            Some(d) if !d.trim().is_empty() => path::parse(&d).map_err(at("d"))?,
            _ => Path::default(),
        })),
        Tag::Circle => Element::Shape(Shape::Circle {
            center: [length("cx")?, length("cy")?],
            r: length("r")?,
        }),
        Tag::Ellipse => Element::Shape(Shape::Ellipse {
            center: [length("cx")?, length("cy")?],
            r: [auto("rx")?, auto("ry")?],
        }),
        Tag::Rect => Element::Shape(Shape::Rect {
            at: [length("x")?, length("y")?],
            size: [length("width")?, length("height")?],
            r: [auto("rx")?, auto("ry")?],
        }),
        Tag::Line => Element::Shape(Shape::Line([
            length("x1")?,
            length("y1")?,
            length("x2")?,
            length("y2")?,
        ])),
        Tag::Polyline | Tag::Polygon => Element::Shape(Shape::Points {
            points: match text("points") {
                Some(p) => points(&p).map_err(at("points"))?,
                None => Vec::new(),
            },
            closed: tag == Tag::Polygon,
        }),
        Tag::Defs => Element::Defs,
        Tag::LinearGradient | Tag::RadialGradient => {
            let radial = tag == Tag::RadialGradient;
            let names = match radial {
                true => ["cx", "cy", "r", "fx", "fy", "fr"],
                false => ["x1", "y1", "x2", "y2", "", ""],
            };
            let mut lengths = [None; 6];
            for (l, name) in lengths.iter_mut().zip(names) {
                *l = get(name)
                    .map(|v| self::length(v).map_err(at(name)))
                    .transpose()?;
            }
            let href = href()?;
            let user_space = text("gradientUnits").map(|u| units(&u).map_err(at("gradientUnits")));
            let spread = text("spreadMethod").map(|m| match m.trim() {
                "pad" => Ok(Spread::Pad),
                "repeat" => Ok(Spread::Repeat),
                "reflect" => Ok(Spread::Reflect),
                m => Err(format!("spreadMethod: {m} is not pad, reflect or repeat")),
            });
            let matrix = text("gradientTransform")
                .map(|t| self::transform(&t).map_err(at("gradientTransform")));
            Element::Gradient(Gradient {
                radial,
                href,
                lengths,
                user_space: user_space.transpose()?,
                transform: matrix.transpose()?,
                spread: spread.transpose()?,
            })
        }
        Tag::Stop => Element::Stop(match get("offset") {
            Some(v) => offset(v).map_err(at("offset"))?,
            None => 0.0,
        }),
        Tag::ClipPath => Element::ClipPath {
            user_space: text("clipPathUnits")
                .map_or(Ok(true), |u| units(&u).map_err(at("clipPathUnits")))?,
        },
        Tag::Mask => {
            let percent = |percent| Length {
                units: 0.0,
                percent,
            };
            let region = [
                ("x", -10.0),
                ("y", -10.0),
                ("width", 120.0),
                ("height", 120.0),
            ];
            let mut lengths = [Length::default(); 4];
            for (l, (name, or)) in lengths.iter_mut().zip(region) {
                *l = match get(name) {
                    Some(v) => self::length(v).map_err(at(name))?,
                    None => percent(or),
                };
            }
            Element::Mask(Mask {
                user_space: text("maskUnits")
                    .map_or(Ok(false), |u| units(&u).map_err(at("maskUnits")))?,
                content_user_space: text("maskContentUnits")
                    .map_or(Ok(true), |u| units(&u).map_err(at("maskContentUnits")))?,
                region: lengths,
            })
        }
        Tag::Use => Element::Use {
            href: href()?.ok_or("a <use> copies the element its href names: write href=\"#id\"")?,
            at: [length("x")?, length("y")?],
            size: [auto("width")?, auto("height")?],
        },
        Tag::Symbol => {
            let (view_box, fit) = fitting()?;
            Element::Symbol { view_box, fit }
        }
    };
    if transform.is_some() && matches!(element, Element::Defs | Element::Stop(_)) {
        return Err(format!(
            "transform: a <{}> draws nothing to transform",
            tag.name()
        ));
    }
    if transform.is_some() && matches!(element, Element::Gradient(_)) {
        return Err("transform: a gradient takes gradientTransform".into());
    }
    if transform.is_some() && matches!(element, Element::Mask(_) | Element::Symbol { .. }) {
        return Err(format!(
            "transform: a <{}> takes none, but what is in it may",
            tag.name()
        ));
    }
    let negative = |name: &str, l: Option<Length>| match l {
        Some(l) if l.units < 0.0 || l.percent < 0.0 => Err(format!("{name} is negative")),
        _ => Ok(()),
    };
    match &element {
        Element::Shape(Shape::Circle { r, .. }) => negative("r", Some(*r))?,
        Element::Shape(Shape::Ellipse { r, .. }) => {
            negative("rx", r[0])?;
            negative("ry", r[1])?;
        }
        Element::Shape(Shape::Rect { size, r, .. }) => {
            negative("width", Some(size[0]))?;
            negative("height", Some(size[1]))?;
            negative("rx", r[0])?;
            negative("ry", r[1])?;
        }
        Element::Gradient(g) if g.radial => {
            negative("r", g.lengths[2])?;
            negative("fr", g.lengths[5])?;
        }
        Element::Mask(m) => {
            negative("width", Some(m.region[2]))?;
            negative("height", Some(m.region[3]))?;
        }
        Element::Use { size, .. } => {
            negative("width", size[0])?;
            negative("height", size[1])?;
        }
        _ => {}
    }
    let path_length = match get("pathLength") {
        None => None,
        Some(v) => match v.text().trim().parse::<f64>() {
            Ok(l) if l > 0.0 && l.is_finite() => Some(l),
            _ => return Err(format!("pathLength {} is not a number above 0", v.text())),
        },
    };
    Ok(Computed {
        element,
        transform,
        path_length,
        id: text("id").map(Rc::from),
    })
}

/// A stop's offset: a number or a percentage, clamped to [0, 1].
fn offset(v: &V) -> Result<f64, String> {
    let text = v.text();
    let s = text.trim();
    let x = match s.strip_suffix('%') {
        Some(p) => p.trim().parse::<f64>().map(|p| p / 100.0),
        None => s.parse::<f64>(),
    };
    match x {
        Ok(x) if x.is_finite() => Ok(x.clamp(0.0, 1.0)),
        _ => Err(format!("{s} is not a number or a percentage")),
    }
}

/// Units of a gradient, a clip path or a mask: user units (true), or fractions of a bounding box.
fn units(s: &str) -> Result<bool, String> {
    match s.trim() {
        "userSpaceOnUse" => Ok(true),
        "objectBoundingBox" => Ok(false),
        u => Err(format!("{u} is not userSpaceOnUse or objectBoundingBox")),
    }
}

/// An error in attribute `name`, which says so.
fn at(name: &str) -> impl FnOnce(String) -> String + '_ {
    move |e| format!("{name}: {e}")
}

/// `xMidYMid meet`, the default fit.
const MEET: Fit = Fit {
    align: Some([0.5, 0.5]),
    slice: false,
};

/// A number of user units, or a string of one, in px, or a percentage.
fn length(v: &V) -> Result<Length, String> {
    let s = match v {
        V::Num(x) if x.is_finite() => {
            return Ok(Length {
                units: *x,
                percent: 0.0,
            });
        }
        V::Str(s) => s.trim(),
        _ => "",
    };
    let (number, percent) = match (s.strip_suffix('%'), s.strip_suffix("px")) {
        (Some(n), _) => (n, true),
        (None, Some(n)) => (n, false),
        (None, None) => (s, false),
    };
    match path::numbers(number).as_deref() {
        Some(&[n]) if percent => Ok(Length {
            units: 0.0,
            percent: f64::from(n),
        }),
        Some(&[n]) => Ok(Length {
            units: f64::from(n),
            percent: 0.0,
        }),
        _ => Err(format!(
            "{} is not a length: a number, px or a percentage",
            v.text()
        )),
    }
}

/// `min-x min-y width height`.
fn view_box(s: &str) -> Result<[f64; 4], String> {
    match path::numbers(s).as_deref() {
        Some(&[x, y, w, h]) if w >= 0.0 && h >= 0.0 => Ok([x, y, w, h].map(f64::from)),
        Some(&[_, _, _, _]) => Err(format!("{s:?} has a negative width or height")),
        _ => Err(format!("{s:?} is not 4 numbers: min-x min-y width height")),
    }
}

/// `none`, or an alignment such as `xMidYMid` and `meet` or `slice`.
fn fit(s: &str) -> Result<Fit, String> {
    let words: Vec<&str> = s.split_whitespace().collect();
    let (align, slice) = match words[..] {
        [a] => (a, false),
        [a, "meet"] => (a, false),
        [a, "slice"] => (a, true),
        _ => return Err(format!("{s:?} is not an alignment and meet or slice")),
    };
    if align == "none" {
        return Ok(Fit { align: None, slice });
    }
    let at = |s: &str| match s {
        "Min" => Some(0.0),
        "Mid" => Some(0.5),
        "Max" => Some(1.0),
        _ => None,
    };
    let parts = (align.strip_prefix('x'))
        .and_then(|r| Some((at(r.get(..3)?)?, r.get(3..)?.strip_prefix('Y')?)))
        .and_then(|(x, r)| Some([x, at(r)?]));
    match parts {
        Some(align) => Ok(Fit {
            align: Some(align),
            slice,
        }),
        None => Err(format!(
            "{align:?} is not an alignment: none, or x and Min, Mid or Max, then Y and one"
        )),
    }
}

/// Pairs of numbers.
fn points(s: &str) -> Result<Vec<Point>, String> {
    match path::numbers(s) {
        Some(n) if n.len() % 2 == 0 => Ok(n.chunks(2).map(|p| [p[0], p[1]]).collect()),
        _ => Err(format!("{s:?} is not pairs of numbers")),
    }
}

/// An SVG transform list: `matrix()`, `translate()`, `scale()`, `rotate()`, `skewX()` and
/// `skewY()` of numbers, applied right to left.
fn transform(s: &str) -> Result<Matrix, String> {
    let bad = |what: &str| format!("{s:?} is not a transform list: {what}");
    let mut m = Matrix::IDENTITY;
    let mut rest = s.trim_start();
    while !rest.is_empty() {
        let (name, after) = rest
            .split_once('(')
            .ok_or_else(|| bad("a function lacks ("))?;
        let (args, after) = after
            .split_once(')')
            .ok_or_else(|| bad("a function lacks )"))?;
        let name = name.trim();
        let n = path::numbers(args)
            .ok_or_else(|| bad(&format!("{name}() takes numbers without units")))?
            .into_iter()
            .map(f64::from)
            .collect::<Vec<_>>();
        let tan = |a: f64| a.to_radians().tan();
        let rotate = |a: f64| {
            let (sin, cos) = a.to_radians().sin_cos();
            Matrix::affine([cos, sin, -sin, cos, 0.0, 0.0])
        };
        let f = match (name, &n[..]) {
            ("matrix", &[a, b, c, d, e, f]) => Matrix::affine([a, b, c, d, e, f]),
            ("translate", &[x]) => Matrix::translate(x, 0.0),
            ("translate", &[x, y]) => Matrix::translate(x, y),
            ("scale", &[k]) => Matrix::affine([k, 0.0, 0.0, k, 0.0, 0.0]),
            ("scale", &[x, y]) => Matrix::affine([x, 0.0, 0.0, y, 0.0, 0.0]),
            ("rotate", &[a]) => rotate(a),
            ("rotate", &[a, x, y]) => Matrix::translate(x, y)
                .times(rotate(a))
                .times(Matrix::translate(-x, -y)),
            ("skewX", &[a]) => Matrix::affine([1.0, 0.0, tan(a), 1.0, 0.0, 0.0]),
            ("skewY", &[a]) => Matrix::affine([1.0, tan(a), 0.0, 1.0, 0.0, 0.0]),
            _ => {
                return Err(bad(&format!(
                    "{name}() with {} numbers is not matrix(6), translate(1 or 2), scale(1 or \
                     2), rotate(1 or 3), skewX(1) or skewY(1)",
                    n.len()
                )));
            }
        };
        m = m.times(f);
        rest = after.trim_start_matches(|c: char| c.is_whitespace() || c == ',');
    }
    Ok(m)
}

/// The transform from a viewBox's units to its viewport's px, a viewport `size` px big at the
/// origin.
pub fn viewport([x, y, w, h]: [f64; 4], fit: Fit, size: [f64; 2]) -> Matrix {
    let mut k = [size[0] / w, size[1] / h];
    let mut at = [-x * k[0], -y * k[1]];
    if let Some(align) = fit.align {
        let even = if fit.slice {
            k[0].max(k[1])
        } else {
            k[0].min(k[1])
        };
        k = [even; 2];
        at = [0, 1].map(|a| -[x, y][a] * even + align[a] * (size[a] - [w, h][a] * even));
    }
    Matrix::affine([k[0], 0.0, 0.0, k[1], at[0], at[1]])
}

/// The path a shape fills, in a viewport `size` user units big; `None` for one that draws
/// nothing.
pub fn path(shape: &Shape, size: [f64; 2]) -> Option<Path> {
    let [w, h] = size;
    let diagonal = ((w * w + h * h) / 2.0).sqrt();
    let point = |x: Length, y: Length| [x.resolve(w) as f32, y.resolve(h) as f32];
    let path = match shape {
        Shape::Path(p) => p.clone(),
        Shape::Circle { center, r } => {
            let r = r.resolve(diagonal) as f32;
            ellipse(point(center[0], center[1]), [r, r])?
        }
        Shape::Ellipse { center, r } => {
            let [rx, ry] = [r[0].map(|l| l.resolve(w)), r[1].map(|l| l.resolve(h))];
            let [rx, ry] = [rx.or(ry)?, ry.or(rx)?].map(|r| r as f32);
            ellipse(point(center[0], center[1]), [rx, ry])?
        }
        Shape::Rect { at, size, r } => {
            let [x, y] = point(at[0], at[1]);
            let [width, height] = [size[0].resolve(w), size[1].resolve(h)].map(|v| v as f32);
            if width <= 0.0 || height <= 0.0 {
                return None;
            }
            let [rx, ry] = [r[0].map(|l| l.resolve(w)), r[1].map(|l| l.resolve(h))];
            let [rx, ry] = [rx.or(ry).unwrap_or(0.0), ry.or(rx).unwrap_or(0.0)];
            rect([x, y, width, height], [rx as f32, ry as f32])
        }
        Shape::Line([x1, y1, x2, y2]) => Path(vec![
            Segment::Move(point(*x1, *y1)),
            Segment::Line(point(*x2, *y2)),
        ]),
        Shape::Points { points, closed } => {
            let (first, rest) = points.split_first()?;
            let mut segments = vec![Segment::Move(*first)];
            segments.extend(rest.iter().map(|&p| Segment::Line(p)));
            if *closed {
                segments.push(Segment::Close);
            }
            Path(segments)
        }
    };
    (!path.0.is_empty()).then_some(path)
}

/// How far a cubic curve's control points are from its ends, in radii, for a quarter of a circle.
const KAPPA: f32 = 0.552_284_8;

/// An ellipse around `c` as four cubic curves, from its rightmost point clockwise.
fn ellipse(c: Point, [rx, ry]: [f32; 2]) -> Option<Path> {
    if rx <= 0.0 || ry <= 0.0 {
        return None;
    }
    let [kx, ky] = [rx * KAPPA, ry * KAPPA];
    let [x, y] = c;
    Some(Path(vec![
        Segment::Move([x + rx, y]),
        Segment::Cubic([x + rx, y + ky], [x + kx, y + ry], [x, y + ry]),
        Segment::Cubic([x - kx, y + ry], [x - rx, y + ky], [x - rx, y]),
        Segment::Cubic([x - rx, y - ky], [x - kx, y - ry], [x, y - ry]),
        Segment::Cubic([x + kx, y - ry], [x + rx, y - ky], [x + rx, y]),
        Segment::Close,
    ]))
}

/// A rectangle, its corners rounded by radii no more than half its sides, clockwise from the
/// top left corner's end.
fn rect([x, y, w, h]: [f32; 4], [rx, ry]: [f32; 2]) -> Path {
    let [rx, ry] = [rx.min(w / 2.0), ry.min(h / 2.0)];
    if rx <= 0.0 || ry <= 0.0 {
        return Path(vec![
            Segment::Move([x, y]),
            Segment::Line([x + w, y]),
            Segment::Line([x + w, y + h]),
            Segment::Line([x, y + h]),
            Segment::Close,
        ]);
    }
    let [kx, ky] = [rx * (1.0 - KAPPA), ry * (1.0 - KAPPA)];
    let [r, b] = [x + w, y + h];
    Path(vec![
        Segment::Move([x + rx, y]),
        Segment::Line([r - rx, y]),
        Segment::Cubic([r - kx, y], [r, y + ky], [r, y + ry]),
        Segment::Line([r, b - ry]),
        Segment::Cubic([r, b - ky], [r - kx, b], [r - rx, b]),
        Segment::Line([x + rx, b]),
        Segment::Cubic([x + kx, b], [x, b - ky], [x, b - ry]),
        Segment::Line([x, y + ry]),
        Segment::Cubic([x, y + ky], [x + kx, y], [x + rx, y]),
        Segment::Close,
    ])
}

/// A stroke as it is drawn, in user units.
#[derive(Clone, Debug, PartialEq)]
pub struct Pen {
    pub width: f64,
    pub cap: Cap,
    pub join: Join,
    pub miter_limit: f64,
    /// Dashes and gaps in turn, an odd number of them repeated; none, or all 0, for a solid
    /// stroke.
    pub dashes: Vec<f64>,
    pub dash_offset: f64,
    /// The length the dashes take the path to have, if not its own.
    pub path_length: Option<f64>,
}

/// How far a stroke's outline may be from where it should, px.
const ACCURACY: f64 = 0.01;

/// The length a dot is drawn as, px: a subpath of no length, which shows as its caps if they
/// are round or square.
const DOT: f64 = 1e-3;

/// The outline of `path` stroked by `pen`, which fills it nonzero; `scale` is px a user unit,
/// for its accuracy. Each subpath starts the dashes again. A subpath or a dash of no length
/// shows as its caps, a square one along the x axis, as Chrome draws them. `None` for a stroke
/// that draws nothing.
pub fn stroke(path: &Path, pen: &Pen, scale: f64) -> Option<Path> {
    let s = scale.max(1e-9);
    let mut els = bez(path, s);
    // In px: user units, or those of the path length.
    let k = match pen.path_length {
        Some(l) if !pen.dashes.is_empty() => arclen(&els) / l,
        _ => s,
    };
    let mut dashes: Vec<f64> = pen.dashes.iter().map(|d| d * k).collect();
    if dashes.len() % 2 == 1 {
        dashes.extend_from_within(..);
    }
    if dashes.iter().sum::<f64>() > 0.0 {
        els = kurbo::dash(els.iter(), pen.dash_offset * k, &dashes).collect();
    }
    let els = dotted(&els, pen.cap != Cap::Butt);
    let cap = match pen.cap {
        Cap::Butt => kurbo::Cap::Butt,
        Cap::Round => kurbo::Cap::Round,
        Cap::Square => kurbo::Cap::Square,
    };
    let join = match pen.join {
        Join::Miter => kurbo::Join::Miter,
        Join::Round => kurbo::Join::Round,
        Join::Bevel => kurbo::Join::Bevel,
    };
    let style = kurbo::Stroke::new(pen.width * s)
        .with_caps(cap)
        .with_join(join)
        .with_miter_limit(pen.miter_limit);
    let outline = kurbo::stroke(els, &style, &StrokeOpts::default(), ACCURACY);
    let back = |p: kurbo::Point| [(p.x / s) as f32, (p.y / s) as f32];
    let segments: Vec<Segment> = (outline.elements().iter())
        .map(|e| match *e {
            PathEl::MoveTo(p) => Segment::Move(back(p)),
            PathEl::LineTo(p) => Segment::Line(back(p)),
            PathEl::QuadTo(c, p) => Segment::Quad(back(c), back(p)),
            PathEl::CurveTo(c1, c2, p) => Segment::Cubic(back(c1), back(c2), back(p)),
            PathEl::ClosePath => Segment::Close,
        })
        .collect();
    (!segments.is_empty()).then_some(Path(segments))
}

/// `path` with each subpath of no length a line `DOT` long along the x axis if `dots`, else
/// left out.
fn dotted(path: &BezPath, dots: bool) -> BezPath {
    let mut out = BezPath::new();
    let flush = |out: &mut BezPath, start: kurbo::Point, subpath: &mut Vec<PathEl>| {
        let away = |e: &PathEl| match *e {
            PathEl::LineTo(p) => p != start,
            PathEl::QuadTo(c, p) => c != start || p != start,
            PathEl::CurveTo(c1, c2, p) => c1 != start || c2 != start || p != start,
            PathEl::MoveTo(_) | PathEl::ClosePath => false,
        };
        if subpath.iter().any(away) {
            out.move_to(start);
            subpath.iter().for_each(|&e| out.push(e));
        } else if !subpath.is_empty() && dots {
            out.move_to(start);
            out.line_to(start + kurbo::Vec2::new(DOT, 0.0));
        }
        subpath.clear();
    };
    let (mut start, mut subpath) = (kurbo::Point::ORIGIN, Vec::new());
    for el in path.iter() {
        match el {
            PathEl::MoveTo(p) => {
                flush(&mut out, start, &mut subpath);
                start = p;
            }
            // What follows a close starts a subpath where it started.
            PathEl::ClosePath => {
                subpath.push(el);
                flush(&mut out, start, &mut subpath);
            }
            _ => subpath.push(el),
        }
    }
    flush(&mut out, start, &mut subpath);
    out
}

/// `path` in kurbo's terms, `s` times as big.
fn bez(path: &Path, s: f64) -> BezPath {
    let to = |p: Point| kurbo::Point::new(f64::from(p[0]) * s, f64::from(p[1]) * s);
    (path.0.iter())
        .map(|segment| match *segment {
            Segment::Move(p) => PathEl::MoveTo(to(p)),
            Segment::Line(p) => PathEl::LineTo(to(p)),
            Segment::Quad(c, p) => PathEl::QuadTo(to(c), to(p)),
            Segment::Cubic(c1, c2, p) => PathEl::CurveTo(to(c1), to(c2), to(p)),
            Segment::Close => PathEl::ClosePath,
        })
        .collect()
}

/// What takes fractions of a bounding box `[x, y, width, height]` to the units it is in.
pub fn fractions([x, y, w, h]: [f64; 4]) -> Matrix {
    Matrix::affine([w, 0.0, 0.0, h, x, y])
}

/// A path's bounding box, as its curves reach: x, y, width and height.
pub fn bounds(path: &Path) -> [f64; 4] {
    let r = bez(path, 1.0).bounding_box();
    [r.x0, r.y0, r.width(), r.height()]
}

/// The ramp a gradient paints a shape with, and the transform from where the ramp is to the
/// shape's user units. `chain` is the gradient and those its `href`s name in turn, each
/// attribute from the first that has it, the geometry's from those of its kind; `stops` its
/// stops' offsets and colors; `bbox` the shape's bounding box, which its default units are
/// fractions of; `viewport` the size user-space percentages measure. `None` where it paints
/// nothing: without stops, or in units of a bounding box without area.
pub fn gradient(
    chain: &[&Gradient],
    stops: &[(f64, Rgba)],
    bbox: [f64; 4],
    viewport: [f64; 2],
) -> Option<(Ramp, Matrix)> {
    fn pick<T>(chain: &[&Gradient], f: impl Fn(&Gradient) -> Option<T>) -> Option<T> {
        chain.iter().find_map(|g| f(g))
    }
    let radial = chain.first()?.radial;
    let lengths: [Option<Length>; 6] = std::array::from_fn(|k| {
        pick(chain, |g| {
            (g.radial == radial).then_some(g.lengths[k]).flatten()
        })
    });
    let [.., w, h] = bbox;
    let (units, basis) = match pick(chain, |g| g.user_space).unwrap_or(false) {
        true => {
            let [vw, vh] = viewport;
            (
                Matrix::IDENTITY,
                [vw, vh, ((vw * vw + vh * vh) / 2.0).sqrt()],
            )
        }
        false if w > 0.0 && h > 0.0 => (fractions(bbox), [1.0; 3]),
        false => return None,
    };
    let transform = pick(chain, |g| g.transform).unwrap_or(Matrix::IDENTITY);
    let (first, last) = (stops.first()?, stops.last()?);
    let color = |c: Rgba| c.map(|v| v as f32);
    let percent = |p| Length {
        units: 0.0,
        percent: p,
    };
    let at =
        |k: usize, or: Length, axis: usize| lengths[k].unwrap_or(or).resolve(basis[axis]) as f32;
    let geometry = match radial {
        true => {
            let center = [at(0, percent(50.0), 0), at(1, percent(50.0), 1)];
            let radius = at(2, percent(50.0), 2);
            let focus = [
                lengths[3].map_or(center[0], |l| l.resolve(basis[0]) as f32),
                lengths[4].map_or(center[1], |l| l.resolve(basis[1]) as f32),
            ];
            let focus_radius = at(5, percent(0.0), 2);
            match (focus == center && focus_radius == 0.0, radius > 0.0) {
                (_, false) => None,
                (true, true) => Some(Geometry::Ellipse(center, [radius; 2])),
                (false, true) => Some(Geometry::Focal {
                    focus,
                    focus_radius,
                    center,
                    radius,
                }),
            }
        }
        false => {
            let start = [at(0, percent(0.0), 0), at(1, percent(0.0), 1)];
            let end = [at(2, percent(100.0), 0), at(3, percent(0.0), 1)];
            (start != end).then_some(Geometry::Line([start, end]))
        }
    };
    // Without a line or a radius, the last stop's color everywhere.
    let Some(geometry) = geometry else {
        let ramp = Ramp {
            geometry: Geometry::Line([[0.0, 0.0], [1.0, 0.0]]),
            spread: Spread::Pad,
            space: Space::Srgb,
            premultiplied: false,
            segments: vec![paint::Segment {
                at: [0.0, 1.0],
                colors: [color(last.1); 2],
            }],
        };
        return Some((ramp, units.times(transform)));
    };
    // Offsets never go back, and the first and last colors reach 0 and 1.
    let mut list = vec![(0.0, color(first.1))];
    let mut floor = 0.0;
    for &(o, c) in stops {
        floor = o.max(floor);
        list.push((floor as f32, color(c)));
    }
    list.push((1.0, color(last.1)));
    let segments = (list.windows(2))
        .map(|w| paint::Segment {
            at: [w[0].0, w[1].0],
            colors: [w[0].1, w[1].1],
        })
        .collect();
    let ramp = Ramp {
        geometry,
        spread: pick(chain, |g| g.spread).unwrap_or(Spread::Pad),
        space: Space::Srgb,
        premultiplied: false,
        segments,
    };
    Some((ramp, units.times(transform)))
}

/// A path's length.
fn arclen(path: &BezPath) -> f64 {
    path.segments().map(|s| s.arclen(ACCURACY)).sum()
}

#[cfg(test)]
mod tests;
