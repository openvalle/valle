# Valle

Valle is being rebuilt from scratch: one Rust engine that renders motion graphics and video
edits, written as TSX and timeline JSON, natively and in the browser through WebGPU.

The previous code is archived at the
[`archive/v0`](https://github.com/openvalle/valle/tree/archive/v0) tag.

Build a standalone executable for the current platform with `cargo xtask package`. It writes
`dist/valle` (`dist/valle.exe` on Windows), with Studio and its WebAssembly engine embedded as
Brotli-compressed resources. Copy that one file to run it outside this checkout; Rust and Bun
are needed to build the package, not to run it. Video uses separately installed FFmpeg 7, 8 or 9,
and built-in fonts download on first use.

On Linux and Windows, the package also embeds the CPU Vulkan driver of
[valle-lavapipe](https://github.com/openvalle/valle-lavapipe) for its platform, the pinned release
downloaded and checked (15 to 23 MB more; `--lavapipe <driver.br>` gives another). Without a GPU,
or with only a CPU one, Valle draws on it, calling it directly rather than through the system's
Vulkan loader, so every such machine draws the same pixels; a render's JSON result names the
adapter. The driver is decompressed once into the global cache and loaded from there; where the
cache cannot be written, Linux loads it from memory. `cargo xtask lavapipe` downloads the driver
alone: with `VALLE_LAVAPIPE` set to its path, a development build and the GPU tests draw on it
too, as CI does.

For development, `cargo xtask build-web` builds the browser assets on disk. A packaged executable
can use those instead with `--web-assets-dir /path/to/valle/web` when starting Studio.

Licensed under the [Apache License 2.0](LICENSE).
