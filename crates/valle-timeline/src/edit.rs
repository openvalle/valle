//! Edits of a document, the one implementation Studio and `valle timeline edit` share: each takes
//! the document's text and an operation, and gives the edited text, or an error and no change
//! where the result is not a valid document. Times are snapped to the canvas's frames.
//!
//! A clip is named by its `id`, or by where it is written: `tracks.visual[0].items[2]`.
//!
//! - `{ "op": "set", "clip", "field", "value" }`: a field of the clip; `null` removes it.
//! - `{ "op": "move", "clip", "start" }`: to start at `start` seconds: a placed clip's `start`,
//!   or an anchored one's offset. `{ "op": "move", "clip", "index" }`: a sequence's item to
//!   another place in it.
//! - `{ "op": "trim", "clip", "head"?, "duration"? }`: `head` seconds off its start (a placed
//!   clip starts later, its source further in), then its length set to `duration`.
//! - `{ "op": "delete", "clip" }`: a sequence closes up behind it.
//! - `{ "op": "add", "track", "clip" }`: a clip at the end of a track's sequence, or among its
//!   placed clips.
//!
//! The text written back is indented JSON, its keys in order.

use serde_json::{Map, Value};

use crate::{Document, Error};

/// `text` with `op` done.
pub fn apply(text: &str, op: &Value) -> Result<String, Error> {
    let mut root: Value = serde_json::from_str(text).map_err(|e| format!("not JSON: {e}"))?;
    let doc = Document::read(&root)?;
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
        let at = op["track"]
            .as_str()
            .ok_or("add needs track: where it is, tracks.visual[0]")?;
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
    let named = op["clip"]
        .as_str()
        .ok_or_else(|| format!("{name} needs clip: an id or where"))?;
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
        }
        "delete" => {
            let list = find(&mut root, &list)?.as_array_mut().ok_or("not a list")?;
            list.remove(index);
        }
        "move" if op.get("index").is_some() => {
            if !in_sequence {
                return Err(format!("{at} is placed by start: move it with start"));
            }
            let to = op["index"]
                .as_u64()
                .ok_or("index: a place in the sequence")? as usize;
            let list = find(&mut root, &list)?.as_array_mut().ok_or("not a list")?;
            let item = list.remove(index);
            list.insert(to.min(list.len()), item);
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

/// The text of `root`, if it is a valid document.
fn checked(root: Value) -> Result<String, Error> {
    Document::read(&root)?;
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
    let open = at
        .rfind('[')
        .ok_or_else(|| format!("{at}: not in a list"))?;
    let index = at[open + 1..at.len() - 1]
        .parse()
        .map_err(|_| format!("{at}: no index"))?;
    Ok((at[..open].to_string(), index))
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
    let doc = crate::parse(text)?;
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
        // Deleting what a clip is anchored to leaves an anchor to nothing: refused.
        let refused = apply(DOC, &json!({ "op": "delete", "clip": "b" }));
        assert!(refused.is_err(), "{refused:?}");
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
}
