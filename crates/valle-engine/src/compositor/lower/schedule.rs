use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use valle_draw::requirements::LocalBounds;

use crate::resource::{TextureFormat, TextureUsage, WorkingAlphaMode, WorkingColorSpace};

use super::{
    PlanIdError, ProgramDestinationId, ProgramPassId, ProgramPassKind, ProgramPlan,
    ProgramResourceId, ProgramSurfaceSlotId,
};

/// The exact production texture contract shared by every program-local physical surface.
///
/// Extent is deliberately absent: it is resolved from each allocation's local bounds and the
/// frame's `DeviceTransform` when bindings are admitted. Keeping the class in the template makes
/// backend allocation requirements explicit without turning animated bounds into topology.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProgramSurfaceClass {
    pub format: TextureFormat,
    pub working_space: WorkingColorSpace,
    pub alpha: WorkingAlphaMode,
    pub usages: Vec<TextureUsage>,
    pub sample_count: u8,
}

impl ProgramSurfaceClass {
    fn production() -> Self {
        Self {
            format: TextureFormat::Rgba16Float,
            working_space: WorkingColorSpace::LinearRec2020D65,
            alpha: WorkingAlphaMode::PremultipliedCoverage,
            usages: vec![
                TextureUsage::Sampled,
                TextureUsage::StorageRead,
                TextureUsage::StorageWrite,
                TextureUsage::ColorAttachment,
            ],
            sample_count: 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProgramPassInterval {
    pub first: ProgramPassId,
    pub last: ProgramPassId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProgramAllocationReason {
    RasterIntermediate,
    DestinationComposite,
    BackdropKernel,
    MotionGlassKernel,
    MotionGlassForegroundKernel,
    OrderedComposite,
    ClipKernel,
    FilterKernel,
    MaskKernel,
    OpacityKernel,
    ShaderKernel,
    TransformKernel,
    BlendKernel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ProgramStorageKind {
    /// The value is known transparent. It may still carry a non-empty semantic ROI.
    Transparent {},
    /// Immutable view of one outer `RasterProgram.destination_inputs` entry.
    Destination { destination: ProgramDestinationId },
    /// No-op local value forwarding an earlier canonical physical owner.
    Alias { source: ProgramResourceId },
    /// Caller-owned program result. This is never returned to the local surface pool.
    Output {},
    /// A real program-local intermediate assigned to a reusable physical slot.
    Surface { slot: ProgramSurfaceSlotId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProgramResourceStorage {
    pub resource: ProgramResourceId,
    pub storage: ProgramStorageKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProgramSurfaceAllocation {
    pub resource: ProgramResourceId,
    pub local_bounds: LocalBounds,
    pub interval: ProgramPassInterval,
    pub reason: ProgramAllocationReason,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProgramSurfaceSlot {
    pub id: ProgramSurfaceSlotId,
    pub allocations: Vec<ProgramSurfaceAllocation>,
}

/// Canonical physical storage schedule for a [`ProgramPlan`].
///
/// Logical resources remain intact for semantic execution, but only `Surface` entries allocate
/// program-local pixels. Inclusive pass intervals prevent an input from aliasing an output in the
/// same pass; deterministic lowest-slot interval coloring reuses storage as soon as the previous
/// value is dead.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProgramSchedule {
    surface_class: ProgramSurfaceClass,
    resources: Vec<ProgramResourceStorage>,
    surface_slots: Vec<ProgramSurfaceSlot>,
}

impl ProgramSchedule {
    pub const fn surface_class(&self) -> &ProgramSurfaceClass {
        &self.surface_class
    }

    pub fn resources(&self) -> &[ProgramResourceStorage] {
        &self.resources
    }

    pub fn surface_slots(&self) -> &[ProgramSurfaceSlot] {
        &self.surface_slots
    }

    pub fn storage(&self, resource: ProgramResourceId) -> Option<ProgramStorageKind> {
        self.resources
            .get(resource.index())
            .filter(|binding| binding.resource == resource)
            .map(|binding| binding.storage)
    }

    pub(crate) fn derive(plan: &ProgramPlan) -> Result<Self, ProgramScheduleError> {
        Scheduler::new(plan).schedule()
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ProgramScheduleError {
    #[error(transparent)]
    Id(#[from] PlanIdError),
    #[error("{path}: {reason}")]
    InvalidContract { path: String, reason: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StorageDraft {
    Transparent,
    Destination { destination: ProgramDestinationId },
    Alias { source: ProgramResourceId },
    Materialized { reason: ProgramAllocationReason },
    Output,
}

#[derive(Debug, Clone, Copy)]
struct AllocationDraft {
    resource: ProgramResourceId,
    local_bounds: LocalBounds,
    interval: ProgramPassInterval,
    reason: ProgramAllocationReason,
}

struct SlotDraft {
    last: ProgramPassId,
    allocations: Vec<AllocationDraft>,
}

struct Scheduler<'a> {
    plan: &'a ProgramPlan,
    storage: Vec<Option<StorageDraft>>,
    writers: Vec<Option<ProgramPassId>>,
}

impl<'a> Scheduler<'a> {
    fn new(plan: &'a ProgramPlan) -> Self {
        Self {
            plan,
            storage: vec![None; plan.resources().len()],
            writers: vec![None; plan.resources().len()],
        }
    }

    fn schedule(mut self) -> Result<ProgramSchedule, ProgramScheduleError> {
        if self.plan.resources().is_empty() || self.plan.passes().is_empty() {
            return invalid("localSchedule", "program plan must not be empty");
        }

        for pass in self.plan.passes() {
            let output = pass.kind.output();
            let Some(writer) = self.writers.get_mut(output.index()) else {
                return invalid(&pass.semantic_path, "pass output resource is undefined");
            };
            if writer.replace(pass.id).is_some() {
                return invalid(&pass.semantic_path, "resource has more than one writer");
            }
            let draft = self.storage_for_pass(&pass.kind, output, &pass.semantic_path)?;
            let slot = self
                .storage
                .get_mut(output.index())
                .expect("writer lookup proved the resource index exists");
            if slot.replace(draft).is_some() {
                return invalid(&pass.semantic_path, "resource storage was assigned twice");
            }
        }

        if self.storage.iter().any(Option::is_none) || self.writers.iter().any(Option::is_none) {
            return invalid(
                "localSchedule.resources",
                "every program resource must have one writer and storage decision",
            );
        }

        match self.owner(self.plan.output())? {
            ValueOwner::Transparent => {}
            ValueOwner::Destination { .. } => {
                return invalid(
                    "localSchedule.output",
                    "a local program result must never alias the outer destination",
                );
            }
            ValueOwner::Local(owner) => {
                let storage = self.storage[owner.index()]
                    .as_mut()
                    .expect("all resource storage is assigned");
                match storage {
                    StorageDraft::Materialized { .. } => *storage = StorageDraft::Output,
                    StorageDraft::Output => {}
                    _ => {
                        return invalid(
                            "localSchedule.output",
                            "program output owner is not a materialized local value",
                        );
                    }
                }
            }
        }

        let allocations = self.allocations()?;
        let (surface_slots, slot_by_resource) = color_intervals(allocations)?;
        let resources = self
            .storage
            .iter()
            .enumerate()
            .map(|(index, storage)| {
                let resource = ProgramResourceId::from_index(index)?;
                let storage = match storage.expect("all resource storage is assigned") {
                    StorageDraft::Transparent => ProgramStorageKind::Transparent {},
                    StorageDraft::Destination { destination } => {
                        ProgramStorageKind::Destination { destination }
                    }
                    StorageDraft::Alias { source } => ProgramStorageKind::Alias { source },
                    StorageDraft::Output => ProgramStorageKind::Output {},
                    StorageDraft::Materialized { .. } => ProgramStorageKind::Surface {
                        slot: *slot_by_resource.get(&resource).ok_or_else(|| {
                            ProgramScheduleError::InvalidContract {
                                path: "localSchedule.surfaceSlots".to_owned(),
                                reason: format!(
                                    "materialized resource {} has no physical slot",
                                    resource.get()
                                ),
                            }
                        })?,
                    },
                };
                Ok(ProgramResourceStorage { resource, storage })
            })
            .collect::<Result<Vec<_>, ProgramScheduleError>>()?;

        Ok(ProgramSchedule {
            surface_class: ProgramSurfaceClass::production(),
            resources,
            surface_slots,
        })
    }

    fn storage_for_pass(
        &self,
        kind: &ProgramPassKind,
        output: ProgramResourceId,
        path: &str,
    ) -> Result<StorageDraft, ProgramScheduleError> {
        if self.bounds(output)?.rect().is_none() {
            return Ok(StorageDraft::Transparent);
        }
        match kind {
            ProgramPassKind::Clear { .. } => Ok(StorageDraft::Transparent),
            ProgramPassKind::RasterNode { .. } => Ok(StorageDraft::Materialized {
                reason: ProgramAllocationReason::RasterIntermediate,
            }),
            ProgramPassKind::RasterTree { .. } => Ok(StorageDraft::Materialized {
                reason: ProgramAllocationReason::RasterIntermediate,
            }),
            ProgramPassKind::ReadDestination {
                external,
                local_inputs,
                ..
            } => self.destination_storage(*external, local_inputs, path),
            ProgramPassKind::Backdrop { .. } => Ok(StorageDraft::Materialized {
                reason: ProgramAllocationReason::BackdropKernel,
            }),
            ProgramPassKind::MotionGlass { .. } => Ok(StorageDraft::Materialized {
                reason: ProgramAllocationReason::MotionGlassKernel,
            }),
            ProgramPassKind::ApplyMotionGlassForeground { .. } => Ok(StorageDraft::Materialized {
                reason: ProgramAllocationReason::MotionGlassForegroundKernel,
            }),
            ProgramPassKind::SourceOver {
                source,
                destination,
                ..
            } => self.binary_storage(
                *source,
                *destination,
                ProgramAllocationReason::OrderedComposite,
                path,
            ),
            ProgramPassKind::ApplyClip { .. } => Ok(StorageDraft::Materialized {
                reason: ProgramAllocationReason::ClipKernel,
            }),
            ProgramPassKind::ApplyFilter { .. } => Ok(StorageDraft::Materialized {
                reason: ProgramAllocationReason::FilterKernel,
            }),
            ProgramPassKind::ApplyMask { .. } => Ok(StorageDraft::Materialized {
                reason: ProgramAllocationReason::MaskKernel,
            }),
            ProgramPassKind::ApplyOpacity { .. } => Ok(StorageDraft::Materialized {
                reason: ProgramAllocationReason::OpacityKernel,
            }),
            ProgramPassKind::ApplyShader { .. } => Ok(StorageDraft::Materialized {
                reason: ProgramAllocationReason::ShaderKernel,
            }),
            ProgramPassKind::ApplyTransform { .. } => Ok(StorageDraft::Materialized {
                reason: ProgramAllocationReason::TransformKernel,
            }),
            ProgramPassKind::Blend { .. } => Ok(StorageDraft::Materialized {
                reason: ProgramAllocationReason::BlendKernel,
            }),
        }
    }

    fn destination_storage(
        &self,
        external: Option<ProgramDestinationId>,
        local_inputs: &[ProgramResourceId],
        path: &str,
    ) -> Result<StorageDraft, ProgramScheduleError> {
        let mut local_owners = Vec::new();
        for input in local_inputs {
            match self.owner(*input)? {
                ValueOwner::Transparent => {}
                ValueOwner::Destination { .. } => {
                    return invalid(
                        path,
                        "a local destination contribution cannot forward an external destination",
                    );
                }
                ValueOwner::Local(owner) => local_owners.push(owner),
            }
        }
        match (external, local_owners.as_slice()) {
            (None, []) => Ok(StorageDraft::Transparent),
            (None, [source]) => Ok(StorageDraft::Alias { source: *source }),
            (None, _) | (Some(_), [_, ..]) => Ok(StorageDraft::Materialized {
                reason: ProgramAllocationReason::DestinationComposite,
            }),
            (Some(destination), []) => Ok(StorageDraft::Destination { destination }),
        }
    }

    fn binary_storage(
        &self,
        source: ProgramResourceId,
        destination: ProgramResourceId,
        reason: ProgramAllocationReason,
        path: &str,
    ) -> Result<StorageDraft, ProgramScheduleError> {
        match (self.owner(source)?, self.owner(destination)?) {
            (ValueOwner::Transparent, ValueOwner::Transparent) => Ok(StorageDraft::Transparent),
            (ValueOwner::Transparent, owner) | (owner, ValueOwner::Transparent) => {
                Ok(StorageDraft::Alias {
                    source: owner.local(path)?,
                })
            }
            _ => Ok(StorageDraft::Materialized { reason }),
        }
    }

    fn owner(&self, resource: ProgramResourceId) -> Result<ValueOwner, ProgramScheduleError> {
        let Some(storage) = self.storage.get(resource.index()).and_then(|value| *value) else {
            return invalid(
                "localSchedule.resources",
                format!(
                    "resource {} is read before storage is assigned",
                    resource.get()
                ),
            );
        };
        Ok(match storage {
            StorageDraft::Transparent => ValueOwner::Transparent,
            StorageDraft::Destination { destination } => ValueOwner::Destination { destination },
            StorageDraft::Alias { source } => ValueOwner::Local(source),
            StorageDraft::Materialized { .. } | StorageDraft::Output => ValueOwner::Local(resource),
        })
    }

    fn bounds(&self, resource: ProgramResourceId) -> Result<LocalBounds, ProgramScheduleError> {
        self.plan
            .resources()
            .get(resource.index())
            .filter(|value| value.id == resource)
            .map(|value| value.bounds)
            .ok_or_else(|| ProgramScheduleError::InvalidContract {
                path: "localSchedule.resources".to_owned(),
                reason: format!("resource {} is undefined", resource.get()),
            })
    }

    fn allocations(&self) -> Result<Vec<AllocationDraft>, ProgramScheduleError> {
        let mut last_use = BTreeMap::new();
        for (index, storage) in self.storage.iter().enumerate() {
            if matches!(storage, Some(StorageDraft::Materialized { .. })) {
                let resource = ProgramResourceId::from_index(index)?;
                let writer = self.writers[index].expect("all resources have a writer");
                last_use.insert(resource, writer);
            }
        }
        for pass in self.plan.passes() {
            for input in pass.kind.reads() {
                if let ValueOwner::Local(owner) = self.owner(input)?
                    && let Some(last) = last_use.get_mut(&owner)
                    && pass.id > *last
                {
                    *last = pass.id;
                }
            }
        }

        let mut allocations = Vec::with_capacity(last_use.len());
        for (resource, last) in last_use {
            let first = self.writers[resource.index()].expect("all resources have a writer");
            let StorageDraft::Materialized { reason } =
                self.storage[resource.index()].expect("all resource storage is assigned")
            else {
                unreachable!("last-use table contains only materialized resources");
            };
            allocations.push(AllocationDraft {
                resource,
                local_bounds: self.bounds(resource)?,
                interval: ProgramPassInterval { first, last },
                reason,
            });
        }
        allocations.sort_by_key(|allocation| (allocation.interval.first, allocation.resource));
        Ok(allocations)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ValueOwner {
    Transparent,
    Destination { destination: ProgramDestinationId },
    Local(ProgramResourceId),
}

impl ValueOwner {
    fn local(self, path: &str) -> Result<ProgramResourceId, ProgramScheduleError> {
        match self {
            Self::Local(resource) => Ok(resource),
            Self::Transparent => invalid(path, "transparent value has no physical owner"),
            Self::Destination { destination } => invalid(
                path,
                format!(
                    "source-over cannot forward external destination {} as a local source",
                    destination.get()
                ),
            ),
        }
    }
}

fn color_intervals(
    allocations: Vec<AllocationDraft>,
) -> Result<
    (
        Vec<ProgramSurfaceSlot>,
        BTreeMap<ProgramResourceId, ProgramSurfaceSlotId>,
    ),
    ProgramScheduleError,
> {
    let mut slots: Vec<SlotDraft> = Vec::new();
    let mut slot_by_resource = BTreeMap::new();
    for allocation in allocations {
        let slot_index = slots
            .iter()
            .position(|slot| slot.last < allocation.interval.first)
            .unwrap_or_else(|| {
                slots.push(SlotDraft {
                    last: allocation.interval.last,
                    allocations: Vec::new(),
                });
                slots.len() - 1
            });
        let slot = slots
            .get_mut(slot_index)
            .expect("new or existing slot index is valid");
        slot.last = allocation.interval.last;
        slot.allocations.push(allocation);
        let id = ProgramSurfaceSlotId::from_index(slot_index)?;
        if slot_by_resource.insert(allocation.resource, id).is_some() {
            return invalid(
                "localSchedule.surfaceSlots",
                "resource was assigned to more than one physical slot",
            );
        }
    }

    let surface_slots = slots
        .into_iter()
        .enumerate()
        .map(|(index, slot)| {
            Ok(ProgramSurfaceSlot {
                id: ProgramSurfaceSlotId::from_index(index)?,
                allocations: slot
                    .allocations
                    .into_iter()
                    .map(|allocation| ProgramSurfaceAllocation {
                        resource: allocation.resource,
                        local_bounds: allocation.local_bounds,
                        interval: allocation.interval,
                        reason: allocation.reason,
                    })
                    .collect(),
            })
        })
        .collect::<Result<Vec<_>, ProgramScheduleError>>()?;

    let unique = surface_slots
        .iter()
        .flat_map(|slot| {
            slot.allocations
                .iter()
                .map(|allocation| allocation.resource)
        })
        .collect::<BTreeSet<_>>();
    if unique.len() != slot_by_resource.len() {
        return invalid(
            "localSchedule.surfaceSlots",
            "surface allocation table is not one-to-one",
        );
    }
    Ok((surface_slots, slot_by_resource))
}

fn invalid<T>(
    path: impl Into<String>,
    reason: impl Into<String>,
) -> Result<T, ProgramScheduleError> {
    Err(ProgramScheduleError::InvalidContract {
        path: path.into(),
        reason: reason.into(),
    })
}

#[cfg(test)]
mod tests {
    use valle_draw::{
        Rect,
        program::{
            BackdropRead, BackdropScope, DrawProgramBuilder, Filter, Group, LinearColor, Node,
            Paint, PathData, PathNode, PathVerb,
        },
        requirements::Insets,
    };

    use super::*;

    fn rect_path(builder: &mut DrawProgramBuilder, rect: Rect) -> valle_draw::program::NodeId {
        let paint = builder.push_paint(Paint::Solid(LinearColor::new(0.4, 0.3, 0.2, 1.0)));
        let path = builder.push_path(PathData {
            verbs: vec![
                PathVerb::MoveTo,
                PathVerb::LineTo,
                PathVerb::LineTo,
                PathVerb::LineTo,
                PathVerb::Close,
            ],
            points: vec![
                [rect.left(), rect.top()],
                [rect.right(), rect.top()],
                [rect.right(), rect.bottom()],
                [rect.left(), rect.bottom()],
            ],
        });
        builder.push_node(Node::Path(PathNode {
            path,
            fill_rule: Default::default(),
            fill: Some(paint),
            stroke: None,
        }))
    }

    #[test]
    fn simple_leaf_writes_caller_output_without_a_local_surface() {
        let mut builder = DrawProgramBuilder::new(Rect::new(0.0, 0.0, 64.0, 36.0));
        let leaf = rect_path(&mut builder, Rect::new(4.0, 5.0, 12.0, 8.0));
        builder.add_root(leaf);
        let program = builder.finish().unwrap();
        let plan = ProgramPlan::derive(&program).unwrap();
        let schedule = ProgramSchedule::derive(&plan).unwrap();

        assert!(schedule.surface_slots().is_empty());
        assert_eq!(
            schedule.storage(plan.output()),
            Some(ProgramStorageKind::Output {})
        );
        assert!(schedule.resources().iter().all(|resource| !matches!(
            resource.storage,
            ProgramStorageKind::Surface { .. } | ProgramStorageKind::Alias { .. }
        )));
    }

    #[test]
    fn sequential_group_kernels_reuse_non_overlapping_slots() {
        let mut builder = DrawProgramBuilder::new(Rect::new(0.0, 0.0, 64.0, 36.0));
        let leaf = rect_path(&mut builder, Rect::new(4.0, 5.0, 12.0, 8.0));
        let mut group = Group::plain(vec![leaf]);
        group.filters = vec![
            Filter::Blur {
                sigma_x: 1.0,
                sigma_y: 1.0,
            },
            Filter::ColorMatrix {
                matrix: Box::new([
                    1.0, 0.0, 0.0, 0.0, 0.0, // red
                    0.0, 1.0, 0.0, 0.0, 0.0, // green
                    0.0, 0.0, 1.0, 0.0, 0.0, // blue
                    0.0, 0.0, 0.0, 1.0, 0.0, // alpha
                ]),
            },
        ];
        group.opacity = 0.5;
        let root = builder.push_node(Node::Group(group));
        builder.add_root(root);
        let program = builder.finish().unwrap();
        let plan = ProgramPlan::derive(&program).unwrap();
        let schedule = ProgramSchedule::derive(&plan).unwrap();

        let surface_resources = schedule
            .resources()
            .iter()
            .filter(|resource| matches!(resource.storage, ProgramStorageKind::Surface { .. }))
            .count();
        assert!(surface_resources >= 3);
        assert_eq!(schedule.surface_slots().len(), 2);
        assert!(
            schedule
                .surface_slots()
                .iter()
                .any(|slot| slot.allocations.len() >= 2)
        );
        for slot in schedule.surface_slots() {
            for pair in slot.allocations.windows(2) {
                assert!(pair[0].interval.last < pair[1].interval.first);
            }
        }
    }

    #[test]
    fn external_destination_view_is_not_copied_into_a_local_surface() {
        let mut builder = DrawProgramBuilder::new(Rect::new(0.0, 0.0, 64.0, 36.0));
        let leaf = rect_path(&mut builder, Rect::new(4.0, 5.0, 12.0, 8.0));
        let mut group = Group::plain(vec![leaf]);
        group.backdrop = Some(BackdropRead {
            scope: BackdropScope::Current,
            bounds: Rect::new(4.0, 5.0, 12.0, 8.0),
            footprint: Insets::uniform(2.0),
            sampling: valle_draw::requirements::SamplingMode::LinearClamp,
            filters: Vec::new(),
        });
        let root = builder.push_node(Node::Group(group));
        builder.add_root(root);
        let program = builder.finish().unwrap();
        let plan = ProgramPlan::derive(&program).unwrap();
        let schedule = ProgramSchedule::derive(&plan).unwrap();

        let destination_resource = plan
            .passes()
            .iter()
            .find_map(|pass| match pass.kind {
                ProgramPassKind::ReadDestination {
                    external: Some(_),
                    output,
                    ..
                } => Some(output),
                _ => None,
            })
            .unwrap();
        assert!(matches!(
            schedule.storage(destination_resource),
            Some(ProgramStorageKind::Destination { .. })
        ));
        assert!(schedule.surface_slots().iter().all(|slot| {
            slot.allocations
                .iter()
                .all(|allocation| allocation.resource != destination_resource)
        }));
    }
}
