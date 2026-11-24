//! The models media commands run, installed under `~/.valle/models` (`$VALLE_MODELS_DIR` names
//! another folder) as `<id>/<revision>/<file>`, one folder for each revision of each model.
//!
//! A file is checked against the catalog's SHA-256 before it gets its name, so a file with its
//! name and size was checked when it was written: `installed` compares sizes only. Files are
//! written as `<file>.part` first, and a lock file keeps two processes from installing the same
//! revision at once.

use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use ring::digest::{Context, SHA256};
pub use valle_asr::models::{ModelFile, ModelSpec};

/// The model that turns speech into text.
pub const ASR: &str = "qwen3-asr-0.6b";
/// The model that times each word of a transcript.
pub const ALIGNER: &str = "qwen3-forced-aligner-0.6b";

/// Every model media commands can run, by ID.
pub fn catalog() -> Vec<ModelSpec> {
    valle_asr::models::builtin_models()
}

/// The model with ID `id`.
pub fn find(id: &str) -> Result<ModelSpec, String> {
    let models = catalog();
    let ids = models
        .iter()
        .map(|m| m.id.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    models
        .iter()
        .find(|m| m.id == id)
        .cloned()
        .ok_or_else(|| format!("no model {id}; the models are {ids}"))
}

/// What a model is for, in a few words.
pub fn purpose(spec: &ModelSpec) -> &'static str {
    match spec.id.as_str() {
        ASR => "speech to text",
        ALIGNER => "word timestamps",
        _ => "",
    }
}

/// The size of all of a model's files, in bytes.
pub fn size(spec: &ModelSpec) -> u64 {
    spec.files.iter().map(|f| f.bytes).sum()
}

/// The folder models are installed in: `$VALLE_MODELS_DIR`, or `~/.valle/models`.
pub fn root() -> Result<PathBuf, String> {
    Ok(match std::env::var_os("VALLE_MODELS_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => std::env::home_dir()
            .ok_or("no home directory to keep models in; set VALLE_MODELS_DIR")?
            .join(".valle")
            .join("models"),
    })
}

/// The folder of a model's revision under `root`.
pub fn dir(root: &Path, spec: &ModelSpec) -> PathBuf {
    root.join(&spec.id).join(&spec.revision)
}

/// Whether every file of a model is installed under `root`.
pub fn installed(root: &Path, spec: &ModelSpec) -> bool {
    spec.missing(dir(root, spec)).is_empty()
}

/// Installs a model from the folder `from`, which holds its files, as a model downloaded before
/// does: a Hugging Face or ModelScope snapshot, or a plain folder. Each file is checked against
/// the catalog, then hard-linked into place, or copied where it cannot be: on another disk, or
/// where links are not supported. Links share the file, so it must not be changed in place.
pub fn import(root: &Path, spec: &ModelSpec, from: &Path) -> Result<PathBuf, String> {
    // Every file is there at its size before any is placed, so a wrong folder places nothing.
    let mut problems = Vec::new();
    let mut sources = Vec::new();
    for file in &spec.files {
        let path = from.join(&file.path);
        // Snapshots link their files to blobs elsewhere: the link is followed to the file.
        match fs::metadata(&path) {
            // Linux links a link itself, not its file.
            Ok(m) if m.is_file() && m.len() == file.bytes => sources.push(
                path.canonicalize()
                    .map_err(|e| format!("{}: {e}", path.display()))?,
            ),
            Ok(m) if m.is_file() => problems.push(format!(
                "{} is {} bytes, not {}",
                path.display(),
                m.len(),
                file.bytes
            )),
            _ => problems.push(format!("{} is missing", path.display())),
        }
    }
    if !problems.is_empty() {
        return Err(format!(
            "{} is not {} at revision {}: {}",
            from.display(),
            spec.id,
            spec.revision,
            problems.join("; ")
        ));
    }
    let target = dir(root, spec);
    let _lock = lock(root, spec)?;
    for (file, source) in spec.files.iter().zip(sources) {
        let path = target.join(&file.path);
        if fs::metadata(&path).is_ok_and(|m| m.is_file() && m.len() == file.bytes) {
            continue;
        }
        let part = part(&path);
        let _ = fs::remove_file(&part);
        if fs::hard_link(&source, &part).is_err() {
            fs::copy(&source, &part).map_err(|e| format!("{}: {e}", source.display()))?;
        }
        // What is checked is what is placed, linked or copied.
        place(&part, &path, file).map_err(|e| format!("{}: {e}", source.display()))?;
    }
    Ok(target)
}

/// Holds the lock on installing a model's revision under `root` until it is dropped.
pub(crate) fn lock(root: &Path, spec: &ModelSpec) -> Result<File, String> {
    let target = dir(root, spec);
    fs::create_dir_all(&target).map_err(|e| format!("{}: {e}", target.display()))?;
    let path = root.join(&spec.id).join(format!("{}.lock", spec.revision));
    File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .and_then(|file| file.lock().map(|()| file))
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// Where a file is written before it is checked: `<file>.part` beside it.
pub(crate) fn part(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    path.with_file_name(name)
}

/// Gives `part` the name `path` if it is `file`, by size and SHA-256; otherwise removes it.
pub(crate) fn place(part: &Path, path: &Path, file: &ModelFile) -> Result<(), String> {
    let checked = sha256(part).and_then(|sha256| {
        if sha256 == file.sha256 {
            Ok(())
        } else {
            Err(format!("its SHA-256 is {sha256}, not {}", file.sha256))
        }
    });
    if let Err(e) = checked {
        let _ = fs::remove_file(part);
        return Err(e);
    }
    fs::rename(part, path).map_err(|e| format!("{}: {e}", path.display()))
}

/// The SHA-256 of the file at `path`, in hex.
pub(crate) fn sha256(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut digest = Context::new(&SHA256);
    let mut buffer = vec![0; 1 << 20];
    loop {
        match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => digest.update(&buffer[..read]),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        }
    }
    Ok(digest
        .finish()
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub struct Folder(pub PathBuf);

    impl Folder {
        pub fn new(name: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("valle-media-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// A model of two small files, `a.bin` and `b.bin`.
    pub fn spec() -> ModelSpec {
        let file = |path: &str, bytes: &[u8]| ModelFile {
            path: path.into(),
            bytes: bytes.len() as u64,
            sha256: ring::digest::digest(&SHA256, bytes)
                .as_ref()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect(),
        };
        ModelSpec {
            id: "test-model".into(),
            repository: "test/model".into(),
            revision: "0123abcd".into(),
            license: "Apache-2.0".into(),
            files: vec![file("a.bin", b"first file"), file("b.bin", b"second")],
        }
    }

    #[test]
    fn the_catalog_has_both_models_media_runs() {
        for id in [ASR, ALIGNER] {
            let spec = find(id).unwrap();
            assert!(!purpose(&spec).is_empty());
            assert!(size(&spec) > 1_000_000_000);
        }
        let e = find("nope").unwrap_err();
        assert!(e.contains(ASR) && e.contains(ALIGNER), "{e}");
    }

    #[test]
    fn a_folder_with_every_file_is_imported_and_checked() {
        let root = Folder::new("import-root");
        let from = Folder::new("import-from");
        let spec = spec();
        assert!(!installed(&root.0, &spec));

        fs::write(from.0.join("a.bin"), b"first file").unwrap();
        let e = import(&root.0, &spec, &from.0).unwrap_err();
        assert!(e.contains("b.bin is missing"), "{e}");
        assert!(!dir(&root.0, &spec).join("a.bin").exists());

        // The right size with other bytes is refused, and leaves no part behind.
        fs::write(from.0.join("b.bin"), b"SECOND").unwrap();
        let e = import(&root.0, &spec, &from.0).unwrap_err();
        assert!(e.contains("b.bin") && e.contains("SHA-256"), "{e}");
        assert!(!installed(&root.0, &spec));
        assert!(!part(&dir(&root.0, &spec).join("b.bin")).exists());

        fs::write(from.0.join("b.bin"), b"second").unwrap();
        let target = import(&root.0, &spec, &from.0).unwrap();
        assert_eq!(target, root.0.join("test-model").join("0123abcd"));
        assert!(installed(&root.0, &spec));
        assert_eq!(fs::read(target.join("b.bin")).unwrap(), b"second");
        // Again, it finds the files in place.
        import(&root.0, &spec, &from.0).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn snapshot_links_are_followed_to_their_files() {
        let root = Folder::new("link-root");
        let from = Folder::new("link-from");
        let blobs = from.0.join("blobs");
        let snapshot = from.0.join("snapshots").join("0123abcd");
        fs::create_dir_all(&blobs).unwrap();
        fs::create_dir_all(&snapshot).unwrap();
        for (name, bytes) in [("a.bin", &b"first file"[..]), ("b.bin", b"second")] {
            fs::write(blobs.join(name), bytes).unwrap();
            std::os::unix::fs::symlink(blobs.join(name), snapshot.join(name)).unwrap();
        }
        let target = import(&root.0, &spec(), &snapshot).unwrap();
        let placed = fs::symlink_metadata(target.join("a.bin")).unwrap();
        assert!(placed.is_file());
        assert_eq!(fs::read(target.join("a.bin")).unwrap(), b"first file");
    }
}
