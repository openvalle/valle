//! Motion JSX Scene Artifact → shared Takumi layout tree.
//!
//! This is a direct bridge: it evaluates the Motion expression arena and creates Takumi nodes from
//! [`SceneArtifact`]. The backend-neutral [`crate::emit`] then shapes text and emits the same [`valle_draw::program::recording::ProgramRecording`]
//! consumed by Native and CanvasKit executors.

use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use std::str::FromStr;
use std::sync::Arc;

use takumi_core::layout::node::Node;
use takumi_core::layout::tree::{LayoutResults, RenderNode};
use takumi_core::scene::{PaintItemKind, StackingContextNode, build_stacking_contexts};
use takumi_core::style::{
    Affine as TAffine, Background, BackgroundImage, BackgroundImages, ComputedStyle, FromCssStr,
    SizingContext,
};
use valle_motion::MotionValue;
use valle_motion::value::{Length, Length2, LengthUnit};
use valle_motion::{
    BatchPositions, BoolValue, ColorValue, CoordinateSpace, EvalError, EvalInputs, Expr, ExprId,
    GeometryBatchGeometry, GradientStopValue, MaskValue, NodeId, NodeKind, NumberValue, PaintValue,
    PathData, PathValue, PointValue, RectValue, ResolvedProps, SceneArtifact, ShaderUniformValue,
    StyleValue, TextSplit, TextValue, ValidationError, css_token, eval_all,
};

use crate::layout::bridge::{
    GlassLayoutEnvironment, GlassLayoutField, GlassLayoutForeground, GlassLayoutFrame,
    GlassLayoutMaterial, GlassLayoutMotion, GlassLayoutSurface, LayoutOptions, LayoutTree,
    ResolvedUnit, parse_style,
};

/// Fail-closed failures from Scene Artifact evaluation or layout construction.
#[derive(Debug, Clone, PartialEq)]
pub enum LayoutError {
    InvalidArtifact(Vec<ValidationError>),
    Eval(EvalError),
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
    BadScene3D {
        node: String,
        reason: String,
    },
    BadFormula {
        node: String,
        reason: String,
    },
    /// Reject paint surfaces that still use platform transcendental math instead of Valle's
    /// deterministic implementation.
    NondeterministicSurface {
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
            LayoutError::BadNode { at } => write!(f, "node {at}: out of range"),
            LayoutError::BadViewport => f.write_str("viewport must have non-zero size"),
            LayoutError::BadStyle {
                node,
                declarations,
                reason,
            } => write!(f, "node `{node}`: bad CSS {declarations:?} ({reason})"),
            LayoutError::BadPaint { node, reason } => {
                write!(f, "node `{node}`: bad paint ({reason})")
            }
            LayoutError::BadMask { node, reason } => {
                write!(f, "node `{node}`: bad mask ({reason})")
            }
            LayoutError::BadScene3D { node, reason } => {
                write!(f, "node `{node}`: bad Scene3D frame ({reason})")
            }
            LayoutError::BadFormula { node, reason } => {
                write!(f, "node `{node}`: bad MathFormula ({reason})")
            }
            LayoutError::NondeterministicSurface { node, surface } => write!(
                f,
                "node `{node}` uses `{surface}`, which is outside the deterministic paint surface"
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
}

impl PreparedScene {
    pub fn artifact(&self) -> &SceneArtifact {
        &self.artifact
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
    admit_deterministic_surface(&artifact)?;
    Ok(PreparedScene {
        artifact: Arc::new(artifact),
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
    signals: &valle_motion::ResolvedSignals,
    opts: &LayoutOptions<'_>,
) -> Result<LayoutTree, LayoutError> {
    let artifact = prepared.artifact();
    if opts.viewport.size.width == Some(0) || opts.viewport.size.height == Some(0) {
        return Err(LayoutError::BadViewport);
    }

    let values = eval_all(
        artifact,
        EvalInputs {
            ctx,
            props,
            signals,
            // The base pass has no unit context; unit expressions use a separate evaluation pass.
            unit: None,
            viewport: eval_viewport(opts),
        },
    )?;
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
    let camera = if camera_depends_on_bounds(artifact) {
        let probe = identity_camera_wrappers(opts)?;
        let probe_node = node_of(
            artifact,
            artifact.root.0 as usize,
            &values,
            opts.styles,
            Some(&probe),
            &formulas,
            None,
        )?;
        let probe_boxes = layout_and_collect_boxes(artifact, probe_node, opts)?;
        let probe_values = valle_motion::eval_layout_bounds(
            artifact,
            &values,
            EvalInputs {
                ctx,
                props,
                signals,
                unit: None,
                viewport: eval_viewport(opts),
            },
            &probe_boxes,
        )?;
        camera_wrappers(artifact, &probe_values, opts)?
    } else {
        camera_wrappers(artifact, &values, opts)?
    };
    let node = node_of(
        artifact,
        artifact.root.0 as usize,
        &values,
        opts.styles,
        camera.as_ref(),
        &formulas,
        None,
    )?;
    let render_context = takumi_core::context::RenderContext::builder()
        .fonts(opts.fonts.snapshot())
        .sizing(SizingContext::builder().viewport(opts.viewport).build())
        .images(Rc::new(Default::default()))
        .stylesheet(Default::default())
        .time_ms(LayoutOptions::TIME_MS)
        .draw_debug_border(false)
        .style(Box::new(ComputedStyle::default()))
        .build();
    let layout_root = RenderNode::from_node(&render_context, node);
    let mut tree = takumi_core::layout::tree::LayoutTree::from_render_node(&layout_root);
    tree.compute_layout(render_context.sizing.viewport.into());
    let layout = Rc::new(tree.into_results());

    let mut keys = HashMap::new();
    // Collect Scene keys and layout boxes during the same traversal for post-layout evaluation.
    let mut boxes = BTreeMap::new();
    let mut scene3d_content_boxes = BTreeMap::new();
    walk_pairs(
        artifact,
        artifact.root,
        &layout,
        takumi_core::geometry::NodeId::ROOT,
        (0.0, 0.0),
        &mut |at, id, origin| {
            if let Some(node) = artifact.nodes.get(at) {
                keys.insert(u64::from(id), node.key.clone());
                if let Ok(computed) = layout.layout(id) {
                    boxes.insert(
                        node.key.clone(),
                        valle_draw::Rect::new(
                            f64::from(origin.0),
                            f64::from(origin.1),
                            f64::from(computed.size.width),
                            f64::from(computed.size.height),
                        ),
                    );
                    if matches!(node.kind, NodeKind::Scene3D { .. }) {
                        scene3d_content_boxes.insert(
                            node.key.clone(),
                            valle_draw::Rect::new(
                                f64::from(origin.0 + computed.border.left + computed.padding.left),
                                f64::from(origin.1 + computed.border.top + computed.padding.top),
                                f64::from(computed.content_box_width().max(0.0)),
                                f64::from(computed.content_box_height().max(0.0)),
                            ),
                        );
                    }
                }
            }
        },
    )?;

    // The second evaluation pass replaces placeholders for bounds and their dependents, leaving
    // other expressions unchanged.
    //
    // Evaluate bounds after layout and before resolving paths, clips, masks, and text paths.
    let bounds_values = valle_motion::eval_layout_bounds(
        artifact,
        &values,
        EvalInputs {
            ctx,
            props,
            signals,
            unit: None,
            viewport: eval_viewport(opts),
        },
        &boxes,
    )?;
    let scene3d = resolve_scene3d_requests(artifact, &bounds_values)?;
    let projected = project_scene3d_anchors(artifact, &scene3d, &scene3d_content_boxes)?;
    let values = valle_motion::eval_post_layout(
        artifact,
        &bounds_values,
        EvalInputs {
            ctx,
            props,
            signals,
            unit: None,
            viewport: eval_viewport(opts),
        },
        &boxes,
        &projected,
    )?;
    check_post_layout_origins(artifact, &values, &boxes)?;
    let css_3d_planes = resolve_css_3d_planes(artifact, &values, &boxes, opts)?;

    // Post-layout values are admitted only into paint/transform slots or sidecar geometry. Rebuild
    // the same fixed-topology RenderNode with their final values, but keep the already-computed
    // LayoutResults. This makes `translate: project3d(...)` visible without a layout feedback pass.
    let final_node = node_of(
        artifact,
        artifact.root.0 as usize,
        &values,
        opts.styles,
        camera.as_ref(),
        &formulas,
        Some(&css_3d_planes),
    )?;
    let root = RenderNode::from_node(&render_context, final_node);

    // Map render paths to Scene keys and retain original text to recover node-local offsets from
    // concatenated inline text. Layout keys cannot identify inline nodes that have no box.
    let mut render_keys = HashMap::new();
    walk_render_keys(
        artifact,
        artifact.root,
        &root,
        &mut Vec::new(),
        &mut render_keys,
    );
    let units = resolve_units(
        artifact,
        &values,
        ctx,
        props,
        signals,
        &boxes,
        &projected,
        eval_viewport(opts),
    )?;
    let node_texts = artifact
        .nodes
        .iter()
        .enumerate()
        .filter_map(|(at, node)| match &node.kind {
            NodeKind::Text { text, .. } => {
                Some((node.key.clone(), text_value(text, &values, at).ok()?))
            }
            _ => None,
        })
        .collect();

    let mut paths = HashMap::new();
    let mut batches = HashMap::new();
    let mut text_paths = HashMap::new();
    let mut clips = HashMap::new();
    let mut masks = HashMap::new();
    let mut videos = HashMap::new();
    let mut advanced_filters = HashMap::new();
    let mut backdrop_advanced_filters = HashMap::new();
    let mut shaders = HashMap::new();
    let mut layer_fx = HashMap::new();
    let scene3d = scene3d;
    for (at, node) in artifact.nodes.iter().enumerate() {
        let filters = resolve_advanced_filters(node, &values, at)?;
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
                            ShaderUniformValue::Color { value } => {
                                let value = color_value(value, &values, at)?;
                                valle_draw::program::recording::ShaderUniformValue::Color {
                                    value: [
                                        f32::from(value.r) / 255.0,
                                        f32::from(value.g) / 255.0,
                                        f32::from(value.b) / 255.0,
                                        f32::from(value.a) / 255.0,
                                    ],
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
                            uri: program.uri.clone(),
                            content_hash: valle_draw::requirements::DigestBytes::from_bytes(
                                *program.content_hash.as_bytes(),
                            ),
                            abi_hash: valle_draw::requirements::DigestBytes::from_bytes(
                                *program.abi_hash.as_bytes(),
                            ),
                        },
                        uniforms,
                        inputs: inputs
                            .iter()
                            .map(|input| (input.name.clone(), input.source.clone()))
                            .collect(),
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
                batches.insert(
                    node.key.clone(),
                    crate::layout::bridge::BatchContent {
                        geometry: match batch.geometry {
                            GeometryBatchGeometry::Circle => {
                                valle_draw::program::recording::BatchGeometry::Circle
                            }
                            GeometryBatchGeometry::Rect => {
                                valle_draw::program::recording::BatchGeometry::Rect
                            }
                        },
                        instances: valle_motion::resolve_geometry_batch(
                            batch,
                            frame,
                            crate::frame_rate_as_f64(ctx.fps),
                            valle_motion::BatchFieldProgress {
                                position: position_progress,
                                size: size_progress,
                                fill: fill_progress,
                                opacity: opacity_progress,
                            },
                        ),
                    },
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
                // Motion video source time is a render-domain projection after the exact local
                // sample has been chosen. It does not call the removed Timeline 1.x f64 helper.
                let local_seconds = f64::from(ctx.local_frame) / crate::frame_rate_as_f64(ctx.fps);
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
    )?;

    Ok(LayoutTree {
        root,
        layout,
        values,
        keys: Rc::new(keys),
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
        shaders: Rc::new(shaders),
        scene3d: Rc::new(scene3d),
        layer_fx: Rc::new(layer_fx),
        css_3d_planes: Rc::new(css_3d_planes),
        formulas: Rc::new(formulas),
        glass,
    })
}

fn resolve_glass_layout(
    artifact: &SceneArtifact,
    values: &[MotionValue],
    root: &RenderNode,
    layout: &Rc<LayoutResults>,
    keys: &HashMap<u64, String>,
    boxes: &BTreeMap<String, valle_draw::Rect>,
    css_3d_planes: &HashMap<String, crate::layout::bridge::Css3dPlane>,
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
                foreground_auto_luma(artifact, at, values, boxes)?.ok_or_else(|| {
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
        if foreground_node_paints(node)
            && let Some(layout) = boxes.get(&node.key)
        {
            let viewport_points = if let Some(transform) = transforms.get(&node.key) {
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
                | NodeKind::Image { .. }
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
            colors.push(*color);
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
            colors.extend(batch.fills.iter().copied());
            if let Some(field) = &batch.fill_field {
                colors.extend(field.to.iter().copied());
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
        crate::layout::bridge::ResolvedPaint::Solid(color) => vec![color],
        crate::layout::bridge::ResolvedPaint::Linear { stops, .. }
        | crate::layout::bridge::ResolvedPaint::Radial { stops, .. }
        | crate::layout::bridge::ResolvedPaint::Conic { stops, .. } => {
            stops.into_iter().map(|(_, color)| color).collect()
        }
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
        let resolved = valle_motion::scene3d::Frame3DState {
            camera: valle_motion::scene3d::CameraFrameState {
                orbit_yaw_degrees: scalar(
                    &frame.camera.orbit_yaw_degrees,
                    "scene3d camera orbit yaw",
                )?,
                orbit_pitch_degrees: scalar(
                    &frame.camera.orbit_pitch_degrees,
                    "scene3d camera orbit pitch",
                )?,
                distance: scalar(&frame.camera.distance, "scene3d camera distance")?,
                fov_y_degrees: scalar(&frame.camera.fov_y_degrees, "scene3d camera fov")?,
            },
            meshes: frame
                .meshes
                .iter()
                .map(|mesh| {
                    Ok(valle_motion::scene3d::MeshFrameState {
                        key: mesh.key.clone(),
                        translation_x: scalar(&mesh.translation_x, "scene3d mesh translation x")?,
                        translation_y: scalar(&mesh.translation_y, "scene3d mesh translation y")?,
                        translation_z: scalar(&mesh.translation_z, "scene3d mesh translation z")?,
                        rotation_x_degrees: scalar(
                            &mesh.rotation_x_degrees,
                            "scene3d mesh rotation x",
                        )?,
                        rotation_y_degrees: scalar(
                            &mesh.rotation_y_degrees,
                            "scene3d mesh rotation y",
                        )?,
                        rotation_z_degrees: scalar(
                            &mesh.rotation_z_degrees,
                            "scene3d mesh rotation z",
                        )?,
                        scale_x: scalar(&mesh.scale_x, "scene3d mesh scale x")?,
                        scale_y: scalar(&mesh.scale_y, "scene3d mesh scale y")?,
                        scale_z: scalar(&mesh.scale_z, "scene3d mesh scale z")?,
                    })
                })
                .collect::<Result<Vec<_>, LayoutError>>()?,
            light_intensities: frame
                .light_intensities
                .iter()
                .map(|value| scalar(value, "scene3d light intensity"))
                .collect::<Result<Vec<_>, LayoutError>>()?,
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
            let rem = sizing
                .root_font_size
                .map(f64::from)
                .unwrap_or(f64::from(sizing.viewport.font_size) * dpr);
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
    transform_origin: Option<Length2>,
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
            "transform-origin" => {
                out.transform_origin = Some(match value {
                    MotionValue::Length2(value) => *value,
                    _ => {
                        return Err(LayoutError::BadStyle {
                            node: node.key.clone(),
                            declarations: "transform-origin".into(),
                            reason: "CSS 3D transformOrigin must be a two-axis length".into(),
                        });
                    }
                })
            }
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
) -> Result<HashMap<String, crate::layout::bridge::Css3dPlane>, LayoutError> {
    let sizing = SizingContext::builder().viewport(opts.viewport).build();
    let mut out = HashMap::new();
    resolve_css_3d_node(
        artifact,
        artifact.root.0 as usize,
        values,
        boxes,
        &sizing,
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
    state: Css3dState,
    out: &mut HashMap<String, crate::layout::bridge::Css3dPlane>,
) -> Result<(), LayoutError> {
    let node = artifact.nodes.get(at).ok_or(LayoutError::BadNode { at })?;
    let style = css_3d_style(node, values, at)?;
    let rect = boxes.get(&node.key).copied().unwrap_or_default();
    let transform_origin = style.transform_origin.unwrap_or(Length2 {
        x: Length {
            value: 50.0,
            unit: LengthUnit::Percent,
        },
        y: Length {
            value: 50.0,
            unit: LengthUnit::Percent,
        },
    });
    let origin = valle_draw::Point::new(
        resolve_css_position(
            transform_origin.x,
            rect.x,
            rect.width,
            sizing,
            &node.key,
            "transformOrigin.x",
        )?,
        resolve_css_position(
            transform_origin.y,
            rect.y,
            rect.height,
            sizing,
            &node.key,
            "transformOrigin.y",
        )?,
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
            child_state,
            out,
        )?;
    }
    Ok(())
}

fn resolve_advanced_filters(
    node: &valle_motion::SceneNode,
    values: &[MotionValue],
    at: usize,
) -> Result<Vec<valle_draw::program::recording::FilterOp>, LayoutError> {
    use valle_draw::program::recording::FilterOp;

    let mut filters = Vec::new();
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
        let Some(MotionValue::Point(velocity)) =
            resolved_style(node, values, at, "motion-velocity-blur-velocity")?
        else {
            return Err(bad_advanced_filter(
                node,
                "motion blur velocity is missing or is not point(x, y)",
            ));
        };
        let Some(MotionValue::Number(shutter_angle)) =
            resolved_style(node, values, at, "motion-velocity-blur-shutter")?
        else {
            return Err(bad_advanced_filter(
                node,
                "motion blur shutter angle must be a number",
            ));
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
) -> Result<valle_draw::Rgba, LayoutError> {
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

fn gradient_stops(
    bindings: &[GradientStopValue],
    values: &[MotionValue],
    at: usize,
    node: &str,
) -> Result<Vec<(f64, valle_draw::Rgba)>, LayoutError> {
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
                    style.color = *c;
                }
            }
            _ => {}
        }
    }
    Ok(style)
}

fn node_of(
    artifact: &SceneArtifact,
    at: usize,
    values: &[MotionValue],
    cache: Option<&crate::StyleCache>,
    camera: Option<&CameraWrappers>,
    formulas: &HashMap<String, crate::math_formula::FormulaFragment>,
    css_3d_planes: Option<&HashMap<String, crate::layout::bridge::Css3dPlane>>,
) -> Result<Node, LayoutError> {
    let mut projected =
        projected_nodes_of(artifact, at, values, cache, camera, formulas, css_3d_planes)?;
    if projected.len() != 1 {
        return Err(LayoutError::BadNode { at });
    }
    Ok(projected.remove(0))
}

/// Project one Artifact node into zero or one Takumi boxes. GlassField is a semantic material
/// scope, not a CSS box, so it contributes its projected children directly to the parent.
fn projected_nodes_of(
    artifact: &SceneArtifact,
    at: usize,
    values: &[MotionValue],
    cache: Option<&crate::StyleCache>,
    camera: Option<&CameraWrappers>,
    formulas: &HashMap<String, crate::math_formula::FormulaFragment>,
    css_3d_planes: Option<&HashMap<String, crate::layout::bridge::Css3dPlane>>,
) -> Result<Vec<Node>, LayoutError> {
    let template = artifact.nodes.get(at).ok_or(LayoutError::BadNode { at })?;
    let child_range = template.children.start as usize..template.children.end as usize;
    let child_ids = artifact
        .node_children
        .get(child_range)
        .ok_or(LayoutError::BadNode { at })?;
    let child_groups = child_ids
        .iter()
        .map(|child| {
            projected_nodes_of(
                artifact,
                child.0 as usize,
                values,
                cache,
                camera,
                formulas,
                css_3d_planes,
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

    if matches!(template.kind, NodeKind::GlassField(_)) {
        return Ok(children);
    }

    let mut node = match &template.kind {
        NodeKind::Group
        | NodeKind::Box
        | NodeKind::Clip { .. }
        | NodeKind::Mask { .. }
        | NodeKind::ShaderLayer { .. } => Node::container(children),
        NodeKind::Glass(_) => Node::container(children),
        NodeKind::GlassField(_) => unreachable!("GlassField is projected without a layout box"),
        NodeKind::Text { text, .. } => Node::text(text_value(text, values, at)?),
        NodeKind::Path { .. } | NodeKind::GeometryBatch { .. } => Node::container([]),
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
    if matches!(
        template.kind,
        NodeKind::Group
            | NodeKind::Box
            | NodeKind::Clip { .. }
            | NodeKind::Mask { .. }
            | NodeKind::Path { .. }
            | NodeKind::GeometryBatch { .. }
            | NodeKind::ShaderLayer { .. }
            | NodeKind::Glass(_)
            | NodeKind::Scene3D { .. }
    ) {
        let preset = "display: block";
        node = node.with_preset(parse_style(cache, preset).map_err(|reason| {
            LayoutError::BadStyle {
                node: template.key.clone(),
                declarations: preset.into(),
                reason,
            }
        })?);
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
    if matches!(
        template.kind,
        NodeKind::Clip { .. }
            | NodeKind::Mask { .. }
            | NodeKind::ShaderLayer { .. }
            | NodeKind::Glass(_)
    ) || is_mask_source
    {
        node = node.with_preset(parse_style(cache, "isolation: isolate").map_err(|reason| {
            LayoutError::BadStyle {
                node: template.key.clone(),
                declarations: "isolation: isolate".into(),
                reason,
            }
        })?);
    }
    // Advanced filters require a stacking context so BeginFilter encloses the full subtree.
    if template.styles.iter().any(|style| {
        style.property.starts_with("motion-displacement-")
            || style.property.starts_with("motion-velocity-blur-")
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
        node = node.with_preset(parse_style(cache, "isolation: isolate").map_err(|reason| {
            LayoutError::BadStyle {
                node: template.key.clone(),
                declarations: "isolation: isolate".into(),
                reason,
            }
        })?);
    }
    // Text along a path defaults to one line. Wrapping would restart glyph positions at the path
    // origin and overlap earlier glyphs.
    //
    // Use a preset so explicit author styles still win; emission reports unsupported multiline
    // text.
    if let NodeKind::MathFormula { .. } = &template.kind {
        let fragment = formulas
            .get(&template.key)
            .ok_or_else(|| LayoutError::BadFormula {
                node: template.key.clone(),
                reason: "formula fragment missing after prepare".into(),
            })?;
        let total_h = fragment.height + fragment.depth;
        // One preset: with_preset replaces the whole layer, so display/isolation
        // cannot be applied in earlier calls.
        let decl = format!(
            "display: block; isolation: isolate; width: {}px; height: {}px",
            fragment.width.max(0.0),
            total_h.max(0.0)
        );
        node = node.with_preset(parse_style(cache, &decl).map_err(|reason| {
            LayoutError::BadStyle {
                node: template.key.clone(),
                declarations: decl.clone(),
                reason,
            }
        })?);
    }
    if matches!(template.kind, NodeKind::Text { path: Some(_), .. }) {
        node = node.with_preset(parse_style(cache, "white-space: nowrap").map_err(|reason| {
            LayoutError::BadStyle {
                node: template.key.clone(),
                declarations: "white-space: nowrap".into(),
                reason,
            }
        })?);
    }

    if !template.class_names.is_empty() {
        let joined = template.class_names.join(" ");
        let tw = takumi_core::style::TailwindValues::from_str(&joined)
            .unwrap_or_else(|_| unreachable!("Takumi TailwindValues::from_str is infallible"));
        node = node.with_tw(tw).with_class_name(joined);
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
    if let Some(plane) = css_3d_planes.and_then(|planes| planes.get(&template.key)) {
        // `preserve-3d` needs a painter-order fallback when the author leaves stacking at auto.
        // An explicit z-index is stronger author intent and must not be overwritten by the
        // center-depth approximation: two parallel, laterally offset planes can exchange center
        // depth during a camera orbit even though their physical front/back order never changes.
        if !template
            .styles
            .iter()
            .any(|style| style.property == "z-index")
        {
            let z_index = (plane.depth * 1024.0)
                .round()
                .clamp(f64::from(i32::MIN + 1), f64::from(i32::MAX - 1))
                as i32;
            if !declarations.is_empty() {
                declarations.push_str("; ");
            }
            declarations.push_str(&format!("z-index: {z_index}"));
        }
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
        if style.property.starts_with("motion-displacement-")
            || style.property.starts_with("motion-velocity-blur-")
            || style.property.starts_with("motion-transform-3d-")
            || style.property.starts_with("motion-perspective-origin-")
        {
            continue;
        }
        if matches!(
            style.property.as_str(),
            "motion-path-anchor"
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
        if matches!(style.value, StyleValue::Expr { .. })
            && matches!(style.property.as_str(), "background" | "background-image")
            && !gradient_background_source(&style.property, &css_token(value))
        {
            return Err(LayoutError::NondeterministicSurface {
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
        } else {
            declarations.push_str(&css_token(value));
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

/// Expose the viewport only when both dimensions are available; missing dimensions must not produce
/// guessed values.
fn eval_viewport(opts: &LayoutOptions<'_>) -> Option<(f64, f64)> {
    let width = opts.viewport.size.width?;
    let height = opts.viewport.size.height?;
    Some((f64::from(width), f64::from(height)))
}

fn resolve_units(
    artifact: &SceneArtifact,
    values: &[MotionValue],
    ctx: &valle_motion::MotionContext,
    props: &ResolvedProps,
    signals: &valle_motion::ResolvedSignals,
    boxes: &BTreeMap<String, valle_draw::Rect>,
    projected: &BTreeMap<(String, String), valle_draw::Point>,
    viewport: Option<(f64, f64)>,
) -> Result<HashMap<String, Vec<ResolvedUnit>>, LayoutError> {
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
        let source = text_value(text, values, at)?;
        let boundaries = unit_boundaries(&source, per_unit.split);
        let count = boundaries.len() as u32;
        let style = &per_unit.style;
        let mut resolved = Vec::with_capacity(boundaries.len());
        for (index, (start, end)) in boundaries.into_iter().enumerate() {
            let unit = valle_motion::UnitContext {
                index: index as u32,
                count,
                start,
                end,
            };
            let values = valle_motion::eval_units(
                artifact,
                values,
                EvalInputs {
                    ctx,
                    props,
                    signals,
                    unit: None,
                    viewport,
                },
                unit,
                boxes,
                projected,
            )?;
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
) {
    let at = node_id.0 as usize;
    let Some(scene) = artifact.nodes.get(at) else {
        return;
    };
    out.insert(path.clone(), scene.key.clone());
    let Some(children) = node.children.as_deref() else {
        return;
    };
    let Some(child_ids) = projected_child_ids(artifact, node_id) else {
        return;
    };

    // Skip the camera wrappers, which have no Artifact counterparts. Admission guarantees direct
    // Screen children follow all World children.
    //
    // Apply the wrapper traversal rule here as well to preserve source identities for World text.
    if at == artifact.root.0 as usize && artifact.camera.is_some() {
        let mut world_ids = Vec::new();
        let mut screen_ids = Vec::new();
        for id in &child_ids {
            let is_screen = artifact
                .nodes
                .get(id.0 as usize)
                .is_some_and(|node| node.space == Some(CoordinateSpace::Screen));
            if is_screen {
                screen_ids.push(*id);
            } else {
                world_ids.push(*id);
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
                    );
                }
                path.pop();
            }
            path.pop();
        }
        // Screen children follow the camera wrapper, starting at render index 1.
        for (index, child) in children.iter().enumerate().skip(1) {
            let Some(id) = screen_ids.get(index - 1).copied() else {
                break;
            };
            path.push(index);
            walk_render_keys(artifact, id, child, path, out);
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
    );
}

/// Artifact children that actually own Takumi boxes. GlassField scopes are flattened exactly as
/// [`projected_nodes_of`] does; every other node remains one-to-one.
fn projected_child_ids(artifact: &SceneArtifact, owner: NodeId) -> Option<Vec<NodeId>> {
    fn append(artifact: &SceneArtifact, id: NodeId, out: &mut Vec<NodeId>) -> Option<()> {
        let node = artifact.nodes.get(id.0 as usize)?;
        if matches!(node.kind, NodeKind::GlassField(_)) {
            let range = node.children.start as usize..node.children.end as usize;
            for child in artifact.node_children.get(range)? {
                append(artifact, *child, out)?;
            }
        } else {
            out.push(id);
        }
        Some(())
    }

    let node = artifact.nodes.get(owner.0 as usize)?;
    let range = node.children.start as usize..node.children.end as usize;
    let mut out = Vec::new();
    for child in artifact.node_children.get(range)? {
        append(artifact, *child, &mut out)?;
    }
    Some(out)
}

/// Match render children to `child_ids`, handling anonymous boxes as in [`walk_render_keys`].
fn match_render_children(
    artifact: &SceneArtifact,
    children: &[RenderNode],
    child_ids: &[NodeId],
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
                match_render_children(artifact, grandchildren, child_ids, cursor, owner, path, out);
            }
        } else if let Some(id) = child_ids.get(*cursor) {
            walk_render_keys(artifact, *id, child, path, out);
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

fn walk_pairs(
    artifact: &SceneArtifact,
    node_id: NodeId,
    layout: &LayoutResults,
    layout_id: takumi_core::geometry::NodeId,
    origin: (f32, f32),
    callback: &mut dyn FnMut(usize, takumi_core::geometry::NodeId, (f32, f32)),
) -> Result<(), LayoutError> {
    let at = node_id.0 as usize;
    artifact.nodes.get(at).ok_or(LayoutError::BadNode { at })?;
    // Accumulate parent-relative layout offsets to obtain scene coordinates for bounds and anchors.
    let here = origin_of(layout, layout_id, origin);
    callback(at, layout_id, here);
    let children = projected_child_ids(artifact, node_id).ok_or(LayoutError::BadNode { at })?;
    let Ok(layout_children) = layout.box_children(layout_id) else {
        return Ok(());
    };
    // Skip the two camera wrappers when pairing trees. Admission guarantees Screen nodes are direct
    // root children after World content.
    if at == artifact.root.0 as usize && artifact.camera.is_some() {
        let world_count = children
            .iter()
            .filter(|child| {
                artifact
                    .nodes
                    .get(child.0 as usize)
                    .is_some_and(|node| node.space != Some(CoordinateSpace::Screen))
            })
            .count();
        for child in layout_children {
            if child.render_index == 0 {
                // Outer wrapper, inner wrapper, then World content.
                let Ok(outer_children) = layout.box_children(child.node_id) else {
                    continue;
                };
                let Some(inner) = outer_children.first() else {
                    continue;
                };
                let Ok(world) = layout.box_children(inner.node_id) else {
                    continue;
                };
                // Accumulate wrapper origins too; zero offsets are a style choice, not a traversal
                // invariant.
                let outer_origin = origin_of(layout, child.node_id, here);
                let inner_origin = origin_of(layout, inner.node_id, outer_origin);
                for item in world {
                    let Some(node_id) = children.get(item.render_index).copied() else {
                        continue;
                    };
                    walk_pairs(
                        artifact,
                        node_id,
                        layout,
                        item.node_id,
                        inner_origin,
                        callback,
                    )?;
                }
            } else if let Some(node_id) =
                children.get(world_count + child.render_index - 1).copied()
            {
                walk_pairs(artifact, node_id, layout, child.node_id, here, callback)?;
            }
        }
        return Ok(());
    }
    for child in layout_children {
        let Some(node_id) = children.get(child.render_index).copied() else {
            continue;
        };
        walk_pairs(artifact, node_id, layout, child.node_id, here, callback)?;
    }
    Ok(())
}

/// Absolute node origin equals the parent's absolute origin plus the relative layout location.
fn origin_of(
    layout: &LayoutResults,
    layout_id: takumi_core::geometry::NodeId,
    parent: (f32, f32),
) -> (f32, f32) {
    match layout.layout(layout_id) {
        Ok(computed) => (
            parent.0 + computed.location.x,
            parent.1 + computed.location.y,
        ),
        Err(_) => parent,
    }
}

/// Admit Takumi paint surfaces only when they avoid platform-dependent transcendental math.
fn admit_deterministic_surface(artifact: &SceneArtifact) -> Result<(), LayoutError> {
    const STYLE_DENY: &[&str] = &[
        "transform",
        "skew",
        "border-image",
        "offset",
        "offset-path",
        "clip-path",
        "mask",
        "mask-image",
    ];
    const CLASS_MARKERS: &[&str] = &[
        "transform",
        "translate-",
        "rotate-",
        "scale-",
        "skew-",
        "blur",
        "backdrop-",
        "filter",
        "clip-",
        "mask-",
    ];

    let expr_types =
        crate::expr::validate_exprs(&artifact.exprs, &artifact.controls, &mut Vec::new()).types;
    for node in &artifact.nodes {
        for style in &node.styles {
            if matches!(style.property.as_str(), "background" | "background-image")
                && !gradient_background_binding(style, artifact, &expr_types)
            {
                return Err(LayoutError::NondeterministicSurface {
                    node: node.key.clone(),
                    surface: style.property.clone(),
                });
            }
            if STYLE_DENY.contains(&style.property.as_str()) {
                return Err(LayoutError::NondeterministicSurface {
                    node: node.key.clone(),
                    surface: style.property.clone(),
                });
            }
        }
        for class_name in &node.class_names {
            if CLASS_MARKERS
                .iter()
                .any(|marker| class_name.contains(marker))
            {
                return Err(LayoutError::NondeterministicSurface {
                    node: node.key.clone(),
                    surface: format!("className:{class_name}"),
                });
            }
        }
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

/// Keep URLs and unsupported interpolation spaces closed for both static and dynamic CSS.
fn gradient_background_source(property: &str, source: &str) -> bool {
    // Takumi 0.23 assigns the same value to omitted interpolation and explicit Oklab.
    // Retain the source distinction: Valle's legacy gradients interpolate in sRGB.
    let mut legacy_defaults = legacy_gradient_defaults(source).into_iter();
    let srgb = takumi_core::style::ColorInterpolationMethod::from_css_str("in srgb")
        .expect("sRGB is a valid CSS interpolation method");
    let mut admitted = |image: &BackgroundImage| {
        let interpolation = match image {
            BackgroundImage::None => return true,
            BackgroundImage::Linear(gradient) => gradient.interpolation,
            BackgroundImage::Radial(gradient) => gradient.interpolation,
            BackgroundImage::Conic(gradient) => gradient.interpolation,
            BackgroundImage::Url(_) => return false,
        };
        let legacy = legacy_defaults.next().unwrap_or(false);
        interpolation == srgb
            || (legacy && interpolation == takumi_core::style::ColorInterpolationMethod::default())
    };
    match property {
        "background-image" => BackgroundImages::from_css_str(source)
            .is_ok_and(|images| !images.is_empty() && images.iter().all(admitted)),
        "background" => Background::from_css_str(source).is_ok_and(|value| admitted(&value.image)),
        _ => false,
    }
}

/// CSS tokens preserve escapes and comments when distinguishing omitted interpolation.
/// Takumi still parses and validates the complete value; this only classifies its gradients.
fn legacy_gradient_defaults(source: &str) -> Vec<bool> {
    use cssparser::{Parser, ParserInput, Token};
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    let mut defaults = Vec::new();
    while let Ok(token) = parser.next().cloned() {
        let Token::Function(name) = token else {
            continue;
        };
        if !matches!(
            name.to_ascii_lowercase().as_str(),
            "linear-gradient"
                | "radial-gradient"
                | "conic-gradient"
                | "repeating-linear-gradient"
                | "repeating-radial-gradient"
                | "repeating-conic-gradient"
        ) {
            continue;
        }
        let legacy = parser.parse_nested_block(|body| {
            let mut explicit = false;
            let mut modern = false;
            while let Ok(token) = body.next().cloned() {
                match token {
                    Token::Ident(ident) if ident.eq_ignore_ascii_case("in") => explicit = true,
                    Token::Function(name) => {
                        modern |= matches!(name.to_ascii_lowercase().as_str(),
                            "lab" | "lch" | "oklab" | "oklch" | "color" | "color-mix");
                        let relative = body.parse_nested_block(|args| {
                            let relative = matches!(args.next(), Ok(Token::Ident(ident)) if ident.eq_ignore_ascii_case("from"));
                            while args.next().is_ok() {}
                            Ok::<_, cssparser::ParseError<'_, ()>>(relative)
                        }).unwrap_or(false);
                        modern |= relative;
                    }
                    _ => {}
                }
            }
            Ok::<_, cssparser::ParseError<'_, ()>>(!explicit && !modern)
        }).unwrap_or(false);
        defaults.push(legacy);
    }
    defaults
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
fn camera_depends_on_bounds(artifact: &SceneArtifact) -> bool {
    let Some(camera) = &artifact.camera else {
        return false;
    };
    let dependent = valle_motion::bounds_dependent(&artifact.exprs);
    let is_dependent =
        |expr: valle_motion::ExprId| dependent.get(expr.0 as usize).copied().unwrap_or_default();
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
    artifact: &SceneArtifact,
    node: Node,
    opts: &LayoutOptions<'_>,
) -> Result<BTreeMap<String, valle_draw::Rect>, LayoutError> {
    let render_context = takumi_core::context::RenderContext::builder()
        .fonts(opts.fonts.snapshot())
        .sizing(SizingContext::builder().viewport(opts.viewport).build())
        .images(Rc::new(Default::default()))
        .stylesheet(Default::default())
        .time_ms(LayoutOptions::TIME_MS)
        .draw_debug_border(false)
        .style(Box::new(ComputedStyle::default()))
        .build();
    let root = RenderNode::from_node(&render_context, node);
    let mut tree = takumi_core::layout::tree::LayoutTree::from_render_node(&root);
    tree.compute_layout(render_context.sizing.viewport.into());
    let layout = tree.into_results();
    let mut boxes = BTreeMap::new();
    walk_pairs(
        artifact,
        artifact.root,
        &layout,
        takumi_core::geometry::NodeId::ROOT,
        (0.0, 0.0),
        &mut |at, id, origin| {
            if let (Some(node), Ok(computed)) = (artifact.nodes.get(at), layout.layout(id)) {
                boxes.insert(
                    node.key.clone(),
                    valle_draw::Rect::new(
                        f64::from(origin.0),
                        f64::from(origin.1),
                        f64::from(computed.size.width),
                        f64::from(computed.size.height),
                    ),
                );
            }
        },
    )?;
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
        artifact.root,
        &tree.layout,
        takumi_core::geometry::NodeId::ROOT,
        (0.0, 0.0),
        &mut |at, id, origin| {
            if let (Some(node), Ok(layout)) = (artifact.nodes.get(at), tree.layout.layout(id)) {
                boxes.insert(
                    node.key.clone(),
                    [origin.0, origin.1, layout.size.width, layout.size.height],
                );
            }
        },
    )?;
    Ok(boxes)
}

#[cfg(test)]
mod tests {
    use super::{declarations, resolve_layer_fx, unit_boundaries};
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
    fn motion_path_center_anchor_lowers_to_box_relative_translate() {
        let node = SceneNode {
            key: "follower".into(),
            kind: NodeKind::Box,
            space: None,
            class_names: vec![],
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
