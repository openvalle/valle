#![cfg(feature = "motion")]

use valle_compiler::motion::compile_motion;
use valle_draw::program::Node;
use valle_motion::layout::SceneReferenceKind;
use valle_motion::layout::build_tree_profiled;
use valle_motion::{
    Expr, ExprId, Fonts, IndexFormula, InstanceColumnValues, LayoutOptions, NodeId, Viewport,
    default_font_naming, emit, layout_boxes, motion_context_at_frame, motion_context_at_source,
    prepare_scene, resolve_props,
};
use valle_timeline::{FrameRate, RationalTime, internal::SampleTime};

#[test]
fn expanded_absolute_leaves_match_full_evaluation() {
    let source = r##"
        export const composition = { width: 640, height: 360, fps: 30, duration: 2 };
        const DOTS = Array.from({ length: 64 }, (_, i) => ({ id: `d${i}`, i, x: 4 + (i % 8) * 8, y: 4 + Math.floor(i / 8) * 14 }));
        export default function LazyEval(ctx) {
          return <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
            {DOTS.map((dot) => (
              <View key={dot.id} className="absolute" visible={ctx.seconds * 1000 >= dot.i}
                style={{ left: dot.x, top: dot.y, width: 4, height: 4, backgroundColor: "#ffffff",
                  opacity: clamp(ctx.seconds * 1000 - dot.i, 0, 1) }} />
            ))}
          </Scene>;
        }
    "##;
    let expanded = source.replace("className=\"absolute\"", "className=\"absolute \"");
    let artifact = compile_motion(&expanded).unwrap().artifact;
    // The ternary preserves the visibility condition but prevents the simple time-gate fast path.
    let eager = compile_motion(&expanded.replace(
        "visible={ctx.seconds * 1000 >= dot.i}",
        "visible={(ctx.seconds * 1000 >= dot.i ? true : false)}",
    ))
    .unwrap()
    .artifact;
    assert!(artifact.instance_groups.is_empty());
    assert!(eager.instance_groups.is_empty());
    let prepared = prepare_scene(&artifact).unwrap();
    let eager_prepared = prepare_scene(&eager).unwrap();
    let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
    let fonts = Fonts::default();
    for (frame, visible_dots) in [(0, 1), (1, 34), (30, 64)] {
        let ctx = motion_context_at_frame(frame, 60, FrameRate::new(30, 1).unwrap()).unwrap();
        let options = LayoutOptions {
            viewport: Viewport::new((640, 360)),
            fonts: &fonts,
            styles: None,
        };
        let (tree, timings) = build_tree_profiled(&prepared, &ctx, &props, &options).unwrap();
        let (eager_tree, eager_timings) =
            build_tree_profiled(&eager_prepared, &ctx, &props, &options).unwrap();
        assert_eq!(timings.sample_static_computed, 0);
        assert_eq!(timings.sample_static_reused, 0);
        assert_eq!(timings.active_nodes, visible_dots + 1, "frame {frame}");
        assert_eq!(eager_timings.active_nodes, eager.nodes.len());
        assert_eq!(eager_timings.evaluated_expressions, eager.exprs.len());
        assert!(
            timings.evaluated_expressions < eager_timings.evaluated_expressions,
            "frame {frame}: lazy {} eager {}",
            timings.evaluated_expressions,
            eager_timings.evaluated_expressions
        );
        let boxes = layout_boxes(&artifact, &tree).unwrap();
        for dot in 0..64 {
            assert_eq!(
                boxes.contains_key(&format!("d{dot}")),
                dot < visible_dots,
                "dot {dot} at frame {frame}"
            );
        }
        assert_eq!(
            emit(&tree, &default_font_naming).unwrap().program,
            emit(&eager_tree, &default_font_naming).unwrap().program,
            "DrawProgram at frame {frame}"
        );
    }
}

#[test]
fn time_scope_keeps_independent_leaves_lazy_at_mapped_and_random_access_times() {
    let source = r##"
        export const composition = { width: 640, height: 360, fps: 30, duration: 2 };
        const DOTS = Array.from({ length: 64 }, (_, i) => ({ id: `d${i}`, i, x: 4 + (i % 8) * 8, y: 4 + Math.floor(i / 8) * 14 }));
        export default function Scoped(ctx) {
          return <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
            <TimeScope key="scope" offset={0.5} speed={2}>
              {DOTS.map((dot) => (
                <View key={dot.id} className="absolute " visible={ctx.seconds * 30 >= dot.i}
                  style={{ left: dot.x, top: dot.y, width: 4, height: 4, backgroundColor: "#ffffff",
                    opacity: clamp(ctx.seconds * 30 - dot.i, 0, 1) }} />
              ))}
            </TimeScope>
          </Scene>;
        }
    "##;
    let artifact = compile_motion(source).unwrap().artifact;
    let eager = compile_motion(&source.replace(
        "visible={ctx.seconds * 30 >= dot.i}",
        "visible={(ctx.seconds * 30 >= dot.i ? true : false)}",
    ))
    .unwrap()
    .artifact;
    assert!(artifact.instance_groups.is_empty());
    assert!(eager.instance_groups.is_empty());
    let prepared = prepare_scene(&artifact).unwrap();
    let eager_prepared = prepare_scene(&eager).unwrap();
    let dot = artifact
        .nodes
        .iter()
        .position(|node| node.key == "d0")
        .unwrap();
    assert!(
        prepared
            .dependencies()
            .node(NodeId(dot as u32))
            .unwrap()
            .ordinary_container_path
    );
    let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
    let fonts = Fonts::default();
    let options = LayoutOptions {
        viewport: Viewport::new((640, 360)),
        fonts: &fonts,
        styles: None,
    };
    for (frame, visible) in [(30, 31), (0, 0), (59, 64), (16, 2), (15, 1), (30, 31)] {
        let ctx = motion_context_at_frame(frame, 60, FrameRate::new(30, 1).unwrap()).unwrap();
        let (tree, timings) = build_tree_profiled(&prepared, &ctx, &props, &options).unwrap();
        let (full, full_timings) =
            build_tree_profiled(&eager_prepared, &ctx, &props, &options).unwrap();
        assert_eq!(timings.active_nodes, visible + 2, "frame {frame}");
        assert!(timings.evaluated_expressions < full_timings.evaluated_expressions);
        assert_eq!(
            emit(&tree, &default_font_naming).unwrap().program,
            emit(&full, &default_font_naming).unwrap().program,
            "frame {frame}"
        );
    }
}

#[test]
fn mixed_path_sibling_keeps_unreferenced_leaves_lazy_and_referenced_geometry() {
    let source = include_str!("fixtures/motion/composition/mixed-lazy-evaluation.motion.tsx");
    let artifact = compile_motion(source).unwrap().artifact;
    let anchored = source.replace(
        "bounds(\"d20\").width / 4",
        "anchor(\"d20\", \"right\").x / 164",
    );
    assert!(
        compile_motion(&anchored)
            .unwrap()
            .artifact
            .instance_groups
            .is_empty()
    );
    let eager =
        compile_motion(&source.replace("className=\"absolute\"", "className=\"absolute \""))
            .unwrap()
            .artifact;
    let prepared = prepare_scene(&artifact).unwrap();
    let eager_prepared = prepare_scene(&eager).unwrap();
    let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
    let fonts = Fonts::default();
    for frame in [0, 1, 30] {
        let ctx = motion_context_at_frame(frame, 60, FrameRate::new(30, 1).unwrap()).unwrap();
        let options = LayoutOptions {
            viewport: Viewport::new((640, 360)),
            fonts: &fonts,
            styles: None,
        };
        let (tree, timings) = build_tree_profiled(&prepared, &ctx, &props, &options).unwrap();
        let eager_tree = build_tree_profiled(&eager_prepared, &ctx, &props, &options)
            .unwrap()
            .0;
        assert_eq!(
            emit(&tree, &default_font_naming)
                .unwrap()
                .program
                .packed_bytes()
                .unwrap(),
            emit(&eager_tree, &default_font_naming)
                .unwrap()
                .program
                .packed_bytes()
                .unwrap(),
            "frame {frame}"
        );
        if frame == 0 {
            let boxes = layout_boxes(&artifact, &tree).unwrap();
            assert!(boxes.contains_key("d20"));
            assert!(!boxes.contains_key("d21"));
            assert!(timings.active_nodes < 10);
            assert!(timings.evaluated_expressions * 5 < artifact.exprs.len());
        }
    }
}

#[test]
fn nested_absolute_leaves_keep_container_layout_and_eager_draw_program() {
    let source = include_str!("fixtures/motion/composition/nested-lazy-evaluation.motion.tsx");
    let artifact = compile_motion(source).unwrap().artifact;
    let eager =
        compile_motion(&source.replace("className=\"absolute\"", "className=\"absolute \""))
            .unwrap()
            .artifact;
    let prepared = prepare_scene(&artifact).unwrap();
    let eager_prepared = prepare_scene(&eager).unwrap();
    let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
    let fonts = Fonts::default();
    for frame in [0, 1, 30] {
        let ctx = motion_context_at_frame(frame, 60, FrameRate::new(30, 1).unwrap()).unwrap();
        let options = LayoutOptions {
            viewport: Viewport::new((640, 360)),
            fonts: &fonts,
            styles: None,
        };
        let (tree, timings) = build_tree_profiled(&prepared, &ctx, &props, &options).unwrap();
        let eager_tree = build_tree_profiled(&eager_prepared, &ctx, &props, &options)
            .unwrap()
            .0;
        assert_eq!(
            emit(&tree, &default_font_naming)
                .unwrap()
                .program
                .packed_bytes()
                .unwrap(),
            emit(&eager_tree, &default_font_naming)
                .unwrap()
                .program
                .packed_bytes()
                .unwrap(),
            "frame {frame}"
        );
        let boxes = layout_boxes(&artifact, &tree).unwrap();
        let eager_boxes = layout_boxes(&eager, &eager_tree).unwrap();
        for key in ["outer", "inner", "d20"] {
            assert_eq!(boxes[key], eager_boxes[key], "{key} at frame {frame}");
        }
        if frame == 0 {
            assert!(!boxes.contains_key("d21"));
            assert!(timings.active_nodes < 10);
            assert!(timings.evaluated_expressions * 5 < artifact.exprs.len());
        } else if frame == 1 {
            assert!(boxes.contains_key("d3"));
            assert!(!boxes.contains_key("d4"));
            assert!(timings.active_nodes < 10);
            assert!(timings.evaluated_expressions * 5 < artifact.exprs.len());
        }
    }
}

#[test]
fn shifted_reversed_time_gate_keeps_absolute_leaves_lazy() {
    let source = include_str!("fixtures/motion/composition/nested-lazy-evaluation.motion.tsx")
        .replace(
            "ctx.seconds * 100 >= dot.i",
            "dot.i <= (ctx.seconds - 0.25) * 100",
        );
    let artifact = compile_motion(&source).unwrap().artifact;
    let eager =
        compile_motion(&source.replace("className=\"absolute\"", "className=\"absolute \""))
            .unwrap()
            .artifact;
    let prepared = prepare_scene(&artifact).unwrap();
    let eager_prepared = prepare_scene(&eager).unwrap();
    let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
    let fonts = Fonts::default();
    for frame in [0, 8, 30] {
        let ctx = motion_context_at_frame(frame, 60, FrameRate::new(30, 1).unwrap()).unwrap();
        let options = LayoutOptions {
            viewport: Viewport::new((640, 360)),
            fonts: &fonts,
            styles: None,
        };
        let (tree, timings) = build_tree_profiled(&prepared, &ctx, &props, &options).unwrap();
        let eager_tree = build_tree_profiled(&eager_prepared, &ctx, &props, &options)
            .unwrap()
            .0;
        assert_eq!(
            emit(&tree, &default_font_naming)
                .unwrap()
                .program
                .packed_bytes()
                .unwrap(),
            emit(&eager_tree, &default_font_naming)
                .unwrap()
                .program
                .packed_bytes()
                .unwrap(),
            "frame {frame}"
        );
        if frame == 0 {
            assert!(!layout_boxes(&artifact, &tree).unwrap().contains_key("d21"));
            assert!(timings.active_nodes < 10);
            assert!(timings.evaluated_expressions * 5 < artifact.exprs.len());
        }
    }
}

#[test]
fn local_visibility_certificates_skip_gates_away_from_a_threshold() {
    let source = include_str!("fixtures/motion/composition/nested-lazy-evaluation.motion.tsx");
    let artifact = compile_motion(source).unwrap().artifact;
    let prepared = prepare_scene(&artifact).unwrap();
    let mut without_fixed_domain = artifact.clone();
    without_fixed_domain.composition.as_mut().unwrap().fps = None;
    let without_certificate = prepare_scene(&without_fixed_domain).unwrap();
    let eager =
        compile_motion(&source.replace("className=\"absolute\"", "className=\"absolute \""))
            .unwrap()
            .artifact;
    let eager_prepared = prepare_scene(&eager).unwrap();
    let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
    let fonts = Fonts::default();
    let fps = FrameRate::new(30, 1).unwrap();
    let contexts = [
        motion_context_at_frame(0, 60, fps).unwrap(),
        motion_context_at_frame(1, 60, fps).unwrap(),
        motion_context_at_source(
            0,
            SampleTime::new(RationalTime::new(1_999_999, 100_000_000).unwrap()),
            60,
            fps,
        )
        .unwrap(),
        motion_context_at_source(
            0,
            SampleTime::new(RationalTime::new(1, 50).unwrap()),
            60,
            fps,
        )
        .unwrap(),
        motion_context_at_source(
            0,
            SampleTime::new(RationalTime::new(2_000_001, 100_000_000).unwrap()),
            60,
            fps,
        )
        .unwrap(),
        motion_context_at_frame(30, 60, fps).unwrap(),
    ];
    for (index, ctx) in contexts.iter().enumerate() {
        let options = LayoutOptions {
            viewport: Viewport::new((640, 360)),
            fonts: &fonts,
            styles: None,
        };
        let (tree, timings) = build_tree_profiled(&prepared, ctx, &props, &options).unwrap();
        let eager_tree = build_tree_profiled(&eager_prepared, ctx, &props, &options)
            .unwrap()
            .0;
        assert_eq!(
            emit(&tree, &default_font_naming)
                .unwrap()
                .program
                .packed_bytes()
                .unwrap(),
            emit(&eager_tree, &default_font_naming)
                .unwrap()
                .program
                .packed_bytes()
                .unwrap(),
            "sample {index}"
        );
        if index < 2 || index == 5 {
            let uncached = without_certificate.frame_evaluation_stats(ctx);
            assert!(
                timings.evaluated_expressions + 40 < uncached.evaluated_expressions,
                "sample {index}: certified {} uncached {}",
                timings.evaluated_expressions,
                uncached.evaluated_expressions
            );
        }
    }
}

#[test]
fn whole_duration_visibility_certificate_skips_only_proven_hidden_leaves() {
    let source = include_str!("fixtures/motion/composition/nested-lazy-evaluation.motion.tsx")
        .replace(
            "ctx.seconds * 100 >= dot.i",
            "ctx.seconds * 100 >= dot.i + 1000",
        );
    let artifact = compile_motion(&source).unwrap().artifact;
    let eager =
        compile_motion(&source.replace("className=\"absolute\"", "className=\"absolute \""))
            .unwrap()
            .artifact;
    let prepared = prepare_scene(&artifact).unwrap();
    let eager_prepared = prepare_scene(&eager).unwrap();
    let mut without_fixed_domain = artifact.clone();
    without_fixed_domain.composition.as_mut().unwrap().fps = None;
    let without_certificate = prepare_scene(&without_fixed_domain).unwrap();
    let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
    let fonts = Fonts::default();
    let fps = FrameRate::new(30, 1).unwrap();
    let outside = motion_context_at_source(
        0,
        SampleTime::new(RationalTime::new(20, 1).unwrap()),
        60,
        fps,
    )
    .unwrap();
    for ctx in [
        motion_context_at_frame(0, 60, fps).unwrap(),
        motion_context_at_frame(30, 60, fps).unwrap(),
        outside,
    ] {
        let options = LayoutOptions {
            viewport: Viewport::new((640, 360)),
            fonts: &fonts,
            styles: None,
        };
        let (tree, timings) = build_tree_profiled(&prepared, &ctx, &props, &options).unwrap();
        let eager_tree = build_tree_profiled(&eager_prepared, &ctx, &props, &options)
            .unwrap()
            .0;
        assert!(
            emit(&tree, &default_font_naming)
                .unwrap()
                .program
                .packed_bytes()
                .unwrap()
                == emit(&eager_tree, &default_font_naming)
                    .unwrap()
                    .program
                    .packed_bytes()
                    .unwrap()
        );
        if ctx.sample < SampleTime::new(RationalTime::new(2, 1).unwrap()) {
            let uncached = without_certificate.frame_evaluation_stats(&ctx);
            assert!(timings.evaluated_expressions + 40 < uncached.evaluated_expressions);
            assert!(timings.active_nodes < 10);
        } else {
            assert_eq!(timings.active_nodes, artifact.nodes.len());
        }
    }
}

#[test]
fn mask_sibling_does_not_disable_independent_leaf_activation() {
    let source =
        include_str!("fixtures/motion/composition/mask-sibling-lazy-evaluation.motion.tsx");
    let artifact = compile_motion(source).unwrap().artifact;
    assert!(artifact.instance_groups.is_empty());
    let eager =
        compile_motion(&source.replace("className=\"absolute\"", "className=\"absolute \""))
            .unwrap()
            .artifact;
    let prepared = prepare_scene(&artifact).unwrap();
    let eager_prepared = prepare_scene(&eager).unwrap();
    let node_id = |key: &str| {
        NodeId(
            artifact
                .nodes
                .iter()
                .position(|node| node.key == key)
                .unwrap() as u32,
        )
    };
    let graph = prepared.dependencies();
    assert!(graph.node(node_id("mask")).unwrap().composition_boundary);
    assert!(graph.node(node_id("clip")).unwrap().composition_boundary);
    assert!(
        graph
            .node(node_id("transition"))
            .unwrap()
            .composition_boundary
    );
    assert!(
        graph
            .node(node_id("backdrop"))
            .unwrap()
            .composition_boundary
    );
    assert!(graph.node(node_id("backdrop")).unwrap().reads_backdrop);
    assert!(graph.node(node_id("transition")).unwrap().sample_dependent);
    assert!(graph.node(node_id("label")).unwrap().sample_dependent);
    assert!(graph.node(node_id("d21")).unwrap().ordinary_container_path);
    assert!(graph.node(node_id("d20")).unwrap().geometry_referenced);
    assert!(!graph.node(node_id("d21")).unwrap().geometry_referenced);
    assert!(graph.references().iter().any(|edge| {
        edge.consumer == node_id("marker")
            && edge.source == node_id("d20")
            && edge.kind == SceneReferenceKind::Geometry
    }));
    assert!(graph.references().iter().any(|edge| {
        edge.consumer == node_id("mask")
            && edge.source == node_id("mask-source")
            && edge.kind == SceneReferenceKind::MaskPaint
    }));
    let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
    let fonts = Fonts::default();
    for frame in [0, 1, 30] {
        let ctx = motion_context_at_frame(frame, 60, FrameRate::new(30, 1).unwrap()).unwrap();
        let options = LayoutOptions {
            viewport: Viewport::new((640, 360)),
            fonts: &fonts,
            styles: None,
        };
        let (tree, timings) = build_tree_profiled(&prepared, &ctx, &props, &options).unwrap();
        let eager_tree = build_tree_profiled(&eager_prepared, &ctx, &props, &options)
            .unwrap()
            .0;
        assert_eq!(
            emit(&tree, &default_font_naming)
                .unwrap()
                .program
                .packed_bytes()
                .unwrap(),
            emit(&eager_tree, &default_font_naming)
                .unwrap()
                .program
                .packed_bytes()
                .unwrap(),
            "frame {frame}"
        );
        if frame == 0 {
            let boxes = layout_boxes(&artifact, &tree).unwrap();
            assert!(boxes.contains_key("d20"));
            assert!(!boxes.contains_key("d21"));
            assert!(timings.active_nodes < 19);
            assert!(timings.evaluated_expressions * 5 < artifact.exprs.len());
        }
    }
}

#[test]
fn frame_one_evaluates_less_than_one_fifth_of_full_work() {
    let source = include_str!("fixtures/motion/composition/lazy-evaluation.motion.tsx");
    let artifact = compile_motion(source).unwrap().artifact;
    let mut forged = artifact.clone();
    let number = forged.instance_groups[0]
        .exprs
        .iter()
        .position(|expr| {
            matches!(
                expr,
                Expr::Const {
                    value: valle_motion::MotionValue::Number(_)
                }
            )
        })
        .unwrap();
    forged.instance_groups[0].template.visibility = Some(ExprId(number as u32));
    assert!(forged.validate().is_err());
    let prepared = prepare_scene(&artifact).unwrap();
    let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
    let fonts = Fonts::default();
    let ctx = motion_context_at_frame(1, 60, FrameRate::new(30, 1).unwrap()).unwrap();
    let (tree, timings) = build_tree_profiled(
        &prepared,
        &ctx,
        &props,
        &LayoutOptions {
            viewport: Viewport::new((640, 360)),
            fonts: &fonts,
            styles: None,
        },
    )
    .unwrap();
    assert_eq!(artifact.nodes.len(), 2);
    assert_eq!(artifact.instance_groups.len(), 1);
    assert_eq!(artifact.instance_groups[0].rows(), 2000);
    assert_eq!(timings.active_nodes, 2);
    assert_eq!(timings.layout_nodes, 2);
    let stats = prepared.frame_evaluation_stats(&ctx);
    assert_eq!(stats.evaluated_expressions, timings.evaluated_expressions);
    assert_eq!(stats.active_nodes, timings.active_nodes);
    assert_eq!(stats.layout_nodes, timings.layout_nodes);
    let full_evaluations =
        artifact.exprs.len() + artifact.instance_groups[0].frame_expression_evaluations();
    assert!(
        timings.evaluated_expressions * 5 < full_evaluations,
        "evaluated {} of {} possible expressions",
        timings.evaluated_expressions,
        full_evaluations
    );
    let initial = emit(&tree, &default_font_naming).unwrap().program;
    initial.validate().unwrap();
    let initial_batch = initial
        .nodes()
        .iter()
        .find_map(|node| match node {
            Node::InstanceBatch(batch) => Some(batch),
            _ => None,
        })
        .unwrap();
    assert_eq!(initial_batch.instances.len(), 34);
    let initial_bytes = initial.packed_bytes().unwrap();

    // The same local frame may be sampled at an earlier shutter/echo time. Its activation must
    // follow that sample, and a later request for the original time must recover identical bytes.
    let earlier = motion_context_at_source(
        1,
        SampleTime::new(RationalTime::new(1, 60).unwrap()),
        60,
        FrameRate::new(30, 1).unwrap(),
    )
    .unwrap();
    let (earlier_tree, earlier_timings) = build_tree_profiled(
        &prepared,
        &earlier,
        &props,
        &LayoutOptions {
            viewport: Viewport::new((640, 360)),
            fonts: &fonts,
            styles: None,
        },
    )
    .unwrap();
    assert_eq!(earlier_timings.active_nodes, 2);
    let earlier_program = emit(&earlier_tree, &default_font_naming).unwrap().program;
    let earlier_batch = earlier_program
        .nodes()
        .iter()
        .find_map(|node| match node {
            Node::InstanceBatch(batch) => Some(batch),
            _ => None,
        })
        .unwrap();
    assert_eq!(earlier_batch.instances.len(), 17);
    let repeated = build_tree_profiled(
        &prepared,
        &ctx,
        &props,
        &LayoutOptions {
            viewport: Viewport::new((640, 360)),
            fonts: &fonts,
            styles: None,
        },
    )
    .unwrap()
    .0;
    assert_eq!(
        emit(&repeated, &default_font_naming)
            .unwrap()
            .program
            .packed_bytes()
            .unwrap(),
        initial_bytes
    );
}

#[test]
fn twenty_thousand_rows_keep_frame_one_work_near_constant() {
    let source = include_str!("fixtures/motion/composition/lazy-evaluation.motion.tsx");
    let larger = source.replace("length: 2000", "length: 20000");
    let small = compile_motion(source).unwrap().artifact;
    let large = compile_motion(&larger).unwrap().artifact;
    assert_eq!(small.nodes.len(), 2);
    assert_eq!(large.nodes.len(), 2);
    assert_eq!(small.instance_groups[0].rows(), 2000);
    assert_eq!(large.instance_groups[0].rows(), 20000);
    assert_eq!(
        small.instance_groups[0].exprs,
        large.instance_groups[0].exprs
    );
    assert!(large.instance_groups[0].columns.iter().any(|column| {
        column.name == "i"
            && matches!(
                &column.values,
                InstanceColumnValues::Formula {
                    expression: IndexFormula::Index,
                    ..
                }
            )
    }));
    let ctx = motion_context_at_frame(1, 60, FrameRate::new(30, 1).unwrap()).unwrap();
    let small_stats = prepare_scene(&small).unwrap().frame_evaluation_stats(&ctx);
    let prepared = prepare_scene(&large).unwrap();
    let large_stats = prepared.frame_evaluation_stats(&ctx);
    assert!(large_stats.evaluated_expressions <= small_stats.evaluated_expressions + 4);
    let props = resolve_props(&large.controls, &Default::default()).unwrap();
    let fonts = Fonts::default();
    let (tree, timings) = build_tree_profiled(
        &prepared,
        &ctx,
        &props,
        &LayoutOptions {
            viewport: Viewport::new((640, 360)),
            fonts: &fonts,
            styles: None,
        },
    )
    .unwrap();
    assert_eq!(
        timings.evaluated_expressions,
        large_stats.evaluated_expressions
    );
    let program = emit(&tree, &default_font_naming).unwrap().program;
    let batch = program
        .nodes()
        .iter()
        .find_map(|node| match node {
            Node::InstanceBatch(batch) => Some(batch),
            _ => None,
        })
        .unwrap();
    assert_eq!(batch.instances.len(), 34);
}

#[test]
fn shifted_reversed_time_gate_selects_instance_range() {
    let source = include_str!("fixtures/motion/composition/affine-time-dependency.motion.tsx");
    let artifact = compile_motion(source).unwrap().artifact;
    assert_eq!(artifact.instance_groups.len(), 1);
    let eager = compile_motion(&source.replace(
        "dot.i <= (ctx.seconds - 0.2) * 1000",
        "(dot.i <= (ctx.seconds - 0.2) * 1000 ? true : false)",
    ))
    .unwrap()
    .artifact;
    assert_eq!(eager.instance_groups.len(), 1);
    let prepared = prepare_scene(&artifact).unwrap();
    let eager_prepared = prepare_scene(&eager).unwrap();
    let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
    let fonts = Fonts::default();
    for (frame, expected_rows) in [(0, 0), (8, 67), (30, 128)] {
        let ctx = motion_context_at_frame(frame, 60, FrameRate::new(30, 1).unwrap()).unwrap();
        let options = LayoutOptions {
            viewport: Viewport::new((640, 360)),
            fonts: &fonts,
            styles: None,
        };
        let (tree, timings) = build_tree_profiled(&prepared, &ctx, &props, &options).unwrap();
        let eager_tree = build_tree_profiled(&eager_prepared, &ctx, &props, &options)
            .unwrap()
            .0;
        let program = emit(&tree, &default_font_naming).unwrap().program;
        let selected_rows = program
            .nodes()
            .iter()
            .find_map(|node| match node {
                Node::InstanceBatch(batch) => Some(batch.instances.len()),
                _ => None,
            })
            .unwrap_or(0);
        assert_eq!(selected_rows, expected_rows, "frame {frame}");
        assert!(
            program.packed_bytes().unwrap()
                == emit(&eager_tree, &default_font_naming)
                    .unwrap()
                    .program
                    .packed_bytes()
                    .unwrap(),
            "frame {frame}"
        );
        if frame == 0 {
            let full =
                artifact.exprs.len() + artifact.instance_groups[0].frame_expression_evaluations();
            assert!(timings.evaluated_expressions * 5 < full);
        }
    }

    // The local frame remains 8 while a shutter sample moves to an earlier composition time.
    let earlier = motion_context_at_source(
        8,
        SampleTime::new(RationalTime::new(1, 5).unwrap()),
        60,
        FrameRate::new(30, 1).unwrap(),
    )
    .unwrap();
    let options = LayoutOptions {
        viewport: Viewport::new((640, 360)),
        fonts: &fonts,
        styles: None,
    };
    let early_tree = build_tree_profiled(&prepared, &earlier, &props, &options)
        .unwrap()
        .0;
    let eager_tree = build_tree_profiled(&eager_prepared, &earlier, &props, &options)
        .unwrap()
        .0;
    let early_program = emit(&early_tree, &default_font_naming).unwrap().program;
    let early_rows = early_program.nodes().iter().find_map(|node| match node {
        Node::InstanceBatch(batch) => Some(batch.instances.len()),
        _ => None,
    });
    assert_eq!(early_rows, Some(1));
    assert!(
        early_program.packed_bytes().unwrap()
            == emit(&eager_tree, &default_font_naming)
                .unwrap()
                .program
                .packed_bytes()
                .unwrap()
    );
}

#[test]
fn nonmonotone_instance_visibility_evaluates_rows_individually() {
    let source = r##"
        export const composition = { width: 64, height: 32, fps: 30, duration: 1 };
        const DOTS = Array.from({ length: 8 }, (_, i) => ({ id: `d${i}`, i, x: i * 8 }));
        export default function Alternating(ctx) {
          return <Scene className="relative h-full w-full">
            {DOTS.map((dot) => <View key={dot.id} className="absolute"
              visible={dot.i % 2 === 0}
              style={{ left: dot.x, top: 4, width: 4, height: 4,
                backgroundColor: "#ffffff", opacity: ctx.progress }} />)}
          </Scene>;
        }
    "##;
    let artifact = compile_motion(source).unwrap().artifact;
    assert_eq!(artifact.instance_groups.len(), 1);
    let prepared = prepare_scene(&artifact).unwrap();
    let ctx = motion_context_at_frame(15, 30, FrameRate::new(30, 1).unwrap()).unwrap();
    let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
    let fonts = Fonts::default();
    let (tree, _) = build_tree_profiled(
        &prepared,
        &ctx,
        &props,
        &LayoutOptions {
            viewport: Viewport::new((64, 32)),
            fonts: &fonts,
            styles: None,
        },
    )
    .unwrap();
    let program = emit(&tree, &default_font_naming).unwrap().program;
    let batch = program
        .nodes()
        .iter()
        .find_map(|node| match node {
            Node::InstanceBatch(batch) => Some(batch),
            _ => None,
        })
        .unwrap();
    assert_eq!(batch.instances.len(), 4);
    assert_eq!(
        batch
            .instances
            .transforms
            .iter()
            .map(|transform| transform.0[4])
            .collect::<Vec<_>>(),
        [0.0, 16.0, 32.0, 48.0]
    );
}

#[test]
fn index_visibility_range_respects_inclusive_and_exclusive_boundaries() {
    let source = include_str!("fixtures/motion/composition/lazy-evaluation.motion.tsx")
        .replace("length: 2000", "length: 64");
    let fonts = Fonts::default();
    let ctx = motion_context_at_frame(0, 60, FrameRate::new(30, 1).unwrap()).unwrap();
    for (operator, count) in [("<", 63), ("<=", 64), (">", 0), (">=", 1)] {
        let source = source.replace(">= dot.i", &format!("{operator} dot.i"));
        let artifact = compile_motion(&source).unwrap().artifact;
        assert_eq!(artifact.instance_groups.len(), 1);
        let prepared = prepare_scene(&artifact).unwrap();
        let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
        let (tree, _) = build_tree_profiled(
            &prepared,
            &ctx,
            &props,
            &LayoutOptions {
                viewport: Viewport::new((640, 360)),
                fonts: &fonts,
                styles: None,
            },
        )
        .unwrap();
        let program = emit(&tree, &default_font_naming).unwrap().program;
        let drawn = program
            .nodes()
            .iter()
            .find_map(|node| match node {
                Node::InstanceBatch(batch) => Some(batch.instances.len()),
                _ => None,
            })
            .unwrap_or(0);
        assert_eq!(drawn, count, "{operator}");
    }
}

#[test]
fn callback_index_drives_the_same_visible_prefix() {
    let source = include_str!("fixtures/motion/composition/lazy-evaluation.motion.tsx")
        .replace("length: 2000", "length: 64")
        .replace("DOTS.map((dot) =>", "DOTS.map((dot, index) =>")
        .replace(">= dot.i", ">= index");
    let artifact = compile_motion(&source).unwrap().artifact;
    assert_eq!(artifact.instance_groups.len(), 1);
    assert!(
        artifact.instance_groups[0]
            .exprs
            .iter()
            .any(|expr| matches!(expr, Expr::InstanceIndex))
    );
    let prepared = prepare_scene(&artifact).unwrap();
    let ctx = motion_context_at_frame(1, 60, FrameRate::new(30, 1).unwrap()).unwrap();
    let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
    let fonts = Fonts::default();
    let (tree, _) = build_tree_profiled(
        &prepared,
        &ctx,
        &props,
        &LayoutOptions {
            viewport: Viewport::new((640, 360)),
            fonts: &fonts,
            styles: None,
        },
    )
    .unwrap();
    let program = emit(&tree, &default_font_naming).unwrap().program;
    let active = program
        .nodes()
        .iter()
        .find_map(|node| match node {
            Node::InstanceBatch(batch) => Some(batch.instances.len()),
            _ => None,
        })
        .unwrap();
    assert_eq!(active, 34);
}

#[test]
fn hidden_flow_node_still_positions_its_sibling() {
    let artifact = compile_motion(
        r##"export default function Flow(ctx) { return (
          <Scene key="root" style={{display:"flex",width:160,height:60}}>
            <View key="hidden" visible={ctx.seconds > 1} style={{width:50,height:20}} />
            <View key="sibling" style={{width:10,height:20,backgroundColor:"#fff"}} />
          </Scene>
        ); }"##,
    )
    .unwrap()
    .artifact;
    let prepared = prepare_scene(&artifact).unwrap();
    let ctx = motion_context_at_frame(0, 60, FrameRate::new(30, 1).unwrap()).unwrap();
    let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
    let fonts = Fonts::default();
    let (tree, timings) = build_tree_profiled(
        &prepared,
        &ctx,
        &props,
        &LayoutOptions {
            viewport: Viewport::new((160, 60)),
            fonts: &fonts,
            styles: None,
        },
    )
    .unwrap();
    assert_eq!(timings.layout_nodes, artifact.nodes.len());
    let boxes = layout_boxes(&artifact, &tree).unwrap();
    assert_eq!(boxes["hidden"][2], 50.0);
    assert_eq!(boxes["sibling"][0], 50.0);
}

#[test]
fn bounds_reference_keeps_hidden_absolute_geometry() {
    let artifact = compile_motion(
        r##"export default function Referenced(ctx) { return (
          <Scene key="root" style={{width:160,height:60}}>
            <View key="hidden" className="absolute" visible={ctx.seconds > 1}
              style={{left:10,top:4,width:40,height:20,backgroundColor:"#fff"}} />
            <View key="reader" className="absolute"
              style={{left:70,top:4,width:10,height:10,backgroundColor:"#fff",
                opacity: bounds("hidden").width / 40}} />
          </Scene>
        ); }"##,
    )
    .unwrap()
    .artifact;
    let prepared = prepare_scene(&artifact).unwrap();
    let ctx = motion_context_at_frame(0, 60, FrameRate::new(30, 1).unwrap()).unwrap();
    let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
    let fonts = Fonts::default();
    let (tree, timings) = build_tree_profiled(
        &prepared,
        &ctx,
        &props,
        &LayoutOptions {
            viewport: Viewport::new((160, 60)),
            fonts: &fonts,
            styles: None,
        },
    )
    .unwrap();
    assert_eq!(timings.layout_nodes, artifact.nodes.len());
    let boxes = layout_boxes(&artifact, &tree).unwrap();
    assert_eq!(boxes["hidden"][2], 40.0);
    emit(&tree, &default_font_naming)
        .unwrap()
        .program
        .validate()
        .unwrap();
}

#[test]
fn visible_descendant_keeps_its_hidden_parent_in_layout() {
    let artifact = compile_motion(
        r##"export default function Descendant(ctx) { return (
          <Scene key="root" style={{width:160,height:60}}>
            <View key="parent" className="absolute" visible={ctx.seconds > 1}
              style={{left:10,top:4,width:40,height:20,backgroundColor:"#f00"}}>
              <View key="child" visible={true}
                style={{width:10,height:10,backgroundColor:"#fff"}} />
            </View>
          </Scene>
        ); }"##,
    )
    .unwrap()
    .artifact;
    let prepared = prepare_scene(&artifact).unwrap();
    let ctx = motion_context_at_frame(0, 60, FrameRate::new(30, 1).unwrap()).unwrap();
    let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
    let fonts = Fonts::default();
    let (tree, timings) = build_tree_profiled(
        &prepared,
        &ctx,
        &props,
        &LayoutOptions {
            viewport: Viewport::new((160, 60)),
            fonts: &fonts,
            styles: None,
        },
    )
    .unwrap();
    assert_eq!(timings.layout_nodes, artifact.nodes.len());
    let boxes = layout_boxes(&artifact, &tree).unwrap();
    assert_eq!(boxes["parent"], [10.0, 4.0, 40.0, 20.0]);
    assert_eq!(boxes["child"], [10.0, 4.0, 10.0, 10.0]);
    emit(&tree, &default_font_naming)
        .unwrap()
        .program
        .validate()
        .unwrap();
}
