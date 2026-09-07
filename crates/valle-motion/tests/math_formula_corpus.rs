//! Every locked KaTeX corpus entry is classified, with finite accepted layout dimensions.
use std::collections::BTreeMap;
use valle_motion::math_formula::{AdmitPolicy, Classification, classify_formula};
const CORPUS: &str = include_str!("fixtures/katex-golden-0.16.45.txt");

#[test]
fn katex_main_corpus_is_fully_classified() {
    let policy = AdmitPolicy::default();
    let lines: Vec<&str> = CORPUS.lines().filter(|line| !line.is_empty()).collect();
    assert_eq!(lines.len(), 1075, "locked KaTeX 0.16.45 golden count");

    let mut counts = BTreeMap::<&'static str, usize>::new();
    for source in &lines {
        let row = classify_formula(source, true, &policy);
        let key = match row.class {
            Classification::Accepted => "accepted",
            Classification::ParseError => "parse_error",
            Classification::PolicyIncludeGraphics => "includegraphics",
            Classification::PolicyAutoNumber => "auto_number",
            Classification::PolicyLeqno => "leqno",
            Classification::PolicyCjkEmoji => "cjk_emoji",
            Classification::PolicyExtension => "extension",
            Classification::LeftoverEmpty => "leftover_empty",
            Classification::NonFinite => "non_finite",
            Classification::UnknownFont => "unknown_font",
            Classification::Budget => "budget",
        };
        *counts.entry(key).or_insert(0) += 1;
        if row.class == Classification::Accepted {
            assert!(row.width.is_finite());
            assert!(row.height.is_finite());
        }
    }
    let classified: usize = counts.values().copied().sum();
    eprintln!("katex 0.16.45 corpus classification: {counts:?}");
    assert_eq!(classified, 1075);
    assert_eq!(
        counts.get("accepted").copied().unwrap_or(0),
        1063,
        "the locked corpus admission ledger must remain stable"
    );
    // Starred golden corpus should not need auto-number as the dominant class.
    assert!(
        counts.get("auto_number").copied().unwrap_or(0) < 50,
        "auto-number exclusions too high for starred corpus: {counts:?}"
    );
}
