//! Evaluates a frame template's expressions at one time. Tracing computes every traced value
//! through the same functions, so the two agree bit for bit.

use std::cmp::Ordering;
use std::rc::Rc;

use crate::js;
use crate::noise;
use crate::path::edit;
use crate::style::{color, css};
use crate::template::{
    Axis, Bin, Boolean, Cmp, Error, Interp, Measure, Op, Part, PathFn, Program, Shape, V,
};
use crate::time::Tick;

/// The clock a frame is evaluated at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Clock {
    pub seconds: f64,
    /// `seconds / duration`, within [0, 1].
    pub progress: f64,
}

/// A unit's weight at `position` (its index over the count) in the range from `start` to `end`,
/// both moved by `offset`, as After Effects' range selector weighs it: an empty or reversed range
/// weighs nothing, and a soft edge is centered on its end, unless the range reaches past it.
fn range(
    shape: Shape,
    position: f64,
    [start, end]: [f64; 2],
    offset: f64,
    softness: f64,
) -> Result<f64, Error> {
    let within = |v: f64| (0.0..=1.0).contains(&v);
    if !within(start) || !within(end) || !within(softness) || !offset.is_finite() {
        return Err(
            "rangeSelector start, end and softness are numbers from 0 to 1, and offset a number"
                .into(),
        );
    }
    if start >= end {
        return Ok(0.0);
    }
    let [left, right] = [start + offset, end + offset];
    if shape == Shape::Square || softness == 0.0 {
        return Ok(f64::from(u8::from(position >= left && position <= right)));
    }
    let left_edge = match left <= 0.0 {
        true => 1.0,
        false => (0.5 + (position - left) / softness).clamp(0.0, 1.0),
    };
    let right_edge = match right >= 1.0 {
        true => 1.0,
        false => (0.5 + (right - position) / softness).clamp(0.0, 1.0),
    };
    let smooth = |v: f64| v * v * (3.0 - 2.0 * v);
    Ok(match shape {
        Shape::Square | Shape::Ramp => left_edge.min(right_edge),
        Shape::Smooth => smooth(left_edge).min(smooth(right_edge)),
        Shape::Triangle => {
            let center = left / 2.0 + right / 2.0;
            let half = (right - left + softness) / 2.0;
            (1.0 - (position - center).abs() / half).clamp(0.0, 1.0)
        }
    })
}

/// Every expression's value, in program order, a late one's 0 until `settle` gives it.
pub fn run(program: &Program, clock: Clock) -> Result<Vec<V>, Error> {
    let late = program.late();
    let mut values = Vec::with_capacity(program.ops.len());
    for (op, late) in program.ops.iter().zip(late) {
        let v = match late {
            true => V::Num(0.0),
            false => value(op, &values, clock)?,
        };
        values.push(v);
    }
    Ok(values)
}

/// The values of the expressions `read` marks, none of them late, as `run` gives them; the
/// others' are 0.
pub fn run_only(program: &Program, clock: Clock, read: &[bool]) -> Result<Vec<V>, Error> {
    let mut values = Vec::with_capacity(program.ops.len());
    for (op, &read) in program.ops.iter().zip(read) {
        let v = match read {
            true => value(op, &values, clock)?,
            false => V::Num(0.0),
        };
        values.push(v);
    }
    Ok(values)
}

/// Gives the late expressions their values once the frame is laid out: `bounds` measures the box
/// of the element with a key path.
pub fn settle(
    program: &Program,
    clock: Clock,
    values: &mut [V],
    bounds: &dyn Fn(&str, Measure) -> f64,
) -> Result<(), Error> {
    for (i, (op, late)) in program.ops.iter().zip(program.late()).enumerate() {
        if late {
            values[i] = match op {
                Op::Bounds(path, measure) => V::Num(bounds(path, *measure)),
                _ => value(op, &values[..i], clock)?,
            };
        }
    }
    Ok(())
}

/// One expression's value over the values of the expressions before it; a box measures 0, as it
/// does while tracing.
pub fn value(op: &Op, values: &[V], clock: Clock) -> Result<V, Error> {
    let num = |i: &u32| values[*i as usize].num();
    Ok(match op {
        Op::Const(v) => v.clone(),
        Op::Seconds => V::Num(clock.seconds),
        Op::Progress => V::Num(clock.progress),
        Op::Bin(b, x, y) => {
            let (x, y) = (num(x), num(y));
            V::Num(match b {
                Bin::Add => x + y,
                Bin::Sub => x - y,
                Bin::Mul => x * y,
                Bin::Div => x / y,
                // Rust's `%` on floats is C's fmod, as JavaScript's.
                Bin::Rem => x % y,
                Bin::Pow => js::pow(x, y),
            })
        }
        Op::Cmp(c, x, y) => {
            let (x, y) = (num(x), num(y));
            V::Bool(match c {
                Cmp::Lt => x < y,
                Cmp::Le => x <= y,
                Cmp::Gt => x > y,
                Cmp::Ge => x >= y,
                Cmp::Eq => x == y,
                Cmp::Ne => x != y,
            })
        }
        Op::Neg(x) => V::Num(-num(x)),
        Op::Select(c, a, b) => {
            let pick = if values[*c as usize].truthy() { a } else { b };
            values[*pick as usize].clone()
        }
        Op::Math(f, args) => V::Num(f.call(&args.iter().map(num).collect::<Vec<_>>())),
        Op::Template(parts) => {
            let mut s = String::new();
            for p in parts {
                match p {
                    Part::Text(t) => s.push_str(t),
                    Part::Expr(i) => s.push_str(&values[*i as usize].text()),
                }
            }
            V::Str(Rc::from(s))
        }
        Op::Interp(i) => interpolate(i, values)?,
        Op::Stage(input, stage) => V::Num(stage.at(num(input))),
        Op::Table(input, table) => V::Num(table.at(num(input))),
        Op::Wiggle(input, wiggle) => V::Num(wiggle.at(num(input))),
        Op::Particle(input, system, index, field) => system.field(*index, *field, num(input)),
        Op::Path(f, args) => path(
            *f,
            &args
                .iter()
                .map(|a| &values[*a as usize])
                .collect::<Vec<_>>(),
        )?,
        Op::Format(input, format) => V::Str(Rc::from(format.text(num(input)))),
        Op::Bounds(..) => V::Num(0.0),
        Op::Range(shape, args) => {
            let [position, start, end, offset, softness] = args[..] else {
                return Err(
                    "rangeSelector takes a position, start, end, offset and softness".into(),
                );
            };
            let [p, start, end, offset, softness] =
                [position, start, end, offset, softness].map(|i| values[i as usize].num());
            V::Num(range(*shape, p, [start, end], offset, softness)?)
        }
        Op::Noise(seed, args) => V::Num(match args[..] {
            [x] => noise::noise1(*seed, 0, num(&x)),
            [x, y] => noise::noise2(*seed, num(&x), num(&y)),
            _ => return Err("noise takes one or two coordinates".into()),
        }),
        Op::Freeze(input, [from, to]) => V::Num(match num(input) {
            t if t < *from => t,
            t if t < *to => *from,
            t => t - (to - from),
        }),
        Op::Scope(input, offset, speed) => {
            let t = Tick::from_seconds(num(input))?;
            let t = (t.0.checked_sub(offset.0))
                .ok_or("a TimeScope's time is outside the supported range of about ±420 days")?;
            V::Num(speed.apply(Tick(t))?.seconds())
        }
        Op::Sample(input, sample) => V::Num(sample.at(Tick::from_seconds(num(input))?).seconds()),
        Op::Clip(input, clip, part) => clip.at(*part, Tick::from_seconds(num(input))?)?,
        Op::Spring {
            input,
            spring,
            velocity,
        } => {
            let (x, v) = spring.at(num(input));
            V::Num(if *velocity { v } else { x })
        }
    })
}

fn path(f: PathFn, args: &[&V]) -> Result<V, Error> {
    let p = edit::bez(&args[0].text())?;
    let (point, direction) = match f {
        PathFn::Length => return Ok(V::Num(edit::length(&p))),
        PathFn::Inflate(join) => {
            let made = edit::inflate(&p, args[1].num(), join);
            return Ok(V::Str(Rc::from(edit::data(&made))));
        }
        PathFn::Morph(samples) => {
            let to = edit::bez(&args[1].text())?;
            let made = edit::morph(&p, &to, args[2].num(), samples.map(|n| n as usize));
            return Ok(V::Str(Rc::from(edit::data(&made))));
        }
        PathFn::Boolean(op) => {
            let op = match op {
                Boolean::Union => "union",
                Boolean::Intersect => "intersect",
                Boolean::Subtract => "subtract",
                Boolean::Xor => "xor",
            };
            let made = edit::boolean(&p, &edit::bez(&args[1].text())?, op)?;
            return Ok(V::Str(Rc::from(edit::data(&made))));
        }
        PathFn::RoundCorners | PathFn::PuckerBloat | PathFn::Twist | PathFn::Zigzag => {
            let n = args[1].num();
            let made = match f {
                PathFn::RoundCorners => edit::round_corners(&p, n),
                PathFn::PuckerBloat => edit::pucker_bloat(&p, n),
                PathFn::Twist => edit::twist(&p, n),
                _ => {
                    let ridges = args[2].num();
                    if !(0.0..=1000.0).contains(&ridges) || ridges.fract() != 0.0 {
                        return Err(format!(
                            "zigzag ridges must be a whole number, 0 to 1000, got {}",
                            args[2].text()
                        ));
                    }
                    edit::zigzag(&p, n, ridges as usize)
                }
            };
            return Ok(V::Str(Rc::from(edit::data(&made))));
        }
        _ => edit::at(&p, args[1].num()),
    };
    Ok(V::Num(match f {
        PathFn::Point(Axis::X) => point.x,
        PathFn::Point(Axis::Y) => point.y,
        PathFn::Tangent(Axis::X) => direction.x,
        PathFn::Tangent(Axis::Y) => direction.y,
        _ => direction.y.atan2(direction.x).to_degrees(),
    }))
}

/// Numbers, colors, or lengths and angles; a segment between two units is a `calc()`.
fn interpolate(i: &Interp, values: &[V]) -> Result<V, Error> {
    let num = |id: u32| values[id as usize].num();
    let x = num(i.input);
    let stops: Vec<(f64, &V)> = (i.stops.iter())
        .map(|&(a, b)| (num(a), &values[b as usize]))
        .collect();
    // NaN stops fail too.
    let increasing = |w: &[(f64, &V)]| w[0].0.partial_cmp(&w[1].0) == Some(Ordering::Less);
    if !stops.windows(2).all(increasing) {
        let inputs: Vec<String> = stops.iter().map(|s| js::number_to_string(s.0)).collect();
        return Err(format!(
            "interpolate: the input range must strictly increase, got [{}]",
            inputs.join(", ")
        ));
    }
    let numbers = stops.iter().all(|s| matches!(s.1, V::Num(_)));
    if !numbers && !stops.iter().all(|s| matches!(s.1, V::Str(_))) {
        return Err("interpolate: the outputs are all numbers or all strings".into());
    }
    if x.is_nan() && numbers {
        return Ok(V::Num(f64::NAN));
    }
    let (first, last) = (stops[0], stops[stops.len() - 1]);
    if !(x >= first.0 || i.extend[0]) {
        return Ok(first.1.clone());
    }
    if x > last.0 && !i.extend[1] {
        return Ok(last.1.clone());
    }
    // The segment holding x; the end segments extend past the range.
    let k = stops[1..stops.len() - 1].partition_point(|s| s.0 <= x);
    let ((x0, y0), (x1, y1)) = (stops[k], stops[k + 1]);
    let t = i.easing.at((x - x0) / (x1 - x0));
    let (a, b) = match (y0, y1) {
        (V::Num(a), V::Num(b)) => return Ok(V::Num(a + t * (b - a))),
        _ => (y0.text(), y1.text()),
    };
    let lerp = |a: f64, b: f64| js::number_to_string(a + t * (b - a));
    let text = match (css::num(&a), css::num(&b)) {
        (Some(p), Some(q)) if p.unit == q.unit => format!("{}{}", lerp(p.value, q.value), p.unit),
        (Some(p), Some(q)) => format!(
            "calc({}{} * {} + {}{} * {})",
            js::number_to_string(p.value),
            p.unit,
            js::number_to_string(1.0 - t),
            js::number_to_string(q.value),
            q.unit,
            js::number_to_string(t),
        ),
        _ => color::between(&a, &b, t, i.mixing).map_err(|e| {
            format!("interpolate: outputs {a} and {b} are not two lengths, angles or colors: {e}")
        })?,
    };
    Ok(V::Str(Rc::from(text)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ease::Easing;
    use crate::js::MathFn;
    use crate::template::{Stage, Table};

    fn program(ops: Vec<Op>) -> Program {
        Program { ops }
    }

    const AT: Clock = Clock {
        seconds: 1.5,
        progress: 0.75,
    };

    #[test]
    fn arithmetic_and_selects() {
        let p = program(vec![
            Op::Seconds,
            Op::Const(V::Num(2.0)),
            Op::Bin(Bin::Mul, 0, 1),
            Op::Cmp(Cmp::Gt, 2, 1),
            Op::Select(3, 0, 1),
            Op::Math(MathFn::Floor, vec![2]),
            Op::Neg(5),
            Op::Bin(Bin::Rem, 0, 1),
            Op::Bin(Bin::Pow, 1, 1),
            Op::Progress,
        ]);
        let v = run(&p, AT).unwrap();
        assert_eq!(v[2], V::Num(3.0));
        assert_eq!(v[3], V::Bool(true));
        assert_eq!(v[4], V::Num(1.5));
        assert_eq!(v[6], V::Num(-3.0));
        assert_eq!(v[7], V::Num(1.5));
        assert_eq!(v[8], V::Num(4.0));
        assert_eq!(v[9], V::Num(0.75));
    }

    #[test]
    fn run_only_evaluates_what_the_roots_read() {
        let p = program(vec![
            Op::Seconds,
            Op::Const(V::Num(2.0)),
            Op::Bin(Bin::Mul, 0, 1),
            Op::Progress,
            Op::Bin(Bin::Add, 3, 1),
        ]);
        let read = p.reading([2]);
        assert_eq!(read, [true, true, true, false, false]);
        let v = run_only(&p, AT, &read).unwrap();
        assert_eq!(v[..3], run(&p, AT).unwrap()[..3]);
        assert_eq!(v[3..], [V::Num(0.0), V::Num(0.0)]);
    }

    #[test]
    fn templates_print_as_javascript() {
        let p = program(vec![
            Op::Seconds,
            Op::Const(V::Num(1e21)),
            Op::Const(V::Bool(false)),
            Op::Template(vec![
                Part::Text("blur(".into()),
                Part::Expr(0),
                Part::Text("px) ".into()),
                Part::Expr(1),
                Part::Text(" ".into()),
                Part::Expr(2),
            ]),
        ]);
        assert_eq!(
            run(&p, AT).unwrap()[3],
            V::Str("blur(1.5px) 1e+21 false".into())
        );
    }

    fn interp(x: f64, extend: [bool; 2]) -> Result<f64, Error> {
        let ops = vec![
            Op::Const(V::Num(x)),
            Op::Const(V::Num(0.0)),
            Op::Const(V::Num(1.0)),
            Op::Const(V::Num(3.0)),
            Op::Const(V::Num(10.0)),
            Op::Const(V::Num(20.0)),
            Op::Const(V::Num(0.0)),
            Op::Interp(Rc::new(Interp {
                input: 0,
                stops: vec![(1, 4), (2, 5), (3, 6)],
                extend,
                easing: Easing::Linear,
                mixing: None,
            })),
        ];
        Ok(run(&program(ops), AT)?[7].num())
    }

    #[test]
    fn interpolation() {
        assert_eq!(interp(0.5, [false; 2]), Ok(15.0));
        assert_eq!(interp(2.0, [false; 2]), Ok(10.0));
        assert_eq!(interp(1.0, [false; 2]), Ok(20.0));
        assert_eq!(interp(-1.0, [false; 2]), Ok(10.0));
        assert_eq!(interp(-1.0, [true, false]), Ok(0.0));
        assert_eq!(interp(5.0, [false; 2]), Ok(0.0));
        assert_eq!(interp(5.0, [false, true]), Ok(-20.0));
        assert!(interp(f64::NAN, [false; 2]).unwrap().is_nan());
        let decreasing = vec![
            Op::Const(V::Num(0.0)),
            Op::Const(V::Num(1.0)),
            Op::Const(V::Num(0.0)),
            Op::Interp(Rc::new(Interp {
                input: 0,
                stops: vec![(1, 1), (2, 2)],
                extend: [false; 2],
                easing: Easing::Linear,
                mixing: None,
            })),
        ];
        let e = run(&program(decreasing), AT).unwrap_err();
        assert!(e.contains("[1, 0]"), "{e}");
    }

    #[test]
    fn stages() {
        let stage = |spec: &str| Stage::parse(spec).unwrap();
        let once = stage("1|2|1|once");
        let at = |s: Stage, ts: &[f64]| ts.iter().map(|&t| s.at(t)).collect::<Vec<_>>();
        assert_eq!(
            at(once, &[0.0, 1.0, 2.0, 3.0, 9.0]),
            [0.0, 0.0, 0.5, 1.0, 1.0]
        );
        let repeat = stage("0|1|4|repeat");
        assert_eq!(at(repeat, &[0.125, 0.25, 0.375, 1.0]), [0.5, 0.0, 0.5, 1.0]);
        let yoyo = stage("0|1|2|yoyo");
        assert_eq!(
            at(yoyo, &[0.125, 0.25, 0.375, 0.5, 1.0]),
            [0.5, 1.0, 0.5, 0.0, 0.0]
        );
        assert!(stage("0|1|1|once").at(f64::NAN).is_nan());
        for bad in [
            "0|0|1|once",
            "0|1|0|repeat",
            "0|1|1|twice",
            "x|1|1|once",
            "0|1|1",
        ] {
            assert!(Stage::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn tables() {
        let table = Table::parse("0.5|1.2|0 1 3 4").unwrap();
        let at = |t| table.at(t);
        assert_eq!(
            [-1.0, 0.25, 0.5, 0.75, 1.0, 1.2, 9.0].map(at),
            [0.0, 0.5, 1.0, 2.0, 3.0, 3.4, 3.4]
        );
        for bad in ["0|1|0 1", "1|1|0", "0.5|1.2|0 1 2", "1|1|0 x", "1|1"] {
            assert!(Table::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn frozen_time() {
        let ops = |t| vec![Op::Const(V::Num(t)), Op::Freeze(0, [1.0, 1.5])];
        let at = |t| run(&program(ops(t)), AT).unwrap()[1].num();
        assert_eq!(
            [0.5, 1.0, 1.25, 1.5, 2.0].map(at),
            [0.5, 1.0, 1.0, 1.0, 1.5]
        );
    }

    fn between(x: f64, a: V, b: V) -> Result<String, Error> {
        let ops = vec![
            Op::Const(V::Num(x)),
            Op::Const(V::Num(0.0)),
            Op::Const(V::Num(1.0)),
            Op::Const(a),
            Op::Const(b),
            Op::Interp(Rc::new(Interp {
                input: 0,
                stops: vec![(1, 3), (2, 4)],
                extend: [false; 2],
                easing: Easing::Linear,
                mixing: None,
            })),
        ];
        Ok(run(&program(ops), AT)?[5].text().into_owned())
    }

    #[test]
    fn outputs_with_units_and_colors() {
        let s = |s: &str| V::Str(Rc::from(s));
        assert_eq!(between(0.25, s("10px"), s("30px")), Ok("15px".into()));
        assert_eq!(
            between(0.5, s("0deg"), s("1turn")).unwrap(),
            "calc(0deg * 0.5 + 1turn * 0.5)"
        );
        assert_eq!(between(2.0, s("1em"), s("2em")), Ok("2em".into()));
        let gray = "color(srgb 0.25 0.25 0.25 / 1)";
        assert_eq!(between(0.25, s("#000"), s("white")).unwrap(), gray);
        assert!(between(0.5, V::Num(0.0), s("1px")).is_err());
        assert!(between(0.5, s("1px"), s("wide")).is_err());
    }

    #[test]
    fn range_selectors_weigh_units_by_their_shape() {
        let at = |shape, [start, end]: [f64; 2], softness| -> Vec<f64> {
            (0..4)
                .map(|i| range(shape, f64::from(i) / 4.0, [start, end], 0.0, softness).unwrap())
                .collect()
        };
        assert_eq!(at(Shape::Square, [0.25, 0.5], 0.5), [0.0, 1.0, 1.0, 0.0]);
        assert_eq!(at(Shape::Ramp, [0.0, 0.5], 0.5), [1.0, 1.0, 0.5, 0.0]);
        assert_eq!(at(Shape::Smooth, [0.0, 0.5], 0.5), [1.0, 1.0, 0.5, 0.0]);
        assert_eq!(at(Shape::Triangle, [0.25, 0.75], 0.5), [0.0, 0.5, 1.0, 0.5]);
        assert_eq!(at(Shape::Ramp, [0.5, 0.5], 0.5), [0.0; 4]);
        // Moved by the offset.
        assert_eq!(range(Shape::Square, 0.75, [0.0, 0.25], 0.5, 0.0), Ok(1.0));
        assert!(range(Shape::Square, 0.0, [0.0, 1.5], 0.0, 0.0).is_err());
    }
}
