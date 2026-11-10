---
name: valle
description: Make, edit and render video with the Valle CLI. Use for Valle work: Motion scenes written in TSX (animated titles, graphics, charts), Timeline JSON edits of footage, images, sound and captions, and Valle project folders with snapshots.
---

# Valle

Name Motion entries `*.motion.tsx` and timelines `*.timeline.json`. Imported helper components
keep `.tsx`, modules `.ts`, and data `.json`. A new project's entry is `main.timeline.json`.

Use the user's `valle`: the installed executable, or `valle` on PATH. Its guides are built in
and match its version; read them instead of guessing a field or a function:

```sh
valle docs start                          # read this first: the workflow and how results look
valle docs timeline --section clips       # then the section a task needs
valle docs motion --section animation
valle <command> --help                    # a command's options
```

Work in this order: write the file, `check` it until it passes, look at frames
(`render --frame N -o f.png`, or `--frames 0,30,60 --storyboard -o sheet.png`), then render
the video. Pass `--json` and decide by the exit code; on failure read `message` and `hint`,
which name the field or line and what it takes, fix that, and run the same command again.

Report the files you made and how far you got: rendered and looked at, rendered only, checked
only, or blocked with the message that blocked it.
