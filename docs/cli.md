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

## Updating

| Command | What it does |
| --- | --- |
| `valle update` | Replaces this `valle` with the latest release's, if that is newer |
| `valle update VERSION` | Installs that release, such as `0.1.0`, newer or not |

The package is checked against the release's `SHA256SUMS`, and the `valle` in it must run and
say its version before it takes this one's place: when anything fails, the code is `update`
and this `valle` is as it was. The result has `from`, `version`, the `file` replaced, the
release's `notes` and, when the minor version changed, the command that updates the `skill`;
when this is the latest already, `version` and `latest`. Tell the user to start an open Studio
again.

## Render options

| Option | What it does |
| --- | --- |
| `--frame N` | One frame, from 0 |
| `--frames 0,30,60` or `0..59` | Some frames: a list, or a range with both ends |
| `--storyboard` | PNG pages of frames, labeled with zero-based frame numbers and timecodes; defaults to 12 evenly spaced frames |
| `--samples N` | Storyboard: evenly sample up to N frames including the first and last; 1 selects the first |
| `--step N` | Storyboard: take frames 0, N, 2N, …; 1 takes every frame |
| `--per-page N` | Storyboard: frames per PNG, default 12; multiple pages use `NAME-001.png`, `NAME-002.png`, … |
| `--output-size WxH` | Output pixels, without changing the layout; up to 7680x4320 |
| `--fps FPS` | Output frame rate, such as `30`, `60` or `30000/1001`; samples the same animation at that rate without editing the source |
| `--codec h264\|hevc\|prores` | `h264` for `.mp4` and `prores` (4444, with alpha) for `.mov` unless given |
| `--bitrate 8M` | Bits a second, in place of the codec's quality setting |
| `--encoder vt\|ffmpeg` | The hardware encoder where it has the codec, unless given |
| `--decoder vt\|ffmpeg` | How videos the scene shows are decoded |

`--samples`, `--step` and `--per-page` require `--storyboard` and positive integers.
Choose only one of `--samples`, `--step`, `--frame` or `--frames`. A storyboard output must
end in `.png`; a single page keeps that name. See `valle docs motion storyboards` for examples.

`--fps` applies to Motion, Timeline and Project renders: video, PNG sequences, individual
frames and storyboards. Frame numbers and `--step` refer to the output rate. Without it,
the scene's `composition.fps` or the timeline's `canvas.fps` is used. A `.wav` has no video
frame rate and rejects this option. The JSON result reports the effective `fps` as a ratio
or integer string; Project render records also keep an explicit `--fps`.

## Output

Every command takes `--json` (one JSON result) and `--events` (progress lines, at most a
hundred, then the result). `--perf` times each stage of the work and adds a `perf` object to
the result: each stage with its time in all its spans (`ms`), how many spans it had
(`count`), and the middle (`p50`), 95th (`p95`) and slowest (`max`) span in milliseconds;
the process's peak resident memory (`peak_rss_kb`); and, for a render, what it still holds
(`hold`, such as `gpu_kb`, the GPU memory its buffers and textures take). Stages are
`load.network` (a resource fetched), `load.scene` (a scene compiled and laid out), `eval` (a
frame made), `draw` (a frame drawn and its commands submitted), `gpu-wait` (waiting for
one the GPU drew) and `encoder` (waiting for the encoder's surface). A stage's `ms` leaves
out the stages inside it, so all stages' `ms` added together is the work done and not more.
Without `--perf` timing costs one atomic check a span. A result leaves
out fields that would be `null` or empty. It has
`status`, and a failure `code`, `message`, `hint` and, for a check, `problems`. The exit code is 0 on success and 1 on failure. A render's result includes the
file, the frames, its size and, for a video, the codec, the encoder, the GPU it drew on as
`adapter`, and the sound's loudness.

A storyboard's JSON also includes `storyboard.pages`, in output order. Each page gives its
`file`, `width`, `height`, `columns`, `rows`, and `cells` in the requested frame order
(including repeats). Each cell gives `frame`, `time` in seconds, the displayed `timecode`, and its image rectangle
`x`, `y`, `width`, `height`, in page pixels; labels and borders lie outside that rectangle.
Transparent pixels appear over a checkerboard. `files` lists all PNGs actually written.

## Rendering without a GPU

Valle draws on the GPU. On Linux and Windows without one, it draws on lavapipe, a CPU Vulkan
driver built into the executable, with the same pixels on every machine; `adapter` then says
`(bundled lavapipe)`. CPU rendering is slower: render a few frames first.

## FFmpeg

Video needs FFmpeg 7, 8 or 9 as shared libraries (`libavcodec`, `libavformat`, `libavutil`,
`libswscale`, `libswresample`); the `ffmpeg` program alone is not enough. H.264 and HEVC are
encoded by libx264 and libx265, which only a GPL build has; on macOS the hardware encoder is
used first. Valle looks for the libraries in the first of these:

1. `--ffmpeg-dir FOLDER`, an option of every command
2. `VALLE_FFMPEG_DIR`
3. `ffmpegDir` in `~/.valle/settings.json`
4. Where each system usually has them: Homebrew and MacPorts on macOS; `/usr/lib` and the
   system's library path on Linux; the folders of `PATH` on Windows

A folder is the one with the libraries, or an installation whose `lib` or `bin` has them.

## Settings

`~/.valle/settings.json` sets what you want for every command. A field it does not know is an
error that names it, and a command that needs none of it does not read the file. `VALLE_HOME`
names another folder for it.

```json
{ "ffmpegDir": "/opt/ffmpeg/lib" }
```

| Field | What it does |
| --- | --- |
| `ffmpegDir` | The folder of FFmpeg's shared libraries; `--ffmpeg-dir` and `VALLE_FFMPEG_DIR` come first |

## Environment

| Variable | What it does |
| --- | --- |
| `VALLE_CACHE_DIR` | The cache of downloads and built-in fonts; `~/.valle/cache` unless set |

Files from the web are kept in the cache by the SHA-256 of their content, whatever their
kind: fonts, images, shaders and videos. A URL whose path holds a content hash, such as
jsDelivr's `…/gh/google/fonts@<commit>/…`, serves what that hash names and cannot change, so
a file fetched from one is used again without asking the server. Any other URL is asked
about each time with its ETag and Last-Modified, and the cached copy is used when the
server says it is current, or when the server cannot be reached.
| `VALLE_FFMPEG_DIR` | The folder of FFmpeg's shared libraries; see FFmpeg |
| `VALLE_HOME` | The folder of `settings.json`, `~/.valle` unless set |
| `WGPU_BACKEND`, `WGPU_ADAPTER_NAME` | Choose another GPU backend or adapter |
| `VALLE_RELEASES` | Where `valle update` finds releases: a mirror of `https://github.com/openvalle/valle/releases` |
| `VALLE_LAVAPIPE` | A lavapipe driver to use instead of the built-in one |
