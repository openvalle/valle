//! Font-dependent small/petite-cap fallback, resolved before line breaking. Synthetic casing
//! keeps a map to authored bytes, including one character expanding to several glyphs.
use std::ops::Range;
use std::rc::Rc;

use parley::PositionedLayoutItem;
use skrifa::raw::TableProvider;

use super::{Content, Text, bidi};
use crate::style::{Feature, Font, Variant};

pub(super) struct Adjustment {
    pub range: Range<usize>,
    pub features: Option<Rc<[Feature]>>,
    pub size: Option<f32>,
}

#[derive(Clone, Copy)]
enum Case {
    Lower,
    Upper,
    All,
}

struct Action {
    range: Range<usize>,
    features: Option<Rc<[Feature]>>,
    case: Option<Case>,
    size: f32,
}

fn fallback(font: &Font, data: &[u8], index: u32) -> Option<(Vec<Feature>, Option<Case>)> {
    let caps = &font.variants[Variant::Caps as usize];
    if caps.is_empty() {
        return None;
    }
    if caps.iter().all(|f| f.tag == *b"titl") {
        return None;
    }
    let asked = |tag: &[u8; 4]| caps.iter().any(|f| f.tag == *tag && f.value != 0);
    let raw = skrifa::FontRef::from_index(data, index).ok()?;
    let available = |tag: &[u8; 4]| {
        raw.gsub()
            .ok()
            .and_then(|g| g.feature_list().ok())
            .is_some_and(|f| {
                f.feature_records()
                    .iter()
                    .any(|r| r.feature_tag().to_be_bytes() == *tag)
            })
    };
    if caps.iter().all(|f| f.value == 0 || available(&f.tag)) {
        return None;
    }
    let (tags, case): (&[[u8; 4]], Case) = if asked(b"unic") {
        (&[*b"c2sc"], Case::Upper)
    } else if asked(b"c2pc") || asked(b"c2sc") {
        (&[*b"smcp", *b"c2sc"], Case::All)
    } else {
        (&[*b"smcp"], Case::Lower)
    };
    let native = tags.iter().all(available);
    let features: Vec<_> = font
        .variants
        .iter()
        .enumerate()
        .flat_map(|(i, v)| {
            if i == Variant::Caps as usize {
                if native {
                    tags.iter().map(|&tag| Feature { tag, value: 1 }).collect()
                } else {
                    Vec::new()
                }
            } else {
                v.to_vec()
            }
        })
        .chain(font.features.iter().copied())
        .collect();
    Some((features, (!native).then_some(case)))
}

pub(super) fn vertical(
    font: &Font,
    face: &super::Face,
    value: &str,
) -> Option<(String, Vec<Feature>, Vec<usize>)> {
    let (features, case) = fallback(font, face.data.data(), face.index)?;
    let mut text = String::new();
    let mut offsets = Vec::new();
    for (at, c) in value.char_indices() {
        let before = text.len();
        let change = case.is_some_and(|case| case.applies(c));
        if change {
            text.extend(c.to_uppercase());
        } else {
            text.push(c);
        }
        offsets.extend(std::iter::repeat_n(at, text.len() - before));
    }
    offsets.push(value.len());
    Some((text, features, offsets))
}

impl Case {
    fn applies(self, c: char) -> bool {
        match self {
            Case::Lower => c.is_lowercase(),
            Case::Upper => c.is_uppercase(),
            Case::All => c.is_lowercase() || c.is_uppercase(),
        }
    }
}

pub(super) fn prepare(
    text: &mut Text,
    input: &mut bidi::Input,
    content: &Content,
) -> Result<Vec<Adjustment>, String> {
    let mut actions = Vec::new();
    for (piece, range) in content.pieces.iter().zip(&input.ranges) {
        if piece.edge.is_some()
            || piece.font.variants[Variant::Caps as usize].is_empty()
            || range.is_empty()
        {
            continue;
        }
        let layout = text.alone(&input.text[range.clone()], piece.font)?;
        for line in layout.lines() {
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(run) = item else {
                    continue;
                };
                let run = run.run();
                let Some((features, case)) =
                    fallback(piece.font, run.font().data.data(), run.font().index)
                else {
                    continue;
                };
                let r = run.text_range();
                actions.push(Action {
                    range: range.start + r.start..range.start + r.end,
                    features: Some(features.into()),
                    case,
                    size: piece.font.size as f32,
                });
            }
        }
    }
    if actions.is_empty() {
        return Ok(Vec::new());
    }
    actions.sort_by_key(|a| a.range.start);
    let mut value = String::new();
    let mut offsets = Vec::new();
    let mut boundaries = vec![0; input.text.len() + 1];
    let mut adjusted: Vec<Adjustment> = Vec::new();
    let mut index = 0;
    for (at, c) in input.text.char_indices() {
        while actions.get(index).is_some_and(|a| a.range.end <= at) {
            index += 1;
        }
        let action = actions.get(index).filter(|a| a.range.contains(&at));
        let before = value.len();
        let change = action
            .and_then(|a| a.case)
            .is_some_and(|mode| mode.applies(c))
            && !input.map.inserted(at);
        if change {
            value.extend(c.to_uppercase());
        } else {
            value.push(c);
        }
        boundaries[at..at + c.len_utf8()].fill(before);
        if change {
            offsets.extend(std::iter::repeat_n(at, value.len() - before));
        } else {
            offsets.extend(at..at + c.len_utf8());
        }
        if let Some(a) = action {
            let size = change.then_some(a.size * 0.8);
            if let Some(last) = adjusted.last_mut()
                && last.range.end == before
                && last.features == a.features
                && last.size == size
            {
                last.range.end = value.len();
            } else {
                adjusted.push(Adjustment {
                    range: before..value.len(),
                    features: a.features.clone(),
                    size,
                });
            }
        }
    }
    boundaries[input.text.len()] = value.len();
    offsets.push(input.text.len());
    input.map = input.map.remap(&offsets);
    for range in &mut input.ranges {
        *range = boundaries[range.start]..boundaries[range.end];
    }
    input.text = value.into();
    Ok(adjusted)
}
