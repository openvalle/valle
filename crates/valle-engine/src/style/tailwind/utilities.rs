//! The utilities, as Tailwind's `utilities.ts` registers them (MIT, see THIRD_PARTY.md): each
//! makes CSS of the candidates that are its own, and nothing of the rest. Families come over one
//! at a time; a class of one that has not is not supported yet.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use super::{CHILDREN, Candidate, Modifier, Node, Value};
use super::{dimension, positive_integer, quarter, segment, spaced, theme, types};

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
    let alpha = percent(alpha);
    if alpha == "100%" {
        return value.into();
    }
    format!("color-mix(in oklab, {value} {alpha}, transparent)")
}

/// As Tailwind's `replaceAlpha`: the color at the opacity, whatever its own.
fn replace_alpha(value: &str, alpha: &str) -> String {
    format!("oklab(from {value} l a b / {})", percent(alpha))
}

/// An opacity as a percentage, a number of them times 100.
fn percent(alpha: &str) -> String {
    match alpha.trim().parse::<f64>() {
        Ok(n) if !alpha.trim().is_empty() => format!("{}%", crate::js::number_to_string(n * 100.0)),
        _ => alpha.to_string(),
    }
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
    backgrounds(r);
    borders(r);
    shadows(r);
    blending(r);
    transforms(r);
    r.add("bg".into(), true, Box::new(background));
    r.fixed("fill-none", &[("fill", "none")]);
    let keys = &["--fill", "--color"];
    colors(r, "fill", keys, |c| vec![decl("fill", c)]);
    let keys = &["--accent-color", "--color"];
    colors(r, "accent", keys, |c| vec![decl("accent-color", c)]);
    let keys = &["--caret-color", "--color"];
    colors(r, "caret", keys, |c| vec![decl("caret-color", c)]);
    r.fixed("stroke-none", &[("stroke", "none")]);
    r.add("stroke".into(), true, Box::new(stroke));
}

/// The background utilities but `bg-*`: sizes, positions, repeats, boxes, and gradients with their
/// color stops.
fn backgrounds(r: &mut Registry) {
    for (name, value) in [("auto", "auto"), ("cover", "cover"), ("contain", "contain")] {
        r.fixed(format!("bg-{name}"), &[("background-size", value)]);
    }
    for value in ["fixed", "local", "scroll"] {
        r.fixed(format!("bg-{value}"), &[("background-attachment", value)]);
    }
    for (name, value) in [
        ("top", "top"),
        ("top-left", "left top"),
        ("top-right", "right top"),
        ("bottom", "bottom"),
        ("bottom-left", "left bottom"),
        ("bottom-right", "right bottom"),
        ("left", "left"),
        ("right", "right"),
        ("center", "center"),
    ] {
        r.fixed(format!("bg-{name}"), &[("background-position", value)]);
    }
    for (name, property) in [
        ("bg-size", "background-size"),
        ("bg-position", "background-position"),
    ] {
        let handle = Some(one(property));
        r.functional(
            name,
            Desc {
                handle,
                ..Desc::default()
            },
        );
    }
    for (name, value) in [
        ("repeat", "repeat"),
        ("no-repeat", "no-repeat"),
        ("repeat-x", "repeat-x"),
        ("repeat-y", "repeat-y"),
        ("repeat-round", "round"),
        ("repeat-space", "space"),
    ] {
        r.fixed(format!("bg-{name}"), &[("background-repeat", value)]);
    }
    r.fixed("bg-none", &[("background-image", "none")]);
    for (name, value) in [
        ("text", "text"),
        ("border", "border-box"),
        ("padding", "padding-box"),
        ("content", "content-box"),
    ] {
        r.fixed(format!("bg-clip-{name}"), &[("background-clip", value)]);
    }
    for (name, value) in [
        ("border", "border-box"),
        ("padding", "padding-box"),
        ("content", "content-box"),
    ] {
        r.fixed(format!("bg-origin-{name}"), &[("background-origin", value)]);
    }
    for negative in [false, true] {
        let sign = if negative { "-" } else { "" };
        r.add(
            format!("{sign}bg-linear"),
            true,
            Box::new(move |c| linear(c, negative)),
        );
        r.add(
            format!("{sign}bg-conic"),
            true,
            Box::new(move |c| conic(c, negative)),
        );
    }
    r.add("bg-radial".into(), true, Box::new(radial));
    r.fixed("via-none", &[("--tw-gradient-via-stops", "initial")]);
    for stop in ["from", "via", "to"] {
        r.add(stop.into(), true, Box::new(move |c| color_stop(c, stop)));
    }
}

/// The color space a gradient's modifier names its colors to mix in: Oklab without one, a way
/// round Oklch's hue, or written.
fn interpolation(modifier: Option<&Modifier>) -> String {
    const WAYS: [&str; 4] = ["longer", "shorter", "increasing", "decreasing"];
    match modifier {
        None => "in oklab".into(),
        Some(Modifier::Named(way)) if WAYS.contains(&way.as_str()) => {
            format!("in oklch {way} hue")
        }
        Some(Modifier::Named(space)) => format!("in {space}"),
        Some(Modifier::Arbitrary(method)) => method.clone(),
    }
}

/// `bg-linear-*`: a linear gradient of the color stops, to a side or at an angle, or written; its
/// colors mix in the space its modifier names, Oklab without one.
fn linear(c: &Candidate, negative: bool) -> Option<Vec<Node>> {
    const SIDES: [(&str, &str); 8] = [
        ("to-t", "to top"),
        ("to-tr", "to top right"),
        ("to-r", "to right"),
        ("to-br", "to bottom right"),
        ("to-b", "to bottom"),
        ("to-bl", "to bottom left"),
        ("to-l", "to left"),
        ("to-tl", "to top left"),
    ];
    let Candidate::Functional {
        value: Some(value),
        modifier,
        ..
    } = c
    else {
        return None;
    };
    let position = match value {
        Value::Arbitrary { kind, value } => {
            if modifier.is_some() {
                return None;
            }
            let angle =
                kind.as_deref().or_else(|| types::infer(value, &["angle"])) == Some("angle");
            let position = match (angle, negative) {
                (true, true) => format!("calc({value} * -1)"),
                (_, false) => value.clone(),
                (false, true) => return None,
            };
            let image = format!("linear-gradient(var(--tw-gradient-stops,{position}))");
            return Some(vec![
                decl("--tw-gradient-position", position),
                decl("background-image", image),
            ]);
        }
        Value::Named { value, .. } => {
            match SIDES.iter().find(|(name, _)| *name == value.as_str()) {
                Some((_, side)) if !negative => side.to_string(),
                _ if positive_integer(value) && negative => format!("calc({value}deg * -1)"),
                _ if positive_integer(value) => format!("{value}deg"),
                _ => return None,
            }
        }
    };
    let mixed = format!("{position} {}", interpolation(modifier.as_ref()));
    Some(vec![
        decl("--tw-gradient-position", position),
        Node::At(
            "@supports (background-image: linear-gradient(in lab, red, red))".into(),
            vec![decl("--tw-gradient-position", mixed)],
        ),
        decl(
            "background-image",
            "linear-gradient(var(--tw-gradient-stops))",
        ),
    ])
}

/// `bg-conic`, `bg-conic-*`: a conic gradient of the color stops, from an angle, or written.
fn conic(c: &Candidate, negative: bool) -> Option<Vec<Node>> {
    let Candidate::Functional {
        value, modifier, ..
    } = c
    else {
        return None;
    };
    let method = interpolation(modifier.as_ref());
    let stops = "conic-gradient(var(--tw-gradient-stops))".to_string();
    let (position, image) = match value {
        Some(Value::Arbitrary { value, .. }) if modifier.is_none() => (
            value.clone(),
            format!("conic-gradient(var(--tw-gradient-stops,{value}))"),
        ),
        None => (method, stops),
        Some(Value::Named { value, .. }) if positive_integer(value) => {
            let angle = match negative {
                true => format!("calc({value}deg * -1)"),
                false => format!("{value}deg"),
            };
            (format!("from {angle} {method}"), stops)
        }
        _ => return None,
    };
    Some(vec![
        decl("--tw-gradient-position", position),
        decl("background-image", image),
    ])
}

/// `bg-radial`, `bg-radial-[…]`: a radial gradient of the color stops, or written.
fn radial(c: &Candidate) -> Option<Vec<Node>> {
    let Candidate::Functional {
        value, modifier, ..
    } = c
    else {
        return None;
    };
    let (position, image) = match value {
        None => (
            interpolation(modifier.as_ref()),
            "radial-gradient(var(--tw-gradient-stops))".to_string(),
        ),
        Some(Value::Arbitrary { value, .. }) if modifier.is_none() => (
            value.clone(),
            format!("radial-gradient(var(--tw-gradient-stops,{value}))"),
        ),
        Some(_) => return None,
    };
    Some(vec![
        decl("--tw-gradient-position", position),
        decl("background-image", image),
    ])
}

/// `from-*`, `via-*`, `to-*`: a gradient's color stop, its color or its position.
fn color_stop(c: &Candidate, stop: &str) -> Option<Vec<Node>> {
    let Candidate::Functional {
        value: Some(value),
        modifier,
        ..
    } = c
    else {
        return None;
    };
    let modifier = modifier.as_ref();
    let position = |at: &str| vec![decl(&format!("--tw-gradient-{stop}-position"), at)];
    let color = match value {
        Value::Arbitrary { kind, value } => {
            let kinds = ["color", "length", "percentage"];
            match kind.as_deref().or_else(|| types::infer(value, &kinds)) {
                Some("length" | "percentage") => {
                    return modifier.is_none().then(|| position(value));
                }
                _ => as_color(value, modifier)?,
            }
        }
        Value::Named { value, .. } => {
            match theme_color(value, modifier, &["--background-color", "--color"]) {
                Some(color) => color,
                None if modifier.is_some() => return None,
                None => {
                    let keys = ["--gradient-color-stop-positions"];
                    if let Some(at) = theme().resolve(Some(value), &keys) {
                        return Some(position(&at));
                    }
                    let percent = value.strip_suffix('%').is_some_and(positive_integer);
                    return percent.then(|| position(value));
                }
            }
        }
    };
    // From and to stand alone; a via stop goes between them.
    let ends = "var(--tw-gradient-from) var(--tw-gradient-from-position), var(--tw-gradient-to) \
                var(--tw-gradient-to-position)";
    let stops = match stop {
        "via" => vec![
            decl(
                "--tw-gradient-via-stops",
                "var(--tw-gradient-position), var(--tw-gradient-from) \
                 var(--tw-gradient-from-position), var(--tw-gradient-via) \
                 var(--tw-gradient-via-position), var(--tw-gradient-to) \
                 var(--tw-gradient-to-position)",
            ),
            decl("--tw-gradient-stops", "var(--tw-gradient-via-stops)"),
        ],
        _ => vec![decl(
            "--tw-gradient-stops",
            format!("var(--tw-gradient-via-stops, var(--tw-gradient-position), {ends})"),
        )],
    };
    let variable = format!("--tw-gradient-{stop}");
    let mut nodes = vec![decl("--tw-sort", &variable), decl(&variable, color)];
    nodes.extend(stops);
    Some(nodes)
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

/// A color utility, as Tailwind's `colorUtility`: a color of the theme's `keys` at the opacity
/// its modifier gives, or written, into the declarations `make` makes of it.
fn colors(
    r: &mut Registry,
    name: &str,
    keys: &'static [&'static str],
    make: fn(String) -> Vec<Node>,
) {
    let compile = move |c: &Candidate| {
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
            Value::Named { value, .. } => theme_color(value, modifier.as_ref(), keys)?,
        };
        Some(make(color))
    };
    r.add(name.into(), true, Box::new(compile));
}

/// The border utilities: radii of each corner, widths, colors and styles of each side, lines
/// between children, and outlines.
fn borders(r: &mut Registry) {
    const CORNERS: [(&str, &[&str]); 15] = [
        ("rounded", &["border-radius"]),
        (
            "rounded-s",
            &["border-start-start-radius", "border-end-start-radius"],
        ),
        (
            "rounded-e",
            &["border-start-end-radius", "border-end-end-radius"],
        ),
        (
            "rounded-t",
            &["border-top-left-radius", "border-top-right-radius"],
        ),
        (
            "rounded-r",
            &["border-top-right-radius", "border-bottom-right-radius"],
        ),
        (
            "rounded-b",
            &["border-bottom-right-radius", "border-bottom-left-radius"],
        ),
        (
            "rounded-l",
            &["border-top-left-radius", "border-bottom-left-radius"],
        ),
        ("rounded-ss", &["border-start-start-radius"]),
        ("rounded-se", &["border-start-end-radius"]),
        ("rounded-ee", &["border-end-end-radius"]),
        ("rounded-es", &["border-end-start-radius"]),
        ("rounded-tl", &["border-top-left-radius"]),
        ("rounded-tr", &["border-top-right-radius"]),
        ("rounded-br", &["border-bottom-right-radius"]),
        ("rounded-bl", &["border-bottom-left-radius"]),
    ];
    for (name, properties) in CORNERS {
        let all = |v: &str| properties.iter().map(|p| decl(p, v)).collect::<Vec<_>>();
        let statics = vec![("none", all("0")), ("full", all("calc(infinity * 1px)"))];
        let handle: Handle =
            Arc::new(move |v, _| Some(properties.iter().map(|p| decl(p, v)).collect()));
        r.functional(
            name,
            Desc {
                keys: &["--radius"],
                statics,
                handle: Some(handle),
                ..Desc::default()
            },
        );
    }
    for style in ["solid", "dashed", "dotted", "double", "hidden", "none"] {
        let declarations = [("--tw-border-style", style), ("border-style", style)];
        r.fixed(format!("border-{style}"), &declarations);
    }
    for (name, side) in [
        ("border", "border"),
        ("border-x", "border-inline"),
        ("border-y", "border-block"),
        ("border-s", "border-inline-start"),
        ("border-e", "border-inline-end"),
        ("border-bs", "border-block-start"),
        ("border-be", "border-block-end"),
        ("border-t", "border-top"),
        ("border-r", "border-right"),
        ("border-b", "border-bottom"),
        ("border-l", "border-left"),
    ] {
        r.add(name.into(), true, Box::new(move |c| border(c, side)));
    }
    for axis in ["x", "y"] {
        let reverse = match axis {
            "x" => "--tw-divide-x-reverse",
            _ => "--tw-divide-y-reverse",
        };
        let handle: Handle = Arc::new(move |w, _| {
            let start = format!("calc({w} * var({reverse}))");
            let end = format!("calc({w} * calc(1 - var({reverse})))");
            let mut children = vec![decl("--tw-sort", format!("divide-{axis}-width"))];
            children.push(decl(reverse, "0"));
            children.extend(match axis {
                "x" => vec![
                    decl("border-inline-style", "var(--tw-border-style)"),
                    decl("border-inline-start-width", start),
                    decl("border-inline-end-width", end),
                ],
                _ => vec![
                    decl("border-bottom-style", "var(--tw-border-style)"),
                    decl("border-top-style", "var(--tw-border-style)"),
                    decl("border-top-width", start),
                    decl("border-bottom-width", end),
                ],
            });
            Some(vec![Node::Rule(CHILDREN, children)])
        });
        r.functional(
            &format!("divide-{axis}"),
            Desc {
                keys: &["--divide-width", "--border-width"],
                bare: Bare::Value("1px"),
                value: Some(|v, _| positive_integer(v).then(|| format!("{v}px"))),
                handle: Some(handle),
                ..Desc::default()
            },
        );
        let children = vec![decl(reverse, "1")];
        r.nodes(
            format!("divide-{axis}-reverse"),
            vec![Node::Rule(CHILDREN, children)],
        );
    }
    for style in ["solid", "dashed", "dotted", "double", "none"] {
        let children = vec![
            decl("--tw-sort", "divide-style"),
            decl("--tw-border-style", style),
            decl("border-style", style),
        ];
        r.nodes(
            format!("divide-{style}"),
            vec![Node::Rule(CHILDREN, children)],
        );
    }
    let keys = &["--divide-color", "--border-color", "--color"];
    colors(r, "divide", keys, |c| {
        let children = vec![decl("--tw-sort", "divide-color"), decl("border-color", c)];
        vec![Node::Rule(CHILDREN, children)]
    });
    r.nodes(
        "outline-hidden",
        vec![
            decl("--tw-outline-style", "none"),
            decl("outline-style", "none"),
            Node::At(
                "@media (forced-colors: active)".into(),
                vec![
                    decl("outline", "2px solid transparent"),
                    decl("outline-offset", "2px"),
                ],
            ),
        ],
    );
    for style in ["none", "solid", "dashed", "dotted", "double"] {
        let declarations = [("--tw-outline-style", style), ("outline-style", style)];
        r.fixed(format!("outline-{style}"), &declarations);
    }
    r.add("outline".into(), true, Box::new(outline));
    r.functional(
        "outline-offset",
        Desc {
            negative: true,
            keys: &["--outline-offset"],
            value: Some(|v, _| positive_integer(v).then(|| format!("{v}px"))),
            handle: Some(one("outline-offset")),
            ..Desc::default()
        },
    );
}

/// `border-*`, `border-x-*` and the other sides: a width, of the theme, in px or written, in the
/// style `--tw-border-style` says; or a color.
fn border(c: &Candidate, side: &str) -> Option<Vec<Node>> {
    let Candidate::Functional {
        value, modifier, ..
    } = c
    else {
        return None;
    };
    let modifier = modifier.as_ref();
    let width = |w: &str| {
        Some(vec![
            decl(&format!("{side}-style"), "var(--tw-border-style)"),
            decl(&format!("{side}-width"), w),
        ])
    };
    let color = |c: String| Some(vec![decl(&format!("{side}-color"), c)]);
    let value = match value {
        None if modifier.is_some() => return None,
        None => return width("1px"),
        Some(Value::Arbitrary { kind, value }) => {
            let kinds = ["color", "line-width", "length"];
            return match kind.as_deref().or_else(|| types::infer(value, &kinds)) {
                Some("line-width" | "length") if modifier.is_none() => width(value),
                Some("line-width" | "length") => None,
                _ => color(as_color(value, modifier)?),
            };
        }
        Some(Value::Named { value, .. }) => value,
    };
    if let Some(c) = theme_color(value, modifier, &["--border-color", "--color"]) {
        return color(c);
    }
    if modifier.is_some() {
        return None;
    }
    if let Some(w) = theme().resolve(Some(value), &["--border-width"]) {
        return width(&w);
    }
    positive_integer(value)
        .then(|| width(&format!("{value}px")))
        .flatten()
}

/// `outline`, `outline-*`: a width, of the theme, in px or written, in the style
/// `--tw-outline-style` says; or a color.
fn outline(c: &Candidate) -> Option<Vec<Node>> {
    let Candidate::Functional {
        value, modifier, ..
    } = c
    else {
        return None;
    };
    let modifier = modifier.as_ref();
    let width = |w: &str| {
        Some(vec![
            decl("outline-style", "var(--tw-outline-style)"),
            decl("outline-width", w),
        ])
    };
    let value = match value {
        None if modifier.is_some() => return None,
        None => return width("1px"),
        Some(Value::Arbitrary { kind, value }) => {
            let kinds = ["color", "length", "number", "percentage"];
            return match kind.as_deref().or_else(|| types::infer(value, &kinds)) {
                Some("length" | "number" | "percentage") if modifier.is_none() => width(value),
                Some("length" | "number" | "percentage") => None,
                _ => Some(vec![decl("outline-color", as_color(value, modifier)?)]),
            };
        }
        Some(Value::Named { value, .. }) => value,
    };
    if let Some(c) = theme_color(value, modifier, &["--outline-color", "--color"]) {
        return Some(vec![decl("outline-color", c)]);
    }
    if modifier.is_some() {
        return None;
    }
    if let Some(w) = theme().resolve(Some(value), &["--outline-width"]) {
        return width(&w);
    }
    positive_integer(value)
        .then(|| width(&format!("{value}px")))
        .flatten()
}

/// What the shadow utilities set `box-shadow` to: each kind of shadow, `0 0 #0000` until one
/// sets it, inset ones on top.
const BOX_SHADOW: &str = "var(--tw-inset-shadow), var(--tw-inset-ring-shadow), \
                          var(--tw-ring-offset-shadow), var(--tw-ring-shadow), var(--tw-shadow)";

/// The theme's namespaces of box shadow colors.
const BOX_SHADOW_COLOR: [&str; 2] = ["--box-shadow-color", "--color"];

/// The shadow utilities: box shadows, inset shadows and text shadows, each of the theme or
/// written, and their colors; and rings, inset rings and the offsets between rings and boxes.
fn shadows(r: &mut Registry) {
    for name in ["shadow", "inset-shadow", "text-shadow"] {
        let color = format!("--tw-{name}-color");
        r.fixed(format!("{name}-initial"), &[(color.as_str(), "initial")]);
        r.add(name.into(), true, Box::new(move |c| shadow(c, name)));
    }
    r.fixed("ring-inset", &[("--tw-ring-inset", "inset")]);
    r.add("ring".into(), true, Box::new(|c| ring(c, false)));
    r.add("inset-ring".into(), true, Box::new(|c| ring(c, true)));
    r.add("ring-offset".into(), true, Box::new(ring_offset));
}

/// `shadow-*`, `inset-shadow-*` or `text-shadow-*`, as `name` says: shadows of the theme, at the
/// opacity the modifier gives, or written, or `none`; or their color.
fn shadow(c: &Candidate, name: &str) -> Option<Vec<Node>> {
    let Candidate::Functional {
        value, modifier, ..
    } = c
    else {
        return None;
    };
    let modifier = modifier.as_ref();
    let (color, alpha_variable) = (format!("--tw-{name}-color"), format!("--tw-{name}-alpha"));
    // The property the shadows go in, and what `none` sets.
    let (property, none, keys) = match name {
        "text-shadow" => ("text-shadow", "none", ["--text-shadow-color", "--color"]),
        "inset-shadow" => ("--tw-inset-shadow", "inset 0 0 #0000", BOX_SHADOW_COLOR),
        _ => ("--tw-shadow", "0 0 #0000", BOX_SHADOW_COLOR),
    };
    let boxed = |mut nodes: Vec<Node>| {
        if name != "text-shadow" {
            nodes.push(decl("box-shadow", BOX_SHADOW));
        }
        Some(nodes)
    };
    let alpha = match modifier {
        Some(Modifier::Arbitrary(a)) => Some(a.clone()),
        Some(Modifier::Named(m)) if quarter(m) => Some(format!("{m}%")),
        _ => None,
    };
    let shadows = |value: &str, prefix: &str| {
        let mut nodes: Vec<Node> = (alpha.iter())
            .map(|a| decl(&alpha_variable, a.clone()))
            .collect();
        nodes.extend(alpha_shadows(
            property,
            value,
            alpha.as_deref(),
            &color,
            prefix,
        ));
        boxed(nodes)
    };
    let colored = |c: String| {
        let alpha = format!("var({alpha_variable})");
        Some(vec![decl(&color, with_alpha(&c, &alpha))])
    };
    let value = match value {
        None => return shadows(theme().value(&format!("--{name}"))?, ""),
        Some(Value::Arbitrary { kind, value }) => {
            return match kind.as_deref().or_else(|| types::infer(value, &["color"])) {
                Some("color") => colored(as_color(value, modifier)?),
                _ if name == "inset-shadow" => shadows(value, "inset"),
                _ => shadows(value, ""),
            };
        }
        Some(Value::Named { value, .. }) => value,
    };
    match &value[..] {
        "none" | "inherit" if modifier.is_some() => None,
        "none" => boxed(vec![decl(property, none)]),
        "inherit" => Some(vec![decl(&color, "inherit")]),
        _ => match theme().value(&format!("--{name}-{value}")) {
            Some(shadows_of_theme) => shadows(shadows_of_theme, ""),
            None => colored(theme_color(value, modifier, &keys)?),
        },
    }
}

/// The declaration of `property` that sets shadows `value`, each of `prefix` and with its color
/// that of `variable` if it is set, at opacity `alpha` if one is given, as Tailwind's
/// `alphaReplacedShadowProperties`. Where the opacity, or a color, is a `var()`, the shadows at
/// their own opacities come first, for browsers without relative colors.
fn alpha_shadows(
    property: &str,
    value: &str,
    alpha: Option<&str>,
    variable: &str,
    prefix: &str,
) -> Vec<Node> {
    let colored = |c: &str| format!("var({variable}, {c})");
    let prefixed = |shadows: String| match prefix {
        "" => shadows,
        _ => (segment(&shadows, b',').iter())
            .map(|s| format!("{prefix} {}", s.trim()))
            .collect::<Vec<_>>()
            .join(", "),
    };
    let mut relative = false;
    let shadows = shadow_colors(value, &mut |c| match alpha {
        None => colored(c),
        Some(a) if c.starts_with("current") => colored(&with_alpha(c, a)),
        Some(a) => {
            relative |= c.starts_with("var(") || a.starts_with("var(");
            colored(&replace_alpha(c, a))
        }
    });
    let shadows = prefixed(shadows);
    if !relative {
        return vec![decl(property, shadows)];
    }
    let plain = prefixed(shadow_colors(value, &mut |c| colored(c)));
    let supports = "@supports (color: lab(from red l a b))".to_string();
    vec![
        decl(property, plain),
        Node::At(supports, vec![decl(property, shadows)]),
    ]
}

/// Each of the shadows `value` with its color, or `currentcolor` where it has none, made into
/// `replace` of it, as Tailwind's `replaceShadowColors`; one of fewer than two lengths, or whose
/// color it cannot tell, as it is.
fn shadow_colors(value: &str, replace: &mut dyn FnMut(&str) -> String) -> String {
    const KEYWORDS: [&str; 5] = ["inset", "inherit", "initial", "revert", "unset"];
    const MATH: [&str; 5] = ["calc", "clamp", "max", "min", "--spacing"];
    const COLORS: [&str; 15] = [
        "color",
        "color-mix",
        "contrast-color",
        "device-cmyk",
        "hsl",
        "hsla",
        "hwb",
        "lab",
        "lch",
        "light-dark",
        "oklab",
        "oklch",
        "rgb",
        "rgba",
        "--alpha",
    ];
    // A number, a length of one, as Tailwind tells them: `-?(\d+|\.\d+)` first.
    let numeric = |w: &str| {
        let w = w.strip_prefix('-').unwrap_or(w);
        let w = w.strip_prefix('.').unwrap_or(w);
        w.starts_with(|c: char| c.is_ascii_digit())
    };
    let shadows = segment(value, b',').into_iter().map(|shadow| {
        let shadow = shadow.trim();
        let mut words: Vec<String> = segment(shadow, b' ')
            .iter()
            .map(|w| w.to_string())
            .collect();
        // The color's word if it has one, how many lengths, and the words that may be its color.
        let (mut color, mut lengths, mut others) = (None, 0, Vec::new());
        for (i, w) in words.iter().enumerate() {
            let lower = w.to_ascii_lowercase();
            match lower.find('(').map(|p| &lower[..p]) {
                _ if w.is_empty() => {}
                Some(f) if COLORS.contains(&f) => {
                    color = Some(i);
                    break;
                }
                Some(f) if MATH.contains(&f) => lengths += 1,
                Some(_) => others.push(i),
                None if KEYWORDS.contains(&&lower[..]) => {}
                None if numeric(&lower) => lengths += 1,
                None if w.starts_with('#') || types::named_color(w) => {
                    color = Some(i);
                    break;
                }
                None => others.push(i),
            }
        }
        let i = match (color, &others[..]) {
            (Some(i), _) => i,
            _ if lengths < 2 => return shadow.to_string(),
            (None, []) => return format!("{shadow} {}", replace("currentcolor")),
            (None, [i]) => *i,
            _ => return shadow.to_string(),
        };
        words[i] = replace(&words[i]);
        words.join(" ")
    });
    shadows.collect::<Vec<_>>().join(", ")
}

/// `ring-*`, or `inset-ring-*` (`inset` true): a ring, 1px wide bare, of a width of the theme, in
/// px or written, in the ring color, outside the ring offset; or that color.
fn ring(c: &Candidate, inset: bool) -> Option<Vec<Node>> {
    let Candidate::Functional {
        value, modifier, ..
    } = c
    else {
        return None;
    };
    let modifier = modifier.as_ref();
    let (shadow, color) = match inset {
        true => ("--tw-inset-ring-shadow", "--tw-inset-ring-color"),
        false => ("--tw-ring-shadow", "--tw-ring-color"),
    };
    let width = |w: &str| {
        let ring = match inset {
            true => format!("inset 0 0 0 {w} var(--tw-inset-ring-color, currentcolor)"),
            false => format!(
                "var(--tw-ring-inset,) 0 0 0 calc({w} + var(--tw-ring-offset-width)) \
                 var(--tw-ring-color, currentcolor)"
            ),
        };
        Some(vec![decl(shadow, ring), decl("box-shadow", BOX_SHADOW)])
    };
    let value = match value {
        None if modifier.is_some() => return None,
        None => return width("1px"),
        Some(Value::Arbitrary { kind, value }) => {
            return match kind
                .as_deref()
                .or_else(|| types::infer(value, &["color", "length"]))
            {
                Some("length") if modifier.is_none() => width(value),
                Some("length") => None,
                _ => Some(vec![decl(color, as_color(value, modifier)?)]),
            };
        }
        Some(Value::Named { value, .. }) => value,
    };
    if let Some(c) = theme_color(value, modifier, &["--ring-color", "--color"]) {
        return Some(vec![decl(color, c)]);
    }
    if modifier.is_some() {
        return None;
    }
    if let Some(w) = theme().resolve(Some(value), &["--ring-width"]) {
        return width(&w);
    }
    positive_integer(value)
        .then(|| width(&format!("{value}px")))
        .flatten()
}

/// `ring-offset-*`: how far outside the box a ring starts, of the theme, in px or written; or the
/// color between them.
fn ring_offset(c: &Candidate) -> Option<Vec<Node>> {
    let Candidate::Functional {
        value: Some(value),
        modifier,
        ..
    } = c
    else {
        return None;
    };
    let modifier = modifier.as_ref();
    let width = |w: String| {
        let shadow = "var(--tw-ring-inset,) 0 0 0 var(--tw-ring-offset-width) \
                      var(--tw-ring-offset-color)";
        let nodes = vec![
            decl("--tw-ring-offset-width", w),
            decl("--tw-ring-offset-shadow", shadow),
        ];
        modifier.is_none().then_some(nodes)
    };
    let color = |c: String| Some(vec![decl("--tw-ring-offset-color", c)]);
    let value = match value {
        Value::Arbitrary { kind, value } => {
            return match kind
                .as_deref()
                .or_else(|| types::infer(value, &["color", "length"]))
            {
                Some("length") => width(value.clone()),
                _ => color(as_color(value, modifier)?),
            };
        }
        Value::Named { value, .. } => value,
    };
    if let Some(w) = theme().resolve(Some(value), &["--ring-offset-width"]) {
        return width(w);
    }
    if positive_integer(value) {
        return width(format!("{value}px"));
    }
    color(theme_color(
        value,
        modifier,
        &["--ring-offset-color", "--color"],
    )?)
}

/// How elements mix with what is under them: opacity, blend modes and isolation.
fn blending(r: &mut Registry) {
    const MODES: [&str; 16] = [
        "normal",
        "multiply",
        "screen",
        "overlay",
        "darken",
        "lighten",
        "color-dodge",
        "color-burn",
        "hard-light",
        "soft-light",
        "difference",
        "exclusion",
        "hue",
        "saturation",
        "color",
        "luminosity",
    ];
    r.functional(
        "opacity",
        Desc {
            keys: &["--opacity"],
            value: Some(|v, _| quarter(v).then(|| format!("{v}%"))),
            handle: Some(one("opacity")),
            ..Desc::default()
        },
    );
    for mode in MODES {
        r.fixed(
            format!("bg-blend-{mode}"),
            &[("background-blend-mode", mode)],
        );
        r.fixed(format!("mix-blend-{mode}"), &[("mix-blend-mode", mode)]);
    }
    for mode in ["plus-darker", "plus-lighter"] {
        r.fixed(format!("mix-blend-{mode}"), &[("mix-blend-mode", mode)]);
    }
    r.fixed("isolate", &[("isolation", "isolate")]);
    r.fixed("isolation-auto", &[("isolation", "auto")]);
}

/// What `transform` is set to by what rotates and skews on one axis, each `var()` empty until a
/// class sets it.
const TRANSFORM: &str = "var(--tw-rotate-x,) var(--tw-rotate-y,) var(--tw-rotate-z,) \
                         var(--tw-skew-x,) var(--tw-skew-y,)";

/// The transform utilities: origins, perspective, the `translate`, `scale` and `rotate`
/// properties and each of their axes, skews, and how the 3D space is kept.
fn transforms(r: &mut Registry) {
    const PLACES: [(&str, &str); 9] = [
        ("center", "center"),
        ("top", "top"),
        ("top-right", "100% 0"),
        ("right", "100%"),
        ("bottom-right", "100% 100%"),
        ("bottom", "bottom"),
        ("bottom-left", "0 100%"),
        ("left", "0"),
        ("top-left", "0 0"),
    ];
    const XY: &str = "var(--tw-translate-x) var(--tw-translate-y)";
    const XYZ: &str = "var(--tw-translate-x) var(--tw-translate-y) var(--tw-translate-z)";
    let places = |property: &str| {
        (PLACES.iter())
            .map(|&(name, value)| (name, vec![decl(property, value)]))
            .collect()
    };
    r.functional(
        "origin",
        Desc {
            keys: &["--transform-origin"],
            statics: places("transform-origin"),
            handle: Some(one("transform-origin")),
            ..Desc::default()
        },
    );
    r.functional(
        "perspective-origin",
        Desc {
            keys: &["--perspective-origin"],
            statics: places("perspective-origin"),
            handle: Some(one("perspective-origin")),
            ..Desc::default()
        },
    );
    r.functional(
        "perspective",
        Desc {
            keys: &["--perspective"],
            statics: vec![("none", vec![decl("perspective", "none")])],
            handle: Some(one("perspective")),
            ..Desc::default()
        },
    );

    r.fixed("translate-none", &[("translate", "none")]);
    for (sign, full) in [("-", "-100%"), ("", "100%")] {
        r.fixed(
            format!("{sign}translate-full"),
            &[
                ("--tw-translate-x", full),
                ("--tw-translate-y", full),
                ("translate", XY),
            ],
        );
    }
    let keys = &["--translate", "--spacing"];
    let handle: Handle = Arc::new(|v, _| {
        Some(vec![
            decl("--tw-translate-x", v),
            decl("--tw-translate-y", v),
            decl("translate", XY),
        ])
    });
    r.spacing("translate", keys, handle, true, true);
    for axis in ["x", "y"] {
        let variable = format!("--tw-translate-{axis}");
        for (sign, full) in [("-", "-100%"), ("", "100%")] {
            r.fixed(
                format!("{sign}translate-{axis}-full"),
                &[(variable.as_str(), full), ("translate", XY)],
            );
        }
        let handle: Handle =
            Arc::new(move |v, _| Some(vec![decl(&variable, v), decl("translate", XY)]));
        r.spacing(&format!("translate-{axis}"), keys, handle, true, true);
    }
    let handle: Handle =
        Arc::new(|v, _| Some(vec![decl("--tw-translate-z", v), decl("translate", XYZ)]));
    r.spacing("translate-z", keys, handle, true, false);
    r.fixed("translate-3d", &[("translate", XYZ)]);

    r.fixed("scale-none", &[("scale", "none")]);
    r.add("-scale".into(), true, Box::new(|c| scale(c, true)));
    r.add("scale".into(), true, Box::new(|c| scale(c, false)));
    for axis in ["x", "y", "z"] {
        let last = if axis == "z" {
            " var(--tw-scale-z)"
        } else {
            ""
        };
        let variable = format!("--tw-scale-{axis}");
        let scales = format!("var(--tw-scale-x) var(--tw-scale-y){last}");
        r.functional(
            &format!("scale-{axis}"),
            Desc {
                negative: true,
                keys: &["--scale"],
                value: Some(percentage),
                handle: Some(Arc::new(move |v, _| {
                    Some(vec![decl(&variable, v), decl("scale", &scales[..])])
                })),
                ..Desc::default()
            },
        );
    }
    let scales = "var(--tw-scale-x) var(--tw-scale-y) var(--tw-scale-z)";
    r.fixed("scale-3d", &[("scale", scales)]);

    r.fixed("rotate-none", &[("rotate", "none")]);
    r.add("-rotate".into(), true, Box::new(|c| rotate(c, true)));
    r.add("rotate".into(), true, Box::new(|c| rotate(c, false)));
    for (axis, function) in [("x", "rotateX"), ("y", "rotateY"), ("z", "rotateZ")] {
        let variable = format!("--tw-rotate-{axis}");
        r.functional(
            &format!("rotate-{axis}"),
            Desc {
                negative: true,
                keys: &["--rotate"],
                value: Some(whole_degrees),
                handle: Some(Arc::new(move |v, _| {
                    Some(vec![
                        decl(&variable, format!("{function}({v})")),
                        decl("transform", TRANSFORM),
                    ])
                })),
                ..Desc::default()
            },
        );
    }
    for (name, x, y) in [
        ("skew", true, true),
        ("skew-x", true, false),
        ("skew-y", false, true),
    ] {
        r.functional(
            name,
            Desc {
                negative: true,
                keys: &["--skew"],
                value: Some(whole_degrees),
                handle: Some(Arc::new(move |v, _| {
                    let mut nodes = Vec::new();
                    if x {
                        nodes.push(decl("--tw-skew-x", format!("skewX({v})")));
                    }
                    if y {
                        nodes.push(decl("--tw-skew-y", format!("skewY({v})")));
                    }
                    nodes.push(decl("transform", TRANSFORM));
                    Some(nodes)
                })),
                ..Desc::default()
            },
        );
    }
    r.add("transform".into(), true, Box::new(transform));
    r.fixed("transform-cpu", &[("transform", TRANSFORM)]);
    let gpu = format!("translateZ(0) {TRANSFORM}");
    r.fixed("transform-gpu", &[("transform", gpu.as_str())]);
    r.fixed("transform-none", &[("transform", "none")]);
    r.fixed("transform-flat", &[("transform-style", "flat")]);
    r.fixed("transform-3d", &[("transform-style", "preserve-3d")]);
    for (name, value) in [
        ("content", "content-box"),
        ("border", "border-box"),
        ("fill", "fill-box"),
        ("stroke", "stroke-box"),
        ("view", "view-box"),
    ] {
        r.fixed(format!("transform-{name}"), &[("transform-box", value)]);
    }
    r.fixed("backface-visible", &[("backface-visibility", "visible")]);
    r.fixed("backface-hidden", &[("backface-visibility", "hidden")]);
}

/// A whole number as a percentage.
fn percentage(v: &str, _: Option<&str>) -> Option<String> {
    positive_integer(v).then(|| format!("{v}%"))
}

/// A whole number as an angle.
fn whole_degrees(v: &str, _: Option<&str>) -> Option<String> {
    positive_integer(v).then(|| format!("{v}deg"))
}

/// `scale-*` or `-scale-*`: of the theme or in percent on each axis, or written as the property.
fn scale(c: &Candidate, negative: bool) -> Option<Vec<Node>> {
    let Candidate::Functional {
        value: Some(value),
        modifier: None,
        ..
    } = c
    else {
        return None;
    };
    let signed = |v: String| match negative {
        true => format!("calc({v} * -1)"),
        false => v,
    };
    let value = match value {
        Value::Arbitrary { value, .. } => return Some(vec![decl("scale", signed(value.clone()))]),
        Value::Named { value, .. } => value,
    };
    let scale = theme().resolve(Some(value), &["--scale"]);
    let scale = signed(scale.or_else(|| percentage(value, None))?);
    Some(vec![
        decl("--tw-scale-x", &scale[..]),
        decl("--tw-scale-y", &scale[..]),
        decl("--tw-scale-z", &scale[..]),
        decl("scale", "var(--tw-scale-x) var(--tw-scale-y)"),
    ])
}

/// `rotate-*` or `-rotate-*`: an angle of the theme, in degrees or written. Tailwind's written
/// axis reads a variable nothing sets, so it is not taken.
fn rotate(c: &Candidate, negative: bool) -> Option<Vec<Node>> {
    let Candidate::Functional {
        value: Some(value),
        modifier: None,
        ..
    } = c
    else {
        return None;
    };
    let angle = match value {
        Value::Arbitrary { kind, value } => {
            let kinds = ["angle", "vector"];
            match kind.as_deref().or_else(|| types::infer(value, &kinds)) {
                Some("vector") => return None,
                _ => value.clone(),
            }
        }
        Value::Named { value, .. } => {
            let angle = theme().resolve(Some(value), &["--rotate"]);
            angle.or_else(|| whole_degrees(value, None))?
        }
    };
    let angle = match negative {
        true => format!("calc({angle} * -1)"),
        false => angle,
    };
    Some(vec![decl("rotate", angle)])
}

/// `transform`, what the classes of its axes set, or `transform-[…]`, written.
fn transform(c: &Candidate) -> Option<Vec<Node>> {
    match c {
        Candidate::Functional {
            value: None,
            modifier: None,
            ..
        } => Some(vec![decl("transform", TRANSFORM)]),
        Candidate::Functional {
            value: Some(Value::Arbitrary { value, .. }),
            modifier: None,
            ..
        } => Some(vec![decl("transform", value.clone())]),
        _ => None,
    }
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
