//! Motion text: font registration, deterministic fallback, and intrinsic measurement.

mod measure;

/// Shared variable Noto Sans fallback for deterministic measurement and rendering across all Motion
/// hosts.
#[cfg(not(target_arch = "wasm32"))]
pub static DEFAULT_MOTION_FONT: &[u8] = &crate::font_data::NOTO_NOTOSANS_VARIABLE_TTF;

/// Default font family set: variable Noto Sans, KaTeX serif faces, Noto Sans Mono,
/// symbol and math fonts, Noto Sans CJK SC, and Noto Color Emoji. The variable sans face
/// maps CSS weight to its continuous `wght` axis. Measurement and rendering share this pack.
#[cfg(not(target_arch = "wasm32"))]
pub static DEFAULT_MOTION_FONT_WEIGHTS: &[&[u8]] = &[
    &crate::font_data::NOTO_NOTOSANS_VARIABLE_TTF,
    &crate::font_data::KATEX_KATEX_MAIN_REGULAR_TTF,
    &crate::font_data::KATEX_KATEX_MAIN_BOLD_TTF,
    &crate::font_data::KATEX_KATEX_MAIN_ITALIC_TTF,
    &crate::font_data::KATEX_KATEX_MAIN_BOLDITALIC_TTF,
    &crate::font_data::NOTO_NOTOSANSMONO_REGULAR_TTF,
    &crate::font_data::NOTO_NOTOSANSSYMBOLS_REGULAR_TTF,
    &crate::font_data::NOTO_NOTOSANSSYMBOLS2_REGULAR_TTF,
    &crate::font_data::NOTO_NOTOSANSMATH_REGULAR_TTF,
    &crate::font_data::NOTO_NOTOSANSCJKSC_VARIABLE_OTF,
    &crate::font_data::NOTO_NOTO_COLRV1_TTF,
];

/// Stable file names for the default pack, same order as [`DEFAULT_MOTION_FONT_WEIGHTS`].
/// Hosts that serve `/runtime/fonts/<name>` must use these names.
pub const DEFAULT_MOTION_FONT_FILES: &[&str] = &[
    "NotoSans-Variable.ttf",
    "KaTeX_Main-Regular.ttf",
    "KaTeX_Main-Bold.ttf",
    "KaTeX_Main-Italic.ttf",
    "KaTeX_Main-BoldItalic.ttf",
    "NotoSansMono-Regular.ttf",
    "NotoSansSymbols-Regular.ttf",
    "NotoSansSymbols2-Regular.ttf",
    "NotoSansMath-Regular.ttf",
    "NotoSansCJKsc-Variable.otf",
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
    for bytes in default_motion_fonts() {
        fonts.register(motion_font_resource(FontSource::from_static(bytes)))?;
    }
    Ok(())
}

/// Register any font using its own name and metadata. Generic CSS families are
/// derived from the face, never from a bundled file's digest or array position.
pub fn motion_font_resource(source: impl Into<FontSource<'static>>) -> FontResource<'static> {
    let source = source.into();
    let generic = ttf_parser::Face::parse(source.as_ref(), 0)
        .ok()
        .and_then(|face| {
            let family = face
                .names()
                .into_iter()
                .filter(|name| {
                    matches!(
                        name.name_id,
                        ttf_parser::name_id::FAMILY | ttf_parser::name_id::TYPOGRAPHIC_FAMILY
                    )
                })
                .filter_map(|name| name.to_string())
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase();
            let os2 = face.raw_face().table(ttf_parser::Tag::from_bytes(b"OS/2"));
            let panose = os2.and_then(|table| table.get(32..42));
            if face.tables().colr.is_some()
                || face.tables().cbdt.is_some()
                || face.tables().sbix.is_some()
            {
                Some(GenericFamily::EMOJI)
            } else if family.contains("symbol") || family.contains("math") || family.contains("cjk")
            {
                // These faces may also contain Latin glyphs. Keep them in script fallback,
                // not in the explicit sans-serif family list ahead of color emoji.
                None
            } else if face.is_monospaced()
                || family.contains("mono")
                || family.contains("typewriter")
                || panose.is_some_and(|p| p[0] == 2 && p[3] == 9)
            {
                Some(GenericFamily::MONOSPACE)
            } else {
                let serif = os2
                    .and_then(|table| table.get(30))
                    .is_some_and(|class| (1..=7).contains(class))
                    || panose.is_some_and(|p| p[0] == 2 && (2..=10).contains(&p[1]))
                    || (family.contains("serif") && !family.contains("sans"))
                    || family.contains("katex_main");
                Some(if serif {
                    GenericFamily::SERIF
                } else {
                    GenericFamily::SANS_SERIF
                })
            }
        });
    let resource = FontResource::new(source);
    match generic {
        Some(generic) => resource.generic_family(generic),
        None => resource,
    }
}

pub use measure::{
    MeasureError, MeasuredBox, OutlineBounds, OutlinedGlyph, OutlinedText, TextMeasure,
    TextOutlineAlign, measure_text, text_outline,
};

pub use takumi_core::resources::font::{
    FontOverride, FontResource, FontSource, Fonts, GenericFamily,
};

/// Shared default bytes are only provided by Native hosts.
#[cfg(not(target_arch = "wasm32"))]
pub fn default_motion_fonts() -> &'static [&'static [u8]] {
    DEFAULT_MOTION_FONT_WEIGHTS
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn default_sans_is_the_pinned_variable_face_with_continuous_axes() {
        assert_eq!(
            DEFAULT_MOTION_FONT_FILES.len(),
            DEFAULT_MOTION_FONT_WEIGHTS.len()
        );
        assert_eq!(DEFAULT_MOTION_FONT_FILES[0], "NotoSans-Variable.ttf");
        assert_eq!(
            crate::ContentDigest::of_bytes(DEFAULT_MOTION_FONT).as_hex(),
            "bfb7bb691513f12e734dc346c03a03f784912432d7e3fa8e56efcf906fe86b3d"
        );
        let face = ttf_parser::Face::parse(DEFAULT_MOTION_FONT, 0).unwrap();
        for (tag, min, default, max) in [
            (*b"wght", 100.0, 400.0, 900.0),
            (*b"wdth", 62.5, 100.0, 100.0),
        ] {
            let axis = face
                .variation_axes()
                .into_iter()
                .find(|axis| axis.tag.to_bytes() == tag)
                .unwrap();
            assert_eq!(
                (axis.min_value, axis.def_value, axis.max_value),
                (min, default, max)
            );
        }
        // Static sans siblings must not compete with the variable face during family matching.
        assert_eq!(
            DEFAULT_MOTION_FONT_FILES
                .iter()
                .filter(|name| name.starts_with("NotoSans-"))
                .count(),
            1
        );
    }
}
