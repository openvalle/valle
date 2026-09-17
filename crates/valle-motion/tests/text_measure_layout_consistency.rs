//! Text measurement must tightly match layout, wrapping and shaping.
//!
//! Options are explicit numbers with the same meaning as the identically named authored style
//! property, so every case here can be written once as a request and once as a `Text` style.

use takumi_core::resources::font::{FontResource, Fonts};
use takumi_core::viewport::Viewport;
use valle_motion::{MeasuredBox, TextMeasure, measure_text};

const FONT: &[u8] = include_bytes!("../../../assets/fonts/noto/NotoSansCJKsc-Regular.otf");

fn fonts() -> Fonts {
    let mut fonts = Fonts::default();
    fonts
        .register(FontResource::new(FONT.to_vec()))
        .expect("register measure font");
    fonts
}

fn viewport() -> Viewport {
    Viewport::new((1920, 1080))
}

fn measure(text: &str, font_size: f64, max_width: Option<f64>) -> MeasuredBox {
    measure_text(
        &TextMeasure {
            text,
            font_size,
            max_width,
            ..Default::default()
        },
        &fonts(),
        viewport(),
    )
    .expect("measure succeeds")
}

#[test]
fn the_measured_width_is_tight_enough_to_size_a_box_with() {
    // A box sized from measured text should fit that text on one line.
    let text = "全球业务网络 Overview";
    let free = measure(text, 48.0, None);

    let exact = measure(text, 48.0, Some(free.width));
    assert_eq!(
        exact.height, free.height,
        "constraining to the measured width must not wrap: {free:?} vs {exact:?}"
    );

    // Reducing the measured width by one pixel must wrap the text.
    let tighter = measure(text, 48.0, Some(free.width - 1.0));
    assert!(
        tighter.height > free.height,
        "measured width must be tight — one pixel less should wrap ({free:?} vs {tighter:?})"
    );
}

#[test]
fn wrapping_grows_height_and_respects_the_constraint() {
    let free = measure("全球业务网络 Overview", 48.0, None);
    let wrapped = measure("全球业务网络 Overview", 48.0, Some(200.0));
    assert!(
        wrapped.width <= 200.0,
        "wrapped width must respect maxWidth"
    );
    assert!(
        wrapped.height > free.height,
        "wrapping must grow height ({free:?} vs {wrapped:?})"
    );
}

#[test]
fn measurement_is_deterministic_across_calls_and_fresh_registries() {
    // Fresh font registries must produce the same measurement regardless of registration timing.
    assert_eq!(measure("确定性", 32.0, None), measure("确定性", 32.0, None));
}

#[test]
fn text_and_style_actually_reach_the_shaper() {
    // Both assertions require real layout metrics rather than estimated widths.
    let small = measure("Aa", 20.0, None);
    let large = measure("Aa", 40.0, None);
    assert!(
        large.width > small.width && large.height > small.height,
        "font-size must reach the shaper: {small:?} vs {large:?}"
    );

    let short = measure("A", 32.0, None);
    let long = measure("AAAAAAAA", 32.0, None);
    assert!(
        long.width > short.width,
        "text content must reach the shaper: {short:?} vs {long:?}"
    );
}

#[test]
fn every_option_reaches_the_shaper() {
    let fonts = fonts();
    let options = |request: TextMeasure<'_>| {
        measure_text(&request, &fonts, viewport()).expect("measure succeeds")
    };
    let base = options(TextMeasure {
        text: "iiii WWWW",
        font_size: 32.0,
        ..Default::default()
    });
    let spaced = options(TextMeasure {
        text: "iiii WWWW",
        font_size: 32.0,
        letter_spacing: Some(6.0),
        ..Default::default()
    });
    assert!(
        spaced.width > base.width,
        "letterSpacing must widen the run: {base:?} vs {spaced:?}"
    );
    let tall = options(TextMeasure {
        text: "iiii WWWW",
        font_size: 32.0,
        line_height: Some(3.0),
        ..Default::default()
    });
    assert!(
        tall.height > base.height,
        "a unitless line-height multiplier must grow the box: {base:?} vs {tall:?}"
    );
    let weighted = options(TextMeasure {
        text: "iiii WWWW",
        font_size: 32.0,
        font_weight: Some(700.0),
        ..Default::default()
    });
    assert!(
        weighted.width > 0.0 && weighted.height > 0.0,
        "fontWeight must reach the shaper: {weighted:?}"
    );
}

#[test]
fn bad_options_and_constraints_fail_closed() {
    let fonts = fonts();
    for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        let result = measure_text(
            &TextMeasure {
                text: "x",
                font_size: 16.0,
                max_width: Some(bad),
                ..Default::default()
            },
            &fonts,
            viewport(),
        );
        assert!(
            matches!(
                result,
                Err(valle_motion::MeasureError::BadConstraint { .. })
            ),
            "maxWidth {bad} must fail closed, got {result:?}"
        );
    }

    for (option, request) in [
        (
            "fontSize",
            TextMeasure {
                text: "x",
                ..Default::default()
            },
        ),
        (
            "fontSize",
            TextMeasure {
                text: "x",
                font_size: f64::NAN,
                ..Default::default()
            },
        ),
        (
            "fontWeight",
            TextMeasure {
                text: "x",
                font_size: 16.0,
                font_weight: Some(2000.0),
                ..Default::default()
            },
        ),
        (
            "fontFamily",
            TextMeasure {
                text: "x",
                font_size: 16.0,
                font_family: Some("  "),
                ..Default::default()
            },
        ),
    ] {
        let error =
            measure_text(&request, &fonts, viewport()).expect_err("a bad option must fail closed");
        assert!(
            error.to_string().contains(option),
            "the message must name `{option}`: {error}"
        );
    }
}

#[test]
fn bundled_noto_fonts_cover_chinese_and_preserve_monospace_metrics() {
    let face = ttf_parser::Face::parse(FONT, 0).expect("parse Noto CJK font");
    for ch in "全球业务网络确定性".chars() {
        assert!(
            face.glyph_index(ch).is_some(),
            "missing Chinese glyph: {ch}"
        );
    }

    let mut fonts = Fonts::default();
    valle_motion::register_default_motion_fonts(&mut fonts).expect("register default pack");
    let width = |text| {
        measure_text(
            &TextMeasure {
                text,
                font_size: 32.0,
                font_family: Some("monospace"),
                ..Default::default()
            },
            &fonts,
            viewport(),
        )
        .expect("measure monospace text")
        .width
    };
    assert!((width("iiii") - width("WWWW")).abs() < 0.01);
}

#[test]
fn measurement_uses_the_explicit_viewport_without_scaling_by_dpr() {
    let fonts = fonts();
    let at = |viewport, max_width| {
        measure_text(
            &TextMeasure {
                text: "全球业务网络 Overview",
                font_size: 20.0,
                max_width,
                ..Default::default()
            },
            &fonts,
            viewport,
        )
        .unwrap()
    };
    // Layout rounds glyph extents to device pixels, so DPR can change the CSS-pixel rounding by less
    // than one pixel. It must not multiply either the result or maxWidth by the DPR.
    for max_width in [None, Some(200.0)] {
        let a = at(Viewport::new((640, 400)), max_width);
        let b = at(
            Viewport::new((1280, 800)).with_device_pixel_ratio(2.0),
            max_width,
        );
        assert!(
            (a.width - b.width).abs() < 1.0 && (a.height - b.height).abs() < 1.0,
            "{a:?} vs {b:?}"
        );
    }
    assert_eq!(
        at(Viewport::new((640, 400)), None),
        at(Viewport::new((640, 400)), None),
        "measurement must not leave mutable state"
    );
}
