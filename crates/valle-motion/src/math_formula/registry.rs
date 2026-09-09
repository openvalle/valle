//! Formula-only font registry. Never injected into Takumi/Parley.

use std::collections::BTreeMap;
use std::path::Path;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::OnceLock;

use ratex_font::FontId;
use valle_draw::program::recording::FontFace;

use super::admit::AdmitError;
use super::fonts::{
    FORMULA_FACES, FormulaFace, REJECTED_FALLBACK_FONT_NAMES, face_matches_lock, formula_faces,
};
use crate::emit::default_font_naming;

#[derive(Debug, Clone)]
pub struct LoadedFace {
    pub spec: FormulaFace,
    pub bytes: Vec<u8>,
    pub family: String,
}

#[derive(Debug, Clone, Default)]
pub struct FormulaFontRegistry {
    by_ratex_name: BTreeMap<&'static str, LoadedFace>,
}

impl FormulaFontRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: FormulaFace, bytes: Vec<u8>) -> Result<(), AdmitError> {
        if !face_matches_lock(&spec, &bytes) {
            return Err(AdmitError::UnknownFont {
                name: format!("{} hash mismatch", spec.file_name),
            });
        }
        ttf_parser::Face::parse(&bytes, 0).map_err(|err| AdmitError::UnknownFont {
            name: format!("{}: {err}", spec.file_name),
        })?;
        let family = default_font_naming(&bytes, 0, 0.0).family;
        self.by_ratex_name.insert(
            spec.ratex_name,
            LoadedFace {
                spec,
                bytes,
                family,
            },
        );
        Ok(())
    }

    pub fn load_dir(dir: &Path) -> Result<Self, AdmitError> {
        let mut registry = Self::new();
        for face in formula_faces() {
            let path = dir.join(face.file_name);
            let bytes = std::fs::read(&path).map_err(|err| AdmitError::UnknownFont {
                name: format!("{}: {err}", path.display()),
            })?;
            registry.insert(*face, bytes)?;
        }
        if registry.len() != FORMULA_FACES.len() {
            return Err(AdmitError::UnknownFont {
                name: "incomplete formula face set".into(),
            });
        }
        Ok(registry)
    }

    pub fn len(&self) -> usize {
        self.by_ratex_name.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_ratex_name.is_empty()
    }

    pub fn get(&self, ratex_name: &str) -> Result<&LoadedFace, AdmitError> {
        if REJECTED_FALLBACK_FONT_NAMES.contains(&ratex_name) {
            return Err(AdmitError::CjkOrEmoji {
                detail: ratex_name.into(),
            });
        }
        self.by_ratex_name
            .get(ratex_name)
            .ok_or_else(|| AdmitError::UnknownFont {
                name: ratex_name.into(),
            })
    }

    pub fn font_face(&self, ratex_name: &str, size: f64) -> Result<FontFace, AdmitError> {
        let loaded = self.get(ratex_name)?;
        Ok(FontFace {
            family: loaded.family.clone(),
            weight: 400,
            italic: false,
            size,
        })
    }

    /// Load the locked 19-face pack from Native embedded bytes.
    /// Browser layout uses the formula faces supplied by the render resources.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load_embedded() -> Result<Self, AdmitError> {
        let mut registry = Self::new();
        for (face, bytes) in super::fonts::formula_font_pack() {
            registry.insert(face, bytes.to_vec())?;
        }
        if registry.len() != FORMULA_FACES.len() {
            return Err(AdmitError::UnknownFont {
                name: "incomplete formula face set".into(),
            });
        }
        Ok(registry)
    }

    /// `char_code` → `katex_ttf_glyph_char` → same TTF cmap → gid.
    pub fn glyph_id(&self, ratex_name: &str, char_code: u32) -> Result<u32, AdmitError> {
        let loaded = self.get(ratex_name)?;
        let font_id = FontId::parse(ratex_name).ok_or_else(|| AdmitError::UnknownFont {
            name: ratex_name.into(),
        })?;
        if matches!(
            font_id,
            FontId::CjkRegular | FontId::CjkFallback | FontId::EmojiFallback
        ) {
            return Err(AdmitError::CjkOrEmoji {
                detail: ratex_name.into(),
            });
        }
        let mapped = ratex_font::katex_ttf_glyph_char(font_id, char_code);
        let face =
            ttf_parser::Face::parse(&loaded.bytes, 0).map_err(|err| AdmitError::UnknownFont {
                name: format!("{}: {err}", loaded.spec.file_name),
            })?;
        let gid = face
            .glyph_index(mapped)
            .ok_or_else(|| AdmitError::UnknownFont {
                name: format!(
                    "{ratex_name} missing U+{:04X} (mapped {mapped:?})",
                    char_code
                ),
            })?;
        Ok(u32::from(gid.0))
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn load_default() -> Result<&'static Self, AdmitError> {
        static REGISTRY: OnceLock<Result<FormulaFontRegistry, String>> = OnceLock::new();
        match REGISTRY.get_or_init(|| Self::load_embedded().map_err(|err| err.to_string())) {
            Ok(registry) => Ok(registry),
            Err(err) => Err(AdmitError::UnknownFont { name: err.clone() }),
        }
    }

    /// Direct cmap lookup without the KaTeX math-alnum remap. Tests use this to
    /// prove raw U+1D4xx codepoints are not in the TTF.
    pub fn raw_cmap_gid(&self, ratex_name: &str, ch: char) -> Result<Option<u32>, AdmitError> {
        let loaded = self.get(ratex_name)?;
        let face =
            ttf_parser::Face::parse(&loaded.bytes, 0).map_err(|err| AdmitError::UnknownFont {
                name: format!("{}: {err}", loaded.spec.file_name),
            })?;
        Ok(face.glyph_index(ch).map(|gid| u32::from(gid.0)))
    }
}
