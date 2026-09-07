//! Structured events with one NDJSON envelope per stdout line: `ts_ms`, `seq`, `level`, `type`, and
//! `data`. Timestamps use UTC epoch milliseconds; sequence numbers increase within the process.
//! Human mode renders the same events to stderr. Ignore write failures when a consumer closes its
//! pipe. Machine-readable events use typed variants rather than parsed log messages.

use serde::Serialize;
use serde_json::Value;
use std::io::Write as _;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

/// Process-wide output mode, initialized once; defaults to human-readable output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    Human,
    Ndjson,
}

static MODE: OnceLock<Mode> = OnceLock::new();
static SEQ: AtomicU64 = AtomicU64::new(0);

/// Initialize NDJSON output for service commands or commands using `--events`.
pub(crate) fn init_ndjson() {
    let _ = MODE.set(Mode::Ndjson);
}

pub(crate) fn mode() -> Mode {
    *MODE.get().unwrap_or(&Mode::Human)
}

/// Severity derived from the event type, except for explicit log events.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Level {
    Info,
    Warn,
}

/// Typed events form the machine-readable contract; log messages are informational.
#[derive(Serialize, Clone, Debug)]
#[serde(tag = "type", content = "data")]
pub(crate) enum EventKind {
    // Service events.
    /// Service readiness, including the actual bound port.
    #[serde(rename = "ready")]
    Ready {
        url: String,
        port: u16,
        runtime_version: String,
        runtime_source: String,
    },
    /// Motion source hot reload completed and config advanced.
    #[serde(rename = "reload")]
    Reload { generation: u64 },
    /// Browser observation from POST /result. PNG data is saved to disk and replaced by its capture
    /// path.
    #[serde(rename = "browser_report")]
    BrowserReport {
        status: String,
        message: Value,
        generation: Value,
        capture: Value,
        stats: Value,
        audio: Value,
    },

    // Command events.
    /// Final event with the report envelope used directly as `data`.
    #[serde(rename = "report")]
    Report(Value),
    /// Analysis progress forwarded from the kernel.
    #[serde(rename = "analyze.progress")]
    AnalyzeProgress { message: String },
    #[serde(rename = "render.progress")]
    RenderProgress { completed: usize, total: usize },
}

impl EventKind {
    /// Derive envelope severity from the event.
    fn level(&self) -> Level {
        match self {
            EventKind::BrowserReport { status, .. } if status != "ok" => Level::Warn,
            _ => Level::Info,
        }
    }

    /// Render an optional human-readable stderr line. Service events remain exclusive to NDJSON.
    fn human(&self) -> Option<String> {
        match self {
            EventKind::AnalyzeProgress { message } => Some(message.clone()),
            _ => None,
        }
    }
}

/// Complete stdout envelope with `type` and `data` flattened at the top level.
#[derive(Serialize)]
struct Envelope {
    ts_ms: u64,
    seq: u64,
    level: Level,
    #[serde(flatten)]
    event: EventKind,
}

fn ts_ms_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Serialize one compact envelope without a trailing newline.
fn envelope_line(event: &EventKind, ts_ms: u64, seq: u64) -> String {
    let env = Envelope {
        ts_ms,
        seq,
        level: event.level(),
        event: event.clone(),
    };
    serde_json::to_string(&env).unwrap_or_default()
}

/// Write and flush one stdout line; ignore closed-pipe errors.
pub(crate) fn stdout_line(line: &str) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

/// Emit an event as NDJSON on stdout or human-readable text on stderr.
pub(crate) fn emit(event: EventKind) {
    match mode() {
        Mode::Ndjson => {
            let line = envelope_line(&event, ts_ms_now(), SEQ.fetch_add(1, Ordering::Relaxed));
            if !line.is_empty() {
                stdout_line(&line);
            }
        }
        Mode::Human => {
            if let Some(text) = event.human() {
                let mut err = std::io::stderr().lock();
                let _ = writeln!(err, "{text}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Keep type and data at the top level of the envelope.
    #[test]
    fn envelope_flattens_type_and_data() {
        let line = envelope_line(
            &EventKind::Ready {
                url: "http://127.0.0.1:7777/console".into(),
                port: 7777,
                runtime_version: "1.2.3".into(),
                runtime_source: "dev-dir".into(),
            },
            1_752_812_345_678,
            3,
        );
        let v: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["ts_ms"], 1_752_812_345_678u64);
        assert_eq!(v["seq"], 3);
        assert_eq!(v["level"], "info");
        assert_eq!(v["type"], "ready");
        assert_eq!(v["data"]["port"], 7777);
        assert_eq!(v["data"]["runtime_source"], "dev-dir");
        assert!(
            v.get("event").is_none(),
            "event payload must not be nested: {line}"
        );
    }

    /// Each event occupies one compact NDJSON line.
    #[test]
    fn envelope_is_single_line() {
        let line = envelope_line(
            &EventKind::Report(serde_json::json!({"message": "boom"})),
            0,
            0,
        );
        assert!(!line.contains('\n'), "{line}");
    }

    /// Level is derived from browser status and typed log severity.
    #[test]
    fn level_derivation() {
        let report = |status: &str| EventKind::BrowserReport {
            status: status.into(),
            message: Value::Null,
            generation: Value::Null,
            capture: Value::Null,
            stats: Value::Null,
            audio: Value::Null,
        };
        assert_eq!(report("ok").level(), Level::Info);
        assert_eq!(report("error").level(), Level::Warn);
    }

    /// Sequence numbers increase within the process.
    #[test]
    fn seq_is_monotonic() {
        let a = SEQ.fetch_add(1, Ordering::Relaxed);
        let b = SEQ.fetch_add(1, Ordering::Relaxed);
        assert!(b > a);
    }
}
