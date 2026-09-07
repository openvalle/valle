fn main() {
    println!("cargo:rerun-if-env-changed=RUSTFLAGS");
    println!("cargo:rerun-if-env-changed=CARGO_ENCODED_RUSTFLAGS");

    for name in ["RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS"] {
        let Ok(flags) = std::env::var(name) else {
            continue;
        };
        let flags = flags.to_ascii_lowercase();
        for forbidden in ["fast-math", "unsafe-fp-math", "fp-contract=fast"] {
            assert!(
                !flags.contains(forbidden),
                "{name} enables `{forbidden}`; Valle render semantics require strict floating-point math"
            );
        }
    }
}
