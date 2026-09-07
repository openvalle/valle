//! RaTeX-backed MathFormula admission, classification, and layout adapter. RaTeX AST and
//! ProgramRecording remain internal representations.

pub mod adapter;
pub mod admit;
pub mod classify;
pub mod fonts;
pub mod registry;

pub use adapter::{
    FORMULA_PREPARE_RUNS, FormulaFragment, FormulaStyle, UNFILLED_PATH_STROKE_PX, emit_formula,
    ratex_color_to_rgba,
};
pub use admit::{AdmitError, AdmitPolicy, admit_nodes};
pub use classify::{
    Classification, ClassifiedFormula, classify_formula, prepare_ratex_list,
    prepare_ratex_list_with_color,
};
pub use fonts::{
    FORMULA_ENGINE_ID, FORMULA_FONT_COUNT, FormulaFace, KATEX_GOLDEN_VERSION, RATEX_CORE_COMMIT,
    RATEX_CORE_VERSION, allowed_ratex_font_name, face_matches_lock, formula_faces,
    formula_font_pack,
};
pub use registry::FormulaFontRegistry;

/// Build-fingerprint identity for the formula layout engine. Independent of
/// [`crate::LAYOUT_ENGINE_ID`] (Takumi) and `valle_draw::math`.
pub const FORMULA_LAYOUT_ENGINE: &str = FORMULA_ENGINE_ID;
