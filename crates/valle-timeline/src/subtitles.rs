//! SubRip (`.srt`) and WebVTT (`.vtt`) subtitles: read into cues and put on a caption track, or
//! a caption track's clips written out as them.
//!
//! Reading takes the cues' times and text. Styling is dropped: tags such as `<i>` and `<c.red>`,
//! VTT cue settings and SRT's `{\an8}`. NOTE, STYLE and REGION blocks are skipped.

use serde_json::{Value, json};

use crate::{Content, Document, Error, Kind};

/// A subtitle: from `start` to `end` seconds, its lines joined by `\n`.
#[derive(Clone, Debug, PartialEq)]
pub struct Cue {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Srt,
    Vtt,
}

impl Format {
    /// The format a file's extension names.
    pub fn of(path: &str) -> Option<Format> {
        let ext = path.rsplit_once('.')?.1.to_ascii_lowercase();
        match ext.as_str() {
            "srt" => Some(Format::Srt),
            "vtt" => Some(Format::Vtt),
            _ => None,
        }
    }
}

/// The cues of an SRT or VTT file's text, in the order written.
pub fn read(text: &str) -> Result<Vec<Cue>, Error> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let vtt = text.starts_with("WEBVTT");
    let mut cues = Vec::new();
    // Each block with the line it starts on, from 1.
    let mut block: Vec<&str> = Vec::new();
    let mut first = 0;
    let lines: Vec<&str> = text.split('\n').collect();
    for (n, line) in lines.iter().chain(std::iter::once(&"")).enumerate() {
        if !line.trim().is_empty() {
            if block.is_empty() {
                first = n + 1;
            }
            block.push(line);
            continue;
        }
        if block.is_empty() {
            continue;
        }
        let head = block[0].trim_start();
        let skipped = vtt
            && (first == 1
                || ["NOTE", "STYLE", "REGION"].iter().any(|k| {
                    head == *k
                        || head.starts_with(&format!("{k} "))
                        || head.starts_with(&format!("{k}\t"))
                }));
        if !skipped {
            cues.extend(cue(&block, first, vtt)?);
        }
        block.clear();
    }
    Ok(cues)
}

/// The cue of a block starting on line `first`: an optional id or number, the timing line, then
/// the text. None when it has no text.
fn cue(block: &[&str], first: usize, vtt: bool) -> Result<Option<Cue>, Error> {
    let Some(i) = block.iter().position(|l| l.contains("-->")) else {
        return Err(format!(
            "line {first}: a cue's timing, such as 00:00:01,000 --> 00:00:02,500, is not there"
        ));
    };
    let at = first + i;
    let bad = || {
        format!(
            "line {at}: {:?} is not a timing such as 00:00:01,000 --> 00:00:02,500",
            block[i].trim()
        )
    };
    let (a, b) = block[i].split_once("-->").ok_or_else(bad)?;
    // VTT cue settings and SRT coordinates follow the end time.
    let b = b.split_whitespace().next().ok_or_else(bad)?;
    let (start, end) = (time(a.trim()).ok_or_else(bad)?, time(b).ok_or_else(bad)?);
    if end < start {
        return Err(format!("line {at}: the cue ends before it starts"));
    }
    let lines: Vec<String> = (block[i + 1..].iter())
        .map(|l| clean(l, vtt))
        .filter(|l| !l.is_empty())
        .collect();
    if lines.is_empty() {
        return Ok(None);
    }
    Ok(Some(Cue {
        start,
        end,
        text: lines.join("\n"),
    }))
}

/// `[hh:]mm:ss,mmm` or `[hh:]mm:ss.mmm` in seconds.
fn time(text: &str) -> Option<f64> {
    let (clock, fraction) = text.split_once([',', '.'])?;
    let parts: Vec<&str> = clock.split(':').collect();
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !(2..=3).contains(&parts.len()) || !parts.iter().all(|p| digits(p)) || !digits(fraction) {
        return None;
    }
    let whole = parts
        .iter()
        .try_fold(0.0, |t, p| Some(t * 60.0 + p.parse::<f64>().ok()?))?;
    let fraction: f64 = format!("0.{fraction}").parse().ok()?;
    let (minutes, seconds) = (parts[parts.len() - 2], parts[parts.len() - 1]);
    if minutes.parse::<u32>().ok()? > 59 || seconds.parse::<u32>().ok()? > 59 {
        return None;
    }
    Some(whole + fraction)
}

/// A text line without its tags, SRT's `{\…}` overrides and VTT's character references.
fn clean(line: &str, vtt: bool) -> String {
    let mut out = String::new();
    let mut rest = line;
    while let Some(c) = rest.chars().next() {
        let close = match c {
            '<' => Some('>'),
            '{' if rest.starts_with("{\\") => Some('}'),
            _ => None,
        };
        if let Some(end) = close.and_then(|k| rest.find(k)) {
            rest = &rest[end + 1..];
            continue;
        }
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    if vtt {
        for (from, to) in [
            ("&lt;", "<"),
            ("&gt;", ">"),
            ("&nbsp;", "\u{a0}"),
            ("&lrm;", "\u{200e}"),
            ("&rlm;", "\u{200f}"),
            ("&amp;", "&"),
        ] {
            out = out.replace(from, to);
        }
    }
    out.trim().to_string()
}

/// The cues as a file's text.
pub fn write(cues: &[Cue], format: Format) -> String {
    let stamp = |t: f64| {
        let ms = (t.max(0.0) * 1000.0).round() as u64;
        let sep = if format == Format::Srt { ',' } else { '.' };
        format!(
            "{:02}:{:02}:{:02}{sep}{:03}",
            ms / 3_600_000,
            ms / 60_000 % 60,
            ms / 1000 % 60,
            ms % 1000
        )
    };
    let mut out = String::new();
    if format == Format::Vtt {
        out.push_str("WEBVTT\n\n");
    }
    for (n, c) in cues.iter().enumerate() {
        if format == Format::Srt {
            out.push_str(&format!("{}\n", n + 1));
        }
        out.push_str(&format!("{} --> {}\n", stamp(c.start), stamp(c.end)));
        // A blank line ends a cue.
        for line in c.text.lines().map(str::trim).filter(|l| !l.is_empty()) {
            match format {
                Format::Srt => out.push_str(line),
                Format::Vtt => out.push_str(
                    &line
                        .replace('&', "&amp;")
                        .replace('<', "&lt;")
                        .replace('>', "&gt;"),
                ),
            }
            out.push('\n');
        }
        out.push('\n');
    }
    out
}

/// What `import` did: the track the captions are on, how many it put there, and how many it
/// shortened so as not to overlap the next, or left out as outside the clip or under a frame.
#[derive(Debug)]
pub struct Imported {
    pub track: String,
    pub captions: usize,
    pub shortened: usize,
    pub left_out: usize,
}

/// The document `text` with `cues` as caption clips: on the caption track `track` (an id or
/// path), in place of its clips, or else on a new caption track. Cue times are the timeline's,
/// or with `clip` times in the file that clip plays, each caption anchored to it.
pub fn import(
    text: &str,
    cues: &[Cue],
    track: Option<&str>,
    clip: Option<&str>,
) -> Result<(String, Imported), Error> {
    let mut root: Value = serde_json::from_str(text).map_err(|e| format!("not JSON: {e}"))?;
    let doc = Document::read_edit(&root)?;
    let fps = doc.canvas.fps.as_f64();
    let snap = |s: f64| (s * fps).round() / fps;
    // Where a cue's time is on the clip it hangs on, and how long that clip is.
    let on = match clip {
        None => None,
        Some(named) => {
            let c = (doc.tracks.iter().flat_map(|t| &t.clips))
                .find(|c| c.id == named || c.at == named)
                .ok_or_else(|| format!("no clip {named}"))?;
            let crate::SourceTime::Linear { trim, rate } = &c.source.time else {
                return Err(format!(
                    "{} maps its time: place its captions by the map, without --clip",
                    c.at
                ));
            };
            Some((
                c.id.clone(),
                trim.seconds(),
                rate.as_f64(),
                c.duration.seconds(),
            ))
        }
    };
    let mut done = Imported {
        track: String::new(),
        captions: 0,
        shortened: 0,
        left_out: 0,
    };
    let mut spans: Vec<(f64, f64, &str)> = Vec::new();
    for c in cues {
        let (mut a, mut b) = (c.start, c.end);
        if let Some((_, trim, rate, length)) = &on {
            (a, b) = (
                ((a - trim) / rate).max(0.0),
                ((b - trim) / rate).min(*length),
            );
        }
        let (a, b) = (snap(a), snap(b));
        if b - a < 0.5 / fps {
            done.left_out += 1;
            continue;
        }
        spans.push((a, b, &c.text));
    }
    spans.sort_by(|x, y| x.0.total_cmp(&y.0));
    let mut clips = Vec::new();
    for i in 0..spans.len() {
        let (a, mut b, text) = spans[i];
        if let Some(next) = spans.get(i + 1)
            && next.0 < b
        {
            b = next.0;
            done.shortened += 1;
        }
        if b - a < 0.5 / fps {
            done.left_out += 1;
            continue;
        }
        let start = match &on {
            Some((id, ..)) => json!({ "clip": id, "offset": crate::edit::seconds(a) }),
            None => crate::edit::seconds(a),
        };
        clips.push(json!({
            "start": start,
            "duration": crate::edit::seconds(b - a),
            "text": text,
        }));
    }
    done.captions = clips.len();
    match track {
        Some(named) => {
            let t = (doc.tracks.iter())
                .find(|t| t.id == named || t.at == named)
                .ok_or_else(|| format!("no track {named}"))?;
            if t.kind != Kind::Caption {
                return Err(format!(
                    "{} is a {} track, not a caption track",
                    t.at,
                    t.kind.name()
                ));
            }
            crate::edit::find(&mut root, &t.at)?["clips"] = Value::Array(clips);
            done.track = t.at.clone();
        }
        None => {
            let list = root["tracks"]
                .as_object_mut()
                .ok_or("tracks: an object")?
                .entry("caption")
                .or_insert_with(|| Value::Array(Vec::new()))
                .as_array_mut()
                .ok_or("tracks.caption: a list")?;
            done.track = format!("tracks.caption[{}]", list.len());
            list.push(json!({ "clips": clips }));
        }
    }
    Ok((crate::edit::checked(root)?, done))
}

/// The caption track `track` (an id or path), or the first, as cues on the timeline's time:
/// its enabled clips in order, and its path.
pub fn export(doc: &Document, track: Option<&str>) -> Result<(String, Vec<Cue>), Error> {
    let t = match track {
        Some(named) => {
            let t = (doc.tracks.iter())
                .find(|t| t.id == named || t.at == named)
                .ok_or_else(|| format!("no track {named}"))?;
            if t.kind != Kind::Caption {
                return Err(format!(
                    "{} is a {} track, not a caption track",
                    t.at,
                    t.kind.name()
                ));
            }
            t
        }
        None => (doc.tracks.iter())
            .find(|t| t.kind == Kind::Caption)
            .ok_or("the timeline has no caption track")?,
    };
    let cues = (t.clips.iter())
        .filter(|c| c.enabled)
        .filter_map(|c| match &c.content {
            Content::Caption { text, .. } => Some(Cue {
                start: c.start.seconds(),
                end: c.end().seconds(),
                text: text.clone(),
            }),
            _ => None,
        })
        .collect();
    Ok((t.at.clone(), cues))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRT: &str = "\u{feff}1\r\n00:00:01,000 --> 00:00:02,500\r\n<i>Hello</i> there\r\n\
        {\\an8}second line\r\n\r\n2\r\n00:00:02,400 --> 00:00:04,000 X1:10 X2:20\r\nAgain\r\n\r\n\
        3\r\n00:01:00,000 --> 00:01:00,010\r\nShort\r\n";

    const VTT: &str = "WEBVTT - made by hand\n\nNOTE a comment\nover lines\n\nSTYLE\n::cue { color: red }\n\n\
        intro\n00:01.000 --> 00:02.500 align:start line:10%\n<v Ann>Fish &amp; <c.big>chips</c>\n\n\
        01:00:00.250 --> 01:00:01.000\n<00:00:00.500>word &lt;3\n";

    #[test]
    fn srt_and_vtt_read_into_cues_without_their_styling() {
        let cues = read(SRT).unwrap();
        assert_eq!(cues.len(), 3);
        assert_eq!(
            cues[0],
            Cue {
                start: 1.0,
                end: 2.5,
                text: "Hello there\nsecond line".into()
            }
        );
        assert_eq!((cues[1].start, cues[1].end), (2.4, 4.0));
        let cues = read(VTT).unwrap();
        assert_eq!(cues.len(), 2);
        assert_eq!(
            cues[0],
            Cue {
                start: 1.0,
                end: 2.5,
                text: "Fish & chips".into()
            }
        );
        assert_eq!((cues[1].start, cues[1].text.as_str()), (3600.25, "word <3"));
        for bad in [
            "1\nsoon --> later\nHi\n",
            "1\nHi\n",
            "1\n00:00:02,000 --> 00:00:01,000\nHi\n",
        ] {
            let e = read(bad).unwrap_err();
            assert!(e.starts_with("line "), "{e}");
        }
        assert!(read("00:00:01,000 --> 00:01:61,000\nHi\n").is_err());
    }

    #[test]
    fn cues_write_back_as_they_read() {
        let cues = vec![
            Cue {
                start: 0.5,
                end: 3723.25,
                text: "One\n\ntwo".into(),
            },
            Cue {
                start: 4000.0,
                end: 4001.0,
                text: "a < b & c".into(),
            },
        ];
        let srt = write(&cues, Format::Srt);
        assert!(srt.starts_with("1\n00:00:00,500 --> 01:02:03,250\nOne\ntwo\n\n2\n"));
        let vtt = write(&cues, Format::Vtt);
        assert!(vtt.contains("a &lt; b &amp; c"));
        for text in [srt, vtt] {
            let back = read(&text).unwrap();
            assert_eq!(back[0].text, "One\ntwo");
            assert_eq!(back[1], cues[1]);
        }
        assert_eq!(Format::of("a/B.SRT"), Some(Format::Srt));
        assert_eq!(Format::of("x.ass"), None);
    }

    const DOC: &str = r#"{
      "canvas": {"width":64,"height":36,"fps":25},
      "resources": {"talk":"talk.mp4"},
      "tracks": {
        "visual": [{"id":"story","items":[
          {"id":"talk","kind":"video","src":"talk","duration":10,"trimStart":20,"rate":2}
        ]}],
        "caption": [{"id":"subs","style":{"fontSize":40},"clips":[{"start":0,"duration":1,"text":"old"}]}]
      }
    }"#;

    #[test]
    fn imports_go_on_a_new_track_or_replace_one_and_can_follow_a_clip() {
        let cues = read(SRT).unwrap();
        let (text, done) = import(DOC, &cues, None, None).unwrap();
        assert_eq!(done.track, "tracks.caption[1]");
        // The second starts before the first ends; the last is under a frame.
        assert_eq!((done.captions, done.shortened, done.left_out), (2, 1, 1));
        let doc = crate::parse(&text).unwrap();
        let first = &doc
            .tracks
            .iter()
            .find(|t| t.at == "tracks.caption[1]")
            .unwrap()
            .clips[0];
        assert_eq!((first.start.seconds(), first.end().seconds()), (1.0, 2.4));
        let (_, out) = export(&doc, Some("tracks.caption[1]")).unwrap();
        assert_eq!(out[0].text, "Hello there\nsecond line");
        assert_eq!(out[1].end, 4.0);
        // Over a track's clips, keeping its style; times in the clip's file: 20 s in, twice as
        // fast.
        let cues = vec![
            Cue {
                start: 22.0,
                end: 26.0,
                text: "in".into(),
            },
            Cue {
                start: 38.0,
                end: 44.0,
                text: "past the end".into(),
            },
            Cue {
                start: 50.0,
                end: 51.0,
                text: "after".into(),
            },
        ];
        let (text, done) = import(DOC, &cues, Some("subs"), Some("talk")).unwrap();
        assert_eq!((done.captions, done.left_out), (2, 1));
        let root: Value = serde_json::from_str(&text).unwrap();
        let subs = &root["tracks"]["caption"][0];
        assert_eq!(subs["style"], json!({"fontSize":40}));
        assert_eq!(subs["clips"][0]["start"], json!({"clip":"talk","offset":1}));
        assert_eq!(subs["clips"][0]["duration"], json!(2));
        assert_eq!(subs["clips"][1]["duration"], json!(1));
        assert!(
            import(DOC, &cues, Some("story"), None)
                .unwrap_err()
                .contains("not a caption")
        );
        assert!(import(DOC, &cues, None, Some("nothing")).is_err());
    }

    #[test]
    fn exports_take_the_first_caption_track_and_skip_disabled_clips() {
        let mut root: Value = serde_json::from_str(DOC).unwrap();
        root["tracks"]["caption"][0]["clips"]
            .as_array_mut()
            .unwrap()
            .push(json!({"start":2,"duration":1,"text":"off","enabled":false}));
        let doc = Document::read_edit(&root).unwrap();
        let (at, cues) = export(&doc, None).unwrap();
        assert_eq!(at, "tracks.caption[0]");
        assert_eq!(
            cues,
            vec![Cue {
                start: 0.0,
                end: 1.0,
                text: "old".into()
            }]
        );
        assert!(export(&doc, Some("story")).is_err());
        root["tracks"]["caption"] = json!([]);
        assert!(export(&Document::read_edit(&root).unwrap(), None).is_err());
    }
}
