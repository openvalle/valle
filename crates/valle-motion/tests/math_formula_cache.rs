//! A formula cache hit must not repeat parsing or layout.

use std::path::PathBuf;
use std::sync::atomic::Ordering;

use valle_motion::math_formula::{
    AdmitPolicy, FORMULA_PREPARE_RUNS, FormulaFontRegistry, FormulaStyle, emit_formula,
};

#[test]
fn second_emit_is_a_cache_hit() {
    let registry = FormulaFontRegistry::load_dir(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts/katex"),
    )
    .unwrap();
    let policy = AdmitPolicy::default();
    let style = FormulaStyle::default();
    let latex = r"\frac{a+b}{c}";
    let before = FORMULA_PREPARE_RUNS.load(Ordering::Relaxed);
    let a = emit_formula(latex, true, style, &registry, &policy).unwrap();
    let mid = FORMULA_PREPARE_RUNS.load(Ordering::Relaxed);
    let b = emit_formula(latex, true, style, &registry, &policy).unwrap();
    let after = FORMULA_PREPARE_RUNS.load(Ordering::Relaxed);
    assert_eq!(
        a.list.canonical_bytes().unwrap(),
        b.list.canonical_bytes().unwrap()
    );
    assert!(mid > before, "first emit must prepare");
    assert_eq!(after, mid, "second emit must not re-run parse/layout");
}
