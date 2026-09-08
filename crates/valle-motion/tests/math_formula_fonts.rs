//! Locked formula font files and the embedded default font pack.
use std::{fs, path::PathBuf};
use valle_motion::math_formula::{
    FORMULA_FONT_COUNT, FormulaFace, RATEX_CORE_COMMIT, RATEX_CORE_VERSION, face_matches_lock,
    formula_faces, formula_font_pack,
};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn face_path(face: &FormulaFace) -> PathBuf {
    repo_root()
        .join("assets/fonts/katex")
        .join(face.file_name)
}

#[test]
fn locked_nineteen_faces_match_sha256_and_exclude_caligraphic_bold() {
    assert_eq!(formula_faces().len(), FORMULA_FONT_COUNT);
    assert_eq!(RATEX_CORE_VERSION, "0.1.14");
    assert_eq!(
        RATEX_CORE_COMMIT,
        "08cae05377938391117913ca4f278e6a3ffb6a8a"
    );
    let mut names = Vec::new();
    for face in formula_faces() {
        names.push(face.file_name);
        let bytes = fs::read(face_path(face)).unwrap_or_else(|err| {
            panic!("missing formula face {}: {err}", face.file_name);
        });
        assert!(
            face_matches_lock(face, &bytes),
            "{} hash mismatch",
            face.file_name
        );
    }
    assert!(!names.iter().any(|name| name.contains("Caligraphic-Bold")));
    assert!(
        !repo_root()
            .join("assets/fonts/katex/KaTeX_Caligraphic-Bold.ttf")
            .exists()
    );
}

#[test]
fn load_default_uses_embedded_pack_not_filesystem() {
    unsafe { std::env::set_var("VALLE_FORMULA_FONT_DIR", "/no/such/formula/fonts") };
    let registry = valle_motion::math_formula::FormulaFontRegistry::load_default()
        .expect("embedded pack must load without a font directory");
    assert_eq!(registry.len(), FORMULA_FONT_COUNT);
    for (face, bytes) in formula_font_pack() {
        assert!(
            face_matches_lock(&face, bytes),
            "{} embedded bytes must match the lock hash",
            face.file_name
        );
        registry
            .get(face.ratex_name)
            .unwrap_or_else(|err| panic!("{}: {err}", face.ratex_name));
    }
}
