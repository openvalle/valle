//! Narrow parser for Timeline string-encoded sRGB colors.

use serde::{Deserialize, Serialize};

/// A validated-on-use `#RRGGBB` or `#RRGGBBAA` color token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Color(pub String);

impl Color {
    /// Parse RGBA bytes; six-digit RGB defaults alpha to 255. Invalid formats return `None`.
    pub fn parse_rgba(&self) -> Option<[u8; 4]> {
        let s = self.0.strip_prefix('#')?;
        let bytes = match s.len() {
            6 => {
                let r = u8::from_str_radix(&s[0..2], 16).ok()?;
                let g = u8::from_str_radix(&s[2..4], 16).ok()?;
                let b = u8::from_str_radix(&s[4..6], 16).ok()?;
                [r, g, b, 255]
            }
            8 => {
                let r = u8::from_str_radix(&s[0..2], 16).ok()?;
                let g = u8::from_str_radix(&s[2..4], 16).ok()?;
                let b = u8::from_str_radix(&s[4..6], 16).ok()?;
                let a = u8::from_str_radix(&s[6..8], 16).ok()?;
                [r, g, b, a]
            }
            _ => return None,
        };
        Some(bytes)
    }

    /// Whether the value is six- or eight-digit hexadecimal color with a leading hash.
    pub fn is_valid(&self) -> bool {
        self.parse_rgba().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rgb_and_rgba() {
        assert_eq!(
            Color("#ffffff".into()).parse_rgba(),
            Some([255, 255, 255, 255])
        );
        assert_eq!(Color("#000000".into()).parse_rgba(), Some([0, 0, 0, 255]));
        assert_eq!(Color("#00000080".into()).parse_rgba(), Some([0, 0, 0, 128]));
        assert_eq!(
            Color("#ffcc00".into()).parse_rgba(),
            Some([255, 204, 0, 255])
        );
    }

    #[test]
    fn rejects_bad_formats() {
        assert!(!Color("ffffff".into()).is_valid()); // Missing leading hash.
        assert!(!Color("#fff".into()).is_valid()); // Three-digit shorthand is unsupported.
        assert!(!Color("#gggggg".into()).is_valid()); // Non-hexadecimal digits.
        assert!(!Color("#ffffff00ff".into()).is_valid()); // Too many digits.
    }

    #[test]
    fn serde_is_transparent_string() {
        let c: Color = serde_json::from_str("\"#ffcc00\"").unwrap();
        assert_eq!(c, Color("#ffcc00".into()));
        assert_eq!(serde_json::to_string(&c).unwrap(), "\"#ffcc00\"");
    }
}
