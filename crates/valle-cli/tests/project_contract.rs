use serde_json::{Value, json};
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

fn command(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_valle"))
        .env("VALLE_HOME", root.join("home"))
        .current_dir(root)
        .arg("--json")
        .args(args)
        .output()
        .unwrap()
}
fn ok(root: &Path, args: &[&str]) -> Value {
    let output = command(root, args);
    assert!(
        output.status.success(),
        "{:?}\n{}\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
fn timeline() -> Value {
    json!({"canvas":{"width":32,"height":32,"fps":4},"tracks":{"visual":[{"clips":[{"kind":"solid","color":"#ff0000","start":0,"duration":1}]}]}})
}

#[test]
fn folder_cli_add_save_restore_and_render_survive_a_project_move() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();
    ok(
        root,
        &[
            "project", "create", "promo", "--width", "32", "--height", "32", "--fps", "4",
        ],
    );
    fs::write(root.join("notes.txt"), "independent input").unwrap();
    let added = ok(root, &["project", "add", "notes.txt", "-C", "promo"]);
    let imported = added["items"][0]["path"].as_str().unwrap();
    assert_eq!(imported, "media/notes.txt");
    assert!(
        ok(root, &["project", "add", "notes.txt", "-C", "promo"])["items"][0]["alreadyExists"]
            .as_bool()
            .unwrap()
    );
    fs::write(root.join("notes.txt"), "changed original").unwrap();
    assert_eq!(
        fs::read_to_string(root.join("promo").join(imported)).unwrap(),
        "independent input"
    );
    image::RgbaImage::from_pixel(32, 32, image::Rgba([255, 0, 0, 255]))
        .save(root.join("promo/media/card.png"))
        .unwrap();
    let mut original = timeline();
    original["resources"] = json!({"card":"media/card.png"});
    original["tracks"]["visual"][0]["clips"][0] =
        json!({"kind":"image","src":"card","start":0,"duration":1});
    fs::write(root.join("promo/timeline.json"), original.to_string()).unwrap();
    assert_eq!(ok(root, &["project", "save", "-C", "promo"])["snapshot"], 1);
    assert_eq!(
        ok(root, &["project", "save", "-C", "promo"])["unchanged"],
        true
    );
    let mut edited = original;
    edited["canvas"]["background"] = json!("#123456");
    fs::write(root.join("promo/timeline.json"), edited.to_string()).unwrap();
    let restored = ok(root, &["project", "restore", "1", "-C", "promo"]);
    assert_eq!(restored["beforeSnapshot"], 2);
    fs::rename(root.join("promo"), root.join("moved")).unwrap();
    let output = ok(
        root,
        &[
            "project",
            "render",
            "--frame",
            "0",
            "-o",
            "renders/frame.png",
            "-C",
            "moved",
        ],
    );
    assert_eq!(output["outputs"][0]["path"], "renders/frame.png");
    assert!(root.join("moved/renders/frame.png").is_file());
    assert_eq!(
        ok(root, &["project", "show", "-C", "moved"])["name"],
        "promo"
    );
    // Full PNG delivery needs no codec installation and must have one record for every run.
    let exported = ok(
        root,
        &["project", "render", "-o", "renders/%03d.png", "-C", "moved"],
    );
    assert_eq!(exported["outputs"].as_array().unwrap().len(), 4);
    let number = exported["snapshot"].as_u64().unwrap();
    let record = root.join(format!(
        "moved/.valle/history/{number}/exports/{}.json",
        exported["exportId"].as_str().unwrap()
    ));
    let record: Value = serde_json::from_slice(&fs::read(record).unwrap()).unwrap();
    assert_eq!(record["outputs"], exported["outputs"]);
    assert_eq!(record["receipt"]["frameCount"], 4);
    assert!(record["receipt"]["renderId"].is_string());
    let inputs = record["inputs"].as_array().unwrap();
    assert!(inputs.iter().all(|input| input["fileIdentity"].is_object()));
    let media = inputs
        .iter()
        .find(|input| input["path"] == "media/card.png")
        .unwrap();
    let path = root.join("moved/media/card.png");
    assert_eq!(
        media["digest"],
        json!(valle_project::folder::digest_file(&path).unwrap())
    );
    assert_eq!(
        media["fileIdentity"],
        json!(valle_project::files::FileStamp::read(&path).unwrap())
    );
    assert!(
        record["inputs"]
            .as_array()
            .unwrap()
            .iter()
            .all(|input| !input["path"].as_str().unwrap().starts_with('/'))
    );
    let image = fs::read(root.join("moved/renders/000.png")).unwrap();
    assert!(
        !command(
            root,
            &["project", "render", "-o", "renders/%03d.png", "-C", "moved"]
        )
        .status
        .success()
    );
    assert_eq!(fs::read(root.join("moved/renders/000.png")).unwrap(), image);
}

#[test]
fn imports_report_partial_success_and_reject_multifile_components_before_copying() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();
    ok(root, &["project", "create", "project"]);
    fs::write(root.join("existing.txt"), "keep").unwrap();
    let output = command(
        root,
        &[
            "project",
            "add",
            "existing.txt",
            "missing.txt",
            "-C",
            "project",
        ],
    );
    assert!(!output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["status"], "partial");
    assert_eq!(report["failed"], 1);
    assert_eq!(
        fs::read_to_string(root.join("project/media/existing.txt")).unwrap(),
        "keep"
    );
    fs::write(root.join("helper.ts"), "export const color = '#ff0000';").unwrap();
    fs::write(root.join("title.motion.tsx"),"import { color } from './helper'; export default function Title() { return <Scene><Rect width={32} height={32} fill={color}/></Scene>; }").unwrap();
    let output = command(
        root,
        &["project", "add", "title.motion.tsx", "-C", "project"],
    );
    assert!(!output.status.success());
    assert!(!root.join("project/components/title.motion.tsx").exists());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        report["items"][0]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("helper.ts")
    );
}

#[test]
fn source_snapshots_include_relative_modules_and_restore_broken_timelines() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();
    ok(root, &["project", "create", "project"]);
    let source = "import { color } from './helper'; export const composition = { width:32, height:32, duration:1 }; export default function Title() { return <Scene><Rect width={32} height={32} fill={color}/></Scene>; }";
    fs::write(root.join("project/components/title.motion.tsx"), source).unwrap();
    fs::write(
        root.join("project/components/helper.ts"),
        "export const color = '#ff0000';",
    )
    .unwrap();
    let mut document = timeline();
    document["resources"] = json!({"title":"components/title.motion.tsx"});
    document["tracks"]["visual"][0]["clips"][0] =
        json!({"kind":"motion","component":"title","start":0,"duration":1});
    fs::write(root.join("project/timeline.json"), document.to_string()).unwrap();
    ok(root, &["project", "save", "-C", "project"]);
    let snapshot: Value = serde_json::from_slice(
        &fs::read(root.join("project/.valle/history/1/snapshot.json")).unwrap(),
    )
    .unwrap();
    assert!(
        snapshot["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| file["path"] == "components/helper.ts")
    );
    fs::write(root.join("project/timeline.json"), "broken json").unwrap();
    fs::write(
        root.join("project/components/helper.ts"),
        "export const color = '#0000ff';",
    )
    .unwrap();
    ok(root, &["project", "restore", "1", "-C", "project"]);
    assert!(
        fs::read_to_string(root.join("project/components/helper.ts"))
            .unwrap()
            .contains("#ff0000")
    );
    assert!(
        serde_json::from_slice::<Value>(&fs::read(root.join("project/timeline.json")).unwrap())
            .is_ok()
    );
}
