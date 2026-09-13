//! Decode strided accessors into shared source geometry; never apply instance transforms here.
use super::{
    ARRAY_BUFFER, ContractErrors, ELEMENT_ARRAY_BUFFER, FLOAT, ModelMaterial, ModelMesh,
    ModelVertex, PrimitiveRange, Root, UNSIGNED_INT, UNSIGNED_SHORT, accessor_error, one_error,
};
use crate::scene3d::{MAX_ABS_POSITION, MAX_TRIANGLES, MAX_VERTICES};

const UNSIGNED_BYTE: u32 = 5121;

type Geometry = (
    Vec<ModelVertex>,
    Vec<u32>,
    Vec<PrimitiveRange>,
    Vec<ModelMesh>,
);

pub(super) fn admit_geometry(
    root: &Root,
    bin: &[u8],
    materials: &[ModelMaterial],
) -> Result<Geometry, ContractErrors> {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    let mut primitives = Vec::new();
    let mut meshes = Vec::new();
    for mesh in &root.meshes {
        let first_primitive = primitives.len() as u32;
        let first_vertex = vertices.len();
        let first_index = indices.len();
        for primitive in &mesh.primitives {
            let positions = read_vectors::<3>(root, bin, primitive.attributes.position, false)?;
            // Stop before reading later attributes or expanding a primitive that cannot fit.
            check_vertices(vertices.len() + positions.len())?;
            if positions
                .iter()
                .flatten()
                .any(|v| !v.is_finite() || v.abs() > MAX_ABS_POSITION)
            {
                return Err(one_error(
                    "/glb/accessors/POSITION",
                    "positions must be finite and bounded",
                ));
            }
            let local_indices = match primitive.indices {
                Some(index) => read_indices(root, bin, index)?,
                None => (0..positions.len() as u32).collect(),
            };
            if local_indices.is_empty() || local_indices.len() % 3 != 0 {
                return Err(one_error(
                    "/glb/accessors/indices",
                    "triangle index count must be positive and divisible by three",
                ));
            }
            if local_indices.iter().any(|&i| i as usize >= positions.len()) {
                return Err(one_error(
                    "/glb/accessors/indices",
                    "index references a vertex outside this primitive",
                ));
            }
            if (indices.len() + local_indices.len()) / 3 > MAX_TRIANGLES as usize {
                return Err(one_error(
                    "/glb/budget/triangles",
                    "aggregate decoded triangles exceed the fixed budget",
                ));
            }
            let normals = primitive
                .attributes
                .normal
                .map(|a| read_vectors::<3>(root, bin, a, false))
                .transpose()?;
            if let Some(normals) = &normals {
                if normals.len() != positions.len() {
                    return Err(one_error(
                        "/glb/accessors/NORMAL",
                        "POSITION and NORMAL counts must match",
                    ));
                }
                if normals.iter().any(|n| {
                    let length: f32 = n.iter().map(|v| v * v).sum();
                    !length.is_finite() || length <= 1.0e-12
                }) {
                    return Err(one_error(
                        "/glb/accessors/NORMAL",
                        "normals must be finite and non-zero",
                    ));
                }
            }
            let has_uv = primitive.attributes.texcoord_0.is_some();
            if !has_uv && primitive.material.is_some_and(|m| materials[m].uses_uv()) {
                return Err(one_error(
                    "/glb/accessors/TEXCOORD_0",
                    "textured material requires TEXCOORD_0",
                ));
            }
            let uvs = primitive
                .attributes
                .texcoord_0
                .map(|a| read_vectors::<2>(root, bin, a, true))
                .transpose()?
                .unwrap_or_else(|| vec![[0.0; 2]; positions.len()]);
            if uvs.len() != positions.len()
                || uvs
                    .iter()
                    .flatten()
                    .any(|v| !v.is_finite() || v.abs() > 1_000_000.0)
            {
                return Err(one_error(
                    "/glb/accessors/TEXCOORD_0",
                    "UV values must be finite and bounded with the POSITION count",
                ));
            }
            let start = indices.len() as u32;
            if let Some(normals) = normals {
                let base = vertices.len() as u32;
                vertices.extend(positions.into_iter().zip(normals).zip(uvs).map(
                    |((position, normal), uv)| ModelVertex {
                        position,
                        normal: normalize(normal),
                        uv,
                    },
                ));
                indices.extend(local_indices.into_iter().map(|i| base + i));
            } else {
                // glTF requires flat normals when NORMAL is absent. Split triangle corners so a
                // shared index never smooths across faces. Degenerate triangles use a finite normal
                // and are discarded by raster setup, without contaminating neighbouring triangles.
                check_vertices(vertices.len() + local_indices.len())?;
                for triangle in local_indices.chunks_exact(3) {
                    let p = [
                        positions[triangle[0] as usize],
                        positions[triangle[1] as usize],
                        positions[triangle[2] as usize],
                    ];
                    let a = std::array::from_fn(|i| p[1][i] - p[0][i]);
                    let b = std::array::from_fn(|i| p[2][i] - p[0][i]);
                    let normal = normalize(cross(a, b));
                    for &index in triangle {
                        indices.push(vertices.len() as u32);
                        vertices.push(ModelVertex {
                            position: positions[index as usize],
                            normal,
                            uv: uvs[index as usize],
                        });
                    }
                }
            }
            primitives.push(PrimitiveRange {
                first_index: start,
                index_count: indices.len() as u32 - start,
                material_index: primitive.material.map(|i| i as u32),
                has_uv,
            });
        }
        meshes.push(ModelMesh {
            first_primitive,
            primitive_count: primitives.len() as u32 - first_primitive,
            vertex_count: (vertices.len() - first_vertex) as u32,
            triangle_count: ((indices.len() - first_index) / 3) as u32,
        });
    }
    Ok((vertices, indices, primitives, meshes))
}

fn check_vertices(count: usize) -> Result<(), ContractErrors> {
    if count > MAX_VERTICES as usize {
        Err(one_error(
            "/glb/budget/vertices",
            "aggregate decoded vertices exceed the fixed budget",
        ))
    } else {
        Ok(())
    }
}

pub(super) fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
pub(super) fn normalize(v: [f32; 3]) -> [f32; 3] {
    let length = libm::sqrtf(v.iter().map(|v| v * v).sum());
    if length > 0.0 {
        v.map(|v| v / length)
    } else {
        [0.0, 0.0, 1.0]
    }
}

struct AccessorBytes<'a> {
    bytes: &'a [u8],
    stride: usize,
    size: usize,
    count: usize,
}
impl<'a> AccessorBytes<'a> {
    fn elements(&self) -> impl Iterator<Item = &'a [u8]> + '_ {
        (0..self.count).map(|i| &self.bytes[i * self.stride..i * self.stride + self.size])
    }
}

fn accessor_bytes<'a>(
    root: &Root,
    bin: &'a [u8],
    index: usize,
    size: usize,
    component_size: usize,
    vertex: bool,
) -> Result<AccessorBytes<'a>, ContractErrors> {
    let accessor = &root.accessors[index];
    let Some(view) = root.buffer_views.get(accessor.buffer_view) else {
        return accessor_error(index, "bufferView index is out of range");
    };
    let limit = if vertex {
        MAX_VERTICES as usize
    } else {
        MAX_TRIANGLES as usize * 3
    };
    if accessor.count == 0 || accessor.count > limit {
        return accessor_error(
            index,
            "accessor count must be positive and within the geometry budget",
        );
    }
    if view.target.is_some_and(|t| {
        t != if vertex {
            ARRAY_BUFFER
        } else {
            ELEMENT_ARRAY_BUFFER
        }
    }) || (!vertex && view.byte_stride.is_some())
    {
        return accessor_error(index, "buffer view target or stride does not match its use");
    }
    let stride = view.byte_stride.unwrap_or(size);
    let start = view.byte_offset.checked_add(accessor.byte_offset);
    let end = (accessor.count - 1)
        .checked_mul(stride)
        .and_then(|v| v.checked_add(size))
        .and_then(|v| start?.checked_add(v));
    let view_end = view.byte_offset.checked_add(view.byte_length);
    if stride < size
        || stride % component_size != 0
        || accessor.byte_offset % component_size != 0
        || (vertex && (accessor.byte_offset % 4 != 0 || stride % 4 != 0))
        || start.is_none_or(|s| s % component_size != 0)
        || end.is_none_or(|e| e > view_end.unwrap_or(0) || e > bin.len())
    {
        return accessor_error(index, "accessor is misaligned or exceeds its buffer view");
    }
    Ok(AccessorBytes {
        bytes: &bin[start.unwrap()..end.unwrap()],
        stride,
        size,
        count: accessor.count,
    })
}

fn read_vectors<const N: usize>(
    root: &Root,
    bin: &[u8],
    index: usize,
    uv: bool,
) -> Result<Vec<[f32; N]>, ContractErrors> {
    let accessor = &root.accessors[index];
    let expected = if N == 2 { "VEC2" } else { "VEC3" };
    if accessor.kind != expected {
        return accessor_error(index, "incorrect vector accessor type");
    }
    let size = match (accessor.component_type, accessor.normalized, uv) {
        (FLOAT, false, _) => 4,
        (UNSIGNED_SHORT, true, true) => 2,
        (UNSIGNED_BYTE, true, true) => 1,
        _ => {
            return accessor_error(
                index,
                "expected FLOAT vector or normalized unsigned UV vector",
            );
        }
    };
    let bytes = accessor_bytes(root, bin, index, N * size, size, true)?;
    Ok(bytes
        .elements()
        .map(|element| {
            std::array::from_fn(|i| {
                let c = &element[i * size..(i + 1) * size];
                match size {
                    4 => f32::from_le_bytes(c.try_into().unwrap()),
                    2 => u16::from_le_bytes(c.try_into().unwrap()) as f32 / 65535.0,
                    _ => c[0] as f32 / 255.0,
                }
            })
        })
        .collect())
}

fn read_indices(root: &Root, bin: &[u8], index: usize) -> Result<Vec<u32>, ContractErrors> {
    let accessor = &root.accessors[index];
    if accessor.kind != "SCALAR" || accessor.normalized {
        return accessor_error(index, "indices must be a non-normalized SCALAR accessor");
    }
    let (size, restart) = match accessor.component_type {
        UNSIGNED_BYTE => (1, 255),
        UNSIGNED_SHORT => (2, 65535),
        UNSIGNED_INT => (4, u32::MAX),
        _ => {
            return accessor_error(
                index,
                "indices must use UNSIGNED_BYTE, UNSIGNED_SHORT or UNSIGNED_INT",
            );
        }
    };
    let bytes = accessor_bytes(root, bin, index, size, size, false)?;
    bytes
        .elements()
        .map(|c| {
            let value = match size {
                1 => c[0] as u32,
                2 => u16::from_le_bytes(c.try_into().unwrap()) as u32,
                _ => u32::from_le_bytes(c.try_into().unwrap()),
            };
            if value == restart {
                accessor_error(index, "primitive restart index is not allowed in glTF")
            } else {
                Ok(value)
            }
        })
        .collect()
}
