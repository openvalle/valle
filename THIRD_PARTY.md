# Third-party software

Valle builds on the projects below; thanks to their authors and maintainers. Versions are locked
in `Cargo.lock` and `bun.lock`.

## Engine

| Project | Use | License |
| --- | --- | --- |
| [OXC](https://github.com/oxc-project/oxc) | Parsing and rewriting scene modules | MIT |
| [QuickJS-ng](https://github.com/quickjs-ng/quickjs) and [rquickjs](https://github.com/DelSkayn/rquickjs) | Running scene code in a sandbox | MIT |
| [Taffy](https://github.com/DioxusLabs/taffy) | Block, flex and grid layout | MIT |
| [Parley](https://github.com/linebender/parley) | Shaping text and breaking it into lines | MIT or Apache-2.0 |
| [Skrifa](https://github.com/googlefonts/fontations) | Reading glyph outlines | MIT or Apache-2.0 |
| [Vello CPU](https://github.com/linebender/vello) | Rasterizing color glyphs (COLR) | MIT or Apache-2.0 |
| [kurbo](https://github.com/linebender/kurbo) | The outlines of SVG strokes | MIT or Apache-2.0 |
| [wgpu](https://github.com/gfx-rs/wgpu), with naga | Drawing on the GPU through Metal, Vulkan and Direct3D 12 | MIT or Apache-2.0 |
| [png](https://github.com/image-rs/image-png), [zune-jpeg](https://github.com/etemesi254/zune-image) and [image-webp](https://github.com/image-rs/image-webp) | Decoding PNG, JPEG and WebP images | MIT or Apache-2.0; zune-jpeg also Zlib |
| [roxmltree](https://github.com/RazrFalcon/roxmltree) | Reading SVG files | MIT or Apache-2.0 |
| [wuff](https://github.com/nicoburns/wuff), with [brotli-decompressor](https://github.com/dropbox/rust-brotli-decompressor) and [flate2](https://github.com/rust-lang/flate2-rs) | Decoding WOFF and WOFF2 fonts | MIT; brotli-decompressor BSD-3-Clause or MIT; flate2 MIT or Apache-2.0 |
| [Tailwind CSS](https://github.com/tailwindlabs/tailwindcss) v4.3.3 | Its default theme, `theme.css` unmodified, its property order, and its utilities, rewritten in Rust | MIT |
| [unicode-segmentation](https://github.com/unicode-rs/unicode-segmentation) | The characters a text is split into | MIT or Apache-2.0 |
| [libm](https://github.com/rust-lang/compiler-builtins) | Math functions that agree on every platform | MIT or Apache-2.0 |
| [Serde](https://github.com/serde-rs/serde) and [serde_json](https://github.com/serde-rs/json) | Reading what tracing records | MIT or Apache-2.0 |

## Native host

| Project | Use | License |
| --- | --- | --- |
| [ureq](https://github.com/algesten/ureq) | Downloading files from the web | MIT or Apache-2.0 |
| [rustls](https://github.com/rustls/rustls), with [rustls-webpki](https://github.com/rustls/webpki) | The TLS of those downloads | Apache-2.0, ISC or MIT; rustls-webpki ISC |
| [ring](https://github.com/briansmith/ring) | The cryptography of that TLS, and the SHA-256 of downloads | Apache-2.0 and ISC |
| [webpki-roots](https://github.com/rustls/webpki-roots) | Mozilla's root certificates | CDLA-Permissive-2.0 |

## Fonts

No font ships with Valle. The built-in ones, Noto Sans, Noto Sans Italic, Noto Serif, Noto Serif
Italic, Noto Sans Mono, Noto Sans SC, Noto Sans KR and Noto Color Emoji, are downloaded from
[Google Fonts](https://fonts.google.com/noto) the first time text needs them, unmodified; they are
under the SIL Open Font License 1.1. The tests also download
[Source Sans 3](https://github.com/adobe-fonts/source-sans) in each of its formats, under the same
license.

## Tools

| Project | Use | License |
| --- | --- | --- |
| [Bun](https://github.com/oven-sh/bun) | Running the script tools | MIT |
| [Prettier](https://github.com/prettier/prettier) | Formatting scripts | MIT |
| [Oxlint](https://github.com/oxc-project/oxc) | Linting scripts | MIT |
| [png](https://github.com/image-rs/image-png) | Reading and writing the test baselines | MIT or Apache-2.0 |
| [indoc](https://github.com/dtolnay/indoc) | Indenting scene sources in tests with the code around them | MIT or Apache-2.0 |
| [Tailwind CSS](https://github.com/tailwindlabs/tailwindcss) | Compiling the classes the Tailwind baselines compare with | MIT |
| [Chrome for Testing](https://github.com/GoogleChromeLabs/chrome-for-testing), headless shell | Drawing the Chrome baselines; the tests download it, nothing ships it | BSD-3-Clause, with Chromium's third-party licenses |
