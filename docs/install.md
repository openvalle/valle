# Installing Valle

Valle is one executable. Graphics need nothing else; video also needs FFmpeg.

## The executable

On macOS and Linux:

```sh
curl -fsSL https://openvalle.ai/install.sh | sh
```

On Windows, in PowerShell:

```powershell
irm https://openvalle.ai/install.ps1 | iex
```

Each downloads the latest release's executable for the machine, checks it against the
release's `SHA256SUMS`, puts it in `~/.valle/bin` and adds that folder to your `PATH`.

| Variable | What it does |
| --- | --- |
| `VALLE_VERSION` | Installs that release, such as `v0.1.0`, instead of the latest |
| `VALLE_INSTALL` | Installs under that folder instead of `~/.valle` |
| `VALLE_NO_MODIFY_PATH=1` | Leaves `PATH` alone, and says the line to add |

Or download a package yourself from
[Releases](https://github.com/openvalle/valle/releases), unpack it and put `valle` on your
`PATH`.

| Platform | Package |
| --- | --- |
| macOS | Apple silicon and Intel, signed and notarized |
| Linux | x86_64 and arm64, glibc 2.28 or later: AlmaLinux and RHEL 8, Debian 10, Ubuntu 20.04 and later |
| Windows | x86_64 and arm64 |

## Updating

```sh
valle update
```

It replaces the `valle` it is run as with the latest release's, wherever that one is: the
package is checked against the release's `SHA256SUMS`, and the `valle` in it must run before
it takes the old one's place. `valle update 0.1.0` installs that release, an earlier one too.
Running the install script again does the same for `~/.valle/bin`.

A Studio that is open keeps running the `valle` it started with: start it again. When the
update is to another minor version, such as 0.1 to 0.2, update the skill too, with the command
below.

## What it draws on

Valle draws on the GPU through Metal, Vulkan or Direct3D 12. On Linux and Windows without a
GPU, it draws on a CPU driver it carries, with the same pixels on every such machine. The
built-in fonts download on first use.

## FFmpeg

Video needs FFmpeg 7, 8 or 9 as **shared libraries**: `libavcodec`, `libavformat`,
`libavutil`, `libswscale` and `libswresample`. The `ffmpeg` program alone is not enough. H.264
and HEVC are encoded by libx264 and libx265, which only a **GPL build** has; on macOS the
hardware encoder is used first.

| Platform | Install |
| --- | --- |
| macOS | `brew install ffmpeg` |
| Debian 13, Fedora, Arch | The distribution's `ffmpeg` package |
| Ubuntu, older Debian | Their `ffmpeg` is older than 7. Download a `gpl-shared` build, `linux64` or `linuxarm64`, from [BtbN's FFmpeg-Builds](https://github.com/BtbN/FFmpeg-Builds/releases) and point Valle at its `lib` folder |
| Windows | Download a `gpl-shared` build, `win64` or `winarm64`, from the same page; add its `bin` folder to `PATH`, or point Valle at it |

Valle finds Homebrew's and the distribution's on its own. Where it does not, say where FFmpeg
is, once, in `~/.valle/settings.json`:

```json
{ "ffmpegDir": "/opt/ffmpeg/lib" }
```

Or per command with `--ffmpeg-dir FOLDER`, or with `VALLE_FFMPEG_DIR`. The option comes first,
then the variable, then the file. `valle docs cli --section ffmpeg` has the details.

## Your agent

```sh
npx skills add openvalle/valle
```

It installs [the Valle skill](../skills/valle) for the coding agents you choose, such as
Claude Code, Codex or Cursor; `-g` installs it for every project. The skill tells the agent
which commands to run and how to read their results. The guides it points to are built into
the executable, so they match its version. Without Node.js, copy `skills/valle` into your
agent's skills folder, such as `~/.claude/skills/`.

## The browser engine

[`@openvalle/engine`](../web/packages/engine) plays a Motion scene or a Timeline in a page with
WebGPU, and exports it to MP4:

```sh
npm install @openvalle/engine
```

## Building from source

`cargo xtask package` builds the executable for the current platform into `dist/valle`
(`dist/valle.exe` on Windows), with Studio and its WebAssembly engine embedded. It needs Rust
and Bun; on macOS, Homebrew's LLVM builds the engine for the browser.

On Linux and Windows the package also embeds the CPU Vulkan driver of
[valle-lavapipe](https://github.com/openvalle/valle-lavapipe), the pinned release downloaded
and checked. Valle calls it directly rather than through the system's Vulkan loader, so every
machine without a GPU draws the same pixels. `cargo xtask lavapipe` downloads the driver
alone: with `VALLE_LAVAPIPE` set to its path, a development build and the GPU tests draw on it
too, as CI does.

| Command | What it does |
| --- | --- |
| `cargo xtask build-web` | Builds the browser assets on disk; a package uses them with `--web-assets-dir /path/to/valle/web` when starting Studio |
| `cargo xtask test` | Runs the tests |
| `cargo xtask fmt`, `cargo xtask lint` | Check the code |
