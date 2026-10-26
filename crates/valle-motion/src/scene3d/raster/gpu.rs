//! GPU triangle input using the CPU reference's validated transforms, six-plane clipping,
//! culling and 8-bit subpixel grid. Pixel coverage and depth evaluation remain on the GPU.
use super::{
    AlphaMode, AnchorProjection, Camera, ContractErrors, Frame3DState, MAX_TILE_REFERENCES,
    MaterialKind, PreparedScene, SUBPIXEL_SCALE, anchor_projections, collect_geometry,
    f16_bits_to_f32, output_color,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GpuRasterVertex {
    /// Pixel coordinates snapped to the reference's 1/256-pixel coverage grid.
    pub position: [f32; 2],
    /// Interpolate linearly in screen space, then invert to recover camera-space depth.
    pub inv_z: f32,
    /// Opaque premultiplied sRGB, already quantized at the reference F16 boundary.
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
}

impl GpuRasterGeometry {
    pub const VERTEX_STRIDE: u64 = 36;

    pub fn vertex_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.vertices.len() * Self::VERTEX_STRIDE as usize);
        for vertex in &self.vertices {
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
        }
        bytes
    }
}

/// Prepare the initial untextured, unlit opaque/mask subset. Unsupported lighting or material
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
                material.kind != MaterialKind::Unlit
                    || material.values.uses_uv()
                    || material.values.alpha_mode == AlphaMode::Blend
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
    for command in &geometry.commands {
        let mesh = &geometry.frame_meshes[command.mesh_index];
        let material = &mesh.materials[command.material_index].values;
        if material
            .alpha_cutoff
            .is_some_and(|cutoff| material.base_color[3] < cutoff)
        {
            continue;
        }
        let radiance = std::array::from_fn(|i| {
            material.base_color[i] + material.emissive[i] * material.emissive_intensity
        });
        let color = output_color(
            radiance,
            1.0,
            prepared.scene.pbr.tone_mapping,
            frame.exposure,
        )
        .map(f16_bits_to_f32);
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
    }))
}
