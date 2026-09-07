use sha2::{Digest, Sha256};
use thiserror::Error;

use super::{
    BatchGeometry, BatchInstance, DrawProgram, DrawProgramError, GeometryBatchNode, GlyphRun,
    Group, ImageNode, Node, NodeId, Paint, PathData, PathNode, RuntimeShaderNode, Scene3dNode,
    ShadowNode,
    validate::{
        MAX_BATCH_INSTANCES, MAX_NODES, MAX_PACKED_BYTES, MAX_PAINTS, MAX_PATHS, MAX_ROOTS,
    },
};
use crate::requirements::DrawRequirements;

pub const DRAW_PROGRAM_FORMAT_VERSION: u32 = 1;

const MAGIC: &[u8; 8] = b"VLDRAW\0\0";
const ENDIAN_MARKER: u32 = 0x0102_0304;
const HEADER_LEN: usize = 60;
const ENTRY_LEN: usize = 24;
const SECTION_COUNT: usize = 7;
const TABLE_END: usize = HEADER_LEN + ENTRY_LEN * SECTION_COUNT;
const BATCH_INSTANCE_BYTES: usize = 48;

const ROOTS: u16 = 1;
const NODES: u16 = 2;
const PATHS: u16 = 3;
const PAINTS: u16 = 4;
const VIEWPORT: u16 = 5;
const REQUIREMENTS: u16 = 6;
const BATCH_INSTANCES: u16 = 7;
const SECTION_KINDS: [u16; SECTION_COUNT] = [
    ROOTS,
    NODES,
    BATCH_INSTANCES,
    PATHS,
    PAINTS,
    VIEWPORT,
    REQUIREMENTS,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PackedBatchRange {
    start: u32,
    count: u32,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PackedGeometryBatchNode {
    geometry: BatchGeometry,
    instances: PackedBatchRange,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "camelCase",
    deny_unknown_fields
)]
enum PackedNode {
    Group(Group),
    Path(PathNode),
    GeometryBatch(PackedGeometryBatchNode),
    Image(ImageNode),
    GlyphRun(GlyphRun),
    Shadow(ShadowNode),
    RuntimeShader(RuntimeShaderNode),
    Scene3d(Scene3dNode),
}

#[derive(Debug, Error)]
pub enum PackedDrawError {
    #[error("packed DrawProgram is truncated: need at least {minimum} bytes, got {actual}")]
    Truncated { minimum: usize, actual: usize },
    #[error("packed DrawProgram exceeds byte budget: {actual} > {limit}")]
    TooLarge { actual: usize, limit: usize },
    #[error("packed DrawProgram has the wrong magic")]
    BadMagic,
    #[error("unsupported DrawProgram format version {found}; supported version is {supported}")]
    UnsupportedFormatVersion { found: u32, supported: u32 },
    #[error("wrong DrawProgram endianness marker {found:#010x}")]
    WrongEndianness { found: u32 },
    #[error("packed length mismatch: header declares {declared}, input has {actual}")]
    LengthMismatch { declared: u64, actual: usize },
    #[error("invalid section count {found}; expected {expected}")]
    InvalidSectionCount { found: u16, expected: u16 },
    #[error("invalid packed section table: {0}")]
    InvalidSectionTable(String),
    #[error("section {kind} has {actual} entries; limit is {limit}")]
    SectionCountBudget {
        kind: u16,
        actual: usize,
        limit: usize,
    },
    #[error("section {kind} exceeds byte budget: {actual} > {limit}")]
    SectionByteBudget {
        kind: u16,
        actual: usize,
        limit: usize,
    },
    #[error("packed DrawProgram checksum mismatch")]
    ChecksumMismatch,
    #[error("could not serialize DrawProgram section: {0}")]
    Serialize(#[source] serde_json::Error),
    #[error("could not deserialize DrawProgram section {kind}: {source}")]
    Deserialize {
        kind: u16,
        #[source]
        source: serde_json::Error,
    },
    #[error("packed DrawProgram is not the canonical byte representation")]
    NonCanonicalEncoding,
    #[error("invalid DrawProgram: {0}")]
    InvalidProgram(#[from] DrawProgramError),
}

struct EncodedSection {
    kind: u16,
    count: usize,
    bytes: Vec<u8>,
}

#[derive(Clone, Copy)]
struct SectionEntry {
    kind: u16,
    count: usize,
    offset: usize,
    length: usize,
}

pub(crate) fn encode(program: &DrawProgram) -> Result<Vec<u8>, PackedDrawError> {
    // DrawProgram is an immutable admitted arena: its fields are crate-private, and both builder
    // finish and packed decode derive/validate the complete geometry and requirements tables.
    // Encoding therefore preserves an established type invariant instead of re-walking every
    // node (notably large GeometryBatch tables) on every frame.
    let (nodes, batch_instances) = encode_nodes(&program.nodes)?;

    let sections = [
        encode_section(ROOTS, program.roots.len(), &program.roots)?,
        encode_section(NODES, nodes.len(), &nodes)?,
        encode_batch_instances(&batch_instances)?,
        encode_section(PATHS, program.paths.len(), &program.paths)?,
        encode_section(PAINTS, program.paints.len(), &program.paints)?,
        encode_section(VIEWPORT, 1, &program.viewport)?,
        encode_section(REQUIREMENTS, 1, &program.requirements)?,
    ];
    let payload_len = sections.iter().try_fold(0usize, |sum, section| {
        sum.checked_add(section.bytes.len())
            .ok_or(PackedDrawError::TooLarge {
                actual: usize::MAX,
                limit: MAX_PACKED_BYTES,
            })
    })?;
    let total_len = TABLE_END
        .checked_add(payload_len)
        .ok_or(PackedDrawError::TooLarge {
            actual: usize::MAX,
            limit: MAX_PACKED_BYTES,
        })?;
    check_wire_size(total_len)?;

    let mut wire = Vec::with_capacity(total_len);
    wire.extend_from_slice(MAGIC);
    wire.extend_from_slice(&DRAW_PROGRAM_FORMAT_VERSION.to_le_bytes());
    wire.extend_from_slice(&ENDIAN_MARKER.to_le_bytes());
    wire.extend_from_slice(&(SECTION_COUNT as u16).to_le_bytes());
    wire.extend_from_slice(&0u16.to_le_bytes());
    wire.extend_from_slice(&(total_len as u64).to_le_bytes());
    wire.extend_from_slice(&[0u8; 32]);

    let mut offset = TABLE_END;
    for section in &sections {
        check_section_budget(section.kind, section.count, section.bytes.len())?;
        wire.extend_from_slice(&section.kind.to_le_bytes());
        wire.extend_from_slice(&0u16.to_le_bytes());
        wire.extend_from_slice(&(section.count as u32).to_le_bytes());
        wire.extend_from_slice(&(offset as u64).to_le_bytes());
        wire.extend_from_slice(&(section.bytes.len() as u64).to_le_bytes());
        offset += section.bytes.len();
    }
    debug_assert_eq!(wire.len(), TABLE_END);
    for section in sections {
        wire.extend_from_slice(&section.bytes);
    }
    debug_assert_eq!(wire.len(), total_len);

    let checksum = wire_checksum(&wire);
    wire[28..60].copy_from_slice(&checksum);
    Ok(wire)
}

pub(crate) fn decode(bytes: &[u8]) -> Result<DrawProgram, PackedDrawError> {
    check_wire_size(bytes.len())?;
    if bytes.len() < HEADER_LEN {
        return Err(PackedDrawError::Truncated {
            minimum: HEADER_LEN,
            actual: bytes.len(),
        });
    }
    if &bytes[..8] != MAGIC {
        return Err(PackedDrawError::BadMagic);
    }

    let format_version = u32_at(bytes, 8);
    if format_version != DRAW_PROGRAM_FORMAT_VERSION {
        return Err(PackedDrawError::UnsupportedFormatVersion {
            found: format_version,
            supported: DRAW_PROGRAM_FORMAT_VERSION,
        });
    }
    let endian = u32_at(bytes, 12);
    if endian != ENDIAN_MARKER {
        return Err(PackedDrawError::WrongEndianness { found: endian });
    }
    let section_count = u16_at(bytes, 16);
    if section_count as usize != SECTION_COUNT {
        return Err(PackedDrawError::InvalidSectionCount {
            found: section_count,
            expected: SECTION_COUNT as u16,
        });
    }
    if u16_at(bytes, 18) != 0 {
        return Err(PackedDrawError::InvalidSectionTable(
            "header reserved bits are non-zero".into(),
        ));
    }
    let declared_len = u64_at(bytes, 20);
    if declared_len != bytes.len() as u64 {
        return Err(PackedDrawError::LengthMismatch {
            declared: declared_len,
            actual: bytes.len(),
        });
    }
    if bytes.len() < TABLE_END {
        return Err(PackedDrawError::Truncated {
            minimum: TABLE_END,
            actual: bytes.len(),
        });
    }

    let mut entries = Vec::with_capacity(SECTION_COUNT);
    let mut expected_offset = TABLE_END;
    for (index, expected_kind) in SECTION_KINDS.into_iter().enumerate() {
        let start = HEADER_LEN + index * ENTRY_LEN;
        let kind = u16_at(bytes, start);
        let flags = u16_at(bytes, start + 2);
        let count = u32_at(bytes, start + 4) as usize;
        let offset = usize_from_u64(u64_at(bytes, start + 8), "section offset")?;
        let length = usize_from_u64(u64_at(bytes, start + 16), "section length")?;
        if kind != expected_kind {
            return Err(PackedDrawError::InvalidSectionTable(format!(
                "entry {index} has kind {kind}, expected {expected_kind}"
            )));
        }
        if flags != 0 {
            return Err(PackedDrawError::InvalidSectionTable(format!(
                "section {kind} has unknown flags {flags:#06x}"
            )));
        }
        if offset != expected_offset {
            return Err(PackedDrawError::InvalidSectionTable(format!(
                "section {kind} starts at {offset}, expected {expected_offset}"
            )));
        }
        let end = offset.checked_add(length).ok_or_else(|| {
            PackedDrawError::InvalidSectionTable(format!("section {kind} range overflows"))
        })?;
        if end > bytes.len() {
            return Err(PackedDrawError::InvalidSectionTable(format!(
                "section {kind} ends outside input"
            )));
        }
        check_section_budget(kind, count, length)?;
        entries.push(SectionEntry {
            kind,
            count,
            offset,
            length,
        });
        expected_offset = end;
    }
    if expected_offset != bytes.len() {
        return Err(PackedDrawError::InvalidSectionTable(
            "trailing bytes are not owned by a section".into(),
        ));
    }

    if bytes[28..60] != wire_checksum(bytes) {
        return Err(PackedDrawError::ChecksumMismatch);
    }

    for entry_index in [0usize, 1, 3, 4] {
        let entry = entries[entry_index];
        let actual = count_top_level_array(section_bytes(bytes, entry)).map_err(|reason| {
            PackedDrawError::InvalidSectionTable(format!("section {}: {reason}", entry.kind))
        })?;
        if actual != entry.count {
            return Err(PackedDrawError::InvalidSectionTable(format!(
                "section {} declares {} entries but contains {actual}",
                entry.kind, entry.count
            )));
        }
    }
    if entries[5].count != 1 {
        return Err(PackedDrawError::InvalidSectionTable(
            "viewport section must contain exactly one object".into(),
        ));
    }
    if entries[6].count != 1 {
        return Err(PackedDrawError::InvalidSectionTable(
            "requirements section must contain exactly one object".into(),
        ));
    }

    let roots: Vec<NodeId> = decode_section(bytes, entries[0])?;
    let packed_nodes: Vec<PackedNode> = decode_section(bytes, entries[1])?;
    let batch_instances = decode_batch_instances(bytes, entries[2])?;
    let nodes = decode_nodes(packed_nodes, batch_instances)?;
    let paths: Vec<PathData> = decode_section(bytes, entries[3])?;
    let paints: Vec<Paint> = decode_section(bytes, entries[4])?;
    let viewport: crate::Rect = decode_section(bytes, entries[5])?;
    let requirements: DrawRequirements = decode_section(bytes, entries[6])?;
    if roots.len() != entries[0].count
        || nodes.len() != entries[1].count
        || paths.len() != entries[3].count
        || paints.len() != entries[4].count
    {
        return Err(PackedDrawError::InvalidSectionTable(
            "decoded section count changed during deserialization".into(),
        ));
    }

    let program = super::validate::canonicalize(
        viewport,
        roots.clone(),
        nodes.iter().cloned().map(Some).collect(),
        paths.clone(),
        paints.clone(),
    )?;
    if program.viewport != viewport
        || program.roots != roots
        || program.nodes != nodes
        || program.paths != paths
        || program.paints != paints
    {
        return Err(PackedDrawError::InvalidProgram(
            DrawProgramError::NonCanonical,
        ));
    }
    if program.requirements != requirements {
        return Err(PackedDrawError::InvalidProgram(
            DrawProgramError::RequirementsMismatch,
        ));
    }
    if encode(&program)? != bytes {
        return Err(PackedDrawError::NonCanonicalEncoding);
    }
    Ok(program)
}

pub(crate) fn content_hash(program: &DrawProgram) -> Result<[u8; 32], PackedDrawError> {
    let bytes = encode(program)?;
    Ok(Sha256::digest(bytes).into())
}

fn encode_nodes(nodes: &[Node]) -> Result<(Vec<PackedNode>, Vec<BatchInstance>), PackedDrawError> {
    let mut packed = Vec::with_capacity(nodes.len());
    let mut instances = Vec::new();
    for node in nodes {
        packed.push(match node {
            Node::Group(value) => PackedNode::Group(value.clone()),
            Node::Path(value) => PackedNode::Path(value.clone()),
            Node::GeometryBatch(value) => {
                let start = u32::try_from(instances.len()).map_err(|_| {
                    PackedDrawError::SectionCountBudget {
                        kind: BATCH_INSTANCES,
                        actual: instances.len(),
                        limit: MAX_BATCH_INSTANCES,
                    }
                })?;
                let count = u32::try_from(value.instances.len()).map_err(|_| {
                    PackedDrawError::SectionCountBudget {
                        kind: BATCH_INSTANCES,
                        actual: value.instances.len(),
                        limit: MAX_BATCH_INSTANCES,
                    }
                })?;
                instances.extend_from_slice(&value.instances);
                PackedNode::GeometryBatch(PackedGeometryBatchNode {
                    geometry: value.geometry,
                    instances: PackedBatchRange { start, count },
                })
            }
            Node::Image(value) => PackedNode::Image(value.clone()),
            Node::GlyphRun(value) => PackedNode::GlyphRun(value.clone()),
            Node::Shadow(value) => PackedNode::Shadow(value.clone()),
            Node::RuntimeShader(value) => PackedNode::RuntimeShader(value.clone()),
            Node::Scene3d(value) => PackedNode::Scene3d(value.clone()),
        });
    }
    if instances.len() > MAX_BATCH_INSTANCES {
        return Err(PackedDrawError::SectionCountBudget {
            kind: BATCH_INSTANCES,
            actual: instances.len(),
            limit: MAX_BATCH_INSTANCES,
        });
    }
    Ok((packed, instances))
}

fn decode_nodes(
    nodes: Vec<PackedNode>,
    batch_instances: Vec<BatchInstance>,
) -> Result<Vec<Node>, PackedDrawError> {
    let total_instances = batch_instances.len();
    let mut instances = batch_instances.into_iter();
    let mut consumed = 0usize;
    let mut decoded = Vec::with_capacity(nodes.len());
    for node in nodes {
        decoded.push(match node {
            PackedNode::Group(value) => Node::Group(value),
            PackedNode::Path(value) => Node::Path(value),
            PackedNode::GeometryBatch(value) => {
                let start = value.instances.start as usize;
                let count = value.instances.count as usize;
                if start != consumed || count > total_instances.saturating_sub(consumed) {
                    return Err(PackedDrawError::InvalidSectionTable(
                        "GeometryBatch ranges do not canonically partition the instance table"
                            .into(),
                    ));
                }
                let values: Vec<_> = instances.by_ref().take(count).collect();
                if values.len() != count {
                    return Err(PackedDrawError::InvalidSectionTable(
                        "GeometryBatch range exceeds the instance table".into(),
                    ));
                }
                consumed += count;
                Node::GeometryBatch(GeometryBatchNode {
                    geometry: value.geometry,
                    instances: values,
                })
            }
            PackedNode::Image(value) => Node::Image(value),
            PackedNode::GlyphRun(value) => Node::GlyphRun(value),
            PackedNode::Shadow(value) => Node::Shadow(value),
            PackedNode::RuntimeShader(value) => Node::RuntimeShader(value),
            PackedNode::Scene3d(value) => Node::Scene3d(value),
        });
    }
    if consumed != total_instances || instances.next().is_some() {
        return Err(PackedDrawError::InvalidSectionTable(
            "unreferenced GeometryBatch instances remain".into(),
        ));
    }
    Ok(decoded)
}

fn encode_batch_instances(instances: &[BatchInstance]) -> Result<EncodedSection, PackedDrawError> {
    let byte_len =
        instances
            .len()
            .checked_mul(BATCH_INSTANCE_BYTES)
            .ok_or(PackedDrawError::TooLarge {
                actual: usize::MAX,
                limit: MAX_PACKED_BYTES,
            })?;
    check_section_budget(BATCH_INSTANCES, instances.len(), byte_len)?;
    let mut bytes = Vec::with_capacity(byte_len);
    for instance in instances {
        for value in instance.position.into_iter().chain(instance.size) {
            bytes.extend_from_slice(&value.to_bits().to_le_bytes());
        }
        for value in [
            instance.color.red,
            instance.color.green,
            instance.color.blue,
            instance.color.alpha,
        ] {
            bytes.extend_from_slice(&value.to_bits().to_le_bytes());
        }
    }
    debug_assert_eq!(bytes.len(), byte_len);
    Ok(EncodedSection {
        kind: BATCH_INSTANCES,
        count: instances.len(),
        bytes,
    })
}

fn decode_batch_instances(
    bytes: &[u8],
    entry: SectionEntry,
) -> Result<Vec<BatchInstance>, PackedDrawError> {
    let expected = entry
        .count
        .checked_mul(BATCH_INSTANCE_BYTES)
        .ok_or_else(|| {
            PackedDrawError::InvalidSectionTable(
                "GeometryBatch instance byte length overflows".into(),
            )
        })?;
    if entry.length != expected {
        return Err(PackedDrawError::InvalidSectionTable(format!(
            "GeometryBatch section has {} bytes for {} instances; expected {expected}",
            entry.length, entry.count
        )));
    }
    let mut result = Vec::with_capacity(entry.count);
    for chunk in section_bytes(bytes, entry).chunks_exact(BATCH_INSTANCE_BYTES) {
        let f64_at = |offset: usize| {
            f64::from_bits(u64::from_le_bytes(
                chunk[offset..offset + 8]
                    .try_into()
                    .expect("fixed GeometryBatch f64 range"),
            ))
        };
        let f32_at = |offset: usize| {
            f32::from_bits(u32::from_le_bytes(
                chunk[offset..offset + 4]
                    .try_into()
                    .expect("fixed GeometryBatch f32 range"),
            ))
        };
        result.push(BatchInstance {
            position: [f64_at(0), f64_at(8)],
            size: [f64_at(16), f64_at(24)],
            color: super::LinearColor {
                red: f32_at(32),
                green: f32_at(36),
                blue: f32_at(40),
                alpha: f32_at(44),
            },
        });
    }
    Ok(result)
}

fn encode_section<T: serde::Serialize>(
    kind: u16,
    count: usize,
    value: &T,
) -> Result<EncodedSection, PackedDrawError> {
    let bytes = serde_json::to_vec(value).map_err(PackedDrawError::Serialize)?;
    check_section_budget(kind, count, bytes.len())?;
    Ok(EncodedSection { kind, count, bytes })
}

fn decode_section<T: serde::de::DeserializeOwned>(
    bytes: &[u8],
    entry: SectionEntry,
) -> Result<T, PackedDrawError> {
    serde_json::from_slice(section_bytes(bytes, entry)).map_err(|source| {
        PackedDrawError::Deserialize {
            kind: entry.kind,
            source,
        }
    })
}

fn section_bytes(bytes: &[u8], entry: SectionEntry) -> &[u8] {
    &bytes[entry.offset..entry.offset + entry.length]
}

fn check_wire_size(actual: usize) -> Result<(), PackedDrawError> {
    if actual > MAX_PACKED_BYTES {
        Err(PackedDrawError::TooLarge {
            actual,
            limit: MAX_PACKED_BYTES,
        })
    } else {
        Ok(())
    }
}

fn check_section_budget(kind: u16, count: usize, bytes: usize) -> Result<(), PackedDrawError> {
    let (count_limit, byte_limit) = match kind {
        ROOTS => (MAX_ROOTS, 256 * 1024),
        NODES => (MAX_NODES, 48 * 1024 * 1024),
        BATCH_INSTANCES => (
            MAX_BATCH_INSTANCES,
            MAX_BATCH_INSTANCES * BATCH_INSTANCE_BYTES,
        ),
        PATHS => (MAX_PATHS, 48 * 1024 * 1024),
        PAINTS => (MAX_PAINTS, 16 * 1024 * 1024),
        VIEWPORT => (1, 1024),
        REQUIREMENTS => (1, 8 * 1024 * 1024),
        _ => {
            return Err(PackedDrawError::InvalidSectionTable(format!(
                "unknown section kind {kind}"
            )));
        }
    };
    if count > count_limit {
        return Err(PackedDrawError::SectionCountBudget {
            kind,
            actual: count,
            limit: count_limit,
        });
    }
    if bytes > byte_limit {
        return Err(PackedDrawError::SectionByteBudget {
            kind,
            actual: bytes,
            limit: byte_limit,
        });
    }
    Ok(())
}

/// Counts an array without allocating its elements. This lets section count budgets run before
/// serde creates the arena vectors. JSON syntax itself is still authoritatively checked by serde.
fn count_top_level_array(bytes: &[u8]) -> Result<usize, &'static str> {
    let mut index = 0usize;
    skip_ws(bytes, &mut index);
    if bytes.get(index) != Some(&b'[') {
        return Err("payload is not an array");
    }
    index += 1;
    skip_ws(bytes, &mut index);
    if bytes.get(index) == Some(&b']') {
        index += 1;
        skip_ws(bytes, &mut index);
        return if index == bytes.len() {
            Ok(0)
        } else {
            Err("trailing bytes after array")
        };
    }

    let mut count = 1usize;
    let mut depth = 1usize;
    let mut in_string = false;
    let mut escaped = false;
    while index < bytes.len() {
        let byte = bytes[index];
        index += 1;
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'[' | b'{' => depth = depth.checked_add(1).ok_or("nesting overflow")?,
            b'}' => {
                depth = depth.checked_sub(1).ok_or("unbalanced object")?;
            }
            b']' => {
                depth = depth.checked_sub(1).ok_or("unbalanced array")?;
                if depth == 0 {
                    skip_ws(bytes, &mut index);
                    return if index == bytes.len() {
                        Ok(count)
                    } else {
                        Err("trailing bytes after array")
                    };
                }
            }
            b',' if depth == 1 => {
                count = count.checked_add(1).ok_or("entry count overflow")?;
            }
            _ => {}
        }
    }
    Err("unterminated array")
}

fn skip_ws(bytes: &[u8], index: &mut usize) {
    while matches!(bytes.get(*index), Some(b' ' | b'\n' | b'\r' | b'\t')) {
        *index += 1;
    }
}

fn wire_checksum(bytes: &[u8]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(&bytes[..28]);
    digest.update([0u8; 32]);
    digest.update(&bytes[60..]);
    digest.finalize().into()
}

fn usize_from_u64(value: u64, field: &str) -> Result<usize, PackedDrawError> {
    usize::try_from(value).map_err(|_| {
        PackedDrawError::InvalidSectionTable(format!("{field} does not fit this platform"))
    })
}

fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(
        bytes[offset..offset + 2]
            .try_into()
            .expect("validated header"),
    )
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("validated header"),
    )
}

fn u64_at(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(
        bytes[offset..offset + 8]
            .try_into()
            .expect("validated header"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Rect,
        program::{BackdropRead, BackdropScope, DrawProgramBuilder, Group},
        requirements::Insets,
    };

    fn backdrop_fixture() -> DrawProgram {
        let mut builder = DrawProgramBuilder::new(Rect::new(0.0, 0.0, 32.0, 18.0));
        let mut group = Group::plain(Vec::new());
        group.backdrop = Some(BackdropRead {
            scope: BackdropScope::Current,
            bounds: Rect::new(0.0, 0.0, 32.0, 18.0),
            footprint: Insets::uniform(4.0),
            sampling: crate::requirements::SamplingMode::LinearClamp,
            filters: Vec::new(),
        });
        let root = builder.push_node(Node::Group(group));
        builder.add_root(root);
        builder.finish().unwrap()
    }

    fn batch_fixture(count: usize) -> DrawProgram {
        let mut builder = DrawProgramBuilder::new(Rect::new(0.0, 0.0, 2048.0, 2048.0));
        let instances = (0..count)
            .map(|index| BatchInstance {
                position: [(index % 100) as f64 * 10.0, (index / 100) as f64 * 10.0],
                size: [2.0, 2.0],
                color: super::super::LinearColor::new(0.25, 0.5, 0.75, 1.0),
            })
            .collect();
        let root = builder.push_node(Node::GeometryBatch(GeometryBatchNode {
            geometry: BatchGeometry::Circle,
            instances,
        }));
        builder.add_root(root);
        builder.finish().unwrap()
    }

    #[test]
    fn a_valid_checksum_cannot_make_forged_requirements_authoritative() {
        let mut bytes = encode(&backdrop_fixture()).unwrap();
        let start = bytes
            .windows(b"linearClamp".len())
            .position(|window| window == b"linearClamp")
            .expect("fixture sampling mode");
        bytes[start..start + b"linearDecal".len()].copy_from_slice(b"linearDecal");
        let checksum = wire_checksum(&bytes);
        bytes[28..60].copy_from_slice(&checksum);

        assert!(matches!(
            decode(&bytes),
            Err(PackedDrawError::InvalidProgram(
                DrawProgramError::RequirementsMismatch
            ))
        ));
    }

    #[test]
    fn declared_arena_budget_is_checked_before_checksum_or_deserialization() {
        let mut bytes = encode(&backdrop_fixture()).unwrap();
        let node_entry = HEADER_LEN + ENTRY_LEN;
        bytes[node_entry + 4..node_entry + 8]
            .copy_from_slice(&((MAX_NODES as u32) + 1).to_le_bytes());
        assert!(matches!(
            decode(&bytes),
            Err(PackedDrawError::SectionCountBudget {
                kind: NODES,
                actual,
                limit: MAX_NODES,
            }) if actual == MAX_NODES + 1
        ));
    }

    #[test]
    fn geometry_batches_roundtrip_through_the_fixed_width_side_table() {
        let program = batch_fixture(10_000);
        let bytes = encode(&program).unwrap();
        let batch_entry = HEADER_LEN + ENTRY_LEN * 2;
        assert_eq!(u16_at(&bytes, batch_entry), BATCH_INSTANCES);
        assert_eq!(u32_at(&bytes, batch_entry + 4), 10_000);
        assert_eq!(u64_at(&bytes, batch_entry + 16), 480_000);
        assert_eq!(decode(&bytes).unwrap(), program);
        // The payload stays near the 48-byte physical instance layout instead of expanding each
        // instance into a generic JSON object with repeated field names.
        assert!(
            bytes.len() < 500_000,
            "packed batch is {} bytes",
            bytes.len()
        );
    }
}
