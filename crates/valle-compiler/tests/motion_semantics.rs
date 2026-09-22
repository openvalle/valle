#![cfg(feature = "motion")]
//! Transform grammar and deterministic math contracts across compilation and frame-time evaluation.

#![cfg(feature = "motion")]

use valle_compiler::motion::compile_motion;
use valle_motion::{Expr, MotionValue, StyleValue};

/// Insert style fields verbatim into a one-node scene.
fn scene(style: &str) -> String {
    format!(
        "export default function Probe(ctx) {{\n  return <Scene key=\"scene\" style={{{{ {style} }}}} />;\n}}\n"
    )
}

fn reject(style: &str, needle: &str) {
    let diagnostics = compile_motion(&scene(style)).expect_err(style);
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains(needle)),
        "{style}: no diagnostic contains {needle:?}:\n{}",
        diagnostics
            .iter()
            .map(|diagnostic| diagnostic.message.clone())
            .collect::<Vec<_>>()
            .join("\n")
    );
}

// CSS transform lists stay ordered and independent from individual properties.

#[test]
fn static_transform_preserves_the_list_without_overwriting_individual_properties() {
    let compiled = compile_motion(&scene(
        r#"translate:'4px 0px', scale:2, transform: "translate(12px, 0px) rotate(45deg) scale(1.5)""#,
    )).unwrap();
    let node = &compiled.artifact.nodes[0];
    let properties = node
        .styles
        .iter()
        .map(|style| style.property.as_str())
        .collect::<Vec<_>>();
    assert_eq!(properties, ["translate", "scale", "transform"]);
    assert!(
        matches!(&node.styles[2].value, StyleValue::Static { value: MotionValue::Str(value) }
        if value == "translate(12px, 0px) rotate(45deg) scale(1.5)")
    );
    compiled.artifact.validate().unwrap();
}

#[test]
fn numeric_transform_holes_keep_closed_css_structure() {
    let artifact = compile_motion(&scene("transform: `rotate(${ctx.progress * 90}deg)`"))
        .unwrap()
        .artifact;
    assert!(
        artifact
            .exprs
            .iter()
            .any(|value| matches!(value, Expr::Template { .. }))
    );
    assert!(matches!(
        artifact.nodes[0].styles[0].value,
        StyleValue::Expr { .. }
    ));
    artifact.validate().unwrap();
}

#[test]
fn transform_order_optional_arguments_and_length_math_are_admitted() {
    for value in [
        "scale(1.2) rotate(10deg)",
        "translate(10px, 20px)",
        "translate(10px)",
        "translate(calc(10% + 2px))",
        "skew(10deg)",
        "matrix(1,0,0,1,3,4)",
    ] {
        compile_motion(&scene(&format!("transform:'{value}'"))).unwrap();
    }
}

#[test]
fn transform_functions_require_valid_css_arguments_and_check_inactive_branches() {
    for value in [
        "translate(12px 0px)",
        "translateX(1)",
        "translate(auto)",
        "matrix(1,0,0,1,3,4,5)",
        "translateX(1px,2px)",
        "skew(1deg,2deg,3deg)",
        "scale(1) trailing",
        "matrix3d(1)",
        "rotate(10px)",
    ] {
        reject(&format!("transform:'{value}'"), "transform");
    }
    reject("transform: `scale(${ctx.progress}x)`", "transform");
    reject(
        "transform: ctx.localFrame < 30 ? 'none' : 'translate(auto)'",
        "transform",
    );
}

// Compile-time and frame-time math boundaries.

#[test]
fn exactly_specified_math_folds_to_a_cross_host_stable_constant() {
    // IEEE 754 square root gives identical folded constants across hosts.
    let compiled = compile_motion(&scene("opacity: Math.sqrt(0.25)"))
        .expect("exactly-specified Math folds at compile time");
    // Verify the folded value in either a static style or constant expression.
    let folded_half = compiled
        .artifact
        .nodes
        .iter()
        .flat_map(|node| &node.styles)
        .any(|binding| {
            matches!(&binding.value,
                valle_motion::StyleValue::Static { value: MotionValue::Number(value) }
                    if *value == 0.5)
        })
        || compiled.artifact.exprs.iter().any(|expression| {
            matches!(expression,
                Expr::Const { value: MotionValue::Number(value) } if *value == 0.5)
        });
    assert!(
        folded_half,
        "`Math.sqrt(0.25)` must reach the artifact as the constant 0.5"
    );
}

#[test]
fn approximated_math_is_rejected_at_the_module_gate() {
    // Math functions outside the deterministic allowlist cannot use host libm, even with static inputs.
    reject("opacity: Math.log(2)", "implementation-approximated");
}

#[test]
fn deterministic_sqrt_works_with_frame_time_inputs() {
    let compiled = compile_motion(&scene("opacity: Math.sqrt(ctx.progress)"))
        .expect("dynamic Math.sqrt lowers through deterministic Rust math");
    assert!(compiled.artifact.exprs.iter().any(|expression| matches!(
        expression,
        Expr::MathUnary {
            op: valle_motion::MathUnaryOp::Sqrt,
            ..
        }
    )));
}

#[test]
fn deterministic_pow_folds_and_works_with_frame_time_inputs() {
    let static_pow = compile_motion(&scene("opacity: Math.pow(0.25, 0.5)"))
        .expect("static Math.pow uses the deterministic Rust bridge");
    assert!(
        static_pow
            .artifact
            .nodes
            .iter()
            .flat_map(|node| &node.styles)
            .any(|binding| {
                matches!(&binding.value,
                    valle_motion::StyleValue::Static { value: MotionValue::Number(value) }
                        if *value == 0.5)
            })
    );

    let dynamic_pow = compile_motion(&scene("opacity: Math.pow(ctx.progress, 0.8)"))
        .expect("dynamic Math.pow lowers through deterministic Rust math");
    assert!(dynamic_pow.artifact.exprs.iter().any(|expression| matches!(
        expression,
        Expr::MathBinary {
            op: valle_motion::MathBinaryOp::Pow,
            ..
        }
    )));
}
