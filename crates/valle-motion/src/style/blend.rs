//! One admission/lowering table: upstream CSS parsing does not imply rendering support.

use takumi_core::style::BlendMode as Css;
use valle_draw::program::recording::BlendMode as Draw;

pub fn space(value: &str) -> Result<valle_draw::program::BlendSpace, String> {
    match value {
        "srgb" => Ok(valle_draw::program::BlendSpace::Srgb),
        "linear" => Ok(valle_draw::program::BlendSpace::Linear),
        _ => Err("mixBlendSpace must be `linear` or `srgb`".into()),
    }
}

pub fn lower(mode: Css) -> Result<Option<Draw>, String> {
    Ok(Some(match mode {
        Css::Normal => return Ok(None),
        Css::PlusLighter => Draw::Plus,
        Css::Multiply => Draw::Multiply,
        Css::Screen => Draw::Screen,
        Css::Overlay => Draw::Overlay,
        Css::Darken => Draw::Darken,
        Css::Lighten => Draw::Lighten,
        Css::ColorDodge => Draw::ColorDodge,
        Css::ColorBurn => Draw::ColorBurn,
        Css::HardLight => Draw::HardLight,
        Css::SoftLight => Draw::SoftLight,
        Css::Difference => Draw::Difference,
        Css::Exclusion => Draw::Exclusion,
        Css::Hue => Draw::Hue,
        Css::Saturation => Draw::Saturation,
        Css::Color => Draw::Color,
        Css::Luminosity => Draw::Luminosity,
        other => {
            return Err(format!(
                "mix-blend-mode {other:?} has no compositor implementation"
            ));
        }
    }))
}
