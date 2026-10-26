//! Stateful WASM host for the staged Product Compositor API.
//!
//! One pinned Engine render is opened atomically, then every frame crosses the JS/WASM boundary
//! as three bounded binary packets: resource requests, plan template and bindings. Platform
//! objects never enter the semantic render; JavaScript keeps them in its generation-scoped
//! object table.

use std::{
    collections::{BTreeMap, btree_map::Entry},
    sync::Arc,
};

use serde::Serialize;
use wasm_bindgen::prelude::*;

use crate::fixed_package::{
    COMMON_PROFILE_KEY, canonical_fixed_execution_profile, fixed_package_files,
    open_verified_fixed_package,
};
use crate::{
    compositor::{graph::GraphCapability, lower::BackendCapabilities},
    frame::{RenderQuality, RenderSpec},
    prepare::{DeviceRect, PreparedLayer, PreparedSource, PreparedVisualItem},
    product::{BoundTicket, EngineRender, FrameCompiler, LoweredTicket, PreparedTicket},
    render::{AudioPcmMixError, CompiledExecutionResourceKind, SampleRange},
    resource::{
        ContentDigest, Extent2d, ExternalGeneration, ExternalPixelLayout, OutputBackground,
        OutputSpec, TextureFormat, TextureUsage,
    },
};

const MAX_SCENE3D_PREPARED_CACHE: usize = 8;
const MAX_SCENE3D_FRAME_CACHE: usize = 64;
const MAX_WEB_AUDIO_BLOCK_SAMPLES: i64 = 8_192;
const CANVAS_KIT_EXTERNAL_PIXEL_LAYOUTS: [ExternalPixelLayout; 2] =
    [ExternalPixelLayout::Rgba8, ExternalPixelLayout::Rgba16Float];

enum Ticket {
    Prepared(PreparedTicket),
    Lowered(LoweredTicket),
    Bound(BoundTicket),
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProductHitRect<'a> {
    clip_id: &'a str,
    kind: &'static str,
    rect: ProductRect,
}

#[derive(Serialize)]
struct ProductRect {
    x: i32,
    y: i32,
    w: u32,
    h: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProductScene3dPlacement {
    clip_id: String,
    content_hash: ContentDigest,
    bounds: valle_draw::Rect,
    device_from_local: [f64; 9],
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct Scene3dTextureNeed {
    digest: ContentDigest,
    role: valle_motion::scene3d::TextureRole,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Scene3dResourceNeeds {
    models: Vec<ContentDigest>,
    textures: Vec<Scene3dTextureNeed>,
    environments: Vec<ContentDigest>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CompiledExecutionResourcePacket<'a> {
    resource_id: &'a str,
    kind: CompiledExecutionResourceKind,
    content_digest: &'a ContentDigest,
    abi_digest: Option<&'a ContentDigest>,
}

struct DecodedAudioPcm {
    start: i64,
    left: Vec<f32>,
    right: Vec<f32>, // Empty for mono; the shared mixer duplicates its left channel.
}

struct GeneratedScene3dFrame {
    raster: valle_motion::scene3d::RasterFrame,
    metadata: valle_motion::scene3d::Scene3DFrameMetadata,
}

impl From<DeviceRect> for ProductRect {
    fn from(value: DeviceRect) -> Self {
        Self {
            x: value.x,
            y: value.y,
            w: value.width,
            h: value.height,
        }
    }
}

impl Ticket {
    fn prepared(&self) -> &crate::prepare::PrepareOutput {
        match self {
            Self::Prepared(ticket) => ticket.prepared(),
            Self::Lowered(ticket) => ticket.prepared(),
            Self::Bound(ticket) => ticket.prepared(),
        }
    }
}

/// One Web product engine. Opening a fixed package atomically discards old
/// tickets and deterministic frame caches; all semantic execution payloads
/// arrive inside that render's verified binding bundle.
#[wasm_bindgen]
pub struct ProductEngine {
    scene3d_models: BTreeMap<ContentDigest, Arc<valle_motion::scene3d::AdmittedModel>>,
    scene3d_environments: BTreeMap<ContentDigest, Arc<valle_motion::scene3d::EnvironmentAsset>>,
    scene3d_textures: BTreeMap<
        (ContentDigest, valle_motion::scene3d::TextureRole),
        Arc<valle_motion::scene3d::MaterialImage>,
    >,
    scene3d_prepared: BTreeMap<
        valle_motion::scene3d::ScenePrepareCacheKey,
        Arc<valle_motion::scene3d::PreparedScene>,
    >,
    scene3d_frames: BTreeMap<ContentDigest, GeneratedScene3dFrame>,
    audio_pcm: BTreeMap<ContentDigest, Vec<DecodedAudioPcm>>,
    active_render: Option<EngineRender>,
    compiler: Option<FrameCompiler>,
    tickets: BTreeMap<u32, Ticket>,
    next_ticket: u32,
    realtime_geometry: bool,
    delivery_output: bool,
    motion_kernel: crate::prepare::motion_kernel::MotionKernel,
}

#[wasm_bindgen]
impl ProductEngine {
    /// UI clock for a receipt already admitted by the worker. Uses the same rounding and range
    /// checks as CompiledCanvas without retaining another compiled Timeline on the UI thread.
    pub fn quantize_frame_seconds(
        &self,
        seconds: f64,
        numerator: u32,
        denominator: u32,
        count: i64,
    ) -> Result<i64, JsError> {
        if numerator == 0 || denominator == 0 || count <= 0 {
            return Err(JsError::new("invalid frame clock"));
        }
        crate::render::quantize_seconds_clock(
            seconds,
            f64::from(numerator) / f64::from(denominator),
            count,
            false,
            "frame",
        )
        .map_err(|error| js_error("frame_clock", error))
    }

    pub fn quantize_sample_seconds(
        &self,
        seconds: f64,
        sample_rate: u32,
        count: i64,
    ) -> Result<i64, JsError> {
        if sample_rate == 0 || count <= 0 {
            return Err(JsError::new("invalid sample clock"));
        }
        crate::render::quantize_seconds_clock(
            seconds,
            f64::from(sample_rate),
            count,
            false,
            "sample",
        )
        .map_err(|error| js_error("sample_clock", error))
    }
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self {
            scene3d_models: BTreeMap::new(),
            scene3d_environments: BTreeMap::new(),
            scene3d_textures: BTreeMap::new(),
            scene3d_prepared: BTreeMap::new(),
            scene3d_frames: BTreeMap::new(),
            audio_pcm: BTreeMap::new(),
            active_render: None,
            compiler: None,
            tickets: BTreeMap::new(),
            next_ticket: 1,
            realtime_geometry: false,
            delivery_output: false,
            motion_kernel: Default::default(),
        }
    }

    pub fn set_delivery_output(&mut self, enabled: bool) {
        self.delivery_output = enabled;
    }

    /// CPU preview output using the shared color math. CanvasKit owns F16 storage and final
    /// delivery quantization; this boundary only converts packed premultiplied float pixels.
    pub fn transform_srgb_preview_pixels(
        &self,
        pixels: &mut [f32],
        opaque: bool,
    ) -> Result<(), JsError> {
        crate::compositor::reference::transform_srgb_preview_pixels(pixels, opaque)
            .map_err(|error| js_error("srgb_preview_output", error))
    }

    pub fn transform_srgb_preview_f16(
        &self,
        pixels: &mut [u8],
        opaque: bool,
    ) -> Result<(), JsError> {
        crate::compositor::reference::transform_canvaskit_srgb_preview_f16(pixels, opaque)
            .map_err(|error| js_error("srgb_preview_output", error))
    }

    /// GPU host recipe for the same padded F16 pyramid as Native. Only pass dimensions,
    /// uniforms and resource lifetimes cross WASM; source pixels stay on the GPU.
    #[expect(
        clippy::too_many_arguments,
        reason = "Expose dimensions and uniforms as scalar arguments at the Wasm boundary."
    )]
    pub fn plan_pyramid_filter(
        &self,
        input_width: u32,
        input_height: u32,
        output_width: u32,
        output_height: u32,
        offset_x: i32,
        offset_y: i32,
        filter_json: &str,
    ) -> Result<Vec<f64>, JsError> {
        use crate::compositor::bloom::{PyramidGraph, PyramidPassKind, PyramidRegion};
        use valle_draw::program::Filter;
        let filter: Filter = parse_json(filter_json, "GPU pyramid filter")?;
        let (radius, spread_radius, parameters, inactive) = match filter {
            Filter::Glow {
                color,
                intensity,
                radius,
            } => {
                let color = color.to_working();
                (
                    radius,
                    radius * 2.0,
                    [color.red, color.green, color.blue, color.alpha, intensity],
                    intensity == 0.0 || color.alpha == 0.0,
                )
            }
            Filter::Bloom {
                threshold,
                knee,
                intensity,
                radius,
            } => {
                if !threshold.is_finite()
                    || !(0.0..=1.0).contains(&threshold)
                    || !knee.is_finite()
                    || !(0.0..=1.0).contains(&knee)
                {
                    return Err(JsError::new("[gpu_pyramid] invalid bloom parameters"));
                }
                (
                    radius,
                    radius,
                    [threshold, knee, 0.0, 0.0, intensity],
                    intensity == 0.0,
                )
            }
            _ => {
                return Err(JsError::new(
                    "[gpu_pyramid] only bloom and glow have pyramid recipes",
                ));
            }
        };
        if !radius.is_finite()
            || radius < 0.0
            || !parameters[4].is_finite()
            || !(0.0..=4.0).contains(&parameters[4])
        {
            return Err(JsError::new("[gpu_pyramid] invalid radius or intensity"));
        }
        if radius == 0.0 || inactive {
            return Ok(Vec::new());
        }
        let region = PyramidRegion::new(
            [input_width, input_height],
            [output_width, output_height],
            [offset_x, offset_y],
            radius,
            spread_radius,
        )
        .map_err(|e| js_error("gpu_pyramid_region", e))?;
        let graph = PyramidGraph::new(region.size[0], region.size[1], radius, spread_radius)
            .map_err(|e| js_error("gpu_pyramid_graph", e))?;
        // Header: padded origin/extent/input offset, spread, output/resource IDs,
        // five color/threshold/intensity uniforms and pass count. Each pass contains
        // kind/input/fine/weight/output/extent/retirement count followed by retired IDs.
        let mut recipe = vec![
            f64::from(region.origin[0]),
            f64::from(region.origin[1]),
            f64::from(region.size[0]),
            f64::from(region.size[1]),
            f64::from(region.input_offset[0]),
            f64::from(region.input_offset[1]),
            graph.spread(),
            graph.output() as f64,
            graph.resource_count() as f64,
        ];
        recipe.extend(parameters.into_iter().map(f64::from));
        recipe.push(graph.passes().len() as f64);
        for pass in graph.passes() {
            let (kind, input, fine, weight) = match pass.kind() {
                PyramidPassKind::Downsample { input } => (0.0, input, -1.0, 0.0),
                PyramidPassKind::Blur { input } => (1.0, input, -1.0, 0.0),
                PyramidPassKind::Upsample {
                    coarse,
                    fine,
                    local_weight,
                } => (2.0, coarse, fine.map_or(-1.0, |id| id as f64), local_weight),
            };
            recipe.extend([
                kind,
                input as f64,
                fine,
                weight,
                pass.output() as f64,
                f64::from(pass.width()),
                f64::from(pass.height()),
                pass.retire_after().len() as f64,
            ]);
            recipe.extend(pass.retire_after().iter().map(|&id| id as f64));
        }
        Ok(recipe)
    }

    /// Apply the same working-linear F16 bloom pyramid used by the Native executor.
    #[expect(
        clippy::too_many_arguments,
        reason = "Expose dimensions and uniforms as scalar arguments at the Wasm boundary."
    )]
    pub fn apply_bloom_f16(
        &self,
        input: &[u8],
        input_width: u32,
        input_height: u32,
        output_width: u32,
        output_height: u32,
        offset_x: i32,
        offset_y: i32,
        threshold: f32,
        knee: f32,
        intensity: f32,
        radius: f32,
    ) -> Result<Vec<u8>, JsError> {
        crate::compositor::bloom::apply_bloom_f16(
            input,
            input_width,
            input_height,
            output_width,
            output_height,
            [offset_x, offset_y],
            crate::compositor::bloom::BloomParams {
                threshold,
                knee,
                intensity,
                radius,
            },
        )
        .map_err(|error| js_error("bloom", error))
    }

    /// Render node-local colored glow using the shared F16 pyramid.
    #[expect(
        clippy::too_many_arguments,
        reason = "Expose dimensions and uniforms as scalar arguments at the Wasm boundary."
    )]
    pub fn apply_glow_f16(
        &self,
        input: &[u8],
        input_width: u32,
        input_height: u32,
        output_width: u32,
        output_height: u32,
        offset_x: i32,
        offset_y: i32,
        color_json: &str,
        intensity: f32,
        radius: f32,
    ) -> Result<Vec<u8>, JsError> {
        let color: valle_draw::program::AuthorColor = parse_json(color_json, "Glow color")?;
        let color = color.to_working();
        crate::compositor::bloom::apply_glow_f16(
            input,
            input_width,
            input_height,
            output_width,
            output_height,
            [offset_x, offset_y],
            crate::compositor::bloom::GlowParams {
                color: [color.red, color.green, color.blue, color.alpha],
                intensity,
                radius,
            },
        )
        .map_err(|error| js_error("glow", error))
    }

    /// Shared ROI support and geometric pass spacings for Native and CanvasKit.
    /// This sends a few numbers per pass; rendered pixels remain GPU-resident.
    #[expect(
        clippy::too_many_arguments,
        reason = "Expose dimensions and uniforms as scalar arguments at the Wasm boundary."
    )]
    pub fn plan_radial_blur(
        &self,
        amount: f32,
        input_x: i32,
        input_y: i32,
        input_width: u32,
        input_height: u32,
        output_x: i32,
        output_y: i32,
        output_width: u32,
        output_height: u32,
    ) -> Result<Vec<f64>, JsError> {
        let plan = crate::compositor::radial::radial_blur_plan(
            amount,
            [input_x, input_y],
            [input_width, input_height],
            [output_x, output_y],
            [output_width, output_height],
        )
        .map_err(|error| js_error("radial_blur_plan", error))?;
        Ok(plan
            .iter()
            .flat_map(|p| {
                [
                    f64::from(p.origin[0]),
                    f64::from(p.origin[1]),
                    f64::from(p.size[0]),
                    f64::from(p.size[1]),
                    f64::from(p.fraction),
                ]
            })
            .collect())
    }

    /// Blur along rays from a local-center filter mapped into device space.
    #[expect(
        clippy::too_many_arguments,
        reason = "Expose dimensions and uniforms as scalar arguments at the Wasm boundary."
    )]
    pub fn apply_radial_blur_f16(
        &self,
        input: &[u8],
        input_width: u32,
        input_height: u32,
        output_width: u32,
        output_height: u32,
        offset_x: i32,
        offset_y: i32,
        origin_x: i32,
        origin_y: i32,
        center_x: f32,
        center_y: f32,
        amount: f32,
    ) -> Result<Vec<u8>, JsError> {
        crate::compositor::radial::apply_radial_blur_f16(
            input,
            input_width,
            input_height,
            output_width,
            output_height,
            [offset_x, offset_y],
            [origin_x, origin_y],
            crate::compositor::radial::RadialBlurParams {
                center: [center_x, center_y],
                amount,
            },
        )
        .map_err(|error| js_error("radial_blur", error))
    }

    /// Stable film grain over a working-linear F16 image ROI.
    #[expect(
        clippy::too_many_arguments,
        reason = "Expose dimensions and uniforms as scalar arguments at the Wasm boundary."
    )]
    pub fn apply_film_grain_f16(
        &self,
        input: &[u8],
        input_width: u32,
        input_height: u32,
        output_width: u32,
        output_height: u32,
        offset_x: i32,
        offset_y: i32,
        origin_x: i32,
        origin_y: i32,
        seed: u32,
        amount: f32,
        size: f32,
    ) -> Result<Vec<u8>, JsError> {
        crate::compositor::film::apply_film_grain_f16(
            input,
            input_width,
            input_height,
            output_width,
            output_height,
            [offset_x, offset_y],
            [origin_x, origin_y],
            crate::compositor::film::FilmGrainParams { seed, amount, size },
        )
        .map_err(|error| js_error("film_grain", error))
    }

    /// Skia's Perlin noise tables for a noise displacement seed, as the shared kernel reads them:
    /// the opaque RGBA8 pixels of a `compositor::perlin::PERLIN_IMAGE_SIZE` image.
    pub fn perlin_noise_tables(&self, seed: u32) -> Vec<u8> {
        crate::compositor::perlin::PerlinTables::new(seed as f32).image()
    }

    /// Distort a fixed device-space source frame without changing its output bounds.
    #[expect(
        clippy::too_many_arguments,
        reason = "Expose dimensions and uniforms as scalar arguments at the Wasm boundary."
    )]
    pub fn apply_lens_distortion_f16(
        &self,
        input: &[u8],
        input_width: u32,
        input_height: u32,
        output_width: u32,
        output_height: u32,
        offset_x: i32,
        offset_y: i32,
        origin_x: i32,
        origin_y: i32,
        k1: f32,
        k2: f32,
        frame_x: f32,
        frame_y: f32,
        frame_width: f32,
        frame_height: f32,
    ) -> Result<Vec<u8>, JsError> {
        crate::compositor::lens::apply_lens_distortion_f16(
            input,
            input_width,
            input_height,
            output_width,
            output_height,
            [offset_x, offset_y],
            [origin_x, origin_y],
            crate::compositor::lens::LensDistortionParams {
                k1,
                k2,
                frame: [frame_x, frame_y, frame_width, frame_height],
            },
        )
        .map_err(|error| js_error("lens_distortion", error))
    }

    /// Clear the active render, frame tickets and host-side fulfillment caches. Backend objects
    /// remain JS-owned and must be retired by the host under the matching external generation.
    pub fn reset(&mut self) {
        self.motion_kernel = Default::default();
        self.scene3d_models.clear();
        self.scene3d_environments.clear();
        self.scene3d_textures.clear();
        self.scene3d_prepared.clear();
        self.scene3d_frames.clear();
        self.audio_pcm.clear();
        self.active_render = None;
        self.compiler = None;
        self.tickets.clear();
        self.next_ticket = 1;
    }

    pub fn set_realtime_geometry(&mut self, enabled: bool) {
        self.realtime_geometry = enabled;
    }

    /// Atomically open one verified fixed package. Its Timeline, resource
    /// manifest and binding bundle cross the boundary together; no mutable
    /// authoring registry is consulted after this call.
    pub fn open_fixed_package(
        &mut self,
        fixed_package_manifest_json: &str,
        timeline_json: &str,
        manifest_json: &str,
        verified_binding_bundle_json: &str,
        resource_bytes: &[u8],
    ) -> Result<String, JsError> {
        let opened = open_product_fixed_package(
            fixed_package_manifest_json,
            timeline_json,
            manifest_json,
            verified_binding_bundle_json,
            resource_bytes,
        )
        .map_err(|error| JsError::new(&error))?;
        let render = opened.engine_render();
        self.compiler = Some(render.frame_compiler());
        self.audio_pcm.clear();
        self.active_render = Some(render);
        self.tickets.clear();
        self.next_ticket = 1;
        Ok(opened.receipt_json().to_owned())
    }

    /// Quantize a non-negative composition second to a frame in the active
    /// half-open render. The caller must name that render explicitly.
    pub fn frame_at_seconds(&self, render_id: &str, seconds: f64) -> Result<i64, JsError> {
        let render = self.active_render(render_id)?;
        let canvas = render.compiled().canvas();
        canvas
            .frame_at_seconds(seconds)
            .map(|frame| frame.index())
            .map_err(|error| js_error("frame_clock", error))
    }

    /// Quantize a non-negative composition second to an output sample in the
    /// active half-open render.
    pub fn sample_at_seconds(&self, render_id: &str, seconds: f64) -> Result<i64, JsError> {
        let render = self.active_render(render_id)?;
        let canvas = render.compiled().canvas();
        canvas
            .sample_at_seconds(seconds)
            .map_err(|error| js_error("sample_clock", error))
    }

    /// Read-only compiled AudioProgram. This serializes the immutable program
    /// opened above and never reparses the authoring Timeline.
    pub fn audio_program_json(&self, render_id: &str) -> Result<String, JsError> {
        let render = self.active_render(render_id)?;
        serde_json::to_string(render.compiled().audio())
            .map_err(|error| js_error("audio_program", error))
    }

    /// Read-only, render-local preparation cache counters, independent of executor caches.
    pub fn motion_instance_cache_stats_json(&self, render_id: &str) -> Result<String, JsError> {
        let render = self.active_render(render_id)?;
        serde_json::to_string(&render.compiled().motion_instance_cache_stats())
            .map_err(|error| js_error("motion_instance_cache_stats", error))
    }

    /// Enumerates fixed backend payloads carried by the active compiled
    /// render. The returned identities are the only valid inputs to
    /// `compiled_resource_bytes`.
    pub fn compiled_execution_resources_json(&self, render_id: &str) -> Result<String, JsError> {
        let render = self.active_render(render_id)?;
        let resources = render
            .compiled()
            .execution_resources()
            .into_iter()
            .map(|resource| CompiledExecutionResourcePacket {
                resource_id: resource.resource_id(),
                kind: resource.kind(),
                content_digest: resource.content_digest(),
                abi_digest: resource.abi_digest(),
            })
            .collect::<Vec<_>>();
        serde_json::to_string(&resources)
            .map_err(|error| js_error("compiled_execution_resources", error))
    }

    /// Returns immutable font bytes or engine-generated SkSL from the
    /// named render. No Host-side semantic registration or shader lowering
    /// is permitted.
    pub fn compiled_resource_bytes(
        &self,
        render_id: &str,
        kind: &str,
        content_digest: &str,
        abi_digest: Option<String>,
    ) -> Result<Vec<u8>, JsError> {
        let render = self.active_render(render_id)?;
        let kind = match kind {
            "font-bytes" => CompiledExecutionResourceKind::FontBytes,
            "model3d-bytes" => CompiledExecutionResourceKind::Model3dBytes,
            "environment-bytes" => CompiledExecutionResourceKind::EnvironmentBytes,
            "runtime-shader" => CompiledExecutionResourceKind::RuntimeShader,
            _ => return Err(JsError::new("[compiled_resource_kind] unsupported kind")),
        };
        let content_digest = ContentDigest::parse(content_digest)
            .map_err(|error| js_error("compiled_resource_digest", error))?;
        let abi_digest = abi_digest
            .as_deref()
            .map(ContentDigest::parse)
            .transpose()
            .map_err(|error| js_error("compiled_resource_abi_digest", error))?;
        render
            .compiled()
            .execution_resource(
                render.render_id(),
                kind,
                &content_digest,
                abi_digest.as_ref(),
            )
            .map(|resource| resource.bytes().to_vec())
            .map_err(|error| js_error("compiled_resource", error))
    }

    /// Small fulfillment list: one record per active source, never one per sample.
    pub fn audio_sources_json(
        &self,
        render_id: &str,
        start_sample: i64,
        end_sample: i64,
    ) -> Result<String, JsError> {
        let range = self.audio_range(render_id, start_sample, end_sample)?;
        let sources = self
            .active_render(render_id)?
            .compiled()
            .audio()
            .source_requirements(range)
            .map_err(|error| js_error("audio_sources", error))?;
        serde_json::to_string(&sources).map_err(|error| js_error("audio_sources", error))
    }

    pub fn has_audio_pcm(&self, render_id: &str, digest: &str) -> Result<bool, JsError> {
        self.active_render(render_id)?;
        let digest =
            ContentDigest::parse(digest).map_err(|error| js_error("audio_digest", error))?;
        Ok(self.audio_pcm.contains_key(&digest))
    }

    pub fn audio_windows_json(
        &self,
        render_id: &str,
        start_sample: i64,
        end_sample: i64,
    ) -> Result<String, JsError> {
        let range = self.audio_range(render_id, start_sample, end_sample)?;
        let windows = self
            .active_render(render_id)?
            .compiled()
            .audio()
            .source_windows(range)
            .map_err(|error| js_error("audio_windows", error))?;
        serde_json::to_string(&windows).map_err(|error| js_error("audio_windows", error))
    }

    pub fn clear_audio_pcm(&mut self, render_id: &str) -> Result<(), JsError> {
        self.active_render(render_id)?;
        self.audio_pcm.clear();
        Ok(())
    }

    /// Copy decoded planes once per admitted resource. Repeated blocks reuse them; opening a
    /// different render clears this cache, and mixing retires sources no longer in the window.
    pub fn register_audio_pcm(
        &mut self,
        render_id: &str,
        digest: &str,
        left: Vec<f32>,
        right: Vec<f32>,
    ) -> Result<(), JsError> {
        let key = ContentDigest::parse(digest).map_err(|error| js_error("audio_digest", error))?;
        self.register_audio_pcm_window(render_id, digest, 0, left, right)?;
        self.audio_pcm
            .get_mut(&key)
            .unwrap()
            .retain(|window| window.start == 0);
        Ok(())
    }

    pub fn register_audio_pcm_window(
        &mut self,
        render_id: &str,
        digest: &str,
        start: i64,
        left: Vec<f32>,
        right: Vec<f32>,
    ) -> Result<(), JsError> {
        let render = self.active_render(render_id)?;
        let digest =
            ContentDigest::parse(digest).map_err(|error| js_error("audio_digest", error))?;
        let sources = render
            .compiled()
            .audio()
            .source_requirements(
                SampleRange::new(0, render.compiled().canvas().sample_count())
                    .map_err(|error| js_error("audio_pcm", error))?,
            )
            .map_err(|error| js_error("audio_pcm", error))?;
        let expected = sources
            .iter()
            .find(|source| source.digest == digest)
            .ok_or_else(|| JsError::new("[audio_pcm] resource is not admitted audio"))?;
        if start < 0
            || start.checked_add(left.len() as i64).is_none()
            || left.is_empty()
            || (expected.source_channels == 1 && !right.is_empty())
            || (expected.source_channels == 2 && left.len() != right.len())
        {
            return Err(JsError::new(
                "[audio_pcm] decoded channel layout does not match admitted audio",
            ));
        }
        let windows = self.audio_pcm.entry(digest).or_default();
        windows.retain(|window| window.start != start);
        windows.push(DecodedAudioPcm { start, left, right });
        Ok(())
    }

    /// Execute the compact program and return only interleaved stereo f32 PCM.
    pub fn mix_audio_pcm(
        &mut self,
        render_id: &str,
        start_sample: i64,
        end_sample: i64,
    ) -> Result<Vec<f32>, JsError> {
        let range = self.audio_range(render_id, start_sample, end_sample)?;
        let render = self.active_render(render_id)?;
        let program = render.compiled().audio();
        let sources = program
            .source_requirements(range)
            .map_err(|error| js_error("audio_mix", error))?;
        let mut decoded = vec![None; render.compiled().sources().len()];
        for source in &sources {
            let pcm = self.audio_pcm.get(&source.digest).ok_or_else(|| {
                js_error(
                    "audio_mix",
                    AudioPcmMixError::MissingSource {
                        source_index: source.source_index,
                    },
                )
            })?;
            if pcm
                .iter()
                .any(|window| (source.source_channels == 1) != window.right.is_empty())
            {
                return Err(JsError::new(
                    "[audio_mix] cached PCM channel layout does not match the source",
                ));
            }
            decoded[source.source_index as usize] = Some(pcm);
        }
        let output = program
            .mix_pcm(range, |source_index, sample| {
                let windows = decoded[source_index as usize]
                    .ok_or(AudioPcmMixError::MissingSource { source_index })?;
                let pcm = windows
                    .iter()
                    .find(|window| {
                        sample >= window.start && sample - window.start < window.left.len() as i64
                    })
                    .ok_or(AudioPcmMixError::SourceSampleOutOfRange {
                        source_index,
                        sample,
                    })?;
                let index = usize::try_from(sample - pcm.start).map_err(|_| {
                    AudioPcmMixError::SourceSampleOutOfRange {
                        source_index,
                        sample,
                    }
                })?;
                let left =
                    *pcm.left
                        .get(index)
                        .ok_or(AudioPcmMixError::SourceSampleOutOfRange {
                            source_index,
                            sample,
                        })?;
                let right = if pcm.right.is_empty() {
                    left
                } else {
                    *pcm.right
                        .get(index)
                        .ok_or(AudioPcmMixError::SourceSampleOutOfRange {
                            source_index,
                            sample,
                        })?
                };
                Ok([left, right])
            })
            .map_err(|error| js_error("audio_mix", error))?;
        let active = sources
            .iter()
            .map(|source| source.digest)
            .collect::<std::collections::BTreeSet<_>>();
        while self.audio_pcm.len() > active.len().max(2) {
            let retired = self
                .audio_pcm
                .keys()
                .find(|digest| !active.contains(digest))
                .copied();
            if let Some(digest) = retired {
                self.audio_pcm.remove(&digest);
            } else {
                break;
            }
        }
        Ok(output)
    }

    fn audio_range(&self, render_id: &str, start: i64, end: i64) -> Result<SampleRange, JsError> {
        let render = self.active_render(render_id)?;
        if end
            .checked_sub(start)
            .is_none_or(|len| !(0..=MAX_WEB_AUDIO_BLOCK_SAMPLES).contains(&len))
            || start < 0
            || end > render.compiled().canvas().sample_count()
        {
            return Err(JsError::new(
                "[audio_range] invalid or unbounded audio block",
            ));
        }
        SampleRange::new(start, end).map_err(|error| js_error("audio_range", error))
    }

    /// Report the immutable model/texture content that the host still needs for one exact
    /// Scene3D resource request. JavaScript supplies frozen bytes, but it never
    /// interprets the scene topology or decides which controls are models versus textures.
    pub fn scene3d_resource_needs_json(
        &self,
        content_digest: &str,
        topology_digest: &str,
        canonical_request: &str,
    ) -> Result<String, JsError> {
        let sealed = open_scene3d_request(content_digest, topology_digest, canonical_request)?;
        let frame = sealed.request();
        let mut models = BTreeMap::<ContentDigest, ()>::new();
        let mut textures =
            BTreeMap::<(ContentDigest, valle_motion::scene3d::TextureRole), ()>::new();
        for mesh in &frame.scene.meshes {
            if let Some(control) = &mesh.model_control {
                let model = scene3d_binding_digest(frame, control)?;
                if !self.scene3d_models.contains_key(&model) {
                    models.insert(model, ());
                }
            }
            for (control, role) in mesh.texture_controls() {
                let texture = scene3d_binding_digest(frame, control)?;
                if !self.scene3d_textures.contains_key(&(texture, role)) {
                    textures.insert((texture, role), ());
                }
            }
        }
        let environment = frame
            .scene
            .pbr
            .environment
            .as_ref()
            .map(|e| scene3d_binding_digest(frame, &e.control))
            .transpose()?;
        serde_json::to_string(&Scene3dResourceNeeds {
            environments: environment
                .into_iter()
                .filter(|d| !self.scene3d_environments.contains_key(d))
                .collect(),
            models: models.into_keys().collect(),
            textures: textures
                .into_keys()
                .map(|(digest, role)| Scene3dTextureNeed { digest, role })
                .collect(),
        })
        .map_err(|error| js_error("scene3d_needs", error))
    }

    pub fn register_scene3d_environment(
        &mut self,
        digest: &str,
        bytes: &[u8],
    ) -> Result<(), JsError> {
        let digest = verified_content_digest(digest, bytes, "scene3d_environment_digest")?;
        if let Entry::Vacant(entry) = self.scene3d_environments.entry(digest) {
            let environment = valle_motion::scene3d::EnvironmentAsset::from_frozen(bytes)
                .map_err(|e| js_error("scene3d_environment", e))?;
            entry.insert(Arc::new(environment));
        }
        Ok(())
    }

    /// Admit GLB bytes under the same content identity carried by the Product resource request.
    /// This is host fulfillment state, so it remains legal after the semantic render is frozen.
    pub fn register_scene3d_model(&mut self, digest: &str, bytes: &[u8]) -> Result<(), JsError> {
        let digest = verified_content_digest(digest, bytes, "scene3d_model_digest")?;
        let model = valle_motion::scene3d::admit_glb(bytes)
            .map_err(|error| js_error("scene3d_model", error))?;
        self.scene3d_models.insert(digest, Arc::new(model));
        self.scene3d_prepared.clear();
        Ok(())
    }

    /// Decode encoded material images in the shared Rust core, with color/data in cache identity.
    pub fn register_scene3d_texture(
        &mut self,
        digest: &str,
        encoded_bytes: &[u8],
        role: &str,
    ) -> Result<(), JsError> {
        use valle_motion::scene3d::{MaterialImage, TextureRole};
        let digest = verified_content_digest(digest, encoded_bytes, "scene3d_texture_digest")?;
        let role = match role {
            "color" => TextureRole::Color,
            "data" => TextureRole::Data,
            _ => return Err(JsError::new("[scene3d_texture] role must be color or data")),
        };
        if let Entry::Vacant(entry) = self.scene3d_textures.entry((digest, role)) {
            let texture = MaterialImage::from_encoded(encoded_bytes, role)
                .map_err(|e| js_error("scene3d_texture", e))?;
            entry.insert(Arc::new(texture));
        }
        Ok(())
    }

    /// Fulfill one complete random-access Scene3D request. The returned plane is premultiplied
    /// RGBA16F in little-endian bytes for CanvasKit; depth/object metadata stays in Rust for picking.
    pub fn render_scene3d_request(
        &mut self,
        content_digest: &str,
        topology_digest: &str,
        canonical_request: &str,
    ) -> Result<Vec<u8>, JsError> {
        self.render_scene3d_request_impl(content_digest, topology_digest, canonical_request, false)
    }

    /// Keep the reference geometry rasterizer and picking metadata; let the GPU finish DOF.
    /// Returns a versioned, cropped color/depth packet rather than a CPU-blurred color plane.
    pub fn render_scene3d_request_deferred(
        &mut self,
        content_digest: &str,
        topology_digest: &str,
        canonical_request: &str,
    ) -> Result<Vec<u8>, JsError> {
        self.render_scene3d_request_impl(content_digest, topology_digest, canonical_request, true)
    }

    pub fn has_scene3d_frame(&self, content_digest: &str) -> bool {
        ContentDigest::parse(content_digest)
            .is_ok_and(|digest| self.scene3d_frames.contains_key(&digest))
    }

    /// Browser delivery owns the returned pixels. Do not retain additional full color/depth/id
    /// planes after that job; interactive picking can render and inspect one atomic job on demand.
    pub fn clear_scene3d_frames(&mut self) {
        self.scene3d_frames.clear();
    }

    /// Admission stays in Rust even when the browser fulfills pixels on its GPU: one call admits
    /// the request, prepares its topology once, and returns the lighting input the GPU shades.
    pub fn scene3d_gpu_lighting_input(
        &mut self,
        content_digest: &str,
        topology_digest: &str,
        canonical_request: &str,
    ) -> Result<Vec<f32>, JsError> {
        let sealed = open_scene3d_request(content_digest, topology_digest, canonical_request)?;
        let frame = sealed.request();
        frame
            .scene
            .validate()
            .map_err(|error| js_error("scene3d_request", error))?;
        frame
            .frame
            .validate_for(&frame.scene)
            .map_err(|error| js_error("scene3d_request", error))?;
        // Include aggregate geometry and layer budgets, not just each primitive's validation.
        let resources = valle_motion::scene3d::SceneResources::default();
        let key = valle_motion::scene3d::prepare_cache_key(
            &frame.scene,
            frame.width,
            frame.height,
            &resources,
        )
        .map_err(|error| js_error("scene3d_gpu_admission", error))?;
        let prepared = if let Some(prepared) = self.scene3d_prepared.get(&key) {
            Arc::clone(prepared)
        } else {
            let prepared = Arc::new(
                valle_motion::scene3d::prepare_scene(
                    &frame.scene,
                    frame.width,
                    frame.height,
                    &resources,
                )
                .map_err(|error| js_error("scene3d_gpu_admission", error))?,
            );
            if self.scene3d_prepared.len() >= MAX_SCENE3D_PREPARED_CACHE {
                self.scene3d_prepared.clear();
            }
            self.scene3d_prepared.insert(key, Arc::clone(&prepared));
            prepared
        };
        valle_motion::scene3d::prepare_gpu_lighting_input(&prepared, &frame.frame)
            .map_err(|error| js_error("scene3d_gpu_vertices", error))
    }

    fn render_scene3d_request_impl(
        &mut self,
        content_digest: &str,
        topology_digest: &str,
        canonical_request: &str,
        defer_dof: bool,
    ) -> Result<Vec<u8>, JsError> {
        let sealed = open_scene3d_request(content_digest, topology_digest, canonical_request)?;
        let content_digest = sealed.content_digest();
        let frame = sealed.request();

        let mut resources = valle_motion::scene3d::SceneResources::default();
        if let Some(environment) = &frame.scene.pbr.environment {
            let digest = scene3d_binding_digest(frame, &environment.control)?;
            let asset = self.scene3d_environments.get(&digest).ok_or_else(|| {
                JsError::new("[scene3d_environment] frozen environment has not been fulfilled")
            })?;
            resources
                .environments
                .insert(environment.control.clone(), Arc::clone(asset));
        }

        for mesh in &frame.scene.meshes {
            if let Some(control) = &mesh.model_control {
                let model_digest = scene3d_binding_digest(frame, control)?;
                let model = self.scene3d_models.get(&model_digest).ok_or_else(|| {
                    JsError::new(&format!(
                        "[scene3d_resource] model {} has not been fulfilled",
                        model_digest
                    ))
                })?;
                resources.models.insert(control.clone(), Arc::clone(model));
            }
            for (control, role) in mesh.texture_controls() {
                let texture_digest = scene3d_binding_digest(frame, control)?;
                let texture = self
                    .scene3d_textures
                    .get(&(texture_digest, role))
                    .ok_or_else(|| {
                        JsError::new(&format!(
                            "[scene3d_resource] texture {} has not been fulfilled",
                            texture_digest
                        ))
                    })?;
                resources
                    .textures
                    .insert((control.to_owned(), role), Arc::clone(texture));
            }
        }

        let cache_key = valle_motion::scene3d::prepare_cache_key(
            &frame.scene,
            frame.width,
            frame.height,
            &resources,
        )
        .map_err(|error| js_error("scene3d_prepare_key", error))?;
        let prepared = if let Some(prepared) = self.scene3d_prepared.get(&cache_key) {
            Arc::clone(prepared)
        } else {
            let prepared = Arc::new(
                valle_motion::scene3d::prepare_scene(
                    &frame.scene,
                    frame.width,
                    frame.height,
                    &resources,
                )
                .map_err(|error| js_error("scene3d_prepare", error))?,
            );
            if self.scene3d_prepared.len() >= MAX_SCENE3D_PREPARED_CACHE {
                self.scene3d_prepared.clear();
            }
            self.scene3d_prepared
                .insert(cache_key, Arc::clone(&prepared));
            prepared
        };
        let raster = if defer_dof {
            valle_motion::scene3d::render_scene_without_depth_of_field(&prepared, &frame.frame)
        } else {
            valle_motion::scene3d::render_scene(&prepared, &frame.frame)
        }
        .map_err(|error| js_error("scene3d_render", error))?;
        let scene_key = frame
            .provider_key
            .strip_prefix("scene3d://")
            .unwrap_or(&frame.provider_key);
        let metadata = raster
            .metadata(scene_key, &frame.scene)
            .map_err(|error| js_error("scene3d_metadata", error))?;
        let rgba = if defer_dof {
            super::scene3d_dof::encode(&raster, &frame.frame.camera)
        } else {
            raster.premul_rgba16f_le_bytes()
        };
        if self.scene3d_frames.len() >= MAX_SCENE3D_FRAME_CACHE
            && !self.scene3d_frames.contains_key(&content_digest)
        {
            self.scene3d_frames.clear();
        }
        self.scene3d_frames
            .insert(content_digest, GeneratedScene3dFrame { raster, metadata });
        Ok(rgba)
    }

    pub fn scene3d_frame_width(&self, content_digest: &str) -> Result<u32, JsError> {
        Ok(self.scene3d_frame(content_digest)?.raster.width)
    }

    pub fn scene3d_frame_height(&self, content_digest: &str) -> Result<u32, JsError> {
        Ok(self.scene3d_frame(content_digest)?.raster.height)
    }

    /// Pick within an already fulfilled frame. Product inspection owns canvas→raster placement;
    /// Rust owns the object-id/depth interpretation and semantic address table.
    pub fn scene3d_pick_json(
        &self,
        content_digest: &str,
        pixel_x: u32,
        pixel_y: u32,
    ) -> Result<String, JsError> {
        let generated = self.scene3d_frame(content_digest)?;
        serde_json::to_string(&generated.raster.pick(&generated.metadata, pixel_x, pixel_y))
            .map_err(|error| js_error("scene3d_pick", error))
    }

    pub fn evaluate_prepare(
        &mut self,
        render_id: &str,
        frame: i64,
        render_spec_json: &str,
    ) -> Result<u32, JsError> {
        let spec: RenderSpec = parse_json(render_spec_json, "RenderSpec")?;
        self.evaluate_prepare_with_spec(render_id, frame, spec)
    }

    /// Product preview convenience. The output contract is still constructed by Engine rather
    /// than duplicated in TypeScript; callers choose only extent and alpha policy.
    pub fn evaluate_prepare_preview(
        &mut self,
        render_id: &str,
        frame: i64,
        width: u32,
        height: u32,
        transparent: bool,
    ) -> Result<u32, JsError> {
        let spec = self.preview_spec(width, height, transparent)?;
        self.evaluate_prepare_with_spec(render_id, frame, spec)
    }

    pub fn motion_kernel_available(&self) -> bool {
        self.compiler
            .as_ref()
            .is_some_and(FrameCompiler::motion_kernel_available)
    }

    pub fn capture_motion_jobs_preview(
        &mut self,
        render_id: &str,
        frame: i64,
        width: u32,
        height: u32,
        transparent: bool,
    ) -> Result<String, JsError> {
        let spec = self.preview_spec(width, height, transparent)?;
        let id = self.active_render_id(render_id)?;
        let compiler = self
            .compiler
            .as_mut()
            .ok_or_else(|| JsError::new("render closed"))?;
        let jobs = compiler
            .capture_motion_jobs(id, valle_timeline::internal::FrameKey::new(frame), spec)
            .map_err(|error| js_error("capture_motion", error))?;
        serde_json::to_string(&jobs).map_err(|error| js_error("motion_jobs", error))
    }

    pub fn motion_kernel_definition(&self, id: &str) -> Result<String, JsError> {
        self.compiler
            .as_ref()
            .and_then(|compiler| compiler.motion_definition(id))
            .map(str::to_owned)
            .ok_or_else(|| JsError::new("unknown Motion kernel definition"))
    }

    pub fn motion_kernel_definition_bytes(&self, id: &str) -> Result<Vec<u8>, JsError> {
        self.compiler
            .as_ref()
            .and_then(|compiler| compiler.motion_definition_bytes(id))
            .map(<[u8]>::to_vec)
            .ok_or_else(|| JsError::new("unknown Motion kernel definition"))
    }

    pub fn run_motion_kernel(
        &mut self,
        definition_id: &str,
        definition_json: &str,
        resource_bytes: &[u8],
        input_json: &str,
        compact: bool,
    ) -> Result<Vec<u8>, JsError> {
        self.motion_kernel
            .run(
                definition_id,
                definition_json,
                resource_bytes,
                input_json,
                compact,
            )
            .map_err(|error| js_error("motion_kernel", error))
    }

    pub fn evaluate_prepare_motion_preview(
        &mut self,
        render_id: &str,
        frame: i64,
        width: u32,
        height: u32,
        transparent: bool,
        results: &[u8],
    ) -> Result<u32, JsError> {
        let spec = self.preview_spec(width, height, transparent)?;
        let id = self.active_render_id(render_id)?;
        let compiler = self
            .compiler
            .as_mut()
            .ok_or_else(|| JsError::new("render closed"))?;
        let ticket = compiler
            .evaluate_prepare_motion(
                id,
                valle_timeline::internal::FrameKey::new(frame),
                spec,
                results,
            )
            .map_err(|error| js_error("complete_motion", error))?;
        self.store_prepared_ticket(ticket)
    }

    fn preview_spec(
        &self,
        width: u32,
        height: u32,
        transparent: bool,
    ) -> Result<RenderSpec, JsError> {
        let background = if transparent {
            OutputBackground::Transparent
        } else {
            let render = self
                .active_render
                .as_ref()
                .ok_or_else(|| JsError::new("[render_closed] call open_fixed_package first"))?;
            OutputBackground::AuthorSrgbStraight {
                color: crate::resource::AuthorSrgbStraight(
                    render.compiled().canvas().background_rgba(),
                ),
            }
        };
        let (quality, output) = if self.delivery_output {
            (RenderQuality::Final, OutputSpec::srgb_delivery(background))
        } else {
            (RenderQuality::Preview, OutputSpec::srgb_preview(background))
        };
        let output = output.map_err(|error| js_error("output_spec", error))?;
        RenderSpec::new(width, height, quality, output)
            .map_err(|error| js_error("render_spec", error))
    }

    fn evaluate_prepare_with_spec(
        &mut self,
        render_id: &str,
        frame: i64,
        spec: RenderSpec,
    ) -> Result<u32, JsError> {
        let render_id = self.active_render_id(render_id)?;
        let compiler = self
            .compiler
            .as_mut()
            .ok_or_else(|| JsError::new("[render_closed] call open_fixed_package first"))?;
        let ticket = valle_draw::program::with_instance_compaction(self.realtime_geometry, || {
            compiler.evaluate_prepare(
                render_id,
                valle_timeline::internal::FrameKey::new(frame),
                spec,
            )
        })
        .map_err(|error| js_error("evaluate_prepare", error))?;
        self.store_prepared_ticket(ticket)
    }

    fn store_prepared_ticket(&mut self, mut ticket: PreparedTicket) -> Result<u32, JsError> {
        ticket
            .adapt_external_pixel_layouts(&CANVAS_KIT_EXTERNAL_PIXEL_LAYOUTS)
            .map_err(|error| js_error("canvas_kit_layouts", error))?;
        let id = self.allocate_ticket()?;
        self.tickets.insert(id, Ticket::Prepared(ticket));
        Ok(id)
    }

    /// Shared raw-channel decoder; output is eight numeric byte planes in Alpha8 storage.
    pub fn decode_shader_data_texture(
        &self,
        digest: &str,
        encoded: &[u8],
        width: u32,
        height: u32,
    ) -> Result<Vec<u8>, JsError> {
        verified_content_digest(digest, encoded, "data_texture_digest")?;
        valle_motion::shader::decode_data_texture(encoded, width, height)
            .map_err(|error| js_error("data_texture", error))
    }

    /// Complete batch of resources for this frame. The host may begin asynchronous fulfillment
    /// immediately, while WASM lowers the same ticket in parallel.
    pub fn resource_requests(&self, ticket: u32) -> Result<Vec<u8>, JsError> {
        self.ticket(ticket)?
            .prepared()
            .resource_requests
            .packed_bytes()
            .map_err(|error| js_error("resource_requests", error))
    }

    /// Authoring metadata projected from the exact prepared Product frame. It is intentionally a
    /// sidecar of the same ticket—not a second Scene evaluation—so selection geometry cannot
    /// disagree with the plan being executed.
    pub fn frame_inspection_json(&self, ticket: u32) -> Result<String, JsError> {
        let prepared = self.ticket(ticket)?.prepared();
        let frame = &prepared.frame;
        let mut hits = Vec::new();
        let mut scene3d = Vec::new();
        for item in &frame.visual {
            match item {
                PreparedVisualItem::Layer(layer) => {
                    push_hit(&mut hits, layer);
                    push_scene3d_placements(&mut scene3d, layer, &frame.programs)?;
                }
                PreparedVisualItem::Transition(transition) => {
                    push_hit(&mut hits, &transition.from);
                    push_hit(&mut hits, &transition.to);
                    push_scene3d_placements(&mut scene3d, &transition.from, &frame.programs)?;
                    push_scene3d_placements(&mut scene3d, &transition.to, &frame.programs)?;
                }
                // The clips the template reads are what a click selects.
                PreparedVisualItem::MotionTransition(transition) => {
                    for clip in transition.from.iter().chain(&transition.to) {
                        push_hit(&mut hits, clip);
                        push_scene3d_placements(&mut scene3d, clip, &frame.programs)?;
                    }
                }
            }
        }
        for caption in &frame.captions {
            hits.push(ProductHitRect {
                clip_id: caption.clip_id(),
                kind: "caption",
                rect: caption.bounds().output.into(),
            });
        }
        serde_json::to_string(&serde_json::json!({
            "renderId": frame.render_id.to_string(),
            "compositionFrame": frame.key.index(),
            "hitRects": hits,
            "motion": &prepared.inspection.motion,
            "scene3d": scene3d,
        }))
        .map_err(|error| js_error("frame_inspection", error))
    }

    /// Exact CanvasKit product capability set. JavaScript reports only memory limits; it never
    /// mirrors GraphCapability ordering or invents lowering policy.
    pub fn lower_canvas_kit(
        &mut self,
        ticket: u32,
        max_surface_bytes: u64,
        max_frame_bytes: u64,
    ) -> Result<(), JsError> {
        let capabilities = BackendCapabilities::new(
            Extent2d::new(16_384, 16_384)
                .map_err(|error| js_error("canvas_kit_capabilities", error))?,
            [TextureFormat::Rgba16Float],
            [
                TextureUsage::Sampled,
                TextureUsage::StorageRead,
                TextureUsage::StorageWrite,
                TextureUsage::ColorAttachment,
                TextureUsage::CopySource,
                TextureUsage::CopyDestination,
            ],
            [1],
            CANVAS_KIT_EXTERNAL_PIXEL_LAYOUTS,
            [
                GraphCapability::Clear,
                GraphCapability::ExternalImport,
                GraphCapability::SourcePipeline,
                GraphCapability::DrawProgram,
                GraphCapability::BackdropRead,
                GraphCapability::Group,
                GraphCapability::Filter,
                GraphCapability::Mask,
                GraphCapability::Blend,
                GraphCapability::Transition,
                GraphCapability::AdjustmentEffect,
                GraphCapability::Caption,
                GraphCapability::OutputTransform,
                GraphCapability::HostFrame,
            ],
            true,
            None,
            max_surface_bytes,
            max_frame_bytes,
        )
        .map_err(|error| js_error("canvas_kit_capabilities", error))?;
        self.lower_with_capabilities(ticket, capabilities)
    }

    fn lower_with_capabilities(
        &mut self,
        ticket: u32,
        capabilities: BackendCapabilities,
    ) -> Result<(), JsError> {
        let state = self
            .tickets
            .remove(&ticket)
            .ok_or_else(|| unknown_ticket(ticket))?;
        let Ticket::Prepared(prepared) = state else {
            return Err(JsError::new(
                "[ticket_state] lower requires a prepared ticket",
            ));
        };
        let lowered = prepared
            .lower(&capabilities)
            .map_err(|error| js_error("lower", error))?;
        self.tickets.insert(ticket, Ticket::Lowered(lowered));
        Ok(())
    }

    pub fn template_cache_hit(&self, ticket: u32) -> Result<bool, JsError> {
        match self.ticket(ticket)? {
            Ticket::Prepared(_) => Err(JsError::new(
                "[ticket_state] template cache status requires a lowered ticket",
            )),
            Ticket::Lowered(ticket) => Ok(ticket.template_cache_hit()),
            Ticket::Bound(ticket) => Ok(ticket.template_cache_hit()),
        }
    }

    pub fn bind(&mut self, ticket: u32, generation: u64) -> Result<(), JsError> {
        let generation = ExternalGeneration::new(generation)
            .map_err(|error| js_error("external_generation", error))?;
        let state = self
            .tickets
            .remove(&ticket)
            .ok_or_else(|| unknown_ticket(ticket))?;
        let Ticket::Lowered(lowered) = state else {
            return Err(JsError::new(
                "[ticket_state] bind requires a lowered ticket",
            ));
        };
        let bound = lowered
            .bind(generation)
            .map_err(|error| js_error("bind", error))?;
        self.tickets.insert(ticket, Ticket::Bound(bound));
        Ok(())
    }

    pub fn plan_template_bytes(&self, ticket: u32) -> Result<Vec<u8>, JsError> {
        let template = match self.ticket(ticket)? {
            Ticket::Prepared(_) => {
                return Err(JsError::new(
                    "[ticket_state] plan requires a lowered ticket",
                ));
            }
            Ticket::Lowered(ticket) => ticket.template(),
            Ticket::Bound(ticket) => ticket.template(),
        };
        template
            .packed_bytes()
            .map_err(|error| js_error("plan_packet", error))
    }

    pub fn plan_template_hash(&self, ticket: u32) -> Result<String, JsError> {
        let template = match self.ticket(ticket)? {
            Ticket::Prepared(_) => {
                return Err(JsError::new(
                    "[ticket_state] plan hash requires a lowered ticket",
                ));
            }
            Ticket::Lowered(ticket) => ticket.template(),
            Ticket::Bound(ticket) => ticket.template(),
        };
        template
            .template_hash()
            .map(|digest| digest.to_string())
            .map_err(|error| js_error("plan_hash", error))
    }

    pub fn binding_bytes(&self, ticket: u32) -> Result<Vec<u8>, JsError> {
        let Ticket::Bound(ticket) = self.ticket(ticket)? else {
            return Err(JsError::new(
                "[ticket_state] bindings require a bound ticket",
            ));
        };
        ticket
            .bindings()
            .packed_bytes()
            .map_err(|error| js_error("binding_packet", error))
    }

    pub fn bound_schedule_bytes(&self, ticket: u32) -> Result<Vec<u8>, JsError> {
        let Ticket::Bound(ticket) = self.ticket(ticket)? else {
            return Err(JsError::new(
                "[ticket_state] bound schedule requires a bound ticket",
            ));
        };
        let schedule = ticket
            .bound_program_schedules()
            .map_err(|error| js_error("bound_schedule", error))?;
        schedule
            .packed_bytes()
            .map_err(|error| js_error("bound_schedule_packet", error))
    }

    pub fn release_ticket(&mut self, ticket: u32) -> bool {
        self.tickets.remove(&ticket).is_some()
    }

    fn active_render(&self, declared: &str) -> Result<&EngineRender, JsError> {
        let render = self
            .active_render
            .as_ref()
            .ok_or_else(|| JsError::new("[render_closed] call open_fixed_package first"))?;
        let actual = valle_timeline::internal::RenderId::parse(declared)
            .map_err(|error| js_error("render_id", error))?;
        if actual != render.render_id() {
            return Err(JsError::new(&format!(
                "[render_mismatch] expected {}, got {}",
                render.render_id(),
                actual
            )));
        }
        Ok(render)
    }

    fn active_render_id(
        &self,
        declared: &str,
    ) -> Result<valle_timeline::internal::RenderId, JsError> {
        Ok(self.active_render(declared)?.render_id())
    }

    fn allocate_ticket(&mut self) -> Result<u32, JsError> {
        let id = self.next_ticket;
        if id == 0 {
            return Err(JsError::new("[ticket_budget] ticket id space exhausted"));
        }
        self.next_ticket = self
            .next_ticket
            .checked_add(1)
            .ok_or_else(|| JsError::new("[ticket_budget] ticket id space exhausted"))?;
        Ok(id)
    }

    fn ticket(&self, ticket: u32) -> Result<&Ticket, JsError> {
        self.tickets
            .get(&ticket)
            .ok_or_else(|| unknown_ticket(ticket))
    }

    fn scene3d_frame(&self, content_digest: &str) -> Result<&GeneratedScene3dFrame, JsError> {
        let digest = ContentDigest::parse(content_digest)
            .map_err(|error| js_error("scene3d_content_digest", error))?;
        self.scene3d_frames.get(&digest).ok_or_else(|| {
            JsError::new(&format!(
                "[scene3d_frame] generated frame {} is not available",
                digest
            ))
        })
    }
}

/// A click selects the clip under its effects.
fn push_hit<'a>(hits: &mut Vec<ProductHitRect<'a>>, layer: &'a PreparedLayer) {
    let layer = layer.innermost();
    let kind = match &layer.source {
        PreparedSource::External { source_kind, .. }
        | PreparedSource::Program { source_kind, .. } => match source_kind {
            crate::prepare::PreparedSourceKind::Video => "video",
            crate::prepare::PreparedSourceKind::Image => "image",
            crate::prepare::PreparedSourceKind::Motion => "motion",
            crate::prepare::PreparedSourceKind::Solid => "solid",
        },
    };
    hits.push(ProductHitRect {
        clip_id: &layer.clip_id,
        kind,
        rect: layer.bounds.output.into(),
    });
}

fn push_scene3d_placements(
    output: &mut Vec<ProductScene3dPlacement>,
    layer: &PreparedLayer,
    programs: &[crate::prepare::PreparedProgram],
) -> Result<(), JsError> {
    let layer = layer.innermost();
    let PreparedSource::Program { program, .. } = &layer.source else {
        return Ok(());
    };
    let prepared = programs
        .iter()
        .find(|candidate| candidate.id == *program)
        .ok_or_else(|| JsError::new("[frame_inspection] Motion program is missing"))?;
    // Preparation already admitted this exact DrawProgram. Re-decoding its packed form for
    // picking repeated all node/geometry validation even on large, entirely 2D frames.
    // Deserialized fixture programs still go through the original admission boundary.
    let decoded;
    let draw = if let Some(runtime) = prepared.runtime_program() {
        runtime.as_ref()
    } else {
        decoded = valle_draw::program::DrawProgram::from_packed(&prepared.packed)
            .map_err(|error| js_error("frame_inspection", error))?;
        &decoded
    };
    let viewport = draw.viewport();
    let normalize = valle_draw::program::Transform2d([
        viewport.width.recip(),
        0.0,
        -viewport.x / viewport.width,
        0.0,
        viewport.height.recip(),
        -viewport.y / viewport.height,
        0.0,
        0.0,
        1.0,
    ]);
    let device = valle_draw::program::Transform2d(layer.device_transform.matrix());
    for root in draw.roots() {
        collect_scene3d_placements(
            output,
            draw,
            *root,
            valle_draw::program::Transform2d::IDENTITY,
            normalize,
            device,
            &layer.clip_id,
        )?;
    }
    Ok(())
}

fn collect_scene3d_placements(
    output: &mut Vec<ProductScene3dPlacement>,
    program: &valle_draw::program::DrawProgram,
    node: valle_draw::program::NodeId,
    local_to_program: valle_draw::program::Transform2d,
    normalize: valle_draw::program::Transform2d,
    device: valle_draw::program::Transform2d,
    clip_id: &str,
) -> Result<(), JsError> {
    let value = program
        .nodes()
        .get(node.raw() as usize)
        .ok_or_else(|| JsError::new("[frame_inspection] DrawProgram node is missing"))?;
    match value {
        valle_draw::program::Node::Group(group) => {
            if group.opacity <= 0.0 {
                return Ok(());
            }
            let child_to_program = group.transform.then(local_to_program);
            for child in &group.children {
                collect_scene3d_placements(
                    output,
                    program,
                    *child,
                    child_to_program,
                    normalize,
                    device,
                    clip_id,
                )?;
            }
        }
        valle_draw::program::Node::Scene3d(scene) => {
            output.push(ProductScene3dPlacement {
                clip_id: clip_id.to_owned(),
                content_hash: ContentDigest::from_bytes(scene.scene.content_hash.into_bytes()),
                bounds: scene.bounds,
                device_from_local: local_to_program.then(normalize).then(device).0,
            });
        }
        _ => {}
    }
    Ok(())
}

impl Default for ProductEngine {
    fn default() -> Self {
        Self::new()
    }
}

fn open_product_fixed_package(
    fixed_package_manifest_json: &str,
    timeline_json: &str,
    manifest_json: &str,
    verified_binding_bundle_json: &str,
    resource_bytes: &[u8],
) -> Result<crate::fixed_package::OpenedFixedPackage, String> {
    let execution_profile_json = canonical_fixed_execution_profile(COMMON_PROFILE_KEY)?;
    let files = fixed_package_files(
        timeline_json,
        manifest_json,
        verified_binding_bundle_json,
        &execution_profile_json,
        resource_bytes,
    );
    open_verified_fixed_package(fixed_package_manifest_json, &files)
        .map_err(|error| error.to_string())
}

fn parse_json<T: serde::de::DeserializeOwned>(json: &str, kind: &str) -> Result<T, JsError> {
    serde_json::from_str(json)
        .map_err(|error| JsError::new(&format!("[input_parse] invalid {kind}: {error}")))
}

fn unknown_ticket(ticket: u32) -> JsError {
    JsError::new(&format!("[unknown_ticket] ticket {ticket} does not exist"))
}

fn js_error(code: &str, error: impl std::fmt::Display) -> JsError {
    JsError::new(&format!("[{code}] {error}"))
}

/// Open a request sealed by the authoritative engine. Both digests name the exact text, which
/// proves it is canonical; it is parsed once and never re-serialized.
fn open_scene3d_request(
    content_digest: &str,
    topology_digest: &str,
    canonical_request: &str,
) -> Result<valle_motion::SealedScene3DFrame, JsError> {
    let content = ContentDigest::parse(content_digest)
        .map_err(|error| js_error("scene3d_content_digest", error))?;
    let topology = ContentDigest::parse(topology_digest)
        .map_err(|error| js_error("scene3d_topology_digest", error))?;
    valle_motion::SealedScene3DFrame::from_canonical(canonical_request, content, topology)
        .map_err(|error| js_error("scene3d_request", error))
}

fn scene3d_binding_digest(
    frame: &valle_motion::Scene3DFrameRequest,
    control: &str,
) -> Result<ContentDigest, JsError> {
    let digest = frame.resource_bindings.get(control).ok_or_else(|| {
        JsError::new(&format!(
            "[scene3d_request] control {control:?} has no content binding"
        ))
    })?;
    Ok(*digest)
}

fn verified_content_digest(
    declared: &str,
    bytes: &[u8],
    code: &str,
) -> Result<ContentDigest, JsError> {
    let declared = ContentDigest::parse(declared).map_err(|error| js_error(code, error))?;
    let actual = ContentDigest::of_bytes(bytes);
    if actual != declared {
        return Err(JsError::new(&format!(
            "[{code}] expected {}, got {}",
            declared, actual
        )));
    }
    Ok(declared)
}

#[cfg(test)]
mod tests {
    use super::{ProductEngine, open_product_fixed_package};
    use crate::fixed_package::{
        canonical_fixed_package_manifest, empty_fixed_package_fixtures, fixed_package_files,
    };

    fn fixed_package_fixture() -> (String, String, String, String) {
        let (timeline, resource_manifest, bundle, execution_profile) =
            empty_fixed_package_fixtures();
        let files = fixed_package_files(
            &timeline,
            &resource_manifest,
            &bundle,
            &execution_profile,
            &[],
        );
        let fixed_package_manifest = canonical_fixed_package_manifest(&files).unwrap();
        (fixed_package_manifest, timeline, resource_manifest, bundle)
    }

    #[test]
    fn preview_accepts_authored_background_and_explicit_transparency() {
        let (timeline, resources, bundle, profile) = empty_fixed_package_fixtures();
        let mut value: serde_json::Value = serde_json::from_str(&timeline).unwrap();
        value["document"]["background"]["color"] = serde_json::json!("#102030ff");
        let timeline = valle_timeline::internal::encode_canonical(
            &valle_timeline::internal::decode_canonical(&value.to_string()).unwrap(),
        )
        .unwrap();
        let files = fixed_package_files(&timeline, &resources, &bundle, &profile, &[]);
        let package = canonical_fixed_package_manifest(&files).unwrap();
        let mut engine = ProductEngine::new();
        let receipt: serde_json::Value = serde_json::from_str(
            &engine
                .open_fixed_package(&package, &timeline, &resources, &bundle, &[])
                .unwrap(),
        )
        .unwrap();
        let id = receipt["renderId"].as_str().unwrap();
        for transparent in [false, true] {
            assert!(
                engine
                    .evaluate_prepare_preview(id, 0, 320, 180, transparent)
                    .is_ok()
            );
        }
    }

    #[test]
    fn fixed_package_clocks_and_audio_are_render_scoped() {
        let (fixed_manifest, timeline, manifest, bundle) = fixed_package_fixture();
        let mut engine = ProductEngine::new();
        let first: serde_json::Value = serde_json::from_str(
            &engine
                .open_fixed_package(&fixed_manifest, &timeline, &manifest, &bundle, &[])
                .unwrap(),
        )
        .unwrap();
        assert_eq!(first.as_object().unwrap().len(), 7);
        let render_id = first["renderId"].as_str().unwrap();
        assert_eq!(engine.frame_at_seconds(render_id, 0.5).unwrap(), 15);
        assert_eq!(engine.sample_at_seconds(render_id, 0.5).unwrap(), 24_000);
        let audio_program: serde_json::Value =
            serde_json::from_str(&engine.audio_program_json(render_id).unwrap()).unwrap();
        assert_eq!(audio_program["tracks"], serde_json::json!([]));
        let execution_resources: serde_json::Value =
            serde_json::from_str(&engine.compiled_execution_resources_json(render_id).unwrap())
                .unwrap();
        assert_eq!(execution_resources, serde_json::json!([]));
        let sources: serde_json::Value =
            serde_json::from_str(&engine.audio_sources_json(render_id, 0, 3).unwrap()).unwrap();
        assert_eq!(sources, serde_json::json!([]));
        assert_eq!(engine.mix_audio_pcm(render_id, 0, 3).unwrap(), vec![0.0; 6]);

        engine.reset();
        let second: serde_json::Value = serde_json::from_str(
            &engine
                .open_fixed_package(&fixed_manifest, &timeline, &manifest, &bundle, &[])
                .unwrap(),
        )
        .unwrap();
        assert_eq!(second["renderId"], first["renderId"]);
    }

    #[test]
    fn web_clock_uses_half_away_from_zero_at_positive_ties() {
        let (fixed_manifest, timeline, manifest, bundle) = fixed_package_fixture();
        let mut engine = ProductEngine::new();
        let receipt: serde_json::Value = serde_json::from_str(
            &engine
                .open_fixed_package(&fixed_manifest, &timeline, &manifest, &bundle, &[])
                .unwrap(),
        )
        .unwrap();
        let render_id = receipt["renderId"].as_str().unwrap();
        assert_eq!(engine.frame_at_seconds(render_id, 1.0 / 60.0).unwrap(), 1);
    }

    #[test]
    fn web_product_rejects_swapped_member_before_inner_admission() {
        let (fixed_manifest, timeline, resource_manifest, bundle) = fixed_package_fixture();
        let error = open_product_fixed_package(
            &fixed_manifest,
            &resource_manifest,
            &timeline,
            &bundle,
            &[],
        )
        .unwrap_err();
        assert!(
            error.starts_with("[fixed_package] byte length mismatch")
                || error.starts_with("[fixed_package] digest mismatch"),
            "unexpected admission layer: {error}"
        );
    }

    #[test]
    fn web_product_rejects_missing_outer_member_before_inner_admission() {
        let (fixed_manifest, timeline, resource_manifest, bundle) = fixed_package_fixture();
        assert!(
            open_product_fixed_package(
                &fixed_manifest,
                &timeline,
                &resource_manifest,
                &bundle,
                &[],
            )
            .is_ok(),
            "the unchanged inner package must be admissible"
        );
        let mut manifest: serde_json::Value = serde_json::from_str(&fixed_manifest).unwrap();
        manifest["members"].as_array_mut().unwrap().pop();
        let missing = String::from_utf8(serde_jcs::to_vec(&manifest).unwrap()).unwrap();
        let error =
            open_product_fixed_package(&missing, &timeline, &resource_manifest, &bundle, &[])
                .unwrap_err();
        assert!(
            error.starts_with("[fixed_package_manifest] expected 5 closed members"),
            "unexpected admission layer: {error}"
        );
    }

    #[test]
    fn web_product_rejects_noncanonical_outer_manifest_before_inner_admission() {
        let (fixed_manifest, timeline, resource_manifest, bundle) = fixed_package_fixture();
        let error = open_product_fixed_package(
            &format!("{fixed_manifest}\n"),
            &timeline,
            &resource_manifest,
            &bundle,
            &[],
        )
        .unwrap_err();
        assert_eq!(
            error,
            "[fixed_package_manifest] manifest must use canonical JCS bytes"
        );
    }
}
