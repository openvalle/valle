use serde_json::{Value, json};
use valle_timeline::internal::{
    CanonicalDecodeError, CanonicalJsonIssue, CanonicalTimeline, CaptionAlign, CaptionItem,
    FrameRate, LayerMask, Param, RationalRate, RationalTime, VisualItem, VisualSource,
    canonical_bytes, decode_canonical, encode_canonical,
};

fn empty_document() -> Value {
    json!({
        "document": {
            "canvas": {
                "width": 1920,
                "height": 1080,
                "fps": "30/1",
                "sampleRate": 48000,
                "channelLayout": "stereo",
                "colorSpace": "srgb",
                "duration": "10/1"
            },
            "background": { "color": "#000000ff" },
            "visual": { "tracks": [] },
            "audio": { "tracks": [] },
            "adjustments": [],
            "captions": { "tracks": [] },
            "camera": null,
            "metadata": {}
        }
    })
}

fn constant(value: Value) -> Value {
    json!({ "type": "constant", "value": value })
}

fn solid_clip(id: &str, duration: &str) -> Value {
    json!({
        "type": "clip",
        "id": id,
        "duration": duration,
        "layer": {
            "transform": {
                "position": constant(json!([0.5, 0.5])),
                "scale": constant(json!([1.0, 1.0])),
                "rotation": constant(json!(0.0)),
                "anchor": [0.5, 0.5]
            },
            "opacity": constant(json!(1.0)),
            "mask": null,
            "filters": [],
            "blend": "normal"
        },
        "source": { "type": "solid", "color": "#ffffffff" }
    })
}

fn decode_value(
    value: &Value,
) -> Result<valle_timeline::internal::CanonicalTimeline, CanonicalDecodeError> {
    decode_canonical(&serde_json::to_string(value).unwrap())
}

fn caption(id: &str, region: [f64; 4], align: &str) -> Value {
    json!({
        "type": "clip",
        "id": id,
        "duration": "1/1",
        "runs": [{
            "id": format!("run:{id}"),
            "text": "hello",
            "timing": null,
            "style": null
        }],
        "style": {
            "font": "font:main",
            "fontSize": 32.0,
            "color": "#ffffffff",
            "shadow": null
        },
        "layout": {"region": region, "align": align},
        "presentation": {
            "opacity": constant(json!(1.0)),
            "translation": constant(json!([0.0, 0.0])),
            "scale": constant(json!(1.0)),
            "rotation": constant(json!(0.0)),
            "clipInset": constant(json!([0.0, 0.0, 0.0, 0.0])),
            "blurSigma": constant(json!(0.0))
        },
        "behavior": null
    })
}

#[test]
fn canonical_round_trip_is_byte_idempotent() {
    let mut value = empty_document();
    value["document"]["metadata"] = json!({"z": -0.0, "a": {"b": 2, "a": 1}});
    let timeline = decode_value(&value).unwrap();
    let first = canonical_bytes(&timeline).unwrap();
    let second =
        canonical_bytes(&decode_canonical(&String::from_utf8(first.clone()).unwrap()).unwrap())
            .unwrap();
    assert_eq!(first, second);
    assert_eq!(encode_canonical(&timeline).unwrap().as_bytes(), first);
    assert!(
        String::from_utf8(first)
            .unwrap()
            .contains(r#""metadata":{"a":{"a":1,"b":2},"z":0}"#)
    );
}

#[test]
fn closed_schema_scalars_quantize_before_validation_and_canonicalization() {
    let mut value = empty_document();
    let large_grid_member = f64::from_bits(0x41f0_b16a_09c8_b75c);
    let mut clip = solid_clip("clip:quantized", "1/1");
    clip["layer"]["transform"]["position"] =
        constant(json!([0.9299999999999999, 0.13333333333333333]));
    clip["layer"]["transform"]["scale"] = constant(json!([1.0000004, 1.0]));
    clip["layer"]["transform"]["rotation"] = constant(json!(0.123456789));
    clip["layer"]["transform"]["anchor"] = json!([0.5000004, 0.4999996]);
    clip["layer"]["opacity"] = json!({
        "type": "curve",
        "id": "curve:quantized:opacity",
        "interpolation": "linear",
        "keyframes": [
            {
                "id": "keyframe:quantized:opacity:0",
                "time": "1/25",
                "value": 0.13333333333333333,
                "outEasing": {
                    "type": "cubic-bezier",
                    "x1": 0.123456789,
                    "y1": 0.234567891,
                    "x2": 0.765432109,
                    "y2": 0.876543219
                }
            },
            {
                "id": "keyframe:quantized:opacity:1",
                "time": "1/1",
                "value": 1.0000004,
                "outEasing": null
            }
        ],
        "extrapolation": "clamp"
    });
    clip["layer"]["mask"] = json!({
        "type": "rect",
        "rect": constant(json!([0.123456789, 0.234567891, 0.765432109, 0.876543219])),
        "feather": constant(json!(1.23456789)),
        "invert": false
    });
    clip["layer"]["filters"] = json!([{
        "id": "filter:open-precision",
        "type": "example.visual/glow@1",
        "parameters": {"precision": 0.123456789}
    }]);
    clip["source"] = json!({
        "type": "motion",
        "component": "motion:quantized",
        "sourceStart": "0/1",
        "sourceDuration": "1/1",
        "rate": "1/1",
        "endBehavior": "hold",
        "props": {
            "precision": constant(json!(0.123456789)),
            "animated": {
                "type": "curve",
                "id": "curve:motion:open-value",
                "interpolation": "linear",
                "keyframes": [
                    {
                        "id": "keyframe:motion:open-value:0",
                        "time": "0/1",
                        "value": {"precision": 0.123456789},
                        "outEasing": {
                            "type": "cubic-bezier",
                            "x1": 0.123456789,
                            "y1": 0.234567891,
                            "x2": 0.765432109,
                            "y2": 0.876543219
                        }
                    },
                    {
                        "id": "keyframe:motion:open-value:1",
                        "time": "1/1",
                        "value": {"precision": 0.987654321},
                        "outEasing": null
                    }
                ],
                "extrapolation": "clamp"
            }
        },
        "cues": {},
        "resources": {},
        "phases": {"enterDuration": null, "exitDuration": null}
    });
    value["document"]["visual"]["tracks"] = json!([{
        "id": "visual:quantized",
        "items": [
            clip,
            {
                "type": "transition",
                "id": "transition:open-precision",
                "duration": "1/4",
                "kernel": {
                    "type": "example.transition/iris@1",
                    "parameters": {"precision": 0.123456789}
                }
            },
            solid_clip("clip:quantized:tail", "1/1")
        ]
    }]);
    value["document"]["audio"]["tracks"] = json!([{
        "id": "audio:quantized",
        "items": [{
            "type": "clip",
            "id": "audio-clip:quantized",
            "duration": "1/1",
            "source": {
                "type": "media",
                "resource": "audio:source",
                "sourceStart": "0/1",
                "rate": "1/1",
                "endBehavior": "hold"
            },
            "gain": constant(json!(0.26666666666666666)),
            "pan": constant(json!(-0.13333333333333333)),
            "effects": [{
                "id": "audio-effect:open-precision",
                "type": "example.audio/limiter@1",
                "parameters": {"precision": 0.123456789}
            }]
        }]
    }]);
    value["document"]["adjustments"] = json!([
        {
            "id": "global:quantized",
            "start": "0/1",
            "duration": "1/1",
            "effect": {"type": "color-grade", "temperature": 0.123456789}
        },
        {
            "id": "global:open-precision",
            "start": "0/1",
            "duration": "1/1",
            "effect": {
                "type": "example.global/tone-map@1",
                "parameters": {"precision": 0.123456789}
            }
        }
    ]);
    let mut caption = caption(
        "caption:quantized",
        [0.07, 0.6, 0.9299999999999999, 0.4],
        "top-left",
    );
    caption["runs"][0]["style"] = json!({
        "fontSize": 80.123456789,
        "color": null
    });
    caption["style"]["fontSize"] = json!(32.123456789);
    caption["style"]["shadow"] = json!({
        "color": "#000000ff",
        "offset": [1.123456789, 2.987654321],
        "blurSigma": 5.123456789
    });
    caption["presentation"]["opacity"] = constant(json!(0.13333333333333333));
    caption["presentation"]["translation"] = constant(json!([0.123456789, -0.987654321]));
    caption["presentation"]["scale"] = constant(json!(1.23456789));
    caption["presentation"]["rotation"] = constant(json!(-0.123456789));
    caption["presentation"]["clipInset"] =
        constant(json!([0.012345678, 0.023456789, 0.034567891, 0.045678912]));
    caption["presentation"]["blurSigma"] = constant(json!(5.123456789));
    caption["behavior"] = json!({
        "type": "scroll",
        "axis": "horizontal",
        "speed": 1.23456789
    });
    value["document"]["captions"]["tracks"] = json!([{
        "id": "caption:track",
        "items": [caption]
    }]);
    value["document"]["camera"] = json!({
        "centerX": constant(json!(0.9299999999999999)),
        "centerY": constant(json!(0.13333333333333333)),
        "zoom": constant(json!(1.0000004)),
        "rotation": constant(json!(large_grid_member))
    });
    value["document"]["metadata"] = json!({"precision": 0.123456789});

    let timeline = decode_value(&value).unwrap();
    let encoded: Value = serde_json::from_slice(&canonical_bytes(&timeline).unwrap()).unwrap();
    let document = &encoded["document"];
    let encoded_clip = &document["visual"]["tracks"][0]["items"][0];

    assert_eq!(
        encoded_clip["layer"]["transform"]["position"]["value"],
        json!([0.93, 0.133333])
    );
    assert_eq!(
        encoded_clip["layer"]["transform"]["scale"]["value"],
        json!([1, 1])
    );
    assert_eq!(
        encoded_clip["layer"]["transform"]["rotation"]["value"],
        json!(0.123457)
    );
    assert_eq!(
        encoded_clip["layer"]["transform"]["anchor"],
        json!([0.5, 0.5])
    );
    assert_eq!(
        encoded_clip["layer"]["opacity"]["keyframes"][0]["value"],
        json!(0.133333)
    );
    assert_eq!(
        encoded_clip["layer"]["opacity"]["keyframes"][0]["outEasing"],
        json!({
            "type": "cubic-bezier",
            "x1": 0.123457,
            "y1": 0.234568,
            "x2": 0.765432,
            "y2": 0.876543
        })
    );
    assert_eq!(
        encoded_clip["layer"]["opacity"]["keyframes"][1]["value"],
        json!(1),
        "quantization must happen before the opacity bound is validated"
    );
    assert_eq!(
        encoded_clip["layer"]["mask"]["rect"]["value"],
        json!([0.123457, 0.234568, 0.765432, 0.876543])
    );
    assert_eq!(
        encoded_clip["layer"]["mask"]["feather"]["value"],
        json!(1.234568)
    );
    assert_eq!(
        document["audio"]["tracks"][0]["items"][0]["gain"]["value"],
        json!(0.266667)
    );
    assert_eq!(
        document["audio"]["tracks"][0]["items"][0]["pan"]["value"],
        json!(-0.133333)
    );
    assert_eq!(
        document["adjustments"][0]["effect"]["temperature"],
        json!(0.123457)
    );
    let encoded_caption = &document["captions"]["tracks"][0]["items"][0];
    assert_eq!(
        encoded_caption["runs"][0]["style"]["fontSize"],
        json!(80.123457)
    );
    assert_eq!(encoded_caption["style"]["fontSize"], json!(32.123457));
    assert_eq!(
        encoded_caption["style"]["shadow"]["offset"],
        json!([1.123457, 2.987654])
    );
    assert_eq!(
        encoded_caption["style"]["shadow"]["blurSigma"],
        json!(5.123457)
    );
    assert_eq!(
        encoded_caption["layout"]["region"],
        json!([0.07, 0.6, 0.93, 0.4])
    );
    assert_eq!(
        encoded_caption["presentation"]["opacity"]["value"],
        json!(0.133333)
    );
    assert_eq!(
        encoded_caption["presentation"]["translation"]["value"],
        json!([0.123457, -0.987654])
    );
    assert_eq!(
        encoded_caption["presentation"]["scale"]["value"],
        json!(1.234568)
    );
    assert_eq!(
        encoded_caption["presentation"]["rotation"]["value"],
        json!(-0.123457)
    );
    assert_eq!(
        encoded_caption["presentation"]["clipInset"]["value"],
        json!([0.012346, 0.023457, 0.034568, 0.045679])
    );
    assert_eq!(
        encoded_caption["presentation"]["blurSigma"]["value"],
        json!(5.123457)
    );
    assert_eq!(encoded_caption["behavior"]["speed"], json!(1.234568));
    assert_eq!(document["camera"]["centerY"]["value"], json!(0.133333));
    assert_eq!(document["camera"]["zoom"]["value"], json!(1));
    assert_eq!(
        document["camera"]["rotation"]["value"],
        json!(4_480_999_580.544_765)
    );

    // Extension-owned open namespaces retain their exact binary64/JCS semantics. Motion props are
    // Timeline keyframe values and therefore use the same q6 numeric-leaf rule as other params.
    assert_eq!(
        encoded_clip["layer"]["filters"][0]["parameters"]["precision"],
        json!(0.123456789)
    );
    assert_eq!(
        encoded_clip["source"]["props"]["precision"]["value"],
        json!(0.123457)
    );
    assert_eq!(
        encoded_clip["source"]["props"]["animated"]["keyframes"][0]["value"]["precision"],
        json!(0.123457)
    );
    assert_eq!(
        encoded_clip["source"]["props"]["animated"]["keyframes"][0]["outEasing"],
        json!({
            "type": "cubic-bezier",
            "x1": 0.123457,
            "y1": 0.234568,
            "x2": 0.765432,
            "y2": 0.876543
        })
    );
    assert_eq!(
        document["visual"]["tracks"][0]["items"][1]["kernel"]["parameters"]["precision"],
        json!(0.123456789)
    );
    assert_eq!(
        document["audio"]["tracks"][0]["items"][0]["effects"][0]["parameters"]["precision"],
        json!(0.123456789)
    );
    assert_eq!(
        document["adjustments"][1]["effect"]["parameters"]["precision"],
        json!(0.123456789)
    );
    assert_eq!(document["metadata"]["precision"], json!(0.123456789));
    assert_eq!(
        encoded_clip["layer"]["opacity"]["keyframes"][0]["time"],
        json!("1/25")
    );

    let mut equivalent = value;
    equivalent["document"]["visual"]["tracks"][0]["items"][0]["layer"]["transform"]["position"]["value"]
        [0] = json!(0.9300004);
    let equivalent = decode_value(&equivalent).unwrap();
    assert_eq!(canonical_bytes(&timeline), canonical_bytes(&equivalent));

    let encoded_bytes = canonical_bytes(&timeline).unwrap();
    let encoded_text = std::str::from_utf8(&encoded_bytes).unwrap();
    let decoded_again = decode_canonical(encoded_text).unwrap();
    assert_eq!(canonical_bytes(&timeline), canonical_bytes(&decoded_again));
}

#[test]
fn scalar_quantization_cannot_create_an_invalid_canonical_domain_value() {
    let mut value = empty_document();
    value["document"]["captions"]["tracks"] = json!([{
        "id": "caption:track",
        "items": [caption(
            "caption:tiny",
            [0.0, 0.0, 0.0000004, 1.0],
            "center"
        )]
    }]);

    let CanonicalDecodeError::InvalidDocument(report) = decode_value(&value).unwrap_err() else {
        panic!("expected quantized zero-width region to be rejected");
    };
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "caption_region_out_of_bounds")
    );
}

#[test]
fn caption_timing_is_karaoke_only_and_scroll_speed_must_move() {
    let mut timed_plain = empty_document();
    let mut plain = caption("caption:timed-plain", [0.0, 0.0, 1.0, 1.0], "center");
    plain["runs"][0]["timing"] = json!({"start": "0/1", "end": "1/1"});
    timed_plain["document"]["captions"]["tracks"] = json!([{
        "id": "caption:track",
        "items": [plain]
    }]);
    let CanonicalDecodeError::InvalidDocument(report) = decode_value(&timed_plain).unwrap_err()
    else {
        panic!("expected non-karaoke timing to be rejected")
    };
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "run_timing_requires_karaoke")
    );

    let mut zero_scroll = empty_document();
    let mut scroll = caption("caption:scroll", [0.0, 0.0, 1.0, 1.0], "center");
    scroll["behavior"] = json!({"type": "scroll", "axis": "horizontal", "speed": 0});
    zero_scroll["document"]["captions"]["tracks"] = json!([{
        "id": "caption:track",
        "items": [scroll]
    }]);
    let CanonicalDecodeError::InvalidDocument(report) = decode_value(&zero_scroll).unwrap_err()
    else {
        panic!("expected zero scroll speed to be rejected")
    };
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "scroll_speed_zero")
    );
}

#[test]
fn canonical_domain_uses_dimensional_time_and_round_trips_through_wire() {
    let mut value = empty_document();
    let mut clip = solid_clip("clip:video", "1/1");
    clip["source"] = json!({
        "type": "video",
        "resource": "video:main",
        "sourceStart": "1/2",
        "rate": "3/2",
        "endBehavior": "hold",
        "sampling": {"fit": "contain"}
    });
    value["document"]["visual"]["tracks"] = json!([{"id": "visual:main", "items": [clip]}]);

    let timeline = decode_value(&value).unwrap();
    let document = timeline.document();
    let _: FrameRate = document.canvas.fps;
    let _: RationalTime = document.canvas.duration;
    let VisualItem::Clip(clip) = &document.visual.tracks[0].items[0] else {
        panic!("expected a visual clip");
    };
    let _: RationalTime = clip.duration;
    let VisualSource::Video(source) = &clip.source else {
        panic!("expected a video source");
    };
    let _: RationalTime = source.source_start;
    let _: RationalRate = source.rate;

    let wire = timeline.to_wire();
    assert_eq!(wire.document.canvas.fps.to_string(), "30/1");
    assert_eq!(wire.document.canvas.duration.to_string(), "10/1");
    assert_eq!(
        canonical_bytes(&timeline).unwrap(),
        canonical_bytes(&CanonicalTimeline::try_from_wire(wire).unwrap()).unwrap()
    );
}

#[test]
fn mask_rect_refinement_rejects_non_positive_extents() {
    let masks = [
        json!({
            "type": "rect",
            "rect": constant(json!([0.0, 0.0, 0.0, 1.0])),
            "feather": constant(json!(0.0)),
            "invert": false
        }),
        json!({
            "type": "ellipse",
            "rect": {
                "type": "curve",
                "id": "curve:mask",
                "interpolation": "linear",
                "keyframes": [{
                    "id": "keyframe:mask",
                    "time": "0/1",
                    "value": [0.0, 0.0, 1.0, -1.0],
                    "outEasing": null
                }],
                "extrapolation": "clamp"
            },
            "feather": constant(json!(0.0)),
            "invert": false
        }),
    ];

    for mask in masks {
        let mut value = empty_document();
        let mut clip = solid_clip("clip:masked", "1/1");
        clip["layer"]["mask"] = mask;
        value["document"]["visual"]["tracks"] = json!([{"id": "visual:main", "items": [clip]}]);
        let CanonicalDecodeError::InvalidDocument(report) = decode_value(&value).unwrap_err()
        else {
            panic!("expected local invariant rejection");
        };
        assert!(
            report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "mask_rect_non_positive_extent")
        );
    }

    let mut valid = empty_document();
    let mut clip = solid_clip("clip:masked", "1/1");
    clip["layer"]["mask"] = json!({
        "type": "rect",
        "rect": constant(json!([0.0, 0.0, 2.0, 3.0])),
        "feather": constant(json!(0.0)),
        "invert": false
    });
    valid["document"]["visual"]["tracks"] = json!([{"id": "visual:main", "items": [clip]}]);
    let timeline = decode_value(&valid).unwrap();
    let VisualItem::Clip(clip) = &timeline.document().visual.tracks[0].items[0] else {
        panic!("expected a visual clip");
    };
    let Some(LayerMask::Rect { rect, .. }) = &clip.layer.mask else {
        panic!("expected a rect mask");
    };
    let Param::Constant(rect) = rect else {
        panic!("expected a constant mask rect");
    };
    assert_eq!(rect.value.width(), 2.0);
    assert_eq!(rect.value.height(), 3.0);
}

#[test]
fn linear_scale_curves_cannot_cross_a_singular_transform() {
    let scale_curve = |interpolation: &str| {
        json!({
            "type": "curve",
            "id": format!("curve:scale:{interpolation}"),
            "interpolation": interpolation,
            "keyframes": [
                {
                    "id": format!("keyframe:scale:{interpolation}:from"),
                    "time": "0/1",
                    "value": [1.0, 1.0],
                    "outEasing": "linear"
                },
                {
                    "id": format!("keyframe:scale:{interpolation}:to"),
                    "time": "1/1",
                    "value": [-1.0, 1.0],
                    "outEasing": null
                }
            ],
            "extrapolation": "clamp"
        })
    };

    let mut invalid = empty_document();
    let mut clip = solid_clip("clip:scaled", "2/1");
    clip["layer"]["transform"]["scale"] = scale_curve("linear");
    invalid["document"]["visual"]["tracks"] = json!([{"id": "visual:main", "items": [clip]}]);
    let CanonicalDecodeError::InvalidDocument(report) = decode_value(&invalid).unwrap_err() else {
        panic!("expected local invariant rejection");
    };
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "scale_curve_crosses_zero")
    );

    let mut valid_step = empty_document();
    let mut clip = solid_clip("clip:scaled", "2/1");
    clip["layer"]["transform"]["scale"] = scale_curve("step");
    valid_step["document"]["visual"]["tracks"] = json!([{"id": "visual:main", "items": [clip]}]);
    decode_value(&valid_step)
        .expect("a step scale changes sign without interpolating through zero");
}

#[test]
fn camera_center_uses_normalized_composition_units_without_clamping() {
    let mut value = empty_document();
    value["document"]["camera"] = json!({
        "centerX": constant(json!(1.01)),
        "centerY": constant(json!(-0.25)),
        "zoom": constant(json!(1.0)),
        "rotation": constant(json!(0.0))
    });
    decode_value(&value).expect("camera centers may intentionally sit outside the canvas");
}

#[test]
fn caption_region_refinement_requires_normalized_bounds() {
    for region in [
        [-0.1, 0.0, 0.5, 0.5],
        [0.0, -0.1, 0.5, 0.5],
        [0.0, 0.0, 0.0, 0.5],
        [0.0, 0.0, 0.5, 0.0],
        [0.6, 0.0, 0.5, 0.5],
        [0.0, 0.6, 0.5, 0.5],
    ] {
        let mut value = empty_document();
        value["document"]["captions"]["tracks"] = json!([{
            "id": "caption:track",
            "items": [caption("caption:item", region, "center")]
        }]);
        let CanonicalDecodeError::InvalidDocument(report) = decode_value(&value).unwrap_err()
        else {
            panic!("expected local invariant rejection");
        };
        assert!(
            report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "caption_region_out_of_bounds")
        );
    }

    let mut valid = empty_document();
    valid["document"]["captions"]["tracks"] = json!([{
        "id": "caption:track",
        "items": [caption("caption:item", [0.1, 0.2, 0.8, 0.7], "bottom-right")]
    }]);
    let timeline = decode_value(&valid).unwrap();
    let CaptionItem::Clip(caption) = &timeline.document().captions.tracks[0].items[0] else {
        panic!("expected a caption");
    };
    assert_eq!(caption.layout.region.as_array(), &[0.1, 0.2, 0.8, 0.7]);
    assert_eq!(caption.layout.align.anchor(), [1.0, 1.0]);

    for align in [
        CaptionAlign::TopLeft,
        CaptionAlign::TopCenter,
        CaptionAlign::TopRight,
        CaptionAlign::CenterLeft,
        CaptionAlign::Center,
        CaptionAlign::CenterRight,
        CaptionAlign::BottomLeft,
        CaptionAlign::BottomCenter,
        CaptionAlign::BottomRight,
    ] {
        assert!(
            align
                .anchor()
                .into_iter()
                .all(|coordinate| (0.0..=1.0).contains(&coordinate))
        );
    }
}

#[test]
fn caption_rich_style_shadow_and_presentation_are_closed() {
    let rich_caption = || {
        let mut item = caption("caption:rich", [0.1, 0.2, 0.8, 0.4], "center");
        item["runs"][0]["style"] = json!({
            "fontSize": 80.0,
            "color": "#fff2a0ff"
        });
        item["style"]["shadow"] = json!({
            "color": "#000000e6",
            "offset": [2.0, 2.0],
            "blurSigma": 5.0
        });
        item["presentation"]["opacity"] = json!({
            "type": "curve",
            "id": "curve:caption:opacity",
            "interpolation": "linear",
            "keyframes": [
                {
                    "id": "keyframe:caption:opacity:start",
                    "time": "0/1",
                    "value": 0.0,
                    "outEasing": "ease-out"
                },
                {
                    "id": "keyframe:caption:opacity:end",
                    "time": "1/2",
                    "value": 1.0,
                    "outEasing": null
                }
            ],
            "extrapolation": "clamp"
        });
        item
    };
    let document_with = |item: Value| {
        let mut value = empty_document();
        value["document"]["captions"]["tracks"] = json!([{
            "id": "caption:track",
            "items": [item]
        }]);
        value
    };
    let diagnostic_codes = |item: Value| {
        let CanonicalDecodeError::InvalidDocument(report) =
            decode_value(&document_with(item)).unwrap_err()
        else {
            panic!("expected local invariant rejection");
        };
        report
            .diagnostics
            .into_iter()
            .map(|diagnostic| diagnostic.code)
            .collect::<std::collections::BTreeSet<_>>()
    };

    let valid = decode_value(&document_with(rich_caption())).unwrap();
    let CaptionItem::Clip(caption) = &valid.document().captions.tracks[0].items[0] else {
        panic!("expected a caption");
    };
    let run_style = caption.runs[0].style.as_ref().unwrap();
    assert_eq!(run_style.font_size, Some(80.0));
    assert_eq!(run_style.color.as_deref(), Some("#fff2a0ff"));
    assert_eq!(caption.style.shadow.as_ref().unwrap().blur_sigma, 5.0);
    assert!(matches!(caption.presentation.opacity, Param::Curve(_)));
    assert_eq!(
        canonical_bytes(&valid).unwrap(),
        canonical_bytes(&CanonicalTimeline::try_from_wire(valid.to_wire()).unwrap()).unwrap()
    );

    let mut empty_override = rich_caption();
    empty_override["runs"][0]["style"] = json!({"fontSize": null, "color": null});
    assert!(diagnostic_codes(empty_override).contains("caption_run_style_empty"));

    let mut invalid_run_size = rich_caption();
    invalid_run_size["runs"][0]["style"]["fontSize"] = json!(0.0);
    assert!(diagnostic_codes(invalid_run_size).contains("font_size_non_positive"));

    let mut huge_base_size = rich_caption();
    huge_base_size["style"]["fontSize"] = json!(1_000_001.0);
    assert!(diagnostic_codes(huge_base_size).contains("font_size_out_of_range"));

    let mut huge_run_size = rich_caption();
    huge_run_size["runs"][0]["style"]["fontSize"] = json!(1_000_001.0);
    assert!(diagnostic_codes(huge_run_size).contains("font_size_out_of_range"));

    let mut invalid_shadow = rich_caption();
    invalid_shadow["style"]["shadow"]["blurSigma"] = json!(-1.0);
    assert!(diagnostic_codes(invalid_shadow).contains("caption_shadow_blur_sigma_out_of_range"));

    let mut boundary_shadow = rich_caption();
    boundary_shadow["style"]["shadow"]["blurSigma"] = json!(1_000_000.0);
    assert!(decode_value(&document_with(boundary_shadow)).is_ok());

    let mut huge_shadow = rich_caption();
    huge_shadow["style"]["shadow"]["blurSigma"] = json!(1_000_001.0);
    assert!(diagnostic_codes(huge_shadow).contains("caption_shadow_blur_sigma_out_of_range"));

    let mut invalid_opacity = rich_caption();
    invalid_opacity["presentation"]["opacity"] = constant(json!(1.1));
    assert!(diagnostic_codes(invalid_opacity).contains("opacity_out_of_range"));

    let mut invalid_scale = rich_caption();
    invalid_scale["presentation"]["scale"] = constant(json!(-0.1));
    assert!(diagnostic_codes(invalid_scale).contains("caption_scale_negative"));

    let mut invalid_inset = rich_caption();
    invalid_inset["presentation"]["clipInset"] = constant(json!([0.0, 1.1, 0.0, 0.0]));
    assert!(diagnostic_codes(invalid_inset).contains("caption_clip_inset_out_of_range"));

    let mut invalid_blur = rich_caption();
    invalid_blur["presentation"]["blurSigma"] = constant(json!(-0.1));
    assert!(diagnostic_codes(invalid_blur).contains("caption_blur_sigma_out_of_range"));

    let mut boundary_blur = rich_caption();
    boundary_blur["presentation"]["blurSigma"] = constant(json!(1_000_000.0));
    assert!(decode_value(&document_with(boundary_blur)).is_ok());

    let mut huge_blur = rich_caption();
    huge_blur["presentation"]["blurSigma"] = constant(json!(1_000_001.0));
    assert!(diagnostic_codes(huge_blur).contains("caption_blur_sigma_out_of_range"));

    let mut late_curve = rich_caption();
    late_curve["presentation"]["opacity"]["keyframes"][1]["time"] = json!("2/1");
    assert!(diagnostic_codes(late_curve).contains("keyframe_time_out_of_owner_range"));
}

#[test]
fn canonical_bytes_cover_metadata_and_timeline_ids() {
    let base = decode_value(&empty_document()).unwrap();

    let mut metadata = empty_document();
    metadata["document"]["metadata"] = json!({"review": "approved"});
    let metadata = decode_value(&metadata).unwrap();
    assert_ne!(
        canonical_bytes(&base).unwrap(),
        canonical_bytes(&metadata).unwrap()
    );

    let mut with_track = empty_document();
    with_track["document"]["visual"]["tracks"] = json!([{"id":"visual:a","items":[]}]);
    let first = decode_value(&with_track).unwrap();
    with_track["document"]["visual"]["tracks"][0]["id"] = json!("visual:b");
    let renamed = decode_value(&with_track).unwrap();
    assert_ne!(
        canonical_bytes(&first).unwrap(),
        canonical_bytes(&renamed).unwrap()
    );
}

#[test]
fn strict_json_and_shape_errors_happen_before_local_invariants() {
    let duplicate = r#"{
      "document":{"metadata":{"x":1,"\u0078":2}}
    }"#;
    assert!(matches!(
        decode_canonical(duplicate),
        Err(CanonicalDecodeError::CanonicalJson {
            issue: CanonicalJsonIssue::DuplicateObjectKey,
            ..
        })
    ));

    let mut unknown = empty_document();
    unknown["document"]["version"] = json!(2);
    assert_eq!(
        decode_value(&unknown),
        Err(CanonicalDecodeError::InvalidShape)
    );

    let mut retired_contract = empty_document();
    retired_contract["contract"] = json!("valle.timeline/document@2.0");
    assert_eq!(
        decode_value(&retired_contract),
        Err(CanonicalDecodeError::InvalidShape)
    );
}

#[test]
fn global_entity_ids_are_unique_across_bands_and_kinds() {
    let mut value = empty_document();
    value["document"]["visual"]["tracks"] = json!([{
        "id": "shared-id",
        "items": [solid_clip("clip:a", "1/1")]
    }]);
    value["document"]["audio"]["tracks"] = json!([{"id":"shared-id","items":[]}]);

    let report = match decode_value(&value).unwrap_err() {
        CanonicalDecodeError::InvalidDocument(report) => report,
        other => panic!("unexpected error: {other:?}"),
    };
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "duplicate_entity_id")
    );
}

#[test]
fn sequence_structure_and_track_duration_are_local_invariants() {
    let mut value = empty_document();
    value["document"]["visual"]["tracks"] = json!([{
        "id": "v-main",
        "items": [
            {"type":"gap","id":"gap:a","duration":"1/1"},
            {"type":"gap","id":"gap:b","duration":"1/1"},
            {"type":"transition","id":"transition:orphan","duration":"1/2","kernel":{"type":"cross-fade"}},
            solid_clip("clip:too-long", "9/1")
        ]
    }]);

    let report = match decode_value(&value).unwrap_err() {
        CanonicalDecodeError::InvalidDocument(report) => report,
        other => panic!("unexpected error: {other:?}"),
    };
    let codes: std::collections::BTreeSet<_> = report
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect();
    assert!(codes.contains("adjacent_gap"));
    assert!(codes.contains("transition_adjacency"));
    assert!(codes.contains("track_exceeds_canvas"));
}

#[test]
fn motion_cues_and_phase_overrides_are_source_clock_invariants() {
    let mut value = empty_document();
    let mut clip = solid_clip("clip:motion", "4/1");
    clip["source"] = json!({
        "type": "motion",
        "component": "component:card",
        "sourceStart": "0/1",
        "sourceDuration": "4/1",
        "rate": "1/1",
        "endBehavior": "hold",
        "props": {},
        "cues": {
            "bad-range": {
                "type": "source-range",
                "start": "3/1",
                "end": "2/1",
                "enterDuration": "1/1",
                "exitDuration": "1/1"
            }
        },
        "resources": {},
        "phases": { "enterDuration": "3/1", "exitDuration": "2/1" }
    });
    value["document"]["visual"]["tracks"] = json!([{
        "id": "visual:main",
        "items": [clip]
    }]);

    let report = match decode_value(&value).unwrap_err() {
        CanonicalDecodeError::InvalidDocument(report) => report,
        other => panic!("unexpected error: {other:?}"),
    };
    let codes: std::collections::BTreeSet<_> = report
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect();
    assert!(codes.contains("motion_cue_range_invalid"));
    assert!(codes.contains("motion_phase_exceeds_source_duration"));
}

#[test]
fn curves_are_closed_during_construction() {
    let mut value = empty_document();
    let mut first = solid_clip("clip:a", "5/1");
    first["layer"]["opacity"] = json!({
        "type": "curve",
        "id": "curve:opacity",
        "interpolation": "linear",
        "keyframes": [
            {"id":"kf:late","time":"2/1","value":1.0,"outEasing":null},
            {"id":"kf:early","time":"1/1","value":0.0,"outEasing":"linear"}
        ],
        "extrapolation": "clamp"
    });
    value["document"]["visual"]["tracks"] = json!([{"id":"v:a","items":[first]}]);

    let report = match decode_value(&value).unwrap_err() {
        CanonicalDecodeError::InvalidDocument(report) => report,
        other => panic!("unexpected error: {other:?}"),
    };
    let codes: std::collections::BTreeSet<_> = report
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect();
    assert!(codes.contains("keyframe_time_not_strictly_increasing"));
    assert!(codes.contains("last_keyframe_out_easing_not_null"));
}

#[test]
fn canonical_construction_enforces_hard_timeline_budgets() {
    let mut value = empty_document();
    value["document"]["visual"]["tracks"] = Value::Array(
        (0..1_025)
            .map(|index| json!({"id": format!("track:{index}"), "items": []}))
            .collect(),
    );
    let report = match decode_value(&value).unwrap_err() {
        CanonicalDecodeError::InvalidDocument(report) => report,
        other => panic!("unexpected error: {other:?}"),
    };
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "track_budget_exceeded")
    );
}
