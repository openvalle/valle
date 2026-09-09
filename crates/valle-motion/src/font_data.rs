//! One shared storage location per bundled font, used by text and formulas.

pub(crate) static KATEX_KATEX_AMS_REGULAR_TTF: [u8; include_bytes!(
    "../../../assets/fonts/katex/KaTeX_AMS-Regular.ttf"
)
.len()] = *include_bytes!("../../../assets/fonts/katex/KaTeX_AMS-Regular.ttf");
pub(crate) static KATEX_KATEX_CALIGRAPHIC_REGULAR_TTF: [u8; include_bytes!(
    "../../../assets/fonts/katex/KaTeX_Caligraphic-Regular.ttf"
)
.len()] = *include_bytes!("../../../assets/fonts/katex/KaTeX_Caligraphic-Regular.ttf");
pub(crate) static KATEX_KATEX_FRAKTUR_BOLD_TTF: [u8; include_bytes!(
    "../../../assets/fonts/katex/KaTeX_Fraktur-Bold.ttf"
)
.len()] = *include_bytes!("../../../assets/fonts/katex/KaTeX_Fraktur-Bold.ttf");
pub(crate) static KATEX_KATEX_FRAKTUR_REGULAR_TTF: [u8; include_bytes!(
    "../../../assets/fonts/katex/KaTeX_Fraktur-Regular.ttf"
)
.len()] = *include_bytes!("../../../assets/fonts/katex/KaTeX_Fraktur-Regular.ttf");
pub(crate) static KATEX_KATEX_MAIN_BOLD_TTF: [u8; include_bytes!(
    "../../../assets/fonts/katex/KaTeX_Main-Bold.ttf"
)
.len()] = *include_bytes!("../../../assets/fonts/katex/KaTeX_Main-Bold.ttf");
pub(crate) static KATEX_KATEX_MAIN_BOLDITALIC_TTF: [u8; include_bytes!(
    "../../../assets/fonts/katex/KaTeX_Main-BoldItalic.ttf"
)
.len()] = *include_bytes!("../../../assets/fonts/katex/KaTeX_Main-BoldItalic.ttf");
pub(crate) static KATEX_KATEX_MAIN_ITALIC_TTF: [u8; include_bytes!(
    "../../../assets/fonts/katex/KaTeX_Main-Italic.ttf"
)
.len()] = *include_bytes!("../../../assets/fonts/katex/KaTeX_Main-Italic.ttf");
pub(crate) static KATEX_KATEX_MAIN_REGULAR_TTF: [u8; include_bytes!(
    "../../../assets/fonts/katex/KaTeX_Main-Regular.ttf"
)
.len()] = *include_bytes!("../../../assets/fonts/katex/KaTeX_Main-Regular.ttf");
pub(crate) static KATEX_KATEX_MATH_BOLDITALIC_TTF: [u8; include_bytes!(
    "../../../assets/fonts/katex/KaTeX_Math-BoldItalic.ttf"
)
.len()] = *include_bytes!("../../../assets/fonts/katex/KaTeX_Math-BoldItalic.ttf");
pub(crate) static KATEX_KATEX_MATH_ITALIC_TTF: [u8; include_bytes!(
    "../../../assets/fonts/katex/KaTeX_Math-Italic.ttf"
)
.len()] = *include_bytes!("../../../assets/fonts/katex/KaTeX_Math-Italic.ttf");
pub(crate) static KATEX_KATEX_SANSSERIF_BOLD_TTF: [u8; include_bytes!(
    "../../../assets/fonts/katex/KaTeX_SansSerif-Bold.ttf"
)
.len()] = *include_bytes!("../../../assets/fonts/katex/KaTeX_SansSerif-Bold.ttf");
pub(crate) static KATEX_KATEX_SANSSERIF_ITALIC_TTF: [u8; include_bytes!(
    "../../../assets/fonts/katex/KaTeX_SansSerif-Italic.ttf"
)
.len()] = *include_bytes!("../../../assets/fonts/katex/KaTeX_SansSerif-Italic.ttf");
pub(crate) static KATEX_KATEX_SANSSERIF_REGULAR_TTF: [u8; include_bytes!(
    "../../../assets/fonts/katex/KaTeX_SansSerif-Regular.ttf"
)
.len()] = *include_bytes!("../../../assets/fonts/katex/KaTeX_SansSerif-Regular.ttf");
pub(crate) static KATEX_KATEX_SCRIPT_REGULAR_TTF: [u8; include_bytes!(
    "../../../assets/fonts/katex/KaTeX_Script-Regular.ttf"
)
.len()] = *include_bytes!("../../../assets/fonts/katex/KaTeX_Script-Regular.ttf");
pub(crate) static KATEX_KATEX_SIZE1_REGULAR_TTF: [u8; include_bytes!(
    "../../../assets/fonts/katex/KaTeX_Size1-Regular.ttf"
)
.len()] = *include_bytes!("../../../assets/fonts/katex/KaTeX_Size1-Regular.ttf");
pub(crate) static KATEX_KATEX_SIZE2_REGULAR_TTF: [u8; include_bytes!(
    "../../../assets/fonts/katex/KaTeX_Size2-Regular.ttf"
)
.len()] = *include_bytes!("../../../assets/fonts/katex/KaTeX_Size2-Regular.ttf");
pub(crate) static KATEX_KATEX_SIZE3_REGULAR_TTF: [u8; include_bytes!(
    "../../../assets/fonts/katex/KaTeX_Size3-Regular.ttf"
)
.len()] = *include_bytes!("../../../assets/fonts/katex/KaTeX_Size3-Regular.ttf");
pub(crate) static KATEX_KATEX_SIZE4_REGULAR_TTF: [u8; include_bytes!(
    "../../../assets/fonts/katex/KaTeX_Size4-Regular.ttf"
)
.len()] = *include_bytes!("../../../assets/fonts/katex/KaTeX_Size4-Regular.ttf");
pub(crate) static KATEX_KATEX_TYPEWRITER_REGULAR_TTF: [u8; include_bytes!(
    "../../../assets/fonts/katex/KaTeX_Typewriter-Regular.ttf"
)
.len()] = *include_bytes!("../../../assets/fonts/katex/KaTeX_Typewriter-Regular.ttf");
pub(crate) static NOTO_NOTO_COLRV1_TTF: [u8; include_bytes!(
    "../../../assets/fonts/noto/Noto-COLRv1.ttf"
)
.len()] = *include_bytes!("../../../assets/fonts/noto/Noto-COLRv1.ttf");
pub(crate) static NOTO_NOTOSANS_BOLD_TTF: [u8; include_bytes!(
    "../../../assets/fonts/noto/NotoSans-Bold.ttf"
)
.len()] = *include_bytes!("../../../assets/fonts/noto/NotoSans-Bold.ttf");
pub(crate) static NOTO_NOTOSANS_EXTRABOLD_TTF: [u8; include_bytes!(
    "../../../assets/fonts/noto/NotoSans-ExtraBold.ttf"
)
.len()] = *include_bytes!("../../../assets/fonts/noto/NotoSans-ExtraBold.ttf");
pub(crate) static NOTO_NOTOSANS_MEDIUM_TTF: [u8; include_bytes!(
    "../../../assets/fonts/noto/NotoSans-Medium.ttf"
)
.len()] = *include_bytes!("../../../assets/fonts/noto/NotoSans-Medium.ttf");
pub(crate) static NOTO_NOTOSANS_REGULAR_TTF: [u8; include_bytes!(
    "../../../assets/fonts/noto/NotoSans-Regular.ttf"
)
.len()] = *include_bytes!("../../../assets/fonts/noto/NotoSans-Regular.ttf");
pub(crate) static NOTO_NOTOSANS_SEMIBOLD_TTF: [u8; include_bytes!(
    "../../../assets/fonts/noto/NotoSans-SemiBold.ttf"
)
.len()] = *include_bytes!("../../../assets/fonts/noto/NotoSans-SemiBold.ttf");
pub(crate) static NOTO_NOTOSANSCJKSC_REGULAR_OTF: [u8; include_bytes!(
    "../../../assets/fonts/noto/NotoSansCJKsc-Regular.otf"
)
.len()] = *include_bytes!("../../../assets/fonts/noto/NotoSansCJKsc-Regular.otf");
pub(crate) static NOTO_NOTOSANSMATH_REGULAR_TTF: [u8; include_bytes!(
    "../../../assets/fonts/noto/NotoSansMath-Regular.ttf"
)
.len()] = *include_bytes!("../../../assets/fonts/noto/NotoSansMath-Regular.ttf");
pub(crate) static NOTO_NOTOSANSMONO_REGULAR_TTF: [u8; include_bytes!(
    "../../../assets/fonts/noto/NotoSansMono-Regular.ttf"
)
.len()] = *include_bytes!("../../../assets/fonts/noto/NotoSansMono-Regular.ttf");
pub(crate) static NOTO_NOTOSANSSYMBOLS_REGULAR_TTF: [u8; include_bytes!(
    "../../../assets/fonts/noto/NotoSansSymbols-Regular.ttf"
)
.len()] = *include_bytes!("../../../assets/fonts/noto/NotoSansSymbols-Regular.ttf");
pub(crate) static NOTO_NOTOSANSSYMBOLS2_REGULAR_TTF: [u8; include_bytes!(
    "../../../assets/fonts/noto/NotoSansSymbols2-Regular.ttf"
)
.len()] = *include_bytes!("../../../assets/fonts/noto/NotoSansSymbols2-Regular.ttf");
