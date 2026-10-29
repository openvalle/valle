//! Camera-facing particle geometry joins the same commands/depth/OIT as model geometry.
use super::*;
use crate::particles3d::PreparedParticles3D;
use crate::scene3d::{ParticleShape3D, asset::Affine};

pub(super) fn tinted(
    material: &ModelMaterial,
    tint: [f32; 4],
) -> std::borrow::Cow<'_, ModelMaterial> {
    if tint == [1.0; 4] {
        return std::borrow::Cow::Borrowed(material);
    }
    let mut result = material.clone();
    for (channel, factor) in result.base_color.iter_mut().zip(tint) {
        *channel *= factor;
    }
    for (channel, factor) in result.emissive.iter_mut().zip(tint) {
        *channel *= factor;
    }
    std::borrow::Cow::Owned(result)
}

#[expect(
    clippy::too_many_arguments,
    reason = "Shared geometry buffers and current scene state are explicit"
)]
pub(super) fn collect(
    prepared: &PreparedScene,
    frame: &Frame3DState,
    camera: &Camera,
    mesh_index: usize,
    particles: &PreparedParticles3D,
    transform: MeshTransform,
    commands: &mut Vec<RasterCommand>,
    tile_references: &mut u64,
) -> Result<(), ContractErrors> {
    let spec = prepared.scene.meshes[mesh_index]
        .particles
        .as_ref()
        .expect("prepared emitter");
    let state = &frame.meshes[mesh_index];
    let mut samples = Vec::new();
    particles
        .sample_into(
            state.particle_time.expect("validated particle time"),
            &mut samples,
        )
        .map_err(|e| super::super::asset::one_error("/particles/time", e.to_string()))?;
    let local = Affine::from_transform(state.transform)?;
    let world = transform
        .parent
        .map_or(Ok(local), |parent| parent.compose(local))?;
    let rows = world.gpu_rows();
    let directional_scale = |direction: V3| {
        let d = [direction.x, direction.y, direction.z];
        let local: [f32; 3] = std::array::from_fn(|i| (0..3).map(|j| rows[3 + j][i] * d[j]).sum());
        1.0 / libm::sqrtf(local.iter().map(|v| v * v).sum::<f32>())
    };
    let colors = spec.colors.map(|color| {
        std::array::from_fn::<_, 4, _>(|i| {
            if i == 3 {
                color.0[i]
            } else {
                super::super::asset::srgb_to_linear(color.0[i])
            }
        })
    });
    let normal = camera.forward * -1.0;
    for particle in samples {
        let p = particle.progress;
        let mix = |v: [f32; 2]| v[0] * (1.0 - p) + v[1] * p;
        let size = mix(spec.sizes);
        let mut tint = std::array::from_fn(|i| colors[0][i] * (1.0 - p) + colors[1][i] * p);
        tint[3] *= mix(spec.opacities);
        if size <= 0.0 || tint[3] <= 0.0 {
            continue;
        }
        let center = transform.point(V3::from(Vec3(particle.position)));
        let angle = degrees(mix(spec.rotations));
        let (sin, cos) = (libm::sinf(angle), libm::cosf(angle));
        let right = camera.right * cos + camera.up * sin;
        let up = camera.up * cos - camera.right * sin;
        let right = right * (size * directional_scale(right));
        let up = up * (size * directional_scale(up));
        let vertex = |x: f32, y: f32| WorldVertex {
            position: center + right * x + up * y,
            normal,
            uv: V2 {
                x: x + 0.5,
                y: 0.5 - y,
            },
        };
        let mut emit = |world: [WorldVertex; 3]| {
            project_world_triangle(world, camera, |screen| {
                if let Some(coverage) = Coverage::new(screen, true, prepared.width, prepared.height)
                {
                    *tile_references = tile_references.saturating_add(
                        u64::from(
                            coverage.max_x / TILE_COLUMNS - coverage.min_x / TILE_COLUMNS + 1,
                        ) * u64::from(coverage.max_y / TILE_ROWS - coverage.min_y / TILE_ROWS + 1),
                    );
                    commands.push(RasterCommand {
                        world,
                        coverage,
                        mesh_index,
                        material_index: 0,
                        node_id: 0,
                        tint,
                    });
                }
            });
        };
        match spec.shape {
            ParticleShape3D::Quad => {
                let a = vertex(-0.5, -0.5);
                let b = vertex(0.5, -0.5);
                let c = vertex(0.5, 0.5);
                let d = vertex(-0.5, 0.5);
                emit([a, b, c]);
                emit([a, c, d]);
            }
            ParticleShape3D::Circle => {
                let ring = |i| {
                    let a = std::f32::consts::TAU * i as f32 / 16.0;
                    vertex(libm::cosf(a) * 0.5, libm::sinf(a) * 0.5)
                };
                for i in 0..16 {
                    emit([vertex(0.0, 0.0), ring(i), ring((i + 1) % 16)]);
                }
            }
        }
    }
    Ok(())
}
