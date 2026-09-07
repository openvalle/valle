#![cfg(feature = "motion")]
//! Formula teaching and engineering examples compile through the public author surface.
use valle_compiler::motion::compile_motion;
use valle_motion::MATH_FORMULA_CAPABILITY;

#[test]
fn lecture_and_engineering_films_compile_on_check_path() {
    for path in [
        "crates/valle-compiler/tests/fixtures/motion/formulas/formula-lecture.motion.tsx",
        "crates/valle-compiler/tests/fixtures/motion/formulas/formula-engineering.motion.tsx",
        "crates/valle-compiler/tests/fixtures/motion/formulas/formula-wall.motion.tsx",
        "crates/valle-compiler/tests/fixtures/math_formula.motion.tsx",
    ] {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let source = std::fs::read_to_string(root.join(path)).expect(path);
        let compiled = compile_motion(&source).unwrap_or_else(|err| panic!("{path}: {err:?}"));
        assert!(
            compiled
                .artifact
                .capability_set
                .names
                .iter()
                .any(|name| name == MATH_FORMULA_CAPABILITY),
            "{path}"
        );
        if path.ends_with("formula-lecture.motion.tsx") {
            assert!(source.contains(r"\begin{pmatrix}"), "{path} matrix");
            assert!(source.contains(r"\begin{cases}"), "{path} cases");
            assert!(source.contains(r"\tag{"), "{path} tag");
            assert!(source.contains(r"\color{"), "{path} color");
            assert!(
                !source.contains("传输") && !source.contains("中文"),
                "{path} CJK must stay outside latex"
            );
        }
    }
}
