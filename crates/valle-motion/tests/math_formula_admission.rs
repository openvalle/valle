//! Formula input budgets, forbidden syntax and diagnostic contracts.

use valle_motion::math_formula::{
    AdmitError, AdmitPolicy, Classification, classify_formula, prepare_ratex_list,
};

#[test]
fn budget_and_malicious_inputs_fail_in_prepare() {
    let tiny = AdmitPolicy {
        max_source_bytes: 8,
        ..AdmitPolicy::default()
    };
    let row = classify_formula(r"\frac{1}{2}+\frac{3}{4}", true, &tiny);
    assert_eq!(row.class, Classification::Budget);

    let leftover = classify_formula(r"x\nonumber", true, &AdmitPolicy::default());
    assert_eq!(leftover.class, Classification::LeftoverEmpty);

    let img = classify_formula(r"\includegraphics{x}", true, &AdmitPolicy::default());
    assert_eq!(img.class, Classification::PolicyIncludeGraphics);
}

#[test]
fn engine_identity_is_stable_for_upgrade_gate() {
    assert!(valle_motion::math_formula::FORMULA_LAYOUT_ENGINE.contains("ratex-core@0.1.14"));
    let err = AdmitError::UnknownFont {
        name: "not-a-face".into(),
    };
    assert!(err.to_string().contains("not-a-face"));
}

#[test]
fn includegraphics_and_auto_number_and_leqno_fail_closed() {
    let policy = AdmitPolicy::default();
    let img = classify_formula(r"\includegraphics{x.png}", true, &policy);
    assert_eq!(img.class, Classification::PolicyIncludeGraphics);

    let align = classify_formula(r"\begin{align} a &= b \end{align}", true, &policy);
    assert_eq!(align.class, Classification::PolicyAutoNumber);

    let leqno = classify_formula(r"\leqno x", true, &policy);
    assert_eq!(leqno.class, Classification::PolicyLeqno);

    let cjk = classify_formula(r"\text{中}", true, &policy);
    assert_eq!(cjk.class, Classification::PolicyCjkEmoji);
}

#[test]
fn leftover_nonumber_outside_environment_is_not_success() {
    let policy = AdmitPolicy::default();
    let row = classify_formula(r"x\nonumber", true, &policy);
    assert_eq!(row.class, Classification::LeftoverEmpty, "{}", row.detail);
}

#[test]
fn lecture_matrix_cases_tag_color_are_accepted() {
    let policy = AdmitPolicy::default();
    for source in [
        r"\begin{pmatrix} a & b \\ c & d \end{pmatrix}\begin{pmatrix} x \\ y \end{pmatrix}",
        r"f(x)=\begin{cases} x^{2} & x\ge 0 \\ -x & x<0 \end{cases}",
        r"\begin{equation*} \color{magenta}{c^{2}=a^{2}+b^{2}}\tag{P} \end{equation*}",
    ] {
        let row = classify_formula(source, true, &policy);
        assert_eq!(
            row.class,
            Classification::Accepted,
            "{source}: {}",
            row.detail
        );
    }
}

#[test]
fn simple_formula_prepares_without_default_display_options() {
    let policy = AdmitPolicy::default();
    let inline = prepare_ratex_list("x_i^2", false, &policy).expect("inline");
    let display = prepare_ratex_list("x_i^2", true, &policy).expect("display");
    assert!(!inline.items.is_empty());
    assert!(!display.items.is_empty());
    // Display vs text style must not be identical boxes for a scripted atom.
    assert!(
        inline.width != display.width
            || inline.height != display.height
            || inline.depth != display.depth
            || inline.items.len() != display.items.len()
    );
}
