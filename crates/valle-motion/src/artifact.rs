use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::geometry::{GeometryEvalPolicy, PathData};
use crate::value::MotionValue;

use super::controls::ControlsSchema;
use super::expr::{
    ContextInput, Expr, ExprId, ExprType, TemplatePart, geometry_eval_policy, validate_exprs,
};
use super::tailwind::{TAILWIND_CATALOG, TailwindClassError, validate_tailwind_class};
use crate::ContentDigest;
use valle_draw::program::recording::{FillRule, MaskMode, SpreadMode};
use valle_draw::{Point, Rgba};

/// Exact decoder format for the complete canonical Motion artifact.
///
/// The artifact and its scene IR are one serialized boundary, so they share
/// one discriminator instead of advancing two coupled counters.
pub const ARTIFACT_FORMAT_VERSION: u32 = 1;
pub const BASE_CAPABILITIES: &[&str] = &[
    "backdrop-filter",
    "blend",
    "box",
    "cue-signals",
    "dynamic-path",
    "filter",
    "geometry-path",
    "glow",
    "gradient-paint",
    "clip-mask",
    "group",
    "image",
    "path",
    "svg",
    TAILWIND_CATALOG,
    "text",
    // Declare text-on-path separately for precise capability admission.
    "text-on-path",
    "text-per-unit",
    // Text split supports character, word, and line units.
    "text-split",
];

/// Scene camera capability, declared only when the artifact uses camera binding.
pub const CAMERA_CAPABILITY: &str = "scene-camera";

/// Viewport-context capability, declared only when expressions read `ctx.viewport.*`.
pub const VIEWPORT_CAPABILITY: &str = "viewport-context";

/// Optional capabilities are declared only when used.
///
/// Validation requires every base capability and rejects names outside the base and optional
/// catalogs, so the artifact states its exact runtime requirements and unknown features fail closed.
/// Video-node capability, declared only when the artifact contains a video leaf.
pub const VIDEO_CAPABILITY: &str = "video-node";

/// Project/Timeline font asset bound through a Motion control.
pub const FONT_ASSET_CAPABILITY: &str = "font-asset";
pub const MOTION_MATH_CAPABILITY: &str = "motion-math";
pub const NUMBER_FORMAT_CAPABILITY: &str = "number-format";
/// `<Text>` with a fixed set of inline `<Span>` runs, lowered to the existing inline text path.
pub const RICH_TEXT_CAPABILITY: &str = "rich-text";
/// Fixed inline image boxes participating in the same deterministic line layout as rich text.
pub const RICH_TEXT_INLINE_IMAGE_CAPABILITY: &str = "rich-text-inline-image";
/// Fixed-topology FLIP supports two-axis CSS scale under an explicit capability.
pub const FLIP_CAPABILITY: &str = "flip-layout";
/// General two-axis scale; `flip-layout` identifies the FLIP state plan separately.
pub const TRANSFORM_SCALE2D_CAPABILITY: &str = "transform-scale2d";
/// One Scene leaf and one Display command for up to 100k homogeneous primitives.
pub const GEOMETRY_BATCH_CAPABILITY: &str = "geometry-batch";
/// Fixed-count GeometryBatch fields evaluated from one frame progress expression plus a compact
/// per-index stagger. The resolved ProgramRecording stays the existing BatchInstance side table.
pub const GEOMETRY_BATCH_FIELD_CAPABILITY: &str = "geometry-batch-field";
/// Closed-form, seeded particle positions evaluated directly at an arbitrary frame.
pub const PARTICLE_FIELD_CAPABILITY: &str = "particle-field";
/// Node-local displacement and velocity-aware blur carried by ProgramRecording filters.
pub const NODE_ADVANCED_FILTER_CAPABILITY: &str = "node-advanced-filter";
/// A deterministic displacement filter applied to the pixels behind a node while keeping the
/// node's own foreground content sharp. It lowers to the existing backdrop ProgramRecording group.
pub const BACKDROP_DISPLACEMENT_CAPABILITY: &str = "backdrop-displacement";
/// `motion-displacement-seed` may be a frame-evaluated Number expression.
pub const DISPLACEMENT_SEED_EXPR_CAPABILITY: &str = "displacement-seed-expr";
/// Deterministic CSS 3D subset: ordered transform functions, authored transform origin,
/// parent perspective, `preserve-3d`, and back-face culling. The renderer lowers each
/// flattened plane to the existing 3x3 homography display primitive.
pub const CSS_3D_TRANSFORM_CAPABILITY: &str = "css-3d-transform";
/// Self-relative percentage translation inside an ordered CSS transform list.
/// The compiler keeps the percentage as a scalar expression and layout resolves
/// it against the transformed node's own border-box width/height.
pub const CSS_TRANSFORM_PERCENT_CAPABILITY: &str = "css-transform-percent";
/// CSS `perspective-origin` on a perspective owner. The two typed lengths are
/// resolved against that owner's border box before child planes are projected.
pub const CSS_3D_PERSPECTIVE_ORIGIN_CAPABILITY: &str = "css-3d-perspective-origin";
/// A controlled, content-addressed shader package applied to one component-local child surface.
pub const SHADER_LAYER_CAPABILITY: &str = "shader-layer";
/// A fixed Scene3D contract rendered into one generated 2D texture.
pub const SCENE3D_LAYER_CAPABILITY: &str = "scene3d-layer";
/// Prepare-time RaTeX `MathFormula` leaf, declared only when present.
pub const MATH_FORMULA_CAPABILITY: &str = "motion-math-formula";

pub const OPTIONAL_CAPABILITIES: &[&str] = &[
    CAMERA_CAPABILITY,
    FLIP_CAPABILITY,
    TRANSFORM_SCALE2D_CAPABILITY,
    GEOMETRY_BATCH_CAPABILITY,
    GEOMETRY_BATCH_FIELD_CAPABILITY,
    FONT_ASSET_CAPABILITY,
    MOTION_MATH_CAPABILITY,
    NODE_ADVANCED_FILTER_CAPABILITY,
    BACKDROP_DISPLACEMENT_CAPABILITY,
    DISPLACEMENT_SEED_EXPR_CAPABILITY,
    CSS_3D_TRANSFORM_CAPABILITY,
    CSS_TRANSFORM_PERCENT_CAPABILITY,
    CSS_3D_PERSPECTIVE_ORIGIN_CAPABILITY,
    NUMBER_FORMAT_CAPABILITY,
    RICH_TEXT_CAPABILITY,
    RICH_TEXT_INLINE_IMAGE_CAPABILITY,
    PARTICLE_FIELD_CAPABILITY,
    SHADER_LAYER_CAPABILITY,
    SCENE3D_LAYER_CAPABILITY,
    VIEWPORT_CAPABILITY,
    VIDEO_CAPABILITY,
    MATH_FORMULA_CAPABILITY,
    crate::glass::MOTION_GLASS_CAPABILITY,
];

/// Stable logical family used by layout for a bound font resource.
///
/// Author control names are per instance while the font registry is job-level. Keying the
/// internal family by bytes prevents two clips with the same control name from sharing the wrong
/// embedded family.
pub fn font_family_alias(hash: &ContentDigest) -> String {
    format!("valle-font-{}", hash.as_hex())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilitySet {
    /// The complete canonical authority: non-empty, sorted, unique names.
    ///
    /// A digest of this same list would only duplicate derivable state.
    pub names: Vec<String>,
}

impl CapabilitySet {
    pub fn new(names: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let mut names: Vec<String> = names.into_iter().map(Into::into).collect();
        names.sort();
        names.dedup();
        CapabilitySet { names }
    }

    pub fn base() -> Self {
        Self::new(BASE_CAPABILITIES.iter().copied())
    }

    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.names.is_empty()
            || self.names.windows(2).any(|pair| pair[0] >= pair[1])
            || self.names.iter().any(String::is_empty)
        {
            return Err(ValidationError::new(
                "/capabilitySet/names",
                "capabilities must be non-empty, sorted, and unique",
            ));
        }
        Ok(())
    }
}

/// Text animation units are derived after shaping the full text, preserving kerning, ligatures, and
/// wrapping. Per-unit animation changes drawing parameters rather than splitting Text nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum TextSplit {
    /// One unit per Unicode scalar, counted before shaping. A cluster spanning multiple units
    /// belongs to the unit containing its start; other units may have no glyphs.
    Char,
    /// One unit per maximal non-whitespace run; whitespace does not form or belong to a unit.
    Word,
    /// One unit per source newline segment, independent of viewport wrapping. This keeps unit count
    /// and stagger timing stable across output resolutions.
    Line,
}

/// Per-unit text animation binding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PerUnit {
    pub split: TextSplit,
    pub style: UnitStyle,
}

/// Closed set of per-unit paint and transform properties, each handled during emission.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UnitStyle {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<NumberValue>,
    /// Translation relative to the unit's own position, in pixels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub translate: Option<PointValue>,
    /// Independent x/y scaling around the unit center.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<PointValue>,
    /// Rotation around the unit center, in degrees.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotate: Option<NumberValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<ColorValue>,
}

impl UnitStyle {
    pub fn is_empty(&self) -> bool {
        *self == UnitStyle::default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NodeId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChildRange {
    pub start: u32,
    pub end: u32,
}

impl ChildRange {
    pub const EMPTY: ChildRange = ChildRange { start: 0, end: 0 };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CoordinateSpace {
    Local,
    World,
    Screen,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum TextValue {
    Static { value: String },
    Expr { expr: ExprId },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(tag = "kind"))]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind",
    deny_unknown_fields
)]
pub enum PathValue {
    Static {
        value: PathData,
    },
    Expr {
        expr: ExprId,
        policy: GeometryEvalPolicy,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind",
    deny_unknown_fields
)]
pub enum NumberValue {
    Static { value: f64 },
    Expr { expr: ExprId },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind",
    deny_unknown_fields
)]
pub enum PointValue {
    Static { value: Point },
    Expr { expr: ExprId },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind",
    deny_unknown_fields
)]
pub enum RectValue {
    Static { value: valle_draw::Rect },
    Expr { expr: ExprId },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind",
    deny_unknown_fields
)]
pub enum ColorValue {
    Static {
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        value: Rgba,
    },
    Expr {
        expr: ExprId,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind",
    deny_unknown_fields
)]
pub enum BoolValue {
    Static { value: bool },
    Expr { expr: ExprId },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShaderProgramRef {
    pub uri: String,
    pub content_hash: ContentDigest,
    pub abi_hash: ContentDigest,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind",
    deny_unknown_fields
)]
pub enum ShaderUniformValue {
    Float { value: NumberValue },
    Float2 { value: PointValue },
    Color { value: ColorValue },
    Bool { value: BoolValue },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShaderUniformBinding {
    pub name: String,
    pub value: ShaderUniformValue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShaderTextureInput {
    pub name: String,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GradientStopValue {
    pub offset: NumberValue,
    pub color: ColorValue,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind",
    deny_unknown_fields
)]
pub enum PaintValue {
    Solid {
        color: ColorValue,
    },
    Linear {
        start: PointValue,
        end: PointValue,
        stops: Vec<GradientStopValue>,
        spread: SpreadMode,
    },
    Radial {
        center: PointValue,
        radius: NumberValue,
        stops: Vec<GradientStopValue>,
        spread: SpreadMode,
    },
    Conic {
        center: PointValue,
        start_angle: NumberValue,
        stops: Vec<GradientStopValue>,
        spread: SpreadMode,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind",
    deny_unknown_fields
)]
pub enum MaskValue {
    Paint {
        paint: PaintValue,
    },
    Image {
        source: String,
    },
    /// An authored `<MaskSource>` subtree. The referenced node is the final direct child of the
    /// mask and is composited with destination-in after the ordinary mask contents.
    Subtree {
        source: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum ArrowKind {
    Triangle,
    Open,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArrowSpec {
    pub kind: ArrowKind,
    pub size: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PathStroke {
    pub paint: PaintValue,
    pub width: NumberValue,
    pub dash: Option<Vec<f64>>,
    /// SVG `stroke-dashoffset`, in authored path units. It is frame-evaluated but history-free.
    pub dash_offset: NumberValue,
    pub cap: valle_draw::Cap,
    pub join: valle_draw::Join,
    pub miter_limit: f64,
}

pub const MAX_GEOMETRY_BATCH_INSTANCES_PER_NODE: usize = 100_000;
pub const MAX_GEOMETRY_BATCH_INSTANCES_PER_DISPLAY: usize =
    valle_draw::program::recording::MAX_BATCH_INSTANCES_PER_RECORDING;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum GeometryBatchGeometry {
    Circle,
    Rect,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ParticleSpec {
    pub seed: u64,
    pub count: u32,
    pub emitter: valle_draw::Rect,
    pub birth_interval: f64,
    pub lifetime: f64,
    pub velocity_x: [f64; 2],
    pub velocity_y: [f64; 2],
    pub gravity: valle_draw::Point,
    #[serde(default)]
    pub looping: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum BatchPositions {
    Static { values: Vec<valle_draw::Point> },
    Particles { frame: ExprId, spec: ParticleSpec },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BatchPointField {
    pub to: Vec<valle_draw::Point>,
    pub progress: ExprId,
    /// Normalized phase offset added per instance. Instance `i` starts at `i * stagger`, then
    /// remaps the remaining progress interval so every instance reaches `to` at progress 1.
    pub stagger: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BatchColorField {
    pub to: Vec<Rgba>,
    pub progress: ExprId,
    pub stagger: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BatchNumberField {
    pub to: Vec<f64>,
    pub progress: ExprId,
    pub stagger: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GeometryBatchSpec {
    pub geometry: GeometryBatchGeometry,
    pub positions: BatchPositions,
    /// One value broadcasts. Particle batches additionally accept two endpoints as a life curve.
    pub sizes: Vec<valle_draw::Point>,
    /// One value broadcasts. Particle batches additionally accept two endpoints as a life curve.
    pub fills: Vec<Rgba>,
    /// One value broadcasts. Particle batches additionally accept two endpoints as a life curve.
    #[serde(default)]
    pub opacities: Vec<f64>,
    #[serde(default)]
    pub semantic_keys: Vec<String>,
    /// Optional fixed-count frame fields. Their `from` values are the corresponding base arrays
    /// above, so the artifact does not duplicate the large prepare-time side tables.
    #[serde(default)]
    pub position_field: Option<BatchPointField>,
    #[serde(default)]
    pub size_field: Option<BatchPointField>,
    #[serde(default)]
    pub fill_field: Option<BatchColorField>,
    #[serde(default)]
    pub opacity_field: Option<BatchNumberField>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum NodeKind {
    Group,
    Box,
    Clip {
        path: PathValue,
        fill_rule: FillRule,
    },
    Mask {
        source: MaskValue,
        mode: MaskMode,
        rect: RectValue,
    },
    Text {
        text: TextValue,
        /// Optional per-unit animation; None treats the text as one block.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        per_unit: Option<Box<PerUnit>>,
        /// Optional text path in node-local coordinates. Per-unit animation composes with the glyph
        /// positions along the path.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<PathValue>,
    },
    Path {
        d: PathValue,
        fill: Option<PaintValue>,
        stroke: Option<Box<PathStroke>>,
        trim_start: NumberValue,
        trim_end: NumberValue,
        arrow_start: Option<ArrowSpec>,
        arrow_end: Option<ArrowSpec>,
    },
    GeometryBatch {
        batch: GeometryBatchSpec,
    },
    Image {
        source: String,
    },
    ShaderLayer {
        program: ShaderProgramRef,
        uniforms: Vec<ShaderUniformBinding>,
        inputs: Vec<ShaderTextureInput>,
    },
    Scene3D {
        scene: crate::scene3d::Scene3DSpec,
        frame: Scene3DFrameBinding,
    },
    /// Video node bound to an asset control. Sample time is max(sourceStart + localSeconds * speed,
    /// 0); timing parameters may be per-frame expressions.
    Video {
        source: String,
        source_start: NumberValue,
        speed: NumberValue,
    },
    /// Prepare-only LaTeX formula. Artifact stores source/options, never RaTeX AST.
    MathFormula {
        latex: String,
        display: bool,
        aria_label: Option<String>,
    },
    /// Independent or field-member refractive surface. Admitted in production (G4.7);
    /// consumed through the engine kernel route (sample_tracks → typed execution program →
    /// F32/F16 reference → Native Skia / WASM kernel); product layout/emit fail-closes
    /// instead of silently dropping it.
    Glass(crate::glass::GlassNode),
    /// Shared material domain. Not a layout box.
    GlassField(crate::glass::GlassFieldNode),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Scene3DFrameBinding {
    pub camera: Scene3DCameraBinding,
    pub meshes: Vec<Scene3DMeshBinding>,
    pub light_intensities: Vec<NumberValue>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Scene3DCameraBinding {
    pub orbit_yaw_degrees: NumberValue,
    pub orbit_pitch_degrees: NumberValue,
    pub distance: NumberValue,
    pub fov_y_degrees: NumberValue,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Scene3DMeshBinding {
    pub key: String,
    pub translation_x: NumberValue,
    pub translation_y: NumberValue,
    pub translation_z: NumberValue,
    pub rotation_x_degrees: NumberValue,
    pub rotation_y_degrees: NumberValue,
    pub rotation_z_degrees: NumberValue,
    pub scale_x: NumberValue,
    pub scale_y: NumberValue,
    pub scale_z: NumberValue,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum StyleValue {
    Static { value: MotionValue },
    Expr { expr: ExprId },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StyleBinding {
    pub property: String,
    pub value: StyleValue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SemanticMeta {
    pub role: Option<String>,
    pub label: Option<String>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SceneNode {
    pub key: String,
    pub kind: NodeKind,
    pub space: Option<CoordinateSpace>,
    pub class_names: Vec<String>,
    pub styles: Vec<StyleBinding>,
    pub visibility: Option<ExprId>,
    pub children: ChildRange,
    pub semantic: Option<SemanticMeta>,
}

impl SceneNode {
    /// List all expression references outside per-unit styles for admission checks. Exhaustive node
    /// matching ensures new node types cannot bypass unit-scope validation.
    pub fn expr_refs_outside_per_unit(&self) -> Vec<(String, ExprId)> {
        let mut refs = Vec::new();
        let number = |path: String, value: &NumberValue, refs: &mut Vec<(String, ExprId)>| {
            if let NumberValue::Expr { expr } = value {
                refs.push((path, *expr));
            }
        };
        fn point(path: String, value: &PointValue, refs: &mut Vec<(String, ExprId)>) {
            if let PointValue::Expr { expr } = value {
                refs.push((path, *expr));
            }
        }
        fn color(path: String, value: &ColorValue, refs: &mut Vec<(String, ExprId)>) {
            if let ColorValue::Expr { expr } = value {
                refs.push((path, *expr));
            }
        }
        fn rect(path: String, value: &RectValue, refs: &mut Vec<(String, ExprId)>) {
            if let RectValue::Expr { expr } = value {
                refs.push((path, *expr));
            }
        }
        fn path_value(path: String, value: &PathValue, refs: &mut Vec<(String, ExprId)>) {
            if let PathValue::Expr { expr, .. } = value {
                refs.push((path, *expr));
            }
        }
        fn number_at(path: String, value: &NumberValue, refs: &mut Vec<(String, ExprId)>) {
            if let NumberValue::Expr { expr } = value {
                refs.push((path, *expr));
            }
        }
        fn paint(path: String, value: &PaintValue, refs: &mut Vec<(String, ExprId)>) {
            let stops =
                |path: &str, stops: &[GradientStopValue], refs: &mut Vec<(String, ExprId)>| {
                    for (at, stop) in stops.iter().enumerate() {
                        number_at(format!("{path}/stops/{at}/offset"), &stop.offset, refs);
                        color(format!("{path}/stops/{at}/color"), &stop.color, refs);
                    }
                };
            match value {
                PaintValue::Solid { color: value } => color(format!("{path}/color"), value, refs),
                PaintValue::Linear {
                    start,
                    end,
                    stops: items,
                    ..
                } => {
                    point(format!("{path}/start"), start, refs);
                    point(format!("{path}/end"), end, refs);
                    stops(&path, items, refs);
                }
                PaintValue::Radial {
                    center,
                    radius,
                    stops: items,
                    ..
                } => {
                    point(format!("{path}/center"), center, refs);
                    number_at(format!("{path}/radius"), radius, refs);
                    stops(&path, items, refs);
                }
                PaintValue::Conic {
                    center,
                    start_angle,
                    stops: items,
                    ..
                } => {
                    point(format!("{path}/center"), center, refs);
                    number_at(format!("{path}/startAngle"), start_angle, refs);
                    stops(&path, items, refs);
                }
            }
        }

        if let Some(expr) = self.visibility {
            refs.push(("/visibility".into(), expr));
        }
        for (at, style) in self.styles.iter().enumerate() {
            if let StyleValue::Expr { expr } = &style.value {
                refs.push((format!("/styles/{at}/value"), *expr));
            }
        }
        match &self.kind {
            NodeKind::Group
            | NodeKind::Box
            | NodeKind::Image { .. }
            | NodeKind::MathFormula { .. } => {}
            NodeKind::GlassField(field) => {
                number(
                    "/kind/material/clarity".into(),
                    &field.material.clarity,
                    &mut refs,
                );
                number(
                    "/kind/material/depth".into(),
                    &field.material.depth,
                    &mut refs,
                );
                color(
                    "/kind/material/tint".into(),
                    &field.material.tint,
                    &mut refs,
                );
                number(
                    "/kind/motion/intensity".into(),
                    &field.motion.intensity,
                    &mut refs,
                );
                point(
                    "/kind/environment/light/direction".into(),
                    &field.environment.light.direction,
                    &mut refs,
                );
                number(
                    "/kind/environment/light/elevation".into(),
                    &field.environment.light.elevation,
                    &mut refs,
                );
                number(
                    "/kind/environment/light/intensity".into(),
                    &field.environment.light.intensity,
                    &mut refs,
                );
            }
            NodeKind::Glass(glass) => {
                number("/kind/presence".into(), &glass.presence, &mut refs);
                number(
                    "/kind/motion/intensity".into(),
                    &glass.motion.intensity,
                    &mut refs,
                );
                number(
                    "/kind/foreground/protection".into(),
                    &glass.foreground.protection,
                    &mut refs,
                );
                if let Some(material) = &glass.material {
                    number(
                        "/kind/material/clarity".into(),
                        &material.clarity,
                        &mut refs,
                    );
                    number("/kind/material/depth".into(), &material.depth, &mut refs);
                    color("/kind/material/tint".into(), &material.tint, &mut refs);
                }
                if let Some(PointValue::Expr { expr }) = &glass.motion.drive.translation {
                    refs.push(("/kind/motion/drive/translation".into(), *expr));
                }
                if let Some(value) = &glass.motion.drive.pressure {
                    number("/kind/motion/drive/pressure".into(), value, &mut refs);
                }
                if let Some(value) = &glass.motion.drive.twist {
                    number("/kind/motion/drive/twist".into(), value, &mut refs);
                }
                match &glass.shape {
                    crate::glass::GlassShapeBinding::ContinuousRect { radius } => {
                        number("/kind/shape/radius".into(), radius, &mut refs);
                    }
                    crate::glass::GlassShapeBinding::Path {
                        path,
                        reveal_origin,
                    } => {
                        path_value("/kind/shape/path".into(), path, &mut refs);
                        if let Some(origin) = reveal_origin {
                            point("/kind/shape/revealOrigin".into(), origin, &mut refs);
                        }
                    }
                    crate::glass::GlassShapeBinding::Capsule
                    | crate::glass::GlassShapeBinding::Circle => {}
                }
            }
            NodeKind::Text {
                text,
                per_unit: _,
                path,
            } => {
                // Per-unit styles are the only permitted consumers of unit-dependent expressions.
                if let TextValue::Expr { expr } = text {
                    refs.push(("/kind/text".into(), *expr));
                }
                if let Some(path) = path {
                    path_value("/kind/path".into(), path, &mut refs);
                }
            }
            NodeKind::Clip { path, .. } => path_value("/kind/path".into(), path, &mut refs),
            NodeKind::Mask {
                source, rect: r, ..
            } => {
                if let MaskValue::Paint { paint: value } = source {
                    paint("/kind/source/paint".into(), value, &mut refs);
                }
                rect("/kind/rect".into(), r, &mut refs);
            }
            NodeKind::Path {
                d,
                fill,
                stroke,
                trim_start,
                trim_end,
                arrow_start: _,
                arrow_end: _,
            } => {
                path_value("/kind/d".into(), d, &mut refs);
                if let Some(value) = fill {
                    paint("/kind/fill".into(), value, &mut refs);
                }
                if let Some(value) = stroke {
                    paint("/kind/stroke/paint".into(), &value.paint, &mut refs);
                    number(
                        "/kind/stroke/dashOffset".into(),
                        &value.dash_offset,
                        &mut refs,
                    );
                }
                number("/kind/trimStart".into(), trim_start, &mut refs);
                number("/kind/trimEnd".into(), trim_end, &mut refs);
            }
            NodeKind::GeometryBatch { batch } => {
                if let BatchPositions::Particles { frame, .. } = &batch.positions {
                    refs.push(("/kind/batch/positions/frame".into(), *frame));
                }
                for (path, expr) in [
                    (
                        "/kind/batch/positionField/progress",
                        batch.position_field.as_ref().map(|field| field.progress),
                    ),
                    (
                        "/kind/batch/sizeField/progress",
                        batch.size_field.as_ref().map(|field| field.progress),
                    ),
                    (
                        "/kind/batch/fillField/progress",
                        batch.fill_field.as_ref().map(|field| field.progress),
                    ),
                    (
                        "/kind/batch/opacityField/progress",
                        batch.opacity_field.as_ref().map(|field| field.progress),
                    ),
                ] {
                    if let Some(expr) = expr {
                        refs.push((path.into(), expr));
                    }
                }
            }
            NodeKind::ShaderLayer { uniforms, .. } => {
                for (at, uniform) in uniforms.iter().enumerate() {
                    let path = format!("/kind/uniforms/{at}/value");
                    match &uniform.value {
                        ShaderUniformValue::Float { value } => number(path, value, &mut refs),
                        ShaderUniformValue::Float2 { value } => point(path, value, &mut refs),
                        ShaderUniformValue::Color { value } => color(path, value, &mut refs),
                        ShaderUniformValue::Bool {
                            value: BoolValue::Expr { expr },
                        } => refs.push((path, *expr)),
                        ShaderUniformValue::Bool {
                            value: BoolValue::Static { .. },
                        } => {}
                    }
                }
            }
            NodeKind::Scene3D { frame, .. } => {
                for (name, value) in [
                    ("orbitYawDegrees", &frame.camera.orbit_yaw_degrees),
                    ("orbitPitchDegrees", &frame.camera.orbit_pitch_degrees),
                    ("distance", &frame.camera.distance),
                    ("fovYDegrees", &frame.camera.fov_y_degrees),
                ] {
                    number(format!("/kind/frame/camera/{name}"), value, &mut refs);
                }
                for (at, mesh) in frame.meshes.iter().enumerate() {
                    for (name, value) in [
                        ("translationX", &mesh.translation_x),
                        ("translationY", &mesh.translation_y),
                        ("translationZ", &mesh.translation_z),
                        ("rotationXDegrees", &mesh.rotation_x_degrees),
                        ("rotationYDegrees", &mesh.rotation_y_degrees),
                        ("rotationZDegrees", &mesh.rotation_z_degrees),
                        ("scaleX", &mesh.scale_x),
                        ("scaleY", &mesh.scale_y),
                        ("scaleZ", &mesh.scale_z),
                    ] {
                        number(format!("/kind/frame/meshes/{at}/{name}"), value, &mut refs);
                    }
                }
                for (at, value) in frame.light_intensities.iter().enumerate() {
                    number(
                        format!("/kind/frame/lightIntensities/{at}"),
                        value,
                        &mut refs,
                    );
                }
            }
            NodeKind::Video {
                source: _,
                source_start,
                speed,
            } => {
                number("/kind/sourceStart".into(), source_start, &mut refs);
                number("/kind/speed".into(), speed, &mut refs);
            }
        }
        refs
    }

    /// Expressions directly referenced by per-unit styles.
    pub fn per_unit_expr_refs(&self) -> Vec<(String, ExprId)> {
        let NodeKind::Text {
            per_unit: Some(per_unit),
            ..
        } = &self.kind
        else {
            return Vec::new();
        };
        let mut refs = Vec::new();
        let style = &per_unit.style;
        if let Some(NumberValue::Expr { expr }) = &style.opacity {
            refs.push(("/kind/perUnit/style/opacity".into(), *expr));
        }
        if let Some(PointValue::Expr { expr }) = &style.translate {
            refs.push(("/kind/perUnit/style/translate".into(), *expr));
        }
        if let Some(PointValue::Expr { expr }) = &style.scale {
            refs.push(("/kind/perUnit/style/scale".into(), *expr));
        }
        if let Some(NumberValue::Expr { expr }) = &style.rotate {
            refs.push(("/kind/perUnit/style/rotate".into(), *expr));
        }
        if let Some(ColorValue::Expr { expr }) = &style.color {
            refs.push(("/kind/perUnit/style/color".into(), *expr));
        }
        refs
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResourceRef {
    pub control: String,
    pub content_hash: ContentDigest,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SceneArtifact {
    pub format_version: u32,
    pub capability_set: CapabilitySet,
    pub component: String,
    pub controls: ControlsSchema,
    pub resource_refs: Vec<ResourceRef>,
    pub exprs: Vec<Expr>,
    pub nodes: Vec<SceneNode>,
    pub node_children: Vec<NodeId>,
    pub root: NodeId,
    /// Optional scene camera; absent cameras leave World and Screen aligned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera: Option<CameraBinding>,
}

/// Inspectable scene-camera bindings lowered to ordinary transform groups after layout, sharing the
/// existing recording and executor paths.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CameraBinding {
    /// World-space point placed at the viewport center, in pixels.
    pub center: PointValue,
    /// Positive zoom factor.
    pub zoom: NumberValue,
    /// Rotation around the viewport center, in degrees.
    pub rotation: NumberValue,
}

impl SceneArtifact {
    /// Whether evaluating this artifact samples the immutable compositor entry
    /// backdrop. This is derived from the admitted IR so package builders and
    /// Engine admission cannot disagree about Glass/backdrop semantics.
    pub fn reads_destination(&self) -> bool {
        self.nodes.iter().any(|node| {
            matches!(node.kind, NodeKind::Glass(_) | NodeKind::GlassField(_))
                || node.styles.iter().any(|style| {
                    style.property == "backdrop-filter"
                        || style.property.starts_with("motion-backdrop-displacement-")
                })
        })
    }

    pub fn validate(&self) -> Result<(), Vec<ValidationError>> {
        self.validate_impl()
    }

    fn validate_impl(&self) -> Result<(), Vec<ValidationError>> {
        let mut errors = Vec::new();
        if self.format_version != ARTIFACT_FORMAT_VERSION {
            errors.push(ValidationError::new(
                "/formatVersion",
                format!("expected exact artifact format {ARTIFACT_FORMAT_VERSION}"),
            ));
        }
        if self.component.is_empty() {
            errors.push(ValidationError::new(
                "/component",
                "component name must not be empty",
            ));
        }
        if let Err(error) = self.capability_set.validate() {
            errors.push(error);
        }
        let known =
            |name: &str| BASE_CAPABILITIES.contains(&name) || OPTIONAL_CAPABILITIES.contains(&name);
        if let Some(unknown) = self
            .capability_set
            .names
            .iter()
            .find(|name| !known(name.as_str()))
        {
            errors.push(ValidationError::new(
                "/capabilitySet",
                format!("`{unknown}` is not a capability this compiler/runtime implements"),
            ));
        }
        if let Some(missing) = BASE_CAPABILITIES
            .iter()
            .find(|name| !self.capability_set.names.iter().any(|have| have == *name))
        {
            errors.push(ValidationError::new(
                "/capabilitySet",
                format!("base capability `{missing}` is missing from the capability set"),
            ));
        }
        self.controls.validate(&mut errors);
        self.validate_resources(&mut errors);
        let expr_types = validate_exprs(&self.exprs, &self.controls, &mut errors).types;
        self.validate_nodes(&expr_types, &mut errors);
        self.validate_unit_scope(&mut errors);
        self.validate_bounds_scope(&mut errors);
        self.validate_camera(&mut errors);
        self.validate_viewport_capability(&mut errors);
        self.validate_video_capability(&mut errors);
        self.validate_batch_capabilities(&mut errors);
        self.validate_advanced_filter_capability(&mut errors);
        self.validate_shader_capability(&mut errors);
        self.validate_scene3d_capability(&mut errors);
        self.validate_math_formula_capability(&mut errors);
        self.validate_motion_glass_capability(&mut errors);
        if let Err(glass_errors) = crate::glass::validate_glass_schema(self) {
            errors.extend(glass_errors);
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Admission with the external, content-addressed shader package environment.
    ///
    /// The ordinary wire validator proves self-contained shape/type/capability invariants. A host
    /// that may execute ShaderLayer must additionally call this method before layout or pixels so
    /// URI resolution, content/ABI pins and manifest slots cannot drift after compilation.
    pub fn validate_with_shaders(
        &self,
        registry: &crate::shader::ShaderRegistry,
    ) -> Result<(), Vec<ValidationError>> {
        let mut errors = self.validate().err().unwrap_or_default();
        for (node_at, node) in self.nodes.iter().enumerate() {
            let NodeKind::ShaderLayer {
                program,
                uniforms,
                inputs,
            } = &node.kind
            else {
                continue;
            };
            let base = format!("/nodes/{node_at}/kind");
            let uri = match program.uri.parse::<crate::shader::ShaderUri>() {
                Ok(uri) => uri,
                Err(error) => {
                    errors.push(ValidationError::new(
                        format!("{base}/program/uri"),
                        error.to_string(),
                    ));
                    continue;
                }
            };
            let package = match registry.resolve(&uri) {
                Ok(package) => package,
                Err(error) => {
                    errors.push(ValidationError::new(
                        format!("{base}/program/uri"),
                        error.to_string(),
                    ));
                    continue;
                }
            };
            if program.content_hash != package.content_hash {
                errors.push(ValidationError::new(
                    format!("{base}/program/contentHash"),
                    format!(
                        "artifact pins {}, registry resolves {}",
                        program.content_hash, package.content_hash
                    ),
                ));
            }
            if program.abi_hash != package.manifest.abi_digest {
                errors.push(ValidationError::new(
                    format!("{base}/program/abiHash"),
                    format!(
                        "artifact pins {}, registry resolves {}",
                        program.abi_hash, package.manifest.abi_digest
                    ),
                ));
            }

            if uniforms.len() != package.manifest.uniforms.len() {
                errors.push(ValidationError::new(
                    format!("{base}/uniforms"),
                    "artifact must materialize every manifest uniform in ABI order",
                ));
            }
            for (slot, declared) in package.manifest.uniforms.iter().enumerate() {
                let Some(binding) = uniforms.get(slot) else {
                    continue;
                };
                let actual_type = match binding.value {
                    ShaderUniformValue::Float { .. } => crate::shader::UniformType::Float,
                    ShaderUniformValue::Float2 { .. } => crate::shader::UniformType::Float2,
                    ShaderUniformValue::Color { .. } => crate::shader::UniformType::Color,
                    ShaderUniformValue::Bool { .. } => crate::shader::UniformType::Bool,
                };
                if binding.name != declared.name || actual_type != declared.uniform_type {
                    errors.push(ValidationError::new(
                        format!("{base}/uniforms/{slot}"),
                        format!(
                            "expected ABI slot `{}` ({:?}), got `{}` ({actual_type:?})",
                            declared.name, declared.uniform_type, binding.name
                        ),
                    ));
                }
                let static_value = match &binding.value {
                    ShaderUniformValue::Float {
                        value: NumberValue::Static { value },
                    } => Some(crate::shader::UniformValue::Float(*value as f32)),
                    ShaderUniformValue::Float2 {
                        value: PointValue::Static { value },
                    } => Some(crate::shader::UniformValue::Float2([
                        value.x as f32,
                        value.y as f32,
                    ])),
                    ShaderUniformValue::Color {
                        value: ColorValue::Static { value },
                    } => Some(crate::shader::UniformValue::Color([
                        f32::from(value.r) / 255.0,
                        f32::from(value.g) / 255.0,
                        f32::from(value.b) / 255.0,
                        f32::from(value.a) / 255.0,
                    ])),
                    ShaderUniformValue::Bool {
                        value: BoolValue::Static { value },
                    } => Some(crate::shader::UniformValue::Bool(*value)),
                    _ => None,
                };
                if let Some(value) = static_value
                    && let Err(error) = declared.validate_value(&value)
                {
                    errors.push(ValidationError::new(
                        format!("{base}/uniforms/{slot}/value"),
                        error.to_string(),
                    ));
                }
            }

            let authored = inputs
                .iter()
                .map(|input| input.name.as_str())
                .collect::<BTreeSet<_>>();
            for declared in &package.manifest.inputs {
                if declared.required && !authored.contains(declared.name.as_str()) {
                    errors.push(ValidationError::new(
                        format!("{base}/inputs"),
                        format!("missing required texture input `{}`", declared.name),
                    ));
                }
            }
            for (slot, input) in inputs.iter().enumerate() {
                if !package
                    .manifest
                    .inputs
                    .iter()
                    .any(|declared| declared.name == input.name)
                {
                    errors.push(ValidationError::new(
                        format!("{base}/inputs/{slot}/name"),
                        format!("manifest has no texture input `{}`", input.name),
                    ));
                }
            }
            let expected_order = package
                .manifest
                .inputs
                .iter()
                .filter(|declared| authored.contains(declared.name.as_str()))
                .map(|declared| declared.name.as_str())
                .collect::<Vec<_>>();
            let actual_order = inputs
                .iter()
                .map(|input| input.name.as_str())
                .collect::<Vec<_>>();
            if actual_order != expected_order {
                errors.push(ValidationError::new(
                    format!("{base}/inputs"),
                    "shader texture inputs must follow manifest ABI order",
                ));
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Reject unit-dependent expressions outside per-unit styles because the base pass does not
    /// evaluate them. Also reject empty per-unit declarations without bound styles.
    fn validate_unit_scope(&self, errors: &mut Vec<ValidationError>) {
        let unit_dependent = super::expr::unit_dependent(&self.exprs);
        for (at, node) in self.nodes.iter().enumerate() {
            let path = format!("/nodes/{at}");
            for (slot, expr) in node.expr_refs_outside_per_unit() {
                if unit_dependent
                    .get(expr.0 as usize)
                    .copied()
                    .unwrap_or(false)
                {
                    errors.push(ValidationError::new(
                        format!("{path}{slot}"),
                        "ctx.unit.* is only available inside a Text per-unit style",
                    ));
                }
            }
            if let NodeKind::Text {
                per_unit: Some(per_unit),
                ..
            } = &node.kind
                && per_unit.style.is_empty()
            {
                errors.push(ValidationError::new(
                    format!("{path}/kind/perUnit/style"),
                    "per-unit style must bind at least one property",
                ));
            }
        }
    }

    /// Allow post-layout geometry only in paint properties, preventing feedback into layout. Treat
    /// properties as layout-affecting unless explicitly allowlisted; this preserves the two-pass
    /// evaluation order.
    fn validate_bounds_scope(&self, errors: &mut Vec<ValidationError>) {
        /// Properties known not to affect layout; all others are treated conservatively.
        const PAINT_ONLY: &[&str] = &[
            "opacity",
            "color",
            "background-color",
            "background-image",
            "border-color",
            "border-radius",
            "outline-color",
            "box-shadow",
            "text-shadow",
            "fill",
            "stroke",
            "filter",
            "backdrop-filter",
            "mix-blend-mode",
            "isolation",
            "visibility",
            // CSS transforms affect painting without changing layout boxes.
            "transform",
            "translate",
            "rotate",
            "scale",
            "rotate-x",
            "rotate-y",
            "perspective",
            "transform-style",
            "backface-visibility",
            "motion-perspective-origin-x",
            "motion-perspective-origin-y",
            "motion-perspective-origin-x-px",
            "motion-perspective-origin-y-px",
            "motion-perspective-origin-x-percent",
            "motion-perspective-origin-y-percent",
            "motion-transform-3d-translate-x",
            "motion-transform-3d-translate-y",
            "motion-transform-3d-translate-z",
            "motion-transform-3d-translate-x-percent",
            "motion-transform-3d-translate-y-percent",
            "motion-transform-3d-rotate-x",
            "motion-transform-3d-rotate-y",
            "motion-transform-3d-rotate-z",
            "motion-transform-3d-rotate-axis-x",
            "motion-transform-3d-rotate-axis-y",
            "motion-transform-3d-rotate-axis-z",
            "motion-transform-3d-rotate-axis-angle",
            "motion-transform-3d-scale-x",
            "motion-transform-3d-scale-y",
            "motion-transform-3d-scale-z",
            "paper-grain",
            "contact-shadow",
        ];

        let post_layout_dependent = super::expr::post_layout_dependent(&self.exprs);
        let projection_dependent = super::expr::projection_dependent(&self.exprs);
        let is_post_layout = |expr: ExprId| {
            post_layout_dependent
                .get(expr.0 as usize)
                .copied()
                .unwrap_or(false)
        };
        let keys = self
            .nodes
            .iter()
            .map(|node| node.key.as_str())
            .collect::<BTreeSet<_>>();

        // Require bounds target keys to exist at admission time.
        for (at, expr) in self.exprs.iter().enumerate() {
            if let Expr::NodeBounds { key } = expr
                && !keys.contains(key.as_str())
            {
                errors.push(ValidationError::new(
                    format!("/exprs/{at}/key"),
                    format!("bounds target `{key}` is not a node in this scene"),
                ));
            }
            if let Expr::Project3D {
                scene_key,
                anchor_key,
            } = expr
            {
                let target = self.nodes.iter().find(|node| node.key == *scene_key);
                let Some(SceneNode {
                    kind: NodeKind::Scene3D { scene, .. },
                    ..
                }) = target
                else {
                    errors.push(ValidationError::new(
                        format!("/exprs/{at}/sceneKey"),
                        format!("project3d target `{scene_key}` is not a Scene3D node"),
                    ));
                    continue;
                };
                let Some((parent, anchor)) = anchor_key.split_once("::") else {
                    errors.push(ValidationError::new(
                        format!("/exprs/{at}/anchorKey"),
                        "project3d anchor address must be objectKey::anchorKey",
                    ));
                    continue;
                };
                if !scene
                    .anchors
                    .iter()
                    .any(|candidate| candidate.parent == parent && candidate.key == anchor)
                {
                    errors.push(ValidationError::new(
                        format!("/exprs/{at}/anchorKey"),
                        format!("project3d target `{scene_key}` has no anchor `{anchor_key}`"),
                    ));
                }
            }
        }

        for (at, node) in self.nodes.iter().enumerate() {
            let path = format!("/nodes/{at}");
            for (slot, expr) in node.expr_refs_outside_per_unit() {
                if !is_post_layout(expr) {
                    continue;
                }
                if matches!(node.kind, NodeKind::Scene3D { .. })
                    && slot.starts_with("/kind/frame/")
                    && projection_dependent
                        .get(expr.0 as usize)
                        .copied()
                        .unwrap_or(false)
                {
                    errors.push(ValidationError::new(
                        format!("{path}{slot}"),
                        "Scene3D frame state cannot depend on project3d from the same post-layout projection pass",
                    ));
                    continue;
                }
                // Classify styles by property; text content also affects box size.
                let layout_affecting = if let Some(index) = slot
                    .strip_prefix("/styles/")
                    .and_then(|rest| rest.strip_suffix("/value"))
                    .and_then(|index| index.parse::<usize>().ok())
                {
                    node.styles
                        .get(index)
                        .is_none_or(|style| !PAINT_ONLY.contains(&style.property.as_str()))
                } else {
                    slot == "/kind/text"
                };
                if layout_affecting {
                    errors.push(ValidationError::new(
                        format!("{path}{slot}"),
                        "post-layout geometry (bounds/anchor/connect/project3d) may only feed paint; \
                         feeding it back into layout would require re-running layout on its \
                         own output",
                    ));
                }
            }
        }

        if let Some(camera) = &self.camera {
            let mut refs = Vec::new();
            if let PointValue::Expr { expr } = camera.center {
                refs.push(("/camera/center", expr));
            }
            if let NumberValue::Expr { expr } = camera.zoom {
                refs.push(("/camera/zoom", expr));
            }
            if let NumberValue::Expr { expr } = camera.rotation {
                refs.push(("/camera/rotation", expr));
            }
            for (path, expr) in refs {
                if projection_dependent
                    .get(expr.0 as usize)
                    .copied()
                    .unwrap_or(false)
                {
                    errors.push(ValidationError::new(
                        path,
                        "Motion camera cannot depend on project3d in Scene3D v1; project3d may drive 2D geometry or CSS transforms only",
                    ));
                }
            }
        }
    }

    /// Require viewport inputs and the viewport capability to agree in both directions, rejecting
    /// undeclared use and unused declarations.
    fn validate_viewport_capability(&self, errors: &mut Vec<ValidationError>) {
        let uses_viewport = self.exprs.iter().any(|expr| {
            matches!(
                expr,
                Expr::Context {
                    input: ContextInput::ViewportWidth | ContextInput::ViewportHeight
                }
            )
        });
        let declared = self
            .capability_set
            .names
            .iter()
            .any(|name| name == VIEWPORT_CAPABILITY);
        match (uses_viewport, declared) {
            (true, false) => errors.push(ValidationError::new(
                "/capabilitySet/names",
                format!(
                    "scene reads `ctx.viewport.*` but does not declare `{VIEWPORT_CAPABILITY}`; \
                     a consumer without it cannot supply the value"
                ),
            )),
            (false, true) => errors.push(ValidationError::new(
                "/capabilitySet/names",
                format!(
                    "`{VIEWPORT_CAPABILITY}` is declared but the scene never reads ctx.viewport"
                ),
            )),
            _ => {}
        }
    }

    /// Require video nodes and the video capability to agree in both directions.
    fn validate_video_capability(&self, errors: &mut Vec<ValidationError>) {
        let uses_video = self
            .nodes
            .iter()
            .any(|node| matches!(node.kind, NodeKind::Video { .. }));
        let declared = self
            .capability_set
            .names
            .iter()
            .any(|name| name == VIDEO_CAPABILITY);
        match (uses_video, declared) {
            (true, false) => errors.push(ValidationError::new(
                "/capabilitySet/names",
                format!(
                    "scene has a <Video> node but does not declare `{VIDEO_CAPABILITY}`; a \
                     consumer without it would silently drop the whole node"
                ),
            )),
            (false, true) => errors.push(ValidationError::new(
                "/capabilitySet/names",
                format!("`{VIDEO_CAPABILITY}` is declared but the scene has no <Video> node"),
            )),
            _ => {}
        }
    }

    /// Motion Glass nodes and their capability are an exact pair. A missing declaration would
    /// let an older consumer silently lose the material; a declaration without nodes makes the
    /// artifact's execution requirements dishonest.
    fn validate_motion_glass_capability(&self, errors: &mut Vec<ValidationError>) {
        let uses = self
            .nodes
            .iter()
            .any(|node| matches!(node.kind, NodeKind::Glass(_) | NodeKind::GlassField(_)));
        let capability = crate::glass::MOTION_GLASS_CAPABILITY;
        let declared = self
            .capability_set
            .names
            .iter()
            .any(|name| name == capability);
        match (uses, declared) {
            (true, false) => errors.push(ValidationError::new(
                "/capabilitySet/names",
                format!("scene uses Motion Glass but does not declare `{capability}`"),
            )),
            (false, true) => errors.push(ValidationError::new(
                "/capabilitySet/names",
                format!("`{capability}` is declared but the scene has no Motion Glass node"),
            )),
            _ => {}
        }
    }

    fn validate_batch_capabilities(&self, errors: &mut Vec<ValidationError>) {
        let uses_batch = self
            .nodes
            .iter()
            .any(|node| matches!(node.kind, NodeKind::GeometryBatch { .. }));
        let uses_particles = self.nodes.iter().any(|node| {
            matches!(
                node.kind,
                NodeKind::GeometryBatch {
                    batch: GeometryBatchSpec {
                        positions: BatchPositions::Particles { .. },
                        ..
                    }
                }
            )
        });
        let uses_fields = self.nodes.iter().any(|node| {
            matches!(
                &node.kind,
                NodeKind::GeometryBatch { batch }
                    if batch.position_field.is_some()
                        || batch.size_field.is_some()
                        || batch.fill_field.is_some()
                        || batch.opacity_field.is_some()
            )
        });
        for (uses, capability, label) in [
            (uses_batch, GEOMETRY_BATCH_CAPABILITY, "GeometryBatch"),
            (
                uses_fields,
                GEOMETRY_BATCH_FIELD_CAPABILITY,
                "GeometryBatch field",
            ),
            (uses_particles, PARTICLE_FIELD_CAPABILITY, "particles"),
        ] {
            let declared = self
                .capability_set
                .names
                .iter()
                .any(|name| name == capability);
            match (uses, declared) {
                (true, false) => errors.push(ValidationError::new(
                    "/capabilitySet/names",
                    format!("scene uses {label} but does not declare `{capability}`"),
                )),
                (false, true) => errors.push(ValidationError::new(
                    "/capabilitySet/names",
                    format!("`{capability}` is declared but the scene does not use {label}"),
                )),
                _ => {}
            }
        }
        let total = self
            .nodes
            .iter()
            .filter_map(|node| match &node.kind {
                NodeKind::GeometryBatch { batch } => Some(match &batch.positions {
                    BatchPositions::Static { values } => values.len(),
                    BatchPositions::Particles { spec, .. } => spec.count as usize,
                }),
                _ => None,
            })
            .try_fold(0usize, usize::checked_add);
        if total.is_none_or(|total| total > MAX_GEOMETRY_BATCH_INSTANCES_PER_DISPLAY) {
            errors.push(ValidationError::new(
                "/nodes",
                format!(
                    "geometry batches reserve {} instances across one display (max {}); reduce per-node counts or split the scene",
                    total.unwrap_or(usize::MAX),
                    MAX_GEOMETRY_BATCH_INSTANCES_PER_DISPLAY
                ),
            ));
        }
    }

    fn validate_shader_capability(&self, errors: &mut Vec<ValidationError>) {
        let uses_shader = self
            .nodes
            .iter()
            .any(|node| matches!(node.kind, NodeKind::ShaderLayer { .. }));
        let declared = self
            .capability_set
            .names
            .iter()
            .any(|name| name == SHADER_LAYER_CAPABILITY);
        match (uses_shader, declared) {
            (true, false) => errors.push(ValidationError::new(
                "/capabilitySet/names",
                format!(
                    "scene has a <ShaderLayer> node but does not declare \
                     `{SHADER_LAYER_CAPABILITY}`; a consumer without it would silently draw the \
                     unprocessed children"
                ),
            )),
            (false, true) => errors.push(ValidationError::new(
                "/capabilitySet/names",
                format!(
                    "`{SHADER_LAYER_CAPABILITY}` is declared but the scene has no <ShaderLayer> node"
                ),
            )),
            _ => {}
        }
    }

    fn validate_math_formula_capability(&self, errors: &mut Vec<ValidationError>) {
        let uses = self
            .nodes
            .iter()
            .any(|node| matches!(node.kind, NodeKind::MathFormula { .. }));
        let declared = self
            .capability_set
            .names
            .iter()
            .any(|name| name == MATH_FORMULA_CAPABILITY);
        match (uses, declared) {
            (true, false) => errors.push(ValidationError::new(
                "/capabilitySet/names",
                format!(
                    "scene has a <MathFormula> node but does not declare `{MATH_FORMULA_CAPABILITY}`"
                ),
            )),
            (false, true) => errors.push(ValidationError::new(
                "/capabilitySet/names",
                format!(
                    "`{MATH_FORMULA_CAPABILITY}` is declared but the scene has no <MathFormula> node"
                ),
            )),
            _ => {}
        }
    }

    fn validate_scene3d_capability(&self, errors: &mut Vec<ValidationError>) {
        let uses = self
            .nodes
            .iter()
            .any(|node| matches!(node.kind, NodeKind::Scene3D { .. }));
        let declared = self
            .capability_set
            .names
            .iter()
            .any(|name| name == SCENE3D_LAYER_CAPABILITY);
        match (uses, declared) {
            (true, false) => errors.push(ValidationError::new(
                "/capabilitySet/names",
                format!(
                    "scene has a <Scene3D> node but does not declare `{SCENE3D_LAYER_CAPABILITY}`"
                ),
            )),
            (false, true) => errors.push(ValidationError::new(
                "/capabilitySet/names",
                format!(
                    "`{SCENE3D_LAYER_CAPABILITY}` is declared but the scene has no <Scene3D> node"
                ),
            )),
            _ => {}
        }
    }

    /// Require advanced filter style slots and capability declarations to agree so consumers cannot
    /// silently discard unsupported effects.
    fn validate_advanced_filter_capability(&self, errors: &mut Vec<ValidationError>) {
        const DISPLACEMENT: &[&str] = &[
            "motion-displacement-seed",
            "motion-displacement-frequency",
            "motion-displacement-scale",
            "motion-displacement-octaves",
            "motion-displacement-mode",
        ];
        const VELOCITY_BLUR: &[&str] = &[
            "motion-velocity-blur-velocity",
            "motion-velocity-blur-shutter",
        ];
        const BACKDROP_DISPLACEMENT: &[&str] = &[
            "motion-backdrop-displacement-seed",
            "motion-backdrop-displacement-frequency",
            "motion-backdrop-displacement-scale",
            "motion-backdrop-displacement-octaves",
            "motion-backdrop-displacement-mode",
        ];
        let uses = self.nodes.iter().any(|node| {
            node.styles.iter().any(|style| {
                style.property.starts_with("motion-displacement-")
                    || style.property.starts_with("motion-velocity-blur-")
            })
        });
        let declared = self
            .capability_set
            .names
            .iter()
            .any(|name| name == NODE_ADVANCED_FILTER_CAPABILITY);
        match (uses, declared) {
            (true, false) => errors.push(ValidationError::new(
                "/capabilitySet/names",
                format!(
                    "scene uses node-local displacement or velocity blur but does not declare \
                     `{NODE_ADVANCED_FILTER_CAPABILITY}`"
                ),
            )),
            (false, true) => errors.push(ValidationError::new(
                "/capabilitySet/names",
                format!(
                    "`{NODE_ADVANCED_FILTER_CAPABILITY}` is declared but the scene has no \
                     advanced node filter"
                ),
            )),
            _ => {}
        }

        for (at, node) in self.nodes.iter().enumerate() {
            for (prefix, required) in [
                ("motion-displacement-", DISPLACEMENT),
                ("motion-velocity-blur-", VELOCITY_BLUR),
            ] {
                let authored: Vec<&str> = node
                    .styles
                    .iter()
                    .map(|style| style.property.as_str())
                    .filter(|property| property.starts_with(prefix))
                    .collect();
                if authored.is_empty() {
                    continue;
                }
                if let Some(unknown) = authored
                    .iter()
                    .find(|property| !required.contains(property))
                {
                    errors.push(ValidationError::new(
                        format!("/nodes/{at}/styles"),
                        format!("unknown advanced filter slot `{unknown}`"),
                    ));
                }
                if let Some(missing) = required
                    .iter()
                    .find(|property| !authored.contains(property))
                {
                    errors.push(ValidationError::new(
                        format!("/nodes/{at}/styles"),
                        format!("advanced filter slot set is incomplete; missing `{missing}`"),
                    ));
                }
            }
        }

        let uses_backdrop_displacement = self.nodes.iter().any(|node| {
            node.styles
                .iter()
                .any(|style| style.property.starts_with("motion-backdrop-displacement-"))
        });
        let declares_backdrop_displacement = self
            .capability_set
            .names
            .iter()
            .any(|name| name == BACKDROP_DISPLACEMENT_CAPABILITY);
        match (uses_backdrop_displacement, declares_backdrop_displacement) {
            (true, false) => errors.push(ValidationError::new(
                "/capabilitySet/names",
                format!(
                    "scene uses backdrop displacement but does not declare \
                     `{BACKDROP_DISPLACEMENT_CAPABILITY}`"
                ),
            )),
            (false, true) => errors.push(ValidationError::new(
                "/capabilitySet/names",
                format!(
                    "`{BACKDROP_DISPLACEMENT_CAPABILITY}` is declared but the scene has no \
                     backdrop displacement"
                ),
            )),
            _ => {}
        }
        for (at, node) in self.nodes.iter().enumerate() {
            let authored: Vec<&str> = node
                .styles
                .iter()
                .map(|style| style.property.as_str())
                .filter(|property| property.starts_with("motion-backdrop-displacement-"))
                .collect();
            if authored.is_empty() {
                continue;
            }
            if let Some(unknown) = authored
                .iter()
                .find(|property| !BACKDROP_DISPLACEMENT.contains(property))
            {
                errors.push(ValidationError::new(
                    format!("/nodes/{at}/styles"),
                    format!("unknown backdrop displacement slot `{unknown}`"),
                ));
            }
            if let Some(missing) = BACKDROP_DISPLACEMENT
                .iter()
                .find(|property| !authored.contains(property))
            {
                errors.push(ValidationError::new(
                    format!("/nodes/{at}/styles"),
                    format!("backdrop displacement slot set is incomplete; missing `{missing}`"),
                ));
            }
        }
    }

    fn validate_camera(&self, errors: &mut Vec<ValidationError>) {
        let Some(camera) = &self.camera else {
            // Reject a declared camera capability without a camera.
            if self
                .capability_set
                .names
                .iter()
                .any(|name| name == CAMERA_CAPABILITY)
            {
                errors.push(ValidationError::new(
                    "/capabilitySet/names",
                    format!("`{CAMERA_CAPABILITY}` is declared but the scene has no camera"),
                ));
            }
            return;
        };
        if !self
            .capability_set
            .names
            .iter()
            .any(|name| name == CAMERA_CAPABILITY)
        {
            errors.push(ValidationError::new(
                "/camera",
                format!(
                    "scene declares a camera but not the `{CAMERA_CAPABILITY}` capability; a \
                     consumer that ignores the field would render the whole scene in the wrong \
                     framing without any error"
                ),
            ));
        }
        // Require positive static zoom; validate dynamic zoom during evaluation.
        if let NumberValue::Static { value } = &camera.zoom
            && (!value.is_finite() || *value <= 0.0)
        {
            errors.push(ValidationError::new(
                "/camera/zoom",
                "camera zoom must be finite and positive",
            ));
        }
        for (at, node) in self.nodes.iter().enumerate() {
            if node.space == Some(CoordinateSpace::Screen)
                && self.descendant_has_space(NodeId(at as u32), CoordinateSpace::World)
            {
                errors.push(ValidationError::new(
                    format!("/nodes/{at}/space"),
                    "a <World> subtree cannot live inside <Screen>: Screen is defined as the \
                     space the camera does not reach, so re-entering World there has no meaning",
                ));
            }
        }

        // Screen groups must be direct Scene children after all World children.
        //
        // Keep Screen content outside the camera wrapper with a fixed tree structure. Avoid
        // inverse-camera cancellation, whose floating-point error can make overlays jitter.
        let root_children = self
            .nodes
            .get(self.root.0 as usize)
            .map(|root| root.children.start as usize..root.children.end as usize)
            .and_then(|range| self.node_children.get(range))
            .unwrap_or(&[]);
        let mut seen_screen = false;
        for (index, child) in root_children.iter().enumerate() {
            let is_screen = self
                .nodes
                .get(child.0 as usize)
                .is_some_and(|node| node.space == Some(CoordinateSpace::Screen));
            if is_screen {
                seen_screen = true;
            } else if seen_screen {
                errors.push(ValidationError::new(
                    format!("/nodes/{}/space", child.0),
                    "with a camera present, every <Screen> child of <Scene> must come after all \
                     World children",
                ));
            }
            let _ = index;
        }
        for (at, node) in self.nodes.iter().enumerate() {
            if node.space == Some(CoordinateSpace::Screen)
                && !root_children.iter().any(|child| child.0 as usize == at)
            {
                errors.push(ValidationError::new(
                    format!("/nodes/{at}/space"),
                    "with a camera present, <Screen> must be a direct child of <Scene>",
                ));
            }
        }
    }

    fn descendant_has_space(&self, node: NodeId, space: CoordinateSpace) -> bool {
        let Some(template) = self.nodes.get(node.0 as usize) else {
            return false;
        };
        let range = template.children.start as usize..template.children.end as usize;
        self.node_children.get(range).is_some_and(|children| {
            children.iter().any(|child| {
                self.nodes
                    .get(child.0 as usize)
                    .is_some_and(|node| node.space == Some(space))
                    || self.descendant_has_space(*child, space)
            })
        })
    }

    fn validate_resources(&self, errors: &mut Vec<ValidationError>) {
        let mut seen = BTreeSet::new();
        for (index, resource) in self.resource_refs.iter().enumerate() {
            if !self.controls.assets.contains_key(&resource.control) {
                errors.push(ValidationError::new(
                    format!("/resourceRefs/{index}/control"),
                    "resource does not name an assets control",
                ));
            }
            if !seen.insert(&resource.control) {
                errors.push(ValidationError::new(
                    format!("/resourceRefs/{index}/control"),
                    "resource control is duplicated",
                ));
            }
        }
        // `resourceRefs` are content-addressed component defaults, not the only binding site.
        // Required controls are enforced when a standalone authoring command or Timeline
        // instance resolves its assets. Keeping that distinction allows one Artifact to be
        // instantiated with different assets without baking an instance choice into its hash.
    }

    fn validate_nodes(&self, expr_types: &[Option<ExprType>], errors: &mut Vec<ValidationError>) {
        if self.nodes.is_empty() || self.root.0 as usize >= self.nodes.len() {
            errors.push(ValidationError::new("/root", "root must reference a node"));
            return;
        }
        if !matches!(self.nodes[self.root.0 as usize].kind, NodeKind::Group) {
            errors.push(ValidationError::new("/root", "Scene root must be a group"));
        }

        let mut keys = BTreeSet::new();
        let mut parents = vec![0u32; self.nodes.len()];
        let mut child_slots = vec![0u32; self.node_children.len()];
        for (index, node) in self.nodes.iter().enumerate() {
            let path = format!("/nodes/{index}");
            if node.key.is_empty() || !keys.insert(&node.key) {
                errors.push(ValidationError::new(
                    format!("{path}/key"),
                    "node keys must be non-empty and globally unique",
                ));
            }
            for (class_index, class_name) in node.class_names.iter().enumerate() {
                if let Err(reason) = validate_tailwind_class(class_name) {
                    let message = match reason {
                        TailwindClassError::Forbidden => format!(
                            "Tailwind class `{class_name}` is forbidden in frame-pure Motion"
                        ),
                        TailwindClassError::Unsupported => {
                            format!("Tailwind class `{class_name}` is not in {TAILWIND_CATALOG}")
                        }
                    };
                    errors.push(ValidationError::new(
                        format!("{path}/classNames/{class_index}"),
                        message,
                    ));
                }
            }
            if node.children.start > node.children.end
                || node.children.end as usize > self.node_children.len()
            {
                errors.push(ValidationError::new(
                    format!("{path}/children"),
                    "child range is out of bounds",
                ));
                continue;
            }
            if matches!(
                node.kind,
                NodeKind::Text { .. }
                    | NodeKind::Path { .. }
                    | NodeKind::GeometryBatch { .. }
                    | NodeKind::Image { .. }
                    | NodeKind::Video { .. }
                    | NodeKind::Scene3D { .. }
                    | NodeKind::MathFormula { .. }
            ) && node.children.start != node.children.end
            {
                // Reject children on video leaves instead of silently constructing and discarding
                // them.
                errors.push(ValidationError::new(
                    format!("{path}/children"),
                    "leaf nodes cannot have children",
                ));
            }
            if let NodeKind::Text {
                path: Some(text_path),
                ..
            } = &node.kind
                && !path_value_valid(text_path, &self.exprs, expr_types)
            {
                errors.push(ValidationError::new(
                    format!("{path}/kind/path"),
                    "text path must be finite path data or a PathData expression",
                ));
            }
            if let NodeKind::Path {
                d,
                fill,
                stroke,
                trim_start,
                trim_end,
                arrow_start,
                arrow_end,
            } = &node.kind
            {
                let d_valid = path_value_valid(d, &self.exprs, expr_types);
                let number_valid = |value: &NumberValue| match value {
                    NumberValue::Static { value } => {
                        value.is_finite() && (0.0..=1.0).contains(value)
                    }
                    NumberValue::Expr { expr } => {
                        expr_types.get(expr.0 as usize) == Some(&Some(ExprType::Number))
                    }
                };
                let stroke_valid = stroke.as_ref().is_none_or(|stroke| {
                    paint_value_valid(&stroke.paint, expr_types)
                        && match &stroke.width {
                            NumberValue::Static { value } => value.is_finite() && *value >= 0.0,
                            NumberValue::Expr { expr } => {
                                expr_types.get(expr.0 as usize) == Some(&Some(ExprType::Number))
                            }
                        }
                        && stroke.miter_limit.is_finite()
                        && stroke.miter_limit > 0.0
                        && stroke.dash.as_ref().is_none_or(|dash| {
                            !dash.is_empty()
                                && dash.len() % 2 == 0
                                && dash.iter().all(|value| value.is_finite() && *value >= 0.0)
                                && dash.iter().any(|value| *value > 0.0)
                        })
                        && match &stroke.dash_offset {
                            NumberValue::Static { value } => value.is_finite(),
                            NumberValue::Expr { expr } => {
                                expr_types.get(expr.0 as usize) == Some(&Some(ExprType::Number))
                            }
                        }
                });
                let arrow_valid = |arrow: &Option<ArrowSpec>| {
                    arrow
                        .as_ref()
                        .is_none_or(|arrow| arrow.size.is_finite() && arrow.size > 0.0)
                };
                if !d_valid
                    || !number_valid(trim_start)
                    || !number_valid(trim_end)
                    || !stroke_valid
                    || !arrow_valid(arrow_start)
                    || !arrow_valid(arrow_end)
                    || (arrow_start.is_some() || arrow_end.is_some()) && stroke.is_none()
                    || (fill.is_none() && stroke.is_none())
                    || fill
                        .as_ref()
                        .is_some_and(|fill| !paint_value_valid(fill, expr_types))
                {
                    errors.push(ValidationError::new(
                        format!("{path}/kind"),
                        "path needs valid typed d/fill/stroke/trim/arrow paint values and matching geometry policy",
                    ));
                }
            }
            if let NodeKind::GeometryBatch { batch } = &node.kind {
                let count = match &batch.positions {
                    BatchPositions::Static { values } => values.len(),
                    BatchPositions::Particles { frame, spec } => {
                        if expr_types.get(frame.0 as usize) != Some(&Some(ExprType::Number)) {
                            errors.push(ValidationError::new(
                                format!("{path}/kind/batch/positions/frame"),
                                "particle frame must be a Number expression",
                            ));
                        }
                        spec.count as usize
                    }
                };
                let positions_valid = match &batch.positions {
                    BatchPositions::Static { values } => {
                        values.iter().all(|p| p.x.is_finite() && p.y.is_finite())
                    }
                    BatchPositions::Particles { spec, .. } => {
                        let r = spec.emitter;
                        spec.count > 0
                            && spec.count as usize <= MAX_GEOMETRY_BATCH_INSTANCES_PER_NODE
                            && r.x.is_finite()
                            && r.y.is_finite()
                            && r.width.is_finite()
                            && r.height.is_finite()
                            && r.width >= 0.0
                            && r.height >= 0.0
                            && spec.birth_interval.is_finite()
                            && spec.birth_interval > 0.0
                            && spec.lifetime.is_finite()
                            && spec.lifetime > 0.0
                            && spec.velocity_x.iter().all(|v| v.is_finite())
                            && spec.velocity_y.iter().all(|v| v.is_finite())
                            && spec.velocity_x[0] <= spec.velocity_x[1]
                            && spec.velocity_y[0] <= spec.velocity_y[1]
                            && spec.gravity.x.is_finite()
                            && spec.gravity.y.is_finite()
                            && (!spec.looping
                                || spec.count as f64 * spec.birth_interval >= spec.lifetime)
                    }
                };
                let arity = |len: usize, curve: bool| len == 1 || len == count || curve && len == 2;
                let particle = matches!(batch.positions, BatchPositions::Particles { .. });
                let has_fields = batch.position_field.is_some()
                    || batch.size_field.is_some()
                    || batch.fill_field.is_some()
                    || batch.opacity_field.is_some();
                let field_progress_valid =
                    |expr: ExprId| expr_types.get(expr.0 as usize) == Some(&Some(ExprType::Number));
                let field_stagger_valid = |stagger: f64| {
                    stagger.is_finite()
                        && stagger >= 0.0
                        && (count <= 1 || stagger * ((count - 1) as f64) < 1.0)
                };
                let point_field_valid = |field: &BatchPointField, exact: bool| {
                    field_progress_valid(field.progress)
                        && field_stagger_valid(field.stagger)
                        && if exact {
                            field.to.len() == count
                        } else {
                            field.to.len() == 1 || field.to.len() == count
                        }
                        && field
                            .to
                            .iter()
                            .all(|point| point.x.is_finite() && point.y.is_finite())
                };
                let color_field_valid = |field: &BatchColorField| {
                    field_progress_valid(field.progress)
                        && field_stagger_valid(field.stagger)
                        && (field.to.len() == 1 || field.to.len() == count)
                };
                let number_field_valid = |field: &BatchNumberField| {
                    field_progress_valid(field.progress)
                        && field_stagger_valid(field.stagger)
                        && (field.to.len() == 1 || field.to.len() == count)
                        && field
                            .to
                            .iter()
                            .all(|value| value.is_finite() && (0.0..=1.0).contains(value))
                };
                if count == 0
                    || count > MAX_GEOMETRY_BATCH_INSTANCES_PER_NODE
                    || !positions_valid
                    || particle && has_fields
                    || !arity(batch.sizes.len(), particle)
                    || !arity(batch.fills.len(), particle)
                    || !(batch.opacities.is_empty() || arity(batch.opacities.len(), particle))
                    || !(batch.semantic_keys.is_empty() || batch.semantic_keys.len() == count)
                    || batch
                        .sizes
                        .iter()
                        .any(|s| !s.x.is_finite() || !s.y.is_finite() || s.x <= 0.0 || s.y <= 0.0)
                    || batch
                        .opacities
                        .iter()
                        .any(|a| !a.is_finite() || !(0.0..=1.0).contains(a))
                    || batch
                        .position_field
                        .as_ref()
                        .is_some_and(|field| !point_field_valid(field, true))
                    || batch.size_field.as_ref().is_some_and(|field| {
                        !point_field_valid(field, false)
                            || field.to.iter().any(|size| size.x <= 0.0 || size.y <= 0.0)
                    })
                    || batch
                        .fill_field
                        .as_ref()
                        .is_some_and(|field| !color_field_valid(field))
                    || batch
                        .opacity_field
                        .as_ref()
                        .is_some_and(|field| !number_field_valid(field))
                    || batch.semantic_keys.iter().any(String::is_empty)
                    || batch.semantic_keys.iter().collect::<BTreeSet<_>>().len()
                        != batch.semantic_keys.len()
                {
                    errors.push(ValidationError::new(
                        format!("{path}/kind/batch"),
                        "geometry batch needs 1..=100000 finite instances; side arrays must broadcast, match count, or be a two-point particle life curve; fixed fields need Number progress, valid target arity, and a stagger whose final delay is < 1",
                    ));
                }
            }
            if let NodeKind::Clip { path: clip, .. } = &node.kind
                && !path_value_valid(clip, &self.exprs, expr_types)
            {
                errors.push(ValidationError::new(
                    format!("{path}/kind/path"),
                    "clip path must be valid typed PathData with matching geometry policy",
                ));
            }
            if let NodeKind::Mask { source, mode, rect } = &node.kind {
                let source_valid = match source {
                    MaskValue::Paint { paint } => paint_value_valid(paint, expr_types),
                    MaskValue::Image { source } => source
                        .strip_prefix("asset://")
                        .and_then(|name| self.controls.assets.get(name))
                        .is_some_and(|asset| asset.kind == crate::controls::AssetKind::Image),
                    MaskValue::Subtree { source } => {
                        *mode == MaskMode::Alpha
                            && self
                                .node_children
                                .get(node.children.start as usize..node.children.end as usize)
                                .and_then(|children| children.last())
                                .and_then(|child| self.nodes.get(child.0 as usize))
                                .is_some_and(|child| child.key == *source)
                    }
                };
                if !source_valid || !rect_value_valid(rect, expr_types) {
                    errors.push(ValidationError::new(
                        format!("{path}/kind"),
                        "mask needs a valid paint/image source or a final direct alpha subtree source, plus a positive typed rect",
                    ));
                }
            }
            if let NodeKind::MathFormula { latex, .. } = &node.kind {
                if latex.is_empty() {
                    errors.push(ValidationError::new(
                        format!("{path}/kind/latex"),
                        "MathFormula latex must be non-empty",
                    ));
                }
            }
            if let NodeKind::Image { source } = &node.kind {
                let valid = source
                    .strip_prefix("asset://")
                    .and_then(|name| self.controls.assets.get(name))
                    .is_some_and(|asset| asset.kind == crate::controls::AssetKind::Image);
                if !valid {
                    errors.push(ValidationError::new(
                        format!("{path}/kind/source"),
                        "image source must be asset://<image-control>",
                    ));
                }
            }
            if let NodeKind::Video {
                source,
                source_start,
                speed,
            } = &node.kind
            {
                let valid = source
                    .strip_prefix("asset://")
                    .and_then(|name| self.controls.assets.get(name))
                    .is_some_and(|asset| asset.kind == crate::controls::AssetKind::Video);
                if !valid {
                    errors.push(ValidationError::new(
                        format!("{path}/kind/source"),
                        "video source must be asset://<video-control>",
                    ));
                }
                if !scalar_value_valid(source_start, expr_types)
                    || !scalar_value_valid(speed, expr_types)
                {
                    errors.push(ValidationError::new(
                        format!("{path}/kind"),
                        "video sourceStart/speed must be typed numbers",
                    ));
                }
            }
            if let NodeKind::ShaderLayer {
                program,
                uniforms,
                inputs,
            } = &node.kind
            {
                if program.uri.parse::<crate::shader::ShaderUri>().is_err() {
                    errors.push(ValidationError::new(
                        format!("{path}/kind/program/uri"),
                        "shader program must use canonical shader://<name>@<positive-version>",
                    ));
                }
                if node.children.start == node.children.end {
                    errors.push(ValidationError::new(
                        format!("{path}/children"),
                        "ShaderLayer needs at least one child for its implicit content texture",
                    ));
                }

                let mut uniform_names = BTreeSet::new();
                let mut scalar_count = 2usize; // system `resolution` float2
                for (uniform_at, uniform) in uniforms.iter().enumerate() {
                    if uniform.name.is_empty() || !uniform_names.insert(&uniform.name) {
                        errors.push(ValidationError::new(
                            format!("{path}/kind/uniforms/{uniform_at}/name"),
                            "shader uniform names must be non-empty and unique",
                        ));
                    }
                    let valid = match &uniform.value {
                        ShaderUniformValue::Float { value } => {
                            scalar_count += 1;
                            scalar_value_valid(value, expr_types)
                        }
                        ShaderUniformValue::Float2 { value } => {
                            scalar_count += 2;
                            point_value_valid(value, expr_types)
                        }
                        ShaderUniformValue::Color { value } => {
                            scalar_count += 4;
                            color_value_valid(value, expr_types)
                        }
                        ShaderUniformValue::Bool {
                            value: BoolValue::Static { .. },
                        } => {
                            scalar_count += 1;
                            true
                        }
                        ShaderUniformValue::Bool {
                            value: BoolValue::Expr { expr },
                        } => {
                            scalar_count += 1;
                            expr_types.get(expr.0 as usize) == Some(&Some(ExprType::Bool))
                        }
                    };
                    if !valid {
                        errors.push(ValidationError::new(
                            format!("{path}/kind/uniforms/{uniform_at}/value"),
                            "shader uniform value does not match its typed binding",
                        ));
                    }
                }
                if scalar_count > crate::shader::MAX_UNIFORM_SCALARS {
                    errors.push(ValidationError::new(
                        format!("{path}/kind/uniforms"),
                        format!(
                            "shader ABI uses {scalar_count} scalars; maximum is {}",
                            crate::shader::MAX_UNIFORM_SCALARS
                        ),
                    ));
                }

                if inputs.len() > crate::shader::MAX_TEXTURE_INPUTS {
                    errors.push(ValidationError::new(
                        format!("{path}/kind/inputs"),
                        format!(
                            "shader declares {} texture inputs; maximum is {}",
                            inputs.len(),
                            crate::shader::MAX_TEXTURE_INPUTS
                        ),
                    ));
                }
                let mut input_names = BTreeSet::new();
                for (input_at, input) in inputs.iter().enumerate() {
                    if input.name.is_empty() || !input_names.insert(&input.name) {
                        errors.push(ValidationError::new(
                            format!("{path}/kind/inputs/{input_at}/name"),
                            "shader input names must be non-empty and unique",
                        ));
                    }
                    let control = input.source.strip_prefix("asset://");
                    let valid_image = control
                        .and_then(|name| self.controls.assets.get(name))
                        .is_some_and(|asset| asset.kind == crate::controls::AssetKind::Image);
                    if !valid_image {
                        errors.push(ValidationError::new(
                            format!("{path}/kind/inputs/{input_at}/source"),
                            "shader texture input must be asset://<image-control>",
                        ));
                    } else if let Some(control) = control
                        && !self
                            .resource_refs
                            .iter()
                            .any(|resource| resource.control == control)
                    {
                        errors.push(ValidationError::new(
                            format!("{path}/kind/inputs/{input_at}/source"),
                            "shader texture input must have a content-addressed resourceRef",
                        ));
                    }
                }
            }
            if let NodeKind::Scene3D { scene, frame } = &node.kind {
                if let Err(scene_errors) = scene.validate() {
                    for error in scene_errors.0 {
                        errors.push(ValidationError::new(
                            format!("{path}/kind/scene{}", error.path),
                            error.message,
                        ));
                    }
                }
                let expected_keys = scene
                    .meshes
                    .iter()
                    .map(|mesh| mesh.key.as_str())
                    .collect::<Vec<_>>();
                let actual_keys = frame
                    .meshes
                    .iter()
                    .map(|mesh| mesh.key.as_str())
                    .collect::<Vec<_>>();
                if actual_keys != expected_keys {
                    errors.push(ValidationError::new(
                        format!("{path}/kind/frame/meshes"),
                        "Scene3D frame mesh bindings must match static mesh keys in exact order",
                    ));
                }
                if frame.light_intensities.len() != scene.lights.len() {
                    errors.push(ValidationError::new(
                        format!("{path}/kind/frame/lightIntensities"),
                        "Scene3D frame light bindings must match static lights in exact order",
                    ));
                }
                let scalar = |value: &NumberValue| scalar_value_valid(value, expr_types);
                let camera_valid = [
                    &frame.camera.orbit_yaw_degrees,
                    &frame.camera.orbit_pitch_degrees,
                    &frame.camera.distance,
                    &frame.camera.fov_y_degrees,
                ]
                .into_iter()
                .all(scalar);
                let meshes_valid = frame.meshes.iter().all(|mesh| {
                    [
                        &mesh.translation_x,
                        &mesh.translation_y,
                        &mesh.translation_z,
                        &mesh.rotation_x_degrees,
                        &mesh.rotation_y_degrees,
                        &mesh.rotation_z_degrees,
                        &mesh.scale_x,
                        &mesh.scale_y,
                        &mesh.scale_z,
                    ]
                    .into_iter()
                    .all(scalar)
                });
                if !camera_valid || !meshes_valid || !frame.light_intensities.iter().all(scalar) {
                    errors.push(ValidationError::new(
                        format!("{path}/kind/frame"),
                        "Scene3D frame slots must all be typed NumberValue bindings",
                    ));
                }
                for (mesh_at, mesh) in scene.meshes.iter().enumerate() {
                    let model = self.controls.assets.get(&mesh.model_control);
                    if !model.is_some_and(|asset| asset.kind == crate::controls::AssetKind::Model3d)
                    {
                        errors.push(ValidationError::new(
                            format!("{path}/kind/scene/meshes/{mesh_at}/modelControl"),
                            "Scene3D modelControl must name a model3d asset control",
                        ));
                    }
                    if !self
                        .resource_refs
                        .iter()
                        .any(|resource| resource.control == mesh.model_control)
                    {
                        errors.push(ValidationError::new(
                            format!("{path}/kind/scene/meshes/{mesh_at}/modelControl"),
                            "Scene3D model control needs a content-addressed resourceRef",
                        ));
                    }
                    if let Some(texture) = &mesh.material.texture_control {
                        if !self
                            .controls
                            .assets
                            .get(texture)
                            .is_some_and(|asset| asset.kind == crate::controls::AssetKind::Image)
                        {
                            errors.push(ValidationError::new(
                                format!(
                                    "{path}/kind/scene/meshes/{mesh_at}/material/textureControl"
                                ),
                                "Scene3D textureControl must name an image asset control",
                            ));
                        }
                        if !self
                            .resource_refs
                            .iter()
                            .any(|resource| resource.control == *texture)
                        {
                            errors.push(ValidationError::new(
                                format!(
                                    "{path}/kind/scene/meshes/{mesh_at}/material/textureControl"
                                ),
                                "Scene3D texture control needs a content-addressed resourceRef",
                            ));
                        }
                    }
                }
            }
            let child_start = node.children.start as usize;
            let child_end = node.children.end as usize;
            for (child_slot, child) in child_slots[child_start..child_end]
                .iter_mut()
                .zip(&self.node_children[child_start..child_end])
            {
                *child_slot += 1;
                if child.0 as usize >= self.nodes.len() {
                    errors.push(ValidationError::new(
                        format!("{path}/children"),
                        "child node is out of bounds",
                    ));
                } else {
                    parents[child.0 as usize] += 1;
                }
            }
            if let Some(expr) = node.visibility {
                match expr_types.get(expr.0 as usize) {
                    Some(Some(ExprType::Bool)) => {}
                    _ => errors.push(ValidationError::new(
                        format!("{path}/visibility"),
                        "visibility must reference a bool expression",
                    )),
                }
            }
            if let NodeKind::Text {
                text: TextValue::Expr { expr },
                ..
            } = &node.kind
                && !matches!(
                    expr_types.get(expr.0 as usize),
                    Some(Some(ExprType::String)) | Some(Some(ExprType::Enum))
                )
            {
                errors.push(ValidationError::new(
                    format!("{path}/kind/text"),
                    "dynamic text must reference a string or enum expression",
                ));
            }
            for (style_index, style) in node.styles.iter().enumerate() {
                if style.property.is_empty() {
                    errors.push(ValidationError::new(
                        format!("{path}/styles/{style_index}/property"),
                        "style property must not be empty",
                    ));
                }
                match style.property.as_str() {
                    "motion-path-anchor"
                        if !matches!(
                            &style.value,
                            StyleValue::Static {
                                value: MotionValue::Enum(value)
                            } if matches!(value.as_str(), "center" | "topLeft")
                        ) =>
                    {
                        errors.push(ValidationError::new(
                            format!("{path}/styles/{style_index}/value"),
                            "motion-path anchor must be the static enum `center` or `topLeft`",
                        ));
                    }
                    "motion-path-angle-offset"
                        if !matches!(
                            style.value,
                            StyleValue::Static {
                                value: MotionValue::Number(value)
                            } if value.is_finite()
                        ) =>
                    {
                        errors.push(ValidationError::new(
                            format!("{path}/styles/{style_index}/value"),
                            "motion-path angle offset must be a finite static number",
                        ));
                    }
                    _ => {}
                }
                match &style.value {
                    StyleValue::Static { value } if !value.is_finite() => {
                        errors.push(ValidationError::new(
                            format!("{path}/styles/{style_index}/value"),
                            "static style value must be finite",
                        ));
                    }
                    StyleValue::Expr { expr }
                        if !matches!(expr_types.get(expr.0 as usize), Some(Some(_))) =>
                    {
                        errors.push(ValidationError::new(
                            format!("{path}/styles/{style_index}/value"),
                            "style references an invalid expression",
                        ));
                    }
                    _ => {}
                }
                let actual = match &style.value {
                    StyleValue::Static { value } => Some(ExprType::of_value(value)),
                    StyleValue::Expr { expr } => expr_types.get(expr.0 as usize).copied().flatten(),
                };
                let expected = match style.property.as_str() {
                    "translate" => Some(actual == Some(ExprType::Length2)),
                    "rotate" => Some(actual == Some(ExprType::Angle)),
                    "rotate-x" | "rotate-y" => {
                        Some(matches!(actual, Some(ExprType::Number | ExprType::Angle)))
                    }
                    "perspective" => {
                        Some(matches!(actual, Some(ExprType::Number | ExprType::Length)))
                    }
                    "motion-perspective-origin-x" | "motion-perspective-origin-y" => {
                        Some(actual == Some(ExprType::Length))
                    }
                    "motion-perspective-origin-x-px"
                    | "motion-perspective-origin-y-px"
                    | "motion-perspective-origin-x-percent"
                    | "motion-perspective-origin-y-percent" => {
                        Some(actual == Some(ExprType::Number))
                    }
                    "motion-transform-3d-translate-x-percent"
                    | "motion-transform-3d-translate-y-percent" => {
                        Some(actual == Some(ExprType::Number))
                    }
                    "motion-transform-3d-translate-x"
                    | "motion-transform-3d-translate-y"
                    | "motion-transform-3d-translate-z"
                    | "motion-transform-3d-rotate-x"
                    | "motion-transform-3d-rotate-y"
                    | "motion-transform-3d-rotate-z"
                    | "motion-transform-3d-rotate-axis-x"
                    | "motion-transform-3d-rotate-axis-y"
                    | "motion-transform-3d-rotate-axis-z"
                    | "motion-transform-3d-rotate-axis-angle"
                    | "motion-transform-3d-scale-x"
                    | "motion-transform-3d-scale-y"
                    | "motion-transform-3d-scale-z" => Some(actual == Some(ExprType::Number)),
                    "transform-style" | "backface-visibility" => {
                        Some(actual == Some(ExprType::Enum))
                    }
                    "paper-grain" | "contact-shadow" => Some(actual == Some(ExprType::Number)),
                    "scale" => Some(matches!(actual, Some(ExprType::Number | ExprType::Point))),
                    "opacity" => Some(actual == Some(ExprType::Number)),
                    _ => None,
                };
                if let Some(matches) = expected {
                    if !matches {
                        errors.push(ValidationError::new(
                            format!("{path}/styles/{style_index}/value"),
                            format!("style `{}` has the wrong typed value", style.property),
                        ));
                    }
                }
                if style.property == "scale"
                    && actual == Some(ExprType::Point)
                    && !self
                        .capability_set
                        .names
                        .iter()
                        .any(|name| name == TRANSFORM_SCALE2D_CAPABILITY)
                {
                    errors.push(ValidationError::new(
                        format!("{path}/styles/{style_index}/value"),
                        format!(
                            "two-axis scale requires the `{TRANSFORM_SCALE2D_CAPABILITY}` capability"
                        ),
                    ));
                }
                if (style.property.starts_with("motion-transform-3d-")
                    || matches!(
                        style.property.as_str(),
                        "perspective" | "transform-style" | "backface-visibility"
                    ))
                    && !self
                        .capability_set
                        .names
                        .iter()
                        .any(|name| name == CSS_3D_TRANSFORM_CAPABILITY)
                {
                    errors.push(ValidationError::new(
                        format!("{path}/styles/{style_index}/value"),
                        format!(
                            "CSS 3D style requires the `{CSS_3D_TRANSFORM_CAPABILITY}` capability"
                        ),
                    ));
                }
                if matches!(
                    style.property.as_str(),
                    "motion-transform-3d-translate-x-percent"
                        | "motion-transform-3d-translate-y-percent"
                ) && !self
                    .capability_set
                    .names
                    .iter()
                    .any(|name| name == CSS_TRANSFORM_PERCENT_CAPABILITY)
                {
                    errors.push(ValidationError::new(
                        format!("{path}/styles/{style_index}/value"),
                        format!(
                            "percentage transform requires the `{CSS_TRANSFORM_PERCENT_CAPABILITY}` capability"
                        ),
                    ));
                }
                if matches!(
                    style.property.as_str(),
                    "motion-perspective-origin-x"
                        | "motion-perspective-origin-y"
                        | "motion-perspective-origin-x-px"
                        | "motion-perspective-origin-y-px"
                        | "motion-perspective-origin-x-percent"
                        | "motion-perspective-origin-y-percent"
                ) && !self
                    .capability_set
                    .names
                    .iter()
                    .any(|name| name == CSS_3D_PERSPECTIVE_ORIGIN_CAPABILITY)
                {
                    errors.push(ValidationError::new(
                        format!("{path}/styles/{style_index}/value"),
                        format!(
                            "perspectiveOrigin requires the `{CSS_3D_PERSPECTIVE_ORIGIN_CAPABILITY}` capability"
                        ),
                    ));
                }
                if style.property == "transform-style"
                    && !matches!(
                        &style.value,
                        StyleValue::Static { value: MotionValue::Enum(value) }
                            if value == "preserve-3d"
                    )
                {
                    errors.push(ValidationError::new(
                        format!("{path}/styles/{style_index}/value"),
                        "transformStyle must be the static enum `preserve-3d`",
                    ));
                }
                if style.property == "backface-visibility"
                    && !matches!(
                        &style.value,
                        StyleValue::Static { value: MotionValue::Enum(value) }
                            if matches!(value.as_str(), "hidden" | "visible")
                    )
                {
                    errors.push(ValidationError::new(
                        format!("{path}/styles/{style_index}/value"),
                        "backfaceVisibility must be the static enum `hidden` or `visible`",
                    ));
                }
                if matches!(
                    style.property.as_str(),
                    "motion-displacement-seed" | "motion-backdrop-displacement-seed"
                ) {
                    if actual != Some(ExprType::Number) {
                        errors.push(ValidationError::new(
                            format!("{path}/styles/{style_index}/value"),
                            "displacement seed must be a number",
                        ));
                    }
                    if matches!(style.value, StyleValue::Expr { .. })
                        && !self
                            .capability_set
                            .names
                            .iter()
                            .any(|name| name == DISPLACEMENT_SEED_EXPR_CAPABILITY)
                    {
                        errors.push(ValidationError::new(
                            format!("{path}/styles/{style_index}/value"),
                            format!(
                                "frame-time displacement seed requires the `{DISPLACEMENT_SEED_EXPR_CAPABILITY}` capability"
                            ),
                        ));
                    }
                }
                if matches!(style.property.as_str(), "filter" | "backdrop-filter")
                    && let StyleValue::Expr { expr } = &style.value
                {
                    let valid = css_expression_variants(*expr, &self.exprs, expr_types).is_some();
                    if !valid {
                        errors.push(ValidationError::new(
                            format!("{path}/styles/{style_index}/value"),
                            format!(
                                "dynamic `{}` must use literal strings, typed templates, or finite conditional branches",
                                style.property
                            ),
                        ));
                    }
                }
            }
        }
        if parents[self.root.0 as usize] != 0 {
            errors.push(ValidationError::new("/root", "root must not have a parent"));
        }
        for (index, count) in parents.iter().enumerate() {
            if index != self.root.0 as usize && *count != 1 {
                errors.push(ValidationError::new(
                    format!("/nodes/{index}"),
                    "every non-root node must have exactly one parent",
                ));
            }
        }
        for (index, count) in child_slots.iter().enumerate() {
            if *count != 1 {
                errors.push(ValidationError::new(
                    format!("/nodeChildren/{index}"),
                    "every child slot must belong to exactly one node range",
                ));
            }
        }

        let mut reachable = vec![false; self.nodes.len()];
        let mut stack = vec![self.root];
        while let Some(node_id) = stack.pop() {
            let index = node_id.0 as usize;
            if index >= self.nodes.len() || reachable[index] {
                continue;
            }
            reachable[index] = true;
            let range = self.nodes[index].children;
            if range.start <= range.end && range.end as usize <= self.node_children.len() {
                stack.extend_from_slice(
                    &self.node_children[range.start as usize..range.end as usize],
                );
            }
        }
        for (index, is_reachable) in reachable.iter().enumerate() {
            if !is_reachable {
                errors.push(ValidationError::new(
                    format!("/nodes/{index}"),
                    "node must be reachable from root",
                ));
            }
        }
    }
}

fn path_value_valid(value: &PathValue, exprs: &[Expr], expr_types: &[Option<ExprType>]) -> bool {
    match value {
        PathValue::Static { value } => value.validate().is_ok(),
        PathValue::Expr { expr, policy } => {
            expr_types.get(expr.0 as usize) == Some(&Some(ExprType::PathData))
                && geometry_eval_policy(exprs, *expr) == Some(*policy)
        }
    }
}

fn scalar_value_valid(value: &NumberValue, expr_types: &[Option<ExprType>]) -> bool {
    match value {
        NumberValue::Static { value } => value.is_finite(),
        NumberValue::Expr { expr } => {
            expr_types.get(expr.0 as usize) == Some(&Some(ExprType::Number))
        }
    }
}

fn point_value_valid(value: &PointValue, expr_types: &[Option<ExprType>]) -> bool {
    match value {
        PointValue::Static { value } => value.x.is_finite() && value.y.is_finite(),
        PointValue::Expr { expr } => {
            expr_types.get(expr.0 as usize) == Some(&Some(ExprType::Point))
        }
    }
}

fn color_value_valid(value: &ColorValue, expr_types: &[Option<ExprType>]) -> bool {
    match value {
        ColorValue::Static { .. } => true,
        ColorValue::Expr { expr } => {
            expr_types.get(expr.0 as usize) == Some(&Some(ExprType::Color))
        }
    }
}

fn gradient_stops_valid(stops: &[GradientStopValue], expr_types: &[Option<ExprType>]) -> bool {
    if !(2..=64).contains(&stops.len()) {
        return false;
    }
    let mut previous = None;
    for stop in stops {
        if !scalar_value_valid(&stop.offset, expr_types)
            || !color_value_valid(&stop.color, expr_types)
        {
            return false;
        }
        if let NumberValue::Static { value } = stop.offset {
            if !(0.0..=1.0).contains(&value) || previous.is_some_and(|previous| value < previous) {
                return false;
            }
            previous = Some(value);
        } else {
            previous = None;
        }
    }
    true
}

fn paint_value_valid(value: &PaintValue, expr_types: &[Option<ExprType>]) -> bool {
    match value {
        PaintValue::Solid { color } => color_value_valid(color, expr_types),
        PaintValue::Linear {
            start, end, stops, ..
        } => {
            point_value_valid(start, expr_types)
                && point_value_valid(end, expr_types)
                && gradient_stops_valid(stops, expr_types)
        }
        PaintValue::Radial {
            center,
            radius,
            stops,
            ..
        } => {
            point_value_valid(center, expr_types)
                && scalar_value_valid(radius, expr_types)
                && !matches!(radius, NumberValue::Static { value } if *value <= 0.0)
                && gradient_stops_valid(stops, expr_types)
        }
        PaintValue::Conic {
            center,
            start_angle,
            stops,
            ..
        } => {
            point_value_valid(center, expr_types)
                && scalar_value_valid(start_angle, expr_types)
                && gradient_stops_valid(stops, expr_types)
        }
    }
}

fn rect_value_valid(value: &RectValue, expr_types: &[Option<ExprType>]) -> bool {
    match value {
        RectValue::Static { value } => {
            value.x.is_finite()
                && value.y.is_finite()
                && value.width.is_finite()
                && value.height.is_finite()
                && value.width > 0.0
                && value.height > 0.0
        }
        RectValue::Expr { expr } => expr_types.get(expr.0 as usize) == Some(&Some(ExprType::Rect)),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValidationError {
    pub path: String,
    pub message: String,
}

impl ValidationError {
    pub(crate) fn new(path: impl Into<String>, message: impl Into<String>) -> Self {
        ValidationError {
            path: path.into(),
            message: message.into(),
        }
    }
}

impl core::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}: {}", self.path, self.message)
    }
}

/// Enumerate the syntax of finite CSS branches without executing frame expressions.
/// Typed holes cannot inject arbitrary CSS tokens; concrete values are parsed at frame time.
/// A visited set bounds traversal by the expression DAG, including shared branch subtrees.
pub(crate) fn css_expression_variants(
    root: ExprId,
    exprs: &[Expr],
    types: &[Option<ExprType>],
) -> Option<Vec<String>> {
    let mut pending = vec![root];
    let mut visited = BTreeSet::new();
    let mut variants = Vec::new();
    while let Some(id) = pending.pop() {
        if !visited.insert(id.0) {
            continue;
        }
        match exprs.get(id.0 as usize)? {
            Expr::Const {
                value: MotionValue::Str(text),
            } => variants.push(text.clone()),
            Expr::Select {
                when_true,
                when_false,
                ..
            } => {
                pending.extend([*when_true, *when_false]);
            }
            Expr::Template { parts } => {
                let mut text = String::new();
                for part in parts {
                    match part {
                        TemplatePart::Text { value } => text.push_str(value),
                        TemplatePart::Expr { expr } => {
                            text.push_str(match types.get(expr.0 as usize)? {
                                Some(ExprType::Number) => "1",
                                Some(ExprType::Length) => "1px",
                                Some(ExprType::Angle) => "1deg",
                                Some(ExprType::Color) => "#000000",
                                _ => return None,
                            })
                        }
                    }
                }
                variants.push(text);
            }
            _ => return None,
        }
    }
    Some(variants)
}
