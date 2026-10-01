use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

static SRGB8_LINEAR_LUT: OnceLock<[f64; 256]> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct LinearColor {
    pub red: f32,
    pub green: f32,
    pub blue: f32,
    pub alpha: f32,
}

/// Author color in straight-alpha linear sRGB. Keeping the unassociated channels preserves the
/// hue of a fully transparent endpoint until interpolation applies alpha weighting. DrawProgram
/// paints convert this to premultiplied linear Rec.2020 only at the paint boundary.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct AuthorColor {
    pub red: f64,
    pub green: f64,
    pub blue: f64,
    pub alpha: f64,
}

impl AuthorColor {
    pub fn from_srgb8(color: crate::Rgba) -> Self {
        Self {
            red: decode_srgb_channel(f64::from(color.r) / 255.0),
            green: decode_srgb_channel(f64::from(color.g) / 255.0),
            blue: decode_srgb_channel(f64::from(color.b) / 255.0),
            alpha: f64::from(color.a) / 255.0,
        }
    }

    pub fn is_finite(self) -> bool {
        self.red.is_finite()
            && self.green.is_finite()
            && self.blue.is_finite()
            && self.alpha.is_finite()
            && (0.0..=1.0).contains(&self.alpha)
    }

    pub fn to_working(self) -> LinearColor {
        from_linear_srgb([self.red, self.green, self.blue], self.alpha as f32)
    }

    pub fn with_opacity(self, opacity: f64) -> Self {
        Self {
            alpha: self.alpha * opacity.clamp(0.0, 1.0),
            ..self
        }
    }

    pub fn to_srgb_straight(self) -> [f32; 4] {
        [
            encode_srgb_channel(self.red) as f32,
            encode_srgb_channel(self.green) as f32,
            encode_srgb_channel(self.blue) as f32,
            self.alpha as f32,
        ]
    }

    /// Only for CSS/layout and existing 8-bit leaf interfaces. Motion paint uses `to_working`.
    pub fn to_srgb8(self) -> crate::Rgba {
        let byte = |linear: f64| {
            (encode_srgb_channel(linear).clamp(0.0, 1.0) * 255.0 + 1e-12).round() as u8
        };
        crate::Rgba::new(
            byte(self.red),
            byte(self.green),
            byte(self.blue),
            (self.alpha.clamp(0.0, 1.0) * 255.0 + 1e-12).round() as u8,
        )
    }
}

impl LinearColor {
    pub const fn new(red: f32, green: f32, blue: f32, alpha: f32) -> Self {
        Self {
            red,
            green,
            blue,
            alpha,
        }
    }

    /// Decode an author sRGB straight-alpha color into Valle's Linear Rec.2020 premultiplied
    /// working domain. Motion and caption producers use this before a DrawProgram is validated;
    /// executors never repeat author-color interpretation.
    pub fn from_srgb8(color: crate::Rgba) -> Self {
        let lut = SRGB8_LINEAR_LUT.get_or_init(|| {
            core::array::from_fn(|encoded| decode_srgb_channel(f64::from(encoded as f32 / 255.0)))
        });
        let alpha = f32::from(color.a) / 255.0;
        from_linear_srgb(
            [
                lut[color.r as usize],
                lut[color.g as usize],
                lut[color.b as usize],
            ],
            alpha,
        )
    }

    pub fn from_srgb_straight(color: [f32; 4]) -> Self {
        let srgb = [
            decode_srgb_channel(f64::from(color[0])),
            decode_srgb_channel(f64::from(color[1])),
            decode_srgb_channel(f64::from(color[2])),
        ];
        from_linear_srgb(srgb, color[3])
    }

    pub fn scale_opacity(self, opacity: f32) -> Self {
        Self {
            red: self.red * opacity,
            green: self.green * opacity,
            blue: self.blue * opacity,
            alpha: self.alpha * opacity,
        }
    }
}

/// Shader author color -> linear sRGB, preserving RGB even when alpha is zero.
/// This is intentionally straight alpha; coverage is applied at the shader output boundary.
pub fn decode_srgb_straight(color: [f32; 4]) -> [f32; 4] {
    [
        decode_srgb_channel(f64::from(color[0])) as f32,
        decode_srgb_channel(f64::from(color[1])) as f32,
        decode_srgb_channel(f64::from(color[2])) as f32,
        color[3],
    ]
}

pub(super) fn decode_srgb_channel(encoded: f64) -> f64 {
    if encoded.abs() <= 0.04045 {
        encoded / 12.92
    } else {
        encoded.signum() * crate::math::pow((encoded.abs() + 0.055) / 1.055, 2.4)
    }
}

pub(super) fn encode_srgb_channel(linear: f64) -> f64 {
    if linear.abs() <= 0.0031308 {
        linear * 12.92
    } else {
        linear.signum() * (1.055 * crate::math::pow(linear.abs(), 1.0 / 2.4) - 0.055)
    }
}

pub(super) fn from_linear_srgb(srgb: [f64; 3], alpha: f32) -> LinearColor {
    // Rec.709/sRGB D65 linear primaries -> Rec.2020 D65 linear primaries.
    let rec2020 = [
        0.627_403_895_934_699 * srgb[0]
            + 0.329_283_038_377_884 * srgb[1]
            + 0.043_313_065_687_417 * srgb[2],
        0.069_097_289_358_232 * srgb[0]
            + 0.919_540_395_075_459 * srgb[1]
            + 0.011_362_315_566_309 * srgb[2],
        0.016_391_438_875_151 * srgb[0]
            + 0.088_013_307_877_226 * srgb[1]
            + 0.895_595_253_247_623 * srgb[2],
    ];
    LinearColor {
        red: (rec2020[0] as f32) * alpha,
        green: (rec2020[1] as f32) * alpha,
        blue: (rec2020[2] as f32) * alpha,
        alpha,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GradientStop {
    pub offset: f32,
    pub color: LinearColor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum SpreadMode {
    #[default]
    Pad,
    Repeat,
    Reflect,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Paint {
    Solid(LinearColor),
    LinearGradient {
        start: [f64; 2],
        end: [f64; 2],
        stops: Vec<GradientStop>,
        spread: SpreadMode,
    },
    RadialGradient {
        center: [f64; 2],
        radii: [f64; 2],
        stops: Vec<GradientStop>,
        spread: SpreadMode,
    },
    TwoCircleGradient {
        start: [f64; 2],
        start_radius: f64,
        end: [f64; 2],
        end_radius: f64,
        stops: Vec<GradientStop>,
        spread: SpreadMode,
    },
    ConicGradient {
        center: [f64; 2],
        start_angle_degrees: f64,
        sweep_angle_degrees: f64,
        stops: Vec<GradientStop>,
        spread: SpreadMode,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb8_lookup_is_bit_exact_with_the_float_decoder() {
        for value in 0..=u8::MAX {
            let color = crate::Rgba {
                r: value,
                g: u8::MAX - value,
                b: value.wrapping_mul(17),
                a: value.wrapping_mul(29),
            };
            assert_eq!(
                LinearColor::from_srgb8(color),
                LinearColor::from_srgb_straight([
                    f32::from(color.r) / 255.0,
                    f32::from(color.g) / 255.0,
                    f32::from(color.b) / 255.0,
                    f32::from(color.a) / 255.0,
                ])
            );
        }
    }
}
