//! Text: paragraphs shaped by parley in the built-in fonts, broken into lines and stacked as
//! Chrome stacks them. No system font is read, so every host shapes with the same bytes.
//!
//! A paragraph is shaped once per text and font and kept while it is in use; measuring it at a
//! width only breaks its lines again.

use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

pub use parley::fontique::Blob;
use parley::fontique::{Collection, CollectionOptions, GenericFamily, SourceCache};
use parley::{
    Alignment, AlignmentOptions, FontContext, FontFamily, FontFamilyName, FontWeight,
    LayoutContext, PositionedLayoutItem, StyleProperty,
};

use crate::style::{Family, Font, LineHeight, TextAlign};
use crate::template::Error;

/// A font face as shaping chose it: a font file, the face's index in it, and its variation
/// coordinates (normalized, 2.14 fixed point).
#[derive(Clone, Debug, PartialEq)]
pub struct Face {
    pub data: Blob<u8>,
    pub index: u32,
    pub coords: Vec<i16>,
}

/// A glyph of `Text::faces()[face]`, in px from the text box's top left, `y` on its baseline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glyph {
    pub face: u32,
    pub id: u32,
    pub x: f32,
    pub y: f32,
}

/// A paragraph broken into lines at a width and aligned in it.
#[derive(Clone, Debug, PartialEq)]
pub struct Lines {
    /// The font size, px.
    pub size: f32,
    pub glyphs: Vec<Glyph>,
    /// Each line box's top and height.
    pub boxes: Vec<[f32; 2]>,
    /// Each line's text: a byte range of the text as `collapse` leaves it.
    pub text: Vec<Range<usize>>,
}

/// What shaping a paragraph depends on.
#[derive(Clone, PartialEq, Eq, Hash)]
struct Key {
    text: Box<str>,
    family: Rc<[Family]>,
    /// f32 bits.
    size: u32,
    weight: u32,
    spacing: u32,
}

struct Paragraph {
    layout: parley::Layout<()>,
    /// Nothing is left once white space collapses: no line boxes, as in CSS (parley still makes
    /// one line).
    empty: bool,
    /// The width its lines were last broken at.
    broken: Option<Option<f32>>,
    used: bool,
}

pub struct Text {
    fonts: FontContext,
    layouts: LayoutContext<()>,
    paragraphs: HashMap<Key, Paragraph>,
    faces: Vec<Face>,
}

impl Text {
    /// Text in the built-in families. Only `sans-serif` exists so far.
    pub fn new(sans_serif: Blob<u8>) -> Result<Text, Error> {
        let mut collection = Collection::new(CollectionOptions {
            shared: false,
            system_fonts: false,
        });
        let families = collection.register_fonts(sans_serif, None);
        if families.is_empty() {
            return Err("the sans-serif font has no families".into());
        }
        collection
            .set_generic_families(GenericFamily::SansSerif, families.iter().map(|(id, _)| *id));
        Ok(Text {
            fonts: FontContext {
                collection,
                source_cache: SourceCache::new(Default::default()),
            },
            layouts: LayoutContext::new(),
            paragraphs: HashMap::new(),
            faces: Vec::new(),
        })
    }

    /// The faces glyphs name.
    pub fn faces(&self) -> &[Face] {
        &self.faces
    }

    /// The box `text` takes in `font` with its lines broken to fit `width` px (`None`: only at
    /// forced breaks; 0: at every opportunity, the min-content width). Widths round up to 1/64
    /// px, as Chrome's layout units do, so the text fits again in the width it measures.
    ///
    /// Text that breaks takes all of the width, unless a word is wider: a box sized to its
    /// content is as wide as it may be once the content is wider (CSS's fit-content).
    pub fn measure(
        &mut self,
        text: &str,
        font: &Font,
        width: Option<f32>,
    ) -> Result<[f32; 2], Error> {
        let p = self.paragraph(text, font, width)?;
        if p.empty {
            return Ok([0.0, 0.0]);
        }
        let boxes = stack(&p.layout, font);
        let height = boxes.last().map_or(0.0, |[top, h]| top + h);
        let widest = (p.layout.width() * 64.0).ceil() / 64.0;
        match width {
            Some(w) if boxes.len() > 1 => Ok([widest.max(w), height]),
            _ => Ok([widest, height]),
        }
    }

    /// The baseline of `text`'s first line from the top of its box, its lines broken to fit
    /// `width` px; empty text has no lines, and no baseline.
    pub fn baseline(&mut self, text: &str, font: &Font, width: f32) -> Result<Option<f32>, Error> {
        let p = self.paragraph(text, font, Some(width))?;
        if p.empty {
            return Ok(None);
        }
        Ok(p.layout.lines().next().map(|line| above(&line, font)))
    }

    /// `text` in `font`, broken to fit `width` px and aligned in it.
    pub fn lines(&mut self, text: &str, font: &Font, width: f32) -> Result<Lines, Error> {
        let alignment = match font.align {
            TextAlign::Start => Alignment::Start,
            TextAlign::End => Alignment::End,
            TextAlign::Left => Alignment::Left,
            TextAlign::Right => Alignment::Right,
            TextAlign::Center => Alignment::Center,
        };
        let p = self.paragraph(text, font, Some(width))?;
        if p.empty {
            return Ok(Lines {
                size: font.size as f32,
                glyphs: Vec::new(),
                boxes: Vec::new(),
                text: Vec::new(),
            });
        }
        p.layout.align(alignment, AlignmentOptions::default());
        let boxes = stack(&p.layout, font);
        let text = p.layout.lines().map(|line| line.text_range()).collect();
        let mut runs = Vec::new();
        for (line, [top, _]) in p.layout.lines().zip(&boxes) {
            let baseline = top + above(&line, font);
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(run) = item else {
                    continue;
                };
                let face = run.run().font();
                let face = Face {
                    data: face.data.clone(),
                    index: face.index,
                    coords: run
                        .run()
                        .normalized_coords()
                        .iter()
                        .map(|c| c.to_bits())
                        .collect(),
                };
                let mut x = run.offset();
                let mut glyphs = Vec::new();
                for g in run.glyphs() {
                    glyphs.push((g.id, x + g.x, baseline + g.y));
                    x += g.advance;
                }
                runs.push((face, glyphs));
            }
        }
        let mut glyphs = Vec::new();
        for (face, list) in runs {
            let face = self.face(face);
            glyphs.extend(list.into_iter().map(|(id, x, y)| Glyph { face, id, x, y }));
        }
        Ok(Lines {
            size: font.size as f32,
            glyphs,
            boxes,
            text,
        })
    }

    /// Forgets the paragraphs not used since the last sweep.
    pub fn sweep(&mut self) {
        self.paragraphs.retain(|_, p| std::mem::take(&mut p.used));
    }

    fn face(&mut self, face: Face) -> u32 {
        let i = match self.faces.iter().position(|f| {
            f.data.id() == face.data.id() && f.index == face.index && f.coords == face.coords
        }) {
            Some(i) => i,
            None => {
                self.faces.push(face);
                self.faces.len() - 1
            }
        };
        i as u32
    }

    /// The paragraph of `text` in `font`, shaped if it is new, its lines broken at `width`.
    fn paragraph(
        &mut self,
        text: &str,
        font: &Font,
        width: Option<f32>,
    ) -> Result<&mut Paragraph, Error> {
        let key = Key {
            text: text.into(),
            family: font.family.clone(),
            size: (font.size as f32).to_bits(),
            weight: (font.weight as f32).to_bits(),
            spacing: (font.spacing as f32).to_bits(),
        };
        if !self.paragraphs.contains_key(&key) {
            let families = families(&font.family)?;
            let text = collapse(text);
            let mut builder = self
                .layouts
                .ranged_builder(&mut self.fonts, &text, 1.0, false);
            builder.push_default(StyleProperty::FontFamily(FontFamily::List(Cow::Borrowed(
                &families,
            ))));
            builder.push_default(StyleProperty::FontSize(font.size as f32));
            builder.push_default(StyleProperty::FontWeight(FontWeight::new(
                font.weight as f32,
            )));
            builder.push_default(StyleProperty::LetterSpacing(font.spacing as f32));
            let layout = builder.build(&*text);
            let paragraph = Paragraph {
                layout,
                empty: text.is_empty(),
                broken: None,
                used: false,
            };
            self.paragraphs.insert(key.clone(), paragraph);
        }
        let p = self
            .paragraphs
            .get_mut(&key)
            .ok_or("engine bug: a paragraph went missing")?;
        p.used = true;
        if p.broken != Some(width) {
            p.layout.break_all_lines(width);
            p.broken = Some(width);
        }
        Ok(p)
    }
}

/// `white-space: normal`: each run of spaces, tabs and line breaks is one space, and none starts
/// or ends the text. (Parley's own collapsing keeps a line feed as a forced break.)
pub fn collapse(text: &str) -> Cow<'_, str> {
    let white = |c: char| matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0C');
    let text = text.trim_matches(white);
    if !text.contains(|c: char| white(c) && c != ' ') && !text.contains("  ") {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut space = false;
    for c in text.chars() {
        if white(c) {
            space = true;
        } else {
            if space {
                out.push(' ');
                space = false;
            }
            out.push(c);
        }
    }
    Cow::Owned(out)
}

fn families(list: &[Family]) -> Result<Vec<FontFamilyName<'static>>, Error> {
    list.iter()
        .map(|f| match f {
            Family::SansSerif => Ok(FontFamilyName::Generic(GenericFamily::SansSerif)),
            Family::Serif => Err("font family serif is not built in yet".into()),
            Family::Monospace => Err("font family monospace is not built in yet".into()),
            Family::Named(name) => Err(format!(
                "font family {name}: font resources are not supported yet"
            )),
        })
        .collect()
}

/// A run's ascent above its baseline and descent below it in a line box of `font`'s line
/// height, as Chrome places them: ascent and descent rounded to whole px, then half the leading
/// left (floored) above, the rest below.
fn run_extent(run: &parley::GlyphRun<'_, ()>, font: &Font) -> [f32; 2] {
    let m = run.run().font_metrics();
    let (ascent, descent) = (m.ascent.round(), m.descent.round());
    // Chrome keeps a line height in its layout unit, 1/64 px: a factor of the font size floors to
    // it, a length rounds.
    let height = match font.line_height {
        LineHeight::Normal => ascent + descent + m.leading.round(),
        LineHeight::Factor(f) => (f as f32 * font.size as f32 * 64.0).floor() / 64.0,
        LineHeight::Px(px) => (px as f32 * 64.0).round() / 64.0,
    };
    let above = ascent + ((height - (ascent + descent)) * 0.5).floor();
    [above, height - above]
}

/// How far the line's baseline sits below its top.
fn above(line: &parley::Line<'_, ()>, font: &Font) -> f32 {
    extent(line, font)[0]
}

fn extent(line: &parley::Line<'_, ()>, font: &Font) -> [f32; 2] {
    let mut out: Option<[f32; 2]> = None;
    for item in line.items() {
        if let PositionedLayoutItem::GlyphRun(run) = item {
            let [a, b] = run_extent(&run, font);
            out = Some(out.map_or([a, b], |[x, y]| [x.max(a), y.max(b)]));
        }
    }
    // A line without glyphs keeps parley's own metrics.
    out.unwrap_or_else(|| {
        let m = line.metrics();
        let above = m.baseline - m.block_min_coord;
        [above, m.line_height - above]
    })
}

/// Each line box's top and height, stacked from 0.
fn stack(layout: &parley::Layout<()>, font: &Font) -> Vec<[f32; 2]> {
    let mut top = 0.0;
    layout
        .lines()
        .map(|line| {
            let [a, b] = extent(&line, font);
            let line_box = [top, a + b];
            top += a + b;
            line_box
        })
        .collect()
}

#[cfg(test)]
mod tests;
