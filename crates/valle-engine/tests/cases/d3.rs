//! d3 baselines: what d3 of `package.json` gives for each check of `tests/cases/d3/checks.js`,
//! kept as `tests/baselines/d3/checks.json`. `web/tests/d3.ts` runs them when they are missing or
//! out of date, which CI never does. The case `d3/checks` runs the same checks on the built-in
//! charts and holds each result to d3's: numbers, and the numbers in text such as path data,
//! within a rounding of the last digit d3 writes.

use std::collections::BTreeMap;
use std::path::Path;

use indoc::indoc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use valle_engine::front::{Front, Inputs};
use valle_engine::template::{Kind, Piece, Prop, V};
use valle_engine::time::Tick;

use crate::chrome;

pub const CASE: &str = "d3/checks";

/// A scene that runs the checks and writes their results as JSON text.
const SCENE: &str = indoc! {r#"
    import { checks } from "./checks.js";

    export const composition = { width: 8, height: 8, fps: 1, duration: 1 };

    export default function Scene() {
      const results = {};
      for (const name of Object.keys(checks)) results[name] = checks[name]();
      return <Text>{JSON.stringify(results)}</Text>;
    }
"#};

#[derive(Deserialize, Serialize)]
struct Baseline {
    d3: String,
    /// The checks the results are of.
    source: String,
    results: BTreeMap<String, Value>,
}

/// What `web/tests/d3.ts` writes.
#[derive(Deserialize)]
struct Ran {
    d3: String,
    results: BTreeMap<String, Value>,
}

/// The d3 version `package.json` pins.
fn version(repo: &Path) -> Result<String, String> {
    let path = repo.join("package.json");
    let bad = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    let text = std::fs::read_to_string(&path).map_err(|e| bad(&e))?;
    let json: Value = serde_json::from_str(&text).map_err(|e| bad(&e))?;
    (json["devDependencies"]["d3"].as_str())
        .map(str::to_string)
        .ok_or_else(|| bad(&"no devDependencies.d3"))
}

/// Holds the built-in charts to the baselines, made first when they are missing or out of date.
pub fn case(repo: &Path, ci: bool) -> Result<(), String> {
    let checks = repo.join("tests/cases/d3/checks.js");
    let source =
        std::fs::read_to_string(&checks).map_err(|e| format!("{}: {e}", checks.display()))?;
    let path = repo.join("tests/baselines/d3/checks.json");
    let version = version(repo)?;
    let baseline = match chrome::read::<Baseline>(&path)? {
        Some(b) if b.d3 == version && b.source == source => b,
        _ if ci => {
            return Err(format!(
                "the d3 {version} baselines are missing or out of date, and CI never makes them: \
                 `cargo xtask test` makes them on a machine that can run bun"
            ));
        }
        _ => make(repo, &version, &source, &path)?,
    };
    let ours = run(repo, &source)?;
    let mut problems = Vec::new();
    for (name, theirs) in &baseline.results {
        match ours.get(name) {
            None => problems.push(format!("{name}: no result")),
            Some(v) => {
                if let Err(e) = same(v, theirs) {
                    problems.push(format!("{name}: {e}\n  ours: {v}\n    d3: {theirs}"));
                }
            }
        }
    }
    match problems.is_empty() {
        true => Ok(()),
        false => Err(problems.join("\n")),
    }
}

/// Runs the checks with d3 through `web/tests/d3.ts`, and keeps the results.
fn make(repo: &Path, version: &str, source: &str, path: &Path) -> Result<Baseline, String> {
    let out = repo.join("target/d3/results.json");
    let dir = out.parent().expect("a file in a folder");
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    println!("\nrunning the checks with d3 {version}");
    let checks = repo.join("tests/cases/d3/checks.js");
    crate::bun(
        repo,
        &[
            "web/tests/d3.ts".as_ref(),
            checks.as_os_str(),
            out.as_os_str(),
        ],
    )?;
    let ran = chrome::read::<Ran>(&out)?.ok_or("web/tests/d3.ts wrote nothing")?;
    if ran.d3 != version {
        return Err(format!(
            "package.json pins d3 {version}, and {} is installed",
            ran.d3
        ));
    }
    let baseline = Baseline {
        d3: ran.d3,
        source: source.to_string(),
        results: ran.results,
    };
    let text = serde_json::to_string_pretty(&baseline).map_err(|e| e.to_string())? + "\n";
    let dir = path.parent().expect("a file in a folder");
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(baseline)
}

/// The checks' results on the built-in charts.
fn run(repo: &Path, source: &str) -> Result<BTreeMap<String, Value>, String> {
    let root = repo.join("tests/cases");
    let read = |p: &str| match p {
        "d3/run.tsx" => Some(SCENE.to_string()),
        "d3/checks.js" => Some(source.to_string()),
        _ => std::fs::read_to_string(root.join(p)).ok(),
    };
    let mut front = Front::load("d3/run.tsx", &read, &Inputs::default())?;
    let frame = front.frame(Tick::ZERO)?;
    let text = (frame.template.nodes.iter())
        .find_map(|n| match &n.kind {
            Kind::Text { content } => match content.first() {
                Some(Piece::Text(Prop::Const(V::Str(s)))) => Some(s.clone()),
                _ => None,
            },
            _ => None,
        })
        .ok_or("the checks gave no text")?;
    serde_json::from_str(&text).map_err(|e| format!("the checks' results: {e}"))
}

/// Whether `ours` is `theirs`, numbers within a rounding.
fn same(ours: &Value, theirs: &Value) -> Result<(), String> {
    match (ours, theirs) {
        (Value::Number(a), Value::Number(b)) => {
            let (a, b) = (
                a.as_f64().unwrap_or(f64::NAN),
                b.as_f64().unwrap_or(f64::NAN),
            );
            match close(a, b) {
                true => Ok(()),
                false => Err(format!("{a} is not {b}")),
            }
        }
        (Value::String(a), Value::String(b)) => same_text(a, b),
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
            for (i, (x, y)) in a.iter().zip(b).enumerate() {
                same(x, y).map_err(|e| format!("[{i}]: {e}"))?;
            }
            Ok(())
        }
        (Value::Object(a), Value::Object(b)) if a.len() == b.len() => {
            for (k, y) in b {
                let x = a.get(k).ok_or_else(|| format!("no {k}"))?;
                same(x, y).map_err(|e| format!(".{k}: {e}"))?;
            }
            Ok(())
        }
        _ if ours == theirs => Ok(()),
        _ => Err("differs".into()),
    }
}

/// Numbers within what rounding the last of 3 decimals, as d3's paths write them, or of the last
/// bits of a double, may change.
fn close(a: f64, b: f64) -> bool {
    a == b || (a - b).abs() <= 1.0001e-3 + 1e-9 * a.abs().max(b.abs())
}

/// Texts the same but for their numbers, which are close.
fn same_text(a: &str, b: &str) -> Result<(), String> {
    let (x, y) = (pieces(a), pieces(b));
    let fits = x.len() == y.len()
        && x.iter().zip(&y).all(|(p, q)| match (p, q) {
            (Token::Number(p), Token::Number(q)) => close(*p, *q),
            _ => p == q,
        });
    match fits {
        true => Ok(()),
        false => Err("the texts differ".into()),
    }
}

#[derive(PartialEq)]
enum Token<'a> {
    Number(f64),
    Other(&'a str),
}

/// Text cut into numbers, as JavaScript writes them, and what is between them.
fn pieces(text: &str) -> Vec<Token<'_>> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let (mut i, mut start) = (0, 0);
    while i < bytes.len() {
        let c = bytes[i];
        let signed = c == b'-' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit);
        if c.is_ascii_digit() || signed {
            let mut j = i + 1;
            while j < bytes.len()
                && (bytes[j].is_ascii_digit()
                    || bytes[j] == b'.'
                    || bytes[j] == b'e'
                    || (matches!(bytes[j], b'-' | b'+') && bytes[j - 1] == b'e'))
            {
                j += 1;
            }
            if let Ok(x) = text[i..j].parse() {
                if start < i {
                    out.push(Token::Other(&text[start..i]));
                }
                out.push(Token::Number(x));
                start = j;
            }
            i = j;
        } else {
            i += 1;
        }
    }
    if start < text.len() {
        out.push(Token::Other(&text[start..]));
    }
    out
}
