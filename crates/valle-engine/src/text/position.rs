//! Typographic sub/superscripts. A partially supported run is synthesized as a whole,
//! preserving CSS font size and line metrics while shaping smaller ordinary glyphs.
use super::{Content, Face, Text};
use crate::style::{Font, Variant};
use parley::PositionedLayoutItem;

#[derive(Clone, Copy)]
pub(super) struct Adjustment {
    pub scale: f32,
    /// Downward displacement in computed CSS pixels; size-adjust does not change it.
    pub shift: f32,
}

pub(super) fn prepare(
    text: &mut Text,
    content: &Content,
    input: &super::bidi::Input,
) -> Result<Vec<Option<Adjustment>>, String> {
    if content
        .pieces
        .iter()
        .all(|p| p.font.variants[Variant::Position as usize].is_empty())
    {
        return Ok(Vec::new());
    }
    content
        .pieces
        .iter()
        .zip(&input.ranges)
        .map(|(piece, range)| {
            let position = &piece.font.variants[Variant::Position as usize];
            let Some(tag) = position
                .first()
                .filter(|_| piece.edge.is_none() && !piece.text.is_empty())
                .map(|f| f.tag)
            else {
                return Ok(None);
            };
            let value = &input.text[range.clone()];
            let layout = text.alone(value, piece.font)?;
            let mut missing = false;
            for line in layout.lines() {
                for item in line.items() {
                    let PositionedLayoutItem::GlyphRun(run) = item else {
                        continue;
                    };
                    let value = &value[run.run().text_range()];
                    missing |=
                        !available(&Face::of(run.run(), piece.font), value, piece.font, tag)?;
                }
            }
            Ok(missing.then_some(Adjustment {
                scale: scale(piece.font.size as f32),
                shift: piece.font.size as f32 * if tag == *b"sups" { -0.34 } else { 0.2 },
            }))
        })
        .collect()
}

/// Check actual substitutions, not just the presence of the feature tag: many fonts
/// provide numerals but no letters. Combining characters share their cluster's result.
fn available(face: &Face, value: &str, font: &Font, tag: [u8; 4]) -> Result<bool, String> {
    let file =
        harfrust::FontRef::from_index(face.data.data(), face.index).map_err(|e| e.to_string())?;
    let data = harfrust::ShaperData::new(&file);
    let mut instance = harfrust::ShaperInstance::default();
    instance.set_coords(
        &file,
        face.coords
            .iter()
            .map(|&c| harfrust::NormalizedCoord::from_bits(c)),
    );
    let shaper = data.shaper(&file).instance(Some(&instance)).build();
    let shape = |enabled| {
        let mut features: Vec<_> = font
            .variants
            .iter()
            .flat_map(|v| v.iter())
            .chain(font.features.iter())
            .map(|f| harfrust::Feature::new(harfrust::Tag::new(&f.tag), u32::from(f.value), ..))
            .collect();
        features.push(harfrust::Feature::new(
            harfrust::Tag::new(&tag),
            enabled,
            ..,
        ));
        let mut buffer = harfrust::UnicodeBuffer::new();
        buffer.push_str(value);
        buffer.guess_segment_properties();
        shaper.shape(buffer, harfrust::ShapeOptions::new().features(&features))
    };
    let (plain, alternate) = (shape(0), shape(1));
    for g in plain.glyph_infos() {
        let at = g.cluster as usize;
        if value[at..].chars().next().is_none_or(|c| {
            c.is_whitespace()
                || c.is_control()
                || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        }) {
            continue;
        }
        let ids = |buffer: &harfrust::GlyphBuffer| {
            buffer
                .glyph_infos()
                .iter()
                .filter(|i| i.cluster == g.cluster)
                .map(|i| i.glyph_id)
                .collect::<Vec<_>>()
        };
        if ids(&plain) == ids(&alternate) {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Preserve readability at small sizes, interpolating to two-thirds at 45px.
fn scale(size: f32) -> f32 {
    let t = ((size - 20.0) / 25.0).clamp(0.0, 1.0);
    (1.0 - t) * 0.82 + t * 0.667
}
