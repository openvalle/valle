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

/// No compatible FFmpeg installation could be loaded. Callers can find it in an error chain to
/// report a dedicated code; its message ends with the platform install hint.
#[derive(Debug)]
pub struct FfmpegUnavailable(pub(crate) String);

impl std::fmt::Display for FfmpegUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}\nhint: {}", self.0, ffmpeg_install_hint())
    }
}

impl std::error::Error for FfmpegUnavailable {}

/// One platform-specific way to install loadable FFmpeg shared libraries.
fn ffmpeg_install_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        "install FFmpeg with `brew install ffmpeg`, or set VALLE_FFMPEG_DIR to another installation. Run `valle docs cli` for details."
    } else if cfg!(target_os = "windows") {
        "install a shared build (for example `winget install Gyan.FFmpeg.Shared`) and add its bin directory to PATH, or set VALLE_FFMPEG_DIR to the build's folder. Run `valle docs cli` for details."
    } else {
        "install FFmpeg 7 or newer from your distribution (for example `sudo apt install ffmpeg` on Debian 13 or Ubuntu 26.04), or extract a shared build and set VALLE_FFMPEG_DIR to its folder. Run `valle docs cli` for details."
    }
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
