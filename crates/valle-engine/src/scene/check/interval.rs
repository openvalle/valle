//! Conservative validation over a closed time interval. Unknown operations and properties
//! stay on the sampled checker; a successful first frame alone never proves later frames.

use crate::eval::Clock;
use crate::js::MathFn;
use crate::template::{Bin, Cmp, Kind, Op, Part, Piece, Program, Prop, Template, V, Want};

#[derive(Clone, Debug)]
enum Value {
    Exact(V),
    Number(f64, f64),
    Boolean,
    Text,
}

impl Value {
    fn number(a: f64, b: f64) -> Option<Self> {
        (a.is_finite() && b.is_finite()).then_some({
            if a == b {
                Self::Exact(V::Num(a))
            } else {
                Self::Number(a, b)
            }
        })
    }
    fn range(&self) -> Option<(f64, f64)> {
        match self {
            Self::Exact(V::Num(x)) if x.is_finite() => Some((*x, *x)),
            Self::Number(a, b) => Some((*a, *b)),
            _ => None,
        }
    }
    fn truth(&self) -> Option<bool> {
        match self {
            Self::Exact(v) => Some(v.truthy()),
            Self::Number(a, b) if *a > 0.0 || *b < 0.0 => Some(true),
            _ => None,
        }
    }
}

fn supported(op: &Op) -> bool {
    match op {
        Op::Const(_)
        | Op::Seconds
        | Op::Progress
        | Op::Neg(_)
        | Op::Cmp(..)
        | Op::Select(..)
        | Op::Template(_) => true,
        Op::Bin(b, ..) => matches!(b, Bin::Add | Bin::Sub | Bin::Mul | Bin::Div),
        Op::Math(f, _) => matches!(
            f,
            MathFn::Sin
                | MathFn::Cos
                | MathFn::Abs
                | MathFn::Min
                | MathFn::Max
                | MathFn::Floor
                | MathFn::Ceil
        ),
        _ => false,
    }
}

fn paint_property(name: &str) -> bool {
    matches!(
        name,
        "opacity" | "translate" | "rotate" | "scale" | "transform" | "transformOrigin"
    )
}
fn box_property(name: &str) -> bool {
    matches!(
        name,
        "left" | "top" | "right" | "bottom" | "width" | "height"
    )
}

/// Reject work that needs sampled resources, layout-dependent values or unanalysed functions
/// before trying any ranges. This avoids adding a failed proof attempt to every such frame.
pub(super) fn candidate(t: &Template) -> bool {
    if !t.program.ops.iter().all(supported) {
        return false;
    }
    let mut changes = Vec::new();
    for op in &t.program.ops {
        let mut changes_at_time = matches!(op, Op::Seconds | Op::Progress);
        op.operands(|e| changes_at_time |= changes[e as usize]);
        changes.push(changes_at_time);
    }
    let changing = |p: &Prop| matches!(p,Prop::Expr(e) if changes[*e as usize]);
    let text = t
        .nodes
        .iter()
        .any(|n| matches!(n.kind, Kind::Text { .. } | Kind::Span { .. }));
    let property = |name: &str, p: &Prop| {
        !changing(p) || paint_property(name) || (!text && box_property(name))
    };
    for n in &t.nodes {
        if !n.style.iter().all(|(k, p)| property(k, p)) {
            return false;
        }
        match &n.kind {
            Kind::Image { video: Some(_), .. } => return false,
            Kind::Text { content } | Kind::Span { content, .. }
                if content
                    .iter()
                    .any(|p| matches!(p,Piece::Text(p) if changing(p))) =>
            {
                return false;
            }
            Kind::Svg {
                attributes,
                geometry,
                ..
            } if attributes.iter().any(|(_, p)| changing(p))
                || geometry
                    .iter()
                    .flatten()
                    .any(|p| matches!(p, Part::Expr(e) if changes[*e as usize])) =>
            {
                return false;
            }
            Kind::Shader { uniforms, .. } if uniforms.iter().flat_map(|(_, p)| p).any(changing) => {
                return false;
            }
            _ => {}
        }
    }
    t.instances.iter().all(|(_, table)| {
        if let Some(f) = &table.formula {
            if !f.program.ops.iter().all(supported) {
                return false;
            }
            let mut local = vec![false; table.rows[0].len()];
            local.extend(f.imports.iter().map(|e| changes[*e as usize]));
            for op in f.program.ops.iter().skip(local.len()) {
                let mut changing = false;
                op.operands(|e| changing |= local[e as usize]);
                local.push(changing);
            }
            table.fields.iter().zip(&f.outputs).all(|(k, p)| {
                !matches!(p, Prop::Expr(e) if local[*e as usize])
                    || paint_property(k)
                    || (!text && box_property(k))
            })
        } else {
            table
                .rows
                .iter()
                .all(|row| table.fields.iter().zip(row).all(|(k, p)| property(k, p)))
        }
    })
}

fn binary(b: Bin, (a, c): (f64, f64), (d, e): (f64, f64)) -> Option<Value> {
    let all = match b {
        Bin::Add => [a + d, c + e, a + d, c + e],
        Bin::Sub => [a - e, c - d, a - e, c - d],
        Bin::Mul => [a * d, a * e, c * d, c * e],
        Bin::Div if d > 0.0 || e < 0.0 => [a / d, a / e, c / d, c / e],
        _ => return None,
    };
    if all.iter().any(|n| !n.is_finite()) {
        return None;
    }
    let lo = all.into_iter().fold(f64::INFINITY, f64::min);
    let hi = all.into_iter().fold(f64::NEG_INFINITY, f64::max);
    // Each operation encloses floating-point rounding, not just real-number arithmetic.
    Value::number(
        if lo == 0.0 { lo } else { lo.next_down() },
        if hi == 0.0 { hi } else { hi.next_up() },
    )
}

fn compare(c: Cmp, (a, b): (f64, f64), (d, e): (f64, f64)) -> Value {
    let (yes, no) = match c {
        Cmp::Lt => (b < d, a >= e),
        Cmp::Le => (b <= d, a > e),
        Cmp::Gt => (a > e, b <= d),
        Cmp::Ge => (a >= e, b < d),
        Cmp::Eq => (a == b && d == e && a == d, b < d || a > e),
        Cmp::Ne => (b < d || a > e, a == b && d == e && a == d),
    };
    if yes || no {
        Value::Exact(V::Bool(yes))
    } else {
        Value::Boolean
    }
}

fn evaluate(
    program: &Program,
    mut values: Vec<Value>,
    first: Clock,
    last: Clock,
) -> Option<Vec<Value>> {
    let mut concrete: Vec<V> = values
        .iter()
        .map(|v| match v {
            Value::Exact(v) => v.clone(),
            _ => V::Num(0.0),
        })
        .collect();
    for op in program.ops.iter().skip(values.len()) {
        let mut exact = true;
        op.operands(|e| exact &= matches!(values[e as usize], Value::Exact(_)));
        if exact && !matches!(op, Op::Seconds | Op::Progress) {
            // Preserve JavaScript rounding and equality for invariant subexpressions.
            let value = crate::eval::value(op, &concrete, first).ok()?;
            concrete.push(value.clone());
            values.push(Value::Exact(value));
            continue;
        }
        let range = |e: u32| values[e as usize].range();
        let value = match op {
            Op::Seconds => Value::number(first.seconds, last.seconds)?,
            Op::Progress => Value::number(first.progress, last.progress)?,
            Op::Bin(b, a, c) => binary(*b, range(*a)?, range(*c)?)?,
            Op::Neg(a) => {
                let (a, b) = range(*a)?;
                Value::number(-b, -a)?
            }
            Op::Cmp(c, a, b) => compare(*c, range(*a)?, range(*b)?),
            Op::Select(c, a, b) => match values[*c as usize].truth() {
                Some(pick) => values[if pick { *a } else { *b } as usize].clone(),
                None => {
                    let (lo, hi) = range(*a)?;
                    let (c, d) = range(*b)?;
                    Value::number(lo.min(c), hi.max(d))?
                }
            },
            Op::Math(f, args) => {
                let ranges: Vec<_> = args.iter().map(|i| range(*i)).collect::<Option<_>>()?;
                let (lo, hi) = match (f, ranges.as_slice()) {
                    (MathFn::Sin | MathFn::Cos, [_]) => (-1.0, 1.0),
                    (MathFn::Abs, [(a, b)]) => (
                        if *a <= 0.0 && *b >= 0.0 {
                            0.0
                        } else {
                            a.abs().min(b.abs())
                        },
                        a.abs().max(b.abs()),
                    ),
                    (MathFn::Floor, [(a, b)]) => (a.floor(), b.floor()),
                    (MathFn::Ceil, [(a, b)]) => (a.ceil(), b.ceil()),
                    (MathFn::Min, _) if !ranges.is_empty() => (
                        ranges.iter().map(|v| v.0).fold(f64::INFINITY, f64::min),
                        ranges.iter().map(|v| v.1).fold(f64::INFINITY, f64::min),
                    ),
                    (MathFn::Max, _) if !ranges.is_empty() => (
                        ranges.iter().map(|v| v.0).fold(f64::NEG_INFINITY, f64::max),
                        ranges.iter().map(|v| v.1).fold(f64::NEG_INFINITY, f64::max),
                    ),
                    _ => return None,
                };
                Value::number(lo, hi)?
            }
            Op::Template(parts) => {
                if !parts.iter().all(|p| match p {
                    Part::Text(_) => true,
                    Part::Expr(e) => range(*e).is_some(),
                }) {
                    return None;
                }
                Value::Text
            }
            _ => return None,
        };
        concrete.push(match &value {
            Value::Exact(v) => v.clone(),
            _ => V::Num(0.0), // Never read by an exact operation.
        });
        values.push(value);
    }
    Some(values)
}

/// A changing numeric token can replace a complete CSS number, never part of an identifier,
/// sign, exponent or another number. Only bounded, ordinary transform functions are admitted.
fn transform_template(name: &str, op: &Op, values: &[Value]) -> bool {
    let Op::Template(parts) = op else {
        return false;
    };
    let mut pattern = String::new();
    for p in parts {
        match p {
            Part::Text(s) => pattern.push_str(s),
            Part::Expr(e) => {
                let Some((a, b)) = values[*e as usize].range() else {
                    return false;
                };
                if a < -1e6 || b > 1e6 {
                    return false;
                }
                pattern.push('@');
            }
        }
    }
    let bytes = pattern.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b != b'@' {
            continue;
        }
        if i > 0 && !matches!(bytes[i - 1], b'(' | b',' | b' ' | b'\t' | b'\n') {
            return false;
        }
        let rest = &pattern[i + 1..];
        let tail = ["px", "deg", "rad", "turn", "grad", "%"]
            .into_iter()
            .find_map(|u| rest.strip_prefix(u))
            .unwrap_or(rest);
        if tail
            .as_bytes()
            .first()
            .is_some_and(|b| !matches!(b, b')' | b',' | b' ' | b'\t' | b'\n'))
        {
            return false;
        }
    }
    // No calc(), variables, perspective or arbitrary CSS functions. Their constraints need
    // their own rules; syntactically valid samples do not prove them for a whole interval.
    let allowed = [
        "translate",
        "translateX",
        "translateY",
        "translateZ",
        "translate3d",
        "rotate",
        "rotateX",
        "rotateY",
        "rotateZ",
        "scale",
        "scaleX",
        "scaleY",
        "scaleZ",
        "scale3d",
        "skew",
        "skewX",
        "skewY",
        "matrix",
    ];
    for word in pattern.split(|c: char| !c.is_ascii_alphabetic() && !c.is_ascii_digit()) {
        if word.is_empty()
            || word.bytes().all(|b| b.is_ascii_digit())
            || ["px", "deg", "rad", "grad", "turn"]
                .contains(&word.trim_start_matches(|c: char| c.is_ascii_digit()))
        {
            continue;
        }
        if !allowed.contains(&word) {
            return false;
        }
    }
    ["1", "-1"]
        .into_iter()
        .all(|number| crate::style::transform::accepts(name, &pattern.replace('@', number)))
}

pub(super) fn proves(t: &Template, first: Clock, last: Clock) -> bool {
    let Some(values) = evaluate(&t.program, Vec::new(), first, last) else {
        return false;
    };
    for g in &t.guards {
        if g.when
            .iter()
            .any(|(e, side)| values[*e as usize].truth() == Some(!side))
        {
            continue;
        }
        if !g
            .when
            .iter()
            .all(|(e, side)| values[*e as usize].truth() == Some(*side))
        {
            return false;
        }
        let v = &values[g.expr as usize];
        let holds = match &g.want {
            Want::Truthy(b) => v.truth() == Some(*b),
            Want::Eq(want) => matches!(v,Value::Exact(v) if v==want),
        };
        if !holds {
            return false;
        }
    }
    t.nodes.iter().all(|n| {
        n.style
            .iter()
            .all(|(k, p)| property(&t.program, &values, k, p))
    }) && t.instances.iter().all(|(_, table)| {
        table.rows.iter().all(|row| {
            if let Some(f) = &table.formula {
                let mut local = Vec::with_capacity(f.program.ops.len());
                for p in row {
                    let Prop::Const(v) = p else {
                        return false;
                    };
                    local.push(Value::Exact(v.clone()));
                }
                local.extend(f.imports.iter().map(|e| values[*e as usize].clone()));
                let Some(local) = evaluate(&f.program, local, first, last) else {
                    return false;
                };
                table
                    .fields
                    .iter()
                    .zip(&f.outputs)
                    .all(|(k, p)| property(&f.program, &local, k, p))
            } else {
                table
                    .fields
                    .iter()
                    .zip(row)
                    .all(|(k, p)| property(&t.program, &values, k, p))
            }
        })
    })
}

fn property(program: &Program, values: &[Value], name: &str, p: &Prop) -> bool {
    let Prop::Expr(e) = p else {
        return true;
    };
    let v = &values[*e as usize];
    if matches!(v, Value::Exact(_)) {
        return true;
    }
    match name {
        "opacity" => v.range().is_some_and(|(a, b)| a >= -1e6 && b <= 1e6),
        "scale" => {
            v.range().is_some_and(|(a, b)| a >= -1e6 && b <= 1e6)
                || transform_template(name, &program.ops[*e as usize], values)
        }
        "width" | "height" => v
            .range()
            .is_some_and(|(a, b)| a >= 0.0 && b <= 33_554_431.0),
        "left" | "top" | "right" | "bottom" => v
            .range()
            .is_some_and(|(a, b)| a >= -33_554_431.0 && b <= 33_554_431.0),
        "translate" | "rotate" | "transform" | "transformOrigin" => {
            transform_template(name, &program.ops[*e as usize], values)
        }
        _ => false,
    }
}
