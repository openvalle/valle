use std::collections::{BTreeMap, BTreeSet};

use thiserror::Error;

use crate::{
    compositor::reference::{PremulRgba32, decode_author_srgb},
    prepare::{
        DynamicBindingId, PreparedEffect, PreparedEffectKernel, PreparedEffectSpace,
        PreparedExternalBackdrop, PreparedProgram, PreparedProgramKind,
        bounds::MAX_DEVICE_INTERMEDIATE_PIXELS, validate_prepared_program_table,
    },
    resource::{
        ContentDigest, Extent2d, ExternalHandleId, ExternalResourceDesc, LogicalTextureDesc,
        ResourceInterpretation, TextureUsage,
    },
};

use super::{
    GraphCapability, GraphOrigin, GraphResource, GraphResourceKind, GraphRoi, LayerRole,
    LogicalPassKind, OrderEdge, OrderReason, PassId, PassStage, RenderGraph, ResourceAccess,
    ResourceEdge, ResourceId,
};

pub fn validate_graph(graph: &RenderGraph) -> Result<(), GraphValidationError> {
    validate_graph_impl(graph, true)
}

pub(crate) fn validate_constructed_graph(graph: &RenderGraph) -> Result<(), GraphValidationError> {
    validate_graph_impl(graph, false)
}

fn validate_graph_impl(
    graph: &RenderGraph,
    verify_packed_programs: bool,
) -> Result<(), GraphValidationError> {
    validate_envelope(graph)?;
    validate_ids(graph)?;
    validate_resource_contracts(graph)?;
    validate_programs(graph, verify_packed_programs)?;
    validate_capabilities(graph)?;
    validate_edges_and_resources(graph)?;
    validate_terminal_order(graph)?;
    validate_pass_contracts(graph)?;
    validate_program_bindings(graph)?;
    validate_effect_bindings(graph)?;
    validate_dynamic_contracts(graph)?;
    validate_order_edges(graph)?;
    validate_reachability(graph)?;
    let actual_order = graph.topological_order()?;
    let canonical_order: Vec<_> = graph.passes.iter().map(|pass| pass.id).collect();
    if actual_order != canonical_order {
        return Err(invalid(
            "passes",
            "pass table is not in the canonical deterministic topological order",
        ));
    }
    Ok(())
}

fn validate_envelope(graph: &RenderGraph) -> Result<(), GraphValidationError> {
    let pixels = u64::from(graph.render_spec.width()) * u64::from(graph.render_spec.height());
    if pixels > MAX_DEVICE_INTERMEDIATE_PIXELS {
        return Err(invalid(
            "renderSpec",
            format!("output pixel budget exceeded: {pixels} > {MAX_DEVICE_INTERMEDIATE_PIXELS}"),
        ));
    }
    Ok(())
}

fn validate_resource_contracts(graph: &RenderGraph) -> Result<(), GraphValidationError> {
    let extent = Extent2d::new(graph.render_spec.width(), graph.render_spec.height())
        .map_err(|error| invalid("renderSpec", error))?;
    let working_texture = LogicalTextureDesc::production(
        extent,
        [
            TextureUsage::Sampled,
            TextureUsage::StorageRead,
            TextureUsage::StorageWrite,
            TextureUsage::ColorAttachment,
        ],
    )
    .map_err(|error| invalid("resources", error))?;
    let mut next_external = 1_u32;
    let mut reached_internal = false;

    for resource in &graph.resources {
        let path = format!("resources[{}]", resource.id.index());
        if resource.semantic_path.is_empty() {
            return Err(invalid(
                format!("{path}.semanticPath"),
                "resource semantic path must not be empty",
            ));
        }
        match &resource.kind {
            GraphResourceKind::ExternalResource {
                handle,
                key,
                expected,
            } => {
                if reached_internal || handle.get() != next_external {
                    return Err(invalid(
                        &path,
                        "external resources must be the leading contiguous handle-ordered block",
                    ));
                }
                next_external = next_external
                    .checked_add(1)
                    .ok_or_else(|| invalid(&path, "external handle budget exceeded"))?;
                if !external_contract_matches(&key.interpretation, expected) {
                    return Err(invalid(
                        &path,
                        "external key interpretation and expected object type disagree",
                    ));
                }
                expect_layout(resource, None, &GraphRoi::FullFrame, &zero_origin(), &path)?;
            }
            GraphResourceKind::Layer { .. } => {
                reached_internal = true;
                let GraphRoi::Dynamic { binding } = resource.roi else {
                    return Err(invalid(
                        &path,
                        "layer resources require a dynamic bounds ROI",
                    ));
                };
                expect_layout(
                    resource,
                    Some(&working_texture),
                    &GraphRoi::Dynamic { binding },
                    &GraphOrigin::Dynamic { bounds: binding },
                    &path,
                )?;
            }
            GraphResourceKind::Composite { .. } => {
                reached_internal = true;
                expect_layout(
                    resource,
                    Some(&working_texture),
                    &GraphRoi::FullFrame,
                    &zero_origin(),
                    &path,
                )?;
            }
            GraphResourceKind::BackdropView { .. } => {
                reached_internal = true;
                let GraphRoi::Dynamic { binding } = resource.roi else {
                    return Err(invalid(
                        &path,
                        "backdrop views require a dynamic sample ROI",
                    ));
                };
                expect_layout(
                    resource,
                    Some(&working_texture),
                    &GraphRoi::Dynamic { binding },
                    &GraphOrigin::Dynamic { bounds: binding },
                    &path,
                )?;
            }
            GraphResourceKind::Output { spec } => {
                reached_internal = true;
                if *spec != graph.render_spec.output() {
                    return Err(invalid(
                        &path,
                        "output resource spec differs from RenderSpec.output",
                    ));
                }
                expect_layout(resource, None, &GraphRoi::FullFrame, &zero_origin(), &path)?;
            }
            GraphResourceKind::Mask
            | GraphResourceKind::Auxiliary
            | GraphResourceKind::HistorySlot { .. } => {
                return Err(invalid(
                    &path,
                    "resource kind has no producer and execution contract in this RenderGraph",
                ));
            }
        }
    }

    if graph.output.index().checked_add(1) != Some(graph.resources.len()) {
        return Err(invalid(
            "output",
            "terminal output must be the final canonical resource",
        ));
    }
    Ok(())
}

fn expect_layout(
    resource: &GraphResource,
    texture: Option<&LogicalTextureDesc>,
    roi: &GraphRoi,
    origin: &GraphOrigin,
    path: &str,
) -> Result<(), GraphValidationError> {
    if resource.texture.as_ref() != texture || &resource.roi != roi || &resource.origin != origin {
        return Err(invalid(
            path,
            "texture, ROI or surface origin differs from the canonical resource contract",
        ));
    }
    Ok(())
}

const fn zero_origin() -> GraphOrigin {
    GraphOrigin::Static { x: 0, y: 0 }
}

fn external_contract_matches(
    interpretation: &ResourceInterpretation,
    expected: &ExternalResourceDesc,
) -> bool {
    matches!(
        (interpretation, expected),
        (
            ResourceInterpretation::Visual { .. },
            ExternalResourceDesc::VisualFrame { .. }
        ) | (
            ResourceInterpretation::FontFace { .. },
            ExternalResourceDesc::FontBytes
        ) | (
            ResourceInterpretation::RuntimeShader { .. },
            ExternalResourceDesc::RuntimeShader
        ) | (
            ResourceInterpretation::Scene3d { .. },
            ExternalResourceDesc::Scene3d
        )
    )
}

fn validate_ids(graph: &RenderGraph) -> Result<(), GraphValidationError> {
    for (index, resource) in graph.resources.iter().enumerate() {
        if resource.id.index() != index {
            return Err(GraphValidationError::NonCanonicalResourceId { id: resource.id });
        }
    }
    for (index, pass) in graph.passes.iter().enumerate() {
        if pass.id.index() != index {
            return Err(GraphValidationError::NonCanonicalPassId { id: pass.id });
        }
    }
    let mut versions = Vec::new();
    for resource in &graph.resources {
        if let GraphResourceKind::Composite { version } = resource.kind {
            versions.push(version);
        }
    }
    for (index, version) in versions.into_iter().enumerate() {
        if version.index() != index {
            return Err(GraphValidationError::NonCanonicalCompositeVersion);
        }
    }
    Ok(())
}

fn validate_programs(graph: &RenderGraph, verify_packed: bool) -> Result<(), GraphValidationError> {
    validate_prepared_program_table(&graph.programs, verify_packed)
        .map_err(|error| invalid("programs", error))
}

fn validate_capabilities(graph: &RenderGraph) -> Result<(), GraphValidationError> {
    let declared: BTreeSet<_> = graph.capabilities.iter().copied().collect();
    if declared.len() != graph.capabilities.len() {
        return Err(GraphValidationError::DuplicateCapability);
    }
    let required: BTreeSet<_> = graph
        .passes
        .iter()
        .flat_map(|pass| pass.kind.capabilities())
        .collect();
    let canonical: Vec<_> = required.iter().copied().collect();
    if declared != required || graph.capabilities != canonical {
        let missing = required.difference(&declared).next().copied();
        let unused = declared.difference(&required).next().copied();
        return Err(GraphValidationError::CapabilityMismatch { missing, unused });
    }
    Ok(())
}

fn validate_edges_and_resources(graph: &RenderGraph) -> Result<(), GraphValidationError> {
    let mut actual = BTreeSet::new();
    let mut writers: BTreeMap<ResourceId, PassId> = BTreeMap::new();
    for edge in &graph.edges {
        if edge.pass.index() >= graph.passes.len() {
            return Err(GraphValidationError::UndefinedPass { id: edge.pass });
        }
        if edge.resource.index() >= graph.resources.len() {
            return Err(GraphValidationError::UndefinedResource { id: edge.resource });
        }
        let access = match edge.access {
            ResourceAccess::Read => 0_u8,
            ResourceAccess::Write => 1,
        };
        if !actual.insert((edge.pass, edge.resource, access)) {
            return Err(GraphValidationError::DuplicateResourceEdge {
                pass: edge.pass,
                resource: edge.resource,
            });
        }
        if edge.access == ResourceAccess::Write
            && let Some(first) = writers.insert(edge.resource, edge.pass)
        {
            return Err(GraphValidationError::MultipleWriters {
                resource: edge.resource,
                first,
                second: edge.pass,
            });
        }
    }

    let mut expected = Vec::new();
    for pass in &graph.passes {
        for resource in pass.kind.reads() {
            if resource.index() >= graph.resources.len() {
                return Err(GraphValidationError::UndefinedResource { id: resource });
            }
            expected.push(ResourceEdge {
                pass: pass.id,
                resource,
                access: ResourceAccess::Read,
            });
        }
        let output = pass.kind.output();
        if output.index() >= graph.resources.len() {
            return Err(GraphValidationError::UndefinedResource { id: output });
        }
        expected.push(ResourceEdge {
            pass: pass.id,
            resource: output,
            access: ResourceAccess::Write,
        });
    }
    if graph.edges != expected {
        return Err(GraphValidationError::PassEdgeMismatch);
    }

    let mut external_handles = BTreeSet::new();
    for resource in &graph.resources {
        match &resource.kind {
            GraphResourceKind::ExternalResource { handle, .. } => {
                if !external_handles.insert(*handle) {
                    return Err(GraphValidationError::DuplicateExternalHandle {
                        handle: handle.get(),
                    });
                }
                if writers.contains_key(&resource.id) {
                    return Err(GraphValidationError::ExternalHasWriter { id: resource.id });
                }
            }
            kind if kind.is_external() => {
                if writers.contains_key(&resource.id) {
                    return Err(GraphValidationError::ExternalHasWriter { id: resource.id });
                }
            }
            _ if !writers.contains_key(&resource.id) => {
                return Err(GraphValidationError::ResourceHasNoWriter { id: resource.id });
            }
            _ => {}
        }
    }
    let output = graph
        .resources
        .get(graph.output.index())
        .ok_or(GraphValidationError::UndefinedResource { id: graph.output })?;
    if !matches!(output.kind, GraphResourceKind::Output { .. }) {
        return Err(GraphValidationError::InvalidOutputResource);
    }
    let output_count = graph
        .resources
        .iter()
        .filter(|resource| matches!(resource.kind, GraphResourceKind::Output { .. }))
        .count();
    if output_count != 1 {
        return Err(GraphValidationError::InvalidOutputCount {
            found: output_count,
        });
    }
    Ok(())
}

fn validate_pass_contracts(graph: &RenderGraph) -> Result<(), GraphValidationError> {
    let expected_background =
        decode_author_srgb(graph.render_spec.output().background().author_color())
            .map_err(|error| invalid("passes.background", error))?
            .channels();
    let mut clear_count = 0_usize;
    let mut output_count = 0_usize;
    let mut previous_stage = 0_u8;

    for pass in &graph.passes {
        if pass.semantic_path.is_empty() {
            return Err(invalid(
                format!("passes[{}].semanticPath", pass.id.index()),
                "pass semantic path must not be empty",
            ));
        }
        let expected_stage = match pass.kind {
            LogicalPassKind::Caption { .. } => PassStage::Caption,
            LogicalPassKind::OutputTransform { .. } => PassStage::Output,
            _ => PassStage::Visual,
        };
        if pass.stage != expected_stage {
            return Err(GraphValidationError::InvalidPassStage { pass: pass.id });
        }
        let stage = match pass.stage {
            PassStage::Visual => 0,
            PassStage::Caption => 1,
            PassStage::Output => 2,
        };
        if stage < previous_stage {
            return Err(invalid(
                format!("passes[{}].stage", pass.id.index()),
                "pass stages must form visual, caption, then output terminal bands",
            ));
        }
        previous_stage = stage;
        match &pass.kind {
            LogicalPassKind::ClearComposite {
                output,
                working_linear_rec2020_premul,
            } => {
                clear_count += 1;
                if pass.id.index() != 0 || pass.semantic_path != "background.clear" {
                    return Err(invalid(
                        &pass.semantic_path,
                        "clear must be the first pass at background.clear",
                    ));
                }
                PremulRgba32::from_premultiplied(*working_linear_rec2020_premul)
                    .map_err(|error| invalid(&pass.semantic_path, error))?;
                if *working_linear_rec2020_premul != expected_background {
                    return Err(invalid(
                        &pass.semantic_path,
                        "clear color differs from the RenderSpec background",
                    ));
                }
                expect_composite(graph, pass.id, *output)?;
                expect_resource_path(graph, *output, "background.composite")?;
            }
            LogicalPassKind::Import {
                external,
                source_pipeline,
                output,
                placement,
                transform,
            } => {
                placement
                    .validate()
                    .map_err(|error| invalid(&pass.semantic_path, error))?;
                expect_visual_external(graph, pass.id, *external)?;
                let owner = pass.semantic_path.strip_suffix(".import").ok_or_else(|| {
                    invalid(&pass.semantic_path, "Import path must end in .import")
                })?;
                let expected_space = PreparedEffectSpace::Layer {
                    transform: *transform,
                    bounds: layer_bounds_binding(graph, *output)?,
                };
                if let Some(effect) = &source_pipeline.chroma_key {
                    effect
                        .validate_wire()
                        .map_err(|error| invalid(&pass.semantic_path, error))?;
                    if effect.semantic_path != format!("{owner}.effects[0]")
                        || effect.space != expected_space
                        || !matches!(effect.kernel, PreparedEffectKernel::ChromaKey { .. })
                    {
                        return Err(invalid(
                            &pass.semantic_path,
                            "source effect identity, space or kernel is not canonical",
                        ));
                    }
                }
                expect_layer_role(graph, pass.id, *output, LayerRole::ImportedSource)?;
            }
            LogicalPassKind::Draw {
                program,
                external_inputs,
                destination_inputs,
                output,
                ..
            } => {
                expect_program(graph, *program)?;
                for resource in external_inputs {
                    expect_external(graph, pass.id, *resource)?;
                }
                for resource in destination_inputs {
                    expect_resource(graph, pass.id, *resource, "backdrop view", |kind| {
                        matches!(kind, GraphResourceKind::BackdropView { .. })
                    })?;
                }
                expect_layer_role(graph, pass.id, *output, LayerRole::ProgramSource)?;
            }
            LogicalPassKind::Group { input, output } => {
                expect_layer(graph, pass.id, *input)?;
                expect_layer_role(graph, pass.id, *output, LayerRole::Grouped)?;
            }
            LogicalPassKind::BackdropRead {
                token,
                output,
                sample_bounds,
                ..
            } => {
                let Some(composite) = graph.resources.get(token.composite.index()) else {
                    return Err(GraphValidationError::UndefinedResource {
                        id: token.composite,
                    });
                };
                if !matches!(
                    composite.kind,
                    GraphResourceKind::Composite { version } if version == token.version
                ) {
                    return Err(GraphValidationError::InvalidBackdropToken { pass: pass.id });
                }
                let Some(view) = graph.resources.get(output.index()) else {
                    return Err(GraphValidationError::UndefinedResource { id: *output });
                };
                if !matches!(
                    &view.kind,
                    GraphResourceKind::BackdropView {
                        source_version,
                        scope,
                    } if *source_version == token.version && scope == &token.scope
                ) {
                    return Err(GraphValidationError::InvalidBackdropView { pass: pass.id });
                }
                if !matches!(view.roi, GraphRoi::Dynamic { binding } if binding == *sample_bounds) {
                    return Err(invalid(
                        &pass.semantic_path,
                        "backdrop view ROI does not use the pass sample-bounds slot",
                    ));
                }
            }
            LogicalPassKind::Filter { input, output, .. } => {
                expect_layer(graph, pass.id, *input)?;
                expect_layer_role(graph, pass.id, *output, LayerRole::Filtered)?;
            }
            LogicalPassKind::Mask {
                input,
                output,
                mask,
            } => {
                expect_layer(graph, pass.id, *input)?;
                expect_layer_role(graph, pass.id, *output, LayerRole::Masked)?;
                validate_mask(mask, &pass.semantic_path)?;
            }
            LogicalPassKind::CompositeLayer {
                backdrop,
                layer,
                output,
                ..
            }
            | LogicalPassKind::Blend {
                backdrop,
                layer,
                output,
                ..
            } => {
                expect_composite(graph, pass.id, *backdrop)?;
                expect_layer(graph, pass.id, *layer)?;
                expect_composite(graph, pass.id, *output)?;
            }
            LogicalPassKind::Transition {
                backdrop,
                from,
                to,
                output,
                ..
            } => {
                expect_composite(graph, pass.id, *backdrop)?;
                expect_layer(graph, pass.id, *from)?;
                expect_layer(graph, pass.id, *to)?;
                expect_composite(graph, pass.id, *output)?;
            }
            LogicalPassKind::AdjustmentEffect { input, output, .. } => {
                expect_composite(graph, pass.id, *input)?;
                expect_composite(graph, pass.id, *output)?;
            }
            LogicalPassKind::Caption {
                backdrop,
                program,
                external_inputs,
                output,
                ..
            } => {
                expect_program(graph, *program)?;
                expect_composite(graph, pass.id, *backdrop)?;
                for resource in external_inputs {
                    expect_external(graph, pass.id, *resource)?;
                }
                expect_composite(graph, pass.id, *output)?;
            }
            LogicalPassKind::OutputTransform {
                input,
                output,
                spec,
            } => {
                output_count += 1;
                if pass.id.index().checked_add(1) != Some(graph.passes.len())
                    || pass.semantic_path != "output.transform"
                    || *output != graph.output
                    || *spec != graph.render_spec.output()
                {
                    return Err(invalid(
                        &pass.semantic_path,
                        "output transform must be the final pass and mirror RenderSpec.output",
                    ));
                }
                expect_composite(graph, pass.id, *input)?;
                expect_resource(graph, pass.id, *output, "output", |kind| {
                    matches!(kind, GraphResourceKind::Output { .. })
                })?;
                expect_resource_path(graph, *output, "output")?;
            }
        }
        for resource in pass.kind.reads() {
            if matches!(
                graph.resources[resource.index()].kind,
                GraphResourceKind::Output { .. }
            ) {
                return Err(GraphValidationError::OutputRead { pass: pass.id });
            }
        }
    }
    if clear_count != 1 {
        return Err(invalid(
            "passes",
            format!("graph must contain exactly one clear pass, found {clear_count}"),
        ));
    }
    if output_count != 1 {
        return Err(invalid(
            "passes",
            format!("graph must contain exactly one output transform, found {output_count}"),
        ));
    }
    Ok(())
}

fn validate_mask(
    mask: &crate::prepare::PreparedMask,
    path: &str,
) -> Result<(), GraphValidationError> {
    mask.validate_wire().map_err(|error| invalid(path, error))
}

fn validate_program_bindings(graph: &RenderGraph) -> Result<(), GraphValidationError> {
    let writers: BTreeMap<_, _> = graph
        .edges
        .iter()
        .filter(|edge| edge.access == ResourceAccess::Write)
        .map(|edge| (edge.resource, edge.pass))
        .collect();
    let mut next_program = 1_u32;

    for pass in &graph.passes {
        match &pass.kind {
            LogicalPassKind::Draw {
                program,
                external_inputs,
                destination_inputs,
                output,
                bounds_reason,
                ..
            } => {
                if program.get() != next_program {
                    return Err(invalid(
                        &pass.semantic_path,
                        "Draw/Caption program references are not in canonical first-use order",
                    ));
                }
                next_program = next_program
                    .checked_add(1)
                    .ok_or_else(|| invalid("programs", "program id budget exceeded"))?;
                let prepared = prepared_program(graph, *program)?;
                if !matches!(
                    prepared.kind,
                    PreparedProgramKind::Motion | PreparedProgramKind::Solid
                ) {
                    return Err(invalid(
                        &pass.semantic_path,
                        "Draw pass must reference a visual program",
                    ));
                }
                if prepared
                    .destination_uses
                    .iter()
                    .any(|destination| destination.bounds_reason != *bounds_reason)
                {
                    return Err(invalid(
                        &pass.semantic_path,
                        "Draw bounds reason disagrees with its destination projection policy",
                    ));
                }
                let owner = pass
                    .semantic_path
                    .strip_suffix(".draw")
                    .ok_or_else(|| invalid(&pass.semantic_path, "Draw path must end in .draw"))?;
                let program_suffix = match prepared.kind {
                    PreparedProgramKind::Motion => "motion",
                    PreparedProgramKind::Solid => "solid",
                    PreparedProgramKind::Caption => unreachable!(),
                };
                if prepared.semantic_path != format!("{owner}.{program_suffix}") {
                    return Err(invalid(
                        &pass.semantic_path,
                        "visual program semantic path does not match its Draw owner",
                    ));
                }
                if graph.resources[output.index()].semantic_path != format!("{owner}.program") {
                    return Err(invalid(
                        &pass.semantic_path,
                        "Draw output semantic path does not match its layer owner",
                    ));
                }
                let expected_inputs = expected_program_inputs(graph, prepared)?;
                if *external_inputs != expected_inputs {
                    return Err(invalid(
                        &pass.semantic_path,
                        "Draw external inputs are not the exact program resource projection",
                    ));
                }
                validate_program_destinations(
                    graph,
                    &writers,
                    prepared,
                    destination_inputs,
                    owner,
                )?;
            }
            LogicalPassKind::Caption {
                program,
                external_inputs,
                output,
                ..
            } => {
                if program.get() != next_program {
                    return Err(invalid(
                        &pass.semantic_path,
                        "Draw/Caption program references are not in canonical first-use order",
                    ));
                }
                next_program = next_program
                    .checked_add(1)
                    .ok_or_else(|| invalid("programs", "program id budget exceeded"))?;
                let prepared = prepared_program(graph, *program)?;
                if prepared.kind != PreparedProgramKind::Caption
                    || prepared.semantic_path != format!("{}.program", pass.semantic_path)
                    || !prepared.destination_uses.is_empty()
                    || !prepared.requirements.destination_uses.is_empty()
                {
                    return Err(invalid(
                        &pass.semantic_path,
                        "Caption pass has a mismatched program identity or destination dependency",
                    ));
                }
                if graph.resources[output.index()].semantic_path
                    != format!("{}.composite", pass.semantic_path)
                {
                    return Err(invalid(
                        &pass.semantic_path,
                        "Caption output semantic path does not match its owner",
                    ));
                }
                let expected_inputs = expected_program_inputs(graph, prepared)?;
                if *external_inputs != expected_inputs {
                    return Err(invalid(
                        &pass.semantic_path,
                        "Caption external inputs are not the exact program resource projection",
                    ));
                }
            }
            _ => {}
        }
    }
    if next_program as usize != graph.programs.len() + 1 {
        return Err(invalid(
            "programs",
            "program table contains an unreferenced entry",
        ));
    }
    Ok(())
}

fn prepared_program(
    graph: &RenderGraph,
    id: crate::prepare::ProgramId,
) -> Result<&PreparedProgram, GraphValidationError> {
    graph
        .programs
        .get(id.get() as usize - 1)
        .filter(|program| program.id == id)
        .ok_or(GraphValidationError::UndefinedProgram { id: id.get() })
}

fn expected_program_inputs(
    graph: &RenderGraph,
    program: &PreparedProgram,
) -> Result<Vec<ResourceId>, GraphValidationError> {
    let mut result = Vec::new();
    for binding in &program.resources.textures {
        let (id, key, expected) =
            external_for_handle(graph, binding.handle, &program.semantic_path)?;
        if !matches!(key.interpretation, ResourceInterpretation::Visual { .. })
            || !matches!(expected, ExternalResourceDesc::VisualFrame { .. })
        {
            return Err(invalid(
                &program.semantic_path,
                "program texture handle is not a visual external resource",
            ));
        }
        result.push(id);
    }
    for binding in &program.resources.fonts {
        let (id, key, expected) =
            external_for_handle(graph, binding.handle, &program.semantic_path)?;
        if key.content != binding.face_hash
            || !matches!(
                key.interpretation,
                ResourceInterpretation::FontFace { face_index }
                    if face_index == binding.face_index
            )
            || !matches!(expected, ExternalResourceDesc::FontBytes)
        {
            return Err(invalid(
                &program.semantic_path,
                "program font handle identity does not match its requirement",
            ));
        }
        result.push(id);
    }
    for (binding, requirement) in program
        .resources
        .runtime_shaders
        .iter()
        .zip(&program.requirements.runtime_shaders)
    {
        let (id, key, expected) =
            external_for_handle(graph, binding.handle, &program.semantic_path)?;
        let required_content = ContentDigest::from_bytes(requirement.content_hash.into_bytes());
        let required_abi = ContentDigest::from_bytes(requirement.abi_hash.into_bytes());
        if key.content != required_content
            || !matches!(
                &key.interpretation,
                ResourceInterpretation::RuntimeShader { abi_digest, .. }
                    if *abi_digest == required_abi
            )
            || !matches!(expected, ExternalResourceDesc::RuntimeShader)
        {
            return Err(invalid(
                &program.semantic_path,
                "program runtime-shader handle identity does not match its requirement",
            ));
        }
        result.push(id);
    }
    for (binding, requirement) in program
        .resources
        .scenes
        .iter()
        .zip(&program.requirements.scene3d)
    {
        let (id, key, expected) =
            external_for_handle(graph, binding.handle, &program.semantic_path)?;
        let required_content = ContentDigest::from_bytes(requirement.content_hash.into_bytes());
        let required_topology = ContentDigest::from_bytes(requirement.topology_hash.into_bytes());
        if key.content != required_content
            || !matches!(
                &key.interpretation,
                ResourceInterpretation::Scene3d { topology_digest }
                    if *topology_digest == required_topology
            )
            || !matches!(expected, ExternalResourceDesc::Scene3d)
        {
            return Err(invalid(
                &program.semantic_path,
                "program Scene3D handle identity does not match its requirement",
            ));
        }
        result.push(id);
    }
    result.sort_unstable();
    result.dedup();
    Ok(result)
}

fn external_for_handle<'a>(
    graph: &'a RenderGraph,
    handle: ExternalHandleId,
    path: &str,
) -> Result<
    (
        ResourceId,
        &'a crate::resource::ResourceKey,
        &'a ExternalResourceDesc,
    ),
    GraphValidationError,
> {
    let index = handle.get() as usize - 1;
    let resource = graph
        .resources
        .get(index)
        .ok_or_else(|| invalid(path, format!("undefined external handle {}", handle.get())))?;
    let GraphResourceKind::ExternalResource {
        handle: actual,
        key,
        expected,
    } = &resource.kind
    else {
        return Err(invalid(
            path,
            format!(
                "handle {} does not identify an external resource",
                handle.get()
            ),
        ));
    };
    if *actual != handle {
        return Err(invalid(path, "external handle/resource identity mismatch"));
    }
    Ok((resource.id, key, expected))
}

fn validate_program_destinations(
    graph: &RenderGraph,
    writers: &BTreeMap<ResourceId, PassId>,
    program: &PreparedProgram,
    inputs: &[ResourceId],
    owner: &str,
) -> Result<(), GraphValidationError> {
    if inputs.len() != program.destination_uses.len() {
        return Err(invalid(
            owner,
            "Draw destination inputs do not match prepared destination bindings",
        ));
    }
    for (index, (input, prepared)) in inputs.iter().zip(&program.destination_uses).enumerate() {
        let expected_path = format!("{owner}.destination[{index}]");
        let view = &graph.resources[input.index()];
        let GraphResourceKind::BackdropView {
            source_version,
            scope,
        } = &view.kind
        else {
            return Err(invalid(
                &expected_path,
                "Draw input is not a destination view",
            ));
        };
        let writer = writers
            .get(input)
            .copied()
            .ok_or_else(|| invalid(&expected_path, "destination view has no writer"))?;
        let writer = &graph.passes[writer.index()];
        let LogicalPassKind::BackdropRead {
            token,
            output,
            sample_bounds,
            output_bounds,
        } = &writer.kind
        else {
            return Err(invalid(
                &expected_path,
                "backdrop view writer is not BackdropRead",
            ));
        };
        if view.semantic_path != expected_path
            || writer.semantic_path != expected_path
            || *output != *input
            || scope != &prepared.scope
            || token.scope != prepared.scope
            || *source_version != token.version
            || *sample_bounds != prepared.sample_bounds
            || *output_bounds != prepared.output_bounds
        {
            return Err(invalid(
                &expected_path,
                "destination resource/pass does not exactly project the prepared program binding",
            ));
        }
    }
    Ok(())
}

fn validate_effect_bindings(graph: &RenderGraph) -> Result<(), GraphValidationError> {
    for pass in &graph.passes {
        let (effect, root_space) = match &pass.kind {
            LogicalPassKind::Filter { effect, .. } => (effect, false),
            LogicalPassKind::AdjustmentEffect { effect, .. } => (effect, true),
            _ => continue,
        };
        validate_effect_binding(effect, &pass.semantic_path, root_space)?;
    }
    Ok(())
}

fn validate_effect_binding(
    effect: &PreparedEffect,
    pass_path: &str,
    root_space: bool,
) -> Result<(), GraphValidationError> {
    effect
        .validate_wire()
        .map_err(|error| invalid(pass_path, error))?;
    if effect.kernel.is_source_operator() {
        return Err(invalid(
            pass_path,
            "source-alpha operators cannot execute as Filter or AdjustmentEffect kernels",
        ));
    }
    if effect.semantic_path != pass_path
        || root_space != matches!(effect.space, PreparedEffectSpace::Root)
    {
        return Err(invalid(
            pass_path,
            "prepared effect identity or coordinate space does not match its visual pass",
        ));
    }

    Ok(())
}

#[derive(Debug)]
struct LayerSlots {
    owner: String,
    source_lengths: Vec<(DynamicBindingId, String)>,
    transform: DynamicBindingId,
    bounds: DynamicBindingId,
    destinations: Vec<(DynamicBindingId, DynamicBindingId, ResourceId)>,
    effect_lengths: Vec<(DynamicBindingId, String)>,
}

fn validate_dynamic_contracts(graph: &RenderGraph) -> Result<(), GraphValidationError> {
    let writers: BTreeMap<_, _> = graph
        .edges
        .iter()
        .filter(|edge| edge.access == ResourceAccess::Write)
        .map(|edge| (edge.resource, edge.pass))
        .collect();
    let mut visited_layer_passes = BTreeSet::new();
    let mut next = 1_u32;

    for pass in &graph.passes {
        match &pass.kind {
            LogicalPassKind::CompositeLayer {
                backdrop,
                layer,
                opacity,
                ..
            }
            | LogicalPassKind::Blend {
                backdrop,
                layer,
                opacity,
                ..
            } => {
                let owner = pass
                    .semantic_path
                    .strip_suffix(".composite")
                    .ok_or_else(|| {
                        invalid(
                            &pass.semantic_path,
                            "layer composite path must end in .composite",
                        )
                    })?;
                let slots = trace_layer_slots(graph, &writers, *layer, &mut visited_layer_passes)?;
                if slots.owner != owner {
                    return Err(invalid(
                        &pass.semantic_path,
                        "layer source slots do not match their composite owner",
                    ));
                }
                admit_layer_slots(&slots, *opacity, *backdrop, &mut next)?;
            }
            LogicalPassKind::Transition {
                backdrop,
                from,
                to,
                progress,
                from_opacity,
                to_opacity,
                ..
            } => {
                let owner = pass
                    .semantic_path
                    .strip_suffix(".transition")
                    .ok_or_else(|| {
                        invalid(
                            &pass.semantic_path,
                            "transition pass path must end in .transition",
                        )
                    })?;
                expect_next_dynamic(*progress, &format!("{owner}.progress"), &mut next)?;
                let from_slots =
                    trace_layer_slots(graph, &writers, *from, &mut visited_layer_passes)?;
                if from_slots.owner != format!("{owner}.from") {
                    return Err(invalid(
                        &pass.semantic_path,
                        "transition from-layer semantic owner is not canonical",
                    ));
                }
                admit_layer_slots(&from_slots, *from_opacity, *backdrop, &mut next)?;
                let to_slots = trace_layer_slots(graph, &writers, *to, &mut visited_layer_passes)?;
                if to_slots.owner != format!("{owner}.to") {
                    return Err(invalid(
                        &pass.semantic_path,
                        "transition to-layer semantic owner is not canonical",
                    ));
                }
                admit_layer_slots(&to_slots, *to_opacity, *backdrop, &mut next)?;
            }
            LogicalPassKind::Caption {
                transform,
                bounds,
                opacity,
                ..
            } => {
                expect_next_dynamic(
                    *transform,
                    &format!("{}.transform", pass.semantic_path),
                    &mut next,
                )?;
                expect_next_dynamic(
                    *bounds,
                    &format!("{}.bounds", pass.semantic_path),
                    &mut next,
                )?;
                expect_next_dynamic(
                    *opacity,
                    &format!("{}.opacity", pass.semantic_path),
                    &mut next,
                )?;
            }
            LogicalPassKind::AdjustmentEffect { effect, .. } => {
                if effect.space != PreparedEffectSpace::Root {
                    return Err(invalid(
                        &pass.semantic_path,
                        "adjustment effect must execute in root space",
                    ));
                }
                for (id, suffix) in effect.kernel.device_lengths() {
                    expect_next_dynamic(
                        id,
                        &format!("{}.{}", effect.semantic_path, suffix),
                        &mut next,
                    )?;
                }
            }
            _ => {}
        }
    }

    for pass in &graph.passes {
        if matches!(
            pass.kind,
            LogicalPassKind::Import { .. }
                | LogicalPassKind::Draw { .. }
                | LogicalPassKind::Group { .. }
                | LogicalPassKind::Filter { .. }
                | LogicalPassKind::Mask { .. }
        ) && !visited_layer_passes.contains(&pass.id)
        {
            return Err(invalid(
                &pass.semantic_path,
                "layer-producing pass is not owned by exactly one causal-spine item",
            ));
        }
    }
    Ok(())
}

fn trace_layer_slots(
    graph: &RenderGraph,
    writers: &BTreeMap<ResourceId, PassId>,
    resource: ResourceId,
    visited: &mut BTreeSet<PassId>,
) -> Result<LayerSlots, GraphValidationError> {
    let writer = writers
        .get(&resource)
        .copied()
        .ok_or_else(|| invalid("dynamic", "layer resource has no writer"))?;
    if !visited.insert(writer) {
        return Err(invalid(
            &graph.passes[writer.index()].semantic_path,
            "layer pipeline is shared or cyclic instead of uniquely owned",
        ));
    }
    let pass = &graph.passes[writer.index()];
    match &pass.kind {
        LogicalPassKind::Import {
            output,
            source_pipeline,
            placement,
            transform,
            ..
        } => {
            let owner = pass
                .semantic_path
                .strip_suffix(".import")
                .ok_or_else(|| invalid(&pass.semantic_path, "Import path must end in .import"))?;
            if *output != resource
                || graph.resources[resource.index()].semantic_path != format!("{owner}.imported")
            {
                return Err(invalid(
                    &pass.semantic_path,
                    "Import output does not match its canonical layer resource",
                ));
            }
            Ok(LayerSlots {
                owner: owner.to_owned(),
                source_lengths: match placement.backdrop {
                    Some(PreparedExternalBackdrop::Blur {
                        sigma_device_px, ..
                    }) => vec![(
                        sigma_device_px,
                        format!("{owner}.sourcePlacement.backdrop.sigmaDevicePx"),
                    )],
                    _ => Vec::new(),
                },
                transform: *transform,
                bounds: layer_bounds_binding(graph, resource)?,
                destinations: Vec::new(),
                effect_lengths: source_pipeline
                    .chroma_key
                    .iter()
                    .flat_map(|effect| {
                        effect
                            .kernel
                            .device_lengths()
                            .into_iter()
                            .map(|(id, suffix)| {
                                (id, format!("{}.{}", effect.semantic_path, suffix))
                            })
                    })
                    .collect(),
            })
        }
        LogicalPassKind::Draw {
            program,
            destination_inputs,
            output,
            transform,
            ..
        } => {
            let owner = pass
                .semantic_path
                .strip_suffix(".draw")
                .ok_or_else(|| invalid(&pass.semantic_path, "Draw path must end in .draw"))?;
            if *output != resource {
                return Err(invalid(
                    &pass.semantic_path,
                    "Draw output does not match the traced layer resource",
                ));
            }
            let prepared = prepared_program(graph, *program)?;
            let mut destinations = Vec::with_capacity(prepared.destination_uses.len());
            for (input, binding) in destination_inputs.iter().zip(&prepared.destination_uses) {
                let writer = writers.get(input).copied().ok_or_else(|| {
                    invalid(&pass.semantic_path, "destination input has no writer")
                })?;
                let LogicalPassKind::BackdropRead { token, .. } =
                    &graph.passes[writer.index()].kind
                else {
                    return Err(invalid(
                        &pass.semantic_path,
                        "Draw destination input is not produced by BackdropRead",
                    ));
                };
                destinations.push((
                    binding.sample_bounds,
                    binding.output_bounds,
                    token.composite,
                ));
            }
            Ok(LayerSlots {
                owner: owner.to_owned(),
                source_lengths: Vec::new(),
                transform: *transform,
                bounds: layer_bounds_binding(graph, resource)?,
                destinations,
                effect_lengths: Vec::new(),
            })
        }
        LogicalPassKind::Group { input, output } | LogicalPassKind::Mask { input, output, .. } => {
            if *output != resource
                || layer_bounds_binding(graph, *input)? != layer_bounds_binding(graph, *output)?
            {
                return Err(invalid(
                    &pass.semantic_path,
                    "layer operator must preserve its owner's dynamic bounds slot",
                ));
            }
            if !matches!(pass.kind, LogicalPassKind::Group { .. })
                && graph.resources[output.index()].semantic_path != pass.semantic_path
            {
                return Err(invalid(
                    &pass.semantic_path,
                    "layer operator output semantic path is not canonical",
                ));
            }
            trace_layer_slots(graph, writers, *input, visited)
        }
        LogicalPassKind::Filter {
            input,
            output,
            effect,
            ..
        } => {
            if *output != resource
                || layer_bounds_binding(graph, *input)? != layer_bounds_binding(graph, *output)?
                || graph.resources[output.index()].semantic_path != pass.semantic_path
            {
                return Err(invalid(
                    &pass.semantic_path,
                    "filter must preserve its owner's bounds and canonical output path",
                ));
            }
            let mut slots = trace_layer_slots(graph, writers, *input, visited)?;
            if effect.space
                != (PreparedEffectSpace::Layer {
                    transform: slots.transform,
                    bounds: slots.bounds,
                })
            {
                return Err(invalid(
                    &pass.semantic_path,
                    "filter coordinate space does not match its causal layer",
                ));
            }
            slots.effect_lengths.extend(
                effect
                    .kernel
                    .device_lengths()
                    .into_iter()
                    .map(|(id, suffix)| (id, format!("{}.{}", effect.semantic_path, suffix))),
            );
            Ok(slots)
        }
        _ => Err(invalid(
            &pass.semantic_path,
            "layer resource writer has no layer-slot contract",
        )),
    }
}

fn layer_bounds_binding(
    graph: &RenderGraph,
    resource: ResourceId,
) -> Result<DynamicBindingId, GraphValidationError> {
    match graph.resources[resource.index()].roi {
        GraphRoi::Dynamic { binding } => Ok(binding),
        _ => Err(invalid(
            format!("resources[{}].roi", resource.index()),
            "layer resource has no dynamic bounds slot",
        )),
    }
}

fn admit_layer_slots(
    slots: &LayerSlots,
    opacity: DynamicBindingId,
    expected_backdrop: ResourceId,
    next: &mut u32,
) -> Result<(), GraphValidationError> {
    for (id, path) in &slots.source_lengths {
        expect_next_dynamic(*id, path, next)?;
    }
    expect_next_dynamic(slots.transform, &format!("{}.transform", slots.owner), next)?;
    expect_next_dynamic(slots.bounds, &format!("{}.bounds", slots.owner), next)?;
    expect_next_dynamic(opacity, &format!("{}.opacity", slots.owner), next)?;
    for (index, (sample, output, composite)) in slots.destinations.iter().copied().enumerate() {
        if composite != expected_backdrop {
            return Err(invalid(
                format!("{}.destination[{index}]", slots.owner),
                "Motion destination token is not bound to the layer-entry composite",
            ));
        }
        expect_next_dynamic(
            sample,
            &format!("{}.destination[{index}].sampleBounds", slots.owner),
            next,
        )?;
        expect_next_dynamic(
            output,
            &format!("{}.destination[{index}].outputBounds", slots.owner),
            next,
        )?;
    }
    for (id, path) in &slots.effect_lengths {
        expect_next_dynamic(*id, path, next)?;
    }
    Ok(())
}

fn expect_next_dynamic(
    id: DynamicBindingId,
    path: &str,
    next: &mut u32,
) -> Result<(), GraphValidationError> {
    if id.get() != *next {
        return Err(invalid(
            path,
            format!(
                "dynamic id {} is not the next canonical id {}",
                id.get(),
                *next
            ),
        ));
    }
    *next = next
        .checked_add(1)
        .ok_or_else(|| invalid(path, "dynamic binding budget exceeded"))?;
    Ok(())
}

fn expect_program(
    graph: &RenderGraph,
    program: crate::prepare::ProgramId,
) -> Result<(), GraphValidationError> {
    let valid = graph
        .programs
        .get(program.get() as usize - 1)
        .is_some_and(|candidate| candidate.id == program);
    if !valid {
        return Err(GraphValidationError::UndefinedProgram { id: program.get() });
    }
    Ok(())
}

fn expect_external(
    graph: &RenderGraph,
    pass: PassId,
    resource: ResourceId,
) -> Result<(), GraphValidationError> {
    expect_resource(graph, pass, resource, "external resource", |kind| {
        matches!(kind, GraphResourceKind::ExternalResource { .. })
    })
}

fn expect_visual_external(
    graph: &RenderGraph,
    pass: PassId,
    resource: ResourceId,
) -> Result<(), GraphValidationError> {
    expect_resource(graph, pass, resource, "visual external resource", |kind| {
        matches!(
            kind,
            GraphResourceKind::ExternalResource {
                key,
                expected: ExternalResourceDesc::VisualFrame { .. },
                ..
            } if matches!(key.interpretation, ResourceInterpretation::Visual { .. })
        )
    })
}

fn expect_layer(
    graph: &RenderGraph,
    pass: PassId,
    resource: ResourceId,
) -> Result<(), GraphValidationError> {
    expect_resource(graph, pass, resource, "layer", |kind| {
        matches!(kind, GraphResourceKind::Layer { .. })
    })
}

fn expect_layer_role(
    graph: &RenderGraph,
    pass: PassId,
    resource: ResourceId,
    role: LayerRole,
) -> Result<(), GraphValidationError> {
    expect_resource(
        graph,
        pass,
        resource,
        "layer with the pass output role",
        |kind| matches!(kind, GraphResourceKind::Layer { role: actual } if *actual == role),
    )
}

fn expect_composite(
    graph: &RenderGraph,
    pass: PassId,
    resource: ResourceId,
) -> Result<(), GraphValidationError> {
    expect_resource(graph, pass, resource, "composite", |kind| {
        matches!(kind, GraphResourceKind::Composite { .. })
    })
}

fn expect_resource(
    graph: &RenderGraph,
    pass: PassId,
    resource: ResourceId,
    expected: &'static str,
    predicate: impl FnOnce(&GraphResourceKind) -> bool,
) -> Result<(), GraphValidationError> {
    let Some(candidate) = graph.resources.get(resource.index()) else {
        return Err(GraphValidationError::UndefinedResource { id: resource });
    };
    if predicate(&candidate.kind) {
        Ok(())
    } else {
        Err(GraphValidationError::PassResourceKind {
            pass,
            resource,
            expected,
        })
    }
}

fn expect_resource_path(
    graph: &RenderGraph,
    resource: ResourceId,
    expected: &str,
) -> Result<(), GraphValidationError> {
    let candidate = graph
        .resources
        .get(resource.index())
        .ok_or(GraphValidationError::UndefinedResource { id: resource })?;
    if candidate.semantic_path == expected {
        Ok(())
    } else {
        Err(invalid(
            format!("resources[{}].semanticPath", resource.index()),
            format!("semantic path must be {expected:?}"),
        ))
    }
}

fn validate_terminal_order(graph: &RenderGraph) -> Result<(), GraphValidationError> {
    let writers: BTreeMap<_, _> = graph
        .edges
        .iter()
        .filter(|edge| edge.access == ResourceAccess::Write)
        .map(|edge| (edge.resource, edge.pass))
        .collect();
    for edge in graph
        .edges
        .iter()
        .filter(|edge| edge.access == ResourceAccess::Read)
    {
        let Some(writer) = writers.get(&edge.resource).copied() else {
            continue;
        };
        let writer_stage = graph.passes[writer.index()].stage;
        let reader_stage = graph.passes[edge.pass.index()].stage;
        if writer_stage == PassStage::Caption && reader_stage == PassStage::Visual {
            return Err(GraphValidationError::CaptionFeedsVisual {
                caption: writer,
                visual: edge.pass,
            });
        }
        if writer_stage == PassStage::Output {
            return Err(GraphValidationError::OutputIsNotTerminal { pass: writer });
        }
    }
    for edge in &graph.order_edges {
        if edge.before.index() >= graph.passes.len() {
            return Err(GraphValidationError::UndefinedPass { id: edge.before });
        }
        if edge.after.index() >= graph.passes.len() {
            return Err(GraphValidationError::UndefinedPass { id: edge.after });
        }
        if edge.before == edge.after {
            return Err(GraphValidationError::SelfOrderEdge { pass: edge.before });
        }
        let before = graph.passes[edge.before.index()].stage;
        let after = graph.passes[edge.after.index()].stage;
        if before == PassStage::Caption && after == PassStage::Visual {
            return Err(GraphValidationError::CaptionFeedsVisual {
                caption: edge.before,
                visual: edge.after,
            });
        }
        if before == PassStage::Output {
            return Err(GraphValidationError::OutputIsNotTerminal { pass: edge.before });
        }
    }
    Ok(())
}

fn validate_order_edges(graph: &RenderGraph) -> Result<(), GraphValidationError> {
    let mut expected = Vec::new();
    let mut last_spine = None;
    for pass in &graph.passes {
        let reason = match pass.kind {
            LogicalPassKind::ClearComposite { .. } => {
                last_spine = Some(pass.id);
                continue;
            }
            LogicalPassKind::CompositeLayer { .. }
            | LogicalPassKind::Blend { .. }
            | LogicalPassKind::Transition { .. }
            | LogicalPassKind::AdjustmentEffect { .. } => Some(OrderReason::VisualSpine),
            LogicalPassKind::Caption { .. } => Some(OrderReason::CaptionTerminal),
            LogicalPassKind::OutputTransform { .. } => Some(OrderReason::OutputTerminal),
            LogicalPassKind::Import { .. }
            | LogicalPassKind::Draw { .. }
            | LogicalPassKind::Group { .. }
            | LogicalPassKind::BackdropRead { .. }
            | LogicalPassKind::Filter { .. }
            | LogicalPassKind::Mask { .. } => None,
        };
        let Some(reason) = reason else {
            continue;
        };
        let Some(before) = last_spine else {
            return Err(invalid(
                &pass.semantic_path,
                "causal spine pass appears before the background clear",
            ));
        };
        expected.push(OrderEdge {
            before,
            after: pass.id,
            reason,
        });
        last_spine = Some(pass.id);
    }
    if graph.order_edges != expected {
        return Err(invalid(
            "orderEdges",
            "ordering edges are not the exact canonical visual/caption/output spine",
        ));
    }
    Ok(())
}

fn validate_reachability(graph: &RenderGraph) -> Result<(), GraphValidationError> {
    let writers: BTreeMap<_, _> = graph
        .edges
        .iter()
        .filter(|edge| edge.access == ResourceAccess::Write)
        .map(|edge| (edge.resource, edge.pass))
        .collect();
    let mut reader_counts = vec![0_usize; graph.resources.len()];
    for edge in graph
        .edges
        .iter()
        .filter(|edge| edge.access == ResourceAccess::Read)
    {
        reader_counts[edge.resource.index()] += 1;
    }
    for resource in &graph.resources {
        let count = reader_counts[resource.id.index()];
        match resource.kind {
            GraphResourceKind::Layer { .. } | GraphResourceKind::BackdropView { .. }
                if count != 1 =>
            {
                return Err(invalid(
                    format!("resources[{}]", resource.id.index()),
                    "ephemeral layer/backdrop resources must have exactly one consumer",
                ));
            }
            GraphResourceKind::Output { .. } if count != 0 => {
                return Err(invalid(
                    format!("resources[{}]", resource.id.index()),
                    "terminal output must not have a graph consumer",
                ));
            }
            _ => {}
        }
    }

    let mut resources = BTreeSet::new();
    let mut passes = BTreeSet::new();
    let mut pending = vec![graph.output];
    while let Some(resource) = pending.pop() {
        if !resources.insert(resource) {
            continue;
        }
        if let Some(pass) = writers.get(&resource).copied()
            && passes.insert(pass)
        {
            pending.extend(graph.passes[pass.index()].kind.reads());
        }
    }
    if resources.len() != graph.resources.len() || passes.len() != graph.passes.len() {
        return Err(invalid(
            "graph",
            "every pass and resource must contribute to the terminal output",
        ));
    }
    Ok(())
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum GraphValidationError {
    #[error("{path}: {reason}")]
    InvalidContract { path: String, reason: String },
    #[error("resource {id:?} is not in canonical index order")]
    NonCanonicalResourceId { id: ResourceId },
    #[error("pass {id:?} is not in canonical index order")]
    NonCanonicalPassId { id: PassId },
    #[error("composite versions are not contiguous in resource order")]
    NonCanonicalCompositeVersion,
    #[error("program id {id} is not in canonical index order")]
    NonCanonicalProgramId { id: u32 },
    #[error("program {id} destination bindings do not match its requirements")]
    ProgramDestinationMismatch { id: u32 },
    #[error("graph declares a capability more than once")]
    DuplicateCapability,
    #[error(
        "graph capability set differs from pass requirements (missing={missing:?}, unused={unused:?})"
    )]
    CapabilityMismatch {
        missing: Option<GraphCapability>,
        unused: Option<GraphCapability>,
    },
    #[error("edge references undefined pass {id:?}")]
    UndefinedPass { id: PassId },
    #[error("edge or pass references undefined resource {id:?}")]
    UndefinedResource { id: ResourceId },
    #[error("pass {pass:?} repeats an edge for resource {resource:?}")]
    DuplicateResourceEdge { pass: PassId, resource: ResourceId },
    #[error("resource {resource:?} has multiple writers {first:?} and {second:?}")]
    MultipleWriters {
        resource: ResourceId,
        first: PassId,
        second: PassId,
    },
    #[error("explicit resource edges do not match the typed pass I/O contract")]
    PassEdgeMismatch,
    #[error("external handle {handle} is declared more than once")]
    DuplicateExternalHandle { handle: u32 },
    #[error("external resource {id:?} has a graph writer")]
    ExternalHasWriter { id: ResourceId },
    #[error("non-external resource {id:?} has no writer")]
    ResourceHasNoWriter { id: ResourceId },
    #[error("graph output id does not refer to an Output resource")]
    InvalidOutputResource,
    #[error("graph must contain exactly one Output resource, found {found}")]
    InvalidOutputCount { found: usize },
    #[error("pass {pass:?} is assigned to the wrong causal stage")]
    InvalidPassStage { pass: PassId },
    #[error("pass {pass:?} uses resource {resource:?} where {expected} is required")]
    PassResourceKind {
        pass: PassId,
        resource: ResourceId,
        expected: &'static str,
    },
    #[error("pass references undefined program {id}")]
    UndefinedProgram { id: u32 },
    #[error("pass {pass:?} has a backdrop token that does not match its composite version")]
    InvalidBackdropToken { pass: PassId },
    #[error("pass {pass:?} writes a backdrop view with mismatched source/scope")]
    InvalidBackdropView { pass: PassId },
    #[error("pass {pass:?} attempts to read the terminal output")]
    OutputRead { pass: PassId },
    #[error("graph contains a dependency cycle")]
    Cycle,
    #[error("caption pass {caption:?} feeds visual pass {visual:?}")]
    CaptionFeedsVisual { caption: PassId, visual: PassId },
    #[error("output pass {pass:?} is not terminal")]
    OutputIsNotTerminal { pass: PassId },
    #[error("pass {pass:?} has a self ordering edge")]
    SelfOrderEdge { pass: PassId },
}

fn invalid(path: impl Into<String>, reason: impl std::fmt::Display) -> GraphValidationError {
    GraphValidationError::InvalidContract {
        path: path.into(),
        reason: reason.to_string(),
    }
}
