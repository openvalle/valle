use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use valle_draw::{
    Rect,
    program::{
        BackdropScope, BlendMode, Clip, DrawProgram, FILTER_GAUSSIAN_SUPPORT_SIGMAS, Filter, Group,
        MaskMode, MotionGlassForegroundProgram, MotionGlassProgram, Node, NodeId, ShaderLayer,
        Transform2d,
    },
    requirements::{DestinationOperation, Insets, LocalBounds, SamplingMode},
};

use super::{PlanIdError, ProgramDestinationId, ProgramPassId, ProgramResourceId};

/// A deterministic, backend-free expansion of one DrawProgram into local SSA resources.
///
/// The outer RenderPlan owns Timeline resources. This nested plan owns only transparent local
/// contributions and immutable destination views; it can never address a platform object or a
/// Timeline resource directly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProgramPlan {
    resources: Vec<ProgramResource>,
    passes: Vec<ProgramPass>,
    output: ProgramResourceId,
}

impl ProgramPlan {
    pub fn resources(&self) -> &[ProgramResource] {
        &self.resources
    }

    pub fn passes(&self) -> &[ProgramPass] {
        &self.passes
    }

    pub const fn output(&self) -> ProgramResourceId {
        self.output
    }

    pub(crate) fn derive(program: &DrawProgram) -> Result<Self, ProgramPlanError> {
        Compiler::new(program).compile()
    }

    pub(crate) fn validate_shape(&self, destination_count: usize) -> Result<(), ProgramPlanError> {
        if self.resources.is_empty() || self.passes.is_empty() {
            return invalid("localPlan", "resources and passes must not be empty");
        }
        let mut writers = BTreeMap::new();
        let mut produced = BTreeSet::new();
        let mut paths = BTreeSet::new();
        let mut destinations = BTreeSet::new();
        for (index, resource) in self.resources.iter().enumerate() {
            if resource.id != ProgramResourceId::from_index(index)? {
                return invalid("localPlan.resources", "resource ids are not canonical");
            }
            if resource.local_to_program.inverse().is_none() {
                return invalid(
                    "localPlan.resources",
                    "resource coordinate transforms must be finite and invertible",
                );
            }
        }
        for (index, pass) in self.passes.iter().enumerate() {
            if pass.id != ProgramPassId::from_index(index)? || pass.semantic_path.is_empty() {
                return invalid(
                    "localPlan.passes",
                    "pass ids or semantic paths are not canonical",
                );
            }
            if !paths.insert(&pass.semantic_path) {
                return invalid("localPlan.passes", "pass semantic paths must be unique");
            }
            if let ProgramPassKind::ApplyMotionGlassForeground {
                owner_to_program, ..
            } = &pass.kind
                && owner_to_program.inverse().is_none()
            {
                return invalid(
                    &pass.semantic_path,
                    "Motion Glass foreground owner transform must be finite and invertible",
                );
            }
            let output = pass.kind.output();
            if self.resources.get(output.index()).is_none()
                || writers.insert(output, pass.id).is_some()
            {
                return invalid(
                    &pass.semantic_path,
                    "pass output is undefined or has more than one writer",
                );
            }
            for input in pass.kind.reads() {
                if input == output
                    || self.resources.get(input.index()).is_none()
                    || !produced.contains(&input)
                {
                    return invalid(
                        &pass.semantic_path,
                        "pass reads an unavailable or simultaneously written resource",
                    );
                }
            }
            if let ProgramPassKind::ReadDestination {
                external: Some(destination),
                ..
            } = pass.kind
                && !destinations.insert(destination)
            {
                return invalid(
                    &pass.semantic_path,
                    "external destination slots must have exactly one reader",
                );
            }
            produced.insert(output);
        }
        if writers.len() != self.resources.len()
            || self
                .passes
                .last()
                .is_none_or(|pass| pass.kind.output() != self.output)
            || self.resources[self.output.index()].local_to_program != Transform2d::IDENTITY
        {
            return invalid(
                "localPlan.output",
                "every resource needs one writer and the terminal output must use program coordinates",
            );
        }
        let expected_destinations = (0..destination_count)
            .map(ProgramDestinationId::from_index)
            .collect::<Result<BTreeSet<_>, _>>()?;
        if destinations != expected_destinations {
            return invalid(
                "localPlan.destinations",
                "external destination slots are not the exact contiguous layout",
            );
        }

        let mut reachable_resources = BTreeSet::new();
        let mut reachable_passes = BTreeSet::new();
        let mut pending = vec![self.output];
        while let Some(resource) = pending.pop() {
            if !reachable_resources.insert(resource) {
                continue;
            }
            if let Some(pass) = writers.get(&resource).copied()
                && reachable_passes.insert(pass)
            {
                pending.extend(self.passes[pass.index()].kind.reads());
            }
        }
        if reachable_resources.len() != self.resources.len()
            || reachable_passes.len() != self.passes.len()
        {
            return invalid(
                "localPlan.output",
                "every local pass and resource must reach the program output",
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProgramResource {
    pub id: ProgramResourceId,
    pub bounds: LocalBounds,
    /// Maps this resource's own coordinate space into the DrawProgram viewport space.
    /// Intermediate resources before an ancestor transform therefore retain exact local
    /// geometry without making the executor infer a hidden transform stack.
    pub local_to_program: Transform2d,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProgramDestinationKind {
    Backdrop,
    Blend,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProgramPass {
    pub id: ProgramPassId,
    pub semantic_path: String,
    pub kind: ProgramPassKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ProgramPassKind {
    Clear {
        output: ProgramResourceId,
    },
    RasterNode {
        node: NodeId,
        output: ProgramResourceId,
    },
    /// Ordered, destination-independent subtrees sharing one raster target. Groups inside these
    /// subtrees transform their children and isolate group opacity. Clips and other pixel
    /// operations remain separate passes. Native, Web and reference executors preserve
    /// nested transforms, group opacity and painter order within one destination.
    RasterTree {
        roots: Vec<NodeId>,
        output: ProgramResourceId,
    },
    ReadDestination {
        node: NodeId,
        operation: ProgramDestinationKind,
        scope: BackdropScope,
        external: Option<ProgramDestinationId>,
        /// Local source contributions in bottom-to-top order. They are composed over the
        /// immutable external input (or transparent when `external` is absent).
        local_inputs: Vec<ProgramResourceId>,
        output: ProgramResourceId,
    },
    Backdrop {
        node: NodeId,
        input: ProgramResourceId,
        output: ProgramResourceId,
        bounds: Rect,
        footprint: Insets,
        sampling: SamplingMode,
        filters: Vec<Filter>,
    },
    /// One digest-verified Motion Glass material contribution. `input` is the resolved Current
    /// backdrop sample; ordinary group children are composited afterward as foreground.
    MotionGlass {
        node: NodeId,
        input: ProgramResourceId,
        output: ProgramResourceId,
        program: MotionGlassProgram,
    },
    /// Clip one real ordinary subtree to its member's materialized shape. Protection/tone are
    /// validated owner metadata; no fake foreground pixels exist in this pass.
    ApplyMotionGlassForeground {
        node: NodeId,
        input: ProgramResourceId,
        output: ProgramResourceId,
        /// The material owner's coordinate space is authoritative for the packed foreground
        /// homography. A Field member can be nested under a different local transform, so the
        /// executor must not infer this matrix from the foreground output resource.
        owner_to_program: Transform2d,
        program: MotionGlassForegroundProgram,
    },
    SourceOver {
        source: ProgramResourceId,
        destination: ProgramResourceId,
        output: ProgramResourceId,
    },
    ApplyClip {
        node: NodeId,
        input: ProgramResourceId,
        output: ProgramResourceId,
        clip: Clip,
    },
    ApplyFilter {
        node: NodeId,
        filter_index: u32,
        input: ProgramResourceId,
        output: ProgramResourceId,
        filter: Filter,
    },
    ApplyMask {
        node: NodeId,
        input: ProgramResourceId,
        mask: ProgramResourceId,
        output: ProgramResourceId,
        mode: MaskMode,
    },
    ApplyOpacity {
        node: NodeId,
        input: ProgramResourceId,
        output: ProgramResourceId,
        opacity: f32,
    },
    ApplyShader {
        node: NodeId,
        input: ProgramResourceId,
        output: ProgramResourceId,
        shader: ShaderLayer,
    },
    ApplyTransform {
        node: NodeId,
        input: ProgramResourceId,
        output: ProgramResourceId,
        transform: Transform2d,
    },
    Blend {
        node: NodeId,
        source: ProgramResourceId,
        destination: ProgramResourceId,
        output: ProgramResourceId,
        mode: BlendMode,
    },
}

impl ProgramPassKind {
    pub fn reads(&self) -> Vec<ProgramResourceId> {
        match self {
            Self::Clear { .. } | Self::RasterNode { .. } | Self::RasterTree { .. } => Vec::new(),
            Self::ReadDestination { local_inputs, .. } => local_inputs.clone(),
            Self::Backdrop { input, .. }
            | Self::MotionGlass { input, .. }
            | Self::ApplyMotionGlassForeground { input, .. }
            | Self::ApplyClip { input, .. }
            | Self::ApplyFilter { input, .. }
            | Self::ApplyOpacity { input, .. }
            | Self::ApplyShader { input, .. }
            | Self::ApplyTransform { input, .. } => vec![*input],
            Self::SourceOver {
                source,
                destination,
                ..
            }
            | Self::Blend {
                source,
                destination,
                ..
            } => vec![*source, *destination],
            Self::ApplyMask { input, mask, .. } => vec![*input, *mask],
        }
    }

    pub const fn output(&self) -> ProgramResourceId {
        match self {
            Self::Clear { output }
            | Self::RasterNode { output, .. }
            | Self::RasterTree { output, .. }
            | Self::ReadDestination { output, .. }
            | Self::Backdrop { output, .. }
            | Self::MotionGlass { output, .. }
            | Self::ApplyMotionGlassForeground { output, .. }
            | Self::SourceOver { output, .. }
            | Self::ApplyClip { output, .. }
            | Self::ApplyFilter { output, .. }
            | Self::ApplyMask { output, .. }
            | Self::ApplyOpacity { output, .. }
            | Self::ApplyShader { output, .. }
            | Self::ApplyTransform { output, .. }
            | Self::Blend { output, .. } => *output,
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ProgramPlanError {
    #[error(transparent)]
    Id(#[from] PlanIdError),
    #[error("{path}: {reason}")]
    InvalidContract { path: String, reason: String },
}

struct Compiler<'a> {
    program: &'a DrawProgram,
    resources: Vec<ProgramResource>,
    passes: Vec<ProgramPass>,
    next_destination: usize,
    raster_subtrees: Vec<Option<bool>>,
}

impl<'a> Compiler<'a> {
    fn new(program: &'a DrawProgram) -> Self {
        Self {
            program,
            resources: Vec::new(),
            passes: Vec::new(),
            next_destination: 0,
            raster_subtrees: vec![None; program.nodes().len()],
        }
    }

    fn compile(mut self) -> Result<ProgramPlan, ProgramPlanError> {
        let roots = self.program.roots();
        let output = if roots.iter().all(|root| self.raster_subtree(*root)) {
            self.raster_tree(roots, Transform2d::IDENTITY, "root.rasterTree")?
        } else {
            let clear = self.clear("root.clear", Transform2d::IDENTITY)?;
            self.compose_nodes(roots, clear, &[], true, Transform2d::IDENTITY, None, "root")?
        };
        if self.next_destination != self.program.requirements().destination_uses.len() {
            return self.invalid(
                "destinationUses",
                "local plan did not consume the exact derived destination layout",
            );
        }
        let plan = ProgramPlan {
            resources: self.resources,
            passes: self.passes,
            output,
        };
        plan.validate_shape(self.program.requirements().destination_uses.len())?;
        Ok(plan)
    }

    fn raster_subtree(&mut self, id: NodeId) -> bool {
        let index = id.raw() as usize;
        if let Some(eligible) = self.raster_subtrees[index] {
            return eligible;
        }
        // DrawProgram validation already rejects cycles. Cache eligibility so revisiting a
        // subtree while splitting sibling runs does not repeat a full descendant traversal.
        let program = self.program;
        let eligible = match &program.nodes()[index] {
            Node::Group(group) => {
                group.is_raster_group()
                    && group
                        .children
                        .iter()
                        .all(|child| self.raster_subtree(*child))
            }
            _ => true,
        };
        self.raster_subtrees[index] = Some(eligible);
        eligible
    }

    fn raster_tree(
        &mut self,
        roots: &[NodeId],
        local_to_program: Transform2d,
        path: impl Into<String>,
    ) -> Result<ProgramResourceId, ProgramPlanError> {
        let mut bounds = LocalBounds::Empty;
        for root in roots {
            bounds = union(bounds, self.node_bounds(*root)?);
        }
        let output = self.resource(bounds, local_to_program)?;
        self.pass(
            path,
            ProgramPassKind::RasterTree {
                roots: roots.to_vec(),
                output,
            },
        )?;
        Ok(output)
    }

    #[allow(clippy::too_many_arguments)]
    fn compose_nodes(
        &mut self,
        nodes: &[NodeId],
        mut output: ProgramResourceId,
        base_prefix: &[ProgramResourceId],
        entry_is_external: bool,
        local_to_program: Transform2d,
        glass_owner_to_program: Option<Transform2d>,
        path: &str,
    ) -> Result<ProgramResourceId, ProgramPlanError> {
        let mut index = 0;
        while index < nodes.len() {
            let start = index;
            if self.raster_subtree(nodes[index]) {
                index += 1;
                while index < nodes.len() && self.raster_subtree(nodes[index]) {
                    index += 1;
                }
            } else {
                index += 1;
            }
            let source = if index - start > 1 {
                self.raster_tree(
                    &nodes[start..index],
                    local_to_program,
                    format!("{path}[{start}].rasterTree"),
                )?
            } else {
                let mut prefix = base_prefix.to_vec();
                prefix.extend(self.nonempty_prefix(&[output]));
                self.node(
                    nodes[start],
                    &prefix,
                    entry_is_external,
                    local_to_program,
                    glass_owner_to_program,
                )?
            };
            output = self.source_over(source, output, format!("{path}[{start}].composite"))?;
        }
        Ok(output)
    }

    fn node(
        &mut self,
        id: NodeId,
        parent_prefix: &[ProgramResourceId],
        entry_is_external: bool,
        parent_to_program: Transform2d,
        glass_owner_to_program: Option<Transform2d>,
    ) -> Result<ProgramResourceId, ProgramPlanError> {
        let node = self
            .program
            .nodes()
            .get(id.raw() as usize)
            .cloned()
            .ok_or_else(|| ProgramPlanError::InvalidContract {
                path: format!("node[{}]", id.raw()),
                reason: "node id is undefined".to_owned(),
            })?;
        match node {
            Node::Group(_) if self.raster_subtree(id) => self.raster_tree(
                &[id],
                parent_to_program,
                format!("node[{}].rasterTree", id.raw()),
            ),
            Node::Group(group) => self.group(
                id,
                group,
                parent_prefix,
                entry_is_external,
                parent_to_program,
                glass_owner_to_program,
            ),
            _ => {
                let bounds = self.node_bounds(id)?;
                let output = self.resource(bounds, parent_to_program)?;
                self.pass(
                    format!("node[{}].raster", id.raw()),
                    ProgramPassKind::RasterNode { node: id, output },
                )?;
                Ok(output)
            }
        }
    }

    fn group(
        &mut self,
        id: NodeId,
        group: Group,
        parent_prefix: &[ProgramResourceId],
        entry_is_external: bool,
        parent_to_program: Transform2d,
        glass_owner_to_program: Option<Transform2d>,
    ) -> Result<ProgramResourceId, ProgramPlanError> {
        let node_path = format!("node[{}]", id.raw());
        let expected_bounds = self.node_bounds(id)?;
        let local_to_program = group.transform.then(parent_to_program);
        let child_glass_owner_to_program = if group.glass.is_some() {
            Some(local_to_program)
        } else {
            glass_owner_to_program
        };
        let group_is_visible = expected_bounds.rect().is_some();
        let child_entry_is_external = entry_is_external && !group.isolated;
        let base_prefix = if group.isolated {
            Vec::new()
        } else {
            parent_prefix.to_vec()
        };
        let mut output = self.clear(format!("{node_path}.clear"), local_to_program)?;

        if let Some(glass) = group.glass.as_deref()
            && group_is_visible
        {
            let footprint = glass_footprint(glass);
            let operation = DestinationOperation::Backdrop {
                footprint,
                sampling: SamplingMode::LinearClamp,
            };
            let mut local_inputs = base_prefix.clone();
            local_inputs.extend(self.nonempty_prefix(&[output]));
            let destination = self.read_destination(
                id,
                ProgramDestinationKind::Backdrop,
                glass.backdrop.scope.clone(),
                operation,
                glass.backdrop.output_bounds,
                LocalBounds::from_rect(glass.backdrop.sample_bounds),
                local_to_program,
                entry_is_external,
                local_inputs,
                format!("{node_path}.glass.destination"),
            )?;
            let source = self.resource(
                LocalBounds::from_rect(glass.backdrop.output_bounds),
                local_to_program,
            )?;
            self.pass(
                format!("{node_path}.glass"),
                ProgramPassKind::MotionGlass {
                    node: id,
                    input: destination,
                    output: source,
                    program: glass.clone(),
                },
            )?;
            output = self.source_over(source, output, format!("{node_path}.glass.composite"))?;
        }

        if let Some(backdrop) = &group.backdrop
            && group_is_visible
            && !backdrop.bounds.is_empty()
        {
            let operation = DestinationOperation::Backdrop {
                footprint: backdrop.footprint,
                sampling: backdrop.sampling,
            };
            let sample_bounds = LocalBounds::from_rect(outset(backdrop.bounds, backdrop.footprint));
            let local_inputs = match backdrop.scope {
                BackdropScope::Current => {
                    let mut inputs = base_prefix.clone();
                    inputs.extend(self.nonempty_prefix(&[output]));
                    inputs
                }
                BackdropScope::LayerEntry(_) | BackdropScope::ScopeEntry(_) => Vec::new(),
            };
            let external = match backdrop.scope {
                BackdropScope::Current => child_entry_is_external,
                BackdropScope::LayerEntry(_) | BackdropScope::ScopeEntry(_) => true,
            };
            let destination = self.read_destination(
                id,
                ProgramDestinationKind::Backdrop,
                backdrop.scope.clone(),
                operation,
                backdrop.bounds,
                sample_bounds,
                local_to_program,
                external,
                local_inputs,
                format!("{node_path}.backdrop.destination"),
            )?;
            let source =
                self.resource(LocalBounds::from_rect(backdrop.bounds), local_to_program)?;
            self.pass(
                format!("{node_path}.backdrop"),
                ProgramPassKind::Backdrop {
                    node: id,
                    input: destination,
                    output: source,
                    bounds: backdrop.bounds,
                    footprint: backdrop.footprint,
                    sampling: backdrop.sampling,
                    filters: backdrop.filters.clone(),
                },
            )?;
            output = self.source_over(source, output, format!("{node_path}.backdrop.composite"))?;
        }

        output = self.compose_nodes(
            &group.children,
            output,
            &base_prefix,
            child_entry_is_external,
            local_to_program,
            child_glass_owner_to_program,
            &format!("{node_path}.children"),
        )?;

        if let Some(foreground) = group.glass_foreground.as_deref() {
            let owner_to_program =
                glass_owner_to_program.ok_or_else(|| ProgramPlanError::InvalidContract {
                    path: format!("{node_path}.glassForeground"),
                    reason: "foreground has no active material owner coordinate space".to_owned(),
                })?;
            let program_to_local =
                local_to_program
                    .inverse()
                    .ok_or_else(|| ProgramPlanError::InvalidContract {
                        path: format!("{node_path}.glassForeground"),
                        reason: "foreground local transform is singular".to_owned(),
                    })?;
            let owner_to_local = owner_to_program.then(program_to_local);
            let surface_to_local =
                Transform2d(foreground.local_to_owner.map(f64::from)).then(owner_to_local);
            let bounds = if foreground.presence == 0.0 {
                LocalBounds::Empty
            } else {
                let shape = surface_to_local
                    .map_bounds(foreground.rect)
                    .ok_or_else(|| ProgramPlanError::InvalidContract {
                        path: format!("{node_path}.glassForeground"),
                        reason: "foreground shape crosses a projective horizon".to_owned(),
                    })?;
                intersect(self.bounds(output), LocalBounds::from_rect(shape))
            };
            let clipped = self.resource(bounds, local_to_program)?;
            self.pass(
                format!("{node_path}.glassForeground"),
                ProgramPassKind::ApplyMotionGlassForeground {
                    node: id,
                    input: output,
                    output: clipped,
                    owner_to_program,
                    program: foreground.clone(),
                },
            )?;
            output = clipped;
        }

        if let Some(clip) = &group.clip {
            let bounds = intersect(self.bounds(output), clip_bounds(self.program, clip));
            let clipped = self.resource(bounds, local_to_program)?;
            self.pass(
                format!("{node_path}.clip"),
                ProgramPassKind::ApplyClip {
                    node: id,
                    input: output,
                    output: clipped,
                    clip: clip.clone(),
                },
            )?;
            output = clipped;
        }

        for (index, filter) in group.filters.iter().cloned().enumerate() {
            let bounds = program_filter_bounds(self.bounds(output), &filter);
            let filtered = self.resource(bounds, local_to_program)?;
            let filter_index = u32::try_from(index).map_err(|_| PlanIdError::BudgetExceeded {
                kind: "program filter",
            })?;
            self.pass(
                format!("{node_path}.filters[{index}]"),
                ProgramPassKind::ApplyFilter {
                    node: id,
                    filter_index,
                    input: output,
                    output: filtered,
                    filter,
                },
            )?;
            output = filtered;
        }

        if let Some(mask) = &group.mask {
            let mask_source = self.node(
                mask.source,
                &[],
                false,
                local_to_program,
                child_glass_owner_to_program,
            )?;
            let masked = self.resource(
                intersect(self.bounds(output), self.bounds(mask_source)),
                local_to_program,
            )?;
            self.pass(
                format!("{node_path}.mask"),
                ProgramPassKind::ApplyMask {
                    node: id,
                    input: output,
                    mask: mask_source,
                    output: masked,
                    mode: mask.mode,
                },
            )?;
            output = masked;
        }

        if group.opacity != 1.0 {
            let bounds = if group.opacity == 0.0 {
                LocalBounds::Empty
            } else {
                self.bounds(output)
            };
            let faded = self.resource(bounds, local_to_program)?;
            self.pass(
                format!("{node_path}.opacity"),
                ProgramPassKind::ApplyOpacity {
                    node: id,
                    input: output,
                    output: faded,
                    opacity: group.opacity,
                },
            )?;
            output = faded;
        }

        if let Some(shader) = &group.shader {
            let bounds = if self.bounds(output).rect().is_some() {
                LocalBounds::from_rect(shader.bounds)
            } else {
                LocalBounds::Empty
            };
            let shaded = self.resource(bounds, local_to_program)?;
            self.pass(
                format!("{node_path}.shader"),
                ProgramPassKind::ApplyShader {
                    node: id,
                    input: output,
                    output: shaded,
                    shader: shader.clone(),
                },
            )?;
            output = shaded;
        }

        if group.transform != Transform2d::IDENTITY {
            let bounds = self.node_bounds(id)?;
            let transformed = self.resource(bounds, parent_to_program)?;
            self.pass(
                format!("{node_path}.transform"),
                ProgramPassKind::ApplyTransform {
                    node: id,
                    input: output,
                    output: transformed,
                    transform: group.transform,
                },
            )?;
            output = transformed;
        }

        if group.internal_blend != BlendMode::Normal
            && let Some(rect) = self.bounds(output).rect()
        {
            let bounds = self.bounds(output);
            let destination = self.read_destination(
                id,
                ProgramDestinationKind::Blend,
                BackdropScope::Current,
                DestinationOperation::Blend {
                    mode: group.internal_blend,
                },
                rect,
                bounds,
                parent_to_program,
                entry_is_external,
                parent_prefix.to_vec(),
                format!("{node_path}.blend.destination"),
            )?;
            let blended = self.resource(bounds, parent_to_program)?;
            self.pass(
                format!("{node_path}.blend"),
                ProgramPassKind::Blend {
                    node: id,
                    source: output,
                    destination,
                    output: blended,
                    mode: group.internal_blend,
                },
            )?;
            output = blended;
        }

        if self.bounds(output) != expected_bounds {
            return self.invalid(
                node_path,
                "local pass bounds do not reproduce validator-derived NodeGeometry",
            );
        }
        Ok(output)
    }

    #[allow(clippy::too_many_arguments)]
    fn read_destination(
        &mut self,
        node: NodeId,
        kind: ProgramDestinationKind,
        scope: BackdropScope,
        operation: DestinationOperation,
        output_bounds: Rect,
        sample_bounds: LocalBounds,
        local_to_program: Transform2d,
        external: bool,
        local_inputs: Vec<ProgramResourceId>,
        semantic_path: String,
    ) -> Result<ProgramResourceId, ProgramPlanError> {
        let external = if external {
            let index = self.next_destination;
            let Some(required) = self.program.requirements().destination_uses.get(index) else {
                return self.invalid(
                    &semantic_path,
                    "local plan requests an undeclared external destination",
                );
            };
            let required_output = local_to_program.map_bounds(output_bounds).ok_or_else(|| {
                ProgramPlanError::InvalidContract {
                    path: semantic_path.clone(),
                    reason: "destination output crosses a projective vanishing line".to_owned(),
                }
            })?;
            let required_sample = sample_bounds
                .rect()
                .and_then(|bounds| local_to_program.map_bounds(bounds));
            if required.node != node
                || required.scope != scope
                || required.output_bounds != required_output
                || required_sample != Some(required.sample_bounds)
                || required.operation != operation
            {
                return self.invalid(
                    &semantic_path,
                    "local destination does not match DrawRequirements",
                );
            }
            self.next_destination =
                self.next_destination
                    .checked_add(1)
                    .ok_or(PlanIdError::BudgetExceeded {
                        kind: "program destination",
                    })?;
            Some(ProgramDestinationId::from_index(index)?)
        } else {
            None
        };
        let output = self.resource(sample_bounds, local_to_program)?;
        self.pass(
            semantic_path,
            ProgramPassKind::ReadDestination {
                node,
                operation: kind,
                scope,
                external,
                local_inputs,
                output,
            },
        )?;
        Ok(output)
    }

    fn clear(
        &mut self,
        semantic_path: impl Into<String>,
        local_to_program: Transform2d,
    ) -> Result<ProgramResourceId, ProgramPlanError> {
        let output = self.resource(LocalBounds::Empty, local_to_program)?;
        self.pass(semantic_path, ProgramPassKind::Clear { output })?;
        Ok(output)
    }

    fn source_over(
        &mut self,
        source: ProgramResourceId,
        destination: ProgramResourceId,
        semantic_path: impl Into<String>,
    ) -> Result<ProgramResourceId, ProgramPlanError> {
        let semantic_path = semantic_path.into();
        let source_transform = self.local_to_program(source);
        let destination_transform = self.local_to_program(destination);
        if source_transform != destination_transform {
            return self.invalid(
                semantic_path,
                "source-over inputs belong to different coordinate spaces",
            );
        }
        let output = self.resource(
            union(self.bounds(source), self.bounds(destination)),
            source_transform,
        )?;
        self.pass(
            semantic_path,
            ProgramPassKind::SourceOver {
                source,
                destination,
                output,
            },
        )?;
        Ok(output)
    }

    fn resource(
        &mut self,
        bounds: LocalBounds,
        local_to_program: Transform2d,
    ) -> Result<ProgramResourceId, ProgramPlanError> {
        let id = ProgramResourceId::from_index(self.resources.len())?;
        self.resources.push(ProgramResource {
            id,
            bounds,
            local_to_program,
        });
        Ok(id)
    }

    fn pass(
        &mut self,
        semantic_path: impl Into<String>,
        kind: ProgramPassKind,
    ) -> Result<ProgramPassId, ProgramPlanError> {
        let id = ProgramPassId::from_index(self.passes.len())?;
        self.passes.push(ProgramPass {
            id,
            semantic_path: semantic_path.into(),
            kind,
        });
        Ok(id)
    }

    fn bounds(&self, id: ProgramResourceId) -> LocalBounds {
        self.resources[id.index()].bounds
    }

    fn local_to_program(&self, id: ProgramResourceId) -> Transform2d {
        self.resources[id.index()].local_to_program
    }

    fn node_bounds(&self, id: NodeId) -> Result<LocalBounds, ProgramPlanError> {
        self.program
            .geometry(id)
            .map(|geometry| geometry.output_bounds)
            .ok_or_else(|| ProgramPlanError::InvalidContract {
                path: format!("node[{}]", id.raw()),
                reason: "node geometry is undefined".to_owned(),
            })
    }

    fn nonempty_prefix(&self, resources: &[ProgramResourceId]) -> Vec<ProgramResourceId> {
        resources
            .iter()
            .copied()
            .filter(|resource| self.bounds(*resource).rect().is_some())
            .collect()
    }

    fn invalid<T>(
        &self,
        path: impl Into<String>,
        reason: impl Into<String>,
    ) -> Result<T, ProgramPlanError> {
        Err(ProgramPlanError::InvalidContract {
            path: path.into(),
            reason: reason.into(),
        })
    }
}

fn union(left: LocalBounds, right: LocalBounds) -> LocalBounds {
    match (left.rect(), right.rect()) {
        (None, None) => LocalBounds::Empty,
        (Some(rect), None) | (None, Some(rect)) => LocalBounds::from_rect(rect),
        (Some(left), Some(right)) => LocalBounds::from_rect(Rect::from_edges(
            left.left().min(right.left()),
            left.top().min(right.top()),
            left.right().max(right.right()),
            left.bottom().max(right.bottom()),
        )),
    }
}

fn intersect(left: LocalBounds, right: LocalBounds) -> LocalBounds {
    let (Some(left), Some(right)) = (left.rect(), right.rect()) else {
        return LocalBounds::Empty;
    };
    LocalBounds::from_rect(Rect::from_edges(
        left.left().max(right.left()),
        left.top().max(right.top()),
        left.right().min(right.right()),
        left.bottom().min(right.bottom()),
    ))
}

fn outset(rect: Rect, insets: Insets) -> Rect {
    Rect::from_edges(
        rect.left() - f64::from(insets.left),
        rect.top() - f64::from(insets.top),
        rect.right() + f64::from(insets.right),
        rect.bottom() + f64::from(insets.bottom),
    )
}

fn glass_footprint(glass: &MotionGlassProgram) -> Insets {
    let output = glass.backdrop.output_bounds;
    let sample = glass.backdrop.sample_bounds;
    Insets::new(
        (output.left() - sample.left()).max(0.0) as f32,
        (output.top() - sample.top()).max(0.0) as f32,
        (sample.right() - output.right()).max(0.0) as f32,
        (sample.bottom() - output.bottom()).max(0.0) as f32,
    )
}

fn clip_bounds(program: &DrawProgram, clip: &Clip) -> LocalBounds {
    match clip {
        Clip::Rect(rect) => LocalBounds::from_rect(*rect),
        Clip::RoundRect(round_rect) => LocalBounds::from_rect(round_rect.rect),
        Clip::Path { path, .. } => {
            let Some(path) = program.paths().get(path.raw() as usize) else {
                return LocalBounds::Empty;
            };
            let Some([first_x, first_y]) = path.points.first().copied() else {
                return LocalBounds::Empty;
            };
            let mut left = first_x;
            let mut top = first_y;
            let mut right = first_x;
            let mut bottom = first_y;
            for [x, y] in path.points.iter().copied().skip(1) {
                left = left.min(x);
                top = top.min(y);
                right = right.max(x);
                bottom = bottom.max(y);
            }
            LocalBounds::from_rect(Rect::from_edges(left, top, right, bottom))
        }
    }
}

fn program_filter_bounds(bounds: LocalBounds, filter: &Filter) -> LocalBounds {
    let Some(rect) = bounds.rect() else {
        return LocalBounds::Empty;
    };
    match filter {
        Filter::Blur { sigma_x, sigma_y } => LocalBounds::from_rect(outset(
            rect,
            Insets::new(
                FILTER_GAUSSIAN_SUPPORT_SIGMAS * *sigma_x,
                FILTER_GAUSSIAN_SUPPORT_SIGMAS * *sigma_y,
                FILTER_GAUSSIAN_SUPPORT_SIGMAS * *sigma_x,
                FILTER_GAUSSIAN_SUPPORT_SIGMAS * *sigma_y,
            ),
        )),
        Filter::DropShadow {
            offset,
            sigma_x,
            sigma_y,
            ..
        } => {
            let shadow = Rect::from_edges(
                rect.left() + f64::from(offset[0] - FILTER_GAUSSIAN_SUPPORT_SIGMAS * *sigma_x),
                rect.top() + f64::from(offset[1] - FILTER_GAUSSIAN_SUPPORT_SIGMAS * *sigma_y),
                rect.right() + f64::from(offset[0] + FILTER_GAUSSIAN_SUPPORT_SIGMAS * *sigma_x),
                rect.bottom() + f64::from(offset[1] + FILTER_GAUSSIAN_SUPPORT_SIGMAS * *sigma_y),
            );
            union(LocalBounds::from_rect(rect), LocalBounds::from_rect(shadow))
        }
        Filter::NoiseDisplacement { scale, .. } => {
            LocalBounds::from_rect(outset(rect, Insets::uniform(*scale)))
        }
        Filter::VelocityBlur {
            velocity,
            shutter_angle_degrees,
        } => {
            let scale = *shutter_angle_degrees / 360.0;
            let dx = velocity[0] * scale;
            let dy = velocity[1] * scale;
            LocalBounds::from_rect(Rect::from_edges(
                rect.left() + f64::from(dx.min(0.0)),
                rect.top() + f64::from(dy.min(0.0)),
                rect.right() + f64::from(dx.max(0.0)),
                rect.bottom() + f64::from(dy.max(0.0)),
            ))
        }
        Filter::ColorMatrix { .. }
        | Filter::Brightness { .. }
        | Filter::Contrast { .. }
        | Filter::Grayscale { .. }
        | Filter::HueRotate { .. }
        | Filter::Invert { .. }
        | Filter::Opacity { .. }
        | Filter::Saturate { .. }
        | Filter::Sepia { .. } => LocalBounds::from_rect(rect),
    }
}

fn invalid<T>(path: impl Into<String>, reason: impl Into<String>) -> Result<T, ProgramPlanError> {
    Err(ProgramPlanError::InvalidContract {
        path: path.into(),
        reason: reason.into(),
    })
}

#[cfg(test)]
mod tests {
    use valle_draw::program::{
        BackdropRead, BackdropScope, DrawProgramBuilder, LinearColor, Mask, Node, Paint, PathData,
        PathNode, PathVerb,
        glass::{
            BackdropUse, GlassOwnerKind, MOTION_GLASS_KERNEL_ID, MotionGlassForegroundProgram,
            PackedGlassField, PackedGlassForegroundTone, PackedGlassMaterial, PackedGlassMotion,
            PackedGlassShapeKind, PackedGlassSurface, motion_glass_kernel_digest, schema_digest,
        },
    };

    use super::*;

    fn rect_path(builder: &mut DrawProgramBuilder, rect: Rect, color: LinearColor) -> NodeId {
        let paint = builder.push_paint(Paint::Solid(color));
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

    fn field_owner_program() -> MotionGlassProgram {
        MotionGlassProgram {
            owner_kind: GlassOwnerKind::Field,
            owner_id: "pair".into(),
            surfaces: vec![PackedGlassSurface {
                surface_id: "member".into(),
                shape: PackedGlassShapeKind::ContinuousRect,
                rect: Rect::new(0.0, 0.0, 1.0, 1.0),
                // Normalized member coordinates -> Field owner coordinates.
                local_to_owner: [30.0, 0.0, 20.0, 0.0, 20.0, 10.0, 0.0, 0.0, 1.0],
                radius: 0.2,
                presence: 1.0,
                response: PackedGlassMotion::ZERO,
                path_points: Vec::new(),
                foreground_tone: PackedGlassForegroundTone::None,
                foreground_protection: 0.0,
                foreground_bounds: None,
                foreground_luma: None,
            }],
            field: Some(PackedGlassField {
                field_id: "pair".into(),
                merge_distance: 4.0,
                member_ids: vec!["member".into()],
            }),
            material: PackedGlassMaterial {
                bevel_width: 4.0,
                thickness: 8.0,
                refractive_index: 1.18,
                roughness: 0.08,
                dispersion: 0.01,
                tint_linear: [0.2, 0.3, 0.8, 0.2],
                specular_strength: 0.45,
                shadow_strength: 0.1,
                foreground_gain: 0.0,
                light: valle_draw::program::PackedGlassLight::DEFAULT,
            },
            backdrop: BackdropUse {
                scope: BackdropScope::ScopeEntry("pair".into()),
                output_bounds: Rect::new(0.0, 0.0, 70.0, 50.0),
                sample_bounds: Rect::new(-10.0, -10.0, 90.0, 70.0),
            },
            kernel_id: MOTION_GLASS_KERNEL_ID.into(),
            kernel_digest: motion_glass_kernel_digest(),
            schema_digest: schema_digest(),
        }
    }

    #[test]
    fn field_foreground_uses_owner_space_while_keeping_member_local_bounds() {
        let owner = field_owner_program();
        let foreground = MotionGlassForegroundProgram::from_owner(&owner, "member").unwrap();
        let mut builder = DrawProgramBuilder::new(Rect::new(0.0, 0.0, 128.0, 72.0));
        let child = rect_path(
            &mut builder,
            Rect::new(0.0, 0.0, 30.0, 20.0),
            LinearColor::new(0.9, 0.8, 0.7, 1.0),
        );
        let mut foreground_group = Group::plain(vec![child]);
        foreground_group.glass_foreground = Some(Box::new(foreground));
        let foreground_node = builder.push_node(Node::Group(foreground_group));

        let member_to_owner = Transform2d([1.0, 0.0, 20.0, 0.0, 1.0, 10.0, 0.0, 0.0, 1.0]);
        let mut member_group = Group::plain(vec![foreground_node]);
        member_group.transform = member_to_owner;
        let member_node = builder.push_node(Node::Group(member_group));

        let mut owner_group = Group::plain(vec![member_node]);
        owner_group.glass = Some(Box::new(owner));
        let owner_node = builder.push_node(Node::Group(owner_group));

        let owner_to_program = Transform2d([1.0, 0.0, 4.0, 0.0, 1.0, 3.0, 0.0, 0.0, 1.0]);
        let mut outer_group = Group::plain(vec![owner_node]);
        outer_group.transform = owner_to_program;
        let root = builder.push_node(Node::Group(outer_group));
        builder.add_root(root);
        let program = builder.finish().unwrap();
        let foreground_node = program
            .nodes()
            .iter()
            .position(|node| matches!(node, Node::Group(group) if group.glass_foreground.is_some()))
            .map(|index| NodeId::from_raw(u32::try_from(index).unwrap()))
            .expect("canonical foreground node");

        assert_eq!(
            program.geometry(foreground_node).unwrap().output_bounds,
            LocalBounds::from_rect(Rect::new(0.0, 0.0, 30.0, 20.0)),
            "normalized material geometry must be projected back into the member's local pixels",
        );

        let plan = ProgramPlan::derive(&program).unwrap();
        let (output, admitted_owner) = plan
            .passes()
            .iter()
            .find_map(|pass| match &pass.kind {
                ProgramPassKind::ApplyMotionGlassForeground {
                    node,
                    output,
                    owner_to_program,
                    ..
                } if *node == foreground_node => Some((*output, *owner_to_program)),
                _ => None,
            })
            .expect("foreground pass");
        assert_eq!(admitted_owner, owner_to_program);
        assert_eq!(
            plan.resources()[output.index()].bounds,
            LocalBounds::from_rect(Rect::new(0.0, 0.0, 30.0, 20.0)),
        );
        assert_eq!(
            plan.resources()[output.index()].local_to_program,
            member_to_owner.then(owner_to_program),
            "the foreground image remains in member-local coordinates even though its kernel uses the Field owner",
        );
    }

    #[test]
    fn destination_independent_leaf_roots_share_one_explicit_raster_tree_pass() {
        let mut builder = DrawProgramBuilder::new(Rect::new(0.0, 0.0, 32.0, 18.0));
        let lower = rect_path(
            &mut builder,
            Rect::new(1.0, 2.0, 8.0, 6.0),
            LinearColor::new(0.5, 0.0, 0.0, 1.0),
        );
        let upper = rect_path(
            &mut builder,
            Rect::new(10.0, 4.0, 12.0, 9.0),
            LinearColor::new(0.0, 0.5, 0.0, 0.75),
        );
        builder.add_root(lower);
        builder.add_root(upper);
        let program = builder.finish().unwrap();

        let plan = ProgramPlan::derive(&program).unwrap();
        assert_eq!(plan.passes().len(), 1);
        assert!(matches!(
            &plan.passes()[0].kind,
            ProgramPassKind::RasterTree { roots, output }
                if roots == &[lower, upper] && *output == plan.output()
        ));
        assert_eq!(
            plan.resources()[plan.output().index()].bounds,
            union(
                program.geometry(lower).unwrap().output_bounds,
                program.geometry(upper).unwrap().output_bounds,
            )
        );
    }

    fn transformed_leaf(builder: &mut DrawProgramBuilder, x: f64) -> NodeId {
        let leaf = rect_path(
            builder,
            Rect::new(0.0, 0.0, 2.0, 8.0),
            LinearColor::new(0.25, 0.0, 0.0, 0.5),
        );
        let mut group = Group::plain(vec![leaf]);
        group.transform = Transform2d([1.0, 0.0, x, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
        builder.push_node(Node::Group(group))
    }

    #[test]
    fn dense_transformed_tree_uses_one_pass_and_no_scratch_surfaces() {
        use super::super::ProgramSchedule;
        let mut builder = DrawProgramBuilder::new(Rect::new(0.0, 0.0, 2400.0, 18.0));
        let children = (0..1200)
            .map(|index| {
                let leaf = rect_path(
                    &mut builder,
                    Rect::new(0.0, 0.0, 1.0, 8.0),
                    LinearColor::new(0.25, 0.0, 0.0, 0.5),
                );
                let mut group = Group::plain(vec![leaf]);
                group.transform =
                    Transform2d([1.0, 0.0, f64::from(index), 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
                group.isolated = index % 2 == 0;
                builder.push_node(Node::Group(group))
            })
            .collect();
        let mut outer = Group::plain(children);
        outer.transform = Transform2d([2.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 1.0]);
        let root = builder.push_node(Node::Group(outer));
        builder.add_root(root);
        let program = builder.finish().unwrap();
        let plan = ProgramPlan::derive(&program).unwrap();
        assert_eq!(plan.passes().len(), 1);
        assert_eq!(plan.resources().len(), 1);
        assert!(
            matches!(&plan.passes()[0].kind, ProgramPassKind::RasterTree { roots, .. }
            if roots == program.roots())
        );
        assert_eq!(
            plan.resources()[0].bounds,
            LocalBounds::from_rect(Rect::new(0.0, 0.0, 2400.0, 16.0))
        );
        assert!(
            ProgramSchedule::derive(&plan)
                .unwrap()
                .surface_slots()
                .is_empty()
        );
    }

    #[test]
    fn ordinary_runs_fuse_on_both_sides_and_inside_pixel_boundaries() {
        let mut builder = DrawProgramBuilder::new(Rect::new(0.0, 0.0, 64.0, 18.0));
        for x in [0.0, 2.0] {
            let node = transformed_leaf(&mut builder, x);
            builder.add_root(node);
        }
        let children = [4.0, 6.0]
            .map(|x| transformed_leaf(&mut builder, x))
            .to_vec();
        let mut effect = Group::plain(children);
        effect.clip = Some(Clip::Rect(Rect::new(4.5, 0.0, 3.0, 8.0)));
        effect.filters.push(Filter::Blur {
            sigma_x: 1.0,
            sigma_y: 1.0,
        });
        effect.opacity = 0.5;
        let node = builder.push_node(Node::Group(effect));
        builder.add_root(node);
        for x in [8.0, 10.0] {
            let node = transformed_leaf(&mut builder, x);
            builder.add_root(node);
        }
        let program = builder.finish().unwrap();
        let plan = ProgramPlan::derive(&program).unwrap();
        let operations = plan
            .passes()
            .iter()
            .filter_map(|pass| match &pass.kind {
                ProgramPassKind::RasterTree { roots, .. } => {
                    assert_eq!(roots.len(), 2);
                    Some("raster")
                }
                ProgramPassKind::ApplyClip { .. } => Some("clip"),
                ProgramPassKind::ApplyFilter { .. } => Some("filter"),
                ProgramPassKind::ApplyOpacity { .. } => Some("opacity"),
                ProgramPassKind::ApplyTransform { .. } => {
                    panic!("simple transforms belong in RasterTree")
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            operations,
            ["raster", "raster", "clip", "filter", "opacity", "raster"]
        );
    }

    #[test]
    fn destination_reads_keep_the_fused_prefix_and_exclude_later_siblings() {
        let mut builder = DrawProgramBuilder::new(Rect::new(0.0, 0.0, 64.0, 18.0));
        for x in [0.0, 2.0] {
            let node = transformed_leaf(&mut builder, x);
            builder.add_root(node);
        }
        let mut group = Group::plain(Vec::new());
        group.backdrop = Some(BackdropRead {
            scope: BackdropScope::Current,
            bounds: Rect::new(0.0, 0.0, 64.0, 18.0),
            footprint: Insets::uniform(0.0),
            sampling: SamplingMode::LinearClamp,
            filters: Vec::new(),
        });
        let node = builder.push_node(Node::Group(group));
        builder.add_root(node);
        for x in [40.0, 42.0] {
            let node = transformed_leaf(&mut builder, x);
            builder.add_root(node);
        }
        let program = builder.finish().unwrap();
        let plan = ProgramPlan::derive(&program).unwrap();
        plan.validate_shape(1).unwrap();
        let mut reads = 0;
        for pass in plan.passes() {
            if let ProgramPassKind::ReadDestination {
                external,
                local_inputs,
                ..
            } = &pass.kind
            {
                reads += 1;
                assert!(external.is_some());
                assert_eq!(local_inputs.len(), 1);
                assert_eq!(
                    plan.resources()[local_inputs[0].index()].bounds,
                    LocalBounds::from_rect(Rect::new(0.0, 0.0, 4.0, 8.0))
                );
            }
        }
        assert_eq!(reads, 1);
        assert_eq!(
            plan.passes()
                .iter()
                .filter(|pass| matches!(pass.kind, ProgramPassKind::RasterTree { .. }))
                .count(),
            2
        );
    }

    #[test]
    fn styled_group_expands_to_ordered_local_ssa_with_exact_destination_slots() {
        let mut builder = DrawProgramBuilder::new(Rect::new(0.0, 0.0, 32.0, 18.0));
        let child = rect_path(
            &mut builder,
            Rect::new(4.0, 3.0, 12.0, 8.0),
            LinearColor::new(0.5, 0.25, 0.1, 1.0),
        );
        let mask = rect_path(
            &mut builder,
            Rect::new(3.0, 2.0, 16.0, 10.0),
            LinearColor::new(1.0, 1.0, 1.0, 1.0),
        );
        let mut group = Group::plain(vec![child]);
        group.clip = Some(Clip::Rect(Rect::new(2.0, 1.0, 20.0, 14.0)));
        group.filters = vec![
            Filter::Blur {
                sigma_x: 1.0,
                sigma_y: 2.0,
            },
            Filter::ColorMatrix {
                matrix: Box::new([
                    1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0,
                    0.0, 0.0, 1.0, 0.0,
                ]),
            },
        ];
        group.mask = Some(Mask {
            source: mask,
            mode: MaskMode::Alpha,
        });
        group.opacity = 0.5;
        group.internal_blend = BlendMode::Screen;
        group.backdrop = Some(BackdropRead {
            scope: BackdropScope::Current,
            bounds: Rect::new(4.0, 3.0, 12.0, 8.0),
            footprint: Insets::uniform(1.0),
            sampling: valle_draw::requirements::SamplingMode::LinearClamp,
            filters: Vec::new(),
        });
        let root = builder.push_node(Node::Group(group));
        builder.add_root(root);
        let program = builder.finish().unwrap();

        let plan = ProgramPlan::derive(&program).unwrap();
        plan.validate_shape(2).unwrap();
        let paths = plan
            .passes()
            .iter()
            .map(|pass| pass.semantic_path.as_str())
            .collect::<Vec<_>>();
        let ordered = [
            "backdrop.destination",
            "backdrop",
            "children[0].composite",
            "clip",
            "filters[0]",
            "filters[1]",
            "mask",
            "opacity",
            "blend.destination",
            "blend",
        ];
        let mut previous = 0;
        for suffix in ordered {
            let index = paths
                .iter()
                .enumerate()
                .skip(previous)
                .find_map(|(index, path)| path.ends_with(suffix).then_some(index))
                .unwrap_or_else(|| panic!("missing ordered local pass {suffix}"));
            previous = index + 1;
        }
        let destinations = plan
            .passes()
            .iter()
            .filter_map(|pass| match pass.kind {
                ProgramPassKind::ReadDestination {
                    operation,
                    external,
                    ..
                } => Some((operation, external)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            destinations,
            vec![
                (
                    ProgramDestinationKind::Backdrop,
                    Some(ProgramDestinationId::try_from(1).unwrap()),
                ),
                (
                    ProgramDestinationKind::Blend,
                    Some(ProgramDestinationId::try_from(2).unwrap()),
                ),
            ]
        );
        assert_eq!(
            plan.resources()[plan.output().index()].bounds,
            program.geometry(root).unwrap().output_bounds
        );
        let restored: ProgramPlan =
            serde_json::from_value(serde_json::to_value(&plan).unwrap()).unwrap();
        assert_eq!(restored, plan);

        let mut duplicate_destination = plan.clone();
        let mut first = None;
        for pass in &mut duplicate_destination.passes {
            if let ProgramPassKind::ReadDestination {
                external: Some(destination),
                ..
            } = &mut pass.kind
            {
                match first {
                    Some(first) => *destination = first,
                    None => first = Some(*destination),
                }
            }
        }
        assert!(duplicate_destination.validate_shape(2).is_err());
    }

    #[test]
    fn isolation_turns_current_into_a_local_read_without_hiding_parent_blend() {
        let mut builder = DrawProgramBuilder::new(Rect::new(0.0, 0.0, 16.0, 9.0));
        let child = rect_path(
            &mut builder,
            Rect::new(2.0, 2.0, 8.0, 4.0),
            LinearColor::new(0.5, 0.5, 0.5, 1.0),
        );
        let mut group = Group::plain(vec![child]);
        group.isolated = true;
        group.internal_blend = BlendMode::Multiply;
        group.backdrop = Some(BackdropRead {
            scope: BackdropScope::Current,
            bounds: Rect::new(2.0, 2.0, 8.0, 4.0),
            footprint: Insets::uniform(0.0),
            sampling: valle_draw::requirements::SamplingMode::LinearClamp,
            filters: Vec::new(),
        });
        let root = builder.push_node(Node::Group(group));
        builder.add_root(root);
        let program = builder.finish().unwrap();
        let plan = ProgramPlan::derive(&program).unwrap();

        let destinations = plan
            .passes()
            .iter()
            .filter_map(|pass| match pass.kind {
                ProgramPassKind::ReadDestination {
                    operation,
                    external,
                    ..
                } => Some((operation, external)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            destinations,
            vec![
                (ProgramDestinationKind::Backdrop, None),
                (
                    ProgramDestinationKind::Blend,
                    Some(ProgramDestinationId::try_from(1).unwrap()),
                ),
            ]
        );
    }

    #[test]
    fn current_destination_composes_prior_local_siblings_over_the_external_backdrop() {
        let mut builder = DrawProgramBuilder::new(Rect::new(0.0, 0.0, 24.0, 12.0));
        let lower = rect_path(
            &mut builder,
            Rect::new(0.0, 0.0, 24.0, 12.0),
            LinearColor::new(0.1, 0.2, 0.3, 1.0),
        );
        let glass_child = rect_path(
            &mut builder,
            Rect::new(5.0, 3.0, 10.0, 6.0),
            LinearColor::new(0.8, 0.6, 0.4, 0.75),
        );
        let mut glass = Group::plain(vec![glass_child]);
        glass.backdrop = Some(BackdropRead {
            scope: BackdropScope::Current,
            bounds: Rect::new(5.0, 3.0, 10.0, 6.0),
            footprint: Insets::uniform(1.0),
            sampling: valle_draw::requirements::SamplingMode::LinearClamp,
            filters: Vec::new(),
        });
        glass.internal_blend = BlendMode::Overlay;
        let glass = builder.push_node(Node::Group(glass));
        let root = builder.push_node(Node::Group(Group::plain(vec![lower, glass])));
        builder.add_root(root);
        let program = builder.finish().unwrap();
        let plan = ProgramPlan::derive(&program).unwrap();

        let reads = plan
            .passes()
            .iter()
            .filter_map(|pass| match &pass.kind {
                ProgramPassKind::ReadDestination {
                    operation,
                    external,
                    local_inputs,
                    ..
                } => Some((*operation, *external, local_inputs.as_slice())),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(reads.len(), 2);
        assert_eq!(reads[0].0, ProgramDestinationKind::Backdrop);
        assert_eq!(reads[1].0, ProgramDestinationKind::Blend);
        for (index, (_, external, local_inputs)) in reads.iter().enumerate() {
            assert_eq!(
                *external,
                Some(ProgramDestinationId::from_index(index).unwrap())
            );
            assert_eq!(local_inputs.len(), 1);
            let writer = plan
                .passes()
                .iter()
                .find(|pass| pass.kind.output() == local_inputs[0])
                .unwrap();
            assert!(writer.semantic_path.ends_with("children[0].composite"));
        }
    }
}
