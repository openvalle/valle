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
        crate::events::stdout_line(&value.to_string());
    }
}
pub(crate) fn error(message: &str) {
    error_with_code("command_failed", message);
}
pub(crate) fn error_with_code(code: &str, message: &str) {
    error_with_details(code, message, serde_json::Map::new(), None);
}

/// An error with machine-readable fields beside its code and message, and a next step that
/// human output prints as a `hint:` line.
pub(crate) fn error_with_details(
    code: &str,
    message: &str,
    mut fields: serde_json::Map<String, Value>,
    hint: Option<&str>,
) {
    if machine() {
        fields.insert("code".into(), code.into());
        fields.insert("message".into(), message.into());
        if let Some(hint) = hint {
            fields.insert("hint".into(), hint.into());
        }
        emit(json!({"status":"error","error":fields}));
    } else {
        eprintln!("error: {message}");
        if let Some(hint) = hint {
            eprintln!("  hint: {hint}");
        }
    }
}

pub(crate) fn render_progress() -> Option<valle_render::host::ProgressCallback> {
    events().then(|| {
        std::sync::Arc::new(|completed, total| {
            crate::events::emit(crate::events::EventKind::RenderProgress { completed, total });
        }) as valle_render::host::ProgressCallback
    })
}
