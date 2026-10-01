#![cfg(feature = "motion")]

use valle_compiler::motion::compile_motion;
use valle_motion::Expr;

#[test]
fn modifiers_compile_with_fixed_topology_and_dynamic_noise() {
    let source = include_str!("fixtures/motion/composition/path-modifiers.motion.tsx");
    let artifact = compile_motion(source)
        .expect("path-modifiers compiles")
        .artifact;
    assert!(artifact.validate().is_ok());
    assert!(
        artifact
            .exprs
            .iter()
            .any(|expr| matches!(expr, Expr::PathNoiseDisplace { seed: 3, .. }))
    );
    let dynamic = source
        .replace("), 36)}", "), 36 * ctx.progress)}")
        .replace("point(300, 80)", "point(300, 80 + ctx.progress)")
        .replace("size: 10", "size: 10 * ctx.progress");
    let dynamic_artifact = compile_motion(&dynamic)
        .expect("frame geometry modifiers compile")
        .artifact;
    assert!(
        dynamic_artifact
            .exprs
            .iter()
            .any(|expr| matches!(expr, Expr::PathRoundCorners { .. }))
    );
    assert!(
        dynamic_artifact
            .exprs
            .iter()
            .any(|expr| matches!(expr, Expr::PathZigzag { ridges: 24, .. }))
    );
    assert!(compile_motion(&source.replace("ridges: 24", "ridges: ctx.localFrame")).is_err());
    assert!(compile_motion(&source.replace("ridges: 24", "ridges: 0")).is_err());
    assert!(compile_motion(&source.replace("amount: 16", "amount: -1")).is_err());
    assert!(
        compile_motion(&source.replace(
            "roundCorners(path(\"M 40 40 L 240 40 L 240 200 L 40 200 Z\"), 36)",
            "roundCorners(123, 36)"
        ))
        .is_err()
    );
}
