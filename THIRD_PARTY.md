# Third-party software

Valle builds on the projects below; thanks to their authors and maintainers. Versions are locked
in `Cargo.lock` and `bun.lock`.

## Engine

| Project | Use | License |
| --- | --- | --- |
| [OXC](https://github.com/oxc-project/oxc) | Parsing and rewriting scene modules | MIT |
| [QuickJS-ng](https://github.com/quickjs-ng/quickjs) and [rquickjs](https://github.com/DelSkayn/rquickjs) | Running scene code in a sandbox | MIT |
| [Taffy](https://github.com/DioxusLabs/taffy) | Block and flex layout | MIT |
| [Parley](https://github.com/linebender/parley) | Shaping text and breaking it into lines | MIT or Apache-2.0 |
| [Skrifa](https://github.com/googlefonts/fontations) | Reading glyph outlines | MIT or Apache-2.0 |
| [wgpu](https://github.com/gfx-rs/wgpu), with naga | Drawing on the GPU through Metal, Vulkan and Direct3D 12 | MIT or Apache-2.0 |
| [libm](https://github.com/rust-lang/compiler-builtins) | Math functions that agree on every platform | MIT or Apache-2.0 |
| [Serde](https://github.com/serde-rs/serde) and [serde_json](https://github.com/serde-rs/json) | Reading what tracing records | MIT or Apache-2.0 |

## Fonts

| Font | Use | License |
| --- | --- | --- |
| [Noto Sans](https://github.com/google/fonts/tree/2984c575fdce412ee02b2baaba67672b9a9434d8/ofl/notosans), `NotoSans[wdth,wght].ttf` unmodified | The built-in sans-serif | SIL OFL 1.1, [fonts/NotoSans-OFL.txt](fonts/NotoSans-OFL.txt) |

## Tools

| Project | Use | License |
| --- | --- | --- |
| [Bun](https://github.com/oven-sh/bun) | Running the script tools | MIT |
| [Prettier](https://github.com/prettier/prettier) | Formatting scripts | MIT |
| [Oxlint](https://github.com/oxc-project/oxc) | Linting scripts | MIT |
| [png](https://github.com/image-rs/image-png) | Reading and writing the test baselines | MIT or Apache-2.0 |
