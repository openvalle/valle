// Real-time construction: replace ordered runs of ordinary ellipse leaves with one
// compact instance table. Parent opacity, effects, painter order and noneligible nodes survive.
// Enabled only inside an owner-scoped compilation call, before canonicalization and validation.
use crate::program::{
    Affine2d, BlendSpace, Clip, DrawProgramBuilder, InstanceBatchNode, InstanceColumns,
    InstancePathStyle, InstanceShape, LinearColor, Node, NodeId, Paint, PaintId, PathData, PathId,
    PathVerb, Transform2d,
};

const MIN_RUN: usize = 64;
const K: f64 = 0.5522847498307936;
const UNIT: [[f64; 2]; 13] = [
    [1., 0.],
    [1., K],
    [K, 1.],
    [0., 1.],
    [-K, 1.],
    [-1., K],
    [-1., 0.],
    [-1., -K],
    [-K, -1.],
    [0., -1.],
    [K, -1.],
    [1., -K],
    [1., 0.],
];
fn unit_path() -> PathData {
    PathData {
        verbs: vec![
            PathVerb::MoveTo,
            PathVerb::CubicTo,
            PathVerb::CubicTo,
            PathVerb::CubicTo,
            PathVerb::CubicTo,
        ],
        points: UNIT.to_vec(),
    }
}
type Row = (Affine2d, LinearColor, f32);

fn leaf(
    source: &DrawProgramBuilder,
    id: NodeId,
    transform: Transform2d,
    opacity: f32,
    depth: usize,
) -> Option<Row> {
    if depth > 12 {
        return None;
    }
    match source.nodes.get(id.index())?.as_ref()? {
        Node::Group(g) => {
            if !g.is_raster_group()
                || g.blend_space != BlendSpace::Srgb
                || g.children.len() != 1
                || (opacity < 1. && g.opacity < 1.)
            {
                return None;
            }
            leaf(
                source,
                g.children[0],
                g.transform.then(transform),
                opacity * g.opacity,
                depth + 1,
            )
        }
        Node::Path(p) => {
            if p.stroke.is_some() {
                return None;
            }
            let Paint::Solid(color) = source.paints.get(p.fill?.index())? else {
                return None;
            };
            let path = source.paths.get(p.path.index())?;
            if path.verbs.as_slice()
                != [
                    PathVerb::MoveTo,
                    PathVerb::CubicTo,
                    PathVerb::CubicTo,
                    PathVerb::CubicTo,
                    PathVerb::CubicTo,
                ]
                || path.points.len() != 13
            {
                return None;
            }
            let points = &path.points;
            let cx = (points[0][0] + points[6][0]) / 2.;
            let cy = (points[3][1] + points[9][1]) / 2.;
            let rx = (points[0][0] - points[6][0]) / 2.;
            let ry = (points[3][1] - points[9][1]) / 2.;
            let t = transform.0;
            if rx <= 0. || ry <= 0. || rx > 4. || ry > 4. || t[6] != 0. || t[7] != 0. || t[8] != 1.
            {
                return None;
            }
            let tolerance = rx.max(ry) * 1e-5 + 1e-7;
            if points.iter().zip(UNIT).any(|(p, u)| {
                (p[0] - cx - u[0] * rx).abs() > tolerance
                    || (p[1] - cy - u[1] * ry).abs() > tolerance
            }) {
                return None;
            }
            Some((
                Affine2d([
                    t[0] * rx,
                    t[3] * rx,
                    t[1] * ry,
                    t[4] * ry,
                    t[0] * cx + t[1] * cy + t[2],
                    t[3] * cx + t[4] * cy + t[5],
                ]),
                *color,
                opacity,
            ))
        }
        _ => None,
    }
}

struct Rewriter<'a> {
    source: &'a DrawProgramBuilder,
    output: DrawProgramBuilder,
    nodes: Vec<Option<NodeId>>,
    paths: Vec<Option<PathId>>,
    paints: Vec<Option<PaintId>>,
    unit: Option<PathId>,
}
impl Rewriter<'_> {
    fn path(&mut self, old: PathId) -> PathId {
        if let Some(mapped) = self.paths[old.index()] {
            return mapped;
        }
        let id = self
            .output
            .push_path(self.source.paths[old.index()].clone());
        self.paths[old.index()] = Some(id);
        id
    }
    fn paint(&mut self, old: PaintId) -> PaintId {
        if let Some(mapped) = self.paints[old.index()] {
            return mapped;
        }
        let id = self
            .output
            .push_paint(self.source.paints[old.index()].clone());
        self.paints[old.index()] = Some(id);
        id
    }
    fn children(&mut self, children: &[NodeId]) -> Vec<NodeId> {
        let mut result = Vec::with_capacity(children.len());
        let mut at = 0;
        while at < children.len() {
            let mut rows = Vec::new();
            while at + rows.len() < children.len() {
                let Some(row) = leaf(
                    self.source,
                    children[at + rows.len()],
                    Transform2d::IDENTITY,
                    1.,
                    0,
                ) else {
                    break;
                };
                rows.push(row);
            }
            if rows.len() >= MIN_RUN {
                let path = match self.unit {
                    Some(path) => path,
                    None => {
                        let path = self.output.push_path(unit_path());
                        self.unit = Some(path);
                        path
                    }
                };
                let mut instances = InstanceColumns::with_capacity(rows.len());
                for (transform, color, opacity) in &rows {
                    instances.transforms.push(*transform);
                    instances.colors.push(*color);
                    instances.opacities.push(*opacity);
                    instances.stroke_widths.push(0.);
                }
                result.push(
                    self.output
                        .push_node(Node::InstanceBatch(InstanceBatchNode {
                            shape: InstanceShape::Path(path),
                            instances,
                            path_style: Some(InstancePathStyle::default()),
                        })),
                );
                at += rows.len();
            } else {
                // A failed eligibility check is a painter-order boundary; never cross it.
                let count = rows.len().max(1);
                for &child in &children[at..at + count] {
                    result.push(self.node(child));
                }
                at += count;
            }
        }
        result
    }
    fn node(&mut self, old: NodeId) -> NodeId {
        if let Some(mapped) = self.nodes[old.index()] {
            return mapped;
        }
        let id = self.output.reserve_node();
        self.nodes[old.index()] = Some(id);
        let source = self.source.nodes[old.index()]
            .as_ref()
            .expect("compiler constructed node")
            .clone();
        let mapped = match source {
            Node::Group(mut g) => {
                g.children = self.children(&g.children);
                if let Some(Clip::Path { path, .. }) = &mut g.clip {
                    *path = self.path(*path);
                }
                if let Some(mask) = &mut g.mask {
                    mask.source = self.node(mask.source);
                }
                Node::Group(g)
            }
            Node::Path(mut p) => {
                p.path = self.path(p.path);
                p.fill = p.fill.map(|id| self.paint(id));
                if let Some(stroke) = &mut p.stroke {
                    stroke.paint = self.paint(stroke.paint);
                }
                Node::Path(p)
            }
            Node::InstanceBatch(mut batch) => {
                if let InstanceShape::Path(path) = &mut batch.shape {
                    *path = self.path(*path);
                }
                Node::InstanceBatch(batch)
            }
            Node::GlyphRun(mut glyph) => {
                glyph.outline = glyph.outline.map(|id| self.path(id));
                glyph.paint = self.paint(glyph.paint);
                if let Some(stroke) = &mut glyph.stroke {
                    stroke.paint = self.paint(stroke.paint);
                }
                Node::GlyphRun(glyph)
            }
            other => other,
        };
        self.output
            .define_node(id, mapped)
            .expect("fresh node reservation");
        id
    }
}
pub(super) fn compact(source: DrawProgramBuilder) -> DrawProgramBuilder {
    // Small scenes retain the original builder; a compact table is worthwhile for long runs.
    if source.nodes.len() < MIN_RUN {
        return source;
    }
    let mut rewrite = Rewriter {
        source: &source,
        output: DrawProgramBuilder::new(source.viewport),
        nodes: vec![None; source.nodes.len()],
        paths: vec![None; source.paths.len()],
        paints: vec![None; source.paints.len()],
        unit: None,
    };
    let roots = rewrite.children(&source.roots);
    for root in roots {
        rewrite.output.add_root(root);
    }
    rewrite.output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Rect,
        program::{FillRule, Group, PathNode, PathStroke},
    };
    fn circles() -> DrawProgramBuilder {
        let mut builder = DrawProgramBuilder::new(Rect::new(0., 0., 1920., 1080.));
        let paint = builder.push_paint(Paint::Solid(LinearColor::new(0.3, 0.2, 0.1, 1.)));
        let mut children = Vec::new();
        for i in 0..80 {
            let mut path = unit_path();
            for p in &mut path.points {
                p[0] = p[0] * 3. + i as f64 * 10.;
                p[1] = p[1] * 2. + 40.;
            }
            let path = builder.push_path(path);
            let path = builder.push_node(Node::Path(PathNode {
                path,
                fill_rule: FillRule::NonZero,
                fill: Some(paint),
                stroke: None,
            }));
            let mut group = Group::plain(vec![path]);
            group.opacity = 0.7;
            children.push(builder.push_node(Node::Group(group)));
        }
        let mut echo = Group::plain(children);
        echo.opacity = 0.4;
        let root = builder.push_node(Node::Group(echo));
        builder.add_root(root);
        builder
    }
    #[test]
    fn ellipse_batch_keeps_parent_isolation_and_cubic_geometry() {
        let p = compact(circles()).finish().unwrap();
        assert_eq!(p.nodes().len(), 2);
        assert_eq!(p.paths().len(), 1);
        assert!(p.paints().is_empty());
        let Node::Group(g) = &p.nodes()[0] else {
            panic!()
        };
        assert_eq!(g.opacity, 0.4);
        let Node::InstanceBatch(b) = &p.nodes()[1] else {
            panic!()
        };
        assert_eq!(b.instances.len(), 80);
        assert_eq!(b.instances.opacities, vec![0.7; 80]);
        assert_eq!(b.instances.transforms[79].0, [3., 0., 0., 2., 790., 40.]);
        assert!(matches!(b.shape, InstanceShape::Path(_)));
        assert_eq!(p.paths()[0], unit_path());
    }
    #[test]
    fn clipping_and_strokes_split_runs_and_remap_reachable_tables() {
        let mut source = circles();
        if let Some(Node::Group(g)) = &mut source.nodes[81] {
            g.clip = Some(Clip::Rect(Rect::new(0., 0., 100., 100.)));
        }
        if let Some(Node::Path(p)) = &mut source.nodes[120] {
            p.stroke = Some(PathStroke::new(p.fill.unwrap(), 1.));
        }
        let p = compact(source).finish().unwrap();
        assert!(
            !p.nodes()
                .iter()
                .any(|n| matches!(n, Node::InstanceBatch(_)))
        );
        assert!(
            p.nodes()
                .iter()
                .any(|n| matches!(n, Node::Group(g) if g.clip.is_some()))
        );
        assert!(
            p.nodes()
                .iter()
                .any(|n| matches!(n, Node::Path(p) if p.stroke.is_some()))
        );
    }
    #[test]
    fn two_opacity_layers_are_not_folded_into_one_instance() {
        let mut source = circles();
        let Some(Node::Group(parent)) = source.nodes.last().cloned().flatten() else {
            panic!()
        };
        let mut children = Vec::new();
        for child in parent.children {
            let mut g = Group::plain(vec![child]);
            g.opacity = 0.5;
            children.push(source.push_node(Node::Group(g)));
        }
        if let Some(Node::Group(parent)) = &mut source.nodes[160] {
            parent.children = children;
        }
        let p = compact(source).finish().unwrap();
        assert_eq!(p.nodes().len(), 241);
        assert!(
            !p.nodes()
                .iter()
                .any(|n| matches!(n, Node::InstanceBatch(_)))
        );
    }
}

thread_local! { static ENABLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }

/// Geometry normalization for an explicitly selected real-time backend. Synchronous and scoped:
/// another engine or a reference frame cannot inherit the setting, including after an unwind.
pub fn with_instance_compaction<T>(enabled: bool, build: impl FnOnce() -> T) -> T {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            ENABLED.with(|flag| flag.set(self.0));
        }
    }
    let previous = ENABLED.with(|flag| flag.replace(enabled));
    let _restore = Restore(previous);
    build()
}
pub(super) fn enabled() -> bool {
    ENABLED.with(std::cell::Cell::get)
}

#[cfg(test)]
mod scope_tests {
    use super::*;
    #[test]
    fn compaction_scope_is_restored_after_nested_calls_and_unwind() {
        assert!(!enabled());
        with_instance_compaction(true, || {
            assert!(enabled());
            with_instance_compaction(false, || assert!(!enabled()));
            assert!(enabled());
            let failure = std::panic::catch_unwind(|| {
                with_instance_compaction(false, || panic!("test unwind"))
            });
            assert!(failure.is_err());
            assert!(enabled());
        });
        assert!(!enabled());
    }
}
