<p align="center">
  <img src="web/apps/studio/brand/valle-mark.svg" width="80" alt="">
</p>

<h1 align="center">Valle</h1>

<p align="center">
  <strong>The video engine for AI agents.</strong><br>
  Motion graphics written in TSX, edits of footage, sound and captions written in JSON,
  rendered on your machine by one executable.
</p>

Coding agents already write components, CSS and JSON well. Valle lets them make and edit video
the same way: a **Motion scene** (`*.motion.tsx`) is animated graphics written as React-style
components with Tailwind, and a **Timeline** (`*.timeline.json`) cuts footage, images, scenes,
music and captions together. Valle checks what the agent wrote, shows it the frames, and
renders the video.

```tsx
export const composition = { width: 1920, height: 1080, fps: 60, duration: 5 };

export default function Launch() {
  const { seconds } = useCurrentTime();
  return (
    <View className="grid size-full place-items-center bg-linear-to-br from-indigo-600 via-fuchsia-500 to-orange-400">
      <Text
        split="char"
        className="text-9xl font-black tracking-tight text-white"
        perUnit={(unit) => {
          const shown = interpolate(seconds, [unit.index * 0.04, 0.8 + unit.index * 0.04], [0, 1], {
            easing: "easeOutCubic",
          });
          return { opacity: shown, translate: `0px ${40 * (1 - shown)}px` };
        }}
      >
        Hello, Valle!
      </Text>
    </View>
  );
}
```

```sh
valle motion check launch.motion.tsx                                   # compile it, check every frame
valle motion render launch.motion.tsx --storyboard -o sheet.png        # 12 frames in one image
valle motion render launch.motion.tsx -o launch.mp4
```

## Why Valle

**It is code an agent already knows.** Components, props, Flexbox, Grid, Tailwind classes and
its own modules for graphics; plain JSON for an edit, with tracks, clips, transitions and
keyframes. Both are files it can diff, review and change.

**It checks before it draws.** `check` compiles a scene or validates a timeline and its
sources. Every error names the line or field and what it takes, so an agent fixes it and runs
the same command again. Every command answers in one short line, or in JSON with `--json`.

**It renders natively, not in a browser.** Scenes are compiled and drawn by Valle's own engine
in Rust, on the GPU: no headless Chrome taking screenshots, no page to load or wait for. Every
frame is a function of its time and inputs, so the same scene draws the same pixels on every
machine, and a render is as fast as the GPU draws.

**It shows the agent what it made.** An agent cannot watch a video, so Valle gives it frames:
any frame as a PNG, or a storyboard of many frames in one image, labeled with frame numbers and
timecodes. Frame 1,000 renders without the 999 before it, and the PNG is exactly what the video
shows.

**It runs on your machine.** One executable checks, renders and opens Studio, an editor in the
browser for a person to watch and adjust the agent's work. It draws on the GPU through Metal,
Vulkan or Direct3D 12, and without one on a CPU driver it carries. No cloud service, no account.

## What you can make

| | |
| --- | --- |
| **Motion graphics** | Kinetic type with per-letter animation, easing and springs, SVG paths and shapes, charts, masks, filters and your own WGSL shaders |
| **Edits** | Footage, images, Motion scenes and sound on tracks, with trims, transitions, keyframes, adjustments and nested timelines |
| **Captions** | Word-timed captions, styled, or drawn by a Motion scene of your own |
| **Projects** | A folder with its timelines and files, snapshots of its sources, and a record of each render |
| **Output** | MP4 (H.264 or HEVC), ProRes 4444 with alpha, PNG frames and sequences, WAV; any output size up to 8K and any frame rate without changing the source |

## Install

Download the executable for your platform from
[Releases](https://github.com/openvalle/valle/releases): macOS (Apple silicon and Intel),
Linux (x86_64 and arm64, glibc 2.28 or later) and Windows (x86_64 and arm64). It is one file;
put it on your `PATH`.

Video needs FFmpeg 7, 8 or 9 as **shared libraries**, a GPL build for H.264 and HEVC; the
`ffmpeg` program alone is not enough. Graphics alone do not need it, and the built-in fonts
download on first use.

| Platform | Install |
| --- | --- |
| macOS | `brew install ffmpeg`; Valle finds it |
| Debian 13, Fedora, Arch | The distribution's `ffmpeg` package; Valle finds it |
| Ubuntu, older Debian | Their `ffmpeg` is older than 7: use a `gpl-shared` build from [BtbN's FFmpeg-Builds](https://github.com/BtbN/FFmpeg-Builds/releases), `linux64` or `linuxarm64`, and point Valle at its `lib` folder |
| Windows | A `gpl-shared` build from the same page, `win64` or `winarm64`; add its `bin` folder to `PATH`, or point Valle at it |

Where Valle does not find it, say where it is, once, in `~/.valle/settings.json`:

```json
{ "ffmpegDir": "/opt/ffmpeg/lib" }
```

or per command with `--ffmpeg-dir FOLDER`, or with `VALLE_FFMPEG_DIR`; the option comes first,
then the variable, then the file. [The CLI guide](docs/cli.md) has the details.

```sh
valle docs start                                  # read this first
valle motion studio examples/01-kinetic-title.motion.tsx
```

Studio prints a local address to open in your browser, and refreshes when the file changes, so
you can watch an agent work. [`examples`](examples) has ten scenes to start from: kinetic type,
a logo sting, a dashboard, a product UI, a vertical story, a loop, a route map, confetti, 3D
cards and a lower third.

## Hand it to your agent

Copy [`skills/valle`](skills/valle) into your agent's skills folder, such as `~/.claude/skills/`
for Claude Code. The skill tells the agent which commands to run, in which order, and how to
read their results. The full guides are built into the executable and match its version:

| Guide | Covers |
| --- | --- |
| [`valle docs start`](docs/start.md) | The workflow, results and errors |
| [`valle docs motion`](docs/motion.md) | Scenes: elements, styles, animation, text, controls, shaders |
| [`valle docs timeline`](docs/timeline.md) | Tracks, clips, transitions, keyframes, sound and captions |
| [`valle docs project`](docs/project.md) | Project folders, snapshots and rendering |
| [`valle docs cli`](docs/cli.md) | Every command, render options and the output contract |

## In the browser

[`@openvalle/engine`](web/packages/engine) plays a Motion scene or a Timeline in a canvas with
WebGPU, and exports it to MP4, with the same engine compiled to WebAssembly.

```sh
npm install @openvalle/engine
```

## Building from source

`cargo xtask package` builds a standalone executable for the current platform into
`dist/valle` (`dist/valle.exe` on Windows), with Studio and its WebAssembly engine embedded. It
needs Rust and Bun; on macOS, Homebrew's LLVM builds the engine for the browser.

On Linux and Windows the package also embeds the CPU Vulkan driver of
[valle-lavapipe](https://github.com/openvalle/valle-lavapipe), the pinned release downloaded and
checked. Without a GPU, or with only a CPU one, Valle draws on it, calling it directly rather
than through the system's Vulkan loader, so every such machine draws the same pixels; a
render's JSON result names the adapter. `cargo xtask lavapipe` downloads the driver alone: with
`VALLE_LAVAPIPE` set to its path, a development build and the GPU tests draw on it too, as CI
does.

For development, `cargo xtask build-web` builds the browser assets on disk, and a packaged
executable uses them with `--web-assets-dir /path/to/valle/web` when starting Studio.
`cargo xtask test` runs the tests, `cargo xtask fmt` and `cargo xtask lint` check the code.

The previous version is archived at the
[`archive/v0`](https://github.com/openvalle/valle/tree/archive/v0) tag.

Valle is in early development: expect rough edges and breaking changes between versions.

Licensed under the [Apache License 2.0](LICENSE). See the
[third-party acknowledgements](THIRD_PARTY.md).
