# valle-engine

A programmable video engine for the browser. One SDK provides Motion playback, Timeline compilation, resource management, and CanvasKit rendering. It does not include the Studio application.

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
      full: {
        glue: "/canvaskit/canvaskit.js",
        wasm: "/canvaskit/canvaskit.wasm",
      },
    },
  },
});

await player.play();
player.pause();
await player.seek(1.5);
await player.close();
```

Deploy this package's `wasm/` and `workers/` directories at the example `/valle/` location. Deploy `bin/full/canvaskit.js` and `bin/full/canvaskit.wasm` from the installed **official `canvaskit-wasm` dependency** together, or configure equivalent version-pinned URLs. These URLs are application configuration, not built-in CDN endpoints. The worker must be loadable by the browser under the application's origin and security policy.

The SDK archive contains one Engine WASM and no CanvasKit WASM, bundled font pack, or Studio files. Fonts and project assets are supplied by the host. The local CLI runtime separately keeps its CanvasKit and fonts available for offline Studio use.

Motion source (`.motion.tsx`) must first be compiled and prepared by the Native CLI/backend. The browser plays the prepared Motion artifact; this package does not expose a browser TSX compiler or the Native FFmpeg export pipeline.

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
| `valle-engine/compiler` | Timeline compile/normalize helpers without loading the playback JavaScript |
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

`build` produces both the local runtime in `web/dist/` and the npm SDK in `web/packages/engine/dist/`. Once the WASM bindings exist, `bun run build:sdk` rebuilds only the npm SDK. The generated npm manifest takes its version from the Cargo workspace and replaces workspace/catalog dependency references with published versions. Building or packing does not publish to npm.
