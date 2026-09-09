//! Shared Takumi layout types and deterministic style cache.

use std::rc::Rc;
use std::str::FromStr;

use takumi_core::layout::tree::{LayoutResults, RenderNode};
use takumi_core::resources::font::Fonts;
use takumi_core::style::Style;
use takumi_core::viewport::Viewport;
use valle_motion::{MotionContext, MotionValue};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlassLayoutMaterial {
    pub clarity: f64,
    pub depth: f64,
    pub tint: valle_draw::Rgba,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlassLayoutMotion {
    pub character: crate::glass::GlassCharacter,
    pub settle_seconds: f64,
    pub intensity: f64,
    pub drive: [f64; 4],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlassLayoutEnvironment {
    pub light_direction: [f64; 2],
    pub light_elevation: f64,
    pub light_intensity: f64,
    pub light_space: crate::glass::GlassLightSpace,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlassLayoutForeground {
    pub tone: crate::glass::GlassForegroundTone,
    pub protection: f64,
    /// Conservative owner-local coverage of real foreground descendants. `None` means the
    /// subtree produces no foreground pixels at this sample.
    pub bounds: Option<valle_draw::Rect>,
    pub luma: Option<f32>,
}

/// One fully evaluated Glass shell after layout. `viewport_matrix` maps the normalized local box
/// `[0,1]²` into the Motion program viewport and includes CSS layout/camera transforms.
#[derive(Debug, Clone, PartialEq)]
pub struct GlassLayoutSurface {
    pub node_key: String,
    pub surface_id: String,
    pub field_id: Option<String>,
    pub shape: valle_draw::program::PackedGlassShapeKind,
    pub local_rect: valle_draw::Rect,
    pub viewport_matrix: [f64; 9],
    /// Material owner local pixels → Motion program viewport. Independent owners use the Glass
    /// shell transform; field members share their semantic field parent's transform.
    pub owner_to_viewport: [f64; 9],
    pub radius: f32,
    pub path_points: Vec<[f32; 2]>,
    pub presence: f64,
    pub motion: GlassLayoutMotion,
    pub material: Option<GlassLayoutMaterial>,
    pub environment: Option<GlassLayoutEnvironment>,
    pub foreground: GlassLayoutForeground,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GlassLayoutField {
    pub node_key: String,
    pub field_id: String,
    pub owner_to_viewport: [f64; 9],
    pub material: GlassLayoutMaterial,
    pub environment: GlassLayoutEnvironment,
    pub motion: GlassLayoutMotion,
    pub merge_distance: f64,
    pub member_surface_ids: Vec<String>,
}

/// Layout/evaluation sidecar. Packed programs are installed by Engine only after it has sampled
/// the causal history; the Motion emitter consumes them without interpreting optics.
#[derive(Debug, Clone, Default)]
pub struct GlassLayoutFrame {
    pub surfaces: Vec<GlassLayoutSurface>,
    pub fields: Vec<GlassLayoutField>,
    /// Scene node key → nearest field outside member foreground. Used to synthesize a single
    /// non-layout material scope around the exact painter-order run.
    pub node_fields: std::collections::HashMap<String, String>,
    pub material_programs:
        std::collections::BTreeMap<String, valle_draw::program::MotionGlassProgram>,
    pub foreground_programs:
        std::collections::BTreeMap<String, valle_draw::program::MotionGlassForegroundProgram>,
}

#[derive(Debug, Clone)]
pub(crate) enum ResolvedPaint {
    Solid(valle_draw::Rgba),
    Linear {
        start: valle_draw::Point,
        end: valle_draw::Point,
        stops: Vec<(f64, valle_draw::Rgba)>,
        spread: valle_draw::program::recording::SpreadMode,
    },
    Radial {
        center: valle_draw::Point,
        radius: f64,
        stops: Vec<(f64, valle_draw::Rgba)>,
        spread: valle_draw::program::recording::SpreadMode,
    },
    Conic {
        center: valle_draw::Point,
        start_angle: f64,
        stops: Vec<(f64, valle_draw::Rgba)>,
        spread: valle_draw::program::recording::SpreadMode,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedStroke {
    pub paint: ResolvedPaint,
    pub width: f64,
    pub dash: Option<Vec<f64>>,
    pub dash_offset: f64,
    pub cap: valle_draw::Cap,
    pub join: valle_draw::Join,
    pub miter_limit: f64,
}

#[derive(Debug, Clone)]
pub(crate) struct ClipContent {
    pub path: valle_motion::PathData,
    pub fill_rule: valle_draw::program::recording::FillRule,
}

#[derive(Debug, Clone)]
pub(crate) enum ResolvedMaskSource {
    Paint(ResolvedPaint),
    Image(String),
    Subtree(String),
}

#[derive(Debug, Clone)]
pub(crate) struct MaskContent {
    pub source: ResolvedMaskSource,
    pub mode: valle_draw::program::recording::MaskMode,
    pub rect: valle_draw::Rect,
}

/// Path leaf data consumed by the backend-neutral emitter.
#[derive(Debug, Clone)]
pub struct PathContent {
    pub verbs: Vec<valle_draw::PathVerb>,
    pub points: Vec<valle_draw::Point>,
    pub(crate) fill: Option<ResolvedPaint>,
    pub(crate) stroke: Option<ResolvedStroke>,
    pub arrow_start: Option<valle_motion::ArrowSpec>,
    pub arrow_end: Option<valle_motion::ArrowSpec>,
}

#[derive(Debug, Clone)]
pub(crate) struct BatchContent {
    pub(crate) geometry: valle_draw::program::recording::BatchGeometry,
    pub(crate) instances: Vec<valle_draw::program::recording::BatchInstance>,
}

/// Inputs shared by expression evaluation and Takumi layout for one frame.
pub struct LayoutOptions<'a> {
    #[cfg(target_arch = "wasm32")]
    pub formula_fonts: &'a crate::math_formula::FormulaFontRegistry,
    pub viewport: Viewport,
    pub fonts: &'a Fonts,
    pub styles: Option<&'a StyleCache>,
}

/// CSS declaration string to parsed Takumi style memoization.
#[derive(Default)]
pub struct StyleCache {
    inner: std::cell::RefCell<StyleCacheInner>,
}

#[derive(Default)]
struct StyleCacheInner {
    parsed: std::collections::HashMap<String, Style>,
    hits: u64,
    misses: u64,
}

impl StyleCache {
    const MAX_ENTRIES: usize = 4096;

    pub fn new() -> Self {
        Self::default()
    }

    pub fn stats(&self) -> (u64, u64) {
        let inner = self.inner.borrow();
        (inner.hits, inner.misses)
    }

    fn get_or_parse(&self, declarations: &str) -> Result<Style, String> {
        {
            let mut inner = self.inner.borrow_mut();
            if let Some(style) = inner.parsed.get(declarations).cloned() {
                inner.hits += 1;
                return Ok(style);
            }
            inner.misses += 1;
        }
        let style = Style::from_str(declarations).map_err(|error| error.to_string())?;
        let mut inner = self.inner.borrow_mut();
        if inner.parsed.len() >= Self::MAX_ENTRIES {
            inner.parsed.clear();
        }
        inner.parsed.insert(declarations.to_owned(), style.clone());
        Ok(style)
    }
}

pub(crate) fn parse_style(cache: Option<&StyleCache>, declarations: &str) -> Result<Style, String> {
    match cache {
        Some(cache) => cache.get_or_parse(declarations),
        None => Style::from_str(declarations).map_err(|error| error.to_string()),
    }
}

impl LayoutOptions<'_> {
    /// Takumi's independent CSS animation clock is never advanced. MotionContext is the only clock.
    pub const TIME_MS: u64 = 0;
}

/// Resolved paint parameters for one text unit. `start` and `end` are node-local byte offsets,
/// matching the source addresses in ProgramRecording.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResolvedUnit {
    pub start: u32,
    pub end: u32,
    pub opacity: Option<f64>,
    pub translate: Option<valle_draw::Point>,
    pub scale: Option<valle_draw::Point>,
    pub rotate: Option<f64>,
    pub color: Option<valle_draw::Rgba>,
}

pub struct LayoutTree {
    pub root: RenderNode,
    pub layout: Rc<LayoutResults>,
    pub values: Vec<MotionValue>,
    pub keys: Rc<std::collections::HashMap<u64, String>>,
    /// Scene key to evaluated unit parameters; emission reads these without reevaluation.
    pub units: Rc<std::collections::HashMap<String, Vec<ResolvedUnit>>>,
    /// Render-tree path to Scene key, using the same coordinates as `NodePaint::path`. Inline text
    /// may not have a layout NodeId, so the layout-key map cannot identify it.
    pub render_keys: Rc<std::collections::HashMap<Vec<usize>, String>>,
    /// Original text by Scene key. Emission disables source addressing when CSS transformations
    /// change the text and invalidate node-local offsets.
    pub node_texts: Rc<std::collections::HashMap<String, String>>,
    pub paths: Rc<std::collections::HashMap<String, PathContent>>,
    pub(crate) batches: Rc<std::collections::HashMap<String, BatchContent>>,
    /// Evaluated, finite text paths by Scene key. Emission only samples arc lengths, points, and
    /// tangents.
    pub text_paths: Rc<std::collections::HashMap<String, valle_motion::geometry::PathData>>,
    pub(crate) clips: Rc<std::collections::HashMap<String, ClipContent>>,
    pub(crate) masks: Rc<std::collections::HashMap<String, MaskContent>>,
    /// Resolved video content by Scene key, including the source time for this frame.
    pub(crate) videos: Rc<std::collections::HashMap<String, VideoContent>>,
    /// Scene key to node-local advanced filters resolved for this frame.
    pub(crate) advanced_filters:
        Rc<std::collections::HashMap<String, Vec<valle_draw::program::recording::FilterOp>>>,
    /// Scene key → advanced filters applied only to the pixels behind the node.
    pub(crate) backdrop_advanced_filters:
        Rc<std::collections::HashMap<String, Vec<valle_draw::program::recording::FilterOp>>>,
    /// Scene key → frame-evaluated ShaderLayer payload. Package bytes remain host-owned; this
    /// bridge carries only the locked program pin and typed values into ProgramRecording v10.
    pub(crate) shaders: Rc<std::collections::HashMap<String, ResolvedShaderLayer>>,
    /// Scene key → one fully evaluated 3D request. Domain data stays in this Rust sidecar; the
    /// ProgramRecording receives only `provider_key` and target dimensions.
    pub(crate) scene3d: Rc<std::collections::HashMap<String, Scene3DRequest>>,
    /// Scene key → 2.5D layer extras that Takumi CSS cannot express.
    pub(crate) layer_fx: Rc<std::collections::HashMap<String, LayerFx>>,
    /// Scene key → one flattened CSS 3D plane in absolute viewport coordinates.
    /// Ordered parent/child 4x4 transforms are resolved before paint; emit lowers
    /// the final quad to the existing homography command.
    pub(crate) css_3d_planes: Rc<std::collections::HashMap<String, Css3dPlane>>,
    pub(crate) formulas:
        Rc<std::collections::HashMap<String, crate::math_formula::FormulaFragment>>,
    pub glass: GlassLayoutFrame,
}

/// Per-node paint extras: perspective card, desk contact shadow, paper grain.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct LayerFx {
    pub rotate_x_deg: f64,
    pub rotate_y_deg: f64,
    /// Authored `perspective`. `None` means [`PerspectiveLength::DEFAULT_PX`].
    pub perspective: Option<PerspectiveLength>,
    pub paper_grain: f64,
    pub contact_shadow: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Css3dPlane {
    /// Absolute viewport-space TL/TR/BR/BL after ordered 4x4 transform + perspective.
    pub quad: [valle_draw::Point; 4],
    /// Positive z points toward the viewer. Used to establish deterministic plane order.
    pub depth: f64,
    pub hidden: bool,
    /// Preserve-3d containers carry depth for sibling painter-order without applying a
    /// second homography around their already-projected descendant planes.
    pub project: bool,
}

/// CSS `perspective` length, stored until emit can resolve it against Takumi's
/// computed [`takumi_core::style::SizingContext`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum PerspectiveLength {
    Px(f64),
    Rem(f64),
    Em(f64),
    Vw(f64),
    Vh(f64),
}

impl PerspectiveLength {
    /// CSS-pixel default when `rotateX`/`rotateY` is set without `perspective`.
    /// Must go through [`Self::to_px`] so it matches an authored `1200`.
    pub(crate) const DEFAULT_PX: Self = Self::Px(1200.0);

    /// Resolve to a positive device-pixel distance. Percent is rejected before
    /// this is constructed; missing viewport axes fail closed rather than
    /// collapsing the rotation.
    pub(crate) fn to_px(self, sizing: &takumi_core::style::SizingContext) -> Result<f64, String> {
        let dpr = if sizing.viewport.device_pixel_ratio > 0.0 {
            f64::from(sizing.viewport.device_pixel_ratio)
        } else {
            1.0
        };
        let px = match self {
            Self::Px(value) => value * dpr,
            Self::Rem(value) => {
                let rem = sizing
                    .root_font_size
                    .map(f64::from)
                    .unwrap_or(f64::from(sizing.viewport.font_size) * dpr);
                value * rem
            }
            Self::Em(value) => value * f64::from(sizing.font_size),
            Self::Vw(value) => {
                let width = sizing
                    .viewport
                    .size
                    .width
                    .ok_or_else(|| "perspective vw requires a viewport width".to_owned())?;
                value * f64::from(width) / 100.0
            }
            Self::Vh(value) => {
                let height = sizing
                    .viewport
                    .size
                    .height
                    .ok_or_else(|| "perspective vh requires a viewport height".to_owned())?;
                value * f64::from(height) / 100.0
            }
        };
        if px.is_finite() && px > 0.0 {
            Ok(px)
        } else {
            Err("perspective must resolve to a positive length".into())
        }
    }
}

impl LayerFx {
    pub(crate) fn is_empty(self) -> bool {
        self.rotate_x_deg.abs() < 1e-6
            && self.rotate_y_deg.abs() < 1e-6
            && self.paper_grain <= 0.0
            && self.contact_shadow <= 0.0
    }
}

impl LayoutTree {
    pub fn scene3d_requests(&self) -> impl Iterator<Item = &Scene3DRequest> {
        self.scene3d.values()
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Scene3DRequest {
    pub provider_key: String,
    pub scene: valle_motion::scene3d::Scene3DSpec,
    pub frame: valle_motion::scene3d::Frame3DState,
}

/// Complete random-access Scene3D raster request carried by the Product Compositor resource
/// contract. Static topology and evaluated state are explicit; executors never consult a clock or
/// a previous frame.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Scene3DFrameRequest {
    pub provider_key: String,
    pub width: u32,
    pub height: u32,
    /// Scene control name to immutable asset content digest. Host paths and decoded objects never
    /// enter this map, but two clip instances with different bindings cannot alias a raster.
    pub resource_bindings: std::collections::BTreeMap<String, crate::ContentDigest>,
    pub scene: valle_motion::scene3d::Scene3DSpec,
    pub frame: valle_motion::scene3d::Frame3DState,
}

impl Scene3DFrameRequest {
    pub fn new(
        request: &Scene3DRequest,
        width: u32,
        height: u32,
        resource_bindings: std::collections::BTreeMap<String, crate::ContentDigest>,
    ) -> Result<Self, crate::CanonicalError> {
        let value = Self {
            provider_key: request.provider_key.clone(),
            width,
            height,
            resource_bindings,
            scene: request.scene.clone(),
            frame: request.frame.clone(),
        };
        // Canonicalization also rejects every non-finite number before this request can become a
        // resource identity. Scene/frame domain validation already ran during layout.
        value.canonical_bytes()?;
        Ok(value)
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, crate::CanonicalError> {
        crate::canonical_bytes(self)
    }

    pub fn content_digest(&self) -> Result<crate::ContentDigest, crate::CanonicalError> {
        use sha2::{Digest, Sha256};
        let mut hash = Sha256::new();
        hash.update(b"valle-scene3d-frame-request-v1\0");
        hash.update(self.canonical_bytes()?);
        Ok(crate::ContentDigest::from_bytes(hash.finalize().into()))
    }

    pub fn topology_digest(&self) -> Result<crate::ContentDigest, crate::CanonicalError> {
        use sha2::{Digest, Sha256};
        let mut hash = Sha256::new();
        hash.update(b"valle-scene3d-topology-v1\0");
        hash.update(crate::canonical_bytes(&self.scene)?);
        Ok(crate::ContentDigest::from_bytes(hash.finalize().into()))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ResolvedShaderLayer {
    pub(crate) program: valle_draw::program::recording::ShaderProgram,
    pub(crate) uniforms: Vec<valle_draw::program::recording::ShaderUniformBinding>,
    pub(crate) inputs: Vec<(String, String)>,
}

/// Frame request for one video node.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct VideoContent {
    /// `asset://<video control name>`, resolved through the same asset table as images.
    pub(crate) asset: String,
    pub(crate) source_time_s: f64,
}

// Keep the type visible in rustdoc for the single-clock invariant above.
const _: Option<MotionContext> = None;
