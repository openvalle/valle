# Development

Build Valle from source, test it and package it. To use Valle, start with the
[README](../README.md) and the [CLI guide](cli.md).

## Requirements

- [Rust via rustup](https://rustup.rs/): `rust-toolchain.toml` selects Rust 1.99.0 and the
  `wasm32-unknown-unknown` target.
- [Bun](https://bun.com/docs/installation) 1.4.2.
- Native build tools: a C/C++ compiler, LLVM/libclang, `pkg-config`, CMake, Meson, Make,
  Perl, Python 3, Ninja, Git, curl and tar (NASM on x86_64); see
  [rust-skia's requirements](https://github.com/rust-skia/rust-skia#building).

On macOS, with [Homebrew](https://brew.sh/):

```sh
xcode-select --install # if the Command Line Tools are missing
brew install pkgconf llvm cmake ninja meson python
export PATH="$(brew --prefix llvm)/bin:$PATH"
export LIBCLANG_PATH="$(brew --prefix llvm)/lib"
```

The [build workflows](../.github/workflows) show the Linux and Windows setups.

## Build

```sh
cargo xtask build # add --release for an optimized build
```

This builds the Web runtime and writes `dist/bin/valle` (`valle.exe` on Windows) with Studio,
CanvasKit, fonts and license notices embedded. The first build needs network access: Cargo
downloads a prebuilt Skia or builds it from source. The `valle-ffmpeg` crates ship the FFmpeg
headers, so building needs libclang but no FFmpeg.

`cargo build -p valle-cli` builds the executable without the embedded resources; Studio then
needs `--web-assets-dir /absolute/path/to/web/dist`.

## Runtime dependencies

FFmpeg is optional and not bundled. MP4 and MOV export, media commands, and a Timeline with
audio or video need FFmpeg 7, 8 or 9 **shared libraries**; a standalone `ffmpeg` executable is
not enough. Motion checks, PNG frames and Studio work without them.

| Platform | Install FFmpeg shared libraries |
| --- | --- |
| macOS | `brew install ffmpeg` |
| Linux | `sudo apt install ffmpeg` where the distribution ships FFmpeg 7 or newer (Debian 13, Ubuntu 26.04). Otherwise extract a shared build such as [BtbN](https://github.com/BtbN/FFmpeg-Builds/releases)'s `linux64-gpl-shared` and set `VALLE_FFMPEG_DIR` to its folder |
| Windows | `winget install Gyan.FFmpeg.Shared`, or BtbN's `win64-gpl-shared` build with its `bin` folder on `PATH` or in `VALLE_FFMPEG_DIR` |

`valle media capabilities` shows what loads. Media commands run local models:
`valle models install ID` downloads the weights, and the ONNX-based tools also need ONNX
Runtime 1.28, beside `valle` or named by `ORT_DYLIB_PATH`; see the
[Media guide](media.md#models-and-runtimes).

## Repository

| Path | Contents |
| --- | --- |
| `crates/valle-cli` | The `valle` executable and Studio's local server |
| `crates/valle-compiler` | Motion TSX and Timeline compilation |
| `crates/valle-motion` | Motion evaluation, layout, text and Scene3D |
| `crates/valle-engine` | Preparation and composition shared by native and Web |
| `crates/valle-render` | Native Skia rendering, scheduling and output |
| `crates/valle-draw` | Geometry, drawing commands and deterministic math |
| `crates/valle-timeline`, `crates/valle-project` | Timeline types and schema; projects and the asset library |
| `crates/valle-media` | FFmpeg decoding and encoding, and the model-backed tools |
| `web/packages/engine`, `web/apps/studio` | The browser SDK ([guide](../web/packages/engine/README.md)) and Studio |
| `docs`, `skills/valle` | The guides that `valle docs` embeds, and the agent skill |
| `xtask` | Build and packaging |

## Test

`cargo xtask build` also writes the Web runtime and license notices that the CLI embeds, so
build first, then run from the repository root:

```sh
export VALLE_EMBED_WEB_DIR="$PWD/web/dist"
export VALLE_EMBED_NOTICES="$PWD/target/distribution-notices.txt"
cargo test --workspace --features valle-cli/embedded-runtime --tests
cargo fmt --all --check
(cd web && bun run gate) # typecheck, build, test and check generated code
```

- Codec, media and video delivery tests need the FFmpeg shared libraries; `VALLE_FFMPEG_DIR`
  selects the installation under test.
- Web tests that compare against native rendering run `target/debug/valle`, or the executable
  named by `VALLE_TEST_CLI`.
- Tests that need GPU hardware, real model weights or several FFmpeg versions are ignored by
  default; run them by name with `-- --ignored`.

Some files are generated, and the checks fail when they are stale. Regenerate them with:

| Generated | Command |
| --- | --- |
| Timeline JSON Schema | `cargo run -p valle-timeline --features schema --bin valle-schema-gen` |
| Web protocol types | `cd web && bun run generate:protocol` |
| The Unsupported CSS tables in `docs/motion.md` | `VALLE_UPDATE_DOCS=1 cargo test -p valle-motion --test style_suggestions` |

## Continuous integration

The six `CI <OS> <architecture>` workflows run on demand, for example
`gh workflow run ci-macos-arm64.yml --ref main`. Each builds and verifies a package, then runs
the Rust tests and the codec tests: against FFmpeg 7, 8 and 9 on macOS, and FFmpeg 9 on Linux
and Windows. macOS arm64 also runs every Web test. Formatting and the Web SDK package are
checked on every pull request and push to `main`.

## Package

```sh
cargo xtask package                                                # Linux or Windows, arm64 or x86_64
cargo xtask package --min-macos 15.0                               # macOS arm64
cargo xtask package --target x86_64-apple-darwin --min-macos 15.0  # macOS x86_64, on an Intel Mac
cargo xtask verify-package /absolute/path/to/valle
```

`package` writes the executable and `SHA256SUMS` to `target/package/`, after checking startup,
the embedded assets, and Motion, PNG and Studio with FFmpeg deliberately unavailable.
`verify-package` repeats those checks on any executable. macOS builds are signed ad hoc.

Release builds come from the manually dispatched `Package <OS> <architecture>` workflows. They
upload an archive and its checksum as an artifact kept for 30 days, not a GitHub Release. The
macOS workflows sign with Developer ID and require Apple's notarization; their secrets are
listed in `.github/workflows/build-macos.yml`. `Package Web engine SDK` builds the browser SDK
archive.

A package needs macOS 15 or later; on Linux, glibc 2.39 with Fontconfig and FreeType; on
Windows, possibly the Visual C++ Redistributable. `valle licenses` prints the embedded
dependency notices and their source links; see the
[third-party acknowledgements](../THIRD_PARTY.md).

Studio is the only shipped Web app and is served at `/`. Console has been removed.
The playback-only Preview fixture lives in `web/tests/preview`; `bun run build:test-preview`
outputs it under `web/test-dist`, outside the public runtime manifest and distribution.
`bun web/tests/project-studio-browser.ts` verifies the project home, imports, library
metadata, source reload/conflict, native exports, restore, folder moves, and Motion conversion
in a real Chrome process. The Rust `motion_studio` suite includes this flow. Build the CLI
and `web/dist` first; browser evidence is retained in a `valle-project-studio-*` temp directory.
