#![cfg(feature = "motion")]

use valle_compiler::motion::compile_motion;
use valle_motion::{Expr, ExprId, MotionValue, geometry::POLAR_MORPH_POINTS};

#[test]
fn disjoint_concave_pair_and_sequence_are_certified_at_prepare_time() {
    let source = include_str!("fixtures/motion/composition/arc-length-morph.motion.tsx");
    let artifact = compile_motion(source)
        .expect("safe concave morph compiles")
        .artifact;
    assert!(artifact.validate().is_ok());
    let (from, to) = artifact
        .exprs
        .iter()
        .find_map(|expr| match expr {
            Expr::PathMorph { from, to, .. } => Some((*from, *to)),
            _ => None,
        })
        .expect("prepared pair");
    let path = |id: ExprId| match &artifact.exprs[id.0 as usize] {
        Expr::Const {
            value: MotionValue::PathData(path),
        } => path,
        _ => panic!("pair correspondence must be static"),
    };
    assert!(path(from).has_same_topology(path(to)));
    assert_eq!(path(from).points.len(), POLAR_MORPH_POINTS);
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
        .expect("prepared sequence");
    assert!(sequence.iter().all(|stop| match &stop.output {
        MotionValue::PathData(path) => path.points.len() == POLAR_MORPH_POINTS,
        _ => false,
    }));
    assert!(compile_motion(&source.replace("method: \"arcLength\"", "method: \"polar\"")).is_err());
    let forced_pair = source.replace(
        "morph(A, B, ctx.progress)",
        "morph(A, B, ctx.progress, { method: \"arcLength\" })",
    );
    assert!(compile_motion(&forced_pair).is_ok());

    let unsafe_source = r##"
        export const composition = { width: 64, height: 64, fps: 30, duration: 2 };
        const FROM = path("M 26 6 L 5 18 L 23 17 L 14 15 L 28 16 Z");
        const TO = path("M 7 5 L 19 2 L 28 20 L 28 28 L 15 24 Z");
        export default function Unsafe(ctx) {
          return <Scene style={{width:64,height:64}}>
            <Path d={morph(FROM, TO, ctx.progress, { method: "arcLength" })} fill="#fff" />
          </Scene>;
        }
    "##;
    assert!(compile_motion(unsafe_source).is_err());
    let allowed = unsafe_source.replace(
        "method: \"arcLength\"",
        "method: \"arcLength\", allowSelfIntersection: true",
    );
    assert!(compile_motion(&allowed).is_ok());
    assert!(
        compile_motion(&allowed.replace("allowSelfIntersection: true", "allowSelfIntersection: 1"))
            .is_err()
    );

    let anchored = source
        .replace(
            "morph(A, B, ctx.progress)",
            "morph(A, B, ctx.progress, { anchors: [[point(140, 40), point(450, 40)], [point(140, 120), point(450, 120)]] })",
        )
        .replace(
            "method: \"arcLength\"",
            "method: \"arcLength\", anchors: [[point(140, 220), point(380, 220), point(590, 220)], [point(140, 300), point(380, 300), point(590, 300)]]",
        );
    let anchored_artifact = compile_motion(&anchored)
        .expect("authored anchors compile")
        .artifact;
    let (from, to) = anchored_artifact
        .exprs
        .iter()
        .find_map(|expr| match expr {
            Expr::PathMorph { from, to, .. } => Some((*from, *to)),
            _ => None,
        })
        .unwrap();
    let anchored_path = |id: ExprId| match &anchored_artifact.exprs[id.0 as usize] {
        Expr::Const {
            value: MotionValue::PathData(path),
        } => path,
        _ => panic!("anchor correspondence must be prepared"),
    };
    for (start, end) in [
        ((140.0, 40.0), (450.0, 40.0)),
        ((140.0, 120.0), (450.0, 120.0)),
    ] {
        let index = anchored_path(from)
            .points
            .iter()
            .position(|point| point.x == start.0 && point.y == start.1)
            .unwrap();
        assert_eq!(
            (
                anchored_path(to).points[index].x,
                anchored_path(to).points[index].y
            ),
            end
        );
    }
    assert!(compile_motion(&anchored.replace("point(450, 40)", "point(400, 80)")).is_err());
    assert!(
        compile_motion(&anchored.replace(
            "point(140, 120), point(450, 120)",
            "point(140, 40), point(450, 40)"
        ))
        .is_err()
    );
}

#[test]
fn authored_anchor_fixture_and_explicit_intersection_fixture_compile() {
    let source = include_str!("fixtures/motion/composition/anchor-morph.motion.tsx");
    let artifact = compile_motion(source)
        .expect("authored anchor fixture compiles")
        .artifact;
    assert!(artifact.validate().is_ok());
    let (from, to) = artifact
        .exprs
        .iter()
        .find_map(|expr| match expr {
            Expr::PathMorph { from, to, .. } => Some((*from, *to)),
            _ => None,
        })
        .unwrap();
    let pair = |id: ExprId| match &artifact.exprs[id.0 as usize] {
        Expr::Const {
            value: MotionValue::PathData(path),
        } => path,
        _ => panic!("pair anchor path must be prepared"),
    };
    for (left, right) in [
        ((140.0, 40.0), (450.0, 40.0)),
        ((140.0, 120.0), (450.0, 120.0)),
    ] {
        let at = pair(from)
            .points
            .iter()
            .position(|point| point.x == left.0 && point.y == left.1)
            .unwrap();
        assert_eq!((pair(to).points[at].x, pair(to).points[at].y), right);
    }
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
        .unwrap();
    let paths = stops
        .iter()
        .map(|stop| match &stop.output {
            MotionValue::PathData(path) => path,
            _ => unreachable!(),
        })
        .collect::<Vec<_>>();
    for row in [
        [(140.0, 220.0), (380.0, 220.0), (590.0, 220.0)],
        [(140.0, 300.0), (380.0, 300.0), (590.0, 300.0)],
    ] {
        let at = paths[0]
            .points
            .iter()
            .position(|point| point.x == row[0].0 && point.y == row[0].1)
            .unwrap();
        for path in 1..3 {
            assert_eq!(
                (paths[path].points[at].x, paths[path].points[at].y),
                row[path]
            );
        }
    }

    let unsafe_source =
        include_str!("fixtures/motion/composition/allow-self-intersection.motion.tsx");
    assert!(compile_motion(unsafe_source).is_ok());
    assert!(
        compile_motion(&unsafe_source.replace(
            "allowSelfIntersection: true",
            "allowSelfIntersection: false"
        ))
        .is_err()
    );
}
