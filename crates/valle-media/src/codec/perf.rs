//! Codec timing counters controlled by VALLE_PERF and read by render reports. Track source
//! conversion, encoder conversion, and encoder submission separately; source conversion may run on
//! raster workers. Disabled timing requires only a relaxed flag read.

use std::env;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::time::Instant;

static ENABLED: AtomicBool = AtomicBool::new(false);
static SRC_CONVERT_NS: AtomicU64 = AtomicU64::new(0);
static ENC_CONVERT_NS: AtomicU64 = AtomicU64::new(0);
static ENC_SUBMIT_NS: AtomicU64 = AtomicU64::new(0);

/// Initialize from VALLE_PERF and reset counters alongside render timing state.
pub fn init_from_env_and_reset() {
    let on = env::var("VALLE_PERF")
        .ok()
        .map(|v| !v.is_empty() && v != "0" && v != "false")
        .unwrap_or(false);
    ENABLED.store(on, Relaxed);
    for s in [&SRC_CONVERT_NS, &ENC_CONVERT_NS, &ENC_SUBMIT_NS] {
        s.store(0, Relaxed);
    }
}

/// Cumulative nanoseconds for source conversion, encoder conversion, and encoder submission.
pub fn snapshot_ns() -> (u64, u64, u64) {
    (
        SRC_CONVERT_NS.load(Relaxed),
        ENC_CONVERT_NS.load(Relaxed),
        ENC_SUBMIT_NS.load(Relaxed),
    )
}

/// Accumulate execution time when enabled; otherwise call the function directly.
#[inline]
fn time<T>(slot: &AtomicU64, f: impl FnOnce() -> T) -> T {
    if !ENABLED.load(Relaxed) {
        return f();
    }
    let start = Instant::now();
    let out = f();
    slot.fetch_add(start.elapsed().as_nanos() as u64, Relaxed);
    out
}

/// Time source YUV-to-RGBA conversion, plane copying, and rotation.
#[inline]
pub fn time_src_convert<T>(f: impl FnOnce() -> T) -> T {
    time(&SRC_CONVERT_NS, f)
}

/// Time encoder-side pixel allocation, copying, and RGBA-to-YUV conversion.
#[inline]
pub fn time_enc_convert<T>(f: impl FnOnce() -> T) -> T {
    time(&ENC_CONVERT_NS, f)
}

/// Time encoder submission and mux packet writes.
#[inline]
pub fn time_enc_submit<T>(f: impl FnOnce() -> T) -> T {
    time(&ENC_SUBMIT_NS, f)
}
