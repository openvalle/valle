#![cfg(feature = "motion")]
//! Explicit geometry batches and independent styles of ordinary repeated nodes.
use valle_compiler::motion::compile_motion;
use valle_draw::program::{DrawProgram, Node};
use valle_motion::{
    BatchFieldProgress, BatchPositions, BatchStagger, Fonts, GeometryBatchGeometry, LayoutOptions,
    MotionValue, NodeKind, StyleValue, Viewport, build_tree, default_font_naming, emit,
    motion_context_at_frame, prepare_scene, resolve_geometry_batch, resolve_props,
};

#[test]
fn distance_stagger_uses_per_instance_delays_and_zero_matches_scalar_zero() {
    let source = include_str!("fixtures/motion/composition/distance-stagger-batch.motion.tsx");
    let compiled = compile_motion(source).expect("distance stagger compiles");
    fn batch_of(artifact: &valle_motion::SceneArtifact) -> &valle_motion::GeometryBatchSpec {
        artifact
            .nodes
            .iter()
            .find_map(|node| match &node.kind {
                NodeKind::GeometryBatch { batch } => Some(batch),
                _ => None,
            })
            .expect("geometry batch")
    }
    let batch = batch_of(&compiled.artifact);
    assert_eq!(
        batch.opacity_field.as_ref().unwrap().stagger,
        BatchStagger::PerInstance(vec![0.4, 0.2, 0.0, 0.2, 0.4])
    );
    let progress = BatchFieldProgress {
        opacity: 0.5,
        ..Default::default()
    };
    let resolved = resolve_geometry_batch(batch, 15.0, 30.0, progress);
    let alphas = resolved
        .iter()
        .map(|instance| instance.color.alpha)
        .collect::<Vec<_>>();
    assert_eq!(alphas[0], alphas[4]);
    assert_eq!(alphas[1], alphas[3]);
    assert!(alphas[2] > alphas[1] && alphas[1] > alphas[0]);

    let zero_array = compile_motion(&source.replace("stagger: DELAYS", "stagger: [0,0,0,0,0]"))
        .unwrap()
        .artifact;
    let zero_scalar = compile_motion(&source.replace("stagger: DELAYS", "stagger: 0"))
        .unwrap()
        .artifact;
    assert_eq!(
        resolve_geometry_batch(batch_of(&zero_array), 15.0, 30.0, progress),
        resolve_geometry_batch(batch_of(&zero_scalar), 15.0, 30.0, progress)
    );
    for invalid in ["[0,0]", "[-0.1,0,0,0,0]", "[1,0,0,0,0]"] {
        assert!(
            compile_motion(&source.replace("stagger: DELAYS", &format!("stagger: {invalid}")))
                .is_err(),
            "reject {invalid}"
        );
    }
}

#[test]
fn particle_forces_bake_once_and_zero_terms_match_omitted_terms() {
    let source = include_str!("fixtures/motion/composition/particle-forces.motion.tsx");
    let curl = "curlNoise({ seed: 3, scale: 0.012, strength: 220 })";
    let drag = "drag(0.8)";
    let packed = |source: &str| {
        let artifact = compile_motion(source)
            .expect("particle force scene compiles")
            .artifact;
        let prepared = prepare_scene(&artifact).expect("force trajectory bakes");
        let context = motion_context_at_frame(45, 60, FrameRate::new(30, 1).unwrap()).unwrap();
        let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
        let fonts = Fonts::default();
        let tree = build_tree(
            &prepared,
            &context,
            &props,
            &LayoutOptions {
                viewport: Viewport::new((640, 360)),
                fonts: &fonts,
                styles: None,
            },
        )
        .unwrap();
        emit(&tree, &default_font_naming)
            .unwrap()
            .program
            .packed_bytes()
            .unwrap()
    };
    let without_forces = source.replace(&format!("forces: [{curl}, {drag}],"), "");
    let drag_only = source.replace(&format!("{curl}, "), "");
    let curl_only = source.replace(&format!(", {drag}"), "");
    assert_eq!(
        packed(source),
        packed(source),
        "independent prepares are deterministic"
    );
    assert_ne!(packed(source), packed(&without_forces));
    assert_eq!(
        packed(&source.replace("strength: 220", "strength: 0")),
        packed(&drag_only)
    );
    assert_eq!(packed(&source.replace(drag, "drag(0)")), packed(&curl_only));

    let mut invalid = compile_motion(source).unwrap().artifact;
    let spec = invalid
        .nodes
        .iter_mut()
        .find_map(|node| match &mut node.kind {
            NodeKind::GeometryBatch { batch } => match &mut batch.positions {
                BatchPositions::Particles { spec, .. } => Some(spec),
                _ => None,
            },
            _ => None,
        })
        .unwrap();
    spec.forces
        .push(valle_motion::ParticleForce::Drag { coefficient: -1.0 });
    assert!(invalid.validate().is_err());
    assert!(compile_motion(&source.replace(drag, "drag(-1)")).is_err());
}

#[test]
fn prepared_path_batch_shares_path_across_rotated_instances() {
    let source = include_str!("fixtures/motion/composition/path-geometry-batch.motion.tsx");
    let compiled = compile_motion(source).expect("geometry-batch path batch compiles");
    let batch = compiled
        .artifact
        .nodes
        .iter()
        .find_map(|node| match &node.kind {
            NodeKind::GeometryBatch { batch } => Some(batch),
            _ => None,
        })
        .expect("batch node");
    match &batch.geometry {
        GeometryBatchGeometry::Path { path } => {
            assert!(!path.verbs.is_empty());
            assert_eq!(path.verbs.len(), 11);
        }
        _ => panic!("expected prepared path geometry"),
    }
    let prepared = prepare_scene(&compiled.artifact).unwrap();
    let context = motion_context_at_frame(0, 30, FrameRate::new(30, 1).unwrap()).unwrap();
    let props = resolve_props(&compiled.artifact.controls, &Default::default()).unwrap();
    let fonts = Fonts::default();
    let tree = build_tree(
        &prepared,
        &context,
        &props,
        &LayoutOptions {
            viewport: Viewport::new((640, 360)),
            fonts: &fonts,
            styles: None,
        },
    )
    .unwrap();
    let program = emit(&tree, &default_font_naming).unwrap().program;
    let decoded = DrawProgram::from_packed(&program.packed_bytes().unwrap()).unwrap();
    let batches = decoded
        .nodes()
        .iter()
        .filter_map(|node| match node {
            Node::InstanceBatch(batch) => Some(batch),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(batches.len(), 1);
    let valle_draw::program::InstanceShape::Path(path) = batches[0].shape else {
        panic!("expected one shared path shape");
    };
    assert_eq!(decoded.paths()[path.raw() as usize].verbs.len(), 11);
    assert_eq!(batches[0].instances.len(), 2);
    let angle = |row: usize| {
        let transform = batches[0].instances.transforms[row].0;
        transform[1].atan2(transform[0]).to_degrees()
    };
    assert!(angle(0).abs() < 0.001);
    assert!((angle(1) - 36.0).abs() < 0.001);

    let mut invalid = compiled.artifact.clone();
    let batch = invalid
        .nodes
        .iter_mut()
        .find_map(|node| match &mut node.kind {
            NodeKind::GeometryBatch { batch } => Some(batch),
            _ => None,
        })
        .unwrap();
    if let GeometryBatchGeometry::Path { path } = &mut batch.geometry {
        path.points[0].x = f64::INFINITY;
    }
    assert!(
        invalid.validate().is_err(),
        "non-finite batch path is rejected"
    );
}

#[test]
fn atlas_batch_binds_one_image_and_keeps_per_row_transforms() {
    let source = include_str!("fixtures/motion/composition/atlas-geometry-batch.motion.tsx");
    let compiled = compile_motion(source).expect("atlas batch compiles");
    let batch = compiled
        .artifact
        .nodes
        .iter()
        .find_map(|node| match &node.kind {
            NodeKind::GeometryBatch { batch } => Some(batch),
            _ => None,
        })
        .expect("atlas batch");
    assert!(matches!(&batch.geometry,
        GeometryBatchGeometry::Image { source, src }
            if source == "asset://sprites" && src.width == 0.5));
    let prepared = prepare_scene(&compiled.artifact).unwrap();
    let context = motion_context_at_frame(0, 30, FrameRate::new(30, 1).unwrap()).unwrap();
    let props = resolve_props(&compiled.artifact.controls, &Default::default()).unwrap();
    let fonts = Fonts::default();
    let tree = build_tree(
        &prepared,
        &context,
        &props,
        &LayoutOptions {
            viewport: Viewport::new((320, 160)),
            fonts: &fonts,
            styles: None,
        },
    )
    .unwrap();
    let program = emit(&tree, &default_font_naming).unwrap().program;
    let decoded = DrawProgram::from_packed(&program.packed_bytes().unwrap()).unwrap();
    let batches = decoded
        .nodes()
        .iter()
        .filter_map(|node| match node {
            Node::InstanceBatch(batch) => Some(batch),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].instances.len(), 2);
    assert!(matches!(&batches[0].shape,
        valle_draw::program::InstanceShape::Image(region)
            if region.texture.key == "asset://sprites" && region.src.width == 0.5));
    assert_eq!(decoded.requirements().external_textures.len(), 1);
    let first = batches[0].instances.transforms[0].0;
    let second = batches[0].instances.transforms[1].0;
    assert_eq!(first[0], 80.0);
    assert!((second[1].atan2(second[0]).to_degrees() - 36.0).abs() < 0.001);
    for invalid in [
        source.replace("rect(0, 0, 0.5, 1)", "rect(0.8, 0, 0.5, 1)"),
        source.replace("asset://sprites", "asset://missing"),
        source.replace(
            "opacities={[1, 0.7]}",
            "strokeWidths={[1, 1]} opacities={[1, 0.7]}",
        ),
    ] {
        assert!(compile_motion(&invalid).is_err());
    }
}

#[test]
fn batch_path_must_be_prepared() {
    assert!(
        compile_motion(
            r##"
const STAR = path("M 0 0 L 10 0 L 0 10 Z");
export default function P(ctx) {
  return <GeometryBatch geometry={ctx.seconds < 1 ? STAR : STAR}
    positions={[point(20,20)]} sizes={1} fills="#fff" />;
}
"##
        )
        .is_err()
    );
}
use valle_timeline::FrameRate;

#[test]
fn geometry_batch_lowers_to_one_explicit_leaf() {
    let compiled = compile_motion(
        r##"
const PTS = [point(10, 10), point(20, 20)];
export default function P(ctx) {
  return <GeometryBatch key="dots" geometry="circle" positions={PTS} sizes={4} fills="#fff" />;
}
"##,
    )
    .expect("GeometryBatch compiles");
    assert!(
        compiled
            .artifact
            .nodes
            .iter()
            .any(|node| matches!(node.kind, NodeKind::GeometryBatch { .. }))
    );
}

#[test]
fn ordinary_static_nodes_keep_independent_styles_not_a_hidden_batch() {
    let compiled = compile_motion(
        r##"
const PTS = [{x:10,y:10},{x:20,y:20}];
export default function P(ctx) {
  return <View>{PTS.map((p, i) => <View key={`p-${i}`} style={{position:"absolute",left:p.x,top:p.y,width:4,height:4}} />)}</View>;
}
"##,
    )
    .expect("ordinary points compile");
    let lefts = compiled
        .artifact
        .nodes
        .iter()
        .flat_map(|node| &node.styles)
        .filter(|style| style.property == "left")
        .filter_map(|style| match &style.value {
            StyleValue::Static {
                value: MotionValue::Number(value),
            } => Some(*value),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(lefts, vec![10.0, 20.0]);
}

#[test]
fn rotations_are_per_instance_and_support_frame_fields() {
    let compiled = compile_motion(
        r##"
export const composition = { width: 320, height: 180, fps: 30, duration: 1 };
const PTS = [point(60, 90), point(220, 90)];
export default function P(ctx) {
  return <Scene style={{width:320,height:180}}>
    <GeometryBatch key="rects" geometry="rect" positions={PTS}
      sizes={[point(80, 40)]} fills="#ffd000" opacities={[1, 0.5]}
      rotations={field({ from: [0, 0], to: [0, 36], progress: ctx.progress })}
      style={{position:"absolute",left:0,top:0,width:320,height:180}} />
  </Scene>;
}

"##,
    )
    .expect("rotated batch compiles");
    let batch = compiled
        .artifact
        .nodes
        .iter()
        .find_map(|node| match &node.kind {
            NodeKind::GeometryBatch { batch } => Some(batch),
            _ => None,
        })
        .unwrap();
    let resolved = resolve_geometry_batch(
        batch,
        15.0,
        30.0,
        BatchFieldProgress {
            rotation: 0.5,
            ..Default::default()
        },
    );
    assert_eq!(resolved.len(), 2);
    assert_eq!(resolved[0].rotation, 0.0);
    assert_eq!(resolved[1].rotation, 18.0);
    assert_eq!(resolved[1].color.alpha, 0.5);

    let prepared = prepare_scene(&compiled.artifact).unwrap();
    let context = motion_context_at_frame(15, 30, FrameRate::new(30, 1).unwrap()).unwrap();
    let props = resolve_props(&compiled.artifact.controls, &Default::default()).unwrap();
    let fonts = Fonts::default();
    let tree = build_tree(
        &prepared,
        &context,
        &props,
        &LayoutOptions {
            viewport: Viewport::new((320, 180)),
            fonts: &fonts,
            styles: None,
        },
    )
    .unwrap();
    let program = emit(&tree, &default_font_naming).unwrap().program;
    let packed = program.packed_bytes().unwrap();
    let decoded = DrawProgram::from_packed(&packed).unwrap();
    let rotations = decoded
        .nodes()
        .iter()
        .find_map(|node| match node {
            Node::InstanceBatch(batch) => Some(
                batch
                    .instances
                    .transforms
                    .iter()
                    .map(|transform| transform.0[1].atan2(transform.0[0]).to_degrees())
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        })
        .unwrap();
    assert!(rotations[0].abs() < 0.001);
    assert!((rotations[1] - 18.0).abs() < 0.001);

    let mut invalid = compiled.artifact;
    let batch = invalid
        .nodes
        .iter_mut()
        .find_map(|node| match &mut node.kind {
            NodeKind::GeometryBatch { batch } => Some(batch),
            _ => None,
        })
        .unwrap();
    batch.rotations = vec![0.0, 12.0, 36.0];
    assert!(invalid.validate().is_err());
}

#[test]
fn skew_and_stroke_fields_reach_packed_instance_columns() {
    let compiled = compile_motion(
        r##"
export const composition = { width: 320, height: 180, fps: 30, duration: 1 };
const PTS = [point(60, 90), point(220, 90)];
export default function P(ctx) {
  return <Scene style={{width:320,height:180}}>
    <GeometryBatch key="rects" geometry="rect" positions={PTS}
      sizes={point(80,40)} fills="#ffd000"
      skewXs={field({ from: [0, 0], to: [0, 30], progress: ctx.progress })}
      strokeWidths={field({ from: [0, 0], to: [0, 0.05], progress: ctx.progress })}
      style={{position:"absolute",left:0,top:0,width:320,height:180}} />
  </Scene>;
}
"##,
    )
    .expect("skewed and stroked batch compiles");
    let batch = compiled
        .artifact
        .nodes
        .iter()
        .find_map(|node| match &node.kind {
            NodeKind::GeometryBatch { batch } => Some(batch),
            _ => None,
        })
        .unwrap();
    let resolved = resolve_geometry_batch(
        batch,
        15.0,
        30.0,
        BatchFieldProgress {
            skew_x: 0.5,
            stroke_width: 0.5,
            ..Default::default()
        },
    );
    assert_eq!(resolved[0].skew_x, 0.0);
    assert_eq!(resolved[1].skew_x, 15.0);
    assert!((resolved[1].stroke_width - 0.025).abs() < 1e-6);

    let prepared = prepare_scene(&compiled.artifact).unwrap();
    let context = motion_context_at_frame(15, 30, FrameRate::new(30, 1).unwrap()).unwrap();
    let props = resolve_props(&compiled.artifact.controls, &Default::default()).unwrap();
    let fonts = Fonts::default();
    let tree = build_tree(
        &prepared,
        &context,
        &props,
        &LayoutOptions {
            viewport: Viewport::new((320, 180)),
            fonts: &fonts,
            styles: None,
        },
    )
    .unwrap();
    let packed = emit(&tree, &default_font_naming)
        .unwrap()
        .program
        .packed_bytes()
        .unwrap();
    let decoded = DrawProgram::from_packed(&packed).unwrap();
    let instances = decoded
        .nodes()
        .iter()
        .find_map(|node| match node {
            Node::InstanceBatch(batch) => Some(&batch.instances),
            _ => None,
        })
        .unwrap();
    assert_eq!(instances.len(), 2);
    assert_eq!(instances.transforms[0].0[2], 0.0);
    assert!(
        (instances.transforms[1].0[2] / instances.transforms[1].0[3] - 15.0f64.to_radians().tan())
            .abs()
            < 1e-5
    );
    assert_eq!(instances.stroke_widths[0], 0.0);
    assert!((instances.stroke_widths[1] - 0.025).abs() < 1e-6);

    let mut invalid = compiled.artifact.clone();
    let batch = invalid
        .nodes
        .iter_mut()
        .find_map(|node| match &mut node.kind {
            NodeKind::GeometryBatch { batch } => Some(batch),
            _ => None,
        })
        .unwrap();
    batch.skew_xs = vec![90.0];
    assert!(invalid.validate().is_err());
    let mut invalid = compiled.artifact;
    let batch = invalid
        .nodes
        .iter_mut()
        .find_map(|node| match &mut node.kind {
            NodeKind::GeometryBatch { batch } => Some(batch),
            _ => None,
        })
        .unwrap();
    batch.stroke_widths = vec![-0.1];
    assert!(invalid.validate().is_err());
}

#[test]
fn particle_sizes_can_grow_from_zero_and_bad_sizes_name_the_property() {
    let source = include_str!("fixtures/motion/composition/particle-forces.motion.tsx")
        .replace("sizes={[3, 3]}", "sizes={[0, 6]}");
    let artifact = compile_motion(&source).unwrap().artifact;
    let prepared = prepare_scene(&artifact).unwrap();
    let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
    let fonts = Fonts::default();
    for frame in [0, 1, 20] {
        let context = motion_context_at_frame(frame, 60, FrameRate::new(30, 1).unwrap()).unwrap();
        let tree = build_tree(
            &prepared,
            &context,
            &props,
            &LayoutOptions {
                fonts: &fonts,
                viewport: Viewport::new((640, 360)),
                styles: None,
            },
        )
        .unwrap();
        emit(&tree, &default_font_naming)
            .unwrap()
            .program
            .validate()
            .unwrap();
    }
    let errors = compile_motion(&source.replace("sizes={[0, 6]}", "sizes={[-1, 6]}")).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.message.contains("sizes") && error.message.contains("non-negative")),
        "{errors:?}"
    );
}
