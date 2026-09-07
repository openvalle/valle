use serde_json::json;
use valle_timeline::{
    Timeline, TimelineDecodeError, TimelineJsonIssue, decode_timeline, timeline_bytes,
    wire::timeline::{
        EasingWire, TimelineFrameRateWire, TimelineKeyframeWire, TimelineParamWire,
        TimelineVisualSourceWire,
    },
};

fn solid_with_curve() -> String {
    serde_json::to_string(&json!({
        "canvas": { "width": 1920, "height": 1080, "fps": "30000/1001" },
        "tracks": {
            "visual": [{
                "clips": [{
                    "start": 0.133333333333,
                    "duration": 0.266666666666,
                    "kind": "solid",
                    "color": "#ffffffff",
                    "anchor": [0.9299999999999999, -0.0000004],
                    "opacity": {
                        "keyframes": [[
                            5e-7,
                            0.9299999999999999,
                            {
                                "type": "cubic-bezier",
                                "x1": 0.133333333333,
                                "y1": 0.266666666666,
                                "x2": 0.9299999999999999,
                                "y2": -0.0000004
                            }
                        ]]
                    }
                }]
            }]
        }
    }))
    .unwrap()
}

fn caption(text_and_runs: serde_json::Value, effects: serde_json::Value) -> String {
    let mut clip = json!({ "start": 0, "duration": 1 });
    clip.as_object_mut()
        .unwrap()
        .extend(text_and_runs.as_object().unwrap().clone());
    clip.as_object_mut()
        .unwrap()
        .extend(effects.as_object().unwrap().clone());
    serde_json::to_string(&json!({
        "canvas": { "width": 1280, "height": 720, "fps": 30 },
        "resources": { "font-main": "https://cdn.example.com/font.woff2" },
        "tracks": {
            "caption": [{
                "style": { "font": "font-main" },
                "clips": [clip]
            }]
        }
    }))
    .unwrap()
}

#[test]
fn exact_times_and_closed_scalars_normalize_to_q6_and_reencode_idempotently() {
    let timeline = decode_timeline(&solid_with_curve()).unwrap();
    assert!(matches!(
        timeline.wire().canvas.fps,
        TimelineFrameRateWire::Rational(ref fps) if fps == "30000/1001"
    ));
    let clip = &timeline.wire().tracks.visual[0].clips[0];
    assert_eq!(clip.start.token(), "0.133333");
    assert_eq!(clip.duration.token(), "0.266667");
    assert_eq!(clip.anchor, Some([0.93, 0.0]));
    let Some(TimelineParamWire::Curve(curve)) = &clip.opacity else {
        panic!("expected opacity curve")
    };
    let TimelineKeyframeWire::Eased((time, value, easing)) = &curve.keyframes[0] else {
        panic!("expected eased keyframe")
    };
    assert_eq!(time.token(), "0.000001");
    assert_eq!(*value, 0.93);
    let EasingWire::CubicBezier(bezier) = easing else {
        panic!("expected cubic bezier")
    };
    assert_eq!(
        [bezier.x1, bezier.y1, bezier.x2, bezier.y2],
        [0.133333, 0.266667, 0.93, 0.0]
    );

    let once = timeline_bytes(&timeline).unwrap();
    let decoded_again = decode_timeline(std::str::from_utf8(&once).unwrap()).unwrap();
    let twice = timeline_bytes(&decoded_again).unwrap();
    assert_eq!(once, twice);
}

#[test]
fn motion_prop_json_numbers_are_recursively_quantized_to_q6() {
    let json = serde_json::to_string(&json!({
        "canvas": { "width": 1280, "height": 720, "fps": 30 },
        "resources": { "motion-main": "https://cdn.example.com/main.js" },
        "tracks": {
            "visual": [{
                "clips": [{
                    "start": 0,
                    "duration": 1,
                    "kind": "motion",
                    "component": "motion-main",
                    "sourceDuration": 1.266666666666,
                    "props": {
                        "amount": 0.133333333333,
                        "nested": {
                            "values": [1, 0.929999999999, true, "unchanged"]
                        },
                        "curve": {
                            "keyframes": [[
                                0,
                                0.266666666666,
                                {
                                    "type": "cubic-bezier",
                                    "x1": 0.133333333333,
                                    "y1": 0.266666666666,
                                    "x2": 0.929999999999,
                                    "y2": 1
                                }
                            ]]
                        }
                    },
                    "cues": {
                        "main": {
                            "type": "source-range",
                            "start": 0.133333333333,
                            "end": 0.266666666666,
                            "enterDuration": 5e-7
                        }
                    },
                    "phases": { "exitDuration": 0.266666666666 }
                }]
            }]
        }
    }))
    .unwrap();
    let timeline = decode_timeline(&json).unwrap();
    let TimelineVisualSourceWire::Motion { props, .. } =
        &timeline.wire().tracks.visual[0].clips[0].source
    else {
        panic!("expected Motion source")
    };
    assert_eq!(props["amount"], TimelineParamWire::Value(json!(0.133333)));
    assert_eq!(
        props["nested"],
        TimelineParamWire::Value(json!({
            "values": [1, 0.93, true, "unchanged"]
        }))
    );
    let encoded: serde_json::Value =
        serde_json::from_slice(&timeline_bytes(&timeline).unwrap()).unwrap();
    let source = &encoded["tracks"]["visual"][0]["clips"][0];
    assert_eq!(source["sourceDuration"], json!(1.266667));
    assert_eq!(source["cues"]["main"]["enterDuration"], json!(0.000001));
    assert_eq!(source["phases"]["exitDuration"], json!(0.266667));
    assert_eq!(source["props"]["curve"]["keyframes"][0][1], json!(0.266667));
    assert_eq!(
        source["props"]["curve"]["keyframes"][0][2]["x1"],
        json!(0.133333)
    );
    assert!(source.get("source_duration").is_none());
}

#[test]
fn motion_runtime_data_is_not_part_of_timeline() {
    let value = json!({
        "canvas": { "width": 1280, "height": 720, "fps": 30 },
        "resources": { "motion-main": "https://cdn.example.com/main.js" },
        "tracks": {
            "visual": [{
                "clips": [{
                    "start": 0,
                    "duration": 1,
                    "kind": "motion",
                    "component": "motion-main",
                    "data": { "title": "already baked into the artifact" }
                }]
            }]
        }
    });
    assert!(matches!(
        decode_timeline(&serde_json::to_string(&value).unwrap()),
        Err(TimelineDecodeError::InvalidShape)
    ));
}

#[test]
fn every_closed_scalar_family_is_quantized() {
    let json = serde_json::to_string(&json!({
        "canvas": { "width": 1280, "height": 720, "fps": 30 },
        "resources": {
            "audio-main": "https://cdn.example.com/main.wav",
            "font-main": "https://cdn.example.com/main.woff2"
        },
        "tracks": {
            "audio": [{
                "clips": [{
                    "start": 0,
                    "duration": 2,
                    "src": "audio-main",
                    "gain": 0.929999999999,
                    "pan": -0.133333333333
                }]
            }],
            "adjustment": [{
                "clips": [{
                    "start": 0,
                    "duration": 2,
                    "kind": "color-grade",
                    "temperature": 0.133333333333
                }]
            }],
            "caption": [{
                "style": {
                    "font": "font-main",
                    "fontSize": 48.1234567,
                    "shadow": {
                        "color": "#000000ff",
                        "offset": [0.133333333333, 0.266666666666],
                        "blur": 2.929999999999
                    }
                },
                "layout": { "region": [0.133333333333, 0.266666666666, 0.929999999999, 1] },
                "clips": [
                    {
                        "start": 0,
                        "duration": 1,
                        "text": "first",
                        "layout": { "region": [0.133333333333, 0.266666666666, 0.929999999999, 1] },
                        "behavior": { "type": "scroll", "axis": "horizontal", "speed": 1.133333333333 },
                        "display": { "preset": "breathe", "rate": 0.929999999999 }
                    },
                    {
                        "start": 1,
                        "duration": 1,
                        "runs": [{ "text": "second", "fontSize": 32.266666666666 }],
                        "presentation": {
                            "opacity": 0.929999999999,
                            "translation": [0.133333333333, 0.266666666666],
                            "scale": 1.133333333333,
                            "rotation": 2.266666666666,
                            "clipInset": [0.133333333333, 0.266666666666, 0.929999999999, 0],
                            "blur": 3.133333333333
                        }
                    }
                ]
            }]
        }
    }))
    .unwrap();
    let value: serde_json::Value =
        serde_json::from_slice(&timeline_bytes(&decode_timeline(&json).unwrap()).unwrap()).unwrap();
    assert_eq!(value["tracks"]["audio"][0]["clips"][0]["gain"], json!(0.93));
    assert_eq!(
        value["tracks"]["audio"][0]["clips"][0]["pan"],
        json!(-0.133333)
    );
    assert_eq!(
        value["tracks"]["adjustment"][0]["clips"][0]["temperature"],
        json!(0.133333)
    );
    assert_eq!(
        value["tracks"]["caption"][0]["style"]["fontSize"],
        json!(48.123457)
    );
    assert_eq!(
        value["tracks"]["caption"][0]["style"]["shadow"]["offset"],
        json!([0.133333, 0.266667])
    );
    assert_eq!(
        value["tracks"]["caption"][0]["layout"]["region"],
        json!([0.133333, 0.266667, 0.93, 1])
    );
    assert_eq!(
        value["tracks"]["caption"][0]["clips"][0]["behavior"]["speed"],
        json!(1.133333)
    );
    assert_eq!(
        value["tracks"]["caption"][0]["clips"][0]["display"]["rate"],
        json!(0.93)
    );
    assert_eq!(
        value["tracks"]["caption"][0]["clips"][1]["runs"][0]["fontSize"],
        json!(32.266667)
    );
    assert_eq!(
        value["tracks"]["caption"][0]["clips"][1]["presentation"]["translation"],
        json!([0.133333, 0.266667])
    );
    assert_eq!(
        value["tracks"]["caption"][0]["clips"][1]["presentation"]["clipInset"],
        json!([0.133333, 0.266667, 0.93, 0])
    );
}

#[test]
fn strict_json_unsupported_shape_fps_and_flattened_source_are_closed() {
    let duplicate = r#"{"canvas":{"width":1,"height":1,"fps":30},"canvas":{"width":1,"height":1,"fps":30},"tracks":{}}"#;
    assert!(matches!(
        decode_timeline(duplicate),
        Err(TimelineDecodeError::CanonicalJson {
            issue: TimelineJsonIssue::DuplicateObjectKey,
            ..
        })
    ));
    assert!(matches!(
        decode_timeline(&format!("\u{feff}{}", solid_with_curve())),
        Err(TimelineDecodeError::CanonicalJson {
            issue: TimelineJsonIssue::Utf8Bom,
            ..
        })
    ));
    let unsafe_integer = solid_with_curve().replace("\"width\":1920", "\"width\":9007199254740992");
    assert!(matches!(
        decode_timeline(&unsafe_integer),
        Err(TimelineDecodeError::CanonicalJson {
            issue: TimelineJsonIssue::UnsafeInteger,
            ..
        })
    ));

    let unsupported_version = solid_with_curve().replacen("{", "{\"version\":2,", 1);
    assert!(matches!(
        decode_timeline(&unsupported_version),
        Err(TimelineDecodeError::InvalidShape)
    ));
    let mut unsupported_track_type: serde_json::Value =
        serde_json::from_str(&solid_with_curve()).unwrap();
    unsupported_track_type["tracks"]["visual"][0]["type"] = json!("visual");
    assert!(matches!(
        decode_timeline(&serde_json::to_string(&unsupported_track_type).unwrap()),
        Err(TimelineDecodeError::InvalidShape)
    ));
    let mut unsupported_track_type_name: serde_json::Value =
        serde_json::from_str(&solid_with_curve()).unwrap();
    unsupported_track_type_name["tracks"]["visual"][0]["trackType"] = json!("visual");
    assert!(matches!(
        decode_timeline(&serde_json::to_string(&unsupported_track_type_name).unwrap()),
        Err(TimelineDecodeError::InvalidShape)
    ));
    let unsupported_track_list = serde_json::to_string(&json!({
        "canvas": { "width": 1920, "height": 1080, "fps": 30 },
        "tracks": [{ "type": "visual", "clips": [] }]
    }))
    .unwrap();
    assert!(matches!(
        decode_timeline(&unsupported_track_list),
        Err(TimelineDecodeError::InvalidShape)
    ));
    for unsupported_slot in ["subtitle", "subtitles", "effect"] {
        let unsupported = serde_json::to_string(&json!({
            "canvas": { "width": 1920, "height": 1080, "fps": 30 },
            "tracks": { (unsupported_slot): [] }
        }))
        .unwrap();
        assert!(matches!(
            decode_timeline(&unsupported),
            Err(TimelineDecodeError::InvalidShape)
        ));
    }
    let noncanonical_fps = solid_with_curve().replace("30000/1001", "60000/2002");
    assert!(matches!(
        decode_timeline(&noncanonical_fps),
        Err(TimelineDecodeError::InvalidShape)
    ));
    let unknown =
        solid_with_curve().replace("\"kind\":\"solid\"", "\"kind\":\"solid\",\"bogus\":true");
    assert!(matches!(
        decode_timeline(&unknown),
        Err(TimelineDecodeError::InvalidShape)
    ));
    let wrong_variant_field = solid_with_curve().replace(
        "\"kind\":\"solid\"",
        "\"kind\":\"solid\",\"src\":\"unused\"",
    );
    assert!(matches!(
        decode_timeline(&wrong_variant_field),
        Err(TimelineDecodeError::InvalidShape)
    ));

    let decimal_fps = solid_with_curve().replace("\"30000/1001\"", "29.97002997003");
    let timeline = decode_timeline(&decimal_fps).unwrap();
    assert!(matches!(
        timeline.wire().canvas.fps,
        TimelineFrameRateWire::Decimal(ref fps) if fps.token() == "29.97003"
    ));
}

#[test]
fn resources_caption_content_and_adjustment_range_are_validated() {
    let invalid_alias = solid_with_curve().replace(
        "\"tracks\":{",
        "\"resources\":{\"bad:alias\":\"https://cdn.example.com/a.png\"},\"tracks\":{",
    );
    assert!(matches!(
        decode_timeline(&invalid_alias),
        Err(TimelineDecodeError::InvalidTimeline(ref report))
            if report.diagnostics.iter().any(|item| item.code == "invalid_resource_key")
    ));
    let invalid_url = solid_with_curve().replace(
        "\"tracks\":{",
        "\"resources\":{\"hero\":\"https://\"},\"tracks\":{",
    );
    assert!(matches!(
        decode_timeline(&invalid_url),
        Err(TimelineDecodeError::InvalidTimeline(ref report))
            if report.diagnostics.iter().any(|item| item.code == "invalid_resource_url")
    ));

    for content in [
        json!({}),
        json!({ "text": "hello", "runs": [{ "text": "hello" }] }),
    ] {
        assert!(matches!(
            decode_timeline(&caption(content, json!({}))),
            Err(TimelineDecodeError::InvalidTimeline(ref report))
                if report.diagnostics.iter().any(|item| item.code == "caption_content_exactly_one")
        ));
    }
    assert!(matches!(
        decode_timeline(&caption(
            json!({ "text": "hello" }),
            json!({ "enter": "fade", "presentation": { "opacity": 0.5 } }),
        )),
        Err(TimelineDecodeError::InvalidTimeline(ref report))
            if report.diagnostics.iter().any(|item| item.code == "preset_presentation_conflict")
    ));
    for effects in [
        json!({ "enter": "breathe" }),
        json!({ "display": "fade" }),
        json!({ "exit": "shake" }),
        json!({ "enter": "enter.fade" }),
    ] {
        assert!(matches!(
            decode_timeline(&caption(json!({ "text": "hello" }), effects)),
            Err(TimelineDecodeError::InvalidShape)
        ));
    }
    assert!(matches!(
        decode_timeline(&caption(
            json!({ "runs": [{ "text": "hello", "fontWeight": 700 }] }),
            json!({}),
        )),
        Err(TimelineDecodeError::InvalidShape)
    ));

    let invalid_temperature = serde_json::to_string(&json!({
        "canvas": { "width": 1280, "height": 720, "fps": 30 },
        "tracks": {
            "adjustment": [{
                "clips": [{
                    "start": 0,
                    "duration": 1,
                    "kind": "color-grade",
                    "temperature": 1.1
                }]
            }]
        }
    }))
    .unwrap();
    assert!(matches!(
        decode_timeline(&invalid_temperature),
        Err(TimelineDecodeError::InvalidTimeline(ref report))
            if report.diagnostics.iter().any(|item| item.code == "temperature_out_of_range")
    ));
}

#[test]
fn karaoke_consumes_inline_timed_runs_not_words_sidecars() {
    let valid = caption(
        json!({
            "runs": [
                { "text": "kara", "start": 0, "end": 0.4 },
                { "text": "oke", "start": 0.4, "end": 1 }
            ]
        }),
        json!({ "behavior": { "type": "karaoke", "mode": "word" } }),
    );
    let timeline = decode_timeline(&valid).expect("inline timed runs are render-ready");
    let encoded: serde_json::Value =
        serde_json::from_slice(&timeline_bytes(&timeline).unwrap()).unwrap();
    let clip = &encoded["tracks"]["caption"][0]["clips"][0];
    assert_eq!(clip["runs"][0]["start"], json!(0));
    assert_eq!(clip["runs"][1]["end"], json!(1));
    assert!(clip["behavior"].get("words").is_none());

    let untimed = caption(
        json!({ "runs": [{ "text": "karaoke" }] }),
        json!({ "behavior": { "type": "karaoke", "mode": "word" } }),
    );
    assert!(matches!(
        decode_timeline(&untimed),
        Err(TimelineDecodeError::InvalidTimeline(ref report))
            if report.diagnostics.iter().any(|item| item.code == "karaoke_run_timing_required")
    ));

    let timed_without_karaoke = caption(
        json!({ "runs": [{ "text": "plain", "start": 0, "end": 1 }] }),
        json!({}),
    );
    assert!(matches!(
        decode_timeline(&timed_without_karaoke),
        Err(TimelineDecodeError::InvalidTimeline(ref report))
            if report.diagnostics.iter().any(|item| item.code == "run_timing_requires_karaoke")
    ));

    let unsupported_words = caption(
        json!({
            "runs": [{ "text": "karaoke", "start": 0, "end": 1 }]
        }),
        json!({
            "behavior": {
                "type": "karaoke",
                "mode": "word",
                "words": "words-json"
            }
        }),
    );
    assert!(matches!(
        decode_timeline(&unsupported_words),
        Err(TimelineDecodeError::InvalidShape)
    ));
}

#[test]
fn scroll_speed_is_non_zero_canvas_pixels_per_second() {
    let zero = caption(
        json!({ "text": "scroll" }),
        json!({ "behavior": { "type": "scroll", "axis": "horizontal", "speed": 0 } }),
    );
    assert!(matches!(
        decode_timeline(&zero),
        Err(TimelineDecodeError::InvalidTimeline(ref report))
            if report.diagnostics.iter().any(|item| item.code == "scroll_speed_zero")
    ));

    let non_zero = caption(
        json!({ "text": "scroll" }),
        json!({ "behavior": { "type": "scroll", "axis": "vertical", "speed": -80 } }),
    );
    decode_timeline(&non_zero).expect("signed canvas pixels per second are valid");
}

#[test]
fn from_wire_revalidates_and_normalizes_programmatic_inputs() {
    let timeline = decode_timeline(&solid_with_curve()).unwrap();
    let rebuilt = Timeline::from_wire(timeline.to_wire()).unwrap();
    assert_eq!(
        timeline_bytes(&timeline).unwrap(),
        timeline_bytes(&rebuilt).unwrap()
    );

    let mut invalid_fps = timeline.to_wire();
    invalid_fps.canvas.fps = TimelineFrameRateWire::Rational("60000/2002".to_owned());
    assert!(matches!(
        Timeline::from_wire(invalid_fps),
        Err(ref report) if report.diagnostics.iter().any(|item| item.code == "invalid_frame_rate")
    ));
}
