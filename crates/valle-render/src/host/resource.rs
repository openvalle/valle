//! Native fulfillment for the Product Compositor request contract.
//!
//! This module owns paths, bytes, decoder cursors and generated Scene3D rasters. It never sees a
//! authoring document or RenderGraph: every decision is driven by one immutable [`ResourceRequest`].

use std::{collections::BTreeMap, path::PathBuf, sync::Arc, time::Instant};

use skia_safe::{Data, images};
use thiserror::Error;
use valle_engine::{
    motion::{
        Scene3DFrameRequest, SealedScene3DFrame,
        scene3d::{
            AdmittedModel, MaterialImage, PreparedScene, ScenePrepareCacheKey, SceneResources,
            TextureRole, admit_glb, prepare_cache_key, prepare_scene, render_scene,
            render_scene_without_depth_of_field,
        },
    },
    resource::{
        ContentDigest, Extent2d, ExternalPixelLayout, ExternalResourceDesc, ResourceCacheIdentity,
        ResourceInterpretation, ResourcePayload, ResourceRequest, ResourceSample,
    },
};
use valle_media::codec::{LibavVideoSource, VideoFrameTransport, VideoSource};

use super::frame::{
    ResourceCacheFrameReport, ResourceFrameReport, ResourceProvider, Scene3dFrameReport,
};
use crate::executor::skia::{SkiaBackendKind, SkiaExternalObject, SkiaObjectError};

/// A content-addressed host source. File-backed entries keep large video bytes out of memory;
/// immutable byte entries support bundle/in-memory projects. A digest names one content, so a
/// second source for a known digest is redundant and ignored.
#[derive(Debug, Clone)]
pub enum NativeResourceSource {
    File(PathBuf),
    Bytes(Arc<[u8]>),
}

/// Complete content store for one opened project render.
#[derive(Debug, Clone, Default)]
pub struct NativeResourceCatalog {
    sources: BTreeMap<ContentDigest, NativeResourceSource>,
}

impl NativeResourceCatalog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a file under the digest its caller computed from it. Bytes read back for
    /// rendering are checked against the digest, so a file edited mid-render fails the frame.
    pub fn insert_file(&mut self, digest: ContentDigest, path: impl Into<PathBuf>) {
        self.sources
            .entry(digest)
            .or_insert_with(|| NativeResourceSource::File(path.into()));
    }

    /// Register in-memory bytes under the digest its caller computed from them.
    pub fn insert_bytes(&mut self, digest: ContentDigest, bytes: impl Into<Arc<[u8]>>) {
        self.sources
            .entry(digest)
            .or_insert_with(|| NativeResourceSource::Bytes(bytes.into()));
    }

    /// Hash and admit in-memory bytes once, returning their digest.
    pub fn admit_bytes(&mut self, bytes: impl Into<Arc<[u8]>>) -> ContentDigest {
        let bytes = bytes.into();
        let digest = ContentDigest::of_bytes(&bytes);
        self.sources
            .entry(digest)
            .or_insert(NativeResourceSource::Bytes(bytes));
        digest
    }

    pub fn source(&self, digest: &ContentDigest) -> Option<&NativeResourceSource> {
        self.sources.get(digest)
    }
}

const DEFAULT_RESOURCE_CACHE_ENTRIES: usize = 256;
const DEFAULT_RESOURCE_CACHE_BYTES: u64 = 512 * 1024 * 1024;
/// Time-varying objects (a video frame at one source time, a Scene3D frame raster) are
/// seldom requested twice: parallel workers take interleaved frames, so reuse only happens when a
/// held or slowed source repeats within one worker. Keeping only the most recent few stops them
/// from filling the byte budget with full-frame working-space copies (~16.6 MB each at 1080p).
const MAX_TRANSIENT_RESOURCE_ENTRIES: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeResourceCacheLimits {
    max_entries: usize,
    max_bytes: u64,
}

impl NativeResourceCacheLimits {
    pub fn new(max_entries: usize, max_bytes: u64) -> Result<Self, NativeResourceError> {
        if max_entries == 0 || max_bytes == 0 {
            return Err(NativeResourceError::InvalidCacheLimits);
        }
        Ok(Self {
            max_entries,
            max_bytes,
        })
    }

    pub const fn max_entries(self) -> usize {
        self.max_entries
    }

    pub const fn max_bytes(self) -> u64 {
        self.max_bytes
    }
}

impl Default for NativeResourceCacheLimits {
    fn default() -> Self {
        Self {
            max_entries: DEFAULT_RESOURCE_CACHE_ENTRIES,
            max_bytes: DEFAULT_RESOURCE_CACHE_BYTES,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct NativeResourceCacheIdentity {
    generation: u64,
    request: ResourceCacheIdentity,
}

struct CachedResourceObject {
    object: SkiaExternalObject,
    bytes: u64,
    last_used: u64,
    transient: bool,
}

struct NativeResourceObjectCache {
    limits: NativeResourceCacheLimits,
    entries: BTreeMap<NativeResourceCacheIdentity, CachedResourceObject>,
    resident_bytes: u64,
    transient_entries: usize,
    clock: u64,
}

impl NativeResourceObjectCache {
    fn new(limits: NativeResourceCacheLimits) -> Self {
        Self {
            limits,
            entries: BTreeMap::new(),
            resident_bytes: 0,
            transient_entries: 0,
            clock: 0,
        }
    }

    fn get(
        &mut self,
        identity: &NativeResourceCacheIdentity,
    ) -> Result<Option<SkiaExternalObject>, NativeResourceError> {
        let clock = self.tick()?;
        let Some(entry) = self.entries.get_mut(identity) else {
            return Ok(None);
        };
        entry.last_used = clock;
        Ok(Some(entry.object.clone()))
    }

    fn insert(
        &mut self,
        identity: NativeResourceCacheIdentity,
        object: SkiaExternalObject,
        bytes: u64,
        transient: bool,
        counters: &mut ResourceCacheCounters,
    ) -> Result<(), NativeResourceError> {
        if bytes > self.limits.max_bytes {
            increment(&mut counters.bypasses)?;
            return Ok(());
        }
        while transient && self.transient_entries >= MAX_TRANSIENT_RESOURCE_ENTRIES {
            self.evict_lru(true)?;
            increment(&mut counters.evictions)?;
        }
        while self.entries.len() >= self.limits.max_entries
            || self
                .resident_bytes
                .checked_add(bytes)
                .is_none_or(|required| required > self.limits.max_bytes)
        {
            self.evict_lru(false)?;
            increment(&mut counters.evictions)?;
        }
        if self.entries.contains_key(&identity) {
            return Err(NativeResourceError::CacheIdentityCollision);
        }
        let clock = self.tick()?;
        self.resident_bytes = self
            .resident_bytes
            .checked_add(bytes)
            .ok_or(NativeResourceError::CacheByteOverflow)?;
        self.entries.insert(
            identity,
            CachedResourceObject {
                object,
                bytes,
                last_used: clock,
                transient,
            },
        );
        if transient {
            self.transient_entries += 1;
        }
        increment(&mut counters.insertions)?;
        Ok(())
    }

    /// Evict the least recently used entry, or the least recently used transient one.
    fn evict_lru(&mut self, transient_only: bool) -> Result<(), NativeResourceError> {
        let identity = self
            .entries
            .iter()
            .filter(|(_, entry)| !transient_only || entry.transient)
            .min_by_key(|(identity, entry)| (entry.last_used, *identity))
            .map(|(identity, _)| identity.clone())
            .ok_or(NativeResourceError::CacheEvictionInvariant)?;
        let removed = self
            .entries
            .remove(&identity)
            .ok_or(NativeResourceError::CacheEvictionInvariant)?;
        self.resident_bytes = self
            .resident_bytes
            .checked_sub(removed.bytes)
            .ok_or(NativeResourceError::CacheEvictionInvariant)?;
        if removed.transient {
            self.transient_entries -= 1;
        }
        Ok(())
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.resident_bytes = 0;
        self.transient_entries = 0;
        self.clock = 0;
    }

    fn tick(&mut self) -> Result<u64, NativeResourceError> {
        self.clock = self
            .clock
            .checked_add(1)
            .ok_or(NativeResourceError::CacheCounterOverflow)?;
        Ok(self.clock)
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct ResourceCacheCounters {
    requests: u64,
    hits: u64,
    misses: u64,
    insertions: u64,
    evictions: u64,
    bypasses: u64,
}

/// Mutable resource ownership. Native schedules share one instance between preparation tasks;
/// returned immutable images retain their own lifetime independently of this cache.
pub struct NativeResourceProvider {
    catalog: Arc<NativeResourceCatalog>,
    runtime_shaders: BTreeMap<(ContentDigest, ContentDigest), Arc<[u8]>>,
    objects: NativeResourceObjectCache,
    cache_generation: u64,
    cache_invalidations: u64,
    reported_cache_invalidations: u64,
    frame_cache: ResourceCacheCounters,
    frame_scene3d: Scene3dFrameReport,
    bytes: BTreeMap<ContentDigest, Arc<[u8]>>,
    video: BTreeMap<ContentDigest, LibavVideoSource>,
    video_memory: BTreeMap<ContentDigest, Option<valle_engine::motion::memory::MemoryLease>>,
    video_transport: VideoFrameTransport,
    defer_scene3d_dof: bool,
    defer_scene3d_geometry: bool,
    models: BTreeMap<ContentDigest, Arc<AdmittedModel>>,
    environments: BTreeMap<ContentDigest, Arc<valle_engine::motion::scene3d::EnvironmentAsset>>,
    textures: BTreeMap<(ContentDigest, TextureRole), Arc<MaterialImage>>,
    scenes: BTreeMap<ScenePrepareCacheKey, Arc<PreparedScene>>,
}

impl NativeResourceProvider {
    pub fn new(catalog: Arc<NativeResourceCatalog>) -> Self {
        Self::with_cache_limits(catalog, NativeResourceCacheLimits::default())
    }

    pub fn with_cache_limits(
        catalog: Arc<NativeResourceCatalog>,
        limits: NativeResourceCacheLimits,
    ) -> Self {
        Self {
            catalog,
            runtime_shaders: BTreeMap::new(),
            objects: NativeResourceObjectCache::new(limits),
            cache_generation: 1,
            cache_invalidations: 0,
            reported_cache_invalidations: 0,
            frame_cache: ResourceCacheCounters::default(),
            frame_scene3d: Scene3dFrameReport::default(),
            bytes: BTreeMap::new(),
            video: BTreeMap::new(),
            video_memory: BTreeMap::new(),
            video_transport: VideoFrameTransport::Cpu,
            defer_scene3d_dof: false,
            defer_scene3d_geometry: false,
            models: BTreeMap::new(),
            environments: BTreeMap::new(),
            textures: BTreeMap::new(),
            scenes: BTreeMap::new(),
        }
    }

    /// Freeze backend payloads from the admitted render. Shader package identity is not the
    /// digest of generated SkSL; the engine has already verified and lowered the package.
    pub fn with_render_resources(mut self, render: &valle_engine::render::CompiledRender) -> Self {
        for (digest, environment) in render.admitted_environments() {
            self.environments.insert(*digest, Arc::clone(environment));
        }
        for (digest, model) in render.admitted_models() {
            self.models.insert(*digest, Arc::clone(model));
        }
        for resource in render.execution_resources() {
            if resource.kind() == valle_engine::render::CompiledExecutionResourceKind::Model3dBytes
            {
                self.bytes
                    .insert(*resource.content_digest(), Arc::from(resource.bytes()));
            }
            if resource.kind() == valle_engine::render::CompiledExecutionResourceKind::RuntimeShader
                && let Some(abi) = resource.abi_digest()
            {
                self.runtime_shaders.insert(
                    (*resource.content_digest(), *abi),
                    Arc::from(resource.bytes()),
                );
            }
        }
        self
    }

    pub(super) fn retain_media(&mut self, fulfilled_requests: &[ResourceRequest]) {
        self.video_memory.retain(|digest, _| {
            fulfilled_requests.iter().any(|request| {
                &request.key().content == digest
                    && matches!(request.sample(), ResourceSample::SourceTime(_))
            })
        });
        self.video.retain(|digest, _| {
            fulfilled_requests.iter().any(|request| {
                &request.key().content == digest
                    && matches!(request.sample(), ResourceSample::SourceTime(_))
            })
        });
    }

    pub(super) fn frame_report(&mut self) -> ResourceFrameReport {
        let generation_invalidations = self.cache_invalidations - self.reported_cache_invalidations;
        self.reported_cache_invalidations = self.cache_invalidations;
        ResourceFrameReport {
            cache: ResourceCacheFrameReport {
                generation: self.cache_generation,
                generation_invalidations,
                requests: self.frame_cache.requests,
                hits: self.frame_cache.hits,
                misses: self.frame_cache.misses,
                insertions: self.frame_cache.insertions,
                evictions: self.frame_cache.evictions,
                bypasses: self.frame_cache.bypasses,
                resident_entries: self.objects.entries.len(),
                resident_bytes: self.objects.resident_bytes,
            },
            scene3d: self.frame_scene3d,
        }
    }

    pub fn catalog(&self) -> &Arc<NativeResourceCatalog> {
        &self.catalog
    }

    pub(super) fn lookup_request(
        &mut self,
        request: &ResourceRequest,
    ) -> Result<Option<SkiaExternalObject>, NativeResourceError> {
        increment(&mut self.frame_cache.requests)?;
        if matches!(request.expected(), ExternalResourceDesc::Scene3d) {
            increment(&mut self.frame_scene3d.requests)?;
        }
        let identity = NativeResourceCacheIdentity {
            generation: self.cache_generation,
            request: request.cache_identity()?,
        };
        if let Some(object) = self.objects.get(&identity)? {
            increment(&mut self.frame_cache.hits)?;
            return Ok(Some(object));
        }
        increment(&mut self.frame_cache.misses)?;
        Ok(None)
    }

    pub(super) fn cache_fulfilled(
        &mut self,
        request: &ResourceRequest,
        mut object: SkiaExternalObject,
    ) -> Result<SkiaExternalObject, NativeResourceError> {
        let identity = NativeResourceCacheIdentity {
            generation: self.cache_generation,
            request: request.cache_identity()?,
        };
        // Independent preparation tasks can finish the same immutable 3D request together.
        // Keep the first admitted allocation rather than retaining or rejecting its duplicate.
        if let Some(existing) = self.objects.get(&identity)? {
            return Ok(existing);
        }
        object.charge_memory()?;
        let bytes = object
            .resident_bytes()
            .ok_or(NativeResourceError::CacheByteOverflow)?;
        let transient = matches!(request.sample(), ResourceSample::SourceTime(_))
            || matches!(request.expected(), ExternalResourceDesc::Scene3d);
        self.objects.insert(
            identity,
            object.clone(),
            bytes,
            transient,
            &mut self.frame_cache,
        )?;
        Ok(object)
    }

    fn fulfill_visual(
        &mut self,
        request: &ResourceRequest,
        extent: Extent2d,
        layout: ExternalPixelLayout,
    ) -> Result<SkiaExternalObject, NativeResourceError> {
        let source = self.source(request.key().content)?;
        match request.sample() {
            ResourceSample::Static => {
                let bytes = self.source_bytes(&request.key().content, &source)?;
                let object = SkiaExternalObject::visual_encoded(
                    request.key().clone(),
                    layout,
                    extent,
                    &bytes,
                )?;
                Ok(object)
            }
            ResourceSample::SourceTime(time) => {
                let path = source_path(&source)?;
                if !self.video.contains_key(&request.key().content) {
                    let memory = valle_engine::motion::memory::reserve(
                        valle_engine::motion::memory::MemoryKind::Media,
                        u64::from(extent.width()) * u64::from(extent.height()) * 12,
                    )?;
                    self.video_memory.insert(request.key().content, memory);
                    self.video.insert(
                        request.key().content,
                        if valle_engine::motion::cpu::is_budgeted() {
                            LibavVideoSource::open_with_transport_and_threads(
                                &path,
                                self.video_transport,
                                1,
                            )
                        } else {
                            LibavVideoSource::open_with_transport(&path, self.video_transport)
                        }
                        .map_err(|error| NativeResourceError::Video {
                            digest: request.key().content.to_string(),
                            reason: error.to_string(),
                        })?,
                    );
                }
                let decoder = self
                    .video
                    .get_mut(&request.key().content)
                    .expect("decoder was inserted");
                let source_frame = decoder.frame_at_lazy(time.as_f64()).map_err(|error| {
                    NativeResourceError::Video {
                        digest: request.key().content.to_string(),
                        reason: error.to_string(),
                    }
                })?;
                if source_frame.decoded_gpu().is_some() {
                    return Ok(SkiaExternalObject::visual_decoded(
                        request.key().clone(),
                        layout,
                        extent,
                        source_frame,
                    )?);
                }
                let frame = source_frame
                    .rgba()
                    .map_err(|error| NativeResourceError::Video {
                        digest: request.key().content.to_string(),
                        reason: error.to_string(),
                    })?;
                if frame.width != extent.width() || frame.height != extent.height() {
                    return Err(NativeResourceError::VisualExtent {
                        expected_width: extent.width(),
                        expected_height: extent.height(),
                        actual_width: frame.width,
                        actual_height: frame.height,
                    });
                }
                Ok(SkiaExternalObject::visual_rgba8_as(
                    request.key().clone(),
                    layout,
                    extent,
                    &frame.data,
                )?)
            }
        }
    }

    pub(super) fn prepare_scene3d(
        &mut self,
        request: &ResourceRequest,
    ) -> Result<Scene3dWork, NativeResourceError> {
        let prepare_started = Instant::now();
        // The request's digests name this sealed frame, so its parsed request is used as is.
        let ResourcePayload::Scene3dFrame(sealed) = request
            .payload()
            .ok_or(NativeResourceError::MissingScenePayload)?;
        let sealed = sealed.clone();
        let frame = sealed.request();
        let extent = Extent2d::new(frame.width, frame.height).map_err(|error| {
            NativeResourceError::SceneRequest {
                reason: error.to_string(),
            }
        })?;
        let resources = self.scene_resources(frame)?;
        let cache_key = prepare_cache_key(&frame.scene, frame.width, frame.height, &resources)
            .map_err(|error| NativeResourceError::ScenePrepare {
                reason: error.to_string(),
            })?;
        let prepared = if let Some(prepared) = self.scenes.get(&cache_key) {
            increment(&mut self.frame_scene3d.prepared_cache_hits)?;
            Arc::clone(prepared)
        } else {
            increment(&mut self.frame_scene3d.prepared_cache_misses)?;
            let prepared = Arc::new(
                prepare_scene(&frame.scene, frame.width, frame.height, &resources).map_err(
                    |error| NativeResourceError::ScenePrepare {
                        reason: error.to_string(),
                    },
                )?,
            );
            // Immutable topology is small compared with frame pixels, but moving/generated
            // scenes must not accumulate for the duration of a long render.
            while self.scenes.len() >= 8 {
                self.scenes.pop_first();
            }
            self.scenes.insert(cache_key, Arc::clone(&prepared));
            prepared
        };
        add_elapsed(
            &mut self.frame_scene3d.prepare_us,
            prepare_started.elapsed(),
        )?;
        Ok(Scene3dWork {
            frame: sealed,
            prepared,
            extent,
            key: request.key().clone(),
            defer_dof: self.defer_scene3d_dof,
            defer_geometry: self.defer_scene3d_geometry,
        })
    }

    fn fulfill_scene3d(
        &mut self,
        request: &ResourceRequest,
    ) -> Result<SkiaExternalObject, NativeResourceError> {
        self.prepare_scene3d(request)?
            .render(&mut self.frame_scene3d)
    }

    fn scene_resources(
        &mut self,
        frame: &Scene3DFrameRequest,
    ) -> Result<SceneResources, NativeResourceError> {
        let mut resources = SceneResources::default();
        for mesh in &frame.scene.meshes {
            if let Some(control) = &mesh.model_control {
                let model_digest = binding_digest(frame, control)?;
                let model = self.model(&model_digest)?;
                resources.models.insert(control.clone(), model);
            }
            for (control, role) in mesh.texture_controls() {
                let texture_digest = binding_digest(frame, control)?;
                let texture = self.texture(&texture_digest, role)?;
                resources
                    .textures
                    .insert((control.to_owned(), role), texture);
            }
        }
        if let Some(environment) = &frame.scene.pbr.environment {
            let digest = binding_digest(frame, &environment.control)?;
            let asset = self.environments.get(&digest).ok_or_else(|| {
                NativeResourceError::SceneRequest {
                    reason: format!("environment {digest} has no admitted frozen payload"),
                }
            })?;
            resources
                .environments
                .insert(environment.control.clone(), Arc::clone(asset));
        }
        Ok(resources)
    }

    fn model(&mut self, digest: &ContentDigest) -> Result<Arc<AdmittedModel>, NativeResourceError> {
        if let Some(model) = self.models.get(digest) {
            return Ok(Arc::clone(model));
        }
        let bytes = if let Some(bytes) = self.bytes.get(digest) {
            Arc::clone(bytes)
        } else {
            let source = self.source(*digest)?;
            self.source_bytes(digest, &source)?
        };
        let model = Arc::new(
            admit_glb(&bytes).map_err(|error| NativeResourceError::Model {
                digest: digest.to_string(),
                reason: error.to_string(),
            })?,
        );
        self.models.insert(*digest, Arc::clone(&model));
        Ok(model)
    }

    fn texture(
        &mut self,
        digest: &ContentDigest,
        role: TextureRole,
    ) -> Result<Arc<MaterialImage>, NativeResourceError> {
        if let Some(texture) = self.textures.get(&(*digest, role)) {
            return Ok(Arc::clone(texture));
        }
        let bytes = if let Some(bytes) = self.bytes.get(digest) {
            Arc::clone(bytes)
        } else {
            let source = self.source(*digest)?;
            self.source_bytes(digest, &source)?
        };
        let texture = Arc::new(MaterialImage::from_encoded(&bytes, role).map_err(|error| {
            NativeResourceError::Texture {
                digest: digest.to_string(),
                reason: error.to_string(),
            }
        })?);
        self.textures.insert((*digest, role), Arc::clone(&texture));
        Ok(texture)
    }

    fn source(&self, digest: ContentDigest) -> Result<NativeResourceSource, NativeResourceError> {
        self.catalog
            .source(&digest)
            .cloned()
            .ok_or_else(|| NativeResourceError::MissingSource {
                digest: digest.to_string(),
            })
    }

    fn source_bytes(
        &mut self,
        digest: &ContentDigest,
        source: &NativeResourceSource,
    ) -> Result<Arc<[u8]>, NativeResourceError> {
        if let Some(bytes) = self.bytes.get(digest) {
            return Ok(Arc::clone(bytes));
        }
        let bytes: Arc<[u8]> = match source {
            NativeResourceSource::File(path) => {
                let bytes = std::fs::read(path).map_err(|error| NativeResourceError::Io {
                    path: path.clone(),
                    reason: error.to_string(),
                })?;
                let actual = ContentDigest::of_bytes(&bytes);
                if &actual != digest {
                    return Err(NativeResourceError::DigestMismatch {
                        expected: digest.to_string(),
                        actual: actual.to_string(),
                    });
                }
                Arc::from(bytes)
            }
            // Admitted bytes were hashed into their digest.
            NativeResourceSource::Bytes(bytes) => Arc::clone(bytes),
        };
        self.bytes.insert(*digest, Arc::clone(&bytes));
        Ok(bytes)
    }

    fn fulfill_uncached(
        &mut self,
        request: &ResourceRequest,
    ) -> Result<SkiaExternalObject, NativeResourceError> {
        match request.expected() {
            ExternalResourceDesc::VisualFrame {
                extent,
                pixel_layout,
            } => self.fulfill_visual(request, *extent, *pixel_layout),
            ExternalResourceDesc::DataTexture { extent } => {
                let digest = request.key().content;
                let source = self.source(digest)?;
                let bytes = self.source_bytes(&digest, &source)?;
                Ok(SkiaExternalObject::data_texture_encoded(
                    request.key().clone(),
                    *extent,
                    &bytes,
                )?)
            }
            ExternalResourceDesc::FontBytes => {
                let ResourceInterpretation::FontFace { face_index } = request.key().interpretation
                else {
                    return Err(NativeResourceError::WrongInterpretation {
                        expected: "fontFace",
                    });
                };
                let digest = request.key().content;
                let source = self
                    .source(digest)
                    .map_err(|_| NativeResourceError::MissingFont {
                        digest: digest.to_string(),
                        face_index,
                    })?;
                let bytes = self.source_bytes(&digest, &source)?;
                Ok(SkiaExternalObject::font_bytes(
                    request.key().clone(),
                    bytes,
                )?)
            }
            ExternalResourceDesc::RuntimeShader => {
                let ResourceInterpretation::RuntimeShader { abi_digest, .. } =
                    &request.key().interpretation
                else {
                    return Err(NativeResourceError::WrongInterpretation {
                        expected: "runtimeShader",
                    });
                };
                let digest = request.key().content;
                let bytes = self
                    .runtime_shaders
                    .get(&(digest, *abi_digest))
                    .cloned()
                    .ok_or_else(|| NativeResourceError::MissingShader {
                        content: digest.to_string(),
                        abi: abi_digest.to_string(),
                    })?;
                Ok(SkiaExternalObject::runtime_shader(
                    request.key().clone(),
                    digest,
                    *abi_digest,
                    bytes,
                )?)
            }
            ExternalResourceDesc::Scene3d => self.fulfill_scene3d(request),
            other => Err(NativeResourceError::UnsupportedDescriptor {
                descriptor: format!("{other:?}"),
            }),
        }
    }
}

/// Immutable geometry and frame state leave the shared cache before CPU rasterization begins.
pub(super) struct Scene3dWork {
    frame: SealedScene3DFrame,
    prepared: Arc<PreparedScene>,
    extent: Extent2d,
    key: valle_engine::resource::ResourceKey,
    defer_dof: bool,
    defer_geometry: bool,
}
impl Scene3dWork {
    pub(super) fn render(
        self,
        report: &mut Scene3dFrameReport,
    ) -> Result<SkiaExternalObject, NativeResourceError> {
        let Self {
            frame: sealed,
            prepared,
            extent,
            key,
            defer_dof,
            defer_geometry,
        } = self;
        let frame = sealed.request();
        let _working = valle_engine::motion::memory::reserve(
            valle_engine::motion::memory::MemoryKind::Temporary,
            u64::from(frame.width) * u64::from(frame.height) * 96,
        )?;
        if defer_geometry {
            let started = Instant::now();
            let geometry =
                valle_engine::motion::scene3d::prepare_gpu_geometry(&prepared, &frame.frame)
                    .map_err(|error| NativeResourceError::SceneRender {
                        reason: error.to_string(),
                    })?;
            add_elapsed(&mut report.prepare_us, started.elapsed())?;
            if let Some(geometry) = geometry {
                let object = SkiaExternalObject::scene3d_gpu(
                    key.clone(),
                    geometry,
                    Arc::clone(&prepared),
                    Arc::new(frame.frame.clone()),
                )?;
                // Tiny outputs with dense geometry can retain more vertices than the
                // current per-pixel admission reserves. Keep the CPU path for them.
                let geometry_allowance = u64::from(frame.width) * u64::from(frame.height) * 12;
                if object
                    .deferred_scene3d_gpu()
                    .and_then(|input| input.frame_bytes())
                    .is_some_and(|bytes| bytes <= geometry_allowance)
                {
                    return Ok(object);
                }
            }
        }
        let raster_started = Instant::now();
        let deferred_dof = defer_dof
            && frame
                .frame
                .camera
                .depth_of_field
                .is_some_and(|dof| dof.max_blur_radius > 0.0);
        let raster = if deferred_dof {
            render_scene_without_depth_of_field(&prepared, &frame.frame)
        } else {
            render_scene(&prepared, &frame.frame)
        }
        .map_err(|error| NativeResourceError::SceneRender {
            reason: error.to_string(),
        })?;
        add_elapsed(&mut report.raster_us, raster_started.elapsed())?;
        let upload_started = Instant::now();
        let object = if deferred_dof {
            let dof = frame
                .frame
                .camera
                .depth_of_field
                .expect("deferred DOF was checked");
            SkiaExternalObject::scene3d_deferred_dof(
                key.clone(),
                extent,
                &raster.premul_rgba16f,
                &raster.depth,
                dof.focus_distance,
                dof.max_blur_radius,
                frame.frame.camera.far,
            )?
        } else {
            SkiaExternalObject::scene3d_rgba16f(key.clone(), extent, &raster.premul_rgba16f)?
        };
        add_elapsed(&mut report.upload_us, upload_started.elapsed())?;
        Ok(object)
    }
}
impl ResourceProvider for NativeResourceProvider {
    type Error = NativeResourceError;

    fn trim_memory(&mut self) {
        self.objects.clear();
        // Keep bounded immutable topology. Clearing it whenever another frame waits for bytes
        // repeats geometry construction and serializes the shared provider under pressure.
        self.video.clear();
        self.video_memory.clear();
    }

    fn begin_frame(&mut self) {
        self.frame_cache = ResourceCacheCounters::default();
        self.frame_scene3d = Scene3dFrameReport::default();
    }

    fn fulfill(&mut self, request: &ResourceRequest) -> Result<SkiaExternalObject, Self::Error> {
        if let Some(object) = self.lookup_request(request)? {
            return Ok(object);
        }
        let object = self.fulfill_uncached(request)?;
        self.cache_fulfilled(request, object)
    }

    fn source_resident_bytes(&self, request: &ResourceRequest) -> u64 {
        let digest = request.key().content;
        match (request.expected(), &request.key().interpretation) {
            (ExternalResourceDesc::FontBytes, _) => match self.bytes.get(&digest) {
                Some(bytes) => bytes.len() as u64,
                None => match self.catalog.source(&digest) {
                    Some(NativeResourceSource::Bytes(bytes)) => bytes.len() as u64,
                    // An unreadable font fails in fulfilment instead.
                    Some(NativeResourceSource::File(path)) => {
                        std::fs::metadata(path).map_or(0, |file| file.len())
                    }
                    None => 0,
                },
            },
            (
                ExternalResourceDesc::RuntimeShader,
                ResourceInterpretation::RuntimeShader { abi_digest, .. },
            ) => self
                .runtime_shaders
                .get(&(digest, *abi_digest))
                .map_or(0, |bytes| bytes.len() as u64),
            _ => 0,
        }
    }

    fn finish_frame(&mut self, fulfilled_requests: &[ResourceRequest]) -> ResourceFrameReport {
        self.retain_media(fulfilled_requests);
        self.frame_report()
    }

    fn invalidate_generation(&mut self) -> Result<(), Self::Error> {
        self.cache_generation = self
            .cache_generation
            .checked_add(1)
            .ok_or(NativeResourceError::CacheGenerationExhausted)?;
        self.cache_invalidations = self
            .cache_invalidations
            .checked_add(1)
            .ok_or(NativeResourceError::CacheCounterOverflow)?;
        self.objects.clear();
        Ok(())
    }

    fn configure_backend(&mut self, backend: SkiaBackendKind) -> Result<(), Self::Error> {
        if !self.video.is_empty() {
            return Err(NativeResourceError::BackendConfiguredAfterDecode);
        }
        self.video_transport = match backend {
            SkiaBackendKind::Raster => VideoFrameTransport::Cpu,
            #[cfg(target_os = "macos")]
            SkiaBackendKind::Metal => VideoFrameTransport::SharedGpu,
        };
        let defer_scene3d_dof = backend != SkiaBackendKind::Raster
            && std::env::var_os("VALLE_SCENE3D_DOF_CPU_REFERENCE").is_none();
        let defer_scene3d_geometry = backend != SkiaBackendKind::Raster
            && std::env::var_os("VALLE_SCENE3D_RASTER_CPU_REFERENCE").is_none()
            && std::env::var_os("VALLE_SCENE3D_DOF_CPU_REFERENCE").is_none();
        if (defer_scene3d_dof != self.defer_scene3d_dof
            || defer_scene3d_geometry != self.defer_scene3d_geometry)
            && !self.objects.entries.is_empty()
        {
            // A deferred GPU image is not usable on Raster after a backend switch.
            self.invalidate_generation()?;
        }
        self.defer_scene3d_dof = defer_scene3d_dof;
        self.defer_scene3d_geometry = defer_scene3d_geometry;
        Ok(())
    }
}

fn increment(value: &mut u64) -> Result<(), NativeResourceError> {
    *value = value
        .checked_add(1)
        .ok_or(NativeResourceError::CacheCounterOverflow)?;
    Ok(())
}

fn add_elapsed(value: &mut u64, elapsed: std::time::Duration) -> Result<(), NativeResourceError> {
    let micros = u64::try_from(elapsed.as_micros())
        .map_err(|_| NativeResourceError::CacheCounterOverflow)?;
    *value = value
        .checked_add(micros)
        .ok_or(NativeResourceError::CacheCounterOverflow)?;
    Ok(())
}

fn binding_digest(
    frame: &Scene3DFrameRequest,
    control: &str,
) -> Result<ContentDigest, NativeResourceError> {
    let digest =
        frame
            .resource_bindings
            .get(control)
            .ok_or_else(|| NativeResourceError::SceneRequest {
                reason: format!("control {control:?} has no content binding"),
            })?;
    Ok(*digest)
}

fn source_path(source: &NativeResourceSource) -> Result<PathBuf, NativeResourceError> {
    match source {
        NativeResourceSource::File(path) => Ok(path.clone()),
        NativeResourceSource::Bytes(_) => Err(NativeResourceError::FileBackedRequired),
    }
}

#[derive(Debug, Error)]
pub enum NativeResourceError {
    #[error(transparent)]
    Memory(#[from] valle_engine::motion::memory::MemoryError),
    #[error("resource cache limits must have non-zero entry and byte capacity")]
    InvalidCacheLimits,
    #[error("resource cache byte accounting overflowed")]
    CacheByteOverflow,
    #[error("resource cache counter overflowed")]
    CacheCounterOverflow,
    #[error("resource cache generation id space exhausted")]
    CacheGenerationExhausted,
    #[error("resource cache canonical identity was inserted twice")]
    CacheIdentityCollision,
    #[error("resource cache LRU eviction accounting is inconsistent")]
    CacheEvictionInvariant,
    #[error("resource backend was selected after a decoder had already been opened")]
    BackendConfiguredAfterDecode,
    #[error("resource {digest} has no admitted host source")]
    MissingSource { digest: String },
    #[error("resource digest mismatch: expected {expected}, got {actual}")]
    DigestMismatch { expected: String, actual: String },
    #[error("resource I/O failed for {path}: {reason}")]
    Io { path: PathBuf, reason: String },
    #[error("this resource must be file-backed for streaming decode")]
    FileBackedRequired,
    #[error("external resource requires {expected} interpretation")]
    WrongInterpretation { expected: &'static str },
    #[error(
        "decoded visual extent mismatch: expected {expected_width}x{expected_height}, got {actual_width}x{actual_height}"
    )]
    VisualExtent {
        expected_width: u32,
        expected_height: u32,
        actual_width: u32,
        actual_height: u32,
    },
    #[error("video resource {digest} failed: {reason}")]
    Video { digest: String, reason: String },
    #[error("font {digest}:{face_index} has no render-owned bytes")]
    MissingFont { digest: String, face_index: u32 },
    #[error("runtime shader content {content} / ABI {abi} is not in the admitted render")]
    MissingShader { content: String, abi: String },
    #[error("Scene3D request has no typed frame payload")]
    MissingScenePayload,
    #[error("invalid Scene3D frame request: {reason}")]
    SceneRequest { reason: String },
    #[error("Scene3D model {digest} failed admission: {reason}")]
    Model { digest: String, reason: String },
    #[error("Scene3D texture {digest} failed admission: {reason}")]
    Texture { digest: String, reason: String },
    #[error("Scene3D prepare failed: {reason}")]
    ScenePrepare { reason: String },
    #[error("Scene3D render failed: {reason}")]
    SceneRender { reason: String },
    #[error("Skia executor does not accept external descriptor {descriptor}")]
    UnsupportedDescriptor { descriptor: String },
    #[error(transparent)]
    Object(#[from] SkiaObjectError),
    #[error(transparent)]
    Contract(#[from] valle_engine::resource::ResourceContractError),
}

/// Probe the same encoded image decoder used by native resource fulfillment.
pub fn probe_image_extent(bytes: &[u8]) -> Result<(u32, u32), NativeResourceError> {
    let digest = ContentDigest::of_bytes(bytes);
    let image =
        images::deferred_from_encoded_data(Data::new_copy(bytes), None).ok_or_else(|| {
            NativeResourceError::Texture {
                digest: digest.to_string(),
                reason: "encoded image cannot be decoded".into(),
            }
        })?;
    Ok((image.width() as u32, image.height() as u32))
}

#[cfg(test)]
mod tests {
    use super::*;
    use valle_engine::resource::{
        ColorDescription, ExternalHandleId, InputAlphaMode, ResourceKey, SignalLuminance,
        VisualInterpretation,
    };

    #[test]
    fn switching_back_to_raster_invalidates_deferred_resource_generation() {
        let mut provider = NativeResourceProvider::new(Arc::new(NativeResourceCatalog::new()));
        provider.defer_scene3d_dof = true;
        provider.defer_scene3d_geometry = true;
        let (identity, object) = cached_frame(0);
        provider
            .objects
            .insert(identity, object, 8, true, &mut provider.frame_cache)
            .unwrap();
        provider.configure_backend(SkiaBackendKind::Raster).unwrap();
        assert!(!provider.defer_scene3d_dof);
        assert!(!provider.defer_scene3d_geometry);
        assert!(provider.objects.entries.is_empty());
        assert_eq!(provider.cache_generation, 2);
    }

    fn cached_frame(id: u8) -> (NativeResourceCacheIdentity, SkiaExternalObject) {
        let key = ResourceKey::new(
            ContentDigest::of_bytes(&[id]),
            ResourceInterpretation::Visual {
                interpretation: VisualInterpretation::new(
                    ColorDescription::SRGB,
                    SignalLuminance::SDR_100,
                    InputAlphaMode::StraightCoverage,
                ),
            },
        );
        let extent = Extent2d::new(1, 1).unwrap();
        let request = ResourceRequest::new(
            ExternalHandleId::new(1).unwrap(),
            key.clone(),
            ResourceSample::Static,
            ExternalResourceDesc::VisualFrame {
                extent,
                pixel_layout: ExternalPixelLayout::Rgba8,
            },
        )
        .unwrap();
        (
            NativeResourceCacheIdentity {
                generation: 1,
                request: request.cache_identity().unwrap(),
            },
            SkiaExternalObject::visual_rgba8(key, extent, &[id, 0, 0, 255]).unwrap(),
        )
    }

    #[test]
    fn transient_limit_evicts_the_oldest_transient_and_preserves_static_resources() {
        let mut cache = NativeResourceObjectCache::new(NativeResourceCacheLimits::default());
        let mut counters = ResourceCacheCounters::default();
        let (static_id, object) = cached_frame(0);
        cache
            .insert(static_id.clone(), object, 8, false, &mut counters)
            .unwrap();
        let mut transient_ids = Vec::new();
        for id in 1..=4 {
            let (identity, object) = cached_frame(id);
            cache
                .insert(identity.clone(), object, 8, true, &mut counters)
                .unwrap();
            transient_ids.push(identity);
        }
        assert!(cache.get(&transient_ids[0]).unwrap().is_some());
        let (identity, object) = cached_frame(5);
        cache
            .insert(identity, object, 8, true, &mut counters)
            .unwrap();
        assert!(cache.get(&static_id).unwrap().is_some());
        assert!(cache.get(&transient_ids[0]).unwrap().is_some());
        assert!(cache.get(&transient_ids[1]).unwrap().is_none());
        assert_eq!(cache.transient_entries, 4);
        assert_eq!(cache.entries.len(), 5);
        assert_eq!(cache.resident_bytes, 40);
        assert_eq!(counters.evictions, 1);
    }

    #[test]
    fn transient_accounting_survives_global_eviction_and_generation_reset() {
        for limits in [
            NativeResourceCacheLimits::new(2, 1024).unwrap(),
            NativeResourceCacheLimits::new(100, 16).unwrap(),
        ] {
            let mut cache = NativeResourceObjectCache::new(limits);
            let mut counters = ResourceCacheCounters::default();
            for id in 0..12 {
                let (identity, object) = cached_frame(id);
                cache
                    .insert(identity, object, 8, id % 3 != 0, &mut counters)
                    .unwrap();
                assert!(cache.entries.len() <= 2);
                assert_eq!(cache.resident_bytes, cache.entries.len() as u64 * 8);
                assert_eq!(
                    cache.transient_entries,
                    cache.entries.values().filter(|e| e.transient).count()
                );
            }
            cache.clear();
            assert!(cache.entries.is_empty());
            assert_eq!(cache.resident_bytes, 0);
            assert_eq!(cache.transient_entries, 0);
            let (identity, object) = cached_frame(20);
            cache
                .insert(identity, object, 8, true, &mut counters)
                .unwrap();
            assert_eq!(cache.transient_entries, 1);
            assert_eq!(cache.resident_bytes, 8);
        }
    }
}
