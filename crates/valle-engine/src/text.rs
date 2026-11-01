//! Text: paragraphs shaped by parley in the built-in fonts, broken into lines and stacked as
//! Chrome stacks them. No system font is read, so every host shapes with the same bytes.
//!
//! A paragraph is pieces of text, each in the font of the Text or Span it is in. It is shaped
//! once per text and fonts and kept while it is in use; measuring it at a width only breaks its
//! lines again. Its line boxes stack as Chrome stacks them, each from the paragraph's own font
//! (CSS's strut) and the runs of glyphs on it.

use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

pub use parley::fontique::Blob;
use parley::fontique::{Collection, CollectionOptions, GenericFamily, SourceCache};
use parley::{
    Alignment, AlignmentOptions, FontContext, FontFamily, FontFamilyName, FontWeight,
    LayoutContext, PositionedLayoutItem, StyleProperty, TextWrapMode, WhiteSpaceCollapse,
};

use crate::style::{Collapse, Family, Font, LineHeight, TextAlign, TextTransform};
use crate::template::Error;

/// A font face as shaping chose it: a font file, the face's index in it, and its variation
/// coordinates (normalized, 2.14 fixed point).
#[derive(Clone, Debug, PartialEq)]
pub struct Face {
    pub data: Blob<u8>,
    pub index: u32,
    pub coords: Vec<i16>,
}

/// A paragraph as written: the font of its box, which each line box starts from (CSS's strut)
/// and which aligns its lines, and its pieces of text in order.
pub struct Content<'a> {
    pub font: &'a Font,
    pub pieces: Vec<Piece<'a>>,
}

/// Text in the font of the node it is in, which its glyphs name.
pub struct Piece<'a> {
    pub text: Cow<'a, str>,
    pub node: u32,
    pub font: &'a Font,
}

/// A glyph of `Text::faces()[face]` in the text of `node`, in px from the text box's top left,
/// `y` on its baseline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glyph {
    pub face: u32,
    pub id: u32,
    pub x: f32,
    pub y: f32,
    pub node: u32,
}

/// A run of glyphs of one node on a line, with its left and right edges, less the white space
/// that hangs at the end of the line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Run {
    pub node: u32,
    pub line: u32,
    pub x: [f32; 2],
}

/// A paragraph broken into lines at a width and aligned in it.
#[derive(Clone, Debug, PartialEq)]
pub struct Lines {
    pub glyphs: Vec<Glyph>,
    pub runs: Vec<Run>,
    /// Each line box's top and height, and each line's baseline, from the top.
    pub boxes: Vec<[f32; 2]>,
    pub baselines: Vec<f32>,
    /// The text as white space collapses it, and each line's byte range of it.
    pub string: Rc<str>,
    pub text: Vec<Range<usize>>,
}

/// What shaping a piece and breaking it into lines depend on.
#[derive(Clone, PartialEq, Eq, Hash)]
struct Shaping {
    family: Rc<[Family]>,
    /// f32 bits.
    size: u32,
    weight: u32,
    spacing: u32,
    word_spacing: u32,
    collapse: Collapse,
    wrap: bool,
    transform: TextTransform,
}

impl Shaping {
    fn of(font: &Font) -> Shaping {
        Shaping {
            family: font.family.clone(),
            size: (font.size as f32).to_bits(),
            weight: (font.weight as f32).to_bits(),
            spacing: (font.spacing as f32).to_bits(),
            word_spacing: (font.word_spacing as f32).to_bits(),
            collapse: font.collapse,
            wrap: font.wrap,
            transform: font.transform,
        }
    }
}

/// A paragraph shaped: its glyph runs' brushes are the pieces they are in.
struct Paragraph {
    layout: parley::Layout<u32>,
    string: Rc<str>,
    /// Nothing is left once white space collapses: no line boxes, as in CSS (parley still makes
    /// one line).
    empty: bool,
    /// The width its lines were last broken at.
    broken: Option<Option<f32>>,
    used: bool,
}

pub struct Text {
    fonts: FontContext,
    layouts: LayoutContext<u32>,
    /// By each piece's text and shaping.
    paragraphs: HashMap<Vec<(Box<str>, Shaping)>, Paragraph>,
    /// The ascent, descent and line gap of each font's first face, px.
    metrics: HashMap<Shaping, [f32; 3]>,
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
            metrics: HashMap::new(),
            faces: Vec::new(),
        })
    }

    /// The faces glyphs name.
    pub fn faces(&self) -> &[Face] {
        &self.faces
    }

    /// The box `content` takes with its lines broken to fit `width` px (`None`: only at forced
    /// breaks; 0: at every opportunity, the min-content width). Widths round up to 1/64 px, as
    /// Chrome's layout units do, so the text fits again in the width it measures.
    ///
    /// Text that breaks takes all of the width, unless a word is wider: a box sized to its
    /// content is as wide as it may be once the content is wider (CSS's fit-content).
    pub fn measure(&mut self, content: &Content, width: Option<f32>) -> Result<[f32; 2], Error> {
        let strut = self.strut(content.font)?;
        let p = self.paragraph(content, width)?;
        if p.empty {
            return Ok([0.0, 0.0]);
        }
        let boxes = stack(&p.layout, content, strut);
        let height = boxes.last().map_or(0.0, |[top, h]| top + h);
        let widest = (p.layout.width() * 64.0).ceil() / 64.0;
        match width {
            Some(w) if boxes.len() > 1 => Ok([widest.max(w), height]),
            _ => Ok([widest, height]),
        }
    }

    /// The baseline of `content`'s first line from the top of its box, its lines broken to fit
    /// `width` px; empty text has no lines, and no baseline.
    pub fn baseline(&mut self, content: &Content, width: f32) -> Result<Option<f32>, Error> {
        let strut = self.strut(content.font)?;
        let p = self.paragraph(content, Some(width))?;
        if p.empty {
            return Ok(None);
        }
        Ok(p.layout
            .lines()
            .next()
            .map(|line| extent(&line, content, strut)[0]))
    }

    /// `content` broken to fit `width` px and aligned in it.
    pub fn lines(&mut self, content: &Content, width: f32) -> Result<Lines, Error> {
        let alignment = match content.font.align {
            TextAlign::Start => Alignment::Start,
            TextAlign::End => Alignment::End,
            TextAlign::Left => Alignment::Left,
            TextAlign::Right => Alignment::Right,
            TextAlign::Center => Alignment::Center,
        };
        let strut = self.strut(content.font)?;
        let p = self.paragraph(content, Some(width))?;
        let string = p.string.clone();
        if p.empty {
            return Ok(Lines {
                glyphs: Vec::new(),
                runs: Vec::new(),
                boxes: Vec::new(),
                baselines: Vec::new(),
                string,
                text: Vec::new(),
            });
        }
        p.layout.align(alignment, AlignmentOptions::default());
        let boxes = stack(&p.layout, content, strut);
        let text = p.layout.lines().map(|line| line.text_range()).collect();
        let mut baselines = Vec::new();
        let mut runs = Vec::new();
        let mut shaped = Vec::new();
        for (i, (line, [top, _])) in p.layout.lines().zip(&boxes).enumerate() {
            let baseline = top + extent(&line, content, strut)[0];
            baselines.push(baseline);
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(run) = item else {
                    continue;
                };
                let node = content.pieces[run.style().brush as usize].node;
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
                runs.push(Run {
                    node,
                    line: i as u32,
                    x: [run.offset(), x],
                });
                shaped.push((face, node, glyphs));
            }
            // White space that hangs past the end of the line is not in its runs' boxes.
            let mut hanging = line.metrics().hanging_advance;
            for run in runs.iter_mut().rev().take_while(|r| r.line == i as u32) {
                let cut = hanging.min(run.x[1] - run.x[0]);
                run.x[1] -= cut;
                hanging -= cut;
            }
        }
        let mut glyphs = Vec::new();
        for (face, node, list) in shaped {
            let face = self.face(face);
            glyphs.extend((list.into_iter()).map(|(id, x, y)| Glyph {
                face,
                id,
                x,
                y,
                node,
            }));
        }
        Ok(Lines {
            glyphs,
            runs,
            boxes,
            baselines,
            string,
            text,
        })
    }

    /// The ascent, descent and line gap, px, of the first face of `font`, at its size and
    /// weight, as runs in it have them.
    pub fn metrics(&mut self, font: &Font) -> Result<[f32; 3], Error> {
        let key = Shaping::of(font);
        if let Some(m) = self.metrics.get(&key) {
            return Ok(*m);
        }
        let families = families(&font.family)?;
        let mut builder = self
            .layouts
            .ranged_builder(&mut self.fonts, "0", 1.0, false);
        builder.push_default(StyleProperty::FontFamily(FontFamily::List(Cow::Borrowed(
            &families,
        ))));
        builder.push_default(StyleProperty::FontSize(font.size as f32));
        builder.push_default(StyleProperty::FontWeight(FontWeight::new(
            font.weight as f32,
        )));
        let mut layout = builder.build("0");
        layout.break_all_lines(None);
        let m = (layout.lines())
            .find_map(|line| {
                line.items().find_map(|item| match item {
                    PositionedLayoutItem::GlyphRun(run) => {
                        let m = run.run().font_metrics();
                        Some([m.ascent, m.descent, m.leading])
                    }
                    _ => None,
                })
            })
            .ok_or("engine bug: a font without metrics")?;
        self.metrics.insert(key, m);
        Ok(m)
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

    /// What the line box of a paragraph in `font` takes above and below its baseline, with no
    /// text in it.
    fn strut(&mut self, font: &Font) -> Result<[f32; 2], Error> {
        Ok(placed(self.metrics(font)?, font))
    }

    /// The paragraph of `content`, shaped if it is new, its lines broken at `width`.
    fn paragraph(
        &mut self,
        content: &Content,
        width: Option<f32>,
    ) -> Result<&mut Paragraph, Error> {
        let key: Vec<(Box<str>, Shaping)> = (content.pieces.iter())
            .map(|p| (p.text.as_ref().into(), Shaping::of(p.font)))
            .collect();
        if !self.paragraphs.contains_key(&key) {
            let pieces = (content.pieces.iter())
                .map(|p| (p.text.as_ref(), p.font.collapse, p.font.transform));
            let (string, ranges) = prepare(pieces)?;
            let families = (content.pieces.iter())
                .map(|p| families(&p.font.family))
                .collect::<Result<Vec<_>, _>>()?;
            let mut builder = (self.layouts).ranged_builder(&mut self.fonts, &string, 1.0, false);
            for (k, (piece, range)) in content.pieces.iter().zip(ranges).enumerate() {
                if range.is_empty() {
                    continue;
                }
                let font = piece.font;
                let family = FontFamily::List(Cow::Borrowed(&families[k]));
                builder.push(StyleProperty::FontFamily(family), range.clone());
                builder.push(StyleProperty::FontSize(font.size as f32), range.clone());
                let weight = FontWeight::new(font.weight as f32);
                builder.push(StyleProperty::FontWeight(weight), range.clone());
                let spacing = font.spacing as f32;
                builder.push(StyleProperty::LetterSpacing(spacing), range.clone());
                let spacing = font.word_spacing as f32;
                builder.push(StyleProperty::WordSpacing(spacing), range.clone());
                if !font.wrap {
                    let mode = StyleProperty::TextWrapMode(TextWrapMode::NoWrap);
                    builder.push(mode, range.clone());
                }
                if font.collapse == Collapse::BreakSpaces {
                    let spaces = WhiteSpaceCollapse::BreakSpaces;
                    builder.push(StyleProperty::WhiteSpaceCollapse(spaces), range.clone());
                }
                builder.push(StyleProperty::Brush(k as u32), range);
            }
            let layout = builder.build(&string);
            let paragraph = Paragraph {
                layout,
                empty: string.is_empty(),
                string: string.into(),
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

/// The text of a paragraph's pieces as it is laid out, and each piece's byte range of it: its
/// white space as each piece's `white-space-collapse` has it, by CSS's rules across piece edges,
/// and its letters as its `text-transform` has them. A collapsible space after another one, or at
/// the start or end of the text or of a line, goes; a run of them is one space, in the piece it
/// starts in. (Parley's own collapsing keeps a line feed as a forced break.)
pub fn prepare<'a>(
    pieces: impl Iterator<Item = (&'a str, Collapse, TextTransform)>,
) -> Result<(String, Vec<Range<usize>>), Error> {
    let mut out = String::new();
    let mut ranges: Vec<Range<usize>> = Vec::new();
    // The piece a run of collapsible white space started in, until something follows it.
    let mut space: Option<usize> = None;
    // Whether the last character is in a word, so capitalizing leaves the next one.
    let mut word = false;
    for (k, (text, collapse, transform)) in pieces.enumerate() {
        ranges.push(out.len()..out.len());
        for c in text.chars() {
            let collapsible = match collapse {
                Collapse::Collapse => matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0C'),
                Collapse::PreserveBreaks => matches!(c, ' ' | '\t' | '\r' | '\x0C'),
                Collapse::Preserve | Collapse::BreakSpaces => false,
            };
            if collapsible {
                if space.is_none() && !(out.is_empty() || out.ends_with('\n')) {
                    space = Some(k);
                }
                word = false;
                continue;
            }
            if c == '\t' {
                return Err("tabs in white space that stays are not supported yet".into());
            }
            match space.take() {
                // A line break takes the spaces before it.
                Some(_) if c == '\n' => {}
                Some(j) => {
                    out.push(' ');
                    // The pieces after its own start after it.
                    ranges[j].end = out.len();
                    for r in &mut ranges[j + 1..] {
                        *r = out.len()..out.len();
                    }
                }
                None => {}
            }
            match transform {
                TextTransform::Uppercase => out.extend(c.to_uppercase()),
                TextTransform::Lowercase => out.extend(c.to_lowercase()),
                TextTransform::Capitalize if !word => out.extend(c.to_uppercase()),
                _ => out.push(c),
            }
            // An apostrophe in a word keeps it going: "don't".
            word = c.is_alphanumeric() || (word && matches!(c, '\'' | '\u{2019}'));
        }
        ranges[k].end = out.len();
    }
    Ok((out, ranges))
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

/// What a box in `font`, with ascent, descent and line gap `m`, takes above and below its
/// baseline in a line box of its line height, as Chrome places it: ascent and descent rounded to
/// whole px, then half the leading left (floored) above, the rest below.
fn placed(m: [f32; 3], font: &Font) -> [f32; 2] {
    let (ascent, descent) = (m[0].round(), m[1].round());
    // Chrome keeps a line height in its layout unit, 1/64 px: a factor of the font size floors to
    // it, a length rounds.
    let height = match font.line_height {
        LineHeight::Normal => ascent + descent + m[2].round(),
        LineHeight::Factor(f) => (f as f32 * font.size as f32 * 64.0).floor() / 64.0,
        LineHeight::Px(px) => (px as f32 * 64.0).round() / 64.0,
    };
    let above = ascent + ((height - (ascent + descent)) * 0.5).floor();
    [above, height - above]
}

/// What the line takes above and below its baseline: its strut, and each run of glyphs in the
/// font of its piece.
fn extent(line: &parley::Line<'_, u32>, content: &Content, strut: [f32; 2]) -> [f32; 2] {
    let mut out = strut;
    for item in line.items() {
        if let PositionedLayoutItem::GlyphRun(run) = item {
            let m = run.run().font_metrics();
            let font = content.pieces[run.style().brush as usize].font;
            let [a, b] = placed([m.ascent, m.descent, m.leading], font);
            out = [out[0].max(a), out[1].max(b)];
        }
    }
    out
}

/// Each line box's top and height, stacked from 0.
fn stack(layout: &parley::Layout<u32>, content: &Content, strut: [f32; 2]) -> Vec<[f32; 2]> {
    let mut top = 0.0;
    layout
        .lines()
        .map(|line| {
            let [a, b] = extent(&line, content, strut);
            let line_box = [top, a + b];
            top += a + b;
            line_box
        })
        .collect()
}

#[cfg(test)]
mod tests;
