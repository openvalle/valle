//! Native hash printer and WebAssembly export for the shared numerical corpus.
use sha2::{Digest, Sha256};
use std::sync::OnceLock;

fn digest() -> &'static [u8; 32] {
    static DIGEST: OnceLock<[u8; 32]> = OnceLock::new();
    DIGEST.get_or_init(|| {
        Sha256::digest(serde_json::to_vec(&valle_draw::math::determinism_corpus()).unwrap()).into()
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn deterministic_math_hash_byte(index: u32) -> u32 {
    digest()
        .get(index as usize)
        .copied()
        .map(u32::from)
        .unwrap_or(0)
}

fn main() {
    #[cfg(not(target_arch = "wasm32"))]
    println!("{}", hex::encode(digest()));
}
