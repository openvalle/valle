use std::rc::Rc;

use super::*;

fn text() -> Text {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fonts/NotoSans-Variable.ttf"
    );
    Text::new(Blob::from(std::fs::read(path).unwrap())).unwrap()
}

fn font(size: f64) -> Font {
    Font {
        size,
        weight: 400.0,
        family: Rc::new([Family::SansSerif]),
        line_height: LineHeight::Normal,
        align: TextAlign::Start,
        spacing: 0.0,
    }
}

#[test]
fn lines_stack_as_chrome_stacks_them() {
    let mut t = text();
    // Noto Sans: ascent 1.069 em and descent 0.293, which at 16px round to 17 and 5.
    let normal = font(16.0);
    assert_eq!(t.measure("Hello", &normal, None).unwrap()[1], 22.0);
    let lines = t.lines("Hello", &normal, 100.0).unwrap();
    assert_eq!(lines.boxes, [[0.0, 22.0]]);
    assert!(lines.glyphs.iter().all(|g| g.y == 17.0));

    // Half the leading, floored, goes above the ascent.
    let tall = Font {
        line_height: LineHeight::Factor(1.5),
        ..font(16.0)
    };
    let lines = t.lines("Hello", &tall, 100.0).unwrap();
    assert_eq!(lines.boxes, [[0.0, 24.0]]);
    assert_eq!(lines.glyphs[0].y, 18.0);
    let tight = Font {
        line_height: LineHeight::Px(18.0),
        ..font(16.0)
    };
    let lines = t.lines("one two", &tight, 0.0).unwrap();
    assert_eq!(lines.boxes, [[0.0, 18.0], [18.0, 18.0]]);
    assert_eq!(lines.glyphs[0].y, 15.0);
    assert_eq!(lines.glyphs.last().unwrap().y, 33.0);
    assert_eq!(t.measure("", &normal, None).unwrap(), [0.0, 0.0]);
    assert!(t.lines("", &normal, 100.0).unwrap().boxes.is_empty());
}

#[test]
fn lines_break_to_fit() {
    let mut t = text();
    let f = font(16.0);
    let [wide, one] = t.measure("one two three", &f, None).unwrap();
    assert_eq!(one, 22.0);
    // Widths are whole layout units, so the text fits again in what it measured.
    assert_eq!((wide * 64.0).fract(), 0.0);
    assert_eq!(
        t.measure("one two three", &f, Some(wide)).unwrap(),
        [wide, 22.0]
    );
    // Broken, it takes all of the width.
    let broken = t.measure("one two three", &f, Some(wide - 1.0)).unwrap();
    assert_eq!(broken, [wide - 1.0, 44.0]);
    // The min-content width: the widest word.
    let [three, _] = t.measure("three", &f, None).unwrap();
    assert_eq!(
        t.measure("one two three", &f, Some(0.0)).unwrap(),
        [three, 66.0]
    );
    // `white-space: normal` collapses spaces and line breaks, and trims them.
    assert_eq!(
        t.measure(" one \n  two\tthree\n", &f, None).unwrap(),
        [wide, 22.0]
    );
    assert_eq!(t.measure(" \n ", &f, None).unwrap(), [0.0, 0.0]);
}

#[test]
fn weight_spacing_and_alignment() {
    let mut t = text();
    let [regular, _] = t.measure("Hello", &font(16.0), None).unwrap();
    let bold = Font {
        weight: 700.0,
        ..font(16.0)
    };
    assert!(t.measure("Hello", &bold, None).unwrap()[0] > regular);
    let spaced = Font {
        spacing: 2.0,
        ..font(16.0)
    };
    assert_eq!(
        t.measure("Hello", &spaced, None).unwrap()[0],
        regular + 10.0
    );
    // Bold is another instance of the variable face.
    t.lines("Hello", &font(16.0), 100.0).unwrap();
    t.lines("Hello", &bold, 100.0).unwrap();
    assert_eq!(t.faces().len(), 2);
    assert_ne!(t.faces()[0].coords, t.faces()[1].coords);

    let first = |t: &mut Text, align| {
        let f = Font {
            align,
            ..font(16.0)
        };
        t.lines("Hello", &f, 100.0).unwrap().glyphs[0].x
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
        t.measure("a", &with(Family::Serif), None).unwrap_err(),
        "font family serif is not built in yet"
    );
    assert_eq!(
        t.measure("a", &with(Family::Named("Brand".into())), None)
            .unwrap_err(),
        "font family Brand: font resources are not supported yet"
    );
}

#[test]
fn paragraphs_are_kept_while_used() {
    let mut t = text();
    t.measure("a", &font(16.0), None).unwrap();
    t.measure("a", &font(16.0), Some(10.0)).unwrap();
    t.measure("b", &font(16.0), None).unwrap();
    assert_eq!(t.paragraphs.len(), 2);
    t.sweep();
    t.measure("a", &font(16.0), None).unwrap();
    t.sweep();
    assert_eq!(t.paragraphs.len(), 1);
    t.sweep();
    assert!(t.paragraphs.is_empty());
}
