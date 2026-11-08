//! Timeline edits through the CLI: query addresses are usable by edit, linked edits are
//! atomic, and an empty document is still an editing document.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

const VALLE: &str = env!("CARGO_BIN_EXE_valle");

struct Timeline(PathBuf);

impl Timeline {
    fn new(name: &str, value: Value) -> Self {
        let dir =
            std::env::temp_dir().join(format!("valle-timeline-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("edit.timeline.json");
        std::fs::write(&file, value.to_string()).unwrap();
        Self(file)
    }

    fn run(&self, command: &str, args: &[&str]) -> Value {
        let output = Command::new(VALLE)
            .args(["timeline", command])
            .arg(&self.0)
            .args(args)
            .arg("--json")
            .output()
            .unwrap();
        let result: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            output.status.success(),
            result["status"] == "ok",
            "{result}"
        );
        result
    }

    fn edit(&self, op: Value) -> Value {
        self.run("edit", &["--op", &op.to_string()])
    }

    fn text(&self) -> String {
        std::fs::read_to_string(&self.0).unwrap()
    }
}

impl Drop for Timeline {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.0.parent().unwrap_or(Path::new("")));
    }
}

#[test]
fn a_new_projects_default_timeline_can_be_edited_and_queried() {
    let root = std::env::temp_dir().join(format!("valle-default-timeline-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let created = Command::new(VALLE)
        .args(["project", "create"])
        .arg(&root)
        .output()
        .unwrap();
    assert!(created.status.success(), "{created:?}");
    let timeline = Timeline(root.join("timeline.json"));
    assert_eq!(timeline.run("query", &[])["status"], "ok");
    assert_eq!(
        timeline.edit(json!({"op":"add","track":"tracks.visual[0]","clip":{
            "id":"first","kind":"solid","color":"blue","duration":1
        }}))["status"],
        "ok"
    );
    assert_eq!(timeline.run("query", &[])["clips"][0]["id"], "first");
}

#[test]
fn query_and_edit_share_addresses_and_zero_based_array_indices() {
    let timeline = Timeline::new(
        "addresses",
        json!({
            "canvas":{"width":64,"height":36,"fps":30},
            "tracks":{"visual":[{"id":"story","items":[
                {"id":"a","kind":"solid","color":"red","duration":1},
                {"gap":1},
                {"id":"b","kind":"solid","color":"blue","duration":1}
            ]}]}
        }),
    );
    let result = timeline.run("query", &["--track", "tracks.visual[0]", "--index", "2"]);
    let clip = &result["clips"][0];
    assert_eq!(clip["id"], "b", "{result}");
    assert_eq!(clip["index"], 2);
    assert_eq!(
        timeline.edit(json!({"op":"move","clip":clip["path"],"index":0}))["status"],
        "ok"
    );
    let result = timeline.run("query", &["--track", "story", "--index", "0"]);
    assert_eq!(result["clips"][0]["id"], "b");
    let added = json!({"id":"c","kind":"solid","color":"green","duration":1});
    assert_eq!(
        timeline.edit(json!({"op":"add","track":"story","clip":added}))["status"],
        "ok"
    );
    assert_eq!(
        timeline.run("query", &["--clip", "tracks.visual[0].items[3]"])["clips"][0]["id"],
        "c"
    );
    let before = timeline.text();
    assert_eq!(
        timeline.edit(json!({"op":"move","clip":"b","index":99}))["status"],
        "error"
    );
    assert_eq!(timeline.text(), before);
    assert!(
        timeline.run("query", &["--index", "-1"])["message"]
            .as_str()
            .unwrap()
            .contains("from 0")
    );
}

#[test]
fn linked_edits_persist_together_and_invalid_renames_do_not_write() {
    let timeline = Timeline::new(
        "linked",
        json!({
            "canvas":{"width":64,"height":36,"fps":30},
            "tracks":{
                "visual":[{"items":[
                    {"id":"a","kind":"solid","color":"red","duration":1},
                    {"transition":"fade","inOffset":0.1,"outOffset":0.1},
                    {"id":"b","kind":"solid","color":"blue","duration":1}
                ]}],
                "caption":[{"clips":[{"id":"title","text":"Caption","duration":0.5,"start":{"clip":"b"}}]},
                    {"clips":[{"id":"detail","text":"Detail","duration":0.5,"start":{"clip":"title"}}]}]
            }
        }),
    );
    assert_eq!(
        timeline.edit(json!({"op":"set","clip":"b","field":"id","value":"hero"}))["status"],
        "ok"
    );
    assert_eq!(
        timeline.run("query", &["--clip", "title"])["clips"][0]["anchor"]["clip"],
        "hero"
    );
    let before = timeline.text();
    assert_eq!(
        timeline.edit(json!({"op":"set","clip":"hero","field":"id","value":"a"}))["status"],
        "error"
    );
    assert_eq!(timeline.text(), before);
    assert_eq!(
        timeline.edit(json!({"op":"delete","clip":"hero"}))["status"],
        "ok"
    );
    let root: Value = serde_json::from_str(&timeline.text()).unwrap();
    assert_eq!(
        root["tracks"]["visual"][0]["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(root["tracks"]["caption"][0]["clips"], json!([]));
    assert_eq!(root["tracks"]["caption"][1]["clips"], json!([]));
    assert_eq!(timeline.run("check", &[])["status"], "ok");
}

#[test]
fn empty_timeline_can_add_delete_query_and_add_again() {
    let timeline = Timeline::new(
        "empty",
        json!({"canvas":{"width":64,"height":36,"fps":30},"tracks":{}}),
    );
    assert_eq!(timeline.run("query", &[])["count"], 0);
    let op = json!({"op":"add","track":"tracks.visual[0]","clip":{"id":"first","kind":"solid","color":"red","duration":1}});
    assert_eq!(timeline.edit(op.clone())["status"], "ok");
    assert_eq!(
        timeline.edit(json!({"op":"delete","clip":"first"}))["status"],
        "ok"
    );
    assert_eq!(timeline.run("query", &[])["count"], 0);
    assert_eq!(timeline.run("check", &[])["status"], "error");
    assert_eq!(timeline.edit(op)["status"], "ok");
}
