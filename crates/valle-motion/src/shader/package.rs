use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, de};
use valle_timeline::internal::ContentDigest;

use super::canonical::canonical_bytes;
use super::manifest::{validate_package_name, validate_value};
use super::{
    DialectReport, ShaderAbi, ShaderInput, ShaderManifest, ShaderUniform, UniformType,
    UniformValue, lower_to_sksl, validate_dialect,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DiagnosticCode {
    ManifestParse,
    ManifestVersion,
    ManifestInvalid,
    InvalidUri,
    Identifier,
    DuplicateName,
    DialectVersion,
    DialectViolation,
    OutputContract,
    BudgetExceeded,
    SourceEncoding,
    SourceDigest,
    AbiDigest,
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
    pub name: String,
    pub version: u32,
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
        write!(formatter, "shader://{}@{}", self.name, self.version)
    }
}

impl FromStr for ShaderUri {
    type Err = ShaderDiagnostic;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let Some(rest) = value.strip_prefix("shader://") else {
            return Err(invalid_uri(value));
        };
        let Some((name, version_text)) = rest.rsplit_once('@') else {
            return Err(invalid_uri(value));
        };
        validate_package_name(name, "source").map_err(|_| invalid_uri(value))?;
        let version = version_text
            .parse::<u32>()
            .map_err(|_| invalid_uri(value))?;
        if version == 0 || version.to_string() != version_text {
            return Err(invalid_uri(value));
        }
        Ok(Self {
            name: name.to_owned(),
            version,
        })
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
    pub fn admit(manifest_bytes: &[u8], source_bytes: &[u8]) -> Result<Self, ShaderDiagnostic> {
        let manifest = ShaderManifest::parse(manifest_bytes)?;
        Self::admit_manifest(manifest, source_bytes)
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
        Self::admit_manifest(manifest, &source_bytes)
    }

    fn admit_manifest(
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
        let actual_source_digest = ContentDigest::of_bytes(source_bytes);
        if actual_source_digest != manifest.source_digest {
            return Err(ShaderDiagnostic::new(
                DiagnosticCode::SourceDigest,
                "sourceDigest",
                format!(
                    "declared {} does not match source {}",
                    manifest.source_digest, actual_source_digest
                ),
            ));
        }
        let abi = manifest.abi()?;
        let actual_abi_digest = abi.digest()?;
        if actual_abi_digest != manifest.abi_digest {
            return Err(ShaderDiagnostic::new(
                DiagnosticCode::AbiDigest,
                "abiDigest",
                format!(
                    "declared {} does not match ABI {}",
                    manifest.abi_digest, actual_abi_digest
                ),
            ));
        }
        let dialect_report = validate_dialect(&manifest, source)?;
        let generated_sksl = lower_to_sksl(&manifest, source)?;
        let canonical_manifest = manifest.canonical_bytes()?;
        let mut package_bytes = canonical_manifest.clone();
        package_bytes.push(b'\n');
        package_bytes.extend_from_slice(source_bytes);
        let content_hash = ContentDigest::of_bytes(&package_bytes);
        Ok(Self {
            manifest,
            source: source.to_owned(),
            generated_sksl,
            abi,
            dialect_report,
            canonical_manifest,
            content_hash,
        })
    }

    pub fn runtime_record(&self) -> ShaderRuntimeRecord {
        ShaderRuntimeRecord {
            uri: self.uri().to_string(),
            content_hash: self.content_hash,
            abi_hash: self.manifest.abi_digest,
            generated_sksl: self.generated_sksl.clone(),
            inputs: self.manifest.inputs.clone(),
            uniforms: self.manifest.uniforms.clone(),
            scalar_count: self.abi.scalar_count,
        }
    }

    pub fn uri(&self) -> ShaderUri {
        ShaderUri {
            name: self.manifest.name.clone(),
            version: self.manifest.version,
        }
    }

    pub fn validate_uri(&self, uri: &ShaderUri) -> Result<(), ShaderDiagnostic> {
        if uri.name == self.manifest.name && uri.version == self.manifest.version {
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
        let pixels = u64::from(width)
            .checked_mul(u64::from(height))
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
            pack_value(&mut bytes, value, uniform.uniform_type);
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
            abi_hash: &self.manifest.abi_digest,
            source_hash: &self.manifest.source_digest,
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
        format!("invalid shader URI `{value}`; expected shader://lower-kebab@positive-version"),
    )
}
