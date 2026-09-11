//! Version-independent native media runtime configuration.
use super::backend::{self, Version};
use anyhow::Result;
use std::sync::atomic::{AtomicI32, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum FfmpegLogLevel {
    Quiet = -8,
    Panic = 0,
    Fatal = 8,
    Error = 16,
    Warning = 24,
    Info = 32,
    Verbose = 40,
    Debug = 48,
    Trace = 56,
}
static LOG_LEVEL: AtomicI32 = AtomicI32::new(FfmpegLogLevel::Error as i32);
pub(crate) fn log_level() -> i32 {
    LOG_LEVEL.load(Ordering::Relaxed)
}
pub fn set_ffmpeg_log_level(level: FfmpegLogLevel) {
    LOG_LEVEL.store(level as i32, Ordering::Relaxed);
    backend::update_log_level(level as i32);
}
pub fn set_ffmpeg_directory(path: std::path::PathBuf) -> Result<()> {
    backend::set_directory(path)
}
pub fn ffmpeg_init() -> Result<()> {
    backend::version().map(|_| ())
}
pub fn ffmpeg_capabilities() -> Result<serde_json::Value> {
    match backend::version()? {
        Version::V7 => backend::v7::ffi::ffmpeg_capabilities(),
        Version::V8 => backend::v8::ffi::ffmpeg_capabilities(),
        Version::V9 => backend::v9::ffi::ffmpeg_capabilities(),
    }
}

/// Supported SDR YUV/RGB matrices: BT.601 and BT.709.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum YuvMatrix {
    Bt601,
    Bt709,
}
