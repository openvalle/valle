# CLI guide

Valle has six workflow groups, plus `docs` for bundled guides and `licenses` for
dependency notices. Commands below use `valle` on PATH. A packaged executable needs
no source checkout; for a source build, use `cargo xtask build --release` and see
[build requirements](../README.md#requirements).

| Group | Use it for | First command |
| --- | --- | --- |
| `motion` | Author animated graphics in JSX, check, preview, render | `valle motion check examples/hello.motion.tsx` |
| `timeline` | Validate, edit and render a timeline document | `valle timeline studio examples/timeline.json` |
| `project` | Keep timeline revisions, apply edits, restore, preview, render | `valle project create demo --timeline examples/timeline.json` |
| `assets` | Import, organize, annotate and search local media | `valle assets add cover.png --tag demo` |
| `media` | Process individual media files with local models | `valle media enhance speech.wav -o clean.wav` |
| `models` | Discover, install and verify model weights | `valle models list` |

For a source build, add `dist/bin/valle` to PATH from the repository root. Skip the
first line if your installed executable is already on PATH. Create an isolated
demo directory for the examples:

```sh
export PATH="$PWD/dist/bin:$PATH"
demo_dir=$(mktemp -d)
export VALLE_HOME="$demo_dir/library"
```

`VALLE_HOME` contains the asset library and project history (normally `~/.valle`).
It does not relocate model weights. `VALLE_MODEL_CACHE` selects the model directory;
otherwise models use the platform cache directory, or `$VALLE_CACHE_DIR/models`.
Each demo uses new output paths. Keep `demo_dir` for the following sections.

## Read bundled documentation

```sh
valle docs
valle docs cli
valle docs motion
valle docs timeline
valle docs media
valle docs project
valle docs assets
valle docs motion > motion.md
valle docs project --json
```

`docs` without a topic lists the bundled guides. With a topic it writes the original
Markdown to stdout, including examples and tables. Use a pager such as
`valle docs motion | less` where available, or redirect to a file. Documentation is
embedded at build time and follows the installed binary; it needs no checkout,
network, FFmpeg, model weights or runtime extraction. Related guides can be read
through their topic names; Markdown source links are preserved as authored.
Model setup is covered in the Media and CLI guides.

`docs --json` returns `status`, executable `version` and `topics` (each with
`topic` and `title`). `docs TOPIC --json` returns `status`, `version`, `topic`,
`title`, `format: "markdown"` and the complete `content` string. `--events`
wraps the same result in one final `report` event. Unknown topics are argument
errors (exit 2); help lists the accepted names. `docs --help` remains plain text.

## Motion: JSX to video

See the [Motion authoring reference](motion.md) for supported JSX, components, CSS,
animation helpers, assets and complete scene examples.

```sh
valle motion check examples/hello.motion.tsx
valle motion review examples/hello.motion.tsx --json
valle motion render examples/hello.motion.tsx \
  --backend raster -o "$demo_dir/hello.mp4" --events
valle motion render examples/hello.motion.tsx \
  --frame 45 -o "$demo_dir/cover.png"
valle motion studio examples/hello.motion.tsx
```

Studio prints a local URL and keeps running; stop it with Ctrl-C. `--port 0` asks
the OS for a free port. Studio uses the Web resources embedded by `cargo xtask build`. `valle licenses` displays the embedded dependency notices and source links.

`motion review --json` checks every output frame using final screen positions after layout and
transforms. A `strobe` warning names a visible painted node that moves over a quarter of its
extent along the travel direction per frame for
at least three consecutive frame intervals without an active motion blur filter. A single
discontinuous jump is excluded from this sustained-motion rule. The report includes the frame
range and peak displacement. Content whose square-root area times opacity is below 4 px is ignored.
Batch strobe warnings are grouped, with `affectedRows` and up to three `exampleKeys`. A `text_readability` warning names a text node that stays visible
for less than 0.3 seconds plus 0.3 seconds per word or 0.225 seconds per CJK character. It reports the visible frame range,
duration, word count and recommended duration; hidden or fully offscreen text is excluded.
`text_out_of_frame` reports visible text ink extending beyond the canvas, and `text_overlap`
reports two visible text ink bounds intersecting by more than 1 px in both directions. Consecutive
frames are combined into one issue. The informational `motion_while_reading` hint reports a
prominent moving subject during readable text, and `simultaneous_main_actions` reports two
separate prominent subjects moving together. These hints require at least three consecutive
intervals; subjects below 1% of the canvas area or moving at most 4 px per frame are ignored.
Area is measured inside the output frame, so a mostly offscreen shape is not counted as a main action.
The informational `linear_motion` hint reports a visible, prominent subject traveling at nearly
constant speed for at least eight frame intervals and at least 10% of the shorter canvas edge.
Small motion, motion-blurred subjects, and motion with changing speed are excluded.
`stagger_timing` is an informational hint for three to 64 consecutively numbered painted
nodes sharing a key prefix. It compares their observed movement starts and reports gaps
outside 30–100 ms; a simultaneous group, missing starts, or mixed start order is excluded.
An empty `issues` array means none of these rules detected a problem.
Pass `--trajectories` to include each visible painted node's per-frame screen anchor and
bounds in JSON. Consecutive samples also include velocity in pixels per second and
acceleration in pixels per second squared; both reset after a hidden or offscreen gap and at particle rebirth.
Pass `--trajectory-sheet review.png` to render up to 12 sampled frames as a 320 px contact
sheet with every visible node's screen path and a marker at each sampled frame. The JSON report
includes frame keys and a node-to-color legend; the PNG is written only after rendering succeeds.
By default the budget covers the complete composition. Pass `--max-frames N` to impose a limit; review fails
when that explicit budget is too small instead of silently skipping frames. It accepts the same `--fps`,
`--props`, `--data`, `--asset`, and `--font` inputs as other Motion authoring commands.

For audio-driven Motion, declare an audio asset and bind it with `--asset beat=beat.wav`.
`const BEAT = audioAnalysis("asset://beat", { bands: 8, fps: 30 })` prepares a frozen table;
`BEAT.level(ctx.seconds)`, `BEAT.band(3, ctx.seconds)`, `BEAT.onset(ctx.seconds)`, and
`BEAT.beatPhase(ctx.seconds)` read it at frame time. `fps` defaults to an integer
`composition.fps` when omitted. Analysis uses frozen decoded 48 kHz mono PCM, accepts 1–32
logarithmic bands and 1–120 analysis frames per second, and limits each table to 16,384 frames.

`--frame` selects a zero-based frame and requires a `.png` output. Full exports
accept H.264 `.mp4`, transparent qtrle or ProRes 4444 `.mov`, or a PNG filename pattern such as
`-o 'frames/%05d.png'`. `.mov` defaults to lossless qtrle; use `--codec prores4444`
for ProRes 4444. PNG and MOV retain alpha; Motion MP4 uses a black output background.
Transparent MOV currently delivers video only; a Timeline with audio is rejected
instead of silently dropping its audio.

`--frames 0,30,59` selects exact frame keys for a PNG sequence or
`--storyboard sheet.png`. A sheet needs no `-o`; providing a PNG pattern and sheet
together saves both from one rendering pass. Without `--frames`, a sheet uses up
to 12 evenly spaced keys including the first and last. Cells keep the delivery
resolution in at most four columns and preserve transparency. Sequence filenames
use original frame keys, not consecutive selection indices. Parent directories
are created; existing outputs are never replaced. Compilation and preparation are
shared across all selected frames. See [Motion delivery](motion.md) for limits.

The entry file declares its base canvas and duration in
seconds: `export const composition = { width, height, duration, fps? }`.
`--fps` overrides the optional file FPS for this invocation. If both are absent,
the command asks for an FPS. `--output-size` scales delivery dimensions without
changing source layout; its aspect ratio must match the composition. Motion in a
Timeline also lays out on its own base canvas and is fitted into the clip size.
`composition` and `--fps` accept a rational rate such as `"30000/1001"`. Bind
declared asset controls with repeated `--asset name=path`, prepared data with
`--data file.json`, and extra fonts with repeated `--font path`.
Use `--props props.json` for constant prop values. `check` accepts the same bindings and fonts, and validates
one Native Raster frame (`--frame 0` by default).

`--workers 1..8` controls frame concurrency for all Motion outputs. H.264 delivery
also accepts `--encode-threads N` or `--hardware-encode`; `--bitrate` requires
hardware encoding. These encoder controls are rejected for PNG and transparent MOV. `--backend auto`
selects an available Metal device on macOS and otherwise Raster; `--backend raster`
explicitly selects CPU composition. See `valle motion render --help` for constraints.

## Timeline: edit a document and render it

See the [Timeline authoring reference](timeline.md) for the JSON format, time and
track rules, footage/audio, captions, keyframes and Motion integration.

The [example timeline](../examples/timeline.json) cuts from blue to teal after
1.5 seconds. It is self-contained and needs no model weights or external assets.

```sh
valle timeline check examples/timeline.json --json
valle timeline studio examples/timeline.json --port 0
valle timeline render examples/timeline.json -o "$demo_dir/timeline.mp4" --events
valle timeline render examples/timeline.json --frame 60 -o "$demo_dir/timeline.png"
```

Canvas size, frame rate, background and clip timing come from the document. PNG
frames and transparent MOV preserve transparency; MP4 delivery requires an opaque canvas
background. Timeline and Project render expose the same `-o`, `--frame`, `--frames`,
`--storyboard` and `--codec` selection as Motion. Motion's delivery tuning flags
are not exposed on those commands.

For actual footage, declare named resources and refer to them from clips. Video
clips play source audio by default; `gain: 0` mutes, `gain: 1` preserves the original
level and larger values amplify. Missing source audio stays silent:

```json
{
  "canvas": { "width": 640, "height": 360, "fps": 30 },
  "resources": { "footage": "hello.mp4" },
  "tracks": {
    "visual": [{ "clips": [
      { "kind": "video", "src": "footage", "start": 0, "duration": 3 }
    ] }]
  }
}
```

Save that document beside `hello.mp4`. Relative resource paths resolve beside the
timeline file, independent of the current working directory. A Motion clip uses
`"kind": "motion", "component": "title"`, with `"title": "hello.motion.tsx"` in
`resources`. `timeline check` validates the document, prepares used resources and
opens the same verified package as rendering. It reports missing used files and
invalid source ranges; it does not test every frame or encoder.

`timeline studio <file>` opens a JSON file directly. Relative resource paths resolve
from that file's directory. Edits preview without changing the file; **Save** atomically
writes the complete JSON back to the opened path. External file changes cause a conflict
until explicitly reloaded. This session does not create a project or persistent revisions.
Studio shows the authored Timeline and referenced Motion source in one editor. JSX
changes update the browser preview before saving; **Save** writes changed source
files first, then the Timeline when its arrangement or instance bindings changed.
If a source file cannot compile, the author document and code editor stay open so
the error can be corrected. Source and Timeline conflicts keep the local draft.
Use `--web-assets-dir web/dist` when running a development binary without embedded assets.

## Project: versioned timeline editing

See the [Project editing reference](project.md), also available through
`valle docs project`, for revision handling, resource storage and Studio workflows.
`project studio` takes a project ID from this store, not a JSON path; use
`timeline studio` to edit a standalone file.

```sh
valle project create demo --timeline examples/timeline.json --json
valle project show demo -o "$demo_dir/edited.timeline.json" --json
# Edit the exported document before applying it.
valle project apply demo --base-revision 1 \
  --timeline "$demo_dir/edited.timeline.json" --intent "Adjust the edit" --json
valle project history demo --limit 20 --json
valle project render demo -o "$demo_dir/project.mp4" --events
valle project studio demo --port 0 --events
```

`apply` submits a complete timeline document. Unchanged content returns
`outcome: "unchanged"` without adding a revision. Changed content returns
`outcome: "committed"`; reuse its returned revision as the next `--base-revision`.
A stale base returns `outcome: "staleBase"` and a nonzero exit code. Fetch `show`
and reconcile the edit before retrying. Validation rejection returns
`outcome: "rejected"` with errors.

After a changed edit creates revision 2, restore the first version with:

```sh
valle project restore demo --base-revision 2 --revision 1 --json
valle project render demo --revision 1 --frame 0 -o "$demo_dir/original.png"
```

Restore appends a revision; it does not delete history. Project import resolves
relative resources to absolute paths, so changing directories does not break them.
Referenced files must remain available. Public timeline JSON has no version field;
project revision numbers describe saved edits.

## Assets: import, annotate and find media

See the [Asset library reference](assets.md), also available through
`valle docs assets`, for import modes, annotations, entities, analysis and maintenance.

Use the PNG rendered in the Motion example:

```sh
valle assets add "$demo_dir/cover.png" --mode copy --title "Demo cover" --tag demo \
  --json > "$demo_dir/asset.json"
asset_id=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["data"]["items"][0]["content_digest"])' "$demo_dir/asset.json")
valle assets show "$asset_id" --json
valle assets tag "$asset_id" --add approved
valle assets annotate "$asset_id" --text "Opening title" --tag intro
valle assets search "Opening" --json
valle assets list --tag demo --json
valle assets resolve "$asset_id" --json
valle assets stats --json
```

IDs are content hashes; a full `sha256:...` value or an unambiguous prefix is
accepted. Reimporting identical bytes reuses the ID. `reference` registers a file
in place, `copy` stores a copy, and the default `reflink` attempts a copy-on-write
import. `remove` deletes stored bytes but keeps metadata; reimport restores them.

For timed annotations on audio/video, use `--at 1.5` or `--range 1 2`.
`assets analyze ID --with shots,beats --events` explicitly invokes supported
analyzers; they have their own media/model prerequisites. `vlm` additionally needs
a configured service. Search and tags work without semantic analysis or models.
`assets transcript` reads stored ASR; use `media transcribe` to produce a new
standalone transcript. `media` commands do not automatically import outputs or
attach their analyses to the asset library.

## Models and Media: local file processing

See the [Media processing reference](media.md) for each operation's options,
prompt/mask/shot-list formats, audio behavior and Timeline integration.

List and verify are offline. Only `models install` downloads weights:

```sh
valle models list --json
valle models install dpdfnet --backend onnx --events
valle models verify dpdfnet --artifact onnx-int8-dynamic-linear --json
valle media enhance speech.wav -o clean.wav --report enhance-run.json --events
```

The artifact above is the current default ONNX choice for DPDFNet. For another
model, verify the `artifact` returned by installation. Without `--artifact`,
verification checks all catalog variants (or all matching `--backend`), including
uninstalled variants; it can exit 3 even when the variant you use is ready.

ONNX adapters require **ONNX Runtime 1.28.x**. Put the platform library beside the
CLI (`libonnxruntime.dylib`, `libonnxruntime.so`, or `onnxruntime.dll`) or set
`ORT_DYLIB_PATH` to its absolute path, including any dependencies it needs.
`models install` installs weights, not this runtime. Artifact readiness and
`compatible` in `models list` do not prove that inference can start; a processing
command checks the runtime too.

Supply your own input files below and install the listed model first. Image
commands shown here use PNG inputs. Output paths must be new unless the command
allows `--overwrite`.

| Operation | Model to install | Command |
| --- | --- | --- |
| Word transcription | `qwen3-asr-0.6b` and `qwen3-aligner-0.6b` | `valle media transcribe speech.wav --lang en -o words.json` |
| Foreground matting | `birefnet` | `valle media matte input.png -o foreground.png` |
| Speech enhancement | `dpdfnet` | `valle media enhance speech.wav -o clean.wav` |
| Stem separation | `demucs` | `valle media separate song.wav -o stems` |
| Shot detection | `omnishotcut` | `valle media shots input.mp4 -o shots.json` |
| Prompted segmentation | `edgetam` | `valle media segment input.png --prompt prompt.json -o mask.png` |
| Inpainting | `lama` | `valle media inpaint input.png --mask mask.png -o repaired.png` |
| Upscaling | `realesrgan` | `valle media upscale input.png --scale 4 -o large.png` |
| Frame interpolation | `rife` | `valle media interpolate input.mp4 --fps 60 --shots shots.json -o smooth.mp4` |

Important input/output details:

- Transcription and forced alignment currently support macOS and Linux only; Windows is not yet supported. Transcription uses the native CPU route and accepts `--backend auto` only.
  `--text-only` skips alignment and only needs the ASR model. It cannot be
  combined with `-o`; with `--json` the text is inside the run result.
- Matting can use ONNX or macOS CoreML; the other listed non-ASR tools currently
  implement ONNX. Explicit backend requests never silently fall back.
- Segmentation requires exactly three labeled points in source-pixel coordinates.
  Supply coordinates for your image in the prompt file. The mask must be strict binary
  Gray8 (0/255); white selects pixels for inpainting. A video mask uses lossless
  Gray8 `.mkv`. Optional foreground output is PNG or RGBA MOV.
- `separate` creates a new directory with `vocals.wav` and `instrumental.wav`;
  `--overwrite` cannot replace that directory. Segmentation with
  `--foreground-output` also requires new destinations.
- For tools with `--range`, use seconds as `start,end` (for example `--range 1,3`);
  the end is exclusive. Inpaint masks must match the selected range rebased to zero.
- Interpolation's target FPS must exceed the source FPS. `--shots` is optional;
  use it to avoid interpolation across known cuts. Upscaling currently supports x4.
- `--report path.json` saves the same versioned run envelope as `--json`, separately
  from the media artifact. Missing models return an actionable install hint.

## Output contract for scripts and agents

Global `--json` and `--events` work before or after subcommands and are mutually
exclusive. Human output is for reading; scripts should select one of these modes.
`--help` and `--version` remain plain text even with a machine-output flag.

| Mode | stdout | stderr |
| --- | --- | --- |
| Default | Command-specific human output or JSON | Diagnostics and human progress |
| `--json` | One JSON value for a finite command, including failures | Diagnostics only |
| `--events` | NDJSON events, followed by one `report` for a finite command | Diagnostics only |

The transport is shared; result payloads retain domain-specific information:

| Command | Result shape / success indicator |
| --- | --- |
| Motion/Timeline check or render; Project render | `status: "ok"`; delivery includes output, frame count and render ID |
| Project create/show/history | Snapshot or revision page; use the process exit code |
| Project apply/restore | `outcome`: `committed`, `unchanged`, `staleBase`, or `rejected` |
| Assets | `ok`, `data`, `error`, `warnings`; batch import can contain failed items |
| Media | `format: "valle.media-run"`, `formatVersion: 1`, `status`, `result`, `report`, `warnings`, `error` |
| Models | List array, install result, or verification report; inspect artifact `state` and exit code |
| Docs | `status: "ok"`, executable `version`, plus `topics` or the selected Markdown `content` |

Always check the process exit code. A JSON object or the arrival of `report` alone
does not mean success. Argument errors use `error.code: "invalid_arguments"`;
uncategorized command errors use `command_failed`. Motion compile failures use
`motion_compile_failed` and include `error.diagnostics` with messages, codes,
source spans and, where available, source paths. Media and Assets retain their
typed error codes and optional hints.

Render reports include the actual Motion compiler-entry count in `compilations`
(zero for a Timeline without Motion), and separate `compileMs`, `prepareMs`, and
`renderMs` in `delivery.timing`. Image deliveries include `delivery.frameKeys`,
the sequence paths in `delivery.outputs`, and the optional `delivery.storyboard`.

| Exit code | Meaning |
| --- | --- |
| 0 | Success |
| 1 | General failure, rejected/stale project edit, or asset command/batch failure |
| 2 | CLI argument error; Media invalid input |
| 3 | Media/model dependency, installation or runtime unavailable; failed model verification |
| 4 | Media execution/inference/output validation failure |
| 130 | Media operation cancelled with Ctrl-C |

An NDJSON line has `ts_ms` (UTC epoch milliseconds), increasing `seq`, `level`,
`type`, and `data`. Event types are:

- `render.progress`: `{ "completed": 10, "total": 90 }`.
- `motion.compilation`: `{ "entry": "title.motion.tsx", "elapsedMs": 12.5 }`, emitted
  for each actual Motion compiler invocation, including a failed invocation.
- `media.progress`: `phase`, optional `completed`, `total`, and `message`.
- `models.progress` and `analyze.progress`: a human-readable `message`.
- `report`: the command's unchanged result payload in `data`.
- `ready`: Studio URL, actual port, `runtime_version`, `runtime_source`; Project
  Studio additionally includes `projectId` and `revision`.
- `reload` and `browser_report`: service events during Studio sessions.

Studio is long-running: `--json` emits one `status: "ready"` value;
`--events` emits `ready` and subsequent service events. Readiness does not signal
process completion, and Studio does not emit a normal final render report.

For render investigation, `VALLE_PERF=1` enables timing diagnostics on stderr.
`--ffmpeg-log-level info` (or `warning`/`debug`) controls FFmpeg diagnostics there.
Keep stdout and stderr separate when parsing events.

## Runtime notes

FFmpeg 7.x, 8.x and 9.x shared libraries are loaded on the first media operation.
Help, source checking, pure Motion PNG rendering and Studio startup work without
FFmpeg. A standalone/static `ffmpeg` executable does not supply these libraries.

On macOS, install a shared-library build with Homebrew:

```sh
brew install ffmpeg
valle media capabilities

# Select a custom installation (the CLI flag takes precedence over the environment):
export VALLE_FFMPEG_DIR=/absolute/path/to/ffmpeg/lib
valle --ffmpeg-dir /absolute/path/to/ffmpeg/lib media capabilities
```

A library directory or installation prefix is accepted. An explicit path is
authoritative: an invalid installation returns an error without falling back to
another one. Setting the path alone does not load FFmpeg. A failed load can be
retried after installing or fixing the libraries; after a successful load, restart
Valle to select a different installation.

On macOS, automatic discovery searches Homebrew `ffmpeg`, `ffmpeg@8` and `ffmpeg@7`
under `/opt/homebrew/opt` and `/usr/local/opt`, plus `/opt/local/lib` and
`/usr/local/lib`. Directories are searched in order; within each directory the
loader tries FFmpeg 9, then 8, then 7. Windows uses versioned DLL names and absolute
PATH entries; Linux uses common library directories and versioned system-loader
names. Install each FFmpeg library set with all of its dependencies.

The loader validates architecture, matching library ABI majors, minimum versions
and required symbols before use. The header baselines are FFmpeg 7.0, 8.0 and 9.0;
later compatible versions within each ABI are accepted. Libraries from different
installations must not be mixed. One process retains its selected ABI until exit.

`valle media capabilities` reports the selected major, actual library paths and
versions, registered codecs and a real H.264 VideoToolbox availability probe.
Software H.264 export requires `libx264` for its CRF/preset controls. Use
`--hardware-encode` to request hardware encoding explicitly; an unavailable
hardware encoder returns an error without falling back to software. Codec
registration alone does not guarantee hardware availability on the current device.
