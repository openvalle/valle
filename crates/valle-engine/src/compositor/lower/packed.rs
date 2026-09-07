//! Canonical self-describing binary envelope for product RenderPlan traffic.
//!
//! JSON remains useful for inspectors, but Native/Web steady-state traffic uses this format. The
//! value codec is deliberately small and closed: it carries the exact admitted serde shape while
//! enforcing byte, depth, string and aggregate element budgets before allocating containers.

use std::collections::BTreeMap;

use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Map, Number, Value};
use sha2::{Digest, Sha256};
use thiserror::Error;

const ENDIAN_MARKER: u32 = 0x0102_0304;
const HEADER_LEN: usize = 56;
const CHECKSUM_RANGE: std::ops::Range<usize> = 24..56;
const MAX_DEPTH: usize = 128;
// The packet envelope already has a stricter per-format byte cap. RenderPlan carries nested
// DrawProgram packets as base64 strings, so a single legitimate string may exceed 1 MiB without
// representing millions of generic container values.
const MAX_STRING_BYTES: usize = MAX_PACKED_PLAN_BYTES;
const MAX_CONTAINER_ITEMS: usize = 1_000_000;
const MAX_TOTAL_VALUES: usize = 2_000_000;

const NULL: u8 = 0;
const FALSE: u8 = 1;
const TRUE: u8 = 2;
const I64: u8 = 3;
const U64: u8 = 4;
const F64: u8 = 5;
const STRING: u8 = 6;
const ARRAY: u8 = 7;
const OBJECT: u8 = 8;

pub const MAX_PACKED_PLAN_BYTES: usize = 64 * 1024 * 1024;
// Exact per-frame DrawPrograms live in RenderBindings. Give the binding packet the same bounded
// payload budget that protects the enclosing RenderPlan packet.
pub const MAX_PACKED_BINDINGS_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_PACKED_REQUESTS_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_PACKED_SCHEDULE_BYTES: usize = 16 * 1024 * 1024;

pub(crate) const PLAN_MAGIC: [u8; 8] = *b"VLPLAN\0\0";
pub(crate) const BINDINGS_MAGIC: [u8; 8] = *b"VLBIND\0\0";
pub(crate) const REQUESTS_MAGIC: [u8; 8] = *b"VLREQ\0\0\0";
pub(crate) const SCHEDULE_MAGIC: [u8; 8] = *b"VLSCHED\0";

#[derive(Debug, Error)]
pub enum PackedPlanError {
    #[error("packed {kind} is truncated: need at least {minimum} bytes, got {actual}")]
    Truncated {
        kind: &'static str,
        minimum: usize,
        actual: usize,
    },
    #[error("packed {kind} exceeds byte budget: {actual} > {maximum}")]
    ByteBudget {
        kind: &'static str,
        actual: usize,
        maximum: usize,
    },
    #[error("packed {kind} has the wrong magic")]
    BadMagic { kind: &'static str },
    #[error("unsupported packed {kind} format version {found}; expected {expected}")]
    UnsupportedFormatVersion {
        kind: &'static str,
        found: u32,
        expected: u32,
    },
    #[error("packed {kind} has the wrong endianness marker {found:#010x}")]
    WrongEndianness { kind: &'static str, found: u32 },
    #[error("packed {kind} length mismatch: declared {declared}, got {actual}")]
    LengthMismatch {
        kind: &'static str,
        declared: u64,
        actual: usize,
    },
    #[error("packed {kind} checksum mismatch")]
    ChecksumMismatch { kind: &'static str },
    #[error("packed {kind} value is invalid: {reason}")]
    InvalidValue { kind: &'static str, reason: String },
    #[error("packed {kind} is not the canonical binary representation")]
    NonCanonical { kind: &'static str },
    #[error("could not serialize {kind}: {source}")]
    Serialize {
        kind: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("could not deserialize {kind}: {source}")]
    Deserialize {
        kind: &'static str,
        #[source]
        source: serde_json::Error,
    },
}

pub(crate) fn encode<T: Serialize>(
    value: &T,
    contract: Contract,
) -> Result<Vec<u8>, PackedPlanError> {
    let value = serde_json::to_value(value).map_err(|source| PackedPlanError::Serialize {
        kind: contract.kind,
        source,
    })?;
    let value = normalize(value);
    let mut payload = Vec::new();
    encode_value(&value, &mut payload, 0, contract)?;
    let total = HEADER_LEN
        .checked_add(payload.len())
        .ok_or(PackedPlanError::ByteBudget {
            kind: contract.kind,
            actual: usize::MAX,
            maximum: contract.maximum,
        })?;
    check_size(total, contract)?;

    let mut bytes = Vec::with_capacity(total);
    bytes.extend_from_slice(&contract.magic);
    bytes.extend_from_slice(&contract.version.to_le_bytes());
    bytes.extend_from_slice(&ENDIAN_MARKER.to_le_bytes());
    bytes.extend_from_slice(&(total as u64).to_le_bytes());
    bytes.extend_from_slice(&[0; 32]);
    bytes.extend_from_slice(&payload);
    let checksum = checksum(&bytes);
    bytes[CHECKSUM_RANGE].copy_from_slice(&checksum);
    Ok(bytes)
}

pub(crate) fn decode<T: DeserializeOwned + Serialize>(
    bytes: &[u8],
    contract: Contract,
) -> Result<T, PackedPlanError> {
    let payload = envelope(bytes, contract)?;
    let mut cursor = Cursor::new(payload, contract);
    let value = cursor.value(0)?;
    if !cursor.is_empty() {
        return Err(cursor.invalid("trailing payload bytes"));
    }
    let decoded = serde_json::from_value(value).map_err(|source| PackedPlanError::Deserialize {
        kind: contract.kind,
        source,
    })?;
    if encode(&decoded, contract)? != bytes {
        return Err(PackedPlanError::NonCanonical {
            kind: contract.kind,
        });
    }
    Ok(decoded)
}

#[derive(Clone, Copy)]
pub(crate) struct Contract {
    kind: &'static str,
    magic: [u8; 8],
    version: u32,
    maximum: usize,
}

impl Contract {
    pub(crate) const fn plan(version: u32) -> Self {
        Self {
            kind: "RenderPlanTemplate",
            magic: PLAN_MAGIC,
            version,
            maximum: MAX_PACKED_PLAN_BYTES,
        }
    }

    pub(crate) const fn bindings(version: u32) -> Self {
        Self {
            kind: "RenderBindings",
            magic: BINDINGS_MAGIC,
            version,
            maximum: MAX_PACKED_BINDINGS_BYTES,
        }
    }

    pub(crate) const fn schedule(version: u32) -> Self {
        Self {
            kind: "BoundProgramSchedules",
            magic: SCHEDULE_MAGIC,
            version,
            maximum: MAX_PACKED_SCHEDULE_BYTES,
        }
    }

    pub(crate) const fn requests(version: u32) -> Self {
        Self {
            kind: "ResourceRequestSet",
            magic: REQUESTS_MAGIC,
            version,
            maximum: MAX_PACKED_REQUESTS_BYTES,
        }
    }
}

fn envelope<'a>(bytes: &'a [u8], contract: Contract) -> Result<&'a [u8], PackedPlanError> {
    check_size(bytes.len(), contract)?;
    if bytes.len() < HEADER_LEN {
        return Err(PackedPlanError::Truncated {
            kind: contract.kind,
            minimum: HEADER_LEN,
            actual: bytes.len(),
        });
    }
    if bytes[..8] != contract.magic {
        return Err(PackedPlanError::BadMagic {
            kind: contract.kind,
        });
    }
    let version = u32::from_le_bytes(bytes[8..12].try_into().expect("fixed range"));
    if version != contract.version {
        return Err(PackedPlanError::UnsupportedFormatVersion {
            kind: contract.kind,
            found: version,
            expected: contract.version,
        });
    }
    let endian = u32::from_le_bytes(bytes[12..16].try_into().expect("fixed range"));
    if endian != ENDIAN_MARKER {
        return Err(PackedPlanError::WrongEndianness {
            kind: contract.kind,
            found: endian,
        });
    }
    let declared = u64::from_le_bytes(bytes[16..24].try_into().expect("fixed range"));
    if declared != bytes.len() as u64 {
        return Err(PackedPlanError::LengthMismatch {
            kind: contract.kind,
            declared,
            actual: bytes.len(),
        });
    }
    if bytes[CHECKSUM_RANGE] != checksum(bytes) {
        return Err(PackedPlanError::ChecksumMismatch {
            kind: contract.kind,
        });
    }
    Ok(&bytes[HEADER_LEN..])
}

fn check_size(actual: usize, contract: Contract) -> Result<(), PackedPlanError> {
    if actual > contract.maximum {
        Err(PackedPlanError::ByteBudget {
            kind: contract.kind,
            actual,
            maximum: contract.maximum,
        })
    } else {
        Ok(())
    }
}

fn checksum(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(&bytes[..bytes.len().min(CHECKSUM_RANGE.start)]);
    hasher.update([0; 32]);
    if bytes.len() > CHECKSUM_RANGE.end {
        hasher.update(&bytes[CHECKSUM_RANGE.end..]);
    }
    hasher.finalize().into()
}

fn normalize(value: Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.into_iter().map(normalize).collect()),
        Value::Object(values) => {
            let sorted = values
                .into_iter()
                .map(|(key, value)| (key, normalize(value)))
                .collect::<BTreeMap<_, _>>();
            let mut output = Map::new();
            output.extend(sorted);
            Value::Object(output)
        }
        Value::Number(number)
            if number
                .as_f64()
                .is_some_and(|value| value == 0.0 && value.is_sign_negative()) =>
        {
            Value::Number(Number::from_f64(0.0).expect("zero is finite"))
        }
        other => other,
    }
}

fn encode_value(
    value: &Value,
    output: &mut Vec<u8>,
    depth: usize,
    contract: Contract,
) -> Result<(), PackedPlanError> {
    if depth > MAX_DEPTH {
        return Err(PackedPlanError::InvalidValue {
            kind: contract.kind,
            reason: "value nesting exceeds the depth budget".into(),
        });
    }
    match value {
        Value::Null => output.push(NULL),
        Value::Bool(false) => output.push(FALSE),
        Value::Bool(true) => output.push(TRUE),
        Value::Number(number) => {
            if let Some(value) = number.as_i64() {
                output.push(I64);
                output.extend_from_slice(&value.to_le_bytes());
            } else if let Some(value) = number.as_u64() {
                output.push(U64);
                output.extend_from_slice(&value.to_le_bytes());
            } else if let Some(mut value) = number.as_f64() {
                if !value.is_finite() {
                    return Err(PackedPlanError::InvalidValue {
                        kind: contract.kind,
                        reason: "non-finite number".into(),
                    });
                }
                if value == 0.0 {
                    value = 0.0;
                }
                output.push(F64);
                output.extend_from_slice(&value.to_bits().to_le_bytes());
            } else {
                return Err(PackedPlanError::InvalidValue {
                    kind: contract.kind,
                    reason: "unrepresentable number".into(),
                });
            }
        }
        Value::String(value) => {
            output.push(STRING);
            write_bytes(value.as_bytes(), output, contract)?;
        }
        Value::Array(values) => {
            check_count(values.len(), contract)?;
            output.push(ARRAY);
            write_len(values.len(), output, contract)?;
            for value in values {
                encode_value(value, output, depth + 1, contract)?;
            }
        }
        Value::Object(values) => {
            check_count(values.len(), contract)?;
            output.push(OBJECT);
            write_len(values.len(), output, contract)?;
            for (key, value) in values {
                write_bytes(key.as_bytes(), output, contract)?;
                encode_value(value, output, depth + 1, contract)?;
            }
        }
    }
    if output.len().saturating_add(HEADER_LEN) > contract.maximum {
        return Err(PackedPlanError::ByteBudget {
            kind: contract.kind,
            actual: output.len().saturating_add(HEADER_LEN),
            maximum: contract.maximum,
        });
    }
    Ok(())
}

fn write_bytes(
    bytes: &[u8],
    output: &mut Vec<u8>,
    contract: Contract,
) -> Result<(), PackedPlanError> {
    if bytes.len() > MAX_STRING_BYTES {
        return Err(PackedPlanError::InvalidValue {
            kind: contract.kind,
            reason: "string exceeds byte budget".into(),
        });
    }
    write_len(bytes.len(), output, contract)?;
    output.extend_from_slice(bytes);
    Ok(())
}

fn write_len(
    length: usize,
    output: &mut Vec<u8>,
    contract: Contract,
) -> Result<(), PackedPlanError> {
    let length = u32::try_from(length).map_err(|_| PackedPlanError::InvalidValue {
        kind: contract.kind,
        reason: "length exceeds u32".into(),
    })?;
    output.extend_from_slice(&length.to_le_bytes());
    Ok(())
}

fn check_count(count: usize, contract: Contract) -> Result<(), PackedPlanError> {
    if count > MAX_CONTAINER_ITEMS {
        Err(PackedPlanError::InvalidValue {
            kind: contract.kind,
            reason: "container exceeds item budget".into(),
        })
    } else {
        Ok(())
    }
}

struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
    values: usize,
    contract: Contract,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8], contract: Contract) -> Self {
        Self {
            bytes,
            offset: 0,
            values: 0,
            contract,
        }
    }

    fn is_empty(&self) -> bool {
        self.offset == self.bytes.len()
    }

    fn invalid(&self, reason: impl Into<String>) -> PackedPlanError {
        PackedPlanError::InvalidValue {
            kind: self.contract.kind,
            reason: reason.into(),
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, PackedPlanError> {
        if depth > MAX_DEPTH {
            return Err(self.invalid("value nesting exceeds the depth budget"));
        }
        self.values = self
            .values
            .checked_add(1)
            .ok_or_else(|| self.invalid("aggregate value count overflow"))?;
        if self.values > MAX_TOTAL_VALUES {
            return Err(self.invalid("aggregate value count exceeds budget"));
        }
        let tag = self.byte()?;
        match tag {
            NULL => Ok(Value::Null),
            FALSE => Ok(Value::Bool(false)),
            TRUE => Ok(Value::Bool(true)),
            I64 => Ok(Value::Number(Number::from(self.i64()?))),
            U64 => Ok(Value::Number(Number::from(self.u64()?))),
            F64 => {
                let bits = self.u64()?;
                let value = f64::from_bits(bits);
                if !value.is_finite() || (value == 0.0 && value.is_sign_negative()) {
                    return Err(self.invalid("float is non-finite or negative zero"));
                }
                Number::from_f64(value)
                    .map(Value::Number)
                    .ok_or_else(|| self.invalid("float is not representable"))
            }
            STRING => self.string().map(Value::String),
            ARRAY => {
                let length = self.length()?;
                check_count(length, self.contract)?;
                // Do not trust an untrusted wire length as an allocation request. Growth remains
                // bounded by successfully decoded values and the aggregate value budget.
                let mut values = Vec::with_capacity(length.min(4096));
                for _ in 0..length {
                    values.push(self.value(depth + 1)?);
                }
                Ok(Value::Array(values))
            }
            OBJECT => {
                let length = self.length()?;
                check_count(length, self.contract)?;
                let mut values = Map::new();
                let mut previous: Option<String> = None;
                for _ in 0..length {
                    let key = self.string()?;
                    if previous.as_ref().is_some_and(|previous| previous >= &key) {
                        return Err(self.invalid("object keys are duplicated or not canonical"));
                    }
                    previous = Some(key.clone());
                    values.insert(key, self.value(depth + 1)?);
                }
                Ok(Value::Object(values))
            }
            other => Err(self.invalid(format!("unknown value tag {other}"))),
        }
    }

    fn byte(&mut self) -> Result<u8, PackedPlanError> {
        let value = self
            .bytes
            .get(self.offset)
            .copied()
            .ok_or_else(|| self.invalid("unexpected end of payload"))?;
        self.offset += 1;
        Ok(value)
    }

    fn take<const N: usize>(&mut self) -> Result<[u8; N], PackedPlanError> {
        let end = self
            .offset
            .checked_add(N)
            .ok_or_else(|| self.invalid("payload range overflow"))?;
        let bytes = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(|| self.invalid("unexpected end of payload"))?;
        self.offset = end;
        Ok(bytes.try_into().expect("fixed length"))
    }

    fn i64(&mut self) -> Result<i64, PackedPlanError> {
        Ok(i64::from_le_bytes(self.take()?))
    }

    fn u64(&mut self) -> Result<u64, PackedPlanError> {
        Ok(u64::from_le_bytes(self.take()?))
    }

    fn length(&mut self) -> Result<usize, PackedPlanError> {
        Ok(u32::from_le_bytes(self.take()?) as usize)
    }

    fn string(&mut self) -> Result<String, PackedPlanError> {
        let length = self.length()?;
        if length > MAX_STRING_BYTES {
            return Err(self.invalid("string exceeds byte budget"));
        }
        let end = self
            .offset
            .checked_add(length)
            .ok_or_else(|| self.invalid("string range overflow"))?;
        let bytes = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(|| self.invalid("string extends beyond payload"))?;
        self.offset = end;
        std::str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|_| self.invalid("string is not UTF-8"))
    }
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    use super::*;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Probe {
        z: f64,
        a: Vec<String>,
    }

    #[test]
    fn envelope_round_trip_is_canonical_and_checked() {
        let contract = Contract::plan(1);
        let probe = Probe {
            z: -0.0,
            a: vec!["x".into(), "y".into()],
        };
        let bytes = encode(&probe, contract).unwrap();
        assert_eq!(
            decode::<Probe>(&bytes, contract).unwrap(),
            Probe { z: 0.0, a: probe.a }
        );

        let mut corrupt = bytes.clone();
        *corrupt.last_mut().unwrap() ^= 1;
        assert!(matches!(
            decode::<Probe>(&corrupt, contract),
            Err(PackedPlanError::ChecksumMismatch { .. })
        ));
    }

    #[test]
    fn envelope_rejects_a_wrong_exact_format_version_with_a_valid_checksum() {
        let probe = Probe {
            z: 1.0,
            a: vec!["version".into()],
        };
        for contract in [
            Contract::plan(1),
            Contract::bindings(1),
            Contract::requests(1),
            Contract::schedule(1),
        ] {
            let mut bytes = encode(&probe, contract).unwrap();
            bytes[8..12].copy_from_slice(&2_u32.to_le_bytes());
            let digest = checksum(&bytes);
            bytes[CHECKSUM_RANGE].copy_from_slice(&digest);

            assert!(matches!(
                decode::<Probe>(&bytes, contract),
                Err(PackedPlanError::UnsupportedFormatVersion {
                    found: 2,
                    expected: 1,
                    ..
                })
            ));
        }
    }

    #[test]
    fn decoder_rejects_declared_container_budget_before_allocating_it() {
        let contract = Contract::bindings(1);
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&contract.magic);
        bytes.extend_from_slice(&contract.version.to_le_bytes());
        bytes.extend_from_slice(&ENDIAN_MARKER.to_le_bytes());
        bytes.extend_from_slice(&((HEADER_LEN + 5) as u64).to_le_bytes());
        bytes.extend_from_slice(&[0; 32]);
        bytes.push(ARRAY);
        bytes.extend_from_slice(&u32::MAX.to_le_bytes());
        let checksum = checksum(&bytes);
        bytes[CHECKSUM_RANGE].copy_from_slice(&checksum);
        assert!(matches!(
            decode::<Vec<u8>>(&bytes, contract),
            Err(PackedPlanError::InvalidValue { .. })
        ));
    }
}
