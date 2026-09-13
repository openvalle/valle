use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, de};
use valle_timeline::internal::ContentDigest;

use super::canonical::canonical_bytes;
use super::manifest::validate_value;
use super::{
    DialectReport, ShaderAbi, ShaderInput, ShaderManifest, ShaderUniform, UniformType, UniformValue,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DiagnosticCode {
    ManifestParse,
    ManifestInvalid,
    InvalidUri,
    Identifier,
    DuplicateName,
    DialectViolation,
    OutputContract,
    BudgetExceeded,
    SourceEncoding,
    UniformMissing,
    UniformUnknown,
    UniformType,
    UniformRange,
    UniformDefault,
    LayerBounds,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShaderDiagnostic {
    pub code: DiagnosticCode,
    pub path: String,
    pub message: String,
}

impl ShaderDiagnostic {
    pub fn new(code: DiagnosticCode, path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code,
            path: path.into(),
            message: message.into(),
        }
    }
}

impl core::fmt::Display for ShaderDiagnostic {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            formatter,
            "{:?} at {}: {}",
            self.code, self.path, self.message
        )
    }
}

impl std::error::Error for ShaderDiagnostic {}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ShaderUri {
    pub digest: ContentDigest,
}

impl Serialize for ShaderUri {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for ShaderUri {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::from_str(&value).map_err(de::Error::custom)
    }
}

impl core::fmt::Display for ShaderUri {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(formatter, "shader://{}", self.digest.as_hex())
    }
}

impl FromStr for ShaderUri {
    type Err = ShaderDiagnostic;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let Some(rest) = value.strip_prefix("shader://") else {
            return Err(invalid_uri(value));
        };
        let digest = ContentDigest::from_hex(rest).map_err(|_| invalid_uri(value))?;
        if digest.as_hex() != rest {
            return Err(invalid_uri(value));
        }
        Ok(Self { digest })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct UniformBindings(pub BTreeMap<String, UniformValue>);

impl UniformBindings {
    pub fn empty() -> Self {
        Self(BTreeMap::new())
    }
}

#[derive(Debug, Clone)]
pub struct ShaderPackage {
    pub manifest: ShaderManifest,
    pub source: String,
    pub generated_sksl: String,
    pub abi: ShaderAbi,
    pub dialect_report: DialectReport,
    pub canonical_manifest: Vec<u8>,
    pub content_hash: ContentDigest,
    pub source_hash: ContentDigest,
    pub abi_hash: ContentDigest,
}

/// Portable author source and its contract. Generated backend code is never trusted input.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FrozenShader {
    manifest: ShaderManifest,
    source: String,
}

/// Backend-neutral runtime record exported by the single Rust admission path.
///
/// Web receives this record after WASM has admitted the exact manifest/source bytes. It never
/// parses a manifest or lowers the Valle shader dialect itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShaderRuntimeRecord {
    pub uri: String,
    pub content_hash: ContentDigest,
    pub abi_hash: ContentDigest,
    pub generated_sksl: String,
    pub inputs: Vec<ShaderInput>,
    pub uniforms: Vec<ShaderUniform>,
    pub scalar_count: u32,
}

impl ShaderPackage {
    pub fn work_per_pixel(&self) -> valle_draw::requirements::ShaderWork {
        valle_draw::requirements::ShaderWork {
            // Account for helper dispatch and the fixed finite/color/alpha output adapter.
            operations: (self.dialect_report.operations_per_pixel
                + self.dialect_report.calls_per_pixel
                + 32) as u64,
            samples: self.dialect_report.samples_per_pixel as u64,
        }
    }

    pub fn from_frozen(bytes: &[u8]) -> Result<Self, ShaderDiagnostic> {
        let frozen: FrozenShader = serde_json::from_slice(bytes).map_err(|error| {
            ShaderDiagnostic::new(DiagnosticCode::ManifestParse, "shader", error.to_string())
        })?;
        Self::compile(frozen.manifest, frozen.source.as_bytes())
    }

    pub fn frozen_bytes(&self) -> Result<Vec<u8>, ShaderDiagnostic> {
        canonical_bytes(&FrozenShader {
            manifest: self.manifest.clone(),
            source: self.source.clone(),
        })
    }

    pub fn admit(manifest_bytes: &[u8], source_bytes: &[u8]) -> Result<Self, ShaderDiagnostic> {
        let manifest = ShaderManifest::parse(manifest_bytes)?;
        Self::compile(manifest, source_bytes)
    }

    /// Admit a package while leaving byte discovery to the host. The manifest is parsed exactly
    /// once here; the resolver only receives the already-validated relative entry name.
    pub fn admit_with_resolver<F>(
        manifest_bytes: &[u8],
        resolver: F,
    ) -> Result<Self, ShaderDiagnostic>
    where
        F: FnOnce(&str) -> Result<Vec<u8>, String>,
    {
        let manifest = ShaderManifest::parse(manifest_bytes)?;
        manifest.validate_shape()?;
        let source_bytes = resolver(&manifest.entry).map_err(|message| {
            ShaderDiagnostic::new(DiagnosticCode::ManifestInvalid, "entry", message)
        })?;
        Self::compile(manifest, &source_bytes)
    }

    pub fn compile(
        manifest: ShaderManifest,
        source_bytes: &[u8],
    ) -> Result<Self, ShaderDiagnostic> {
        manifest.validate_shape()?;
        let source = core::str::from_utf8(source_bytes).map_err(|error| {
            ShaderDiagnostic::new(
                DiagnosticCode::SourceEncoding,
                "source",
                format!("shader source is not UTF-8: {error}"),
            )
        })?;
        let source_hash = ContentDigest::of_bytes(source_bytes);
        let abi = manifest.abi()?;
        let abi_hash = abi.digest()?;
        let (dialect_report, generated_sksl) = super::dialect::compile_dialect(&manifest, source)?;
        let canonical_manifest = manifest.canonical_bytes()?;
        let package_bytes = canonical_bytes(&FrozenShader {
            manifest: manifest.clone(),
            source: source.to_owned(),
        })?;
        let content_hash = ContentDigest::of_bytes(&package_bytes);
        Ok(Self {
            manifest,
            source: source.to_owned(),
            generated_sksl,
            abi,
            dialect_report,
            canonical_manifest,
            content_hash,
            source_hash,
            abi_hash,
        })
    }

    pub fn runtime_record(&self) -> ShaderRuntimeRecord {
        ShaderRuntimeRecord {
            uri: self.uri().to_string(),
            content_hash: self.content_hash,
            abi_hash: self.abi_hash,
            generated_sksl: self.generated_sksl.clone(),
            inputs: self.manifest.inputs.clone(),
            uniforms: self.manifest.uniforms.clone(),
            scalar_count: self.abi.scalar_count,
        }
    }

    pub fn uri(&self) -> ShaderUri {
        ShaderUri {
            digest: self.content_hash,
        }
    }

    pub fn validate_uri(&self, uri: &ShaderUri) -> Result<(), ShaderDiagnostic> {
        if uri.digest == self.content_hash {
            Ok(())
        } else {
            Err(ShaderDiagnostic::new(
                DiagnosticCode::InvalidUri,
                "source",
                format!("{uri} does not identify package {}", self.uri()),
            ))
        }
    }

    pub fn validate_layer_pixels(&self, width: u32, height: u32) -> Result<u64, ShaderDiagnostic> {
        let [left, top, right, bottom] = self.manifest.output.padding.map(u64::from);
        let pixels = (u64::from(width) + left + right)
            .checked_mul(u64::from(height) + top + bottom)
            .ok_or_else(|| {
                ShaderDiagnostic::new(
                    DiagnosticCode::LayerBounds,
                    "bounds",
                    "layer pixel area overflow",
                )
            })?;
        if width == 0 || height == 0 || pixels > super::MAX_LAYER_PIXELS {
            return Err(ShaderDiagnostic::new(
                DiagnosticCode::LayerBounds,
                "bounds",
                format!(
                    "layer {width}x{height} has {pixels} pixels; expected 1..={} with non-zero axes",
                    super::MAX_LAYER_PIXELS
                ),
            ));
        }
        Ok(pixels)
    }

    pub fn pack_uniforms(
        &self,
        resolution: [f32; 2],
        input_dimensions: &BTreeMap<String, [u32; 2]>,
        bindings: &UniformBindings,
    ) -> Result<Vec<u8>, ShaderDiagnostic> {
        if resolution
            .iter()
            .any(|value| !value.is_finite() || *value <= 0.0)
        {
            return Err(ShaderDiagnostic::new(
                DiagnosticCode::LayerBounds,
                "resolution",
                "resolution must contain two positive finite values",
            ));
        }
        let declared = self
            .manifest
            .uniforms
            .iter()
            .map(|uniform| uniform.name.as_str())
            .collect::<BTreeSet<_>>();
        if let Some(unknown) = bindings
            .0
            .keys()
            .find(|name| !declared.contains(name.as_str()))
        {
            return Err(ShaderDiagnostic::new(
                DiagnosticCode::UniformUnknown,
                format!("uniforms.{unknown}"),
                "uniform is not declared by the shader manifest",
            ));
        }

        let mut bytes = Vec::with_capacity(self.abi.scalar_count as usize * 4);
        for value in resolution {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        if input_dimensions
            .keys()
            .any(|name| !self.manifest.inputs.iter().any(|input| &input.name == name))
        {
            return Err(ShaderDiagnostic::new(
                DiagnosticCode::UniformUnknown,
                "inputDimensions",
                "dimensions reference an undeclared texture",
            ));
        }
        for input in &self.manifest.inputs {
            let size = input_dimensions
                .get(&input.name)
                .copied()
                .or_else(|| (!input.required).then_some([1, 1]))
                .ok_or_else(|| {
                    ShaderDiagnostic::new(
                        DiagnosticCode::UniformMissing,
                        "inputDimensions",
                        format!("missing dimensions for {}", input.name),
                    )
                })?;
            if size.contains(&0) {
                return Err(ShaderDiagnostic::new(
                    DiagnosticCode::UniformRange,
                    "inputDimensions",
                    "texture dimensions must be positive",
                ));
            }
            for component in size {
                bytes.extend_from_slice(&(component as f32).to_le_bytes());
            }
        }
        for uniform in &self.manifest.uniforms {
            let value = bindings
                .0
                .get(&uniform.name)
                .or(uniform.default.as_ref())
                .ok_or_else(|| {
                    ShaderDiagnostic::new(
                        DiagnosticCode::UniformMissing,
                        format!("uniforms.{}", uniform.name),
                        "required shader uniform is missing",
                    )
                })?;
            if value.uniform_type() != uniform.uniform_type {
                return Err(ShaderDiagnostic::new(
                    DiagnosticCode::UniformType,
                    format!("uniforms.{}", uniform.name),
                    format!(
                        "expected {:?}, got {:?}",
                        uniform.uniform_type,
                        value.uniform_type()
                    ),
                ));
            }
            validate_value(
                value,
                uniform.min,
                uniform.max,
                &format!("uniforms.{}", uniform.name),
            )?;
            if let UniformValue::Color(rgba) = value {
                let straight = valle_draw::program::decode_srgb_straight(*rgba);
                pack_value(
                    &mut bytes,
                    &UniformValue::Color(straight),
                    uniform.uniform_type,
                );
            } else {
                pack_value(&mut bytes, value, uniform.uniform_type);
            }
        }
        debug_assert_eq!(bytes.len(), self.abi.scalar_count as usize * 4);
        Ok(bytes)
    }

    pub fn lock_record(&self) -> Result<Vec<u8>, ShaderDiagnostic> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct LockRecord<'a> {
            uri: String,
            content_hash: &'a ContentDigest,
            abi_hash: &'a ContentDigest,
            source_hash: &'a ContentDigest,
        }
        canonical_bytes(&LockRecord {
            uri: self.uri().to_string(),
            content_hash: &self.content_hash,
            abi_hash: &self.abi_hash,
            source_hash: &self.source_hash,
        })
    }
}

fn pack_value(bytes: &mut Vec<u8>, value: &UniformValue, uniform_type: UniformType) {
    match (value, uniform_type) {
        (UniformValue::Float(value), UniformType::Float) => {
            bytes.extend_from_slice(&value.to_le_bytes())
        }
        (UniformValue::Float2(values), UniformType::Float2) => {
            for value in values {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        (UniformValue::Float3(values), UniformType::Float3) => {
            for value in values {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        (UniformValue::Float4(values), UniformType::Float4) => {
            for value in values {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        (UniformValue::Float2x2(values), UniformType::Float2x2) => {
            for value in values {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        (UniformValue::Float3x3(values), UniformType::Float3x3) => {
            for value in values {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        (UniformValue::Float4x4(values), UniformType::Float4x4) => {
            for value in values {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        (UniformValue::Color(values), UniformType::Color) => {
            for value in values {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        (UniformValue::Bool(value), UniformType::Bool) => {
            bytes.extend_from_slice(&(if *value { 1.0_f32 } else { 0.0_f32 }).to_le_bytes())
        }
        _ => unreachable!("type checked before packing"),
    }
}

fn invalid_uri(value: &str) -> ShaderDiagnostic {
    ShaderDiagnostic::new(
        DiagnosticCode::InvalidUri,
        "source",
        format!("invalid shader URI `{value}`; expected shader://<content-hash>"),
    )
}
