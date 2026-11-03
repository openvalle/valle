//! Lowering: a traced variant, as `runtime.js` serializes it, into a frame template. Ops arrive by
//! name over operands; `make` turns one into the engine's expression, for tracing (`__host.op`) and
//! for the template alike.

use std::rc::Rc;

use serde::Deserialize;

use super::{Holds, Slot};
use crate::ease::Easing;
use crate::format::Format;
use crate::js::MathFn;
use crate::noise::Wiggle;
use crate::spring::Spring;
use crate::style::color::Mixing;
use crate::template::{
    Bin, Cmp, Error, Guard, Interp, Kind, Measure, Node, Op, Part, PathFn, Piece, Program, Prop,
    Stage, Table, Tag, Template, V, Want,
};

/// An operand of an op by name: an earlier expression, or literal text of a template.
pub enum Arg {
    Expr(u32),
    Text(Rc<str>),
}

/// The expression `name` over `args`.
pub fn make(name: &str, args: &[Arg]) -> Result<Op, Error> {
    let exprs = || -> Result<Vec<u32>, Error> {
        args.iter()
            .map(|a| match a {
                Arg::Expr(i) => Ok(*i),
                Arg::Text(_) => Err(format!("op {name} takes no text")),
            })
            .collect()
    };
    let fixed = |n: usize| -> Result<Vec<u32>, Error> {
        let a = exprs()?;
        if a.len() == n {
            Ok(a)
        } else {
            Err(format!("op {name} takes {n} operands, got {}", a.len()))
        }
    };
    if let Some(f) = name.strip_prefix("math|") {
        let f = MathFn::parse(f).ok_or_else(|| format!("no Math function {f}"))?;
        return Ok(Op::Math(f, exprs()?));
    }
    if let Some(f) = name.strip_prefix("path|") {
        let f = PathFn::parse(f).ok_or_else(|| format!("no path function {f}"))?;
        return Ok(Op::Path(f, fixed(f.operands())?));
    }
    if let Some(spec) = name.strip_prefix("bounds|") {
        let (path, measure) = match spec.split_once('|') {
            Some(("x", path)) => (path, Measure::X),
            Some(("y", path)) => (path, Measure::Y),
            Some(("width", path)) => (path, Measure::Width),
            Some(("height", path)) => (path, Measure::Height),
            _ => return Err(format!("bounds {spec}")),
        };
        fixed(0)?;
        return Ok(Op::Bounds(Rc::from(path), measure));
    }
    if let Some(spec) = name.strip_prefix("format|") {
        return Ok(Op::Format(fixed(1)?[0], Format::parse(spec)?));
    }
    if let Some(seed) = name.strip_prefix("noise|") {
        let seed = seed.parse().map_err(|_| format!("noise seed {seed}"))?;
        let args = exprs()?;
        if !(1..=2).contains(&args.len()) {
            return Err("noise takes one or two coordinates".into());
        }
        return Ok(Op::Noise(seed, args));
    }
    if let Some(spec) = name.strip_prefix("wiggle|") {
        return Ok(Op::Wiggle(fixed(1)?[0], Wiggle::parse(spec)?));
    }
    if let Some(spec) = name.strip_prefix("table|") {
        return Ok(Op::Table(fixed(1)?[0], Rc::new(Table::parse(spec)?)));
    }
    if let Some(spec) = name.strip_prefix("freeze|") {
        let span = spec
            .split_once('|')
            .map(|(a, b)| (a.parse::<f64>(), b.parse::<f64>()));
        return match span {
            Some((Ok(from), Ok(to))) if from.is_finite() && to.is_finite() && from < to => {
                Ok(Op::Freeze(fixed(1)?[0], [from, to]))
            }
            _ => Err(format!("freeze {spec}")),
        };
    }
    if let Some(spec) = name.strip_prefix("progress|") {
        return Ok(Op::Stage(fixed(1)?[0], Stage::parse(spec)?));
    }
    for (prefix, velocity) in [("spring|", false), ("springVelocity|", true)] {
        if let Some(spec) = name.strip_prefix(prefix) {
            let mut parts = spec.split('|').map(|p| (!p.is_empty()).then_some(p));
            let options = std::array::from_fn(|_| parts.next().flatten());
            let input = fixed(1)?[0];
            let spring = Spring::new(options)?;
            return Ok(Op::Spring {
                input,
                spring,
                velocity,
            });
        }
    }
    if let Some(spec) = name.strip_prefix("interpolate|") {
        let side = |s: &str| match s {
            "clamp" => Ok(false),
            "extend" => Ok(true),
            _ => Err(format!("interpolate extrapolation {s}")),
        };
        let mut spec = spec.splitn(5, '|');
        let mut part = || spec.next().ok_or("interpolate spec");
        let (left, right, space, hue) = (part()?, part()?, part()?, part()?);
        let easing = part()?;
        let mixing = match (space, hue) {
            ("", "") => None,
            ("", _) => return Err("interpolate hue needs a colorSpace".into()),
            (space, "") => Some(Mixing::parse(space, "shorter")?),
            (space, hue) => Some(Mixing::parse(space, hue)?),
        };
        let a = exprs()?;
        let n = a.len().saturating_sub(1) / 2;
        if n < 2 || a.len() != 1 + 2 * n {
            return Err(
                "interpolate takes an input and two ranges of one length, at least 2".into(),
            );
        }
        return Ok(Op::Interp(Rc::new(Interp {
            input: a[0],
            stops: (0..n).map(|i| (a[1 + i], a[1 + n + i])).collect(),
            extend: [side(left)?, side(right)?],
            easing: Easing::parse(easing)?,
            mixing,
        })));
    }
    let bin = |b: Bin| -> Result<Op, Error> {
        let a = fixed(2)?;
        Ok(Op::Bin(b, a[0], a[1]))
    };
    let cmp = |c: Cmp| -> Result<Op, Error> {
        let a = fixed(2)?;
        Ok(Op::Cmp(c, a[0], a[1]))
    };
    match name {
        "seconds" => fixed(0).map(|_| Op::Seconds),
        "progress" => fixed(0).map(|_| Op::Progress),
        "add" => bin(Bin::Add),
        "sub" => bin(Bin::Sub),
        "mul" => bin(Bin::Mul),
        "div" => bin(Bin::Div),
        "rem" => bin(Bin::Rem),
        "pow" => bin(Bin::Pow),
        "lt" => cmp(Cmp::Lt),
        "le" => cmp(Cmp::Le),
        "gt" => cmp(Cmp::Gt),
        "ge" => cmp(Cmp::Ge),
        "eq" => cmp(Cmp::Eq),
        "ne" => cmp(Cmp::Ne),
        "neg" => fixed(1).map(|a| Op::Neg(a[0])),
        "select" => fixed(3).map(|a| Op::Select(a[0], a[1], a[2])),
        "template" => Ok(Op::Template(
            args.iter()
                .map(|a| match a {
                    Arg::Expr(i) => Part::Expr(*i),
                    Arg::Text(t) => Part::Text(t.clone()),
                })
                .collect(),
        )),
        _ => Err(format!("unknown op {name}")),
    }
}

/// Where the runtime recorded something: a site the rewrite numbered, or a module's line taken
/// from the stack (`m3.js:12`).
#[derive(Deserialize)]
#[serde(untagged)]
pub enum At {
    Site(u32),
    Stack(String),
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Constant {
    Num(f64),
    Str(String),
    Bool(bool),
    /// -0, NaN and the infinities, which JSON has no numbers for.
    NonFinite {
        nf: String,
    },
}

impl Constant {
    fn value(self) -> Result<V, Error> {
        Ok(match self {
            Constant::Num(x) => V::Num(x),
            Constant::Str(s) => V::Str(s.into()),
            Constant::Bool(b) => V::Bool(b),
            Constant::NonFinite { nf } => V::Num(match &*nf {
                "-0" => -0.0,
                "NaN" => f64::NAN,
                "Infinity" => f64::INFINITY,
                "-Infinity" => f64::NEG_INFINITY,
                _ => return Err(format!("constant {nf}")),
            }),
        })
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Ref {
    Expr { e: u32 },
    Const { c: Constant },
    Text { text: String },
}

#[derive(Deserialize)]
struct Traced {
    ops: Vec<(String, Vec<Ref>)>,
    nodes: Vec<TracedNode>,
    roots: Vec<u32>,
    guards: Vec<TracedGuard>,
}

#[derive(Deserialize)]
struct TracedNode {
    #[serde(rename = "type")]
    kind: String,
    path: Option<String>,
    classes: Vec<String>,
    style: Vec<(String, Ref)>,
    children: Vec<u32>,
    site: Option<u32>,
    content: Option<Vec<TracedPiece>>,
    mask: Option<TracedMask>,
    src: Option<String>,
    tag: Option<String>,
    attributes: Option<Vec<(String, Ref)>>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum TracedPiece {
    Span { span: u32 },
    Text { text: Ref },
}

#[derive(Deserialize)]
struct TracedMask {
    source: u32,
    luminance: bool,
    invert: bool,
}

#[derive(Deserialize)]
struct TracedGuard {
    e: u32,
    truthy: Option<bool>,
    eq: Option<Constant>,
    when: Vec<(u32, bool)>,
    kind: String,
    at: At,
}

/// The template of a variant `runtime.js` serialized; `place` names where a site or stack line is
/// in the source, and `resources` are the scene's.
pub fn variant(
    json: &str,
    place: &dyn Fn(&At) -> Rc<str>,
    resources: &[Slot],
) -> Result<Template, Error> {
    let traced: Traced =
        serde_json::from_str(json).map_err(|e| format!("a traced variant: {e}"))?;
    let mut program = Program::default();
    // Program index of each traced op; ops come in dependency order.
    let mut ids: Vec<u32> = Vec::with_capacity(traced.ops.len());
    let expr = |ids: &[u32], e: u32| -> Result<u32, Error> {
        ids.get(e as usize)
            .copied()
            .ok_or_else(|| format!("op {e} is used before it is defined"))
    };
    for (name, refs) in traced.ops {
        let mut args = Vec::with_capacity(refs.len());
        for r in refs {
            args.push(match r {
                Ref::Expr { e } => Arg::Expr(expr(&ids, e)?),
                Ref::Const { c } => {
                    program.ops.push(Op::Const(c.value()?));
                    Arg::Expr(program.ops.len() as u32 - 1)
                }
                Ref::Text { text } => Arg::Text(text.into()),
            });
        }
        program.ops.push(make(&name, &args)?);
        ids.push(program.ops.len() as u32 - 1);
    }
    let prop = |r: Ref| -> Result<Prop, Error> {
        match r {
            Ref::Expr { e } => Ok(Prop::Expr(expr(&ids, e)?)),
            Ref::Const { c } => Ok(Prop::Const(c.value()?)),
            Ref::Text { .. } => Err("a prop cannot be template text".into()),
        }
    };
    let count = traced.nodes.len() as u32;
    let node_id = |i: u32| {
        (i < count)
            .then_some(i)
            .ok_or_else(|| format!("node {i} does not exist"))
    };
    let content = |pieces: Option<Vec<TracedPiece>>| -> Result<Vec<Piece>, Error> {
        (pieces.ok_or("a text node without content")?.into_iter())
            .map(|p| match p {
                TracedPiece::Span { span } => Ok(Piece::Span(node_id(span)?)),
                TracedPiece::Text { text } => Ok(Piece::Text(prop(text)?)),
            })
            .collect()
    };
    let mut nodes = Vec::with_capacity(traced.nodes.len());
    for n in traced.nodes {
        let kind = match &*n.kind {
            "view" => Kind::View,
            "text" => Kind::Text {
                content: content(n.content)?,
            },
            "span" => Kind::Span {
                content: content(n.content)?,
            },
            "mask" => {
                let m = n.mask.ok_or("a mask node without a source")?;
                Kind::Mask {
                    source: node_id(m.source)?,
                    luminance: m.luminance,
                    invert: m.invert,
                }
            }
            "image" => {
                let name = n.src.as_deref().ok_or("an image node without a source")?;
                let at = || n.site.map_or("<Image>".into(), |s| place(&At::Site(s)));
                let resource =
                    (resources.iter().position(|r| r.name == name)).ok_or_else(|| {
                        format!(
                            "{}: <Image> src resource://{name}: the scene declares no resource \
                         {name} in controls.resources",
                            at()
                        )
                    })?;
                if resources[resource].kind != Holds::Image {
                    return Err(format!(
                        "{}: <Image> src resource://{name}: {name} is a font, not an image",
                        at()
                    ));
                }
                Kind::Image {
                    resource: resource as u32,
                }
            }
            "svg" => {
                let tag = n.tag.as_deref().ok_or("an svg node without a tag")?;
                Kind::Svg {
                    tag: Tag::by_name(tag).ok_or_else(|| format!("svg tag {tag}"))?,
                    attributes: (n.attributes.ok_or("an svg node without attributes")?)
                        .into_iter()
                        .map(|(k, v)| Ok((Rc::from(k), prop(v)?)))
                        .collect::<Result<_, Error>>()?,
                }
            }
            other => return Err(format!("node type {other}")),
        };
        nodes.push(Node {
            kind,
            path: n.path.map(Rc::from),
            classes: n.classes.into_iter().map(Rc::from).collect(),
            style: n
                .style
                .into_iter()
                .map(|(k, v)| Ok((Rc::from(k), prop(v)?)))
                .collect::<Result<_, Error>>()?,
            children: n
                .children
                .into_iter()
                .map(node_id)
                .collect::<Result<_, _>>()?,
            site: n.site.map(|s| place(&At::Site(s))),
        });
    }
    let roots = traced
        .roots
        .into_iter()
        .map(node_id)
        .collect::<Result<_, _>>()?;
    let mut guards = Vec::with_capacity(traced.guards.len());
    for g in traced.guards {
        let want = match (g.truthy, g.eq) {
            (Some(t), None) => Want::Truthy(t),
            (None, Some(v)) => Want::Eq(v.value()?),
            _ => return Err("a guard wants truthiness or a value".into()),
        };
        guards.push(Guard {
            expr: expr(&ids, g.e)?,
            want,
            when: g
                .when
                .into_iter()
                .map(|(c, side)| Ok((expr(&ids, c)?, side)))
                .collect::<Result<_, Error>>()?,
            kind: g.kind.into(),
            at: place(&g.at),
        });
    }
    Ok(Template {
        program,
        nodes,
        roots,
        guards,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ops_are_made_by_name() {
        let e = |i| Arg::Expr(i);
        assert_eq!(make("add", &[e(0), e(1)]), Ok(Op::Bin(Bin::Add, 0, 1)));
        assert_eq!(make("select", &[e(0), e(1), e(2)]), Ok(Op::Select(0, 1, 2)));
        assert_eq!(
            make("math|sin", &[e(3)]),
            Ok(Op::Math(MathFn::Sin, vec![3]))
        );
        let Ok(Op::Interp(i)) = make(
            "interpolate|clamp|extend|||ease",
            &[e(0), e(1), e(2), e(3), e(4)],
        ) else {
            panic!("an interpolation")
        };
        assert_eq!(
            (i.input, &i.stops[..], i.extend, i.easing.at(1.0)),
            (0, &[(1, 3), (2, 4)][..], [false, true], 1.0)
        );
        assert!(make("interpolate|clamp|clamp|||linear", &[e(0), e(1), e(2)]).is_err());
        assert!(
            make("add", &[e(0)])
                .unwrap_err()
                .contains("takes 2 operands")
        );
        assert!(make("math|random", &[]).is_err());
        assert!(make("add", &[e(0), Arg::Text("x".into())]).is_err());
    }

    #[test]
    fn variants_lower_to_templates() {
        let json = r#"{
            "ops": [["seconds", []], ["mul", [{"e": 0}, {"c": 2}]], ["gt", [{"e": 1}, {"c": 1}]],
                    ["template", [{"text": "t="}, {"e": 1}]]],
            "nodes": [
                {"type": "view", "path": "a", "classes": ["flex"], "style": [["opacity", {"e": 1}],
                 ["width", {"c": {"nf": "Infinity"}}]], "children": [1], "site": 4},
                {"type": "text", "path": null, "classes": [], "style": [], "children": [],
                 "site": null, "content": [{"text": {"e": 3}}]}
            ],
            "roots": [0],
            "guards": [{"e": 2, "truthy": false, "when": [], "kind": "if", "at": 7},
                       {"e": 0, "eq": {"nf": "NaN"}, "when": [[2, true]], "kind": "escape",
                        "at": "m0.js:3"}]
        }"#;
        let place = |at: &At| -> Rc<str> {
            match at {
                At::Site(s) => format!("site {s}").into(),
                At::Stack(s) => s.as_str().into(),
            }
        };
        let t = variant(json, &place, &[]).unwrap();
        // seconds, 2, mul, 1, gt, template
        assert_eq!(t.program.ops.len(), 6);
        assert_eq!(t.program.ops[2], Op::Bin(Bin::Mul, 0, 1));
        assert_eq!(t.program.ops[4], Op::Cmp(Cmp::Gt, 2, 3));
        assert_eq!(t.nodes[0].style[0], ("opacity".into(), Prop::Expr(2)));
        assert_eq!(
            t.nodes[0].style[1],
            ("width".into(), Prop::Const(V::Num(f64::INFINITY)))
        );
        assert_eq!(t.nodes[0].site.as_deref(), Some("site 4"));
        assert_eq!(
            t.nodes[1].kind,
            Kind::Text {
                content: vec![Piece::Text(Prop::Expr(5))]
            }
        );
        assert_eq!(t.guards[0].expr, 4);
        assert_eq!(&*t.guards[1].at, "m0.js:3");
        assert_eq!(t.guards[1].when, [(4, true)]);
        assert!(variant(r#"{"ops": [["add", [{"e": 5}, {"c": 1}]]], "nodes": [], "roots": [], "guards": []}"#, &place, &[]).is_err());
    }
}
