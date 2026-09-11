# Media processing reference

`valle media` processes local audio, images and video into standalone files:
transcripts, foregrounds, cleaned audio, stems, shot lists, masks and new media.
It does not modify a Timeline, Project or asset-library entry automatically.

This guide covers the current CLI's nine processing operations. Use the
[CLI guide](cli.md) for installation, shared output modes and exit codes, the
[Timeline reference](timeline.md) to assemble results, and the
[Motion reference](motion.md) to animate them.

## Contents

- [Choose an operation](#choose-an-operation)
- [Models and runtimes](#models-and-runtimes)
- [Shared options and file rules](#shared-options-and-file-rules)
- [Transcribe: speech to text](#transcribe-speech-to-text)
- [Matte: automatic foreground extraction](#matte-automatic-foreground-extraction)
- [Enhance: speech noise reduction](#enhance-speech-noise-reduction)
- [Separate: vocals and instrumental](#separate-vocals-and-instrumental)
- [Shots: detect edit boundaries](#shots-detect-edit-boundaries)
- [Segment: select and track an object](#segment-select-and-track-an-object)
- [Inpaint: repair masked regions](#inpaint-repair-masked-regions)
- [Upscale: enlarge images and video](#upscale-enlarge-images-and-video)
- [Interpolate: increase video frame rate](#interpolate-increase-video-frame-rate)
- [Use results in a Timeline or Motion](#use-results-in-a-timeline-or-motion)
- [Reports and troubleshooting](#reports-and-troubleshooting)

## Choose an operation

Commands below assume `valle` is on PATH; in a source checkout use
`./dist/bin/valle`. Supply your own input files and install the selected model
before running an operation. Each subcommand exposes its options through `--help`.

| Operation | Input | Primary output | Default model |
| --- | --- | --- | --- |
| `transcribe` | Audio, or video with audio | Word/sentence JSON, or text-only result | `qwen3-asr-0.6b` plus `qwen3-aligner-0.6b` for timestamps |
| `matte` | PNG or video | Transparent PNG or lossless RGBA MOV | `birefnet` |
| `enhance` | Audio, or video with audio | 48 kHz mono WAV/FLAC | `dpdfnet` |
| `separate` | Audio, or video with audio | Directory with two 44.1 kHz stereo WAV files | `demucs` |
| `shots` | Video | Canonical shot-list JSON | `omnishotcut` |
| `segment` | PNG or video, plus point-prompt JSON | Binary Gray8 PNG or lossless mask MKV | `edgetam` |
| `inpaint` | PNG/video plus matching binary mask | PNG or H.264 MP4 | `lama` |
| `upscale` | PNG or video | PNG or H.264 MP4 at 4× width and height | `realesrgan` |
| `interpolate` | Video, optionally a shot-list JSON | H.264 MP4 at higher FPS | `rife` |

Use `matte` for automatic foreground extraction, `segment` to select a particular
object with points, and `inpaint` to replace selected pixels with inferred content.
A segmentation mask marks selected pixels; a transparent foreground contains the
source colors and a soft alpha channel for compositing.

## Models and runtimes

### Install only the needed weights

For speech enhancement:

```sh
valle models list --json
valle models install dpdfnet --backend onnx --events
valle models verify dpdfnet --artifact onnx-int8-dynamic-linear --json
valle media enhance speech.wav -o clean.wav --backend onnx --report enhance-run.json --events
```

The verification example names the current default DPDFNet ONNX artifact. In
scripts, read `artifacts[].artifact` from the install result and verify that exact
artifact. Without `--artifact`, verification checks all catalog variants, or all
variants matching `--backend`; an unused missing variant can make it exit with 3.

`models list` and `models verify` are offline. `models install` explicitly downloads
weights; Media processing resolves installed weights without downloading them.
`VALLE_MODEL_CACHE` selects the model store independently of `VALLE_HOME`.

Omitting a model version selects the catalog's pinned default. Use install/verify
`--version VERSION` and processing `--model-version VERSION` to select an exact
release. `models install --refresh-catalog --version latest` explicitly refreshes
the catalog and asks for its latest release; it is not the default run behavior.
An adapter may support only specific releases, so a downloaded version is not
necessarily executable by an older Valle binary.

### Select an inference backend

| Operation | Supported model choices | Current execution backends |
| --- | --- | --- |
| `transcribe` | `qwen3-asr-0.6b`; timestamps also use `qwen3-aligner-0.6b` | `auto` only: native CPU, macOS/Linux |
| `matte` | `birefnet`, `modnet` | ONNX CPU; CoreML on supported macOS routes |
| `shots` | `omnishotcut`, `transnetv2` | ONNX CPU |
| Remaining operations | The default model in the operation table | ONNX CPU |

`--backend auto` selects a compatible installed route **within the chosen model**.
It can try another installed route if opening the first model session fails, with
a warning identifying the rejected route. It does not switch models or change
routes partway through processing. Explicit `--backend onnx` or `coreml` does not
fall back after a session-load failure.

Media's `--backend` selects inference, not Motion's `raster`/`metal` renderer or a
video encoder. The current ONNX adapters use CPU execution; installing a GPU
runtime does not enable CUDA or DirectML inference. Catalog entries can include
routes that the current host or binary cannot execute. Inspect `compatible`,
`incompatibility`, `routeVerified` and artifact `state` in `models list --json`.
Neither catalog compatibility nor artifact integrity proves a real inference run
will load successfully. Transcription is currently unavailable on Windows.

### Supply the runtime libraries

Weights and runtime libraries are separate dependencies:

- Audio/video decoding and video encoding require compatible FFmpeg 7, 8 or 9
  shared libraries. Check them with `valle media capabilities --json`. A standalone
  `ffmpeg` executable does not supply the libraries Valle loads. For custom paths,
  use `--ffmpeg-dir PATH` or `VALLE_FFMPEG_DIR`; see
  [FFmpeg runtime setup](cli.md#runtime-notes).
- ONNX routes require **ONNX Runtime 1.28.x** for the executable's architecture.
  Set `ORT_DYLIB_PATH` to the full library filename, or place the runtime beside
  the Valle executable: `libonnxruntime.dylib` on macOS, `libonnxruntime.so` on
  Linux, or `onnxruntime.dll` on Windows. Supply its dependent libraries too.
- CoreML matting uses macOS's runtime. Native CPU transcription does not use ONNX
  Runtime. These routes still need FFmpeg when decoding audio/video.

For example, on macOS with an ONNX Runtime installation:

```sh
ORT_DYLIB_PATH=/absolute/path/libonnxruntime.dylib valle media enhance speech.wav -o clean.wav --backend onnx --json
```

`media capabilities` checks FFmpeg; it is not a model or ONNX Runtime readiness
check. The simplest full readiness check is a short run of the operation you need.

## Shared options and file rules

All nine operations accept `--model`, `--model-version`, `--backend`, `--report`
and `--overwrite`, subject to the operation-specific restrictions below. Use
`--json` for one result or `--events` for progress and a final result. They are
mutually exclusive; see the [shared output contract](cli.md#output-contract-for-scripts-and-agents).

Input, output, report and auxiliary prompt/mask paths must be distinct where they
would otherwise overwrite an input or another output. Existing output files are
rejected by default. `--overwrite` allows supported file replacement; it does not
permit in-place processing. Exceptions: `separate` always requires a new output
directory, and `segment --foreground-output` requires two new output destinations
and cannot be combined with `--overwrite`.

Use PNG for still-image input. The image/video branch is selected by the `.png`
extension; renaming a JPEG to `.png` is not conversion. Audio/video support also
depends on the available FFmpeg decoders. These commands take local paths, not
asset-library IDs or Timeline resource aliases.

### Selecting part of a video

`matte`, `segment`, `inpaint` and `upscale` support `--range START,END` in seconds.
The start must be non-negative and the end greater than the start. The interval is
half-open: start included, end excluded. `--range 2,` means from second 2 to EOF.
Ranges are video-only; these flags do not trim PNG input or apply to the other
five operations.

Range outputs are trimmed artifacts. Place them at the desired Timeline `start`
and trim them relative to their own time origin. In particular, a range mask for
`inpaint` must match the selected source frames and start at timestamp zero; use
identical ranges for `segment` and `inpaint` as shown below. Frame selection is
limited to actual decoded frame timestamps, so durations can differ from requested
seconds by frame rounding.

### Video output behavior

| Output | Encoding | Sound |
| --- | --- | --- |
| `matte` / `segment --foreground-output` MOV | Lossless qtrle with straight alpha | No audio |
| `segment` MKV | Lossless FFV1, strict binary Gray8 | No audio |
| `inpaint` / `upscale` / `interpolate` MP4 | H.264, opaque YUV420P | Selected source audio re-encoded as 48 kHz stereo AAC, if present |

MP4 processing normalizes color metadata to BT.709 limited range and does not
preserve alpha. Audio is decoded and re-encoded; it is not a lossless packet copy,
and secondary audio streams are not copied. A source without audio stays silent.
These MP4 operations currently use software H.264 encoding, requiring FFmpeg's
`libx264` encoder. They do not expose Motion's `--workers`, `--crf`, `--preset`
or `--hardware-encode` options. Read the run report's output summaries and warnings
when the exact encoding matters.

## Transcribe: speech to text

Install ASR and alignment for timestamped output:

```sh
valle models install qwen3-asr-0.6b --events
valle models install qwen3-aligner-0.6b --events
valle media transcribe interview.mp4 --lang en -o words.json --json
valle media transcribe interview.mp4 --lang en --level sentence -o sentences.json --json
```

`--level` accepts `word` (default) or `sentence`. Omit `--lang` for automatic
language handling; use a supported language code such as `en` or `zh` when known.
The audio track is decoded to 16 kHz mono for recognition. The primary JSON file
is an analysis document, separate from the run envelope on stdout.

Illustrative word output; all times are seconds relative to the input:

```json
{
  "audio": "interview.mp4",
  "lang": "en",
  "words": [
    { "id": "w0", "text": "Hello", "start": 0.2, "end": 0.5 },
    { "id": "w1", "text": " world!", "start": 0.5, "end": 0.9 }
  ]
}
```

`audio` and `lang` are optional fields. Word IDs are unique within the document;
consume them as IDs instead of inferring timing from their spelling. Sentence
output has `lang` and `sentences`, each with `text`, `start` and `end`. It is derived
from aligned words using punctuation and pauses, with a 15-second grouping target;
it is not a separate sentence recognition model.

For text without alignment, only the ASR model is needed:

```sh
valle media transcribe speech.wav --text-only
valle media transcribe speech.wav --text-only --json
```

The first command prints text to stdout. In JSON mode read `result.text`.
`--text-only` cannot be combined with `-o` or `--level sentence`. Normal output
paths default to `<stem>.words.json` or `<stem>.sentences.json`. There is no direct
SRT/VTT export or automatic insertion into Timeline captions; see
[using analysis results](#use-results-in-a-timeline-or-motion).

## Matte: automatic foreground extraction

```sh
valle models install birefnet --backend onnx --events
valle media matte portrait.png -o foreground.png --backend onnx --json
valle media matte input.mp4 --range 0,3 --fps 30 -o foreground.mov --backend onnx --events
```

The result keeps the source image dimensions and colors with inferred soft alpha.
PNG input produces transparent PNG; video produces a lossless RGBA MOV without
audio. Defaults are `<stem>.foreground.png` and `<stem>.foreground.mov`.

For video, **`--fps` defaults to 5**. It controls both sampling and output frame
rate; it is not just a progress or preview setting. Set it explicitly when you
need smoother footage. Fractional rates such as `29.97` are accepted. Frames are
sampled across the requested range; the duration rounds to that output frame grid.
If input duration cannot be determined, supply an explicit range end.

`--model modnet` is another supported matting choice; install `modnet` before
selecting it. Both models support ONNX and compatible macOS CoreML routes. For
CoreML, install with `--backend coreml` and run with the same backend. `auto` does
not substitute MODNet for BiRefNet.

## Enhance: speech noise reduction

```sh
valle models install dpdfnet --backend onnx --events
valle media enhance speech.wav -o clean.wav --events
valle media enhance interview.mp4 -o clean.flac --report enhance-run.json --json
```

DPDFNet enhances speech and suppresses noise. It decodes the selected audio stream
and writes **48 kHz mono** audio: float32 WAV or 24-bit PCM FLAC, selected by output
extension. The default is `<stem>.enhanced.wav`. FLAC output quantizes floating
point samples to 24-bit PCM; the report records this conversion.

Video input produces an audio file, not a video with replaced sound. Stereo input
is downmixed to mono. Use `separate` for vocal/instrumental separation and a
Timeline to combine cleaned speech with picture or music.

## Separate: vocals and instrumental

```sh
valle models install demucs --backend onnx --events
valle media separate song.wav -o stems --report separate-run.json --events
```

The required output directory receives exactly `vocals.wav` and
`instrumental.wav`, both 44.1 kHz stereo float32 WAV. Video with an audio track is
also accepted; output remains audio-only. The CLI does not expose individual drum,
bass or other instrument outputs.

`stems` must not exist, even if it is empty. `--overwrite` can replace an existing
report file but cannot replace the stem directory. Supply a new directory for each
run, and keep `--report` outside that output directory.

## Shots: detect edit boundaries

```sh
valle models install omnishotcut --backend onnx --events
valle media shots input.mp4 -o shots.json --report shots-run.json --events
```

The default output is `<stem>.shots.json`. Its primary artifact is a standalone,
model-neutral shot list, suitable for `interpolate --shots`. It does not cut the
input video into files. Illustrative output for a four-second, 30 FPS source:

```json
{
  "durationSeconds": 4.0,
  "boundaries": [{ "frameIndex": 60, "ptsSeconds": 2.0 }],
  "shots": [
    { "index": 0, "startSeconds": 0.0, "endSeconds": 2.0 },
    { "index": 1, "startSeconds": 2.0, "endSeconds": 4.0 }
  ]
}
```

Frame indices are zero-based. Boundary times are source-local seconds; shots are
contiguous half-open intervals covering the source duration. `confidence` is
optional on a boundary and is present only when the detector provides it.
Model-specific transition details are in `result.transitions` in the run result,
not in the primary shot-list file.

`transnetv2` is also supported: install it and pass `--model transnetv2`. `auto`
chooses a backend within the selected detector, not whichever detector happens to
be installed. Both currently use ONNX CPU.

## Segment: select and track an object

Install `edgetam` and create a point-prompt JSON for your actual image. This example
uses coordinates inside a 640×360 image; replace them with points on your subject
and background. Save it as `prompt.json`:

```json
{
  "format": "valle.segment-prompt",
  "formatVersion": 1,
  "coordinateSpace": "source_pixels_xy",
  "points": [
    { "x": 320, "y": 180, "label": "positive" },
    { "x": 340, "y": 200, "label": "positive" },
    { "x": 20, "y": 20, "label": "negative" }
  ]
}
```

The prompt requires **exactly three labeled points**. Coordinates are pixels in
the decoded display image, with origin at the top left, `x` increasing right and
`y` increasing down; they are not normalized 0–1 values. Each point must be inside
the image. Labels are `positive` for the desired object or `negative` for excluded
areas. At least one point must be positive. Extra JSON fields are rejected.

```sh
valle models install edgetam --backend onnx --events
valle media segment input.png --prompt prompt.json -o mask.png --foreground-output foreground.png --json
```

`-o` is required and writes a **binary** mask: selected pixels are 255 (white),
others 0 (black). `--threshold` defaults to `0.5` and must be strictly between 0
and 1. It sets the probability threshold for this mask.

`--foreground-output` separately writes source colors with **soft alpha**; it is
not just the binary mask copied into alpha. Use the foreground for compositing,
and the binary mask for `inpaint`. Both output paths must be new when requesting
the two outputs together; `--overwrite` is rejected in this mode.

For video:

```sh
valle media segment input.mp4 --prompt prompt.json --range 2,5 -o mask.mkv --foreground-output foreground.mov --events
```

The points initialize one object on the **first selected frame**, then the model
tracks it sequentially. With `--range 2,5`, pick points from the first selected
frame near second 2, not from the beginning of the video. There is no CLI text
prompt, box prompt, multiple-object list or mid-video re-prompting option.

The mask uses lossless FFV1 Gray8 in MKV, preserving selected-frame timing; the
optional foreground uses RGBA MOV. Both are silent. Range outputs start at zero;
a full-source mask preserves the source timestamp origin for matching. Do not
convert masks to JPEG, H.264 or a soft grayscale image before passing to `inpaint`.

## Inpaint: repair masked regions

```sh
valle models install lama --backend onnx --events
valle media inpaint input.png --mask mask.png -o repaired.png --json
```

White pixels in the mask are the region to repair; black pixels are retained.
The image mask must have the same dimensions as the source and be **8-bit,
single-channel Gray8 PNG containing only 0 and 255**. An RGB image that looks
black and white or a soft alpha matte does not satisfy this contract.

For video, use a lossless FFV1 Gray8 mask with the same selected frame count,
dimensions and timestamps. A single PNG mask cannot be applied to an entire
video through this command. Create a range mask with `segment`, then use the same
range for `inpaint`:

```sh
valle media segment input.mp4 --prompt prompt.json --range 2,5 -o repair-mask.mkv --events
valle media inpaint input.mp4 --mask repair-mask.mkv --range 2,5 -o repaired.mp4 --events
```

The range mask must start at timestamp zero. A full-duration mask from a different
range, even if its dimensions match, is not interchangeable. Keep the source
unchanged between segmentation and repair.

Output defaults to `<stem>.inpainted.png` or `<stem>.inpainted.mp4`. Video is
re-encoded as H.264 with source audio re-encoded as AAC when present. Video frames
are repaired individually; the command does not offer a temporal-consistency or
text-guided generation model.

## Upscale: enlarge images and video

```sh
valle models install realesrgan --backend onnx --events
valle media upscale input.png --scale 4 -o large.png --json
valle media upscale input.mp4 --range 0,2 --scale 4 -o large.mp4 --events
```

The published adapter supports **only `--scale 4`**, which is also the default.
Width and height each grow by four, so a 640×360 source becomes 2560×1440 and the
pixel count grows by sixteen. It does not choose an arbitrary target resolution
or perform downscaling. Defaults are `<stem>-x4.png` and `<stem>-x4.mp4`.

PNG keeps a separately scaled alpha channel; the neural model processes RGB.
Video output is opaque H.264 MP4, retaining source frame timing with timestamps
rebased for output. The selected audio stream becomes 48 kHz stereo AAC. For
large sources, check output dimensions and available memory with a short range
before processing the complete file.

## Interpolate: increase video frame rate

```sh
valle models install rife --backend onnx --events
valle media interpolate input.mp4 --fps 60 -o smooth.mp4 --events
```

`-o` is required. `--fps` is an integer, defaults to 60, must be between 1 and
1000, and must be **greater than the source frame rate**. This increases temporal
sampling at the same playback speed; it is not a slow-motion or speed-ramp command.
The input needs at least two decodable frames and even display dimensions for
H.264 output. Output duration is rounded to the target frame grid.

For footage containing edits, detect shots first and pass the primary shot-list
artifact:

```sh
valle media shots input.mp4 -o shots.json --events
valle media interpolate input.mp4 --fps 60 --shots shots.json -o smooth.mp4 --events
```

Install `omnishotcut` as well for this workflow. The shot list must describe the
same unchanged source, with canonical contiguous shots and matching duration.
It prevents interpolation between frame pairs that cross a listed boundary.
Without `--shots`, the command does not automatically detect cuts. Pass
`shots.json`, not `shots-run.json` or an entire `valle.media-run` envelope.

The output is opaque H.264 MP4 with source audio re-encoded as 48 kHz stereo AAC
when present. There is no `--range` option for interpolation; trim the source
first if only one excerpt should be processed, and detect shots on that excerpt.

## Use results in a Timeline or Motion

### Replace a video's sound with enhanced speech

First produce `clean.wav` using `media enhance input.mp4 -o clean.wav`. For a
640×360 input lasting at least three seconds, save this as `clean.timeline.json`
beside `input.mp4` and `clean.wav`:

```json
{
  "canvas": { "width": 640, "height": 360, "fps": 30 },
  "resources": { "footage": "input.mp4", "speech": "clean.wav" },
  "tracks": {
    "visual": [{ "clips": [
      { "kind": "video", "src": "footage", "start": 0, "duration": 3, "gain": 0 }
    ] }],
    "audio": [{ "clips": [
      { "src": "speech", "start": 0, "duration": 3, "gain": 1 }
    ] }]
  }
}
```

```sh
valle timeline check clean.timeline.json --json
valle timeline render clean.timeline.json -o clean.mp4 --events
```

Visual video clips play source audio by default. `gain: 0` on the video prevents
mixing its noisy original sound with the cleaned track. For silent foreground MOV
outputs, add an audio track from the original source if needed, applying the same
source trim used to create the foreground.

### Use analysis and image results

- **Captions:** turn transcript words or sentences into Timeline caption clips:
  `start = item.start`, `duration = item.end - item.start`, and `text = item.text`.
  Omit zero-duration items and supply the caption track's required font resource.
  For trimmed or retimed footage, map source times into the edited timeline;
  see [caption authoring](timeline.md#captions) and
  [source-time mapping](timeline.md#time-trimming-and-playback).
- **Cuts:** use each shot's `startSeconds` as video `trimStart` and
  `endSeconds - startSeconds` as its duration, then choose Timeline placement.
  The shot-list JSON itself is not a Timeline document.
- **Foregrounds and upscaled images:** add the PNG/MOV as a Timeline resource or
  bind an image to Motion through `--asset NAME=PATH`. Transparent PNG and RGBA
  MOV contain compositable color; a binary mask is an analysis artifact.
- **Asset library:** explicitly run `valle assets add FILE --mode copy --json`
  when the processed file should become a managed asset. Media processing alone
  does not import it or attach its analysis to an existing asset.

## Reports and troubleshooting

`-o` writes the operation's primary artifact. `--report run.json` writes the
versioned run envelope, also returned by `--json`; in event mode the final
`report` event carries that envelope in `data`. These are different documents
from a word transcript, point prompt or shot list.

Useful fields in a successful Media envelope:

| Field | Use |
| --- | --- |
| `format`, `formatVersion`, `status` | Expect `valle.media-run`, `1`, `ok`; also check process exit code |
| `result` | Operation-specific analysis, output paths and metrics |
| `report.outputs[].summary` | Actual dimensions, duration, channels, codecs and alpha mode where applicable |
| `report.models[]` | Model ID/version, revision, artifact, route, backend and precision actually used |
| `report.timing` | `loadSeconds`, `decodeSeconds`, `preprocessSeconds`, `inferenceSeconds`, `postprocessSeconds`, `encodeSeconds`, `totalSeconds` |
| `warnings` | Route fallback, color/audio conversion or report-publication issues |
| `error` | Typed error and optional hint on failure; null on success |

Timing fields describe work within one Media run. Stages can overlap in a pipeline;
use `totalSeconds` for elapsed run time, rather than summing all stage values.
Model loading and output validation can be significant for short inputs. Compare
runs using the same input, model artifact, backend and output settings.

Reports are published after successful processing. A failed operation does not
guarantee a report file; capture the JSON/event result and stderr for diagnosis.
If media succeeds but publishing the report fails, the result includes
`report_write_failed` as a warning. Check it if your workflow requires that file.

| Symptom | What to check |
| --- | --- |
| Missing model / exit 3 | Read the install hint, install the chosen model and use the same `VALLE_MODEL_CACHE` when running |
| `models verify` fails after installation | Verify the exact installed artifact instead of all unused variants |
| Runtime unavailable despite installed weights | Check ONNX Runtime version, architecture and dependencies, or FFmpeg shared-library setup as named in the error |
| No compatible route / unsupported adapter | Check model ID, version, requested backend and `models list` compatibility; downloaded catalog routes may not be executable |
| Input rejected / exit 2 | Check required paths, extensions, range, three-point prompt, binary mask format and frame/timestamp correspondence |
| No sound in a foreground MOV | Expected for `matte` and `segment`; add the source sound explicitly in a Timeline |
| Matting video looks like 5 FPS | Set `matte --fps` to the intended output sampling rate |
| Blended frames at a cut | Supply a shot list for the same source to `interpolate --shots` |
| Model load, inference or output validation fails / exit 4 | Keep the typed error, warnings and runtime diagnostics; validate with a shorter input after correcting the reported cause |

CLI option parsing and model-file verification do not establish visual or audio
quality. Review the resulting media and its report for the operation and backend
you intend to use.
