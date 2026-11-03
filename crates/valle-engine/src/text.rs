//! Text: paragraphs shaped by parley in the built-in fonts and the scene's, broken into lines and
//! stacked as Chrome stacks them. No system font is read, so every host shapes with the same bytes.
//!
//! The built-in fonts are not in the engine: each is a file of Google Fonts at a fixed version,
//! which the host gives the first time text needs it (`Fonts`), checked against its SHA-256.
//!
//! A paragraph is pieces of text, each in the font of the Text or Span it is in. It is shaped
//! once per text and fonts and kept while it is in use; measuring it at a width only breaks its
//! lines again. Its line boxes stack as Chrome stacks them, each from the paragraph's own font
//! (CSS's strut) and the runs of glyphs on it.

use std::borrow::Cow;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::ops::Range;
use std::rc::Rc;

pub use parley::fontique::Blob;
use parley::fontique::{
    Collection, CollectionOptions, FamilyId, FontInfoOverride, GenericFamily, SourceCache,
};
use parley::setting::Tag;
use parley::{
    Alignment, AlignmentOptions, BreakReason, FontContext, FontFamily, FontFamilyName, FontFeature,
    FontFeatures, FontVariation, FontVariations, FontWeight, FontWidth, IndentOptions,
    LayoutContext, PositionedLayoutItem, StyleProperty, TextWrapMode, WhiteSpaceCollapse,
    YieldData,
};

use crate::path::{self, Curve};
use crate::slug;
use crate::style::color::Rgba;
use crate::style::{
    Collapse, DecorationStyle, Family, Feature, Font, FontStyle, LineHeight, OverflowWrap,
    TextAlign, TextTransform, VARIANTS, Variation, WordBreak,
};
use crate::template::Error;

/// A font face as shaping chose it: a font file, the face's index in it, its variation
/// coordinates (normalized, 2.14 fixed point), and whether it is slanted for an italic it does
/// not have.
#[derive(Clone, Debug, PartialEq)]
pub struct Face {
    pub data: Blob<u8>,
    pub index: u32,
    pub coords: Vec<i16>,
    pub slanted: bool,
}

impl Face {
    /// The face a run of glyphs in `style` is shaped in.
    fn of(run: &parley::Run<'_, u32>, style: FontStyle) -> Face {
        let font = run.font();
        Face {
            data: font.data.clone(),
            index: font.index,
            coords: (run.normalized_coords().iter())
                .map(|c| c.to_bits())
                .collect(),
            slanted: run.synthesis().skew().is_some() && style.slants(),
        }
    }
}

/// A paragraph as written: the font of its box, which each line box starts from (CSS's strut)
/// and which aligns its lines, its pieces of text in order, and how many lines it shows, if it
/// is clamped.
pub struct Content<'a> {
    pub font: &'a Font,
    pub pieces: Vec<Piece<'a>>,
    pub clamp: Option<usize>,
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
/// that hangs at the end of the line, and the range of its glyphs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Run {
    pub node: u32,
    pub line: u32,
    pub x: [f32; 2],
    pub glyphs: [u32; 2],
}

/// A line of a text decoration over run `run`, or a piece of one between the gaps it leaves for
/// glyphs: left, top, right and bottom, px from the text box's top left. It goes over the glyphs
/// if it is a line-through.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stripe {
    pub rect: [f32; 4],
    /// Its line's left and right, which its dots, dashes or wave are spaced along; its style,
    /// solid but for those, as a double decoration is two solid lines; and how thick it is.
    pub along: [f32; 2],
    pub style: DecorationStyle,
    pub thickness: f32,
    pub color: Rgba,
    pub run: u32,
    pub through: bool,
}

/// A wavy decoration `thickness` thick as Chrome draws it: a cubic curve each wave, two steps
/// long, whose control points are both a step along, the control distance either side of its axis,
/// which takes it 0.29 of that distance off the axis. Both are whole px and a half.
pub fn wave(thickness: f32) -> [f32; 2] {
    [2.0, 3.0].map(|k| 0.5 + (k * thickness + 0.5).round())
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
    /// Its text decorations, which layout gives it.
    pub decorations: Vec<Stripe>,
}

/// What shaping a piece and breaking it into lines depend on.
#[derive(Clone, PartialEq, Eq, Hash)]
struct Shaping {
    family: Rc<[Family]>,
    /// f32 bits.
    size: u32,
    weight: u32,
    stretch: u32,
    /// Normal, italic or oblique, and the angle's f32 bits.
    style: [u32; 2],
    spacing: u32,
    word_spacing: u32,
    collapse: Collapse,
    wrap: bool,
    word_break: WordBreak,
    /// `word-break: break-word` is `anywhere` here.
    overflow_wrap: OverflowWrap,
    transform: TextTransform,
    variants: [Rc<[Feature]>; VARIANTS],
    features: Rc<[Feature]>,
    variations: Rc<[Variation]>,
}

impl Shaping {
    /// How `font` shapes and breaks text; for its min-content width when `min`, which
    /// `overflow-wrap: break-word` breaks no words for (CSS Text 3, 5.5).
    fn of(font: &Font, min: bool) -> Shaping {
        Shaping {
            family: font.family.clone(),
            size: (font.size as f32).to_bits(),
            weight: (font.weight as f32).to_bits(),
            stretch: (font.stretch as f32).to_bits(),
            style: match font.style {
                FontStyle::Normal => [0, 0],
                FontStyle::Italic => [1, 0],
                FontStyle::Oblique(angle) => [2, (angle as f32).to_bits()],
            },
            spacing: (font.spacing as f32).to_bits(),
            word_spacing: (font.word_spacing as f32).to_bits(),
            collapse: font.collapse,
            wrap: font.wrap,
            word_break: font.word_break,
            overflow_wrap: match (font.word_break, font.overflow_wrap) {
                (WordBreak::BreakWord, _) => OverflowWrap::Anywhere,
                (_, OverflowWrap::BreakWord) if min => OverflowWrap::Normal,
                (_, wrap) => wrap,
            },
            transform: font.transform,
            variants: font.variants.clone(),
            features: font.features.clone(),
            variations: font.variations.clone(),
        }
    }
}

/// An ellipsis shaped in a font: its glyphs, each a face, an id and an x from its origin, and its
/// advance.
#[derive(Clone)]
struct Ellipsis {
    glyphs: Vec<(u32, u32, f32)>,
    advance: f32,
}

/// A paragraph shaped: its glyph runs' brushes are the pieces they are in.
struct Paragraph {
    layout: parley::Layout<u32>,
    string: Rc<str>,
    /// Nothing is left once white space collapses: no line boxes, as in CSS (parley still makes
    /// one line).
    empty: bool,
    /// The width its lines were last broken at, their indent, and whether they balance.
    broken: Option<(Option<f32>, f32, bool)>,
    used: bool,
}

pub struct Text {
    fonts: FontContext,
    layouts: LayoutContext<u32>,
    /// By each piece's text and shaping.
    paragraphs: HashMap<Vec<(Box<str>, Shaping)>, Paragraph>,
    /// The ascent, descent and line gap of each font's first face, px.
    metrics: HashMap<Shaping, [f32; 3]>,
    ellipses: HashMap<Shaping, Ellipsis>,
    faces: Vec<Face>,
    /// Glyph outlines by face and glyph id, in font units y up, and their units per em.
    outlines: HashMap<(u32, u32), (f32, Vec<Curve>)>,
    /// The families of font resources, by their resources' names.
    named: Vec<Rc<str>>,
    /// Where the built-in fonts come from, and the family of each one given so far.
    built_in: Fonts,
    given: [Option<FamilyId>; BUILT_IN.len()],
}

/// A built-in font: a file of Google Fonts at a fixed version, as fonts.google.com downloads it.
pub struct BuiltIn {
    pub name: &'static str,
    /// The name the file is kept as, in the cache and in test pages.
    pub file: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    /// Bytes.
    pub size: u64,
}

/// The built-in fonts, which the host gives by their entries here.
pub const BUILT_IN: [BuiltIn; 8] = [
    BuiltIn {
        name: "Noto Sans",
        file: "NotoSans.ttf",
        url: "https://fonts.gstatic.com/s/notosans/v42/o-0IIpQlx3QUlC5A4PNb4j5Ba_2c7A.ttf",
        sha256: "7b8cac46a1c86d2533a616b1fcf4e1926b8e39bda69034508b0df96791f56d97",
        size: 2_044_548,
    },
    BuiltIn {
        name: "Noto Sans Italic",
        file: "NotoSansItalic.ttf",
        url: "https://fonts.gstatic.com/s/notosans/v42/o-0OIpQlx3QUlC5A4PNr4DRFSfiM7HBj.ttf",
        sha256: "4bf7b366af79c434984d67eae3967e9cd7a2f51c196101c43f21a7e21e608844",
        size: 2_300_468,
    },
    BuiltIn {
        name: "Noto Serif",
        file: "NotoSerif.ttf",
        url: "https://fonts.gstatic.com/s/notoserif/v33/ga6Iaw1J5X9T9RW6j9bNTFAcaRi_bMQ.ttf",
        sha256: "26485cbd00507110de7a77cd63d0ec99d356f38e5d9c0c5d07ecea1c7827ea24",
        size: 1_868_872,
    },
    BuiltIn {
        name: "Noto Serif Italic",
        file: "NotoSerifItalic.ttf",
        url: "https://fonts.gstatic.com/s/notoserif/v33/ga6Kaw1J5X9T9RW6j9bNfFIWbTq6fMRRMw.ttf",
        sha256: "061daf34bfdd0e0635ce3d4f701398772ce2944ee0b24f584e0273366bbea468",
        size: 2_429_468,
    },
    BuiltIn {
        name: "Noto Sans Mono",
        file: "NotoSansMono.ttf",
        url: "https://fonts.gstatic.com/s/notosansmono/v37/BngRUXNETWXI6LwhGYvaxZikqYCByxyKeuDp.ttf",
        sha256: "47d856141683450ee297592b27be447eb5141c68516e5e0e748c66b6e0a54afe",
        size: 1_707_908,
    },
    BuiltIn {
        name: "Noto Sans SC",
        file: "NotoSansSC.ttf",
        url: "https://fonts.gstatic.com/s/notosanssc/v41/k3kXo84MPvpLmixcA63oeALhKYiJ-Q7m8w.ttf",
        sha256: "6cbd3ac22ef454e4e9bcfb67369ea94297002a218e9abebc90852534afee096a",
        size: 17_711_448,
    },
    BuiltIn {
        name: "Noto Sans KR",
        file: "NotoSansKR.ttf",
        url: "https://fonts.gstatic.com/s/notosanskr/v40/PbykFmXiEBPT4ITbgNA5Cgm21nTs4JMMuA.ttf",
        sha256: "16efec3833f189345a7185d85790cd0db4d9729ea2d58c57fccc06ba5d1f3017",
        size: 10_359_708,
    },
    BuiltIn {
        name: "Noto Color Emoji",
        file: "NotoColorEmoji.ttf",
        url: "https://fonts.gstatic.com/s/notocoloremoji/v50/Yq6P-KqIXTD0t4D9z1ESnKM3-HpFab5s79iz64w.ttf",
        sha256: "d8c8a5e2eaecf93bf46be895640139d344ace174e2cdb6b0d988f7e83fd97844",
        size: 25_347_176,
    },
];

/// The generic families, and their files in `BUILT_IN`: upright,
/// and italic if they have one.
const GENERIC: [(GenericFamily, usize, Option<usize>); 3] = [
    (GenericFamily::SansSerif, 0, Some(1)),
    (GenericFamily::Serif, 2, Some(3)),
    (GenericFamily::Monospace, 4, None),
];

/// Where the built-in fonts come from: the host's file of each, by its entry in `BUILT_IN`.
pub type Fonts = Rc<dyn Fn(&BuiltIn) -> Result<Blob<u8>, Error>>;

impl Text {
    /// Text in the built-in families, whose files `fonts` gives when text first needs them.
    pub fn new(fonts: Fonts) -> Text {
        let collection = Collection::new(CollectionOptions {
            shared: false,
            system_fonts: false,
        });
        Text {
            fonts: FontContext {
                collection,
                source_cache: SourceCache::new(Default::default()),
            },
            layouts: LayoutContext::new(),
            paragraphs: HashMap::new(),
            metrics: HashMap::new(),
            ellipses: HashMap::new(),
            faces: Vec::new(),
            outlines: HashMap::new(),
            named: Vec::new(),
            built_in: fonts,
            given: [None; BUILT_IN.len()],
        }
    }

    /// The family of built-in font `i`, which the host gives the first time; an upright one is
    /// its generic family's from then on.
    fn built_in(&mut self, i: usize) -> Result<FamilyId, Error> {
        if let Some(family) = self.given[i] {
            return Ok(family);
        }
        let font = &BUILT_IN[i];
        let data = (self.built_in)(font).map_err(|e| format!("the font {}: {e}", font.name))?;
        let families = self.fonts.collection.register_fonts(data, None);
        let [(family, _)] = families[..] else {
            return Err(format!("the font {}: not one family", font.name));
        };
        self.given[i] = Some(family);
        if let Some((generic, ..)) = GENERIC.iter().find(|g| g.1 == i) {
            (self.fonts.collection).set_generic_families(*generic, [family].into_iter());
        }
        Ok(family)
    }

    /// Has the host give the built-in fonts that text in `font` may be shaped in: those of the
    /// generic families it names, and sans-serif, which every list ends in; their italics if it
    /// is not upright.
    fn give(&mut self, font: &Font) -> Result<(), Error> {
        let slanted = font.style != FontStyle::Normal;
        for (generic, upright, italic) in GENERIC {
            let named = (font.family.iter()).any(|f| match f {
                Family::SansSerif => generic == GenericFamily::SansSerif,
                Family::Serif => generic == GenericFamily::Serif,
                Family::Monospace => generic == GenericFamily::Monospace,
                Family::Named(_) => false,
            });
            if !(named || generic == GenericFamily::SansSerif) {
                continue;
            }
            self.built_in(upright)?;
            if let Some(italic) = italic.filter(|_| slanted) {
                self.built_in(italic)?;
            }
        }
        Ok(())
    }

    /// The font resource `name`, a family by that name, of the faces in `data`.
    pub fn add_font(&mut self, name: &str, data: Blob<u8>) -> Result<(), Error> {
        let named = FontInfoOverride {
            family_name: Some(name),
            ..Default::default()
        };
        if (self.fonts.collection.register_fonts(data, Some(named))).is_empty() {
            return Err("not a font file: TrueType, OpenType or a collection of them".into());
        }
        self.named.push(name.into());
        Ok(())
    }

    /// The faces glyphs name.
    pub fn faces(&self) -> &[Face] {
        &self.faces
    }

    /// The box `content` takes with its lines broken to fit `width` px (`None`: only at forced
    /// breaks). Widths round up to 1/64 px, as Chrome's layout units do, so the text fits again
    /// in the width it measures.
    ///
    /// Text that breaks takes all of the width, unless a word is wider: a box sized to its
    /// content is as wide as it may be once the content is wider (CSS's fit-content).
    pub fn measure(&mut self, content: &Content, width: Option<f32>) -> Result<[f32; 2], Error> {
        self.measured(content, width, false)
    }

    /// The box `content` takes at its min-content width: its lines broken at every soft wrap
    /// opportunity, and in words where `overflow-wrap: anywhere` breaks them.
    pub fn min_content(&mut self, content: &Content) -> Result<[f32; 2], Error> {
        self.measured(content, Some(0.0), true)
    }

    fn measured(
        &mut self,
        content: &Content,
        width: Option<f32>,
        min: bool,
    ) -> Result<[f32; 2], Error> {
        let strut = self.strut(content.font)?;
        let p = self.paragraph(content, width, min)?;
        if p.empty {
            return Ok([0.0, 0.0]);
        }
        let boxes = stack(&p.layout, content, strut);
        let shown = &boxes[..content.clamp.unwrap_or(usize::MAX).min(boxes.len())];
        let height = shown.last().map_or(0.0, |[top, h]| top + h);
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
        let p = self.paragraph(content, Some(width), false)?;
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
            TextAlign::Justify => Alignment::Justify,
        };
        let strut = self.strut(content.font)?;
        let p = self.paragraph(content, Some(width), false)?;
        let string = p.string.clone();
        if p.empty {
            return Ok(Lines {
                glyphs: Vec::new(),
                runs: Vec::new(),
                boxes: Vec::new(),
                baselines: Vec::new(),
                string,
                text: Vec::new(),
                decorations: Vec::new(),
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
                let piece = &content.pieces[run.style().brush as usize];
                let (node, face) = (piece.node, Face::of(run.run(), piece.font.style));
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
                    glyphs: [0, 0],
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
        for (run, (face, node, list)) in runs.iter_mut().zip(shaped) {
            run.glyphs = [glyphs.len() as u32, (glyphs.len() + list.len()) as u32];
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
            decorations: Vec::new(),
        })
    }

    /// The left and right, px from its origin, of where glyph `id` of `faces()[face]` at `size`
    /// px has ink between `band` px below its baseline, if it has any there.
    pub fn ink(
        &mut self,
        face: u32,
        id: u32,
        size: f32,
        band: [f32; 2],
    ) -> Result<Option<[f32; 2]>, Error> {
        let (em, curves) = match self.outlines.entry((face, id)) {
            Entry::Occupied(o) => o.into_mut(),
            Entry::Vacant(v) => {
                let (em, contours) = slug::contours(&self.faces[face as usize], id)?;
                v.insert((em, contours.concat()))
            }
        };
        let k = size / *em;
        // Font units are y up.
        let x = path::across(curves, -band[1] / k, -band[0] / k);
        Ok(x.map(|x| x.map(|x| x * k)))
    }

    /// The ascent, descent and line gap, px, of the first face of `font`, at its size and
    /// weight, as runs in it have them.
    pub fn metrics(&mut self, font: &Font) -> Result<[f32; 3], Error> {
        let key = Shaping::of(font, false);
        if let Some(m) = self.metrics.get(&key) {
            return Ok(*m);
        }
        let layout = self.alone("0", font)?;
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

    /// `lines`, broken to fit `width`, ending in an ellipsis in `font`, the text box's, as Chrome
    /// ends them: each line that overflows if `overflow`, and line `clamp` if more follow it. The
    /// glyphs that end past where the ellipsis starts go, and the decorations under them, but the
    /// runs keep their extents, and so do the Spans they are in; a clamped line with room keeps
    /// them all. The ellipsis follows the last glyph left, as a run of `node`, the Text, with no
    /// decorations.
    pub fn truncate(
        &mut self,
        lines: &mut Lines,
        font: &Font,
        node: u32,
        width: f32,
        overflow: bool,
        clamp: Option<usize>,
    ) -> Result<(), Error> {
        let Ellipsis {
            glyphs: ellipsis,
            advance,
        } = self.ellipsis(font)?;
        // Each glyph's right edge: where the next one in its run starts, or the run ends.
        let right = |lines: &Lines, run: &Run, g: usize| match g + 1 < run.glyphs[1] as usize {
            true => lines.glyphs[g + 1].x,
            false => run.x[1],
        };
        let mut cuts: Vec<Option<f32>> = vec![None; lines.boxes.len()];
        for (l, cut) in cuts.iter_mut().enumerate() {
            let runs: Vec<&Run> = (lines.runs.iter()).filter(|r| r.line == l as u32).collect();
            let over = overflow && runs.iter().any(|r| r.x[1] > width);
            let clamped = clamp.is_some_and(|n| l + 1 == n && lines.boxes.len() > n);
            if !over && !clamped {
                continue;
            }
            let mut at = runs.first().map_or(0.0, |r| r.x[0]);
            let end = runs.iter().map(|r| r.x[1]).fold(at, f32::max);
            if !over && end - at + advance <= width {
                *cut = Some(end);
                continue;
            }
            'fit: for run in runs {
                for g in run.glyphs[0] as usize..run.glyphs[1] as usize {
                    let end = right(lines, run, g);
                    if end > width - advance {
                        break 'fit;
                    }
                    at = end;
                }
            }
            *cut = Some(at);
        }
        let mut glyphs = Vec::new();
        for k in 0..lines.runs.len() {
            let run = lines.runs[k];
            let start = glyphs.len() as u32;
            for g in run.glyphs[0] as usize..run.glyphs[1] as usize {
                if cuts[run.line as usize].is_none_or(|c| right(lines, &run, g) <= c) {
                    glyphs.push(lines.glyphs[g]);
                }
            }
            lines.runs[k].glyphs = [start, glyphs.len() as u32];
        }
        lines.decorations.retain_mut(|d| {
            let line = lines.runs[d.run as usize].line as usize;
            let Some(c) = cuts[line] else {
                return true;
            };
            d.rect[2] = d.rect[2].min(c);
            d.rect[0] < d.rect[2]
        });
        for (l, cut) in cuts.into_iter().enumerate() {
            let Some(c) = cut else {
                continue;
            };
            let start = glyphs.len() as u32;
            glyphs.extend(ellipsis.iter().map(|&(face, id, x)| Glyph {
                face,
                id,
                x: c + x,
                y: lines.baselines[l],
                node,
            }));
            lines.runs.push(Run {
                node,
                line: l as u32,
                x: [c, c + advance],
                glyphs: [start, glyphs.len() as u32],
            });
        }
        lines.glyphs = glyphs;
        Ok(())
    }

    /// An ellipsis in `font`.
    fn ellipsis(&mut self, font: &Font) -> Result<Ellipsis, Error> {
        let key = Shaping::of(font, false);
        if let Some(e) = self.ellipses.get(&key) {
            return Ok(e.clone());
        }
        let layout = self.alone("…", font)?;
        let mut shaped = Vec::new();
        let mut x = 0.0;
        for line in layout.lines() {
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(run) = item else {
                    continue;
                };
                let face = Face::of(run.run(), font.style);
                for g in run.glyphs() {
                    shaped.push((face.clone(), g.id, x + g.x));
                    x += g.advance;
                }
            }
        }
        let glyphs = (shaped.into_iter())
            .map(|(face, id, x)| (self.face(face), id, x))
            .collect();
        let e = Ellipsis { glyphs, advance: x };
        self.ellipses.insert(key, e.clone());
        Ok(e)
    }

    /// `text` shaped in `font` alone, on one line.
    fn alone(&mut self, text: &str, font: &Font) -> Result<parley::Layout<u32>, Error> {
        self.give(font)?;
        let families = families(&font.family, &self.named)?;
        let mut builder = (self.layouts).ranged_builder(&mut self.fonts, text, 1.0, false);
        let family = FontFamily::List(Cow::Borrowed(&families));
        builder.push_default(StyleProperty::FontFamily(family));
        builder.push_default(StyleProperty::FontSize(font.size as f32));
        let weight = FontWeight::new(font.weight as f32);
        builder.push_default(StyleProperty::FontWeight(weight));
        let width = FontWidth::from_percentage(font.stretch as f32);
        builder.push_default(StyleProperty::FontWidth(width));
        builder.push_default(StyleProperty::FontStyle(style(font)));
        builder.push_default(StyleProperty::FontVariations(variations(font)));
        let mut layout = builder.build(text);
        layout.break_all_lines(None);
        Ok(layout)
    }

    /// Forgets the paragraphs not used since the last sweep.
    pub fn sweep(&mut self) {
        self.paragraphs.retain(|_, p| std::mem::take(&mut p.used));
    }

    fn face(&mut self, face: Face) -> u32 {
        let i = match self.faces.iter().position(|f| {
            f.data.id() == face.data.id()
                && (f.index, &f.coords, f.slanted) == (face.index, &face.coords, face.slanted)
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

    /// The paragraph of `content`, shaped if it is new, its lines broken at `width`, and as at
    /// its min-content width when `min`.
    fn paragraph(
        &mut self,
        content: &Content,
        width: Option<f32>,
        min: bool,
    ) -> Result<&mut Paragraph, Error> {
        let key: Vec<(Box<str>, Shaping)> = (content.pieces.iter())
            .map(|p| (p.text.as_ref().into(), Shaping::of(p.font, min)))
            .collect();
        if !self.paragraphs.contains_key(&key) {
            for piece in &content.pieces {
                self.give(piece.font)?;
            }
            let pieces = (content.pieces.iter())
                .map(|p| (p.text.as_ref(), p.font.collapse, p.font.transform));
            let (string, ranges) = prepare(pieces)?;
            let families = (content.pieces.iter())
                .map(|p| families(&p.font.family, &self.named))
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
                let width = FontWidth::from_percentage(font.stretch as f32);
                builder.push(StyleProperty::FontWidth(width), range.clone());
                builder.push(StyleProperty::FontStyle(style(font)), range.clone());
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
                let breaks = match font.word_break {
                    WordBreak::Normal | WordBreak::BreakWord => parley::WordBreak::Normal,
                    WordBreak::BreakAll => parley::WordBreak::BreakAll,
                    WordBreak::KeepAll => parley::WordBreak::KeepAll,
                };
                builder.push(StyleProperty::WordBreak(breaks), range.clone());
                let wrap = match key[k].1.overflow_wrap {
                    OverflowWrap::Normal => parley::OverflowWrap::Normal,
                    OverflowWrap::Anywhere => parley::OverflowWrap::Anywhere,
                    OverflowWrap::BreakWord => parley::OverflowWrap::BreakWord,
                };
                builder.push(StyleProperty::OverflowWrap(wrap), range.clone());
                let features = font.variants.iter().flat_map(|v| v.iter());
                let mut features = features.chain(font.features.iter()).peekable();
                if features.peek().is_some() {
                    let features = (features)
                        .map(|f| FontFeature::new(Tag::new(&f.tag), f.value))
                        .collect();
                    let features = FontFeatures::List(Cow::Owned(features));
                    builder.push(StyleProperty::FontFeatures(features), range.clone());
                }
                if !font.variations.is_empty() {
                    let axes = StyleProperty::FontVariations(variations(font));
                    builder.push(axes, range.clone());
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
        let balanced = content.font.balance && !min;
        let indent = (content.font.indent.as_ref())
            .map_or(0.0, |indent| indent.resolve(width.unwrap_or(0.0)));
        if p.broken != Some((width, indent, balanced)) {
            p.layout.set_text_indent(indent, IndentOptions::default());
            p.layout.break_all_lines(width);
            if let Some(width) = width.filter(|_| balanced && !p.empty) {
                balance(&mut p.layout, width);
            }
            p.broken = Some((width, indent, balanced));
        }
        Ok(p)
    }
}

/// `text-wrap: balance` as Chrome has it, on lines broken to fit `width`: those up to the first
/// forced break, if there are 2 to 6 of them, broken as narrow as they go (to 1/64 px) without
/// one more, and still aligned in `width`.
fn balance(layout: &mut parley::Layout<u32>, width: f32) {
    let n = leading(layout);
    if !(2..=6).contains(&n) {
        return;
    }
    // The narrowest width found that keeps them to `n`, and in 1/64 px what bounds it.
    let mut narrowest = width;
    let (mut lo, mut hi) = (0, (width * 64.0).floor() as u32);
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        balanced(layout, mid as f32 / 64.0, width);
        match leading(layout) <= n {
            true => (hi, narrowest) = (mid, mid as f32 / 64.0),
            false => lo = mid,
        }
    }
    balanced(layout, narrowest, width);
}

/// How many lines `layout` has up to its first forced break.
fn leading(layout: &parley::Layout<u32>) -> usize {
    let mut n = 0;
    for line in layout.lines() {
        n += 1;
        if line.break_reason() == BreakReason::Explicit {
            break;
        }
    }
    n
}

/// `layout`'s lines broken to fit `width`, but those up to its first forced break to fit
/// `narrow`, aligned in `width` all the same.
fn balanced(layout: &mut parley::Layout<u32>, narrow: f32, width: f32) {
    let mut lines = layout.break_lines();
    lines.state_mut().set_layout_max_advance(width);
    lines.state_mut().set_line_max_advance(narrow);
    let mut first = true;
    while let Some(line) = lines.break_next() {
        if first {
            lines.set_prior_line_width(width);
        }
        if let YieldData::LineBreak(data) = line
            && data.reason == BreakReason::Explicit
        {
            first = false;
            lines.state_mut().set_line_max_advance(width);
        }
    }
    lines.finish();
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

/// The style of `font`, as parley takes it.
fn style(font: &Font) -> parley::FontStyle {
    match font.style {
        FontStyle::Normal => parley::FontStyle::Normal,
        FontStyle::Italic => parley::FontStyle::Italic,
        // Chrome takes an italic face for 14deg and more, where fontique would slant the upright
        // one; a family without one is slanted either way.
        FontStyle::Oblique(angle) if angle >= 14.0 => parley::FontStyle::Italic,
        FontStyle::Oblique(angle) => parley::FontStyle::Oblique(Some(angle as f32)),
    }
}

/// The axes `font` sets, as parley takes them.
fn variations(font: &Font) -> FontVariations<'static> {
    let axes = (font.variations.iter()).map(|v| FontVariation::new(Tag::new(&v.tag), v.value));
    FontVariations::List(Cow::Owned(axes.collect()))
}

/// `list` as parley takes it: built-in families, and the font resources of `named`, as CSS
/// matches family names regardless of ASCII case; then sans-serif, for the characters none of
/// them has.
fn families(list: &[Family], named: &[Rc<str>]) -> Result<Vec<FontFamilyName<'static>>, Error> {
    let mut out = (list.iter())
        .map(|f| match f {
            Family::SansSerif => Ok(FontFamilyName::Generic(GenericFamily::SansSerif)),
            Family::Serif => Ok(FontFamilyName::Generic(GenericFamily::Serif)),
            Family::Monospace => Ok(FontFamilyName::Generic(GenericFamily::Monospace)),
            Family::Named(name) => match named.iter().find(|n| n.eq_ignore_ascii_case(name)) {
                Some(n) => Ok(FontFamilyName::Named(Cow::Owned(n.to_string()))),
                None => Err(format!(
                    "font family {name}: the scene declares no font resource {name} in \
                     controls.resources"
                )),
            },
        })
        .collect::<Result<Vec<_>, Error>>()?;
    out.push(FontFamilyName::Generic(GenericFamily::SansSerif));
    Ok(out)
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
