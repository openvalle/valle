//! Rendering configuration for an independent particle system.
use super::{
    AdmittedModel, Color4, ContractErrors, ModelInstance, ModelMesh, ModelNode, ModelVertex,
    PrimitiveRange, asset::Affine,
};
use crate::particles3d::ParticleSystem3D;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum ParticleShape3D {
    Circle,
    Quad,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Particles3DSpec {
    pub motion: ParticleSystem3D,
    pub shape: ParticleShape3D,
    /// Diameter/side in emitter-local world units at birth and death.
    pub sizes: [f32; 2],
    /// sRGB author colors; interpolate in linear light during sampling.
    pub colors: [Color4; 2],
    pub opacities: [f32; 2],
    /// Billboard rotation in degrees, from birth to death.
    pub rotations: [f32; 2],
}
impl Particles3DSpec {
    pub fn triangle_count(&self) -> u32 {
        self.motion.count.saturating_mul(match self.shape {
            ParticleShape3D::Circle => 16,
            ParticleShape3D::Quad => 2,
        })
    }
    pub fn vertex_count(&self) -> u32 {
        self.motion.count.saturating_mul(match self.shape {
            ParticleShape3D::Circle => 17,
            ParticleShape3D::Quad => 4,
        })
    }
    pub fn validate(&self) -> Result<(), ContractErrors> {
        let mut errors = ContractErrors::default();
        if let Err(e) = self.motion.validate() {
            errors.push("/particles/motion", e.to_string());
        }
        if self
            .sizes
            .iter()
            .any(|v| !v.is_finite() || !(0.0..=10000.0).contains(v))
            || self
                .opacities
                .iter()
                .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
            || self
                .rotations
                .iter()
                .any(|v| !v.is_finite() || v.abs() > super::MAX_ABS_ROTATION_DEGREES)
            || self
                .colors
                .iter()
                .flat_map(|c| c.0)
                .any(|v| !v.is_finite() || !(0.0..=1.0).contains(&v))
        {
            errors.push("/particles/appearance","particle life endpoints must be finite, nonnegative sizes, bounded colors/opacities and rotations");
        }
        if self.vertex_count() > super::MAX_VERTICES || self.triangle_count() > super::MAX_TRIANGLES
        {
            errors.push(
                "/particles/count",
                "particle shape expansion exceeds the scene geometry budget",
            );
        }
        errors.finish()
    }
}

/// One small UV carrier per emitter, never one model/node per particle. Actual
/// billboards are emitted in the shared geometry stage with current camera axes.
pub(super) fn template() -> AdmittedModel {
    let vertices = [
        [-0.5, -0.5, 0.0],
        [0.5, -0.5, 0.0],
        [0.5, 0.5, 0.0],
        [-0.5, 0.5, 0.0],
    ]
    .into_iter()
    .map(|position| ModelVertex {
        position,
        normal: [0.0, 0.0, 1.0],
        uv: [position[0] + 0.5, 0.5 - position[1]],
    })
    .collect();
    let identity = Affine::identity();
    AdmittedModel {
        content_digest: crate::ContentDigest::of_bytes(b"valle-particle-billboard-v1"),
        source_bytes: 152,
        vertices,
        indices: vec![0, 1, 2, 0, 2, 3],
        primitives: vec![PrimitiveRange {
            first_vertex: 0,
            vertex_count: 4,
            first_index: 0,
            index_count: 6,
            material_index: None,
            has_uv: true,
            has_normal: true,
        }],
        meshes: vec![ModelMesh {
            first_primitive: 0,
            primitive_count: 1,
            vertex_count: 4,
            triangle_count: 2,
            morph_target_count: 0,
        }],
        nodes: vec![ModelNode {
            id: 0,
            name: None,
            parent: None,
            mesh: Some(0),
            transform: [
                1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
            ],
            active: true,
        }],
        instances: vec![ModelInstance {
            node: 0,
            mesh: 0,
            skin: None,
            transform: identity,
            weights: Vec::new(),
            joint_palette: Vec::new(),
        }],
        morph_targets: vec![Vec::new()],
        skin_vertices: vec![None],
        skins: Vec::new(),
        materials: Vec::new(),
        images: Vec::new(),
        animations: Vec::new(),
        source_trs: vec![None],
    }
}
