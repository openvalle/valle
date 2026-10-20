//! Shader code written in a ShaderLayer's `code`. It declares its uniforms and texture inputs at
//! the top, as SkSL does, so it needs no descriptor: `uniform <type> <name>;` declares a uniform,
//! and `uniform shader <name>;` an input read with `sample_<name>(uv)`.

use super::{
    BudgetClass, DiagnosticCode, InputKind, InputSampling, InputWrap, OutputContract,
    ShaderDiagnostic, ShaderInput, ShaderManifest, ShaderPackage, ShaderUniform, UniformType,
};

/// The descriptor name and entry of every inline package. The package's content hash, which
/// covers the code, tells packages apart.
const INLINE_NAME: &str = "inline";
const INLINE_ENTRY: &str = "inline.vsksl";

const UNIFORM_TYPES: &str = concat!(
    "float, float2, float3, float4, float2x2, float3x3, float4x4, color or bool, ",
    "or shader for an input"
);

/// A declaration at the top level of inline code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InlineDeclaration {
    /// `uniform <type> <name>;`
    Uniform {
        name: String,
        uniform_type: UniformType,
    },
    /// `uniform shader <name>;`
    Input { name: String },
}

/// Inline code split into its declarations, in source order, and the shader body: the code with
/// each declaration blanked, so the dialect reports positions in the code as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineCode {
    pub declarations: Vec<InlineDeclaration>,
    pub body: String,
}

/// How a ShaderLayer samples one texture input. A descriptor states these per input; inline code
/// takes them from the layer's `inputs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputOptions {
    pub kind: InputKind,
    pub sampling: InputSampling,
    pub wrap: InputWrap,
}

impl InputOptions {
    /// Color inputs filter linearly by default and data inputs read exact texels; both clamp.
    pub fn new(kind: InputKind, sampling: Option<InputSampling>, wrap: Option<InputWrap>) -> Self {
        Self {
            kind,
            sampling: sampling.unwrap_or(match kind {
                InputKind::Color => InputSampling::Linear,
                InputKind::Data => InputSampling::Nearest,
            }),
            wrap: wrap.unwrap_or_default(),
        }
    }
}

impl Default for InputOptions {
    fn default() -> Self {
        Self::new(InputKind::Color, None, None)
    }
}

impl InlineCode {
    pub fn parse(code: &str) -> Result<Self, ShaderDiagnostic> {
        Self::parse_positions(code).map_err(|error| super::dialect::locate(code, error))
    }

    fn parse_positions(code: &str) -> Result<Self, ShaderDiagnostic> {
        if let Some(position) = code.bytes().position(|byte| !byte.is_ascii()) {
            return Err(error(position, "character is outside the shader language"));
        }
        let tokens = tokens(code);
        let mut body = code.as_bytes().to_vec();
        let mut declarations = Vec::new();
        let mut depth = 0usize;
        let mut at = 0;
        while let Some(&(position, text)) = tokens.get(at) {
            match text {
                "{" => depth += 1,
                "}" => depth = depth.saturating_sub(1),
                "uniform" if depth == 0 => {
                    let text_at = |offset: usize| tokens.get(at + offset).map(|(_, text)| *text);
                    let malformed = || {
                        error(
                            position,
                            "declare a uniform as `uniform <type> <name>;` and an input as \
                             `uniform shader <name>;`",
                        )
                    };
                    let (Some(ty), Some(name)) = (text_at(1), text_at(2)) else {
                        return Err(malformed());
                    };
                    if !is_identifier(ty) || !is_identifier(name) {
                        return Err(malformed());
                    }
                    match text_at(3) {
                        Some(";") => {}
                        Some("[") => {
                            return Err(error(
                                position,
                                "uniform arrays are not available; declare one uniform per value",
                            ));
                        }
                        Some("=") => {
                            return Err(error(
                                position,
                                "a uniform takes its value from the ShaderLayer's `uniforms`, not \
                                 an initializer",
                            ));
                        }
                        _ => return Err(malformed()),
                    }
                    declarations.push(if ty == "shader" {
                        InlineDeclaration::Input {
                            name: name.to_owned(),
                        }
                    } else {
                        InlineDeclaration::Uniform {
                            name: name.to_owned(),
                            uniform_type: uniform_type(ty).ok_or_else(|| {
                                error(
                                    position,
                                    format!(
                                        "uniform type `{ty}` is not available; use {UNIFORM_TYPES}"
                                    ),
                                )
                            })?,
                        }
                    });
                    let end = tokens[at + 3].0 + 1;
                    for byte in &mut body[position..end] {
                        if *byte != b'\n' {
                            *byte = b' ';
                        }
                    }
                    at += 4;
                    continue;
                }
                _ => {}
            }
            at += 1;
        }
        Ok(Self {
            declarations,
            body: String::from_utf8(body).expect("blanking ASCII code keeps it ASCII"),
        })
    }

    /// The package of this code, with each declared input sampled as `options` gives and the
    /// output padded by `padding`, [left, top, right, bottom] local pixels.
    pub fn package(
        &self,
        options: impl Fn(&str) -> InputOptions,
        padding: [u32; 4],
    ) -> Result<ShaderPackage, ShaderDiagnostic> {
        let mut inputs = Vec::new();
        let mut uniforms = Vec::new();
        for declaration in &self.declarations {
            match declaration {
                InlineDeclaration::Input { name } => {
                    let options = options(name);
                    inputs.push(ShaderInput {
                        kind: options.kind,
                        name: name.clone(),
                        required: true,
                        sampling: options.sampling,
                        wrap: options.wrap,
                    });
                }
                InlineDeclaration::Uniform { name, uniform_type } => {
                    // Without a range, a number takes any finite value.
                    uniforms.push(ShaderUniform {
                        name: name.clone(),
                        uniform_type: *uniform_type,
                        required: true,
                        default: None,
                        min: None,
                        max: None,
                    });
                }
            }
        }
        let manifest = ShaderManifest {
            name: INLINE_NAME.to_owned(),
            entry: INLINE_ENTRY.to_owned(),
            inputs,
            uniforms,
            output: OutputContract {
                padding,
                ..OutputContract::default()
            },
            budget: BudgetClass::Local,
        };
        ShaderPackage::compile(manifest, self.body.as_bytes())
    }
}

fn uniform_type(name: &str) -> Option<UniformType> {
    Some(match name {
        "float" | "half" => UniformType::Float,
        "float2" | "half2" => UniformType::Float2,
        "float3" | "half3" => UniformType::Float3,
        "float4" | "half4" => UniformType::Float4,
        "float2x2" => UniformType::Float2x2,
        "float3x3" => UniformType::Float3x3,
        "float4x4" => UniformType::Float4x4,
        "color" => UniformType::Color,
        "bool" => UniformType::Bool,
        _ => return None,
    })
}

fn is_identifier(text: &str) -> bool {
    text.bytes()
        .next()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
}

/// Words, numbers and single punctuation characters with their byte positions, skipping
/// whitespace and comments. An unterminated block comment ends the scan; the dialect reports it.
fn tokens(code: &str) -> Vec<(usize, &str)> {
    let bytes = code.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let start = i;
        if bytes[i].is_ascii_whitespace() {
            i += 1;
        } else if bytes[i..].starts_with(b"//") {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
        } else if bytes[i..].starts_with(b"/*") {
            match code[i + 2..].find("*/") {
                Some(end) => i += end + 4,
                None => break,
            }
        } else if bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' {
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            tokens.push((start, &code[start..i]));
        } else {
            i += 1;
            tokens.push((start, &code[start..i]));
        }
    }
    tokens
}

fn error(position: usize, message: impl Into<String>) -> ShaderDiagnostic {
    ShaderDiagnostic::new(
        DiagnosticCode::DialectViolation,
        position.to_string(),
        message,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const DISSOLVE: &str = "// A dissolve over noise.
uniform float progress;
uniform color edgeColor; /* the burning edge */
uniform shader noise;

float4 valle_main(float2 uv) {
  float4 c = sampleContent(uv);
  float4 grain = sample_noise(uv);
  float edge = smoothstep(progress - 0.05, progress, grain.r);
  return mix(c, edgeColor, edge * (1.0 - edge) * 4.0);
}
";

    #[test]
    fn declarations_leave_the_code_and_keep_its_positions() {
        let inline = InlineCode::parse(DISSOLVE).unwrap();
        assert_eq!(
            inline.declarations,
            [
                InlineDeclaration::Uniform {
                    name: "progress".into(),
                    uniform_type: UniformType::Float,
                },
                InlineDeclaration::Uniform {
                    name: "edgeColor".into(),
                    uniform_type: UniformType::Color,
                },
                InlineDeclaration::Input {
                    name: "noise".into(),
                },
            ]
        );
        assert_eq!(inline.body.len(), DISSOLVE.len());
        assert_eq!(inline.body.lines().count(), DISSOLVE.lines().count());
        assert!(!inline.body.contains("uniform"));
        let main = DISSOLVE.find("float4 valle_main").unwrap();
        assert_eq!(&inline.body[main..], &DISSOLVE[main..]);
        // The comment beside a declaration stays.
        assert!(inline.body.contains("/* the burning edge */"));
    }

    #[test]
    fn the_package_takes_declaration_order_and_input_options() {
        let inline = InlineCode::parse(DISSOLVE).unwrap();
        let package = inline
            .package(
                |_| InputOptions::new(InputKind::Color, Some(InputSampling::Nearest), None),
                [4, 0, 4, 0],
            )
            .unwrap();
        let manifest = &package.manifest;
        assert_eq!(
            manifest
                .uniforms
                .iter()
                .map(|uniform| (
                    uniform.name.as_str(),
                    uniform.uniform_type,
                    uniform.required
                ))
                .collect::<Vec<_>>(),
            [
                ("progress", UniformType::Float, true),
                ("edgeColor", UniformType::Color, true)
            ]
        );
        assert_eq!(manifest.inputs.len(), 1);
        assert_eq!(manifest.inputs[0].sampling, InputSampling::Nearest);
        assert_eq!(manifest.inputs[0].wrap, InputWrap::Clamp);
        assert_eq!(manifest.output.padding, [4, 0, 4, 0]);
        assert!(package.generated_sksl.contains("uniform float progress;"));
        // The same code and options make the same package; other options another one.
        let again = inline
            .package(
                |_| InputOptions::new(InputKind::Color, Some(InputSampling::Nearest), None),
                [4, 0, 4, 0],
            )
            .unwrap();
        assert_eq!(again.content_hash, package.content_hash);
        let linear = inline.package(|_| InputOptions::default(), [4, 0, 4, 0]);
        assert_ne!(linear.unwrap().content_hash, package.content_hash);
        // A frozen package reads back to the same one.
        let frozen = ShaderPackage::from_frozen_shader(&package.frozen()).unwrap();
        assert_eq!(frozen.content_hash, package.content_hash);
        assert_eq!(frozen.abi_hash, package.abi_hash);
    }

    #[test]
    fn defaults_filter_colors_and_read_data_exactly() {
        assert_eq!(InputOptions::default().sampling, InputSampling::Linear);
        assert_eq!(
            InputOptions::new(InputKind::Data, None, Some(InputWrap::Repeat)),
            InputOptions {
                kind: InputKind::Data,
                sampling: InputSampling::Nearest,
                wrap: InputWrap::Repeat,
            }
        );
    }

    #[test]
    fn declarations_inside_functions_stay_for_the_dialect_to_reject() {
        let code = "float4 valle_main(float2 uv) { uniform float x; return float4(x); }";
        let inline = InlineCode::parse(code).unwrap();
        assert!(inline.declarations.is_empty());
        assert_eq!(inline.body, code);
        assert!(inline.package(|_| InputOptions::default(), [0; 4]).is_err());
    }

    #[test]
    fn malformed_declarations_name_the_form_to_write() {
        for (code, expected) in [
            ("uniform int count;", "uniform type `int` is not available"),
            (
                "uniform float values[4];",
                "uniform arrays are not available",
            ),
            ("uniform float x = 1.0;", "not an initializer"),
            (
                "uniform float;",
                "declare a uniform as `uniform <type> <name>;`",
            ),
            (
                "uniform float x",
                "declare a uniform as `uniform <type> <name>;`",
            ),
            (
                "uniform 4 x;",
                "declare a uniform as `uniform <type> <name>;`",
            ),
            ("// \u{e9}\nuniform float x;", "outside the shader language"),
        ] {
            let error = InlineCode::parse(code).unwrap_err();
            assert!(error.message.contains(expected), "{code}: {error}");
        }
        // Descriptor rules still apply to the declared names.
        let duplicate = InlineCode::parse(
            "uniform float x; uniform float x; float4 valle_main(float2 uv) { return float4(x); }",
        )
        .unwrap();
        assert!(
            duplicate
                .package(|_| InputOptions::default(), [0; 4])
                .is_err()
        );
    }
}
