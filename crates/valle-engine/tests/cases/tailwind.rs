//! Compare utility expansion, declaration order and rejection with official Tailwind results.

use serde::Deserialize;
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;
use valle_engine::style::tailwind;

#[derive(Deserialize)]
struct Baseline {
    tailwind: String,
    /// The custom properties Tailwind registers, and their initial values.
    properties: BTreeMap<String, Option<String>>,
    /// The classes the engine expands, and Tailwind's CSS for them, in Tailwind's order.
    expanded: Vec<(String, String)>,
    /// Samples Tailwind makes nothing of.
    invalid: Vec<String>,
    /// Tailwind's other classes, which the engine refuses.
    refused: Vec<String>,
}

pub fn check(repo: &Path) -> Result<usize, String> {
    let bytes = std::fs::read(repo.join("tests/baselines/tailwind/classes.json"))
        .map_err(|e| e.to_string())?;
    let baseline: Baseline = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if baseline.tailwind != crate::version(repo, "tailwindcss")? {
        return Err("Tailwind baseline version differs from package.json".into());
    }
    #[derive(Deserialize)]
    struct Samples {
        classes: Vec<String>,
    }
    let bytes =
        std::fs::read(repo.join("tests/cases/tailwind/classes.json")).map_err(|e| e.to_string())?;
    let samples: Samples = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let names: BTreeSet<_> = baseline
        .expanded
        .iter()
        .map(|(name, _)| name)
        .chain(&baseline.invalid)
        .chain(&baseline.refused)
        .collect();
    if samples.classes.iter().any(|name| !names.contains(name)) {
        return Err("Tailwind samples are missing from the baseline; generate and review candidates with bun tests/tools/tailwind.ts".into());
    }
    compare(&baseline)
}

/// Where the engine and the baselines disagree.
fn compare(b: &Baseline) -> Result<usize, String> {
    let mut problems = Vec::new();
    for (name, initial) in tailwind::PROPERTIES {
        match b.properties.get(*name) {
            Some(theirs) if theirs.as_deref() == *initial => {}
            Some(theirs) => problems.push(format!(
                "{name} starts as {initial:?}, and Tailwind's as {theirs:?}"
            )),
            None => problems.push(format!("Tailwind registers no {name}")),
        }
    }
    let mut classes = Vec::new();
    for (name, theirs) in &b.expanded {
        match tailwind::class(name) {
            Ok(class) => {
                let ours = tailwind::print(&class);
                if ours != *theirs {
                    problems.push(format!("{name}: {ours}\n  Tailwind's: {theirs}"));
                }
                classes.push(class);
            }
            Err(e) => problems.push(format!("{e}, and it expanded when the baselines were made")),
        }
    }
    for pair in classes.windows(2) {
        if tailwind::compare(&pair[0], &pair[1]) != Ordering::Less {
            let (a, b) = (&pair[0].name, &pair[1].name);
            problems.push(format!("{b} comes before {a}, which Tailwind puts first"));
        }
    }
    for name in &b.invalid {
        if tailwind::class(name).is_ok() {
            problems.push(format!("{name} expands, and Tailwind makes nothing of it"));
        }
    }
    for name in &b.refused {
        if tailwind::class(name).is_ok() {
            problems.push(format!(
                "{name} expands, and did not when the baselines were made"
            ));
        }
    }
    if problems.is_empty() {
        return Ok(tailwind::PROPERTIES.len()
            + b.expanded.len()
            + b.invalid.len()
            + b.refused.len());
    }
    let mut out = String::new();
    for p in problems.iter().take(30) {
        let _ = writeln!(out, "{p}");
    }
    if problems.len() > 30 {
        let _ = writeln!(out, "… and {} more", problems.len() - 30);
    }
    Err(out.trim_end().to_string())
}
