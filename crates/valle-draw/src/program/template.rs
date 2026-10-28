//! Admitted value bindings over immutable drawing geometry. This first binding class cannot
//! change bounds, resource requirements, or pass topology; range-affecting bindings must supply
//! their own range analysis before they can join this contract.

use super::{DrawProgram, LinearColor, Paint, PaintId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaintParameter {
    pub paint: PaintId,
    pub slot: usize,
}

#[derive(Debug, Clone)]
pub struct PaintTemplate {
    program: DrawProgram,
    bindings: Vec<PaintParameter>,
    slots: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PaintTemplateError {
    #[error("paint parameter must name a unique existing solid paint and an existing slot")]
    Binding,
    #[error("paint parameter count does not match the template")]
    SlotCount,
    #[error("paint parameter {0} is not a finite premultiplied working color")]
    Color(usize),
}

impl PaintTemplate {
    pub fn new(
        program: DrawProgram,
        bindings: Vec<PaintParameter>,
        slots: usize,
    ) -> Result<Self, PaintTemplateError> {
        let mut seen = std::collections::BTreeSet::new();
        for binding in &bindings {
            if binding.slot >= slots
                || !seen.insert(binding.paint)
                || !matches!(
                    program.paints().get(binding.paint.raw() as usize),
                    Some(Paint::Solid(_))
                )
            {
                return Err(PaintTemplateError::Binding);
            }
        }
        Ok(Self {
            program,
            bindings,
            slots,
        })
    }

    /// Bind only paints. The validated node/path/range arenas remain shared with the template;
    /// there is no layout, recording compilation, or whole-program revalidation on this path.
    pub fn bind(&self, values: &[LinearColor]) -> Result<DrawProgram, PaintTemplateError> {
        if values.len() != self.slots {
            return Err(PaintTemplateError::SlotCount);
        }
        for (slot, color) in values.iter().enumerate() {
            if !color.is_finite()
                || (color.alpha == 0.0
                    && (color.red != 0.0 || color.green != 0.0 || color.blue != 0.0))
            {
                return Err(PaintTemplateError::Color(slot));
            }
        }
        let mut bound = self.program.clone();
        bound.paints = self
            .program
            .paints
            .replaced(self.bindings.iter().map(|binding| {
                (
                    binding.paint.raw() as usize,
                    Paint::Solid(values[binding.slot]),
                )
            }));
        Ok(bound)
    }

    pub fn bindings(&self) -> &[PaintParameter] {
        &self.bindings
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Rect, program::*};
    use std::sync::Arc;

    fn program(color: LinearColor) -> DrawProgram {
        let mut b = DrawProgramBuilder::new(Rect::new(0., 0., 64., 64.));
        let paint = b.push_paint(Paint::Solid(color));
        let path = b.push_path(PathData {
            verbs: vec![
                PathVerb::MoveTo,
                PathVerb::LineTo,
                PathVerb::LineTo,
                PathVerb::Close,
            ],
            points: vec![[0., 0.], [64., 0.], [64., 64.]],
        });
        let root = b.push_node(Node::Path(PathNode {
            path,
            fill_rule: FillRule::NonZero,
            fill: Some(paint),
            stroke: None,
        }));
        b.add_root(root);
        b.finish().unwrap()
    }

    #[test]
    fn binding_shares_geometry_and_matches_full_build_in_any_order() {
        let initial = program(LinearColor::new(1., 0., 0., 1.));
        let template = PaintTemplate::new(
            initial.clone(),
            vec![PaintParameter {
                paint: PaintId::from_index(0),
                slot: 0,
            }],
            1,
        )
        .unwrap();
        for color in [
            LinearColor::new(4., -0.2, 0.5, 0.3),
            LinearColor::new(0., 0., 0., 0.),
            LinearColor::new(1., 0., 0., 1.),
        ] {
            let bound = template.bind(&[color]).unwrap();
            bound.validate().unwrap();
            assert!(Arc::ptr_eq(&bound.nodes, &initial.nodes));
            assert!(Arc::ptr_eq(&bound.paths, &initial.paths));
            assert!(Arc::ptr_eq(&bound.node_geometry, &initial.node_geometry));
            assert_eq!(
                bound.packed_bytes().unwrap(),
                program(color).packed_bytes().unwrap()
            );
        }
        assert!(template.bind(&[]).is_err());
        assert!(template.bind(&[LinearColor::new(1., 0., 0., 0.)]).is_err());
        assert!(
            template
                .bind(&[LinearColor::new(f32::NAN, 0., 0., 1.)])
                .is_err()
        );
        assert!(template.bind(&[LinearColor::new(0., 0., 0., 1.1)]).is_err());
    }
}
