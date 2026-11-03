//! Video files through FFmpeg, loaded at run time: version 7, 8 or 9, whichever the machine has.
//! The code is written once (`encode.rs`, `decode.rs`) and built against each version's API; the
//! one loaded picks which runs.
#![allow(clippy::duplicate_mod)]

use std::path::Path;

use valle_ffmpeg::Version;

mod v7;
mod v8;
mod v9;

/// What a video is compressed as.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Codec {
    H264,
    Hevc,
    /// ProRes 4444 with its alpha, in a .mov.
    ProRes4444,
}

/// Which encoder compresses it: the machine's hardware (VideoToolbox), or FFmpeg's own on the
/// CPU (x264, x265, prores_ks).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Encoding {
    Hardware,
    Software,
}

/// Ticks a second: the engine's unit of time, which frame times are given in.
pub const TICKS_PER_SECOND: i64 = 254_016_000_000;

/// Which decoder reads a video: the machine's hardware (VideoToolbox), or FFmpeg's own on the
/// CPU.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Decoding {
    Hardware,
    Software,
}

/// A video's frames, by time.
pub trait Read {
    /// The size of its frames, in px.
    fn size(&self) -> [u32; 2];
    /// The frame shown `at` ticks into the video, the last that starts at or before then, or the
    /// first: straight sRGB RGBA, rows top down.
    fn frame(&mut self, at: i64) -> Result<Vec<u8>, String>;
    /// When the frame it decoded last starts, in ticks: from there to `AHEAD` past it, it decodes
    /// on rather than seeks.
    fn position(&self) -> Option<i64>;
}

/// How far past the frame a reader decoded last it decodes on rather than seeks: 2 s, in ticks.
pub const AHEAD: i64 = 2 * TICKS_PER_SECOND;

/// The frames of the video at `path`, decoded as `decoding` says: by default the hardware's on
/// macOS where it opens, else FFmpeg's own.
pub fn reader(path: &Path, decoding: Option<Decoding>) -> Result<Box<dyn Read>, String> {
    let open = |d: Decoding| -> Result<Box<dyn Read>, String> {
        Ok(match version()? {
            Version::V7 => Box::new(v7::decode::Reader::open(path, d)?),
            Version::V8 => Box::new(v8::decode::Reader::open(path, d)?),
            Version::V9 => Box::new(v9::decode::Reader::open(path, d)?),
        })
    };
    match decoding {
        Some(d) => open(d),
        None if cfg!(target_os = "macos") => {
            open(Decoding::Hardware).or_else(|_| open(Decoding::Software))
        }
        None => open(Decoding::Software),
    }
}

/// How long the video in the file at `path` is, in ticks.
pub fn length(path: &Path) -> Result<i64, String> {
    match version()? {
        Version::V7 => v7::decode::length(path),
        Version::V8 => v8::decode::length(path),
        Version::V9 => v9::decode::length(path),
    }
}

/// A video to write.
#[derive(Clone, Debug)]
pub struct Options {
    pub size: [u32; 2],
    /// Frames a second, as a ratio.
    pub fps: [i64; 2],
    pub codec: Codec,
    pub encoding: Encoding,
    /// Bits a second, in place of the encoder's quality target.
    pub bitrate: Option<u64>,
}

/// Frames into a video, one after another.
pub trait Write {
    /// The next frame: straight sRGB RGBA, rows top down.
    fn push(&mut self, rgba: &[u8]) -> Result<(), String>;
    /// Writes what is left and closes the file.
    fn finish(&mut self) -> Result<(), String>;
}

/// The FFmpeg version loaded, loading it the first time.
pub fn version() -> Result<Version, String> {
    // Errors only: encoders report what they do on every run.
    valle_ffmpeg::set_log_level(16);
    valle_ffmpeg::init().map(|r| r.version).map_err(|e| {
        format!(
            "FFmpeg 7, 8 or 9 is needed for video ({e}); on macOS `brew install ffmpeg`, on Linux \
             the distribution's ffmpeg package"
        )
    })
}

/// Whether the FFmpeg loaded has the encoder of `codec` that `encoding` names.
pub fn has(codec: Codec, encoding: Encoding) -> Result<bool, String> {
    Ok(match version()? {
        Version::V7 => v7::encode::has(codec, encoding),
        Version::V8 => v8::encode::has(codec, encoding),
        Version::V9 => v9::encode::has(codec, encoding),
    })
}

/// A video file at `path`, its container by its extension.
pub fn writer(path: &Path, options: &Options) -> Result<Box<dyn Write>, String> {
    let [w, h] = options.size;
    if options.codec != Codec::ProRes4444 && (w % 2 == 1 || h % 2 == 1) {
        return Err(format!(
            "H.264 and HEVC keep color at half size, so the frame is an even number of px each \
             way, not {w} × {h}"
        ));
    }
    let mov = (path.extension()).is_some_and(|e| e.eq_ignore_ascii_case("mov"));
    if options.codec == Codec::ProRes4444 && !mov {
        return Err("ProRes goes in a .mov".into());
    }
    Ok(match version()? {
        Version::V7 => Box::new(v7::encode::Writer::new(path, options)?),
        Version::V8 => Box::new(v8::encode::Writer::new(path, options)?),
        Version::V9 => Box::new(v9::encode::Writer::new(path, options)?),
    })
}
