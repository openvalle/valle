//! Stable, backend-independent inspection artifacts for an admitted compositor plan.
//!
//! These values are machine contracts, not log formatting. They deliberately contain semantic
//! paths and content identities but never an input filename or an executor-owned object. Timing
//! values are supplied by the host because Engine planning remains deterministic and clock-free.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    compositor::{
        graph::{
            GraphResourceKind, GraphRoi, LogicalPassKind, PassStage, RenderGraph, ResourceAccess,
        },
        lower::{
            BackendCapabilities, BoundProgramSchedules, ExecutionPassKind, PassInterval,
            PlanResourceId, PlanResourceKind, ProgramAllocationReason, ProgramPassInterval,
            ProgramResourceId, ProgramSurfaceSlotId, RenderBindings, RenderPlanTemplate,
            ResourceAliasReason, SurfaceAllocationReason, SurfaceSlotId,
        },
    },
    frame::RenderSpec,
    prepare::{DeviceRect, DynamicValue, PreparedFrame, ProgramId},
    render::{FrameKey, RenderId},
    resource::ContentDigest,
};

pub const PLAN_INSPECTION_SCHEMA_VERSION: u16 = 1;
pub const FRAME_PERF_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InspectionArtifactKind {
    CompositorPlan,
    CompositorFramePerf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BoundProgramResourceInspection {
    pub resource: ProgramResourceId,
    pub device_roi: DeviceRect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BoundProgramSurfaceAllocationInspection {
    pub resource: ProgramResourceId,
    pub device_roi: DeviceRect,
    pub interval: ProgramPassInterval,
    pub reason: ProgramAllocationReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BoundProgramSurfaceSlotInspection {
    pub id: ProgramSurfaceSlotId,
    pub extent: Option<crate::resource::Extent2d>,
    pub allocations: Vec<BoundProgramSurfaceAllocationInspection>,
    pub estimated_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BoundProgramInspection {
    pub program: ProgramId,
    pub execution_pass: crate::compositor::lower::ExecutionPassId,
    pub bounds_reason: crate::prepare::BoundsReason,
    pub resources: Vec<BoundProgramResourceInspection>,
    pub surface_slots: Vec<BoundProgramSurfaceSlotInspection>,
    pub estimated_surface_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BoundPlanResourceInspection {
    pub resource: PlanResourceId,
    pub device_roi: DeviceRect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BoundSurfaceAllocationInspection {
    pub resource: PlanResourceId,
    pub device_roi: DeviceRect,
    pub interval: PassInterval,
    pub reason: SurfaceAllocationReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BoundSurfaceSlotInspection {
    pub id: SurfaceSlotId,
    pub extent: Option<crate::resource::Extent2d>,
    pub allocations: Vec<BoundSurfaceAllocationInspection>,
    pub estimated_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BoundProgramSchedulesInspection {
    pub template_hash: ContentDigest,
    pub binding_hash: ContentDigest,
    pub capability_fingerprint: ContentDigest,
    pub resources: Vec<BoundPlanResourceInspection>,
    pub surface_slots: Vec<BoundSurfaceSlotInspection>,
    pub programs: Vec<BoundProgramInspection>,
    pub outer_peak_surface_bytes: u64,
    pub estimated_peak_surface_bytes: u64,
}

impl From<&BoundProgramSchedules> for BoundProgramSchedulesInspection {
    fn from(schedules: &BoundProgramSchedules) -> Self {
        Self {
            template_hash: schedules.template_hash().clone(),
            binding_hash: schedules.binding_hash().clone(),
            capability_fingerprint: schedules.capability_fingerprint().clone(),
            resources: schedules
                .resources()
                .iter()
                .map(|resource| BoundPlanResourceInspection {
                    resource: resource.resource(),
                    device_roi: resource.device_roi(),
                })
                .collect(),
            surface_slots: schedules
                .surface_slots()
                .iter()
                .map(|slot| BoundSurfaceSlotInspection {
                    id: slot.id(),
                    extent: slot.extent(),
                    allocations: slot
                        .allocations()
                        .iter()
                        .map(|allocation| BoundSurfaceAllocationInspection {
                            resource: allocation.resource(),
                            device_roi: allocation.device_roi(),
                            interval: allocation.interval(),
                            reason: allocation.reason(),
                        })
                        .collect(),
                    estimated_bytes: slot.estimated_bytes(),
                })
                .collect(),
            programs: schedules
                .programs()
                .iter()
                .map(|program| BoundProgramInspection {
                    program: program.program(),
                    execution_pass: program.execution_pass(),
                    bounds_reason: program.bounds_reason(),
                    resources: program
                        .resources()
                        .iter()
                        .map(|resource| BoundProgramResourceInspection {
                            resource: resource.resource(),
                            device_roi: resource.device_roi(),
                        })
                        .collect(),
                    surface_slots: program
                        .surface_slots()
                        .iter()
                        .map(|slot| BoundProgramSurfaceSlotInspection {
                            id: slot.id(),
                            extent: slot.extent(),
                            allocations: slot
                                .allocations()
                                .iter()
                                .map(|allocation| BoundProgramSurfaceAllocationInspection {
                                    resource: allocation.resource(),
                                    device_roi: allocation.device_roi(),
                                    interval: allocation.interval(),
                                    reason: allocation.reason(),
                                })
                                .collect(),
                            estimated_bytes: slot.estimated_bytes(),
                        })
                        .collect(),
                    estimated_surface_bytes: program.estimated_surface_bytes(),
                })
                .collect(),
            outer_peak_surface_bytes: schedules.outer_peak_surface_bytes(),
            estimated_peak_surface_bytes: schedules.estimated_peak_surface_bytes(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanInspectionCounts {
    pub logical_passes: u64,
    pub logical_resources: u64,
    pub logical_surfaces: u64,
    pub execution_passes: u64,
    pub plan_resources: u64,
    pub outer_surface_slots: u64,
    pub live_outer_surface_slots: u64,
    pub declared_program_surface_slots: u64,
    pub live_program_surface_slots: u64,
    pub planned_roi_count: u64,
    /// Sum of per-resource ROI pixels, not the union area.
    pub planned_roi_pixels: u64,
    pub resolve_passes: u64,
    pub direct_backdrop_views: u64,
    pub reused_backdrop_resolves: u64,
    pub elided_group_surfaces: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanInspection {
    pub schema_version: u16,
    pub kind: InspectionArtifactKind,
    pub graph_hash: ContentDigest,
    pub template_hash: ContentDigest,
    pub capability_fingerprint: ContentDigest,
    pub capabilities: BackendCapabilities,
    pub template: RenderPlanTemplate,
    pub bindings: RenderBindings,
    pub bound_program_schedules: BoundProgramSchedulesInspection,
    pub counts: PlanInspectionCounts,
}

impl PlanInspection {
    pub fn build(
        graph: &RenderGraph,
        template: &RenderPlanTemplate,
        bindings: &RenderBindings,
        capabilities: &BackendCapabilities,
    ) -> Result<Self, InspectionError> {
        graph.validate().map_err(|error| invalid("graph", error))?;
        template
            .validate()
            .map_err(|error| invalid("template", error))?;
        template
            .validate_bindings(bindings)
            .map_err(|error| invalid("bindings", error))?;
        capabilities
            .validate()
            .map_err(|error| invalid("capabilities", error))?;
        let graph_hash = graph
            .semantic_hash()
            .map_err(|error| invalid("graph", error))?;
        if template.render_id() != &graph.render_id {
            return Err(InspectionError::CrossArtifactMismatch);
        }
        let fingerprint = capabilities
            .fingerprint()
            .map_err(|error| invalid("capabilities", error))?;
        if template.capability_fingerprint() != &fingerprint {
            return Err(InspectionError::CrossArtifactMismatch);
        }
        let schedules = template
            .bind_program_schedules(bindings, capabilities)
            .map_err(|error| invalid("programSchedules", error))?;
        let bound_program_schedules = BoundProgramSchedulesInspection::from(&schedules);
        let counts = plan_counts(graph, template, bindings, &bound_program_schedules)?;

        Ok(Self {
            schema_version: PLAN_INSPECTION_SCHEMA_VERSION,
            kind: InspectionArtifactKind::CompositorPlan,
            graph_hash,
            template_hash: template
                .template_hash()
                .map_err(|error| invalid("template", error))?,
            capability_fingerprint: fingerprint,
            capabilities: capabilities.clone(),
            template: template.clone(),
            bindings: bindings.clone(),
            bound_program_schedules,
            counts,
        })
    }

    pub fn validate(&self) -> Result<(), InspectionError> {
        if self.schema_version != PLAN_INSPECTION_SCHEMA_VERSION
            || self.kind != InspectionArtifactKind::CompositorPlan
        {
            return Err(InspectionError::WrongArtifactContract);
        }
        self.template
            .validate()
            .map_err(|error| invalid("template", error))?;
        self.template
            .validate_bindings(&self.bindings)
            .map_err(|error| invalid("bindings", error))?;
        if self.template.capability_fingerprint() != &self.capability_fingerprint
            || self
                .template
                .template_hash()
                .map_err(|error| invalid("template", error))?
                != self.template_hash
            || self
                .capabilities
                .fingerprint()
                .map_err(|error| invalid("capabilities", error))?
                != self.capability_fingerprint
        {
            return Err(InspectionError::CrossArtifactMismatch);
        }
        let expected_schedule = self
            .template
            .bind_program_schedules(&self.bindings, &self.capabilities)
            .map_err(|error| invalid("boundProgramSchedules", error))?;
        if self.bound_program_schedules != BoundProgramSchedulesInspection::from(&expected_schedule)
        {
            return Err(InspectionError::CrossArtifactMismatch);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FrameStageTimings {
    pub render_open_us: u64,
    pub semantic_preflight_us: u64,
    pub evaluate_us: u64,
    pub prepare_us: u64,
    pub build_us: u64,
    pub validate_us: u64,
    pub lower_us: u64,
    pub bind_us: u64,
    pub execute_us: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "kebab-case", deny_unknown_fields)]
pub enum FrameMetric {
    Measured { value: u64 },
    Planned { value: u64 },
    NotObservable {},
    NotApplicable {},
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FrameStructuralMetrics {
    pub logical_passes: u64,
    pub physical_passes: u64,
    pub logical_surfaces: u64,
    pub physical_surfaces: FrameMetric,
    pub peak_surface_bytes: FrameMetric,
    pub roi_count: u64,
    pub roi_pixels: FrameMetric,
    pub resolve_passes: u64,
    pub direct_backdrop_views: u64,
    pub reused_backdrop_resolves: u64,
    pub elided_group_surfaces: u64,
    pub peak_rss_bytes: FrameMetric,
    pub peak_vram_bytes: FrameMetric,
    pub upload_bytes: FrameMetric,
    pub readback_bytes: FrameMetric,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FramePerfInspection {
    pub schema_version: u16,
    pub kind: InspectionArtifactKind,
    pub render_id: RenderId,
    pub frame: FrameKey,
    pub graph_hash: ContentDigest,
    pub template_hash: ContentDigest,
    pub stages: FrameStageTimings,
    pub metrics: FrameStructuralMetrics,
}

impl FramePerfInspection {
    pub fn build(
        frame: &PreparedFrame,
        graph: &RenderGraph,
        plan: &PlanInspection,
        stages: FrameStageTimings,
    ) -> Result<Self, InspectionError> {
        frame
            .validate()
            .map_err(|error| invalid("preparedFrame", error))?;
        graph.validate().map_err(|error| invalid("graph", error))?;
        plan.validate()?;
        let graph_hash = graph
            .semantic_hash()
            .map_err(|error| invalid("graph", error))?;
        if frame.render_id != graph.render_id
            || frame.render_id != *plan.template.render_id()
            || frame.render_spec != graph.render_spec
            || frame.render_spec != plan.template.render_spec()
            || graph_hash != plan.graph_hash
        {
            return Err(InspectionError::CrossArtifactMismatch);
        }
        let physical_surfaces = plan
            .counts
            .live_outer_surface_slots
            .checked_add(plan.counts.live_program_surface_slots)
            .ok_or(InspectionError::CountOverflow)?;
        Ok(Self {
            schema_version: FRAME_PERF_SCHEMA_VERSION,
            kind: InspectionArtifactKind::CompositorFramePerf,
            render_id: frame.render_id,
            frame: frame.key,
            graph_hash: plan.graph_hash.clone(),
            template_hash: plan.template_hash.clone(),
            stages,
            metrics: FrameStructuralMetrics {
                logical_passes: plan.counts.logical_passes,
                physical_passes: plan.counts.execution_passes,
                logical_surfaces: plan.counts.logical_surfaces,
                physical_surfaces: FrameMetric::Planned {
                    value: physical_surfaces,
                },
                peak_surface_bytes: FrameMetric::Planned {
                    value: plan.bound_program_schedules.estimated_peak_surface_bytes,
                },
                roi_count: plan.counts.planned_roi_count,
                roi_pixels: FrameMetric::Planned {
                    value: plan.counts.planned_roi_pixels,
                },
                resolve_passes: plan.counts.resolve_passes,
                direct_backdrop_views: plan.counts.direct_backdrop_views,
                reused_backdrop_resolves: plan.counts.reused_backdrop_resolves,
                elided_group_surfaces: plan.counts.elided_group_surfaces,
                // Deterministic graph/lower inspection cannot observe host process or device
                // counters. A delivery host may publish measured metrics in its own envelope.
                peak_rss_bytes: FrameMetric::NotObservable {},
                peak_vram_bytes: FrameMetric::NotObservable {},
                upload_bytes: FrameMetric::NotObservable {},
                readback_bytes: FrameMetric::NotObservable {},
            },
        })
    }

    pub fn validate(&self) -> Result<(), InspectionError> {
        if self.schema_version != FRAME_PERF_SCHEMA_VERSION
            || self.kind != InspectionArtifactKind::CompositorFramePerf
        {
            return Err(InspectionError::WrongArtifactContract);
        }
        Ok(())
    }
}

/// Emits a deterministic DOT projection of logical pass/resource/version and explicit order
/// edges. Only semantic paths already admitted by the graph are rendered.
pub fn render_graph_dot(graph: &RenderGraph) -> Result<String, InspectionError> {
    use std::fmt::Write as _;

    graph.validate().map_err(|error| invalid("graph", error))?;
    let mut dot = String::from(
        "strict digraph valle_render_graph {\n  rankdir=LR;\n  graph [charset=\"UTF-8\"];\n",
    );
    for resource in &graph.resources {
        writeln!(
            dot,
            "  r{} [shape=ellipse,label=\"r{}\\n{}\\n{}\"];",
            resource.id.get(),
            resource.id.get(),
            dot_escape(&resource_kind_label(&resource.kind)),
            dot_escape(&resource.semantic_path),
        )
        .expect("writing to String cannot fail");
    }
    for pass in &graph.passes {
        writeln!(
            dot,
            "  p{} [shape=box,label=\"p{} / {}\\n{}\\n{}\"];",
            pass.id.get(),
            pass.id.get(),
            pass_stage_label(pass.stage),
            pass_kind_label(&pass.kind),
            dot_escape(&pass.semantic_path),
        )
        .expect("writing to String cannot fail");
    }
    for edge in &graph.edges {
        match edge.access {
            ResourceAccess::Read => writeln!(
                dot,
                "  r{} -> p{} [label=\"read\"];",
                edge.resource.get(),
                edge.pass.get()
            ),
            ResourceAccess::Write => writeln!(
                dot,
                "  p{} -> r{} [label=\"write\"];",
                edge.pass.get(),
                edge.resource.get()
            ),
        }
        .expect("writing to String cannot fail");
    }
    for edge in &graph.order_edges {
        writeln!(
            dot,
            "  p{} -> p{} [style=dashed,label=\"order:{}\"];",
            edge.before.get(),
            edge.after.get(),
            order_reason_label(edge.reason),
        )
        .expect("writing to String cannot fail");
    }
    dot.push_str("}\n");
    Ok(dot)
}

fn plan_counts(
    graph: &RenderGraph,
    template: &RenderPlanTemplate,
    bindings: &RenderBindings,
    schedules: &BoundProgramSchedulesInspection,
) -> Result<PlanInspectionCounts, InspectionError> {
    let declared_program_surface_slots =
        bindings
            .programs()
            .iter()
            .try_fold(0_u64, |total, program| {
                total
                    .checked_add(count(program.local_schedule().surface_slots().len())?)
                    .ok_or(InspectionError::CountOverflow)
            })?;
    let live_program_surface_slots =
        schedules
            .programs
            .iter()
            .try_fold(0_u64, |total, program| {
                total
                    .checked_add(count(
                        program
                            .surface_slots
                            .iter()
                            .filter(|slot| slot.extent.is_some())
                            .count(),
                    )?)
                    .ok_or(InspectionError::CountOverflow)
            })?;
    let live_outer_surface_slots = count(
        schedules
            .surface_slots
            .iter()
            .filter(|slot| slot.extent.is_some())
            .count(),
    )?;
    let mut planned_roi_count = 0_u64;
    let mut planned_roi_pixels = 0_u64;
    for resource in template.resources().iter().filter(|resource| {
        !matches!(
            resource.kind,
            PlanResourceKind::External { .. } | PlanResourceKind::OutputTarget {}
        )
    }) {
        let roi = resolve_roi(&resource.roi, template.render_spec(), bindings)?;
        planned_roi_count = planned_roi_count
            .checked_add(1)
            .ok_or(InspectionError::CountOverflow)?;
        planned_roi_pixels = planned_roi_pixels
            .checked_add(roi.pixels())
            .ok_or(InspectionError::CountOverflow)?;
    }
    let resolve_passes = template
        .passes()
        .iter()
        .filter(|pass| matches!(pass.kind, ExecutionPassKind::ResolveRegion { .. }))
        .count();
    let direct_backdrop_views = template
        .passes()
        .iter()
        .filter(|pass| {
            matches!(
                pass.kind,
                ExecutionPassKind::BindBackdropView {
                    reason: ResourceAliasReason::DirectSampleableImmutableInput,
                    ..
                }
            )
        })
        .count();
    let reused_backdrop_resolves = template
        .passes()
        .iter()
        .filter(|pass| {
            matches!(
                pass.kind,
                ExecutionPassKind::BindBackdropView {
                    reason: ResourceAliasReason::ReusedBackdropResolve,
                    ..
                }
            )
        })
        .count();
    let elided_group_surfaces = template
        .passes()
        .iter()
        .filter(|pass| {
            matches!(
                pass.kind,
                ExecutionPassKind::AliasResource {
                    reason: ResourceAliasReason::NoOpGroup,
                    ..
                }
            )
        })
        .count();

    Ok(PlanInspectionCounts {
        logical_passes: count(graph.passes.len())?,
        logical_resources: count(graph.resources.len())?,
        logical_surfaces: count(
            graph
                .resources
                .iter()
                .filter(|resource| resource.texture.is_some())
                .count(),
        )?,
        execution_passes: count(template.passes().len())?,
        plan_resources: count(template.resources().len())?,
        outer_surface_slots: count(template.surface_slots().len())?,
        live_outer_surface_slots,
        declared_program_surface_slots,
        live_program_surface_slots,
        planned_roi_count,
        planned_roi_pixels,
        resolve_passes: count(resolve_passes)?,
        direct_backdrop_views: count(direct_backdrop_views)?,
        reused_backdrop_resolves: count(reused_backdrop_resolves)?,
        elided_group_surfaces: count(elided_group_surfaces)?,
    })
}

fn resolve_roi(
    roi: &GraphRoi,
    render_spec: RenderSpec,
    bindings: &RenderBindings,
) -> Result<DeviceRect, InspectionError> {
    match roi {
        GraphRoi::FullFrame => Ok(DeviceRect::full(render_spec.width(), render_spec.height())),
        GraphRoi::Static { rect } => Ok(*rect),
        GraphRoi::Dynamic { binding } => bindings
            .dynamic()
            .get(*binding)
            .and_then(|value| match value.value {
                DynamicValue::Bounds(bounds) => Some(bounds),
                _ => None,
            })
            .ok_or_else(|| InspectionError::InvalidArtifact {
                path: "plan.resources.roi".to_owned(),
                reason: format!("dynamic ROI {} is not a bound rectangle", binding.get()),
            }),
    }
}

fn count(value: usize) -> Result<u64, InspectionError> {
    u64::try_from(value).map_err(|_| InspectionError::CountOverflow)
}

fn invalid(path: impl Into<String>, error: impl std::fmt::Display) -> InspectionError {
    InspectionError::InvalidArtifact {
        path: path.into(),
        reason: error.to_string(),
    }
}

fn pass_stage_label(stage: PassStage) -> &'static str {
    match stage {
        PassStage::Visual => "visual",
        PassStage::Caption => "caption",
        PassStage::Output => "output",
    }
}

fn pass_kind_label(kind: &LogicalPassKind) -> &'static str {
    match kind {
        LogicalPassKind::ClearComposite { .. } => "clear",
        LogicalPassKind::Import { .. } => "import",
        LogicalPassKind::Draw { .. } => "draw",
        LogicalPassKind::Group { .. } => "group",
        LogicalPassKind::BackdropRead { .. } => "backdrop-read",
        LogicalPassKind::Filter { .. } => "filter",
        LogicalPassKind::Mask { .. } => "mask",
        LogicalPassKind::CompositeLayer { .. } => "composite",
        LogicalPassKind::Blend { .. } => "blend",
        LogicalPassKind::Transition { .. } => "transition",
        LogicalPassKind::AdjustmentEffect { .. } => "adjustment-effect",
        LogicalPassKind::Caption { .. } => "caption",
        LogicalPassKind::OutputTransform { .. } => "output-transform",
    }
}

fn resource_kind_label(kind: &GraphResourceKind) -> String {
    match kind {
        GraphResourceKind::ExternalResource { .. } => "external".to_owned(),
        GraphResourceKind::Layer { role } => format!("layer:{role:?}"),
        GraphResourceKind::Composite { version } => format!("composite:v{}", version.get()),
        GraphResourceKind::BackdropView { source_version, .. } => {
            format!("backdrop-view:v{}", source_version.get())
        }
        GraphResourceKind::Mask => "mask".to_owned(),
        GraphResourceKind::Auxiliary => "auxiliary".to_owned(),
        GraphResourceKind::HistorySlot { slot } => format!("history:{slot}"),
        GraphResourceKind::Output { .. } => "output".to_owned(),
    }
}

fn order_reason_label(reason: crate::compositor::graph::OrderReason) -> &'static str {
    use crate::compositor::graph::OrderReason;
    match reason {
        OrderReason::VisualSpine => "visual-spine",
        OrderReason::CaptionTerminal => "caption-terminal",
        OrderReason::OutputTerminal => "output-terminal",
    }
}

fn dot_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' | '\r' => escaped.push_str("\\n"),
            value if value.is_control() => escaped.push('?'),
            value => escaped.push(value),
        }
    }
    escaped
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum InspectionError {
    #[error("inspection artifacts do not describe the same admitted frame")]
    CrossArtifactMismatch,
    #[error("inspection artifact has the wrong schema version or kind")]
    WrongArtifactContract,
    #[error("inspection count overflow")]
    CountOverflow,
    #[error("{path}: {reason}")]
    InvalidArtifact { path: String, reason: String },
}
