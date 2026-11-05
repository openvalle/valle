//! CSS math functions in lengths, numbers and angles: `calc()`, `min()`, `max()` and `clamp()`,
//! with the constants `e`, `pi`, `infinity` and `NaN`. Once numbers have multiplied into the
//! lengths, any of them is the larger or smaller of sums of lengths: adding to a `min()` adds to
//! each of its arguments, and a negative factor turns it into a `max()`.
//!
//! A computed one with a percentage in it waits for layout, which knows what the percentage is
//! of. Taffy holds a handle to it, which numbers the calcs of a frame's styles in order.

use super::css::{self, Num};
use super::units::Len;

/// The larger or smaller of sums: of lengths in their units as declared, or of px and a fraction
/// of what percentages are of once computed ([`Calc`]).
#[derive(Clone, Debug, PartialEq)]
pub enum Math<T> {
    Sum(T),
    Min(Vec<Math<T>>),
    Max(Vec<Math<T>>),
}

/// A computed math function with a percentage in it: sums of px and a fraction of the basis.
pub type Calc = Math<[f64; 2]>;

/// A math function as declared: sums of lengths in their units.
pub(super) type Expr = Math<Vec<Len>>;

impl<T> Math<T> {
    pub fn map<U>(&self, f: &impl Fn(&T) -> U) -> Math<U> {
        match self {
            Math::Sum(t) => Math::Sum(f(t)),
            Math::Min(ms) => Math::Min(ms.iter().map(|m| m.map(f)).collect()),
            Math::Max(ms) => Math::Max(ms.iter().map(|m| m.map(f)).collect()),
        }
    }

    /// Its value, `f` giving each sum's.
    pub fn value(&self, f: &impl Fn(&T) -> f64) -> f64 {
        match self {
            Math::Sum(t) => f(t),
            Math::Min(ms) => (ms.iter().map(|m| m.value(f))).fold(f64::INFINITY, f64::min),
            Math::Max(ms) => (ms.iter().map(|m| m.value(f))).fold(f64::NEG_INFINITY, f64::max),
        }
    }

    pub fn any(&self, f: &impl Fn(&T) -> bool) -> bool {
        match self {
            Math::Sum(t) => f(t),
            Math::Min(ms) | Math::Max(ms) => ms.iter().any(|m| m.any(f)),
        }
    }
}

impl Calc {
    /// px, its percentages of `basis`.
    pub fn resolve(&self, basis: f32) -> f32 {
        self.value(&|&[px, of]| px + of * f64::from(basis)) as f32
    }
}

/// The handle Taffy holds to the frame's `i`th calc: not a pointer, as Taffy allows, but it
/// keeps the low three bits clear, and 0 for nothing.
pub fn handle(i: usize) -> *const () {
    std::ptr::without_provenance((i + 1) << 3)
}

/// Which of the frame's calcs a handle names.
pub fn index(handle: *const ()) -> usize {
    (handle.addr() >> 3) - 1
}

/// Whether `s` is a math function.
pub(super) fn is_math(s: &str) -> bool {
    css::function(s).is_some_and(|(name, _)| FUNCTIONS.contains(&name))
}

const FUNCTIONS: [&str; 4] = ["calc", "min", "max", "clamp"];

/// The math function `s` as a length, `unit` giving the lengths of numbers with units in it; an
/// infinity in it comes to the largest length, and NaN to 0.
pub(super) fn length(s: &str, unit: impl Fn(Num) -> Result<Len, String>) -> Result<Expr, String> {
    match parse(s, unit)? {
        Value::Length(m) => Ok(m.map(&|sum: &Vec<Len>| sum.iter().map(Len::finite).collect())),
        Value::Number(_) => Err(format!("{s} is a number, not a length")),
    }
}

/// The math function `s` as a number: `calc(10 * -1)`.
pub(super) fn number(s: &str) -> Result<f64, String> {
    let unit = |n: Num| Err(format!("{}{} is not a number", n.value, n.unit));
    match parse(s, unit)? {
        Value::Number(x) if x.is_finite() => Ok(x),
        Value::Number(x) => Err(format!("{s} is {x}, not a finite number")),
        Value::Length(_) => Err(format!("{s} is a length, not a number")),
    }
}

/// Degrees of an angle, `45deg`, or of a math function of angles, `calc(45deg * -1)`.
pub(super) fn degrees(s: &str) -> Result<f64, String> {
    if let Some(degrees) = css::angle(s) {
        return Ok(degrees);
    }
    if !is_math(s) {
        return Err(format!("{s} is not an angle"));
    }
    // Angles are degrees as px are lengths: nothing they are of waits for layout.
    let unit = |n: Num| match css::degrees(&n) {
        Some(degrees) => Ok(Len::Px(degrees)),
        None => Err(format!("{}{} is not an angle", n.value, n.unit)),
    };
    match parse(s, unit)? {
        Value::Length(m) => match m.value(&|terms: &Vec<Len>| plain(terms)) {
            d if d.is_finite() => Ok(d),
            d => Err(format!("{s} is {d}, not a finite angle")),
        },
        Value::Number(_) => Err(format!("{s} is a number, not an angle")),
    }
}

/// A number of a math function of numbers or of percentages, `calc(125% * -1)` as -1.25.
pub(super) fn factor(s: &str) -> Result<f64, String> {
    // Like angles, percentages of 1 wait for nothing.
    let unit = |n: Num| match n.unit {
        "%" => Ok(Len::Px(n.value / 100.0)),
        _ => Err(format!("{}{} is not a number", n.value, n.unit)),
    };
    let x = match parse(s, unit)? {
        Value::Length(m) => m.value(&|terms: &Vec<Len>| plain(terms)),
        Value::Number(x) => x,
    };
    match x.is_finite() {
        true => Ok(x),
        false => Err(format!("{s} is {x}, not a finite number")),
    }
}

/// The sum of terms that are all px, standing for what is not a length.
fn plain(terms: &[Len]) -> f64 {
    (terms.iter())
        .map(|l| match l {
            Len::Px(x) => *x,
            _ => f64::NAN,
        })
        .sum()
}

fn parse(s: &str, unit: impl Fn(Num) -> Result<Len, String>) -> Result<Value, String> {
    let mut p = Parser { s, at: 0, unit };
    let v = p.term().and_then(|v| match p.s[p.at..].trim() {
        "" => Ok(v),
        rest => Err(format!("{rest} is left over")),
    });
    v.map_err(|e| format!("{s}: {e}"))
}

/// What math comes to: a number, or a length.
enum Value {
    Number(f64),
    Length(Expr),
}

struct Parser<'a, F> {
    s: &'a str,
    at: usize,
    unit: F,
}

impl<F: Fn(Num) -> Result<Len, String>> Parser<'_, F> {
    fn rest(&self) -> &str {
        &self.s[self.at..]
    }

    fn next(&self) -> Option<char> {
        self.rest().chars().next()
    }

    /// Skips white space, and says whether there was any.
    fn space(&mut self) -> bool {
        let n = self.rest().len() - self.rest().trim_start().len();
        self.at += n;
        n > 0
    }

    fn eat(&mut self, c: char) -> bool {
        let next = self.next() == Some(c);
        self.at += usize::from(next);
        next
    }

    fn unexpected(&self) -> String {
        match self.next() {
            Some(c) => format!("{c} is out of place"),
            None => "it ends too soon".into(),
        }
    }

    /// Products added and subtracted; `+` and `-` need white space around them, or they are
    /// signs.
    fn sum(&mut self) -> Result<Value, String> {
        let mut v = self.product()?;
        loop {
            let before = self.at;
            let spaced = self.space();
            let sign = match self.next() {
                Some('+') => 1.0,
                Some('-') => -1.0,
                _ => {
                    self.at = before;
                    return Ok(v);
                }
            };
            self.at += 1;
            if !spaced || !self.space() {
                return Err("+ and - need white space around them".into());
            }
            let b = self.product()?;
            v = add(v, mul(Value::Number(sign), b)?)?;
        }
    }

    fn product(&mut self) -> Result<Value, String> {
        let mut v = self.term()?;
        loop {
            let before = self.at;
            self.space();
            let op = match self.next() {
                Some(op @ ('*' | '/')) => op,
                _ => {
                    self.at = before;
                    return Ok(v);
                }
            };
            self.at += 1;
            self.space();
            let b = self.term()?;
            v = if op == '*' { mul(v, b)? } else { div(v, b)? };
        }
    }

    /// A number, a sum in parentheses, or a math function.
    fn term(&mut self) -> Result<Value, String> {
        if self.eat('(') {
            self.space();
            let v = self.sum()?;
            self.space();
            return if self.eat(')') {
                Ok(v)
            } else {
                Err(self.unexpected())
            };
        }
        let name = FUNCTIONS.into_iter().find(|f| {
            let rest = self.rest();
            rest.starts_with(f) && rest[f.len()..].starts_with('(')
        });
        let Some(name) = name else {
            return self.number();
        };
        self.at += name.len() + 1;
        let mut args = self.args()?;
        match (name, args.len()) {
            ("calc", 1) => Ok(args.remove(0)),
            ("min", _) => extreme(args, Math::Min, f64::min),
            ("max", _) => extreme(args, Math::Max, f64::max),
            // clamp(low, v, high) = max(low, min(v, high)).
            ("clamp", 3) => {
                let below = extreme(args.split_off(1), Math::Min, f64::min)?;
                extreme(vec![args.remove(0), below], Math::Max, f64::max)
            }
            ("calc", _) => Err("calc() takes one sum".into()),
            _ => Err("clamp() takes a minimum, a value and a maximum".into()),
        }
    }

    /// Sums separated by commas, to a closing parenthesis.
    fn args(&mut self) -> Result<Vec<Value>, String> {
        let mut args = Vec::new();
        loop {
            self.space();
            args.push(self.sum()?);
            self.space();
            if self.eat(')') {
                return Ok(args);
            }
            if !self.eat(',') {
                return Err(self.unexpected());
            }
        }
    }

    fn number(&mut self) -> Result<Value, String> {
        let b = self.rest().as_bytes();
        let mut end = usize::from(matches!(b.first(), Some(b'+' | b'-')));
        while let Some(&c) = b.get(end) {
            let sign = matches!(c, b'+' | b'-') && matches!(b[end - 1], b'e' | b'E');
            if !(c.is_ascii_alphanumeric() || matches!(c, b'.' | b'%') || sign) {
                break;
            }
            end += 1;
        }
        let token = &self.rest()[..end];
        let constant = match token {
            "e" => Some(std::f64::consts::E),
            "pi" => Some(std::f64::consts::PI),
            "infinity" => Some(f64::INFINITY),
            "-infinity" => Some(f64::NEG_INFINITY),
            "nan" => Some(f64::NAN),
            _ => None,
        };
        if let Some(c) = constant {
            self.at += end;
            return Ok(Value::Number(c));
        }
        let n = css::num(token).ok_or_else(|| match token {
            "" => self.unexpected(),
            _ => format!("{token} is not a number or a length"),
        })?;
        let v = match n.unit {
            "" => Value::Number(n.value),
            _ => Value::Length(Math::Sum(vec![(self.unit)(n)?])),
        };
        self.at += end;
        Ok(v)
    }
}

fn add(a: Value, b: Value) -> Result<Value, String> {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => Ok(Value::Number(x + y)),
        (Value::Length(x), Value::Length(y)) => Ok(Value::Length(sum(&x, &y))),
        _ => Err("a number and a length do not add".into()),
    }
}

/// `a + b`, added into the arguments of any `min()` and `max()`.
fn sum(a: &Expr, b: &Expr) -> Expr {
    match (a, b) {
        (Math::Sum(x), Math::Sum(y)) => Math::Sum([&x[..], &y[..]].concat()),
        (Math::Min(xs), _) => Math::Min(xs.iter().map(|x| sum(x, b)).collect()),
        (Math::Max(xs), _) => Math::Max(xs.iter().map(|x| sum(x, b)).collect()),
        (_, Math::Min(_) | Math::Max(_)) => sum(b, a),
    }
}

/// `m × k`: a negative factor swaps `min()` and `max()`.
fn scale(m: &Expr, k: f64) -> Expr {
    match m {
        Math::Sum(ls) => Math::Sum(ls.iter().map(|l| l.scaled(k)).collect()),
        Math::Min(ms) | Math::Max(ms) => {
            let ms = ms.iter().map(|m| scale(m, k)).collect();
            if matches!(m, Math::Min(_)) == (k >= 0.0) {
                Math::Min(ms)
            } else {
                Math::Max(ms)
            }
        }
    }
}

fn mul(a: Value, b: Value) -> Result<Value, String> {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => Ok(Value::Number(x * y)),
        (Value::Number(k), Value::Length(m)) | (Value::Length(m), Value::Number(k)) => {
            Ok(Value::Length(scale(&m, k)))
        }
        (Value::Length(_), Value::Length(_)) => Err("two lengths do not multiply".into()),
    }
}

fn div(a: Value, b: Value) -> Result<Value, String> {
    match (a, b) {
        (_, Value::Number(0.0)) => Err("it divides by 0".into()),
        (Value::Number(x), Value::Number(y)) => Ok(Value::Number(x / y)),
        (Value::Length(m), Value::Number(k)) => Ok(Value::Length(scale(&m, 1.0 / k))),
        (_, Value::Length(_)) => Err("it divides by a length".into()),
    }
}

/// The least or the greatest of `args`, all numbers or all lengths.
fn extreme(
    args: Vec<Value>,
    lengths: fn(Vec<Expr>) -> Expr,
    numbers: fn(f64, f64) -> f64,
) -> Result<Value, String> {
    let mut ns = Vec::new();
    let mut ms = Vec::new();
    for v in args {
        match v {
            Value::Number(x) => ns.push(x),
            Value::Length(m) => ms.push(m),
        }
    }
    match (ns.into_iter().reduce(numbers), ms.is_empty()) {
        (Some(x), true) => Ok(Value::Number(x)),
        (None, false) => Ok(Value::Length(lengths(ms))),
        _ => Err("numbers and lengths do not compare".into()),
    }
}
