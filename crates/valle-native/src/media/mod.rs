//! Video files through FFmpeg, loaded at run time: version 7, 8 or 9, whichever the machine has.
//! The code is written once (`encode.rs`, `decode.rs`) and built against each version's API; the
//! one loaded picks which runs.
#![allow(clippy::duplicate_mod)]

use std::ffi::c_void;
use std::path::Path;

use valle_ffmpeg::Version;

mod orientation;
mod textures;
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

/// Stream metadata, without decoding the whole file.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Info {
    /// Selected video stream's duration, or audio's for audio-only files, in ticks.
    pub duration: Option<i64>,
    /// Display dimensions, after applying the stream's orientation.
    pub size: Option<[u32; 2]>,
    /// Average video frame rate, as numerator and denominator; unknown for some sources.
    pub fps: Option<[u32; 2]>,
    pub audio: bool,
    /// HDR transfer function of the selected video stream, when tagged: PQ or HLG.
    pub hdr: Option<&'static str>,
}

pub fn probe(path: &Path) -> Result<Info, String> {
    match version()? {
        Version::V7 => v7::decode::probe(path),
        Version::V8 => v8::decode::probe(path),
        Version::V9 => v9::decode::probe(path),
    }
}

/// Sound's samples a second: every file's is resampled to it, and the mix runs at it.
pub const RATE: u32 = 48_000;

/// The sound of the file at `path`: interleaved stereo 32-bit float samples at `RATE`; none
/// when it has no audio.
pub fn sound(path: &Path) -> Result<Option<Vec<f32>>, String> {
    match version()? {
        Version::V7 => v7::sound::decode(path),
        Version::V8 => v8::sound::decode(path),
        Version::V9 => v9::sound::decode(path),
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
    /// Whether it has sound, which `Write::sound` gives.
    pub sound: bool,
    /// Whether a GPU draws the frames into the encoder's own pixel buffers (`Write::surface`)
    /// where the encoder has them: VideoToolbox's H.264 and HEVC on macOS.
    pub surfaces: bool,
}

pub use textures::{Planes, Textures};

/// A frame in the encoder's own memory, for a GPU to draw into and `Write::send` to encode: a
/// VideoToolbox pixel buffer of the video's size, NV12 at limited range.
pub struct Surface {
    /// The `CVPixelBufferRef`.
    pub buffer: *mut c_void,
    /// The FFmpeg `AVFrame` that holds it, and how it is freed if it is not sent.
    frame: *mut c_void,
    free: unsafe fn(*mut c_void),
}

impl Drop for Surface {
    fn drop(&mut self) {
        if !self.frame.is_null() {
            // SAFETY: the frame was allocated by the FFmpeg that `free` belongs to, and is
            // freed once.
            unsafe { (self.free)(self.frame) };
        }
    }
}

/// Frames into a video, one after another.
pub trait Write {
    /// The next frame: straight sRGB RGBA, rows top down.
    fn push(&mut self, rgba: &[u8]) -> Result<(), String>;
    /// More of its sound: interleaved stereo 32-bit float samples at `RATE`.
    fn sound(&mut self, samples: &[f32]) -> Result<(), String>;
    /// Writes what is left and closes the file.
    fn finish(&mut self) -> Result<(), String>;
    /// A frame of the encoder's own to draw the next into, where it was opened on them
    /// (`Options::surfaces`); `None` where frames go through `push`.
    fn surface(&mut self) -> Result<Option<Surface>, String>;
    /// The next frame: a surface drawn, once the GPU is done with it.
    fn send(&mut self, surface: Surface) -> Result<(), String>;
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
    if w == 0 || h == 0 || w > i32::MAX as u32 || h > i32::MAX as u32 {
        return Err("video dimensions must be positive 32-bit integers".into());
    }
    if options
        .fps
        .iter()
        .any(|&n| n <= 0 || n > i64::from(i32::MAX))
    {
        return Err("video fps must be a ratio of positive 32-bit integers".into());
    }
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
    let open = |o: &Options| -> Result<Box<dyn Write>, String> {
        Ok(match version()? {
            Version::V7 => Box::new(v7::encode::Writer::new(path, o)?),
            Version::V8 => Box::new(v8::encode::Writer::new(path, o)?),
            Version::V9 => Box::new(v9::encode::Writer::new(path, o)?),
        })
    };
    // An encoder that does not open on its own pixel buffers takes frames from memory.
    open(options).or_else(|e| match options.surfaces {
        true => open(&Options {
            surfaces: false,
            ..options.clone()
        }),
        false => Err(e),
    })
}
