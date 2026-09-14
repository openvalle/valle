//! Rebuild a package's ordinary Motion font stack from host-selected bytes.
//! Asset-bound fonts and formula dependencies remain explicit resources of the work.
use super::*;
use valle_timeline::internal::wire::resource::{
    FontVariationAxisWire, ResourceManifestEnvelopeWire,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MotionFontBytes {
    pub bytes_base64: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MotionFontPackage {
    pub fixed_package_manifest_json: String,
    pub timeline_json: String,
    pub resource_manifest_json: String,
    pub verified_binding_bundle_json: String,
}

/// Select a new ordered font stack before opening a render. This creates a new
/// resource identity; it never substitutes bytes under an old content digest.
pub fn with_motion_fonts(
    package_json: &str,
    timeline_json: &str,
    manifest_json: &str,
    bundle_json: &str,
    fonts: &[MotionFontBytes],
) -> Result<MotionFontPackage, String> {
    let profile = canonical_fixed_execution_profile(COMMON_PROFILE_KEY)?;
    let files = fixed_package_files(timeline_json, manifest_json, bundle_json, &profile);
    verify_fixed_package(package_json, &files)?;
    let mut manifest: ResourceManifestEnvelopeWire =
        serde_json::from_str(manifest_json).map_err(|error| error.to_string())?;
    let mut bundle: UnresolvedBindingBundleWire =
        serde_json::from_str(bundle_json).map_err(|error| error.to_string())?;
    let mut next_handle = bundle
        .bindings
        .values()
        .filter_map(|binding| binding["handle"].as_u64())
        .max()
        .unwrap_or(0);
    let mut font_ids = Vec::new();
    for font in fonts {
        let bytes = decode_base64_payload(font.bytes_base64.clone(), "font.bytesBase64")?;
        let face = ttf_parser::Face::parse(&bytes, 0)
            .map_err(|error| format!("invalid font (expected TTF/OTF): {error}"))?;
        let digest = ContentDigest::of_bytes(&bytes);
        let id = format!("font:{}:0", digest.as_hex());
        if font_ids.contains(&id) {
            continue;
        }
        let descriptor = FontResourceDescriptorWire {
            face_index: 0,
            variation_axes: face
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
                .collect(),
        };
        if !bundle.bindings.contains_key(&id) {
            next_handle = next_handle.checked_add(1).ok_or("font handle overflow")?;
            manifest.entries.insert(
                id.clone(),
                ResourceEntryWire::Font {
                    digest,
                    descriptor: descriptor.clone(),
                },
            );
            bundle.bindings.insert(
                id.clone(),
                serde_json::to_value(ResourceBindingWire {
                    digest,
                    handle: next_handle,
                    facts: VerifiedResourceFactsWire::Font {
                        descriptor,
                        bytes_base64: font.bytes_base64.clone(),
                    },
                    dependencies: Vec::new(),
                })
                .map_err(|error| error.to_string())?,
            );
        }
        font_ids.push(id);
    }
    let mut removed = BTreeSet::new();
    for value in bundle.bindings.values_mut() {
        if value.pointer("/facts/kind").and_then(Value::as_str) != Some("motion-artifact") {
            continue;
        }
        let mut binding: ResourceBindingWire =
            serde_json::from_value(value.take()).map_err(|error| error.to_string())?;
        binding.dependencies.retain(|dependency| {
            if dependency.role.starts_with("font:") {
                removed.insert(dependency.resource_id.clone());
                false
            } else {
                true
            }
        });
        binding
            .dependencies
            .extend(
                font_ids
                    .iter()
                    .enumerate()
                    .map(|(index, id)| ResourceDependencyWire {
                        role: format!("font:{index}"),
                        resource_id: id.clone(),
                    }),
            );
        *value = serde_json::to_value(binding).map_err(|error| error.to_string())?;
    }
    // Keep dependencies shared with captions, asset controls, or another resource.
    let referenced: BTreeSet<_> = bundle
        .bindings
        .values()
        .filter_map(|binding| binding["dependencies"].as_array())
        .flatten()
        .filter_map(|dep| dep["resourceId"].as_str())
        .collect();
    let timeline: Value = serde_json::from_str(timeline_json).map_err(|error| error.to_string())?;
    let removable: Vec<_> = removed
        .into_iter()
        .filter(|id| !referenced.contains(id.as_str()) && !contains_reference(&timeline, id))
        .collect();
    for id in removable {
        bundle.bindings.remove(&id);
        manifest.entries.remove(&id);
    }
    let resource_manifest_json = String::from_utf8(
        ResourceManifest::try_from_wire(manifest)
            .map_err(|error| error.to_string())?
            .canonical_bytes()
            .to_vec(),
    )
    .map_err(|error| error.to_string())?;
    let verified_binding_bundle_json =
        serde_jcs::to_string(&bundle).map_err(|error| error.to_string())?;
    let files = fixed_package_files(
        timeline_json,
        &resource_manifest_json,
        &verified_binding_bundle_json,
        &profile,
    );
    let fixed_package_manifest_json = canonical_fixed_package_manifest(&files)?;
    open_verified_fixed_package(&fixed_package_manifest_json, &files)
        .map_err(|error| error.to_string())?;
    Ok(MotionFontPackage {
        fixed_package_manifest_json,
        timeline_json: timeline_json.to_owned(),
        resource_manifest_json,
        verified_binding_bundle_json,
    })
}

fn contains_reference(value: &Value, id: &str) -> bool {
    match value {
        Value::String(value) => value == id,
        Value::Array(values) => values.iter().any(|value| contains_reference(value, id)),
        Value::Object(values) => values.values().any(|value| contains_reference(value, id)),
        _ => false,
    }
}
