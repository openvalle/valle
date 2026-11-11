//! Every scene and timeline in `examples/` and `examples/showcase/` checks without problems or
//! warnings, every frame traced, so an example nobody renders still breaks the build when the
//! engine changes under it.

use std::path::Path;
use std::process::Command;

use serde_json::Value;

#[test]
fn every_example_checks_cleanly() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut files: Vec<_> = [dir.clone(), dir.join("showcase")]
        .iter()
        .flat_map(|dir| std::fs::read_dir(dir).unwrap())
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_file())
        .collect();
    files.sort();
    let mut checked = 0;
    let mut failures = Vec::new();
    for file in &files {
        let name = file.file_name().unwrap().to_string_lossy();
        let group = if name.ends_with(".timeline.json") {
            "timeline"
        } else if name.ends_with(".motion.tsx") {
            "motion"
        } else {
            continue;
        };
        let out = Command::new(env!("CARGO_BIN_EXE_valle"))
            .args([group, "check"])
            .arg(file)
            .arg("--json")
            .output()
            .unwrap();
        let result: Value = serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|_| panic!("{name}: {}", String::from_utf8_lossy(&out.stderr)));
        if !out.status.success() || result["status"] != "ok" || result.get("warnings").is_some() {
            failures.push(format!("{name}: {result}"));
        }
        checked += 1;
    }
    // A wrong folder or renamed suffixes must not pass by checking nothing.
    assert!(checked > 15, "{checked} examples in {}", dir.display());
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
