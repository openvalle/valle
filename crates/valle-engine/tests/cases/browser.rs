//! Read reviewed browser captures. Capturing and accepting them are separate, manual actions.

use std::collections::BTreeMap;
use std::path::Path;

use crate::scene::Measurements;
use serde::Deserialize;

pub const NAMES: [&str; 3] = ["chrome", "firefox", "webkit"];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capture {
    pub size: [u32; 2],
    sources: BTreeMap<String, String>,
    pub measurements: Measurements,
}

pub fn load(root: &Path, dir: &Path, name: &str) -> Result<Capture, String> {
    crate::fonts::prepare()?;
    let file = dir.join(format!("{name}.json"));
    let bytes = std::fs::read(&file).map_err(|e| {
        format!(
            "{}: {e}; capture and review browser baselines first",
            file.display()
        )
    })?;
    let capture: Capture = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if capture.sources.is_empty() {
        return Err("browser capture has no source hashes".into());
    }
    for (source, expected) in &capture.sources {
        if !source.starts_with("tests/")
            || !Path::new(source)
                .components()
                .all(|p| matches!(p, std::path::Component::Normal(_)))
        {
            return Err(format!("invalid browser source path {source:?}"));
        }
        let bytes = std::fs::read(root.join(source)).map_err(|e| format!("{source}: {e}"))?;
        verify_source(source, expected, &bytes)?;
    }
    Ok(capture)
}

fn verify_source(source: &str, expected: &str, bytes: &[u8]) -> Result<(), String> {
    if sha256(bytes) == expected {
        Ok(())
    } else {
        Err(format!(
            "stale browser baseline: {source} changed; recapture and review it"
        ))
    }
}

pub fn sha256(bytes: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, bytes)
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[test]
fn edited_source_requires_a_new_browser_capture() {
    let hash = sha256(b"original HTML");
    assert!(verify_source("scene.html", &hash, b"original HTML").is_ok());
    assert!(
        verify_source("scene.html", &hash, b"edited HTML")
            .unwrap_err()
            .contains("stale browser baseline")
    );
}
