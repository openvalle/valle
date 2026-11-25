//! The media code built against FFmpeg 7.

use valle_ffmpeg::backend::v7 as ff;

#[path = "../decode.rs"]
pub mod decode;
#[path = "../encode.rs"]
pub mod encode;
#[path = "../scenes.rs"]
pub mod scenes;
#[path = "../sound.rs"]
pub mod sound;
