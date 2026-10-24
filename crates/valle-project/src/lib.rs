//! File-backed projects, source snapshots, and the cross-project asset library.

pub use valle_timeline::internal::ContentDigest;

#[cfg(feature = "host")]
pub mod files;
#[cfg(feature = "host")]
pub mod folder;

#[cfg(feature = "host")]
pub mod assets;

#[cfg(feature = "host")]
mod fs;
#[cfg(feature = "host")]
use fs::sync_directory;
