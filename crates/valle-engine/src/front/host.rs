//! Functions exposed by the Rust host to the tracing runtime.
use std::cell::RefCell;
use std::rc::Rc;

use rquickjs::{Array, Ctx, Exception, Function, Object, Value};

use super::lower::Arg;
use super::{Typeset, lower};

use crate::eval::{self, Clock};
use crate::style::color;
use crate::template::{Clips, Error, V};

const NO_FONTS: &str = "it lays text out with a scene's fonts, which this has not";

/// The units `<Text split>` splits `text` into, by `split`: each unit's start and end one after
/// another, in UTF-16 code units as JavaScript indexes strings. `char` gives grapheme clusters, but white space;
/// `word` runs of characters between white space; `line` the lines between line feeds, but empty
/// ones.
fn units(text: &str, split: &str) -> Vec<usize> {
    use unicode_segmentation::UnicodeSegmentation;
    // Each char boundary's UTF-16 index, by its byte index.
    let mut index = vec![0; text.len() + 1];
    for (i, c) in text.char_indices() {
        index[i + c.len_utf8()] = index[i] + c.len_utf16();
    }
    let utf16 = |byte: usize| index[byte];
    let pieces: Vec<(usize, &str)> = match split {
        "char" => text.grapheme_indices(true).collect(),
        "word" => {
            let mut out = Vec::new();
            let mut start = None;
            for (i, c) in text.char_indices().chain([(text.len(), ' ')]) {
                match (c.is_whitespace(), start) {
                    (false, None) => start = Some(i),
                    (true, Some(s)) => {
                        out.push((s, &text[s..i]));
                        start = None;
                    }
                    _ => {}
                }
            }
            out
        }
        _ => {
            let mut at = 0;
            (text.split('\n'))
                .map(|line| {
                    let piece = (at, line);
                    at += line.len() + 1;
                    piece
                })
                .collect()
        }
    };
    (pieces.into_iter())
        .filter(|(_, p)| !p.trim().is_empty())
        .flat_map(|(i, p)| [utf16(i), utf16(i + p.len())])
        .collect()
}

/// `__host.op(name, args)`: one op's value over concrete operands, through the evaluator frames
/// use, so tracing sees the values frames compute; `__host.svg`, the SVG tables as JSON;
/// `__host.random(seed, count, low, high)`, checked by the runtime; `__host.units(text, split)`,
/// the units of a split text; and `__host.measure(text, style)` and `__host.outline(text, style,
/// x, y, align)`, what `measureText` and `textOutline` give.
pub(super) fn install_host<'js>(
    ctx: &Ctx<'js>,
    typeset: Rc<RefCell<Option<Box<dyn Typeset>>>>,
    clips: Rc<Clips>,
) -> rquickjs::Result<()> {
    let host = Object::new(ctx.clone())?;
    let set = typeset.clone();
    host.set(
        "measure",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, text: String, style: String| {
                let measured = match &*set.borrow() {
                    Some(t) => t.measure(&text, &style).map(Vec::from),
                    None => Err(NO_FONTS.into()),
                };
                measured.map_err(|e| Exception::throw_type(&ctx, &format!("measureText: {e}")))
            },
        )?,
    )?;
    host.set(
        "outline",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, text: String, style: String, x: f64, y: f64, align: String| {
                let outlined = match &*typeset.borrow() {
                    Some(t) => t.outline(&text, &style, [x, y], &align),
                    None => Err(NO_FONTS.into()),
                };
                outlined.map_err(|e| Exception::throw_type(&ctx, &format!("textOutline: {e}")))
            },
        )?,
    )?;
    host.set(
        "op",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, name: String, args: Array<'js>| {
                op(&ctx, &name, &args, &clips).map_err(|e| Exception::throw_type(&ctx, &e))
            },
        )?,
    )?;
    host.set("svg", crate::svg::tables())?;
    host.set(
        "units",
        Function::new(ctx.clone(), |text: String, split: String| {
            units(&text, &split)
        })?,
    )?;
    // A color as d3 reads one: sRGB written as CSS had it before color spaces, as 0–255 channels
    // and an opacity; one with no opacity has no channels either, NaN.
    host.set(
        "color",
        Function::new(ctx.clone(), |text: String| {
            let text = text.trim().to_ascii_lowercase();
            let c = color::legacy(&text)
                .then(|| color::wide(&text).ok())
                .flatten()?;
            let channel = |x: f64| {
                if c[3] <= 0.0 { f64::NAN } else { x * 255.0 }
            };
            Some(vec![channel(c[0]), channel(c[1]), channel(c[2]), c[3]])
        })?,
    )?;
    host.set(
        "random",
        Function::new(ctx.clone(), |seed: f64, count: f64, low: f64, high: f64| {
            crate::noise::random(seed as u64, count as usize, [low, high])
        })?,
    )?;
    ctx.globals().set("__host", host)
}

fn op<'js>(
    ctx: &Ctx<'js>,
    name: &str,
    args: &Array<'js>,
    clips: &Clips,
) -> Result<Value<'js>, Error> {
    let mut values = Vec::new();
    let mut operands = Vec::new();
    for a in args.iter::<Value>() {
        let a = a.map_err(|e| e.to_string())?;
        let v = if let Some(x) = a.as_number() {
            V::Num(x)
        } else if let Some(b) = a.as_bool() {
            V::Bool(b)
        } else if let Some(s) = a.as_string() {
            V::Str(s.to_string().map_err(|e| e.to_string())?.into())
        } else if let Some(o) = a.as_object() {
            let text: String = o.get("text").map_err(|_| format!("op {name}: an object"))?;
            operands.push(Arg::Text(text.into()));
            continue;
        } else {
            return Err(format!("op {name}: cannot take {}", a.type_of()));
        };
        operands.push(Arg::Expr(values.len() as u32));
        values.push(v);
    }
    let op = lower::make(name, &operands, clips)?;
    let clock = Clock {
        seconds: 0.0,
        progress: 0.0,
    };
    let value = match eval::value(&op, &values, clock)? {
        V::Num(x) => Value::new_number(ctx.clone(), x),
        V::Bool(b) => Value::new_bool(ctx.clone(), b),
        V::Str(s) => rquickjs::String::from_str(ctx.clone(), &s)
            .map_err(|e| e.to_string())?
            .into_value(),
    };
    Ok(value)
}
