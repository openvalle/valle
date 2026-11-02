//! Tailwind baselines: what the Tailwind compiler of `package.json` makes of every class it lists
//! and of the samples here, `tests/baselines/tailwind/classes.json`. `web/tests/tailwind.ts`
//! compiles them when they are missing or out of date, which CI never does. The case
//! `tailwind/classes` holds the engine to them: each class it expands is Tailwind's CSS, and in
//! Tailwind's order, and every other fails.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

use serde::Deserialize;
use valle_engine::style::tailwind;

use crate::chrome;

pub const CASE: &str = "tailwind/classes";

/// Classes Tailwind does not list: how values, modifiers and the `!` read, and what the engine
/// refuses.
const SAMPLES: [&str; 104] = [
    "p-13",
    "p-0.75",
    "p-0.3",
    "p-4!",
    "!p-4",
    "p-4/50",
    "p-foo",
    "-p-4",
    "p-[3px]",
    "-m-[3px]",
    "m-[calc(100%-2rem)]",
    "w-[calc(100%-2rem*2)]",
    "w-[min(10px,2rem)]",
    "w-[length:2em]",
    "h-[var(--x)]",
    "[padding:3px_1px]",
    "[--gap:4px]",
    "w-(--size)",
    "hover:p-4",
    "w-13/17",
    "w-1/0",
    "-inset-1/2",
    "inset-[auto]",
    "aspect-[4/3]",
    "aspect-7/5",
    "z-[5]",
    "-z-[2.5]",
    "space-x-[3px]",
    "space-y-[0px]",
    "-space-x-2",
    "size-[3px]",
    "gap-x-[2%]",
    "max-w-screen-md",
    "bg-gradient-to-r",
    "flex-grow-0",
    "-start-4",
    "container",
    "w-",
    "p-[]",
    "m-[a;b]",
    "flex-1/0",
    "grow-[2]/3",
    "grid-cols-0",
    "-col-span-2",
    "col-span-[3]",
    "-order-first",
    "order-[7]",
    "auto-cols-[1fr]",
    "basis-3.5",
    "-basis-4",
    "text-sm/6",
    "text-sm/none",
    "text-sm/[20px]",
    "text-sm/foo",
    "text-[13px]/[20px]",
    "text-[13px]/6",
    "text-[larger]",
    "text-red-500/50",
    "text-[#f00]/25",
    "font-['Brand_Sans']",
    "font-[300]",
    "font-bold/50",
    "font-stretch-75%",
    "font-stretch-49%",
    "decoration-[3px]",
    "decoration-[#f00]",
    "-underline-offset-2",
    "-tracking-[1px]",
    "leading-[1.3]",
    "indent-[2em]",
    "bg-[10px]",
    "bg-[10px]/50",
    "bg-[center_top]",
    "bg-[length:200px_100px]",
    "bg-[url(/a.png)]",
    "bg-[linear-gradient(red,blue)]",
    "bg-black/[0.3]",
    "bg-opacity-50",
    "fill-[#0af]/50",
    "stroke-[1.5]",
    "stroke-[3px]/50",
    "stroke-2/50",
    "text-opacity-25",
    "bg-size-[auto_50%]",
    "bg-position-[center_10px]",
    "bg-linear-to-r!",
    "bg-linear-to-r/srgb",
    "bg-linear-to-r/[in_hsl]",
    "bg-linear-45/longer",
    "-bg-linear-45",
    "bg-linear-[25deg,red_5%,blue]",
    "-bg-linear-[25deg]",
    "-bg-linear-[to_right]",
    "bg-conic-90/srgb",
    "-bg-conic-90",
    "bg-conic/[in_oklch_longer_hue]",
    "bg-radial/oklch",
    "bg-radial-[at_25%_25%]",
    "bg-radial-50",
    "from-33%",
    "from-[10px]",
    "via-[#0f0]/50",
    "to-red-500/[0.5]",
    "from-[3px]/50",
];

/// What `web/tests/tailwind.ts` writes.
#[derive(Deserialize)]
struct Compiled {
    tailwind: String,
    properties: BTreeMap<String, Option<String>>,
    /// Every class, listed or asked for, and its CSS, those with CSS first in Tailwind's order.
    classes: Vec<(String, Option<String>)>,
}

#[derive(Deserialize)]
pub struct Baseline {
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

/// The Tailwind version `package.json` pins.
pub fn version(repo: &Path) -> Result<String, String> {
    let path = repo.join("package.json");
    let bad = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    let text = std::fs::read_to_string(&path).map_err(|e| bad(&e))?;
    let json: serde_json::Value = serde_json::from_str(&text).map_err(|e| bad(&e))?;
    (json["devDependencies"]["tailwindcss"].as_str())
        .map(str::to_string)
        .ok_or_else(|| bad(&"no devDependencies.tailwindcss"))
}

/// Holds the engine to the baselines, made first when they are missing or out of date.
pub fn case(repo: &Path, ci: bool) -> Result<(), String> {
    let path = repo.join("tests/baselines/tailwind/classes.json");
    let version = version(repo)?;
    let fresh = |b: &Baseline| b.tailwind == version && fresh(b);
    let baseline = match chrome::read::<Baseline>(&path)? {
        Some(b) if fresh(&b) => b,
        _ if ci => {
            return Err(format!(
                "the Tailwind {version} baselines are missing or out of date, and CI never makes \
                 them: `cargo xtask test` makes them on a machine that can run bun"
            ));
        }
        _ => {
            make(repo, &version, &path)?;
            (chrome::read::<Baseline>(&path)?).ok_or("the Tailwind baselines were not made")?
        }
    };
    check(&baseline)
}

/// Whether the baselines name the samples, and the engine still expands the classes it did and
/// refuses those it did.
fn fresh(b: &Baseline) -> bool {
    let named: BTreeSet<&str> = (b.expanded.iter().map(|(c, _)| c))
        .chain(&b.invalid)
        .chain(&b.refused)
        .map(String::as_str)
        .collect();
    SAMPLES.iter().all(|s| named.contains(s))
        && b.expanded.iter().all(|(c, _)| tailwind::class(c).is_ok())
        && b.refused.iter().all(|c| tailwind::class(c).is_err())
}

/// Compiles the classes with `web/tests/tailwind.ts`, and writes what the engine makes of them.
fn make(repo: &Path, version: &str, path: &Path) -> Result<(), String> {
    let dir = repo.join("target/tailwind");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let request = dir.join("request.json");
    let out = dir.join("compiled.json");
    let text = serde_json::json!({ "classes": &SAMPLES[..] }).to_string();
    std::fs::write(&request, text).map_err(|e| format!("{}: {e}", request.display()))?;
    println!("\ncompiling the Tailwind {version} baselines");
    crate::bun(
        repo,
        &[
            "web/tests/tailwind.ts".as_ref(),
            request.as_os_str(),
            out.as_os_str(),
        ],
    )?;
    let compiled = chrome::read::<Compiled>(&out)?.ok_or("web/tests/tailwind.ts wrote nothing")?;
    if compiled.tailwind != version {
        return Err(format!(
            "package.json pins Tailwind {version}, and {} is installed",
            compiled.tailwind
        ));
    }
    let (mut expanded, mut invalid, mut refused) = (Vec::new(), Vec::new(), Vec::new());
    for (class, css) in compiled.classes {
        match css {
            None => invalid.push(class),
            Some(css) if tailwind::class(&class).is_ok() => expanded.push((class, css)),
            Some(_) => refused.push(class),
        }
    }
    // An entry a line, so a change shows as the lines it moves.
    let entries = |items: Vec<String>| {
        let lines: Vec<String> = items.iter().map(|i| format!("  {i}")).collect();
        lines.join(",\n")
    };
    let properties = (compiled.properties.iter())
        .map(|(k, v)| format!("{}: {}", json(k), json(v)))
        .collect();
    let text = format!(
        "{{\n \"tailwind\": {},\n \"properties\": {{\n{}\n }},\n \"expanded\": [\n{}\n ],\n \
         \"invalid\": [\n{}\n ],\n \"refused\": [\n{}\n ]\n}}\n",
        json(version),
        entries(properties),
        entries(expanded.iter().map(json).collect()),
        entries(invalid.iter().map(json).collect()),
        entries(refused.iter().map(json).collect()),
    );
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

/// A value as JSON on one line.
fn json<T: serde::Serialize + ?Sized>(v: &T) -> String {
    serde_json::to_string(v).unwrap_or_default()
}

/// Where the engine and the baselines disagree.
fn check(b: &Baseline) -> Result<(), String> {
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
        return Ok(());
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
