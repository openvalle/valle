//! CSS shape() commands retain lengths until the reference box is known. Their output
//! uses the same path segments and arc conversion as SVG and clip-path: path().

use super::LENGTH;
use crate::path::{self, Path, Point as XY, Segment};
use crate::style::math::{self, Calc};
use crate::style::position::{Offset, position};
use crate::style::units::{Len, Units, length_text};
use crate::style::{WritingMode, css};

#[derive(Clone, Copy, Debug, PartialEq)]
enum Origin {
    Box,
    Start,
    End,
}

#[derive(Clone, Debug, PartialEq)]
struct Point<P> {
    at: [Offset<P>; 2],
    origin: Origin,
}

#[derive(Clone, Debug, PartialEq)]
enum Command<P> {
    Move(Point<P>),
    Line(Point<P>),
    Axis {
        axis: usize,
        relative: bool,
        logical: bool,
        by: P,
    },
    Quad(Point<P>, Point<P>),
    Cubic(Point<P>, Point<P>, Point<P>),
    SmoothQuad(Point<P>),
    SmoothCubic(Point<P>, Point<P>),
    Arc {
        to: Point<P>,
        radii: Vec<P>,
        degrees: f32,
        large: bool,
        clockwise: bool,
    },
    Close,
}

/// A CSS path whose coordinates can depend on its box and computed font units.
#[derive(Clone, Debug, PartialEq)]
pub struct Commands<P = Calc> {
    start: [Offset<P>; 2],
    commands: Vec<Command<P>>,
}

fn map_at<P, Q>(at: &[Offset<P>; 2], f: &impl Fn(&P) -> Q) -> [Offset<Q>; 2] {
    at.each_ref().map(|o| Offset {
        end: o.end,
        by: f(&o.by),
    })
}

impl Commands<Len> {
    pub(super) fn computed(&self, u: Units, writing: WritingMode) -> Commands {
        let calc = |l: &Len| l.calc(u);
        let point = |p: &Point<Len>| Point {
            at: map_at(&p.at, &calc),
            origin: p.origin,
        };
        Commands {
            start: map_at(&self.start, &calc),
            commands: self
                .commands
                .iter()
                .map(|c| match c {
                    Command::Move(p) => Command::Move(point(p)),
                    Command::Line(p) => Command::Line(point(p)),
                    Command::Axis {
                        axis,
                        relative,
                        logical,
                        by,
                    } => Command::Axis {
                        axis: *axis,
                        relative: *relative,
                        logical: false,
                        by: if *logical && *axis == 0 && writing == WritingMode::VerticalRl {
                            Calc::Sum([0.0, 1.0 - by.px(u, 1.0)])
                        } else {
                            calc(by)
                        },
                    },
                    Command::Quad(c, p) => Command::Quad(point(c), point(p)),
                    Command::Cubic(a, b, p) => Command::Cubic(point(a), point(b), point(p)),
                    Command::SmoothQuad(p) => Command::SmoothQuad(point(p)),
                    Command::SmoothCubic(c, p) => Command::SmoothCubic(point(c), point(p)),
                    Command::Arc {
                        to,
                        radii,
                        degrees,
                        large,
                        clockwise,
                    } => Command::Arc {
                        to: point(to),
                        radii: radii.iter().map(calc).collect(),
                        degrees: *degrees,
                        large: *large,
                        clockwise: *clockwise,
                    },
                    Command::Close => Command::Close,
                })
                .collect(),
        }
    }
}

impl Point<Calc> {
    fn resolve(&self, size: XY, start: XY, end: XY) -> XY {
        let origin = match self.origin {
            Origin::Box => [0.0; 2],
            Origin::Start => start,
            Origin::End => end,
        };
        [0, 1].map(|k| origin[k] + self.at[k].resolve(size[k]))
    }
}

impl Commands {
    /// Resolve all percentages in the reference box, before the element's transform.
    pub(crate) fn path(&self, size: XY) -> Path {
        let mut at = [0, 1].map(|k| self.start[k].resolve(size[k]));
        let mut start = at;
        let mut segments = vec![Segment::Move(at)];
        let (mut quad, mut cubic) = (None, None);
        for c in &self.commands {
            let reflect =
                |p: Option<XY>| p.map_or(at, |p| [2.0 * at[0] - p[0], 2.0 * at[1] - p[1]]);
            let endpoint = |p: &Point<Calc>| p.resolve(size, at, at);
            let (mut next_quad, mut next_cubic) = (None, None);
            match c {
                Command::Move(p) => {
                    at = endpoint(p);
                    start = at;
                    segments.push(Segment::Move(at));
                }
                Command::Line(p) => {
                    at = endpoint(p);
                    segments.push(Segment::Line(at));
                }
                Command::Axis {
                    axis, relative, by, ..
                } => {
                    at[*axis] = by.resolve(size[*axis]) + if *relative { at[*axis] } else { 0.0 };
                    segments.push(Segment::Line(at));
                }
                Command::Quad(control, p) => {
                    let end = endpoint(p);
                    let control = control.resolve(size, at, end);
                    segments.push(Segment::Quad(control, end));
                    next_quad = Some(control);
                    at = end;
                }
                Command::Cubic(a, b, p) => {
                    let end = endpoint(p);
                    let a = a.resolve(size, at, end);
                    let b = b.resolve(size, at, end);
                    segments.push(Segment::Cubic(a, b, end));
                    next_cubic = Some(b);
                    at = end;
                }
                Command::SmoothQuad(p) => {
                    let end = endpoint(p);
                    let control = reflect(quad);
                    segments.push(Segment::Quad(control, end));
                    next_quad = Some(control);
                    at = end;
                }
                Command::SmoothCubic(b, p) => {
                    let end = endpoint(p);
                    let a = reflect(cubic);
                    let b = b.resolve(size, at, end);
                    segments.push(Segment::Cubic(a, b, end));
                    next_cubic = Some(b);
                    at = end;
                }
                Command::Arc {
                    to,
                    radii,
                    degrees,
                    large,
                    clockwise,
                } => {
                    let end = endpoint(to);
                    let radii = match radii.as_slice() {
                        [r] => [r.resolve(size[0].hypot(size[1]) / std::f32::consts::SQRT_2); 2],
                        [x, y] => [x.resolve(size[0]), y.resolve(size[1])],
                        _ => unreachable!("parsed arcs have one or two radii"),
                    };
                    path::arc(&mut segments, at, radii, *degrees, *large, *clockwise, end);
                    at = end;
                }
                Command::Close => {
                    segments.push(Segment::Close);
                    at = start;
                }
            }
            (quad, cubic) = (next_quad, next_cubic);
        }
        Path(segments)
    }
}

pub(super) fn parse(s: &str) -> Result<(bool, Commands<Len>), String> {
    let parts = css::commas(s);
    let Some((head, tail)) = parts.split_first().filter(|(_, tail)| !tail.is_empty()) else {
        return Err("takes from and a position, then comma-separated commands".into());
    };
    let mut head = css::words(head);
    let even_odd = match head.first().copied() {
        Some("evenodd" | "nonzero") => head.remove(0) == "evenodd",
        _ => false,
    };
    let Some(("from", start)) = head.split_first().map(|(a, b)| (*a, b)) else {
        return Err("must start with from and a position".into());
    };
    Ok((
        even_odd,
        Commands {
            start: position(start, false)?,
            commands: tail.iter().map(|s| command(s)).collect::<Result<_, _>>()?,
        },
    ))
}

fn pair(words: &[&str]) -> Result<[Offset<Len>; 2], String> {
    let [x, y] = words else {
        return Err("relative coordinates take two lengths or percentages".into());
    };
    Ok([
        Offset {
            end: false,
            by: length_text(x, LENGTH)?,
        },
        Offset {
            end: false,
            by: length_text(y, LENGTH)?,
        },
    ])
}

fn point(words: &[&str], relative: bool) -> Result<Point<Len>, String> {
    Ok(Point {
        at: if relative {
            pair(words)?
        } else {
            position(words, false)?
        },
        origin: if relative { Origin::Start } else { Origin::Box },
    })
}

fn control(words: &[&str], relative: bool) -> Result<Point<Len>, String> {
    if let Some(i) = words.iter().position(|w| *w == "from") {
        let origin = match words[i + 1..] {
            ["start"] => Origin::Start,
            ["end"] => Origin::End,
            ["origin"] => Origin::Box,
            _ => return Err("a control point's from takes start, end or origin".into()),
        };
        Ok(Point {
            at: pair(&words[..i])?,
            origin,
        })
    } else {
        point(words, relative)
    }
}

fn command(s: &str) -> Result<Command<Len>, String> {
    let words = css::words(s);
    if words == ["close"] {
        return Ok(Command::Close);
    }
    let [name, mode, rest @ ..] = words.as_slice() else {
        return Err(format!("{s:?} is not a shape command"));
    };
    let relative = match *mode {
        "to" => false,
        "by" => true,
        _ => return Err(format!("{name} takes to or by")),
    };
    match *name {
        "move" => Ok(Command::Move(point(rest, relative)?)),
        "line" => Ok(Command::Line(point(rest, relative)?)),
        "hline" | "vline" => {
            let [word] = rest else {
                return Err(format!("{name} takes one coordinate"));
            };
            let axis = usize::from(*name == "vline");
            let by = match (*word, relative, axis) {
                ("left" | "x-start", false, 0) | ("top" | "y-start", false, 1) => Len::Percent(0.0),
                ("right" | "x-end", false, 0) | ("bottom" | "y-end", false, 1) => {
                    Len::Percent(100.0)
                }
                ("center", false, _) => Len::Percent(50.0),
                _ => length_text(word, LENGTH)?,
            };
            Ok(Command::Axis {
                axis,
                relative,
                logical: matches!(*word, "x-start" | "x-end" | "y-start" | "y-end"),
                by,
            })
        }
        "curve" | "smooth" => {
            let i = rest.iter().position(|w| *w == "with");
            let end = point(&rest[..i.unwrap_or(rest.len())], relative)?;
            let controls = i.map(|i| rest[i + 1..].join(" "));
            if controls.as_deref() == Some("") {
                return Err("with needs a control point".into());
            }
            let controls = controls.as_deref().map(css::slashes).unwrap_or_default();
            let controls = controls
                .iter()
                .map(|s| control(&css::words(s), relative))
                .collect::<Result<Vec<_>, _>>()?;
            match (*name, controls.as_slice()) {
                ("curve", [a]) => Ok(Command::Quad(a.clone(), end)),
                ("curve", [a, b]) => Ok(Command::Cubic(a.clone(), b.clone(), end)),
                ("smooth", []) => Ok(Command::SmoothQuad(end)),
                ("smooth", [b]) => Ok(Command::SmoothCubic(b.clone(), end)),
                _ => Err(format!(
                    "{name} takes {} control points after with",
                    if *name == "curve" {
                        "one or two"
                    } else {
                        "one or no"
                    }
                )),
            }
        }
        "arc" => arc(rest, relative),
        _ => Err(format!(
            "{name} is not a shape command: move, line, hline, vline, curve, smooth, arc or close"
        )),
    }
}

fn arc(words: &[&str], relative: bool) -> Result<Command<Len>, String> {
    let option = |s: &str| matches!(s, "of" | "cw" | "ccw" | "large" | "small" | "rotate");
    let i = words.iter().position(|w| option(w)).unwrap_or(words.len());
    let to = point(&words[..i], relative)?;
    let (mut radii, mut clockwise, mut large, mut degrees) = (None, None, None, None);
    let mut words = &words[i..];
    while let Some((word, tail)) = words.split_first() {
        words = tail;
        let duplicate = match *word {
            "of" => {
                let n = words.iter().position(|w| option(w)).unwrap_or(words.len());
                if !(1..=2).contains(&n) {
                    return Err("arc of takes one or two radii".into());
                }
                let r = words[..n]
                    .iter()
                    .map(|w| length_text(w, LENGTH))
                    .collect::<Result<Vec<_>, _>>()?;
                words = &words[n..];
                radii.replace(r).is_some()
            }
            "cw" | "ccw" => clockwise.replace(*word == "cw").is_some(),
            "large" | "small" => large.replace(*word == "large").is_some(),
            "rotate" => {
                let Some((angle, tail)) = words.split_first() else {
                    return Err("arc rotate needs an angle".into());
                };
                words = tail;
                let angle = math::degrees(angle)?;
                degrees.replace(angle as f32).is_some()
            }
            _ => {
                return Err(format!(
                    "{word} is not an arc option: of, cw, ccw, large, small or rotate"
                ));
            }
        };
        if duplicate {
            return Err(format!("arc sets {word} twice"));
        }
    }
    Ok(Command::Arc {
        to,
        radii: radii.ok_or("arc needs of and one or two radii")?,
        clockwise: clockwise.unwrap_or(false),
        large: large.unwrap_or(false),
        degrees: degrees.unwrap_or(0.0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commands(s: &str) -> Commands {
        parse(s).unwrap().1.computed(
            Units {
                em: 20.0,
                canvas: [800.0, 600.0],
            },
            WritingMode::Horizontal,
        )
    }

    #[test]
    fn commands_match_svg_with_absolute_relative_and_smooth_curves() {
        let shape = commands(
            "from 10px 20px, line by 20px 0, hline by 10px, vline to 40px, curve by 20px 20px with 5px 0, smooth by 20px -20px, curve to 120px 40px with 90px 10px / 110px 10px, smooth by 30px 0 with 20px 20px, close, move by 0 60px, arc by 80px 0 of 50px 30px cw large rotate 20deg, close",
        );
        let svg=path::parse("M10 20 l20 0 h10 V40 q5 0 20 20 t20 -20 C90 10 110 10 120 40 s20 20 30 0 z m0 60 a50 30 20 1 1 80 0 z").unwrap();
        assert_eq!(shape.path([200.0, 150.0]), svg);
    }

    #[test]
    fn positions_and_control_origins_resolve_against_the_live_box() {
        let shape = commands(
            "from left 1em, curve to right bottom with 10px 5px from start / -10% -5px from end, line by calc(-50% + 2px) -10px, hline to center, vline to y-start, close",
        );
        assert_eq!(
            shape.path([200.0, 100.0]),
            Path(vec![
                Segment::Move([0.0, 20.0]),
                Segment::Cubic([10.0, 25.0], [180.0, 95.0], [200.0, 100.0]),
                Segment::Line([102.0, 90.0]),
                Segment::Line([100.0, 90.0]),
                Segment::Line([100.0, 0.0]),
                Segment::Close,
            ])
        );
        assert_eq!(
            shape.path([400.0, 200.0]).0[1],
            Segment::Cubic([10.0, 25.0], [360.0, 195.0], [400.0, 200.0])
        );
        let relative = commands(
            "from 20px 30px, curve by 80px 40px with 10px 20px from origin, curve by 20px 10px with -5px 0 from end",
        );
        assert_eq!(
            relative.path([200.0, 100.0]).0[1..],
            [
                Segment::Quad([10.0, 20.0], [100.0, 70.0]),
                Segment::Quad([115.0, 80.0], [120.0, 80.0])
            ]
        );
    }

    #[test]
    fn single_percentage_arc_uses_the_normalized_diagonal() {
        let one = commands("from 10px 40px, arc to 110px 40px of 50% cw").path([120.0, 160.0]);
        let two = commands("from 10px 40px, arc to 110px 40px of 50% 50% cw").path([120.0, 160.0]);
        let mut expected = vec![Segment::Move([10.0, 40.0])];
        path::arc(
            &mut expected,
            [10.0, 40.0],
            [100.0 / std::f32::consts::SQRT_2; 2],
            0.0,
            false,
            true,
            [110.0, 40.0],
        );
        assert_eq!(one, Path(expected));
        assert_ne!(one, two);
        assert_eq!(
            commands("from 0 0, arc by 50px 0 of 0 cw")
                .path([100.0; 2])
                .0,
            [Segment::Move([0.0; 2]), Segment::Line([50.0, 0.0])]
        );
        assert_eq!(
            commands("from 0 0, arc by 50px 0 of -40px rotate calc(1turn / 4)").path([100.0; 2]),
            commands("from 0 0, arc by 50px 0 rotate 90deg of 40px ccw small").path([100.0; 2])
        );
    }

    #[test]
    fn smooth_controls_reset_after_lines_moves_and_different_curve_types() {
        let shape = commands(
            "from 0 0, curve by 20px 0 with 10px 20px, line by 5px 0, smooth by 20px 0, curve by 20px 0 with 5px 10px / 15px 10px, smooth by 20px 0, move to 0 40px, smooth by 20px 0 with 10px 20px",
        );
        assert_eq!(
            shape.path([100.0; 2]),
            path::parse("M0 0 q10 20 20 0 l5 0 t20 0 c5 10 15 10 20 0 t20 0 M0 40 s10 20 20 0")
                .unwrap()
        );
    }

    #[test]
    fn axis_start_and_end_follow_vertical_column_direction() {
        let path = parse("from 20px 10px, hline to x-start, hline to x-end, vline to y-end")
            .unwrap()
            .1;
        let u = Units {
            em: 16.0,
            canvas: [200.0, 100.0],
        };
        let rl = path
            .computed(u, WritingMode::VerticalRl)
            .path([200.0, 100.0]);
        let lr = path
            .computed(u, WritingMode::VerticalLr)
            .path([200.0, 100.0]);
        assert_eq!(
            rl.0[1..],
            [
                Segment::Line([200.0, 10.0]),
                Segment::Line([0.0, 10.0]),
                Segment::Line([0.0, 100.0])
            ]
        );
        assert_eq!(
            lr.0[1..],
            [
                Segment::Line([0.0, 10.0]),
                Segment::Line([200.0, 10.0]),
                Segment::Line([200.0, 100.0])
            ]
        );
    }

    #[test]
    fn grammar_rejects_missing_coordinates_controls_and_duplicate_arc_options() {
        for s in [
            "",
            "from 0 0",
            "from 0 0,",
            "from 0 0 line to 10px 10px",
            "from 0 0, line by left top",
            "from 0 0, hline to bottom",
            "from 0 0, vline by top",
            "from 0 0, curve to center",
            "from 0 0, smooth to center with",
            "from 0 0, curve by 10px 20px with 10px 0 /",
            "from 0 0, curve to center with left top from end",
            "from 0 0, arc to center",
            "from 0 0, arc to center of 10px cw ccw",
            "from 0 0, arc to center of 10px small large",
            "from 0 0, arc to center of 10px rotate 0 rotate 0",
            "from 0 0, line to 4 20px",
            "from 0 0, close to 0 0",
        ] {
            assert!(parse(s).is_err(), "accepted {s}");
        }
        assert!(
            parse("evenodd from right bottom, move to center, line to left top, close")
                .unwrap()
                .0
        );
    }
}
