//! Tailwind classes, as the Tailwind v4 compiler of `package.json` expands them with its default
//! theme. A class is CSS: declarations on the element, some of them `--tw-*` custom properties
//! that other declarations read, and declarations for its children; an element's classes
//! cascade in the order Tailwind's stylesheets give them. Their `var()`s resolve on each element:
//! its own custom properties, then the theme's, then the initial values Tailwind registers, then
//! the fallbacks. What comes out is plain declarations, which the style layer takes as it takes a
//! style's: before the inline style, or after it for an important class (`p-4!`).
//!
//! The utilities are Tailwind's (MIT, see THIRD_PARTY.md), family by family, and the case runner
//! compares each class's CSS with what the compiler makes of it. Variants, CSS variables and the
//! names Tailwind v4 keeps from v3 are errors that say what to write instead. So is a class whose
//! CSS the style layer does not take, as its declarations resolve on an element of it alone: it
//! fails where it is written, with the style layer's reason.

mod order;
mod types;
mod utilities;

use std::cmp::Ordering;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::LazyLock;

use crate::js;
use crate::template::{Error, Template};

/// A node of a class's CSS, as Tailwind's syntax tree has it.
#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    /// `property: value`. A `--tw-sort` declaration only sorts the class, as if it were that
    /// property.
    Declaration(String, String),
    /// A rule in the class's: its selector, `&` the element, and what it holds.
    Rule(&'static str, Vec<Node>),
    /// An at-rule, `@media (width >= 40rem)` say, and what it holds.
    At(String, Vec<Node>),
}

/// The selector of the rule for the element's children but the last.
pub const CHILDREN: &str = ":where(& > :not(:last-child))";

/// A class: its CSS, once for each utility it names, and whether it is important.
#[derive(Clone, Debug, PartialEq)]
pub struct Class {
    pub name: Rc<str>,
    pub important: bool,
    pub rules: Vec<Rule>,
}

/// One utility's CSS, and where Tailwind sorts it.
#[derive(Clone, Debug, PartialEq)]
pub struct Rule {
    pub nodes: Vec<Node>,
    sort: Sort,
}

/// What Tailwind sorts a rule by: the places in [`order::ORDER`] of the properties it sets, in
/// order, and how many declarations it has.
#[derive(Clone, Debug, PartialEq)]
struct Sort {
    order: Vec<usize>,
    count: usize,
}

/// A declaration of an element's classes, its `var()`s resolved: its property as CSS names it,
/// its value, whether it is important, and the class it is from.
#[derive(Clone, Debug, PartialEq)]
pub struct Declaration {
    pub property: String,
    pub value: String,
    pub important: bool,
    pub class: Rc<str>,
}

/// The custom properties Tailwind registers that the utilities here read, and their initial
/// values; one without is invalid until a class sets it.
pub const PROPERTIES: &[(&str, Option<&str>)] = &[
    ("--tw-space-x-reverse", Some("0")),
    ("--tw-space-y-reverse", Some("0")),
    ("--tw-font-weight", None),
    ("--tw-leading", None),
    ("--tw-tracking", None),
    ("--tw-ordinal", None),
    ("--tw-slashed-zero", None),
    ("--tw-numeric-figure", None),
    ("--tw-numeric-spacing", None),
    ("--tw-numeric-fraction", None),
    ("--tw-gradient-position", None),
    ("--tw-gradient-from", Some("#0000")),
    ("--tw-gradient-via", Some("#0000")),
    ("--tw-gradient-to", Some("#0000")),
    ("--tw-gradient-stops", None),
    ("--tw-gradient-via-stops", None),
    ("--tw-gradient-from-position", Some("0%")),
    ("--tw-gradient-via-position", Some("50%")),
    ("--tw-gradient-to-position", Some("100%")),
    ("--tw-border-style", Some("solid")),
    ("--tw-divide-x-reverse", Some("0")),
    ("--tw-divide-y-reverse", Some("0")),
    ("--tw-outline-style", Some("solid")),
    ("--tw-shadow", Some("0 0 #0000")),
    ("--tw-shadow-color", None),
    ("--tw-shadow-alpha", Some("100%")),
    ("--tw-inset-shadow", Some("0 0 #0000")),
    ("--tw-inset-shadow-color", None),
    ("--tw-inset-shadow-alpha", Some("100%")),
    ("--tw-ring-color", None),
    ("--tw-ring-shadow", Some("0 0 #0000")),
    ("--tw-inset-ring-color", None),
    ("--tw-inset-ring-shadow", Some("0 0 #0000")),
    ("--tw-ring-inset", None),
    ("--tw-ring-offset-width", Some("0px")),
    ("--tw-ring-offset-color", Some("#fff")),
    ("--tw-ring-offset-shadow", Some("0 0 #0000")),
    ("--tw-text-shadow-color", None),
    ("--tw-text-shadow-alpha", Some("100%")),
    ("--tw-translate-x", Some("0")),
    ("--tw-translate-y", Some("0")),
    ("--tw-translate-z", Some("0")),
    ("--tw-scale-x", Some("1")),
    ("--tw-scale-y", Some("1")),
    ("--tw-scale-z", Some("1")),
    ("--tw-rotate-x", None),
    ("--tw-rotate-y", None),
    ("--tw-rotate-z", None),
    ("--tw-skew-x", None),
    ("--tw-skew-y", None),
    ("--tw-blur", None),
    ("--tw-brightness", None),
    ("--tw-contrast", None),
    ("--tw-grayscale", None),
    ("--tw-hue-rotate", None),
    ("--tw-invert", None),
    ("--tw-saturate", None),
    ("--tw-sepia", None),
    ("--tw-drop-shadow", None),
    ("--tw-drop-shadow-color", None),
    ("--tw-drop-shadow-alpha", Some("100%")),
    ("--tw-drop-shadow-size", None),
    ("--tw-backdrop-blur", None),
    ("--tw-backdrop-brightness", None),
    ("--tw-backdrop-contrast", None),
    ("--tw-backdrop-grayscale", None),
    ("--tw-backdrop-hue-rotate", None),
    ("--tw-backdrop-invert", None),
    ("--tw-backdrop-opacity", None),
    ("--tw-backdrop-saturate", None),
    ("--tw-backdrop-sepia", None),
    ("--tw-mask-linear", Some("linear-gradient(#fff, #fff)")),
    ("--tw-mask-radial", Some("linear-gradient(#fff, #fff)")),
    ("--tw-mask-conic", Some("linear-gradient(#fff, #fff)")),
    ("--tw-mask-left", Some("linear-gradient(#fff, #fff)")),
    ("--tw-mask-right", Some("linear-gradient(#fff, #fff)")),
    ("--tw-mask-bottom", Some("linear-gradient(#fff, #fff)")),
    ("--tw-mask-top", Some("linear-gradient(#fff, #fff)")),
    ("--tw-mask-top-from-position", Some("0%")),
    ("--tw-mask-top-to-position", Some("100%")),
    ("--tw-mask-top-from-color", Some("black")),
    ("--tw-mask-top-to-color", Some("transparent")),
    ("--tw-mask-right-from-position", Some("0%")),
    ("--tw-mask-right-to-position", Some("100%")),
    ("--tw-mask-right-from-color", Some("black")),
    ("--tw-mask-right-to-color", Some("transparent")),
    ("--tw-mask-bottom-from-position", Some("0%")),
    ("--tw-mask-bottom-to-position", Some("100%")),
    ("--tw-mask-bottom-from-color", Some("black")),
    ("--tw-mask-bottom-to-color", Some("transparent")),
    ("--tw-mask-left-from-position", Some("0%")),
    ("--tw-mask-left-to-position", Some("100%")),
    ("--tw-mask-left-from-color", Some("black")),
    ("--tw-mask-left-to-color", Some("transparent")),
    ("--tw-mask-linear-position", Some("0deg")),
    ("--tw-mask-linear-from-position", Some("0%")),
    ("--tw-mask-linear-to-position", Some("100%")),
    ("--tw-mask-linear-from-color", Some("black")),
    ("--tw-mask-linear-to-color", Some("transparent")),
    ("--tw-mask-radial-from-position", Some("0%")),
    ("--tw-mask-radial-to-position", Some("100%")),
    ("--tw-mask-radial-from-color", Some("black")),
    ("--tw-mask-radial-to-color", Some("transparent")),
    ("--tw-mask-radial-shape", Some("ellipse")),
    ("--tw-mask-radial-size", Some("farthest-corner")),
    ("--tw-mask-radial-position", Some("center")),
    ("--tw-mask-conic-position", Some("0deg")),
    ("--tw-mask-conic-from-position", Some("0%")),
    ("--tw-mask-conic-to-position", Some("100%")),
    ("--tw-mask-conic-from-color", Some("black")),
    ("--tw-mask-conic-to-color", Some("transparent")),
];

/// What Tailwind writes for a browser that no canvas is: Safari's name of `backdrop-filter`
/// beside it, and font smoothing, which is always grayscale here.
const BESIDE: [&str; 3] = [
    "-webkit-backdrop-filter",
    "-webkit-font-smoothing",
    "-moz-osx-font-smoothing",
];

/// The variables Tailwind reads without registering them, which have no value until set.
const UNREGISTERED: [&str; 3] = [
    "--tw-mask-linear-stops",
    "--tw-mask-radial-stops",
    "--tw-mask-conic-stops",
];

const VARIANTS: &str = "variants are not supported: the canvas has a fixed size, and nothing \
                        hovers, focuses or switches themes; choose the classes with a condition, \
                        as in `${big ? \"text-6xl\" : \"text-4xl\"}`";
const VARIABLES: &str = "CSS variables are not supported: name the value with a const";

/// What `name` expands to, as Tailwind reads it, or why it does not.
pub fn class(name: &str) -> Result<Class, String> {
    let unknown = || format!("className {name} is not a Tailwind class, or not supported yet");
    if segment(name, b':').len() > 1 {
        return Err(format!("className {name}: {VARIANTS}"));
    }
    let (base, important) = match name.strip_suffix('!') {
        Some(base) => (base, true),
        None if name.starts_with('!') => {
            return Err(format!(
                "className {name}: Tailwind v4 marks a class important at its end: write {}!",
                &name[1..]
            ));
        }
        None => (name, false),
    };
    if let Some(v4) = legacy(base) {
        return Err(format!(
            "className {name} is a Tailwind v3 name: write {v4}"
        ));
    }
    let candidates = candidates(base).map_err(|e| format!("className {name}: {e}"))?;
    let mut rules = Vec::new();
    for c in &candidates {
        for nodes in utilities::compile(c) {
            check(&nodes).map_err(|e| format!("className {name}: {e}"))?;
            let sort = Sort::of(&nodes);
            rules.push(Rule { nodes, sort });
        }
    }
    if rules.is_empty() {
        return Err(unknown());
    }
    rules.sort_by(|a, b| a.sort.compare(&b.sort));
    let class = Class {
        name: name.into(),
        important,
        rules,
    };
    // What the engine cannot draw fails where the class is written, not when a frame takes it:
    // its declarations as an element of it alone has them, and its children. One that reads what
    // other classes set, a gradient's color stops say, is checked where the classes meet.
    let one = std::slice::from_ref(&class);
    for d in element(one, &[])?.iter().chain(&element(&[], one)?) {
        let what = format!("className {name}: {}", d.property);
        if let Err(e) = super::property::accepts(&what, &d.property, &d.value)
            && !open(&class, &d.property)
        {
            return Err(e);
        }
    }
    Ok(class)
}

/// Whether a class's `property` reads a variable of Tailwind's that the class leaves to others.
fn open(class: &Class, property: &str) -> bool {
    fn walk<'a>(nodes: &'a [Node], out: &mut Vec<(&'a str, &'a str)>) {
        for n in nodes {
            match n {
                Node::Declaration(p, v) => out.push((p, v)),
                Node::Rule(_, inner) | Node::At(_, inner) => walk(inner, out),
            }
        }
    }
    fn reads(value: &str, set: &[(&str, &str)], depth: usize) -> bool {
        value.match_indices("var(--tw-").any(|(i, _)| {
            let rest = &value[i + "var(".len()..];
            let name = &rest[..rest.find([',', ')']).unwrap_or(rest.len())];
            let values: Vec<&str> = (set.iter())
                .filter(|(p, _)| *p == name.trim())
                .map(|(_, v)| *v)
                .collect();
            values.is_empty() || (depth < 8 && values.iter().any(|v| reads(v, set, depth + 1)))
        })
    }
    let mut set = Vec::new();
    for r in &class.rules {
        walk(&r.nodes, &mut set);
    }
    (set.iter())
        .filter(|(p, _)| *p == property)
        .any(|(_, v)| reads(v, &set, 0))
}

/// That a class's CSS holds only what the engine applies: declarations on the element and on
/// its children but the last.
fn check(nodes: &[Node]) -> Result<(), String> {
    for n in nodes {
        match n {
            Node::Declaration(..) => {}
            Node::Rule(CHILDREN, inner)
                if inner.iter().all(|n| matches!(n, Node::Declaration(..))) => {}
            Node::Rule(selector, _) => {
                return Err(format!(
                    "its CSS styles {selector}, which is not supported yet"
                ));
            }
            // Forced colors never apply: the canvas has no such mode.
            Node::At(rule, inner)
                if (rule.starts_with("@supports (")
                    || rule == "@media (forced-colors: active)")
                    && inner.iter().all(|n| matches!(n, Node::Declaration(..))) => {}
            Node::At(rule, _) => {
                let at = rule.split_whitespace().next().unwrap_or_default();
                return Err(format!(
                    "its CSS has {at} rules, which do not apply: the canvas has a fixed size"
                ));
            }
        }
    }
    Ok(())
}

/// Whether `a` comes before `b` in Tailwind's stylesheets, by their first rules.
pub fn compare(a: &Class, b: &Class) -> Ordering {
    match (a.rules.first(), b.rules.first()) {
        (Some(x), Some(y)) => x
            .sort
            .compare(&y.sort)
            .then_with(|| natural(&a.name, &b.name)),
        _ => natural(&a.name, &b.name),
    }
}

impl Sort {
    /// As Tailwind's `getPropertySort`: breadth first, a `--tw-sort` standing for the rest.
    fn of(nodes: &[Node]) -> Sort {
        let mut order = Vec::new();
        let mut count = 0;
        let mut sorted = false;
        let mut queue: std::collections::VecDeque<&Node> = nodes.iter().collect();
        while let Some(n) = queue.pop_front() {
            match n {
                Node::Declaration(property, value) => {
                    count += 1;
                    if sorted {
                        continue;
                    }
                    if property == "--tw-sort"
                        && let Some(i) = place(value)
                    {
                        order.push(i);
                        sorted = true;
                        continue;
                    }
                    order.extend(place(property));
                }
                Node::Rule(_, inner) | Node::At(_, inner) => queue.extend(inner),
            }
        }
        order.sort_unstable();
        order.dedup();
        Sort { order, count }
    }

    /// By the first property that differs, none coming last, then by the most declarations.
    fn compare(&self, other: &Sort) -> Ordering {
        let first = (self.order.iter().zip(&other.order)).position(|(a, b)| a != b);
        let i = first.unwrap_or(self.order.len().min(other.order.len()));
        let at = |o: &[usize]| o.get(i).copied().unwrap_or(usize::MAX);
        (at(&self.order).cmp(&at(&other.order))).then(other.count.cmp(&self.count))
    }
}

/// Where `property` is in Tailwind's property order.
fn place(property: &str) -> Option<usize> {
    static PLACES: LazyLock<HashMap<&str, usize>> = LazyLock::new(|| {
        (order::ORDER.iter().enumerate())
            .map(|(i, p)| (*p, i))
            .collect()
    });
    PLACES.get(property).copied()
}

/// Strings compared as Tailwind does, runs of digits by their numbers.
fn natural(a: &str, b: &str) -> Ordering {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if a[i].is_ascii_digit() && b[j].is_ascii_digit() {
            let run =
                |s: &[u8], k: usize| k + s[k..].iter().take_while(|c| c.is_ascii_digit()).count();
            let (x, y) = (run(a, i), run(b, j));
            let (m, n) = (&a[i..x], &b[j..y]);
            let number = |d: &[u8]| {
                std::str::from_utf8(d)
                    .ok()
                    .and_then(|d| d.parse::<f64>().ok())
            };
            let by = (number(m).partial_cmp(&number(n))).unwrap_or(Ordering::Equal);
            match by.then(m.cmp(n)) {
                Ordering::Equal => (i, j) = (x, y),
                o => return o,
            }
            continue;
        }
        if a[i] != b[j] {
            return a[i].cmp(&b[j]);
        }
        (i, j) = (i + 1, j + 1);
    }
    (a.len() - i).cmp(&(b.len() - j))
}

/// The v4 spelling of a name Tailwind v4 keeps from v3, if `base` is one.
fn legacy(base: &str) -> Option<String> {
    const RENAMED: [(&str, &str); 22] = [
        ("bg-left-top", "bg-top-left"),
        ("bg-right-top", "bg-top-right"),
        ("bg-left-bottom", "bg-bottom-left"),
        ("bg-right-bottom", "bg-bottom-right"),
        ("object-left-top", "object-top-left"),
        ("object-right-top", "object-top-right"),
        ("object-left-bottom", "object-bottom-left"),
        ("object-right-bottom", "object-bottom-right"),
        ("overflow-ellipsis", "text-ellipsis"),
        ("decoration-slice", "box-decoration-slice"),
        ("decoration-clone", "box-decoration-clone"),
        ("order-none", "order-0"),
        ("break-words", "wrap-break-word"),
        ("max-w-prose", "max-w-[65ch]"),
        // The theme's deprecated values, which v4 names with a size.
        ("shadow", "shadow-sm"),
        ("shadow-inner", "inset-shadow-sm"),
        ("drop-shadow", "drop-shadow-sm"),
        ("blur", "blur-sm"),
        ("backdrop-blur", "backdrop-blur-sm"),
        ("rounded", "rounded-sm"),
        ("flex-grow", "grow"),
        ("flex-shrink", "shrink"),
    ];
    const SIDES: [&str; 8] = ["t", "tr", "r", "br", "b", "bl", "l", "tl"];
    const CORNERS: [&str; 14] = [
        "s", "e", "t", "r", "b", "l", "ss", "se", "ee", "es", "tl", "tr", "br", "bl",
    ];
    if let Some(&(_, v4)) = RENAMED.iter().find(|(v3, _)| *v3 == base) {
        return Some(v4.into());
    }
    if let Some(side) = base.strip_prefix("bg-gradient-to-")
        && SIDES.contains(&side)
    {
        return Some(format!("bg-linear-to-{side}"));
    }
    if let Some(corner) = base.strip_prefix("rounded-")
        && CORNERS.contains(&corner)
    {
        return Some(format!("{base}-sm"));
    }
    if let Some(size) = base.strip_prefix("max-w-screen-") {
        let width = THEME.value(&format!("--breakpoint-{size}"))?;
        return Some(format!("max-w-[{width}]"));
    }
    // Opacities are the colors' own: bg-black/50.
    for color in ["bg", "text", "border", "divide", "ring", "placeholder"] {
        if let Some(opacity) = base.strip_prefix(&format!("{color}-opacity-"))
            && !opacity.is_empty()
        {
            return Some(format!("{color}-<color>/{opacity}"));
        }
    }
    for (v3, v4) in [
        ("flex-grow-", "grow-"),
        ("flex-shrink-", "shrink-"),
        ("start-", "inset-s-"),
        ("-start-", "-inset-s-"),
        ("end-", "inset-e-"),
        ("-end-", "-inset-e-"),
    ] {
        if let Some(rest) = base.strip_prefix(v3) {
            return Some(format!("{v4}{rest}"));
        }
    }
    None
}

// ---------------------------------------------------------------------------------- candidates

/// A class as Tailwind reads it, without its `!`.
#[derive(Clone, Debug, PartialEq)]
enum Candidate<'a> {
    /// A utility of no value, `flex`.
    Static(&'a str),
    /// A utility of a root and maybe a value and a modifier, `p-4`, `w-1/2`, `p-[3px]`.
    Functional {
        root: &'a str,
        value: Option<Value>,
        modifier: Option<Modifier>,
    },
    /// A declaration, `[mask-type:luminance]`.
    Arbitrary {
        property: &'a str,
        value: String,
        modifier: Option<Modifier>,
    },
}

#[derive(Clone, Debug, PartialEq)]
enum Value {
    /// `4` of `p-4`; `1` of `w-1/2`, which is the fraction `1/2`.
    Named {
        value: String,
        fraction: Option<String>,
    },
    /// `3px` of `p-[3px]`, which may say what it is, as `length:` in `text-[length:2em]`.
    Arbitrary { kind: Option<String>, value: String },
}

#[derive(Clone, Debug, PartialEq)]
enum Modifier {
    Named(String),
    Arbitrary(String),
}

/// Values and modifiers Tailwind takes as written, without brackets.
fn named(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_.%-".contains(&c))
}

/// The ways Tailwind reads `base`, a class without its `!`: as a static utility, as an arbitrary
/// property, or as a functional utility of each root it starts with; or why it is not a class.
fn candidates(base: &str) -> Result<Vec<Candidate<'_>>, String> {
    let mut out = Vec::new();
    if utilities::has(base, false) && !base.contains('[') {
        out.push(Candidate::Static(base));
    }
    let parts = segment(base, b'/');
    let (base, modifier_text) = match parts[..] {
        [base] => (base, None),
        [base, modifier] => (base, Some(modifier)),
        _ => return Ok(out),
    };
    let modifier = match modifier_text {
        Some(m) if m.starts_with('(') && m.ends_with(')') => return Err(VARIABLES.into()),
        Some(m) => match parse_modifier(m) {
            Some(m) => Some(m),
            None => return Ok(out),
        },
        None => None,
    };
    if let Some(inside) = base.strip_prefix('[') {
        let Some(inside) = inside.strip_suffix(']') else {
            return Ok(out);
        };
        if !inside.starts_with(|c: char| c == '-' || c.is_ascii_lowercase()) {
            return Ok(out);
        }
        let Some((property, value)) = inside.split_once(':') else {
            return Ok(out);
        };
        if property.is_empty() || value.is_empty() {
            return Ok(out);
        }
        if property.starts_with("--") {
            return Err(VARIABLES.into());
        }
        let value = decode(value);
        if valid(&value) {
            out.push(Candidate::Arbitrary {
                property,
                value,
                modifier,
            });
        }
        return Ok(out);
    }
    let roots: Vec<(&str, Option<&str>)> = if base.ends_with(']') {
        let Some(i) = base.find("-[") else {
            return Ok(out);
        };
        if !utilities::has(&base[..i], true) {
            return Ok(out);
        }
        vec![(&base[..i], Some(&base[i + 1..]))]
    } else if base.ends_with(')') && base.contains("-(") {
        return Err(VARIABLES.into());
    } else {
        roots(base)
    };
    for (root, value) in roots {
        let Some(value) = value else {
            out.push(Candidate::Functional {
                root,
                value: None,
                modifier: modifier.clone(),
            });
            continue;
        };
        let value = if let Some(start) = value.find('[') {
            let Some(inside) = value[start + 1..].strip_suffix(']') else {
                return Ok(out);
            };
            let mut decoded = decode(inside);
            if !valid(&decoded) {
                continue;
            }
            // A kind first, as `length:`.
            let mut kind = None;
            let end = decoded.find(|c: char| c != '-' && !c.is_ascii_lowercase());
            if let Some(i) = end.filter(|&i| decoded.as_bytes()[i] == b':') {
                if i == 0 {
                    continue;
                }
                kind = Some(decoded[..i].to_string());
                decoded = decoded[i + 1..].to_string();
            }
            if decoded.trim().is_empty() {
                continue;
            }
            Value::Arbitrary {
                kind,
                value: decoded,
            }
        } else {
            if !named(value) {
                continue;
            }
            let fraction = match (&modifier, modifier_text) {
                (Some(Modifier::Named(_)), Some(m)) => Some(format!("{value}/{m}")),
                _ => None,
            };
            Value::Named {
                value: value.into(),
                fraction,
            }
        };
        out.push(Candidate::Functional {
            root,
            value: Some(value),
            modifier: modifier.clone(),
        });
    }
    Ok(out)
}

fn parse_modifier(m: &str) -> Option<Modifier> {
    if let Some(inside) = m.strip_prefix('[').and_then(|m| m.strip_suffix(']')) {
        let decoded = decode(inside);
        return (valid(&decoded) && !decoded.trim().is_empty())
            .then_some(Modifier::Arbitrary(decoded));
    }
    named(m).then(|| Modifier::Named(m.into()))
}

/// The functional roots `input` may start with, and the value after each, as Tailwind's
/// `findRoots` tries them: the whole of it, then up to each dash from the last.
fn roots(input: &str) -> Vec<(&str, Option<&str>)> {
    let mut out = Vec::new();
    if utilities::has(input, true) {
        out.push((input, None));
    }
    let dashes = input.match_indices('-').map(|(i, _)| i).filter(|&i| i > 0);
    for i in dashes.collect::<Vec<_>>().into_iter().rev() {
        if utilities::has(&input[..i], true) {
            let value = &input[i + 1..];
            if value.is_empty() {
                break;
            }
            out.push((&input[..i], Some(value)));
        }
    }
    out
}

// -------------------------------------------------------------------------------------- values

/// `input` split at `separator` outside brackets, parentheses, braces and quotes, as Tailwind's
/// `segment`.
fn segment(input: &str, separator: u8) -> Vec<&str> {
    let b = input.as_bytes();
    let mut parts = Vec::new();
    let mut closing = Vec::new();
    let (mut last, mut i) = (0, 0);
    while i < b.len() {
        let c = b[i];
        if closing.is_empty() && c == separator {
            parts.push(&input[last..i]);
            last = i + 1;
            i += 1;
            continue;
        }
        match c {
            b'\\' => i += 1,
            b'"' | b'\'' => {
                i += 1;
                while i < b.len() && b[i] != c {
                    i += 1 + usize::from(b[i] == b'\\');
                }
            }
            b'(' => closing.push(b')'),
            b'[' => closing.push(b']'),
            b'{' => closing.push(b'}'),
            b')' | b']' | b'}' if closing.last() == Some(&c) => {
                closing.pop();
            }
            _ => {}
        }
        i += 1;
    }
    parts.push(&input[last..]);
    parts
}

/// Whether an arbitrary value is one: no `;` or unbalanced brackets outside quotes, as
/// Tailwind's `isValidArbitrary`.
fn valid(input: &str) -> bool {
    let b = input.as_bytes();
    let mut closing = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        match c {
            b'\\' => i += 1,
            b'"' | b'\'' => {
                i += 1;
                while i < b.len() && b[i] != c {
                    i += 1 + usize::from(b[i] == b'\\');
                }
            }
            b'(' => closing.push(b')'),
            b'[' => closing.push(b']'),
            b']' | b'}' | b')' => {
                if closing.is_empty() {
                    return false;
                }
                if closing.last() == Some(&c) {
                    closing.pop();
                }
            }
            b';' if closing.is_empty() => return false,
            _ => {}
        }
        i += 1;
    }
    true
}

/// An arbitrary value as written in a class into CSS, as Tailwind's `decodeArbitraryValue`:
/// underscores are spaces, `\_` is one, but in `url()` nothing changes and in the name a `var()`
/// reads underscores stay; and operators in math functions get the white space CSS needs.
fn decode(input: &str) -> String {
    #[derive(Clone, Copy, PartialEq)]
    enum In {
        Url,
        /// A `var()` or a `theme()`, and whether its first argument is still to end.
        Var(bool),
        Other,
    }
    let mut out = String::with_capacity(input.len());
    let mut within: Vec<In> = Vec::new();
    // Where the name before a parenthesis starts.
    let mut name = 0;
    let mut chars = input.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if within.contains(&In::Url) {
            match c {
                '(' => within.push(In::Other),
                ')' => drop(within.pop()),
                _ => {}
            }
            out.push(c);
            continue;
        }
        match c {
            '\\' if chars.peek().is_some_and(|&(_, n)| n == '_') => {
                chars.next();
                out.push('_');
            }
            '_' if within.last() == Some(&In::Var(true)) => out.push('_'),
            '_' => out.push(' '),
            '(' => {
                let f = &input[name..i];
                let is = |n: &str| f == n || f.ends_with(&format!("_{n}"));
                within.push(match () {
                    _ if is("url") => In::Url,
                    _ if is("var") || is("theme") => In::Var(true),
                    _ => In::Other,
                });
                out.push(c);
            }
            ')' => {
                within.pop();
                out.push(c);
            }
            ',' => {
                if let Some(top @ In::Var(true)) = within.last_mut() {
                    *top = In::Var(false);
                }
                out.push(c);
            }
            c => out.push(c),
        }
        if !(c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            name = i + c.len_utf8();
        }
    }
    match input.contains('(') {
        true => spaced(&out),
        false => out,
    }
}

const MATH: [&str; 19] = [
    "calc", "min", "max", "clamp", "mod", "rem", "sin", "cos", "tan", "asin", "acos", "atan",
    "atan2", "pow", "sqrt", "hypot", "log", "exp", "round",
];

/// `input` with white space around the operators in its math functions, as Tailwind's
/// `addWhitespaceAroundMathOperators`.
fn spaced(input: &str) -> String {
    if !MATH.iter().any(|f| input.contains(f)) {
        return input.to_string();
    }
    let b = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len() + 8);
    // Whether each function the text is in, innermost first, is math.
    let mut math: Vec<bool> = Vec::new();
    let (mut value_at, mut last_value_at): (Option<usize>, Option<usize>) = (None, None);
    for (i, &c) in b.iter().enumerate() {
        if c.is_ascii_digit() || (value_at.is_some() && (c == b'%' || c.is_ascii_alphabetic())) {
            value_at = Some(i);
        } else {
            last_value_at = value_at;
            value_at = None;
        }
        let top = math.first().copied().unwrap_or(false);
        match c {
            b'(' => {
                out.push(c);
                let start = (0..i)
                    .rev()
                    .take_while(|&j| b[j].is_ascii_digit() || b[j].is_ascii_lowercase())
                    .last()
                    .unwrap_or(i);
                let name = &input[start..i];
                math.insert(0, MATH.contains(&name) || (top && name.is_empty()));
            }
            b')' => {
                out.push(c);
                if !math.is_empty() {
                    math.remove(0);
                }
            }
            b',' if top => out.extend(b", "),
            b' ' if top && out.last() == Some(&b' ') => {}
            b'+' | b'*' | b'/' | b'-' if top => {
                let end = out
                    .iter()
                    .rposition(|c| !c.is_ascii_whitespace())
                    .map_or(0, |e| e + 1);
                let prev = end.checked_sub(1).map_or(0, |k| out[k]);
                let before = end.checked_sub(2).map_or(0, |k| out[k]);
                let next = b.get(i + 1).copied().unwrap_or(0);
                let operator = |c: u8| matches!(c, b'+' | b'*' | b'/' | b'-');
                if (matches!(prev, b'e' | b'E') && before.is_ascii_digit())
                    || operator(prev)
                    || matches!(prev, b'(' | b',')
                {
                    out.push(c);
                } else if i > 0 && b[i - 1] == b' ' {
                    out.extend([c, b' ']);
                } else if prev.is_ascii_digit()
                    || next.is_ascii_digit()
                    || prev == b')'
                    || next == b'('
                    || operator(next)
                    || (i > 0 && last_value_at == Some(i - 1))
                {
                    out.extend([b' ', c, b' ']);
                } else {
                    out.push(c);
                }
            }
            _ => out.push(c),
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| input.to_string())
}

/// A number and its unit, as Tailwind's `dimensions` reads them.
fn dimension(s: &str) -> Option<(f64, &str)> {
    let body = s.strip_prefix(['+', '-']).unwrap_or(s);
    let end = (body.find(|c: char| !(c.is_ascii_digit() || c == '.'))).unwrap_or(body.len());
    let (number, unit) = body.split_at(end);
    let digits = |d: &str| !d.is_empty() && d.bytes().all(|c| c.is_ascii_digit());
    let number_ok = match number.split_once('.') {
        Some((whole, fraction)) => (whole.is_empty() || digits(whole)) && digits(fraction),
        None => digits(number),
    };
    let unit_ok = unit.is_empty() || unit == "%" || unit.bytes().all(|c| c.is_ascii_alphabetic());
    if !(number_ok && unit_ok) {
        return None;
    }
    Some((s[..s.len() - unit.len()].parse().ok()?, unit))
}

/// Whether `s` is `String(n)` of a number `n` that is a whole number at least 0, as Tailwind's
/// `isPositiveInteger`.
fn positive_integer(s: &str) -> bool {
    s.parse::<f64>()
        .is_ok_and(|n| n.fract() == 0.0 && n >= 0.0 && js::number_to_string(n) == s)
}

/// Whether `s` is `String(n)` of a number `n` at least 0 that is a multiple of a quarter, as
/// Tailwind's `isValidSpacingMultiplier`.
fn quarter(s: &str) -> bool {
    s.parse::<f64>()
        .is_ok_and(|n| n >= 0.0 && n % 0.25 == 0.0 && js::number_to_string(n) == s)
}

// ----------------------------------------------------------------------------------------- theme

/// Tailwind's default theme, its `theme.css`, with Valle's font stacks.
static THEME: LazyLock<Theme> = LazyLock::new(|| {
    let mut theme = Theme::parse(include_str!("tailwind/theme.css"));
    for (key, family) in FONTS {
        theme.values.insert(key.into(), (family.into(), false));
    }
    theme
});

/// The font stacks: Valle draws its built-in fonts and font resources, never the system's that
/// Tailwind's stacks name first, so `font-sans` is its sans-serif.
const FONTS: [(&str, &str); 3] = [
    ("--font-sans", "sans-serif"),
    ("--font-serif", "serif"),
    ("--font-mono", "monospace"),
];

struct Theme {
    /// Each variable's value, and whether utilities write it inline rather than as a `var()`:
    /// the deprecated ones.
    values: HashMap<String, (String, bool)>,
}

/// Keys of each namespace that belong to others, as `--font-weight-*` is not a `--font-*`.
const IGNORED: [(&str, &[&str]); 5] = [
    ("--font", &["--font-weight", "--font-size"]),
    ("--inset", &["--inset-shadow", "--inset-ring"]),
    (
        "--text",
        &[
            "--text-color",
            "--text-decoration-color",
            "--text-decoration-thickness",
            "--text-indent",
            "--text-shadow",
            "--text-underline-offset",
        ],
    ),
    (
        "--grid-column",
        &["--grid-column-start", "--grid-column-end"],
    ),
    ("--grid-row", &["--grid-row-start", "--grid-row-end"]),
];

impl Theme {
    /// The `--name: value;` declarations in each `@theme` block, outside its `@keyframes`.
    fn parse(css: &str) -> Theme {
        let mut values = HashMap::new();
        let (mut depth, mut start, mut inline) = (0, 0, false);
        let b = css.as_bytes();
        let mut i = 0;
        while i < b.len() {
            match b[i] {
                b'/' if b.get(i + 1) == Some(&b'*') => {
                    i = css[i..].find("*/").map_or(b.len(), |e| i + e + 2);
                    start = i;
                    continue;
                }
                b'{' => {
                    if depth == 0 {
                        inline = css[start..i].split_whitespace().any(|w| w == "inline");
                    }
                    depth += 1;
                    start = i + 1;
                }
                b'}' => {
                    depth -= 1;
                    start = i + 1;
                }
                b';' => {
                    if depth == 1
                        && let Some((name, value)) = css[start..i].split_once(':')
                    {
                        let value = value.split_whitespace().collect::<Vec<_>>().join(" ");
                        values.insert(name.trim().to_string(), (value, inline));
                    }
                    start = i + 1;
                }
                _ => {}
            }
            i += 1;
        }
        Theme { values }
    }

    /// The variable of the namespaces `keys` that `value` names, or the first namespace's own
    /// for none, as Tailwind's `resolveKey`.
    fn key(&self, value: Option<&str>, keys: &[&str]) -> Option<String> {
        for namespace in keys {
            let mut key = match value {
                Some(v) => format!("{namespace}-{v}"),
                None => namespace.to_string(),
            };
            if !self.values.contains_key(&key) {
                match value {
                    Some(v) if v.contains('.') => {
                        key = format!("{namespace}-{}", v.replace('.', "_"));
                        if !self.values.contains_key(&key) {
                            continue;
                        }
                    }
                    _ => continue,
                }
            }
            let ignored = (IGNORED.iter().find(|(n, _)| n == namespace)).is_some_and(|(_, ks)| {
                ks.iter()
                    .any(|k| key == *k || key.starts_with(&format!("{k}-")))
            });
            if !ignored {
                return Some(key);
            }
        }
        None
    }

    /// What a utility writes for the variable `value` names: a `var()`, or a deprecated value
    /// itself.
    fn resolve(&self, value: Option<&str>, keys: &[&str]) -> Option<String> {
        let key = self.key(value, keys)?;
        Some(self.written(&key))
    }

    /// What `resolve` writes, and for each of `nested` the variable of that key with it after,
    /// if the theme has one, as Tailwind's `resolveWith`: `--text-sm--line-height` of `text-sm`.
    fn resolve_with(
        &self,
        value: &str,
        keys: &[&str],
        nested: &[&str],
    ) -> Option<(String, Vec<Option<String>>)> {
        let key = self.key(Some(value), keys)?;
        let nested = (nested.iter())
            .map(|n| format!("{key}{n}"))
            .map(|k| self.values.contains_key(&k).then(|| self.written(&k)))
            .collect();
        Some((self.written(&key), nested))
    }

    fn written(&self, key: &str) -> String {
        match &self.values[key] {
            (v, true) => v.clone(),
            (_, false) => format!("var({key})"),
        }
    }

    fn value(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(|(v, _)| &**v)
    }
}

fn theme() -> &'static Theme {
    &THEME
}

// --------------------------------------------------------------------------------------- elements

/// The declarations each node's classes make, in template order, in the order they cascade:
/// those its parent's classes make for its children, then its own, then the important ones in
/// the same order.
pub fn declarations(template: &Template) -> Result<Vec<Vec<Declaration>>, Error> {
    let mut classes = Vec::with_capacity(template.nodes.len());
    for node in &template.nodes {
        let expanded = (node.classes.iter())
            .map(|c| class(c).map_err(|e| format!("{}: {e}", node.place())))
            .collect::<Result<Vec<_>, _>>()?;
        classes.push(expanded);
    }
    // Who is a child, and not the last, of whom.
    let mut parent: Vec<Option<usize>> = vec![None; template.nodes.len()];
    for (i, node) in template.nodes.iter().enumerate() {
        if let Some((_, rest)) = node.children.split_last() {
            for &c in rest {
                parent[c as usize] = Some(i);
            }
        }
    }
    let mut out = Vec::with_capacity(template.nodes.len());
    for (i, node) in template.nodes.iter().enumerate() {
        let from_parent = parent[i].map_or(&[][..], |p| &classes[p][..]);
        out.push(element(&classes[i], from_parent).map_err(|e| format!("{}: {e}", node.place()))?);
    }
    Ok(out)
}

/// An element's declarations, from its classes `own` and those of its parent, `parent`, when it
/// is a child of it but the last.
fn element(own: &[Class], parent: &[Class]) -> Result<Vec<Declaration>, String> {
    // Rules of lower specificity first, the parent's for its children, then in Tailwind's order.
    let mut rules: Vec<(&Class, &Rule, bool)> = Vec::new();
    for (classes, inherited) in [(parent, true), (own, false)] {
        let start = rules.len();
        rules.extend(
            classes
                .iter()
                .flat_map(|c| c.rules.iter().map(move |r| (c, r, inherited))),
        );
        rules[start..].sort_by(|(a, x, _), (b, y, _)| {
            (x.sort.compare(&y.sort)).then_with(|| natural(&a.name, &b.name))
        });
    }
    let mut written: Vec<(&str, &str, &Class)> = Vec::new();
    for important in [false, true] {
        for &(c, r, inherited) in rules.iter().filter(|(c, ..)| c.important == important) {
            for n in &r.nodes {
                match (n, inherited) {
                    (Node::Declaration(p, v), false) => written.push((p, v, c)),
                    (Node::At(rule, inner), false) if supports(rule) => {
                        for n in inner {
                            if let Node::Declaration(p, v) = n {
                                written.push((p, v, c));
                            }
                        }
                    }
                    (Node::Rule(CHILDREN, inner), true) => {
                        for n in inner {
                            if let Node::Declaration(p, v) = n {
                                written.push((p, v, c));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    let custom: HashMap<&str, &str> = (written.iter())
        .filter(|(p, ..)| p.starts_with("--"))
        .map(|&(p, v, _)| (p, v))
        .collect();
    let mut out = Vec::new();
    for (p, v, c) in written {
        if p.starts_with("--") || BESIDE.contains(&p) {
            continue;
        }
        let at = |e: String| format!("className {}: {p}: {e}", c.name);
        // One that is invalid at computed-value time is as if it were not written, and so is one
        // whose variables are all empty.
        if let Some(value) = substitute(v, &custom, 0).map_err(at)?
            && !value.trim().is_empty()
        {
            out.push(Declaration {
                property: p.to_string(),
                value,
                important: c.important,
                class: c.name.clone(),
            });
        }
    }
    Ok(out)
}

/// Whether the engine takes the declaration an `@supports (property: value)` rule asks about.
fn supports(rule: &str) -> bool {
    let condition = rule
        .strip_prefix("@supports (")
        .and_then(|r| r.strip_suffix(')'));
    condition
        .and_then(|c| c.split_once(':'))
        .is_some_and(|(p, v)| super::property::accepts(rule, p.trim(), v.trim()).is_ok())
}

/// `value` with its `var()`s resolved for an element of these custom properties; `None` if one
/// has no value and no fallback, which makes it invalid.
fn substitute(
    value: &str,
    custom: &HashMap<&str, &str>,
    depth: usize,
) -> Result<Option<String>, String> {
    if depth > 16 {
        return Err("engine bug: custom properties refer to each other in a cycle".into());
    }
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(i) = rest.find("var(") {
        out.push_str(&rest[..i]);
        rest = &rest[i + "var(".len()..];
        let end = closing(rest).ok_or_else(|| format!("{value}: a var() is not closed"))?;
        let (name, fallback) = match rest[..end].split_once(',') {
            Some((name, fallback)) => (name.trim(), Some(fallback.trim())),
            None => (rest[..end].trim(), None),
        };
        rest = &rest[end + 1..];
        let registered = || match PROPERTIES.iter().find(|(p, _)| *p == name) {
            Some((_, initial)) => Ok(initial.map(str::to_string)),
            None if UNREGISTERED.contains(&name) => Ok(None),
            None if name.starts_with("--tw-") => {
                Err(format!("engine bug: {name} is not registered"))
            }
            None => Err(VARIABLES.to_string()),
        };
        let found = match custom.get(name) {
            // `initial` is the value it is registered with, if any.
            Some(v) if v.trim() == "initial" => registered()?,
            Some(v) => substitute(v, custom, depth + 1)?,
            None => match THEME.value(name) {
                Some(v) => substitute(v, custom, depth + 1)?,
                None => registered()?,
            },
        };
        let found = match (found, fallback) {
            (Some(v), _) => v,
            (None, Some(f)) => match substitute(f, custom, depth + 1)? {
                Some(v) => v,
                None => return Ok(None),
            },
            (None, None) => return Ok(None),
        };
        out.push_str(&found);
    }
    out.push_str(rest);
    Ok(Some(out))
}

/// Where the parenthesis that `s` is inside closes.
fn closing(s: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' if depth == 0 => return Some(i),
            ')' => depth -= 1,
            _ => {}
        }
    }
    None
}

/// A class's CSS on one line, as `web/tests/tailwind.ts` prints Tailwind's: its rules, `&` its
/// element.
pub fn print(class: &Class) -> String {
    fn body(head: &str, nodes: &[Node], important: &str) -> String {
        let mut inner = Vec::new();
        for n in nodes {
            match n {
                Node::Declaration(p, _) if p == "--tw-sort" => {}
                Node::Declaration(p, v) => inner.push(format!("{p}: {v}{important};")),
                Node::Rule(s, nodes) => inner.push(body(s, nodes, important)),
                Node::At(s, nodes) => inner.push(body(s, nodes, important)),
            }
        }
        format!("{head} {{ {} }}", inner.join(" "))
    }
    let important = if class.important { " !important" } else { "" };
    let mut rules = Vec::new();
    for r in &class.rules {
        let own: Vec<Node> = (r.nodes.iter())
            .filter(|n| !matches!(n, Node::Rule(..)))
            .cloned()
            .collect();
        if own
            .iter()
            .any(|n| !matches!(n, Node::Declaration(p, _) if p == "--tw-sort"))
        {
            rules.push(body("&", &own, important));
        }
        for n in &r.nodes {
            if let Node::Rule(s, inner) = n {
                rules.push(body(s, inner, important));
            }
        }
    }
    rules.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn css(name: &str) -> String {
        print(&class(name).unwrap())
    }

    fn error(name: &str) -> String {
        class(name).unwrap_err()
    }

    #[test]
    fn classes_expand_to_tailwinds_css() {
        let cases = [
            ("p-4", "& { padding: calc(var(--spacing) * 4); }"),
            ("p-1", "& { padding: var(--spacing); }"),
            ("inset-x-0", "& { inset-inline: 0px; }"),
            ("-m-2", "& { margin: calc(var(--spacing) * -2); }"),
            ("p-0.75", "& { padding: calc(var(--spacing) * 0.75); }"),
            ("w-1/2", "& { width: calc(1 / 2 * 100%); }"),
            ("-inset-1/2", "& { inset: calc(calc(1 / 2 * 100%) * -1); }"),
            ("w-px", "& { width: 1px; }"),
            ("-top-px", "& { top: -1px; }"),
            ("max-w-md", "& { max-width: var(--container-md); }"),
            ("z-10", "& { z-index: 10; }"),
            ("-z-10", "& { z-index: calc(10 * -1); }"),
            ("aspect-3/2", "& { aspect-ratio: 3/2; }"),
            ("aspect-video", "& { aspect-ratio: var(--aspect-video); }"),
            (
                "size-4",
                "& { width: calc(var(--spacing) * 4); height: calc(var(--spacing) * 4); }",
            ),
            ("p-[3px]", "& { padding: 3px; }"),
            ("w-[calc(100%-2rem)]", "& { width: calc(100% - 2rem); }"),
            ("[padding:3px_1px]", "& { padding: 3px 1px; }"),
            (
                "p-4!",
                "& { padding: calc(var(--spacing) * 4) !important; }",
            ),
            ("overflow-x-clip", "& { overflow-x: clip; }"),
            (
                "space-x-4",
                ":where(& > :not(:last-child)) { --tw-space-x-reverse: 0; \
             margin-inline-start: calc(calc(var(--spacing) * 4) * var(--tw-space-x-reverse)); \
             margin-inline-end: calc(calc(var(--spacing) * 4) * calc(1 - \
             var(--tw-space-x-reverse))); }",
            ),
            (
                "space-y-0",
                ":where(& > :not(:last-child)) { --tw-space-y-reverse: 0; margin-block-start: 0; \
             margin-block-end: 0; }",
            ),
            (
                "space-x-reverse",
                ":where(& > :not(:last-child)) { --tw-space-x-reverse: 1; }",
            ),
            ("order-first", "& { order: -9999; }"),
            ("-order-2", "& { order: calc(2 * -1); }"),
            ("col-span-2", "& { grid-column: span 2 / span 2; }"),
            ("row-span-full", "& { grid-row: 1 / -1; }"),
            ("-col-start-2", "& { grid-column-start: calc(2 * -1); }"),
            ("flex-1", "& { flex: 1; }"),
            ("flex-1/2", "& { flex: calc(1/2 * 100%); }"),
            ("flex-[2_2_0%]", "& { flex: 2 2 0%; }"),
            ("grow", "& { flex-grow: 1; }"),
            ("shrink-0", "& { flex-shrink: 0; }"),
            ("basis-md", "& { flex-basis: var(--container-md); }"),
            ("basis-1/3", "& { flex-basis: calc(1 / 3 * 100%); }"),
            (
                "grid-cols-3",
                "& { grid-template-columns: repeat(3, minmax(0, 1fr)); }",
            ),
            ("auto-rows-fr", "& { grid-auto-rows: minmax(0, 1fr); }"),
            (
                "auto-cols-4",
                "& { grid-auto-columns: calc(var(--spacing) * 4); }",
            ),
            ("grid-flow-col-dense", "& { grid-auto-flow: column dense; }"),
            ("justify-self-end", "& { justify-self: flex-end; }"),
            (
                "text-sm",
                "& { font-size: var(--text-sm); line-height: var(--tw-leading, \
             var(--text-sm--line-height)); }",
            ),
            (
                "text-sm/6",
                "& { font-size: var(--text-sm); line-height: calc(var(--spacing) * 6); }",
            ),
            (
                "text-[13px]/[20px]",
                "& { font-size: 13px; line-height: 20px; }",
            ),
            ("text-[1.5em]", "& { font-size: 1.5em; }"),
            (
                "text-red-500/50",
                "& { color: color-mix(in oklab, var(--color-red-500) 50%, transparent); }",
            ),
            ("text-current", "& { color: currentcolor; }"),
            ("text-[#f00]", "& { color: #f00; }"),
            (
                "font-bold",
                "& { --tw-font-weight: var(--font-weight-bold); font-weight: \
             var(--font-weight-bold); }",
            ),
            ("font-sans", "& { font-family: var(--font-sans); }"),
            (
                "font-[300]",
                "& { --tw-font-weight: 300; font-weight: 300; }",
            ),
            ("font-['Brand_Sans']", "& { font-family: 'Brand Sans'; }"),
            (
                "leading-tight",
                "& { --tw-leading: var(--leading-tight); line-height: var(--leading-tight); }",
            ),
            ("leading-none", "& { --tw-leading: 1; line-height: 1; }"),
            (
                "-tracking-[1px]",
                "& { --tw-tracking: calc(1px * -1); letter-spacing: calc(1px * -1); }",
            ),
            ("decoration-2", "& { text-decoration-thickness: 2px; }"),
            (
                "decoration-sky-500",
                "& { text-decoration-color: var(--color-sky-500); }",
            ),
            (
                "-underline-offset-2",
                "& { text-underline-offset: calc(2px * -1); }",
            ),
            ("font-stretch-75%", "& { font-stretch: 75%; }"),
            (
                "tabular-nums",
                "& { --tw-numeric-spacing: tabular-nums; font-variant-numeric: var(--tw-ordinal,) \
             var(--tw-slashed-zero,) var(--tw-numeric-figure,) var(--tw-numeric-spacing,) \
             var(--tw-numeric-fraction,); }",
            ),
            (
                "line-clamp-3",
                "& { overflow: hidden; display: -webkit-box; -webkit-box-orient: vertical; \
             -webkit-line-clamp: 3; }",
            ),
            (
                "bg-sky-500",
                "& { background-color: var(--color-sky-500); }",
            ),
            (
                "bg-black/[0.3]",
                "& { background-color: color-mix(in oklab, var(--color-black) 30%, transparent); }",
            ),
            ("bg-[#0af]", "& { background-color: #0af; }"),
            ("bg-[center_top]", "& { background-position: center top; }"),
            ("bg-[10px]", "& { background-position: 10px; }"),
            (
                "bg-[length:200px_100px]",
                "& { background-size: 200px 100px; }",
            ),
            (
                "bg-[linear-gradient(red,blue)]",
                "& { background-image: linear-gradient(red,blue); }",
            ),
            ("fill-none", "& { fill: none; }"),
            (
                "fill-red-500/50",
                "& { fill: color-mix(in oklab, var(--color-red-500) 50%, transparent); }",
            ),
            ("stroke-current", "& { stroke: currentcolor; }"),
            ("stroke-2", "& { stroke-width: 2; }"),
            ("stroke-[1.5]", "& { stroke-width: 1.5; }"),
            ("stroke-[#f00]", "& { stroke: #f00; }"),
            ("bg-top-left", "& { background-position: left top; }"),
            ("bg-size-[auto_50%]", "& { background-size: auto 50%; }"),
            ("bg-repeat-round", "& { background-repeat: round; }"),
            (
                "bg-linear-to-r",
                "& { --tw-gradient-position: to right; @supports (background-image: \
             linear-gradient(in lab, red, red)) { --tw-gradient-position: to right in oklab; } \
             background-image: linear-gradient(var(--tw-gradient-stops)); }",
            ),
            (
                "-bg-linear-45/longer",
                "& { --tw-gradient-position: calc(45deg * -1); @supports (background-image: \
             linear-gradient(in lab, red, red)) { --tw-gradient-position: calc(45deg * -1) in \
             oklch longer hue; } background-image: linear-gradient(var(--tw-gradient-stops)); }",
            ),
            (
                "bg-linear-[25deg,red_5%,blue]",
                "& { --tw-gradient-position: 25deg,red 5%,blue; background-image: \
             linear-gradient(var(--tw-gradient-stops,25deg,red 5%,blue)); }",
            ),
            (
                "bg-conic-90/srgb",
                "& { --tw-gradient-position: from 90deg in srgb; background-image: \
             conic-gradient(var(--tw-gradient-stops)); }",
            ),
            (
                "bg-radial-[at_25%_25%]",
                "& { --tw-gradient-position: at 25% 25%; background-image: \
             radial-gradient(var(--tw-gradient-stops,at 25% 25%)); }",
            ),
            (
                "from-red-500",
                "& { --tw-gradient-from: var(--color-red-500); --tw-gradient-stops: \
             var(--tw-gradient-via-stops, var(--tw-gradient-position), var(--tw-gradient-from) \
             var(--tw-gradient-from-position), var(--tw-gradient-to) \
             var(--tw-gradient-to-position)); }",
            ),
            ("via-30%", "& { --tw-gradient-via-position: 30%; }"),
            ("to-[3px]", "& { --tw-gradient-to-position: 3px; }"),
            ("rounded-lg", "& { border-radius: var(--radius-lg); }"),
            ("rounded-full", "& { border-radius: calc(infinity * 1px); }"),
            (
                "rounded-t-none",
                "& { border-top-left-radius: 0; border-top-right-radius: 0; }",
            ),
            (
                "border",
                "& { border-style: var(--tw-border-style); border-width: 1px; }",
            ),
            (
                "border-x-4",
                "& { border-inline-style: var(--tw-border-style); border-inline-width: 4px; }",
            ),
            (
                "border-t-sky-500/50",
                "& { border-top-color: color-mix(in oklab, var(--color-sky-500) 50%, transparent); }",
            ),
            (
                "border-dashed",
                "& { --tw-border-style: dashed; border-style: dashed; }",
            ),
            (
                "divide-y-2",
                ":where(& > :not(:last-child)) { --tw-divide-y-reverse: 0; border-bottom-style: \
             var(--tw-border-style); border-top-style: var(--tw-border-style); border-top-width: \
             calc(2px * var(--tw-divide-y-reverse)); border-bottom-width: calc(2px * calc(1 - \
             var(--tw-divide-y-reverse))); }",
            ),
            (
                "divide-red-500",
                ":where(& > :not(:last-child)) { border-color: var(--color-red-500); }",
            ),
            (
                "outline-2",
                "& { outline-style: var(--tw-outline-style); outline-width: 2px; }",
            ),
            ("-outline-offset-2", "& { outline-offset: calc(2px * -1); }"),
            (
                "outline-hidden",
                "& { --tw-outline-style: none; outline-style: none; @media (forced-colors: active) { \
             outline: 2px solid transparent; outline-offset: 2px; } }",
            ),
            (
                "shadow-md/50",
                "& { --tw-shadow-alpha: 50%; --tw-shadow: 0 4px 6px -1px var(--tw-shadow-color, \
             oklab(from rgb(0 0 0 / 0.1) l a b / 50%)), 0 2px 4px -2px var(--tw-shadow-color, \
             oklab(from rgb(0 0 0 / 0.1) l a b / 50%)); box-shadow: var(--tw-inset-shadow), \
             var(--tw-inset-ring-shadow), var(--tw-ring-offset-shadow), var(--tw-ring-shadow), \
             var(--tw-shadow); }",
            ),
            (
                "shadow-[0_0_4px]",
                "& { --tw-shadow: 0 0 4px var(--tw-shadow-color, currentcolor); box-shadow: \
             var(--tw-inset-shadow), var(--tw-inset-ring-shadow), var(--tw-ring-offset-shadow), \
             var(--tw-ring-shadow), var(--tw-shadow); }",
            ),
            (
                "shadow-red-500/50",
                "& { --tw-shadow-color: color-mix(in oklab, color-mix(in oklab, var(--color-red-500) \
             50%, transparent) var(--tw-shadow-alpha), transparent); }",
            ),
            (
                "inset-shadow-[0_2px_red]/20",
                "& { --tw-inset-shadow-alpha: 20%; --tw-inset-shadow: inset 0 2px \
             var(--tw-inset-shadow-color, oklab(from red l a b / 20%)); box-shadow: \
             var(--tw-inset-shadow), var(--tw-inset-ring-shadow), var(--tw-ring-offset-shadow), \
             var(--tw-ring-shadow), var(--tw-shadow); }",
            ),
            (
                "ring",
                "& { --tw-ring-shadow: var(--tw-ring-inset,) 0 0 0 calc(1px + \
             var(--tw-ring-offset-width)) var(--tw-ring-color, currentcolor); box-shadow: \
             var(--tw-inset-shadow), var(--tw-inset-ring-shadow), var(--tw-ring-offset-shadow), \
             var(--tw-ring-shadow), var(--tw-shadow); }",
            ),
            (
                "ring-offset-2",
                "& { --tw-ring-offset-width: 2px; --tw-ring-offset-shadow: var(--tw-ring-inset,) 0 0 \
             0 var(--tw-ring-offset-width) var(--tw-ring-offset-color); }",
            ),
            (
                "text-shadow-[0_1px_red]",
                "& { text-shadow: 0 1px var(--tw-text-shadow-color, red); }",
            ),
            ("opacity-40", "& { opacity: 40%; }"),
            ("opacity-[0.3]", "& { opacity: 0.3; }"),
            ("mix-blend-multiply", "& { mix-blend-mode: multiply; }"),
            ("bg-blend-screen", "& { background-blend-mode: screen; }"),
            ("isolate", "& { isolation: isolate; }"),
            (
                "-translate-x-1/2",
                "& { --tw-translate-x: calc(calc(1 / 2 * 100%) * -1); \
             translate: var(--tw-translate-x) var(--tw-translate-y); }",
            ),
            (
                "translate-full",
                "& { --tw-translate-x: 100%; --tw-translate-y: 100%; \
             translate: var(--tw-translate-x) var(--tw-translate-y); }",
            ),
            (
                "scale-50",
                "& { --tw-scale-x: 50%; --tw-scale-y: 50%; --tw-scale-z: 50%; \
             scale: var(--tw-scale-x) var(--tw-scale-y); }",
            ),
            ("-rotate-45", "& { rotate: calc(45deg * -1); }"),
            (
                "rotate-x-45",
                "& { --tw-rotate-x: rotateX(45deg); transform: var(--tw-rotate-x,) \
             var(--tw-rotate-y,) var(--tw-rotate-z,) var(--tw-skew-x,) var(--tw-skew-y,); }",
            ),
            ("origin-top-right", "& { transform-origin: 100% 0; }"),
            ("mask-[50%]", "& { mask-position: 50%; }"),
            (
                "mask-radial-at-top-left",
                "& { --tw-mask-radial-position: top left; }",
            ),
            (
                "mask-t-from-50%",
                "& { mask-image: var(--tw-mask-linear), var(--tw-mask-radial), var(--tw-mask-conic); \
             mask-composite: intersect; --tw-mask-linear: var(--tw-mask-left), \
             var(--tw-mask-right), var(--tw-mask-bottom), var(--tw-mask-top); --tw-mask-top: \
             linear-gradient(to top, var(--tw-mask-top-from-color) \
             var(--tw-mask-top-from-position), var(--tw-mask-top-to-color) \
             var(--tw-mask-top-to-position)); --tw-mask-top-from-position: 50%; }",
            ),
            ("object-cover", "& { object-fit: cover; }"),
            ("object-top-left", "& { object-position: left top; }"),
            ("object-[25%_75%]", "& { object-position: 25% 75%; }"),
            (
                "drop-shadow-[0_0_4px_red]/50",
                "& { --tw-drop-shadow-alpha: 50%; --tw-drop-shadow-size: drop-shadow(0 0 4px \
             var(--tw-drop-shadow-color, oklab(from red l a b / 50%))); --tw-drop-shadow: \
             var(--tw-drop-shadow-size); filter: var(--tw-blur,) var(--tw-brightness,) \
             var(--tw-contrast,) var(--tw-grayscale,) var(--tw-hue-rotate,) var(--tw-invert,) \
             var(--tw-saturate,) var(--tw-sepia,) var(--tw-drop-shadow,); }",
            ),
            (
                "perspective-near",
                "& { perspective: var(--perspective-near); }",
            ),
        ];
        for (name, expected) in cases {
            assert_eq!(css(name), expected, "{name}");
        }
    }

    #[test]
    fn classes_tailwind_does_not_make_are_errors() {
        for name in [
            "p-foo",
            "p-0.3",
            "p-4/50",
            "w-",
            "p-[]",
            "m-[a;b]",
            "-p-4",
            "foo",
            "grid-cols-0",
            "-col-span-2",
            "grow-[2]/3",
            "bg-[10px]/50",
            "bg-foo",
            "stroke-[3px]/50",
            "bg-linear",
            "-bg-linear-[to_right]",
            "-bg-linear-to-r",
            "bg-radial-50",
            "from-31",
            "from-[3px]/50",
            "border-2/50",
            "outline-[3px]/50",
            "rounded-foo",
            "-rounded-lg",
            "divide-x-[3px]/2",
            "shadow-none/50",
            "inset-shadow",
            "text-shadow",
            "ring-2/50",
            "ring-offset-2/50",
            "opacity-0.3",
            "opacity-50/50",
            "scale-50/50",
            "-rotate-[1_1_0]",
            "transform-foo",
            "filter-foo",
            "mask-foo",
            "mask-t-from-foo",
            "mask-l-to-33.3%",
            "-mask-radial-[closest-side]",
            "drop-shadow-sm/foo",
            "brightness-[50%]/50",
        ] {
            assert_eq!(
                error(name),
                format!("className {name} is not a Tailwind class, or not supported yet")
            );
        }
    }

    #[test]
    fn errors_say_what_to_write_instead() {
        let e = error("hover:p-4");
        assert!(
            e.starts_with("className hover:p-4: variants are not supported: "),
            "{e}"
        );
        assert_eq!(
            error("!p-4"),
            "className !p-4: Tailwind v4 marks a class important at its end: write p-4!"
        );
        for name in ["w-(--size)", "[--gap:4px]", "p-4/(--a)"] {
            assert_eq!(
                error(name),
                format!(
                    "className {name}: CSS variables are not supported: name the value with a const"
                )
            );
        }
        for (v3, v4) in [
            ("bg-gradient-to-r", "bg-linear-to-r"),
            ("flex-grow", "grow"),
            ("flex-shrink-0", "shrink-0"),
            ("-start-4", "-inset-s-4"),
            ("rounded-t", "rounded-t-sm"),
            ("max-w-screen-md", "max-w-[48rem]"),
            ("max-w-prose", "max-w-[65ch]"),
            ("bg-opacity-50", "bg-<color>/50"),
            ("text-opacity-25", "text-<color>/25"),
        ] {
            assert_eq!(
                error(v3),
                format!("className {v3} is a Tailwind v3 name: write {v4}")
            );
        }
        // What the engine cannot draw fails as the style would.
        assert_eq!(
            error("w-[var(--size)]"),
            "className w-[var(--size)]: width: CSS variables are not supported: name the value with a \
         const"
        );
        assert_eq!(
            error("sticky"),
            "className sticky: position: sticky is not supported: the canvas does not scroll; use \
         relative or absolute"
        );
        assert_eq!(
            error("float-left"),
            "className float-left: float is not supported: there is no document flow; use flex or \
         grid layout"
        );
        assert!(
            error("items-baseline-last")
                .starts_with("className items-baseline-last: align-items: ")
        );
        assert_eq!(
            error("align-middle"),
            "className align-middle: vertical-align is not supported: there is no HTML inline flow; \
         use flex layout, and Span for styled runs of text"
        );
        let e = error("container");
        assert_eq!(
            e,
            "className container: its CSS has @media rules, which do not apply: the canvas has a \
         fixed size"
        );
    }

    /// The declarations of an element of `own` classes, the child but the last of one of `parent`
    /// classes.
    fn applied(own: &[&str], parent: &[&str]) -> Vec<(String, String, bool)> {
        let classes = |names: &[&str]| names.iter().map(|n| class(n).unwrap()).collect::<Vec<_>>();
        let declarations = element(&classes(own), &classes(parent)).unwrap();
        (declarations.into_iter())
            .map(|d| (d.property, d.value, d.important))
            .collect()
    }

    #[test]
    fn what_no_canvas_takes_is_left_out() {
        assert!(applied(&["antialiased"], &[]).is_empty());
        assert!(applied(&["subpixel-antialiased"], &[]).is_empty());
    }

    fn d(property: &str, value: &str, important: bool) -> (String, String, bool) {
        (property.into(), value.into(), important)
    }

    #[test]
    fn variables_resolve_on_each_element() {
        assert_eq!(
            applied(&["p-4", "max-w-md"], &[]),
            [
                d("max-width", "28rem", false),
                d("padding", "calc(0.25rem * 4)", false)
            ]
        );
        // The parent's spacing, reversed by its other class, which comes later.
        assert_eq!(
            applied(&[], &["space-x-reverse", "space-x-2"]),
            [
                d("margin-inline-start", "calc(calc(0.25rem * 2) * 1)", false),
                d(
                    "margin-inline-end",
                    "calc(calc(0.25rem * 2) * calc(1 - 1))",
                    false
                )
            ]
        );
    }

    #[test]
    fn text_classes_read_each_others_variables() {
        // The size's line height gives way to the leading's, whatever the order written.
        assert_eq!(
            applied(&["leading-6", "text-sm"], &[]),
            [
                d("font-size", "0.875rem", false),
                d("line-height", "calc(0.25rem * 6)", false),
                d("line-height", "calc(0.25rem * 6)", false)
            ]
        );
        assert_eq!(
            applied(&["text-sm"], &[])[1],
            d("line-height", "calc(1.25 / 0.875)", false)
        );
        // Numeric variants add up.
        let numeric = applied(&["ordinal", "tabular-nums"], &[]);
        let words: Vec<&str> = numeric[1].1.split_whitespace().collect();
        assert_eq!(words, ["ordinal", "tabular-nums"]);
    }

    #[test]
    fn borders_take_their_style_and_divide_children() {
        let own = applied(&["border-2", "border-dotted"], &[]);
        assert!(own.contains(&d("border-width", "2px", false)), "{own:?}");
        for (p, v, _) in &own {
            assert!(p != "border-style" || v == "dotted", "{own:?}");
        }
        assert_eq!(
            applied(&[], &["divide-x-2", "divide-x-reverse", "divide-dashed"]),
            [
                d("border-inline-style", "dashed", false),
                d("border-inline-start-width", "calc(2px * 1)", false),
                d("border-inline-end-width", "calc(2px * calc(1 - 1))", false),
                d("border-style", "dashed", false)
            ]
        );
    }

    #[test]
    fn shadows_stack_rings_over_box_shadows_in_their_colors() {
        let shadow = |classes: &[&str]| applied(classes, &[]).pop().unwrap().1;
        // Each kind is none until a class sets it; the offset is the background's, white unless set.
        assert_eq!(
            shadow(&["ring-red-500", "shadow-sm", "ring-offset-2", "ring-2"]),
            "0 0 #0000, 0 0 #0000,  0 0 0 2px #fff,  0 0 0 calc(2px + 2px) oklch(63.7% 0.237 25.331), \
         0 1px 3px 0 rgb(0 0 0 / 0.1), 0 1px 2px -1px rgb(0 0 0 / 0.1)"
        );
        // A shadow's color takes the opacity the shadow's modifier gives.
        let red = "color-mix(in oklab, oklch(63.7% 0.237 25.331) 50%, transparent)";
        assert_eq!(
            shadow(&["shadow-red-500", "shadow-md/50"]),
            format!(
                "0 0 #0000, 0 0 #0000, 0 0 #0000, 0 0 #0000, 0 4px 6px -1px {red}, 0 2px 4px -2px {red}"
            )
        );
    }

    #[test]
    fn transforms_read_each_axis() {
        let last = |classes: &[&str]| applied(classes, &[]).pop();
        assert_eq!(
            last(&["translate-x-full", "-translate-y-1/2"]),
            Some(d("translate", "100% calc(calc(1 / 2 * 100%) * -1)", false))
        );
        assert_eq!(
            last(&["rotate-y-45", "skew-x-6"]),
            Some(d("transform", " rotateY(45deg)  skewX(6deg) ", false))
        );
        // With no axis set there is nothing to transform.
        assert_eq!(last(&["transform"]), None);
    }

    #[test]
    fn filters_run_in_tailwinds_order_and_drop_shadows_take_their_color() {
        let last = |classes: &[&str]| applied(classes, &[]).pop();
        // Whatever order they are written in, and without the name only Safari reads.
        let filter = last(&["sepia", "blur-sm", "brightness-150"]).unwrap().1;
        assert_eq!(
            filter.split_whitespace().collect::<Vec<_>>(),
            ["blur(8px)", "brightness(150%)", "sepia(100%)"]
        );
        let backdrop = applied(&["backdrop-blur-sm"], &[]);
        assert_eq!(backdrop.len(), 1);
        assert_eq!(backdrop[0].0, "backdrop-filter");
        let red = "color-mix(in oklab, oklch(63.7% 0.237 25.331) 50%, transparent)";
        let filter = last(&["drop-shadow-md/50", "drop-shadow-red-500"]);
        assert_eq!(
            filter.unwrap().1.trim(),
            format!("drop-shadow(0 3px 3px {red})")
        );
        // None of a filter's functions set: nothing to apply.
        assert_eq!(last(&["filter"]), None);
        assert_eq!(last(&["blur-none"]), None);
    }

    #[test]
    fn masks_intersect_their_gradients() {
        let image = |classes: &[&str]| {
            let all = applied(classes, &[]);
            (all.iter())
                .find(|(p, ..)| p == "mask-image")
                .map(|(_, v, _)| v.clone())
                .unwrap()
        };
        let white = "linear-gradient(#fff, #fff)";
        assert_eq!(
            image(&["mask-b-from-50%", "mask-t-to-75%"]),
            format!(
                "{white}, {white}, linear-gradient(to bottom, black 50%, transparent 100%), \
             linear-gradient(to top, black 0%, transparent 75%), {white}, {white}"
            )
        );
        assert_eq!(
            image(&["mask-radial-from-20%", "mask-circle", "mask-radial-at-top"]),
            format!(
                "{white}, radial-gradient(circle farthest-corner at top, black 20%, transparent \
             100%), {white}"
            )
        );
        assert_eq!(
            image(&["mask-conic-90", "mask-conic-to-50%"]),
            format!(
                "{white}, {white}, conic-gradient(from calc(1deg * 90), black 0%, transparent 50%)"
            )
        );
    }

    #[test]
    fn gradients_read_their_color_stops() {
        let image = |classes: &[&str]| applied(classes, &[]).pop().unwrap().1;
        // Mixed in Oklab, as the engine takes Lab, from 0% to 100% unless the stops say.
        assert_eq!(
            image(&["bg-linear-to-r", "from-[#f00]", "to-[#00f]"]),
            "linear-gradient(to right in oklab, #f00 0%, #00f 100%)"
        );
        assert_eq!(
            image(&[
                "to-[#00f]",
                "via-[#0f0]",
                "via-20%",
                "from-[#f00]",
                "bg-linear-90"
            ]),
            "linear-gradient(90deg in oklab, #f00 0%, #0f0 20%, #00f 100%)"
        );
        // Without stops the gradient is not drawn; one stop goes to transparent.
        assert!(applied(&["bg-radial"], &[]).is_empty());
        assert_eq!(
            image(&["bg-conic", "from-[#f00]"]),
            "conic-gradient(in oklab, #f00 0%, #0000 100%)"
        );
    }

    #[test]
    fn classes_cascade_in_tailwinds_order_and_important_ones_last() {
        // `padding` comes before `padding-inline` whatever the order written.
        assert_eq!(
            applied(&["px-2", "p-4"], &[]),
            [
                d("padding", "calc(0.25rem * 4)", false),
                d("padding-inline", "calc(0.25rem * 2)", false)
            ]
        );
        // By name, numbers by their values, where the properties are the same.
        assert_eq!(
            applied(&["p-10", "p-2!", "p-3"], &[]),
            [
                d("padding", "calc(0.25rem * 3)", false),
                d("padding", "calc(0.25rem * 10)", false),
                d("padding", "calc(0.25rem * 2)", true)
            ]
        );
        // The parent's rules for its children are less specific than the child's own.
        assert_eq!(
            applied(&["ms-1"], &["space-x-4"])[2],
            d("margin-inline-start", "0.25rem", false)
        );
    }

    #[test]
    fn rules_sort_by_their_first_property_then_by_how_many_they_set() {
        let order = |a: &str, b: &str| compare(&class(a).unwrap(), &class(b).unwrap());
        assert_eq!(order("p-4", "px-4"), Ordering::Less);
        assert_eq!(order("m-4", "p-4"), Ordering::Less);
        assert_eq!(order("p-2", "p-10"), Ordering::Less);
        assert_eq!(order("p-1.5", "p-1"), Ordering::Greater);
        assert_eq!(natural("a10b", "a9b"), Ordering::Greater);
        assert_eq!(natural("a-1", "a-1.5"), Ordering::Less);
    }

    #[test]
    fn arbitrary_values_decode_as_tailwind_does() {
        assert_eq!(decode("1px_solid"), "1px solid");
        assert_eq!(decode("a\\_b"), "a_b");
        assert_eq!(decode("url(a_b.png)"), "url(a_b.png)");
        assert_eq!(decode("var(--my_size,1_px)"), "var(--my_size,1 px)");
        assert_eq!(decode("calc(100%-2rem*2)"), "calc(100% - 2rem * 2)");
        assert_eq!(decode("min(1px,2px)"), "min(1px, 2px)");
        assert_eq!(decode("calc(1e-3*2)"), "calc(1e-3 * 2)");
        assert_eq!(segment("a:[b:c]:d", b':'), ["a", "[b:c]", "d"]);
        let kinds = ["color", "length", "percentage", "relative-size"];
        for (value, kind) in [
            ("larger", Some("relative-size")),
            ("1e3px", Some("length")),
            ("calc(1px+2%)", Some("length")),
            ("50%", Some("percentage")),
            ("RGB(0 0 0)", Some("color")),
            ("Red", Some("color")),
            ("var(--x)", None),
            ("1.px", None),
        ] {
            assert_eq!(types::infer(value, &kinds), kind, "{value}");
        }
        assert!(!valid("a;b") && valid("'a;b'") && !valid("a)"));
    }

    #[test]
    fn the_theme_has_tailwinds_variables() {
        assert_eq!(THEME.value("--spacing"), Some("0.25rem"));
        assert_eq!(
            THEME.value("--text-sm--line-height"),
            Some("calc(1.25 / 0.875)")
        );
        // Valle's font stacks, its built-in families.
        assert_eq!(THEME.value("--font-sans"), Some("sans-serif"));
        assert_eq!(THEME.value("--font-mono"), Some("monospace"));
        assert_eq!(
            THEME.resolve_with("sm", &["--text"], &["--line-height", "--font-weight"]),
            Some((
                "var(--text-sm)".into(),
                vec![Some("var(--text-sm--line-height)".into()), None]
            ))
        );
        // Deprecated values are written as they are.
        assert_eq!(
            THEME.resolve(None, &["--radius"]).as_deref(),
            Some("0.25rem")
        );
        assert_eq!(
            THEME.resolve(Some("md"), &["--radius"]).as_deref(),
            Some("var(--radius-md)")
        );
        // A namespace's keys that are another's.
        assert_eq!(THEME.resolve(Some("weight-bold"), &["--font"]), None);
        assert_eq!(
            THEME.value("--animate-spin"),
            Some("spin 1s linear infinite")
        );
    }
}
