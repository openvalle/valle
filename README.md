# Valle

English | [简体中文](README.zh-CN.md)

A video creation and editing engine built for AI agents.

**Early development release.** We are sharing the code early. Many features, build tools, and documentation are still incomplete. Expect bugs and breaking changes as the project evolves.

## Requirements

- [Rust via rustup](https://rustup.rs/). `rust-toolchain.toml` selects Rust **1.96.0** and the `wasm32-unknown-unknown` target.
- [Bun](https://bun.com/docs/installation) **1.4.2**.
- **FFmpeg development headers and shared libraries**: `libavcodec`, `libavformat`, `libavutil`, `libswscale`, and `libswresample`. Locally validated with FFmpeg **9.0.1** on macOS. Installing only a standalone `ffmpeg` executable is insufficient.
- Native build tools: C/C++ compiler, LLVM/libclang, `pkg-config`, CMake, Python 3, Ninja, Git, curl, and tar. See [rust-skia build requirements](https://github.com/rust-skia/rust-skia#building).

The setup below targets **macOS**; setup on other platforms still needs validation. Install Rust and Bun above, then install the native dependencies with [Homebrew](https://brew.sh/):

```sh
xcode-select --install # If Command Line Tools are not installed
brew install ffmpeg pkgconf llvm cmake ninja python
export PATH="$(brew --prefix llvm)/bin:$PATH"
export LIBCLANG_PATH="$(brew --prefix llvm)/lib"
export PKG_CONFIG_PATH="$(brew --prefix ffmpeg)/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
```

## Build and run

```sh
git clone https://github.com/openvalle/valle.git
cd valle
cargo xtask build
```

The first build needs network access and can take a while. Cargo is configured to build Skia from source for the CLI's Skottie support. The Rust `xtask` installs Web dependencies, builds Wasm and the CLI, and places the result in `dist/`. Use `cargo xtask build --release` for a release build. FFmpeg shared libraries must also be available when running the CLI.

For CLI-only development, use `cargo build -p valle-cli`. Studio also needs the Web resources produced by the full build above.

## Use the CLI

| Command | Capability |
| --- | --- |
| `motion` | Author JSX, check, preview in Studio, render video or frames |
| `timeline` | Validate and render timeline JSON |
| `project` | Save timeline revisions, apply edits, restore, preview and render |
| `assets` | Import, organize, annotate and search local media |
| `media` | Transcribe, matte, enhance, separate, detect shots, segment, inpaint, upscale and interpolate |
| `models` | List, install and verify local model weights |

See the [CLI guide](docs/cli.md) for workflows across all six groups, model/runtime
requirements, JSON results, progress events and exit codes. Each command also has `--help`.

Try the checked-in examples (no model downloads needed):

```sh
./dist/bin/valle motion check examples/hello.motion.tsx
./dist/bin/valle motion render examples/hello.motion.tsx --duration 3 --size 640x360 -o hello.mp4 --events
./dist/bin/valle timeline check examples/timeline.json
./dist/bin/valle timeline render examples/timeline.json -o timeline.mp4 --events
./dist/bin/valle motion studio examples/hello.motion.tsx --size 640x360
```

Output files must be new. Use `--json` for one machine-readable result or `--events`
for NDJSON progress and a final report. Studio keeps serving after its readiness message.

Licensed under [Apache-2.0](LICENSE). See [third-party acknowledgements](THIRD_PARTY.md) for dependencies and references.
