//! Keep the layout dependencies of sampled subtrees, excluding unrelated absolute branches.
//! Flow siblings remain: removing them can change containing-block size, flex/grid placement,
//! margin collapsing, percentage bases or transform origins. Cross-node readers pin their source
//! subtrees, even if currently inactive.

use super::dependencies::{SceneDependencies, TemporalSampling};
use super::{LayoutError, LayoutTree, PreparedScene};
use crate::{
    ContextInput, Expr, MotionValue, NodeId, NodeKind, SceneArtifact, StyleBinding, StyleValue,
};

/// The same checked clock arithmetic serves full layout and retained templates. Static pixels
/// do not make overflowing sample times legal. Keep visits in exposure order and report the
/// original scope-specific error, including the frame-duration check for an empty Shutter.
pub(super) fn visit_sample_contexts(
    shutter: bool,
    key: &str,
    offsets: &[valle_timeline::RationalTime],
    ctx: &crate::MotionContext,
    mut visit: impl FnMut(&crate::MotionContext) -> Result<(), LayoutError>,
) -> Result<(), LayoutError> {
    let bad_time = || {
        if shutter {
            LayoutError::BadShutter {
                node: key.to_owned(),
                reason: "shutter sample overflows exact time".into(),
            }
        } else {
            LayoutError::BadEcho {
                node: key.to_owned(),
                reason: "echo sample overflows exact time".into(),
            }
        }
    };
    let scale = shutter
        .then(|| ctx.frame_duration().ok_or_else(bad_time))
        .transpose()?;
    for &offset in offsets {
        let offset = match scale {
            Some(scale) => scale.checked_mul(offset).map_err(|_| bad_time())?,
            None => offset,
        };
        let sample = ctx.offset(offset).ok_or_else(bad_time)?;
        visit(&sample)?;
    }
    Ok(())
}

/// Only scopes visited by the cold layout are checked. Hidden scopes may have invalid sample
/// offsets without being evaluated. Admission fixes their visibility under the template key.
pub(super) struct SampleClockChecks(Vec<(String, bool, Vec<valle_timeline::RationalTime>)>);

impl SampleClockChecks {
    pub(super) fn new(prepared: &PreparedScene, tree: &LayoutTree) -> Self {
        let nodes = prepared.dependencies().temporal_nodes();
        let mut checks = Vec::new();
        // Full layout visits every Shutter before any Echo, regardless of tree order.
        for shutter in [true, false] {
            for node in nodes {
                if matches!(node.sampling, TemporalSampling::Shutter { .. }) != shutter {
                    continue;
                }
                let key = &prepared.artifact().nodes[node.node.0 as usize].key;
                if (shutter && tree.shutters.contains_key(key))
                    || (!shutter && tree.echoes.contains_key(key))
                {
                    checks.push((key.clone(), shutter, node.offsets.clone()));
                }
            }
        }
        Self(checks)
    }

    pub(super) fn contexts(
        &self,
        ctx: &crate::MotionContext,
    ) -> Result<
        std::collections::BTreeMap<crate::emit::TemporalPaintSample, crate::MotionContext>,
        LayoutError,
    > {
        let mut contexts = std::collections::BTreeMap::new();
        for (key, shutter, offsets) in &self.0 {
            let mut index = 0;
            visit_sample_contexts(*shutter, key, offsets, ctx, |sample| {
                contexts.insert(
                    crate::emit::TemporalPaintSample {
                        scope: key.clone(),
                        index,
                    },
                    *sample,
                );
                index += 1;
                Ok(())
            })?;
        }
        Ok(contexts)
    }
}

/// Proof for sharing the *layout tree* of this request's temporal samples. Keep sampling,
/// recording and F16 accumulation unchanged. Props, fonts and viewport belong to the request;
/// clocks vary between samples. This first class excludes implicit time/resource producers,
/// visibility ranges, unit text, CSS utilities and auto-velocity estimation.
pub(super) fn sample_tree_invariant(
    artifact: &SceneArtifact,
    plan: &crate::eval::EvalPlan,
) -> bool {
    artifact.camera.is_none()
        && !samples_read_output_time(artifact)
        && artifact.nodes.iter().all(|node| {
            matches!(
                node.kind,
                NodeKind::Group
                    | NodeKind::Box
                    | NodeKind::Shutter { .. }
                    | NodeKind::Echo { .. }
                    | NodeKind::Text {
                        text: crate::TextValue::Static { .. },
                        per_unit: None,
                        path: None
                    }
            ) && node.space.is_none()
                && node.visibility.is_none()
                && node.class_names.is_empty()
                && node.class_conditions.is_empty()
                && node
                    .expr_refs_outside_per_unit()
                    .into_iter()
                    .all(|(_, id)| !plan.sample_dependent(id))
        })
}

/// Whether a temporal sample's tree depends on its output time beyond the scene time it shows:
/// the scene reads the clip clock, or blurs by motion, whose trail lies around the sample.
pub(super) fn samples_read_output_time(artifact: &SceneArtifact) -> bool {
    let reads_clip_clock = |exprs: &[Expr]| {
        exprs.iter().any(|expr| {
            matches!(
                expr,
                Expr::Context {
                    input: ContextInput::HostSeconds | ContextInput::HostProgress
                }
            )
        })
    };
    reads_clip_clock(&artifact.exprs)
        || artifact
            .instance_groups
            .iter()
            .any(|group| reads_clip_clock(&group.exprs))
        || artifact.any_node(|node| {
            node.styles
                .iter()
                .any(|style| style.property.starts_with("motion-velocity-blur-"))
        })
}

/// Emission reserves display instances for the batches of every built temporal sample tree and
/// gives paper grain the remainder. Which sample trees are built, and what they contain, can
/// therefore change the pixels only when a scene has both batches and paper grain.
pub(super) fn sample_trees_share_batch_budget(artifact: &SceneArtifact) -> bool {
    artifact.any_node(|node| {
        matches!(
            node.kind,
            NodeKind::GeometryBatch { .. } | NodeKind::InstanceBatch { .. }
        )
    }) && artifact.any_node(|node| {
        node.styles
            .iter()
            .any(|style| style.property == "paper-grain")
    })
}

fn static_keyword(style: &StyleBinding) -> Option<&str> {
    match &style.value {
        StyleValue::Static {
            value: MotionValue::Str(value) | MotionValue::Enum(value),
        } => Some(value.as_str()),
        _ => None,
    }
}

pub(super) fn sample_artifact(
    artifact: &SceneArtifact,
    dependencies: &SceneDependencies,
) -> Option<SceneArtifact> {
    let targets: Vec<_> = dependencies
        .temporal_nodes()
        .iter()
        .map(|node| node.node)
        .chain(dependencies.auto_blur_nodes().iter().copied())
        .collect();
    // Emission paints each Shutter/Echo sample from its scope's own stacking context into a
    // fresh transparent layer, so a backdrop reader inside that scope sees only the same
    // sample's earlier pixels, all of which the subset keeps. Glass derives its optics from
    // other nodes. A smaller sample tree can also enlarge the paper-grain remainder (see
    // `sample_trees_share_batch_budget`); such scenes keep full trees around backdrop readers.
    if targets.is_empty()
        || artifact
            .nodes
            .iter()
            .any(|node| matches!(node.kind, NodeKind::Glass(_) | NodeKind::GlassField(_)))
        || (sample_trees_share_batch_budget(artifact)
            && artifact.nodes.iter().enumerate().any(|(at, _)| {
                dependencies
                    .node(NodeId(at as u32))
                    .is_some_and(|facts| facts.reads_backdrop)
            }))
    {
        return None;
    }
    let mut required = vec![false; artifact.nodes.len()];
    let mut todo = targets;
    for (at, node) in artifact.nodes.iter().enumerate() {
        let facts = dependencies.node(NodeId(at as u32)).unwrap();
        if facts.geometry_referenced
            || facts.paint_referenced
            || matches!(node.kind, NodeKind::Scene3D { .. })
        {
            todo.push(NodeId(at as u32));
        }
    }
    while let Some(id) = todo.pop() {
        let at = id.0 as usize;
        if std::mem::replace(&mut required[at], true) {
            continue;
        }
        let node = &artifact.nodes[at];
        todo.extend(
            &artifact.node_children[node.children.start as usize..node.children.end as usize],
        );
    }
    // Pin ancestors without pinning unrelated children.
    for at in 0..required.len() {
        if required[at] {
            let mut parent = dependencies.node(NodeId(at as u32)).unwrap().parent;
            while let Some(id) = parent {
                required[id.0 as usize] = true;
                parent = dependencies.node(id).unwrap().parent;
            }
        }
    }
    // An absolutely positioned box is out of the flow, so its other classes and styles shape
    // only its own subtree. It must take its position from one `absolute` class or one static
    // style: a class that could put it back in the flow, a second position declaration, even a
    // redundant one, or `display: contents`, which lays its children out in its place, keeps it.
    let independent = |at: usize| {
        let node = &artifact.nodes[at];
        let placing = |class: &&String| {
            let utility = class.rsplit(':').next().unwrap_or(class).trim_matches('!');
            class.contains("position")
                || class.contains("display")
                || matches!(
                    utility,
                    "static" | "fixed" | "absolute" | "relative" | "sticky" | "contents"
                )
        };
        let classes = node.class_names.iter().filter(placing).collect::<Vec<_>>();
        let positions = node
            .styles
            .iter()
            .filter(|style| style.property == "position")
            .collect::<Vec<_>>();
        let absolute = match (classes.as_slice(), positions.as_slice()) {
            ([class], []) => class.as_str() == "absolute",
            ([], [style]) => static_keyword(style) == Some("absolute"),
            _ => false,
        };
        absolute
            && node.space.is_none()
            && node.class_conditions.is_empty()
            && node
                .styles
                .iter()
                .filter(|style| style.property == "display")
                .all(|style| static_keyword(style).is_some_and(|value| value != "contents"))
    };
    let mut keep = vec![false; artifact.nodes.len()];
    let mut todo = vec![artifact.root];
    while let Some(id) = todo.pop() {
        let at = id.0 as usize;
        if at != artifact.root.0 as usize && !required[at] && independent(at) {
            continue;
        }
        keep[at] = true;
        let node = &artifact.nodes[at];
        todo.extend(
            &artifact.node_children[node.children.start as usize..node.children.end as usize],
        );
    }
    if keep.iter().all(|keep| *keep) {
        return None;
    }
    let mut ids = vec![NodeId(0); keep.len()];
    let mut nodes = Vec::new();
    for (at, node) in artifact.nodes.iter().enumerate() {
        if keep[at] {
            ids[at] = NodeId(nodes.len() as u32);
            nodes.push(node.clone());
        }
    }
    let mut children = Vec::new();
    for node in &mut nodes {
        let start = children.len() as u32;
        children.extend(
            artifact.node_children[node.children.start as usize..node.children.end as usize]
                .iter()
                .filter(|id| keep[id.0 as usize])
                .map(|id| ids[id.0 as usize]),
        );
        node.children.start = start;
        node.children.end = children.len() as u32;
    }
    let mut subset = artifact.clone();
    subset.root = ids[artifact.root.0 as usize];
    subset.nodes = nodes;
    subset.node_children = children;
    // Instance indices and ExprIds stay identical so the request's value caches can be shared.
    // This internal projection is not a new author artifact: unused instance tables are allowed.
    Some(subset)
}
