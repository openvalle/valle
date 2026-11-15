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

mod bidi;
pub(crate) mod emphasis;
mod vertical;

use std::borrow::Cow;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::ops::Range;
use std::rc::Rc;
use unicode_segmentation::UnicodeSegmentation;

pub use parley::fontique::Blob;
use parley::fontique::{
    Collection, CollectionOptions, FallbackKey, FamilyId, FontInfoOverride, GenericFamily, Script,
    SourceCache,
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
    TextAlign, TextTransform, VARIANTS, Variation, WordBreak, WrapStyle, WritingMode,
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

impl Content<'_> {
    /// Vertical inline padding percentages use the physical containing width, not its height.
    pub(crate) fn with_inline_basis(&self, width: f32) -> Option<Content<'_>> {
        if self.font.writing == WritingMode::Horizontal
            || !self.pieces.iter().any(|p| {
                p.edge
                    .as_ref()
                    .is_some_and(|e| e.width.any(&|v| v[1] != 0.0))
            })
        {
            return None;
        }
        Some(Content {
            font: self.font,
            clamp: self.clamp,
            pieces: self
                .pieces
                .iter()
                .map(|p| {
                    let mut p = p.clone();
                    if let Some(edge) = &mut p.edge {
                        edge.width =
                            crate::style::math::Math::Sum([edge.width.resolve(width) as f64, 0.0]);
                    }
                    p
                })
                .collect(),
        })
    }
}

/// Text in the font of the node it is in, which its glyphs name.
#[derive(Clone)]
pub struct Piece<'a> {
    pub text: Cow<'a, str>,
    pub node: u32,
    pub font: &'a Font,
    /// An inline box edge occupies space but is not part of the authored text.
    pub edge: Option<Edge>,
}

#[derive(Clone)]
pub struct Edge {
    pub width: crate::style::math::Calc,
    pub start: bool,
}

/// A glyph of `Text::faces()[face]` in the text of `node`, in px from the text box's top left,
/// `y` on its baseline, and where its characters start in `Lines::string` (an ellipsis's at its
/// end).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glyph {
    pub face: u32,
    pub id: u32,
    pub x: f32,
    pub y: f32,
    pub node: u32,
    pub cluster: u32,
    /// Counter-rotate a vertical upright glyph before the paragraph turns clockwise.
    pub upright: bool,
    /// Logical inline cell before vertical-origin offsets, for SVG character positioning.
    pub vertical_cell: Option<[f32; 2]>,
    /// Horizontal glyph compression for combined upright text; advances are already scaled.
    pub scale_x: f32,
}

/// A run of glyphs of one node on a line, with its left and right edges, less the white space
/// that hangs at the end of the line, and the range of its glyphs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Run {
    /// An inserted inline edge and its authored byte position.
    pub edge: Option<(bool, u32)>,
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

/// Byte ranges where `text-decoration-skip-ink: auto` keeps CJK underlines continuous.
/// Unicode Script_Extensions covers shared kana marks and punctuation too. Other Common or
/// Inherited characters follow the preceding script; leading and opening punctuation follow
/// the next script. A Span boundary does not start a new script context.
pub(crate) fn cjk_ranges(text: &str) -> Vec<Range<usize>> {
    use icu_properties::CodePointMapData;
    use icu_properties::props::{GeneralCategory, Script};
    use icu_properties::script::ScriptWithExtensions;

    let scripts = ScriptWithExtensions::new();
    let categories = CodePointMapData::<GeneralCategory>::new();
    let mut ranges = Vec::new();
    let (mut start, mut previous) = (0, None);
    let mut opening = None;
    for (i, c) in text.char_indices() {
        if matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}') {
            if previous == Some(true) {
                ranges.push(start..i);
            }
            (start, previous) = (i + c.len_utf8(), None);
            opening = None;
            continue;
        }
        let cjk = scripts.get_script_extensions_val(c).iter().any(|s| {
            matches!(
                s,
                Script::Han
                    | Script::Hiragana
                    | Script::Katakana
                    | Script::Bopomofo
                    | Script::Hangul
            )
        });
        if !cjk
            && matches!(
                scripts.get_script_val(c),
                Script::Common | Script::Inherited
            )
        {
            if matches!(
                categories.get(c),
                GeneralCategory::OpenPunctuation | GeneralCategory::InitialPunctuation
            ) {
                opening.get_or_insert(i);
            }
            continue;
        }
        let boundary = opening.take().unwrap_or(i);
        if previous.is_some_and(|p| p != cjk) {
            if previous == Some(true) {
                ranges.push(start..boundary);
            }
            start = boundary;
        }
        previous = Some(cjk);
    }
    if previous == Some(true) {
        ranges.push(start..text.len());
    }
    ranges
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
    pub marks: Vec<emphasis::Mark>,
    pub fragments: Vec<crate::layout::inline::Fragment>,
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
    line_break: u8,
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
            line_break: font.line_break as u8,
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

type ParagraphKey = (
    bool,
    crate::style::bidi::Bidi,
    Vec<(
        Box<str>,
        Shaping,
        Rc<[crate::style::bidi::Scope]>,
        Option<(u32, bool)>,
    )>,
);

/// A paragraph shaped: its glyph runs' brushes are the pieces they are in.
struct Paragraph {
    layout: parley::Layout<u32>,
    string: Rc<str>,
    source: Rc<str>,
    map: bidi::Map,
    /// Nothing is left once white space collapses: no line boxes, as in CSS (parley still makes
    /// one line).
    empty: bool,
    /// The width its lines were last broken at, their indent, and whether they balance.
    broken: Option<(Option<f32>, f32, WrapStyle)>,
    used: bool,
}

/// A face in a file of a font resource: its family, the file's path, and what CSS's
/// `@font-face` would say of it: its weights and widths (%), from one to another for a variable
/// font, and whether it is italic.
#[derive(Clone, Debug, PartialEq)]
pub struct FontFile {
    pub family: Rc<str>,
    pub path: String,
    pub weight: [f32; 2],
    pub width: [f32; 2],
    pub italic: bool,
}

pub struct Text {
    fonts: FontContext,
    layouts: LayoutContext<u32>,
    /// By each piece's text and shaping.
    paragraphs: HashMap<ParagraphKey, Paragraph>,
    /// The ascent, descent and line gap of each font's first face, px.
    metrics: HashMap<Shaping, [f32; 3]>,
    x_heights: HashMap<Shaping, f32>,
    ellipses: HashMap<Shaping, Ellipsis>,
    faces: Vec<Face>,
    /// Glyph outlines by face and glyph id, in font units y up, and their units per em.
    outlines: HashMap<(u32, u32), (f32, Vec<Curve>)>,
    /// The families of font resources, by their resources' names, and their files.
    named: Vec<Rc<str>>,
    files: Vec<FontFile>,
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
const GENERIC: [(GenericFamily, usize, Option<usize>); 4] = [
    (GenericFamily::SansSerif, 0, Some(1)),
    (GenericFamily::Serif, 2, Some(3)),
    (GenericFamily::Monospace, 4, None),
    (GenericFamily::Emoji, 7, None),
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
            x_heights: HashMap::new(),
            ellipses: HashMap::new(),
            faces: Vec::new(),
            outlines: HashMap::new(),
            named: Vec::new(),
            files: Vec::new(),
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
                Family::Emoji => generic == GenericFamily::Emoji,
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

    /// Has the host give the built-in fonts `text` falls back to for its scripts, and makes
    /// each the fallback of its scripts.
    fn give_fallbacks(&mut self, text: &str) -> Result<(), Error> {
        for &(font, scripts, has) in &FALLBACKS {
            if self.given[font].is_some() || !text.chars().any(has) {
                continue;
            }
            // The emoji font is a generic family's, which `built_in` sets.
            let family = self.built_in(font)?;
            for &script in scripts {
                let key = FallbackKey::new(Script::from_bytes(script), None);
                (self.fonts.collection).set_fallbacks(key, [family].into_iter());
            }
        }
        Ok(())
    }

    /// Adds the faces in `data`, the file at `path`, to the family of font resource `name`; no
    /// two files of a family have a face of the same weight, style and width.
    pub fn add_font(&mut self, name: &str, path: &str, data: Blob<u8>) -> Result<(), Error> {
        let named = FontInfoOverride {
            family_name: Some(name),
            ..Default::default()
        };
        let data = match sfnt(data.data())? {
            Some(file) => Blob::from(file),
            None => data,
        };
        let added = self.fonts.collection.register_fonts(data, Some(named));
        let Some((_, infos)) = added.first() else {
            return Err("not a font file: TrueType, OpenType, WOFF, WOFF2 or a collection".into());
        };
        let family: Rc<str> = match self.named.iter().find(|n| ***n == *name) {
            Some(family) => family.clone(),
            None => {
                self.named.push(name.into());
                name.into()
            }
        };
        let file = self.files.len();
        for info in infos {
            let range = |tag: &[u8; 4], value: f32| match (info.axes().iter())
                .find(|a| a.tag == skrifa::Tag::new(tag))
            {
                Some(axis) => [axis.min, axis.max],
                None => [value, value],
            };
            let face = FontFile {
                family: family.clone(),
                path: path.to_string(),
                weight: range(b"wght", info.weight().value()),
                width: range(b"wdth", info.width().ratio() * 100.0),
                italic: info.style() != parley::fontique::FontStyle::Normal,
            };
            let same = |f: &FontFile| {
                (f.family == face.family && f.weight[0] == face.weight[0])
                    && (f.width[0] == face.width[0] && f.italic == face.italic)
            };
            if let Some(other) = self.files[..file].iter().find(|f| same(f)) {
                return Err(format!(
                    "the same weight, style and width as {}",
                    other.path
                ));
            }
            self.files.push(face);
        }
        Ok(())
    }

    /// The files of font resources, a face each.
    pub fn font_files(&self) -> &[FontFile] {
        &self.files
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
        if content.font.writing != WritingMode::Horizontal {
            let mut font = content.font.clone();
            font.align = TextAlign::Start;
            font.align_last = None;
            let measured = Content {
                font: &font,
                pieces: content
                    .pieces
                    .iter()
                    .map(|p| Piece {
                        edge: p.edge.clone(),
                        text: p.text.clone(),
                        node: p.node,
                        font: p.font,
                    })
                    .collect(),
                clamp: content.clamp,
            };
            let lines = vertical::lines(self, &measured, width, min)?;
            let shown = content.clamp.unwrap_or(usize::MAX).min(lines.boxes.len());
            let height = lines
                .boxes
                .get(shown.saturating_sub(1))
                .map_or(0.0, |b| b[0] + b[1]);
            let wide = lines.runs.iter().map(|r| r.x[1]).fold(0.0, f32::max);
            return Ok([(wide * 64.0).ceil() / 64.0, height]);
        }
        let marks = emphasis::shapes(self, content)?;
        let strut = self.strut(content.font)?;
        let edges = self.edge_struts(content)?;
        let p = self.paragraph(content, width, min)?;
        if p.empty {
            return Ok([0.0, 0.0]);
        }
        let boxes = stack(&p.layout, content, strut, &edges, &marks, &p.string);
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
        if content.font.writing != WritingMode::Horizontal {
            return Ok(None);
        }
        let strut = self.strut(content.font)?;
        let edges = self.edge_struts(content)?;
        let p = self.paragraph(content, Some(width), false)?;
        if p.empty {
            return Ok(None);
        }
        Ok(p.layout
            .lines()
            .next()
            .map(|line| extent(&line, content, strut, &edges, &p.string)[0]))
    }

    /// `content` broken to fit `width` px and aligned in it.
    pub fn lines(&mut self, content: &Content, width: f32) -> Result<Lines, Error> {
        if content.font.writing != WritingMode::Horizontal {
            return vertical::lines(self, content, Some(width), false);
        }
        let alignment = |align| match align {
            TextAlign::Start => Alignment::Start,
            TextAlign::End => Alignment::End,
            TextAlign::Left => Alignment::Left,
            TextAlign::Right => Alignment::Right,
            TextAlign::Center => Alignment::Center,
            TextAlign::Justify => Alignment::Justify,
        };
        let marks = emphasis::shapes(self, content)?;
        let strut = self.strut(content.font)?;
        let edges = self.edge_struts(content)?;
        let p = self.paragraph(content, Some(width), false)?;
        let string = p.source.clone();
        if p.empty {
            return Ok(Lines {
                glyphs: Vec::new(),
                runs: Vec::new(),
                boxes: Vec::new(),
                baselines: Vec::new(),
                string,
                text: Vec::new(),
                decorations: Vec::new(),
                marks: Vec::new(),
                fragments: Vec::new(),
            });
        }
        p.layout.align(
            alignment(content.font.align),
            AlignmentOptions {
                last_line_alignment: content.font.align_last.map(alignment),
                ..AlignmentOptions::default()
            },
        );
        let boxes = stack(&p.layout, content, strut, &edges, &marks, &p.string);
        let text = p
            .layout
            .lines()
            .map(|line| {
                let r = line.text_range();
                p.map.at(r.start)..p.map.at(r.end)
            })
            .collect();
        let mut baselines = Vec::new();
        let mut runs = Vec::new();
        let mut shaped = Vec::new();
        for (i, (line, [top, _])) in p.layout.lines().zip(&boxes).enumerate() {
            let baseline = top + extent(&line, content, strut, &edges, &p.string)[0];
            baselines.push(baseline);
            let shift = bidi::shift(content.font, &line, &p.string);
            // Where each glyph's characters start: the clusters of a shaping run in visual
            // order, which its glyph runs on the line take one after another.
            let mut starts = Vec::new().into_iter();
            let mut from = None;
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(run) = item else {
                    continue;
                };
                let piece = &content.pieces[run.style().brush as usize];
                let (node, face) = (piece.node, Face::of(run.run(), piece.font.style));
                if from != Some(run.run().index()) {
                    from = Some(run.run().index());
                    starts = (run.run().visual_clusters())
                        .flat_map(|c| {
                            let at = c.text_range().start as u32;
                            c.glyphs().map(move |_| at)
                        })
                        .collect::<Vec<_>>()
                        .into_iter();
                }
                let mut x = run.offset() + shift;
                let mut glyphs = Vec::new();
                for g in run.glyphs() {
                    let at = starts.next().unwrap_or(run.run().text_range().end as u32);
                    if !p.map.inserted(at as usize) {
                        glyphs.push((g.id, x + g.x, baseline + g.y, p.map.at(at as usize) as u32));
                    }
                    x += g.advance;
                }
                runs.push(Run {
                    edge: piece
                        .edge
                        .as_ref()
                        .map(|e| (e.start, p.map.at(run.run().text_range().start) as u32)),
                    node,
                    line: i as u32,
                    x: [run.offset() + shift, x],
                    glyphs: [0, 0],
                });
                shaped.push((face, node, glyphs));
            }
            // White space that hangs past the end of the line is not in its runs' boxes.
            let mut hanging = line.metrics().hanging_advance;
            let rtl = if content.font.bidi == crate::style::bidi::Bidi::Plaintext {
                bidi::direction(&p.string, line.text_range().start)
            } else {
                content.font.rtl
            };
            let first = runs.partition_point(|r| r.line < i as u32);
            for k in 0..runs.len() - first {
                let index = if rtl { first + k } else { runs.len() - 1 - k };
                let run = &mut runs[index];
                let cut = hanging.min(run.x[1] - run.x[0]);
                if rtl {
                    run.x[0] += cut;
                } else {
                    run.x[1] -= cut;
                }
                hanging -= cut;
            }
        }
        let mut glyphs = Vec::new();
        for (run, (face, node, list)) in runs.iter_mut().zip(shaped) {
            run.glyphs = [glyphs.len() as u32, (glyphs.len() + list.len()) as u32];
            let face = self.face(face);
            glyphs.extend((list.into_iter()).map(|(id, x, y, cluster)| Glyph {
                face,
                id,
                x,
                y,
                node,
                cluster,
                upright: false,
                vertical_cell: None,
                scale_x: 1.0,
            }));
        }
        let mut lines = Lines {
            glyphs,
            runs,
            boxes,
            baselines,
            string,
            text,
            decorations: Vec::new(),
            marks: Vec::new(),
            fragments: Vec::new(),
        };
        emphasis::horizontal(&mut lines, content, &marks);
        Ok(lines)
    }

    /// The left and right, px from its origin, of where glyph `id` of `faces()[face]` at `size`
    /// px has ink between `band` px below its baseline, if it has any there.
    pub fn ink(
        &mut self,
        face: u32,
        id: u32,
        size: f32,
        band: [f32; 2],
        upright: bool,
        scale_x: f32,
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
        let turned;
        let curves = if upright {
            turned = curves
                .iter()
                .map(|c| c.map(|[x, y]| [-y, x * scale_x]))
                .collect::<Vec<_>>();
            &turned
        } else {
            curves
        };
        let x = path::across(curves, -band[1] / k, -band[0] / k);
        Ok(x.map(|x| x.map(|x| x * k)))
    }

    /// The font's x-height in pixels, used by SVG's middle baseline.
    pub(crate) fn x_height(&mut self, font: &Font) -> Result<f32, Error> {
        let key = Shaping::of(font, false);
        if let Some(height) = self.x_heights.get(&key) {
            return Ok(*height);
        }
        let layout = self.alone("x", font)?;
        let height = layout
            .lines()
            .find_map(|line| {
                line.items().find_map(|item| match item {
                    PositionedLayoutItem::GlyphRun(run) => run.run().font_metrics().x_height,
                    _ => None,
                })
            })
            .filter(|h| *h > 0.0)
            .unwrap_or(font.size as f32 * 0.5);
        self.x_heights.insert(key, height);
        Ok(height)
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
        lines
            .marks
            .retain(|m| cuts[m.line as usize].is_none_or(|c| m.inline[1] <= c));
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
                cluster: lines.string.len() as u32,
                upright: false,
                vertical_cell: None,
                scale_x: 1.0,
            }));
            lines.runs.push(Run {
                edge: None,
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
        self.give_fallbacks(text)?;
        let families = families(&font.family, &self.named)?;
        let mut builder = (self.layouts).ranged_builder(&mut self.fonts, text, 1.0, false);
        let family = FontFamily::List(Cow::Borrowed(&families));
        builder.push_default(StyleProperty::FontFamily(family));
        let first = emoji_first(&families);
        for range in presented(text) {
            let family = FontFamily::List(Cow::Borrowed(&first));
            builder.push(StyleProperty::FontFamily(family), range);
        }
        builder.push_default(StyleProperty::FontSize(font.size as f32));
        let weight = FontWeight::new(font.weight as f32);
        builder.push_default(StyleProperty::FontWeight(weight));
        let width = FontWidth::from_percentage(font.stretch as f32);
        builder.push_default(StyleProperty::FontWidth(width));
        builder.push_default(StyleProperty::FontStyle(style(font)));
        builder.push_default(StyleProperty::FontVariations(variations(font)));
        let mut layout = builder.build(text);
        layout.break_all_lines(None);
        covered(&layout, text)?;
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

    /// Empty inline edges retain their font's line height even without a visible glyph.
    fn edge_struts(&mut self, content: &Content) -> Result<HashMap<u32, [f32; 2]>, Error> {
        let mut out = HashMap::new();
        for p in &content.pieces {
            if p.edge.is_some() && !out.contains_key(&p.node) {
                out.insert(p.node, self.strut(p.font)?);
            }
        }
        Ok(out)
    }

    /// The paragraph shaped once and broken at `width`, or at its minimum content width.
    fn paragraph(
        &mut self,
        content: &Content,
        width: Option<f32>,
        min: bool,
    ) -> Result<&mut Paragraph, Error> {
        let key: ParagraphKey = (
            content.font.rtl,
            content.font.bidi,
            content
                .pieces
                .iter()
                .map(|p| {
                    (
                        p.text.as_ref().into(),
                        Shaping::of(p.font, min),
                        p.font.bidi_scopes.clone(),
                        p.edge.as_ref().map(|e| {
                            (
                                e.width.resolve(width.unwrap_or(0.0)).max(0.0).to_bits(),
                                e.start,
                            )
                        }),
                    )
                })
                .collect(),
        );
        if !self.paragraphs.contains_key(&key) {
            for piece in &content.pieces {
                self.give(piece.font)?;
            }
            let pieces = (content.pieces.iter())
                .map(|p| (p.text.as_ref(), p.font.collapse, p.font.transform));
            let (string, ranges) = prepare(pieces)?;
            let bidi::Input {
                source,
                text: string,
                ranges,
                map,
            } = bidi::Input::new(string, ranges, content);
            self.give_fallbacks(&string)?;
            let families = (content.pieces.iter())
                .map(|p| families(&p.font.family, &self.named))
                .collect::<Result<Vec<_>, _>>()?;
            let firsts: Vec<_> = families.iter().map(|f| emoji_first(f)).collect();
            let mut builder = (self.layouts).ranged_builder(&mut self.fonts, &string, 1.0, false);
            builder.set_base_direction(
                if content.font.bidi == crate::style::bidi::Bidi::Plaintext {
                    parley::BaseDirection::Auto
                } else if content.font.rtl {
                    parley::BaseDirection::Rtl
                } else {
                    parley::BaseDirection::Ltr
                },
            );
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
                builder.push(StyleProperty::LineBreak(font.line_break), range.clone());
                // CSS Text 3 forbids a break before small kana in normal as well as strict.
                // ICU's normal table still treats CJ as an ideograph. Tailor only those
                // characters, preserving normal's other punctuation rules and Span styles.
                if font.line_break == parley::LineBreak::Normal {
                    use icu_properties::{CodePointMapData, props::LineBreak};
                    let classes = CodePointMapData::<LineBreak>::new();
                    for (offset, c) in string[range.clone()].char_indices() {
                        if classes.get(c) == LineBreak::ConditionalJapaneseStarter {
                            let start = range.start + offset;
                            builder.push(
                                StyleProperty::LineBreak(parley::LineBreak::Strict),
                                start..start + c.len_utf8(),
                            );
                        }
                    }
                }
                let wrap = match key.2[k].1.overflow_wrap {
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
                for emoji in presented(&string[range.clone()]) {
                    let family = FontFamily::List(Cow::Borrowed(&firsts[k]));
                    let at = range.start + emoji.start..range.start + emoji.end;
                    builder.push(StyleProperty::FontFamily(family), at);
                }
                if let Some((advance, _)) = key.2[k].3 {
                    builder.push(StyleProperty::FontSize(0.0), range.clone());
                    builder.push(StyleProperty::WordSpacing(0.0), range.clone());
                    builder.push(StyleProperty::LetterSpacing(0.0), range.clone());
                    for (offset, c) in string[range.clone()].char_indices() {
                        if c == '\u{a0}' {
                            let at = range.start + offset;
                            builder.push(
                                StyleProperty::LetterSpacing(f32::from_bits(advance)),
                                at..at + c.len_utf8(),
                            );
                        }
                    }
                }
                builder.push(StyleProperty::Brush(k as u32), range);
            }
            let mut layout = builder.build(&string);
            layout.break_all_lines(None);
            covered(&layout, &string)?;
            let paragraph = Paragraph {
                layout,
                empty: string.is_empty(),
                source,
                map,
                string,
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
        let wrap_style = if min {
            WrapStyle::Auto
        } else {
            content.font.wrap_style
        };
        let indent = (content.font.indent.as_ref())
            .map_or(0.0, |indent| indent.resolve(width.unwrap_or(0.0)));
        if p.broken != Some((width, indent, wrap_style)) {
            p.layout.set_text_indent(indent, IndentOptions::default());
            p.layout.break_all_lines(width);
            if let Some(width) = width.filter(|_| !p.empty) {
                match wrap_style {
                    WrapStyle::Auto => {}
                    WrapStyle::Balance => balance(&mut p.layout, width),
                    WrapStyle::Pretty => pretty(&mut p.layout, width, &p.string),
                }
            }
            p.broken = Some((width, indent, wrap_style));
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

/// Avoid a final one-word line by moving the preceding word with it. Earlier lines
/// retain their greedy breaks. Each forced-break paragraph is considered separately;
/// a change that adds a line or still leaves a widow is discarded.
fn pretty(layout: &mut parley::Layout<u32>, width: f32, text: &str) {
    let mut targets = Vec::new();
    let mut paragraph = 0;
    let count = layout.len();
    for (i, line) in layout.lines().enumerate() {
        if matches!(
            line.break_reason(),
            BreakReason::Explicit | BreakReason::None
        ) {
            if i > paragraph && text[line.text_range()].unicode_words().count() == 1 {
                let previous = layout.get(i - 1).unwrap();
                let range = previous.text_range();
                let words: Vec<_> = text[range.clone()].unicode_word_indices().collect();
                if let Some((start, _)) = words.last().filter(|_| words.len() > 1) {
                    let start = range.start + start;
                    let tail: f32 = previous
                        .runs()
                        .flat_map(|r| r.clusters())
                        .filter(|c| c.text_range().start >= start)
                        .map(|c| c.advance())
                        .sum();
                    let narrow = (previous.metrics().advance - tail - 1.0 / 64.0).max(0.0);
                    targets.push((i - 1, narrow, line.text_range().end));
                }
            }
            paragraph = i + 1;
        }
    }
    if targets.is_empty() {
        return;
    }
    let mut lines = layout.break_lines();
    lines.state_mut().set_layout_max_advance(width);
    let mut index = 0;
    lines.state_mut().set_line_max_advance(
        targets
            .iter()
            .find(|(i, ..)| *i == 0)
            .map_or(width, |(_, w, _)| *w),
    );
    while let Some(yielded) = lines.break_next() {
        if matches!(yielded, YieldData::LineBreak(_)) {
            lines.set_prior_line_width(width);
            index += 1;
            lines.state_mut().set_line_max_advance(
                targets
                    .iter()
                    .find(|(i, ..)| *i == index)
                    .map_or(width, |(_, w, _)| *w),
            );
        }
    }
    lines.finish();
    if layout.len() != count
        || targets.iter().any(|(_, _, end)| {
            layout
                .lines()
                .find(|l| l.text_range().end == *end)
                .is_none_or(|l| text[l.text_range()].unicode_words().count() < 2)
        })
    {
        layout.break_all_lines(Some(width));
    }
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

/// A built-in font text falls back to: its index in `BUILT_IN`, the scripts it is the fallback
/// of, and whether a character needs it.
pub type Fallback = (usize, &'static [[u8; 4]], fn(char) -> bool);

/// The built-in fonts text falls back to when its fonts do not have a character: by the scripts
/// they are the fallback of, and the characters that need them. Noto Sans SC has Han, kana and
/// bopomofo, but no Hangul. Shaping takes an emoji's font from the `emoji` family, after those
/// its text names.
pub const FALLBACKS: [Fallback; 3] = [
    (5, &[*b"Hani", *b"Hira", *b"Kana", *b"Bopo"], |c| {
        matches!(c,
            '\u{2E80}'..='\u{2FFF}' | '\u{3000}'..='\u{312F}' | '\u{31A0}'..='\u{4DBF}'
            | '\u{4E00}'..='\u{9FFF}' | '\u{F900}'..='\u{FAFF}' | '\u{FE30}'..='\u{FE4F}'
            | '\u{FF00}'..='\u{FFEF}' | '\u{20000}'..='\u{3FFFF}')
    }),
    (6, &[*b"Hang"], |c| {
        matches!(c,
            '\u{1100}'..='\u{11FF}' | '\u{3130}'..='\u{318F}' | '\u{A960}'..='\u{A97F}'
            | '\u{AC00}'..='\u{D7FF}')
    }),
    (7, &[], emoji),
];

/// Whether `c` may show as an emoji: a pictograph, a regional indicator, or the selector that
/// asks for an emoji (VS16).
fn emoji(c: char) -> bool {
    matches!(c,
        '\u{231A}'..='\u{23FF}' | '\u{2600}'..='\u{27BF}' | '\u{2B05}'..='\u{2B55}'
        | '\u{FE0F}' | '\u{1F000}'..='\u{1FAFF}')
}

/// The characters of `text` that ask to show as an emoji with the selector VS16, as graphemes,
/// which the emoji font shapes first, as in Chrome, before a text font that has them.
fn presented(text: &str) -> impl Iterator<Item = Range<usize>> + '_ {
    (text.grapheme_indices(true))
        .filter(|(_, g)| g.contains('\u{FE0F}'))
        .map(|(i, g)| i..i + g.len())
}

/// `families` with the emoji family first.
fn emoji_first(families: &[FontFamilyName<'static>]) -> Vec<FontFamilyName<'static>> {
    let emoji = FontFamilyName::Generic(GenericFamily::Emoji);
    std::iter::once(emoji.clone())
        .chain(families.iter().filter(|&f| *f != emoji).cloned())
        .collect()
}

/// Fails on the first character of `text` that `layout` has no glyph for, as no font has it.
fn covered(layout: &parley::Layout<u32>, text: &str) -> Result<(), Error> {
    for line in layout.lines() {
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(glyphs) = item else {
                continue;
            };
            for cluster in glyphs.run().clusters() {
                if !cluster.glyphs().any(|g| g.id == 0) {
                    continue;
                }
                let shown = text[cluster.text_range()].chars().find(|&c| !invisible(c));
                if let Some(c) = shown {
                    return Err(format!(
                        "{c:?} (U+{:04X}): no font has it; declare a font resource that does, \
                         such as one for Thai or Arabic text",
                        c as u32
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Whether `c` shows nothing of its own: white space, a control, or a default ignorable code
/// point, such as a joiner or a variation selector.
fn invisible(c: char) -> bool {
    c.is_whitespace()
        || c.is_control()
        || matches!(c,
            '\u{00AD}' | '\u{034F}' | '\u{061C}' | '\u{115F}' | '\u{1160}' | '\u{17B4}'
            | '\u{17B5}' | '\u{180B}'..='\u{180F}' | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{206F}' | '\u{3164}'
            | '\u{FE00}'..='\u{FE0F}' | '\u{FEFF}' | '\u{FFA0}' | '\u{FFF0}'..='\u{FFF8}'
            | '\u{1BCA0}'..='\u{1BCA3}' | '\u{1D173}'..='\u{1D17A}' | '\u{E0000}'..='\u{E0FFF}')
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
            Family::Emoji => Ok(FontFamilyName::Generic(GenericFamily::Emoji)),
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

/// The TrueType or OpenType file to register for font file `data`, where that is not `data`
/// itself: the one a WOFF or WOFF2 file holds, and of a collection (TTC), its first face.
fn sfnt(data: &[u8]) -> Result<Option<Vec<u8>>, Error> {
    let file = match data.get(..4) {
        Some(b"wOFF") => wuff::decompress_woff1(data).map_err(|e| format!("WOFF: {e}"))?,
        Some(b"wOF2") => wuff::decompress_woff2(data).map_err(|e| format!("WOFF2: {e}"))?,
        Some(b"ttcf") => {
            return first_face(data)
                .map(Some)
                .ok_or_else(|| "a broken TTC".into());
        }
        _ => return Ok(None),
    };
    match file.starts_with(b"ttcf") {
        true => first_face(&file)
            .map(Some)
            .ok_or_else(|| "a broken TTC".into()),
        false => Ok(Some(file)),
    }
}

/// The first face of collection `data` as a font file of its own: its table directory, each
/// table's offset where the table is copied after it.
fn first_face(data: &[u8]) -> Option<Vec<u8>> {
    let u32_at = |at: usize| Some(u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?));
    let face = u32_at(12)? as usize;
    let tables = u16::from_be_bytes(data.get(face + 4..face + 6)?.try_into().ok()?) as usize;
    let mut out = data.get(face..face + 12 + 16 * tables)?.to_vec();
    for i in 0..tables {
        let record = face + 12 + 16 * i;
        let (offset, length) = (u32_at(record + 8)? as usize, u32_at(record + 12)? as usize);
        let table = data.get(offset..offset.checked_add(length)?)?;
        let at = u32::try_from(out.len()).ok()?;
        out[12 + 16 * i + 8..12 + 16 * i + 12].copy_from_slice(&at.to_be_bytes());
        out.extend_from_slice(table);
        out.resize(out.len().next_multiple_of(4), 0);
    }
    Some(out)
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
fn extent(
    line: &parley::Line<'_, u32>,
    content: &Content,
    strut: [f32; 2],
    edges: &HashMap<u32, [f32; 2]>,
    string: &str,
) -> [f32; 2] {
    let mut out = strut;
    for item in line.items() {
        if let PositionedLayoutItem::GlyphRun(run) = item {
            let piece = &content.pieces[run.style().brush as usize];
            if piece.edge.is_some() {
                if let Some([a, b]) = edges.get(&piece.node) {
                    out = [out[0].max(*a), out[1].max(*b)];
                }
                continue;
            }
            if string[run.run().text_range()]
                .chars()
                .all(|c| matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'))
            {
                continue;
            }
            let m = run.run().font_metrics();
            let font = piece.font;
            let [a, b] = placed([m.ascent, m.descent, m.leading], font);
            out = [out[0].max(a), out[1].max(b)];
        }
    }
    out
}

/// Each line box's top and height, stacked from 0.
fn stack(
    layout: &parley::Layout<u32>,
    content: &Content,
    strut: [f32; 2],
    edges: &HashMap<u32, [f32; 2]>,
    marks: &HashMap<u32, emphasis::Shape>,
    string: &str,
) -> Vec<[f32; 2]> {
    let (mut top, mut previous) = (0.0, 0.0);
    layout
        .lines()
        .enumerate()
        .map(|(index, line)| {
            let [a, b] = extent(&line, content, strut, edges, string);
            let mut extra = [0.0_f32; 2];
            for item in line.items() {
                if let PositionedLayoutItem::GlyphRun(run) = item {
                    let piece = &content.pieces[run.style().brush as usize];
                    if let Some(mark) = marks.get(&piece.node) {
                        let range = run.run().text_range();
                        if string[range].graphemes(true).any(emphasis::eligible) {
                            let required = emphasis::leading(mark, piece.font, [a, b]);
                            for k in 0..2 {
                                extra[k] = extra[k].max(required[k]);
                            }
                        }
                    }
                }
            }
            if index > 0 {
                top += previous + extra[0];
            }
            let line_box = [top, a + b];
            top += a + b;
            previous = extra[1];
            line_box
        })
        .collect()
}

pub mod color;

#[cfg(test)]
mod tests {
    use std::borrow::Cow;
    use std::rc::Rc;

    use super::*;

    #[test]
    fn cjk_ink_skipping_keeps_script_extensions_and_contextual_punctuation() {
        let parts = |s: &str| {
            cjk_ranges(s)
                .into_iter()
                .map(|r| s[r].to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            parts("(汉字), abc がー。 한글 ㄅ˪"),
            ["(汉字), ", "がー。 한글 ㄅ˪"]
        );
        assert_eq!(parts("Latin（中文） gyp"), ["（中文） "]);
        assert_eq!(
            parts("...汉\u{301}字!\n...gyp\n...かな。"),
            ["...汉\u{301}字!", "...かな。"]
        );
        assert_eq!(parts("𠀀𰀀"), ["𠀀𰀀"]);
        assert!(parts("gyp, ... 123").is_empty());
        assert!(parts("...").is_empty());
    }

    fn text() -> Text {
        crate::testing::text()
    }

    /// `text`, all in its paragraph's own font.
    fn plain<'a>(text: &'a str, font: &'a Font) -> Content<'a> {
        Content {
            font,
            pieces: vec![Piece {
                edge: None,
                text: Cow::Borrowed(text),
                node: 0,
                font,
            }],
            clamp: None,
        }
    }

    fn font(size: f64) -> Font {
        Font {
            size,
            weight: 400.0,
            stretch: 100.0,
            style: FontStyle::Normal,
            family: Rc::new([Family::SansSerif]),
            line_height: LineHeight::Normal,
            align: TextAlign::Start,
            align_last: None,
            line_break: parley::LineBreak::Strict,
            writing: WritingMode::Horizontal,
            orientation: crate::style::Orientation::Mixed,
            rtl: false,
            bidi: Default::default(),
            bidi_scopes: Rc::new([]),
            combine: None,
            emphasis: crate::style::emphasis::Mark::None,
            emphasis_color: None,
            emphasis_position: Default::default(),
            indent: None,
            spacing: 0.0,
            word_spacing: 0.0,
            collapse: Collapse::Collapse,
            wrap: true,
            wrap_style: WrapStyle::Auto,
            word_break: WordBreak::Normal,
            overflow_wrap: OverflowWrap::Normal,
            transform: TextTransform::None,
            variants: Default::default(),
            features: Rc::new([]),
            variations: Rc::new([]),
            fill: None,
            stroke: 0.0,
            stroke_color: None,
            shadows: Rc::new([]),
            skip_ink: crate::style::SkipInk::Auto,
            underline_offset: None,
            underline_position: Default::default(),
        }
    }

    #[test]
    fn emphasis_marks_graphemes_without_changing_advances() {
        let mut t = text();
        let plain_font = font(32.0);
        let marked = Font {
            emphasis: crate::style::emphasis::Mark::Text("•".into()),
            ..plain_font.clone()
        };
        let value = "office á, ! #";
        let a = t.lines(&plain(value, &plain_font), 600.0).unwrap();
        let b = t.lines(&plain(value, &marked), 600.0).unwrap();
        assert_eq!(a.glyphs, b.glyphs);
        assert_eq!(a.boxes, b.boxes);
        assert_eq!(b.marks.len(), 8); // six letters, one combining grapheme, #
        for mark in &b.marks {
            let [x, y, w, h] = crate::svg::bounds(&mark.path);
            assert!(
                (x + w / 2.0 - f64::from((mark.inline[0] + mark.inline[1]) / 2.0)).abs() < 0.01
            );
            assert!(y + h <= f64::from(b.baselines[0]));
        }
        let under = Font {
            emphasis_position: crate::style::emphasis::Position {
                under: true,
                left: false,
            },
            ..marked.clone()
        };
        let c = t.lines(&plain(value, &under), 600.0).unwrap();
        assert!(
            c.marks
                .iter()
                .all(|m| crate::svg::bounds(&m.path)[1] > f64::from(c.baselines[0]))
        );
        assert_eq!(b, t.lines(&plain(value, &marked), 600.0).unwrap());
    }

    #[test]
    fn vertical_emphasis_keeps_marks_upright_on_the_selected_side() {
        let mut t = text();
        let marked = Font {
            writing: WritingMode::VerticalRl,
            emphasis: crate::style::emphasis::Mark::Symbol("▲"),
            ..font(24.0)
        };
        let right = t.lines(&plain("春，风", &marked), 300.0).unwrap();
        let left_font = Font {
            emphasis_position: crate::style::emphasis::Position {
                under: false,
                left: true,
            },
            ..marked.clone()
        };
        let left = t.lines(&plain("春，风", &left_font), 300.0).unwrap();
        assert_eq!(right.marks.len(), 2);
        assert_eq!(right.glyphs, left.glyphs);
        for (a, b) in right.marks.iter().zip(&left.marks) {
            let a = crate::svg::bounds(&a.path);
            let b = crate::svg::bounds(&b.path);
            assert!(a[1] + a[3] < b[1]);
            assert_eq!(a[0], b[0]);
        }
    }

    #[test]
    fn emphasis_reserves_interline_space_and_truncates_with_text() {
        let mut t = text();
        let marked = Font {
            emphasis: crate::style::emphasis::Mark::Text("•".into()),
            collapse: Collapse::Preserve,
            ..font(32.0)
        };
        let a = t.lines(&plain("ABC\nDEF", &marked), 600.0).unwrap();
        let unmarked = Font {
            emphasis: crate::style::emphasis::Mark::None,
            ..marked.clone()
        };
        let b = t.lines(&plain("ABC\nDEF", &unmarked), 600.0).unwrap();
        assert!(a.baselines[1] > b.baselines[1]);
        assert_eq!(a.baselines[0], b.baselines[0]);
        let mut a = t.lines(&plain("ABCDEFG", &marked), 600.0).unwrap();
        t.truncate(&mut a, &marked, 0, 55.0, true, None).unwrap();
        assert!(a.marks.len() < 7);
        assert!(a.marks.iter().all(|m| m.inline[1] <= 55.0));
    }

    #[test]
    fn lines_stack_as_chrome_stacks_them() {
        let mut t = text();
        // Noto Sans: ascent 1.069 em and descent 0.293, which at 16px round to 17 and 5.
        let normal = font(16.0);
        assert_eq!(t.measure(&plain("Hello", &normal), None).unwrap()[1], 22.0);
        let lines = t.lines(&plain("Hello", &normal), 100.0).unwrap();
        assert_eq!(lines.boxes, [[0.0, 22.0]]);
        assert!(lines.glyphs.iter().all(|g| g.y == 17.0));

        // Half the leading, floored, goes above the ascent.
        let tall = Font {
            line_height: LineHeight::Factor(1.5),
            ..font(16.0)
        };
        let lines = t.lines(&plain("Hello", &tall), 100.0).unwrap();
        assert_eq!(lines.boxes, [[0.0, 24.0]]);
        assert_eq!(lines.glyphs[0].y, 18.0);
        let tight = Font {
            line_height: LineHeight::Px(18.0),
            ..font(16.0)
        };
        let lines = t.lines(&plain("one two", &tight), 0.0).unwrap();
        assert_eq!(lines.boxes, [[0.0, 18.0], [18.0, 18.0]]);
        assert_eq!(lines.glyphs[0].y, 15.0);
        assert_eq!(lines.glyphs.last().unwrap().y, 33.0);
        assert_eq!(t.measure(&plain("", &normal), None).unwrap(), [0.0, 0.0]);
        assert!(
            t.lines(&plain("", &normal), 100.0)
                .unwrap()
                .boxes
                .is_empty()
        );
    }

    #[test]
    fn lines_break_to_fit() {
        let mut t = text();
        let f = font(16.0);
        let [wide, one] = t.measure(&plain("one two three", &f), None).unwrap();
        assert_eq!(one, 22.0);
        // Widths are whole layout units, so the text fits again in what it measured.
        assert_eq!((wide * 64.0).fract(), 0.0);
        assert_eq!(
            t.measure(&plain("one two three", &f), Some(wide)).unwrap(),
            [wide, 22.0]
        );
        // Broken, it takes all of the width.
        let broken = t
            .measure(&plain("one two three", &f), Some(wide - 1.0))
            .unwrap();
        assert_eq!(broken, [wide - 1.0, 44.0]);
        // The min-content width: the widest word.
        let [three, _] = t.measure(&plain("three", &f), None).unwrap();
        assert_eq!(
            t.min_content(&plain("one two three", &f)).unwrap(),
            [three, 66.0]
        );
        // `white-space: normal` collapses spaces and line breaks, and trims them.
        assert_eq!(
            t.measure(&plain(" one \n  two\tthree\n", &f), None)
                .unwrap(),
            [wide, 22.0]
        );
        assert_eq!(t.measure(&plain(" \n ", &f), None).unwrap(), [0.0, 0.0]);
    }

    #[test]
    fn weight_spacing_and_alignment() {
        let mut t = text();
        let [regular, _] = t.measure(&plain("Hello", &font(16.0)), None).unwrap();
        let bold = Font {
            weight: 700.0,
            ..font(16.0)
        };
        assert!(t.measure(&plain("Hello", &bold), None).unwrap()[0] > regular);
        let spaced = Font {
            spacing: 2.0,
            ..font(16.0)
        };
        assert_eq!(
            t.measure(&plain("Hello", &spaced), None).unwrap()[0],
            regular + 10.0
        );
        // Bold is another instance of the variable face.
        t.lines(&plain("Hello", &font(16.0)), 100.0).unwrap();
        t.lines(&plain("Hello", &bold), 100.0).unwrap();
        assert_eq!(t.faces().len(), 2);
        assert_ne!(t.faces()[0].coords, t.faces()[1].coords);

        let first = |t: &mut Text, align| {
            let f = Font {
                align,
                ..font(16.0)
            };
            t.lines(&plain("Hello", &f), 100.0).unwrap().glyphs[0].x
        };
        assert_eq!(first(&mut t, TextAlign::Start), 0.0);
        let center = first(&mut t, TextAlign::Center);
        let right = first(&mut t, TextAlign::Right);
        assert!(
            (center - (100.0 - regular) / 2.0).abs() < 1.0 / 64.0,
            "{center}"
        );
        assert!((right - (100.0 - regular)).abs() < 1.0 / 64.0, "{right}");
        assert_eq!(first(&mut t, TextAlign::End), right);
    }

    #[test]
    fn families_are_built_in_or_resources() {
        let mut t = text();
        let with = |family: Family| Font {
            family: Rc::new([family, Family::SansSerif]),
            ..font(16.0)
        };
        // Each built-in family is its own font: monospace gives i and W one width.
        let mut width =
            |family: Family, s: &str| t.measure(&plain(s, &with(family)), None).unwrap()[0];
        let [sans, serif] = [Family::SansSerif, Family::Serif].map(|f| width(f, "Serif"));
        assert_ne!(sans, serif);
        assert_eq!(
            width(Family::Monospace, "iiii"),
            width(Family::Monospace, "WWWW")
        );
        assert_ne!(
            width(Family::SansSerif, "iiii"),
            width(Family::SansSerif, "WWWW")
        );
        assert_eq!(
            t.measure(&plain("a", &with(Family::Named("Brand".into()))), None)
                .unwrap_err(),
            "font family Brand: the scene declares no font resource Brand in controls.resources"
        );
        // A font resource is a family by its name, whatever its ASCII case.
        t.add_font("brand", "Serif.ttf", crate::testing::built_in(2))
            .unwrap();
        let mut width =
            |family: Family| t.measure(&plain("Serif", &with(family)), None).unwrap()[0];
        assert_eq!(width(Family::Named("Brand".into())), width(Family::Serif));
        assert_eq!(
            t.add_font("bad", "bad.ttf", Blob::from(b"not a font".to_vec()))
                .unwrap_err(),
            "not a font file: TrueType, OpenType, WOFF, WOFF2 or a collection"
        );
    }

    #[test]
    fn a_font_resource_is_a_family_of_its_files() {
        let mut t = text();
        t.add_font("brand", "Sans.ttf", crate::testing::built_in(0))
            .unwrap();
        t.add_font("brand", "Italic.ttf", crate::testing::built_in(1))
            .unwrap();
        let file = |path: &str, italic| FontFile {
            family: "brand".into(),
            path: path.into(),
            weight: [100.0, 900.0],
            width: [62.5, 100.0],
            italic,
        };
        assert_eq!(
            t.font_files(),
            [file("Sans.ttf", false), file("Italic.ttf", true)]
        );
        // Italic text takes the italic file, not the upright one slanted.
        let mut face = |style| {
            let f = Font {
                family: Rc::from([Family::Named("brand".into())]),
                style,
                ..font(16.0)
            };
            let lines = t.lines(&plain("Lean", &f), 1000.0).unwrap();
            let face = &t.faces()[lines.glyphs[0].face as usize];
            (face.data.id(), face.slanted)
        };
        let (upright, _) = face(FontStyle::Normal);
        let (italic, slanted) = face(FontStyle::Italic);
        assert!(italic != upright && !slanted);
        assert_eq!(
            t.add_font("brand", "Again.ttf", crate::testing::built_in(0))
                .unwrap_err(),
            "the same weight, style and width as Sans.ttf"
        );
    }

    #[test]
    fn a_collection_gives_its_first_face() {
        // Noto Sans, then Noto Serif, as a TTC.
        let fonts = [0, 2].map(|i| crate::testing::built_in(i).data().to_vec());
        let mut ttc = b"ttcf\0\x01\0\0\0\0\0\x02".to_vec();
        ttc.resize(20, 0);
        for (n, font) in fonts.iter().enumerate() {
            let base = ttc.len();
            ttc[12 + 4 * n..16 + 4 * n].copy_from_slice(&(base as u32).to_be_bytes());
            let mut font = font.clone();
            let tables = u16::from_be_bytes([font[4], font[5]]) as usize;
            for i in 0..tables {
                let at = 12 + 16 * i + 8;
                let offset = u32::from_be_bytes(font[at..at + 4].try_into().unwrap());
                font[at..at + 4].copy_from_slice(&(offset + base as u32).to_be_bytes());
            }
            ttc.extend(font);
            ttc.resize(ttc.len().next_multiple_of(4), 0);
        }
        let mut t = text();
        t.add_font("pair", "Pair.ttc", Blob::from(ttc)).unwrap();
        assert_eq!(t.font_files().len(), 1);
        let with = |family: Family| Font {
            family: Rc::new([family]),
            ..font(16.0)
        };
        let mut width = |family| t.measure(&plain("Serif", &with(family)), None).unwrap()[0];
        assert_eq!(
            width(Family::Named("pair".into())),
            width(Family::SansSerif)
        );
    }

    #[test]
    fn text_falls_back_to_the_fonts_of_its_scripts() {
        let mut t = text();
        let mut faces = |s: &str| {
            let lines = t.lines(&plain(s, &font(16.0)), 1000.0).unwrap();
            let faces = lines
                .glyphs
                .iter()
                .map(|g| t.faces()[g.face as usize].data.id());
            faces.collect::<Vec<_>>()
        };
        let sans = faces("A")[0];
        // Han, kana and bopomofo in Noto Sans SC; Hangul in Noto Sans KR.
        let [han, kana, bopomofo, hangul] = ["视", "か", "ㄅ", "한"].map(|s| faces(s)[0]);
        assert!(han != sans && hangul != sans && hangul != han);
        assert_eq!([kana, bopomofo], [han, han]);
        // Each character in its own: Latin stays in sans-serif.
        assert_eq!(faces("A视"), [sans, han]);
        assert_eq!(
            t.measure(&plain("ไทย", &font(16.0)), None).unwrap_err(),
            "'ไ' (U+0E44): no font has it; declare a font resource that does, such as one for \
         Thai or Arabic text"
        );
    }

    #[test]
    fn emoji_take_the_emoji_font_and_paint_in_its_colors() {
        let mut t = text();
        let lines = t.lines(&plain("A😀", &font(32.0)), 1000.0).unwrap();
        let [a, smile] = [0, 1].map(|i| lines.glyphs[i].face as usize);
        assert_ne!(t.faces()[a].data.id(), t.faces()[smile].data.id());
        let (face, id) = (&t.faces()[smile], lines.glyphs[1].id);
        assert_eq!(color::colored(face, id), Ok(true));
        assert_eq!(color::colored(&t.faces()[a], lines.glyphs[0].id), Ok(false));
        // Yellow, about an em across, from above the baseline.
        let (image, [left, top]) = color::raster(face, id, 32.0, [0, 0, 0]).unwrap();
        let [w, h] = image.size;
        assert!(
            (24..=40).contains(&w) && (24..=40).contains(&h),
            "{w} × {h}"
        );
        assert!(top < -20.0 && left > -4.0, "{left}, {top}");
        let middle = &image.pixels[((h / 2 * w + w / 4) * 4) as usize..][..4];
        assert!(
            middle[0] > 200 && middle[1] > 150 && middle[2] < 100,
            "{middle:?}"
        );
        // The family by its name.
        let emoji = Font {
            family: Rc::new([Family::Emoji]),
            ..font(16.0)
        };
        let lines = t.lines(&plain("😀", &emoji), 1000.0).unwrap();
        assert_eq!(
            t.faces()[lines.glyphs[0].face as usize].data.id(),
            t.faces()[smile].data.id()
        );
        // The selector VS16 asks for the emoji font before a text font that has the character.
        let lines = t
            .lines(&plain("A\u{A9}\u{A9}\u{FE0F}A", &font(16.0)), 1000.0)
            .unwrap();
        let ids = (lines.glyphs.iter())
            .map(|g| t.faces()[g.face as usize].data.id())
            .collect::<Vec<_>>();
        let emoji = t.faces()[smile].data.id();
        assert!(
            ids[0] != emoji && ids[1] != emoji && ids[2] == emoji,
            "{ids:?}"
        );
        assert_ne!(*ids.last().unwrap(), emoji);
    }

    #[test]
    fn paragraphs_are_kept_while_used() {
        let mut t = text();
        t.measure(&plain("a", &font(16.0)), None).unwrap();
        t.measure(&plain("a", &font(16.0)), Some(10.0)).unwrap();
        t.measure(&plain("b", &font(16.0)), None).unwrap();
        assert_eq!(t.paragraphs.len(), 2);
        t.sweep();
        t.measure(&plain("a", &font(16.0)), None).unwrap();
        t.sweep();
        assert_eq!(t.paragraphs.len(), 1);
        t.sweep();
        assert!(t.paragraphs.is_empty());
    }

    #[test]
    fn white_space_collapses_across_pieces_into_the_piece_it_starts_in() {
        let normal = |text| (text, Collapse::Collapse, TextTransform::None);
        let pieces = [" a ", " ", "b  ", ""].map(normal);
        assert_eq!(
            prepare(pieces.into_iter()),
            Ok(("a b".to_string(), vec![0..2, 2..2, 2..3, 3..3]))
        );
        // Kept white space keeps what follows it, but not the spaces a line break takes.
        let pieces = [
            normal("a  "),
            ("  b \n c  ", Collapse::PreserveBreaks, TextTransform::None),
            (" d  e ", Collapse::Preserve, TextTransform::None),
            normal("\n f"),
        ];
        let (text, _) = prepare(pieces.into_iter()).unwrap();
        assert_eq!(text, "a b\nc  d  e  f");
        let tab = [("a\tb", Collapse::Preserve, TextTransform::None)];
        assert!(prepare(tab.into_iter()).is_err());
    }

    #[test]
    fn text_transforms_case_letters_across_pieces() {
        let piece = |text, transform| (text, Collapse::Collapse, transform);
        let pieces = [
            piece("don't stop-", TextTransform::Capitalize),
            piece("me 3d", TextTransform::Capitalize),
            piece(" Straße", TextTransform::Uppercase),
            piece(" ÉCOLE", TextTransform::Lowercase),
        ];
        let (text, _) = prepare(pieces.into_iter()).unwrap();
        assert_eq!(text, "Don't Stop-Me 3d STRASSE école");
    }

    #[test]
    fn pieces_shape_in_their_own_fonts_on_shared_lines() {
        let mut t = text();
        let (normal, big) = (font(16.0), font(32.0));
        let piece = |text: &'static str, node, font| Piece {
            edge: None,
            text: Cow::Borrowed(text),
            node,
            font,
        };
        let content = Content {
            font: &normal,
            pieces: vec![
                piece("Hi ", 0, &normal),
                piece(" big ", 1, &big),
                piece(" end", 0, &normal),
            ],
            clamp: None,
        };
        let lines = t.lines(&content, 1000.0).unwrap();
        assert_eq!(&*lines.string, "Hi big end");
        let nodes: Vec<u32> = lines.runs.iter().map(|r| r.node).collect();
        assert_eq!(nodes, [0, 1, 0]);
        // At 32px the ascent rounds to 34 and the descent to 9: the big run sets the line, and all
        // its glyphs sit on one baseline.
        assert_eq!(
            (&lines.boxes[..], &lines.baselines[..]),
            (&[[0.0, 43.0]][..], &[34.0][..])
        );
        assert!(lines.glyphs.iter().all(|g| g.y == 34.0));
        assert_eq!(lines.glyphs[0].node, 0);
        // A line holds its paragraph's own font, even with only smaller text on it.
        let small = Content {
            font: &big,
            pieces: vec![piece("small", 1, &normal)],
            clamp: None,
        };
        assert_eq!(t.measure(&small, None).unwrap()[1], 43.0);
        assert_eq!(t.metrics(&normal).unwrap()[0].round(), 17.0);
    }

    #[test]
    fn ink_is_where_a_glyph_crosses_a_band() {
        let mut t = text();
        let f = font(20.0);
        let lines = t.lines(&plain("xg", &f), 1000.0).unwrap();
        assert_eq!(lines.runs[0].glyphs, [0, 2]);
        let [x, g] = [lines.glyphs[0], lines.glyphs[1]];
        let below = [1.5, 2.5];
        assert_eq!(t.ink(x.face, x.id, 20.0, below, false, 1.0).unwrap(), None);
        let [left, right] = t
            .ink(g.face, g.id, 20.0, below, false, 1.0)
            .unwrap()
            .unwrap();
        // The g's tail, within its advance.
        assert!(
            0.0 < left && left < right && right < g.x - x.x,
            "{left} {right}"
        );
    }

    #[test]
    fn words_break_where_word_break_and_overflow_wrap_let_them() {
        let mut t = text();
        let with = |word_break, overflow_wrap| Font {
            word_break,
            overflow_wrap,
            ..font(16.0)
        };
        let normal = font(16.0);
        let [word, _] = t.measure(&plain("unbreakable", &normal), None).unwrap();
        let lines =
            |t: &mut Text, f: &Font| t.lines(&plain("unbreakable go", f), word / 2.0).unwrap();
        // A word too wide for its line overflows it, unless a word may break.
        assert_eq!(lines(&mut t, &normal).text.len(), 2);
        let breaking = [
            with(WordBreak::BreakAll, OverflowWrap::Normal),
            with(WordBreak::BreakWord, OverflowWrap::Normal),
            with(WordBreak::Normal, OverflowWrap::Anywhere),
            with(WordBreak::Normal, OverflowWrap::BreakWord),
        ];
        for f in &breaking {
            assert!(lines(&mut t, f).text.len() > 2, "{f:?}");
        }
        // The min-content width is the widest word's, or a letter's where words break, except that
        // overflow-wrap: break-word only breaks words that do not fit.
        let min = |t: &mut Text, f: &Font| t.min_content(&plain("unbreakable go", f)).unwrap()[0];
        assert_eq!(min(&mut t, &normal), word);
        assert_eq!(min(&mut t, &breaking[3]), word);
        for f in &breaking[..3] {
            assert!(min(&mut t, f) < word / 4.0, "{f:?}");
        }
        let zero = t.measure(&plain("unbreakable go", &breaking[3]), Some(0.0));
        assert!(zero.unwrap()[0] < word / 4.0);
    }

    #[test]
    fn last_line_alignment_applies_after_wrapping_and_forced_breaks() {
        let mut t = text();
        let mut f = font(20.0);
        f.collapse = Collapse::Preserve;
        f.align_last = Some(TextAlign::Right);
        let c = plain("One two three four\nLast", &f);
        let lines = t.lines(&c, 130.0).unwrap();
        for (i, range) in lines.text.iter().enumerate() {
            let right = lines
                .runs
                .iter()
                .filter(|r| r.line as usize == i)
                .map(|r| r.x[1])
                .fold(0.0, f32::max);
            let last =
                range.end == lines.string.len() || lines.string[range.clone()].ends_with('\n');
            if last {
                assert!((right - 130.0).abs() < 0.05, "{range:?}: {right}");
            }
        }
        let lines = t
            .lines(
                &plain(
                    "One two",
                    &Font {
                        align_last: Some(TextAlign::Justify),
                        ..f
                    },
                ),
                200.0,
            )
            .unwrap();
        assert!((lines.runs.last().unwrap().x[1] - 200.0).abs() < 0.05);
    }

    #[test]
    fn last_column_alignment_does_not_change_intrinsic_measurement() {
        let mut t = text();
        let f = Font {
            writing: WritingMode::VerticalRl,
            ..font(20.0)
        };
        let expected = t.measure(&plain("ABC", &f), Some(200.0)).unwrap();
        for align in [TextAlign::Right, TextAlign::Center, TextAlign::Justify] {
            let aligned = Font {
                align_last: Some(align),
                ..f.clone()
            };
            assert_eq!(
                t.measure(&plain("ABC", &aligned), Some(200.0)).unwrap(),
                expected
            );
        }
    }

    #[test]
    fn pretty_moves_a_word_to_a_lonely_last_line_without_adding_lines() {
        let mut t = text();
        let f = font(20.0);
        let words = "One two three four five six seven";
        let pretty = Font {
            wrap_style: WrapStyle::Pretty,
            ..f.clone()
        };
        let mut found = false;
        for width in [240.0, 250.0, 260.0, 270.0, 280.0, 290.0] {
            let a = t.lines(&plain(words, &f), width).unwrap();
            let b = t.lines(&plain(words, &pretty), width).unwrap();
            assert_eq!(a.boxes.len(), b.boxes.len());
            if a.text
                .last()
                .is_some_and(|r| a.string[r.clone()].unicode_words().count() == 1)
                && b.text
                    .last()
                    .is_some_and(|r| b.string[r.clone()].unicode_words().count() > 1)
            {
                found = true;
            }
            let again = t.lines(&plain(words, &f), width).unwrap();
            assert_eq!(a, again, "changing wrap style must invalidate line breaks");
        }
        assert!(found, "pretty must improve a lonely final word");
    }

    #[test]
    fn line_break_anywhere_overrides_no_break_spaces_and_keep_all() {
        let mut t = text();
        let f = Font {
            word_break: WordBreak::KeepAll,
            ..font(20.0)
        };
        let normal = t.lines(&plain("AB\u{a0}CD", &f), 25.0).unwrap();
        let anywhere = t
            .lines(
                &plain(
                    "AB\u{a0}CD",
                    &Font {
                        line_break: parley::LineBreak::Anywhere,
                        ..f.clone()
                    },
                ),
                25.0,
            )
            .unwrap();
        assert_eq!(normal.boxes.len(), 1);
        assert!(anywhere.boxes.len() > 1);
        assert_eq!(normal, t.lines(&plain("AB\u{a0}CD", &f), 25.0).unwrap());
    }

    #[test]
    fn normal_and_strict_keep_small_kana_with_the_preceding_character() {
        let mut t = text();
        let f = font(20.0);
        let words = "春あぁ天あぁ山";
        let strict = t.lines(&plain(words, &f), 52.0).unwrap();
        let normal = t
            .lines(
                &plain(
                    words,
                    &Font {
                        line_break: parley::LineBreak::Normal,
                        ..f.clone()
                    },
                ),
                52.0,
            )
            .unwrap();
        let loose = t
            .lines(
                &plain(
                    words,
                    &Font {
                        line_break: parley::LineBreak::Loose,
                        ..f
                    },
                ),
                52.0,
            )
            .unwrap();
        assert_eq!(strict.text, normal.text);
        assert_eq!(strict.text.len(), 5);
        assert_eq!(loose.text.len(), 4);
    }

    #[test]
    fn balanced_lines_are_as_narrow_as_they_go_up_to_the_first_forced_break() {
        let mut t = text();
        let (normal, balanced) = (
            font(16.0),
            Font {
                wrap_style: WrapStyle::Balance,
                align: TextAlign::Center,
                ..font(16.0)
            },
        );
        let words = "The quick brown fox jumps over the lazy dog";
        let [wide, _] = t.measure(&plain(words, &normal), None).unwrap();
        let width = wide * 0.7;
        let line_widths = |lines: &Lines| -> Vec<f32> {
            (0..lines.boxes.len() as u32)
                .map(|i| {
                    let runs = lines.runs.iter().filter(|r| r.line == i);
                    runs.map(|r| r.x[1] - r.x[0]).sum()
                })
                .collect()
        };
        let greedy = line_widths(&t.lines(&plain(words, &normal), width).unwrap());
        let lines = t.lines(&plain(words, &balanced), width).unwrap();
        let even = line_widths(&lines);
        assert_eq!((even.len(), greedy.len()), (2, 2));
        assert!(
            even[0].max(even[1]) < greedy[0] && (even[0] - even[1]).abs() < greedy[0] - greedy[1],
            "{even:?} {greedy:?}"
        );
        // Still centered in the whole width.
        let first = lines.runs[0].x;
        assert!(
            ((first[0] + first[1]) / 2.0 - width / 2.0).abs() < 1.0,
            "{first:?}"
        );
        assert_eq!(
            t.measure(&plain(words, &balanced), Some(width)).unwrap(),
            t.measure(&plain(words, &normal), Some(width)).unwrap()
        );
        // Lines after a forced break, and text of more than 6 lines, break as usual.
        let pre_line = |f: &Font| Font {
            collapse: Collapse::PreserveBreaks,
            ..f.clone()
        };
        let text = format!("{words}\n{words}");
        let after = t.lines(&plain(&text, &pre_line(&balanced)), width).unwrap();
        let usual = t.lines(&plain(&text, &pre_line(&normal)), width).unwrap();
        assert_eq!(after.text[0], lines.text[0]);
        assert_eq!(after.text[2..], usual.text[2..]);
        let long = [words; 4].join(" ");
        let many = t.lines(&plain(&long, &balanced), width).unwrap();
        assert!(many.boxes.len() > 6);
        assert_eq!(
            many.text,
            t.lines(&plain(&long, &normal), width).unwrap().text
        );
    }

    #[test]
    fn lines_that_overflow_end_in_an_ellipsis_after_the_glyphs_that_fit() {
        let mut t = text();
        let f = Font {
            wrap: false,
            ..font(16.0)
        };
        let words = "The quick brown fox";
        let [wide, _] = t.measure(&plain(words, &f), None).unwrap();
        let width = (wide / 2.0).round();
        let mut lines = t.lines(&plain(words, &f), width).unwrap();
        let whole = lines.clone();
        lines.decorations.push(Stripe {
            rect: [0.0, 18.0, whole.runs[0].x[1], 19.0],
            along: [0.0, whole.runs[0].x[1]],
            style: DecorationStyle::Solid,
            thickness: 1.0,
            color: [0.0, 0.0, 0.0, 1.0],
            run: 0,
            through: false,
        });
        t.truncate(&mut lines, &f, 9, width, true, None).unwrap();
        // The run keeps its extent; the ellipsis is a run of its own, of the Text.
        assert_eq!(
            lines.runs[0],
            Run {
                edge: None,
                glyphs: lines.runs[0].glyphs,
                ..whole.runs[0]
            }
        );
        let [_, ellipsis] = &lines.runs[..] else {
            panic!("{:?}", lines.runs)
        };
        assert_eq!((ellipsis.node, ellipsis.line), (9, 0));
        let cut = ellipsis.x[0];
        assert!(
            ellipsis.x[1] <= width && ellipsis.x[1] > width - 12.0,
            "{ellipsis:?}"
        );
        let kept = lines.runs[0].glyphs[1] as usize;
        assert!(kept > 2 && kept < whole.glyphs.len() - 2, "{kept}");
        assert_eq!(lines.glyphs[..kept], whole.glyphs[..kept]);
        assert_eq!(cut, whole.glyphs[kept].x);
        // Its decorations end where the ellipsis starts.
        assert_eq!(lines.decorations[0].rect[2], cut);
        // A line that fits stays as it is.
        let mut fits = t.lines(&plain(words, &f), wide).unwrap();
        let before = fits.clone();
        t.truncate(&mut fits, &f, 9, wide, true, None).unwrap();
        assert_eq!(fits, before);
    }

    #[test]
    fn clamped_text_is_as_tall_as_its_lines_and_ends_the_last_in_an_ellipsis() {
        let mut t = text();
        let f = font(16.0);
        let words = "aaaa aaaa aaaa";
        let [word, _] = t.measure(&plain("aaaa", &f), None).unwrap();
        let advance = t.ellipsis(&f).unwrap().advance;
        let clamped = |n| Content {
            clamp: Some(n),
            ..plain(words, &f)
        };
        // With room for the ellipsis, it follows line 2's glyphs, all kept.
        let roomy = word + advance + 1.0;
        let whole = t.lines(&plain(words, &f), roomy).unwrap();
        assert_eq!(whole.boxes.len(), 3);
        let [top, height] = whole.boxes[1];
        assert_eq!(
            t.measure(&clamped(2), Some(roomy)).unwrap()[1],
            top + height
        );
        let mut lines = whole.clone();
        t.truncate(&mut lines, &f, 9, roomy, false, Some(2))
            .unwrap();
        assert_eq!(lines.glyphs.len(), whole.glyphs.len() + 1);
        let ellipsis = lines.runs.last().unwrap();
        assert_eq!((ellipsis.node, ellipsis.line), (9, 1));
        assert_eq!(ellipsis.x[0], whole.runs[1].x[1]);
        // Without it, the glyphs past where it starts go.
        let tight = word + advance - 2.0;
        let mut lines = t.lines(&plain(words, &f), tight).unwrap();
        let before = lines.glyphs.len();
        t.truncate(&mut lines, &f, 9, tight, false, Some(2))
            .unwrap();
        assert!(lines.glyphs.len() < before, "{lines:?}");
        assert!(lines.runs.last().unwrap().x[1] <= tight);
        // The last line shown, with none after it, stays as it is.
        let mut last = whole.clone();
        t.truncate(&mut last, &f, 9, roomy, false, Some(3)).unwrap();
        assert_eq!(last, whole);
    }

    #[test]
    fn font_features_choose_glyphs_and_a_later_one_of_a_tag_wins() {
        let mut t = text();
        let mut ids = |f: &Font| {
            let lines = t.lines(&plain("abc", f), 1000.0).unwrap();
            lines.glyphs.iter().map(|g| g.id).collect::<Vec<_>>()
        };
        let with = |features: &[(&[u8; 4], u16)]| Font {
            features: (features.iter())
                .map(|&(tag, value)| Feature { tag: *tag, value })
                .collect(),
            ..font(16.0)
        };
        let normal = ids(&font(16.0));
        assert_ne!(ids(&with(&[(b"smcp", 1)])), normal);
        assert_eq!(ids(&with(&[(b"smcp", 1), (b"smcp", 0)])), normal);
    }

    #[test]
    fn font_variations_set_axes_over_the_weight() {
        let mut t = text();
        let mut wide = |f: &Font| t.measure(&plain("Heavy words", f), None).unwrap()[0];
        let axes = |axes: &[(&[u8; 4], f32)]| Font {
            variations: (axes.iter())
                .map(|&(tag, value)| Variation { tag: *tag, value })
                .collect(),
            ..font(16.0)
        };
        let bold = Font {
            weight: 900.0,
            ..font(16.0)
        };
        assert_eq!(wide(&axes(&[(b"wght", 900.0)])), wide(&bold));
        assert_ne!(wide(&bold), wide(&font(16.0)));
        let narrow = Font {
            weight: 900.0,
            ..axes(&[(b"wdth", 62.5), (b"wght", 400.0)])
        };
        assert!(wide(&narrow) < wide(&font(16.0)));
    }

    #[test]
    fn font_stretch_sets_the_width_axis_up_to_its_ends() {
        let mut t = text();
        let mut wide = |stretch| {
            let f = Font {
                stretch,
                ..font(16.0)
            };
            t.measure(&plain("Wide words", &f), None).unwrap()[0]
        };
        assert!(wide(75.0) < wide(100.0));
        assert_eq!(wide(150.0), wide(100.0));
        assert_eq!(wide(50.0), wide(62.5));
    }

    #[test]
    fn italic_text_has_a_slanted_face_where_its_font_has_no_italic() {
        let mut t = text();
        let mut slanted = |family, style| {
            let f = Font {
                family: Rc::from([family]),
                style,
                ..font(16.0)
            };
            let lines = t.lines(&plain("Lean", &f), 1000.0).unwrap();
            let face = &t.faces()[lines.glyphs[0].face as usize];
            (face.data.id(), face.slanted, lines.runs[0].x)
        };
        let (mono, upright, x) = slanted(Family::Monospace, FontStyle::Normal);
        assert!(!upright);
        // Slanting moves no glyph.
        assert_eq!(
            slanted(Family::Monospace, FontStyle::Italic),
            (mono, true, x)
        );
        assert_eq!(
            slanted(Family::Monospace, FontStyle::Oblique(14.0)),
            (mono, true, x)
        );
        assert_eq!(
            slanted(Family::Monospace, FontStyle::Oblique(10.0)),
            (mono, false, x)
        );
        // Sans-serif has an italic, its own file.
        let (sans, _, x) = slanted(Family::SansSerif, FontStyle::Normal);
        let (italic, slanted, italic_x) = slanted(Family::SansSerif, FontStyle::Italic);
        assert!(italic != sans && !slanted && italic_x != x);
    }
}
