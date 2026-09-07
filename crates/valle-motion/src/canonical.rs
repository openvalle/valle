use serde::{Serialize, ser};

/// Canonical JSON failure. The canonical serializer rejects non-finite values before `serde_json`
/// can silently turn them into `null`; Artifact validation still reports the more precise path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalError(pub String);

impl core::fmt::Display for CanonicalError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "canonical serialization: {}", self.0)
    }
}

impl std::error::Error for CanonicalError {}

impl ser::Error for CanonicalError {
    fn custom<T: core::fmt::Display>(message: T) -> Self {
        CanonicalError(message.to_string())
    }
}

/// Object keys are byte-sorted, whitespace is absent, floating-point values use their shortest
/// round-trip form, and both `-0.0` and `0.0` canonicalize to `0.0`.
pub fn canonical_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, CanonicalError> {
    value.serialize(FiniteSerializer)?;
    let value = serde_json::to_value(value).map_err(|e| CanonicalError(e.to_string()))?;
    let mut out = String::new();
    write_value(&value, &mut out)?;
    Ok(out.into_bytes())
}

/// `serde_json` intentionally serializes NaN and infinities as `null`. Run a no-allocation walk
/// through the same `Serialize` implementation first so wire hashing can never accept that lossy
/// substitution.
struct FiniteSerializer;

struct FiniteCompound;

impl ser::Serializer for FiniteSerializer {
    type Ok = ();
    type Error = CanonicalError;
    type SerializeSeq = FiniteCompound;
    type SerializeTuple = FiniteCompound;
    type SerializeTupleStruct = FiniteCompound;
    type SerializeTupleVariant = FiniteCompound;
    type SerializeMap = FiniteCompound;
    type SerializeStruct = FiniteCompound;
    type SerializeStructVariant = FiniteCompound;

    fn serialize_bool(self, _: bool) -> Result<(), Self::Error> {
        Ok(())
    }
    fn serialize_i8(self, _: i8) -> Result<(), Self::Error> {
        Ok(())
    }
    fn serialize_i16(self, _: i16) -> Result<(), Self::Error> {
        Ok(())
    }
    fn serialize_i32(self, _: i32) -> Result<(), Self::Error> {
        Ok(())
    }
    fn serialize_i64(self, _: i64) -> Result<(), Self::Error> {
        Ok(())
    }
    fn serialize_i128(self, _: i128) -> Result<(), Self::Error> {
        Ok(())
    }
    fn serialize_u8(self, _: u8) -> Result<(), Self::Error> {
        Ok(())
    }
    fn serialize_u16(self, _: u16) -> Result<(), Self::Error> {
        Ok(())
    }
    fn serialize_u32(self, _: u32) -> Result<(), Self::Error> {
        Ok(())
    }
    fn serialize_u64(self, _: u64) -> Result<(), Self::Error> {
        Ok(())
    }
    fn serialize_u128(self, _: u128) -> Result<(), Self::Error> {
        Ok(())
    }
    fn serialize_f32(self, value: f32) -> Result<(), Self::Error> {
        finite(value.is_finite())
    }
    fn serialize_f64(self, value: f64) -> Result<(), Self::Error> {
        finite(value.is_finite())
    }
    fn serialize_char(self, _: char) -> Result<(), Self::Error> {
        Ok(())
    }
    fn serialize_str(self, _: &str) -> Result<(), Self::Error> {
        Ok(())
    }
    fn serialize_bytes(self, _: &[u8]) -> Result<(), Self::Error> {
        Ok(())
    }
    fn serialize_none(self) -> Result<(), Self::Error> {
        Ok(())
    }
    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Result<(), Self::Error> {
        value.serialize(self)
    }
    fn serialize_unit(self) -> Result<(), Self::Error> {
        Ok(())
    }
    fn serialize_unit_struct(self, _: &'static str) -> Result<(), Self::Error> {
        Ok(())
    }
    fn serialize_unit_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        value.serialize(self)
    }
    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        value.serialize(self)
    }
    fn serialize_seq(self, _: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        Ok(FiniteCompound)
    }
    fn serialize_tuple(self, _: usize) -> Result<Self::SerializeTuple, Self::Error> {
        Ok(FiniteCompound)
    }
    fn serialize_tuple_struct(
        self,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        Ok(FiniteCompound)
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        Ok(FiniteCompound)
    }
    fn serialize_map(self, _: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        Ok(FiniteCompound)
    }
    fn serialize_struct(
        self,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        Ok(FiniteCompound)
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        Ok(FiniteCompound)
    }
}

fn finite(is_finite: bool) -> Result<(), CanonicalError> {
    is_finite
        .then_some(())
        .ok_or_else(|| CanonicalError("non-finite number".into()))
}

impl ser::SerializeSeq for FiniteCompound {
    type Ok = ();
    type Error = CanonicalError;

    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        value.serialize(FiniteSerializer)
    }
    fn end(self) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl ser::SerializeTuple for FiniteCompound {
    type Ok = ();
    type Error = CanonicalError;

    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        value.serialize(FiniteSerializer)
    }
    fn end(self) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl ser::SerializeTupleStruct for FiniteCompound {
    type Ok = ();
    type Error = CanonicalError;

    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        value.serialize(FiniteSerializer)
    }
    fn end(self) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl ser::SerializeTupleVariant for FiniteCompound {
    type Ok = ();
    type Error = CanonicalError;

    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        value.serialize(FiniteSerializer)
    }
    fn end(self) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl ser::SerializeMap for FiniteCompound {
    type Ok = ();
    type Error = CanonicalError;

    fn serialize_key<T: ?Sized + Serialize>(&mut self, key: &T) -> Result<(), Self::Error> {
        key.serialize(FiniteSerializer)
    }
    fn serialize_value<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        value.serialize(FiniteSerializer)
    }
    fn end(self) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl ser::SerializeStruct for FiniteCompound {
    type Ok = ();
    type Error = CanonicalError;

    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        _: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        value.serialize(FiniteSerializer)
    }
    fn end(self) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl ser::SerializeStructVariant for FiniteCompound {
    type Ok = ();
    type Error = CanonicalError;

    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        _: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        value.serialize(FiniteSerializer)
    }
    fn end(self) -> Result<(), Self::Error> {
        Ok(())
    }
}

fn write_value(value: &serde_json::Value, out: &mut String) -> Result<(), CanonicalError> {
    use serde_json::Value;
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(value) => out.push_str(if *value { "true" } else { "false" }),
        Value::Number(value) => {
            if let Some(value) = value.as_i64() {
                out.push_str(&value.to_string());
            } else if let Some(value) = value.as_u64() {
                out.push_str(&value.to_string());
            } else {
                let value = value
                    .as_f64()
                    .ok_or_else(|| CanonicalError("unrepresentable number".into()))?;
                if !value.is_finite() {
                    return Err(CanonicalError("non-finite number".into()));
                }
                if value == 0.0 {
                    out.push_str("0.0");
                } else {
                    out.push_str(&format!("{value:?}"));
                }
            }
        }
        Value::String(value) => {
            out.push_str(&serde_json::to_string(value).map_err(|e| CanonicalError(e.to_string()))?)
        }
        Value::Array(values) => {
            out.push('[');
            for (index, value) in values.iter().enumerate() {
                if index != 0 {
                    out.push(',');
                }
                write_value(value, out)?;
            }
            out.push(']');
        }
        Value::Object(values) => {
            let mut keys: Vec<&String> = values.keys().collect();
            keys.sort_unstable_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
            out.push('{');
            for (index, key) in keys.iter().enumerate() {
                if index != 0 {
                    out.push(',');
                }
                out.push_str(
                    &serde_json::to_string(key).map_err(|e| CanonicalError(e.to_string()))?,
                );
                out.push(':');
                write_value(&values[*key], out)?;
            }
            out.push('}');
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorts_keys_removes_whitespace_and_normalizes_negative_zero() {
        let value = serde_json::json!({"z": -0.0, "a": [1.0, 0.1]});
        assert_eq!(
            String::from_utf8(canonical_bytes(&value).unwrap()).unwrap(),
            r#"{"a":[1.0,0.1],"z":0.0}"#
        );
    }

    #[test]
    fn rejects_non_finite_before_serde_json_can_turn_it_into_null() {
        assert_eq!(
            canonical_bytes(&[f64::NAN]).unwrap_err(),
            CanonicalError("non-finite number".into())
        );
        assert!(canonical_bytes(&[f64::INFINITY]).is_err());
        assert!(canonical_bytes(&[f64::NEG_INFINITY]).is_err());
    }
}
