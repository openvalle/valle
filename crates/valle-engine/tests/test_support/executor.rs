#![allow(dead_code)]
use valle_draw::program::DrawProgram;
use valle_engine::{
    compositor::{
        graph::{GraphCapability, build_render_graph},
        lower::{
            BackendCapabilities, FramebufferFetchSemantics, RenderBindings, RenderPlanTemplate,
            lower_render_graph,
        },
    },
    frame::{CompositeBlendMode, RenderQuality, RenderSpec},
    prepare::{
        BoundsReason, DeviceRect, DeviceTransform, DynamicBinding, DynamicBindingId,
        DynamicBindingKind, DynamicBindings, DynamicValue, FrameGeometry, FrameMetadata,
        LayerDynamicSlots, PreparedBackground, PreparedFrame, PreparedLayer, PreparedProgram,
        PreparedSource, PreparedSourceKind, PreparedVisualItem, ProgramId,
    },
    render::{FrameKey, RenderId},
    resource::{
        ContentDigest, Extent2d, ExternalGeneration, ExternalPixelLayout, OutputBackground,
        OutputSpec, TextureFormat, TextureUsage,
    },
};

pub fn capabilities() -> BackendCapabilities {
    use GraphCapability::*;
    BackendCapabilities::new(
        Extent2d::new(4096, 4096).unwrap(),
        [TextureFormat::Rgba32Float, TextureFormat::Rgba16Float],
        [
            TextureUsage::Sampled,
            TextureUsage::StorageRead,
            TextureUsage::StorageWrite,
            TextureUsage::ColorAttachment,
        ],
        [1],
        [ExternalPixelLayout::Rgba8],
        [
            Clear,
            ExternalImport,
            SourcePipeline,
            DrawProgram,
            BackdropRead,
            Group,
            Filter,
            Mask,
            Blend,
            Transition,
            AdjustmentEffect,
            Caption,
            OutputTransform,
        ],
        true,
        Some(FramebufferFetchSemantics::CoherentWorkingPremultiplied),
        32 * 1024 * 1024,
        64 * 1024 * 1024,
    )
    .unwrap()
}

pub fn fixture(draw: DrawProgram) -> (PreparedFrame, Vec<DynamicBinding>) {
    let projected = |bounds: valle_draw::requirements::LocalBounds| {
        bounds
            .rect()
            .map(|rect| {
                DeviceRect::new(
                    rect.x.floor() as i32,
                    rect.y.floor() as i32,
                    (rect.right().ceil() - rect.x.floor()) as u32,
                    (rect.bottom().ceil() - rect.y.floor()) as u32,
                )
                .intersect(DeviceRect::full(32, 32))
            })
            .unwrap_or(DeviceRect::new(0, 0, 0, 0))
    };
    let content = projected(draw.requirements().content_bounds);
    let rect = projected(draw.requirements().output_bounds);
    let transform = DeviceTransform::from_affine([32.0, 0.0, 0.0, 0.0, 32.0, 0.0]);
    let bounds = valle_engine::prepare::PreparedBounds {
        content,
        output: rect,
        sample: rect,
        reason: BoundsReason::Exact,
        max_intermediate_pixels: 32 * 32,
    };
    let mut dynamic = vec![
        binding(
            1,
            "visual[0].transform",
            DynamicBindingKind::DeviceTransform,
            DynamicValue::DeviceTransform(transform),
        ),
        binding(
            2,
            "visual[0].bounds",
            DynamicBindingKind::Bounds,
            DynamicValue::Bounds(rect),
        ),
        binding(
            3,
            "visual[0].opacity",
            DynamicBindingKind::Opacity,
            DynamicValue::Scalar(1.0),
        ),
    ];
    let destination_uses = draw
        .requirements()
        .destination_uses
        .iter()
        .enumerate()
        .map(|(index, required)| {
            let sample = dynamic.len() as u32 + 1;
            dynamic.push(binding(
                sample,
                &format!("visual[0].destination[{index}].sampleBounds"),
                DynamicBindingKind::BackdropSampleBounds,
                DynamicValue::Bounds(DeviceRect::full(32, 32)),
            ));
            let output = dynamic.len() as u32 + 1;
            dynamic.push(binding(
                output,
                &format!("visual[0].destination[{index}].outputBounds"),
                DynamicBindingKind::BackdropOutputBounds,
                DynamicValue::Bounds(DeviceRect::full(32, 32)),
            ));
            valle_engine::prepare::PreparedDestinationUse {
                node: required.node,
                scope: required.scope.clone(),
                sample_bounds: id(sample),
                output_bounds: id(output),
                operation: required.operation.clone(),
                bounds_reason: BoundsReason::Exact,
            }
        })
        .collect::<Vec<_>>();
    let packed = draw.packed_bytes().unwrap();
    let program: PreparedProgram = serde_json::from_value(serde_json::json!({
        "id":1, "kind":"solid", "semanticPath":"visual[0].solid", "contentHash":ContentDigest::of_bytes(&packed),
        "viewport":draw.viewport(), "packed":packed, "requirements":draw.requirements(),
        "resources":{"textures":[],"fonts":[],"runtimeShaders":[],"scenes":[]}, "destinationUses":destination_uses,
    })).unwrap();
    let layer = PreparedLayer {
        semantic_path: "visual[0]".into(),
        clip_id: "clip:contract".into(),
        track_id: "track:contract".into(),
        source: PreparedSource::Program {
            source_kind: PreparedSourceKind::Solid,
            program: serde_json::from_value::<ProgramId>(serde_json::json!(1)).unwrap(),
        },
        device_transform: transform,
        bounds,
        dynamic: LayerDynamicSlots {
            transform: id(1),
            bounds: id(2),
            opacity: id(3),
        },
        effects: vec![],
        mask: None,
        blend: CompositeBlendMode::Normal,
    };
    let frame = PreparedFrame {
        render_id: RenderId::from_bytes([0x44; 32]),
        key: FrameKey::new(0),
        sample_time: serde_json::from_value(serde_json::json!({"composition":"0/1"})).unwrap(),
        render_spec: RenderSpec::new(
            32,
            32,
            RenderQuality::Preview,
            OutputSpec::srgb_preview(OutputBackground::Transparent).unwrap(),
        )
        .unwrap(),
        background: PreparedBackground {
            working_linear_rec2020_premul: [0.0; 4],
        },
        visual: vec![PreparedVisualItem::Layer(Box::new(layer))],
        adjustments: vec![],
        captions: vec![],
        programs: vec![program],
        resources: Default::default(),
        metadata: FrameMetadata {
            geometry: vec![FrameGeometry {
                semantic_path: "visual[0]".into(),
                transform,
                bounds,
            }],
        },
    };
    (frame, dynamic)
}

pub fn id(value: u32) -> DynamicBindingId {
    DynamicBindingId::try_from(value).unwrap()
}
pub fn binding(
    value: u32,
    path: &str,
    kind: DynamicBindingKind,
    value_data: DynamicValue,
) -> DynamicBinding {
    DynamicBinding {
        id: id(value),
        semantic_path: path.into(),
        binding_kind: kind,
        value: value_data,
    }
}
pub fn lower(
    frame: &PreparedFrame,
    values: Vec<DynamicBinding>,
) -> (RenderPlanTemplate, RenderBindings) {
    frame.validate().unwrap();
    let dynamic: DynamicBindings =
        serde_json::from_value(serde_json::to_value(values).unwrap()).unwrap();
    let graph = build_render_graph(frame).unwrap();
    let plan = lower_render_graph(&graph, &dynamic, &capabilities()).unwrap();
    let bindings = RenderBindings::new(
        frame.render_id,
        plan.template_hash().unwrap(),
        dynamic,
        ExternalGeneration::new(1).unwrap(),
        vec![],
        plan.programs().to_vec(),
    )
    .unwrap();
    plan.validate_bindings(&bindings).unwrap();
    (plan, bindings)
}

pub fn solid_program(filters: Vec<valle_draw::program::Filter>) -> DrawProgram {
    use valle_draw::{Rect, program::*};
    let mut builder = DrawProgramBuilder::new(Rect::new(0.0, 0.0, 32.0, 32.0));
    let path = builder.push_path(PathData {
        verbs: vec![
            PathVerb::MoveTo,
            PathVerb::LineTo,
            PathVerb::LineTo,
            PathVerb::LineTo,
            PathVerb::Close,
        ],
        points: vec![[4.0, 4.0], [28.0, 4.0], [28.0, 28.0], [4.0, 28.0]],
    });
    let paint = builder.push_paint(Paint::Solid(LinearColor::new(0.4, 0.2, 0.1, 1.0)));
    let node = builder.push_node(Node::Path(PathNode {
        path,
        fill_rule: FillRule::NonZero,
        fill: Some(paint),
        stroke: None,
    }));
    let mut group = Group::plain(vec![node]);
    group.filters = filters;
    let root = builder.push_node(Node::Group(group));
    builder.add_root(root);
    builder.finish().unwrap()
}

pub fn effect_fixture(
    kind: usize,
    root: bool,
    region: bool,
) -> (PreparedFrame, Vec<DynamicBinding>) {
    use valle_engine::prepare::{
        PreparedBlurAxis, PreparedEffect, PreparedEffectKernel, PreparedEffectSpace,
        PreparedUnitRect,
    };
    let (mut frame, mut dynamic) = fixture(solid_program(vec![]));
    let path = if root {
        "adjustments[0]"
    } else {
        "visual[0].effects[0]"
    };
    let mut length = |suffix: &str, value: f64| {
        let number = dynamic.len() as u32 + 1;
        dynamic.push(binding(
            number,
            &format!("{path}.{suffix}"),
            DynamicBindingKind::DeviceLength,
            DynamicValue::Scalar(value),
        ));
        id(number)
    };
    let region = region.then_some(PreparedUnitRect {
        x: 0.25,
        y: 0.25,
        width: 0.5,
        height: 0.5,
    });
    let kernel = match kind {
        0 => PreparedEffectKernel::ColorGrade {
            brightness: 0.1,
            contrast: 0.2,
            saturation: -0.3,
            temperature: 0.2,
            vignette: 0.3,
        },
        1 => PreparedEffectKernel::GaussianBlur {
            sigma_device_px: length("sigmaDevicePx", 1.0),
            region,
        },
        2 => PreparedEffectKernel::Mosaic {
            block_size_device_px: length("blockSizeDevicePx", 4.0),
            region,
        },
        3 | 4 => PreparedEffectKernel::DirectionalBlur {
            axis: if kind == 3 {
                PreparedBlurAxis::Horizontal
            } else {
                PreparedBlurAxis::Vertical
            },
            span_device_px: length("spanDevicePx", 3.0),
        },
        5 => PreparedEffectKernel::Spotlight {
            center: [0.5, 0.5],
            radius: 0.3,
            feather: 0.2,
            intensity: 0.5,
        },
        6 => PreparedEffectKernel::ExtensionColorGain {
            implementation_sha256: valle_engine::render::engine_owned_kernel_implementation_sha256(
                valle_engine::render::EXTENSION_COLOR_GAIN_ABI,
            )
            .unwrap(),
            gain: 2.0,
            past_frames: 0,
            future_frames: 0,
        },
        _ => panic!("unknown test effect"),
    };
    let space = if root {
        PreparedEffectSpace::Root
    } else {
        PreparedEffectSpace::Layer {
            transform: id(1),
            bounds: id(2),
        }
    };
    let effect = PreparedEffect {
        semantic_path: path.into(),
        space,
        kernel,
    };
    if root {
        frame.adjustments.push(effect);
    } else if let PreparedVisualItem::Layer(layer) = &mut frame.visual[0] {
        layer.effects.push(effect);
    }
    (frame, dynamic)
}

pub fn mask_fixture(
    ellipse: bool,
    feather: f64,
    invert: bool,
) -> (PreparedFrame, Vec<DynamicBinding>) {
    let (mut frame, dynamic) = fixture(solid_program(vec![]));
    let mask = serde_json::from_value(serde_json::json!({
        "shape": if ellipse { "ellipse" } else { "rect" },
        "deviceFromMask": DeviceTransform::from_affine([16.0, 0.0, 8.0, 0.0, 16.0, 8.0]),
        "featherSigmaDevicePx": feather, "invert": invert,
    }))
    .unwrap();
    if let PreparedVisualItem::Layer(layer) = &mut frame.visual[0] {
        layer.mask = Some(mask);
    }
    (frame, dynamic)
}
