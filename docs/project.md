# Project folders

A project is an ordinary folder: a `project.json` naming its entry timeline, the timelines,
the files they use, snapshots of the sources and records of renders. Every project command
works on the project the current folder is in, or the one `-C <folder>` names.

## Layout

```text
promo/
  project.json        { "name": "promo", "timeline": "timeline.json" }
  timeline.json       the entry timeline
  timelines/          more timelines, if any
  components/         Motion scenes
  media/              footage, images, sound and fonts
  renders/            what project render writes
  .valle/             snapshots and render records
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
use that name: `"src": "intro"` for footage, `"component": "title"` for a scene.

## Look and check

```sh
valle project show            # timelines, resources, files out of the project or missing, snapshots
valle project check           # the entry timeline, its files and its components' folders
valle project check timelines/teaser.json --cost   # plan every frame too
```

`check` reports every problem at once; fix them and check again.

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

## Render

```sh
valle project render                          # the entry timeline into renders/
valle project render timelines/teaser.json -o teaser.mov --codec prores
valle project render --frame 120 -o frame.png
valle project render -o mix.wav               # the sound alone
```

Without `-o`, a render goes into `renders/` under a new name. A full render, every frame into
a video, first saves a snapshot and then records beside it which files it read, with their
digests, and what it made; a media file that changed while it was read is reported as a
warning. `project show` gives the last render.

## Studio

`valle project studio` opens the project in Studio, in the browser, served from 127.0.0.1
until stopped; outside a project it lists recent projects. Studio writes files back when you
save. If a person may have Studio open, read a file again before you change it.
