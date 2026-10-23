use std::{fs, path::Path};

use valle_project::folder::{CapturedInput, Project};

fn capture(project: &Project) -> Vec<CapturedInput> {
    vec![CapturedInput::source(
        project.timeline_path(),
        fs::read(project.timeline_path()).unwrap(),
    )]
}

#[test]
fn folder_project_snapshots_move_with_the_project_and_back_up_restore_targets() {
    let temporary = tempfile::tempdir().unwrap();
    let project = Project::create(&temporary.path().join("promo"), 640, 360, "30").unwrap();
    assert_eq!(
        Project::discover(&project.root.join("components"))
            .unwrap()
            .root,
        project.root
    );
    let _lock = project.lock().unwrap();
    let source = project.root.join("components/title.motion.tsx");
    fs::write(&source, "original source").unwrap();
    let mut inputs = capture(&project);
    inputs.push(CapturedInput::source(
        source.clone(),
        fs::read(&source).unwrap(),
    ));
    let first = project
        .save_captured(inputs.clone(), Some("first"), "save")
        .unwrap();
    assert_eq!(first.snapshot, 1);
    assert!(
        project
            .save_captured(inputs, None, "save")
            .unwrap()
            .unchanged
    );
    // The current Timeline no longer uses the old source, but restore must still back it up.
    fs::write(&source, "unreferenced work").unwrap();
    let restored = project.restore(1, capture(&project)).unwrap();
    assert_eq!(fs::read_to_string(&source).unwrap(), "original source");
    project
        .restore(restored.before_snapshot, capture(&project))
        .unwrap();
    assert_eq!(fs::read_to_string(&source).unwrap(), "unreferenced work");
    drop(_lock);
    let moved = temporary.path().join("moved");
    fs::rename(&project.root, &moved).unwrap();
    let project = Project::open(&moved).unwrap();
    let _lock = project.lock().unwrap();
    project.restore(1, capture(&project)).unwrap();
    assert_eq!(
        fs::read_to_string(project.root.join("components/title.motion.tsx")).unwrap(),
        "original source"
    );
}

#[test]
fn restore_handles_a_broken_timeline_and_preserves_media_and_new_files() {
    let temporary = tempfile::tempdir().unwrap();
    let project = Project::create(temporary.path(), 640, 360, "30").unwrap();
    let _lock = project.lock().unwrap();
    let media = project.root.join("media/music.wav");
    fs::write(&media, b"first media").unwrap();
    let mut inputs = capture(&project);
    inputs.push(CapturedInput::media(
        media.clone(),
        valle_project::ContentDigest::of_bytes(b"first media"),
    ));
    project.save_captured(inputs, None, "save").unwrap();
    fs::write(project.timeline_path(), "broken json").unwrap();
    fs::write(&media, b"modified media").unwrap();
    fs::write(project.root.join("draft.tsx"), "keep me").unwrap();
    let restored = project.restore(1, Vec::new()).unwrap();
    assert!(
        restored
            .skipped
            .iter()
            .any(|item| item["path"] == "media/music.wav")
    );
    assert_eq!(fs::read(&media).unwrap(), b"modified media");
    assert!(project.root.join("draft.tsx").exists());
    assert!(
        valle_timeline::decode_timeline(&fs::read_to_string(project.timeline_path()).unwrap())
            .is_ok()
    );
    project
        .restore(restored.before_snapshot, capture(&project))
        .unwrap();
    assert_eq!(
        fs::read_to_string(project.timeline_path()).unwrap(),
        "broken json"
    );
}

#[cfg(unix)]
#[test]
fn restore_does_not_follow_symlinked_parent_directories() {
    let temporary = tempfile::tempdir().unwrap();
    let project = Project::create(&temporary.path().join("project"), 640, 360, "30").unwrap();
    let outside = temporary.path().join("outside");
    fs::create_dir(&outside).unwrap();
    let source = project.root.join("components/title.motion.tsx");
    fs::write(&source, "saved").unwrap();
    let _lock = project.lock().unwrap();
    let mut inputs = capture(&project);
    inputs.push(CapturedInput::source(source.clone(), b"saved".to_vec()));
    project.save_captured(inputs, None, "save").unwrap();
    fs::remove_file(&source).unwrap();
    fs::remove_dir(project.root.join("components")).unwrap();
    fs::write(outside.join("title.motion.tsx"), "outside").unwrap();
    std::os::unix::fs::symlink(&outside, project.root.join("components")).unwrap();
    let result = project.restore(1, capture(&project)).unwrap();
    assert!(
        result
            .skipped
            .iter()
            .any(|item| item["reason"] == "symbolic_link_or_nonregular")
    );
    assert_eq!(
        fs::read_to_string(outside.join("title.motion.tsx")).unwrap(),
        "outside"
    );
}

#[test]
fn create_preserves_existing_files_and_rejects_invalid_canvas() {
    let temporary = tempfile::tempdir().unwrap();
    fs::write(temporary.path().join("notes.txt"), "existing").unwrap();
    assert!(Project::create(temporary.path(), 0, 360, "30").is_err());
    assert!(!temporary.path().join("project.json").exists());
    Project::create(temporary.path(), 640, 360, "30").unwrap();
    assert_eq!(
        fs::read_to_string(temporary.path().join("notes.txt")).unwrap(),
        "existing"
    );
    assert!(Project::create(temporary.path(), 640, 360, "30").is_err());
    assert_eq!(
        Project::discover(Path::new(temporary.path()))
            .unwrap()
            .config
            .name,
        temporary.path().file_name().unwrap().to_string_lossy()
    );
}
