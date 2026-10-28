//! Cold, exact structural interning within one template namespace. This does not simplify
//! arithmetic or rebuild a DrawProgram: doing so could fold opacity and change F16 rounding.
//! Only arena references are replaced with identities of their unchanged referenced values.

use std::collections::BTreeMap;

use super::{Clip, Node, NodeId, PaintId, PaintTemplate, PathId};

#[derive(Default)]
struct Intern {
    entries: BTreeMap<Vec<u8>, u32>,
}

impl Intern {
    fn value(&mut self, domain: &str, value: &impl serde::Serialize) -> Option<u32> {
        // Direct serialization preserves -0 and round-trips every admitted finite f32/f64.
        // Do not pass through serde_json::Value, numeric equality, or a rounded display form.
        let bytes = serde_json::to_vec(&(domain, value)).ok()?;
        let next = self.entries.len() as u32;
        Some(*self.entries.entry(bytes).or_insert(next))
    }
}

pub(super) fn derive(template: &PaintTemplate) -> Vec<Option<u32>> {
    let program = template.program();
    let mut intern = Intern::default();
    let paths: Vec<_> = program
        .paths()
        .iter()
        .map(|p| intern.value("path", p))
        .collect();
    let paints: Vec<_> = program
        .paints()
        .iter()
        .map(|p| intern.value("paint", p))
        .collect();
    let mut nodes = vec![None; program.nodes().len()];
    let mut visited = vec![false; nodes.len()];
    for id in 0..nodes.len() {
        visit(
            template,
            id,
            &paths,
            &paints,
            &mut nodes,
            &mut visited,
            &mut intern,
        );
    }
    // Descriptor strings and interning maps are cold scratch; retain only one optional u32
    // per node. Iteration order is fixed, so cold seeks get the same namespace-local IDs.
    nodes
}

fn visit(
    template: &PaintTemplate,
    id: usize,
    paths: &[Option<u32>],
    paints: &[Option<u32>],
    nodes: &mut [Option<u32>],
    visited: &mut [bool],
    intern: &mut Intern,
) -> Option<u32> {
    if visited[id] {
        return nodes[id];
    }
    visited[id] = true;
    let reads = template.subtree_dependencies(NodeId::from_raw(id as u32))?;
    if !reads.slots().is_empty() || !reads.resources().is_empty() || reads.reads_destination() {
        return None;
    }
    let mut node = template.program().nodes()[id].clone();
    match &mut node {
        Node::Path(path) => {
            path.path = PathId::from_raw(paths[path.path.raw() as usize]?);
            if let Some(fill) = &mut path.fill {
                *fill = PaintId::from_raw(paints[fill.raw() as usize]?);
            }
            if let Some(stroke) = &mut path.stroke {
                stroke.paint = PaintId::from_raw(paints[stroke.paint.raw() as usize]?);
            }
        }
        Node::Group(group) => {
            for child in group
                .children
                .iter_mut()
                .chain(group.mask.iter_mut().map(|m| &mut m.source))
            {
                *child = NodeId::from_raw(visit(
                    template,
                    child.raw() as usize,
                    paths,
                    paints,
                    nodes,
                    visited,
                    intern,
                )?);
            }
            if let Some(Clip::Path { path, .. }) = &mut group.clip {
                *path = PathId::from_raw(paths[path.raw() as usize]?);
            }
        }
        Node::Shadow(_) => {}
        // Instance columns use different packed representations on Web; retain their normal
        // per-node key until both paths have an explicit exact column-content identity.
        _ => return None,
    }
    let identity = intern.value("node", &node)?;
    nodes[id] = Some(identity);
    Some(identity)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Rect, program::*};
    use std::sync::Arc;

    #[test]
    fn serialization_distinguishes_exact_float_bits_and_types() {
        let mut intern = Intern::default();
        let numbers = [
            0.0_f64,
            -0.0,
            f64::from_bits(1),
            0.5,
            f64::from_bits(0.5_f64.to_bits() + 1),
        ];
        let ids: Vec<_> = numbers
            .iter()
            .map(|v| intern.value("number", v).unwrap())
            .collect();
        assert_eq!(
            ids.iter().collect::<std::collections::BTreeSet<_>>().len(),
            numbers.len()
        );
        for (number, id) in numbers.iter().zip(ids) {
            assert_eq!(intern.value("number", number), Some(id));
        }
        assert_ne!(
            intern.value("number", &0.5_f32),
            intern.value("number", &"0.5")
        );
    }

    #[test]
    fn static_copies_share_identity_but_all_drawing_attributes_remain_significant() {
        let mut builder = DrawProgramBuilder::new(Rect::new(0., 0., 64., 64.));
        for variant in 0..8 {
            let path = builder.push_path(PathData {
                verbs: vec![
                    PathVerb::MoveTo,
                    PathVerb::LineTo,
                    PathVerb::LineTo,
                    PathVerb::Close,
                ],
                points: vec![
                    [0., 0.],
                    [if variant == 3 { 63.999 } else { 64. }, 0.],
                    [64., 64.],
                ],
            });
            let red = if variant == 2 {
                f32::from_bits(0.25_f32.to_bits() + 1)
            } else {
                0.25
            };
            let paint = builder.push_paint(Paint::Solid(LinearColor::new(red, 0., 0., 1.)));
            let leaf = Node::Path(PathNode {
                path,
                fill_rule: FillRule::NonZero,
                fill: Some(paint),
                stroke: (variant == 4).then(|| PathStroke::new(paint, 1.)),
            });
            let source = builder.push_node(leaf.clone());
            let mask = builder.push_node(leaf);
            let mut group = Group::plain(vec![source]);
            group.mask = Some(Mask {
                source: mask,
                mode: if variant == 6 {
                    MaskMode::Luminance
                } else {
                    MaskMode::Alpha
                },
            });
            group.clip = Some(Clip::Path {
                path,
                fill_rule: if variant == 7 {
                    FillRule::EvenOdd
                } else {
                    FillRule::NonZero
                },
            });
            if variant == 5 {
                group.opacity = 0.5;
            }
            let root = builder.push_node(Node::Group(group));
            builder.add_root(root);
        }
        let template = Arc::new(PaintTemplate::new(builder.finish().unwrap(), vec![], 0).unwrap());
        let frame = template.bind_frame(vec![]).unwrap();
        let roots = template.program().roots();
        let keys: Vec<_> = roots
            .iter()
            .map(|root| frame.self_contained_subtree_digest(&[*root]).unwrap())
            .collect();
        assert_eq!(keys[0], keys[1]);
        for key in &keys[2..] {
            assert_ne!(key, &keys[0]);
        }
        assert_ne!(
            frame.self_contained_subtree_digest(&[roots[0], roots[2]]),
            frame.self_contained_subtree_digest(&[roots[2], roots[0]])
        );
        let cold = Arc::new(PaintTemplate::from_packed(template.packed().unwrap()).unwrap())
            .bind_frame(vec![])
            .unwrap();
        for (root, expected) in roots.iter().zip(keys) {
            assert_eq!(cold.self_contained_subtree_digest(&[*root]), Some(expected));
        }
    }
}
