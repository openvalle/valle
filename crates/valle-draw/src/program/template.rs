//! Admitted value bindings over immutable drawing geometry. This first binding class cannot
//! change bounds, resource requirements, or pass topology; range-affecting bindings must supply
//! their own range analysis before they can join this contract.

use super::{DrawProgram, LinearColor, NodeId, Paint, PaintId, SubtreeDependencies};
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
    dependencies: OnceLock<Vec<Arc<SubtreeDependencies>>>,
    subtree_identities: OnceLock<Vec<Option<super::subtree_identity::SubtreeIdentity>>>,
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
    /// The normalized immutable arena. Paint parameters cannot alter this arena's geometry,
    /// requirements, bounds, or execution topology.
    pub fn program(&self) -> &DrawProgram {
        &self.program
    }

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
            dependencies: OnceLock::new(),
            subtree_identities: OnceLock::new(),
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

    /// Derived once per immutable template; hot queries never walk static descendants.
    pub fn subtree_dependencies(&self, node: NodeId) -> Option<&SubtreeDependencies> {
        self.dependencies
            .get_or_init(|| super::dependencies::derive(&self.program, &self.bindings))
            .get(node.raw() as usize)
            .map(Arc::as_ref)
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

    /// Identity of an ordered set of complete, destination-independent subtrees. Only their
    /// actual parameter reads participate. Resolved external-resource identities are not part
    /// of this API, so such subtrees deliberately return None. Pixel caches must additionally
    /// key the device transform, clipping, render specification and backend/context generation.
    pub fn self_contained_subtree_digest(&self, roots: &[NodeId]) -> Option<[u8; 32]> {
        let mut slots = std::collections::BTreeSet::new();
        let mut hash = Sha256::new();
        hash.update(b"valle.paint-subtree@3\0");
        hash.update(self.template.digest().ok()?);
        hash.update((roots.len() as u64).to_le_bytes());
        for root in roots {
            let dependencies = self.template.subtree_dependencies(*root)?;
            if dependencies.reads_destination() || !dependencies.resources().is_empty() {
                return None;
            }
            let identities = self
                .template
                .subtree_identities
                .get_or_init(|| super::subtree_identity::derive(&self.template));
            if let Some(identity) = &identities[root.raw() as usize] {
                hash.update([1]);
                hash.update(identity.shape.to_le_bytes());
                hash.update((identity.slots.len() as u64).to_le_bytes());
                for slot in identity.slots.iter() {
                    let color = self.values[*slot];
                    for lane in [color.red, color.green, color.blue, color.alpha] {
                        hash.update(lane.to_le_bytes());
                    }
                }
                continue;
            }
            hash.update([0]);
            hash.update(root.raw().to_le_bytes());
            slots.extend(dependencies.slots().iter().copied());
        }
        for slot in slots {
            hash.update((slot as u64).to_le_bytes());
            let color = self.values[slot];
            for lane in [color.red, color.green, color.blue, color.alpha] {
                hash.update(lane.to_le_bytes());
            }
        }
        Some(hash.finalize().into())
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

// Standalone PreparedFrame packets own their template. RenderBindings use a separate admitted
// template reference and transmit only parameters; this codec is not their per-frame wire path.
impl serde::Serialize for PaintFrame {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::{Error, SerializeStruct};
        let mut wire = serializer.serialize_struct("PaintFrame", 2)?;
        wire.serialize_field(
            "template",
            self.template.packed().map_err(S::Error::custom)?,
        )?;
        wire.serialize_field("parameters", &self.parameter_bytes())?;
        wire.end()
    }
}

impl<'de> serde::Deserialize<'de> for PaintFrame {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            template: Vec<u8>,
            parameters: Vec<u8>,
        }
        let wire = Wire::deserialize(deserializer)?;
        let template = PaintTemplate::from_packed(&wire.template).map_err(D::Error::custom)?;
        Self::from_parameters(Arc::new(template), &wire.parameters).map_err(D::Error::custom)
    }
}

impl PartialEq for PaintFrame {
    fn eq(&self, other: &Self) -> bool {
        self.template.digest().ok() == other.template.digest().ok()
            && self.parameter_bytes() == other.parameter_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Rect, program::*};
    use std::sync::Arc;

    #[test]
    fn subtree_identity_tracks_exact_consumers_including_masks_and_shared_paints() {
        let mut b = DrawProgramBuilder::new(Rect::new(0., 0., 64., 64.));
        let paints: Vec<_> = (0..3)
            .map(|index| {
                b.push_paint(Paint::Solid(LinearColor::new(
                    index as f32 / 4.,
                    0.,
                    0.,
                    1.,
                )))
            })
            .collect();
        let path = b.push_path(PathData {
            verbs: vec![
                PathVerb::MoveTo,
                PathVerb::LineTo,
                PathVerb::LineTo,
                PathVerb::Close,
            ],
            points: vec![[0., 0.], [64., 0.], [64., 64.]],
        });
        let leaves: Vec<_> = [0, 1, 2, 0]
            .into_iter()
            .map(|index| {
                b.push_node(Node::Path(PathNode {
                    path,
                    fill_rule: FillRule::NonZero,
                    fill: Some(paints[index]),
                    stroke: None,
                }))
            })
            .collect();
        let mut masked = Group::plain(vec![leaves[0]]);
        masked.mask = Some(Mask {
            source: leaves[1],
            mode: MaskMode::Alpha,
        });
        let masked = b.push_node(Node::Group(masked));
        b.add_root(masked);
        b.add_root(leaves[2]);
        b.add_root(leaves[3]);
        let template = Arc::new(
            PaintTemplate::new(
                b.finish().unwrap(),
                vec![
                    PaintParameter {
                        paint: paints[0],
                        slot: 0,
                    },
                    PaintParameter {
                        paint: paints[1],
                        slot: 1,
                    },
                ],
                2,
            )
            .unwrap(),
        );
        let roots = template.program().roots();
        assert_eq!(
            template.subtree_dependencies(roots[0]).unwrap().slots(),
            &[0, 1]
        );
        assert!(
            template
                .subtree_dependencies(roots[1])
                .unwrap()
                .slots()
                .is_empty()
        );
        assert_eq!(
            template.subtree_dependencies(roots[2]).unwrap().slots(),
            &[0]
        );
        let values = [
            LinearColor::new(0.2, 0., 0., 1.),
            LinearColor::new(0.4, 0., 0., 1.),
        ];
        let baseline = template.bind_frame(values.to_vec()).unwrap();
        for slot in 0..2 {
            let mut changed = values;
            // A one-ULP change is a distinct dependency, even when an 8-bit output is equal.
            changed[slot].red = f32::from_bits(changed[slot].red.to_bits() + 1);
            let frame = template.bind_frame(changed.to_vec()).unwrap();
            assert_ne!(
                baseline.self_contained_subtree_digest(&roots[..1]),
                frame.self_contained_subtree_digest(&roots[..1])
            );
            assert_eq!(
                baseline.self_contained_subtree_digest(&roots[1..2]),
                frame.self_contained_subtree_digest(&roots[1..2])
            );
            assert_eq!(
                baseline.self_contained_subtree_digest(&roots[2..])
                    == frame.self_contained_subtree_digest(&roots[2..]),
                slot == 1,
            );
        }
        assert_ne!(
            baseline.self_contained_subtree_digest(&[roots[0], roots[2]]),
            baseline.self_contained_subtree_digest(&[roots[2], roots[0]])
        );
        assert!(
            baseline
                .self_contained_subtree_digest(&[NodeId::from_raw(u32::MAX)])
                .is_none()
        );
        let cold = Arc::new(PaintTemplate::from_packed(template.packed().unwrap()).unwrap());
        let cold = cold.bind_frame(values.to_vec()).unwrap();
        for root in roots {
            assert_eq!(
                baseline.self_contained_subtree_digest(&[*root]),
                cold.self_contained_subtree_digest(&[*root])
            );
        }
    }

    #[test]
    fn resource_and_destination_reads_cannot_use_a_self_contained_cache_key() {
        use crate::requirements::{
            AlphaMode, ColorDomain, ExternalTexture, SamplingMode, TextureKind,
        };
        let mut b = DrawProgramBuilder::new(Rect::new(0., 0., 64., 64.));
        let texture = ExternalTexture {
            key: "test://image".into(),
            kind: TextureKind::Image,
            color_domain: ColorDomain::LinearRec2020,
            alpha: AlphaMode::Premultiplied,
            sample_time_micros: None,
        };
        let image = b.push_node(Node::Image(ImageNode {
            texture: texture.clone(),
            src: Rect::new(0., 0., 1., 1.),
            dst: Rect::new(0., 0., 64., 64.),
            sampling: SamplingMode::LinearClamp,
            opacity: 1.,
        }));
        b.add_root(image);
        let template = Arc::new(PaintTemplate::new(b.finish().unwrap(), vec![], 0).unwrap());
        let root = template.program().roots()[0];
        assert_eq!(
            template.subtree_dependencies(root).unwrap().resources(),
            &[SubtreeResource::Texture(texture)]
        );
        assert!(
            template
                .bind_frame(vec![])
                .unwrap()
                .self_contained_subtree_digest(&[root])
                .is_none()
        );

        let source = program(LinearColor::new(0.5, 0., 0., 1.));
        let mut b = DrawProgramBuilder::new(source.viewport());
        b.push_path(source.paths()[0].clone());
        b.push_paint(source.paints()[0].clone());
        let leaf = b.push_node(source.nodes()[0].clone());
        let mut group = Group::plain(vec![leaf]);
        group.internal_blend = BlendMode::Multiply;
        let root = b.push_node(Node::Group(group));
        b.add_root(root);
        let template = Arc::new(PaintTemplate::new(b.finish().unwrap(), vec![], 0).unwrap());
        let root = template.program().roots()[0];
        assert!(
            template
                .subtree_dependencies(root)
                .unwrap()
                .reads_destination()
        );
        assert!(
            template
                .bind_frame(vec![])
                .unwrap()
                .self_contained_subtree_digest(&[root])
                .is_none()
        );
    }

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
