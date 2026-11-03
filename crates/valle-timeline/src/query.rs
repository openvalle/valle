//! `valle timeline query`: the clips a filter picks, each with its times as program and source
//! timecodes, the line it is written on, its anchor and the clips anchored to it, so a tool reads
//! a few clips of a long document instead of all of it.

use serde_json::{Value, json};

use crate::time::{Rate, Tick};
use crate::{Clip, Content, Document, Error, SourceTime, Track, timecode};

/// What a query keeps: every condition given holds.
#[derive(Default)]
pub struct Filter {
    pub clip: Option<String>,
    pub track: Option<String>,
    /// Place among its track's clips by start, from 1.
    pub index: Option<usize>,
    /// Seconds from the start, or a program timecode.
    pub at: Option<String>,
    /// A marker's name: clips that carry it, or that the timeline's marker of that name is on.
    pub marker: Option<String>,
    /// Metadata `key=value`: a dotted path from the top, or a key at any depth.
    pub meta: Vec<(String, String)>,
}

/// The clips of `doc`, read from `text`, that `filter` keeps.
pub fn query(text: &str, doc: &Document, filter: &Filter) -> Result<Value, Error> {
    let root: Value = serde_json::from_str(text).map_err(|e| format!("not JSON: {e}"))?;
    let fps = doc.canvas.fps;
    let first = match &doc.start_timecode {
        Some(tc) => timecode::parse(tc, fps.as_f64()).map_err(|e| format!("startTimecode: {e}"))?,
        None => 0,
    };
    let program = |t: Tick| timecode::format(first + fps.frame_at(t), fps.as_f64());
    let at = match &filter.at {
        None => None,
        Some(s) => Some(match Tick::parse_seconds(s) {
            Ok(t) => t,
            Err(_) => {
                let frame = timecode::parse(s, fps.as_f64()).map_err(|e| format!("--at: {e}"))?;
                fps.frame_start(frame - first)?
            }
        }),
    };
    // The timeline's own markers of that name, as the spans they cover.
    let marked: Vec<(Tick, Tick)> = match &filter.marker {
        None => Vec::new(),
        Some(name) => (doc.markers.iter())
            .filter(|m| m["name"].as_str() == Some(name))
            .map(|m| {
                let t = Tick::parse_seconds(&m["at"].to_string()).unwrap_or_default();
                let d = Tick::parse_seconds(&m["duration"].to_string()).unwrap_or_default();
                (t, t + d)
            })
            .collect(),
    };
    let mut out = Vec::new();
    for track in &doc.tracks {
        for (i, clip) in track.clips.iter().enumerate() {
            let on = |t: Tick| t >= clip.start && t < clip.end();
            let keep = filter.clip.as_ref().is_none_or(|id| *id == clip.id)
                && filter.track.as_ref().is_none_or(|t| *t == track.id)
                && filter.index.is_none_or(|n| n == i + 1)
                && at.is_none_or(on)
                && filter.marker.as_ref().is_none_or(|name| {
                    (clip.markers.iter()).any(|m| m["name"].as_str() == Some(name))
                        || (marked.iter())
                            .any(|&(a, b)| on(a) || (a < clip.start && b > clip.start))
                })
                && (filter.meta.iter())
                    .all(|(k, v)| (clip.metadata.as_ref()).is_some_and(|m| meta_has(m, k, v)));
            if keep {
                let mut info = found(doc, track, i, clip, at, &program)?;
                if let Some(line) = line(text, &clip.at) {
                    info["line"] = json!(line);
                }
                if let Some(written) = root.pointer(&pointer(&clip.at)) {
                    info["clip"] = written.clone();
                }
                out.push(info);
            }
        }
    }
    Ok(json!({ "count": out.len(), "clips": out }))
}

/// One clip's times and anchors.
fn found(
    doc: &Document,
    track: &Track,
    i: usize,
    clip: &Clip,
    at: Option<Tick>,
    program: &dyn Fn(Tick) -> String,
) -> Result<Value, Error> {
    let mut info = json!({
        "id": clip.id,
        "track": track.id,
        "index": i + 1,
        "start": clip.start.seconds_text().parse::<f64>().unwrap_or_default(),
        "end": clip.end().seconds_text().parse::<f64>().unwrap_or_default(),
        "program": [program(clip.start), program(clip.end())],
    });
    if let Some((to, offset)) = &clip.anchor {
        info["anchor"] = json!({ "clip": to, "offset": offset.seconds() });
    }
    let anchored: Vec<&str> = (doc.tracks.iter())
        .flat_map(|t| &t.clips)
        .filter(|c| c.anchor.as_ref().is_some_and(|(to, _)| *to == clip.id))
        .map(|c| c.id.as_str())
        .collect();
    if !anchored.is_empty() {
        info["anchored"] = json!(anchored);
    }
    if let Some(label) = source_label(doc, clip)? {
        info["source"] = json!([label(Tick::ZERO), label(clip.duration)]);
        if let Some(t) = at {
            info["at"] = json!({ "program": program(t), "source": label(t - clip.start) });
        }
    } else if let Some(t) = at {
        info["at"] = json!({ "program": program(t) });
    }
    Ok(info)
}

type Label = Box<dyn Fn(Tick) -> String>;

/// A media clip's source timecode at a time of the clip: from its resource's `timecode` (else
/// 00:00:00:00), at the resource's `fps` (else the canvas's).
fn source_label(doc: &Document, clip: &Clip) -> Result<Option<Label>, Error> {
    let (Content::Video { src } | Content::Audio { src } | Content::Timeline { src }) =
        &clip.content
    else {
        return Ok(None);
    };
    let resource = doc.resources.get(src);
    let fps = match resource.and_then(|r| r.fps) {
        Some(f) => Rate::from_f64(f)?,
        None => doc.canvas.fps,
    };
    let first = match resource.and_then(|r| r.timecode.as_deref()) {
        Some(tc) => timecode::parse(tc, fps.as_f64()).map_err(|e| format!("{src}: {e}"))?,
        None => 0,
    };
    let time = clip.source.time.clone();
    Ok(Some(Box::new(move |t| {
        timecode::format(first + fps.frame_at(source_at(&time, t)), fps.as_f64())
    })))
}

/// The source time at clip time `t`: `trimStart + t × rate`, or straight between the time map's
/// points and held past its ends.
pub fn source_at(time: &SourceTime, t: Tick) -> Tick {
    match time {
        SourceTime::Linear { trim, rate } => *trim + rate.apply(t).unwrap_or(t),
        SourceTime::Map(points) => {
            let (first, last) = (points[0], points[points.len() - 1]);
            if t <= first.0 {
                return first.1;
            }
            if t >= last.0 {
                return last.1;
            }
            let k = points.partition_point(|p| p.0 <= t);
            let ((x0, y0), (x1, y1)) = (points[k - 1], points[k]);
            let (dx, dy, u) = (
                i128::from((x1 - x0).0),
                i128::from((y1 - y0).0),
                i128::from((t - x0).0),
            );
            y0 + Tick((dy * u).div_euclid(dx) as i64)
        }
    }
}

/// Whether `meta` holds `key` = `value`: at a dotted path from the top, or as a key of that name
/// at any depth, its value a string, number or boolean written so.
fn meta_has(meta: &Value, key: &str, value: &str) -> bool {
    let same = |v: &Value| match v {
        Value::String(s) => s == value,
        Value::Number(_) | Value::Bool(_) => {
            serde_json::from_str::<Value>(value).ok().as_ref() == Some(v)
        }
        _ => false,
    };
    if key.contains('.') {
        return key
            .split('.')
            .try_fold(meta, |v, part| match v {
                Value::Array(a) => part.parse::<usize>().ok().and_then(|i| a.get(i)),
                _ => v.get(part),
            })
            .is_some_and(same);
    }
    match meta {
        Value::Object(o) => {
            (o.iter()).any(|(k, v)| (k == key && same(v)) || meta_has(v, key, value))
        }
        Value::Array(a) => a.iter().any(|v| meta_has(v, key, value)),
        _ => false,
    }
}

/// `tracks.visual[0].items[2]` as the JSON pointer `/tracks/visual/0/items/2`.
fn pointer(at: &str) -> String {
    at.split(['.', '['])
        .map(|p| format!("/{}", p.trim_end_matches(']')))
        .collect()
}

/// The line (from 1) the value at `at` starts on in `text`, a document already read as JSON.
fn line(text: &str, at: &str) -> Option<usize> {
    let mut s = Scan {
        b: text.as_bytes(),
        i: 0,
    };
    for part in pointer(at).split('/').skip(1) {
        s.space();
        match s.b.get(s.i)? {
            b'{' => {
                s.i += 1;
                loop {
                    s.space();
                    let key = s.string()?;
                    s.space();
                    s.i += 1; // :
                    s.space();
                    if key == part {
                        break;
                    }
                    s.skip()?;
                    s.space();
                    if s.b.get(s.i)? != &b',' {
                        return None;
                    }
                    s.i += 1;
                }
            }
            b'[' => {
                s.i += 1;
                for _ in 0..part.parse::<usize>().ok()? {
                    s.space();
                    s.skip()?;
                    s.space();
                    if s.b.get(s.i)? != &b',' {
                        return None;
                    }
                    s.i += 1;
                }
                s.space();
            }
            _ => return None,
        }
    }
    Some(1 + s.b[..s.i].iter().filter(|&&c| c == b'\n').count())
}

struct Scan<'a> {
    b: &'a [u8],
    i: usize,
}

impl Scan<'_> {
    fn space(&mut self) {
        while self.b.get(self.i).is_some_and(u8::is_ascii_whitespace) {
            self.i += 1;
        }
    }

    /// A string's raw text (escapes kept), past its closing quote.
    fn string(&mut self) -> Option<String> {
        if self.b.get(self.i)? != &b'"' {
            return None;
        }
        let start = self.i + 1;
        let mut j = start;
        while *self.b.get(j)? != b'"' {
            j += if self.b[j] == b'\\' { 2 } else { 1 };
        }
        self.i = j + 1;
        Some(String::from_utf8_lossy(&self.b[start..j]).into_owned())
    }

    /// Past one value.
    fn skip(&mut self) -> Option<()> {
        match self.b.get(self.i)? {
            b'"' => {
                self.string()?;
            }
            b'{' | b'[' => {
                let mut depth = 0usize;
                loop {
                    match self.b.get(self.i)? {
                        b'"' => {
                            self.string()?;
                            continue;
                        }
                        b'{' | b'[' => depth += 1,
                        b'}' | b']' => {
                            depth -= 1;
                            if depth == 0 {
                                self.i += 1;
                                return Some(());
                            }
                        }
                        _ => {}
                    }
                    self.i += 1;
                }
            }
            _ => {
                while !matches!(self.b.get(self.i), None | Some(b',' | b'}' | b']'))
                    && !self.b[self.i].is_ascii_whitespace()
                {
                    self.i += 1;
                }
            }
        }
        Some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = r#"{
  "canvas": { "width": 64, "height": 36, "fps": 24 },
  "startTimecode": "01:00:00:00",
  "markers": [{ "at": 2.5, "name": "drop" }],
  "resources": {
    "a": "a.mov",
    "b": { "path": "b.mov", "timecode": "10:00:00:00", "fps": 25 }
  },
  "tracks": {
    "visual": [{ "id": "V1", "items": [
      { "id": "c1", "kind": "video", "src": "a", "duration": 2, "metadata": { "scene": { "take": 3 } } },
      { "id": "c2", "kind": "video", "src": "b", "duration": 2, "trimStart": 4,
        "markers": [{ "at": 0.5, "name": "beat" }] }
    ] }],
    "caption": [{ "id": "C", "clips": [
      { "id": "t", "start": { "clip": "c2", "offset": 1 }, "duration": 1, "text": "hi \"there\"" }
    ] }]
  }
}"#;

    fn ask(filter: Filter) -> Value {
        let doc = crate::parse(TEXT).unwrap();
        query(TEXT, &doc, &filter).unwrap()
    }

    #[test]
    fn a_clip_comes_with_its_timecodes_line_and_anchors() {
        let r = ask(Filter {
            clip: Some("c2".into()),
            ..Filter::default()
        });
        assert_eq!(r["count"], 1);
        let c = &r["clips"][0];
        assert_eq!(c["track"], "V1");
        assert_eq!(c["index"], 2);
        assert_eq!(c["line"], 12);
        assert_eq!(c["program"], json!(["01:00:02:00", "01:00:04:00"]));
        assert_eq!(c["source"], json!(["10:00:04:00", "10:00:06:00"]));
        assert_eq!(c["anchored"], json!(["t"]));
        assert_eq!(c["clip"]["trimStart"], 4);
        let t = &ask(Filter {
            clip: Some("t".into()),
            ..Filter::default()
        })["clips"][0];
        assert_eq!(t["line"], 16);
        assert_eq!(t["anchor"], json!({ "clip": "c2", "offset": 1.0 }));
    }

    #[test]
    fn filters_pick_by_time_marker_and_metadata() {
        let ids = |r: Value| {
            (r["clips"].as_array().unwrap().iter())
                .map(|c| c["id"].as_str().unwrap().to_string())
                .collect::<Vec<_>>()
        };
        let at = ask(Filter {
            at: Some("01:00:03:12".into()),
            ..Filter::default()
        });
        assert_eq!(ids(at.clone()), ["c2", "t"]);
        assert_eq!(
            at["clips"][0]["at"],
            json!({ "program": "01:00:03:12", "source": "10:00:05:12" })
        );
        let marker = |name: &str| Filter {
            marker: Some(name.into()),
            ..Filter::default()
        };
        assert_eq!(ids(ask(marker("beat"))), ["c2"]);
        assert_eq!(ids(ask(marker("drop"))), ["c2"]);
        let meta = |k: &str, v: &str| Filter {
            meta: vec![(k.into(), v.into())],
            ..Filter::default()
        };
        assert_eq!(ids(ask(meta("take", "3"))), ["c1"]);
        assert_eq!(ids(ask(meta("scene.take", "3"))), ["c1"]);
        assert!(ids(ask(meta("take", "4"))).is_empty());
        let second = Filter {
            track: Some("V1".into()),
            index: Some(1),
            ..Filter::default()
        };
        assert_eq!(ids(ask(second)), ["c1"]);
    }

    #[test]
    fn a_time_map_holds_past_its_ends() {
        let s = |x: &str| Tick::parse_seconds(x).unwrap();
        let map = SourceTime::Map(vec![(s("0"), s("1")), (s("2"), s("5"))]);
        assert_eq!(source_at(&map, s("1")), s("3"));
        assert_eq!(source_at(&map, s("3")), s("5"));
    }
}
