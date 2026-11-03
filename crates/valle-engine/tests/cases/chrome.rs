//! Chrome baselines: what the pinned Chrome for Testing makes of a case's frames printed as HTML,
//! `tests/baselines/chrome/<case>.<frame>.json` and `.png`. `web/tests/chrome.ts` draws them when
//! they are missing, or were drawn from other HTML or by another Chrome. The engine's boxes and
//! lines are compared with them, and on the GPU its pixels, without glyphs or their shadows:
//! Chrome's own text rendering is not what the engine draws. Whenever it draws, it also lists the CSS properties
//! Chrome knows, `tests/baselines/chrome/properties.json`.

use std::collections::BTreeMap;
use std::path::Path;
use std::rc::Rc;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use valle_engine::image::Picture;
use valle_engine::paint::{self, Item};
use valle_engine::scene::Frame;
use valle_engine::style::color::{self, Rgba};
use valle_engine::style::{FontStyle, Style, TextShadow, TextTransform};
use valle_engine::template::{Kind, Tag};

/// How far a box edge may be from Chrome's: two of its 1/64 px layout units. Chrome truncates
/// each length it resolves, a percentage say, to a unit, which the engine does not, and an edge
/// adds up several.
const EDGE: f64 = 2.0 / 64.0;
/// How far a pixel may be from Chrome's, in levels of each premultiplied channel.
const LEVELS: i32 = 8;
/// How far a length or a number of a font may be from Chrome's: one of its 1/64 px units.
const UNIT: f64 = 1.0 / 64.0;

#[derive(Deserialize)]
pub struct Baseline {
    /// The Chrome that drew it, and from what.
    pub chrome: String,
    pub html: String,
    /// Each node's border box, x, y, width and height, by node index: around it as transformed.
    pub boxes: Vec<[f64; 4]>,
    /// The text of each line of each text node, by node index.
    pub lines: BTreeMap<u32, Vec<String>>,
    /// The computed style of each Text and Span, by node index.
    pub fonts: BTreeMap<u32, Font>,
}

/// What a text node's glyphs show, which are not in the pixels: computed values as Chrome writes
/// them.
#[derive(Deserialize)]
pub struct Font {
    color: String,
    fill: String,
    stroke: String,
    stroke_color: String,
    shadow: String,
    size: String,
    weight: String,
    stretch: String,
    style: String,
    transform: String,
}

/// The CSS properties a Chrome knows, by their CSS names.
#[derive(Deserialize)]
pub struct Properties {
    pub chrome: String,
    pub properties: Vec<String>,
    /// Each easing's progress at twenty-firsts of the way, which `ease.rs` is tested against.
    #[serde(default)]
    pub easings: BTreeMap<String, Vec<f64>>,
}

/// A baseline to draw: the HTML, the canvas size, and its path without the extension.
#[derive(Serialize)]
pub struct Job {
    pub html: String,
    pub size: [u32; 2],
    pub out: String,
}

pub fn read<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, String> {
    let bad = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).map(Some).map_err(|e| bad(&e)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(bad(&e)),
    }
}

/// Where the frame's boxes and lines differ from Chrome's.
pub fn compare(frame: &Frame, chrome: &Baseline) -> Vec<String> {
    let nodes = &frame.template.nodes;
    if chrome.boxes.len() != nodes.len() {
        let (theirs, ours) = (chrome.boxes.len(), nodes.len());
        return vec![format!("Chrome has {theirs} boxes for {ours} nodes")];
    }
    let mut problems = Vec::new();
    for (i, (node, theirs)) in nodes.iter().zip(&chrome.boxes).enumerate() {
        // The elements in an <svg> are no boxes; its pixels show them.
        if matches!(&node.kind, Kind::Svg { tag, .. } if *tag != Tag::Svg) {
            continue;
        }
        let r = frame.transforms[i].bounds(frame.layout.boxes[i].border);
        let ours = [r.x, r.y, r.width, r.height].map(f64::from);
        let edges = |[x, y, w, h]: [f64; 4]| [x, y, x + w, y + h];
        let off = (edges(ours).iter().zip(edges(*theirs))).any(|(a, b)| (a - b).abs() > EDGE);
        if off {
            let (ours, theirs) = (shown(ours), shown(*theirs));
            problems.push(format!("{}: box {ours}, Chrome's {theirs}", node.place()));
        }
        if let Kind::Text { .. } = &node.kind {
            let ours = lines(frame, i);
            let theirs = chrome.lines.get(&(i as u32)).cloned().unwrap_or_default();
            let theirs: Vec<String> = theirs.iter().map(|l| canonical(l)).collect();
            if ours != theirs {
                problems.push(format!(
                    "{}: lines {ours:?}, Chrome's {theirs:?}",
                    node.place()
                ));
            }
        }
        if let Some(theirs) = chrome.fonts.get(&(i as u32)) {
            let off = font(&frame.styles[i], theirs);
            problems.extend(off.into_iter().map(|p| format!("{}: {p}", node.place())));
        }
    }
    problems
}

/// Where a text node's style differs from Chrome's.
fn font(style: &Style, theirs: &Font) -> Vec<String> {
    let f = &style.font;
    let slant = match f.style {
        FontStyle::Normal => "normal".to_string(),
        FontStyle::Italic => "italic".to_string(),
        // Chrome leaves out the angle it defaults to.
        FontStyle::Oblique(14.0) => "oblique".to_string(),
        FontStyle::Oblique(angle) => format!("oblique {angle}deg"),
    };
    let transform = match f.transform {
        TextTransform::None => "none",
        TextTransform::Uppercase => "uppercase",
        TextTransform::Lowercase => "lowercase",
        TextTransform::Capitalize => "capitalize",
    };
    let word = |what: &str, ours: &str, theirs: &str| {
        (ours != theirs).then(|| format!("{what} {ours}, Chrome's {theirs}"))
    };
    let stroke = f.stroke_color.unwrap_or(style.color);
    let colors = [
        ("color", style.color, &theirs.color),
        ("fill", f.fill.unwrap_or(style.color), &theirs.fill),
        ("stroke color", stroke, &theirs.stroke_color),
    ];
    let numbers = [
        ("stroke", f.stroke, &theirs.stroke),
        ("size", f.size, &theirs.size),
        ("weight", f.weight, &theirs.weight),
        ("stretch", f.stretch, &theirs.stretch),
    ];
    let shadows = (shadows(&theirs.shadow)).filter(|s| {
        let alike = |(a, b): (&TextShadow, &TextShadow)| {
            let lengths = [a.offset[0], a.offset[1], a.blur];
            let others = [b.offset[0], b.offset[1], b.blur];
            let current = |c: Option<Rgba>| c.unwrap_or(style.color);
            (lengths.iter().zip(others)).all(|(a, b)| (a - b).abs() <= UNIT)
                && close(current(a.color), current(b.color))
        };
        s.len() == f.shadows.len() && f.shadows.iter().zip(s).all(alike)
    });
    let colors = colors.into_iter().filter_map(|(what, ours, theirs)| {
        let same = color::parse(theirs).is_ok_and(|t| close(ours, t));
        (!same).then(|| format!("{what} {ours:?}, Chrome's {theirs}"))
    });
    let numbers = numbers.into_iter().filter_map(|(what, ours, theirs)| {
        let value = theirs.trim_end_matches(['p', 'x', '%']).parse::<f64>();
        let same = value.is_ok_and(|t| (ours - t).abs() <= UNIT);
        (!same).then(|| format!("{what} {ours}, Chrome's {theirs}"))
    });
    let shadow = shadows.is_none().then(|| {
        let ours = &f.shadows;
        format!("text shadows {ours:?}, Chrome's {}", theirs.shadow)
    });
    (colors.chain(numbers))
        .chain(word("font style", &slant, &theirs.style))
        .chain(word("text transform", transform, &theirs.transform))
        .chain(shadow)
        .collect()
}

/// Whether two colors are the same to within a level of each channel.
fn close(a: Rgba, b: Rgba) -> bool {
    (0..4).all(|k| (a[k] - b[k]).abs() <= 1.5 / 255.0)
}

/// A computed `text-shadow`: `none`, or a color and three lengths each.
fn shadows(s: &str) -> Option<Vec<TextShadow>> {
    if s == "none" {
        return Some(Vec::new());
    }
    let mut depth = 0;
    let parts = s.split(|c| {
        depth += (c == '(') as i32 - (c == ')') as i32;
        c == ',' && depth == 0
    });
    parts
        .map(|shadow| {
            let px = |w: &str| w.strip_suffix("px")?.parse::<f64>().ok();
            let (rest, blur) = shadow.trim().rsplit_once(' ')?;
            let (rest, y) = rest.rsplit_once(' ')?;
            let (color, x) = rest.rsplit_once(' ')?;
            Some(TextShadow {
                offset: [px(x)?, px(y)?],
                blur: px(blur)?,
                color: Some(color::parse(color).ok()?),
            })
        })
        .collect()
}

/// The text of each of the node's lines, as `canonical` has it.
fn lines(frame: &Frame, i: usize) -> Vec<String> {
    let Some(lines) = &frame.layout.lines[i] else {
        return Vec::new();
    };
    (lines.text.iter())
        .map(|r| canonical(&lines.string[r.clone()]))
        .collect()
}

/// A line's text as both sides can give it: Chrome's reader takes the text as written, before
/// white space processing and `text-transform`, so runs of white space are one space, none at
/// the ends, and letters are upper case.
fn canonical(line: &str) -> String {
    let white = |c: char| matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0C');
    let words: Vec<&str> = line.split(white).filter(|w| !w.is_empty()).collect();
    words.join(" ").to_uppercase()
}

fn shown([x, y, w, h]: [f64; 4]) -> String {
    format!("{w} × {h} at {x}, {y}")
}

/// The frame painted as Chrome draws its baselines: its glyphs unfilled
/// (`-webkit-text-fill-color: transparent`), and without strokes or text shadows, which are
/// glyphs too.
pub fn glyphless(frame: &Frame, pictures: &[Picture]) -> Vec<Item> {
    let mut styles = frame.styles.clone();
    for s in &mut styles {
        s.font.fill = Some(color::TRANSPARENT);
        s.font.stroke = 0.0;
        s.font.shadows = Rc::new([]);
    }
    paint::paint(&frame.template, &styles, &frame.layout, pictures)
}

/// The pixels of `a` or `b`, both `size` and RGBA, with a channel more than `LEVELS` outside the
/// values it takes in the 3 × 3 around them in the other; and an image marking them. An edge's
/// pixels range from inside to outside, so antialiasing that differs passes, and Chrome's CPU
/// raster is not exact on curves.
pub fn far(a: &[u8], b: &[u8], size: [u32; 2]) -> (usize, Vec<u8>) {
    let [w, h] = size.map(|n| n as usize);
    let at = |image: &[u8], x: usize, y: usize| {
        let p = &image[(y * w + x) * 4..][..4];
        let alpha = i32::from(p[3]);
        let pre = |c: u8| (i32::from(c) * alpha + 127) / 255;
        [pre(p[0]), pre(p[1]), pre(p[2]), alpha]
    };
    let near = |from: &[u8], to: &[u8], x: usize, y: usize| {
        let (mut low, mut high) = ([i32::MAX; 4], [i32::MIN; 4]);
        for ny in y.saturating_sub(1)..=(y + 1).min(h - 1) {
            for nx in x.saturating_sub(1)..=(x + 1).min(w - 1) {
                for (c, v) in at(to, nx, ny).into_iter().enumerate() {
                    (low[c], high[c]) = (low[c].min(v), high[c].max(v));
                }
            }
        }
        let p = at(from, x, y);
        (0..4).all(|c| low[c] - LEVELS <= p[c] && p[c] <= high[c] + LEVELS)
    };
    let mut count = 0;
    let mut marked = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let off = !near(a, b, x, y) || !near(b, a, x, y);
            count += usize::from(off);
            marked.extend(if off { [255, 0, 0, 255] } else { [255; 4] });
        }
    }
    (count, marked)
}
