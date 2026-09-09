//! Built-in font identities, without font bytes. Other fonts are ordinary render resources.

#[derive(serde::Deserialize)]
pub struct FontSpec {
    pub name: String,
    pub sha256: String,
}

pub fn specs() -> &'static [FontSpec] {
    static SPECS: std::sync::OnceLock<Vec<FontSpec>> = std::sync::OnceLock::new();
    SPECS.get_or_init(|| {
        serde_json::from_str(include_str!("runtime-fonts.json")).expect("font identity manifest")
    })
}

pub fn default_index(digest: &crate::ContentDigest) -> Option<usize> {
    let hex = hex::encode(digest.as_bytes());
    let spec = specs().iter().find(|spec| spec.sha256 == hex)?;
    crate::DEFAULT_MOTION_FONT_FILES
        .iter()
        .position(|name| *name == spec.name)
}

#[cfg(test)]
mod tests {
    #[test]
    fn manifest_matches_native_fonts() {
        let mut fonts = std::collections::BTreeMap::new();
        for (name, bytes) in crate::DEFAULT_MOTION_FONT_FILES
            .iter()
            .zip(crate::default_motion_fonts())
        {
            fonts.insert(*name, *bytes);
        }
        for (face, bytes) in crate::math_formula::formula_font_pack() {
            fonts.insert(face.file_name, bytes);
        }
        assert_eq!(fonts.len(), super::specs().len());
        for spec in super::specs() {
            assert_eq!(
                hex::encode(crate::ContentDigest::of_bytes(fonts[spec.name.as_str()]).as_bytes()),
                spec.sha256
            );
        }
    }
}
