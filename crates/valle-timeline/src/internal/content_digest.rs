use std::{fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

/// SHA-256 digest of one exact byte sequence.
///
/// The authoritative Rust representation is the 32 digest bytes. JSON and
/// protocol boundaries have one spelling, `sha256:<64 lowercase hex>`. Bare
/// hexadecimal is exposed only through [`Self::as_hex`] for content-addressed
/// paths whose surrounding format already fixes the algorithm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContentDigest([u8; 32]);

impl ContentDigest {
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn of_bytes(bytes: &[u8]) -> Self {
        Self(super::hash::sha256(bytes))
    }

    /// Decode bare lowercase hexadecimal at an internal hash/CAS boundary.
    /// JSON and other protocol input must use [`Self::parse`] instead.
    pub fn from_hex(value: &str) -> Result<Self, InvalidContentDigest> {
        if value.len() != 64 {
            return Err(InvalidContentDigest);
        }

        let mut bytes = [0_u8; 32];
        for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
            let high = lowercase_hex_nibble(pair[0]).ok_or(InvalidContentDigest)?;
            let low = lowercase_hex_nibble(pair[1]).ok_or(InvalidContentDigest)?;
            bytes[index] = (high << 4) | low;
        }
        Ok(Self(bytes))
    }

    /// Decode the one protocol spelling, `sha256:<64 lowercase hex>`.
    pub fn parse(value: &str) -> Result<Self, InvalidContentDigest> {
        let hex = value.strip_prefix("sha256:").ok_or(InvalidContentDigest)?;
        Self::from_hex(hex)
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Encode bare lowercase hexadecimal for an explicitly content-addressed
    /// path or hash-algorithm boundary.
    pub fn as_hex(&self) -> String {
        let mut value = String::with_capacity(64);
        for byte in self.0 {
            use fmt::Write as _;
            write!(&mut value, "{byte:02x}").expect("writing to String cannot fail");
        }
        value
    }

    /// Encode the one protocol spelling.
    pub fn to_wire(&self) -> String {
        format!("sha256:{}", self.as_hex())
    }
}

impl fmt::Display for ContentDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_wire())
    }
}

impl FromStr for ContentDigest {
    type Err = InvalidContentDigest;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl Serialize for ContentDigest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_wire())
    }
}

impl<'de> Deserialize<'de> for ContentDigest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(de::Error::custom)
    }
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("invalid SHA-256 content digest")]
pub struct InvalidContentDigest;

const fn lowercase_hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}
