//! Generate one embedded notice document without vendoring or archiving dependency sources.
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
};

pub fn generate(
    root: &Path,
    output: &Path,
    release: bool,
    environment: &BTreeMap<String, String>,
) -> Result<()> {
    let supplements: Vec<Value> =
        serde_json::from_slice(&fs::read(root.join("xtask/license-supplements.json"))?)?;
    let host = super::package::capture(Command::new("rustc").arg("-vV"))?;
    let host = host
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .context("rustc host missing")?;
    let metadata: Value = serde_json::from_str(&super::package::capture(
        Command::new("cargo")
            .args([
                "metadata",
                "--locked",
                "--format-version",
                "1",
                "--filter-platform",
                host,
            ])
            .current_dir(root),
    )?)?;
    let packages = metadata["packages"]
        .as_array()
        .context("Cargo packages missing")?;
    // A clean checkout has Cargo sources but not the native inputs downloaded by build.rs.
    // Cargo check prepares them without linking a throwaway CLI; subsequent builds reuse them.
    let skia = packages
        .iter()
        .find(|p| p["name"] == "skia-bindings")
        .context("Skia dependency missing")?;
    let skia_dir = Path::new(skia["manifest_path"].as_str().unwrap())
        .parent()
        .unwrap();
    if !skia_dir.join("skia/LICENSE").is_file() {
        let mut command = Command::new("cargo");
        command
            .args(["check", "--locked", "-p", "valle-cli"])
            .envs(environment)
            .current_dir(root);
        if release {
            command.arg("--release");
        }
        super::run(&mut command)?;
    }
    let cli = packages
        .iter()
        .find(|p| p["name"] == "valle-cli")
        .context("CLI package missing")?["id"]
        .as_str()
        .unwrap();
    let nodes: BTreeMap<_, _> = metadata["resolve"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|node| (node["id"].as_str().unwrap(), node))
        .collect();
    let mut included = BTreeSet::new();
    let mut pending = vec![cli];
    while let Some(id) = pending.pop() {
        if !included.insert(id) {
            continue;
        }
        for dep in nodes[id]["deps"].as_array().unwrap() {
            if dep["dep_kinds"]
                .as_array()
                .unwrap()
                .iter()
                .any(|kind| kind["kind"].is_null())
            {
                pending.push(dep["pkg"].as_str().unwrap());
            }
        }
    }
    let mut text = String::from(
        "Valle — licenses, acknowledgements and dependency sources\n\nValle source: https://github.com/openvalle/valle\nFFmpeg and codec libraries are not included. Media operations use separately installed libraries. Model weights and inference runtimes are separate downloads.\n\nThe versioned source links below identify upstream Rust and Web packages, including MPL-2.0 components. Adapted sources and their changes are identified in the accompanying notices. Original license and copyright notices follow each component.\n",
    );
    let revision = super::package::capture(
        Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(root),
    )?;
    text.push_str(&format!("\nValle base revision: {revision}\n"));
    let dirty = super::package::capture(
        Command::new("git")
            .args(["status", "--porcelain", "--untracked-files=no"])
            .current_dir(root),
    )?;
    if !dirty.is_empty() {
        text.push_str("Local development build: this working tree contains unpublished changes.\n");
    }
    let mut seen = BTreeSet::new();
    for relative in [
        "LICENSE",
        "THIRD_PARTY.md",
        "assets/fonts",
        "crates/valle-motion/licenses",
        "crates/valle-draw/licenses",
        "crates/valle-media/licenses",
    ] {
        let path = root.join(relative);
        if path.is_file() {
            append(&mut text, relative, &path, &mut seen)?;
        } else {
            collect(&mut text, &path, root, &mut seen)?;
        }
    }
    let mut selected: Vec<_> = packages
        .iter()
        .filter(|p| included.contains(p["id"].as_str().unwrap()) && !p["source"].is_null())
        .collect();
    selected.sort_by_key(|p| (p["name"].as_str(), p["version"].as_str()));
    for p in selected {
        let name = p["name"].as_str().unwrap();
        let version = p["version"].as_str().unwrap();
        let license = p["license"].as_str().unwrap_or("see included notices");
        let source = if p["source"]
            .as_str()
            .is_some_and(|source| source.starts_with("registry+"))
        {
            format!("https://static.crates.io/crates/{name}/{name}-{version}.crate")
        } else {
            anyhow::bail!("record an immutable source URL for {name}");
        };
        text.push_str(&format!(
            "\n===== Rust: {name} {version} ({license}) =====\nSource: {source}\n"
        ));
        let dir = Path::new(p["manifest_path"].as_str().unwrap())
            .parent()
            .unwrap();
        let before = text.len();
        collect(&mut text, dir, dir, &mut seen)?;
        supplement(&mut text, &supplements, name, version)?;
        ensure!(
            text.len() > before,
            "{name} {version} has no packaged license; record its upstream notice in xtask/license-supplements.json"
        );
        // Skia is downloaded by skia-bindings after Cargo extraction, so it has separate sources.
        if name == "skia-bindings" {
            ensure!(
                dir.join("skia/LICENSE").is_file(),
                "Skia notices missing; build Skia first"
            );
            let revision = p["metadata"]["skia"]
                .as_str()
                .context("Skia revision missing")?;
            text.push_str(&format!(
                "Skia source: https://github.com/rust-skia/skia/tree/{revision}\n"
            ));
        }
    }
    // Resolve production dependencies from installed workspace packages, never sweep Bun's cache
    // or unrelated old versions. Workspace packages retain the project-level notices above.
    let script = r#"
import fs from 'node:fs'; import path from 'node:path'; import {createRequire} from 'node:module';
const seen = new Set(), result = [];
function visit(dir) {
  dir = fs.realpathSync(dir); if (seen.has(dir)) return; seen.add(dir);
  const file = path.join(dir, 'package.json'), p = JSON.parse(fs.readFileSync(file, 'utf8'));
  if (!p.name.startsWith('@valle/')) result.push({dir, name:p.name, version:p.version, license:p.license});
  const require = createRequire(file);
  for (const name of Object.keys(p.dependencies || {})) {
    let located = null;
    for (const base of require.resolve.paths(name) || []) {
      const candidate = path.join(base, name);
      if (fs.existsSync(path.join(candidate, 'package.json'))) { located = candidate; break; }
    }
    if (!located) throw new Error('Cannot resolve production dependency ' + name + ' from ' + dir);
    visit(located);
  }
}
for (const group of ['apps','packages']) for (const name of fs.readdirSync(group).sort()) {
  const dir = path.resolve(group, name); if (fs.existsSync(path.join(dir,'package.json'))) visit(dir);
}
process.stdout.write(JSON.stringify(result.sort((a,b)=>(a.name+a.version).localeCompare(b.name+b.version))));
"#;
    let npm: Vec<Value> = serde_json::from_str(&super::package::capture(
        Command::new("bun")
            .args(["-e", script])
            .current_dir(root.join("web")),
    )?)?;
    for p in npm {
        let name = p["name"].as_str().unwrap();
        let version = p["version"].as_str().unwrap();
        let short = name.rsplit('/').next().unwrap();
        text.push_str(&format!("\n===== Web: {name} {version} ({}) =====\nSource: https://registry.npmjs.org/{name}/-/{short}-{version}.tgz\n", p["license"].as_str().unwrap_or("see included notices")));
        let dir = Path::new(p["dir"].as_str().unwrap());
        let before = text.len();
        collect(&mut text, dir, dir, &mut seen)?;
        supplement(&mut text, &supplements, name, version)?;
        ensure!(
            text.len() > before,
            "{name} {version} has no packaged license; record its upstream notice in xtask/license-supplements.json"
        );
    }
    fs::create_dir_all(output.parent().unwrap())?;
    if fs::read_to_string(output).ok().as_deref() != Some(&text) {
        fs::write(output, text)?;
    }
    Ok(())
}

fn supplement(text: &mut String, entries: &[Value], name: &str, version: &str) -> Result<()> {
    let identity = format!("{name}@{version}");
    for entry in entries {
        if entry["packages"]
            .as_array()
            .context("supplement packages missing")?
            .iter()
            .any(|p| p.as_str() == Some(&identity))
        {
            let source = entry["source"]
                .as_str()
                .context("supplement source missing")?;
            let notice = entry["text"].as_str().context("supplement text missing")?;
            ensure!(
                !notice.trim().is_empty(),
                "empty supplemental notice for {identity}"
            );
            text.push_str(&format!("\n--- Upstream notice: {source} ---\n{notice}\n"));
        }
    }
    Ok(())
}

fn is_notice(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    // Explicit separators avoid accidentally collecting source files such as copying.rs.
    [
        "license",
        "licence",
        "copying",
        "notice",
        "copyright",
        "ofl",
        "authors",
    ]
    .iter()
    .any(|base| {
        name == *base
            || name
                .strip_prefix(base)
                .is_some_and(|suffix| suffix.starts_with(['-', '_', '.']))
    }) && !name.ends_with(".rs")
        && !name.ends_with(".py")
        && !name.ends_with(".go")
}

fn collect(text: &mut String, dir: &Path, root: &Path, seen: &mut BTreeSet<PathBuf>) -> Result<()> {
    let mut entries = fs::read_dir(dir)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let kind = entry.file_type()?;
        if kind.is_symlink()
            || matches!(
                name.as_ref(),
                ".git" | "target" | "node_modules" | "__pycache__"
            )
        {
            continue;
        }
        if kind.is_dir() {
            collect(text, &entry.path(), root, seen)?;
        } else if kind.is_file()
            && (is_notice(&name)
                || (dir.components().any(|c| c.as_os_str() == "licenses")
                    && (name.ends_with(".txt") || name.ends_with(".md"))))
        {
            append(
                text,
                &entry.path().strip_prefix(root)?.to_string_lossy(),
                &entry.path(),
                seen,
            )?;
        }
    }
    Ok(())
}
fn append(text: &mut String, label: &str, path: &Path, seen: &mut BTreeSet<PathBuf>) -> Result<()> {
    if seen.insert(path.canonicalize()?) {
        let bytes = fs::read(path)?;
        // Some upstream notices predate UTF-8; retain all text instead of silently omitting them.
        text.push_str(&format!(
            "\n--- {label} ---\n{}\n",
            String::from_utf8_lossy(&bytes)
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn notice_detection_excludes_source_filenames() {
        for file in ["LICENSE", "LICENSE-MIT", "NOTICE.txt", "OFL-notosans.txt"] {
            assert!(is_notice(file));
        }
        for file in ["copying.rs", "copyright.py", "license_test.go"] {
            assert!(!is_notice(file));
        }
    }
}
