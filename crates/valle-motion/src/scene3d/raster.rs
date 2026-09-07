use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::ContentDigest;

use super::{
    AdmittedModel, BudgetUsage, Color4, ContractErrors, Frame3DState, LightSpec, MaterialKind,
    MaterialSpec, ModelVertex, Scene3DSpec, Transform3D, Vec3,
};

pub const BACKGROUND_OBJECT_ID: u16 = 0;
pub const CLEAR_DEPTH: u16 = u16::MAX;
const SUBPIXEL_BITS: i32 = 8;
const SUBPIXEL_SCALE: f32 = (1 << SUBPIXEL_BITS) as f32;
const SUBPIXEL_STEP: i64 = 1 << SUBPIXEL_BITS;

#[derive(Clone, Debug, PartialEq)]
pub struct TextureAsset {
    pub(crate) content_digest: ContentDigest,
    pub(crate) width: u32,
    pub(crate) height: u32,
    decoded_digest: ContentDigest,
    /// Row-major premultiplied RGBA8, top row first.
    pub(crate) premul_rgba8: Vec<u8>,
}

impl TextureAsset {
    pub fn new(
        content_digest: ContentDigest,
        width: u32,
        height: u32,
        premul_rgba8: Vec<u8>,
    ) -> Result<Self, ContractErrors> {
        let mut errors = ContractErrors::default();
        let expected = u64::from(width)
            .checked_mul(u64::from(height))
            .and_then(|pixels| pixels.checked_mul(4));
        if width == 0 || height == 0 || expected != Some(premul_rgba8.len() as u64) {
            errors.push(
                "/texture/rgba",
                "texture dimensions must be positive and exactly match RGBA8 bytes",
            );
        }
        for (index, pixel) in premul_rgba8.chunks_exact(4).enumerate() {
            if pixel[0] > pixel[3] || pixel[1] > pixel[3] || pixel[2] > pixel[3] {
                errors.push(
                    format!("/texture/rgba/{index}"),
                    "texture RGB must be premultiplied by alpha",
                );
                break;
            }
        }
        errors.finish()?;
        let mut decoded_hasher = Sha256::new();
        decoded_hasher.update(b"valle-premul-rgba8-v1\0");
        decoded_hasher.update(width.to_le_bytes());
        decoded_hasher.update(height.to_le_bytes());
        decoded_hasher.update(&premul_rgba8);
        Ok(Self {
            content_digest,
            width,
            height,
            decoded_digest: ContentDigest::from_bytes(decoded_hasher.finalize().into()),
            premul_rgba8,
        })
    }

    pub fn pixel_count(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }

    pub fn content_digest(&self) -> ContentDigest {
        self.content_digest
    }

    pub fn size(&self) -> [u32; 2] {
        [self.width, self.height]
    }

    pub fn premul_rgba8(&self) -> &[u8] {
        &self.premul_rgba8
    }

    pub fn decoded_digest(&self) -> ContentDigest {
        self.decoded_digest
    }
}

/// Worker-local identity for an admitted Scene3D prepare result.
///
/// It is intentionally distinct from an external resource [`ContentDigest`], even though both
/// use the same digest bytes. The value remains opaque and is used only as an in-memory cache key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScenePrepareCacheKey(ContentDigest);

#[derive(Clone, Debug, Default)]
pub struct SceneResources {
    pub models: BTreeMap<String, Arc<AdmittedModel>>,
    pub textures: BTreeMap<String, Arc<TextureAsset>>,
}

#[derive(Clone, Debug)]
struct PreparedMesh {
    object_id: u16,
    model: Arc<AdmittedModel>,
    material: MaterialSpec,
    texture: Option<Arc<TextureAsset>>,
    flat_source: Option<[u8; 4]>,
    base_transform: Transform3D,
}

#[derive(Clone, Debug)]
pub struct PreparedScene {
    scene: Scene3DSpec,
    width: u32,
    height: u32,
    meshes: Vec<PreparedMesh>,
    budget: BudgetUsage,
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
        Some(Scene3DPick {
            scene_key: metadata.scene_key.clone(),
            object_id,
            object_key: object.object_key.clone(),
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
            .filter_map(|mesh| mesh.material.texture_control.as_deref())
            .collect::<BTreeSet<_>>()
            .len() as u32,
        anchors: scene.anchors.len() as u32,
        frame_scalars: scene.frame_scalar_count() as u32,
        ..BudgetUsage::default()
    };
    usage.validate()?;
    let camera = Camera::new(scene, frame, width, height)?;
    let transforms = scene
        .meshes
        .iter()
        .zip(&frame.meshes)
        .map(|(mesh, state)| MeshTransform::new(mesh.transform, state))
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
    let mut texture_contents = BTreeSet::new();
    let mut hasher = Sha256::new();
    hasher.update(b"valle-scene3d-prepare-v1\0");
    hasher.update(width.to_le_bytes());
    hasher.update(height.to_le_bytes());
    let scene_bytes = serde_json::to_vec(scene).expect("Scene3DSpec serialization is infallible");
    hasher.update((scene_bytes.len() as u64).to_le_bytes());
    hasher.update(&scene_bytes);
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
        if let Some(control) = &spec.material.texture_control {
            let Some(texture) = resources.textures.get(control) else {
                errors.push(
                    format!("/resources/textures/{control}"),
                    "missing admitted image control binding",
                );
                continue;
            };
            if texture_contents.insert(texture.content_digest) {
                hasher.update(b"texture\0");
                hash_string(&mut hasher, control);
                hasher.update(texture.content_digest.as_bytes());
                hasher.update(texture.decoded_digest.as_bytes());
                hasher.update(texture.width.to_le_bytes());
                hasher.update(texture.height.to_le_bytes());
            }
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
    for (index, spec) in scene.meshes.iter().enumerate() {
        let Some(model) = resources.models.get(&spec.model_control) else {
            errors.push(
                format!("/resources/models/{}", spec.model_control),
                "missing admitted model3d control binding",
            );
            continue;
        };
        if model_contents.insert(model.content_digest) {
            model_bytes = model_bytes.saturating_add(model.source_bytes);
        }
        vertices = vertices.saturating_add(model.vertices.len() as u32);
        triangles = triangles.saturating_add(model.triangle_count());
        let texture = if let Some(control) = &spec.material.texture_control {
            let Some(texture) = resources.textures.get(control) else {
                errors.push(
                    format!("/resources/textures/{control}"),
                    "missing admitted image control binding",
                );
                continue;
            };
            if texture_contents.insert(texture.content_digest) {
                texture_pixels = texture_pixels.saturating_add(texture.pixel_count());
            }
            Some(texture.clone())
        } else {
            None
        };
        meshes.push(PreparedMesh {
            object_id: (index + 1) as u16,
            model: Arc::clone(model),
            material: spec.material.clone(),
            flat_source: texture
                .is_none()
                .then(|| material_pixel(spec.material.color, None, V2 { x: 0.0, y: 0.0 })),
            texture,
            base_transform: spec.transform,
        });
    }
    if meshes.len() != scene.meshes.len() {
        errors.finish()?;
        unreachable!("missing resources always add a diagnostic");
    }
    let budget = BudgetUsage {
        width,
        height,
        model_bytes,
        vertices,
        triangles,
        meshes: scene.meshes.len() as u32,
        materials: scene.meshes.len() as u32,
        textures: texture_contents.len() as u32,
        texture_pixels,
        anchors: scene.anchors.len() as u32,
        frame_scalars: scene.frame_scalar_count() as u32,
    };
    budget.validate()?;
    Ok(PreparedScene {
        scene: scene.clone(),
        width,
        height,
        meshes,
        budget,
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
                && output.object_ids.len() == pixel_count =>
        {
            output.premul_rgba8.fill(0);
            output.depth.fill(CLEAR_DEPTH);
            output.object_ids.fill(BACKGROUND_OBJECT_ID);
            output.anchors.clear();
            output
        }
        _ => RasterFrame {
            width: prepared.width,
            height: prepared.height,
            premul_rgba8: vec![0; pixel_count * 4],
            depth: vec![CLEAR_DEPTH; pixel_count],
            object_ids: vec![BACKGROUND_OBJECT_ID; pixel_count],
            anchors: Vec::with_capacity(prepared.scene.anchors.len()),
        },
    };
    let camera = Camera::new(&prepared.scene, frame, prepared.width, prepared.height)?;
    let lights = Lights::new(&prepared.scene, frame);
    let mut transforms = Vec::with_capacity(prepared.meshes.len());
    for (mesh, state) in prepared.meshes.iter().zip(&frame.meshes) {
        let transform = MeshTransform::new(mesh.base_transform, state);
        transforms.push(transform);
        for triangle in mesh.model.indices.chunks_exact(3) {
            let vertices = [
                transformed_vertex(&mesh.model, triangle[0], transform),
                transformed_vertex(&mesh.model, triangle[1], transform),
                transformed_vertex(&mesh.model, triangle[2], transform),
            ];
            raster_world_triangle(
                vertices,
                mesh,
                &camera,
                &lights,
                prepared.width,
                prepared.height,
                &mut output,
            );
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

impl Camera {
    fn new(
        scene: &Scene3DSpec,
        frame: &Frame3DState,
        width: u32,
        height: u32,
    ) -> Result<Self, ContractErrors> {
        let target = V3::from(scene.camera.target);
        let base_offset = V3::from(scene.camera.position) - target;
        let yaw = degrees(frame.camera.orbit_yaw_degrees);
        let (sin_yaw, cos_yaw) = (libm::sinf(yaw), libm::cosf(yaw));
        let yawed = rotate_y(base_offset.normalized(), sin_yaw, cos_yaw);
        let pitch_axis = V3::Y.cross(yawed).normalized();
        let pitch = degrees(frame.camera.orbit_pitch_degrees);
        let (sin_pitch, cos_pitch) = (libm::sinf(pitch), libm::cosf(pitch));
        let direction = rotate_axis(yawed, pitch_axis, sin_pitch, cos_pitch).normalized();
        let eye = target + direction * frame.camera.distance;
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
            near: scene.camera.near,
            far: scene.camera.far,
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
    fn new(base: Transform3D, frame: &super::MeshFrameState) -> Self {
        let rotation = V3::new(
            base.rotation_degrees.0[0] + frame.rotation_x_degrees,
            base.rotation_degrees.0[1] + frame.rotation_y_degrees,
            base.rotation_degrees.0[2] + frame.rotation_z_degrees,
        );
        let radians = V3::new(
            degrees(rotation.x),
            degrees(rotation.y),
            degrees(rotation.z),
        );
        Self {
            translation: V3::new(
                base.translation.0[0] + frame.translation_x,
                base.translation.0[1] + frame.translation_y,
                base.translation.0[2] + frame.translation_z,
            ),
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
            scale: V3::new(
                base.scale.0[0] * frame.scale_x,
                base.scale.0[1] * frame.scale_y,
                base.scale.0[2] * frame.scale_z,
            ),
        }
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

fn transformed_vertex(model: &AdmittedModel, index: u32, transform: MeshTransform) -> WorldVertex {
    let ModelVertex {
        position,
        normal,
        uv,
    } = model.vertices[index as usize];
    WorldVertex {
        position: transform.point(V3::new(position[0], position[1], position[2])),
        normal: transform.normal(V3::new(normal[0], normal[1], normal[2])),
        uv: V2 { x: uv[0], y: uv[1] },
    }
}

struct Lights {
    ambient: f32,
    directional: Vec<(V3, f32)>,
}

impl Lights {
    fn new(scene: &Scene3DSpec, frame: &Frame3DState) -> Self {
        let mut ambient = 0.0;
        let mut directional = Vec::new();
        for (index, light) in scene.lights.iter().enumerate() {
            let intensity = frame.light_intensities[index];
            match light {
                LightSpec::Ambient { .. } => ambient += intensity,
                LightSpec::Directional { direction, .. } => {
                    directional.push((V3::from(*direction).normalized(), intensity));
                }
            }
        }
        Self {
            ambient,
            directional,
        }
    }

    fn factor(&self, kind: MaterialKind, normal: V3) -> f32 {
        if kind == MaterialKind::Unlit {
            return 1.0;
        }
        let mut factor = self.ambient;
        for (direction, intensity) in &self.directional {
            factor += normal.dot(*direction).max(0.0) * *intensity;
        }
        factor.clamp(0.0, 1.0)
    }
}

fn raster_world_triangle(
    vertices: [WorldVertex; 3],
    mesh: &PreparedMesh,
    camera: &Camera,
    lights: &Lights,
    width: u32,
    height: u32,
    output: &mut RasterFrame,
) {
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
        raster_projected_triangle(screen, mesh, camera, lights, width, height, output);
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
    camera: &Camera,
    lights: &Lights,
    width: u32,
    height: u32,
    output: &mut RasterFrame,
) {
    let signed_area = orient_f32(vertices[0], vertices[1], vertices[2]);
    // glTF front faces are CCW in +Y-up NDC, therefore negative after mapping to screen +Y down.
    if signed_area >= -1.0e-8 {
        return;
    }
    vertices.swap(1, 2);
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
        for _x in min_x..=max_x {
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
            output.depth[offset] = depth;
            output.object_ids[offset] = mesh.object_id;
            let pixel = &mut output.premul_rgba8[offset * 4..offset * 4 + 4];
            if mesh.material.kind == MaterialKind::Unlit {
                if let Some(source) = mesh.flat_source {
                    pixel.copy_from_slice(&source);
                } else {
                    let corrected = perspective_weights(bary, vertices, perspective_sum);
                    let uv = interpolated_uv(vertices, corrected);
                    let source = material_pixel(mesh.material.color, mesh.texture.as_deref(), uv);
                    pixel.copy_from_slice(&source);
                }
            } else {
                let corrected = perspective_weights(bary, vertices, perspective_sum);
                let normal = (vertices[0].normal * corrected[0]
                    + vertices[1].normal * corrected[1]
                    + vertices[2].normal * corrected[2])
                    .normalized();
                let source = match mesh.flat_source {
                    Some(source) => source,
                    None => {
                        let uv = interpolated_uv(vertices, corrected);
                        material_pixel(mesh.material.color, mesh.texture.as_deref(), uv)
                    }
                };
                let light = lights.factor(mesh.material.kind, normal);
                pixel[0] = scale_u8(source[0], light);
                pixel[1] = scale_u8(source[1], light);
                pixel[2] = scale_u8(source[2], light);
                pixel[3] = source[3];
            }
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

fn material_pixel(color: Color4, texture: Option<&TextureAsset>, uv: V2) -> [u8; 4] {
    let base = color.0;
    let alpha_factor = base[3];
    match texture {
        Some(texture) => {
            let u = uv.x.clamp(0.0, 1.0);
            let v = uv.y.clamp(0.0, 1.0);
            let x = libm::roundf(u * texture.width.saturating_sub(1) as f32) as u32;
            let y = libm::roundf(v * texture.height.saturating_sub(1) as f32) as u32;
            let offset = (y as usize * texture.width as usize + x as usize) * 4;
            let texel = &texture.premul_rgba8[offset..offset + 4];
            [
                scale_u8(texel[0], base[0] * alpha_factor),
                scale_u8(texel[1], base[1] * alpha_factor),
                scale_u8(texel[2], base[2] * alpha_factor),
                scale_u8(texel[3], alpha_factor),
            ]
        }
        None => {
            let alpha = quantize_unit(alpha_factor);
            [
                scale_u8(alpha, base[0]),
                scale_u8(alpha, base[1]),
                scale_u8(alpha, base[2]),
                alpha,
            ]
        }
    }
}

fn scale_u8(value: u8, factor: f32) -> u8 {
    libm::roundf(value as f32 * factor.clamp(0.0, 1.0)).clamp(0.0, 255.0) as u8
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
