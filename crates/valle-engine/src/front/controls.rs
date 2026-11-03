//! A scene's props and data: what `controls.props` and `controls.data` declare, read without
//! running the scene so tools can list them and their defaults, and the values bound to them from
//! outside, checked against their declarations. The root component takes both as its props.

use oxc::ast::ast::{
    ArrayExpressionElement, Expression, ObjectExpression, ObjectPropertyKind, UnaryOperator,
};
use oxc::span::GetSpan;
use serde_json::{Map, Number, Value};

use super::rewrite::{bare, key};
use crate::style::{color, css};

/// A prop `controls.props` declares, or data `controls.data` does.
#[derive(Clone, Debug, PartialEq)]
pub struct Prop {
    pub name: String,
    pub kind: Kind,
    pub default: Option<Value>,
    /// Whether a value must be bound to it, as it has no default.
    pub required: bool,
    /// What an editor shows for it.
    pub label: Option<String>,
}

/// What a prop takes.
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Number {
        min: Option<f64>,
        max: Option<f64>,
        step: Option<f64>,
    },
    String,
    Boolean,
    /// A CSS color, as text.
    Color,
    /// One of `values`, strings or numbers.
    Select(Vec<Value>),
    /// `{ x, y }`.
    Point,
    /// A CSS length, as text: `"24px"`.
    Length,
    /// A CSS angle, as text: `"15deg"`.
    Angle,
    /// `{ x, y, width, height }`.
    Rect,
    /// Any JSON value: what `controls.data` declares, such as a chart's rows.
    Json,
}

const KINDS: &str = "number, string, boolean, color, select, point, length, angle or rect";

/// The units of a CSS length.
const LENGTHS: [&str; 15] = [
    "px", "em", "rem", "%", "vw", "vh", "vmin", "vmax", "pt", "pc", "cm", "mm", "in", "ch", "ex",
];

/// Values bound to a scene's controls from outside: the command line's, or a Timeline clip's.
#[derive(Clone, Debug, Default)]
pub struct Inputs {
    pub props: Map<String, Value>,
    pub data: Map<String, Value>,
}

/// The props of `controls.props`, an object of `name: number({ … })` and the like.
pub fn props(object: &ObjectExpression<'_>) -> Result<Vec<Prop>, (u32, String)> {
    entries(object, "props", "prop", prop)
}

/// The data of `controls.data`, an object of `name: { default: [] }` and the like.
pub fn data(object: &ObjectExpression<'_>) -> Result<Vec<Prop>, (u32, String)> {
    entries(object, "data", "data", datum)
}

/// The entries of `controls.{section}`, each read by `read`.
fn entries(
    object: &ObjectExpression<'_>,
    section: &str,
    what: &str,
    read: fn(&str, &Expression<'_>) -> Result<Prop, String>,
) -> Result<Vec<Prop>, (u32, String)> {
    let literal = || literal(&format!("controls.{section}"));
    let mut out = Vec::new();
    for p in &object.properties {
        let ObjectPropertyKind::ObjectProperty(p) = p else {
            return Err((p.span().start, literal()));
        };
        let at = p.span.start;
        let name = key(p).ok_or_else(|| (at, literal()))?;
        let named = name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        if !named || name.len() > 64 {
            return Err((
                at,
                format!(
                    "{what} name {name:?} is not 1 to 64 ASCII letters, digits and underscores, \
                     starting with a letter or an underscore"
                ),
            ));
        }
        let entry = read(name, bare(&p.value)).map_err(|e| (at, format!("{what} {name}: {e}")))?;
        out.push(entry);
    }
    Ok(out)
}

/// The name `props` and `data` both declare: the root component takes both as its props.
pub fn clash(props: &[Prop], data: &[Prop]) -> Option<String> {
    let both = props
        .iter()
        .find(|p| data.iter().any(|d| d.name == p.name))?;
    Some(format!(
        "{} is both a prop and data: the root component takes both as its props, so each name \
         is declared once",
        both.name
    ))
}

fn literal(what: &str) -> String {
    format!("{what} must be an object literal: tools read it without running the scene")
}

/// `number({ default: 3, min: 1 })` and the like.
fn prop(name: &str, e: &Expression<'_>) -> Result<Prop, String> {
    let written =
        || format!("write its kind and options, such as number({{ default: 1 }}): {KINDS}");
    let Expression::CallExpression(call) = e else {
        return Err(written());
    };
    let Expression::Identifier(f) = &call.callee else {
        return Err(written());
    };
    let mut options = match call.arguments.as_slice() {
        [] => Map::new(),
        [a] => match a.as_expression().map(bare) {
            Some(Expression::ObjectExpression(o)) => object(o)?,
            _ => return Err(written()),
        },
        _ => return Err(written()),
    };
    let mut take = |k: &str| options.remove(k);
    let number = |k: &str, v: Option<Value>| match v {
        None => Ok(None),
        Some(Value::Number(n)) => Ok(n.as_f64()),
        Some(_) => Err(format!("{k} is a number")),
    };
    let kind = match f.name.as_str() {
        "number" => Kind::Number {
            min: number("min", take("min"))?,
            max: number("max", take("max"))?,
            step: number("step", take("step"))?,
        },
        "string" => Kind::String,
        "boolean" => Kind::Boolean,
        "color" => Kind::Color,
        "select" => match take("values") {
            Some(Value::Array(values))
                if !values.is_empty() && values.iter().all(|v| v.is_string() || v.is_number()) =>
            {
                Kind::Select(values)
            }
            _ => return Err("select() needs values, an array of strings and numbers".into()),
        },
        "point" => Kind::Point,
        "length" => Kind::Length,
        "angle" => Kind::Angle,
        "rect" => Kind::Rect,
        other => return Err(format!("{other} is not a kind of prop: {KINDS}")),
    };
    if let Kind::Number {
        min: Some(min),
        max: Some(max),
        ..
    } = kind
        && min > max
    {
        return Err(format!("its min, {min}, is over its max, {max}"));
    }
    if let Kind::Number { step: Some(s), .. } = kind
        && s <= 0.0
    {
        return Err(format!("its step, {s}, is not over 0"));
    }
    let default = take("default").map(|v| check(&kind, v)).transpose()?;
    common(name, kind, default, options, &format!("{}()", f.name))
}

/// `{ default: [] }`: data, any JSON.
fn datum(name: &str, e: &Expression<'_>) -> Result<Prop, String> {
    let Expression::ObjectExpression(o) = e else {
        return Err("write its options, such as { default: [] }".into());
    };
    let mut options = object(o)?;
    let default = options.remove("default");
    common(name, Kind::Json, default, options, "data")
}

/// The options props and data share, `required` and `label`, then the entry.
fn common(
    name: &str,
    kind: Kind,
    default: Option<Value>,
    mut options: Map<String, Value>,
    taker: &str,
) -> Result<Prop, String> {
    let required = match options.remove("required") {
        None => false,
        Some(Value::Bool(b)) => b,
        Some(_) => return Err("required is true or false".into()),
    };
    if required && default.is_some() {
        return Err("one with a default is never missing, so it is not required".into());
    }
    let label = match options.remove("label") {
        None => None,
        Some(Value::String(s)) => Some(s),
        Some(_) => return Err("label is a string".into()),
    };
    if let Some(k) = options.keys().next() {
        return Err(format!("{taker} takes no option {k}"));
    }
    Ok(Prop {
        name: name.to_string(),
        kind,
        default,
        required,
        label,
    })
}

/// `value` as `kind` takes it, or why it does not.
fn check(kind: &Kind, value: Value) -> Result<Value, String> {
    let shown = value.to_string();
    let fail = |what: &str| Err(format!("{shown} is not {what}"));
    let numbers = |v: &Value, fields: &[&str]| match v {
        Value::Object(o) => {
            o.len() == fields.len()
                && fields
                    .iter()
                    .all(|f| o.get(*f).is_some_and(|x| x.as_f64().is_some()))
        }
        _ => false,
    };
    match kind {
        Kind::Number { min, max, step } => {
            let Some(x) = value.as_f64() else {
                return fail("a number");
            };
            if let Some(min) = min
                && x < *min
            {
                return Err(format!("{shown} is under its min, {min}"));
            }
            if let Some(max) = max
                && x > *max
            {
                return Err(format!("{shown} is over its max, {max}"));
            }
            if let Some(step) = step {
                let steps = (x - min.unwrap_or(0.0)) / step;
                if (steps - steps.round()).abs() > 1e-9 {
                    return Err(format!("{shown} is not on its steps of {step}"));
                }
            }
        }
        Kind::String if !value.is_string() => return fail("a string"),
        Kind::Boolean if !value.is_boolean() => return fail("true or false"),
        Kind::Color => match value.as_str().map(color::parse) {
            Some(Ok(_)) => {}
            _ => return fail("a CSS color"),
        },
        Kind::Select(values) if !values.contains(&value) => {
            let list = values.iter().map(Value::to_string).collect::<Vec<_>>();
            return Err(format!("{shown} is not one of {}", list.join(", ")));
        }
        Kind::Point if !numbers(&value, &["x", "y"]) => return fail("a point, { x, y }"),
        Kind::Rect if !numbers(&value, &["x", "y", "width", "height"]) => {
            return fail("a rect, { x, y, width, height }");
        }
        Kind::Length => {
            let n = value.as_str().and_then(css::num);
            if !n
                .is_some_and(|n| LENGTHS.contains(&n.unit) || (n.unit.is_empty() && n.value == 0.0))
            {
                return fail("a CSS length, such as \"24px\"");
            }
        }
        Kind::Angle if value.as_str().and_then(css::angle).is_none() => {
            return fail("a CSS angle, such as \"15deg\"");
        }
        _ => {}
    }
    Ok(value)
}

/// What the root component takes from `props` and `data`: each bound value, or its default;
/// one missing is left out, unless it is required.
pub fn values(
    props: &[Prop],
    data: &[Prop],
    inputs: &Inputs,
) -> Result<Map<String, Value>, String> {
    let mut out = Map::new();
    bind(props, &inputs.props, "props", "prop", &mut out)?;
    bind(data, &inputs.data, "data", "data", &mut out)?;
    Ok(out)
}

/// The values `bound` to the entries of `controls.{section}`, into `out`.
fn bind(
    entries: &[Prop],
    bound: &Map<String, Value>,
    section: &str,
    what: &str,
    out: &mut Map<String, Value>,
) -> Result<(), String> {
    if let Some(k) = (bound.keys()).find(|k| !entries.iter().any(|p| p.name == **k)) {
        return Err(format!(
            "{section}: the scene declares no {what} {k} in controls.{section}"
        ));
    }
    for p in entries {
        let value = match bound.get(&p.name) {
            Some(v) => {
                Some(check(&p.kind, v.clone()).map_err(|e| format!("{section}.{}: {e}", p.name))?)
            }
            None => p.default.clone(),
        };
        match value {
            Some(v) => {
                out.insert(p.name.clone(), v);
            }
            None if p.required => {
                return Err(format!(
                    "{section}.{}: it is required, and nothing is bound to it",
                    p.name
                ));
            }
            None => {}
        }
    }
    Ok(())
}

/// An object literal of JSON values.
fn object(o: &ObjectExpression<'_>) -> Result<Map<String, Value>, String> {
    let mut out = Map::new();
    for p in &o.properties {
        let ObjectPropertyKind::ObjectProperty(p) = p else {
            return Err("spread is not supported: tools read it without running the scene".into());
        };
        let k = key(p).ok_or("each key is written out: tools read it without running the scene")?;
        out.insert(k.to_string(), json(bare(&p.value))?);
    }
    Ok(out)
}

/// A literal as JSON: numbers, strings, booleans, null, and arrays and objects of them.
fn json(e: &Expression<'_>) -> Result<Value, String> {
    // Whole numbers as integers, as JSON text reads them, so `0` shows as 0, not 0.0.
    let number = |x: f64| match x {
        _ if x.fract() == 0.0 && x.abs() < 9007199254740992.0 => Ok(Value::from(x as i64)),
        _ => (Number::from_f64(x).map(Value::Number))
            .ok_or_else(|| format!("{x} is not a finite number")),
    };
    Ok(match e {
        Expression::NumericLiteral(n) => number(n.value)?,
        Expression::UnaryExpression(u) if u.operator == UnaryOperator::UnaryNegation => {
            match bare(&u.argument) {
                Expression::NumericLiteral(n) => number(-n.value)?,
                _ => return Err(not_json()),
            }
        }
        Expression::StringLiteral(s) => Value::String(s.value.to_string()),
        Expression::TemplateLiteral(t) if t.expressions.is_empty() => Value::String(
            (t.quasis.iter())
                .map(|q| q.value.cooked.as_ref().map_or("", |c| c.as_str()))
                .collect(),
        ),
        Expression::BooleanLiteral(b) => Value::Bool(b.value),
        Expression::NullLiteral(_) => Value::Null,
        Expression::ArrayExpression(a) => Value::Array(
            (a.elements.iter())
                .map(|x| match x {
                    ArrayExpressionElement::SpreadElement(_)
                    | ArrayExpressionElement::Elision(_) => Err(not_json()),
                    x => json(bare(x.to_expression())),
                })
                .collect::<Result<_, _>>()?,
        ),
        Expression::ObjectExpression(o) => Value::Object(object(o)?),
        _ => return Err(not_json()),
    })
}

fn not_json() -> String {
    "its values are literals, such as numbers, strings, arrays and objects of them: tools read \
     them without running the scene"
        .into()
}
