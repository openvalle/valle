//! Poll metadata only; a changed dependency triggers the existing reload/conflict flow.
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
use valle_project::files::FileStamp;

pub(crate) fn stamps(paths: &BTreeSet<PathBuf>) -> BTreeMap<PathBuf, Option<FileStamp>> {
    paths
        .iter()
        .map(|p| (p.clone(), FileStamp::read(p).ok()))
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
        let before = stamps(&paths);
        std::fs::write(root.join("shared.ts"), "export const size=30;").unwrap();
        assert_ne!(stamps(&paths), before);
    }
}
