use std::fs;
use std::path::{Path, PathBuf};

use serde_json::json;

use crate::{Project, pretty};

/// An empty folder of its own for a test.
fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("valle-project-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("a test folder");
    dir
}

fn put(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().expect("a folder")).expect("its folder");
    fs::write(path, text).expect("written");
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).expect("read")
}

/// A project with a timeline playing one solid clip, and its resources as given.
fn project(name: &str, resources: serde_json::Value) -> Project {
    let dir = folder(name);
    let p = Project::create(&dir.join("promo"), 640, 360, "30000/1001").expect("created");
    let timeline = json!({
        "canvas": { "width": 640, "height": 360, "fps": "30000/1001" },
        "resources": resources,
        "tracks": { "visual": [{ "clips": [{ "kind": "solid", "color": "#000000ff", "start": 0, "duration": 1 }] }] }
    });
    fs::write(p.root.join("timeline.json"), pretty(&timeline)).expect("timeline");
    p
}

#[test]
fn a_project_is_found_from_any_folder_in_it() {
    let dir = folder("found");
    let p = Project::create(&dir.join("promo"), 1920, 1080, "30").expect("created");
    assert_eq!(
        (p.name.as_str(), p.timeline.as_str()),
        ("promo", "timeline.json")
    );
    for sub in [
        "timelines",
        "components",
        "media",
        "renders",
        ".valle/history",
    ] {
        assert!(p.root.join(sub).is_dir(), "{sub}");
    }
    let found = Project::discover(&p.root.join("media")).expect("found");
    assert_eq!(found.root, p.root);
    assert!(Project::create(&p.root, 1920, 1080, "30").is_err());
    assert!(
        Project::discover(&dir)
            .unwrap_err()
            .contains("project create")
    );
    assert!(Project::create(&dir.join("bad"), 1920, 1080, "fast").is_err());
    // A path out of the project is not its timeline.
    put(
        &dir.join("odd/project.json"),
        r#"{"name":"odd","timeline":"../t.json"}"#,
    );
    assert!(Project::open(&dir.join("odd")).is_err());
}

#[test]
fn files_are_copied_in_once_and_named_apart() {
    let p = project("add", json!({}));
    let out = folder("add-src");
    put(&out.join("clip.mp4"), "one");
    put(&out.join("other/Clip.MP4"), "two");
    put(
        &out.join("title.motion.tsx"),
        "export default function T() { return null; }\n",
    );
    let added = p
        .add(&[out.join("clip.mp4"), out.join("title.motion.tsx")], false)
        .expect("added");
    assert_eq!(added["status"], "ok");
    assert_eq!(added["added"][0]["path"], "media/clip.mp4");
    assert_eq!(
        added["added"][0]["resource"],
        json!({ "clip": "media/clip.mp4" })
    );
    assert_eq!(added["added"][1]["path"], "components/title.motion.tsx");
    // The same content again is the same file; other content of a name taken, in any case, is
    // named apart.
    let again = p
        .add(&[out.join("clip.mp4"), out.join("other/Clip.MP4")], false)
        .expect("again");
    assert_eq!(again["added"][0]["reused"], true);
    assert_eq!(again["added"][1]["path"], "media/Clip-2.MP4");
    assert_eq!(read(&p.root.join("media/clip.mp4")), "one");
    // The original can go: the project keeps its copy.
    fs::remove_file(out.join("clip.mp4")).expect("removed");
    assert_eq!(read(&p.root.join("media/clip.mp4")), "one");
    // Linked, it stays where it is.
    let linked = p.add(&[out.join("other/Clip.MP4")], true).expect("linked");
    let path = linked["added"][0]["path"].as_str().expect("a path");
    assert!(Path::new(path).is_absolute(), "{path}");
}

#[test]
fn some_files_failing_keeps_the_others() {
    let p = project("partial", json!({}));
    let out = folder("partial-src");
    put(&out.join("a.png"), "png");
    let added = p
        .add(&[out.join("a.png"), out.join("gone.png")], false)
        .expect("partial");
    assert_eq!(added["status"], "partial");
    assert_eq!(
        added["failed"][0]["from"],
        out.join("gone.png").display().to_string()
    );
    assert!(p.root.join("media/a.png").is_file());
    assert!(p.add(&[out.join("gone.png")], false).is_err());
}

#[test]
fn a_component_with_its_own_files_is_added_as_its_folder() {
    let p = project("component", json!({}));
    let out = folder("component-src");
    let scene = "import { K } from \"./k\";\nexport default function C() { return null; }\n";
    put(&out.join("chart/chart.motion.tsx"), scene);
    put(&out.join("chart/k.ts"), "export const K = 1;\n");
    let lone = p
        .add(&[out.join("chart/chart.motion.tsx")], false)
        .unwrap_err();
    assert!(lone.contains("add its folder"), "{lone}");
    let added = p.add(&[out.join("chart")], false).expect("added");
    assert_eq!(added["added"][0]["path"], "components/chart");
    assert_eq!(
        added["added"][0]["resource"],
        json!({ "chart": "components/chart/chart.motion.tsx" })
    );
    assert_eq!(
        read(&p.root.join("components/chart/k.ts")),
        "export const K = 1;\n"
    );
}

#[test]
fn components_keep_to_their_folders() {
    let scene =
        |import: &str| format!("{import}\nexport default function C() {{ return null; }}\n");
    let p = project(
        "boundary",
        json!({
            "lone": "components/lone.motion.tsx",
            "inside": "components/ok/ok.motion.tsx",
            "outside": "components/bad/bad.motion.tsx",
            "gone": "media/gone.mp4",
            "web": "https://example.com/a.png",
        }),
    );
    put(
        &p.root.join("components/lone.motion.tsx"),
        &scene("import { K } from \"./k\";"),
    );
    put(&p.root.join("components/k.ts"), "export const K = 1;\n");
    put(
        &p.root.join("components/ok/ok.motion.tsx"),
        &scene("import { K } from \"./k\";"),
    );
    put(&p.root.join("components/ok/k.ts"), "export const K = 1;\n");
    put(
        &p.root.join("components/bad/bad.motion.tsx"),
        &scene("import { K } from \"../k\";"),
    );
    let (problems, external) = p.check("timeline.json").expect("checked");
    let has = |what: &str| problems.iter().any(|m| m.contains(what));
    assert!(
        has("components/lone.motion.tsx reads components/k.ts"),
        "{problems:?}"
    );
    assert!(
        has("components/bad/bad.motion.tsx reads components/k.ts"),
        "{problems:?}"
    );
    assert!(has("resource gone names media/gone.mp4"), "{problems:?}");
    assert!(!has("ok.motion.tsx"), "{problems:?}");
    assert_eq!(problems.len(), 3, "{problems:?}");
    assert_eq!(external.len(), 1);
    assert_eq!(external[0]["file"], "https://example.com/a.png");
    let shown = p.show().expect("shown");
    assert_eq!(shown["timelines"][0]["clips"], 1);
    assert_eq!(shown["timelines"][0]["canvas"]["fps"], "30000/1001");
    assert_eq!(shown["missing"].as_array().map(Vec::len), Some(1));
}

#[test]
fn a_restore_puts_the_sources_back_and_can_be_undone() {
    let p = project("restore", json!({}));
    put(&p.root.join("media/data.json"), "[1]");
    put(&p.root.join("media/clip.mp4"), "video");
    put(&p.root.join("timelines/cut.json"), "{}");
    let (one, new) = p.save(Some("first"), "save").expect("saved");
    assert_eq!((one, new), (1, true));
    // Nothing changed: the same snapshot.
    assert_eq!(p.save(None, "save").expect("saved"), (1, false));
    let first = read(&p.root.join("timeline.json"));
    put(&p.root.join("timeline.json"), "broken");
    put(&p.root.join("media/data.json"), "[2]");
    put(&p.root.join("media/clip.mp4"), "edited video");
    put(&p.root.join("components/new.motion.tsx"), "added");
    let restored = p.restore(1).expect("restored");
    assert_eq!(restored["backup"], 2);
    assert_eq!(read(&p.root.join("timeline.json")), first);
    assert_eq!(read(&p.root.join("media/data.json")), "[1]");
    // Media is never written, and a file added since stays.
    assert_eq!(read(&p.root.join("media/clip.mp4")), "edited video");
    assert_eq!(restored["skipped"][0]["path"], "media/clip.mp4");
    assert_eq!(restored["added"], json!(["components/new.motion.tsx"]));
    // The backup has what the restore replaced.
    p.restore(2).expect("undone");
    assert_eq!(read(&p.root.join("timeline.json")), "broken");
    assert_eq!(read(&p.root.join("media/data.json")), "[2]");
    let history = p.history().expect("history");
    assert_eq!(
        history.iter().map(|s| s.number).collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert_eq!(history[1].by, "restore");
    assert_eq!(history[0].message.as_deref(), Some("first"));
    let kept = |n: usize, path: &str| {
        history[n]
            .files
            .iter()
            .find(|f| f.path == path)
            .map(|f| f.kept)
    };
    assert_eq!(kept(0, "media/clip.mp4"), Some(false));
    assert_eq!(kept(0, "timelines/cut.json"), Some(true));
}

#[test]
fn a_render_is_recorded_with_its_snapshot() {
    let p = project("record", json!({}));
    let (n, _) = p.save(None, "render").expect("saved");
    assert!(p.last_render().is_none());
    p.record(n, json!({ "output": "renders/a.mp4" }))
        .expect("recorded");
    p.record(n, json!({ "output": "renders/b.mp4" }))
        .expect("recorded");
    assert_eq!(
        p.last_render().expect("a render")["output"],
        "renders/b.mp4"
    );
    assert_eq!(p.records(n).len(), 2);
}
