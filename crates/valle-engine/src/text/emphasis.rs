//! Emphasis marks are placed per grapheme. Built-in shapes are synthesized; custom
//! marks use scene fonts at half size with the ruby feature.
use std::borrow::Cow;
use std::collections::HashMap;

use icu_properties::{CodePointMapData, props::GeneralCategory as G};
use unicode_segmentation::UnicodeSegmentation;

use super::{Content, Lines, Piece, Text};
use crate::paint::Matrix;
use crate::path::{Path, Segment};
use crate::style::{Feature, Font, TextAlign, WritingMode};

#[derive(Clone, Debug, PartialEq)]
pub struct Mark {
    pub node: u32,
    pub line: u32,
    pub inline: [f32; 2],
    /// Logical inline/block coordinates, like the paragraph's glyphs.
    pub path: Path,
}

pub(super) struct Shape {
    path: Path,
    size: [f32; 2],
    metrics: [f32; 3],
}

pub(super) fn shapes(text: &mut Text, content: &Content) -> Result<HashMap<u32, Shape>, String> {
    let mut out = HashMap::new();
    for piece in &content.pieces {
        let font = piece.font;
        let vertical = font.writing != WritingMode::Horizontal;
        let Some(mark) = font.emphasis.text(vertical) else {
            continue;
        };
        if out.contains_key(&piece.node) {
            continue;
        }
        let metrics = text.metrics(font)?;
        let mut font = font.clone();
        font.size *= 0.5;
        font.writing = WritingMode::Horizontal;
        font.emphasis = crate::style::emphasis::Mark::None;
        font.align = TextAlign::Left;
        font.align_last = None;
        font.indent = None;
        font.spacing = 0.0;
        font.word_spacing = 0.0;
        font.wrap = false;
        font.features = font
            .features
            .iter()
            .copied()
            .chain([Feature {
                tag: *b"ruby",
                value: 1,
            }])
            .collect();
        let source = Content {
            font: &font,
            clamp: None,
            pieces: vec![Piece {
                edge: None,
                text: Cow::Borrowed(mark),
                node: 0,
                font: &font,
            }],
        };
        let mut path = if matches!(piece.font.emphasis, crate::style::emphasis::Mark::Text(_)) {
            custom(text, &source, font.size as f32)?
        } else {
            symbol(mark, font.size as f32)
        };
        let [x, y, w, h] = crate::svg::bounds(&path).map(|v| v as f32);
        path = path.moved([-x - w / 2.0, -y - h / 2.0]);
        out.insert(
            piece.node,
            Shape {
                path,
                size: [w, h],
                metrics,
            },
        );
    }
    Ok(out)
}

fn custom(text: &mut Text, source: &Content, size: f32) -> Result<Path, String> {
    let glyphs = text.lines(source, f32::MAX)?;
    let mut path = Path::default();
    for g in &glyphs.glyphs {
        let (em, contours) = crate::slug::contours(&text.faces()[g.face as usize], g.id)?;
        let scale = size / em;
        let point = |[x, y]: [f32; 2]| [g.x + x * scale, g.y - y * scale];
        for contour in contours {
            if let Some(c) = contour.first() {
                path.0.push(Segment::Move(point(c[0])));
            }
            for [_, c, p] in contour {
                path.0.push(Segment::Quad(point(c), point(p)));
            }
            path.0.push(Segment::Close);
        }
    }
    Ok(path)
}

/// CSS permits synthesized marks, so a Latin font need not contain CJK symbols.
fn symbol(mark: &str, em: f32) -> Path {
    let dot = matches!(mark, "•" | "◦");
    let r = em * if dot { 0.125 } else { 0.375 };
    let triangle = matches!(mark, "▲" | "△");
    let sesame = matches!(mark, "﹅" | "﹆");
    let shape = |r: f32| {
        if triangle {
            Path(vec![
                Segment::Move([0.0, -r]),
                Segment::Line([r, r]),
                Segment::Line([-r, r]),
                Segment::Close,
            ])
        } else if sesame {
            crate::svg::ellipse([0.0, 0.0], [r * 0.5, r])
                .unwrap_or_default()
                .mapped(|p| Matrix::rotate(30.0).apply(p))
        } else {
            crate::svg::ellipse([0.0, 0.0], [r, r]).unwrap_or_default()
        }
    };
    let mut out = shape(r);
    if matches!(mark, "◦" | "○" | "◎" | "△" | "﹆" | "◉") {
        let inner = (r - em * 0.045).max(0.0);
        out.0.extend(shape(inner).mapped(|[x, y]| [-x, y]).0);
    }
    if matches!(mark, "◎" | "◉") {
        out.0.extend(shape(r * 0.5).0);
        if mark == "◎" {
            out.0.extend(
                shape((r * 0.5 - em * 0.045).max(0.0))
                    .mapped(|[x, y]| [-x, y])
                    .0,
            );
        }
    }
    out
}

/// Additional leading needed to keep annotations clear of adjacent lines. First/last
/// annotations can overflow the block; explicit roomy line heights absorb the marks.
pub(super) fn leading(shape: &Shape, font: &Font, extent: [f32; 2]) -> [f32; 2] {
    let vertical = font.writing != WritingMode::Horizontal;
    let under = if vertical {
        font.emphasis_position.left
    } else {
        font.emphasis_position.under
    };
    let axis = usize::from(under);
    let height = shape.size[usize::from(!vertical)].ceil();
    let mut out = [0.0; 2];
    out[axis] = (shape.metrics[axis].round() + height + 2.0 - extent[axis]).max(0.0);
    out
}

pub(super) struct Placement {
    pub node: u32,
    pub line: u32,
    pub inline: [f32; 2],
    pub baseline: f32,
}

pub(super) fn append(
    out: &mut Vec<Mark>,
    shape: &Shape,
    font: &Font,
    value: &str,
    Placement {
        node,
        line,
        inline,
        baseline,
    }: Placement,
) {
    let vertical = font.writing != WritingMode::Horizontal;
    let under = if vertical {
        font.emphasis_position.left
    } else {
        font.emphasis_position.under
    };
    let graphemes: Vec<_> = value.graphemes(true).collect();
    let width = (inline[1] - inline[0]) / graphemes.len().max(1) as f32;
    for (k, value) in graphemes.iter().enumerate() {
        if !eligible(value) {
            continue;
        }
        let start = inline[0] + k as f32 * width;
        let center = start + width / 2.0;
        let block = if vertical {
            let central = (shape.metrics[0].round() - shape.metrics[1].round()) / 2.0;
            baseline - central + font.size as f32 * 0.75 * if under { 1.0 } else { -1.0 }
        } else if under {
            baseline + shape.metrics[1].round() + shape.size[1] / 2.0
        } else {
            baseline - shape.metrics[0].round() - shape.size[1] / 2.0
        };
        let rotation = if vertical {
            Matrix::rotate(-90.0)
        } else {
            Matrix::IDENTITY
        };
        let matrix = Matrix::translate(center.into(), block.into()).times(rotation);
        out.push(Mark {
            node,
            line,
            inline: [start, start + width],
            path: shape.path.mapped(|p| matrix.apply(p)),
        });
    }
}

pub(super) fn horizontal(lines: &mut Lines, content: &Content, shapes: &HashMap<u32, Shape>) {
    for run in &lines.runs {
        let Some(shape) = shapes.get(&run.node) else {
            continue;
        };
        let font = content
            .pieces
            .iter()
            .find(|p| p.node == run.node)
            .unwrap()
            .font;
        let glyphs = &lines.glyphs[run.glyphs[0] as usize..run.glyphs[1] as usize];
        let groups: Vec<_> = glyphs.chunk_by(|a, b| a.cluster == b.cluster).collect();
        for (k, group) in groups.iter().enumerate() {
            let start = group[0].cluster as usize;
            let end = lines
                .glyphs
                .iter()
                .filter(|g| g.cluster as usize > start)
                .map(|g| g.cluster as usize)
                .min()
                .unwrap_or(lines.string.len());
            let x0 = group[0].x;
            let x1 = groups.get(k + 1).map_or(run.x[1], |g| g[0].x);
            append(
                &mut lines.marks,
                shape,
                font,
                &lines.string[start..end],
                Placement {
                    node: run.node,
                    line: run.line,
                    inline: [x0, x1],
                    baseline: lines.baselines[run.line as usize],
                },
            );
        }
    }
}

pub(super) fn eligible(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(c) = chars.next() else {
        return false;
    };
    // A space with a combining mark is a typographic unit, not a word separator.
    if c.is_whitespace() && chars.next().is_some() {
        return true;
    }
    if matches!(
        c,
        '#' | '%'
            | '‰'
            | '‱'
            | '٪'
            | '؉'
            | '؊'
            | '&'
            | '⁊'
            | '@'
            | '§'
            | '¶'
            | '⁋'
            | '⁓'
            | '〽'
            | '＃'
            | '％'
            | '＆'
            | '＠'
            | '﹟'
            | '﹠'
            | '﹪'
            | '﹫'
    ) {
        return true;
    }
    !matches!(
        CodePointMapData::<G>::new().get(c),
        G::SpaceSeparator
            | G::LineSeparator
            | G::ParagraphSeparator
            | G::Control
            | G::Format
            | G::Unassigned
            | G::ConnectorPunctuation
            | G::DashPunctuation
            | G::OpenPunctuation
            | G::ClosePunctuation
            | G::InitialPunctuation
            | G::FinalPunctuation
            | G::OtherPunctuation
    )
}
