//! Deterministic physical rewrites applied after semantic lowering and before liveness coloring.
//!
//! Every rewrite keeps the closed logical pass/resource tables intact: only storage and mechanical
//! execution change. That makes the logical graph hash and reference semantics invariant while
//! reason-coded alias passes remain observable in the final plan.

use crate::{
    canonical,
    prepare::DynamicBindingId,
    resource::{ContentDigest, LogicalTextureDesc},
};

use super::{
    ExecutionPass, ExecutionPassKind, KernelInvocation, LowerError, PlanResource, PlanResourceId,
    PlanResourceKind, ResourceAliasReason, SurfaceSlotId,
    build::SurfaceDraft,
    plan::{OptimizationReport, OptimizationRewrite, OptimizationRewriteKind},
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolveKey {
    input: PlanResourceId,
    sample_bounds: DynamicBindingId,
    output_bounds: DynamicBindingId,
    texture: LogicalTextureDesc,
}

pub(super) fn optimize(
    resources: &mut [PlanResource],
    surface_drafts: &mut Vec<SurfaceDraft>,
    passes: &mut [ExecutionPass],
) -> Result<OptimizationReport, LowerError> {
    let input_physical_hash = state_hash(resources, passes)?;
    let mut rewrites = Vec::new();
    reuse_backdrop_resolves(resources, surface_drafts, passes, &mut rewrites)?;
    eliminate_noop_group_surfaces(resources, surface_drafts, passes, &mut rewrites)?;
    rewrites.sort_unstable_by_key(|rewrite| rewrite.pass);
    let output_physical_hash = state_hash(resources, passes)?;
    Ok(OptimizationReport {
        input_physical_hash,
        output_physical_hash,
        rewrites,
    })
}

/// Hash the physical rewrite state without liveness-assigned slot numbers. Surface coloring is a
/// later deterministic phase and must not make the rewrite proof backend-allocation-order specific.
pub(super) fn state_hash(
    resources: &[PlanResource],
    passes: &[ExecutionPass],
) -> Result<ContentDigest, LowerError> {
    let placeholder = SurfaceSlotId::try_from(1_u32)?;
    let mut normalized = resources.to_vec();
    for resource in &mut normalized {
        if matches!(resource.kind, PlanResourceKind::Surface { .. }) {
            resource.kind = PlanResourceKind::Surface { slot: placeholder };
        }
    }
    let bytes = canonical::bytes(&(normalized, passes))
        .map_err(|error| LowerError::Canonical(error.to_string()))?;
    Ok(ContentDigest::of_bytes(&bytes))
}

fn reuse_backdrop_resolves(
    resources: &mut [PlanResource],
    surface_drafts: &mut Vec<SurfaceDraft>,
    passes: &mut [ExecutionPass],
    rewrites: &mut Vec<OptimizationRewrite>,
) -> Result<(), LowerError> {
    let mut canonical = Vec::<(ResolveKey, PlanResourceId)>::new();
    for pass in passes {
        let ExecutionPassKind::ResolveRegion {
            input,
            output,
            sample_bounds,
            output_bounds,
            ..
        } = pass.kind
        else {
            continue;
        };
        let texture = surface_draft(surface_drafts, output)?.texture.clone();
        let key = ResolveKey {
            input,
            sample_bounds,
            output_bounds,
            texture,
        };
        let Some((_, source)) = canonical.iter().find(|(candidate, _)| candidate == &key) else {
            canonical.push((key, output));
            continue;
        };
        let source = *source;
        make_alias(
            resources,
            surface_drafts,
            output,
            source,
            ResourceAliasReason::ReusedBackdropResolve,
        )?;
        pass.kind = ExecutionPassKind::BindBackdropView {
            input: source,
            output,
            reason: ResourceAliasReason::ReusedBackdropResolve,
        };
        rewrites.push(OptimizationRewrite {
            kind: OptimizationRewriteKind::BackdropResolveReuse,
            pass: pass.id,
            output,
            source,
        });
    }
    Ok(())
}

fn eliminate_noop_group_surfaces(
    resources: &mut [PlanResource],
    surface_drafts: &mut Vec<SurfaceDraft>,
    passes: &mut [ExecutionPass],
    rewrites: &mut Vec<OptimizationRewrite>,
) -> Result<(), LowerError> {
    for pass in passes {
        let ExecutionPassKind::DispatchKernel {
            invocation: KernelInvocation::Group { input, output },
        } = pass.kind
        else {
            continue;
        };
        let source = physical_source(resources, input)?;
        make_alias(
            resources,
            surface_drafts,
            output,
            source,
            ResourceAliasReason::NoOpGroup,
        )?;
        pass.kind = ExecutionPassKind::AliasResource {
            input: source,
            output,
            reason: ResourceAliasReason::NoOpGroup,
        };
        rewrites.push(OptimizationRewrite {
            kind: OptimizationRewriteKind::NoOpGroupElimination,
            pass: pass.id,
            output,
            source,
        });
    }
    Ok(())
}

fn physical_source(
    resources: &[PlanResource],
    resource: PlanResourceId,
) -> Result<PlanResourceId, LowerError> {
    let value = resources
        .get(resource.index())
        .filter(|candidate| candidate.id == resource)
        .ok_or(LowerError::MissingSurfaceSlot { resource })?;
    match value.kind {
        PlanResourceKind::Surface { .. } => Ok(resource),
        PlanResourceKind::Alias { source, .. } => physical_source(resources, source),
        PlanResourceKind::External { .. } | PlanResourceKind::OutputTarget {} => {
            Err(LowerError::MissingSurfaceSlot { resource })
        }
    }
}

fn make_alias(
    resources: &mut [PlanResource],
    surface_drafts: &mut Vec<SurfaceDraft>,
    output: PlanResourceId,
    source: PlanResourceId,
    reason: ResourceAliasReason,
) -> Result<(), LowerError> {
    if source.index() >= output.index()
        || !matches!(
            resources.get(source.index()).map(|resource| &resource.kind),
            Some(PlanResourceKind::Surface { .. })
        )
    {
        return Err(LowerError::MissingSurfaceSlot { resource: source });
    }
    let resource = resources
        .get_mut(output.index())
        .filter(|candidate| candidate.id == output)
        .ok_or(LowerError::MissingSurfaceSlot { resource: output })?;
    if !matches!(resource.kind, PlanResourceKind::Surface { .. }) {
        return Err(LowerError::MissingSurfaceSlot { resource: output });
    }
    resource.kind = PlanResourceKind::Alias { source, reason };
    let index = surface_drafts
        .iter()
        .position(|draft| draft.resource == output)
        .ok_or(LowerError::MissingSurfaceSlot { resource: output })?;
    surface_drafts.remove(index);
    Ok(())
}

fn surface_draft(
    drafts: &[SurfaceDraft],
    resource: PlanResourceId,
) -> Result<&SurfaceDraft, LowerError> {
    drafts
        .iter()
        .find(|draft| draft.resource == resource)
        .ok_or(LowerError::MissingSurfaceSlot { resource })
}

#[cfg(test)]
mod tests {
    use crate::{
        compositor::{
            graph::{GraphOrigin, GraphRoi, PassId, ResourceId},
            lower::{ExecutionPassId, ResolveReason, SurfaceAllocationReason, SurfaceSlotId},
        },
        prepare::DynamicBindingId,
        resource::{Extent2d, LogicalTextureDesc, TextureUsage},
    };

    use super::*;

    fn resource_id(value: u32) -> PlanResourceId {
        PlanResourceId::try_from(value).unwrap()
    }

    fn graph_resource_id(value: u32) -> ResourceId {
        ResourceId::try_from(value).unwrap()
    }

    fn pass_id(value: u32) -> ExecutionPassId {
        ExecutionPassId::try_from(value).unwrap()
    }

    fn logical_pass_id(value: u32) -> PassId {
        PassId::try_from(value).unwrap()
    }

    fn binding(value: u32) -> DynamicBindingId {
        DynamicBindingId::try_from(value).unwrap()
    }

    fn texture() -> LogicalTextureDesc {
        LogicalTextureDesc::production(
            Extent2d::new(64, 48).unwrap(),
            [TextureUsage::Sampled, TextureUsage::ColorAttachment],
        )
        .unwrap()
    }

    fn surface_resource(value: u32, roi: GraphRoi, origin: GraphOrigin) -> PlanResource {
        PlanResource {
            id: resource_id(value),
            semantic_path: format!("resource[{value}]"),
            logical_resources: vec![graph_resource_id(value)],
            kind: PlanResourceKind::Surface {
                slot: SurfaceSlotId::try_from(value).unwrap(),
            },
            roi,
            origin,
        }
    }

    fn draft(value: u32, reason: SurfaceAllocationReason) -> SurfaceDraft {
        SurfaceDraft {
            logical_resource: graph_resource_id(value),
            resource: resource_id(value),
            texture: texture(),
            reason,
        }
    }

    #[test]
    fn exact_backdrop_resolves_share_one_physical_surface() {
        let sample = binding(1);
        let output_bounds = binding(2);
        let roi = GraphRoi::Dynamic { binding: sample };
        let origin = GraphOrigin::Dynamic { bounds: sample };
        let mut resources = vec![
            surface_resource(1, GraphRoi::FullFrame, GraphOrigin::Static { x: 0, y: 0 }),
            surface_resource(2, roi.clone(), origin.clone()),
            surface_resource(3, roi, origin),
        ];
        let mut drafts = vec![
            draft(2, SurfaceAllocationReason::BackdropResolve),
            draft(3, SurfaceAllocationReason::BackdropResolve),
        ];
        let mut passes = vec![
            ExecutionPass {
                id: pass_id(1),
                semantic_path: "destination[0]".into(),
                logical_passes: vec![logical_pass_id(1)],
                kind: ExecutionPassKind::ResolveRegion {
                    input: resource_id(1),
                    output: resource_id(2),
                    sample_bounds: sample,
                    output_bounds,
                    reason: ResolveReason::NonSampleableRenderTarget,
                },
            },
            ExecutionPass {
                id: pass_id(2),
                semantic_path: "destination[1]".into(),
                logical_passes: vec![logical_pass_id(2)],
                kind: ExecutionPassKind::ResolveRegion {
                    input: resource_id(1),
                    output: resource_id(3),
                    sample_bounds: sample,
                    output_bounds,
                    reason: ResolveReason::NonSampleableRenderTarget,
                },
            },
        ];

        let report = optimize(&mut resources, &mut drafts, &mut passes).unwrap();

        assert_eq!(drafts.len(), 1);
        assert_ne!(report.input_physical_hash, report.output_physical_hash);
        assert_eq!(
            report.rewrites,
            [OptimizationRewrite {
                kind: OptimizationRewriteKind::BackdropResolveReuse,
                pass: pass_id(2),
                output: resource_id(3),
                source: resource_id(2),
            }]
        );
        assert_eq!(drafts[0].resource, resource_id(2));
        assert!(matches!(
            resources[2].kind,
            PlanResourceKind::Alias {
                source,
                reason: ResourceAliasReason::ReusedBackdropResolve,
            } if source == resource_id(2)
        ));
        assert!(matches!(
            passes[1].kind,
            ExecutionPassKind::BindBackdropView {
                input,
                output,
                reason: ResourceAliasReason::ReusedBackdropResolve,
            } if input == resource_id(2) && output == resource_id(3)
        ));
    }

    #[test]
    fn no_op_group_aliases_are_flattened_to_the_physical_owner() {
        let mut resources = vec![
            surface_resource(1, GraphRoi::FullFrame, GraphOrigin::Static { x: 0, y: 0 }),
            surface_resource(2, GraphRoi::FullFrame, GraphOrigin::Static { x: 0, y: 0 }),
            surface_resource(3, GraphRoi::FullFrame, GraphOrigin::Static { x: 0, y: 0 }),
        ];
        let mut drafts = vec![
            draft(2, SurfaceAllocationReason::LayerIntermediate),
            draft(3, SurfaceAllocationReason::LayerIntermediate),
        ];
        let mut passes = vec![
            ExecutionPass {
                id: pass_id(1),
                semantic_path: "group[0]".into(),
                logical_passes: vec![logical_pass_id(1)],
                kind: ExecutionPassKind::DispatchKernel {
                    invocation: KernelInvocation::Group {
                        input: resource_id(1),
                        output: resource_id(2),
                    },
                },
            },
            ExecutionPass {
                id: pass_id(2),
                semantic_path: "group[1]".into(),
                logical_passes: vec![logical_pass_id(2)],
                kind: ExecutionPassKind::DispatchKernel {
                    invocation: KernelInvocation::Group {
                        input: resource_id(2),
                        output: resource_id(3),
                    },
                },
            },
        ];

        let report = optimize(&mut resources, &mut drafts, &mut passes).unwrap();

        assert!(drafts.is_empty());
        assert_ne!(report.input_physical_hash, report.output_physical_hash);
        assert_eq!(report.rewrites.len(), 2);
        assert!(
            report
                .rewrites
                .iter()
                .all(|rewrite| rewrite.kind == OptimizationRewriteKind::NoOpGroupElimination)
        );
        for resource in &resources[1..] {
            assert!(matches!(
                resource.kind,
                PlanResourceKind::Alias {
                    source,
                    reason: ResourceAliasReason::NoOpGroup,
                } if source == resource_id(1)
            ));
        }
        for (index, pass) in passes.iter().enumerate() {
            assert!(matches!(
                pass.kind,
                ExecutionPassKind::AliasResource {
                    input,
                    output,
                    reason: ResourceAliasReason::NoOpGroup,
                } if input == resource_id(1) && output == resource_id(index as u32 + 2)
            ));
        }
    }
}
