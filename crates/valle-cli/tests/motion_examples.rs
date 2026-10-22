//! motion-examples: the bundled examples check clean.

use std::{path::Path, process::Command};

use serde_json::{Value, json};

/// Every `examples/*.motion.tsx` compiles without warnings and draws its first frame.
#[test]
fn every_example_compiles_without_warnings() {
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut paths: Vec<_> = std::fs::read_dir(&examples)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.to_string_lossy().ends_with(".motion.tsx"))
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "no examples in {}", examples.display());
    let dir = tempfile::tempdir().unwrap();
    for path in &paths {
        let name = path.file_name().unwrap().to_string_lossy();
        let output = Command::new(env!("CARGO_BIN_EXE_valle"))
            .current_dir(dir.path())
            .args(["motion", "check"])
            .arg(path)
            .arg("--json")
            .output()
            .unwrap();
        let report: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
            panic!(
                "{name}\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        });
        assert!(output.status.success(), "{name}: {report}");
        assert_eq!(report["warnings"], json!([]), "{name}");
    }
}
