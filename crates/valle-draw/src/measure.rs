//! Host-supplied text measurement shared by geometry and layout consumers.

use crate::text::{FontStyle, FontWeight, TextStyle};
use serde::{Deserialize, Serialize};

/// Resolved text style with inheritance applied before drawing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedTextStyle {
    pub color: crate::color::Rgba,
    /// None delegates font selection to the host registry.
    pub font_family: Option<String>,
    pub font_size: f64,
    pub font_weight: FontWeight,
    pub font_style: FontStyle,
}

impl ResolvedTextStyle {
    /// Root style defaults to size 12 and normal weight; leave the family to host selection.
    pub fn root() -> Self {
        ResolvedTextStyle {
            color: crate::color::Rgba::rgb(0x3c, 0x3c, 0x41),
            font_family: None,
            font_size: 12.0,
            font_weight: FontWeight::Normal,
            font_style: FontStyle::Normal,
        }
    }
}

impl TextStyle {
    /// Override inherited fields that are explicitly provided.
    pub fn resolve(&self, base: &ResolvedTextStyle) -> ResolvedTextStyle {
        ResolvedTextStyle {
            color: self.color.unwrap_or(base.color),
            font_family: self
                .font_family
                .clone()
                .or_else(|| base.font_family.clone()),
            font_size: self.font_size.unwrap_or(base.font_size),
            font_weight: self.font_weight.unwrap_or(base.font_weight),
            font_style: self.font_style.unwrap_or(base.font_style),
        }
    }
}

/// Text metrics with positive ascent above and descent below the baseline.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextMetrics {
    pub width: f64,
    pub ascent: f64,
    pub descent: f64,
}

impl TextMetrics {
    pub fn height(&self) -> f64 {
        self.ascent + self.descent
    }
}

pub trait TextMeasure {
    fn measure(&self, text: &str, style: &ResolvedTextStyle) -> TextMetrics;
}

/// Approximate measurement for tests and fontless checks, counting CJK/full-width characters as
/// double width. Production rendering must supply real font metrics.
#[derive(Debug, Clone, Copy)]
pub struct ApproxTextMeasure {
    /// Half-width character advance divided by font size; 0.6 approximates common sans-serif
    /// averages.
    pub advance_ratio: f64,
    pub ascent_ratio: f64,
    pub descent_ratio: f64,
}

impl Default for ApproxTextMeasure {
    fn default() -> Self {
        ApproxTextMeasure {
            advance_ratio: 0.6,
            ascent_ratio: 0.8,
            descent_ratio: 0.2,
        }
    }
}

impl TextMeasure for ApproxTextMeasure {
    fn measure(&self, text: &str, style: &ResolvedTextStyle) -> TextMetrics {
        let units: f64 = text
            .chars()
            .map(|c| if is_wide(c) { 2.0 } else { 1.0 })
            .sum();
        let bold = matches!(style.font_weight, FontWeight::Bold)
            || matches!(style.font_weight, FontWeight::Weight(w) if w >= 600);
        // Account for slightly wider bold text.
        let w = units * style.font_size * self.advance_ratio * if bold { 1.05 } else { 1.0 };
        TextMetrics {
            width: w,
            ascent: style.font_size * self.ascent_ratio,
            descent: style.font_size * self.descent_ratio,
        }
    }
}

/// Approximate East Asian full-width classification.
fn is_wide(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x115F      // Hangul Jamo.
        | 0x2E80..=0x303E    // CJK radicals and punctuation.
        | 0x3041..=0x33FF    // Kana and compatibility characters.
        | 0x3400..=0x4DBF    // CJK Extension A.
        | 0x4E00..=0x9FFF    // CJK Unified Ideographs.
        | 0xA000..=0xA4CF    // Yi syllables and radicals.
        | 0xAC00..=0xD7A3    // Hangul syllables.
        | 0xF900..=0xFAFF    // Compatibility ideographs.
        | 0xFE30..=0xFE6F    // Vertical and small forms.
        | 0xFF00..=0xFF60    // Full-width ASCII forms.
        | 0xFFE0..=0xFFE6
        | 0x20000..=0x2FFFD  // CJK Extension B and later blocks.
        | 0x30000..=0x3FFFD
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Rgba;

    #[test]
    fn cjk_counts_double() {
        let m = ApproxTextMeasure::default();
        let s = ResolvedTextStyle {
            font_size: 10.0,
            ..ResolvedTextStyle::root()
        };
        // Three half-width characters.
        assert_eq!(m.measure("abc", &s).width, 18.0);
        // Two full-width characters occupy four width units.
        assert_eq!(m.measure("营收", &s).width, 24.0);
        // Mixed half-width and full-width text.
        assert_eq!(m.measure("Q1营收", &s).width, 36.0);
    }

    #[test]
    fn resolve_walks_the_inheritance_chain() {
        let root = ResolvedTextStyle::root();
        let mid = TextStyle {
            font_size: Some(20.0),
            ..TextStyle::default()
        };
        let leaf = TextStyle {
            color: Some(Rgba::rgb(1, 2, 3)),
            ..TextStyle::default()
        };
        let r = leaf.resolve(&mid.resolve(&root));
        assert_eq!(
            r.font_size, 20.0,
            "an intermediate font size must override the root"
        );
        assert_eq!(
            r.color,
            Rgba::rgb(1, 2, 3),
            "the leaf color must override inherited color"
        );
        assert_eq!(
            r.font_weight,
            FontWeight::Normal,
            "inherit the root when no override exists"
        );
    }

    #[test]
    fn bold_is_measured_wider_than_normal() {
        let m = ApproxTextMeasure::default();
        let n = ResolvedTextStyle {
            font_size: 10.0,
            ..ResolvedTextStyle::root()
        };
        let b = ResolvedTextStyle {
            font_weight: FontWeight::Bold,
            ..n.clone()
        };
        assert!(m.measure("title", &b).width > m.measure("title", &n).width);
        // Numeric bold weights also affect width.
        let w700 = ResolvedTextStyle {
            font_weight: FontWeight::Weight(700),
            ..n.clone()
        };
        assert_eq!(
            m.measure("title", &w700).width,
            m.measure("title", &b).width
        );
    }
}
