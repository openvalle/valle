//! Low-level `valle-json/1` parsing and RFC 8785 encoding.
//!
//! This module deliberately accepts untrusted bytes instead of a pre-built
//! [`serde_json::Value`].  Building a normal JSON object first would make
//! duplicate member names impossible to detect.

use std::{cell::RefCell, collections::HashSet, fmt};

use serde::{
    Serialize,
    de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor},
};
use serde_json::{Map, Number, Value};

pub(crate) const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

/// A failure to parse or encode `valle-json/1`.
///
/// Parser implementation details are intentionally not retained.  Callers can
/// therefore map these stable classes into their contract-specific decode
/// errors without exposing serde's diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum CanonicalJsonError {
    #[error("UTF-8 BOM is not allowed")]
    Utf8Bom,
    #[error("input contains invalid Unicode")]
    InvalidUnicode,
    #[error("object contains duplicate key `{key}`")]
    DuplicateObjectKey { key: String },
    #[error("integer `{value}` is outside the interoperable JSON integer range")]
    UnsafeInteger { value: String },
    #[error("input contains a non-finite number")]
    NonFiniteNumber,
    #[error("input is not valid JSON")]
    MalformedJson,
    #[error("value cannot be encoded as canonical JSON")]
    Encode,
}

/// Parse untrusted `valle-json/1` bytes without losing duplicate object keys.
///
/// Arrays retain their input order. Objects are not sorted here; RFC 8785's
/// UTF-16 code-unit ordering is applied only when [`to_canonical_bytes`] is
/// called.
pub(crate) fn parse_strict(input: &[u8]) -> Result<Value, CanonicalJsonError> {
    if input.starts_with(&[0xef, 0xbb, 0xbf]) {
        return Err(CanonicalJsonError::Utf8Bom);
    }

    let input = std::str::from_utf8(input).map_err(|_| CanonicalJsonError::InvalidUnicode)?;
    validate_unicode_escapes(input)?;
    validate_integer_tokens(input)?;

    let failure = RefCell::new(None);
    let seed = StrictValueSeed { failure: &failure };
    let mut deserializer = serde_json::Deserializer::from_str(input);
    let value = seed.deserialize(&mut deserializer);

    match value {
        Ok(value) => {
            deserializer
                .end()
                .map_err(|_| CanonicalJsonError::MalformedJson)?;
            Ok(value)
        }
        Err(_) => Err(failure
            .into_inner()
            .unwrap_or(CanonicalJsonError::MalformedJson)),
    }
}

/// Encode a trusted value with Valle's input constraints and RFC 8785 JCS.
///
/// The intermediate ordinary JSON bytes are fed back through [`parse_strict`]
/// so an unsafe integer in a typed field or nested metadata cannot bypass the
/// same checks applied to wire input.  The first JCS serialization is a
/// finiteness probe: serde_json otherwise represents non-finite floats as
/// `null`, which would erase the violation before strict parsing.
pub(crate) fn to_canonical_bytes<T>(value: &T) -> Result<Vec<u8>, CanonicalJsonError>
where
    T: Serialize + ?Sized,
{
    serde_jcs::to_vec(value).map_err(|_| CanonicalJsonError::Encode)?;
    let serialized = serde_json::to_vec(value).map_err(|_| CanonicalJsonError::Encode)?;
    let strict_value = parse_strict(&serialized)?;
    serde_jcs::to_vec(&strict_value).map_err(|_| CanonicalJsonError::Encode)
}

/// Strictly parse untrusted JSON and immediately emit its canonical bytes.
#[allow(dead_code)] // exercised by the conformance corpus; product decode also needs the parsed Value.
pub(crate) fn canonicalize(input: &[u8]) -> Result<Vec<u8>, CanonicalJsonError> {
    let value = parse_strict(input)?;
    to_canonical_bytes(&value)
}

#[derive(Clone, Copy)]
struct StrictValueSeed<'a> {
    failure: &'a RefCell<Option<CanonicalJsonError>>,
}

impl<'de> DeserializeSeed<'de> for StrictValueSeed<'_> {
    type Value = Value;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(StrictValueVisitor {
            failure: self.failure,
        })
    }
}

struct StrictValueVisitor<'a> {
    failure: &'a RefCell<Option<CanonicalJsonError>>,
}

impl StrictValueVisitor<'_> {
    fn reject<E>(&self, failure: CanonicalJsonError) -> E
    where
        E: de::Error,
    {
        if self.failure.borrow().is_none() {
            *self.failure.borrow_mut() = Some(failure);
        }
        E::custom("valle-json/1 preflight rejected the input")
    }

    fn number_from_f64<E>(&self, value: f64) -> Result<Value, E>
    where
        E: de::Error,
    {
        if !value.is_finite() {
            return Err(self.reject(CanonicalJsonError::NonFiniteNumber));
        }

        Number::from_f64(value)
            .map(Value::Number)
            .ok_or_else(|| self.reject(CanonicalJsonError::NonFiniteNumber))
    }
}

impl<'de> Visitor<'de> for StrictValueVisitor<'_> {
    type Value = Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a valle-json/1 value")
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(Value::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        let limit = MAX_SAFE_INTEGER as i64;
        if !(-limit..=limit).contains(&value) {
            return Err(self.reject(CanonicalJsonError::UnsafeInteger {
                value: value.to_string(),
            }));
        }
        Ok(Value::Number(Number::from(value)))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        if value > MAX_SAFE_INTEGER {
            return Err(self.reject(CanonicalJsonError::UnsafeInteger {
                value: value.to_string(),
            }));
        }
        Ok(Value::Number(Number::from(value)))
    }

    fn visit_f32<E>(self, value: f32) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.number_from_f64(f64::from(value))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.number_from_f64(value)
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
        Ok(Value::String(value.to_owned()))
    }

    fn visit_borrowed_str<E>(self, value: &'de str) -> Result<Self::Value, E> {
        Ok(Value::String(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(Value::String(value))
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::with_capacity(sequence.size_hint().unwrap_or(0));
        let seed = StrictValueSeed {
            failure: self.failure,
        };
        while let Some(value) = sequence.next_element_seed(seed)? {
            values.push(value);
        }
        Ok(Value::Array(values))
    }

    fn visit_map<A>(self, mut object: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut seen = HashSet::with_capacity(object.size_hint().unwrap_or(0));
        let mut values = Map::new();
        let seed = StrictValueSeed {
            failure: self.failure,
        };

        while let Some(key) = object.next_key::<String>()? {
            if !seen.insert(key.clone()) {
                return Err(self.reject(CanonicalJsonError::DuplicateObjectKey { key }));
            }
            let value = object.next_value_seed(seed)?;
            values.insert(key, value);
        }

        Ok(Value::Object(values))
    }
}

fn validate_unicode_escapes(input: &str) -> Result<(), CanonicalJsonError> {
    let bytes = input.as_bytes();
    let mut index = 0;
    let mut in_string = false;

    while index < bytes.len() {
        match (in_string, bytes[index]) {
            (false, b'"') => {
                in_string = true;
                index += 1;
            }
            (false, _) => index += 1,
            (true, b'"') => {
                in_string = false;
                index += 1;
            }
            (true, b'\\') if bytes.get(index + 1) == Some(&b'u') => {
                let Some(first) = parse_hex_quad(bytes, index + 2) else {
                    // Let the JSON parser classify malformed escape syntax.
                    index += 2;
                    continue;
                };

                if (0xd800..=0xdbff).contains(&first) {
                    let second_escape = index + 6;
                    if bytes.get(second_escape) != Some(&b'\\')
                        || bytes.get(second_escape + 1) != Some(&b'u')
                    {
                        return Err(CanonicalJsonError::InvalidUnicode);
                    }
                    let Some(second) = parse_hex_quad(bytes, second_escape + 2) else {
                        return Err(CanonicalJsonError::InvalidUnicode);
                    };
                    if !(0xdc00..=0xdfff).contains(&second) {
                        return Err(CanonicalJsonError::InvalidUnicode);
                    }
                    index += 12;
                } else if (0xdc00..=0xdfff).contains(&first) {
                    return Err(CanonicalJsonError::InvalidUnicode);
                } else {
                    index += 6;
                }
            }
            (true, b'\\') => index += 2,
            (true, _) => index += 1,
        }
    }

    Ok(())
}

fn parse_hex_quad(bytes: &[u8], start: usize) -> Option<u16> {
    let digits = bytes.get(start..start + 4)?;
    digits.iter().try_fold(0_u16, |value, digit| {
        let digit = match digit {
            b'0'..=b'9' => u16::from(*digit - b'0'),
            b'a'..=b'f' => u16::from(*digit - b'a' + 10),
            b'A'..=b'F' => u16::from(*digit - b'A' + 10),
            _ => return None,
        };
        Some((value << 4) | digit)
    })
}

fn validate_integer_tokens(input: &str) -> Result<(), CanonicalJsonError> {
    let bytes = input.as_bytes();
    let mut index = 0;
    let mut in_string = false;

    while index < bytes.len() {
        match bytes[index] {
            b'"' => {
                in_string = !in_string;
                index += 1;
            }
            b'\\' if in_string => index += 2,
            b'-' | b'0'..=b'9' if !in_string => {
                let start = index;
                if bytes[index] == b'-' {
                    index += 1;
                }
                while matches!(bytes.get(index), Some(b'0'..=b'9')) {
                    index += 1;
                }

                let mut is_integer = true;
                if bytes.get(index) == Some(&b'.') {
                    is_integer = false;
                    index += 1;
                    while matches!(bytes.get(index), Some(b'0'..=b'9')) {
                        index += 1;
                    }
                }
                if matches!(bytes.get(index), Some(b'e' | b'E')) {
                    is_integer = false;
                    index += 1;
                    if matches!(bytes.get(index), Some(b'+' | b'-')) {
                        index += 1;
                    }
                    while matches!(bytes.get(index), Some(b'0'..=b'9')) {
                        index += 1;
                    }
                }

                let token = &input[start..index];
                if is_integer {
                    if integer_token_is_unsafe(token) {
                        return Err(CanonicalJsonError::UnsafeInteger {
                            value: token.to_owned(),
                        });
                    }
                } else if token.parse::<f64>().is_ok_and(|number| !number.is_finite()) {
                    return Err(CanonicalJsonError::NonFiniteNumber);
                }
            }
            _ => index += 1,
        }
    }

    Ok(())
}

fn integer_token_is_unsafe(token: &str) -> bool {
    const MAX: &str = "9007199254740991";

    let magnitude = token.strip_prefix('-').unwrap_or(token);
    let magnitude = magnitude.trim_start_matches('0');
    magnitude.len() > MAX.len() || (magnitude.len() == MAX.len() && magnitude > MAX)
}
