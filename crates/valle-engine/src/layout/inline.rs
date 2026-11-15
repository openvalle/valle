//! Inline box edges participate in shaping; decorations slice one continuous box across lines.
use std::borrow::Cow;

use super::{Boxes, Rect};
use crate::style::{
    Collapse, Style, WritingMode,
    math::{self, Calc, Math},
};
use crate::text::{Edge, Lines, Piece, Text};

#[derive(Clone, Debug, PartialEq)]
pub struct Fragment {
    pub node: usize,
    pub line: u32,
    pub rect: Rect,
    /// The unbroken decoration box, positioned behind this fragment.
    pub tape: Rect,
    pub border: [f32; 4],
    pub padding: [f32; 4],
}

fn padding(styles: &[Style], i: usize) -> [Calc; 4] {
    let p = styles[i].layout.padding;
    [p.top, p.right, p.bottom, p.left].map(|v| match v.expand() {
        taffy::style::ExpandedLengthPercentage::Length(n) => Math::Sum([n as f64, 0.0]),
        taffy::style::ExpandedLengthPercentage::Percent(n) => Math::Sum([0.0, n as f64]),
        taffy::style::ExpandedLengthPercentage::Calc(h) => styles
            .iter()
            .flat_map(|s| s.calcs.iter())
            .nth(math::index(h))
            .expect("computed calc handle")
            .clone(),
    })
}

/// Physical sides mapped to the paragraph's clockwise logical paint coordinates.
fn logical<T: Clone>(values: [T; 4], vertical: bool) -> [T; 4] {
    if vertical {
        [
            values[1].clone(),
            values[2].clone(),
            values[3].clone(),
            values[0].clone(),
        ]
    } else {
        values
    }
}

pub(super) fn edge(styles: &[Style], i: usize, start: bool) -> Option<Piece<'_>> {
    let s = &styles[i];
    let vertical = s.font.writing != WritingMode::Horizontal;
    let side = if start != s.font.rtl { 3 } else { 1 };
    let border = logical(s.border, vertical)[side];
    let pad = logical(padding(styles, i), vertical)[side].clone();
    if border == 0.0
        && !pad.any(&|v| v[0] != 0.0 || v[1] != 0.0)
        && s.vertical_align == crate::style::inline::Align::Baseline
    {
        return None;
    }
    Some(Piece {
        atom: None,
        text: Cow::Borrowed(""),
        node: i as u32,
        font: &s.font,
        edge: Some(Edge {
            width: Math::Max(vec![pad, Math::Sum([0.0, 0.0])]).map(&|&[px, p]| [px + border, p]),
            start,
        }),
    })
}

pub(super) fn spans(
    lines: &mut Lines,
    at: Rect,
    styles: &[Style],
    parents: &[Option<usize>],
    text: &mut Text,
    boxes: &mut [Boxes],
) -> Result<(), String> {
    let mut extents: Vec<(usize, u32, [f32; 4])> = Vec::new();
    let mut hanging = Vec::new();
    for run in &lines.runs {
        let mut x = run.x;
        let font = &styles[run.node as usize].font;
        if matches!(font.collapse, Collapse::Collapse | Collapse::PreserveBreaks) {
            // Bidi override runs can retain the advance of a space at a soft break.
            // It has no ink and is not part of the inline decoration fragment.
            let range = &lines.text[run.line as usize];
            let end = range.start
                + lines.string[range.clone()]
                    .trim_end_matches([' ', '\t', '\n', '\r', '\x0c'])
                    .len();
            let glyphs = &lines.glyphs[run.glyphs[0] as usize..run.glyphs[1] as usize];
            for (k, g) in glyphs
                .iter()
                .enumerate()
                .filter(|(_, g)| g.cluster as usize >= end)
            {
                if font.rtl {
                    x[0] = x[0].max(glyphs.get(k + 1).map_or(x[1], |next| next.x));
                } else {
                    x[1] = x[1].min(g.x);
                }
            }
        }
        let hangs = x[1] <= x[0];
        let extents = if hangs { &mut hanging } else { &mut extents };
        let baseline = at.y + lines.baselines[run.line as usize];
        let mut span = if lines.atoms.iter().any(|a| a.node == run.node) {
            parents[run.node as usize]
        } else {
            Some(run.node as usize)
        };
        while let Some(s) = span.filter(|&s| parents[s].is_some()) {
            let [ascent, descent, _] = text.metrics(&styles[s].font)?;
            let baseline = baseline + lines.offset(s as u32, run.line);
            let r = [
                at.x + x[0],
                baseline - ascent.round(),
                at.x + x[1],
                baseline + descent.round(),
            ];
            match extents
                .iter_mut()
                .find(|(n, l, _)| *n == s && *l == run.line)
            {
                Some((_, _, e)) => {
                    e[0] = e[0].min(r[0]);
                    e[1] = e[1].min(r[1]);
                    e[2] = e[2].max(r[2]);
                    e[3] = e[3].max(r[3]);
                }
                None => extents.push((s, run.line, r)),
            }
            span = parents[s];
        }
    }
    for (s, l, e) in hanging {
        if !extents.iter().any(|(n, _, _)| *n == s) {
            extents.push((s, l, e));
        }
    }
    extents.sort_by_key(|(s, line, _)| (*s, *line));
    for group in extents.chunk_by(|a, b| a.0 == b.0) {
        let i = group[0].0;
        let s = &styles[i];
        let vertical = s.font.writing != WritingMode::Horizontal;
        let border = logical(s.border.map(|v| v as f32), vertical);
        // CSS percentages in padding use the containing block's inline size.
        let basis = if vertical { at.height } else { at.width };
        let padding = logical(
            padding(styles, i).map(|p| p.resolve(basis).max(0.0)),
            vertical,
        );
        let width: f32 = group.iter().map(|(_, _, e)| e[2] - e[0]).sum();
        let mut offset = 0.0;
        let mut union = [
            f32::INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
        ];
        for &(_, line, [x0, y0, x1, y1]) in group {
            let rect = Rect {
                x: x0,
                y: y0 - padding[0] - border[0],
                width: x1 - x0,
                height: y1 - y0 + padding[0] + padding[2] + border[0] + border[2],
            };
            let start = if s.font.rtl {
                width - offset - rect.width
            } else {
                offset
            };
            let tape = Rect {
                x: rect.x - start,
                width,
                ..rect
            };
            lines.fragments.push(Fragment {
                node: i,
                line,
                rect,
                tape,
                border,
                padding,
            });
            offset += rect.width;
            union[0] = union[0].min(rect.x);
            union[1] = union[1].min(rect.y);
            union[2] = union[2].max(rect.x + rect.width);
            union[3] = union[3].max(rect.y + rect.height);
        }
        let [x, y, right, bottom] = union;
        let r = Rect {
            x,
            y,
            width: right - x,
            height: bottom - y,
        };
        boxes[i] = Boxes {
            border: r,
            padding: r,
            content: r,
        };
    }
    Ok(())
}

/// Preserve the authored inline hierarchy for baseline-relative alignment.
pub(super) fn scopes<'a>(
    template: &crate::template::Template,
    styles: &'a [Style],
    root: usize,
) -> Vec<crate::text::align::Scope<'a>> {
    use crate::style::inline::Align;
    use crate::template::Kind;
    fn needed(t: &crate::template::Template, s: &[Style], i: usize) -> bool {
        t.nodes[i].children.iter().any(|&c| {
            !matches!(t.nodes[c as usize].kind, Kind::Span { .. })
                || s[c as usize].vertical_align != Align::Baseline
                || needed(t, s, c as usize)
        })
    }
    fn visit<'a>(
        t: &crate::template::Template,
        s: &'a [Style],
        i: usize,
        parent: Option<usize>,
        out: &mut Vec<crate::text::align::Scope<'a>>,
    ) {
        let at = out.len();
        out.push(crate::text::align::Scope {
            node: i as u32,
            parent,
            font: &s[i].font,
            align: &s[i].vertical_align,
        });
        if matches!(t.nodes[i].kind, Kind::Text { .. } | Kind::Span { .. }) {
            for &c in &t.nodes[i].children {
                visit(t, s, c as usize, Some(at), out);
            }
        }
    }
    let mut out = Vec::new();
    if needed(template, styles, root) {
        visit(template, styles, root, None, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use crate::{front::Inputs, scene::Scene, time::Tick};

    fn load(body: &str) -> Scene {
        let source = format!(
            "export const composition={{width:320,height:240,fps:4,duration:2}}; export default function Scene(){{{body}}}"
        );
        Scene::load(
            "inline.motion.tsx",
            &|_| Some(source.as_bytes().to_vec()),
            &Inputs::default(),
            crate::testing::fonts(),
        )
        .unwrap()
    }

    #[test]
    fn inline_edges_add_advance_without_authored_characters_or_extra_line_height() {
        let mut scene = load(
            r#"return <Text style={{fontSize:20,lineHeight:"40px"}}>A<Span style={{padding:"8px 10px",border:"2px solid red"}}>B</Span>C</Text>"#,
        );
        let frame = scene.frame(Tick::ZERO).unwrap();
        let lines = frame.layout.lines.iter().flatten().next().unwrap();
        assert_eq!(&*lines.string, "ABC");
        assert_eq!(lines.boxes, [[0.0, 40.0]]);
        let edges: Vec<_> = lines
            .runs
            .iter()
            .filter(|r| r.edge.is_some() && r.x[1] > r.x[0])
            .collect();
        assert_eq!(edges.len(), 2);
        for r in edges {
            assert!((r.x[1] - r.x[0] - 12.0).abs() < 0.01, "{r:?}");
        }
        let fragment = &lines.fragments[0];
        assert!(fragment.rect.height > 40.0);
        assert_eq!(
            lines.glyphs.iter().map(|g| g.cluster).collect::<Vec<_>>(),
            [0, 1, 2]
        );
    }

    #[test]
    fn wrapped_fragments_slice_one_box_and_seek_without_stale_padding() {
        let body = r#"const {seconds}=useCurrentTime();return <Text style={{width:150,fontSize:20,lineHeight:"40px"}}><Span style={{padding:seconds<1?"2px 8px":"2px 20px",border:"2px solid red",background:"yellow"}}>one two three four five six seven</Span></Text>"#;
        let mut scene = load(body);
        for second in [0.0, 1.5, 0.25] {
            let at = Tick::from_seconds(second).unwrap();
            let frame = scene.frame(at).unwrap();
            assert_eq!(frame.items, load(body).frame(at).unwrap().items);
            let lines = frame.layout.lines.iter().flatten().next().unwrap();
            assert!(lines.fragments.len() > 1);
            let total: f32 = lines.fragments.iter().map(|f| f.rect.width).sum();
            assert!(
                lines
                    .fragments
                    .iter()
                    .all(|f| (f.tape.width - total).abs() < 0.01)
            );
            assert_eq!(
                lines
                    .text
                    .iter()
                    .map(|r| &lines.string[r.clone()])
                    .collect::<String>(),
                &*lines.string
            );
        }
    }

    #[test]
    fn rtl_controls_do_not_extend_the_visible_fragment_into_hanging_space() {
        let frame = load(r#"return <Text style={{width:230,fontSize:20,lineHeight:"40px",direction:"rtl",unicodeBidi:"bidi-override"}}>{"AB "}<Span style={{padding:"3px 8px",border:"2px solid blue"}}>CD EF GH IJ KL MN OP</Span>{" QR"}</Text>"#).frame(Tick::ZERO).unwrap();
        let lines = frame.layout.lines.iter().flatten().next().unwrap();
        assert_eq!(
            lines.boxes.iter().map(|b| b[1]).collect::<Vec<_>>(),
            [40.0, 40.0]
        );
        assert!(
            (lines.fragments[0].rect.x - 16.0).abs() < 1.0,
            "{:?} {:?}",
            lines.fragments,
            lines.runs
        );
    }

    #[test]
    fn no_break_spaces_remain_inside_the_decoration() {
        let frame = load(r#"return <Text style={{fontSize:20}}><Span style={{background:"yellow"}}>{"A\u00a0"}</Span></Text>"#).frame(Tick::ZERO).unwrap();
        let lines = frame.layout.lines.iter().flatten().next().unwrap();
        assert!((lines.fragments[0].rect.width - lines.runs.last().unwrap().x[1]).abs() < 0.01);
    }

    #[test]
    fn empty_span_keeps_its_font_line_height() {
        let frame = load(r#"return <Text style={{fontSize:20,lineHeight:1.5}}>A<Span style={{fontSize:40,padding:4,border:"1px solid red"}}/>B</Text>"#).frame(Tick::ZERO).unwrap();
        let lines = frame.layout.lines.iter().flatten().next().unwrap();
        assert!((lines.boxes[0][1] - 60.0).abs() < 0.01);
    }

    #[test]
    fn vertical_percentage_padding_uses_containing_inline_size() {
        let frame = load(r#"return <Text style={{width:100,height:160,fontSize:20,writingMode:"vertical-rl"}}><Span style={{padding:"10%",border:"2px solid red"}}>AB</Span></Text>"#).frame(Tick::ZERO).unwrap();
        let lines = frame.layout.lines.iter().flatten().next().unwrap();
        for edge in lines
            .runs
            .iter()
            .filter(|r| r.edge.is_some() && r.x[1] > r.x[0])
        {
            assert!((edge.x[1] - edge.x[0] - 18.0).abs() < 0.01, "{edge:?}");
        }
    }

    #[test]
    fn empty_and_vertical_spans_keep_inline_edges() {
        for writing in ["horizontal-tb", "vertical-rl"] {
            let body = format!(
                r#"return <Text style={{{{width:120,height:180,fontSize:20,lineHeight:"40px",writingMode:"{writing}"}}}}>A<Span style={{{{padding:8,border:"2px solid red"}}}}/>B</Text>"#
            );
            let frame = load(&body).frame(Tick::ZERO).unwrap();
            let lines = frame.layout.lines.iter().flatten().next().unwrap();
            assert_eq!(&*lines.string, "AB");
            assert!(lines.runs.iter().any(|r| r.edge.is_some()));
            assert!(
                (lines.fragments[0].rect.width - 20.0).abs() < 0.01,
                "{:?}",
                lines.fragments
            );
        }
    }
}
