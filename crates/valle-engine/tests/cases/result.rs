//! Machine-readable results alongside the rendered PNGs.

use std::path::Path;

use crate::scene::Sample;
use serde_json::json;

use crate::case::Case;
use crate::pixels::Comparison;

pub fn write(
    out: &Path,
    name: &str,
    case: &Case,
    sample: &Sample,
    layout: &Result<usize, String>,
    pixels: &Result<Option<Comparison>, String>,
) -> Result<(), String> {
    let frame = &sample.frame;
    let mut values = Vec::new();
    for expected in &case.layout {
        let actual = frame
            .template
            .nodes
            .iter()
            .position(|node| node.path.as_deref() == Some(expected.key.as_str()))
            .map(|i| frame.layout.boxes[i]);
        for (part, want, got) in [
            ("border", &expected.border, actual.map(|b| b.border)),
            ("padding", &expected.padding, actual.map(|b| b.padding)),
            ("content", &expected.content, actual.map(|b| b.content)),
        ] {
            for (field, want, got) in [
                ("x", want.x, got.map(|r| r.x)),
                ("y", want.y, got.map(|r| r.y)),
                ("width", want.width, got.map(|r| r.width)),
                ("height", want.height, got.map(|r| r.height)),
            ] {
                values.push(json!({
                    "field": format!("{}.{part}.{field}", expected.key),
                    "expected": want, "actual": got, "passed": got == Some(want),
                }));
            }
        }
    }
    let visual = match pixels {
        Ok(Some(p)) => json!({
            "status": if p.passed() { "passed" } else { "failed" },
            "size": p.size, "comparisons": p.checks,
            "device": p.device,
        }),
        Ok(None) => {
            json!({ "status": if case.visual() { "skipped" } else { "not-requested" } })
        }
        Err(error) => json!({ "status": "error", "error": error }),
    };
    let status = match pixels {
        _ if layout.is_err() => "FAIL",
        Err(_) => "FAIL",
        Ok(Some(p)) if !p.passed() => "FAIL",
        Ok(None) if case.visual() => "PARTIAL",
        _ => "PASS",
    };
    let data = json!({
        "case": name, "status": status, "spec": case.spec, "assertion": case.assertion,
        "frame": sample.number(), "at": sample.at.seconds_text(), "layout": values, "measurements": crate::scene::measure(frame), "layoutError": layout.as_ref().err(), "pixels": visual,
    });
    let json = serde_json::to_string_pretty(&data).map_err(|e| e.to_string())?;
    std::fs::write(out.join("result.json"), json).map_err(|e| e.to_string())
}
