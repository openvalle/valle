use super::*;

fn doc(tracks: &str) -> Result<Document, Error> {
    parse(&format!(
        r#"{{ "canvas": {{ "width": 64, "height": 36, "fps": 30 }},
              "resources": {{ "a": "a.mp4", "b": "b.png" }},
              "tracks": {tracks} }}"#
    ))
}

fn s(text: &str) -> Tick {
    Tick::parse_seconds(text).unwrap()
}

#[test]
fn a_sequence_places_its_clips_one_after_another() {
    let d = doc(r#"{ "visual": [{ "items": [
            { "id": "c1", "kind": "solid", "color": "red", "duration": 1.5 },
            { "gap": 0.5 },
            { "id": "c2", "kind": "video", "src": "a", "duration": 2, "trimStart": 3 },
            { "transition": "fade", "inOffset": 0.5, "outOffset": 0.25 },
            { "id": "c3", "kind": "image", "src": "b", "duration": 1 }
        ] }] }"#)
    .unwrap();
    let clips = &d.tracks[0].clips;
    let spans: Vec<(Tick, Tick)> = clips.iter().map(|c| (c.start, c.end())).collect();
    assert_eq!(
        spans,
        [(s("0"), s("1.5")), (s("2"), s("4")), (s("4"), s("5"))]
    );
    let t = &d.tracks[0].transitions[0];
    assert_eq!((t.from, t.to), (1, 2));
    assert_eq!(t.span(&d.tracks[0]), (s("3.5"), s("4.25")));
    assert_eq!(d.duration, s("5"));
    assert_eq!(d.tracks[0].id, "visual0");
}

#[test]
fn anchored_clips_follow_their_anchor() {
    let d = doc(r#"{ "visual": [
              { "items": [
                { "id": "c1", "kind": "solid", "color": "red", "duration": 2 },
                { "id": "c2", "kind": "solid", "color": "blue", "duration": 2 } ] },
              { "id": "V2", "clips": [
                { "id": "t", "kind": "solid", "color": "white", "duration": 1,
                  "start": { "clip": "c2", "offset": 0.5 } },
                { "kind": "solid", "color": "black", "duration": 1, "start": { "clip": "t", "offset": 1 } } ] }
            ],
            "audio": [{ "clips": [{ "src": "a", "start": { "clip": "c1" }, "duration": 2 }] }] }"#)
    .unwrap();
    assert_eq!(d.clip("t").unwrap().start, s("2.5"));
    assert_eq!(d.clip("V2.2").unwrap().start, s("3.5"));
    assert_eq!(d.clip("audio0.1").unwrap().start, s("0"));
}

#[test]
fn cuts_land_on_frames() {
    let d = parse(
        r#"{ "canvas": { "width": 64, "height": 36, "fps": "30000/1001" },
             "tracks": { "visual": [{ "items": [
               { "id": "a", "kind": "solid", "color": "red", "duration": 0.033367 },
               { "id": "b", "kind": "solid", "color": "red", "duration": 0.033367 } ] }] } }"#,
    )
    .unwrap();
    let fps = Rate::parse("30000/1001").unwrap();
    assert_eq!(d.clip("b").unwrap().start, fps.frame_start(1).unwrap());
    assert_eq!(d.duration, fps.frame_start(2).unwrap());
}

#[test]
fn mistakes_name_the_field_and_what_to_write() {
    let cases = [
        (
            r#"{ "visual": [{ "items": [{ "id": "a", "kind": "solid", "color": "red", "duration": 1, "speed": 2 }] }] }"#,
            "tracks.visual[0].items[0]: no field \"speed\"",
        ),
        (
            r#"{ "visual": [{ "items": [{ "kind": "solid", "color": "red", "duration": 1 }] }] }"#,
            "tracks.visual[0].items[0].id: a clip in a sequence has an id",
        ),
        (
            r#"{ "visual": [{ "clips": [{ "kind": "solid", "color": "red", "start": "00:00:01:00", "duration": 1 }] }] }"#,
            "a timecode is only a label",
        ),
        (
            r#"{ "visual": [{ "clips": [
                { "id": "a", "kind": "solid", "color": "red", "start": 0, "duration": 2 },
                { "id": "b", "kind": "solid", "color": "red", "start": 1, "duration": 2 } ] }] }"#,
            "tracks.visual[0].clips[1]: it overlaps tracks.visual[0].clips[0]",
        ),
        (
            r#"{ "visual": [{ "clips": [
                { "id": "a", "kind": "solid", "color": "red", "start": { "clip": "b" }, "duration": 2 },
                { "id": "b", "kind": "solid", "color": "red", "start": { "clip": "a" }, "duration": 2 } ] }] }"#,
            "anchors go round in a loop",
        ),
        (
            r#"{ "visual": [{ "items": [
                { "id": "a", "kind": "solid", "color": "red", "duration": 1 },
                { "transition": "circleOpen", "inOffset": 0.5, "params": { "radius": 2 } },
                { "id": "b", "kind": "solid", "color": "red", "duration": 1 } ] }] }"#,
            "circleOpen has no param radius; it takes centerX, centerY, softness",
        ),
        (
            r#"{ "visual": [{ "items": [
                { "id": "a", "kind": "solid", "color": "red", "duration": 1 },
                { "gap": 1 },
                { "transition": "fade", "inOffset": 0.5 },
                { "id": "b", "kind": "solid", "color": "red", "duration": 1 } ] }] }"#,
            "a transition goes between two clips",
        ),
        (
            r#"{ "visual": [{ "items": [{ "id": "a", "kind": "video", "src": "a", "duration": 1, "rate": 2, "timeMap": [[0, 0], [1, 1]] }] }] }"#,
            "a time map replaces trimStart and rate",
        ),
        (
            r#"{ "audio": [{ "items": [
                { "id": "a", "src": "a", "duration": 1 },
                { "transition": "fade", "inOffset": 0.5 },
                { "id": "b", "src": "a", "duration": 1 } ] }] }"#,
            "a sound transition is \"crossfade\"",
        ),
        (r#"{ "visual": [] }"#, "the timeline has no clips"),
    ];
    for (tracks, expected) in cases {
        let Err(e) = doc(tracks) else {
            panic!("no error; expected {expected}");
        };
        assert!(e.contains(expected), "{e}\nexpected: {expected}");
    }
}

#[test]
fn keyframes_rise_and_ease_the_stretch_after_them() {
    let d = doc(
        r#"{ "visual": [{ "clips": [{ "kind": "solid", "color": "red", "start": 0, "duration": 3,
              "opacity": { "keyframes": [[0, 0, "ease-out"], [0.4, 1], [3, 0]] } }] }] }"#,
    )
    .unwrap();
    let Some(Animated::Keys { keys, step }) = &d.tracks[0].clips[0].look.opacity else {
        panic!("keyframes");
    };
    assert!(!step);
    assert_eq!(keys.len(), 3);
    assert_eq!(keys[0].easing.as_deref(), Some("ease-out"));
    let e = doc(
        r#"{ "visual": [{ "clips": [{ "kind": "solid", "color": "red", "start": 0, "duration": 3,
              "opacity": { "keyframes": [[1, 0], [0.5, 1]] } }] }] }"#,
    );
    assert!(e.err().unwrap().contains("keyframe times rise"));
}

#[test]
fn timecodes_are_labels_the_rate_counts() {
    let e = parse(
        r#"{ "canvas": { "width": 64, "height": 36, "fps": 25 }, "startTimecode": "01:00:00;00",
             "tracks": { "visual": [{ "items": [{ "id": "a", "kind": "solid", "color": "red", "duration": 1 }] }] } }"#,
    );
    assert!(e.err().unwrap().starts_with("startTimecode: timecode"));
}
