//! Versioned wire DTOs.
//!
//! These types describe JSON shape only. Domain construction, local invariants,
//! resource admission, and execution-profile checks live outside this module.

pub mod edit;
pub mod timeline;

pub use edit::*;
pub use timeline::*;
