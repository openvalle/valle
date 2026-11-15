//! SVG character positioning after shaping with the scene's fonts. The result is ordinary
//! vector geometry, so text shares SVG paint servers, strokes, masks and transforms.
mod decoration;
mod stretch;

use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

use kurbo::{ParamCurve, ParamCurveArclen, ParamCurveDeriv, PathSeg};

use super::{Element, Length, Shape, length};
use crate::paint::Matrix;
use crate::path::{Path, Segment};
use crate::slug;
use crate::style::{Collapse, Style, TextAlign};
use crate::template::{Tag, Template, V};
use crate::text::{Content, Piece, Text};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Baseline {
    #[default]
    Auto,
    Alphabetic,
    Middle,
    Central,
    Hanging,
    Mathematical,
    Before,
    After,
}

impl Baseline {
    pub fn parse(value: &str) -> Result<Self, String> {
        Ok(match value.trim() {
            "auto" => Self::Auto,
            "alphabetic" => Self::Alphabetic,
            "middle" => Self::Middle,
            "central" => Self::Central,
            "hanging" => Self::Hanging,
            "mathematical" => Self::Mathematical,
            "text-before-edge" | "text-top" => Self::Before,
            "text-after-edge" | "text-bottom" | "ideographic" => Self::After,
            other => return Err(format!("dominant-baseline: {other} is not supported")),
        })
    }
}

/// A non-inherited baseline shift. Each text ancestor contributes its own shift once.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Shift<T> {
    #[default]
    Baseline,
    Sub,
    Super,
    Length(T),
}

impl<T> Shift<T> {
    pub fn map<U>(&self, f: impl FnOnce(&T) -> U) -> Shift<U> {
        match self {
            Self::Baseline => Shift::Baseline,
            Self::Sub => Shift::Sub,
            Self::Super => Shift::Super,
            Self::Length(v) => Shift::Length(f(v)),
        }
    }
}

/// Decoration outlines keep their declaring paint and painting order separate from glyphs.
#[derive(Clone, Debug, PartialEq)]
pub struct Decoration {
    pub path: Path,
    pub by: usize,
    pub through: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Position {
    pub xy: [Vec<Length>; 2],
    pub delta: [Vec<Length>; 2],
    pub rotate: Vec<f64>,
    pub path: Option<Rc<str>>,
    pub offset: Length,
    pub length: Option<Length>,
    pub scale: bool,
    pub stretch: bool,
}

impl Position {
    pub fn parse(tag: Tag, attributes: &[(&str, &V)]) -> Result<Self, String> {
        let get = |key: &str| {
            attributes
                .iter()
                .find_map(|(k, v)| (*k == key).then_some(*v))
        };
        let list = |key: &str| -> Result<Vec<Length>, String> {
            get(key).map_or(Ok(Vec::new()), |v| {
                v.text()
                    .split(|c: char| c == ',' || c.is_ascii_whitespace())
                    .filter(|p| !p.is_empty())
                    .map(|p| length(&V::Str(p.into())).map_err(|e| format!("{key}: {e}")))
                    .collect()
            })
        };
        let word = |key: &str| get(key).map(|v| v.text().into_owned());
        let mut p = Position {
            xy: [list("x")?, list("y")?],
            delta: [list("dx")?, list("dy")?],
            rotate: word("rotate").map_or(Ok(Vec::new()), |v| {
                crate::path::numbers(&v)
                    .map(|values| values.into_iter().map(f64::from).collect())
                    .ok_or("rotate: angles are numbers in degrees")
            })?,
            length: get("textLength")
                .map(length)
                .transpose()?
                .map(|l| l.nonnegative("textLength"))
                .transpose()?,
            scale: match word("lengthAdjust").as_deref() {
                None | Some("spacing") => false,
                Some("spacingAndGlyphs") => true,
                Some(s) => {
                    return Err(format!(
                        "lengthAdjust: {s} is not spacing or spacingAndGlyphs"
                    ));
                }
            },
            ..Self::default()
        };
        if tag == Tag::TextPath {
            let href = word("href")
                .or_else(|| word("xlinkHref"))
                .unwrap_or_default();
            p.path = Some(
                href.strip_prefix('#')
                    .filter(|s| !s.is_empty())
                    .ok_or("textPath needs href=\"#id\" naming a path in this SVG")?
                    .into(),
            );
            p.stretch = word("method").as_deref() == Some("stretch");
            p.offset = get("startOffset")
                .map(length)
                .transpose()?
                .unwrap_or_default();
            for (key, accepted) in [
                ("method", &["align", "stretch"][..]),
                ("spacing", &["auto", "exact"][..]),
                ("side", &["left"][..]),
            ] {
                if let Some(v) = word(key)
                    && !accepted.contains(&v.as_str())
                {
                    return Err(format!(
                        "textPath {key}: {v} is not supported; use {}",
                        accepted[0]
                    ));
                }
            }
        }
        Ok(p)
    }
}

struct Leaf {
    node: usize,
    text: String,
    ancestors: Vec<usize>,
}

struct Placed {
    node: usize,
    character: usize,
    path: Path,
    at: [f64; 2],
    advance: f64,
    cell: [f64; 2],
    baselines: Vec<(usize, f64)>,
    angle: f64,
    scale: f64,
    track: Option<usize>,
}

struct Track {
    segments: Vec<(PathSeg, f64)>,
    length: f64,
    unit_scale: f64,
    stretch: bool,
}

impl Track {
    fn new(path: &Path, authored: Option<f64>) -> Self {
        let segments: Vec<_> = super::bez(path, 1.0)
            .segments()
            .map(|s| (s, s.arclen(0.001)))
            .collect();
        let length = segments.iter().map(|(_, l)| l).sum();
        let unit_scale = authored.map_or(1.0, |n| length / n);
        Self {
            segments,
            length,
            unit_scale,
            stretch: false,
        }
    }

    fn offset(&self, value: Length) -> f64 {
        value.units * self.unit_scale + value.percent / 100.0 * self.length
    }

    fn point(&self, mut distance: f64) -> Option<([f64; 2], f64)> {
        if distance < 0.0 || distance > self.length {
            return None;
        }
        for (segment, length) in &self.segments {
            if *length == 0.0 {
                continue;
            }
            if distance <= *length {
                let t = segment.inv_arclen(distance, 0.001);
                let p = segment.eval(t);
                let v = match segment {
                    PathSeg::Line(s) => s.deriv().eval(t),
                    PathSeg::Quad(s) => s.deriv().eval(t),
                    PathSeg::Cubic(s) => s.deriv().eval(t),
                };
                return Some(([p.x, p.y], v.y.atan2(v.x).to_degrees()));
            }
            distance -= length;
        }
        None
    }
}

/// Shape each text subtree, and separately each use instance's inherited fonts and positions.
/// SVG images call this after font loading; inline SVG calls it after settling frame values.
pub fn shape(
    template: &Template,
    styles: &mut [Style],
    text: &mut Text,
    viewport: impl Fn(usize) -> [f64; 2],
) -> Result<(), String> {
    let roots: Vec<_> = template
        .nodes
        .iter()
        .enumerate()
        .filter_map(|(i, n)| {
            matches!(n.kind, crate::template::Kind::Svg { tag: Tag::Text, .. }).then_some(i)
        })
        .collect();
    if roots.is_empty() {
        return Ok(());
    }
    // Instances are already styled independently. Temporarily overlay each copy to reuse the
    // same shaping path without inheriting typography from the referenced source element.
    for i in 0..styles.len() {
        if let Some(instance) = styles[i].instance.clone() {
            let mut copied = styles.to_vec();
            for (k, style) in instance.styles.iter().enumerate() {
                copied[instance.root + k] = style.clone();
            }
            for &root in &roots {
                if (instance.root..instance.root + instance.styles.len()).contains(&root) {
                    let size = units(template, &copied, root, &viewport);
                    subtree(template, &mut copied, text, root, size)?;
                }
            }
            let mut instance = (*instance).clone();
            let end = instance.root + instance.styles.len();
            instance
                .styles
                .clone_from_slice(&copied[instance.root..end]);
            styles[i].instance = Some(Rc::new(instance));
        }
    }
    for root in roots {
        subtree(
            template,
            styles,
            text,
            root,
            units(template, styles, root, &viewport),
        )?;
    }
    Ok(())
}

fn units(
    template: &Template,
    styles: &[Style],
    root: usize,
    viewport: &impl Fn(usize) -> [f64; 2],
) -> [f64; 2] {
    let mut at = root;
    while let Some((i, _)) = template
        .nodes
        .iter()
        .enumerate()
        .find(|(_, n)| n.children.contains(&(at as u32)))
    {
        if let Some(c) = styles[i].svg.as_deref() {
            match &c.element {
                Element::Svg { view_box, .. } => {
                    if let Some([_, _, w, h]) = view_box {
                        return [*w, *h];
                    }
                    let nested = template.nodes.iter().any(|n| {
                        n.children.contains(&(i as u32))
                            && matches!(n.kind, crate::template::Kind::Svg { .. })
                    });
                    return if nested {
                        c.element
                            .viewport(units(template, styles, i, viewport))
                            .map_or([0.0; 2], |v| v.units)
                    } else {
                        viewport(i)
                    };
                }
                Element::Symbol { view_box, .. } => {
                    return view_box.map_or_else(|| viewport(i), |[_, _, w, h]| [w, h]);
                }
                Element::Pattern(p) => {
                    let mut pattern = p;
                    let mut seen = vec![i];
                    let mut view_box = p.view_box;
                    let mut content_user = p.content_user_space;
                    while let Some(href) = &pattern.href {
                        let Some((n, c)) = styles.iter().enumerate().find_map(|(n, s)| {
                            s.svg
                                .as_deref()
                                .filter(|c| c.id.as_deref() == Some(href))
                                .map(|c| (n, c))
                        }) else {
                            break;
                        };
                        if seen.contains(&n) {
                            break;
                        }
                        seen.push(n);
                        let Element::Pattern(p) = &c.element else {
                            break;
                        };
                        view_box = view_box.or(p.view_box);
                        content_user = content_user.or(p.content_user_space);
                        pattern = p;
                    }
                    if let Some([_, _, w, h]) = view_box {
                        return [w, h];
                    }
                    if content_user == Some(false) {
                        return [1.0; 2];
                    }
                }
                _ => {}
            }
        }
        at = i;
    }
    viewport(root)
}

fn subtree(
    template: &Template,
    styles: &mut [Style],
    text: &mut Text,
    root: usize,
    size: [f64; 2],
) -> Result<(), String> {
    struct Collected {
        leaves: Vec<Leaf>,
        ranges: HashMap<usize, Range<usize>>,
        count: usize,
        space: bool,
    }
    fn collect(
        template: &Template,
        styles: &[Style],
        node: usize,
        ancestors: &mut Vec<usize>,
        out: &mut Collected,
    ) {
        let start = out.count;
        if styles[node].layout.display == taffy::Display::None {
            return;
        }
        let Some(c) = styles[node].svg.as_deref() else {
            return;
        };
        match &c.element {
            Element::TextRun(value) => {
                let collapse = styles[node].font.collapse == Collapse::Collapse;
                let mut text = String::new();
                for c in value.chars() {
                    if collapse && matches!(c, '\n' | '\r') {
                        continue;
                    }
                    let c = if matches!(c, '\n' | '\r' | '\t') {
                        ' '
                    } else {
                        c
                    };
                    if collapse && c == ' ' && out.space {
                        continue;
                    }
                    out.space = c == ' ';
                    text.push(c);
                    out.count += 1;
                }
                out.leaves.push(Leaf {
                    node,
                    text,
                    ancestors: ancestors.clone(),
                });
            }
            Element::Text(_) => {
                ancestors.push(node);
                for &child in &template.nodes[node].children {
                    collect(template, styles, child as usize, ancestors, out);
                }
                ancestors.pop();
            }
            _ => {}
        }
        out.ranges.insert(node, start..out.count);
    }
    let mut collected = Collected {
        leaves: Vec::new(),
        ranges: HashMap::new(),
        count: 0,
        space: true,
    };
    collect(template, styles, root, &mut Vec::new(), &mut collected);
    let Collected {
        mut leaves, ranges, ..
    } = collected;
    if leaves.is_empty() {
        return Ok(());
    }
    // Collapsed trailing space contributes no advance. Leave character indices before it intact.
    if let Some(last) = leaves.last_mut()
        && styles[last.node].font.collapse == Collapse::Collapse
    {
        last.text.truncate(last.text.trim_end_matches(' ').len());
    }
    let font = |font: &crate::style::Font| {
        let mut f = font.clone();
        f.wrap = false;
        f.wrap_style = crate::style::WrapStyle::Auto;
        f.align = TextAlign::Left;
        f.align_last = None;
        f.indent = None;
        f.collapse = Collapse::Preserve;
        f
    };
    let base = font(&styles[root].font);
    let vertical = base.writing != crate::style::WritingMode::Horizontal;
    let fonts: Vec<_> = leaves
        .iter()
        .map(|leaf| font(&styles[leaf.node].font))
        .collect();
    let content = Content {
        scopes: Vec::new(),
        font: &base,
        clamp: None,
        pieces: leaves
            .iter()
            .zip(&fonts)
            .map(|(leaf, font)| Piece {
                atom: None,
                edge: None,
                text: Cow::Borrowed(&leaf.text),
                node: leaf.node as u32,
                font,
            })
            .collect(),
    };
    let lines = text.lines(&content, f32::MAX)?;
    let first = lines.baselines.first().copied().unwrap_or(0.0);
    let mut char_at = vec![0; lines.string.len() + 1];
    for (index, (byte, c)) in lines.string.char_indices().enumerate() {
        char_at[byte..byte + c.len_utf8()].fill(index);
    }
    let mut tracks = HashMap::new();
    for &n in ranges.keys() {
        if let Some(Element::Text(p)) = styles[n].svg.as_deref().map(|c| &c.element)
            && let Some(id) = &p.path
        {
            let target = styles
                .iter()
                .find_map(|s| s.svg.as_deref().filter(|c| c.id.as_ref() == Some(id)));
            let path = target.and_then(|c| match &c.element {
                Element::Shape(s) => super::path(s, size).map(|path| {
                    c.transform
                        .map_or(path.clone(), |m| path.mapped(|p| m.apply(p)))
                }),
                _ => None,
            });
            // A missing reference draws no text, as it draws no referenced shape.
            tracks.insert(
                n,
                Track {
                    stretch: p.stretch,
                    ..Track::new(
                        &path.unwrap_or_default(),
                        target.and_then(|c| c.path_length),
                    )
                },
            );
        }
    }
    // SVG positions address logical characters, while the shaper returns visual glyph
    // order. Partition at absolute positions before placing bidi runs, so a root x/y
    // applies to its entire chunk even when its first logical character draws last.
    #[derive(Clone, Default)]
    struct Address {
        node: usize,
        absolute: [Option<f64>; 2],
        delta: [f64; 2],
        angle: f64,
        track: Option<usize>,
    }
    let mut addresses = Vec::new();
    for leaf in &leaves {
        for _ in leaf.text.chars() {
            let character = addresses.len();
            let mut address = Address {
                node: leaf.node,
                ..Address::default()
            };
            for &ancestor in &leaf.ancestors {
                let Element::Text(p) = &styles[ancestor].svg.as_deref().unwrap().element else {
                    continue;
                };
                let index = character.saturating_sub(ranges[&ancestor].start);
                for (axis, &basis) in size.iter().enumerate() {
                    if let Some(v) = p.xy[axis].get(index) {
                        address.absolute[axis] = Some(v.resolve(basis));
                    }
                    if let Some(v) = p.delta[axis].get(index) {
                        address.delta[axis] = v.resolve(basis);
                    }
                }
                if let Some(v) = p.rotate.get(index).or(p.rotate.last()) {
                    address.angle = *v;
                }
                if p.path.is_some() {
                    address.track = Some(ancestor);
                }
            }
            if vertical {
                address.absolute = [address.absolute[1], address.absolute[0].map(|v| -v)];
                address.delta = [address.delta[1], -address.delta[0]];
            }
            addresses.push(address);
        }
    }
    let starts: Vec<_> = addresses
        .iter()
        .enumerate()
        .filter_map(|(i, a)| {
            (i == 0 || a.absolute.iter().any(Option::is_some) || a.track != addresses[i - 1].track)
                .then_some(i)
        })
        .collect();
    let mut glyphs = Vec::new();
    for run in &lines.runs {
        let row = &lines.glyphs[run.glyphs[0] as usize..run.glyphs[1] as usize];
        for (k, glyph) in row.iter().enumerate() {
            let character = char_at[glyph.cluster as usize];
            let chunk = starts.partition_point(|s| *s <= character) - 1;
            let advance = f64::from(glyph.vertical_cell.map_or_else(
                || row.get(k + 1).map_or(run.x[1], |g| g.x) - glyph.x,
                |c| c[1],
            ));
            glyphs.push((chunk, glyph, character, advance));
        }
    }
    glyphs.sort_by_key(|g| g.0);
    let mut placed = Vec::<Placed>::new();
    let mut chunks = Vec::<(usize, f64)>::new();
    let mut cursor = [0.0; 2];
    let mut previous_track = None;
    let mut remaining = &glyphs[..];
    let mut displacements = vec![[0.0; 2]; addresses.len()];
    for (chunk, &start) in starts.iter().enumerate() {
        let (row, rest) = remaining.split_at(remaining.partition_point(|g| g.0 == chunk));
        remaining = rest;
        if row.is_empty() {
            continue;
        }
        let end = starts.get(chunk + 1).copied().unwrap_or(addresses.len());
        let first_address = &addresses[start];
        let track = first_address.track;
        let style = &styles[first_address.node];
        let rtl = style.font.rtl;
        let sign = if rtl { -1.0 } else { 1.0 };
        let min = row
            .iter()
            .map(|g| f64::from(g.1.vertical_cell.map_or(g.1.x, |c| c[0])))
            .fold(f64::INFINITY, f64::min);
        let max = row
            .iter()
            .map(|g| f64::from(g.1.vertical_cell.map_or(g.1.x, |c| c[0])) + g.3)
            .fold(f64::NEG_INFINITY, f64::max);
        let width = max - min;
        if track != previous_track
            && let Some(n) = track
        {
            let Element::Text(p) = &styles[n].svg.as_deref().unwrap().element else {
                unreachable!()
            };
            cursor = [tracks[&n].offset(p.offset), 0.0];
        }
        for (axis, c) in cursor.iter_mut().enumerate() {
            if let Some(value) = first_address.absolute[axis] {
                *c = value;
            }
        }
        let offset = [cursor[0] - min - if rtl { width } else { 0.0 }, cursor[1]];
        let mut delta = [0.0; 2];
        for (a, position) in addresses[start..end]
            .iter()
            .zip(&mut displacements[start..end])
        {
            for (d, add) in delta.iter_mut().zip(a.delta) {
                *d += add;
            }
            *position = delta;
        }
        chunks.push((placed.len(), style.text_anchor * sign));
        cursor[0] += sign * width + delta[0];
        cursor[1] += delta[1];
        for &(_, glyph, character, advance) in row {
            let node = glyph.node as usize;
            let leaf = leaves
                .iter()
                .find(|leaf| leaf.node == node)
                .expect("shaped leaf");
            let angle = addresses[character].angle;
            let shift = displacements[character];
            let font = &styles[node].font;
            let mut baselines = Vec::new();
            let mut baseline = 0.0;
            for (k, &ancestor) in leaf.ancestors.iter().enumerate() {
                let style = &styles[ancestor];
                let align = style.alignment_baseline.unwrap_or(style.dominant_baseline);
                let own = baseline_offset(text, &style.font, align)?;
                if k == 0 {
                    baseline = own;
                } else {
                    let parent = &styles[leaf.ancestors[k - 1]];
                    baseline += own
                        - baseline_offset(
                            text,
                            &parent.font,
                            parent
                                .alignment_baseline
                                .unwrap_or(parent.dominant_baseline),
                        )?;
                    baseline -= match &style.baseline_shift {
                        Shift::Baseline => 0.0,
                        Shift::Length(v) => f64::from(v.resolve(style.font.size as f32)),
                        Shift::Sub | Shift::Super => {
                            let [a, d, _] = text.metrics(&style.font)?.map(f64::from);
                            (a + d) / 2.0
                                * if matches!(style.baseline_shift, Shift::Sub) {
                                    -1.0
                                } else {
                                    1.0
                                }
                        }
                    };
                }
                baselines.push((ancestor, baseline));
            }
            let (em, contours) = slug::contours(&text.faces()[glyph.face as usize], glyph.id)?;
            let scale = glyph.size / em;
            let mut path = Path::default();
            let origin = glyph.vertical_cell.map_or(glyph.x, |c| c[0]);
            let point = |[x, y]: [f32; 2]| {
                let p = [x * scale, -y * scale];
                let p = if glyph.upright {
                    [p[1], -p[0] * glyph.scale_x]
                } else {
                    p
                };
                [p[0] + glyph.x - origin, p[1]]
            };
            for contour in contours {
                if let Some(curve) = contour.first() {
                    path.0.push(Segment::Move(point(curve[0])));
                }
                for [_, c, e] in contour {
                    path.0.push(Segment::Quad(point(c), point(e)));
                }
                path.0.push(Segment::Close);
            }
            let [ascent, descent, _] = text.metrics(font)?.map(|v| f64::from(v.round()));
            placed.push(Placed {
                node,
                character,
                path,
                at: [
                    f64::from(origin) + offset[0] + shift[0],
                    f64::from(glyph.y - first) + offset[1] + shift[1] + baseline,
                ],
                advance,
                cell: [ascent, descent],
                baselines,
                angle,
                scale: 1.0,
                track,
            });
        }
        previous_track = track;
    }
    // Apply textLength from inner spans outward. The spacing form changes positions only.
    let mut adjusting: Vec<_> = ranges.keys().copied().collect();
    adjusting.sort_unstable_by(|a, b| b.cmp(a));
    for n in adjusting {
        let Some(Element::Text(p)) = styles[n].svg.as_deref().map(|c| &c.element) else {
            continue;
        };
        let Some(length) = p.length else {
            continue;
        };
        let indices: Vec<_> = placed
            .iter()
            .enumerate()
            .filter_map(|(i, g)| ranges[&n].contains(&g.character).then_some(i))
            .collect();
        let (Some(&a), Some(&b)) = (indices.first(), indices.last()) else {
            continue;
        };
        let start = placed[a].at[0];
        let natural = placed[b].at[0] + placed[b].advance - start;
        if natural <= 0.0 {
            continue;
        }
        let width = length.resolve(size[usize::from(vertical)]);
        let factor = width / natural;
        let gap = (width - natural) / indices.len().saturating_sub(1).max(1) as f64;
        for (k, &i) in indices.iter().enumerate() {
            if p.scale {
                placed[i].at[0] = start + (placed[i].at[0] - start) * factor;
                placed[i].scale *= factor;
                placed[i].advance *= factor;
            } else {
                placed[i].at[0] += gap * k as f64;
            }
        }
        if styles[n].font.rtl {
            for &i in &indices {
                placed[i].at[0] -= width - natural;
            }
            let begin = chunks.iter().rev().find(|c| c.0 <= a).map_or(0, |c| c.0);
            for glyph in &mut placed[begin..a] {
                glyph.at[0] -= width - natural;
            }
        } else {
            let end = chunks
                .iter()
                .find(|c| c.0 > b)
                .map_or(placed.len(), |c| c.0);
            for glyph in &mut placed[b + 1..end] {
                glyph.at[0] += width - natural;
            }
        }
    }
    for k in 0..chunks.len() {
        let (start, anchor) = chunks[k];
        let end = chunks.get(k + 1).map_or(placed.len(), |c| c.0);
        if start == end {
            continue;
        }
        let width = placed[end - 1].at[0] + placed[end - 1].advance - placed[start].at[0];
        for g in &mut placed[start..end] {
            g.at[0] -= width * anchor;
        }
    }
    let mut paths: HashMap<usize, Path> = leaves
        .iter()
        .map(|leaf| (leaf.node, Path::default()))
        .collect();
    let mut cells: HashMap<usize, Vec<[f64; 4]>> = HashMap::new();
    let mut decorations = decoration::outlines(&placed, styles, text, &tracks)?;
    if vertical {
        for (node, decorations) in &mut decorations {
            if placed.iter().any(|g| g.node == *node && g.track.is_some()) {
                continue;
            }
            for decoration in decorations {
                decoration.path = decoration.path.mapped(|[x, y]| [-y, x]);
            }
        }
    }
    for g in placed {
        let matrix = if let Some(n) = g.track {
            let Some((point, tangent)) = tracks[&n].point(g.at[0] + g.advance / 2.0) else {
                continue;
            };
            Matrix::translate(point[0], point[1])
                .times(Matrix::rotate(tangent))
                .times(Matrix::translate(-g.advance / 2.0, g.at[1]))
                .times(Matrix::rotate(g.angle))
        } else {
            Matrix::translate(g.at[0], g.at[1]).times(Matrix::rotate(g.angle))
        };
        let matrix = matrix.times(Matrix::affine([g.scale, 0.0, 0.0, 1.0, 0.0, 0.0]));
        let matrix = if vertical && g.track.is_none() {
            Matrix::rotate(90.0).times(matrix)
        } else {
            matrix
        };
        let b = matrix.bounds(crate::layout::Rect {
            x: 0.0,
            y: -g.cell[0] as f32,
            width: (g.advance / g.scale.max(1e-12)) as f32,
            height: (g.cell[0] + g.cell[1]) as f32,
        });
        cells.entry(g.node).or_default().push([
            b.x.into(),
            b.y.into(),
            b.width.into(),
            b.height.into(),
        ]);
        let path = if let Some(n) = g.track.filter(|n| tracks[n].stretch) {
            let local = Matrix::translate(g.at[0], g.at[1])
                .times(Matrix::rotate(g.angle))
                .times(Matrix::affine([g.scale, 0.0, 0.0, 1.0, 0.0, 0.0]));
            let cell = super::rect(
                [
                    0.0,
                    -g.cell[0] as f32,
                    (g.advance / g.scale.max(1e-12)) as f32,
                    (g.cell[0] + g.cell[1]) as f32,
                ],
                [0.0; 2],
            );
            *cells.get_mut(&g.node).unwrap().last_mut().unwrap() =
                super::bounds(&tracks[&n].warp(&cell.mapped(|p| local.apply(p))));
            tracks[&n].warp(&g.path.mapped(|p| local.apply(p)))
        } else {
            g.path.mapped(|p| matrix.apply(p))
        };
        paths.get_mut(&g.node).unwrap().0.extend(path.0);
    }
    for (node, path) in paths {
        let c = Rc::make_mut(styles[node].svg.as_mut().unwrap());
        let bounds = cells
            .get(&node)
            .and_then(|boxes| {
                boxes.iter().copied().reduce(|a, b| {
                    let x = a[0].min(b[0]);
                    let y = a[1].min(b[1]);
                    [
                        x,
                        y,
                        (a[0] + a[2]).max(b[0] + b[2]) - x,
                        (a[1] + a[3]).max(b[1] + b[3]) - y,
                    ]
                })
            })
            .unwrap_or([0.0; 4]);
        c.element = Element::Shape(Shape::Text {
            path,
            bounds,
            decorations: decorations.remove(&node).unwrap_or_default(),
        });
    }
    Ok(())
}

fn baseline_offset(
    text: &mut Text,
    font: &crate::style::Font,
    baseline: Baseline,
) -> Result<f64, String> {
    let baseline = match baseline {
        Baseline::Auto
            if font.writing != crate::style::WritingMode::Horizontal
                && font.orientation != crate::style::Orientation::Sideways =>
        {
            Baseline::Central
        }
        Baseline::Auto => Baseline::Alphabetic,
        other => other,
    };
    Ok(match baseline {
        Baseline::Alphabetic => 0.0,
        Baseline::Middle => f64::from(text.x_height(font)?) / 2.0,
        other => {
            let [a, d, _] = text.metrics(font)?.map(f64::from);
            match other {
                Baseline::Central => (a - d) / 2.0,
                Baseline::Mathematical => a / 2.0,
                Baseline::Hanging => a * 0.8,
                Baseline::Before => a,
                Baseline::After => -d,
                _ => 0.0,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::front::Inputs;
    use crate::scene::Scene;
    use crate::time::Tick;

    #[test]
    fn stretch_text_paths_keep_straight_output_and_seek_consistency() {
        let source = |method: &str, path: &str| {
            format!(
                "export const composition = {{width:320,height:200,fps:4,duration:2}}; export default function Scene() {{const {{seconds}}=useCurrentTime();return <svg width=\"320\" height=\"200\" fontSize=\"30\"><defs><path id=\"p\" d=\"{path}\"/></defs><text><textPath href=\"#p\" method=\"{method}\" startOffset={{20+seconds*10}}>WAVES</textPath></text></svg>;}}"
            )
        };
        let load = |source: &str| {
            Scene::load(
                "stretch.motion.tsx",
                &|_| Some(source.as_bytes().to_vec()),
                &Inputs::default(),
                crate::testing::fonts(),
            )
            .unwrap()
        };
        let shape = |frame: &crate::scene::Frame| {
            frame
                .styles
                .iter()
                .filter_map(|s| match &s.svg.as_deref()?.element {
                    Element::Shape(Shape::Text { path, .. }) => {
                        Some(path.mapped(|p| p.map(|v| (v * 1000.).round() / 1000.)))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let straight = "M10 100L300 100";
        let a = load(&source("align", straight)).frame(Tick::ZERO).unwrap();
        let b = load(&source("stretch", straight))
            .frame(Tick::ZERO)
            .unwrap();
        assert_eq!(shape(&a), shape(&b));
        let curved = source("stretch", "M10 100Q150 -40 300 100");
        let mut scene = load(&curved);
        for seconds in [1., 0.25, 1.75, 0.] {
            let at = Tick::from_seconds(seconds).unwrap();
            assert_eq!(
                scene.frame(at).unwrap().items,
                load(&curved).frame(at).unwrap().items
            );
        }
    }

    #[test]
    fn vertical_svg_positions_cells_and_preserves_seek_results() {
        let source = indoc::indoc! {r#"
            export const composition = {width:320,height:240,fps:4,duration:2};
            export default function Scene() {
              const {seconds} = useCurrentTime();
              return <svg width="320" height="240" fontFamily="monospace" fontSize="20">
                <text x="80" y="20" writingMode="vertical-rl" style={{textOrientation:"upright"}}>ABC</text>
                <text x="140" y="20 70 120" writingMode="vertical-lr" style={{textOrientation:"upright"}}>ABC</text>
                <text x={200 + seconds * 10} y="120" writingMode="vertical-rl" textAnchor="middle" textLength="100" lengthAdjust="spacingAndGlyphs">ABC</text>
              </svg>;
            }
        "#};
        let load = || {
            Scene::load(
                "vertical.motion.tsx",
                &|_| Some(source.as_bytes().to_vec()),
                &Inputs::default(),
                crate::testing::fonts(),
            )
            .unwrap()
        };
        let mut scene = load();
        let frame = scene.frame(Tick::ZERO).unwrap();
        let cells: Vec<_> = frame
            .styles
            .iter()
            .filter_map(|s| match &s.svg.as_deref()?.element {
                Element::Shape(Shape::Text { bounds, .. }) => Some(*bounds),
                _ => None,
            })
            .collect();
        assert_eq!(cells.len(), 3);
        // The pinned Latin font uses a 1.362em fallback vertical advance.
        // Explicit y positions remain physical coordinates.
        assert!((cells[0][1] - 20.).abs() < 0.01, "{cells:?}");
        assert!((cells[0][3] - 81.72).abs() < 0.01, "{cells:?}");
        assert!((cells[1][1] - 20.).abs() < 0.01, "{cells:?}");
        assert!((cells[1][3] - 127.24).abs() < 0.01, "{cells:?}");
        assert!((cells[2][1] - 70.).abs() < 0.01, "{cells:?}");
        assert!((cells[2][3] - 100.).abs() < 0.01, "{cells:?}");
        for seconds in [1., 0.25, 1.75, 0.] {
            let at = Tick::from_seconds(seconds).unwrap();
            assert_eq!(
                scene.frame(at).unwrap().items,
                load().frame(at).unwrap().items
            );
        }
    }

    #[test]
    fn changing_svg_text_direction_keeps_seek_results_independent() {
        let source = indoc::indoc! {r#"
            export const composition = {width:320,height:120,fps:4,duration:2};
            export default function Scene() {
              const {seconds} = useCurrentTime();
              return <svg width="320" height="120" fontFamily="monospace" fontSize="20">
                <text x={180 + seconds * 10} y="40" direction={seconds < 1 ? "rtl" : "ltr"} unicodeBidi="bidi-override">AB<tspan x="260">CD</tspan></text>
              </svg>;
            }
        "#};
        let load = || {
            Scene::load(
                "bidi.motion.tsx",
                &|_| Some(source.as_bytes().to_vec()),
                &Inputs::default(),
                crate::testing::fonts(),
            )
            .unwrap()
        };
        let mut scene = load();
        for at in [1.0, 0.25, 1.75, 0.0] {
            let at = Tick::from_seconds(at).unwrap();
            assert_eq!(
                scene.frame(at).unwrap().items,
                load().frame(at).unwrap().items
            );
        }
    }

    #[test]
    fn bidi_chunks_keep_logical_positions_and_directional_anchors() {
        let paths = |markup: &str| {
            let source = format!(
                "export const composition = {{width:320,height:120,fps:1,duration:1}}; export default function Scene() {{return <svg width=\"320\" height=\"120\" fontFamily=\"monospace\" fontSize=\"20\">{markup}</svg>;}}"
            );
            let frame = Scene::load(
                "bidi.motion.tsx",
                &|_| Some(source.as_bytes().to_vec()),
                &Inputs::default(),
                crate::testing::fonts(),
            )
            .unwrap()
            .frame(Tick::ZERO)
            .unwrap();
            let mut joined = Path::default();
            for style in &*frame.styles {
                if let Some(Element::Shape(Shape::Text { path, .. })) =
                    style.svg.as_deref().map(|s| &s.element)
                {
                    joined
                        .0
                        .extend(path.mapped(|p| p.map(|v| (v * 1000.0).round() / 1000.0)).0);
                }
            }
            joined
        };
        for (actual, expected) in [
            (
                r#"<text x="100" y="40" direction="rtl" unicodeBidi="bidi-override">ABC</text>"#,
                r#"<text x="100" y="40" textAnchor="end">CBA</text>"#,
            ),
            (
                r#"<text x="100" y="40" direction="rtl">ABC</text>"#,
                r#"<text x="100" y="40" textAnchor="end">ABC</text>"#,
            ),
            (
                r#"<text x="100" y="40" direction="rtl" unicodeBidi="bidi-override" textAnchor="end">ABC</text>"#,
                r#"<text x="100" y="40">CBA</text>"#,
            ),
            (
                r#"<text x="100 120 140" y="40" direction="rtl" unicodeBidi="bidi-override">ABC</text>"#,
                r#"<text y="40" textAnchor="end"><tspan x="100">A</tspan><tspan x="120">B</tspan><tspan x="140">C</tspan></text>"#,
            ),
            (
                r#"<text x="100" y="40" direction="rtl" unicodeBidi="bidi-override">AB<tspan x="200">CD</tspan></text>"#,
                r#"<text x="100" y="40" textAnchor="end">BA<tspan x="200">DC</tspan></text>"#,
            ),
            (
                r#"<text x="100" y="40" direction="rtl" unicodeBidi="bidi-override" textLength="90" lengthAdjust="spacingAndGlyphs">ABC</text>"#,
                r#"<text x="100" y="40" textAnchor="end" textLength="90" lengthAdjust="spacingAndGlyphs">CBA</text>"#,
            ),
        ] {
            assert_eq!(paths(actual), paths(expected), "{actual}");
        }
    }

    #[test]
    fn alignment_baseline_places_the_requested_typographic_edge() {
        let source = indoc::indoc! {r#"
          export const composition = { width: 200, height: 100, fps: 1, duration: 1 };
          export default function Scene() { return <svg width={200} height={100}>
            <text x={10} y={40} fontSize={30}><tspan alignmentBaseline="text-before-edge" fontSize={20}>A</tspan><tspan alignmentBaseline="text-after-edge" fontSize={20}>B</tspan></text>
          </svg>; }
        "#};
        let mut scene = Scene::load(
            "alignment.motion.tsx",
            &|_| Some(source.as_bytes().to_vec()),
            &Inputs::default(),
            crate::testing::fonts(),
        )
        .unwrap();
        let frame = scene.frame(Tick::ZERO).unwrap();
        let bounds: Vec<_> = frame
            .styles
            .iter()
            .filter_map(|s| match &s.svg.as_deref()?.element {
                Element::Shape(Shape::Text { bounds, .. }) => Some(*bounds),
                _ => None,
            })
            .collect();
        assert_eq!(bounds.len(), 2);
        // Typographic cells snap ascent/descent; the selected edge stays within half a pixel.
        assert!((bounds[0][1] - 40.0).abs() < 0.5, "{bounds:?}");
        assert!(
            (bounds[1][1] + bounds[1][3] - 40.0).abs() < 0.5,
            "{bounds:?}"
        );
    }

    #[test]
    fn baseline_shifts_accumulate_without_moving_following_characters() {
        let source = indoc::indoc! {r#"
          export const composition = { width: 250, height: 100, fps: 4, duration: 2 };
          export default function Scene() { const { seconds } = useCurrentTime();
            return <svg width={250} height={100}><text x={10} y={60} fontSize={20}>A<tspan baselineShift={10 + seconds * 4}>A<tspan baselineShift="25%">A</tspan></tspan>A</text></svg>;
          }
        "#};
        let load = || {
            Scene::load(
                "baseline.motion.tsx",
                &|_| Some(source.as_bytes().to_vec()),
                &Inputs::default(),
                crate::testing::fonts(),
            )
            .unwrap()
        };
        let mut scene = load();
        let frame = scene.frame(Tick::ZERO).unwrap();
        let ys: Vec<_> = frame
            .styles
            .iter()
            .filter_map(|s| match &s.svg.as_deref()?.element {
                Element::Shape(Shape::Text { path, .. }) => Some(super::super::bounds(path)[1]),
                _ => None,
            })
            .collect();
        assert_eq!(ys.len(), 4);
        for (y, delta) in ys.iter().zip([0.0, -10.0, -15.0, 0.0]) {
            assert!((y - ys[0] - delta).abs() < 1e-5, "{ys:?}");
        }
        for k in [5, 2, 7, 0] {
            let at = Tick::from_seconds(k as f64 / 4.0).unwrap();
            assert_eq!(
                scene.frame(at).unwrap().items,
                load().frame(at).unwrap().items
            );
        }
    }

    #[test]
    fn decorations_keep_the_declaring_paint_and_leave_glyph_silhouettes_unchanged() {
        let source = indoc::indoc! {r#"
          export const composition = { width: 200, height: 100, fps: 1, duration: 1 };
          export default function Scene() { return <svg width={200} height={100}>
            <text id="owner" x={10} y={60} fill="blue" fontSize={30} textDecoration="underline"><tspan fill="red">A</tspan><tspan textDecoration="line-through">B</tspan></text>
          </svg>; }
        "#};
        let load = |source: &str| {
            Scene::load(
                "decoration.motion.tsx",
                &|_| Some(source.as_bytes().to_vec()),
                &Inputs::default(),
                crate::testing::fonts(),
            )
            .unwrap()
            .frame(Tick::ZERO)
            .unwrap()
        };
        let decorated = load(source);
        let plain = load(
            &source
                .replace("textDecoration=\"underline\"", "")
                .replace("textDecoration=\"line-through\"", ""),
        );
        let owner = decorated
            .styles
            .iter()
            .position(|s| {
                s.svg
                    .as_deref()
                    .is_some_and(|g| g.id.as_deref() == Some("owner"))
            })
            .unwrap();
        let mut count = 0;
        for (s, p) in decorated.styles.iter().zip(plain.styles.iter()) {
            if let Some(Element::Shape(Shape::Text {
                path, decorations, ..
            })) = s.svg.as_deref().map(|s| &s.element)
            {
                let Some(Element::Shape(Shape::Text { path: clean, .. })) =
                    p.svg.as_deref().map(|s| &s.element)
                else {
                    panic!("plain glyph");
                };
                assert_eq!(path, clean);
                assert!(
                    decorations
                        .iter()
                        .any(|d| d.by == owner && !d.through && !d.path.0.is_empty())
                );
                count += decorations.len();
            }
        }
        assert_eq!(count, 3);
    }

    #[test]
    fn use_text_shapes_with_the_instances_font() {
        let source = indoc::indoc! {r##"
          export const composition = { width: 200, height: 100, fps: 1, duration: 1 };
          export default function Scene() { return <svg width={200} height={100}>
            <defs><text id="label" x={10} y={45}>M</text></defs>
            <use href="#label" fontSize={40} fill="red" />
          </svg>; }
        "##};
        let mut scene = Scene::load(
            "use.motion.tsx",
            &|_| Some(source.as_bytes().to_vec()),
            &Inputs::default(),
            crate::testing::fonts(),
        )
        .unwrap();
        let frame = scene.frame(Tick::ZERO).unwrap();
        let copy = frame
            .styles
            .iter()
            .find_map(|s| s.instance.as_deref())
            .unwrap();
        assert_eq!(copy.styles[0].font.size, 40.0);
        let Element::Shape(Shape::Text { bounds, .. }) =
            &copy.styles[1].svg.as_deref().unwrap().element
        else {
            panic!("unshaped instance")
        };
        assert!((bounds[2] - 40.0 * 0.907).abs() < 0.001);
    }

    #[test]
    fn character_data_uses_its_text_elements_vector_effect() {
        let source = indoc::indoc! {r#"
          export const composition = { width: 100, height: 50, fps: 1, duration: 1 };
          export default function Scene() { return <svg width={100} height={50}>
            <text x={10} y={30} vectorEffect="non-scaling-stroke" stroke="black">A</text>
          </svg>; }
        "#};
        let mut scene = Scene::load(
            "pen.motion.tsx",
            &|_| Some(source.as_bytes().to_vec()),
            &Inputs::default(),
            crate::testing::fonts(),
        )
        .unwrap();
        let frame = scene.frame(Tick::ZERO).unwrap();
        let run = frame
            .template
            .nodes
            .iter()
            .position(|n| {
                matches!(
                    n.kind,
                    crate::template::Kind::Svg {
                        tag: Tag::TextRun,
                        ..
                    }
                )
            })
            .unwrap();
        assert!(frame.styles[run].non_scaling_stroke);
    }

    #[test]
    fn anchors_and_span_lengths_use_typographic_advances() {
        let source = indoc::indoc! {r#"
          export const composition = { width: 300, height: 180, fps: 1, duration: 1 };
          export default function Scene() { return <svg width={300} height={180} fontSize={24}>
            <text x={100} y={30}>MM</text>
            <text x={100} y={60} textAnchor="middle">MM</text>
            <text x={100} y={90} textAnchor="end">MM</text>
            <text x={10} y={130}><tspan textLength={60} lengthAdjust="spacingAndGlyphs">M</tspan>M</text>
          </svg>; }
        "#};
        let mut scene = Scene::load(
            "anchor.motion.tsx",
            &|_| Some(source.as_bytes().to_vec()),
            &Inputs::default(),
            crate::testing::fonts(),
        )
        .unwrap();
        let frame = scene.frame(Tick::ZERO).unwrap();
        let bounds: Vec<_> = frame
            .template
            .nodes
            .iter()
            .zip(frame.styles.iter())
            .filter_map(|(n, s)| {
                if !matches!(
                    n.kind,
                    crate::template::Kind::Svg {
                        tag: Tag::TextRun,
                        ..
                    }
                ) {
                    return None;
                }
                let Element::Shape(Shape::Text { path, .. }) = &s.svg.as_deref()?.element else {
                    return None;
                };
                Some(super::super::bounds(path))
            })
            .collect();
        // The pinned Noto Sans M advances 907/1000 em. Anchoring uses advances, not ink bounds.
        let advance = 24.0 * 0.907;
        assert!((bounds[0][0] - bounds[1][0] - advance).abs() < 0.001);
        assert!((bounds[0][0] - bounds[2][0] - 2.0 * advance).abs() < 0.001);
        assert!((bounds[4][0] - (bounds[0][0] - 100.0 + 70.0)).abs() < 0.001);
    }

    #[test]
    fn shaped_text_uses_positions_anchor_and_live_tspan_values() {
        let source = indoc::indoc! {r#"
          export const composition = { width: 300, height: 150, fps: 4, duration: 2 };
          export default function Scene() {
            const { seconds } = useCurrentTime();
            return <svg width={300} height={150}><text x={150} y={50} textAnchor="middle" fontSize={24}>
              AB<tspan dx={5 + seconds * 10} fill="red">CD</tspan>
            </text></svg>;
          }
        "#};
        let load = || {
            Scene::load(
                "text.motion.tsx",
                &|_| Some(source.as_bytes().to_vec()),
                &Inputs::default(),
                crate::testing::fonts(),
            )
            .unwrap()
        };
        let mut scene = load();
        let mut previous = None;
        for frame in [0, 5, 2, 7, 0] {
            let at = Tick::from_seconds(frame as f64 / 4.0).unwrap();
            let drawn = scene.frame(at).unwrap();
            assert_eq!(drawn.items, load().frame(at).unwrap().items);
            assert!(!drawn.items.is_empty());
            if let Some(items) = previous {
                assert_ne!(drawn.items, items);
            }
            previous = Some(drawn.items);
        }
    }

    #[test]
    fn inherited_paragraph_alignment_does_not_move_svg_glyphs() {
        let draw = |last: &str| {
            let source = indoc::indoc! {r#"
              export const composition = { width: 200, height: 80, fps: 1, duration: 1 };
              export default function Scene() {
                return <View style={{textAlignLast: "LAST", textWrap: "pretty"}}>
                  <svg width={200} height={80}><text x={10} y={40} fontSize={24}>AB<tspan fill="red">CD</tspan></text></svg>
                </View>;
              }
            "#}.replace("LAST",last);
            let mut scene = Scene::load(
                "alignment.motion.tsx",
                &|_| Some(source.as_bytes().to_vec()),
                &Inputs::default(),
                crate::testing::fonts(),
            )
            .unwrap();
            scene.frame(Tick::ZERO).unwrap().items
        };
        let expected = draw("auto");
        for last in ["right", "center", "justify"] {
            assert_eq!(draw(last), expected, "{last}");
        }
    }

    #[test]
    fn path_length_calibrates_offsets_without_resizing_glyphs() {
        let path = crate::path::parse("M10 20H210").unwrap();
        let track = Track::new(&path, Some(100.0));
        let value = |s: &str| super::super::length(&V::Str(s.into())).unwrap();
        assert_eq!(track.offset(value("20")), 40.0);
        assert_eq!(track.offset(value("20px")), 40.0);
        assert_eq!(track.offset(value("20%")), 40.0);
        assert_eq!(track.offset(value("-10")), -20.0);
        assert_eq!(
            track.point(track.offset(value("20"))),
            Some(([50.0, 20.0], 0.0))
        );
        let draw = |attributes: &str, offset: &str| {
            let source = indoc::formatdoc!(
                r##"
                export const composition={{width:320,height:100,fps:1,duration:1}};
                export default function Scene() {{ return <svg width="320" height="100">
                <defs><path id="p" d="M10 50H210" {attributes}/></defs>
                <text fontSize="24"><textPath href="#p" startOffset="{offset}">ABCD</textPath></text>
                </svg>; }}
            "##
            );
            let mut scene = Scene::load(
                "s.motion.tsx",
                &|_| Some(source.as_bytes().to_vec()),
                &Inputs::default(),
                crate::testing::fonts(),
            )
            .unwrap();
            scene.frame(Tick::ZERO).unwrap().items
        };
        assert_eq!(draw("pathLength=\"100\"", "20"), draw("", "40"));
        assert_eq!(draw("pathLength=\"100\"", "20%"), draw("", "20%"));
        assert_eq!(
            draw("pathLength=\"100\" transform=\"scale(2)\"", "20"),
            draw("transform=\"scale(2)\"", "80")
        );
    }

    #[test]
    fn animated_path_calibration_keeps_seek_results_independent() {
        let source = indoc::indoc! {r##"
            export const composition = {width:320,height:100,fps:2,duration:2};
            export default function Scene() {
                const {seconds:t}=useCurrentTime();
                return <svg width="320" height="100"><defs>
                    <path id="p" d="M10 50H210" pathLength={100+t*100}/>
                </defs><text fontSize="24"><textPath href="#p" startOffset={20+t*20}>HI</textPath></text></svg>;
            }
        "##};
        let mut scene = Scene::load(
            "s.motion.tsx",
            &|_| Some(source.as_bytes().to_vec()),
            &Inputs::default(),
            crate::testing::fonts(),
        )
        .unwrap();
        let first = scene.frame(Tick::ZERO).unwrap().items;
        for seconds in ["1", "0.5", "0"] {
            assert_eq!(
                scene
                    .frame(Tick::parse_seconds(seconds).unwrap())
                    .unwrap()
                    .items,
                first,
                "at {seconds}"
            );
        }
    }

    #[test]
    fn text_path_samples_curves_and_clips_outside_the_path() {
        let t = Track::new(&crate::path::parse("M10 20H110").unwrap(), None);
        assert_eq!(t.length, 100.0);
        assert_eq!(t.point(50.0), Some(([60.0, 20.0], 0.0)));
        assert!(t.point(-1.0).is_none());
        assert!(t.point(101.0).is_none());
        assert!(
            Position::parse(Tag::TextPath, &[("href", &V::Str("other.svg#x".into()))]).is_err()
        );
    }

    #[test]
    fn xml_space_preserves_spaces_and_default_removes_newlines() {
        let draw = |mode: &str, value: &str| {
            let source = format!(
                "export const composition={{width:240,height:80,fps:1,duration:1}}; \
                 export default function Scene() {{ return <svg width={{240}} height={{80}}>\
                 <text x={{10}} y={{40}} fontSize={{24}} xmlSpace={mode:?}>{{{value:?}}}</text></svg>; }}"
            );
            let mut scene = Scene::load(
                "space.motion.tsx",
                &|_| Some(source.as_bytes().to_vec()),
                &Inputs::default(),
                crate::testing::fonts(),
            )
            .unwrap();
            scene.frame(Tick::ZERO).unwrap().items
        };
        assert_eq!(draw("preserve", " A\n\t B "), draw("preserve", " A   B "));
        assert_eq!(draw("default", "  A\nB\t  C  "), draw("preserve", "AB C"));
        assert_ne!(draw("preserve", " A   B "), draw("default", " A   B "));
    }
}
