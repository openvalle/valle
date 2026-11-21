//! Review notes: what someone watching an edit asks to change, in `reviews/notes.json`, with the
//! picture they saw in `reviews/crops/`. Notes are not part of the edit, as a timeline's `markers`
//! are, so writing one never touches a source. Each message of a note keeps the snapshot it was
//! written against, saved first when the sources changed since the last; a resolved note keeps the
//! snapshot that resolved it. Notes are written whole, under the project's lock.

use std::fs;

use serde_json::{Map, Value, json};
use valle_timeline::time::Tick;
use valle_timeline::{Content, Document, Kind};

use crate::history::now;
use crate::{Error, Project, inside, pretty, write};

/// Where the notes are, in the project.
pub const FILE: &str = "reviews/notes.json";

/// A note on a timeline.
#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub id: String,
    /// The timeline it is on, a path in the project.
    pub timeline: String,
    /// The snapshot it was written against.
    pub snapshot: u64,
    /// Seconds of the timeline: a frame, or a span to `end`.
    pub start: f64,
    pub end: Option<f64>,
    /// The part of the picture it is about, in fractions of the canvas; all of it when none.
    pub region: Option<Region>,
    /// What it is about, as far as was found.
    pub targets: Vec<Target>,
    /// The picture seen, a path in `reviews/`.
    pub image: Option<String>,
    /// The snapshot that resolved it, while it is resolved.
    pub resolved_in: Option<u64>,
    /// What was said, the note first.
    pub thread: Vec<Message>,
}

/// A rectangle of the canvas, from its top left, each side a fraction of the canvas's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Region {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// A clip a note is about, and where in it: seconds from its start, a caption's run, or an
/// element of a scene by its key path and where its JSX is written.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub clip: String,
    pub offset: Option<f64>,
    pub run: Option<u64>,
    pub path: Option<String>,
    pub site: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    /// Who said it: `user`, `ai`, or a name.
    pub by: String,
    pub text: String,
    /// The snapshot it was said of.
    pub snapshot: u64,
    /// Milliseconds since 1970.
    pub time: u64,
}

/// A note to add: its first message, and the picture seen as PNG, JPEG or WebP.
pub struct NewNote {
    /// The timeline, as `Project::timeline` takes its name.
    pub timeline: String,
    pub start: f64,
    pub end: Option<f64>,
    pub region: Option<Region>,
    /// What it is about; when none are given, the clips shown at `start`, as `targets` finds.
    pub targets: Vec<Target>,
    pub image: Option<Vec<u8>>,
    pub by: String,
    pub text: String,
}

impl Note {
    pub fn resolved(&self) -> bool {
        self.resolved_in.is_some()
    }

    /// The note as `reviews/notes.json` has it.
    pub fn to_json(&self) -> Value {
        let mut at = json!({ "start": self.start });
        if let Some(end) = self.end {
            at["end"] = json!(end);
        }
        let mut note = json!({
            "id": self.id,
            "timeline": self.timeline,
            "snapshot": self.snapshot,
            "at": at,
        });
        if let Some(r) = self.region {
            note["region"] = json!({ "x": r.x, "y": r.y, "w": r.w, "h": r.h });
        }
        if !self.targets.is_empty() {
            let targets: Vec<Value> = self.targets.iter().map(Target::to_json).collect();
            note["targets"] = json!(targets);
        }
        if let Some(image) = &self.image {
            note["image"] = json!(image);
        }
        note["status"] = json!(if self.resolved() { "resolved" } else { "open" });
        if let Some(n) = self.resolved_in {
            note["resolvedIn"] = json!(n);
        }
        let thread: Vec<Value> = (self.thread.iter())
            .map(|m| json!({ "by": m.by, "text": m.text, "snapshot": m.snapshot, "time": m.time }))
            .collect();
        note["thread"] = json!(thread);
        note
    }

    /// What a note may not be, wherever it came from.
    fn check(&self, at: &str) -> Result<(), Error> {
        let bad = |field: &str, what: &str| Err(format!("{at}.{field}: {what}"));
        if !self.timeline.ends_with(".timeline.json") || inside(&self.timeline).is_none() {
            return bad("timeline", "a timeline's path in the project");
        }
        if self.snapshot == 0 || self.resolved_in == Some(0) {
            return bad("snapshot", "snapshots are numbered from 1");
        }
        if !(self.start.is_finite() && self.start >= 0.0) {
            return bad("at.start", "seconds from 0");
        }
        if let Some(end) = self.end
            && !(end.is_finite() && end > self.start)
        {
            return bad("at.end", "seconds after at.start");
        }
        if let Some(r) = self.region {
            let fits = |from: f64, size: f64| {
                from.is_finite() && size.is_finite() && from >= 0.0 && size > 0.0
                    // Fractions of a canvas a picture was cut from are not exact.
                    && from + size <= 1.0 + 1e-6
            };
            if !fits(r.x, r.w) || !fits(r.y, r.h) {
                return bad(
                    "region",
                    "x, y, w and h are fractions of the canvas within it",
                );
            }
        }
        for (i, t) in self.targets.iter().enumerate() {
            if t.clip.is_empty() {
                return bad(&format!("targets[{i}].clip"), "a clip's id");
            }
            if let Some(offset) = t.offset
                && !(offset.is_finite() && offset >= 0.0)
            {
                return bad(
                    &format!("targets[{i}].offset"),
                    "seconds from the clip's start",
                );
            }
            if t.path.as_deref() == Some("") || t.site.as_deref() == Some("") {
                return bad(&format!("targets[{i}]"), "path and site are not empty");
            }
        }
        if let Some(image) = &self.image
            && !image.starts_with("crops/")
        {
            return bad("image", "a picture's path in reviews/crops/");
        }
        if self.thread.is_empty() {
            return bad("thread", "the note itself first");
        }
        for (i, m) in self.thread.iter().enumerate() {
            if m.by.trim().is_empty() || m.text.trim().is_empty() {
                return bad(&format!("thread[{i}]"), "by and text are not empty");
            }
            if m.snapshot == 0 {
                return bad(
                    &format!("thread[{i}].snapshot"),
                    "snapshots are numbered from 1",
                );
            }
        }
        Ok(())
    }
}

impl Note {
    /// Where the note is in `doc`, its timeline as it is now: moved with the first clip it is
    /// about that is still there and still shows its moment, or as written where it is about no
    /// clip; none where every clip it is about is gone or trimmed past it.
    pub fn now(&self, doc: &Document) -> Option<(f64, Option<f64>)> {
        if self.targets.is_empty() {
            return Some((self.start, self.end));
        }
        let clips = || doc.tracks.iter().flat_map(|t| &t.clips);
        self.targets.iter().find_map(|t| {
            let clip = clips().find(|c| c.id == t.clip)?;
            let Some(offset) = t.offset else {
                return Some((self.start, self.end));
            };
            if offset > clip.duration.seconds() + 1e-6 {
                return None;
            }
            let start = clip.start.seconds() + offset;
            Some((start, self.end.map(|end| end - self.start + start)))
        })
    }
}

/// The clips of `doc` shown at `seconds`, the one drawn on top first: captions over the picture,
/// later tracks over earlier ones, then what is heard when `heard`. Each says how far into it the
/// time is and, for a caption with timed runs, the run then.
pub fn targets(doc: &Document, seconds: f64, heard: bool) -> Vec<Target> {
    let Ok(at) = Tick::from_seconds(seconds) else {
        return Vec::new();
    };
    let order = |kind| match kind {
        Kind::Caption => Some(0),
        Kind::Visual => Some(1),
        Kind::Audio if heard => Some(2),
        _ => None,
    };
    let mut tracks: Vec<_> = (doc.tracks.iter().rev())
        .filter(|t| t.enabled)
        .filter_map(|t| Some((order(t.kind)?, t)))
        .collect();
    tracks.sort_by_key(|(order, _)| *order);
    let clips = (tracks.into_iter()).flat_map(|(_, t)| t.clips.iter().rev());
    clips
        .filter(|c| c.enabled && c.start <= at && at < c.end())
        .map(|c| {
            let into = at - c.start;
            let run = match &c.content {
                Content::Caption { runs, .. } => (runs.iter())
                    .position(|r| r.start <= into && into < r.end)
                    .map(|i| i as u64),
                _ => None,
            };
            Target {
                clip: c.id.clone(),
                offset: Some((into.seconds() * 1e6).round() / 1e6),
                run,
                path: None,
                site: None,
            }
        })
        .collect()
}

/// The targets given, those without an offset at it where they are shown, then the other clips
/// shown. A target given that is on a clip not shown keeps what it was given.
fn with_shown(given: Vec<Target>, shown: Vec<Target>) -> Vec<Target> {
    let mut out: Vec<Target> = (given.into_iter())
        .map(|mut t| {
            if let Some(s) = shown.iter().find(|s| s.clip == t.clip) {
                t.offset = t.offset.or(s.offset);
                t.run = t.run.or(s.run);
            }
            t
        })
        .collect();
    let rest: Vec<Target> = (shown.into_iter())
        .filter(|s| !out.iter().any(|t| t.clip == s.clip))
        .collect();
    out.extend(rest);
    out
}

impl Target {
    pub fn to_json(&self) -> Value {
        let mut t = json!({ "clip": self.clip });
        if let Some(offset) = self.offset {
            t["offset"] = json!(offset);
        }
        if let Some(run) = self.run {
            t["run"] = json!(run);
        }
        if let Some(path) = &self.path {
            t["path"] = json!(path);
        }
        if let Some(site) = &self.site {
            t["site"] = json!(site);
        }
        t
    }
}

/// The notes as the file has them, and the number the next one takes, never one given before.
struct Notes {
    next: u64,
    notes: Vec<Note>,
}

impl Project {
    /// The notes, oldest first; none before the first.
    pub fn notes(&self) -> Result<Vec<Note>, Error> {
        Ok(self.read_notes()?.notes)
    }

    pub fn note(&self, id: &str) -> Result<Note, Error> {
        (self.notes()?.into_iter())
            .find(|n| n.id == id)
            .ok_or_else(|| format!("no note {id}: valle project notes lists them"))
    }

    /// Adds a note against the sources as they are, saved as a snapshot first if they changed.
    pub fn add_note(&self, new: NewNote) -> Result<Note, Error> {
        // A path in the project first: the current folder may be anywhere.
        let timeline = match inside(&new.timeline) {
            Some(rel) if rel.ends_with(".timeline.json") && self.root.join(&rel).is_file() => rel,
            _ => {
                let path = self.timeline(Some(&new.timeline))?;
                (self.relative(&path))
                    .ok_or_else(|| format!("{}: not a timeline of the project", new.timeline))?
            }
        };
        // A timeline that does not read yet is still noted on, where it was.
        let shown = (self.document(&timeline).ok())
            .map(|doc| targets(&doc, new.start, new.region.is_none()))
            .unwrap_or_default();
        let found = match new.targets.is_empty() {
            true => shown,
            false => with_shown(new.targets, shown),
        };
        let kind = match &new.image {
            Some(bytes) => Some(picture(bytes)?),
            None => None,
        };
        let (snapshot, _) = self.save(None, "note")?;
        let _lock = self.lock()?;
        let mut file = self.read_notes()?;
        let id = format!("n{}", file.next);
        let note = Note {
            image: kind.map(|ext| format!("crops/{id}.{ext}")),
            id,
            timeline,
            snapshot,
            start: new.start,
            end: new.end,
            region: new.region,
            targets: found,
            resolved_in: None,
            thread: vec![Message {
                by: new.by,
                text: new.text,
                snapshot,
                time: now(),
            }],
        };
        note.check("the note")?;
        let image = note
            .image
            .as_ref()
            .map(|p| self.root.join("reviews").join(p));
        if let (Some(path), Some(bytes)) = (&image, &new.image) {
            write(path, bytes, true)?;
        }
        file.next += 1;
        file.notes.push(note.clone());
        if let Err(e) = self.write_notes(&file) {
            if let Some(path) = image {
                let _ = fs::remove_file(path);
            }
            return Err(e);
        }
        Ok(note)
    }

    /// Adds `text` by `by` to note `id`.
    pub fn reply(&self, id: &str, by: &str, text: &str) -> Result<Note, Error> {
        self.change(id, by, Some(text), |_, _| Ok(()))
    }

    /// Marks note `id` resolved by the sources as they are, with what was done if `text` says.
    pub fn resolve(&self, id: &str, by: &str, text: Option<&str>) -> Result<Note, Error> {
        self.change(id, by, text, |note, snapshot| {
            if note.resolved() {
                return Err(format!("{id} is resolved already: reopen it first"));
            }
            note.resolved_in = Some(snapshot);
            Ok(())
        })
    }

    /// Opens resolved note `id` again, with why if `text` says.
    pub fn reopen(&self, id: &str, by: &str, text: Option<&str>) -> Result<Note, Error> {
        self.change(id, by, text, |note, _| {
            if !note.resolved() {
                return Err(format!("{id} is open"));
            }
            note.resolved_in = None;
            Ok(())
        })
    }

    /// Deletes note `id` and its picture.
    pub fn delete_note(&self, id: &str) -> Result<(), Error> {
        let _lock = self.lock()?;
        let mut file = self.read_notes()?;
        let i = (file.notes.iter().position(|n| n.id == id))
            .ok_or_else(|| format!("no note {id}: valle project notes lists them"))?;
        let note = file.notes.remove(i);
        self.write_notes(&file)?;
        if let Some(image) = note.image {
            let _ = fs::remove_file(self.root.join("reviews").join(image));
        }
        Ok(())
    }

    /// Changes note `id` by `change`, given the snapshot of the sources as they are, and adds
    /// `text` by `by` of that snapshot to it.
    fn change(
        &self,
        id: &str,
        by: &str,
        text: Option<&str>,
        change: impl FnOnce(&mut Note, u64) -> Result<(), Error>,
    ) -> Result<Note, Error> {
        self.note(id)?;
        let (snapshot, _) = self.save(None, "note")?;
        let _lock = self.lock()?;
        let mut file = self.read_notes()?;
        let i = (file.notes.iter().position(|n| n.id == id))
            .ok_or_else(|| format!("no note {id}: valle project notes lists them"))?;
        let note = &mut file.notes[i];
        change(note, snapshot)?;
        if let Some(text) = text {
            note.thread.push(Message {
                by: by.to_string(),
                text: text.to_string(),
                snapshot,
                time: now(),
            });
        }
        note.check(id)?;
        let note = note.clone();
        self.write_notes(&file)?;
        Ok(note)
    }

    fn read_notes(&self) -> Result<Notes, Error> {
        let path = self.root.join(FILE);
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Notes {
                    next: 1,
                    notes: Vec::new(),
                });
            }
            Err(e) => return Err(format!("{FILE}: {e}")),
        };
        let root: Value = serde_json::from_str(&text).map_err(|e| format!("{FILE}: {e}"))?;
        parse(&root).map_err(|e| format!("{FILE}: {e}"))
    }

    fn write_notes(&self, file: &Notes) -> Result<(), Error> {
        let notes: Vec<Value> = file.notes.iter().map(Note::to_json).collect();
        let root = json!({ "next": file.next, "notes": notes });
        write(&self.root.join(FILE), &pretty(&root), true)
    }
}

/// The extension of a picture by its first bytes: PNG, JPEG or WebP.
fn picture(bytes: &[u8]) -> Result<&'static str, Error> {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => Ok("png"),
        [0xff, 0xd8, 0xff, ..] => Ok("jpg"),
        [
            b'R',
            b'I',
            b'F',
            b'F',
            _,
            _,
            _,
            _,
            b'W',
            b'E',
            b'B',
            b'P',
            ..,
        ] => Ok("webp"),
        _ => Err("a note's picture is PNG, JPEG or WebP".into()),
    }
}

fn parse(root: &Value) -> Result<Notes, Error> {
    let o = object(root, "the file", &["next", "notes"])?;
    let list = (o.get("notes").and_then(Value::as_array)).ok_or("notes: a list of notes")?;
    let mut notes = Vec::new();
    for (i, v) in list.iter().enumerate() {
        let at = format!("notes[{i}]");
        let note = parse_note(v, &at)?;
        note.check(&at)?;
        if notes.iter().any(|n: &Note| n.id == note.id) {
            return Err(format!("{at}.id: {} is another note's", note.id));
        }
        notes.push(note);
    }
    let taken = (notes.iter())
        .filter_map(|n| n.id.strip_prefix('n')?.parse::<u64>().ok())
        .max()
        .unwrap_or(0);
    let next = match o.get("next") {
        None => taken + 1,
        Some(v) => v.as_u64().ok_or("next: the number of the next note")?,
    };
    if next <= taken {
        return Err(format!("next: {next}, but n{taken} is taken"));
    }
    Ok(Notes { next, notes })
}

fn parse_note(v: &Value, at: &str) -> Result<Note, Error> {
    const FIELDS: [&str; 10] = [
        "id",
        "timeline",
        "snapshot",
        "at",
        "region",
        "targets",
        "image",
        "status",
        "resolvedIn",
        "thread",
    ];
    let o = object(v, at, &FIELDS)?;
    let field = |k: &str| format!("{at}.{k}");
    let times = object(required(o, "at", at)?, &field("at"), &["start", "end"])?;
    let region = match o.get("region") {
        None => None,
        Some(v) => {
            let r = object(v, &field("region"), &["x", "y", "w", "h"])?;
            let n = |k: &str| {
                number(
                    required(r, k, &field("region"))?,
                    &field(&format!("region.{k}")),
                )
            };
            Some(Region {
                x: n("x")?,
                y: n("y")?,
                w: n("w")?,
                h: n("h")?,
            })
        }
    };
    let targets = match o.get("targets") {
        None => Vec::new(),
        Some(v) => (v
            .as_array()
            .ok_or_else(|| format!("{}: a list", field("targets")))?)
        .iter()
        .enumerate()
        .map(|(i, v)| parse_target(v, &field(&format!("targets[{i}]"))))
        .collect::<Result<_, _>>()?,
    };
    let resolved_in = o
        .get("resolvedIn")
        .map(|v| count(v, &field("resolvedIn")))
        .transpose()?;
    match (
        string(required(o, "status", at)?, &field("status"))?.as_str(),
        resolved_in,
    ) {
        ("open", None) | ("resolved", Some(_)) => {}
        ("open", Some(_)) => {
            return Err(format!(
                "{}: an open note is not resolved",
                field("resolvedIn")
            ));
        }
        ("resolved", None) => {
            return Err(format!(
                "{}: the snapshot that resolved it",
                field("resolvedIn")
            ));
        }
        _ => return Err(format!("{}: open or resolved", field("status"))),
    }
    let thread = (required(o, "thread", at)?.as_array())
        .ok_or_else(|| format!("{}: a list of messages", field("thread")))?
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let at = field(&format!("thread[{i}]"));
            let m = object(v, &at, &["by", "text", "snapshot", "time"])?;
            let f = |k: &str| format!("{at}.{k}");
            Ok(Message {
                by: string(required(m, "by", &at)?, &f("by"))?,
                text: string(required(m, "text", &at)?, &f("text"))?,
                snapshot: count(required(m, "snapshot", &at)?, &f("snapshot"))?,
                time: m.get("time").map_or(Ok(0), |v| count(v, &f("time")))?,
            })
        })
        .collect::<Result<_, Error>>()?;
    Ok(Note {
        id: string(required(o, "id", at)?, &field("id"))?,
        timeline: string(required(o, "timeline", at)?, &field("timeline"))?,
        snapshot: count(required(o, "snapshot", at)?, &field("snapshot"))?,
        start: number(required(times, "start", &field("at"))?, &field("at.start"))?,
        end: (times.get("end"))
            .map(|v| number(v, &field("at.end")))
            .transpose()?,
        region,
        targets,
        image: (o.get("image"))
            .map(|v| string(v, &field("image")))
            .transpose()?,
        resolved_in,
        thread,
    })
}

fn parse_target(v: &Value, at: &str) -> Result<Target, Error> {
    let o = object(v, at, &["clip", "offset", "run", "path", "site"])?;
    let field = |k: &str| format!("{at}.{k}");
    let text = |k: &str| o.get(k).map(|v| string(v, &field(k))).transpose();
    Ok(Target {
        clip: string(required(o, "clip", at)?, &field("clip"))?,
        offset: (o.get("offset"))
            .map(|v| number(v, &field("offset")))
            .transpose()?,
        run: o.get("run").map(|v| count(v, &field("run"))).transpose()?,
        path: text("path")?,
        site: text("site")?,
    })
}

/// `v` as an object of only `fields`.
fn object<'a>(v: &'a Value, at: &str, fields: &[&str]) -> Result<&'a Map<String, Value>, Error> {
    let o = v.as_object().ok_or_else(|| format!("{at}: an object"))?;
    if let Some(k) = o.keys().find(|k| !fields.contains(&k.as_str())) {
        return Err(format!("{at}: no field {k}: it has {}", fields.join(", ")));
    }
    Ok(o)
}

fn required<'a>(o: &'a Map<String, Value>, k: &str, at: &str) -> Result<&'a Value, Error> {
    o.get(k).ok_or_else(|| format!("{at}: {k} is missing"))
}

fn string(v: &Value, at: &str) -> Result<String, Error> {
    (v.as_str().map(str::to_string)).ok_or_else(|| format!("{at}: a string"))
}

fn number(v: &Value, at: &str) -> Result<f64, Error> {
    v.as_f64().ok_or_else(|| format!("{at}: a number"))
}

fn count(v: &Value, at: &str) -> Result<u64, Error> {
    v.as_u64().ok_or_else(|| format!("{at}: a whole number"))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;

    use super::{NewNote, Region, Target};
    use crate::{Project, pretty};

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nrest";

    fn project(name: &str) -> Project {
        let dir = std::env::temp_dir().join(format!("valle-notes-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        Project::create(&dir, 640, 360, "30").expect("created")
    }

    fn new(text: &str) -> NewNote {
        NewNote {
            timeline: "main.timeline.json".into(),
            start: 1.5,
            end: None,
            region: None,
            targets: Vec::new(),
            image: None,
            by: "user".into(),
            text: text.into(),
        }
    }

    #[test]
    fn a_note_is_written_against_a_snapshot_of_the_sources() {
        let p = project("snapshot");
        let first = p
            .add_note(NewNote {
                end: Some(3.0),
                region: Some(Region {
                    x: 0.5,
                    y: 0.0,
                    w: 0.5,
                    h: 0.25,
                }),
                targets: vec![Target {
                    clip: "intro".into(),
                    offset: Some(0.5),
                    run: None,
                    path: Some("card/title".into()),
                    site: Some("components/title.motion.tsx:4:7".into()),
                }],
                image: Some(PNG.to_vec()),
                ..new("the logo covers the caption")
            })
            .expect("added");
        assert_eq!((first.id.as_str(), first.snapshot), ("n1", 1));
        assert_eq!(first.image.as_deref(), Some("crops/n1.png"));
        assert_eq!(fs::read(p.root.join("reviews/crops/n1.png")).unwrap(), PNG);
        assert_eq!(p.history().unwrap()[0].by, "note");
        // Nothing changed: the same snapshot.
        assert_eq!(p.add_note(new("again")).unwrap().snapshot, 1);
        // Notes are not sources: writing them saves no snapshot.
        assert_eq!(p.save(None, "save").unwrap(), (1, false));
        let mut timeline: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(p.root.join(&p.timeline)).unwrap()).unwrap();
        timeline["name"] = json!("cut");
        fs::write(p.root.join(&p.timeline), pretty(&timeline)).unwrap();
        let third = p.add_note(new("after an edit")).unwrap();
        assert_eq!((third.id.as_str(), third.snapshot), ("n3", 2));
        assert_eq!(p.note("n1").unwrap(), first);
        let text = fs::read_to_string(p.root.join(super::FILE)).unwrap();
        let file: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(file["notes"][0]["at"], json!({ "start": 1.5, "end": 3.0 }));
        assert_eq!(file["notes"][0]["status"], "open");
        assert_eq!(file["notes"][0]["targets"][0]["path"], "card/title");
    }

    #[test]
    fn resolving_records_the_snapshot_that_did() {
        let p = project("resolve");
        p.add_note(new("too fast")).unwrap();
        let path = p.root.join(&p.timeline);
        fs::write(&path, fs::read_to_string(&path).unwrap().replace("{", "{ ")).unwrap();
        let done = p.resolve("n1", "ai", Some("slowed to 0.8")).unwrap();
        assert_eq!(done.resolved_in, Some(2));
        assert_eq!(done.thread[1].by, "ai");
        assert_eq!(done.thread[1].snapshot, 2);
        assert!(
            p.resolve("n1", "ai", None)
                .unwrap_err()
                .contains("resolved already")
        );
        let open = p.reopen("n1", "user", Some("still too fast")).unwrap();
        assert_eq!(open.resolved_in, None);
        assert_eq!(open.thread.len(), 3);
        assert!(
            p.reopen("n1", "user", None)
                .unwrap_err()
                .contains("is open")
        );
        let replied = p.reply("n1", "ai", "which part?").unwrap();
        assert_eq!(replied.thread.len(), 4);
        assert!(p.reply("n9", "ai", "x").unwrap_err().contains("no note n9"));
    }

    #[test]
    fn a_deleted_note_takes_its_picture_and_its_number() {
        let p = project("delete");
        p.add_note(NewNote {
            image: Some(PNG.to_vec()),
            ..new("one")
        })
        .unwrap();
        p.add_note(new("two")).unwrap();
        p.delete_note("n2").unwrap();
        p.delete_note("n1").unwrap();
        assert!(!p.root.join("reviews/crops/n1.png").exists());
        assert!(p.notes().unwrap().is_empty());
        // A number given before is never given again.
        assert_eq!(p.add_note(new("three")).unwrap().id, "n3");
        assert!(p.delete_note("n1").is_err());
    }

    #[test]
    fn a_note_moves_with_its_clip_until_it_is_gone() {
        let p = project("moves");
        let doc = |second: f64, trim: f64| {
            let timeline = json!({
                "canvas": { "width": 640, "height": 360, "fps": 30 },
                "tracks": { "visual": [{ "clips": [
                    { "id": "a", "kind": "solid", "color": "#000000ff", "start": 0, "duration": 2 },
                    { "id": "b", "kind": "solid", "color": "#ffffffff", "start": second, "duration": trim }
                ] }] }
            });
            valle_timeline::Document::read_edit(&timeline).expect("a timeline")
        };
        let target = |clip: &str, offset: f64| Target {
            clip: clip.into(),
            offset: Some(offset),
            run: None,
            path: None,
            site: None,
        };
        let note = p
            .add_note(NewNote {
                start: 3.0,
                end: Some(3.5),
                targets: vec![target("gone", 0.0), target("b", 1.0)],
                ..new("cut here")
            })
            .unwrap();
        assert_eq!(note.now(&doc(2.0, 2.0)), Some((3.0, Some(3.5))));
        assert_eq!(note.now(&doc(4.0, 2.0)), Some((5.0, Some(5.5))));
        // Trimmed past its moment, or gone, the note is nowhere.
        assert_eq!(note.now(&doc(2.0, 0.5)), None);
        let lone = p.add_note(new("all of it")).unwrap();
        assert_eq!(lone.now(&doc(9.0, 1.0)), Some((1.5, None)));
    }

    #[test]
    fn a_note_without_targets_is_about_what_is_shown_on_top_first() {
        let p = project("targets");
        let solid = |id: &str, start: f64| json!({ "id": id, "kind": "solid", "color": "#000000ff", "start": start, "duration": 2 });
        let timeline = json!({
            "canvas": { "width": 640, "height": 360, "fps": 30 },
            "tracks": {
                "visual": [{ "clips": [solid("back", 0.0)] }, { "clips": [solid("over", 1.0)] }],
                "audio": [{ "clips": [{ "id": "music", "start": 0, "duration": 4, "src": "x" }] }],
                "caption": [{ "clips": [{ "id": "line", "start": 1, "duration": 2, "runs": [
                    { "text": "Word ", "start": 0, "end": 0.6 },
                    { "text": "by word", "start": 0.6, "end": 1.8 }
                ] }] }]
            }
        });
        let doc = valle_timeline::Document::read_edit(&timeline).expect("a timeline");
        let found = |seconds: f64, heard: bool| {
            (super::targets(&doc, seconds, heard).into_iter())
                .map(|t| (t.clip, t.offset.unwrap(), t.run))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            found(1.75, false),
            [
                ("line".into(), 0.75, Some(1)),
                ("over".into(), 0.75, None),
                ("back".into(), 1.75, None),
            ]
        );
        assert_eq!(found(0.5, true).len(), 2);
        assert_eq!(found(0.5, true)[1].0, "music");
        // Added without targets, a note is about the clips found there.
        fs::write(p.root.join(&p.timeline), pretty(&timeline)).unwrap();
        let note = p.add_note(new("too dark")).unwrap();
        let clips: Vec<_> = note.targets.iter().map(|t| t.clip.as_str()).collect();
        assert_eq!(clips, ["line", "over", "back", "music"]);
        assert_eq!(note.targets[0].run, Some(0));
        // What was picked comes first, where it is shown, and the rest after.
        let picked = Target {
            clip: "over".into(),
            offset: None,
            run: None,
            path: Some("card/title".into()),
            site: Some("scene.motion.tsx:6:9".into()),
        };
        let note = p
            .add_note(NewNote {
                targets: vec![picked],
                ..new("the title")
            })
            .unwrap();
        let clips: Vec<_> = note.targets.iter().map(|t| t.clip.as_str()).collect();
        assert_eq!(clips, ["over", "line", "back", "music"]);
        assert_eq!(
            note.targets[0].offset,
            note.targets[2].offset.map(|o| o - 1.0)
        );
        assert_eq!(note.targets[0].path.as_deref(), Some("card/title"));
    }

    #[test]
    fn what_a_note_may_not_be_is_refused() {
        let p = project("refused");
        let refused = |n: NewNote| p.add_note(n).unwrap_err();
        assert!(
            refused(NewNote {
                start: -1.0,
                ..new("x")
            })
            .contains("at.start")
        );
        assert!(
            refused(NewNote {
                end: Some(1.0),
                ..new("x")
            })
            .contains("at.end")
        );
        let region = Region {
            x: 0.8,
            y: 0.0,
            w: 0.5,
            h: 0.1,
        };
        assert!(
            refused(NewNote {
                region: Some(region),
                ..new("x")
            })
            .contains("region")
        );
        assert!(refused(new(" ")).contains("thread[0]"));
        assert!(
            refused(NewNote {
                image: Some(b"GIF89a".to_vec()),
                ..new("x")
            })
            .contains("PNG")
        );
        assert!(
            refused(NewNote {
                timeline: "nope".into(),
                ..new("x")
            })
            .contains("no timeline")
        );
        assert!(p.notes().unwrap().is_empty());
        // A file edited by hand is read as strictly as one written here.
        fs::create_dir_all(p.root.join("reviews")).unwrap();
        let bad = |notes: serde_json::Value| {
            fs::write(p.root.join(super::FILE), notes.to_string()).unwrap();
            p.notes().unwrap_err()
        };
        let note = json!({ "id": "n1", "timeline": "main.timeline.json", "snapshot": 1,
            "at": { "start": 0 }, "status": "open",
            "thread": [{ "by": "user", "text": "x", "snapshot": 1 }] });
        let mut odd = note.clone();
        odd["colour"] = json!("red");
        assert!(bad(json!({ "notes": [odd] })).contains("no field colour"));
        let mut resolved = note.clone();
        resolved["status"] = json!("resolved");
        assert!(bad(json!({ "notes": [resolved] })).contains("resolvedIn"));
        assert!(bad(json!({ "notes": [note, note] })).contains("another note's"));
        assert!(bad(json!({ "next": 1, "notes": [note] })).contains("n1 is taken"));
        fs::write(
            p.root.join(super::FILE),
            json!({ "notes": [note] }).to_string(),
        )
        .unwrap();
        assert_eq!(p.add_note(new("next")).unwrap().id, "n2");
    }
}
