//! The utilities, as Tailwind's `utilities.ts` registers them (MIT, see THIRD_PARTY.md): each
//! makes CSS of the candidates that are its own, and nothing of the rest. Families come over one
//! at a time; a class of one that has not is not supported yet.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use super::{CHILDREN, Candidate, Modifier, Node, Value};
use super::{dimension, positive_integer, quarter, spaced, theme};

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
            value: Some(|v, _| positive_integer(v).then(|| v.to_string())),
            statics: vec![("auto", vec![decl("z-index", "auto")])],
            handle: Some(one("z-index")),
            ..Desc::default()
        },
    );
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
                || dimension(v).is_some_and(|(n, unit)| n == 0.0 && (unit.is_empty() || length(v)));
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

/// Whether `v` is a length, as Tailwind's `isLength` has it.
fn length(v: &str) -> bool {
    const UNITS: [&str; 31] = [
        "cm", "mm", "Q", "in", "pc", "pt", "px", "em", "ex", "ch", "rem", "lh", "rlh", "vw", "vh",
        "vmin", "vmax", "vb", "vi", "svw", "svh", "lvw", "lvh", "dvw", "dvh", "cqw", "cqh", "cqi",
        "cqb", "cqmin", "cqmax",
    ];
    dimension(v).is_some_and(|(_, unit)| UNITS.contains(&unit))
        || v.starts_with("--spacing(")
        || super::MATH.iter().any(|f| v.contains(&format!("{f}(")))
}
