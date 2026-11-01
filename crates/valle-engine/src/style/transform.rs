//! Transforms: `translate`, `rotate`, `scale` and then `transform`, functions applied from the
//! last to the first, around `transform-origin`, a point in the border box and a z.

use super::css::{self, angle};
use super::math::{self, Calc};
use super::position::{Offset, position};
use super::{Accept, Len, Units, length_text};

/// A transform function.
#[derive(Clone, Debug, PartialEq)]
pub enum Function<P = Calc> {
    /// x, y and z; x and y may be percentages of the border box's width and height.
    Translate([P; 3]),
    Scale([f64; 3]),
    /// Degrees, clockwise about z.
    Rotate(f64),
    /// Degrees about an axis, x, y and z, clockwise as it points to the viewer; an axis of
    /// length 0 does not rotate.
    Rotate3d([f64; 3], f64),
    /// Degrees, of x toward y and of y toward x: `skew(ax, ay)`.
    Skew([f64; 2]),
    /// `matrix(a, b, c, d, e, f)`.
    Matrix([f64; 6]),
    /// `matrix3d()`, its 16 numbers column by column.
    Matrix3d([f64; 16]),
    /// The viewer's distance from z = 0; `None` for `none`, from infinitely far.
    Perspective(Option<P>),
}

impl Function<Len> {
    pub(super) fn computed(&self, u: Units) -> Function {
        match self {
            Function::Translate(t) => Function::Translate(t.each_ref().map(|l| l.calc(u))),
            Function::Scale(s) => Function::Scale(*s),
            Function::Rotate(a) => Function::Rotate(*a),
            Function::Rotate3d(axis, a) => Function::Rotate3d(*axis, *a),
            Function::Skew(a) => Function::Skew(*a),
            Function::Matrix(m) => Function::Matrix(*m),
            Function::Matrix3d(m) => Function::Matrix3d(*m),
            Function::Perspective(d) => Function::Perspective(d.as_ref().map(|l| l.calc(u))),
        }
    }
}

const LENGTH: Accept = Accept {
    keyword: None,
    negative: true,
    percent: true,
    sizing: &[],
};

/// A z: nothing to take a percentage of.
const Z: Accept = Accept {
    percent: false,
    ..LENGTH
};

const DEPTH: Accept = Accept {
    negative: false,
    ..Z
};

/// `transform`: `none`, or functions separated by spaces.
pub(super) fn transform(s: &str) -> Result<Vec<Function<Len>>, String> {
    match s {
        "none" => Ok(Vec::new()),
        "" => Err("\"\" is not a transform; use none".into()),
        _ => css::words(s).into_iter().map(function).collect(),
    }
}

fn function(w: &str) -> Result<Function<Len>, String> {
    let Some((name, inner)) = css::function(w) else {
        return Err(format!("{w} is not a transform function"));
    };
    let args = css::commas(inner);
    let named = |e: String| format!("{name}(): {e}");
    let length = |a: &str| length_text(a, LENGTH).map_err(named);
    let z = |a: &str| length_text(a, Z).map_err(named);
    let number = |a: &str, percent: bool| self::number(a, percent).map_err(named);
    let scale = |a: &str| number(a, true);
    let turn = |a: &str| degrees(a).map_err(named);
    let zero = || Len::Px(0.0);
    let axis = |x: &str, y: &str, d: &str| -> Result<[f64; 3], String> {
        Ok([number(x, false)?, number(y, false)?, number(d, false)?])
    };
    Ok(match (name, &args[..]) {
        ("translate" | "translatex", [x]) => Function::Translate([length(x)?, zero(), zero()]),
        ("translate", [x, y]) => Function::Translate([length(x)?, length(y)?, zero()]),
        ("translatey", [y]) => Function::Translate([zero(), length(y)?, zero()]),
        ("translatez", [d]) => Function::Translate([zero(), zero(), z(d)?]),
        ("translate3d", [x, y, d]) => Function::Translate([length(x)?, length(y)?, z(d)?]),
        ("scale", [s]) => Function::Scale([scale(s)?, scale(s)?, 1.0]),
        ("scale", [x, y]) => Function::Scale([scale(x)?, scale(y)?, 1.0]),
        ("scalex", [x]) => Function::Scale([scale(x)?, 1.0, 1.0]),
        ("scaley", [y]) => Function::Scale([1.0, scale(y)?, 1.0]),
        ("scalez", [d]) => Function::Scale([1.0, 1.0, scale(d)?]),
        ("scale3d", [x, y, d]) => Function::Scale([scale(x)?, scale(y)?, scale(d)?]),
        ("rotate" | "rotatez", [a]) => Function::Rotate(turn(a)?),
        ("rotatex", [a]) => Function::Rotate3d([1.0, 0.0, 0.0], turn(a)?),
        ("rotatey", [a]) => Function::Rotate3d([0.0, 1.0, 0.0], turn(a)?),
        ("rotate3d", [x, y, d, a]) => Function::Rotate3d(axis(x, y, d)?, turn(a)?),
        ("skew" | "skewx", [x]) => Function::Skew([turn(x)?, 0.0]),
        ("skew", [x, y]) => Function::Skew([turn(x)?, turn(y)?]),
        ("skewy", [y]) => Function::Skew([0.0, turn(y)?]),
        ("matrix", [_, _, _, _, _, _]) => {
            let m = args.iter().map(|a| number(a, false));
            Function::Matrix(m.collect::<Result<Vec<_>, _>>()?.try_into().expect("six"))
        }
        ("matrix3d", a) if a.len() == 16 => {
            let m = args.iter().map(|a| number(a, false));
            Function::Matrix3d(m.collect::<Result<Vec<_>, _>>()?.try_into().expect("16"))
        }
        ("perspective", ["none"]) => Function::Perspective(None),
        ("perspective", [d]) => Function::Perspective(Some(length_text(d, DEPTH).map_err(named)?)),
        _ => {
            let takes = match name {
                "translate" => "one or two lengths",
                "translatex" | "translatey" | "translatez" => "a length",
                "translate3d" => "three lengths",
                "scale" => "one or two numbers",
                "scalex" | "scaley" | "scalez" => "a number",
                "scale3d" => "three numbers",
                "rotate" | "rotatez" | "rotatex" | "rotatey" | "skewx" | "skewy" => "an angle",
                "rotate3d" => "three numbers and an angle",
                "skew" => "one or two angles",
                "matrix" => "six numbers",
                "matrix3d" => "16 numbers",
                "perspective" => "a length or none",
                _ => return Err(format!("{name}() is not a transform function")),
            };
            return Err(format!("{name}() takes {takes}, separated by commas"));
        }
    })
}

/// A number, or a percentage of one where `percent`.
fn number(a: &str, percent: bool) -> Result<f64, String> {
    match css::num(a) {
        Some(n) if n.unit.is_empty() => Ok(n.value),
        Some(n) if percent && n.unit == "%" => Ok(n.value / 100.0),
        _ => Err(format!("{a} is not a number")),
    }
}

fn degrees(a: &str) -> Result<f64, String> {
    match angle(a) {
        Some(degrees) => Ok(degrees),
        None if math::is_math(a) => Err("math functions of angles are not supported yet".into()),
        None => Err(format!("{a} is not an angle")),
    }
}

/// `translate`: `none`, or x, y and z lengths, y and z 0 when left out.
pub(super) fn translate(s: &str) -> Result<Option<Function<Len>>, String> {
    let length = |a: &str| length_text(a, LENGTH);
    let zero = || Len::Px(0.0);
    match css::words(s)[..] {
        ["none"] => Ok(None),
        [x] => Ok(Some(Function::Translate([length(x)?, zero(), zero()]))),
        [x, y] => Ok(Some(Function::Translate([length(x)?, length(y)?, zero()]))),
        [x, y, z] => Ok(Some(Function::Translate([
            length(x)?,
            length(y)?,
            length_text(z, Z)?,
        ]))),
        _ => Err(format!(
            "{s:?} is not a translation: none, or one to three lengths"
        )),
    }
}

/// `rotate`: `none`, or an angle about z, x, y or an axis of three numbers, before or after it.
/// Unlike in `rotate()`, a bare 0 is not an angle.
pub(super) fn rotate(s: &str) -> Result<Option<Function<Len>>, String> {
    let along = |k: &str| match k {
        "x" => Some([1.0, 0.0, 0.0]),
        "y" => Some([0.0, 1.0, 0.0]),
        "z" => Some([0.0, 0.0, 1.0]),
        _ => None,
    };
    let axis = |x: &str, y: &str, z: &str| -> Result<_, String> {
        Ok(Some([
            number(x, false)?,
            number(y, false)?,
            number(z, false)?,
        ]))
    };
    let (axis, a) = match css::words(s)[..] {
        ["none"] => return Ok(None),
        [a] => (None, a),
        [k, a] | [a, k] if along(k).is_some() => (along(k), a),
        [x, y, z, a] if number(x, false).is_ok() => (axis(x, y, z)?, a),
        [a, x, y, z] => (axis(x, y, z)?, a),
        _ => {
            return Err(format!(
                "{s:?} is not a rotation: none, or an angle about x, y, z or three numbers"
            ));
        }
    };
    let degrees = match css::num(a) {
        Some(n) if n.unit.is_empty() => return Err(format!("{a} needs a unit: {a}deg")),
        _ => degrees(a)?,
    };
    Ok(Some(match axis {
        Some(axis) if axis != [0.0, 0.0, 1.0] => Function::Rotate3d(axis, degrees),
        _ => Function::Rotate(degrees),
    }))
}

/// `scale`: `none`, or x, y and z factors, numbers or percentages, y x and z 1 when left out.
pub(super) fn scale(s: &str) -> Result<Option<Function<Len>>, String> {
    let factor = |a: &str| number(a, true);
    match css::words(s)[..] {
        ["none"] => Ok(None),
        [x] => Ok(Some(Function::Scale([factor(x)?, factor(x)?, 1.0]))),
        [x, y] => Ok(Some(Function::Scale([factor(x)?, factor(y)?, 1.0]))),
        [x, y, z] => Ok(Some(Function::Scale([factor(x)?, factor(y)?, factor(z)?]))),
        _ => Err(format!(
            "{s:?} is not a scale: none, or one to three numbers"
        )),
    }
}

/// `perspective`: `none`, or the viewer's distance from the box's children.
pub(super) fn perspective(s: &str) -> Result<Option<Len>, String> {
    match s {
        "none" => Ok(None),
        _ => length_text(s, DEPTH).map(Some),
    }
}

/// `perspective-origin`: a position of one, two or four words.
pub(super) fn perspective_origin(s: &str) -> Result<[Offset<Len>; 2], String> {
    position(&css::words(s), false).map_err(|_| {
        format!(
            "{s:?} is not a perspective origin: one or two of left, center, right, top, bottom \
             and lengths, or keywords with lengths after them"
        )
    })
}

/// `transform-origin`: a point as one or two words of a position, and a z length, 0 when left
/// out.
pub(super) fn origin(s: &str) -> Result<([Offset<Len>; 2], Len), String> {
    let words = css::words(s);
    let bad = || {
        format!(
            "{s:?} is not a transform origin: one or two of left, center, right, top, bottom and \
             lengths, and a length"
        )
    };
    let (at, z) = match words.len() {
        1 | 2 => (&words[..], Len::Px(0.0)),
        3 => (&words[..2], length_text(words[2], Z).map_err(|_| bad())?),
        _ => return Err(bad()),
    };
    Ok((position(at, false).map_err(|_| bad())?, z))
}
