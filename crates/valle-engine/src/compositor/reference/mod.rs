//! Scalar RGBA32F reference implementation.
//!
//! No Skia, CanvasKit, codec or platform color API is used here. Every conversion and composite
//! formula is visible Rust code so all executors can be tested against the same truth.

mod blend;
mod color;
mod composite;
mod effect;
mod executor;
mod output;
mod pixel;
mod transition;

pub use blend::*;
pub use color::*;
pub use composite::*;
pub(crate) use effect::*;
pub use executor::*;
pub use output::*;
pub use pixel::*;
pub use transition::*;
