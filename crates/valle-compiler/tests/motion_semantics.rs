#![cfg(feature = "motion")]
//! Transform grammar and deterministic math contracts across compilation and frame-time evaluation.

#![cfg(feature = "motion")]

use valle_compiler::motion::compile_motion;
use valle_motion::value::{Angle, AngleUnit};
use valle_motion::{Expr, Extrapolation, MotionValue, StyleValue};

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

// Typed 2D transforms and authored-order CSS 3D transforms.

#[test]
fn static_transform_lowers_to_per_property_typed_bindings_in_fixed_order() {
    let compiled = compile_motion(&scene(
        r#"transform: "translate(12px 0px) rotate(45deg) scale(1.5)""#,
    ))
    .expect("fixed-order static transform compiles");
    let artifact = compiled.artifact;
    let node = &artifact.nodes[0];
    let properties = node
        .styles
        .iter()
        .map(|style| style.property.as_str())
        .collect::<Vec<_>>();
    assert_eq!(properties, ["translate", "rotate", "scale"]);
    assert!(matches!(
        &node.styles[1].value,
        StyleValue::Static { value: MotionValue::Angle(angle) }
            if angle.value == 45.0 && angle.unit == AngleUnit::Deg
    ));
    assert!(matches!(
        &node.styles[2].value,
        StyleValue::Static { value: MotionValue::Number(scale) } if *scale == 1.5
    ));
    artifact.validate().expect("artifact validates");
}

#[test]
fn rotate_hole_with_unit_suffix_desugars_to_the_bit_exact_identity_interpolate() {
    let compiled = compile_motion(&scene("transform: `rotate(${ctx.enter.progress * 90}deg)`"))
        .expect("rotate(${a}deg) is the sanctioned Number-hole form");
    let artifact = compiled.artifact;
    // Convert a numeric rotation hole to Angle with identity interpolation and Extend extrapolation.
    let identity = artifact.exprs.iter().any(|expression| {
        let Expr::Interpolate {
            stops,
            easings,
            extrapolate_left: Extrapolation::Extend,
            extrapolate_right: Extrapolation::Extend,
            ..
        } = expression
        else {
            return false;
        };
        easings.is_empty()
            && stops.len() == 2
            && stops.iter().enumerate().all(|(index, stop)| {
                let expected = index as f64;
                stop.input == expected
                    && stop.output
                        == MotionValue::Angle(Angle {
                            value: expected,
                            unit: AngleUnit::Deg,
                        })
            })
    });
    assert!(identity, "identity interpolate not found in expr arena");
    // Run artifact validation to require Angle-valued rotation bindings.
    artifact
        .validate()
        .expect("rotate binding type-checks as Angle");
}

#[test]
fn authored_order_and_css_comma_forms_are_admitted_but_nested_functions_stay_closed() {
    compile_motion(&scene(r#"transform: "scale(1.2) rotate(10deg)""#))
        .expect("supported transform functions preserve authored order");
    compile_motion(&scene(r#"transform: "translate(10px, 20px)""#))
        .expect("CSS comma-separated translate is in the deterministic subset");
    reject(
        r#"transform: "scale(calc(1))""#,
        "ordered CSS 3D translate/rotate/scale functions",
    );
}

#[test]
fn hole_suffix_contract_is_per_property() {
    reject(
        "transform: `scale(${ctx.enter.progress}x)`",
        "unitless Number hole",
    );
    reject(
        "transform: `translate(${ctx.enter.progress}px ${ctx.enter.progress}px)`",
        "one Length2-valued hole",
    );
    reject(
        "transform: `rotate(${ctx.enter.progress}grad)`",
        "`deg`/`rad`/`turn` suffix",
    );
    // Diagnostics must not expose internal hole identifiers.
    reject(
        "transform: `rotate(-${ctx.enter.progress}deg)`",
        "span the whole argument",
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
    let compiled = compile_motion(&scene("opacity: Math.sqrt(ctx.enter.progress)"))
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

    let dynamic_pow = compile_motion(&scene("opacity: Math.pow(ctx.enter.progress, 0.8)"))
        .expect("dynamic Math.pow lowers through deterministic Rust math");
    assert!(dynamic_pow.artifact.exprs.iter().any(|expression| matches!(
        expression,
        Expr::MathBinary {
            op: valle_motion::MathBinaryOp::Pow,
            ..
        }
    )));
}
