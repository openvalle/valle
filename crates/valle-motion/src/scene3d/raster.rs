use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::ContentDigest;

use super::{
    AdmittedModel, AlphaMode, BudgetUsage, Color4, ContractErrors, Frame3DState, LightFrameState,
    MaterialImage, MaterialKind, MaterialSpec, ModelMaterial, ModelVertex, Scene3DSpec,
    TextureRole, Transform3D, Vec3,
};
mod lighting;
mod pbr;

pub const BACKGROUND_OBJECT_ID: u16 = 0;
pub const BACKGROUND_NODE_ID: u16 = u16::MAX;
pub const CLEAR_DEPTH: u16 = u16::MAX;
const SUBPIXEL_BITS: i32 = 8;
const SUBPIXEL_SCALE: f32 = (1 << SUBPIXEL_BITS) as f32;
const SUBPIXEL_STEP: i64 = 1 << SUBPIXEL_BITS;

/// Worker-local identity for an admitted Scene3D prepare result.
///
/// It is intentionally distinct from an external resource [`ContentDigest`], even though both
/// use the same digest bytes. The value remains opaque and is used only as an in-memory cache key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScenePrepareCacheKey(ContentDigest);

#[derive(Clone, Debug, Default)]
pub struct SceneResources {
    pub models: BTreeMap<String, Arc<AdmittedModel>>,
    pub textures: BTreeMap<(String, TextureRole), Arc<MaterialImage>>,
    pub environments: BTreeMap<String, Arc<super::EnvironmentAsset>>,
}

#[derive(Clone, Debug)]
struct PreparedMesh {
    object_id: u16,
    model: Arc<AdmittedModel>,
    images: Vec<MaterialImage>,
    /// Original material indices followed by the implicit glTF default material.
    materials: Vec<PreparedMaterial>,
}

#[derive(Clone, Debug)]
struct PreparedMaterial {
    kind: MaterialKind,
    values: ModelMaterial,
}

#[derive(Clone, Debug)]
pub struct PreparedScene {
    scene: Scene3DSpec,
    width: u32,
    height: u32,
    meshes: Vec<PreparedMesh>,
    budget: BudgetUsage,
    environment: Option<Arc<super::EnvironmentAsset>>,
}

impl PreparedScene {
    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn budget(&self) -> BudgetUsage {
        self.budget
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AnchorProjection {
    pub key: String,
    pub object_id: u16,
    pub screen: [f32; 2],
    pub depth: u16,
    pub in_front: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RasterFrame {
    pub width: u32,
    pub height: u32,
    pub premul_rgba8: Vec<u8>,
    pub depth: Vec<u16>,
    pub object_ids: Vec<u16>,
    /// Original glTF node index within the pixel's model. Background has no node.
    pub node_ids: Vec<u16>,
    pub anchors: Vec<AnchorProjection>,
}

/// Small, JSON-safe description of the exact color/depth/id frame returned by the raster core.
/// The large depth and object-id planes stay in explicit u16-LE byte exports; this record freezes
/// only their dimensions, sentinels and stable semantic-address lookup table.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Scene3DFrameMetadata {
    pub scene_key: String,
    pub width: u32,
    pub height: u32,
    pub background_object_id: u16,
    pub background_node_id: u16,
    pub clear_depth: u16,
    pub objects: Vec<Scene3DObjectMetadata>,
    pub anchors: Vec<Scene3DAnchorMetadata>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Scene3DObjectMetadata {
    pub object_id: u16,
    pub object_key: String,
    pub semantic_address: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Scene3DAnchorMetadata {
    pub anchor_key: String,
    pub object_id: u16,
    pub object_key: String,
    pub semantic_address: String,
    pub screen: [f32; 2],
    pub depth: u16,
    pub in_front: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Scene3DPick {
    pub scene_key: String,
    pub object_id: u16,
    pub object_key: String,
    pub node_id: u32,
    pub semantic_address: String,
    pub pixel_x: u32,
    pub pixel_y: u32,
    pub depth: u16,
}

impl RasterFrame {
    /// Construct the only admitted host metadata record for this frame. Object ids are defined by
    /// the frozen mesh order (`0` is background, mesh index + 1 is the object); anchors retain the
    /// same projection/depth values that accompanied the color surface.
    pub fn metadata(
        &self,
        scene_key: &str,
        scene: &Scene3DSpec,
    ) -> Result<Scene3DFrameMetadata, ContractErrors> {
        scene.validate()?;
        let expected_pixels = u64::from(self.width) * u64::from(self.height);
        let mut errors = ContractErrors::default();
        if self.premul_rgba8.len() as u64 != expected_pixels.saturating_mul(4) {
            errors.push(
                "/frame/premulRgba8",
                "color plane length must equal width * height * 4",
            );
        }
        if self.depth.len() as u64 != expected_pixels {
            errors.push(
                "/frame/depth",
                "depth plane length must equal width * height",
            );
        }
        if self.object_ids.len() as u64 != expected_pixels {
            errors.push(
                "/frame/objectIds",
                "object-id plane length must equal width * height",
            );
        }
        if self.node_ids.len() as u64 != expected_pixels {
            errors.push(
                "/frame/nodeIds",
                "node-id plane length must equal width * height",
            );
        }
        if self.anchors.len() != scene.anchors.len() {
            errors.push(
                "/frame/anchors",
                "projected anchor count must equal Scene3D anchor count",
            );
        }
        errors.finish()?;

        let objects = scene
            .meshes
            .iter()
            .enumerate()
            .map(|(index, mesh)| {
                Ok(Scene3DObjectMetadata {
                    object_id: (index + 1) as u16,
                    object_key: mesh.key.clone(),
                    semantic_address: Scene3DSpec::semantic_address(scene_key, &mesh.key)?,
                })
            })
            .collect::<Result<Vec<_>, ContractErrors>>()?;
        let anchors = scene
            .anchors
            .iter()
            .zip(&self.anchors)
            .map(|(spec, projection)| {
                let object_id = scene
                    .meshes
                    .iter()
                    .position(|mesh| mesh.key == spec.parent)
                    .map(|index| (index + 1) as u16)
                    .expect("Scene3D validation proved anchor parent");
                let mut errors = ContractErrors::default();
                if projection.key != spec.key {
                    errors.push(
                        format!("/frame/anchors/{}/key", spec.key),
                        "projected anchor key does not match the frozen anchor table",
                    );
                }
                if projection.object_id != object_id {
                    errors.push(
                        format!("/frame/anchors/{}/objectId", spec.key),
                        "projected anchor object id does not match its parent mesh",
                    );
                }
                errors.finish()?;
                Ok(Scene3DAnchorMetadata {
                    anchor_key: spec.key.clone(),
                    object_id,
                    object_key: spec.parent.clone(),
                    semantic_address: Scene3DSpec::semantic_address(scene_key, &spec.parent)?,
                    screen: projection.screen,
                    depth: projection.depth,
                    in_front: projection.in_front,
                })
            })
            .collect::<Result<Vec<_>, ContractErrors>>()?;
        Ok(Scene3DFrameMetadata {
            scene_key: scene_key.to_owned(),
            width: self.width,
            height: self.height,
            background_object_id: BACKGROUND_OBJECT_ID,
            background_node_id: BACKGROUND_NODE_ID,
            clear_depth: CLEAR_DEPTH,
            objects,
            anchors,
        })
    }

    pub fn depth_u16_le_bytes(&self) -> Vec<u8> {
        u16_le_bytes(&self.depth)
    }

    pub fn object_ids_u16_le_bytes(&self) -> Vec<u8> {
        u16_le_bytes(&self.object_ids)
    }

    pub fn node_ids_u16_le_bytes(&self) -> Vec<u8> {
        u16_le_bytes(&self.node_ids)
    }

    /// Resolve one raster-local pixel through the frozen metadata table. Background and out-of-
    /// range coordinates are misses, never synthetic scene-node hits.
    pub fn pick(
        &self,
        metadata: &Scene3DFrameMetadata,
        pixel_x: u32,
        pixel_y: u32,
    ) -> Option<Scene3DPick> {
        if pixel_x >= self.width
            || pixel_y >= self.height
            || metadata.width != self.width
            || metadata.height != self.height
        {
            return None;
        }
        let offset =
            usize::try_from(u64::from(pixel_y) * u64::from(self.width) + u64::from(pixel_x))
                .ok()?;
        let object_id = *self.object_ids.get(offset)?;
        if object_id == metadata.background_object_id {
            return None;
        }
        let object = metadata
            .objects
            .iter()
            .find(|object| object.object_id == object_id)?;
        let node_id = *self.node_ids.get(offset)?;
        if usize::from(node_id) >= super::MAX_MODEL_NODES {
            return None;
        }
        Some(Scene3DPick {
            scene_key: metadata.scene_key.clone(),
            object_id,
            object_key: object.object_key.clone(),
            node_id: u32::from(node_id),
            semantic_address: object.semantic_address.clone(),
            pixel_x,
            pixel_y,
            depth: *self.depth.get(offset)?,
        })
    }
}

fn u16_le_bytes(values: &[u16]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(values.len().saturating_mul(2));
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

/// Project named anchors without admitting model/texture bytes or rasterizing color/depth.
///
/// Anchor geometry depends only on the frozen Scene3D spec, current frame scalars and the exact
/// generated-texture dimensions. Motion layout uses this narrow function after its 2D box pass so
/// `project3d()` can feed paint geometry without pulling host-owned assets into `valle-motion`.
pub fn project_anchors(
    scene: &Scene3DSpec,
    frame: &Frame3DState,
    width: u32,
    height: u32,
) -> Result<Vec<AnchorProjection>, ContractErrors> {
    scene.validate()?;
    frame.validate_for(scene)?;
    let usage = BudgetUsage {
        width,
        height,
        meshes: scene.meshes.len() as u32,
        materials: scene.meshes.len() as u32,
        textures: scene
            .meshes
            .iter()
            .flat_map(|mesh| mesh.texture_controls())
            .collect::<BTreeSet<_>>()
            .len() as u32,
        anchors: scene.anchors.len() as u32,
        frame_scalars: scene.frame_scalar_count() as u32,
        ..BudgetUsage::default()
    };
    usage.validate()?;
    let camera = Camera::new(frame, width, height)?;
    let transforms = scene
        .meshes
        .iter()
        .zip(&frame.meshes)
        .map(|(_, state)| MeshTransform::new(state.transform))
        .collect::<Vec<_>>();
    Ok(anchor_projections(scene, &camera, &transforms))
}

/// Compute the worker-local pre-prepare key from the validated spec and admitted resource bytes.
/// This does not clone mesh state or calculate the full budget, so a host can answer a steady-state
/// cache hit before entering [`prepare_scene`].
pub fn prepare_cache_key(
    scene: &Scene3DSpec,
    width: u32,
    height: u32,
    resources: &SceneResources,
) -> Result<ScenePrepareCacheKey, ContractErrors> {
    scene.validate()?;
    let mut errors = ContractErrors::default();
    let mut hasher = Sha256::new();
    hasher.update(b"valle-scene3d-prepare\0");
    hasher.update(include_bytes!("raster/assets/dfg.rg16f"));
    hasher.update(width.to_le_bytes());
    hasher.update(height.to_le_bytes());
    let scene_bytes = serde_json::to_vec(scene).expect("Scene3DSpec serialization is infallible");
    hasher.update((scene_bytes.len() as u64).to_le_bytes());
    hasher.update(&scene_bytes);
    if let Some(spec) = &scene.pbr.environment {
        let environment = resources.environments.get(&spec.control).ok_or_else(|| {
            let mut errors = ContractErrors::default();
            errors.push(
                "/resources/environment",
                "missing frozen environment control binding",
            );
            errors
        })?;
        hash_string(&mut hasher, &spec.control);
        hasher.update(environment.content_digest().as_bytes());
    }
    for spec in &scene.meshes {
        let Some(model) = resources.models.get(&spec.model_control) else {
            errors.push(
                format!("/resources/models/{}", spec.model_control),
                "missing admitted model3d control binding",
            );
            continue;
        };
        hasher.update(b"model\0");
        hash_string(&mut hasher, &spec.model_control);
        hasher.update(model.content_digest.as_bytes());
        for (control, role) in spec.texture_controls() {
            let Some(texture) = resources
                .textures
                .get(&(control.to_owned(), role))
                .filter(|t| t.role() == role)
            else {
                errors.push(
                    format!("/resources/textures/{control}"),
                    "missing admitted image interpretation",
                );
                continue;
            };
            hasher.update(b"texture\0");
            hash_string(&mut hasher, control);
            hasher.update([if role == TextureRole::Color { 1 } else { 0 }]);
            hasher.update(texture.content_digest().as_bytes());
        }
    }
    errors.finish()?;
    Ok(ScenePrepareCacheKey(ContentDigest::from_bytes(
        hasher.finalize().into(),
    )))
}

/// Resolve all project controls once and freeze the content-addressed prepare result. Repeated
/// controls are charged per rendered mesh for geometry work, while texture pixels are charged once
/// per distinct bound image. No IO or lazy lookup remains after this function returns.
pub fn prepare_scene(
    scene: &Scene3DSpec,
    width: u32,
    height: u32,
    resources: &SceneResources,
) -> Result<PreparedScene, ContractErrors> {
    scene.validate()?;
    let mut errors = ContractErrors::default();
    let mut meshes = Vec::with_capacity(scene.meshes.len());
    let mut model_bytes = 0u64;
    let mut vertices = 0u32;
    let mut triangles = 0u32;
    let mut model_contents = BTreeSet::new();
    let mut texture_contents = BTreeSet::new();
    let mut texture_pixels = 0u64;
    let mut texture_storage = 0u64;
    let mut stored_images = BTreeSet::new();
    let mut material_count = 0u32;
    for (index, spec) in scene.meshes.iter().enumerate() {
        let Some(model) = resources.models.get(&spec.model_control) else {
            errors.push(
                format!("/resources/models/{}", spec.model_control),
                "missing admitted model3d control binding",
            );
            continue;
        };
        for id in &spec.node_ids {
            if model.nodes.get(*id as usize).is_none_or(|n| !n.active) {
                errors.push(
                    format!("/meshes/{index}/nodeIds"),
                    "node binding must reference an active node in the selected model scene",
                );
            }
        }
        if model_contents.insert(model.content_digest) {
            model_bytes = model_bytes.saturating_add(model.source_bytes);
        }
        vertices = vertices.saturating_add(model.rendered_vertex_count());
        triangles = triangles.saturating_add(model.rendered_triangle_count());
        for image in &model.images {
            if texture_contents.insert(image.storage_key()) {
                texture_pixels = texture_pixels.saturating_add(image.pixel_count());
            }
            // Separate models can own separate mip allocations even for the same encoded image.
            if stored_images.insert((model.content_digest, image.storage_key())) {
                texture_storage = texture_storage.saturating_add(image.storage_bytes());
            }
        }
        let mut images = model.images.clone();
        let mut materials = model
            .materials
            .iter()
            .cloned()
            .chain(std::iter::once(ModelMaterial::default()))
            .map(|values| PreparedMaterial {
                kind: MaterialKind::Pbr,
                values,
            })
            .collect::<Vec<_>>();
        let mut bind =
            |material: &mut PreparedMaterial, spec: &MaterialSpec| -> Result<(), ContractErrors> {
                if let Some(kind) = spec.kind {
                    material.kind = kind;
                }
                if let Some(mode) = spec.alpha_mode {
                    material.values.alpha_cutoff = match mode {
                        AlphaMode::Opaque => None,
                        AlphaMode::Mask => Some(material.values.alpha_cutoff.unwrap_or(0.5)),
                    };
                }
                if let Some(value) = spec.double_sided {
                    material.values.double_sided = value;
                }
                for (&slot, texture) in &spec.textures {
                    let binding = if let Some(texture) = texture {
                        let image = resources
                            .textures
                            .get(&(texture.control.clone(), slot.role()))
                            .filter(|v| v.role() == slot.role())
                            .ok_or_else(|| {
                                let mut e = ContractErrors::default();
                                e.push(
                                    format!("/resources/textures/{}", texture.control),
                                    "missing admitted material image interpretation",
                                );
                                e
                            })?;
                        let image_index = images
                            .iter()
                            .position(|v| v.storage_key() == image.storage_key())
                            .unwrap_or_else(|| {
                                images.push((**image).clone());
                                images.len() - 1
                            });
                        if texture_contents.insert(image.storage_key()) {
                            texture_pixels = texture_pixels.saturating_add(image.pixel_count());
                        }
                        if stored_images.insert((image.content_digest(), image.storage_key())) {
                            texture_storage = texture_storage.saturating_add(image.storage_bytes());
                        }
                        Some(super::asset::ModelTextureBinding::external(
                            image_index,
                            texture,
                        ))
                    } else {
                        None
                    };
                    *material.values.texture_mut(slot) = binding;
                }
                Ok(())
            };
        for material in &mut materials {
            bind(material, &spec.material)?;
        }
        for override_ in &spec.material_overrides {
            if override_.id as usize >= model.materials.len() {
                errors.push(
                    format!("/meshes/{index}/materialOverrides"),
                    "material ID is out of range for the bound model",
                );
                continue;
            }
            bind(&mut materials[override_.id as usize], &override_.material)?;
        }
        let mut used = BTreeSet::new();
        for primitive in &model.primitives {
            let id = primitive
                .material_index
                .map_or(model.materials.len(), |v| v as usize);
            used.insert(id);
            if materials[id].values.uses_uv() && !primitive.has_uv {
                errors.push(
                    format!("/meshes/{index}/materials/{id}"),
                    "material textures require TEXCOORD_0 on the affected primitive",
                );
            }
        }
        material_count = material_count.saturating_add(used.len() as u32);
        meshes.push(PreparedMesh {
            object_id: (index + 1) as u16,
            model: Arc::clone(model),
            images,
            materials,
        });
    }
    errors.clone().finish()?;
    if meshes.len() != scene.meshes.len() {
        errors.finish()?;
        unreachable!("missing resources always add a diagnostic");
    }
    let environment = scene
        .pbr
        .environment
        .as_ref()
        .map(|spec| {
            resources
                .environments
                .get(&spec.control)
                .cloned()
                .ok_or_else(|| {
                    let mut e = ContractErrors::default();
                    e.push(
                        "/resources/environment",
                        "missing frozen environment control binding",
                    );
                    e
                })
        })
        .transpose()?;
    if let Some(environment) = &environment {
        texture_pixels = texture_pixels.saturating_add(environment.pixel_count());
        texture_storage = texture_storage.saturating_add(environment.storage_bytes());
    }
    let budget = BudgetUsage {
        width,
        height,
        model_bytes,
        vertices,
        triangles,
        meshes: scene.meshes.len() as u32,
        materials: material_count,
        textures: texture_contents.len() as u32 + u32::from(environment.is_some()),
        texture_pixels,
        anchors: scene.anchors.len() as u32,
        frame_scalars: scene.frame_scalar_count() as u32,
    };
    budget.validate()?;
    if texture_storage > super::MAX_TEXTURE_STORAGE_BYTES {
        errors.push(
            "/budget/textureStorage",
            "texture mip storage exceeds the fixed byte budget",
        );
        errors.finish()?;
    }
    Ok(PreparedScene {
        scene: scene.clone(),
        width,
        height,
        meshes,
        budget,
        environment,
    })
}

/// Render one complete random-access frame. The only varying input is `Frame3DState`; output does
/// not depend on submission order, a clock, a previous frame or mutable renderer state.
pub fn render_scene(
    prepared: &PreparedScene,
    frame: &Frame3DState,
) -> Result<RasterFrame, ContractErrors> {
    render_scene_reusing(prepared, frame, None)
}

/// Render into a previous frame allocation when its dimensions still match. All planes are fully
/// reset before drawing, so reuse changes allocation behavior only; random access and exact pixels
/// remain identical to [`render_scene`].
pub fn render_scene_reusing(
    prepared: &PreparedScene,
    frame: &Frame3DState,
    reusable: Option<RasterFrame>,
) -> Result<RasterFrame, ContractErrors> {
    prepared.scene.validate()?;
    frame.validate_for(&prepared.scene)?;
    prepared.budget.validate()?;
    let pixel_count = usize::try_from(prepared.budget.layer_pixels().unwrap_or_default())
        .expect("layer budget fits usize on supported targets");
    let mut output = match reusable {
        Some(mut output)
            if output.width == prepared.width
                && output.height == prepared.height
                && output.premul_rgba8.len() == pixel_count * 4
                && output.depth.len() == pixel_count
                && output.object_ids.len() == pixel_count
                && output.node_ids.len() == pixel_count =>
        {
            output.premul_rgba8.fill(0);
            output.depth.fill(CLEAR_DEPTH);
            output.object_ids.fill(BACKGROUND_OBJECT_ID);
            output.node_ids.fill(BACKGROUND_NODE_ID);
            output.anchors.clear();
            output
        }
        _ => RasterFrame {
            width: prepared.width,
            height: prepared.height,
            premul_rgba8: vec![0; pixel_count * 4],
            depth: vec![CLEAR_DEPTH; pixel_count],
            object_ids: vec![BACKGROUND_OBJECT_ID; pixel_count],
            node_ids: vec![BACKGROUND_NODE_ID; pixel_count],
            anchors: Vec::with_capacity(prepared.scene.anchors.len()),
        },
    };
    let camera = Camera::new(frame, prepared.width, prepared.height)?;
    let lights = Lights::new(&prepared.scene, frame, prepared.environment.clone());
    if prepared
        .scene
        .pbr
        .environment
        .as_ref()
        .is_some_and(|e| e.background)
    {
        let environment = prepared
            .environment
            .as_ref()
            .expect("environment admission precedes rendering");
        for y in 0..prepared.height {
            for x in 0..prepared.width {
                let horizontal = ((x as f32 + 0.5) / prepared.width as f32 * 2.0 - 1.0)
                    * camera.aspect
                    / camera.focal;
                let vertical =
                    (1.0 - (y as f32 + 0.5) / prepared.height as f32 * 2.0) / camera.focal;
                let ray = (camera.forward + camera.right * horizontal + camera.up * vertical)
                    .normalized();
                let rgb = environment
                    .specular(lights.environment_direction(ray), 0.0)
                    .map(|v| v * lights.environment_intensity);
                let pixel = output_color(rgb, 1.0, lights.pbr.tone_mapping, lights.exposure);
                let offset = (y as usize * prepared.width as usize + x as usize) * 4;
                output.premul_rgba8[offset..offset + 4].copy_from_slice(&pixel);
            }
        }
    }
    let mut transforms = Vec::with_capacity(prepared.meshes.len());
    for (mesh, state) in prepared.meshes.iter().zip(&frame.meshes) {
        let transform = MeshTransform::new(state.transform);
        transforms.push(transform);
        let current_instances;
        let instances = if state.nodes.is_empty() {
            &mesh.model.instances
        } else {
            current_instances = mesh.model.frame_instances(&state.nodes)?;
            &current_instances
        };
        let mut materials = mesh.materials.clone();
        for material in &mut materials {
            material.values.apply_values(&state.material);
        }
        for override_ in &state.material_overrides {
            materials[override_.id as usize]
                .values
                .apply_values(&override_.material);
        }
        for instance in instances {
            let source = mesh.model.meshes[instance.mesh as usize];
            for primitive in &mesh.model.primitives[source.first_primitive as usize
                ..(source.first_primitive + source.primitive_count) as usize]
            {
                let material = &materials[primitive
                    .material_index
                    .map_or(mesh.model.materials.len(), |i| i as usize)];
                let start = primitive.first_index as usize;
                for triangle in mesh.model.indices[start..start + primitive.index_count as usize]
                    .chunks_exact(3)
                {
                    let mut vertices = [
                        transformed_vertex(&mesh.model, triangle[0], instance.transform, transform),
                        transformed_vertex(&mesh.model, triangle[1], instance.transform, transform),
                        transformed_vertex(&mesh.model, triangle[2], instance.transform, transform),
                    ];
                    if instance.transform.mirrored != transform.mirrored() {
                        vertices.swap(1, 2);
                    }
                    raster_world_triangle(
                        vertices,
                        mesh,
                        material,
                        instance.node as u16,
                        &camera,
                        &lights,
                        prepared.width,
                        prepared.height,
                        &mut output,
                    );
                }
            }
        }
    }
    output.anchors = anchor_projections(&prepared.scene, &camera, &transforms);
    Ok(output)
}

fn anchor_projections(
    scene: &Scene3DSpec,
    camera: &Camera,
    transforms: &[MeshTransform],
) -> Vec<AnchorProjection> {
    let mut output = Vec::with_capacity(scene.anchors.len());
    for anchor in &scene.anchors {
        let mesh_index = scene
            .meshes
            .iter()
            .position(|mesh| mesh.key == anchor.parent)
            .expect("scene validation checked anchor parent");
        let world = transforms[mesh_index].point(V3::from(anchor.position));
        let projected = camera.project_world(world);
        output.push(match projected {
            Some(projected) => AnchorProjection {
                key: anchor.key.clone(),
                object_id: (mesh_index + 1) as u16,
                screen: [projected.x, projected.y],
                depth: quantize_depth(projected.view_z, camera.near, camera.far),
                // Off-canvas anchors still have a valid projection and may drive an entering
                // callout. `false` is reserved for behind-near/beyond-far, where no point exists.
                in_front: true,
            },
            None => AnchorProjection {
                key: anchor.key.clone(),
                object_id: (mesh_index + 1) as u16,
                screen: [-1.0, -1.0],
                depth: CLEAR_DEPTH,
                in_front: false,
            },
        });
    }
    output
}

fn hash_string(hasher: &mut Sha256, value: &str) {
    hasher.update((value.len() as u64).to_le_bytes());
    hasher.update(value.as_bytes());
}

#[derive(Clone, Copy, Debug)]
struct V2 {
    x: f32,
    y: f32,
}

#[derive(Clone, Copy, Debug)]
struct V3 {
    x: f32,
    y: f32,
    z: f32,
}

impl V3 {
    const Y: Self = Self::new(0.0, 1.0, 0.0);

    const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    fn dot(self, rhs: Self) -> f32 {
        self.x * rhs.x + self.y * rhs.y + self.z * rhs.z
    }

    fn cross(self, rhs: Self) -> Self {
        Self::new(
            self.y * rhs.z - self.z * rhs.y,
            self.z * rhs.x - self.x * rhs.z,
            self.x * rhs.y - self.y * rhs.x,
        )
    }

    fn normalized(self) -> Self {
        let length = libm::sqrtf(self.dot(self)).max(1.0e-12);
        self * (1.0 / length)
    }
}

impl From<Vec3> for V3 {
    fn from(value: Vec3) -> Self {
        Self::new(value.0[0], value.0[1], value.0[2])
    }
}

impl core::ops::Add for V3 {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self::new(self.x + rhs.x, self.y + rhs.y, self.z + rhs.z)
    }
}

impl core::ops::Sub for V3 {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self::new(self.x - rhs.x, self.y - rhs.y, self.z - rhs.z)
    }
}

impl core::ops::Mul<f32> for V3 {
    type Output = Self;

    fn mul(self, rhs: f32) -> Self::Output {
        Self::new(self.x * rhs, self.y * rhs, self.z * rhs)
    }
}

#[derive(Clone, Copy)]
struct WorldVertex {
    position: V3,
    normal: V3,
    uv: V2,
}

#[derive(Clone, Copy)]
struct ViewVertex {
    position: V3,
    normal: V3,
    uv: V2,
}

#[derive(Clone, Copy)]
struct ScreenVertex {
    x: f32,
    y: f32,
    view_z: f32,
    inv_z: f32,
    normal: V3,
    uv: V2,
}

struct Camera {
    eye: V3,
    forward: V3,
    right: V3,
    up: V3,
    focal: f32,
    aspect: f32,
    near: f32,
    far: f32,
    width: u32,
    height: u32,
}

impl super::CameraFrameState {
    /// Resolve author orbit conveniences once at the Motion boundary. Raster frames contain the
    /// final camera only, so a request never depends on the order in which earlier frames ran.
    pub fn with_orbit(
        mut self,
        yaw: f32,
        pitch: f32,
        distance: Option<f32>,
    ) -> Result<Self, ContractErrors> {
        self.validate()?;
        let mut errors = ContractErrors::default();
        if !yaw.is_finite()
            || yaw.abs() > super::MAX_ABS_ROTATION_DEGREES
            || !pitch.is_finite()
            || !(-89.0..=89.0).contains(&pitch)
        {
            errors.push(
                "/camera/orbit",
                "camera orbit must be finite; pitch is bounded to -89..=89 degrees",
            );
        }
        if distance
            .is_some_and(|v| !v.is_finite() || !(0.01..=super::MAX_ABS_POSITION).contains(&v))
        {
            errors.push(
                "/camera/distance",
                "camera distance must be finite in 0.01..=10000",
            );
        }
        errors.finish()?;
        if yaw == 0.0 && pitch == 0.0 && distance.is_none() {
            return Ok(self);
        }
        let target = V3::from(self.target);
        let offset = V3::from(self.position) - target;
        let length = distance.unwrap_or_else(|| libm::sqrtf(offset.dot(offset)));
        let yaw = degrees(yaw);
        let pitch = degrees(pitch);
        let yawed = rotate_y(offset.normalized(), libm::sinf(yaw), libm::cosf(yaw));
        let axis = V3::Y.cross(yawed).normalized();
        let direction = rotate_axis(yawed, axis, libm::sinf(pitch), libm::cosf(pitch)).normalized();
        let eye = target + direction * length;
        self.position = Vec3::new(eye.x, eye.y, eye.z);
        self.validate()?;
        Ok(self)
    }
}

impl Camera {
    fn new(frame: &Frame3DState, width: u32, height: u32) -> Result<Self, ContractErrors> {
        let target = V3::from(frame.camera.target);
        let eye = V3::from(frame.camera.position);
        let forward = (target - eye).normalized();
        let right = forward.cross(V3::Y).normalized();
        let up = right.cross(forward).normalized();
        let focal = 1.0 / libm::tanf(degrees(frame.camera.fov_y_degrees) * 0.5);
        if !focal.is_finite() {
            let mut errors = ContractErrors::default();
            errors.push("/camera/fovYDegrees", "camera projection is not finite");
            return Err(errors);
        }
        Ok(Self {
            eye,
            forward,
            right,
            up,
            focal,
            aspect: width as f32 / height as f32,
            near: frame.camera.near,
            far: frame.camera.far,
            width,
            height,
        })
    }

    fn view(&self, world: WorldVertex) -> ViewVertex {
        let relative = world.position - self.eye;
        ViewVertex {
            position: V3::new(
                relative.dot(self.right),
                relative.dot(self.up),
                relative.dot(self.forward),
            ),
            normal: world.normal,
            uv: world.uv,
        }
    }

    fn project(&self, vertex: ViewVertex) -> ScreenVertex {
        let inv_z = 1.0 / vertex.position.z;
        let ndc_x = vertex.position.x * self.focal * inv_z / self.aspect;
        let ndc_y = vertex.position.y * self.focal * inv_z;
        ScreenVertex {
            x: (ndc_x * 0.5 + 0.5) * self.width as f32,
            y: (0.5 - ndc_y * 0.5) * self.height as f32,
            view_z: vertex.position.z,
            inv_z,
            normal: vertex.normal,
            uv: vertex.uv,
        }
    }

    fn project_world(&self, world: V3) -> Option<ScreenVertex> {
        let view = self.view(WorldVertex {
            position: world,
            normal: V3::Y,
            uv: V2 { x: 0.0, y: 0.0 },
        });
        (view.position.z >= self.near && view.position.z <= self.far).then(|| self.project(view))
    }
}

#[derive(Clone, Copy)]
struct MeshTransform {
    translation: V3,
    rotation_sin: V3,
    rotation_cos: V3,
    scale: V3,
}

impl MeshTransform {
    fn new(frame: Transform3D) -> Self {
        let rotation = V3::from(frame.rotation_degrees);
        let radians = V3::new(
            degrees(rotation.x),
            degrees(rotation.y),
            degrees(rotation.z),
        );
        Self {
            translation: V3::from(frame.translation),
            rotation_sin: V3::new(
                libm::sinf(radians.x),
                libm::sinf(radians.y),
                libm::sinf(radians.z),
            ),
            rotation_cos: V3::new(
                libm::cosf(radians.x),
                libm::cosf(radians.y),
                libm::cosf(radians.z),
            ),
            scale: V3::from(frame.scale),
        }
    }

    fn mirrored(self) -> bool {
        self.scale.x * self.scale.y * self.scale.z < 0.0
    }

    fn point(self, point: V3) -> V3 {
        self.rotate(V3::new(
            point.x * self.scale.x,
            point.y * self.scale.y,
            point.z * self.scale.z,
        )) + self.translation
    }

    fn normal(self, normal: V3) -> V3 {
        self.rotate(V3::new(
            normal.x / self.scale.x,
            normal.y / self.scale.y,
            normal.z / self.scale.z,
        ))
        .normalized()
    }

    fn rotate(self, value: V3) -> V3 {
        // Column vectors: Rz * Ry * Rx.
        let x = rotate_x(value, self.rotation_sin.x, self.rotation_cos.x);
        let y = rotate_y(x, self.rotation_sin.y, self.rotation_cos.y);
        rotate_z(y, self.rotation_sin.z, self.rotation_cos.z)
    }
}

impl super::asset::Affine {
    pub(crate) fn from_transform(value: Transform3D) -> Result<Self, ContractErrors> {
        value.validate()?;
        let transform = MeshTransform::new(value);
        let columns = [
            transform.rotate(V3::new(transform.scale.x, 0.0, 0.0)),
            transform.rotate(V3::new(0.0, transform.scale.y, 0.0)),
            transform.rotate(V3::new(0.0, 0.0, transform.scale.z)),
        ];
        Self::new([
            columns[0].x,
            columns[0].y,
            columns[0].z,
            0.0,
            columns[1].x,
            columns[1].y,
            columns[1].z,
            0.0,
            columns[2].x,
            columns[2].y,
            columns[2].z,
            0.0,
            transform.translation.x,
            transform.translation.y,
            transform.translation.z,
            1.0,
        ])
    }
}

fn transformed_vertex(
    model: &AdmittedModel,
    index: u32,
    node: super::asset::Affine,
    transform: MeshTransform,
) -> WorldVertex {
    let ModelVertex {
        position,
        normal,
        uv,
    } = model.vertices[index as usize];
    let position = node.point(position);
    let normal = node.normal(normal);
    WorldVertex {
        position: transform.point(V3::new(position[0], position[1], position[2])),
        normal: transform.normal(V3::new(normal[0], normal[1], normal[2])),
        uv: V2 { x: uv[0], y: uv[1] },
    }
}

struct Lights {
    ambient: [f32; 3],
    directional: Vec<(V3, [f32; 3])>,
    hemisphere: Vec<(V3, [f32; 3], [f32; 3])>,
    environment: Option<Arc<super::EnvironmentAsset>>,
    pbr: super::PbrOptions,
    exposure: f32,
    environment_intensity: f32,
    environment_rotation: [f32; 2],
}

impl Lights {
    fn new(
        scene: &Scene3DSpec,
        frame: &Frame3DState,
        environment: Option<Arc<super::EnvironmentAsset>>,
    ) -> Self {
        let mut ambient = [0.0; 3];
        let mut directional = Vec::new();
        let mut hemisphere = Vec::new();
        let linear = |c: Color4, intensity: f32| {
            std::array::from_fn(|i| super::asset::srgb_to_linear(c.0[i]) * intensity)
        };
        for light in &frame.lights {
            match light {
                LightFrameState::Ambient { color, intensity } => {
                    let c = linear(*color, *intensity);
                    for i in 0..3 {
                        ambient[i] += c[i];
                    }
                }
                LightFrameState::Directional {
                    direction,
                    color,
                    intensity,
                } => directional.push((
                    V3::from(*direction).normalized(),
                    linear(*color, *intensity),
                )),
                LightFrameState::Hemisphere {
                    direction,
                    sky_color,
                    ground_color,
                    intensity,
                } => hemisphere.push((
                    V3::from(*direction).normalized(),
                    linear(*sky_color, *intensity),
                    linear(*ground_color, *intensity),
                )),
            }
        }
        Self {
            ambient,
            directional,
            hemisphere,
            environment,
            pbr: scene.pbr.clone(),
            exposure: frame.exposure,
            environment_intensity: frame.environment_intensity,
            environment_rotation: [
                libm::sinf(degrees(frame.environment_rotation_degrees)),
                libm::cosf(degrees(frame.environment_rotation_degrees)),
            ],
        }
    }

    fn environment_direction(&self, direction: V3) -> [f32; 3] {
        let [s, c] = self.environment_rotation;
        [
            c * direction.x - s * direction.z,
            direction.y,
            s * direction.x + c * direction.z,
        ]
    }

    fn factor(&self, normal: V3, occlusion: f32) -> [f32; 3] {
        let mut factor = self.ambient.map(|v| v * occlusion);
        for (direction, color) in &self.directional {
            let cosine = normal.dot(*direction).max(0.0);
            for i in 0..3 {
                factor[i] += color[i] * cosine;
            }
        }
        for (direction, sky, ground) in &self.hemisphere {
            let t = normal.dot(*direction) * 0.5 + 0.5;
            for i in 0..3 {
                factor[i] += (ground[i] * (1.0 - t) + sky[i] * t) * occlusion;
            }
        }
        if let Some(environment) = &self.environment {
            let diffuse = environment.diffuse(self.environment_direction(normal));
            for i in 0..3 {
                factor[i] += diffuse[i] * self.environment_intensity * occlusion;
            }
        }
        factor
    }
}

fn raster_world_triangle(
    vertices: [WorldVertex; 3],
    mesh: &PreparedMesh,
    material: &PreparedMaterial,
    node_id: u16,
    camera: &Camera,
    lights: &Lights,
    width: u32,
    height: u32,
    output: &mut RasterFrame,
) {
    let shading = pbr::Triangle::new(material.kind, &material.values, vertices);
    let view = [
        camera.view(vertices[0]),
        camera.view(vertices[1]),
        camera.view(vertices[2]),
    ];
    let horizontal = camera.aspect / camera.focal;
    let vertical = 1.0 / camera.focal;
    if view.iter().all(|vertex| {
        vertex.position.z >= camera.near
            && vertex.position.z <= camera.far
            && vertex.position.x + vertex.position.z * horizontal >= 0.0
            && vertex.position.z * horizontal - vertex.position.x >= 0.0
            && vertex.position.y + vertex.position.z * vertical >= 0.0
            && vertex.position.z * vertical - vertex.position.y >= 0.0
    }) {
        raster_projected_triangle(
            view.map(|vertex| camera.project(vertex)),
            mesh,
            &shading,
            node_id,
            camera,
            lights,
            width,
            height,
            output,
        );
        return;
    }
    // Clip all six view-frustum planes before fixed-point setup. Besides correct near-edge
    // triangles, this bounds screen coordinates and therefore makes i64 edge products safe under
    // the public position/scale budget.
    let mut clipped = clip_polygon_by(&view, |vertex| vertex.position.z - camera.near);
    clipped = clip_polygon_by(&clipped, |vertex| camera.far - vertex.position.z);
    clipped = clip_polygon_by(&clipped, |vertex| {
        vertex.position.x + vertex.position.z * horizontal
    });
    clipped = clip_polygon_by(&clipped, |vertex| {
        vertex.position.z * horizontal - vertex.position.x
    });
    clipped = clip_polygon_by(&clipped, |vertex| {
        vertex.position.y + vertex.position.z * vertical
    });
    clipped = clip_polygon_by(&clipped, |vertex| {
        vertex.position.z * vertical - vertex.position.y
    });
    if clipped.len() < 3 {
        return;
    }
    for index in 1..clipped.len() - 1 {
        let screen = [
            camera.project(clipped[0]),
            camera.project(clipped[index]),
            camera.project(clipped[index + 1]),
        ];
        raster_projected_triangle(
            screen, mesh, &shading, node_id, camera, lights, width, height, output,
        );
    }
}

fn clip_polygon_by(
    input: &[ViewVertex],
    signed_distance: impl Fn(ViewVertex) -> f32,
) -> Vec<ViewVertex> {
    let mut output = Vec::with_capacity(input.len() + 1);
    let Some(mut previous) = input.last().copied() else {
        return output;
    };
    let mut previous_distance = signed_distance(previous);
    let mut previous_inside = previous_distance >= 0.0;
    for current in input.iter().copied() {
        let current_distance = signed_distance(current);
        let current_inside = current_distance >= 0.0;
        if current_inside != previous_inside {
            let t = (previous_distance / (previous_distance - current_distance)).clamp(0.0, 1.0);
            output.push(interpolate_view(previous, current, t));
        }
        if current_inside {
            output.push(current);
        }
        previous = current;
        previous_distance = current_distance;
        previous_inside = current_inside;
    }
    output
}

fn interpolate_view(a: ViewVertex, b: ViewVertex, t: f32) -> ViewVertex {
    ViewVertex {
        position: a.position + (b.position - a.position) * t,
        normal: (a.normal + (b.normal - a.normal) * t).normalized(),
        uv: V2 {
            x: a.uv.x + (b.uv.x - a.uv.x) * t,
            y: a.uv.y + (b.uv.y - a.uv.y) * t,
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn raster_projected_triangle(
    mut vertices: [ScreenVertex; 3],
    mesh: &PreparedMesh,
    shading: &pbr::Triangle<'_>,
    node_id: u16,
    camera: &Camera,
    lights: &Lights,
    width: u32,
    height: u32,
    output: &mut RasterFrame,
) {
    let signed_area = orient_f32(vertices[0], vertices[1], vertices[2]);
    // glTF front faces are CCW in +Y-up NDC, therefore negative after mapping to screen +Y down.
    if signed_area.abs() <= 1.0e-8 {
        return;
    }
    if signed_area > 0.0 {
        if !shading.material.double_sided {
            return;
        }
        for vertex in &mut vertices {
            vertex.normal = vertex.normal * -1.0;
        }
    } else {
        vertices.swap(1, 2);
    }
    let gradients = pbr::Gradients::new(vertices);
    let fixed = vertices.map(|vertex| FixedPoint {
        x: libm::roundf(vertex.x * SUBPIXEL_SCALE) as i64,
        y: libm::roundf(vertex.y * SUBPIXEL_SCALE) as i64,
    });
    let area = orient_fixed(fixed[0], fixed[1], fixed[2]);
    if area <= 0 {
        return;
    }
    let min_x = vertices
        .iter()
        .map(|vertex| vertex.x)
        .fold(f32::INFINITY, f32::min)
        .floor()
        .max(0.0) as u32;
    let max_x = vertices
        .iter()
        .map(|vertex| vertex.x)
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil()
        .min(width as f32 - 1.0) as u32;
    let min_y = vertices
        .iter()
        .map(|vertex| vertex.y)
        .fold(f32::INFINITY, f32::min)
        .floor()
        .max(0.0) as u32;
    let max_y = vertices
        .iter()
        .map(|vertex| vertex.y)
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil()
        .min(height as f32 - 1.0) as u32;
    if min_x > max_x || min_y > max_y {
        return;
    }
    let top_left = [
        is_top_left(fixed[1], fixed[2]),
        is_top_left(fixed[2], fixed[0]),
        is_top_left(fixed[0], fixed[1]),
    ];
    let inv_area = 1.0 / area as f32;
    let first_sample = FixedPoint {
        x: (i64::from(min_x) << SUBPIXEL_BITS) + (1 << (SUBPIXEL_BITS - 1)),
        y: (i64::from(min_y) << SUBPIXEL_BITS) + (1 << (SUBPIXEL_BITS - 1)),
    };
    let mut row_edge = [
        orient_fixed(fixed[1], fixed[2], first_sample),
        orient_fixed(fixed[2], fixed[0], first_sample),
        orient_fixed(fixed[0], fixed[1], first_sample),
    ];
    let edge_step_x = [
        -(fixed[2].y - fixed[1].y) * SUBPIXEL_STEP,
        -(fixed[0].y - fixed[2].y) * SUBPIXEL_STEP,
        -(fixed[1].y - fixed[0].y) * SUBPIXEL_STEP,
    ];
    let edge_step_y = [
        (fixed[2].x - fixed[1].x) * SUBPIXEL_STEP,
        (fixed[0].x - fixed[2].x) * SUBPIXEL_STEP,
        (fixed[1].x - fixed[0].x) * SUBPIXEL_STEP,
    ];
    for y in min_y..=max_y {
        let mut edge = row_edge;
        let mut offset = y as usize * width as usize + min_x as usize;
        for x in min_x..=max_x {
            let current_edge = edge;
            edge[0] += edge_step_x[0];
            edge[1] += edge_step_x[1];
            edge[2] += edge_step_x[2];
            if current_edge
                .iter()
                .zip(top_left)
                .any(|(value, owns_edge)| *value < 0 || (*value == 0 && !owns_edge))
            {
                offset += 1;
                continue;
            }
            let bary = [
                current_edge[0] as f32 * inv_area,
                current_edge[1] as f32 * inv_area,
                current_edge[2] as f32 * inv_area,
            ];
            let perspective_sum = bary[0] * vertices[0].inv_z
                + bary[1] * vertices[1].inv_z
                + bary[2] * vertices[2].inv_z;
            if perspective_sum <= 0.0 {
                offset += 1;
                continue;
            }
            let view_z = 1.0 / perspective_sum;
            let depth = quantize_depth(view_z, camera.near, camera.far);
            let previous = output.depth[offset];
            if depth > previous
                || (depth == previous
                    && output.object_ids[offset] != 0
                    && mesh.object_id >= output.object_ids[offset])
            {
                offset += 1;
                continue;
            }
            let pixel = &mut output.premul_rgba8[offset * 4..offset * 4 + 4];
            let corrected = perspective_weights(bary, vertices, perspective_sum);
            let uv = interpolated_uv(vertices, corrected);
            let normal = (vertices[0].normal * corrected[0]
                + vertices[1].normal * corrected[1]
                + vertices[2].normal * corrected[2])
                .normalized();
            let view = (camera.forward * -1.0
                + camera.right
                    * (-(2.0 * (x as f32 + 0.5) / width as f32 - 1.0) * camera.aspect
                        / camera.focal)
                + camera.up * (-(1.0 - 2.0 * (y as f32 + 0.5) / height as f32) / camera.focal))
                .normalized();
            let Some(color) = pbr::shade(
                mesh,
                shading,
                normal,
                view,
                uv,
                gradients.at(uv, perspective_sum),
                lights,
            ) else {
                offset += 1;
                continue;
            };
            pixel.copy_from_slice(&color);
            // Masked fragments never write any visibility plane, so objects behind holes can
            // still draw and be picked regardless of submission order.
            output.depth[offset] = depth;
            output.object_ids[offset] = mesh.object_id;
            output.node_ids[offset] = node_id;
            offset += 1;
        }
        row_edge[0] += edge_step_y[0];
        row_edge[1] += edge_step_y[1];
        row_edge[2] += edge_step_y[2];
    }
}

fn perspective_weights(
    bary: [f32; 3],
    vertices: [ScreenVertex; 3],
    perspective_sum: f32,
) -> [f32; 3] {
    [
        bary[0] * vertices[0].inv_z / perspective_sum,
        bary[1] * vertices[1].inv_z / perspective_sum,
        bary[2] * vertices[2].inv_z / perspective_sum,
    ]
}

fn interpolated_uv(vertices: [ScreenVertex; 3], corrected: [f32; 3]) -> V2 {
    V2 {
        x: vertices[0].uv.x * corrected[0]
            + vertices[1].uv.x * corrected[1]
            + vertices[2].uv.x * corrected[2],
        y: vertices[0].uv.y * corrected[0]
            + vertices[1].uv.y * corrected[1]
            + vertices[2].uv.y * corrected[2],
    }
}

fn output_color(
    radiance: [f32; 3],
    alpha: f32,
    tone: super::ToneMapping,
    exposure: f32,
) -> [u8; 4] {
    let color = match tone {
        super::ToneMapping::Aces => lighting::aces(radiance, exposure),
        super::ToneMapping::None => radiance.map(|v| v * exposure),
    };
    let rgb = color.map(|v| quantize_unit(super::asset::linear_to_srgb(v).clamp(0.0, 1.0) * alpha));
    [rgb[0], rgb[1], rgb[2], quantize_unit(alpha)]
}

fn quantize_unit(value: f32) -> u8 {
    libm::roundf(value.clamp(0.0, 1.0) * 255.0) as u8
}

fn quantize_depth(view_z: f32, near: f32, far: f32) -> u16 {
    let normalized = ((view_z - near) / (far - near)).clamp(0.0, 1.0);
    libm::roundf(normalized * (u16::MAX - 1) as f32) as u16
}

#[derive(Clone, Copy)]
struct FixedPoint {
    x: i64,
    y: i64,
}

fn orient_fixed(a: FixedPoint, b: FixedPoint, point: FixedPoint) -> i64 {
    (b.x - a.x) * (point.y - a.y) - (b.y - a.y) * (point.x - a.x)
}

fn orient_f32(a: ScreenVertex, b: ScreenVertex, point: ScreenVertex) -> f32 {
    (b.x - a.x) * (point.y - a.y) - (b.y - a.y) * (point.x - a.x)
}

fn is_top_left(a: FixedPoint, b: FixedPoint) -> bool {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    dy > 0 || (dy == 0 && dx < 0)
}

fn degrees(value: f32) -> f32 {
    value * (core::f32::consts::PI / 180.0)
}

fn rotate_x(value: V3, sin: f32, cos: f32) -> V3 {
    V3::new(
        value.x,
        value.y * cos - value.z * sin,
        value.y * sin + value.z * cos,
    )
}

fn rotate_y(value: V3, sin: f32, cos: f32) -> V3 {
    V3::new(
        value.x * cos + value.z * sin,
        value.y,
        -value.x * sin + value.z * cos,
    )
}

fn rotate_z(value: V3, sin: f32, cos: f32) -> V3 {
    V3::new(
        value.x * cos - value.y * sin,
        value.x * sin + value.y * cos,
        value.z,
    )
}

fn rotate_axis(value: V3, axis: V3, sin: f32, cos: f32) -> V3 {
    value * cos + axis.cross(value) * sin + axis * (axis.dot(value) * (1.0 - cos))
}
