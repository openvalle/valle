//! Positive/negative JSON corpus for the canonical Timeline document wire shape.
//!
use serde_json::{Value, json};
use valle_timeline::internal::wire::*;

fn canonical_document() -> Value {
    json!({
        "document": {
            "canvas": {
                "width": 1920,
                "height": 1080,
                "fps": "30000/1001",
                "sampleRate": 48000,
                "channelLayout": "stereo",
                "colorSpace": "srgb",
                "duration": "1001/100"
            },
            "background": { "color": "#000000ff" },
            "visual": {
                "tracks": [{
                    "id": "v-main",
                    "items": [
                        {
                            "type": "clip",
                            "id": "shot-a",
                            "duration": "3003/1000",
                            "layer": layer_with_curve(),
                            "source": {
                                "type": "video",
                                "resource": "asset:hero-a",
                                "sourceStart": "0/1",
                                "rate": "1/1",
                                "endBehavior": "error",
                                "sampling": { "fit": "cover" }
                            }
                        },
                        {
                            "type": "transition",
                            "id": "x-main",
                            "duration": "1/2",
                            "kernel": { "type": "cross-fade" }
                        },
                        {
                            "type": "clip",
                            "id": "shot-b",
                            "duration": "7007/1000",
                            "layer": constant_layer(),
                            "source": {
                                "type": "image",
                                "resource": "asset:hero-b",
                                "sampling": { "fit": "contain" }
                            }
                        }
                    ]
                }]
            },
            "audio": {
                "tracks": [{
                    "id": "a-main",
                    "items": [
                        audio_clip("voice-a", "asset:voice-a"),
                        { "type": "crossfade", "id": "ax-main", "duration": "1/4" },
                        audio_clip("voice-b", "asset:voice-b")
                    ]
                }]
            },
            "adjustments": [{
                "id": "grade",
                "start": "0/1",
                "duration": "1001/100",
                "effect": { "type": "color-grade", "temperature": 0.1 }
            }],
            "captions": {
                "tracks": [{
                    "id": "s-main",
                    "items": [
                        { "type": "gap", "id": "gap-sub", "duration": "2/1" },
                        {
                            "type": "clip",
                            "id": "line-1",
                            "duration": "3/1",
                            "runs": [{
                                "id": "run-1",
                                "text": "JSON in, MP4 out",
                                "timing": null,
                                "style": null
                            }],
                            "style": {
                                "font": "font:inter",
                                "fontSize": 64.0,
                                "color": "#ffffffff",
                                "shadow": null
                            },
                            "layout": {
                                "region": [0.1, 0.76, 0.8, 0.18],
                                "align": "bottom-center"
                            },
                            "presentation": {
                                "opacity": constant(1.0),
                                "translation": constant_vec2(0.0, 0.0),
                                "scale": constant(1.0),
                                "rotation": constant(0.0),
                                "clipInset": {
                                    "type": "constant",
                                    "value": [0.0, 0.0, 0.0, 0.0]
                                },
                                "blurSigma": constant(0.0)
                            },
                            "behavior": null
                        }
                    ]
                }]
            },
            "camera": {
                "centerX": constant(0.5),
                "centerY": constant(0.5),
                "zoom": constant(1.0),
                "rotation": constant(0.0)
            },
            "metadata": {}
        }
    })
}

fn constant(value: f64) -> Value {
    json!({ "type": "constant", "value": value })
}

fn constant_vec2(x: f64, y: f64) -> Value {
    json!({ "type": "constant", "value": [x, y] })
}

fn constant_layer() -> Value {
    json!({
        "transform": {
            "position": constant_vec2(0.5, 0.5),
            "scale": constant_vec2(1.0, 1.0),
            "rotation": constant(0.0),
            "anchor": [0.5, 0.5]
        },
        "opacity": constant(1.0),
        "mask": null,
        "filters": [],
        "blend": "normal"
    })
}

fn layer_with_curve() -> Value {
    let mut layer = constant_layer();
    layer["opacity"] = json!({
        "type": "curve",
        "id": "curve:opacity",
        "interpolation": "linear",
        "keyframes": [
            {
                "id": "kf:opacity:0",
                "time": "0/1",
                "value": 0.0,
                "outEasing": "ease-out"
            },
            {
                "id": "kf:opacity:1",
                "time": "1/2",
                "value": 1.0,
                "outEasing": null
            }
        ],
        "extrapolation": "clamp"
    });
    layer
}

fn audio_clip(id: &str, resource: &str) -> Value {
    json!({
        "type": "clip",
        "id": id,
        "duration": "5/1",
        "source": {
            "type": "media",
            "resource": resource,
            "sourceStart": "0/1",
            "rate": "1/1",
            "endBehavior": "error"
        },
        "gain": constant(0.9),
        "pan": constant(0.0),
        "effects": []
    })
}

#[test]
fn complete_document_round_trips_without_materializing_defaults() {
    let value = canonical_document();
    let decoded: TimelineDocumentEnvelopeWire = serde_json::from_value(value.clone()).unwrap();

    let encoded = serde_json::to_value(decoded).unwrap();
    assert_eq!(encoded, value);
    assert_eq!(encoded["document"]["canvas"]["fps"], "30000/1001");
    assert_eq!(
        encoded["document"]["visual"]["tracks"][0]["items"][0]["layer"]["opacity"]["keyframes"][1]
            ["outEasing"],
        Value::Null
    );
}

#[test]
fn all_visual_source_variants_have_closed_shapes() {
    let sources = [
        json!({
            "type": "video", "resource": "asset:v", "sourceStart": "0/1", "rate": "1/1",
            "endBehavior": "error", "sampling": { "fit": "cover" }
        }),
        json!({
            "type": "image", "resource": "asset:i", "sampling": { "fit": "contain" }
        }),
        json!({
            "type": "lottie", "resource": "asset:l", "sourceStart": "0/1", "rate": "1/1",
            "endBehavior": "loop", "sampling": { "fit": "contain" }
        }),
        json!({
            "type": "motion", "component": "component:Title", "sourceStart": "0/1",
            "sourceDuration": "4/1", "rate": "1/1", "endBehavior": "hold",
            "props": { "title": { "type": "constant", "value": "VALLE" } },
            "cues": {
                "intro": {
                    "type": "source-range", "start": "0/1", "end": "1/1",
                    "enterDuration": "1/10", "exitDuration": "1/10"
                }
            },
            "resources": { "hero": "asset:i" },
            "phases": { "enterDuration": null, "exitDuration": "1/2" }
        }),
        json!({ "type": "solid", "color": "#ff0000ff" }),
    ];

    for source in sources {
        let decoded: VisualSourceWire = serde_json::from_value(source.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), source);
    }

    let retired_runtime_data = json!({
        "type": "motion", "component": "component:Title", "sourceStart": "0/1",
        "sourceDuration": "4/1", "rate": "1/1", "endBehavior": "hold",
        "props": {}, "data": {}, "cues": {}, "resources": {},
        "phases": { "enterDuration": null, "exitDuration": null }
    });
    assert!(serde_json::from_value::<VisualSourceWire>(retired_runtime_data).is_err());
}

#[test]
fn image_cannot_carry_media_timing_fields() {
    for field in [
        ("sourceStart", json!("0/1")),
        ("rate", json!("1/1")),
        ("endBehavior", json!("hold")),
        ("sourceDuration", json!("2/1")),
    ] {
        let mut image = json!({
            "type": "image",
            "resource": "asset:image",
            "sampling": { "fit": "cover" }
        });
        image[field.0] = field.1;
        assert!(
            serde_json::from_value::<VisualSourceWire>(image).is_err(),
            "ImageSource must reject {}",
            field.0
        );
    }
}

#[test]
fn audio_source_and_caption_behaviors_have_closed_variant_shapes() {
    let audio = json!({
        "type": "media",
        "resource": "asset:voice",
        "sourceStart": "0/1",
        "rate": "1/1",
        "endBehavior": "error"
    });
    let decoded: AudioSourceWire = serde_json::from_value(audio.clone()).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), audio);
    for field in ["resource", "sourceStart", "rate", "endBehavior"] {
        let mut missing = audio.clone();
        missing.as_object_mut().unwrap().remove(field);
        assert!(
            serde_json::from_value::<AudioSourceWire>(missing).is_err(),
            "AudioSource accepted missing {field}"
        );
    }
    for invalid in [
        json!({
            "type": "media", "resource": "asset:voice", "sourceStart": "0/1",
            "rate": "1/1", "endBehavior": "error", "channels": 2
        }),
        json!({"type": "tone", "frequency": 440}),
    ] {
        assert!(serde_json::from_value::<AudioSourceWire>(invalid).is_err());
    }

    let behaviors = [
        json!({"type": "scroll", "axis": "horizontal", "speed": 20.0}),
        json!({"type": "scroll", "axis": "vertical", "speed": 20.0}),
        json!({"type": "karaoke", "mode": "word"}),
        json!({"type": "karaoke", "mode": "line"}),
    ];
    for behavior in behaviors {
        let decoded: CaptionBehaviorWire = serde_json::from_value(behavior.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), behavior);
    }
    for invalid in [
        json!({"type": "scroll", "axis": "horizontal"}),
        json!({"type": "scroll", "axis": "horizontal", "speed": 20.0, "words": "words:x"}),
        json!({"type": "karaoke"}),
        json!({"type": "karaoke", "mode": "word", "words": "words:x"}),
        json!({"type": "karaoke", "mode": "word", "speed": 1.0}),
        json!({"type": "bounce", "speed": 1.0}),
    ] {
        assert!(serde_json::from_value::<CaptionBehaviorWire>(invalid).is_err());
    }
}

#[test]
fn canonical_nullable_and_default_fields_are_explicitly_required() {
    let mut missing_camera = canonical_document();
    missing_camera["document"]
        .as_object_mut()
        .unwrap()
        .remove("camera");
    assert!(serde_json::from_value::<TimelineDocumentEnvelopeWire>(missing_camera).is_err());

    let mut missing_metadata = canonical_document();
    missing_metadata["document"]
        .as_object_mut()
        .unwrap()
        .remove("metadata");
    assert!(serde_json::from_value::<TimelineDocumentEnvelopeWire>(missing_metadata).is_err());

    let mut missing_mask = constant_layer();
    missing_mask.as_object_mut().unwrap().remove("mask");
    assert!(serde_json::from_value::<VisualLayerWire>(missing_mask).is_err());

    let keyframe_without_nullable_easing = json!({
        "id": "kf", "time": "0/1", "value": 1.0
    });
    assert!(serde_json::from_value::<KeyframeWire<f64>>(keyframe_without_nullable_easing).is_err());

    let caption = canonical_document()["document"]["captions"]["tracks"][0]["items"][1].clone();
    for required in ["presentation"] {
        let mut missing = caption.clone();
        missing.as_object_mut().unwrap().remove(required);
        assert!(serde_json::from_value::<CaptionItemWire>(missing).is_err());
    }
    let mut missing_run_style = caption.clone();
    missing_run_style["runs"][0]
        .as_object_mut()
        .unwrap()
        .remove("style");
    assert!(serde_json::from_value::<CaptionItemWire>(missing_run_style).is_err());
    let mut missing_shadow = caption.clone();
    missing_shadow["style"]
        .as_object_mut()
        .unwrap()
        .remove("shadow");
    assert!(serde_json::from_value::<CaptionItemWire>(missing_shadow).is_err());
    for required in [
        "opacity",
        "translation",
        "scale",
        "rotation",
        "clipInset",
        "blurSigma",
    ] {
        let mut missing = caption.clone();
        missing["presentation"]
            .as_object_mut()
            .unwrap()
            .remove(required);
        assert!(
            serde_json::from_value::<CaptionItemWire>(missing).is_err(),
            "canonical caption presentation accepted missing {required}"
        );
    }

    let rich_run = json!({
        "id": "run:rich",
        "text": "rich",
        "timing": null,
        "style": {"fontSize": 80.0, "color": null}
    });
    let decoded: TextRunWire = serde_json::from_value(rich_run.clone()).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), rich_run);
    let mut missing_timing = rich_run.clone();
    missing_timing.as_object_mut().unwrap().remove("timing");
    assert!(
        serde_json::from_value::<TextRunWire>(missing_timing).is_err(),
        "canonical run accepted missing timing"
    );
    for required in ["fontSize", "color"] {
        let mut missing = rich_run.clone();
        missing["style"].as_object_mut().unwrap().remove(required);
        assert!(
            serde_json::from_value::<TextRunWire>(missing).is_err(),
            "canonical run style accepted missing {required}"
        );
    }

    let motion = json!({
        "type": "motion", "component": "component:card", "sourceStart": "0/1",
        "sourceDuration": "2/1", "rate": "1/1", "endBehavior": "hold",
        "props": {}, "cues": {}, "resources": {},
        "phases": { "enterDuration": null, "exitDuration": null }
    });
    for required in ["cues", "phases"] {
        let mut missing = motion.clone();
        missing.as_object_mut().unwrap().remove(required);
        assert!(
            serde_json::from_value::<VisualSourceWire>(missing).is_err(),
            "canonical Motion must require {required}"
        );
    }
    for required in ["enterDuration", "exitDuration"] {
        let mut missing = motion.clone();
        missing["phases"].as_object_mut().unwrap().remove(required);
        assert!(
            serde_json::from_value::<VisualSourceWire>(missing).is_err(),
            "canonical Motion phases must require {required}"
        );
    }
}

#[test]
fn closed_objects_and_unions_fail_closed() {
    let mut unknown_root = canonical_document();
    unknown_root["document"]["trackKind"] = json!("visual");
    assert!(serde_json::from_value::<TimelineDocumentEnvelopeWire>(unknown_root).is_err());

    let mut unknown_canvas = canonical_document();
    unknown_canvas["document"]["canvas"]["fpsFloat"] = json!(29.97);
    assert!(serde_json::from_value::<TimelineDocumentEnvelopeWire>(unknown_canvas).is_err());

    assert!(
        serde_json::from_value::<VisualItemWire>(json!({
            "type": "stack", "id": "not-supported", "items": []
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<VisualSourceWire>(json!({
            "type": "unknown", "resource": "asset:x"
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<ParamWire<f64>>(json!({
            "type": "constant", "value": 1.0, "default": true
        }))
        .is_err()
    );
}

#[test]
fn effect_domains_and_transition_kernels_are_distinct() {
    let grade = json!({ "type": "color-grade", "temperature": 0.1 });
    assert!(serde_json::from_value::<AdjustmentEffectWire>(grade.clone()).is_ok());
    assert!(serde_json::from_value::<VisualFilterWire>(grade.clone()).is_err());
    assert!(serde_json::from_value::<AudioEffectWire>(grade).is_err());

    let cross_fade = json!({ "type": "cross-fade" });
    assert!(serde_json::from_value::<TransitionKernelWire>(cross_fade.clone()).is_ok());
    assert!(serde_json::from_value::<AdjustmentEffectWire>(cross_fade.clone()).is_err());
    assert!(serde_json::from_value::<VisualFilterWire>(cross_fade).is_err());

    let visual_extension = json!({
        "type": "example.visual/glow@1",
        "id": "filter:glow",
        "parameters": { "radius": 12.0 }
    });
    let decoded_visual_extension =
        serde_json::from_value::<VisualFilterWire>(visual_extension.clone()).unwrap();
    assert_eq!(
        decoded_visual_extension.kind.as_str(),
        "example.visual/glow@1"
    );
    assert_eq!(
        decoded_visual_extension.kind.clone().into_string(),
        "example.visual/glow@1"
    );
    assert_eq!(
        serde_json::to_value(decoded_visual_extension).unwrap(),
        visual_extension
    );

    let audio_extension = json!({
        "type": "example.audio/limiter@1",
        "id": "audio-effect:limiter",
        "parameters": { "ceiling": 0.9 }
    });
    assert!(serde_json::from_value::<AudioEffectWire>(audio_extension).is_ok());

    let global_extension = json!({
        "type": "example.global/tone-map@1",
        "parameters": { "exposure": 0.25 }
    });
    assert!(serde_json::from_value::<AdjustmentEffectWire>(global_extension).is_ok());

    let transition_extension = json!({
        "type": "example.transition/iris@1",
        "parameters": { "blades": 8 }
    });
    assert!(serde_json::from_value::<TransitionKernelWire>(transition_extension).is_ok());

    for unnamespaced in [
        json!({ "type": "glow", "id": "f", "parameters": {} }),
        json!({ "type": "extension", "id": "f", "parameters": {} }),
    ] {
        assert!(serde_json::from_value::<VisualFilterWire>(unnamespaced).is_err());
    }

    assert!(
        serde_json::from_value::<TransitionKernelWire>(json!({
            "type": "cross-fade", "parameters": {}
        }))
        .is_err()
    );
}

#[test]
fn document_envelope_rejects_embedded_contract_authority() {
    for retired in [
        "valle.timeline/document@2.0",
        "valle.timeline/draft@1.0",
        "valle.timeline/document@2.1",
        "valle.timeline/edit@1.0",
    ] {
        let mut value = canonical_document();
        value["contract"] = json!(retired);
        assert!(
            serde_json::from_value::<TimelineDocumentEnvelopeWire>(value).is_err(),
            "inner contract authority `{retired}` must be rejected"
        );
    }
}
