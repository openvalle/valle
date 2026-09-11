use valle_ffmpeg::backend::v7 as ff;
#[path = "../implementation/alpha.rs"]
pub(crate) mod alpha;
#[path = "../implementation/audio.rs"]
pub(crate) mod audio;
#[path = "../implementation/decode.rs"]
pub(crate) mod decode;
#[path = "../implementation/encode.rs"]
pub(crate) mod encode;
#[path = "../implementation/ffi.rs"]
pub(crate) mod ffi;
#[path = "../implementation/flac.rs"]
pub(crate) mod flac;
#[cfg(feature = "tool-segment")]
#[path = "../implementation/mask.rs"]
pub(crate) mod mask;
#[path = "../implementation/muxer.rs"]
pub(crate) mod muxer;
#[path = "../implementation/shared_frame.rs"]
pub(crate) mod shared_frame;
#[cfg(any(
    feature = "tool-inpaint",
    feature = "tool-interpolate",
    feature = "tool-matte",
    feature = "tool-segment",
    feature = "tool-shots",
    feature = "tool-upscale"
))]
#[path = "../implementation/video_sequence.rs"]
pub(crate) mod video_sequence;
