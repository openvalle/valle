//! Preserve independent transform identity after CSS cascade and before layout.
use takumi_core::{
    layout::tree::RenderNode,
    style::{Affine, Transform, Transforms},
};

pub(super) fn preserve_identity(root: &mut RenderNode) {
    let mut pending = vec![(root, false)];
    while let Some((node, parent)) = pending.pop() {
        let style = &mut node.context.style;
        let state = style
            .custom_properties
            .get(crate::style::SCALE_IMPORTANT_STATE)
            .or_else(|| style.custom_properties.get(crate::style::SCALE_STATE))
            .map(String::as_str);
        let present = match state {
            Some("present") => true,
            Some("inherit") => parent,
            _ => false,
        };
        if present && !style.contains_fixed_descendants() {
            // An identity transform list preserves containing-block and stacking semantics
            // without changing the matrix. Existing nonempty transform lists stay ordered.
            style.transform = Some(Transforms(
                vec![Transform::Matrix(Affine::IDENTITY)].into_boxed_slice(),
            ));
        }
        if let Some(children) = node.children.as_deref_mut() {
            pending.extend(children.iter_mut().map(|child| (child, present)));
        }
    }
}
