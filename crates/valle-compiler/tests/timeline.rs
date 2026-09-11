use serde_json::json;
use valle_compiler::{CompileTimelineError, compile_timeline};
use valle_timeline::internal::wire::document;
use valle_timeline::{Timeline, TimelineDecodeError, decode_timeline};

fn decode(value: serde_json::Value) -> Timeline {
    decode_timeline(&value.to_string()).expect("valid Timeline fixture")
}

fn validation_codes(value: serde_json::Value) -> Vec<String> {
    match decode_timeline(&value.to_string()) {
        Err(TimelineDecodeError::InvalidTimeline(report)) => report
            .diagnostics
            .into_iter()
            .map(|diagnostic| diagnostic.code)
            .collect(),
        other => panic!("expected Timeline validation error, got {other:?}"),
    }
}

#[test]
fn sparse_author_timeline_materializes_internal_defaults_gaps_and_resource_ids() {
    let author = decode(json!({
        "canvas": {"width": 1080, "height": 1920, "fps": 25},
        "resources": {
            "video": "https://example.test/video.mp4",
            "voice": "https://example.test/voice.wav",
            "font": "https://example.test/font.ttf"
        },
        "tracks": {
            "visual": [{
                "clips": [{
                    "start": 1,
                    "duration": 2,
                    "kind": "video",
                    "src": "video",
                    "position": {"keyframes": [[0, [0.5, 0.4]], [1, [0.5, 0.5]]]}
                }]
            }],
            "audio": [{
                "clips": [{"start": 0.5, "duration": 4, "src": "voice"}]
            }],
            "caption": [{
                "style": {"font": "font", "fontSize": 72, "color": "#ffffffff"},
                "layout": {"region": [0.1, 0.7, 0.8, 0.2], "align": "bottom-center"},
                "clips": [{
                    "start": 1.25,
                    "duration": 1.5,
                    "text": "Timeline",
                    "layout": {"align": "center"}
                }]
            }]
        }
    }));

    let timeline = compile_timeline(author).expect("compile sparse Timeline");
    let wire = timeline.to_wire().document;
    assert_eq!(wire.canvas.sample_rate, 48_000);
    assert_eq!(
        wire.canvas.channel_layout,
        document::ChannelLayoutWire::Stereo
    );
    assert_eq!(wire.canvas.color_space, document::ColorSpaceWire::Srgb);
    assert_eq!(wire.canvas.duration.to_string(), "9/2");
    assert_eq!(wire.background.color, "#000000ff");

    let visual = &wire.visual.tracks[0];
    assert_eq!(visual.id, "timeline:visual-track:0");
    assert!(
        matches!(&visual.items[0], document::VisualItemWire::Gap(gap) if gap.duration.to_string() == "1/1")
    );
    let document::VisualItemWire::Clip(clip) = &visual.items[1] else {
        panic!("expected visual clip after generated gap")
    };
    assert_eq!(clip.id, "timeline:visual-track:0:clip:0");
    let document::VisualSourceWire::Video(source) = &clip.source else {
        panic!("expected video source")
    };
    assert_eq!(source.resource, "resource:video");
    assert_eq!(source.rate.to_string(), "1/1");
    let document::ParamWire::Curve(position) = &clip.layer.transform.position else {
        panic!("compact author curve must become an internal curve")
    };
    assert_eq!(position.id, "timeline:visual-track:0:clip:0:curve:position");
    assert_eq!(
        position.keyframes[0].id,
        "timeline:visual-track:0:clip:0:curve:position:keyframe:0"
    );

    let audio = &wire.audio.tracks[0];
    assert_eq!(audio.id, "timeline:audio-track:0");
    assert!(
        matches!(&audio.items[0], document::AudioItemWire::Gap(gap) if gap.duration.to_string() == "1/2")
    );
    let document::AudioItemWire::Clip(clip) = &audio.items[1] else {
        panic!("expected audio clip after generated gap")
    };
    let document::AudioSourceWire::Media(source) = &clip.source;
    assert_eq!(source.resource, "resource:voice");

    let captions = &wire.captions.tracks[0];
    assert_eq!(captions.id, "timeline:caption-track:0");
    assert!(
        matches!(&captions.items[0], document::CaptionItemWire::Gap(gap) if gap.duration.to_string() == "5/4")
    );
    let document::CaptionItemWire::Clip(caption) = &captions.items[1] else {
        panic!("expected caption after generated gap")
    };
    assert_eq!(caption.style.font, "resource:font");
    assert_eq!(caption.style.font_size, 72.0);
    assert_eq!(caption.layout.region, [0.1, 0.7, 0.8, 0.2]);
    assert_eq!(caption.layout.align, document::CaptionAlignWire::Center);
    assert_eq!(caption.runs[0].text, "Timeline");
}

#[test]
fn adjustment_track_lowers_to_timed_internal_adjustments() {
    let author = decode(json!({
        "canvas": {"width": 1920, "height": 1080, "fps": 30},
        "tracks": {
            "adjustment": [{
                "clips": [{
                    "start": 0.25,
                    "duration": 1.5,
                    "kind": "color-grade",
                    "temperature": -0.25
                }]
            }]
        }
    }));

    let timeline = compile_timeline(author).expect("compile typed adjustment track");
    let effect = &timeline.to_wire().document.adjustments[0];
    assert_eq!(effect.id, "timeline:adjustment-track:0:clip:0");
    assert_eq!(effect.start.to_string(), "1/4");
    assert_eq!(effect.duration.to_string(), "3/2");
    let document::AdjustmentEffectWire::ColorGrade(grade) = &effect.effect else {
        panic!("expected executable color-grade adjustment")
    };
    assert_eq!(grade.temperature, -0.25);

    let overlap = decode(json!({
        "canvas": {"width": 1920, "height": 1080, "fps": 30},
        "tracks": {
            "adjustment": [{
                "clips": [
                    {"start": 0, "duration": 2, "kind": "color-grade", "temperature": 0.1},
                    {"start": 1, "duration": 1, "kind": "color-grade", "temperature": 0.2}
                ]
            }]
        }
    }));
    assert!(matches!(
        compile_timeline(overlap),
        Err(CompileTimelineError::TrackOverlap { path, .. })
            if path == "/tracks/adjustment/0/clips/1"
    ));
}

#[test]
fn compact_caption_preset_expands_but_custom_presentation_remains_an_escape_hatch() {
    let preset = decode(json!({
        "canvas": {"width": 1920, "height": 1080, "fps": 30},
        "resources": {"font": "https://example.test/font.ttf"},
        "tracks": {
            "caption": [{
                "style": {"font": "font"},
                "clips": [{"start": 0, "duration": 2, "text": "hello", "enter": { "preset": "slide-up" }}]
            }]
        }
    }));
    let timeline = compile_timeline(preset).expect("expand compact preset name");
    let document::CaptionItemWire::Clip(caption) =
        &timeline.to_wire().document.captions.tracks[0].items[0]
    else {
        panic!("expected caption")
    };
    assert!(matches!(
        caption.presentation.opacity,
        document::ParamWire::Curve(_)
    ));
    assert!(matches!(
        caption.presentation.translation,
        document::ParamWire::Curve(_)
    ));

    let conflicting = json!({
        "canvas": {"width": 1920, "height": 1080, "fps": 30},
        "resources": {"font": "https://example.test/font.ttf"},
        "tracks": {
            "caption": [{
                "style": {"font": "font"},
                "clips": [{
                    "start": 0,
                    "duration": 2,
                    "text": "hello",
                    "enter": { "preset": "fade" },
                    "presentation": {"opacity": 0.5}
                }]
            }]
        }
    });
    assert!(validation_codes(conflicting).contains(&"preset_presentation_conflict".to_owned()));
}

#[test]
fn resource_alias_validation_and_track_order_fail_before_canonical_normalization() {
    let missing = json!({
        "canvas": {"width": 320, "height": 180, "fps": 25},
        "tracks": {
            "visual": [{
                "clips": [{"start": 0, "duration": 1, "kind": "image", "src": "missing"}]
            }]
        }
    });
    assert!(validation_codes(missing).contains(&"missing_resource".to_owned()));

    let overlap = decode(json!({
        "canvas": {"width": 320, "height": 180, "fps": 25},
        "tracks": {
            "visual": [{
                "clips": [
                    {"start": 0, "duration": 2, "kind": "solid", "color": "#000000ff"},
                    {"start": 1, "duration": 1, "kind": "solid", "color": "#ffffffff"}
                ]
            }]
        }
    }));
    assert!(matches!(
        compile_timeline(overlap),
        Err(CompileTimelineError::TrackOverlap { .. })
    ));
}

#[test]
fn resource_aliases_are_deliberately_small_and_portable() {
    let invalid = json!({
        "canvas": {"width": 320, "height": 180, "fps": 25},
        "resources": {"not/a-portable-alias": "https://example.test/image.png"},
        "tracks": {
            "visual": [{
                "clips": [{"start": 0, "duration": 1, "kind": "solid", "color": "#000000ff"}]
            }]
        }
    });
    assert!(validation_codes(invalid).contains(&"invalid_resource_key".to_owned()));
}

#[test]
fn motion_resources_and_inline_karaoke_timings_lower_to_canonical_data() {
    let author = decode(json!({
        "canvas": {"width": 1280, "height": 720, "fps": "30000/1001"},
        "resources": {
            "component": "https://example.test/component.json",
            "texture": "https://example.test/texture.png",
            "font": "https://example.test/font.ttf"
        },
        "tracks": {
            "visual": [{
                "clips": [{
                    "start": 0,
                    "duration": 3,
                    "kind": "motion",
                    "component": "component",
                    "resources": {"hero": "texture"},
                    "cues": {
                        "intro": {"type": "source-range", "start": 0, "end": 2}
                    }
                }]
            }],
            "caption": [{
                "style": {"font": "font"},
                "clips": [{
                    "start": 0,
                    "duration": 1,
                    "runs": [
                        {"text": "kara", "start": 0, "end": 0.4},
                        {"text": "oke", "start": 0.4, "end": 1}
                    ],
                    "behavior": {"type": "karaoke", "mode": "word"}
                }]
            }]
        }
    }));
    let timeline = compile_timeline(author).expect("compile all resource-bearing fields");
    let wire = timeline.to_wire().document;
    let document::VisualItemWire::Clip(clip) = &wire.visual.tracks[0].items[0] else {
        panic!("expected Motion clip")
    };
    let document::VisualSourceWire::Motion(motion) = &clip.source else {
        panic!("expected Motion source")
    };
    assert_eq!(motion.component, "resource:component");
    assert_eq!(motion.resources["hero"], "resource:texture");
    assert_eq!(motion.source_duration.to_string(), "3/1");

    let document::CaptionItemWire::Clip(caption) = &wire.captions.tracks[0].items[0] else {
        panic!("expected caption")
    };
    assert!(matches!(
        &caption.behavior,
        Some(document::CaptionBehaviorWire::Karaoke {
            mode: document::KaraokeModeWire::Word
        })
    ));
    assert_eq!(
        caption.runs[0].timing.as_ref().unwrap().start.to_string(),
        "0/1"
    );
    assert_eq!(
        caption.runs[1].timing.as_ref().unwrap().end.to_string(),
        "1/1"
    );
}

#[test]
fn visual_video_volume_and_pixel_size_survive_lowering_and_angles_are_degrees() {
    let timeline = compile_timeline(decode(json!({
        "canvas":{"width":640,"height":360,"fps":30}, "resources":{"v":"v.mp4"},
        "tracks":{"visual":[{"clips":[{"kind":"video","src":"v","start":0,"duration":2,
          "gain":{"keyframes":[[0,0],[1,2]]}, "size":[320,180], "rotation":90}]}]}
    })))
    .unwrap();
    let wire = serde_json::to_value(timeline.to_wire()).unwrap();
    let clip = &wire["document"]["visual"]["tracks"][0]["items"][0];
    assert_eq!(
        clip["layer"]["transform"]["size"]["value"],
        json!([320.0, 180.0])
    );
    let angle = clip["layer"]["transform"]["rotation"]["value"]
        .as_f64()
        .unwrap();
    assert!((angle - std::f64::consts::FRAC_PI_2).abs() < 0.000001);
    assert_eq!(clip["source"]["gain"]["keyframes"][1]["value"], json!(2.0));
    for (field, value) in [("gain", json!(-1)), ("size", json!([0, 180]))] {
        let mut source = json!({"canvas":{"width":320,"height":180,"fps":30},"resources":{"v":"v.mp4"},"tracks":{"visual":[{"clips":[{"kind":"video","src":"v","start":0,"duration":1}]}]}});
        source["tracks"]["visual"][0]["clips"][0][field] = value;
        assert!(compile_timeline(decode(source)).is_err());
    }
}
