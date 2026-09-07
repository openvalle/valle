//! Crash injection through `VALLE_ASSETS_CRASH_AT`. Abort at named write checkpoints without
//! destructors or flushing to verify atomic metadata commits and index recovery.

/// Abort immediately when the environment selects this checkpoint.
#[inline]
pub fn maybe_crash(label: &str) {
    if let Some(v) = std::env::var_os("VALLE_ASSETS_CRASH_AT") {
        if v.to_string_lossy() == label {
            std::process::abort();
        }
    }
}
