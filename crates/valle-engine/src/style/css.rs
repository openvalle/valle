//! The CSS tokens style values are made of: numbers with units, functions, and lists split at
//! top-level commas or whitespace. Values reach here lowercased, except where case matters (font
//! family names).

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Num<'a> {
    pub value: f64,
    pub unit: &'a str,
}

/// A CSS number with its unit as the whole of `s`: `12px`, `-0.5`, `.5em`, `50%`, `1e3`. As in
/// CSS, a decimal point needs a digit after it.
pub fn num(s: &str) -> Option<Num<'_>> {
    let s = s.trim();
    let b = s.as_bytes();
    let mut i = 0;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    let digits = |i: &mut usize| {
        let start = *i;
        while *i < b.len() && b[*i].is_ascii_digit() {
            *i += 1;
        }
        *i > start
    };
    let whole = digits(&mut i);
    if i < b.len() && b[i] == b'.' {
        i += 1;
        if !digits(&mut i) {
            return None;
        }
    } else if !whole {
        return None;
    }
    if i + 1 < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let mut j = i + 1;
        if b[j] == b'+' || b[j] == b'-' {
            j += 1;
        }
        if digits(&mut j) {
            i = j;
        }
    }
    let value: f64 = s[..i].parse().ok()?;
    let unit = &s[i..];
    (value.is_finite() && unit.bytes().all(|c| c.is_ascii_alphabetic() || c == b'%'))
        .then_some(Num { value, unit })
}

/// `name(args)` as the whole of `s`.
pub fn function(s: &str) -> Option<(&str, &str)> {
    let s = s.trim();
    let open = s.find('(')?;
    let inner = s[open + 1..].strip_suffix(')')?;
    let name = &s[..open];
    (!name.is_empty() && name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-'))
        .then_some((name, inner))
}

/// Degrees, from `deg`, `rad`, `grad` or `turn`, or a 0.
pub fn angle(s: &str) -> Option<f64> {
    let n = num(s)?;
    match n.unit {
        "" if n.value == 0.0 => Some(0.0),
        _ => degrees(&n),
    }
}

/// The degrees of a number in `deg`, `rad`, `grad` or `turn`.
pub fn degrees(n: &Num) -> Option<f64> {
    Some(match n.unit {
        "deg" => n.value,
        "rad" => n.value.to_degrees(),
        "grad" => n.value * 0.9,
        "turn" => n.value * 360.0,
        _ => return None,
    })
}

/// The parts of `s` between separators outside parentheses and quotes; `sep` of `None` splits at
/// whitespace and drops empty parts.
fn split(s: &str, sep: Option<char>) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut quote = None;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        match (quote, c) {
            (Some(q), _) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '(') => depth += 1,
            (None, ')') => depth -= 1,
            (None, _) if depth == 0 && (Some(c) == sep || (sep.is_none() && c.is_whitespace())) => {
                let part = s[start..i].trim();
                if !part.is_empty() || sep.is_some() {
                    out.push(part);
                }
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    let part = s[start..].trim();
    if !part.is_empty() || (sep.is_some() && !out.is_empty()) {
        out.push(part);
    }
    out
}

pub fn commas(s: &str) -> Vec<&str> {
    split(s, Some(','))
}

pub fn words(s: &str) -> Vec<&str> {
    split(s, None)
}

pub fn slashes(s: &str) -> Vec<&str> {
    split(s, Some('/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_take_css_syntax() {
        let n = |s| num(s).map(|n| (n.value, n.unit));
        assert_eq!(n("12px"), Some((12.0, "px")));
        assert_eq!(n(" -0.5 "), Some((-0.5, "")));
        assert_eq!(n(".5em"), Some((0.5, "em")));
        assert_eq!(n("+50%"), Some((50.0, "%")));
        assert_eq!(n("1e3"), Some((1000.0, "")));
        assert_eq!(n("2e-1rem"), Some((0.2, "rem")));
        // `e` not followed by digits is a unit.
        assert_eq!(n("2em"), Some((2.0, "em")));
        for bad in ["", "px", "5.", ".", "-", "1.2.3", "1px2", "1 px", "1e999"] {
            assert_eq!(num(bad), None, "{bad}");
        }
    }

    #[test]
    fn lists_split_outside_parentheses_and_quotes() {
        assert_eq!(
            words(" 1px  rgb(1, 2, 3)\t2px "),
            ["1px", "rgb(1, 2, 3)", "2px"]
        );
        assert_eq!(commas("\"a, b\", c"), ["\"a, b\"", "c"]);
        assert_eq!(commas("a,,b"), ["a", "", "b"]);
        assert_eq!(commas(""), Vec::<&str>::new());
        assert_eq!(function("rgb(1 2 3)"), Some(("rgb", "1 2 3")));
        assert_eq!(function("rgb (1 2 3)"), None);
        assert_eq!(function("rgb(1 2 3"), None);
    }
}
