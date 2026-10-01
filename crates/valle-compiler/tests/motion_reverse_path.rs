#![cfg(feature = "motion")]

use valle_compiler::motion::compile_motion;
use valle_motion::Expr;

#[test]
fn reverses_the_path_and_accepts_negative_trail_gap() {
    let source = include_str!("fixtures/motion/composition/reverse-path-trail.motion.tsx");
    let artifact = compile_motion(source)
        .expect("reverse-path-trail compiles")
        .artifact;
    assert!(artifact.validate().is_ok());
    let dynamic = source.replace(
        "reversePath(ORBIT)",
        "reversePath(arc(point(320 + ctx.progress, 180), 140, 0, TAU))",
    );
    let dynamic_artifact = compile_motion(&dynamic)
        .expect("dynamic path reversal compiles")
        .artifact;
    assert!(
        dynamic_artifact
            .exprs
            .iter()
            .any(|expr| matches!(expr, Expr::PathReverse { .. }))
    );
    assert!(compile_motion(&source.replace("reversePath(ORBIT)", "reversePath(123)")).is_err());
    assert!(compile_motion(&source.replace("gap: -0.01", "gap: 'bad'")).is_err());
}
