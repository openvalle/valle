//! Codec I/O contracts for decoding, encoding, muxing, and probing. The optional libav feature
//! supplies in-process implementations; transport traits remain available without native codec
//! dependencies.

use crate::frame::RgbaFrame;

/// Encoding backend contract.
pub trait Encoder {
    /// Encode one straight-alpha RGBA8 frame.
    fn encode_frame(&mut self, frame: &RgbaFrame) -> anyhow::Result<()>;
    /// Flush and write the trailer; safe to call repeatedly.
    fn finish(&mut self) -> anyhow::Result<()>;
}

/// Synchronous frame and sample source contracts, available without libav.
pub use crate::transport::{AudioSource, SourceFrame, SourceMeta, VideoSource};

pub mod image;
/// Optional VALLE_PERF timing slots available independently of libav.
pub mod perf;
pub mod wav;

pub use image::{read_gray8_png, read_rgba_png, write_gray8_png, write_rgba_png};
pub use wav::{FloatWavWriter, validate_float_wav};

#[cfg(feature = "libav")]
pub mod alpha;
#[cfg(feature = "libav")]
pub mod audio;
#[cfg(feature = "libav")]
pub mod decode;
#[cfg(feature = "libav")]
pub mod encode;
#[cfg(feature = "libav")]
pub mod ffi;
#[cfg(feature = "libav")]
pub mod flac;
#[cfg(feature = "libav")]
pub mod muxer;

#[cfg(feature = "libav")]
pub use crate::transport::{
    SharedFrameBackend, SharedVideoFrame, SharedVideoFrameHandle, SharedVideoFramePool,
    VideoFrameTransport,
};
#[cfg(feature = "libav")]
pub use alpha::TransparentVideoMuxer;
#[cfg(feature = "libav")]
pub use audio::{LibavAudioSource, LibavAudioStream, decode_audio_mono_f32};
#[cfg(feature = "libav")]
pub use decode::{
    AvProbe, DecodedGpuFrame, LibavVideoSource, decode_rgba_frames, probe_av, probe_dimensions,
};
#[cfg(feature = "libav")]
pub use encode::Mp4Encoder;
#[cfg(feature = "libav")]
pub use ffi::YuvMatrix;
#[cfg(feature = "libav")]
pub use flac::{
    FLAC_BITS_PER_SAMPLE, FLAC_QUANTIZATION_POLICY, FlacPcm24Writer, validate_flac_pcm24,
};
#[cfg(feature = "libav")]
pub use muxer::{AudioMuxer, Muxer, RgbaToYuv, YuvFrame, hw_h264_available};
