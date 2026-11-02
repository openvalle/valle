//! The utilities, as Tailwind's `utilities.ts` registers them (MIT, see THIRD_PARTY.md): each
//! makes CSS of the candidates that are its own, and nothing of the rest. Families come over one
//! at a time; a class of one that has not is not supported yet.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use super::{CHILDREN, Candidate, Modifier, Node, Value};
use super::{dimension, positive_integer, quarter, spaced, theme, types};

/// What a utility makes of a candidate: its CSS, or nothing when it is not the utility's.
type Compile = Box<dyn Fn(&Candidate) -> Option<Vec<Node>> + Send + Sync>;

/// What a functional utility makes of its value, which may say what kind it is.
type Handle = Arc<dyn Fn(&str, Option<&str>) -> Option<Vec<Node>> + Send + Sync>;

/// The CSS value of a named value, and of the fraction it is part of.
type Named = fn(&str, Option<&str>) -> Option<String>;

struct Utility {
    functional: bool,
    compile: Compile,
}

static UTILITIES: LazyLock<HashMap<String, Vec<Utility>>> = LazyLock::new(|| {
    let mut r = Registry::default();
    register(&mut r);
    r.0
});

/// Whether a static or a functional utility is named `name`.
pub(super) fn has(name: &str, functional: bool) -> bool {
    (UTILITIES.get(name)).is_some_and(|us| us.iter().any(|u| u.functional == functional))
}

/// The CSS each utility of a candidate makes of it, as Tailwind's `compileBaseUtility`.
pub(super) fn compile(c: &Candidate) -> Vec<Vec<Node>> {
    let made: Vec<Vec<Node>> = match c {
        Candidate::Arbitrary {
            property,
            value,
            modifier,
        } => {
            let value = match modifier {
                Some(m) => as_color(value, Some(m)),
                None => Some(value.clone()),
            };
            value.map(|v| vec![decl(property, v)]).into_iter().collect()
        }
        Candidate::Static(name) | Candidate::Functional { root: name, .. } => {
            let functional = matches!(c, Candidate::Functional { .. });
            (UTILITIES.get(*name).into_iter().flatten())
                .filter(|u| u.functional == functional)
                .filter_map(|u| (u.compile)(c))
                .collect()
        }
    };
    made.into_iter()
        .map(|nodes| nodes.into_iter().map(functions).collect())
        .collect()
}

/// `node` with Tailwind's CSS functions in its values worked out: `--spacing(4)`.
fn functions(node: Node) -> Node {
    match node {
        Node::Declaration(p, v) if v.contains("--spacing(") => {
            let mut out = String::new();
            let mut rest = &v[..];
            while let Some(i) = rest.find("--spacing(") {
                out.push_str(&rest[..i]);
                rest = &rest[i + "--spacing(".len()..];
                let end = rest.find(')').unwrap_or(rest.len());
                let n = &rest[..end];
                let multiplier = theme().resolve(None, &["--spacing"]).unwrap_or_default();
                match dimension(n) {
                    Some((0.0, _)) => out.push_str("0px"),
                    Some((1.0, _)) => out.push_str(&multiplier),
                    _ => out.push_str(&format!("calc({multiplier} * {n})")),
                }
                rest = rest.get(end + 1..).unwrap_or_default();
            }
            out.push_str(rest);
            Node::Declaration(p, out)
        }
        Node::Rule(s, nodes) => Node::Rule(s, nodes.into_iter().map(functions).collect()),
        Node::At(s, nodes) => Node::At(s, nodes.into_iter().map(functions).collect()),
        n => n,
    }
}

fn decl(property: &str, value: impl Into<String>) -> Node {
    Node::Declaration(property.into(), value.into())
}

/// A color at the opacity a modifier gives, as Tailwind's `asColor`: `/50`, `/[0.5]`.
fn as_color(value: &str, modifier: Option<&Modifier>) -> Option<String> {
    let alpha = match modifier {
        None => return Some(value.into()),
        Some(Modifier::Arbitrary(a)) => a.clone(),
        Some(Modifier::Named(m)) => match theme().resolve(Some(m), &["--opacity"]) {
            Some(a) => a,
            None if quarter(m) => format!("{m}%"),
            None => return None,
        },
    };
    Some(with_alpha(value, &alpha))
}

/// As Tailwind's `withAlpha`.
fn with_alpha(value: &str, alpha: &str) -> String {
    let alpha = match alpha.trim().parse::<f64>() {
        Ok(n) if !alpha.trim().is_empty() => format!("{}%", crate::js::number_to_string(n * 100.0)),
        _ => alpha.to_string(),
    };
    if alpha == "100%" {
        return value.into();
    }
    format!("color-mix(in oklab, {value} {alpha}, transparent)")
}

#[derive(Default)]
struct Registry(HashMap<String, Vec<Utility>>);

/// What a functional utility makes of a candidate without a value.
#[derive(Clone, Copy, Default)]
enum Bare {
    /// The theme's variable of the utility's first namespace, if it has one.
    #[default]
    Theme,
    Value(&'static str),
    Nothing,
}

/// A functional utility, as Tailwind's `functionalUtility` describes one.
#[derive(Default)]
struct Desc {
    negative: bool,
    fractions: bool,
    keys: &'static [&'static str],
    bare: Bare,
    /// Values that stand for themselves: `auto` of `z-auto`.
    statics: Vec<(&'static str, Vec<Node>)>,
    /// The CSS value of a named value the theme has not, and of its negative.
    value: Option<Named>,
    negative_value: Option<Named>,
    handle: Option<Handle>,
}

impl Registry {
    fn add(&mut self, name: String, functional: bool, compile: Compile) {
        let utility = Utility {
            functional,
            compile,
        };
        self.0.entry(name).or_default().push(utility);
    }

    /// A static utility: these declarations.
    fn fixed(&mut self, name: impl Into<String>, declarations: &[(&str, &str)]) {
        let nodes: Vec<Node> = declarations.iter().map(|(p, v)| decl(p, *v)).collect();
        self.nodes(name, nodes);
    }

    fn nodes(&mut self, name: impl Into<String>, nodes: Vec<Node>) {
        self.add(name.into(), false, Box::new(move |_| Some(nodes.clone())));
    }

    /// A functional utility, and its negative when it has one, as Tailwind's
    /// `functionalUtility`.
    fn functional(&mut self, root: &str, desc: Desc) {
        let desc = Arc::new(desc);
        if desc.negative {
            self.add(format!("-{root}"), true, made(desc.clone(), true));
        }
        self.add(root.into(), true, made(desc, false));
    }

    /// A utility of the spacing scale, as Tailwind's `spacingUtility`: `p-4`, `p-px`, and
    /// `-m-4` when it takes negatives.
    fn spacing(
        &mut self,
        name: &str,
        keys: &'static [&'static str],
        handle: Handle,
        negative: bool,
        fractions: bool,
    ) {
        if negative {
            let h = handle.clone();
            self.add(
                format!("-{name}-px"),
                false,
                Box::new(move |_| h("-1px", None)),
            );
        }
        let h = handle.clone();
        self.add(
            format!("{name}-px"),
            false,
            Box::new(move |_| h("1px", None)),
        );

        self.functional(
            name,
            Desc {
                negative,
                fractions,
                keys,
                bare: Bare::Nothing,
                value: Some(|v, _| multiplier(v, "")),
                negative_value: Some(|v, _| multiplier(v, "-")),
                handle: Some(handle),
                ..Desc::default()
            },
        );
    }
}

/// `--spacing(4)` for a value of the spacing scale, `--spacing(-4)` negative.
fn multiplier(v: &str, sign: &str) -> Option<String> {
    theme().resolve(None, &["--spacing"])?;
    quarter(v).then(|| format!("--spacing({sign}{v})"))
}

/// What a functional utility makes of its candidates, the negative's or the positive's, as
/// Tailwind's `handleFunctionalUtility`.
fn made(d: Arc<Desc>, negative: bool) -> Compile {
    Box::new(move |c| {
        let Candidate::Functional {
            value, modifier, ..
        } = c
        else {
            return None;
        };
        let handle = d.handle.as_ref()?;
        let mut kind = None;
        let value = match value {
            None => {
                if modifier.is_some() {
                    return None;
                }
                match d.bare {
                    Bare::Theme => theme().resolve(None, d.keys)?,
                    Bare::Value(v) => v.to_string(),
                    Bare::Nothing => return None,
                }
            }
            Some(Value::Arbitrary { kind: k, value }) => {
                if modifier.is_some() {
                    return None;
                }
                kind = k.as_deref();
                value.clone()
            }
            Some(Value::Named { value, fraction }) => {
                let fraction = fraction.as_deref();
                let mut v = theme().resolve(Some(fraction.unwrap_or(value)), d.keys);
                if v.is_none()
                    && d.fractions
                    && let Some(f) = fraction
                {
                    let (l, r) = f.split_once('/')?;
                    if !positive_integer(l) || !positive_integer(r) {
                        return None;
                    }
                    v = Some(format!("calc({l} / {r} * 100%)"));
                }
                if v.is_none()
                    && negative
                    && let Some(f) = d.negative_value
                {
                    let n = f(value, fraction);
                    if !n.as_deref().is_some_and(|n| n.contains('/')) && modifier.is_some() {
                        return None;
                    }
                    if let Some(n) = n {
                        return handle(&n, None);
                    }
                }
                if v.is_none()
                    && let Some(f) = d.value
                {
                    v = f(value, fraction);
                    if !v.as_deref().is_some_and(|v| v.contains('/')) && modifier.is_some() {
                        return None;
                    }
                }
                if v.is_none() && !negative && modifier.is_none() {
                    let fixed = d.statics.iter().find(|(k, _)| k == value);
                    if let Some((_, nodes)) = fixed {
                        return Some(nodes.clone());
                    }
                }
                v?
            }
        };
        let value = match negative {
            true => spaced(&format!("calc({value} * -1)")),
            false => value,
        };
        handle(&value, kind)
    })
}

/// A whole number at least 0 as itself, as Tailwind's `isPositiveInteger`.
fn integer(v: &str, _: Option<&str>) -> Option<String> {
    positive_integer(v).then(|| v.to_string())
}

/// A handle that sets `property` to the value.
fn one(property: &'static str) -> Handle {
    Arc::new(move |v, _| Some(vec![decl(property, v)]))
}

/// The families come in Tailwind's order, which is the order of its utilities of a root.
fn register(r: &mut Registry) {
    r.fixed(
        "sr-only",
        &[
            ("position", "absolute"),
            ("width", "1px"),
            ("height", "1px"),
            ("padding", "0"),
            ("margin", "-1px"),
            ("overflow", "hidden"),
            ("clip-path", "inset(50%)"),
            ("white-space", "nowrap"),
            ("border-width", "0"),
        ],
    );
    r.fixed(
        "not-sr-only",
        &[
            ("position", "static"),
            ("width", "auto"),
            ("height", "auto"),
            ("padding", "0"),
            ("margin", "0"),
            ("overflow", "visible"),
            ("clip-path", "none"),
            ("white-space", "normal"),
        ],
    );
    r.fixed("visible", &[("visibility", "visible")]);
    r.fixed("invisible", &[("visibility", "hidden")]);
    r.fixed("collapse", &[("visibility", "collapse")]);
    for position in ["static", "fixed", "absolute", "relative", "sticky"] {
        r.fixed(position, &[("position", position)]);
    }
    for (name, property) in [
        ("inset", "inset"),
        ("inset-x", "inset-inline"),
        ("inset-y", "inset-block"),
        ("inset-s", "inset-inline-start"),
        ("inset-e", "inset-inline-end"),
        ("inset-bs", "inset-block-start"),
        ("inset-be", "inset-block-end"),
        ("top", "top"),
        ("right", "right"),
        ("bottom", "bottom"),
        ("left", "left"),
    ] {
        r.fixed(format!("{name}-auto"), &[(property, "auto")]);
        r.fixed(format!("{name}-full"), &[(property, "100%")]);
        r.fixed(format!("-{name}-full"), &[(property, "-100%")]);
        r.spacing(name, &["--inset", "--spacing"], one(property), true, true);
    }
    r.functional(
        "z",
        Desc {
            negative: true,
            keys: &["--z-index"],
            value: Some(integer),
            statics: vec![("auto", vec![decl("z-index", "auto")])],
            handle: Some(one("z-index")),
            ..Desc::default()
        },
    );
    r.functional(
        "order",
        Desc {
            negative: true,
            keys: &["--order"],
            value: Some(integer),
            statics: vec![
                ("first", vec![decl("order", "-9999")]),
                ("last", vec![decl("order", "9999")]),
            ],
            handle: Some(one("order")),
            ..Desc::default()
        },
    );
    lines(r, "col", "grid-column");
    lines(r, "row", "grid-row");
    for value in ["start", "end", "right", "left", "none"] {
        let float = match value {
            "start" | "end" => format!("inline-{value}"),
            _ => value.to_string(),
        };
        r.fixed(format!("float-{value}"), &[("float", float.as_str())]);
    }
    for value in ["start", "end", "right", "left", "both", "none"] {
        let clear = match value {
            "start" | "end" => format!("inline-{value}"),
            _ => value.to_string(),
        };
        r.fixed(format!("clear-{value}"), &[("clear", clear.as_str())]);
    }
    for (name, property) in [
        ("m", "margin"),
        ("mx", "margin-inline"),
        ("my", "margin-block"),
        ("ms", "margin-inline-start"),
        ("me", "margin-inline-end"),
        ("mbs", "margin-block-start"),
        ("mbe", "margin-block-end"),
        ("mt", "margin-top"),
        ("mr", "margin-right"),
        ("mb", "margin-bottom"),
        ("ml", "margin-left"),
    ] {
        r.fixed(format!("{name}-auto"), &[(property, "auto")]);
        r.spacing(name, &["--margin", "--spacing"], one(property), true, false);
    }
    r.fixed("box-border", &[("box-sizing", "border-box")]);
    r.fixed("box-content", &[("box-sizing", "content-box")]);
    for (name, display) in [
        ("block", "block"),
        ("inline-block", "inline-block"),
        ("inline", "inline"),
        ("hidden", "none"),
        ("inline-flex", "inline-flex"),
        ("table", "table"),
        ("inline-table", "inline-table"),
        ("table-caption", "table-caption"),
        ("table-cell", "table-cell"),
        ("table-column", "table-column"),
        ("table-column-group", "table-column-group"),
        ("table-footer-group", "table-footer-group"),
        ("table-header-group", "table-header-group"),
        ("table-row-group", "table-row-group"),
        ("table-row", "table-row"),
        ("flow-root", "flow-root"),
        ("flex", "flex"),
        ("grid", "grid"),
        ("inline-grid", "inline-grid"),
        ("contents", "contents"),
        ("list-item", "list-item"),
    ] {
        r.fixed(name, &[("display", display)]);
    }
    r.fixed("field-sizing-content", &[("field-sizing", "content")]);
    r.fixed("field-sizing-fixed", &[("field-sizing", "fixed")]);
    r.functional(
        "aspect",
        Desc {
            keys: &["--aspect"],
            value: Some(|_, fraction| {
                let (l, r) = fraction?.split_once('/')?;
                (quarter(l) && quarter(r)).then(|| format!("{l}/{r}"))
            }),
            statics: vec![
                ("auto", vec![decl("aspect-ratio", "auto")]),
                ("square", vec![decl("aspect-ratio", "1 / 1")]),
            ],
            handle: Some(one("aspect-ratio")),
            ..Desc::default()
        },
    );
    sizes(r);
    r.fixed("flex-auto", &[("flex", "auto")]);
    r.fixed("flex-initial", &[("flex", "0 auto")]);
    r.fixed("flex-none", &[("flex", "none")]);
    r.add(
        "flex".into(),
        true,
        Box::new(|c| {
            let Candidate::Functional {
                value: Some(value),
                modifier,
                ..
            } = c
            else {
                return None;
            };
            let flex = match value {
                Value::Arbitrary { value, .. } if modifier.is_none() => value.clone(),
                Value::Named {
                    fraction: Some(f), ..
                } => {
                    let (l, r) = f.split_once('/')?;
                    if !positive_integer(l) || !positive_integer(r) {
                        return None;
                    }
                    format!("calc({f} * 100%)")
                }
                Value::Named { value, .. } if positive_integer(value) && modifier.is_none() => {
                    value.clone()
                }
                _ => return None,
            };
            Some(vec![decl("flex", flex)])
        }),
    );
    for (name, property) in [("shrink", "flex-shrink"), ("grow", "flex-grow")] {
        r.functional(
            name,
            Desc {
                bare: Bare::Value("1"),
                value: Some(integer),
                handle: Some(one(property)),
                ..Desc::default()
            },
        );
    }
    r.fixed("basis-auto", &[("flex-basis", "auto")]);
    r.fixed("basis-full", &[("flex-basis", "100%")]);
    let keys = &["--flex-basis", "--spacing", "--container"];
    r.spacing("basis", keys, one("flex-basis"), false, true);
    r.fixed("table-auto", &[("table-layout", "auto")]);
    r.fixed("table-fixed", &[("table-layout", "fixed")]);
    r.fixed("caption-top", &[("caption-side", "top")]);
    r.fixed("caption-bottom", &[("caption-side", "bottom")]);
    r.fixed("border-collapse", &[("border-collapse", "collapse")]);
    r.fixed("border-separate", &[("border-collapse", "separate")]);
    r.functional(
        "columns",
        Desc {
            keys: &["--columns", "--container"],
            value: Some(integer),
            statics: vec![("auto", vec![decl("columns", "auto")])],
            handle: Some(one("columns")),
            ..Desc::default()
        },
    );
    let before = [
        "auto",
        "avoid",
        "all",
        "avoid-page",
        "page",
        "left",
        "right",
        "column",
    ];
    for (side, values) in [
        ("before", &before[..]),
        (
            "inside",
            &["auto", "avoid", "avoid-page", "avoid-column"][..],
        ),
        ("after", &before[..]),
    ] {
        for value in values {
            let property = format!("break-{side}");
            r.fixed(
                format!("{property}-{value}"),
                &[(property.as_str(), *value)],
            );
        }
    }
    for (name, flow) in [
        ("row", "row"),
        ("col", "column"),
        ("dense", "dense"),
        ("row-dense", "row dense"),
        ("col-dense", "column dense"),
    ] {
        r.fixed(format!("grid-flow-{name}"), &[("grid-auto-flow", flow)]);
    }
    for (name, property) in [
        ("auto-cols", "grid-auto-columns"),
        ("auto-rows", "grid-auto-rows"),
    ] {
        let keys: &'static [&'static str] = match name {
            "auto-cols" => &["--grid-auto-columns"],
            _ => &["--grid-auto-rows"],
        };
        r.functional(
            name,
            Desc {
                keys,
                value: Some(|v, _| multiplier(v, "")),
                statics: vec![
                    ("auto", vec![decl(property, "auto")]),
                    ("min", vec![decl(property, "min-content")]),
                    ("max", vec![decl(property, "max-content")]),
                    ("fr", vec![decl(property, "minmax(0, 1fr)")]),
                ],
                handle: Some(one(property)),
                ..Desc::default()
            },
        );
    }
    for (name, property) in [
        ("grid-cols", "grid-template-columns"),
        ("grid-rows", "grid-template-rows"),
    ] {
        let keys: &'static [&'static str] = match name {
            "grid-cols" => &["--grid-template-columns"],
            _ => &["--grid-template-rows"],
        };
        r.functional(
            name,
            Desc {
                keys,
                value: Some(|v, _| {
                    let n = positive_integer(v) && v != "0";
                    n.then(|| format!("repeat({v}, minmax(0, 1fr))"))
                }),
                statics: vec![
                    ("none", vec![decl(property, "none")]),
                    ("subgrid", vec![decl(property, "subgrid")]),
                ],
                handle: Some(one(property)),
                ..Desc::default()
            },
        );
    }
    for (name, direction) in [
        ("row", "row"),
        ("row-reverse", "row-reverse"),
        ("col", "column"),
        ("col-reverse", "column-reverse"),
    ] {
        r.fixed(format!("flex-{name}"), &[("flex-direction", direction)]);
    }
    r.fixed("flex-wrap", &[("flex-wrap", "wrap")]);
    r.fixed("flex-nowrap", &[("flex-wrap", "nowrap")]);
    r.fixed("flex-wrap-reverse", &[("flex-wrap", "wrap-reverse")]);
    // Alignment: each prefix, its property, and its values as named and as written.
    let place_content: &[(&str, &str)] = &[
        ("center", "center"),
        ("start", "start"),
        ("end", "end"),
        ("center-safe", "safe center"),
        ("end-safe", "safe end"),
        ("between", "space-between"),
        ("around", "space-around"),
        ("evenly", "space-evenly"),
        ("baseline", "baseline"),
        ("stretch", "stretch"),
    ];
    let place_items: &[(&str, &str)] = &[
        ("center", "center"),
        ("start", "start"),
        ("end", "end"),
        ("center-safe", "safe center"),
        ("end-safe", "safe end"),
        ("baseline", "baseline"),
        ("stretch", "stretch"),
    ];
    let content: &[(&str, &str)] = &[
        ("normal", "normal"),
        ("center", "center"),
        ("start", "flex-start"),
        ("end", "flex-end"),
        ("center-safe", "safe center"),
        ("end-safe", "safe flex-end"),
        ("between", "space-between"),
        ("around", "space-around"),
        ("evenly", "space-evenly"),
        ("baseline", "baseline"),
        ("stretch", "stretch"),
    ];
    let items: &[(&str, &str)] = &[
        ("center", "center"),
        ("start", "flex-start"),
        ("end", "flex-end"),
        ("center-safe", "safe center"),
        ("end-safe", "safe flex-end"),
        ("baseline", "baseline"),
        ("baseline-last", "last baseline"),
        ("stretch", "stretch"),
    ];
    let justify_items: &[(&str, &str)] = &[
        ("normal", "normal"),
        ("center", "center"),
        ("start", "start"),
        ("end", "end"),
        ("center-safe", "safe center"),
        ("end-safe", "safe end"),
        ("stretch", "stretch"),
    ];
    for (prefix, property, values) in [
        ("place-content", "place-content", place_content),
        ("place-items", "place-items", place_items),
        ("content", "align-content", content),
        ("items", "align-items", items),
        ("justify", "justify-content", content),
        ("justify-items", "justify-items", justify_items),
    ] {
        aligned(r, prefix, property, values);
    }
    for (name, property) in [
        ("gap", "gap"),
        ("gap-x", "column-gap"),
        ("gap-y", "row-gap"),
    ] {
        r.spacing(name, &["--gap", "--spacing"], one(property), false, false);
    }
    for (axis, sort, start, end) in [
        ("x", "row-gap", "margin-inline-start", "margin-inline-end"),
        ("y", "column-gap", "margin-block-start", "margin-block-end"),
    ] {
        let reverse = match axis {
            "x" => "--tw-space-x-reverse",
            _ => "--tw-space-y-reverse",
        };
        let handle: Handle = Arc::new(move |v, _| {
            let zero = v == "--spacing(0)"
                || v == "--spacing(-0)"
                || dimension(v)
                    .is_some_and(|(n, unit)| n == 0.0 && (unit.is_empty() || types::length(v)));
            let (s, e) = match zero {
                true => ("0".to_string(), "0".to_string()),
                false => (
                    format!("calc({v} * var({reverse}))"),
                    format!("calc({v} * calc(1 - var({reverse})))"),
                ),
            };
            let children = vec![
                decl("--tw-sort", sort),
                decl(reverse, "0"),
                decl(start, s),
                decl(end, e),
            ];
            Some(vec![Node::Rule(CHILDREN, children)])
        });
        let name = format!("space-{axis}");
        r.spacing(&name, &["--space", "--spacing"], handle, true, false);
        let children = vec![decl("--tw-sort", sort), decl(reverse, "1")];
        r.nodes(
            format!("{name}-reverse"),
            vec![Node::Rule(CHILDREN, children)],
        );
    }
    let place_self: &[(&str, &str)] = &[
        ("auto", "auto"),
        ("start", "start"),
        ("end", "end"),
        ("center", "center"),
        ("end-safe", "safe end"),
        ("center-safe", "safe center"),
        ("stretch", "stretch"),
    ];
    let align_self: &[(&str, &str)] = &[
        ("auto", "auto"),
        ("start", "flex-start"),
        ("end", "flex-end"),
        ("center", "center"),
        ("end-safe", "safe flex-end"),
        ("center-safe", "safe center"),
        ("stretch", "stretch"),
        ("baseline", "baseline"),
        ("baseline-last", "last baseline"),
    ];
    aligned(r, "place-self", "place-self", place_self);
    aligned(r, "self", "align-self", align_self);
    aligned(r, "justify-self", "justify-self", &align_self[..7]);
    for value in ["auto", "hidden", "clip", "visible", "scroll"] {
        r.fixed(format!("overflow-{value}"), &[("overflow", value)]);
        r.fixed(format!("overflow-x-{value}"), &[("overflow-x", value)]);
        r.fixed(format!("overflow-y-{value}"), &[("overflow-y", value)]);
    }
    for (name, property) in [
        ("p", "padding"),
        ("px", "padding-inline"),
        ("py", "padding-block"),
        ("ps", "padding-inline-start"),
        ("pe", "padding-inline-end"),
        ("pbs", "padding-block-start"),
        ("pbe", "padding-block-end"),
        ("pt", "padding-top"),
        ("pr", "padding-right"),
        ("pb", "padding-bottom"),
        ("pl", "padding-left"),
    ] {
        r.spacing(
            name,
            &["--padding", "--spacing"],
            one(property),
            false,
            false,
        );
    }
    text(r);
    r.add("bg".into(), true, Box::new(background));
    r.fixed("fill-none", &[("fill", "none")]);
    r.add("fill".into(), true, Box::new(fill));
    r.fixed("stroke-none", &[("stroke", "none")]);
    r.add("stroke".into(), true, Box::new(stroke));
}

/// `bg-*`: a background color, or a background image of the theme; or written, a color, a
/// position, a size or an image, whichever kind it is.
fn background(c: &Candidate) -> Option<Vec<Node>> {
    let Candidate::Functional {
        value: Some(value),
        modifier,
        ..
    } = c
    else {
        return None;
    };
    let modifier = modifier.as_ref();
    let value = match value {
        Value::Arbitrary { kind, value } => {
            let kinds = [
                "image",
                "color",
                "percentage",
                "position",
                "bg-size",
                "length",
                "url",
            ];
            let set = |property: &str| match modifier {
                Some(_) => None,
                None => Some(vec![decl(property, value.clone())]),
            };
            return match kind.as_deref().or_else(|| types::infer(value, &kinds)) {
                Some("percentage" | "position") => set("background-position"),
                Some("bg-size" | "length" | "size") => set("background-size"),
                Some("image" | "url") => set("background-image"),
                _ => Some(vec![decl("background-color", as_color(value, modifier)?)]),
            };
        }
        Value::Named { value, .. } => value,
    };
    let keys = ["--background-color", "--color"];
    if let Some(color) = theme_color(value, modifier, &keys) {
        return Some(vec![decl("background-color", color)]);
    }
    if modifier.is_some() {
        return None;
    }
    let image = theme().resolve(Some(value), &["--background-image"])?;
    Some(vec![decl("background-image", image)])
}

/// `fill-*`: an SVG shape's fill color.
fn fill(c: &Candidate) -> Option<Vec<Node>> {
    let Candidate::Functional {
        value: Some(value),
        modifier,
        ..
    } = c
    else {
        return None;
    };
    let color = match value {
        Value::Arbitrary { value, .. } => as_color(value, modifier.as_ref())?,
        Value::Named { value, .. } => {
            theme_color(value, modifier.as_ref(), &["--fill", "--color"])?
        }
    };
    Some(vec![decl("fill", color)])
}

/// `stroke-*`: an SVG shape's stroke color, or its width.
fn stroke(c: &Candidate) -> Option<Vec<Node>> {
    let Candidate::Functional {
        value: Some(value),
        modifier,
        ..
    } = c
    else {
        return None;
    };
    let modifier = modifier.as_ref();
    let value = match value {
        Value::Arbitrary { kind, value } => {
            let kinds = ["color", "number", "length", "percentage"];
            return match kind.as_deref().or_else(|| types::infer(value, &kinds)) {
                Some("number" | "length" | "percentage") => match modifier {
                    Some(_) => None,
                    None => Some(vec![decl("stroke-width", value)]),
                },
                _ => Some(vec![decl("stroke", as_color(value, modifier)?)]),
            };
        }
        Value::Named { value, .. } => value,
    };
    if let Some(color) = theme_color(value, modifier, &["--stroke", "--color"]) {
        return Some(vec![decl("stroke", color)]);
    }
    if let Some(width) = theme().resolve(Some(value), &["--stroke-width"]) {
        return Some(vec![decl("stroke-width", width)]);
    }
    positive_integer(value).then(|| vec![decl("stroke-width", value)])
}

/// The text utilities: fonts, sizes, line heights and letter spacing, alignment, decoration,
/// case, wrapping and truncating.
fn text(r: &mut Registry) {
    r.functional(
        "line-clamp",
        Desc {
            keys: &["--line-clamp"],
            value: Some(integer),
            statics: vec![(
                "none",
                vec![
                    decl("overflow", "visible"),
                    decl("display", "block"),
                    decl("-webkit-box-orient", "horizontal"),
                    decl("-webkit-line-clamp", "unset"),
                ],
            )],
            handle: Some(Arc::new(|v, _| {
                Some(vec![
                    decl("overflow", "hidden"),
                    decl("display", "-webkit-box"),
                    decl("-webkit-box-orient", "vertical"),
                    decl("-webkit-line-clamp", v),
                ])
            })),
            ..Desc::default()
        },
    );
    r.fixed(
        "truncate",
        &[
            ("overflow", "hidden"),
            ("text-overflow", "ellipsis"),
            ("white-space", "nowrap"),
        ],
    );
    r.fixed("text-ellipsis", &[("text-overflow", "ellipsis")]);
    r.fixed("text-clip", &[("text-overflow", "clip")]);
    for value in ["none", "manual", "auto"] {
        let declarations = [("-webkit-hyphens", value), ("hyphens", value)];
        r.fixed(format!("hyphens-{value}"), &declarations);
    }
    for value in [
        "normal",
        "nowrap",
        "pre",
        "pre-line",
        "pre-wrap",
        "break-spaces",
    ] {
        r.fixed(format!("whitespace-{value}"), &[("white-space", value)]);
    }
    r.fixed("text-wrap", &[("text-wrap", "wrap")]);
    r.fixed("text-nowrap", &[("text-wrap", "nowrap")]);
    r.fixed("text-balance", &[("text-wrap", "balance")]);
    r.fixed("text-pretty", &[("text-wrap", "pretty")]);
    r.fixed(
        "break-normal",
        &[("overflow-wrap", "normal"), ("word-break", "normal")],
    );
    r.fixed("break-all", &[("word-break", "break-all")]);
    r.fixed("break-keep", &[("word-break", "keep-all")]);
    r.fixed("wrap-anywhere", &[("overflow-wrap", "anywhere")]);
    r.fixed("wrap-break-word", &[("overflow-wrap", "break-word")]);
    r.fixed("wrap-normal", &[("overflow-wrap", "normal")]);
    for align in ["left", "center", "right", "justify", "start", "end"] {
        r.fixed(format!("text-{align}"), &[("text-align", align)]);
    }
    let keys = &["--text-indent", "--spacing"];
    r.spacing("indent", keys, one("text-indent"), true, false);
    for align in [
        "baseline",
        "top",
        "middle",
        "bottom",
        "text-top",
        "text-bottom",
        "sub",
        "super",
    ] {
        r.fixed(format!("align-{align}"), &[("vertical-align", align)]);
    }
    r.functional(
        "align",
        Desc {
            handle: Some(one("vertical-align")),
            ..Desc::default()
        },
    );
    r.add("font".into(), true, Box::new(font));
    r.functional(
        "font-features",
        Desc {
            handle: Some(one("font-feature-settings")),
            ..Desc::default()
        },
    );
    r.fixed("uppercase", &[("text-transform", "uppercase")]);
    r.fixed("lowercase", &[("text-transform", "lowercase")]);
    r.fixed("capitalize", &[("text-transform", "capitalize")]);
    r.fixed("normal-case", &[("text-transform", "none")]);
    r.fixed("italic", &[("font-style", "italic")]);
    r.fixed("not-italic", &[("font-style", "normal")]);
    for (name, line) in [
        ("underline", "underline"),
        ("overline", "overline"),
        ("line-through", "line-through"),
        ("no-underline", "none"),
    ] {
        r.fixed(name, &[("text-decoration-line", line)]);
    }
    for stretch in [
        "normal",
        "ultra-condensed",
        "extra-condensed",
        "condensed",
        "semi-condensed",
        "semi-expanded",
        "expanded",
        "extra-expanded",
        "ultra-expanded",
    ] {
        r.fixed(
            format!("font-stretch-{stretch}"),
            &[("font-stretch", stretch)],
        );
    }
    r.functional(
        "font-stretch",
        Desc {
            value: Some(|v, _| {
                let n: f64 = v.strip_suffix('%')?.parse().ok()?;
                (n.fract() == 0.0 && (50.0..=200.0).contains(&n)).then(|| v.to_string())
            }),
            handle: Some(one("font-stretch")),
            ..Desc::default()
        },
    );
    for style in ["solid", "double", "dotted", "dashed", "wavy"] {
        let name = format!("decoration-{style}");
        r.fixed(name, &[("text-decoration-style", style)]);
    }
    r.fixed("decoration-auto", &[("text-decoration-thickness", "auto")]);
    r.fixed(
        "decoration-from-font",
        &[("text-decoration-thickness", "from-font")],
    );
    r.add("decoration".into(), true, Box::new(decoration));
    r.functional(
        "underline-offset",
        Desc {
            negative: true,
            keys: &["--text-underline-offset"],
            value: Some(|v, _| positive_integer(v).then(|| format!("{v}px"))),
            statics: vec![("auto", vec![decl("text-underline-offset", "auto")])],
            handle: Some(one("text-underline-offset")),
            ..Desc::default()
        },
    );
    r.add("text".into(), true, Box::new(text_size_or_color));
    let leading: Handle =
        Arc::new(|v, _| Some(vec![decl("--tw-leading", v), decl("line-height", v)]));
    r.spacing(
        "leading",
        &["--leading", "--spacing"],
        leading,
        false,
        false,
    );
    r.nodes(
        "leading-none",
        vec![decl("--tw-leading", "1"), decl("line-height", "1")],
    );
    r.functional(
        "tracking",
        Desc {
            negative: true,
            keys: &["--tracking"],
            handle: Some(Arc::new(|v, _| {
                Some(vec![decl("--tw-tracking", v), decl("letter-spacing", v)])
            })),
            ..Desc::default()
        },
    );
    r.fixed(
        "antialiased",
        &[
            ("-webkit-font-smoothing", "antialiased"),
            ("-moz-osx-font-smoothing", "grayscale"),
        ],
    );
    r.fixed(
        "subpixel-antialiased",
        &[
            ("-webkit-font-smoothing", "auto"),
            ("-moz-osx-font-smoothing", "auto"),
        ],
    );
    r.fixed("normal-nums", &[("font-variant-numeric", "normal")]);
    let numeric = "var(--tw-ordinal,) var(--tw-slashed-zero,) var(--tw-numeric-figure,) \
                   var(--tw-numeric-spacing,) var(--tw-numeric-fraction,)";
    for (name, variable) in [
        ("ordinal", "--tw-ordinal"),
        ("slashed-zero", "--tw-slashed-zero"),
        ("lining-nums", "--tw-numeric-figure"),
        ("oldstyle-nums", "--tw-numeric-figure"),
        ("proportional-nums", "--tw-numeric-spacing"),
        ("tabular-nums", "--tw-numeric-spacing"),
        ("diagonal-fractions", "--tw-numeric-fraction"),
        ("stacked-fractions", "--tw-numeric-fraction"),
    ] {
        r.fixed(name, &[(variable, name), ("font-variant-numeric", numeric)]);
    }
}

/// `font-*`: a family of the theme or written, or a weight.
fn font(c: &Candidate) -> Option<Vec<Node>> {
    let Candidate::Functional {
        value: Some(value),
        modifier: None,
        ..
    } = c
    else {
        return None;
    };
    let weight = |w: String| {
        Some(vec![
            decl("--tw-font-weight", w.clone()),
            decl("font-weight", w),
        ])
    };
    let value = match value {
        Value::Arbitrary { kind, value } => {
            let kinds = ["number", "generic-name", "family-name"];
            return match kind.as_deref().or_else(|| types::infer(value, &kinds)) {
                Some("generic-name" | "family-name") => Some(vec![decl("font-family", value)]),
                _ => weight(value.clone()),
            };
        }
        Value::Named { value, .. } => value,
    };
    let nested = ["--font-feature-settings", "--font-variation-settings"];
    if let Some((family, settings)) = theme().resolve_with(value, &["--font"], &nested) {
        let mut nodes = vec![decl("font-family", family)];
        for (property, setting) in ["font-feature-settings", "font-variation-settings"]
            .into_iter()
            .zip(settings)
        {
            nodes.extend(setting.map(|s| decl(property, s)));
        }
        return Some(nodes);
    }
    weight(theme().resolve(Some(value), &["--font-weight"])?)
}

/// `decoration-*`: the thickness of text decoration lines, or their color.
fn decoration(c: &Candidate) -> Option<Vec<Node>> {
    let Candidate::Functional {
        value: Some(value),
        modifier,
        ..
    } = c
    else {
        return None;
    };
    let thickness = |t: String| match modifier {
        Some(_) => None,
        None => Some(vec![decl("text-decoration-thickness", t)]),
    };
    let value = match value {
        Value::Arbitrary { kind, value } => {
            let kinds = ["color", "length", "percentage"];
            return match kind.as_deref().or_else(|| types::infer(value, &kinds)) {
                Some("length" | "percentage") => thickness(value.clone()),
                _ => Some(vec![decl(
                    "text-decoration-color",
                    as_color(value, modifier.as_ref())?,
                )]),
            };
        }
        Value::Named { value, .. } => value,
    };
    if let Some(t) = theme().resolve(Some(value), &["--text-decoration-thickness"]) {
        return thickness(t);
    }
    if positive_integer(value) {
        return thickness(format!("{value}px"));
    }
    let keys = ["--text-decoration-color", "--color"];
    let color = theme_color(value, modifier.as_ref(), &keys)?;
    Some(vec![decl("text-decoration-color", color)])
}

/// `text-*`: a font size of the theme, with its line height and the rest, or written, with a line
/// height after `/`; or a color.
fn text_size_or_color(c: &Candidate) -> Option<Vec<Node>> {
    let Candidate::Functional {
        value: Some(value),
        modifier,
        ..
    } = c
    else {
        return None;
    };
    let modifier = modifier.as_ref();
    let value = match value {
        Value::Arbitrary { kind, value } => {
            let kinds = [
                "color",
                "length",
                "percentage",
                "absolute-size",
                "relative-size",
            ];
            return match kind.as_deref().or_else(|| types::infer(value, &kinds)) {
                Some("size" | "length" | "percentage" | "absolute-size" | "relative-size") => {
                    let mut nodes = vec![decl("font-size", value)];
                    if let Some(m) = modifier {
                        nodes.push(decl("line-height", leading(m)?));
                    }
                    Some(nodes)
                }
                _ => Some(vec![decl("color", as_color(value, modifier)?)]),
            };
        }
        Value::Named { value, .. } => value,
    };
    if let Some(color) = theme_color(value, modifier, &["--text-color", "--color"]) {
        return Some(vec![decl("color", color)]);
    }
    let nested = ["--line-height", "--letter-spacing", "--font-weight"];
    let (size, nested) = theme().resolve_with(value, &["--text"], &nested)?;
    let [line_height, letter_spacing, font_weight] =
        <[Option<String>; 3]>::try_from(nested).ok()?;
    let mut nodes = vec![decl("font-size", size)];
    if let Some(m) = modifier {
        nodes.push(decl("line-height", leading(m)?));
        return Some(nodes);
    }
    for (property, variable, value) in [
        ("line-height", "--tw-leading", line_height),
        ("letter-spacing", "--tw-tracking", letter_spacing),
        ("font-weight", "--tw-font-weight", font_weight),
    ] {
        if let Some(v) = value {
            nodes.push(decl(property, format!("var({variable}, {v})")));
        }
    }
    Some(nodes)
}

/// The line height a font size's modifier gives: `text-sm/6`, `text-sm/[20px]`, `text-sm/none`.
fn leading(m: &Modifier) -> Option<String> {
    match m {
        Modifier::Arbitrary(v) if !v.is_empty() => Some(v.clone()),
        Modifier::Arbitrary(_) => None,
        Modifier::Named(m) => {
            if let Some(v) = theme().resolve(Some(m), &["--leading"]) {
                return Some(v);
            }
            if quarter(m) {
                theme().resolve(None, &["--spacing"])?;
                return Some(format!("--spacing({m})"));
            }
            (m == "none").then(|| "1".to_string())
        }
    }
}

/// A color a named value names, at the opacity its modifier gives, as Tailwind's
/// `resolveThemeColor`: `current`, `inherit`, `transparent`, or a variable of `keys`.
fn theme_color(value: &str, modifier: Option<&Modifier>, keys: &[&str]) -> Option<String> {
    let color = match value {
        "inherit" => "inherit".to_string(),
        "transparent" => "transparent".to_string(),
        "current" => "currentcolor".to_string(),
        _ => theme().resolve(Some(value), keys)?,
    };
    as_color(&color, modifier)
}

/// `col-*` or `row-*`: a grid item's lines, `-span-*`, `-start-*` and `-end-*`.
fn lines(r: &mut Registry, name: &str, property: &'static str) {
    let key = |suffix: &str| -> &'static [&'static str] {
        match (property, suffix) {
            ("grid-column", "") => &["--grid-column"],
            ("grid-column", "-start") => &["--grid-column-start"],
            ("grid-column", _) => &["--grid-column-end"],
            ("grid-row", "") => &["--grid-row"],
            ("grid-row", "-start") => &["--grid-row-start"],
            _ => &["--grid-row-end"],
        }
    };
    r.functional(
        name,
        Desc {
            negative: true,
            keys: key(""),
            value: Some(integer),
            statics: vec![("auto", vec![decl(property, "auto")])],
            handle: Some(one(property)),
            ..Desc::default()
        },
    );
    r.functional(
        &format!("{name}-span"),
        Desc {
            value: Some(integer),
            statics: vec![("full", vec![decl(property, "1 / -1")])],
            handle: Some(Arc::new(move |v, _| {
                Some(vec![decl(property, format!("span {v} / span {v}"))])
            })),
            ..Desc::default()
        },
    );
    for (suffix, longhand) in match property {
        "grid-column" => [("-start", "grid-column-start"), ("-end", "grid-column-end")],
        _ => [("-start", "grid-row-start"), ("-end", "grid-row-end")],
    } {
        r.functional(
            &format!("{name}{suffix}"),
            Desc {
                negative: true,
                keys: key(suffix),
                value: Some(integer),
                statics: vec![("auto", vec![decl(longhand, "auto")])],
                handle: Some(one(longhand)),
                ..Desc::default()
            },
        );
    }
}

/// Static alignment utilities: `{prefix}-{name}` sets `property` to each value.
fn aligned(r: &mut Registry, prefix: &str, property: &str, values: &[(&str, &str)]) {
    for (name, value) in values {
        r.fixed(format!("{prefix}-{name}"), &[(property, value)]);
    }
}

/// `w-*`, `h-*`, `size-*` and the rest that size a box.
fn sizes(r: &mut Registry) {
    let sized = |v: &str| {
        vec![
            decl("--tw-sort", "size"),
            decl("width", v),
            decl("height", v),
        ]
    };
    for (key, value) in [
        ("full", "100%"),
        ("svw", "100svw"),
        ("lvw", "100lvw"),
        ("dvw", "100dvw"),
        ("svh", "100svh"),
        ("lvh", "100lvh"),
        ("dvh", "100dvh"),
        ("min", "min-content"),
        ("max", "max-content"),
        ("fit", "fit-content"),
    ] {
        r.nodes(format!("size-{key}"), sized(value));
        for (name, property) in [
            ("w", "width"),
            ("h", "height"),
            ("min-w", "min-width"),
            ("min-h", "min-height"),
            ("max-w", "max-width"),
            ("max-h", "max-height"),
        ] {
            r.fixed(format!("{name}-{key}"), &[(property, value)]);
        }
    }
    r.nodes("size-auto", sized("auto"));
    for (name, property, value) in [
        ("w-auto", "width", "auto"),
        ("h-auto", "height", "auto"),
        ("min-w-auto", "min-width", "auto"),
        ("min-h-auto", "min-height", "auto"),
        ("h-lh", "height", "1lh"),
        ("min-h-lh", "min-height", "1lh"),
        ("max-h-lh", "max-height", "1lh"),
        ("w-screen", "width", "100vw"),
        ("min-w-screen", "min-width", "100vw"),
        ("max-w-screen", "max-width", "100vw"),
        ("h-screen", "height", "100vh"),
        ("min-h-screen", "min-height", "100vh"),
        ("max-h-screen", "max-height", "100vh"),
        ("max-w-none", "max-width", "none"),
        ("max-h-none", "max-height", "none"),
    ] {
        r.fixed(name, &[(property, value)]);
    }
    let handle: Handle = Arc::new(move |v, _| Some(sized(v)));
    r.spacing("size", &["--size", "--spacing"], handle, false, true);
    for (name, keys, property) in [
        ("w", &["--width", "--spacing", "--container"][..], "width"),
        (
            "min-w",
            &["--min-width", "--spacing", "--container"],
            "min-width",
        ),
        (
            "max-w",
            &["--max-width", "--spacing", "--container"],
            "max-width",
        ),
        ("h", &["--height", "--spacing"], "height"),
        (
            "min-h",
            &["--min-height", "--height", "--spacing"],
            "min-height",
        ),
        (
            "max-h",
            &["--max-height", "--height", "--spacing"],
            "max-height",
        ),
    ] {
        r.spacing(name, keys, one(property), false, true);
    }
    // The logical sizes.
    for (key, value) in [
        ("full", "100%"),
        ("min", "min-content"),
        ("max", "max-content"),
        ("fit", "fit-content"),
    ] {
        for (name, property) in [
            ("inline", "inline-size"),
            ("block", "block-size"),
            ("min-inline", "min-inline-size"),
            ("min-block", "min-block-size"),
            ("max-inline", "max-inline-size"),
            ("max-block", "max-block-size"),
        ] {
            r.fixed(format!("{name}-{key}"), &[(property, value)]);
        }
    }
    for (axis, units) in [
        ("inline", ["svw", "lvw", "dvw"]),
        ("block", ["svh", "lvh", "dvh"]),
    ] {
        for unit in units {
            for name in [
                axis.to_string(),
                format!("min-{axis}"),
                format!("max-{axis}"),
            ] {
                let property = format!("{name}-size");
                r.fixed(
                    format!("{name}-{unit}"),
                    &[(&property, &format!("100{unit}"))],
                );
            }
        }
    }
    for (name, property, value) in [
        ("inline-auto", "inline-size", "auto"),
        ("block-auto", "block-size", "auto"),
        ("min-inline-auto", "min-inline-size", "auto"),
        ("min-block-auto", "min-block-size", "auto"),
        ("block-lh", "block-size", "1lh"),
        ("min-block-lh", "min-block-size", "1lh"),
        ("max-block-lh", "max-block-size", "1lh"),
        ("inline-screen", "inline-size", "100vw"),
        ("min-inline-screen", "min-inline-size", "100vw"),
        ("max-inline-screen", "max-inline-size", "100vw"),
        ("block-screen", "block-size", "100vh"),
        ("min-block-screen", "min-block-size", "100vh"),
        ("max-block-screen", "max-block-size", "100vh"),
        ("max-inline-none", "max-inline-size", "none"),
        ("max-block-none", "max-block-size", "none"),
    ] {
        r.fixed(name, &[(property, value)]);
    }
    for (name, keys, property) in [
        ("inline", &["--spacing", "--container"][..], "inline-size"),
        (
            "min-inline",
            &["--spacing", "--container"],
            "min-inline-size",
        ),
        (
            "max-inline",
            &["--spacing", "--container"],
            "max-inline-size",
        ),
        ("block", &["--spacing"], "block-size"),
        ("min-block", &["--spacing"], "min-block-size"),
        ("max-block", &["--spacing"], "max-block-size"),
    ] {
        r.spacing(name, keys, one(property), false, true);
    }
    // Its widths follow the theme's breakpoints.
    let mut container = vec![
        decl("--tw-sort", "--tw-container-component"),
        decl("width", "100%"),
    ];
    let mut breakpoints: Vec<&str> = ["sm", "md", "lg", "xl", "2xl"]
        .iter()
        .filter_map(|b| theme().value(&format!("--breakpoint-{b}")))
        .collect();
    breakpoints.sort_by(|a, b| {
        let rem = |s: &str| s.trim_end_matches("rem").parse::<f64>().unwrap_or(0.0);
        rem(a).total_cmp(&rem(b))
    });
    for b in breakpoints {
        let width = vec![decl("max-width", b)];
        container.push(Node::At(format!("@media (width >= {b})"), width));
    }
    r.nodes("container", container);
}
