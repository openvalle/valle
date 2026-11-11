//! Edits of a document, the one implementation Studio and `valle timeline edit` share: each takes
//! the document's text and an operation, and gives the edited text, or an error and no change
//! where the result is not a valid document. Times are snapped to the canvas's frames.
//!
//! Clips and tracks are named by id or their JSON path, e.g. `tracks.visual[0].items[2]`.
//! All indices start at 0 and refer to the written arrays, including gaps and transitions.
//!
//! - `{ "op": "set", "clip", "field", "value" }`: a field of the clip; `null` removes it.
//!   Changing an id updates anchors that name it. `{ "op": "set", "track", "field", "value" }`:
//!   a track's own field, such as `enabled`.
//! - `{ "op": "move", "clip", "start" }`: to start at `start` seconds: a placed clip's `start`,
//!   or an anchored one's offset. `{ "op": "move", "clip", "index" }`: a sequence's item to
//!   another place in it. Transitions whose original neighbors separate are removed.
//!   `{ "op": "move", "clip", "track", "start"? }`: to another track of its kind: at the end of
//!   a sequence, or among placed clips at `start`, else where it was.
//! - `{ "op": "trim", "clip", "head"?, "duration"? }`: `head` seconds off its start (a placed
//!   clip starts later, its source further in), then its length set to `duration`.
//! - `{ "op": "delete", "clip" }`: a sequence closes up behind it; adjacent transitions and
//!   clips anchored to it (including through other anchors) are deleted too.
//! - `{ "op": "add", "track", "clip" }`: a clip at the end of a track's sequence, or among its
//!   placed clips.
//!
//! The text written back is indented JSON, its keys in the order they were written; a value
//! that fits its line stays on one.

use std::collections::{BTreeMap, BTreeSet, HashSet};

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
        let at = &track_at(&mut root, &doc, named, op["clip"].get("start").is_some())?;
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
    if name == "set" && op.get("clip").is_none() && op.get("track").is_some() {
        let named = op["track"].as_str().ok_or("track: an id or path")?;
        let track = (doc.tracks.iter())
            .find(|t| t.id == named || t.at == named)
            .ok_or_else(|| format!("no track {named}"))?;
        let field = op["field"].as_str().ok_or("set needs field")?;
        if ["items", "clips"].contains(&field) {
            return Err(format!("{}.{field}: edit its clips one by one", track.at));
        }
        // A generated clip id includes the track id. Preserve it before renaming the track,
        // so anchors and the editor's selection keep referring to the same clip.
        if field == "id" {
            for clip in &track.clips {
                if !clip.named {
                    find(&mut root, &clip.at)?["id"] = clip.id.clone().into();
                }
            }
        }
        let o = object(find(&mut root, &track.at)?, &track.at)?;
        match &op["value"] {
            Value::Null => o.shift_remove(field),
            v => o.insert(field.to_string(), v.clone()),
        };
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
                Value::Null => o.shift_remove(field),
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
        "move" if op.get("track").is_some() => {
            let to = op["track"].as_str().ok_or("track: an id or path")?;
            let from = (doc.tracks.iter())
                .find(|t| t.clips.iter().any(|c| c.at == at))
                .unwrap();
            let target = track_at(
                &mut root,
                &doc,
                to,
                !in_sequence || op.get("start").is_some(),
            )?;
            if target == from.at {
                return Err(format!("{at} is on {to} already"));
            }
            if target.split('[').next() != from.at.split('[').next() {
                return Err(format!(
                    "{at} is a {} clip: it moves to another {} track",
                    from.kind.name(),
                    from.kind.name()
                ));
            }
            // Generated ids depend on the track and the index: keep the moved clip's id as
            // well as its former neighbors', for anchors and the editor's selection.
            for c in &from.clips {
                if !c.named {
                    find(&mut root, &c.at)?["id"] = c.id.clone().into();
                }
            }
            let mut indices = BTreeSet::from([index]);
            for t in &from.transitions {
                if from.clips[t.from].at == at || from.clips[t.to].at == at {
                    indices.insert(split(&t.at)?.1);
                }
            }
            let source = find(&mut root, &list)?.as_array_mut().ok_or("not a list")?;
            let mut moved = Value::Null;
            for i in indices.into_iter().rev() {
                let item = source.remove(i);
                if i == index {
                    moved = item;
                }
            }
            let o = object(&mut moved, &at)?;
            let track = object(find(&mut root, &target)?, &target)?;
            let sequence = track.contains_key("items");
            if sequence {
                o.shift_remove("start");
                if !o.contains_key("id") {
                    o.insert("id".into(), clip.id.clone().into());
                }
            } else if op.get("start").is_some() {
                let start = snap(number("start")?.max(0.0));
                let base = (clip.anchor.as_ref())
                    .and_then(|(id, _)| doc.clip(id))
                    .map(|a| a.start.seconds());
                match (base, o.get_mut("start")) {
                    (Some(from), Some(Value::Object(anchor))) => {
                        anchor.insert("offset".into(), seconds(snap(start - from)));
                    }
                    _ => {
                        o.insert("start".into(), seconds(start));
                    }
                }
            } else if !o.contains_key("start") {
                o.insert("start".into(), seconds(clip.start.seconds()));
            }
            let list = if sequence { "items" } else { "clips" };
            (track.entry(list))
                .or_insert_with(|| Value::Array(Vec::new()))
                .as_array_mut()
                .ok_or("the track's list is not an array")?
                .push(moved);
        }
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

/// An edit and its effects, shared by command-line and interactive editors.
pub struct Edited {
    pub text: String,
    /// Changed clips and transitions, with their authored addresses and resolved time spans.
    pub changes: Vec<Value>,
}

impl Edited {
    pub fn summary(&self) -> String {
        let mut lines = Vec::new();
        for change in &self.changes {
            let item = if change["after"].is_null() {
                &change["before"]
            } else {
                &change["after"]
            };
            let action = change["action"].as_str().unwrap_or_default();
            lines.push(if change["kind"] == "track" {
                format!(
                    "{action} track {} at {}",
                    item["id"].as_str().unwrap_or_default(),
                    item["path"].as_str().unwrap_or_default()
                )
            } else if change["kind"] == "clip" {
                format!(
                    "{action} {} at {}: {}–{} s",
                    item["id"].as_str().unwrap_or_default(),
                    item["path"].as_str().unwrap_or_default(),
                    item["start"],
                    item["end"]
                )
            } else {
                format!(
                    "{action} transition {} → {} at {}",
                    item["from"].as_str().unwrap_or_default(),
                    item["to"].as_str().unwrap_or_default(),
                    item["path"].as_str().unwrap_or_default()
                )
            });
        }
        if lines.is_empty() {
            "no changes".into()
        } else {
            lines.join("\n")
        }
    }
}

/// Apply once, then describe both direct and dependent changes. Matching uses clip identities,
/// not array positions, which move when a sequence is reordered or shortened.
pub fn report(text: &str, op: &Value) -> Result<Edited, Error> {
    let edited = apply(text, op)?;
    let mut before_root: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let mut after_root: Value = serde_json::from_str(&edited).map_err(|e| e.to_string())?;
    let before = Document::read_edit(&before_root)?;
    let after = Document::read_edit(&after_root)?;
    let rename = if op["op"] == "set" && op["field"] == "id" {
        before
            .tracks
            .iter()
            .flat_map(|t| &t.clips)
            .find(|c| op["clip"] == c.id || op["clip"] == c.at)
            .and_then(|old| {
                after
                    .tracks
                    .iter()
                    .flat_map(|t| &t.clips)
                    .find(|c| c.at == old.at)
                    .map(|new| (old.id.as_str(), new.id.as_str()))
            })
    } else {
        None
    };
    let old = records(&before, &mut before_root)?;
    let mut new: BTreeMap<_, _> = records(&after, &mut after_root)?
        .into_iter()
        .map(|(key, summary, raw)| (key, (summary, raw)))
        .collect();
    let mut changes = Vec::new();
    for (mut key, before, raw) in old {
        if let Some((from, to)) = rename {
            // Clip key: [kind, id]; transition key: [kind, track, from, to]; track: [kind, at].
            let start = if key[0] == "clip" { 1 } else { 2 };
            for id in &mut key[start..] {
                if id == from {
                    *id = to.to_string();
                }
            }
        }
        let kind = key[0].clone();
        if let Some((after, next_raw)) = new.remove(&key) {
            if before != after || raw != next_raw {
                changes.push(serde_json::json!({"kind":kind,"action":"updated","before":before,"after":after}));
            }
        } else {
            changes.push(
                serde_json::json!({"kind":kind,"action":"removed","before":before,"after":null}),
            );
        }
    }
    for (key, (after, _)) in new {
        changes
            .push(serde_json::json!({"kind":key[0],"action":"added","before":null,"after":after}));
    }
    Ok(Edited {
        text: edited,
        changes,
    })
}

/// Identity, public summary, and authored fields (to detect edits that leave timing intact).
fn records(doc: &Document, root: &mut Value) -> Result<Vec<(Vec<String>, Value, Value)>, Error> {
    let mut out = Vec::new();
    for t in &doc.tracks {
        let mut raw = find(root, &t.at)?.clone();
        for list in ["items", "clips"] {
            raw.as_object_mut().unwrap().shift_remove(list);
        }
        let summary = serde_json::json!({ "id": t.id, "path": t.at, "enabled": t.enabled });
        out.push((vec!["track".into(), t.at.clone()], summary, raw));
        for c in &t.clips {
            let mut raw = find(root, &c.at)?.clone();
            // Making a generated id explicit preserves identity; it is not an authored edit.
            raw.as_object_mut().unwrap().shift_remove("id");
            let summary = serde_json::json!({
                "id":c.id,"path":c.at,"start":c.start.seconds(),"end":c.end().seconds(),
                "kind":kind(&c.content),"anchor":c.anchor.as_ref().map(|(id, _)| id),
            });
            out.push((vec!["clip".into(), c.id.clone()], summary, raw));
        }
        for transition in &t.transitions {
            let from = &t.clips[transition.from].id;
            let to = &t.clips[transition.to].id;
            let raw = find(root, &transition.at)?.clone();
            let summary = serde_json::json!({"path":transition.at,"from":from,"to":to,"transition":raw["transition"]});
            out.push((
                vec!["transition".into(), t.at.clone(), from.clone(), to.clone()],
                summary,
                raw,
            ));
        }
    }
    Ok(out)
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
    let mut text = String::new();
    print(&root, 0, 0, true, &mut text);
    text.push('\n');
    Ok(text)
}

/// The longest line a value is kept on.
const WIDTH: usize = 100;

/// `v` as it is written by hand: on its line at `column` where it fits, else one item a line,
/// indented by two spaces. The document itself is always broken.
fn print(v: &Value, indent: usize, column: usize, broken: bool, out: &mut String) {
    let flat = inline(v);
    let nested = matches!(v, Value::Object(o) if !o.is_empty())
        || matches!(v, Value::Array(a) if !a.is_empty());
    if !nested || (!broken && column + flat.chars().count() <= WIDTH) {
        out.push_str(&flat);
        return;
    }
    let pad = " ".repeat(indent + 2);
    let item = |out: &mut String, key: Option<&String>, value: &Value, last: bool| {
        out.push_str(&pad);
        let mut column = pad.len();
        if let Some(key) = key {
            let key = format!("{}: ", Value::from(key.as_str()));
            column += key.chars().count();
            out.push_str(&key);
        }
        print(value, indent + 2, column, false, out);
        out.push_str(if last { "\n" } else { ",\n" });
    };
    match v {
        Value::Object(o) => {
            out.push_str("{\n");
            for (i, (key, value)) in o.iter().enumerate() {
                item(out, Some(key), value, i + 1 == o.len());
            }
            out.push_str(&" ".repeat(indent));
            out.push('}');
        }
        Value::Array(a) => {
            out.push_str("[\n");
            for (i, value) in a.iter().enumerate() {
                item(out, None, value, i + 1 == a.len());
            }
            out.push_str(&" ".repeat(indent));
            out.push(']');
        }
        _ => unreachable!("a scalar fits its line"),
    }
}

/// `v` on one line: `{ "a": 1, "b": [1, 2] }`.
fn inline(v: &Value) -> String {
    match v {
        Value::Object(o) if !o.is_empty() => {
            let fields: Vec<String> = (o.iter())
                .map(|(k, v)| format!("{}: {}", Value::from(k.as_str()), inline(v)))
                .collect();
            format!("{{ {} }}", fields.join(", "))
        }
        Value::Array(a) if !a.is_empty() => {
            let items: Vec<String> = a.iter().map(inline).collect();
            format!("[{}]", items.join(", "))
        }
        v => v.to_string(),
    }
}

/// The path of the track `named`, an id or `tracks.<kind>[n]`. One past the last of its kind
/// is made: a sequence, or with `placed` a track of clips placed by start.
fn track_at(root: &mut Value, doc: &Document, named: &str, placed: bool) -> Result<String, Error> {
    let at = (doc.tracks.iter())
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
        let sequence = ["visual", "audio"].contains(&kind) && !placed;
        list.push(match sequence {
            true => serde_json::json!({ "items": [] }),
            false => serde_json::json!({ "clips": [] }),
        });
    }
    Ok(at.to_string())
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

/// Where each clip is, for an editor to draw: the document's length, rate and markers, and each
/// track's clips with their places, starts and lengths in seconds, kinds, names and anchors,
/// and its transitions with the clips they go between and when they play.
pub fn layout(text: &str) -> Result<Value, Error> {
    let root: Value = serde_json::from_str(text).map_err(|e| format!("not JSON: {e}"))?;
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
                        "text": match &c.content {
                            crate::Content::Caption { text, .. } => Some(text),
                            _ => None,
                        },
                        "media": media(&doc, c),
                    })
                })
                .collect();
            let transitions: Vec<Value> = (t.transitions.iter())
                .map(|x| {
                    let (start, end) = x.span(t);
                    serde_json::json!({
                        "at": x.at,
                        "kind": x.kind,
                        "from": t.clips[x.from].id,
                        "to": t.clips[x.to].id,
                        "start": start.seconds(),
                        "end": end.seconds(),
                    })
                })
                .collect();
            let written = (root.pointer(&format!("/tracks/{}", t.kind.name())))
                .and_then(|list| list.get(split(&t.at).ok()?.1));
            serde_json::json!({
                "id": t.id,
                "kind": t.kind.name(),
                "at": t.at,
                "enabled": t.enabled,
                "sequence": written.is_some_and(|w| w.get("items").is_some()),
                "clips": clips,
                "transitions": transitions,
            })
        })
        .collect();
    Ok(serde_json::json!({
        "duration": doc.duration.seconds(),
        "fps": doc.canvas.fps.as_f64(),
        "width": doc.canvas.width,
        "height": doc.canvas.height,
        "markers": doc.markers,
        "tracks": tracks,
    }))
}

/// The file a clip plays and where in it the clip begins, for an editor's pictures of it.
fn media(doc: &Document, clip: &crate::Clip) -> Value {
    use crate::Content::*;
    let (Image { src } | Video { src } | Audio { src }) = &clip.content else {
        return Value::Null;
    };
    let (trim, rate) = match &clip.source.time {
        crate::SourceTime::Linear { trim, rate } => (trim.seconds(), rate.as_f64()),
        crate::SourceTime::Map(points) => (points.first().map_or(0.0, |p| p.1.seconds()), 1.0),
    };
    let file = doc.resources.get(src).and_then(|r| r.files.first());
    serde_json::json!({ "file": file, "trim": trim, "rate": rate })
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

    #[test]
    fn reports_direct_and_cascaded_changes_without_mistaking_shifted_clips() {
        let mut root: Value = serde_json::from_str(DOC).unwrap();
        root["tracks"]["visual"][0]["items"]
            .as_array_mut()
            .unwrap()
            .insert(
                1,
                json!({"transition":"fade","inOffset":0.1,"outOffset":0.1}),
            );
        let text = root.to_string();
        let deleted = report(&text, &json!({"op":"delete","clip":"b"})).unwrap();
        let removed: Vec<_> = deleted
            .changes
            .iter()
            .filter(|c| c["action"] == "removed")
            .collect();
        assert_eq!(removed.len(), 3, "{}", deleted.summary());
        assert!(removed.iter().any(|c| c["before"]["anchor"] == "b"));
        assert!(
            removed
                .iter()
                .any(|c| c["kind"] == "transition" && c["before"]["to"] == "b")
        );
        let moved = report(&text, &json!({"op":"move","clip":"b","index":0})).unwrap();
        let b = moved
            .changes
            .iter()
            .find(|c| c["after"]["id"] == "b")
            .unwrap();
        assert_eq!(b["before"]["start"], 1.0);
        assert_eq!(b["after"]["start"], 0.0);
        assert_eq!(b["after"]["end"], 2.0);
        assert_eq!(
            moved
                .changes
                .iter()
                .filter(|c| c["action"] == "removed")
                .count(),
            1
        );
        let rename = report(
            &text,
            &json!({"op":"set","clip":"b","field":"id","value":"hero"}),
        )
        .unwrap();
        assert!(rename.changes.iter().all(|c| c["action"] == "updated"));
        assert!(
            rename
                .changes
                .iter()
                .any(|c| c["before"]["id"] == "b" && c["after"]["id"] == "hero")
        );
        let style = report(
            DOC,
            &json!({"op":"set","clip":"a","field":"color","value":"white"}),
        )
        .unwrap();
        assert_eq!(style.changes.len(), 1);
        assert_eq!(style.changes[0]["before"], style.changes[0]["after"]);
        let noop = report(DOC, &json!({"op":"trim","clip":"a","duration":1})).unwrap();
        assert!(noop.changes.is_empty());
    }

    #[test]
    fn deletion_preserves_unnamed_survivors_and_reports_additions() {
        let root = json!({"canvas":{"width":64,"height":36,"fps":30},"tracks":{
            "visual":[{"id":"story","clips":[
                {"kind":"solid","color":"red","start":0,"duration":1},
                {"kind":"solid","color":"blue","start":1,"duration":2}
            ]}],
            "caption":[{"clips":[{"id":"title","text":"Here","start":{"clip":"story.1"},"duration":0.5}]}]
        }});
        let edited = report(&root.to_string(), &json!({"op":"delete","clip":"story.0"})).unwrap();
        let next = crate::parse(&edited.text).unwrap();
        assert_eq!(next.clip("story.1").unwrap().duration.seconds(), 2.0);
        assert_eq!(next.clip("title").unwrap().start.seconds(), 1.0);
        assert_eq!(edited.changes.len(), 2);
        assert_eq!(edited.changes[0]["action"], "removed");
        assert_eq!(edited.changes[0]["before"]["id"], "story.0");
        assert_eq!(edited.changes[1]["action"], "updated");
        assert_eq!(edited.changes[1]["after"]["id"], "story.1");
        let added = report(
            &edited.text,
            &json!({"op":"add","track":"story","clip":{
                "id":"last","kind":"solid","color":"green","start":3,"duration":1
            }}),
        )
        .unwrap();
        assert_eq!(added.changes.len(), 1);
        assert_eq!(added.changes[0]["action"], "added");
        assert_eq!(added.changes[0]["after"]["start"], 3.0);
        assert_eq!(added.changes[0]["after"]["end"], 4.0);
    }

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
    fn clips_change_tracks_and_tracks_are_switched_off() {
        // From a sequence to placed clips: where it was, its transition gone.
        let text = apply(
            LINKED,
            &json!({"op":"move","clip":"b","track":"tracks.visual[1]","start":1}),
        );
        let doc = crate::parse(&text.unwrap()).unwrap();
        assert_eq!(doc.tracks.len(), 5);
        assert_eq!(doc.clip("b").unwrap().at, "tracks.visual[1].clips[0]");
        assert_eq!(doc.clip("b").unwrap().start.seconds(), 1.0);
        assert!(doc.tracks[0].transitions.is_empty());
        assert_eq!(doc.clip("title").unwrap().start.seconds(), 1.1);
        // Between placed tracks, at a time; an unnamed clip others follow keeps its id.
        let doc = edited(json!({"op":"move","clip":"c","track":"tracks.visual[2]","start":2}));
        assert_eq!(doc["tracks"]["visual"][1]["clips"], json!([]));
        assert_eq!(doc["tracks"]["visual"][2]["clips"][1]["start"], json!(2));
        // Into a sequence: at its end, without a start.
        let doc = edited(json!({"op":"move","clip":"c","track":"tracks.visual[0]"}));
        assert_eq!(doc["tracks"]["visual"][0]["items"][2]["id"], "c");
        assert!(
            doc["tracks"]["visual"][0]["items"][2]
                .get("start")
                .is_none()
        );
        let other = json!({"op":"move","clip":"title","track":"tracks.visual[0]"});
        assert!(apply(LINKED, &other).unwrap_err().contains("caption"));

        let op = json!({"op":"set","track":"story","field":"enabled","value":false});
        let off = report(LINKED, &op).unwrap();
        assert!(!crate::parse(&off.text).unwrap().tracks[0].enabled);
        assert_eq!(off.summary(), "updated track story at tracks.visual[0]");
        assert_eq!(layout(&off.text).unwrap()["tracks"][0]["enabled"], false);
    }

    #[test]
    fn the_layout_has_transitions_and_markers_and_the_text_keeps_its_shape() {
        let laid = layout(LINKED).unwrap();
        let fade = &laid["tracks"][0]["transitions"][0];
        assert_eq!(fade["kind"], "fade");
        assert_eq!((&fade["from"], &fade["to"]), (&json!("a"), &json!("b")));
        assert_eq!((&fade["start"], &fade["end"]), (&json!(0.9), &json!(1.1)));
        assert_eq!(laid["tracks"][0]["sequence"], true);
        let text = "{\n  \"name\": \"Cut\",\n  \"canvas\": { \"width\": 64, \"height\": 36, \"fps\": 30 },\n  \"markers\": [{ \"at\": 1, \"name\": \"beat\" }],\n  \"tracks\": {\n    \"visual\": [\n      {\n        \"clips\": [\n          { \"id\": \"a\", \"kind\": \"solid\", \"color\": \"red\", \"start\": 0, \"duration\": 1 },\n          { \"id\": \"b\", \"kind\": \"solid\", \"color\": \"blue\", \"start\": 1, \"duration\": 1 }\n        ]\n      },\n      { \"clips\": [] }\n    ]\n  }\n}\n";
        assert_eq!(layout(text).unwrap()["markers"][0]["name"], "beat");
        // An edit that changes nothing writes the text as it was: order and lines kept.
        let same = apply(text, &json!({"op":"move","clip":"b","start":1})).unwrap();
        assert_eq!(same, text);
        let moved = apply(text, &json!({"op":"move","clip":"b","start":2})).unwrap();
        assert_eq!(moved, text.replace("\"start\": 1,", "\"start\": 2,"));
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

    #[test]
    fn renaming_a_track_preserves_generated_ids_and_anchors() {
        let text = json!({"canvas":{"width":64,"height":36,"fps":30},"tracks":{
            "visual":[{"clips":[{"kind":"solid","color":"red","start":1,"duration":2}]}],
            "caption":[{"clips":[{"id":"title","text":"Hi","start":{"clip":"visual0.0"},"duration":1}]}]
        }}).to_string();
        let renamed = apply(
            &text,
            &json!({"op":"set","track":"visual0","field":"id","value":"story"}),
        )
        .unwrap();
        let doc = crate::parse(&renamed).unwrap();
        assert_eq!(doc.tracks[0].id, "story");
        assert!(doc.clip("visual0.0").unwrap().named);
        assert_eq!(doc.clip("title").unwrap().start.seconds(), 1.0);
        let reset = apply(
            &renamed,
            &json!({"op":"set","track":"story","field":"id","value":null}),
        )
        .unwrap();
        let doc = crate::parse(&reset).unwrap();
        assert_eq!(doc.tracks[0].id, "visual0");
        assert_eq!(doc.clip("title").unwrap().start.seconds(), 1.0);
    }

    #[test]
    fn moving_an_unreferenced_clip_between_tracks_keeps_its_id() {
        let text = json!({"canvas":{"width":64,"height":36,"fps":30},"tracks":{
            "visual":[{"clips":[{"kind":"solid","color":"red","start":0,"duration":2}]},{"clips":[]}]
        }}).to_string();
        let moved = apply(
            &text,
            &json!({"op":"move","clip":"visual0.0","track":"tracks.visual[1]","start":1}),
        )
        .unwrap();
        let doc = crate::parse(&moved).unwrap();
        let clip = doc.clip("visual0.0").unwrap();
        assert_eq!(clip.at, "tracks.visual[1].clips[0]");
        assert_eq!(clip.start.seconds(), 1.0);
        assert!(clip.named);
    }
}
