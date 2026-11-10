# The command line

`valle <group> <command> [file] [options]`. Each command's `--help` lists its options and
values; this guide gives what each one is for and what it returns.

## Motion

| Command | What it does |
| --- | --- |
| `valle motion check SCENE` | Compiles the scene and checks every frame, or `--frame`/`--frames`, without drawing |
| `valle motion render SCENE -o OUT` | Draws into `OUT`: a `.png` (one frame), a folder (a PNG sequence), or a `.mp4` or `.mov` |
| `valle motion studio SCENE` | Opens Studio on the scene |

Both `check` and `render` take the bindings `--props`, `--data` (JSON, or `@file`),
`--resource name=path`, `--input name=path` and `--caption`.

## Timeline

| Command | What it does |
| --- | --- |
| `valle timeline check TIMELINE` | Validates the document and its sources, then checks every frame |
| `valle timeline render TIMELINE -o OUT` | Draws it as `motion render` does, with its sound; a `.wav` is the sound alone |
| `valle timeline query TIMELINE [filters]` | The clips that `--clip`, `--track`, `--index`, `--at`, `--marker` and `--meta` pick |
| `valle timeline edit TIMELINE --op JSON` | Sets, moves, trims, deletes or adds a clip, and writes the file only if the result is valid |
| `valle timeline studio TIMELINE` | Opens Studio on it |

## Project

| Command | What it does |
| --- | --- |
| `valle project create FOLDER` | A new project, `--width`, `--height`, `--fps` |
| `valle project add PATH…` | Copies files in, or `--link`s them, and names them as resources |
| `valle project show` | Its timelines, resources, snapshots and last render |
| `valle project check [TIMELINE]` | Checks a timeline and its files; `--cost` plans every frame |
| `valle project save [-m MESSAGE]` | Saves a snapshot of the sources |
| `valle project history` | The snapshots |
| `valle project restore N` | Puts snapshot N back |
| `valle project render [TIMELINE] [-o OUT]` | Renders, by default a video into `renders/` |
| `valle project studio [FOLDER]` | Opens Studio on the project |

`-C FOLDER` works on that project instead of the current folder's.

## Render options

| Option | What it does |
| --- | --- |
| `--frame N` | One frame, from 0 |
| `--frames 0,30,60` or `0..59` | Some frames: a list, or a range with both ends |
| `--storyboard` | The frames side by side in one `.png` |
| `--output-size WxH` | Output pixels, without changing the layout; up to 7680x4320 |
| `--codec h264\|hevc\|prores` | `h264` for `.mp4` and `prores` (4444, with alpha) for `.mov` unless given |
| `--bitrate 8M` | Bits a second, in place of the codec's quality setting |
| `--encoder vt\|ffmpeg` | The hardware encoder where it has the codec, unless given |
| `--decoder vt\|ffmpeg` | How videos the scene shows are decoded |

## Output

Every command takes `--json` (one JSON result) and `--events` (progress lines, then the
result). A result has `status`, and a failure `code`, `message`, `hint` and, for a check,
`problems`. The exit code is 0 on success and 1 on failure. A render's result includes the
file, the frames, its size and, for a video, the codec, the encoder, the GPU it drew on as
`adapter`, and the sound's loudness.

## Rendering without a GPU

Valle draws on the GPU. On Linux and Windows without one, it draws on lavapipe, a CPU Vulkan
driver built into the executable, with the same pixels on every machine; `adapter` then says
`(bundled lavapipe)`. CPU rendering is slower: render a few frames first.

## Environment

| Variable | What it does |
| --- | --- |
| `VALLE_CACHE_DIR` | The cache of downloads and built-in fonts; `~/.valle/cache` unless set |
| `VALLE_FFMPEG_DIR` | The folder of FFmpeg's shared libraries, where it is not found |
| `WGPU_BACKEND`, `WGPU_ADAPTER_NAME` | Choose another GPU backend or adapter |
| `VALLE_LAVAPIPE` | A lavapipe driver to use instead of the built-in one |
