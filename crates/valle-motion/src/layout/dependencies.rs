//! Load-time dependency facts shared by activation and future composition planning.
//!
//! References are conservative: every `bounds()` target stays in layout even when its
//! expression is currently unused. The graph also records the owning node for reachable
//! references, and identifies paint sources and composition boundaries independently of layout.

use std::collections::{BTreeMap, BTreeSet};
use valle_timeline::{FrameRate, RationalTime};

use crate::eval::EvalPlan;
use crate::tailwind::CompositionClassFacts;
use crate::{Expr, ExprId, MaskValue, NodeId, NodeKind, SceneArtifact};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SceneReferenceKind {
    Geometry,
    MaskPaint,
    GlassField,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceneReference {
    pub consumer: NodeId,
    pub source: NodeId,
    pub kind: SceneReferenceKind,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TimeRemap {
    offset_seconds: f64,
    speed: f64,
}

impl TimeRemap {
    fn apply(self, inherited: f64) -> f64 {
        (inherited - self.offset_seconds) * self.speed
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum TemporalSampling {
    Shutter {
        samples: u8,
        angle_degrees: u16,
    },
    Echo {
        count: u8,
        interval_frames: u32,
        decay: f64,
    },
}

impl TemporalSampling {
    fn frame_offsets(self) -> Vec<RationalTime> {
        match self {
            Self::Shutter {
                samples,
                angle_degrees,
            } if samples > 1 && angle_degrees > 0 => (0..samples)
                .map(|index| {
                    let numerator =
                        (i64::from(index) * 2 + 1 - i64::from(samples)) * i64::from(angle_degrees);
                    RationalTime::new(numerator, u32::from(samples) * 720)
                        .expect("validated shutter phase fits an exact rational")
                })
                .collect(),
            Self::Echo {
                count,
                interval_frames,
                decay,
            } if decay > 0.0 => (1..=count)
                .rev()
                .map(|lag| {
                    RationalTime::new(-i64::from(lag) * i64::from(interval_frames), 1)
                        .expect("validated echo offset fits an exact rational")
                })
                .collect(),
            _ => Vec::new(),
        }
    }
}

/// Conservative output-frame reach of all temporal effects in a prepared scene.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TemporalWindow {
    pub past_frames: RationalTime,
    pub future_frames: RationalTime,
}

impl Default for TemporalWindow {
    fn default() -> Self {
        Self {
            past_frames: RationalTime::ZERO,
            future_frames: RationalTime::ZERO,
        }
    }
}

impl TemporalWindow {
    fn include(&mut self, offset: RationalTime) {
        if offset.is_negative() {
            self.past_frames = self.past_frames.max(
                offset
                    .checked_neg()
                    .expect("validated sample offset can be negated"),
            );
        } else {
            self.future_frames = self.future_frames.max(offset);
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct TemporalNode {
    pub node: NodeId,
    pub sampling: TemporalSampling,
    /// Exact output-frame offsets, in paint order. Sampling uses these same values.
    pub offsets: Vec<RationalTime>,
}

#[derive(Debug, Clone, Default)]
pub struct NodeDependencies {
    /// Parent in the validated Scene tree; the root has none.
    pub parent: Option<NodeId>,
    /// All ancestors can contain an independent positioned leaf as an ordinary layout box.
    pub ordinary_container_path: bool,
    /// Some expression in the artifact reads this node's layout box.
    pub geometry_referenced: bool,
    /// This node is used as a separate paint source, currently a mask subtree.
    pub paint_referenced: bool,
    /// This node creates or may create an isolated paint/composition scope.
    pub composition_boundary: bool,
    /// This node may sample pixels drawn before its own layer.
    pub reads_backdrop: bool,
    /// A node or per-unit text expression changes with the sample.
    pub sample_dependent: bool,
    /// Structural time remapping for this node and all of its descendants.
    pub(crate) time_remap: Option<TimeRemap>,
    /// Temporal samples requested by this node; zero-strength effects request none.
    pub(crate) temporal_sampling: Option<TemporalSampling>,
}

#[derive(Debug, Clone)]
pub struct SceneDependencies {
    nodes: Vec<NodeDependencies>,
    references: Vec<SceneReference>,
    traversal: Vec<NodeId>,
    temporal_nodes: Vec<TemporalNode>,
    temporal_sample_count: usize,
    temporal_window: TemporalWindow,
    auto_blur_nodes: Vec<NodeId>,
}

impl SceneDependencies {
    /// The artifact must have passed topology and reference validation first.
    pub(crate) fn new(
        artifact: &SceneArtifact,
        plan: &EvalPlan,
        class_facts: &[CompositionClassFacts],
    ) -> Self {
        assert_eq!(artifact.nodes.len(), class_facts.len());
        let mut nodes = vec![NodeDependencies::default(); artifact.nodes.len()];
        let mut keys = artifact
            .nodes
            .iter()
            .enumerate()
            .map(|(at, node)| (node.key.clone(), at))
            .collect::<BTreeMap<_, _>>();
        let referenced_instance_keys = artifact
            .exprs
            .iter()
            .filter_map(|expr| match expr {
                Expr::NodeBounds { key } => Some(key.as_str()),
                _ => None,
            })
            .chain(artifact.nodes.iter().filter_map(|node| match &node.kind {
                NodeKind::Mask {
                    source: MaskValue::Subtree { source },
                    ..
                } => Some(source.as_str()),
                _ => None,
            }))
            .collect::<BTreeSet<_>>();
        for (at, node) in artifact.nodes.iter().enumerate() {
            if let NodeKind::InstanceLayout { group } = node.kind {
                if let Some(group) = artifact.instance_groups.get(group as usize) {
                    for row in 0..group.rows() {
                        if let Some(key) = group.keys.key_at(row) {
                            keys.insert(key, at);
                        }
                        if !referenced_instance_keys.is_empty() {
                            let mut stack = group.template_children.iter().collect::<Vec<_>>();
                            while let Some(child) = stack.pop() {
                                if let Some(key) = group.key_for_node(row, &child.node.key)
                                    && referenced_instance_keys.contains(key.as_str())
                                {
                                    keys.insert(key, at);
                                }
                                stack.extend(&child.children);
                            }
                        }
                    }
                }
            }
        }
        let mut stack = vec![(artifact.root.0 as usize, None, true)];
        let mut traversal = Vec::with_capacity(artifact.nodes.len());
        while let Some((at, parent, ordinary_path)) = stack.pop() {
            traversal.push(NodeId(at as u32));
            let node = &artifact.nodes[at];
            let facts = &mut nodes[at];
            facts.parent = parent.map(|at| NodeId(at as u32));
            facts.ordinary_container_path = ordinary_path;
            let classes = class_facts[at];
            facts.reads_backdrop = classes.reads_backdrop
                || matches!(node.kind, NodeKind::Glass(_) | NodeKind::GlassField(_))
                || node.styles.iter().any(|style| {
                    style.property == "mix-blend-mode"
                        || crate::style::property_spec(&style.property).reads_destination
                });
            facts.composition_boundary = classes.composition_boundary
                || facts.reads_backdrop
                || matches!(
                    node.kind,
                    NodeKind::Clip { .. }
                        | NodeKind::Mask { .. }
                        | NodeKind::Transition { .. }
                        | NodeKind::Shutter { .. }
                        | NodeKind::Echo { .. }
                        | NodeKind::ShaderLayer { .. }
                        | NodeKind::Glass(_)
                        | NodeKind::GlassField(_)
                )
                || node.styles.iter().any(|style| {
                    matches!(
                        style.property.as_str(),
                        "opacity" | "filter" | "backdrop-filter" | "mix-blend-mode" | "isolation"
                    )
                });
            facts.sample_dependent = node
                .expr_refs_outside_per_unit()
                .into_iter()
                .chain(node.per_unit_expr_refs())
                .any(|(_, id)| plan.sample_dependent(id));
            facts.time_remap = match node.kind {
                NodeKind::TimeScope {
                    offset_seconds,
                    speed,
                } => Some(TimeRemap {
                    offset_seconds,
                    speed,
                }),
                _ => None,
            };
            facts.temporal_sampling = match node.kind {
                NodeKind::Shutter {
                    samples,
                    angle_degrees,
                } => Some(TemporalSampling::Shutter {
                    samples,
                    angle_degrees,
                }),
                NodeKind::Echo {
                    count,
                    interval_frames,
                    decay,
                } => Some(TemporalSampling::Echo {
                    count,
                    interval_frames,
                    decay,
                }),
                _ => None,
            };
            // TimeScope contributes no layout box or paint. Its mapped expressions are already
            // in the artifact, so an independent absolute descendant remains eligible.
            let child_path = ordinary_path
                && matches!(
                    node.kind,
                    NodeKind::Group | NodeKind::Box | NodeKind::TimeScope { .. }
                );
            for child in
                &artifact.node_children[node.children.start as usize..node.children.end as usize]
            {
                stack.push((child.0 as usize, Some(at), child_path));
            }
        }

        // Camera and unused expressions have no ordinary node owner. Pin every named target
        // before building owner edges so those readers cannot lose layout geometry.
        for expr in &artifact.exprs {
            if let Expr::NodeBounds { key } = expr
                && let Some(&source) = keys.get(key.as_str())
            {
                nodes[source].geometry_referenced = true;
            }
        }

        let field_nodes = artifact
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(at, node)| match &node.kind {
                NodeKind::GlassField(field) => Some((field.field_id.as_str(), at)),
                _ => None,
            })
            .collect::<BTreeMap<_, _>>();
        let mut reference_ids = BTreeSet::new();
        for (consumer, node) in artifact.nodes.iter().enumerate() {
            let mut visited = BTreeSet::new();
            let mut todo = node
                .expr_refs_outside_per_unit()
                .into_iter()
                .chain(node.per_unit_expr_refs())
                .map(|(_, id)| id)
                .collect::<Vec<ExprId>>();
            while let Some(id) = todo.pop() {
                if !visited.insert(id) {
                    continue;
                }
                let expr = &artifact.exprs[id.0 as usize];
                if let Expr::NodeBounds { key } = expr
                    && let Some(&source) = keys.get(key.as_str())
                {
                    reference_ids.insert((consumer, source, 0u8));
                }
                todo.extend(expr.children());
            }
            if let NodeKind::Mask {
                source: MaskValue::Subtree { source },
                ..
            } = &node.kind
                && let Some(&source) = keys.get(source.as_str())
            {
                nodes[source].paint_referenced = true;
                reference_ids.insert((consumer, source, 1u8));
            }
            if let NodeKind::Glass(glass) = &node.kind
                && let Some(field_id) = &glass.field_id
                && let Some(&source) = field_nodes.get(field_id.as_str())
            {
                reference_ids.insert((consumer, source, 2u8));
            }
        }
        let references = reference_ids
            .into_iter()
            .map(|(consumer, source, kind)| SceneReference {
                consumer: NodeId(consumer as u32),
                source: NodeId(source as u32),
                kind: match kind {
                    0 => SceneReferenceKind::Geometry,
                    1 => SceneReferenceKind::MaskPaint,
                    2 => SceneReferenceKind::GlassField,
                    _ => unreachable!("reference kinds are constructed above"),
                },
            })
            .collect();
        let temporal_nodes = nodes
            .iter()
            .enumerate()
            .filter_map(|(at, facts)| {
                let sampling = facts.temporal_sampling?;
                let offsets = sampling.frame_offsets();
                (!offsets.is_empty()).then_some(TemporalNode {
                    node: NodeId(at as u32),
                    sampling,
                    offsets,
                })
            })
            .collect::<Vec<_>>();
        let temporal_sample_count = temporal_nodes.iter().map(|node| node.offsets.len()).sum();
        let mut temporal_window = TemporalWindow::default();
        for node in &temporal_nodes {
            for &offset in &node.offsets {
                temporal_window.include(offset);
            }
        }
        let auto_blur_nodes = artifact
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(at, node)| {
                node.styles
                    .iter()
                    .any(|style| style.property == "motion-velocity-blur-auto")
                    .then_some(NodeId(at as u32))
            })
            .collect::<Vec<_>>();
        if !auto_blur_nodes.is_empty() {
            let half_frame = RationalTime::new(1, 2).expect("half frame is exact");
            // Temporal sample layouts also evaluate automatic blur, so its two neighbors can
            // extend either side of every Shutter/Echo offset by another half frame.
            temporal_window.past_frames = temporal_window
                .past_frames
                .checked_add(half_frame)
                .expect("validated temporal window fits an exact rational");
            temporal_window.future_frames = temporal_window
                .future_frames
                .checked_add(half_frame)
                .expect("validated temporal window fits an exact rational");
        }
        Self {
            nodes,
            references,
            traversal,
            temporal_nodes,
            temporal_sample_count,
            temporal_window,
            auto_blur_nodes,
        }
    }

    pub fn node(&self, id: NodeId) -> Option<&NodeDependencies> {
        self.nodes.get(id.0 as usize)
    }

    pub fn references(&self) -> &[SceneReference] {
        &self.references
    }

    pub(crate) fn temporal_nodes(&self) -> &[TemporalNode] {
        &self.temporal_nodes
    }

    pub(crate) fn temporal_sample_count(&self) -> usize {
        self.temporal_sample_count
    }

    pub fn temporal_window(&self) -> TemporalWindow {
        self.temporal_window
    }

    pub(crate) fn auto_blur_nodes(&self) -> &[NodeId] {
        &self.auto_blur_nodes
    }

    pub(crate) fn auto_blur_offset(&self, fps: FrameRate) -> Option<RationalTime> {
        if self.auto_blur_nodes.is_empty() {
            return None;
        }
        fps.numerator()
            .checked_mul(2)
            .and_then(|denominator| u32::try_from(denominator).ok())
            .and_then(|denominator| {
                RationalTime::new(i64::from(fps.denominator()), denominator).ok()
            })
    }

    /// Resolve each node's local time through its structural parent chain. Retaining the
    /// authored subtraction/multiplication order avoids changing floating-point edge samples.
    pub(crate) fn mapped_seconds(&self, output_seconds: f64) -> Result<Vec<f64>, NodeId> {
        let mut seconds = vec![output_seconds; self.nodes.len()];
        for &id in &self.traversal {
            let facts = &self.nodes[id.0 as usize];
            let inherited = facts
                .parent
                .map_or(output_seconds, |parent| seconds[parent.0 as usize]);
            let local = facts
                .time_remap
                .map_or(inherited, |remap| remap.apply(inherited));
            if !local.is_finite() {
                return Err(id);
            }
            seconds[id.0 as usize] = local;
        }
        Ok(seconds)
    }
}
