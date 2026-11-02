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
        if let Err(e) = super::accepts(&what, &d.property, &d.value)
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
        if p.starts_with("--") {
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
        .is_some_and(|(p, v)| super::accepts(rule, p.trim(), v.trim()).is_ok())
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
mod tests;
