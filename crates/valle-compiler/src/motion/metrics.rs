//! Native scoped observations of actual compiler invocations, independent of host scheduling.

use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CompilationMetrics {
    pub compilations: u64,
    pub elapsed: Duration,
    pub template_compile: Duration,
    pub instance_data: Duration,
}

#[derive(Debug, Clone)]
pub struct CompilationEvent {
    pub entry: String,
    pub elapsed: Duration,
}

struct Observer {
    metrics: Cell<CompilationMetrics>,
    completed: Box<dyn Fn(&CompilationEvent)>,
}

thread_local! {
    static OBSERVERS: RefCell<Vec<Weak<Observer>>> = const { RefCell::new(Vec::new()) };
}

/// Observe compilations made on this thread while the trace is alive, including failed calls.
/// Nested traces each observe the same invocation; other threads cannot contaminate the count.
/// Hosts compiling on worker threads must create a trace on each worker and aggregate explicitly.
pub struct CompilationTrace(Rc<Observer>);

impl CompilationTrace {
    pub fn new(completed: impl Fn(&CompilationEvent) + 'static) -> Self {
        let observer = Rc::new(Observer {
            metrics: Cell::new(CompilationMetrics::default()),
            completed: Box::new(completed),
        });
        OBSERVERS.with_borrow_mut(|observers| {
            observers.retain(|observer| observer.strong_count() != 0);
            observers.push(Rc::downgrade(&observer));
        });
        Self(observer)
    }

    pub fn metrics(&self) -> CompilationMetrics {
        self.0.metrics.get()
    }
}

pub(super) struct CompilationCall {
    entry: String,
    started: Instant,
    observers: Vec<Rc<Observer>>,
}

pub(super) fn record_compilation(entry: &str) -> Option<CompilationCall> {
    let observers = OBSERVERS.with_borrow(|observers| {
        observers
            .iter()
            .filter_map(Weak::upgrade)
            .collect::<Vec<_>>()
    });
    if observers.is_empty() {
        return None;
    }
    for observer in &observers {
        let mut metrics = observer.metrics.get();
        metrics.compilations += 1;
        observer.metrics.set(metrics);
    }
    Some(CompilationCall {
        entry: entry.to_owned(),
        started: Instant::now(),
        observers,
    })
}

pub(super) fn record_template_compile(elapsed: Duration) {
    OBSERVERS.with_borrow(|observers| {
        for observer in observers.iter().filter_map(Weak::upgrade) {
            let mut metrics = observer.metrics.get();
            metrics.template_compile += elapsed;
            observer.metrics.set(metrics);
        }
    });
}

pub(super) fn record_instance_data(elapsed: Duration) {
    OBSERVERS.with_borrow(|observers| {
        for observer in observers.iter().filter_map(Weak::upgrade) {
            let mut metrics = observer.metrics.get();
            metrics.instance_data += elapsed;
            observer.metrics.set(metrics);
        }
    });
}

impl Drop for CompilationCall {
    fn drop(&mut self) {
        let event = CompilationEvent {
            entry: self.entry.clone(),
            elapsed: self.started.elapsed(),
        };
        for observer in &self.observers {
            let mut metrics = observer.metrics.get();
            metrics.elapsed += event.elapsed;
            observer.metrics.set(metrics);
            (observer.completed)(&event);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::motion::{MotionModuleGraph, compile_motion, compile_motion_modules};

    #[test]
    fn counts_real_calls_through_every_entry_and_reports_failed_compiles() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let received = Rc::clone(&events);
        let trace = CompilationTrace::new(move |event| received.borrow_mut().push(event.clone()));
        let source = "export default function Title() { return <Scene />; }";
        for _ in 0..3 {
            compile_motion(source).unwrap();
        }
        let graph = MotionModuleGraph::new(
            "main.motion.tsx",
            [("main.motion.tsx".into(), source.into())].into(),
        )
        .unwrap();
        compile_motion_modules(&graph).unwrap();
        assert!(compile_motion("export default function Broken( {").is_err());
        assert_eq!(trace.metrics().compilations, 5);
        assert_eq!(events.borrow().len(), 5);
        assert_eq!(events.borrow()[3].entry, "main.motion.tsx");
        assert_eq!(
            trace.metrics().elapsed,
            events.borrow().iter().map(|e| e.elapsed).sum()
        );
        // Independent worker compilations must not affect this caller's measurements.
        std::thread::spawn(move || compile_motion(source).unwrap())
            .join()
            .unwrap();
        assert_eq!(trace.metrics().compilations, 5);
        drop(trace);
        compile_motion(source).unwrap();
        assert_eq!(events.borrow().len(), 5);
    }
}
