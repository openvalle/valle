#![cfg(feature = "motion")]

use valle_compiler::motion::compile_motion;
use valle_motion::Expr;

#[test]
fn remaining_path_modifiers_compile_for_preparation_and_frames() {
    let source = include_str!("fixtures/motion/composition/extended-path-modifiers.motion.tsx");
    let artifact = compile_motion(source)
        .expect("remaining modifiers compile")
        .artifact;
    assert!(artifact.validate().is_ok());
    assert!(
        artifact
            .exprs
            .iter()
            .any(|expr| matches!(expr, Expr::PathPuckerBloat { .. }))
    );
    assert!(
        artifact
            .exprs
            .iter()
            .any(|expr| matches!(expr, Expr::PathTwist { .. }))
    );
    assert!(
        artifact
            .exprs
            .iter()
            .any(|expr| matches!(expr, Expr::PathSimplify { .. }))
    );
    assert!(
        artifact
            .exprs
            .iter()
            .any(|expr| matches!(expr, Expr::PathStrokeToPath { .. }))
    );
    assert!(compile_motion(&source.replace("sin(ctx.seconds * 2) * 0.7", "2")).is_err());
    assert!(compile_motion(&source.replace("ctx.progress * 18", "-1")).is_err());
    assert!(compile_motion(&source.replace("8 + ctx.progress * 32", "-1")).is_err());
    assert!(compile_motion(&source.replace("ctx.progress * 1.3", "'bad'")).is_err());
}
