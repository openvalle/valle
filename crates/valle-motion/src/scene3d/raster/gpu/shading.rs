use super::*;
use crate::scene3d::{MaterialImage, TextureRole};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone, Copy, Debug)]
pub struct GpuRasterTextureVarying {
    pub uv: [f32; 2],
    pub dx: [f32; 3],
    pub dy: [f32; 3],
    pub tangent: [f32; 3],
    pub bitangent: [f32; 3],
}

impl GpuRasterShading {
    pub const HEADER_LEN: usize = 6;
    pub const MATERIAL_STRIDE: usize = 23;

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
        if camera.orthographic {
            flags |= 8;
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
                (if textures.is_empty() { 13.0 } else { 27.0 })
                    + if prepared.meshes.iter().any(|m| m.particles.is_some()) {
                        4.0
                    } else {
                        0.0
                    }, // vertex stride in u32 words
                textures.len() as f32,
                -1.0,
                0.0,
            ],
        ];
        values.push([
            lights.punctual.len() as f32,
            lights.area.len() as f32,
            0.0,
            0.0,
        ]);
        let mut material_offsets = Vec::new();
        for mesh in meshes {
            material_offsets
                .push(((values.len() - Self::HEADER_LEN) / Self::MATERIAL_STRIDE) as u32);
            for material in &mesh.materials {
                let m = &material.values;
                let start = values.len();
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
                        if m.needs_blend() { 1.0 } else { 0.0 },
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
                for binding in [
                    m.base_color_texture,
                    m.metallic_roughness_texture,
                    m.normal_texture,
                    m.occlusion_texture,
                    m.emissive_texture,
                ] {
                    let matrix = binding.map_or(
                        super::super::super::asset::ModelTextureBinding::IDENTITY,
                        |b| b.uv_transform,
                    );
                    values.extend(matrix.map(|r| [r[0], r[1], r[2], 0.0]));
                }
                values.extend([
                    [
                        m.transmission,
                        m.thickness,
                        m.attenuation_distance.unwrap_or(0.0),
                        if m.alpha_mode == AlphaMode::Blend {
                            1.0
                        } else {
                            0.0
                        },
                    ],
                    [
                        m.attenuation_color[0],
                        m.attenuation_color[1],
                        m.attenuation_color[2],
                        0.0,
                    ],
                    {
                        let (f0, f90) = m.dielectric();
                        [f0[0], f0[1], f0[2], f90]
                    },
                    [
                        m.ior,
                        m.clearcoat,
                        m.clearcoat_roughness,
                        f32::from(m.thickness_is_local),
                    ],
                ]);
                if m.thickness_is_local {
                    // Texture binding Z/W lanes are reserved for the volume metric.
                    for (i, pair) in m.thickness_metric.as_chunks::<2>().0.iter().enumerate() {
                        values[start + 4 + i][2] = pair[0];
                        values[start + 4 + i][3] = pair[1];
                    }
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
        for light in &lights.punctual {
            let (axis, outer, inner) =
                light
                    .cone
                    .unwrap_or((super::super::V3::new(0.0, 0.0, 0.0), -1.0, -1.0));
            values.extend([
                [
                    light.position.x,
                    light.position.y,
                    light.position.z,
                    light.range,
                ],
                [
                    light.color[0],
                    light.color[1],
                    light.color[2],
                    f32::from(light.cone.is_some()),
                ],
                [axis.x, axis.y, axis.z, outer],
                [inner, 0.0, 0.0, 0.0],
            ]);
        }
        for light in &lights.area {
            for v in [light.position, light.half_width, light.half_height] {
                values.push([v.x, v.y, v.z, 0.0]);
            }
            values.push([light.color[0], light.color[1], light.color[2], 0.0]);
        }
        let rays = PixelRays::new(camera);
        values.extend(
            rays.columns
                .into_iter()
                .chain(rays.rows)
                .map(|v| [v.x, v.y, v.z, 0.0]),
        );
        values.extend([
            [
                camera.right.x,
                camera.right.y,
                camera.right.z,
                camera.focal / camera.aspect,
            ],
            [camera.up.x, camera.up.y, camera.up.z, camera.focal],
            [
                camera.forward.x,
                camera.forward.y,
                camera.forward.z,
                camera.near,
            ],
        ]);
        Self {
            particle_tints: prepared
                .meshes
                .iter()
                .any(|m| m.particles.is_some())
                .then(Vec::new),
            vertices: Vec::new(),
            texture_vertices: (!textures.is_empty()).then(Vec::new),
            textures,
            shadows: Vec::new(),
            shadow_source_key: crate::ContentDigest::of_bytes(&[]),
            environment: prepared.environment.clone(),
            values,
            material_offsets,
        }
    }

    pub(in crate::scene3d::raster) fn material_id(&self, mesh: usize, material: usize) -> u32 {
        self.material_offsets[mesh] + material as u32
    }
}
