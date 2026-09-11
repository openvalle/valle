//! Documentation must remain usable after relocating the executable without its source tree.

use std::{path::Path, process::Command};

use serde_json::Value;

const GUIDES: &[(&str, &[u8])] = &[
    ("cli", include_bytes!("../../../docs/cli.md")),
    ("motion", include_bytes!("../../../docs/motion.md")),
    ("timeline", include_bytes!("../../../docs/timeline.md")),
    ("media", include_bytes!("../../../docs/media.md")),
    ("project", include_bytes!("../../../docs/project.md")),
    ("assets", include_bytes!("../../../docs/assets.md")),
];

fn valle(directory: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_valle"));
    isolate(&mut command, directory);
    command
}

fn isolate(command: &mut Command, directory: &Path) {
    command
        .current_dir(directory)
        .env("VALLE_HOME", directory.join("unavailable"))
        .env("VALLE_CACHE_DIR", directory.join("unavailable"))
        .env("VALLE_MODEL_CACHE", directory.join("unavailable"))
        .env("VALLE_FFMPEG_DIR", directory.join("missing-libraries"))
        .env("ORT_DYLIB_PATH", directory.join("missing-onnx-runtime"));
}

#[test]
fn relocated_binary_reads_all_guides_without_files_or_runtimes() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path();
    std::fs::write(directory.join("unavailable"), b"not a directory").unwrap();
    let executable = directory.join(format!("relocated valle{}", std::env::consts::EXE_SUFFIX));
    std::fs::copy(env!("CARGO_BIN_EXE_valle"), &executable).unwrap();
    for (topic, expected) in GUIDES {
        let mut command = Command::new(&executable);
        isolate(&mut command, directory);
        let output = command.args(["docs", topic]).output().unwrap();
        assert!(output.status.success(), "{topic}: {output:?}");
        assert!(output.stderr.is_empty(), "{topic}: {output:?}");
        assert_eq!(output.stdout, *expected, "{topic}");
    }
    assert_eq!(std::fs::read_dir(directory).unwrap().count(), 2);
}

#[test]
fn docs_inventory_and_json_preserve_every_guide() {
    let temp = tempfile::tempdir().unwrap();
    let output = valle(temp.path())
        .args(["docs", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let index: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(index["status"], "ok");
    assert_eq!(index["version"], env!("CARGO_PKG_VERSION"));
    let topics = index["topics"].as_array().unwrap();
    assert_eq!(topics.len(), GUIDES.len());
    for (entry, (topic, expected)) in topics.iter().zip(GUIDES) {
        assert_eq!(entry["topic"], *topic);
        assert!(!entry["title"].as_str().unwrap().is_empty());
        let output = valle(temp.path())
            .args(["--json", "docs", topic])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let document: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(document["status"], "ok");
        assert_eq!(document["version"], index["version"]);
        assert_eq!(document["topic"], *topic);
        assert_eq!(document["title"], entry["title"]);
        assert_eq!(document["format"], "markdown");
        assert_eq!(document["content"].as_str().unwrap().as_bytes(), *expected);
    }
    let output = valle(temp.path()).arg("docs").output().unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    for (topic, _) in GUIDES {
        assert!(
            text.lines()
                .any(|line| line.split_whitespace().next() == Some(*topic))
        );
    }
}

#[test]
fn docs_follow_shared_event_help_and_argument_error_contracts() {
    let temp = tempfile::tempdir().unwrap();
    for args in [vec!["docs", "--events"], vec!["docs", "motion", "--events"]] {
        let output = valle(temp.path()).args(args).output().unwrap();
        assert!(output.status.success(), "{output:?}");
        let event: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(event["type"], "report");
        assert_eq!(event["data"]["status"], "ok");
    }
    let output = valle(temp.path())
        .args(["docs", "--help", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("[TOPIC]")
    );
    for args in [
        vec!["docs", "unknown", "--json"],
        vec!["docs", "skill", "--json"],
        vec!["docs", "../README.md", "--json"],
        vec!["docs", "motion", "--json", "--events"],
    ] {
        let output = valle(temp.path()).args(args).output().unwrap();
        assert_eq!(output.status.code(), Some(2));
        let error: Value = serde_json::from_slice(&output.stdout).unwrap();
        let error = if error["type"] == "report" {
            &error["data"]
        } else {
            &error
        };
        assert_eq!(error["error"]["code"], "invalid_arguments");
    }
}

#[cfg(unix)]
#[test]
fn docs_allow_a_reader_to_close_the_pipe_early() {
    use std::process::Stdio;
    let temp = tempfile::tempdir().unwrap();
    let mut child = valle(temp.path())
        .args(["docs", "motion"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
}
