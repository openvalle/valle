//! Motion text: font registration, deterministic fallback, and intrinsic measurement.

mod measure;

/// Shared Noto Sans Regular fallback for deterministic measurement and rendering across all Motion
/// hosts.
pub const DEFAULT_MOTION_FONT: &[u8] =
    include_bytes!("../../assets/fonts/noto/NotoSans-Regular.ttf");

/// Default font family set: Noto Sans weights, KaTeX serif faces, Noto Sans Mono,
/// symbol and math fonts, Noto Sans CJK SC, and Twemoji Mozilla. Register real weight variants for CSS
/// font selection; Twemoji supplies deterministic COLR/CPAL emoji layers. `DEFAULT_MOTION_FONT`
/// remains the single-face fallback for measurement contracts.
pub const DEFAULT_MOTION_FONT_WEIGHTS: &[&[u8]] = &[
    include_bytes!("../../assets/fonts/noto/NotoSans-Regular.ttf"),
    include_bytes!("../../assets/fonts/noto/NotoSans-Medium.ttf"),
    include_bytes!("../../assets/fonts/noto/NotoSans-SemiBold.ttf"),
    include_bytes!("../../assets/fonts/noto/NotoSans-Bold.ttf"),
    include_bytes!("../../assets/fonts/noto/NotoSans-ExtraBold.ttf"),
    include_bytes!("../../assets/fonts/katex/KaTeX_Main-Regular.ttf"),
    include_bytes!("../../assets/fonts/katex/KaTeX_Main-Bold.ttf"),
    include_bytes!("../../assets/fonts/katex/KaTeX_Main-Italic.ttf"),
    include_bytes!("../../assets/fonts/katex/KaTeX_Main-BoldItalic.ttf"),
    include_bytes!("../../assets/fonts/noto/NotoSansMono-Regular.ttf"),
    include_bytes!("../../assets/fonts/noto/NotoSansSymbols-Regular.ttf"),
    include_bytes!("../../assets/fonts/noto/NotoSansSymbols2-Regular.ttf"),
    include_bytes!("../../assets/fonts/noto/NotoSansMath-Regular.ttf"),
    include_bytes!("../../assets/fonts/noto/NotoSansCJKsc-Regular.otf"),
    include_bytes!("../../assets/fonts/twemoji/TwemojiMozilla.ttf"),
];

/// Stable file names for the default pack, same order as [`DEFAULT_MOTION_FONT_WEIGHTS`].
/// Hosts that serve `/runtime/fonts/<name>` must use these names.
pub const DEFAULT_MOTION_FONT_FILES: &[&str] = &[
    "NotoSans-Regular.ttf",
    "NotoSans-Medium.ttf",
    "NotoSans-SemiBold.ttf",
    "NotoSans-Bold.ttf",
    "NotoSans-ExtraBold.ttf",
    "KaTeX_Main-Regular.ttf",
    "KaTeX_Main-Bold.ttf",
    "KaTeX_Main-Italic.ttf",
    "KaTeX_Main-BoldItalic.ttf",
    "NotoSansMono-Regular.ttf",
    "NotoSansSymbols-Regular.ttf",
    "NotoSansSymbols2-Regular.ttf",
    "NotoSansMath-Regular.ttf",
    "NotoSansCJKsc-Regular.otf",
    "TwemojiMozilla.ttf",
];

/// Register the default Motion pack with CSS generic family aliases.
pub fn register_default_motion_fonts(
    fonts: &mut Fonts,
) -> Result<(), takumi_core::resources::font::FontError> {
    debug_assert_eq!(
        DEFAULT_MOTION_FONT_WEIGHTS.len(),
        DEFAULT_MOTION_FONT_FILES.len()
    );
    for (index, bytes) in DEFAULT_MOTION_FONT_WEIGHTS.iter().enumerate() {
        fonts.register(default_motion_font_resource(index, bytes.to_vec()))?;
    }
    Ok(())
}

/// Map a default-pack slot onto the CSS generic it fulfills.
pub fn default_motion_font_resource(index: usize, bytes: Vec<u8>) -> FontResource<'static> {
    use takumi_core::resources::font::GenericFamily;
    let resource = FontResource::new(bytes);
    match index {
        0..=4 => resource.generic_family(GenericFamily::SANS_SERIF),
        9 => resource.generic_family(GenericFamily::MONOSPACE),
        5..=8 => resource.generic_family(GenericFamily::SERIF),
        _ => resource,
    }
}

/// Host registration for unnamed font blobs. Default-pack bytes keep their CSS
/// generic; everything else is an ordinary face.
pub fn motion_font_resource(bytes: Vec<u8>) -> FontResource<'static> {
    match DEFAULT_MOTION_FONT_WEIGHTS
        .iter()
        .position(|pack| *pack == bytes.as_slice())
    {
        Some(index) => default_motion_font_resource(index, bytes),
        None => FontResource::new(bytes),
    }
}

pub use measure::{MeasureError, MeasuredBox, TextMeasure, measure_text};

pub use takumi_core::resources::font::{FontOverride, FontResource, Fonts, GenericFamily};
