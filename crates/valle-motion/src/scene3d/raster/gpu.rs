//! GPU triangle input using the CPU reference's validated transforms, six-plane clipping,
//! culling and 8-bit subpixel grid. Pixel coverage and depth evaluation remain on the GPU.
use super::{
    AlphaMode, AnchorProjection, Camera, ContractErrors, Frame3DState, Lights, MAX_TILE_REFERENCES,
    MaterialKind, PixelRays, PreparedScene, SUBPIXEL_SCALE, anchor_projections, collect_geometry,
    f16_bits_to_f32, output_color,
};

#[derive(Clone, Copy, Debug)]
pub struct GpuRasterVarying {
    pub normal_over_z: [f32; 3],
    pub material_index: u32,
}

#[derive(Clone, Debug)]
pub struct GpuRasterShading {
    pub vertices: Vec<GpuRasterVarying>,
    /// vec4 records: ambient/exposure, counts/tone, three per material, two per
    /// directional light, three per hemisphere, then camera ray columns and rows.
    pub values: Vec<[f32; 4]>,
}

impl GpuRasterShading {
    pub fn brdf_lut() -> &'static [u8; 1024] {
        super::lighting::DFG_BYTES
    }

    pub fn bytes(&self) -> Vec<u8> {
        self.values
            .iter()
            .flatten()
            .flat_map(|v| v.to_le_bytes())
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GpuRasterVertex {
    /// Pixel coordinates snapped to the reference's 1/256-pixel coverage grid.
    pub position: [f32; 2],
    /// Interpolate linearly in screen space, then invert to recover camera-space depth.
    pub inv_z: f32,
    /// Constant Unlit sRGB at the reference F16 boundary; lit packets use material data.
    pub color: [f32; 4],
    pub object_id: u32,
    pub node_id: u32,
}

#[derive(Clone, Debug)]
pub struct GpuRasterGeometry {
    pub width: u32,
    pub height: u32,
    /// Independent triangles in the same submission order as the CPU renderer.
    pub vertices: Vec<GpuRasterVertex>,
    pub anchors: Vec<AnchorProjection>,
    /// A power of two, so scaling camera depth into a depth attachment is exact.
    pub depth_scale: f32,
    /// Absent for the exact, constant-color Unlit path.
    pub shading: Option<GpuRasterShading>,
}

impl GpuRasterGeometry {
    pub const VERTEX_STRIDE: u64 = 36;

    pub fn vertex_stride(&self) -> u64 {
        Self::VERTEX_STRIDE + if self.shading.is_some() { 16 } else { 0 }
    }

    pub fn vertex_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.vertices.len() * self.vertex_stride() as usize);
        for (index, vertex) in self.vertices.iter().enumerate() {
            for value in vertex
                .position
                .into_iter()
                .chain([vertex.inv_z])
                .chain(vertex.color)
            {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
            bytes.extend_from_slice(&vertex.object_id.to_le_bytes());
            bytes.extend_from_slice(&vertex.node_id.to_le_bytes());
            if let Some(shading) = &self.shading {
                let varying = shading.vertices[index];
                for value in varying.normal_over_z {
                    bytes.extend_from_slice(&value.to_le_bytes());
                }
                bytes.extend_from_slice(&varying.material_index.to_le_bytes());
            }
        }
        bytes
    }
}

/// Prepare untextured opaque/mask geometry and optional Lambert/PBR shading. Unsupported
/// features return `None`; invalid scenes/frames and work-budget violations remain errors.
/// This does not allocate color, depth or ID planes or perform CPU pixel rasterization.
pub fn prepare_gpu_geometry(
    prepared: &PreparedScene,
    frame: &Frame3DState,
) -> Result<Option<GpuRasterGeometry>, ContractErrors> {
    prepared.scene.validate()?;
    frame.validate_for(&prepared.scene)?;
    prepared.budget.validate()?;
    if prepared.scene.pbr.environment.is_some()
        || prepared.scene.pbr.shadows
        || frame
            .camera
            .depth_of_field
            .is_some_and(|dof| dof.max_blur_radius > 0.0)
        || prepared.meshes.iter().any(|mesh| {
            mesh.materials.iter().any(|material| {
                material.values.uses_uv() || material.values.alpha_mode == AlphaMode::Blend
            })
        })
    {
        return Ok(None);
    }
    let camera = Camera::new(frame, prepared.width, prepared.height)?;
    let geometry = collect_geometry(prepared, frame, &camera)?;
    if geometry.tile_references > MAX_TILE_REFERENCES {
        let mut errors = ContractErrors::default();
        errors.push(
            "/budget/tileReferences",
            "projected triangle coverage exceeds the tile work budget",
        );
        return Err(errors);
    }
    let mut vertices = Vec::with_capacity(geometry.commands.len() * 3);
    let lit = prepared
        .meshes
        .iter()
        .any(|mesh| mesh.materials.iter().any(|m| m.kind != MaterialKind::Unlit));
    let mut varyings = Vec::new();
    let mut materials = Vec::new();
    let mut material_ids = std::collections::BTreeMap::new();
    for command in &geometry.commands {
        let mesh = &geometry.frame_meshes[command.mesh_index];
        let material = &mesh.materials[command.material_index].values;
        if material
            .alpha_cutoff
            .is_some_and(|cutoff| material.base_color[3] < cutoff)
        {
            continue;
        }
        let color = if lit {
            [0.0; 4]
        } else {
            let radiance = std::array::from_fn(|i| {
                material.base_color[i] + material.emissive[i] * material.emissive_intensity
            });
            output_color(
                radiance,
                1.0,
                prepared.scene.pbr.tone_mapping,
                frame.exposure,
            )
            .map(f16_bits_to_f32)
        };
        let material_index = if lit {
            *material_ids
                .entry((command.mesh_index, command.material_index))
                .or_insert_with(|| {
                    let index = materials.len() as u32 / 3;
                    let kind = match mesh.materials[command.material_index].kind {
                        MaterialKind::Unlit => 0.0,
                        MaterialKind::Lambert => 1.0,
                        MaterialKind::Pbr => 2.0,
                    };
                    materials.extend([
                        [
                            material.base_color[0],
                            material.base_color[1],
                            material.base_color[2],
                            kind,
                        ],
                        [
                            material.emissive[0] * material.emissive_intensity,
                            material.emissive[1] * material.emissive_intensity,
                            material.emissive[2] * material.emissive_intensity,
                            material.metallic,
                        ],
                        [material.roughness.max(0.0525), 0.0, 0.0, 0.0],
                    ]);
                    index
                })
        } else {
            0
        };
        for (screen, fixed) in command.coverage.vertices.iter().zip(command.coverage.fixed) {
            vertices.push(GpuRasterVertex {
                position: [
                    fixed.x as f32 / SUBPIXEL_SCALE,
                    fixed.y as f32 / SUBPIXEL_SCALE,
                ],
                inv_z: screen.inv_z,
                color,
                object_id: mesh.mesh.object_id,
                node_id: command.node_id,
            });
            if lit {
                varyings.push(GpuRasterVarying {
                    normal_over_z: [
                        screen.normal.x * screen.inv_z,
                        screen.normal.y * screen.inv_z,
                        screen.normal.z * screen.inv_z,
                    ],
                    material_index,
                });
            }
        }
    }
    let mut depth_scale = 1.0;
    while frame.camera.far * depth_scale >= 1.0 {
        depth_scale *= 0.5;
    }
    Ok(Some(GpuRasterGeometry {
        width: prepared.width,
        height: prepared.height,
        vertices,
        anchors: anchor_projections(&prepared.scene, &camera, &geometry.transforms),
        depth_scale,
        shading: lit.then(|| {
            let lights = Lights::new(&prepared.scene, frame, None);
            let mut values = vec![
                [
                    lights.ambient[0],
                    lights.ambient[1],
                    lights.ambient[2],
                    frame.exposure,
                ],
                [
                    (materials.len() / 3) as f32,
                    lights.directional.len() as f32,
                    lights.hemisphere.len() as f32,
                    if prepared.scene.pbr.tone_mapping == crate::scene3d::ToneMapping::Aces {
                        1.0
                    } else {
                        0.0
                    },
                ],
            ];
            values.extend(materials);
            for (direction, color) in &lights.directional {
                values.push([direction.x, direction.y, direction.z, 0.0]);
                values.push([color[0], color[1], color[2], 0.0]);
            }
            for (direction, sky, ground) in &lights.hemisphere {
                values.push([direction.x, direction.y, direction.z, 0.0]);
                values.push([sky[0], sky[1], sky[2], 0.0]);
                values.push([ground[0], ground[1], ground[2], 0.0]);
            }
            let rays = PixelRays::new(&camera);
            values.extend(
                rays.columns
                    .into_iter()
                    .chain(rays.rows)
                    .map(|ray| [ray.x, ray.y, ray.z, 0.0]),
            );
            GpuRasterShading {
                vertices: varyings,
                values,
            }
        }),
    }))
}
