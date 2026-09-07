use std::collections::{BTreeMap, BTreeSet};

use crate::NodeKind;
use crate::artifact::{
    ColorValue, NumberValue, PointValue, SceneArtifact, StyleValue, ValidationError,
};
use crate::expr::ExprId;
use crate::glass::continuity::{TemporalContinuity, infer_continuity};
use crate::glass::ids::GlassFieldId;
use crate::glass::intent::{
    GlassCharacter, GlassEnvironmentBinding, GlassFieldNode, GlassForegroundTone, GlassNode,
    GlassShapeBinding, GlassSurfaceMotionBinding, MERGE_MAX_DISTANCE, MERGE_MIN_DISTANCE,
    SETTLE_MAX_SECONDS, SETTLE_MIN_SECONDS,
};

/// Schema checks for Glass nodes. Production [`SceneArtifact::validate`] calls this after proving
/// that the artifact declares `motion-glass` if and only if it contains Glass topology.
pub fn validate_glass_schema(artifact: &SceneArtifact) -> Result<(), Vec<ValidationError>> {
    let mut errors = Vec::new();
    let continuity = infer_continuity(&artifact.exprs);
    let mut surfaces = BTreeMap::<String, usize>::new();
    let mut fields = BTreeMap::<String, usize>::new();
    let (field_of_node, tree_is_valid) = checked_field_membership(artifact);
    if !tree_is_valid {
        errors.push(ValidationError::new(
            "/nodes",
            "Glass validation requires a rooted tree with valid, non-cyclic child links",
        ));
    }

    for (at, node) in artifact.nodes.iter().enumerate() {
        match &node.kind {
            NodeKind::Glass(glass) => validate_glass_node(
                artifact,
                at,
                glass,
                &continuity,
                &field_of_node,
                &mut surfaces,
                &mut errors,
            ),
            NodeKind::GlassField(field) => validate_field_node(at, field, &mut fields, &mut errors),
            _ => {}
        }
    }
    validate_field_structure(artifact, &field_of_node, &mut errors);

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn paints_or_reorders(property: &str) -> bool {
    property == "opacity"
        || property == "filter"
        || property == "mix-blend-mode"
        || property == "backdrop-filter"
        || property == "box-shadow"
        || property == "outline"
        || property == "clip-path"
        || property == "mask"
        || property.starts_with("mask-")
        || property == "background"
        || property.starts_with("background-")
        || property == "border"
        || property.starts_with("border-")
}

fn paints_glass_shell(property: &str) -> bool {
    property == "box-shadow"
        || property == "outline"
        || property == "background"
        || property.starts_with("background-")
        || property == "border"
        || property.starts_with("border-")
}

/// G2.1 field structure checks: painted wrappers inside a field, painted members, and a
/// GlassField nested structurally inside another GlassField all fail closed. A nested field is
/// legal only inside a member foreground (its parent chain passes through a Glass member).
fn validate_field_structure(
    artifact: &SceneArtifact,
    field_of_node: &[Option<GlassFieldId>],
    errors: &mut Vec<ValidationError>,
) {
    let mut field_nodes = BTreeMap::<String, usize>::new();
    for (at, node) in artifact.nodes.iter().enumerate() {
        if let NodeKind::GlassField(field) = &node.kind {
            field_nodes.insert(field.field_id.as_str().to_owned(), at);
        }
    }
    let mut member_counts = BTreeMap::<String, usize>::new();
    for (at, node) in artifact.nodes.iter().enumerate() {
        match &node.kind {
            NodeKind::GlassField(field) => {
                let path = format!("/nodes/{at}/kind");
                reject_paint(artifact, at, &path, "GlassField", errors);
                // Walk through non-painting wrappers. Encountering a Glass shell means this field
                // is in that member's foreground and begins a legal, isolated material scope.
                let mut cursor = parent_index(artifact, at);
                let mut visited = vec![false; artifact.nodes.len()];
                while let Some(parent) = cursor {
                    if parent >= visited.len() || visited[parent] {
                        break;
                    }
                    visited[parent] = true;
                    match &artifact.nodes[parent].kind {
                        NodeKind::Glass(_) => break,
                        NodeKind::GlassField(parent_field) => {
                            errors.push(ValidationError::new(
                                format!("{path}/fieldId"),
                                format!(
                                    "GlassField '{}' is structurally nested inside GlassField '{}'; nested fields belong in a member foreground",
                                    field.field_id.as_str(),
                                    parent_field.field_id.as_str()
                                ),
                            ));
                            break;
                        }
                        _ => cursor = parent_index(artifact, parent),
                    }
                }
            }
            NodeKind::Glass(_glass) => {
                for style in &node.styles {
                    if paints_glass_shell(&style.property) {
                        errors.push(ValidationError::new(
                            format!("/nodes/{at}/styles"),
                            format!(
                                "Glass shell '{}' cannot paint `{}`; put ordinary pixels in its foreground children",
                                node.key, style.property
                            ),
                        ));
                    }
                }
                let Some(field_id) = field_of_node.get(at).cloned().flatten() else {
                    continue;
                };
                *member_counts
                    .entry(field_id.as_str().to_owned())
                    .or_default() += 1;
                let path = format!("/nodes/{at}/kind");
                reject_paint(artifact, at, &path, "Glass member", errors);
                // Any painted ancestor between the member and its field root is a painted
                // wrapper inside the field material.
                let Some(field_root) = field_nodes.get(field_id.as_str()).copied() else {
                    continue;
                };
                let mut cursor = parent_index(artifact, at);
                let mut visited = vec![false; artifact.nodes.len()];
                while let Some(index) = cursor {
                    if index >= visited.len() || visited[index] {
                        break;
                    }
                    visited[index] = true;
                    if index == field_root {
                        break;
                    }
                    reject_paint(
                        artifact,
                        index,
                        &format!("/nodes/{index}/kind"),
                        "Glass field wrapper",
                        errors,
                    );
                    cursor = parent_index(artifact, index);
                }
            }
            NodeKind::Group | NodeKind::Box => {
                if field_of_node.get(at).is_some_and(Option::is_some) {
                    reject_paint(
                        artifact,
                        at,
                        &format!("/nodes/{at}/kind"),
                        "Glass field wrapper",
                        errors,
                    );
                }
            }
            _ => {
                if let Some(field) = field_of_node.get(at).and_then(Option::as_ref) {
                    errors.push(ValidationError::new(
                        format!("/nodes/{at}/kind"),
                        format!(
                            "GlassField '{}' contains ordinary painted node '{}' outside a member Glass foreground",
                            field.as_str(), node.key
                        ),
                    ));
                }
            }
        }
    }
    for (field, at) in field_nodes {
        if member_counts.get(&field).copied().unwrap_or(0) == 0 {
            errors.push(ValidationError::new(
                format!("/nodes/{at}/kind"),
                format!("GlassField '{field}' must contain at least one Glass member"),
            ));
        }
    }
}

fn reject_paint(
    artifact: &SceneArtifact,
    at: usize,
    path: &str,
    owner: &str,
    errors: &mut Vec<ValidationError>,
) {
    for style in &artifact.nodes[at].styles {
        if paints_or_reorders(&style.property) {
            errors.push(ValidationError::new(
                format!("{path}/styles"),
                format!(
                    "{owner} '{}' has a paint style `{}` inside a GlassField; field members and wrappers cannot paint outside the shared material",
                    artifact.nodes[at].key, style.property
                ),
            ));
        }
    }
}

fn validate_glass_node(
    artifact: &SceneArtifact,
    at: usize,
    glass: &GlassNode,
    continuity: &[TemporalContinuity],
    field_of_node: &[Option<GlassFieldId>],
    surfaces: &mut BTreeMap<String, usize>,
    errors: &mut Vec<ValidationError>,
) {
    let path = format!("/nodes/{at}");
    let id = glass.surface_id.as_str().to_owned();
    if let Some(previous) = surfaces.insert(id.clone(), at) {
        errors.push(ValidationError::new(
            format!("{path}/kind/surfaceId"),
            format!(
                "Glass surfaceId '{id}' appears twice at the same sample time (also /nodes/{previous})"
            ),
        ));
    }
    let inherited = field_of_node.get(at).cloned().flatten();
    match (&glass.field_id, inherited) {
        (Some(declared), Some(actual)) if declared != &actual => {
            errors.push(ValidationError::new(
                format!("{path}/kind/fieldId"),
                format!(
                    "Glass '{}' is collected by field '{}' but declares '{}'",
                    glass.surface_id.as_str(),
                    actual.as_str(),
                    declared.as_str()
                ),
            ));
        }
        (None, Some(actual)) => {
            errors.push(ValidationError::new(
                format!("{path}/kind/fieldId"),
                format!(
                    "Glass '{}' must record field '{}'",
                    glass.surface_id.as_str(),
                    actual.as_str()
                ),
            ));
        }
        (Some(_), None) => {
            errors.push(ValidationError::new(
                format!("{path}/kind/fieldId"),
                format!(
                    "Glass '{}' declares a field but is not inside one",
                    glass.surface_id.as_str()
                ),
            ));
        }
        _ => {}
    }
    let member = glass.field_id.is_some();
    if member {
        if glass.material.is_some() {
            errors.push(ValidationError::new(
                format!("{path}/kind/material"),
                format!(
                    "Glass '{}' inherits material from its field and cannot declare material",
                    glass.surface_id.as_str()
                ),
            ));
        }
        if glass.motion.character.is_some() || glass.motion.settle.is_some() {
            errors.push(ValidationError::new(
                format!("{path}/kind/motion"),
                format!(
                    "Glass '{}' cannot override field character or settle",
                    glass.surface_id.as_str()
                ),
            ));
        }
        if glass.environment.is_some() {
            errors.push(ValidationError::new(
                format!("{path}/kind/environment"),
                format!(
                    "Glass '{}' inherits environment from its field and cannot declare environment",
                    glass.surface_id.as_str()
                ),
            ));
        }
    } else if glass.material.is_none() {
        errors.push(ValidationError::new(
            format!("{path}/kind/material"),
            "independent Glass must include expanded material defaults",
        ));
    } else if glass.environment.is_none() {
        errors.push(ValidationError::new(
            format!("{path}/kind/environment"),
            "independent Glass must include expanded environment defaults",
        ));
    }
    if let Some(environment) = &glass.environment {
        validate_environment(environment, &format!("{path}/kind/environment"), errors);
    }
    if let Some(settle) = glass.motion.settle {
        reject_settle(settle, format!("{path}/kind/motion/settle"), errors);
    }
    reject_unit(
        "presence",
        &glass.presence,
        &format!("{path}/kind/presence"),
        errors,
    );
    if glass.foreground.tone == GlassForegroundTone::Auto
        && !matches!(
            glass.foreground.protection,
            NumberValue::Static { value: 0.0 }
        )
        && subtree_has_unknown_foreground(artifact, at)
    {
        errors.push(ValidationError::new(
            format!("{path}/kind/foreground/tone"),
            format!(
                "Glass '{}' has image, video, Scene3D, or custom-shader foreground content; tone=auto with protection>0 requires an explicit light/dark/none hint",
                glass.surface_id.as_str()
            ),
        ));
    }
    reject_unit(
        "intensity",
        &glass.motion.intensity,
        &format!("{path}/kind/motion/intensity"),
        errors,
    );
    reject_unit(
        "protection",
        &glass.foreground.protection,
        &format!("{path}/kind/foreground/protection"),
        errors,
    );
    if let Some(material) = &glass.material {
        reject_unit(
            "clarity",
            &material.clarity,
            &format!("{path}/kind/material/clarity"),
            errors,
        );
        reject_unit(
            "depth",
            &material.depth,
            &format!("{path}/kind/material/depth"),
            errors,
        );
        reject_color(
            &material.tint,
            &format!("{path}/kind/material/tint"),
            errors,
        );
    }
    match &glass.shape {
        GlassShapeBinding::ContinuousRect { radius } => {
            reject_non_negative(radius, &format!("{path}/kind/shape/radius"), errors);
        }
        GlassShapeBinding::Path { .. } | GlassShapeBinding::Capsule | GlassShapeBinding::Circle => {
        }
    }
    reject_discrete_track_bindings(artifact, at, glass, continuity, errors);
}

fn subtree_has_unknown_foreground(artifact: &SceneArtifact, glass_at: usize) -> bool {
    let Some(_) = artifact.nodes.get(glass_at) else {
        return true;
    };
    let mut visited = vec![false; artifact.nodes.len()];
    visited[glass_at] = true;
    let Some(children) = node_children(artifact, glass_at) else {
        return true;
    };
    let mut stack: Vec<usize> = children
        .iter()
        .rev()
        .map(|child| child.0 as usize)
        .collect();
    while let Some(at) = stack.pop() {
        let Some(node) = artifact.nodes.get(at) else {
            return true;
        };
        if visited[at] {
            // A repeated node is either a cycle or a multiple-parent DAG. Both are invalid scene
            // topology, so the foreground classifier must fail closed instead of accepting it.
            return true;
        }
        visited[at] = true;
        if matches!(
            node.kind,
            NodeKind::Image { .. }
                | NodeKind::Video { .. }
                | NodeKind::ShaderLayer { .. }
                | NodeKind::Scene3D { .. }
        ) {
            return true;
        }
        let Some(children) = node_children(artifact, at) else {
            return true;
        };
        stack.extend(children.iter().rev().map(|child| child.0 as usize));
    }
    false
}

fn validate_field_node(
    at: usize,
    field: &GlassFieldNode,
    fields: &mut BTreeMap<String, usize>,
    errors: &mut Vec<ValidationError>,
) {
    let path = format!("/nodes/{at}");
    let id = field.field_id.as_str().to_owned();
    if let Some(previous) = fields.insert(id.clone(), at) {
        errors.push(ValidationError::new(
            format!("{path}/kind/fieldId"),
            format!("GlassField id '{id}' appears twice (also /nodes/{previous})"),
        ));
    }
    reject_settle(
        field.motion.settle,
        format!("{path}/kind/motion/settle"),
        errors,
    );
    if !(MERGE_MIN_DISTANCE..=MERGE_MAX_DISTANCE).contains(&field.merge.distance)
        || !field.merge.distance.is_finite()
    {
        errors.push(ValidationError::new(
            format!("{path}/kind/merge/distance"),
            format!("merge distance must be finite in {MERGE_MIN_DISTANCE}..={MERGE_MAX_DISTANCE}"),
        ));
    }
    validate_environment(
        &field.environment,
        &format!("{path}/kind/environment"),
        errors,
    );
}

fn validate_environment(
    environment: &GlassEnvironmentBinding,
    path: &str,
    errors: &mut Vec<ValidationError>,
) {
    if let PointValue::Static { value } = &environment.light.direction
        && (!value.x.is_finite()
            || !value.y.is_finite()
            || valle_draw::math::sqrt(value.x * value.x + value.y * value.y) <= f64::EPSILON)
    {
        errors.push(ValidationError::new(
            format!("{path}/light/direction"),
            "light direction must be a finite non-zero vector",
        ));
    }
    reject_unit(
        "light elevation",
        &environment.light.elevation,
        &format!("{path}/light/elevation"),
        errors,
    );
    reject_unit(
        "light intensity",
        &environment.light.intensity,
        &format!("{path}/light/intensity"),
        errors,
    );
}

fn reject_settle(settle: f64, path: String, errors: &mut Vec<ValidationError>) {
    if !settle.is_finite() || !(SETTLE_MIN_SECONDS..=SETTLE_MAX_SECONDS).contains(&settle) {
        errors.push(ValidationError::new(
            path,
            format!("settle must be a static finite value in {SETTLE_MIN_SECONDS}..={SETTLE_MAX_SECONDS} seconds"),
        ));
    }
}

fn reject_unit(name: &str, value: &NumberValue, path: &str, errors: &mut Vec<ValidationError>) {
    if let NumberValue::Static { value } = value
        && (!value.is_finite() || !(0.0..=1.0).contains(value))
    {
        errors.push(ValidationError::new(
            path,
            format!("{name} must be finite in 0..=1"),
        ));
    }
}

fn reject_non_negative(value: &NumberValue, path: &str, errors: &mut Vec<ValidationError>) {
    if let NumberValue::Static { value } = value
        && (!value.is_finite() || *value < 0.0)
    {
        errors.push(ValidationError::new(
            path,
            "value must be a finite non-negative number",
        ));
    }
}

fn reject_color(value: &ColorValue, path: &str, errors: &mut Vec<ValidationError>) {
    let _ = (value, path, errors);
}

fn reject_discrete_track_bindings(
    artifact: &SceneArtifact,
    at: usize,
    glass: &GlassNode,
    continuity: &[TemporalContinuity],
    errors: &mut Vec<ValidationError>,
) {
    let mut roots = Vec::new();
    push_number(&mut roots, &glass.presence);
    push_number(&mut roots, &glass.motion.intensity);
    push_drive(
        &mut roots,
        &glass.motion.drive.translation,
        &glass.motion.drive.pressure,
        &glass.motion.drive.twist,
    );
    if let Some(material) = &glass.material {
        push_number(&mut roots, &material.clarity);
        push_number(&mut roots, &material.depth);
    }
    if let Some(environment) = &glass.environment {
        if let PointValue::Expr { expr } = &environment.light.direction {
            roots.push(*expr);
        }
        push_number(&mut roots, &environment.light.elevation);
        push_number(&mut roots, &environment.light.intensity);
    }
    match &glass.shape {
        GlassShapeBinding::ContinuousRect { radius } => push_number(&mut roots, radius),
        GlassShapeBinding::Path {
            path,
            reveal_origin,
        } => {
            if let crate::artifact::PathValue::Expr { expr, .. } = path {
                roots.push(*expr);
            }
            if let Some(PointValue::Expr { expr }) = reveal_origin {
                roots.push(*expr);
            }
        }
        _ => {}
    }
    for style in &artifact.nodes[at].styles {
        if matches!(
            style.property.as_str(),
            "left" | "top" | "width" | "height" | "transform" | "translate" | "rotate" | "scale"
        ) && let StyleValue::Expr { expr } = &style.value
        {
            roots.push(*expr);
        }
    }
    for expr in roots {
        let Some(class) = continuity.get(expr.0 as usize) else {
            continue;
        };
        if !class.is_track_legal() {
            errors.push(ValidationError::new(
                format!("/nodes/{at}/kind"),
                format!(
                    "Glass '{}' track binding depends on a discrete or unknown-time expression",
                    glass.surface_id.as_str()
                ),
            ));
            break;
        }
    }
}

fn push_number(out: &mut Vec<ExprId>, value: &NumberValue) {
    if let NumberValue::Expr { expr } = value {
        out.push(*expr);
    }
}

fn push_number_set(out: &mut BTreeSet<ExprId>, value: &NumberValue) {
    if let NumberValue::Expr { expr } = value {
        out.insert(*expr);
    }
}

fn push_drive(
    out: &mut Vec<ExprId>,
    translation: &Option<PointValue>,
    pressure: &Option<NumberValue>,
    twist: &Option<NumberValue>,
) {
    if let Some(PointValue::Expr { expr }) = translation {
        out.push(*expr);
    }
    if let Some(value) = pressure {
        push_number(out, value);
    }
    if let Some(value) = twist {
        push_number(out, value);
    }
}

/// Walk structural wrappers to assign each Glass shell its nearest enclosing field. Descending
/// into a Glass child crosses into foreground content, so field collection stops at that shell.
pub fn field_membership(artifact: &SceneArtifact) -> Vec<Option<GlassFieldId>> {
    checked_field_membership(artifact).0
}

fn checked_field_membership(artifact: &SceneArtifact) -> (Vec<Option<GlassFieldId>>, bool) {
    let mut assigned = vec![None; artifact.nodes.len()];
    let root = artifact.root.0 as usize;
    if root >= artifact.nodes.len() {
        return (assigned, false);
    }

    let mut tree_is_valid = true;
    let mut visited = vec![false; artifact.nodes.len()];
    let mut stack = vec![(root, None)];
    while let Some((node_at, current)) = stack.pop() {
        let Some(node) = artifact.nodes.get(node_at) else {
            tree_is_valid = false;
            continue;
        };
        if visited[node_at] {
            tree_is_valid = false;
            continue;
        }
        visited[node_at] = true;
        let inherited = match &node.kind {
            NodeKind::GlassField(field) => Some(field.field_id.clone()),
            _ => current,
        };
        assigned[node_at] = inherited.clone();
        let child_field = if matches!(&node.kind, NodeKind::Glass(_)) {
            None
        } else {
            inherited
        };
        let Some(children) = node_children(artifact, node_at) else {
            tree_is_valid = false;
            continue;
        };
        for child in children.iter().rev() {
            let child_at = child.0 as usize;
            if child_at >= artifact.nodes.len() {
                tree_is_valid = false;
                continue;
            }
            stack.push((child_at, child_field.clone()));
        }
    }
    tree_is_valid &= visited.iter().all(|visited| *visited);
    (assigned, tree_is_valid)
}

fn node_children(artifact: &SceneArtifact, node_at: usize) -> Option<&[crate::NodeId]> {
    let children = artifact.nodes.get(node_at)?.children;
    let start = children.start as usize;
    let end = children.end as usize;
    (start <= end)
        .then(|| artifact.node_children.get(start..end))
        .flatten()
}

pub fn glass_track_expr_roots(artifact: &SceneArtifact, node_index: usize) -> BTreeSet<ExprId> {
    let mut roots = BTreeSet::new();
    let Some(node) = artifact.nodes.get(node_index) else {
        return roots;
    };
    let NodeKind::Glass(glass) = &node.kind else {
        return roots;
    };
    push_number_set(&mut roots, &glass.presence);
    push_number_set(&mut roots, &glass.motion.intensity);
    push_number_set(&mut roots, &glass.foreground.protection);
    if let Some(material) = &glass.material {
        push_number_set(&mut roots, &material.clarity);
        push_number_set(&mut roots, &material.depth);
    }
    if let Some(environment) = &glass.environment {
        if let PointValue::Expr { expr } = &environment.light.direction {
            roots.insert(*expr);
        }
        push_number_set(&mut roots, &environment.light.elevation);
        push_number_set(&mut roots, &environment.light.intensity);
    }
    if let Some(value) = glass.motion.drive.pressure.as_ref() {
        push_number_set(&mut roots, value);
    }
    if let Some(value) = glass.motion.drive.twist.as_ref() {
        push_number_set(&mut roots, value);
    }
    if let Some(PointValue::Expr { expr }) = glass.motion.drive.translation.as_ref() {
        roots.insert(*expr);
    }
    if let GlassShapeBinding::ContinuousRect { radius } = &glass.shape {
        push_number_set(&mut roots, radius);
    }
    let mut cursor = Some(node_index);
    let mut visited = vec![false; artifact.nodes.len()];
    while let Some(index) = cursor {
        let Some(node) = artifact.nodes.get(index) else {
            break;
        };
        if visited[index] {
            break;
        }
        visited[index] = true;
        for style in &node.styles {
            if matches!(
                style.property.as_str(),
                "left"
                    | "top"
                    | "width"
                    | "height"
                    | "transform"
                    | "translate"
                    | "rotate"
                    | "scale"
                    | "opacity"
            ) && let StyleValue::Expr { expr } = &style.value
            {
                roots.insert(*expr);
            }
        }
        cursor = parent_index(artifact, index);
    }
    roots
}

fn parent_index(artifact: &SceneArtifact, child: usize) -> Option<usize> {
    for (at, node) in artifact.nodes.iter().enumerate() {
        let start = node.children.start as usize;
        let end = node.children.end as usize;
        if start <= end
            && artifact
                .node_children
                .get(start..end)
                .is_some_and(|children| children.iter().any(|id| id.0 as usize == child))
        {
            return Some(at);
        }
    }
    None
}

pub fn reachable_exprs(artifact: &SceneArtifact, roots: &BTreeSet<ExprId>) -> BTreeSet<ExprId> {
    let mut seen = BTreeSet::new();
    let mut stack: Vec<ExprId> = roots.iter().copied().collect();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        if let Some(expr) = artifact.exprs.get(id.0 as usize) {
            stack.extend(expr.children());
        }
    }
    seen
}

#[allow(dead_code)]
fn _character_is_static(_character: GlassCharacter, _motion: &GlassSurfaceMotionBinding) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact::{
        ARTIFACT_FORMAT_VERSION, CapabilitySet, ChildRange, NumberValue, SceneArtifact, SceneNode,
        StyleBinding, StyleValue,
    };
    use crate::controls::{ControlsSchema, FrameControl, OptionalFrameControl, TimingControls};
    use crate::glass::ids::{GlassFieldId, GlassSurfaceId};
    use crate::glass::intent::{
        GlassEnvironmentBinding, GlassFieldMotionBinding, GlassFieldNode, GlassForegroundIntent,
        GlassMaterialBinding, GlassMergeIntent, GlassNode, GlassShapeBinding,
        GlassSurfaceMotionBinding,
    };
    use crate::value::MotionValue;
    use crate::{NodeId, NodeKind};

    fn controls() -> ControlsSchema {
        ControlsSchema {
            props: Default::default(),
            data: Default::default(),
            timing: TimingControls {
                enter_frames: FrameControl {
                    default: 0,
                    min: 0,
                    max: None,
                },
                hold_cycle_frames: OptionalFrameControl {
                    default: None,
                    min: 1,
                    max: None,
                },
                exit_frames: FrameControl {
                    default: 0,
                    min: 0,
                    max: None,
                },
            },
            cues: Default::default(),
            assets: Default::default(),
            camera: Default::default(),
        }
    }

    /// root(0) -> [field(1) -> wrapper(2) -> member(3)] with an optional member foreground.
    fn field_artifact(
        wrapper_styles: Vec<StyleBinding>,
        member_styles: Vec<StyleBinding>,
    ) -> SceneArtifact {
        let field_id = GlassFieldId::new("orbit").unwrap();
        let member_id = GlassSurfaceId::new("left").unwrap();
        let field = GlassFieldNode {
            field_id: field_id.clone(),
            material: GlassMaterialBinding::defaults(),
            motion: GlassFieldMotionBinding::defaults(),
            merge: GlassMergeIntent::defaults(),
            environment: GlassEnvironmentBinding::defaults(),
        };
        let member = GlassNode {
            surface_id: member_id,
            field_id: Some(field_id),
            shape: GlassShapeBinding::Circle,
            material: None,
            environment: None,
            motion: GlassSurfaceMotionBinding::field_member_defaults(),
            presence: NumberValue::Static { value: 1.0 },
            foreground: GlassForegroundIntent::defaults(),
        };
        SceneArtifact {
            camera: None,
            format_version: ARTIFACT_FORMAT_VERSION,
            capability_set: CapabilitySet::base(),
            component: "field-test".into(),
            controls: controls(),
            resource_refs: vec![],
            exprs: vec![],
            nodes: vec![
                SceneNode {
                    key: "root".into(),
                    kind: NodeKind::Group,
                    space: None,
                    class_names: vec![],
                    styles: vec![],
                    visibility: None,
                    children: ChildRange { start: 0, end: 1 },
                    semantic: None,
                },
                SceneNode {
                    key: "orbit".into(),
                    kind: NodeKind::GlassField(field),
                    space: None,
                    class_names: vec![],
                    styles: vec![],
                    visibility: None,
                    children: ChildRange { start: 1, end: 2 },
                    semantic: None,
                },
                SceneNode {
                    key: "wrapper".into(),
                    kind: NodeKind::Group,
                    space: None,
                    class_names: vec![],
                    styles: wrapper_styles,
                    visibility: None,
                    children: ChildRange { start: 2, end: 3 },
                    semantic: None,
                },
                SceneNode {
                    key: "left".into(),
                    kind: NodeKind::Glass(member),
                    space: None,
                    class_names: vec![],
                    styles: member_styles,
                    visibility: None,
                    children: ChildRange::EMPTY,
                    semantic: None,
                },
            ],
            node_children: vec![NodeId(1), NodeId(2), NodeId(3)],
            root: NodeId(0),
        }
    }

    #[test]
    fn clean_field_artifact_validates() {
        let artifact = field_artifact(vec![], vec![]);
        validate_glass_schema(&artifact).unwrap();
        let membership = field_membership(&artifact);
        assert_eq!(membership[3], Some(GlassFieldId::new("orbit").unwrap()));
    }

    #[test]
    fn cyclic_and_out_of_bounds_trees_fail_closed() {
        let mut cyclic = field_artifact(vec![], vec![]);
        cyclic.nodes[3].children = ChildRange { start: 2, end: 3 };
        let errors = validate_glass_schema(&cyclic).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.message.contains("non-cyclic child links")),
            "{errors:?}"
        );
        assert!(subtree_has_unknown_foreground(&cyclic, 3));

        let mut out_of_bounds = field_artifact(vec![], vec![]);
        out_of_bounds.node_children[2] = NodeId(99);
        let errors = validate_glass_schema(&out_of_bounds).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.message.contains("valid, non-cyclic child links")),
            "{errors:?}"
        );
    }

    #[test]
    fn painted_wrapper_inside_field_fails_closed() {
        let artifact = field_artifact(
            vec![StyleBinding {
                property: "background".into(),
                value: StyleValue::Static {
                    value: MotionValue::Color(valle_draw::Rgba::new(255, 0, 0, 255)),
                },
            }],
            vec![],
        );
        let errors = validate_glass_schema(&artifact).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.message.contains("paint style")),
            "{errors:?}"
        );
    }

    #[test]
    fn member_outer_opacity_fails_closed() {
        let artifact = field_artifact(
            vec![],
            vec![StyleBinding {
                property: "opacity".into(),
                value: StyleValue::Static {
                    value: MotionValue::Number(0.5),
                },
            }],
        );
        let errors = validate_glass_schema(&artifact).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.message.contains("paint style")),
            "{errors:?}"
        );
    }

    #[test]
    fn member_material_override_fails_closed() {
        let mut artifact = field_artifact(vec![], vec![]);
        let NodeKind::Glass(member) = &mut artifact.nodes[3].kind else {
            panic!("member");
        };
        member.material = Some(GlassMaterialBinding::defaults());
        let errors = validate_glass_schema(&artifact).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.message.contains("inherits material")),
            "{errors:?}"
        );
    }

    #[test]
    fn structurally_nested_field_fails_closed() {
        let mut artifact = field_artifact(vec![], vec![]);
        // Replace the wrapper with a nested GlassField that owns the member.
        let nested = GlassFieldId::new("nested").unwrap();
        artifact.nodes[2].kind = NodeKind::GlassField(GlassFieldNode {
            field_id: nested.clone(),
            material: GlassMaterialBinding::defaults(),
            motion: GlassFieldMotionBinding::defaults(),
            merge: GlassMergeIntent::defaults(),
            environment: GlassEnvironmentBinding::defaults(),
        });
        let NodeKind::Glass(member) = &mut artifact.nodes[3].kind else {
            panic!("member");
        };
        member.field_id = Some(nested);
        let errors = validate_glass_schema(&artifact).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.message.contains("structurally nested")),
            "{errors:?}"
        );
    }
}
