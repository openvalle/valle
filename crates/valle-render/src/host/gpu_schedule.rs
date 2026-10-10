//! Conservative, feedback-driven concurrency for the shared Metal delivery path.
//! This changes scheduling only; shader selection and pixel semantics stay in the executor.

const WINDOW_FRAMES: usize = 24;
const WINDOW_MICROS: u64 = 100_000;
// A short GPU burst does not repay creation and warming of independent contexts.
const MINIMUM_GPU_WAIT_MICROS: u64 = 200_000;
const WARMUP_FRAMES: usize = 8;
const RECHECK_FRAMES: usize = 12;
const COOLDOWN_FRAMES: usize = 96;
const MINIMUM_EXTRA_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Clone, Copy, Debug)]
pub(super) struct MetalMemory {
    pub allocated: u64,
    pub budget: u64,
}

impl MetalMemory {
    fn permits_growth(self, queues: usize, target: usize) -> bool {
        let extra = (self.allocated / queues as u64)
            .max(MINIMUM_EXTRA_BYTES)
            .saturating_mul(target.saturating_sub(queues) as u64);
        self.allocated.saturating_add(extra) <= self.budget
    }
}

pub(super) fn metal_memory() -> Option<MetalMemory> {
    use objc2_metal::MTLDevice;
    let device = objc2_metal::MTLCreateSystemDefaultDevice()?;
    // Device allocations include all this process's Metal contexts, including Scene3D.
    // Leave half the recommended working set available to CPU/encoder/system work.
    // This gates optional concurrency; it is not a hard limit on one frame's resources.
    let budget = device.recommendedMaxWorkingSetSize() / 2;
    (budget > 0).then(|| MetalMemory {
        allocated: device.currentAllocatedSize() as u64,
        budget,
    })
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct QueueSample {
    pub wall_us: u64,
    pub gpu_wait_us: u64,
    pub prepare_wait_us: u64,
    pub sink_us: u64,
    pub submit_us: u64,
    pub execute_us: u64,
}

#[derive(Clone, Copy, Debug, Default)]
struct Window {
    frames: usize,
    sample: QueueSample,
}

impl Window {
    fn add(&mut self, sample: QueueSample) {
        self.frames += 1;
        self.sample.wall_us = self.sample.wall_us.saturating_add(sample.wall_us);
        self.sample.gpu_wait_us = self.sample.gpu_wait_us.saturating_add(sample.gpu_wait_us);
        self.sample.prepare_wait_us = self
            .sample
            .prepare_wait_us
            .saturating_add(sample.prepare_wait_us);
        self.sample.sink_us = self.sample.sink_us.saturating_add(sample.sink_us);
        self.sample.submit_us = self.sample.submit_us.saturating_add(sample.submit_us);
        self.sample.execute_us = self.sample.execute_us.saturating_add(sample.execute_us);
    }

    fn gpu_bound(self) -> bool {
        let s = self.sample;
        s.gpu_wait_us >= s.wall_us / 3
            && s.gpu_wait_us >= MINIMUM_GPU_WAIT_MICROS
            && s.prepare_wait_us <= s.wall_us / 5
            && s.sink_us <= s.wall_us / 5
    }

    fn improves(self, previous: Self) -> bool {
        // Retained node counts include invisible work and cannot normalize GPU
        // cost. Compare measured throughput only for comparable recording work.
        u128::from(self.sample.wall_us) * previous.frames as u128 * 10
            <= u128::from(previous.sample.wall_us) * self.frames as u128 * 9
    }

    fn comparable(self, previous: Self) -> bool {
        // A retained tree can keep the same node count while invisible layers
        // fade in. Recording cost is a drift guard, not a GPU cost multiplier:
        // if it changes materially, collect a fresh reference instead.
        let current = u128::from(self.sample.execute_us) * previous.frames as u128;
        let prior = u128::from(previous.sample.execute_us) * self.frames as u128;
        current * 4 >= prior * 3 && current * 4 <= prior * 5
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct QueueChange {
    pub queues: usize,
    /// A short reference check keeps the candidate contexts warm but inactive.
    pub retained_queues: usize,
    pub reason: &'static str,
}

enum Probe {
    Expanded {
        previous: usize,
        reference: Window,
    },
    Recheck {
        previous: usize,
        candidate: usize,
        parallel: Window,
    },
}

pub(super) struct AdaptiveQueues {
    queues: usize,
    retained_queues: usize,
    maximum: usize,
    warmup: usize,
    cooldown: usize,
    window: Window,
    probe: Option<Probe>,
    idle_windows: usize,
}

impl AdaptiveQueues {
    pub fn new(maximum: usize) -> Self {
        Self {
            queues: 1,
            retained_queues: 1,
            maximum: maximum.max(1),
            warmup: WARMUP_FRAMES,
            cooldown: 0,
            window: Window::default(),
            probe: None,
            idle_windows: 0,
        }
    }

    pub fn restrict_to(&mut self, queues: usize) {
        self.maximum = queues;
        self.queues = queues;
        self.retained_queues = queues;
        self.probe = None;
        self.window = Window::default();
        self.warmup = WARMUP_FRAMES;
    }

    fn change(
        &mut self,
        queues: usize,
        retained_queues: usize,
        reason: &'static str,
    ) -> Option<QueueChange> {
        self.queues = queues;
        self.retained_queues = retained_queues;
        self.warmup = if matches!(self.probe, Some(Probe::Recheck { .. })) {
            WARMUP_FRAMES / 2
        } else {
            WARMUP_FRAMES
        };
        self.idle_windows = 0;
        Some(QueueChange {
            queues,
            retained_queues,
            reason,
        })
    }

    pub fn observe(
        &mut self,
        sample: QueueSample,
        remaining: usize,
        memory: impl FnOnce() -> Option<MetalMemory>,
    ) -> Option<QueueChange> {
        if self.maximum == 1 {
            return None;
        }
        if self.cooldown > 0 {
            self.cooldown -= 1;
        }
        if self.warmup > 0 {
            self.warmup -= 1;
            return None;
        }
        self.window.add(sample);
        let rechecking = matches!(self.probe, Some(Probe::Recheck { .. }));
        if self.window.frames
            < if rechecking {
                RECHECK_FRAMES
            } else {
                WINDOW_FRAMES
            }
            || (!rechecking && self.window.sample.wall_us < WINDOW_MICROS)
        {
            return None;
        }
        let window = std::mem::take(&mut self.window);
        let memory = memory();
        if std::env::var_os("VALLE_PERF").is_some() {
            eprintln!(
                "[valle metal] queue-window queues={} frames={} frame-us={} gpu-wait-us={} prepare-wait-us={} sink-us={} submit-us={} execute-us={} allocated={} budget={}",
                self.queues,
                window.frames,
                window.sample.wall_us / window.frames as u64,
                window.sample.gpu_wait_us,
                window.sample.prepare_wait_us,
                window.sample.sink_us,
                window.sample.submit_us,
                window.sample.execute_us,
                memory.map_or(0, |m| m.allocated),
                memory.map_or(0, |m| m.budget),
            );
        }
        if self.retained_queues > 1 && memory.is_none_or(|m| m.allocated > m.budget) {
            self.probe = None;
            self.cooldown = COOLDOWN_FRAMES;
            return self.change(1, 1, "memory-budget");
        }
        if let Some(probe) = self.probe.take() {
            match probe {
                Probe::Expanded {
                    previous,
                    reference,
                } => {
                    if !window.comparable(reference) {
                        let candidate = self.queues;
                        self.probe = Some(Probe::Recheck {
                            previous,
                            candidate,
                            parallel: window,
                        });
                        return self.change(previous, candidate, "recheck-changing-work");
                    }
                    if !window.improves(reference) {
                        self.cooldown = COOLDOWN_FRAMES;
                        return self.change(previous, previous, "no-throughput-gain");
                    }
                    if std::env::var_os("VALLE_PERF").is_some() {
                        eprintln!("[valle metal] queue-probe accepted queues={}", self.queues);
                    }
                }
                Probe::Recheck {
                    previous,
                    candidate,
                    parallel,
                } => {
                    if parallel.comparable(window) && parallel.improves(window) {
                        return self.change(candidate, candidate, "confirmed-throughput-gain");
                    }
                    self.cooldown = COOLDOWN_FRAMES;
                    return self.change(previous, previous, "no-confirmed-throughput-gain");
                }
            }
        }
        // Low completion wait is normally the purpose of concurrency, not a
        // reason to undo it. Reduce only when another stage actually dominates.
        if self.queues > 1
            && window.sample.gpu_wait_us < window.sample.wall_us / 10
            && window
                .sample
                .prepare_wait_us
                .saturating_add(window.sample.sink_us)
                > window.sample.wall_us / 5 * 3
        {
            self.idle_windows += 1;
            if self.idle_windows >= 2 {
                self.cooldown = COOLDOWN_FRAMES;
                return self.change(1, 1, "prepare-or-encoder-bound");
            }
        } else {
            self.idle_windows = 0;
        }
        if self.cooldown == 0
            && self.queues < self.maximum
            && remaining >= (WINDOW_FRAMES + WARMUP_FRAMES) * 2
            && window.gpu_bound()
        {
            // Cover the bounded in-flight window when memory permits. Going
            // through two queues first can thrash temporal caches and does not
            // predict the benefit of three independent frame contexts.
            let target = (self.queues + 1..=self.maximum)
                .rev()
                .find(|&target| memory.is_some_and(|m| m.permits_growth(self.queues, target)));
            if let Some(target) = target {
                self.probe = Some(Probe::Expanded {
                    previous: self.queues,
                    reference: window,
                });
                return self.change(target, target, "gpu-wait-probe");
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(wall: u64, wait: u64) -> QueueSample {
        QueueSample {
            wall_us: wall,
            gpu_wait_us: wait,
            execute_us: 5_000,
            ..Default::default()
        }
    }

    fn feed(
        policy: &mut AdaptiveQueues,
        sample: QueueSample,
        memory: MetalMemory,
    ) -> Vec<QueueChange> {
        let frames = if matches!(policy.probe, Some(Probe::Recheck { .. })) {
            RECHECK_FRAMES
        } else {
            WINDOW_FRAMES
        };
        (0..frames + policy.warmup)
            .filter_map(|_| policy.observe(sample, 300, || Some(memory)))
            .collect()
    }

    fn start_probe(policy: &mut AdaptiveQueues, memory: MetalMemory) -> Vec<QueueChange> {
        feed(policy, sample(20_000, 12_000), memory)
    }

    const MEMORY: MetalMemory = MetalMemory {
        allocated: 1024 * 1024 * 1024,
        budget: 6 * 1024 * 1024 * 1024,
    };

    #[test]
    fn cpu_encoder_and_light_gpu_work_do_not_create_extra_contexts() {
        for sample in [
            sample(10_000, 100),
            sample(10_000, 4_000),
            QueueSample {
                prepare_wait_us: 6_000,
                ..sample(20_000, 12_000)
            },
            QueueSample {
                sink_us: 6_000,
                ..sample(20_000, 12_000)
            },
        ] {
            let mut policy = AdaptiveQueues::new(3);
            assert!(feed(&mut policy, sample, MEMORY).is_empty());
            assert!(feed(&mut policy, sample, MEMORY).is_empty());
        }
    }

    #[test]
    fn only_profitable_probes_expand_and_unprofitable_probes_back_off() {
        let mut policy = AdaptiveQueues::new(3);
        assert_eq!(start_probe(&mut policy, MEMORY)[0].queues, 3);
        assert!(feed(&mut policy, sample(15_000, 100), MEMORY).is_empty());
        // Eliminating completion waits is a successful probe and stays enabled.
        assert!(feed(&mut policy, sample(15_000, 100), MEMORY).is_empty());
        assert!(feed(&mut policy, sample(15_000, 100), MEMORY).is_empty());
        assert_eq!(policy.queues, 3);
        let mut policy = AdaptiveQueues::new(3);
        start_probe(&mut policy, MEMORY);
        let result = feed(&mut policy, sample(20_000, 12_000), MEMORY);
        assert_eq!(result[0].queues, 1);
        assert_eq!(result[0].reason, "no-throughput-gain");
        assert!(feed(&mut policy, sample(20_000, 12_000), MEMORY).is_empty());
    }

    #[test]
    fn process_wide_memory_and_shadow_restrictions_override_gpu_wait() {
        let tight = MetalMemory {
            budget: MEMORY.allocated + MINIMUM_EXTRA_BYTES - 1,
            ..MEMORY
        };
        assert!(start_probe(&mut AdaptiveQueues::new(3), tight).is_empty());
        let mut policy = AdaptiveQueues::new(3);
        start_probe(&mut policy, MEMORY);
        let result = feed(
            &mut policy,
            sample(15_000, 7_000),
            MetalMemory {
                budget: 1,
                ..MEMORY
            },
        );
        assert_eq!(result[0].reason, "memory-budget");
        assert_eq!(result[0].queues, 1);
        policy.restrict_to(1);
        assert!(feed(&mut policy, sample(20_000, 12_000), MEMORY).is_empty());
    }

    #[test]
    fn short_tails_and_one_worker_do_not_probe() {
        let mut policy = AdaptiveQueues::new(3);
        for _ in 0..100 {
            assert!(
                policy
                    .observe(sample(20_000, 12_000), 30, || Some(MEMORY))
                    .is_none()
            );
        }
        assert!(feed(&mut AdaptiveQueues::new(1), sample(20_000, 12_000), MEMORY).is_empty());
    }

    #[test]
    fn memory_can_limit_a_probe_to_two_contexts() {
        let memory = MetalMemory {
            budget: MEMORY.allocated * 2,
            ..MEMORY
        };
        let mut policy = AdaptiveQueues::new(3);
        assert_eq!(start_probe(&mut policy, memory)[0].queues, 2);
    }

    #[test]
    fn sustained_preparation_bottleneck_releases_optional_contexts() {
        let mut policy = AdaptiveQueues::new(3);
        start_probe(&mut policy, MEMORY);
        feed(&mut policy, sample(15_000, 100), MEMORY);
        let cpu_bound = QueueSample {
            prepare_wait_us: 14_000,
            ..sample(20_000, 100)
        };
        assert!(feed(&mut policy, cpu_bound, MEMORY).is_empty());
        let result = feed(&mut policy, cpu_bound, MEMORY);
        assert_eq!(result[0].queues, 1);
        assert_eq!(result[0].reason, "prepare-or-encoder-bound");
    }

    #[test]
    fn changing_visible_work_gets_a_fresh_reference_without_cold_contexts() {
        let mut policy = AdaptiveQueues::new(3);
        assert_eq!(start_probe(&mut policy, MEMORY)[0].queues, 3);
        let grown = QueueSample {
            execute_us: 12_000,
            ..sample(40_000, 24_000)
        };
        let result = feed(&mut policy, grown, MEMORY);
        assert_eq!((result[0].queues, result[0].retained_queues), (1, 3));
        let serial = QueueSample {
            wall_us: 60_000,
            gpu_wait_us: 40_000,
            ..grown
        };
        let result = feed(&mut policy, serial, MEMORY);
        assert_eq!(result[0].queues, 3);
        assert_eq!(result[0].reason, "confirmed-throughput-gain");
    }

    #[test]
    fn a_short_gpu_burst_does_not_create_contexts() {
        let mut policy = AdaptiveQueues::new(3);
        for _ in 0..WARMUP_FRAMES {
            policy.observe(sample(10_000, 100), 300, || Some(MEMORY));
        }
        for _ in 0..WINDOW_FRAMES / 2 {
            assert!(
                policy
                    .observe(sample(20_000, 12_000), 300, || Some(MEMORY))
                    .is_none()
            );
        }
        assert!(feed(&mut policy, sample(10_000, 100), MEMORY).is_empty());
        assert_eq!(policy.queues, 1);
    }

    #[test]
    fn unsuccessful_rechecks_and_memory_pressure_release_retained_contexts() {
        for memory_pressure in [false, true] {
            let mut policy = AdaptiveQueues::new(3);
            start_probe(&mut policy, MEMORY);
            let grown = QueueSample {
                execute_us: 12_000,
                ..sample(40_000, 24_000)
            };
            feed(&mut policy, grown, MEMORY);
            assert_eq!((policy.queues, policy.retained_queues), (1, 3));
            let memory = if memory_pressure {
                MetalMemory {
                    budget: 1,
                    ..MEMORY
                }
            } else {
                MEMORY
            };
            let result = feed(&mut policy, grown, memory);
            assert_eq!((result[0].queues, result[0].retained_queues), (1, 1));
            assert_eq!(
                result[0].reason,
                if memory_pressure {
                    "memory-budget"
                } else {
                    "no-confirmed-throughput-gain"
                }
            );
        }
    }
}
