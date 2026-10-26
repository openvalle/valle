//! One resource owner per render session, independent of preparation task concurrency.

use std::sync::{Arc, Mutex, MutexGuard};

use valle_engine::resource::ResourceRequest;

use super::{NativeResourceError, NativeResourceProvider, ResourceFrameReport, ResourceProvider};
use crate::executor::skia::{SkiaBackendKind, SkiaExternalObject};

#[derive(Clone)]
pub(crate) struct SharedResourceProvider {
    state: Arc<Mutex<NativeResourceProvider>>,
    backend: SkiaBackendKind,
    report: ResourceFrameReport,
}

impl SharedResourceProvider {
    fn lock(&self) -> MutexGuard<'_, NativeResourceProvider> {
        // Every entry point must return its compute slot while waiting for this owner. A
        // fulfilled request may hold this lock while reacquiring its slot after a blocking wait.
        valle_engine::motion::cpu::blocking(|| self.state.lock().unwrap_or_else(|e| e.into_inner()))
    }

    pub(crate) fn new(
        mut provider: NativeResourceProvider,
        backend: SkiaBackendKind,
    ) -> Result<Self, NativeResourceError> {
        provider.configure_backend(backend)?;
        let report = provider.frame_report();
        Ok(Self {
            state: Arc::new(Mutex::new(provider)),
            backend,
            report,
        })
    }
}

impl ResourceProvider for SharedResourceProvider {
    type Error = NativeResourceError;

    fn configure_backend(&mut self, backend: SkiaBackendKind) -> Result<(), Self::Error> {
        if backend != self.backend {
            return Err(NativeResourceError::BackendConfiguredAfterDecode);
        }
        Ok(())
    }

    fn trim_memory(&mut self) {
        self.lock().trim_memory();
    }

    fn begin_frame(&mut self) {
        let mut state = self.lock();
        state.begin_frame();
        let report = state.frame_report();
        drop(state);
        self.report = report;
    }

    fn fulfill(&mut self, request: &ResourceRequest) -> Result<SkiaExternalObject, Self::Error> {
        let mut state = self.lock();
        state.begin_frame();
        let object;
        let report;
        let video = matches!(
            request.sample(),
            valle_engine::resource::ResourceSample::SourceTime(_)
        ) && matches!(
            request.expected(),
            valle_engine::resource::ExternalResourceDesc::VisualFrame { .. }
        );
        if video
            || matches!(
                request.expected(),
                valle_engine::resource::ExternalResourceDesc::Scene3d
            )
        {
            if let Some(cached) = state.lookup_request(request)? {
                object = cached;
                report = state.frame_report();
            } else {
                let image = if video {
                    let work = state.prepare_video(request)?;
                    let before = state.frame_report();
                    drop(state);
                    self.accumulate(before)?;
                    work.render(request)?
                } else {
                    let work = state.prepare_scene3d(request)?;
                    let before = state.frame_report();
                    drop(state);
                    self.accumulate(before)?;
                    work.render(&mut self.report.scene3d)?
                };
                state = self.lock();
                state.begin_frame();
                object = state.cache_fulfilled(request, image)?;
                report = state.frame_report();
            }
        } else {
            object = state.fulfill(request)?;
            report = state.frame_report();
        }
        drop(state);
        self.accumulate(report)?;
        Ok(object)
    }

    fn source_resident_bytes(&self, request: &ResourceRequest) -> u64 {
        self.lock().source_resident_bytes(request)
    }

    fn finish_frame(&mut self, requests: &[ResourceRequest]) -> ResourceFrameReport {
        self.lock().retain_media(requests);
        self.report
    }

    fn invalidate_generation(&mut self) -> Result<(), Self::Error> {
        self.lock().invalidate_generation()
    }
}

impl SharedResourceProvider {
    fn accumulate(&mut self, report: ResourceFrameReport) -> Result<(), NativeResourceError> {
        macro_rules! add { ($target:expr, $source:expr, $($field:ident),+) => { $(
            $target.$field = $target.$field.checked_add($source.$field)
                .ok_or(NativeResourceError::CacheCounterOverflow)?;
        )+ }; }
        add!(
            self.report.cache,
            report.cache,
            generation_invalidations,
            requests,
            hits,
            misses,
            insertions,
            evictions,
            bypasses
        );
        add!(
            self.report.scene3d,
            report.scene3d,
            requests,
            prepared_cache_hits,
            prepared_cache_misses,
            prepare_us,
            raster_us,
            upload_us,
            composite_us
        );
        self.report.cache.generation = report.cache.generation;
        self.report.cache.resident_entries = report.cache.resident_entries;
        self.report.cache.resident_bytes = report.cache.resident_bytes;
        Ok(())
    }
}
