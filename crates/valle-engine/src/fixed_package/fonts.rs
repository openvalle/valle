//! Rebuild a package's ordinary Motion font stack from host-selected bytes.
//! Asset-bound fonts and formula dependencies remain explicit resources of the work.
use super::*;
use valle_timeline::internal::wire::resource::{
    FontVariationAxisWire, ResourceManifestEnvelopeWire,
};

/// Freeze every selectable face of a font file, including TTC/OTC collections.
/// Layout registers the whole file, so recording only face zero leaves bold/italic faces unbound.
pub fn font_face_descriptors(bytes: &[u8]) -> Result<Vec<FontResourceDescriptorWire>, String> {
    let count = ttf_parser::fonts_in_collection(bytes).unwrap_or(1);
    if count == 0 || count > 256 {
        return Err("a font file must contain between 1 and 256 faces".into());
    }
    (0..count)
        .map(|face_index| {
            let face = ttf_parser::Face::parse(bytes, face_index)
                .map_err(|error| format!("invalid font face {face_index}: {error}"))?;
            Ok(FontResourceDescriptorWire {
                face_index,
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
            })
        })
        .collect()
}

/// Materialize a collection face for renderers whose font loader only accepts index zero
/// (CoreText and CanvasKit). Resource identity remains the original digest plus face index.
pub fn font_face_bytes(bytes: &[u8], index: u32) -> Result<std::borrow::Cow<'_, [u8]>, String> {
    use std::borrow::Cow;
    let invalid = || format!("invalid font collection face {index}");
    let Some(count) = ttf_parser::fonts_in_collection(bytes) else {
        return if index == 0 {
            Ok(Cow::Borrowed(bytes))
        } else {
            Err(invalid())
        };
    };
    if count == 0 || count > 256 || index >= count {
        return Err(invalid());
    }
    let entry = 12 + index as usize * 4;
    let start = u32::from_be_bytes(
        bytes
            .get(entry..entry + 4)
            .ok_or_else(invalid)?
            .try_into()
            .unwrap(),
    ) as usize;
    let raw = ttf_parser::RawFace::parse(bytes, index).map_err(|_| invalid())?;
    let count = raw.table_records.len() as usize;
    if count == 0 || count > 4096 {
        return Err(invalid());
    }
    let header = 12 + count * 16;
    let mut result = bytes
        .get(start..start.checked_add(header).ok_or_else(invalid)?)
        .ok_or_else(invalid)?
        .to_vec();
    let mut adjustment = None;
    for (table, record) in raw.table_records.into_iter().enumerate() {
        let data = bytes
            .get(
                record.offset as usize
                    ..(record.offset as usize)
                        .checked_add(record.length as usize)
                        .ok_or_else(invalid)?,
            )
            .ok_or_else(invalid)?;
        let offset = result.len();
        let end = offset
            .checked_add(data.len())
            .ok_or_else(invalid)?
            .next_multiple_of(4);
        if end > 64 * 1024 * 1024 {
            return Err(invalid());
        }
        result[12 + table * 16 + 8..12 + table * 16 + 12]
            .copy_from_slice(&(offset as u32).to_be_bytes());
        result.extend_from_slice(data);
        result.resize(end, 0);
        if record.tag == ttf_parser::Tag::from_bytes(b"head") && data.len() >= 12 {
            adjustment = Some(offset + 8);
            result[offset + 8..offset + 12].fill(0);
        }
    }
    if let Some(at) = adjustment {
        let checksum = result
            .as_chunks::<4>()
            .0
            .iter()
            .fold(0u32, |sum, b| sum.wrapping_add(u32::from_be_bytes(*b)));
        result[at..at + 4].copy_from_slice(&0xb1b0afbau32.wrapping_sub(checksum).to_be_bytes());
    }
    Ok(Cow::Owned(result))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MotionFontBytes {
    pub bytes: Vec<u8>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MotionFontPackage {
    pub fixed_package_manifest_json: String,
    pub timeline_json: String,
    pub resource_manifest_json: String,
    pub verified_binding_bundle_json: String,
    #[serde(skip)]
    pub resource_bytes: Vec<u8>,
}

/// Select a new ordered font stack before opening a render. This creates a new
/// resource identity; it never substitutes bytes under an old content digest.
pub fn with_motion_fonts(
    package_json: &str,
    timeline_json: &str,
    manifest_json: &str,
    bundle_json: &str,
    resource_bytes: &[u8],
    fonts: &[MotionFontBytes],
) -> Result<MotionFontPackage, String> {
    let profile = canonical_fixed_execution_profile(COMMON_PROFILE_KEY)?;
    let files = fixed_package_files(
        timeline_json,
        manifest_json,
        bundle_json,
        &profile,
        resource_bytes,
    );
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
    let mut binary = ResourceBytes {
        bytes: resource_bytes.to_vec(),
        ranges: BTreeMap::new(),
    };
    for font in fonts {
        let bytes = &font.bytes;
        let digest = ContentDigest::of_bytes(bytes);
        for descriptor in font_face_descriptors(bytes)? {
            let id = format!("font:{}:{}", digest.as_hex(), descriptor.face_index);
            if font_ids.contains(&id) {
                continue;
            }
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
                            bytes: binary.append(bytes),
                        },
                        dependencies: Vec::new(),
                    })
                    .map_err(|error| error.to_string())?,
                );
            }
            font_ids.push(id);
        }
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
        &binary.bytes,
    );
    let fixed_package_manifest_json = canonical_fixed_package_manifest(&files)?;
    open_verified_fixed_package(&fixed_package_manifest_json, &files)
        .map_err(|error| error.to_string())?;
    Ok(MotionFontPackage {
        fixed_package_manifest_json,
        timeline_json: timeline_json.to_owned(),
        resource_manifest_json,
        verified_binding_bundle_json,
        resource_bytes: binary.bytes,
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

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn two_face_collection() -> Vec<u8> {
        let fonts: [&[u8]; 2] = [
            include_bytes!("../../../../assets/fonts/katex/KaTeX_Main-Regular.ttf"),
            include_bytes!("../../../../assets/fonts/katex/KaTeX_Main-Bold.ttf"),
        ];
        let mut bytes = b"ttcf\0\x01\0\0\0\0\0\x02\0\0\0\0\0\0\0\0".to_vec();
        for (index, font) in fonts.into_iter().enumerate() {
            let start = bytes.len();
            bytes[12 + index * 4..16 + index * 4].copy_from_slice(&(start as u32).to_be_bytes());
            let mut face = font.to_vec();
            let count = u16::from_be_bytes(face[4..6].try_into().unwrap()) as usize;
            for table in 0..count {
                let at = 12 + table * 16 + 8;
                let offset = u32::from_be_bytes(face[at..at + 4].try_into().unwrap());
                face[at..at + 4].copy_from_slice(&(offset + start as u32).to_be_bytes());
            }
            bytes.extend_from_slice(&face);
            bytes.resize(bytes.len().next_multiple_of(4), 0);
        }
        bytes
    }

    #[test]
    fn selected_font_faces_preserve_tables_and_reject_bad_offsets() {
        let bytes = two_face_collection();
        assert_eq!(font_face_descriptors(&bytes).unwrap().len(), 2);
        for index in 0..2 {
            let standalone = font_face_bytes(&bytes, index).unwrap();
            let before = ttf_parser::Face::parse(&bytes, index).unwrap();
            let after = ttf_parser::Face::parse(&standalone, 0).unwrap();
            assert_eq!(before.weight(), after.weight());
            assert_eq!(before.number_of_glyphs(), after.number_of_glyphs());
            for tag in [b"glyf", b"cmap", b"hmtx", b"name"] {
                let tag = ttf_parser::Tag::from_bytes(tag);
                assert_eq!(before.raw_face().table(tag), after.raw_face().table(tag));
            }
            assert_eq!(
                standalone.chunks_exact(4).fold(0u32, |sum, word| sum
                    .wrapping_add(u32::from_be_bytes(word.try_into().unwrap()))),
                0xb1b0afba
            );
        }
        assert!(font_face_bytes(&bytes, 2).is_err());
        assert!(font_face_bytes(&bytes[..16], 0).is_err());
        let mut invalid = bytes.clone();
        invalid[12..16].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(font_face_bytes(&invalid, 0).is_err());
    }
}
