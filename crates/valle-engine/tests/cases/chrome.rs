//! Chrome baselines: what the pinned Chrome for Testing makes of a case's frames printed as HTML,
//! `tests/baselines/chrome/<case>.<frame>.json` and `.png`. `web/tests/chrome.ts` draws them when
//! they are missing, or were drawn from other HTML or by another Chrome. The engine's boxes and
//! lines are compared with them, and on the GPU its pixels, without the glyphs: Chrome's own text
//! rendering is not what the engine draws. Whenever it draws, it also lists the CSS properties
//! Chrome knows, `tests/baselines/chrome/properties.json`.

use std::collections::BTreeMap;
use std::path::Path;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use valle_engine::paint::Item;
use valle_engine::scene::Frame;
use valle_engine::template::{Kind, Prop};
use valle_engine::text::collapse;

use crate::html::value;

/// How far a box edge may be from Chrome's: two of its 1/64 px layout units. Chrome truncates
/// each length it resolves, a percentage say, to a unit, which the engine does not, and an edge
/// adds up several.
const EDGE: f64 = 2.0 / 64.0;
/// How far a pixel may be from Chrome's, in levels of each premultiplied channel.
const LEVELS: i32 = 8;

#[derive(Deserialize)]
pub struct Baseline {
    /// The Chrome that drew it, and from what.
    pub chrome: String,
    pub html: String,
    /// Each node's border box, x, y, width and height, by node index.
    pub boxes: Vec<[f64; 4]>,
    /// The text of each line of each text node, by node index.
    pub lines: BTreeMap<u32, Vec<String>>,
}

/// The CSS properties a Chrome knows, by their CSS names.
#[derive(Deserialize)]
pub struct Properties {
    pub chrome: String,
    pub properties: Vec<String>,
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
        let r = frame.layout.boxes[i].border;
        let ours = [r.x, r.y, r.width, r.height].map(f64::from);
        let edges = |[x, y, w, h]: [f64; 4]| [x, y, x + w, y + h];
        let off = (edges(ours).iter().zip(edges(*theirs))).any(|(a, b)| (a - b).abs() > EDGE);
        if off {
            let (ours, theirs) = (shown(ours), shown(*theirs));
            problems.push(format!("{}: box {ours}, Chrome's {theirs}", node.place()));
        }
        if let Kind::Text { text } = &node.kind {
            let ours = lines(frame, i, text);
            let theirs = chrome.lines.get(&(i as u32)).cloned().unwrap_or_default();
            if ours != theirs {
                problems.push(format!(
                    "{}: lines {ours:?}, Chrome's {theirs:?}",
                    node.place()
                ));
            }
        }
    }
    problems
}

/// The text of each of the node's lines.
fn lines(frame: &Frame, i: usize, text: &Prop) -> Vec<String> {
    let Some(lines) = &frame.layout.lines[i] else {
        return Vec::new();
    };
    let text = value(frame, text).text();
    let text = collapse(&text);
    (lines.text.iter())
        .map(|r| text[r.clone()].trim_matches(' ').to_string())
        .collect()
}

fn shown([x, y, w, h]: [f64; 4]) -> String {
    format!("{w} × {h} at {x}, {y}")
}

/// The display list without its glyphs.
pub fn shapes(items: &[Item]) -> Vec<Item> {
    (items.iter())
        .filter_map(|item| match item {
            Item::Glyphs { .. } => None,
            Item::Group { opacity, items } => {
                let items = shapes(items);
                let opacity = *opacity;
                (!items.is_empty()).then_some(Item::Group { opacity, items })
            }
            item => Some(item.clone()),
        })
        .collect()
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
