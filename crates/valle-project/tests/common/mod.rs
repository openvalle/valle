#![allow(dead_code)]

use serde_json::json;
use valle_project::revision::{Actor, AuthenticatedContext, ProjectId, ProjectStore};
use valle_timeline::{Timeline, decode_timeline};

pub fn solid_timeline(background: Option<&str>, color: &str) -> Timeline {
    let mut canvas = json!({"width": 1920, "height": 1080, "fps": 30});
    if let Some(background) = background {
        canvas["background"] = json!(background);
    }
    decode_timeline(
        &json!({
            "canvas": canvas,
            "tracks": {
                "visual": [{
                    "clips": [{
                        "start": 0,
                        "duration": 2,
                        "kind": "solid",
                        "color": color
                    }]
                }]
            }
        })
        .to_string(),
    )
    .expect("valid sparse Author Timeline fixture")
}

pub fn timeline(variant: u8) -> Timeline {
    let color = match variant {
        0 => "#000000ff",
        1 => "#111111ff",
        2 => "#222222ff",
        3 => "#333333ff",
        _ => "#999999ff",
    };
    solid_timeline(None, color)
}

pub fn fixture() -> (
    tempfile::TempDir,
    ProjectStore,
    ProjectId,
    AuthenticatedContext,
) {
    let temporary = tempfile::tempdir().unwrap();
    let store = ProjectStore::at(temporary.path());
    let id = ProjectId::new("p1").unwrap();
    let auth = AuthenticatedContext::new(Actor::new("agent:test").unwrap());
    (temporary, store, id, auth)
}
