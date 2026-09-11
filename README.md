# Valle

English | [简体中文](README.zh-CN.md)

A video creation and editing engine built for AI agents.

**Early development release.** We are sharing the code early. Many features, build tools, and documentation are still incomplete. Expect bugs and breaking changes as the project evolves.

## Requirements

- [Rust via rustup](https://rustup.rs/). `rust-toolchain.toml` selects Rust **1.96.0** and the `wasm32-unknown-unknown` target.
- [Bun](https://bun.com/docs/installation) **1.4.2**.
- Native build tools: C/C++ compiler, LLVM/libclang, `pkg-config`, CMake, Meson, Make, Perl, Python 3, Ninja, Git, curl, and tar (NASM on x86_64). See [rust-skia build requirements](https://github.com/rust-skia/rust-skia#building).

The setup below targets **macOS**; Linux and Windows build environments are defined in their [packaging workflows](.github/workflows). Install Rust and Bun above, then install the native dependencies with [Homebrew](https://brew.sh/):

```sh
xcode-select --install # If Command Line Tools are not installed
brew install pkgconf llvm cmake ninja meson python
export PATH="$(brew --prefix llvm)/bin:$PATH"
export LIBCLANG_PATH="$(brew --prefix llvm)/lib"
```

## Build and run

```sh
git clone https://github.com/openvalle/valle.git
cd valle
cargo xtask build
```

The first build needs network access and can take a while. Cargo builds Skia from source for Skottie support. The published `valle-ffmpeg` and `valle-ffmpeg-sys` crates include pinned FFmpeg 7.0/8.0/9.0 public headers; building needs libclang, but no FFmpeg installation or extra repository checkout.

`cargo xtask build` produces **`dist/bin/valle`** (`dist/bin/valle.exe` on Windows), an executable with the Studio Web runtime, CanvasKit full, fonts and dependency notices embedded. Use `cargo xtask build --release` for an optimized build. For CLI-only development, `cargo build -p valle-cli` skips these embedded resources; pass `--web-assets-dir /absolute/path/to/web/dist` when starting Studio.

FFmpeg is optional at runtime and is not bundled. Install FFmpeg 7.x, 8.x or 9.x **shared libraries** for media operations (`brew install ffmpeg` on macOS). Motion checks, pure Motion PNG rendering and Studio startup work without them. Run `./dist/bin/valle media capabilities` to inspect an installation; see [runtime setup](docs/cli.md#runtime-notes) for custom paths and compatibility rules.

## Package

Build on the platform you want to distribute:

```sh
# macOS arm64
cargo xtask package --min-macos 15.0
# Linux x86_64 or Windows x86_64 (MSVC)
cargo xtask package
```

The command writes the executable and `SHA256SUMS` under `target/package/`, after verifying startup, embedded assets and Motion/PNG/Studio operation with FFmpeg deliberately unavailable. macOS executables are also signed. GitHub Actions creates the distribution archive and its `.sha256` checksum. Version and platform identifiers appear only in the archive name. Each archive contains only `valle` (`valle.exe` on Windows) and `SHA256SUMS`.

| Actions workflow | Distribution archive | Executable inside |
| --- | --- | --- |
| Package macOS arm64 | `valle-vVERSION-darwin-arm64.tar.gz` | `valle` |
| Package Linux x86_64 | `valle-vVERSION-linux-x86_64.tar.gz` | `valle` |
| Package Windows x86_64 | `valle-vVERSION-windows-x86_64.zip` | `valle.exe` |

Choose a workflow under **Actions → Run workflow**. Each uploads the archive and its checksum as an artifact retained for 30 days, with build/test diagnostics retained separately; it does not publish a GitHub Release. The macOS workflow runs on `openvalle/valle`'s `main` branch and tests the final Developer ID-signed executable with FFmpeg 7/8/9 before submitting it to Apple. Once notarization returns `Accepted`, the workflow creates and uploads the archive and checksum. Packaging does not require GPU hardware tests.

Unzip the GitHub artifact, then extract the archive inside. The `.tar.gz` preserves macOS/Linux execution permissions, so no `chmod` is needed. On macOS, verify and extract with:

```sh
shasum -a 256 -c valle-vVERSION-darwin-arm64.tar.gz.sha256
tar -xzf valle-vVERSION-darwin-arm64.tar.gz
shasum -a 256 -c SHA256SUMS
./valle --help
```

On Linux, use `sha256sum -c` with the Linux filenames. On Windows, extract the inner ZIP and run `.\valle.exe --help` in PowerShell; `Get-FileHash -Algorithm SHA256` can be compared with the supplied checksum. To repeat the runtime checks on any supported host with build tools installed, run `cargo xtask verify-package /absolute/path/to/EXECUTABLE`.

macOS packages target macOS 15+. Official Actions artifacts use Developer ID signing with Hardened Runtime, a secure timestamp and Apple notarization; local `cargo xtask package` builds use ad-hoc signing. Standalone CLI executables cannot carry a stapled notarization ticket, so Gatekeeper needs network access to retrieve Apple's ticket. Linux packages build on Ubuntu 24.04 (glibc 2.39) and require system C/C++ libraries, Fontconfig, FreeType and OpenBLAS; on Ubuntu, install `libfontconfig1 libfreetype6 libopenblas0-pthread`. Windows packages use MSVC and may require the x64 Visual C++ Redistributable. ASR transcription and forced alignment are currently supported on macOS and Linux only; Windows support is planned for later. FFmpeg and model runtimes remain separate optional installations.

The macOS workflow requires `MACOS_CERT_P12_BASE64`, `MACOS_CERT_PASSWORD`, `MACOS_SIGN_IDENTITY`, `APPLE_API_KEY_ID`, `APPLE_API_ISSUER_ID` and `APPLE_API_KEY_P8_BASE64` as Actions secrets. Missing credentials, invalid signatures, rejected notarization or a notarization wait exceeding 60 minutes prevent artifact publication. Diagnostics retain the submission ID and status so a pending request can be inspected without resubmitting it. The certificate is imported into a temporary keychain, and the API private key is removed after submission processing.

`valle licenses` (or `valle --json licenses`) displays embedded notices and versioned dependency source links. Sources are available separately; publish the corresponding Valle source commit before distributing a build. See [third-party acknowledgements](THIRD_PARTY.md#binary-distribution-notices).

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

The [Motion authoring reference](docs/motion.md) covers JSX, components, CSS,
animation, resources and complete examples. The [Timeline authoring reference](docs/timeline.md)
covers clip timing, tracks, audio, captions and Motion integration.

For AI agents, the [Valle skill](skills/valle/SKILL.md) covers command selection,
authoring inputs, project revisions and result handling in one file.

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
