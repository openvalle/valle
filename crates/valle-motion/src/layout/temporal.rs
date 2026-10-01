//! Keep the layout dependencies of sampled subtrees, excluding unrelated absolute branches.
//! Flow siblings remain: removing them can change containing-block size, flex/grid placement or
//! inherited geometry. Cross-node readers pin their source subtrees, even if currently inactive.

use super::dependencies::SceneDependencies;
use crate::{MotionValue, NodeId, NodeKind, SceneArtifact, StyleValue};

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
    if targets.is_empty()
        || artifact.nodes.iter().enumerate().any(|(at, _)| {
            dependencies
                .node(NodeId(at as u32))
                .is_some_and(|facts| facts.reads_backdrop)
        })
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
    let independent = |at: usize| {
        let node = &artifact.nodes[at];
        node.space.is_none() && node.class_conditions.is_empty()
            && ((node.class_names.len() == 1 && node.class_names[0] == "absolute"
                && !node.styles.iter().any(|style| style.property == "position"))
                || (node.class_names.is_empty() && node.styles.iter().any(|style| {
                    style.property == "position" && matches!(&style.value,
                        StyleValue::Static { value: MotionValue::Str(value) | MotionValue::Enum(value) }
                        if value == "absolute")
                })))
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
