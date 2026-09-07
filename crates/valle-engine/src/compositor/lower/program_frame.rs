use std::collections::BTreeMap;

use serde::Serialize;
use thiserror::Error;

use crate::{
    compositor::graph::{GraphOrigin, GraphRoi},
    prepare::{
        BoundsError, BoundsReason, DeviceRect, DynamicBindingId, DynamicBindingKind, DynamicValue,
        ProgramId,
    },
    resource::{ContentDigest, Extent2d, TextureFormat},
};

use super::{
    BackendCapabilities, CapabilityContractError, ExecutionPassId, ExecutionPassKind, PassInterval,
    PlanResourceId, PlanValidationError, ProgramAllocationReason, ProgramPassInterval,
    ProgramPassKind, ProgramResourceId, ProgramStorageKind, ProgramSurfaceSlotId, RenderBindings,
    RenderPlanTemplate, SurfaceAllocationReason, SurfaceSlotId,
};

pub const BOUND_PROGRAM_SCHEDULES_FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoundPlanResource {
    resource: PlanResourceId,
    device_roi: DeviceRect,
}

impl BoundPlanResource {
    pub const fn resource(&self) -> PlanResourceId {
        self.resource
    }

    pub const fn device_roi(&self) -> DeviceRect {
        self.device_roi
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoundSurfaceAllocation {
    resource: PlanResourceId,
    device_roi: DeviceRect,
    interval: PassInterval,
    reason: SurfaceAllocationReason,
}

impl BoundSurfaceAllocation {
    pub const fn resource(&self) -> PlanResourceId {
        self.resource
    }

    pub const fn device_roi(&self) -> DeviceRect {
        self.device_roi
    }

    pub const fn interval(&self) -> PassInterval {
        self.interval
    }

    pub const fn reason(&self) -> SurfaceAllocationReason {
        self.reason
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoundSurfaceSlot {
    id: SurfaceSlotId,
    /// `None` means every logical allocation is offscreen for this frame.
    extent: Option<Extent2d>,
    allocations: Vec<BoundSurfaceAllocation>,
    estimated_bytes: u64,
}

impl BoundSurfaceSlot {
    pub const fn id(&self) -> SurfaceSlotId {
        self.id
    }

    pub const fn extent(&self) -> Option<Extent2d> {
        self.extent
    }

    pub fn allocations(&self) -> &[BoundSurfaceAllocation] {
        &self.allocations
    }

    pub const fn estimated_bytes(&self) -> u64 {
        self.estimated_bytes
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoundProgramResource {
    resource: ProgramResourceId,
    device_roi: DeviceRect,
}

impl BoundProgramResource {
    pub const fn resource(&self) -> ProgramResourceId {
        self.resource
    }

    pub const fn device_roi(&self) -> DeviceRect {
        self.device_roi
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoundProgramSurfaceAllocation {
    resource: ProgramResourceId,
    device_roi: DeviceRect,
    interval: ProgramPassInterval,
    reason: ProgramAllocationReason,
}

impl BoundProgramSurfaceAllocation {
    pub const fn resource(&self) -> ProgramResourceId {
        self.resource
    }

    pub const fn device_roi(&self) -> DeviceRect {
        self.device_roi
    }

    pub const fn interval(&self) -> ProgramPassInterval {
        self.interval
    }

    pub const fn reason(&self) -> ProgramAllocationReason {
        self.reason
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoundProgramSurfaceSlot {
    id: ProgramSurfaceSlotId,
    /// `None` means every allocation is offscreen for this frame and no texture is created.
    extent: Option<Extent2d>,
    allocations: Vec<BoundProgramSurfaceAllocation>,
    estimated_bytes: u64,
}

impl BoundProgramSurfaceSlot {
    pub const fn id(&self) -> ProgramSurfaceSlotId {
        self.id
    }

    pub const fn extent(&self) -> Option<Extent2d> {
        self.extent
    }

    pub fn allocations(&self) -> &[BoundProgramSurfaceAllocation] {
        &self.allocations
    }

    pub const fn estimated_bytes(&self) -> u64 {
        self.estimated_bytes
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoundProgramSchedule {
    program: ProgramId,
    execution_pass: ExecutionPassId,
    bounds_reason: BoundsReason,
    resources: Vec<BoundProgramResource>,
    surface_slots: Vec<BoundProgramSurfaceSlot>,
    estimated_surface_bytes: u64,
}

impl BoundProgramSchedule {
    pub const fn program(&self) -> ProgramId {
        self.program
    }

    pub const fn execution_pass(&self) -> ExecutionPassId {
        self.execution_pass
    }

    pub const fn bounds_reason(&self) -> BoundsReason {
        self.bounds_reason
    }

    pub fn resources(&self) -> &[BoundProgramResource] {
        &self.resources
    }

    pub fn surface_slots(&self) -> &[BoundProgramSurfaceSlot] {
        &self.surface_slots
    }

    pub const fn estimated_surface_bytes(&self) -> u64 {
        self.estimated_surface_bytes
    }
}

/// Per-frame device-space projection of all nested program schedules.
///
/// This value is derived after `RenderBindings` admission and is intentionally serialize-only:
/// executors may inspect or trace it, but cannot accept a caller-authored allocation packet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoundProgramSchedules {
    template_hash: ContentDigest,
    binding_hash: ContentDigest,
    capability_fingerprint: ContentDigest,
    resources: Vec<BoundPlanResource>,
    surface_slots: Vec<BoundSurfaceSlot>,
    programs: Vec<BoundProgramSchedule>,
    outer_peak_surface_bytes: u64,
    estimated_peak_surface_bytes: u64,
}

impl BoundProgramSchedules {
    pub const fn template_hash(&self) -> &ContentDigest {
        &self.template_hash
    }

    pub const fn binding_hash(&self) -> &ContentDigest {
        &self.binding_hash
    }

    pub const fn capability_fingerprint(&self) -> &ContentDigest {
        &self.capability_fingerprint
    }

    pub fn resources(&self) -> &[BoundPlanResource] {
        &self.resources
    }

    pub fn surface_slots(&self) -> &[BoundSurfaceSlot] {
        &self.surface_slots
    }

    pub fn programs(&self) -> &[BoundProgramSchedule] {
        &self.programs
    }

    pub const fn outer_peak_surface_bytes(&self) -> u64 {
        self.outer_peak_surface_bytes
    }

    pub const fn estimated_peak_surface_bytes(&self) -> u64 {
        self.estimated_peak_surface_bytes
    }

    pub fn packed_bytes(&self) -> Result<Vec<u8>, super::PackedPlanError> {
        super::packed::encode(
            self,
            super::packed::Contract::schedule(BOUND_PROGRAM_SCHEDULES_FORMAT_VERSION),
        )
    }
}

impl RenderPlanTemplate {
    /// Admits frame bindings, projects program-local bounds and proves the combined surface budget
    /// before an executor may allocate scratch memory.
    pub fn bind_program_schedules(
        &self,
        bindings: &RenderBindings,
        capabilities: &BackendCapabilities,
    ) -> Result<BoundProgramSchedules, ProgramBindingError> {
        // RenderPlanTemplate is validated and assigned its packed identity by every constructor
        // and decoder. It is immutable, so the frame path validates only dynamic bindings and
        // capability identity instead of decoding every packed DrawProgram again.
        self.validate_bindings(bindings)?;
        capabilities.validate()?;
        if capabilities.fingerprint()? != *self.capability_fingerprint() {
            return Err(ProgramBindingError::CapabilityFingerprintMismatch);
        }

        let root = DeviceRect::full(self.render_spec().width(), self.render_spec().height());
        validate_root_extent(root, capabilities)?;
        let template_hash = self.template_hash()?;
        let binding_hash = bindings.binding_hash().clone();
        let capability_fingerprint = capabilities.fingerprint()?;
        let resources = bind_plan_resources(self, bindings, root)?;
        let surface_slots = bind_surface_slots(self, &resources, root, capabilities)?;
        let mut programs = Vec::with_capacity(bindings.programs().len());
        let mut seen = BTreeMap::new();
        for pass in self.passes() {
            let context = match pass.kind {
                ExecutionPassKind::RasterProgram {
                    program,
                    transform,
                    bounds,
                    bounds_reason,
                    ..
                }
                | ExecutionPassKind::RasterCaption {
                    program,
                    transform,
                    bounds,
                    bounds_reason,
                    ..
                } => Some((program, transform, bounds, bounds_reason)),
                _ => None,
            };
            let Some((program_id, transform_id, bounds_id, bounds_reason)) = context else {
                continue;
            };
            if seen.insert(program_id, pass.id).is_some() {
                return invalid(
                    "passes",
                    format!(
                        "program {} has more than one execution pass",
                        program_id.get()
                    ),
                );
            }
            let program = bindings
                .programs()
                .get(program_id.get() as usize - 1)
                .filter(|candidate| candidate.id == program_id)
                .ok_or_else(|| ProgramBindingError::InvalidContract {
                    path: pass.semantic_path.clone(),
                    reason: format!("program {} is undefined", program_id.get()),
                })?;
            let transform = dynamic_transform(bindings, transform_id, &pass.semantic_path)?;
            let outer_bounds = dynamic_bounds(
                bindings,
                bounds_id,
                DynamicBindingKind::Bounds,
                &pass.semantic_path,
            )?;
            validate_rect_in_root(outer_bounds, root, &pass.semantic_path)?;
            if bounds_reason == BoundsReason::ConservativeCameraTarget && outer_bounds != root {
                return invalid(
                    &pass.semantic_path,
                    "conservative program bounds must equal the full render root",
                );
            }
            // Exact outer bounds may include later Timeline effects. Program-local ROIs are
            // therefore projected from the nested program, never guessed from this rectangle.
            programs.push(bind_program(
                program,
                pass.id,
                bounds_reason,
                transform,
                root,
                bindings,
                capabilities,
            )?);
        }
        programs.sort_by_key(|program| program.program.get());
        if programs.len() != bindings.programs().len()
            || programs
                .iter()
                .enumerate()
                .any(|(index, program)| program.program.get() as usize != index + 1)
        {
            return invalid(
                "programs",
                "every plan program must have one canonical execution pass",
            );
        }

        let outer_active = outer_surface_bytes_by_pass(self, &surface_slots)?;
        let outer_peak_surface_bytes = outer_active.iter().copied().max().unwrap_or(0);
        let mut combined = outer_active;
        for program in &programs {
            let active = combined
                .get_mut(program.execution_pass.index())
                .ok_or_else(|| ProgramBindingError::InvalidContract {
                    path: "programs".to_owned(),
                    reason: "program execution pass is outside the plan".to_owned(),
                })?;
            *active = active
                .checked_add(program.estimated_surface_bytes)
                .ok_or(ProgramBindingError::ByteEstimateOverflow)?;
        }
        let estimated_peak_surface_bytes = combined.into_iter().max().unwrap_or(0);
        if estimated_peak_surface_bytes > capabilities.max_frame_bytes() {
            return Err(ProgramBindingError::FrameBudgetExceeded {
                required_bytes: estimated_peak_surface_bytes,
                max_bytes: capabilities.max_frame_bytes(),
            });
        }

        Ok(BoundProgramSchedules {
            template_hash,
            binding_hash,
            capability_fingerprint,
            resources,
            surface_slots,
            programs,
            outer_peak_surface_bytes,
            estimated_peak_surface_bytes,
        })
    }
}

fn bind_plan_resources(
    template: &RenderPlanTemplate,
    bindings: &RenderBindings,
    root: DeviceRect,
) -> Result<Vec<BoundPlanResource>, ProgramBindingError> {
    template
        .resources()
        .iter()
        .map(|resource| {
            let device_roi = match resource.roi {
                GraphRoi::FullFrame => root,
                GraphRoi::Static { rect } => rect,
                GraphRoi::Dynamic { binding } => {
                    dynamic_bounds_any(bindings, binding, &resource.semantic_path)?
                }
            };
            validate_rect_in_root(device_roi, root, &resource.semantic_path)?;
            let expected_origin = [device_roi.x, device_roi.y];
            let actual_origin = match resource.origin {
                GraphOrigin::Static { x, y } => [x, y],
                GraphOrigin::Dynamic { bounds } => {
                    let bounds = dynamic_bounds_any(bindings, bounds, &resource.semantic_path)?;
                    [bounds.x, bounds.y]
                }
            };
            if actual_origin != expected_origin {
                return invalid(
                    &resource.semantic_path,
                    "bound resource origin does not equal its device ROI origin",
                );
            }
            Ok(BoundPlanResource {
                resource: resource.id,
                device_roi,
            })
        })
        .collect()
}

fn bind_surface_slots(
    template: &RenderPlanTemplate,
    resources: &[BoundPlanResource],
    root: DeviceRect,
    capabilities: &BackendCapabilities,
) -> Result<Vec<BoundSurfaceSlot>, ProgramBindingError> {
    let mut result = Vec::with_capacity(template.surface_slots().len());
    for slot in template.surface_slots() {
        let mut width = 0_u32;
        let mut height = 0_u32;
        let mut allocations = Vec::with_capacity(slot.allocations.len());
        for allocation in &slot.allocations {
            let device_roi = resources
                .get(allocation.resource.index())
                .filter(|resource| resource.resource == allocation.resource)
                .map(|resource| resource.device_roi)
                .ok_or_else(|| ProgramBindingError::InvalidContract {
                    path: "surfaceSlots".to_owned(),
                    reason: format!(
                        "surface allocation references undefined resource {}",
                        allocation.resource.get()
                    ),
                })?;
            let physical_roi = if allocation_is_regional(allocation.reason) {
                device_roi
            } else {
                root
            };
            width = width.max(physical_roi.width);
            height = height.max(physical_roi.height);
            allocations.push(BoundSurfaceAllocation {
                resource: allocation.resource,
                device_roi,
                interval: allocation.interval,
                reason: allocation.reason,
            });
        }
        let extent = if width == 0 || height == 0 {
            None
        } else {
            Some(Extent2d::new(width, height).map_err(|error| {
                ProgramBindingError::InvalidContract {
                    path: format!("surfaceSlots[{}]", slot.id.index()),
                    reason: error.to_string(),
                }
            })?)
        };
        if let Some(extent) = extent
            && (extent.width() > capabilities.max_extent().width()
                || extent.height() > capabilities.max_extent().height())
        {
            return invalid(
                format!("surfaceSlots[{}]", slot.id.index()),
                format!(
                    "bound extent {:?} exceeds backend maximum {:?}",
                    extent,
                    capabilities.max_extent()
                ),
            );
        }
        let estimated_bytes = extent.map_or(Ok(0), |extent| {
            program_texture_bytes(extent, slot.texture.format, slot.texture.sample_count())
        })?;
        if estimated_bytes > capabilities.max_surface_bytes() {
            return Err(ProgramBindingError::PlanSurfaceBudgetExceeded {
                slot: slot.id,
                required_bytes: estimated_bytes,
                max_bytes: capabilities.max_surface_bytes(),
            });
        }
        if estimated_bytes > slot.estimated_bytes {
            return invalid(
                format!("surfaceSlots[{}]", slot.id.index()),
                "bound surface exceeds its admitted logical texture",
            );
        }
        result.push(BoundSurfaceSlot {
            id: slot.id,
            extent,
            allocations,
            estimated_bytes,
        });
    }
    Ok(result)
}

const fn allocation_is_regional(reason: SurfaceAllocationReason) -> bool {
    matches!(
        reason,
        SurfaceAllocationReason::LayerIntermediate
            | SurfaceAllocationReason::BackdropResolve
            | SurfaceAllocationReason::ProgramIntermediate
    )
}

fn bind_program(
    program: &super::PlanProgram,
    execution_pass: ExecutionPassId,
    bounds_reason: BoundsReason,
    transform: crate::prepare::DeviceTransform,
    root: DeviceRect,
    bindings: &RenderBindings,
    capabilities: &BackendCapabilities,
) -> Result<BoundProgramSchedule, ProgramBindingError> {
    if !program.local_schedule().surface_slots().is_empty() {
        validate_surface_class(program, capabilities)?;
    }
    let destination_rois =
        resolve_destination_rois(program, bounds_reason, transform, root, bindings)?;

    let mut resources = Vec::with_capacity(program.local_plan().resources().len());
    for resource in program.local_plan().resources() {
        let storage = program
            .local_schedule()
            .storage(resource.id)
            .ok_or_else(|| ProgramBindingError::InvalidContract {
                path: program.semantic_path.clone(),
                reason: format!("resource {} has no storage decision", resource.id.get()),
            })?;
        let device_roi = if matches!(storage, ProgramStorageKind::Transparent {}) {
            DeviceRect::new(0, 0, 0, 0)
        } else if bounds_reason == BoundsReason::ConservativeCameraTarget {
            root
        } else if let Some(destination) = destination_rois.get(&resource.id) {
            *destination
        } else {
            crate::prepare::bounds::program_bounds_to_device(
                resource_bounds_in_program(resource, &program.semantic_path)?,
                program.viewport,
                transform,
                root,
            )?
        };
        validate_rect_in_root(device_roi, root, &program.semantic_path)?;
        resources.push(BoundProgramResource {
            resource: resource.id,
            device_roi,
        });
    }

    let mut surface_slots = Vec::with_capacity(program.local_schedule().surface_slots().len());
    let mut estimated_surface_bytes = 0_u64;
    for slot in program.local_schedule().surface_slots() {
        let mut width = 0_u32;
        let mut height = 0_u32;
        let mut allocations = Vec::with_capacity(slot.allocations.len());
        for allocation in &slot.allocations {
            let device_roi = resources
                .get(allocation.resource.index())
                .filter(|resource| resource.resource == allocation.resource)
                .map(|resource| resource.device_roi)
                .ok_or_else(|| ProgramBindingError::InvalidContract {
                    path: program.semantic_path.clone(),
                    reason: format!(
                        "surface allocation references undefined resource {}",
                        allocation.resource.get()
                    ),
                })?;
            width = width.max(device_roi.width);
            height = height.max(device_roi.height);
            allocations.push(BoundProgramSurfaceAllocation {
                resource: allocation.resource,
                device_roi,
                interval: allocation.interval,
                reason: allocation.reason,
            });
        }
        let extent = if width == 0 || height == 0 {
            None
        } else {
            Some(Extent2d::new(width, height).map_err(|error| {
                ProgramBindingError::InvalidContract {
                    path: program.semantic_path.clone(),
                    reason: error.to_string(),
                }
            })?)
        };
        if let Some(extent) = extent
            && (extent.width() > capabilities.max_extent().width()
                || extent.height() > capabilities.max_extent().height())
        {
            return invalid(
                &program.semantic_path,
                format!(
                    "program surface slot {} extent {:?} exceeds backend maximum {:?}",
                    slot.id.get(),
                    extent,
                    capabilities.max_extent()
                ),
            );
        }
        let estimated_bytes = extent.map_or(Ok(0), |extent| {
            program_texture_bytes(
                extent,
                program.local_schedule().surface_class().format,
                program.local_schedule().surface_class().sample_count,
            )
        })?;
        if estimated_bytes > capabilities.max_surface_bytes() {
            return Err(ProgramBindingError::SurfaceBudgetExceeded {
                program: program.id,
                slot: slot.id,
                required_bytes: estimated_bytes,
                max_bytes: capabilities.max_surface_bytes(),
            });
        }
        estimated_surface_bytes = estimated_surface_bytes
            .checked_add(estimated_bytes)
            .ok_or(ProgramBindingError::ByteEstimateOverflow)?;
        surface_slots.push(BoundProgramSurfaceSlot {
            id: slot.id,
            extent,
            allocations,
            estimated_bytes,
        });
    }

    Ok(BoundProgramSchedule {
        program: program.id,
        execution_pass,
        bounds_reason,
        resources,
        surface_slots,
        estimated_surface_bytes,
    })
}

fn resolve_destination_rois(
    program: &super::PlanProgram,
    bounds_reason: BoundsReason,
    transform: crate::prepare::DeviceTransform,
    root: DeviceRect,
    bindings: &RenderBindings,
) -> Result<BTreeMap<ProgramResourceId, DeviceRect>, ProgramBindingError> {
    let mut result = BTreeMap::new();
    for pass in program.local_plan().passes() {
        match &pass.kind {
            ProgramPassKind::ReadDestination {
                external: Some(destination),
                output,
                ..
            } => {
                let prepared = program
                    .destination_uses
                    .get(destination.index())
                    .ok_or_else(|| ProgramBindingError::InvalidContract {
                        path: pass.semantic_path.clone(),
                        reason: format!(
                            "destination {} has no prepared binding",
                            destination.get()
                        ),
                    })?;
                let required = program
                    .requirements
                    .destination_uses
                    .get(destination.index())
                    .ok_or_else(|| ProgramBindingError::InvalidContract {
                        path: pass.semantic_path.clone(),
                        reason: format!(
                            "destination {} has no DrawRequirements entry",
                            destination.get()
                        ),
                    })?;
                let sample = dynamic_bounds(
                    bindings,
                    prepared.sample_bounds,
                    DynamicBindingKind::BackdropSampleBounds,
                    &pass.semantic_path,
                )?;
                let output_bounds = dynamic_bounds(
                    bindings,
                    prepared.output_bounds,
                    DynamicBindingKind::BackdropOutputBounds,
                    &pass.semantic_path,
                )?;
                validate_rect_in_root(sample, root, &pass.semantic_path)?;
                validate_rect_in_root(output_bounds, root, &pass.semantic_path)?;
                let expected = if bounds_reason == BoundsReason::ConservativeCameraTarget {
                    (root, root)
                } else {
                    let sample = crate::prepare::bounds::program_bounds_to_device(
                        valle_draw::requirements::LocalBounds::from_rect(required.sample_bounds),
                        program.viewport,
                        transform,
                        root,
                    )?;
                    let output = crate::prepare::bounds::program_bounds_to_device(
                        valle_draw::requirements::LocalBounds::from_rect(required.output_bounds),
                        program.viewport,
                        transform,
                        root,
                    )?;
                    (sample, output)
                };
                if (sample, output_bounds) != expected {
                    return invalid(
                        &pass.semantic_path,
                        "destination binding does not equal its device-space projection",
                    );
                }
                insert_destination_roi(&mut result, *output, sample, &pass.semantic_path)?;
            }
            _ => {}
        }
    }
    Ok(result)
}

fn resource_bounds_in_program(
    resource: &super::ProgramResource,
    path: &str,
) -> Result<valle_draw::requirements::LocalBounds, ProgramBindingError> {
    let Some(bounds) = resource.bounds.rect() else {
        return Ok(valle_draw::requirements::LocalBounds::Empty);
    };
    let mapped = resource
        .local_to_program
        .map_bounds(bounds)
        .ok_or_else(|| ProgramBindingError::InvalidContract {
            path: path.to_owned(),
            reason: format!(
                "program resource {} crosses a projective vanishing line",
                resource.id.get()
            ),
        })?;
    Ok(valle_draw::requirements::LocalBounds::from_rect(mapped))
}

fn insert_destination_roi(
    destinations: &mut BTreeMap<ProgramResourceId, DeviceRect>,
    resource: ProgramResourceId,
    roi: DeviceRect,
    path: &str,
) -> Result<(), ProgramBindingError> {
    if destinations
        .insert(resource, roi)
        .is_some_and(|old| old != roi)
    {
        return invalid(path, "destination ROI has conflicting derivations");
    }
    Ok(())
}

fn validate_surface_class(
    program: &super::PlanProgram,
    capabilities: &BackendCapabilities,
) -> Result<(), ProgramBindingError> {
    let class = program.local_schedule().surface_class();
    if !capabilities.formats().contains(&class.format) {
        return invalid(
            &program.semantic_path,
            format!(
                "backend does not support program surface format {:?}",
                class.format
            ),
        );
    }
    if let Some(usage) = class
        .usages
        .iter()
        .copied()
        .find(|usage| !capabilities.texture_usages().contains(usage))
    {
        return invalid(
            &program.semantic_path,
            format!("backend does not support program surface usage {usage:?}"),
        );
    }
    if !capabilities.sample_counts().contains(&class.sample_count) {
        return invalid(
            &program.semantic_path,
            format!(
                "backend does not support program sample count {}",
                class.sample_count
            ),
        );
    }
    Ok(())
}

fn dynamic_transform(
    bindings: &RenderBindings,
    id: DynamicBindingId,
    path: &str,
) -> Result<crate::prepare::DeviceTransform, ProgramBindingError> {
    let binding = bindings
        .dynamic()
        .get(id)
        .filter(|binding| binding.binding_kind == DynamicBindingKind::DeviceTransform)
        .ok_or_else(|| ProgramBindingError::InvalidContract {
            path: path.to_owned(),
            reason: format!("dynamic transform binding {} is invalid", id.get()),
        })?;
    match &binding.value {
        DynamicValue::DeviceTransform(value) => Ok(*value),
        _ => invalid(path, "dynamic transform binding has the wrong value type"),
    }
}

fn dynamic_bounds(
    bindings: &RenderBindings,
    id: DynamicBindingId,
    expected: DynamicBindingKind,
    path: &str,
) -> Result<DeviceRect, ProgramBindingError> {
    let binding = bindings
        .dynamic()
        .get(id)
        .filter(|binding| binding.binding_kind == expected)
        .ok_or_else(|| ProgramBindingError::InvalidContract {
            path: path.to_owned(),
            reason: format!("dynamic bounds binding {} is invalid", id.get()),
        })?;
    match &binding.value {
        DynamicValue::Bounds(value) => Ok(*value),
        _ => invalid(path, "dynamic bounds binding has the wrong value type"),
    }
}

fn dynamic_bounds_any(
    bindings: &RenderBindings,
    id: DynamicBindingId,
    path: &str,
) -> Result<DeviceRect, ProgramBindingError> {
    let binding =
        bindings
            .dynamic()
            .get(id)
            .ok_or_else(|| ProgramBindingError::InvalidContract {
                path: path.to_owned(),
                reason: format!("dynamic bounds binding {} is undefined", id.get()),
            })?;
    if !matches!(
        binding.binding_kind,
        DynamicBindingKind::Bounds
            | DynamicBindingKind::BackdropSampleBounds
            | DynamicBindingKind::BackdropOutputBounds
    ) {
        return invalid(path, "dynamic resource ROI binding has the wrong kind");
    }
    match binding.value {
        DynamicValue::Bounds(value) => Ok(value),
        _ => invalid(
            path,
            "dynamic resource ROI binding has the wrong value type",
        ),
    }
}

fn validate_root_extent(
    root: DeviceRect,
    capabilities: &BackendCapabilities,
) -> Result<(), ProgramBindingError> {
    if root.width > capabilities.max_extent().width()
        || root.height > capabilities.max_extent().height()
    {
        return invalid(
            "renderSpec",
            format!(
                "render root {}x{} exceeds backend maximum {:?}",
                root.width,
                root.height,
                capabilities.max_extent()
            ),
        );
    }
    Ok(())
}

fn validate_rect_in_root(
    rect: DeviceRect,
    root: DeviceRect,
    path: &str,
) -> Result<(), ProgramBindingError> {
    if rect.is_empty() {
        return if rect == DeviceRect::new(0, 0, 0, 0) {
            Ok(())
        } else {
            invalid(path, "empty device ROI is not canonical")
        };
    }
    let right = i64::from(rect.x) + i64::from(rect.width);
    let bottom = i64::from(rect.y) + i64::from(rect.height);
    if rect.x < root.x
        || rect.y < root.y
        || right > i64::from(root.x) + i64::from(root.width)
        || bottom > i64::from(root.y) + i64::from(root.height)
    {
        return invalid(path, "device ROI escapes the render root");
    }
    Ok(())
}

fn program_texture_bytes(
    extent: Extent2d,
    format: TextureFormat,
    sample_count: u8,
) -> Result<u64, ProgramBindingError> {
    let bytes_per_pixel = match format {
        TextureFormat::Rgba16Float => 8_u64,
        TextureFormat::Rgba32Float => 16_u64,
    };
    u64::from(extent.width())
        .checked_mul(u64::from(extent.height()))
        .and_then(|pixels| pixels.checked_mul(bytes_per_pixel))
        .and_then(|bytes| bytes.checked_mul(u64::from(sample_count)))
        .ok_or(ProgramBindingError::ByteEstimateOverflow)
}

fn outer_surface_bytes_by_pass(
    template: &RenderPlanTemplate,
    slots: &[BoundSurfaceSlot],
) -> Result<Vec<u64>, ProgramBindingError> {
    let mut active = vec![0_u64; template.passes().len()];
    for slot in slots {
        for allocation in &slot.allocations {
            for pass in allocation.interval.first.index()..=allocation.interval.last.index() {
                let bytes =
                    active
                        .get_mut(pass)
                        .ok_or_else(|| ProgramBindingError::InvalidContract {
                            path: "surfaceSlots".to_owned(),
                            reason: "surface interval escapes the execution pass table".to_owned(),
                        })?;
                *bytes = bytes
                    .checked_add(slot.estimated_bytes)
                    .ok_or(ProgramBindingError::ByteEstimateOverflow)?;
            }
        }
    }
    Ok(active)
}

fn invalid<T>(
    path: impl Into<String>,
    reason: impl Into<String>,
) -> Result<T, ProgramBindingError> {
    Err(ProgramBindingError::InvalidContract {
        path: path.into(),
        reason: reason.into(),
    })
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ProgramBindingError {
    #[error(transparent)]
    Plan(#[from] PlanValidationError),
    #[error(transparent)]
    Capability(#[from] CapabilityContractError),
    #[error(transparent)]
    Bounds(#[from] BoundsError),
    #[error("backend capability fingerprint does not match the RenderPlanTemplate")]
    CapabilityFingerprintMismatch,
    #[error("invalid bound program schedule at {path}: {reason}")]
    InvalidContract { path: String, reason: String },
    #[error(
        "program {program:?} surface slot {slot:?} requires {required_bytes} bytes, exceeding {max_bytes}"
    )]
    SurfaceBudgetExceeded {
        program: ProgramId,
        slot: ProgramSurfaceSlotId,
        required_bytes: u64,
        max_bytes: u64,
    },
    #[error("plan surface slot {slot:?} requires {required_bytes} bytes, exceeding {max_bytes}")]
    PlanSurfaceBudgetExceeded {
        slot: SurfaceSlotId,
        required_bytes: u64,
        max_bytes: u64,
    },
    #[error("bound frame requires {required_bytes} surface bytes, exceeding {max_bytes}")]
    FrameBudgetExceeded { required_bytes: u64, max_bytes: u64 },
    #[error("bound program surface byte estimate overflow")]
    ByteEstimateOverflow,
}
