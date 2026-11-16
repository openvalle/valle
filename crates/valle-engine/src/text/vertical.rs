//! Vertical paragraphs keep logical inline/block coordinates until paint. Font fallback
//! comes from Parley; upright clusters use HarfRust's vertical forms, origins and advances.
use std::collections::{HashMap, HashSet};
use unicode_segmentation::UnicodeSegmentation;

use icu_properties::{
    CodePointMapData,
    props::{EastAsianWidth, GeneralCategory, VerticalOrientation},
};
use skrifa::raw::TableProvider;

use super::{Content, Glyph, Lines, Run, Text};
use crate::style::{
    Collapse, Font, Orientation, OverflowWrap, TextAlign, WordBreak, WrapStyle, WritingMode,
};

struct Cluster {
    atom: Option<super::Atom>,
    range: std::ops::Range<usize>,
    edge: Option<(bool, u32)>,
    node: u32,
    glyphs: Vec<Glyph>,
    advance: f32,
    upright: bool,
    combined: bool,
    space: bool,
    collapse: bool,
    wrap: bool,
    emergency: bool,
    break_after: bool,
    extent: [f32; 2],
}

pub(super) fn lines(
    text: &mut Text,
    content: &Content,
    width: Option<f32>,
    min: bool,
) -> Result<Lines, String> {
    let original = layout(text, content, width, min, width)?;
    let pretty = content.font.wrap_style == WrapStyle::Pretty;
    let widow = |lines: &Lines| {
        lines
            .text
            .last()
            .is_some_and(|r| lines.string[r.clone()].unicode_words().count() == 1)
    };
    let eligible = match content.font.wrap_style {
        WrapStyle::Auto => false,
        WrapStyle::Balance => (2..=6).contains(&original.boxes.len()),
        WrapStyle::Pretty => original.boxes.len() > 1 && widow(&original),
    };
    let Some(width) = width.filter(|_| eligible && !min) else {
        return Ok(original);
    };
    let n = original.boxes.len();
    let (mut low, mut high) = (0, (width * 64.0).floor() as u32);
    while high.saturating_sub(low) > 1 {
        let mid = low + (high - low) / 2;
        if layout(text, content, Some(mid as f32 / 64.0), min, Some(width))?
            .boxes
            .len()
            <= n
        {
            high = mid;
        } else {
            low = mid;
        }
    }
    let result = layout(text, content, Some(high as f32 / 64.0), min, Some(width))?;
    Ok(if pretty && widow(&result) {
        original
    } else {
        result
    })
}

fn layout(
    text: &mut Text,
    content: &Content,
    width: Option<f32>,
    min: bool,
    alignment_width: Option<f32>,
) -> Result<Lines, String> {
    let marks = super::emphasis::shapes(text, content)?;
    // An inherited combination split by an inline box boundary must not combine
    // fragments independently. Explicit sibling scopes remain independent.
    let mut owners = HashMap::new();
    let mut split = HashSet::new();
    for p in &content.pieces {
        if let Some(owner) = p.font.combine
            && owners.insert(owner, p.node).is_some_and(|n| n != p.node)
        {
            split.insert(owner);
        }
    }
    let combines = |font: &Font| font.combine.is_some_and(|owner| !split.contains(&owner));
    let plain: Vec<_> = content
        .pieces
        .iter()
        .map(|p| {
            let mut f = p.font.clone();
            f.emphasis = crate::style::emphasis::Mark::None;
            if combines(&f) {
                f.spacing = 0.0;
                f.collapse = Collapse::Collapse;
                f.wrap = false;
                let count = p.text.graphemes(true).count();
                if let Some(tag) = match count {
                    2 => Some(*b"hwid"),
                    3 => Some(*b"twid"),
                    4 => Some(*b"qwid"),
                    _ => None,
                } {
                    f.features = f
                        .features
                        .iter()
                        .copied()
                        .chain([crate::style::Feature { tag, value: 1 }])
                        .collect();
                }
            }
            f
        })
        .collect();
    let mut font = content.font.clone();
    font.emphasis = crate::style::emphasis::Mark::None;
    font.writing = WritingMode::Horizontal;
    font.align = TextAlign::Left;
    font.align_last = None;
    font.indent = None;
    let horizontal = Content {
        scopes: Vec::new(),
        font: &font,
        pieces: content
            .pieces
            .iter()
            .zip(&plain)
            .map(|(p, font)| super::Piece {
                atom: p.atom,
                edge: p.edge.as_ref().map(|e| super::Edge {
                    width: crate::style::math::Math::Sum([
                        e.width.resolve(width.unwrap_or(0.0)) as f64,
                        0.0,
                    ]),
                    start: e.start,
                }),
                text: p.text.clone(),
                node: p.node,
                font,
            })
            .collect(),
        clamp: None,
    };
    let paragraph = text.paragraph(&horizontal, Some(0.0), true)?;
    let breaks: HashSet<_> = paragraph
        .layout
        .lines()
        .map(|line| paragraph.map.at(line.text_range().end))
        .collect();
    let base = text.lines(&horizontal, f32::MAX)?;
    let mut out = Lines {
        atoms: Vec::new(),
        offsets: Vec::new(),
        glyphs: Vec::new(),
        runs: Vec::new(),
        boxes: Vec::new(),
        baselines: Vec::new(),
        string: base.string.clone(),
        text: Vec::new(),
        decorations: Vec::new(),
        marks: Vec::new(),
        fragments: Vec::new(),
    };
    let strut = text.strut(content.font)?;
    let metrics = text.metrics(content.font)?;
    let central = (metrics[0].round() - metrics[1].round()) * 0.5;
    let mut top = 0.0;
    let mut previous_extra = 0.0;
    for (line, range) in base.text.iter().enumerate() {
        let mut clusters: Vec<Cluster> = Vec::new();
        for run in base.runs.iter().filter(|r| r.line as usize == line) {
            let font = content
                .pieces
                .iter()
                .find(|p| p.node == run.node)
                .map_or(content.font, |p| p.font);
            // Mixed/upright paragraphs align different sizes on their central baseline.
            // The horizontal shaping pass aligned them on the alphabetic baseline.
            let baseline_shift = if content.font.orientation == Orientation::Sideways {
                0.0
            } else {
                let metrics = text.metrics(font)?;
                (metrics[0].round() - metrics[1].round()) * 0.5 - central
            };
            let mut extent = text.strut(font)?;
            if content.font.orientation != Orientation::Sideways {
                let leading = (extent[0] + extent[1] - strut[0] - strut[1]) * 0.5;
                extent = [strut[0] + leading, strut[1] + leading];
            }
            if let Some(a) = base.atoms.iter().find(|a| a.node == run.node) {
                let atom = content
                    .pieces
                    .iter()
                    .find(|p| p.node == run.node)
                    .unwrap()
                    .atom
                    .unwrap();
                if let Some(c) = clusters.last_mut() {
                    c.break_after = true;
                }
                clusters.push(Cluster {
                    atom: Some(atom),
                    range: a.at as usize..a.at as usize,
                    edge: None,
                    node: a.node,
                    glyphs: Vec::new(),
                    advance: atom.size[0],
                    upright: false,
                    combined: false,
                    space: false,
                    collapse: false,
                    wrap: font.wrap,
                    emergency: false,
                    break_after: true,
                    extent: [atom.baseline, atom.size[1] - atom.baseline],
                });
                continue;
            }
            let glyphs = &base.glyphs[run.glyphs[0] as usize..run.glyphs[1] as usize];
            if let Some((start, at)) = run.edge {
                if !start && let Some(c) = clusters.last_mut() {
                    c.break_after = false;
                }
                clusters.push(Cluster {
                    atom: None,
                    range: at as usize..at as usize,
                    edge: run.edge,
                    node: run.node,
                    glyphs: Vec::new(),
                    advance: run.x[1] - run.x[0],
                    upright: false,
                    combined: false,
                    space: false,
                    collapse: false,
                    wrap: font.wrap,
                    emergency: false,
                    break_after: !start && breaks.contains(&(at as usize)),
                    extent,
                });
                continue;
            }
            let groups: Vec<_> = glyphs.chunk_by(|a, b| a.cluster == b.cluster).collect();
            for (k, group) in groups.iter().enumerate() {
                let start = group[0].cluster as usize;
                let end = base
                    .glyphs
                    .iter()
                    .filter(|g| g.cluster as usize > start)
                    .map(|g| g.cluster as usize)
                    .min()
                    .unwrap_or(base.string.len());
                let value = base.string[start..end].trim_end_matches('\n');
                if value.is_empty() {
                    continue;
                }
                if value.contains('\n') && value.trim().is_empty() {
                    continue;
                }
                let first = value.chars().next().unwrap_or(' ');
                let category = CodePointMapData::<VerticalOrientation>::new().get(first);
                let mut upright = match font.orientation {
                    _ if combines(font) => false,
                    Orientation::Upright => true,
                    Orientation::Sideways => false,
                    Orientation::Mixed => matches!(
                        category,
                        VerticalOrientation::Upright | VerticalOrientation::TransformedUpright
                    ),
                };
                let origin = group[0].x;
                let advance = groups.get(k + 1).map_or(run.x[1], |g| g[0].x) - origin;
                let vertical = if !combines(font)
                    && (upright
                        || font.orientation == Orientation::Mixed
                            && category == VerticalOrientation::TransformedRotated)
                {
                    let shaped = upright_glyphs(text, group[0], value, font)?;
                    upright |= shaped.0.iter().map(|g| g.id).ne(group.iter().map(|g| g.id));
                    upright.then_some(shaped)
                } else {
                    None
                };
                let (glyphs, advance) = if let Some((glyphs, advance, _)) = vertical {
                    (glyphs, advance)
                } else {
                    (
                        group
                            .iter()
                            .map(|g| Glyph {
                                x: g.x - origin,
                                y: g.y - base.baselines[line] + baseline_shift,
                                ..*g
                            })
                            .collect(),
                        advance,
                    )
                };
                let space = value.chars().all(char::is_whitespace);
                let break_after = match font.word_break {
                    WordBreak::BreakAll => true,
                    WordBreak::KeepAll => space || value.ends_with('-'),
                    _ => breaks.contains(&end),
                };
                clusters.push(Cluster {
                    atom: None,
                    edge: None,
                    range: start..end,
                    node: run.node,
                    glyphs,
                    advance,
                    upright,
                    combined: false,
                    space,
                    collapse: matches!(
                        font.collapse,
                        Collapse::Collapse | Collapse::PreserveBreaks
                    ),
                    wrap: font.wrap,
                    emergency: font.word_break == WordBreak::BreakWord
                        || font.overflow_wrap == OverflowWrap::Anywhere
                        || (!min && font.overflow_wrap == OverflowWrap::BreakWord),
                    break_after,
                    extent,
                });
            }
        }
        // Shape adjacent upright clusters together: vchw and vertical kerning need
        // neighboring punctuation, including across Span boundaries with equal fonts.
        let font_of = |node| {
            content
                .pieces
                .iter()
                .find(|p| p.node == node)
                .map_or(content.font, |p| p.font)
        };
        // One box's consecutive text becomes one upright 1em cell. A font fallback
        // may split the shaping runs, but does not split this composition.
        let mut from = 0;
        while from < clusters.len() {
            let font = font_of(clusters[from].node);
            if !combines(font) || clusters[from].edge.is_some() || clusters[from].atom.is_some() {
                from += 1;
                continue;
            }
            let mut to = from + 1;
            while to < clusters.len()
                && clusters[to].node == clusters[from].node
                && clusters[to].edge.is_none()
                && clusters[to].atom.is_none()
            {
                to += 1;
            }
            let em = font.size as f32;
            let width: f32 = clusters[from..to].iter().map(|c| c.advance).sum();
            let scale = if width > em && width > 0.0 {
                em / width
            } else {
                1.0
            };
            let metrics = text.metrics(font)?;
            let baseline = em / 2.0 + (metrics[0].round() - metrics[1].round()) / 2.0;
            let mut pen = 0.0;
            let mut glyphs = Vec::new();
            for c in &clusters[from..to] {
                glyphs.extend(c.glyphs.iter().map(|g| Glyph {
                    x: (g.x + pen - width / 2.0) * scale,
                    y: baseline + g.y,
                    scale_x: scale,
                    ..*g
                }));
                pen += c.advance;
            }
            let end = clusters[to - 1].range.end;
            let break_after = clusters[to - 1].break_after;
            let first = &mut clusters[from];
            first.glyphs = glyphs;
            first.range.end = end;
            first.advance = em;
            first.upright = true;
            first.combined = true;
            first.emergency = false;
            first.break_after = break_after;
            clusters.drain(from + 1..to);
            from += 1;
        }
        let mut from = 0;
        while from < clusters.len() {
            if !clusters[from].upright || clusters[from].combined {
                from += 1;
                continue;
            }
            let font = font_of(clusters[from].node);
            let face = clusters[from].glyphs[0].face;
            let mut to = from + 1;
            while to < clusters.len()
                && clusters[to].upright
                && !clusters[to].combined
                && clusters[to].glyphs[0].face == face
                && clusters[to].glyphs[0].size == clusters[from].glyphs[0].size
                && font_of(clusters[to].node) == font
            {
                to += 1;
            }
            let first = clusters[from].glyphs[0];
            let value = &base.string[clusters[from].range.start..clusters[to - 1].range.end];
            let (glyphs, _, advances) = upright_glyphs(text, first, value, font)?;
            let mut pen = 0.0;
            for c in &mut clusters[from..to] {
                c.glyphs.clear();
                c.advance = 0.0;
                let origin = pen;
                for (g, a) in glyphs
                    .iter()
                    .zip(&advances)
                    .filter(|(g, _)| c.range.contains(&(g.cluster as usize)))
                {
                    c.glyphs.push(Glyph {
                        y: g.y - origin,
                        node: c.node,
                        ..*g
                    });
                    c.advance += a;
                    pen += a;
                }
                c.advance += font.spacing as f32
                    + if c.space {
                        font.word_spacing as f32
                    } else {
                        0.0
                    };
            }
            from = to;
        }
        let mut start = 0;
        loop {
            let indent = if out.boxes.is_empty() {
                content
                    .font
                    .indent
                    .as_ref()
                    .map_or(0.0, |v| v.resolve(width.unwrap_or(0.0)))
            } else {
                0.0
            };
            let mut end = clusters.len();
            if let Some(width) = width {
                let mut advance = indent;
                let mut fitting = None;
                for (i, c) in clusters.iter().enumerate().skip(start) {
                    advance += c.advance;
                    let visible = advance
                        - if c.space && c.collapse {
                            c.advance
                        } else {
                            0.0
                        };
                    if visible > width + 0.001 && c.wrap {
                        if let Some(at) = fitting {
                            end = at;
                            break;
                        }
                        if c.emergency && i > start {
                            end = i;
                            break;
                        }
                        if c.break_after {
                            end = i + 1;
                            break;
                        }
                    }
                    if c.break_after && c.wrap {
                        fitting = Some(i + 1);
                    }
                }
            }
            let mut shown = end;
            while shown > start && clusters[shown - 1].space && clusters[shown - 1].collapse {
                shown -= 1;
            }
            let slice = &clusters[start..shown];
            let advance = indent + slice.iter().map(|c| c.advance).sum::<f32>();
            let extent = slice
                .iter()
                .fold(strut, |a, c| [a[0].max(c.extent[0]), a[1].max(c.extent[1])]);
            let mut extra = [0.0_f32; 2];
            for c in slice {
                if let Some(mark) = marks.get(&c.node)
                    && super::emphasis::eligible(&base.string[c.range.clone()])
                {
                    let required = super::emphasis::leading(mark, font_of(c.node), extent);
                    for k in 0..2 {
                        extra[k] = extra[k].max(required[k]);
                    }
                }
            }
            if !out.boxes.is_empty() {
                top += previous_extra + extra[0];
            }
            previous_extra = extra[1];
            let height = extent[0] + extent[1];
            let baseline = top + extent[0];
            let center = baseline - central;
            let extra = alignment_width.map_or(0.0, |w| (w - advance).max(0.0));
            let last = end == clusters.len();
            let align = if last {
                content
                    .font
                    .align_last
                    .unwrap_or(if content.font.align == TextAlign::Justify {
                        TextAlign::Start
                    } else {
                        content.font.align
                    })
            } else {
                content.font.align
            };
            let alignment = match align {
                TextAlign::Start if content.font.rtl => extra,
                TextAlign::End if !content.font.rtl => extra,
                TextAlign::Right => extra,
                TextAlign::Center => extra * 0.5,
                _ => 0.0,
            };
            let spaces = slice.iter().filter(|c| c.space).count();
            let justify = if align == TextAlign::Justify && spaces > 0 {
                extra / spaces as f32
            } else {
                0.0
            };
            let index = out.boxes.len() as u32;
            let mut cursor = indent + alignment;
            for c in slice {
                if let Some(a) = c.atom {
                    out.atoms.push(super::InlineBox {
                        at: c.range.start as u32,
                        node: c.node,
                        line: index,
                        rect: crate::layout::Rect {
                            x: cursor,
                            y: baseline - a.baseline,
                            width: a.size[0],
                            height: a.size[1],
                        },
                    });
                }
                let first = out.glyphs.len() as u32;
                out.glyphs.extend(c.glyphs.iter().map(|g| {
                    if c.upright {
                        Glyph {
                            vertical_cell: Some([cursor, c.advance]),
                            x: cursor + g.y,
                            y: center - g.x,
                            upright: true,
                            ..*g
                        }
                    } else {
                        Glyph {
                            vertical_cell: Some([cursor, c.advance]),
                            x: cursor + g.x,
                            y: baseline + g.y,
                            ..*g
                        }
                    }
                }));
                let next = cursor + c.advance + if c.space { justify } else { 0.0 };
                if let Some(run) = out.runs.last_mut().filter(|r| {
                    r.line == index && r.node == c.node && r.edge.is_none() && c.edge.is_none()
                }) {
                    run.x[1] = next;
                    run.glyphs[1] = out.glyphs.len() as u32;
                } else {
                    out.runs.push(Run {
                        edge: c.edge,
                        node: c.node,
                        line: index,
                        x: [cursor, next],
                        glyphs: [first, out.glyphs.len() as u32],
                    });
                }
                if let Some(mark) = marks.get(&c.node) {
                    super::emphasis::append(
                        &mut out.marks,
                        mark,
                        font_of(c.node),
                        if c.combined {
                            "\u{fffc}"
                        } else {
                            &base.string[c.range.clone()]
                        },
                        super::emphasis::Placement {
                            node: c.node,
                            line: index,
                            inline: [cursor, next],
                            baseline,
                        },
                    );
                }
                cursor = next;
            }
            out.boxes.push([top, height]);
            out.baselines.push(baseline);
            // Bidi runs are in visual order. Their first/last clusters need not be
            // the first/last bytes, especially around an atomic inline box.
            let bytes = &clusters[start..end];
            out.text.push(
                bytes
                    .iter()
                    .map(|c| c.range.start)
                    .min()
                    .unwrap_or(range.start)
                    ..bytes.iter().map(|c| c.range.end).max().unwrap_or(range.end),
            );
            top += height;
            if end >= clusters.len() {
                break;
            }
            start = end;
            while start < clusters.len() && clusters[start].space && clusters[start].collapse {
                start += 1;
            }
            if start == clusters.len() {
                break;
            }
        }
    }
    Ok(out)
}

fn upright_glyphs(
    text: &Text,
    first: Glyph,
    value: &str,
    font: &Font,
) -> Result<(Vec<Glyph>, f32, Vec<f32>), String> {
    let face = &text.faces[first.face as usize];
    let file = harfrust::FontRef::from_index(face.data.as_ref(), face.index)
        .map_err(|e| format!("vertical font: {e}"))?;
    let data = harfrust::ShaperData::new(&file);
    let mut instance = harfrust::ShaperInstance::default();
    instance.set_coords(
        &file,
        face.coords
            .iter()
            .map(|&c| harfrust::NormalizedCoord::from_bits(c)),
    );
    let shaper = data.shaper(&file).instance(Some(&instance)).build();
    let caps = super::caps::vertical(font, face, value);
    let value = caps.as_ref().map_or(value, |c| c.0.as_str());
    let original: Vec<_> = font
        .variants
        .iter()
        .flat_map(|v| v.iter())
        .chain(font.features.iter())
        .copied()
        .collect();
    let selected = caps.as_ref().map_or(&original, |c| &c.1);
    let mut features: Vec<_> = selected
        .iter()
        .map(|f| harfrust::Feature::new(harfrust::Tag::new(&f.tag), u32::from(f.value), ..))
        .collect();
    if first.position_shift.is_some() {
        features.extend(
            [b"subs", b"sups"].map(|tag| harfrust::Feature::new(harfrust::Tag::new(tag), 0, ..)),
        );
    }
    let contextual = file
        .gpos()
        .ok()
        .and_then(|p| p.feature_list().ok())
        .is_some_and(|f| {
            f.feature_records()
                .iter()
                .any(|r| r.feature_tag() == harfrust::Tag::new(b"vchw"))
        });
    if !features
        .iter()
        .any(|f| matches!(&f.tag.to_be_bytes(), b"vchw" | b"vhal"))
    {
        if contextual {
            features.push(harfrust::Feature::new(harfrust::Tag::new(b"vchw"), 1, ..));
        } else {
            features.extend(
                punctuation_trims(value)
                    .into_iter()
                    .map(|r| harfrust::Feature::new(harfrust::Tag::new(b"vhal"), 1, r)),
            );
        }
    }
    let mut buffer = harfrust::UnicodeBuffer::new();
    buffer.push_str(value);
    buffer.set_direction(harfrust::Direction::TopToBottom);
    buffer.guess_segment_properties();
    let shaped = shaper.shape(buffer, harfrust::ShapeOptions::new().features(&features));
    let scale = first.size / shaper.units_per_em() as f32;
    let mut cursor = 0.0;
    let glyphs = shaped
        .glyph_infos()
        .iter()
        .zip(shaped.glyph_positions())
        .map(|(g, p)| {
            let glyph = Glyph {
                id: g.glyph_id,
                x: p.x_offset as f32 * scale - first.position_shift.unwrap_or(0.0),
                y: cursor - p.y_offset as f32 * scale,
                cluster: first.cluster
                    + caps.as_ref().map_or(g.cluster, |c| {
                        c.2.get(g.cluster as usize)
                            .copied()
                            .unwrap_or(g.cluster as usize) as u32
                    }),
                ..first
            };
            cursor -= p.y_advance as f32 * scale;
            glyph
        })
        .collect();
    let spacing = font.spacing
        + if value.chars().all(char::is_whitespace) {
            font.word_spacing
        } else {
            0.0
        };
    let advances = shaped
        .glyph_positions()
        .iter()
        .map(|p| -p.y_advance as f32 * scale)
        .collect();
    Ok((glyphs, cursor + spacing as f32, advances))
}

/// CSS Text's adjacent fullwidth punctuation trimming. Use a font's vhal metrics,
/// never substitute halfwidth characters or scale their outlines.
fn punctuation_trims(value: &str) -> Vec<std::ops::Range<usize>> {
    let class = |c: char| {
        let wide = ('\u{3000}'..='\u{303f}').contains(&c)
            || CodePointMapData::<EastAsianWidth>::new().get(c) == EastAsianWidth::Fullwidth;
        match c {
            '‘' | '“' => 1,
            '’' | '”' | '、' | '。' | '，' | '．' | '：' | '；' => 2,
            '·' | '‧' | '・' | '　' => 3,
            _ if wide => match CodePointMapData::<GeneralCategory>::new().get(c) {
                GeneralCategory::OpenPunctuation => 1,
                GeneralCategory::ClosePunctuation => 2,
                _ => 0,
            },
            _ => 0,
        }
    };
    let chars: Vec<_> = value.char_indices().collect();
    chars
        .iter()
        .enumerate()
        .filter_map(|(i, &(start, c))| {
            let before = i.checked_sub(1).map(|n| chars[n].1);
            let after = chars.get(i + 1).map(|p| p.1);
            let trim = match class(c) {
                1 => before.is_some_and(|p| {
                    class(p) != 0
                        || CodePointMapData::<GeneralCategory>::new().get(p)
                            == GeneralCategory::OpenPunctuation
                }),
                2 => after.is_some_and(|p| {
                    matches!(class(p), 2 | 3)
                        || CodePointMapData::<GeneralCategory>::new().get(p)
                            == GeneralCategory::ClosePunctuation
                }),
                _ => false,
            };
            trim.then_some(start..start + c.len_utf8())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::{front::Inputs, scene::Scene, time::Tick};

    fn load(source: &str) -> Scene {
        Scene::load(
            "vertical.motion.tsx",
            &|_| Some(source.as_bytes().to_vec()),
            &Inputs::default(),
            crate::testing::fonts(),
        )
        .unwrap()
    }

    #[test]
    fn container_axes_and_inherited_text_are_restored_on_backward_seeks() {
        let source = indoc::indoc! {r#"
            export const composition = {width:320,height:260,fps:4,duration:3};
            export default function Scene() {
                const {seconds} = useCurrentTime();
                const writingMode = seconds < 1 ? "vertical-rl" : seconds < 2 ? "vertical-lr" : "horizontal-tb";
                return <View style={{writingMode,width:300,height:240,padding:10,display:"flex",gap:8}}>
                    <Text style={{width:80,height:120,fontSize:20,lineHeight:"28px"}}>春风<Span style={{padding:"2%"}}>山水</Span></Text>
                    <View style={{writingMode:"horizontal-tb",width:40,height:30}}/>
                    <svg viewBox="0 0 20 10" style={{height:30,flexShrink:0,alignSelf:"start"}}><rect width="20" height="10" fill="green"/></svg>
                </View>;
            }
        "#};
        let mut scene = load(source);
        for number in [0, 5, 9, 2, 11, 0] {
            let at = Tick::from_seconds(number as f64 / 4.0).unwrap();
            let frame = scene.frame(at).unwrap();
            let expected = load(source).frame(at).unwrap();
            assert_eq!(frame.layout.boxes, expected.layout.boxes, "frame {number}");
            assert_eq!(frame.items, expected.items, "frame {number}");
            let svg = frame.styles.iter().position(|s| s.svg.is_some()).unwrap();
            assert_eq!(frame.layout.boxes[svg].border.width, 60.0);
            assert_eq!(frame.layout.boxes[svg].border.height, 30.0);
        }
    }

    #[test]
    fn combined_text_occupies_one_cell_and_seeks_restore_glyphs() {
        let source = indoc::indoc! {r#"
            export const composition = {width:160,height:200,fps:4,duration:2};
            export default function Scene() {
                const {seconds} = useCurrentTime();
                return <Text style={{width:70,height:180,fontSize:20,lineHeight:"30px",writingMode:"vertical-rl"}}>第<Span style={{textCombineUpright:seconds<1?"all":"none",textEmphasis:"filled dot",letterSpacing:"4px"}}>123456</Span>回</Text>;
            }
        "#};
        let mut scene = load(source);
        for number in [0, 5, 2, 7, 0] {
            let at = Tick::from_seconds(number as f64 / 4.0).unwrap();
            let frame = scene.frame(at).unwrap();
            assert_eq!(frame.items, load(source).frame(at).unwrap().items);
            let lines = frame.layout.lines.iter().flatten().next().unwrap();
            if number < 4 {
                assert_eq!(lines.marks.len(), 1);
                assert!((lines.runs.last().unwrap().x[1] - 60.0).abs() < 0.01);
                assert_eq!(lines.glyphs.iter().filter(|g| g.scale_x < 1.0).count(), 6);
                assert!(lines.glyphs.iter().all(|g| g.upright));
            } else {
                assert_eq!(lines.marks.len(), 6);
                assert!(lines.glyphs.iter().all(|g| g.scale_x == 1.0));
            }
        }
    }

    #[test]
    fn inherited_combine_does_not_join_across_inline_box_boundaries() {
        let source = indoc::indoc! {r#"
            export const composition={width:120,height:160,fps:1,duration:1};
            export default function Scene(){return <Text style={{writingMode:"vertical-rl",height:150}}><Span style={{textCombineUpright:"all"}}>12<Span>34</Span></Span></Text>}
        "#};
        let mut scene = load(source);
        let frame = scene.frame(Tick::ZERO).unwrap();
        let lines = frame.layout.lines.iter().flatten().next().unwrap();
        assert!(lines.glyphs.iter().all(|g| !g.upright && g.scale_x == 1.0));
    }

    #[test]
    fn combine_is_inherited_but_has_no_effect_in_horizontal_text() {
        let source = indoc::indoc! {r#"
            export const composition={width:200,height:100,fps:1,duration:1};
            export default function Scene(){return <Text style={{textCombineUpright:"all"}}>12<Span>34</Span></Text>}
        "#};
        let mut scene = load(source);
        let frame = scene.frame(Tick::ZERO).unwrap();
        let mut plain = load(&source.replace("all", "none"));
        assert_eq!(frame.items, plain.frame(Tick::ZERO).unwrap().items);
        assert!(frame.styles[1].font.combine.is_some());
    }

    #[test]
    fn vertical_columns_wrap_by_height_and_preserve_upright_cjk() {
        let source = indoc::indoc! {r#"
            export const composition = { width: 160, height: 160, fps: 1, duration: 1 };
            export default function Scene() {
                return <Text style={{width:100,height:100,fontSize:20,lineHeight:"30px",writingMode:"vertical-rl"}}>天地玄黄宇宙洪荒</Text>;
            }
        "#};
        let mut scene = load(source);
        let frame = scene.frame(Tick::ZERO).unwrap();
        let lines = frame.layout.lines.iter().flatten().next().unwrap();
        assert_eq!(lines.boxes, [[0.0, 30.0], [30.0, 30.0]]);
        assert_eq!(
            lines
                .text
                .iter()
                .map(|r| &lines.string[r.clone()])
                .collect::<Vec<_>>(),
            ["天地玄黄宇", "宙洪荒"]
        );
        assert!(lines.glyphs.iter().all(|g| g.upright));
        assert!((lines.runs[0].x[1] - 100.0).abs() < 0.01);
        assert!((lines.runs[1].x[1] - 60.0).abs() < 0.01);
    }

    #[test]
    fn mixed_sizes_share_the_vertical_central_baseline() {
        for writing in ["vertical-rl", "vertical-lr"] {
            for orientation in ["mixed", "sideways"] {
                let source = indoc::indoc! {r#"
                    export const composition={width:120,height:120,fps:1,duration:1};
                    export default function Scene(){return <Text style={{width:100,height:100,fontSize:24,lineHeight:"36px",writingMode:"WRITING",textOrientation:"ORIENTATION"}}>Ag<Span style={{fontSize:16}}>pq</Span></Text>}
                "#}.replace("WRITING", writing).replace("ORIENTATION", orientation);
                let frame = load(&source).frame(Tick::ZERO).unwrap();
                let lines = frame.layout.lines.iter().flatten().next().unwrap();
                // Noto Sans central baselines: (26 - 7)/2 at 24px, (17 - 5)/2 at 16px.
                let expected = if orientation == "mixed" { -3.5 } else { 0.0 };
                assert_eq!(lines.glyphs[2].y - lines.glyphs[0].y, expected);
                if orientation == "mixed" {
                    assert_eq!(lines.boxes[0][1], 36.0);
                }
            }
        }
    }

    #[test]
    fn vertical_logical_sizes_cascade_and_seeks_restore_span_styles() {
        let source = indoc::indoc! {r#"
            export const composition = { width: 200, height: 200, fps: 4, duration: 2 };
            export default function Scene() {
                const { seconds } = useCurrentTime();
                return <Text style={{width:70,inlineSize:120,blockSize:90,paddingInline:"4px 6px",paddingBlock:"8px 10px",fontSize:20,lineHeight:"30px",writingMode:seconds<1?"vertical-rl":"vertical-lr"}}>
                    春<Span style={{color:"red",textOrientation:"upright"}}>ABC</Span>山水
                </Text>;
            }
        "#};
        let mut scene = load(source);
        for number in [0, 5, 2, 7, 0] {
            let at = Tick::from_seconds(number as f64 / 4.0).unwrap();
            let frame = scene.frame(at).unwrap();
            assert_eq!(frame.items, load(source).frame(at).unwrap().items);
            let root = frame.layout.boxes[0];
            assert_eq!([root.border.width, root.border.height], [90.0, 120.0]);
            assert_eq!([root.content.width, root.content.height], [72.0, 110.0]);
            assert_eq!(root.content.y - root.border.y, 4.0);
            assert_eq!(
                root.content.x - root.border.x,
                if number < 4 { 10.0 } else { 8.0 }
            );
        }
    }

    #[test]
    fn vertical_punctuation_keeps_contextual_spacing() {
        let source = indoc::indoc! {r#"
            export const composition = {width:160,height:160,fps:1,duration:1};
            export default function Scene() {
                return <Text style={{width:60,height:110,fontSize:20,lineHeight:"30px",writingMode:"vertical-rl"}}>，（山水）。</Text>;
            }
        "#};
        let mut scene = load(source);
        let frame = scene.frame(Tick::ZERO).unwrap();
        let lines = frame.layout.lines.iter().flatten().next().unwrap();
        assert_eq!(lines.boxes.len(), 1);
        assert!(
            (lines.runs[0].x[1] - 100.0).abs() < 0.01,
            "{:?}",
            lines.runs
        );
        assert_eq!(super::punctuation_trims("，（山水）。"), [3..6, 12..15]);
    }
}
