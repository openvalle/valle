use serde_json::json;
use valle_draw::{PathVerb, Point};
use valle_motion::geometry::PathData;
use valle_motion::{ControlsSchema, Expr, ExprId, ExprType, MotionValue};

#[test]
fn geometry_dependency_remapping_preserves_order_and_nonreference_metadata() {
    let rows = [
        (json!({"kind":"makePoint","x":1,"y":2}), vec![1, 2]),
        (
            json!({"kind":"makeRect","x":1,"y":2,"width":3,"height":4}),
            vec![1, 2, 3, 4],
        ),
        (
            json!({"kind":"rangeSelector","start":1,"end":2,"offset":3,"softness":4,"shape":"triangle"}),
            vec![1, 2, 3, 4],
        ),
        (json!({"kind":"pathLine","points":[1,2]}), vec![1, 2]),
        (
            json!({"kind":"pathTemplate","verbs":["move","line"],"points":[1,2]}),
            vec![1, 2],
        ),
        (
            json!({"kind":"pathCubic","from":1,"control1":2,"control2":3,"to":4}),
            vec![1, 2, 3, 4],
        ),
        (
            json!({"kind":"pathArc","center":1,"radius":2,"startAngle":3,"endAngle":4}),
            vec![1, 2, 3, 4],
        ),
        (json!({"kind":"pathArea","path":1,"baseline":2}), vec![1, 2]),
        (
            json!({"kind":"pathSector","center":1,"inner":2,"outer":3,"start":4,"end":5,"cornerRadius":6}),
            vec![1, 2, 3, 4, 5, 6],
        ),
        (
            json!({"kind":"pathAreaBand","upper":1,"lower":2}),
            vec![1, 2],
        ),
        (
            json!({"kind":"pathOffset","path":1,"distance":2}),
            vec![1, 2],
        ),
        (json!({"kind":"pathResample","path":1,"count":8}), vec![1]),
        (json!({"kind":"pathReverse","path":1}), vec![1]),
        (
            json!({"kind":"pathRoundCorners","path":1,"radius":2}),
            vec![1, 2],
        ),
        (
            json!({"kind":"pathZigzag","path":1,"size":2,"ridges":8}),
            vec![1, 2],
        ),
        (
            json!({"kind":"pathNoiseDisplace","path":1,"seed":7,"amount":2,"frequency":3,"phase":4}),
            vec![1, 2, 3, 4],
        ),
        (
            json!({"kind":"pathPuckerBloat","path":1,"amount":2}),
            vec![1, 2],
        ),
        (json!({"kind":"pathTwist","path":1,"angle":2}), vec![1, 2]),
        (
            json!({"kind":"pathSimplify","path":1,"tolerance":2}),
            vec![1, 2],
        ),
        (
            json!({"kind":"pathStrokeToPath","path":1,"width":2}),
            vec![1, 2],
        ),
        (
            json!({"kind":"pathMorph","from":1,"to":2,"progress":3}),
            vec![1, 2, 3],
        ),
        (
            json!({"kind":"pathCompatibleMorphSequence","segments":[],"stops":[],"progress":1}),
            vec![1],
        ),
        (
            json!({"kind":"pathPointAt","path":1,"progress":2}),
            vec![1, 2],
        ),
        (
            json!({"kind":"pathTangentAt","path":1,"progress":2}),
            vec![1, 2],
        ),
        (
            json!({"kind":"pathAngleAt","path":1,"progress":2}),
            vec![1, 2],
        ),
        (json!({"kind":"pathLength","path":1}), vec![1]),
        (
            json!({"kind":"pathTrajectory","frames":[],"frame":1}),
            vec![1],
        ),
        (
            json!({"kind":"geometryField","input":1,"field":"width"}),
            vec![1],
        ),
        (json!({"kind":"toLength2","input":1}), vec![1]),
        (
            json!({"kind":"select","condition":1,"whenTrue":2,"whenFalse":3}),
            vec![1, 2, 3],
        ),
        (json!({"kind":"noise2D","seed":7,"x":1,"y":2}), vec![1, 2]),
        (
            json!({"kind":"simulationSample","time":1,"dt":0.1,"duration":0.2,"samples":[0.0,1.0,2.0]}),
            vec![1],
        ),
        (
            json!({"kind":"audioSample","time":1,"fps":30,"samples":[0.0,1.0]}),
            vec![1],
        ),
        (
            json!({"kind":"template","parts":[{"kind":"text","value":"x"},{"kind":"expr","expr":1},{"kind":"expr","expr":1}]}),
            vec![1, 1],
        ),
    ];
    for (wire, children) in rows {
        let mut expr: Expr =
            serde_json::from_value(wire.clone()).unwrap_or_else(|e| panic!("{wire}: {e}"));
        let expected = children.iter().map(|id| ExprId(*id)).collect::<Vec<_>>();
        assert_eq!(expr.children(), expected, "{wire}");
        let original = serde_json::to_value(&expr).unwrap();
        let mut visited = vec![];
        expr.remap_children(|id| {
            visited.push(id);
            ExprId(id.0 + 100)
        });
        assert_eq!(visited, expected, "{wire}");
        assert_eq!(
            expr.children(),
            children
                .iter()
                .map(|id| ExprId(id + 100))
                .collect::<Vec<_>>()
        );
        let changed = serde_json::to_value(&expr).unwrap();
        for key in [
            "seed", "ridges", "count", "dt", "duration", "samples", "fps", "verbs", "stops",
            "segments",
        ] {
            assert_eq!(
                changed.get(key),
                original.get(key),
                "metadata {key}: {wire}"
            );
        }
        expr.remap_children(|id| ExprId(id.0 - 100));
        assert_eq!(serde_json::to_value(expr).unwrap(), original);
    }
}

#[test]
fn path_trajectory_validation_checks_frame_type_topology_and_point_budget() {
    let line = PathData::new(
        vec![PathVerb::Move, PathVerb::Line],
        vec![Point::new(0.0, 0.0), Point::new(1.0, 1.0)],
    )
    .unwrap();
    let curve = PathData::new(
        vec![
            PathVerb::Move,
            PathVerb::Quad,
            PathVerb::Cubic,
            PathVerb::Close,
        ],
        vec![
            Point::new(0.0, 0.0),
            Point::new(1.0, 0.0),
            Point::new(1.0, 1.0),
            Point::new(1.0, 2.0),
            Point::new(0.0, 2.0),
            Point::new(0.0, 0.0),
        ],
    )
    .unwrap();
    let arena = vec![
        Expr::Const {
            value: MotionValue::Number(0.0),
        },
        Expr::Const {
            value: MotionValue::PathData(line.clone()),
        },
    ];
    for (frames, frame, valid) in [
        (vec![line.clone(), line.clone()], ExprId(0), true),
        (vec![line.clone(), curve.clone()], ExprId(0), false),
        (vec![], ExprId(0), false),
        (vec![line.clone()], ExprId(1), false),
    ] {
        let expr = Expr::PathTrajectory { frames, frame };
        let mut errors = vec![];
        assert_eq!(
            valle_motion::expr::validate_next_expr(
                &expr,
                &arena,
                &[Some(ExprType::Number), Some(ExprType::PathData)],
                &ControlsSchema {
                    props: Default::default(),
                    assets: Default::default(),
                    data: Default::default()
                },
                &mut errors
            ),
            Some(ExprType::PathData)
        );
        assert_eq!(errors.is_empty(), valid);
    }
    let mut expressions = arena;
    expressions.push(Expr::Context {
        input: valle_motion::ContextInput::LocalFrame,
    });
    expressions.push(Expr::PathTrajectory {
        frames: vec![curve.clone(), curve],
        frame: ExprId(2),
    });
    expressions.push(Expr::PathOffset {
        path: ExprId(3),
        distance: ExprId(0),
    });
    expressions.push(Expr::PathResample {
        path: ExprId(3),
        count: 8,
    });
    assert_eq!(
        valle_motion::expr::geometry_eval_policy(&expressions, ExprId(3)),
        Some(valle_motion::geometry::GeometryEvalPolicy::PrecomputedTrajectory)
    );
    let mut errors = vec![];
    let expr = Expr::PathStrokeToPath {
        path: ExprId(3),
        width: ExprId(0),
    };
    assert_eq!(
        valle_motion::expr::validate_next_expr(
            &expr,
            &expressions,
            &[
                Some(ExprType::Number),
                Some(ExprType::PathData),
                Some(ExprType::Number),
                Some(ExprType::PathData),
                Some(ExprType::PathData),
                Some(ExprType::PathData)
            ],
            &ControlsSchema {
                props: Default::default(),
                assets: Default::default(),
                data: Default::default()
            },
            &mut errors
        ),
        Some(ExprType::PathData)
    );
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn scalar_controls_normalize_css_units_and_reject_incompatible_values() {
    use valle_motion::ControlType;
    use valle_motion::controls::scalar_binding;
    assert_eq!(
        scalar_binding(&json!("12px"), &ControlType::Length),
        Some(12.0)
    );
    assert_eq!(
        scalar_binding(&json!("0.5turn"), &ControlType::Angle),
        Some(180.0)
    );
    assert_eq!(scalar_binding(&json!(3), &ControlType::Length), Some(3.0));
    assert_eq!(scalar_binding(&json!("10%"), &ControlType::Length), None);
    assert_eq!(scalar_binding(&json!("12px"), &ControlType::String), None);
    assert_eq!(scalar_binding(&json!(true), &ControlType::Angle), None);
}

#[test]
fn admitted_model_counts_and_digest_preserve_the_actual_source() {
    let bytes = include_bytes!("fixtures/scene3d/triangle.glb");
    let model = valle_motion::scene3d::admit_glb(bytes).unwrap();
    let source_bytes = std::hint::black_box(
        valle_motion::scene3d::AdmittedModel::source_bytes
            as fn(&valle_motion::scene3d::AdmittedModel) -> u64,
    );
    assert_eq!(source_bytes(&model), bytes.len() as u64);
    assert_eq!(
        model.content_digest(),
        valle_motion::ContentDigest::of_bytes(bytes)
    );
    assert_eq!(model.vertex_count(), 3);
    assert_eq!(model.triangle_count(), 1);
    assert_eq!(model.rendered_vertex_count(), 3);
    assert_eq!(model.rendered_triangle_count(), 1);
}

#[test]
fn malformed_glb_header_is_rejected_before_any_asset_is_admitted() {
    let error = valle_motion::scene3d::admit_glb(b"invalid header").unwrap_err();
    assert!(error.to_string().contains("header"));
}
