//! What kind of value an arbitrary value is, as Tailwind's `inferDataType` guesses it (MIT, see
//! THIRD_PARTY.md): `text-[13px]` sets the font size and `text-[#f00]` the color.

use super::{MATH, segment};

/// The first of `kinds` that `value` is, unless it is a `var()`.
pub(super) fn infer(value: &str, kinds: &[&'static str]) -> Option<&'static str> {
    if value.starts_with("var(") {
        return None;
    }
    kinds.iter().copied().find(|kind| is(kind, value))
}

/// Whether `value` is of the kind Tailwind names `kind`.
pub(super) fn is(kind: &str, value: &str) -> bool {
    match kind {
        "color" => color(value),
        "length" => length(value),
        "percentage" => numbered(value, |rest| rest == "%") || math(value),
        "ratio" => ratio(value),
        "number" => numbered(value, str::is_empty) || math(value),
        "integer" => super::positive_integer(value),
        "url" => url(value),
        "position" => position(value),
        "bg-size" => bg_size(value),
        "line-width" => segment(value, b' ')
            .iter()
            .all(|w| length(w) || is("number", w) || ["thin", "medium", "thick"].contains(w)),
        "image" => image(value),
        "family-name" => family(value),
        "generic-name" => GENERIC.contains(&value),
        "absolute-size" => ABSOLUTE.contains(&value),
        "relative-size" => value == "larger" || value == "smaller",
        "angle" => numbered(value, |unit| ["deg", "rad", "grad", "turn"].contains(&unit)),
        "vector" => vector(value),
        _ => false,
    }
}

const GENERIC: [&str; 13] = [
    "serif",
    "sans-serif",
    "monospace",
    "cursive",
    "fantasy",
    "system-ui",
    "ui-serif",
    "ui-sans-serif",
    "ui-monospace",
    "ui-rounded",
    "math",
    "emoji",
    "fangsong",
];

const ABSOLUTE: [&str; 8] = [
    "xx-small",
    "x-small",
    "small",
    "medium",
    "large",
    "x-large",
    "xx-large",
    "xxx-large",
];

const UNITS: [&str; 31] = [
    "cm", "mm", "Q", "in", "pc", "pt", "px", "em", "ex", "ch", "rem", "lh", "rlh", "vw", "vh",
    "vmin", "vmax", "vb", "vi", "svw", "svh", "lvw", "lvh", "dvw", "dvh", "cqw", "cqh", "cqi",
    "cqb", "cqmin", "cqmax",
];

/// Whether `s` is a number as Tailwind's pattern `[+-]?\d*\.?\d+(?:[eE][+-]?\d+)?` matches one,
/// followed by what `rest` takes.
fn numbered(s: &str, rest: impl Fn(&str) -> bool) -> bool {
    let b = s.as_bytes();
    let digits = |from: usize| from + b[from..].iter().take_while(|c| c.is_ascii_digit()).count();
    let start = usize::from(matches!(b.first(), Some(b'+' | b'-')));
    let whole = digits(start);
    let end = match b.get(whole) {
        Some(b'.') if digits(whole + 1) > whole + 1 => digits(whole + 1),
        _ if whole > start => whole,
        _ => return false,
    };
    if rest(&s[end..]) {
        return true;
    }
    // An exponent, if one follows.
    let sign = usize::from(matches!(b.get(end + 1), Some(b'+' | b'-')));
    let exponent = digits(end + 1 + sign);
    matches!(b.get(end), Some(b'e' | b'E')) && exponent > end + 1 + sign && rest(&s[exponent..])
}

/// Whether `s` has a math function in it, which may compute to anything.
fn math(s: &str) -> bool {
    s.contains('(') && MATH.iter().any(|f| s.contains(&format!("{f}(")))
}

pub(super) fn color(s: &str) -> bool {
    const FUNCTIONS: [&str; 12] = [
        "rgb(",
        "rgba(",
        "hsl(",
        "hsla(",
        "hwb(",
        "color(",
        "lab(",
        "lch(",
        "oklab(",
        "oklch(",
        "light-dark(",
        "color-mix(",
    ];
    let lower = s.to_ascii_lowercase();
    s.starts_with('#')
        || FUNCTIONS.iter().any(|f| lower.starts_with(f))
        || lower.starts_with("--alpha(")
        || NAMED.contains(&lower.as_str())
}

pub(super) fn length(s: &str) -> bool {
    numbered(s, |unit| UNITS.contains(&unit))
        || s.get(..10)
            .is_some_and(|p| p.eq_ignore_ascii_case("--spacing("))
        || math(s)
}

fn ratio(s: &str) -> bool {
    if math(s) {
        return true;
    }
    let Some((a, b)) = s.split_once('/') else {
        return false;
    };
    numbered(a.trim_end(), str::is_empty) && numbered(b.trim_start(), str::is_empty)
}

fn url(s: &str) -> bool {
    s.starts_with("url(") && s.ends_with(')')
}

fn position(s: &str) -> bool {
    let mut count = 0;
    for w in segment(s, b' ') {
        if ["center", "top", "right", "bottom", "left"].contains(&w) {
            count += 1;
        } else if !w.starts_with("var(") {
            if !(length(w) || is("percentage", w)) {
                return false;
            }
            count += 1;
        }
    }
    count > 0
}

fn bg_size(s: &str) -> bool {
    let mut count = 0;
    for layer in segment(s, b',') {
        if layer == "cover" || layer == "contain" {
            count += 1;
            continue;
        }
        let words = segment(layer, b' ');
        if words.len() != 1 && words.len() != 2 {
            return false;
        }
        if (words.iter()).all(|w| *w == "auto" || length(w) || is("percentage", w)) {
            count += 1;
        }
    }
    count > 0
}

fn image(s: &str) -> bool {
    const GRADIENTS: [&str; 6] = [
        "linear-gradient(",
        "radial-gradient(",
        "conic-gradient(",
        "repeating-linear-gradient(",
        "repeating-radial-gradient(",
        "repeating-conic-gradient(",
    ];
    const IMAGES: [&str; 4] = ["element(", "image(", "cross-fade(", "image-set("];
    let mut count = 0;
    for layer in segment(s, b',') {
        if layer.starts_with("var(") {
            continue;
        }
        let known = url(layer)
            || GRADIENTS.iter().any(|g| layer.starts_with(g))
            || IMAGES.iter().any(|i| layer.starts_with(i));
        if !known {
            return false;
        }
        count += 1;
    }
    count > 0
}

fn family(s: &str) -> bool {
    let mut count = 0;
    for name in segment(s, b',') {
        if name.starts_with(|c: char| c.is_ascii_digit()) {
            return false;
        }
        if !name.starts_with("var(") {
            count += 1;
        }
    }
    count > 0
}

fn vector(s: &str) -> bool {
    let words: Vec<&str> = s.split(' ').filter(|w| !w.is_empty()).collect();
    !s.starts_with(' ')
        && !s.ends_with(' ')
        && words.len() == 3
        && words.iter().all(|w| numbered(w, str::is_empty))
}

/// The named colors, and the system colors, as Tailwind lists them.
const NAMED: [&str; 169] = [
    "black",
    "silver",
    "gray",
    "white",
    "maroon",
    "red",
    "purple",
    "fuchsia",
    "green",
    "lime",
    "olive",
    "yellow",
    "navy",
    "blue",
    "teal",
    "aqua",
    "aliceblue",
    "antiquewhite",
    "aquamarine",
    "azure",
    "beige",
    "bisque",
    "blanchedalmond",
    "blueviolet",
    "brown",
    "burlywood",
    "cadetblue",
    "chartreuse",
    "chocolate",
    "coral",
    "cornflowerblue",
    "cornsilk",
    "crimson",
    "cyan",
    "darkblue",
    "darkcyan",
    "darkgoldenrod",
    "darkgray",
    "darkgreen",
    "darkgrey",
    "darkkhaki",
    "darkmagenta",
    "darkolivegreen",
    "darkorange",
    "darkorchid",
    "darkred",
    "darksalmon",
    "darkseagreen",
    "darkslateblue",
    "darkslategray",
    "darkslategrey",
    "darkturquoise",
    "darkviolet",
    "deeppink",
    "deepskyblue",
    "dimgray",
    "dimgrey",
    "dodgerblue",
    "firebrick",
    "floralwhite",
    "forestgreen",
    "gainsboro",
    "ghostwhite",
    "gold",
    "goldenrod",
    "greenyellow",
    "grey",
    "honeydew",
    "hotpink",
    "indianred",
    "indigo",
    "ivory",
    "khaki",
    "lavender",
    "lavenderblush",
    "lawngreen",
    "lemonchiffon",
    "lightblue",
    "lightcoral",
    "lightcyan",
    "lightgoldenrodyellow",
    "lightgray",
    "lightgreen",
    "lightgrey",
    "lightpink",
    "lightsalmon",
    "lightseagreen",
    "lightskyblue",
    "lightslategray",
    "lightslategrey",
    "lightsteelblue",
    "lightyellow",
    "limegreen",
    "linen",
    "magenta",
    "mediumaquamarine",
    "mediumblue",
    "mediumorchid",
    "mediumpurple",
    "mediumseagreen",
    "mediumslateblue",
    "mediumspringgreen",
    "mediumturquoise",
    "mediumvioletred",
    "midnightblue",
    "mintcream",
    "mistyrose",
    "moccasin",
    "navajowhite",
    "oldlace",
    "olivedrab",
    "orange",
    "orangered",
    "orchid",
    "palegoldenrod",
    "palegreen",
    "paleturquoise",
    "palevioletred",
    "papayawhip",
    "peachpuff",
    "peru",
    "pink",
    "plum",
    "powderblue",
    "rebeccapurple",
    "rosybrown",
    "royalblue",
    "saddlebrown",
    "salmon",
    "sandybrown",
    "seagreen",
    "seashell",
    "sienna",
    "skyblue",
    "slateblue",
    "slategray",
    "slategrey",
    "snow",
    "springgreen",
    "steelblue",
    "tan",
    "thistle",
    "tomato",
    "turquoise",
    "violet",
    "wheat",
    "whitesmoke",
    "yellowgreen",
    "transparent",
    "currentcolor",
    "canvas",
    "canvastext",
    "linktext",
    "visitedtext",
    "activetext",
    "buttonface",
    "buttontext",
    "buttonborder",
    "field",
    "fieldtext",
    "highlight",
    "highlighttext",
    "selecteditem",
    "selecteditemtext",
    "mark",
    "marktext",
    "graytext",
    "accentcolor",
    "accentcolortext",
];
