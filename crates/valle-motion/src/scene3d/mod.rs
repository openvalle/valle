//! Shared contracts for controlled 3D texture layers: scene types, frame scalars, budgets, GLB
//! admission, preparation keys, and deterministic software rasterization. Native and Wasm consumers
//! use the same validation without a separate mutable scene runtime.

mod asset;
mod raster;

pub use asset::{AdmittedModel, ModelVertex, PrimitiveRange, admit_glb};
pub use raster::{
    AnchorProjection, BACKGROUND_OBJECT_ID, CLEAR_DEPTH, PreparedScene, RasterFrame,
    Scene3DAnchorMetadata, Scene3DFrameMetadata, Scene3DObjectMetadata, Scene3DPick,
    ScenePrepareCacheKey, SceneResources, TextureAsset, prepare_cache_key, prepare_scene,
    project_anchors, render_scene, render_scene_reusing,
};

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};

pub const MAX_LAYER_EDGE: u32 = 2_048;
pub const MAX_LAYER_PIXELS: u64 = 2_000_000;
pub const MAX_FRAME_BUFFER_BYTES: u64 = MAX_LAYER_PIXELS * 8; // premul RGBA8 + u16 depth + u16 id
pub const MAX_MODEL_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_VERTICES: u32 = 65_535;
pub const MAX_TRIANGLES: u32 = 20_000;
pub const MAX_MESHES: usize = 8;
pub const MAX_MATERIALS: usize = MAX_MESHES;
pub const MAX_TEXTURES: u32 = 4;
pub const MAX_TEXTURE_PIXELS: u64 = 4_194_304;
pub const MAX_ANCHORS: usize = 32;
pub const MAX_DIRECTIONAL_LIGHTS: usize = 2;
pub const MAX_FRAME_SCALARS: usize = 4 + MAX_MESHES * 9 + 1 + MAX_DIRECTIONAL_LIGHTS;
pub const MAX_ABS_POSITION: f32 = 10_000.0;
pub const MAX_SCALE: f32 = 1_000.0;
pub const MAX_ABS_ROTATION_DEGREES: f32 = 1_000_000.0;
pub const MAX_CAMERA_FAR: f32 = 100_000.0;

/// glTF right-handed coordinates, +Y up. Author Euler values are degrees. With column vectors,
/// points are transformed by `T * Rz * Ry * Rx * S` (scale, then local X/Y/Z rotation, then
/// translation). Camera view maps `position → target` to view -Z; screen +X is right and +Y down.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Scene3DSpec {
    pub camera: CameraSpec,
    pub meshes: Vec<MeshSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lights: Vec<LightSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub anchors: Vec<AnchorSpec>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CameraSpec {
    pub position: Vec3,
    pub target: Vec3,
    #[serde(default = "default_fov")]
    pub fov_y_degrees: f32,
    #[serde(default = "default_near")]
    pub near: f32,
    #[serde(default = "default_far")]
    pub far: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MeshSpec {
    pub key: String,
    /// Logical project control name. 10.3c resolves it to an admitted content hash before raster.
    pub model_control: String,
    pub material: MaterialSpec,
    #[serde(default)]
    pub transform: Transform3D,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MaterialSpec {
    pub kind: MaterialKind,
    pub color: Color4,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub texture_control: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum MaterialKind {
    Unlit,
    Lambert,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum LightSpec {
    Ambient { intensity: f32 },
    Directional { direction: Vec3, intensity: f32 },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AnchorSpec {
    pub key: String,
    pub parent: String,
    pub position: Vec3,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Transform3D {
    pub translation: Vec3,
    pub rotation_degrees: Vec3,
    pub scale: Vec3,
}

impl Default for Transform3D {
    fn default() -> Self {
        Self {
            translation: Vec3::ZERO,
            rotation_degrees: Vec3::ZERO,
            scale: Vec3::ONE,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(transparent)]
pub struct Vec3(pub [f32; 3]);

impl Vec3 {
    pub const ZERO: Self = Self([0.0, 0.0, 0.0]);
    pub const ONE: Self = Self([1.0, 1.0, 1.0]);

    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self([x, y, z])
    }

    fn finite(self) -> bool {
        self.0.into_iter().all(f32::is_finite)
    }

    fn length_squared(self) -> f32 {
        self.0.into_iter().map(|value| value * value).sum()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(transparent)]
pub struct Color4(pub [f32; 4]);

impl Color4 {
    fn valid(self) -> bool {
        self.0
            .into_iter()
            .all(|value| value.is_finite() && (0.0..=1.0).contains(&value))
    }
}

/// Only these scalar slots may vary per frame. Vec3/quaternion/matrix are deliberately absent.
/// Motion evaluates its NumberValue/Expr graph first, converts finite f64 to f32 once, then sends
/// a complete `Frame3DState` to Native or Wasm raster; the core never reads a clock or prior frame.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Frame3DState {
    pub camera: CameraFrameState,
    pub meshes: Vec<MeshFrameState>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub light_intensities: Vec<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CameraFrameState {
    pub orbit_yaw_degrees: f32,
    pub orbit_pitch_degrees: f32,
    pub distance: f32,
    pub fov_y_degrees: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MeshFrameState {
    pub key: String,
    pub translation_x: f32,
    pub translation_y: f32,
    pub translation_z: f32,
    pub rotation_x_degrees: f32,
    pub rotation_y_degrees: f32,
    pub rotation_z_degrees: f32,
    pub scale_x: f32,
    pub scale_y: f32,
    pub scale_z: f32,
}

/// One unit shared by asset admission, Artifact validation and every frame execution entry.
/// Counts are aggregate for one Scene3D layer; callers may not reinterpret them per mesh/node.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BudgetUsage {
    pub width: u32,
    pub height: u32,
    pub model_bytes: u64,
    pub vertices: u32,
    pub triangles: u32,
    pub meshes: u32,
    pub materials: u32,
    pub textures: u32,
    pub texture_pixels: u64,
    pub anchors: u32,
    pub frame_scalars: u32,
}

impl BudgetUsage {
    pub fn layer_pixels(self) -> Option<u64> {
        u64::from(self.width).checked_mul(u64::from(self.height))
    }

    pub fn frame_buffer_bytes(self) -> Option<u64> {
        self.layer_pixels()?.checked_mul(8)
    }

    pub fn validate(self) -> Result<(), ContractErrors> {
        let mut errors = ContractErrors::default();
        if self.width == 0 || self.height == 0 {
            errors.push("/budget/layer", "width and height must both be positive");
        }
        if self.width > MAX_LAYER_EDGE || self.height > MAX_LAYER_EDGE {
            errors.push(
                "/budget/layer",
                format!("each layer edge must be <= {MAX_LAYER_EDGE}"),
            );
        }
        check_max(
            &mut errors,
            "/budget/layerPixels",
            self.layer_pixels(),
            MAX_LAYER_PIXELS,
        );
        check_max(
            &mut errors,
            "/budget/frameBufferBytes",
            self.frame_buffer_bytes(),
            MAX_FRAME_BUFFER_BYTES,
        );
        check_max(
            &mut errors,
            "/budget/modelBytes",
            Some(self.model_bytes),
            MAX_MODEL_BYTES,
        );
        check_max(
            &mut errors,
            "/budget/vertices",
            Some(u64::from(self.vertices)),
            u64::from(MAX_VERTICES),
        );
        check_max(
            &mut errors,
            "/budget/triangles",
            Some(u64::from(self.triangles)),
            u64::from(MAX_TRIANGLES),
        );
        check_max(
            &mut errors,
            "/budget/meshes",
            Some(u64::from(self.meshes)),
            MAX_MESHES as u64,
        );
        check_max(
            &mut errors,
            "/budget/materials",
            Some(u64::from(self.materials)),
            MAX_MATERIALS as u64,
        );
        check_max(
            &mut errors,
            "/budget/textures",
            Some(u64::from(self.textures)),
            u64::from(MAX_TEXTURES),
        );
        check_max(
            &mut errors,
            "/budget/texturePixels",
            Some(self.texture_pixels),
            MAX_TEXTURE_PIXELS,
        );
        check_max(
            &mut errors,
            "/budget/anchors",
            Some(u64::from(self.anchors)),
            MAX_ANCHORS as u64,
        );
        check_max(
            &mut errors,
            "/budget/frameScalars",
            Some(u64::from(self.frame_scalars)),
            MAX_FRAME_SCALARS as u64,
        );
        errors.finish()
    }
}

impl Scene3DSpec {
    pub fn validate(&self) -> Result<(), ContractErrors> {
        let mut errors = ContractErrors::default();
        validate_camera(&self.camera, &mut errors);
        if self.meshes.is_empty() || self.meshes.len() > MAX_MESHES {
            errors.push(
                "/meshes",
                format!("Scene3D requires 1..={MAX_MESHES} meshes"),
            );
        }
        if self.anchors.len() > MAX_ANCHORS {
            errors.push(
                "/anchors",
                format!("Scene3D allows at most {MAX_ANCHORS} anchors"),
            );
        }
        let mut mesh_keys = BTreeSet::new();
        for (index, mesh) in self.meshes.iter().enumerate() {
            let path = format!("/meshes/{index}");
            validate_key(&mesh.key, &format!("{path}/key"), &mut errors);
            if !mesh_keys.insert(mesh.key.as_str()) {
                errors.push(format!("{path}/key"), "mesh keys must be unique");
            }
            validate_control(
                &mesh.model_control,
                &format!("{path}/modelControl"),
                &mut errors,
            );
            validate_transform(mesh.transform, &format!("{path}/transform"), &mut errors);
            if !mesh.material.color.valid() {
                errors.push(
                    format!("{path}/material/color"),
                    "material color channels must be finite in 0..=1",
                );
            }
            if let Some(texture) = &mesh.material.texture_control {
                validate_control(
                    texture,
                    &format!("{path}/material/textureControl"),
                    &mut errors,
                );
            }
        }
        let mut ambient = 0;
        let mut directional = 0;
        for (index, light) in self.lights.iter().enumerate() {
            match light {
                LightSpec::Ambient { intensity } => {
                    ambient += 1;
                    validate_intensity(
                        *intensity,
                        &format!("/lights/{index}/intensity"),
                        &mut errors,
                    );
                }
                LightSpec::Directional {
                    direction,
                    intensity,
                } => {
                    directional += 1;
                    let length_squared = direction.length_squared();
                    if !direction.finite()
                        || !length_squared.is_finite()
                        || length_squared <= 1.0e-12
                        || direction
                            .0
                            .into_iter()
                            .any(|component| component.abs() > MAX_ABS_POSITION)
                    {
                        errors.push(
                            format!("/lights/{index}/direction"),
                            format!(
                                "direction must be a finite non-zero surface-to-light vector with components <= {MAX_ABS_POSITION}"
                            ),
                        );
                    }
                    validate_intensity(
                        *intensity,
                        &format!("/lights/{index}/intensity"),
                        &mut errors,
                    );
                }
            }
        }
        if ambient > 1 {
            errors.push("/lights", "Scene3D allows at most one ambient light");
        }
        if directional > MAX_DIRECTIONAL_LIGHTS {
            errors.push(
                "/lights",
                format!("Scene3D allows at most {MAX_DIRECTIONAL_LIGHTS} directional lights"),
            );
        }
        let mut anchor_keys = BTreeSet::new();
        for (index, anchor) in self.anchors.iter().enumerate() {
            let path = format!("/anchors/{index}");
            validate_key(&anchor.key, &format!("{path}/key"), &mut errors);
            if !anchor_keys.insert(anchor.key.as_str()) {
                errors.push(format!("{path}/key"), "anchor keys must be unique");
            }
            if !mesh_keys.contains(anchor.parent.as_str()) {
                errors.push(
                    format!("{path}/parent"),
                    "anchor parent must name a mesh in the same Scene3D",
                );
            }
            if !anchor.position.finite() {
                errors.push(format!("{path}/position"), "anchor position must be finite");
            } else if anchor
                .position
                .0
                .into_iter()
                .any(|component| component.abs() > MAX_ABS_POSITION)
            {
                errors.push(
                    format!("{path}/position"),
                    format!("anchor coordinates must have magnitude <= {MAX_ABS_POSITION}"),
                );
            }
        }
        errors.finish()
    }

    pub fn semantic_address(scene_key: &str, object_key: &str) -> Result<String, ContractErrors> {
        let mut errors = ContractErrors::default();
        validate_key(scene_key, "/sceneKey", &mut errors);
        validate_key(object_key, "/objectKey", &mut errors);
        errors.finish()?;
        Ok(format!("{scene_key}::{object_key}"))
    }

    pub fn frame_scalar_count(&self) -> usize {
        4 + self.meshes.len() * 9 + self.lights.len()
    }
}

impl Frame3DState {
    pub fn validate_for(&self, scene: &Scene3DSpec) -> Result<(), ContractErrors> {
        let mut errors = ContractErrors::default();
        for (path, value) in [
            ("/camera/orbitYawDegrees", self.camera.orbit_yaw_degrees),
            ("/camera/orbitPitchDegrees", self.camera.orbit_pitch_degrees),
            ("/camera/distance", self.camera.distance),
            ("/camera/fovYDegrees", self.camera.fov_y_degrees),
        ] {
            if !value.is_finite() {
                errors.push(path, "frame scalar must be finite");
            }
        }
        if !(-89.0..=89.0).contains(&self.camera.orbit_pitch_degrees) {
            errors.push(
                "/camera/orbitPitchDegrees",
                "orbit pitch must be in -89..=89 degrees",
            );
        }
        if self.camera.orbit_yaw_degrees.abs() > MAX_ABS_ROTATION_DEGREES {
            errors.push(
                "/camera/orbitYawDegrees",
                format!("orbit yaw magnitude must be <= {MAX_ABS_ROTATION_DEGREES}"),
            );
        }
        if !(0.01..=MAX_ABS_POSITION).contains(&self.camera.distance) {
            errors.push(
                "/camera/distance",
                format!("camera distance must be in 0.01..={MAX_ABS_POSITION}"),
            );
        }
        if !(10.0..=120.0).contains(&self.camera.fov_y_degrees) {
            errors.push(
                "/camera/fovYDegrees",
                "vertical FOV must be in 10..=120 degrees",
            );
        }
        let expected = scene
            .meshes
            .iter()
            .map(|mesh| mesh.key.as_str())
            .collect::<Vec<_>>();
        let actual = self
            .meshes
            .iter()
            .map(|mesh| mesh.key.as_str())
            .collect::<Vec<_>>();
        if actual != expected {
            errors.push(
                "/meshes",
                "frame mesh states must match Scene3D mesh keys in exact static order",
            );
        }
        for (index, mesh) in self.meshes.iter().enumerate() {
            for (name, value) in mesh.scalars() {
                if !value.is_finite() {
                    errors.push(
                        format!("/meshes/{index}/{name}"),
                        "frame scalar must be finite",
                    );
                }
            }
            if mesh.scale_x <= 0.0 || mesh.scale_y <= 0.0 || mesh.scale_z <= 0.0 {
                errors.push(
                    format!("/meshes/{index}/scale"),
                    "frame scale axes must all be > 0",
                );
            }
            for (name, value) in [
                ("translationX", mesh.translation_x),
                ("translationY", mesh.translation_y),
                ("translationZ", mesh.translation_z),
            ] {
                if value.abs() > MAX_ABS_POSITION {
                    errors.push(
                        format!("/meshes/{index}/{name}"),
                        format!("translation magnitude must be <= {MAX_ABS_POSITION}"),
                    );
                }
            }
            for (name, value) in [
                ("rotationXDegrees", mesh.rotation_x_degrees),
                ("rotationYDegrees", mesh.rotation_y_degrees),
                ("rotationZDegrees", mesh.rotation_z_degrees),
            ] {
                if value.abs() > MAX_ABS_ROTATION_DEGREES {
                    errors.push(
                        format!("/meshes/{index}/{name}"),
                        format!("rotation magnitude must be <= {MAX_ABS_ROTATION_DEGREES}"),
                    );
                }
            }
            if mesh.scale_x > MAX_SCALE || mesh.scale_y > MAX_SCALE || mesh.scale_z > MAX_SCALE {
                errors.push(
                    format!("/meshes/{index}/scale"),
                    format!("frame scale axes must be <= {MAX_SCALE}"),
                );
            }
        }
        if self.light_intensities.len() != scene.lights.len() {
            errors.push(
                "/lightIntensities",
                "frame light intensities must match all Scene3D lights in static order",
            );
        }
        for (index, intensity) in self.light_intensities.iter().enumerate() {
            validate_intensity(
                *intensity,
                &format!("/lightIntensities/{index}"),
                &mut errors,
            );
        }
        errors.finish()
    }
}

impl MeshFrameState {
    fn scalars(&self) -> BTreeMap<&'static str, f32> {
        BTreeMap::from([
            ("translationX", self.translation_x),
            ("translationY", self.translation_y),
            ("translationZ", self.translation_z),
            ("rotationXDegrees", self.rotation_x_degrees),
            ("rotationYDegrees", self.rotation_y_degrees),
            ("rotationZDegrees", self.rotation_z_degrees),
            ("scaleX", self.scale_x),
            ("scaleY", self.scale_y),
            ("scaleZ", self.scale_z),
        ])
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContractError {
    pub path: String,
    pub message: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContractErrors(pub Vec<ContractError>);

impl ContractErrors {
    fn push(&mut self, path: impl Into<String>, message: impl Into<String>) {
        self.0.push(ContractError {
            path: path.into(),
            message: message.into(),
        });
    }

    fn finish(mut self) -> Result<(), Self> {
        self.0.sort_by(|left, right| {
            left.path
                .cmp(&right.path)
                .then_with(|| left.message.cmp(&right.message))
        });
        if self.0.is_empty() { Ok(()) } else { Err(self) }
    }
}

impl fmt::Display for ContractErrors {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, error) in self.0.iter().enumerate() {
            if index > 0 {
                formatter.write_str("; ")?;
            }
            write!(formatter, "{}: {}", error.path, error.message)?;
        }
        Ok(())
    }
}

impl std::error::Error for ContractErrors {}

fn default_fov() -> f32 {
    38.0
}

fn default_near() -> f32 {
    0.1
}

fn default_far() -> f32 {
    100.0
}

fn validate_camera(camera: &CameraSpec, errors: &mut ContractErrors) {
    if !camera.position.finite() || !camera.target.finite() {
        errors.push("/camera", "camera position and target must be finite");
    }
    let delta = Vec3::new(
        camera.target.0[0] - camera.position.0[0],
        camera.target.0[1] - camera.position.0[1],
        camera.target.0[2] - camera.position.0[2],
    );
    if delta.length_squared() <= 1.0e-12 {
        errors.push("/camera/target", "camera target must differ from position");
    }
    if delta.0[0] * delta.0[0] + delta.0[2] * delta.0[2] <= 1.0e-12 {
        errors.push(
            "/camera/target",
            "camera direction cannot be parallel to the fixed +Y up vector",
        );
    }
    for (path, value) in [
        ("/camera/position", camera.position),
        ("/camera/target", camera.target),
    ] {
        if value
            .0
            .into_iter()
            .any(|component| component.abs() > MAX_ABS_POSITION)
        {
            errors.push(
                path,
                format!("camera coordinates must have magnitude <= {MAX_ABS_POSITION}"),
            );
        }
    }
    if !camera.fov_y_degrees.is_finite() || !(10.0..=120.0).contains(&camera.fov_y_degrees) {
        errors.push(
            "/camera/fovYDegrees",
            "vertical FOV must be finite in 10..=120 degrees",
        );
    }
    if !camera.near.is_finite()
        || !camera.far.is_finite()
        || camera.near <= 0.0
        || camera.far <= camera.near
        || camera.far > MAX_CAMERA_FAR
    {
        errors.push(
            "/camera/nearFar",
            format!("camera requires finite 0 < near < far <= {MAX_CAMERA_FAR}"),
        );
    }
}

fn validate_transform(transform: Transform3D, path: &str, errors: &mut ContractErrors) {
    if !transform.translation.finite()
        || !transform.rotation_degrees.finite()
        || !transform.scale.finite()
    {
        errors.push(path, "transform literals must be finite");
    }
    if transform.scale.0.into_iter().any(|value| value <= 0.0) {
        errors.push(format!("{path}/scale"), "scale axes must all be > 0");
    }
    if transform
        .translation
        .0
        .into_iter()
        .any(|value| value.abs() > MAX_ABS_POSITION)
    {
        errors.push(
            format!("{path}/translation"),
            format!("translation magnitude must be <= {MAX_ABS_POSITION}"),
        );
    }
    if transform
        .rotation_degrees
        .0
        .into_iter()
        .any(|value| value.abs() > MAX_ABS_ROTATION_DEGREES)
    {
        errors.push(
            format!("{path}/rotationDegrees"),
            format!("rotation magnitude must be <= {MAX_ABS_ROTATION_DEGREES}"),
        );
    }
    if transform.scale.0.into_iter().any(|value| value > MAX_SCALE) {
        errors.push(
            format!("{path}/scale"),
            format!("scale axes must be <= {MAX_SCALE}"),
        );
    }
}

fn validate_intensity(value: f32, path: &str, errors: &mut ContractErrors) {
    if !value.is_finite() || !(0.0..=4.0).contains(&value) {
        errors.push(path, "light intensity must be finite in 0..=4");
    }
}

fn validate_key(value: &str, path: &str, errors: &mut ContractErrors) {
    if value.is_empty()
        || value.len() > 96
        || value.contains("::")
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_./".contains(&byte))
    {
        errors.push(
            path,
            "key must be 1..=96 ASCII [A-Za-z0-9._/-] bytes and cannot contain `::`",
        );
    }
}

fn validate_control(value: &str, path: &str, errors: &mut ContractErrors) {
    if value.is_empty()
        || value.len() > 96
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
    {
        errors.push(path, "control name must be 1..=96 ASCII identifier bytes");
    }
}

fn check_max(errors: &mut ContractErrors, path: &str, value: Option<u64>, maximum: u64) {
    match value {
        Some(value) if value <= maximum => {}
        Some(value) => errors.push(path, format!("{value} exceeds maximum {maximum}")),
        None => errors.push(path, "count overflow"),
    }
}
