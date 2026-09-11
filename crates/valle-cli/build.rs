use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    env, fs,
    path::{Component, Path, PathBuf},
};

fn main() {
    println!("cargo:rerun-if-env-changed=VALLE_EMBED_WEB_DIR");
    println!("cargo:rerun-if-env-changed=VALLE_EMBED_NOTICES");
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    if env::var_os("CARGO_FEATURE_EMBEDDED_RUNTIME").is_none() {
        for file in ["LICENSE", "THIRD_PARTY.md"] {
            println!("cargo:rerun-if-changed={}", root.join(file).display());
        }
        let notices = format!(
            "Development build. Use `cargo xtask build` for complete bundled dependency notices.\n\n{}\n{}",
            fs::read_to_string(root.join("LICENSE")).unwrap(),
            fs::read_to_string(root.join("THIRD_PARTY.md")).unwrap()
        );
        fs::write(out.join("embedded.rs"), format!("pub const WEB_FILES: &[(&str, &[u8])] = &[];\npub const NOTICES: &str = {notices:?};\n")).unwrap();
        return;
    }
    let web = PathBuf::from(
        env::var_os("VALLE_EMBED_WEB_DIR")
            .expect("build embedded-runtime with `cargo xtask build`"),
    );
    let notices = PathBuf::from(
        env::var_os("VALLE_EMBED_NOTICES")
            .expect("missing generated notices; use `cargo xtask build`"),
    );
    let manifest = web.join("runtime/manifest.json");
    println!("cargo:rerun-if-changed={}", manifest.display());
    let build: serde_json::Value = serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    assert_eq!(
        build["runtimeVersion"].as_str(),
        Some(env!("CARGO_PKG_VERSION")),
        "Web/CLI version mismatch"
    );
    let mut files = BTreeSet::from(["runtime/manifest.json".to_owned()]);
    for asset in build["assets"].as_array().unwrap() {
        let relative = asset["path"].as_str().unwrap();
        let data = read(&web, relative);
        assert_eq!(
            data.len() as u64,
            asset["bytes"].as_u64().unwrap(),
            "Web asset size mismatch: {relative}"
        );
        assert_eq!(
            hex::encode(Sha256::digest(&data)),
            asset["sha256"].as_str().unwrap(),
            "Web asset hash mismatch: {relative}"
        );
        assert!(
            files.insert(relative.to_owned()),
            "duplicate Web asset: {relative}"
        );
    }
    for package in build["packages"].as_array().unwrap() {
        assert!(
            files.insert(package["manifest"].as_str().unwrap().to_owned()),
            "duplicate package manifest"
        );
    }
    // Snapshot the admitted files before rustc reads them. No runtime extraction or custom archive format.
    let mut generated = String::from("pub const WEB_FILES: &[(&str, &[u8])] = &[\n");
    for relative in files {
        let data = read(&web, &relative);
        let target = out.join("web").join(&relative);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, data).unwrap();
        generated.push_str(&format!(
            "({relative:?}, include_bytes!({:?})),\n",
            target.to_str().unwrap()
        ));
    }
    println!("cargo:rerun-if-changed={}", notices.display());
    let target = out.join("notices.txt");
    fs::copy(notices, &target).unwrap();
    generated.push_str(&format!(
        "];\npub const NOTICES: &str = include_str!({:?});\n",
        target.to_str().unwrap()
    ));
    fs::write(out.join("embedded.rs"), generated).unwrap();
}

fn read(root: &Path, relative: &str) -> Vec<u8> {
    assert!(
        !relative.is_empty()
            && !relative.contains('\\')
            && relative
                .split('/')
                .all(|part| !part.is_empty() && part != "." && part != ".."),
        "unsafe embedded asset path"
    );
    let mut path = root.to_path_buf();
    for part in Path::new(relative).components() {
        assert!(
            matches!(part, Component::Normal(_)),
            "unsafe embedded asset path"
        );
        path.push(part);
        assert!(
            !fs::symlink_metadata(&path).unwrap().is_symlink(),
            "embedded assets cannot contain symlinks"
        );
    }
    assert!(path.is_file(), "embedded asset must be a regular file");
    println!("cargo:rerun-if-changed={}", path.display());
    fs::read(path).unwrap()
}
