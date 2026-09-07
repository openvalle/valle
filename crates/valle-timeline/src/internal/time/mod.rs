//! Execution-time identities derived from the public exact-time kernel.

mod epoch;
mod sample;

pub use crate::{ExactRational, FrameRate, RationalRate, RationalTime, TimeError};
pub use epoch::{DiscontinuityIndex, MotionInstanceId, TimeMapSegment, TrackEpoch};
pub use sample::{FrameKey, SampleTime};
