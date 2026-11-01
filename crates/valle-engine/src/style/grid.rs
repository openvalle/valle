//! CSS grid's own values: track lists with the names of the lines between their tracks, and the
//! lines items go between. Line names are case-sensitive, so these read values as written;
//! keywords, units and functions are not.

use taffy::{
    GridAutoFlow, GridPlacement, GridTemplateComponent, GridTemplateRepetition, Line,
    MaxTrackSizingFunction as Max, MinMax, MinTrackSizingFunction as Min, RepetitionCount,
    TrackSizingFunction,
};

use super::{Accept, Len, Lengths, css, length, length_text, shown, text};
use crate::template::V;

/// A track's size: the least and the most it takes, or `fit-content()`'s limit.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Track {
    MinMax(Breadth, Breadth),
    FitContent(Len),
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Breadth {
    Len(Len),
    Fr(f64),
    MinContent,
    MaxContent,
    Auto,
}

impl Track {
    /// A track of the length `l`.
    fn len(l: Len) -> Track {
        Track::MinMax(Breadth::Len(l.clone()), Breadth::Len(l))
    }

    /// Whether a length or a percentage bounds it, as tracks beside `auto-fill` need.
    fn fixed(&self) -> bool {
        matches!(
            self,
            Track::MinMax(Breadth::Len(_), _) | Track::MinMax(_, Breadth::Len(_))
        )
    }
}

/// A track list: tracks and repetitions of them, each after the names of the line before it.
/// `none` is empty.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Tracks {
    parts: Vec<Part>,
    /// The names of the line before each part, and of the last line.
    names: Vec<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq)]
enum Part {
    Track(Track),
    Repeat {
        count: RepetitionCount,
        tracks: Vec<Track>,
        /// The names of the line before each track, and of the line after the last.
        names: Vec<Vec<String>>,
    },
}

/// Track lengths: not negative, and percentages of the grid's content box.
const TRACK: Accept = Accept {
    keyword: None,
    negative: false,
    percent: true,
    sizing: &[],
};

/// A string as written, trimmed.
fn written(v: &V) -> Result<&str, String> {
    match v {
        V::Str(s) => Ok(s.trim()),
        other => Err(format!("{} is not a string", shown(other))),
    }
}

/// `grid-template-columns` and `-rows`: `none`, or a track list; a number is one track of px.
pub(super) fn template(v: &V) -> Result<Tracks, String> {
    if let V::Num(_) = v {
        return Ok(Tracks {
            parts: vec![Part::Track(Track::len(length(v, TRACK)?))],
            names: vec![Vec::new(); 2],
        });
    }
    let s = written(v)?;
    let lower = s.to_ascii_lowercase();
    if lower == "none" {
        return Ok(Tracks::default());
    }
    if let Some(w) = ["subgrid", "masonry"]
        .into_iter()
        .find(|w| lower.starts_with(w))
    {
        return Err(format!("{w} is not supported"));
    }
    let (parts, names) = parts(s, false)?;
    let auto = |p: &&Part| {
        matches!(
            p,
            Part::Repeat {
                count: RepetitionCount::AutoFill | RepetitionCount::AutoFit,
                ..
            }
        )
    };
    match parts.iter().filter(auto).count() {
        0 => {}
        1 => {
            let fixed = parts.iter().all(|p| match p {
                Part::Track(t) => t.fixed(),
                Part::Repeat { tracks, .. } => tracks.iter().all(Track::fixed),
            });
            if !fixed {
                return Err(format!(
                    "{s}: beside auto-fill or auto-fit, every track needs a length or a \
                     percentage, at least at one end of minmax()"
                ));
            }
        }
        _ => {
            return Err(format!(
                "{s}: only one repeat() may be auto-fill or auto-fit"
            ));
        }
    }
    Ok(Tracks { parts, names })
}

/// `grid-auto-columns` and `-rows`: track sizes; a number is one track of px.
pub(super) fn auto(v: &V) -> Result<Vec<Track>, String> {
    if let V::Num(_) = v {
        return Ok(vec![Track::len(length(v, TRACK)?)]);
    }
    let s = written(v)?;
    let words = css::words(s);
    if words.is_empty() {
        return Err(format!("{} has no tracks", shown(v)));
    }
    let named = |w: &str| {
        w.starts_with('[')
            || css::function(w).is_some_and(|(f, _)| f.eq_ignore_ascii_case("repeat"))
    };
    if words.iter().any(|w| named(w)) {
        return Err(format!(
            "{s}: implicit tracks take sizes only, without repeat() or line names"
        ));
    }
    words.into_iter().map(track).collect()
}

/// `grid-auto-flow`: `row` or `column`, and `dense`.
pub(super) fn flow(v: &V) -> Result<GridAutoFlow, String> {
    let s = text(v)?;
    let words = css::words(&s);
    let (mut column, mut dense) = (None, false);
    for &w in &words {
        match w {
            "row" | "column" if column.is_none() => column = Some(w == "column"),
            "dense" if !dense => dense = true,
            _ => {
                return Err(format!(
                    "{s} is not row or column, dense, or one of them and dense"
                ));
            }
        }
    }
    Ok(match (column, dense) {
        (None, false) => return Err(format!("{} is not a flow", shown(v))),
        (Some(true), false) => GridAutoFlow::Column,
        (Some(true), true) => GridAutoFlow::ColumnDense,
        (_, true) => GridAutoFlow::RowDense,
        (_, false) => GridAutoFlow::Row,
    })
}

/// `grid-row` and `grid-column`: the start line, and after `/` the end line. Without one the end
/// is the start's name, if the start is a name alone, or `auto`.
pub(super) fn lines(v: &V) -> Result<Line<GridPlacement<String>>, String> {
    let s = match v {
        V::Str(_) => written(v)?,
        _ => {
            return Ok(Line {
                start: line(v)?,
                end: GridPlacement::Auto,
            });
        }
    };
    match s.split('/').collect::<Vec<_>>()[..] {
        [start] => {
            let start = placement(start.trim())?;
            let end = match &start {
                GridPlacement::NamedLine(name, 0) => GridPlacement::NamedLine(name.clone(), 0),
                _ => GridPlacement::Auto,
            };
            Ok(Line { start, end })
        }
        [start, end] => Ok(Line {
            start: placement(start.trim())?,
            end: placement(end.trim())?,
        }),
        _ => Err(format!("{s} takes a start line, and an end line after /")),
    }
}

/// `grid-row-start` and the like: `auto`, a line by its number, its name, or both, or a span of
/// tracks by count or to a line's name.
pub(super) fn line(v: &V) -> Result<GridPlacement<String>, String> {
    match v {
        V::Num(x) => match number(*x, &shown(v))? {
            0 => Err(no_line_0(&shown(v))),
            n => Ok(GridPlacement::Line(n.into())),
        },
        _ => placement(written(v)?),
    }
}

fn placement(s: &str) -> Result<GridPlacement<String>, String> {
    let bad = || {
        format!("{s} is not a grid line: auto, a number, a name, or span with a count or a name")
    };
    let words = css::words(s);
    if let [w] = words[..]
        && w.eq_ignore_ascii_case("auto")
    {
        return Ok(GridPlacement::Auto);
    }
    let (mut span, mut count, mut name) = (false, None, None);
    for w in words {
        if w.eq_ignore_ascii_case("span") && !span {
            span = true;
        } else if let Some(n) = css::num(w) {
            if !n.unit.is_empty() || count.is_some() {
                return Err(bad());
            }
            count = Some(number(n.value, s)?);
        } else if name.is_none() {
            name = Some(self::name(w)?);
        } else {
            return Err(bad());
        }
    }
    Ok(match (span, count, name) {
        (_, None, None) => return Err(bad()),
        (true, Some(n), _) if n < 1 => return Err(format!("{s}: a span is at least 1")),
        (true, n, Some(name)) => GridPlacement::NamedSpan(name, n.unwrap_or(1) as u16),
        (true, n, None) => GridPlacement::Span(n.unwrap_or(1) as u16),
        (false, Some(0), _) => return Err(no_line_0(s)),
        (false, n, Some(name)) => GridPlacement::NamedLine(name, n.unwrap_or(0)),
        (false, Some(n), None) => GridPlacement::Line(n.into()),
    })
}

fn no_line_0(s: &str) -> String {
    format!("{s}: there is no line 0; lines count from 1, and from -1 at the end")
}

/// A line's number, or a span's.
fn number(x: f64, s: &str) -> Result<i16, String> {
    if x.fract() != 0.0 {
        return Err(format!("{s}: {x} is not a whole number"));
    }
    if x.abs() > f64::from(i16::MAX) {
        return Err(format!("{s}: {x} is too many lines"));
    }
    Ok(x as i16)
}

/// A line name: an identifier other than `span`, `auto` and the CSS-wide keywords.
fn name(s: &str) -> Result<String, String> {
    const KEYWORDS: [&str; 8] = [
        "span",
        "auto",
        "initial",
        "inherit",
        "unset",
        "default",
        "revert",
        "revert-layer",
    ];
    if KEYWORDS.iter().any(|k| s.eq_ignore_ascii_case(k)) {
        return Err(format!("{s} is a keyword, not a line name"));
    }
    let char = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_' || !c.is_ascii();
    let first = s.strip_prefix('-').unwrap_or(s).chars().next();
    let starts = first.is_some_and(|c| c == '-' || (char(c) && !c.is_ascii_digit()));
    if !starts || !s.chars().all(char) {
        return Err(format!("{s} is not a line name"));
    }
    Ok(s.to_string())
}

/// A track list's parts as written: `[names]`, and words, which keep functions whole.
enum Token<'a> {
    Names(Vec<&'a str>),
    Word(&'a str),
}

fn tokens(s: &str) -> Result<Vec<Token<'_>>, String> {
    let mut out = Vec::new();
    let mut rest = s.trim_start();
    while !rest.is_empty() {
        if let Some(inner) = rest.strip_prefix('[') {
            let end = inner
                .find(']')
                .ok_or_else(|| format!("{s}: a [ is not closed"))?;
            if inner[..end].contains('[') {
                return Err(format!("{s}: [ ] do not nest"));
            }
            out.push(Token::Names(inner[..end].split_whitespace().collect()));
            rest = inner[end + 1..].trim_start();
            continue;
        }
        // To white space or a `[` outside parentheses.
        let mut depth = 0;
        let end = rest.char_indices().find(|&(_, c)| {
            depth += i32::from(c == '(') - i32::from(c == ')');
            depth == 0 && (c.is_whitespace() || c == '[')
        });
        let end = end.map_or(rest.len(), |(i, _)| i);
        out.push(Token::Word(&rest[..end]));
        rest = rest[end..].trim_start();
    }
    Ok(out)
}

/// The parts of a track list, or of a repetition `inside` one, and the names of the line before
/// each and of the last line.
fn parts(s: &str, inside: bool) -> Result<(Vec<Part>, Vec<Vec<String>>), String> {
    let (mut parts, mut names) = (Vec::new(), Vec::new());
    let mut before: Option<Vec<String>> = None;
    for token in tokens(s)? {
        match token {
            Token::Names(ns) => {
                if before.is_some() {
                    return Err(format!("{s}: put names of one line in one [ ]"));
                }
                before = Some(ns.into_iter().map(name).collect::<Result<_, _>>()?);
            }
            Token::Word(w) => {
                let part = match css::function(w) {
                    Some((f, _)) if f.eq_ignore_ascii_case("repeat") && inside => {
                        return Err(format!("{s}: repeat() does not nest"));
                    }
                    Some((f, args)) if f.eq_ignore_ascii_case("repeat") => repeat(w, args)?,
                    _ => Part::Track(track(w)?),
                };
                names.push(before.take().unwrap_or_default());
                parts.push(part);
            }
        }
    }
    names.push(before.unwrap_or_default());
    if parts.is_empty() {
        return Err(format!("{s:?} has no tracks"));
    }
    Ok((parts, names))
}

/// `repeat(count, tracks)`: a count from 1, `auto-fill` or `auto-fit`.
fn repeat(s: &str, args: &str) -> Result<Part, String> {
    let (count, list) = args
        .split_once(',')
        .ok_or_else(|| format!("{s}: repeat() takes a count and tracks"))?;
    let count = match count.trim().to_ascii_lowercase().as_str() {
        "auto-fill" => RepetitionCount::AutoFill,
        "auto-fit" => RepetitionCount::AutoFit,
        c => match css::num(c) {
            Some(n) if n.unit.is_empty() && n.value.fract() == 0.0 && n.value >= 1.0 => {
                RepetitionCount::Count(n.value.min(f64::from(u16::MAX)) as u16)
            }
            _ => {
                return Err(format!(
                    "{s}: {c} is not a count: a whole number from 1, auto-fill or auto-fit"
                ));
            }
        },
    };
    let (parts, names) = parts(list, true)?;
    let tracks = parts.into_iter().map(|p| match p {
        Part::Track(t) => t,
        Part::Repeat { .. } => unreachable!("repeats do not nest"),
    });
    Ok(Part::Repeat {
        count,
        tracks: tracks.collect(),
        names,
    })
}

/// `<track-size>`: a breadth, `minmax(min, max)` or `fit-content(limit)`. A flexible breadth
/// alone is `minmax(auto, fr)`, any other both ends.
fn track(s: &str) -> Result<Track, String> {
    let s = s.to_ascii_lowercase();
    match css::function(&s) {
        Some(("minmax", args)) => match &css::commas(args)[..] {
            [min, max] => match breadth(min)? {
                Breadth::Fr(_) => Err(format!("{s}: the minimum is not flexible, in fr")),
                min => Ok(Track::MinMax(min, breadth(max)?)),
            },
            _ => Err(format!("{s}: minmax() takes a minimum and a maximum")),
        },
        Some(("fit-content", limit)) => Ok(Track::FitContent(length_text(limit.trim(), TRACK)?)),
        _ => Ok(match breadth(&s)? {
            Breadth::Fr(f) => Track::MinMax(Breadth::Auto, Breadth::Fr(f)),
            b => Track::MinMax(b.clone(), b),
        }),
    }
}

fn breadth(s: &str) -> Result<Breadth, String> {
    Ok(match s {
        "auto" => Breadth::Auto,
        "min-content" => Breadth::MinContent,
        "max-content" => Breadth::MaxContent,
        _ if s.starts_with("fit-content") => {
            return Err(format!(
                "{s}: tracks take fit-content(limit), alone and not in minmax()"
            ));
        }
        _ => match css::num(s) {
            Some(n) if n.unit == "fr" && n.value < 0.0 => return Err(format!("{s} is negative")),
            Some(n) if n.unit == "fr" => Breadth::Fr(n.value),
            _ => Breadth::Len(length_text(s, TRACK)?),
        },
    })
}

impl Lengths {
    /// A track list as Taffy takes it: its parts, and its line names.
    pub(super) fn template(
        &mut self,
        t: &Tracks,
    ) -> (Vec<GridTemplateComponent<String>>, Vec<Vec<String>>) {
        let mut parts = Vec::with_capacity(t.parts.len());
        for p in &t.parts {
            parts.push(match p {
                Part::Track(track) => GridTemplateComponent::Single(self.track(track)),
                Part::Repeat {
                    count,
                    tracks,
                    names,
                } => {
                    let mut sized = Vec::with_capacity(tracks.len());
                    for track in tracks {
                        sized.push(self.track(track));
                    }
                    GridTemplateComponent::Repeat(GridTemplateRepetition {
                        count: *count,
                        tracks: sized,
                        line_names: names.clone(),
                    })
                }
            });
        }
        (parts, t.names.clone())
    }

    pub(super) fn track(&mut self, t: &Track) -> TrackSizingFunction {
        match t {
            Track::MinMax(min, max) => MinMax {
                min: self.min(min),
                max: self.max(max),
            },
            Track::FitContent(l) => MinMax {
                min: Min::auto(),
                max: self.taffy(
                    l,
                    Max::fit_content_px,
                    Max::fit_content_percent,
                    Max::fit_content_calc,
                ),
            },
        }
    }

    fn min(&mut self, b: &Breadth) -> Min {
        match b {
            Breadth::Len(l) => self.taffy(l, Min::length, Min::percent, Min::calc),
            Breadth::MinContent => Min::min_content(),
            Breadth::MaxContent => Min::max_content(),
            Breadth::Auto | Breadth::Fr(_) => Min::auto(),
        }
    }

    fn max(&mut self, b: &Breadth) -> Max {
        match b {
            Breadth::Len(l) => self.taffy(l, Max::length, Max::percent, Max::calc),
            Breadth::Fr(f) => Max::fr(*f as f32),
            Breadth::MinContent => Max::min_content(),
            Breadth::MaxContent => Max::max_content(),
            Breadth::Auto => Max::auto(),
        }
    }
}
