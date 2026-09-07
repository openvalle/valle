use serde::Deserialize;

use crate::ContentDigest;

use super::{ContractErrors, MAX_ABS_POSITION, MAX_MODEL_BYTES, MAX_TRIANGLES, MAX_VERTICES};

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
    pub(crate) position: [f32; 3],
    pub(crate) normal: [f32; 3],
    pub(crate) uv: [f32; 2],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrimitiveRange {
    pub first_index: u32,
    pub index_count: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AdmittedModel {
    pub(crate) content_digest: ContentDigest,
    pub(crate) source_bytes: u64,
    pub(crate) vertices: Vec<ModelVertex>,
    pub(crate) indices: Vec<u32>,
    pub(crate) primitives: Vec<PrimitiveRange>,
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

    pub fn indices(&self) -> &[u32] {
        &self.indices
    }

    pub fn primitives(&self) -> &[PrimitiveRange] {
        &self.primitives
    }

    pub fn triangle_count(&self) -> u32 {
        self.indices.len() as u32 / 3
    }
}

/// Admit the deliberately narrow Valle GLB v1 geometry profile.
///
/// Accepted files contain exactly one JSON chunk and one embedded BIN chunk. Geometry is triangle
/// primitives with tightly packed FLOAT POSITION/NORMAL/TEXCOORD_0 and U16/U32 indices. Nodes may
/// only name meshes; every mesh must appear once in the default scene. Materials, images, external
/// URI, extensions, transforms, skins, morphs and animation are rejected: Scene3D owns transforms
/// and material textures are separately bound project image controls.
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
            format!("not in the exact Valle GLB v1 subset: {error}"),
        );
        errors
    })?;
    validate_root(&root, bin, &mut errors);
    errors.finish()?;
    let mut errors = ContractErrors::default();

    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    let mut primitives = Vec::new();
    // Enforce budgets incrementally before allocation. Many primitives may reuse a large accessor,
    // so a small valid GLB could otherwise cause unbounded expansion before a final total-size
    // check.
    'meshes: for mesh in &root.meshes {
        for primitive in &mesh.primitives {
            let positions = read_f32_accessor::<3>(&root, bin, primitive.attributes.position)?;
            if vertices.len() + positions.len() > MAX_VERTICES as usize {
                errors.push(
                    "/glb/budget/vertices",
                    format!("aggregate decoded vertices must be <= {MAX_VERTICES}"),
                );
                break 'meshes;
            }
            let normals = read_f32_accessor::<3>(&root, bin, primitive.attributes.normal)?;
            let uvs = read_f32_accessor::<2>(&root, bin, primitive.attributes.texcoord_0)?;
            if positions.len() != normals.len() || positions.len() != uvs.len() {
                errors.push(
                    "/glb/meshes",
                    "POSITION, NORMAL and TEXCOORD_0 counts must match",
                );
                continue;
            }
            let base = vertices.len() as u32;
            for ((position, normal), uv) in positions.into_iter().zip(normals).zip(uvs) {
                if position
                    .into_iter()
                    .any(|value| !value.is_finite() || value.abs() > MAX_ABS_POSITION)
                {
                    errors.push(
                        "/glb/accessors/POSITION",
                        format!("positions must be finite with magnitude <= {MAX_ABS_POSITION}"),
                    );
                }
                let normal_len = normal.into_iter().map(|value| value * value).sum::<f32>();
                if normal.into_iter().any(|value| !value.is_finite())
                    || !normal_len.is_finite()
                    || normal_len <= 1.0e-12
                {
                    errors.push(
                        "/glb/accessors/NORMAL",
                        "normals must be finite and non-zero",
                    );
                }
                if uv
                    .into_iter()
                    .any(|value| !value.is_finite() || value.abs() > 1_000_000.0)
                {
                    errors.push(
                        "/glb/accessors/TEXCOORD_0",
                        "UV values must be finite and bounded",
                    );
                }
                vertices.push(ModelVertex {
                    position,
                    normal,
                    uv,
                });
            }
            let local_indices = read_indices(&root, bin, primitive.indices)?;
            if local_indices.len() % 3 != 0 {
                errors.push(
                    "/glb/accessors/indices",
                    "triangle index count must be divisible by three",
                );
            }
            if local_indices
                .iter()
                .any(|index| *index as usize >= vertices.len() - base as usize)
            {
                errors.push(
                    "/glb/accessors/indices",
                    "index references a vertex outside this primitive",
                );
                // Reject invalid indices before `base + index` can overflow; malformed input must
                // return diagnostics rather than panic.
                continue;
            }
            if (indices.len() + local_indices.len()) / 3 > MAX_TRIANGLES as usize {
                errors.push(
                    "/glb/budget/triangles",
                    format!("aggregate decoded triangles must be <= {MAX_TRIANGLES}"),
                );
                break 'meshes;
            }
            let first_index = indices.len() as u32;
            indices.extend(local_indices.into_iter().map(|index| base + index));
            primitives.push(PrimitiveRange {
                first_index,
                index_count: indices.len() as u32 - first_index,
            });
        }
    }
    errors.finish()?;

    Ok(AdmittedModel {
        content_digest: ContentDigest::of_bytes(bytes),
        source_bytes: bytes.len() as u64,
        vertices,
        indices,
        primitives,
    })
}

fn reject_forbidden_root_features(value: &serde_json::Value) -> Result<(), ContractErrors> {
    let mut errors = ContractErrors::default();
    let Some(object) = value.as_object() else {
        errors.push("/glb/json", "glTF root must be an object");
        return Err(errors);
    };
    for name in [
        "animations",
        "cameras",
        "images",
        "materials",
        "samplers",
        "skins",
        "textures",
    ] {
        if object.contains_key(name) {
            errors.push(
                format!("/glb/{name}"),
                "feature is outside Valle GLB v1; bind materials/textures through Scene3D controls",
            );
        }
    }
    for name in ["extensionsUsed", "extensionsRequired"] {
        if object
            .get(name)
            .and_then(serde_json::Value::as_array)
            .is_some_and(|items| !items.is_empty())
        {
            errors.push(
                format!("/glb/{name}"),
                "GLB extensions are not admitted in v1",
            );
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
        errors.push("/glb/extensions", "GLB extensions are not admitted in v1");
    }
    if root.scene >= root.scenes.len() {
        errors.push("/glb/scene", "default scene index is out of range");
    } else {
        let scene_nodes = &root.scenes[root.scene].nodes;
        if scene_nodes.len() != root.meshes.len() {
            errors.push(
                "/glb/scenes",
                "default scene must reference every mesh exactly once",
            );
        }
        for (mesh_index, node_index) in scene_nodes.iter().copied().enumerate() {
            if root.nodes.get(node_index).map(|node| node.mesh) != Some(mesh_index) {
                errors.push(
                    "/glb/scenes",
                    "default scene nodes must map one-to-one to meshes in exact order",
                );
            }
        }
    }
    for (index, view) in root.buffer_views.iter().enumerate() {
        if view.buffer != 0 || view.byte_stride.is_some() {
            errors.push(
                format!("/glb/bufferViews/{index}"),
                "only tightly packed views into embedded buffer 0 are admitted",
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
            .is_none_or(|end| end > bin.len())
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
            if primitive.mode != TRIANGLES {
                errors.push(
                    format!("/glb/meshes/{mesh_index}/primitives/{primitive_index}/mode"),
                    "only TRIANGLES mode is admitted",
                );
            }
            for accessor in [
                primitive.attributes.position,
                primitive.attributes.normal,
                primitive.attributes.texcoord_0,
                primitive.indices,
            ] {
                if accessor >= root.accessors.len() {
                    errors.push(
                        format!("/glb/meshes/{mesh_index}/primitives/{primitive_index}"),
                        "primitive accessor index is out of range",
                    );
                }
            }
            for accessor in [
                primitive.attributes.position,
                primitive.attributes.normal,
                primitive.attributes.texcoord_0,
            ] {
                if accessor_target(root, accessor) != Some(ARRAY_BUFFER) {
                    errors.push(
                        format!("/glb/meshes/{mesh_index}/primitives/{primitive_index}/attributes"),
                        "vertex accessors must use ARRAY_BUFFER views",
                    );
                }
            }
            if accessor_target(root, primitive.indices) != Some(ELEMENT_ARRAY_BUFFER) {
                errors.push(
                    format!("/glb/meshes/{mesh_index}/primitives/{primitive_index}/indices"),
                    "index accessor must use an ELEMENT_ARRAY_BUFFER view",
                );
            }
        }
    }
}

fn accessor_target(root: &Root, accessor_index: usize) -> Option<u32> {
    let accessor = root.accessors.get(accessor_index)?;
    root.buffer_views.get(accessor.buffer_view)?.target
}

fn read_f32_accessor<const N: usize>(
    root: &Root,
    bin: &[u8],
    index: usize,
) -> Result<Vec<[f32; N]>, ContractErrors> {
    let accessor = checked_accessor(root, index)?;
    let expected_type = match N {
        2 => "VEC2",
        3 => "VEC3",
        _ => unreachable!(),
    };
    if accessor.component_type != FLOAT || accessor.kind != expected_type || accessor.normalized {
        return accessor_error(index, "expected non-normalized FLOAT vector accessor");
    }
    let bytes = accessor_bytes(root, bin, index, N * 4)?;
    let mut values = Vec::with_capacity(accessor.count);
    for chunk in bytes.chunks_exact(N * 4) {
        let mut value = [0.0; N];
        for component in 0..N {
            value[component] = f32::from_le_bytes(
                chunk[component * 4..component * 4 + 4]
                    .try_into()
                    .expect("four-byte component"),
            );
        }
        values.push(value);
    }
    Ok(values)
}

fn read_indices(root: &Root, bin: &[u8], index: usize) -> Result<Vec<u32>, ContractErrors> {
    let accessor = checked_accessor(root, index)?;
    if accessor.kind != "SCALAR" || accessor.normalized {
        return accessor_error(index, "indices must be a non-normalized SCALAR accessor");
    }
    let size = match accessor.component_type {
        UNSIGNED_SHORT => 2,
        UNSIGNED_INT => 4,
        _ => return accessor_error(index, "indices must use UNSIGNED_SHORT or UNSIGNED_INT"),
    };
    let bytes = accessor_bytes(root, bin, index, size)?;
    Ok(bytes
        .chunks_exact(size)
        .map(|chunk| match size {
            2 => u16::from_le_bytes(chunk.try_into().expect("two-byte index")) as u32,
            4 => u32::from_le_bytes(chunk.try_into().expect("four-byte index")),
            _ => unreachable!(),
        })
        .collect())
}

fn checked_accessor(root: &Root, index: usize) -> Result<&Accessor, ContractErrors> {
    root.accessors.get(index).ok_or_else(|| {
        one_error(
            format!("/glb/accessors/{index}"),
            "accessor index is out of range",
        )
    })
}

fn accessor_bytes<'a>(
    root: &Root,
    bin: &'a [u8],
    index: usize,
    element_size: usize,
) -> Result<&'a [u8], ContractErrors> {
    let accessor = checked_accessor(root, index)?;
    let Some(view) = root.buffer_views.get(accessor.buffer_view) else {
        return accessor_error(index, "bufferView index is out of range");
    };
    let Some(length) = accessor.count.checked_mul(element_size) else {
        return accessor_error(index, "accessor byte length overflow");
    };
    let Some(start) = view.byte_offset.checked_add(accessor.byte_offset) else {
        return accessor_error(index, "accessor byte offset overflow");
    };
    let Some(end) = start.checked_add(length) else {
        return accessor_error(index, "accessor byte range overflow");
    };
    let Some(view_end) = view.byte_offset.checked_add(view.byte_length) else {
        return accessor_error(index, "buffer view byte range overflow");
    };
    if start % element_size.min(4) != 0 || end > view_end || end > bin.len() {
        return accessor_error(index, "accessor is misaligned or exceeds its buffer view");
    }
    Ok(&bin[start..end])
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
    scene: usize,
    #[serde(default)]
    extensions_used: Vec<String>,
    #[serde(default)]
    extensions_required: Vec<String>,
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
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Buffer {
    byte_length: usize,
    #[serde(default)]
    uri: Option<String>,
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
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Mesh {
    primitives: Vec<Primitive>,
    #[serde(default)]
    name: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Primitive {
    attributes: Attributes,
    indices: usize,
    #[serde(default = "triangle_mode")]
    mode: u32,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Attributes {
    #[serde(rename = "POSITION")]
    position: usize,
    #[serde(rename = "NORMAL")]
    normal: usize,
    #[serde(rename = "TEXCOORD_0")]
    texcoord_0: usize,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Node {
    mesh: usize,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Scene {
    nodes: Vec<usize>,
    #[serde(default, rename = "name")]
    _name: Option<String>,
}

const fn triangle_mode() -> u32 {
    TRIANGLES
}
