---
name: valle
description: Create, edit and render video with the Valle CLI. Use for Valle or OpenValle work, including Motion scenes written in TSX, Timeline JSON edits, project folders and source snapshots, the local asset library, and local media processing such as transcription, voice cloning, matting and upscaling, with its model setup.
---

# Valle

Use the user's `valle`: the installed executable, `valle` on PATH, or `dist/bin/valle` in a
source checkout (`valle.exe` on Windows). Before writing input, read the relevant sections of
the task's guide from that executable, so they match the installed version. The guides are
long; save one and search it:

```sh
valle docs motion > motion.md # or timeline, project, assets, media, cli
valle motion render --help
```

## Choose the command

| Task | Commands | Guide |
| --- | --- | --- |
| Make animated titles, graphics, charts or a whole scene | `motion check`, `review`, `render`, `studio` | `motion` |
| Cut footage, images, Motion, audio and captions together | `timeline check`, `render`, `studio` | `timeline` |
| Work on a project folder and keep source snapshots | `project create`, `add`, `show`, `check`, `save`, `history`, `restore`, `render`, `studio` | `project` |
| Find or organize local media | `assets add`, `search`, `show`, `edit`, `tag`, `annotate` | `assets` |
| Process a media file | `media transcribe`, `synthesize`, `matte`, `enhance`, `separate`, `shots`, `segment`, `inpaint`, `upscale`, `interpolate`, with weights from `models` | `media` |

Use `timeline` for a standalone file and `project` when the user wants snapshots or already
has a project. A Motion scene can be a clip in a Timeline. Media commands write standalone
files; they never change a timeline, a project or the asset library.

## Run commands and read results

- Pass `--json` for one result, or `--events` for NDJSON progress that ends in a `report`
  event carrying the same payload. Parse stdout; diagnostics go to stderr.
- The exit code decides success: 0 ok; 1 a general, project or asset failure; 2 invalid
  arguments or media input; 3 a missing media or model dependency; 4 a failed media operation.
- Checks, renders and project operations report `status`; batch project imports can report
  `partial`. Assets report `ok`, and media returns a `valle.media-run` envelope.
- On failure, read `error.code`, the message and its `hint:` before retrying.
  `motion_compile_failed` lists `error.diagnostics` with source spans; a failed render names
  its frame.
- Outputs must be new paths: choose a new name rather than deleting a file. Only some media
  commands take `--overwrite`.
- Studio is a server. Read its URL from the `ready` event (`status: "ready"` with `--json`),
  pass `--port 0` for a free port, and do not wait for it to exit.

## Motion

Write Motion as frontend code: React-style components, CSS and Tailwind classes in a
`.motion.tsx` file. It is compiled, not run in a browser, and each frame is a pure function
of its time and inputs.

```tsx
export const composition = { width: 1920, height: 1080, fps: 30, duration: 4 };

export default function Title() {
  const { seconds } = useCurrentTime();
  const reveal = interpolate(seconds, [0, 0.8], [0, 1], { easing: "easeOutCubic" });
  return (
    <Scene className="relative h-full w-full flex items-center justify-center"
      style={{ backgroundColor: "#102030" }}>
      <Text style={{ fontSize: 96, fontWeight: 700, color: "#ffffff", opacity: reveal,
        translate: point(0, (1 - reveal) * 40) }}>
        Hello, Valle!
      </Text>
    </Scene>
  );
}
```

- Components take `(props)`. Read the time in seconds with `useCurrentTime()` (`seconds`,
  `progress`), the size, duration and frame rate with `useComposition()`, and prepared data
  with `useData()`. Write animation in seconds; `--fps` changes the frame count, not the
  speed. Built-ins need no import;
  `import { … } from "valle"` and relative imports of local modules work, npm packages do not.
- Nothing persists between frames and nothing runs after one: there is no `useState`,
  `useEffect`, `useRef`, DOM, `fetch`, `Date` or `Math.random`. Compute values from the time,
  use `seededRandom` or `wiggle` for randomness, and declare files as assets.
- A body runs once, while the scene is prepared. `let`, `if`/`else`, early `return` and loops
  work, but a loop's condition cannot depend on the frame, and both sides of a frame-dependent
  `if` stay in the scene: give them different keys, and `position: absolute` when they share
  a place.
- Lists come from prepared arrays: `.map` or `Array.from(source, fn)`, with a stable `key` per
  item. To show items over time, map the whole array and decide each item per frame. A list that
  sets the number of nodes comes from a module `const` or `useData()`, not from props.
- `interpolate` takes stops written as literals or constants. For a destination that depends
  on a prop, interpolate from 0 to 1 and multiply.
- For visual movement, keep layout positions fixed and animate `translate: point(dx, dy)`
  with fractional offsets. Animating `left`/`top` or margins can produce pixel steps in
  slow moves; do not round the offsets. Move a card and its attached decorations together.
  Animate layout only when wrapping, sibling placement or box size should change.
  `motionPath`/`follow` and `layoutTransition: flip(...)` already own movement: do not add
  translation on the same node. See "Smooth movement" in `valle docs motion`; if a move
  steps, inspect consecutive frames before changing FPS or adding blur.
- The compiler rejects what it cannot render and usually names what to write instead; apply
  that. Check unfamiliar CSS against the guide's "Unsupported CSS" section.

Work in this order. You cannot watch a video, so look at the frames:

```sh
valle motion check scene.motion.tsx --json                          # compile, draw frame 0
valle motion render scene.motion.tsx --storyboard sheet.png --json  # up to 12 frames in one PNG
valle motion review scene.motion.tsx --json                         # strobing, unreadable or overlapping text
valle motion check scene.motion.tsx --cost --output-size 3840x2160 --json # before a long or 4K export
valle motion render scene.motion.tsx -o scene.mp4 --events
```

Bind inputs the same way for every command: `--props props.json` for constant prop values,
`--data data.json` for prepared data, `--asset name=path` for an asset declared in
`controls.assets` and read as `asset://name`, and `--font path`. `--frame N` (zero-based)
writes one PNG, and `--frames 0,30,59` selects frames for a PNG pattern or a storyboard; `.mov`
keeps alpha. `--output-size` scales delivery without changing layout, `--fps` overrides the
composition's, and `--backend raster` forces CPU composition (`auto` uses Metal on macOS).

## Timeline and Project

A Timeline document has `canvas`, optional named `resources` and `tracks`, and nothing else;
unknown fields are rejected. Times are in seconds:

```json
{
  "canvas": { "width": 1920, "height": 1080, "fps": 30 },
  "resources": { "title": "title.motion.tsx", "footage": "input.mp4" },
  "tracks": {
    "visual": [{ "clips": [
      { "kind": "motion", "component": "title", "start": 0, "duration": 3 },
      { "kind": "video", "src": "footage", "start": 3, "trimStart": 2, "duration": 5 }
    ] }]
  }
}
```

- Resource paths resolve from the Timeline file's directory; clips refer to them by alias.
- `start` places a clip in the output and `trimStart` picks the source offset; stay within the
  source's real duration. Clips on a track run in order without overlapping, except across a
  visual `transitions` entry. Put simultaneous clips on separate tracks; later visual tracks
  paint on top.
- A visual clip's `kind` is `video`, `image`, `solid` or `motion`. A video plays its own
  sound; set its `gain` to 0 when a clip in `tracks.audio` replaces it. A caption track needs
  a font file among the resources.
- MP4 needs an opaque `canvas.background`, the default; PNG frames and `.mov` keep transparency.

Validate with `valle timeline check FILE --json`, then render with the same `-o`, `--frame`,
`--frames` and `--storyboard` options as Motion. `timeline studio FILE` writes the file back
on Save.

For a project, edit the files directly:

```sh
valle project create promo --json
valle project add /path/to/title.motion.tsx -C promo --json
# Add the returned resource entries and clips to promo/timeline.json.
valle project check -C promo --json
valle project save -m "First cut" -C promo --json
valle project render -C promo --events
```

`add` copies files but does not edit the Timeline. `--link` returns an external reference;
otherwise the project contains independent copies. Commands discover `project.json` above
the working directory or use `-C DIR`. Snapshots include reachable source files and media
digests; drafts and binary preimages need Git or a separate backup. `restore N` backs up
sources before overwriting them, preserves new files and does not restore media. Full
exports automatically save a snapshot and input/output records; media changes during
rendering produce warnings. When Studio is open, reread a file before changing it so an
external edit does not overwrite a person's newer work. `VALLE_HOME` selects the global
asset library and recent-project list; projects live in their own directories.
Studio is at `/`. `project studio` outside a project opens recent projects and creation.
Its Assets panel imports files and edits library tags/notes; dragging into the Timeline
imports all files before saving one draft. Failed imports leave the Timeline unchanged,
and undo leaves copied files in place. History and native project Exports operate only
on the entry Timeline, saving dirty drafts first. Other Timeline files retain file export.
Motion's **Save as project** copies the component and media into a new project.

## Assets and Media

- `assets add FILE --to DIR --tag TAG --note TEXT --json` copies reusable files into
  the global library. Commands use library-relative paths, such as `sfx/whoosh.wav`;
  `items[].absolutePath` can go directly into `project add`. `assets search QUERY --json`
  searches names, directories, tags, notes, and font names. `assets annotate PATH --at 45
  --text TEXT` or `--range 1,3` records selection hints. Read those notes to choose clip trims;
  project imports contain files only. Use `media` tools on the absolute path for analysis.
  Repeated adds deduplicate content and merge knowledge. Inspect current state with `show`
  before retrying an interrupted edit or remove. Missing files retain notes until removed.
  If the primary catalog cannot open, use `assets recover` to list backups, close Studio and
  other library commands, then restore a chosen backup; do not delete the database.
- `valle media capabilities --json` shows the FFmpeg libraries in use. For model-backed work,
  read `models list --json` and install only what a command's error names, with
  `models install ID`. Verify the `artifact` that install returned with
  `models verify ID --artifact ARTIFACT`; without `--artifact`, uninstalled variants fail it.
  ONNX models also need ONNX Runtime 1.28, beside `valle` or named by `ORT_DYLIB_PATH`.
- Chain operations through their `-o` artifacts, such as a `shots` list into
  `interpolate --shots` or a `segment` mask into `inpaint --mask`. A `--report` file is a run
  report, not a transcript or a shot list. The Media guide shows how a transcript becomes
  captions and a shot list becomes cuts.

## Report back

Give the output paths and the details that matter, and say which state was reached: finished,
blocked on a dependency, Studio running, or not rendered yet. A passing check does not prove
that an export works, and a render nobody has looked at is not verified.
