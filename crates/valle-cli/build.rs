//! A package embeds the already-built web assets. Ordinary development builds read them from
//! disk, without compiling the compressor or requiring a browser build.

#[cfg(not(feature = "embed-web"))]
fn main() {}

#[cfg(feature = "embed-web")]
fn main() -> std::io::Result<()> {
    use std::fs;
    use std::path::{Path, PathBuf};

    fn files(dir: &Path, found: &mut Vec<PathBuf>) -> std::io::Result<()> {
        println!("cargo:rerun-if-changed={}", dir.display());
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                files(&entry.path(), found)?;
            } else if entry.file_type()?.is_file() {
                found.push(entry.path());
            }
        }
        Ok(())
    }

    println!("cargo:rerun-if-changed=build.rs");
    let web = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web");
    for required in [
        "apps/studio/dist/index.html",
        "packages/engine/dist/index.js",
        "packages/engine/dist/valle_web_bg.wasm",
    ] {
        if !web.join(required).is_file() {
            return Err(std::io::Error::other(format!(
                "{} is missing; run cargo xtask build-web before building with embed-web",
                web.join(required).display()
            )));
        }
    }
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo sets OUT_DIR"));
    let mut source = String::from("&[\n");
    let (mut raw_size, mut br_size) = (0, 0);
    let params = brotli::enc::BrotliEncoderParams {
        quality: 11,
        lgwin: 22,
        ..Default::default()
    };
    for (prefix, dir) in [("", "apps/studio/dist"), ("valle/", "packages/engine/dist")] {
        let dir = web.join(dir);
        let mut found = Vec::new();
        files(&dir, &mut found)?;
        found.sort();
        for path in found {
            println!("cargo:rerun-if-changed={}", path.display());
            let name = path.strip_prefix(&dir).expect("inside the web build");
            let name = format!("{prefix}{}", name.to_string_lossy().replace('\\', "/"));
            let raw = fs::read(&path)?;
            let mut compressed = Vec::new();
            brotli::BrotliCompress(&mut raw.as_slice(), &mut compressed, &params)?;
            raw_size += raw.len();
            br_size += compressed.len();
            let dest = out.join(format!("{name}.br"));
            fs::create_dir_all(dest.parent().expect("inside OUT_DIR"))?;
            fs::write(&dest, compressed)?;
            source.push_str(&format!("    ({name:?}, include_bytes!({:?})),\n", dest));
        }
    }
    source.push_str("]\n");
    fs::write(out.join("web.rs"), source)?;
    println!("embedded web: {raw_size} bytes -> {br_size} bytes (Brotli)");
    Ok(())
}
