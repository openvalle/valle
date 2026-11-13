//! Where a render's time and memory go, said when `--perf` is given.
//!
//! A stage is a named span of work, `perf::span("eval")`, timed by its thread. Every span of a
//! stage on any thread adds to it: how many, the time in all of them, the middle one and the
//! slowest. A frame loop's stages report a distribution, so the one frame that traced is not
//! hidden by the ones that did not.
//!
//! A span inside another, as a scene loaded while it is read, is taken out of its parent's
//! time: each stage's `ms` is its own, and every stage's `ms` added together is the work done,
//! not more.
//!
//! Off, a span is a check of one atomic and nothing else.

use std::cell::RefCell;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use serde_json::{Value, json};

/// Whether spans are timed at all: set once before work starts.
static ON: AtomicBool = AtomicBool::new(false);

/// The stages, by name, each with every span it was given.
static STAGES: OnceLock<Mutex<Vec<Stage>>> = OnceLock::new();

/// What one stage has been given.
#[derive(Default)]
struct Stage {
    name: &'static str,
    /// Each span's microseconds, for the distribution.
    spans: Vec<u64>,
    /// The resident bytes when the stage first began.
    rss: Option<u64>,
}

/// Turns timing on: spans given from here on are recorded.
pub fn enable() {
    ON.store(true, Ordering::Relaxed);
}

/// Whether timing is on.
pub fn enabled() -> bool {
    ON.load(Ordering::Relaxed)
}

thread_local! {
    /// The spans this thread has open, innermost last: what a span inside another is taken
    /// out of.
    static OPEN: RefCell<Vec<Open>> = const { RefCell::new(Vec::new()) };
    /// This thread's next span's id.
    static NEXT: RefCell<u64> = const { RefCell::new(0) };
}

/// A span this thread has open.
struct Open {
    id: u64,
    started: Instant,
    /// The µs of the spans opened and closed inside this one.
    nested: u64,
}

/// Times `name` from here to the end of the guard's scope.
pub fn span(name: &'static str) -> Span {
    if !enabled() {
        return Span { name, id: None };
    }
    let id = NEXT.with(|n| {
        let mut next = n.borrow_mut();
        *next += 1;
        *next
    });
    OPEN.with(|open| {
        open.borrow_mut().push(Open {
            id,
            started: Instant::now(),
            nested: 0,
        })
    });
    Span { name, id: Some(id) }
}

/// A stage being timed, ended when it is dropped; with timing off it times nothing.
pub struct Span {
    name: &'static str,
    id: Option<u64>,
}

impl Drop for Span {
    fn drop(&mut self) {
        let Some(id) = self.id.take() else {
            return;
        };
        let micros = OPEN.with(|open| {
            let mut open = open.borrow_mut();
            // Closed on another thread than it was opened: timed, taken out of nothing.
            let at = open.iter().rposition(|o| o.id == id)?;
            let span = open.remove(at);
            let elapsed = u64::try_from(span.started.elapsed().as_micros()).unwrap_or(u64::MAX);
            if at > 0 {
                open[at - 1].nested += elapsed;
            }
            Some(elapsed.saturating_sub(span.nested))
        });
        if let Some(micros) = micros {
            record(self.name, micros);
        }
    }
}

fn stages() -> MutexGuard<'static, Vec<Stage>> {
    STAGES
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

fn record(name: &'static str, micros: u64) {
    let mut stages = stages();
    let stage = match stages.iter().position(|s| s.name == name) {
        Some(i) => &mut stages[i],
        None => {
            stages.push(Stage {
                name,
                spans: Vec::new(),
                rss: rss(),
            });
            let last = stages.len() - 1;
            &mut stages[last]
        }
    };
    stage.spans.push(micros);
}

/// The resident bytes this process holds, if the system tells it.
pub fn rss() -> Option<u64> {
    #[cfg(unix)]
    {
        let mut usage = std::mem::MaybeUninit::uninit();
        // SAFETY: `getrusage` fills the structure it is given for this process.
        if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } != 0 {
            return None;
        }
        // SAFETY: the call succeeded, so the structure is filled.
        let usage = unsafe { usage.assume_init() };
        // Linux counts KiB, macOS bytes.
        #[cfg(target_os = "linux")]
        let bytes = u64::from(usage.ru_maxrss) * 1024;
        #[cfg(not(target_os = "linux"))]
        let bytes = usage.ru_maxrss as u64;
        Some(bytes)
    }
    #[cfg(not(unix))]
    {
        None
    }
}

/// The values `--perf` reports beside the stages: the last reading of each.
static GAUGES: OnceLock<Mutex<Vec<(&'static str, u64)>>> = OnceLock::new();

/// Says what a name holds now, such as the GPU's bytes at the end of a render.
pub fn gauge(name: &'static str, value: u64) {
    if !enabled() {
        return;
    }
    let mut gauges = GAUGES
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    match gauges.iter_mut().find(|(n, _)| *n == name) {
        Some((_, held)) => *held = value,
        None => gauges.push((name, value)),
    }
}

/// Everything the stages were given, as `--perf` prints it: each stage's time in all its spans
/// and the distribution of its spans, the slowest first.
pub fn report() -> Value {
    let mut stages = stages();
    stages.sort_by(|a, b| {
        b.spans
            .iter()
            .sum::<u64>()
            .cmp(&a.spans.iter().sum::<u64>())
    });
    let mut entries = Vec::with_capacity(stages.len());
    for stage in stages.iter_mut() {
        let mut spans = std::mem::take(&mut stage.spans);
        spans.sort_unstable();
        let count = spans.len();
        let at = |q: usize| {
            spans
                .get(count.saturating_sub(1) * q / 100)
                .copied()
                .unwrap_or(0)
        };
        entries.push(json!({
            "stage": stage.name,
            "ms": spans.iter().sum::<u64>() as f64 / 1000.0,
            "count": count,
            "p50": at(50) as f64 / 1000.0,
            "p95": at(95) as f64 / 1000.0,
            "max": at(100) as f64 / 1000.0,
            "rss_kb": stage.rss.map(|b| b / 1024),
        }));
    }
    let gauges = GAUGES
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let mut held = serde_json::Map::new();
    for (name, value) in gauges.iter() {
        held.insert((*name).to_string(), (*value).into());
    }
    json!({
        "stages": entries,
        "ms": entries.iter().map(|e| e["ms"].as_f64().unwrap_or(0.0)).sum::<f64>(),
        "peak_rss_kb": rss().map(|b| b / 1024),
        "hold": held,
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    /// One test: enabling is for the whole process, so two tests in this process would race
    /// each other's view of it.
    #[test]
    fn spans_are_timed_once_enabled_and_a_span_inside_another_is_taken_out_of_it() {
        let quiet = stages().len();
        drop(span("quiet-test"));
        assert_eq!(stages().len(), quiet, "timed before it was enabled");

        enable();
        drop(span("loud-test"));
        let started = Instant::now();
        {
            let _outer = span("outer-test");
            std::thread::sleep(Duration::from_millis(20));
            {
                let _inner = span("inner-test");
                std::thread::sleep(Duration::from_millis(10));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let elapsed = started.elapsed().as_secs_f64() * 1000.0;
        let report = report();
        let stages = report["stages"].as_array().unwrap();
        let ms = |name: &str| {
            stages
                .iter()
                .find(|s| s["stage"] == name)
                .map(|s| s["ms"].as_f64().unwrap())
                .unwrap()
        };
        assert!(stages.iter().any(|s| s["stage"] == "loud-test"));
        let (outer, inner) = (ms("outer-test"), ms("inner-test"));
        assert!(inner >= 10.0, "inner {inner}");
        // The outer's own time leaves the inner's out: it is under what the whole took minus
        // the inner's, where timing it whole would put it at what the whole took.
        assert!(
            outer < elapsed - inner + 5.0,
            "outer {outer} elapsed {elapsed} inner {inner}"
        );
        assert!(outer >= 20.0, "outer {outer}");
    }
}
