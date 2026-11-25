//! The media code built against FFmpeg 8.

use valle_ffmpeg::backend::v8 as ff;

#[path = "../decode.rs"]
pub mod decode;
#[path = "../encode.rs"]
pub mod encode;
#[path = "../scenes.rs"]
pub mod scenes;
#[path = "../sound.rs"]
pub mod sound;
