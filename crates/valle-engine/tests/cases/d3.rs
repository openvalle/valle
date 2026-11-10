//! Compare Valle chart helpers with recorded results from the pinned official D3.

use indoc::indoc;
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;
use valle_engine::front::{Front, Inputs};
use valle_engine::template::{Kind, Piece, Prop, V};
use valle_engine::time::Tick;

/// A scene that runs the checks and writes their results as JSON text.
const SCENE: &str = indoc! {r#"
    import { cases } from "./cases.ts";

    export const composition = { width: 8, height: 8, fps: 1, duration: 1 };

    export default function Scene() {
      const results = {};
      for (const name of Object.keys(cases)) results[name] = cases[name]();
      return <Text>{JSON.stringify(results)}</Text>;
    }
"#};

#[derive(Deserialize)]
struct Baseline {
    d3: String,
    /// The checks the results are of.
    source_sha256: String,
    results: BTreeMap<String, Value>,
}

pub fn check(repo: &Path) -> Result<usize, String> {
    let source =
        std::fs::read_to_string(repo.join("tests/cases/d3/cases.ts")).map_err(|e| e.to_string())?;
    let bytes =
        std::fs::read(repo.join("tests/baselines/d3/results.json")).map_err(|e| e.to_string())?;
    let baseline: Baseline = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if baseline.d3 != crate::version(repo, "d3")?
        || baseline.source_sha256 != crate::browser::sha256(source.as_bytes())
    {
        return Err(
            "D3 baseline is stale; generate and review candidates with bun tests/tools/d3.ts"
                .into(),
        );
    }
    let ours = run(repo, &source)?;
    let mut problems = Vec::new();
    if !ours.keys().eq(baseline.results.keys()) {
        problems.push("D3 result names differ from the expected set".into());
    }
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
        true => Ok(baseline.results.len()),
        false => Err(problems.join("\n")),
    }
}

/// The checks' results on the built-in charts.
fn run(repo: &Path, source: &str) -> Result<BTreeMap<String, Value>, String> {
    let root = repo.join("tests/cases");
    let read = |p: &str| match p {
        "d3/run.motion.tsx" => Some(SCENE.to_string()),
        "d3/cases.ts" => Some(source.to_string()),
        _ => std::fs::read_to_string(root.join(p)).ok(),
    };
    let mut front = Front::load("d3/run.motion.tsx", &read, &Inputs::default())?;
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
