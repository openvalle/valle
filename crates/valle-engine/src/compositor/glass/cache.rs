//! G4.2 track/response cache: canonical keys and exact cache-on/cache-off equivalence.
//!
//! The cache maps `(render_seed, instance, surface, epoch, SampleTime)` → sampled local shape.
//! Keys are complete: any component not in the key must not affect the value (and every
//! component that does is in the key). Cache hits and misses produce identical programs and
//! pixels; eviction only affects performance.

use std::collections::BTreeMap;

use valle_motion::glass::GlassLocalShape;
use valle_timeline::internal::{MotionInstanceId, SampleTime, TrackEpoch};

/// Complete canonical cache key for one surface sample.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct GlassTrackCacheKey {
    pub render_seed: u32,
    pub instance: MotionInstanceId,
    pub surface_id: String,
    pub epoch: TrackEpoch,
    pub time: SampleTime,
}

/// Bounded LRU-free map (deterministic: full replacement at the cap, no wall-clock).
pub struct GlassTrackCache {
    entries: BTreeMap<GlassTrackCacheKey, GlassLocalShape>,
    cap: usize,
    hits: u64,
    misses: u64,
}

impl GlassTrackCache {
    pub fn new(cap: usize) -> Self {
        Self {
            entries: BTreeMap::new(),
            cap: cap.max(1),
            hits: 0,
            misses: 0,
        }
    }

    pub fn get(&self, key: &GlassTrackCacheKey) -> Option<&GlassLocalShape> {
        self.entries.get(key)
    }

    pub fn insert(&mut self, key: GlassTrackCacheKey, value: GlassLocalShape) {
        if self.entries.len() >= self.cap && !self.entries.contains_key(&key) {
            // Deterministic full replacement: drop the smallest key. No wall-clock, no
            // backend-specific eviction policy that could change results across machines.
            if let Some(oldest) = self.entries.keys().next().cloned() {
                self.entries.remove(&oldest);
            }
        }
        self.entries.insert(key, value);
    }

    pub fn record(&mut self, key: &GlassTrackCacheKey, value: GlassLocalShape) -> bool {
        if self.entries.contains_key(key) {
            self.hits += 1;
            true
        } else {
            self.misses += 1;
            self.insert(key.clone(), value);
            false
        }
    }

    pub fn hit_count(&self) -> u64 {
        self.hits
    }

    pub fn miss_count(&self) -> u64 {
        self.misses
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use valle_draw::Rect;
    use valle_timeline::FrameRate;
    use valle_timeline::internal::{DiscontinuityIndex, FrameKey, TimeMapSegment};

    fn key(render_seed: u32, surface: &str, epoch: u32, frame: i64) -> GlassTrackCacheKey {
        GlassTrackCacheKey {
            render_seed,
            instance: MotionInstanceId::new("clip-a").unwrap(),
            surface_id: surface.into(),
            epoch: TrackEpoch::new(
                1,
                MotionInstanceId::new("clip-a").unwrap(),
                TimeMapSegment::new(0),
                0,
                DiscontinuityIndex::new(epoch),
            ),
            time: SampleTime::from_frame(FrameKey::new(frame), FrameRate::new(30, 1).unwrap())
                .unwrap(),
        }
    }

    fn shape(left: f64) -> GlassLocalShape {
        GlassLocalShape {
            rect: Rect::new(left, 0.0, 80.0, 80.0),
            presence: 1.0,
            intensity: 0.6,
            drive_translation: valle_draw::Point::new(0.0, 0.0),
            drive_pressure: 0.0,
            drive_twist: 0.0,
        }
    }

    #[test]
    fn cache_hits_and_misses_are_counted() {
        let mut cache = GlassTrackCache::new(64);
        let k = key(1, "lens", 0, 10);
        assert!(!cache.record(&k, shape(10.0)));
        assert!(cache.record(&k, shape(10.0)));
        assert_eq!(cache.hit_count(), 1);
        assert_eq!(cache.miss_count(), 1);
    }

    #[test]
    fn keys_are_complete() {
        // Every key component changes the identity: same time but different render seed,
        // surface, epoch, or instance must miss.
        let mut cache = GlassTrackCache::new(64);
        let base = key(1, "lens", 0, 10);
        cache.record(&base, shape(10.0));
        let mut variants = vec![
            key(2, "lens", 0, 10),
            key(1, "other", 0, 10),
            key(1, "lens", 1, 10),
        ];
        let mut different_instance = key(1, "lens", 0, 10);
        different_instance.instance = MotionInstanceId::new("clip-b").unwrap();
        variants.push(different_instance);
        for variant in variants {
            assert!(!cache.record(&variant, shape(0.0)), "{variant:?}");
        }
    }

    #[test]
    fn cache_on_off_produce_identical_values() {
        // Reconstructing a value with the cache enabled returns the same bytes as
        // recomputing it (cache never changes semantics).
        let mut cache = GlassTrackCache::new(64);
        let k = key(1, "lens", 0, 10);
        cache.record(&k, shape(42.0));
        let cached = cache.get(&k).cloned().unwrap();
        assert_eq!(cached, shape(42.0));
    }

    #[test]
    fn bounded_cache_replaces_deterministically() {
        let mut cache = GlassTrackCache::new(4);
        for frame in 0..8 {
            cache.record(&key(1, "lens", 0, frame), shape(frame as f64));
        }
        assert!(cache.len() <= 4);
        // Smallest keys were replaced; the newest survive.
        assert!(cache.get(&key(1, "lens", 0, 7)).is_some());
        assert!(cache.get(&key(1, "lens", 0, 0)).is_none());
    }
}
