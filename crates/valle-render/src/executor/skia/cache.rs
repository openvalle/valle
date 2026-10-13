use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::Arc,
};

use skia_safe::{FontMgr, Image, Rect as SkRect, RuntimeEffect, Typeface};
use valle_draw::program::DrawProgram;
use valle_engine::{
    compositor::{
        ExternalObject,
        lower::{PlanProgram, PlanStructureBinding},
    },
    resource::{ContentDigest, ResourceKey},
};

use super::{SkiaExternalObject, draw::DrawError, surface::PlanImage};

const PROGRAM_CACHE_LIMIT: usize = 256;
const PROGRAM_CACHE_BYTES: usize = 32 * 1024 * 1024;
const FONT_CACHE_LIMIT: usize = 256;
const SHADER_CACHE_LIMIT: usize = 128;
const RASTER_LAYER_CACHE_LIMIT: usize = 64;
const RASTER_LAYER_CACHE_BYTES: usize = 64 * 1024 * 1024;
const LARGE_RASTER_LAYER_BYTES: usize = 8 * 1024 * 1024;
const LARGE_RASTER_HISTORY_LIMIT: usize = 256;

#[derive(Debug, Clone, Copy)]
pub(crate) struct SkiaCacheLimits {
    /// Skia counts both live and purgeable GPU resources toward this soft cache target.
    pub(crate) gpu_resources: usize,
    pub(crate) programs: usize,
    pub(crate) raster_layers: usize,
    pub(crate) opacity_layers: usize,
    pub(crate) surface_pool: u64,
}

/// A destination-independent RasterProgram result in the executor's working color space.
/// Full device-matrix bits retain subpixel phase; ROI retains tile clipping and origin.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct RasterLayerKey {
    pub(crate) program: ContentDigest,
    pub(crate) frame: ContentDigest,
    pub(crate) render_spec: ContentDigest,
    pub(crate) transform: [u64; 9],
    pub(crate) roi: (i32, i32, u32, u32),
}

pub(crate) struct RasterLayerCache {
    images: BoundedCache<RasterLayerKey, PlanImage>,
    observed_large: BoundedCache<RasterLayerKey, ()>,
    hits: u64,
    misses: u64,
}

impl std::fmt::Debug for RasterLayerCache {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RasterLayerCache")
            .field("entries", &self.images.values.len())
            .field("bytes", &self.images.weight)
            .finish()
    }
}

impl RasterLayerCache {
    pub(crate) fn set_byte_limit(&mut self, bytes: usize) {
        self.images = BoundedCache::new(RASTER_LAYER_CACHE_LIMIT).with_weight_limit(bytes);
        self.observed_large = BoundedCache::new(LARGE_RASTER_HISTORY_LIMIT);
    }
    pub(crate) fn new() -> Self {
        Self {
            images: BoundedCache::new(RASTER_LAYER_CACHE_LIMIT)
                .with_weight_limit(RASTER_LAYER_CACHE_BYTES),
            observed_large: BoundedCache::new(LARGE_RASTER_HISTORY_LIMIT),
            hits: 0,
            misses: 0,
        }
    }

    pub(crate) fn clear(&mut self) {
        let limit = self.images.maximum_weight;
        self.images = BoundedCache::new(RASTER_LAYER_CACHE_LIMIT).with_weight_limit(limit);
        self.observed_large = BoundedCache::new(LARGE_RASTER_HISTORY_LIMIT);
    }

    pub(crate) fn get(&mut self, key: &RasterLayerKey) -> Option<PlanImage> {
        match self.images.get(key) {
            Some(image) => {
                self.hits = self.hits.saturating_add(1);
                Some(image.clone())
            }
            None => {
                self.misses = self.misses.saturating_add(1);
                None
            }
        }
    }

    pub(crate) fn insert(&mut self, key: RasterLayerKey, mut image: PlanImage) {
        let weight = usize::try_from(image.roi().pixels().saturating_mul(8)).unwrap_or(usize::MAX);
        if weight > self.images.maximum_weight {
            return;
        }
        if weight >= LARGE_RASTER_LAYER_BYTES && image.image().is_some_and(Image::is_texture_backed)
        {
            // A full dynamic frame otherwise pins its snapshot and forces copy-on-write
            // on every subsequent GPU rasterization, despite never producing a cache hit.
            // Retain only bounded keys on the first GPU request. CPU images keep their
            // established admission/retirement behavior; dropping these snapshots earlier
            // increased measured CPU peak footprint without improving export throughput.
            let repeated = self.observed_large.get(&key).is_some();
            self.observed_large.insert(key.clone(), ());
            if !repeated {
                return;
            }
        }
        if image.admit_retention(weight as u64) {
            self.images.insert_weighted(key, image, weight);
        }
    }

    pub(crate) fn counters(&self) -> (u64, u64, usize, usize) {
        (
            self.hits,
            self.misses,
            self.images.values.len(),
            self.images.weight,
        )
    }
}

// GPU pages belong to one executor/context. Store pixels before the leaf's group
// opacity, so an Echo or a fade can reuse coverage without merging alpha stages.
const SMALL_OPACITY_CACHE_BYTES: usize = 32 * 1024 * 1024;
const SMALL_OPACITY_CACHE_ENTRIES: usize = 32_768;

pub(crate) struct SmallOpacityCache {
    layers: BTreeMap<ContentDigest, (Image, SkRect)>,
    pages: VecDeque<(
        Image,
        Vec<ContentDigest>,
        Option<valle_engine::motion::memory::MemoryLease>,
    )>,
    seen: BTreeSet<ContentDigest>,
    seen_order: VecDeque<ContentDigest>,
    bytes: usize,
    byte_limit: usize,
}

impl Default for SmallOpacityCache {
    fn default() -> Self {
        Self {
            layers: BTreeMap::new(),
            pages: VecDeque::new(),
            seen: BTreeSet::new(),
            seen_order: VecDeque::new(),
            bytes: 0,
            byte_limit: SMALL_OPACITY_CACHE_BYTES,
        }
    }
}

impl std::fmt::Debug for SmallOpacityCache {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SmallOpacityCache")
            .field("entries", &self.layers.len())
            .field("bytes", &self.bytes)
            .finish()
    }
}

impl SmallOpacityCache {
    // First observe reuse. Unique moving shapes retain ordinary saveLayer rendering
    // instead of paying for an atlas that will never be sampled on a later frame.
    pub(crate) fn observe(&mut self, keys: &[ContentDigest]) -> bool {
        let repeated = keys
            .iter()
            .filter(|key| self.seen.contains(key) || self.layers.contains_key(key))
            .count();
        for &key in keys {
            if self.seen.insert(key) {
                self.seen_order.push_back(key);
            }
        }
        while self.seen.len() > SMALL_OPACITY_CACHE_ENTRIES {
            self.seen.remove(&self.seen_order.pop_front().unwrap());
        }
        !keys.is_empty() && repeated >= keys.len().div_ceil(4)
    }

    pub(crate) fn get(&self, key: &ContentDigest) -> Option<&(Image, SkRect)> {
        self.layers.get(key)
    }

    pub(crate) fn insert_page(&mut self, image: Image, entries: Vec<(ContentDigest, SkRect)>) {
        let bytes = image.width() as usize * image.height() as usize * 8;
        let Ok(memory) = valle_engine::motion::memory::reserve(
            valle_engine::motion::memory::MemoryKind::Surface,
            bytes as u64,
        ) else {
            return;
        };
        self.bytes += bytes;
        let mut keys = Vec::with_capacity(entries.len());
        for (key, rect) in entries {
            keys.push(key);
            self.layers.insert(key, (image.clone(), rect));
        }
        self.pages.push_back((image, keys, memory));
        // Retire complete pages; counting only small occupied rectangles would
        // undercount the GPU allocation retained by their shared image.
        while self.bytes > self.byte_limit || self.layers.len() > SMALL_OPACITY_CACHE_ENTRIES {
            let (image, keys, _memory) = self.pages.pop_front().unwrap();
            self.bytes -= image.width() as usize * image.height() as usize * 8;
            for key in keys {
                if self
                    .layers
                    .get(&key)
                    .is_some_and(|(value, _)| value.unique_id() == image.unique_id())
                {
                    self.layers.remove(&key);
                }
            }
        }
    }

    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }
    pub(crate) fn under_pressure(&self) -> bool {
        self.bytes >= self.byte_limit / 4 * 3
    }
    pub(crate) fn clear(&mut self) {
        self.layers.clear();
        self.pages.clear();
        self.seen.clear();
        self.seen_order.clear();
        self.bytes = 0;
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct BackendCacheCounters {
    pub(crate) program_hits: u64,
    pub(crate) program_misses: u64,
    pub(crate) program_entries: usize,
    pub(crate) program_cost_bytes: usize,
    pub(crate) font_hits: u64,
    pub(crate) font_misses: u64,
    pub(crate) shader_hits: u64,
    pub(crate) shader_misses: u64,
}

impl BackendCacheCounters {
    pub(crate) fn since(self, before: Self) -> Self {
        Self {
            program_hits: self.program_hits.saturating_sub(before.program_hits),
            program_misses: self.program_misses.saturating_sub(before.program_misses),
            program_entries: self.program_entries,
            program_cost_bytes: self.program_cost_bytes,
            font_hits: self.font_hits.saturating_sub(before.font_hits),
            font_misses: self.font_misses.saturating_sub(before.font_misses),
            shader_hits: self.shader_hits.saturating_sub(before.shader_hits),
            shader_misses: self.shader_misses.saturating_sub(before.shader_misses),
        }
    }
}

#[derive(Debug)]
struct CacheEntry<V> {
    value: V,
    last_use: u64,
    weight: usize,
}

#[derive(Debug)]
struct BoundedCache<K, V> {
    values: BTreeMap<K, CacheEntry<V>>,
    maximum: usize,
    maximum_weight: usize,
    weight: usize,
    clock: u64,
}

impl<K: Ord + Clone, V> BoundedCache<K, V> {
    fn new(maximum: usize) -> Self {
        debug_assert!(maximum > 0);
        Self {
            values: BTreeMap::new(),
            maximum,
            maximum_weight: usize::MAX,
            weight: 0,
            clock: 0,
        }
    }

    fn with_weight_limit(mut self, maximum: usize) -> Self {
        self.maximum_weight = maximum;
        self
    }

    fn get(&mut self, key: &K) -> Option<&V> {
        self.clock = self.clock.saturating_add(1);
        let entry = self.values.get_mut(key)?;
        entry.last_use = self.clock;
        Some(&entry.value)
    }

    fn insert(&mut self, key: K, value: V) {
        self.insert_weighted(key, value, 1);
    }

    fn insert_weighted(&mut self, key: K, value: V, weight: usize) {
        self.clock = self.clock.saturating_add(1);
        if let Some(previous) = self.values.remove(&key) {
            self.weight -= previous.weight;
        }
        if weight > self.maximum_weight {
            return;
        }
        while self.values.len() >= self.maximum
            || self
                .weight
                .checked_add(weight)
                .is_none_or(|total| total > self.maximum_weight)
        {
            let oldest = self
                .values
                .iter()
                .min_by_key(|(key, entry)| (entry.last_use, (*key).clone()))
                .map(|(key, _)| key.clone());
            if let Some(oldest) = oldest {
                self.weight -= self.values.remove(&oldest).unwrap().weight;
            }
        }
        self.weight += weight;
        self.values.insert(
            key,
            CacheEntry {
                value,
                last_use: self.clock,
                weight,
            },
        );
    }
}

/// Immutable backend objects whose identity is independent from one frame's external handles.
/// Every cache is bounded and keyed by content plus interpretation; clearing it can only affect
/// admission cost, never pixels or plan identity.
#[derive(Debug)]
pub(crate) struct BackendCaches {
    programs: BoundedCache<ContentDigest, Arc<DrawProgram>>,
    fonts: BoundedCache<(ContentDigest, u32), Arc<Typeface>>,
    shaders: BoundedCache<ResourceKey, Arc<RuntimeEffect>>,
    counters: BackendCacheCounters,
    font_manager: FontMgr,
    pub(crate) opacity_layers: Arc<std::sync::Mutex<SmallOpacityCache>>,
}

impl BackendCaches {
    pub(crate) fn set_limits(&mut self, limits: SkiaCacheLimits) {
        self.programs = BoundedCache::new(PROGRAM_CACHE_LIMIT).with_weight_limit(limits.programs);
        let mut opacity = self
            .opacity_layers
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        opacity.clear();
        opacity.byte_limit = limits.opacity_layers;
    }
    pub(crate) fn new() -> Self {
        Self {
            programs: BoundedCache::new(PROGRAM_CACHE_LIMIT).with_weight_limit(PROGRAM_CACHE_BYTES),
            fonts: BoundedCache::new(FONT_CACHE_LIMIT),
            shaders: BoundedCache::new(SHADER_CACHE_LIMIT),
            counters: BackendCacheCounters::default(),
            font_manager: FontMgr::default(),
            opacity_layers: Default::default(),
        }
    }

    pub(crate) fn counters(&self) -> BackendCacheCounters {
        BackendCacheCounters {
            program_entries: self.programs.values.len(),
            program_cost_bytes: self.programs.weight,
            ..self.counters
        }
    }

    /// Programs are keyed by the plan's content hash, the digest of their packed bytes.
    pub(crate) fn program(&mut self, plan: &PlanProgram) -> Result<Arc<DrawProgram>, DrawError> {
        if let Some(cached) = self.programs.get(plan.content_hash()) {
            self.counters.program_hits = self.counters.program_hits.saturating_add(1);
            return Ok(Arc::clone(cached));
        }

        self.counters.program_misses = self.counters.program_misses.saturating_add(1);
        // Native lowering already holds this frame's validated arena. Preserve its identity
        // instead of repeating packed JSON decode, geometry derivation, and canonical encoding
        // on the ordered GPU execution thread. Wire programs retain independent admission.
        let program = match plan.runtime_program() {
            Some(program) => Arc::clone(program),
            None => Arc::new(
                DrawProgram::from_packed(plan.packed())
                    .map_err(|error| DrawError::ProgramDecode(error.to_string()))?,
            ),
        };
        // Charge the packed payload plus decoded node/geometry slots. This is a bounded
        // retained-program cost, not an assertion about allocator or process RSS.
        let weight = plan
            .packed()
            .len()
            .saturating_add(program.nodes().len().saturating_mul(
                std::mem::size_of::<valle_draw::program::Node>()
                    + std::mem::size_of::<valle_draw::program::NodeGeometry>(),
            ));
        self.programs
            .insert_weighted(*plan.content_hash(), Arc::clone(&program), weight);
        Ok(program)
    }

    pub(crate) fn font(
        &mut self,
        key: (ContentDigest, u32),
        bytes: &[u8],
    ) -> Result<Typeface, DrawError> {
        if let Some(cached) = self.fonts.get(&key) {
            self.counters.font_hits = self.counters.font_hits.saturating_add(1);
            return Ok(cached.as_ref().clone());
        }

        self.counters.font_misses = self.counters.font_misses.saturating_add(1);
        // Skia takes a signed collection index; reject invalid input before its checked FFI cast.
        if key.1 > i32::MAX as u32 {
            return Err(DrawError::InvalidFont {
                hash: key.0.to_string(),
                index: key.1,
            });
        }
        // Typeface owns Skia's immutable font data. Share it between render workers while any
        // worker retains the face; weak entries never pin a retired font collection in memory.
        type Key = (ContentDigest, u32);
        static SHARED: std::sync::OnceLock<
            std::sync::Mutex<BTreeMap<Key, std::sync::Weak<Typeface>>>,
        > = std::sync::OnceLock::new();
        let mut shared = SHARED
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(face) = shared.get(&key).and_then(std::sync::Weak::upgrade) {
            self.fonts.insert(key, face.clone());
            return Ok(face.as_ref().clone());
        }
        shared.retain(|_, face| face.strong_count() > 0);
        let face = self
            .font_manager
            .new_from_bytes(bytes, key.1)
            .ok_or_else(|| DrawError::InvalidFont {
                hash: key.0.to_string(),
                index: key.1,
            })?;
        let face = Arc::new(face);
        shared.insert(key, Arc::downgrade(&face));
        self.fonts.insert(key, face.clone());
        Ok(face.as_ref().clone())
    }

    pub(crate) fn shader(
        &mut self,
        binding: &PlanStructureBinding,
        object: &SkiaExternalObject,
        uri: &str,
    ) -> Result<Arc<RuntimeEffect>, DrawError> {
        // A cache hit must not bypass the plan's declared structure binding. Physical cache
        // identity and logical admission identity are intentionally checked independently.
        if binding.key != uri {
            return Err(DrawError::MissingShader(uri.to_owned()));
        }
        let key = object.key().clone();
        if let Some(cached) = self.shaders.get(&key) {
            self.counters.shader_hits = self.counters.shader_hits.saturating_add(1);
            return Ok(Arc::clone(cached));
        }

        self.counters.shader_misses = self.counters.shader_misses.saturating_add(1);
        let bytes = object
            .shader_data()
            .ok_or_else(|| DrawError::MissingShader(uri.to_owned()))?;
        let source = core::str::from_utf8(bytes)
            .map_err(|_| DrawError::InvalidShaderEncoding(uri.to_owned()))?;
        let effect = Arc::new(
            RuntimeEffect::make_for_shader(source, None).map_err(|message| {
                DrawError::ShaderCompile {
                    uri: uri.to_owned(),
                    message,
                }
            })?,
        );
        self.shaders.insert(key, Arc::clone(&effect));
        Ok(effect)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn large_raster_key(index: u32) -> RasterLayerKey {
        RasterLayerKey {
            program: ContentDigest::of_bytes(&index.to_le_bytes()),
            frame: ContentDigest::of_bytes(b"frame"),
            render_spec: ContentDigest::of_bytes(b"spec"),
            transform: [0; 9],
            roi: (0, 0, 1536, 1024),
        }
    }

    #[test]
    fn large_cpu_layers_keep_immediate_admission() {
        let info = skia_safe::ImageInfo::new(
            (1536, 1024),
            skia_safe::ColorType::RGBAF16,
            skia_safe::AlphaType::Premul,
            None,
        );
        let image = PlanImage::new(
            skia_safe::surfaces::raster(&info, None, None)
                .unwrap()
                .image_snapshot(),
            valle_engine::prepare::DeviceRect::full(1536, 1024),
        );
        assert!(!image.image().unwrap().is_texture_backed());
        let mut cache = RasterLayerCache::new();
        cache.insert(large_raster_key(0), image);
        assert!(cache.get(&large_raster_key(0)).is_some());
        assert_eq!(cache.counters().3, 1536 * 1024 * 8);
        assert!(cache.observed_large.values.is_empty());
        cache.clear();
        assert!(cache.get(&large_raster_key(0)).is_none());
        assert_eq!(cache.counters().3, 0);
    }

    #[cfg(all(feature = "native", target_os = "macos"))]
    #[test]
    #[ignore = "requires a real Metal device"]
    fn large_gpu_layers_require_reuse_and_keep_bounded_cold_history() {
        use objc2_metal::MTLDevice;
        use skia_safe::gpu;
        let device = objc2_metal::MTLCreateSystemDefaultDevice().unwrap();
        let queue = device.newCommandQueue().unwrap();
        // SAFETY: BackendContext retains the live Metal device and command queue.
        let backend = unsafe {
            gpu::mtl::BackendContext::new(
                std::ptr::from_ref(&*device).cast(),
                std::ptr::from_ref(&*queue).cast(),
            )
        };
        let mut context = gpu::direct_contexts::make_metal(&backend, None).unwrap();
        let info = skia_safe::ImageInfo::new(
            (1536, 1024),
            skia_safe::ColorType::RGBAF16,
            skia_safe::AlphaType::Premul,
            None,
        );
        let mut surface = gpu::surfaces::render_target(
            &mut context,
            gpu::Budgeted::Yes,
            &info,
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        surface.canvas().clear(skia_safe::Color::TRANSPARENT);
        let image = PlanImage::new(
            surface.image_snapshot(),
            valle_engine::prepare::DeviceRect::full(1536, 1024),
        );
        assert!(image.image().unwrap().is_texture_backed());
        let key = large_raster_key;
        let mut cache = RasterLayerCache::new();
        assert!(cache.get(&key(0)).is_none());
        cache.insert(key(0), image.clone());
        assert_eq!(cache.counters().3, 0);
        assert!(cache.get(&key(0)).is_none());
        cache.insert(key(0), image.clone());
        assert!(cache.get(&key(0)).is_some());
        let retained = cache.counters().3;
        assert_eq!(retained, 1536 * 1024 * 8);
        for index in 1..1000 {
            cache.insert(key(index), image.clone());
            assert_eq!(cache.counters().3, retained);
            assert!(cache.observed_large.values.len() <= LARGE_RASTER_HISTORY_LIMIT);
        }
        cache.clear();
        cache.insert(key(0), image);
        assert!(cache.get(&key(0)).is_none());
        assert_eq!(cache.counters().3, 0);
    }

    #[test]
    fn opacity_cache_observes_reuse_and_bounds_its_history() {
        let key = |index: u32| {
            let mut bytes = [0; 32];
            bytes[..4].copy_from_slice(&index.to_le_bytes());
            ContentDigest::from_bytes(bytes)
        };
        let mut cache = SmallOpacityCache::default();
        assert!(!cache.observe(&[key(0)]));
        assert!(cache.observe(&[key(0)]));
        assert!(!cache.observe(&[key(1)]));
        let unique = (2..SMALL_OPACITY_CACHE_ENTRIES as u32 + 4)
            .map(key)
            .collect::<Vec<_>>();
        assert!(!cache.observe(&unique));
        assert_eq!(cache.seen.len(), SMALL_OPACITY_CACHE_ENTRIES);
        assert_eq!(cache.seen_order.len(), SMALL_OPACITY_CACHE_ENTRIES);
        assert!(!cache.seen.contains(&key(0)));
        assert!(cache.observe(&unique[unique.len() - 4..]));
        cache.clear();
        assert!(!cache.observe(&unique[unique.len() - 4..]));
    }

    #[test]
    fn opacity_cache_retires_whole_pages_without_removing_newer_entries() {
        let image = |size| {
            let info = skia_safe::ImageInfo::new(
                (size, size),
                skia_safe::ColorType::RGBAF16,
                skia_safe::AlphaType::Premul,
                None,
            );
            skia_safe::surfaces::raster(&info, None, None)
                .unwrap()
                .image_snapshot()
        };
        let a = ContentDigest::of_bytes(b"a");
        let b = ContentDigest::of_bytes(b"b");
        let c = ContentDigest::of_bytes(b"c");
        let rect = SkRect::from_wh(1.0, 1.0);
        let mut cache = SmallOpacityCache {
            byte_limit: 64,
            ..SmallOpacityCache::default()
        };
        cache.insert_page(image(2), vec![(a, rect), (b, rect)]);
        let newer = image(2);
        cache.insert_page(newer.clone(), vec![(a, rect)]);
        cache.insert_page(image(2), vec![(c, rect)]);
        assert_eq!(cache.bytes(), 64);
        assert!(cache.get(&b).is_none());
        assert_eq!(cache.get(&a).unwrap().0.unique_id(), newer.unique_id());
        assert!(cache.get(&c).is_some());
        // Charge the full page, even though its only occupied rectangle is 1x1.
        cache.insert_page(image(4), vec![(a, rect)]);
        assert_eq!(cache.bytes(), 0);
        assert!(cache.layers.is_empty());
        assert!(cache.pages.is_empty());
    }

    #[test]
    fn byte_budget_evicts_cold_entries_and_bypasses_oversized_programs() {
        let mut cache = BoundedCache::new(256).with_weight_limit(10);
        cache.insert_weighted(1, "first", 4);
        cache.insert_weighted(2, "second", 4);
        assert_eq!(cache.get(&1), Some(&"first"));
        cache.insert_weighted(3, "third", 5);
        assert_eq!(cache.get(&2), None);
        assert_eq!(cache.get(&1), Some(&"first"));
        assert_eq!(cache.weight, 9);
        cache.insert_weighted(3, "replacement", 2);
        assert_eq!(cache.weight, 6);
        cache.insert_weighted(4, "oversized", 11);
        assert_eq!(cache.get(&4), None);
        assert_eq!(cache.weight, 6);
        for frame in 0..10_000 {
            cache.insert_weighted(frame + 10, "dynamic program", 3);
            assert!(cache.weight <= 10);
            assert!(cache.values.len() <= 3);
        }
    }

    #[test]
    fn out_of_range_font_collection_index_returns_an_error() {
        let mut caches = BackendCaches::new();
        let digest = ContentDigest::of_bytes(b"font");
        assert!(matches!(
            caches.font((digest, u32::MAX), b"font"),
            Err(DrawError::InvalidFont {
                index: u32::MAX,
                ..
            })
        ));
    }
}
