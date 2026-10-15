//! Byte admission for host-owned resources and kernel working sets. A cloned lease describes
//! the same allocation, so shared images are charged once until their last owner releases them.

use std::{
    cell::RefCell,
    sync::{
        Arc, Condvar, Mutex, Weak,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub enum MemoryKind {
    Resource,
    Surface,
    Temporary,
    Media,
    Delivery,
    Plan,
    Admission,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MemoryReport {
    pub used_bytes: u64,
    pub peak_bytes: u64,
    /// Live storage and conservative kernel claims, excluding unspent frame reservations.
    /// Driver/codec internals and immutable compiled inputs are outside this ledger.
    pub peak_live_bytes: u64,
    pub by_kind: [u64; 7],
    pub peak_by_kind: [u64; 7],
}

#[derive(Debug)]
struct Inner {
    limit: Option<u64>,
    state: Mutex<MemoryReport>,
    released: Condvar,
    pressure: AtomicBool,
    credits: AtomicUsize,
}

#[derive(Clone, Debug)]
pub struct MemoryBudget(Arc<Inner>);

#[derive(Clone, Debug)]
pub struct MemoryLease(Arc<Reservation>);

#[derive(Debug)]
struct Reservation {
    budget: MemoryBudget,
    kind: MemoryKind,
    bytes: AtomicU64,
    recycle_limit: AtomicU64,
    return_to: Option<Weak<Reservation>>,
}

#[derive(Clone, Debug)]
pub struct MemoryCredit(MemoryLease);

thread_local! { static CURRENT_CREDIT: RefCell<Option<MemoryCredit>> = const { RefCell::new(None) }; }

struct CreditScope(Option<MemoryCredit>);
impl Drop for CreditScope {
    fn drop(&mut self) {
        CURRENT_CREDIT.with(|current| current.replace(self.0.take()));
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum MemoryError {
    #[error("managed memory request {requested} bytes exceeds budget {limit} bytes")]
    TooLarge { requested: u64, limit: u64 },
    #[error("managed memory is busy: {used} + {requested} bytes exceeds budget {limit} bytes")]
    Busy {
        used: u64,
        requested: u64,
        limit: u64,
    },
    #[error("managed memory byte count overflow")]
    Overflow,
    #[error(
        "frame working-set reservation exhausted: {requested} bytes requested, {remaining} bytes remaining"
    )]
    WorkingSetExceeded { requested: u64, remaining: u64 },
}

impl MemoryBudget {
    pub fn new(limit: Option<u64>) -> Self {
        Self(Arc::new(Inner {
            limit,
            state: Mutex::new(MemoryReport::default()),
            released: Condvar::new(),
            pressure: AtomicBool::new(false),
            credits: AtomicUsize::new(0),
        }))
    }

    pub fn limit(&self) -> Option<u64> {
        self.0.limit
    }
    pub fn active_credits(&self) -> usize {
        self.0.credits.load(Ordering::Acquire)
    }

    pub fn under_pressure(&self) -> bool {
        self.0.pressure.load(Ordering::Acquire)
    }

    pub fn clear_pressure(&self) {
        self.0.pressure.store(false, Ordering::Release);
    }

    pub fn report(&self) -> MemoryReport {
        *self.0.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn reserve(&self, kind: MemoryKind, bytes: u64) -> Result<MemoryLease, MemoryError> {
        let mut state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        let next = state
            .used_bytes
            .checked_add(bytes)
            .ok_or(MemoryError::Overflow)?;
        if let Some(limit) = self.0.limit {
            if bytes > limit {
                return Err(MemoryError::TooLarge {
                    requested: bytes,
                    limit,
                });
            }
            if next > limit {
                return Err(MemoryError::Busy {
                    used: state.used_bytes,
                    requested: bytes,
                    limit,
                });
            }
        }
        state.used_bytes = next;
        state.peak_bytes = state.peak_bytes.max(next);
        state.by_kind[kind as usize] += bytes;
        state.peak_by_kind[kind as usize] =
            state.peak_by_kind[kind as usize].max(state.by_kind[kind as usize]);
        state.peak_live_bytes = state
            .peak_live_bytes
            .max(state.used_bytes - state.by_kind[MemoryKind::Admission as usize]);
        if kind == MemoryKind::Admission {
            self.0.credits.fetch_add(1, Ordering::Release);
        }
        Ok(MemoryLease(Arc::new(Reservation {
            budget: self.clone(),
            kind,
            bytes: AtomicU64::new(bytes),
            recycle_limit: AtomicU64::new(bytes),
            return_to: None,
        })))
    }

    /// Wait only at a host admission boundary, before acquiring CPU slots or starting an
    /// allocation. Kernels use reserve(), which cannot deadlock while holding a partial frame.
    pub fn wait_reserve<E: From<MemoryError>>(
        &self,
        kind: MemoryKind,
        bytes: u64,
        mut check: impl FnMut() -> Result<(), E>,
    ) -> Result<MemoryLease, E> {
        loop {
            check()?;
            match self.reserve(kind, bytes) {
                Ok(lease) => {
                    self.clear_pressure();
                    return Ok(lease);
                }
                Err(MemoryError::Busy { .. }) => {
                    self.0.pressure.store(true, Ordering::Release);
                    let state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
                    drop(
                        self.0
                            .released
                            .wait_timeout(state, std::time::Duration::from_millis(10))
                            .unwrap_or_else(|e| e.into_inner()),
                    );
                }
                Err(error) => return Err(error.into()),
            }
        }
    }

    pub fn wait_credit<E: From<MemoryError>>(
        &self,
        bytes: u64,
        check: impl FnMut() -> Result<(), E>,
    ) -> Result<MemoryCredit, E> {
        self.wait_reserve(MemoryKind::Admission, bytes, check)
            .map(MemoryCredit)
    }
}

impl MemoryLease {
    pub fn bytes(&self) -> u64 {
        self.0.bytes.load(Ordering::Acquire)
    }

    fn split(&self, kind: MemoryKind, bytes: u64) -> Result<MemoryLease, MemoryError> {
        let mut state = self
            .0
            .budget
            .0
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let remaining = self.bytes();
        if bytes > remaining {
            return Err(MemoryError::WorkingSetExceeded {
                requested: bytes,
                remaining,
            });
        }
        self.0.bytes.store(remaining - bytes, Ordering::Release);
        state.by_kind[self.0.kind as usize] -= bytes;
        state.by_kind[kind as usize] += bytes;
        state.peak_by_kind[kind as usize] =
            state.peak_by_kind[kind as usize].max(state.by_kind[kind as usize]);
        state.peak_live_bytes = state
            .peak_live_bytes
            .max(state.used_bytes - state.by_kind[MemoryKind::Admission as usize]);
        Ok(MemoryLease(Arc::new(Reservation {
            budget: self.0.budget.clone(),
            kind,
            bytes: AtomicU64::new(bytes),
            recycle_limit: AtomicU64::new(bytes),
            return_to: Some(Arc::downgrade(&self.0)),
        })))
    }
}

impl MemoryCredit {
    /// Retire the preceding phase's unused allowance without releasing images still owned by
    /// that frame. Later image drops cannot grow the reservation back to its old phase's size.
    pub fn limit_remaining(&self, maximum: u64) {
        let reservation = &self.0.0;
        let mut state = reservation
            .budget
            .0
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let maximum = maximum.min(reservation.recycle_limit.load(Ordering::Acquire));
        reservation.recycle_limit.store(maximum, Ordering::Release);
        let previous = reservation.bytes.load(Ordering::Acquire);
        let released = previous.saturating_sub(maximum);
        reservation
            .bytes
            .store(previous - released, Ordering::Release);
        state.used_bytes -= released;
        state.by_kind[reservation.kind as usize] -= released;
        reservation.budget.0.released.notify_all();
    }
    pub fn run<R>(&self, work: impl FnOnce() -> R) -> R {
        let _scope =
            CreditScope(CURRENT_CREDIT.with(|current| current.replace(Some(self.clone()))));
        work()
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        // Recycle transient storage into the active frame's credit. A cached image can outlive
        // that frame; its weak link then expires and releasing it returns bytes to the host.
        let credit = self.return_to.as_ref().and_then(Weak::upgrade);
        let mut state = self
            .budget
            .0
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let bytes = self.bytes.load(Ordering::Acquire);
        if self.kind == MemoryKind::Admission && self.return_to.is_none() {
            self.budget.0.credits.fetch_sub(1, Ordering::Release);
        }
        state.by_kind[self.kind as usize] -= bytes;
        if let Some(credit) = &credit {
            let returned = bytes.min(
                credit
                    .recycle_limit
                    .load(Ordering::Acquire)
                    .saturating_sub(credit.bytes.load(Ordering::Acquire)),
            );
            credit.bytes.fetch_add(returned, Ordering::Release);
            state.by_kind[credit.kind as usize] += returned;
            state.used_bytes -= bytes - returned;
        } else {
            state.used_bytes -= bytes;
        }
        self.budget.0.released.notify_all();
        drop(state);
    }
}

pub fn reserve(kind: MemoryKind, bytes: u64) -> Result<Option<MemoryLease>, MemoryError> {
    if let Some(credit) = CURRENT_CREDIT.with(|current| current.borrow().clone()) {
        return credit.0.split(kind, bytes).map(Some);
    }
    reserve_retained(kind, bytes)
}

/// Charge optional retained storage (a cache entry) to the shared budget. A running frame's
/// credit covers only that frame's planned working set, so a cache insert never spends it.
pub fn reserve_retained(kind: MemoryKind, bytes: u64) -> Result<Option<MemoryLease>, MemoryError> {
    crate::cpu::memory_budget()
        .map(|budget| budget.reserve(kind, bytes))
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_allocation_is_charged_until_its_last_owner_drops() {
        let budget = MemoryBudget::new(Some(16));
        let first = budget.reserve(MemoryKind::Resource, 12).unwrap();
        let second = first.clone();
        drop(first);
        assert!(matches!(
            budget.reserve(MemoryKind::Surface, 8),
            Err(MemoryError::Busy { .. })
        ));
        drop(second);
        let image = budget.reserve(MemoryKind::Surface, 16).unwrap();
        assert_eq!(budget.report().by_kind, [0, 16, 0, 0, 0, 0, 0]);
        drop(image);
        assert_eq!(budget.report().used_bytes, 0);
        assert_eq!(budget.report().peak_bytes, 16);
        assert!(matches!(
            budget.reserve(MemoryKind::Temporary, 17),
            Err(MemoryError::TooLarge { .. })
        ));
    }

    #[test]
    fn concurrent_admission_never_exceeds_bytes_and_unwind_releases() {
        let budget = MemoryBudget::new(Some(64));
        std::thread::scope(|scope| {
            for _ in 0..8 {
                let budget = &budget;
                scope.spawn(move || {
                    for _ in 0..30 {
                        let _lease = budget
                            .wait_reserve(MemoryKind::Temporary, 16, || Ok::<_, MemoryError>(()))
                            .unwrap();
                        std::thread::yield_now();
                    }
                });
            }
        });
        let _ = std::panic::catch_unwind(|| {
            let _lease = budget.reserve(MemoryKind::Plan, 8).unwrap();
            panic!("test");
        });
        assert_eq!(budget.report().used_bytes, 0);
        assert!(budget.report().peak_bytes <= 64);
    }

    #[test]
    fn frame_credit_transfers_bytes_to_surviving_images_without_double_charging() {
        let budget = MemoryBudget::new(Some(64));
        let credit = budget.wait_credit(64, || Ok::<_, MemoryError>(())).unwrap();
        let image = credit.run(|| reserve(MemoryKind::Surface, 32).unwrap().unwrap());
        assert_eq!(budget.report().used_bytes, 64);
        assert_eq!(budget.report().peak_live_bytes, 32);
        drop(credit);
        assert_eq!(budget.report().used_bytes, 32);
        let next = budget.wait_credit(32, || Ok::<_, MemoryError>(())).unwrap();
        drop(image);
        drop(next);
        assert_eq!(budget.report().used_bytes, 0);
    }

    #[test]
    fn retained_storage_bypasses_the_running_frame_credit() {
        let budget = MemoryBudget::new(Some(64));
        let cpu = crate::cpu::CpuBudget::new_with_memory(1, Some(budget.clone())).unwrap();
        let credit = budget.wait_credit(48, || Ok::<_, MemoryError>(())).unwrap();
        let cached = cpu.run(|| credit.run(|| reserve_retained(MemoryKind::Surface, 16)));
        let cached = cached.unwrap().unwrap();
        assert_eq!(budget.report().used_bytes, 64);
        // The frame still owns its whole credit; the cache could not take a byte of it.
        let scratch = credit.run(|| reserve(MemoryKind::Temporary, 48)).unwrap();
        assert!(
            cpu.run(|| reserve_retained(MemoryKind::Surface, 1))
                .is_err()
        );
        drop((scratch, cached, credit));
        assert_eq!(budget.report().used_bytes, 0);
    }

    #[test]
    fn transient_credit_is_recycled_between_passes() {
        let budget = MemoryBudget::new(Some(64));
        let credit = budget.wait_credit(64, || Ok::<_, MemoryError>(())).unwrap();
        for _ in 0..100 {
            let scratch = credit.run(|| reserve(MemoryKind::Temporary, 64).unwrap());
            drop(scratch);
            assert_eq!(budget.report().by_kind[MemoryKind::Admission as usize], 64);
        }
        drop(credit);
        assert_eq!(budget.report().used_bytes, 0);
    }

    #[test]
    fn retiring_preparation_returns_capacity_without_releasing_live_images() {
        let budget = MemoryBudget::new(Some(100));
        let credit = budget
            .wait_credit(100, || Ok::<_, MemoryError>(()))
            .unwrap();
        let image = credit.run(|| reserve(MemoryKind::Resource, 30).unwrap());
        credit.limit_remaining(20);
        assert_eq!(budget.report().used_bytes, 50);
        let another = budget.wait_credit(50, || Ok::<_, MemoryError>(())).unwrap();
        drop(image);
        assert_eq!(budget.report().used_bytes, 70); // The retired phase cannot grow back.
        assert_eq!(budget.active_credits(), 2);
        drop(credit);
        drop(another);
        assert_eq!(budget.report().used_bytes, 0);
        assert_eq!(budget.active_credits(), 0);
    }

    #[test]
    fn waiting_admission_observes_cancellation_without_releasing_existing_owner() {
        let budget = MemoryBudget::new(Some(1));
        let _lease = budget.reserve(MemoryKind::Delivery, 1).unwrap();
        let mut checks = 0;
        let result = budget.wait_reserve(MemoryKind::Temporary, 1, || {
            checks += 1;
            if checks == 2 {
                Err(MemoryError::Overflow)
            } else {
                Ok(())
            }
        });
        assert_eq!(result.unwrap_err(), MemoryError::Overflow);
        assert_eq!(budget.report().used_bytes, 1);
    }
}
