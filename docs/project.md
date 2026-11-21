# Project folders

A project is an ordinary folder: a `project.json` naming its entry timeline, the timelines,
the files they use, snapshots of the sources and records of renders. Every project command
works on the project the current folder is in, or the one `-C <folder>` names.

## Layout

```text
promo/
  project.json          { "name": "promo", "timeline": "main.timeline.json" }
  main.timeline.json    the entry timeline
  timelines/            more timelines, if any
  components/           Motion scenes
  media/                footage, images, sound and fonts
  renders/              what project render writes
  reviews/              review notes and the pictures they were written on
  .valle/               snapshots and render records
```

## Create and add files

```sh
valle project create promo                          # 1920×1080 at 30 fps
valle project create reel --width 1080 --height 1920 --fps 30
valle project add intro.mp4 logo.png music.wav      # copied into media/
valle project add title.motion.tsx                  # copied into components/
valle project add /Volumes/shoot --link                # left where they are
```

`add` copies each file in, or with `--link` refers to it where it is, and names it in the
entry timeline's `resources`, by its file name without the extension (`title.motion.tsx` is
`title`; a second `title` is `title-2`). A file already in the project is reused. Clips then
use that name: `"src": "intro"` for footage, `"component": "title"` for a scene. When
copying a component from another project, add its folder if it needs accompanying files.

For local video and audio files, `add` and `show` include `media`: source `duration` in
seconds, display `width` and `height` after rotation, rational `fps` as text (for example
`"30000/1001"`), and `audio` indicating whether an audio stream exists. These are source
facts, not the timeline canvas. Images also report their intrinsic dimensions when present.
Unknown or inapplicable values are `null`. A failed probe
adds `media.error`; the import and the rest of `show` remain usable. Remote files are not
downloaded just to display their metadata.

## Look and check

```sh
valle project show            # timelines, resources, files out of the project or missing, snapshots
valle project check           # the entry timeline, its files and component dependencies
valle project check timelines/teaser.timeline.json --cost   # plan every frame too
```

`check` reports missing files and local component dependencies outside the project. Components can
import shared modules and use assets anywhere inside the project. The command validates the timeline and requires
an audio stream for audio-track sources, including nested timelines. It warns about tagged
HDR footage. `--cost` also checks every frame and retains those warnings in both text and
`warnings` in JSON. Fix reported problems and check again.

## Snapshots

```sh
valle project save -m "rough cut"
valle project history
valle project restore 2
```

A snapshot, in `.valle/history/<n>/`, keeps the sources as they were: `project.json`, the
timelines, everything in `components/`, and the data and shaders in `media/`. Videos, sounds,
images and fonts are only listed with their SHA-256, so a snapshot says whether they changed
but does not keep them. `restore` puts a snapshot's sources back after saving what they
replace as a new snapshot, so nothing is lost; files added since are kept, and media is never
written.

## Notes

A note is what someone reviewing the edit asks to change: at a frame, or over a span with
`--end`, and optionally about a part of the picture and the clips there. Notes are kept in
`reviews/notes.json`, with the picture seen in `reviews/crops/`, apart from the timelines:
writing one never changes the edit, and they are not part of any snapshot.

```sh
valle project notes                                   # every note, where it is now
valle project notes list --open
valle project notes show n3                           # with all that was said
valle project notes add "logo over the caption" --at 8.4 --region 0.62,0.08,0.3,0.18 --clip shot-2@1.4
valle project notes reply n3 "which logo?" --by ai
valle project notes resolve n3 "moved the logo down left, clear of the captions"
valle project notes reopen n3 "it still covers the second line"
valle project notes delete n3
```

`--region` is `x,y,w,h`, each a fraction of the canvas from its top left. `--clip id@seconds`
names a clip the note is about and how far into it; it can be given more than once. Without
`--clip`, the note is about the clips shown at `--at`, the one on top first: captions, then
the picture's clips from the top track down, then, for a note on no part of the picture, the
sound. A caption's `targets` entry has the timed `run` shown then, from 0. A note moves with
the first of its clips that is still there, and is listed as `gone` once every one of them
is deleted or trimmed past that point; a note on no clip stays where it was written. In JSON,
each note has `now`, where it is in the timeline as it is now, and `picture`, its picture as
a path in the project.

In Studio, pause and press N (or the note button) to note the frame shown, dragging over the
picture to mark a part of it; Shift-drag over the ruler to note a span. A click on the
picture picks the element drawn there, as `valle timeline pick` does, and a click again the
one beneath; the note is then on its clip first, with the element's key `path` in the clip's
Motion component and the `site` of its JSX, `file:line:column`, which opens in the source.
Notes are marked on the ruler and listed under Notes, where a note is answered, resolved or
reopened.

Every message of a note, the note first, records the snapshot it was written against: when
the sources changed since the last snapshot, one is saved first. `resolve` records the
snapshot that resolved it as `resolvedIn`, so what changed in between is what was done about
it. Replies are by `user` unless `--by` says, and resolutions by `ai`.

To work through the open notes: list them, look at each picture and read each note, find its
clips with `valle timeline query --clip` or `--at`, make the change, render the frames it is
at to look, then resolve it saying what was done. Ask with `reply` where a note is unclear,
and leave it open.

## Render

```sh
valle project render                          # the entry timeline into renders/
valle project render timelines/teaser.timeline.json -o teaser.mov --codec prores
valle project render --frame 120 -o frame.png
valle project render --fps 60 --output-size 3840x2160
valle project render --storyboard --samples 60 --per-page 12
valle project render -o mix.wav               # the sound alone
```

Without `-o`, a render goes into `renders/` under a new name. A full render, every frame into
a video, first saves a snapshot and then records beside it which files it read, with their
digests, and what it made; a media file that changed while it was read is reported as a
warning. `project show` gives the last render.

`--fps` changes this export's sampling rate without modifying the timeline. It also applies
to frame images and storyboards, and is saved in a full video's render record.

`--storyboard` samples up to 12 frames by default. `--samples`, `--step`, `--frames` and
`--per-page` work as in Motion (see `valle docs motion storyboards`). Without `-o`, a
storyboard uses `renders/<timeline>-storyboard.png`, or numbered pages such as
`renders/<timeline>-storyboard-001.png`. Repeated renders choose a new base name even when
the previous storyboard has only numbered pages. An explicit `-o` must end in `.png`.

## Studio

`valle project studio` opens the project in Studio, in the browser, served from 127.0.0.1
until stopped; outside a project it lists recent projects. Studio writes files back when you
save. If a person may have Studio open, read a file again before you change it.
