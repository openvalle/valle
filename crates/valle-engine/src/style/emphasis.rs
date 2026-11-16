//! CSS emphasis marks. The shorthand resets mark and color, but preserves position.
use std::rc::Rc;
use unicode_segmentation::UnicodeSegmentation;

use super::{color, css};

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum Mark {
    #[default]
    None,
    Auto(bool),
    Text(Rc<str>),
    Symbol(&'static str),
}

impl Mark {
    pub fn text(&self, vertical: bool) -> Option<&str> {
        match self {
            Self::None => None,
            Self::Text(s) if s.is_empty() => None,
            Self::Text(s) => Some(s),
            Self::Symbol(s) => Some(s),
            Self::Auto(open) => Some(match (vertical, open) {
                (false, false) => "●",
                (false, true) => "○",
                (true, false) => "﹅",
                (true, true) => "﹆",
            }),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Position {
    pub under: bool,
    pub left: bool,
}

pub(super) fn position(s: &str) -> Result<Position, String> {
    let (mut under, mut left) = (None, None);
    for word in css::words(s) {
        match word {
            "over" | "under" if under.is_none() => under = Some(word == "under"),
            "left" | "right" if left.is_none() => left = Some(word == "left"),
            _ => {
                return Err(format!(
                    "text-emphasis-position: {s} needs over or under, optionally left or right"
                ));
            }
        }
    }
    Ok(Position {
        under: under.ok_or("text-emphasis-position needs over or under")?,
        left: left.unwrap_or(false),
    })
}

pub(super) fn style(s: &str) -> Result<Mark, String> {
    let parts = css::words(s);
    if parts == ["none"] {
        return Ok(Mark::None);
    }
    if let [s] = parts.as_slice() {
        for q in ['\'', '"'] {
            if let Some(s) = s.strip_prefix(q).and_then(|s| s.strip_suffix(q)) {
                if s.contains('\\') {
                    return Err(
                        "text-emphasis-style: use the literal mark character, not a string escape"
                            .into(),
                    );
                }
                return Ok(Mark::Text(s.graphemes(true).next().unwrap_or("").into()));
            }
        }
    }
    let (mut open, mut shape) = (None, None);
    for word in parts {
        match word {
            "filled" | "open" if open.is_none() => open = Some(word == "open"),
            "dot" | "circle" | "double-circle" | "triangle" | "sesame" if shape.is_none() => {
                shape = Some(word)
            }
            _ => {
                return Err(format!(
                    "text-emphasis-style: {s} is not none, a quoted mark, or an optional filled/open shape"
                ));
            }
        }
    }
    let open = open.unwrap_or(false);
    let Some(shape) = shape else {
        return if s.trim().is_empty() {
            Err("text-emphasis-style is empty".into())
        } else {
            Ok(Mark::Auto(open))
        };
    };
    let mark = match (shape, open) {
        ("dot", false) => "•",
        ("dot", true) => "◦",
        ("circle", false) => "●",
        ("circle", true) => "○",
        ("double-circle", false) => "◉",
        ("double-circle", true) => "◎",
        ("triangle", false) => "▲",
        ("triangle", true) => "△",
        ("sesame", false) => "﹅",
        ("sesame", true) => "﹆",
        _ => unreachable!(),
    };
    Ok(Mark::Symbol(mark))
}

pub(super) fn shorthand(s: &str) -> Result<(Mark, Option<color::Specified>), String> {
    let (mut words, mut paint) = (Vec::new(), None);
    for word in css::words(s) {
        let color = if word.eq_ignore_ascii_case("currentcolor") {
            Ok(None)
        } else {
            color::Specified::parse(word).map(Some)
        };
        if let Ok(c) = color {
            if paint.replace(c).is_some() {
                return Err("text-emphasis has more than one color".into());
            }
        } else {
            words.push(word);
        }
    }
    let mark = if words.is_empty() {
        Mark::None
    } else {
        style(&words.join(" "))?
    };
    if words.is_empty() && paint.is_none() {
        return Err("text-emphasis is empty".into());
    }
    Ok((mark, paint.flatten()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_accept_shapes_and_one_grapheme() {
        assert_eq!(style("open").unwrap().text(false), Some("○"));
        assert_eq!(style("open").unwrap().text(true), Some("﹆"));
        assert_eq!(style("dot open").unwrap().text(false), Some("◦"));
        assert_eq!(style("'árest'").unwrap().text(false), Some("á"));
        assert!(style("''").unwrap().text(false).is_none());
        for s in ["", "none dot", "open filled", "circle dot", "star"] {
            assert!(style(s).is_err(), "{s}");
        }
    }

    #[test]
    fn every_shape_has_its_filled_and_open_mark() {
        // CSS Text Decoration 3's marks, by shape: filled then open.
        let marks = [
            ("dot", "•", "◦"),
            ("circle", "●", "○"),
            ("double-circle", "◉", "◎"),
            ("triangle", "▲", "△"),
            ("sesame", "﹅", "﹆"),
        ];
        for (shape, filled, open) in marks {
            for vertical in [false, true] {
                let mark = |s: &str| style(s).unwrap().text(vertical).map(str::to_string);
                assert_eq!(mark(shape).as_deref(), Some(filled), "{shape}");
                assert_eq!(mark(&format!("filled {shape}")).as_deref(), Some(filled));
                assert_eq!(mark(&format!("{shape} open")).as_deref(), Some(open));
            }
        }
        // With no shape, circles in horizontal text and sesames in vertical text.
        let auto = |s: &str, vertical| style(s).unwrap().text(vertical).map(str::to_string);
        assert_eq!(auto("filled", false).as_deref(), Some("●"));
        assert_eq!(auto("filled", true).as_deref(), Some("﹅"));
        assert_eq!(style("none").unwrap(), Mark::None);
        assert!(style("none").unwrap().text(false).is_none());
        assert!(style("'\\25CF'").unwrap_err().contains("literal mark"));
        assert!(style("  ").unwrap_err().contains("empty"));
    }

    #[test]
    fn shorthand_and_position_reset_independently() {
        assert_eq!(
            shorthand("red open dot").unwrap().0,
            style("open dot").unwrap()
        );
        assert_eq!(shorthand("dot currentColor").unwrap().1, None);
        assert_eq!(shorthand("red").unwrap().0, Mark::None);
        assert!(shorthand("red blue dot").is_err());
        assert_eq!(
            position("left under").unwrap(),
            Position {
                under: true,
                left: true
            }
        );
        assert_eq!(position("over").unwrap(), Position::default());
        for s in ["left", "over under", "right left over"] {
            assert!(position(s).is_err());
        }
    }
}
