---
name: valle
description: Create, edit, and render videos with the Valle CLI. Use for Valle/OpenValle Motion JSX, timeline or project editing, asset-library work, and local media processing or model setup.
---

# Valle

Use the user's installed Valle executable. Otherwise look for `valle` on PATH or
`dist/bin/valle` in a source checkout (`valle.exe` on Windows). A packaged CLI works
without a source checkout, Rust, or Bun. Check its `--version` and use subcommand
`--help` when an option is uncertain. Uppercase arguments below are task-specific values.

For details beyond this entrypoint, read the checkout's `docs/cli.md` or the
[CLI guide](https://github.com/openvalle/valle/blob/main/docs/cli.md) as needed.

## Choose the command

| User needs | Command group |
| --- | --- |
| Animated titles, graphics, or a scene authored in JSX | `motion check`, `motion render`, `motion studio` |
| Compose or edit footage in a timeline JSON file | `timeline check`, `timeline render` |
| Edit an existing project with saved revisions | `project show`, `apply`, `history`, `restore`, `render`, `studio` |
| Organize, annotate, find, or resolve local assets | `assets add`, `show`, `tag`, `annotate`, `search`, `resolve` |
| Process a media file | `media transcribe`, `matte`, `enhance`, `separate`, `shots`, `segment`, `inpaint`, `upscale`, `interpolate` |
| Prepare weights for a media operation | `models list`, `install`, `verify` |

Use `timeline` for a file-based edit and `project` when the user needs revision
history or is already working in a project. Motion can also be a clip in a timeline.
Media operations produce standalone files; they do not automatically update a
timeline, project, or asset-library analysis.

## Run and read results

- Use `--json` for a single result, or `--events` for progress. They are mutually
  exclusive. Keep stdout separate from stderr; `--help` and `--version` remain text.
- NDJSON events contain `type` and `data`. A finite command ends with `report`;
  its `data` is the same payload used by `--json`. Check the process exit code too.
- Motion/Timeline rendering and Media use `status`; Assets uses `ok` and may have
  per-item failures. Project edits use `outcome`; model listing returns an array.
  Do not assume every successful response has the same envelope.
- Exit 2 means argument/Media input errors; 3 means Media/model dependency or
  verification failures; 4 means Media execution failures; 1 is a general or
  project/asset failure. Read the error and its hint before retrying. Motion compile
  errors include `error.diagnostics` with source spans.
- Render destinations must be new files. Some Media commands support `--overwrite`;
  check their help before replacing output. `separate` always needs a new directory.
- Studio is a persistent local server: consume `ready` with `--events` (or
  `status: "ready"` with `--json`), then use its returned URL. `--port 0` selects a
  free port. Do not wait for Studio to exit as if it were a render job.

## Author Motion

For syntax, component attributes, CSS/Tailwind support, animation, data, text or
effects, read the relevant sections of the checkout's `docs/motion.md` or the
[Motion authoring reference](https://github.com/openvalle/valle/blob/main/docs/motion.md).
Check its limitations before choosing advanced effects or browser-style CSS.

Motion is a restricted JSX language, not a React application. Use Valle's supplied
`Scene`, `View`, `Text`, and animation helpers; do not assume React hooks, browser
APIs, Remotion imports, or arbitrary JavaScript are supported. Start from existing
working source when available. A minimal `title.motion.tsx` is:

```tsx
export default function Title(ctx) {
  const opacity = interpolate(ctx.hold.progress, [0, 0.6], [0, 1]);
  return (
    <Scene className="relative h-full w-full flex items-center justify-center"
      style={{ backgroundColor: "#102030" }}>
      <Text style={{ fontSize: 48, color: "#ffffff", opacity }}>Hello, Valle!</Text>
    </Scene>
  );
}
```

Use literal or immutable compile-time constant interpolation stop arrays.
Bind declared assets using repeated
`--asset name=path`, prepared data with `--data file.json`, and extra fonts with
`--font path`. Bind constant props with `--props props.json` and Timeline-format
source-range cues with `--cues cues.json`; required cues must be supplied, optional
cues stay inactive. Keep canvas size and bindings consistent across checking and rendering;
use the same duration and FPS for preview frames and final export.

```sh
valle motion check title.motion.tsx --size 640x360 --json
valle motion render title.motion.tsx --duration 3 --fps 30 --size 640x360 --frame 45 -o title.png --json
valle motion render title.motion.tsx --duration 3 --fps 30 --size 640x360 -o title.mp4 --events
```

`--frame` is zero-based and writes PNG; omitting it writes MP4. `--size` controls
layout; `--output-size` scales delivery. `--backend raster` selects CPU composition,
while `auto` may select Metal on macOS. Compositor selection and video encoding
are independent; `--hardware-encode` explicitly requires a supported encoder.

For new visuals, inspect a representative PNG before a long export. Fix source
diagnostics or visible layout problems before spending time tuning workers.

## Edit Timeline and Project

For the JSON format, time domains, track ordering, audio, captions, keyframes or
Motion bindings, read the relevant sections of the checkout's `docs/timeline.md`
or the [Timeline authoring reference](https://github.com/openvalle/valle/blob/main/docs/timeline.md).
Use its tested examples and current CLI limitations when choosing a representation.

Public timeline JSON has `canvas`, optional named `resources`, and `tracks`.
It has no root `version` field. Times are seconds; size and FPS live in `canvas`.
For a three-second input video, this is a minimal document:

```json
{
  "canvas": { "width": 640, "height": 360, "fps": 30 },
  "resources": { "footage": "input.mp4" },
  "tracks": { "visual": [{ "clips": [
    { "kind": "video", "src": "footage", "start": 0, "duration": 3 }
  ] }] }
}
```

`start` places a clip on the output timeline; `trimStart` selects its source offset.
Use the actual source duration. Visual video clips play source audio automatically;
`gain` defaults to 1, 0 mutes, and values above 1 amplify. Add `tracks.audio` for
music or independent sound; mute the video's gain when replacing its original
sound to avoid doubling it. `size: [width,height]` sets a target in canvas pixels,
defaulting to the canvas; `fit: "cover"` fills it. Rotation uses degrees.
Caption presets use objects such as `"enter": {"preset":"fade","duration":0.3}`;
duration is optional. `kind: "solid"` generates a color; `image` and
`video` refer to named resources through `src`. A Motion clip uses
`kind: "motion"` and `component: "title"`, with the JSX path in `resources.title`.
Resolve local resource paths relative to the timeline file. Validate with
`valle timeline check timeline.json --json`, then render with
`valle timeline render timeline.json -o output.mp4 --events`.
`--frame N -o frame.png` selects a frame. MP4 requires an opaque canvas background;
PNG can preserve transparency. Motion's tuning flags are not exposed on Timeline/Project render.

For an existing project:

```sh
valle project show PROJECT_ID -o edit.timeline.json --json
# Edit the exported complete timeline; use the revision returned by show.
valle project apply PROJECT_ID --base-revision REVISION --timeline edit.timeline.json --json
valle project render PROJECT_ID -o output.mp4 --events
```

Create a project only when needed with `project create PROJECT_ID --timeline FILE`.
Submit a complete document, not a patch. `committed` and `unchanged` are successful
outcomes. On `staleBase`, fetch the latest snapshot and reconcile the requested
change; do not blindly resubmit the old document with a newer revision number.
`restore --base-revision CURRENT --revision OLD` appends a revision rather than
erasing history. Imported relative resource paths become absolute; their files
must remain available.

## Work with Assets and Media

Import selected files with `assets add FILE --mode copy --json`. Repeated imports
reuse the content ID. Read IDs from `data.items[].content_digest`; pass a full ID
or unique prefix to `show`, `tag`, `annotate`, or `resolve`. Use `assets search QUERY
--json` to find matches, then `assets resolve ID --json` when a file path is needed.
Analysis is explicit; importing/tagging/searching does not require semantic models.
`VALLE_HOME` selects project/library storage, so retain it when editing existing data.

Before media decoding or MP4 export, `valle media capabilities --json` can check
the installed FFmpeg libraries and encoders. FFmpeg 7/8/9 shared libraries are
optional runtime dependencies; a standalone `ffmpeg` executable is insufficient.
Use `--ffmpeg-dir PATH` or `VALLE_FFMPEG_DIR` for a custom installation. Pure Motion
checks, PNG rendering, and Studio startup work without FFmpeg. Software H.264
export needs `libx264`; hardware availability must be checked on the actual host.

For model-backed work, inspect `models list --json` and the chosen Media
subcommand's help. Install only needed missing models using the error's install
hint. Verify the exact `artifact` returned by installation: without `--artifact`,
`models verify` also checks uninstalled variants and can report failure despite a
usable installation. `VALLE_MODEL_CACHE` selects model storage independently of
`VALLE_HOME`. ONNX adapters need ONNX Runtime 1.28.x, selected by `ORT_DYLIB_PATH`
or a colocated runtime library; installing weights does not install that runtime.

Use `media enhance input.wav -o clean.wav --events` for speech enhancement, for
example. Add `--report run.json` when a persisted processing report is useful.
Transcription defaults to word timing and needs both ASR and alignment models;
`--text-only` skips alignment and cannot be combined with `-o`. ASR currently runs
on macOS/Linux with `--backend auto`. Before constructing Media inputs, read the
relevant operation in the checkout's `docs/media.md` or the
[Media processing reference](https://github.com/openvalle/valle/blob/main/docs/media.md).
It covers model choices, prompt/mask/shot-list formats, output timing and audio
behavior, and integration with Timeline/Motion. Use its primary artifact formats
when chaining operations; a run report is not a shot list or transcript.

Deliver the requested artifact paths and relevant result details. Distinguish
completed processing from a missing dependency, a started Studio, or an untested
render; a successful check alone does not prove an export succeeded.
