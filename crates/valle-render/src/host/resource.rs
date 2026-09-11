//! Native fulfillment for the Product Compositor request contract.
//!
//! This module owns paths, bytes, decoder cursors and generated Scene3D rasters. It never sees a
//! authoring document or RenderGraph: every decision is driven by one immutable [`ResourceRequest`].

use std::{
    collections::BTreeMap,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

use sha2::{Digest, Sha256};
use skia_safe::{AlphaType, ColorSpace, ColorType, Data, ImageInfo, image::CachingHint, images};
use thiserror::Error;
use valle_engine::{
    motion::{
        Scene3DFrameRequest,
        scene3d::{
            AdmittedModel, PreparedScene, ScenePrepareCacheKey, SceneResources, TextureAsset,
            admit_glb, prepare_cache_key, prepare_scene, render_scene,
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
/// immutable byte entries support bundle/in-memory projects.
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

    /// Admit a local file after streaming its SHA-256. No decoder is opened during render
    /// construction, but an incorrect authored digest cannot survive into a frame request.
    pub fn insert_file(
        &mut self,
        digest: ContentDigest,
        path: impl Into<PathBuf>,
    ) -> Result<(), NativeResourceError> {
        let path = path.into();
        let actual = digest_file(&path)?;
        if actual != digest {
            return Err(NativeResourceError::DigestMismatch {
                expected: digest.to_string(),
                actual: actual.to_string(),
            });
        }
        self.insert_source(digest, NativeResourceSource::File(path))
    }

    /// Admit immutable bytes after verifying their content address.
    pub fn insert_bytes(
        &mut self,
        digest: ContentDigest,
        bytes: impl Into<Arc<[u8]>>,
    ) -> Result<(), NativeResourceError> {
        let bytes = bytes.into();
        let actual = content_digest(&bytes)?;
        if actual != digest {
            return Err(NativeResourceError::DigestMismatch {
                expected: digest.to_string(),
                actual: actual.to_string(),
            });
        }
        self.insert_source(digest, NativeResourceSource::Bytes(bytes))
    }

    /// Hash and admit a file when the caller has not precomputed its digest.
    pub fn admit_file(
        &mut self,
        path: impl Into<PathBuf>,
    ) -> Result<ContentDigest, NativeResourceError> {
        let path = path.into();
        let digest = digest_file(&path)?;
        self.insert_source(digest.clone(), NativeResourceSource::File(path))?;
        Ok(digest)
    }

    pub fn source(&self, digest: &ContentDigest) -> Option<&NativeResourceSource> {
        self.sources.get(digest)
    }

    fn insert_source(
        &mut self,
        digest: ContentDigest,
        source: NativeResourceSource,
    ) -> Result<(), NativeResourceError> {
        if let Some(existing) = self.sources.get(&digest) {
            if same_source(existing, &source)? {
                return Ok(());
            }
            return Err(NativeResourceError::ConflictingSource {
                digest: digest.to_string(),
            });
        }
        self.sources.insert(digest, source);
        Ok(())
    }
}

const DEFAULT_RESOURCE_CACHE_ENTRIES: usize = 256;
const DEFAULT_RESOURCE_CACHE_BYTES: u64 = 512 * 1024 * 1024;

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
}

struct NativeResourceObjectCache {
    limits: NativeResourceCacheLimits,
    entries: BTreeMap<NativeResourceCacheIdentity, CachedResourceObject>,
    resident_bytes: u64,
    clock: u64,
}

impl NativeResourceObjectCache {
    fn new(limits: NativeResourceCacheLimits) -> Self {
        Self {
            limits,
            entries: BTreeMap::new(),
            resident_bytes: 0,
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
        counters: &mut ResourceCacheCounters,
    ) -> Result<(), NativeResourceError> {
        if bytes > self.limits.max_bytes {
            increment(&mut counters.bypasses)?;
            return Ok(());
        }
        while self.entries.len() >= self.limits.max_entries
            || self
                .resident_bytes
                .checked_add(bytes)
                .is_none_or(|required| required > self.limits.max_bytes)
        {
            self.evict_lru()?;
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
            },
        );
        increment(&mut counters.insertions)?;
        Ok(())
    }

    fn evict_lru(&mut self) -> Result<(), NativeResourceError> {
        let identity = self
            .entries
            .iter()
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
        Ok(())
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.resident_bytes = 0;
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

/// Worker-local fulfillment state. Decoder and Scene3D caches are deliberately not shared across
/// frame workers; immutable source bytes and the Engine render remain shared.
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
    video_transport: VideoFrameTransport,
    models: BTreeMap<ContentDigest, Arc<AdmittedModel>>,
    textures: BTreeMap<ContentDigest, Arc<TextureAsset>>,
    scenes: BTreeMap<ScenePrepareCacheKey, Arc<PreparedScene>>,
    #[cfg(feature = "lottie")]
    lottie: BTreeMap<ContentDigest, super::lottie::LottieDocument>,
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
            video_transport: VideoFrameTransport::Cpu,
            models: BTreeMap::new(),
            textures: BTreeMap::new(),
            scenes: BTreeMap::new(),
            #[cfg(feature = "lottie")]
            lottie: BTreeMap::new(),
        }
    }

    /// Freeze backend payloads from the admitted render. Shader package identity is not the
    /// digest of generated SkSL; the engine has already verified and lowered the package.
    pub fn with_render_resources(mut self, render: &valle_engine::render::CompiledRender) -> Self {
        for resource in render.execution_resources() {
            if resource.kind() == valle_engine::render::CompiledExecutionResourceKind::RuntimeShader
            {
                if let Some(abi) = resource.abi_digest() {
                    self.runtime_shaders.insert(
                        (*resource.content_digest(), *abi),
                        Arc::from(resource.bytes()),
                    );
                }
            }
        }
        self
    }

    pub fn catalog(&self) -> &Arc<NativeResourceCatalog> {
        &self.catalog
    }

    fn fulfill_visual(
        &mut self,
        request: &ResourceRequest,
        extent: Extent2d,
        layout: ExternalPixelLayout,
    ) -> Result<SkiaExternalObject, NativeResourceError> {
        let source = self.source(request.key().content.clone())?;
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
            ResourceSample::SourceTime(time) if layout == ExternalPixelLayout::Rgba8 => {
                self.fulfill_lottie(request, extent, source, time.as_f64())
            }
            ResourceSample::SourceTime(time) => {
                let path = source_path(&source)?;
                if !self.video.contains_key(&request.key().content) {
                    self.video.insert(
                        request.key().content.clone(),
                        LibavVideoSource::open_with_transport(&path, self.video_transport)
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

    #[cfg(feature = "lottie")]
    fn fulfill_lottie(
        &mut self,
        request: &ResourceRequest,
        extent: Extent2d,
        source: NativeResourceSource,
        time_s: f64,
    ) -> Result<SkiaExternalObject, NativeResourceError> {
        let path = source_path(&source)?;
        if !self.lottie.contains_key(&request.key().content) {
            let document = super::lottie::LottieDocument::load(&path).map_err(|error| {
                NativeResourceError::Lottie {
                    digest: request.key().content.to_string(),
                    reason: error.to_string(),
                }
            })?;
            self.lottie.insert(request.key().content.clone(), document);
        }
        let frame = self
            .lottie
            .get(&request.key().content)
            .expect("Lottie document was inserted")
            .render_at(time_s, extent.width(), extent.height())
            .map_err(|error| NativeResourceError::Lottie {
                digest: request.key().content.to_string(),
                reason: error.to_string(),
            })?;
        Ok(SkiaExternalObject::visual_rgba8_as(
            request.key().clone(),
            ExternalPixelLayout::Rgba8,
            extent,
            &frame.data,
        )?)
    }

    #[cfg(not(feature = "lottie"))]
    fn fulfill_lottie(
        &mut self,
        request: &ResourceRequest,
        _extent: Extent2d,
        _source: NativeResourceSource,
        _time_s: f64,
    ) -> Result<SkiaExternalObject, NativeResourceError> {
        Err(NativeResourceError::Lottie {
            digest: request.key().content.to_string(),
            reason: "this build has no Skottie executor capability".into(),
        })
    }

    fn fulfill_scene3d(
        &mut self,
        request: &ResourceRequest,
    ) -> Result<SkiaExternalObject, NativeResourceError> {
        let prepare_started = Instant::now();
        let ResourcePayload::Scene3dFrame { canonical_request } = request
            .payload()
            .ok_or(NativeResourceError::MissingScenePayload)?;
        let frame: Scene3DFrameRequest =
            serde_json::from_slice(canonical_request).map_err(|error| {
                NativeResourceError::SceneRequest {
                    reason: error.to_string(),
                }
            })?;
        if frame.canonical_bytes().map_err(scene_request_error)? != *canonical_request {
            return Err(NativeResourceError::SceneRequest {
                reason: "payload is not canonical".into(),
            });
        }
        let content_digest = frame.content_digest().map_err(scene_request_error)?;
        if content_digest != request.key().content {
            return Err(NativeResourceError::SceneRequest {
                reason: "content hash does not match the canonical frame request".into(),
            });
        }
        let ResourceInterpretation::Scene3d { topology_digest } = &request.key().interpretation
        else {
            return Err(NativeResourceError::WrongInterpretation {
                expected: "scene3d",
            });
        };
        let frame_topology_digest = frame.topology_digest().map_err(scene_request_error)?;
        if frame_topology_digest != *topology_digest {
            return Err(NativeResourceError::SceneRequest {
                reason: "topology hash does not match the static scene".into(),
            });
        }
        let extent = Extent2d::new(frame.width, frame.height).map_err(|error| {
            NativeResourceError::SceneRequest {
                reason: error.to_string(),
            }
        })?;
        let resources = self.scene_resources(&frame)?;
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
            self.scenes.insert(cache_key, Arc::clone(&prepared));
            prepared
        };
        add_elapsed(
            &mut self.frame_scene3d.prepare_us,
            prepare_started.elapsed(),
        )?;
        let raster_started = Instant::now();
        let raster = render_scene(&prepared, &frame.frame).map_err(|error| {
            NativeResourceError::SceneRender {
                reason: error.to_string(),
            }
        })?;
        add_elapsed(&mut self.frame_scene3d.raster_us, raster_started.elapsed())?;
        let upload_started = Instant::now();
        let object =
            SkiaExternalObject::scene3d_rgba8(request.key().clone(), extent, &raster.premul_rgba8)?;
        add_elapsed(&mut self.frame_scene3d.upload_us, upload_started.elapsed())?;
        Ok(object)
    }

    fn scene_resources(
        &mut self,
        frame: &Scene3DFrameRequest,
    ) -> Result<SceneResources, NativeResourceError> {
        let mut resources = SceneResources::default();
        for mesh in &frame.scene.meshes {
            let model_digest = binding_digest(frame, &mesh.model_control)?;
            let model = self.model(&model_digest)?;
            resources.models.insert(mesh.model_control.clone(), model);
            if let Some(control) = &mesh.material.texture_control {
                let texture_digest = binding_digest(frame, control)?;
                let texture = self.texture(&texture_digest)?;
                resources.textures.insert(control.clone(), texture);
            }
        }
        Ok(resources)
    }

    fn model(&mut self, digest: &ContentDigest) -> Result<Arc<AdmittedModel>, NativeResourceError> {
        if let Some(model) = self.models.get(digest) {
            return Ok(Arc::clone(model));
        }
        let source = self.source(digest.clone())?;
        let bytes = self.source_bytes(digest, &source)?;
        let model = Arc::new(
            admit_glb(&bytes).map_err(|error| NativeResourceError::Model {
                digest: digest.to_string(),
                reason: error.to_string(),
            })?,
        );
        self.models.insert(digest.clone(), Arc::clone(&model));
        Ok(model)
    }

    fn texture(
        &mut self,
        digest: &ContentDigest,
    ) -> Result<Arc<TextureAsset>, NativeResourceError> {
        if let Some(texture) = self.textures.get(digest) {
            return Ok(Arc::clone(texture));
        }
        let source = self.source(digest.clone())?;
        let bytes = self.source_bytes(digest, &source)?;
        let image =
            images::deferred_from_encoded_data(Data::new_copy(&bytes), None).ok_or_else(|| {
                NativeResourceError::Texture {
                    digest: digest.to_string(),
                    reason: "encoded image cannot be decoded".into(),
                }
            })?;
        let width = u32::try_from(image.width()).map_err(|_| NativeResourceError::Texture {
            digest: digest.to_string(),
            reason: "decoded width is invalid".into(),
        })?;
        let height = u32::try_from(image.height()).map_err(|_| NativeResourceError::Texture {
            digest: digest.to_string(),
            reason: "decoded height is invalid".into(),
        })?;
        let info = ImageInfo::new(
            (image.width(), image.height()),
            ColorType::RGBA8888,
            AlphaType::Premul,
            Some(ColorSpace::new_srgb()),
        );
        let row_bytes = usize::try_from(width)
            .ok()
            .and_then(|value| value.checked_mul(4))
            .ok_or_else(|| NativeResourceError::Texture {
                digest: digest.to_string(),
                reason: "decoded row size overflows".into(),
            })?;
        let mut rgba = vec![
            0_u8;
            row_bytes
                .checked_mul(usize::try_from(height).unwrap_or(usize::MAX))
                .ok_or_else(|| NativeResourceError::Texture {
                    digest: digest.to_string(),
                    reason: "decoded byte size overflows".into(),
                })?
        ];
        if !image.read_pixels(&info, &mut rgba, row_bytes, (0, 0), CachingHint::Disallow) {
            return Err(NativeResourceError::Texture {
                digest: digest.to_string(),
                reason: "decoded pixels cannot be read".into(),
            });
        }
        let texture = Arc::new(TextureAsset::new(*digest, width, height, rgba).map_err(
            |error| NativeResourceError::Texture {
                digest: digest.to_string(),
                reason: error.to_string(),
            },
        )?);
        self.textures.insert(digest.clone(), Arc::clone(&texture));
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
            NativeResourceSource::File(path) => Arc::from(std::fs::read(path).map_err(
                |error| NativeResourceError::Io {
                    path: path.clone(),
                    reason: error.to_string(),
                },
            )?),
            NativeResourceSource::Bytes(bytes) => Arc::clone(bytes),
        };
        let actual = content_digest(&bytes)?;
        if &actual != digest {
            return Err(NativeResourceError::DigestMismatch {
                expected: digest.to_string(),
                actual: actual.to_string(),
            });
        }
        self.bytes.insert(digest.clone(), Arc::clone(&bytes));
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
            ExternalResourceDesc::FontBytes => {
                let ResourceInterpretation::FontFace { face_index } = request.key().interpretation
                else {
                    return Err(NativeResourceError::WrongInterpretation {
                        expected: "fontFace",
                    });
                };
                let digest = request.key().content.clone();
                let source =
                    self.source(digest.clone())
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
                let digest = request.key().content.clone();
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

impl ResourceProvider for NativeResourceProvider {
    type Error = NativeResourceError;

    fn begin_frame(&mut self) {
        self.frame_cache = ResourceCacheCounters::default();
        self.frame_scene3d = Scene3dFrameReport::default();
    }

    fn fulfill(&mut self, request: &ResourceRequest) -> Result<SkiaExternalObject, Self::Error> {
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
            return Ok(object);
        }
        increment(&mut self.frame_cache.misses)?;
        let object = self.fulfill_uncached(request)?;
        let bytes = object
            .resident_bytes()
            .ok_or(NativeResourceError::CacheByteOverflow)?;
        self.objects
            .insert(identity, object.clone(), bytes, &mut self.frame_cache)?;
        Ok(object)
    }

    fn finish_frame(&mut self, _fulfilled_requests: &[ResourceRequest]) -> ResourceFrameReport {
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

fn scene_request_error(error: impl ToString) -> NativeResourceError {
    NativeResourceError::SceneRequest {
        reason: error.to_string(),
    }
}

fn source_path(source: &NativeResourceSource) -> Result<PathBuf, NativeResourceError> {
    match source {
        NativeResourceSource::File(path) => Ok(path.clone()),
        NativeResourceSource::Bytes(_) => Err(NativeResourceError::FileBackedRequired),
    }
}

fn same_source(
    left: &NativeResourceSource,
    right: &NativeResourceSource,
) -> Result<bool, NativeResourceError> {
    match (left, right) {
        (NativeResourceSource::Bytes(left), NativeResourceSource::Bytes(right)) => {
            Ok(left.as_ref() == right.as_ref())
        }
        (NativeResourceSource::File(left), NativeResourceSource::File(right)) => {
            Ok(left == right || digest_file(left)? == digest_file(right)?)
        }
        (NativeResourceSource::File(path), NativeResourceSource::Bytes(bytes))
        | (NativeResourceSource::Bytes(bytes), NativeResourceSource::File(path)) => Ok(
            std::fs::read(path).map_err(|error| NativeResourceError::Io {
                path: path.clone(),
                reason: error.to_string(),
            })? == bytes.as_ref(),
        ),
    }
}

fn digest_file(path: &Path) -> Result<ContentDigest, NativeResourceError> {
    let mut file = File::open(path).map_err(|error| NativeResourceError::Io {
        path: path.to_owned(),
        reason: error.to_string(),
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| NativeResourceError::Io {
                path: path.to_owned(),
                reason: error.to_string(),
            })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(ContentDigest::from_bytes(hasher.finalize().into()))
}

fn content_digest(bytes: &[u8]) -> Result<ContentDigest, NativeResourceError> {
    Ok(ContentDigest::from_bytes(Sha256::digest(bytes).into()))
}

#[derive(Debug, Error)]
pub enum NativeResourceError {
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
    #[error("resource {digest} was admitted from conflicting sources")]
    ConflictingSource { digest: String },
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
    #[error("Lottie resource {digest} failed: {reason}")]
    Lottie { digest: String, reason: String },
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
