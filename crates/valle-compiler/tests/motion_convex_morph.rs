#![cfg(feature = "motion")]

use valle_compiler::motion::compile_motion;
use valle_motion::{Expr, ExprId, MotionValue};

#[test]
fn disjoint_convex_pair_and_sequence_prepare_one_edge_direction_grid() {
    let source = include_str!("fixtures/motion/composition/convex-morph.motion.tsx");
    let artifact = compile_motion(source)
        .expect("disjoint convex morphs compile")
        .artifact;
    assert!(artifact.validate().is_ok());
    let pair = artifact
        .exprs
        .iter()
        .find_map(|expr| match expr {
            Expr::PathMorph { from, to, .. } => Some((*from, *to)),
            _ => None,
        })
        .expect("pair");
    let path = |id: ExprId| match &artifact.exprs[id.0 as usize] {
        Expr::Const {
            value: MotionValue::PathData(path),
        } => path,
        _ => panic!("convex correspondence must be prepared"),
    };
    assert!(path(pair.0).has_same_topology(path(pair.1)));
    assert_eq!(path(pair.0).points.len(), 7);
    let sequence = artifact
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
        .expect("sequence");
    let counts = sequence
        .iter()
        .map(|stop| match &stop.output {
            MotionValue::PathData(path) => path.points.len(),
            _ => unreachable!(),
        })
        .collect::<Vec<_>>();
    assert!(counts.iter().all(|count| *count == counts[0]));
    assert!(counts[0] > 4);

    let automatic = source.replace(", { method: \"convex\" })", ")");
    assert!(compile_motion(&automatic).is_ok());
    assert!(compile_motion(&source.replace("method: \"convex\"", "method: \"polar\"")).is_err());
    assert!(
        compile_motion(&source.replace("method: \"convex\"", "method: \"compatible\"")).is_err()
    );
    assert!(compile_motion(&source.replace("method: \"convex\"", "pairs: []")).is_err());

    let star = include_str!("fixtures/motion/composition/star-to-circle-morph.motion.tsx");
    let polar = star.replace(
        "morph(STAR, CIRCLE, ctx.progress)",
        "morph(STAR, CIRCLE, ctx.progress, { method: \"polar\" })",
    );
    assert!(compile_motion(&polar).is_ok());
    assert!(compile_motion(&polar.replace("method: \"polar\"", "method: \"convex\"")).is_err());
}
