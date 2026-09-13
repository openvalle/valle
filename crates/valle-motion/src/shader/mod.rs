//! Backend-neutral contract for controlled `ShaderLayer` packages.
//!
//! This crate deliberately owns no filesystem or renderer. A host resolves bytes, then all
//! producers and consumers call [`ShaderPackage::admit`] so manifest validation, hashes, ABI
//! order, dialect restrictions and diagnostics cannot drift between Native and Web.

mod canonical;
mod data;
mod dialect;
mod manifest;
mod package;
mod registry;

pub use crate::ContentDigest;
pub use dialect::{DialectReport, SHADER_LIMITS, ShaderLimits, lower_to_sksl, validate_dialect};
pub use manifest::{
    ALPHA_MODE, AbiChild, AbiUniform, BudgetClass, COLOR_SPACE, DIALECT_ID, InputKind,
    InputSampling, InputWrap, MAX_LAYER_PIXELS, MAX_SAMPLES_PER_PIXEL, MAX_SOURCE_BYTES,
    MAX_TEXTURE_INPUTS, MAX_UNIFORM_SCALARS, OutputContract, ShaderAbi, ShaderInput,
    ShaderManifest, ShaderUniform, UniformType, UniformValue,
};
pub use package::{
    DiagnosticCode, ShaderDiagnostic, ShaderPackage, ShaderRuntimeRecord, ShaderUri,
    UniformBindings,
};
pub use registry::{ShaderRegistry, ShaderRegistryError};

pub use data::{
    DATA_TEXTURE_BYTES_PER_PIXEL, MAX_DATA_TEXTURE_BYTES, data_texture_storage_bytes,
    decode_data_texture,
};
