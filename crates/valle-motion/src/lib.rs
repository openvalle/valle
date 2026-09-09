//! Backend-neutral Motion contracts: versioned Scene Artifacts, typed controls, deterministic
//! expressions, phase timing, content-addressed bundles, computation, shader admission, and Scene3D
//! rasterization. Layout, text shaping, and DrawProgram emission also live here. The crate builds
//! for wasm32 without JavaScript or a pixel-execution backend.

extern crate self as valle_motion;

pub use valle_timeline::internal::ContentDigest;

#[cfg(not(target_arch = "wasm32"))]
mod font_data;
pub mod runtime_fonts;

pub mod artifact;
pub mod batch;
pub mod builtin;
pub mod canonical;
mod colr;
pub mod compute;
pub mod context;
pub mod controls;
pub mod diag;
pub mod domain;
pub mod emit;
pub mod eval;
pub mod expr;
pub mod geometry;
pub mod glass;
pub mod layout;
pub mod lock;
pub mod math_formula;
pub mod phases;
pub mod plan;
pub mod scene3d;
pub mod shader;
pub mod signals;
pub mod spring;
pub mod tailwind;
pub mod text;
pub mod time;
pub mod value;

pub use artifact::{
    ARTIFACT_FORMAT_VERSION, ArrowKind, ArrowSpec, BACKDROP_DISPLACEMENT_CAPABILITY,
    BASE_CAPABILITIES, BatchColorField, BatchNumberField, BatchPointField, BatchPositions,
    BoolValue, CAMERA_CAPABILITY, CSS_3D_PERSPECTIVE_ORIGIN_CAPABILITY,
    CSS_3D_TRANSFORM_CAPABILITY, CSS_TRANSFORM_PERCENT_CAPABILITY, CameraBinding, CapabilitySet,
    ChildRange, ColorValue, CoordinateSpace, DISPLACEMENT_SEED_EXPR_CAPABILITY, FLIP_CAPABILITY,
    FONT_ASSET_CAPABILITY, GEOMETRY_BATCH_CAPABILITY, GEOMETRY_BATCH_FIELD_CAPABILITY,
    GeometryBatchGeometry, GeometryBatchSpec, GradientStopValue, MATH_FORMULA_CAPABILITY,
    MAX_GEOMETRY_BATCH_INSTANCES_PER_NODE, MOTION_MATH_CAPABILITY, MaskValue,
    NODE_ADVANCED_FILTER_CAPABILITY, NUMBER_FORMAT_CAPABILITY, NodeId, NodeKind, NumberValue,
    OPTIONAL_CAPABILITIES, PARTICLE_FIELD_CAPABILITY, PaintValue, ParticleSpec, PathStroke,
    PathValue, PerUnit, PointValue, RICH_TEXT_CAPABILITY, RICH_TEXT_INLINE_IMAGE_CAPABILITY,
    RectValue, ResourceRef, SCENE3D_LAYER_CAPABILITY, SHADER_LAYER_CAPABILITY,
    Scene3DCameraBinding, Scene3DFrameBinding, Scene3DMeshBinding, SceneArtifact, SceneNode,
    SemanticMeta, ShaderProgramRef, ShaderTextureInput, ShaderUniformBinding, ShaderUniformValue,
    StyleBinding, StyleValue, TRANSFORM_SCALE2D_CAPABILITY, TextSplit, TextValue, UnitStyle,
    VIDEO_CAPABILITY, VIEWPORT_CAPABILITY, ValidationError, font_family_alias,
};
pub use batch::{BatchFieldProgress, resolve_geometry_batch};
pub use canonical::{CanonicalError, canonical_bytes};
pub use context::{HoldContext, MotionContext, PhaseContext, PhaseKind};
pub use controls::{
    AssetControl, AssetKind, CameraControls, ControlType, ControlsSchema, CueControl, CueKind,
    FrameControl, MAX_PREPARE_DATA_ARRAY_ITEMS, MAX_PREPARE_DATA_BYTES, MAX_PREPARE_DATA_DEPTH,
    MAX_PREPARE_DATA_TOTAL_ITEMS, OptionalFrameControl, PrepareDataType, PropControl,
    TimingControls, TimingError,
};
pub use diag::{DiagClass, DiagCode, MotionDiagnostic, PlanError};
pub use domain::MotionViewport;
pub use emit::{
    EmitError, EmitReport, FaceCache, default_font_naming, emit, emit_program_with_faces,
    emit_with_faces,
};
pub use eval::{
    EvalError, EvalInputs, ResolvedProps, UnitContext, css_token, eval_all, eval_layout_bounds,
    eval_post_layout, eval_units, expr_reads_runtime_inputs, fold_constants, resolve_props,
    subtree_reads_runtime_inputs,
};
pub use expr::{
    CompareOp, ContextInput, CueField, Expr, ExprId, ExprType, Extrapolation, GeometryField,
    InterpolateStop, MAX_TEMPLATE_OUTPUT_BYTES, MAX_TEMPLATE_PARTS, MAX_TEMPLATE_STATIC_BYTES,
    MathBinaryOp, MathUnaryOp, NumberFormat, TemplatePart, bounds_dependent, geometry_eval_policy,
    post_layout_dependent, projection_dependent,
};
pub use geometry::{
    GeometryError, GeometryEvalPolicy, MAX_FRAME_GEOMETRY_POINTS, MAX_GEOMETRY_TRAJECTORY_FRAMES,
    MAX_PATH_POINTS, PATH_ARC_SEGMENTS, PATH_CURVE_STEPS, PathBooleanOp, PathData,
};
pub use glass::{
    GlassFieldId, GlassSurfaceId, MOTION_GLASS_CAPABILITY, MOTION_GLASS_SCHEMA_ID,
    TemporalContinuity, sample_tracks, validate_glass_schema,
};
pub use layout::{
    GlassLayoutEnvironment, GlassLayoutField, GlassLayoutForeground, GlassLayoutFrame,
    GlassLayoutMaterial, GlassLayoutMotion, GlassLayoutSurface, LAYOUT_ENGINE_ID, LayoutError,
    LayoutOptions, LayoutTree, PreparedScene, Scene3DFrameRequest, Scene3DRequest, StyleCache,
    Viewport, build_tree, layout_boxes, prepare_owned_scene, prepare_scene,
};
pub use lock::{
    ArtifactEnvelope, BuildFingerprint, BundledAsset, BundledFont, BundledSource, EnvelopeDigest,
    LockError, MOTION_BUNDLE_FORMAT_VERSION, MotionBundleManifest,
};
pub use phases::{
    NarrationRetime, PhaseLayout, PhaseSpec, RetimeError, motion_context_at,
    motion_context_at_sample, phase_windows, retime_to_narration, round_div,
};
pub use plan::{PlanClip, PlanStep, SignalPlan};
pub use signals::{CueError, CueSchedule, CueState, CueWindow, ResolvedSignals};
pub use spring::{SpringParams, spring_at};
pub use tailwind::{TAILWIND_CATALOG, TailwindClassError, validate_tailwind_class};
pub use text::{
    DEFAULT_MOTION_FONT_FILES, FontOverride, FontResource, Fonts, GenericFamily, MeasureError,
    MeasuredBox, TextMeasure, default_motion_font_resource, measure_text,
};
pub use time::{frame_at_sample_floor, frame_rate_as_f64, sample_time_at_frame};
pub use value::{MotionEasing, MotionValue};

pub const MOTION_MATH_ENGINE_ID: &str = valle_draw::math::ENGINE_ID;

#[cfg(not(target_arch = "wasm32"))]
pub use text::{DEFAULT_MOTION_FONT, DEFAULT_MOTION_FONT_WEIGHTS};
#[cfg(not(target_arch = "wasm32"))]
pub use text::{default_motion_fonts, motion_font_resource, register_default_motion_fonts};
