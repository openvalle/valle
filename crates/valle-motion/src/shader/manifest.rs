use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::canonical::canonical_bytes;
use crate::ContentDigest;

use super::{DiagnosticCode, ShaderDiagnostic};

pub const SHADER_MANIFEST_VERSION: u32 = 1;
pub const DIALECT_ID: &str = "valle-sksl";
pub const DIALECT_VERSION: u32 = 1;
pub const COLOR_SPACE: &str = "srgb";
pub const ALPHA_MODE: &str = "premultiplied";

// Bound shader source size, sampling, and local surface allocation. A local ShaderLayer must not
// silently allocate a full-canvas surface.
pub const MAX_SOURCE_BYTES: usize = 4 * 1024;
pub const MAX_TEXTURE_INPUTS: usize = 3; // plus the implicit `content` child = 4 total
pub const MAX_SAMPLES_PER_PIXEL: usize = 8;
pub const MAX_UNIFORM_SCALARS: usize = 64;
pub const MAX_LAYER_PIXELS: u64 = 1920 * 1080;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BudgetClass {
    Local,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InputSampling {
    Nearest,
    Linear,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UniformType {
    Float,
    Float2,
    Color,
    Bool,
}

impl UniformType {
    pub fn scalar_width(self) -> usize {
        match self {
            Self::Float | Self::Bool => 1,
            Self::Float2 => 2,
            Self::Color => 4,
        }
    }

    pub fn sksl_name(self) -> &'static str {
        match self {
            Self::Float => "float",
            Self::Float2 => "float2",
            Self::Color => "float4",
            // RuntimeEffect rejects `uniform bool`. The ABI stores bool as f32 0/1 and lowering
            // rewrites the author identifier to a comparison against this private float slot.
            Self::Bool => "float",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum UniformValue {
    Float(f32),
    Float2([f32; 2]),
    Color([f32; 4]),
    Bool(bool),
}

impl UniformValue {
    pub fn uniform_type(&self) -> UniformType {
        match self {
            Self::Float(_) => UniformType::Float,
            Self::Float2(_) => UniformType::Float2,
            Self::Color(_) => UniformType::Color,
            Self::Bool(_) => UniformType::Bool,
        }
    }

    pub(crate) fn numbers(&self) -> &[f32] {
        match self {
            Self::Float(value) => core::slice::from_ref(value),
            Self::Float2(values) => values,
            Self::Color(values) => values,
            Self::Bool(_) => &[],
        }
    }
}

impl ShaderUniform {
    /// Validate one concrete author/runtime value against this manifest slot.
    /// Dynamic Artifact bindings call this after frame evaluation; static bindings call it again
    /// during package-backed Artifact admission so compiler and renderer cannot disagree.
    pub fn validate_value(&self, value: &UniformValue) -> Result<(), ShaderDiagnostic> {
        if value.uniform_type() != self.uniform_type {
            return Err(ShaderDiagnostic::new(
                DiagnosticCode::UniformType,
                format!("uniforms.{}", self.name),
                format!(
                    "expected {:?}, got {:?}",
                    self.uniform_type,
                    value.uniform_type()
                ),
            ));
        }
        validate_value(
            value,
            self.min,
            self.max,
            &format!("uniforms.{}", self.name),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShaderInput {
    pub name: String,
    pub required: bool,
    pub sampling: InputSampling,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShaderUniform {
    pub name: String,
    #[serde(rename = "type")]
    pub uniform_type: UniformType,
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<UniformValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OutputContract {
    pub color_space: String,
    pub alpha_mode: String,
    pub allow_transparent: bool,
}

impl Default for OutputContract {
    fn default() -> Self {
        Self {
            color_space: COLOR_SPACE.into(),
            alpha_mode: ALPHA_MODE.into(),
            allow_transparent: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShaderManifest {
    pub manifest_version: u32,
    pub name: String,
    pub version: u32,
    pub dialect: String,
    pub dialect_version: u32,
    pub entry: String,
    #[serde(default)]
    pub inputs: Vec<ShaderInput>,
    #[serde(default)]
    pub uniforms: Vec<ShaderUniform>,
    pub output: OutputContract,
    pub budget: BudgetClass,
    pub source_digest: ContentDigest,
    pub abi_digest: ContentDigest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AbiChild {
    pub name: String,
    pub required: bool,
    pub sampling: InputSampling,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AbiUniform {
    pub name: String,
    #[serde(rename = "type")]
    pub uniform_type: UniformType,
    pub scalar_offset: u32,
    pub scalar_width: u32,
    pub system: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShaderAbi {
    pub children: Vec<AbiChild>,
    pub uniforms: Vec<AbiUniform>,
    pub scalar_count: u32,
    pub color_space: String,
    pub alpha_mode: String,
    pub allow_transparent: bool,
}

impl ShaderAbi {
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, ShaderDiagnostic> {
        canonical_bytes(self)
    }

    pub fn digest(&self) -> Result<ContentDigest, ShaderDiagnostic> {
        Ok(ContentDigest::of_bytes(&self.canonical_bytes()?))
    }
}

impl ShaderManifest {
    /// Fill the content-derived fields for a newly authored manifest. The returned manifest still
    /// has to pass [`super::ShaderPackage::admit`]; sealing is a deterministic authoring helper,
    /// not a trusted bypass.
    pub fn seal(mut self, source: &[u8]) -> Result<Self, ShaderDiagnostic> {
        self.validate_shape()?;
        let source_text = core::str::from_utf8(source).map_err(|error| {
            ShaderDiagnostic::new(
                DiagnosticCode::SourceEncoding,
                "source",
                format!("shader source is not UTF-8: {error}"),
            )
        })?;
        super::validate_dialect(&self, source_text)?;
        self.source_digest = ContentDigest::of_bytes(source);
        self.abi_digest = self.abi()?.digest()?;
        Ok(self)
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, ShaderDiagnostic> {
        serde_json::from_slice(bytes).map_err(|error| {
            ShaderDiagnostic::new(
                DiagnosticCode::ManifestParse,
                "manifest",
                format!("invalid shader manifest JSON: {error}"),
            )
        })
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, ShaderDiagnostic> {
        self.validate_shape()?;
        canonical_bytes(self)
    }

    pub fn abi(&self) -> Result<ShaderAbi, ShaderDiagnostic> {
        self.validate_shape()?;
        let mut children = vec![AbiChild {
            name: "content".into(),
            required: true,
            sampling: InputSampling::Linear,
        }];
        children.extend(self.inputs.iter().map(|input| AbiChild {
            name: input.name.clone(),
            required: input.required,
            sampling: input.sampling,
        }));

        let mut scalar_offset = 0_u32;
        let mut uniforms = vec![AbiUniform {
            name: "resolution".into(),
            uniform_type: UniformType::Float2,
            scalar_offset,
            scalar_width: 2,
            system: true,
        }];
        scalar_offset += 2;
        for uniform in &self.uniforms {
            let width = uniform.uniform_type.scalar_width() as u32;
            uniforms.push(AbiUniform {
                name: uniform.name.clone(),
                uniform_type: uniform.uniform_type,
                scalar_offset,
                scalar_width: width,
                system: false,
            });
            scalar_offset += width;
        }
        Ok(ShaderAbi {
            children,
            uniforms,
            scalar_count: scalar_offset,
            color_space: self.output.color_space.clone(),
            alpha_mode: self.output.alpha_mode.clone(),
            allow_transparent: self.output.allow_transparent,
        })
    }

    pub fn validate_shape(&self) -> Result<(), ShaderDiagnostic> {
        if self.manifest_version != SHADER_MANIFEST_VERSION {
            return Err(ShaderDiagnostic::new(
                DiagnosticCode::ManifestVersion,
                "manifestVersion",
                format!(
                    "shader manifest version {} does not match {SHADER_MANIFEST_VERSION}",
                    self.manifest_version
                ),
            ));
        }
        if self.version == 0 {
            return Err(ShaderDiagnostic::new(
                DiagnosticCode::ManifestInvalid,
                "version",
                "shader package version must be positive",
            ));
        }
        validate_package_name(&self.name, "name")?;
        if self.dialect != DIALECT_ID || self.dialect_version != DIALECT_VERSION {
            return Err(ShaderDiagnostic::new(
                DiagnosticCode::DialectVersion,
                "dialect",
                format!("only {DIALECT_ID}@{DIALECT_VERSION} is supported"),
            ));
        }
        if self.entry != "shader.vsksl" {
            return Err(ShaderDiagnostic::new(
                DiagnosticCode::ManifestInvalid,
                "entry",
                "v1 entry must be the package-relative path shader.vsksl",
            ));
        }
        if self.output.color_space != COLOR_SPACE || self.output.alpha_mode != ALPHA_MODE {
            return Err(ShaderDiagnostic::new(
                DiagnosticCode::OutputContract,
                "output",
                format!("v1 output must be {COLOR_SPACE}/{ALPHA_MODE}"),
            ));
        }
        if self.inputs.len() > MAX_TEXTURE_INPUTS {
            return Err(ShaderDiagnostic::new(
                DiagnosticCode::BudgetExceeded,
                "inputs",
                format!(
                    "{} declared textures exceed the v1 limit {MAX_TEXTURE_INPUTS}",
                    self.inputs.len()
                ),
            ));
        }

        let mut names = BTreeSet::new();
        names.insert("content".to_owned());
        names.insert("resolution".to_owned());
        for (index, input) in self.inputs.iter().enumerate() {
            validate_identifier(&input.name, &format!("inputs[{index}].name"))?;
            if !names.insert(input.name.clone()) {
                return Err(duplicate_name(&input.name, format!("inputs[{index}].name")));
            }
        }

        let mut scalar_count = 2_usize;
        for (index, uniform) in self.uniforms.iter().enumerate() {
            let path = format!("uniforms[{index}]");
            validate_identifier(&uniform.name, &format!("{path}.name"))?;
            if !names.insert(uniform.name.clone()) {
                return Err(duplicate_name(&uniform.name, format!("{path}.name")));
            }
            validate_uniform(uniform, &path)?;
            scalar_count += uniform.uniform_type.scalar_width();
        }
        if scalar_count > MAX_UNIFORM_SCALARS {
            return Err(ShaderDiagnostic::new(
                DiagnosticCode::BudgetExceeded,
                "uniforms",
                format!("{scalar_count} ABI scalars exceed the v1 limit {MAX_UNIFORM_SCALARS}"),
            ));
        }
        Ok(())
    }
}

fn validate_uniform(uniform: &ShaderUniform, path: &str) -> Result<(), ShaderDiagnostic> {
    if uniform.required == uniform.default.is_some() {
        return Err(ShaderDiagnostic::new(
            DiagnosticCode::UniformDefault,
            format!("{path}.default"),
            "required uniforms cannot have defaults; optional uniforms must have one",
        ));
    }
    if let Some(default) = &uniform.default {
        if default.uniform_type() != uniform.uniform_type {
            return Err(ShaderDiagnostic::new(
                DiagnosticCode::UniformType,
                format!("{path}.default"),
                "default kind does not match the declared uniform type",
            ));
        }
        validate_value(
            default,
            uniform.min,
            uniform.max,
            &format!("{path}.default"),
        )?;
    }
    if uniform.uniform_type == UniformType::Color || uniform.uniform_type == UniformType::Bool {
        if uniform.min.is_some() || uniform.max.is_some() {
            return Err(ShaderDiagnostic::new(
                DiagnosticCode::UniformRange,
                path,
                "color and bool uniforms do not accept min/max",
            ));
        }
    } else {
        let (Some(min), Some(max)) = (uniform.min, uniform.max) else {
            return Err(ShaderDiagnostic::new(
                DiagnosticCode::UniformRange,
                path,
                "float and float2 uniforms require finite min and max",
            ));
        };
        if !min.is_finite() || !max.is_finite() || min > max {
            return Err(ShaderDiagnostic::new(
                DiagnosticCode::UniformRange,
                path,
                "uniform range must be finite and min <= max",
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_value(
    value: &UniformValue,
    min: Option<f32>,
    max: Option<f32>,
    path: &str,
) -> Result<(), ShaderDiagnostic> {
    for number in value.numbers() {
        if !number.is_finite() {
            return Err(ShaderDiagnostic::new(
                DiagnosticCode::UniformRange,
                path,
                "uniform value must be finite",
            ));
        }
        if let (Some(min), Some(max)) = (min, max) {
            if *number < min || *number > max {
                return Err(ShaderDiagnostic::new(
                    DiagnosticCode::UniformRange,
                    path,
                    format!("uniform value {number} is outside {min}..={max}"),
                ));
            }
        }
    }
    if let UniformValue::Color(channels) = value {
        if channels
            .iter()
            .any(|channel| !(0.0..=1.0).contains(channel))
        {
            return Err(ShaderDiagnostic::new(
                DiagnosticCode::UniformRange,
                path,
                "straight color channels must be in 0..=1",
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_package_name(name: &str, path: &str) -> Result<(), ShaderDiagnostic> {
    let valid = !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--");
    if valid {
        Ok(())
    } else {
        Err(ShaderDiagnostic::new(
            DiagnosticCode::Identifier,
            path,
            "package name must be 1..=64 lowercase kebab-case bytes",
        ))
    }
}

pub(crate) fn validate_identifier(name: &str, path: &str) -> Result<(), ShaderDiagnostic> {
    let mut bytes = name.bytes();
    let reserved = [
        "break",
        "case",
        "const",
        "continue",
        "default",
        "discard",
        "do",
        "else",
        "false",
        "for",
        "half",
        "half2",
        "half3",
        "half4",
        "if",
        "in",
        "inout",
        "main",
        "out",
        "return",
        "sampleContent",
        "shader",
        "struct",
        "switch",
        "true",
        "uniform",
        "while",
    ];
    let valid = name.len() <= 64
        && bytes
            .next()
            .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        && !reserved.contains(&name)
        && !name.starts_with("sk_")
        && !name.starts_with("valle_")
        && !name.starts_with("sample_");
    if valid {
        Ok(())
    } else {
        Err(ShaderDiagnostic::new(
            DiagnosticCode::Identifier,
            path,
            "ABI name must be a non-reserved 1..=64 ASCII identifier",
        ))
    }
}

fn duplicate_name(name: &str, path: String) -> ShaderDiagnostic {
    ShaderDiagnostic::new(
        DiagnosticCode::DuplicateName,
        path,
        format!("duplicate or reserved ABI name {name}"),
    )
}
