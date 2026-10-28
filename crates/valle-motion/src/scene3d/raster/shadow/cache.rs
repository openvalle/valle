use super::{FrameMesh, ShadowCasters, ShadowMap, V3, render_maps};
use crate::{
    ContentDigest, cpu,
    memory::{self, MemoryBudget, MemoryKind, MemoryLease},
};
use std::sync::{Arc, Mutex};

#[derive(Debug, Default)]
pub(in crate::scene3d::raster) struct ShadowMaps {
    pub maps: Vec<Option<ShadowMap>>,
    // Active frames keep both the maps and their charge alive across cache eviction.
    _memory: Option<MemoryLease>,
}

#[derive(Debug, Default)]
pub(in crate::scene3d::raster) struct ShadowCache {
    state: Mutex<State>,
}

#[derive(Debug, Default)]
struct State {
    budget: Option<MemoryBudget>,
    observed: Option<ContentDigest>,
    entry: Option<(ContentDigest, Arc<ShadowMaps>)>,
}

impl State {
    fn select_budget(&mut self) {
        let current = cpu::memory_budget();
        let same = match (&self.budget, &current) {
            (None, None) => true,
            (Some(a), Some(b)) => a.same_budget(b),
            _ => false,
        };
        if !same {
            self.observed = None;
            self.entry = None;
            self.budget = current;
        }
    }
}

impl ShadowCache {
    pub fn clear(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.entry = None;
        state.observed = None;
    }

    pub fn render(
        &self,
        directions: &[(V3, [f32; 3])],
        casters: &ShadowCasters,
        meshes: &[FrameMesh<'_>],
        workers: usize,
    ) -> Arc<ShadowMaps> {
        let key = casters.content_key(directions, meshes);
        let retain = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.select_budget();
            if let Some((stored, maps)) = &state.entry
                && *stored == key
            {
                return Arc::clone(maps);
            }
            // Avoid retaining a new full map on every animated frame. Admission only affects
            // storage: first observation, random seeks and pressure always render the same map.
            let repeated = state.observed == Some(key);
            state.observed = Some(key);
            repeated
        };
        let maps = render_maps(directions, casters, meshes, workers);
        let pixels = maps
            .iter()
            .flatten()
            .map(|map| map.depth.capacity())
            .sum::<usize>();
        let bytes = (pixels * size_of::<f32>()
            + maps.capacity() * size_of::<Option<ShadowMap>>()
            + size_of::<ShadowMaps>()) as u64;
        let mut maps = ShadowMaps {
            maps,
            _memory: None,
        };
        if !retain || pixels == 0 {
            return Arc::new(maps);
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.select_budget();
        if let Some((stored, maps)) = &state.entry
            && *stored == key
        {
            return Arc::clone(maps);
        }
        // One set per prepared scene, at most two directional maps. Drop old retention before
        // requesting optional bytes; in-flight readers may legitimately still own its lease.
        state.entry = None;
        if let Ok(lease) = memory::reserve_retained(MemoryKind::Resource, bytes) {
            maps._memory = lease;
            let maps = Arc::new(maps);
            state.entry = Some((key, Arc::clone(&maps)));
            maps
        } else {
            Arc::new(maps)
        }
    }
}

#[cfg(test)]
mod tests;
