//! Backend-independent rewrites of validated drawing semantics. Keep overlap decisions here
//! rather than inferring different opacity fast paths in individual executors.

use super::{
    DrawProgram, InstanceBatchNode, InstanceShape, LinearColor, Node, Paint, PaintId, PathData,
    PathVerb,
};

/// Resolve ordinary rectangular View opacity at IR construction too. A shared source paint
/// can appear under different group opacities, so create independent scaled paints and then
/// compact the paint arena in canonical first-use order. Never mutate a shared source paint.
pub(super) fn fold_group_opacity(program: &mut DrawProgram) {
    fold_group_opacity_with_limit(program, super::MAX_PAINTS);
}

fn fold_group_opacity_with_limit(program: &mut DrawProgram, max_paints: usize) {
    let mut opacity_by_child = std::collections::BTreeMap::new();
    let mut groups = Vec::new();
    for (index, node) in program.nodes.iter().enumerate() {
        let Node::Group(group) = node else {
            continue;
        };
        if group.opacity == 1.0 || !group.is_raster_group() || group.children.len() != 1 {
            continue;
        }
        let child = group.children[0].raw() as usize;
        let Node::Path(path) = &program.nodes[child] else {
            continue;
        };
        if !rectangular_path(&program.paths[path.path.raw() as usize]) {
            continue;
        }
        let paint = match (path.fill, path.stroke.as_ref()) {
            (Some(fill), None) => fill,
            (None, Some(stroke)) => stroke.paint,
            _ => continue,
        };
        let Paint::Solid(_) = program.paints[paint.raw() as usize] else {
            continue;
        };
        opacity_by_child.insert(child, group.opacity);
        groups.push(index);
    }
    if groups.is_empty() {
        return;
    }

    // Budget the final arena, not temporary appended paints. Partially rewriting until a
    // temporary limit is hit would free paints during compaction and allow more rewrites on
    // the next validation, violating canonicalization's idempotence.
    let mut paints = Vec::new();
    let mut mapping = std::collections::BTreeMap::new();
    for (index, node) in program.nodes.iter().enumerate() {
        let opacity = opacity_by_child.get(&index).copied().unwrap_or(1.0);
        let mut register = |paint: PaintId| {
            mapping
                .entry((paint.raw(), opacity.to_bits()))
                .or_insert_with(|| {
                    let source = &program.paints[paint.raw() as usize];
                    let scaled = match source {
                        Paint::Solid(color) if opacity != 1.0 => {
                            Paint::Solid(scale(*color, opacity))
                        }
                        _ => source.clone(),
                    };
                    let id = PaintId::from_index(paints.len());
                    paints.push(scaled);
                    id
                });
        };
        match node {
            Node::Path(path) => {
                if let Some(fill) = path.fill {
                    register(fill);
                }
                if let Some(stroke) = &path.stroke {
                    register(stroke.paint);
                }
            }
            Node::GlyphRun(run) => {
                register(run.paint);
                if let Some(stroke) = &run.stroke {
                    register(stroke.paint);
                }
            }
            _ => (),
        }
        if paints.len() > max_paints {
            return;
        }
    }
    for (index, node) in std::sync::Arc::make_mut(&mut program.nodes)
        .iter_mut()
        .enumerate()
    {
        let opacity = opacity_by_child.get(&index).copied().unwrap_or(1.0);
        let remap = |paint: &mut PaintId| {
            let id = mapping[&(paint.raw(), opacity.to_bits())];
            *paint = id;
        };
        match node {
            Node::Path(path) => {
                if let Some(fill) = &mut path.fill {
                    remap(fill);
                }
                if let Some(stroke) = &mut path.stroke {
                    remap(&mut stroke.paint);
                }
            }
            Node::GlyphRun(run) => {
                remap(&mut run.paint);
                if let Some(stroke) = &mut run.stroke {
                    remap(&mut stroke.paint);
                }
            }
            _ => (),
        }
    }
    for index in groups {
        let Node::Group(group) = &mut std::sync::Arc::make_mut(&mut program.nodes)[index] else {
            unreachable!()
        };
        group.opacity = 1.0;
    }
    program.paints = paints.into();
}

/// A single fill or stroke has one coverage mask. Multiplying its premultiplied paint by
/// opacity avoids an isolated surface. Fill plus stroke must still be composited together
/// before applying opacity, including when both paints have the same color.
///
/// This preserves compositing algebra, not historical intermediate F16 rounding. Executors
/// consume the canonical colors; they must not quantize opacity to match an old saveLayer.
pub(super) fn fold_instance_opacity(batch: &mut InstanceBatchNode, paths: &[PathData]) {
    // Curved coverage under antialiased parent clips differs by more than two SDR levels
    // between a direct draw and saveLayer on Raster. Admit rectangles first; images also
    // have a separate tinting contract. These are quality conditions, not byte-equality rules.
    let rectangular = match &batch.shape {
        InstanceShape::Rect => true,
        InstanceShape::RoundRect(rect) => rect.radii.iter().flatten().all(|radius| *radius == 0.0),
        InstanceShape::Path(id) => paths.get(id.raw() as usize).is_some_and(rectangular_path),
        _ => false,
    };
    if !rectangular {
        return;
    }
    let paints_fill = batch.path_style.as_ref().is_none_or(|style| style.fill);
    for row in 0..batch.instances.len() {
        let opacity = batch.instances.opacities[row];
        if opacity == 1.0 {
            continue;
        }
        let color = batch.instances.colors[row];
        let stroke = batch
            .instances
            .stroke_colors
            .get(row)
            .copied()
            .unwrap_or(color);
        let has_fill = paints_fill && color.alpha > 0.0;
        let has_stroke = batch.instances.stroke_widths[row] > 0.0 && stroke.alpha > 0.0;
        if has_fill && has_stroke {
            continue;
        }
        batch.instances.colors[row] = scale(color, opacity);
        if let Some(color) = batch.instances.stroke_colors.get_mut(row) {
            *color = scale(*color, opacity);
        }
        batch.instances.opacities[row] = 1.0;
    }
}

fn rectangular_path(path: &PathData) -> bool {
    if path.verbs
        != [
            PathVerb::MoveTo,
            PathVerb::LineTo,
            PathVerb::LineTo,
            PathVerb::LineTo,
            PathVerb::Close,
        ]
    {
        return false;
    }
    let [a, b, c, d] = path.points.as_slice() else {
        return false;
    };
    // Accept either winding, but reject diagonal edges, collapsed rectangles and bow ties.
    (a[0] == b[0] && b[1] == c[1] && c[0] == d[0] && d[1] == a[1] && a[1] != b[1] && b[0] != c[0])
        || (a[1] == b[1]
            && b[0] == c[0]
            && c[1] == d[1]
            && d[0] == a[0]
            && a[0] != b[0]
            && b[1] != c[1])
}

fn scale(color: LinearColor, opacity: f32) -> LinearColor {
    LinearColor::new(
        color.red * opacity,
        color.green * opacity,
        color.blue * opacity,
        color.alpha * opacity,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Rect, program::*};

    fn pending_groups(shared_paint: bool) -> DrawProgram {
        let mut builder = DrawProgramBuilder::new(Rect::new(0.0, 0.0, 32.0, 32.0));
        let path = builder.push_path(PathData {
            verbs: vec![
                PathVerb::MoveTo,
                PathVerb::LineTo,
                PathVerb::LineTo,
                PathVerb::LineTo,
                PathVerb::Close,
            ],
            points: vec![[0.0, 0.0], [8.0, 0.0], [8.0, 8.0], [0.0, 8.0]],
        });
        let first = builder.push_paint(Paint::Solid(LinearColor::new(0.3, 0.2, 0.1, 0.8)));
        let second = if shared_paint {
            first
        } else {
            builder.push_paint(Paint::Solid(LinearColor::new(0.1, 0.3, 0.2, 0.8)))
        };
        for paint in [first, second] {
            let leaf = builder.push_node(Node::Path(PathNode {
                path,
                fill_rule: FillRule::NonZero,
                fill: Some(paint),
                stroke: None,
            }));
            let root = builder.push_node(Node::Group(Group::plain(vec![leaf])));
            builder.add_root(root);
        }
        let mut program = builder.finish().unwrap();
        // Supply not-yet-canonical opacities to exercise the pass with a small arena budget.
        for (root, opacity) in program.roots.clone().iter().zip([0.3, 0.6]) {
            let Node::Group(group) =
                &mut std::sync::Arc::make_mut(&mut program.nodes)[root.raw() as usize]
            else {
                unreachable!()
            };
            group.opacity = opacity;
        }
        program
    }

    #[test]
    fn paint_budget_counts_final_paints_and_keeps_canonicalization_idempotent() {
        let mut program = pending_groups(false);
        fold_group_opacity_with_limit(&mut program, 3);
        assert_eq!(program.paints.len(), 2);
        assert!(
            program
                .nodes
                .iter()
                .all(|node| !matches!(node, Node::Group(group) if group.opacity != 1.0))
        );
        let once = program.clone();
        fold_group_opacity_with_limit(&mut program, 3);
        assert_eq!(program, once);
        program.validate().unwrap();

        let mut program = pending_groups(true);
        let before = program.clone();
        for _ in 0..2 {
            fold_group_opacity_with_limit(&mut program, 1);
            assert_eq!(
                program, before,
                "budget overflow must not leave a partial rewrite"
            );
        }
    }

    #[test]
    fn groups_with_pixel_operations_keep_their_isolation() {
        for variant in 0..4 {
            let mut program = pending_groups(true);
            for root in program.roots.iter() {
                let Node::Group(group) =
                    &mut std::sync::Arc::make_mut(&mut program.nodes)[root.raw() as usize]
                else {
                    unreachable!()
                };
                match variant {
                    0 => group.clip = Some(Clip::Rect(Rect::new(0.0, 0.0, 4.0, 4.0))),
                    1 => group.internal_blend = BlendMode::Multiply,
                    2 => group.filters.push(Filter::Blur {
                        sigma_x: 1.0,
                        sigma_y: 1.0,
                    }),
                    _ => group.children.clear(),
                }
            }
            let before = program.clone();
            fold_group_opacity(&mut program);
            assert_eq!(program, before);
        }
    }

    #[test]
    fn shared_paints_under_distinct_opacities_do_not_contaminate_other_groups() {
        let mut builder = DrawProgramBuilder::new(Rect::new(0.0, 0.0, 32.0, 32.0));
        let path = builder.push_path(PathData {
            verbs: vec![
                PathVerb::MoveTo,
                PathVerb::LineTo,
                PathVerb::LineTo,
                PathVerb::LineTo,
                PathVerb::Close,
            ],
            points: vec![[0.0, 0.0], [8.0, 0.0], [8.0, 8.0], [0.0, 8.0]],
        });
        let color = LinearColor::new(0.3, 0.2, 0.1, 0.8);
        let paint = builder.push_paint(Paint::Solid(color));
        for opacity in [0.3, 1.0, 0.6, 0.3] {
            let leaf = builder.push_node(Node::Path(PathNode {
                path,
                fill_rule: FillRule::NonZero,
                fill: Some(paint),
                stroke: None,
            }));
            let mut group = Group::plain(vec![leaf]);
            group.opacity = opacity;
            let root = builder.push_node(Node::Group(group));
            builder.add_root(root);
        }
        let p = builder.finish().unwrap();
        p.validate().unwrap();
        assert_eq!(p.paints().len(), 3);
        for (root, opacity) in p.roots().iter().zip([0.3, 1.0, 0.6, 0.3]) {
            let Node::Group(group) = &p.nodes()[root.raw() as usize] else {
                unreachable!()
            };
            assert_eq!(group.opacity, 1.0);
            let Node::Path(path) = &p.nodes()[group.children[0].raw() as usize] else {
                unreachable!()
            };
            assert_eq!(
                p.paints()[path.fill.unwrap().raw() as usize],
                Paint::Solid(scale(color, opacity))
            );
        }
        assert_eq!(
            DrawProgram::from_packed(&p.packed_bytes().unwrap()).unwrap(),
            p
        );
    }

    fn batch(
        fill: bool,
        stroke_width: f32,
        stroke_color: Option<LinearColor>,
    ) -> InstanceBatchNode {
        InstanceBatchNode {
            shape: InstanceShape::Rect,
            path_style: None,
            instances: InstanceColumns {
                transforms: vec![Affine2d([3.0, 0.7, 1.0, 2.0, 4.3, 6.7])],
                colors: vec![if fill {
                    LinearColor::new(2.0, -0.25, 4.0, 0.5)
                } else {
                    LinearColor::new(0.0, 0.0, 0.0, 0.0)
                }],
                stroke_colors: stroke_color.into_iter().collect(),
                opacities: vec![0.37],
                stroke_widths: vec![stroke_width],
                dash_offsets: Vec::new(),
            },
        }
    }

    fn program(batch: InstanceBatchNode) -> DrawProgram {
        let mut builder = DrawProgramBuilder::new(Rect::new(0.0, 0.0, 32.0, 32.0));
        let root = builder.push_node(Node::InstanceBatch(batch));
        builder.add_root(root);
        builder.finish().unwrap()
    }

    #[test]
    fn one_coverage_paint_folds_opacity_without_clamping_hdr_or_affine_geometry() {
        for (fill, width, stroke) in [
            (true, 0.0, None),
            (false, 0.2, None),
            (false, 0.2, Some(LinearColor::new(0.1, 0.2, 0.3, 0.4))),
            (true, 0.2, Some(LinearColor::new(0.0, 0.0, 0.0, 0.0))),
        ] {
            let before = batch(fill, width, stroke);
            let p = program(before.clone());
            let Node::InstanceBatch(after) = &p.nodes()[0] else {
                unreachable!()
            };
            assert_eq!(after.instances.opacities, [1.0]);
            assert_eq!(
                after.instances.colors,
                [scale(before.instances.colors[0], 0.37)]
            );
            assert_eq!(after.instances.transforms, before.instances.transforms);
            assert_eq!(
                after.instances.stroke_widths,
                before.instances.stroke_widths
            );
            if let Some(color) = stroke {
                assert_eq!(after.instances.stroke_colors, [scale(color, 0.37)]);
            }
            p.validate().unwrap();
            assert_eq!(
                DrawProgram::from_packed(&p.packed_bytes().unwrap()).unwrap(),
                p
            );
        }
    }

    #[test]
    fn overlapping_fill_and_stroke_keep_group_opacity() {
        for stroke in [None, Some(LinearColor::new(0.1, 0.2, 0.3, 0.4))] {
            let before = batch(true, 0.2, stroke);
            assert_eq!(
                program(before.clone()).nodes(),
                &[Node::InstanceBatch(before)]
            );
        }
    }

    #[test]
    fn rounded_coverage_keeps_isolation_until_it_passes_clipped_pixel_comparisons() {
        let mut batch = batch(true, 0.0, None);
        batch.shape =
            InstanceShape::RoundRect(RoundRect::circular(Rect::new(0.0, 0.0, 1.0, 1.0), [0.2; 4]));
        batch.path_style = None;
        let original = batch.clone();
        fold_instance_opacity(&mut batch, &[]);
        assert_eq!(batch, original);
    }

    #[test]
    fn zero_opacity_cannot_leave_a_visible_stroke() {
        let mut before = batch(false, 0.2, Some(LinearColor::new(0.1, 0.2, 0.3, 0.4)));
        before.instances.opacities[0] = 0.0;
        let p = program(before);
        let Node::InstanceBatch(after) = &p.nodes()[0] else {
            unreachable!()
        };
        assert_eq!(
            after.instances.colors[0],
            LinearColor::new(0.0, -0.0, 0.0, 0.0)
        );
        assert_eq!(
            after.instances.stroke_colors[0],
            LinearColor::new(0.0, 0.0, 0.0, 0.0)
        );
        assert_eq!(after.instances.opacities, [1.0]);
    }
}
