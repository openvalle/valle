//! Every bundled Motion input must be referenced by a test or another fixture.
use std::{
    fs,
    path::{Path, PathBuf},
};

fn files_under(dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read test directory") {
        let path = entry.expect("test directory entry").path();
        if path.is_dir() {
            files_under(&path, files);
        } else {
            files.push(path);
        }
    }
}

#[test]
fn every_motion_fixture_has_a_consumer() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut files = Vec::new();
    for entry in fs::read_dir(crates).unwrap() {
        let tests = entry.unwrap().path().join("tests");
        if tests.is_dir() {
            files_under(&tests, &mut files);
        }
    }
    // Native/CanvasKit pixel acceptance tests also consume the shared Motion fixtures.
    let web_tests = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/packages/engine/src");
    let mut web_files = Vec::new();
    files_under(&web_tests, &mut web_files);
    files.extend(
        web_files
            .into_iter()
            .filter(|path| path.to_string_lossy().ends_with(".test.ts")),
    );
    let sources: Vec<_> = files
        .iter()
        .filter(|path| {
            matches!(
                path.extension().and_then(|s| s.to_str()),
                Some("rs" | "ts" | "tsx")
            )
        })
        .map(|path| (path, fs::read_to_string(path).unwrap()))
        .collect();
    let fixtures: Vec<_> = files
        .iter()
        .filter(|path| path.to_string_lossy().ends_with(".motion.tsx"))
        .collect();
    assert!(!fixtures.is_empty(), "no Motion fixtures found");
    for fixture in fixtures {
        let name = fixture.file_name().unwrap().to_str().unwrap();
        let stem = name.trim_end_matches(".motion.tsx");
        assert!(
            sources
                .iter()
                .any(|(path, source)| *path != fixture && source.contains(stem)),
            "Motion fixture has no consumer: {}",
            fixture.display()
        );
    }
}
