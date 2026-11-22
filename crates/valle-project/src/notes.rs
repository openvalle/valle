//! Review notes: what someone watching an edit asks to change, in `reviews/notes.json`, with the
//! picture they saw in `reviews/crops/`. Notes are not part of the edit, as a timeline's `markers`
//! are, so writing one never touches a source. Each message of a note keeps the snapshot it was
//! written against, saved first when the sources changed since the last; a resolved note keeps the
//! snapshot that resolved it. Notes are written whole, under the project's lock.

use std::fs;

use serde_json::{Map, Value, json};
use valle_timeline::query::source_at;
use valle_timeline::time::Tick;
use valle_timeline::{Clip, Content, Document, Kind, SourceTime};

use crate::history::{Source, linked, now};
use crate::{Error, Project, inside, pretty, write};

/// Where the notes are, in the project.
pub const FILE: &str = "reviews/notes.json";

/// A note on a timeline, or on a component reviewed alone.
#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub id: String,
    /// The timeline or the component, a `*.motion.tsx`, it is on: a path in the project.
    pub file: String,
    /// The snapshot it was written against.
    pub snapshot: u64,
    /// Seconds of the timeline or component: a frame, or a span to `end`.
    pub start: f64,
    pub end: Option<f64>,
    /// The part of the picture it is about, in fractions of the canvas; all of it when none.
    pub region: Option<Region>,
    /// What it is about, as far as was found.
    pub targets: Vec<Target>,
    /// On a component, the props its preview was played with but its defaults.
    pub props: Option<Map<String, Value>>,
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
/// element of a scene by its key path and where its JSX is written. On a component, it is an
/// element of it, and of no clip.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub clip: Option<String>,
    pub offset: Option<f64>,
    /// Seconds of what the clip plays, a video, a sound, a timeline or a scene, then: the note
    /// follows the picture there through trims and changes of speed.
    pub source: Option<f64>,
    /// For a note over a span, the seconds of the source it plays at the span's end.
    pub source_end: Option<f64>,
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
    /// The timeline, as `Project::timeline` takes its name, or a component's path.
    pub file: String,
    pub start: f64,
    pub end: Option<f64>,
    pub region: Option<Region>,
    /// What it is about; on a timeline, the clips shown at `start` after those given, as
    /// `targets` finds them.
    pub targets: Vec<Target>,
    /// On a component, the props its preview was played with but its defaults.
    pub props: Option<Map<String, Value>>,
    /// The files the picture was drawn from, by path in the project, each by its SHA-256 or,
    /// read in parts, its version as `crate::version` gives it: the note is refused where its
    /// snapshot has another or none, as when a file changed or went after it was seen.
    pub seen: Vec<(String, String)>,
    pub image: Option<Vec<u8>>,
    pub by: String,
    pub text: String,
}

impl Note {
    pub fn resolved(&self) -> bool {
        self.resolved_in.is_some()
    }

    /// Whether it is on a component reviewed alone, not a timeline.
    pub fn on_component(&self) -> bool {
        self.file.ends_with(".motion.tsx")
    }

    /// The note as `reviews/notes.json` has it.
    pub fn to_json(&self) -> Value {
        let mut at = json!({ "start": self.start });
        if let Some(end) = self.end {
            at["end"] = json!(end);
        }
        let mut note = json!({ "id": self.id });
        note[if self.on_component() {
            "component"
        } else {
            "timeline"
        }] = json!(self.file);
        note["snapshot"] = json!(self.snapshot);
        note["at"] = at;
        if let Some(r) = self.region {
            note["region"] = json!({ "x": r.x, "y": r.y, "w": r.w, "h": r.h });
        }
        if !self.targets.is_empty() {
            let targets: Vec<Value> = self.targets.iter().map(Target::to_json).collect();
            note["targets"] = json!(targets);
        }
        if let Some(props) = &self.props {
            note["props"] = json!(props);
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
        // As add_note numbers them: its picture is named by it.
        let number = (self.id.strip_prefix('n')).and_then(|n| n.parse::<u64>().ok());
        if !number.is_some_and(|n| n > 0 && format!("n{n}") == self.id) {
            return bad("id", "n and the note's number, from 1");
        }
        let component = self.on_component();
        if !(component || self.file.ends_with(".timeline.json")) || inside(&self.file).is_none() {
            return bad(
                if component { "component" } else { "timeline" },
                "a timeline's path in the project, or a *.motion.tsx component's",
            );
        }
        if self.props.is_some() && !component {
            return bad(
                "props",
                "a component's, which a timeline's clips give their own",
            );
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
            match (&t.clip, component) {
                (Some(clip), false) if !clip.is_empty() => {}
                (None, true) if t.offset.is_none() && t.run.is_none() => {}
                (_, false) => return bad(&format!("targets[{i}].clip"), "a clip's id"),
                (_, true) => {
                    return bad(
                        &format!("targets[{i}]"),
                        "on a component, an element: its path and site, of no clip",
                    );
                }
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
        // Its own picture only: deleting the note deletes it.
        if let Some(image) = &self.image
            && !(image.strip_prefix(&format!("crops/{}.", self.id)))
                .is_some_and(|ext| ["png", "jpg", "webp"].contains(&ext))
        {
            return bad(
                "image",
                &format!("its picture, crops/{}.png, .jpg or .webp", self.id),
            );
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
            let clip = clips().find(|c| Some(&c.id) == t.clip.as_ref())?;
            let duration = clip.duration.seconds();
            let span = self.end.map(|end| end - self.start);
            let (written, playing) = (t.offset.unwrap_or(0.0), plays(clip));
            // A clip shows its last moment just before its end.
            let offset = match (t.source.filter(|_| playing), t.offset) {
                (Some(source), _) => clip_time(clip, source, written, [-1e-6, duration - 1e-6])?,
                (None, Some(offset)) if offset < duration - 1e-6 => offset,
                (None, Some(_)) => return None,
                (None, None) => return Some((self.start, self.end)),
            };
            // A span past the clip's end, over a cut, keeps its length past it.
            let end = span.and_then(|span| {
                let near = written + span;
                // An end in the clip that a trim took off is at the clip's end.
                let end = match t.source_end.filter(|_| playing) {
                    Some(source) => (clip_time(clip, source, near, [1e-6, duration + 1e-6]))
                        .map_or(duration, |end| end.min(duration)),
                    None => offset + span,
                };
                (end > offset + 1e-6).then_some(end)
            });
            let start = clip.start.seconds();
            Some((start + offset, end.map(|end| start + end)))
        })
    }
}

/// Whether `clip` plays a source of its own, whose time its trim and speed say.
fn plays(clip: &Clip) -> bool {
    matches!(
        clip.content,
        Content::Video { .. }
            | Content::Audio { .. }
            | Content::Timeline { .. }
            | Content::Motion(_)
    )
}

/// The seconds into `clip`, within `[from, to]`, at which it plays `source` seconds of its
/// source, the nearest to `near` where it plays them more than once, as a time map going back or
/// held does; none where it does not play them.
fn clip_time(clip: &Clip, source: f64, near: f64, [lo, hi]: [f64; 2]) -> Option<f64> {
    let same = |y: f64| (y - source).abs() < 1e-6;
    // The spans of the clip's time that play it: a moment, or a while where it is held.
    let mut spans = Vec::new();
    match &clip.source.time {
        SourceTime::Linear { trim, rate } => {
            let rate = rate.as_f64();
            if rate != 0.0 {
                let x = (source - trim.seconds()) / rate;
                spans.push((x, x));
            }
        }
        SourceTime::Map(points) => {
            let points: Vec<(f64, f64)> = (points.iter())
                .map(|(x, y)| (x.seconds(), y.seconds()))
                .collect();
            let (first, last) = (points[0], points[points.len() - 1]);
            // It holds its ends past them.
            if same(first.1) {
                spans.push((f64::NEG_INFINITY, first.0));
            }
            for w in points.windows(2) {
                let [(x0, y0), (x1, y1)] = [w[0], w[1]];
                if same(y0) && same(y1) {
                    spans.push((x0, x1));
                } else if y0.min(y1) - 1e-6 <= source && source <= y0.max(y1) + 1e-6 {
                    let x = x0 + (source - y0) * (x1 - x0) / (y1 - y0);
                    spans.push((x, x));
                }
            }
            if same(last.1) {
                spans.push((last.0, f64::INFINITY));
            }
        }
    }
    (spans.into_iter())
        .map(|(from, to)| (from.max(lo), to.min(hi)))
        .filter(|(from, to)| from <= to)
        .map(|(from, to)| near.clamp(from, to).max(0.0))
        .min_by(|a, b| (a - near).abs().total_cmp(&(b - near).abs()))
}

/// The seconds of its source that `t`'s clip in `doc` plays `seconds` into it, if it plays one
/// and is still playing then.
fn source_of(doc: &Document, t: &Target, seconds: f64) -> Option<f64> {
    let clip =
        (doc.tracks.iter().flat_map(|t| &t.clips)).find(|c| Some(&c.id) == t.clip.as_ref())?;
    let into = Tick::from_seconds(seconds).ok()?;
    let within = seconds <= clip.duration.seconds() + 1e-6;
    (plays(clip) && within).then(|| round(source_at(&clip.source.time, into).seconds()))
}

/// Seconds rounded as notes keep them.
fn round(seconds: f64) -> f64 {
    (seconds * 1e6).round() / 1e6
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
                clip: Some(c.id.clone()),
                offset: Some(round(into.seconds())),
                source: None,
                source_end: None,
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
        let mut t = json!({});
        if let Some(clip) = &self.clip {
            t["clip"] = json!(clip);
        }
        if let Some(offset) = self.offset {
            t["offset"] = json!(offset);
        }
        if let Some(source) = self.source {
            t["source"] = json!(source);
        }
        if let Some(end) = self.source_end {
            t["sourceEnd"] = json!(end);
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
        let on = match inside(&new.file) {
            Some(rel) if rel.ends_with(".timeline.json") && self.root.join(&rel).is_file() => rel,
            _ if new.file.ends_with(".motion.tsx") => self.reviewed(&new.file)?,
            _ => {
                let path = self.timeline(Some(&new.file))?;
                (self.relative(&path))
                    .ok_or_else(|| format!("{}: not a timeline of the project", new.file))?
            }
        };
        let found = match on.ends_with(".motion.tsx") {
            true => new.targets,
            // A timeline that does not read yet is still noted on, where it was.
            false => match self.document(&on) {
                Ok(doc) => {
                    let shown = targets(&doc, new.start, new.region.is_none());
                    let mut found = with_shown(new.targets, shown);
                    for t in &mut found {
                        let Some(offset) = t.offset else { continue };
                        t.source = t.source.or_else(|| source_of(&doc, t, offset));
                        if let Some(end) = new.end {
                            let to = offset + end - new.start;
                            t.source_end = t.source_end.or_else(|| source_of(&doc, t, to));
                        }
                    }
                    found
                }
                Err(_) => new.targets,
            },
        };
        let kind = match &new.image {
            Some(bytes) => Some(picture(bytes)?),
            None => None,
        };
        // What it is on is kept, wherever it is, as all a note is on; the files the picture was
        // drawn from are checked against it before it is saved, so a note refused saves none.
        let seen = |kept: &[Source]| {
            for (path, seen) in &new.seen {
                let again = "look at it again, then add the note";
                let at = self.root.join(path);
                let same = match kept.iter().find(|f| &f.path == path) {
                    Some(f) if seen.contains(':') => self.is_version(path, seen, &f.sha256),
                    Some(f) => &f.sha256 == seen,
                    // Read but no source, it is checked where it is.
                    None if !at.is_file() => {
                        return Err(format!(
                            "{path} is gone since the picture was drawn: {again}"
                        ));
                    }
                    None if seen.contains(':') => crate::version(&at).as_ref() == Some(seen),
                    None => crate::digest(&at).ok().as_ref() == Some(seen),
                };
                if !same {
                    return Err(format!(
                        "{path} changed after the picture was drawn: {again}"
                    ));
                }
            }
            Ok(())
        };
        let (snapshot, _) = self.save_with(None, "note", std::slice::from_ref(&on), seen)?;
        let _lock = self.lock()?;
        let mut file = self.read_notes()?;
        let id = format!("n{}", file.next);
        let note = Note {
            image: kind.map(|ext| format!("crops/{id}.{ext}")),
            id,
            file: on,
            snapshot,
            start: new.start,
            end: new.end,
            region: new.region,
            targets: found,
            props: new.props.filter(|p| !p.is_empty()),
            resolved_in: None,
            thread: vec![Message {
                by: new.by,
                text: new.text,
                snapshot,
                time: now(),
            }],
        };
        note.check("the note")?;
        let image_at = note.image.as_ref().map(|p| format!("reviews/{p}"));
        if let Some(path) = &image_at {
            self.unlinked(path)?;
        }
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
        // As read, its picture is its own: check allows no other, and none through a link.
        if let Some(image) = note.image
            && self.unlinked(&format!("reviews/{image}")).is_ok()
        {
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

    /// `path` in the project if no symbolic link is on the way to it, which would take a write or
    /// a delete out of `reviews/`.
    fn unlinked(&self, path: &str) -> Result<(), Error> {
        match linked(&self.root, path) {
            true => Err(format!(
                "{path}: a symbolic link is on the way to it, or it is not a file"
            )),
            false => Ok(()),
        }
    }

    /// The timelines and components notes are on, as the file has them; none where it does not
    /// read.
    pub(crate) fn noted(&self) -> Vec<String> {
        let notes = self.read_notes().map(|f| f.notes).unwrap_or_default();
        let mut on: Vec<String> = notes.into_iter().map(|n| n.file).collect();
        on.sort();
        on.dedup();
        on
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
        self.unlinked(FILE)?;
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
    const FIELDS: [&str; 12] = [
        "id",
        "timeline",
        "component",
        "snapshot",
        "at",
        "region",
        "targets",
        "props",
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
        file: match (o.get("timeline"), o.get("component")) {
            (Some(v), None) => string(v, &field("timeline"))?,
            (None, Some(v)) => string(v, &field("component"))?,
            _ => {
                return Err(format!(
                    "{at}: on a timeline or a component, one of the two"
                ));
            }
        },
        snapshot: count(required(o, "snapshot", at)?, &field("snapshot"))?,
        start: number(required(times, "start", &field("at"))?, &field("at.start"))?,
        end: (times.get("end"))
            .map(|v| number(v, &field("at.end")))
            .transpose()?,
        region,
        targets,
        props: match o.get("props") {
            None => None,
            Some(v) => Some(
                (v.as_object().cloned()).ok_or_else(|| format!("{}: an object", field("props")))?,
            ),
        },
        image: (o.get("image"))
            .map(|v| string(v, &field("image")))
            .transpose()?,
        resolved_in,
        thread,
    })
}

fn parse_target(v: &Value, at: &str) -> Result<Target, Error> {
    let o = object(
        v,
        at,
        &[
            "clip",
            "offset",
            "source",
            "sourceEnd",
            "run",
            "path",
            "site",
        ],
    )?;
    let field = |k: &str| format!("{at}.{k}");
    let text = |k: &str| o.get(k).map(|v| string(v, &field(k))).transpose();
    Ok(Target {
        clip: text("clip")?,
        offset: (o.get("offset"))
            .map(|v| number(v, &field("offset")))
            .transpose()?,
        source: (o.get("source"))
            .map(|v| number(v, &field("source")))
            .transpose()?,
        source_end: (o.get("sourceEnd"))
            .map(|v| number(v, &field("sourceEnd")))
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
            file: "main.timeline.json".into(),
            start: 1.5,
            end: None,
            region: None,
            targets: Vec::new(),
            props: None,
            seen: Vec::new(),
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
                    clip: Some("intro".into()),
                    offset: Some(0.5),
                    source: None,
                    source_end: None,
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
            clip: Some(clip.into()),
            offset: Some(offset),
            source: None,
            source_end: None,
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
    fn a_note_on_footage_follows_its_picture_through_trims_and_speed() {
        let p = project("footage");
        let timeline = |trim: f64, rate: f64, duration: f64| {
            json!({
                "canvas": { "width": 640, "height": 360, "fps": 30 },
                "tracks": { "visual": [{ "clips": [{ "id": "shot", "kind": "video", "src": "x.mp4",
                    "start": 0, "duration": duration, "trimStart": trim, "rate": rate }] }] }
            })
        };
        let doc =
            |t: serde_json::Value| valle_timeline::Document::read_edit(&t).expect("a timeline");
        fs::write(p.root.join(&p.timeline), pretty(&timeline(0.0, 1.0, 8.0))).unwrap();
        let at = |start: f64| {
            p.add_note(NewNote {
                start,
                ..new("here")
            })
            .unwrap()
        };
        let (first, second) = (at(1.0), at(3.0));
        assert_eq!(second.targets[0].source, Some(3.0));
        // The first 2 s trimmed off: what was at 3 s is at 1 s, and what was at 1 s is gone.
        let trimmed = doc(timeline(2.0, 1.0, 6.0));
        assert_eq!(first.now(&trimmed), None);
        assert_eq!(second.now(&trimmed), Some((1.0, None)));
        // Played twice as fast from the start, 3 s of it is at 1.5 s.
        assert_eq!(second.now(&doc(timeline(0.0, 2.0, 4.0))), Some((1.5, None)));
        // Cut at 4 s, the clip no longer shows what was at 4 s.
        let fourth = at(4.0);
        assert_eq!(fourth.now(&doc(timeline(0.0, 1.0, 4.0))), None);
        // Over a span, both ends follow: 2 to 4 s of it, twice as fast, is 1 to 2 s.
        let span = p
            .add_note(NewNote {
                start: 2.0,
                end: Some(4.0),
                ..new("over")
            })
            .unwrap();
        assert_eq!(span.targets[0].source_end, Some(4.0));
        assert_eq!(
            span.now(&doc(timeline(0.0, 2.0, 4.0))),
            Some((1.0, Some(2.0)))
        );
        // Its end trimmed off, a span ends with the clip.
        assert_eq!(
            span.now(&doc(timeline(0.0, 1.0, 3.0))),
            Some((2.0, Some(3.0)))
        );
        // Played forth then back, or held, a moment is shown more than once: the note stays at
        // the one it was written on.
        let mapped = |map: serde_json::Value| {
            doc(json!({
                "canvas": { "width": 640, "height": 360, "fps": 30 },
                "tracks": { "visual": [{ "clips": [{ "id": "shot", "kind": "video", "src": "x.mp4",
                    "start": 0, "duration": 8, "timeMap": map }] }] }
            }))
        };
        for map in [
            json!([[0, 0], [4, 4], [8, 0]]),
            json!([[0, 0], [2, 2], [6, 2], [8, 4]]),
        ] {
            fs::write(
                p.root.join(&p.timeline),
                pretty(&json!({
                    "canvas": { "width": 640, "height": 360, "fps": 30 },
                    "tracks": { "visual": [{ "clips": [{ "id": "shot", "kind": "video",
                        "src": "x.mp4", "start": 0, "duration": 8, "timeMap": map }] }] }
                })),
            )
            .unwrap();
            let late = at(if map[1][0] == 4 { 6.0 } else { 4.0 });
            assert_eq!(
                late.now(&mapped(map.clone())),
                Some((late.start, None)),
                "{map}"
            );
        }
        // A span over a cut, from one clip into the next, keeps its end there.
        fs::write(
            p.root.join(&p.timeline),
            pretty(&json!({
                "canvas": { "width": 640, "height": 360, "fps": 30 },
                "tracks": { "visual": [{ "clips": [
                    { "id": "a", "kind": "video", "src": "x.mp4", "start": 0, "duration": 2 },
                    { "id": "b", "kind": "video", "src": "x.mp4", "start": 2, "duration": 2 }
                ] }] }
            })),
        )
        .unwrap();
        let cut = p
            .add_note(NewNote {
                start: 1.0,
                end: Some(3.0),
                ..new("the cut")
            })
            .unwrap();
        let now = p.document(&p.timeline).unwrap();
        assert_eq!(cut.targets[0].source_end, None);
        assert_eq!(cut.now(&now), Some((1.0, Some(3.0))));
        // A solid plays no source: its note keeps the offset.
        let solid = json!({ "canvas": { "width": 640, "height": 360, "fps": 30 },
            "tracks": { "visual": [{ "clips": [{ "id": "shot", "kind": "solid", "color": "#000000ff",
                "start": 1, "duration": 8 }] }] } });
        assert_eq!(second.now(&doc(solid)), Some((4.0, None)));
    }

    #[test]
    fn a_note_keeps_the_timeline_it_is_on_and_the_sources_seen() {
        let p = project("seen");
        let other = "cut-v2.timeline.json";
        let text = json!({ "canvas": { "width": 640, "height": 360, "fps": 30 }, "tracks": {} });
        fs::write(p.root.join(other), pretty(&text)).unwrap();
        let note = p
            .add_note(NewNote {
                file: other.into(),
                ..new("on the other cut")
            })
            .unwrap();
        let kept = p
            .root
            .join(format!(".valle/history/{}/files", note.snapshot));
        assert!(kept.join(other).is_file());
        // A file seen one way and changed since is not noted on.
        let sha = crate::digest(&p.root.join(other)).unwrap();
        let changed = json!({ "canvas": text["canvas"], "tracks": { "visual": [] } });
        fs::write(p.root.join(other), pretty(&changed)).unwrap();
        let err = p
            .add_note(NewNote {
                file: other.into(),
                seen: vec![(other.into(), sha)],
                ..new("seen before")
            })
            .unwrap_err();
        assert!(err.contains("changed after the picture"), "{err}");
        assert_eq!(p.notes().unwrap().len(), 1);
        // Refused, it saved no snapshot.
        assert_eq!(p.history().unwrap().len(), 1);
        let now = crate::digest(&p.root.join(other)).unwrap();
        p.add_note(NewNote {
            file: other.into(),
            seen: vec![(other.into(), now)],
            ..new("seen as it is")
        })
        .unwrap();
        // A module read and moved since is gone from the note's snapshot.
        fs::write(p.root.join("theme.ts"), "export const T = 1;\n").unwrap();
        let theme = crate::digest(&p.root.join("theme.ts")).unwrap();
        fs::rename(p.root.join("theme.ts"), p.root.join("theme-old.ts")).unwrap();
        let err = p
            .add_note(NewNote {
                file: other.into(),
                seen: vec![("theme.ts".into(), theme)],
                ..new("moved")
            })
            .unwrap_err();
        assert!(err.contains("theme.ts is gone"), "{err}");
        // A video read in parts is known by its version, and replaced it is another.
        let footage = json!({ "canvas": text["canvas"], "resources": { "shot": "media/shot.mp4" },
            "tracks": { "visual": [{ "clips": [{ "id": "s", "src": "shot", "start": 0, "duration": 1 }] }] } });
        fs::write(p.root.join(other), pretty(&footage)).unwrap();
        fs::write(p.root.join("media/shot.mp4"), "blue").unwrap();
        let blue = crate::version(&p.root.join("media/shot.mp4")).unwrap();
        let on_video = |text: &str| {
            p.add_note(NewNote {
                file: other.into(),
                seen: vec![("media/shot.mp4".into(), blue.clone())],
                ..new(text)
            })
        };
        on_video("blue").unwrap();
        fs::write(p.root.join("media/shot.mp4"), "red, longer").unwrap();
        let err = on_video("red").unwrap_err();
        assert!(err.contains("media/shot.mp4 changed"), "{err}");
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
                .map(|t| (t.clip.unwrap(), t.offset.unwrap(), t.run))
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
        let clips: Vec<_> = note
            .targets
            .iter()
            .filter_map(|t| t.clip.as_deref())
            .collect();
        assert_eq!(clips, ["line", "over", "back", "music"]);
        assert_eq!(note.targets[0].run, Some(0));
        // What was picked comes first, where it is shown, and the rest after.
        let picked = Target {
            clip: Some("over".into()),
            offset: None,
            source: None,
            source_end: None,
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
        let clips: Vec<_> = note
            .targets
            .iter()
            .filter_map(|t| t.clip.as_deref())
            .collect();
        assert_eq!(clips, ["over", "line", "back", "music"]);
        assert_eq!(
            note.targets[0].offset,
            note.targets[2].offset.map(|o| o - 1.0)
        );
        assert_eq!(note.targets[0].path.as_deref(), Some("card/title"));
    }

    #[test]
    fn a_note_on_a_component_is_on_its_elements_and_no_clip() {
        let p = project("component");
        fs::write(
            p.root.join("title.motion.tsx"),
            "import { K } from './k';\nexport default function Title() { return <View />; }",
        )
        .unwrap();
        fs::write(p.root.join("k.ts"), "export const K = 1;\n").unwrap();
        let element = Target {
            clip: None,
            offset: None,
            source: None,
            source_end: None,
            run: None,
            path: Some("card/title".into()),
            site: Some("title.motion.tsx:6:9".into()),
        };
        let accent = json!({ "accent": "#ff0000" });
        let note = p
            .add_note(NewNote {
                file: "title.motion.tsx".into(),
                targets: vec![element.clone()],
                props: accent.as_object().cloned(),
                ..new("smaller")
            })
            .unwrap();
        assert_eq!(note.file, "title.motion.tsx");
        // Out of components/, it is kept, with what it imports, and played as it was seen.
        let kept = p
            .root
            .join(format!(".valle/history/{}/files", note.snapshot));
        assert!(kept.join("title.motion.tsx").is_file() && kept.join("k.ts").is_file());
        assert_eq!(note.to_json()["props"], accent);
        // A later snapshot keeps it too, while a note is on it.
        fs::write(p.root.join("k.ts"), "export const K = 2;\n").unwrap();
        let (later, _) = p.save(None, "save").unwrap();
        assert!(
            p.root
                .join(format!(".valle/history/{later}/files/k.ts"))
                .is_file()
        );
        assert_eq!(note.targets, std::slice::from_ref(&element));
        let saved = &fs::read_to_string(p.root.join(super::FILE)).unwrap();
        let saved: serde_json::Value = serde_json::from_str(saved).unwrap();
        assert_eq!(saved["notes"][0]["component"], "title.motion.tsx");
        assert!(saved["notes"][0].get("timeline").is_none());
        assert_eq!(p.notes().unwrap(), [note]);
        // A clip is a timeline's; a component that is not there is no component.
        let refused = |n: NewNote| p.add_note(n).unwrap_err();
        assert!(
            refused(NewNote {
                file: "title.motion.tsx".into(),
                targets: vec![Target {
                    clip: Some("a".into()),
                    ..element.clone()
                }],
                ..new("x")
            })
            .contains("of no clip")
        );
        assert!(
            refused(NewNote {
                file: "gone.motion.tsx".into(),
                ..new("x")
            })
            .contains("no component")
        );
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
                file: "nope".into(),
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
        // Through a link out of reviews/, a picture is neither written nor deleted.
        #[cfg(unix)]
        {
            let away = p.root.join("away");
            fs::create_dir_all(&away).unwrap();
            fs::write(away.join("n1.png"), "theirs").unwrap();
            std::os::unix::fs::symlink(&away, p.root.join("reviews/crops")).unwrap();
            let mut linked = note.clone();
            linked["image"] = json!("crops/n1.png");
            fs::write(
                p.root.join(super::FILE),
                json!({ "notes": [linked] }).to_string(),
            )
            .unwrap();
            p.delete_note("n1").unwrap();
            assert_eq!(fs::read_to_string(away.join("n1.png")).unwrap(), "theirs");
            let png = b"\x89PNG\r\n\x1a\n".to_vec();
            let err = p
                .add_note(NewNote {
                    image: Some(png),
                    ..new("x")
                })
                .unwrap_err();
            assert!(err.contains("symbolic link"), "{err}");
            assert_eq!(fs::read_dir(&away).unwrap().count(), 1);
            fs::remove_file(p.root.join("reviews/crops")).unwrap();
        }
        for image in ["crops/../../project.json", "crops/n2.png", "crops/n1.gif"] {
            let mut stray = note.clone();
            stray["image"] = json!(image);
            assert!(
                bad(json!({ "notes": [stray] })).contains("its picture"),
                "{image}"
            );
        }
        // Its picture is named by its id, so the id is a number as notes are given.
        for id in ["../x", "n01", "n0", "x1"] {
            let mut stray = note.clone();
            stray["id"] = json!(id);
            stray["image"] = json!(format!("crops/{id}.png"));
            assert!(bad(json!({ "notes": [stray] })).contains(".id:"), "{id}");
        }
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
