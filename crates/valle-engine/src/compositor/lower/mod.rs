//! Capability-explicit lowering from immutable [`RenderGraph`](super::graph::RenderGraph) values.
//!
//! This module owns only deterministic planning data. Backend objects, decoder frames, promises,
//! canvases and fallback policy remain outside Engine.

mod bindings;
mod build;
mod capability;
mod ids;
mod optimizer;
pub(crate) mod packed;
mod plan;
mod program;
mod program_frame;
mod program_patch;
mod schedule;

pub use bindings::{
    BindingContractError, DynamicSlot, ExternalSlot, ExternalSlotId, PlanBindingLayout,
    RENDER_BINDINGS_FORMAT_VERSION, RenderBindings,
};
pub use build::{LowerError, lower_render_graph};
pub(crate) use build::{
    lower_constructed_programs_for_template, lower_constructed_render_graph,
    lower_programs_for_template,
};
pub use capability::{BackendCapabilities, CapabilityContractError, FramebufferFetchSemantics};
pub use ids::{
    ExecutionPassId, PlanIdError, PlanResourceId, ProgramDestinationId, ProgramPassId,
    ProgramResourceId, ProgramSurfaceSlotId, SurfaceSlotId,
};
pub use packed::{
    MAX_PACKED_BINDINGS_BYTES, MAX_PACKED_PLAN_BYTES, MAX_PACKED_REQUESTS_BYTES,
    MAX_PACKED_SCHEDULE_BYTES, PackedPlanError,
};
pub use plan::{
    CompositeMode, CopyOperation, CopyReason, ExecutionPass, ExecutionPassKind,
    FormatConversionReason, KernelInvocation, OptimizationReport, OptimizationRewrite,
    OptimizationRewriteKind, PassInterval, PlanEffect, PlanFontBinding, PlanProgram,
    PlanProgramLayout, PlanProgramResources, PlanResource, PlanResourceKind, PlanSourcePipeline,
    PlanStructureBinding, PlanTextureBinding, PlanValidationError, RENDER_PLAN_FORMAT_VERSION,
    RenderPlanTemplate, ResolveReason, ResourceAliasReason, SurfaceAllocation,
    SurfaceAllocationReason, SurfaceSlot,
};
pub use program::{
    ProgramDestinationKind, ProgramPass, ProgramPassKind, ProgramPlan, ProgramPlanError,
    ProgramResource,
};
pub use program_frame::{
    BOUND_PROGRAM_SCHEDULES_FORMAT_VERSION, BoundPlanResource, BoundProgramResource,
    BoundProgramSchedule, BoundProgramSchedules, BoundProgramSurfaceAllocation,
    BoundProgramSurfaceSlot, BoundSurfaceAllocation, BoundSurfaceSlot, ProgramBindingError,
};
pub use schedule::{
    ProgramAllocationReason, ProgramPassInterval, ProgramResourceStorage, ProgramSchedule,
    ProgramScheduleError, ProgramStorageKind, ProgramSurfaceAllocation, ProgramSurfaceClass,
    ProgramSurfaceSlot,
};
