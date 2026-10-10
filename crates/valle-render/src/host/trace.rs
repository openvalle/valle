//! Opt-in, bounded Chrome/Perfetto trace. Every timestamp uses one monotonic clock;
//! spans describe CPU work/waits, never pretend to be device GPU timestamps.
use std::{
    collections::HashMap,
    io::{self, Write},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    thread::ThreadId,
    time::Instant,
};

use serde_json::{Value, json};

#[derive(Clone)]
pub struct RenderTrace(Arc<Inner>);

struct Inner {
    epoch: Instant,
    limit: usize,
    next_frame: AtomicU64,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    events: Vec<Value>,
    threads: HashMap<ThreadId, usize>,
    dropped: u64,
}

pub(crate) struct TraceSpan {
    trace: RenderTrace,
    started: Instant,
    name: &'static str,
    args: Value,
    tid: usize,
}

#[derive(Clone)]
pub(crate) struct TraceFrame {
    pub(crate) trace: RenderTrace,
    id: u64,
    frame: i64,
}

impl RenderTrace {
    pub fn new(maximum_events: usize) -> Self {
        Self(Arc::new(Inner {
            epoch: Instant::now(),
            limit: maximum_events,
            next_frame: AtomicU64::new(1),
            state: Mutex::new(State::default()),
        }))
    }

    pub fn record(&self, name: &'static str, started: Instant, args: Value) {
        self.span_at(name, started, args);
    }

    pub fn evidence(&self, name: &'static str, args: Value) {
        self.push(
            json!({"name": name, "cat": "valle.evidence", "ph": "i", "s": "t", "pid": 1,
            "tid": self.thread(), "ts": self.0.epoch.elapsed().as_micros() as u64, "args": args}),
        );
    }

    pub(crate) fn span(&self, name: &'static str, args: Value) -> TraceSpan {
        TraceSpan {
            trace: self.clone(),
            started: Instant::now(),
            name,
            args,
            tid: self.thread(),
        }
    }

    fn span_at(&self, name: &'static str, started: Instant, args: Value) {
        let now = Instant::now();
        self.push(json!({
            "name": name, "cat": "valle.cpu", "ph": "X", "pid": 1, "tid": self.thread(),
            "ts": started.saturating_duration_since(self.0.epoch).as_micros() as u64,
            "dur": now.saturating_duration_since(started).as_micros() as u64, "args": args,
        }));
    }

    fn thread(&self) -> usize {
        let mut state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        let current = std::thread::current();
        if let Some(tid) = state.threads.get(&current.id()) {
            return *tid;
        }
        if state.threads.len() >= self.0.limit {
            return 0;
        }
        let next = state.threads.len() + 1;
        *state.threads.entry(current.id()).or_insert(next)
    }

    fn push(&self, event: Value) {
        let mut state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.events.len() < self.0.limit {
            state.events.push(event);
        } else {
            state.dropped += 1;
        }
    }

    pub(crate) fn frame(&self, frame: i64) -> TraceFrame {
        TraceFrame {
            trace: self.clone(),
            id: self.0.next_frame.fetch_add(1, Ordering::Relaxed),
            frame,
        }
    }

    /// A snapshot is bounded even on long exports. A nonzero dropped count means the trace
    /// is incomplete; the benchmark runner rejects it rather than silently reporting full coverage.
    pub fn snapshot(&self, metadata: Value) -> Value {
        let state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        json!({
            "traceEvents": state.events, "displayTimeUnit": "ms",
            "metadata": { "description": metadata, "maximumEvents": self.0.limit,
                "droppedEvents": state.dropped, "clock": "shared monotonic Instant",
                "gpuDeviceTime": "unmeasured", "memoryScope": "per-owner reported estimates; not process allocations" },
        })
    }

    pub fn write(&self, path: &Path, metadata: Value) -> io::Result<()> {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        let mut file = io::BufWriter::new(file);
        serde_json::to_writer(&mut file, &self.snapshot(metadata))?;
        file.write_all(b"\n")?;
        file.flush()
    }
}

impl TraceFrame {
    pub(crate) fn span(&self, name: &'static str) -> TraceSpan {
        self.trace
            .span(name, json!({"frame": self.frame, "request": self.id}))
    }

    pub(crate) fn counters(&self, name: &'static str, values: Value) {
        self.trace.push(json!({
            "name": name, "cat": "valle.evidence", "ph": "i", "s": "t",
            "pid": 1, "tid": self.trace.thread(),
            "ts": self.trace.0.epoch.elapsed().as_micros() as u64,
            "args": {"frame": self.frame, "request": self.id, "values": values},
        }));
    }
}

impl Drop for TraceSpan {
    fn drop(&mut self) {
        self.trace.push(json!({
            "name": self.name, "cat": "valle.cpu", "ph": "X", "pid": 1, "tid": self.tid,
            "ts": self.started.saturating_duration_since(self.trace.0.epoch).as_micros() as u64,
            "dur": self.started.elapsed().as_micros() as u64, "args": self.args,
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trace_is_bounded_and_cross_thread_spans_share_one_clock() {
        let trace = RenderTrace::new(3);
        let frame = trace.frame(7);
        {
            let _span = frame.span("prepare");
        }
        std::thread::scope(|s| {
            s.spawn(|| {
                let _span = frame.span("lower");
            });
        });
        frame.counters("cache", json!({"bytes": 12}));
        {
            let _span = frame.span("execute");
        }
        let snapshot = trace.snapshot(json!({}));
        let events = snapshot["traceEvents"].as_array().unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(snapshot["metadata"]["droppedEvents"], 1);
        assert_ne!(events[0]["tid"], events[1]["tid"]);
        assert_eq!(events[0]["args"]["request"], events[1]["args"]["request"]);
        assert!(events[1]["ts"].as_u64().unwrap() >= events[0]["ts"].as_u64().unwrap());
    }

    #[cfg(feature = "native")]
    #[test]
    fn trace_write_flushes_complete_json_and_preserves_existing_files() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("trace.json");
        let trace = RenderTrace::new(8);
        trace.evidence("cache", json!({"bytes": 12}));
        trace.write(&path, json!({"status": "complete"})).unwrap();
        let saved = std::fs::read(&path).unwrap();
        assert!(saved.ends_with(b"\n"));
        let parsed: Value = serde_json::from_slice(&saved).unwrap();
        assert_eq!(parsed["traceEvents"][0]["args"]["bytes"], 12);
        assert_eq!(parsed["metadata"]["description"]["status"], "complete");
        assert_eq!(
            trace.write(&path, json!({})).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(std::fs::read(&path).unwrap(), saved);
    }
}
