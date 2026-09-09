//! Motion text: font registration, deterministic fallback, and intrinsic measurement.

mod measure;

/// Shared Noto Sans Regular fallback for deterministic measurement and rendering across all Motion
/// hosts.
#[cfg(not(target_arch = "wasm32"))]
pub static DEFAULT_MOTION_FONT: &[u8] = &crate::font_data::NOTO_NOTOSANS_REGULAR_TTF;

/// Default font family set: Noto Sans weights, KaTeX serif faces, Noto Sans Mono,
/// symbol and math fonts, Noto Sans CJK SC, and Noto Color Emoji. Register real weight variants for CSS
/// font selection; Noto supplies deterministic COLRv1 emoji paints. `DEFAULT_MOTION_FONT`
/// remains the single-face fallback for measurement contracts.
#[cfg(not(target_arch = "wasm32"))]
pub static DEFAULT_MOTION_FONT_WEIGHTS: &[&[u8]] = &[
    &crate::font_data::NOTO_NOTOSANS_REGULAR_TTF,
    &crate::font_data::NOTO_NOTOSANS_MEDIUM_TTF,
    &crate::font_data::NOTO_NOTOSANS_SEMIBOLD_TTF,
    &crate::font_data::NOTO_NOTOSANS_BOLD_TTF,
    &crate::font_data::NOTO_NOTOSANS_EXTRABOLD_TTF,
    &crate::font_data::KATEX_KATEX_MAIN_REGULAR_TTF,
    &crate::font_data::KATEX_KATEX_MAIN_BOLD_TTF,
    &crate::font_data::KATEX_KATEX_MAIN_ITALIC_TTF,
    &crate::font_data::KATEX_KATEX_MAIN_BOLDITALIC_TTF,
    &crate::font_data::NOTO_NOTOSANSMONO_REGULAR_TTF,
    &crate::font_data::NOTO_NOTOSANSSYMBOLS_REGULAR_TTF,
    &crate::font_data::NOTO_NOTOSANSSYMBOLS2_REGULAR_TTF,
    &crate::font_data::NOTO_NOTOSANSMATH_REGULAR_TTF,
    &crate::font_data::NOTO_NOTOSANSCJKSC_REGULAR_OTF,
    &crate::font_data::NOTO_NOTO_COLRV1_TTF,
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
    "Noto-COLRv1.ttf",
];

/// Register the default Motion pack with CSS generic family aliases.
#[cfg(not(target_arch = "wasm32"))]
pub fn register_default_motion_fonts(
    fonts: &mut Fonts,
) -> Result<(), takumi_core::resources::font::FontError> {
    debug_assert_eq!(
        default_motion_fonts().len(),
        DEFAULT_MOTION_FONT_FILES.len()
    );
    for (index, bytes) in default_motion_fonts().iter().enumerate() {
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
        14 => resource.generic_family(GenericFamily::EMOJI),
        5..=8 => resource.generic_family(GenericFamily::SERIF),
        _ => resource,
    }
}

/// Host registration for unnamed font blobs. Default-pack bytes keep their CSS
/// generic; everything else is an ordinary face.
#[cfg(not(target_arch = "wasm32"))]
pub fn motion_font_resource(bytes: Vec<u8>) -> FontResource<'static> {
    match default_motion_fonts()
        .iter()
        .position(|pack| *pack == bytes.as_slice())
    {
        Some(index) => default_motion_font_resource(index, bytes),
        None => FontResource::new(bytes),
    }
}

pub use measure::{MeasureError, MeasuredBox, TextMeasure, measure_text};

pub use takumi_core::resources::font::{FontOverride, FontResource, Fonts, GenericFamily};

/// Shared default bytes are only provided by Native hosts.
#[cfg(not(target_arch = "wasm32"))]
pub fn default_motion_fonts() -> &'static [&'static [u8]] {
    DEFAULT_MOTION_FONT_WEIGHTS
}
