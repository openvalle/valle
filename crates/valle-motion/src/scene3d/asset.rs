use serde::Deserialize;
mod geometry;
mod material;
mod nodes;
pub use material::{MaterialImage, ModelMaterial};
pub(crate) use material::{ModelTextureBinding, linear_to_srgb, srgb_to_linear};
pub(crate) use nodes::Affine;
pub use nodes::{ModelInstance, ModelNode};

use crate::ContentDigest;

use super::{ContractErrors, MAX_MODEL_BYTES};

const GLB_MAGIC: u32 = 0x4654_6c67;
const GLB_VERSION: u32 = 2;
const JSON_CHUNK: u32 = 0x4e4f_534a;
const BIN_CHUNK: u32 = 0x004e_4942;
const ARRAY_BUFFER: u32 = 34_962;
const ELEMENT_ARRAY_BUFFER: u32 = 34_963;
const FLOAT: u32 = 5_126;
const UNSIGNED_SHORT: u32 = 5_123;
const UNSIGNED_INT: u32 = 5_125;
const TRIANGLES: u32 = 4;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrimitiveRange {
    pub first_index: u32,
    pub index_count: u32,
    pub material_index: Option<u32>,
    pub has_uv: bool,
}

/// A source mesh owns geometry once, regardless of the number of nodes that instance it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelMesh {
    pub first_primitive: u32,
    pub primitive_count: u32,
    pub vertex_count: u32,
    pub triangle_count: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AdmittedModel {
    pub(crate) content_digest: ContentDigest,
    pub(crate) source_bytes: u64,
    pub(crate) vertices: Vec<ModelVertex>,
    pub(crate) indices: Vec<u32>,
    pub(crate) primitives: Vec<PrimitiveRange>,
    pub(crate) meshes: Vec<ModelMesh>,
    pub(crate) nodes: Vec<ModelNode>,
    pub(crate) instances: Vec<ModelInstance>,
    pub(crate) materials: Vec<ModelMaterial>,
    pub(crate) images: Vec<MaterialImage>,
}

impl AdmittedModel {
    pub fn content_digest(&self) -> ContentDigest {
        self.content_digest
    }

    pub fn source_bytes(&self) -> u64 {
        self.source_bytes
    }

    pub fn vertex_count(&self) -> u32 {
        self.vertices.len() as u32
    }

    pub fn vertices(&self) -> &[ModelVertex] {
        &self.vertices
    }

    pub fn indices(&self) -> &[u32] {
        &self.indices
    }

    pub fn primitives(&self) -> &[PrimitiveRange] {
        &self.primitives
    }

    pub fn meshes(&self) -> &[ModelMesh] {
        &self.meshes
    }
    pub fn nodes(&self) -> &[ModelNode] {
        &self.nodes
    }
    pub fn instances(&self) -> &[ModelInstance] {
        &self.instances
    }
    pub fn rendered_vertex_count(&self) -> u32 {
        self.instances
            .iter()
            .map(|i| self.meshes[i.mesh as usize].vertex_count)
            .sum()
    }
    pub fn rendered_triangle_count(&self) -> u32 {
        self.instances
            .iter()
            .map(|i| self.meshes[i.mesh as usize].triangle_count)
            .sum()
    }

    pub fn triangle_count(&self) -> u32 {
        self.indices.len() as u32 / 3
    }

    pub fn materials(&self) -> &[ModelMaterial] {
        &self.materials
    }
    pub fn images(&self) -> &[MaterialImage] {
        &self.images
    }
}

/// Admit closed static glTF geometry, materials and node instances. Geometry stays in source
/// coordinates; transforms are immutable hierarchy data, evaluated without a previous frame.
/// Embedded images are decoded and mipmapped within fixed budgets. External URIs, extensions,
/// skins, morphs and imported animation are outside the supported profile.
pub fn admit_glb(bytes: &[u8]) -> Result<AdmittedModel, ContractErrors> {
    let mut errors = ContractErrors::default();
    if bytes.len() as u64 > MAX_MODEL_BYTES {
        errors.push(
            "/glb/bytes",
            format!("GLB bytes must be <= {MAX_MODEL_BYTES}"),
        );
        return Err(errors);
    }
    if bytes.len() < 20 {
        errors.push("/glb", "GLB is shorter than its header and first chunk");
        return Err(errors);
    }
    let magic = read_u32(bytes, 0).unwrap_or_default();
    let version = read_u32(bytes, 4).unwrap_or_default();
    let declared_length = read_u32(bytes, 8).unwrap_or_default() as usize;
    if magic != GLB_MAGIC {
        errors.push("/glb/magic", "expected glTF magic");
    }
    if version != GLB_VERSION {
        errors.push("/glb/version", "only GLB version 2 is admitted");
    }
    if declared_length != bytes.len() {
        errors.push(
            "/glb/length",
            "declared GLB length must exactly match bound asset bytes",
        );
    }
    errors.finish()?;
    let mut errors = ContractErrors::default();

    let mut cursor = 12usize;
    let mut json = None;
    let mut bin = None;
    while cursor < bytes.len() {
        let Some(chunk_length) = read_u32(bytes, cursor).map(|value| value as usize) else {
            errors.push("/glb/chunks", "truncated chunk header");
            break;
        };
        let Some(chunk_type) = read_u32(bytes, cursor + 4) else {
            errors.push("/glb/chunks", "truncated chunk type");
            break;
        };
        cursor += 8;
        let Some(end) = cursor.checked_add(chunk_length) else {
            errors.push("/glb/chunks", "chunk length overflow");
            break;
        };
        if end > bytes.len() || chunk_length % 4 != 0 {
            errors.push(
                "/glb/chunks",
                "chunk must be four-byte aligned and contained in the GLB",
            );
            break;
        }
        match chunk_type {
            JSON_CHUNK if json.is_none() && bin.is_none() => json = Some(&bytes[cursor..end]),
            BIN_CHUNK if json.is_some() && bin.is_none() => bin = Some(&bytes[cursor..end]),
            JSON_CHUNK | BIN_CHUNK => {
                errors.push("/glb/chunks", "GLB must contain JSON then BIN exactly once")
            }
            _ => errors.push("/glb/chunks", "unknown GLB chunk type is not admitted"),
        }
        cursor = end;
    }
    if cursor != bytes.len() {
        errors.push("/glb/chunks", "GLB chunks do not consume the asset exactly");
    }
    let Some(json) = json else {
        errors.push("/glb/json", "missing JSON chunk");
        return Err(errors);
    };
    let Some(bin) = bin else {
        errors.push("/glb/bin", "missing embedded BIN chunk");
        return Err(errors);
    };
    errors.finish()?;
    let mut errors = ContractErrors::default();

    let json = trim_json_padding(json);
    let root_value: serde_json::Value = serde_json::from_slice(json).map_err(|error| {
        let mut errors = ContractErrors::default();
        errors.push("/glb/json", format!("invalid glTF JSON: {error}"));
        errors
    })?;
    reject_forbidden_root_features(&root_value)?;
    let root: Root = serde_json::from_value(root_value).map_err(|error| {
        let mut errors = ContractErrors::default();
        errors.push(
            "/glb/json",
            format!("unsupported or invalid static GLB: {error}"),
        );
        errors
    })?;
    validate_root(&root, bin, &mut errors);
    errors.finish()?;
    let (materials, images) = material::admit_materials(&root, bin)?;
    let (vertices, indices, primitives, meshes) = geometry::admit_geometry(&root, bin, &materials)?;
    let (nodes, instances) = nodes::admit_nodes(&root, &meshes, &primitives, &indices, &vertices)?;
    Ok(AdmittedModel {
        content_digest: ContentDigest::of_bytes(bytes),
        source_bytes: bytes.len() as u64,
        vertices,
        indices,
        primitives,
        meshes,
        nodes,
        instances,
        materials,
        images,
    })
}

fn reject_forbidden_root_features(value: &serde_json::Value) -> Result<(), ContractErrors> {
    let mut errors = ContractErrors::default();
    let Some(object) = value.as_object() else {
        errors.push("/glb/json", "glTF root must be an object");
        return Err(errors);
    };
    for name in ["animations", "cameras", "skins"] {
        if object.contains_key(name) {
            errors.push(
                format!("/glb/{name}"),
                "imported animations, cameras and skins are outside the static GLB profile",
            );
        }
    }
    for name in ["extensionsUsed", "extensionsRequired"] {
        if object
            .get(name)
            .and_then(serde_json::Value::as_array)
            .is_some_and(|items| !items.is_empty())
        {
            errors.push(format!("/glb/{name}"), "GLB extensions are not admitted");
        }
    }
    errors.finish()
}

fn validate_root(root: &Root, bin: &[u8], errors: &mut ContractErrors) {
    if root.asset.version != "2.0"
        || root
            .asset
            .min_version
            .as_deref()
            .is_some_and(|v| v != "2.0")
    {
        errors.push(
            "/glb/asset/version",
            "asset version must be exactly glTF 2.0",
        );
    }
    if root.buffers.len() != 1 || root.buffers[0].uri.is_some() {
        errors.push(
            "/glb/buffers",
            "exactly one embedded buffer with no URI is required",
        );
    } else if root.buffers[0].byte_length > bin.len()
        || bin.len().saturating_sub(root.buffers[0].byte_length) > 3
    {
        errors.push(
            "/glb/buffers/0/byteLength",
            "buffer byteLength must match BIN chunk apart from at most three padding bytes",
        );
    }
    if root.meshes.is_empty() {
        errors.push("/glb/meshes", "at least one mesh is required");
    }
    if !root.extensions_used.is_empty() || !root.extensions_required.is_empty() {
        errors.push("/glb/extensions", "GLB extensions are not admitted");
    }
    if root.scene >= root.scenes.len() {
        errors.push("/glb/scene", "default scene index is out of range");
    }
    if root.nodes.len() > super::MAX_MODEL_NODES || root.meshes.len() > super::MAX_MODEL_NODES {
        errors.push(
            "/glb/budget/nodes",
            "model node and mesh counts exceed the fixed budget",
        );
    }
    for (index, view) in root.buffer_views.iter().enumerate() {
        if view.buffer != 0 {
            errors.push(
                format!("/glb/bufferViews/{index}"),
                "views must reference embedded buffer 0",
            );
        }
        if view
            .byte_stride
            .is_some_and(|s| !(4..=252).contains(&s) || s % 4 != 0)
        {
            errors.push(
                format!("/glb/bufferViews/{index}/byteStride"),
                "vertex byteStride must be a multiple of four in 4..=252",
            );
        }
        if view
            .target
            .is_some_and(|target| target != ARRAY_BUFFER && target != ELEMENT_ARRAY_BUFFER)
        {
            errors.push(
                format!("/glb/bufferViews/{index}/target"),
                "unknown buffer view target",
            );
        }
        if view
            .byte_offset
            .checked_add(view.byte_length)
            .is_none_or(|end| end > root.buffers.first().map_or(0, |b| b.byte_length))
        {
            errors.push(
                format!("/glb/bufferViews/{index}"),
                "buffer view exceeds embedded BIN chunk",
            );
        }
    }
    for (mesh_index, mesh) in root.meshes.iter().enumerate() {
        if mesh.primitives.is_empty() {
            errors.push(
                format!("/glb/meshes/{mesh_index}/primitives"),
                "mesh must contain at least one triangle primitive",
            );
        }
        for (primitive_index, primitive) in mesh.primitives.iter().enumerate() {
            if primitive
                .material
                .is_some_and(|index| index >= root.materials.len())
            {
                errors.push(
                    format!("/glb/meshes/{mesh_index}/primitives/{primitive_index}/material"),
                    "material index is out of range",
                );
            }
            if primitive.mode != TRIANGLES {
                errors.push(
                    format!("/glb/meshes/{mesh_index}/primitives/{primitive_index}/mode"),
                    "only TRIANGLES mode is admitted",
                );
            }
            for accessor in std::iter::once(primitive.attributes.position)
                .chain(primitive.attributes.normal)
                .chain(primitive.attributes.texcoord_0)
                .chain(primitive.indices)
            {
                if accessor >= root.accessors.len() {
                    errors.push(
                        format!("/glb/meshes/{mesh_index}/primitives/{primitive_index}"),
                        "primitive accessor index is out of range",
                    );
                }
            }
        }
    }
}

fn accessor_error<T>(index: usize, message: &str) -> Result<T, ContractErrors> {
    Err(one_error(format!("/glb/accessors/{index}"), message))
}

fn one_error(path: impl Into<String>, message: impl Into<String>) -> ContractErrors {
    let mut errors = ContractErrors::default();
    errors.push(path, message);
    errors
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    let end = offset.checked_add(4)?;
    Some(u32::from_le_bytes(bytes.get(offset..end)?.try_into().ok()?))
}

fn trim_json_padding(mut bytes: &[u8]) -> &[u8] {
    while bytes.last().is_some_and(|byte| *byte == b' ' || *byte == 0) {
        bytes = &bytes[..bytes.len() - 1];
    }
    bytes
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Root {
    asset: Asset,
    buffers: Vec<Buffer>,
    buffer_views: Vec<BufferView>,
    accessors: Vec<Accessor>,
    meshes: Vec<Mesh>,
    nodes: Vec<Node>,
    scenes: Vec<Scene>,
    #[serde(default)]
    scene: usize,
    #[serde(default)]
    materials: Vec<material::GltfMaterial>,
    #[serde(default)]
    images: Vec<material::GltfImage>,
    #[serde(default)]
    textures: Vec<material::GltfTexture>,
    #[serde(default)]
    samplers: Vec<material::GltfSampler>,
    #[serde(default)]
    extensions_used: Vec<String>,
    #[serde(default)]
    extensions_required: Vec<String>,
    #[serde(default, rename = "extras")]
    _extras: Option<serde_json::Value>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Asset {
    version: String,
    #[serde(default)]
    min_version: Option<String>,
    #[serde(default)]
    generator: Option<String>,
    #[serde(default, rename = "copyright")]
    _copyright: Option<String>,
    #[serde(default, rename = "extras")]
    _extras: Option<serde_json::Value>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Buffer {
    byte_length: usize,
    #[serde(default)]
    uri: Option<String>,
    #[serde(default, rename = "name")]
    _name: Option<String>,
    #[serde(default, rename = "extras")]
    _extras: Option<serde_json::Value>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BufferView {
    buffer: usize,
    #[serde(default)]
    byte_offset: usize,
    byte_length: usize,
    #[serde(default)]
    byte_stride: Option<usize>,
    #[serde(default)]
    target: Option<u32>,
    #[serde(default, rename = "name")]
    _name: Option<String>,
    #[serde(default, rename = "extras")]
    _extras: Option<serde_json::Value>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Accessor {
    buffer_view: usize,
    #[serde(default)]
    byte_offset: usize,
    component_type: u32,
    #[serde(default)]
    normalized: bool,
    count: usize,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    min: Option<Vec<f32>>,
    #[serde(default)]
    max: Option<Vec<f32>>,
    #[serde(default, rename = "name")]
    _name: Option<String>,
    #[serde(default, rename = "extras")]
    _extras: Option<serde_json::Value>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Mesh {
    primitives: Vec<Primitive>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default, rename = "extras")]
    _extras: Option<serde_json::Value>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Primitive {
    attributes: Attributes,
    #[serde(default)]
    indices: Option<usize>,
    #[serde(default)]
    material: Option<usize>,
    #[serde(default = "triangle_mode")]
    mode: u32,
    #[serde(default, rename = "extras")]
    _extras: Option<serde_json::Value>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Attributes {
    #[serde(rename = "POSITION")]
    position: usize,
    #[serde(default, rename = "NORMAL")]
    normal: Option<usize>,
    #[serde(default, rename = "TEXCOORD_0")]
    texcoord_0: Option<usize>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Node {
    #[serde(default)]
    mesh: Option<usize>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    children: Vec<usize>,
    #[serde(default)]
    matrix: Option<[f32; 16]>,
    #[serde(default)]
    rotation: Option<[f32; 4]>,
    #[serde(default)]
    translation: Option<[f32; 3]>,
    #[serde(default)]
    scale: Option<[f32; 3]>,
    #[serde(default, rename = "extras")]
    _extras: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Scene {
    nodes: Vec<usize>,
    #[serde(default, rename = "name")]
    _name: Option<String>,
    #[serde(default, rename = "extras")]
    _extras: Option<serde_json::Value>,
}

const fn triangle_mode() -> u32 {
    TRIANGLES
}
