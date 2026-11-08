//! Materialize the test fonts from the same pinned definitions and cache as the native host.

use std::fs;
use std::path::Path;
use std::sync::OnceLock;

use valle_engine::text::{BUILT_IN, BuiltIn};
use valle_native::urls::UrlCache;

/// Shared by scene tests and the browser capture preparation command.
pub fn prepare() -> Result<(), String> {
    static READY: OnceLock<Result<(), String>> = OnceLock::new();
    READY
        .get_or_init(|| {
            let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/resources");
            let cache = UrlCache::global()?;
            for file in ["NotoSans.ttf", "NotoSansItalic.ttf", "NotoSansSC.ttf"] {
                let font = BUILT_IN
                    .iter()
                    .find(|font| font.file == file)
                    .ok_or_else(|| format!("test font {file} is not a built-in font"))?;
                materialize(font, &cache, &dir)?;
            }
            Ok(())
        })
        .clone()
}

fn materialize(font: &BuiltIn, cache: &UrlCache, dir: &Path) -> Result<(), String> {
    let to = dir.join(font.file);
    match fs::read(&to) {
        Ok(bytes) if sha256(&bytes) == font.sha256 => return Ok(()),
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("{}: {e}", to.display())),
    }
    let from = cache.fetch_pinned(font.url, font.sha256, font.file)?;
    let bytes = fs::read(&from).map_err(|e| format!("{}: {e}", from.display()))?;
    // A pinned download is verified by UrlCache, but a cached file may have been damaged.
    if sha256(&bytes) != font.sha256 {
        return Err(format!(
            "{} does not match the pinned font; remove the corrupt cache file and retry",
            from.display()
        ));
    }
    fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    // Publish whole files, including when another test process is preparing the same fonts.
    let part = to.with_extension(format!("{}.part", std::process::id()));
    let result = fs::write(&part, &bytes).and_then(|()| fs::rename(&part, &to));
    if let Err(e) = result {
        let _ = fs::remove_file(&part);
        return Err(format!("{}: {e}", to.display()));
    }
    Ok(())
}

fn sha256(bytes: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, bytes)
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FONT: BuiltIn = BuiltIn {
        name: "Test",
        file: "Font.ttf",
        // No server exists: a valid cached or prepared file must not access the network.
        url: "http://127.0.0.1:1/font.ttf",
        sha256: "795ea3efa43d0872b63bf0067be97553b46983e4f075097669391e9d15388ecc",
        size: 4,
    };

    #[test]
    fn cached_fonts_prepare_and_repair_resources_without_network() {
        let root = std::env::temp_dir().join(format!("valle-fonts-cache-{}", std::process::id()));
        let cache_dir = root.join("cache");
        let cached = cache_dir.join("files").join(FONT.sha256).join(FONT.file);
        fs::create_dir_all(cached.parent().unwrap()).unwrap();
        fs::write(&cached, b"font").unwrap();
        let cache = UrlCache::new(&cache_dir);
        let dir = root.join("resources");
        let file = dir.join(FONT.file);

        materialize(&FONT, &cache, &dir).unwrap();
        assert_eq!(fs::read(&file).unwrap(), b"font");
        fs::write(&file, b"damaged").unwrap();
        materialize(&FONT, &cache, &dir).unwrap();
        assert_eq!(fs::read(&file).unwrap(), b"font");
        // An already prepared, verified file needs neither the cache nor the server.
        fs::remove_dir_all(&cache_dir).unwrap();
        materialize(&FONT, &cache, &dir).unwrap();
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn corrupt_cache_files_are_rejected_before_publication() {
        let root = std::env::temp_dir().join(format!("valle-fonts-corrupt-{}", std::process::id()));
        let cache_dir = root.join("cache");
        let cached = cache_dir.join("files").join(FONT.sha256).join(FONT.file);
        fs::create_dir_all(cached.parent().unwrap()).unwrap();
        fs::write(&cached, b"damaged").unwrap();
        let dir = root.join("resources");

        let error = materialize(&FONT, &UrlCache::new(cache_dir), &dir).unwrap_err();
        assert!(error.contains("does not match the pinned font"), "{error}");
        assert!(!dir.join(FONT.file).exists());
        fs::remove_dir_all(root).unwrap();
    }
}
