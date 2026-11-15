//! Align nested inline subtrees after line breaking. The ordinary baseline-only path
//! keeps its existing metrics; aligned subtrees contribute to the shared line box.
use std::collections::HashMap;

use super::{Content, Lines, Text};
use crate::style::{Font, inline::Align, math::Calc};

#[derive(Clone)]
pub struct Scope<'a> {
    pub node: u32,
    /// Index in Content::scopes, not a template node id.
    pub parent: Option<usize>,
    pub font: &'a Font,
    pub align: &'a Align<Calc>,
}

pub(super) fn needed(content: &Content) -> bool {
    !content.scopes.is_empty()
        && (content.pieces.iter().any(|p| p.atom.is_some())
            || content
                .scopes
                .iter()
                .skip(1)
                .any(|s| s.align != &Align::Baseline))
}

pub(super) fn apply(text: &mut Text, content: &Content, lines: &mut Lines) -> Result<(), String> {
    if !needed(content) || lines.boxes.is_empty() {
        return Ok(());
    }
    let scopes = &content.scopes;
    let indices: HashMap<_, _> = scopes
        .iter()
        .enumerate()
        .map(|(k, s)| (s.node, k))
        .collect();
    let mut metrics = Vec::new();
    for s in scopes {
        let m = text.metrics(s.font)?;
        let extent = content
            .pieces
            .iter()
            .find(|p| p.node == s.node)
            .and_then(|p| p.atom)
            .map_or_else(
                || super::placed(m, s.font),
                |a| [a.baseline, a.size[1] - a.baseline],
            );
        metrics.push((m, extent, text.x_height(s.font)?));
    }
    let mut top = 0.0;
    let mut previous_end = 0.0;
    for line in 0..lines.boxes.len() {
        top += lines.boxes[line][0] - previous_end;
        previous_end = lines.boxes[line][0] + lines.boxes[line][1];
        let mut present = vec![false; scopes.len()];
        present[0] = true;
        for run in lines.runs.iter().filter(|r| r.line as usize == line) {
            let mut index = indices.get(&run.node).copied();
            while let Some(k) = index {
                present[k] = true;
                index = scopes[k].parent;
            }
        }
        let mut roots = vec![0; scopes.len()];
        let mut offsets = vec![0.0; scopes.len()];
        let mut groups: HashMap<usize, [f32; 2]> = HashMap::new();
        for (k, s) in scopes.iter().enumerate().filter(|(k, _)| present[*k]) {
            let [above, below] = metrics[k].1;
            if let Some(parent) = s.parent {
                let (m, _, x_height) = metrics[parent];
                let central = content.font.writing != crate::style::WritingMode::Horizontal;
                let center = (metrics[parent].1[0] - metrics[parent].1[1]) * 0.5;
                let base = if central {
                    (above - below) * 0.5 - center
                } else {
                    0.0
                };
                let shift = match s.align {
                    Align::Baseline => base,
                    Align::Middle => {
                        (above - below - x_height) * 0.5 - if central { center } else { 0.0 }
                    }
                    Align::Sub => base + scopes[parent].font.size as f32 * 0.2 + 1.0,
                    Align::Super => base - (scopes[parent].font.size as f32 / 3.0 + 1.0),
                    Align::TextTop => above - m[0].round(),
                    Align::TextBottom => m[1].round() - below,
                    Align::Offset(value) => {
                        base - value.resolve(super::placed(metrics[k].0, s.font).iter().sum())
                    }
                    Align::Top | Align::Bottom => 0.0,
                };
                if matches!(s.align, Align::Top | Align::Bottom) {
                    roots[k] = k;
                } else {
                    roots[k] = roots[parent];
                    offsets[k] = offsets[parent] + shift;
                }
            }
            let extent = [above - offsets[k], below + offsets[k]];
            groups
                .entry(roots[k])
                .and_modify(|v| {
                    v[0] = v[0].max(extent[0]);
                    v[1] = v[1].max(extent[1]);
                })
                .or_insert(extent);
        }
        let [mut above, mut below] = groups[&0];
        for (&root, [a, b]) in &groups {
            if matches!(scopes[root].align, Align::Bottom) {
                above = above.max(a + b - below)
            }
        }
        for (&root, [a, b]) in &groups {
            if matches!(scopes[root].align, Align::Top) {
                below = below.max(a + b - above)
            }
        }
        let baseline = top + above;
        let old = lines.baselines[line];
        for k in 0..scopes.len() {
            if !present[k] {
                continue;
            }
            let root = roots[k];
            offsets[k] += match scopes[root].align {
                Align::Top if root != 0 => groups[&root][0] - above,
                Align::Bottom if root != 0 => below - groups[&root][1],
                _ => 0.0,
            };
            lines
                .offsets
                .push((scopes[k].node, line as u32, offsets[k]));
        }
        for run in lines.runs.iter().filter(|r| r.line as usize == line) {
            let shift = baseline - old + indices.get(&run.node).map_or(0.0, |&k| offsets[k]);
            for g in &mut lines.glyphs[run.glyphs[0] as usize..run.glyphs[1] as usize] {
                g.y += shift;
            }
        }
        for atom in lines.atoms.iter_mut().filter(|a| a.line as usize == line) {
            atom.rect.y += baseline - old + indices.get(&atom.node).map_or(0.0, |&k| offsets[k]);
        }
        for mark in lines.marks.iter_mut().filter(|m| m.line as usize == line) {
            let shift = baseline - old + indices.get(&mark.node).map_or(0.0, |&k| offsets[k]);
            mark.path = mark.path.moved([0.0, shift]);
        }
        lines.boxes[line] = [top, above + below];
        lines.baselines[line] = baseline;
        top += above + below;
    }
    Ok(())
}
