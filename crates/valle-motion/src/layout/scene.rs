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
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use std::sync::Arc;

use takumi_core::layout::node::Node;
use takumi_core::layout::tree::{LayoutResults, RenderNode};
use takumi_core::scene::{PaintItemKind, StackingContextNode, build_stacking_contexts};
use takumi_core::style::{Affine as TAffine, ComputedStyle, SizingContext, ZIndex};
use valle_motion::MotionValue;
use valle_motion::value::{Length, LengthUnit};
use valle_motion::{
    BatchPositions, BoolValue, ColorValue, CoordinateSpace, EvalError, EvalInputs, Expr, ExprId,
    GeometryBatchGeometry, GradientStopValue, InstanceColumnValues, InstanceGroup,
    InstanceTemplateNode, MaskValue, NodeId, NodeKind, NumberValue, PaintValue, PathData,
    PathValue, PointValue, RectValue, ResolvedProps, SceneArtifact, ShaderUniformValue, StyleValue,
    TextSplit, TextValue, ValidationError, css_token,
};

use super::activation::{ActivationPlan, InstanceRangeGate, InstanceRowSelection};
use super::dependencies::{SceneDependencies, TemporalSampling};
use crate::layout::bridge::{
    EchoSample, GlassLayoutEnvironment, GlassLayoutField, GlassLayoutForeground, GlassLayoutFrame,
    GlassLayoutMaterial, GlassLayoutMotion, GlassLayoutSurface, LayoutOptions, LayoutTree,
    ResolvedUnit, TemporalSample, parse_style,
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
                "viewport dimensions must be non-zero and DPR finite and positive; a scene that reads ctx.viewport also requires both dimensions and a finite positive initial font size",
            ),
            LayoutError::BadStyle {
                node,
                reason, ..
            } => write!(f, "node `{node}`: {reason}"),
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
    /// Force trajectories are baked once at scene preparation, never integrated during a frame.
    particle_trajectories: Arc<Vec<Option<BakedParticleTrajectories>>>,
    instance_activation: Arc<Vec<Option<InstanceRangeGate>>>,
    eval_plan: EvalPlan,
    dependencies: Arc<SceneDependencies>,
    activation: ActivationPlan,
    layout_reusable: bool,
    /// Output-seconds derivatives of inline translations, when the entire scene has static
    /// geometry and no other time-varying transforms. `None` selects the sampled layout path.
    auto_blur_translations: Option<HashMap<String, valle_draw::Point>>,
    stylesheet: Arc<takumi_core::style::StyleSheet>,
    sample_scene: Option<Arc<PreparedScene>>,
    sample_roots: Option<Vec<ExprId>>,
    requires_explicit_viewport: bool,
    layout_classes: Vec<Vec<String>>,
    layout_probes: Vec<(crate::ExprId, MotionValue)>,
}

struct LayoutInstanceRow {
    key: String,
    class_name: String,
    declarations: String,
    background_color: Option<valle_draw::program::AuthorColor>,
    text_color: Option<valle_draw::program::AuthorColor>,
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
    pub(super) fn same_instance(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.artifact, &other.artifact)
    }

    pub fn artifact(&self) -> &SceneArtifact {
        &self.artifact
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
            None,
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
    prepare_admitted(artifact, false)
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
    let sample_scene = if is_sample {
        None
    } else {
        super::temporal::sample_artifact(&artifact, &dependencies)
            .map(|subset| prepare_admitted(subset, true).map(Arc::new))
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
            if let PointValue::Expr { expr } = camera.center {
                roots.push(expr);
            }
            for value in [&camera.zoom, &camera.rotation] {
                if let NumberValue::Expr { expr } = value {
                    roots.push(*expr);
                }
            }
        }
        eval_plan.schedule(roots)
    });
    Ok(PreparedScene {
        sample_scene,
        sample_roots,
        eval_plan,
        dependencies,
        activation,
        materialized_instance_groups: Arc::new(materialized_instance_groups),
        instance_eval_plans: Arc::new(instance_eval_plans),
        particle_trajectories: Arc::new(particle_trajectories),
        instance_activation: Arc::new(instance_activation),
        layout_classes,
        // Only expressions that read `ctx.viewport` still need both dimensions; the canvas itself
        // comes from the artifact's delivery contract.
        requires_explicit_viewport: artifact
            .capability_set
            .names
            .iter()
            .any(|name| name == crate::artifact::VIEWPORT_CAPABILITY),
        layout_reusable,
        auto_blur_translations,
        artifact: Arc::new(artifact),
        stylesheet,
        layout_probes,
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
    Ok((tree, timings))
}

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
    let camera = if camera_depends_on_bounds(artifact, &prepared.eval_plan) {
        let probe = identity_camera_wrappers(opts)?;
        let probe_node = node_of(
            prepared,
            artifact.root.0 as usize,
            &values,
            opts.styles,
            Some(&probe),
            &formulas,
            None,
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
        camera_wrappers(artifact, &probe_values, opts)?
    } else {
        camera_wrappers(artifact, &values, opts)?
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
            None,
            &inactive_nodes,
            &layout_instances,
        )?;
        let mut layout_root = RenderNode::from_node(&render_context, node);
        super::transform::preserve_identity(&mut layout_root);
        let mut tree = takumi_core::layout::tree::LayoutTree::from_render_node(&layout_root);
        if let Some(timings) = timings.as_deref_mut() {
            timings.tree_ms = elapsed(&mut started);
        }
        tree.compute_layout(render_context.sizing.viewport.into());
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
    // Read the current frame's author cascade before adding derived depth. This includes
    // conditional utilities, inline declarations, and importance in one computed result.
    let author_styles = if artifact.nodes.iter().any(|node| {
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
        let author_node = node_of(
            prepared,
            artifact.root.0 as usize,
            &values,
            opts.styles,
            camera.as_ref(),
            &formulas,
            None,
            &inactive_nodes,
            &layout_instances,
        )?;
        let author_root = RenderNode::from_node(&render_context, author_node);
        css_3d_author_styles(artifact, &author_root, &boxes, &inactive_nodes)?
    } else {
        HashMap::new()
    };
    let css_3d_planes = resolve_css_3d_planes(artifact, &values, &boxes, opts, &author_styles)?;

    // Post-layout values are admitted only into paint/transform slots or sidecar geometry. Rebuild
    // the same fixed-topology RenderNode with their final values, but keep the already-computed
    // LayoutResults. This makes `translate: project3d(...)` visible without a layout feedback pass.
    let final_node = node_of(
        prepared,
        artifact.root.0 as usize,
        &values,
        opts.styles,
        camera.as_ref(),
        &formulas,
        Some(&css_3d_planes),
        &inactive_nodes,
        &layout_instances,
    )?;
    let mut root = RenderNode::from_node(&render_context, final_node);
    super::transform::preserve_identity(&mut root);
    apply_css_3d_depth(artifact, &mut root, &css_3d_planes, &inactive_nodes);

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
    let auto_velocities = if auto_blur {
        let (velocities, neighbor_layouts) = auto_blur_velocities(
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
        velocities
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

    // Resolve advanced CSS filters only after normal/important class and inline cascade.
    let mut css_filters = HashMap::new();
    let mut pending = vec![(&root, takumi_core::geometry::NodeId::ROOT)];
    while let Some((render_node, id)) = pending.pop() {
        if let Some(key) = keys.get(&u64::from(id)) {
            let properties = &render_node.context.style.custom_properties;
            if let Some(text) = properties
                .get(crate::style::advanced_filter::IMPORTANT_SLOT)
                .or_else(|| properties.get(crate::style::advanced_filter::SLOT))
                && let Some(filter) =
                    crate::style::advanced_filter::parse(text).map_err(|reason| {
                        LayoutError::BadStyle {
                            node: key.clone(),
                            declarations: text.clone(),
                            reason,
                        }
                    })?
            {
                css_filters.insert(key.clone(), filter);
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
    let mut backdrop_advanced_filters = HashMap::new();
    let mut transitions = HashMap::new();
    let mut shaders = HashMap::new();
    let mut layer_fx = HashMap::new();
    let mut blend_spaces = HashMap::new();
    let scene3d = scene3d;
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
        let mut filters =
            resolve_advanced_filters(node, &values, at, auto_velocities.get(&node.key).copied())?;
        if let Some(mut filter) = css_filters.remove(&node.key) {
            if let valle_draw::program::recording::FilterOp::FilmGrain { seed, .. } = &mut filter {
                let frame = match resolved_style(node, &values, at, "motion-filter-frame")? {
                    Some(MotionValue::Number(frame)) => *frame as i64 as u32,
                    _ => ctx.local_frame,
                };
                *seed = seed.wrapping_add(frame.wrapping_mul(0x9e37_79b9));
            }
            filters.push(filter);
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
            NodeKind::Transition {
                effect,
                progress,
                params,
            } => {
                let bindings = params;
                let params = params
                    .iter()
                    .map(|(name, value)| {
                        Ok((
                            name.clone(),
                            number_value(value, &values, at, "transition parameter")?,
                        ))
                    })
                    .collect::<Result<std::collections::BTreeMap<_, _>, LayoutError>>()?;
                let params =
                    effect
                        .resolve_params(&params)
                        .map_err(|error| LayoutError::BadTransition {
                            node: node.key.clone(),
                            reason: match bindings.get(&error.parameter) {
                                Some(NumberValue::Expr { expr }) => {
                                    format!("expression {}: {error}", expr.0)
                                }
                                _ => error.to_string(),
                            },
                        })?;
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
                        kind: *effect,
                        params,
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
                let uniforms = uniforms
                    .iter()
                    .map(|binding| {
                        let value = match &binding.value {
                            ShaderUniformValue::Float { value } => {
                                valle_draw::program::recording::ShaderUniformValue::Float {
                                    value: number_value(value, &values, at, "shader float")? as f32,
                                }
                            }
                            ShaderUniformValue::Float2 { value } => {
                                let value = point_value(value, &values, at, "shader float2")?;
                                valle_draw::program::recording::ShaderUniformValue::Float2 {
                                    value: [value.x as f32, value.y as f32],
                                }
                            }
                            ShaderUniformValue::Float3 { value } => {
                                let mut components = [0.0f32; 3];
                                for (slot, component) in components.iter_mut().zip(value) {
                                    *slot =
                                        number_value(component, &values, at, "shader component")?
                                            as f32;
                                }
                                valle_draw::program::recording::ShaderUniformValue::Float3 {
                                    value: components,
                                }
                            }
                            ShaderUniformValue::Float4 { value } => {
                                let mut components = [0.0f32; 4];
                                for (slot, component) in components.iter_mut().zip(value) {
                                    *slot =
                                        number_value(component, &values, at, "shader component")?
                                            as f32;
                                }
                                valle_draw::program::recording::ShaderUniformValue::Float4 {
                                    value: components,
                                }
                            }
                            ShaderUniformValue::Float2x2 { value } => {
                                let mut components = [0.0f32; 4];
                                for (slot, component) in components.iter_mut().zip(value) {
                                    *slot =
                                        number_value(component, &values, at, "shader component")?
                                            as f32;
                                }
                                valle_draw::program::recording::ShaderUniformValue::Float2x2 {
                                    value: components,
                                }
                            }
                            ShaderUniformValue::Float3x3 { value } => {
                                let mut components = [0.0f32; 9];
                                for (slot, component) in components.iter_mut().zip(value) {
                                    *slot =
                                        number_value(component, &values, at, "shader component")?
                                            as f32;
                                }
                                valle_draw::program::recording::ShaderUniformValue::Float3x3 {
                                    value: components,
                                }
                            }
                            ShaderUniformValue::Float4x4 { value } => {
                                let mut components = [0.0f32; 16];
                                for (slot, component) in components.iter_mut().zip(value) {
                                    *slot =
                                        number_value(component, &values, at, "shader component")?
                                            as f32;
                                }
                                valle_draw::program::recording::ShaderUniformValue::Float4x4 {
                                    value: components,
                                }
                            }
                            ShaderUniformValue::Color { value } => {
                                let value = color_value(value, &values, at)?;
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
                                            return Err(LayoutError::Eval(
                                                EvalError::TypeMismatch {
                                                    at,
                                                    op: "shader bool",
                                                },
                                            ));
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
                                    node: node.key.clone(),
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
                    .collect::<Result<Vec<_>, LayoutError>>()?;
                shaders.insert(
                    node.key.clone(),
                    crate::layout::bridge::ResolvedShaderLayer {
                        program: valle_draw::program::recording::ShaderProgram {
                            work_per_pixel: program.work_per_pixel,
                            padding: program.padding,
                            uri: program.uri.clone(),
                            content_hash: valle_draw::requirements::DigestBytes::from_bytes(
                                *program.content_hash.as_bytes(),
                            ),
                            abi_hash: valle_draw::requirements::DigestBytes::from_bytes(
                                *program.abi_hash.as_bytes(),
                            ),
                        },
                        uniforms,
                        inputs: inputs.clone(),
                    },
                );
            }
            NodeKind::GeometryBatch { batch } => {
                let frame = match &batch.positions {
                    BatchPositions::Static { .. } => 0.0,
                    BatchPositions::Particles { frame, .. } => match values.get(frame.0 as usize) {
                        Some(MotionValue::Number(value)) => *value,
                        _ => {
                            return Err(LayoutError::Eval(EvalError::TypeMismatch {
                                at,
                                op: "particles frame",
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
                    frame,
                    crate::frame_rate_as_f64(ctx.fps),
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
                    resolve_instance_batch(
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
                // rest of the subtree, not at its quantized localFrame address.
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

    let glass = resolve_glass_layout(
        artifact,
        &values,
        &root,
        &layout,
        &keys,
        &boxes,
        &css_3d_planes,
        &layout_instances,
    )?;

    if let Some(timings) = timings.as_deref_mut() {
        timings.finish_ms = elapsed(&mut started);
    }
    let (shutters, echoes) = if temporal_samples {
        let mut at_time = HashMap::new();
        let shutters = resolve_shutter_samples(
            prepared,
            ctx,
            props,
            opts,
            &mut at_time,
            shared.as_deref_mut(),
        )?;
        let echoes = resolve_echo_samples(
            prepared,
            ctx,
            props,
            opts,
            &mut at_time,
            shared.as_deref_mut(),
        )?;
        (shutters, echoes)
    } else {
        (HashMap::new(), HashMap::new())
    };
    let mut background_colors = authored_style_colors(artifact, &values, "background-color");
    let mut text_colors = authored_style_colors(artifact, &values, "color");
    for rows in layout_instances.values() {
        for row in rows {
            visit_layout_instance_node(row, &mut |node| {
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

fn resolve_glass_layout(
    artifact: &SceneArtifact,
    values: &[MotionValue],
    root: &RenderNode,
    layout: &Arc<LayoutResults>,
    keys: &HashMap<u64, String>,
    boxes: &BTreeMap<String, valle_draw::Rect>,
    css_3d_planes: &HashMap<String, crate::layout::bridge::Css3dPlane>,
    layout_instances: &LayoutInstanceRows,
) -> Result<GlassLayoutFrame, LayoutError> {
    if !artifact
        .nodes
        .iter()
        .any(|node| matches!(node.kind, NodeKind::Glass(_) | NodeKind::GlassField(_)))
    {
        return Ok(GlassLayoutFrame::default());
    }

    let root_layout = layout
        .layout(takumi_core::geometry::NodeId::ROOT)
        .map_err(|error| LayoutError::BadStyle {
            node: artifact.nodes[artifact.root.0 as usize].key.clone(),
            declarations: "glass transform qualification".into(),
            reason: error.to_string(),
        })?;
    let contexts = build_stacking_contexts(
        root,
        layout,
        takumi_core::geometry::NodeId::ROOT,
        TAffine::IDENTITY,
        (Some(root_layout.size.width), Some(root_layout.size.height)),
    )
    .map_err(|error| LayoutError::BadStyle {
        node: artifact.nodes[artifact.root.0 as usize].key.clone(),
        declarations: "glass painter transform qualification".into(),
        reason: error.to_string(),
    })?;
    let mut transforms = HashMap::<String, TAffine>::new();
    collect_paint_transforms(&contexts, 0, keys, &mut transforms);

    let mut parents = vec![None; artifact.nodes.len()];
    for (parent, node) in artifact.nodes.iter().enumerate() {
        let range = node.children.start as usize..node.children.end as usize;
        for child in &artifact.node_children[range] {
            parents[child.0 as usize] = Some(parent);
        }
    }
    let owner_matrix_for = |mut at: usize| {
        while let Some(parent) = parents[at] {
            if let Some(transform) = transforms.get(&artifact.nodes[parent].key) {
                return affine_matrix(*transform);
            }
            at = parent;
        }
        [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]
    };

    let membership = crate::glass::field_membership(artifact);
    let mut frame = GlassLayoutFrame::default();
    for (index, field) in membership.iter().enumerate() {
        if let Some(field) = field {
            frame
                .node_fields
                .insert(artifact.nodes[index].key.clone(), field.as_str().to_owned());
            if let NodeKind::InstanceLayout { group } = artifact.nodes[index].kind
                && let Some(group) = artifact.instance_groups.get(group as usize)
            {
                for row in 0..group.rows() {
                    if let Some(key) = group.keys.key_at(row) {
                        frame.node_fields.insert(key, field.as_str().to_owned());
                    }
                }
            }
        }
    }

    let resolve_material = |material: &crate::glass::GlassMaterialBinding,
                            at: usize|
     -> Result<GlassLayoutMaterial, LayoutError> {
        Ok(GlassLayoutMaterial {
            clarity: number_value(&material.clarity, values, at, "glass material clarity")?,
            depth: number_value(&material.depth, values, at, "glass material depth")?,
            tint: color_value(&material.tint, values, at)?,
        })
    };
    let resolve_drive =
        |drive: &crate::glass::GlassDriveBinding, at: usize| -> Result<[f64; 4], LayoutError> {
            let translation = drive
                .translation
                .as_ref()
                .map(|value| point_value(value, values, at, "glass drive translation"))
                .transpose()?
                .unwrap_or_else(|| valle_draw::Point::new(0.0, 0.0));
            Ok([
                translation.x,
                translation.y,
                drive
                    .pressure
                    .as_ref()
                    .map(|value| number_value(value, values, at, "glass drive pressure"))
                    .transpose()?
                    .unwrap_or(0.0),
                drive
                    .twist
                    .as_ref()
                    .map(|value| number_value(value, values, at, "glass drive twist"))
                    .transpose()?
                    .unwrap_or(0.0),
            ])
        };
    let resolve_environment = |environment: &crate::glass::GlassEnvironmentBinding,
                               at: usize|
     -> Result<GlassLayoutEnvironment, LayoutError> {
        let direction = point_value(
            &environment.light.direction,
            values,
            at,
            "glass light direction",
        )?;
        Ok(GlassLayoutEnvironment {
            light_direction: [direction.x, direction.y],
            light_elevation: number_value(
                &environment.light.elevation,
                values,
                at,
                "glass light elevation",
            )?,
            light_intensity: number_value(
                &environment.light.intensity,
                values,
                at,
                "glass light intensity",
            )?,
            light_space: environment.light.space,
        })
    };

    for (at, node) in artifact.nodes.iter().enumerate() {
        let NodeKind::GlassField(field) = &node.kind else {
            continue;
        };
        frame.fields.push(GlassLayoutField {
            node_key: node.key.clone(),
            field_id: field.field_id.as_str().to_owned(),
            owner_to_viewport: owner_matrix_for(at),
            material: resolve_material(&field.material, at)?,
            environment: resolve_environment(&field.environment, at)?,
            motion: GlassLayoutMotion {
                character: field.motion.character,
                settle_seconds: field.motion.settle,
                intensity: number_value(
                    &field.motion.intensity,
                    values,
                    at,
                    "glass field intensity",
                )?,
                drive: resolve_drive(&field.motion.drive, at)?,
            },
            merge_distance: field.merge.distance,
            member_surface_ids: Vec::new(),
        });
    }
    frame
        .fields
        .sort_by(|left, right| left.field_id.cmp(&right.field_id));
    let field_by_id = frame
        .fields
        .iter()
        .map(|field| (field.field_id.clone(), field.clone()))
        .collect::<BTreeMap<_, _>>();

    for (at, node) in artifact.nodes.iter().enumerate() {
        let NodeKind::Glass(glass) = &node.kind else {
            continue;
        };
        let rect = boxes
            .get(&node.key)
            .copied()
            .ok_or(LayoutError::BadNode { at })?;
        if rect.is_empty() {
            return Err(LayoutError::BadStyle {
                node: node.key.clone(),
                declarations: "Glass layout box".into(),
                reason: "Glass requires a positive layout box".into(),
            });
        }
        let local_rect = valle_draw::Rect::new(0.0, 0.0, rect.width, rect.height);
        let transform = transforms.get(&node.key).copied().unwrap_or(TAffine {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            x: rect.x as f32,
            y: rect.y as f32,
        });
        let width = rect.width;
        let height = rect.height;
        let affine_viewport_matrix = [
            f64::from(transform.a) * width,
            f64::from(transform.c) * height,
            f64::from(transform.x),
            f64::from(transform.b) * width,
            f64::from(transform.d) * height,
            f64::from(transform.y),
            0.0,
            0.0,
            1.0,
        ];
        // CSS 3D is resolved before paint into one authoritative absolute viewport quad. The
        // foreground emitter consumes that same quad through BeginPerspective; the material
        // sidecar must therefore carry its homography as well or the two layers visibly split.
        let viewport_matrix = match css_3d_planes.get(&node.key) {
            Some(plane) if plane.project => {
                valle_draw::program::recording::Homography::map_rect(
                    valle_draw::Rect::new(0.0, 0.0, 1.0, 1.0),
                    plane.quad,
                )
                .ok_or_else(|| LayoutError::BadStyle {
                    node: node.key.clone(),
                    declarations: "Glass CSS 3D projection".into(),
                    reason: "projected Glass quad is degenerate or self-intersecting".into(),
                })?
                .0
            }
            _ => affine_viewport_matrix,
        };
        let (shape, radius, path_points) = match &glass.shape {
            crate::glass::GlassShapeBinding::Capsule => (
                valle_draw::program::PackedGlassShapeKind::Capsule,
                (width.min(height) * 0.5) as f32,
                Vec::new(),
            ),
            crate::glass::GlassShapeBinding::Circle => (
                valle_draw::program::PackedGlassShapeKind::Circle,
                (width.min(height) * 0.5) as f32,
                Vec::new(),
            ),
            crate::glass::GlassShapeBinding::ContinuousRect { radius } => (
                valle_draw::program::PackedGlassShapeKind::ContinuousRect,
                number_value(radius, values, at, "glass continuous radius")? as f32,
                Vec::new(),
            ),
            crate::glass::GlassShapeBinding::Path { path, .. } => {
                let points = path_value(path, values, at)?
                    .closed_polygon_points()
                    .map_err(|error| LayoutError::BadPaint {
                        node: node.key.clone(),
                        reason: format!("Glass path must be one closed contour: {error}"),
                    })?;
                if points.len() > valle_draw::program::glass::MAX_GLASS_PATH_POINTS {
                    return Err(LayoutError::BadPaint {
                        node: node.key.clone(),
                        reason: format!(
                            "Glass path has {} flattened points; maximum is {}",
                            points.len(),
                            valle_draw::program::glass::MAX_GLASS_PATH_POINTS
                        ),
                    });
                }
                (
                    valle_draw::program::PackedGlassShapeKind::Path,
                    0.0,
                    points
                        .into_iter()
                        .map(|point| [point.x as f32, point.y as f32])
                        .collect(),
                )
            }
        };
        let field_id = membership[at]
            .as_ref()
            .map(|field| field.as_str().to_owned());
        let member_intensity = number_value(
            &glass.motion.intensity,
            values,
            at,
            "glass member intensity",
        )?;
        let member_drive = resolve_drive(&glass.motion.drive, at)?;
        let (character, settle_seconds, intensity, drive) = match field_id
            .as_ref()
            .and_then(|field| field_by_id.get(field))
        {
            Some(field) => {
                let normalization = crate::glass::DEFAULT_INTENSITY.max(f64::EPSILON);
                (
                    field.motion.character,
                    field.motion.settle_seconds,
                    (field.motion.intensity * member_intensity / normalization).clamp(0.0, 1.0),
                    std::array::from_fn(|index| field.motion.drive[index] + member_drive[index]),
                )
            }
            None => (
                glass
                    .motion
                    .character
                    .unwrap_or(crate::glass::GlassCharacter::DEFAULT),
                glass
                    .motion
                    .settle
                    .unwrap_or(crate::glass::DEFAULT_SETTLE_SECONDS),
                member_intensity,
                member_drive,
            ),
        };
        let owner_to_viewport = field_id
            .as_ref()
            .and_then(|field| field_by_id.get(field))
            .map_or_else(|| affine_matrix(transform), |field| field.owner_to_viewport);
        let foreground_bounds =
            foreground_protection_bounds(artifact, at, boxes, &transforms, owner_to_viewport);
        let foreground_protection = number_value(
            &glass.foreground.protection,
            values,
            at,
            "glass foreground protection",
        )?;
        let foreground_luma = if glass.foreground.tone == crate::glass::GlassForegroundTone::Auto
            && foreground_protection > 0.0
            && foreground_bounds.is_some()
        {
            Some(
                foreground_auto_luma(artifact, at, values, boxes, layout_instances)?.ok_or_else(|| {
                    LayoutError::BadPaint {
                        node: node.key.clone(),
                        reason: "auto Glass foreground has pixels without a resolvable color descriptor; use tone light, dark, or none".into(),
                    }
                })?,
            )
        } else {
            None
        };
        frame.surfaces.push(GlassLayoutSurface {
            node_key: node.key.clone(),
            surface_id: glass.surface_id.as_str().to_owned(),
            field_id,
            shape,
            local_rect,
            viewport_matrix,
            owner_to_viewport,
            radius,
            path_points,
            // A backface-hidden member must disappear from a field's shared material program as
            // well as from its foreground subtree. Keeping the stable surface with zero presence
            // preserves topology while matching CSS visibility.
            presence: if css_3d_planes
                .get(&node.key)
                .is_some_and(|plane| plane.hidden)
            {
                0.0
            } else {
                number_value(&glass.presence, values, at, "glass presence")?
            },
            motion: GlassLayoutMotion {
                character,
                settle_seconds,
                intensity,
                drive,
            },
            material: glass
                .material
                .as_ref()
                .map(|material| resolve_material(material, at))
                .transpose()?,
            environment: glass
                .environment
                .as_ref()
                .map(|environment| resolve_environment(environment, at))
                .transpose()?,
            foreground: GlassLayoutForeground {
                tone: glass.foreground.tone,
                protection: foreground_protection,
                bounds: foreground_bounds,
                luma: foreground_luma,
            },
        });
    }
    frame
        .surfaces
        .sort_by(|left, right| left.surface_id.cmp(&right.surface_id));
    for field in &mut frame.fields {
        field.member_surface_ids = frame
            .surfaces
            .iter()
            .filter(|surface| surface.field_id.as_deref() == Some(field.field_id.as_str()))
            .map(|surface| surface.surface_id.clone())
            .collect();
    }
    Ok(frame)
}

fn collect_paint_transforms(
    contexts: &[StackingContextNode],
    index: usize,
    keys: &HashMap<u64, String>,
    output: &mut HashMap<String, TAffine>,
) {
    let Some(context) = contexts.get(index) else {
        return;
    };
    if let Some(root) = context.root()
        && let Some(key) = keys.get(&u64::from(root.node_id))
    {
        output.insert(key.clone(), root.transform);
    }
    for bucket in context.in_paint_order() {
        for item in bucket {
            match &item.kind {
                PaintItemKind::Node(node) => {
                    if let Some(key) = keys.get(&u64::from(node.node_id)) {
                        output.insert(key.clone(), node.transform);
                    }
                }
                PaintItemKind::Context(child) => {
                    collect_paint_transforms(contexts, *child, keys, output);
                }
            }
        }
    }
}

fn affine_matrix(transform: TAffine) -> [f64; 9] {
    [
        f64::from(transform.a),
        f64::from(transform.c),
        f64::from(transform.x),
        f64::from(transform.b),
        f64::from(transform.d),
        f64::from(transform.y),
        0.0,
        0.0,
        1.0,
    ]
}

fn foreground_protection_bounds(
    artifact: &SceneArtifact,
    glass_at: usize,
    boxes: &BTreeMap<String, valle_draw::Rect>,
    transforms: &HashMap<String, TAffine>,
    owner_to_viewport: [f64; 9],
) -> Option<valle_draw::Rect> {
    let viewport_to_owner = inverse_matrix3(owner_to_viewport)?;
    let children = artifact.nodes.get(glass_at)?.children;
    let mut stack = artifact.node_children[children.start as usize..children.end as usize]
        .iter()
        .map(|id| id.0 as usize)
        .collect::<Vec<_>>();
    let mut bounds: Option<valle_draw::Rect> = None;
    while let Some(at) = stack.pop() {
        let node = artifact.nodes.get(at)?;
        let paint_keys = if let NodeKind::InstanceLayout { group } = node.kind {
            let group = artifact.instance_groups.get(group as usize)?;
            let mut templates = vec![&group.template];
            let mut children = group.template_children.iter().collect::<Vec<_>>();
            while let Some(child) = children.pop() {
                templates.push(&child.node);
                children.extend(&child.children);
            }
            templates
                .into_iter()
                .filter(|template| foreground_node_paints(template))
                .flat_map(|template| {
                    (0..group.rows()).filter_map(move |row| group.key_for_node(row, &template.key))
                })
                .collect()
        } else if foreground_node_paints(node) {
            vec![node.key.clone()]
        } else {
            Vec::new()
        };
        for key in paint_keys {
            let Some(layout) = boxes.get(&key) else {
                continue;
            };
            let viewport_points = if let Some(transform) = transforms.get(&key) {
                let matrix = affine_matrix(*transform);
                [
                    project_matrix3(matrix, [0.0, 0.0])?,
                    project_matrix3(matrix, [layout.width, 0.0])?,
                    project_matrix3(matrix, [layout.width, layout.height])?,
                    project_matrix3(matrix, [0.0, layout.height])?,
                ]
            } else {
                [
                    [layout.left(), layout.top()],
                    [layout.right(), layout.top()],
                    [layout.right(), layout.bottom()],
                    [layout.left(), layout.bottom()],
                ]
            };
            let owner_points = viewport_points
                .map(|point| project_matrix3(viewport_to_owner, point))
                .into_iter()
                .collect::<Option<Vec<_>>>()?;
            let left = owner_points
                .iter()
                .map(|point| point[0])
                .fold(f64::INFINITY, f64::min);
            let top = owner_points
                .iter()
                .map(|point| point[1])
                .fold(f64::INFINITY, f64::min);
            let right = owner_points
                .iter()
                .map(|point| point[0])
                .fold(f64::NEG_INFINITY, f64::max);
            let bottom = owner_points
                .iter()
                .map(|point| point[1])
                .fold(f64::NEG_INFINITY, f64::max);
            let node_bounds = valle_draw::Rect::from_edges(left, top, right, bottom);
            if !node_bounds.is_empty() {
                bounds = Some(bounds.map_or(node_bounds, |current| {
                    valle_draw::Rect::from_edges(
                        current.left().min(node_bounds.left()),
                        current.top().min(node_bounds.top()),
                        current.right().max(node_bounds.right()),
                        current.bottom().max(node_bounds.bottom()),
                    )
                }));
            }
        }
        if !matches!(node.kind, NodeKind::Glass(_)) {
            let children = node.children;
            stack.extend(
                artifact.node_children[children.start as usize..children.end as usize]
                    .iter()
                    .map(|id| id.0 as usize),
            );
        }
    }
    bounds
}

fn foreground_node_paints(node: &crate::SceneNode) -> bool {
    let paints_style = node.styles.iter().any(|style| {
        matches!(
            style.property.as_str(),
            "background" | "border" | "box-shadow" | "outline"
        ) || style.property.starts_with("background-")
            || style.property.starts_with("border-")
    });
    paints_style
        || matches!(
            node.kind,
            NodeKind::Text { .. }
                | NodeKind::Path { .. }
                | NodeKind::GeometryBatch { .. }
                | NodeKind::InstanceBatch { .. }
                | NodeKind::Image { .. }
                | NodeKind::Transition { .. }
                | NodeKind::ShaderLayer { .. }
                | NodeKind::Scene3D { .. }
                | NodeKind::Video { .. }
                | NodeKind::MathFormula { .. }
                | NodeKind::Glass(_)
        )
}

fn foreground_auto_luma(
    artifact: &SceneArtifact,
    glass_at: usize,
    values: &[MotionValue],
    boxes: &BTreeMap<String, valle_draw::Rect>,
    layout_instances: &LayoutInstanceRows,
) -> Result<Option<f32>, LayoutError> {
    let children = artifact.nodes[glass_at].children;
    let mut stack = artifact.node_children[children.start as usize..children.end as usize]
        .iter()
        .map(|id| id.0 as usize)
        .collect::<Vec<_>>();
    let mut weighted_luma = 0.0_f64;
    let mut total_weight = 0.0_f64;
    while let Some(at) = stack.pop() {
        let node = artifact.nodes.get(at).ok_or(LayoutError::BadNode { at })?;
        if let NodeKind::InstanceLayout { group } = node.kind
            && let Some(rows) = layout_instances.get(&group)
        {
            for row in rows {
                visit_layout_instance_node(row, &mut |node| {
                    for color in [node.background_color, node.text_color]
                        .into_iter()
                        .flatten()
                    {
                        let color = color.to_srgb8();
                        let area = boxes
                            .get(&node.key)
                            .map_or(1.0, |bounds| (bounds.width * bounds.height).max(1.0));
                        let weight = area * f64::from(color.a) / 255.0;
                        weighted_luma += weight * color.relative_luminance();
                        total_weight += weight;
                    }
                });
            }
        }
        let colors = foreground_node_colors(node, values, at)?;
        if !colors.is_empty() {
            let area = boxes
                .get(&node.key)
                .map_or(1.0, |bounds| (bounds.width * bounds.height).max(1.0));
            let color_weight = area / colors.len() as f64;
            for color in colors {
                let alpha = f64::from(color.a) / 255.0;
                let weight = color_weight * alpha;
                weighted_luma += weight * color.relative_luminance();
                total_weight += weight;
            }
        }
        if !matches!(node.kind, NodeKind::Glass(_)) {
            let children = node.children;
            stack.extend(
                artifact.node_children[children.start as usize..children.end as usize]
                    .iter()
                    .map(|id| id.0 as usize),
            );
        }
    }
    Ok((total_weight > 0.0).then(|| (weighted_luma / total_weight).clamp(0.0, 1.0) as f32))
}

fn foreground_node_colors(
    node: &crate::SceneNode,
    values: &[MotionValue],
    at: usize,
) -> Result<Vec<valle_draw::Rgba>, LayoutError> {
    let mut colors = Vec::new();
    for style in &node.styles {
        let property = style.property.as_str();
        if property != "color"
            && property != "background-color"
            && property != "border-color"
            && !(property.starts_with("border-") && property.ends_with("-color"))
        {
            continue;
        }
        let resolved = match &style.value {
            StyleValue::Static { value } => value,
            StyleValue::Expr { expr } => value(values, *expr, at)?,
        };
        if let MotionValue::Color(color) = resolved {
            colors.push(color.to_srgb8());
        }
    }
    match &node.kind {
        NodeKind::Text { .. } | NodeKind::MathFormula { .. } if colors.is_empty() => {
            colors.push(valle_draw::Rgba::rgb(0, 0, 0));
        }
        NodeKind::Path { fill, stroke, .. } => {
            if let Some(fill) = fill {
                colors.extend(foreground_paint_colors(fill, values, at, &node.key)?);
            }
            if let Some(stroke) = stroke {
                colors.extend(foreground_paint_colors(
                    &stroke.paint,
                    values,
                    at,
                    &node.key,
                )?);
            }
        }
        NodeKind::GeometryBatch { batch } => {
            colors.extend(batch.fills.iter().map(|color| color.to_srgb8()));
            if let Some(field) = &batch.fill_field {
                colors.extend(field.to.iter().map(|color| color.to_srgb8()));
            }
        }
        _ => {}
    }
    Ok(colors)
}

fn foreground_paint_colors(
    paint: &PaintValue,
    values: &[MotionValue],
    at: usize,
    node: &str,
) -> Result<Vec<valle_draw::Rgba>, LayoutError> {
    let paint = paint_value(paint, values, at, node)?;
    Ok(match paint {
        crate::layout::bridge::ResolvedPaint::Solid(color) => vec![color.to_srgb8()],
        crate::layout::bridge::ResolvedPaint::Linear { stops, .. }
        | crate::layout::bridge::ResolvedPaint::Radial { stops, .. }
        | crate::layout::bridge::ResolvedPaint::Conic { stops, .. } => stops
            .into_iter()
            .map(|(_, color)| color.to_srgb8())
            .collect(),
    })
}

fn project_matrix3(matrix: [f64; 9], point: [f64; 2]) -> Option<[f64; 2]> {
    let w = matrix[6] * point[0] + matrix[7] * point[1] + matrix[8];
    if !w.is_finite() || w.abs() <= 1.0e-12 {
        return None;
    }
    let projected = [
        (matrix[0] * point[0] + matrix[1] * point[1] + matrix[2]) / w,
        (matrix[3] * point[0] + matrix[4] * point[1] + matrix[5]) / w,
    ];
    projected
        .iter()
        .all(|value| value.is_finite())
        .then_some(projected)
}

fn inverse_matrix3(value: [f64; 9]) -> Option<[f64; 9]> {
    let [a, b, c, d, e, f, g, h, i] = value;
    let cofactors = [
        e * i - f * h,
        c * h - b * i,
        b * f - c * e,
        f * g - d * i,
        a * i - c * g,
        c * d - a * f,
        d * h - e * g,
        b * g - a * h,
        a * e - b * d,
    ];
    let determinant = a * cofactors[0] + b * cofactors[3] + c * cofactors[6];
    if !determinant.is_finite() || determinant.abs() <= 1.0e-15 {
        return None;
    }
    Some(cofactors.map(|value| value / determinant))
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
                color: m.color.as_ref().map(|v| color(v)).transpose()?,
                emissive: m.emissive.as_ref().map(|v| color(v)).transpose()?,
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
        text,
        children: children
            .iter()
            .map(|child| {
                resolve_layout_instance_node(group, &child.node, &child.children, row, values)
            })
            .collect::<Result<_, _>>()?,
    })
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
                declarations: property.into(),
                reason: "instance template needs a finite supported value".into(),
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
                    Some(MotionValue::Number(value)) if value.is_finite() && *value > 0.0 => *value,
                    _ => return Err(invalid("r")),
                };
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
                let rotation = match resolved_style(&group.template, values, at, "rotate")? {
                    Some(MotionValue::Angle(angle)) => angle.as_degrees(),
                    Some(MotionValue::Str(value)) => valle_motion::value::Angle::parse(value)
                        .map(valle_motion::value::Angle::as_degrees)
                        .ok_or_else(|| invalid("rotate"))?,
                    None => 0.0,
                    _ => return Err(invalid("rotate")),
                };
                if !rotation.is_finite() || !(rotation as f32).is_finite() {
                    return Err(invalid("rotate"));
                }
                let scale = match resolved_style(&group.template, values, at, "scale")? {
                    Some(MotionValue::Number(value)) => valle_draw::Point::new(*value, *value),
                    Some(MotionValue::Point(point)) => *point,
                    None => valle_draw::Point::new(1.0, 1.0),
                    _ => return Err(invalid("scale")),
                };
                if !(scale.x as f32).is_finite() || !(scale.y as f32).is_finite() {
                    return Err(invalid("scale"));
                }
                let skew_x = match resolved_style(&group.template, values, at, "transform")? {
                    Some(MotionValue::Str(value)) => {
                        valle_motion::artifact::path_instance_skew_x(value)
                            .ok_or_else(|| invalid("transform"))?
                    }
                    None => 0.0,
                    _ => return Err(invalid("transform")),
                };
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
            // Takumi snaps both absolute box edges in f32 layout coordinates. Snapping only the
            // origin would keep B01's 4px cells correct but drift for fractional instance sizes.
            let (raw_x, raw_y) = (left as f32, top as f32);
            let x = raw_x.round();
            let y = raw_y.round();
            let snapped_width = (raw_x + width as f32).round() - x;
            let snapped_height = (raw_y + height as f32).round() - y;
            if snapped_width <= 0.0 || snapped_height <= 0.0 {
                return Ok(());
            }
            semantic_keys.push(
                group
                    .key_for_node(at, &group.template.key)
                    .unwrap_or_default(),
            );
            instances.push(valle_draw::program::recording::BatchInstance {
                position: valle_draw::Point::new(f64::from(x), f64::from(y)),
                size: valle_draw::Point::new(f64::from(snapped_width), f64::from(snapped_height)),
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
        if !tx.is_finite() || !ty.is_finite() || !tz.is_finite() || !w.is_finite() || w == 0.0 {
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
    for binding in &node.styles {
        let value = match &binding.value {
            StyleValue::Static { value } => value,
            StyleValue::Expr { expr } => value(values, *expr, at)?,
        };
        let scalar = |property: &str| layer_scalar(value, &node.key, property);
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
            "transform-style" => {
                out.preserve_3d =
                    matches!(value, MotionValue::Enum(value) if value == "preserve-3d")
            }
            "backface-visibility" => {
                out.backface_hidden = matches!(value, MotionValue::Enum(value) if value == "hidden")
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
    Ok(out)
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

#[derive(Clone, Copy)]
struct Css3dAuthorStyle {
    origin: valle_draw::Point,
}

/// Read the resolved author cascade from the same Takumi tree used for final paint. The origin
/// is resolved against this node's own font sizing and layout box, rather than a global guess.
fn css_3d_author_styles(
    artifact: &SceneArtifact,
    root: &RenderNode,
    boxes: &BTreeMap<String, valle_draw::Rect>,
    inactive_nodes: &[bool],
) -> Result<HashMap<String, Css3dAuthorStyle>, LayoutError> {
    let mut keys = HashMap::new();
    walk_render_keys(
        artifact,
        artifact.root,
        root,
        &mut Vec::new(),
        &mut keys,
        inactive_nodes,
    );
    let mut out = HashMap::new();
    for (path, key) in keys {
        let Some(node) = root
            .node_at_path(&path)
            .filter(|node| node.source_order().is_some())
        else {
            continue;
        };
        let Some(rect) = boxes.get(&key) else {
            continue;
        };
        let origin = node.context.style.transform_origin.0;
        let x = f64::from(
            takumi_core::style::Length::from(origin.x)
                .to_px(&node.context.sizing, rect.width as f32),
        ) + rect.x;
        let y = f64::from(
            takumi_core::style::Length::from(origin.y)
                .to_px(&node.context.sizing, rect.height as f32),
        ) + rect.y;
        if !x.is_finite() || !y.is_finite() {
            return Err(LayoutError::BadStyle {
                node: key,
                declarations: "transform-origin".into(),
                reason: "origin must resolve to finite coordinates".into(),
            });
        }
        out.insert(
            key,
            Css3dAuthorStyle {
                origin: valle_draw::Point::new(x, y),
            },
        );
    }
    Ok(out)
}

/// Auto depth is a derived computed default. Applying it after the author cascade means even an
/// important `z-index: auto` receives the fallback, while every explicit integer wins.
fn apply_css_3d_depth(
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
    author_styles: &HashMap<String, Css3dAuthorStyle>,
) -> Result<HashMap<String, crate::layout::bridge::Css3dPlane>, LayoutError> {
    let sizing = SizingContext::builder().viewport(opts.viewport).build();
    let mut out = HashMap::new();
    resolve_css_3d_node(
        artifact,
        artifact.root.0 as usize,
        values,
        boxes,
        &sizing,
        author_styles,
        Css3dState::default(),
        &mut out,
    )?;
    Ok(out)
}

fn resolve_css_3d_node(
    artifact: &SceneArtifact,
    at: usize,
    values: &[MotionValue],
    boxes: &BTreeMap<String, valle_draw::Rect>,
    sizing: &SizingContext,
    author_styles: &HashMap<String, Css3dAuthorStyle>,
    state: Css3dState,
    out: &mut HashMap<String, crate::layout::bridge::Css3dPlane>,
) -> Result<(), LayoutError> {
    let node = artifact.nodes.get(at).ok_or(LayoutError::BadNode { at })?;
    let style = css_3d_style(node, values, at)?;
    let rect = boxes.get(&node.key).copied().unwrap_or_default();
    let origin = author_styles.get(&node.key).map_or_else(
        || valle_draw::Point::new(rect.x + rect.width / 2.0, rect.y + rect.height / 2.0),
        |style| style.origin,
    );
    let local = css_3d_local_matrix(&style, origin, rect);
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
                sizing,
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
                sizing,
                &node.key,
                "perspectiveOrigin.y",
            )?,
        );
        Some(Css3dPerspective {
            distance: value
                .to_px(sizing)
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

    let is_plane =
        !style.preserve_3d && (state.preserve_3d || !style.ops.is_empty() || style.backface_hidden);
    if is_plane && rect.width > 0.0 && rect.height > 0.0 {
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
                reason: "CSS 3D transform produced a non-finite plane".into(),
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
        let signed_area = (quad[1].x - quad[0].x) * (quad[2].y - quad[0].y)
            - (quad[1].y - quad[0].y) * (quad[2].x - quad[0].x);
        let center =
            matrix
                .apply(origin.x, origin.y, 0.0)
                .ok_or_else(|| LayoutError::BadStyle {
                    node: node.key.clone(),
                    declarations: "transform".into(),
                    reason: "CSS 3D transform produced a non-finite center".into(),
                })?;
        out.insert(
            node.key.clone(),
            crate::layout::bridge::Css3dPlane {
                quad,
                depth: center[2],
                hidden: signed_area.abs() <= 1e-8 || (style.backface_hidden && signed_area < 0.0),
                project: true,
            },
        );
    }

    // Positioned preserve-3d children are siblings in their parent's 3D painter order. Their
    // descendants own the actual projected planes, so this record contributes only z-index.
    if style.preserve_3d
        && state.preserve_3d
        && rect.width > 0.0
        && rect.height > 0.0
        && let Some(center) = matrix.apply(origin.x, origin.y, 0.0)
    {
        out.insert(
            node.key.clone(),
            crate::layout::bridge::Css3dPlane {
                quad: [valle_draw::Point::default(); 4],
                depth: center[2],
                hidden: false,
                project: false,
            },
        );
    }

    let child_state = if style.preserve_3d {
        Css3dState {
            matrix,
            perspective,
            preserve_3d: true,
        }
    } else if style.perspective.is_some() {
        Css3dState {
            matrix: Matrix4::IDENTITY,
            perspective,
            preserve_3d: false,
        }
    } else {
        Css3dState::default()
    };
    let range = node.children.start as usize..node.children.end as usize;
    let child_ids = artifact
        .node_children
        .get(range)
        .ok_or(LayoutError::BadNode { at })?;
    for child in child_ids {
        resolve_css_3d_node(
            artifact,
            child.0 as usize,
            values,
            boxes,
            sizing,
            author_styles,
            child_state,
            out,
        )?;
    }
    Ok(())
}

/// Evaluate each exposure at an exact output-time offset. A distinct evaluation round and layout
/// belong to every sample; repeated times across Shutter nodes share one immutable result.
fn resolve_shutter_samples(
    prepared: &PreparedScene,
    ctx: &valle_motion::MotionContext,
    props: &ResolvedProps,
    opts: &LayoutOptions<'_>,
    at_time: &mut HashMap<valle_timeline::internal::SampleTime, Rc<LayoutTree>>,
    mut shared: Option<&mut RequestValueCache>,
) -> Result<HashMap<String, Vec<TemporalSample>>, LayoutError> {
    use valle_timeline::{RationalTime, internal::SampleTime};

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
    let last =
        crate::motion_context_at_frame(ctx.duration_frames - 1, ctx.duration_frames, ctx.fps)
            .ok_or_else(|| LayoutError::BadShutter {
                node: shutters[0].0.to_owned(),
                reason: "last output frame has no exact sample time".into(),
            })?;
    let mut out = HashMap::new();
    for (key, phases) in shutters {
        let bad_time = |reason: &str| LayoutError::BadShutter {
            node: key.to_owned(),
            reason: reason.into(),
        };
        let frame_duration = RationalTime::new(
            i64::from(ctx.fps.denominator()),
            u32::try_from(ctx.fps.numerator())
                .map_err(|_| bad_time("output frame rate cannot form exact shutter offsets"))?,
        )
        .map_err(|_| bad_time("output frame rate cannot form exact shutter offsets"))?;
        let mut exposure = Vec::with_capacity(phases.len());
        for &phase in phases {
            let offset = frame_duration
                .checked_mul(phase)
                .map_err(|_| bad_time("shutter offset overflows exact time"))?;
            let requested = ctx
                .sample
                .checked_offset(offset)
                .map_err(|_| bad_time("shutter sample overflows exact time"))?;
            let sample = crate::motion_context_at_sample(requested, ctx.duration_frames, ctx.fps)
                .unwrap_or_else(|| {
                    if requested < SampleTime::ZERO {
                        crate::motion_context_at_frame(0, ctx.duration_frames, ctx.fps)
                            .expect("validated nonempty composition")
                    } else {
                        last
                    }
                });
            let tree = if let Some(tree) = at_time.get(&sample.sample) {
                Rc::clone(tree)
            } else {
                let tree = Rc::new(build_tree_inner(
                    prepared.sample_scene.as_deref().unwrap_or(prepared),
                    &sample,
                    props,
                    opts,
                    None,
                    None,
                    shared.as_deref_mut(),
                    true,
                    false,
                )?);
                at_time.insert(sample.sample, Rc::clone(&tree));
                tree
            };
            exposure.push(TemporalSample {
                time: sample.sample,
                tree,
            });
        }
        out.insert(key.to_owned(), exposure);
    }
    Ok(out)
}

/// Evaluate prior output frames, oldest first. The current subtree is painted separately above
/// these samples, so a stationary subject naturally covers all of its converged echoes.
fn resolve_echo_samples(
    prepared: &PreparedScene,
    ctx: &valle_motion::MotionContext,
    props: &ResolvedProps,
    opts: &LayoutOptions<'_>,
    at_time: &mut HashMap<valle_timeline::internal::SampleTime, Rc<LayoutTree>>,
    mut shared: Option<&mut RequestValueCache>,
) -> Result<HashMap<String, Vec<EchoSample>>, LayoutError> {
    use valle_timeline::{RationalTime, internal::SampleTime};

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
        let bad_time = |reason: &str| LayoutError::BadEcho {
            node: key.to_owned(),
            reason: reason.into(),
        };
        let frame_duration = RationalTime::new(
            i64::from(ctx.fps.denominator()),
            u32::try_from(ctx.fps.numerator())
                .map_err(|_| bad_time("output frame rate cannot form exact echo offsets"))?,
        )
        .map_err(|_| bad_time("output frame rate cannot form exact echo offsets"))?;
        let mut trail = Vec::with_capacity(offsets.len());
        for (index, &offset_frames) in offsets.iter().enumerate() {
            let lag = count - index as u8;
            let offset = frame_duration
                .checked_mul(offset_frames)
                .map_err(|_| bad_time("echo offset overflows exact time"))?;
            let requested = ctx
                .sample
                .checked_offset(offset)
                .map_err(|_| bad_time("echo sample overflows exact time"))?;
            let sample = if requested < SampleTime::ZERO {
                crate::motion_context_at_frame(0, ctx.duration_frames, ctx.fps)
                    .ok_or_else(|| bad_time("first output frame has no exact sample time"))?
            } else {
                crate::motion_context_at_sample(requested, ctx.duration_frames, ctx.fps)
                    .ok_or_else(|| bad_time("echo sample lies outside the composition"))?
            };
            let tree = if let Some(tree) = at_time.get(&sample.sample) {
                Rc::clone(tree)
            } else {
                let tree = Rc::new(build_tree_inner(
                    prepared.sample_scene.as_deref().unwrap_or(prepared),
                    &sample,
                    props,
                    opts,
                    None,
                    None,
                    shared.as_deref_mut(),
                    true,
                    false,
                )?);
                at_time.insert(sample.sample, Rc::clone(&tree));
                tree
            };
            trail.push(EchoSample {
                sample: TemporalSample {
                    time: sample.sample,
                    tree,
                },
                opacity: valle_draw::math::pow(decay, f64::from(lag)) as f32,
            });
        }
        out.insert(key.to_owned(), trail);
    }
    Ok(out)
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

/// Apply the derivative of each CSS `translate` before its fixed local rotation/scale. The parent
/// matrix's linear part carries the local velocity into screen space; the accumulated parent
/// velocity then carries ancestor translations to every descendant. Geometry is already laid out.
fn analytic_auto_blur_velocities(
    root: &RenderNode,
    layout: &LayoutResults,
    keys: &HashMap<u64, String>,
    translations: &HashMap<String, valle_draw::Point>,
    targets: &[&str],
    fps: f64,
) -> HashMap<String, valle_draw::Point> {
    let target_keys: std::collections::HashSet<_> = targets.iter().copied().collect();
    let mut velocities = HashMap::new();
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
            let velocity = valle_draw::Point::new(screen.x / fps, screen.y / fps);
            if target_keys.contains(key.as_str())
                && velocity.x.is_finite()
                && velocity.y.is_finite()
                && velocity.x.abs() <= 2048.0
                && velocity.y.abs() <= 2048.0
            {
                velocities.insert(key.clone(), velocity);
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
    velocities
}

/// Sample final screen positions in output time. The regular layout path is reused for the two
/// neighbors, so flow layout, ancestor transforms, and camera movement contribute to velocity.
/// Failed or discontinuous neighbors suppress blur without changing the requested main frame.
fn auto_blur_velocities(
    prepared: &PreparedScene,
    ctx: &valle_motion::MotionContext,
    props: &ResolvedProps,
    opts: &LayoutOptions<'_>,
    root: &RenderNode,
    layout: &Arc<LayoutResults>,
    keys: &Arc<HashMap<u64, String>>,
    css_3d_planes: &HashMap<String, crate::layout::bridge::Css3dPlane>,
    mut shared: Option<&mut RequestValueCache>,
) -> (HashMap<String, valle_draw::Point>, u8) {
    let targets = prepared
        .dependencies
        .auto_blur_nodes()
        .iter()
        .map(|id| prepared.artifact.nodes[id.0 as usize].key.as_str())
        .collect::<Vec<_>>();
    if targets.is_empty() {
        return (HashMap::new(), 0);
    }
    if let Some(translations) = prepared.auto_blur_translations.as_ref()
        && css_3d_planes.is_empty()
    {
        return (
            analytic_auto_blur_velocities(
                root,
                layout,
                keys,
                translations,
                &targets,
                crate::frame_rate_as_f64(ctx.fps),
            ),
            0,
        );
    }
    let Some(offset) = prepared.dependencies.auto_blur_offset(ctx.fps) else {
        return (HashMap::new(), 0);
    };
    let previous_context = offset
        .checked_neg()
        .ok()
        .and_then(|negative| ctx.sample.checked_offset(negative).ok())
        .and_then(|sample| crate::motion_context_at_sample(sample, ctx.duration_frames, ctx.fps));
    let next_context =
        ctx.sample.checked_offset(offset).ok().and_then(|sample| {
            crate::motion_context_at_sample(sample, ctx.duration_frames, ctx.fps)
        });
    let mut sample_anchors = |sample: Option<valle_motion::MotionContext>| {
        sample.map(|sample| {
            build_tree_inner(
                prepared.sample_scene.as_deref().unwrap_or(prepared),
                &sample,
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
            .map(|anchors| (sample.sample.composition().as_f64(), anchors))
        })
    };
    let previous = sample_anchors(previous_context).flatten();
    let next = sample_anchors(next_context).flatten();
    let center = screen_anchors(root, layout, keys, css_3d_planes);
    let center_time = ctx.sample.composition().as_f64();
    let fps = crate::frame_rate_as_f64(ctx.fps);
    let mut velocities = HashMap::new();
    for key in targets {
        let Some(current) = center.get(key) else {
            continue;
        };
        let before = previous.as_ref().and_then(|(_, anchors)| anchors.get(key));
        let after = next.as_ref().and_then(|(_, anchors)| anchors.get(key));
        // A missing node or a failed layout at a valid neighbor is a temporal cut.
        if previous_context.is_some() && before.is_none()
            || next_context.is_some() && after.is_none()
        {
            continue;
        }
        let derivative = |from: &valle_draw::Point, to: &valle_draw::Point, seconds: f64| {
            valle_draw::Point::new(
                (to.x - from.x) / seconds / fps,
                (to.y - from.y) / seconds / fps,
            )
        };
        let velocity = match (before, after) {
            (Some(before), Some(after)) => {
                let before_time = previous.as_ref().unwrap().0;
                let after_time = next.as_ref().unwrap().0;
                let left = derivative(before, current, center_time - before_time);
                let right = derivative(current, after, after_time - center_time);
                let jump = valle_draw::math::hypot(left.x - right.x, left.y - right.y);
                let speed = valle_draw::math::hypot(left.x, left.y)
                    .max(valle_draw::math::hypot(right.x, right.y));
                if jump > 2.0_f64.max(speed * 0.2) {
                    continue;
                }
                derivative(before, after, after_time - before_time)
            }
            (Some(before), None) => {
                derivative(before, current, center_time - previous.as_ref().unwrap().0)
            }
            (None, Some(after)) => {
                derivative(current, after, next.as_ref().unwrap().0 - center_time)
            }
            (None, None) => continue,
        };
        if velocity.x.is_finite()
            && velocity.y.is_finite()
            && velocity.x.abs() <= 2048.0
            && velocity.y.abs() <= 2048.0
        {
            velocities.insert(key.to_owned(), velocity);
        }
    }
    (
        velocities,
        u8::from(previous_context.is_some()) + u8::from(next_context.is_some()),
    )
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
            let point = if let Some(plane) = css_3d_planes.get(key) {
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
                let plane = self.css_3d_planes.get(key);
                let corners = plane.map_or_else(
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
                );
                let position = if plane.is_some() {
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
                    .map_or(true, |(viewport_width, viewport_height)| {
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
                            painted: node
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
                                    *width > 0.0
                                        && color.resolve(node.context.current_color).0[3] != 0
                                }),
                            position,
                            bounds: [left, top, right, bottom],
                            visible: node.context.style.visibility == Visibility::Visible
                                && opacity > 0.0
                                && !plane.is_some_and(|plane| plane.hidden),
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

fn resolve_advanced_filters(
    node: &valle_motion::SceneNode,
    values: &[MotionValue],
    at: usize,
    auto_velocity: Option<valle_draw::Point>,
) -> Result<Vec<valle_draw::program::recording::FilterOp>, LayoutError> {
    use valle_draw::program::recording::FilterOp;

    let mut filters = Vec::new();
    if node
        .styles
        .iter()
        .any(|style| style.property.starts_with("motion-bloom-"))
    {
        let mut params = [0.0; 4];
        for (index, (property, max)) in [
            ("motion-bloom-threshold", 1.0),
            ("motion-bloom-knee", 1.0),
            ("motion-bloom-intensity", 4.0),
            ("motion-bloom-radius", 128.0),
        ]
        .into_iter()
        .enumerate()
        {
            let Some(MotionValue::Number(value)) = resolved_style(node, values, at, property)?
            else {
                return Err(bad_advanced_filter(
                    node,
                    "bloom parameter is missing or invalid",
                ));
            };
            if !value.is_finite() || !(0.0..=max).contains(value) {
                return Err(bad_advanced_filter(
                    node,
                    "bloom parameter is out of bounds",
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
        let velocity = if is_auto {
            let Some(velocity) = auto_velocity else {
                return Ok(filters);
            };
            velocity
        } else {
            let Some(MotionValue::Point(velocity)) =
                resolved_style(node, values, at, "motion-velocity-blur-velocity")?
            else {
                return Err(bad_advanced_filter(
                    node,
                    "motion blur velocity is missing or is not point(x, y)",
                ));
            };
            *velocity
        };
        if !velocity.x.is_finite()
            || !velocity.y.is_finite()
            || velocity.x.abs() > 2048.0
            || velocity.y.abs() > 2048.0
            || !shutter_angle.is_finite()
            || !(0.0..=360.0).contains(shutter_angle)
        {
            return Err(bad_advanced_filter(
                node,
                "motion blur requires velocity components within ±2048 px/frame and shutterAngle 0..=360",
            ));
        }
        if is_auto && velocity.x.abs() < 0.001 && velocity.y.abs() < 0.001 {
            return Ok(filters);
        }
        filters.push(FilterOp::VelocityBlur {
            velocity_x: velocity.x,
            velocity_y: velocity.y,
            shutter_angle: *shutter_angle,
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
                if let StyleValue::Static {
                    value: MotionValue::Number(n),
                } = &binding.value
                {
                    style.font_size = *n;
                } else if let StyleValue::Expr { expr } = &binding.value {
                    if let MotionValue::Number(n) = value(values, *expr, at)? {
                        style.font_size = *n;
                    }
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

fn node_of(
    prepared: &PreparedScene,
    at: usize,
    values: &[MotionValue],
    cache: Option<&crate::StyleCache>,
    camera: Option<&CameraWrappers>,
    formulas: &HashMap<String, crate::math_formula::FormulaFragment>,
    css_3d_planes: Option<&HashMap<String, crate::layout::bridge::Css3dPlane>>,
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
        css_3d_planes,
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
        node = node.with_style(parse_style(cache, &row.declarations).map_err(|reason| {
            LayoutError::BadStyle {
                node: row.key.clone(),
                declarations: row.declarations.clone(),
                reason,
            }
        })?);
    }
    Ok(node)
}

/// Project one Artifact node into zero or one Takumi boxes. GlassField is a semantic material
/// scope, not a CSS box, so it contributes its projected children directly to the parent.
fn projected_nodes_of(
    prepared: &PreparedScene,
    at: usize,
    values: &[MotionValue],
    cache: Option<&crate::StyleCache>,
    camera: Option<&CameraWrappers>,
    formulas: &HashMap<String, crate::math_formula::FormulaFragment>,
    css_3d_planes: Option<&HashMap<String, crate::layout::bridge::Css3dPlane>>,
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
                css_3d_planes,
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
        NodeKind::Scene3D { .. } => Node::image(format!("generated://scene3d/{}", template.key)),
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
    let is_mask_source = artifact.nodes.iter().any(|node| {
        matches!(&node.kind, NodeKind::Mask { source: MaskValue::Subtree { source }, .. } if source == &template.key)
    });
    let is_transition_child = artifact.nodes.iter().any(|node| {
        matches!(node.kind, NodeKind::Transition { .. })
            && artifact.node_children[node.children.start as usize..node.children.end as usize]
                .iter()
                .any(|child| child.0 as usize == at)
    });
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
        declarations.push_str("; isolation: isolate !important");
    }
    if let Some(plane) = css_3d_planes.and_then(|planes| planes.get(&template.key)) {
        if plane.hidden {
            if !declarations.is_empty() {
                declarations.push_str("; ");
            }
            declarations.push_str("visibility: hidden");
        }
    }
    if let NodeKind::MathFormula { .. } = &template.kind {
        if let Some(fragment) = formulas.get(&template.key) {
            let total_h = (fragment.height + fragment.depth).max(0.0);
            let size = format!(
                "width: {}px; height: {}px",
                fragment.width.max(0.0),
                total_h
            );
            declarations = if declarations.is_empty() {
                size
            } else {
                format!("{declarations}; {size}")
            };
        }
    }
    if !declarations.is_empty() {
        node = node.with_style(parse_style(cache, &declarations).map_err(|reason| {
            LayoutError::BadStyle {
                node: template.key.clone(),
                declarations: declarations.clone(),
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
) -> Result<String, LayoutError> {
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
    for style in &node.styles {
        // Author `--*` is rejected at admission; skip it here so it never enters the cascade.
        if style.property.starts_with("--")
            || style.property == "motion-filter-frame"
            || style.property.starts_with("motion-displacement-")
            || style.property.starts_with("motion-velocity-blur-")
            || style.property == "motion-chromatic-aberration-offset"
            || style.property.starts_with("motion-glow-")
            || style.property.starts_with("motion-bloom-")
            || style.property.starts_with("motion-radial-blur-")
            || style.property.starts_with("motion-film-grain-")
            || style.property.starts_with("motion-lens-distortion-")
            || style.property.starts_with("motion-transform-3d-")
            || style.property.starts_with("motion-perspective-origin-")
        {
            continue;
        }
        if matches!(
            style.property.as_str(),
            "motion-path-anchor"
                | "mix-blend-space"
                | "motion-inline-image"
                | "motion-path-angle-offset"
                | "rotate-x"
                | "rotate-y"
                | "perspective"
                | "transform-style"
                | "backface-visibility"
                | "paper-grain"
                | "contact-shadow"
        ) {
            continue;
        }
        let value = match &style.value {
            StyleValue::Static { value } => value,
            StyleValue::Expr { expr } => value(values, *expr, at)?,
        };
        if style.property == "filter" {
            // Preserve the numeric parameter diagnostic before the generic CSS
            // parser wraps the entire declaration block into one error.
            if let Err(reason) = crate::style::advanced_filter::parse(&css_token(value)) {
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
    }
    if !visible {
        if !declarations.is_empty() {
            declarations.push_str("; ");
        }
        declarations.push_str("visibility: hidden");
    }
    Ok(declarations)
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

/// Takumi stores device extents; expressions and media queries use CSS pixels. Native/Wasm
/// hosts lay out at DPR 1 and apply delivery scaling later. Low-level callers may supply a
/// different DPR, so convert here too. Missing dimensions must not produce guessed values.
fn eval_viewport(opts: &LayoutOptions<'_>) -> Option<(f64, f64)> {
    let width = opts.viewport.size.width?;
    let height = opts.viewport.size.height?;
    let dpr = f64::from(opts.viewport.device_pixel_ratio);
    Some((f64::from(width) / dpr, f64::from(height) / dpr))
}

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
    }

    // A rich Text is an inline Group of styled Text runs. Join their authored source in child
    // order before splitting, so ctx.unit.index/count/start/end address the whole Text and a
    // word or line crossing a Span boundary remains one animation unit.
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
                            start,
                            end,
                        },
                    ));
                }
            }
        } else {
            let source = text_value(text, values, at)?;
            let units = unit_boundaries(&source, per_unit.split);
            let count = units.len() as u32;
            boundaries.extend(units.into_iter().enumerate().map(|(index, (start, end))| {
                (
                    start,
                    end,
                    valle_motion::UnitContext {
                        index: index as u32,
                        count,
                        start,
                        end,
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
    callback: &mut dyn FnMut(Option<usize>, &str, takumi_core::geometry::NodeId, (f32, f32)),
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
    let Some(camera) = &artifact.camera else {
        return false;
    };
    let is_dependent = |expr: valle_motion::ExprId| plan.bounds_dependent(expr);
    let point_dependent = match &camera.center {
        PointValue::Static { .. } => false,
        PointValue::Expr { expr } => is_dependent(*expr),
    };
    let number_dependent = |binding: &NumberValue| match binding {
        NumberValue::Static { .. } => false,
        NumberValue::Expr { expr } => is_dependent(*expr),
    };
    point_dependent || number_dependent(&camera.zoom) || number_dependent(&camera.rotation)
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
    tree.compute_layout(render_context.sizing.viewport.into());
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
            declarations(&node, &[], true, 0).unwrap(),
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
