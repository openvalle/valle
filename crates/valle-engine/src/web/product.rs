//! Stateful WASM host for the staged Product Compositor API.
//!
//! One pinned Engine render is opened atomically, then every frame crosses the JS/WASM boundary
//! as three bounded binary packets: resource requests, plan template and bindings. Platform
//! objects never enter the semantic render; JavaScript keeps them in its generation-scoped
//! object table.

use std::{collections::BTreeMap, sync::Arc};

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
    render::{
        CompiledExecutionResourceKind, EvaluatedAudioSample, MappedSourceTime, SampleRange,
        VerifiedResourceFacts,
    },
    resource::{
        ContentDigest, Extent2d, ExternalGeneration, ExternalPixelLayout, OutputBackground,
        OutputSpec, TextureFormat, TextureUsage,
    },
};
use valle_timeline::internal::wire::resource::AudioChannelLayoutWire;

const MAX_SCENE3D_PREPARED_CACHE: usize = 128;
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Scene3dResourceNeeds {
    models: Vec<ContentDigest>,
    textures: Vec<ContentDigest>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AudioSamplePacket {
    render_id: String,
    sample: i64,
    sample_time: valle_timeline::RationalTime,
    tracks: Vec<AudioTrackPacket>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AudioBlockPacket {
    render_id: String,
    sample_rate: u32,
    start_sample: i64,
    end_sample: i64,
    samples: Vec<AudioSamplePacket>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AudioTrackPacket {
    track_order: u32,
    endpoints: Vec<AudioEndpointPacket>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AudioEndpointPacket {
    source_index: u32,
    source_sample_index: i64,
    mapped_time: MappedSourceTimePacket,
    digest: Option<ContentDigest>,
    handle: Option<u64>,
    decoded_pcm_digest: ContentDigest,
    source_channels: u16,
    crossfade_gain: f64,
    gain: f64,
    pan: f64,
    left_gain: f64,
    right_gain: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CompiledExecutionResourcePacket<'a> {
    resource_id: &'a str,
    kind: CompiledExecutionResourceKind,
    content_digest: &'a ContentDigest,
    abi_digest: Option<&'a ContentDigest>,
}

#[derive(Serialize)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
enum MappedSourceTimePacket {
    Static,
    Exact { time: valle_timeline::RationalTime },
    HoldStart,
    HoldEnd,
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
    scene3d_textures: BTreeMap<ContentDigest, Arc<valle_motion::scene3d::TextureAsset>>,
    scene3d_prepared: BTreeMap<
        valle_motion::scene3d::ScenePrepareCacheKey,
        Arc<valle_motion::scene3d::PreparedScene>,
    >,
    scene3d_frames: BTreeMap<ContentDigest, GeneratedScene3dFrame>,
    active_render: Option<EngineRender>,
    compiler: Option<FrameCompiler>,
    tickets: BTreeMap<u32, Ticket>,
    next_ticket: u32,
}

#[wasm_bindgen]
impl ProductEngine {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self {
            scene3d_models: BTreeMap::new(),
            scene3d_textures: BTreeMap::new(),
            scene3d_prepared: BTreeMap::new(),
            scene3d_frames: BTreeMap::new(),
            active_render: None,
            compiler: None,
            tickets: BTreeMap::new(),
            next_ticket: 1,
        }
    }

    /// Pack the canonical production SkSL uniform ABI. CanvasKit binds these floats and the
    /// working-linear backdrop mechanically; no material formulas or pixels cross into JS/Wasm.
    pub fn pack_motion_glass_uniforms(
        &self,
        program_json: &str,
        owner_to_device: &[f64],
    ) -> Result<Vec<f32>, JsError> {
        let program: valle_draw::program::MotionGlassProgram =
            parse_json(program_json, "MotionGlassProgram")?;
        let transform = glass_transform(owner_to_device)?;
        crate::compositor::glass::pack_motion_glass_gpu_uniforms(&program, transform)
            .map(|uniforms| uniforms.to_vec())
            .map_err(|error| js_error("motion_glass_uniforms", error))
    }

    /// Pack the foreground mode of the same production SkSL uniform ABI.
    pub fn pack_motion_glass_foreground_uniforms(
        &self,
        program_json: &str,
        owner_to_device: &[f64],
    ) -> Result<Vec<f32>, JsError> {
        let foreground: valle_draw::program::MotionGlassForegroundProgram =
            parse_json(program_json, "MotionGlassForegroundProgram")?;
        let transform = glass_transform(owner_to_device)?;
        crate::compositor::glass::pack_motion_glass_foreground_gpu_uniforms(&foreground, transform)
            .map(|uniforms| uniforms.to_vec())
            .map_err(|error| js_error("motion_glass_foreground_uniforms", error))
    }

    /// Clear the active render, frame tickets and host-side fulfillment caches. Backend objects
    /// remain JS-owned and must be retired by the host under the matching external generation.
    pub fn reset(&mut self) {
        self.scene3d_models.clear();
        self.scene3d_textures.clear();
        self.scene3d_prepared.clear();
        self.scene3d_frames.clear();
        self.active_render = None;
        self.compiler = None;
        self.tickets.clear();
        self.next_ticket = 1;
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
    ) -> Result<String, JsError> {
        let opened = open_product_fixed_package(
            fixed_package_manifest_json,
            timeline_json,
            manifest_json,
            verified_binding_bundle_json,
        )
        .map_err(|error| JsError::new(&error))?;
        let render = opened.engine_render();
        self.compiler = Some(render.frame_compiler());
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

    /// Evaluate one output sample from the frozen CompiledAudioProgram.
    pub fn audio_sample_json(&self, render_id: &str, sample: i64) -> Result<String, JsError> {
        let render = self.active_render(render_id)?;
        let evaluated = render
            .compiled()
            .audio()
            .evaluate_sample(sample)
            .map_err(|error| js_error("audio_sample", error))?;
        serde_json::to_string(&audio_sample_packet(render.render_id(), &evaluated))
            .map_err(|error| js_error("audio_sample", error))
    }

    /// Evaluate a bounded half-open output sample range from the immutable
    /// CompiledAudioProgram. The cap prevents an untrusted Web caller from
    /// materializing an unbounded JSON packet in WASM memory.
    pub fn audio_block_json(
        &self,
        render_id: &str,
        start_sample: i64,
        end_sample: i64,
    ) -> Result<String, JsError> {
        let render = self.active_render(render_id)?;
        let len = end_sample
            .checked_sub(start_sample)
            .ok_or_else(|| JsError::new("[audio_block] sample range length overflowed"))?;
        if len > MAX_WEB_AUDIO_BLOCK_SAMPLES {
            return Err(JsError::new(&format!(
                "[audio_block] range length {len} exceeds {MAX_WEB_AUDIO_BLOCK_SAMPLES}"
            )));
        }
        let range = SampleRange::new(start_sample, end_sample)
            .map_err(|error| js_error("audio_block", error))?;
        let evaluated = render
            .compiled()
            .audio()
            .evaluate_block(range)
            .map_err(|error| js_error("audio_block", error))?;
        let render_id = render.render_id();
        serde_json::to_string(&AudioBlockPacket {
            render_id: render_id.to_string(),
            sample_rate: render.compiled().canvas().sample_rate(),
            start_sample,
            end_sample,
            samples: evaluated
                .samples()
                .iter()
                .map(|sample| audio_sample_packet(render_id, sample))
                .collect(),
        })
        .map_err(|error| js_error("audio_block", error))
    }

    /// Report the immutable model/texture content that the host still needs for one exact
    /// Scene3D resource request. JavaScript fetches bytes and decodes image pixels, but it never
    /// interprets the scene topology or decides which controls are models versus textures.
    pub fn scene3d_resource_needs_json(&self, canonical_request: &[u8]) -> Result<String, JsError> {
        let frame = parse_scene3d_request(canonical_request)?;
        let mut models = BTreeMap::<ContentDigest, ()>::new();
        let mut textures = BTreeMap::<ContentDigest, ()>::new();
        for mesh in &frame.scene.meshes {
            let model = scene3d_binding_digest(&frame, &mesh.model_control)?;
            if !self.scene3d_models.contains_key(&model) {
                models.insert(model, ());
            }
            if let Some(control) = &mesh.material.texture_control {
                let texture = scene3d_binding_digest(&frame, control)?;
                if !self.scene3d_textures.contains_key(&texture) {
                    textures.insert(texture, ());
                }
            }
        }
        serde_json::to_string(&Scene3dResourceNeeds {
            models: models.into_keys().collect(),
            textures: textures.into_keys().collect(),
        })
        .map_err(|error| js_error("scene3d_needs", error))
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

    /// Admit CanvasKit-decoded premultiplied RGBA8 while retaining the encoded-byte digest as the
    /// texture identity. Native performs the equivalent decode through Skia.
    pub fn register_scene3d_texture(
        &mut self,
        digest: &str,
        encoded_bytes: &[u8],
        width: u32,
        height: u32,
        premul_rgba8: &[u8],
    ) -> Result<(), JsError> {
        let digest = verified_content_digest(digest, encoded_bytes, "scene3d_texture_digest")?;
        let texture =
            valle_motion::scene3d::TextureAsset::new(digest, width, height, premul_rgba8.to_vec())
                .map_err(|error| js_error("scene3d_texture", error))?;
        self.scene3d_textures.insert(digest, Arc::new(texture));
        self.scene3d_prepared.clear();
        Ok(())
    }

    /// Fulfill one complete random-access Scene3D request. The returned plane is premultiplied
    /// RGBA8 for an opaque CanvasKit object; depth/object metadata stays in Rust for picking.
    pub fn render_scene3d_request(
        &mut self,
        content_digest: &str,
        topology_digest: &str,
        canonical_request: &[u8],
    ) -> Result<Vec<u8>, JsError> {
        let content_digest = ContentDigest::parse(content_digest)
            .map_err(|error| js_error("scene3d_content_digest", error))?;
        let topology_digest = ContentDigest::parse(topology_digest)
            .map_err(|error| js_error("scene3d_topology_digest", error))?;
        let frame = parse_scene3d_request(canonical_request)?;
        if frame
            .content_digest()
            .map_err(|error| js_error("scene3d_request", error))?
            != content_digest
        {
            return Err(JsError::new(
                "[scene3d_request] content digest does not name the canonical request",
            ));
        }
        if frame
            .topology_digest()
            .map_err(|error| js_error("scene3d_request", error))?
            != topology_digest
        {
            return Err(JsError::new(
                "[scene3d_request] topology digest does not name the request scene",
            ));
        }

        let mut resources = valle_motion::scene3d::SceneResources::default();
        for mesh in &frame.scene.meshes {
            let model_digest = scene3d_binding_digest(&frame, &mesh.model_control)?;
            let model = self.scene3d_models.get(&model_digest).ok_or_else(|| {
                JsError::new(&format!(
                    "[scene3d_resource] model {} has not been fulfilled",
                    model_digest
                ))
            })?;
            resources
                .models
                .insert(mesh.model_control.clone(), Arc::clone(model));
            if let Some(control) = &mesh.material.texture_control {
                let texture_digest = scene3d_binding_digest(&frame, control)?;
                let texture = self.scene3d_textures.get(&texture_digest).ok_or_else(|| {
                    JsError::new(&format!(
                        "[scene3d_resource] texture {} has not been fulfilled",
                        texture_digest
                    ))
                })?;
                resources
                    .textures
                    .insert(control.clone(), Arc::clone(texture));
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
        let raster = valle_motion::scene3d::render_scene(&prepared, &frame.frame)
            .map_err(|error| js_error("scene3d_render", error))?;
        let metadata = raster
            .metadata(&frame.provider_key, &frame.scene)
            .map_err(|error| js_error("scene3d_metadata", error))?;
        let rgba = raster.premul_rgba8.clone();
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
        let background = if transparent {
            OutputBackground::Transparent
        } else {
            OutputBackground::opaque_srgb([0, 0, 0])
        };
        let output =
            OutputSpec::srgb_preview(background).map_err(|error| js_error("output_spec", error))?;
        let spec = RenderSpec::new(width, height, RenderQuality::Preview, output)
            .map_err(|error| js_error("render_spec", error))?;
        self.evaluate_prepare_with_spec(render_id, frame, spec)
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
        let mut ticket = compiler
            .evaluate_prepare(
                render_id,
                valle_timeline::internal::FrameKey::new(frame),
                spec,
            )
            .map_err(|error| js_error("evaluate_prepare", error))?;
        ticket
            .adapt_external_pixel_layouts(&CANVAS_KIT_EXTERNAL_PIXEL_LAYOUTS)
            .map_err(|error| js_error("canvas_kit_layouts", error))?;
        let id = self.allocate_ticket()?;
        self.tickets.insert(id, Ticket::Prepared(ticket));
        Ok(id)
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
            }
        }
        for caption in &frame.captions {
            hits.push(ProductHitRect {
                clip_id: &caption.clip_id,
                kind: "caption",
                rect: caption.bounds.output.into(),
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

fn push_hit<'a>(hits: &mut Vec<ProductHitRect<'a>>, layer: &'a PreparedLayer) {
    let kind = match &layer.source {
        PreparedSource::External { source_kind, .. }
        | PreparedSource::Program { source_kind, .. } => match source_kind {
            crate::prepare::PreparedSourceKind::Video => "video",
            crate::prepare::PreparedSourceKind::Image => "image",
            crate::prepare::PreparedSourceKind::Lottie => "lottie",
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
    let PreparedSource::Program { program, .. } = &layer.source else {
        return Ok(());
    };
    let prepared = programs
        .iter()
        .find(|candidate| candidate.id == *program)
        .ok_or_else(|| JsError::new("[frame_inspection] Motion program is missing"))?;
    let draw = valle_draw::program::DrawProgram::from_packed(&prepared.packed)
        .map_err(|error| js_error("frame_inspection", error))?;
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
            &draw,
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

fn audio_sample_packet(
    render_id: valle_timeline::internal::RenderId,
    evaluated: &EvaluatedAudioSample,
) -> AudioSamplePacket {
    AudioSamplePacket {
        render_id: render_id.to_string(),
        sample: evaluated.sample(),
        sample_time: evaluated.sample_time(),
        tracks: evaluated
            .tracks()
            .iter()
            .map(|track| AudioTrackPacket {
                track_order: track.track_order(),
                endpoints: track
                    .endpoints()
                    .iter()
                    .map(|endpoint| {
                        let resource = endpoint.source().resource();
                        let (decoded_pcm_digest, source_channels) =
                            match resource.map(|resource| resource.facts()) {
                                Some(VerifiedResourceFacts::Audio {
                                    descriptor,
                                    decoded_pcm_digest,
                                    ..
                                }) => {
                                    let channels = match descriptor.channel_layout {
                                        AudioChannelLayoutWire::Mono => 1,
                                        AudioChannelLayoutWire::Stereo => 2,
                                        AudioChannelLayoutWire::Surround51
                                        | AudioChannelLayoutWire::Surround71 => unreachable!(
                                            "audio admission rejects unsupported channel layouts"
                                        ),
                                    };
                                    (*decoded_pcm_digest, channels)
                                }
                                _ => unreachable!("admitted audio endpoint carries audio facts"),
                            };
                        AudioEndpointPacket {
                            source_index: endpoint.source_index(),
                            source_sample_index: endpoint.source_sample_index(),
                            mapped_time: match endpoint.mapped_time() {
                                MappedSourceTime::Static => MappedSourceTimePacket::Static,
                                MappedSourceTime::Exact(time) => {
                                    MappedSourceTimePacket::Exact { time }
                                }
                                MappedSourceTime::HoldStart => MappedSourceTimePacket::HoldStart,
                                MappedSourceTime::HoldEnd => MappedSourceTimePacket::HoldEnd,
                            },
                            digest: resource.map(|resource| *resource.digest()),
                            handle: resource.map(|resource| resource.handle().get()),
                            decoded_pcm_digest,
                            source_channels,
                            crossfade_gain: endpoint.crossfade_gain(),
                            gain: endpoint.gain(),
                            pan: endpoint.pan(),
                            left_gain: endpoint.left_gain(),
                            right_gain: endpoint.right_gain(),
                        }
                    })
                    .collect(),
            })
            .collect(),
    }
}

fn open_product_fixed_package(
    fixed_package_manifest_json: &str,
    timeline_json: &str,
    manifest_json: &str,
    verified_binding_bundle_json: &str,
) -> Result<crate::fixed_package::OpenedFixedPackage, String> {
    let execution_profile_json = canonical_fixed_execution_profile(COMMON_PROFILE_KEY)?;
    let files = fixed_package_files(
        timeline_json,
        manifest_json,
        verified_binding_bundle_json,
        &execution_profile_json,
    );
    open_verified_fixed_package(fixed_package_manifest_json, &files)
        .map_err(|error| error.to_string())
}

fn parse_json<T: serde::de::DeserializeOwned>(json: &str, kind: &str) -> Result<T, JsError> {
    serde_json::from_str(json)
        .map_err(|error| JsError::new(&format!("[input_parse] invalid {kind}: {error}")))
}

fn glass_transform(values: &[f64]) -> Result<[f64; 9], JsError> {
    let transform: [f64; 9] = values.try_into().map_err(|_| {
        JsError::new("[motion_glass_transform] ownerToDevice must contain exactly 9 values")
    })?;
    if !transform.iter().all(|value| value.is_finite()) {
        return Err(JsError::new(
            "[motion_glass_transform] ownerToDevice must be finite",
        ));
    }
    Ok(transform)
}

fn unknown_ticket(ticket: u32) -> JsError {
    JsError::new(&format!("[unknown_ticket] ticket {ticket} does not exist"))
}

fn js_error(code: &str, error: impl std::fmt::Display) -> JsError {
    JsError::new(&format!("[{code}] {error}"))
}

fn parse_scene3d_request(
    canonical_request: &[u8],
) -> Result<valle_motion::Scene3DFrameRequest, JsError> {
    let frame: valle_motion::Scene3DFrameRequest = serde_json::from_slice(canonical_request)
        .map_err(|error| js_error("scene3d_request", error))?;
    if frame
        .canonical_bytes()
        .map_err(|error| js_error("scene3d_request", error))?
        != canonical_request
    {
        return Err(JsError::new("[scene3d_request] payload is not canonical"));
    }
    Ok(frame)
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
        let files = fixed_package_files(&timeline, &resource_manifest, &bundle, &execution_profile);
        let fixed_package_manifest = canonical_fixed_package_manifest(&files).unwrap();
        (fixed_package_manifest, timeline, resource_manifest, bundle)
    }

    #[test]
    fn fixed_package_clocks_and_audio_are_render_scoped() {
        let (fixed_manifest, timeline, manifest, bundle) = fixed_package_fixture();
        let mut engine = ProductEngine::new();
        let first: serde_json::Value = serde_json::from_str(
            &engine
                .open_fixed_package(&fixed_manifest, &timeline, &manifest, &bundle)
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
        let audio_sample: serde_json::Value =
            serde_json::from_str(&engine.audio_sample_json(render_id, 0).unwrap()).unwrap();
        assert_eq!(audio_sample["renderId"], render_id);
        assert_eq!(audio_sample["sample"], 0);
        let audio_block: serde_json::Value =
            serde_json::from_str(&engine.audio_block_json(render_id, 0, 3).unwrap()).unwrap();
        assert_eq!(audio_block["sampleRate"], 48_000);
        assert_eq!(audio_block["samples"].as_array().unwrap().len(), 3);

        engine.reset();
        let second: serde_json::Value = serde_json::from_str(
            &engine
                .open_fixed_package(&fixed_manifest, &timeline, &manifest, &bundle)
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
                .open_fixed_package(&fixed_manifest, &timeline, &manifest, &bundle)
                .unwrap(),
        )
        .unwrap();
        let render_id = receipt["renderId"].as_str().unwrap();
        assert_eq!(engine.frame_at_seconds(render_id, 1.0 / 60.0).unwrap(), 1);
    }

    #[test]
    fn web_product_rejects_swapped_member_before_inner_admission() {
        let (fixed_manifest, timeline, resource_manifest, bundle) = fixed_package_fixture();
        let error =
            open_product_fixed_package(&fixed_manifest, &resource_manifest, &timeline, &bundle)
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
            open_product_fixed_package(&fixed_manifest, &timeline, &resource_manifest, &bundle,)
                .is_ok(),
            "the unchanged inner package must be admissible"
        );
        let mut manifest: serde_json::Value = serde_json::from_str(&fixed_manifest).unwrap();
        manifest["members"].as_array_mut().unwrap().pop();
        let missing = String::from_utf8(serde_jcs::to_vec(&manifest).unwrap()).unwrap();
        let error = open_product_fixed_package(&missing, &timeline, &resource_manifest, &bundle)
            .unwrap_err();
        assert!(
            error.starts_with("[fixed_package_manifest] expected 4 closed members"),
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
        )
        .unwrap_err();
        assert_eq!(
            error,
            "[fixed_package_manifest] manifest must use canonical JCS bytes"
        );
    }
}
