# @openvalle/engine

[Valle](https://github.com/openvalle/valle)'s engine in the browser: a Motion scene
(`.motion.tsx`) or a Timeline (`.timeline.json`) played in a canvas, drawn with WebGPU in a
worker, and exported to MP4.

```sh
npm install @openvalle/engine
```

It needs a browser with WebGPU; without it, opening fails with the code `no-webgpu`.

## Playing

```ts
import { createPlayer } from "@openvalle/engine";

const player = await createPlayer({
  canvas: document.querySelector("canvas")!,
  entry: "works/card/card.motion.tsx",
  props: { title: "Hello" },
  loop: true,
});
player.play();
player.on("time", ({ frame }) => console.log(frame));
```

The files a scene reads are fetched from the page's address, or from `base`; `files` gives them in
place of fetching, and `fetch` fetches them another way. `player.info` is the composition, its
size, rate and length, and `player.controls` what `setProps`, `setData` and `setResource` take.
`seek`, `seekFrame`, `frame` (a PNG of a frame) and `close` do what they say.

## In markup

```html
<script type="module">
  import "@openvalle/engine/element";
</script>
<valle-player src="/works/card/card.motion.tsx" loop autoplay muted></valle-player>
```

`<valle-player>` shows the canvas and nothing else: it says `ready` once `element.player` has
`src` open, and `error`, with the error as its `detail`, where that fails.

## Exporting

```ts
import { exportBrowserVideo } from "@openvalle/engine/export";

const { blob } = await exportBrowserVideo(player, { width: 1280, height: 720 });
```

The MP4 is H.264, with its sound in AAC where the browser encodes it, else Opus. It is rendered
by a worker of its own, so the player plays on.

## The runtime

The engine's worker, audio worklet and wasm (`worker.js`, `sound.js`, `valle_web_bg.wasm`) are
loaded from beside the module. Where a bundler moves the module and not them, serve `dist/` and
give its address as `runtime` (the `runtime` attribute of `<valle-player>`). The built-in fonts
are fetched from Google's addresses unless `fonts` names another.

## License

Apache-2.0
