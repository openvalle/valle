//! Pixel and audio data carriers without I/O or composition logic. Codec and transport modules own
//! platform resources.

pub mod audio;
pub mod gray;
pub mod rgba;

pub use audio::AudioBuffer;
pub use gray::Gray8Frame;
pub use rgba::RgbaFrame;
