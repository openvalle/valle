//! Shared background capability checks for inline CSS and utilities.
use takumi_core::style::{Background, BackgroundImage, BackgroundImages, FromCssStr};

/// Keep URLs and unsupported interpolation spaces closed for both static and dynamic CSS.
pub(crate) fn gradient_background_source(property: &str, source: &str) -> bool {
    if matches!(
        super::css_keyword(source).as_deref(),
        Some("initial" | "inherit" | "unset")
    ) {
        return true;
    }
    // Takumi 0.23 assigns the same value to omitted interpolation and explicit Oklab.
    // Retain the source distinction: Valle's legacy gradients interpolate in sRGB.
    let mut legacy_defaults = legacy_gradient_defaults(source).into_iter();
    let srgb = takumi_core::style::ColorInterpolationMethod::from_css_str("in srgb")
        .expect("sRGB is a valid CSS interpolation method");
    let mut admitted = |image: &BackgroundImage| {
        let interpolation = match image {
            BackgroundImage::None => return true,
            BackgroundImage::Linear(gradient) => gradient.interpolation,
            BackgroundImage::Radial(gradient) => gradient.interpolation,
            BackgroundImage::Conic(gradient) => gradient.interpolation,
            BackgroundImage::Url(_) => return false,
        };
        let legacy = legacy_defaults.next().unwrap_or(false);
        interpolation == srgb
            || (legacy && interpolation == takumi_core::style::ColorInterpolationMethod::default())
    };
    match property {
        "background-image" => BackgroundImages::from_css_str(source)
            .is_ok_and(|images| !images.is_empty() && images.iter().all(admitted)),
        "background" => Background::from_css_str(source).is_ok_and(|value| admitted(&value.image)),
        _ => false,
    }
}

/// CSS tokens preserve escapes and comments when distinguishing omitted interpolation.
/// Takumi still parses and validates the complete value; this only classifies its gradients.
fn legacy_gradient_defaults(source: &str) -> Vec<bool> {
    use cssparser::{Parser, ParserInput, Token};
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    let mut defaults = Vec::new();
    while let Ok(token) = parser.next().cloned() {
        let Token::Function(name) = token else {
            continue;
        };
        if !matches!(
            name.to_ascii_lowercase().as_str(),
            "linear-gradient"
                | "radial-gradient"
                | "conic-gradient"
                | "repeating-linear-gradient"
                | "repeating-radial-gradient"
                | "repeating-conic-gradient"
        ) {
            continue;
        }
        let legacy = parser.parse_nested_block(|body| {
            let mut explicit = false;
            let mut modern = false;
            while let Ok(token) = body.next().cloned() {
                match token {
                    Token::Ident(ident) if ident.eq_ignore_ascii_case("in") => explicit = true,
                    Token::Function(name) => {
                        modern |= matches!(name.to_ascii_lowercase().as_str(),
                            "lab" | "lch" | "oklab" | "oklch" | "color" | "color-mix");
                        let relative = body.parse_nested_block(|args| {
                            let relative = matches!(args.next(), Ok(Token::Ident(ident)) if ident.eq_ignore_ascii_case("from"));
                            while args.next().is_ok() {}
                            Ok::<_, cssparser::ParseError<'_, ()>>(relative)
                        }).unwrap_or(false);
                        modern |= relative;
                    }
                    _ => {}
                }
            }
            Ok::<_, cssparser::ParseError<'_, ()>>(!explicit && !modern)
        }).unwrap_or(false);
        defaults.push(legacy);
    }
    defaults
}
