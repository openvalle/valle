use takumi_core::resources::font::Fonts;
use takumi_core::viewport::Viewport;
use valle_motion::{TextMeasure, measure_text};

#[test]
fn noto_emoji_measurement_preserves_sequences_and_is_repeatable() {
    let mut fonts = Fonts::default();
    valle_motion::register_default_motion_fonts(&mut fonts).unwrap();
    let measure = |text: &str| {
        measure_text(
            &TextMeasure {
                text,
                style: "font-size: 64px",
                ..Default::default()
            },
            &fonts,
            Viewport::new((960, 540)),
        )
        .unwrap()
    };
    let single = measure("👍");
    for text in [
        "😀",
        "🚀",
        "🔥",
        "🎉",
        "❤️",
        "👍🏻",
        "👍🏽",
        "👍🏿",
        "👩‍💻",
        "👨‍👩‍👧‍👦",
        "🇨🇳",
        "🇺🇸",
        "1️⃣",
        "🏳️‍🌈",
    ] {
        let first = measure(text);
        assert!(first.width > 0.0 && first.height > 0.0, "{text}: {first:?}");
        assert_eq!(first, measure(text), "{text}");
        assert!(
            (first.width - single.width).abs() < 1.0,
            "sequence must occupy one emoji advance: {text}: {first:?}"
        );
    }
}
