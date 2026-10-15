#[path = "test_support/executor.rs"]
mod support;
use valle_draw::{Rect, program::*};
use valle_engine::{
    compositor::{
        ExternalObjectTable,
        graph::build_render_graph,
        inspect::{FramePerfInspection, FrameStageTimings, PlanInspection, render_graph_dot},
        reference::{ReferenceTarget, execute_reference},
    },
    resource::{Extent2d, ExternalGeneration},
};

fn rectangle(builder: &mut DrawProgramBuilder, rect: Rect, color: LinearColor) -> NodeId {
    let path = builder.push_path(PathData {
        verbs: vec![
            PathVerb::MoveTo,
            PathVerb::LineTo,
            PathVerb::LineTo,
            PathVerb::LineTo,
            PathVerb::Close,
        ],
        points: vec![
            [rect.x, rect.y],
            [rect.right(), rect.y],
            [rect.right(), rect.bottom()],
            [rect.x, rect.bottom()],
        ],
    });
    let paint = builder.push_paint(Paint::Solid(color));
    builder.push_node(Node::Path(PathNode {
        path,
        fill_rule: FillRule::NonZero,
        fill: Some(paint),
        stroke: None,
    }))
}

fn program(
    filter: Option<Filter>,
    clip_kind: usize,
    mask_mode: Option<MaskMode>,
    backdrop: bool,
) -> DrawProgram {
    let mut builder = DrawProgramBuilder::new(Rect::new(0.0, 0.0, 32.0, 32.0));
    let red = rectangle(
        &mut builder,
        Rect::new(4.0, 4.0, 24.0, 24.0),
        LinearColor::new(1.0, 0.0, 0.0, 1.0),
    );
    let mut group = Group::plain(vec![red]);
    if let Some(filter) = filter {
        group.filters.push(filter);
    }
    group.clip = match clip_kind {
        1 => Some(Clip::Rect(Rect::new(8.0, 8.0, 16.0, 16.0))),
        2 => Some(Clip::RoundRect(RoundRect::circular(
            Rect::new(8.0, 8.0, 16.0, 16.0),
            [4.0; 4],
        ))),
        3 => {
            let path = builder.push_path(PathData {
                verbs: vec![
                    PathVerb::MoveTo,
                    PathVerb::LineTo,
                    PathVerb::LineTo,
                    PathVerb::Close,
                ],
                points: vec![[4.0, 4.0], [28.0, 4.0], [16.0, 28.0]],
            });
            Some(Clip::Path {
                path,
                fill_rule: FillRule::NonZero,
            })
        }
        _ => None,
    };
    if let Some(mode) = mask_mode {
        let source = rectangle(
            &mut builder,
            Rect::new(0.0, 0.0, 16.0, 32.0),
            LinearColor::new(1.0, 1.0, 1.0, 1.0),
        );
        group.mask = Some(Mask { source, mode });
    }
    if backdrop {
        group.backdrop = Some(BackdropRead {
            scope: BackdropScope::Current,
            bounds: Rect::new(0.0, 0.0, 32.0, 32.0),
            footprint: Default::default(),
            sampling: valle_draw::requirements::SamplingMode::LinearClamp,
            filters: vec![],
        });
    }
    let root = builder.push_node(Node::Group(group));
    builder.add_root(root);
    builder.finish().unwrap()
}

fn execute(draw: DrawProgram) -> ReferenceTarget {
    let (frame, dynamic) = support::fixture(draw);
    let (plan, bindings) = support::lower(&frame, dynamic);
    let table =
        ExternalObjectTable::try_from_entries(ExternalGeneration::new(1).unwrap(), []).unwrap();
    let mut target =
        ReferenceTarget::new(Extent2d::new(32, 32).unwrap(), frame.render_spec.output());
    execute_reference(
        &plan,
        &bindings,
        &support::capabilities(),
        &table,
        &mut target,
    )
    .unwrap();
    assert_eq!(target.frame().unwrap().extent(), target.extent());
    assert_eq!(target.frame().unwrap().spec(), target.spec());
    assert_eq!(target.frame().unwrap().samples().len(), 1024);
    target
}

#[test]
fn reference_raster_and_clip_masks_have_independent_pixel_oracles() {
    for clip in [0, 1, 3] {
        let target = execute(program(None, clip, None, false));
        let samples = target.frame().unwrap().samples();
        assert_ne!(samples[16 * 32 + 16], samples[0]);
        if clip == 1 {
            assert_eq!(samples[5 * 32 + 5], samples[0]);
        }
    }
    for mode in [
        MaskMode::Alpha,
        MaskMode::Luminance,
        MaskMode::AlphaInverted,
        MaskMode::LuminanceInverted,
    ] {
        let target = execute(program(None, 0, Some(mode), false));
        let samples = target.frame().unwrap().samples();
        assert_eq!(samples[16 * 32 + 8] == samples[0], mode.is_inverted());
        assert_eq!(samples[16 * 32 + 24] == samples[0], !mode.is_inverted());
    }
}

#[test]
fn reference_filter_matrix_is_deterministic_and_commits_complete_frames() {
    let filters = vec![
        Filter::Blur {
            sigma_x: 1.0,
            sigma_y: 0.5,
        },
        Filter::ColorMatrix {
            matrix: Box::new([
                1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0,
                0.0, 0.0, 1.0, 0.0,
            ]),
        },
        Filter::DropShadow {
            offset: [2.0, 1.0],
            sigma_x: 1.0,
            sigma_y: 0.5,
            color: LinearColor::new(0.0, 0.5, 0.0, 0.5),
        },
        Filter::Glow {
            color: AuthorColor::from_srgb8(valle_draw::Rgba::rgb(0, 255, 0)),
            radius: 2.0,
            intensity: 0.5,
        },
        Filter::Bloom {
            threshold: 0.1,
            knee: 0.1,
            intensity: 0.5,
            radius: 1.0,
        },
        Filter::RadialBlur {
            center: [0.5, 0.5],
            amount: 0.05,
        },
        Filter::FilmGrain {
            seed: 17,
            amount: 0.1,
            size: 1.0,
        },
        Filter::LensDistortion { k1: 0.02, k2: 0.01 },
        Filter::ChromaticAberration { offset: [1.0, 0.0] },
    ];
    for filter in filters {
        let draw = program(Some(filter), 0, None, false);
        assert_eq!(execute(draw.clone()), execute(draw));
    }
}

#[test]
fn unsupported_reference_operations_are_rejected_without_publishing_a_frame() {
    let mut draws = vec![program(None, 2, None, false)];
    for filter in [
        Filter::Brightness { amount: 0.5 },
        Filter::Contrast { amount: 0.5 },
        Filter::Grayscale { amount: 0.5 },
        Filter::HueRotate { degrees: 45.0 },
        Filter::Invert { amount: 0.5 },
        Filter::Opacity { amount: 0.5 },
        Filter::Saturate { amount: 0.5 },
        Filter::Sepia { amount: 0.5 },
        Filter::NoiseDisplacement {
            frequency: [0.1, 0.1],
            octaves: 2,
            seed: 19,
            scale: 1.0,
            turbulence: true,
        },
        Filter::VelocityBlur {
            velocity: [2.0, 0.0],
            shutter_angle_degrees: 180.0,
        },
    ] {
        draws.push(program(Some(filter), 0, None, false));
    }
    for draw in draws {
        let (frame, dynamic) = support::fixture(draw);
        let (plan, bindings) = support::lower(&frame, dynamic);
        let table =
            ExternalObjectTable::try_from_entries(ExternalGeneration::new(1).unwrap(), []).unwrap();
        let mut target =
            ReferenceTarget::new(Extent2d::new(32, 32).unwrap(), frame.render_spec.output());
        let original = target.clone();
        assert!(matches!(
            execute_reference(
                &plan,
                &bindings,
                &support::capabilities(),
                &table,
                &mut target
            ),
            Err(valle_engine::compositor::reference::ReferenceExecuteError::UnsupportedPass { .. })
        ));
        assert_eq!(target, original);
    }
}

#[test]
fn program_inspection_includes_local_surfaces_and_backdrop_schedules() {
    let draw = program(
        Some(Filter::Blur {
            sigma_x: 0.5,
            sigma_y: 0.5,
        }),
        1,
        None,
        true,
    );
    let (frame, dynamic) = support::fixture(draw);
    let (plan, bindings) = support::lower(&frame, dynamic);
    let graph = build_render_graph(&frame).unwrap();
    let inspection =
        PlanInspection::build(&graph, &plan, &bindings, &support::capabilities()).unwrap();
    inspection.validate().unwrap();
    assert_eq!(inspection.bound_program_schedules.programs.len(), 1);
    assert!(inspection.counts.live_program_surface_slots > 0);
    assert!(
        inspection.counts.declared_program_surface_slots
            >= inspection.counts.live_program_surface_slots
    );
    assert!(render_graph_dot(&graph).unwrap().contains("backdrop-read"));
    let stages = FrameStageTimings {
        render_open_us: 0,
        semantic_preflight_us: 0,
        evaluate_us: 0,
        prepare_us: 0,
        build_us: 0,
        validate_us: 0,
        lower_us: 0,
        bind_us: 0,
        execute_us: 0,
    };
    FramePerfInspection::build(&frame, &graph, &inspection, stages)
        .unwrap()
        .validate()
        .unwrap();
    execute(program(None, 0, None, true));
}

/// A clip reads its input only inside the clip, and composites read their inputs only at their
/// own pixels, so the composites before a clip bind to the clipped region. Raster targets keep
/// their content ROI: relocating or re-clipping them would change their anti-aliasing. A
/// destination read stays bound to its sample bounds, so the backdrop before it covers them.
#[test]
fn composites_feeding_a_clip_bind_only_the_clipped_region() {
    use valle_engine::compositor::lower::ProgramPassKind;
    use valle_engine::prepare::DeviceRect;

    let mut builder = DrawProgramBuilder::new(Rect::new(0.0, 0.0, 32.0, 32.0));
    let back = rectangle(
        &mut builder,
        Rect::new(0.0, 0.0, 32.0, 32.0),
        LinearColor::new(0.0, 0.0, 1.0, 1.0),
    );
    let ink = rectangle(
        &mut builder,
        Rect::new(4.0, 4.0, 24.0, 24.0),
        LinearColor::new(1.0, 0.0, 0.0, 1.0),
    );
    let mut multiply = Group::plain(vec![ink]);
    multiply.internal_blend = BlendMode::Multiply;
    let multiply = builder.push_node(Node::Group(multiply));
    let mut strip = Group::plain(vec![back, multiply]);
    strip.clip = Some(Clip::Rect(Rect::new(0.0, 10.0, 32.0, 4.0)));
    let root = builder.push_node(Node::Group(strip));
    builder.add_root(root);
    let (frame, dynamic) = support::fixture(builder.finish().unwrap());
    let (plan, bindings) = support::lower(&frame, dynamic);
    let schedules = plan
        .bind_program_schedules(&bindings, &support::capabilities())
        .unwrap();
    let bound = &schedules.programs()[0];
    let rois = plan.programs()[0]
        .local_plan()
        .passes()
        .iter()
        .filter(|pass| !matches!(pass.kind, ProgramPassKind::Clear { .. }))
        .map(|pass| {
            let output = bound
                .resources()
                .iter()
                .find(|resource| resource.resource() == pass.kind.output())
                .unwrap();
            (pass.semantic_path.as_str(), output.device_roi())
        })
        .collect::<Vec<_>>();
    let strip = DeviceRect::new(0, 10, 32, 4);
    let ink_strip = DeviceRect::new(4, 10, 24, 4);
    assert_eq!(
        rois.iter().map(|(_, roi)| *roi).collect::<Vec<_>>(),
        [
            DeviceRect::new(0, 0, 32, 32), // back raster
            DeviceRect::new(0, 4, 32, 24), // back over transparent: strip ∪ destination read
            DeviceRect::new(4, 4, 24, 24), // ink raster
            ink_strip,                     // ink into the isolated multiply group
            DeviceRect::new(4, 4, 24, 24), // destination read keeps its sample bounds
            ink_strip,                     // multiply blend
            strip,                         // strip group result
            strip,                         // clip
            strip,                         // program output
        ],
        "{rois:?}"
    );
}

/// A lens may sample anywhere in its input, so the input keeps its projection beyond the render
/// root. A clip below the lens bounds what can reach it: content larger than the clip binds only
/// the clipped region, not its own bounds.
#[test]
fn a_clip_below_a_kernel_bounds_the_content_it_binds() {
    use valle_engine::compositor::lower::ProgramPassKind;
    use valle_engine::prepare::DeviceRect;

    let mut builder = DrawProgramBuilder::new(Rect::new(0.0, 0.0, 32.0, 32.0));
    let wide = rectangle(
        &mut builder,
        Rect::new(-40.0, -40.0, 112.0, 112.0),
        LinearColor::new(0.0, 0.0, 1.0, 1.0),
    );
    let ink = rectangle(
        &mut builder,
        Rect::new(4.0, 4.0, 24.0, 24.0),
        LinearColor::new(1.0, 0.0, 0.0, 1.0),
    );
    // A blend group keeps the window out of a single raster tree, so its clip is its own pass.
    let mut multiply = Group::plain(vec![ink]);
    multiply.internal_blend = BlendMode::Multiply;
    let multiply = builder.push_node(Node::Group(multiply));
    let mut window = Group::plain(vec![wide, multiply]);
    window.clip = Some(Clip::Rect(Rect::new(-8.0, -8.0, 48.0, 48.0)));
    let window = builder.push_node(Node::Group(window));
    let mut lens = Group::plain(vec![window]);
    lens.filters
        .push(Filter::LensDistortion { k1: 0.02, k2: 0.0 });
    let root = builder.push_node(Node::Group(lens));
    builder.add_root(root);
    let (frame, dynamic) = support::fixture(builder.finish().unwrap());
    let (plan, bindings) = support::lower(&frame, dynamic);
    let schedules = plan
        .bind_program_schedules(&bindings, &support::capabilities())
        .unwrap();
    let bound = &schedules.programs()[0];
    let roi = |pass: &valle_engine::compositor::lower::ProgramPass| {
        bound
            .resources()
            .iter()
            .find(|resource| resource.resource() == pass.kind.output())
            .unwrap()
            .device_roi()
    };
    let passes = plan.programs()[0].local_plan().passes();
    let window = DeviceRect::new(-8, -8, 48, 48);
    for pass in passes {
        if matches!(pass.kind, ProgramPassKind::ApplyFilter { .. }) {
            assert_eq!(
                roi(pass),
                DeviceRect::new(0, 0, 32, 32),
                "{}",
                pass.semantic_path
            );
        } else {
            let bounded = roi(pass).intersect(window) == roi(pass) || roi(pass).is_empty();
            assert!(bounded, "{} binds {:?}", pass.semantic_path, roi(pass));
        }
    }
    // The wide rectangle binds exactly the window, behind a clip pass of its own.
    assert!(passes.iter().any(
        |pass| matches!(pass.kind, ProgramPassKind::RasterNode { .. }) && roi(pass) == window
    ));
    assert!(
        passes
            .iter()
            .any(|pass| matches!(pass.kind, ProgramPassKind::ApplyClip { .. }))
    );
}

/// Blurs and drop shadows read their input up to their footprint away from the pixels they
/// write, including beyond the render root. A blurred surface larger than the frame therefore
/// stays uniform at the frame edge instead of fading into pixels its input never rendered, and
/// a drop shadow reads beyond the root on the side its shadow comes from.
#[test]
fn samplers_read_their_footprint_beyond_the_render_root() {
    use valle_engine::compositor::lower::ProgramPassKind;
    use valle_engine::prepare::DeviceRect;

    for (filter, input_roi, output_roi) in [
        (
            Filter::Blur {
                sigma_x: 2.0,
                sigma_y: 2.0,
            },
            // Three sigmas of support and one sampling pixel past the right and bottom edges.
            DeviceRect::new(8, 8, 31, 31),
            DeviceRect::new(2, 2, 30, 30),
        ),
        (
            Filter::DropShadow {
                offset: [-10.0, 0.0],
                sigma_x: 1.0,
                sigma_y: 1.0,
                color: LinearColor::new(0.0, 0.0, 0.0, 0.5),
            },
            // The shadow at x comes from x + 10: 13 + 1 pixels past the right edge, 3 + 1 below.
            DeviceRect::new(8, 8, 38, 28),
            DeviceRect::new(0, 5, 32, 27),
        ),
    ] {
        let mut builder = DrawProgramBuilder::new(Rect::new(0.0, 0.0, 32.0, 32.0));
        let wide = rectangle(
            &mut builder,
            Rect::new(8.0, 8.0, 64.0, 64.0),
            LinearColor::new(0.0, 0.0, 1.0, 1.0),
        );
        let mut group = Group::plain(vec![wide]);
        group.filters.push(filter.clone());
        let root = builder.push_node(Node::Group(group));
        builder.add_root(root);
        let draw = builder.finish().unwrap();
        let (frame, dynamic) = support::fixture(draw.clone());
        let (plan, bindings) = support::lower(&frame, dynamic);
        let schedules = plan
            .bind_program_schedules(&bindings, &support::capabilities())
            .unwrap();
        let bound = &schedules.programs()[0];
        let roi = |resource| {
            bound
                .resources()
                .iter()
                .find(|bound| bound.resource() == resource)
                .unwrap()
                .device_roi()
        };
        let passes = plan.programs()[0].local_plan().passes();
        let Some(ProgramPassKind::ApplyFilter { input, output, .. }) = passes
            .iter()
            .map(|pass| &pass.kind)
            .find(|kind| matches!(kind, ProgramPassKind::ApplyFilter { .. }))
        else {
            panic!("{filter:?} has no filter pass");
        };
        assert_eq!(roi(*input), input_roi, "{filter:?}");
        assert_eq!(roi(*output), output_roi, "{filter:?}");
        if matches!(filter, Filter::Blur { .. }) {
            let target = execute(draw);
            let samples = target.frame().unwrap().samples();
            for edge in [20 * 32 + 31, 31 * 32 + 20, 31 * 32 + 31] {
                assert_eq!(samples[edge], samples[20 * 32 + 20], "pixel {edge}");
            }
        }
    }
}

fn execute_prepared(
    frame: &valle_engine::prepare::PreparedFrame,
    values: Vec<valle_engine::prepare::DynamicBinding>,
) -> ReferenceTarget {
    let (plan, bindings) = support::lower(frame, values);
    let table =
        ExternalObjectTable::try_from_entries(ExternalGeneration::new(1).unwrap(), []).unwrap();
    let mut target =
        ReferenceTarget::new(Extent2d::new(32, 32).unwrap(), frame.render_spec.output());
    execute_reference(
        &plan,
        &bindings,
        &support::capabilities(),
        &table,
        &mut target,
    )
    .unwrap();
    target
}

#[test]
fn prepared_effects_work_in_both_coordinate_spaces_and_preserve_transparency() {
    let (plain, values) = support::fixture(support::solid_program(vec![]));
    let baseline = execute_prepared(&plain, values);
    for root in [false, true] {
        for kind in 0..7 {
            for region in [false, true] {
                let (frame, values) = support::effect_fixture(kind, root, region);
                let output = execute_prepared(&frame, values.clone());
                assert_eq!(output, execute_prepared(&frame, values));
                assert!(output.frame().unwrap().samples()[0].encoded[3] < 1e-6);
                if [0, 6].contains(&kind) {
                    assert_ne!(
                        output.frame().unwrap().samples()[16 * 32 + 16],
                        baseline.frame().unwrap().samples()[16 * 32 + 16]
                    );
                }
            }
        }
    }
}

#[test]
fn prepared_geometric_masks_obey_shape_feather_and_inversion() {
    for ellipse in [false, true] {
        for feather in [0.0, 1.0] {
            for invert in [false, true] {
                let (frame, values) = support::mask_fixture(ellipse, feather, invert);
                let output = execute_prepared(&frame, values);
                let samples = output.frame().unwrap().samples();
                assert_eq!(samples[16 * 32 + 16].encoded[3] < 1e-6, invert);
                assert_eq!(samples[5 * 32 + 5].encoded[3] < 0.02, !invert);
                if ellipse && feather == 0.0 && !invert {
                    assert_eq!(samples[9 * 32 + 9], samples[0]);
                }
            }
        }
    }
}

#[test]
fn prepared_effect_admission_rejects_invalid_parameters_and_impl_identity() {
    use valle_engine::prepare::{PreparedEffectKernel, PreparedUnitRect, PreparedVisualItem};
    for kernel in [
        PreparedEffectKernel::ColorGrade {
            brightness: 2.0,
            contrast: 0.0,
            saturation: 0.0,
            temperature: 0.0,
            vignette: 0.0,
        },
        PreparedEffectKernel::Spotlight {
            center: [-0.1, 0.5],
            radius: 0.2,
            feather: 0.1,
            intensity: 1.0,
        },
        PreparedEffectKernel::ExtensionColorGain {
            implementation_sha256: [0; 32],
            gain: 1.0,
            past_frames: 0,
            future_frames: 0,
        },
        PreparedEffectKernel::ExtensionColorGain {
            implementation_sha256: valle_engine::render::engine_owned_kernel_implementation_sha256(
                valle_engine::render::EXTENSION_COLOR_GAIN_ABI,
            )
            .unwrap(),
            gain: f32::NAN,
            past_frames: 0,
            future_frames: 0,
        },
        PreparedEffectKernel::GaussianBlur {
            sigma_device_px: support::id(4),
            region: Some(PreparedUnitRect {
                x: 0.9,
                y: 0.1,
                width: 0.2,
                height: 0.2,
            }),
        },
        PreparedEffectKernel::ChromaKey {
            key_working_linear_rec2020: [0.0, 1.0, 0.0],
            intensity: 1.0,
            shadow: 0.0,
            feather_sigma_device_px: support::id(4),
            edge_clean: 0.0,
        },
    ] {
        let (mut frame, _) = support::effect_fixture(0, true, false);
        frame.adjustments[0].kernel = kernel;
        assert!(frame.validate().is_err());
    }
    let (mut frame, _) = support::effect_fixture(0, false, false);
    if let PreparedVisualItem::Layer(layer) = &mut frame.visual[0] {
        layer.effects[0].semantic_path.clear();
    }
    assert!(frame.validate().is_err());
}

#[test]
fn graph_packets_reject_forged_resource_layouts_writers_and_capabilities() {
    use valle_engine::compositor::graph::{
        GraphOrigin, GraphResourceKind, GraphRoi, GraphValidationError, RenderGraph,
        ResourceAccess, ResourceId,
    };
    let (frame, _) = support::fixture(support::solid_program(vec![]));
    let graph = build_render_graph(&frame).unwrap();
    let restored: RenderGraph = serde_json::from_slice(&graph.canonical_bytes().unwrap()).unwrap();
    assert_eq!(
        restored.semantic_hash().unwrap(),
        graph.semantic_hash().unwrap()
    );
    let mut duplicate = graph.clone();
    let writer = duplicate
        .edges
        .iter()
        .find(|edge| edge.access == ResourceAccess::Write)
        .unwrap()
        .clone();
    duplicate.edges.push(writer);
    assert!(matches!(
        duplicate.topological_order(),
        Err(GraphValidationError::MultipleWriters { .. })
    ));
    assert!(duplicate.validate().is_err());
    assert!(duplicate.canonical_bytes().is_err());
    let mut mutations = vec![];
    let mut changed = graph.clone();
    changed.resources[0].semantic_path.clear();
    mutations.push(changed);
    let mut changed = graph.clone();
    changed.resources[0].kind = GraphResourceKind::Mask;
    mutations.push(changed);
    let mut changed = graph.clone();
    changed.resources[0].origin = GraphOrigin::Static { x: 1, y: 0 };
    mutations.push(changed);
    let mut changed = graph.clone();
    let layer = changed
        .resources
        .iter_mut()
        .find(|r| matches!(r.kind, GraphResourceKind::Layer { .. }))
        .unwrap();
    layer.roi = GraphRoi::FullFrame;
    mutations.push(changed);
    let mut changed = graph.clone();
    changed.capabilities.clear();
    mutations.push(changed);
    let mut changed = graph.clone();
    changed.passes[0].semantic_path = "unexpected".into();
    mutations.push(changed);
    let mut changed = graph.clone();
    changed.edges.pop();
    mutations.push(changed);
    let mut changed = graph.clone();
    changed.output = ResourceId::try_from(1).unwrap();
    mutations.push(changed);
    for mutation in mutations {
        let error = mutation.validate().unwrap_err();
        assert!(!error.to_string().is_empty());
        assert!(
            serde_json::from_value::<RenderGraph>(serde_json::to_value(&mutation).unwrap())
                .is_err()
        );
    }
    let (plan, bindings) =
        support::lower(&frame, support::fixture(support::solid_program(vec![])).1);
    assert_eq!(bindings.programs(), bindings.clone().programs());
    let program = &bindings.programs()[0];
    assert_eq!(program.frame_id().get(), 1);
    assert_eq!(program.content_hash(), &frame.programs[0].content_hash);
    assert!(!program.packed().is_empty());
    let _ = program.local_plan();
    let _ = program.local_schedule();
    let _ = program.frame_hash();
    let wire = serde_json::to_value(&plan).unwrap();
    for (pointer, value) in [
        ("/passes/0/semanticPath", serde_json::json!("bad")),
        (
            "/resources/0/roi",
            serde_json::json!({"kind":"static","rect":{"x":0,"y":0,"width":1,"height":1}}),
        ),
        ("/requiredCapabilities", serde_json::json!([])),
        ("/surfaceSlots/0/estimatedBytes", serde_json::json!(1)),
    ] {
        let mut altered = wire.clone();
        *altered.pointer_mut(pointer).unwrap() = value;
        assert!(
            serde_json::from_value::<valle_engine::compositor::lower::RenderPlanTemplate>(altered)
                .is_err(),
            "{pointer}"
        );
    }
}

#[test]
fn graph_effect_packets_reject_mismatched_identity_and_coordinate_space() {
    use valle_engine::{compositor::graph::LogicalPassKind, prepare::PreparedEffectSpace};
    let (frame, _) = support::effect_fixture(0, false, false);
    let original = build_render_graph(&frame).unwrap();
    for wrong_space in [false, true] {
        let mut graph = original.clone();
        let effect = graph
            .passes
            .iter_mut()
            .find_map(|pass| {
                if let LogicalPassKind::Filter { effect, .. } = &mut pass.kind {
                    Some(effect)
                } else {
                    None
                }
            })
            .unwrap();
        if wrong_space {
            effect.space = PreparedEffectSpace::Root;
        } else {
            effect.semantic_path = "visual[0].effects[9]".into();
        }
        let error = graph.validate().unwrap_err().to_string();
        assert!(error.contains("identity or coordinate space"), "{error}");
    }
    let mut graph = original;
    for pass in &mut graph.passes {
        if let LogicalPassKind::Filter { effect, .. } = &mut pass.kind {
            effect.kernel = valle_engine::prepare::PreparedEffectKernel::ChromaKey {
                key_working_linear_rec2020: [0.0, 1.0, 0.0],
                intensity: 0.8,
                shadow: 0.0,
                feather_sigma_device_px: support::id(4),
                edge_clean: 0.0,
            };
        }
    }
    let error = graph.validate().unwrap_err().to_string();
    assert!(error.contains("source-alpha operators"), "{error}");
}

#[test]
fn program_packets_roundtrip_and_reject_corrupted_baselines_and_frame_patches() {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use valle_engine::{
        compositor::lower::{PlanProgram, RenderBindings, RenderPlanTemplate},
        resource::ContentDigest,
    };
    let (frame, dynamic) = support::fixture(support::solid_program(vec![]));
    let (plan, bindings) = support::lower(&frame, dynamic);
    let packet = serde_json::to_value(&bindings).unwrap();
    let restored: RenderBindings = serde_json::from_value(packet.clone()).unwrap();
    let template: RenderPlanTemplate =
        serde_json::from_value(serde_json::to_value(&plan).unwrap()).unwrap();
    template.validate_bindings(&restored).unwrap();
    assert_eq!(restored.programs(), bindings.programs());
    assert_eq!(
        restored.programs()[0].packed(),
        bindings.programs()[0].packed()
    );
    assert_eq!(
        template.required_capabilities(),
        plan.required_capabilities()
    );
    assert!(template.programs().is_empty());
    assert_eq!(template.optimization(), plan.optimization());
    let program: PlanProgram = serde_json::from_value(packet["programs"][0].clone()).unwrap();
    assert_eq!(program, restored.programs()[0]);

    for field in ["patch", "framePatch"] {
        for invalid in ["%%%", ""] {
            let mut altered = packet["programs"][0].clone();
            altered[field] = serde_json::json!(invalid);
            let error = serde_json::from_value::<PlanProgram>(altered).unwrap_err();
            assert!(!error.to_string().is_empty());
        }
    }
    let wire = serde_json::to_value(&plan).unwrap();
    for (bytes_field, hash_field) in [
        ("baselinePacked", "baselineContentHash"),
        ("baselineFramePacked", "baselineFrameHash"),
    ] {
        let mut altered = wire.clone();
        let invalid = b"not a valid program";
        altered["programs"][0][bytes_field] = serde_json::json!(STANDARD.encode(invalid));
        altered["programs"][0][hash_field] =
            serde_json::to_value(ContentDigest::of_bytes(invalid)).unwrap();
        let error = serde_json::from_value::<RenderPlanTemplate>(altered).unwrap_err();
        assert!(error.to_string().contains(bytes_field), "{error}");
    }
}
