//! Numbers as text, as `Intl.NumberFormat("en-US")` writes them: the shortest decimal rounded half
//! away from zero, digits grouped by threes with commas.

/// `formatNumber`, `formatPercent` or `padNumber`, with their options.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Format {
    kind: Kind,
    /// Fraction digits, or `None` for Intl's default: up to 3 for a number, none for a percent.
    decimals: Option<u32>,
    grouping: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Number,
    Percent,
    /// Zeros before the integer up to this many digits.
    Pad(u32),
}

/// The most fraction digits Intl takes.
const DECIMALS: u32 = 100;
/// The most integer digits Intl pads to.
const WIDTH: u32 = 21;

impl Format {
    /// From `number|decimals|grouping`, `percent|decimals|grouping` or `pad|width|false`, the
    /// decimals empty for the default.
    pub fn parse(spec: &str) -> Result<Format, String> {
        let bad = || format!("format {spec}");
        let [kind, n, grouping] = spec.split('|').collect::<Vec<_>>()[..] else {
            return Err(bad());
        };
        let n: Option<u32> = match n {
            "" => None,
            n => Some(n.parse().map_err(|_| bad())?),
        };
        let grouping = match grouping {
            "true" => true,
            "false" => false,
            _ => return Err(bad()),
        };
        let kind = match (kind, n) {
            ("number", _) => Kind::Number,
            ("percent", _) => Kind::Percent,
            ("pad", Some(w)) if (1..=WIDTH).contains(&w) && !grouping => Kind::Pad(w),
            _ => return Err(bad()),
        };
        let decimals = if matches!(kind, Kind::Pad(_)) {
            Some(0)
        } else {
            n
        };
        if decimals.is_some_and(|d| d > DECIMALS) {
            return Err(bad());
        }
        Ok(Format {
            kind,
            decimals,
            grouping,
        })
    }

    pub fn text(&self, x: f64) -> String {
        let sign = if x.is_sign_negative() && !x.is_nan() {
            "-"
        } else {
            ""
        };
        let percent = if self.kind == Kind::Percent { "%" } else { "" };
        if !x.is_finite() {
            let body = if x.is_nan() { "NaN" } else { "∞" };
            return format!("{sign}{body}{percent}");
        }
        let (most, least) = match (self.kind, self.decimals) {
            (_, Some(d)) => (d, d),
            (Kind::Number, None) => (3, 0),
            (_, None) => (0, 0),
        };
        // A percent is the number rounded two places further, its point moved two places on.
        let shift = if self.kind == Kind::Percent { 2 } else { 0 };
        let digits = rounded(x.abs(), most, shift);
        let (mut int, mut frac) = match digits.split_once('.') {
            Some((i, f)) => (i.to_string(), f.to_string()),
            None => (digits, String::new()),
        };
        let moved: String = frac.drain(..shift as usize).collect();
        int.push_str(&moved);
        let int = match int.trim_start_matches('0') {
            "" => "0".to_string(),
            i => i.to_string(),
        };
        while frac.len() > least as usize && frac.ends_with('0') {
            frac.pop();
        }
        let width = if let Kind::Pad(w) = self.kind {
            w as usize
        } else {
            1
        };
        let mut int = format!("{int:0>width$}");
        if self.grouping {
            int = grouped(&int);
        }
        let point = if frac.is_empty() { "" } else { "." };
        format!("{sign}{int}{point}{frac}{percent}")
    }
}

/// A finite `x` at least 0 with `places` fraction digits: its shortest decimal, the one
/// `String(x)` writes, rounded half away from zero, as Chrome rounds it.
fn rounded(x: f64, places: u32, shift: u32) -> String {
    // Rust writes the same shortest digits, without an exponent.
    let shortest = format!("{x}");
    let (int, frac) = shortest.split_once('.').unwrap_or((&shortest, ""));
    let mut frac = frac.to_string();
    let width = (places + shift) as usize;
    let up_next = frac.as_bytes().get(width).is_some_and(|&d| d >= b'5');
    frac.truncate(width);
    while frac.len() < width {
        frac.push('0');
    }
    let mut digits = format!("{int}.{frac}");
    if up_next {
        up(&mut digits);
    }
    digits.trim_end_matches('.').to_string()
}

/// Adds one in the last digit place of a decimal.
fn up(digits: &mut String) {
    let mut bytes = std::mem::take(digits).into_bytes();
    let mut i = bytes.len();
    loop {
        if i == 0 {
            bytes.insert(0, b'1');
            break;
        }
        i -= 1;
        match bytes[i] {
            b'.' => continue,
            b'9' => bytes[i] = b'0',
            d => {
                bytes[i] = d + 1;
                break;
            }
        }
    }
    *digits = String::from_utf8(bytes).expect("digits");
}

fn grouped(int: &str) -> String {
    let mut out = String::new();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ties_go_away_from_zero() {
        let f = Format::parse("number|0|true").unwrap();
        assert_eq!(
            [0.5, 1.5, 2.5, -2.5, 999.5].map(|x| f.text(x)),
            ["1", "2", "3", "-3", "1,000"]
        );
        // Its shortest decimal, not the double a little under it.
        assert_eq!(Format::parse("number|2|true").unwrap().text(1.005), "1.01");
        assert_eq!(Format::parse("number|2|true").unwrap().text(0.125), "0.13");
    }

    #[test]
    fn specs_out_of_range_are_errors() {
        for bad in [
            "number|101|true",
            "pad||false",
            "pad|0|false",
            "pad|22|false",
            "pad|2|true",
            "x|1|true",
            "number|1|yes",
            "number|-1|true",
        ] {
            assert!(Format::parse(bad).is_err(), "{bad}");
        }
    }
}
