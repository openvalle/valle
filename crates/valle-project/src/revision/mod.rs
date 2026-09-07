//! Timeline project revisions.
//!
//! A revision persists one immutable sparse Timeline as its source of
//! truth. The compiled Timeline is an internal derived view, never an
//! additional authoring contract. Resource fulfillment belongs to a separately
//! admitted fixed package and is absent from Project history. Authoring commands
//! and deltas are deliberately absent: every mutation selects or constructs the
//! complete next Timeline document before the project HEAD is published.

#[cfg(feature = "host")]
mod api;
#[cfg(feature = "host")]
mod clock;
#[cfg(feature = "host")]
mod crash;
mod model;
#[cfg(feature = "host")]
mod session;
#[cfg(feature = "host")]
mod store;

#[cfg(feature = "host")]
pub use api::EditTimelineServiceError;
pub use model::*;
#[cfg(feature = "host")]
pub use session::*;
#[cfg(feature = "host")]
pub use store::ProjectStore;
