//! Evaluates a frame template's expressions at one time. Tracing computes every traced value
//! through the same functions, so the two agree bit for bit.

use std::cmp::Ordering;
use std::rc::Rc;

use crate::js;
use crate::style::{color, css};
use crate::template::{Bin, Cmp, Error, Interp, Op, Part, Program, V};

/// The clock a frame is evaluated at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Clock {
    pub seconds: f64,
    /// `seconds / duration`, within [0, 1].
    pub progress: f64,
}

/// Every expression's value, in program order.
pub fn run(program: &Program, clock: Clock) -> Result<Vec<V>, Error> {
    let mut values = Vec::with_capacity(program.ops.len());
    for op in &program.ops {
        let v = value(op, &values, clock)?;
        values.push(v);
    }
    Ok(values)
}

/// One expression's value over the values of the expressions before it.
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
        Op::Freeze(input, [from, to]) => V::Num(match num(input) {
            t if t < *from => t,
            t if t < *to => *from,
            t => t - (to - from),
        }),
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
}
