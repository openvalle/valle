---
name: valle
description: Make, edit and render video with the Valle CLI. Use it for Valle work, such as Motion scenes written in TSX (animated titles, graphics, charts), Timeline JSON edits of footage, images, sound and captions, and Valle project folders with snapshots.
metadata:
  version: "0.1.3"
---

# Valle

Use the user's `valle`: the installed executable, or `valle` on PATH. `valle --version` says
which; this skill is written for 0.1. If there is none, ask the user before installing it,
then run `curl -fsSL https://openvalle.ai/install.sh | sh` on macOS and Linux, or
`irm https://openvalle.ai/install.ps1 | iex` in PowerShell on Windows; it installs to
`~/.valle/bin/valle`. Do not build it from source. When the user asks for a newer Valle, run
`valle update`, and tell them what it says about Studio and this skill.

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

When the user has reviewed an edit and asks you to work through the notes, run
`valle project notes list --open --json`. Each note has `now`, where it is in the timeline
now, or `gone`; `region`, the part of the picture it is about; `targets`, the clips it is on,
the top one first, with the element's `path` and the `site` of its JSX where they picked one;
and `picture`, the frame they saw with the part outlined: look at it. Find the clip with
`valle timeline query`, or what is drawn there with `valle timeline pick`, change the source,
`check`, and render the note's frames to look at the result. Then
`valle project notes resolve ID "what you changed"`. Where a note is unclear, or what it asks
cannot be done, `valle project notes reply ID "your question" --by ai` instead, and leave it
open. Do not delete notes. `valle docs project --section notes` explains them.

Unless the user asks you to change them straight away, propose first: for each open note,
`valle project notes reply ID "what you would change" --by ai`, then stop and tell them the
proposals are in Studio's Notes. Change what they confirm, in the thread or to you, and
resolve it. A note reopened has been looked at and is not right yet: read its whole
`thread`, what was proposed, done and said since, before you propose again.

Report the files you made and how far you got: rendered and looked at, rendered only, checked
only, or blocked with the message that blocked it.
