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
#[serde(rename_all = "camelCase")]
pub enum StrokeCap {
    Butt,
    Round,
    Square,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BatchGeometry {
    Circle,
    Rect,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchInstance {
    pub position: [f64; 2],
    pub size: [f64; 2],
    pub color: LinearColor,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeometryBatchNode {
    pub geometry: BatchGeometry,
    pub instances: Vec<BatchInstance>,
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
    GeometryBatch(GeometryBatchNode),
    Image(ImageNode),
    GlyphRun(GlyphRun),
    Shadow(ShadowNode),
    RuntimeShader(RuntimeShaderNode),
    Scene3d(Scene3dNode),
}
