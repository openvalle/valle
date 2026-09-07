//! CSS/ECharts color strings converted to RGBA independently of Timeline types. Support short and
//! full hex, rgb/rgba functions, and common CSS names.

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

use crate::math;

/// Eight-bit RGBA with straight alpha; backends handle premultiplication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const TRANSPARENT: Rgba = Rgba::new(0, 0, 0, 0);

    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Rgba { r, g, b, a }
    }

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Rgba::new(r, g, b, 255)
    }

    /// Multiply the existing opacity.
    pub fn with_opacity(self, o: f64) -> Self {
        let a = (self.a as f64 * o.clamp(0.0, 1.0))
            .round()
            .clamp(0.0, 255.0);
        Rgba { a: a as u8, ..self }
    }

    /// Mix RGB toward the target while preserving alpha.
    pub fn mix(self, to: Rgba, t: f64) -> Rgba {
        let t = t.clamp(0.0, 1.0);
        let m = |a: u8, b: u8| (a as f64 + (b as f64 - a as f64) * t).round() as u8;
        Rgba {
            r: m(self.r, to.r),
            g: m(self.g, to.g),
            b: m(self.b, to.b),
            a: self.a,
        }
    }

    /// Relative luminance for contrast calculations.
    pub fn relative_luminance(self) -> f64 {
        let f = |c: u8| {
            let x = c as f64 / 255.0;
            if x <= 0.03928 {
                x / 12.92
            } else {
                math::pow((x + 0.055) / 1.055, 2.4)
            }
        };
        0.2126 * f(self.r) + 0.7152 * f(self.g) + 0.0722 * f(self.b)
    }

    /// Contrast ratio from 1 for identical colors to 21 for black and white.
    pub fn contrast(self, other: Rgba) -> f64 {
        let (a, b) = (self.relative_luminance(), other.relative_luminance());
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    /// Format as #rrggbb when opaque, otherwise #rrggbbaa.
    pub fn to_hex(self) -> String {
        if self.a == 255 {
            format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
        } else {
            format!("#{:02x}{:02x}{:02x}{:02x}", self.r, self.g, self.b, self.a)
        }
    }

    /// Parse a color string, returning None for the caller to diagnose.
    pub fn parse(s: &str) -> Option<Rgba> {
        let s = s.trim();
        if let Some(hex) = s.strip_prefix('#') {
            return parse_hex(hex);
        }
        if let Some(rest) = s.strip_prefix("rgba").and_then(strip_parens) {
            return parse_rgb_fn(rest, true);
        }
        if let Some(rest) = s.strip_prefix("rgb").and_then(strip_parens) {
            return parse_rgb_fn(rest, false);
        }
        named(&s.to_ascii_lowercase())
    }
}

impl Serialize for Rgba {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_hex())
    }
}

/// Deserialize the same color-string wire format used by Serialize; reject invalid values rather
/// than substituting transparent black.
impl<'de> Deserialize<'de> for Rgba {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Rgba::parse(&s).ok_or_else(|| de::Error::custom(format!("invalid color: {s}")))
    }
}

fn strip_parens(s: &str) -> Option<&str> {
    s.trim_start().strip_prefix('(')?.strip_suffix(')')
}

fn parse_hex(hex: &str) -> Option<Rgba> {
    let b = hex.as_bytes();
    let d = |i: usize| -> Option<u8> { (b[i] as char).to_digit(16).map(|v| v as u8) };
    match b.len() {
        // Expand each short-hex digit by repetition: f becomes ff.
        3 | 4 => {
            let x = |i: usize| d(i).map(|v| v * 17);
            Some(Rgba::new(
                x(0)?,
                x(1)?,
                x(2)?,
                if b.len() == 4 { x(3)? } else { 255 },
            ))
        }
        6 | 8 => {
            let x = |i: usize| Some(d(i)? * 16 + d(i + 1)?);
            Some(Rgba::new(
                x(0)?,
                x(2)?,
                x(4)?,
                if b.len() == 8 { x(6)? } else { 255 },
            ))
        }
        _ => None,
    }
}

/// Parse RGB integers or percentages and optional alpha in 0..1.
fn parse_rgb_fn(args: &str, with_alpha: bool) -> Option<Rgba> {
    let parts: Vec<&str> = args.split(',').map(str::trim).collect();
    if parts.len() != if with_alpha { 4 } else { 3 } {
        return None;
    }
    // Reject non-finite values before float-to-byte conversion can silently turn NaN into zero.
    let chan = |s: &str| -> Option<u8> {
        let v = if let Some(p) = s.strip_suffix('%') {
            p.trim().parse::<f64>().ok()? * 2.55
        } else {
            s.parse::<f64>().ok()?
        };
        v.is_finite().then(|| v.round().clamp(0.0, 255.0) as u8)
    };
    let a = if with_alpha {
        let v: f64 = parts[3].parse().ok()?;
        v.is_finite()
            .then(|| (v.clamp(0.0, 1.0) * 255.0).round() as u8)?
    } else {
        255
    };
    Some(Rgba::new(
        chan(parts[0])?,
        chan(parts[1])?,
        chan(parts[2])?,
        a,
    ))
}

/// Common CSS color names used by the supported authoring surface.
fn named(s: &str) -> Option<Rgba> {
    Some(match s {
        "transparent" | "none" => Rgba::TRANSPARENT,
        "black" => Rgba::rgb(0, 0, 0),
        "white" => Rgba::rgb(255, 255, 255),
        "red" => Rgba::rgb(255, 0, 0),
        "green" => Rgba::rgb(0, 128, 0),
        "lime" => Rgba::rgb(0, 255, 0),
        "blue" => Rgba::rgb(0, 0, 255),
        "yellow" => Rgba::rgb(255, 255, 0),
        "cyan" | "aqua" => Rgba::rgb(0, 255, 255),
        "magenta" | "fuchsia" => Rgba::rgb(255, 0, 255),
        "orange" => Rgba::rgb(255, 165, 0),
        "purple" => Rgba::rgb(128, 0, 128),
        "pink" => Rgba::rgb(255, 192, 203),
        "brown" => Rgba::rgb(165, 42, 42),
        "gray" | "grey" => Rgba::rgb(128, 128, 128),
        "lightgray" | "lightgrey" => Rgba::rgb(211, 211, 211),
        "darkgray" | "darkgrey" => Rgba::rgb(169, 169, 169),
        "silver" => Rgba::rgb(192, 192, 192),
        "navy" => Rgba::rgb(0, 0, 128),
        "teal" => Rgba::rgb(0, 128, 128),
        _ => return None,
    })
}

/// Expose RGBA as a TypeScript string to match its serialized color representation.
#[cfg(feature = "ts")]
impl ts_rs::TS for Rgba {
    type WithoutGenerics = Rgba;
    type OptionInnerType = Rgba;

    fn name(_: &ts_rs::Config) -> String {
        "string".into()
    }

    fn inline(cfg: &ts_rs::Config) -> String {
        <Self as ts_rs::TS>::name(cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_echarts_color_dialect() {
        assert_eq!(Rgba::parse("#5470c6"), Some(Rgba::rgb(0x54, 0x70, 0xc6)));
        // Short-hex digits repeat rather than shift.
        assert_eq!(Rgba::parse("#f0a"), Some(Rgba::rgb(0xff, 0x00, 0xaa)));
        assert_eq!(Rgba::parse("#0000"), Some(Rgba::TRANSPARENT));
        assert_eq!(
            Rgba::parse("#12345678"),
            Some(Rgba::new(0x12, 0x34, 0x56, 0x78))
        );
        assert_eq!(Rgba::parse("rgb(1, 2, 3)"), Some(Rgba::rgb(1, 2, 3)));
        // Transparent rgba is a valid color.
        assert_eq!(Rgba::parse("rgba(0,0,0,0)"), Some(Rgba::TRANSPARENT));
        assert_eq!(
            Rgba::parse("rgba(234,237,245,0.5)"),
            Some(Rgba::new(234, 237, 245, 128))
        );
        assert_eq!(Rgba::parse("rgb(100%, 0%, 0%)"), Some(Rgba::rgb(255, 0, 0)));
        assert_eq!(Rgba::parse("transparent"), Some(Rgba::TRANSPARENT));
        assert_eq!(Rgba::parse("LightGray"), Some(Rgba::rgb(211, 211, 211)));
    }

    #[test]
    fn rejects_what_it_cannot_represent() {
        // Reject malformed strings; gradients and patterns use separate object forms.
        assert_eq!(Rgba::parse("#12345"), None);
        assert_eq!(Rgba::parse("rgb(1,2)"), None);
        assert_eq!(Rgba::parse("hsl(0, 100%, 50%)"), None);
        assert_eq!(Rgba::parse("rebeccapurple"), None);
    }

    #[test]
    fn hex_roundtrip_drops_alpha_only_when_opaque() {
        assert_eq!(Rgba::rgb(0x54, 0x70, 0xc6).to_hex(), "#5470c6");
        assert_eq!(Rgba::new(1, 2, 3, 4).to_hex(), "#01020304");
        assert_eq!(Rgba::rgb(255, 0, 0).with_opacity(0.5).to_hex(), "#ff000080");
    }
}
