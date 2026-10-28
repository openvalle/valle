//! Admitted value bindings over immutable drawing geometry. This first binding class cannot
//! change bounds, resource requirements, or pass topology; range-affecting bindings must supply
//! their own range analysis before they can join this contract.

use super::{DrawProgram, LinearColor, Paint, PaintId};
use sha2::{Digest, Sha256};
use std::sync::{Arc, OnceLock};

const MAGIC: &[u8; 8] = b"VLPAINT1";

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
    encoding: OnceLock<Vec<u8>>,
    digest: OnceLock<[u8; 32]>,
}

/// A frame owns only its exact working-color values; all geometry belongs to its template.
/// The identity is deliberately different from the hash of a materialized DrawProgram.
#[derive(Debug, Clone)]
pub struct PaintFrame {
    template: Arc<PaintTemplate>,
    values: Vec<LinearColor>,
    program: Arc<DrawProgram>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PaintTemplateError {
    #[error("paint parameter must name a unique existing solid paint and an existing slot")]
    Binding,
    #[error("paint parameter count does not match the template")]
    SlotCount,
    #[error("paint parameter {0} is not a finite premultiplied working color")]
    Color(usize),
    #[error("invalid paint template packet: {0}")]
    Packet(String),
}

impl PaintTemplate {
    pub fn new(
        mut program: DrawProgram,
        mut bindings: Vec<PaintParameter>,
        slots: usize,
    ) -> Result<Self, PaintTemplateError> {
        if slots > super::validate::MAX_PAINTS {
            return Err(PaintTemplateError::Binding);
        }
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
        bindings.sort_by_key(|binding| binding.paint);
        // Initial sample values cannot be part of a template's identity: a cold seek at a
        // different time must generate the same template. Every bound paint is neutralized.
        program.paints = program.paints.replaced(bindings.iter().map(|binding| {
            (
                binding.paint.raw() as usize,
                Paint::Solid(LinearColor::new(0., 0., 0., 0.)),
            )
        }));
        Ok(Self {
            program,
            bindings,
            slots,
            encoding: OnceLock::new(),
            digest: OnceLock::new(),
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

    pub fn bind_frame(
        self: &Arc<Self>,
        values: Vec<LinearColor>,
    ) -> Result<PaintFrame, PaintTemplateError> {
        let program = Arc::new(self.bind(&values)?);
        Ok(PaintFrame {
            template: self.clone(),
            values,
            program,
        })
    }

    pub fn packed(&self) -> Result<&[u8], PaintTemplateError> {
        if self.encoding.get().is_none() {
            let program = self
                .program
                .packed_bytes()
                .map_err(|e| PaintTemplateError::Packet(e.to_string()))?;
            let mut bytes = Vec::with_capacity(16 + self.bindings.len() * 8 + program.len());
            bytes.extend_from_slice(MAGIC);
            bytes.extend_from_slice(&(self.slots as u32).to_le_bytes());
            bytes.extend_from_slice(&(self.bindings.len() as u32).to_le_bytes());
            for binding in &self.bindings {
                bytes.extend_from_slice(&binding.paint.raw().to_le_bytes());
                bytes.extend_from_slice(&(binding.slot as u32).to_le_bytes());
            }
            bytes.extend_from_slice(&program);
            let _ = self.encoding.set(bytes);
        }
        Ok(self.encoding.get().expect("initialized above"))
    }

    pub fn digest(&self) -> Result<[u8; 32], PaintTemplateError> {
        let bytes = self.packed()?;
        Ok(*self.digest.get_or_init(|| Sha256::digest(bytes).into()))
    }

    pub fn from_packed(bytes: &[u8]) -> Result<Self, PaintTemplateError> {
        let invalid = || PaintTemplateError::Packet("size, header or canonical bindings".into());
        if bytes.len() < 16
            || bytes.len()
                > super::validate::MAX_PACKED_BYTES + 16 + super::validate::MAX_PAINTS * 8
            || &bytes[..8] != MAGIC
        {
            return Err(invalid());
        }
        let word =
            |offset| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
        let slots = word(8);
        let count = word(12);
        if slots > super::validate::MAX_PAINTS
            || count > super::validate::MAX_PAINTS
            || 16 + count * 8 >= bytes.len()
        {
            return Err(invalid());
        }
        let bindings: Vec<_> = (0..count)
            .map(|index| PaintParameter {
                paint: PaintId::from_raw(word(16 + index * 8) as u32),
                slot: word(20 + index * 8),
            })
            .collect();
        if bindings
            .windows(2)
            .any(|pair| pair[0].paint >= pair[1].paint)
        {
            return Err(invalid());
        }
        let program = DrawProgram::from_packed(&bytes[16 + count * 8..])
            .map_err(|e| PaintTemplateError::Packet(e.to_string()))?;
        // Refuse alternate encodings of a neutral slot (including negative zero).
        for binding in &bindings {
            match program.paints().get(binding.paint.raw() as usize) {
                Some(Paint::Solid(color))
                    if [color.red, color.green, color.blue, color.alpha]
                        .iter()
                        .all(|c| c.to_bits() == 0) => {}
                _ => return Err(invalid()),
            }
        }
        let template = Self::new(program, bindings, slots)?;
        // Admission verifies a single canonical template once, never a full program per frame.
        if template.packed()? != bytes {
            return Err(invalid());
        }
        Ok(template)
    }
}

impl PaintFrame {
    pub fn template(&self) -> &Arc<PaintTemplate> {
        &self.template
    }
    pub fn program(&self) -> &Arc<DrawProgram> {
        &self.program
    }

    pub fn parameter_bytes(&self) -> Vec<u8> {
        self.values
            .iter()
            .flat_map(|c| [c.red, c.green, c.blue, c.alpha])
            .flat_map(f32::to_le_bytes)
            .collect()
    }

    pub fn from_parameters(
        template: Arc<PaintTemplate>,
        bytes: &[u8],
    ) -> Result<Self, PaintTemplateError> {
        if bytes.len() != template.slots * 16 {
            return Err(PaintTemplateError::SlotCount);
        }
        let values = bytes
            .as_chunks::<16>()
            .0
            .iter()
            .map(|color| {
                let c = |offset| f32::from_le_bytes(color[offset..offset + 4].try_into().unwrap());
                LinearColor::new(c(0), c(4), c(8), c(12))
            })
            .collect();
        template.bind_frame(values)
    }

    /// This does not include output/render specifications; callers that cache rendered pixels
    /// must additionally key those specifications and all referenced resource identities.
    pub fn digest(&self) -> Result<[u8; 32], PaintTemplateError> {
        let mut hash = Sha256::new();
        hash.update(b"valle.paint-frame@1\0");
        hash.update(self.template.digest()?);
        hash.update(self.parameter_bytes());
        Ok(hash.finalize().into())
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

    fn template(color: LinearColor) -> Arc<PaintTemplate> {
        Arc::new(
            PaintTemplate::new(
                program(color),
                vec![PaintParameter {
                    paint: PaintId::from_raw(0),
                    slot: 0,
                }],
                1,
            )
            .unwrap(),
        )
    }

    #[test]
    fn template_and_frame_identity_are_independent_of_first_sample_and_admission() {
        let colors = [
            LinearColor::new(1., 0., 0., 1.),
            LinearColor::new(4., -0.2, 0.5, 0.3),
            LinearColor::new(-0., 0., 0., 0.),
        ];
        let first = template(colors[0]);
        let decoded = Arc::new(PaintTemplate::from_packed(first.packed().unwrap()).unwrap());
        for color in colors {
            let cold = template(color);
            assert_eq!(first.digest().unwrap(), cold.digest().unwrap());
            for candidate in [&first, &cold, &decoded] {
                let frame = candidate.bind_frame(vec![color]).unwrap();
                let roundtrip =
                    PaintFrame::from_parameters(decoded.clone(), &frame.parameter_bytes()).unwrap();
                assert_eq!(roundtrip.digest().unwrap(), frame.digest().unwrap());
                assert_eq!(
                    roundtrip.program().packed_bytes().unwrap(),
                    program(color).packed_bytes().unwrap()
                );
            }
        }
        let positive = first
            .bind_frame(vec![LinearColor::new(0., 0., 0., 0.)])
            .unwrap();
        let negative = first
            .bind_frame(vec![LinearColor::new(-0., 0., 0., 0.)])
            .unwrap();
        assert_ne!(positive.digest().unwrap(), negative.digest().unwrap());
        // Binding layout and static literals are also identity inputs.
        let different_slot = PaintTemplate::new(
            program(colors[0]),
            vec![PaintParameter {
                paint: PaintId::from_raw(0),
                slot: 1,
            }],
            2,
        )
        .unwrap();
        assert_ne!(first.digest().unwrap(), different_slot.digest().unwrap());
        let static_a = PaintTemplate::new(program(colors[0]), vec![], 0).unwrap();
        let static_b = PaintTemplate::new(program(colors[1]), vec![], 0).unwrap();
        assert_ne!(static_a.digest().unwrap(), static_b.digest().unwrap());
    }

    #[test]
    fn template_and_parameters_reject_malformed_packets_without_allocating_from_counts() {
        let template = template(LinearColor::new(1., 0., 0., 1.));
        let packed = template.packed().unwrap();
        for end in 0..packed.len() {
            assert!(PaintTemplate::from_packed(&packed[..end]).is_err());
        }
        for (offset, value) in [(8, u32::MAX), (12, u32::MAX), (16, u32::MAX), (20, 1)] {
            let mut bad = packed.to_vec();
            bad[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            assert!(PaintTemplate::from_packed(&bad).is_err());
        }
        let mut bad = packed.to_vec();
        bad.push(0);
        assert!(PaintTemplate::from_packed(&bad).is_err());
        let parameters = template
            .bind_frame(vec![LinearColor::new(1., 0., 0., 1.)])
            .unwrap()
            .parameter_bytes();
        assert!(PaintFrame::from_parameters(template.clone(), &parameters[..15]).is_err());
        for (offset, value) in [
            (0, f32::NAN),
            (0, f32::INFINITY),
            (12, -0.1),
            (12, 1.1),
            (12, 0.),
        ] {
            let mut bad = parameters.clone();
            bad[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            assert!(PaintFrame::from_parameters(template.clone(), &bad).is_err());
        }
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
