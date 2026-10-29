//! Motion JSX Scene Artifact → shared Takumi layout tree.
//!
//! This is a direct bridge: it evaluates the Motion expression arena and creates Takumi nodes from
//! [`SceneArtifact`]. The backend-neutral [`crate::emit`] then shapes text and emits the same [`valle_draw::program::recording::ProgramRecording`]
//! consumed by Native and CanvasKit executors.

use crate::batch::{
    BakedParticleTrajectories, bake_particle_trajectories, resolve_geometry_batch_with_identity,
};
use crate::eval::{
    EvalPlan, InstanceEvalPlan, SampleValueCache, eval_all_planned_cached,
    eval_layout_bounds_planned, eval_post_layout_planned, eval_roots_planned_dense_cached,
    eval_units_planned,
};
use crate::style::gradient_background_source;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use takumi_core::layout::node::Node;
use takumi_core::layout::tree::{LayoutResults, RenderNode};
use takumi_core::style::{Affine as TAffine, ComputedStyle, SizingContext, ZIndex};
use valle_draw::program::recording::Homography;
use valle_motion::MotionValue;
use valle_motion::value::{Length, LengthUnit};
use valle_motion::{
    BatchPositions, BoolValue, ColorValue, CoordinateSpace, EvalError, EvalInputs, Expr, ExprId,
    GeometryBatchGeometry, GradientStopValue, InstanceColumnValues, InstanceGroup,
    InstanceTemplateNode, MaskValue, NodeId, NodeKind, NumberValue, PaintValue, PathData,
    PathValue, PointValue, RectValue, ResolvedProps, SceneArtifact, ShaderUniformValue, StyleValue,
    TextSplit, TextValue, TransitionEffect, ValidationError, css_token,
};

use super::activation::{ActivationPlan, InstanceRangeGate, InstanceRowSelection};
use super::dependencies::{SceneDependencies, TemporalSampling};
use crate::layout::bridge::{
    EchoSample, FrameDeclarations, LayoutOptions, LayoutTree, ResolvedUnit, TemporalSample,
    parse_style,
};

/// Fail-closed failures from Scene Artifact evaluation or layout construction.
#[derive(Debug, Clone, PartialEq)]
pub enum LayoutError {
    InvalidArtifact(Vec<ValidationError>),
    Eval(EvalError),
    Instance {
        group: u32,
        node: String,
        reason: Box<LayoutError>,
    },
    BadNode {
        at: usize,
    },
    UnsupportedNode {
        node: String,
        capability: String,
    },
    BadViewport,
    BadStyle {
        node: String,
        declarations: String,
        reason: String,
    },
    BadPaint {
        node: String,
        reason: String,
    },
    BadMask {
        node: String,
        reason: String,
    },
    BadTransition {
        node: String,
        reason: String,
    },
    BadShutter {
        node: String,
        reason: String,
    },
    BadEcho {
        node: String,
        reason: String,
    },
    BadTimeScope {
        node: String,
        reason: String,
    },
    BadShader {
        node: String,
        reason: String,
    },
    BadScene3D {
        node: String,
        reason: String,
    },
    BadFormula {
        node: String,
        reason: String,
    },
    /// Reject CSS surfaces without a supported Motion lowering or paint implementation.
    UnsupportedSurface {
        node: String,
        surface: String,
    },
    /// A node consumes post-layout scene coordinates but is not at the scene origin. Its local
    /// transform would translate those coordinates again. Position coordinate consumers as absolute
    /// overlays at `left: 0; top: 0`.
    PostLayoutOffOrigin {
        node: String,
        origin: (f64, f64),
    },
    /// Invalid camera parameters for this frame, including non-positive zoom.
    BadCamera {
        reason: String,
    },
}

impl core::fmt::Display for LayoutError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            LayoutError::InvalidArtifact(errors) => {
                write!(f, "invalid Scene Artifact ({} error(s))", errors.len())
            }
            LayoutError::UnsupportedNode { node, capability } => write!(
                f,
                "node `{node}` requires `{capability}`, which this ProgramRecording producer does not execute"
            ),
            LayoutError::Eval(error) => write!(f, "eval: {error}"),
            LayoutError::Instance { group, node, reason } => write!(f, "node `{node}`: instance group {group}: {reason}"),
            LayoutError::BadNode { at } => write!(f, "node {at}: out of range"),
            LayoutError::BadViewport => f.write_str(
                "viewport dimensions must be non-zero and DPR finite and positive; a scene that reads the composition width or height also requires both dimensions and a finite positive initial font size",
            ),
            LayoutError::BadStyle {
                node,
                declarations,
                reason,
            } => {
                write!(f, "node `{node}`: ")?;
                if !declarations.is_empty() {
                    write!(f, "style `{declarations}`: ")?;
                }
                f.write_str(reason)
            }
            LayoutError::BadPaint { node, reason } => {
                write!(f, "node `{node}`: bad paint ({reason})")
            }
            LayoutError::BadMask { node, reason } => {
                write!(f, "node `{node}`: bad mask ({reason})")
            }
            LayoutError::BadTransition { node, reason } => write!(f, "Transition '{node}': {reason}"),
            LayoutError::BadShutter { node, reason } => write!(f, "Shutter '{node}': {reason}"),
            LayoutError::BadEcho { node, reason } => write!(f, "Echo '{node}': {reason}"),
            LayoutError::BadTimeScope { node, reason } => write!(f, "TimeScope '{node}': {reason}"),
            LayoutError::BadShader { node, reason } => {
                write!(f, "node `{node}`: bad Shader parameter ({reason})")
            }
            LayoutError::BadScene3D { node, reason } => {
                write!(f, "node `{node}`: bad Scene3D frame ({reason})")
            }
            LayoutError::BadFormula { node, reason } => {
                write!(f, "node `{node}`: bad MathFormula ({reason})")
            }
            LayoutError::UnsupportedSurface { node, surface } => write!(
                f,
                "node `{node}` uses `{surface}`, which has no supported Motion lowering or paint implementation"
            ),
            LayoutError::PostLayoutOffOrigin {
                node,
                origin: (x, y),
            } => write!(
                f,
                "node `{node}` draws post-layout geometry (bounds/anchor/connect/project3d) but sits at \
                 ({x}, {y}) instead of the scene origin, so those scene coordinates would be \
                 painted offset by that amount; make it an absolute overlay \
                 (`position: absolute; left: 0; top: 0`)"
            ),
            LayoutError::BadCamera { reason } => write!(f, "camera: {reason}"),
        }
    }
}

impl std::error::Error for LayoutError {}

impl From<EvalError> for LayoutError {
    fn from(error: EvalError) -> Self {
        LayoutError::Eval(error)
    }
}

/// A validated and capability-admitted Scene.
///
/// Artifact validation and the deterministic Takumi-surface audit are load-time work. Keeping
/// their proof in this wrapper prevents accidentally skipping either gate while avoiding an
/// `O(nodes)` rescan on every frame.
#[derive(Debug, Clone)]
pub struct PreparedScene {
    artifact: Arc<SceneArtifact>,
    /// Expand compact numeric formulas once at load time, keeping frame lookup in dense arrays.
    materialized_instance_groups: Arc<Vec<Option<InstanceGroup>>>,
    instance_eval_plans: Arc<Vec<InstanceEvalPlan>>,
    instance_cache: Arc<std::sync::Mutex<super::instance_cache::InstanceBatchCache>>,
    /// Force trajectories are baked once at scene preparation, never integrated during a frame.
    particle_trajectories: Arc<Vec<Option<BakedParticleTrajectories>>>,
    instance_activation: Arc<Vec<Option<InstanceRangeGate>>>,
    eval_plan: EvalPlan,
    dependencies: Arc<SceneDependencies>,
    activation: ActivationPlan,
    layout_reusable: bool,
    /// Scene-seconds derivatives of inline translations, when the entire scene has static
    /// geometry and no other time-varying transforms. `None` selects the sampled layout path.
    auto_blur_translations: Option<HashMap<String, valle_draw::Point>>,
    stylesheet: Arc<takumi_core::style::StyleSheet>,
    sample_scene: Option<Arc<PreparedScene>>,
    sample_roots: Option<Vec<ExprId>>,
    requires_explicit_viewport: bool,
    layout_classes: Vec<Vec<String>>,
    /// Parsed once, still cascaded against this frame's parent. This contains specified
    /// declarations only, never inherited colors or computed styles from another frame.
    static_styles: Arc<Vec<Option<takumi_core::style::Style>>>,
    layout_probes: Vec<(crate::ExprId, MotionValue)>,
    // Mask sources and transition children have special containing-block placement. Compute
    // membership once rather than searching every scene node for every emitted node.
    positioned_children: Arc<Vec<u8>>,
    has_post_layout_values: bool,
    /// Emission reserves display instances for the batches of every built temporal sample and
    /// gives paper grain the remainder. Unpainted samples may be skipped only when the scene has
    /// no batch or no paper grain, so that remainder cannot change.
    skip_unpainted_samples: bool,
    /// Whether a temporal sample's tree depends on its output time beyond the scene time it
    /// shows, so samples that show the same scene time cannot share one.
    samples_read_output_time: bool,
    /// Conservative load-time proof for sharing this scene's temporal layout within a request.
    temporal_tree_invariant: bool,
    /// The admitted packages of the artifact's inline shaders, in artifact order.
    inline_shaders: Arc<[crate::shader::ShaderPackage]>,
}

struct LayoutInstanceRow {
    key: String,
    class_name: String,
    declarations: FrameDeclarations,
    background_color: Option<valle_draw::program::AuthorColor>,
    text_color: Option<valle_draw::program::AuthorColor>,
    paint_colors: crate::style::precise_color::PreciseCssColors,
    text: Option<String>,
    children: Vec<LayoutInstanceRow>,
}

type LayoutInstanceRows = HashMap<u32, Vec<LayoutInstanceRow>>;

fn visit_layout_instance_node(row: &LayoutInstanceRow, visit: &mut impl FnMut(&LayoutInstanceRow)) {
    visit(row);
    for child in &row.children {
        visit_layout_instance_node(child, visit);
    }
}

/// Values shared by one render request while its main frame and temporal samples are built.
pub(super) struct RequestValueCache {
    scene: SampleValueCache,
    instances: Vec<SampleValueCache>,
}

impl RequestValueCache {
    pub(super) fn counts(&self) -> (usize, usize) {
        let (computed, reused) = self.instance_counts();
        let (scene_computed, scene_reused) = self.scene.counts();
        (scene_computed + computed, scene_reused + reused)
    }

    pub(super) fn instance_counts(&self) -> (usize, usize) {
        self.instances.iter().fold((0, 0), |total, cache| {
            let (computed, reused) = cache.counts();
            (total.0 + computed, total.1 + reused)
        })
    }
}

/// Work selected for one source sample before layout and rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameEvaluationStats {
    pub evaluated_expressions: usize,
    pub active_nodes: usize,
    pub layout_nodes: usize,
}

impl PreparedScene {
    pub fn can_reuse_layout(&self) -> bool {
        self.layout_reusable
    }
    pub(super) fn temporal_artifact(&self) -> &SceneArtifact {
        self.sample_scene.as_deref().unwrap_or(self).artifact()
    }
    pub(super) fn same_instance(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.artifact, &other.artifact)
    }

    pub fn artifact(&self) -> &SceneArtifact {
        &self.artifact
    }

    /// The admitted packages of the artifact's inline shaders, which a host serves as it serves
    /// the descriptor packages the artifact depends on.
    pub fn inline_shaders(&self) -> &[crate::shader::ShaderPackage] {
        &self.inline_shaders
    }

    /// Configure preparation reuse for this scene and its internal temporal projection. A zero
    /// limit selects full recomputation; existing clones keep their previous cache owner.
    pub fn with_instance_cache_limits(mut self, max_entries: usize, max_bytes: usize) -> Self {
        self.instance_cache = Arc::new(std::sync::Mutex::new(
            super::instance_cache::InstanceBatchCache::new(max_entries, max_bytes),
        ));
        if let Some(sample) = &mut self.sample_scene {
            Arc::make_mut(sample).instance_cache = Arc::clone(&self.instance_cache);
        }
        self
    }

    /// Shared across this scene's main/temporal requests and Native preparer clones.
    pub fn instance_cache_stats(&self) -> Option<super::InstanceCacheStats> {
        self.instance_cache.lock().ok().map(|cache| cache.stats())
    }

    pub(super) fn sample_value_cache(&self) -> Option<RequestValueCache> {
        (self.dependencies.temporal_sample_count() > 0
            || !self.dependencies.auto_blur_nodes().is_empty())
        .then(|| RequestValueCache {
            scene: SampleValueCache::new(&self.eval_plan),
            instances: self
                .instance_eval_plans
                .iter()
                .map(|plan| SampleValueCache::for_len(plan.len()))
                .collect(),
        })
    }

    pub fn dependencies(&self) -> &SceneDependencies {
        &self.dependencies
    }

    /// Report the same pre-layout activation schedule used by `build_tree` for this sample.
    /// The count includes visibility gate work and the selected base expression plan.
    pub fn frame_evaluation_stats(
        &self,
        ctx: &valle_motion::MotionContext,
    ) -> FrameEvaluationStats {
        let (base_expressions, active_nodes) =
            self.activation.stats(&self.artifact, &self.eval_plan, ctx);
        let layout_rows = self
            .artifact
            .nodes
            .iter()
            .filter_map(|node| match node.kind {
                NodeKind::InstanceLayout { group } => self
                    .artifact
                    .instance_groups
                    .get(group as usize)
                    .map(|group| group.rows() * group.template_node_count()),
                _ => None,
            })
            .sum::<usize>();
        FrameEvaluationStats {
            evaluated_expressions: base_expressions + self.instance_expression_evaluations(ctx),
            active_nodes,
            layout_nodes: active_nodes + layout_rows,
        }
    }

    fn instance_expression_evaluations(&self, ctx: &valle_motion::MotionContext) -> usize {
        let layout_groups = self
            .artifact
            .nodes
            .iter()
            .filter_map(|node| match node.kind {
                NodeKind::InstanceLayout { group } => Some(group as usize),
                _ => None,
            })
            .collect::<std::collections::BTreeSet<_>>();
        self.artifact
            .instance_groups
            .iter()
            .enumerate()
            .map(|(at, group)| {
                let plan = &self.instance_eval_plans[at];
                if layout_groups.contains(&at) {
                    return plan.work_for_rows(group.rows());
                }
                self.instance_activation
                    .get(at)
                    .and_then(|gate| gate.as_ref())
                    .and_then(|gate| gate.select(ctx, group.rows()))
                    .map_or_else(
                        || plan.work_for_rows(group.rows()),
                        |selection| {
                            plan.work_for_rows(selection.rows.len()) + selection.gate_evaluations
                        },
                    )
            })
            .sum()
    }

    fn validate_viewport(
        &self,
        viewport: takumi_core::viewport::Viewport,
    ) -> Result<(), LayoutError> {
        if viewport.size.width == Some(0)
            || viewport.size.height == Some(0)
            || !viewport.device_pixel_ratio.is_finite()
            || viewport.device_pixel_ratio <= 0.0
            || self.requires_explicit_viewport
                && (viewport.size.width.is_none()
                    || viewport.size.height.is_none()
                    || !viewport.font_size.is_finite()
                    || viewport.font_size <= 0.0)
        {
            return Err(LayoutError::BadViewport);
        }
        Ok(())
    }

    /// Intrinsic measurement prepares a static Text artifact and shares this exact node/style
    /// construction with frame layout. Only its available-space constraint differs.
    pub(crate) fn intrinsic_node(
        &self,
        at: NodeId,
        viewport: takumi_core::viewport::Viewport,
    ) -> Result<(Node, Arc<takumi_core::style::StyleSheet>), LayoutError> {
        self.validate_viewport(viewport)?;
        let node = node_of(
            self,
            at.0 as usize,
            &[],
            None,
            None,
            &HashMap::new(),
            &[],
            &HashMap::new(),
        )?;
        Ok((node, self.stylesheet.clone()))
    }
}

/// Validate wire versions, topology, and deterministic paint surfaces.
pub fn prepare(artifact: &SceneArtifact) -> Result<PreparedScene, LayoutError> {
    prepare_owned(artifact.clone())
}

/// Owning prepare path for long-lived Native/WASM sessions. Validation and capability admission
/// happen before the Artifact enters the `Arc`; every frame then reuses the same proof and bytes.
pub fn prepare_owned(artifact: SceneArtifact) -> Result<PreparedScene, LayoutError> {
    artifact.validate().map_err(LayoutError::InvalidArtifact)?;
    admit_supported_surface(&artifact)?;
    let mut inline_shaders = artifact
        .inline_shader_packages()
        .map_err(LayoutError::InvalidArtifact)?;
    // Glass draws with built-in shaders, which backends fetch as they fetch an inline one.
    if artifact
        .nodes
        .iter()
        .any(|node| matches!(node.kind, NodeKind::Glass(_) | NodeKind::GlassField(_)))
    {
        inline_shaders.push(crate::glass::optics::optics_package().clone());
        inline_shaders.push(crate::glass::optics::rim_package().clone());
    }
    let mut scene = prepare_admitted(artifact, false)?;
    scene.inline_shaders = inline_shaders.into();
    Ok(scene)
}

fn prepare_admitted(
    artifact: SceneArtifact,
    is_sample: bool,
) -> Result<PreparedScene, LayoutError> {
    let instance_activation = artifact
        .instance_groups
        .iter()
        .map(InstanceRangeGate::of)
        .collect();
    let instance_eval_plans = artifact
        .instance_groups
        .iter()
        .map(InstanceEvalPlan::for_render)
        .collect();
    let materialized_instance_groups = artifact
        .instance_groups
        .iter()
        .map(|group| {
            if !group
                .columns
                .iter()
                .any(|column| matches!(column.values, InstanceColumnValues::Formula { .. }))
            {
                return None;
            }
            let mut materialized = group.clone();
            for column in &mut materialized.columns {
                if let InstanceColumnValues::Formula { rows, expression } = &column.values {
                    column.values = InstanceColumnValues::Numbers(
                        (0..*rows as usize)
                            .map(|index| expression.evaluate(index))
                            .collect(),
                    );
                }
            }
            Some(materialized)
        })
        .collect();
    let particle_trajectories = artifact
        .nodes
        .iter()
        .map(|node| match &node.kind {
            NodeKind::GeometryBatch { batch } => match &batch.positions {
                BatchPositions::Particles { spec, .. } => {
                    bake_particle_trajectories(spec).map_err(|reason| LayoutError::BadStyle {
                        node: node.key.clone(),
                        declarations: "particles forces".into(),
                        reason: reason.into(),
                    })
                }
                BatchPositions::Static { .. } => Ok(None),
            },
            _ => Ok(None),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let eval_plan = EvalPlan::new(&artifact.exprs);
    let (stylesheet, layout_classes, class_facts) = crate::tailwind::prepare_stylesheet(&artifact)
        .map_err(|reason| LayoutError::BadStyle {
            node: artifact.nodes[artifact.root.0 as usize].key.clone(),
            declarations: "className utilities".into(),
            reason,
        })?;
    let dependencies = Arc::new(SceneDependencies::new(&artifact, &eval_plan, &class_facts));
    let activation = ActivationPlan::new(&artifact, &eval_plan, &dependencies);
    let post_layout = crate::post_layout_dependent(&artifact.exprs);
    let types =
        crate::expr::validate_exprs(&artifact.exprs, &artifact.controls, &mut Vec::new()).types;
    let probe_ids: std::collections::BTreeSet<_> = artifact
        .nodes
        .iter()
        .flat_map(|node| &node.styles)
        .filter_map(|style| match style.value {
            StyleValue::Expr { expr } if post_layout[expr.0 as usize] => Some(expr.0),
            _ => None,
        })
        .collect();
    let layout_probes = probe_ids
        .into_iter()
        .map(|id| {
            let expr = crate::ExprId(id);
            (
                expr,
                crate::style::layout_probe_value(expr, &artifact.exprs, &types),
            )
        })
        .collect();
    let layout_reusable = super::reuse::eligible(&artifact, &types);
    let auto_blur_translations =
        analytic_auto_blur_translations(&artifact, &dependencies, layout_reusable);
    let instance_cache = Arc::new(std::sync::Mutex::new(
        super::instance_cache::InstanceBatchCache::default(),
    ));
    let sample_scene = if is_sample {
        None
    } else {
        super::temporal::sample_artifact(&artifact, &dependencies)
            .map(|subset| {
                prepare_admitted(subset, true).map(|mut sample| {
                    // Temporal projection preserves instance tables and indices exactly.
                    sample.instance_cache = Arc::clone(&instance_cache);
                    Arc::new(sample)
                })
            })
            .transpose()?
    };
    let sample_roots = is_sample.then(|| {
        let mut roots: Vec<_> = artifact
            .nodes
            .iter()
            .flat_map(|node| {
                node.expr_refs_outside_per_unit()
                    .into_iter()
                    .chain(node.per_unit_expr_refs())
                    .map(|(_, id)| id)
            })
            .collect();
        // Later passes evaluate their full plan. Preserve the inputs of unused post-layout/unit
        // expressions too, until those passes support their own sparse schedules.
        roots.extend(
            (0..artifact.exprs.len())
                .map(|at| ExprId(at as u32))
                .filter(|id| eval_plan.post_layout_dependent(*id) || eval_plan.unit_dependent(*id)),
        );
        if let Some(camera) = &artifact.camera {
            roots.extend(camera.exprs().into_iter().map(|(_, expr)| expr));
        }
        eval_plan.schedule(roots)
    });
    let mut positioned_children = vec![0_u8; artifact.nodes.len()];
    let node_indices: HashMap<_, _> = artifact
        .nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.key.as_str(), index))
        .collect();
    for node in &artifact.nodes {
        match &node.kind {
            NodeKind::Mask {
                source: MaskValue::Subtree { source },
                ..
            } => {
                if let Some(index) = node_indices.get(source.as_str()) {
                    positioned_children[*index] |= 1;
                }
            }
            NodeKind::Transition { .. } => {
                for child in &artifact.node_children
                    [node.children.start as usize..node.children.end as usize]
                {
                    positioned_children[child.0 as usize] |= 2;
                }
            }
            _ => {}
        }
    }
    let static_styles = artifact
        .nodes
        .iter()
        .enumerate()
        .map(|(at, node)| {
            if node.visibility.is_some()
                || positioned_children[at] & 2 != 0
                || matches!(
                    node.kind,
                    NodeKind::MathFormula { .. } | NodeKind::Transition { .. }
                )
                || node
                    .styles
                    .iter()
                    .any(|style| matches!(style.value, StyleValue::Expr { .. }))
            {
                return Ok(None);
            }
            let declarations = declarations(node, &[], true, at)?;
            declarations
                .resolve(None)
                .map(Some)
                .map_err(|reason| LayoutError::BadStyle {
                    node: node.key.clone(),
                    declarations: declarations.source(),
                    reason,
                })
        })
        .collect::<Result<Vec<_>, LayoutError>>()?;
    let temporal_tree_invariant = super::temporal::sample_tree_invariant(&artifact, &eval_plan);
    Ok(PreparedScene {
        temporal_tree_invariant,
        sample_scene,
        sample_roots,
        eval_plan,
        dependencies,
        activation,
        materialized_instance_groups: Arc::new(materialized_instance_groups),
        instance_eval_plans: Arc::new(instance_eval_plans),
        instance_cache,
        particle_trajectories: Arc::new(particle_trajectories),
        instance_activation: Arc::new(instance_activation),
        layout_classes,
        static_styles: Arc::new(static_styles),
        // Only expressions that read the viewport still need both dimensions; the canvas itself
        // comes from the artifact's delivery contract.
        requires_explicit_viewport: artifact
            .capability_set
            .names
            .iter()
            .any(|name| name == crate::artifact::VIEWPORT_CAPABILITY),
        layout_reusable,
        auto_blur_translations,
        skip_unpainted_samples: !super::temporal::sample_trees_share_batch_budget(&artifact),
        samples_read_output_time: super::temporal::samples_read_output_time(&artifact),
        artifact: Arc::new(artifact),
        stylesheet,
        layout_probes,
        positioned_children: Arc::new(positioned_children),
        has_post_layout_values: post_layout.iter().any(|dependent| *dependent),
        inline_shaders: Arc::from([]),
    })
}

/// Evaluate one frame, build the Takumi render tree, and run Taffy/Parley layout.
///
/// Dynamic visibility is lowered to `visibility: hidden`, not node removal. This keeps the Scene
/// topology and layout geometry fixed while suppressing paint, matching the Scene contract.
pub fn build_tree(
    prepared: &PreparedScene,
    ctx: &valle_motion::MotionContext,
    props: &ResolvedProps,
    opts: &LayoutOptions<'_>,
) -> Result<LayoutTree, LayoutError> {
    let mut shared = prepared.sample_value_cache();
    build_tree_inner(
        prepared,
        ctx,
        props,
        opts,
        None,
        None,
        shared.as_mut(),
        true,
        true,
    )
}

/// Native diagnostics only. These timings never enter an Artifact or DrawProgram.
#[derive(Debug, Default, serde::Serialize)]
pub struct LayoutTimings {
    pub layout_reused: bool,
    /// Additional layout attempts made to estimate automatic motion blur velocity.
    pub auto_blur_neighbor_layouts: u8,
    /// Authored nodes retained in each temporal layout, before instance row projection.
    pub temporal_layout_nodes: usize,
    /// Distinct retained layout trees used by Shutter/Echo, not the number of rendered samples.
    pub temporal_sample_trees: usize,
    pub evaluated_expressions: usize,
    /// Static expressions computed once and reused across this request's temporal samples.
    pub sample_static_computed: usize,
    pub sample_static_reused: usize,
    /// The instance-template subset of the sample-static counts above.
    pub instance_static_computed: usize,
    pub instance_static_reused: usize,
    pub active_nodes: usize,
    pub layout_nodes: usize,
    pub eval_ms: f64,
    pub tree_ms: f64,
    pub layout_ms: f64,
    pub finish_ms: f64,
}

#[cfg(not(target_arch = "wasm32"))]
pub fn build_tree_profiled(
    prepared: &PreparedScene,
    ctx: &valle_motion::MotionContext,
    props: &ResolvedProps,
    opts: &LayoutOptions<'_>,
) -> Result<(LayoutTree, LayoutTimings), LayoutError> {
    let mut timings = LayoutTimings::default();
    let mut shared = prepared.sample_value_cache();
    let tree = build_tree_inner(
        prepared,
        ctx,
        props,
        opts,
        Some(&mut timings),
        None,
        shared.as_mut(),
        true,
        true,
    )?;
    if prepared.dependencies.temporal_sample_count() > 0
        || !prepared.dependencies.auto_blur_nodes().is_empty()
    {
        timings.temporal_layout_nodes = prepared
            .sample_scene
            .as_deref()
            .unwrap_or(prepared)
            .artifact
            .nodes
            .len();
    }
    if let Some(shared) = shared {
        (timings.sample_static_computed, timings.sample_static_reused) = shared.counts();
        (
            timings.instance_static_computed,
            timings.instance_static_reused,
        ) = shared.instance_counts();
    }
    timings.temporal_sample_trees = tree
        .shutters
        .values()
        .flatten()
        .map(|s| Rc::as_ptr(&s.tree))
        .chain(
            tree.echoes
                .values()
                .flatten()
                .map(|s| Rc::as_ptr(&s.sample.tree)),
        )
        .collect::<HashSet<_>>()
        .len();
    Ok((tree, timings))
}

#[expect(
    clippy::too_many_arguments,
    reason = "Keep the layout pass inputs and its independently borrowed caches explicit."
)]
pub(super) fn build_tree_inner(
    prepared: &PreparedScene,
    ctx: &valle_motion::MotionContext,
    props: &ResolvedProps,
    opts: &LayoutOptions<'_>,
    mut timings: Option<&mut LayoutTimings>,
    geometry_cache: Option<&std::cell::RefCell<Option<super::reuse::GeometrySnapshot>>>,
    mut shared: Option<&mut RequestValueCache>,
    auto_blur: bool,
    temporal_samples: bool,
) -> Result<LayoutTree, LayoutError> {
    let mut started = timings.as_ref().map(|_| std::time::Instant::now());
    fn elapsed(started: &mut Option<std::time::Instant>) -> f64 {
        let now = std::time::Instant::now();
        started
            .replace(now)
            .map_or(0.0, |start| (now - start).as_secs_f64() * 1000.0)
    }
    let artifact = prepared.artifact();
    let bound_viewport = opts.viewport.with_font_size(crate::ROOT_FONT_SIZE);
    let opts = LayoutOptions {
        viewport: bound_viewport,
        fonts: opts.fonts,
        styles: opts.styles,
        #[cfg(target_arch = "wasm32")]
        formula_fonts: opts.formula_fonts,
    };
    let opts = &opts;
    prepared.validate_viewport(opts.viewport)?;

    let viewport = eval_viewport(opts);
    let (mut values, evaluated_expressions, active_nodes, inactive_nodes) =
        if let Some(roots) = &prepared.sample_roots {
            let values = eval_roots_planned_dense_cached(
                artifact,
                &prepared.eval_plan,
                EvalInputs {
                    ctx,
                    props,
                    unit: None,
                    viewport,
                },
                roots,
                shared.as_deref_mut().map(|cache| &mut cache.scene),
            )?;
            (values, roots.len(), artifact.nodes.len(), Vec::new())
        } else if let Some(base) = prepared.activation.evaluate(
            artifact,
            &prepared.eval_plan,
            ctx,
            props,
            viewport,
            shared.as_deref_mut().map(|cache| &mut cache.scene),
        )? {
            (
                base.values,
                base.evaluated_expressions,
                base.active_nodes,
                base.inactive_nodes,
            )
        } else {
            (
                eval_all_planned_cached(
                    artifact,
                    &prepared.eval_plan,
                    EvalInputs {
                        ctx,
                        props,
                        unit: None,
                        viewport,
                    },
                    shared.as_deref_mut().map(|cache| &mut cache.scene),
                )?,
                prepared.eval_plan.base_eval_count(),
                artifact.nodes.len(),
                Vec::new(),
            )
        };
    for (expr, probe) in &prepared.layout_probes {
        values[expr.0 as usize] = probe.clone();
    }
    let layout_instances = eval_layout_instances(
        prepared,
        ctx,
        props,
        viewport,
        &inactive_nodes,
        shared.as_deref_mut(),
    )?;
    if let Some(timings) = timings.as_deref_mut() {
        timings.evaluated_expressions =
            evaluated_expressions + prepared.instance_expression_evaluations(ctx);
        timings.active_nodes = active_nodes;
        let mut projected_nodes = 0;
        for rows in layout_instances.values() {
            for row in rows {
                visit_layout_instance_node(row, &mut |_| projected_nodes += 1);
            }
        }
        timings.layout_nodes = active_nodes + projected_nodes;
        timings.eval_ms = elapsed(&mut started);
    }
    let formulas = prepare_formula_fragments(artifact, &values, opts)?;
    // Wrap World subtrees inside the root and leave Screen subtrees outside the camera.
    //
    // A bounds-dependent camera cannot be evaluated until layout has produced its inputs.
    //
    // Probe with the final wrapper structure and identity transforms. CSS transforms do not affect
    // layout, so the probe produces the same boxes as the final tree.
    //
    // Keep both wrappers in the probe: their absolute positioning defines containing blocks for
    // World content.
    //
    // Only bounds-dependent cameras require an extra layout pass.
    let (camera, camera_space) = if camera_depends_on_bounds(artifact, &prepared.eval_plan) {
        let probe = identity_camera_wrappers(opts)?;
        let probe_node = node_of(
            prepared,
            artifact.root.0 as usize,
            &values,
            opts.styles,
            Some(&probe),
            &formulas,
            &inactive_nodes,
            &layout_instances,
        )?;
        let probe_boxes = layout_and_collect_boxes(prepared, probe_node, opts)?;
        let probe_values = eval_layout_bounds_planned(
            artifact,
            &prepared.eval_plan,
            &values,
            EvalInputs {
                ctx,
                props,
                unit: None,
                viewport: eval_viewport(opts),
            },
            &probe_boxes,
        )?;
        (
            camera_wrappers(artifact, &probe_values, opts)?,
            camera_space(artifact, &probe_values)?,
        )
    } else {
        (
            camera_wrappers(artifact, &values, opts)?,
            camera_space(artifact, &values)?,
        )
    };
    let render_context = takumi_core::context::RenderContext::builder()
        .fonts(opts.fonts.snapshot())
        .sizing(SizingContext::builder().viewport(opts.viewport).build())
        .images(Rc::new(Default::default()))
        .stylesheet(prepared.stylesheet.clone())
        .time_ms(LayoutOptions::TIME_MS)
        .draw_debug_border(false)
        .style(Box::new(ComputedStyle::default()))
        .build();
    let geometry_key =
        geometry_cache.map(|_| super::reuse::GeometryKey::new(opts.viewport, props, ctx));
    let cached = geometry_cache.and_then(|cache| {
        cache
            .borrow()
            .as_ref()
            .filter(|snapshot| Some(&snapshot.key) == geometry_key.as_ref())
            .cloned()
    });
    let mut evaluated_root = None;
    let (layout, keys, boxes, scene3d_content_boxes) = if let Some(snapshot) = cached {
        if let Some(timings) = timings.as_deref_mut() {
            timings.layout_reused = true;
        }
        (
            snapshot.layout,
            snapshot.keys,
            snapshot.boxes,
            snapshot.scene3d_boxes,
        )
    } else {
        let node = node_of(
            prepared,
            artifact.root.0 as usize,
            &values,
            opts.styles,
            camera.as_ref(),
            &formulas,
            &inactive_nodes,
            &layout_instances,
        )?;
        let mut layout_root = RenderNode::from_node(&render_context, node);
        super::transform::preserve_identity(&mut layout_root);
        let mut tree = takumi_core::layout::tree::LayoutTree::from_render_node(&layout_root);
        if let Some(timings) = timings.as_deref_mut() {
            timings.tree_ms = elapsed(&mut started);
        }
        super::continuous::compute_layout(&mut tree, render_context.sizing.viewport.into());
        let layout = Arc::new(tree.into_results());
        if let Some(timings) = timings.as_deref_mut() {
            timings.layout_ms = elapsed(&mut started);
        }

        let mut keys = HashMap::new();
        // Collect Scene keys and layout boxes during the same traversal for post-layout evaluation.
        let mut boxes = BTreeMap::new();
        let mut scene3d_content_boxes = BTreeMap::new();
        walk_pairs(
            artifact,
            &layout_root,
            &layout,
            &inactive_nodes,
            &mut |at, key, id, origin| {
                keys.insert(u64::from(id), key.to_owned());
                if let Ok(computed) = layout.layout(id) {
                    boxes.insert(
                        key.to_owned(),
                        valle_draw::Rect::new(
                            f64::from(origin.0),
                            f64::from(origin.1),
                            f64::from(computed.size.width),
                            f64::from(computed.size.height),
                        ),
                    );
                    if at
                        .and_then(|at| artifact.nodes.get(at))
                        .is_some_and(|node| matches!(node.kind, NodeKind::Scene3D { .. }))
                    {
                        scene3d_content_boxes.insert(
                            key.to_owned(),
                            valle_draw::Rect::new(
                                f64::from(origin.0 + computed.border.left + computed.padding.left),
                                f64::from(origin.1 + computed.border.top + computed.padding.top),
                                f64::from(computed.content_box_width().max(0.0)),
                                f64::from(computed.content_box_height().max(0.0)),
                            ),
                        );
                    }
                }
            },
        )?;

        if artifact
            .exprs
            .iter()
            .any(|expr| matches!(expr, crate::Expr::NodeBounds { .. }))
        {
            inline_boxes(artifact, &layout_root, &layout, &inactive_nodes, &mut boxes)?;
        }
        let keys = Arc::new(keys);
        let boxes = Arc::new(boxes);
        let scene3d_content_boxes = Arc::new(scene3d_content_boxes);
        if let Some(cache) = geometry_cache {
            cache.replace(Some(super::reuse::GeometrySnapshot {
                key: geometry_key.expect("cache supplied"),
                layout: layout.clone(),
                keys: keys.clone(),
                boxes: boxes.clone(),
                scene3d_boxes: scene3d_content_boxes.clone(),
            }));
        }
        // Only the cascade is reused here. The final CSS projection may still require a new
        // render tree; layout results and frame values remain independently evaluated.
        evaluated_root = Some(layout_root);
        (layout, keys, boxes, scene3d_content_boxes)
    };

    // The second evaluation pass replaces placeholders for bounds and their dependents, leaving
    // other expressions unchanged.
    //
    // Evaluate bounds after layout and before resolving paths, clips, masks, and text paths.
    let bounds_values = eval_layout_bounds_planned(
        artifact,
        &prepared.eval_plan,
        &values,
        EvalInputs {
            ctx,
            props,
            unit: None,
            viewport: eval_viewport(opts),
        },
        &boxes,
    )?;
    let scene3d = resolve_scene3d_requests(artifact, &bounds_values)?;
    let projected = project_scene3d_anchors(artifact, &scene3d, &scene3d_content_boxes)?;
    let values = eval_post_layout_planned(
        artifact,
        &prepared.eval_plan,
        &bounds_values,
        EvalInputs {
            ctx,
            props,
            unit: None,
            viewport: eval_viewport(opts),
        },
        &boxes,
        &projected,
    )?;
    check_post_layout_origins(artifact, &values, &boxes)?;
    // Post-layout values are admitted only into paint/transform slots or sidecar geometry. Rebuild
    // the same fixed-topology RenderNode with their final values, but keep the already-computed
    // LayoutResults. This makes `translate: project3d(...)` visible without a layout feedback pass.
    let existing_root = if !prepared.has_post_layout_values {
        evaluated_root.take()
    } else {
        None
    };
    drop(evaluated_root);
    let mut root = match existing_root {
        Some(root) => root,
        None => {
            let final_node = node_of(
                prepared,
                artifact.root.0 as usize,
                &values,
                opts.styles,
                camera.as_ref(),
                &formulas,
                &inactive_nodes,
                &layout_instances,
            )?;
            RenderNode::from_node(&render_context, final_node)
        }
    };
    super::transform::preserve_identity(&mut root);
    // CSS 3D reads this frame's cascade from the final tree before adding derived depth:
    // conditional utilities, inline declarations, and importance in one computed result.
    let css_3d_planes = if camera_space.is_some()
        || artifact.nodes.iter().any(|node| {
            node.styles.iter().any(|style| {
                style.property.starts_with("motion-transform-3d-")
                    || matches!(
                        style.property.as_str(),
                        "rotate-x"
                            | "rotate-y"
                            | "perspective"
                            | "transform-style"
                            | "backface-visibility"
                    )
            })
        }) {
        let nodes = css_3d_nodes(&root, &layout, &keys)?;
        let planes = resolve_css_3d_planes(artifact, &values, &boxes, opts, &nodes, camera_space)?;
        apply_css_3d_planes(artifact, &mut root, &planes, &inactive_nodes);
        planes
    } else {
        HashMap::new()
    };

    // Map render paths to Scene keys and retain original text to recover node-local offsets from
    // concatenated inline text. Layout keys cannot identify inline nodes that have no box.
    let mut render_keys = HashMap::new();
    walk_render_keys(
        artifact,
        artifact.root,
        &root,
        &mut Vec::new(),
        &mut render_keys,
        &inactive_nodes,
    );
    let auto_trails = if auto_blur {
        let (trails, neighbor_layouts) = auto_blur_trails(
            prepared,
            ctx,
            props,
            opts,
            &root,
            &layout,
            &keys,
            &css_3d_planes,
            shared.as_deref_mut(),
        );
        if let Some(timings) = timings.as_deref_mut() {
            timings.auto_blur_neighbor_layouts = neighbor_layouts;
        }
        trails
    } else {
        HashMap::new()
    };
    let units = resolve_units(
        artifact,
        &prepared.eval_plan,
        &values,
        ctx,
        props,
        &boxes,
        &projected,
        eval_viewport(opts),
    )?;
    let mut node_texts = artifact
        .nodes
        .iter()
        .enumerate()
        .filter_map(|(at, node)| match &node.kind {
            NodeKind::Text { text, .. } => {
                Some((node.key.clone(), text_value(text, &values, at).ok()?))
            }
            _ => None,
        })
        .collect::<HashMap<_, _>>();

    // Resolve advanced CSS filters only after normal/important class and inline cascade, for
    // `filter` and then `backdrop-filter`.
    let mut css_filters = [HashMap::new(), HashMap::new()];
    let mut pending = vec![(&root, takumi_core::geometry::NodeId::ROOT)];
    while let Some((render_node, id)) = pending.pop() {
        if let Some(key) = keys.get(&u64::from(id)) {
            let properties = &render_node.context.style.custom_properties;
            for (lists, property) in css_filters.iter_mut().zip(["filter", "backdrop-filter"]) {
                let [slot, important_slot] = crate::style::advanced_filter::slots(property)
                    .expect("filter properties have slots");
                if let Some(text) = properties
                    .get(important_slot)
                    .or_else(|| properties.get(slot))
                    && let Some(stages) =
                        crate::style::advanced_filter::stages(text).map_err(|reason| {
                            LayoutError::BadStyle {
                                node: key.clone(),
                                declarations: text.clone(),
                                reason,
                            }
                        })?
                {
                    let list = stages
                        .into_iter()
                        .map(|stage| match stage {
                            crate::style::advanced_filter::Stage::Advanced(filter) => Some(filter),
                            crate::style::advanced_filter::Stage::Css(_) => None,
                        })
                        .collect::<Vec<_>>();
                    lists.insert(key.clone(), list);
                }
            }
        }
        if let (Some(children), Ok(boxes)) = (&render_node.children, layout.box_children(id)) {
            for child in boxes {
                if let Some(node) = children.get(child.render_index) {
                    pending.push((node, child.node_id));
                }
            }
        }
    }
    let mut paths = HashMap::new();
    let mut batches = HashMap::new();
    let mut text_paths = HashMap::new();
    let mut clips = HashMap::new();
    let mut masks = HashMap::new();
    let mut videos = HashMap::new();
    let mut advanced_filters = HashMap::new();
    let mut filter_lists = HashMap::new();
    let mut backdrop_filter_lists = HashMap::new();
    let mut screen_velocity_blurs = std::collections::HashSet::new();
    let mut backdrop_advanced_filters = HashMap::new();
    let mut transitions = HashMap::new();
    let mut shaders = HashMap::new();
    let mut layer_fx = HashMap::new();
    let mut blend_spaces = HashMap::new();
    let scoped_seconds = prepared
        .dependencies
        .mapped_seconds(ctx.sample.composition().as_f64())
        .map_err(|id| LayoutError::BadTimeScope {
            node: artifact.nodes[id.0 as usize].key.clone(),
            reason: "mapped time must remain finite".into(),
        })?;
    for (at, node) in artifact.nodes.iter().enumerate() {
        for style in &node.styles {
            if style.property == "mix-blend-space" {
                let resolved = match &style.value {
                    StyleValue::Static { value } => value,
                    StyleValue::Expr { expr } => value(&values, *expr, at)?,
                };
                let space = crate::style::blend::space(&css_token(resolved)).map_err(|reason| {
                    LayoutError::BadStyle {
                        node: node.key.clone(),
                        declarations: "mixBlendSpace".into(),
                        reason,
                    }
                })?;
                blend_spaces.insert(node.key.clone(), space);
            }
        }
        let auto_trail = auto_trails.get(&node.key).copied();
        let filters = resolve_advanced_filters(node, &values, at, ctx, auto_trail)?;
        if auto_trail.is_some()
            && filters.iter().any(|filter| {
                matches!(
                    filter,
                    valle_draw::program::recording::FilterOp::VelocityBlur { .. }
                )
            })
        {
            screen_velocity_blurs.insert(node.key.clone());
        }
        for (css, lists) in css_filters
            .iter_mut()
            .zip([&mut filter_lists, &mut backdrop_filter_lists])
        {
            let Some(mut list) = css.remove(&node.key) else {
                continue;
            };
            for filter in list.iter_mut().flatten() {
                if let valle_draw::program::recording::FilterOp::FilmGrain { seed, .. } = filter {
                    let frame = match resolved_style(node, &values, at, "motion-filter-frame")? {
                        Some(MotionValue::Number(frame)) => *frame as i64 as u32,
                        _ => ctx.local_frame,
                    };
                    *seed = seed.wrapping_add(frame.wrapping_mul(0x9e37_79b9));
                }
            }
            lists.insert(node.key.clone(), list);
        }
        if !filters.is_empty() {
            advanced_filters.insert(node.key.clone(), filters);
        }
        let backdrop_filters = resolve_backdrop_advanced_filters(node, &values, at)?;
        if !backdrop_filters.is_empty() {
            backdrop_advanced_filters.insert(node.key.clone(), backdrop_filters);
        }
        if let Some(fx) = resolve_layer_fx(node, &values, at)? {
            layer_fx.insert(node.key.clone(), fx);
        }
        if let NodeKind::Text {
            path: Some(path), ..
        } = &node.kind
        {
            text_paths.insert(node.key.clone(), path_value(path, &values, at)?.clone());
        }
        match &node.kind {
            NodeKind::Transition { effect, progress } => {
                let kernel = match effect {
                    TransitionEffect::Builtin {
                        kind,
                        params: bindings,
                    } => {
                        let params = bindings
                            .iter()
                            .map(|(name, value)| {
                                Ok((
                                    name.clone(),
                                    number_value(value, &values, at, "transition parameter")?,
                                ))
                            })
                            .collect::<Result<std::collections::BTreeMap<_, _>, LayoutError>>()?;
                        let params = kind.resolve_params(&params).map_err(|error| {
                            LayoutError::BadTransition {
                                node: node.key.clone(),
                                reason: match bindings.get(&error.parameter) {
                                    Some(NumberValue::Expr { expr }) => {
                                        format!("expression {}: {error}", expr.0)
                                    }
                                    _ => error.to_string(),
                                },
                            }
                        })?;
                        crate::layout::bridge::ResolvedTransitionKernel::Builtin {
                            kind: *kind,
                            params,
                        }
                    }
                    TransitionEffect::Shader {
                        program,
                        uniforms,
                        inputs,
                    } => crate::layout::bridge::ResolvedTransitionKernel::Shader(
                        crate::layout::bridge::ResolvedShaderLayer {
                            program: shader_program(program),
                            uniforms: shader_uniforms(&node.key, uniforms, &values, at)?,
                            inputs: inputs.clone(),
                        },
                    ),
                };
                let progress = number_value(progress, &values, at, "transition progress")?;
                if !(0.0..=1.0).contains(&progress) {
                    return Err(LayoutError::BadTransition {
                        node: node.key.clone(),
                        reason: format!("progress = {progress} must be finite and in [0, 1]"),
                    });
                }
                let children = &artifact.node_children
                    [node.children.start as usize..node.children.end as usize];
                transitions.insert(
                    node.key.clone(),
                    crate::layout::bridge::ResolvedTransition {
                        kernel,
                        progress: progress as f32,
                        children: [
                            artifact.nodes[children[0].0 as usize].key.clone(),
                            artifact.nodes[children[1].0 as usize].key.clone(),
                        ],
                    },
                );
            }
            NodeKind::Scene3D { .. } => {}
            NodeKind::ShaderLayer {
                program,
                uniforms,
                inputs,
            } => {
                shaders.insert(
                    node.key.clone(),
                    crate::layout::bridge::ResolvedShaderLayer {
                        program: shader_program(program),
                        uniforms: shader_uniforms(&node.key, uniforms, &values, at)?,
                        inputs: inputs.clone(),
                    },
                );
            }
            NodeKind::GeometryBatch { batch } => {
                let seconds = match &batch.positions {
                    BatchPositions::Static { .. } => 0.0,
                    BatchPositions::Particles { time, .. } => match values.get(time.0 as usize) {
                        Some(MotionValue::Number(value)) => *value,
                        _ => {
                            return Err(LayoutError::Eval(EvalError::TypeMismatch {
                                at,
                                op: "particles time",
                            }));
                        }
                    },
                };
                let field_progress = |expr: Option<ExprId>, op: &'static str| {
                    let Some(expr) = expr else {
                        return Ok(0.0);
                    };
                    match value(&values, expr, at)? {
                        MotionValue::Number(value) if value.is_finite() => Ok(*value),
                        _ => Err(LayoutError::Eval(EvalError::TypeMismatch { at, op })),
                    }
                };
                let position_progress = field_progress(
                    batch.position_field.as_ref().map(|field| field.progress),
                    "GeometryBatch position field progress",
                )?;
                let size_progress = field_progress(
                    batch.size_field.as_ref().map(|field| field.progress),
                    "GeometryBatch size field progress",
                )?;
                let fill_progress = field_progress(
                    batch.fill_field.as_ref().map(|field| field.progress),
                    "GeometryBatch fill field progress",
                )?;
                let opacity_progress = field_progress(
                    batch.opacity_field.as_ref().map(|field| field.progress),
                    "GeometryBatch opacity field progress",
                )?;
                let rotation_progress = field_progress(
                    batch.rotation_field.as_ref().map(|field| field.progress),
                    "GeometryBatch rotation field progress",
                )?;
                let skew_x_progress = field_progress(
                    batch.skew_x_field.as_ref().map(|field| field.progress),
                    "GeometryBatch skewX field progress",
                )?;
                let stroke_width_progress = field_progress(
                    batch
                        .stroke_width_field
                        .as_ref()
                        .map(|field| field.progress),
                    "GeometryBatch strokeWidth field progress",
                )?;
                let mut row_identity = Vec::new();
                let instances = resolve_geometry_batch_with_identity(
                    batch,
                    prepared
                        .particle_trajectories
                        .get(at)
                        .and_then(Option::as_ref),
                    seconds,
                    valle_motion::BatchFieldProgress {
                        position: position_progress,
                        size: size_progress,
                        fill: fill_progress,
                        opacity: opacity_progress,
                        rotation: rotation_progress,
                        skew_x: skew_x_progress,
                        stroke_width: stroke_width_progress,
                    },
                    &mut row_identity,
                );
                batches.insert(
                    node.key.clone(),
                    crate::layout::bridge::BatchContent {
                        semantic_keys: row_identity
                            .iter()
                            .map(|identity| {
                                batch
                                    .semantic_keys
                                    .get(identity.index)
                                    .cloned()
                                    .unwrap_or_else(|| format!("{}/{}", node.key, identity.index))
                            })
                            .collect(),
                        row_identity,
                        geometry: match &batch.geometry {
                            GeometryBatchGeometry::Circle => {
                                valle_draw::program::recording::BatchGeometry::Circle
                            }
                            GeometryBatchGeometry::Rect => {
                                valle_draw::program::recording::BatchGeometry::Rect
                            }
                            GeometryBatchGeometry::Path { .. } => {
                                valle_draw::program::recording::BatchGeometry::Path
                            }
                            GeometryBatchGeometry::Image { .. } => {
                                valle_draw::program::recording::BatchGeometry::Image
                            }
                        },
                        path: match &batch.geometry {
                            GeometryBatchGeometry::Path { path } => Some(path.clone()),
                            _ => None,
                        },
                        atlas: match &batch.geometry {
                            GeometryBatchGeometry::Image { source, src } => {
                                Some((source.clone(), *src))
                            }
                            _ => None,
                        },
                        instances,
                        stroke_colors: Vec::new(),
                        dash_offsets: Vec::new(),
                        path_style: None,
                        exact_circle_paths: false,
                    },
                );
            }
            NodeKind::InstanceBatch { group } => {
                let group_index = *group as usize;
                let activation = prepared
                    .instance_activation
                    .get(group_index)
                    .and_then(Option::as_ref)
                    .and_then(|gate| {
                        artifact
                            .instance_groups
                            .get(group_index)
                            .and_then(|group| gate.select(ctx, group.rows()))
                    });
                let group = prepared
                    .materialized_instance_groups
                    .get(group_index)
                    .and_then(Option::as_ref)
                    .or_else(|| artifact.instance_groups.get(group_index))
                    .ok_or(LayoutError::BadNode { at })?;
                let plan = prepared
                    .instance_eval_plans
                    .get(group_index)
                    .ok_or(LayoutError::BadNode { at })?;
                batches.insert(
                    node.key.clone(),
                    resolve_instance_batch_cached(
                        prepared,
                        group_index,
                        group,
                        plan,
                        activation.as_ref(),
                        ctx,
                        props,
                        viewport,
                        shared
                            .as_deref_mut()
                            .and_then(|cache| cache.instances.get_mut(group_index)),
                    )
                    .map_err(|reason| LayoutError::Instance {
                        group: group_index as u32,
                        node: node.key.clone(),
                        reason: Box::new(reason),
                    })?,
                );
            }
            NodeKind::Path {
                d,
                fill,
                stroke,
                trim_start,
                trim_end,
                arrow_start,
                arrow_end,
            } => {
                let path = path_value(d, &values, at)?;
                let start = number_value(trim_start, &values, at, "trimStart")?;
                let end = number_value(trim_end, &values, at, "trimEnd")?;
                let path = path
                    .trim(start, end)
                    .map_err(|reason| LayoutError::Eval(EvalError::BadGeometry { at, reason }))?;
                paths.insert(
                    node.key.clone(),
                    crate::layout::bridge::PathContent {
                        verbs: path.verbs,
                        points: path.points,
                        fill: fill
                            .as_ref()
                            .map(|paint| paint_value(paint, &values, at, &node.key))
                            .transpose()?,
                        stroke:
                            stroke
                                .as_ref()
                                .map(
                                    |stroke| -> Result<
                                        crate::layout::bridge::ResolvedStroke,
                                        LayoutError,
                                    > {
                                        Ok(crate::layout::bridge::ResolvedStroke {
                                            paint: paint_value(
                                                &stroke.paint,
                                                &values,
                                                at,
                                                &node.key,
                                            )?,
                                            width: {
                                                let width = number_value(
                                                    &stroke.width,
                                                    &values,
                                                    at,
                                                    "strokeWidth",
                                                )?;
                                                if width < 0.0 {
                                                    return Err(LayoutError::BadStyle {
                                                        node: node.key.clone(),
                                                        declarations: "strokeWidth".into(),
                                                        reason: "strokeWidth must be non-negative"
                                                            .into(),
                                                    });
                                                }
                                                width
                                            },
                                            dash: stroke.dash.clone(),
                                            dash_offset: number_value(
                                                &stroke.dash_offset,
                                                &values,
                                                at,
                                                "strokeDashoffset",
                                            )?,
                                            cap: stroke.cap,
                                            join: stroke.join,
                                            miter_limit: stroke.miter_limit,
                                        })
                                    },
                                )
                                .transpose()?,
                        arrow_start: arrow_start.clone(),
                        arrow_end: arrow_end.clone(),
                    },
                );
            }
            NodeKind::Video {
                source,
                source_start,
                speed,
            } => {
                // Temporal effects must sample video at the same exact subframe chosen for the
                // rest of the subtree, not at its quantized frame address.
                let local_seconds = scoped_seconds[at];
                videos.insert(
                    node.key.clone(),
                    crate::layout::bridge::VideoContent {
                        asset: source.clone(),
                        source_time_s: (number_value(
                            source_start,
                            &values,
                            at,
                            "video.sourceStart",
                        )? + local_seconds
                            * number_value(speed, &values, at, "video.speed")?)
                        .max(0.0),
                    },
                );
            }
            NodeKind::Clip { path, fill_rule } => {
                clips.insert(
                    node.key.clone(),
                    crate::layout::bridge::ClipContent {
                        path: path_value(path, &values, at)?.clone(),
                        fill_rule: *fill_rule,
                    },
                );
            }
            NodeKind::Mask { source, mode, rect } => {
                let source = match source {
                    MaskValue::Paint { paint } => crate::layout::bridge::ResolvedMaskSource::Paint(
                        paint_value(paint, &values, at, &node.key)?,
                    ),
                    MaskValue::Image { source } => {
                        crate::layout::bridge::ResolvedMaskSource::Image(source.clone())
                    }
                    MaskValue::Subtree { source } => {
                        crate::layout::bridge::ResolvedMaskSource::Subtree(source.clone())
                    }
                };
                masks.insert(
                    node.key.clone(),
                    crate::layout::bridge::MaskContent {
                        source,
                        mode: *mode,
                        rect: rect_value(rect, &values, at, &node.key)?,
                    },
                );
            }
            _ => {}
        }
    }

    let glass = Rc::new(resolve_glass(artifact, &values)?);

    if let Some(timings) = timings {
        timings.finish_ms = elapsed(&mut started);
    }
    let (shutters, echoes) = if temporal_samples {
        let painted = if prepared.skip_unpainted_samples
            && !prepared.dependencies.temporal_nodes().is_empty()
        {
            painted_temporal_scopes(artifact, prepared, &root, &layout, &keys, &render_keys)
        } else {
            None
        };
        let mut trees = SampleTrees::new();
        let shutters = resolve_shutter_samples(
            prepared,
            ctx,
            props,
            opts,
            painted.as_ref(),
            &mut trees,
            shared.as_deref_mut(),
        )?;
        let echoes = resolve_echo_samples(
            prepared,
            ctx,
            props,
            opts,
            painted.as_ref(),
            &mut trees,
            shared,
        )?;
        (shutters, echoes)
    } else {
        (HashMap::new(), HashMap::new())
    };
    let mut background_colors = authored_style_colors(artifact, &values, "background-color");
    let mut text_colors = authored_style_colors(artifact, &values, "color");
    let mut paint_colors: HashMap<_, _> = artifact
        .nodes
        .iter()
        .map(|node| {
            (
                node.key.clone(),
                crate::style::precise_color::PreciseCssColors::new(&node.styles, &values),
            )
        })
        .collect();
    for rows in layout_instances.values() {
        for row in rows {
            visit_layout_instance_node(row, &mut |node| {
                paint_colors.insert(node.key.clone(), node.paint_colors.clone());
                if let Some(color) = node.background_color {
                    background_colors.insert(node.key.clone(), color);
                }
                if let Some(color) = node.text_color {
                    text_colors.insert(node.key.clone(), color);
                }
                if let Some(text) = &node.text {
                    node_texts.insert(node.key.clone(), text.clone());
                }
            });
        }
    }
    let text_stroke_colors = authored_style_colors(artifact, &values, "-webkit-text-stroke-color");
    Ok(LayoutTree {
        root,
        inactive_nodes: Rc::new(inactive_nodes),
        viewport: opts.viewport,
        layout,
        values,
        background_colors: Rc::new(background_colors),
        text_colors: Rc::new(text_colors),
        text_stroke_colors: Rc::new(text_stroke_colors),
        paint_colors: Rc::new(paint_colors),
        keys,
        units: Rc::new(units),
        render_keys: Rc::new(render_keys),
        node_texts: Rc::new(node_texts),
        paths: Rc::new(paths),
        batches: Rc::new(batches),
        text_paths: Rc::new(text_paths),
        clips: Rc::new(clips),
        masks: Rc::new(masks),
        videos: Rc::new(videos),
        advanced_filters: Rc::new(advanced_filters),
        filter_lists: Rc::new(filter_lists),
        backdrop_filter_lists: Rc::new(backdrop_filter_lists),
        screen_velocity_blurs: Rc::new(screen_velocity_blurs),
        backdrop_advanced_filters: Rc::new(backdrop_advanced_filters),
        transitions: Rc::new(transitions),
        shutters: Rc::new(shutters),
        echoes: Rc::new(echoes),
        shaders: Rc::new(shaders),
        scene3d: Rc::new(scene3d),
        layer_fx: Rc::new(layer_fx),
        blend_spaces: Rc::new(blend_spaces),
        css_3d_planes: Rc::new(css_3d_planes),
        formulas: Rc::new(formulas),
        glass,
    })
}

/// Each Glass and GlassField's values for this frame. Emit draws them from these and the frame's
/// boxes and transforms.
fn resolve_glass(
    artifact: &SceneArtifact,
    values: &[MotionValue],
) -> Result<crate::layout::bridge::GlassFrame, LayoutError> {
    use crate::layout::bridge::{GlassFrame, GlassMaterial, ResolvedGlass, ResolvedGlassField};

    let mut frame = GlassFrame::default();
    if !artifact
        .nodes
        .iter()
        .any(|node| matches!(node.kind, NodeKind::Glass(_) | NodeKind::GlassField(_)))
    {
        return Ok(frame);
    }
    let bad = |at: usize, declarations: &str, reason: String| LayoutError::BadStyle {
        node: artifact.nodes[at].key.clone(),
        declarations: declarations.into(),
        reason,
    };
    let material = |material: &crate::glass::GlassMaterialBinding,
                    light: &crate::glass::GlassLightBinding,
                    at: usize|
     -> Result<GlassMaterial, LayoutError> {
        let length = |value: &NumberValue, op: &'static str| {
            let length = number_value(value, values, at, op)?;
            if length.is_finite() && length >= 0.0 {
                Ok(length)
            } else {
                Err(bad(
                    at,
                    "Glass material",
                    format!("{op} = {length} must be a finite non-negative number"),
                ))
            }
        };
        let direction = point_value(&light.direction, values, at, "glass light direction")?;
        if !(direction.x.is_finite()
            && direction.y.is_finite()
            && (direction.x != 0.0 || direction.y != 0.0))
        {
            return Err(bad(
                at,
                "Glass light",
                "light direction must be a finite non-zero vector".into(),
            ));
        }
        let elevation = number_value(&light.elevation, values, at, "glass light elevation")?;
        if !(0.0..=1.0).contains(&elevation) {
            return Err(bad(
                at,
                "Glass light",
                format!("light elevation = {elevation} must be in 0..=1"),
            ));
        }
        Ok(GlassMaterial {
            bevel: material
                .bevel
                .as_ref()
                .map(|value| length(value, "glass bevel"))
                .transpose()?,
            refraction: material
                .refraction
                .as_ref()
                .map(|value| length(value, "glass refraction"))
                .transpose()?,
            dispersion: length(&material.dispersion, "glass dispersion")?,
            frost: length(&material.frost, "glass frost")?,
            saturation: length(&material.saturation, "glass saturation")?,
            tint: color_value(&material.tint, values, at)?,
            light: [direction.x, direction.y],
            elevation,
            intensity: length(&light.intensity, "glass light intensity")?,
        })
    };

    let membership = crate::glass::field_membership(artifact);
    for (at, node) in artifact.nodes.iter().enumerate() {
        match &node.kind {
            NodeKind::Glass(glass) => {
                let presence = number_value(&glass.presence, values, at, "glass presence")?;
                if !(0.0..=1.0).contains(&presence) {
                    return Err(bad(
                        at,
                        "Glass presence",
                        format!("presence = {presence} must be in 0..=1"),
                    ));
                }
                let path = match &glass.path {
                    Some(path) => Some(crate::layout::bridge::ClipContent {
                        path: path_value(path, values, at)?.clone(),
                        fill_rule: glass.fill_rule,
                    }),
                    None => None,
                };
                let material = match (&glass.material, &glass.light) {
                    (Some(binding), Some(light)) => Some(material(binding, light, at)?),
                    _ => None,
                };
                frame.glass.insert(
                    node.key.clone(),
                    ResolvedGlass {
                        presence,
                        path,
                        material,
                    },
                );
            }
            NodeKind::GlassField(field) => {
                frame.fields.insert(
                    node.key.clone(),
                    ResolvedGlassField {
                        material: material(&field.material, &field.light, at)?,
                        merge: field.merge.distance,
                        members: Vec::new(),
                    },
                );
            }
            _ => {}
        }
    }
    for (at, field) in membership.iter().enumerate() {
        let Some(field) = field else {
            continue;
        };
        let node = &artifact.nodes[at];
        let field = &artifact.nodes[*field].key;
        frame.field_of.insert(node.key.clone(), field.clone());
        if matches!(node.kind, NodeKind::Glass(_))
            && let Some(resolved) = frame.fields.get_mut(field)
        {
            resolved.members.push(node.key.clone());
        }
        if let NodeKind::InstanceLayout { group } = node.kind
            && let Some(group) = artifact.instance_groups.get(group as usize)
        {
            for row in 0..group.rows() {
                if let Some(key) = group.keys.key_at(row) {
                    frame.field_of.insert(key, field.clone());
                }
            }
        }
    }
    Ok(frame)
}

fn resolve_scene3d_requests(
    artifact: &SceneArtifact,
    values: &[MotionValue],
) -> Result<HashMap<String, crate::layout::bridge::Scene3DRequest>, LayoutError> {
    let mut requests = HashMap::new();
    for (at, node) in artifact.nodes.iter().enumerate() {
        let NodeKind::Scene3D { scene, frame } = &node.kind else {
            continue;
        };
        let scalar = |binding: &NumberValue, op: &'static str| {
            number_value(binding, values, at, op).map(|value| value as f32)
        };
        let vector = |v: &[NumberValue; 3]| -> Result<valle_motion::scene3d::Vec3, LayoutError> {
            Ok(valle_motion::scene3d::Vec3::new(
                scalar(&v[0], "scene3d vector x")?,
                scalar(&v[1], "scene3d vector y")?,
                scalar(&v[2], "scene3d vector z")?,
            ))
        };
        let color = |v: &ColorValue| -> Result<valle_motion::scene3d::Color4, LayoutError> {
            let c = color_value(v, values, at)?;
            Ok(valle_motion::scene3d::Color4(c.to_srgb_straight()))
        };
        let camera = valle_motion::scene3d::CameraFrameState {
            orthographic_height: frame
                .camera
                .orthographic_height
                .as_ref()
                .map(|h| scalar(h, "scene3d orthographic height"))
                .transpose()?,
            position: vector(&frame.camera.position)?,
            target: vector(&frame.camera.target)?,
            near: scalar(&frame.camera.near, "scene3d camera near")?,
            far: scalar(&frame.camera.far, "scene3d camera far")?,
            fov_y_degrees: scalar(&frame.camera.fov_y_degrees, "scene3d camera fov")?,
            depth_of_field: frame
                .camera
                .depth_of_field
                .as_ref()
                .map(|dof| {
                    Ok::<_, LayoutError>(valle_motion::scene3d::DepthOfFieldState {
                        focus_distance: scalar(
                            &dof.focus_distance,
                            "scene3d camera focus distance",
                        )?,
                        max_blur_radius: scalar(
                            &dof.max_blur_radius,
                            "scene3d camera maximum blur radius",
                        )?,
                    })
                })
                .transpose()?,
        }
        .with_orbit(
            scalar(&frame.camera.orbit_yaw_degrees, "scene3d camera yaw")?,
            scalar(&frame.camera.orbit_pitch_degrees, "scene3d camera pitch")?,
            frame
                .camera
                .distance
                .as_ref()
                .map(|n| scalar(n, "scene3d camera distance"))
                .transpose()?,
        )
        .map_err(|error| LayoutError::BadScene3D {
            node: node.key.clone(),
            reason: error.to_string(),
        })?;
        let material = |m: &crate::Scene3DMaterialBinding| -> Result<valle_motion::scene3d::MaterialFrameState, LayoutError> {
            Ok(valle_motion::scene3d::MaterialFrameState {
                ior: m.ior.as_ref().map(|v| scalar(v, "material ior")).transpose()?,
                specular_intensity: m.specular_intensity.as_ref().map(|v| scalar(v, "material specularIntensity")).transpose()?,
                clearcoat: m.clearcoat.as_ref().map(|v| scalar(v, "material clearcoat")).transpose()?,
                clearcoat_roughness: m.clearcoat_roughness.as_ref().map(|v| scalar(v, "material clearcoatRoughness")).transpose()?,
                specular_color: m.specular_color.as_ref().map(color).transpose()?,
                color: m.color.as_ref().map(color).transpose()?,
                attenuation_color: m.attenuation_color.as_ref().map(color).transpose()?,
                transmission: m.transmission.as_ref().map(|v| scalar(v, "material transmission")).transpose()?,
                thickness: m.thickness.as_ref().map(|v| scalar(v, "material thickness")).transpose()?,
                attenuation_distance: m.attenuation_distance.as_ref().map(|v| scalar(v, "material attenuationDistance")).transpose()?,
                emissive: m.emissive.as_ref().map(color).transpose()?,
                metallic: m.metallic.as_ref().map(|v| scalar(v, "material metallic")).transpose()?,
                roughness: m.roughness.as_ref().map(|v| scalar(v, "material roughness")).transpose()?,
                emissive_intensity: m.emissive_intensity.as_ref().map(|v| scalar(v, "material emissive_intensity")).transpose()?,
                normal_scale: m.normal_scale.as_ref().map(|v| scalar(v, "material normal_scale")).transpose()?,
                occlusion_strength: m.occlusion_strength.as_ref().map(|v| scalar(v, "material occlusion_strength")).transpose()?,
                alpha_cutoff: m.alpha_cutoff.as_ref().map(|v| scalar(v, "material alpha_cutoff")).transpose()?,
            })
        };
        let resolved = valle_motion::scene3d::Frame3DState {
            exposure: scalar(&frame.exposure, "scene3d exposure")?,
            environment_intensity: scalar(
                &frame.environment_intensity,
                "scene3d environment intensity",
            )?,
            environment_rotation_degrees: scalar(
                &frame.environment_rotation_degrees,
                "scene3d environment rotation",
            )?,
            camera,
            meshes: frame
                .meshes
                .iter()
                .map(|mesh| {
                    Ok(valle_motion::scene3d::MeshFrameState {
                        parents: mesh
                            .parents
                            .iter()
                            .map(|p| {
                                Ok(valle_motion::scene3d::Transform3D {
                                    translation: vector(&p.translation)?,
                                    rotation_degrees: vector(&p.rotation_degrees)?,
                                    scale: vector(&p.scale)?,
                                })
                            })
                            .collect::<Result<Vec<_>, LayoutError>>()?,
                        key: mesh.key.clone(),
                        material: material(&mesh.material)?,
                        material_overrides: mesh
                            .material_overrides
                            .iter()
                            .map(|m| {
                                Ok(valle_motion::scene3d::MaterialOverrideState {
                                    id: m.id,
                                    material: material(&m.material)?,
                                })
                            })
                            .collect::<Result<Vec<_>, LayoutError>>()?,
                        animation_time: mesh
                            .animation_time
                            .as_ref()
                            .map(|time| scalar(time, "scene3d animation time"))
                            .transpose()?,
                        transform: valle_motion::scene3d::Transform3D {
                            translation: vector(&mesh.transform.translation)?,
                            rotation_degrees: vector(&mesh.transform.rotation_degrees)?,
                            scale: vector(&mesh.transform.scale)?,
                        },
                        nodes: mesh
                            .nodes
                            .iter()
                            .map(|n| {
                                Ok(valle_motion::scene3d::NodeFrameState {
                                    id: n.id,
                                    transform: valle_motion::scene3d::Transform3D {
                                        translation: vector(&n.transform.translation)?,
                                        rotation_degrees: vector(&n.transform.rotation_degrees)?,
                                        scale: vector(&n.transform.scale)?,
                                    },
                                })
                            })
                            .collect::<Result<Vec<_>, LayoutError>>()?,
                    })
                })
                .collect::<Result<Vec<_>, LayoutError>>()?,
            lights: frame
                .lights
                .iter()
                .map(
                    |light| -> Result<valle_motion::scene3d::LightFrameState, LayoutError> {
                        use crate::Scene3DLightBinding as Binding;
                        use valle_motion::scene3d::LightFrameState as Light;
                        Ok(match light {
                            Binding::RectArea {
                                color: c,
                                intensity,
                                position,
                                target,
                                width,
                                height,
                                roll,
                            } => Light::RectArea {
                                color: color(c)?,
                                intensity: scalar(intensity, "area intensity")?,
                                position: vector(position)?,
                                target: vector(target)?,
                                width: scalar(width, "area width")?,
                                height: scalar(height, "area height")?,
                                roll: scalar(roll, "area roll")?,
                            },
                            Binding::Point {
                                color: c,
                                intensity,
                                position,
                                range,
                            } => Light::Point {
                                color: color(c)?,
                                intensity: scalar(intensity, "scene3d light intensity")?,
                                position: vector(position)?,
                                range: scalar(range, "scene3d light range")?,
                            },
                            Binding::Spot {
                                color: c,
                                intensity,
                                position,
                                target,
                                range,
                                angle,
                                penumbra,
                            } => Light::Spot {
                                color: color(c)?,
                                intensity: scalar(intensity, "scene3d light intensity")?,
                                position: vector(position)?,
                                target: vector(target)?,
                                range: scalar(range, "scene3d light range")?,
                                angle: scalar(angle, "scene3d light angle")?,
                                penumbra: scalar(penumbra, "scene3d light penumbra")?,
                            },

                            Binding::Ambient {
                                color: c,
                                intensity,
                            } => Light::Ambient {
                                color: color(c)?,
                                intensity: scalar(intensity, "scene3d light intensity")?,
                            },
                            Binding::Directional {
                                color: c,
                                direction,
                                intensity,
                            } => Light::Directional {
                                color: color(c)?,
                                direction: vector(direction)?,
                                intensity: scalar(intensity, "scene3d light intensity")?,
                            },
                            Binding::Hemisphere {
                                sky_color,
                                ground_color,
                                direction,
                                intensity,
                            } => Light::Hemisphere {
                                sky_color: color(sky_color)?,
                                ground_color: color(ground_color)?,
                                direction: vector(direction)?,
                                intensity: scalar(intensity, "scene3d light intensity")?,
                            },
                        })
                    },
                )
                .collect::<Result<Vec<_>, _>>()?,
        };
        resolved
            .validate_for(scene)
            .map_err(|error| LayoutError::BadScene3D {
                node: node.key.clone(),
                reason: error.to_string(),
            })?;
        requests.insert(
            node.key.clone(),
            crate::layout::bridge::Scene3DRequest {
                texture: node.children.start != node.children.end,
                provider_key: format!("scene3d://{}", node.key),
                scene: scene.clone(),
                frame: resolved,
            },
        );
    }
    Ok(requests)
}

fn project_scene3d_anchors(
    artifact: &SceneArtifact,
    requests: &HashMap<String, crate::layout::bridge::Scene3DRequest>,
    content_boxes: &BTreeMap<String, valle_draw::Rect>,
) -> Result<BTreeMap<(String, String), valle_draw::Point>, LayoutError> {
    let mut required = BTreeMap::<String, Vec<String>>::new();
    for expr in &artifact.exprs {
        if let Expr::Project3D {
            scene_key,
            anchor_key,
        } = expr
        {
            required
                .entry(scene_key.clone())
                .or_default()
                .push(anchor_key.clone());
        }
    }
    let mut points = BTreeMap::new();
    for (scene_key, anchor_keys) in required {
        let request = requests
            .get(&scene_key)
            .expect("Artifact validation proved the project3d Scene3D target");
        let content = content_boxes
            .get(&scene_key)
            .ok_or_else(|| LayoutError::BadScene3D {
                node: scene_key.clone(),
                reason: "project3d target has no content box".into(),
            })?;
        let width = content.width.ceil().max(0.0) as u32;
        let height = content.height.ceil().max(0.0) as u32;
        let projected =
            valle_motion::scene3d::project_anchors(&request.scene, &request.frame, width, height)
                .map_err(|error| LayoutError::BadScene3D {
                node: scene_key.clone(),
                reason: format!("anchor projection failed: {error}"),
            })?;
        for anchor_key in anchor_keys {
            let (parent, key) = anchor_key
                .split_once("::")
                .expect("Artifact validation proved the anchor address");
            let anchor = request
                .scene
                .anchors
                .iter()
                .zip(&projected)
                .find(|(spec, _)| spec.parent == parent && spec.key == key)
                .map(|(_, projection)| projection)
                .expect("Artifact validation proved the project3d anchor target");
            if !anchor.in_front {
                return Err(LayoutError::BadScene3D {
                    node: scene_key.clone(),
                    reason: format!(
                        "project3d anchor `{anchor_key}` is behind the near plane or beyond far"
                    ),
                });
            }
            points.insert(
                (scene_key.clone(), anchor_key),
                valle_draw::Point::new(
                    content.x + f64::from(anchor.screen[0]),
                    content.y + f64::from(anchor.screen[1]),
                ),
            );
        }
    }
    Ok(points)
}

fn resolved_style<'a>(
    node: &'a valle_motion::SceneNode,
    values: &'a [MotionValue],
    at: usize,
    property: &str,
) -> Result<Option<&'a MotionValue>, LayoutError> {
    let Some(binding) = node.styles.iter().find(|style| style.property == property) else {
        return Ok(None);
    };
    Ok(Some(match &binding.value {
        StyleValue::Static { value } => value,
        StyleValue::Expr { expr } => value(values, *expr, at)?,
    }))
}

/// Resolve each flow instance's shared expression program once, then let Takumi lay out every
/// resulting sibling. Invisible rows retain their boxes so later siblings do not shift.
fn eval_layout_instances(
    prepared: &PreparedScene,
    ctx: &valle_motion::MotionContext,
    props: &ResolvedProps,
    viewport: Option<(f64, f64)>,
    inactive_nodes: &[bool],
    mut shared: Option<&mut RequestValueCache>,
) -> Result<LayoutInstanceRows, LayoutError> {
    let artifact = prepared.artifact();
    let mut result = HashMap::new();
    for (node_at, node) in artifact.nodes.iter().enumerate() {
        if inactive_nodes.get(node_at).copied().unwrap_or(false) {
            continue;
        }
        let NodeKind::InstanceLayout { group: index } = node.kind else {
            continue;
        };
        let group_at = index as usize;
        let group = prepared
            .materialized_instance_groups
            .get(group_at)
            .and_then(Option::as_ref)
            .or_else(|| artifact.instance_groups.get(group_at))
            .ok_or(LayoutError::BadNode { at: node_at })?;
        let plan = prepared
            .instance_eval_plans
            .get(group_at)
            .ok_or(LayoutError::BadNode { at: node_at })?;
        let mut rows = Vec::with_capacity(group.rows());
        crate::eval::try_for_each_instance_rows_planned(
            group,
            plan,
            EvalInputs {
                ctx,
                props,
                unit: None,
                viewport,
            },
            0..group.rows(),
            shared
                .as_deref_mut()
                .and_then(|cache| cache.instances.get_mut(group_at)),
            |at, values| {
                rows.push(resolve_layout_instance_node(
                    group,
                    &group.template,
                    &group.template_children,
                    at,
                    values,
                )?);
                Ok::<(), LayoutError>(())
            },
        )
        .map_err(|reason| LayoutError::Instance {
            group: index,
            node: node.key.clone(),
            reason: Box::new(reason),
        })?;
        result.insert(index, rows);
    }
    Ok(result)
}

fn resolve_layout_instance_node(
    group: &InstanceGroup,
    template: &valle_motion::SceneNode,
    children: &[InstanceTemplateNode],
    row: usize,
    values: &[MotionValue],
) -> Result<LayoutInstanceRow, LayoutError> {
    let visible = match template.visibility {
        Some(expr) => match value(values, expr, row)? {
            MotionValue::Bool(value) => *value,
            _ => {
                return Err(LayoutError::Eval(EvalError::TypeMismatch {
                    at: row,
                    op: "instance visibility",
                }));
            }
        },
        None => true,
    };
    let color = |property| -> Result<_, LayoutError> {
        Ok(match resolved_style(template, values, row, property)? {
            Some(MotionValue::Color(color)) => Some(*color),
            _ => None,
        })
    };
    let text = match &template.kind {
        NodeKind::Text { text, .. } => Some(text_value(text, values, row)?),
        _ => None,
    };
    let mut active_classes = Vec::with_capacity(template.class_names.len());
    for class in &template.class_names {
        let active = match template.class_conditions.get(class) {
            Some(expr) => match value(values, *expr, row)? {
                MotionValue::Bool(active) => *active,
                _ => {
                    return Err(LayoutError::Eval(EvalError::TypeMismatch {
                        at: row,
                        op: "instance className condition",
                    }));
                }
            },
            None => true,
        };
        if active {
            active_classes.push(class.as_str());
        }
    }
    Ok(LayoutInstanceRow {
        key: group
            .key_for_node(row, &template.key)
            .ok_or(LayoutError::BadNode { at: row })?,
        class_name: active_classes.join(" "),
        declarations: declarations(template, values, visible, row)?,
        background_color: color("background-color")?,
        text_color: color("color")?,
        paint_colors: crate::style::precise_color::PreciseCssColors::new(&template.styles, values),
        text,
        children: children
            .iter()
            .map(|child| {
                resolve_layout_instance_node(group, &child.node, &child.children, row, values)
            })
            .collect::<Result<_, _>>()?,
    })
}

#[expect(
    clippy::too_many_arguments,
    reason = "Keep the layout pass inputs and its independently borrowed caches explicit."
)]
fn resolve_instance_batch_cached(
    prepared: &PreparedScene,
    group_index: usize,
    group: &valle_motion::InstanceGroup,
    plan: &InstanceEvalPlan,
    activation: Option<&InstanceRowSelection>,
    ctx: &valle_motion::MotionContext,
    props: &ResolvedProps,
    viewport: Option<(f64, f64)>,
    shared: Option<&mut SampleValueCache>,
) -> Result<crate::layout::bridge::BatchContent, LayoutError> {
    let rows = activation.map_or(0..group.rows(), |selection| selection.rows.clone());
    // Limit initial caching to expensive full solid Circle batches in scenes with Echo. Other
    // geometry and non-overlapping Shutter samples keep their existing allocation behavior.
    let eligible = plan.work_for_rows(rows.len()) >= 8192
        && group.circle_template_parameters().is_some()
        && prepared
            .dependencies
            .temporal_nodes()
            .iter()
            .any(|node| matches!(node.sampling, TemporalSampling::Echo { .. }))
        && prepared
            .instance_cache
            .lock()
            .ok()
            .is_some_and(|cache| cache.enabled());
    let key = eligible
        .then(|| {
            super::instance_cache::BatchKey::new(
                group_index,
                ctx,
                plan.reads_host,
                props,
                viewport,
                rows,
            )
        })
        .flatten();
    if let Some(key) = &key
        && let Some(content) = prepared
            .instance_cache
            .lock()
            .ok()
            .and_then(|mut cache| cache.get(key))
    {
        return Ok(content);
    }
    let content = resolve_instance_batch(group, plan, activation, ctx, props, viewport, shared)?;
    if let Some(key) = key
        && let Ok(mut cache) = prepared.instance_cache.lock()
    {
        cache.insert(key, &content);
    }
    Ok(content)
}

fn resolve_instance_batch(
    group: &valle_motion::InstanceGroup,
    plan: &InstanceEvalPlan,
    activation: Option<&InstanceRowSelection>,
    ctx: &valle_motion::MotionContext,
    props: &ResolvedProps,
    viewport: Option<(f64, f64)>,
    shared: Option<&mut SampleValueCache>,
) -> Result<crate::layout::bridge::BatchContent, LayoutError> {
    use valle_draw::program::recording::BatchGeometry;
    use valle_draw::program::{InstancePathStyle, StrokeCap, StrokeJoin};

    let circle = group.circle_template_parameters();
    let static_path = group.static_path_template();
    let (geometry, path) = if circle.is_some() {
        (BatchGeometry::Path, None)
    } else if let Some(path) = static_path {
        (BatchGeometry::Path, Some(path.clone()))
    } else if matches!(group.template.kind, NodeKind::Box) {
        (BatchGeometry::Rect, None)
    } else {
        return Err(LayoutError::BadStyle {
            node: group.template.key.clone(),
            declarations: "instance template".into(),
            reason: "expected a Box, full solid Circle, or translated solid static Path".into(),
        });
    };
    let rows = activation.map_or(0..group.rows(), |selection| selection.rows.clone());
    let mut semantic_keys = Vec::new();
    let mut instances = Vec::with_capacity(rows.len());
    let distinct_stroke = matches!(
        &group.template.kind,
        NodeKind::Path {
            fill: Some(fill),
            stroke: Some(stroke),
            ..
        } if &stroke.paint != fill
    );
    let mut stroke_colors = if distinct_stroke {
        Vec::with_capacity(rows.len())
    } else {
        Vec::new()
    };
    let dynamic_dash_offset = matches!(
        &group.template.kind,
        NodeKind::Path {
            stroke: Some(stroke),
            ..
        } if matches!(stroke.dash_offset, NumberValue::Expr { .. })
    );
    let mut dash_offsets = if dynamic_dash_offset {
        Vec::with_capacity(rows.len())
    } else {
        Vec::new()
    };
    let path_style = if static_path.is_some() {
        let NodeKind::Path { fill, stroke, .. } = &group.template.kind else {
            unreachable!("static path template is a Path");
        };
        let mut style = InstancePathStyle {
            fill: fill.is_some(),
            ..Default::default()
        };
        if let Some(stroke) = stroke {
            style.cap = match stroke.cap {
                valle_draw::Cap::Butt => StrokeCap::Butt,
                valle_draw::Cap::Round => StrokeCap::Round,
                valle_draw::Cap::Square => StrokeCap::Square,
            };
            style.join = match stroke.join {
                valle_draw::Join::Miter => StrokeJoin::Miter,
                valle_draw::Join::Round => StrokeJoin::Round,
                valle_draw::Join::Bevel => StrokeJoin::Bevel,
            };
            style.miter_limit = stroke.miter_limit as f32;
            style.dash = stroke.dash.as_ref().map_or_else(Vec::new, |dash| {
                dash.iter().map(|value| *value as f32).collect()
            });
            if let NumberValue::Static { value } = &stroke.dash_offset {
                style.dash_offset = *value as f32;
            }
        }
        (style != InstancePathStyle::default() || dynamic_dash_offset).then_some(style)
    } else {
        None
    };
    if rows.is_empty() {
        return Ok(crate::layout::bridge::BatchContent {
            row_identity: Vec::new(),
            semantic_keys,
            geometry,
            path,
            atlas: None,
            instances,
            stroke_colors,
            dash_offsets,
            path_style,
            exact_circle_paths: circle.is_some(),
        });
    }
    crate::eval::try_for_each_instance_rows_planned(
        group,
        plan,
        EvalInputs {
            ctx,
            props,
            unit: None,
            viewport,
        },
        rows,
        shared,
        |at, values| {
            let invalid = |property: &str| LayoutError::BadStyle {
                node: group.keys.key_at(at).unwrap_or_default(),
                declarations: format!(
                    "{property}: {}",
                    resolved_style(&group.template, values, at, property)
                        .ok()
                        .flatten()
                        .map(css_token)
                        .unwrap_or_else(|| "<missing>".into())
                ),
                reason: match group
                    .template
                    .styles
                    .iter()
                    .find(|style| style.property == property)
                    .map(|style| &style.value)
                {
                    Some(StyleValue::Expr { expr }) => format!(
                        "expression {}: {}",
                        expr.0,
                        if property == "opacity" {
                            "opacity must be finite and in [0, 1]"
                        } else {
                            "instance template needs a finite supported value"
                        }
                    ),
                    _ => "instance template needs a finite supported value".into(),
                },
            };
            if let Some(visibility) = group.template.visibility {
                match values.get(visibility.0 as usize) {
                    Some(MotionValue::Bool(true)) => {}
                    Some(MotionValue::Bool(false)) => return Ok(()),
                    _ => return Err(invalid("visible")),
                }
            }
            let opacity = match resolved_style(&group.template, values, at, "opacity")? {
                Some(MotionValue::Number(value)) if (0.0..=1.0).contains(value) => *value,
                None => 1.0,
                _ => return Err(invalid("opacity")),
            };
            if let Some((center_expr, radius_expr)) = circle {
                let center = match values.get(center_expr.0 as usize) {
                    Some(MotionValue::Point(value))
                        if value.x.is_finite() && value.y.is_finite() =>
                    {
                        *value
                    }
                    _ => return Err(invalid("cx/cy")),
                };
                let radius = match values.get(radius_expr.0 as usize) {
                    Some(MotionValue::Number(value)) if value.is_finite() && *value >= 0.0 => {
                        *value
                    }
                    _ => return Err(invalid("r")),
                };
                // A circle of radius zero draws nothing, as SVG's `r="0"`.
                if radius == 0.0 {
                    return Ok(());
                }
                let NodeKind::Path {
                    fill: Some(PaintValue::Solid { color }),
                    ..
                } = &group.template.kind
                else {
                    return Err(invalid("fill"));
                };
                let fill = color_value(color, values, at)?;
                semantic_keys.push(
                    group
                        .key_for_node(at, &group.template.key)
                        .unwrap_or_default(),
                );
                instances.push(valle_draw::program::recording::BatchInstance {
                    position: center,
                    size: valle_draw::Point::new(radius, radius),
                    color: fill.to_working(),
                    rotation: 0.0,
                    skew_x: 0.0,
                    stroke_width: 0.0,
                    opacity: opacity as f32,
                });
                return Ok(());
            }
            if static_path.is_some() {
                let position = match resolved_style(&group.template, values, at, "translate")? {
                    Some(MotionValue::Point(point)) => *point,
                    Some(MotionValue::Vec2(vector)) => valle_draw::Point::new(vector.x, vector.y),
                    Some(MotionValue::Length2(lengths))
                        if lengths.x.unit == valle_motion::value::LengthUnit::Px
                            && lengths.y.unit == valle_motion::value::LengthUnit::Px =>
                    {
                        valle_draw::Point::new(lengths.x.value, lengths.y.value)
                    }
                    _ => return Err(invalid("translate")),
                };
                if !position.x.is_finite() || !position.y.is_finite() {
                    return Err(invalid("translate"));
                }
                let NodeKind::Path { fill, stroke, .. } = &group.template.kind else {
                    return Err(invalid("Path"));
                };
                let fill_color = match fill {
                    Some(PaintValue::Solid { color }) => {
                        Some(color_value(color, values, at)?.to_working())
                    }
                    None => None,
                    _ => return Err(invalid("fill")),
                };
                let stroke_color = if distinct_stroke || fill_color.is_none() {
                    let Some(PaintValue::Solid { color }) =
                        stroke.as_ref().map(|stroke| &stroke.paint)
                    else {
                        return Err(invalid("stroke"));
                    };
                    Some(color_value(color, values, at)?.to_working())
                } else {
                    None
                };
                let base_color = fill_color
                    .or(stroke_color)
                    .ok_or_else(|| invalid("fill/stroke"))?;
                let stroke_width = match stroke {
                    Some(stroke) => {
                        let width = number_value(&stroke.width, values, at, "strokeWidth")?;
                        if width <= 0.0 || !(width as f32).is_finite() || (width as f32) == 0.0 {
                            return Err(invalid("strokeWidth"));
                        }
                        width as f32
                    }
                    None => 0.0,
                };
                let dash_offset = if dynamic_dash_offset {
                    let Some(stroke) = stroke else {
                        return Err(invalid("strokeDashoffset"));
                    };
                    let value =
                        number_value(&stroke.dash_offset, values, at, "strokeDashoffset")? as f32;
                    if !value.is_finite() {
                        return Err(invalid("strokeDashoffset"));
                    }
                    Some(value)
                } else {
                    None
                };
                // The transform list stands in for whichever of the rotate and scale
                // properties are not set.
                let transform = match resolved_style(&group.template, values, at, "transform")? {
                    Some(MotionValue::Str(value)) => {
                        valle_motion::artifact::path_instance_transform(value)
                            .ok_or_else(|| invalid("transform"))?
                    }
                    None => Default::default(),
                    _ => return Err(invalid("transform")),
                };
                let rotation = match (
                    resolved_style(&group.template, values, at, "rotate")?,
                    transform.rotate,
                ) {
                    (Some(MotionValue::Angle(angle)), None) => angle.as_degrees(),
                    (Some(MotionValue::Str(value)), None) => {
                        valle_motion::value::Angle::parse(value)
                            .map(valle_motion::value::Angle::as_degrees)
                            .ok_or_else(|| invalid("rotate"))?
                    }
                    (None, angle) => angle.map_or(0.0, valle_motion::value::Angle::as_degrees),
                    _ => return Err(invalid("rotate")),
                };
                if !rotation.is_finite() || !(rotation as f32).is_finite() {
                    return Err(invalid("rotate"));
                }
                let scale = match (
                    resolved_style(&group.template, values, at, "scale")?,
                    transform.scale,
                ) {
                    (Some(MotionValue::Number(value)), None) => {
                        valle_draw::Point::new(*value, *value)
                    }
                    (Some(MotionValue::Point(point)), None) => *point,
                    (None, scale) => scale.unwrap_or(valle_draw::Point::new(1.0, 1.0)),
                    _ => return Err(invalid("scale")),
                };
                if !(scale.x as f32).is_finite() || !(scale.y as f32).is_finite() {
                    return Err(invalid("scale"));
                }
                let skew_x = transform.skew_x;
                // CSS treats a singular scale as invisible. It can arise at an intermediate
                // frame even when the authored end points are nonzero.
                if (scale.x as f32) == 0.0 || (scale.y as f32) == 0.0 {
                    return Ok(());
                }
                semantic_keys.push(
                    group
                        .key_for_node(at, &group.template.key)
                        .unwrap_or_default(),
                );
                instances.push(valle_draw::program::recording::BatchInstance {
                    position,
                    size: scale,
                    color: base_color,
                    rotation: rotation as f32,
                    skew_x,
                    stroke_width,
                    opacity: opacity as f32,
                });
                if distinct_stroke && let Some(color) = stroke_color {
                    stroke_colors.push(color);
                }
                if let Some(offset) = dash_offset {
                    dash_offsets.push(offset);
                }
                return Ok(());
            }
            let number = |property: &str| -> Result<f64, LayoutError> {
                match resolved_style(&group.template, values, at, property)? {
                    Some(MotionValue::Number(value)) if value.is_finite() => Ok(*value),
                    Some(MotionValue::Length(value))
                        if value.unit == valle_motion::value::LengthUnit::Px
                            && value.value.is_finite() =>
                    {
                        Ok(value.value)
                    }
                    _ => Err(invalid(property)),
                }
            };
            let left = number("left")?;
            let top = number("top")?;
            let width = number("width")?;
            let height = number("height")?;
            if width < 0.0 || height < 0.0 {
                return Err(invalid("width/height"));
            }
            let fill = match resolved_style(&group.template, values, at, "background-color")? {
                Some(MotionValue::Color(color)) => *color,
                Some(MotionValue::Str(color)) => valle_draw::program::AuthorColor::from_srgb8(
                    valle_draw::Rgba::parse(color).ok_or_else(|| invalid("background-color"))?,
                ),
                _ => return Err(invalid("background-color")),
            };
            let rotation = match resolved_style(&group.template, values, at, "transform")? {
                Some(MotionValue::Str(value)) => value
                    .trim()
                    .strip_prefix("rotate(")
                    .and_then(|value| value.strip_suffix("deg)"))
                    .and_then(|value| value.trim().parse::<f64>().ok())
                    .filter(|value| value.is_finite())
                    .ok_or_else(|| invalid("transform"))?,
                None => 0.0,
                _ => return Err(invalid("transform")),
            };
            // Match ordinary layout's f32 coordinates without rounding positions or edges.
            // Otherwise native batches would still jitter after the full layout path is smooth.
            let (x, y) = (left as f32, top as f32);
            let (width, height) = (width as f32, height as f32);
            if ![x, y, width, height].into_iter().all(f32::is_finite) {
                return Err(invalid("left/top/width/height"));
            }
            if width <= 0.0 || height <= 0.0 {
                return Ok(());
            }
            semantic_keys.push(
                group
                    .key_for_node(at, &group.template.key)
                    .unwrap_or_default(),
            );
            instances.push(valle_draw::program::recording::BatchInstance {
                position: valle_draw::Point::new(f64::from(x), f64::from(y)),
                size: valle_draw::Point::new(f64::from(width), f64::from(height)),
                color: fill.to_working(),
                rotation: rotation as f32,
                skew_x: 0.0,
                stroke_width: 0.0,
                opacity: opacity as f32,
            });
            Ok(())
        },
    )?;
    Ok(crate::layout::bridge::BatchContent {
        row_identity: Vec::new(),
        semantic_keys,
        geometry,
        path,
        atlas: None,
        instances,
        stroke_colors,
        dash_offsets,
        path_style,
        exact_circle_paths: circle.is_some(),
    })
}

fn bad_advanced_filter(node: &valle_motion::SceneNode, reason: impl Into<String>) -> LayoutError {
    LayoutError::BadStyle {
        node: node.key.clone(),
        declarations: "Motion advanced filter".into(),
        reason: reason.into(),
    }
}

fn resolve_layer_fx(
    node: &valle_motion::SceneNode,
    values: &[MotionValue],
    at: usize,
) -> Result<Option<crate::layout::bridge::LayerFx>, LayoutError> {
    let mut fx = crate::layout::bridge::LayerFx::default();
    for style in &node.styles {
        let value = match &style.value {
            StyleValue::Static { value } => value,
            StyleValue::Expr { expr } => value(values, *expr, at)?,
        };
        match style.property.as_str() {
            "rotate-x" => {
                fx.rotate_x_deg = layer_degrees(value, &node.key, "rotateX")?;
            }
            "rotate-y" => {
                fx.rotate_y_deg = layer_degrees(value, &node.key, "rotateY")?;
            }
            "perspective" => {
                fx.perspective = Some(layer_perspective(value, &node.key)?);
            }
            "paper-grain" => {
                fx.paper_grain = layer_scalar(value, &node.key, "paperGrain")?.clamp(0.0, 1.0);
            }
            "contact-shadow" => {
                fx.contact_shadow =
                    layer_scalar(value, &node.key, "contactShadow")?.clamp(0.0, 1.0);
            }
            _ => {}
        }
    }
    Ok((!fx.is_empty()).then_some(fx))
}

fn layer_degrees(value: &MotionValue, node: &str, property: &str) -> Result<f64, LayoutError> {
    match value {
        MotionValue::Number(value) if value.is_finite() => Ok(*value),
        MotionValue::Angle(angle) if angle.value.is_finite() => Ok(angle.as_degrees()),
        _ => Err(LayoutError::BadStyle {
            node: node.into(),
            declarations: property.into(),
            reason: format!("{property} must be a finite number of degrees or an angle"),
        }),
    }
}

fn layer_scalar(value: &MotionValue, node: &str, property: &str) -> Result<f64, LayoutError> {
    match value {
        MotionValue::Number(value) if value.is_finite() => Ok(*value),
        _ => Err(LayoutError::BadStyle {
            node: node.into(),
            declarations: property.into(),
            reason: format!("{property} must be a finite number"),
        }),
    }
}

fn layer_length(value: &MotionValue, node: &str, property: &str) -> Result<Length, LayoutError> {
    match value {
        MotionValue::Length(value) if value.value.is_finite() => Ok(*value),
        _ => Err(LayoutError::BadStyle {
            node: node.into(),
            declarations: property.into(),
            reason: format!("{property} must be a finite CSS length or percentage"),
        }),
    }
}

fn resolve_css_position(
    value: Length,
    start: f64,
    size: f64,
    sizing: &SizingContext,
    node: &str,
    property: &str,
) -> Result<f64, LayoutError> {
    let bad = |reason: &str| LayoutError::BadStyle {
        node: node.into(),
        declarations: property.into(),
        reason: reason.into(),
    };
    let dpr = if sizing.viewport.device_pixel_ratio > 0.0 {
        f64::from(sizing.viewport.device_pixel_ratio)
    } else {
        1.0
    };
    let offset = match value.unit {
        LengthUnit::Percent => size * value.value / 100.0,
        LengthUnit::Px => value.value * dpr,
        LengthUnit::Rem => {
            let rem = f64::from(crate::ROOT_FONT_SIZE) * dpr;
            value.value * rem
        }
        LengthUnit::Em => value.value * f64::from(sizing.font_size),
        LengthUnit::Vw => {
            let width = sizing
                .viewport
                .size
                .width
                .ok_or_else(|| bad("vw requires a viewport width"))?;
            value.value * f64::from(width) / 100.0
        }
        LengthUnit::Vh => {
            let height = sizing
                .viewport
                .size
                .height
                .ok_or_else(|| bad("vh requires a viewport height"))?;
            value.value * f64::from(height) / 100.0
        }
    };
    let resolved = start + offset;
    if resolved.is_finite() {
        Ok(resolved)
    } else {
        Err(bad("position must resolve to a finite coordinate"))
    }
}

fn layer_perspective(
    value: &MotionValue,
    node: &str,
) -> Result<crate::layout::bridge::PerspectiveLength, LayoutError> {
    use crate::layout::bridge::PerspectiveLength;
    let bad = |reason: &str| LayoutError::BadStyle {
        node: node.into(),
        declarations: "perspective".into(),
        reason: reason.into(),
    };
    match value {
        MotionValue::Number(value) if value.is_finite() && *value > 0.0 => {
            Ok(PerspectiveLength::Px(*value))
        }
        MotionValue::Number(value) if value.is_finite() => {
            Err(bad("perspective must be a positive length"))
        }
        MotionValue::Length(Length { value, unit }) if value.is_finite() && *value > 0.0 => {
            match unit {
                LengthUnit::Px => Ok(PerspectiveLength::Px(*value)),
                LengthUnit::Rem => Ok(PerspectiveLength::Rem(*value)),
                LengthUnit::Em => Ok(PerspectiveLength::Em(*value)),
                LengthUnit::Vw => Ok(PerspectiveLength::Vw(*value)),
                LengthUnit::Vh => Ok(PerspectiveLength::Vh(*value)),
                LengthUnit::Percent => Err(bad(
                    "perspective does not accept percentages; use px, rem, em, vw, or vh",
                )),
            }
        }
        MotionValue::Length(Length { value, .. }) if value.is_finite() => {
            Err(bad("perspective must be a positive length"))
        }
        _ => Err(bad(
            "perspective must be a finite positive number or length (px, rem, em, vw, vh)",
        )),
    }
}

#[derive(Debug, Clone, Copy)]
enum Css3dOp {
    TranslateX(f64),
    TranslateY(f64),
    TranslateZ(f64),
    TranslateXPercent(f64),
    TranslateYPercent(f64),
    RotateX(f64),
    RotateY(f64),
    RotateZ(f64),
    RotateAxis {
        x: f64,
        y: f64,
        z: f64,
        degrees: f64,
    },
    ScaleX(f64),
    ScaleY(f64),
    ScaleZ(f64),
    /// `skew(x, y)` in degrees.
    Skew {
        x: f64,
        y: f64,
    },
    /// `matrix(a, b, c, d, e, f)`.
    Matrix([f64; 6]),
    /// `matrix3d(…)` in CSS's column-major order.
    Matrix3d([f64; 16]),
    /// `perspective(d)` in px.
    Perspective(f64),
}

#[derive(Debug, Default)]
struct Css3dStyle {
    ops: Vec<Css3dOp>,
    preserve_3d: bool,
    backface_hidden: bool,
    perspective: Option<crate::layout::bridge::PerspectiveLength>,
    perspective_origin_x: Option<Length>,
    perspective_origin_y: Option<Length>,
}

#[derive(Debug, Clone, Copy)]
struct Matrix4([f64; 16]);

impl Matrix4 {
    const IDENTITY: Self = Self([
        1.0, 0.0, 0.0, 0.0, //
        0.0, 1.0, 0.0, 0.0, //
        0.0, 0.0, 1.0, 0.0, //
        0.0, 0.0, 0.0, 1.0,
    ]);

    fn multiply(self, rhs: Self) -> Self {
        let mut out = [0.0; 16];
        for row in 0..4 {
            for col in 0..4 {
                out[row * 4 + col] = (0..4)
                    .map(|k| self.0[row * 4 + k] * rhs.0[k * 4 + col])
                    .sum();
            }
        }
        Self(out)
    }

    fn translate(x: f64, y: f64, z: f64) -> Self {
        Self([
            1.0, 0.0, 0.0, x, //
            0.0, 1.0, 0.0, y, //
            0.0, 0.0, 1.0, z, //
            0.0, 0.0, 0.0, 1.0,
        ])
    }

    fn scale(x: f64, y: f64, z: f64) -> Self {
        Self([
            x, 0.0, 0.0, 0.0, //
            0.0, y, 0.0, 0.0, //
            0.0, 0.0, z, 0.0, //
            0.0, 0.0, 0.0, 1.0,
        ])
    }

    fn rotate_x(degrees: f64) -> Self {
        let (s, c) = valle_draw::math::sin_cos(degrees.to_radians());
        Self([
            1.0, 0.0, 0.0, 0.0, //
            0.0, c, -s, 0.0, //
            0.0, s, c, 0.0, //
            0.0, 0.0, 0.0, 1.0,
        ])
    }

    fn rotate_y(degrees: f64) -> Self {
        let (s, c) = valle_draw::math::sin_cos(degrees.to_radians());
        Self([
            c, 0.0, s, 0.0, //
            0.0, 1.0, 0.0, 0.0, //
            -s, 0.0, c, 0.0, //
            0.0, 0.0, 0.0, 1.0,
        ])
    }

    fn rotate_z(degrees: f64) -> Self {
        let (s, c) = valle_draw::math::sin_cos(degrees.to_radians());
        Self([
            c, -s, 0.0, 0.0, //
            s, c, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0, //
            0.0, 0.0, 0.0, 1.0,
        ])
    }

    fn rotate_axis(x: f64, y: f64, z: f64, degrees: f64) -> Self {
        // Scale first so a large but finite authored axis cannot overflow while normalizing.
        let max = x.abs().max(y.abs()).max(z.abs());
        let (x, y, z) = (x / max, y / max, z / max);
        let length = valle_draw::math::sqrt(x * x + y * y + z * z);
        let (x, y, z) = (x / length, y / length, z / length);
        let (s, c) = valle_draw::math::sin_cos(degrees.to_radians());
        let t = 1.0 - c;
        Self([
            t * x * x + c,
            t * x * y - s * z,
            t * x * z + s * y,
            0.0,
            t * x * y + s * z,
            t * y * y + c,
            t * y * z - s * x,
            0.0,
            t * x * z - s * y,
            t * y * z + s * x,
            t * z * z + c,
            0.0,
            0.0,
            0.0,
            0.0,
            1.0,
        ])
    }

    fn apply(self, x: f64, y: f64, z: f64) -> Option<[f64; 3]> {
        let m = self.0;
        let tx = m[0] * x + m[1] * y + m[2] * z + m[3];
        let ty = m[4] * x + m[5] * y + m[6] * z + m[7];
        let tz = m[8] * x + m[9] * y + m[10] * z + m[11];
        let w = m[12] * x + m[13] * y + m[14] * z + m[15];
        // Only `perspective()` and `matrix3d()` move w from 1; a point at or behind their camera
        // has none.
        if !tx.is_finite() || !ty.is_finite() || !tz.is_finite() || !w.is_finite() || w <= 0.0 {
            return None;
        }
        Some([tx / w, ty / w, tz / w])
    }
}

fn css_3d_style(
    node: &valle_motion::SceneNode,
    values: &[MotionValue],
    at: usize,
) -> Result<Css3dStyle, LayoutError> {
    let mut out = Css3dStyle::default();
    let mut rotate_axis = [None, None, None];
    let mut skew_x = None;
    let mut matrix = Vec::new();
    let mut matrix3d = Vec::new();
    let out_of_order = |function: &str| LayoutError::BadStyle {
        node: node.key.clone(),
        declarations: function.into(),
        reason: format!("{function} components must be complete and in order"),
    };
    for binding in &node.styles {
        let value = match &binding.value {
            StyleValue::Static { value } => value,
            StyleValue::Expr { expr } => value(values, *expr, at)?,
        };
        let scalar = |property: &str| layer_scalar(value, &node.key, property);
        if let Some((function, index)) = binding
            .property
            .strip_prefix("motion-transform-3d-")
            .and_then(|suffix| suffix.rsplit_once('-'))
            .filter(|(function, _)| matches!(*function, "matrix" | "matrix3d"))
        {
            let (parts, count) = if function == "matrix" {
                (&mut matrix, 6)
            } else {
                (&mut matrix3d, 16)
            };
            if index.parse::<usize>().ok() != Some(parts.len()) {
                return Err(out_of_order(function));
            }
            parts.push(scalar(function)?);
            if parts.len() == count {
                out.ops.push(if count == 6 {
                    Css3dOp::Matrix(parts[..].try_into().expect("six components"))
                } else {
                    Css3dOp::Matrix3d(parts[..].try_into().expect("sixteen components"))
                });
                parts.clear();
            }
            continue;
        }
        match binding.property.as_str() {
            "motion-transform-3d-translate-x" => {
                out.ops.push(Css3dOp::TranslateX(scalar("translateX")?))
            }
            "motion-transform-3d-translate-y" => {
                out.ops.push(Css3dOp::TranslateY(scalar("translateY")?))
            }
            "motion-transform-3d-translate-z" => {
                out.ops.push(Css3dOp::TranslateZ(scalar("translateZ")?))
            }
            "motion-transform-3d-translate-x-percent" => out
                .ops
                .push(Css3dOp::TranslateXPercent(scalar("translateX")?)),
            "motion-transform-3d-translate-y-percent" => out
                .ops
                .push(Css3dOp::TranslateYPercent(scalar("translateY")?)),
            "motion-transform-3d-rotate-x" => out.ops.push(Css3dOp::RotateX(scalar("rotateX")?)),
            "motion-transform-3d-rotate-y" => out.ops.push(Css3dOp::RotateY(scalar("rotateY")?)),
            "motion-transform-3d-rotate-z" => out.ops.push(Css3dOp::RotateZ(scalar("rotateZ")?)),
            "rotate-x" => out.ops.push(Css3dOp::RotateX(scalar("rotateX")?)),
            "rotate-y" => out.ops.push(Css3dOp::RotateY(scalar("rotateY")?)),
            "motion-transform-3d-rotate-axis-x" => rotate_axis[0] = Some(scalar("rotate3d.x")?),
            "motion-transform-3d-rotate-axis-y" => rotate_axis[1] = Some(scalar("rotate3d.y")?),
            "motion-transform-3d-rotate-axis-z" => rotate_axis[2] = Some(scalar("rotate3d.z")?),
            "motion-transform-3d-rotate-axis-angle" => {
                let [Some(x), Some(y), Some(z)] = rotate_axis else {
                    return Err(LayoutError::BadStyle {
                        node: node.key.clone(),
                        declarations: "rotate3d".into(),
                        reason: "rotate3d angle must follow its three axis components".into(),
                    });
                };
                if x == 0.0 && y == 0.0 && z == 0.0 {
                    return Err(LayoutError::BadStyle {
                        node: node.key.clone(),
                        declarations: "rotate3d".into(),
                        reason: "rotate3d axis must not be the zero vector".into(),
                    });
                }
                out.ops.push(Css3dOp::RotateAxis {
                    x,
                    y,
                    z,
                    degrees: scalar("rotate3d.angle")?,
                });
                rotate_axis = [None, None, None];
            }
            "motion-transform-3d-scale-x" => out.ops.push(Css3dOp::ScaleX(scalar("scaleX")?)),
            "motion-transform-3d-scale-y" => out.ops.push(Css3dOp::ScaleY(scalar("scaleY")?)),
            "motion-transform-3d-scale-z" => out.ops.push(Css3dOp::ScaleZ(scalar("scaleZ")?)),
            "motion-transform-3d-skew-x" => skew_x = Some(scalar("skew")?),
            "motion-transform-3d-skew-y" => {
                let x = skew_x.take().ok_or_else(|| out_of_order("skew"))?;
                out.ops.push(Css3dOp::Skew {
                    x,
                    y: scalar("skew")?,
                });
            }
            "motion-transform-3d-perspective" => {
                let depth = scalar("perspective()")?;
                if depth < 0.0 {
                    return Err(LayoutError::BadStyle {
                        node: node.key.clone(),
                        declarations: "perspective()".into(),
                        reason: "perspective() depth must not be negative".into(),
                    });
                }
                out.ops.push(Css3dOp::Perspective(depth));
            }
            "transform-style" => {
                out.preserve_3d =
                    css_3d_keyword(node, value, "transformStyle", ["flat", "preserve-3d"])?
            }
            "backface-visibility" => {
                out.backface_hidden =
                    css_3d_keyword(node, value, "backfaceVisibility", ["visible", "hidden"])?
            }
            "perspective" => out.perspective = Some(layer_perspective(value, &node.key)?),
            "motion-perspective-origin-x" => {
                out.perspective_origin_x =
                    Some(layer_length(value, &node.key, "perspectiveOrigin.x")?)
            }
            "motion-perspective-origin-y" => {
                out.perspective_origin_y =
                    Some(layer_length(value, &node.key, "perspectiveOrigin.y")?)
            }
            "motion-perspective-origin-x-px" => {
                out.perspective_origin_x = Some(Length::px(scalar("perspectiveOrigin.x")?))
            }
            "motion-perspective-origin-y-px" => {
                out.perspective_origin_y = Some(Length::px(scalar("perspectiveOrigin.y")?))
            }
            "motion-perspective-origin-x-percent" => {
                out.perspective_origin_x = Some(Length {
                    value: scalar("perspectiveOrigin.x")?,
                    unit: LengthUnit::Percent,
                })
            }
            "motion-perspective-origin-y-percent" => {
                out.perspective_origin_y = Some(Length {
                    value: scalar("perspectiveOrigin.y")?,
                    unit: LengthUnit::Percent,
                })
            }
            _ => {}
        }
    }
    if skew_x.is_some() {
        return Err(out_of_order("skew"));
    }
    if !matrix.is_empty() {
        return Err(out_of_order("matrix"));
    }
    if !matrix3d.is_empty() {
        return Err(out_of_order("matrix3d"));
    }
    Ok(out)
}

/// Whether a two-keyword 3D style holds its second keyword. A value chosen per frame is checked
/// here, so a wrong one fails the frame that draws it.
fn css_3d_keyword(
    node: &valle_motion::SceneNode,
    value: &MotionValue,
    property: &str,
    [initial, other]: [&str; 2],
) -> Result<bool, LayoutError> {
    match value {
        MotionValue::Str(text) | MotionValue::Enum(text) if text == initial => Ok(false),
        MotionValue::Str(text) | MotionValue::Enum(text) if text == other => Ok(true),
        value => Err(LayoutError::BadStyle {
            node: node.key.clone(),
            declarations: property.into(),
            reason: match value {
                MotionValue::Str(text) | MotionValue::Enum(text) => {
                    format!("{property} takes `{initial}` or `{other}`, not `{text}`")
                }
                _ => format!("{property} takes `{initial}` or `{other}`"),
            },
        }),
    }
}

fn css_3d_local_matrix(
    style: &Css3dStyle,
    origin: valle_draw::Point,
    rect: valle_draw::Rect,
) -> Matrix4 {
    let mut transform = Matrix4::IDENTITY;
    for op in &style.ops {
        let next = match *op {
            Css3dOp::TranslateX(value) => Matrix4::translate(value, 0.0, 0.0),
            Css3dOp::TranslateY(value) => Matrix4::translate(0.0, value, 0.0),
            Css3dOp::TranslateZ(value) => Matrix4::translate(0.0, 0.0, value),
            Css3dOp::TranslateXPercent(value) => {
                Matrix4::translate(rect.width * value / 100.0, 0.0, 0.0)
            }
            Css3dOp::TranslateYPercent(value) => {
                Matrix4::translate(0.0, rect.height * value / 100.0, 0.0)
            }
            Css3dOp::RotateX(value) => Matrix4::rotate_x(value),
            Css3dOp::RotateY(value) => Matrix4::rotate_y(value),
            Css3dOp::RotateZ(value) => Matrix4::rotate_z(value),
            Css3dOp::RotateAxis { x, y, z, degrees } => Matrix4::rotate_axis(x, y, z, degrees),
            Css3dOp::ScaleX(value) => Matrix4::scale(value, 1.0, 1.0),
            Css3dOp::ScaleY(value) => Matrix4::scale(1.0, value, 1.0),
            Css3dOp::ScaleZ(value) => Matrix4::scale(1.0, 1.0, value),
            Css3dOp::Skew { x, y } => {
                let (x, y) = (
                    valle_draw::math::tan(x.to_radians()),
                    valle_draw::math::tan(y.to_radians()),
                );
                Matrix4([
                    1.0, x, 0.0, 0.0, //
                    y, 1.0, 0.0, 0.0, //
                    0.0, 0.0, 1.0, 0.0, //
                    0.0, 0.0, 0.0, 1.0,
                ])
            }
            Css3dOp::Matrix([a, b, c, d, e, f]) => Matrix4([
                a, c, 0.0, e, //
                b, d, 0.0, f, //
                0.0, 0.0, 1.0, 0.0, //
                0.0, 0.0, 0.0, 1.0,
            ]),
            Css3dOp::Matrix3d(m) => Matrix4([
                m[0], m[4], m[8], m[12], //
                m[1], m[5], m[9], m[13], //
                m[2], m[6], m[10], m[14], //
                m[3], m[7], m[11], m[15],
            ]),
            Css3dOp::Perspective(depth) => {
                // CSS treats a depth below 1px as 1px.
                let w = -1.0 / depth.max(1.0);
                Matrix4([
                    1.0, 0.0, 0.0, 0.0, //
                    0.0, 1.0, 0.0, 0.0, //
                    0.0, 0.0, 1.0, 0.0, //
                    0.0, 0.0, w, 1.0,
                ])
            }
        };
        // CSS transform functions compose in authored order; with column vectors the
        // right-most function acts first, so the authored list multiplies left-to-right.
        transform = transform.multiply(next);
    }
    Matrix4::translate(origin.x, origin.y, 0.0)
        .multiply(transform)
        .multiply(Matrix4::translate(-origin.x, -origin.y, 0.0))
}

#[derive(Clone, Copy)]
struct Css3dPerspective {
    distance: f64,
    origin: valle_draw::Point,
}

#[derive(Clone, Copy)]
struct Css3dState {
    matrix: Matrix4,
    perspective: Option<Css3dPerspective>,
    preserve_3d: bool,
}

/// What CSS 3D reads of one node from the frame's final tree, in absolute layout coordinates.
#[derive(Clone, Copy)]
struct Css3dNode {
    origin: valle_draw::Point,
    /// The 2D transform the tree applies to the node: translate, rotate, scale and a 2D
    /// `transform` list, around its origin.
    local: Matrix4,
    /// From absolute layout coordinates to the output, through the transforms of every ancestor,
    /// a camera's included: where a flat parent draws the node.
    parent_frame: TAffine,
}

/// Read each node's origin and transforms from the same Takumi tree used for final paint. The
/// origin is resolved against this node's own font sizing and layout box, rather than a global
/// guess.
fn css_3d_nodes(
    root: &RenderNode,
    layout: &LayoutResults,
    keys: &HashMap<u64, String>,
) -> Result<HashMap<String, Css3dNode>, LayoutError> {
    let mut out = HashMap::new();
    let mut pending = vec![(
        root,
        takumi_core::geometry::NodeId::ROOT,
        TAffine::IDENTITY,
        (0.0_f32, 0.0_f32),
    )];
    while let Some((node, id, parent, parent_origin)) = pending.pop() {
        let Ok(box_layout) = layout.layout(id) else {
            continue;
        };
        let (width, height) = (box_layout.size.width, box_layout.size.height);
        let at = (
            parent_origin.0 + box_layout.location.x,
            parent_origin.1 + box_layout.location.y,
        );
        let local = node
            .context
            .style
            .local_transform(width, height, &node.context.sizing);
        let matrix =
            parent * TAffine::translation(box_layout.location.x, box_layout.location.y) * local;
        if let Some(key) = keys.get(&u64::from(id)) {
            let origin = node.context.style.transform_origin.0;
            let x = f64::from(
                takumi_core::style::Length::from(origin.x).to_px(&node.context.sizing, width),
            ) + f64::from(at.0);
            let y = f64::from(
                takumi_core::style::Length::from(origin.y).to_px(&node.context.sizing, height),
            ) + f64::from(at.1);
            if !x.is_finite() || !y.is_finite() {
                return Err(LayoutError::BadStyle {
                    node: key.clone(),
                    declarations: "transform-origin".into(),
                    reason: "origin must resolve to finite coordinates".into(),
                });
            }
            let [a, b, c, d, e, f] = local.to_cols_array().map(f64::from);
            out.insert(
                key.clone(),
                Css3dNode {
                    origin: valle_draw::Point::new(x, y),
                    local: Matrix4::translate(f64::from(at.0), f64::from(at.1), 0.0)
                        .multiply(Matrix4([
                            a, c, 0.0, e, //
                            b, d, 0.0, f, //
                            0.0, 0.0, 1.0, 0.0, //
                            0.0, 0.0, 0.0, 1.0,
                        ]))
                        .multiply(Matrix4::translate(-f64::from(at.0), -f64::from(at.1), 0.0)),
                    parent_frame: parent * TAffine::translation(-parent_origin.0, -parent_origin.1),
                },
            );
        }
        if let (Some(children), Ok(boxes)) = (&node.children, layout.box_children(id)) {
            for child in boxes {
                if let Some(node) = children.get(child.render_index) {
                    pending.push((node, child.node_id, matrix, at));
                }
            }
        }
    }
    Ok(out)
}

/// The 3D space a camera moving in depth gives World content: perspective toward the camera's
/// center and the inverse of its orbit and depth. The camera's center, zoom and rotation stay
/// with its wrappers, which draw this space as they draw a flat World.
fn camera_space(
    artifact: &SceneArtifact,
    values: &[MotionValue],
) -> Result<Option<Css3dState>, LayoutError> {
    let Some(camera) = artifact.camera.as_ref().filter(|camera| camera.is_3d()) else {
        return Ok(None);
    };
    let at = artifact.root.0 as usize;
    let center = point_value(&camera.center, values, at, "camera center")?;
    let number = |value: &Option<NumberValue>, name: &'static str| {
        value
            .as_ref()
            .map(|value| number_value(value, values, at, name))
            .transpose()
    };
    let perspective = number(&camera.perspective, "camera perspective")?;
    let [z, rotate_x, rotate_y] = [
        number(&camera.z, "camera z")?,
        number(&camera.rotate_x, "camera rotateX")?,
        number(&camera.rotate_y, "camera rotateY")?,
    ]
    .map(|value| value.unwrap_or(0.0));
    if perspective.is_some_and(|distance| !distance.is_finite() || distance <= 0.0) {
        return Err(LayoutError::BadCamera {
            reason: format!(
                "perspective must be finite and positive, got {}",
                perspective.unwrap_or_default()
            ),
        });
    }
    if ![z, rotate_x, rotate_y]
        .iter()
        .all(|value| value.is_finite())
    {
        return Err(LayoutError::BadCamera {
            reason: "z, rotateX and rotateY must be finite".into(),
        });
    }
    Ok(Some(Css3dState {
        matrix: Matrix4::translate(center.x, center.y, 0.0)
            .multiply(Matrix4::rotate_x(-rotate_x))
            .multiply(Matrix4::rotate_y(-rotate_y))
            .multiply(Matrix4::translate(-center.x, -center.y, -z)),
        perspective: perspective.map(|distance| Css3dPerspective {
            distance,
            origin: center,
        }),
        preserve_3d: true,
    }))
}

/// Auto depth is a derived computed default. Applying it after the author cascade means even an
/// important `z-index: auto` receives the fallback, while every explicit integer wins. A plane
/// that faces away under `backfaceVisibility: hidden`, or edge-on, paints nothing.
fn apply_css_3d_planes(
    artifact: &SceneArtifact,
    root: &mut RenderNode,
    planes: &HashMap<String, crate::layout::bridge::Css3dPlane>,
    inactive_nodes: &[bool],
) {
    let mut keys = HashMap::new();
    walk_render_keys(
        artifact,
        artifact.root,
        root,
        &mut Vec::new(),
        &mut keys,
        inactive_nodes,
    );
    for (path, key) in keys {
        let Some(plane) = planes.get(&key) else {
            continue;
        };
        let Some(node) = root
            .node_at_path_mut(&path)
            .filter(|node| node.source_order().is_some())
        else {
            continue;
        };
        if node.context.style.z_index == ZIndex::Auto {
            let depth = (plane.depth * 1024.0)
                .round()
                .clamp(f64::from(i32::MIN + 1), f64::from(i32::MAX - 1))
                as i32;
            node.context.style.z_index = ZIndex::Integer(depth);
        }
        if plane.hidden {
            node.context.style.visibility = takumi_core::style::Visibility::Hidden;
        }
    }
}

impl Default for Css3dState {
    fn default() -> Self {
        Self {
            matrix: Matrix4::IDENTITY,
            perspective: None,
            preserve_3d: false,
        }
    }
}

fn resolve_css_3d_planes(
    artifact: &SceneArtifact,
    values: &[MotionValue],
    boxes: &BTreeMap<String, valle_draw::Rect>,
    opts: &LayoutOptions<'_>,
    nodes: &HashMap<String, Css3dNode>,
    camera_space: Option<Css3dState>,
) -> Result<HashMap<String, crate::layout::bridge::Css3dPlane>, LayoutError> {
    let sizing = SizingContext::builder().viewport(opts.viewport).build();
    let mut out = HashMap::new();
    resolve_css_3d_node(
        &Css3dScene {
            artifact,
            values,
            boxes,
            sizing: &sizing,
            nodes,
            camera_space,
        },
        artifact.root.0 as usize,
        Css3dState::default(),
        None,
        &mut out,
    )?;
    Ok(out)
}

/// The inputs every node of one frame's CSS 3D resolution shares.
struct Css3dScene<'a> {
    artifact: &'a SceneArtifact,
    values: &'a [MotionValue],
    boxes: &'a BTreeMap<String, valle_draw::Rect>,
    sizing: &'a SizingContext,
    nodes: &'a HashMap<String, Css3dNode>,
    camera_space: Option<Css3dState>,
}

/// Resolve one node and its descendants. `frame` is where the enclosing 3D rendering context
/// draws, from its flat root; outside one, each node's own parent draws it.
fn resolve_css_3d_node(
    scene: &Css3dScene<'_>,
    at: usize,
    state: Css3dState,
    frame: Option<TAffine>,
    out: &mut HashMap<String, crate::layout::bridge::Css3dPlane>,
) -> Result<(), LayoutError> {
    let artifact = scene.artifact;
    let node = artifact.nodes.get(at).ok_or(LayoutError::BadNode { at })?;
    let range = node.children.start as usize..node.children.end as usize;
    let child_ids = artifact
        .node_children
        .get(range)
        .ok_or(LayoutError::BadNode { at })?;
    let (Some(rect), Some(own)) = (
        scene.boxes.get(&node.key).copied(),
        scene.nodes.get(&node.key),
    ) else {
        if state.preserve_3d && matches!(node.kind, NodeKind::InstanceLayout { .. }) {
            return Err(LayoutError::BadStyle {
                node: node.key.clone(),
                declarations: "transform-style".into(),
                reason: "the rows a `.map` lays out from one shared template cannot be planes \
                         of a 3D space; put the list in a View, which they are then drawn on"
                    .into(),
            });
        }
        // GlassField and TimeScope lay their children out as their parent's own. Any other node
        // the frame does not lay out projects nothing, and neither do its descendants.
        let transparent = matches!(
            node.kind,
            NodeKind::GlassField(_) | NodeKind::TimeScope { .. }
        );
        for child in child_ids {
            if transparent {
                resolve_css_3d_node(scene, child.0 as usize, state, frame, out)?;
            } else {
                resolve_css_3d_node(scene, child.0 as usize, Css3dState::default(), None, out)?;
            }
        }
        return Ok(());
    };
    let style = css_3d_style(node, scene.values, at)?;
    // World content shares the space of a camera that moves in depth.
    let preserve_3d = style.preserve_3d
        || (state.preserve_3d
            && scene.camera_space.is_some()
            && node.space == Some(CoordinateSpace::World));
    let origin = own.origin;
    let local = own
        .local
        .multiply(css_3d_local_matrix(&style, origin, rect));
    let matrix = if state.preserve_3d {
        state.matrix.multiply(local)
    } else {
        local
    };
    let perspective = if let Some(value) = style.perspective {
        let perspective_origin = valle_draw::Point::new(
            resolve_css_position(
                style.perspective_origin_x.unwrap_or(Length {
                    value: 50.0,
                    unit: LengthUnit::Percent,
                }),
                rect.x,
                rect.width,
                scene.sizing,
                &node.key,
                "perspectiveOrigin.x",
            )?,
            resolve_css_position(
                style.perspective_origin_y.unwrap_or(Length {
                    value: 50.0,
                    unit: LengthUnit::Percent,
                }),
                rect.y,
                rect.height,
                scene.sizing,
                &node.key,
                "perspectiveOrigin.y",
            )?,
        );
        Some(Css3dPerspective {
            distance: value
                .to_px(scene.sizing)
                .map_err(|reason| LayoutError::BadStyle {
                    node: node.key.clone(),
                    declarations: "perspective".into(),
                    reason,
                })?,
            origin: perspective_origin,
        })
    } else {
        state.perspective
    };
    // The flat root of a 3D rendering context draws all of its planes.
    let frame = frame.unwrap_or(own.parent_frame);

    let is_plane =
        !preserve_3d && (state.preserve_3d || !style.ops.is_empty() || style.backface_hidden);
    if is_plane {
        let source = [
            [rect.x, rect.y],
            [rect.x + rect.width, rect.y],
            [rect.x + rect.width, rect.y + rect.height],
            [rect.x, rect.y + rect.height],
        ];
        let transformed = source
            .map(|[x, y]| matrix.apply(x, y, 0.0))
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| LayoutError::BadStyle {
                node: node.key.clone(),
                declarations: "transform".into(),
                reason: "CSS 3D transform produced a non-finite plane, or one at or behind a \
                         perspective() camera"
                    .into(),
            })?;
        let mut quad = [valle_draw::Point::default(); 4];
        for (index, point) in transformed.iter().enumerate() {
            quad[index] = if let Some(perspective) = perspective {
                let denominator = perspective.distance - point[2];
                if !denominator.is_finite() || denominator <= 1e-6 {
                    return Err(LayoutError::BadStyle {
                        node: node.key.clone(),
                        declarations: "perspective".into(),
                        reason: "CSS 3D plane reached or crossed the perspective camera".into(),
                    });
                }
                let scale = perspective.distance / denominator;
                valle_draw::Point::new(
                    perspective.origin.x + (point[0] - perspective.origin.x) * scale,
                    perspective.origin.y + (point[1] - perspective.origin.y) * scale,
                )
            } else {
                valle_draw::Point::new(point[0], point[1])
            };
        }
        let projection = plane_projection(matrix, perspective);
        // Facing is the context's own: transforms that draw the context do not turn its planes.
        let facing = if rect.width > 0.0 && rect.height > 0.0 {
            let signed_area = (quad[1].x - quad[0].x) * (quad[2].y - quad[0].y)
                - (quad[1].y - quad[0].y) * (quad[2].x - quad[0].x);
            (signed_area.abs() > 1e-8).then_some(signed_area)
        } else {
            // An empty box still draws its overflowing descendants: face by the plane's own
            // local scale there.
            let [a, b, c, d, e, f, g, h, i] = projection.0;
            let w = g * rect.x + h * rect.y + i;
            let jacobian =
                (a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g)) / (w * w * w);
            (jacobian.is_finite() && jacobian.abs() > 1e-12).then_some(jacobian)
        };
        let center =
            matrix
                .apply(origin.x, origin.y, 0.0)
                .ok_or_else(|| LayoutError::BadStyle {
                    node: node.key.clone(),
                    declarations: "transform".into(),
                    reason: "CSS 3D transform produced a non-finite center".into(),
                })?;
        let [fa, fb, fc, fd, fe, ff] = frame.to_cols_array().map(f64::from);
        let frame_matrix = Homography([fa, fc, fe, fb, fd, ff, 0.0, 0.0, 1.0]);
        out.insert(
            node.key.clone(),
            crate::layout::bridge::Css3dPlane {
                quad: quad.map(|point| frame_matrix.apply(point)),
                matrix: normalized(
                    Homography([1.0, 0.0, rect.x, 0.0, 1.0, rect.y, 0.0, 0.0, 1.0])
                        .then(projection)
                        .then(frame_matrix),
                ),
                depth: center[2],
                hidden: facing.is_none_or(|facing| style.backface_hidden && facing < 0.0),
                project: true,
            },
        );
    }

    // Positioned preserve-3d children are siblings in their parent's 3D painter order. Their
    // descendants own the actual projected planes, so this record contributes only z-index.
    if preserve_3d
        && state.preserve_3d
        && rect.width > 0.0
        && rect.height > 0.0
        && let Some(center) = matrix.apply(origin.x, origin.y, 0.0)
    {
        out.insert(
            node.key.clone(),
            crate::layout::bridge::Css3dPlane {
                quad: [valle_draw::Point::default(); 4],
                matrix: Homography::IDENTITY,
                depth: center[2],
                hidden: false,
                project: false,
            },
        );
    }

    let (child_state, child_frame) = if preserve_3d {
        (
            Css3dState {
                matrix,
                perspective,
                preserve_3d: true,
            },
            Some(frame),
        )
    } else if style.perspective.is_some() {
        (
            Css3dState {
                matrix: Matrix4::IDENTITY,
                perspective,
                preserve_3d: false,
            },
            None,
        )
    } else {
        (Css3dState::default(), None)
    };
    for child in child_ids {
        // World content enters the space of a camera that moves in depth.
        let world = at == artifact.root.0 as usize
            && artifact
                .nodes
                .get(child.0 as usize)
                .is_some_and(|child| child.space != Some(CoordinateSpace::Screen));
        match scene.camera_space.filter(|_| world) {
            Some(space) => resolve_css_3d_node(scene, child.0 as usize, space, None, out)?,
            None => resolve_css_3d_node(scene, child.0 as usize, child_state, child_frame, out)?,
        }
    }
    Ok(())
}

/// The projective map of a plane's z = 0 from absolute layout coordinates: its 4×4 transform,
/// then the perspective of the context, if any.
fn plane_projection(matrix: Matrix4, perspective: Option<Css3dPerspective>) -> Homography {
    let m = matrix.0;
    let row = |at: usize| [m[at], m[at + 1], m[at + 3]];
    let [x, y, z, w] = [row(0), row(4), row(8), row(12)];
    let rows = match perspective {
        // Dividing by d − z: x′ = (d·x − oₓ·z) / (d·w − z).
        Some(Css3dPerspective { distance, origin }) => [
            [0, 1, 2].map(|at| distance * x[at] - origin.x * z[at]),
            [0, 1, 2].map(|at| distance * y[at] - origin.y * z[at]),
            [0, 1, 2].map(|at| distance * w[at] - z[at]),
        ],
        None => [x, y, w],
    };
    Homography([
        rows[0][0], rows[0][1], rows[0][2], //
        rows[1][0], rows[1][1], rows[1][2], //
        rows[2][0], rows[2][1], rows[2][2],
    ])
}

/// A homography scaled so a point at the node's origin has unit weight, as one fitted to its
/// corners has.
fn normalized(matrix: Homography) -> Homography {
    let weight = matrix.0[8];
    if weight.is_finite() && weight.abs() > 1e-12 {
        Homography(matrix.0.map(|value| value / weight))
    } else {
        matrix
    }
}

/// The trees of one frame's temporal samples, by what each shows: its scene time, and its output
/// time where the tree also depends on that (see `PreparedScene::samples_read_output_time`).
/// None denotes the admitted request-invariant class; it never crosses a frame request.
type SampleTrees = HashMap<
    Option<(
        valle_timeline::internal::SampleTime,
        Option<valle_timeline::RationalTime>,
    )>,
    Rc<LayoutTree>,
>;

/// The tree of one temporal sample, shared by every sample of the frame that shows the same.
fn sample_tree(
    prepared: &PreparedScene,
    sample: &valle_motion::MotionContext,
    props: &ResolvedProps,
    opts: &LayoutOptions<'_>,
    trees: &mut SampleTrees,
    shared: Option<&mut RequestValueCache>,
) -> Result<TemporalSample, LayoutError> {
    let sample_prepared = prepared.sample_scene.as_deref().unwrap_or(prepared);
    // None is a proof-derived key, not a previous-frame heuristic. This map is local to the
    // current request, so fonts, props, host clock mapping and viewport cannot change under it.
    let key = (!sample_prepared.temporal_tree_invariant).then_some((
        sample.sample,
        prepared.samples_read_output_time.then_some(sample.at),
    ));
    let tree = if let Some(tree) = trees.get(&key) {
        Rc::clone(tree)
    } else {
        let tree = Rc::new(build_tree_inner(
            sample_prepared,
            sample,
            props,
            opts,
            None,
            None,
            shared,
            true,
            false,
        )?);
        trees.insert(key, Rc::clone(&tree));
        tree
    };
    Ok(TemporalSample {
        at: sample.at,
        time: sample.sample,
        tree,
        tree_invariant: sample_prepared.temporal_tree_invariant,
    })
}

/// Evaluate each exposure at an exact output-time offset from the frame, through the host's
/// clock. A distinct evaluation round and layout belong to every sample; samples across Shutter
/// nodes that show the same share one immutable result.
fn resolve_shutter_samples(
    prepared: &PreparedScene,
    ctx: &valle_motion::MotionContext,
    props: &ResolvedProps,
    opts: &LayoutOptions<'_>,
    painted: Option<&HashSet<String>>,
    trees: &mut SampleTrees,
    mut shared: Option<&mut RequestValueCache>,
) -> Result<HashMap<String, Vec<TemporalSample>>, LayoutError> {
    let artifact = prepared.artifact();
    let shutters = prepared
        .dependencies
        .temporal_nodes()
        .iter()
        .filter_map(|temporal| match temporal.sampling {
            TemporalSampling::Shutter { .. } => Some((
                artifact.nodes[temporal.node.0 as usize].key.as_str(),
                temporal.offsets.as_slice(),
            )),
            _ => None,
        })
        .collect::<Vec<_>>();
    if shutters.is_empty() {
        return Ok(HashMap::new());
    }
    if prepared.dependencies.temporal_sample_count() > 128 {
        return Err(LayoutError::BadShutter {
            node: shutters[0].0.to_owned(),
            reason: "one frame may request at most 128 temporal samples".into(),
        });
    }
    let mut out = HashMap::new();
    for (key, phases) in shutters {
        if painted.is_some_and(|scopes| !scopes.contains(key)) {
            continue;
        }
        let mut exposure = Vec::with_capacity(phases.len());
        super::temporal::visit_sample_contexts(true, key, phases, ctx, |sample| {
            exposure.push(sample_tree(
                prepared,
                sample,
                props,
                opts,
                trees,
                shared.as_deref_mut(),
            )?);
            Ok(())
        })?;
        out.insert(key.to_owned(), exposure);
    }
    Ok(out)
}

/// Evaluate earlier output times, oldest first, through the host's clock. The current subtree is
/// painted separately above these samples, so a stationary subject naturally covers all of its
/// converged echoes.
fn resolve_echo_samples(
    prepared: &PreparedScene,
    ctx: &valle_motion::MotionContext,
    props: &ResolvedProps,
    opts: &LayoutOptions<'_>,
    painted: Option<&HashSet<String>>,
    trees: &mut SampleTrees,
    mut shared: Option<&mut RequestValueCache>,
) -> Result<HashMap<String, Vec<EchoSample>>, LayoutError> {
    let artifact = prepared.artifact();
    let echoes = prepared
        .dependencies
        .temporal_nodes()
        .iter()
        .filter_map(|temporal| match temporal.sampling {
            TemporalSampling::Echo { count, decay, .. } => Some((
                artifact.nodes[temporal.node.0 as usize].key.as_str(),
                count,
                decay,
                temporal.offsets.as_slice(),
            )),
            _ => None,
        })
        .collect::<Vec<_>>();
    if echoes.is_empty() {
        return Ok(HashMap::new());
    }
    if prepared.dependencies.temporal_sample_count() > 128 {
        return Err(LayoutError::BadEcho {
            node: echoes[0].0.to_owned(),
            reason: "one frame may request at most 128 temporal samples".into(),
        });
    }
    let mut out = HashMap::new();
    for (key, count, decay, offsets) in echoes {
        if painted.is_some_and(|scopes| !scopes.contains(key)) {
            continue;
        }
        let mut trail = Vec::with_capacity(offsets.len());
        // Echo offsets are output seconds.
        super::temporal::visit_sample_contexts(false, key, offsets, ctx, |sample| {
            let lag = count - trail.len() as u8;
            trail.push(EchoSample {
                sample: sample_tree(prepared, sample, props, opts, trees, shared.as_deref_mut())?,
                opacity: valle_draw::math::pow(decay, f64::from(lag)) as f32,
            });
            Ok(())
        })?;
        out.insert(key.to_owned(), trail);
    }
    Ok(out)
}

/// Shutter and Echo scopes the current frame can paint. Emission reads a scope's samples only
/// from that scope's own stacking context, and stacking-context construction drops the complete
/// render subtree of an invisible node. Ignoring Takumi's other pruning rules over-approximates
/// the painted set. `None` keeps every scope when the render tree and layout do not pair.
fn painted_temporal_scopes(
    artifact: &SceneArtifact,
    prepared: &PreparedScene,
    root: &RenderNode,
    layout: &LayoutResults,
    keys: &HashMap<u64, String>,
    render_keys: &HashMap<Vec<usize>, String>,
) -> Option<HashSet<String>> {
    let scopes: HashSet<&str> = prepared
        .dependencies
        .temporal_nodes()
        .iter()
        .map(|temporal| artifact.nodes[temporal.node.0 as usize].key.as_str())
        .collect();
    let mut painted = HashSet::new();
    let mut visits = vec![(Vec::new(), takumi_core::geometry::NodeId::ROOT)];
    while let Some((path, id)) = visits.pop() {
        let node = root.node_at_path(&path)?;
        if node.context.style.is_invisible() {
            continue;
        }
        if let Some(key) = keys.get(&u64::from(id)).or_else(|| render_keys.get(&path))
            && scopes.contains(key.as_str())
        {
            painted.insert(key.clone());
        }
        if node.children.is_none() {
            continue;
        }
        for child in layout.box_children(id).ok()? {
            let mut path = path.clone();
            path.push(child.render_index);
            visits.push((path, child.node_id));
        }
    }
    Some(painted)
}

/// Only a scene with stable layout and affine, inline pixel translations can use the chain-rule
/// path. Reject any other dynamic transform for the whole scene: it may move an ancestor or alter
/// the coordinate basis of a target. Analysis runs once at preparation, not on every frame.
fn analytic_auto_blur_translations(
    artifact: &SceneArtifact,
    dependencies: &SceneDependencies,
    layout_reusable: bool,
) -> Option<HashMap<String, valle_draw::Point>> {
    if !layout_reusable
        || dependencies.auto_blur_nodes().is_empty()
        || artifact
            .nodes
            .iter()
            .any(|node| matches!(node.kind, NodeKind::TimeScope { .. }))
    {
        return None;
    }
    let mut translations = HashMap::new();
    for node in &artifact.nodes {
        if node.styles.iter().any(|style| {
            style.property.starts_with("motion-")
                && !style.property.starts_with("motion-velocity-blur-")
        }) {
            return None;
        }
        for style in &node.styles {
            let StyleValue::Expr { expr } = style.value else {
                continue;
            };
            if style.property == "translate" {
                // An important utility can override the inline declaration after evaluation.
                if node
                    .class_names
                    .iter()
                    .chain(node.class_conditions.keys())
                    .any(|class| crate::tailwind::explicit_property(class) == Some("translate"))
                {
                    return None;
                }
                let velocity = analytic_translate_derivative(&artifact.exprs, expr)?;
                if translations.insert(node.key.clone(), velocity).is_some() {
                    return None;
                }
            } else if crate::style::property_spec(&style.property)
                .impact
                .containing_block
            {
                return None;
            }
        }
    }
    Some(translations)
}

fn analytic_translate_derivative(exprs: &[Expr], id: ExprId) -> Option<valle_draw::Point> {
    let (x, y) = match exprs.get(id.0 as usize)? {
        Expr::MakePoint { x, y } => (*x, *y),
        Expr::ToLength2 { input } => match exprs.get(input.0 as usize)? {
            Expr::MakePoint { x, y } => (*x, *y),
            _ => return None,
        },
        _ => return None,
    };
    let derivative = |id: ExprId| match exprs.get(id.0 as usize)? {
        Expr::Const {
            value: MotionValue::Number(_),
        } => Some(0.0),
        _ => {
            let function = crate::time_function::TimeFunction::compile(exprs, id)?;
            (function.input() == valle_motion::ContextInput::CompositionSeconds)
                .then_some(function.derivative())
        }
    };
    let velocity = valle_draw::Point::new(derivative(x)?, derivative(y)?);
    (velocity.x.is_finite() && velocity.y.is_finite()).then_some(velocity)
}

/// Automatic motion blur exposes a 180-degree shutter: a quarter of a frame on each side.
const AUTO_BLUR_SHUTTER_DEGREES: f64 = 180.0;

/// The samples at the two ends of an exposure around the sample `ctx`, half of a shutter of
/// `degrees` on each side, counted in whole degrees as Shutter counts them.
fn exposure_ends(
    ctx: &valle_motion::MotionContext,
    degrees: f64,
) -> Option<[valle_motion::MotionContext; 2]> {
    let half = valle_timeline::RationalTime::new(degrees.round() as i64, 720)
        .ok()
        .and_then(|share| ctx.frame_duration()?.checked_mul(share).ok())?;
    Some([
        ctx.at(ctx.at.checked_sub(half).ok()?)?,
        ctx.at(ctx.at.checked_add(half).ok()?)?,
    ])
}

/// Whether a trail end stays within the reach the filter accepts.
fn trail_end_fits(end: valle_draw::Point) -> bool {
    end.x.is_finite() && end.y.is_finite() && end.x.abs() <= 2048.0 && end.y.abs() <= 2048.0
}

/// Apply the derivative of each CSS `translate` before its fixed local rotation/scale. The parent
/// matrix's linear part carries the local velocity into screen space; the accumulated parent
/// velocity then carries ancestor translations to every descendant. Geometry is already laid out.
/// Each end of a trail is that screen velocity over the change of the scene clock to the end,
/// `elapsed`, which is where the sampled path would find the node.
fn analytic_auto_blur_trails(
    root: &RenderNode,
    layout: &LayoutResults,
    keys: &HashMap<u64, String>,
    translations: &HashMap<String, valle_draw::Point>,
    targets: &[&str],
    elapsed: [f64; 2],
) -> HashMap<String, [valle_draw::Point; 2]> {
    let target_keys: std::collections::HashSet<_> = targets.iter().copied().collect();
    let mut trails = HashMap::new();
    let mut pending = vec![(
        root,
        takumi_core::geometry::NodeId::ROOT,
        TAffine::IDENTITY,
        valle_draw::Point::new(0.0, 0.0),
    )];
    while let Some((node, id, parent, inherited)) = pending.pop() {
        let Ok(box_layout) = layout.layout(id) else {
            continue;
        };
        let local = node.context.style.local_transform(
            box_layout.size.width,
            box_layout.size.height,
            &node.context.sizing,
        );
        let matrix =
            parent * TAffine::translation(box_layout.location.x, box_layout.location.y) * local;
        let mut screen = inherited;
        if let Some(key) = keys.get(&u64::from(id)) {
            if let Some(velocity) = translations.get(key) {
                screen.x += f64::from(parent.a) * velocity.x + f64::from(parent.c) * velocity.y;
                screen.y += f64::from(parent.b) * velocity.x + f64::from(parent.d) * velocity.y;
            }
            let trail = elapsed
                .map(|seconds| valle_draw::Point::new(screen.x * seconds, screen.y * seconds));
            if target_keys.contains(key.as_str()) && trail.into_iter().all(trail_end_fits) {
                trails.insert(key.clone(), trail);
            }
        }
        if let (Some(children), Ok(boxes)) = (&node.children, layout.box_children(id)) {
            for child in boxes {
                if let Some(node) = children.get(child.render_index) {
                    pending.push((node, child.node_id, matrix, screen));
                }
            }
        }
    }
    trails
}

/// The trail of each automatically blurred node over the exposure around the sample `ctx`: its
/// final screen place at either end, relative to its place now. The regular layout path is
/// reused for the two ends, so flow layout, ancestor transforms and camera movement contribute.
/// The ends follow the host's clock, so a trail stops at a cut and stays still while the source
/// holds; a value that jumps inside the exposure draws a streak across the jump. A node missing at
/// either end, or reaching past 2048px, has no blur; the requested frame does not change.
#[expect(
    clippy::too_many_arguments,
    reason = "Keep the layout pass inputs and its independently borrowed caches explicit."
)]
fn auto_blur_trails(
    prepared: &PreparedScene,
    ctx: &valle_motion::MotionContext,
    props: &ResolvedProps,
    opts: &LayoutOptions<'_>,
    root: &RenderNode,
    layout: &Arc<LayoutResults>,
    keys: &Arc<HashMap<u64, String>>,
    css_3d_planes: &HashMap<String, crate::layout::bridge::Css3dPlane>,
    mut shared: Option<&mut RequestValueCache>,
) -> (HashMap<String, [valle_draw::Point; 2]>, u8) {
    let targets = prepared
        .dependencies
        .auto_blur_nodes()
        .iter()
        .map(|id| prepared.artifact.nodes[id.0 as usize].key.as_str())
        .collect::<Vec<_>>();
    if targets.is_empty() {
        return (HashMap::new(), 0);
    }
    let Some(ends) = exposure_ends(ctx, AUTO_BLUR_SHUTTER_DEGREES) else {
        return (HashMap::new(), 0);
    };
    if let Some(translations) = prepared.auto_blur_translations.as_ref()
        && css_3d_planes.is_empty()
    {
        let now = ctx.sample.composition().as_f64();
        let elapsed = ends.map(|end| end.sample.composition().as_f64() - now);
        return (
            analytic_auto_blur_trails(root, layout, keys, translations, &targets, elapsed),
            0,
        );
    }
    let mut anchors_at = |sample: &valle_motion::MotionContext| {
        build_tree_inner(
            prepared.sample_scene.as_deref().unwrap_or(prepared),
            sample,
            props,
            opts,
            None,
            None,
            shared.as_deref_mut(),
            false,
            false,
        )
        .ok()
        .map(|tree| screen_anchors(&tree.root, &tree.layout, &tree.keys, &tree.css_3d_planes))
    };
    let before = anchors_at(&ends[0]);
    let after = anchors_at(&ends[1]);
    let center = screen_anchors(root, layout, keys, css_3d_planes);
    let mut trails = HashMap::new();
    for key in targets {
        let (Some(now), Some(before), Some(after)) = (
            center.get(key),
            before.as_ref().and_then(|anchors| anchors.get(key)),
            after.as_ref().and_then(|anchors| anchors.get(key)),
        ) else {
            continue;
        };
        let trail = [before, after].map(|end| valle_draw::Point::new(end.x - now.x, end.y - now.y));
        if trail.into_iter().all(trail_end_fits) {
            trails.insert(key.to_owned(), trail);
        }
    }
    (trails, 2)
}

fn screen_anchors(
    root: &RenderNode,
    layout: &LayoutResults,
    keys: &HashMap<u64, String>,
    css_3d_planes: &HashMap<String, crate::layout::bridge::Css3dPlane>,
) -> HashMap<String, valle_draw::Point> {
    let mut anchors = HashMap::new();
    let mut pending = vec![(root, takumi_core::geometry::NodeId::ROOT, TAffine::IDENTITY)];
    while let Some((node, id, parent)) = pending.pop() {
        let Ok(box_layout) = layout.layout(id) else {
            continue;
        };
        let local = node.context.style.local_transform(
            box_layout.size.width,
            box_layout.size.height,
            &node.context.sizing,
        );
        let matrix =
            parent * TAffine::translation(box_layout.location.x, box_layout.location.y) * local;
        if let Some(key) = keys.get(&u64::from(id)) {
            let center_x = box_layout.size.width * 0.5;
            let center_y = box_layout.size.height * 0.5;
            let point = if let Some(plane) = css_3d_planes.get(key).filter(|plane| plane.project) {
                valle_draw::Point::new(
                    plane.quad.iter().map(|point| point.x).sum::<f64>() / 4.0,
                    plane.quad.iter().map(|point| point.y).sum::<f64>() / 4.0,
                )
            } else {
                valle_draw::Point::new(
                    f64::from(matrix.a * center_x + matrix.c * center_y + matrix.x),
                    f64::from(matrix.b * center_x + matrix.d * center_y + matrix.y),
                )
            };
            if point.x.is_finite() && point.y.is_finite() {
                anchors.insert(key.clone(), point);
            }
        }
        if let (Some(children), Ok(boxes)) = (&node.children, layout.box_children(id)) {
            for child in boxes {
                if let Some(node) = children.get(child.render_index) {
                    pending.push((node, child.node_id, matrix));
                }
            }
        }
    }
    anchors
}

#[derive(Debug, Clone)]
pub(super) struct ReviewNodeSample {
    pub opacity: f64,
    pub cycle: i64,
    pub group: Option<String>,
    pub painted: bool,
    pub position: valle_draw::Point,
    /// Axis-aligned screen bounds after the final 2D or projected 3D transform.
    pub bounds: [f64; 4],
    pub visible: bool,
    pub on_screen: bool,
    pub motion_blurred: bool,
}

impl LayoutTree {
    /// Inspect the same post-layout screen anchors and resolved blur filters used by rendering.
    /// A CSS `visibility: visible` child may override a hidden parent, while opacity multiplies
    /// through the ancestor chain.
    pub(super) fn review_node_samples(&self) -> HashMap<String, ReviewNodeSample> {
        use takumi_core::style::Visibility;
        use valle_draw::program::recording::FilterOp;

        let mut samples = HashMap::new();
        let mut pending = vec![(
            &self.root,
            takumi_core::geometry::NodeId::ROOT,
            TAffine::IDENTITY,
            1.0_f32,
            false,
        )];
        while let Some((node, id, parent, parent_opacity, parent_blurred)) = pending.pop() {
            let Ok(box_layout) = self.layout.layout(id) else {
                continue;
            };
            let local = node.context.style.local_transform(
                box_layout.size.width,
                box_layout.size.height,
                &node.context.sizing,
            );
            let matrix =
                parent * TAffine::translation(box_layout.location.x, box_layout.location.y) * local;
            let opacity = parent_opacity * node.context.style.opacity.0;
            let key = self.keys.get(&u64::from(id));
            let blurred = parent_blurred
                || key.is_some_and(|key| {
                    self.advanced_filters.get(key).is_some_and(|filters| {
                        filters
                            .iter()
                            .any(|filter| matches!(filter, FilterOp::VelocityBlur { .. }))
                    })
                });
            if let Some(key) = key {
                let width = box_layout.size.width;
                let height = box_layout.size.height;
                let plane = self.css_3d_planes.get(key).filter(|plane| plane.project);
                let painted = node
                    .context
                    .style
                    .background_color
                    .resolve(node.context.current_color)
                    .0[3]
                    != 0
                    || node
                        .context
                        .style
                        .background_image
                        .as_ref()
                        .is_some_and(|images| !images.is_empty())
                    || [
                        (box_layout.border.top, node.context.style.border_top_color),
                        (
                            box_layout.border.right,
                            node.context.style.border_right_color,
                        ),
                        (
                            box_layout.border.bottom,
                            node.context.style.border_bottom_color,
                        ),
                        (box_layout.border.left, node.context.style.border_left_color),
                    ]
                    .iter()
                    .any(|(width, color)| {
                        *width > 0.0 && color.resolve(node.context.current_color).0[3] != 0
                    });
                // A path paints its points, which need not lie in its box: a bare `<Path>` in a
                // block has a box of no height. `None` inside when it paints nothing.
                let path = self.paths.get(key).map(|path| {
                    path_paint_rect(
                        path,
                        painted.then_some([0.0, 0.0, f64::from(width), f64::from(height)]),
                    )
                });
                let place = |x: f64, y: f64| match plane {
                    Some(plane) => plane
                        .matrix
                        .try_apply(valle_draw::Point::new(x, y))
                        .unwrap_or(valle_draw::Point::new(f64::NAN, f64::NAN)),
                    None => valle_draw::Point::new(
                        f64::from(matrix.a) * x + f64::from(matrix.c) * y + f64::from(matrix.x),
                        f64::from(matrix.b) * x + f64::from(matrix.d) * y + f64::from(matrix.y),
                    ),
                };
                let corners = match path.flatten() {
                    Some([left, top, right, bottom]) => [
                        place(left, top),
                        place(right, top),
                        place(right, bottom),
                        place(left, bottom),
                    ],
                    None => plane.map_or_else(
                        || {
                            let point = |x: f32, y: f32| {
                                valle_draw::Point::new(
                                    f64::from(matrix.a * x + matrix.c * y + matrix.x),
                                    f64::from(matrix.b * x + matrix.d * y + matrix.y),
                                )
                            };
                            [
                                point(0.0, 0.0),
                                point(width, 0.0),
                                point(width, height),
                                point(0.0, height),
                            ]
                        },
                        |plane| plane.quad,
                    ),
                };
                let position = if let Some(Some([left, top, right, bottom])) = path {
                    place((left + right) * 0.5, (top + bottom) * 0.5)
                } else if plane.is_some() {
                    valle_draw::Point::new(
                        corners.iter().map(|point| point.x).sum::<f64>() / 4.0,
                        corners.iter().map(|point| point.y).sum::<f64>() / 4.0,
                    )
                } else {
                    valle_draw::Point::new(
                        f64::from(matrix.a * width * 0.5 + matrix.c * height * 0.5 + matrix.x),
                        f64::from(matrix.b * width * 0.5 + matrix.d * height * 0.5 + matrix.y),
                    )
                };
                let left = corners
                    .iter()
                    .map(|point| point.x)
                    .fold(f64::INFINITY, f64::min);
                let right = corners
                    .iter()
                    .map(|point| point.x)
                    .fold(f64::NEG_INFINITY, f64::max);
                let top = corners
                    .iter()
                    .map(|point| point.y)
                    .fold(f64::INFINITY, f64::min);
                let bottom = corners
                    .iter()
                    .map(|point| point.y)
                    .fold(f64::NEG_INFINITY, f64::max);
                let on_screen = self
                    .viewport
                    .size
                    .width
                    .zip(self.viewport.size.height)
                    .is_none_or(|(viewport_width, viewport_height)| {
                        right > 0.0
                            && bottom > 0.0
                            && left < f64::from(viewport_width)
                            && top < f64::from(viewport_height)
                    });
                if position.x.is_finite()
                    && position.y.is_finite()
                    && [left, top, right, bottom]
                        .iter()
                        .all(|value| value.is_finite())
                {
                    samples.insert(
                        key.clone(),
                        ReviewNodeSample {
                            opacity: f64::from(opacity),
                            cycle: 0,
                            group: None,
                            painted,
                            position,
                            bounds: [left, top, right, bottom],
                            visible: node.context.style.visibility == Visibility::Visible
                                && opacity > 0.0
                                && !plane.is_some_and(|plane| plane.hidden)
                                && !matches!(path, Some(None)),
                            on_screen,
                            motion_blurred: blurred,
                        },
                    );
                }
            }
            if let Some(key) = key
                && let Some(batch) = self.batches.get(key)
            {
                use valle_draw::program::{Affine2d, recording::BatchGeometry};
                let parent = Affine2d([
                    f64::from(matrix.a),
                    f64::from(matrix.b),
                    f64::from(matrix.c),
                    f64::from(matrix.d),
                    f64::from(matrix.x),
                    f64::from(matrix.y),
                ]);
                for (row, instance) in batch.instances.iter().enumerate() {
                    let hull = if let Some(path) = &batch.path {
                        path.points.iter().fold(
                            [
                                f64::INFINITY,
                                f64::INFINITY,
                                f64::NEG_INFINITY,
                                f64::NEG_INFINITY,
                            ],
                            |b, p| [b[0].min(p.x), b[1].min(p.y), b[2].max(p.x), b[3].max(p.y)],
                        )
                    } else if batch.exact_circle_paths || batch.geometry == BatchGeometry::Circle {
                        [-1.0, -1.0, 1.0, 1.0]
                    } else {
                        [0.0, 0.0, 1.0, 1.0]
                    };
                    let transform = instance.transform(batch.geometry).then(parent).0;
                    let [a, b, c, d, e, f] = transform;
                    let pad = f64::from(instance.stroke_width) * 0.5;
                    let [x0, y0, x1, y1] =
                        [hull[0] - pad, hull[1] - pad, hull[2] + pad, hull[3] + pad];
                    let points = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
                        .map(|[x, y]| [a * x + c * y + e, b * x + d * y + f]);
                    let bounds = points.iter().fold(
                        [
                            f64::INFINITY,
                            f64::INFINITY,
                            f64::NEG_INFINITY,
                            f64::NEG_INFINITY,
                        ],
                        |r, p| {
                            [
                                r[0].min(p[0]),
                                r[1].min(p[1]),
                                r[2].max(p[0]),
                                r[3].max(p[1]),
                            ]
                        },
                    );
                    let on_screen = self
                        .viewport
                        .size
                        .width
                        .zip(self.viewport.size.height)
                        .is_none_or(|(w, h)| {
                            bounds[2] > 0.0
                                && bounds[3] > 0.0
                                && bounds[0] < f64::from(w)
                                && bounds[1] < f64::from(h)
                        });
                    let row_key = batch
                        .semantic_keys
                        .get(row)
                        .cloned()
                        .unwrap_or_else(|| format!("{key}/{row}"));
                    samples.insert(
                        row_key,
                        ReviewNodeSample {
                            opacity: f64::from(opacity * instance.opacity * instance.color.alpha),
                            cycle: batch.row_identity.get(row).map_or(0, |row| row.cycle),
                            group: Some(key.clone()),
                            painted: true,
                            position: valle_draw::Point::new(
                                (bounds[0] + bounds[2]) * 0.5,
                                (bounds[1] + bounds[3]) * 0.5,
                            ),
                            bounds,
                            on_screen,
                            motion_blurred: blurred,
                            visible: node.context.style.visibility == Visibility::Visible
                                && opacity * instance.opacity * instance.color.alpha > 0.0,
                        },
                    );
                }
            }
            if let (Some(children), Ok(boxes)) = (&node.children, self.layout.box_children(id)) {
                for child in boxes {
                    if let Some(node) = children.get(child.render_index) {
                        pending.push((node, child.node_id, matrix, opacity, blurred));
                    }
                }
            }
        }
        samples
    }
}

/// The local rectangle a path node paints: the hull of its points, outset by half its stroke, and
/// `painted_box` when its box paints a background or a border. `None` when it paints neither.
fn path_paint_rect(
    path: &crate::layout::bridge::PathContent,
    painted_box: Option<[f64; 4]>,
) -> Option<[f64; 4]> {
    let pad = path
        .stroke
        .as_ref()
        .map_or(0.0, |stroke| stroke.width * 0.5);
    path.points
        .iter()
        .map(|point| [point.x - pad, point.y - pad, point.x + pad, point.y + pad])
        .chain(painted_box)
        .reduce(|a, b| {
            [
                a[0].min(b[0]),
                a[1].min(b[1]),
                a[2].max(b[2]),
                a[3].max(b[3]),
            ]
        })
}

fn resolve_advanced_filters(
    node: &valle_motion::SceneNode,
    values: &[MotionValue],
    at: usize,
    ctx: &valle_motion::MotionContext,
    auto_trail: Option<[valle_draw::Point; 2]>,
) -> Result<Vec<valle_draw::program::recording::FilterOp>, LayoutError> {
    use valle_draw::program::recording::FilterOp;

    let mut filters = Vec::new();
    if node
        .styles
        .iter()
        .any(|style| style.property.starts_with("motion-bloom-"))
    {
        let mut params = [0.0; 4];
        for (index, (name, max)) in [
            ("threshold", 1.0),
            ("knee", 1.0),
            ("intensity", 4.0),
            ("radius", 128.0),
        ]
        .into_iter()
        .enumerate()
        {
            let property = format!("motion-bloom-{name}");
            let Some(MotionValue::Number(value)) = resolved_style(node, values, at, &property)?
            else {
                return Err(bad_advanced_filter(
                    node,
                    format!("Scene bloom {name} is missing or not a number"),
                ));
            };
            if !value.is_finite() || !(0.0..=max).contains(value) {
                return Err(bad_advanced_filter(
                    node,
                    format!(
                        "Scene bloom {name} = {value} is outside 0..={max}; use clamp() explicitly if intended"
                    ),
                ));
            }
            params[index] = *value;
        }
        filters.push(FilterOp::Bloom {
            threshold: params[0],
            knee: params[1],
            intensity: params[2],
            radius: params[3],
        });
    }
    if node
        .styles
        .iter()
        .any(|style| style.property.starts_with("motion-radial-blur-"))
    {
        let mut params = [0.0; 3];
        for (index, (property, max, signed)) in [
            ("motion-radial-blur-center-x", 10_000_000.0, true),
            ("motion-radial-blur-center-y", 10_000_000.0, true),
            ("motion-radial-blur-amount", 128.0, false),
        ]
        .into_iter()
        .enumerate()
        {
            let Some(MotionValue::Number(value)) = resolved_style(node, values, at, property)?
            else {
                return Err(bad_advanced_filter(
                    node,
                    "radial blur parameter is missing or invalid",
                ));
            };
            if !value.is_finite()
                || (signed && value.abs() > max)
                || (!signed && !(0.0..=max).contains(value))
            {
                return Err(bad_advanced_filter(
                    node,
                    "radial blur parameter is out of bounds",
                ));
            }
            params[index] = *value;
        }
        filters.push(FilterOp::RadialBlur {
            center_x: params[0],
            center_y: params[1],
            amount: params[2],
        });
    }
    if node
        .styles
        .iter()
        .any(|style| style.property.starts_with("motion-film-grain-"))
    {
        let mut params = [0.0; 3];
        for (index, property) in [
            "motion-film-grain-seed",
            "motion-film-grain-amount",
            "motion-film-grain-size",
        ]
        .into_iter()
        .enumerate()
        {
            let Some(MotionValue::Number(value)) = resolved_style(node, values, at, property)?
            else {
                return Err(bad_advanced_filter(
                    node,
                    "film grain parameter is missing or invalid",
                ));
            };
            params[index] = *value;
        }
        if !params[0].is_finite()
            || params[0].fract() != 0.0
            || !(0.0..=u32::MAX as f64).contains(&params[0])
            || !params[1].is_finite()
            || !(0.0..=1.0).contains(&params[1])
            || !params[2].is_finite()
            || !(1.0..=64.0).contains(&params[2])
        {
            return Err(bad_advanced_filter(
                node,
                "film grain parameter is out of bounds",
            ));
        }
        filters.push(FilterOp::FilmGrain {
            seed: params[0] as u32,
            amount: params[1],
            size: params[2],
        });
    }
    if node
        .styles
        .iter()
        .any(|style| style.property.starts_with("motion-lens-distortion-"))
    {
        let mut params = [0.0; 2];
        for (index, property) in ["motion-lens-distortion-k1", "motion-lens-distortion-k2"]
            .into_iter()
            .enumerate()
        {
            let Some(MotionValue::Number(value)) = resolved_style(node, values, at, property)?
            else {
                return Err(bad_advanced_filter(
                    node,
                    "lens distortion parameter is missing or invalid",
                ));
            };
            if !value.is_finite() || value.abs() > 0.5 {
                return Err(bad_advanced_filter(
                    node,
                    "lens distortion parameter is out of bounds",
                ));
            }
            params[index] = *value;
        }
        filters.push(FilterOp::LensDistortion {
            k1: params[0],
            k2: params[1],
        });
    }
    if node
        .styles
        .iter()
        .any(|style| style.property.starts_with("motion-glow-"))
    {
        let Some(MotionValue::Number(radius)) =
            resolved_style(node, values, at, "motion-glow-radius")?
        else {
            return Err(bad_advanced_filter(
                node,
                "glow radius is missing or invalid",
            ));
        };
        let Some(MotionValue::Number(intensity)) =
            resolved_style(node, values, at, "motion-glow-intensity")?
        else {
            return Err(bad_advanced_filter(
                node,
                "glow intensity is missing or invalid",
            ));
        };
        let Some(MotionValue::Color(color)) =
            resolved_style(node, values, at, "motion-glow-color")?
        else {
            return Err(bad_advanced_filter(
                node,
                "glow color is missing or invalid",
            ));
        };
        if !radius.is_finite()
            || !(0.0..=128.0).contains(radius)
            || !intensity.is_finite()
            || !(0.0..=4.0).contains(intensity)
        {
            return Err(bad_advanced_filter(
                node,
                "glow radius or intensity is out of bounds",
            ));
        }
        filters.push(FilterOp::Glow {
            color: *color,
            radius: *radius,
            intensity: *intensity,
        });
    }
    if let Some(value) = resolved_style(node, values, at, "motion-chromatic-aberration-offset")? {
        let MotionValue::Number(offset) = value else {
            return Err(bad_advanced_filter(
                node,
                "chromatic aberration offset must be a number",
            ));
        };
        if !offset.is_finite() || offset.abs() > 256.0 {
            return Err(bad_advanced_filter(
                node,
                "chromatic aberration offset must be within ±256px",
            ));
        }
        filters.push(FilterOp::ChromaticAberration {
            offset_x: *offset,
            offset_y: 0.0,
        });
    }
    let has_displacement = node
        .styles
        .iter()
        .any(|style| style.property.starts_with("motion-displacement-"));
    if has_displacement {
        let Some(MotionValue::Number(seed)) =
            resolved_style(node, values, at, "motion-displacement-seed")?
        else {
            return Err(bad_advanced_filter(
                node,
                "displacement seed is missing or is not a number",
            ));
        };
        let Some(MotionValue::Point(frequency)) =
            resolved_style(node, values, at, "motion-displacement-frequency")?
        else {
            return Err(bad_advanced_filter(
                node,
                "displacement frequency must be point(x, y)",
            ));
        };
        let Some(MotionValue::Number(scale)) =
            resolved_style(node, values, at, "motion-displacement-scale")?
        else {
            return Err(bad_advanced_filter(
                node,
                "displacement scale must be a number",
            ));
        };
        let Some(MotionValue::Number(octaves)) =
            resolved_style(node, values, at, "motion-displacement-octaves")?
        else {
            return Err(bad_advanced_filter(
                node,
                "displacement octaves must be a number",
            ));
        };
        let Some(MotionValue::Enum(mode)) =
            resolved_style(node, values, at, "motion-displacement-mode")?
        else {
            return Err(bad_advanced_filter(
                node,
                "displacement mode must be an enum",
            ));
        };
        if !seed.is_finite() || seed.fract() != 0.0 || !(0.0..=f64::from(u32::MAX)).contains(seed) {
            return Err(bad_advanced_filter(
                node,
                "displacement seed must be a finite integer in 0..=4294967295 this frame; the engine does not round, clamp, wrap, or reuse the previous seed",
            ));
        }
        if !frequency.x.is_finite()
            || !frequency.y.is_finite()
            || !(0.000_001..=4.0).contains(&frequency.x)
            || !(0.000_001..=4.0).contains(&frequency.y)
            || !scale.is_finite()
            || scale.abs() > 512.0
            || octaves.fract() != 0.0
            || !(1.0..=4.0).contains(octaves)
            || !matches!(mode.as_str(), "fractal" | "turbulence")
        {
            return Err(bad_advanced_filter(
                node,
                "displacement requires frequency 0.000001..=4, |scale| <= 512, octaves 1..=4, and a valid mode",
            ));
        }
        filters.push(FilterOp::NoiseDisplacement {
            frequency_x: frequency.x,
            frequency_y: frequency.y,
            octaves: *octaves as u8,
            seed: *seed as u32,
            scale: *scale,
            turbulence: mode == "turbulence",
        });
    }

    let has_velocity_blur = node
        .styles
        .iter()
        .any(|style| style.property.starts_with("motion-velocity-blur-"));
    if has_velocity_blur {
        let is_auto = node
            .styles
            .iter()
            .any(|style| style.property == "motion-velocity-blur-auto");
        let Some(MotionValue::Number(shutter_angle)) =
            resolved_style(node, values, at, "motion-velocity-blur-shutter")?
        else {
            return Err(bad_advanced_filter(
                node,
                "motion blur shutter angle must be a number",
            ));
        };
        if !shutter_angle.is_finite() || !(0.0..=360.0).contains(shutter_angle) {
            return Err(bad_advanced_filter(
                node,
                "motion blur requires a shutter angle in 0..=360",
            ));
        }
        let trail = if is_auto {
            let Some(trail) = auto_trail else {
                return Ok(filters);
            };
            trail
        } else {
            let Some(MotionValue::Point(velocity)) =
                resolved_style(node, values, at, "motion-velocity-blur-velocity")?
            else {
                return Err(bad_advanced_filter(
                    node,
                    "motion blur velocity is missing or is not point(x, y)",
                ));
            };
            // The velocity counts per second of the scene clock, so each end is what the clock
            // runs to it: unwrapped through a declared loop, stopped at a cut and still in a hold.
            let ends = exposure_ends(ctx, *shutter_angle).ok_or_else(|| {
                bad_advanced_filter(node, "motion blur exposure overflows exact time")
            })?;
            let trail = ends.map(|end| {
                let seconds = end.elapsed.as_f64() - ctx.elapsed.as_f64();
                valle_draw::Point::new(velocity.x * seconds, velocity.y * seconds)
            });
            if !trail.into_iter().all(trail_end_fits) {
                return Err(bad_advanced_filter(
                    node,
                    "motion blur must stay within 2048 px of the node on each side of the exposure",
                ));
            }
            trail
        };
        if trail
            .iter()
            .all(|end| end.x.abs() < 0.001 && end.y.abs() < 0.001)
        {
            return Ok(filters);
        }
        filters.push(FilterOp::VelocityBlur {
            from_x: trail[0].x,
            from_y: trail[0].y,
            to_x: trail[1].x,
            to_y: trail[1].y,
        });
    }
    Ok(filters)
}

fn resolve_backdrop_advanced_filters(
    node: &valle_motion::SceneNode,
    values: &[MotionValue],
    at: usize,
) -> Result<Vec<valle_draw::program::recording::FilterOp>, LayoutError> {
    use valle_draw::program::recording::FilterOp;

    let mut filters = Vec::new();
    let has_displacement = node
        .styles
        .iter()
        .any(|style| style.property.starts_with("motion-backdrop-displacement-"));
    if !has_displacement {
        return Ok(filters);
    }
    let Some(MotionValue::Number(seed)) =
        resolved_style(node, values, at, "motion-backdrop-displacement-seed")?
    else {
        return Err(bad_advanced_filter(
            node,
            "backdrop displacement seed is missing or is not a number",
        ));
    };
    let Some(MotionValue::Point(frequency)) =
        resolved_style(node, values, at, "motion-backdrop-displacement-frequency")?
    else {
        return Err(bad_advanced_filter(
            node,
            "backdrop displacement frequency must be point(x, y)",
        ));
    };
    let Some(MotionValue::Number(scale)) =
        resolved_style(node, values, at, "motion-backdrop-displacement-scale")?
    else {
        return Err(bad_advanced_filter(
            node,
            "backdrop displacement scale must be a number",
        ));
    };
    let Some(MotionValue::Number(octaves)) =
        resolved_style(node, values, at, "motion-backdrop-displacement-octaves")?
    else {
        return Err(bad_advanced_filter(
            node,
            "backdrop displacement octaves must be a number",
        ));
    };
    let Some(MotionValue::Enum(mode)) =
        resolved_style(node, values, at, "motion-backdrop-displacement-mode")?
    else {
        return Err(bad_advanced_filter(
            node,
            "backdrop displacement mode must be an enum",
        ));
    };
    if !seed.is_finite()
        || seed.fract() != 0.0
        || !(0.0..=f64::from(u32::MAX)).contains(seed)
        || !frequency.x.is_finite()
        || !frequency.y.is_finite()
        || !(0.000_001..=4.0).contains(&frequency.x)
        || !(0.000_001..=4.0).contains(&frequency.y)
        || !scale.is_finite()
        || scale.abs() > 512.0
        || octaves.fract() != 0.0
        || !(1.0..=4.0).contains(octaves)
        || !matches!(mode.as_str(), "fractal" | "turbulence")
    {
        return Err(bad_advanced_filter(
            node,
            "backdrop displacement requires a finite u32 seed, frequency 0.000001..=4, |scale| <= 512, octaves 1..=4, and a valid mode",
        ));
    }
    filters.push(FilterOp::NoiseDisplacement {
        frequency_x: frequency.x,
        frequency_y: frequency.y,
        octaves: *octaves as u8,
        seed: *seed as u32,
        scale: *scale,
        turbulence: mode == "turbulence",
    });
    Ok(filters)
}

/// The uniforms of a shader binding at this frame, in ABI order, each finite and in its range.
fn shader_uniforms(
    node: &str,
    uniforms: &[valle_motion::ShaderUniformBinding],
    values: &[MotionValue],
    at: usize,
) -> Result<Vec<valle_draw::program::recording::ShaderUniformBinding>, LayoutError> {
    uniforms
        .iter()
        .map(|binding| {
            let value = match &binding.value {
                ShaderUniformValue::Float { value } => {
                    valle_draw::program::recording::ShaderUniformValue::Float {
                        value: number_value(value, values, at, "shader float")? as f32,
                    }
                }
                ShaderUniformValue::Float2 { value } => {
                    let value = point_value(value, values, at, "shader float2")?;
                    valle_draw::program::recording::ShaderUniformValue::Float2 {
                        value: [value.x as f32, value.y as f32],
                    }
                }
                ShaderUniformValue::Float3 { value } => {
                    let mut components = [0.0f32; 3];
                    for (slot, component) in components.iter_mut().zip(value) {
                        *slot = number_value(component, values, at, "shader component")? as f32;
                    }
                    valle_draw::program::recording::ShaderUniformValue::Float3 { value: components }
                }
                ShaderUniformValue::Float4 { value } => {
                    let mut components = [0.0f32; 4];
                    for (slot, component) in components.iter_mut().zip(value) {
                        *slot = number_value(component, values, at, "shader component")? as f32;
                    }
                    valle_draw::program::recording::ShaderUniformValue::Float4 { value: components }
                }
                ShaderUniformValue::Float2x2 { value } => {
                    let mut components = [0.0f32; 4];
                    for (slot, component) in components.iter_mut().zip(value) {
                        *slot = number_value(component, values, at, "shader component")? as f32;
                    }
                    valle_draw::program::recording::ShaderUniformValue::Float2x2 {
                        value: components,
                    }
                }
                ShaderUniformValue::Float3x3 { value } => {
                    let mut components = [0.0f32; 9];
                    for (slot, component) in components.iter_mut().zip(value) {
                        *slot = number_value(component, values, at, "shader component")? as f32;
                    }
                    valle_draw::program::recording::ShaderUniformValue::Float3x3 {
                        value: components,
                    }
                }
                ShaderUniformValue::Float4x4 { value } => {
                    let mut components = [0.0f32; 16];
                    for (slot, component) in components.iter_mut().zip(value) {
                        *slot = number_value(component, values, at, "shader component")? as f32;
                    }
                    valle_draw::program::recording::ShaderUniformValue::Float4x4 {
                        value: components,
                    }
                }
                ShaderUniformValue::Color { value } => {
                    let value = color_value(value, values, at)?;
                    valle_draw::program::recording::ShaderUniformValue::Color {
                        value: value.to_srgb_straight(),
                    }
                }
                ShaderUniformValue::Bool { value } => {
                    let value = match value {
                        BoolValue::Static { value } => *value,
                        BoolValue::Expr { expr } => match values.get(expr.0 as usize) {
                            Some(MotionValue::Bool(value)) => *value,
                            _ => {
                                return Err(LayoutError::Eval(EvalError::TypeMismatch {
                                    at,
                                    op: "shader bool",
                                }));
                            }
                        },
                    };
                    valle_draw::program::recording::ShaderUniformValue::Bool { value }
                }
            };
            for component in value.components() {
                if !component.is_finite()
                    || binding
                        .range
                        .is_some_and(|[min, max]| *component < min || *component > max)
                {
                    return Err(LayoutError::BadShader {
                        node: node.to_owned(),
                        reason: format!(
                            "uniform `{}` is non-finite or outside its declared range",
                            binding.name
                        ),
                    });
                }
            }
            Ok(valle_draw::program::recording::ShaderUniformBinding {
                name: binding.name.clone(),
                value,
            })
        })
        .collect()
}

/// The program pin a recording carries for an artifact's shader program.
fn shader_program(
    program: &valle_motion::ShaderProgramRef,
) -> valle_draw::program::recording::ShaderProgram {
    valle_draw::program::recording::ShaderProgram {
        work_per_pixel: program.work_per_pixel,
        padding: program.padding,
        uri: program.uri.clone(),
        content_hash: valle_draw::requirements::DigestBytes::from_bytes(
            *program.content_hash.as_bytes(),
        ),
        abi_hash: valle_draw::requirements::DigestBytes::from_bytes(*program.abi_hash.as_bytes()),
    }
}

fn point_value(
    binding: &PointValue,
    values: &[MotionValue],
    at: usize,
    op: &'static str,
) -> Result<valle_draw::Point, LayoutError> {
    match binding {
        PointValue::Static { value } => Ok(*value),
        PointValue::Expr { expr } => match value(values, *expr, at)? {
            MotionValue::Point(value) => Ok(*value),
            _ => Err(LayoutError::Eval(EvalError::TypeMismatch { at, op })),
        },
    }
}

fn color_value(
    binding: &ColorValue,
    values: &[MotionValue],
    at: usize,
) -> Result<valle_draw::program::AuthorColor, LayoutError> {
    match binding {
        ColorValue::Static { value } => Ok(*value),
        ColorValue::Expr { expr } => match value(values, *expr, at)? {
            MotionValue::Color(value) => Ok(*value),
            _ => Err(LayoutError::Eval(EvalError::TypeMismatch {
                at,
                op: "gradient color",
            })),
        },
    }
}

fn authored_style_colors(
    artifact: &SceneArtifact,
    values: &[MotionValue],
    property: &str,
) -> HashMap<String, valle_draw::program::AuthorColor> {
    artifact
        .nodes
        .iter()
        .filter_map(|node| {
            let binding = node
                .styles
                .iter()
                .rev()
                .find(|style| style.property == property)?;
            let color = match &binding.value {
                StyleValue::Static {
                    value: MotionValue::Color(color),
                } => *color,
                StyleValue::Expr { expr } => match values.get(expr.0 as usize)? {
                    MotionValue::Color(color) => *color,
                    _ => return None,
                },
                _ => return None,
            };
            Some((node.key.clone(), color))
        })
        .collect()
}

fn gradient_stops(
    bindings: &[GradientStopValue],
    values: &[MotionValue],
    at: usize,
    node: &str,
) -> Result<Vec<(f64, valle_draw::program::AuthorColor)>, LayoutError> {
    let mut stops = Vec::with_capacity(bindings.len());
    let mut previous = 0.0;
    for (index, stop) in bindings.iter().enumerate() {
        let offset = number_value(&stop.offset, values, at, "gradient stop offset")?;
        if !offset.is_finite() || !(0.0..=1.0).contains(&offset) || (index > 0 && offset < previous)
        {
            return Err(LayoutError::BadPaint {
                node: node.into(),
                reason: "gradient stops must be finite, sorted, and inside 0..=1".into(),
            });
        }
        previous = offset;
        stops.push((offset, color_value(&stop.color, values, at)?));
    }
    Ok(stops)
}

fn paint_value(
    binding: &PaintValue,
    values: &[MotionValue],
    at: usize,
    node: &str,
) -> Result<crate::layout::bridge::ResolvedPaint, LayoutError> {
    use crate::layout::bridge::ResolvedPaint;
    match binding {
        PaintValue::Solid { color } => Ok(ResolvedPaint::Solid(color_value(color, values, at)?)),
        PaintValue::Linear {
            start,
            end,
            stops,
            spread,
        } => Ok(ResolvedPaint::Linear {
            start: point_value(start, values, at, "linear gradient start")?,
            end: point_value(end, values, at, "linear gradient end")?,
            stops: gradient_stops(stops, values, at, node)?,
            spread: *spread,
        }),
        PaintValue::Radial {
            center,
            radius,
            stops,
            spread,
        } => {
            let radius = number_value(radius, values, at, "radial gradient radius")?;
            if !radius.is_finite() || radius <= 0.0 {
                return Err(LayoutError::BadPaint {
                    node: node.into(),
                    reason: "radial gradient radius must be finite and positive".into(),
                });
            }
            Ok(ResolvedPaint::Radial {
                center: point_value(center, values, at, "radial gradient center")?,
                radius,
                stops: gradient_stops(stops, values, at, node)?,
                spread: *spread,
            })
        }
        PaintValue::Conic {
            center,
            start_angle,
            stops,
            spread,
        } => Ok(ResolvedPaint::Conic {
            center: point_value(center, values, at, "conic gradient center")?,
            start_angle: number_value(start_angle, values, at, "conic gradient start angle")?,
            stops: gradient_stops(stops, values, at, node)?,
            spread: *spread,
        }),
    }
}

fn path_value<'a>(
    binding: &'a PathValue,
    values: &'a [MotionValue],
    at: usize,
) -> Result<&'a PathData, LayoutError> {
    match binding {
        PathValue::Static { value } => Ok(value),
        PathValue::Expr { expr, .. } => match value(values, *expr, at)? {
            MotionValue::PathData(value) => Ok(value),
            _ => Err(LayoutError::Eval(EvalError::TypeMismatch {
                at,
                op: "path d",
            })),
        },
    }
}

fn number_value(
    binding: &NumberValue,
    values: &[MotionValue],
    at: usize,
    op: &'static str,
) -> Result<f64, LayoutError> {
    match binding {
        NumberValue::Static { value } => Ok(*value),
        NumberValue::Expr { expr } => match value(values, *expr, at)? {
            MotionValue::Number(value) => Ok(*value),
            _ => Err(LayoutError::Eval(EvalError::TypeMismatch { at, op })),
        },
    }
}

fn rect_value(
    binding: &RectValue,
    values: &[MotionValue],
    at: usize,
    node: &str,
) -> Result<valle_draw::Rect, LayoutError> {
    let rect = match binding {
        RectValue::Static { value } => *value,
        RectValue::Expr { expr } => match value(values, *expr, at)? {
            MotionValue::Rect(value) => *value,
            _ => {
                return Err(LayoutError::Eval(EvalError::TypeMismatch {
                    at,
                    op: "mask rect",
                }));
            }
        },
    };
    if !rect.x.is_finite()
        || !rect.y.is_finite()
        || !rect.width.is_finite()
        || !rect.height.is_finite()
        || rect.width <= 0.0
        || rect.height <= 0.0
    {
        return Err(LayoutError::BadMask {
            node: node.into(),
            reason: "rect must have finite coordinates and positive size".into(),
        });
    }
    Ok(rect)
}

fn prepare_formula_fragments(
    artifact: &SceneArtifact,
    values: &[MotionValue],
    _opts: &LayoutOptions<'_>,
) -> Result<HashMap<String, crate::math_formula::FormulaFragment>, LayoutError> {
    let mut out = HashMap::new();
    for (at, node) in artifact.nodes.iter().enumerate() {
        let NodeKind::MathFormula { latex, display, .. } = &node.kind else {
            continue;
        };
        #[cfg(not(target_arch = "wasm32"))]
        let registry = crate::math_formula::FormulaFontRegistry::load_default().map_err(|err| {
            LayoutError::BadFormula {
                node: node.key.clone(),
                reason: err.to_string(),
            }
        })?;
        #[cfg(target_arch = "wasm32")]
        let registry = _opts.formula_fonts;
        let style = formula_style_from_node(node, values, at)?;
        let fragment = crate::math_formula::emit_formula(
            latex,
            *display,
            style,
            registry,
            &crate::math_formula::AdmitPolicy::default(),
        )
        .map_err(|err| LayoutError::BadFormula {
            node: node.key.clone(),
            reason: err.to_string(),
        })?;
        out.insert(node.key.clone(), fragment);
    }
    Ok(out)
}

fn formula_style_from_node(
    node: &valle_motion::SceneNode,
    values: &[MotionValue],
    at: usize,
) -> Result<crate::math_formula::FormulaStyle, LayoutError> {
    let mut style = crate::math_formula::FormulaStyle::default();
    for binding in &node.styles {
        match binding.property.as_str() {
            "fontSize" | "font-size" => {
                let resolved = match &binding.value {
                    StyleValue::Static { value } => value,
                    StyleValue::Expr { expr } => value(values, *expr, at)?,
                };
                if let MotionValue::Number(n) = resolved {
                    style.font_size = *n;
                }
            }
            "color" => {
                if let StyleValue::Static {
                    value: MotionValue::Color(c),
                } = &binding.value
                {
                    style.color = c.to_srgb8();
                }
            }
            _ => {}
        }
    }
    Ok(style)
}

#[expect(
    clippy::too_many_arguments,
    reason = "Keep the layout pass inputs and its independently borrowed caches explicit."
)]
fn node_of(
    prepared: &PreparedScene,
    at: usize,
    values: &[MotionValue],
    cache: Option<&crate::StyleCache>,
    camera: Option<&CameraWrappers>,
    formulas: &HashMap<String, crate::math_formula::FormulaFragment>,
    inactive_nodes: &[bool],
    layout_instances: &LayoutInstanceRows,
) -> Result<Node, LayoutError> {
    let mut projected = projected_nodes_of(
        prepared,
        at,
        values,
        cache,
        camera,
        formulas,
        inactive_nodes,
        layout_instances,
    )?;
    if projected.len() != 1 {
        return Err(LayoutError::BadNode { at });
    }
    Ok(projected.remove(0))
}

fn projected_instance_node(
    row: &LayoutInstanceRow,
    cache: Option<&crate::StyleCache>,
) -> Result<Node, LayoutError> {
    let children = row
        .children
        .iter()
        .map(|child| projected_instance_node(child, cache))
        .collect::<Result<Vec<_>, _>>()?;
    let mut node = if let Some(text) = &row.text {
        Node::text(text.clone())
    } else {
        Node::container(children)
    };
    let preset = if row.text.is_some() {
        "box-sizing: border-box; margin: 0; padding: 0; border-width: 0; border-style: solid;"
    } else {
        "box-sizing: border-box; margin: 0; padding: 0; border-width: 0; border-style: solid; display: block;"
    };
    node =
        node.with_preset(
            parse_style(cache, preset).map_err(|reason| LayoutError::BadStyle {
                node: row.key.clone(),
                declarations: preset.into(),
                reason,
            })?,
        );
    node = node.with_class_name(row.class_name.clone());
    if !row.declarations.is_empty() {
        node = node.with_style(row.declarations.resolve(cache).map_err(|reason| {
            LayoutError::BadStyle {
                node: row.key.clone(),
                declarations: row.declarations.source(),
                reason,
            }
        })?);
    }
    Ok(node)
}

/// Project one Artifact node into zero or one Takumi boxes. GlassField is a semantic material
/// scope, not a CSS box, so it contributes its projected children directly to the parent.
#[expect(
    clippy::too_many_arguments,
    reason = "Keep the layout pass inputs and its independently borrowed caches explicit."
)]
fn projected_nodes_of(
    prepared: &PreparedScene,
    at: usize,
    values: &[MotionValue],
    cache: Option<&crate::StyleCache>,
    camera: Option<&CameraWrappers>,
    formulas: &HashMap<String, crate::math_formula::FormulaFragment>,
    inactive_nodes: &[bool],
    layout_instances: &LayoutInstanceRows,
) -> Result<Vec<Node>, LayoutError> {
    if inactive_nodes.get(at).copied().unwrap_or(false) {
        return Ok(Vec::new());
    }
    let artifact = prepared.artifact();
    let template = artifact.nodes.get(at).ok_or(LayoutError::BadNode { at })?;
    if let NodeKind::InstanceLayout { group } = template.kind {
        let rows = layout_instances
            .get(&group)
            .ok_or(LayoutError::BadNode { at })?;
        return rows
            .iter()
            .map(|row| projected_instance_node(row, cache))
            .collect();
    }
    let child_range = template.children.start as usize..template.children.end as usize;
    let child_ids = artifact
        .node_children
        .get(child_range)
        .ok_or(LayoutError::BadNode { at })?;
    let child_groups = child_ids
        .iter()
        .map(|child| {
            projected_nodes_of(
                prepared,
                child.0 as usize,
                values,
                cache,
                camera,
                formulas,
                inactive_nodes,
                layout_instances,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;

    // Split coordinate spaces only at the root, where admission requires Screen nodes. Keeping
    // Screen outside the camera avoids inverse-transform rounding and HUD jitter.
    let children = match camera {
        Some(camera) if at == artifact.root.0 as usize => {
            let mut world = Vec::new();
            let mut screen = Vec::new();
            for (nodes, id) in child_groups.into_iter().zip(child_ids) {
                let is_screen = artifact
                    .nodes
                    .get(id.0 as usize)
                    .is_some_and(|node| node.space == Some(CoordinateSpace::Screen));
                if is_screen {
                    screen.extend(nodes)
                } else {
                    world.extend(nodes)
                }
            }
            let mut out = Vec::with_capacity(screen.len() + 1);
            out.push(camera.wrap(world));
            out.extend(screen);
            out
        }
        _ => child_groups.into_iter().flatten().collect(),
    };

    if matches!(
        template.kind,
        NodeKind::GlassField(_) | NodeKind::TimeScope { .. }
    ) {
        return Ok(children);
    }

    let mut node = match &template.kind {
        NodeKind::Group
        | NodeKind::Box
        | NodeKind::Shutter { .. }
        | NodeKind::Echo { .. }
        | NodeKind::Clip { .. }
        | NodeKind::Transition { .. }
        | NodeKind::Mask { .. }
        | NodeKind::ShaderLayer { .. } => Node::container(children),
        NodeKind::Glass(_) => Node::container(children),
        NodeKind::GlassField(_) => unreachable!("GlassField is projected without a layout box"),
        NodeKind::TimeScope { .. } => unreachable!("TimeScope is transparent to layout"),
        NodeKind::InstanceLayout { .. } => {
            unreachable!("InstanceLayout expands before node construction")
        }
        NodeKind::Text { text, .. } => Node::text(text_value(text, values, at)?),
        NodeKind::Path { .. } | NodeKind::GeometryBatch { .. } | NodeKind::InstanceBatch { .. } => {
            Node::container([])
        }
        NodeKind::Image { source } => Node::image(source.clone()),
        // Scene3D is a replaced element just like image/video. The opaque generated URL is
        // resolved by the 3D host bridge; layout only owns its destination rectangle.
        NodeKind::Scene3D { .. } => {
            if children.is_empty() {
                Node::image(format!("generated://scene3d/{}", template.key))
            } else {
                Node::container(children)
            }
        }
        // Replaced leaf, same channel as Image/Scene3D. Emit resolves the
        // generated URL; Takumi only owns the destination rectangle.
        NodeKind::MathFormula { .. } => {
            let fragment = formulas
                .get(&template.key)
                .ok_or_else(|| LayoutError::BadFormula {
                    node: template.key.clone(),
                    reason: "formula fragment missing after prepare".into(),
                })?;
            Node::image((
                format!("generated://formula/{}", template.key),
                fragment.width.max(0.0) as f32,
                (fragment.height + fragment.depth).max(0.0) as f32,
            ))
        }
        // Video shares image layout and emission; the video side table supplies the frame time.
        NodeKind::Video { source, .. } => Node::image(source.clone()),
    };
    let mut preset = String::from(
        "box-sizing: border-box; margin: 0; padding: 0; border-width: 0; border-style: solid;",
    );
    if matches!(
        template.kind,
        NodeKind::Group
            | NodeKind::Box
            | NodeKind::Shutter { .. }
            | NodeKind::Echo { .. }
            | NodeKind::Clip { .. }
            | NodeKind::Transition { .. }
            | NodeKind::Mask { .. }
            | NodeKind::Path { .. }
            | NodeKind::GeometryBatch { .. }
            | NodeKind::InstanceBatch { .. }
            | NodeKind::InstanceLayout { .. }
            | NodeKind::ShaderLayer { .. }
            | NodeKind::Glass(_)
            | NodeKind::Scene3D { .. }
    ) {
        preset.push_str("display: block;");
    }
    if matches!(
        template.kind,
        NodeKind::Image { .. } | NodeKind::Video { .. }
    ) {
        preset.push_str(
            if template
                .styles
                .iter()
                .any(|style| style.property == "motion-inline-image")
            {
                "display: inline-block;"
            } else {
                "display: block;"
            },
        );
    }
    // Clip and Mask must create stacking contexts so their begin/end groups enclose descendants.
    //

    //
    // Without a stacking context, the emitted clip or mask closes before its children and has no
    // effect.
    //
    // `isolation: isolate` creates the required stacking context without altering opacity or color.
    if matches!(
        template.kind,
        NodeKind::Mask { .. }
            | NodeKind::Transition { .. }
            | NodeKind::Shutter { .. }
            | NodeKind::Echo { .. }
    ) {
        preset.push_str("position: relative;");
    }
    if matches!(
        template.kind,
        NodeKind::Shutter { .. } | NodeKind::Echo { .. }
    ) {
        preset.push_str("width: 100%; height: 100%;");
    }
    if matches!(template.kind, NodeKind::Scene3D { .. })
        && template.children.start != template.children.end
    {
        preset.push_str("position: relative; isolation: isolate;");
    }
    let is_mask_source = prepared.positioned_children[at] & 1 != 0;
    let is_transition_child = prepared.positioned_children[at] & 2 != 0;
    if is_mask_source || is_transition_child {
        // Source placement is in the mask's coordinate system and never consumes content flow.
        preset.push_str("position: absolute; left: 0; top: 0; width: 100%; height: 100%;");
    }
    if matches!(
        template.kind,
        NodeKind::Clip { .. }
            | NodeKind::Transition { .. }
            | NodeKind::Shutter { .. }
            | NodeKind::Echo { .. }
            | NodeKind::Mask { .. }
            | NodeKind::ShaderLayer { .. }
            | NodeKind::Glass(_)
    ) || is_mask_source
        || is_transition_child
    {
        preset.push_str("isolation: isolate;");
    }
    // Advanced filters require a stacking context so BeginFilter encloses the full subtree.
    if template.styles.iter().any(|style| {
        style.property.starts_with("motion-displacement-")
            || style.property.starts_with("motion-velocity-blur-")
            || style.property == "motion-chromatic-aberration-offset"
            || style.property.starts_with("motion-glow-")
            || style.property.starts_with("motion-bloom-")
            || style.property.starts_with("motion-radial-blur-")
            || style.property.starts_with("motion-film-grain-")
            || style.property.starts_with("motion-lens-distortion-")
            || style.property.starts_with("motion-transform-3d-")
            || matches!(
                style.property.as_str(),
                "rotate-x"
                    | "rotate-y"
                    | "perspective"
                    | "transform-style"
                    | "backface-visibility"
                    | "paper-grain"
                    | "contact-shadow"
            )
    }) {
        preset.push_str("isolation: isolate;");
    }
    // Text along a path defaults to no automatic wrapping. An explicit author white-space style
    // can preserve newlines or allow wrapping; emission gives each shaped line its own offset path.
    if let NodeKind::MathFormula { .. } = &template.kind {
        let fragment = formulas
            .get(&template.key)
            .ok_or_else(|| LayoutError::BadFormula {
                node: template.key.clone(),
                reason: "formula fragment missing after prepare".into(),
            })?;
        let total_h = fragment.height + fragment.depth;
        preset.push_str(&format!(
            "display: block; isolation: isolate; width: {}px; height: {}px;",
            fragment.width.max(0.0),
            total_h.max(0.0)
        ));
    }
    if matches!(template.kind, NodeKind::Text { path: Some(_), .. }) {
        preset.push_str("white-space: nowrap;");
    }

    // Takumi replaces the entire preset layer. Apply the collected defaults once so
    // isolation and other semantic properties cannot erase the node's display mode.
    if !preset.is_empty() {
        node = node.with_preset(parse_style(cache, &preset).map_err(|reason| {
            LayoutError::BadStyle {
                node: template.key.clone(),
                declarations: preset.clone(),
                reason,
            }
        })?);
    }

    let mut active_classes = Vec::with_capacity(template.class_names.len());
    for (class, layout_class) in template
        .class_names
        .iter()
        .zip(&prepared.layout_classes[at])
    {
        let active = match template.class_conditions.get(class) {
            Some(expr) => match value(values, *expr, at)? {
                MotionValue::Bool(active) => *active,
                _ => {
                    return Err(LayoutError::Eval(EvalError::TypeMismatch {
                        at,
                        op: "className condition",
                    }));
                }
            },
            None => true,
        };
        if active {
            active_classes.push(layout_class.as_str());
        }
    }
    node = node.with_class_name(active_classes.join(" "));

    if let Some(style) = &prepared.static_styles[at] {
        return Ok(vec![node.with_style(style.clone())]);
    }

    let visible = match template.visibility {
        Some(expr) => match value(values, expr, at)? {
            MotionValue::Bool(value) => *value,
            _ => {
                return Err(LayoutError::Eval(EvalError::TypeMismatch {
                    at,
                    op: "visibility",
                }));
            }
        },
        None => true,
    };
    let mut declarations = declarations(template, values, visible, at)?;
    if is_transition_child || matches!(template.kind, NodeKind::Transition { .. }) {
        // These are semantic subtree boundaries even when the author requests isolation:auto.
        declarations.push_css("isolation: isolate !important".into());
    }
    if let NodeKind::MathFormula { .. } = &template.kind
        && let Some(fragment) = formulas.get(&template.key)
    {
        let total_h = (fragment.height + fragment.depth).max(0.0);
        declarations.push_css(format!("width: {}px", fragment.width.max(0.0)));
        declarations.push_css(format!("height: {}px", total_h));
    }
    if !declarations.is_empty() {
        node = node.with_style(declarations.resolve(cache).map_err(|reason| {
            LayoutError::BadStyle {
                node: template.key.clone(),
                declarations: declarations.source(),
                reason,
            }
        })?);
    }
    Ok(vec![node])
}

fn text_value(text: &TextValue, values: &[MotionValue], at: usize) -> Result<String, LayoutError> {
    match text {
        TextValue::Static { value } => Ok(value.clone()),
        TextValue::Expr { expr } => match value(values, *expr, at)? {
            MotionValue::Str(value) | MotionValue::Enum(value) => Ok(value.clone()),
            _ => Err(LayoutError::Eval(EvalError::TypeMismatch {
                at,
                op: "text",
            })),
        },
    }
}

fn declarations(
    node: &valle_motion::SceneNode,
    values: &[MotionValue],
    visible: bool,
    at: usize,
) -> Result<FrameDeclarations, LayoutError> {
    let mut resolved = FrameDeclarations::default();
    let mut declarations = String::new();
    let motion_anchor = node.styles.iter().find_map(|style| {
        if style.property != "motion-path-anchor" {
            return None;
        }
        match &style.value {
            StyleValue::Static {
                value: MotionValue::Enum(value),
            } => Some(value.as_str()),
            _ => None,
        }
    });
    let angle_offset = node
        .styles
        .iter()
        .find_map(|style| {
            if style.property != "motion-path-angle-offset" {
                return None;
            }
            match &style.value {
                StyleValue::Static {
                    value: MotionValue::Number(value),
                } => Some(*value),
                _ => None,
            }
        })
        .unwrap_or(0.0);
    if let Some(anchor) = motion_anchor {
        declarations.push_str("transform-origin: ");
        declarations.push_str(if anchor == "center" {
            "50% 50%"
        } else {
            "0px 0px"
        });
    }
    resolved.push_css(std::mem::take(&mut declarations));
    for style in &node.styles {
        // Author `--*` is rejected at admission, and Motion consumes its own bindings before CSS
        // layout; skip both here so they never enter the cascade.
        if style.property.starts_with("--") || crate::style::is_motion_property(&style.property) {
            continue;
        }
        let value = match &style.value {
            StyleValue::Static { value } => value,
            StyleValue::Expr { expr } => value(values, *expr, at)?,
        };
        if style.property == "opacity"
            && let MotionValue::Number(opacity) = value
            && (!opacity.is_finite() || !(0.0..=1.0).contains(opacity))
        {
            let reason = "opacity must be finite and in [0, 1]";
            return Err(LayoutError::BadStyle {
                node: node.key.clone(),
                declarations: format!("opacity: {opacity}"),
                reason: match style.value {
                    StyleValue::Expr { expr } => format!("expression {}: {reason}", expr.0),
                    _ => reason.into(),
                },
            });
        }
        if let Some(declaration) = super::bridge::typed_paint_declaration(&style.property, value) {
            resolved.push_typed(declaration);
            continue;
        }
        if matches!(style.property.as_str(), "filter" | "backdrop-filter") {
            // Preserve the numeric parameter diagnostic before the generic CSS
            // parser wraps the entire declaration block into one error.
            if let Err(reason) = crate::style::advanced_filter::stages(&css_token(value)) {
                let reason = match style.value {
                    StyleValue::Expr { expr } => format!("expression {}: {reason}", expr.0),
                    _ => reason,
                };
                return Err(LayoutError::BadStyle {
                    node: node.key.clone(),
                    declarations: String::new(),
                    reason,
                });
            }
        }
        if matches!(style.value, StyleValue::Expr { .. })
            && matches!(style.property.as_str(), "background" | "background-image")
            && !gradient_background_source(&style.property, &css_token(value))
        {
            return Err(LayoutError::UnsupportedSurface {
                node: node.key.clone(),
                surface: style.property.clone(),
            });
        }
        if !declarations.is_empty() {
            declarations.push_str("; ");
        }
        declarations.push_str(&style.property);
        declarations.push_str(": ");
        if style.property == "translate"
            && motion_anchor == Some("center")
            && let MotionValue::Length2(value) = value
        {
            declarations.push_str("calc(");
            declarations.push_str(&css_token(&MotionValue::Length(value.x)));
            declarations.push_str(" - 50%) calc(");
            declarations.push_str(&css_token(&MotionValue::Length(value.y)));
            declarations.push_str(" - 50%)");
        } else if style.property == "rotate" && angle_offset != 0.0 {
            declarations.push_str("calc(");
            declarations.push_str(&css_token(value));
            declarations.push_str(" + ");
            declarations.push_str(&angle_offset.to_string());
            declarations.push_str("deg)");
        } else if style.property == "display"
            && matches!(node.kind, NodeKind::Image { .. } | NodeKind::Video { .. })
            && css_token(value) == "inline"
        {
            // Atomic inline layout honors the authored media box without a pixel decoder.
            declarations.push_str("inline-block");
        } else {
            declarations.push_str(&crate::style::value_token(&style.property, value));
        }
        resolved.push_css(std::mem::take(&mut declarations));
    }
    if !visible {
        if !declarations.is_empty() {
            declarations.push_str("; ");
        }
        declarations.push_str("visibility: hidden");
    }
    resolved.push_css(declarations);
    Ok(resolved)
}

fn value(values: &[MotionValue], expr: ExprId, at: usize) -> Result<&MotionValue, LayoutError> {
    values.get(expr.0 as usize).ok_or({
        LayoutError::Eval(EvalError::BadExpr {
            at,
            referenced: expr.0 as usize,
        })
    })
}

/// Split source text into node-local byte ranges before shaping. Unit counts depend only on the
/// source, and emission consumes the precomputed per-unit values by index.
fn unit_boundaries(source: &str, split: TextSplit) -> Vec<(u32, u32)> {
    match split {
        TextSplit::Char => source
            .char_indices()
            .map(|(start, ch)| (start as u32, (start + ch.len_utf8()) as u32))
            .collect(),
        // Whitespace is not a word unit; including it would distort stagger spacing.
        TextSplit::Word => {
            let mut units = Vec::new();
            let mut open: Option<usize> = None;
            for (at, ch) in source.char_indices() {
                match (ch.is_whitespace(), open) {
                    (false, None) => open = Some(at),
                    (true, Some(start)) => {
                        units.push((start as u32, at as u32));
                        open = None;
                    }
                    _ => {}
                }
            }
            if let Some(start) = open {
                units.push((start as u32, source.len() as u32));
            }
            units
        }
        // Split at source newlines, independent of layout wrapping. Keep empty lines as units so
        // subsequent indices remain stable.
        TextSplit::Line => {
            let mut units = Vec::new();
            let mut start = 0usize;
            for (at, ch) in source.char_indices() {
                if ch == '\n' {
                    units.push((start as u32, at as u32));
                    start = at + 1;
                }
            }
            units.push((start as u32, source.len() as u32));
            units
        }
    }
}

/// The JavaScript string index, in UTF-16 code units, of every character boundary of `text`,
/// by byte offset. A unit's `start` and `end` are these indexes, as `text.slice(start, end)`.
fn utf16_offsets(text: &str) -> Vec<u32> {
    let mut offsets = vec![0; text.len() + 1];
    let mut units = 0;
    for (byte, ch) in text.char_indices() {
        offsets[byte] = units;
        units += ch.len_utf16() as u32;
    }
    offsets[text.len()] = units;
    offsets
}

/// Takumi stores device extents; expressions and media queries use CSS pixels. Native/Wasm
/// hosts lay out at DPR 1 and apply delivery scaling later. Low-level callers may supply a
/// different DPR, so convert here too. Missing dimensions must not produce guessed values.
fn eval_viewport(opts: &LayoutOptions<'_>) -> Option<(f64, f64)> {
    let width = opts.viewport.size.width?;
    let height = opts.viewport.size.height?;
    let dpr = f64::from(opts.viewport.device_pixel_ratio);
    Some((f64::from(width) / dpr, f64::from(height) / dpr))
}

#[expect(
    clippy::too_many_arguments,
    reason = "Keep the layout pass inputs and its independently borrowed caches explicit."
)]
fn resolve_units(
    artifact: &SceneArtifact,
    eval_plan: &EvalPlan,
    values: &[MotionValue],
    ctx: &valle_motion::MotionContext,
    props: &ResolvedProps,
    boxes: &BTreeMap<String, valle_draw::Rect>,
    projected: &BTreeMap<(String, String), valle_draw::Point>,
    viewport: Option<(f64, f64)>,
) -> Result<HashMap<String, Vec<ResolvedUnit>>, LayoutError> {
    struct RichUnitGroup {
        boundaries: Vec<(u32, u32)>,
        run_ranges: HashMap<usize, (u32, u32)>,
        utf16: Vec<u32>,
    }

    // A rich Text is an inline Group of styled Text runs. Join their authored source in child
    // order before splitting, so a unit's index, count, start and end address the whole Text and
    // a word or line crossing a Span boundary remains one animation unit.
    let mut rich_groups = HashMap::<String, RichUnitGroup>::new();
    for group in &artifact.nodes {
        if !matches!(&group.kind, NodeKind::Group) {
            continue;
        }
        let children =
            &artifact.node_children[group.children.start as usize..group.children.end as usize];
        let split = children.iter().find_map(|child| {
            let NodeKind::Text {
                per_unit: Some(binding),
                ..
            } = &artifact.nodes[child.0 as usize].kind
            else {
                return None;
            };
            (binding.group_key.as_deref() == Some(group.key.as_str())).then_some(binding.split)
        });
        let Some(split) = split else {
            continue;
        };
        let mut source = String::new();
        let mut run_ranges = HashMap::new();
        for child in children {
            let at = child.0 as usize;
            match &artifact.nodes[at].kind {
                NodeKind::Text { text, .. } => {
                    let start = source.len() as u32;
                    source.push_str(&text_value(text, values, at)?);
                    run_ranges.insert(at, (start, source.len() as u32));
                }
                // An inline image separates words, but has no text character or layout line.
                NodeKind::Image { .. } if split == TextSplit::Word => source.push(' '),
                _ => {}
            }
        }
        rich_groups.insert(
            group.key.clone(),
            RichUnitGroup {
                boundaries: unit_boundaries(&source, split),
                run_ranges,
                utf16: utf16_offsets(&source),
            },
        );
    }

    let mut out = HashMap::new();
    for (at, node) in artifact.nodes.iter().enumerate() {
        let NodeKind::Text {
            text,
            per_unit: Some(per_unit),
            ..
        } = &node.kind
        else {
            continue;
        };
        let mut boundaries = Vec::new();
        if let Some(group_key) = &per_unit.group_key {
            let group = rich_groups
                .get(group_key)
                .ok_or(LayoutError::BadNode { at })?;
            let (run_start, run_end) = group
                .run_ranges
                .get(&at)
                .copied()
                .ok_or(LayoutError::BadNode { at })?;
            let count = group.boundaries.len() as u32;
            let first = group
                .boundaries
                .partition_point(|&(_, end)| end <= run_start);
            for (index, &(start, end)) in group.boundaries.iter().enumerate().skip(first) {
                if start >= run_end {
                    break;
                }
                let local_start = start.max(run_start);
                let local_end = end.min(run_end);
                if local_start < local_end {
                    boundaries.push((
                        local_start - run_start,
                        local_end - run_start,
                        valle_motion::UnitContext {
                            index: index as u32,
                            count,
                            start: group.utf16[start as usize],
                            end: group.utf16[end as usize],
                        },
                    ));
                }
            }
        } else {
            let source = text_value(text, values, at)?;
            let units = unit_boundaries(&source, per_unit.split);
            let utf16 = utf16_offsets(&source);
            let count = units.len() as u32;
            boundaries.extend(units.into_iter().enumerate().map(|(index, (start, end))| {
                (
                    start,
                    end,
                    valle_motion::UnitContext {
                        index: index as u32,
                        count,
                        start: utf16[start as usize],
                        end: utf16[end as usize],
                    },
                )
            }));
        }
        let style = &per_unit.style;
        let mut resolved = Vec::with_capacity(boundaries.len());
        for (start, end, unit) in boundaries {
            let values = eval_units_planned(
                artifact,
                eval_plan,
                values,
                EvalInputs {
                    ctx,
                    props,
                    unit: None,
                    viewport,
                },
                unit,
                boxes,
                projected,
            )?;
            let blur = style
                .blur
                .as_ref()
                .map(|binding| number_value(binding, &values, at, "unit blur"))
                .transpose()?;
            if blur.is_some_and(|value| !(0.0..=128.0).contains(&value)) {
                return Err(LayoutError::BadStyle {
                    node: node.key.clone(),
                    declarations: "perUnit.blur".into(),
                    reason: "perUnit blur sigma must be within 0..=128 CSS pixels".into(),
                });
            }
            resolved.push(ResolvedUnit {
                start,
                end,
                opacity: style
                    .opacity
                    .as_ref()
                    .map(|binding| number_value(binding, &values, at, "unit opacity"))
                    .transpose()?,
                translate: style
                    .translate
                    .as_ref()
                    .map(|binding| point_value(binding, &values, at, "unit translate"))
                    .transpose()?,
                scale: style
                    .scale
                    .as_ref()
                    .map(|binding| point_value(binding, &values, at, "unit scale"))
                    .transpose()?,
                rotate: style
                    .rotate
                    .as_ref()
                    .map(|binding| number_value(binding, &values, at, "unit rotate"))
                    .transpose()?,
                color: style
                    .color
                    .as_ref()
                    .map(|binding| color_value(binding, &values, at))
                    .transpose()?,
                blur,
            });
        }
        out.insert(node.key.clone(), resolved);
    }
    Ok(out)
}

/// Pair the Artifact and render trees to map paths to Scene keys. Anonymous text belongs to its
/// owning element; anonymous blocks are transparent and do not advance the Artifact cursor. Stop
/// matching when the cursor is exhausted rather than assign an incorrect source identity.
fn walk_render_keys(
    artifact: &SceneArtifact,
    node_id: NodeId,
    node: &RenderNode,
    path: &mut Vec<usize>,
    out: &mut HashMap<Vec<usize>, String>,
    inactive_nodes: &[bool],
) {
    let at = node_id.0 as usize;
    let Some(scene) = artifact.nodes.get(at) else {
        return;
    };
    out.insert(path.clone(), scene.key.clone());
    let Some(children) = node.children.as_deref() else {
        return;
    };
    let Some(child_ids) = projected_child_ids(artifact, node_id, inactive_nodes) else {
        return;
    };

    // Skip the camera wrappers, which have no Artifact counterparts. Admission guarantees direct
    // Screen children follow all World children.
    //
    // Apply the wrapper traversal rule here as well to preserve source identities for World text.
    if at == artifact.root.0 as usize && artifact.camera.is_some() {
        let mut world_ids = Vec::new();
        let mut screen_ids = Vec::new();
        for projected in &child_ids {
            let id = projected.id;
            let is_screen = artifact
                .nodes
                .get(id.0 as usize)
                .is_some_and(|node| node.space == Some(CoordinateSpace::Screen));
            if is_screen {
                screen_ids.push(projected.clone());
            } else {
                world_ids.push(projected.clone());
            }
        }
        // The first child is the outer wrapper, containing the inner wrapper and World content.
        // Associate both wrapper paths with the root key.
        if let Some(outer) = children.first() {
            path.push(0);
            out.insert(path.clone(), scene.key.clone());
            if let Some(inner) = outer.children.as_deref().and_then(|kids| kids.first()) {
                path.push(0);
                out.insert(path.clone(), scene.key.clone());
                if let Some(world) = inner.children.as_deref() {
                    let mut cursor = 0;
                    match_render_children(
                        artifact,
                        world,
                        &world_ids,
                        &mut cursor,
                        &scene.key,
                        path,
                        out,
                        inactive_nodes,
                    );
                }
                path.pop();
            }
            path.pop();
        }
        // Screen children follow the camera wrapper, starting at render index 1.
        for (index, child) in children.iter().enumerate().skip(1) {
            let Some(projected) = screen_ids.get(index - 1) else {
                break;
            };
            path.push(index);
            if let Some(row) = projected.row {
                if let Some(group) = artifact.instance_groups.get(projected.group as usize) {
                    walk_instance_render_keys(
                        group,
                        row,
                        &group.template,
                        &group.template_children,
                        child,
                        path,
                        out,
                    );
                }
            } else {
                walk_render_keys(artifact, projected.id, child, path, out, inactive_nodes);
            }
            path.pop();
        }
        return;
    }

    let mut cursor = 0;
    match_render_children(
        artifact,
        children,
        &child_ids,
        &mut cursor,
        &scene.key,
        path,
        out,
        inactive_nodes,
    );
}

/// Artifact children that actually own Takumi boxes. Transparent scopes are flattened exactly
/// as [`projected_nodes_of`] does.
#[derive(Clone)]
struct ProjectedChild {
    id: NodeId,
    group: u32,
    row: Option<usize>,
}

fn projected_child_ids(
    artifact: &SceneArtifact,
    owner: NodeId,
    inactive_nodes: &[bool],
) -> Option<Vec<ProjectedChild>> {
    fn append(
        artifact: &SceneArtifact,
        id: NodeId,
        inactive_nodes: &[bool],
        out: &mut Vec<ProjectedChild>,
    ) -> Option<()> {
        if inactive_nodes.get(id.0 as usize).copied().unwrap_or(false) {
            return Some(());
        }
        let node = artifact.nodes.get(id.0 as usize)?;
        if matches!(
            node.kind,
            NodeKind::GlassField(_) | NodeKind::TimeScope { .. }
        ) {
            let range = node.children.start as usize..node.children.end as usize;
            for child in artifact.node_children.get(range)? {
                append(artifact, *child, inactive_nodes, out)?;
            }
        } else if let NodeKind::InstanceLayout { group } = node.kind {
            let rows = artifact.instance_groups.get(group as usize)?.rows();
            out.extend((0..rows).map(|row| ProjectedChild {
                id,
                group,
                row: Some(row),
            }));
        } else {
            out.push(ProjectedChild {
                id,
                group: 0,
                row: None,
            });
        }
        Some(())
    }

    let node = artifact.nodes.get(owner.0 as usize)?;
    let range = node.children.start as usize..node.children.end as usize;
    let mut out = Vec::new();
    for child in artifact.node_children.get(range)? {
        append(artifact, *child, inactive_nodes, &mut out)?;
    }
    Some(out)
}

/// Match render children to `child_ids`, handling anonymous boxes as in [`walk_render_keys`].
#[expect(
    clippy::too_many_arguments,
    reason = "Keep the layout pass inputs and its independently borrowed caches explicit."
)]
fn match_render_children(
    artifact: &SceneArtifact,
    children: &[RenderNode],
    child_ids: &[ProjectedChild],
    cursor: &mut usize,
    owner: &str,
    path: &mut Vec<usize>,
    out: &mut HashMap<Vec<usize>, String>,
    inactive_nodes: &[bool],
) {
    for (at, child) in children.iter().enumerate() {
        path.push(at);
        if child.anonymous_text_content.is_some() {
            walk_owned_subtree_keys(child, path, owner, out);
        } else if child.node.is_none() {
            if let Some(grandchildren) = child.children.as_deref() {
                match_render_children(
                    artifact,
                    grandchildren,
                    child_ids,
                    cursor,
                    owner,
                    path,
                    out,
                    inactive_nodes,
                );
            }
        } else if let Some(projected) = child_ids.get(*cursor) {
            if let Some(row) = projected.row {
                if let Some(group) = artifact.instance_groups.get(projected.group as usize) {
                    walk_instance_render_keys(
                        group,
                        row,
                        &group.template,
                        &group.template_children,
                        child,
                        path,
                        out,
                    );
                }
            } else {
                walk_render_keys(artifact, projected.id, child, path, out, inactive_nodes);
            }
            *cursor += 1;
        } else {
            path.pop();
            return;
        }
        path.pop();
    }
}

fn walk_instance_render_keys(
    group: &InstanceGroup,
    row: usize,
    template: &valle_motion::SceneNode,
    template_children: &[InstanceTemplateNode],
    render: &RenderNode,
    path: &mut Vec<usize>,
    out: &mut HashMap<Vec<usize>, String>,
) {
    let Some(key) = group.key_for_node(row, &template.key) else {
        return;
    };
    out.insert(path.clone(), key.clone());
    let Some(children) = render.children.as_deref() else {
        return;
    };
    let mut cursor = 0;
    match_instance_render_children(
        group,
        row,
        children,
        template_children,
        &mut cursor,
        &key,
        path,
        out,
    );
}

#[expect(
    clippy::too_many_arguments,
    reason = "Keep the layout pass inputs and its independently borrowed caches explicit."
)]
fn match_instance_render_children(
    group: &InstanceGroup,
    row: usize,
    children: &[RenderNode],
    templates: &[InstanceTemplateNode],
    cursor: &mut usize,
    owner: &str,
    path: &mut Vec<usize>,
    out: &mut HashMap<Vec<usize>, String>,
) {
    for (at, child) in children.iter().enumerate() {
        path.push(at);
        if child.anonymous_text_content.is_some() {
            walk_owned_subtree_keys(child, path, owner, out);
        } else if child.node.is_none() {
            if let Some(grandchildren) = child.children.as_deref() {
                match_instance_render_children(
                    group,
                    row,
                    grandchildren,
                    templates,
                    cursor,
                    owner,
                    path,
                    out,
                );
            }
        } else if let Some(template) = templates.get(*cursor) {
            walk_instance_render_keys(
                group,
                row,
                &template.node,
                &template.children,
                child,
                path,
                out,
            );
            *cursor += 1;
        } else {
            path.pop();
            return;
        }
        path.pop();
    }
}

/// Associate a render subtree without an Artifact counterpart with its owning `key`.
fn walk_owned_subtree_keys(
    node: &RenderNode,
    path: &mut Vec<usize>,
    key: &str,
    out: &mut HashMap<Vec<usize>, String>,
) {
    out.insert(path.clone(), key.to_owned());
    let Some(children) = node.children.as_deref() else {
        return;
    };
    for (at, child) in children.iter().enumerate() {
        path.push(at);
        walk_owned_subtree_keys(child, path, key, out);
        path.pop();
    }
}

/// Follow the actual render/layout trees, using the existing source-key mapping.
/// Anonymous inline wrappers and camera wrappers have geometry but are not authored
/// nodes. Pairing artifact child indices directly with them assigns the wrong bounds.
fn walk_pairs(
    artifact: &SceneArtifact,
    root: &RenderNode,
    layout: &LayoutResults,
    inactive_nodes: &[bool],
    callback: &mut impl FnMut(Option<usize>, &str, takumi_core::geometry::NodeId, (f32, f32)),
) -> Result<(), LayoutError> {
    let mut render_keys = HashMap::new();
    walk_render_keys(
        artifact,
        artifact.root,
        root,
        &mut Vec::new(),
        &mut render_keys,
        inactive_nodes,
    );
    let indices: HashMap<_, _> = artifact
        .nodes
        .iter()
        .enumerate()
        .map(|(i, node)| (node.key.as_str(), i))
        .collect();
    let root_key = &artifact.nodes[artifact.root.0 as usize].key;
    let mut pending = vec![(
        root,
        takumi_core::geometry::NodeId::ROOT,
        Vec::new(),
        (0.0, 0.0),
    )];
    while let Some((node, id, path, origin)) = pending.pop() {
        let here = origin_of(layout, id, origin);
        if node.source_order().is_some()
            && let Some(key) = render_keys.get(&path)
            && (path.is_empty() || key != root_key)
        {
            callback(indices.get(key.as_str()).copied(), key, id, here);
        }
        if let (Some(children), Ok(layout_children)) = (&node.children, layout.box_children(id)) {
            for item in layout_children.iter().rev() {
                if let Some(child) = children.get(item.render_index) {
                    let mut child_path = path.clone();
                    child_path.push(item.render_index);
                    pending.push((child, item.node_id, child_path, here));
                }
            }
        }
    }
    Ok(())
}

fn origin_of(
    layout: &LayoutResults,
    id: takumi_core::geometry::NodeId,
    parent: (f32, f32),
) -> (f32, f32) {
    match layout.layout(id) {
        Ok(computed) => (
            parent.0 + computed.location.x,
            parent.1 + computed.location.y,
        ),
        Err(_) => parent,
    }
}

/// Atomic inline boxes live in the inline formatter rather than LayoutResults' box
/// children. Query their actual placement using the same formatter as emission.
fn inline_boxes(
    artifact: &SceneArtifact,
    root: &RenderNode,
    layout: &LayoutResults,
    inactive_nodes: &[bool],
    boxes: &mut BTreeMap<String, valle_draw::Rect>,
) -> Result<(), LayoutError> {
    use takumi_core::layout::inline::{
        InlineItem, InlineLayoutMode, InlineLayoutRequest, ProcessedInlineSpan,
        collect_inline_items, create_inline_layout, resolve_inline_runs,
    };
    let mut render_keys = HashMap::new();
    walk_render_keys(
        artifact,
        artifact.root,
        root,
        &mut Vec::new(),
        &mut render_keys,
        inactive_nodes,
    );
    let mut source_keys = HashMap::new();
    for (path, key) in render_keys {
        if let Some(order) = root.node_at_path(&path).and_then(RenderNode::source_order) {
            source_keys.insert(order, key);
        }
    }
    let mut pending = vec![(root, takumi_core::geometry::NodeId::ROOT, (0.0, 0.0))];
    while let Some((node, id, origin)) = pending.pop() {
        let here = origin_of(layout, id, origin);
        if node.should_create_inline_layout()
            && let Ok(computed) = layout.layout(id)
        {
            let items = collect_inline_items(node);
            let has_inline_box = items.iter().any(|item| {
                matches!(item, InlineItem::RenderNode { render_node }
                    if render_node.participates_as_inline_box())
            });
            if has_inline_box {
                let ctx = &node.context;
                let font_style =
                    takumi_core::font_style::SizedFontStyle::from_style(&ctx.style, ctx);
                let built = create_inline_layout(InlineLayoutRequest::in_content_box(
                    items,
                    takumi_core::geometry::Size {
                        width: computed.content_box_width(),
                        height: computed.content_box_height(),
                    },
                    &font_style,
                    ctx,
                    InlineLayoutMode::Draw,
                ));
                let resolved = resolve_inline_runs(&built, ctx, computed).map_err(|error| {
                    LayoutError::BadStyle {
                        node: artifact.nodes[artifact.root.0 as usize].key.clone(),
                        declarations: "inline layout".into(),
                        reason: error.to_string(),
                    }
                })?;
                for visual in &resolved.inline_boxes {
                    let Some(ProcessedInlineSpan::Box(item)) = built.spans.get(visual.id as usize)
                    else {
                        continue;
                    };
                    let Some(key) = item
                        .render_node
                        .source_order()
                        .and_then(|order| source_keys.get(&order))
                    else {
                        continue;
                    };
                    let offset = computed.content_box_offset();
                    boxes.insert(
                        key.clone(),
                        valle_draw::Rect::new(
                            f64::from(here.0 + offset.x + visual.x + item.margin.left),
                            f64::from(here.1 + offset.y + visual.y + item.margin.top),
                            f64::from(
                                (visual.width - item.margin.left - item.margin.right).max(0.0),
                            ),
                            f64::from(
                                (visual.height - item.margin.top - item.margin.bottom).max(0.0),
                            ),
                        ),
                    );
                }
            }
        }
        if let (Some(children), Ok(layout_children)) = (&node.children, layout.box_children(id)) {
            for item in layout_children.iter().rev() {
                if let Some(child) = children.get(item.render_index) {
                    pending.push((child, item.node_id, here));
                }
            }
        }
    }
    Ok(())
}

/// Check the CSS capabilities implemented by the Motion layout and paint adapters.
fn admit_supported_surface(artifact: &SceneArtifact) -> Result<(), LayoutError> {
    let expr_types =
        crate::expr::validate_exprs(&artifact.exprs, &artifact.controls, &mut Vec::new()).types;
    for node in &artifact.nodes {
        for style in &node.styles {
            if matches!(style.property.as_str(), "background" | "background-image")
                && !gradient_background_binding(style, artifact, &expr_types)
            {
                return Err(LayoutError::UnsupportedSurface {
                    node: node.key.clone(),
                    surface: style.property.clone(),
                });
            }
        }
        // Utility admission and expansion already validate the actual property/value.
        // Substring bans here would reject supported transforms and effects again.
    }
    Ok(())
}

/// Admit every finite branch using the same CSS parser as concrete frame values.
fn gradient_background_binding(
    style: &valle_motion::StyleBinding,
    artifact: &SceneArtifact,
    types: &[Option<crate::expr::ExprType>],
) -> bool {
    match &style.value {
        StyleValue::Static {
            value: MotionValue::Color(_),
        } if style.property == "background" => true,
        StyleValue::Expr { expr }
            if style.property == "background"
                && types.get(expr.0 as usize) == Some(&Some(crate::expr::ExprType::Color)) =>
        {
            true
        }
        StyleValue::Static {
            value: MotionValue::Str(source) | MotionValue::Enum(source),
        } => gradient_background_source(&style.property, source),
        StyleValue::Expr { expr } => {
            crate::artifact::css_expression_variants(*expr, &artifact.exprs, types).is_some_and(
                |variants| {
                    variants
                        .iter()
                        .all(|source| gradient_background_source(&style.property, source))
                },
            )
        }
        _ => false,
    }
}

/// Lower the camera to two CSS wrappers: outer `T(viewport/2) * R * S`, inner `T(-center)`. Two
/// wrappers are required because independent CSS transforms provide only one translation per node.
/// CSS resolves viewport units and leaves layout boxes unchanged. Screen subtrees stay outside the
/// wrappers.
struct CameraWrappers {
    outer: takumi_core::style::Style,
    inner: takumi_core::style::Style,
    block: takumi_core::style::Style,
}

impl CameraWrappers {
    /// Wrap World content in the two camera groups.
    fn wrap(&self, world: Vec<Node>) -> Node {
        let inner = Node::container(world)
            .with_preset(self.block.clone())
            .with_style(self.inner.clone());
        Node::container([inner])
            .with_preset(self.block.clone())
            .with_style(self.outer.clone())
    }
}

/// Whether a camera binding depends on bounds and therefore requires a probe layout pass.
fn camera_depends_on_bounds(artifact: &SceneArtifact, plan: &EvalPlan) -> bool {
    artifact.camera.as_ref().is_some_and(|camera| {
        camera
            .exprs()
            .into_iter()
            .any(|(_, expr)| plan.bounds_dependent(expr))
    })
}

/// Probe wrappers with the same structure and positioning as the final camera, differing only in
/// transform values. Reuse `CAMERA_FILL` to preserve identical layout boxes.
fn identity_camera_wrappers(opts: &LayoutOptions<'_>) -> Result<CameraWrappers, LayoutError> {
    let style = |declarations: String| -> Result<takumi_core::style::Style, LayoutError> {
        parse_style(opts.styles, &declarations).map_err(|reason| LayoutError::BadStyle {
            node: "<camera-probe>".into(),
            declarations,
            reason,
        })
    };
    Ok(CameraWrappers {
        outer: style(format!(
            "{CAMERA_FILL}; translate: 0px 0px; rotate: 0deg; scale: 1"
        ))?,
        inner: style(format!("{CAMERA_FILL}; translate: 0px 0px"))?,
        block: style("display: block".into())?,
    })
}

/// Build, lay out, and collect boxes using the same parameters for probe and final passes.
fn layout_and_collect_boxes(
    prepared: &PreparedScene,
    node: Node,
    opts: &LayoutOptions<'_>,
) -> Result<BTreeMap<String, valle_draw::Rect>, LayoutError> {
    let artifact = prepared.artifact();
    let render_context = takumi_core::context::RenderContext::builder()
        .fonts(opts.fonts.snapshot())
        .sizing(SizingContext::builder().viewport(opts.viewport).build())
        .images(Rc::new(Default::default()))
        .stylesheet(prepared.stylesheet.clone())
        .time_ms(LayoutOptions::TIME_MS)
        .draw_debug_border(false)
        .style(Box::new(ComputedStyle::default()))
        .build();
    let mut root = RenderNode::from_node(&render_context, node);
    super::transform::preserve_identity(&mut root);
    let mut tree = takumi_core::layout::tree::LayoutTree::from_render_node(&root);
    super::continuous::compute_layout(&mut tree, render_context.sizing.viewport.into());
    let layout = tree.into_results();
    let mut boxes = BTreeMap::new();
    walk_pairs(artifact, &root, &layout, &[], &mut |_, key, id, origin| {
        if let Ok(computed) = layout.layout(id) {
            boxes.insert(
                key.to_owned(),
                valle_draw::Rect::new(
                    f64::from(origin.0),
                    f64::from(origin.1),
                    f64::from(computed.size.width),
                    f64::from(computed.size.height),
                ),
            );
        }
    })?;
    inline_boxes(artifact, &root, &layout, &[], &mut boxes)?;
    Ok(boxes)
}

/// Shared positioning and transform origin for both camera wrappers and the identity probe.
const CAMERA_FILL: &str = "position: absolute; left: 0; top: 0; width: 100%; height: 100%; \
                           transform-origin: 0 0";

/// Evaluate this frame's camera and produce the two wrapper styles described by [`CameraWrappers`].
/// Layout remains unchanged, so bounds stay in World coordinates.
fn camera_wrappers(
    artifact: &SceneArtifact,
    values: &[MotionValue],
    opts: &LayoutOptions<'_>,
) -> Result<Option<CameraWrappers>, LayoutError> {
    let Some(camera) = &artifact.camera else {
        return Ok(None);
    };
    let at = artifact.root.0 as usize;
    let center = point_value(&camera.center, values, at, "camera center")?;
    let zoom = number_value(&camera.zoom, values, at, "camera zoom")?;
    let rotation = number_value(&camera.rotation, values, at, "camera rotation")?;
    if !zoom.is_finite() || zoom <= 0.0 {
        return Err(LayoutError::BadCamera {
            reason: format!("zoom must be finite and positive, got {zoom}"),
        });
    }
    if !rotation.is_finite() || !center.x.is_finite() || !center.y.is_finite() {
        return Err(LayoutError::BadCamera {
            reason: "center and rotation must be finite".into(),
        });
    }

    let style = |declarations: String| -> Result<takumi_core::style::Style, LayoutError> {
        parse_style(opts.styles, &declarations).map_err(|reason| LayoutError::BadStyle {
            node: "<camera>".into(),
            declarations,
            reason,
        })
    };
    // Set `transform-origin: 0 0` so the wrappers compose to `T(viewport/2) * R * S * T(-center)`.
    // The CSS default center origin would add unwanted translations.
    Ok(Some(CameraWrappers {
        outer: style(format!(
            "{CAMERA_FILL}; translate: 50vw 50vh; rotate: {rotation}deg; scale: {zoom}"
        ))?,
        inner: style(format!(
            "{CAMERA_FILL}; translate: {}px {}px",
            -center.x, -center.y
        ))?,
        block: style("display: block".into())?,
    }))
}

/// Require consumers of post-layout coordinates to be at the scene origin. Scalar fields such as
/// width do not carry coordinates and are exempt.
fn check_post_layout_origins(
    artifact: &SceneArtifact,
    values: &[MotionValue],
    boxes: &BTreeMap<String, valle_draw::Rect>,
) -> Result<(), LayoutError> {
    let post_layout_dependent = valle_motion::post_layout_dependent(&artifact.exprs);
    for node in &artifact.nodes {
        let Some(node_box) = boxes.get(&node.key) else {
            continue;
        };
        if node_box.x == 0.0 && node_box.y == 0.0 {
            continue;
        }
        let carries_coordinates = node
            .expr_refs_outside_per_unit()
            .into_iter()
            .any(|(_, id)| {
                post_layout_dependent
                    .get(id.0 as usize)
                    .copied()
                    .unwrap_or(false)
                    && matches!(
                        values.get(id.0 as usize),
                        Some(
                            MotionValue::Point(_)
                                | MotionValue::Vec2(_)
                                | MotionValue::Rect(_)
                                | MotionValue::PathData(_)
                        )
                    )
            });
        if carries_coordinates {
            return Err(LayoutError::PostLayoutOffOrigin {
                node: node.key.clone(),
                origin: (node_box.x, node_box.y),
            });
        }
    }
    Ok(())
}

/// Geometry snapshot keyed by stable Scene node key, used by tests and later locate wiring.
pub fn layout_boxes(
    artifact: &SceneArtifact,
    tree: &LayoutTree,
) -> Result<BTreeMap<String, [f32; 4]>, LayoutError> {
    let mut boxes = BTreeMap::new();
    walk_pairs(
        artifact,
        &tree.root,
        &tree.layout,
        &tree.inactive_nodes,
        &mut |_, key, id, origin| {
            if let Ok(layout) = tree.layout.layout(id) {
                boxes.insert(
                    key.to_owned(),
                    [origin.0, origin.1, layout.size.width, layout.size.height],
                );
            }
        },
    )?;
    let mut inlines = BTreeMap::new();
    inline_boxes(
        artifact,
        &tree.root,
        &tree.layout,
        &tree.inactive_nodes,
        &mut inlines,
    )?;
    boxes.extend(inlines.into_iter().map(|(key, rect)| {
        (
            key,
            [
                rect.x as f32,
                rect.y as f32,
                rect.width as f32,
                rect.height as f32,
            ],
        )
    }));
    Ok(boxes)
}

#[cfg(test)]
mod tests {
    use super::{
        Css3dOp, Css3dStyle, css_3d_local_matrix, declarations, resolve_layer_fx, unit_boundaries,
    };
    use crate::layout::bridge::PerspectiveLength;
    use valle_motion::value::{Angle, Length, Length2, LengthUnit};
    use valle_motion::{
        ChildRange, MotionValue, NodeKind, SceneNode, StyleBinding, StyleValue, TextSplit,
    };

    fn static_style(property: &str, value: MotionValue) -> StyleBinding {
        StyleBinding {
            property: property.into(),
            value: StyleValue::Static { value },
        }
    }

    #[test]
    fn css_3d_origin_changes_projected_corner_coordinates() {
        let style = Css3dStyle {
            ops: vec![Css3dOp::RotateY(45.0)],
            ..Default::default()
        };
        let rect = valle_draw::Rect::new(100.0, 100.0, 100.0, 100.0);
        let corner = css_3d_local_matrix(&style, valle_draw::Point::new(100.0, 100.0), rect)
            .apply(100.0, 100.0, 0.0)
            .unwrap();
        let center = css_3d_local_matrix(&style, valle_draw::Point::new(150.0, 150.0), rect)
            .apply(100.0, 100.0, 0.0)
            .unwrap();
        assert!((corner[0] - 100.0).abs() < 1e-9);
        assert!((corner[1] - 100.0).abs() < 1e-9);
        assert!((center[0] - (150.0 - 50.0 / 2.0_f64.sqrt())).abs() < 1e-9);
        assert!((center[1] - 100.0).abs() < 1e-9);
    }

    #[test]
    fn motion_path_center_anchor_lowers_to_box_relative_translate() {
        let node = SceneNode {
            key: "follower".into(),
            kind: NodeKind::Box,
            space: None,
            class_names: vec![],
            class_conditions: Default::default(),
            styles: vec![
                static_style("motion-path-anchor", MotionValue::Enum("center".into())),
                static_style("translate", MotionValue::Length2(Length2::px(30.0, 20.0))),
                static_style("motion-path-angle-offset", MotionValue::Number(15.0)),
                static_style("rotate", MotionValue::Angle(Angle::deg(45.0))),
            ],
            visibility: None,
            children: ChildRange::EMPTY,
            semantic: None,
        };

        assert_eq!(
            declarations(&node, &[], true, 0).unwrap().source(),
            "transform-origin: 50% 50%; translate: calc(30px - 50%) calc(20px - 50%); rotate: calc(45deg + 15deg)"
        );
    }

    fn fx_node(styles: Vec<StyleBinding>) -> SceneNode {
        SceneNode {
            key: "card".into(),
            kind: NodeKind::Box,
            space: None,
            class_names: vec![],
            class_conditions: Default::default(),
            styles,
            visibility: None,
            children: ChildRange::EMPTY,
            semantic: None,
        }
    }

    #[test]
    fn perspective_keeps_relative_units_and_rejects_percent_or_non_positive() {
        let rem = resolve_layer_fx(
            &fx_node(vec![
                static_style("rotate-y", MotionValue::Number(20.0)),
                static_style(
                    "perspective",
                    MotionValue::Length(Length {
                        value: 2.0,
                        unit: LengthUnit::Rem,
                    }),
                ),
            ]),
            &[],
            0,
        )
        .unwrap()
        .unwrap();
        assert_eq!(rem.perspective, Some(PerspectiveLength::Rem(2.0)));

        let percent = resolve_layer_fx(
            &fx_node(vec![static_style(
                "perspective",
                MotionValue::Length(Length {
                    value: 50.0,
                    unit: LengthUnit::Percent,
                }),
            )]),
            &[],
            0,
        )
        .unwrap_err();
        assert!(percent.to_string().contains("percent"), "{percent}");

        let zero = resolve_layer_fx(
            &fx_node(vec![static_style("perspective", MotionValue::Number(0.0))]),
            &[],
            0,
        )
        .unwrap_err();
        assert!(zero.to_string().contains("positive"), "{zero}");

        let negative = resolve_layer_fx(
            &fx_node(vec![static_style(
                "perspective",
                MotionValue::Length(Length::px(-40.0)),
            )]),
            &[],
            0,
        )
        .unwrap_err();
        assert!(negative.to_string().contains("positive"), "{negative}");
    }

    #[test]
    fn perspective_length_resolves_against_takumi_sizing() {
        use crate::Viewport;
        use takumi_core::style::SizingContext;

        let sizing = SizingContext::builder()
            .viewport(Viewport::new((640, 360)))
            .build();
        assert_eq!(PerspectiveLength::Rem(2.0).to_px(&sizing).unwrap(), 32.0);
        assert_eq!(PerspectiveLength::Vw(50.0).to_px(&sizing).unwrap(), 320.0);
        assert_eq!(PerspectiveLength::Vh(25.0).to_px(&sizing).unwrap(), 90.0);
        assert_eq!(PerspectiveLength::Px(900.0).to_px(&sizing).unwrap(), 900.0);

        let retina = SizingContext::builder()
            .viewport(Viewport::new((640, 360)).with_device_pixel_ratio(2.0))
            .build();
        assert_eq!(
            PerspectiveLength::DEFAULT_PX.to_px(&retina).unwrap(),
            PerspectiveLength::Px(1200.0).to_px(&retina).unwrap()
        );
        assert_eq!(
            PerspectiveLength::DEFAULT_PX.to_px(&retina).unwrap(),
            2400.0
        );
    }

    fn slice<'a>(source: &'a str, split: TextSplit) -> Vec<&'a str> {
        unit_boundaries(source, split)
            .into_iter()
            .map(|(start, end)| &source[start as usize..end as usize])
            .collect()
    }

    #[test]
    fn char_units_are_scalars_not_bytes() {
        // CJK characters occupy multiple bytes; character splitting must preserve valid Unicode
        // boundaries.
        assert_eq!(slice("订单a", TextSplit::Char), ["订", "单", "a"]);
    }

    #[test]
    fn word_units_skip_whitespace_entirely() {
        // Whitespace does not form a word unit or affect stagger spacing.
        assert_eq!(
            slice("  hello   world  ", TextSplit::Word),
            ["hello", "world"]
        );
        assert_eq!(slice("单词 之间", TextSplit::Word), ["单词", "之间"]);
    }

    #[test]
    fn word_units_of_blank_text_are_empty_not_one_empty_unit() {
        assert!(slice("   ", TextSplit::Word).is_empty());
    }

    #[test]
    fn line_units_keep_blank_lines_so_indices_do_not_drift() {
        // Keep empty lines so later unit indices do not shift.
        assert_eq!(slice("a\n\nb", TextSplit::Line), ["a", "", "b"]);
        // A trailing newline creates a final empty unit.
        assert_eq!(slice("a\n", TextSplit::Line), ["a", ""]);
        // Text without a newline forms one unit.
        assert_eq!(slice("single", TextSplit::Line), ["single"]);
    }

    #[test]
    fn line_units_leave_cr_on_the_preceding_segment() {
        // Split only at LF; retain CR in the preceding range to preserve source byte offsets.
        assert_eq!(slice("a\r\nb", TextSplit::Line), ["a\r", "b"]);
    }
}
