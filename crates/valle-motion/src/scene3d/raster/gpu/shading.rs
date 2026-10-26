use super::*;
use crate::scene3d::{MaterialImage, TextureRole};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone, Copy, Debug)]
pub struct GpuRasterTextureVarying {
    pub uv_over_z: [f32; 2],
    pub dx: [f32; 3],
    pub dy: [f32; 3],
    pub tangent: [f32; 3],
    pub bitangent: [f32; 3],
}

impl GpuRasterShading {
    pub const HEADER_LEN: usize = 5;
    pub const MATERIAL_STRIDE: usize = 9;

    pub(super) fn new(
        prepared: &PreparedScene,
        frame: &Frame3DState,
        meshes: &[super::super::FrameMesh<'_>],
        camera: &Camera,
    ) -> Self {
        let key = |image: &MaterialImage| {
            (
                image.content_digest(),
                match image.role() {
                    TextureRole::Color => 0u8,
                    TextureRole::Data => 1,
                },
            )
        };
        let mut images = BTreeMap::new();
        for mesh in meshes {
            for material in &mesh.materials {
                let m = &material.values;
                for binding in [
                    m.base_color_texture,
                    m.metallic_roughness_texture,
                    m.normal_texture,
                    m.occlusion_texture,
                    m.emissive_texture,
                ]
                .into_iter()
                .flatten()
                {
                    let image = &mesh.mesh.images[binding.image];
                    images
                        .entry(key(image))
                        .or_insert_with(|| Arc::new(image.clone()));
                }
            }
        }
        let ids = images
            .keys()
            .enumerate()
            .map(|(i, k)| (*k, i as u32))
            .collect::<BTreeMap<_, _>>();
        let textures = images.into_values().collect::<Vec<_>>();
        let lights = Lights::new(&prepared.scene, frame, prepared.environment.clone());
        let mut flags =
            u32::from(prepared.scene.pbr.tone_mapping == crate::scene3d::ToneMapping::Aces);
        if prepared
            .scene
            .pbr
            .environment
            .as_ref()
            .is_some_and(|e| e.background)
        {
            flags |= 2;
        }
        if prepared.scene.pbr.shadows {
            flags |= 4;
        }
        let mut values = vec![
            [
                lights.ambient[0],
                lights.ambient[1],
                lights.ambient[2],
                frame.exposure,
            ],
            [
                0.0,
                lights.directional.len() as f32,
                lights.hemisphere.len() as f32,
                flags as f32,
            ],
            [
                camera.eye.x,
                camera.eye.y,
                camera.eye.z,
                lights.environment_rotation[0],
            ],
            [
                frame.environment_intensity,
                lights.environment_rotation[1],
                if prepared.environment.is_some() {
                    textures.len() as f32
                } else {
                    -1.0
                },
                prepared
                    .environment
                    .as_ref()
                    .map_or(0.0, |e| (e.settings().face_size.ilog2() + 1) as f32),
            ],
            [
                if textures.is_empty() { 13.0 } else { 27.0 }, // vertex stride in u32 words
                textures.len() as f32,
                -1.0,
                0.0,
            ],
        ];
        let mut material_offsets = Vec::new();
        for mesh in meshes {
            material_offsets
                .push(((values.len() - Self::HEADER_LEN) / Self::MATERIAL_STRIDE) as u32);
            for material in &mesh.materials {
                let m = &material.values;
                let kind = match material.kind {
                    MaterialKind::Unlit => 0.0,
                    MaterialKind::Lambert => 1.0,
                    MaterialKind::Pbr => 2.0,
                };
                values.extend([
                    [m.base_color[0], m.base_color[1], m.base_color[2], kind],
                    [
                        m.emissive[0] * m.emissive_intensity,
                        m.emissive[1] * m.emissive_intensity,
                        m.emissive[2] * m.emissive_intensity,
                        m.metallic,
                    ],
                    [
                        m.roughness,
                        m.base_color[3],
                        m.alpha_cutoff.unwrap_or(-1.0),
                        if m.alpha_mode == AlphaMode::Blend {
                            1.0
                        } else {
                            0.0
                        },
                    ],
                    [m.normal_scale, m.occlusion_strength, 0.0, 0.0],
                ]);
                for binding in [
                    m.base_color_texture,
                    m.metallic_roughness_texture,
                    m.normal_texture,
                    m.occlusion_texture,
                    m.emissive_texture,
                ] {
                    values.push(binding.map_or([-1.0, 0.0, 0.0, 0.0], |binding| {
                        [
                            ids[&key(&mesh.mesh.images[binding.image])] as f32,
                            binding.gpu_sampler() as f32,
                            0.0,
                            0.0,
                        ]
                    }));
                }
            }
        }
        values[1][0] = ((values.len() - Self::HEADER_LEN) / Self::MATERIAL_STRIDE) as f32;
        for (direction, color) in &lights.directional {
            values.extend([
                [direction.x, direction.y, direction.z, 0.0],
                [color[0], color[1], color[2], 0.0],
            ]);
        }
        for (direction, sky, ground) in &lights.hemisphere {
            values.extend([
                [direction.x, direction.y, direction.z, 0.0],
                [sky[0], sky[1], sky[2], 0.0],
                [ground[0], ground[1], ground[2], 0.0],
            ]);
        }
        let rays = PixelRays::new(camera);
        values.extend(
            rays.columns
                .into_iter()
                .chain(rays.rows)
                .map(|v| [v.x, v.y, v.z, 0.0]),
        );
        Self {
            vertices: Vec::new(),
            texture_vertices: (!textures.is_empty()).then(Vec::new),
            textures,
            shadows: Vec::new(),
            environment: prepared.environment.clone(),
            values,
            material_offsets,
        }
    }

    pub(in crate::scene3d::raster) fn material_id(&self, mesh: usize, material: usize) -> u32 {
        self.material_offsets[mesh] + material as u32
    }
}
