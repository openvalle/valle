use serde::Serialize;

use super::{DiagnosticCode, ShaderDiagnostic};

pub(crate) fn canonical_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, ShaderDiagnostic> {
    let value = serde_json::to_value(value).map_err(|error| {
        ShaderDiagnostic::new(
            DiagnosticCode::ManifestInvalid,
            "manifest",
            format!("cannot serialize canonical value: {error}"),
        )
    })?;
    let mut output = String::new();
    write_value(&value, &mut output)?;
    Ok(output.into_bytes())
}

fn write_value(value: &serde_json::Value, output: &mut String) -> Result<(), ShaderDiagnostic> {
    use serde_json::Value;
    match value {
        Value::Null => output.push_str("null"),
        Value::Bool(value) => output.push_str(if *value { "true" } else { "false" }),
        Value::Number(value) => {
            if let Some(value) = value.as_i64() {
                output.push_str(&value.to_string());
            } else if let Some(value) = value.as_u64() {
                output.push_str(&value.to_string());
            } else {
                let value = value.as_f64().ok_or_else(|| {
                    ShaderDiagnostic::new(
                        DiagnosticCode::ManifestInvalid,
                        "manifest",
                        "unrepresentable canonical number",
                    )
                })?;
                if !value.is_finite() {
                    return Err(ShaderDiagnostic::new(
                        DiagnosticCode::ManifestInvalid,
                        "manifest",
                        "non-finite canonical number",
                    ));
                }
                if value == 0.0 {
                    output.push_str("0.0");
                } else {
                    output.push_str(&format!("{value:?}"));
                }
            }
        }
        Value::String(value) => output.push_str(
            &serde_json::to_string(value).expect("serializing an existing JSON string cannot fail"),
        ),
        Value::Array(values) => {
            output.push('[');
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                write_value(value, output)?;
            }
            output.push(']');
        }
        Value::Object(values) => {
            let mut keys = values.keys().collect::<Vec<_>>();
            keys.sort_unstable_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
            output.push('{');
            for (index, key) in keys.iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                output.push_str(
                    &serde_json::to_string(key)
                        .expect("serializing an existing JSON object key cannot fail"),
                );
                output.push(':');
                write_value(&values[*key], output)?;
            }
            output.push('}');
        }
    }
    Ok(())
}
