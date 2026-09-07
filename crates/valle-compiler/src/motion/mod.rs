//! Motion JSX compiler.
//!
//! OXC owns syntax and source spans. QuickJS runs only static module data and controls inside the
//! deterministic sandbox; expressions depending on `ctx` or `props` are lowered symbolically into
//! `valle_motion::Expr`. The compiler never evaluates a frame.

use std::collections::{BTreeMap, BTreeSet};

use oxc::allocator::Allocator;
use oxc::ast::ast::{
    Argument, ArrayExpressionElement, BinaryOperator, BindingPattern, Declaration,
    ExportDefaultDeclarationKind, Expression, FormalParameters, Function, FunctionBody,
    JSXAttributeItem, JSXAttributeName, JSXAttributeValue, JSXChild, JSXElement, JSXElementName,
    JSXExpression, ObjectExpression, ObjectPropertyKind, Program, PropertyKey, Statement,
};
use oxc::codegen::{Codegen, CodegenOptions, CommentOptions};
use oxc::parser::Parser;
use oxc::span::{GetSpan, SourceType, Span};
use oxc::syntax::operator::LogicalOperator;
use serde::{Deserialize, Serialize};
use valle_draw::program::recording::{FillRule, MaskMode, SpreadMode};
use valle_draw::{Cap, Join, PathVerb, Point, Rect, Rgba};
use valle_motion::diag::{DiagClass, DiagCode, MotionDiagnostic};
use valle_motion::value::{Angle, AngleUnit, Length, Length2};
use valle_motion::{
    ARTIFACT_FORMAT_VERSION, ArrowKind, ArrowSpec, AssetControl, AssetKind,
    BACKDROP_DISPLACEMENT_CAPABILITY, BASE_CAPABILITIES, BatchColorField, BatchNumberField,
    BatchPointField, BatchPositions, BoolValue, CAMERA_CAPABILITY,
    CSS_3D_PERSPECTIVE_ORIGIN_CAPABILITY, CSS_3D_TRANSFORM_CAPABILITY,
    CSS_TRANSFORM_PERCENT_CAPABILITY, CameraBinding, CameraControls, CapabilitySet, ChildRange,
    ColorValue, CompareOp, ContentDigest, ContextInput, ControlType, ControlsSchema,
    CoordinateSpace, CueControl, CueField, CueKind, DISPLACEMENT_SEED_EXPR_CAPABILITY, Expr,
    ExprId, Extrapolation, FLIP_CAPABILITY, FONT_ASSET_CAPABILITY, FrameControl,
    GEOMETRY_BATCH_CAPABILITY, GEOMETRY_BATCH_FIELD_CAPABILITY, GeometryBatchGeometry,
    GeometryBatchSpec, GeometryField, GradientStopValue, InterpolateStop, MATH_FORMULA_CAPABILITY,
    MOTION_MATH_CAPABILITY, MaskValue, MathBinaryOp, MathUnaryOp, MotionEasing, MotionValue,
    NUMBER_FORMAT_CAPABILITY, NodeId, NodeKind, NumberFormat, NumberValue, OptionalFrameControl,
    PARTICLE_FIELD_CAPABILITY, PaintValue, ParticleSpec, PathBooleanOp, PathData, PathStroke,
    PathValue, PerUnit, PointValue, PrepareDataType, PropControl, RICH_TEXT_CAPABILITY, RectValue,
    ResourceRef, SCENE3D_LAYER_CAPABILITY, SHADER_LAYER_CAPABILITY, Scene3DCameraBinding,
    Scene3DFrameBinding, Scene3DMeshBinding, SceneArtifact, SceneNode, SemanticMeta,
    ShaderProgramRef, ShaderTextureInput, ShaderUniformBinding, ShaderUniformValue, StyleBinding,
    StyleValue, TRANSFORM_SCALE2D_CAPABILITY, TextSplit, TextValue, TimingControls, UnitStyle,
    VIEWPORT_CAPABILITY, font_family_alias, geometry_eval_policy,
};
use valle_motion::{TAILWIND_CATALOG, TailwindClassError, validate_tailwind_class};

use crate::motion_sandbox::{Sandbox, THEME_SCOPE_BINDING, scan_forbidden};

/// Prepare-time intrinsic measurement environment.
pub use crate::motion_sandbox::MeasureEnv;
/// The single admitted package environment shared by compiler and renderer hosts.
pub type ShaderRegistryEnv = valle_motion::shader::ShaderRegistry;

mod attrs;
mod builtins;
mod collections;
mod compiler;
mod controls;
mod css;
mod diagnostics;
mod effects;
mod expr;
mod frontend;
mod geometry;
mod glass;
mod jsx;
mod modules;
mod output;
mod prelude;
mod scene3d;
mod scope;
mod shader;
mod style;
mod syntax;
mod text;
mod theme;
mod transform;
mod values;
use controls::*;
use css::*;
use diagnostics::*;
use frontend::*;
pub use modules::{
    MotionModuleGraph, compile_motion_modules, compile_motion_modules_with_full_env,
    compile_motion_modules_with_full_env_and_data,
};
use output::*;
use prelude::STATIC_HELPERS;
use syntax::*;
use theme::*;
use values::*;

/// The current theme value is rebound inside each prepare evaluation: one compiler can enter and
/// leave nested providers while reusing the same hardened sandbox. The binding is compiler-private
/// and never reaches the Artifact.
const MAX_THEME_DEPTH: usize = 16;
const MAX_THEME_TOKENS: usize = 1_024;
const MAX_THEME_BYTES: usize = 64 * 1_024;
const THEME_TYPED_TOKEN_KINDS: &[&str] = &[
    "conicGradient",
    "fitText",
    "gradientStop",
    "layoutStates",
    "linearGradient",
    "motionPath",
    "nodeDisplacement",
    "nodeMotionBlur",
    "pathArc",
    "pathArea",
    "pathBoolean",
    "pathCubic",
    "pathData",
    "pathLine",
    "pathOffset",
    "point",
    "radialGradient",
    "rect",
    "sequenceStage",
    "sequenceStageInput",
    "sequenceTime",
];

const MAX_STATIC_MAP_ITEMS: usize = 10_000;
/// Bound total helper expansion, including nonrecursive fan-out that can grow exponentially.
const MAX_EXPANDED_HELPER_CALLS: usize = 2_000;
const MAX_TOTAL_EXPANDED_LIST_ITEMS: usize = 50_000;
/// Limit expression nesting to protect even the smaller test-thread stack. Report generated
/// expressions that exceed the budget before entering QuickJS with an exhausted stack.
const MAX_EXPR_NESTING: usize = 256;
/// Compile-time unroll budget for fixed expression collectors. Reject inputs whose length cannot be
/// proven to fit within 4096 entries.
const MAX_EXPR_MAP_UNROLL: usize = 4096;
pub const MOTION_COMPILER_ID: &str = concat!("valle-compiler@", env!("CARGO_PKG_VERSION"));

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
enum SequenceUnit {
    Seconds,
    Frames,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SequenceStage {
    #[serde(rename = "__valleType")]
    marker: String,
    unit: SequenceUnit,
    start: f64,
    duration: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SequenceTime {
    #[serde(rename = "__valleType")]
    marker: String,
    unit: SequenceUnit,
    value: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StaticLayoutRect {
    #[serde(rename = "__valleType")]
    marker: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StaticLayoutStates {
    #[serde(rename = "__valleType")]
    marker: String,
    states: BTreeMap<String, BTreeMap<String, StaticLayoutRect>>,
    ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceSpan {
    pub start: u32,
    pub end: u32,
    pub line: u32,
    pub column: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompilerDiagnostic {
    pub class: DiagClass,
    pub code: DiagCode,
    pub span: SourceSpan,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    pub message: String,
}

pub const MOTION_SOURCE_MAP_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModuleSourceInfo {
    pub path: String,
    pub source_digest: ContentDigest,
    pub normalized_ast_digest: ContentDigest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NodeSourceMapping {
    pub id: NodeId,
    pub key: String,
    pub source_path: String,
    pub expansion_stack: Vec<String>,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExprSourceMapping {
    pub id: ExprId,
    pub source_path: String,
    pub expansion_stack: Vec<String>,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObjectSourceMapping {
    pub scene_key: String,
    pub object_key: String,
    pub semantic_address: String,
    pub source_path: String,
    pub expansion_stack: Vec<String>,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MotionSourceMap {
    pub version: u32,
    pub component: String,
    pub entry: String,
    pub closure_digest: ContentDigest,
    pub modules: Vec<ModuleSourceInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controls_source_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controls: Option<SourceSpan>,
    pub nodes: Vec<NodeSourceMapping>,
    pub exprs: Vec<ExprSourceMapping>,
    pub objects: Vec<ObjectSourceMapping>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompiledMotion {
    pub artifact: SceneArtifact,
    pub source_map: MotionSourceMap,
    pub normalized_source: String,
    pub normalized_ast_digest: ContentDigest,
    /// Canonical hash of the validated prepare binding, including its declared source identity.
    pub prepared_data_digest: ContentDigest,
}

/// One explicit, reproducible `controls.data` binding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PrepareDataBinding {
    /// Stable author-facing origin such as `timeline:clip-id` or a project-relative JSON path.
    pub source: String,
    pub value: serde_json::Value,
}

/// Compile one deterministic Motion TSX module directly into the Scene IR.
pub fn compile_motion(source: &str) -> Result<CompiledMotion, Vec<CompilerDiagnostic>> {
    compile_motion_with_resources(source, &[])
}

/// Compile with content-addressed asset bindings. This is the build/preview entrypoint when a
/// source declares required assets; the plain compiler remains useful for asset-free linting.
pub fn compile_motion_with_resources(
    source: &str,
    resources: &[ResourceRef],
) -> Result<CompiledMotion, Vec<CompilerDiagnostic>> {
    compile_motion_with_env(source, resources, None)
}

/// Compile with explicit prepare-time capabilities. Without a measurement environment, measureText
/// is unavailable and produces a missing-font diagnostic rather than an unrelated constant.
pub fn compile_motion_with_env(
    source: &str,
    resources: &[ResourceRef],
    measure: Option<&MeasureEnv>,
) -> Result<CompiledMotion, Vec<CompilerDiagnostic>> {
    compile_motion_with_full_env(source, resources, measure, None)
}

/// Compile with the complete deterministic authoring environment.
///
/// A `<ShaderLayer>` is rejected unless its exact package is present in `shaders`; filesystem or
/// URL lookup is deliberately a host concern so standalone/project/preview cannot drift.
pub fn compile_motion_with_full_env(
    source: &str,
    resources: &[ResourceRef],
    measure: Option<&MeasureEnv>,
    shaders: Option<&ShaderRegistryEnv>,
) -> Result<CompiledMotion, Vec<CompilerDiagnostic>> {
    compile_motion_with_full_env_and_data(source, resources, measure, shaders, None)
}

/// Compile with the complete deterministic environment plus structured prepare data.
pub fn compile_motion_with_full_env_and_data(
    source: &str,
    resources: &[ResourceRef],
    measure: Option<&MeasureEnv>,
    shaders: Option<&ShaderRegistryEnv>,
    data: Option<&PrepareDataBinding>,
) -> Result<CompiledMotion, Vec<CompilerDiagnostic>> {
    compile_motion_impl(source, resources, measure, shaders, data)
}

fn compile_motion_impl(
    source: &str,
    resources: &[ResourceRef],
    measure: Option<&MeasureEnv>,
    shaders: Option<&ShaderRegistryEnv>,
    data: Option<&PrepareDataBinding>,
) -> Result<CompiledMotion, Vec<CompilerDiagnostic>> {
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, SourceType::tsx()).parse();
    if !parsed.diagnostics.is_empty() {
        return Err(parsed
            .diagnostics
            .into_iter()
            .map(|diagnostic| {
                let span = diagnostic
                    .labels
                    .first()
                    .map_or(Span::new(0, 0), |label| label.span());
                diagnostic_at(source, DiagCode::SyntaxError, span, diagnostic.to_string())
            })
            .collect());
    }

    let program = parsed.program;
    let normalized_source = normalized_ast(&program);
    let normalized_ast_digest = ContentDigest::of_bytes(normalized_source.as_bytes());
    let mut compiler = Compiler::new(source, &program, resources, measure, shaders, data)?;
    let artifact = compiler.compile(&program)?;
    let prepared_data_bytes =
        valle_motion::canonical_bytes(&data).expect("validated prepare data binding is canonical");
    let prepared_data_digest = ContentDigest::of_bytes(&prepared_data_bytes);
    let source_map = MotionSourceMap {
        version: MOTION_SOURCE_MAP_VERSION,
        component: artifact.component.clone(),
        entry: "<inline>".into(),
        closure_digest: ContentDigest::of_bytes(source.as_bytes()),
        modules: vec![ModuleSourceInfo {
            path: "<inline>".into(),
            source_digest: ContentDigest::of_bytes(source.as_bytes()),
            normalized_ast_digest,
        }],
        controls_source_path: compiler.controls_span.map(|_| "<inline>".into()),
        controls: compiler
            .controls_span
            .map(|span| source_span_at(source, span)),
        nodes: artifact
            .nodes
            .iter()
            .zip(&compiler.source_ledger.node_spans)
            .zip(&compiler.source_ledger.node_expansion_stacks)
            .enumerate()
            .map(
                |(index, ((node, span), expansion_stack))| NodeSourceMapping {
                    id: NodeId(index as u32),
                    key: node.key.clone(),
                    source_path: "<inline>".into(),
                    expansion_stack: expansion_stack.clone(),
                    span: source_span_at(source, *span),
                },
            )
            .collect(),
        exprs: compiler
            .expr_arena
            .spans
            .iter()
            .zip(&compiler.expr_arena.expansion_stacks)
            .enumerate()
            .map(|(index, (span, expansion_stack))| ExprSourceMapping {
                id: ExprId(index as u32),
                source_path: "<inline>".into(),
                expansion_stack: expansion_stack.clone(),
                span: source_span_at(source, *span),
            })
            .collect(),
        objects: compiler
            .source_ledger
            .object_spans
            .iter()
            .map(
                |(scene_key, object_key, span, expansion_stack)| ObjectSourceMapping {
                    scene_key: scene_key.clone(),
                    object_key: object_key.clone(),
                    semantic_address: valle_motion::scene3d::Scene3DSpec::semantic_address(
                        scene_key, object_key,
                    )
                    .expect("validated Scene3D keys form one semantic address"),
                    source_path: "<inline>".into(),
                    expansion_stack: expansion_stack.clone(),
                    span: source_span_at(source, *span),
                },
            )
            .collect(),
    };
    Ok(CompiledMotion {
        artifact,
        source_map,
        normalized_source,
        normalized_ast_digest,
        prepared_data_digest,
    })
}

#[derive(Clone)]
struct PendingNode {
    span: Span,
    expansion_stack: Vec<String>,
    key: String,
    kind: NodeKind,
    /// Explicit World/Screen marker; None inherits the parent's space, defaulting to World.
    space: Option<CoordinateSpace>,
    class_names: Vec<String>,
    styles: Vec<StyleBinding>,
    visibility: Option<ExprId>,
    semantic: Option<SemanticMeta>,
    children: Vec<PendingNode>,
    /// Compiler-only marker. It is consumed by the direct parent `<Mask>` and does not enter wire.
    is_mask_source: bool,
}

#[derive(Clone)]
enum AuthorValue {
    Static(serde_json::Value),
    Dynamic(ExprId),
    /// A compile-time-fixed collection whose elements are frame-time expressions.
    /// It is expanded by the compiler and never becomes an Artifact/runtime array.
    DynamicTuple(Vec<ExprId>),
    Children(Vec<PendingNode>),
}

/// An authored function declaration or arrow-function helper.
#[derive(Clone, Copy)]
enum AuthoredFn<'s> {
    Declared(&'s Function<'s>),
    Arrow(&'s oxc::ast::ast::ArrowFunctionExpression<'s>),
}

impl<'s> AuthoredFn<'s> {
    fn params(&self) -> &'s FormalParameters<'s> {
        match self {
            AuthoredFn::Declared(function) => &function.params,
            AuthoredFn::Arrow(arrow) => &arrow.params,
        }
    }

    fn body(&self) -> Option<&'s FunctionBody<'s>> {
        match self {
            AuthoredFn::Declared(function) => function.body.as_deref(),
            AuthoredFn::Arrow(arrow) => arrow.body.as_function_body(),
        }
    }

    fn expression_body(&self) -> Option<&'s Expression<'s>> {
        match self {
            AuthoredFn::Declared(_) => None,
            AuthoredFn::Arrow(arrow) => arrow.get_expression(),
        }
    }

    fn is_async(&self) -> bool {
        match self {
            AuthoredFn::Declared(function) => function.r#async,
            AuthoredFn::Arrow(arrow) => arrow.r#async,
        }
    }

    fn is_generator(&self) -> bool {
        matches!(self, AuthoredFn::Declared(function) if function.generator)
    }

    fn span(&self) -> Span {
        match self {
            AuthoredFn::Declared(function) => function.span(),
            AuthoredFn::Arrow(arrow) => arrow.span(),
        }
    }
}

/// Recognize function-valued const initializers as helpers or components.
fn authored_fn_of<'a>(initializer: &'a Expression<'a>) -> Option<AuthoredFn<'a>> {
    match strip_parens(initializer) {
        Expression::ArrowFunctionExpression(arrow) => Some(AuthoredFn::Arrow(arrow)),
        Expression::FunctionExpression(function) => Some(AuthoredFn::Declared(function)),
        _ => None,
    }
}

fn reserved_theme_binding(pattern: &BindingPattern<'_>) -> Option<Span> {
    match pattern {
        BindingPattern::BindingIdentifier(identifier)
            if matches!(identifier.name.as_str(), "useTheme" | "ThemeProvider") =>
        {
            Some(identifier.span())
        }
        BindingPattern::BindingIdentifier(_) => None,
        BindingPattern::ObjectPattern(pattern) => pattern
            .properties
            .iter()
            .find_map(|property| reserved_theme_binding(&property.value))
            .or_else(|| {
                pattern
                    .rest
                    .as_ref()
                    .and_then(|rest| reserved_theme_binding(&rest.argument))
            }),
        BindingPattern::ArrayPattern(pattern) => pattern
            .elements
            .iter()
            .flatten()
            .find_map(reserved_theme_binding)
            .or_else(|| {
                pattern
                    .rest
                    .as_ref()
                    .and_then(|rest| reserved_theme_binding(&rest.argument))
            }),
        BindingPattern::AssignmentPattern(pattern) => reserved_theme_binding(&pattern.left),
    }
}

fn module_reserved_theme_binding(program: &Program<'_>) -> Option<Span> {
    for statement in &program.body {
        match statement {
            Statement::FunctionDeclaration(function)
                if function.id.as_ref().is_some_and(|identifier| {
                    matches!(identifier.name.as_str(), "useTheme" | "ThemeProvider")
                }) =>
            {
                return function.id.as_ref().map(|identifier| identifier.span());
            }
            Statement::VariableDeclaration(declaration) => {
                if let Some(span) = declaration
                    .declarations
                    .iter()
                    .find_map(|declaration| reserved_theme_binding(&declaration.id))
                {
                    return Some(span);
                }
            }
            _ => {}
        }
    }
    None
}

struct BatchFieldParts<'s> {
    from: &'s Expression<'s>,
    to: &'s Expression<'s>,
    progress: &'s Expression<'s>,
    stagger: f64,
}

#[derive(Clone, Default)]
struct Bindings<'s> {
    /// Component/helper-local authored functions. Lookup remains local-first,
    /// then module scope.
    local_functions: BTreeMap<String, AuthoredFn<'s>>,
    /// Scalar frame-time expressions.
    scalars: BTreeMap<String, ExprId>,
    /// Fixed-length frame-time tuples. These are compiler-only and never enter
    /// the runtime Artifact as arrays.
    tuples: BTreeMap<String, Vec<ExprId>>,
    /// Prepare-time JSON values visible to the deterministic sandbox.
    statics: BTreeMap<String, serde_json::Value>,
    /// JSX child fragments bound by component/helper expansion.
    children: BTreeMap<String, Vec<PendingNode>>,
    /// Props of the component currently being expanded.
    component_props: Option<BTreeMap<String, AuthorValue>>,
}

#[derive(Clone)]
struct BindingFrame<'s> {
    local_functions: BTreeMap<String, AuthoredFn<'s>>,
    scalars: BTreeMap<String, ExprId>,
    tuples: BTreeMap<String, Vec<ExprId>>,
    statics: BTreeMap<String, serde_json::Value>,
    children: BTreeMap<String, Vec<PendingNode>>,
}

impl<'s> Bindings<'s> {
    /// Enter a helper expansion while returning the exact caller state that
    /// must be restored afterwards. Local helpers capture value/function
    /// bindings; module helpers start from an isolated value scope. Children
    /// and component props are always call-local.
    fn enter_helper_scope(&mut self, captures_scope: bool) -> Self {
        Self {
            local_functions: if captures_scope {
                self.local_functions.clone()
            } else {
                std::mem::take(&mut self.local_functions)
            },
            scalars: if captures_scope {
                self.scalars.clone()
            } else {
                std::mem::take(&mut self.scalars)
            },
            tuples: if captures_scope {
                self.tuples.clone()
            } else {
                std::mem::take(&mut self.tuples)
            },
            statics: if captures_scope {
                self.statics.clone()
            } else {
                std::mem::take(&mut self.statics)
            },
            children: std::mem::take(&mut self.children),
            component_props: self.component_props.take(),
        }
    }

    fn enter_isolated_scope(&mut self) -> Self {
        std::mem::take(self)
    }

    fn restore(&mut self, saved: Self) {
        *self = saved;
    }

    /// Snapshot the lexical bindings reset between compile-time loop/map
    /// iterations. Component props belong to the surrounding expansion and
    /// intentionally stay in place.
    fn snapshot_frame(&self) -> BindingFrame<'s> {
        BindingFrame {
            local_functions: self.local_functions.clone(),
            scalars: self.scalars.clone(),
            tuples: self.tuples.clone(),
            statics: self.statics.clone(),
            children: self.children.clone(),
        }
    }

    fn restore_frame(&mut self, frame: BindingFrame<'s>) {
        self.local_functions = frame.local_functions;
        self.scalars = frame.scalars;
        self.tuples = frame.tuples;
        self.statics = frame.statics;
        self.children = frame.children;
    }
}

#[derive(Default)]
struct ExprArena {
    values: Vec<Expr>,
    spans: Vec<Span>,
    expansion_stacks: Vec<Vec<String>>,
    /// Incremental mirror of `expr_reads_runtime_inputs` for each expression.
    reads_runtime: Vec<bool>,
    /// Current recursive lowering depth, guarded by `MAX_EXPR_NESTING`.
    depth: usize,
}

#[derive(Default)]
struct SourceLedger {
    node_spans: Vec<Span>,
    node_expansion_stacks: Vec<Vec<String>>,
    /// Scene3D authored object spans, keyed by stable scene/object identity.
    object_spans: Vec<(String, String, Span, Vec<String>)>,
}

struct Compiler<'s> {
    source: &'s str,
    sandbox: Sandbox,
    /// Scene-level camera lowered from the Scene attribute.
    camera: Option<CameraBinding>,
    /// Optional capabilities declared only when used, allowing consumers to reject unsupported
    /// artifacts.
    extra_capabilities: BTreeSet<String>,
    /// Whether the sandbox can measure text; distinguishes missing fonts from runtime-dependent
    /// inputs.
    has_measure: bool,
    shader_registry: Option<ShaderRegistryEnv>,
    component: Option<String>,
    controls: ControlsSchema,
    controls_span: Option<Span>,
    prepare_data: Option<PrepareDataBinding>,
    default_function: Option<&'s Function<'s>>,
    functions: BTreeMap<String, AuthoredFn<'s>>,
    /// Expression values and their parallel source/runtime metadata are one
    /// append-only arena so truncation and insertion cannot drift apart.
    expr_arena: ExprArena,
    /// Node/object source provenance used to build the standalone source-map format.
    source_ledger: SourceLedger,
    resource_refs: Vec<ResourceRef>,
    /// Font resources are instance-specific compilation inputs. Unlike image/audio resources,
    /// only controls actually consumed by `fontFamily` remain in the Artifact fingerprint.
    used_font_controls: BTreeSet<String>,
    /// Every author binding lives in one namespace. Keeping the mutually
    /// exclusive maps together prevents new binding kinds from being omitted
    /// from component/helper/map scope save-and-restore paths.
    bindings: Bindings<'s>,
    /// Module constant initializers keyed by binding name. Symbol-aware array collection unwraps
    /// parentheses and TypeScript annotations to match sandbox evaluation.
    module_const_inits: BTreeMap<String, &'s Expression<'s>>,
    /// Nested providers merge into a preparation-only theme scope accessed by `useTheme()`, absent
    /// from SceneArtifact.
    current_theme: Option<serde_json::Value>,
    key_prefix: String,
    component_stack: Vec<String>,
    helper_stack: Vec<String>,
    list_depth: usize,
    expanded_list_items: usize,
    expanded_helper_calls: usize,
    keys: BTreeSet<String>,
    /// Scoped layout identities are compile-time-only and must be unique.
    layout_ids: BTreeSet<String>,
    diagnostics: Vec<CompilerDiagnostic>,
}
