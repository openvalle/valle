//! Locked 19-face KaTeX font catalog.

pub const RATEX_CORE_VERSION: &str = "0.1.14";
pub const RATEX_CORE_COMMIT: &str = "08cae05377938391117913ca4f278e6a3ffb6a8a";
pub const KATEX_GOLDEN_VERSION: &str = "0.16.45";
pub const FORMULA_ENGINE_ID: &str =
    "valle-formula-layout@1+ratex-core@0.1.14+08cae05377938391117913ca4f278e6a3ffb6a8a";
pub const FORMULA_FONT_COUNT: usize = 19;

/// One TTF-backed, non-fallback KaTeX face.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormulaFace {
    pub ratex_name: &'static str,
    pub file_name: &'static str,
    pub sha256_hex: &'static str,
}

/// Faces that `ratex_font::FontId` can emit but v1 must reject.
pub const REJECTED_FALLBACK_FONT_NAMES: &[&str] =
    &["CJK-Regular", "CJK-Fallback", "Emoji-Fallback"];

pub const FORMULA_FACES: &[FormulaFace] = &[
    FormulaFace {
        ratex_name: "AMS-Regular",
        file_name: "KaTeX_AMS-Regular.ttf",
        sha256_hex: "68534840bcfdd2bffb6f0e8deb48684dd01e7f04ea2813267577afb906de1d13",
    },
    FormulaFace {
        ratex_name: "Caligraphic-Regular",
        file_name: "KaTeX_Caligraphic-Regular.ttf",
        sha256_hex: "ed0b74372feefcbb9c0666b2e210da37b7e49fa7fbbf3eeb11db5f693dacfbb7",
    },
    FormulaFace {
        ratex_name: "Fraktur-Bold",
        file_name: "KaTeX_Fraktur-Bold.ttf",
        sha256_hex: "9163df9c7122432e6495b4229fa9071cf9ae86a758ae5efc4924ec2e1a6dbce1",
    },
    FormulaFace {
        ratex_name: "Fraktur-Regular",
        file_name: "KaTeX_Fraktur-Regular.ttf",
        sha256_hex: "1e6f9579e90e2cac37f8f60a597c436e075c114385652b7cbeb0dec0421291b3",
    },
    FormulaFace {
        ratex_name: "Main-Bold",
        file_name: "KaTeX_Main-Bold.ttf",
        sha256_hex: "138ac28d1663b3037e9c5f52371fa5c63d8324f4a38d22cd573e6ea3a3fd0cf8",
    },
    FormulaFace {
        ratex_name: "Main-BoldItalic",
        file_name: "KaTeX_Main-BoldItalic.ttf",
        sha256_hex: "70ee1f64a20f2048c21940ef46d0144fd215baa953ca69afd1e31e98544f708f",
    },
    FormulaFace {
        ratex_name: "Main-Italic",
        file_name: "KaTeX_Main-Italic.ttf",
        sha256_hex: "0d85ae7cc30f23790a7f1a58c4a112fdca8aae769b6ba11429af1d98b1b6cb3a",
    },
    FormulaFace {
        ratex_name: "Main-Regular",
        file_name: "KaTeX_Main-Regular.ttf",
        sha256_hex: "d0332f52868370fd83ae7fa46470f90c8f2eab2fcf12bc4f88080b340c95a830",
    },
    FormulaFace {
        ratex_name: "Math-BoldItalic",
        file_name: "KaTeX_Math-BoldItalic.ttf",
        sha256_hex: "f9377ab0271cda59af24bcffbd46a4d0c8a3572ffafdbb38de2ad5ea7b0d5ee5",
    },
    FormulaFace {
        ratex_name: "Math-Italic",
        file_name: "KaTeX_Math-Italic.ttf",
        sha256_hex: "08ce98e51b04d58945a301e639e02b6998af29fdfd61a7b8afdd07bbfc479d4a",
    },
    FormulaFace {
        ratex_name: "SansSerif-Bold",
        file_name: "KaTeX_SansSerif-Bold.ttf",
        sha256_hex: "1ece03f79f95277d57dc7f6b435a74e1379b0d46104a8530286b60ff49369ea0",
    },
    FormulaFace {
        ratex_name: "SansSerif-Italic",
        file_name: "KaTeX_SansSerif-Italic.ttf",
        sha256_hex: "3931dd81faed86ba021bb2bbdc36f5bed9a38d6b4f4077aca59b265aa1b02083",
    },
    FormulaFace {
        ratex_name: "SansSerif-Regular",
        file_name: "KaTeX_SansSerif-Regular.ttf",
        sha256_hex: "f36ea897e19f4a2e571d1e900e4e3710e438deb05a842486045ba0a3e616a4ad",
    },
    FormulaFace {
        ratex_name: "Script-Regular",
        file_name: "KaTeX_Script-Regular.ttf",
        sha256_hex: "1c67f068fea8bb09bf099c088b1cf64bd27516a6e07f4684344873564bb66a67",
    },
    FormulaFace {
        ratex_name: "Size1-Regular",
        file_name: "KaTeX_Size1-Regular.ttf",
        sha256_hex: "95b6d2f1a50173bfedb8c63e1d1c99b10427d0a4df4201cb44513b226951a22b",
    },
    FormulaFace {
        ratex_name: "Size2-Regular",
        file_name: "KaTeX_Size2-Regular.ttf",
        sha256_hex: "a6b2099fb555c60e3a0db3a08842ebf1d732c6eb4e4bf44913613bed4fc4e39b",
    },
    FormulaFace {
        ratex_name: "Size3-Regular",
        file_name: "KaTeX_Size3-Regular.ttf",
        sha256_hex: "500e04d54f0d51666332c9d2089aa803be22aa878eca539e59fa53c6e522b082",
    },
    FormulaFace {
        ratex_name: "Size4-Regular",
        file_name: "KaTeX_Size4-Regular.ttf",
        sha256_hex: "c647367d1dd4e162468717d020e1fc0f1dc5c26ebfdffbe55261713bf88c5877",
    },
    FormulaFace {
        ratex_name: "Typewriter-Regular",
        file_name: "KaTeX_Typewriter-Regular.ttf",
        sha256_hex: "f01f3e87d9c6a61c0c081ceb577abd864eb00a612f7ac1620dd6915fad2ef5aa",
    },
];

/// Locked TTF bytes for each [`FORMULA_FACES`] entry, same order. Product
/// prepare and paint both consume this pack — never a filesystem path.
#[cfg(not(target_arch = "wasm32"))]
static FORMULA_FONT_BYTES: &[&[u8]] = &[
    &crate::font_data::KATEX_KATEX_AMS_REGULAR_TTF,
    &crate::font_data::KATEX_KATEX_CALIGRAPHIC_REGULAR_TTF,
    &crate::font_data::KATEX_KATEX_FRAKTUR_BOLD_TTF,
    &crate::font_data::KATEX_KATEX_FRAKTUR_REGULAR_TTF,
    &crate::font_data::KATEX_KATEX_MAIN_BOLD_TTF,
    &crate::font_data::KATEX_KATEX_MAIN_BOLDITALIC_TTF,
    &crate::font_data::KATEX_KATEX_MAIN_ITALIC_TTF,
    &crate::font_data::KATEX_KATEX_MAIN_REGULAR_TTF,
    &crate::font_data::KATEX_KATEX_MATH_BOLDITALIC_TTF,
    &crate::font_data::KATEX_KATEX_MATH_ITALIC_TTF,
    &crate::font_data::KATEX_KATEX_SANSSERIF_BOLD_TTF,
    &crate::font_data::KATEX_KATEX_SANSSERIF_ITALIC_TTF,
    &crate::font_data::KATEX_KATEX_SANSSERIF_REGULAR_TTF,
    &crate::font_data::KATEX_KATEX_SCRIPT_REGULAR_TTF,
    &crate::font_data::KATEX_KATEX_SIZE1_REGULAR_TTF,
    &crate::font_data::KATEX_KATEX_SIZE2_REGULAR_TTF,
    &crate::font_data::KATEX_KATEX_SIZE3_REGULAR_TTF,
    &crate::font_data::KATEX_KATEX_SIZE4_REGULAR_TTF,
    &crate::font_data::KATEX_KATEX_TYPEWRITER_REGULAR_TTF,
];

#[cfg(not(target_arch = "wasm32"))]
const _: () = assert!(FORMULA_FACES.len() == FORMULA_FONT_BYTES.len());
const _: () = assert!(FORMULA_FACES.len() == FORMULA_FONT_COUNT);

pub fn formula_faces() -> &'static [FormulaFace] {
    FORMULA_FACES
}

/// Locked (face, TTF bytes) pairs. Hosts register the same bytes they prepare with.
#[cfg(not(target_arch = "wasm32"))]
pub fn formula_font_pack() -> impl Iterator<Item = (FormulaFace, &'static [u8])> {
    FORMULA_FACES
        .iter()
        .copied()
        .zip(FORMULA_FONT_BYTES.iter().copied())
}

pub fn allowed_ratex_font_name(name: &str) -> bool {
    FORMULA_FACES.iter().any(|face| face.ratex_name == name)
}

pub fn formula_font_dir() -> &'static str {
    "assets/fonts/katex"
}

/// SHA-256 of face bytes as lowercase hex. Used to lock TTF identity on disk.
fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push(core::char::from_digit((byte >> 4) as u32, 16).unwrap());
        out.push(core::char::from_digit((byte & 0x0f) as u32, 16).unwrap());
    }
    out
}

pub fn face_matches_lock(face: &FormulaFace, bytes: &[u8]) -> bool {
    sha256_hex(bytes) == face.sha256_hex
}
