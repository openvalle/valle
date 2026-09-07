//! Public exact-time values accepted by Timeline authoring contracts.

mod rational;

pub use rational::{ExactRational, FrameRate, RationalRate, RationalTime, TimeError};
