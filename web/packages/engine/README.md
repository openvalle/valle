# valle-engine

A programmable video engine for the browser. One SDK provides Motion playback, Motion JSX and Timeline compilation, resource management, and CanvasKit rendering. It does not include the Studio application.

## Playback

The default entry is independent of UI frameworks. Supply a compiled render package and the URLs of the runtime assets deployed by your application:

```ts
import { createBrowserValleWebPlayer } from "valle-engine";

// Your backend prepares this package, including verified resources and asset locators.
// A local Valle Studio config is one example of this format.
const config = await fetch("/api/render-package").then((response) => response.json());
const player = await createBrowserValleWebPlayer({
  ...config,
  assets: config.resourceLocators,
  canvas: document.querySelector("canvas"),
  runtimeAssets: {
    engine: {
      glue: "/valle/wasm/valle_engine.js",
      wasm: "/valle/wasm/valle_engine_bg.wasm",
    },
    workers: { productFrame: "/valle/workers/product-frame.js" },
    canvasKit: {
      glue: "/canvaskit/canvaskit.js",
      wasm: "/canvaskit/canvaskit.wasm",
    },
  },
});

await player.play();
player.pause();
await player.seek(1.5);
await player.close();
```

Deploy this package's `wasm/` and `workers/` directories at the example `/valle/` location. Deploy `bin/canvaskit.js` and `bin/canvaskit.wasm` from the installed **official `canvaskit-wasm` dependency** together, or configure equivalent version-pinned URLs. These URLs are application configuration, not built-in CDN endpoints. The worker must be loadable by the browser under the application's origin and security policy.

The SDK archive contains one Engine WASM and no CanvasKit WASM, bundled font pack, or Studio files. Fonts and project assets are supplied by the host. The local CLI runtime separately keeps its CanvasKit and fonts available for offline Studio use.

The same Engine Wasm also compiles Motion TSX/JSX in the browser. Compilation produces an artifact
and source map; playback still opens a verified fixed package with its Timeline and bound resources.
The Native FFmpeg export pipeline is outside this browser package. Browser MP4 export is available
through the separate `valle-engine/export` entry and uses WebCodecs for H.264/AAC encoding.
Mediabunny handles media demuxing, video/audio decode scheduling, keyframe extraction and MP4
muxing. Browser codecs perform compression; the engine retains Timeline rendering and PCM mixing.

```ts
import { createTimelineCompilerRuntime } from "valle-engine/compiler";

const compiler = await createTimelineCompilerRuntime({
  runtimeAssets: { engine: {
    glue: "/valle/wasm/valle_engine.js",
    wasm: "/valle/wasm/valle_engine_bg.wasm",
  } },
});
const compiled = compiler.compileMotionModules("card.motion.tsx", {
  "card.motion.tsx": `export const composition = { width: 640, height: 360, duration: 2 };
    export default function Card() { return <Scene><Text>Hello</Text></Scene>; }`,
});
console.log(compiled.artifact, compiled.sourceMap);
```

For one source string, use `compiler.compileMotionJsx(source, options)`. Both methods accept explicit
resource content hashes, prepare data, TTF/OTF font bytes, `asset://` font aliases, and frozen shader
packages in `options`. Supply the same font bytes at rendering time when `measureText()` is used;
the measured values are constants in the artifact. Compilation diagnostics throw `MotionCompileError`
with a `diagnostics` array. The compiler entry needs only the two Engine asset URLs.

## Browser export

Studio's **Export** button opens MP4 settings for file name, resolution, frame rate, video bitrate
and audio. Unsaved changes can be exported once their preview is ready. The export uses an isolated
render instance, so preview mute, volume, and seeks do not change the exported work.

```ts
import { exportBrowserVideo } from "valle-engine/export";

const result = await exportBrowserVideo(playerOptions, {
  width: 1280,
  height: 720,
  frameRate: 30,
  videoBitrate: 5_000_000,
  includeAudio: true,
  audioBitrate: 192_000,
  signal: abortController.signal,
  onProgress: ({ framesCompleted, frameCount }) => console.log(framesCompleted, frameCount),
});
// result.blob is the complete MP4. Use a blob URL to download it.
```

All delivery settings are optional. Omitted dimensions and frame rate retain the admitted receipt;
omitted video bitrate uses automatic high quality. Audio is included when present unless disabled,
with a default AAC bitrate of 192 kbps. Provide both dimensions to resize: the source fits without
stretching. Frame-rate conversion repeats or drops source frames without changing playback speed.
Bitrates are target bits per second; actual variable-bitrate file size can be smaller.

Pass a user-selected `FileSystemWritableFileStream` as `writable` to stream the MP4 directly to disk;
otherwise the encoded file is held in memory and returned as a Blob. Video frames are rendered in
order with exact frame-derived timestamps; audio is mixed in bounded one-second blocks. A slow
encoder applies backpressure without dropping frames. Cancellation releases the renderer and
encoders and aborts the file write.

The shared Rust/WASM audio engine executes the compact compiled clip plan and returns stereo PCM
arrays. JavaScript supplies decoded source planes and schedules or encodes the result; it does not
expand the plan into per-sample JSON or implement a second mixer. Source PCM caches are render-scoped
and retain two resources or the current block's active resources, whichever is larger.

AAC timing is calibrated against a short internal PCM probe, because WebCodecs does not expose
encoder priming metadata. The MP4 edit list removes priming and clips trailing padding to the
admitted sample count. The probe never enters the exported work. Media is written incrementally;
the MP4 index is appended at finalization.

This entry requires a secure context (HTTPS or localhost), even canvas dimensions, and browser
support for the requested H.264 configuration, and AAC encoding/decoding when the work has audio. Unsupported
encoding fails explicitly. Resources must be browser-accessible. Export does not call a native
rendering/encoding endpoint or load FFmpeg; Studio's existing host still serves its page and resources.

## Optional element

Install `lit` when using the Web Component, then import the optional entry:

```ts
import "valle-engine/element";
```

This registers `<valle-player>`. The default `valle-engine` entry neither imports Lit nor registers custom elements. `VallePlayerController`, available from the default entry, provides lifecycle and event handling without the component.

## Entries

| Entry | Purpose |
| --- | --- |
| `valle-engine` | Playback, lifecycle controller, Timeline compiler, and rendering APIs |
| `valle-engine/element` | Optional Lit Web Component and its options |
| `valle-engine/compiler` | Timeline and Motion JSX compilation without loading the playback JavaScript |
| `valle-engine/export` | Browser-local H.264/AAC MP4 export, progress and cancellation |
| `valle-engine/runtime-assets` | Runtime URL types and validation |
| `valle-engine/wasm/*` | Engine WASM and matching generated bindings |
| `valle-engine/workers/*` | Browser frame-planning worker |

The compiler entry currently uses the same Engine WASM as playback. It does not create a player or load CanvasKit. Internal Timeline/protocol and CanvasKit executor subpaths support the repository applications; ordinary consumers should use the entries above.

## Fonts

Fonts are resources of the work. The SDK has no bundled font bytes or mandatory font URL.
CLI packages select default faces from the text, styles and glyph coverage; dynamic text keeps
broader fallback coverage so later frames can introduce another script.

To choose a different Motion font stack, pass TTF/OTF URLs or `Uint8Array` bytes:

```ts
const player = await createBrowserValleWebPlayer({
  ...options,
  fonts: [
    "/fonts/Brand-Regular.ttf",
    "/fonts/Brand-Bold.ttf",
    "/fonts/NotoSansCJKsc-Regular.otf",
    "/fonts/Noto-COLRv1.ttf",
  ],
});
```

The list replaces the ordinary Motion text fonts. Faces keep their own family names and weights;
use those names in `fontFamily`. The order supplies fallback priority. Choose the fallback faces
needed for your text; URLs may use your server or a CDN that permits CORS. Use a direct TTF/OTF
file URL, rather than a Google Fonts CSS URL or WOFF2 file.

Loading starts with player initialization, finishes before text layout, and is reused within that
player. Omitting `fonts` uses the work's selected fonts. Font changes produce a new render identity;
there is no requirement to match a built-in Noto digest. Formula fonts and explicit `asset://` font
bindings belong to their respective resources and are preserved. CSS-loaded document fonts are
separate from the engine's font bytes. Compile-time `measureText()` values are already calculated;
choose the same authoring fonts when those measurements must match a later render.

## Repository build

From `web/`:

```sh
bun install --frozen-lockfile
bun run build
cd packages/engine/dist
npm pack
```

Building the merged Engine Wasm compiles QuickJS C sources. On macOS, install Homebrew LLVM
(`brew install llvm`); the build selects its WebAssembly-capable clang unless a C compiler is
already configured explicitly. Other hosts need a clang toolchain with a wasm32 target.

`build` produces both the local runtime in `web/dist/` and the npm SDK in `web/packages/engine/dist/`. Once the WASM bindings exist, `bun run build:sdk` rebuilds only the npm SDK. The generated npm manifest takes its version from the Cargo workspace and replaces workspace/catalog dependency references with published versions. Building or packing does not publish to npm.
