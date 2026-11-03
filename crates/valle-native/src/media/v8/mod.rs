//! The media code built against FFmpeg 8.

use valle_ffmpeg::backend::v8 as ff;

#[path = "../decode.rs"]
pub mod decode;
#[path = "../encode.rs"]
pub mod encode;
#[path = "../sound.rs"]
pub mod sound;
