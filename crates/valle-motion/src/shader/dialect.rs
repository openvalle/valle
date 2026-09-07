use std::collections::BTreeSet;

use super::{
    DiagnosticCode, MAX_SAMPLES_PER_PIXEL, MAX_SOURCE_BYTES, ShaderDiagnostic, ShaderManifest,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DialectReport {
    pub source_bytes: usize,
    pub samples_per_pixel: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Ident(String),
    Punct(char),
}

pub fn validate_dialect(
    manifest: &ShaderManifest,
    source: &str,
) -> Result<DialectReport, ShaderDiagnostic> {
    manifest.validate_shape()?;
    if source.len() > MAX_SOURCE_BYTES {
        return Err(ShaderDiagnostic::new(
            DiagnosticCode::BudgetExceeded,
            "source",
            format!(
                "{} source bytes exceed the v1 limit {MAX_SOURCE_BYTES}",
                source.len()
            ),
        ));
    }
    if !source.is_ascii() {
        return Err(dialect_error("v1 source must be ASCII"));
    }
    let tokens = tokenize(source)?;
    let expected = ["half4", "valle_main", "float2", "uv"];
    let signature = tokens
        .iter()
        .take(7)
        .map(|token| match token {
            Token::Ident(value) => value.as_str(),
            Token::Punct('(') => "(",
            Token::Punct(')') => ")",
            Token::Punct('{') => "{",
            _ => "?",
        })
        .collect::<Vec<_>>();
    if signature
        != [
            expected[0],
            expected[1],
            "(",
            expected[2],
            expected[3],
            ")",
            "{",
        ]
    {
        return Err(dialect_error(
            "source must contain exactly `half4 valle_main(float2 uv) { ... }`",
        ));
    }
    if !matches!(tokens.last(), Some(Token::Punct('}'))) {
        return Err(dialect_error(
            "valle_main must end at the end of the source",
        ));
    }
    validate_balanced(&tokens)?;
    if tokens[7..tokens.len() - 1]
        .iter()
        .any(|token| matches!(token, Token::Punct('{') | Token::Punct('}')))
    {
        return Err(dialect_error(
            "v1 contains one straight-line function body; nested blocks are forbidden",
        ));
    }

    let forbidden = [
        "for",
        "while",
        "do",
        "switch",
        "discard",
        "uniform",
        "shader",
        "storage",
        "atomic",
        "main",
        "eval",
        "sk_FragCoord",
        "sk_RTAdjust",
    ];
    let child_names = manifest
        .inputs
        .iter()
        .map(|input| input.name.as_str())
        .chain(core::iter::once("content"))
        .collect::<BTreeSet<_>>();
    let allowed_calls = [
        "half4",
        "half3",
        "half2",
        "half",
        "float4",
        "float3",
        "float2",
        "float",
        "bool",
        "abs",
        "ceil",
        "clamp",
        "cos",
        "dot",
        "floor",
        "fract",
        "length",
        "max",
        "min",
        "mix",
        "normalize",
        "pow",
        "saturate",
        "sin",
        "smoothstep",
        "sqrt",
        "step",
    ]
    .into_iter()
    .map(str::to_owned)
    .chain(core::iter::once("sampleContent".to_owned()))
    .chain(
        manifest
            .inputs
            .iter()
            .map(|input| format!("sample_{}", input.name)),
    )
    .collect::<BTreeSet<_>>();

    let mut samples_per_pixel = 0;
    for (index, token) in tokens.iter().enumerate().skip(7) {
        let Token::Ident(identifier) = token else {
            continue;
        };
        if forbidden.contains(&identifier.as_str()) {
            return Err(dialect_error(format!(
                "identifier `{identifier}` is forbidden in Valle-SkSL v1"
            )));
        }
        if child_names.contains(identifier.as_str()) {
            return Err(dialect_error(format!(
                "texture `{identifier}` is opaque; use its Valle sample helper"
            )));
        }
        if matches!(tokens.get(index + 1), Some(Token::Punct('('))) {
            if !allowed_calls.contains(identifier) {
                return Err(dialect_error(format!(
                    "function `{identifier}` is outside the Valle-SkSL v1 allowlist"
                )));
            }
            if identifier == "sampleContent" || identifier.starts_with("sample_") {
                samples_per_pixel += 1;
            }
        }
        if identifier.starts_with("sk_") {
            return Err(dialect_error(
                "Skia runtime globals are not part of the dialect",
            ));
        }
    }
    if samples_per_pixel > MAX_SAMPLES_PER_PIXEL {
        return Err(ShaderDiagnostic::new(
            DiagnosticCode::BudgetExceeded,
            "source",
            format!(
                "{samples_per_pixel} static samples per pixel exceed the v1 limit {MAX_SAMPLES_PER_PIXEL}"
            ),
        ));
    }
    if !tokens.windows(2).any(|window| {
        matches!(&window[0], Token::Ident(value) if value == "return")
            && !matches!(&window[1], Token::Punct(';'))
    }) {
        return Err(dialect_error(
            "valle_main must return a straight-alpha color",
        ));
    }
    Ok(DialectReport {
        source_bytes: source.len(),
        samples_per_pixel,
    })
}

pub fn lower_to_sksl(manifest: &ShaderManifest, source: &str) -> Result<String, ShaderDiagnostic> {
    validate_dialect(manifest, source)?;
    let mut output = String::new();
    output.push_str("// generated by valle-shader valle-sksl@1\n");
    output.push_str("uniform shader content;\n");
    for input in &manifest.inputs {
        output.push_str(&format!("uniform shader {};\n", input.name));
    }
    output.push_str("uniform float2 resolution;\n");
    for uniform in &manifest.uniforms {
        let backend_name = if uniform.uniform_type == super::UniformType::Bool {
            format!("valle_uniform_{}", uniform.name)
        } else {
            uniform.name.clone()
        };
        output.push_str(&format!(
            "uniform {} {};\n",
            uniform.uniform_type.sksl_name(),
            backend_name
        ));
    }
    output.push_str(
        "half4 valle_unpremul(half4 c) { return c.a > 0.0 ? half4(c.rgb / c.a, c.a) : half4(0.0); }\n",
    );
    output.push_str(
        "half4 sampleContent(float2 uv) { return valle_unpremul(content.eval(clamp(uv, 0.0, 1.0) * resolution)); }\n",
    );
    for input in &manifest.inputs {
        output.push_str(&format!(
            "half4 sample_{}(float2 uv) {{ return valle_unpremul({}.eval(clamp(uv, 0.0, 1.0) * resolution)); }}\n",
            input.name, input.name
        ));
    }
    let mut author_source = source.to_owned();
    for uniform in &manifest.uniforms {
        if uniform.uniform_type == super::UniformType::Bool {
            author_source = replace_identifier(
                &author_source,
                &uniform.name,
                &format!("(valle_uniform_{} >= 0.5)", uniform.name),
            );
        }
    }
    output.push_str(&author_source);
    if !source.ends_with('\n') {
        output.push('\n');
    }
    let alpha = if manifest.output.allow_transparent {
        "half(clamp(straight.a, 0.0, 1.0))"
    } else {
        "half(1.0)"
    };
    output.push_str(&format!(
        "half4 main(float2 xy) {{ half4 straight = valle_main(xy / resolution); half a = {alpha}; return half4(half3(clamp(straight.rgb, 0.0, 1.0)) * a, a); }}\n"
    ));
    Ok(output)
}

fn replace_identifier(source: &str, needle: &str, replacement: &str) -> String {
    let bytes = source.as_bytes();
    let mut output = String::with_capacity(source.len() + replacement.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index].is_ascii_alphabetic() || bytes[index] == b'_' {
            let start = index;
            index += 1;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
            {
                index += 1;
            }
            if &source[start..index] == needle {
                output.push_str(replacement);
            } else {
                output.push_str(&source[start..index]);
            }
        } else {
            output.push(bytes[index] as char);
            index += 1;
        }
    }
    output
}

fn tokenize(source: &str) -> Result<Vec<Token>, ShaderDiagnostic> {
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte.is_ascii_whitespace() {
            index += 1;
        } else if byte == b'/' && bytes.get(index + 1) == Some(&b'/') {
            index += 2;
            while index < bytes.len() && bytes[index] != b'\n' {
                index += 1;
            }
        } else if byte == b'/' && bytes.get(index + 1) == Some(&b'*') {
            index += 2;
            let start = index;
            while index + 1 < bytes.len() && !(bytes[index] == b'*' && bytes[index + 1] == b'/') {
                index += 1;
            }
            if index + 1 >= bytes.len() {
                return Err(dialect_error(format!(
                    "unterminated block comment at byte {start}"
                )));
            }
            index += 2;
        } else if byte.is_ascii_alphabetic() || byte == b'_' {
            let start = index;
            index += 1;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
            {
                index += 1;
            }
            tokens.push(Token::Ident(source[start..index].to_owned()));
        } else if byte.is_ascii_digit()
            || (byte == b'.' && bytes.get(index + 1).is_some_and(u8::is_ascii_digit))
        {
            index += 1;
            while index < bytes.len() && (bytes[index].is_ascii_digit() || bytes[index] == b'.') {
                index += 1;
            }
            if index < bytes.len() && matches!(bytes[index], b'e' | b'E') {
                index += 1;
                if index < bytes.len() && matches!(bytes[index], b'+' | b'-') {
                    index += 1;
                }
                while index < bytes.len() && bytes[index].is_ascii_digit() {
                    index += 1;
                }
            }
        } else if matches!(
            byte,
            b'(' | b')'
                | b'{'
                | b'}'
                | b';'
                | b','
                | b'.'
                | b'?'
                | b':'
                | b'+'
                | b'-'
                | b'*'
                | b'/'
                | b'='
                | b'<'
                | b'>'
                | b'!'
                | b'&'
                | b'|'
        ) {
            tokens.push(Token::Punct(byte as char));
            index += 1;
        } else {
            return Err(dialect_error(format!(
                "character `{}` at byte {index} is outside Valle-SkSL v1",
                byte as char
            )));
        }
    }
    Ok(tokens)
}

fn validate_balanced(tokens: &[Token]) -> Result<(), ShaderDiagnostic> {
    let mut stack = Vec::new();
    for token in tokens {
        match token {
            Token::Punct('(') | Token::Punct('{') => stack.push(token),
            Token::Punct(')') => match stack.pop() {
                Some(Token::Punct('(')) => {}
                _ => return Err(dialect_error("unbalanced parenthesis")),
            },
            Token::Punct('}') => match stack.pop() {
                Some(Token::Punct('{')) => {}
                _ => return Err(dialect_error("unbalanced brace")),
            },
            _ => {}
        }
    }
    if stack.is_empty() {
        Ok(())
    } else {
        Err(dialect_error("unclosed parenthesis or brace"))
    }
}

fn dialect_error(message: impl Into<String>) -> ShaderDiagnostic {
    ShaderDiagnostic::new(DiagnosticCode::DialectViolation, "source", message)
}
