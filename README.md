# Valle

English | [简体中文](README.zh-CN.md)

A video creation and editing engine built for AI agents.

**Early development release.** We are sharing the code early. Many features, build tools, and documentation are still incomplete. Expect bugs and breaking changes as the project evolves.

## Requirements

- [Rust via rustup](https://rustup.rs/). `rust-toolchain.toml` selects Rust **1.96.0** and the `wasm32-unknown-unknown` target.
- [Bun](https://bun.com/docs/installation) **1.4.2**.
- **FFmpeg development headers and shared libraries**: `libavcodec`, `libavformat`, `libavutil`, `libswscale`, and `libswresample`. Tested with FFmpeg **8.1.1**. Installing only a standalone `ffmpeg` executable is insufficient.
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

Create `hello.motion.tsx` in the repository root:

```tsx
export default function Hello(ctx) {
  const opacity = interpolate(ctx.hold.progress, [0, 0.6], [0, 1]);

  return (
    <Scene className="relative h-full w-full flex flex-col items-center justify-center"
      style={{ backgroundColor: "#102030" }}>
      <Text style={{ fontSize: 80, fontWeight: 700, color: "#ffffff", opacity }}>
        Hello, Valle!
      </Text>
      <Text style={{ marginTop: 20, fontSize: 28, color: "#94a3b8", opacity }}>
        Create with code. Bring it to life.
      </Text>
    </Scene>
  );
}
```

Start Studio and open the URL printed in the terminal:

```sh
./dist/bin/valle motion studio hello.motion.tsx
```

Or render a video or a single frame:

```sh
./dist/bin/valle motion render hello.motion.tsx --duration 3 --size 1280x720 -o hello.mp4
./dist/bin/valle motion render hello.motion.tsx --frame 30 --size 1280x720 -o hello.png
```

Output files must not already exist. This example needs no model downloads; AI media tools require their corresponding models and, where applicable, ONNX Runtime.

Licensed under [Apache-2.0](LICENSE). See [third-party acknowledgements](THIRD_PARTY.md) for dependencies and references.
