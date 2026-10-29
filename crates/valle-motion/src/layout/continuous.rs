//! Preserve layout coordinates until rasterization. Takumi's convenience entry point snaps
//! boxes to whole device pixels, making slow layout animations jump and changing fractional
//! sizes as their origins move. All Motion layout and measurement paths use this entry point.

use taffy::{RoundTree, TraversePartialTree};
use takumi_core::{
    geometry::{AvailableSpace, Size},
    layout::tree::LayoutTree,
};

pub(crate) fn compute_layout(tree: &mut LayoutTree<'_>, available: Size<AvailableSpace>) {
    let axis = |space| match space {
        AvailableSpace::Definite(value) => taffy::AvailableSpace::Definite(value),
        AvailableSpace::MinContent => taffy::AvailableSpace::MinContent,
        AvailableSpace::MaxContent => taffy::AvailableSpace::MaxContent,
    };
    // Takumi's public LayoutTree constructor assigns index zero to its root.
    let root = taffy::NodeId::from(0usize);
    taffy::compute_root_layout(
        tree,
        root,
        taffy::Size {
            width: axis(available.width),
            height: axis(available.height),
        },
    );
    // Follow the layout tree, including anonymous inline boxes and hoisted absolute/fixed
    // children. Traversing only authored nodes would leave those final layouts uninitialized.
    let mut pending = vec![root];
    while let Some(node) = pending.pop() {
        let layout = tree.get_unrounded_layout(node);
        tree.set_final_layout(node, &layout);
        pending.extend((0..tree.child_count(node)).map(|index| tree.get_child_id(node, index)));
    }
}
