//! Locate authored layout boxes in the same stacking order used for paint.

use takumi_core::geometry::NodeId;
use takumi_core::scene::{NodePaint, PaintItemKind, StackingContextNode, build_stacking_contexts};
use takumi_core::style::Affine;

use super::bridge::LayoutTree;
use super::scene::LayoutError;

/// Return the frontmost Scene key whose layout border box contains a viewport point.
///
/// This is a geometry query, not a pixel-alpha or glyph-outline query. Anonymous inline text has
/// no layout box, so a point over its glyphs resolves to the nearest enclosing layout box. The
/// result uses the same stacking contexts as emission and respects visibility, affine transforms,
/// and rectangular overflow clipping. The caller must use the frame's evaluated `LayoutTree`.
pub fn layout_box_at_point(
    tree: &LayoutTree,
    point: [f32; 2],
) -> Result<Option<String>, LayoutError> {
    if !point.iter().all(|value| value.is_finite()) {
        return Ok(None);
    }
    let root_layout = tree
        .layout
        .layout(NodeId::ROOT)
        .map_err(|error| LayoutError::BadPaint {
            node: String::new(),
            reason: format!("root layout unavailable for location query: {error}"),
        })?;
    let contexts = build_stacking_contexts(
        &tree.root,
        &tree.layout,
        NodeId::ROOT,
        Affine::IDENTITY,
        (Some(root_layout.size.width), Some(root_layout.size.height)),
    )
    .map_err(|error| LayoutError::BadPaint {
        node: String::new(),
        reason: format!("stacking contexts unavailable for location query: {error}"),
    })?;
    // The Motion program itself is clipped to the scene canvas.
    let Some(root) = contexts.first().and_then(StackingContextNode::root) else {
        return Ok(None);
    };
    if !contains_box(root, tree, point) {
        return Ok(None);
    }
    let mut found = None;
    visit_context(&contexts, 0, tree, point, &mut found);
    Ok(found)
}

fn visit_context(
    contexts: &[StackingContextNode],
    index: usize,
    tree: &LayoutTree,
    point: [f32; 2],
    found: &mut Option<String>,
) {
    let Some(context) = contexts.get(index) else {
        return;
    };
    if let Some(root) = context.root() {
        update_hit(root, tree, point, found);
        if !contains_overflow_content(root, tree, point) {
            return;
        }
    }
    for bucket in context.in_paint_order() {
        for item in bucket {
            match &item.kind {
                PaintItemKind::Node(node) => update_hit(node, tree, point, found),
                PaintItemKind::Context(child) => {
                    visit_context(contexts, *child, tree, point, found);
                }
            }
        }
    }
}

fn update_hit(node: &NodePaint, tree: &LayoutTree, point: [f32; 2], found: &mut Option<String>) {
    if !contains_box(node, tree, point) {
        return;
    }
    if let Some(key) = tree
        .keys
        .get(&u64::from(node.node_id))
        .or_else(|| tree.render_keys.get(&node.path))
    {
        *found = Some(key.clone());
    }
}

fn local_point(node: &NodePaint, point: [f32; 2]) -> Option<[f32; 2]> {
    let inverse = node.transform.invert()?;
    let (x, y) = inverse.transform_point(point[0], point[1]);
    (x.is_finite() && y.is_finite()).then_some([x, y])
}

fn contains_box(node: &NodePaint, tree: &LayoutTree, point: [f32; 2]) -> bool {
    let (Some(local), Ok(layout)) = (local_point(node, point), tree.layout.layout(node.node_id))
    else {
        return false;
    };
    contains_rect(local, 0.0, 0.0, layout.size.width, layout.size.height)
}

fn contains_overflow_content(node: &NodePaint, tree: &LayoutTree, point: [f32; 2]) -> bool {
    let Some(render) = tree.root.node_at_path(&node.path) else {
        return false;
    };
    if !render.context.style.clips_overflow() {
        return true;
    }
    let (Some(local), Ok(layout)) = (local_point(node, point), tree.layout.layout(node.node_id))
    else {
        return false;
    };
    let padding = crate::emit::padding_box(&layout);
    contains_rect(
        local,
        padding.x as f32,
        padding.y as f32,
        padding.width as f32,
        padding.height as f32,
    )
}

fn contains_rect(point: [f32; 2], x: f32, y: f32, width: f32, height: f32) -> bool {
    width > 0.0
        && height > 0.0
        && point[0] >= x
        && point[0] < x + width
        && point[1] >= y
        && point[1] < y + height
}
