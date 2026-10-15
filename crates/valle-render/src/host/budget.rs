//! Host CPU allowance, a shared live-byte ledger, and bounded optional cache retention.
//! Driver/codec implementation allocations remain outside the host's managed byte contract.

use thiserror::Error;
use valle_engine::{
    frame::RenderSpec,
    motion::{
        cpu::CpuBudget,
        memory::{MemoryBudget, MemoryReport},
    },
};

use crate::executor::skia::SkiaCacheLimits;

#[derive(Debug, Clone)]
pub struct NativeRenderBudget {
    pub(crate) cpu: CpuBudget,
    memory_bytes: Option<u64>,
    pub(crate) memory: MemoryBudget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeBudgetReport {
    pub cpu_threads: usize,
    /// Runtime-lifetime peak granted slots, including nested kernels; not OS thread count.
    pub peak_cpu_slots: usize,
    pub memory_budget_bytes: Option<u64>,
    pub memory: MemoryReport,
    pub resource_providers: usize,
    pub executors: usize,
    /// Reserved capacity for the covered resources, not measured allocation or footprint.
    pub configured_capacity_bytes: Option<u64>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct OwnerMemoryLimits {
    pub(crate) executor_capacity: u64,
    pub(crate) resource_cache: u64,
    pub(crate) caches: SkiaCacheLimits,
}

/// Preparation owns resource caches; only physical executors own drawing surfaces and caches.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ResidencyOwners {
    pub(crate) providers: usize,
    pub(crate) executors: usize,
    pub(crate) gpu: bool,
}

impl NativeRenderBudget {
    pub fn new(threads: usize, memory_bytes: Option<u64>) -> Result<Self, NativeBudgetError> {
        if memory_bytes == Some(0) {
            return Err(NativeBudgetError::ZeroMemory);
        }
        let memory = MemoryBudget::new(memory_bytes);
        Ok(Self {
            cpu: CpuBudget::new_with_memory(threads, Some(memory.clone()))
                .map_err(|_| NativeBudgetError::ZeroThreads)?,
            memory_bytes,
            memory,
        })
    }

    pub fn threads(&self) -> usize {
        self.cpu.limit()
    }
    pub fn memory_bytes(&self) -> Option<u64> {
        self.memory_bytes
    }
    /// Host-owned claims and conservative temporary reservations, excluding process/driver RSS.
    pub fn memory_report(&self) -> MemoryReport {
        self.memory.report()
    }

    /// A preview and export clone share one host allowance; pending interaction gets the next
    /// compute slot instead of waiting behind newly started export chunks.
    pub fn for_preview(&self) -> Self {
        let mut budget = self.clone();
        budget.cpu = budget.cpu.interactive();
        budget
    }

    pub(crate) fn frame_parallelism(
        &self,
        requested: usize,
        spec: RenderSpec,
    ) -> Result<usize, NativeBudgetError> {
        let requested = requested.min(self.threads()).max(1);
        let Some(bytes) = self.memory_bytes else {
            return Ok(requested);
        };
        // Minimum admitted storage: two working F16 surfaces, persistent RGBA delivery, the
        // current completed frame and its next handoff. Actual plan preflight may require more.
        let minimum = rgba_bytes(spec)?
            .checked_mul(8)
            .ok_or(NativeBudgetError::Overflow)?;
        if bytes < minimum {
            return Err(NativeBudgetError::FrameDoesNotFit {
                required: minimum,
                available: bytes,
            });
        }
        Ok(requested
            .min(usize::try_from(bytes / minimum).unwrap_or(usize::MAX))
            .max(1))
    }

    pub(crate) fn owner_limits(
        &self,
        owners: ResidencyOwners,
        spec: RenderSpec,
    ) -> Result<Option<OwnerMemoryLimits>, NativeBudgetError> {
        let Some(bytes) = self.memory_bytes else {
            return Ok(None);
        };
        let providers = owners.providers.max(1) as u64;
        let executors = owners.executors.max(1) as u64;
        let resource_cache = (bytes / 16 / providers).min(512 * 1024 * 1024);
        let capacity = (bytes - resource_cache * providers) / executors;
        let delivery = rgba_bytes(spec)?
            .checked_mul(3)
            .ok_or(NativeBudgetError::Overflow)?;
        let programs = (capacity / 32).min(32 * 1024 * 1024);
        let caches = SkiaCacheLimits {
            // GrResourceCache charges live budgeted surfaces as well as purgeable resources.
            // Its target overlaps the surface/layer capacities below; setting it to only the
            // spare retention share would continuously evict reuse while a frame is alive.
            gpu_resources: if owners.gpu {
                usize::try_from(capacity.saturating_sub(delivery).saturating_sub(programs))
                    .unwrap_or(usize::MAX)
            } else {
                0
            },
            programs: usize::try_from(programs).unwrap(),
            raster_layers: usize::try_from((capacity / 16).min(64 * 1024 * 1024)).unwrap(),
            opacity_layers: usize::try_from((capacity / 32).min(32 * 1024 * 1024)).unwrap(),
            surface_pool: (capacity / 16).min(128 * 1024 * 1024),
        };
        // Cache retention has owner-local ceilings; frames borrow the whole allowance through
        // admission, which evicts optional retention under pressure.
        Ok(Some(OwnerMemoryLimits {
            executor_capacity: capacity,
            resource_cache,
            caches,
        }))
    }

    pub(crate) fn report(
        &self,
        owners: ResidencyOwners,
        spec: RenderSpec,
    ) -> Result<NativeBudgetReport, NativeBudgetError> {
        let capacity = self.owner_limits(owners, spec)?.map(|limits| {
            limits.resource_cache * owners.providers.max(1) as u64
                + limits.executor_capacity * owners.executors.max(1) as u64
        });
        Ok(NativeBudgetReport {
            cpu_threads: self.threads(),
            peak_cpu_slots: self.cpu.peak_slots(),
            memory_budget_bytes: self.memory_bytes,
            memory: self.memory.report(),
            resource_providers: owners.providers,
            executors: owners.executors,
            configured_capacity_bytes: capacity,
        })
    }
}

impl Default for NativeRenderBudget {
    fn default() -> Self {
        Self::new(
            std::thread::available_parallelism().map_or(1, usize::from),
            None,
        )
        .unwrap()
    }
}

fn rgba_bytes(spec: RenderSpec) -> Result<u64, NativeBudgetError> {
    u64::from(spec.width())
        .checked_mul(u64::from(spec.height()))
        .and_then(|p| p.checked_mul(4))
        .ok_or(NativeBudgetError::Overflow)
}

#[derive(Debug, Error)]
pub enum NativeBudgetError {
    #[error("a synchronous nested render cannot share the CPU/memory allowance held by its caller")]
    ReentrantSession,
    #[error("CPU thread allowance must be positive")]
    ZeroThreads,
    #[error("memory budget must be positive")]
    ZeroMemory,
    #[error("memory budget arithmetic overflow")]
    Overflow,
    #[error(
        "minimum covered frame residency needs {required} bytes, but only {available} bytes are available"
    )]
    FrameDoesNotFit { required: u64, available: u64 },
    #[error(
        "software encoder requests {requested} CPU slots; at most {available} are available while leaving rendering able to progress"
    )]
    EncoderShare { requested: usize, available: usize },
}

#[cfg(test)]
mod tests {
    use super::*;
    use valle_engine::{
        frame::RenderQuality,
        resource::{OutputBackground, OutputSpec},
    };
    fn spec() -> RenderSpec {
        RenderSpec::new(
            1920,
            1080,
            RenderQuality::Final,
            OutputSpec::srgb_preview(OutputBackground::Transparent).unwrap(),
        )
        .unwrap()
    }
    #[test]
    fn owner_capacity_and_cache_shares_never_exceed_the_host_allowance() {
        for bytes in [512, 1024, 2048, 4096].map(|mib| mib * 1024 * 1024) {
            let budget = NativeRenderBudget::new(8, Some(bytes)).unwrap();
            for providers in 1..=8 {
                for (executors, gpu) in [(providers, false), (1, true), (3, true)] {
                    let owners = ResidencyOwners {
                        providers,
                        executors,
                        gpu,
                    };
                    if let Ok(Some(limits)) = budget.owner_limits(owners, spec()) {
                        let retention = limits.caches.raster_layers as u64
                            + limits.caches.opacity_layers as u64
                            + limits.caches.surface_pool
                            + limits.caches.programs as u64;
                        assert!(retention <= limits.executor_capacity);
                        assert!(
                            limits.executor_capacity * executors as u64
                                + limits.resource_cache * providers as u64
                                <= bytes
                        );
                        if gpu {
                            assert!(limits.caches.gpu_resources as u64 <= limits.executor_capacity);
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn preparer_count_does_not_reserve_nonexistent_drawing_executors() {
        let budget = NativeRenderBudget::new(8, Some(2 * 1024 * 1024 * 1024)).unwrap();
        let limits = |providers| {
            budget
                .owner_limits(
                    ResidencyOwners {
                        providers,
                        executors: 1,
                        gpu: true,
                    },
                    spec(),
                )
                .unwrap()
                .unwrap()
        };
        let one = limits(1);
        let eight = limits(8);
        assert_eq!(one.executor_capacity, eight.executor_capacity);
        assert_eq!(one.resource_cache, eight.resource_cache * 8);
    }

    #[test]
    fn explicit_limits_reject_impossible_frames_and_bound_concurrency() {
        assert!(NativeRenderBudget::new(0, None).is_err());
        assert!(NativeRenderBudget::new(1, Some(0)).is_err());
        assert!(
            NativeRenderBudget::new(1, Some(16))
                .unwrap()
                .frame_parallelism(1, spec())
                .is_err()
        );
        assert_eq!(
            NativeRenderBudget::new(1, None)
                .unwrap()
                .frame_parallelism(8, spec())
                .unwrap(),
            1
        );
        assert_eq!(
            NativeRenderBudget::new(8, Some(128 * 1024 * 1024))
                .unwrap()
                .frame_parallelism(8, spec())
                .unwrap(),
            2
        );
    }
    #[test]
    fn cloned_sessions_share_actual_byte_admission() {
        use valle_engine::motion::memory::MemoryKind;
        let budget = NativeRenderBudget::new(4, Some(1024)).unwrap();
        let clone = budget.clone();
        let lease = budget.memory.reserve(MemoryKind::Temporary, 768).unwrap();
        assert!(clone.memory.reserve(MemoryKind::Surface, 512).is_err());
        drop(lease);
        let lease = clone.memory.reserve(MemoryKind::Surface, 512).unwrap();
        assert_eq!(budget.memory.report().used_bytes, 512);
        drop(lease);
        assert_eq!(budget.memory.report().used_bytes, 0);
    }
}
