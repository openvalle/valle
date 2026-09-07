//! Formula dependency boundaries and the portable WASM build contract.
use std::{fs, path::PathBuf, process::Command};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn product_crate_does_not_depend_on_ratex_renderer_or_embedded_fonts() {
    let manifest = fs::read_to_string(repo_root().join("crates/valle-motion/Cargo.toml")).unwrap();
    let dep_lines: Vec<&str> = manifest
        .lines()
        .filter(|line| {
            let trimmed = line.trim_start();
            !trimmed.starts_with('#') && trimmed.contains('=')
        })
        .collect();
    for forbidden in [
        "ratex-render",
        "ratex-svg",
        "ratex-pdf",
        "ratex-katex-fonts",
        "ratex-cairo",
        "ratex-ffi",
        "ratex-unicode-font",
    ] {
        assert!(
            dep_lines.iter().all(|line| !line.contains(forbidden)),
            "valle-motion must not depend on {forbidden}"
        );
    }
    assert!(manifest.contains("ratex-layout"));
    assert!(manifest.contains("ratex-parser"));
}

#[test]
fn wasm32_unknown_unknown_builds_ratex_core_through_layout() {
    let manifest = repo_root().join("Cargo.toml");
    let status = Command::new("rustup")
        .args(["run", "stable", "cargo", "check", "--manifest-path"])
        .arg(&manifest)
        .args([
            "-p",
            "valle-motion",
            "--target",
            "wasm32-unknown-unknown",
            "--offline",
        ])
        .status();
    let status = match status {
        Ok(status) => status,
        Err(err) => {
            eprintln!("skip wasm rustup: {err}");
            return;
        }
    };
    if !status.success() {
        let retry = Command::new("cargo")
            .args(["check", "--manifest-path"])
            .arg(&manifest)
            .args(["-p", "valle-motion", "--target", "wasm32-unknown-unknown"])
            .status()
            .expect("cargo wasm check");
        assert!(
            retry.success(),
            "valle-motion must be wasm32-clean with RaTeX core"
        );
    }
}
