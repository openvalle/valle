//! CSS grid's own values: track lists with the names of the lines between their tracks, named
//! areas, and the lines items go between. Names are case-sensitive, so these read values as
//! written; keywords, units and functions are not.

use taffy::{
    GridAutoFlow, GridPlacement, GridTemplateArea, GridTemplateAreas, GridTemplateComponent,
    GridTemplateRepetition, Line, MaxTrackSizingFunction as Max, MinMax,
    MinTrackSizingFunction as Min, RepetitionCount, TrackSizingFunction,
};

use super::{Accept, Len, Lengths, css, length, length_text, math, shown, text};
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

/// Named areas: each a name and its lines, from 1.
pub(super) type Areas = GridTemplateAreas<String>;

/// A line's placement.
type Place = GridPlacement<String>;

/// `grid-template-columns` and `-rows`: `none`, or a track list; a number is one track of px.
pub(super) fn template(v: &V) -> Result<Tracks, String> {
    if let V::Num(_) = v {
        return Ok(Tracks {
            parts: vec![Part::Track(Track::len(length(v, TRACK)?))],
            names: vec![Vec::new(); 2],
        });
    }
    tracks(written(v)?)
}

fn tracks(s: &str) -> Result<Tracks, String> {
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

/// `grid-row` and `grid-column`: the start line, and after `/` the end line.
pub(super) fn lines(v: &V) -> Result<Line<Place>, String> {
    let [start, end] = match &placements(v, 2)?[..] {
        [start] => [start.clone(), named(start)],
        [start, end] => [start.clone(), end.clone()],
        _ => unreachable!("one or two"),
    };
    Ok(Line { start, end })
}

/// `grid-area`: the row start, the column start, the row end and the column end, between `/`.
/// A start left out is the row start's name, and an end its start's.
pub(super) fn area(v: &V) -> Result<[Line<Place>; 2], String> {
    let lines = placements(v, 4)?;
    let row_start = lines[0].clone();
    let column_start = lines.get(1).cloned().unwrap_or_else(|| named(&row_start));
    let row_end = lines.get(2).cloned().unwrap_or_else(|| named(&row_start));
    let column_end = lines
        .get(3)
        .cloned()
        .unwrap_or_else(|| named(&column_start));
    Ok([
        Line {
            start: row_start,
            end: row_end,
        },
        Line {
            start: column_start,
            end: column_end,
        },
    ])
}

/// One to `most` lines between `/`.
fn placements(v: &V, most: usize) -> Result<Vec<Place>, String> {
    let s = match v {
        V::Str(_) => written(v)?,
        _ => return Ok(vec![line(v)?]),
    };
    let lines = css::slashes(s);
    if lines.len() > most {
        return Err(format!("{s} takes at most {most} lines, between /"));
    }
    match lines[..] {
        [] => Ok(vec![placement(s)?]),
        _ => lines.into_iter().map(placement).collect(),
    }
}

/// What a line left out of a shorthand is: the name of `given` if it is a name alone, or `auto`.
fn named(given: &Place) -> Place {
    match given {
        GridPlacement::NamedLine(name, 0) => GridPlacement::NamedLine(name.clone(), 0),
        _ => GridPlacement::Auto,
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
        } else if math::is_math(w) && count.is_none() {
            // CSS rounds a math function to the nearest integer, halves up.
            count = Some(number((math::number(w)? + 0.5).floor(), s)?);
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
    let first = s.strip_prefix('-').unwrap_or(s).chars().next();
    let starts = first.is_some_and(|c| c == '-' || (in_name(c) && !c.is_ascii_digit()));
    if !starts || !s.chars().all(in_name) {
        return Err(format!("{s} is not a line name"));
    }
    Ok(s.to_string())
}

/// A track list's parts as written: `[names]`, strings, and words, which keep functions whole.
enum Token<'a> {
    Names(Vec<&'a str>),
    Str(&'a str),
    Word(&'a str),
}

fn tokens(s: &str) -> Result<Vec<Token<'_>>, String> {
    let mut out = Vec::new();
    let mut rest = s.trim_start();
    while let Some(c) = rest.chars().next() {
        if c == '"' || c == '\'' {
            let inner = &rest[1..];
            let end = inner
                .find(c)
                .ok_or_else(|| format!("{s}: a string is not closed"))?;
            out.push(Token::Str(&inner[..end]));
            rest = inner[end + 1..].trim_start();
            continue;
        }
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
        // To white space, a `[` or a quote outside parentheses.
        let mut depth = 0;
        let end = rest.char_indices().find(|&(_, c)| {
            depth += i32::from(c == '(') - i32::from(c == ')');
            depth == 0 && (c.is_whitespace() || matches!(c, '[' | '"' | '\''))
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
            Token::Str(_) => {
                return Err(format!(
                    "{s}: strings of areas go in grid-template-areas, or grid-template"
                ));
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

/// `grid-template-areas`: `none`, or a string for each row, of the names of the areas its cells
/// are in, and `.` for cells in none.
pub(super) fn areas(v: &V) -> Result<Option<Areas>, String> {
    let s = written(v)?;
    if s.eq_ignore_ascii_case("none") {
        return Ok(None);
    }
    let mut rows = Vec::new();
    for token in tokens(s)? {
        match token {
            Token::Str(row) => rows.push(row),
            _ => return Err(format!("{s}: each row is a string")),
        }
    }
    named_areas(s, &rows).map(Some)
}

/// The areas that `rows` name, which must each be a rectangle.
fn named_areas(s: &str, rows: &[&str]) -> Result<Areas, String> {
    let grid: Vec<Vec<Option<&str>>> = (rows.iter())
        .map(|row| cells(s, row))
        .collect::<Result<_, _>>()?;
    let columns = grid.first().map_or(0, Vec::len);
    if columns == 0 || grid.iter().any(|row| row.len() != columns) {
        return Err(format!(
            "{s}: every row needs as many cells as the others, and one at least"
        ));
    }
    let mut areas: Vec<GridTemplateArea<String>> = Vec::new();
    for (r, row) in (1..).zip(&grid) {
        for (c, cell) in (1..).zip(row) {
            let Some(name) = *cell else { continue };
            match areas.iter_mut().find(|a| a.name == name) {
                Some(a) => {
                    a.row_end = a.row_end.max(r + 1);
                    a.column_start = a.column_start.min(c);
                    a.column_end = a.column_end.max(c + 1);
                }
                None => areas.push(GridTemplateArea {
                    name: name.to_string(),
                    row_start: r,
                    row_end: r + 1,
                    column_start: c,
                    column_end: c + 1,
                }),
            }
        }
    }
    for a in &areas {
        let rows = &grid[usize::from(a.row_start) - 1..usize::from(a.row_end) - 1];
        let span = usize::from(a.column_start) - 1..usize::from(a.column_end) - 1;
        if rows.iter().any(|row| {
            row[span.clone()]
                .iter()
                .any(|&c| c != Some(a.name.as_str()))
        }) {
            return Err(format!("{s}: area {} is not a rectangle", a.name));
        }
    }
    Ok(Areas {
        areas,
        row_count: grid.len() as u16,
        column_count: columns as u16,
    })
}

/// Whether names, of lines or areas, may have `c` in them.
fn in_name(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_' || !c.is_ascii()
}

/// A row's cells: names, and `None` for runs of `.`.
fn cells<'a>(s: &str, row: &'a str) -> Result<Vec<Option<&'a str>>, String> {
    let run = |rest: &str, f: &dyn Fn(char) -> bool| rest.find(|c| !f(c)).unwrap_or(rest.len());
    let mut out = Vec::new();
    let mut rest = row;
    while let Some(c) = rest.chars().next() {
        let n = if c.is_whitespace() {
            run(rest, &char::is_whitespace)
        } else if c == '.' {
            out.push(None);
            run(rest, &|c| c == '.')
        } else if in_name(c) {
            let n = run(rest, &in_name);
            out.push(Some(&rest[..n]));
            n
        } else {
            return Err(format!("{s}: {c} is not in a name"));
        };
        rest = &rest[n..];
    }
    Ok(out)
}

/// `grid-template`: `none`; rows and columns, between `/`; or rows each as names, a string of
/// areas, a size and names, then columns after `/` without `repeat()`.
pub(super) fn template_all(v: &V) -> Result<([Tracks; 2], Option<Areas>), String> {
    let s = written(v)?;
    if s.eq_ignore_ascii_case("none") {
        return Ok(Default::default());
    }
    let halves = css::slashes(s);
    if !s.contains(['"', '\'']) {
        return match halves[..] {
            [rows, columns] => Ok(([tracks(rows)?, tracks(columns)?], None)),
            _ => Err(format!("{s} takes rows / columns, or rows with areas")),
        };
    }
    let (rows, columns) = match halves[..] {
        [rows] => (rows, None),
        [rows, columns] => (rows, Some(columns)),
        _ => return Err(format!("{s} takes rows with areas, then / and columns")),
    };
    let tokens = tokens(rows)?;
    let mut at = 0;
    let names_at = |at: &mut usize| -> Result<Vec<String>, String> {
        let Some(Token::Names(ns)) = tokens.get(*at) else {
            return Ok(Vec::new());
        };
        *at += 1;
        ns.iter().map(|n| name(n)).collect()
    };
    let (mut parts, mut names, mut strings) = (Vec::new(), vec![Vec::new()], Vec::new());
    while at < tokens.len() {
        let before = names_at(&mut at)?;
        names.last_mut().expect("a line before").extend(before);
        let Some(Token::Str(row)) = tokens.get(at) else {
            return Err(format!(
                "{s}: each row is names, a string of areas, a size and names"
            ));
        };
        strings.push(*row);
        at += 1;
        let size = match tokens.get(at) {
            Some(Token::Word(w)) => {
                at += 1;
                track(w)?
            }
            _ => Track::MinMax(Breadth::Auto, Breadth::Auto),
        };
        parts.push(Part::Track(size));
        names.push(names_at(&mut at)?);
    }
    let columns = match columns {
        None => Tracks::default(),
        Some(columns) => {
            let (parts, names) = self::parts(columns, false)?;
            if parts.iter().any(|p| matches!(p, Part::Repeat { .. })) {
                return Err(format!("{s}: columns with areas take no repeat()"));
            }
            Tracks { parts, names }
        }
    };
    let areas = named_areas(s, &strings)?;
    Ok(([Tracks { parts, names }, columns], Some(areas)))
}

/// What the `grid` shorthand sets: a template, or tracks that items flow into.
#[derive(Default)]
pub(super) struct Grid {
    pub template: [Tracks; 2],
    pub areas: Option<Areas>,
    pub auto: [Vec<Track>; 2],
    pub flow: GridAutoFlow,
}

/// `grid`: a `grid-template`; or rows, then after `/` `auto-flow`, `dense` and the size of the
/// columns items flow into; or the other way round.
pub(super) fn grid(v: &V) -> Result<Grid, String> {
    let s = written(v)?;
    let flows = |half: &str| (css::words(half).iter()).any(|w| w.eq_ignore_ascii_case("auto-flow"));
    Ok(match css::slashes(s)[..] {
        [rows, columns] if flows(columns) => {
            let (dense, sizes) = auto_flow(columns)?;
            Grid {
                template: [tracks(rows)?, Tracks::default()],
                auto: [Vec::new(), sizes],
                flow: match dense {
                    true => GridAutoFlow::ColumnDense,
                    false => GridAutoFlow::Column,
                },
                ..Grid::default()
            }
        }
        [rows, columns] if flows(rows) => {
            let (dense, sizes) = auto_flow(rows)?;
            Grid {
                template: [Tracks::default(), tracks(columns)?],
                auto: [sizes, Vec::new()],
                flow: match dense {
                    true => GridAutoFlow::RowDense,
                    false => GridAutoFlow::Row,
                },
                ..Grid::default()
            }
        }
        _ => {
            let (template, areas) = template_all(v)?;
            Grid {
                template,
                areas,
                ..Grid::default()
            }
        }
    })
}

/// `auto-flow` and `dense`, then sizes; whether it is dense, and the sizes.
fn auto_flow(s: &str) -> Result<(bool, Vec<Track>), String> {
    let words = css::words(s);
    let (mut flow, mut dense, mut at) = (false, false, 0);
    for w in &words {
        if w.eq_ignore_ascii_case("auto-flow") && !flow {
            flow = true;
        } else if w.eq_ignore_ascii_case("dense") && !dense {
            dense = true;
        } else {
            break;
        }
        at += 1;
    }
    if !flow {
        return Err(format!("{s}: auto-flow and dense come before the sizes"));
    }
    let sizes = words[at..].iter().map(|w| track(w));
    Ok((dense, sizes.collect::<Result<_, _>>()?))
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
