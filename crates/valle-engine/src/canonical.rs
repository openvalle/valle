use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::{Map, Number, Value};
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum CanonicalError {
    #[error("value cannot be represented as canonical JSON: {0}")]
    Serialize(#[from] serde_json::Error),
}

/// Stable semantic bytes for cache identities and parity probes.
///
/// Object keys are sorted recursively and negative zero is collapsed. All public compilation and
/// frame types are closed serde shapes, so this is an internal canonical form rather than a public
/// interchange protocol.
pub(crate) fn bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, CanonicalError> {
    let value = serde_json::to_value(value)?;
    serde_json::to_vec(&normalize(value)).map_err(Into::into)
}

fn normalize(value: Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.into_iter().map(normalize).collect()),
        Value::Object(values) => {
            let sorted: BTreeMap<_, _> = values
                .into_iter()
                .map(|(key, value)| (key, normalize(value)))
                .collect();
            let mut object = Map::new();
            for (key, value) in sorted {
                object.insert(key, value);
            }
            Value::Object(object)
        }
        Value::Number(number) if is_negative_zero(&number) => {
            Value::Number(Number::from_f64(0.0).expect("zero is finite"))
        }
        other => other,
    }
}

fn is_negative_zero(number: &Number) -> bool {
    number
        .as_f64()
        .is_some_and(|value| value == 0.0 && value.is_sign_negative())
}

#[cfg(test)]
mod tests {
    use serde::Serialize;

    use super::bytes;

    #[derive(Serialize)]
    struct Probe {
        z: f64,
        a: serde_json::Value,
    }

    #[test]
    fn sorts_objects_and_collapses_negative_zero() {
        let probe = Probe {
            z: -0.0,
            a: serde_json::json!({"z": 2, "a": 1}),
        };
        assert_eq!(bytes(&probe).unwrap(), br#"{"a":{"a":1,"z":2},"z":0.0}"#);
    }
}
