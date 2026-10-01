//! Shared background capability checks for inline CSS and utilities.
use takumi_core::style::{Background, BackgroundImage, BackgroundImages, FromCssStr, ToCss};

pub(crate) fn validate_image(image: &BackgroundImage) -> Result<(), String> {
    let method = match image {
        BackgroundImage::None => return Ok(()),
        BackgroundImage::Linear(g) => g.interpolation,
        BackgroundImage::Radial(g) => g.interpolation,
        BackgroundImage::Conic(g) => g.interpolation,
        BackgroundImage::Url(_) => {
            return Err("background URL images are unsupported; use an <Image> node".into());
        }
    };
    if gradient_interpolation(method).is_some() {
        return Ok(());
    }
    Err(format!(
        "unsupported gradient interpolation `{}`; use in srgb, in srgb-linear, in oklab, or in oklch",
        method.to_css_string()
    ))
}

/// Keep URLs and unsupported interpolation spaces closed for both static and dynamic CSS.
pub(crate) fn gradient_background_source(property: &str, source: &str) -> bool {
    if matches!(
        super::css_keyword(source).as_deref(),
        Some("initial" | "inherit" | "unset")
    ) {
        return true;
    }
    let admitted = |image: &BackgroundImage| validate_image(image).is_ok();
    match property {
        "background-image" => BackgroundImages::from_css_str(source)
            .is_ok_and(|images| !images.is_empty() && images.iter().all(admitted)),
        "background" => Background::from_css_str(source).is_ok_and(|value| admitted(&value.image)),
        _ => false,
    }
}

/// Match the upstream opaque type against the explicitly supported interpolation spaces.
/// Omitted CSS interpolation uses the parser's OKLab default.
pub(crate) fn gradient_interpolation(
    method: takumi_core::style::ColorInterpolationMethod,
) -> Option<valle_draw::program::GradientInterpolation> {
    use std::sync::OnceLock;
    use valle_draw::program::{GradientInterpolation as G, HueDirection as H};
    static METHODS: OnceLock<Vec<(takumi_core::style::ColorInterpolationMethod, G)>> =
        OnceLock::new();
    METHODS
        .get_or_init(|| {
            [
                ("in srgb", G::Srgb),
                ("in srgb-linear", G::LinearSrgb),
                ("in oklab", G::Oklab),
                ("in oklch", G::Oklch { hue: H::Shorter }),
                ("in oklch longer hue", G::Oklch { hue: H::Longer }),
                ("in oklch increasing hue", G::Oklch { hue: H::Increasing }),
                ("in oklch decreasing hue", G::Oklch { hue: H::Decreasing }),
            ]
            .into_iter()
            .map(|(css, mode)| {
                (
                    takumi_core::style::ColorInterpolationMethod::from_css_str(css)
                        .expect("valid interpolation"),
                    mode,
                )
            })
            .collect()
        })
        .iter()
        .find_map(|(parsed, mode)| (*parsed == method).then_some(*mode))
}
