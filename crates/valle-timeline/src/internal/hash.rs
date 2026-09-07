//! SHA-256 implementation boundary shared by `ContentDigest` and `RenderId`.
//!
//! `sha2` is already the workspace implementation and supports the Timeline
//! crate's native and wasm targets. Keeping this wrapper gives callers one
//! digest primitive without maintaining a second cryptographic implementation.

use sha2::{Digest, Sha256};

/// Compute the 32-byte SHA-256 digest of `data`.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

/// Lowercase 64-character SHA-256 representation.
#[cfg(test)]
fn sha256_hex(data: &[u8]) -> String {
    let digest = sha256(data);
    let mut s = String::with_capacity(64);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for b in digest {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0f) as usize] as char);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nist_vectors() {
        // Standard SHA-256 test vectors.
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn spans_multiple_blocks() {
        // Verify hashing across multiple blocks with the standard million-byte vector.
        let data = vec![b'a'; 1_000_000];
        assert_eq!(
            sha256_hex(&data),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    #[test]
    fn hex_is_lowercase_64_chars() {
        let h = sha256_hex(b"valle");
        assert_eq!(h.len(), 64);
        assert!(
            h.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
    }
}
