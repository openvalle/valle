//! Transforms: `translate`, `rotate`, `scale` and then `transform`, functions applied from the
//! last to the first, around `transform-origin`, a point in the border box.

use super::css::{self, angle};
use super::math::{self, Calc};
use super::position::{Offset, position};
use super::{Accept, Len, Units, length_text};

/// A transform function.
#[derive(Clone, Debug, PartialEq)]
pub enum Function<P = Calc> {
    /// x and y, percentages of the border box's width and height.
    Translate([P; 2]),
    Scale([f64; 2]),
    /// Degrees, clockwise.
    Rotate(f64),
    /// Degrees, of x toward y and of y toward x: `skew(ax, ay)`.
    Skew([f64; 2]),
    /// `matrix(a, b, c, d, e, f)`.
    Matrix([f64; 6]),
}

impl Function<Len> {
    pub(super) fn computed(&self, u: Units) -> Function {
        match self {
            Function::Translate(t) => Function::Translate(t.each_ref().map(|l| l.calc(u))),
            Function::Scale(s) => Function::Scale(*s),
            Function::Rotate(a) => Function::Rotate(*a),
            Function::Skew(a) => Function::Skew(*a),
            Function::Matrix(m) => Function::Matrix(*m),
        }
    }
}

const LENGTH: Accept = Accept {
    keyword: None,
    negative: true,
    percent: true,
    sizing: &[],
};

/// `transform`: `none`, or functions separated by spaces.
pub(super) fn transform(s: &str) -> Result<Vec<Function<Len>>, String> {
    match s {
        "none" => Ok(Vec::new()),
        "" => Err("\"\" is not a transform; use none".into()),
        _ => css::words(s).into_iter().map(function).collect(),
    }
}

/// The functions that take 3D transforms.
const THREE_D: [&str; 9] = [
    "translatez",
    "translate3d",
    "scalez",
    "scale3d",
    "rotatex",
    "rotatey",
    "rotate3d",
    "matrix3d",
    "perspective",
];

fn function(w: &str) -> Result<Function<Len>, String> {
    let Some((name, inner)) = css::function(w) else {
        return Err(format!("{w} is not a transform function"));
    };
    if THREE_D.contains(&name) {
        return Err(format!("{name}(): 3D transforms are not supported yet"));
    }
    let args = css::commas(inner);
    let named = |e: String| format!("{name}(): {e}");
    let length = |a: &str| length_text(a, LENGTH).map_err(named);
    let number = |a: &str, percent: bool| self::number(a, percent).map_err(named);
    let scale = |a: &str| number(a, true);
    let turn = |a: &str| degrees(a).map_err(named);
    let zero = || Len::Px(0.0);
    Ok(match (name, &args[..]) {
        ("translate" | "translatex", [x]) => Function::Translate([length(x)?, zero()]),
        ("translate", [x, y]) => Function::Translate([length(x)?, length(y)?]),
        ("translatey", [y]) => Function::Translate([zero(), length(y)?]),
        ("scale", [s]) => Function::Scale([scale(s)?; 2]),
        ("scale", [x, y]) => Function::Scale([scale(x)?, scale(y)?]),
        ("scalex", [x]) => Function::Scale([scale(x)?, 1.0]),
        ("scaley", [y]) => Function::Scale([1.0, scale(y)?]),
        ("rotate" | "rotatez", [a]) => Function::Rotate(turn(a)?),
        ("skew" | "skewx", [x]) => Function::Skew([turn(x)?, 0.0]),
        ("skew", [x, y]) => Function::Skew([turn(x)?, turn(y)?]),
        ("skewy", [y]) => Function::Skew([0.0, turn(y)?]),
        ("matrix", [a, b, c, d, e, f]) => Function::Matrix([
            number(a, false)?,
            number(b, false)?,
            number(c, false)?,
            number(d, false)?,
            number(e, false)?,
            number(f, false)?,
        ]),
        _ => {
            let takes = match name {
                "translate" => "one or two lengths",
                "translatex" | "translatey" => "a length",
                "scale" => "one or two numbers",
                "scalex" | "scaley" => "a number",
                "rotate" | "rotatez" | "skewx" | "skewy" => "an angle",
                "skew" => "one or two angles",
                "matrix" => "six numbers",
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

/// `translate`: `none`, or x and y lengths, y 0 when left out.
pub(super) fn translate(s: &str) -> Result<Option<Function<Len>>, String> {
    let length = |a: &str| length_text(a, LENGTH);
    match css::words(s)[..] {
        ["none"] => Ok(None),
        [x] => Ok(Some(Function::Translate([length(x)?, Len::Px(0.0)]))),
        [x, y] => Ok(Some(Function::Translate([length(x)?, length(y)?]))),
        [_, _, _] => Err("a z length: 3D transforms are not supported yet".into()),
        _ => Err(format!(
            "{s:?} is not a translation: none, or one or two lengths"
        )),
    }
}

/// `rotate`: `none`, or an angle about z. Unlike in `rotate()`, a bare 0 is not an angle.
pub(super) fn rotate(s: &str) -> Result<Option<Function<Len>>, String> {
    let a = match css::words(s)[..] {
        ["none"] => return Ok(None),
        [a] | ["z", a] | [a, "z"] => a,
        ["x" | "y", _] | [_, "x" | "y"] | [_, _, _, _] => {
            return Err("an axis but z: 3D transforms are not supported yet".into());
        }
        _ => return Err(format!("{s:?} is not a rotation: none, or an angle")),
    };
    match css::num(a) {
        Some(n) if n.unit.is_empty() => Err(format!("{a} needs a unit: {a}deg")),
        _ => Ok(Some(Function::Rotate(degrees(a)?))),
    }
}

/// `scale`: `none`, or x and y factors, numbers or percentages, y x when left out.
pub(super) fn scale(s: &str) -> Result<Option<Function<Len>>, String> {
    match css::words(s)[..] {
        ["none"] => Ok(None),
        [x] => Ok(Some(Function::Scale([number(x, true)?; 2]))),
        [x, y] => Ok(Some(Function::Scale([number(x, true)?, number(y, true)?]))),
        [_, _, _] => Err("a z factor: 3D transforms are not supported yet".into()),
        _ => Err(format!("{s:?} is not a scale: none, or one or two numbers")),
    }
}

/// `transform-origin`: a point as one or two words of a position.
pub(super) fn origin(s: &str) -> Result<[Offset<Len>; 2], String> {
    let words = css::words(s);
    let bad = || {
        format!(
            "{s:?} is not a transform origin: one or two of left, center, right, top, bottom and \
             lengths"
        )
    };
    match words.len() {
        1 | 2 => position(&words, false).map_err(|_| bad()),
        3 => Err("a z offset: 3D transforms are not supported yet".into()),
        _ => Err(bad()),
    }
}
