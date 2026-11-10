# Start here

Valle makes video from text files. A **Motion scene** (`*.motion.tsx`) is animated graphics
written as React-style components with CSS and Tailwind. A **Timeline** (`*.timeline.json`)
cuts footage, images, scenes, sound and captions together. A **project** is a folder that
holds a timeline and its files, with snapshots of its sources.

Every command checks before it draws, prints one short result, and takes `--json` for a result
a program reads. Read this guide first, then the section of the guide you need:

```sh
valle docs                                # the guides
valle docs timeline                       # a whole guide
valle docs timeline --section clips       # one section
valle <command> --help                    # a command's options
```

## Choose what to make

| Task | Make | Guide |
| --- | --- | --- |
| A title, an animated graphic, a chart, a whole scene | a Motion scene | `motion` |
| An edit of footage, images, scenes, music and captions | a Timeline | `timeline` |
| An edit kept in a folder, with snapshots and render records | a project | `project` |

A Motion scene can be rendered on its own, or be a clip in a timeline.

## The workflow

1. Write the file, or change it.
2. `check` it. Fix what it reports and check again until it passes.
3. Look at frames: `render --frame N -o frame.png`, or several side by side with
   `--frames 0,30,60 --storyboard -o sheet.png`. You cannot watch a video; open the PNGs.
4. Render the video: `render -o out.mp4`.

```sh
valle motion check title.motion.tsx
valle motion render title.motion.tsx --frames 0,30,60,89 --storyboard -o sheet.png
valle motion render title.motion.tsx -o title.mp4

valle timeline check edit.timeline.json
valle timeline render edit.timeline.json -o edit.mp4
```

A passing check means the file is valid, not that it looks right: look at the frames before
you say it is done.

## A project from start to finish

```sh
valle project create promo --width 1920 --height 1080 --fps 30
cd promo
valle project add ~/footage/interview.mp4 ~/scenes/title.motion.tsx
```

`add` copies each file in and names it in `timeline.json`'s `resources`. Then put clips on
the timeline, by editing `timeline.json` (see `valle docs timeline`) or with
`valle timeline edit`:

```sh
valle timeline edit timeline.json --op '{"op":"add","track":"tracks.visual[0]","clip":{"id":"intro","kind":"motion","component":"title","duration":3}}'
valle project check
valle project render --frame 45 -o frame.png
valle project render
```

`project render` writes the video into `renders/` and, for a full render, saves a snapshot of
the sources first.

## Results

- **Text** (the default): one short line on success, such as
  `60 frames encoded into renders/timeline-1.mp4 in 0.6 s (snapshot 1)`. On failure,
  `error: …` and often `hint: …` on stderr.
- **`--json`**: one JSON object on stdout. Every result has `status`: `ok`, `error`, or
  `partial` when some of the work was done. A failure has `code` (such as `usage`, `scene`,
  `timeline`, `check`, `gpu`, `video`), `message` and, when there is advice, `hint`; a check's
  failure lists each problem in `problems`, with the first frame it is on and how many.
- **`--events`**: lines of JSON as the command goes (`{"event":"progress","done":12,"frames":60}`),
  at most a hundred for any length, then the result. Studio says `ready` with its address and
  keeps running.
- A JSON result leaves out what it has nothing for: a field that would be `null` or an empty
  list is absent.
- **Exit code**: 0 on success, 1 on any failure. Decide by the exit code, then read the result.

## When something fails

- Read the message: it names the file and line, or the JSON path in a timeline, and says what
  the field or tag takes. Do what it says, then run the same command again.
- An option a command does not take, or a value it does not know, is refused with the values
  it does: `valle <command> --help` lists them.
- `gpu`: Valle draws on the GPU, or on the CPU driver it carries where there is none. Report
  the message rather than retrying.
- `video`: video needs FFmpeg 7, 8 or 9 installed; the message says what is missing.

## What to report

Give the files you made and the state you reached: rendered and looked at, rendered but not
looked at, checked only, or blocked, with the message that blocked it.
