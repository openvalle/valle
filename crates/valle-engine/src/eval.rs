//! Evaluates a frame template's expressions at one time. Tracing computes every traced value
//! through the same functions, so the two agree bit for bit.

use std::cmp::Ordering;
use std::rc::Rc;

use crate::js;
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
        Op::Interp(i) => V::Num(interpolate(i, values)?),
    })
}

fn interpolate(i: &Interp, values: &[V]) -> Result<f64, Error> {
    let num = |id: u32| values[id as usize].num();
    let x = num(i.input);
    let stops: Vec<(f64, f64)> = i.stops.iter().map(|&(a, b)| (num(a), num(b))).collect();
    // NaN stops fail too.
    let increasing = |w: &[(f64, f64)]| w[0].0.partial_cmp(&w[1].0) == Some(Ordering::Less);
    if !stops.windows(2).all(increasing) {
        let inputs: Vec<String> = stops.iter().map(|s| js::number_to_string(s.0)).collect();
        return Err(format!(
            "interpolate: the input range must strictly increase, got [{}]",
            inputs.join(", ")
        ));
    }
    if x.is_nan() {
        return Ok(f64::NAN);
    }
    let (first, last) = (stops[0], stops[stops.len() - 1]);
    if x < first.0 && !i.extend[0] {
        return Ok(first.1);
    }
    if x > last.0 && !i.extend[1] {
        return Ok(last.1);
    }
    // The segment holding x; the end segments extend past the range.
    let k = stops[1..stops.len() - 1].partition_point(|s| s.0 <= x);
    let ((x0, y0), (x1, y1)) = (stops[k], stops[k + 1]);
    let t = i.easing.at((x - x0) / (x1 - x0));
    Ok(y0 + t * (y1 - y0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ease::Easing;
    use crate::js::MathFn;

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
            })),
        ];
        let e = run(&program(decreasing), AT).unwrap_err();
        assert!(e.contains("[1, 0]"), "{e}");
    }
}
