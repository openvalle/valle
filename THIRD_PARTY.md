# Third-party sources and acknowledgements

We thank the authors and maintainers of the projects below. This document covers the principal dependencies, resources, adapted implementations, and design references used in the current source. Dependency versions are recorded in `Cargo.toml`, `Cargo.lock`, and `web/bun.lock`; copyright and license details remain in the accompanying LICENSE, NOTICE, and model release manifests.

## Design references

| Source | Reference in the current implementation |
| --- | --- |
| **Remotion** | Video authoring with JSX components, modules, and data, plus embeddable player design. Valle implements its own compiler and renderer and does not depend on Remotion. |
| **React / JSX** | Component-oriented authoring and JSX syntax, parsed by OXC without React DOM. |
| **Tailwind CSS** | Utility-class vocabulary and styling conventions. Valle's supported [class catalog](crates/valle-motion/src/tailwind.rs) is resolved through Takumi. |
| **Apache ECharts** | [Color parsing](crates/valle-draw/src/color.rs) references CSS/ECharts syntax. ECharts and zrender are not dependencies. |

## Adapted implementations

| Source | Use | Attribution and license |
| --- | --- | --- |
| **d3-array / d3-scale** | Ticks, nice bounds, and band scales implemented in Rust with deterministic math. | [Implementation](crates/valle-motion/src/compute/scale.rs), [NOTICE and ISC licenses](crates/valle-motion/licenses/d3) |
| **d3-shape** | Rounded-sector geometry adapted in Rust. | [Implementation](crates/valle-draw/src/draw.rs), [NOTICE](crates/valle-draw/licenses/NOTICE.txt), [ISC license](crates/valle-draw/licenses/d3-shape-ISC.txt) |
| **GL Transitions** | GLSL transitions adapted to SkSL with changes to sampling interfaces, parameters, and DreamyZoom sample counts. | [Shaders](crates/valle-draw/assets/shaders), [authors and adaptation notes](crates/valle-draw/licenses/NOTICE.txt), [MIT license](crates/valle-draw/licenses/GL-Transitions-MIT.txt) |

## Principal dependencies

| Component | Use |
| --- | --- |
| **Takumi Core / Taffy / Parley** | Motion layout, style resolution, and text measurement. |
| **cosmic-text / swash / ttf-parser** | Subtitle shaping, glyph processing, and font inspection. |
| **RaTeX** | LaTeX parsing, font metrics, and formula layout, with Valle's drawing adapter. |
| **Skia / skia-safe / Skottie / CanvasKit** | Native and Web drawing, compositing, effects, and Lottie rendering. |
| **OXC / QuickJS / rquickjs** | Motion source parsing, transforms, and preparation-stage JavaScript execution. |
| **FFmpeg / valle-ffmpeg / valle-ffmpeg-sys / MP4Box.js** | User-installed native media libraries are loaded on demand. The `valle-ffmpeg` and `valle-ffmpeg-sys` crates are maintained in [openvalle/ffmpeg-rs](https://github.com/openvalle/ffmpeg-rs), derived from the WTFPL-licensed `ffmpeg-next`/`ffmpeg-sys-next` 9.0.0 sources. They include typed FFmpeg 7/8/9 runtime adapters and pinned public headers with their upstream LGPL notices. Source hashes and changes are recorded in that repository and each published crate NOTICE. No FFmpeg libraries or codec binaries are shipped. Browser MP4 demuxing uses MP4Box.js and decoding uses WebCodecs. |
| **x264 / x265 / libvpx / SVT-AV1 / dav1d / LAME / Opus** | Optional codecs in user-provided FFmpeg installations; not bundled. |
| **ONNX Runtime / ort / qwen-asr** | Model inference and speech recognition. |
| **Apple CoreML / Metal / objc2** | macOS inference and graphics integration, including platform APIs and Rust bindings. |
| **geo / kurbo / libm / image / png / RustFFT** | Geometry, paths, deterministic math, image processing, and signal processing. |
| **Serde / serde_json / serde_jcs / Schemars / jsonschema** | Serialization, canonical JSON, and schema generation and validation. |
| **SQLite / rusqlite** | Asset indexing and full-text search. |
| **reqwest / SHA-2 / Clap / mimalloc** | Downloads, content identities, command-line parsing, and allocation. |
| **Rust / Cargo / Bun / TypeScript / wasm-bindgen / ts-rs** | Compilation, Web tooling, Wasm, and cross-language bindings. |

## Fonts and reference fixtures

| Source | Use | Attribution and license |
| --- | --- | --- |
| **KaTeX** | Nineteen TTF fonts used by the RaTeX adapter and formula regression references, without the KaTeX JavaScript runtime. | [NOTICE and licenses](assets/fonts/katex), [formula fixtures](crates/valle-motion/tests/fixtures) |
| **Noto** | Text, symbol, mathematical, monospace, and Chinese fonts, including Noto Sans CJK SC. | [Fonts and OFL licenses](assets/fonts/noto) |
| **Noto Color Emoji** | Unmodified COLRv1 emoji font, SIL Open Font License 1.1. | [Source and revision](assets/fonts/noto/README.md), [license](assets/fonts/noto/OFL-notoemoji.txt) |

## Models

Current integrations include **BiRefNet, MODNet, Demucs, DPDFNet, EdgeTAM, LaMa, OmniShotCut, TransNetV2, Real-ESRGAN, RIFE, Qwen3-ASR, and Qwen3 Aligner**. Weights are installed separately.

See the [model release manifests](crates/valle-media/src/models/catalog) for upstream sources, versions, artifact hashes, and licenses, and the [NOTICE](crates/valle-media/licenses/NOTICE.txt) for integration code provenance. Model code, weights, and conversion artifacts are governed by their respective notices.

When adding or updating third-party content, update the dependency manifests, attribution, and original copyright and license files together.

## Binary distribution notices

`valle licenses` displays the texts and versioned source links embedded by `cargo xtask build`.
[`xtask/license-supplements.json`](xtask/license-supplements.json) preserves upstream notices omitted from published package
archives, with exact package versions and retrieval URLs. Update these entries when their pinned
dependencies change. It contains license text only, not dependency implementations or source archives.
