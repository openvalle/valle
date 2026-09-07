//! Shared text style and placement types without domain-specific concepts. Measurement interfaces
//! reside in the measure module.

use serde::{Deserialize, Serialize};

use crate::color::Rgba;
use crate::geom::Point;
use crate::measure::{ResolvedTextStyle, TextMetrics};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum FontWeight {
    Normal,
    Bold,
    Weight(u16),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum FontStyle {
    Normal,
    Italic,
}

/// Optional text-style fields inherit from their parent; resolve the chain before drawing.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct TextStyle {
    pub color: Option<Rgba>,
    pub font_family: Option<String>,
    pub font_size: Option<f64>,
    pub font_weight: Option<FontWeight>,
    pub font_style: Option<FontStyle>,
}

impl TextStyle {
    /// Explicit fields override the base style.
    pub fn over(&self, base: &TextStyle) -> TextStyle {
        TextStyle {
            color: self.color.or(base.color),
            font_family: self
                .font_family
                .clone()
                .or_else(|| base.font_family.clone()),
            font_size: self.font_size.or(base.font_size),
            font_weight: self.font_weight.or(base.font_weight),
            font_style: self.font_style.or(base.font_style),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum HAlign {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum VAlign {
    Top,
    Middle,
    Bottom,
}

/// Measured, positioned text ready for drawing with its anchor and alignment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlacedText {
    pub text: String,
    pub anchor: Point,
    pub h_align: HAlign,
    pub v_align: VAlign,
    /// Clockwise rotation in degrees around the anchor.
    pub rotate: f64,
    pub metrics: TextMetrics,
    pub style: ResolvedTextStyle,
}

impl PlacedText {
    /// Axis-aligned size after rotation for layout margin calculations.
    pub fn footprint(&self) -> (f64, f64) {
        let (w, h) = (self.metrics.width, self.metrics.height());
        if self.rotate == 0.0 {
            return (w, h);
        }
        let (s, c) = crate::math::sin_cos(self.rotate.to_radians());
        (w * c.abs() + h * s.abs(), w * s.abs() + h * c.abs())
    }
}
