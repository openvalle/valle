//! Host CPU allowance shared by frame preparation and nested raster kernels.
//!
//! Pool scopes share kernel workers through work stealing. Callers holding one compute slot
//! release it while joining helpers, so queued chunks can use newly available capacity.
//! A one-slot budget executes nested work inline.
//! Blocking delivery and queue waits must happen outside `CpuBudget::run`.

#[cfg(not(target_family = "wasm"))]
use std::sync::OnceLock;
use std::{
    cell::RefCell,
    marker::PhantomData,
    sync::{Arc, Condvar, Mutex},
};

thread_local! {
    static CURRENT: RefCell<Option<CurrentContext>> = const { RefCell::new(None) };
}

#[derive(Clone, Debug)]
pub struct CpuBudget(Arc<Inner>, bool);

#[derive(Debug)]
struct Inner {
    limit: usize,
    memory: Option<crate::memory::MemoryBudget>,
    state: Mutex<State>,
    released: Condvar,
    #[cfg(not(target_family = "wasm"))]
    pool: OnceLock<rayon::ThreadPool>,
}

#[derive(Debug, Default)]
struct State {
    used: usize,
    peak: usize,
    interactive_waiters: usize,
}

impl CpuBudget {
    /// Reserve asynchronous codec workers for their whole lifetime. Always leave one slot for
    /// upstream work, including when several sessions share this budget.
    pub fn reserve_codec_slots<E>(
        &self,
        slots: usize,
        mut check: impl FnMut() -> Result<(), E>,
    ) -> Result<CodecReservation, E> {
        assert!(
            slots < self.limit(),
            "codec reservation must leave an upstream CPU slot"
        );
        loop {
            // Another delivery can retain codec slots for its entire duration. Cancellation
            // and deadlines must remain observable while waiting, outside the quota lock.
            check()?;
            let mut state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.used <= self.limit() - slots - 1 {
                state.used += slots;
                state.peak = state.peak.max(state.used);
                return Ok(CodecReservation {
                    _permit: Permit {
                        budget: self.clone(),
                        slots,
                    },
                });
            }
            drop(
                self.0
                    .released
                    .wait_timeout(state, std::time::Duration::from_millis(10))
                    .unwrap_or_else(|e| e.into_inner()),
            );
        }
    }
    #[cfg(not(target_family = "wasm"))]
    fn pool(&self) -> &rayon::ThreadPool {
        self.0.pool.get_or_init(|| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(self.limit())
                .thread_name(|index| format!("valle-compute-{index}"))
                .build()
                .expect("cannot create the Native computation pool")
        })
    }

    pub fn new(threads: usize) -> Result<Self, &'static str> {
        Self::new_with_memory(threads, None)
    }

    pub fn new_with_memory(
        threads: usize,
        memory: Option<crate::memory::MemoryBudget>,
    ) -> Result<Self, &'static str> {
        if threads == 0 {
            return Err("CPU thread allowance must be positive");
        }
        Ok(Self(
            Arc::new(Inner {
                limit: threads,
                memory,
                state: Mutex::new(State::default()),
                released: Condvar::new(),
                #[cfg(not(target_family = "wasm"))]
                pool: OnceLock::new(),
            }),
            false,
        ))
    }

    pub fn limit(&self) -> usize {
        self.0.limit
    }

    /// Same pool and total allowance; short preview work wins the next released slot.
    /// Running kernels are never interrupted in the middle of writing a tile.
    pub fn interactive(&self) -> Self {
        Self(self.0.clone(), true)
    }

    pub fn is_current(&self) -> bool {
        CURRENT.with(|current| {
            current
                .borrow()
                .as_ref()
                .is_some_and(|c| Arc::ptr_eq(&c.budget.0, &self.0))
        })
    }

    /// Peak granted compute slots, including nested work; not process thread count.
    pub fn peak_slots(&self) -> usize {
        self.0.state.lock().unwrap_or_else(|e| e.into_inner()).peak
    }

    pub fn run<R>(&self, work: impl FnOnce() -> R) -> R {
        self.run_with_slots(1, work)
    }

    /// Account for a configurable native codec's CPU share as well as its calling stage.
    pub fn run_with_slots<R>(&self, slots: usize, work: impl FnOnce() -> R) -> R {
        assert!((1..=self.limit()).contains(&slots));
        let nested = CURRENT.with(|current| {
            current
                .borrow()
                .as_ref()
                .is_some_and(|b| Arc::ptr_eq(&b.budget.0, &self.0))
        });
        if nested {
            assert_eq!(
                slots, 1,
                "codec shares must be entered outside another CPU stage"
            );
            return work();
        }
        let mut state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        if self.1 {
            state.interactive_waiters += 1;
        }
        while state.used > self.limit() - slots || (!self.1 && state.interactive_waiters > 0) {
            state = self
                .0
                .released
                .wait(state)
                .unwrap_or_else(|e| e.into_inner());
        }
        if self.1 {
            state.interactive_waiters -= 1;
        }
        state.used += slots;
        state.peak = state.peak.max(state.used);
        drop(state);
        let _permit = Permit {
            budget: self.clone(),
            slots,
        };
        let _context = Context::enter(self.clone(), slots == 1);
        work()
    }

    #[cfg(not(target_family = "wasm"))]
    fn borrow_extra(&self, requested: usize) -> Permit {
        let mut state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        let slots = if !self.1 && state.interactive_waiters > 0 {
            0
        } else {
            requested.min(self.limit() - state.used)
        };
        state.used += slots;
        state.peak = state.peak.max(state.used);
        Permit {
            budget: self.clone(),
            slots,
        }
    }

    #[cfg(not(target_family = "wasm"))]
    fn try_slot(&self) -> Option<Permit> {
        let mut state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.used == self.limit() || (!self.1 && state.interactive_waiters > 0) {
            return None;
        }
        state.used += 1;
        state.peak = state.peak.max(state.used);
        Some(Permit {
            budget: self.clone(),
            slots: 1,
        })
    }
}

struct Permit {
    budget: CpuBudget,
    slots: usize,
}

/// Releases the codec's reserved share after its encoder and pending work have been dropped.
pub struct CodecReservation {
    _permit: Permit,
}

impl Drop for Permit {
    fn drop(&mut self) {
        if self.slots > 0 {
            let mut state = self
                .budget
                .0
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            state.used -= self.slots;
            self.budget.0.released.notify_all();
        }
    }
}

#[derive(Clone)]
struct CurrentContext {
    budget: CpuBudget,
    #[cfg(not(target_family = "wasm"))]
    owns_slot: bool,
}

struct Context(Option<CurrentContext>);

impl Context {
    fn enter(budget: CpuBudget, owns_slot: bool) -> Self {
        #[cfg(target_family = "wasm")]
        let _ = owns_slot;
        Self(CURRENT.with(|current| {
            current.replace(Some(CurrentContext {
                budget,
                #[cfg(not(target_family = "wasm"))]
                owns_slot,
            }))
        }))
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        CURRENT.with(|current| current.replace(self.0.take()));
    }
}

/// A host queue or byte-admission wait does no computation. Temporarily return its slot so
/// an earlier frame can finish and release the resource being awaited.
pub fn blocking<R>(work: impl FnOnce() -> R) -> R {
    #[cfg(not(target_family = "wasm"))]
    let _suspended = Suspended::current();
    work()
}

// An admitted parent stops computing when its scope body finishes. Release that slot while
// Rayon joins helpers; otherwise sleeping parents consume the quota needed by those helpers.
#[cfg(not(target_family = "wasm"))]
struct Suspended(CurrentContext);

#[cfg(not(target_family = "wasm"))]
struct SuspendOnScopeExit<'a>(&'a mut Option<Suspended>);

#[cfg(not(target_family = "wasm"))]
impl Drop for SuspendOnScopeExit<'_> {
    fn drop(&mut self) {
        // This also runs if the caller panics. Rayon still joins its helpers before unwinding
        // the outer scope, so leaving the caller's quota held here could deadlock those helpers.
        *self.0 = Suspended::current();
    }
}

#[cfg(not(target_family = "wasm"))]
impl Suspended {
    fn current() -> Option<Self> {
        let context = CURRENT.with(|current| {
            let mut current = current.borrow_mut();
            if current.as_ref().is_some_and(|c| c.owns_slot) {
                current.take()
            } else {
                None
            }
        })?;
        let mut state = context
            .budget
            .0
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        state.used -= 1;
        context.budget.0.released.notify_all();
        drop(state);
        Some(Self(context))
    }
}

#[cfg(not(target_family = "wasm"))]
impl Drop for Suspended {
    fn drop(&mut self) {
        let budget = &self.0.budget;
        let mut state = budget.0.state.lock().unwrap_or_else(|e| e.into_inner());
        if budget.1 {
            state.interactive_waiters += 1;
        }
        while state.used == budget.limit() || (!budget.1 && state.interactive_waiters > 0) {
            if budget.pool().current_thread_index().is_some() {
                drop(state);
                let idle = rayon::yield_now() == Some(rayon::Yield::Idle);
                state = budget.0.state.lock().unwrap_or_else(|e| e.into_inner());
                if !idle
                    || (state.used < budget.limit() && (budget.1 || state.interactive_waiters == 0))
                {
                    continue;
                }
            }
            state = budget
                .0
                .released
                .wait_timeout(state, std::time::Duration::from_micros(250))
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        if budget.1 {
            state.interactive_waiters -= 1;
        }
        state.used += 1;
        state.peak = state.peak.max(state.used);
        drop(state);
        CURRENT.with(|current| current.replace(Some(self.0.clone())));
    }
}

/// The host has admitted this caller into a shared compute allowance.
pub fn is_budgeted() -> bool {
    CURRENT.with(|current| current.borrow().is_some())
}

pub fn memory_budget() -> Option<crate::memory::MemoryBudget> {
    CURRENT.with(|current| {
        current
            .borrow()
            .as_ref()
            .and_then(|context| context.budget.0.memory.clone())
    })
}

#[cfg(not(target_family = "wasm"))]
fn dispatch<'scope>(
    scope: &rayon::Scope<'scope>,
    budget: Option<CpuBudget>,
    borrowed: bool,
    work: impl FnOnce() + Send + 'scope,
) {
    let Some(budget) = budget else {
        work();
        return;
    };
    let nested = CURRENT.with(|current| {
        current
            .borrow()
            .as_ref()
            .is_some_and(|b| Arc::ptr_eq(&b.budget.0, &budget.0))
    });
    if nested {
        work();
        return;
    }
    if borrowed {
        let _context = Context::enter(budget, false);
        work();
        return;
    }
    if let Some(_permit) = budget.try_slot() {
        let _context = Context::enter(budget, true);
        work();
    } else {
        // A stolen child must not block while its parent holds the last slot and waits for it.
        // Return it to the scoped queue so an admitted parent can execute it using its own slot.
        // Briefly park the otherwise idle worker to avoid spinning on codec reservations.
        let state = budget.0.state.lock().unwrap_or_else(|e| e.into_inner());
        drop(
            budget
                .0
                .released
                .wait_timeout(state, std::time::Duration::from_micros(250))
                .unwrap_or_else(|e| e.into_inner()),
        );
        scope.spawn(move |scope| dispatch(scope, Some(budget), false, work));
    }
}

/// At most `parallelism - 1` helpers may be submitted; the caller computes one job.
/// Rayon joins all helpers before returning, including during unwinding.
pub struct CpuScope<'scope, 'env: 'scope, 'borrow> {
    #[cfg(not(target_family = "wasm"))]
    scope: Option<&'borrow rayon::Scope<'scope>>,
    #[cfg(not(target_family = "wasm"))]
    budget: Option<CpuBudget>,
    #[cfg(not(target_family = "wasm"))]
    borrowed_slots: bool,
    #[cfg(not(target_family = "wasm"))]
    helpers_left: std::cell::Cell<usize>,
    _env: PhantomData<&'env &'scope ()>,
    _borrow: PhantomData<&'borrow ()>,
}

impl<'scope, 'env, 'borrow> CpuScope<'scope, 'env, 'borrow> {
    pub fn spawn<F>(&self, work: F)
    where
        F: FnOnce() + Send + 'scope,
    {
        #[cfg(not(target_family = "wasm"))]
        {
            let left = self.helpers_left.get();
            assert!(
                left > 0,
                "nested CPU work exceeded its admitted helper count"
            );
            self.helpers_left.set(left - 1);
            let budget = self.budget.clone();
            let borrowed = self.borrowed_slots;
            self.scope
                .expect("parallel helpers need a scope")
                .spawn(move |scope| dispatch(scope, budget, borrowed, work));
        }
        #[cfg(target_family = "wasm")]
        {
            let _ = work;
            unreachable!("Wasm scopes admit only their caller");
        }
    }
}

pub fn parallel_scope<'env, F, R>(requested: usize, work: F) -> R
where
    F: for<'scope, 'borrow> FnOnce(&CpuScope<'scope, 'env, 'borrow>, usize) -> R,
{
    #[cfg(target_family = "wasm")]
    {
        let _ = requested;
        work(
            &CpuScope {
                _env: PhantomData,
                _borrow: PhantomData,
            },
            1,
        )
    }
    #[cfg(not(target_family = "wasm"))]
    {
        let requested = requested.max(1);
        let current = CURRENT.with(|current| current.borrow().clone());
        let independent_slot = current.as_ref().is_some_and(|c| c.owns_slot);
        let budget = current.map(|c| c.budget);
        // Chunk the work independently of current availability: other frames can release
        // capacity while these chunks are queued. Only grouped codec/borrowed reservations
        // need to acquire helpers up front because they cannot release their parent's slot.
        let permit = if independent_slot {
            None
        } else {
            budget.as_ref().map(|b| b.borrow_extra(requested - 1))
        };
        let count = if independent_slot {
            requested.min(budget.as_ref().unwrap().limit())
        } else {
            permit.as_ref().map_or(requested, |p| p.slots + 1)
        };
        if count == 1 {
            return work(
                &CpuScope {
                    scope: None,
                    budget,
                    borrowed_slots: false,
                    helpers_left: std::cell::Cell::new(0),
                    _env: PhantomData,
                    _borrow: PhantomData,
                },
                1,
            );
        }
        if let Some(budget) = &budget {
            let mut suspended = None;
            let result = budget.pool().in_place_scope(|scope| {
                let _suspend = SuspendOnScopeExit(&mut suspended);
                work(
                    &CpuScope {
                        scope: Some(scope),
                        budget: Some(budget.clone()),
                        borrowed_slots: permit.is_some(),
                        helpers_left: std::cell::Cell::new(count - 1),
                        _env: PhantomData,
                        _borrow: PhantomData,
                    },
                    count,
                )
            });
            drop(suspended);
            result
        } else {
            rayon::in_place_scope(|scope| {
                work(
                    &CpuScope {
                        scope: Some(scope),
                        budget: None,
                        borrowed_slots: false,
                        helpers_left: std::cell::Cell::new(count - 1),
                        _env: PhantomData,
                        _borrow: PhantomData,
                    },
                    count,
                )
            })
        }
    }
}

/// Overlap a sendable helper with caller-owned work, preserving a typed helper panic result.
/// The caller may own thread-affine resources. Waiting uses the pool's scope latch, with no
/// polling loop or scheduling-priority-changing OS yield.
pub fn parallel_pair<A, B: Send>(
    first: impl FnOnce() -> A,
    second: impl FnOnce() -> B + Send,
) -> (A, std::thread::Result<B>) {
    let mut second_result = None;
    let first_result = parallel_scope(2, |scope, count| {
        if count == 1 {
            let first_result = first();
            second_result = Some(std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                second,
            )));
            return first_result;
        }
        let result = &mut second_result;
        scope.spawn(move || {
            *result = Some(std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                second,
            )))
        });
        first()
    });
    (
        first_result,
        second_result.expect("the computation scope completed its helper"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn queued_interaction_precedes_background_after_current_work_releases() {
        let budget = CpuBudget::new(1).unwrap();
        let preview = budget.interactive();
        let (order_tx, order_rx) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            budget.run(|| {
                let tx = order_tx.clone();
                let background = &budget;
                scope.spawn(move || background.run(|| tx.send("export").unwrap()));
                let tx = order_tx.clone();
                scope.spawn(move || preview.run(|| tx.send("preview").unwrap()));
                while budget.0.state.lock().unwrap().interactive_waiters == 0 {
                    std::thread::yield_now();
                }
            });
        });
        assert_eq!(order_rx.recv().unwrap(), "preview");
        assert_eq!(order_rx.recv().unwrap(), "export");
        assert_eq!(budget.peak_slots(), 1);
    }

    #[test]
    fn one_slot_runs_nested_work_inline_and_releases_after_panics() {
        let budget = CpuBudget::new(1).unwrap();
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                || budget.run(|| panic!("test"))
            ))
            .is_err()
        );
        budget.run(|| {
            parallel_scope(8, |_, count| {
                assert_eq!(count, 1);
                budget.run(|| assert!(is_budgeted()));
            })
        });
        assert!(!is_budgeted());
        assert_eq!(budget.peak_slots(), 1);
        assert_eq!(budget.0.state.lock().unwrap().used, 0);
    }

    #[test]
    fn concurrent_and_nested_jobs_share_one_allowance() {
        let budget = CpuBudget::new(3).unwrap();
        let active = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        let job = || {
            let now = active.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(now, Ordering::SeqCst);
            for _ in 0..1000 {
                std::hint::spin_loop();
            }
            active.fetch_sub(1, Ordering::SeqCst);
        };
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    budget.run(|| {
                        parallel_scope(8, |scope, count| {
                            for _ in 1..count {
                                scope.spawn(&job);
                            }
                            job();
                        })
                    })
                });
            }
        });
        assert!(peak.load(Ordering::SeqCst) <= 3);
        assert!(budget.peak_slots() <= 3);
        assert_eq!(budget.0.state.lock().unwrap().used, 0);
    }

    #[test]
    fn codecs_reserve_their_whole_configured_share() {
        let budget = CpuBudget::new(2).unwrap();
        budget.run_with_slots(2, || parallel_scope(8, |_, count| assert_eq!(count, 1)));
        assert_eq!(budget.peak_slots(), 2);
    }
    #[test]
    fn async_codec_reservation_keeps_upstream_live_and_releases_on_drop() {
        let budget = CpuBudget::new(4).unwrap();
        let codec = budget.reserve_codec_slots(2, || Ok::<_, ()>(())).unwrap();
        budget.run(|| {
            parallel_scope(8, |scope, count| {
                assert_eq!(count, 4);
                let nested_budget = budget.clone();
                scope.spawn(move || nested_budget.run(|| assert!(is_budgeted())));
            })
        });
        assert_eq!(budget.0.state.lock().unwrap().used, 2);
        drop(codec);
        assert_eq!(budget.0.state.lock().unwrap().used, 0);
    }

    #[test]
    fn saturated_controllers_finish_nested_jobs_and_codec_reserved_work() {
        for threads in [1, 3, 4] {
            let budget = CpuBudget::new(threads).unwrap();
            let _codec =
                (threads > 1).then(|| budget.reserve_codec_slots(1, || Ok::<_, ()>(())).unwrap());
            let completed = AtomicUsize::new(0);
            std::thread::scope(|scope| {
                for _ in 0..8 {
                    let budget = &budget;
                    let completed = &completed;
                    scope.spawn(move || {
                        budget.run(|| {
                            let values = [1, 2, 3, 4];
                            let sum = AtomicUsize::new(0);
                            parallel_scope(4, |scope, count| {
                                for value in values.iter().take(count) {
                                    let sum = &sum;
                                    if *value == 1 {
                                        sum.fetch_add(*value, Ordering::SeqCst);
                                    } else {
                                        scope.spawn(move || {
                                            let (a, b) = parallel_pair(|| *value, || *value);
                                            sum.fetch_add(a + b.unwrap(), Ordering::SeqCst);
                                        });
                                    }
                                }
                            });
                            assert!(sum.load(Ordering::SeqCst) > 0);
                            completed.fetch_add(1, Ordering::SeqCst);
                        });
                    });
                }
            });
            assert_eq!(completed.load(Ordering::SeqCst), 8);
            assert!(budget.peak_slots() <= threads);
            assert_eq!(
                budget.0.state.lock().unwrap().used,
                usize::from(threads > 1)
            );
        }
    }

    #[test]
    fn unjoined_helper_panics_propagate_and_joined_panics_can_be_handled() {
        let budget = CpuBudget::new(2).unwrap();
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                budget.run(|| {
                    parallel_scope(2, |scope, _| {
                        scope.spawn(|| panic!("unjoined helper"));
                    })
                });
            }))
            .is_err()
        );
        budget.run(|| {
            assert!(parallel_pair(|| (), || panic!("joined helper")).1.is_err());
        });
        assert_eq!(budget.0.state.lock().unwrap().used, 0);
    }

    #[test]
    fn parents_join_helpers_when_every_compute_slot_is_already_occupied() {
        let budget = CpuBudget::new(4).unwrap();
        let _codec = budget.reserve_codec_slots(1, || Ok::<_, ()>(())).unwrap();
        let parents_ready = std::sync::Barrier::new(3);
        std::thread::scope(|scope| {
            for _ in 0..3 {
                scope.spawn(|| {
                    budget.run(|| {
                        // All three admitted parents must hold their slots before any helper runs.
                        parents_ready.wait();
                        let sum = AtomicUsize::new(0);
                        parallel_scope(4, |scope, count| {
                            for _ in 1..count {
                                scope.spawn(|| {
                                    sum.fetch_add(7, Ordering::SeqCst);
                                });
                            }
                        });
                        assert_eq!(sum.load(Ordering::SeqCst), 21);
                    })
                });
            }
        });
        assert_eq!(budget.peak_slots(), 4);
        assert_eq!(budget.0.state.lock().unwrap().used, 1);
    }

    #[test]
    fn cancellation_interrupts_codec_admission_without_waiting_for_another_delivery() {
        let budget = CpuBudget::new(4).unwrap();
        let other_delivery = budget.reserve_codec_slots(2, || Ok::<_, ()>(())).unwrap();
        let mut checks = 0;
        let cancelled = budget.reserve_codec_slots(2, || {
            // This callback can inspect the budget because admission does not hold its lock.
            assert_eq!(budget.peak_slots(), 2);
            checks += 1;
            if checks == 3 {
                Err("cancelled")
            } else {
                Ok(())
            }
        });
        assert_eq!(cancelled.err(), Some("cancelled"));
        assert_eq!(budget.0.state.lock().unwrap().used, 2);
        drop(other_delivery);
        assert_eq!(budget.0.state.lock().unwrap().used, 0);
    }

    #[test]
    fn panicking_saturated_callers_release_quota_before_joining_their_helpers() {
        let budget = CpuBudget::new(2).unwrap();
        let ready = std::sync::Barrier::new(2);
        let finished = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..2 {
                scope.spawn(|| {
                    let error = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        budget.run(|| {
                            ready.wait();
                            parallel_scope(2, |scope, _| {
                                scope.spawn(|| {
                                    finished.fetch_add(1, Ordering::SeqCst);
                                });
                                panic!("caller failed while helpers still need quota");
                            });
                        });
                    }));
                    assert!(error.is_err());
                });
            }
        });
        assert_eq!(finished.load(Ordering::SeqCst), 2);
        assert_eq!(budget.0.state.lock().unwrap().used, 0);
    }
}
