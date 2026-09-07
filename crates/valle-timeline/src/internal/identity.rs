//! The public execution identity used by the Timeline pipeline.
//!
//! Every identity is a SHA-256 digest with an explicit domain tag.  The wire
//! form is always `sha256:<64 lowercase hex>`; accepting bare or uppercase
//! digests would create a second spelling for the same identity.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

pub const RENDER_ID_DOMAIN: &[u8] = b"valle.render/1\0";

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum IdentityParseError {
    #[error("identity must use the `sha256:` prefix")]
    MissingSha256Prefix,
    #[error("sha256 identity must contain exactly 64 lowercase hexadecimal digits")]
    InvalidDigest,
}

fn hash_domain(domain: &[u8], canonical_bytes: &[u8]) -> [u8; 32] {
    let mut payload = Vec::with_capacity(domain.len() + canonical_bytes.len());
    payload.extend_from_slice(domain);
    payload.extend_from_slice(canonical_bytes);
    crate::internal::hash::sha256(&payload)
}

fn parse_wire(value: &str) -> Result<[u8; 32], IdentityParseError> {
    let hex = value
        .strip_prefix("sha256:")
        .ok_or(IdentityParseError::MissingSha256Prefix)?;
    if hex.len() != 64
        || !hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(IdentityParseError::InvalidDigest);
    }

    let mut digest = [0_u8; 32];
    for (index, pair) in hex.as_bytes().chunks_exact(2).enumerate() {
        digest[index] = (nibble(pair[0]) << 4) | nibble(pair[1]);
    }
    Ok(digest)
}

const fn nibble(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        _ => 0,
    }
}

fn format_wire(digest: &[u8; 32], formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    formatter.write_str("sha256:")?;
    for byte in digest {
        write!(formatter, "{byte:02x}")?;
    }
    Ok(())
}

macro_rules! identity_type {
    ($(#[$meta:meta])* $name:ident, $domain:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name([u8; 32]);

        impl $name {
            pub const fn from_bytes(bytes: [u8; 32]) -> Self {
                Self(bytes)
            }

            pub fn from_canonical_bytes(bytes: &[u8]) -> Self {
                Self(hash_domain($domain, bytes))
            }

            pub const fn as_bytes(&self) -> &[u8; 32] {
                &self.0
            }

            pub fn parse(value: &str) -> Result<Self, IdentityParseError> {
                parse_wire(value).map(Self)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                format_wire(&self.0, formatter)
            }
        }

        impl std::str::FromStr for $name {
            type Err = IdentityParseError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::parse(value)
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.collect_str(self)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::parse(&value).map_err(de::Error::custom)
            }
        }
    };
}

identity_type!(
    /// Hash of the complete fixed render identity projection.
    RenderId,
    RENDER_ID_DOMAIN
);
