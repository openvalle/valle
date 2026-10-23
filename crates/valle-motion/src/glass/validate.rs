use crate::NodeKind;
use crate::artifact::{NumberValue, PointValue, SceneArtifact, ValidationError};
use crate::glass::intent::{
    GlassLightBinding, GlassMaterialBinding, MERGE_MAX_DISTANCE, MERGE_MIN_DISTANCE,
};

/// Schema checks for Glass nodes. Production [`SceneArtifact::validate`] calls this after proving
/// that the artifact declares `motion-glass` if and only if it contains Glass topology.
pub fn validate_glass_schema(artifact: &SceneArtifact) -> Result<(), Vec<ValidationError>> {
    let mut errors = Vec::new();
    let (field_of_node, tree_is_valid) = checked_field_membership(artifact);
    if !tree_is_valid {
        errors.push(ValidationError::new(
            "/nodes",
            "Glass validation requires a rooted tree with valid, non-cyclic child links",
        ));
    }
    let mut members = vec![0usize; artifact.nodes.len()];
    for (at, node) in artifact.nodes.iter().enumerate() {
        let path = format!("/nodes/{at}/kind");
        match &node.kind {
            NodeKind::Glass(glass) => {
                let field = field_of_node.get(at).copied().flatten();
                if let Some(field) = field {
                    members[field] += 1;
                }
                match (field, &glass.material, &glass.light) {
                    (Some(_), Some(_), _) => errors.push(ValidationError::new(
                        format!("{path}/material"),
                        format!("Glass '{}' takes its field's material", node.key),
                    )),
                    (Some(_), _, Some(_)) => errors.push(ValidationError::new(
                        format!("{path}/light"),
                        format!("Glass '{}' takes its field's light", node.key),
                    )),
                    (None, None, _) | (None, _, None) => errors.push(ValidationError::new(
                        path.clone(),
                        "a Glass outside a field carries its material and light",
                    )),
                    _ => {}
                }
                if let Some(material) = &glass.material {
                    validate_material(material, &format!("{path}/material"), &mut errors);
                }
                if let Some(light) = &glass.light {
                    validate_light(light, &format!("{path}/light"), &mut errors);
                }
                reject_outside(
                    "presence",
                    &glass.presence,
                    1.0,
                    &format!("{path}/presence"),
                    &mut errors,
                );
            }
            NodeKind::GlassField(field) => {
                if !(MERGE_MIN_DISTANCE..=MERGE_MAX_DISTANCE).contains(&field.merge.distance) {
                    errors.push(ValidationError::new(
                        format!("{path}/merge/distance"),
                        format!(
                            "merge distance must be finite in {MERGE_MIN_DISTANCE}..={MERGE_MAX_DISTANCE}"
                        ),
                    ));
                }
                validate_material(&field.material, &format!("{path}/material"), &mut errors);
                validate_light(&field.light, &format!("{path}/light"), &mut errors);
                // A field directly inside another would split one surface between two materials;
                // inside a member's children it is that member's foreground.
                if let Some(outer) = field_of_node.get(at).copied().flatten() {
                    errors.push(ValidationError::new(
                        path,
                        format!(
                            "GlassField '{}' is inside GlassField '{}'; nest fields only in a member's children",
                            node.key, artifact.nodes[outer].key
                        ),
                    ));
                }
            }
            _ => {}
        }
    }
    for (at, node) in artifact.nodes.iter().enumerate() {
        if matches!(node.kind, NodeKind::GlassField(_)) && members[at] == 0 {
            errors.push(ValidationError::new(
                format!("/nodes/{at}/kind"),
                format!("GlassField '{}' must contain at least one Glass", node.key),
            ));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn validate_material(
    material: &GlassMaterialBinding,
    path: &str,
    errors: &mut Vec<ValidationError>,
) {
    for (name, value) in [
        ("bevel", material.bevel.as_ref()),
        ("refraction", material.refraction.as_ref()),
        ("frost", Some(&material.frost)),
        ("dispersion", Some(&material.dispersion)),
        ("saturation", Some(&material.saturation)),
    ] {
        if let Some(value) = value {
            reject_outside(
                name,
                value,
                f64::INFINITY,
                &format!("{path}/{name}"),
                errors,
            );
        }
    }
}

fn validate_light(light: &GlassLightBinding, path: &str, errors: &mut Vec<ValidationError>) {
    if let PointValue::Static { value } = &light.direction
        && !(value.x.is_finite() && value.y.is_finite() && (value.x != 0.0 || value.y != 0.0))
    {
        errors.push(ValidationError::new(
            format!("{path}/direction"),
            "light direction must be a finite non-zero vector",
        ));
    }
    reject_outside(
        "light elevation",
        &light.elevation,
        1.0,
        &format!("{path}/elevation"),
        errors,
    );
    reject_outside(
        "light intensity",
        &light.intensity,
        f64::INFINITY,
        &format!("{path}/intensity"),
        errors,
    );
}

/// A static value must be finite and in `0..=max`; an expression is checked as each frame draws.
fn reject_outside(
    name: &str,
    value: &NumberValue,
    max: f64,
    path: &str,
    errors: &mut Vec<ValidationError>,
) {
    if let NumberValue::Static { value } = value
        && !(value.is_finite() && (0.0..=max).contains(value))
    {
        errors.push(ValidationError::new(
            path,
            if max.is_finite() {
                format!("{name} must be finite in 0..={max}")
            } else {
                format!("{name} must be a finite non-negative number")
            },
        ));
    }
}

/// The GlassField each node belongs to, by node index: its nearest enclosing field. A Glass's
/// children are its foreground, so a field does not reach past a Glass.
pub fn field_membership(artifact: &SceneArtifact) -> Vec<Option<usize>> {
    checked_field_membership(artifact).0
}

fn checked_field_membership(artifact: &SceneArtifact) -> (Vec<Option<usize>>, bool) {
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
        assigned[node_at] = current;
        let child_field = match &node.kind {
            NodeKind::GlassField(_) => Some(node_at),
            NodeKind::Glass(_) => None,
            _ => current,
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
            stack.push((child_at, child_field));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact::{
        ARTIFACT_FORMAT_VERSION, CapabilitySet, ChildRange, NumberValue, SceneArtifact, SceneNode,
    };
    use crate::controls::ControlsSchema;
    use crate::glass::intent::{
        GlassFieldNode, GlassLightBinding, GlassMaterialBinding, GlassMergeIntent, GlassNode,
    };
    use crate::{NodeId, NodeKind};

    fn node(key: &str, kind: NodeKind, children: ChildRange) -> SceneNode {
        SceneNode {
            key: key.into(),
            kind,
            space: None,
            class_names: vec![],
            class_conditions: Default::default(),
            styles: vec![],
            visibility: None,
            children,
            semantic: None,
        }
    }

    fn field() -> NodeKind {
        NodeKind::GlassField(GlassFieldNode {
            material: GlassMaterialBinding::defaults(),
            light: GlassLightBinding::defaults(),
            merge: GlassMergeIntent::defaults(),
        })
    }

    fn member() -> NodeKind {
        NodeKind::Glass(GlassNode {
            path: None,
            fill_rule: Default::default(),
            material: None,
            light: None,
            presence: NumberValue::Static { value: 1.0 },
        })
    }

    /// root(0) -> field(1) -> wrapper(2) -> member(3).
    fn field_artifact() -> SceneArtifact {
        SceneArtifact {
            role: valle_timeline::MotionRole::Clip,
            camera: None,
            format_version: ARTIFACT_FORMAT_VERSION,
            capability_set: CapabilitySet::base(),
            component: "field-test".into(),
            composition: None,
            controls: ControlsSchema {
                props: Default::default(),
                data: Default::default(),
                assets: Default::default(),
            },
            resource_refs: vec![],
            shaders: Vec::new(),
            exprs: vec![],
            instance_groups: vec![],
            caption_rows: vec![],
            nodes: vec![
                node("root", NodeKind::Group, ChildRange { start: 0, end: 1 }),
                node("orbit", field(), ChildRange { start: 1, end: 2 }),
                node("wrapper", NodeKind::Group, ChildRange { start: 2, end: 3 }),
                node("left", member(), ChildRange::EMPTY),
            ],
            node_children: vec![NodeId(1), NodeId(2), NodeId(3)],
            root: NodeId(0),
        }
    }

    fn messages(artifact: &SceneArtifact) -> Vec<String> {
        validate_glass_schema(artifact)
            .err()
            .unwrap_or_default()
            .into_iter()
            .map(|error| error.message)
            .collect()
    }

    #[test]
    fn a_member_belongs_to_its_nearest_field_through_wrappers() {
        let artifact = field_artifact();
        validate_glass_schema(&artifact).unwrap();
        assert_eq!(field_membership(&artifact), [None, None, Some(1), Some(1)]);
    }

    #[test]
    fn cyclic_and_out_of_bounds_trees_fail_closed() {
        let mut cyclic = field_artifact();
        cyclic.nodes[3].children = ChildRange { start: 2, end: 3 };
        assert!(messages(&cyclic)[0].contains("non-cyclic child links"));

        let mut out_of_bounds = field_artifact();
        out_of_bounds.node_children[2] = NodeId(99);
        assert!(messages(&out_of_bounds)[0].contains("non-cyclic child links"));
    }

    #[test]
    fn a_member_takes_its_fields_material_and_light() {
        let mut artifact = field_artifact();
        let NodeKind::Glass(member) = &mut artifact.nodes[3].kind else {
            panic!("member");
        };
        member.material = Some(GlassMaterialBinding::defaults());
        assert_eq!(
            messages(&artifact),
            ["Glass 'left' takes its field's material"]
        );

        let mut artifact = field_artifact();
        artifact.nodes[1].kind = NodeKind::Group;
        assert_eq!(
            messages(&artifact),
            ["a Glass outside a field carries its material and light"]
        );
    }

    #[test]
    fn a_field_inside_a_field_is_refused_unless_a_member_holds_it() {
        let mut artifact = field_artifact();
        artifact.nodes[2].kind = field();
        let messages = messages(&artifact);
        assert!(
            messages.iter().any(|message| message
                == "GlassField 'wrapper' is inside GlassField 'orbit'; nest fields only in a member's children"),
            "{messages:?}"
        );
        assert!(
            messages
                .iter()
                .any(|message| message == "GlassField 'orbit' must contain at least one Glass"),
            "{messages:?}"
        );

        // root -> field -> member -> field -> member: the inner field is a foreground.
        let mut artifact = field_artifact();
        artifact.nodes[2].kind = member();
        artifact.nodes[3].kind = field();
        artifact.nodes[3].children = ChildRange { start: 3, end: 4 };
        artifact
            .nodes
            .push(node("inner", member(), ChildRange::EMPTY));
        artifact.node_children.push(NodeId(4));
        validate_glass_schema(&artifact).unwrap();
        assert_eq!(
            field_membership(&artifact),
            [None, None, Some(1), None, Some(3)]
        );
    }

    #[test]
    fn static_material_and_light_values_are_checked() {
        let mut artifact = field_artifact();
        let NodeKind::GlassField(field) = &mut artifact.nodes[1].kind else {
            panic!("field");
        };
        field.material.bevel = Some(NumberValue::Static { value: -1.0 });
        field.material.frost = NumberValue::Static { value: f64::NAN };
        field.light.elevation = NumberValue::Static { value: 1.5 };
        field.merge.distance = 0.5;
        assert_eq!(
            messages(&artifact),
            [
                "merge distance must be finite in 1..=128",
                "bevel must be a finite non-negative number",
                "frost must be a finite non-negative number",
                "light elevation must be finite in 0..=1",
            ]
        );
    }
}
