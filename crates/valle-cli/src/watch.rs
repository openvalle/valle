//! Poll dependencies for the existing reload/conflict flow. Fingerprint source contents as well
//! as metadata so same-size edits are detected even when filesystem timestamps do not change.
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
use valle_project::{
    ContentDigest,
    files::{FileStamp, Kind},
};

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct WatchStamp {
    metadata: FileStamp,
    content: Option<ContentDigest>,
}

impl WatchStamp {
    fn read(path: &Path) -> anyhow::Result<Self> {
        let metadata = FileStamp::read(path)?;
        // Keep large binary media on the cheap metadata path. SVG and glTF are editable text.
        let text_asset = path.extension().is_some_and(|extension| {
            extension.eq_ignore_ascii_case("svg") || extension.eq_ignore_ascii_case("gltf")
        });
        let content = if metadata.len <= 8 * 1024 * 1024
            && (text_asset
                || matches!(
                    Kind::detect(path),
                    Kind::Component | Kind::Shader | Kind::Data | Kind::Other
                )) {
            Some(valle_project::folder::digest_file(path)?)
        } else {
            None
        };
        Ok(Self { metadata, content })
    }
}

pub(crate) fn stamps(paths: &BTreeSet<PathBuf>) -> BTreeMap<PathBuf, Option<WatchStamp>> {
    paths
        .iter()
        .map(|p| (p.clone(), WatchStamp::read(p).ok()))
        .collect()
}

pub(crate) fn timeline_paths(file: &crate::timeline_file::TimelineFile) -> BTreeSet<PathBuf> {
    let mut paths = BTreeSet::from([file.input.clone()]);
    if let Ok(sources) = file.source_paths() {
        paths.extend(sources);
    }
    if let Ok(bytes) = std::fs::read(&file.input)
        && let Ok(document) = serde_json::from_slice::<serde_json::Value>(&bytes)
    {
        let base = file.input.parent().unwrap();
        for locator in document["resources"]
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(_, v)| v.as_str())
        {
            if locator.contains("://") {
                continue;
            }
            let path = base.join(locator);
            extend_shader(&mut paths, &path);
            paths.insert(path);
        }
    }
    if let Some(project) = &file.project {
        paths.insert(project.root.join("project.json"));
    }
    paths
}

pub(crate) fn extend_shader(paths: &mut BTreeSet<PathBuf>, path: &Path) {
    if path.to_string_lossy().ends_with(".shader.json")
        && let Ok(bytes) = std::fs::read(path)
        && let Ok(manifest) = valle_motion::shader::ShaderManifest::parse(&bytes)
        && manifest.validate_shape().is_ok()
    {
        paths.insert(path.parent().unwrap_or(Path::new(".")).join(manifest.entry));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn large_editable_assets_only_use_metadata_above_the_hash_limit() {
        let temp = tempfile::tempdir().unwrap();
        for extension in ["other", "svg", "gltf"] {
            let path = temp.path().join(format!("large.{extension}"));
            let file = std::fs::File::create(&path).unwrap();
            file.set_len(8 * 1024 * 1024).unwrap();
            assert!(WatchStamp::read(&path).unwrap().content.is_some());
            file.set_len(8 * 1024 * 1024 + 1).unwrap();
            assert!(WatchStamp::read(&path).unwrap().content.is_none());
            std::fs::remove_file(&path).unwrap();
            assert!(WatchStamp::read(&path).is_err());
            std::fs::write(&path, "changed").unwrap();
            assert!(WatchStamp::read(&path).unwrap().content.is_some());
        }
    }
    #[test]
    fn timeline_watches_component_modules_media_and_shader_source() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        std::fs::write(root.join("shared.ts"), "export const size=20;").unwrap();
        std::fs::write(
            root.join("card.motion.tsx"),
            "import {size} from './shared'; export default function Card(){return <Scene/>;}",
        )
        .unwrap();
        std::fs::write(root.join("image.png"), b"image").unwrap();
        std::fs::write(root.join("timeline.json"),r#"{"canvas":{"width":32,"height":32,"fps":30},"resources":{"card":"card.motion.tsx","image":"image.png"},"tracks":{"visual":[{"clips":[{"kind":"motion","component":"card","start":0,"duration":1}]}]}}"#).unwrap();
        let file = crate::timeline_file::TimelineFile::open(&root.join("timeline.json")).unwrap();
        let paths = timeline_paths(&file);
        assert!(paths.contains(&file.input.parent().unwrap().join("shared.ts")));
        assert!(paths.iter().any(|p| p.ends_with("image.png")));
        let shared = root.join("shared.ts");
        let modified = std::fs::metadata(&shared).unwrap().modified().unwrap();
        let before = stamps(&paths);
        std::fs::write(&shared, "export const size=30;").unwrap();
        // Windows can report identical metadata for a fast same-size write. Reproduce that
        // timestamp collision without depending on the speed or resolution of this filesystem.
        std::fs::File::options()
            .write(true)
            .open(&shared)
            .unwrap()
            .set_modified(modified)
            .unwrap();
        assert_ne!(stamps(&paths), before);
    }
}
