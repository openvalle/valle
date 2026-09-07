//! Shared process output mode. Diagnostics never enter the result stream.
use serde_json::{Value, json};
use std::sync::atomic::{AtomicU8, Ordering};
static MODE: AtomicU8 = AtomicU8::new(0);
pub(crate) fn init(json: bool, events: bool) {
    MODE.store(
        if events {
            2
        } else if json {
            1
        } else {
            0
        },
        Ordering::Relaxed,
    );
    if events {
        crate::events::init_ndjson();
    }
}
pub(crate) fn machine() -> bool {
    MODE.load(Ordering::Relaxed) != 0
}
pub(crate) fn events() -> bool {
    MODE.load(Ordering::Relaxed) == 2
}
pub(crate) fn emit(value: Value) {
    if events() {
        crate::events::emit(crate::events::EventKind::Report(value));
    } else {
        println!("{value}");
    }
}
pub(crate) fn error(message: &str) {
    if machine() {
        emit(json!({"status":"error","error":{"code":"command_failed","message":message}}));
    } else {
        eprintln!("error: {message}");
    }
}

pub(crate) fn render_progress() -> Option<valle_render::host::ProgressCallback> {
    events().then(|| {
        std::sync::Arc::new(|completed, total| {
            crate::events::emit(crate::events::EventKind::RenderProgress { completed, total });
        }) as valle_render::host::ProgressCallback
    })
}
