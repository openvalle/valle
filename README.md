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

For development, `cargo xtask build-web` builds the browser assets on disk. A packaged executable
can use those instead with `--web-assets-dir /path/to/valle/web` when starting Studio.

Licensed under the [Apache License 2.0](LICENSE).
