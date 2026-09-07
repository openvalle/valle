#[cfg(feature = "schema")]
fn main() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml");
    println!("cargo:rerun-if-changed={}", manifest.display());
    let source = std::fs::read_to_string(manifest).expect("workspace Cargo.toml must exist");
    let workspace: toml::Value = toml::from_str(&source).expect("workspace manifest must parse");
    let version = workspace["workspace"]["dependencies"]["schemars"]
        .as_str()
        .and_then(|version| version.strip_prefix('='))
        .expect("schema generation requires an exact workspace schemars pin");
    println!("cargo:rustc-env=VALLE_SCHEMARS_VERSION={version}");
}

#[cfg(not(feature = "schema"))]
fn main() {}
