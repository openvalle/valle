//! Translate CSS scopes to Unicode bidi controls for the shaper, retaining a byte
//! map so layout, per-character effects and diagnostics still address authored text.
use std::{ops::Range, rc::Rc};

use super::Content;
use crate::style::bidi::{Bidi, Scope};

#[derive(Default)]
pub(super) struct Map {
    offsets: Vec<usize>,
    inserted: Vec<bool>,
}

impl Map {
    pub fn at(&self, byte: usize) -> usize {
        self.offsets.get(byte).copied().unwrap_or(byte)
    }
    pub fn inserted(&self, byte: usize) -> bool {
        self.inserted.get(byte).copied().unwrap_or(false)
    }
}

pub(super) struct Input {
    pub source: Rc<str>,
    pub text: Rc<str>,
    pub ranges: Vec<Range<usize>>,
    pub map: Map,
}

impl Input {
    pub fn new(source: String, ranges: Vec<Range<usize>>, content: &Content) -> Self {
        let source: Rc<str> = source.into();
        let root_plain = (content.font.bidi == Bidi::Plaintext)
            .then(|| content.font.bidi_scopes.last().map(|s| s.node))
            .flatten();
        let scopes: Vec<&[Scope]> = content
            .pieces
            .iter()
            .map(|p| {
                let all = &p.font.bidi_scopes;
                let start = root_plain
                    .and_then(|node| all.iter().position(|s| s.node == node))
                    .map_or(0, |k| k + 1);
                &all[start..]
            })
            .collect();
        if scopes.iter().all(|s| s.is_empty()) && content.pieces.iter().all(|p| p.edge.is_none()) {
            return Self {
                text: source.clone(),
                source,
                ranges,
                map: Map::default(),
            };
        }
        let (mut text, mut map, mut mapped) = (String::new(), Map::default(), Vec::new());
        let mut active: &[Scope] = &[];
        let controls = |text: &mut String, map: &mut Map, value: &str, at| {
            text.push_str(value);
            map.offsets.extend(std::iter::repeat_n(at, value.len()));
            map.inserted.extend(std::iter::repeat_n(true, value.len()));
        };
        for ((range, next), piece) in ranges.iter().zip(&scopes).zip(&content.pieces) {
            let start = text.len();
            let common = active
                .iter()
                .zip(next.iter())
                .take_while(|(a, b)| a == b)
                .count();
            for scope in active[common..].iter().rev() {
                controls(&mut text, &mut map, scope.controls().1, range.start);
            }
            for scope in &next[common..] {
                controls(&mut text, &mut map, scope.controls().0, range.start);
            }
            active = next;
            if let Some(edge) = &piece.edge {
                let value = if edge.start {
                    "\u{a0}\u{2060}"
                } else {
                    "\u{2060}\u{a0}"
                };
                controls(&mut text, &mut map, value, range.start);
            }
            for (i, c) in source[range.clone()].char_indices() {
                let at = range.start + i;
                if c == '\n' {
                    for scope in active.iter().rev() {
                        controls(&mut text, &mut map, scope.controls().1, at);
                    }
                }
                text.push(c);
                map.offsets.extend(at..at + c.len_utf8());
                map.inserted
                    .extend(std::iter::repeat_n(false, c.len_utf8()));
                if c == '\n' {
                    for scope in active {
                        controls(&mut text, &mut map, scope.controls().0, at + 1);
                    }
                }
            }
            mapped.push(start..text.len());
        }
        for scope in active.iter().rev() {
            controls(&mut text, &mut map, scope.controls().1, source.len());
        }
        map.offsets.push(source.len());
        Self {
            source,
            text: text.into(),
            ranges: mapped,
            map,
        }
    }
}

/// CSS plaintext chooses a base direction for each preserved paragraph. Parley's
/// glyph order already follows those paragraphs, but its alignment uses one base
/// level for the entire layout; compensate without changing shaping or source text.
pub(super) fn direction(text: &str, at: usize) -> bool {
    use icu_properties::{CodePointMapData, props::BidiClass as C};
    let start = text[..at].rfind('\n').map_or(0, |i| i + 1);
    let tail = &text[start..];
    let paragraph = tail.split('\n').next().unwrap_or("");
    let mut isolated = 0usize;
    for c in paragraph.chars() {
        match c {
            '\u{2066}'..='\u{2068}' => {
                isolated += 1;
                continue;
            }
            '\u{2069}' => {
                isolated = isolated.saturating_sub(1);
                continue;
            }
            _ if isolated > 0 => continue,
            _ => {}
        }
        match CodePointMapData::<C>::new().get(c) {
            C::LeftToRight => return false,
            C::RightToLeft | C::ArabicLetter => return true,
            _ => {}
        }
    }
    false
}

pub(super) fn shift(font: &crate::style::Font, line: &parley::Line<'_, u32>, text: &str) -> f32 {
    use crate::style::TextAlign;
    if font.bidi != Bidi::Plaintext {
        return 0.0;
    }
    let from = direction(text, 0);
    let to = direction(text, line.text_range().start);
    if from == to {
        return 0.0;
    }
    let last = matches!(
        line.break_reason(),
        parley::BreakReason::None | parley::BreakReason::Explicit
    );
    let align = if last {
        font.align_last
            .unwrap_or(if font.align == TextAlign::Justify {
                TextAlign::Start
            } else {
                font.align
            })
    } else {
        font.align
    };
    let m = line.metrics();
    let free = m.inline_max_coord - m.inline_min_coord - m.indent - m.advance + m.hanging_advance;
    let offset = |rtl| {
        let start = if rtl { -m.hanging_advance } else { m.indent };
        if free <= 0.0 {
            return start + if rtl { free } else { 0.0 };
        }
        let factor = match align {
            TextAlign::Start => f32::from(rtl),
            TextAlign::End => f32::from(!rtl),
            TextAlign::Right => 1.0,
            TextAlign::Left => 0.0,
            TextAlign::Center => 0.5,
            TextAlign::Justify => {
                if text[line.text_range()].trim().contains(char::is_whitespace) {
                    0.0
                } else {
                    f32::from(rtl)
                }
            }
        };
        start + free * factor
    };
    offset(to) - offset(from)
}

#[cfg(test)]
mod tests {
    use crate::{front::Inputs, scene::Scene, time::Tick};

    fn load(source: &str) -> Scene {
        Scene::load(
            "bidi.motion.tsx",
            &|_| Some(source.as_bytes().to_vec()),
            &Inputs::default(),
            crate::testing::fonts(),
        )
        .unwrap()
    }

    #[test]
    fn inserted_controls_keep_multibyte_offsets_in_authored_text() {
        use crate::testing::styles::{one, text};
        use std::borrow::Cow;
        let style = one(&[
            ("direction", text("rtl")),
            ("unicodeBidi", text("isolate-override")),
        ]);
        let value = "אבג\nX";
        let content = crate::text::Content {
            font: &style.font,
            clamp: None,
            pieces: vec![crate::text::Piece {
                edge: None,
                text: Cow::Borrowed(value),
                node: 0,
                font: &style.font,
            }],
        };
        let input = super::Input::new(
            value.into(),
            std::iter::once(0..value.len()).collect(),
            &content,
        );
        assert_eq!(&*input.source, value);
        for (byte, c) in input.text.char_indices() {
            if !input.map.inserted(byte) {
                assert_eq!(value[input.map.at(byte)..].chars().next(), Some(c));
            }
        }
        assert_eq!(input.map.at(input.text.len()), value.len());
        assert_eq!(input.text.matches('\u{2068}').count(), 2);
    }

    #[test]
    fn vertical_rtl_reverses_inline_sides_and_keeps_physical_cascade_order() {
        let source = indoc::indoc! {r#"
            export const composition={width:200,height:160,fps:1,duration:1};
            export default function Scene(){return <Text style={{width:80,height:120,fontSize:20,lineHeight:"30px",direction:"rtl",writingMode:"vertical-lr",paddingInline:"4px 6px",paddingBottom:9}}>ABC</Text>}
        "#};
        let frame = load(source).frame(Tick::ZERO).unwrap();
        let b = frame.layout.boxes[0];
        assert_eq!(b.content.y - b.border.y, 6.0);
        assert_eq!(b.content.height, 105.0);
        let lines = frame.layout.lines.iter().flatten().next().unwrap();
        assert!(lines.runs[0].x[0] > 60.0);
        assert!((35.0..50.0).contains(&(lines.runs[0].x[1] - lines.runs[0].x[0])));
        assert!((lines.runs.last().unwrap().x[1] - 105.0).abs() < 0.01);
    }

    #[test]
    fn plaintext_direction_is_per_paragraph_and_ignores_isolated_content() {
        assert!(!super::direction("ABC\nאבג 123", 0));
        assert!(super::direction("ABC\nאבג 123", 4));
        assert!(super::direction("مرحبا 123", 0));
        assert!(!super::direction("\u{2067}אבג\u{2069} ABC", 0));
        assert!(!super::direction("123", 0));
    }

    #[test]
    fn overrides_preserve_source_clusters_across_newlines_and_seeks() {
        let source = indoc::indoc! {r#"
            export const composition={width:300,height:150,fps:4,duration:2};
            export default function Scene(){const {seconds}=useCurrentTime();return <Text style={{width:280,fontSize:20,whiteSpace:"pre"}}>L <Span style={{direction:"rtl",unicodeBidi:seconds<1?"bidi-override":"isolate"}}>{"ABC\nDEF"}</Span> R</Text>}
        "#};
        let mut scene = load(source);
        for n in [0, 5, 2, 7, 0] {
            let at = Tick::from_seconds(n as f64 / 4.0).unwrap();
            let frame = scene.frame(at).unwrap();
            assert_eq!(frame.items, load(source).frame(at).unwrap().items);
            let lines = frame.layout.lines.iter().flatten().next().unwrap();
            assert_eq!(&*lines.string, "L ABC\nDEF R");
            assert_eq!(lines.text.len(), 2);
            for r in &lines.text {
                assert!(lines.string.get(r.clone()).is_some());
            }
            let clusters: Vec<_> = lines
                .glyphs
                .iter()
                .filter(|g| (2..5).contains(&g.cluster))
                .map(|g| g.cluster)
                .collect();
            assert_eq!(clusters, if n < 4 { vec![4, 3, 2] } else { vec![2, 3, 4] });
            assert!(
                lines
                    .glyphs
                    .iter()
                    .all(|g| (g.cluster as usize) < lines.string.len())
            );
        }
    }

    #[test]
    fn plaintext_realigns_each_preserved_paragraph() {
        let source = indoc::indoc! {r#"
            export const composition={width:240,height:100,fps:1,duration:1};
            export default function Scene(){return <Text style={{width:200,fontSize:20,unicodeBidi:"plaintext",whiteSpace:"pre"}}>{"\u200fABC\nABC"}</Text>}
        "#};
        let frame = load(source).frame(Tick::ZERO).unwrap();
        let lines = frame.layout.lines.iter().flatten().next().unwrap();
        let start = |line| {
            lines
                .runs
                .iter()
                .filter(|r| r.line == line)
                .map(|r| r.x[0])
                .fold(f32::INFINITY, f32::min)
        };
        assert!(start(0) > 150.0);
        assert!(start(1).abs() < 0.01);
    }

    #[test]
    fn direction_controls_start_alignment_and_logical_box_sides() {
        let source = indoc::indoc! {r#"
            export const composition={width:300,height:100,fps:1,duration:1};
            export default function Scene(){return <View style={{width:300,height:100,direction:"rtl",display:"flex",paddingInline:"12px 4px",gap:8}}><View key="one" style={{width:20,height:20,flexShrink:0}}/><View key="two" style={{width:30,height:20,flexShrink:0}}/><Text style={{width:150,fontSize:20,flexShrink:0}}>ABC</Text></View>}
        "#};
        let frame = load(source).frame(Tick::ZERO).unwrap();
        let b = &frame.layout.boxes;
        assert_eq!(b[0].content.x, 4.0);
        assert_eq!(b[0].content.width, 284.0);
        assert_eq!(b[1].border.x, 268.0);
        assert_eq!(b[2].border.x, 230.0);
        let lines = frame.layout.lines.iter().flatten().next().unwrap();
        assert!(lines.runs[0].x[0] > 100.0);
    }
}
