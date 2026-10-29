//! GPU inputs: retained source meshes with vertex-stage deformation, or the projected
//! CPU reference packet for features that still consume exact CPU geometry. Both use
//! GPU coverage and depth; projected packets keep CPU six-plane clipping and snapping.
use super::{
    AlphaMode, AnchorProjection, Camera, ContractErrors, Frame3DState, Lights, MAX_TILE_REFERENCES,
    MaterialKind, PixelRays, PreparedScene, SUBPIXEL_SCALE, anchor_projections, collect_geometry,
    f16_bits_to_f32, output_color,
};
mod atlas;
mod mesh;
mod packet;
mod shading;
pub use atlas::GpuRasterAtlas;
pub(super) use mesh::GpuMeshCache;
pub use mesh::{GpuMeshData, prepare_gpu_mesh_geometry};
pub use shading::GpuRasterTextureVarying;

#[derive(Clone, Copy, Debug)]
pub struct GpuRasterVarying {
    pub normal_over_z: [f32; 3],
    pub material_index: u32,
}

#[derive(Clone, Debug)]
pub struct GpuRasterShading {
    pub particle_tints: Option<Vec<[f32; 4]>>,
    pub vertices: Vec<GpuRasterVarying>,
    pub texture_vertices: Option<Vec<GpuRasterTextureVarying>>,
    pub textures: Vec<std::sync::Arc<crate::scene3d::MaterialImage>>,
    pub environment: Option<std::sync::Arc<crate::scene3d::EnvironmentAsset>>,
    pub shadows: Vec<GpuRasterShadow>,
    /// Identity of the sealed caster/light/alpha inputs used to derive `shadows`.
    shadow_source_key: crate::ContentDigest,
    material_offsets: Vec<u32>,
    /// vec4 records: five scene headers, nine per material, two per directional light,
    /// three per hemisphere, camera ray columns/rows, then four per shadow projection.
    pub values: Vec<[f32; 4]>,
}

#[derive(Clone, Copy, Debug)]
pub struct GpuRasterShadowVertex {
    pub position: [f32; 2],
    pub depth: f32,
    pub uv: [f32; 2],
    pub material: u32,
    pub inv_area: f32,
}

#[derive(Clone, Debug)]
pub struct GpuRasterShadow {
    pub light: u32,
    pub vertices: Vec<GpuRasterShadowVertex>,
    pub values: [[f32; 4]; 4],
}

impl GpuRasterShadow {
    pub const SIZE: u32 = super::shadow::SHADOW_SIZE as u32;
    pub fn vertex_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.vertices.len() * 28);
        for v in &self.vertices {
            for value in v.position.into_iter().chain([v.depth]).chain(v.uv) {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
            bytes.extend_from_slice(&v.material.to_le_bytes());
            bytes.extend_from_slice(&v.inv_area.to_le_bytes());
        }
        bytes
    }
}

impl GpuRasterShading {
    /// Shared with CPU shadows. Prepared packets are immutable rendering inputs; camera,
    /// exposure, roughness and unrelated atlas textures do not invalidate a directional map.
    pub fn shadow_content_key(&self) -> crate::ContentDigest {
        self.shadow_source_key
    }
    pub const LIGHTING_LUT_WIDTH: u32 = 128;
    pub const LIGHTING_LUT_HEIGHT: u32 = 80;
    pub fn lighting_lut() -> &'static [u8; 81_920] {
        super::lighting::LIGHTING_BYTES
    }
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
    /// Perspective packets store inverse depth; orthographic packets store linear depth.
    /// The projection flag in shading selects both depth decoding and attribute weights.
    pub inv_z: f32,
    /// Constant Unlit sRGB, or the original normal XYZ and coverage inverse area for lit packets.
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
    /// Projected packets use depth_scale / z, or 1 - z * depth_scale for orthographic.
    /// Retained source meshes instead
    /// use finite reverse Z from the camera near/far rows for hardware clipping.
    pub depth_scale: f32,
    /// Absent for the exact, constant-color Unlit path.
    pub shading: Option<GpuRasterShading>,
    /// Immutable indexed source geometry. When present, vertices are transformed and
    /// clipped on the GPU; `vertices` is empty and shading carries per-frame parameters.
    pub mesh: Option<std::sync::Arc<GpuMeshData>>,
}

impl GpuRasterGeometry {
    pub fn vertex_count(&self) -> u32 {
        self.mesh
            .as_ref()
            .map_or(self.vertices.len() as u32, |m| m.vertex_count)
    }
    pub const VERTEX_STRIDE: u64 = 36;

    pub fn vertex_stride(&self) -> u64 {
        Self::VERTEX_STRIDE
            + self.shading.as_ref().map_or(0, |s| {
                (if s.texture_vertices.is_some() { 72 } else { 16 })
                    + if s.particle_tints.is_some() { 16 } else { 0 }
            })
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
                if let Some(textures) = &shading.texture_vertices {
                    let v = textures[index];
                    for value in
                        v.uv.into_iter()
                            .chain(v.dx)
                            .chain(v.dy)
                            .chain(v.tangent)
                            .chain(v.bitangent)
                    {
                        bytes.extend_from_slice(&value.to_le_bytes());
                    }
                }
                if let Some(tints) = &shading.particle_tints {
                    for value in tints[index] {
                        bytes.extend_from_slice(&value.to_le_bytes());
                    }
                }
            }
        }
        bytes
    }
}

/// Prepare geometry, current materials and optional lighting, texture and shadow inputs.
/// Invalid scenes/frames and work-budget violations remain errors.
/// This does not allocate color, depth or ID planes or perform CPU pixel rasterization.
pub fn prepare_gpu_geometry(
    prepared: &PreparedScene,
    frame: &Frame3DState,
) -> Result<Option<GpuRasterGeometry>, ContractErrors> {
    prepared.scene.validate()?;
    frame.validate_for(&prepared.scene)?;
    prepared.budget.validate()?;
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
    let lit = camera.orthographic
        || prepared.meshes.iter().any(|m| m.particles.is_some())
        || prepared.scene.pbr.shadows
        || prepared.environment.is_some()
        || prepared.meshes.iter().any(|m| {
            m.materials
                .iter()
                .any(|m| m.values.uses_uv() || m.values.alpha_mode == AlphaMode::Blend)
        })
        || frame
            .camera
            .depth_of_field
            .is_some_and(|dof| dof.max_blur_radius > 0.0)
        || prepared
            .meshes
            .iter()
            .any(|mesh| mesh.materials.iter().any(|m| m.kind != MaterialKind::Unlit));
    let mut shading =
        lit.then(|| GpuRasterShading::new(prepared, frame, &geometry.frame_meshes, &camera));
    for command in &geometry.commands {
        let mesh = &geometry.frame_meshes[command.mesh_index];
        let material = &mesh.materials[command.material_index].values;
        if material
            .alpha_cutoff
            .is_some_and(|cutoff| material.base_color[3] * command.tint[3] < cutoff)
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
        let material_index = shading.as_ref().map_or(0, |s| {
            s.material_id(command.mesh_index, command.material_index)
        });
        let triangle = super::pbr::Triangle::new(
            mesh.materials[command.material_index].kind,
            material,
            command.world,
        );
        let gradients = super::pbr::Gradients::new(command.coverage.vertices);
        for (screen, fixed) in command.coverage.vertices.iter().zip(command.coverage.fixed) {
            vertices.push(GpuRasterVertex {
                position: [
                    fixed.x as f32 / SUBPIXEL_SCALE,
                    fixed.y as f32 / SUBPIXEL_SCALE,
                ],
                inv_z: if camera.orthographic {
                    screen.view_z
                } else {
                    screen.inv_z
                },
                color: if lit {
                    [
                        screen.normal.x,
                        screen.normal.y,
                        screen.normal.z,
                        command.coverage.inv_area,
                    ]
                } else {
                    color
                },
                object_id: mesh.mesh.object_id,
                node_id: command.node_id,
            });
            if let Some(shading) = &mut shading {
                if let Some(tints) = &mut shading.particle_tints {
                    tints.push(command.tint);
                }
                shading.vertices.push(GpuRasterVarying {
                    normal_over_z: [
                        screen.normal.x * screen.inv_z,
                        screen.normal.y * screen.inv_z,
                        screen.normal.z * screen.inv_z,
                    ],
                    material_index,
                });
                if let Some(textures) = &mut shading.texture_vertices {
                    let v = |v: super::V3| [v.x, v.y, v.z];
                    textures.push(GpuRasterTextureVarying {
                        uv: [screen.uv.x, screen.uv.y],
                        dx: gradients.dx,
                        dy: gradients.dy,
                        tangent: if triangle.has_basis {
                            v(triangle.tangent)
                        } else {
                            [0.0; 3]
                        },
                        bitangent: if triangle.has_basis {
                            v(triangle.bitangent)
                        } else {
                            [0.0; 3]
                        },
                    });
                }
            }
        }
    }
    if prepared.scene.pbr.shadows {
        let s = shading.as_mut().expect("shadows require shader data");
        let lights = Lights::new(&prepared.scene, frame, prepared.environment.clone());
        s.shadow_source_key = geometry
            .casters
            .content_key(&lights.directional, &geometry.frame_meshes);
        s.shadows = super::shadow::gpu_maps(
            &lights.directional,
            &geometry.casters,
            &geometry.frame_meshes,
            s,
        );
        s.values[4][2] = s.values.len() as f32;
        s.values[4][3] = s.shadows.len() as f32;
        let light_start = GpuRasterShading::HEADER_LEN
            + s.values[1][0] as usize * GpuRasterShading::MATERIAL_STRIDE;
        for i in 0..lights.directional.len() {
            s.values[light_start + i * 2][3] = -1.0;
        }
        for (layer, shadow) in s.shadows.iter().enumerate() {
            s.values[light_start + shadow.light as usize * 2][3] = layer as f32;
            s.values.extend(shadow.values);
        }
    }
    let mut depth_scale = 1.0;
    while depth_scale > frame.camera.near {
        depth_scale *= 0.5;
    }
    if camera.orthographic {
        depth_scale = 0.5 / camera.far;
    }
    Ok(Some(GpuRasterGeometry {
        width: prepared.width,
        height: prepared.height,
        vertices,
        anchors: anchor_projections(&prepared.scene, &camera, &geometry.transforms),
        depth_scale,
        shading,
        mesh: None,
    }))
}
