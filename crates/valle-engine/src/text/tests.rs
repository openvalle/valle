use std::borrow::Cow;
use std::rc::Rc;

use super::*;

fn text() -> Text {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fonts/NotoSans-Variable.ttf"
    );
    Text::new(Blob::from(std::fs::read(path).unwrap())).unwrap()
}

/// `text`, all in its paragraph's own font.
fn plain<'a>(text: &'a str, font: &'a Font) -> Content<'a> {
    Content {
        font,
        pieces: vec![Piece {
            text: Cow::Borrowed(text),
            node: 0,
            font,
        }],
    }
}

fn font(size: f64) -> Font {
    Font {
        size,
        weight: 400.0,
        family: Rc::new([Family::SansSerif]),
        line_height: LineHeight::Normal,
        align: TextAlign::Start,
        spacing: 0.0,
        word_spacing: 0.0,
        collapse: Collapse::Collapse,
        wrap: true,
        transform: TextTransform::None,
        fill: None,
        stroke: 0.0,
        stroke_color: None,
        shadows: Rc::new([]),
        skip_ink: true,
        underline_offset: None,
    }
}

#[test]
fn lines_stack_as_chrome_stacks_them() {
    let mut t = text();
    // Noto Sans: ascent 1.069 em and descent 0.293, which at 16px round to 17 and 5.
    let normal = font(16.0);
    assert_eq!(t.measure(&plain("Hello", &normal), None).unwrap()[1], 22.0);
    let lines = t.lines(&plain("Hello", &normal), 100.0).unwrap();
    assert_eq!(lines.boxes, [[0.0, 22.0]]);
    assert!(lines.glyphs.iter().all(|g| g.y == 17.0));

    // Half the leading, floored, goes above the ascent.
    let tall = Font {
        line_height: LineHeight::Factor(1.5),
        ..font(16.0)
    };
    let lines = t.lines(&plain("Hello", &tall), 100.0).unwrap();
    assert_eq!(lines.boxes, [[0.0, 24.0]]);
    assert_eq!(lines.glyphs[0].y, 18.0);
    let tight = Font {
        line_height: LineHeight::Px(18.0),
        ..font(16.0)
    };
    let lines = t.lines(&plain("one two", &tight), 0.0).unwrap();
    assert_eq!(lines.boxes, [[0.0, 18.0], [18.0, 18.0]]);
    assert_eq!(lines.glyphs[0].y, 15.0);
    assert_eq!(lines.glyphs.last().unwrap().y, 33.0);
    assert_eq!(t.measure(&plain("", &normal), None).unwrap(), [0.0, 0.0]);
    assert!(
        t.lines(&plain("", &normal), 100.0)
            .unwrap()
            .boxes
            .is_empty()
    );
}

#[test]
fn lines_break_to_fit() {
    let mut t = text();
    let f = font(16.0);
    let [wide, one] = t.measure(&plain("one two three", &f), None).unwrap();
    assert_eq!(one, 22.0);
    // Widths are whole layout units, so the text fits again in what it measured.
    assert_eq!((wide * 64.0).fract(), 0.0);
    assert_eq!(
        t.measure(&plain("one two three", &f), Some(wide)).unwrap(),
        [wide, 22.0]
    );
    // Broken, it takes all of the width.
    let broken = t
        .measure(&plain("one two three", &f), Some(wide - 1.0))
        .unwrap();
    assert_eq!(broken, [wide - 1.0, 44.0]);
    // The min-content width: the widest word.
    let [three, _] = t.measure(&plain("three", &f), None).unwrap();
    assert_eq!(
        t.measure(&plain("one two three", &f), Some(0.0)).unwrap(),
        [three, 66.0]
    );
    // `white-space: normal` collapses spaces and line breaks, and trims them.
    assert_eq!(
        t.measure(&plain(" one \n  two\tthree\n", &f), None)
            .unwrap(),
        [wide, 22.0]
    );
    assert_eq!(t.measure(&plain(" \n ", &f), None).unwrap(), [0.0, 0.0]);
}

#[test]
fn weight_spacing_and_alignment() {
    let mut t = text();
    let [regular, _] = t.measure(&plain("Hello", &font(16.0)), None).unwrap();
    let bold = Font {
        weight: 700.0,
        ..font(16.0)
    };
    assert!(t.measure(&plain("Hello", &bold), None).unwrap()[0] > regular);
    let spaced = Font {
        spacing: 2.0,
        ..font(16.0)
    };
    assert_eq!(
        t.measure(&plain("Hello", &spaced), None).unwrap()[0],
        regular + 10.0
    );
    // Bold is another instance of the variable face.
    t.lines(&plain("Hello", &font(16.0)), 100.0).unwrap();
    t.lines(&plain("Hello", &bold), 100.0).unwrap();
    assert_eq!(t.faces().len(), 2);
    assert_ne!(t.faces()[0].coords, t.faces()[1].coords);

    let first = |t: &mut Text, align| {
        let f = Font {
            align,
            ..font(16.0)
        };
        t.lines(&plain("Hello", &f), 100.0).unwrap().glyphs[0].x
    };
    assert_eq!(first(&mut t, TextAlign::Start), 0.0);
    let center = first(&mut t, TextAlign::Center);
    let right = first(&mut t, TextAlign::Right);
    assert!(
        (center - (100.0 - regular) / 2.0).abs() < 1.0 / 64.0,
        "{center}"
    );
    assert!((right - (100.0 - regular)).abs() < 1.0 / 64.0, "{right}");
    assert_eq!(first(&mut t, TextAlign::End), right);
}

#[test]
fn families_must_be_built_in() {
    let mut t = text();
    let with = |family: Family| Font {
        family: Rc::new([family, Family::SansSerif]),
        ..font(16.0)
    };
    assert_eq!(
        t.measure(&plain("a", &with(Family::Serif)), None)
            .unwrap_err(),
        "font family serif is not built in yet"
    );
    assert_eq!(
        t.measure(&plain("a", &with(Family::Named("Brand".into()))), None)
            .unwrap_err(),
        "font family Brand: font resources are not supported yet"
    );
}

#[test]
fn paragraphs_are_kept_while_used() {
    let mut t = text();
    t.measure(&plain("a", &font(16.0)), None).unwrap();
    t.measure(&plain("a", &font(16.0)), Some(10.0)).unwrap();
    t.measure(&plain("b", &font(16.0)), None).unwrap();
    assert_eq!(t.paragraphs.len(), 2);
    t.sweep();
    t.measure(&plain("a", &font(16.0)), None).unwrap();
    t.sweep();
    assert_eq!(t.paragraphs.len(), 1);
    t.sweep();
    assert!(t.paragraphs.is_empty());
}

#[test]
fn white_space_collapses_across_pieces_into_the_piece_it_starts_in() {
    let normal = |text| (text, Collapse::Collapse, TextTransform::None);
    let pieces = [" a ", " ", "b  ", ""].map(normal);
    assert_eq!(
        prepare(pieces.into_iter()),
        Ok(("a b".to_string(), vec![0..2, 2..2, 2..3, 3..3]))
    );
    // Kept white space keeps what follows it, but not the spaces a line break takes.
    let pieces = [
        normal("a  "),
        ("  b \n c  ", Collapse::PreserveBreaks, TextTransform::None),
        (" d  e ", Collapse::Preserve, TextTransform::None),
        normal("\n f"),
    ];
    let (text, _) = prepare(pieces.into_iter()).unwrap();
    assert_eq!(text, "a b\nc  d  e  f");
    let tab = [("a\tb", Collapse::Preserve, TextTransform::None)];
    assert!(prepare(tab.into_iter()).is_err());
}

#[test]
fn text_transforms_case_letters_across_pieces() {
    let piece = |text, transform| (text, Collapse::Collapse, transform);
    let pieces = [
        piece("don't stop-", TextTransform::Capitalize),
        piece("me 3d", TextTransform::Capitalize),
        piece(" Straße", TextTransform::Uppercase),
        piece(" ÉCOLE", TextTransform::Lowercase),
    ];
    let (text, _) = prepare(pieces.into_iter()).unwrap();
    assert_eq!(text, "Don't Stop-Me 3d STRASSE école");
}

#[test]
fn pieces_shape_in_their_own_fonts_on_shared_lines() {
    let mut t = text();
    let (normal, big) = (font(16.0), font(32.0));
    let piece = |text: &'static str, node, font| Piece {
        text: Cow::Borrowed(text),
        node,
        font,
    };
    let content = Content {
        font: &normal,
        pieces: vec![
            piece("Hi ", 0, &normal),
            piece(" big ", 1, &big),
            piece(" end", 0, &normal),
        ],
    };
    let lines = t.lines(&content, 1000.0).unwrap();
    assert_eq!(&*lines.string, "Hi big end");
    let nodes: Vec<u32> = lines.runs.iter().map(|r| r.node).collect();
    assert_eq!(nodes, [0, 1, 0]);
    // At 32px the ascent rounds to 34 and the descent to 9: the big run sets the line, and all
    // its glyphs sit on one baseline.
    assert_eq!(
        (&lines.boxes[..], &lines.baselines[..]),
        (&[[0.0, 43.0]][..], &[34.0][..])
    );
    assert!(lines.glyphs.iter().all(|g| g.y == 34.0));
    assert_eq!(lines.glyphs[0].node, 0);
    // A line holds its paragraph's own font, even with only smaller text on it.
    let small = Content {
        font: &big,
        pieces: vec![piece("small", 1, &normal)],
    };
    assert_eq!(t.measure(&small, None).unwrap()[1], 43.0);
    assert_eq!(t.metrics(&normal).unwrap()[0].round(), 17.0);
}

#[test]
fn ink_is_where_a_glyph_crosses_a_band() {
    let mut t = text();
    let f = font(20.0);
    let lines = t.lines(&plain("xg", &f), 1000.0).unwrap();
    assert_eq!(lines.runs[0].glyphs, [0, 2]);
    let [x, g] = [lines.glyphs[0], lines.glyphs[1]];
    let below = [1.5, 2.5];
    assert_eq!(t.ink(x.face, x.id, 20.0, below).unwrap(), None);
    let [left, right] = t.ink(g.face, g.id, 20.0, below).unwrap().unwrap();
    // The g's tail, within its advance.
    assert!(
        0.0 < left && left < right && right < g.x - x.x,
        "{left} {right}"
    );
}
