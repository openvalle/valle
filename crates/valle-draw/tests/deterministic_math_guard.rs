use std::path::{Path, PathBuf};

const FORBIDDEN: &[&str] = &[
    ".sin(",
    ".cos(",
    ".sin_cos(",
    ".tan(",
    ".asin(",
    ".acos(",
    ".atan(",
    ".atan2(",
    ".exp(",
    ".exp2(",
    ".ln(",
    ".log(",
    ".log2(",
    ".log10(",
    ".powf(",
    ".powi(",
    ".sqrt(",
    ".cbrt(",
    ".hypot(",
];

fn rust_files(root: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(root).expect("read source directory") {
        let path = entry.expect("source entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn render_semantics_cannot_bypass_the_deterministic_math_narrow_waist() {
    let draw = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    // Check deterministic math in layout, geometry and expression evaluation. Raster output is covered by pixel-tolerance tests.
    let motion = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../valle-motion/src");
    let compiler = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../valle-compiler/src");
    // Reference compositor transfer and blend math must preserve exact Native/Wasm corpus bits.
    let compositor_reference =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../valle-engine/src/compositor/reference");
    let mut files = Vec::new();
    rust_files(&draw, &mut files);
    rust_files(&motion, &mut files);
    rust_files(&compiler, &mut files);
    rust_files(&compositor_reference, &mut files);

    let math_module = draw.join("math.rs");
    let mut violations = Vec::new();
    for path in files {
        if path == math_module {
            continue;
        }
        let source = std::fs::read_to_string(&path).expect("read Rust source");
        // Tests may use the platform implementation as an independent oracle. Only production
        // code before the conventional trailing `#[cfg(test)]` module is governed here.
        let production = source.split("#[cfg(test)]").next().unwrap_or(&source);
        for (line_index, line) in production.lines().enumerate() {
            for forbidden in FORBIDDEN {
                if line.contains(forbidden) {
                    violations.push(format!(
                        "{}:{} contains `{forbidden}`",
                        path.display(),
                        line_index + 1
                    ));
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "render math must call valle_draw::math, not platform methods:\n{}",
        violations.join("\n")
    );
}
