use valle_engine::{
    compositor::{
        graph::{GraphCapability, GraphOrigin, GraphRoi, PassId, ResourceId},
        lower::{
            BackendCapabilities, BindingContractError, DynamicSlot, ExecutionPass, ExecutionPassId,
            ExecutionPassKind, ExternalSlot, ExternalSlotId, FramebufferFetchSemantics,
            OptimizationReport, PassInterval, PlanBindingLayout, PlanResource, PlanResourceId,
            PlanResourceKind, RenderBindings, RenderPlanTemplate, SurfaceAllocation,
            SurfaceAllocationReason, SurfaceSlot, SurfaceSlotId,
        },
    },
    frame::{RenderQuality, RenderSpec},
    prepare::{
        DynamicBinding, DynamicBindingId, DynamicBindingKind, DynamicBindings, DynamicValue,
    },
    render::RenderId,
    resource::{
        ContentDigest, Extent2d, ExternalGeneration, ExternalHandleId, ExternalPixelLayout,
        ExternalResourceDesc, LogicalTextureDesc, OutputBackground, OutputSpec,
        ResourceInterpretation, ResourceKey, TextureFormat, TextureUsage,
    },
};

fn digest(byte: char) -> ContentDigest {
    ContentDigest::from_hex(&byte.to_string().repeat(64)).unwrap()
}

fn render_id(byte: char) -> RenderId {
    RenderId::parse(&format!("sha256:{}", byte.to_string().repeat(64))).unwrap()
}

fn generation() -> ExternalGeneration {
    ExternalGeneration::new(1).unwrap()
}

fn dynamic(path: &str, kind: DynamicBindingKind) -> DynamicBindings {
    let values = vec![DynamicBinding {
        id: DynamicBindingId::try_from(1).unwrap(),
        semantic_path: path.to_owned(),
        binding_kind: kind,
        value: DynamicValue::Scalar(0.5),
    }];
    serde_json::from_value(serde_json::to_value(values).unwrap()).unwrap()
}

fn external_slot() -> ExternalSlot {
    ExternalSlot::new(
        ExternalSlotId::try_from(1).unwrap(),
        "visual[0].source",
        ResourceKey::new(
            digest('a'),
            ResourceInterpretation::FontFace { face_index: 0 },
        ),
        ExternalResourceDesc::FontBytes,
    )
    .unwrap()
}

fn layout() -> PlanBindingLayout {
    PlanBindingLayout::new(
        vec![
            DynamicSlot::new(
                DynamicBindingId::try_from(1).unwrap(),
                "visual[0].opacity",
                DynamicBindingKind::Opacity,
            )
            .unwrap(),
        ],
        vec![external_slot()],
    )
    .unwrap()
}

fn bindings() -> RenderBindings {
    RenderBindings::new(
        render_id('a'),
        digest('b'),
        dynamic("visual[0].opacity", DynamicBindingKind::Opacity),
        generation(),
        vec![ExternalHandleId::new(7).unwrap()],
        Vec::new(),
    )
    .unwrap()
}

fn capabilities() -> BackendCapabilities {
    BackendCapabilities::new(
        Extent2d::new(4096, 4096).unwrap(),
        [TextureFormat::Rgba32Float, TextureFormat::Rgba16Float],
        [
            TextureUsage::StorageWrite,
            TextureUsage::Sampled,
            TextureUsage::ColorAttachment,
            TextureUsage::StorageRead,
        ],
        [1, 4],
        [ExternalPixelLayout::Nv12, ExternalPixelLayout::Rgba8],
        [
            GraphCapability::Caption,
            GraphCapability::Clear,
            GraphCapability::OutputTransform,
            GraphCapability::ExternalImport,
        ],
        true,
        Some(FramebufferFetchSemantics::CoherentWorkingPremultiplied),
        128 * 1024 * 1024,
        512 * 1024 * 1024,
    )
    .unwrap()
}

fn render_spec() -> RenderSpec {
    RenderSpec::new(
        64,
        48,
        RenderQuality::Preview,
        OutputSpec::srgb_preview(OutputBackground::opaque_srgb([0, 0, 0])).unwrap(),
    )
    .unwrap()
}
fn template() -> RenderPlanTemplate {
    let working = PlanResourceId::try_from(1).unwrap();
    let output = PlanResourceId::try_from(2).unwrap();
    let surface = SurfaceSlotId::try_from(1).unwrap();
    let clear = ExecutionPassId::try_from(1).unwrap();
    let deliver = ExecutionPassId::try_from(2).unwrap();
    let texture = LogicalTextureDesc::production(
        Extent2d::new(64, 48).unwrap(),
        [TextureUsage::Sampled, TextureUsage::ColorAttachment],
    )
    .unwrap();
    let estimated_bytes = 64 * 48 * 8;

    let resources = vec![
        PlanResource {
            id: working,
            semantic_path: "root.working".to_owned(),
            logical_resources: vec![ResourceId::try_from(1).unwrap()],
            kind: PlanResourceKind::Surface { slot: surface },
            roi: GraphRoi::FullFrame,
            origin: GraphOrigin::Static { x: 0, y: 0 },
        },
        PlanResource {
            id: output,
            semantic_path: "output".to_owned(),
            logical_resources: vec![ResourceId::try_from(2).unwrap()],
            kind: PlanResourceKind::OutputTarget {},
            roi: GraphRoi::FullFrame,
            origin: GraphOrigin::Static { x: 0, y: 0 },
        },
    ];
    let surface_slots = vec![SurfaceSlot {
        id: surface,
        texture,
        allocations: vec![SurfaceAllocation {
            resource: working,
            interval: PassInterval {
                first: clear,
                last: deliver,
            },
            reason: SurfaceAllocationReason::WorkingComposite,
        }],
        estimated_bytes,
    }];
    let passes = vec![
        ExecutionPass {
            id: clear,
            semantic_path: "root.clear".to_owned(),
            logical_passes: vec![PassId::try_from(1).unwrap()],
            kind: ExecutionPassKind::ClearRegion {
                output: working,
                working_linear_rec2020_premul: [0.0, 0.0, 0.0, 1.0],
            },
        },
        ExecutionPass {
            id: deliver,
            semantic_path: "output.transform".to_owned(),
            logical_passes: vec![PassId::try_from(2).unwrap()],
            kind: ExecutionPassKind::CopyConvert {
                input: working,
                output,
                operation: valle_engine::compositor::lower::CopyOperation::OutputTransform {
                    spec: render_spec().output(),
                },
            },
        },
    ];
    let optimization = OptimizationReport::identity(&resources, &passes).unwrap();

    RenderPlanTemplate::new(
        render_id('d'),
        digest('e'),
        render_spec(),
        2,
        2,
        vec![GraphCapability::Clear, GraphCapability::OutputTransform],
        Vec::new(),
        PlanBindingLayout::new(Vec::new(), Vec::new()).unwrap(),
        resources,
        surface_slots,
        passes,
        optimization,
        output,
        estimated_bytes,
    )
    .unwrap()
}

#[test]
fn capability_fingerprint_is_backend_name_free_and_input_order_independent() {
    let first = capabilities();
    let second = BackendCapabilities::new(
        Extent2d::new(4096, 4096).unwrap(),
        [TextureFormat::Rgba16Float, TextureFormat::Rgba32Float],
        [
            TextureUsage::Sampled,
            TextureUsage::StorageRead,
            TextureUsage::StorageWrite,
            TextureUsage::ColorAttachment,
        ],
        [4, 1],
        [ExternalPixelLayout::Rgba8, ExternalPixelLayout::Nv12],
        [
            GraphCapability::Clear,
            GraphCapability::ExternalImport,
            GraphCapability::Caption,
            GraphCapability::OutputTransform,
        ],
        true,
        Some(FramebufferFetchSemantics::CoherentWorkingPremultiplied),
        128 * 1024 * 1024,
        512 * 1024 * 1024,
    )
    .unwrap();
    assert_eq!(first, second);
    assert_eq!(first.fingerprint().unwrap(), second.fingerprint().unwrap());
}

#[test]
fn capability_serde_projection_has_no_process_local_version_and_rejects_invalid_values() {
    let value = serde_json::to_value(capabilities()).unwrap();
    assert!(value.get("header").is_none());
    let restored: BackendCapabilities = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(restored, capabilities());

    let mut reordered = value.clone();
    reordered["formats"].as_array_mut().unwrap().reverse();
    assert!(serde_json::from_value::<BackendCapabilities>(reordered).is_err());

    let mut duplicate = value.clone();
    let first = duplicate["graphCapabilities"][0].clone();
    duplicate["graphCapabilities"]
        .as_array_mut()
        .unwrap()
        .push(first);
    assert!(serde_json::from_value::<BackendCapabilities>(duplicate).is_err());

    let mut sample_count = value.clone();
    sample_count["sampleCounts"] = serde_json::json!([1, 3]);
    assert!(serde_json::from_value::<BackendCapabilities>(sample_count).is_err());

    let mut external_layouts = value.clone();
    external_layouts["externalPixelLayouts"] = serde_json::json!([]);
    assert!(serde_json::from_value::<BackendCapabilities>(external_layouts).is_err());

    let mut budget = value.clone();
    budget["maxSurfaceBytes"] = serde_json::json!(1024);
    budget["maxFrameBytes"] = serde_json::json!(512);
    assert!(serde_json::from_value::<BackendCapabilities>(budget).is_err());

    let mut unknown = value;
    unknown["backendName"] = serde_json::json!("skia");
    assert!(serde_json::from_value::<BackendCapabilities>(unknown).is_err());
}

#[test]
fn binding_layout_accepts_only_an_exact_per_frame_packet() {
    let layout = layout();
    let bindings = bindings();
    layout.validate_bindings(&digest('b'), &bindings).unwrap();

    let layout_wire = serde_json::to_value(&layout).unwrap();
    let restored_layout: PlanBindingLayout = serde_json::from_value(layout_wire).unwrap();
    assert_eq!(restored_layout, layout);
    let bindings_wire = serde_json::to_value(&bindings).unwrap();
    let restored_bindings: RenderBindings = serde_json::from_value(bindings_wire).unwrap();
    assert_eq!(restored_bindings, bindings);

    assert_eq!(
        layout.validate_bindings(&digest('c'), &bindings),
        Err(BindingContractError::TemplateHashMismatch)
    );

    let wrong_dynamic = RenderBindings::new(
        render_id('a'),
        digest('b'),
        dynamic(
            "visual[0].transitionProgress",
            DynamicBindingKind::TransitionProgress,
        ),
        generation(),
        vec![ExternalHandleId::new(7).unwrap()],
        Vec::new(),
    )
    .unwrap();
    assert!(matches!(
        layout.validate_bindings(&digest('b'), &wrong_dynamic),
        Err(BindingContractError::DynamicLayoutMismatch { .. })
    ));

    let missing_external = RenderBindings::new(
        render_id('a'),
        digest('b'),
        dynamic("visual[0].opacity", DynamicBindingKind::Opacity),
        generation(),
        Vec::new(),
        Vec::new(),
    )
    .unwrap();
    assert_eq!(
        layout.validate_bindings(&digest('b'), &missing_external),
        Err(BindingContractError::ExternalCountMismatch {
            expected: 1,
            actual: 0,
        })
    );
}

#[test]
fn binding_wire_rejects_unknown_fields_and_invalid_layouts() {
    assert!(
        serde_json::to_value(bindings())
            .unwrap()
            .get("header")
            .is_none()
    );
    let mut missing_generation = serde_json::to_value(bindings()).unwrap();
    missing_generation
        .as_object_mut()
        .unwrap()
        .remove("externalGeneration");
    assert!(serde_json::from_value::<RenderBindings>(missing_generation).is_err());

    let mut zero_generation = serde_json::to_value(bindings()).unwrap();
    zero_generation["externalGeneration"] = serde_json::json!(0);
    assert!(serde_json::from_value::<RenderBindings>(zero_generation).is_err());

    let mut unknown = serde_json::to_value(bindings()).unwrap();
    unknown["backendObject"] = serde_json::json!("texture");
    assert!(serde_json::from_value::<RenderBindings>(unknown).is_err());

    let mut gap = serde_json::to_value(layout()).unwrap();
    gap["externalSlots"][0]["id"] = serde_json::json!(2);
    assert!(serde_json::from_value::<PlanBindingLayout>(gap).is_err());

    let mut empty_path = serde_json::to_value(layout()).unwrap();
    empty_path["dynamicSlots"][0]["semanticPath"] = serde_json::json!("");
    assert!(serde_json::from_value::<PlanBindingLayout>(empty_path).is_err());

    let mut unknown_slot = serde_json::to_value(layout()).unwrap();
    unknown_slot["externalSlots"][0]["nativeTexture"] = serde_json::json!(1);
    assert!(serde_json::from_value::<PlanBindingLayout>(unknown_slot).is_err());

    let mut unknown_interpretation = serde_json::to_value(layout()).unwrap();
    unknown_interpretation["externalSlots"][0]["key"]["interpretation"]["nativeObject"] =
        serde_json::json!(1);
    assert!(serde_json::from_value::<PlanBindingLayout>(unknown_interpretation).is_err());

    let mut unknown_descriptor = serde_json::to_value(layout()).unwrap();
    unknown_descriptor["externalSlots"][0]["expected"]["nativeTexture"] = serde_json::json!(1);
    assert!(serde_json::from_value::<PlanBindingLayout>(unknown_descriptor).is_err());

    let mut unknown_dynamic_value = serde_json::to_value(bindings()).unwrap();
    unknown_dynamic_value["dynamic"][0]["value"]["nativeUniform"] = serde_json::json!(1);
    assert!(serde_json::from_value::<RenderBindings>(unknown_dynamic_value).is_err());
}

#[test]
fn external_slot_rejects_key_descriptor_type_mismatch() {
    let error = ExternalSlot::new(
        ExternalSlotId::try_from(1).unwrap(),
        "visual[0].source",
        ResourceKey::new(
            digest('a'),
            ResourceInterpretation::FontFace { face_index: 0 },
        ),
        ExternalResourceDesc::RuntimeShader,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        BindingContractError::ExternalTypeMismatch { .. }
    ));
}

#[test]
fn plan_template_hashes_strict_executable_structure_and_binds_atomically() {
    let template = template();
    template.validate().unwrap();
    let bytes = template.canonical_bytes().unwrap();
    let hash = template.template_hash().unwrap();

    let restored: RenderPlanTemplate =
        serde_json::from_value(serde_json::to_value(&template).unwrap()).unwrap();
    assert_eq!(restored, template);
    assert_eq!(restored.canonical_bytes().unwrap(), bytes);
    assert_eq!(restored.template_hash().unwrap(), hash);
    let packed = template.packed_bytes().unwrap();
    assert!(!packed.starts_with(b"{"));
    let packed_restored = RenderPlanTemplate::from_packed(&packed).unwrap();
    assert_eq!(packed_restored, template);
    assert_eq!(packed_restored.packed_bytes().unwrap(), packed);
    assert_eq!(restored.estimated_peak_surface_bytes(), 64 * 48 * 8);

    let bindings = RenderBindings::new(
        *template.render_id(),
        hash,
        DynamicBindings::default(),
        generation(),
        Vec::new(),
        Vec::new(),
    )
    .unwrap();
    template.validate_bindings(&bindings).unwrap();
    let packed_bindings = bindings.packed_bytes().unwrap();
    assert_eq!(
        RenderBindings::from_packed(&packed_bindings).unwrap(),
        bindings
    );
}

#[test]
fn plan_wire_rejects_forged_order_liveness_capability_and_shape() {
    let value = serde_json::to_value(template()).unwrap();
    assert!(value.get("header").is_none());

    let mut unknown = value.clone();
    unknown["nativeCommandBuffer"] = serde_json::json!(1);
    assert!(serde_json::from_value::<RenderPlanTemplate>(unknown).is_err());

    let mut nested_unknown = value.clone();
    nested_unknown["resources"][1]["kind"]["nativeTarget"] = serde_json::json!(1);
    assert!(serde_json::from_value::<RenderPlanTemplate>(nested_unknown).is_err());

    let mut forged_count = value.clone();
    forged_count["logicalResourceCount"] = serde_json::json!(u32::MAX);
    assert!(serde_json::from_value::<RenderPlanTemplate>(forged_count).is_err());

    let mut resource_gap = value.clone();
    resource_gap["resources"][0]["id"] = serde_json::json!(2);
    assert!(serde_json::from_value::<RenderPlanTemplate>(resource_gap).is_err());

    let mut pass_order = value.clone();
    pass_order["passes"].as_array_mut().unwrap().reverse();
    assert!(serde_json::from_value::<RenderPlanTemplate>(pass_order).is_err());

    let mut liveness = value.clone();
    liveness["surfaceSlots"][0]["allocations"][0]["interval"]["last"] = serde_json::json!(1);
    assert!(serde_json::from_value::<RenderPlanTemplate>(liveness).is_err());

    let mut bytes = value.clone();
    bytes["surfaceSlots"][0]["estimatedBytes"] = serde_json::json!(1);
    assert!(serde_json::from_value::<RenderPlanTemplate>(bytes).is_err());

    let mut capability = value;
    capability["requiredCapabilities"]
        .as_array_mut()
        .unwrap()
        .pop();
    assert!(serde_json::from_value::<RenderPlanTemplate>(capability).is_err());
}

#[test]
fn template_structure_hash_commits_the_capability_fingerprint() {
    let first = template();
    let mut wire = serde_json::to_value(&first).unwrap();
    wire["capabilityFingerprint"] = serde_json::to_value(digest('f')).unwrap();
    assert!(serde_json::from_value::<RenderPlanTemplate>(wire).is_err());
}
