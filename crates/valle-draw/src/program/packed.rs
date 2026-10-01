use sha2::{Digest, Sha256};
use thiserror::Error;

use super::{
    Affine2d, DrawProgram, DrawProgramError, GlyphRun, Group, ImageNode, InstanceBatchNode,
    InstanceColumns, InstanceShape, Node, NodeId, Paint, PathData, PathNode, RuntimeShaderNode,
    Scene3dNode, ShadowNode,
    validate::{
        MAX_BATCH_INSTANCES, MAX_NODES, MAX_PACKED_BYTES, MAX_PAINTS, MAX_PATHS, MAX_ROOTS,
    },
};
use crate::requirements::DrawRequirements;

pub const DRAW_PROGRAM_FORMAT_VERSION: u32 = 1;

const MAGIC: &[u8; 8] = b"VLDRAW\0\0";
const ENDIAN_MARKER: u32 = 0x0102_0304;
const HEADER_LEN: usize = 28;
const ENTRY_LEN: usize = 24;
const SECTION_COUNT: usize = 9;
const TABLE_END: usize = HEADER_LEN + ENTRY_LEN * SECTION_COUNT;
const BATCH_INSTANCE_BYTES: usize = 72;
const STROKE_COLOR_BYTES: usize = 16;
const DASH_OFFSET_BYTES: usize = 4;

const ROOTS: u16 = 1;
const NODES: u16 = 2;
const PATHS: u16 = 3;
const PAINTS: u16 = 4;
const VIEWPORT: u16 = 5;
const REQUIREMENTS: u16 = 6;
const BATCH_INSTANCES: u16 = 7;
const STROKE_COLORS: u16 = 8;
const DASH_OFFSETS: u16 = 9;
const SECTION_KINDS: [u16; SECTION_COUNT] = [
    ROOTS,
    NODES,
    BATCH_INSTANCES,
    STROKE_COLORS,
    DASH_OFFSETS,
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
struct PackedInstanceBatchNode {
    shape: InstanceShape,
    instances: PackedBatchRange,
    stroke_colors: Option<PackedBatchRange>,
    dash_offsets: Option<PackedBatchRange>,
    path_style: Option<super::InstancePathStyle>,
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
    InstanceBatch(PackedInstanceBatchNode),
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
    // node (notably large InstanceBatch tables) on every frame.
    let (nodes, batch_instances, stroke_colors, dash_offsets) = encode_nodes(&program.nodes)?;

    let sections = [
        encode_section(ROOTS, program.roots.len(), &program.roots)?,
        encode_section(NODES, nodes.len(), &nodes)?,
        encode_batch_instances(&batch_instances)?,
        encode_stroke_colors(&stroke_colors)?,
        encode_dash_offsets(&dash_offsets)?,
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

    for entry_index in [0usize, 1, 5, 6] {
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
    if entries[7].count != 1 {
        return Err(PackedDrawError::InvalidSectionTable(
            "viewport section must contain exactly one object".into(),
        ));
    }
    if entries[8].count != 1 {
        return Err(PackedDrawError::InvalidSectionTable(
            "requirements section must contain exactly one object".into(),
        ));
    }

    let roots: Vec<NodeId> = decode_section(bytes, entries[0])?;
    let packed_nodes: Vec<PackedNode> = decode_section(bytes, entries[1])?;
    let batch_instances = decode_batch_instances(bytes, entries[2])?;
    let stroke_colors = decode_stroke_colors(bytes, entries[3])?;
    let dash_offsets = decode_dash_offsets(bytes, entries[4])?;
    let nodes = decode_nodes(packed_nodes, batch_instances, stroke_colors, dash_offsets)?;
    let paths: Vec<PathData> = decode_section(bytes, entries[5])?;
    let paints: Vec<Paint> = decode_section(bytes, entries[6])?;
    let viewport: crate::Rect = decode_section(bytes, entries[7])?;
    let requirements: DrawRequirements = decode_section(bytes, entries[8])?;
    if roots.len() != entries[0].count
        || nodes.len() != entries[1].count
        || paths.len() != entries[5].count
        || paints.len() != entries[6].count
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

fn encode_nodes(
    nodes: &[Node],
) -> Result<
    (
        Vec<PackedNode>,
        InstanceColumns,
        Vec<super::LinearColor>,
        Vec<f32>,
    ),
    PackedDrawError,
> {
    let mut packed = Vec::with_capacity(nodes.len());
    let mut instances = InstanceColumns::with_capacity(0);
    let mut stroke_colors = Vec::new();
    let mut dash_offsets = Vec::new();
    for node in nodes {
        packed.push(match node {
            Node::Group(value) => PackedNode::Group(value.clone()),
            Node::Path(value) => PackedNode::Path(value.clone()),
            Node::InstanceBatch(value) => {
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
                instances
                    .transforms
                    .extend_from_slice(&value.instances.transforms);
                instances.colors.extend_from_slice(&value.instances.colors);
                instances
                    .opacities
                    .extend_from_slice(&value.instances.opacities);
                instances
                    .stroke_widths
                    .extend_from_slice(&value.instances.stroke_widths);
                let color_range = if value.instances.stroke_colors.is_empty() {
                    None
                } else {
                    let start = u32::try_from(stroke_colors.len()).map_err(|_| {
                        PackedDrawError::SectionCountBudget {
                            kind: STROKE_COLORS,
                            actual: stroke_colors.len(),
                            limit: MAX_BATCH_INSTANCES,
                        }
                    })?;
                    stroke_colors.extend_from_slice(&value.instances.stroke_colors);
                    Some(PackedBatchRange { start, count })
                };
                let dash_range = if value.instances.dash_offsets.is_empty() {
                    None
                } else {
                    let start = u32::try_from(dash_offsets.len()).map_err(|_| {
                        PackedDrawError::SectionCountBudget {
                            kind: DASH_OFFSETS,
                            actual: dash_offsets.len(),
                            limit: MAX_BATCH_INSTANCES,
                        }
                    })?;
                    dash_offsets.extend_from_slice(&value.instances.dash_offsets);
                    Some(PackedBatchRange { start, count })
                };
                PackedNode::InstanceBatch(PackedInstanceBatchNode {
                    shape: value.shape.clone(),
                    instances: PackedBatchRange { start, count },
                    stroke_colors: color_range,
                    dash_offsets: dash_range,
                    path_style: value.path_style.clone(),
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
    Ok((packed, instances, stroke_colors, dash_offsets))
}

fn decode_nodes(
    nodes: Vec<PackedNode>,
    batch_instances: InstanceColumns,
    stroke_colors: Vec<super::LinearColor>,
    dash_offsets: Vec<f32>,
) -> Result<Vec<Node>, PackedDrawError> {
    let total_instances = batch_instances.len();
    let mut consumed = 0usize;
    let mut colors_consumed = 0usize;
    let mut offsets_consumed = 0usize;
    let mut decoded = Vec::with_capacity(nodes.len());
    for node in nodes {
        decoded.push(match node {
            PackedNode::Group(value) => Node::Group(value),
            PackedNode::Path(value) => Node::Path(value),
            PackedNode::InstanceBatch(value) => {
                let start = value.instances.start as usize;
                let count = value.instances.count as usize;
                if start != consumed || count > total_instances.saturating_sub(consumed) {
                    return Err(PackedDrawError::InvalidSectionTable(
                        "InstanceBatch ranges do not canonically partition the instance table"
                            .into(),
                    ));
                }
                consumed += count;
                let row_stroke_colors = if let Some(range) = value.stroke_colors {
                    let color_start = range.start as usize;
                    if color_start != colors_consumed
                        || range.count as usize != count
                        || count > stroke_colors.len().saturating_sub(colors_consumed)
                    {
                        return Err(PackedDrawError::InvalidSectionTable(
                            "InstanceBatch stroke color ranges do not canonically partition the color table".into(),
                        ));
                    }
                    colors_consumed += count;
                    stroke_colors[color_start..colors_consumed].to_vec()
                } else {
                    Vec::new()
                };
                let row_dash_offsets = if let Some(range) = value.dash_offsets {
                    let offset_start = range.start as usize;
                    if offset_start != offsets_consumed
                        || range.count as usize != count
                        || count > dash_offsets.len().saturating_sub(offsets_consumed)
                    {
                        return Err(PackedDrawError::InvalidSectionTable(
                            "InstanceBatch dash ranges do not canonically partition the offset table".into(),
                        ));
                    }
                    offsets_consumed += count;
                    dash_offsets[offset_start..offsets_consumed].to_vec()
                } else {
                    Vec::new()
                };
                Node::InstanceBatch(InstanceBatchNode {
                    shape: value.shape,
                    path_style: value.path_style,
                    instances: InstanceColumns {
                        transforms: batch_instances.transforms[start..consumed].to_vec(),
                        colors: batch_instances.colors[start..consumed].to_vec(),
                        stroke_colors: row_stroke_colors,
                        dash_offsets: row_dash_offsets,
                        opacities: batch_instances.opacities[start..consumed].to_vec(),
                        stroke_widths: batch_instances.stroke_widths[start..consumed].to_vec(),
                    },
                })
            }
            PackedNode::Image(value) => Node::Image(value),
            PackedNode::GlyphRun(value) => Node::GlyphRun(value),
            PackedNode::Shadow(value) => Node::Shadow(value),
            PackedNode::RuntimeShader(value) => Node::RuntimeShader(value),
            PackedNode::Scene3d(value) => Node::Scene3d(value),
        });
    }
    if consumed != total_instances
        || colors_consumed != stroke_colors.len()
        || offsets_consumed != dash_offsets.len()
    {
        return Err(PackedDrawError::InvalidSectionTable(
            "unreferenced InstanceBatch instances remain".into(),
        ));
    }
    Ok(decoded)
}

fn encode_batch_instances(instances: &InstanceColumns) -> Result<EncodedSection, PackedDrawError> {
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
    for transform in &instances.transforms {
        for value in transform.0 {
            bytes.extend_from_slice(&value.to_bits().to_le_bytes());
        }
    }
    for color in &instances.colors {
        for value in [color.red, color.green, color.blue, color.alpha] {
            bytes.extend_from_slice(&value.to_bits().to_le_bytes());
        }
    }
    for opacity in &instances.opacities {
        bytes.extend_from_slice(&opacity.to_bits().to_le_bytes());
    }
    for width in &instances.stroke_widths {
        bytes.extend_from_slice(&width.to_bits().to_le_bytes());
    }
    debug_assert_eq!(bytes.len(), byte_len);
    Ok(EncodedSection {
        kind: BATCH_INSTANCES,
        count: instances.len(),
        bytes,
    })
}

fn encode_stroke_colors(colors: &[super::LinearColor]) -> Result<EncodedSection, PackedDrawError> {
    let byte_len =
        colors
            .len()
            .checked_mul(STROKE_COLOR_BYTES)
            .ok_or(PackedDrawError::TooLarge {
                actual: usize::MAX,
                limit: MAX_PACKED_BYTES,
            })?;
    check_section_budget(STROKE_COLORS, colors.len(), byte_len)?;
    let mut bytes = Vec::with_capacity(byte_len);
    for color in colors {
        for value in [color.red, color.green, color.blue, color.alpha] {
            bytes.extend_from_slice(&value.to_bits().to_le_bytes());
        }
    }
    Ok(EncodedSection {
        kind: STROKE_COLORS,
        count: colors.len(),
        bytes,
    })
}

fn encode_dash_offsets(offsets: &[f32]) -> Result<EncodedSection, PackedDrawError> {
    let byte_len =
        offsets
            .len()
            .checked_mul(DASH_OFFSET_BYTES)
            .ok_or(PackedDrawError::TooLarge {
                actual: usize::MAX,
                limit: MAX_PACKED_BYTES,
            })?;
    check_section_budget(DASH_OFFSETS, offsets.len(), byte_len)?;
    let mut bytes = Vec::with_capacity(byte_len);
    for offset in offsets {
        bytes.extend_from_slice(&offset.to_bits().to_le_bytes());
    }
    Ok(EncodedSection {
        kind: DASH_OFFSETS,
        count: offsets.len(),
        bytes,
    })
}

fn decode_dash_offsets(bytes: &[u8], entry: SectionEntry) -> Result<Vec<f32>, PackedDrawError> {
    if entry.length != entry.count * DASH_OFFSET_BYTES {
        return Err(PackedDrawError::InvalidSectionTable(
            "InstanceBatch dash offset section length does not match its count".into(),
        ));
    }
    Ok(section_bytes(bytes, entry)
        .chunks_exact(DASH_OFFSET_BYTES)
        .map(|row| {
            f32::from_bits(u32::from_le_bytes(
                row.try_into().expect("fixed dash offset"),
            ))
        })
        .collect())
}

fn decode_stroke_colors(
    bytes: &[u8],
    entry: SectionEntry,
) -> Result<Vec<super::LinearColor>, PackedDrawError> {
    if entry.length != entry.count * STROKE_COLOR_BYTES {
        return Err(PackedDrawError::InvalidSectionTable(
            "InstanceBatch stroke color section length does not match its count".into(),
        ));
    }
    let section = section_bytes(bytes, entry);
    let mut colors = Vec::with_capacity(entry.count);
    for row in section.chunks_exact(STROKE_COLOR_BYTES) {
        let channel = |offset: usize| {
            f32::from_bits(u32::from_le_bytes(
                row[offset..offset + 4]
                    .try_into()
                    .expect("fixed stroke color channel"),
            ))
        };
        colors.push(super::LinearColor {
            red: channel(0),
            green: channel(4),
            blue: channel(8),
            alpha: channel(12),
        });
    }
    Ok(colors)
}

fn decode_batch_instances(
    bytes: &[u8],
    entry: SectionEntry,
) -> Result<InstanceColumns, PackedDrawError> {
    let expected = entry
        .count
        .checked_mul(BATCH_INSTANCE_BYTES)
        .ok_or_else(|| {
            PackedDrawError::InvalidSectionTable(
                "InstanceBatch instance byte length overflows".into(),
            )
        })?;
    if entry.length != expected {
        return Err(PackedDrawError::InvalidSectionTable(format!(
            "InstanceBatch section has {} bytes for {} instances; expected {expected}",
            entry.length, entry.count
        )));
    }
    let section = section_bytes(bytes, entry);
    let mut result = InstanceColumns::with_capacity(entry.count);
    let color_base = entry.count * 48;
    let opacity_base = entry.count * 64;
    let stroke_base = entry.count * 68;
    for index in 0..entry.count {
        let f64_at = |offset: usize| {
            f64::from_bits(u64::from_le_bytes(
                section[offset..offset + 8]
                    .try_into()
                    .expect("fixed InstanceBatch f64 range"),
            ))
        };
        let f32_at = |offset: usize| {
            f32::from_bits(u32::from_le_bytes(
                section[offset..offset + 4]
                    .try_into()
                    .expect("fixed InstanceBatch f32 range"),
            ))
        };
        result.transforms.push(Affine2d(std::array::from_fn(|slot| {
            f64_at(index * 48 + slot * 8)
        })));
        result.colors.push(super::LinearColor {
            red: f32_at(color_base + index * 16),
            green: f32_at(color_base + index * 16 + 4),
            blue: f32_at(color_base + index * 16 + 8),
            alpha: f32_at(color_base + index * 16 + 12),
        });
        result.opacities.push(f32_at(opacity_base + index * 4));
        result.stroke_widths.push(f32_at(stroke_base + index * 4));
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
        STROKE_COLORS => (
            MAX_BATCH_INSTANCES,
            MAX_BATCH_INSTANCES * STROKE_COLOR_BYTES,
        ),
        DASH_OFFSETS => (MAX_BATCH_INSTANCES, MAX_BATCH_INSTANCES * DASH_OFFSET_BYTES),
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
        let instances = InstanceColumns {
            transforms: (0..count)
                .map(|index| {
                    Affine2d([
                        2.0,
                        0.0,
                        0.0,
                        2.0,
                        (index % 100) as f64 * 10.0,
                        (index / 100) as f64 * 10.0,
                    ])
                })
                .collect(),
            colors: vec![super::super::LinearColor::new(0.25, 0.5, 0.75, 1.0); count],
            stroke_colors: Vec::new(),
            dash_offsets: Vec::new(),
            opacities: vec![1.0; count],
            stroke_widths: vec![0.0; count],
        };
        let root = builder.push_node(Node::InstanceBatch(InstanceBatchNode {
            shape: InstanceShape::Circle,
            instances,
            path_style: None,
        }));
        builder.add_root(root);
        builder.finish().unwrap()
    }

    #[test]
    fn forged_requirements_are_never_authoritative() {
        let mut bytes = encode(&backdrop_fixture()).unwrap();
        let start = bytes
            .windows(b"linearClamp".len())
            .position(|window| window == b"linearClamp")
            .expect("fixture sampling mode");
        bytes[start..start + b"linearDecal".len()].copy_from_slice(b"linearDecal");

        assert!(matches!(
            decode(&bytes),
            Err(PackedDrawError::InvalidProgram(
                DrawProgramError::RequirementsMismatch
            ))
        ));
    }

    #[test]
    fn declared_arena_budget_is_checked_before_deserialization() {
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
    fn instance_batches_roundtrip_through_columnar_side_table() {
        let program = batch_fixture(10_000);
        let bytes = encode(&program).unwrap();
        let batch_entry = HEADER_LEN + ENTRY_LEN * 2;
        assert_eq!(u16_at(&bytes, batch_entry), BATCH_INSTANCES);
        assert_eq!(u32_at(&bytes, batch_entry + 4), 10_000);
        assert_eq!(u64_at(&bytes, batch_entry + 16), 720_000);
        assert_eq!(decode(&bytes).unwrap(), program);
        // Matrices, colors, opacities, and stroke widths occupy contiguous columns rather than
        // repeating JSON field names for every instance.
        assert!(
            bytes.len() < 800_000,
            "packed batch is {} bytes",
            bytes.len()
        );
    }

    #[test]
    fn affine_round_rect_and_stroke_columns_roundtrip_and_fail_closed() {
        let build = |batch: InstanceBatchNode| {
            let mut builder = DrawProgramBuilder::new(Rect::new(0.0, 0.0, 64.0, 64.0));
            let root = builder.push_node(Node::InstanceBatch(batch));
            builder.add_root(root);
            builder.finish()
        };
        let batch = InstanceBatchNode {
            shape: InstanceShape::RoundRect(super::super::RoundRect::circular(
                Rect::new(0.0, 0.0, 1.0, 1.0),
                [0.2; 4],
            )),
            instances: InstanceColumns {
                transforms: vec![Affine2d([12.0, 3.0, 2.0, 10.0, 4.0, 5.0])],
                colors: vec![super::super::LinearColor::new(0.25, 0.5, 0.75, 1.0)],
                stroke_colors: Vec::new(),
                dash_offsets: Vec::new(),
                opacities: vec![0.5],
                stroke_widths: vec![0.1],
            },
            path_style: None,
        };
        let program = build(batch.clone()).unwrap();
        assert_eq!(decode(&encode(&program).unwrap()).unwrap(), program);

        let mut distinct = batch.clone();
        distinct.instances.stroke_colors = vec![super::super::LinearColor::new(0.9, 0.1, 0.0, 1.0)];
        let program = build(distinct.clone()).unwrap();
        let bytes = encode(&program).unwrap();
        let color_entry = HEADER_LEN + ENTRY_LEN * 3;
        assert_eq!(u16_at(&bytes, color_entry), STROKE_COLORS);
        assert_eq!(u32_at(&bytes, color_entry + 4), 1);
        assert_eq!(u64_at(&bytes, color_entry + 16), STROKE_COLOR_BYTES as u64);
        assert_eq!(decode(&bytes).unwrap(), program);
        distinct
            .instances
            .stroke_colors
            .push(super::super::LinearColor::new(0.1, 0.9, 0.0, 1.0));
        assert!(build(distinct).is_err());

        let mut mismatched = batch.clone();
        mismatched.instances.stroke_widths.clear();
        assert!(build(mismatched).is_err());
        let mut singular = batch;
        singular.instances.transforms[0] = Affine2d([0.0, 0.0, 0.0, 10.0, 4.0, 5.0]);
        assert!(build(singular).is_err());
    }

    #[test]
    fn path_style_and_dash_phase_use_only_an_optional_side_table() {
        let build = |dash: Vec<f32>, offsets: Vec<f32>| {
            let mut builder = DrawProgramBuilder::new(Rect::new(0.0, 0.0, 64.0, 64.0));
            let path = builder.push_path(PathData {
                verbs: vec![
                    super::super::PathVerb::MoveTo,
                    super::super::PathVerb::LineTo,
                ],
                points: vec![[0.0, 0.0], [10.0, 0.0]],
            });
            let root = builder.push_node(Node::InstanceBatch(InstanceBatchNode {
                shape: InstanceShape::Path(path),
                instances: InstanceColumns {
                    transforms: vec![Affine2d([1.0, 0.0, 0.0, 1.0, 5.0, 5.0])],
                    colors: vec![super::super::LinearColor::new(1.0, 0.0, 0.0, 1.0)],
                    stroke_colors: Vec::new(),
                    dash_offsets: offsets,
                    opacities: vec![1.0],
                    stroke_widths: vec![1.5],
                },
                path_style: Some(super::super::InstancePathStyle {
                    fill: false,
                    dash,
                    dash_offset: 0.0,
                    cap: super::super::StrokeCap::Round,
                    join: super::super::StrokeJoin::Bevel,
                    miter_limit: 6.0,
                }),
            }));
            builder.add_root(root);
            builder.finish()
        };
        let program = build(vec![3.0, 2.0], vec![1.25]).unwrap();
        let bytes = encode(&program).unwrap();
        let entry = HEADER_LEN + ENTRY_LEN * 4;
        assert_eq!(u16_at(&bytes, entry), DASH_OFFSETS);
        assert_eq!(u32_at(&bytes, entry + 4), 1);
        assert_eq!(u64_at(&bytes, entry + 16), DASH_OFFSET_BYTES as u64);
        assert_eq!(decode(&bytes).unwrap(), program);
        assert!(build(vec![0.0, 0.0], vec![1.25]).is_err());
        assert!(build(vec![3.0, 2.0], vec![1.25, 2.0]).is_err());
    }
}
