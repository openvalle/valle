// The browser test layer: the engine's SDK, as `cargo xtask build-web` builds it, plays the
// examples in Playwright's Chromium with WebGPU. Its canvas shows the frames the native engine
// draws, within a step of rounding.
//
//   bun web/tests/player.ts <valle binary>
//
// The native frames are drawn first, which puts the built-in fonts in the cache; the page fetches
// them from there, so the test needs no external network after the browser is installed.

import { mkdtemp, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { cache, evaluate as evaluatePage, launch, root } from "./browser.ts";

/** The examples and frames compared, and the props bound once the player is open. */
const CASES: {
  entry: string;
  frames: number[];
  props?: Record<string, unknown>;
  pixels?: { maxDifference: number; maxPixels: number };
}[] = [
  { entry: "examples/hello.motion.tsx", frames: [0, 60] },
  { entry: "examples/01-kinetic-title.motion.tsx", frames: [45] },
  { entry: "examples/02-logo-sting.motion.tsx", frames: [165, 270] },
  { entry: "examples/03-data-dashboard.motion.tsx", frames: [120] },
  { entry: "examples/04-ui-showcase.motion.tsx", frames: [150] },
  { entry: "examples/05-story-vertical.motion.tsx", frames: [150] },
  { entry: "examples/06-memphis-loop.motion.tsx", frames: [30] },
  { entry: "examples/07-route-map.motion.tsx", frames: [150] },
  // Reviewed native/WASM captures differ at 3 isolated pixels by at most 13 levels in the
  // rotating translucent ray path. Removing that path makes both frames pixel-identical.
  {
    entry: "examples/08-celebrate.motion.tsx",
    frames: [90, 198],
    pixels: { maxDifference: 16, maxPixels: 4 },
  },
  { entry: "examples/launch.motion.tsx", frames: [150] },
  { entry: "tests/cases/motion/spring/scene.motion.tsx", frames: [0, 6, 4, 10] },
  { entry: "tests/cases/motion/noise/scene.motion.tsx", frames: [0, 4, 2] },
  { entry: "tests/cases/motion/particles/scene.motion.tsx", frames: [0, 7, 8, 9] },
  { entry: "examples/09-cards-3d.motion.tsx", frames: [210] },
  { entry: "examples/10-lower-third.motion.tsx", frames: [40] },
  {
    entry: "examples/10-lower-third.motion.tsx",
    frames: [40],
    props: { name: "Ada Lovelace", accent: "#00C2A8" },
  },
  { entry: "examples/edit.timeline.json", frames: [20] },
  { entry: "tests/resources/shaders/scene.motion.tsx", frames: [0, 2, 3, 1] },
  { entry: "tests/resources/shaders/transitions.motion.tsx", frames: [0, 1, 2, 3, 4] },
  { entry: "tests/resources/shaders/grade.timeline.json", frames: [0] },
  {
    entry: "tests/resources/timeline/captions/animations.timeline.json",
    frames: [6, 57, 111, 153, 207, 249, 282],
  },
  {
    entry: "tests/resources/timeline/captions/layout.timeline.json",
    frames: [0, 60, 120],
  },
  {
    entry: "tests/resources/timeline/captions/styled.timeline.json",
    frames: [15, 60, 75, 105, 120, 150, 179, 210],
  },
  {
    entry: "tests/resources/timeline/captions/boundaries.timeline.json",
    frames: [45, 75],
  },
];
/** The most a channel may differ, and the share of pixels that may differ by more than 1. */
const STEP = 2;
const SHARE = 0.001;

const valle = process.argv[2];
if (!valle) {
  throw new Error("usage: bun web/tests/player.ts <valle binary>");
}

const PAGE = `<!doctype html>
<style>html, body { margin: 0; } canvas { display: block; }</style>
<canvas></canvas>
<script type="module">
  import { createPlayer } from "/web/packages/engine/dist/index.js";
  import { exportBrowserVideo } from "/web/packages/engine/dist/export.js";
  import "/web/packages/engine/dist/element.js";
  /** The files players fetched through the page's \`fetch\`. */
  window.fetched = 0;
  const fetching = (url) => (window.fetched++, fetch(url));
  /** The pixels of a frame the native engine drew. */
  const decoded = async (url) => {
    const bitmap = await createImageBitmap(await (await fetch(url)).blob(), {
      colorSpaceConversion: "none",
      premultiplyAlpha: "none",
    });
    const c = new OffscreenCanvas(bitmap.width, bitmap.height).getContext("2d");
    c.drawImage(bitmap, 0, 0);
    return c.getImageData(0, 0, bitmap.width, bitmap.height).data;
  };
  /** Opens \`entry\`, draws frame \`index\`, and says how its pixels differ from \`native\`'s: the
   * most a channel does, and how many pixels do at all. */
  window.check = async (entry, index, native, props) => {
    await window.player?.close();
    const canvas = document.createElement("canvas");
    document.querySelector("canvas").replaceWith(canvas);
    const started = performance.now();
    window.player = await createPlayer({ canvas, entry, base: "/", fonts: "/fonts/", fetch: fetching });
    if (props) await window.player.setProps(props);
    // The canvas laid out at its size now, not once the worker's first frame reaches the page.
    const { width, height } = window.player.info;
    canvas.style.width = width + "px";
    canvas.style.height = height + "px";
    await window.player.draw(index);
    const ms = Math.round(performance.now() - started);
    const [x, y] = [await window.player.pixels(), await decoded(native)];
    if (x.length !== y.length) return { size: true, ms };
    let [most, differ] = [0, 0];
    /** Where pixels differ by more than 1: left, top, right and bottom. */
    const box = [Infinity, Infinity, -1, -1];
    for (let i = 0; i < x.length; i += 4) {
      // A 2D canvas keeps colors premultiplied: those of pixels not opaque come back rounded.
      const channels = x[i + 3] === 255 && y[i + 3] === 255 ? [0, 1, 2, 3] : [3];
      let d = 0;
      for (const k of channels) d = Math.max(d, Math.abs(x[i + k] - y[i + k]));
      most = Math.max(most, d);
      if (d > 1) {
        differ++;
        const [px, py] = [(i / 4) % width, Math.floor(i / 4 / width)];
        [box[0], box[1]] = [Math.min(box[0], px), Math.min(box[1], py)];
        [box[2], box[3]] = [Math.max(box[2], px), Math.max(box[3], py)];
      }
    }
    // A transferred canvas's DOM width/height may still be the initial 300x150 here.
    // Use the composition size for the screenshot, as for its CSS size above.
    return { most, differ, box, pixels: x.length / 4, ms, size: [width, height] };
  };
  /** The frame the player drew last, as a PNG's data address. */
  window.drawn = async () => {
    const { width, height } = window.player.info;
    const c = new OffscreenCanvas(width, height);
    const pixels = new Uint8ClampedArray(await window.player.pixels());
    c.getContext("2d").putImageData(new ImageData(pixels, width, height), 0, 0);
    const bytes = new Uint8Array(await (await c.convertToBlob()).arrayBuffer());
    return "data:image/png;base64," + btoa(Array.from(bytes, (b) => String.fromCharCode(b)).join(""));
  };
  /** How far a player's clock advances in a second of playback, and how many frames
   * it drew on the way: as many as the page's animation frames, which a page in the background
   * has fewer of. */
  window.played = async () => {
    const shown = new Set();
    const stats = [];
    const offs = [
      window.player.on("time", ({ frame }) => shown.add(frame)),
      window.player.on("stats", (s) => stats.push(s)),
    ];
    // Audio initialization is asynchronous. Measure playback once its clock starts;
    // the host's evaluation timeout still fails a player that never starts.
    const started = new Promise((resolve) => {
      offs.push(window.player.on("time", ({ frame }) => { if (frame > 0) resolve(); }));
    });
    window.player.seekFrame(0);
    window.player.play();
    await started;
    const from = window.player.currentTime;
    await new Promise((r) => setTimeout(r, 1000));
    window.player.pause();
    for (const off of offs) off();
    const counted = stats.reduce((n, s) => n + s.shown, 0);
    return { advanced: window.player.currentTime - from, drawn: shown.size, counted, memory: stats.at(-1)?.memory };
  };
  /** How the sound of \`entry\` differs from the native mix in the WAV at \`mix\`, and how far its
   * clock advances in a second of playback. */
  window.heard = async (entry, mix) => {
    await window.player?.close();
    const canvas = document.createElement("canvas");
    document.querySelector("canvas").replaceWith(canvas);
    const player = (window.player = await createPlayer({ canvas, entry, base: "/", fonts: "/fonts/" }));
    const native = new Float32Array(await (await (await fetch(mix)).arrayBuffer()).slice(44));
    const mine = await player.renderSound(0, native.length / 2 / 48000);
    let most = mine.length === native.length ? 0 : Infinity;
    for (let i = 0; i < native.length; i++) most = Math.max(most, Math.abs(mine[i] - native[i]));
    const { advanced } = await window.played();
    return { sound: player.hasSound, samples: native.length / 2, most, advanced };
  };
  /** Whether binding a prop to a value its control does not take fails, and leaves the frame
   * as it was. */
  window.invalid = async (index) => {
    const before = await window.player.pixels();
    try {
      await window.player.setProps({ accent: "not a color" });
      return { rejected: false };
    } catch (error) {
      await window.player.draw(index);
      const after = await window.player.pixels();
      const same = before.length === after.length && before.every((b, i) => b === after[i]);
      return { rejected: true, code: error.code, message: error.message, same };
    }
  };
  /** The MP4 \`entry\` exports as \`options\` say, as a data address, how long it took and how
   * many frames it said it had done of how many. */
  window.exported = async (entry, options = {}) => {
    await window.player?.close();
    const canvas = document.createElement("canvas");
    document.querySelector("canvas").replaceWith(canvas);
    const player = (window.player = await createPlayer({ canvas, entry, base: "/", fonts: "/fonts/" }));
    let [done, count] = [0, 0];
    const started = performance.now();
    const { blob: mp4 } = await exportBrowserVideo(player, {
      ...options,
      onProgress: ({ framesCompleted, frameCount }) => ([done, count] = [framesCompleted, frameCount]),
    });
    const ms = Math.round(performance.now() - started);
    const url = await new Promise((r) => {
      const reader = new FileReader();
      reader.onload = () => r(reader.result);
      reader.readAsDataURL(mp4);
    });
    return { url, ms, done, count, frames: player.info.frames };
  };
  /** How an export of what the player has open, cancelled after 3 frames, fails. */
  window.cancelled = async () => {
    const abort = new AbortController();
    try {
      await exportBrowserVideo(window.player, {
        signal: abort.signal,
        onProgress: ({ framesCompleted }) => framesCompleted === 3 && abort.abort(),
      });
      return "not cancelled";
    } catch (error) {
      return error.code;
    }
  };
  /** A <valle-player> of \`src\`, looping, playing on its own, muted: whether it plays, and how
   * its frame at 1 s differs from \`native\`'s; and whether taking it out of the page closes
   * its player. */
  window.element = async (src, native) => {
    await window.player?.close();
    window.player = undefined;
    const element = document.createElement("valle-player");
    element.setAttribute("src", src);
    element.setAttribute("runtime", "/web/packages/engine/dist/");
    for (const name of ["loop", "autoplay", "muted"]) element.setAttribute(name, "");
    const ready = new Promise((resolve, reject) => {
      element.addEventListener("ready", resolve, { once: true });
      element.addEventListener("error", (event) => reject(event.detail), { once: true });
    });
    document.body.append(element);
    await ready;
    const player = element.player;
    await new Promise((r) => setTimeout(r, 700));
    const [playing, at, muted, loop] = [player.playing, player.currentTime, player.muted, player.loop];
    const png = URL.createObjectURL(await player.frame(1));
    const [x, y] = [await decoded(png), await decoded(native)];
    let most = x.length === y.length ? 0 : Infinity;
    for (let i = 0; i < x.length && most !== Infinity; i++) most = Math.max(most, Math.abs(x[i] - y[i]));
    element.remove();
    return { playing, at, muted, loop, most, closed: element.player === undefined };
  };
  /** How two frames differ: the most a channel does, and on average. */
  window.similar = async (a, b) => {
    const [x, y] = [await decoded(a), await decoded(b)];
    if (x.length !== y.length) return { most: Infinity, mean: Infinity };
    let [most, sum, inkSum, inkChannels] = [0, 0, 0, 0];
    for (let i = 0; i < x.length; i += 4) {
      for (let k = 0; k < 3; k++) {
        const d = Math.abs(x[i + k] - y[i + k]);
        most = Math.max(most, d);
        sum += d;
        // Bright caption glyphs against a dark background: the full-frame mean alone
        // can hide missing words because text occupies little of the canvas.
        if (Math.max(y[i], y[i + 1], y[i + 2]) > 128) {
          inkSum += d;
          inkChannels++;
        }
      }
    }
    return { most, mean: sum / (x.length / 4) / 3, inkMean: inkChannels ? inkSum / inkChannels : 0, inkPixels: inkChannels / 3 };
  };
  /** How the frame on screen differs from \`native\`'s where it is opaque: the canvas is. */
  window.shown = async (screenshot, native) => {
    const [x, y] = [await decoded(screenshot), await decoded(native)];
    if (x.length !== y.length) return { most: Infinity, differ: Infinity };
    let most = 0, differ = 0;
    for (let i = 0; i < x.length; i += 4) {
      if (y[i + 3] !== 255) continue;
      let d = 0;
      for (let k = 0; k < 3; k++) d = Math.max(d, Math.abs(x[i + k] - y[i + k]));
      most = Math.max(most, d);
      if (d > 1) differ++;
    }
    return { most, differ };
  };
</script>
`;

/** The timeline with sound. */
const SOUND = {
  canvas: { width: 320, height: 180, fps: 30 },
  resources: { tone: "tone.wav" },
  tracks: {
    visual: [{ clips: [{ kind: "solid", color: "#204060ff", start: 0, duration: 2 }] }],
    audio: [{ clips: [{ src: "tone", start: 0.25, duration: 1.5, gain: 0.8 }] }],
  },
};

/** A WAV of a sweep from `hertz` up an octave a second, for `seconds`: no two stretches of it
 * alike, so how late a copy of it is shows. Stereo 32-bit floats at 48 000 samples a second. */
function tone(seconds: number, hertz: number): Uint8Array {
  const samples = seconds * 48_000;
  const wav = new DataView(new ArrayBuffer(44 + samples * 8));
  const text = (at: number, s: string) =>
    [...s].forEach((c, i) => wav.setUint8(at + i, c.charCodeAt(0)));
  text(0, "RIFF");
  wav.setUint32(4, 36 + samples * 8, true);
  text(8, "WAVEfmt ");
  wav.setUint32(16, 16, true);
  wav.setUint16(20, 3, true);
  wav.setUint16(22, 2, true);
  wav.setUint32(24, 48_000, true);
  wav.setUint32(28, 48_000 * 8, true);
  wav.setUint16(32, 8, true);
  wav.setUint16(34, 32, true);
  text(36, "data");
  wav.setUint32(40, samples * 8, true);
  for (let i = 0; i < samples; i++) {
    const t = i / 48_000;
    const v = 0.5 * Math.sin((2 * Math.PI * hertz * (2 ** t - 1)) / Math.LN2);
    wav.setFloat32(44 + i * 8, v, true);
    wav.setFloat32(48 + i * 8, v, true);
  }
  return new Uint8Array(wav.buffer);
}

/** A scene that shows the edit's export, at the edit's size and rate. */
const SHOWN = `export const composition = { width: 640, height: 360, fps: 30, duration: 3 };
export const controls = {
  resources: { clip: resource({ kind: "video", default: "edit.mp4" }) },
};
export default function Shown() {
  return <Video src="resource://clip" style={{ width: 640, height: 360 }} />;
}
`;

/** A timeline that sounds the export of the timeline with sound. */
const LISTEN = {
  canvas: { width: 320, height: 180, fps: 30 },
  resources: { exported: "sound.mp4" },
  tracks: { audio: [{ clips: [{ src: "exported", start: 0, duration: 2 }] }] },
};

/** The samples of a WAV the native engine wrote: 32-bit floats after a 44-byte header. */
async function samples(path: string): Promise<Float32Array> {
  const bytes = await Bun.file(path).arrayBuffer();
  return new Float32Array(bytes.slice(44));
}

/** How many samples late `heard` is against `mixed`, at most 4096 either way, and how far apart
 * the two are then, as the RMS of their difference over the middle second. */
function aligned(heard: Float32Array, mixed: Float32Array): { lag: number; rms: number } {
  const apart = (lag: number) => {
    let sum = 0;
    for (let i = 24_000; i < 72_000; i++) {
      const d = (heard[2 * (i + lag)] ?? 0) - (mixed[2 * i] ?? 0);
      sum += d * d;
    }
    return Math.sqrt(sum / 48_000);
  };
  let best = { lag: 0, rms: apart(0) };
  for (let lag = -4096; lag <= 4096; lag++) {
    const rms = apart(lag);
    if (rms < best.rms) {
      best = { lag, rms };
    }
  }
  return best;
}

/** A scene for `<valle-player>`, without text: a square that moves. */
const ELEMENT = `export const composition = { width: 320, height: 180, fps: 30, duration: 2 };
export default function Element() {
  const time = useCurrentTime();
  const left = interpolate(time.progress, [0, 1], [0, 240]);
  return (
    <View className="relative h-full w-full" style={{ backgroundColor: "#203040" }}>
      <View style={{ position: "absolute", left, top: 50, width: 80, height: 80, backgroundColor: "#e0a030" }} />
    </View>
  );
}
`;

/** The width and height of the H.264 video of an MP4, from its `avc1` sample entry, and whether
 * it has a track of sound. */
function mp4(bytes: Uint8Array): { size: [number, number]; sound: boolean } {
  const text = Buffer.from(bytes).toString("latin1");
  // The sample entry in `stsd`: `avc1` is a brand in `ftyp` too.
  const at = text.indexOf("avc1", text.indexOf("stsd"));
  const view = new DataView(bytes.buffer, bytes.byteOffset);
  const size: [number, number] =
    at < 0 ? [0, 0] : [view.getUint16(at + 28), view.getUint16(at + 30)];
  return { size, sound: text.includes("mp4a") || text.includes("Opus") };
}

/** A frame's name: its entry, its index, and the props bound. */
function named(entry: string, frame: number, props?: Record<string, unknown>): string {
  return `${entry}#${frame}${props ? ` ${JSON.stringify(props)}` : ""}`;
}

/** The built-in fonts in the cache, by their file names. */
async function fonts(): Promise<Map<string, string>> {
  const dir = join(cache, "files");
  const found = new Map<string, string>();
  for (const sha of await readdir(dir).catch(() => [] as string[])) {
    for (const name of await readdir(join(dir, sha)).catch(() => [] as string[])) {
      found.set(name, join(dir, sha, name));
    }
  }
  return found;
}

async function main() {
  const out = await mkdtemp(join(tmpdir(), "valle-player-"));
  const natives = new Map<string, string>();
  for (const { entry, frames, props } of CASES) {
    const kind = entry.endsWith(".timeline.json") ? "timeline" : "motion";
    const bound = props ? ["--props", JSON.stringify(props)] : [];
    for (const frame of frames) {
      const png = join(out, `${natives.size}.png`);
      // Decode video with FFmpeg's own decoder: a virtual machine's hardware decoder hands
      // frames over unlike a real one's.
      const run = Bun.spawnSync(
        [
          valle,
          kind,
          "render",
          entry,
          "--frame",
          String(frame),
          ...bound,
          "--decoder",
          "ffmpeg",
          "-o",
          png,
        ],
        { cwd: root, stdout: "pipe", stderr: "pipe" },
      );
      if (run.exitCode !== 0) {
        throw new Error(`valle ${kind} render ${entry}: ${run.stderr.toString()}`);
      }
      natives.set(named(entry, frame, props), png);
    }
  }
  /** Runs the native engine. */
  const run = (args: string[]) => {
    const done = Bun.spawnSync([valle, ...args], { cwd: root, stdout: "pipe", stderr: "pipe" });
    if (done.exitCode !== 0) {
      throw new Error(`valle ${args.join(" ")}: ${done.stderr.toString()}`);
    }
  };
  // A timeline with sound: a tone, its own file, which both engines mix.
  const sound = join(out, "sound");
  await Bun.write(join(sound, "tone.wav"), tone(2, 440));
  await Bun.write(join(sound, "sound.timeline.json"), JSON.stringify(SOUND));
  const mixed = Bun.spawnSync(
    [valle, "timeline", "render", join(sound, "sound.timeline.json"), "-o", join(out, "mix.wav")],
    { cwd: root, stdout: "pipe", stderr: "pipe" },
  );
  if (mixed.exitCode !== 0) {
    throw new Error(`valle timeline render: ${mixed.stderr.toString()}`);
  }
  const cached = await fonts();
  const server = Bun.serve({
    hostname: "127.0.0.1",
    port: 0,
    fetch(request) {
      const path = decodeURIComponent(new URL(request.url).pathname);
      if (path === "/") {
        return new Response(PAGE, { headers: { "content-type": "text/html; charset=utf-8" } });
      }
      const font = /^\/fonts\/([\w.-]+)$/.exec(path);
      if (font) {
        const file = cached.get(font[1]);
        return file ? new Response(Bun.file(file)) : new Response("not found", { status: 404 });
      }
      if (path === "/mix.wav" || path.startsWith("/sound/") || path.startsWith("/out/")) {
        if (path.startsWith("/out/")) {
          return new Response(Bun.file(join(out, path.slice("/out/".length))));
        }
        return new Response(Bun.file(join(out, path.slice(1))));
      }
      const frame = /^\/native\/(.+)$/.exec(path);
      if (frame && natives.has(frame[1])) {
        return new Response(Bun.file(natives.get(frame[1])!));
      }
      const file =
        /^\/((?:examples|tests\/(?:cases|resources)|web\/packages\/engine\/dist)\/[\w./-]+)$/.exec(
          path,
        );
      if (file && !file[1].includes("..")) {
        const f = Bun.file(join(root, file[1]));
        const type = file[1].endsWith(".js") ? { "content-type": "text/javascript" } : undefined;
        return f
          .exists()
          .then((there) =>
            there ? new Response(f, { headers: type }) : new Response("not found", { status: 404 }),
          );
      }
      return new Response("not found", { status: 404 });
    },
  });
  const browser = await launch().catch((error) => {
    server.stop(true);
    throw error;
  });
  const failed: string[] = [];
  try {
    // Room for the largest composition at 1 px a pixel.
    const page = await browser.newPage({
      viewport: { width: 1920, height: 1920 },
      deviceScaleFactor: 1,
      colorScheme: "light",
    });
    console.log(`browser: Chromium ${browser.version()}`);
    page.on("console", (message) => console.log(`console.${message.type()}: ${message.text()}`));
    page.on("pageerror", (error) => console.log(`exception: ${error.message}`));
    await page.goto(`http://127.0.0.1:${server.port}/`);
    const evaluate = (expression: string) => evaluatePage(page, expression);
    for (const { entry, frames, props, pixels } of CASES) {
      for (const frame of frames) {
        const name = named(entry, frame, props);
        const native = `/native/${encodeURIComponent(name)}`;
        let d;
        try {
          d = await evaluate(
            `check(${JSON.stringify(entry)}, ${frame}, ${JSON.stringify(native)}, ${JSON.stringify(props)})`,
          );
        } catch (error) {
          failed.push(`${name}: ${String(error)}`);
          continue;
        }
        const line =
          d.size === true
            ? "a frame of another size"
            : `most ${d.most}, ${d.differ} of ${d.pixels} pixels by more than 1${d.differ ? `, within ${d.box.join(",")}` : ""}`;
        console.log(`${name}: ${line} (${d.ms} ms)`);
        const step = pixels?.maxDifference ?? STEP;
        const maxPixels = pixels?.maxPixels ?? d.pixels * SHARE;
        if (d.size === true || d.most > step || d.differ > maxPixels) {
          const url: string = await evaluate("drawn()");
          const png = Buffer.from(url.slice(url.indexOf(",") + 1), "base64");
          await Bun.write(
            join(out, `${natives.get(name)!.slice(out.length + 1, -4)}.browser.png`),
            png,
          );
          failed.push(`${name}: ${line}`);
          continue;
        }
        // The page shows it too.
        const [width, height] = d.size;
        // The canvas shows a frame the worker drew some animation frames later: how many is
        // the browser's, so the page is looked at again for up to 2 s.
        let shown = { most: Infinity, differ: Infinity };
        let screen = "";
        for (
          let tries = 0;
          tries < 20 && (shown.most > step || shown.differ > maxPixels);
          tries++
        ) {
          await evaluate(`new Promise((r) => setTimeout(() => requestAnimationFrame(r), 100))`);
          const png = await page.screenshot({
            clip: { x: 0, y: 0, width, height },
          });
          screen = png.toString("base64");
          shown = await evaluate(
            `shown(${JSON.stringify(`data:image/png;base64,${screen}`)}, ${JSON.stringify(native)})`,
          );
        }
        if (shown.most > step || shown.differ > maxPixels) {
          const png = natives.get(name)!.slice(0, -4);
          await Bun.write(`${png}.screen.png`, Buffer.from(screen, "base64"));
          failed.push(
            `${name}: on screen, most ${shown.most}, ${shown.differ} pixels by more than 1`,
          );
        }
        // A value its control does not take fails, and what was bound stays.
        if (props) {
          const invalid = await evaluate(`invalid(${frame})`);
          console.log(`invalid prop: ${JSON.stringify(invalid)}`);
          if (!invalid.rejected || !invalid.same) {
            failed.push(`binding an invalid prop: ${JSON.stringify(invalid)}`);
          }
        }
      }
    }
    // The last player plays on its own clock.
    const played = await evaluate("played()");
    console.log(
      `played: advanced ${played.advanced.toFixed(2)} s in 1 s, ${played.drawn} frames drawn, ${played.counted} counted, ${played.memory} bytes of memory`,
    );
    if (played.advanced < 0.5 || played.drawn < 2 || played.counted < 2 || !(played.memory > 0)) {
      failed.push(`playing for 1 s: ${JSON.stringify(played)}`);
    }
    const fetched = await evaluate("fetched");
    console.log(`fetched through the page's fetch: ${fetched} files`);
    if (fetched === 0) {
      failed.push("no file was fetched through the page's fetch");
    }
    // A timeline's sound: the native mix, and played on its clock.
    const heard = await evaluate(`heard("sound/sound.timeline.json", "/mix.wav")`);
    console.log(
      `sound: ${heard.samples} samples, most ${heard.most.toExponential(1)} apart; played: advanced ${heard.advanced.toFixed(2)} s in 1 s on the sound's clock`,
    );
    if (!heard.sound || heard.most > 1e-6 || heard.advanced < 0.5) {
      failed.push(`sound: ${JSON.stringify(heard)}`);
    }
    // Exports: the browser's MP4s, which the native engine decodes.
    for (const [entry, file] of [
      ["examples/edit.timeline.json", "edit.mp4"],
      ["sound/sound.timeline.json", "sound.mp4"],
      ["tests/resources/timeline/captions/animations.timeline.json", "captions.mp4"],
    ]) {
      const e = await evaluate(`exported(${JSON.stringify(entry)})`);
      await Bun.write(join(out, file), Buffer.from(e.url.slice(e.url.indexOf(",") + 1), "base64"));
      console.log(`exported ${entry}: ${e.frames} frames in ${e.ms} ms`);
      if (e.done !== e.frames || e.count !== e.frames) {
        failed.push(`exporting ${entry} said ${e.done} of ${e.count} frames were done`);
      }
    }
    // Reimport the subtitle export through the native decoder and check each animation,
    // including the final blank frame. This also validates the complete exported duration.
    const captionEntry = "tests/resources/timeline/captions/animations.timeline.json";
    const captionImport = join(out, "captions.timeline.json");
    await Bun.write(
      captionImport,
      JSON.stringify({
        canvas: { width: 640, height: 360, fps: 30 },
        resources: { video: "captions.mp4" },
        tracks: {
          visual: [{ items: [{ id: "video", kind: "video", src: "video", duration: 9.6 }] }],
        },
      }),
    );
    for (const frame of [6, 57, 111, 153, 207, 249, 282]) {
      const file = `caption-${frame}.png`;
      run([
        "timeline",
        "render",
        captionImport,
        "--decoder",
        "ffmpeg",
        "--frame",
        String(frame),
        "-o",
        join(out, file),
      ]);
      const native = `/native/${encodeURIComponent(named(captionEntry, frame))}`;
      const d = await evaluate(
        `similar(${JSON.stringify(`/out/${file}`)}, ${JSON.stringify(native)})`,
      );
      console.log(`caption export frame ${frame}: ${JSON.stringify(d)}`);
      if (d.mean > 1.5 || d.inkMean > 12 || (d.inkPixels === 0 && d.most > 8)) {
        failed.push(`subtitle export frame ${frame}: ${JSON.stringify(d)}`);
      }
    }
    // An export at another size and rate, without its sound, and one cancelled.
    const options = { width: 160, frameRate: 15, includeAudio: false };
    const small = await evaluate(
      `exported("sound/sound.timeline.json", ${JSON.stringify(options)})`,
    );
    const smallMp4 = mp4(Buffer.from(small.url.slice(small.url.indexOf(",") + 1), "base64"));
    console.log(
      `exported at ${JSON.stringify(options)}: ${small.count} frames of ${smallMp4.size.join("×")}, sound ${smallMp4.sound}`,
    );
    if (
      small.count !== 30 ||
      small.done !== 30 ||
      smallMp4.size.join() !== "160,90" ||
      smallMp4.sound
    ) {
      failed.push(
        `exporting at ${JSON.stringify(options)}: ${JSON.stringify({ ...small, url: undefined, ...smallMp4 })}`,
      );
    }
    const cancelled = await evaluate("cancelled()");
    if (cancelled !== "cancelled") {
      failed.push(`a cancelled export: ${cancelled}`);
    }
    // Subpixel design details must be rasterized at delivery density, not enlarged as a bitmap.
    const detail = `export const composition = { width: 64, height: 32, fps: 2, duration: 1 };
export default function Scene() {
  return <View style={{ width: 64, height: 32, background: "black" }}>
    {Array.from({ length: 128 }, (_, i) => <View style={{ position: "absolute", left: i / 2, top: 0, width: 0.25, height: 32, background: "white" }} />)}
  </View>;
}`;
    await Bun.write(join(out, "detail.motion.tsx"), detail);
    const detailed = await evaluate(
      'exported("/out/detail.motion.tsx", { width: 256, height: 128, videoBitrate: 20000000 })',
    );
    await Bun.write(join(out, "detail.mp4"), await (await fetch(detailed.url)).arrayBuffer());
    await Bun.write(
      join(out, "detail-video.motion.tsx"),
      `export const composition = { width: 256, height: 128, fps: 2, duration: 1 };
export const controls = { resources: { video: resource({ kind: "video", default: "detail.mp4" }) } };
export default function Scene() { return <Video src="resource://video" style={{ width: 256, height: 128 }} />; }`,
    );
    run([
      "motion",
      "render",
      join(out, "detail.motion.tsx"),
      "--output-size",
      "256x128",
      "-o",
      join(out, "detail.png"),
    ]);
    run([
      "motion",
      "render",
      join(out, "detail-video.motion.tsx"),
      "--decoder",
      "ffmpeg",
      "-o",
      join(out, "detail-video.png"),
    ]);
    const detailDiff = await evaluate('similar("/out/detail.png", "/out/detail-video.png")');
    console.log(`resized export detail: mean ${detailDiff.mean.toFixed(2)} apart`);
    if (detailDiff.mean > 4) failed.push(`resized export lost subpixel detail: ${detailDiff.mean}`);
    for (const size of [{ width: 7680, height: 4322 }, { width: 0 }, { width: 127, height: 72 }]) {
      const rejected = await evaluate(
        `exported("/out/detail.motion.tsx", ${JSON.stringify(size)}).then(() => false, () => true)`,
      );
      if (!rejected) failed.push(`export accepted invalid size ${JSON.stringify(size)}`);
    }
    // The frames: the export of the edit shown by a scene, and the edit drawn.
    await Bun.write(join(out, "shown.motion.tsx"), SHOWN);
    run([
      "motion",
      "render",
      join(out, "shown.motion.tsx"),
      "--frame",
      "45",
      "--decoder",
      "ffmpeg",
      "-o",
      join(out, "shown.png"),
    ]);
    run([
      "timeline",
      "render",
      "examples/edit.timeline.json",
      "--frame",
      "45",
      "-o",
      join(out, "edit.png"),
    ]);
    const frame = await evaluate(`similar("/out/shown.png", "/out/edit.png")`);
    console.log(`export frame 45: most ${frame.most}, mean ${frame.mean.toFixed(2)} apart`);
    if (frame.mean > 1.5) {
      failed.push(`the exported edit's frame 45 is ${frame.mean} apart on average`);
    }
    // <valle-player>.
    await Bun.write(join(out, "element.motion.tsx"), ELEMENT);
    run([
      "motion",
      "render",
      join(out, "element.motion.tsx"),
      "--frame",
      "30",
      "-o",
      join(out, "element.png"),
    ]);
    const element = await evaluate(`element("/out/element.motion.tsx", "/out/element.png")`);
    console.log(`<valle-player>: ${JSON.stringify(element)}`);
    if (
      !element.playing ||
      !(element.at > 0) ||
      !element.muted ||
      !element.loop ||
      element.most > STEP ||
      !element.closed
    ) {
      failed.push(`<valle-player>: ${JSON.stringify(element)}`);
    }
    // The sound: the export's, as a timeline's clip, and the mix.
    await Bun.write(join(out, "listen.timeline.json"), JSON.stringify(LISTEN));
    run(["timeline", "render", join(out, "listen.timeline.json"), "-o", join(out, "listen.wav")]);
    const heardBack = aligned(
      await samples(join(out, "listen.wav")),
      await samples(join(out, "mix.wav")),
    );
    console.log(
      `export sound: ${heardBack.lag} samples late, ${heardBack.rms.toFixed(4)} apart (RMS)`,
    );
    if (Math.abs(heardBack.lag) > 0 || heardBack.rms > 0.01) {
      failed.push(`the exported sound is ${heardBack.lag} samples late, ${heardBack.rms} apart`);
    }
  } finally {
    await Promise.all([browser.close(), server.stop(true)]);
  }
  if (failed.length > 0) {
    throw new Error(
      `the browser's frames are not the native ones (<n>.png beside <n>.browser.png or <n>.screen.png in ${out}):\n${failed.join("\n")}`,
    );
  }
  await rm(out, { recursive: true, force: true });
}

await main();
