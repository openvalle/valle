//! Backend-neutral contract for controlled `ShaderLayer` packages.
//!
//! This crate deliberately owns no filesystem or renderer. A host resolves bytes, then all
//! producers and consumers call [`ShaderPackage::admit`] so manifest validation, hashes, ABI
//! order, dialect restrictions and diagnostics cannot drift between Native and Web.

mod canonical;
mod dialect;
mod manifest;
mod package;
mod registry;

pub use crate::ContentDigest;
pub use dialect::{DialectReport, lower_to_sksl, validate_dialect};
pub use manifest::{
    ALPHA_MODE, AbiChild, AbiUniform, BudgetClass, COLOR_SPACE, DIALECT_ID, DIALECT_VERSION,
    InputSampling, MAX_LAYER_PIXELS, MAX_SAMPLES_PER_PIXEL, MAX_SOURCE_BYTES, MAX_TEXTURE_INPUTS,
    MAX_UNIFORM_SCALARS, OutputContract, SHADER_MANIFEST_VERSION, ShaderAbi, ShaderInput,
    ShaderManifest, ShaderUniform, UniformType, UniformValue,
};
pub use package::{
    DiagnosticCode, ShaderDiagnostic, ShaderPackage, ShaderRuntimeRecord, ShaderUri,
    UniformBindings,
};
pub use registry::{ShaderRegistry, ShaderRegistryError};
