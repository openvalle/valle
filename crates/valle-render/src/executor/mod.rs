//! Sealed Native execution boundary.
//!
//! Code below this module consumes only admitted Engine plans, frame bindings, opaque external
//! objects and a target transaction. Authoring documents, Motion artifacts and host scheduling are not valid
//! executor inputs.

pub mod skia;
