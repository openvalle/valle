use std::collections::BTreeMap;

use thiserror::Error;
use valle_draw::program::DrawProgram;

use crate::{
    compositor::graph::{
        GraphCapability, GraphOrigin, GraphResourceKind, GraphRoi, GraphValidationError, LayerRole,
        LogicalPassKind, PassId, RenderGraph, ResourceId,
    },
    prepare::{
        DynamicBindingId, DynamicBindingKind, DynamicBindings, PreparedExternalBackdrop, ProgramId,
        RequestError,
    },
    resource::{
        Extent2d, ExternalHandleId, ExternalPixelLayout, ExternalResourceDesc, LogicalTextureDesc,
        TextureFormat, TextureUsage,
    },
};

use super::{
    BackendCapabilities, BindingContractError, CapabilityContractError, CompositeMode,
    CopyOperation, DynamicSlot, ExecutionPass, ExecutionPassId, ExecutionPassKind, ExternalSlot,
    ExternalSlotId, KernelInvocation, PlanBindingLayout, PlanEffect, PlanFontBinding, PlanIdError,
    PlanProgram, PlanProgramLayout, PlanProgramResources, PlanResource, PlanResourceId,
    PlanResourceKind, PlanSourcePipeline, PlanStructureBinding, PlanTextureBinding,
    PlanValidationError, ProgramPlan, ProgramPlanError, ProgramSchedule, ProgramScheduleError,
    RenderPlanTemplate, ResolveReason, ResourceAliasReason, SurfaceAllocation,
    SurfaceAllocationReason, SurfaceSlot, SurfaceSlotId,
    plan::{peak_surface_bytes, resource_interval, texture_bytes},
};

/// Deterministically lowers an admitted logical graph into a backend-capability-specific plan.
///
/// Dynamic values are intentionally not copied into the template: only their stable id/path/kind
/// layout is retained. The values themselves remain a per-frame [`super::RenderBindings`] concern.
pub fn lower_render_graph(
    graph: &RenderGraph,
    dynamic: &DynamicBindings,
    capabilities: &BackendCapabilities,
) -> Result<RenderPlanTemplate, LowerError> {
    lower_render_graph_impl(graph, dynamic, capabilities, true)
}

pub(crate) fn lower_constructed_render_graph(
    graph: &RenderGraph,
    dynamic: &DynamicBindings,
    capabilities: &BackendCapabilities,
) -> Result<RenderPlanTemplate, LowerError> {
    lower_render_graph_impl(graph, dynamic, capabilities, false)
}

fn lower_render_graph_impl(
    graph: &RenderGraph,
    dynamic: &DynamicBindings,
    capabilities: &BackendCapabilities,
    verify_packed_programs: bool,
) -> Result<RenderPlanTemplate, LowerError> {
    if verify_packed_programs {
        graph.validate()?;
    }
    dynamic.validate()?;
    validate_dynamic_projection(graph, dynamic)?;
    capabilities.validate()?;
    admit_capabilities(graph, capabilities)?;

    let bindings = lower_binding_layout(graph, dynamic)?;
    let backdrop_choices = choose_backdrop_resources(graph, capabilities);
    let mut passes = lower_passes(graph, &backdrop_choices)?;
    let programs = lower_programs(graph, &bindings.external_by_handle, !verify_packed_programs)?;
    let (mut resources, mut surface_drafts) =
        lower_resources(graph, &bindings.external_by_resource, &backdrop_choices)?;
    let optimization =
        super::optimizer::optimize(&mut resources, &mut surface_drafts, &mut passes)?;
    let (surface_slots, surface_bindings) =
        lower_surface_slots(surface_drafts, &resources, &passes, capabilities)?;
    for resource in &mut resources {
        if matches!(resource.kind, PlanResourceKind::Surface { .. }) {
            let slot = surface_bindings.get(&resource.id).copied().ok_or(
                LowerError::MissingSurfaceSlot {
                    resource: resource.id,
                },
            )?;
            resource.kind = PlanResourceKind::Surface { slot };
        }
    }
    let estimated_peak_surface_bytes = peak_surface_bytes(&surface_slots, passes.len())?;
    if estimated_peak_surface_bytes > capabilities.max_frame_bytes() {
        return Err(LowerError::FrameBudgetExceeded {
            required_bytes: estimated_peak_surface_bytes,
            max_bytes: capabilities.max_frame_bytes(),
        });
    }

    let capability_fingerprint = capabilities.fingerprint()?;
    let logical_pass_count = u32::try_from(graph.passes.len())
        .map_err(|_| LowerError::CountBudgetExceeded { kind: "pass" })?;
    let logical_resource_count = u32::try_from(graph.resources.len())
        .map_err(|_| LowerError::CountBudgetExceeded { kind: "resource" })?;
    let output = plan_resource_id(graph.output)?;

    let build = if verify_packed_programs {
        RenderPlanTemplate::new
    } else {
        RenderPlanTemplate::new_constructed
    };
    build(
        graph.render_id,
        capability_fingerprint,
        graph.render_spec,
        logical_pass_count,
        logical_resource_count,
        graph.capabilities.clone(),
        programs,
        bindings.layout,
        resources,
        surface_slots,
        passes,
        optimization,
        output,
        estimated_peak_surface_bytes,
    )
    .map_err(LowerError::from)
}

fn admit_capabilities(
    graph: &RenderGraph,
    capabilities: &BackendCapabilities,
) -> Result<(), LowerError> {
    if let Some(capability) = graph
        .capabilities
        .iter()
        .copied()
        .find(|required| !capabilities.graph_capabilities().contains(required))
    {
        return Err(LowerError::MissingGraphCapability { capability });
    }

    let frame_extent = Extent2d::new(graph.render_spec.width(), graph.render_spec.height())
        .expect("validated RenderSpec has a non-empty extent");
    require_extent(frame_extent, capabilities.max_extent(), "renderSpec")?;

    for resource in &graph.resources {
        if let Some(texture) = &resource.texture {
            admit_texture(resource.id, texture, capabilities)?;
        }
        if let GraphResourceKind::ExternalResource {
            expected:
                ExternalResourceDesc::VisualFrame {
                    extent,
                    pixel_layout,
                },
            ..
        } = resource.kind
        {
            require_extent(extent, capabilities.max_extent(), &resource.semantic_path)?;
            if !capabilities
                .external_pixel_layouts()
                .contains(&pixel_layout)
            {
                return Err(LowerError::UnsupportedExternalPixelLayout {
                    resource: resource.id,
                    pixel_layout,
                });
            }
        }
    }
    Ok(())
}

fn admit_texture(
    resource: ResourceId,
    texture: &LogicalTextureDesc,
    capabilities: &BackendCapabilities,
) -> Result<(), LowerError> {
    require_extent(
        texture.extent,
        capabilities.max_extent(),
        "graph.resources.texture",
    )?;
    if !capabilities.formats().contains(&texture.format) {
        return Err(LowerError::UnsupportedTextureFormat {
            resource,
            format: texture.format,
        });
    }
    if let Some(usage) = texture
        .usages()
        .iter()
        .copied()
        .find(|usage| !capabilities.texture_usages().contains(usage))
    {
        return Err(LowerError::UnsupportedTextureUsage { resource, usage });
    }
    if !capabilities
        .sample_counts()
        .contains(&texture.sample_count())
    {
        return Err(LowerError::UnsupportedSampleCount {
            resource,
            sample_count: texture.sample_count(),
        });
    }
    let required_bytes = texture_bytes(texture)?;
    if required_bytes > capabilities.max_surface_bytes() {
        return Err(LowerError::SurfaceBudgetExceeded {
            resource,
            required_bytes,
            max_bytes: capabilities.max_surface_bytes(),
        });
    }
    Ok(())
}

fn require_extent(actual: Extent2d, maximum: Extent2d, path: &str) -> Result<(), LowerError> {
    if actual.width() > maximum.width() || actual.height() > maximum.height() {
        return Err(LowerError::ExtentExceeded {
            path: path.to_owned(),
            actual,
            maximum,
        });
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DynamicExpectation {
    semantic_path: String,
    binding_kind: DynamicBindingKind,
}

fn validate_dynamic_projection(
    graph: &RenderGraph,
    dynamic: &DynamicBindings,
) -> Result<(), LowerError> {
    let mut expected = BTreeMap::new();
    for pass in &graph.passes {
        match &pass.kind {
            LogicalPassKind::Import {
                output, transform, ..
            }
            | LogicalPassKind::Draw {
                output, transform, ..
            } => {
                let suffix = if matches!(pass.kind, LogicalPassKind::Import { .. }) {
                    ".import"
                } else {
                    ".draw"
                };
                let owner = pass
                    .semantic_path
                    .strip_suffix(suffix)
                    .ok_or(LowerError::InvalidSemanticPath { pass: pass.id })?;
                insert_dynamic(
                    &mut expected,
                    *transform,
                    format!("{owner}.transform"),
                    DynamicBindingKind::DeviceTransform,
                )?;
                insert_dynamic(
                    &mut expected,
                    resource_bounds_binding(graph, *output)?,
                    format!("{owner}.bounds"),
                    DynamicBindingKind::Bounds,
                )?;
                if let LogicalPassKind::Draw { program, .. } = pass.kind {
                    for (index, destination) in graph.programs[program.get() as usize - 1]
                        .destination_uses
                        .iter()
                        .enumerate()
                    {
                        let path = format!("{owner}.destination[{index}]");
                        insert_dynamic(
                            &mut expected,
                            destination.sample_bounds,
                            format!("{path}.sampleBounds"),
                            DynamicBindingKind::BackdropSampleBounds,
                        )?;
                        insert_dynamic(
                            &mut expected,
                            destination.output_bounds,
                            format!("{path}.outputBounds"),
                            DynamicBindingKind::BackdropOutputBounds,
                        )?;
                    }
                }
                if let LogicalPassKind::Import {
                    ref placement,
                    ref source_pipeline,
                    ..
                } = pass.kind
                {
                    if let Some(PreparedExternalBackdrop::Blur {
                        sigma_device_px, ..
                    }) = placement.backdrop
                    {
                        insert_dynamic(
                            &mut expected,
                            sigma_device_px,
                            format!("{owner}.sourcePlacement.backdrop.sigmaDevicePx"),
                            DynamicBindingKind::DeviceLength,
                        )?;
                    }
                    if let Some(effect) = &source_pipeline.chroma_key {
                        for (id, suffix) in effect.kernel.device_lengths() {
                            insert_dynamic(
                                &mut expected,
                                id,
                                format!("{}.{}", effect.semantic_path, suffix),
                                DynamicBindingKind::DeviceLength,
                            )?;
                        }
                    }
                }
            }
            LogicalPassKind::BackdropRead {
                sample_bounds,
                output_bounds,
                ..
            } => {
                insert_dynamic(
                    &mut expected,
                    *sample_bounds,
                    format!("{}.sampleBounds", pass.semantic_path),
                    DynamicBindingKind::BackdropSampleBounds,
                )?;
                insert_dynamic(
                    &mut expected,
                    *output_bounds,
                    format!("{}.outputBounds", pass.semantic_path),
                    DynamicBindingKind::BackdropOutputBounds,
                )?;
            }
            LogicalPassKind::CompositeLayer { opacity, .. }
            | LogicalPassKind::Blend { opacity, .. } => {
                let owner = pass
                    .semantic_path
                    .strip_suffix(".composite")
                    .ok_or(LowerError::InvalidSemanticPath { pass: pass.id })?;
                insert_dynamic(
                    &mut expected,
                    *opacity,
                    format!("{owner}.opacity"),
                    DynamicBindingKind::Opacity,
                )?;
            }
            LogicalPassKind::Transition {
                progress,
                from_opacity,
                to_opacity,
                ..
            } => {
                let owner = pass
                    .semantic_path
                    .strip_suffix(".transition")
                    .ok_or(LowerError::InvalidSemanticPath { pass: pass.id })?;
                insert_dynamic(
                    &mut expected,
                    *progress,
                    format!("{owner}.progress"),
                    DynamicBindingKind::TransitionProgress,
                )?;
                insert_dynamic(
                    &mut expected,
                    *from_opacity,
                    format!("{owner}.from.opacity"),
                    DynamicBindingKind::Opacity,
                )?;
                insert_dynamic(
                    &mut expected,
                    *to_opacity,
                    format!("{owner}.to.opacity"),
                    DynamicBindingKind::Opacity,
                )?;
            }
            LogicalPassKind::Caption {
                transform,
                bounds,
                opacity,
                ..
            } => {
                insert_dynamic(
                    &mut expected,
                    *transform,
                    format!("{}.transform", pass.semantic_path),
                    DynamicBindingKind::DeviceTransform,
                )?;
                insert_dynamic(
                    &mut expected,
                    *bounds,
                    format!("{}.bounds", pass.semantic_path),
                    DynamicBindingKind::Bounds,
                )?;
                insert_dynamic(
                    &mut expected,
                    *opacity,
                    format!("{}.opacity", pass.semantic_path),
                    DynamicBindingKind::Opacity,
                )?;
            }
            LogicalPassKind::Filter { effect, .. }
            | LogicalPassKind::AdjustmentEffect { effect, .. } => {
                if let crate::prepare::PreparedEffectSpace::Layer { transform, bounds } =
                    effect.space
                {
                    let marker = effect
                        .semantic_path
                        .rfind(".effects[")
                        .ok_or(LowerError::InvalidSemanticPath { pass: pass.id })?;
                    let owner = &effect.semantic_path[..marker];
                    insert_dynamic(
                        &mut expected,
                        transform,
                        format!("{owner}.transform"),
                        DynamicBindingKind::DeviceTransform,
                    )?;
                    insert_dynamic(
                        &mut expected,
                        bounds,
                        format!("{owner}.bounds"),
                        DynamicBindingKind::Bounds,
                    )?;
                }
                for (id, suffix) in effect.kernel.device_lengths() {
                    insert_dynamic(
                        &mut expected,
                        id,
                        format!("{}.{}", effect.semantic_path, suffix),
                        DynamicBindingKind::DeviceLength,
                    )?;
                }
            }
            _ => {}
        }
    }

    if expected.len() != dynamic.values().len() {
        return Err(LowerError::DynamicCountMismatch {
            expected: expected.len(),
            actual: dynamic.values().len(),
        });
    }
    for binding in dynamic.values() {
        let Some(contract) = expected.get(&binding.id) else {
            return Err(LowerError::UnexpectedDynamicBinding { id: binding.id });
        };
        if binding.semantic_path != contract.semantic_path
            || binding.binding_kind != contract.binding_kind
        {
            return Err(LowerError::DynamicLayoutMismatch {
                id: binding.id,
                expected_path: contract.semantic_path.clone(),
                expected_kind: contract.binding_kind,
            });
        }
    }
    Ok(())
}

fn insert_dynamic(
    expected: &mut BTreeMap<DynamicBindingId, DynamicExpectation>,
    id: DynamicBindingId,
    semantic_path: String,
    binding_kind: DynamicBindingKind,
) -> Result<(), LowerError> {
    let contract = DynamicExpectation {
        semantic_path,
        binding_kind,
    };
    if let Some(existing) = expected.insert(id, contract.clone())
        && existing != contract
    {
        return Err(LowerError::ConflictingDynamicBinding { id });
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BackdropPhysicalChoice {
    Direct {
        source: ResourceId,
        reason: ResourceAliasReason,
    },
    Resolve {
        source: ResourceId,
        reason: ResolveReason,
    },
}

fn choose_backdrop_resources(
    graph: &RenderGraph,
    capabilities: &BackendCapabilities,
) -> BTreeMap<ResourceId, BackdropPhysicalChoice> {
    graph
        .passes
        .iter()
        .filter_map(|pass| {
            let LogicalPassKind::BackdropRead { token, output, .. } = &pass.kind else {
                return None;
            };
            let choice = if capabilities.sampleable_render_target() {
                BackdropPhysicalChoice::Direct {
                    source: token.composite,
                    reason: ResourceAliasReason::DirectSampleableImmutableInput,
                }
            } else {
                BackdropPhysicalChoice::Resolve {
                    source: token.composite,
                    reason: ResolveReason::NonSampleableRenderTarget,
                }
            };
            Some((*output, choice))
        })
        .collect()
}

fn lower_passes(
    graph: &RenderGraph,
    backdrop_choices: &BTreeMap<ResourceId, BackdropPhysicalChoice>,
) -> Result<Vec<ExecutionPass>, LowerError> {
    let order = graph.topological_order()?;
    let mut passes = Vec::with_capacity(order.len());
    for logical_id in order {
        let logical = &graph.passes[logical_id.index()];
        let kind = match &logical.kind {
            LogicalPassKind::ClearComposite {
                output,
                working_linear_rec2020_premul,
            } => ExecutionPassKind::ClearRegion {
                output: plan_resource_id(*output)?,
                working_linear_rec2020_premul: *working_linear_rec2020_premul,
            },
            LogicalPassKind::Import {
                external,
                source_pipeline,
                output,
                placement,
                transform,
            } => ExecutionPassKind::ImportRegion {
                external: plan_resource_id(*external)?,
                source_pipeline: PlanSourcePipeline {
                    chroma_key: source_pipeline.chroma_key.as_ref().map(lower_effect),
                },
                output: plan_resource_id(*output)?,
                placement: *placement,
                transform: *transform,
                bounds: resource_bounds_binding(graph, *output)?,
            },
            LogicalPassKind::Draw {
                program,
                external_inputs,
                destination_inputs,
                output,
                transform,
                bounds_reason,
            } => ExecutionPassKind::RasterProgram {
                program: *program,
                external_inputs: plan_resource_ids(external_inputs)?,
                destination_inputs: plan_resource_ids(destination_inputs)?,
                output: plan_resource_id(*output)?,
                transform: *transform,
                bounds: resource_bounds_binding(graph, *output)?,
                bounds_reason: *bounds_reason,
            },
            LogicalPassKind::Group { input, output } => ExecutionPassKind::DispatchKernel {
                invocation: KernelInvocation::Group {
                    input: plan_resource_id(*input)?,
                    output: plan_resource_id(*output)?,
                },
            },
            LogicalPassKind::BackdropRead {
                token,
                output,
                sample_bounds,
                output_bounds,
            } => match backdrop_choice(backdrop_choices, *output)? {
                BackdropPhysicalChoice::Direct { source, reason } => {
                    if source != token.composite {
                        return Err(LowerError::BackdropSourceMismatch { resource: *output });
                    }
                    ExecutionPassKind::BindBackdropView {
                        input: plan_resource_id(source)?,
                        output: plan_resource_id(*output)?,
                        reason,
                    }
                }
                BackdropPhysicalChoice::Resolve { source, reason } => {
                    if source != token.composite {
                        return Err(LowerError::BackdropSourceMismatch { resource: *output });
                    }
                    ExecutionPassKind::ResolveRegion {
                        input: plan_resource_id(source)?,
                        output: plan_resource_id(*output)?,
                        sample_bounds: *sample_bounds,
                        output_bounds: *output_bounds,
                        reason,
                    }
                }
            },
            LogicalPassKind::Filter {
                input,
                output,
                effect,
            } => ExecutionPassKind::DispatchKernel {
                invocation: KernelInvocation::Filter {
                    input: plan_resource_id(*input)?,
                    output: plan_resource_id(*output)?,
                    effect: lower_effect(effect),
                },
            },
            LogicalPassKind::Mask {
                input,
                output,
                mask,
            } => ExecutionPassKind::DispatchKernel {
                invocation: KernelInvocation::Mask {
                    input: plan_resource_id(*input)?,
                    output: plan_resource_id(*output)?,
                    mask: *mask,
                },
            },
            LogicalPassKind::CompositeLayer {
                backdrop,
                layer,
                output,
                opacity,
            } => ExecutionPassKind::CompositeRegion {
                backdrop: plan_resource_id(*backdrop)?,
                layer: plan_resource_id(*layer)?,
                output: plan_resource_id(*output)?,
                opacity: *opacity,
                mode: CompositeMode::SourceOver {},
            },
            LogicalPassKind::Blend {
                backdrop,
                layer,
                output,
                mode,
                opacity,
            } => ExecutionPassKind::CompositeRegion {
                backdrop: plan_resource_id(*backdrop)?,
                layer: plan_resource_id(*layer)?,
                output: plan_resource_id(*output)?,
                opacity: *opacity,
                mode: CompositeMode::Blend { mode: *mode },
            },
            LogicalPassKind::Transition {
                backdrop,
                from,
                to,
                output,
                kernel,
                progress,
                from_opacity,
                to_opacity,
            } => ExecutionPassKind::DispatchKernel {
                invocation: KernelInvocation::Transition {
                    backdrop: plan_resource_id(*backdrop)?,
                    from: plan_resource_id(*from)?,
                    to: plan_resource_id(*to)?,
                    output: plan_resource_id(*output)?,
                    kernel: *kernel,
                    progress: *progress,
                    from_opacity: *from_opacity,
                    to_opacity: *to_opacity,
                },
            },
            LogicalPassKind::AdjustmentEffect {
                input,
                output,
                effect,
            } => ExecutionPassKind::DispatchKernel {
                invocation: KernelInvocation::AdjustmentEffect {
                    input: plan_resource_id(*input)?,
                    output: plan_resource_id(*output)?,
                    effect: lower_effect(effect),
                },
            },
            LogicalPassKind::Caption {
                backdrop,
                program,
                external_inputs,
                output,
                transform,
                bounds,
                bounds_reason,
                opacity,
            } => ExecutionPassKind::RasterCaption {
                program: *program,
                external_inputs: plan_resource_ids(external_inputs)?,
                destination: plan_resource_id(*backdrop)?,
                output: plan_resource_id(*output)?,
                transform: *transform,
                bounds: *bounds,
                bounds_reason: *bounds_reason,
                opacity: *opacity,
            },
            LogicalPassKind::OutputTransform {
                input,
                output,
                spec,
            } => ExecutionPassKind::CopyConvert {
                input: plan_resource_id(*input)?,
                output: plan_resource_id(*output)?,
                operation: CopyOperation::OutputTransform { spec: *spec },
            },
        };
        passes.push(ExecutionPass {
            id: ExecutionPassId::from_index(passes.len())?,
            semantic_path: logical.semantic_path.clone(),
            logical_passes: vec![logical.id],
            kind,
        });
    }
    Ok(passes)
}

fn backdrop_choice(
    choices: &BTreeMap<ResourceId, BackdropPhysicalChoice>,
    resource: ResourceId,
) -> Result<BackdropPhysicalChoice, LowerError> {
    choices
        .get(&resource)
        .copied()
        .ok_or(LowerError::MissingBackdropChoice { resource })
}

fn resource_bounds_binding(
    graph: &RenderGraph,
    resource: ResourceId,
) -> Result<DynamicBindingId, LowerError> {
    let resource = &graph.resources[resource.index()];
    match (&resource.roi, &resource.origin) {
        (GraphRoi::Dynamic { binding }, GraphOrigin::Dynamic { bounds }) if binding == bounds => {
            Ok(*binding)
        }
        _ => Err(LowerError::MissingDynamicBounds {
            resource: resource.id,
        }),
    }
}

fn plan_resource_ids(ids: &[ResourceId]) -> Result<Vec<PlanResourceId>, LowerError> {
    ids.iter().copied().map(plan_resource_id).collect()
}

struct LoweredBindings {
    layout: PlanBindingLayout,
    external_by_resource: BTreeMap<ResourceId, ExternalSlotId>,
    external_by_handle: BTreeMap<ExternalHandleId, ExternalSlotId>,
}

fn lower_binding_layout(
    graph: &RenderGraph,
    dynamic: &DynamicBindings,
) -> Result<LoweredBindings, LowerError> {
    let dynamic_slots = dynamic
        .values()
        .iter()
        .map(|binding| {
            DynamicSlot::new(
                binding.id,
                binding.semantic_path.clone(),
                binding.binding_kind,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut external_slots = Vec::new();
    let mut external_by_resource = BTreeMap::new();
    let mut external_by_handle = BTreeMap::new();
    for resource in &graph.resources {
        if let GraphResourceKind::ExternalResource {
            handle,
            key,
            expected,
        } = &resource.kind
        {
            let slot = external_slot_id(external_slots.len())?;
            external_slots.push(ExternalSlot::new(
                slot,
                resource.semantic_path.clone(),
                key.clone(),
                expected.clone(),
            )?);
            external_by_resource.insert(resource.id, slot);
            external_by_handle.insert(*handle, slot);
        }
    }
    let layout = PlanBindingLayout::new(dynamic_slots, external_slots)?;
    Ok(LoweredBindings {
        layout,
        external_by_resource,
        external_by_handle,
    })
}

fn lower_programs(
    graph: &RenderGraph,
    external_slots: &BTreeMap<ExternalHandleId, ExternalSlotId>,
    constructed: bool,
) -> Result<Vec<PlanProgram>, LowerError> {
    lower_programs_impl(graph, external_slots, None, constructed)
}

fn lower_programs_impl(
    graph: &RenderGraph,
    external_slots: &BTreeMap<ExternalHandleId, ExternalSlotId>,
    layouts: Option<&[PlanProgramLayout]>,
    constructed: bool,
) -> Result<Vec<PlanProgram>, LowerError> {
    if let Some(layouts) = layouts
        && layouts.len() != graph.programs.len()
    {
        return Err(LowerError::CountBudgetExceeded {
            kind: "program layout",
        });
    }
    graph
        .programs
        .iter()
        .enumerate()
        .map(|(index, program)| {
            let decoded_storage;
            let decoded = if let Some(decoded) = program.runtime_program() {
                decoded
            } else {
                decoded_storage = DrawProgram::from_packed(&program.packed).map_err(|error| {
                    LowerError::InvalidProgramPlan {
                        program: program.id,
                        reason: error.to_string(),
                    }
                })?;
                &decoded_storage
            };
            let local_plan = ProgramPlan::derive(&decoded)?;
            let local_schedule = ProgramSchedule::derive(&local_plan)?;
            let resources = PlanProgramResources {
                textures: program
                    .resources
                    .textures
                    .iter()
                    .map(|binding| {
                        Ok(PlanTextureBinding {
                            key: binding.key.clone(),
                            slot: slot_for_handle(external_slots, binding.handle)?,
                        })
                    })
                    .collect::<Result<Vec<_>, LowerError>>()?,
                fonts: program
                    .resources
                    .fonts
                    .iter()
                    .map(|binding| {
                        Ok(PlanFontBinding {
                            face_hash: binding.face_hash,
                            face_index: binding.face_index,
                            slot: slot_for_handle(external_slots, binding.handle)?,
                        })
                    })
                    .collect::<Result<Vec<_>, LowerError>>()?,
                runtime_shaders: lower_structure_bindings(
                    &program.resources.runtime_shaders,
                    external_slots,
                )?,
                scenes: lower_structure_bindings(&program.resources.scenes, external_slots)?,
            };
            let result = match (layouts, constructed) {
                (Some(layouts), true) => PlanProgram::from_lowered_constructed_for_layout(
                    program.id,
                    program.kind,
                    program.semantic_path.clone(),
                    program.content_hash.clone(),
                    program.viewport,
                    program.packed.clone(),
                    program.requirements.clone(),
                    resources,
                    program.destination_uses.clone(),
                    local_plan,
                    local_schedule,
                    &layouts[index],
                ),
                (Some(layouts), false) => PlanProgram::from_lowered_for_layout(
                    program.id,
                    program.kind,
                    program.semantic_path.clone(),
                    program.content_hash.clone(),
                    program.viewport,
                    program.packed.clone(),
                    program.requirements.clone(),
                    resources,
                    program.destination_uses.clone(),
                    local_plan,
                    local_schedule,
                    &layouts[index],
                ),
                (None, true) => PlanProgram::from_lowered_constructed(
                    program.id,
                    program.kind,
                    program.semantic_path.clone(),
                    program.content_hash.clone(),
                    program.viewport,
                    program.packed.clone(),
                    program.requirements.clone(),
                    resources,
                    program.destination_uses.clone(),
                    local_plan,
                    local_schedule,
                ),
                (None, false) => PlanProgram::from_lowered(
                    program.id,
                    program.kind,
                    program.semantic_path.clone(),
                    program.content_hash.clone(),
                    program.viewport,
                    program.packed.clone(),
                    program.requirements.clone(),
                    resources,
                    program.destination_uses.clone(),
                    local_plan,
                    local_schedule,
                ),
            };
            result.map_err(LowerError::from)
        })
        .collect()
}

/// Rebuild only the exact per-frame program payloads after an outer RenderGraph structure hit.
/// The cached template remains authoritative for physical passes/storage; every current external
/// resource is re-admitted by semantic path, key and descriptor before its handle is mapped to a
/// template slot.
pub(crate) fn lower_programs_for_template(
    graph: &RenderGraph,
    dynamic: &DynamicBindings,
    template: &RenderPlanTemplate,
) -> Result<Vec<PlanProgram>, LowerError> {
    // This path accepts only the closed, already-validated product graph constructed immediately
    // before lookup. Public lowering and wire decoding retain their independent validation.
    template.binding_layout().admits_dynamic_layout(dynamic)?;
    let mut external_by_handle = BTreeMap::new();
    let mut matched_slots = BTreeMap::new();
    for resource in &graph.resources {
        let GraphResourceKind::ExternalResource {
            handle,
            key,
            expected,
        } = &resource.kind
        else {
            continue;
        };
        let slot = template
            .binding_layout()
            .external_slots()
            .iter()
            .find(|slot| {
                slot.semantic_path == resource.semantic_path
                    && &slot.key == key
                    && &slot.expected == expected
            })
            .ok_or(LowerError::MissingExternalSlot {
                resource: resource.id,
            })?;
        if matched_slots.insert(slot.id, resource.id).is_some() {
            return Err(LowerError::MissingExternalSlot {
                resource: resource.id,
            });
        }
        external_by_handle.insert(*handle, slot.id);
    }
    if matched_slots.len() != template.binding_layout().external_slots().len() {
        let resource = graph
            .resources
            .iter()
            .find(|resource| matches!(resource.kind, GraphResourceKind::ExternalResource { .. }))
            .map(|resource| resource.id)
            .unwrap_or(graph.output);
        return Err(LowerError::MissingExternalSlot { resource });
    }
    lower_programs_impl(
        graph,
        &external_by_handle,
        Some(template.program_layouts()),
        false,
    )
}

pub(crate) fn lower_constructed_programs_for_template(
    graph: &RenderGraph,
    dynamic: &DynamicBindings,
    template: &RenderPlanTemplate,
) -> Result<Vec<PlanProgram>, LowerError> {
    template.binding_layout().admits_dynamic_layout(dynamic)?;
    let mut external_by_handle = BTreeMap::new();
    let mut matched_slots = BTreeMap::new();
    for resource in &graph.resources {
        let GraphResourceKind::ExternalResource {
            handle,
            key,
            expected,
        } = &resource.kind
        else {
            continue;
        };
        let slot = template
            .binding_layout()
            .external_slots()
            .iter()
            .find(|slot| {
                slot.semantic_path == resource.semantic_path
                    && &slot.key == key
                    && &slot.expected == expected
            })
            .ok_or(LowerError::MissingExternalSlot {
                resource: resource.id,
            })?;
        if matched_slots.insert(slot.id, resource.id).is_some() {
            return Err(LowerError::MissingExternalSlot {
                resource: resource.id,
            });
        }
        external_by_handle.insert(*handle, slot.id);
    }
    if matched_slots.len() != template.binding_layout().external_slots().len() {
        let resource = graph
            .resources
            .iter()
            .find(|resource| matches!(resource.kind, GraphResourceKind::ExternalResource { .. }))
            .map(|resource| resource.id)
            .unwrap_or(graph.output);
        return Err(LowerError::MissingExternalSlot { resource });
    }
    lower_programs_impl(
        graph,
        &external_by_handle,
        Some(template.program_layouts()),
        true,
    )
}

fn lower_effect(effect: &crate::prepare::PreparedEffect) -> PlanEffect {
    PlanEffect {
        semantic_path: effect.semantic_path.clone(),
        space: effect.space,
        kernel: effect.kernel,
    }
}

fn lower_structure_bindings(
    bindings: &[crate::prepare::ProgramStructureBinding],
    external_slots: &BTreeMap<ExternalHandleId, ExternalSlotId>,
) -> Result<Vec<PlanStructureBinding>, LowerError> {
    bindings
        .iter()
        .map(|binding| {
            Ok(PlanStructureBinding {
                key: binding.key.clone(),
                slot: slot_for_handle(external_slots, binding.handle)?,
            })
        })
        .collect()
}

fn slot_for_handle(
    external_slots: &BTreeMap<ExternalHandleId, ExternalSlotId>,
    handle: ExternalHandleId,
) -> Result<ExternalSlotId, LowerError> {
    external_slots
        .get(&handle)
        .copied()
        .ok_or(LowerError::MissingHandleSlot { handle })
}

pub(super) struct SurfaceDraft {
    pub(super) logical_resource: ResourceId,
    pub(super) resource: PlanResourceId,
    pub(super) texture: LogicalTextureDesc,
    pub(super) reason: SurfaceAllocationReason,
}

fn lower_resources(
    graph: &RenderGraph,
    external_slots: &BTreeMap<ResourceId, ExternalSlotId>,
    backdrop_choices: &BTreeMap<ResourceId, BackdropPhysicalChoice>,
) -> Result<(Vec<PlanResource>, Vec<SurfaceDraft>), LowerError> {
    let mut resources = Vec::with_capacity(graph.resources.len());
    let mut surfaces = Vec::new();
    for logical in &graph.resources {
        let id = plan_resource_id(logical.id)?;
        let kind = match &logical.kind {
            GraphResourceKind::ExternalResource { .. } => {
                let slot = external_slots.get(&logical.id).copied().ok_or(
                    LowerError::MissingExternalSlot {
                        resource: logical.id,
                    },
                )?;
                PlanResourceKind::External { slot }
            }
            GraphResourceKind::Output { .. } => PlanResourceKind::OutputTarget {},
            GraphResourceKind::BackdropView { .. } => {
                match backdrop_choice(backdrop_choices, logical.id)? {
                    BackdropPhysicalChoice::Direct { source, reason } => PlanResourceKind::Alias {
                        source: plan_resource_id(source)?,
                        reason,
                    },
                    BackdropPhysicalChoice::Resolve { .. } => {
                        let slot = SurfaceSlotId::from_index(surfaces.len())?;
                        surfaces.push(SurfaceDraft {
                            logical_resource: logical.id,
                            resource: id,
                            texture: require_surface_texture(logical)?,
                            reason: SurfaceAllocationReason::BackdropResolve,
                        });
                        PlanResourceKind::Surface { slot }
                    }
                }
            }
            GraphResourceKind::Composite { .. } | GraphResourceKind::Layer { .. } => {
                let slot = SurfaceSlotId::from_index(surfaces.len())?;
                let reason = surface_allocation_reason(&logical.kind).ok_or(
                    LowerError::UnsupportedResource {
                        resource: logical.id,
                    },
                )?;
                surfaces.push(SurfaceDraft {
                    logical_resource: logical.id,
                    resource: id,
                    texture: require_surface_texture(logical)?,
                    reason,
                });
                PlanResourceKind::Surface { slot }
            }
            _ => {
                return Err(LowerError::UnsupportedResource {
                    resource: logical.id,
                });
            }
        };
        resources.push(PlanResource {
            id,
            semantic_path: logical.semantic_path.clone(),
            logical_resources: vec![logical.id],
            kind,
            roi: logical.roi.clone(),
            origin: logical.origin.clone(),
        });
    }
    Ok((resources, surfaces))
}

fn surface_allocation_reason(kind: &GraphResourceKind) -> Option<SurfaceAllocationReason> {
    match kind {
        GraphResourceKind::Composite { .. } => Some(SurfaceAllocationReason::WorkingComposite),
        GraphResourceKind::Layer {
            role: LayerRole::ImportedSource | LayerRole::ProgramSource,
        } => Some(SurfaceAllocationReason::LayerIntermediate),
        GraphResourceKind::Layer { .. } => Some(SurfaceAllocationReason::KernelIntermediate),
        _ => None,
    }
}

fn require_surface_texture(
    resource: &crate::compositor::graph::GraphResource,
) -> Result<LogicalTextureDesc, LowerError> {
    resource
        .texture
        .clone()
        .ok_or(LowerError::MissingSurfaceTexture {
            resource: resource.id,
        })
}

fn lower_surface_slots(
    drafts: Vec<SurfaceDraft>,
    resources: &[PlanResource],
    passes: &[ExecutionPass],
    capabilities: &BackendCapabilities,
) -> Result<(Vec<SurfaceSlot>, BTreeMap<PlanResourceId, SurfaceSlotId>), LowerError> {
    struct ScheduledDraft {
        draft: SurfaceDraft,
        interval: super::PassInterval,
        estimated_bytes: u64,
    }

    let mut scheduled = Vec::with_capacity(drafts.len());
    for draft in drafts {
        let estimated_bytes = texture_bytes(&draft.texture)?;
        if estimated_bytes > capabilities.max_surface_bytes() {
            return Err(LowerError::SurfaceBudgetExceeded {
                resource: draft.logical_resource,
                required_bytes: estimated_bytes,
                max_bytes: capabilities.max_surface_bytes(),
            });
        }
        let interval = resource_interval(draft.resource, resources, passes)?;
        scheduled.push(ScheduledDraft {
            draft,
            interval,
            estimated_bytes,
        });
    }
    scheduled.sort_by_key(|entry| (entry.interval.first, entry.draft.resource));

    // Deterministic interval coloring. A physical surface can be reused only when its exact
    // texture contract and physical ROI class match, and the previous logical value is dead
    // before the next writer. Keeping root composites out of regional slots is intentional: a
    // later full-frame allocation must not silently inflate an otherwise ROI-sized pool entry.
    // Inclusive intervals deliberately forbid same-pass input/output aliasing.
    let mut slots = Vec::<SurfaceSlot>::new();
    let mut bindings = BTreeMap::new();
    for entry in scheduled {
        let reusable = slots.iter().position(|slot| {
            slot.texture == entry.draft.texture
                && slot.allocations.first().is_some_and(|allocation| {
                    allocation_is_regional(allocation.reason)
                        == allocation_is_regional(entry.draft.reason)
                })
                && slot
                    .allocations
                    .last()
                    .is_some_and(|allocation| allocation.interval.last < entry.interval.first)
        });
        let slot_index = reusable.unwrap_or(slots.len());
        let slot = if slot_index == slots.len() {
            let id = SurfaceSlotId::from_index(slot_index)?;
            slots.push(SurfaceSlot {
                id,
                texture: entry.draft.texture.clone(),
                allocations: Vec::new(),
                estimated_bytes: entry.estimated_bytes,
            });
            id
        } else {
            slots[slot_index].id
        };
        slots[slot_index].allocations.push(SurfaceAllocation {
            resource: entry.draft.resource,
            interval: entry.interval,
            // Preserve the semantic reason for this logical allocation. Reuse is already
            // explicit because several non-overlapping allocations share the same slot.
            reason: entry.draft.reason,
        });
        if bindings.insert(entry.draft.resource, slot).is_some() {
            return Err(LowerError::DuplicateSurfaceBinding {
                resource: entry.draft.resource,
            });
        }
    }
    Ok((slots, bindings))
}

const fn allocation_is_regional(reason: SurfaceAllocationReason) -> bool {
    matches!(
        reason,
        SurfaceAllocationReason::LayerIntermediate
            | SurfaceAllocationReason::BackdropResolve
            | SurfaceAllocationReason::ProgramIntermediate
    )
}

fn plan_resource_id(id: ResourceId) -> Result<PlanResourceId, LowerError> {
    PlanResourceId::try_from(id.get()).map_err(LowerError::from)
}

fn external_slot_id(index: usize) -> Result<ExternalSlotId, LowerError> {
    let value = index
        .checked_add(1)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or(BindingContractError::SlotBudgetExceeded)?;
    ExternalSlotId::try_from(value).map_err(LowerError::from)
}

impl From<crate::compositor::graph::GraphCanonicalError> for LowerError {
    fn from(error: crate::compositor::graph::GraphCanonicalError) -> Self {
        Self::Canonical(error.to_string())
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum LowerError {
    #[error(transparent)]
    Graph(#[from] GraphValidationError),
    #[error(transparent)]
    Dynamic(#[from] RequestError),
    #[error(transparent)]
    CapabilityContract(#[from] CapabilityContractError),
    #[error(transparent)]
    BindingContract(#[from] BindingContractError),
    #[error(transparent)]
    Plan(#[from] PlanValidationError),
    #[error("surface resource {resource:?} has no physical slot")]
    MissingSurfaceSlot { resource: PlanResourceId },
    #[error("surface resource {resource:?} was assigned more than once")]
    DuplicateSurfaceBinding { resource: PlanResourceId },
    #[error(transparent)]
    PlanId(#[from] PlanIdError),
    #[error(transparent)]
    ProgramPlan(#[from] ProgramPlanError),
    #[error(transparent)]
    ProgramSchedule(#[from] ProgramScheduleError),
    #[error("program {program:?} cannot produce a local execution plan: {reason}")]
    InvalidProgramPlan { program: ProgramId, reason: String },
    #[error("lower canonicalization failed: {0}")]
    Canonical(String),
    #[error("lower {kind} count exceeds the u32 plan budget")]
    CountBudgetExceeded { kind: &'static str },
    #[error("backend is missing required graph capability {capability:?}")]
    MissingGraphCapability { capability: GraphCapability },
    #[error("{path} extent {actual:?} exceeds backend maximum {maximum:?}")]
    ExtentExceeded {
        path: String,
        actual: Extent2d,
        maximum: Extent2d,
    },
    #[error("resource {resource:?} uses unsupported texture format {format:?}")]
    UnsupportedTextureFormat {
        resource: ResourceId,
        format: TextureFormat,
    },
    #[error("resource {resource:?} uses unsupported texture usage {usage:?}")]
    UnsupportedTextureUsage {
        resource: ResourceId,
        usage: TextureUsage,
    },
    #[error("resource {resource:?} uses unsupported sample count {sample_count}")]
    UnsupportedSampleCount {
        resource: ResourceId,
        sample_count: u8,
    },
    #[error("resource {resource:?} uses unsupported external pixel layout {pixel_layout:?}")]
    UnsupportedExternalPixelLayout {
        resource: ResourceId,
        pixel_layout: ExternalPixelLayout,
    },
    #[error(
        "resource {resource:?} requires {required_bytes} surface bytes, backend allows {max_bytes}"
    )]
    SurfaceBudgetExceeded {
        resource: ResourceId,
        required_bytes: u64,
        max_bytes: u64,
    },
    #[error("plan requires {required_bytes} peak surface bytes, backend allows {max_bytes}")]
    FrameBudgetExceeded { required_bytes: u64, max_bytes: u64 },
    #[error("lower does not implement logical resource {resource:?}")]
    UnsupportedResource { resource: ResourceId },
    #[error("surface resource {resource:?} has no logical texture contract")]
    MissingSurfaceTexture { resource: ResourceId },
    #[error("external resource {resource:?} has no lowered binding slot")]
    MissingExternalSlot { resource: ResourceId },
    #[error("external handle {handle:?} has no lowered binding slot")]
    MissingHandleSlot { handle: ExternalHandleId },
    #[error("backdrop resource {resource:?} has no physical lowering choice")]
    MissingBackdropChoice { resource: ResourceId },
    #[error("backdrop resource {resource:?} physical choice does not match its logical token")]
    BackdropSourceMismatch { resource: ResourceId },
    #[error("logical layer resource {resource:?} has no dynamic bounds slot")]
    MissingDynamicBounds { resource: ResourceId },
    #[error("logical pass {pass:?} has a non-canonical semantic path")]
    InvalidSemanticPath { pass: PassId },
    #[error("dynamic binding count mismatch: graph expects {expected}, packet has {actual}")]
    DynamicCountMismatch { expected: usize, actual: usize },
    #[error("dynamic binding {id:?} is not referenced by the logical graph")]
    UnexpectedDynamicBinding { id: DynamicBindingId },
    #[error("dynamic binding {id:?} must be {expected_kind:?} at {expected_path}")]
    DynamicLayoutMismatch {
        id: DynamicBindingId,
        expected_path: String,
        expected_kind: DynamicBindingKind,
    },
    #[error("dynamic binding {id:?} has conflicting logical owners")]
    ConflictingDynamicBinding { id: DynamicBindingId },
}
