//! Shared contracts for controlled 3D texture layers: scene types, frame scalars, budgets, GLB
//! admission, preparation keys, and deterministic software rasterization. Native and Wasm consumers
//! use the same validation without a separate mutable scene runtime.

mod asset;
mod environment;
mod material;
pub use material::*;
mod raster;

pub use asset::{
    AdmittedModel, MaterialImage, ModelInstance, ModelMaterial, ModelMesh, ModelNode, ModelVertex,
    PrimitiveRange, admit_glb,
};
pub use environment::{EnvironmentAsset, EnvironmentSettings};
pub use raster::{
    AnchorProjection, BACKGROUND_NODE_ID, BACKGROUND_OBJECT_ID, CLEAR_DEPTH, PreparedScene,
    RasterFrame, Scene3DAnchorMetadata, Scene3DFrameMetadata, Scene3DObjectMetadata, Scene3DPick,
    ScenePrepareCacheKey, SceneResources, prepare_cache_key, prepare_scene, project_anchors,
    render_scene, render_scene_reusing,
};

use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};

pub const MAX_LAYER_EDGE: u32 = 2_048;
pub const MAX_LAYER_PIXELS: u64 = 2_000_000;
pub const MAX_FRAME_BUFFER_BYTES: u64 = MAX_LAYER_PIXELS * 10; // RGBA8 + u16 depth/object/node
pub const MAX_MODEL_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_VERTICES: u32 = 65_535;
pub const MAX_TRIANGLES: u32 = 20_000;
pub const MAX_MESHES: usize = 8;
pub const MAX_MODEL_NODES: usize = 256;
pub const MAX_NODE_DEPTH: usize = 32;
pub const MAX_MATERIALS: usize = MAX_MESHES;
// Texture count, pixel count and decoded mip/environment storage are bounded independently.
pub const MAX_TEXTURES: u32 = 8;
pub const MAX_TEXTURE_PIXELS: u64 = 24 * 1024 * 1024;
pub const MAX_TEXTURE_STORAGE_BYTES: u64 = 128 * 1024 * 1024;
pub const MAX_ANCHORS: usize = 32;
pub const MAX_DIRECTIONAL_LIGHTS: usize = 2;
pub const MAX_FRAME_SCALARS: usize = 12
    + MAX_MESHES * ((1 + MAX_MODEL_NODES) * 9 + (1 + MAX_MATERIALS) * MATERIAL_FRAME_SCALARS)
    + 5
    + 12
    + MAX_DIRECTIONAL_LIGHTS * 8;
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
    pub meshes: Vec<MeshSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lights: Vec<LightKind>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub anchors: Vec<AnchorSpec>,
    #[serde(default, skip_serializing_if = "PbrOptions::is_default")]
    pub pbr: PbrOptions,
}

/// Immutable lighting/output configuration for embedded PBR materials.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct PbrOptions {
    pub environment: Option<EnvironmentSpec>,
    pub tone_mapping: ToneMapping,
}
impl Default for PbrOptions {
    fn default() -> Self {
        Self {
            environment: None,
            tone_mapping: ToneMapping::None,
        }
    }
}
impl PbrOptions {
    fn is_default(&self) -> bool {
        self == &Self::default()
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnvironmentSpec {
    pub control: String,
    pub background: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum ToneMapping {
    None,
    Aces,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MeshSpec {
    pub key: String,
    /// Logical project control name, resolved to admitted content before rasterization.
    pub model_control: String,
    pub material: MaterialSpec,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub material_overrides: Vec<MaterialOverrideSpec>,
    /// Original GLB node indices whose complete local transforms are supplied per frame.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub node_ids: Vec<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum MaterialKind {
    Unlit,
    Lambert,
    Pbr,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum LightKind {
    Ambient,
    Directional,
    Hemisphere,
}
impl LightKind {
    pub fn scalar_count(self) -> usize {
        match self {
            Self::Ambient => 5,
            Self::Directional => 8,
            Self::Hemisphere => 12,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum LightFrameState {
    Ambient {
        color: Color4,
        intensity: f32,
    },
    Directional {
        color: Color4,
        direction: Vec3,
        intensity: f32,
    },
    Hemisphere {
        sky_color: Color4,
        ground_color: Color4,
        direction: Vec3,
        intensity: f32,
    },
}
impl LightFrameState {
    pub fn kind(&self) -> LightKind {
        match self {
            Self::Ambient { .. } => LightKind::Ambient,
            Self::Directional { .. } => LightKind::Directional,
            Self::Hemisphere { .. } => LightKind::Hemisphere,
        }
    }
    pub fn validate(&self) -> Result<(), ContractErrors> {
        let mut errors = ContractErrors::default();
        self.validate_at("/light", &mut errors);
        errors.finish()
    }
    fn validate_at(&self, path: &str, errors: &mut ContractErrors) {
        let intensity = match self {
            Self::Ambient { intensity, .. }
            | Self::Directional { intensity, .. }
            | Self::Hemisphere { intensity, .. } => *intensity,
        };
        validate_intensity(intensity, &format!("{path}/intensity"), errors);
        let mut color = |value: Color4, name: &str| {
            if !value.valid() || value.0[3] != 1.0 {
                errors.push(
                    format!("{path}/{name}"),
                    "light colors must be opaque finite sRGB colors",
                );
            }
        };
        match self {
            Self::Ambient { color: c, .. } | Self::Directional { color: c, .. } => {
                color(*c, "color")
            }
            Self::Hemisphere {
                sky_color,
                ground_color,
                ..
            } => {
                color(*sky_color, "skyColor");
                color(*ground_color, "groundColor");
            }
        }
        if let Self::Directional { direction, .. } | Self::Hemisphere { direction, .. } = self {
            if !direction.finite()
                || direction.length_squared() <= 1.0e-12
                || direction.0.iter().any(|v| v.abs() > MAX_ABS_POSITION)
            {
                errors.push(
                    format!("{path}/direction"),
                    "light direction must be a finite, bounded, non-zero vector",
                );
            }
        }
    }
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

/// Motion evaluates its expression graph first, converts finite f64 to f32 once, then sends
/// a complete `Frame3DState` to Native or Wasm raster; the core never reads a clock or prior frame.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Frame3DState {
    pub exposure: f32,
    pub environment_intensity: f32,
    pub environment_rotation_degrees: f32,
    pub camera: CameraFrameState,
    pub meshes: Vec<MeshFrameState>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lights: Vec<LightFrameState>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CameraFrameState {
    pub position: Vec3,
    pub target: Vec3,
    pub fov_y_degrees: f32,
    pub near: f32,
    pub far: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MeshFrameState {
    pub key: String,
    pub material: MaterialFrameState,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub material_overrides: Vec<MaterialOverrideState>,
    pub transform: Transform3D,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<NodeFrameState>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NodeFrameState {
    pub id: u32,
    /// Complete local TRS replacing the source node's local transform for this frame.
    pub transform: Transform3D,
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
        self.layer_pixels()?.checked_mul(10)
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

impl MeshSpec {
    pub fn texture_controls(&self) -> impl Iterator<Item = (&str, TextureRole)> {
        std::iter::once(&self.material)
            .chain(self.material_overrides.iter().map(|m| &m.material))
            .flat_map(MaterialSpec::texture_controls)
    }
}

impl Scene3DSpec {
    pub fn validate(&self) -> Result<(), ContractErrors> {
        let mut errors = ContractErrors::default();
        if let Some(environment) = &self.pbr.environment {
            validate_control(
                &environment.control,
                "/pbr/environment/control",
                &mut errors,
            );
        }
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
            if mesh.node_ids.len() > MAX_MODEL_NODES
                || mesh
                    .node_ids
                    .iter()
                    .any(|&id| id as usize >= MAX_MODEL_NODES)
                || mesh.node_ids.iter().copied().collect::<BTreeSet<_>>().len()
                    != mesh.node_ids.len()
            {
                errors.push(
                    format!("{path}/nodeIds"),
                    "node IDs must be distinct and within the node budget",
                );
            }
            mesh.material
                .validate_at(&format!("{path}/material"), &mut errors);
            let mut material_ids = BTreeSet::new();
            for (at, material) in mesh.material_overrides.iter().enumerate() {
                if material.id as usize >= MAX_MATERIALS || !material_ids.insert(material.id) {
                    errors.push(
                        format!("{path}/materialOverrides/{at}/id"),
                        "material IDs must be distinct and within the material budget",
                    );
                }
                material.material.validate_at(
                    &format!("{path}/materialOverrides/{at}/material"),
                    &mut errors,
                );
            }
        }
        for (kind, limit, label) in [
            (LightKind::Ambient, 1, "ambient"),
            (LightKind::Hemisphere, 1, "hemisphere"),
            (
                LightKind::Directional,
                MAX_DIRECTIONAL_LIGHTS,
                "directional",
            ),
        ] {
            if self.lights.iter().filter(|&&value| value == kind).count() > limit {
                errors.push(
                    "/lights",
                    format!("Scene3D allows at most {limit} {label} lights"),
                );
            }
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
        12 + self
            .meshes
            .iter()
            .map(|m| {
                (1 + m.node_ids.len()) * 9
                    + (1 + m.material_overrides.len()) * MATERIAL_FRAME_SCALARS
            })
            .sum::<usize>()
            + self
                .lights
                .iter()
                .map(|light| light.scalar_count())
                .sum::<usize>()
    }
}

impl Frame3DState {
    pub fn validate_for(&self, scene: &Scene3DSpec) -> Result<(), ContractErrors> {
        let mut errors = ContractErrors::default();
        if !self.environment_intensity.is_finite()
            || !(0.0..=16.0).contains(&self.environment_intensity)
        {
            errors.push(
                "/environmentIntensity",
                "environment intensity must be finite in 0..=16",
            );
        }
        if !self.environment_rotation_degrees.is_finite()
            || self.environment_rotation_degrees.abs() > MAX_ABS_ROTATION_DEGREES
        {
            errors.push(
                "/environmentRotationDegrees",
                "environment rotation must be finite and within the rotation budget",
            );
        }

        validate_camera(&self.camera, &mut errors);
        if !self.exposure.is_finite() || !(0.0..=16.0).contains(&self.exposure) {
            errors.push("/exposure", "exposure must be finite in 0..=16");
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
            let path = format!("/meshes/{index}");
            validate_transform(mesh.transform, &format!("{path}/transform"), &mut errors);
            if scene.meshes.get(index).is_some_and(|spec| {
                spec.node_ids != mesh.nodes.iter().map(|n| n.id).collect::<Vec<_>>()
            }) {
                errors.push(
                    format!("{path}/nodes"),
                    "frame node IDs must match the frozen node bindings in exact order",
                );
            }
            mesh.material
                .validate_at(&format!("{path}/material"), &mut errors);
            if scene.meshes.get(index).is_some_and(|spec| {
                spec.material_overrides
                    .iter()
                    .map(|m| m.id)
                    .collect::<Vec<_>>()
                    != mesh
                        .material_overrides
                        .iter()
                        .map(|m| m.id)
                        .collect::<Vec<_>>()
            }) {
                errors.push(
                    format!("{path}/materialOverrides"),
                    "frame material IDs must match the frozen material bindings in exact order",
                );
            }
            for (at, material) in mesh.material_overrides.iter().enumerate() {
                material.material.validate_at(
                    &format!("{path}/materialOverrides/{at}/material"),
                    &mut errors,
                );
            }
            for (at, node) in mesh.nodes.iter().enumerate() {
                validate_transform(
                    node.transform,
                    &format!("{path}/nodes/{at}/transform"),
                    &mut errors,
                );
            }
        }
        if self
            .lights
            .iter()
            .map(LightFrameState::kind)
            .collect::<Vec<_>>()
            != scene.lights
        {
            errors.push(
                "/lights",
                "frame lights must match the frozen light kinds in exact order",
            );
        }
        for (index, light) in self.lights.iter().enumerate() {
            light.validate_at(&format!("/lights/{index}"), &mut errors);
        }
        errors.finish()
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

impl CameraFrameState {
    pub fn validate(&self) -> Result<(), ContractErrors> {
        let mut errors = ContractErrors::default();
        validate_camera(self, &mut errors);
        errors.finish()
    }
}

fn validate_camera(camera: &CameraFrameState, errors: &mut ContractErrors) {
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

impl Transform3D {
    pub fn validate(&self) -> Result<(), ContractErrors> {
        let mut errors = ContractErrors::default();
        validate_transform(*self, "/transform", &mut errors);
        errors.finish()
    }
}

fn validate_transform(transform: Transform3D, path: &str, errors: &mut ContractErrors) {
    if !transform.translation.finite()
        || !transform.rotation_degrees.finite()
        || !transform.scale.finite()
    {
        errors.push(path, "transform components must be finite");
    }
    if transform
        .scale
        .0
        .into_iter()
        .any(|value| value.abs() < 0.000001)
    {
        errors.push(
            format!("{path}/scale"),
            "scale axis magnitudes must all be >= 0.000001",
        );
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
    if transform
        .scale
        .0
        .into_iter()
        .any(|value| value.abs() > MAX_SCALE)
    {
        errors.push(
            format!("{path}/scale"),
            format!("scale axis magnitudes must be <= {MAX_SCALE}"),
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
