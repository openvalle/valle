use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Debug,
};

use serde_json::{Value, json};
use valle_compiler::caption_presets::{
    CAPTION_PRESET_COUNT, CAPTION_PRESET_PACK_ID, CaptionPreset, CaptionPresetError,
    CaptionPresetPhase, CaptionPresetRequest, DisplayCaptionPreset, MAX_CAPTION_PRESET_SAMPLES,
    TimedCaptionPreset, expand_caption_presets, parse_caption_preset,
};
use valle_timeline::internal::{
    CanonicalTimeline, ExactRational, FrameRate, RationalTime, decode_canonical,
    wire::document::{
        CaptionItemWire, CaptionPresentationWire, CubicBezierEasingWire, EasingWire,
        NamedEasingWire, ParamWire, TimelineDocumentEnvelopeWire,
    },
};

const ENTER_IDS: [&str; 11] = [
    "enter.fade",
    "enter.slide-up",
    "enter.slide-down",
    "enter.slide-left",
    "enter.slide-right",
    "enter.pop",
    "enter.zoom-in",
    "enter.zoom-out",
    "enter.focus",
    "enter.wipe-right",
    "enter.wipe-left",
];

const EXIT_IDS: [&str; 11] = [
    "exit.fade",
    "exit.slide-up",
    "exit.slide-down",
    "exit.slide-left",
    "exit.slide-right",
    "exit.pop",
    "exit.zoom-in",
    "exit.zoom-out",
    "exit.focus",
    "exit.wipe-right",
    "exit.wipe-left",
];

const DISPLAY_IDS: [&str; 5] = [
    "display.breathe",
    "display.float",
    "display.sway",
    "display.pulse",
    "display.shake",
];

const TIME_REVERSED_PAIRS: [(&str, &str); 11] = [
    ("enter.fade", "exit.fade"),
    ("enter.slide-up", "exit.slide-down"),
    ("enter.slide-down", "exit.slide-up"),
    ("enter.slide-left", "exit.slide-right"),
    ("enter.slide-right", "exit.slide-left"),
    ("enter.pop", "exit.pop"),
    ("enter.zoom-in", "exit.zoom-out"),
    ("enter.zoom-out", "exit.zoom-in"),
    ("enter.focus", "exit.focus"),
    ("enter.wipe-right", "exit.wipe-left"),
    ("enter.wipe-left", "exit.wipe-right"),
];

fn seconds(numerator: i64, denominator: u32) -> RationalTime {
    RationalTime::new(numerator, denominator).expect("valid test time")
}

fn fps(numerator: i64, denominator: u32) -> FrameRate {
    FrameRate::new(numerator, denominator).expect("valid test frame rate")
}

fn preset(id: &str, phase: CaptionPresetPhase) -> CaptionPreset {
    parse_caption_preset(id, phase).unwrap_or_else(|error| panic!("parse `{id}`: {error}"))
}

fn request_for(
    preset: CaptionPreset,
    canvas_size: [u32; 2],
    caption_duration: RationalTime,
    phase_duration: RationalTime,
    frame_rate: FrameRate,
) -> CaptionPresetRequest {
    let (enter, display, exit) = match preset.phase() {
        CaptionPresetPhase::Enter => (
            Some(TimedCaptionPreset {
                preset,
                duration: phase_duration,
            }),
            None,
            None,
        ),
        CaptionPresetPhase::Display => {
            (None, Some(DisplayCaptionPreset { preset, rate: 1.0 }), None)
        }
        CaptionPresetPhase::Exit => (
            None,
            None,
            Some(TimedCaptionPreset {
                preset,
                duration: phase_duration,
            }),
        ),
    };
    CaptionPresetRequest {
        owner_id: "caption:synthetic".to_owned(),
        canvas_size,
        caption_duration,
        fps: frame_rate,
        enter,
        display,
        exit,
    }
}

fn expanded(preset: CaptionPreset, canvas_size: [u32; 2]) -> CaptionPresentationWire {
    expand_caption_presets(&request_for(
        preset,
        canvas_size,
        seconds(4, 1),
        seconds(3, 10),
        fps(60, 1),
    ))
    .unwrap_or_else(|error| panic!("expand `{}`: {error}", preset.id()))
}

fn synthetic_timeline() -> TimelineDocumentEnvelopeWire {
    let source = json!({
        "document": {
            "canvas": {
                "width": 1080,
                "height": 1920,
                "fps": "60/1",
                "sampleRate": 48000,
                "channelLayout": "stereo",
                "colorSpace": "srgb",
                "duration": "4/1"
            },
            "background": { "color": "#000000ff" },
            "visual": { "tracks": [] },
            "audio": { "tracks": [] },
            "adjustments": [],
            "captions": {
                "tracks": [{
                    "id": "caption-track:synthetic",
                    "items": [{
                        "type": "clip",
                        "id": "caption:synthetic",
                        "duration": "4/1",
                        "runs": [{
                            "id": "text-run:synthetic",
                            "text": "Valle caption presets",
                            "timing": null,
                            "style": null
                        }],
                        "style": {
                            "font": "font:synthetic",
                            "fontSize": 64.0,
                            "color": "#ffffffff",
                            "shadow": null
                        },
                        "layout": {
                            "region": [0.1, 0.75, 0.8, 0.15],
                            "align": "bottom-center"
                        },
                        "presentation": {
                            "opacity": { "type": "constant", "value": 1.0 },
                            "translation": { "type": "constant", "value": [0.0, 0.0] },
                            "scale": { "type": "constant", "value": 1.0 },
                            "rotation": { "type": "constant", "value": 0.0 },
                            "clipInset": {
                                "type": "constant",
                                "value": [0.0, 0.0, 0.0, 0.0]
                            },
                            "blurSigma": { "type": "constant", "value": 0.0 }
                        },
                        "behavior": null
                    }]
                }]
            },
            "camera": null,
            "metadata": {}
        }
    });
    decode_canonical(&serde_json::to_string(&source).expect("encode synthetic timeline"))
        .expect("decode synthetic canonical timeline")
        .to_wire()
}

fn first_caption_mut(
    timeline: &mut TimelineDocumentEnvelopeWire,
) -> &mut valle_timeline::internal::wire::document::CaptionWire {
    timeline
        .document
        .captions
        .tracks
        .iter_mut()
        .flat_map(|track| &mut track.items)
        .find_map(|item| match item {
            CaptionItemWire::Clip(caption) => Some(caption),
            CaptionItemWire::Gap(_) => None,
        })
        .expect("synthetic timeline contains one caption")
}

trait ApproxValue: Clone + Debug {
    fn assert_scaled(expected: &Self, actual: &Self, factor: f64, context: &str);
}

impl ApproxValue for f64 {
    fn assert_scaled(expected: &Self, actual: &Self, factor: f64, context: &str) {
        approx(*actual, *expected * factor, context);
    }
}

impl ApproxValue for [f64; 2] {
    fn assert_scaled(expected: &Self, actual: &Self, factor: f64, context: &str) {
        for index in 0..2 {
            approx(
                actual[index],
                expected[index] * factor,
                &format!("{context}[{index}]"),
            );
        }
    }
}

impl ApproxValue for [f64; 4] {
    fn assert_scaled(expected: &Self, actual: &Self, factor: f64, context: &str) {
        for index in 0..4 {
            approx(
                actual[index],
                expected[index] * factor,
                &format!("{context}[{index}]"),
            );
        }
    }
}

fn approx(actual: f64, expected: f64, context: &str) {
    let tolerance = 1e-10_f64.max(expected.abs() * 1e-10);
    assert!(
        (actual - expected).abs() <= tolerance,
        "{context}: expected {expected}, got {actual}"
    );
}

fn assert_param_scaled<T: ApproxValue>(
    expected: &ParamWire<T>,
    actual: &ParamWire<T>,
    factor: f64,
    channel: &str,
) {
    match (expected, actual) {
        (ParamWire::Constant(expected), ParamWire::Constant(actual)) => {
            T::assert_scaled(&expected.value, &actual.value, factor, channel);
        }
        (ParamWire::Curve(expected), ParamWire::Curve(actual)) => {
            assert_eq!(actual.id, expected.id, "{channel} curve id");
            assert_eq!(actual.interpolation, expected.interpolation, "{channel}");
            assert_eq!(actual.extrapolation, expected.extrapolation, "{channel}");
            assert_eq!(
                actual.keyframes.len(),
                expected.keyframes.len(),
                "{channel}"
            );
            for (index, (expected, actual)) in
                expected.keyframes.iter().zip(&actual.keyframes).enumerate()
            {
                assert_eq!(actual.id, expected.id, "{channel} keyframe {index} id");
                assert_eq!(actual.time, expected.time, "{channel} keyframe {index}");
                assert_eq!(
                    actual.out_easing, expected.out_easing,
                    "{channel} keyframe {index} easing"
                );
                T::assert_scaled(
                    &expected.value,
                    &actual.value,
                    factor,
                    &format!("{channel} keyframe {index}"),
                );
            }
        }
        _ => panic!("{channel}: parameter kind changed with canvas size"),
    }
}

fn assert_presentation_scaled(
    expected: &CaptionPresentationWire,
    actual: &CaptionPresentationWire,
    pixel_factor: f64,
) {
    assert_param_scaled(&expected.opacity, &actual.opacity, 1.0, "opacity");
    assert_param_scaled(
        &expected.translation,
        &actual.translation,
        pixel_factor,
        "translation",
    );
    assert_param_scaled(&expected.scale, &actual.scale, 1.0, "scale");
    assert_param_scaled(&expected.rotation, &actual.rotation, 1.0, "rotation");
    assert_param_scaled(&expected.clip_inset, &actual.clip_inset, 1.0, "clipInset");
    assert_param_scaled(
        &expected.blur_sigma,
        &actual.blur_sigma,
        pixel_factor,
        "blurSigma",
    );
}

fn boundary<T: Clone>(parameter: &ParamWire<T>, last: bool) -> T {
    match parameter {
        ParamWire::Constant(constant) => constant.value.clone(),
        ParamWire::Curve(curve) => {
            let keyframe = if last {
                curve.keyframes.last()
            } else {
                curve.keyframes.first()
            }
            .expect("curve has a boundary keyframe");
            keyframe.value.clone()
        }
    }
}

fn assert_identity_boundary(presentation: &CaptionPresentationWire, last: bool, context: &str) {
    approx(boundary(&presentation.opacity, last), 1.0, context);
    <[f64; 2]>::assert_scaled(
        &[0.0, 0.0],
        &boundary(&presentation.translation, last),
        1.0,
        context,
    );
    approx(boundary(&presentation.scale, last), 1.0, context);
    approx(boundary(&presentation.rotation, last), 0.0, context);
    <[f64; 4]>::assert_scaled(
        &[0.0; 4],
        &boundary(&presentation.clip_inset, last),
        1.0,
        context,
    );
    approx(boundary(&presentation.blur_sigma, last), 0.0, context);
}

fn assert_sparse_typed<T>(parameter: &ParamWire<T>, preset_id: &str, channel: &str) -> usize {
    let ParamWire::Curve(curve) = parameter else {
        return 0;
    };
    assert!(
        (2..=3).contains(&curve.keyframes.len()),
        "{preset_id} {channel} should have two or three keyframes, got {}",
        curve.keyframes.len()
    );
    for (index, keyframe) in curve.keyframes.iter().enumerate() {
        if index + 1 == curve.keyframes.len() {
            assert_eq!(
                keyframe.out_easing, None,
                "{preset_id} {channel} final keyframe cannot ease past its segment"
            );
        } else {
            assert!(
                keyframe.out_easing.is_some(),
                "{preset_id} {channel} keyframe {index} needs typed easing"
            );
        }
    }
    1
}

fn assert_reversed_easing(enter: Option<&EasingWire>, exit: Option<&EasingWire>, context: &str) {
    match (enter, exit) {
        (None, None) => {}
        (Some(EasingWire::Named(enter)), Some(EasingWire::Named(exit))) => {
            let expected = match enter {
                NamedEasingWire::Linear => NamedEasingWire::Linear,
                NamedEasingWire::EaseIn => NamedEasingWire::EaseOut,
                NamedEasingWire::EaseOut => NamedEasingWire::EaseIn,
                NamedEasingWire::EaseInOut => NamedEasingWire::EaseInOut,
                NamedEasingWire::Ease => panic!(
                    "{context}: CSS `ease` has no named exact time reverse; use cubic-bezier"
                ),
            };
            assert_eq!(*exit, expected, "{context}");
        }
        (Some(EasingWire::Named(NamedEasingWire::Ease)), Some(EasingWire::CubicBezier(exit))) => {
            assert_bezier(exit, [0.75, 0.0, 0.75, 0.9], context);
        }
        (Some(EasingWire::CubicBezier(enter)), Some(EasingWire::CubicBezier(exit))) => {
            assert_bezier(
                exit,
                [
                    1.0 - enter.x2,
                    1.0 - enter.y2,
                    1.0 - enter.x1,
                    1.0 - enter.y1,
                ],
                context,
            );
        }
        _ => panic!("{context}: easing is not the exact time reverse"),
    }
}

fn assert_bezier(actual: &CubicBezierEasingWire, expected: [f64; 4], context: &str) {
    approx(actual.x1, expected[0], context);
    approx(actual.y1, expected[1], context);
    approx(actual.x2, expected[2], context);
    approx(actual.y2, expected[3], context);
}

fn assert_reversed_param<T: ApproxValue>(
    enter: &ParamWire<T>,
    exit: &ParamWire<T>,
    duration: ExactRational,
    channel: &str,
) {
    match (enter, exit) {
        (ParamWire::Constant(enter), ParamWire::Constant(exit)) => {
            T::assert_scaled(&enter.value, &exit.value, 1.0, channel);
        }
        (ParamWire::Curve(enter), ParamWire::Curve(exit)) => {
            assert_eq!(enter.interpolation, exit.interpolation, "{channel}");
            assert_eq!(enter.extrapolation, exit.extrapolation, "{channel}");
            assert_eq!(enter.keyframes.len(), exit.keyframes.len(), "{channel}");
            let count = enter.keyframes.len();
            for index in 0..count {
                let enter_keyframe = &enter.keyframes[index];
                let exit_keyframe = &exit.keyframes[count - 1 - index];
                assert_eq!(
                    enter_keyframe
                        .time
                        .checked_add(exit_keyframe.time)
                        .expect("test time addition"),
                    duration,
                    "{channel} keyframe {index} time"
                );
                T::assert_scaled(
                    &enter_keyframe.value,
                    &exit_keyframe.value,
                    1.0,
                    &format!("{channel} keyframe {index} value"),
                );
            }
            for exit_index in 0..count.saturating_sub(1) {
                let enter_index = count - 2 - exit_index;
                assert_reversed_easing(
                    enter.keyframes[enter_index].out_easing.as_ref(),
                    exit.keyframes[exit_index].out_easing.as_ref(),
                    &format!("{channel} segment {enter_index}"),
                );
            }
            assert_eq!(
                exit.keyframes
                    .last()
                    .and_then(|keyframe| keyframe.out_easing.as_ref()),
                None,
                "{channel} final exit keyframe"
            );
        }
        _ => panic!("{channel}: time-reversed pair changed parameter kind"),
    }
}

fn assert_reversed_presentation(
    enter: &CaptionPresentationWire,
    exit: &CaptionPresentationWire,
    duration: RationalTime,
) {
    let duration = duration.into_exact();
    assert_reversed_param(&enter.opacity, &exit.opacity, duration, "opacity");
    assert_reversed_param(
        &enter.translation,
        &exit.translation,
        duration,
        "translation",
    );
    assert_reversed_param(&enter.scale, &exit.scale, duration, "scale");
    assert_reversed_param(&enter.rotation, &exit.rotation, duration, "rotation");
    assert_reversed_param(&enter.clip_inset, &exit.clip_inset, duration, "clipInset");
    assert_reversed_param(&enter.blur_sigma, &exit.blur_sigma, duration, "blurSigma");
}

fn cubic_axis(parameter: f64, control_1: f64, control_2: f64) -> f64 {
    let inverse = 1.0 - parameter;
    3.0 * inverse * inverse * parameter * control_1
        + 3.0 * inverse * parameter * parameter * control_2
        + parameter * parameter * parameter
}

fn cubic_bezier_progress(progress: f64, x1: f64, y1: f64, x2: f64, y2: f64) -> f64 {
    if progress <= 0.0 {
        return 0.0;
    }
    if progress >= 1.0 {
        return 1.0;
    }

    // Timeline requires monotone x controls. Bisection gives a deterministic inverse for x(t),
    // including curves whose y controls intentionally overshoot the unit interval.
    let mut lower = 0.0;
    let mut upper = 1.0;
    for _ in 0..64 {
        let parameter = (lower + upper) * 0.5;
        if cubic_axis(parameter, x1, x2) < progress {
            lower = parameter;
        } else {
            upper = parameter;
        }
    }
    cubic_axis((lower + upper) * 0.5, y1, y2)
}

fn eased_progress(easing: Option<&EasingWire>, progress: f64) -> f64 {
    match easing {
        None | Some(EasingWire::Named(NamedEasingWire::Linear)) => progress,
        Some(EasingWire::Named(named)) => {
            let [x1, y1, x2, y2] = match named {
                NamedEasingWire::Linear => unreachable!("linear handled above"),
                NamedEasingWire::Ease => [0.25, 0.1, 0.25, 1.0],
                NamedEasingWire::EaseIn => [0.42, 0.0, 1.0, 1.0],
                NamedEasingWire::EaseOut => [0.0, 0.0, 0.58, 1.0],
                NamedEasingWire::EaseInOut => [0.42, 0.0, 0.58, 1.0],
            };
            cubic_bezier_progress(progress, x1, y1, x2, y2)
        }
        Some(EasingWire::CubicBezier(bezier)) => {
            cubic_bezier_progress(progress, bezier.x1, bezier.y1, bezier.x2, bezier.y2)
        }
    }
}

fn scalar_at(parameter: &ParamWire<f64>, time: ExactRational) -> f64 {
    match parameter {
        ParamWire::Constant(constant) => constant.value,
        ParamWire::Curve(curve) => {
            if time <= curve.keyframes[0].time {
                return curve.keyframes[0].value;
            }
            for pair in curve.keyframes.windows(2) {
                if time <= pair[1].time {
                    let elapsed = time
                        .checked_sub(pair[0].time)
                        .expect("ordered curve keyframes")
                        .as_f64();
                    let duration = pair[1]
                        .time
                        .checked_sub(pair[0].time)
                        .expect("strict curve time")
                        .as_f64();
                    let progress = eased_progress(pair[0].out_easing.as_ref(), elapsed / duration);
                    return pair[0].value + (pair[1].value - pair[0].value) * progress;
                }
            }
            curve.keyframes.last().unwrap().value
        }
    }
}

fn vec2_at(parameter: &ParamWire<[f64; 2]>, time: ExactRational) -> [f64; 2] {
    match parameter {
        ParamWire::Constant(constant) => constant.value,
        ParamWire::Curve(curve) => {
            if time <= curve.keyframes[0].time {
                return curve.keyframes[0].value;
            }
            for pair in curve.keyframes.windows(2) {
                if time <= pair[1].time {
                    let elapsed = time
                        .checked_sub(pair[0].time)
                        .expect("ordered curve keyframes")
                        .as_f64();
                    let duration = pair[1]
                        .time
                        .checked_sub(pair[0].time)
                        .expect("strict curve time")
                        .as_f64();
                    let progress = eased_progress(pair[0].out_easing.as_ref(), elapsed / duration);
                    return [
                        pair[0].value[0] + (pair[1].value[0] - pair[0].value[0]) * progress,
                        pair[0].value[1] + (pair[1].value[1] - pair[0].value[1]) * progress,
                    ];
                }
            }
            curve.keyframes.last().unwrap().value
        }
    }
}

fn vec4_at(parameter: &ParamWire<[f64; 4]>, time: ExactRational) -> [f64; 4] {
    match parameter {
        ParamWire::Constant(constant) => constant.value,
        ParamWire::Curve(curve) => {
            if time <= curve.keyframes[0].time {
                return curve.keyframes[0].value;
            }
            for pair in curve.keyframes.windows(2) {
                if time <= pair[1].time {
                    let elapsed = time
                        .checked_sub(pair[0].time)
                        .expect("ordered curve keyframes")
                        .as_f64();
                    let duration = pair[1]
                        .time
                        .checked_sub(pair[0].time)
                        .expect("strict curve time")
                        .as_f64();
                    let progress = eased_progress(pair[0].out_easing.as_ref(), elapsed / duration);
                    return std::array::from_fn(|index| {
                        pair[0].value[index]
                            + (pair[1].value[index] - pair[0].value[index]) * progress
                    });
                }
            }
            curve.keyframes.last().unwrap().value
        }
    }
}

fn max_abs_horizontal_translation(parameter: &ParamWire<[f64; 2]>) -> f64 {
    match parameter {
        ParamWire::Constant(constant) => constant.value[0].abs(),
        ParamWire::Curve(curve) => curve
            .keyframes
            .iter()
            .map(|keyframe| keyframe.value[0].abs())
            .fold(0.0, f64::max),
    }
}

fn assert_no_negative_zero(value: &Value, context: &str) {
    match value {
        Value::Number(number) => {
            if let Some(value) = number.as_f64() {
                assert!(
                    value != 0.0 || !value.is_sign_negative(),
                    "{context} contains negative zero"
                );
            }
        }
        Value::Array(values) => {
            for value in values {
                assert_no_negative_zero(value, context);
            }
        }
        Value::Object(values) => {
            for value in values.values() {
                assert_no_negative_zero(value, context);
            }
        }
        Value::Null | Value::Bool(_) | Value::String(_) => {}
    }
}

fn assert_stable_curve_ids<T>(parameter: &ParamWire<T>, owner: &str, channel: &str) {
    let ParamWire::Curve(curve) = parameter else {
        return;
    };
    assert_eq!(curve.id, format!("curve:{owner}:{channel}"));
    for (index, keyframe) in curve.keyframes.iter().enumerate() {
        assert_eq!(keyframe.id, format!("keyframe:{owner}:{channel}:{index}"));
    }
}

#[test]
fn catalog_is_a_closed_valle_native_pack() {
    assert_eq!(CAPTION_PRESET_PACK_ID, "valle.caption-presets");
    assert_eq!(CAPTION_PRESET_COUNT, 27);
    assert_eq!(CaptionPreset::ALL.len(), CAPTION_PRESET_COUNT);

    let expected = ENTER_IDS
        .into_iter()
        .chain(EXIT_IDS)
        .chain(DISPLAY_IDS)
        .collect::<BTreeSet<_>>();
    let actual = CaptionPreset::ALL
        .iter()
        .map(|preset| preset.id())
        .collect::<BTreeSet<_>>();
    assert_eq!(actual, expected);
    assert_eq!(actual.len(), CAPTION_PRESET_COUNT);
    assert_eq!(
        CaptionPreset::ALL
            .iter()
            .filter(|preset| preset.phase() == CaptionPresetPhase::Enter)
            .count(),
        ENTER_IDS.len()
    );
    assert_eq!(
        CaptionPreset::ALL
            .iter()
            .filter(|preset| preset.phase() == CaptionPresetPhase::Exit)
            .count(),
        EXIT_IDS.len()
    );
    assert_eq!(
        CaptionPreset::ALL
            .iter()
            .filter(|preset| preset.phase() == CaptionPresetPhase::Display)
            .count(),
        DISPLAY_IDS.len()
    );
    for preset in CaptionPreset::ALL {
        assert_eq!(CaptionPreset::from_id(preset.id()), Some(preset));
        assert!(preset.id().starts_with(match preset.phase() {
            CaptionPresetPhase::Enter => "enter.",
            CaptionPresetPhase::Display => "display.",
            CaptionPresetPhase::Exit => "exit.",
        }));
        let descriptor = preset.descriptor();
        assert_eq!(descriptor.id, preset.id());
        assert_eq!(descriptor.phase, preset.phase());
        assert!(!descriptor.label.is_empty());
        assert!(!descriptor.summary.is_empty());
        assert_eq!(
            descriptor.recommended_duration.is_some(),
            preset.phase() != CaptionPresetPhase::Display
        );
        assert_eq!(
            descriptor.recommended_rate,
            (preset.phase() == CaptionPresetPhase::Display).then_some(1.0)
        );
    }

    for unsupported in [
        "fade_in",
        "dissolve_in",
        "dissovle_in",
        "wave_in",
        "normal_display",
        "typewriter1_in",
        "zoomslightout_out",
    ] {
        assert_eq!(
            CaptionPreset::from_id(unsupported),
            None,
            "unsupported id `{unsupported}`"
        );
        assert!(matches!(
            parse_caption_preset(unsupported, CaptionPresetPhase::Enter),
            Err(CaptionPresetError::UnknownPresetId(id)) if id == unsupported
        ));
    }
}

#[test]
fn every_preset_enters_a_synthetic_canonical_timeline_without_pack_metadata() {
    for preset in CaptionPreset::ALL {
        let presentation = expanded(preset, [1080, 1920]);
        let encoded = serde_json::to_string(&presentation).expect("encode presentation");
        assert!(!encoded.contains(preset.id()), "{} leaked", preset.id());
        assert!(!encoded.contains(CAPTION_PRESET_PACK_ID));
        assert!(!encoded.contains("preset"));

        let mut candidate = synthetic_timeline();
        first_caption_mut(&mut candidate).presentation = presentation;
        CanonicalTimeline::try_from_wire(candidate).unwrap_or_else(|report| {
            panic!("`{}` failed canonical validation: {report:?}", preset.id())
        });
    }
}

#[test]
fn enter_and_exit_recipes_are_sparse_typed_and_frame_rate_independent() {
    let duration = seconds(1, 1);
    let frame_rates = [
        fps(24, 1),
        fps(25, 1),
        fps(30, 1),
        fps(30_000, 1_001),
        fps(60, 1),
    ];
    for preset in CaptionPreset::ALL
        .into_iter()
        .filter(|preset| preset.phase() != CaptionPresetPhase::Display)
    {
        let first = expand_caption_presets(&request_for(
            preset,
            [1080, 1920],
            duration,
            duration,
            frame_rates[0],
        ))
        .unwrap();
        let curve_count = assert_sparse_typed(&first.opacity, preset.id(), "opacity")
            + assert_sparse_typed(&first.translation, preset.id(), "translation")
            + assert_sparse_typed(&first.scale, preset.id(), "scale")
            + assert_sparse_typed(&first.rotation, preset.id(), "rotation")
            + assert_sparse_typed(&first.clip_inset, preset.id(), "clipInset")
            + assert_sparse_typed(&first.blur_sigma, preset.id(), "blurSigma");
        assert!(
            curve_count > 0,
            "{} must animate at least one channel",
            preset.id()
        );

        for frame_rate in &frame_rates[1..] {
            let candidate = expand_caption_presets(&request_for(
                preset,
                [1080, 1920],
                duration,
                duration,
                *frame_rate,
            ))
            .unwrap();
            assert_eq!(
                candidate,
                first,
                "{} must not bake simple curves at fps {}",
                preset.id(),
                frame_rate.into_exact()
            );
        }
    }
}

#[test]
fn pop_easing_and_display_bake_agree_on_the_frame_lattice() {
    let pop = preset("enter.pop", CaptionPresetPhase::Enter);
    let mut edge_request = request_for(pop, [1080, 1920], seconds(2, 1), seconds(1, 1), fps(30, 1));
    let edge_only = expand_caption_presets(&edge_request).unwrap();

    edge_request.display = Some(DisplayCaptionPreset {
        // Float is identity on the scale channel and starts only after the enter phase.
        preset: preset("display.float", CaptionPresetPhase::Display),
        rate: 1.0,
    });
    let frame_baked = expand_caption_presets(&edge_request).unwrap();

    for frame in 0..=30 {
        let time = ExactRational::new(frame, 30).unwrap();
        approx(
            scalar_at(&frame_baked.scale, time),
            scalar_at(&edge_only.scale, time),
            &format!("enter.pop scale at frame {frame}"),
        );
    }
    assert!(
        scalar_at(&edge_only.scale, ExactRational::new(23, 30).unwrap()) > 1.0,
        "the comparison must cover pop's overshoot, not only its endpoints"
    );
}

#[test]
fn enter_finishes_at_identity_and_exit_starts_at_identity() {
    let duration = seconds(1, 1);
    for id in ENTER_IDS {
        let presentation = expand_caption_presets(&request_for(
            preset(id, CaptionPresetPhase::Enter),
            [1080, 1920],
            duration,
            duration,
            fps(60, 1),
        ))
        .unwrap();
        assert_identity_boundary(&presentation, true, id);
    }
    for id in EXIT_IDS {
        let presentation = expand_caption_presets(&request_for(
            preset(id, CaptionPresetPhase::Exit),
            [1080, 1920],
            duration,
            duration,
            fps(60, 1),
        ))
        .unwrap();
        assert_identity_boundary(&presentation, false, id);
    }
}

#[test]
fn exit_recipes_are_exact_time_reversals_of_their_visual_enter_pairs() {
    let duration = seconds(1, 1);
    for (enter_id, exit_id) in TIME_REVERSED_PAIRS {
        let enter = expand_caption_presets(&request_for(
            preset(enter_id, CaptionPresetPhase::Enter),
            [1080, 1920],
            duration,
            duration,
            fps(60, 1),
        ))
        .unwrap();
        let exit = expand_caption_presets(&request_for(
            preset(exit_id, CaptionPresetPhase::Exit),
            [1080, 1920],
            duration,
            duration,
            fps(60, 1),
        ))
        .unwrap();
        assert_reversed_presentation(&enter, &exit, duration);
    }
}

#[test]
fn short_edge_scaling_only_changes_pixel_channels() {
    for preset in CaptionPreset::ALL {
        let reference = expanded(preset, [1080, 1920]);
        let portrait_720 = expanded(preset, [720, 1280]);
        let landscape_720 = expanded(preset, [1280, 720]);
        let portrait_2160 = expanded(preset, [2160, 3840]);
        assert_presentation_scaled(&reference, &portrait_720, 2.0 / 3.0);
        assert_presentation_scaled(&reference, &landscape_720, 2.0 / 3.0);
        assert_presentation_scaled(&reference, &portrait_2160, 2.0);
    }
}

#[test]
fn display_recipes_return_to_identity_at_cycle_boundaries() {
    let periods = [
        ("display.breathe", seconds(14, 5)),
        ("display.float", seconds(14, 5)),
        ("display.sway", seconds(13, 5)),
        ("display.pulse", seconds(8, 5)),
        ("display.shake", seconds(12, 5)),
    ];
    for (id, period) in periods {
        let presentation = expand_caption_presets(&request_for(
            preset(id, CaptionPresetPhase::Display),
            [1080, 1920],
            period,
            seconds(1, 1),
            fps(60, 1),
        ))
        .unwrap();
        assert_identity_boundary(&presentation, false, id);
        assert_identity_boundary(&presentation, true, id);
    }
}

#[test]
fn shake_has_visible_horizontal_samples_at_common_frame_rates() {
    let shake = preset("display.shake", CaptionPresetPhase::Display);
    for frame_rate in [24, 25, 30, 60] {
        let presentation = expand_caption_presets(&request_for(
            shake,
            [1080, 1920],
            seconds(1, 1),
            seconds(1, 1),
            fps(frame_rate, 1),
        ))
        .unwrap();
        let amplitude = max_abs_horizontal_translation(&presentation.translation);
        assert!(
            amplitude > 0.25,
            "display.shake needs visible horizontal motion at {frame_rate} fps, got {amplitude} px"
        );
    }
}

#[test]
fn finite_display_rate_that_overflows_elapsed_time_fails_closed() {
    for id in ["display.pulse", "display.shake"] {
        let mut request = request_for(
            preset(id, CaptionPresetPhase::Display),
            [1080, 1920],
            seconds(2, 1),
            seconds(1, 1),
            fps(30, 1),
        );
        request.display.as_mut().unwrap().rate = f64::MAX;
        assert!(
            expand_caption_presets(&request).is_err(),
            "{id} must reject non-finite elapsed * rate instead of becoming identity"
        );
    }
}

#[test]
fn display_terminal_style_is_continuous_and_held_through_exit() {
    let exit_start = seconds(17, 10);
    let next_frame = ExactRational::new(103, 60).unwrap();
    let display = preset("display.float", CaptionPresetPhase::Display);

    let display_only = expand_caption_presets(&request_for(
        display,
        [1080, 1920],
        exit_start,
        seconds(1, 1),
        fps(60, 1),
    ))
    .unwrap();
    let terminal = boundary(&display_only.translation, true);
    assert!(
        terminal[1].abs() > 1.0,
        "test needs a visibly non-identity terminal style"
    );

    let mut combined = request_for(
        display,
        [1080, 1920],
        seconds(2, 1),
        seconds(1, 1),
        fps(60, 1),
    );
    combined.exit = Some(TimedCaptionPreset {
        preset: preset("exit.fade", CaptionPresetPhase::Exit),
        duration: seconds(3, 10),
    });
    let combined = expand_caption_presets(&combined).unwrap();
    let at_boundary = vec2_at(&combined.translation, exit_start.into_exact());
    let after_boundary = vec2_at(&combined.translation, next_frame);
    <[f64; 2]>::assert_scaled(&terminal, &at_boundary, 1.0, "display at exitStart");
    <[f64; 2]>::assert_scaled(
        &terminal,
        &after_boundary,
        1.0,
        "display one frame into exit",
    );
    approx(
        scalar_at(&combined.opacity, exit_start.into_exact()),
        1.0,
        "exit opacity at boundary",
    );
    assert!(
        scalar_at(&combined.opacity, next_frame) < 1.0,
        "exit must begin after its exact boundary"
    );
}

#[test]
fn enter_display_exit_composition_does_not_mask_the_exit_wipe() {
    let exit_start = ExactRational::new(17, 10).unwrap();
    let next_frame = ExactRational::new(103, 60).unwrap();
    let mut request = request_for(
        preset("enter.wipe-right", CaptionPresetPhase::Enter),
        [1080, 1920],
        seconds(2, 1),
        seconds(3, 10),
        fps(60, 1),
    );
    request.display = Some(DisplayCaptionPreset {
        preset: preset("display.breathe", CaptionPresetPhase::Display),
        rate: 1.0,
    });
    request.exit = Some(TimedCaptionPreset {
        preset: preset("exit.wipe-left", CaptionPresetPhase::Exit),
        duration: seconds(3, 10),
    });

    let presentation = expand_caption_presets(&request).unwrap();
    let at_boundary = vec4_at(&presentation.clip_inset, exit_start);
    let after_boundary = vec4_at(&presentation.clip_inset, next_frame);
    let at_end = vec4_at(&presentation.clip_inset, seconds(2, 1).into_exact());
    <[f64; 4]>::assert_scaled(&[0.0; 4], &at_boundary, 1.0, "exit wipe boundary");
    assert!(
        after_boundary[1] > at_boundary[1],
        "exit wipe must start changing clipInset after exitStart"
    );
    <[f64; 4]>::assert_scaled(&[0.0, 1.0, 0.0, 0.0], &at_end, 1.0, "exit wipe terminal");
}

#[test]
fn every_public_name_has_a_distinct_recipe_signature() {
    let mut signatures = BTreeMap::<String, &'static str>::new();
    for preset in CaptionPreset::ALL {
        let signature = serde_json::to_string(&expanded(preset, [1080, 1920]))
            .expect("encode recipe signature");
        if let Some(previous) = signatures.insert(signature, preset.id()) {
            panic!(
                "presets `{previous}` and `{}` are renamed copies of the same recipe",
                preset.id()
            );
        }
    }
}

#[test]
fn invalid_phase_windows_canvas_rate_and_sample_budget_fail_closed() {
    let enter = preset("enter.fade", CaptionPresetPhase::Enter);
    let exit = preset("exit.fade", CaptionPresetPhase::Exit);
    let display = preset("display.breathe", CaptionPresetPhase::Display);

    let mut wrong_phase = request_for(exit, [1080, 1920], seconds(1, 1), seconds(1, 4), fps(60, 1));
    wrong_phase.enter = wrong_phase.exit.take();
    assert!(matches!(
        expand_caption_presets(&wrong_phase),
        Err(CaptionPresetError::PhaseMismatch {
            phase: CaptionPresetPhase::Enter,
            actual: CaptionPresetPhase::Exit,
            ..
        })
    ));

    let mut overflow = request_for(
        enter,
        [1080, 1920],
        seconds(1, 1),
        seconds(3, 4),
        fps(60, 1),
    );
    overflow.exit = Some(TimedCaptionPreset {
        preset: exit,
        duration: seconds(3, 4),
    });
    assert_eq!(
        expand_caption_presets(&overflow),
        Err(CaptionPresetError::InvalidPhaseWindow)
    );

    let mut exact_fit = overflow.clone();
    exact_fit.enter.as_mut().unwrap().duration = seconds(1, 2);
    exact_fit.exit.as_mut().unwrap().duration = seconds(1, 2);
    assert!(expand_caption_presets(&exact_fit).is_ok());
    exact_fit.display = Some(DisplayCaptionPreset {
        preset: display,
        rate: 1.0,
    });
    assert_eq!(
        expand_caption_presets(&exact_fit),
        Err(CaptionPresetError::InvalidDisplayWindow)
    );

    for canvas_size in [[0, 1080], [1920, 0], [0, 0]] {
        let invalid = request_for(enter, canvas_size, seconds(1, 1), seconds(1, 4), fps(60, 1));
        assert_eq!(
            expand_caption_presets(&invalid),
            Err(CaptionPresetError::InvalidCanvasSize),
            "canvas {canvas_size:?}"
        );
    }

    for rate in [0.0, -1.0, f64::INFINITY, f64::NAN] {
        let mut invalid = request_for(
            display,
            [1080, 1920],
            seconds(1, 1),
            seconds(1, 4),
            fps(60, 1),
        );
        invalid.display.as_mut().unwrap().rate = rate;
        assert!(matches!(
            expand_caption_presets(&invalid),
            Err(CaptionPresetError::InvalidDisplayRate)
        ));
    }

    let too_many_frames = seconds(MAX_CAPTION_PRESET_SAMPLES as i64 + 1, 60);
    let over_budget = request_for(
        display,
        [1080, 1920],
        too_many_frames,
        seconds(1, 4),
        fps(60, 1),
    );
    assert!(matches!(
        expand_caption_presets(&over_budget),
        Err(CaptionPresetError::SampleBudgetExceeded {
            maximum: MAX_CAPTION_PRESET_SAMPLES
        })
    ));
}

#[test]
fn expansion_is_deterministic_with_stable_ids_and_no_negative_zero() {
    for preset in CaptionPreset::ALL {
        let request = request_for(
            preset,
            [1080, 1920],
            seconds(4, 1),
            seconds(3, 10),
            fps(60, 1),
        );
        let first = expand_caption_presets(&request).unwrap();
        let second = expand_caption_presets(&request).unwrap();
        assert_eq!(first, second, "{} must be deterministic", preset.id());

        assert_stable_curve_ids(&first.opacity, &request.owner_id, "opacity");
        assert_stable_curve_ids(&first.translation, &request.owner_id, "translation");
        assert_stable_curve_ids(&first.scale, &request.owner_id, "scale");
        assert_stable_curve_ids(&first.rotation, &request.owner_id, "rotation");
        assert_stable_curve_ids(&first.clip_inset, &request.owner_id, "clipInset");
        assert_stable_curve_ids(&first.blur_sigma, &request.owner_id, "blurSigma");

        let encoded = serde_json::to_value(&first).expect("encode presentation values");
        assert_no_negative_zero(&encoded, preset.id());
    }
}
