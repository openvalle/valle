//! Build one immutable Studio preview package from captured author inputs.
//! The host supplies frozen resource facts; this module owns Timeline identity,
//! Motion bindings, package serialization, and closed-package admission.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use valle_motion::{NodeKind, SceneArtifact};
use valle_timeline::internal::{
    ContentDigest, ResourceManifest, encode_canonical,
    wire::resource::{
        ContinuousBoundarySamplingWire, FontResourceDescriptorWire, FontVariationAxisWire,
        MotionArtifactAbiWire, MotionArtifactDescriptorWire, ResourceEntryWire,
        ResourceManifestEnvelopeWire,
    },
};
use valle_timeline::{decode_timeline, timeline_bytes};

use crate::{
    fixed_package::{
        COMMON_PROFILE_KEY, canonical_fixed_execution_profile, canonical_fixed_package_manifest,
        canonical_verified_binding_bundle, fixed_package_files, open_verified_fixed_package,
        prepared_resource_facts, resource_entry_digest,
    },
    render::{
        Capabilities, ResourceBinding, ResourceBindings, ResourceDependency, VerifiedHandleId,
        VerifiedResourceFacts, VisualFootprint,
    },
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreviewPackageInput {
    pub author_timeline: Value,
    pub motion_instances: Vec<PreviewMotionInstance>,
    #[serde(default)]
    pub resource_inputs: Vec<PreviewResourceInput>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreviewMotionInstance {
    /// Pointer into the sparse author Timeline, before normalization.
    pub clip_path: String,
    pub artifact: SceneArtifact,
    pub artifact_digest: ContentDigest,
    /// Ordered bytes used by this instance's compiler measurement environment.
    pub fonts: Vec<PreviewFont>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreviewFont {
    pub bytes_base64: String,
    /// `font` or `formula-font`; part of the artifact dependency role.
    pub role: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreviewResourceInput {
    pub id: String,
    pub entry: ResourceEntryWire,
    /// Frozen facts obtained from the same content as `entry` by the host.
    pub facts: Value,
    #[serde(default)]
    pub dependencies: Vec<PreviewDependency>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreviewDependency {
    pub role: String,
    pub resource_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedPreview {
    pub fixed_package_manifest_json: String,
    pub timeline_json: String,
    pub resource_manifest_json: String,
    pub verified_binding_bundle_json: String,
    pub execution_profile_json: String,
    pub external_resources: Vec<PreviewExternalResource>,
    pub motion_source_durations: BTreeMap<String, f64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewExternalResource {
    pub id: String,
    pub digest: ContentDigest,
}

struct PackageResources {
    entries: BTreeMap<String, ResourceEntryWire>,
    bindings: ResourceBindings,
    next_handle: u64,
    has_shader: bool,
}

impl PackageResources {
    fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
            bindings: ResourceBindings::new(),
            next_handle: 1,
            has_shader: false,
        }
    }

    fn add(
        &mut self,
        id: String,
        entry: ResourceEntryWire,
        facts: VerifiedResourceFacts,
        dependencies: Vec<PreviewDependency>,
    ) -> Result<(), String> {
        if self.entries.contains_key(&id) {
            return Err(format!("duplicate preview resource {id}"));
        }
        let digest = *resource_entry_digest(&entry);
        self.has_shader |= matches!(entry, ResourceEntryWire::Shader { .. });
        let handle = VerifiedHandleId::new(self.next_handle).map_err(|error| error.to_string())?;
        self.next_handle += 1;
        let mut binding = ResourceBinding::new(digest, handle, facts);
        for dependency in dependencies {
            binding = binding.with_dependency(
                ResourceDependency::new(dependency.role, dependency.resource_id)
                    .map_err(|error| error.to_string())?,
            );
        }
        self.entries.insert(id.clone(), entry);
        self.bindings
            .insert(id, binding)
            .map_err(|error| error.to_string())
    }

    fn capabilities(&self) -> Capabilities {
        let capabilities = Capabilities::new()
            .with_artifact_abi("valle.motion/artifact@1")
            .with_artifact_abi("valle.lottie/artifact@1");
        if self.has_shader {
            capabilities.with_artifact_abi("valle.shader/artifact@1")
        } else {
            capabilities
        }
    }
}

pub fn prepare_preview_package(input: PreviewPackageInput) -> Result<PreparedPreview, String> {
    let author_timeline =
        decode_timeline(&input.author_timeline.to_string()).map_err(|error| error.to_string())?;
    let document: Value = serde_json::from_slice(
        &timeline_bytes(&author_timeline).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let locators = document["resources"]
        .as_object()
        .ok_or("Timeline resources are missing")?;
    let mut instances = BTreeMap::<String, PreviewMotionInstance>::new();
    for instance in input.motion_instances {
        if instances
            .insert(instance.clip_path.clone(), instance)
            .is_some()
        {
            return Err("duplicate Motion preview clip path".into());
        }
    }
    let mut prepared = BTreeMap::<String, (PreviewMotionInstance, BTreeMap<String, String>)>::new();
    let mut durations = BTreeMap::new();
    for (track_index, track) in document["tracks"]["visual"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        for (clip_index, clip) in track["clips"].as_array().into_iter().flatten().enumerate() {
            if clip["kind"] != "motion" {
                continue;
            }
            let path = format!("/tracks/visual/{track_index}/clips/{clip_index}");
            let instance = instances
                .remove(&path)
                .ok_or_else(|| format!("Motion preview instance is missing for {path}"))?;
            let component = clip["component"]
                .as_str()
                .ok_or_else(|| format!("Motion component is missing at {path}"))?;
            let locator = locators
                .get(component)
                .and_then(Value::as_str)
                .ok_or_else(|| format!("Motion resource {component} is missing at {path}"))?;
            let resources = clip
                .get("resources")
                .filter(|value| value.as_object().is_some_and(|map| !map.is_empty()))
                .unwrap_or(&Value::Null);
            let data = clip
                .get("data")
                .filter(|value| value.as_object().is_some_and(|map| !map.is_empty()))
                .unwrap_or(&Value::Null);
            let key = valle_compiler::motion_instance_key(locator, resources, data);
            let composition = instance
                .artifact
                .composition
                .as_ref()
                .ok_or_else(|| format!("Motion {path} has no composition"))?;
            let duration = composition.duration().map_err(|error| error.to_string())?;
            if let Some(old) = durations.insert(key.clone(), duration) {
                if old != duration {
                    return Err(format!("Motion instance {key} has conflicting durations"));
                }
            }
            let bindings = resources
                .as_object()
                .map(|map| {
                    map.iter()
                        .map(|(role, alias)| {
                            Ok((
                                role.clone(),
                                alias
                                    .as_str()
                                    .ok_or_else(|| {
                                        format!("Motion asset {role} is not a resource alias")
                                    })?
                                    .to_owned(),
                            ))
                        })
                        .collect::<Result<BTreeMap<_, _>, String>>()
                })
                .transpose()?
                .unwrap_or_default();
            if let Some((existing, old_bindings)) = prepared.get(&key) {
                if existing.artifact_digest != instance.artifact_digest
                    || old_bindings != &bindings
                    || existing.fonts.len() != instance.fonts.len()
                    || existing
                        .fonts
                        .iter()
                        .zip(&instance.fonts)
                        .any(|(left, right)| {
                            left.role != right.role || left.bytes_base64 != right.bytes_base64
                        })
                {
                    return Err(format!(
                        "Motion instance {key} has inconsistent preparation inputs"
                    ));
                }
            } else {
                prepared.insert(key, (instance, bindings));
            }
        }
    }
    if let Some(path) = instances.keys().next() {
        return Err(format!(
            "Motion preview instance does not match an author clip: {path}"
        ));
    }
    let motion_source_durations = durations
        .iter()
        .map(|(key, duration)| (key.clone(), duration.as_f64()))
        .collect();
    let canonical =
        valle_compiler::compile_timeline_with_motion_sources(author_timeline, &durations)
            .map_err(|error| error.to_string())?;
    let timeline_json = encode_canonical(&canonical).map_err(|error| error.to_string())?;

    let mut resources = PackageResources::new();
    let mut external_resources = Vec::new();
    for resource in input.resource_inputs {
        if resource.id.starts_with("resource:motion-") {
            return Err(format!("preview resource ID is reserved: {}", resource.id));
        }
        let digest = *resource_entry_digest(&resource.entry);
        let facts = prepared_resource_facts(&resource.entry, resource.facts)?;
        external_resources.push(PreviewExternalResource {
            id: resource.id.clone(),
            digest,
        });
        resources.add(resource.id, resource.entry, facts, resource.dependencies)?;
    }
    for (key, (instance, asset_bindings)) in prepared {
        instance
            .artifact
            .validate()
            .map_err(|errors| format!("invalid Motion artifact {key}: {errors:?}"))?;
        let actual_digest = ContentDigest::of_bytes(
            &valle_motion::canonical_bytes(&instance.artifact)
                .map_err(|error| error.to_string())?,
        );
        if actual_digest != instance.artifact_digest {
            return Err(format!("Motion artifact digest changed for {key}"));
        }
        let mut dependencies = Vec::new();
        for (role, alias) in asset_bindings {
            if !instance.artifact.controls.assets.contains_key(&role) {
                return Err(format!("Motion {key} has no asset control {role}"));
            }
            dependencies.push(PreviewDependency {
                role,
                resource_id: format!("resource:{alias}"),
            });
        }
        let has_formula = instance
            .artifact
            .nodes
            .iter()
            .any(|node| matches!(node.kind, NodeKind::MathFormula { .. }));
        for (index, font) in instance.fonts.into_iter().enumerate() {
            if !matches!(font.role.as_str(), "font" | "formula-font") {
                return Err(format!("invalid font role {}", font.role));
            }
            if font.role == "formula-font" && !has_formula {
                return Err(format!(
                    "Motion {key} has no formula requiring a formula font"
                ));
            }
            let bytes = STANDARD
                .decode(font.bytes_base64.as_bytes())
                .map_err(|error| format!("invalid preview font: {error}"))?;
            let digest = ContentDigest::of_bytes(&bytes);
            let id = format!("font:{}:0", digest.as_hex());
            if !resources.entries.contains_key(&id) {
                let descriptor = font_descriptor(&bytes)?;
                resources.add(
                    id.clone(),
                    ResourceEntryWire::Font {
                        digest,
                        descriptor: descriptor.clone(),
                    },
                    VerifiedResourceFacts::Font {
                        descriptor,
                        bytes: Arc::from(bytes),
                    },
                    Vec::new(),
                )?;
            }
            dependencies.push(PreviewDependency {
                role: format!("{}:{index}", font.role),
                resource_id: id,
            });
        }
        let descriptor = MotionArtifactDescriptorWire {
            reads_destination: instance.artifact.reads_destination(),
            boundary_sampling: ContinuousBoundarySamplingWire::LeftLimit,
        };
        resources.add(
            format!("resource:{key}"),
            ResourceEntryWire::MotionArtifact {
                digest: actual_digest,
                abi: MotionArtifactAbiWire::Canonical,
                descriptor: descriptor.clone(),
            },
            VerifiedResourceFacts::MotionArtifact {
                abi: MotionArtifactAbiWire::Canonical,
                descriptor,
                artifact: Arc::new(instance.artifact),
                temporal_footprint: VisualFootprint::default(),
            },
            dependencies,
        )?;
    }
    // The closed admission below rejects missing resource IDs and mismatched facts.
    let capabilities = resources.capabilities();
    let manifest = ResourceManifest::try_from_wire(ResourceManifestEnvelopeWire {
        entries: resources.entries,
    })
    .map_err(|error| error.to_string())?;
    let resource_manifest_json = String::from_utf8(manifest.canonical_bytes().to_vec())
        .map_err(|error| error.to_string())?;
    let verified_binding_bundle_json =
        canonical_verified_binding_bundle(&resources.bindings, &capabilities)?;
    let execution_profile_json = canonical_fixed_execution_profile(COMMON_PROFILE_KEY)?;
    let files = fixed_package_files(
        &timeline_json,
        &resource_manifest_json,
        &verified_binding_bundle_json,
        &execution_profile_json,
    );
    let fixed_package_manifest_json = canonical_fixed_package_manifest(&files)?;
    open_verified_fixed_package(&fixed_package_manifest_json, &files)
        .map_err(|error| format!("preview package admission failed: {error}"))?;
    // Every locator is bound by digest. The browser host supplies the URL only after this succeeds.
    external_resources.sort_by(|a, b| a.id.cmp(&b.id));
    let unique = external_resources
        .iter()
        .map(|resource| resource.id.as_str())
        .collect::<BTreeSet<_>>();
    if unique.len() != external_resources.len() {
        return Err("duplicate external resource locator".into());
    }
    Ok(PreparedPreview {
        fixed_package_manifest_json,
        timeline_json,
        resource_manifest_json,
        verified_binding_bundle_json,
        execution_profile_json,
        external_resources,
        motion_source_durations,
    })
}

fn font_descriptor(bytes: &[u8]) -> Result<FontResourceDescriptorWire, String> {
    let face =
        ttf_parser::Face::parse(bytes, 0).map_err(|_| "invalid preview font face".to_owned())?;
    let variation_axes = face
        .variation_axes()
        .into_iter()
        .map(|axis| {
            (
                axis.tag.to_string(),
                FontVariationAxisWire {
                    minimum: f64::from(axis.min_value),
                    default: f64::from(axis.def_value),
                    maximum: f64::from(axis.max_value),
                },
            )
        })
        .collect();
    Ok(FontResourceDescriptorWire {
        face_index: 0,
        variation_axes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn simple_input() -> PreviewPackageInput {
        let artifact = valle_compiler::motion::compile_motion(
            "export const composition = { width: 64, height: 64, fps: 30, duration: 1 }; \
             export default function Card() { return <Scene><View style={{width:64,height:64,backgroundColor:'#2563eb'}} /></Scene>; }",
        ).unwrap().artifact;
        let artifact_digest =
            ContentDigest::of_bytes(&valle_motion::canonical_bytes(&artifact).unwrap());
        PreviewPackageInput {
            author_timeline: json!({
                "canvas":{"width":64,"height":64,"fps":30},
                "resources":{"card":"card.motion.tsx"},
                "tracks":{"visual":[{"clips":[
                    {"kind":"motion","component":"card","start":0,"duration":1}
                ]}]}
            }),
            motion_instances: vec![PreviewMotionInstance {
                clip_path: "/tracks/visual/0/clips/0".into(),
                artifact,
                artifact_digest,
                fonts: Vec::new(),
            }],
            resource_inputs: Vec::new(),
        }
    }

    #[test]
    fn admits_one_browser_compiled_motion_as_a_closed_timeline_package() {
        let package = prepare_preview_package(simple_input()).unwrap();
        let files = fixed_package_files(
            &package.timeline_json,
            &package.resource_manifest_json,
            &package.verified_binding_bundle_json,
            &package.execution_profile_json,
        );
        let opened =
            open_verified_fixed_package(&package.fixed_package_manifest_json, &files).unwrap();
        assert_eq!(opened.compiled().canvas().width(), 64);
        assert_eq!(opened.compiled().canvas().height(), 64);
        assert!(package.external_resources.is_empty());
    }

    #[test]
    fn rejects_artifact_digest_mismatch_before_publishing_a_package() {
        let mut input = simple_input();
        input.motion_instances[0].artifact_digest = ContentDigest::of_bytes(b"wrong");
        assert!(
            prepare_preview_package(input)
                .unwrap_err()
                .contains("artifact digest changed")
        );
    }

    #[test]
    fn rejects_duplicate_or_missing_motion_clip_inputs() {
        let mut duplicate = simple_input();
        duplicate.motion_instances.push(PreviewMotionInstance {
            clip_path: duplicate.motion_instances[0].clip_path.clone(),
            artifact: duplicate.motion_instances[0].artifact.clone(),
            artifact_digest: duplicate.motion_instances[0].artifact_digest,
            fonts: Vec::new(),
        });
        assert!(
            prepare_preview_package(duplicate)
                .unwrap_err()
                .contains("duplicate Motion")
        );
        let mut missing = simple_input();
        missing.motion_instances.clear();
        assert!(
            prepare_preview_package(missing)
                .unwrap_err()
                .contains("missing")
        );
    }
}
