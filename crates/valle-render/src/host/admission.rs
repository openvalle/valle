//! Ordered frame admission. Later completed preparation must never hold all the bytes needed
//! by the next frame the sink is waiting for. Waiting here holds neither a CPU slot nor images.
use super::FrameRenderError;
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};
use valle_engine::motion::{
    cpu,
    memory::{MemoryBudget, MemoryCredit, MemoryError},
};

pub(crate) struct FrameAdmission {
    memory: MemoryBudget,
    next: Mutex<usize>,
    changed: Condvar,
    failed: AtomicBool,
    cancelled: Arc<AtomicBool>,
    deadline: Option<Instant>,
    delivery_bytes: Vec<u64>,
}

impl FrameAdmission {
    pub(crate) fn new(
        memory: MemoryBudget,
        cancelled: Arc<AtomicBool>,
        deadline: Option<Instant>,
    ) -> Self {
        Self {
            memory,
            next: Mutex::new(0),
            changed: Condvar::new(),
            failed: AtomicBool::new(false),
            cancelled,
            deadline,
            delivery_bytes: Vec::new(),
        }
    }

    pub(crate) fn with_delivery_bytes(mut self, bytes: Vec<u64>) -> Self {
        if bytes.iter().any(|&bytes| bytes != 0) {
            self.delivery_bytes = bytes;
        }
        self
    }

    pub(crate) fn delivery_bytes(&self, sequence: usize) -> u64 {
        self.delivery_bytes.get(sequence).copied().unwrap_or(0)
    }

    pub(crate) fn stop(&self) {
        self.failed.store(true, Ordering::Release);
        self.changed.notify_all();
    }

    fn check(&self) -> Result<(), FrameRenderError> {
        if self.failed.load(Ordering::Acquire)
            || self.cancelled.load(Ordering::Acquire)
            || self
                .deadline
                .is_some_and(|deadline| Instant::now() >= deadline)
        {
            return Err(FrameRenderError::AdmissionStopped);
        }
        Ok(())
    }

    pub(crate) fn admit(
        &self,
        sequence: usize,
        bytes: u64,
        mut trim: impl FnMut(),
    ) -> Result<MemoryCredit, FrameRenderError> {
        cpu::blocking(|| {
            let mut next = self.next.lock().unwrap_or_else(|e| e.into_inner());
            while *next != sequence {
                self.check()?;
                next = self
                    .changed
                    .wait_timeout(next, Duration::from_millis(10))
                    .unwrap_or_else(|e| e.into_inner())
                    .0;
            }
            let mut idle: Option<(Instant, u64)> = None;
            let credit = self.memory.wait_credit(bytes, || {
                self.check()?;
                if self.memory.under_pressure() {
                    trim();
                    let used = self.memory.report().used_bytes;
                    if self.memory.active_credits() == 0
                        && self
                            .memory
                            .limit()
                            .is_some_and(|limit| bytes > limit.saturating_sub(used))
                    {
                        // Idle executors poll pressure every 2-10 ms. Give their evictions time
                        // to finish, then reject a stable non-evictable remainder instead of
                        // waiting forever for a frame that cannot start (e.g. retained PCM).
                        match idle {
                            Some((since, previous))
                                if previous == used
                                    && since.elapsed() >= Duration::from_secs(1) =>
                            {
                                return Err(MemoryError::Busy {
                                    used,
                                    requested: bytes,
                                    limit: self.memory.limit().unwrap(),
                                }
                                .into());
                            }
                            Some((_, previous)) if previous == used => {}
                            _ => idle = Some((Instant::now(), used)),
                        }
                    } else {
                        idle = None;
                    }
                }
                Ok::<_, FrameRenderError>(())
            })?;
            *next += 1;
            self.changed.notify_all();
            Ok(credit)
        })
    }
}

pub(crate) fn checked_add(a: u64, b: u64) -> Result<u64, FrameRenderError> {
    a.checked_add(b).ok_or_else(|| MemoryError::Overflow.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use valle_engine::motion::memory::MemoryKind;

    #[test]
    fn idle_non_evictable_storage_fails_instead_of_waiting_forever() {
        let memory = MemoryBudget::new(Some(100));
        let retained = memory.reserve(MemoryKind::Media, 60).unwrap();
        let admission = FrameAdmission::new(memory.clone(), Arc::new(AtomicBool::new(false)), None);
        let error = admission.admit(0, 60, || {}).unwrap_err();
        assert!(matches!(
            error,
            FrameRenderError::Memory(MemoryError::Busy {
                used: 60,
                requested: 60,
                limit: 100
            })
        ));
        drop(retained);
        assert_eq!(memory.report().used_bytes, 0);
    }

    #[test]
    fn memory_pressure_evicts_completed_storage_before_admitting() {
        let memory = MemoryBudget::new(Some(100));
        let mut retained = Some(memory.reserve(MemoryKind::Resource, 60).unwrap());
        let admission = FrameAdmission::new(memory.clone(), Arc::new(AtomicBool::new(false)), None);
        let credit = admission
            .admit(0, 60, || {
                retained.take();
            })
            .unwrap();
        assert_eq!(memory.report().used_bytes, 60);
        assert!(retained.is_none());
        drop(credit);
        assert_eq!(memory.report().used_bytes, 0);
    }
}
