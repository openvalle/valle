use std::borrow::Cow;
use std::rc::Rc;

use super::*;

fn text() -> Text {
    crate::testing::text()
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
        clamp: None,
    }
}

fn font(size: f64) -> Font {
    Font {
        size,
        weight: 400.0,
        stretch: 100.0,
        style: FontStyle::Normal,
        family: Rc::new([Family::SansSerif]),
        line_height: LineHeight::Normal,
        align: TextAlign::Start,
        indent: None,
        spacing: 0.0,
        word_spacing: 0.0,
        collapse: Collapse::Collapse,
        wrap: true,
        balance: false,
        word_break: WordBreak::Normal,
        overflow_wrap: OverflowWrap::Normal,
        transform: TextTransform::None,
        variants: Default::default(),
        features: Rc::new([]),
        variations: Rc::new([]),
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
        t.min_content(&plain("one two three", &f)).unwrap(),
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
fn families_are_built_in_or_resources() {
    let mut t = text();
    let with = |family: Family| Font {
        family: Rc::new([family, Family::SansSerif]),
        ..font(16.0)
    };
    // Each built-in family is its own font: monospace gives i and W one width.
    let mut width = |family: Family, s: &str| t.measure(&plain(s, &with(family)), None).unwrap()[0];
    let [sans, serif] = [Family::SansSerif, Family::Serif].map(|f| width(f, "Serif"));
    assert_ne!(sans, serif);
    assert_eq!(
        width(Family::Monospace, "iiii"),
        width(Family::Monospace, "WWWW")
    );
    assert_ne!(
        width(Family::SansSerif, "iiii"),
        width(Family::SansSerif, "WWWW")
    );
    assert_eq!(
        t.measure(&plain("a", &with(Family::Named("Brand".into()))), None)
            .unwrap_err(),
        "font family Brand: the scene declares no font resource Brand in controls.resources"
    );
    // A font resource is a family by its name, whatever its ASCII case.
    t.add_font("brand", "Serif.ttf", crate::testing::built_in(2))
        .unwrap();
    let mut width = |family: Family| t.measure(&plain("Serif", &with(family)), None).unwrap()[0];
    assert_eq!(width(Family::Named("Brand".into())), width(Family::Serif));
    assert_eq!(
        t.add_font("bad", "bad.ttf", Blob::from(b"not a font".to_vec()))
            .unwrap_err(),
        "not a font file: TrueType, OpenType, WOFF, WOFF2 or a collection"
    );
}

#[test]
fn a_font_resource_is_a_family_of_its_files() {
    let mut t = text();
    t.add_font("brand", "Sans.ttf", crate::testing::built_in(0))
        .unwrap();
    t.add_font("brand", "Italic.ttf", crate::testing::built_in(1))
        .unwrap();
    let file = |path: &str, italic| FontFile {
        family: "brand".into(),
        path: path.into(),
        weight: [100.0, 900.0],
        width: [62.5, 100.0],
        italic,
    };
    assert_eq!(
        t.font_files(),
        [file("Sans.ttf", false), file("Italic.ttf", true)]
    );
    // Italic text takes the italic file, not the upright one slanted.
    let mut face = |style| {
        let f = Font {
            family: Rc::from([Family::Named("brand".into())]),
            style,
            ..font(16.0)
        };
        let lines = t.lines(&plain("Lean", &f), 1000.0).unwrap();
        let face = &t.faces()[lines.glyphs[0].face as usize];
        (face.data.id(), face.slanted)
    };
    let (upright, _) = face(FontStyle::Normal);
    let (italic, slanted) = face(FontStyle::Italic);
    assert!(italic != upright && !slanted);
    assert_eq!(
        t.add_font("brand", "Again.ttf", crate::testing::built_in(0))
            .unwrap_err(),
        "the same weight, style and width as Sans.ttf"
    );
}

#[test]
fn a_collection_gives_its_first_face() {
    // Noto Sans, then Noto Serif, as a TTC.
    let fonts = [0, 2].map(|i| crate::testing::built_in(i).data().to_vec());
    let mut ttc = b"ttcf\0\x01\0\0\0\0\0\x02".to_vec();
    ttc.resize(20, 0);
    for (n, font) in fonts.iter().enumerate() {
        let base = ttc.len();
        ttc[12 + 4 * n..16 + 4 * n].copy_from_slice(&(base as u32).to_be_bytes());
        let mut font = font.clone();
        let tables = u16::from_be_bytes([font[4], font[5]]) as usize;
        for i in 0..tables {
            let at = 12 + 16 * i + 8;
            let offset = u32::from_be_bytes(font[at..at + 4].try_into().unwrap());
            font[at..at + 4].copy_from_slice(&(offset + base as u32).to_be_bytes());
        }
        ttc.extend(font);
        ttc.resize(ttc.len().next_multiple_of(4), 0);
    }
    let mut t = text();
    t.add_font("pair", "Pair.ttc", Blob::from(ttc)).unwrap();
    assert_eq!(t.font_files().len(), 1);
    let with = |family: Family| Font {
        family: Rc::new([family]),
        ..font(16.0)
    };
    let mut width = |family| t.measure(&plain("Serif", &with(family)), None).unwrap()[0];
    assert_eq!(
        width(Family::Named("pair".into())),
        width(Family::SansSerif)
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
        clamp: None,
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
        clamp: None,
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

#[test]
fn words_break_where_word_break_and_overflow_wrap_let_them() {
    let mut t = text();
    let with = |word_break, overflow_wrap| Font {
        word_break,
        overflow_wrap,
        ..font(16.0)
    };
    let normal = font(16.0);
    let [word, _] = t.measure(&plain("unbreakable", &normal), None).unwrap();
    let lines = |t: &mut Text, f: &Font| t.lines(&plain("unbreakable go", f), word / 2.0).unwrap();
    // A word too wide for its line overflows it, unless a word may break.
    assert_eq!(lines(&mut t, &normal).text.len(), 2);
    let breaking = [
        with(WordBreak::BreakAll, OverflowWrap::Normal),
        with(WordBreak::BreakWord, OverflowWrap::Normal),
        with(WordBreak::Normal, OverflowWrap::Anywhere),
        with(WordBreak::Normal, OverflowWrap::BreakWord),
    ];
    for f in &breaking {
        assert!(lines(&mut t, f).text.len() > 2, "{f:?}");
    }
    // The min-content width is the widest word's, or a letter's where words break, except that
    // overflow-wrap: break-word only breaks words that do not fit.
    let min = |t: &mut Text, f: &Font| t.min_content(&plain("unbreakable go", f)).unwrap()[0];
    assert_eq!(min(&mut t, &normal), word);
    assert_eq!(min(&mut t, &breaking[3]), word);
    for f in &breaking[..3] {
        assert!(min(&mut t, f) < word / 4.0, "{f:?}");
    }
    let zero = t.measure(&plain("unbreakable go", &breaking[3]), Some(0.0));
    assert!(zero.unwrap()[0] < word / 4.0);
}

#[test]
fn balanced_lines_are_as_narrow_as_they_go_up_to_the_first_forced_break() {
    let mut t = text();
    let (normal, balanced) = (
        font(16.0),
        Font {
            balance: true,
            align: TextAlign::Center,
            ..font(16.0)
        },
    );
    let words = "The quick brown fox jumps over the lazy dog";
    let [wide, _] = t.measure(&plain(words, &normal), None).unwrap();
    let width = wide * 0.7;
    let line_widths = |lines: &Lines| -> Vec<f32> {
        (0..lines.boxes.len() as u32)
            .map(|i| {
                let runs = lines.runs.iter().filter(|r| r.line == i);
                runs.map(|r| r.x[1] - r.x[0]).sum()
            })
            .collect()
    };
    let greedy = line_widths(&t.lines(&plain(words, &normal), width).unwrap());
    let lines = t.lines(&plain(words, &balanced), width).unwrap();
    let even = line_widths(&lines);
    assert_eq!((even.len(), greedy.len()), (2, 2));
    assert!(
        even[0].max(even[1]) < greedy[0] && (even[0] - even[1]).abs() < greedy[0] - greedy[1],
        "{even:?} {greedy:?}"
    );
    // Still centered in the whole width.
    let first = lines.runs[0].x;
    assert!(
        ((first[0] + first[1]) / 2.0 - width / 2.0).abs() < 1.0,
        "{first:?}"
    );
    assert_eq!(
        t.measure(&plain(words, &balanced), Some(width)).unwrap(),
        t.measure(&plain(words, &normal), Some(width)).unwrap()
    );
    // Lines after a forced break, and text of more than 6 lines, break as usual.
    let pre_line = |f: &Font| Font {
        collapse: Collapse::PreserveBreaks,
        ..f.clone()
    };
    let text = format!("{words}\n{words}");
    let after = t.lines(&plain(&text, &pre_line(&balanced)), width).unwrap();
    let usual = t.lines(&plain(&text, &pre_line(&normal)), width).unwrap();
    assert_eq!(after.text[0], lines.text[0]);
    assert_eq!(after.text[2..], usual.text[2..]);
    let long = [words; 4].join(" ");
    let many = t.lines(&plain(&long, &balanced), width).unwrap();
    assert!(many.boxes.len() > 6);
    assert_eq!(
        many.text,
        t.lines(&plain(&long, &normal), width).unwrap().text
    );
}

#[test]
fn lines_that_overflow_end_in_an_ellipsis_after_the_glyphs_that_fit() {
    let mut t = text();
    let f = Font {
        wrap: false,
        ..font(16.0)
    };
    let words = "The quick brown fox";
    let [wide, _] = t.measure(&plain(words, &f), None).unwrap();
    let width = (wide / 2.0).round();
    let mut lines = t.lines(&plain(words, &f), width).unwrap();
    let whole = lines.clone();
    lines.decorations.push(Stripe {
        rect: [0.0, 18.0, whole.runs[0].x[1], 19.0],
        along: [0.0, whole.runs[0].x[1]],
        style: DecorationStyle::Solid,
        thickness: 1.0,
        color: [0.0, 0.0, 0.0, 1.0],
        run: 0,
        through: false,
    });
    t.truncate(&mut lines, &f, 9, width, true, None).unwrap();
    // The run keeps its extent; the ellipsis is a run of its own, of the Text.
    assert_eq!(
        lines.runs[0],
        Run {
            glyphs: lines.runs[0].glyphs,
            ..whole.runs[0]
        }
    );
    let [_, ellipsis] = &lines.runs[..] else {
        panic!("{:?}", lines.runs)
    };
    assert_eq!((ellipsis.node, ellipsis.line), (9, 0));
    let cut = ellipsis.x[0];
    assert!(
        ellipsis.x[1] <= width && ellipsis.x[1] > width - 12.0,
        "{ellipsis:?}"
    );
    let kept = lines.runs[0].glyphs[1] as usize;
    assert!(kept > 2 && kept < whole.glyphs.len() - 2, "{kept}");
    assert_eq!(lines.glyphs[..kept], whole.glyphs[..kept]);
    assert_eq!(cut, whole.glyphs[kept].x);
    // Its decorations end where the ellipsis starts.
    assert_eq!(lines.decorations[0].rect[2], cut);
    // A line that fits stays as it is.
    let mut fits = t.lines(&plain(words, &f), wide).unwrap();
    let before = fits.clone();
    t.truncate(&mut fits, &f, 9, wide, true, None).unwrap();
    assert_eq!(fits, before);
}

#[test]
fn clamped_text_is_as_tall_as_its_lines_and_ends_the_last_in_an_ellipsis() {
    let mut t = text();
    let f = font(16.0);
    let words = "aaaa aaaa aaaa";
    let [word, _] = t.measure(&plain("aaaa", &f), None).unwrap();
    let advance = t.ellipsis(&f).unwrap().advance;
    let clamped = |n| Content {
        clamp: Some(n),
        ..plain(words, &f)
    };
    // With room for the ellipsis, it follows line 2's glyphs, all kept.
    let roomy = word + advance + 1.0;
    let whole = t.lines(&plain(words, &f), roomy).unwrap();
    assert_eq!(whole.boxes.len(), 3);
    let [top, height] = whole.boxes[1];
    assert_eq!(
        t.measure(&clamped(2), Some(roomy)).unwrap()[1],
        top + height
    );
    let mut lines = whole.clone();
    t.truncate(&mut lines, &f, 9, roomy, false, Some(2))
        .unwrap();
    assert_eq!(lines.glyphs.len(), whole.glyphs.len() + 1);
    let ellipsis = lines.runs.last().unwrap();
    assert_eq!((ellipsis.node, ellipsis.line), (9, 1));
    assert_eq!(ellipsis.x[0], whole.runs[1].x[1]);
    // Without it, the glyphs past where it starts go.
    let tight = word + advance - 2.0;
    let mut lines = t.lines(&plain(words, &f), tight).unwrap();
    let before = lines.glyphs.len();
    t.truncate(&mut lines, &f, 9, tight, false, Some(2))
        .unwrap();
    assert!(lines.glyphs.len() < before, "{lines:?}");
    assert!(lines.runs.last().unwrap().x[1] <= tight);
    // The last line shown, with none after it, stays as it is.
    let mut last = whole.clone();
    t.truncate(&mut last, &f, 9, roomy, false, Some(3)).unwrap();
    assert_eq!(last, whole);
}

#[test]
fn font_features_choose_glyphs_and_a_later_one_of_a_tag_wins() {
    let mut t = text();
    let mut ids = |f: &Font| {
        let lines = t.lines(&plain("abc", f), 1000.0).unwrap();
        lines.glyphs.iter().map(|g| g.id).collect::<Vec<_>>()
    };
    let with = |features: &[(&[u8; 4], u16)]| Font {
        features: (features.iter())
            .map(|&(tag, value)| Feature { tag: *tag, value })
            .collect(),
        ..font(16.0)
    };
    let normal = ids(&font(16.0));
    assert_ne!(ids(&with(&[(b"smcp", 1)])), normal);
    assert_eq!(ids(&with(&[(b"smcp", 1), (b"smcp", 0)])), normal);
}

#[test]
fn font_variations_set_axes_over_the_weight() {
    let mut t = text();
    let mut wide = |f: &Font| t.measure(&plain("Heavy words", f), None).unwrap()[0];
    let axes = |axes: &[(&[u8; 4], f32)]| Font {
        variations: (axes.iter())
            .map(|&(tag, value)| Variation { tag: *tag, value })
            .collect(),
        ..font(16.0)
    };
    let bold = Font {
        weight: 900.0,
        ..font(16.0)
    };
    assert_eq!(wide(&axes(&[(b"wght", 900.0)])), wide(&bold));
    assert_ne!(wide(&bold), wide(&font(16.0)));
    let narrow = Font {
        weight: 900.0,
        ..axes(&[(b"wdth", 62.5), (b"wght", 400.0)])
    };
    assert!(wide(&narrow) < wide(&font(16.0)));
}

#[test]
fn font_stretch_sets_the_width_axis_up_to_its_ends() {
    let mut t = text();
    let mut wide = |stretch| {
        let f = Font {
            stretch,
            ..font(16.0)
        };
        t.measure(&plain("Wide words", &f), None).unwrap()[0]
    };
    assert!(wide(75.0) < wide(100.0));
    assert_eq!(wide(150.0), wide(100.0));
    assert_eq!(wide(50.0), wide(62.5));
}

#[test]
fn italic_text_has_a_slanted_face_where_its_font_has_no_italic() {
    let mut t = text();
    let mut slanted = |family, style| {
        let f = Font {
            family: Rc::from([family]),
            style,
            ..font(16.0)
        };
        let lines = t.lines(&plain("Lean", &f), 1000.0).unwrap();
        let face = &t.faces()[lines.glyphs[0].face as usize];
        (face.data.id(), face.slanted, lines.runs[0].x)
    };
    let (mono, upright, x) = slanted(Family::Monospace, FontStyle::Normal);
    assert!(!upright);
    // Slanting moves no glyph.
    assert_eq!(
        slanted(Family::Monospace, FontStyle::Italic),
        (mono, true, x)
    );
    assert_eq!(
        slanted(Family::Monospace, FontStyle::Oblique(14.0)),
        (mono, true, x)
    );
    assert_eq!(
        slanted(Family::Monospace, FontStyle::Oblique(10.0)),
        (mono, false, x)
    );
    // Sans-serif has an italic, its own file.
    let (sans, _, x) = slanted(Family::SansSerif, FontStyle::Normal);
    let (italic, slanted, italic_x) = slanted(Family::SansSerif, FontStyle::Italic);
    assert!(italic != sans && !slanted && italic_x != x);
}
