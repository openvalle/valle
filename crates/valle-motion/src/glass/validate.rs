use std::collections::BTreeMap;

use crate::NodeKind;
use crate::artifact::{ColorValue, NumberValue, PointValue, SceneArtifact, ValidationError};
use crate::glass::ids::GlassFieldId;
use crate::glass::intent::{
    GlassEnvironmentBinding, GlassFieldNode, GlassNode, GlassShapeBinding, MERGE_MAX_DISTANCE,
    MERGE_MIN_DISTANCE,
};

/// Schema checks for Glass nodes. Production [`SceneArtifact::validate`] calls this after proving
/// that the artifact declares `motion-glass` if and only if it contains Glass topology.
pub fn validate_glass_schema(artifact: &SceneArtifact) -> Result<(), Vec<ValidationError>> {
    let mut errors = Vec::new();
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
            NodeKind::Glass(glass) => {
                validate_glass_node(at, glass, &field_of_node, &mut surfaces, &mut errors)
            }
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
    for class in &artifact.nodes[at].class_names {
        if crate::tailwind::explicit_property(class).is_some_and(paints_or_reorders) {
            errors.push(ValidationError::new(
                format!("{path}/classNames"),
                format!(
                    "{owner} '{}' cannot use paint utility `{class}` inside a GlassField",
                    artifact.nodes[at].key
                ),
            ));
        }
    }
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
    at: usize,
    glass: &GlassNode,
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
    reject_unit(
        "presence",
        &glass.presence,
        &format!("{path}/kind/presence"),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact::{
        ARTIFACT_FORMAT_VERSION, CapabilitySet, ChildRange, NumberValue, SceneArtifact, SceneNode,
        StyleBinding, StyleValue,
    };
    use crate::controls::ControlsSchema;
    use crate::glass::ids::{GlassFieldId, GlassSurfaceId};
    use crate::glass::intent::{
        GlassEnvironmentBinding, GlassFieldNode, GlassMaterialBinding, GlassMergeIntent, GlassNode,
        GlassShapeBinding,
    };
    use crate::value::MotionValue;
    use crate::{NodeId, NodeKind};

    fn controls() -> ControlsSchema {
        ControlsSchema {
            props: Default::default(),
            data: Default::default(),
            assets: Default::default(),
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
            merge: GlassMergeIntent::defaults(),
            environment: GlassEnvironmentBinding::defaults(),
        };
        let member = GlassNode {
            surface_id: member_id,
            field_id: Some(field_id),
            shape: GlassShapeBinding::Circle,
            material: None,
            environment: None,
            presence: NumberValue::Static { value: 1.0 },
        };
        SceneArtifact {
            role: valle_timeline::MotionRole::Clip,
            camera: None,
            format_version: ARTIFACT_FORMAT_VERSION,
            capability_set: CapabilitySet::base(),
            component: "field-test".into(),
            composition: None,
            controls: controls(),
            resource_refs: vec![],
            exprs: vec![],
            instance_groups: vec![],
            nodes: vec![
                SceneNode {
                    key: "root".into(),
                    kind: NodeKind::Group,
                    space: None,
                    class_names: vec![],
                    class_conditions: Default::default(),
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
                    class_conditions: Default::default(),
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
                    class_conditions: Default::default(),
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
                    class_conditions: Default::default(),
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
                    value: MotionValue::Color(valle_draw::program::AuthorColor::from_srgb8(
                        valle_draw::Rgba::new(255, 0, 0, 255),
                    )),
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
    fn filter_utilities_obey_field_paint_restrictions() {
        for at in [1, 2, 3] {
            for class in [
                "blur-sm",
                "backdrop-blur-md!",
                "filter-none",
                "[filter:blur(2px)]",
                "[backdrop-filter:blur(2px)]!",
                "[background-color:red]",
            ] {
                let mut artifact = field_artifact(vec![], vec![]);
                artifact.nodes[at].class_names.push(class.into());
                let errors = validate_glass_schema(&artifact).unwrap_err();
                assert!(
                    errors.iter().any(|error| error.message.contains(class)),
                    "{errors:?}"
                );
            }
        }
        let mut artifact = field_artifact(vec![], vec![]);
        artifact.nodes[2].class_names.push("translate-x-4".into());
        validate_glass_schema(&artifact).unwrap();
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
