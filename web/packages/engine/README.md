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

For a player opened on a Timeline, replace the document after an edit with
`await player.updateTimeline(timeline)`. Pass a JSON object or its serialized text. Use the
optional `{ files: { [path]: source } }` argument to supply new or replaced assets with the edit.
Use stable paths for unchanged assets so their cached media remains available. The player keeps
its current time at the new frame rate, clamps it to the new duration and resumes if it was
playing. If preparation fails, the previous edit stays open. When updates arrive faster than the
player can apply them, queued calls settle with the latest queued snapshot.

URL-based video and audio use HTTP byte ranges (`206` and `Content-Range`). For cross-origin
sources, allow the page's origin and expose `Content-Range` in CORS. When the server ignores
a range and the whole response exceeds the media cache or has unknown size, the worker
streams the response to a temporary OPFS file,
then seeks locally. This fallback requires enough browser storage for the compressed source.
The worker removes its temporary copy when it releases the asset. Valle shares the media input across clips that reference the same path and decodes the
audio windows the mixer needs. Custom fetchers for authentication or signed URLs must pass
through the request options and return a streaming response.

```ts
fetch: async (url, init) => fetch(await signedUrl(url), init);
```

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

The MP4 uses H.264 video. A separate
worker exports the committed timeline and resource bindings captured at the start of the call.
Playback and later edits can continue during export.

For a timeline with sound, set `audioCodec` (`"aac"` or `"opus"`), `audioSampleRate`
(Hz), `audioChannels` (1 or 2) and `audioBitrate` (bits/s). The default is stereo at
48000 Hz and 192000 bits/s, preferring AAC and falling back to Opus when available.
MP4 Opus uses 48000 Hz. An explicitly requested encoding must be supported by the browser.
For voice, for example, use `{ audioChannels: 1, audioBitrate: 128000 }`. For a silent MP4,
pass `{ includeAudio: false }` on its own or alongside video settings.

For long exports, write the MP4 to the origin's private file system (OPFS). The same output
options and progress callback work with either target.

```ts
const result = await exportBrowserVideo(player, {
  target: { type: "opfs" },
  signal: abortController.signal,
});
const file = await result.fileHandle.getFile();
// Upload large outputs in parts using file.slice(start, end).
// Call cleanup after the upload succeeds or when discarding the file.
await result.cleanup?.();
```

The file target writes MP4 media data as it is encoded, with bounded output buffering. It still
keeps the MP4 sample index until finalization. A successful OPFS export stays on disk so the
caller can retry uploads. The caller manages storage quota, upload credentials and retry state.
To write to a user-selected file, pick a handle in a user gesture and pass
`target: { type: "file", handle }`. Both file targets require a browser with writable file
handles and enough disk space.

## The runtime

The engine's worker, audio worklet and wasm (`worker.js`, `sound.js`, `valle_web_bg.wasm`) are
loaded from beside the module. Where a bundler moves the module and not them, serve `dist/` and
give its address as `runtime` (the `runtime` attribute of `<valle-player>`). The built-in fonts
are fetched from Google's addresses unless `fonts` names another.

## License

Apache-2.0
