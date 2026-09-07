//! Native host, delivery pipeline and sealed Skia executor for the Product Compositor.
//!
//! Preview, frame capture, storyboard and export all enter through [`host::NativeRenderer`]. The
//! executor accepts only admitted Engine plans, bindings and opaque resource objects; no Scene or
//! recording-language fallback is compiled into this crate.

#[cfg(feature = "native")]
mod assets;
#[cfg(any(feature = "native", feature = "motion"))]
pub mod executor;
#[cfg(any(feature = "native", feature = "motion"))]
pub mod host;

#[cfg(feature = "native")]
pub use assets::{AssetCache, default_cache_dir};
#[cfg(feature = "native")]
pub use valle_media::codec::hw_h264_available;
