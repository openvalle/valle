use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use serde_json::{Value, json};
use valle_project::revision::{Actor, AuthenticatedContext, ProjectId, ProjectStore};
use valle_timeline::decode_timeline;

const PROJECT_ID: &str = "list-revision-adapters";

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/timeline")
        .join(name)
}

fn run_cli(home: &Path) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_valle"))
        .env("VALLE_HOME", home)
        .args(["project", "--json", "history", PROJECT_ID])
        .output()
        .expect("run Timeline revision CLI adapter");
    assert!(
        output.status.success(),
        "CLI failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("CLI list response must be JSON")
}

#[test]
fn cli_exposes_the_project_store_sparse_timeline_history() {
    let temporary = tempfile::tempdir().unwrap();
    let home = temporary.path();
    let project_id = ProjectId::new(PROJECT_ID).unwrap();
    let store = ProjectStore::at(home);
    let auth = AuthenticatedContext::new(Actor::new("agent:list-revision-test").unwrap());
    let timeline =
        decode_timeline(&fs::read_to_string(fixture("base.timeline.json")).unwrap()).unwrap();
    let genesis = store
        .create_project(&project_id, &timeline, None, &auth)
        .unwrap();
    let candidate: Value =
        serde_json::from_slice(&fs::read(fixture("candidate.timeline.json")).unwrap()).unwrap();
    let request = json!({
        "baseRevision":genesis.revision().revision,
        "timeline":candidate,
        "intent":"list summary acceptance",
    });
    let response = store
        .edit_timeline_json(&project_id, &request.to_string(), &auth)
        .unwrap();
    let head = store.get_timeline(&project_id, None).unwrap();
    assert_eq!(
        head.revision().revision,
        2,
        "unexpected edit outcome: {response:?}"
    );

    let expected = json!({
        "projectId": project_id,
        "revisions": store.list_revisions(&project_id, None).unwrap().revisions,
        "nextCursor": Value::Null,
    });
    let cli = run_cli(home);
    assert_eq!(cli, expected);
    assert_eq!(cli["revisions"][0]["revision"], 2);
    assert!(cli["revisions"][0].get("seq").is_none());
    assert!(cli["revisions"][0].get("diffSummary").is_none());
    assert!(cli["revisions"][0].get("documentHash").is_none());
    assert!(cli["revisions"][0].get("resourceManifestHash").is_none());
    assert_eq!(cli["revisions"][1]["revision"], 1);

    let invalid_cli = Command::new(env!("CARGO_BIN_EXE_valle"))
        .env("VALLE_HOME", home)
        .args(["project", "--json", "history", PROJECT_ID, "--limit", "0"])
        .output()
        .expect("run zero-limit Timeline revision CLI adapter");
    assert!(!invalid_cli.status.success());
    assert!(
        String::from_utf8_lossy(&invalid_cli.stdout)
            .contains("revision page limit must be greater than zero"),
        "unexpected CLI error: {}",
        String::from_utf8_lossy(&invalid_cli.stdout)
    );
}
