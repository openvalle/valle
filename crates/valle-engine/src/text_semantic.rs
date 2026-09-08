//! Fixed-package caption shaping semantics.
//!
//! Authoring text graphs do not cross this boundary. The only input is the already-admitted
//! caption packet projected from [`crate::render::CompiledRender`]; output is the shared
//! structured [`valle_draw::program::DrawProgram`] narrow waist.

use std::{collections::HashMap, sync::Arc};

use cosmic_text::{FontSystem, fontdb};

#[path = "text_program.rs"]
mod text_program;
/// One evaluated caption packet from an immutable compiled render.
pub(crate) struct CaptionDrawInput<'a> {
    pub runs: &'a [String],
    pub run_styles: &'a [CaptionRunStyle],
    pub font_family: &'a str,
    pub shadow: Option<CaptionShadow>,
    pub region: [f64; 4],
    /// Horizontal and vertical alignment fractions in the closed `[0, 1]` interval.
    pub align: [f64; 2],
    pub presentation: CaptionPresentation,
    pub behavior: CaptionBehavior,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CaptionRunStyle {
    pub font_size: f64,
    pub color: [u8; 4],
    pub font_weight: u16,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CaptionShadow {
    pub color: [u8; 4],
    pub offset: [f64; 2],
    pub blur_sigma: f64,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CaptionPresentation {
    pub opacity: f64,
    pub translation: [f64; 2],
    pub scale: f64,
    pub rotation: f64,
    pub clip_inset: [f64; 4],
    pub blur_sigma: f64,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum CaptionBehavior {
    Static,
    Scroll {
        horizontal: bool,
        speed: f64,
        elapsed_seconds: f64,
    },
    Karaoke {
        /// UTF-8 byte boundary in the concatenated caption text.
        reveal_bytes: usize,
    },
}

const DEFAULT_LINE_SPACING: f64 = 1.2;

#[cfg(test)]
const BUNDLED_FALLBACK_FONT: &[u8] =
    include_bytes!("../../../assets/fonts/noto/NotoSansCJKsc-Regular.otf");

/// Render-scoped deterministic font system.
///
/// Every selectable face is registered from a verified compiled resource and carries its content
/// identity into the resulting `GlyphRun`. No host/system-font fallback is admitted.
pub struct TextSemantics {
    fs: FontSystem,
    program_fonts: HashMap<fontdb::ID, valle_draw::requirements::FontKey>,
    program_face_ids: HashMap<(String, u32), fontdb::ID>,
    program_aliases: HashMap<(String, u32, String), fontdb::ID>,
}

impl TextSemantics {
    pub fn product() -> Self {
        Self {
            fs: FontSystem::new_with_locale_and_db("en-US".into(), fontdb::Database::new()),
            program_fonts: HashMap::new(),
            program_face_ids: HashMap::new(),
            program_aliases: HashMap::new(),
        }
    }

    /// Register one descriptor-selected, content-addressed face under an admitted family alias.
    pub fn register_program_font(
        &mut self,
        alias: &str,
        face_hash: &str,
        face_index: u32,
        bytes: Arc<Vec<u8>>,
    ) -> Result<(), String> {
        let identity = (face_hash.to_owned(), face_index);
        let alias_key = (face_hash.to_owned(), face_index, alias.to_owned());
        if self.program_aliases.contains_key(&alias_key) {
            return Ok(());
        }

        let canonical_id = if let Some(id) = self.program_face_ids.get(&identity).copied() {
            id
        } else {
            let ids: Vec<fontdb::ID> = self
                .fs
                .db_mut()
                .load_font_source(fontdb::Source::Binary(bytes))
                .into_iter()
                .collect();
            let selected = ids
                .iter()
                .copied()
                .find(|id| {
                    self.fs
                        .db()
                        .face(*id)
                        .is_some_and(|face| face.index == face_index)
                })
                .ok_or_else(|| {
                    format!("font {face_hash}:{face_index} has no matching fontdb face")
                })?;
            // A TTC can expose several faces. Only the descriptor-selected face belongs to this
            // fixed render; siblings must not remain queryable as unkeyed fallback candidates.
            for id in ids {
                if id != selected {
                    self.fs.db_mut().remove_face(id);
                }
            }
            self.program_face_ids.insert(identity, selected);
            selected
        };

        let face = self
            .fs
            .db()
            .face(canonical_id)
            .cloned()
            .ok_or_else(|| format!("font {face_hash}:{face_index} disappeared from fontdb"))?;
        let key = valle_draw::requirements::FontKey {
            face_hash: valle_draw::requirements::DigestBytes::from_hex(face_hash)
                .ok_or_else(|| format!("font {face_hash}:{face_index} has an invalid digest"))?,
            face_index,
        };
        self.program_fonts.insert(canonical_id, key.clone());

        let language = face
            .families
            .first()
            .map(|(_, language)| *language)
            .unwrap_or(fontdb::Language::English_UnitedStates);
        let mut alias_face = face;
        alias_face.id = fontdb::ID::dummy();
        alias_face.families.insert(0, (alias.to_owned(), language));
        let alias_id = self.fs.db_mut().push_face_info(alias_face);
        self.program_fonts.insert(alias_id, key);
        self.program_aliases.insert(alias_key, alias_id);
        if alias == crate::resource::DEFAULT_FONT_FAMILY {
            self.fs.db_mut().set_sans_serif_family(alias.to_owned());
        }
        Ok(())
    }
}
