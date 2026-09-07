//! Backend-independent compositor semantics.
//!
//! [`reference`] is deliberately slow RGBA32F code. It is the executable specification against
//! which CPU F16, GPU and Web lowerings are compared; it is never a production fast path.

mod bind;
pub mod delivery;
pub mod inspect;

pub use bind::{
    BoundExternalObject, BoundExternalObjects, ExternalBindError, ExternalObject,
    ExternalObjectTable, bind_external_objects,
};

pub mod glass;
pub mod graph;
pub mod lower;
pub mod reference;
