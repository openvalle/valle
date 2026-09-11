# CLI guide

Valle has six command groups. Run commands below from the repository root after
building with `cargo xtask build --release`. See [build requirements](../README.md#requirements).

| Group | Use it for | First command |
| --- | --- | --- |
| `motion` | Author animated graphics in JSX, check, preview, render | `valle motion check examples/hello.motion.tsx` |
| `timeline` | Validate and render a timeline document | `valle timeline check examples/timeline.json` |
| `project` | Keep timeline revisions, apply edits, restore, preview, render | `valle project create demo --timeline examples/timeline.json` |
| `assets` | Import, organize, annotate and search local media | `valle assets add cover.png --tag demo` |
| `media` | Process individual media files with local models | `valle media enhance speech.wav -o clean.wav` |
| `models` | Discover, install and verify model weights | `valle models list` |

In the examples, `valle` means the executable at `dist/bin/valle`. Add it to PATH
for this shell, and create an isolated demo directory:

```sh
export PATH="$PWD/dist/bin:$PATH"
demo_dir=$(mktemp -d)
export VALLE_HOME="$demo_dir/library"
```

`VALLE_HOME` contains the asset library and project history (normally `~/.valle`).
It does not relocate model weights. `VALLE_MODEL_CACHE` selects the model directory;
otherwise models use the platform cache directory, or `$VALLE_CACHE_DIR/models`.
Each demo uses new output paths. Keep `demo_dir` for the following sections.

## Motion: JSX to video

```sh
valle motion check examples/hello.motion.tsx --size 640x360
valle motion render examples/hello.motion.tsx \
  --duration 3 --fps 30 --size 640x360 --backend raster \
  -o "$demo_dir/hello.mp4" --events
valle motion render examples/hello.motion.tsx \
  --duration 3 --fps 30 --size 640x360 --frame 45 \
  -o "$demo_dir/cover.png"
valle motion studio examples/hello.motion.tsx --size 640x360 --duration 3
```

Studio prints a local URL and keeps running; stop it with Ctrl-C. `--port 0` asks
the OS for a free port. Studio uses the Web resources embedded by `cargo xtask build`. `valle licenses` displays the embedded dependency notices and source links.

`--frame` selects a zero-based frame and requires a `.png` output. Without it,
render writes `.mp4`. `--size` sets the logical layout canvas; `--output-size`
changes delivery dimensions. Motion accepts rational frame rates such as
`--fps 30000/1001`. Bind declared asset controls with repeated `--asset name=path`,
prepared data with `--data file.json`, and extra fonts with repeated `--font path`.

For video delivery, `--workers 1..8`, `--encode-threads N`, or `--hardware-encode`
control execution. `--bitrate` requires hardware encoding. `--backend auto`
selects an available Metal device on macOS and otherwise Raster; `--backend raster`
explicitly selects CPU composition. See `valle motion render --help` for constraints.

## Timeline: edit a document and render it

The [example timeline](../examples/timeline.json) cuts from blue to teal after
1.5 seconds. It is self-contained and needs no model weights or external assets.

```sh
valle timeline check examples/timeline.json --json
valle timeline render examples/timeline.json -o "$demo_dir/timeline.mp4" --events
valle timeline render examples/timeline.json --frame 60 -o "$demo_dir/timeline.png"
```

Canvas size, frame rate, background and clip timing come from the document. PNG
frames preserve transparency; MP4 delivery requires an opaque canvas background. Timeline and
Project render currently expose `-o` and `--frame`; Motion's delivery tuning flags
are not exposed on those commands.

For actual footage, declare named resources and refer to them from clips:

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
`resources`. `timeline check` validates the document; rendering also opens and
prepares resources, so a successful check does not prove that all files are available.

## Project: versioned timeline editing

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

Always check the process exit code. A JSON object or the arrival of `report` alone
does not mean success. Argument errors use `error.code: "invalid_arguments"`;
uncategorized command errors use `command_failed`. Motion compile failures use
`motion_compile_failed` and include `error.diagnostics` with messages, codes,
source spans and, where available, source paths. Media and Assets retain their
typed error codes and optional hints.

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
