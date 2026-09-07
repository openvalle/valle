//! Text measurement must tightly match layout, wrapping and shaping.

use takumi_core::resources::font::{FontResource, Fonts};
use takumi_core::viewport::Viewport;
use valle_motion::{MeasuredBox, TextMeasure, measure_text};

const FONT: &[u8] = include_bytes!("../assets/fonts/noto/NotoSansCJKsc-Regular.otf");

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

fn measure(text: &str, style: &str, max_width: Option<f64>) -> MeasuredBox {
    measure_text(
        &TextMeasure {
            text,
            class_names: &[],
            style,
            max_width,
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
    let style = "font-size: 48px";
    let free = measure(text, style, None);

    let exact = measure(text, style, Some(free.width));
    assert_eq!(
        exact.height, free.height,
        "constraining to the measured width must not wrap: {free:?} vs {exact:?}"
    );

    // Reducing the measured width by one pixel must wrap the text.
    let tighter = measure(text, style, Some(free.width - 1.0));
    assert!(
        tighter.height > free.height,
        "measured width must be tight — one pixel less should wrap ({free:?} vs {tighter:?})"
    );
}

#[test]
fn wrapping_grows_height_and_respects_the_constraint() {
    let free = measure("全球业务网络 Overview", "font-size: 48px", None);
    let wrapped = measure("全球业务网络 Overview", "font-size: 48px", Some(200.0));
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
    assert_eq!(
        measure("确定性", "font-size: 32px", None),
        measure("确定性", "font-size: 32px", None)
    );
}

#[test]
fn text_and_style_actually_reach_the_shaper() {
    // Both assertions require real layout metrics rather than estimated widths.
    let small = measure("Aa", "font-size: 20px", None);
    let large = measure("Aa", "font-size: 40px", None);
    assert!(
        large.width > small.width && large.height > small.height,
        "font-size must reach the shaper: {small:?} vs {large:?}"
    );

    let short = measure("A", "font-size: 32px", None);
    let long = measure("AAAAAAAA", "font-size: 32px", None);
    assert!(
        long.width > short.width,
        "text content must reach the shaper: {short:?} vs {long:?}"
    );
}

#[test]
fn tailwind_classes_go_through_the_same_catalog_as_scene_nodes() {
    let plain = measure_text(
        &TextMeasure {
            text: "Aa",
            class_names: &[],
            style: "font-size: 16px",
            max_width: None,
        },
        &fonts(),
        viewport(),
    )
    .expect("plain measure");
    let tw = measure_text(
        &TextMeasure {
            text: "Aa",
            class_names: &["text-5xl".to_owned()],
            style: "",
            max_width: None,
        },
        &fonts(),
        viewport(),
    )
    .expect("tailwind measure");
    assert!(
        tw.width > plain.width,
        "className must reach the shaper via the same Tailwind path as Scene nodes: \
         {plain:?} vs {tw:?}"
    );
}

#[test]
fn bad_constraints_and_styles_fail_closed() {
    let fonts = fonts();
    for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        let result = measure_text(
            &TextMeasure {
                text: "x",
                class_names: &[],
                style: "",
                max_width: Some(bad),
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

    let bad_style = measure_text(
        &TextMeasure {
            text: "x",
            class_names: &[],
            style: "font-size: ###",
            max_width: None,
        },
        &fonts,
        viewport(),
    );
    assert!(
        matches!(bad_style, Err(valle_motion::MeasureError::BadStyle { .. })),
        "unparseable style must fail closed, got {bad_style:?}"
    );
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
                class_names: &[],
                style: "font-family: monospace; font-size: 32px",
                max_width: None,
            },
            &fonts,
            viewport(),
        )
        .expect("measure monospace text")
        .width
    };
    assert!((width("iiii") - width("WWWW")).abs() < 0.01);
}
