//! Shared, bounded preparation results for expensive Circle batches with Echo reuse.
//! This stores CPU geometry/color data only, before layout transforms, clipping or rasterization.

use std::{collections::VecDeque, io, ops::Range};

use super::bridge::BatchContent;
use crate::{MotionContext, ResolvedProps};

const MAX_PROPS_KEY_BYTES: usize = 16 * 1024;

#[derive(Debug, Default, Clone, Copy, serde::Serialize)]
pub struct InstanceCacheStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub bypasses: u64,
    pub entries: usize,
    /// Retained vector/key storage estimate; excludes allocator overhead and returned copies.
    pub resident_bytes: usize,
    pub max_entries: usize,
    pub max_bytes: usize,
}

#[derive(Debug, PartialEq)]
pub(super) struct BatchKey {
    group: usize,
    sample: valle_timeline::internal::SampleTime,
    frame: u32,
    progress_bits: u64,
    duration_frames: u32,
    fps: valle_timeline::FrameRate,
    host: Option<crate::MotionHostContext>,
    viewport: Option<(u64, u64)>,
    rows: Range<usize>,
    props: Vec<u8>,
}

impl BatchKey {
    pub(super) fn new(
        group: usize,
        ctx: &MotionContext,
        reads_host: bool,
        props: &ResolvedProps,
        viewport: Option<(f64, f64)>,
        rows: Range<usize>,
    ) -> Option<Self> {
        let mut key = PropsKey(Vec::new());
        serde_json::to_writer(&mut key, props.as_map()).ok()?;
        Some(Self {
            group,
            sample: ctx.sample,
            frame: ctx.local_frame,
            progress_bits: ctx.progress.to_bits(),
            duration_frames: ctx.duration_frames,
            fps: ctx.fps,
            host: reads_host.then_some(ctx.host),
            viewport: viewport.map(|(x, y)| (x.to_bits(), y.to_bits())),
            rows,
            props: key.0,
        })
    }
}

#[derive(Debug)]
struct Entry {
    key: BatchKey,
    content: BatchContent,
    bytes: usize,
}

#[derive(Debug)]
pub(super) struct InstanceBatchCache {
    entries: VecDeque<Entry>,
    stats: InstanceCacheStats,
}

impl Default for InstanceBatchCache {
    fn default() -> Self {
        Self::new(32, 8 * 1024 * 1024)
    }
}

impl InstanceBatchCache {
    pub(super) fn new(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            stats: InstanceCacheStats {
                max_entries,
                max_bytes,
                ..Default::default()
            },
        }
    }

    pub(super) fn stats(&self) -> InstanceCacheStats {
        self.stats
    }

    pub(super) fn enabled(&self) -> bool {
        self.stats.max_entries > 0 && self.stats.max_bytes > 0
    }

    pub(super) fn get(&mut self, key: &BatchKey) -> Option<BatchContent> {
        let Some(at) = self.entries.iter().position(|entry| entry.key == *key) else {
            self.stats.misses = self.stats.misses.saturating_add(1);
            return None;
        };
        let entry = self.entries.remove(at).expect("located batch");
        let content = entry.content.clone();
        self.entries.push_back(entry);
        self.stats.hits = self.stats.hits.saturating_add(1);
        Some(content)
    }

    pub(super) fn insert(&mut self, key: BatchKey, content: &BatchContent) {
        // Concurrent preparers may compute the same cold sample. Computation stays outside the
        // lock, and only the first result is retained; all results follow the same pure evaluator.
        if self.entries.iter().any(|entry| entry.key == key) {
            return;
        }
        let minimum_bytes = std::mem::size_of::<Entry>()
            + key.props.capacity()
            + content.instances.len()
                * std::mem::size_of::<valle_draw::program::recording::BatchInstance>()
            + content.semantic_keys.len() * std::mem::size_of::<String>()
            + content.semantic_keys.iter().map(String::len).sum::<usize>();
        if !self.enabled() || minimum_bytes > self.stats.max_bytes {
            self.stats.bypasses = self.stats.bypasses.saturating_add(1);
            return;
        }
        // Clone first: charge the capacities of the actual retained copies rather than temporary
        // buffers with different capacities. Circle batches have no path/atlas/path-style storage.
        let content = content.clone();
        debug_assert!(
            content.path.is_none() && content.atlas.is_none() && content.path_style.is_none()
        );
        let bytes = std::mem::size_of::<Entry>()
            + key.props.capacity()
            + content.instances.capacity()
                * std::mem::size_of::<valle_draw::program::recording::BatchInstance>()
            + content.semantic_keys.capacity() * std::mem::size_of::<String>()
            + content
                .semantic_keys
                .iter()
                .map(String::capacity)
                .sum::<usize>()
            + content.row_identity.capacity()
                * std::mem::size_of::<crate::batch::BatchRowIdentity>()
            + content.stroke_colors.capacity()
                * std::mem::size_of::<valle_draw::program::LinearColor>()
            + content.dash_offsets.capacity() * std::mem::size_of::<f32>();
        if !self.enabled() || bytes > self.stats.max_bytes {
            self.stats.bypasses = self.stats.bypasses.saturating_add(1);
            return;
        }
        while self.entries.len() >= self.stats.max_entries
            || bytes > self.stats.max_bytes - self.stats.resident_bytes
        {
            let removed = self.entries.pop_front().expect("bounded batch eviction");
            self.stats.resident_bytes -= removed.bytes;
            self.stats.evictions = self.stats.evictions.saturating_add(1);
        }
        self.stats.resident_bytes += bytes;
        self.entries.push_back(Entry {
            key,
            content,
            bytes,
        });
        self.stats.entries = self.entries.len();
    }
}

struct PropsKey(Vec<u8>);
impl io::Write for PropsKey {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_PROPS_KEY_BYTES - self.0.len() {
            return Err(io::Error::other("instance cache props key exceeds budget"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
