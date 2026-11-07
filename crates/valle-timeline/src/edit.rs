//! Edits of a document, the one implementation Studio and `valle timeline edit` share: each takes
//! the document's text and an operation, and gives the edited text, or an error and no change
//! where the result is not a valid document. Times are snapped to the canvas's frames.
//!
//! Clips and tracks are named by id or their JSON path, e.g. `tracks.visual[0].items[2]`.
//! All indices start at 0 and refer to the written arrays, including gaps and transitions.
//!
//! - `{ "op": "set", "clip", "field", "value" }`: a field of the clip; `null` removes it.
//!   Changing an id updates anchors that name it.
//! - `{ "op": "move", "clip", "start" }`: to start at `start` seconds: a placed clip's `start`,
//!   or an anchored one's offset. `{ "op": "move", "clip", "index" }`: a sequence's item to
//!   another place in it. Transitions whose original neighbors separate are removed.
//! - `{ "op": "trim", "clip", "head"?, "duration"? }`: `head` seconds off its start (a placed
//!   clip starts later, its source further in), then its length set to `duration`.
//! - `{ "op": "delete", "clip" }`: a sequence closes up behind it; adjacent transitions and
//!   clips anchored to it (including through other anchors) are deleted too.
//! - `{ "op": "add", "track", "clip" }`: a clip at the end of a track's sequence, or among its
//!   placed clips.
//!
//! The text written back is indented JSON, its keys in order.

use std::collections::{BTreeSet, HashSet};

use serde_json::{Map, Value};

use crate::{Document, Error};

/// `text` with `op` done.
pub fn apply(text: &str, op: &Value) -> Result<String, Error> {
    let mut root: Value = serde_json::from_str(text).map_err(|e| format!("not JSON: {e}"))?;
    let doc = Document::read_edit(&root)?;
    let fps = doc.canvas.fps.as_f64();
    let snap = |s: f64| (s * fps).round() / fps;
    let name = op["op"]
        .as_str()
        .ok_or("op: set, move, trim, delete or add")?;
    let number = |k: &str| {
        op[k]
            .as_f64()
            .ok_or_else(|| format!("{name} needs {k}: seconds"))
    };
    if name == "add" {
        let named = op["track"]
            .as_str()
            .ok_or("add needs track: an id or path such as tracks.visual[0]")?;
        let at = doc
            .tracks
            .iter()
            .find(|t| t.id == named)
            .map_or(named, |t| t.at.as_str());
        // A new project's first track, or a new track appended to those already there.
        let (tracks, index) = split(at).map_err(|_| {
            format!("no track {named:?}: use a track id or tracks.<kind>[n], with n from 0")
        })?;
        let kind = tracks
            .strip_prefix("tracks.")
            .ok_or("track: an id or tracks.<kind>[n]")?;
        if !["visual", "audio", "caption", "adjustment"].contains(&kind) {
            return Err(format!("no track kind {kind}"));
        }
        let list = root["tracks"]
            .as_object_mut()
            .ok_or("tracks: an object")?
            .entry(kind)
            .or_insert_with(|| Value::Array(Vec::new()));
        let list = list.as_array_mut().ok_or("tracks: a list")?;
        if index > list.len() {
            return Err(format!(
                "{at}: track index is from 0 to {} (the last creates a track)",
                list.len()
            ));
        }
        if index == list.len() {
            let sequence = ["visual", "audio"].contains(&kind) && op["clip"].get("start").is_none();
            list.push(match sequence {
                true => serde_json::json!({ "items": [] }),
                false => serde_json::json!({ "clips": [] }),
            });
        }
        let clip = op["clip"].clone();
        let track = object(find(&mut root, at)?, at)?;
        let list = match track.contains_key("items") {
            true => "items",
            false => "clips",
        };
        let list = track
            .entry(list)
            .or_insert_with(|| Value::Array(Vec::new()));
        list.as_array_mut()
            .ok_or("the track's list is not an array")?
            .push(clip);
        return checked(root);
    }
    let named = op["clip"].as_str().ok_or_else(|| {
        format!("{name} needs clip: an id or path such as tracks.visual[0].items[0]")
    })?;
    let clip = (doc.tracks.iter().flat_map(|t| &t.clips))
        .find(|c| c.id == named || c.at == named)
        .ok_or_else(|| format!("no clip {named}"))?;
    let at = clip.at.clone();
    let (list, index) = split(&at)?;
    let in_sequence = list.ends_with(".items");
    match name {
        "set" => {
            let field = op["field"].as_str().ok_or("set needs field")?;
            let o = object(find(&mut root, &at)?, &at)?;
            match &op["value"] {
                Value::Null => o.remove(field),
                v => o.insert(field.to_string(), v.clone()),
            };
            if field == "id" {
                let track = doc
                    .tracks
                    .iter()
                    .find(|t| t.clips.iter().any(|c| c.at == at))
                    .unwrap();
                let id = match &op["value"] {
                    Value::Null => format!("{}.{}", track.id, clip.index),
                    value => crate::alias(value, &format!("{at}.id"))?,
                };
                for c in doc.tracks.iter().flat_map(|t| &t.clips) {
                    if c.anchor.as_ref().is_some_and(|(to, _)| *to == clip.id) {
                        find(&mut root, &c.at)?["start"]["clip"] = id.clone().into();
                    }
                }
            }
        }
        "delete" => delete(&mut root, &doc, &clip.id)?,
        "move" if op.get("index").is_some() => {
            if !in_sequence {
                return Err(format!("{at} is placed by start: move it with start"));
            }
            let to = op["index"]
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .ok_or("index: a JSON array position from 0")?;
            let list = find(&mut root, &list)?.as_array_mut().ok_or("not a list")?;
            if to >= list.len() {
                return Err(format!("index: from 0 to {}, not {to}", list.len() - 1));
            }
            // Keep a transition only when it still connects its original pair. Moving a clip
            // away, or inserting one into that pair, must not silently change the transition.
            let mut order: Vec<usize> = (0..list.len()).collect();
            order.remove(index);
            order.insert(to, index);
            let keep: Vec<usize> = order
                .iter()
                .enumerate()
                .filter_map(|(n, &i)| {
                    let connected = n > 0
                        && order.get(n - 1) == i.checked_sub(1).as_ref()
                        && order.get(n + 1) == Some(&(i + 1));
                    (list[i].get("transition").is_none() || connected).then_some(i)
                })
                .collect();
            *list = keep.into_iter().map(|i| list[i].take()).collect();
        }
        "move" => {
            if in_sequence {
                return Err(format!("{at} is in a sequence: move it with index"));
            }
            let start = snap(number("start")?.max(0.0));
            let anchor = clip.anchor.as_ref().map(|(id, _)| id.clone());
            let anchored = anchor
                .and_then(|id| doc.clip(&id))
                .map(|a| a.start.seconds());
            let o = object(find(&mut root, &at)?, &at)?;
            match anchored {
                Some(from) => o["start"]["offset"] = seconds(snap(start - from)),
                None => o["start"] = seconds(start),
            }
        }
        "trim" => {
            let head = op
                .get("head")
                .map(|_| number("head"))
                .transpose()?
                .unwrap_or(0.0);
            let head = snap(head);
            let mut duration = snap(clip.duration.seconds() - head);
            if op.get("duration").is_some() {
                duration = snap(number("duration")?);
            }
            if duration <= 0.0 {
                return Err(format!("{at}: trimmed to nothing"));
            }
            let o = object(find(&mut root, &at)?, &at)?;
            if head != 0.0 {
                if o.contains_key("timeMap") {
                    return Err(format!("{at} maps its time: trim its timeMap"));
                }
                // The source goes further in, at its rate, where the clip has one.
                if ["src", "component"].iter().any(|k| o.contains_key(*k)) {
                    let rate = o.get("rate").and_then(Value::as_f64).unwrap_or(1.0);
                    let from = o.get("trimStart").and_then(Value::as_f64).unwrap_or(0.0);
                    o.insert("trimStart".into(), seconds(from + head * rate));
                }
                if !in_sequence {
                    match o.get_mut("start") {
                        Some(Value::Object(anchor)) => {
                            let offset = anchor.get("offset").and_then(Value::as_f64);
                            anchor.insert("offset".into(), seconds(offset.unwrap_or(0.0) + head));
                        }
                        Some(start) => *start = seconds(start.as_f64().unwrap_or(0.0) + head),
                        None => {}
                    }
                }
            }
            o.insert("duration".into(), seconds(duration));
        }
        other => return Err(format!("op {other}: set, move, trim, delete or add")),
    }
    checked(root)
}

/// Delete the clip, its dependent clips and transitions, preserving unrelated identities.
fn delete(root: &mut Value, doc: &Document, id: &str) -> Result<(), Error> {
    let mut removed = HashSet::from([id]);
    loop {
        let before = removed.len();
        for clip in doc.tracks.iter().flat_map(|t| &t.clips) {
            if clip
                .anchor
                .as_ref()
                .is_some_and(|(to, _)| removed.contains(to.as_str()))
            {
                removed.insert(clip.id.as_str());
            }
        }
        if removed.len() == before {
            break;
        }
    }
    for track in &doc.tracks {
        let mut indices = BTreeSet::new();
        for clip in &track.clips {
            if removed.contains(clip.id.as_str()) {
                indices.insert(clip.index);
            }
        }
        for t in &track.transitions {
            if removed.contains(track.clips[t.from].id.as_str())
                || removed.contains(track.clips[t.to].id.as_str())
            {
                indices.insert(split(&t.at)?.1);
            }
        }
        if !indices.is_empty() {
            // Generated ids depend on array indices. Keep surviving identities explicit before
            // shifting siblings; otherwise anchors could change targets or ids could collide.
            for clip in &track.clips {
                if !clip.named && !removed.contains(clip.id.as_str()) {
                    find(root, &clip.at)?["id"] = clip.id.clone().into();
                }
            }
            let at = split(&track.clips[0].at)?.0;
            let list = find(root, &at)?.as_array_mut().ok_or("not a list")?;
            for i in indices.into_iter().rev() {
                list.remove(i);
            }
        }
    }
    Ok(())
}

/// The text of `root`, if it is a valid document.
fn checked(root: Value) -> Result<String, Error> {
    Document::read_edit(&root)?;
    let mut text = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
    text.push('\n');
    Ok(text)
}

/// Seconds as JSON, rounded off past a millionth of a frame's worth of float noise.
fn seconds(s: f64) -> Value {
    let rounded = (s * 1e9).round() / 1e9;
    match rounded.fract() == 0.0 && rounded.abs() < 1e15 {
        true => Value::from(rounded as i64),
        false => Value::from(rounded),
    }
}

/// `tracks.visual[0].items[2]` as `tracks.visual[0].items` and 2.
fn split(at: &str) -> Result<(String, usize), Error> {
    let (list, index) = at
        .strip_suffix(']')
        .and_then(|s| s.rsplit_once('['))
        .ok_or_else(|| format!("{at}: not in a list"))?;
    let index = index.parse().map_err(|_| format!("{at}: no index"))?;
    Ok((list.to_string(), index))
}

/// The value at `path` in `root`: keys by `.`, list items by `[n]`.
fn find<'a>(root: &'a mut Value, path: &str) -> Result<&'a mut Value, Error> {
    let mut here = root;
    for part in path.split('.') {
        let (key, indices) = part.split_once('[').map_or((part, ""), |(k, i)| (k, i));
        here = here
            .get_mut(key)
            .ok_or_else(|| format!("{path}: no {key}"))?;
        for index in indices.split('[').filter(|i| !i.is_empty()) {
            let n: usize = index
                .trim_end_matches(']')
                .parse()
                .map_err(|_| format!("{path}?"))?;
            here = here
                .get_mut(n)
                .ok_or_else(|| format!("{path}: no item {n}"))?;
        }
    }
    Ok(here)
}

fn object<'a>(v: &'a mut Value, at: &str) -> Result<&'a mut Map<String, Value>, Error> {
    v.as_object_mut()
        .ok_or_else(|| format!("{at}: not an object"))
}

/// Where each clip is, for an editor to draw: the document's length and rate, and each track's
/// clips with their places, starts and lengths in seconds, kinds, names and anchors.
pub fn layout(text: &str) -> Result<Value, Error> {
    let root = serde_json::from_str(text).map_err(|e| format!("not JSON: {e}"))?;
    let doc = Document::read_edit(&root)?;
    let tracks: Vec<Value> = (doc.tracks.iter())
        .map(|t| {
            let clips: Vec<Value> = (t.clips.iter())
                .map(|c| {
                    serde_json::json!({
                        "id": c.id,
                        "named": c.named,
                        "at": c.at,
                        "name": c.name,
                        "start": c.start.seconds(),
                        "duration": c.duration.seconds(),
                        "enabled": c.enabled,
                        "anchor": c.anchor.as_ref().map(|(id, _)| id),
                        "kind": kind(&c.content),
                    })
                })
                .collect();
            serde_json::json!({ "id": t.id, "kind": t.kind.name(), "at": t.at, "clips": clips })
        })
        .collect();
    Ok(serde_json::json!({
        "duration": doc.duration.seconds(),
        "fps": doc.canvas.fps.as_f64(),
        "width": doc.canvas.width,
        "height": doc.canvas.height,
        "tracks": tracks,
    }))
}

fn kind(content: &crate::Content) -> &'static str {
    use crate::Content::*;
    match content {
        Solid { .. } => "solid",
        Image { .. } => "image",
        Video { .. } => "video",
        Motion(_) => "motion",
        Timeline { .. } => "timeline",
        Audio { .. } => "audio",
        Caption { .. } => "caption",
        Grade { .. } => "grade",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const DOC: &str = r##"{
      "canvas": { "width": 64, "height": 36, "fps": 30 },
      "tracks": {
        "visual": [
          { "items": [
              { "id": "a", "kind": "solid", "color": "#ff0000ff", "duration": 1 },
              { "id": "b", "kind": "solid", "color": "#00ff00ff", "duration": 2 }
          ] },
          { "clips": [
              { "id": "c", "kind": "solid", "color": "#0000ffff", "start": 0.5, "duration": 1 }
          ] },
          { "clips": [
              { "kind": "solid", "color": "#ffffffff", "start": { "clip": "b", "offset": 0.5 }, "duration": 0.5 }
          ] }
        ]
      }
    }"##;

    fn edited(op: Value) -> Value {
        let text = apply(DOC, &op).expect("edited");
        serde_json::from_str(&text).expect("JSON")
    }

    #[test]
    fn placed_clips_move_and_anchored_ones_keep_their_anchor() {
        let doc = edited(json!({ "op": "move", "clip": "c", "start": 1.01 }));
        // Snapped to a frame at 30 fps.
        assert_eq!(doc["tracks"]["visual"][1]["clips"][0]["start"], json!(1));
        let doc = edited(json!({ "op": "move", "clip": "tracks.visual[2].clips[0]", "start": 2 }));
        // `b` starts at 1 s: the offset is from there.
        assert_eq!(
            doc["tracks"]["visual"][2]["clips"][0]["start"],
            json!({ "clip": "b", "offset": 1 })
        );
    }

    #[test]
    fn sequences_reorder_and_close_up() {
        let doc = edited(json!({ "op": "move", "clip": "b", "index": 0 }));
        assert_eq!(doc["tracks"]["visual"][0]["items"][0]["id"], "b");
        let out = layout(&apply(DOC, &json!({ "op": "delete", "clip": "c" })).expect("ok"));
        assert_eq!(
            out.expect("laid out")["tracks"][1]["clips"]
                .as_array()
                .map(Vec::len),
            Some(0)
        );
        let removed = edited(json!({ "op": "delete", "clip": "b" }));
        assert_eq!(removed["tracks"]["visual"][2]["clips"], json!([]));
    }

    #[test]
    fn trims_move_the_start_and_the_source() {
        let doc = edited(json!({ "op": "trim", "clip": "c", "head": 0.25, "duration": 0.5 }));
        let c = &doc["tracks"]["visual"][1]["clips"][0];
        // 0.25 s is 7.5 frames: snapped to 8, 0.2666… s.
        assert_eq!(c["duration"], json!(0.5));
        assert!((c["start"].as_f64().expect("a start") - (0.5 + 8.0 / 30.0)).abs() < 1e-9);
        assert!(apply(DOC, &json!({ "op": "trim", "clip": "a", "duration": 0 })).is_err());
    }

    #[test]
    fn fields_are_set_and_removed_and_clips_added() {
        let doc = edited(json!({ "op": "set", "clip": "a", "field": "name", "value": "Open" }));
        assert_eq!(doc["tracks"]["visual"][0]["items"][0]["name"], "Open");
        let bad = json!({ "op": "set", "clip": "a", "field": "color", "value": 3 });
        assert!(apply(DOC, &bad).is_err());
        let clip = json!({ "id": "d", "kind": "solid", "color": "#000000ff", "duration": 1 });
        let doc = edited(json!({ "op": "add", "track": "tracks.visual[0]", "clip": clip }));
        assert_eq!(doc["tracks"]["visual"][0]["items"][2]["id"], "d");
        let laid = layout(&serde_json::to_string(&doc).expect("text")).expect("laid out");
        assert_eq!(laid["tracks"][0]["clips"][2]["start"], json!(3.0));
    }

    #[test]
    fn empty_timelines_can_be_edited_but_not_rendered() {
        let empty = r#"{"canvas":{"width":64,"height":36,"fps":30},"tracks":{}}"#;
        assert_eq!(layout(empty).unwrap()["duration"], 0.0);
        let full = apply(
            empty,
            &json!({"op":"add","track":"tracks.visual[0]",
            "clip":{"id":"first","kind":"solid","color":"red","start":0,"duration":1}}),
        )
        .unwrap();
        assert!(crate::parse(&full).is_ok());
        let empty = apply(&full, &json!({"op":"delete","clip":"first"})).unwrap();
        assert_eq!(layout(&empty).unwrap()["duration"], 0.0);
        assert!(crate::parse(&empty).is_err());
    }

    #[test]
    fn adding_accepts_a_track_id() {
        let mut doc: Value = serde_json::from_str(DOC).unwrap();
        doc["tracks"]["visual"][0]["id"] = "story".into();
        let text = apply(
            &doc.to_string(),
            &json!({"op":"add","track":"story",
            "clip":{"id":"added","kind":"solid","color":"blue","duration":1}}),
        )
        .unwrap();
        assert_eq!(
            layout(&text).unwrap()["tracks"][0]["clips"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
    }

    const LINKED: &str = r#"{
      "canvas": {"width":64,"height":36,"fps":30},
      "tracks": {
        "visual": [{"id":"story","items":[
          {"id":"a","kind":"solid","color":"red","duration":1},
          {"transition":"fade","inOffset":0.1,"outOffset":0.1},
          {"id":"b","kind":"solid","color":"green","duration":1},
          {"transition":"wipeLeft","inOffset":0.1,"outOffset":0.1},
          {"id":"c","kind":"solid","color":"blue","duration":1},
          {"gap":1},
          {"id":"d","kind":"solid","color":"white","duration":1}
        ]}],
        "caption": [
          {"clips":[{"id":"title","text":"b","start":{"clip":"b","offset":0.1},"duration":0.5,"metadata":{"clip":"b"}}]},
          {"clips":[{"id":"detail","text":"detail","start":{"clip":"title"},"duration":0.5}]},
          {"clips":[{"id":"other","text":"other","start":{"clip":"d"},"duration":0.5}]}
        ]
      }
    }"#;

    #[test]
    fn deleting_removes_only_connected_transitions_and_recursive_anchors() {
        for (id, transitions) in [("a", 1), ("b", 0), ("c", 1)] {
            let text = apply(LINKED, &json!({"op":"delete","clip":id})).unwrap();
            let doc = crate::parse(&text).unwrap();
            assert!(doc.clip(id).is_none());
            assert_eq!(doc.tracks[0].transitions.len(), transitions);
            assert_eq!(doc.clip("title").is_some(), id != "b");
            assert_eq!(doc.clip("detail").is_some(), id != "b");
            assert_eq!(
                doc.clip("other").unwrap().start,
                doc.clip("d").unwrap().start
            );
            let root: Value = serde_json::from_str(&text).unwrap();
            assert!(
                root["tracks"]["visual"][0]["items"]
                    .as_array()
                    .unwrap()
                    .contains(&json!({"gap":1}))
            );
        }
    }

    #[test]
    fn renaming_updates_only_anchor_references_and_refuses_duplicate_ids() {
        let text = apply(
            LINKED,
            &json!({"op":"set","clip":"b","field":"id","value":"hero"}),
        )
        .unwrap();
        let doc = crate::parse(&text).unwrap();
        assert_eq!(
            doc.clip("title").unwrap().anchor.as_ref().unwrap().0,
            "hero"
        );
        assert_eq!(
            doc.clip("detail").unwrap().anchor.as_ref().unwrap().0,
            "title"
        );
        assert_eq!(
            doc.clip("title").unwrap().metadata.as_ref().unwrap()["clip"],
            "b"
        );
        let renamed = apply(
            &text,
            &json!({"op":"set","clip":"title","field":"id","value":"headline"}),
        )
        .unwrap();
        assert_eq!(
            crate::parse(&renamed)
                .unwrap()
                .clip("detail")
                .unwrap()
                .anchor
                .as_ref()
                .unwrap()
                .0,
            "headline"
        );
        assert!(
            apply(
                LINKED,
                &json!({"op":"set","clip":"b","field":"id","value":"a"})
            )
            .is_err()
        );
    }

    #[test]
    fn deleting_an_earlier_unnamed_clip_does_not_retarget_surviving_anchors() {
        let text = r#"{"canvas":{"width":64,"height":36,"fps":30},"tracks":{
          "visual":[{"clips":[
            {"kind":"solid","color":"red","start":0,"duration":1},
            {"kind":"solid","color":"blue","start":2,"duration":1},
            {"kind":"solid","color":"green","start":4,"duration":1}
          ]}],
          "caption":[{"clips":[{"id":"title","text":"blue","start":{"clip":"visual0.1"},"duration":1}]}]
        }}"#;
        let text = apply(
            text,
            &json!({"op":"delete","clip":"tracks.visual[0].clips[0]"}),
        )
        .unwrap();
        let doc = crate::parse(&text).unwrap();
        assert_eq!(doc.clip("title").unwrap().start.seconds(), 2.0);
        assert!(doc.clip("visual0.1").unwrap().named);
        // Removing an authored id rewrites anchors to the resulting generated id too.
        let text = apply(
            &text,
            &json!({"op":"set","clip":"visual0.1","field":"id","value":null}),
        )
        .unwrap();
        assert_eq!(
            crate::parse(&text)
                .unwrap()
                .clip("title")
                .unwrap()
                .start
                .seconds(),
            2.0
        );
    }

    #[test]
    fn moving_breaks_only_transitions_whose_original_neighbors_separate() {
        for (clip, index, transitions) in [("b", 0, 0), ("a", 6, 1), ("d", 2, 1), ("b", 2, 2)] {
            let text = apply(LINKED, &json!({"op":"move","clip":clip,"index":index})).unwrap();
            let doc = crate::parse(&text).unwrap();
            assert_eq!(
                doc.tracks[0].transitions.len(),
                transitions,
                "{clip} -> {index}"
            );
            assert_eq!(
                doc.clip("title").unwrap().start,
                doc.clip("b").unwrap().start + crate::time::Tick::parse_seconds("0.1").unwrap()
            );
        }
        for index in [json!(-1), json!(7), json!(1.5)] {
            assert!(
                apply(LINKED, &json!({"op":"move","clip":"a","index":index}))
                    .unwrap_err()
                    .contains("index:")
            );
        }
    }

    #[test]
    fn track_paths_create_only_the_next_track_and_invalid_paths_do_not_panic() {
        let empty = r#"{"canvas":{"width":64,"height":36,"fps":30},"tracks":{}}"#;
        let clip = json!({"id":"first","kind":"solid","color":"red","duration":1});
        let text = apply(
            empty,
            &json!({"op":"add","track":"tracks.visual[0]","clip":clip}),
        )
        .unwrap();
        assert_eq!(
            crate::parse(&text).unwrap().clip("first").unwrap().at,
            "tracks.visual[0].items[0]"
        );
        for track in [
            "[",
            "tracks.visual[",
            "tracks.visual[0",
            "tracks.visual[-1]",
            "tracks.visual[2]",
            "missing",
        ] {
            assert!(
                apply(empty, &json!({"op":"add","track":track,"clip":clip})).is_err(),
                "{track}"
            );
        }
        let duplicate = r#"{"canvas":{"width":64,"height":36,"fps":30},"tracks":{"visual":[{"id":"same"},{"id":"same"}]}}"#;
        assert!(layout(duplicate).unwrap_err().contains("another track"));
    }
}
