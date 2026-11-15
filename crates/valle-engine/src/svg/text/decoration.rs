//! Vector decoration bands following positioned SVG characters. Paint belongs to the
//! declaring text element; clips use the bands as opaque geometry independent of that paint.
use std::collections::HashMap;

use super::{Decoration, Placed, Track};
use crate::paint::Matrix;
use crate::path::{Path, Segment};
use crate::style::{Cap, DecorationStyle, Join, Style};
use crate::svg::{self, Pen};
use crate::text::Text;

pub(super) fn outlines(
    placed: &[Placed],
    styles: &[Style],
    text: &mut Text,
    tracks: &HashMap<usize, Track>,
) -> Result<HashMap<usize, Vec<Decoration>>, String> {
    let mut paths = HashMap::<(usize, usize, usize, usize), (Path, Pen)>::new();
    let mut starts = HashMap::<usize, f64>::new();
    for g in placed {
        if g.advance <= 0.0 {
            continue;
        }
        for d in &styles[g.node].decorations {
            // CSS decoration propagation outside SVG does not decorate its text.
            if styles[d.by].svg.is_none() {
                continue;
            }
            let owner_baseline = g
                .baselines
                .iter()
                .find(|(n, _)| *n == d.by)
                .or_else(|| g.baselines.first())
                .map_or(0.0, |(_, b)| *b);
            let font = &styles[d.by].font;
            let size = font.size as f32;
            let thickness = d
                .thickness
                .as_ref()
                .map_or((size / 10.0).max(1.0), |v| v.resolve(size).max(0.0));
            if thickness == 0.0 {
                continue;
            }
            let [ascent, _, _] = text.metrics(font)?;
            let offset = d
                .offset
                .as_ref()
                .map_or((thickness / 2.0).ceil().max(1.0), |v| v.resolve(size));
            let start = *starts.entry(d.by).or_insert(g.at[0]);
            let baseline = g.at[1] - g.baselines.last().map_or(0.0, |(_, b)| *b) + owner_baseline;
            for (line, enabled) in d.lines.into_iter().enumerate() {
                if !enabled {
                    continue;
                }
                let y = match line {
                    0 => offset + thickness / 2.0,
                    1 => -ascent - thickness / 2.0,
                    _ => -ascent / 3.0,
                };
                let copies = if d.style == DecorationStyle::Double {
                    2
                } else {
                    1
                };
                for copy in 0..copies {
                    let y =
                        y + copy as f32 * (thickness + 1.0) * if line == 1 { -1.0 } else { 1.0 };
                    let y = if d.style == DecorationStyle::Wavy {
                        y - thickness / 2.0
                            + 0.5
                            + (thickness + 1.0) * if line == 1 { -1.0 } else { 1.0 }
                    } else {
                        y
                    };
                    let mut path = Path::default();
                    let count = if g.track.is_some() || d.style == DecorationStyle::Wavy {
                        (g.advance / 1.0).ceil().max(1.0) as usize
                    } else {
                        1
                    };
                    let rotation = Matrix::rotate(g.angle);
                    for k in 0..=count {
                        let local_x = g.advance * k as f64 / count as f64;
                        let x = g.at[0] + local_x;
                        let wave = if d.style == DecorationStyle::Wavy {
                            let [step, control] = crate::text::wave(thickness).map(f64::from);
                            let target = (x - start).rem_euclid(step * 2.0);
                            let (mut lo, mut hi) = (0.0, 1.0);
                            for _ in 0..20 {
                                let u = (lo + hi) * 0.5;
                                let at = step * 3.0 * u * (1.0 - u) + 2.0 * step * u * u * u;
                                if at < target {
                                    lo = u;
                                } else {
                                    hi = u;
                                }
                            }
                            let u = (lo + hi) * 0.5;
                            control * 3.0 * u * (1.0 - u) * (1.0 - 2.0 * u)
                        } else {
                            0.0
                        };
                        let y = baseline + f64::from(y) + wave;
                        let point = match g.track {
                            Some(n) => {
                                let Some((p, angle)) = tracks[&n].point(x) else {
                                    continue;
                                };
                                Matrix::translate(p[0], p[1])
                                    .times(Matrix::rotate(angle))
                                    .apply([0.0, y as f32])
                            }
                            None => {
                                let p = rotation.apply([local_x as f32, (y - g.at[1]) as f32]);
                                [p[0] + g.at[0] as f32, p[1] + g.at[1] as f32]
                            }
                        };
                        path.0.push(if path.0.is_empty() {
                            Segment::Move(point)
                        } else {
                            Segment::Line(point)
                        });
                    }
                    let dotted = d.style == DecorationStyle::Dotted;
                    let w = f64::from(thickness);
                    let dashes = match d.style {
                        DecorationStyle::Dotted => vec![0.0, w * 2.0],
                        DecorationStyle::Dashed => vec![w * 3.0, w * 2.0],
                        _ => Vec::new(),
                    };
                    let pen = Pen {
                        width: w,
                        cap: if dotted { Cap::Round } else { Cap::Butt },
                        join: Join::Round,
                        miter_limit: 4.0,
                        dashes,
                        dash_offset: g.at[0] - start,
                        path_length: None,
                    };
                    let (joined, _) = paths
                        .entry((g.node, d.by, line, copy))
                        .or_insert_with(|| (Path::default(), pen));
                    if let (Some(Segment::Line(last)), Some(Segment::Move(first))) =
                        (joined.0.last(), path.0.first())
                        && (last[0] - first[0]).abs() < 1e-4
                        && (last[1] - first[1]).abs() < 1e-4
                    {
                        path.0.remove(0);
                    }
                    joined.0.extend(path.0);
                }
            }
        }
    }
    let mut out = HashMap::<usize, Vec<Decoration>>::new();
    // Stable paint order across frames, independent of the map's random seed.
    let mut paths: Vec<_> = paths.into_iter().collect();
    paths.sort_by_key(|(key, _)| *key);
    for ((node, by, line, _), (path, pen)) in paths {
        if let Some(path) = svg::stroke(&path, &pen, 1.0) {
            out.entry(node).or_default().push(Decoration {
                path,
                by,
                through: line == 2,
            });
        }
    }
    Ok(out)
}
