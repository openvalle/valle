use std::{collections::BTreeMap, sync::Arc};

use skia_safe::{FontMgr, RuntimeEffect, Typeface};
use valle_draw::program::DrawProgram;
use valle_engine::{
    compositor::{
        ExternalObject,
        lower::{PlanProgram, PlanStructureBinding},
    },
    resource::{ContentDigest, ResourceKey},
};

use super::{SkiaExternalObject, draw::DrawError};

const PROGRAM_CACHE_LIMIT: usize = 256;
const PROGRAM_CACHE_BYTES: usize = 32 * 1024 * 1024;
const FONT_CACHE_LIMIT: usize = 256;
const SHADER_CACHE_LIMIT: usize = 128;

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
struct CachedProgram {
    packed_digest: ContentDigest,
    program: Arc<DrawProgram>,
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
    programs: BoundedCache<ContentDigest, CachedProgram>,
    fonts: BoundedCache<(ContentDigest, u32), Typeface>,
    shaders: BoundedCache<ResourceKey, Arc<RuntimeEffect>>,
    counters: BackendCacheCounters,
    font_manager: FontMgr,
}

impl BackendCaches {
    pub(crate) fn new() -> Self {
        Self {
            programs: BoundedCache::new(PROGRAM_CACHE_LIMIT).with_weight_limit(PROGRAM_CACHE_BYTES),
            fonts: BoundedCache::new(FONT_CACHE_LIMIT),
            shaders: BoundedCache::new(SHADER_CACHE_LIMIT),
            counters: BackendCacheCounters::default(),
            font_manager: FontMgr::default(),
        }
    }

    pub(crate) fn counters(&self) -> BackendCacheCounters {
        BackendCacheCounters {
            program_entries: self.programs.values.len(),
            program_cost_bytes: self.programs.weight,
            ..self.counters
        }
    }

    pub(crate) fn program(&mut self, plan: &PlanProgram) -> Result<Arc<DrawProgram>, DrawError> {
        let packed_digest = ContentDigest::of_bytes(plan.packed());
        if let Some(cached) = self.programs.get(plan.content_hash()) {
            self.counters.program_hits = self.counters.program_hits.saturating_add(1);
            if cached.packed_digest != packed_digest {
                return Err(DrawError::ProgramCacheCollision {
                    content_hash: plan.content_hash().to_string(),
                });
            }
            return Ok(Arc::clone(&cached.program));
        }

        self.counters.program_misses = self.counters.program_misses.saturating_add(1);
        let program = Arc::new(
            DrawProgram::from_packed(plan.packed())
                .map_err(|error| DrawError::ProgramDecode(error.to_string()))?,
        );
        // Charge the packed payload plus decoded node/geometry slots. This is a bounded
        // retained-program cost, not an assertion about allocator or process RSS.
        let weight = plan
            .packed()
            .len()
            .saturating_add(program.nodes().len().saturating_mul(
                std::mem::size_of::<valle_draw::program::Node>()
                    + std::mem::size_of::<valle_draw::program::NodeGeometry>(),
            ));
        self.programs.insert_weighted(
            *plan.content_hash(),
            CachedProgram {
                packed_digest,
                program: Arc::clone(&program),
            },
            weight,
        );
        Ok(program)
    }

    pub(crate) fn font(
        &mut self,
        key: (ContentDigest, u32),
        bytes: &[u8],
    ) -> Result<Typeface, DrawError> {
        if let Some(cached) = self.fonts.get(&key) {
            self.counters.font_hits = self.counters.font_hits.saturating_add(1);
            return Ok(cached.clone());
        }

        self.counters.font_misses = self.counters.font_misses.saturating_add(1);
        // Skia takes a signed collection index; reject invalid input before its checked FFI cast.
        if key.1 > i32::MAX as u32 {
            return Err(DrawError::InvalidFont {
                hash: key.0.to_string(),
                index: key.1,
            });
        }
        let face = self
            .font_manager
            .new_from_bytes(bytes, key.1)
            .ok_or_else(|| DrawError::InvalidFont {
                hash: key.0.to_string(),
                index: key.1,
            })?;
        self.fonts.insert(key, face.clone());
        Ok(face)
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
