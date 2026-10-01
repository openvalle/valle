use serde::{Deserialize, Serialize};

use crate::{
    Rect,
    requirements::{ExternalTexture, FontKey, RuntimeShaderKey, SamplingMode, Scene3dKey},
};

use super::{
    FillRule, Group, LinearColor, PaintId, PathId, RoundRect, ShaderTextureBinding,
    ShaderUniformBinding,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename_all = "camelCase"))]
#[serde(rename_all = "camelCase")]
pub enum StrokeCap {
    Butt,
    Round,
    Square,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename_all = "camelCase"))]
#[serde(rename_all = "camelCase")]
pub enum StrokeJoin {
    Miter,
    Round,
    Bevel,
}

/// Complete local path-stroke geometry. Paint identity alone is insufficient for bounds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PathStroke {
    pub paint: PaintId,
    pub width: f32,
    pub dash: Vec<f32>,
    pub dash_offset: f32,
    pub cap: StrokeCap,
    pub join: StrokeJoin,
    pub miter_limit: f32,
}

impl PathStroke {
    pub const fn new(paint: PaintId, width: f32) -> Self {
        Self {
            paint,
            width,
            dash: Vec::new(),
            dash_offset: 0.0,
            cap: StrokeCap::Butt,
            join: StrokeJoin::Miter,
            miter_limit: 4.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PathNode {
    pub path: PathId,
    pub fill_rule: FillRule,
    pub fill: Option<PaintId>,
    pub stroke: Option<PathStroke>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageNode {
    pub texture: ExternalTexture,
    /// Positive normalized rect in the interpreted display-content domain.
    pub src: Rect,
    pub dst: Rect,
    pub sampling: SamplingMode,
    pub opacity: f32,
}

/// One normalized sprite region shared by the rows of an image instance batch. Its local
/// geometry is the unit rectangle; each row's affine column places that rectangle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AtlasRegion {
    pub texture: ExternalTexture,
    pub src: Rect,
    pub sampling: SamplingMode,
}

/// The local geometry drawn by every row of an instance table. The transform column places it
/// in program-local coordinates; a Rect uses the unit box and a Circle uses the unit radius.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "camelCase",
    deny_unknown_fields
)]
pub enum InstanceShape {
    Circle,
    Rect,
    RoundRect(RoundRect),
    Path(PathId),
    Image(AtlasRegion),
}

/// Parallel, fixed-width instance columns. All columns have the same row count. A zero stroke
/// width means fill only. An empty stroke color column inherits the fill color; otherwise it
/// contains one independently evaluated stroke color per row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstanceColumns {
    pub transforms: Vec<super::Affine2d>,
    pub colors: Vec<LinearColor>,
    pub stroke_colors: Vec<LinearColor>,
    /// Empty uses the shared Path style offset; otherwise one offset per row.
    pub dash_offsets: Vec<f32>,
    pub opacities: Vec<f32>,
    pub stroke_widths: Vec<f32>,
}

impl InstanceColumns {
    pub fn with_capacity(rows: usize) -> Self {
        Self {
            transforms: Vec::with_capacity(rows),
            colors: Vec::with_capacity(rows),
            stroke_colors: Vec::new(),
            dash_offsets: Vec::new(),
            opacities: Vec::with_capacity(rows),
            stroke_widths: Vec::with_capacity(rows),
        }
    }

    pub fn len(&self) -> usize {
        self.transforms.len()
    }

    pub fn is_empty(&self) -> bool {
        self.transforms.is_empty()
    }
}

/// Path paint geometry shared by every instance; width, color, and optionally dash phase are
/// carried by the parallel instance columns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename_all = "camelCase"))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstancePathStyle {
    pub fill: bool,
    pub dash: Vec<f32>,
    pub dash_offset: f32,
    pub cap: StrokeCap,
    pub join: StrokeJoin,
    pub miter_limit: f32,
}

impl Default for InstancePathStyle {
    fn default() -> Self {
        Self {
            fill: true,
            dash: Vec::new(),
            dash_offset: 0.0,
            cap: StrokeCap::Butt,
            join: StrokeJoin::Miter,
            miter_limit: 4.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstanceBatchNode {
    pub shape: InstanceShape,
    pub instances: InstanceColumns,
    pub path_style: Option<InstancePathStyle>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Glyph {
    pub id: u32,
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GlyphRun {
    pub font: FontKey,
    /// Em size in program-local units. Glyph positions are local baseline origins at this size.
    pub font_size: f32,
    pub glyphs: Vec<Glyph>,
    /// Exact ink resolved by the shaper, including variable font axes. When present,
    /// executors draw this path; glyphs remain available for source addressing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outline: Option<PathId>,
    /// Shaper-authored conservative ink bounds in program-local coordinates.
    pub bounds: Rect,
    pub paint: PaintId,
    pub stroke: Option<PathStroke>,
    /// Stable author-node key for edit/pick addressing.
    pub source_node: Option<String>,
    /// Node-local UTF-8 byte ranges parallel to `glyphs`.
    pub source_ranges: Vec<[u32; 2]>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeShaderNode {
    pub shader: RuntimeShaderKey,
    pub bounds: Rect,
    pub uniforms: Vec<ShaderUniformBinding>,
    pub textures: Vec<ShaderTextureBinding>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scene3dNode {
    pub scene: Scene3dKey,
    pub bounds: Rect,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShadowNode {
    pub shape: RoundRect,
    pub offset: [f32; 2],
    pub sigma_x: f32,
    pub sigma_y: f32,
    pub spread: f32,
    pub color: LinearColor,
    pub inset: bool,
}

/// Structured nodes only. There is intentionally no save/restore command and no unstructured payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "camelCase",
    deny_unknown_fields
)]
pub enum Node {
    Group(Group),
    Path(PathNode),
    InstanceBatch(InstanceBatchNode),
    Image(ImageNode),
    GlyphRun(GlyphRun),
    Shadow(ShadowNode),
    RuntimeShader(RuntimeShaderNode),
    Scene3d(Scene3dNode),
}
