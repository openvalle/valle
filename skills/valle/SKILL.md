---
name: valle
description: Make, edit and render video with the Valle CLI. Use it for Valle work, such as Motion scenes written in TSX (animated titles, graphics, charts), Timeline JSON edits of footage, images, sound and captions, and Valle project folders with snapshots.
metadata:
  version: "0.1.0"
---

# Valle

Use the user's `valle`: the installed executable, or `valle` on PATH. `valle --version` says
which; this skill is written for 0.1. If there is none, ask the user to download it from
https://github.com/openvalle/valle/releases and put it on PATH; do not build it from source.

Its guides are built in and match its version; read them instead of guessing a field or a
function:

```sh
valle docs start                          # read this first: the workflow and how results look
valle docs timeline --section clips       # then the section a task needs
valle docs motion --section animation
valle <command> --help                    # a command's options
```

Choose the file for the task. A graphic, a title or a chart alone is one `*.motion.tsx`. An
edit with footage, music or several scenes is a project: `valle project create NAME`, then
`valle project add FILES`, with `main.timeline.json` as its entry. A timeline on its own is a
`*.timeline.json`. Imported helper components keep `.tsx`, modules `.ts`, and data `.json`.

Work in this order: write the file, `check` it until it passes, look at frames
(`render --frame N -o f.png`, or `--frames 0,30,60 --storyboard -o sheet.png`), then render
the video. Pass `--json` and decide by the exit code; on failure read `message` and `hint`,
which name the field or line and what it takes, fix that, and run the same command again.

Do not retry these; tell the user:

- `video`: rendering video needs FFmpeg 7, 8 or 9 as shared libraries, which the message
  explains. On macOS that is `brew install ffmpeg`; elsewhere, `valle docs cli --section ffmpeg`.
  Frames and storyboards render without it.
- `gpu`: report the message.

To let the user watch or adjust the work, give them `valle project studio` in a project, or
`valle motion studio FILE` and `valle timeline studio FILE`. Studio writes files when they
save: read a file again before you change it.

Report the files you made and how far you got: rendered and looked at, rendered only, checked
only, or blocked with the message that blocked it.
