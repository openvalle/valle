use crate::frame::CompositeBlendMode as TimelineBlend;
use thiserror::Error;
use valle_draw::{math, program::BlendMode as DrawBlendMode};

use super::{
    ColorMathError, PixelError, PremulRgba32, extended_srgb_to_working, working_to_extended_srgb,
};

/// The complete creative blend set shared by Timeline clips and Motion DrawPrograms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ReferenceBlendMode {
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    LinearBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlendDomain {
    WorkingLinear,
    PerceptualSrgbExtended,
}

impl ReferenceBlendMode {
    pub const fn domain(self) -> BlendDomain {
        match self {
            Self::Normal => BlendDomain::WorkingLinear,
            _ => BlendDomain::PerceptualSrgbExtended,
        }
    }
}

impl From<TimelineBlend> for ReferenceBlendMode {
    fn from(value: TimelineBlend) -> Self {
        match value {
            TimelineBlend::Normal => Self::Normal,
            TimelineBlend::Multiply => Self::Multiply,
            TimelineBlend::Screen => Self::Screen,
            TimelineBlend::Overlay => Self::Overlay,
            TimelineBlend::Darken => Self::Darken,
            TimelineBlend::Lighten => Self::Lighten,
            TimelineBlend::ColorDodge => Self::ColorDodge,
            TimelineBlend::ColorBurn => Self::ColorBurn,
            TimelineBlend::LinearBurn => Self::LinearBurn,
            TimelineBlend::HardLight => Self::HardLight,
            TimelineBlend::SoftLight => Self::SoftLight,
            TimelineBlend::Difference => Self::Difference,
            TimelineBlend::Exclusion => Self::Exclusion,
            TimelineBlend::Hue => Self::Hue,
            TimelineBlend::Saturation => Self::Saturation,
            TimelineBlend::Color => Self::Color,
            TimelineBlend::Luminosity => Self::Luminosity,
        }
    }
}

impl From<DrawBlendMode> for ReferenceBlendMode {
    fn from(value: DrawBlendMode) -> Self {
        match value {
            DrawBlendMode::Normal => Self::Normal,
            DrawBlendMode::Multiply => Self::Multiply,
            DrawBlendMode::Screen => Self::Screen,
            DrawBlendMode::Overlay => Self::Overlay,
            DrawBlendMode::Darken => Self::Darken,
            DrawBlendMode::Lighten => Self::Lighten,
            DrawBlendMode::ColorDodge => Self::ColorDodge,
            DrawBlendMode::ColorBurn => Self::ColorBurn,
            DrawBlendMode::LinearBurn => Self::LinearBurn,
            DrawBlendMode::HardLight => Self::HardLight,
            DrawBlendMode::SoftLight => Self::SoftLight,
            DrawBlendMode::Difference => Self::Difference,
            DrawBlendMode::Exclusion => Self::Exclusion,
            DrawBlendMode::Hue => Self::Hue,
            DrawBlendMode::Saturation => Self::Saturation,
            DrawBlendMode::Color => Self::Color,
            DrawBlendMode::Luminosity => Self::Luminosity,
        }
    }
}

/// Build the effective source required by the W3C compositing formula. The returned pixel still
/// contains only source contribution; callers choose whether it is composited into a local group
/// accumulator or directly over the layer backdrop.
pub fn effective_blend_source(
    source: PremulRgba32,
    destination: PremulRgba32,
    mode: ReferenceBlendMode,
) -> Result<PremulRgba32, BlendError> {
    if mode == ReferenceBlendMode::Normal || source.alpha() == 0.0 {
        return Ok(source);
    }
    let source_rgb = working_to_extended_srgb(source.straight_rgb()?)?;
    let destination_rgb = working_to_extended_srgb(destination.straight_rgb()?)?;
    let blended = blend_rgb(destination_rgb, source_rgb, mode)?;
    let destination_alpha = destination.alpha();
    let effective = [0, 1, 2].map(|channel| {
        (1.0 - destination_alpha) * source_rgb[channel] + destination_alpha * blended[channel]
    });
    let working = extended_srgb_to_working(effective)?;
    PremulRgba32::from_straight(working, source.alpha()).map_err(Into::into)
}

pub fn blend_over(
    source: PremulRgba32,
    destination: PremulRgba32,
    mode: ReferenceBlendMode,
) -> Result<PremulRgba32, BlendError> {
    effective_blend_source(source, destination, mode)?
        .source_over(destination)
        .map_err(Into::into)
}

/// Straight RGB blend function in the mode's fixed perceptual extended-sRGB domain.
pub fn blend_rgb(
    backdrop: [f32; 3],
    source: [f32; 3],
    mode: ReferenceBlendMode,
) -> Result<[f32; 3], BlendError> {
    if !backdrop
        .iter()
        .chain(&source)
        .all(|channel| channel.is_finite())
    {
        return Err(BlendError::NonFinite);
    }
    let result = match mode {
        ReferenceBlendMode::Normal => source,
        ReferenceBlendMode::Multiply => zip(backdrop, source, |b, s| b * s),
        ReferenceBlendMode::Screen => zip(backdrop, source, |b, s| b + s - b * s),
        ReferenceBlendMode::Overlay => zip(backdrop, source, overlay),
        ReferenceBlendMode::Darken => zip(backdrop, source, f32::min),
        ReferenceBlendMode::Lighten => zip(backdrop, source, f32::max),
        ReferenceBlendMode::ColorDodge => zip(backdrop, source, color_dodge),
        ReferenceBlendMode::ColorBurn => zip(backdrop, source, color_burn),
        ReferenceBlendMode::LinearBurn => zip(backdrop, source, |b, s| (b + s - 1.0).max(0.0)),
        ReferenceBlendMode::HardLight => zip(backdrop, source, |b, s| overlay(s, b)),
        ReferenceBlendMode::SoftLight => zip(backdrop, source, soft_light),
        ReferenceBlendMode::Difference => zip(backdrop, source, |b, s| (b - s).abs()),
        ReferenceBlendMode::Exclusion => zip(backdrop, source, |b, s| b + s - 2.0 * b * s),
        ReferenceBlendMode::Hue => {
            set_lum(set_sat(source, saturation(backdrop)), luminosity(backdrop))
        }
        ReferenceBlendMode::Saturation => {
            set_lum(set_sat(backdrop, saturation(source)), luminosity(backdrop))
        }
        ReferenceBlendMode::Color => set_lum(source, luminosity(backdrop)),
        ReferenceBlendMode::Luminosity => set_lum(backdrop, luminosity(source)),
    };
    if result.iter().all(|channel| channel.is_finite()) {
        Ok(result)
    } else {
        Err(BlendError::NonFinite)
    }
}

fn zip(backdrop: [f32; 3], source: [f32; 3], operation: impl Fn(f32, f32) -> f32) -> [f32; 3] {
    [
        operation(backdrop[0], source[0]),
        operation(backdrop[1], source[1]),
        operation(backdrop[2], source[2]),
    ]
}

fn overlay(backdrop: f32, source: f32) -> f32 {
    if backdrop <= 0.5 {
        2.0 * backdrop * source
    } else {
        1.0 - 2.0 * (1.0 - backdrop) * (1.0 - source)
    }
}

fn color_dodge(backdrop: f32, source: f32) -> f32 {
    if backdrop == 0.0 {
        0.0
    } else if source >= 1.0 {
        1.0
    } else {
        (backdrop / (1.0 - source)).min(1.0)
    }
}

fn color_burn(backdrop: f32, source: f32) -> f32 {
    if backdrop == 1.0 {
        1.0
    } else if source <= 0.0 {
        0.0
    } else {
        1.0 - ((1.0 - backdrop) / source).min(1.0)
    }
}

fn soft_light(backdrop: f32, source: f32) -> f32 {
    if source <= 0.5 {
        backdrop - (1.0 - 2.0 * source) * backdrop * (1.0 - backdrop)
    } else {
        let d = if backdrop <= 0.25 {
            ((16.0 * backdrop - 12.0) * backdrop + 4.0) * backdrop
        } else {
            math::sqrt(f64::from(backdrop)) as f32
        };
        backdrop + (2.0 * source - 1.0) * (d - backdrop)
    }
}

fn luminosity(color: [f32; 3]) -> f32 {
    0.3 * color[0] + 0.59 * color[1] + 0.11 * color[2]
}

fn saturation(color: [f32; 3]) -> f32 {
    color.into_iter().fold(f32::NEG_INFINITY, f32::max)
        - color.into_iter().fold(f32::INFINITY, f32::min)
}

fn set_lum(mut color: [f32; 3], target: f32) -> [f32; 3] {
    let difference = target - luminosity(color);
    for channel in &mut color {
        *channel += difference;
    }
    clip_color(color)
}

fn clip_color(mut color: [f32; 3]) -> [f32; 3] {
    let luminosity = luminosity(color);
    let minimum = color.into_iter().fold(f32::INFINITY, f32::min);
    let maximum = color.into_iter().fold(f32::NEG_INFINITY, f32::max);
    if minimum < 0.0 {
        let denominator = luminosity - minimum;
        if denominator != 0.0 {
            for channel in &mut color {
                *channel = luminosity + (*channel - luminosity) * luminosity / denominator;
            }
        }
    }
    if maximum > 1.0 {
        let denominator = maximum - luminosity;
        if denominator != 0.0 {
            for channel in &mut color {
                *channel = luminosity + (*channel - luminosity) * (1.0 - luminosity) / denominator;
            }
        }
    }
    color
}

fn set_sat(mut color: [f32; 3], target: f32) -> [f32; 3] {
    let mut indices = [0usize, 1, 2];
    indices.sort_by(|left, right| color[*left].total_cmp(&color[*right]));
    let [minimum, middle, maximum] = indices;
    if color[maximum] > color[minimum] {
        color[middle] =
            (color[middle] - color[minimum]) * target / (color[maximum] - color[minimum]);
        color[maximum] = target;
    } else {
        color[middle] = 0.0;
        color[maximum] = 0.0;
    }
    color[minimum] = 0.0;
    color
}

#[derive(Debug, Error, Clone, PartialEq)]
pub enum BlendError {
    #[error(transparent)]
    Pixel(#[from] PixelError),
    #[error(transparent)]
    Color(#[from] ColorMathError),
    #[error("blend formula produced a non-finite channel")]
    NonFinite,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separable_formulas_match_hand_values() {
        let backdrop = [0.25, 0.5, 0.75];
        let source = [0.8, 0.4, 0.2];
        assert_rgb_close(
            blend_rgb(backdrop, source, ReferenceBlendMode::Multiply).unwrap(),
            [0.2, 0.2, 0.15],
        );
        assert_rgb_close(
            blend_rgb(backdrop, source, ReferenceBlendMode::Screen).unwrap(),
            [0.85, 0.7, 0.8],
        );
        assert_rgb_close(
            blend_rgb(backdrop, source, ReferenceBlendMode::Difference).unwrap(),
            [0.55, 0.1, 0.55],
        );
    }

    #[test]
    fn transparent_destination_never_changes_effective_source() {
        let source = PremulRgba32::from_straight([0.2, 0.7, 1.5], 0.4).unwrap();
        for mode in all_modes() {
            let effective =
                effective_blend_source(source, PremulRgba32::TRANSPARENT, mode).unwrap();
            assert!(effective.approx_eq(source, 2e-6), "{mode:?}: {effective:?}");
        }
    }

    #[test]
    fn blend_alpha_always_uses_porter_duff_source_over() {
        let source = PremulRgba32::from_straight([0.8, 0.2, 0.1], 0.25).unwrap();
        let destination = PremulRgba32::from_straight([0.1, 0.3, 0.7], 0.5).unwrap();
        for mode in all_modes() {
            let output = blend_over(source, destination, mode).unwrap();
            assert!((output.alpha() - 0.625).abs() < 1e-7, "{mode:?}");
        }
    }

    #[test]
    fn timeline_and_draw_modes_share_one_closed_enum() {
        assert_eq!(
            ReferenceBlendMode::from(TimelineBlend::Hue),
            ReferenceBlendMode::from(DrawBlendMode::Hue)
        );
        assert_eq!(
            ReferenceBlendMode::from(TimelineBlend::LinearBurn),
            ReferenceBlendMode::LinearBurn
        );
    }

    #[test]
    fn every_mode_is_finite_across_extended_srgb_fixture_grid() {
        let values = [-0.5, 0.0, 0.2, 0.5, 1.0, 2.0];
        for mode in all_modes() {
            for backdrop in values {
                for source in values {
                    let result =
                        blend_rgb([backdrop, 0.25, 1.25], [source, 0.75, -0.25], mode).unwrap();
                    assert!(result.into_iter().all(f32::is_finite), "{mode:?}");
                }
            }
        }
        assert!(blend_rgb([f32::NAN, 0.0, 0.0], [0.0; 3], ReferenceBlendMode::Darken).is_err());
    }

    fn all_modes() -> [ReferenceBlendMode; 17] {
        [
            ReferenceBlendMode::Normal,
            ReferenceBlendMode::Multiply,
            ReferenceBlendMode::Screen,
            ReferenceBlendMode::Overlay,
            ReferenceBlendMode::Darken,
            ReferenceBlendMode::Lighten,
            ReferenceBlendMode::ColorDodge,
            ReferenceBlendMode::ColorBurn,
            ReferenceBlendMode::LinearBurn,
            ReferenceBlendMode::HardLight,
            ReferenceBlendMode::SoftLight,
            ReferenceBlendMode::Difference,
            ReferenceBlendMode::Exclusion,
            ReferenceBlendMode::Hue,
            ReferenceBlendMode::Saturation,
            ReferenceBlendMode::Color,
            ReferenceBlendMode::Luminosity,
        ]
    }

    fn assert_rgb_close(actual: [f32; 3], expected: [f32; 3]) {
        for (actual, expected) in actual.into_iter().zip(expected) {
            assert!((actual - expected).abs() < 1e-6, "{actual} != {expected}");
        }
    }
}
