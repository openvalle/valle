//! Crash-injection hook for the immutable revision publish protocol.

/// Abort at an explicitly selected durability boundary.
///
/// This is intentionally a process abort: destructors and buffered clean-up
/// do not run, which lets recovery tests exercise power-loss semantics.
#[inline]
pub(super) fn maybe_crash(label: &str) {
    if let Some(value) = std::env::var_os("VALLE_PROJECT_CRASH_AT") {
        if value.to_string_lossy() == label {
            std::process::abort();
        }
    }
}
