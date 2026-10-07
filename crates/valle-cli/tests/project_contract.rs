use serde_json::json;
use std::{path::Path, process::Command};

fn command(root: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_valle"));
    command
        .env("VALLE_HOME", root.join("home"))
        .current_dir(root);
    command
}

#[test]
fn project_cli_exports_sparse_snapshots_and_preserves_existing_output_files() {
    let dir = tempfile::tempdir().unwrap();
    let source = json!({"canvas":{"width":32,"height":32,"fps":4},"tracks":{"visual":[{"clips":[{"kind":"solid","color":"#ff0000","start":0,"duration":1}]}]}});
    std::fs::write(dir.path().join("cut.json"), source.to_string()).unwrap();
    let create = command(dir.path())
        .args(["project", "create", "contract", "--timeline", "cut.json"])
        .output()
        .unwrap();
    assert!(
        create.status.success(),
        "{}",
        String::from_utf8_lossy(&create.stderr)
    );
    let show = command(dir.path())
        .args([
            "--json",
            "project",
            "show",
            "contract",
            "--output",
            "export.json",
        ])
        .output()
        .unwrap();
    assert!(
        show.status.success(),
        "{}",
        String::from_utf8_lossy(&show.stderr)
    );
    let exported = std::fs::read(dir.path().join("export.json")).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&exported).unwrap();
    assert_eq!(value["canvas"]["width"], 32);
    let repeated = command(dir.path())
        .args(["project", "show", "contract", "--output", "export.json"])
        .output()
        .unwrap();
    assert!(!repeated.status.success());
    assert_eq!(
        std::fs::read(dir.path().join("export.json")).unwrap(),
        exported
    );
    let mut edited = source;
    edited["canvas"]["background"] = json!("#123456");
    std::fs::write(dir.path().join("edit.json"), edited.to_string()).unwrap();
    for extra in [
        [
            "project",
            "apply",
            "contract",
            "--timeline",
            "edit.json",
            "--base-revision",
            "1",
        ],
        [
            "project", "history", "contract", "--limit", "1", "--cursor", "1",
        ],
    ] {
        let result = command(dir.path()).args(extra).output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    let stale = command(dir.path())
        .args([
            "project",
            "apply",
            "contract",
            "--timeline",
            "edit.json",
            "--base-revision",
            "1",
        ])
        .output()
        .unwrap();
    assert!(!stale.status.success());

    for (base, outcome, success) in [
        ("2", "committed", true),
        ("3", "unchanged", true),
        ("2", "staleBase", false),
    ] {
        let restored = command(dir.path())
            .args([
                "--json",
                "project",
                "restore",
                "contract",
                "--base-revision",
                base,
                "--revision",
                "1",
                "--intent",
                "restore original canvas",
            ])
            .output()
            .unwrap();
        assert_eq!(
            restored.status.success(),
            success,
            "{}",
            String::from_utf8_lossy(&restored.stderr)
        );
        let result: serde_json::Value = serde_json::from_slice(&restored.stdout).unwrap();
        assert_eq!(result["outcome"], outcome);
        assert_eq!(result["revision"], 3);
    }
    let restored = command(dir.path())
        .args(["--json", "project", "show", "contract"])
        .output()
        .unwrap();
    assert!(restored.status.success());
    let value: serde_json::Value = serde_json::from_slice(&restored.stdout).unwrap();
    assert_eq!(value["revision"], 3);
    assert_ne!(value["timeline"]["canvas"]["background"], "#123456");
}
