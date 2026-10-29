//! Immutable model storage and O(instances + joints + morph weights) frame preparation.
//! Shadow casters, textured interpolation and transparent tiles still use the projected
//! reference packet until their independent CPU consumers have a qualified replacement.
//!
//! Wire layout (u32 words): each draw vertex stores [source offset, instance, material, 0].
//! Unique source vertices follow: [morph count, has normal, 0, 0], position/normal (six
//! floats), four joint indices, four weights, then six floats per morph target. Frame
//! shading appends four camera rows, an instance directory and eighteen rows per instance
//! (node affine/normal, mesh scale/sin/cos/translation, eight morph weights, parent affine/normal), then six rows
//! per joint. `mesh.wgsl` consumes the same layout on native GPUs and generated WebGL.
use super::super::{FrameMesh, MeshTransform, V3};
use super::*;
use crate::scene3d::{MAX_ABS_POSITION, ModelInstance, asset::Affine};
use crate::{
    cpu,
    memory::{self, MemoryBudget, MemoryKind, MemoryLease},
};
use std::sync::{Arc, Mutex};

#[derive(Debug)]
pub struct GpuMeshData {
    pub key: crate::ContentDigest,
    pub vertex_count: u32,
    bytes: Vec<u8>,
    bounds: Vec<Vec<Bounds>>,
    _memory: Option<MemoryLease>,
}

impl GpuMeshData {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn web_extent(&self) -> (u32, u32) {
        let texels = self.bytes.len().div_ceil(16);
        let width = texels.min(4096);
        (width as u32, texels.div_ceil(width) as u32)
    }
    pub fn web_bytes(&self) -> Vec<u8> {
        let (width, height) = self.web_extent();
        let mut bytes = self.bytes.clone();
        bytes.resize(width as usize * height as usize * 16, 0);
        bytes
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Bounds {
    position: [f32; 3],
    morph: [[f32; 3]; 8],
    skin_weight_sum: f32,
}

impl Bounds {
    fn valid(self, instance: &ModelInstance) -> bool {
        // Absolute interval arithmetic is deliberately conservative. If it cannot prove
        // the existing per-vertex admission contract, use the exact CPU validator.
        let mut p = self.position.map(f64::from);
        for (delta, &weight) in self.morph.iter().zip(&instance.weights) {
            for i in 0..3 {
                p[i] += f64::from(delta[i]) * f64::from(weight).abs();
            }
        }
        let bound = |transform: Affine| -> f64 {
            transform.gpu_rows()[..3]
                .iter()
                .map(|row| {
                    (0..3).map(|i| f64::from(row[i]).abs() * p[i]).sum::<f64>()
                        + f64::from(row[3]).abs()
                })
                .fold(0.0, f64::max)
        };
        let extent = if instance.skin.is_some() {
            instance
                .joint_palette
                .iter()
                .copied()
                .map(bound)
                .fold(0.0, f64::max)
                * f64::from(self.skin_weight_sum)
        } else {
            bound(instance.transform)
        };
        // Leave room for f32 rounding in morph accumulation and four-joint blending.
        extent.is_finite() && extent < f64::from(MAX_ABS_POSITION) * (1.0 - 1e-5)
    }
}

#[derive(Debug, Default)]
pub(in crate::scene3d::raster) struct GpuMeshCache(Mutex<CacheState>);
#[derive(Debug, Default)]
struct CacheState {
    budget: Option<MemoryBudget>,
    mesh: Option<Arc<GpuMeshData>>,
}
impl GpuMeshCache {
    pub fn clear(&self) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).mesh = None;
    }
    fn get(&self, prepared: &PreparedScene) -> Option<Arc<GpuMeshData>> {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let budget = cpu::memory_budget();
        let same = match (&budget, &state.budget) {
            (None, None) => true,
            (Some(a), Some(b)) => a.same_budget(b),
            _ => false,
        };
        if !same {
            state.mesh = None;
            state.budget = budget;
        }
        if state.mesh.is_none() {
            state.mesh = build(prepared).map(Arc::new);
        }
        state.mesh.clone()
    }
}

fn build(prepared: &PreparedScene) -> Option<GpuMeshData> {
    let vertex_count = prepared
        .meshes
        .iter()
        .map(|m| {
            m.model
                .instances
                .iter()
                .map(|i| m.model.meshes[i.mesh as usize].triangle_count as usize * 3)
                .sum::<usize>()
        })
        .sum::<usize>();
    let source_words = prepared
        .meshes
        .iter()
        .map(|m| {
            m.model
                .primitives
                .iter()
                .enumerate()
                .map(|(i, p)| p.vertex_count as usize * (18 + m.model.morph_targets[i].len() * 6))
                .sum::<usize>()
        })
        .sum::<usize>();
    let byte_len = vertex_count
        .checked_mul(16)?
        .checked_add(source_words.checked_mul(4)?)?;
    // Retained source data is separately charged, but never exceeds the established
    // geometry allowance. A dense tiny output continues on the reference path.
    if byte_len == 0
        || byte_len as u64 > u64::from(prepared.width) * u64::from(prepared.height) * 12
    {
        return None;
    }
    let metadata = prepared
        .meshes
        .iter()
        .map(|m| m.model.primitives.len() * size_of::<Bounds>())
        .sum::<usize>()
        + prepared.meshes.len() * size_of::<Vec<Bounds>>()
        + size_of::<GpuMeshData>();
    let lease =
        memory::reserve_retained(MemoryKind::Resource, (byte_len + metadata) as u64).ok()?;
    let mut bytes = vec![0u8; byte_len];
    let mut write = |at: usize, words: &[u32]| {
        for (i, word) in words.iter().enumerate() {
            bytes[(at + i) * 4..(at + i + 1) * 4].copy_from_slice(&word.to_le_bytes());
        }
    };
    let mut bounds = Vec::with_capacity(prepared.meshes.len());
    let mut source_at = vertex_count * 4;
    let mut record_at = 0;
    let mut instance_id = 0u32;
    let mut material_base = 0u32;
    for mesh in &prepared.meshes {
        let model = &mesh.model;
        let mut primitive_offsets = Vec::with_capacity(model.primitives.len());
        let mut mesh_bounds = Vec::with_capacity(model.primitives.len());
        for (pi, primitive) in model.primitives.iter().enumerate() {
            primitive_offsets.push(source_at);
            let targets = &model.morph_targets[pi];
            let mut bound = Bounds::default();
            for local in 0..primitive.vertex_count as usize {
                let v = model.vertices[primitive.first_vertex as usize + local];
                let skin = model.skin_vertices[pi].as_ref().map(|s| s[local]);
                write(
                    source_at,
                    &[targets.len() as u32, u32::from(primitive.has_normal), 0, 0],
                );
                write(source_at + 4, &v.position.map(f32::to_bits));
                write(source_at + 7, &v.normal.map(f32::to_bits));
                write(
                    source_at + 10,
                    &skin.map_or([0; 4], |s| s.joints.map(u32::from)),
                );
                write(
                    source_at + 14,
                    &skin.map_or([0.0; 4], |s| s.weights).map(f32::to_bits),
                );
                bound.skin_weight_sum = bound
                    .skin_weight_sum
                    .max(skin.map_or(0.0, |s| s.weights.iter().sum()));
                for (i, p) in v.position.into_iter().enumerate() {
                    bound.position[i] = bound.position[i].max(p.abs());
                }
                for (ti, target) in targets.iter().enumerate() {
                    let p = target.positions.as_ref().map_or([0.0; 3], |p| p[local]);
                    let n = target.normals.as_ref().map_or([0.0; 3], |n| n[local]);
                    write(source_at + 18 + ti * 6, &p.map(f32::to_bits));
                    write(source_at + 21 + ti * 6, &n.map(f32::to_bits));
                    for (i, p) in p.into_iter().enumerate() {
                        bound.morph[ti][i] = bound.morph[ti][i].max(p.abs());
                    }
                }
                source_at += 18 + targets.len() * 6;
            }
            mesh_bounds.push(bound);
        }
        for instance in &model.instances {
            let source = model.meshes[instance.mesh as usize];
            for (pi, &primitive_at) in primitive_offsets
                .iter()
                .enumerate()
                .take((source.first_primitive + source.primitive_count) as usize)
                .skip(source.first_primitive as usize)
            {
                let primitive = &model.primitives[pi];
                let material = material_base
                    + primitive
                        .material_index
                        .unwrap_or(model.materials.len() as u32);
                let stride = 18 + model.morph_targets[pi].len() * 6;
                for &index in &model.indices[primitive.first_index as usize
                    ..(primitive.first_index + primitive.index_count) as usize]
                {
                    let at = primitive_at + (index - primitive.first_vertex) as usize * stride;
                    write(record_at, &[at as u32, instance_id, material, 0]);
                    record_at += 4;
                }
            }
            instance_id += 1;
        }
        material_base += mesh.materials.len() as u32;
        bounds.push(mesh_bounds);
    }
    debug_assert_eq!(source_at * 4, bytes.len());
    debug_assert_eq!(record_at, vertex_count * 4);
    Some(GpuMeshData {
        key: crate::ContentDigest::of_bytes(&bytes),
        vertex_count: vertex_count as u32,
        bytes,
        bounds,
        _memory: lease,
    })
}

/// Try the GPU geometry path without running CPU deformation/projection/clipping.
/// `None` requests the projected reference packet; invalid input remains an error.
pub fn prepare_gpu_mesh_geometry(
    prepared: &PreparedScene,
    frame: &Frame3DState,
) -> Result<Option<GpuRasterGeometry>, ContractErrors> {
    prepared.scene.validate()?;
    frame.validate_for(&prepared.scene)?;
    prepared.budget.validate()?;
    if frame.meshes.iter().any(|m| {
        m.material.transmission.unwrap_or(0.0) > 0.0
            || m.material_overrides
                .iter()
                .any(|m| m.material.transmission.unwrap_or(0.0) > 0.0)
    }) || prepared.scene.pbr.shadows
        || prepared.meshes.iter().any(|m| m.particles.is_some())
        || prepared.meshes.iter().any(|m| {
            m.materials
                .iter()
                .any(|m| m.values.uses_uv() || m.values.needs_blend())
        })
    {
        return Ok(None);
    }
    let camera = Camera::new(frame, prepared.width, prepared.height)?;
    let Some(mesh_data) = prepared.gpu_mesh_cache.get(prepared) else {
        return Ok(None);
    };
    let mut instances = Vec::with_capacity(prepared.meshes.len());
    let mut meshes = Vec::with_capacity(prepared.meshes.len());
    let mut transforms = Vec::with_capacity(prepared.meshes.len());
    for (mi, (mesh, state)) in prepared.meshes.iter().zip(&frame.meshes).enumerate() {
        let clip = prepared.scene.meshes[mi].animation_clip;
        let current = if state.nodes.is_empty()
            && clip.is_none()
            && mesh.model.instances.iter().all(|i| i.skin.is_none())
        {
            std::borrow::Cow::Borrowed(mesh.model.instances.as_slice())
        } else {
            std::borrow::Cow::Owned(mesh.model.frame_instances(
                &state.nodes,
                clip.map(|clip| {
                    (
                        clip,
                        state.animation_time.expect("validated animation time"),
                    )
                }),
            )?)
        };
        for instance in current.iter() {
            let source = mesh.model.meshes[instance.mesh as usize];
            if (source.first_primitive as usize
                ..(source.first_primitive + source.primitive_count) as usize)
                .any(|pi| !mesh_data.bounds[mi][pi].valid(instance))
            {
                return Ok(None);
            }
        }
        let mut materials = mesh.materials.clone();
        for material in &mut materials {
            material.values.apply_values(&state.material);
        }
        for override_ in &state.material_overrides {
            materials[override_.id as usize]
                .values
                .apply_values(&override_.material);
        }
        meshes.push(FrameMesh { mesh, materials });
        transforms.push(MeshTransform::from_state(state)?);
        instances.push(current);
    }
    let mut shading = GpuRasterShading::new(prepared, frame, &meshes, &camera);
    for (mi, mesh) in meshes.iter().enumerate() {
        for (pi, material) in mesh.materials.iter().enumerate() {
            let at = GpuRasterShading::HEADER_LEN
                + shading.material_id(mi, pi) as usize * GpuRasterShading::MATERIAL_STRIDE;
            shading.values[at + 3][2] = f32::from(material.values.double_sided);
        }
    }
    shading.values[4][0] = shading.values.len() as f32;
    shading.values[4][1] = mesh_data.vertex_count as f32;
    let vector = |v: V3, w| [v.x, v.y, v.z, w];
    shading.values.extend([
        vector(camera.right, camera.focal / camera.aspect),
        vector(camera.up, camera.focal),
        vector(camera.forward, camera.near),
        vector(camera.eye, camera.far),
    ]);
    let mut directory = shading.values.len();
    shading.values.resize(
        directory + instances.iter().map(|i| i.len()).sum::<usize>(),
        [0.0; 4],
    );
    for (mi, instances) in instances.iter().enumerate() {
        let transform = transforms[mi];
        for instance in instances.iter() {
            let mirrored =
                (instance.skin.is_none() && instance.transform.mirrored) != transform.mirrored();
            shading.values[directory] = [
                shading.values.len() as f32,
                f32::from(mirrored),
                meshes[mi].mesh.object_id as f32,
                instance.node as f32,
            ];
            directory += 1;
            let node = if instance.skin.is_some() {
                Affine::identity()
            } else {
                instance.transform
            };
            shading.values.extend(node.gpu_rows());
            shading.values.extend([
                vector(transform.scale, f32::from(transform.parent.is_some())),
                vector(transform.rotation_sin, instance.joint_palette.len() as f32),
                vector(transform.rotation_cos, instance.weights.len() as f32),
                vector(transform.translation, 0.0),
            ]);
            let mut weights = [0.0; 8];
            weights[..instance.weights.len()].copy_from_slice(&instance.weights);
            shading
                .values
                .extend(weights.as_chunks::<4>().0.iter().copied());
            shading
                .values
                .extend(transform.parent.unwrap_or_else(Affine::identity).gpu_rows());
            for joint in &instance.joint_palette {
                shading.values.extend(joint.gpu_rows());
            }
        }
    }
    Ok(Some(GpuRasterGeometry {
        width: prepared.width,
        height: prepared.height,
        vertices: Vec::new(),
        anchors: anchor_projections(&prepared.scene, &camera, &transforms),
        depth_scale: camera.near.min(1.0),
        shading: Some(shading),
        mesh: Some(mesh_data),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (PreparedScene, Frame3DState) {
        super::super::super::geometry_reuse_tests::fixture(
            include_bytes!("../../../../tests/fixtures/scene3d/animated-morph-cube.glb"),
            false,
        )
    }
    #[test]
    fn static_mesh_survives_animation_seek_and_material_changes() {
        let (prepared, mut frame) = fixture();
        let first = prepare_gpu_mesh_geometry(&prepared, &frame)
            .unwrap()
            .unwrap();
        let source = first.mesh.as_ref().unwrap();
        for time in [1.5, 3.5, 0.0, 4.2, 0.0] {
            frame.meshes[0].animation_time = Some(time);
            frame.meshes[0].material.roughness = Some(0.2 + time * 0.1);
            frame.meshes[0].transform.rotation_degrees =
                crate::scene3d::Vec3::new(0.0, time * 20.0, 0.0);
            let next = prepare_gpu_mesh_geometry(&prepared, &frame)
                .unwrap()
                .unwrap();
            assert!(next.vertices.is_empty());
            assert!(Arc::ptr_eq(source, next.mesh.as_ref().unwrap()));
            assert_eq!(
                next.anchors,
                super::super::super::project_anchors(
                    &prepared.scene,
                    &frame,
                    prepared.width,
                    prepared.height
                )
                .unwrap()
            );
            let packet = next.web_packet(&frame).unwrap();
            assert_eq!(
                u32::from_le_bytes(packet[20..24].try_into().unwrap()) & 16,
                16
            );
            assert!(
                packet.len()
                    < source.bytes().len() + next.shading.as_ref().unwrap().bytes().len() + 512
            );
        }
    }
    #[test]
    fn mesh_cache_obeys_ledger_identity_and_inflight_lifetime() {
        let (prepared, frame) = fixture();
        let memory = MemoryBudget::new(Some(1024 * 1024));
        let budget = cpu::CpuBudget::new_with_memory(1, Some(memory.clone())).unwrap();
        let packet = budget.run(|| {
            prepare_gpu_mesh_geometry(&prepared, &frame)
                .unwrap()
                .unwrap()
        });
        assert!(memory.report().used_bytes >= packet.mesh.as_ref().unwrap().bytes().len() as u64);
        prepared.trim_shadow_cache();
        assert!(memory.report().used_bytes > 0);
        drop(packet);
        assert_eq!(memory.report().used_bytes, 0);
        budget.run(|| {
            drop(prepare_gpu_mesh_geometry(&prepared, &frame).unwrap());
        });
        let tiny = MemoryBudget::new(Some(1));
        let tiny_budget = cpu::CpuBudget::new_with_memory(1, Some(tiny.clone())).unwrap();
        assert!(
            tiny_budget
                .run(|| prepare_gpu_mesh_geometry(&prepared, &frame).unwrap())
                .is_none()
        );
        assert_eq!(memory.report().used_bytes, 0);
        assert_eq!(tiny.report().used_bytes, 0);
    }
    #[test]
    fn consumers_and_uncertain_deformation_keep_exact_reference_validation() {
        let (mut prepared, mut frame) = fixture();
        prepared.scene.pbr.shadows = true;
        assert!(
            prepare_gpu_mesh_geometry(&prepared, &frame)
                .unwrap()
                .is_none()
        );
        prepared.scene.pbr.shadows = false;
        prepared.meshes[0].materials[0].values.alpha_mode = AlphaMode::Blend;
        assert!(
            prepare_gpu_mesh_geometry(&prepared, &frame)
                .unwrap()
                .is_none()
        );
        frame.camera.near = -1.0;
        assert!(prepare_gpu_mesh_geometry(&prepared, &frame).is_err());
        let mut instance = prepared.meshes[0].model.instances[0].clone();
        instance.weights = vec![1e10];
        let bounds = Bounds {
            position: [1.0; 3],
            morph: [[1.0; 3]; 8],
            skin_weight_sum: 1.0,
        };
        assert!(!bounds.valid(&instance));
        instance.weights = vec![0.0];
        assert!(bounds.valid(&instance));
    }
}
