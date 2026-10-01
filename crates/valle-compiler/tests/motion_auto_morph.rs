#![cfg(feature = "motion")]

use valle_compiler::motion::compile_motion;
use valle_motion::{Expr, ExprId, MotionValue, geometry::POLAR_MORPH_POINTS};

#[test]
fn prepares_polar_correspondence_once() {
    let source = include_str!("fixtures/motion/composition/star-to-circle-morph.motion.tsx");
    let artifact = compile_motion(source)
        .expect("star-to-circle-morph compiles")
        .artifact;
    let (from, to) = artifact
        .exprs
        .iter()
        .find_map(|expr| match expr {
            Expr::PathMorph { from, to, .. } => Some((*from, *to)),
            _ => None,
        })
        .expect("aligned morph expression");
    let aligned = |id: ExprId| match &artifact.exprs[id.0 as usize] {
        Expr::Const {
            value: MotionValue::PathData(path),
        } => path,
        _ => panic!("morph correspondence must be prepared"),
    };
    let (from, to) = (aligned(from), aligned(to));
    assert_eq!(from.points.len(), POLAR_MORPH_POINTS);
    assert!(from.has_same_topology(to));
}

#[test]
fn explicit_polar_rejects_nonoverlapping_kernels_and_auto_uses_checked_correspondence() {
    let source = include_str!("fixtures/motion/composition/star-to-circle-morph.motion.tsx");
    let disjoint = source.replace("point(320, 180)", "point(620, 180)");
    assert!(compile_motion(&disjoint).is_ok());
    let forced_polar = disjoint.replace(
        "morph(STAR, CIRCLE, ctx.progress)",
        "morph(STAR, CIRCLE, ctx.progress, { method: \"polar\" })",
    );
    assert!(compile_motion(&forced_polar).is_err());
}

#[test]
fn sequence_retains_square_corners_at_the_middle_stop() {
    let source = include_str!("fixtures/motion/composition/path-morph-sequence.motion.tsx");
    let artifact = compile_motion(source)
        .expect("path-morph-sequence compiles")
        .artifact;
    let stops = artifact
        .exprs
        .iter()
        .find_map(|expr| match expr {
            Expr::Interpolate { stops, .. }
                if stops.len() == 3
                    && stops
                        .iter()
                        .all(|stop| matches!(stop.output, MotionValue::PathData(_))) =>
            {
                Some(stops)
            }
            _ => None,
        })
        .expect("prepared sequence stops");
    assert_eq!(
        stops.iter().map(|stop| stop.input).collect::<Vec<_>>(),
        [0.0, 0.5, 1.0]
    );
    let MotionValue::PathData(square) = &stops[1].output else {
        unreachable!()
    };
    for (x, y) in [(220.0, 80.0), (420.0, 80.0), (420.0, 280.0), (220.0, 280.0)] {
        assert!(
            square
                .points
                .iter()
                .any(|point| (point.x - x).abs() < 1e-6 && (point.y - y).abs() < 1e-6),
            "missing square corner ({x}, {y})"
        );
    }
    let mut invalid = source.replace("[0, 0.5, 1]", "[0, 0.5, 0.5]");
    assert!(compile_motion(&invalid).is_err());
    invalid = source.replace("[0, 0.5, 1]", "[0, 1]");
    assert!(compile_motion(&invalid).is_err());
}
