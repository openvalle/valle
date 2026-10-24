use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};

fn run(home: &Path, args: &[&str], success: bool) -> Value {
    let out = Command::new(env!("CARGO_BIN_EXE_valle"))
        .env("VALLE_HOME", home)
        .arg("--json")
        .arg("assets")
        .args(args)
        .output()
        .unwrap();
    assert_eq!(
        out.status.success(),
        success,
        "{} {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

#[test]
fn library_cli_paths_tags_ranges_and_projects_share_the_same_files() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let source = temp.path().join("whoosh.wav");
    // The library can retain arbitrary files even when no playable facts can be extracted.
    fs::write(&source, b"unprobed audio bytes").unwrap();
    let imported = run(
        &home,
        &[
            "add",
            source.to_str().unwrap(),
            "--to",
            "sfx",
            "--tag",
            "转场",
            "--note",
            "短促风声",
        ],
        true,
    );
    assert_eq!(imported["status"], "ok");
    assert_eq!(imported["items"][0]["path"], "sfx/whoosh.wav");
    run(
        &home,
        &[
            "annotate",
            "sfx/whoosh.wav",
            "--range",
            "1,3",
            "--text",
            "可用片段",
        ],
        true,
    );
    let note = run(&home, &["show", "sfx/whoosh.wav"], true);
    assert_eq!(note["notes"][1]["range"], json!([1.0, 3.0]));
    let search = run(
        &home,
        &[
            "search", "风声", "--kind", "audio", "--in", "sfx", "--tag", "转场",
        ],
        true,
    );
    assert_eq!(search["total"], 1);
    run(
        &home,
        &[
            "edit",
            "sfx/whoosh.wav",
            "--name",
            "wind.wav",
            "--to",
            "brand/acme",
        ],
        true,
    );
    let new = run(&home, &["show", "brand/acme/wind.wav"], true);
    assert_eq!(new["notes"].as_array().unwrap().len(), 2);
    let project = temp.path().join("promo");
    let binary = env!("CARGO_BIN_EXE_valle");
    assert!(
        Command::new(binary)
            .env("VALLE_HOME", &home)
            .args(["project", "create"])
            .arg(&project)
            .output()
            .unwrap()
            .status
            .success()
    );
    let out = Command::new(binary)
        .env("VALLE_HOME", &home)
        .arg("--json")
        .args(["project", "add"])
        .arg(new["absolutePath"].as_str().unwrap())
        .arg("-C")
        .arg(&project)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert_eq!(
        fs::read(project.join("media/wind.wav")).unwrap(),
        b"unprobed audio bytes"
    );
    run(&home, &["sql", "DELETE FROM tags"], false);
    assert_eq!(
        run(&home, &["sql", "SELECT count(*) FROM tags"], true)["rows"][0][0],
        1
    );
}

#[test]
fn imports_report_partial_success_reject_multifile_components_and_retry_safely() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let source = temp.path().join("file.bin");
    fs::write(&source, b"file").unwrap();
    let component = temp.path().join("card.motion.tsx");
    fs::write(
        &component,
        "import {color} from './shared'; export default function Card(){return <Scene/>;}",
    )
    .unwrap();
    fs::write(
        temp.path().join("shared.ts"),
        "export const color='#ffffff';",
    )
    .unwrap();
    let report = run(
        &home,
        &["add", source.to_str().unwrap(), component.to_str().unwrap()],
        false,
    );
    assert_eq!(report["status"], "partial");
    assert_eq!(report["failed"], 1);
    assert_eq!(run(&home, &["list"], true)["total"], 1);
    let retry = run(
        &home,
        &["add", source.to_str().unwrap(), "--tag", "retry"],
        true,
    );
    assert_eq!(retry["items"][0]["alreadyExists"], true);
    assert_eq!(run(&home, &["list", "--tag", "retry"], true)["total"], 1);
    fs::create_dir_all(home.join("assets/.hidden")).unwrap();
    fs::write(home.join("assets/.hidden/ignored.txt"), b"hidden").unwrap();
    assert_eq!(run(&home, &["list"], true)["total"], 1);
    assert!(!home.join("assets/components/card.motion.tsx").exists());
}

#[test]
fn remove_uses_system_trash_and_new_files_do_not_inherit_deleted_notes() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let source = temp.path().join("valle-trash-contract.txt");
    fs::write(&source, b"disposable library trash contract").unwrap();
    run(
        &home,
        &["add", source.to_str().unwrap(), "--note", "old note"],
        true,
    );
    run(&home, &["remove", "other/valle-trash-contract.txt"], true);
    assert!(!home.join("assets/other/valle-trash-contract.txt").exists());
    run(&home, &["add", source.to_str().unwrap()], true);
    assert_eq!(
        run(&home, &["show", "other/valle-trash-contract.txt"], true)["notes"],
        json!([])
    );
}
