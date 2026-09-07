//! Compile public Timeline documents into canonical timelines and standalone Motion JSX into
//! versioned artifacts. Timelines carry resource aliases and URLs; Engine admission resolves
//! artifacts, controls, and capabilities. OXC and QuickJS dependencies remain confined to this
//! crate.

#[cfg(feature = "motion")]
use std::collections::BTreeMap;

use anyhow::{Context, anyhow};
use serde::{Serialize, ser::SerializeStruct};
use valle_timeline::decode_timeline;
use valle_timeline::internal::CanonicalTimeline;

/// Sparse public Timeline → internal canonical normalization.
pub mod timeline;
pub use timeline::{CompileTimelineError, compile_timeline};

/// Generated Timeline contract types used by timeline/compiler clients and fixture builders.
/// Runtime hosts must consume `valle_engine::render` instead.
pub mod timeline_contract {
    pub use valle_timeline::internal::{
        CanonicalTimeline, ResourceManifest, canonical_bytes, decode_canonical,
        decode_resource_manifest, wire::resource::ResourceEntryWire,
    };
}

/// Closed, versioned Valle-native timeline pack that expands a curated caption effect catalog
/// into Timeline presentation curves. Preset names never cross the compiler boundary.
pub mod caption_presets;

/// Compile Motion JSX into a versioned artifact without evaluating per-frame expressions.
#[cfg(feature = "motion")]
pub mod motion;
#[cfg(feature = "motion")]
mod motion_sandbox;

/// Validated canonical project document. External JSON must enter through `decode_timeline`;
/// omitting `Deserialize` prevents bypassing admission checks.
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledProject {
    pub timeline: CanonicalTimeline,
}

impl CompiledProject {
    pub fn new(timeline: CanonicalTimeline) -> Self {
        Self { timeline }
    }
}

impl Serialize for CompiledProject {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut state = serializer.serialize_struct("CompiledProject", 1)?;
        state.serialize_field("timeline", &self.timeline.to_wire())?;
        state.end()
    }
}

/// One file entry in a txtar bundle.
#[derive(Debug, Clone)]
pub struct BundleEntry {
    pub name: String,
    pub data: String,
}

/// Parse a .valle txtar bundle; treat unmarked input as timeline.json.
pub fn parse_bundle(text: &str) -> Vec<BundleEntry> {
    let mut entries = Vec::new();
    let mut current_name: Option<String> = None;
    let mut current_data = String::new();

    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if let Some(name) = marker_name(trimmed) {
            if let Some(previous) = current_name.replace(name) {
                entries.push(BundleEntry {
                    name: previous,
                    data: std::mem::take(&mut current_data),
                });
            }
        } else if current_name.is_some() {
            current_data.push_str(line);
        }
    }
    if let Some(name) = current_name {
        entries.push(BundleEntry {
            name,
            data: current_data,
        });
    }
    if entries.is_empty() && !text.trim().is_empty() {
        entries.push(BundleEntry {
            name: "timeline.json".to_owned(),
            data: text.to_owned(),
        });
    }
    entries
}

/// Decode timeline.json using the public Timeline contract and normalize it. Reject unsupported
/// envelopes and internal document forms.
pub fn compile_project(entries: &[BundleEntry]) -> anyhow::Result<CompiledProject> {
    let mut timeline_entries = entries.iter().filter(|entry| entry.name == "timeline.json");
    let timeline_text = &timeline_entries
        .next()
        .ok_or_else(|| anyhow!("bundle missing timeline.json"))?
        .data;
    if timeline_entries.next().is_some() {
        return Err(anyhow!("bundle contains duplicate timeline.json entries"));
    }

    let timeline = decode_timeline(timeline_text).context("decode timeline.json")?;
    let timeline = compile_timeline(timeline).context("normalize timeline.json")?;
    Ok(CompiledProject::new(timeline))
}

/// Compile a bundle and return its canonical timeline.
pub fn compile_bundle(entries: &[BundleEntry]) -> anyhow::Result<CanonicalTimeline> {
    Ok(compile_project(entries)?.timeline)
}

/// Resolve every locked `shaders/<package>/manifest.json` directory through Valle's single
/// package admission implementation. Bundle entries are loaded first, then an optional project
/// directory; identical URI/content is idempotent and drift is rejected by `ShaderRegistry`.
#[cfg(feature = "motion")]
pub fn load_shader_registry(
    entries: &[BundleEntry],
    base: Option<&std::path::Path>,
) -> anyhow::Result<valle_motion::shader::ShaderRegistry> {
    let mut shader_entry_names = std::collections::BTreeSet::new();
    for entry in entries
        .iter()
        .filter(|entry| entry.name.starts_with("shaders/"))
    {
        if !shader_entry_names.insert(entry.name.as_str()) {
            return Err(anyhow!(
                "bundle contains duplicate shader entry `{}`",
                entry.name
            ));
        }
    }
    let files = entry_map(entries);
    let mut manifests = files
        .keys()
        .filter(|name| {
            let parts = name.split('/').collect::<Vec<_>>();
            parts.len() == 3 && parts[0] == "shaders" && parts[2] == "manifest.json"
        })
        .copied()
        .collect::<Vec<_>>();
    manifests.sort_unstable();

    let mut registry = valle_motion::shader::ShaderRegistry::new();
    for manifest_path in manifests {
        let directory = manifest_path
            .strip_suffix("/manifest.json")
            .expect("filtered manifest suffix");
        let manifest = files[manifest_path].as_bytes();
        let package = valle_motion::shader::ShaderPackage::admit_with_resolver(manifest, |entry| {
            let source_path = format!("{directory}/{entry}");
            files
                .get(source_path.as_str())
                .map(|source| source.as_bytes().to_vec())
                .ok_or_else(|| format!("bundle is missing `{source_path}`"))
        })
        .with_context(|| format!("admitting bundle shader `{manifest_path}`"))?;
        let expected_directory = format!("shaders/{}", package.manifest.name);
        if directory != expected_directory {
            return Err(anyhow!(
                "shader manifest `{manifest_path}` declares package `{}`; directory must be `{expected_directory}`",
                package.manifest.name
            ));
        }
        registry.register(package)?;
    }

    let Some(root) = base.map(|base| base.join("shaders")) else {
        return Ok(registry);
    };
    if !root.exists() {
        return Ok(registry);
    }
    let mut directories = std::fs::read_dir(&root)
        .with_context(|| format!("reading shader directory {}", root.display()))?
        .map(|entry| {
            let entry = entry?;
            Ok(entry.file_type()?.is_dir().then_some(entry.path()))
        })
        .collect::<std::io::Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    directories.sort();
    for directory in directories {
        let manifest_path = directory.join("manifest.json");
        if !manifest_path.is_file() {
            continue;
        }
        let manifest = std::fs::read(&manifest_path)
            .with_context(|| format!("reading shader manifest {}", manifest_path.display()))?;
        let package =
            valle_motion::shader::ShaderPackage::admit_with_resolver(&manifest, |entry| {
                let source_path = directory.join(entry);
                std::fs::read(&source_path)
                    .map_err(|error| format!("reading {}: {error}", source_path.display()))
            })
            .with_context(|| format!("admitting shader {}", manifest_path.display()))?;
        let directory_name = directory.file_name().and_then(|name| name.to_str());
        if directory_name != Some(package.manifest.name.as_str()) {
            return Err(anyhow!(
                "shader manifest {} declares package `{}`; directory name must match",
                manifest_path.display(),
                package.manifest.name
            ));
        }
        registry.register(package)?;
    }
    Ok(registry)
}

fn marker_name(line: &str) -> Option<String> {
    let rest = line.strip_prefix("-- ")?;
    let name = rest.strip_suffix(" --")?.trim();
    (!name.is_empty()).then(|| name.to_owned())
}

#[cfg(feature = "motion")]
fn entry_map(entries: &[BundleEntry]) -> BTreeMap<&str, &str> {
    entries
        .iter()
        .map(|entry| (entry.name.as_str(), entry.data.as_str()))
        .collect()
}
