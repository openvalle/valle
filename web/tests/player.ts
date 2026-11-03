// The browser test layer: the engine's SDK, as `cargo xtask build-web` builds it, plays the
// examples in the pinned Chrome with WebGPU, and the frames its canvas shows are the frames the
// native engine draws, within a step of rounding.
//
//   bun web/tests/player.ts <valle binary>
//
// The native frames are drawn first, which puts the built-in fonts in the cache; the page fetches
// them from there, so the test reads nothing from the network but Chrome the first time.

import { mkdtemp, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { Cdp, cache, install, listening, root } from "./browser.ts";

/** The examples and frames compared, and the props bound once the player is open. */
const CASES: { entry: string; frames: number[]; props?: Record<string, unknown> }[] = [
  { entry: "examples/hello.motion.tsx", frames: [0, 60] },
  { entry: "examples/01-kinetic-title.motion.tsx", frames: [45] },
  { entry: "examples/03-data-dashboard.motion.tsx", frames: [120] },
  { entry: "examples/06-memphis-loop.motion.tsx", frames: [30] },
  { entry: "examples/10-lower-third.motion.tsx", frames: [40] },
  {
    entry: "examples/10-lower-third.motion.tsx",
    frames: [40],
    props: { name: "Ada Lovelace", accent: "#00C2A8" },
  },
  { entry: "examples/edit.timeline.json", frames: [20] },
  { entry: "tests/cases/motion/video.tsx", frames: [1, 3] },
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
    window.player = await createPlayer({ canvas, entry, base: "/", fonts: "/fonts/" });
    if (props) await window.player.setProps(props);
    // The canvas laid out at its size now, not once the worker's first frame reaches the page.
    canvas.style.width = window.player.info.width + "px";
    canvas.style.height = window.player.info.height + "px";
    await window.player.draw(index);
    const ms = Math.round(performance.now() - started);
    const [x, y] = [await window.player.pixels(), await decoded(native)];
    if (x.length !== y.length) return { size: true, ms };
    let [most, differ] = [0, 0];
    for (let i = 0; i < x.length; i += 4) {
      // A 2D canvas keeps colors premultiplied: those of pixels not opaque come back rounded.
      const channels = x[i + 3] === 255 && y[i + 3] === 255 ? [0, 1, 2, 3] : [3];
      let d = 0;
      for (const k of channels) d = Math.max(d, Math.abs(x[i + k] - y[i + k]));
      most = Math.max(most, d);
      if (d > 1) differ++;
    }
    return { most, differ, pixels: x.length / 4, ms, size: [canvas.width, canvas.height] };
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
  /** Where a player's clock is after it plays for a second from the start, and how many frames
   * it drew on the way: as many as the page's animation frames, which a page in the background
   * has fewer of. */
  window.played = async () => {
    const shown = new Set();
    const off = window.player.on("frame", ({ frame }) => shown.add(frame));
    window.player.seekFrame(0);
    window.player.play();
    await new Promise((r) => setTimeout(r, 1000));
    window.player.pause();
    off();
    return { at: window.player.time, drawn: shown.size };
  };
  /** How the sound of \`entry\` differs from the native mix in the WAV at \`mix\`, and where its
   * clock is after it plays for a second. */
  window.heard = async (entry, mix) => {
    await window.player?.close();
    const canvas = document.createElement("canvas");
    document.querySelector("canvas").replaceWith(canvas);
    const player = (window.player = await createPlayer({ canvas, entry, base: "/", fonts: "/fonts/" }));
    const native = new Float32Array(await (await (await fetch(mix)).arrayBuffer()).slice(44));
    const mine = await player.renderSound(0, native.length / 2 / 48000);
    let most = mine.length === native.length ? 0 : Infinity;
    for (let i = 0; i < native.length; i++) most = Math.max(most, Math.abs(mine[i] - native[i]));
    const { at } = await window.played();
    return { sound: player.hasSound, samples: native.length / 2, most, at };
  };
  /** The MP4 \`entry\` exports, as a data address, how long it took and how many frames it
   * said it had done. */
  window.exported = async (entry) => {
    await window.player?.close();
    const canvas = document.createElement("canvas");
    document.querySelector("canvas").replaceWith(canvas);
    const player = (window.player = await createPlayer({ canvas, entry, base: "/", fonts: "/fonts/" }));
    let done = 0;
    const started = performance.now();
    const mp4 = await player.export({ onProgress: (n) => (done = n) });
    const ms = Math.round(performance.now() - started);
    const url = await new Promise((r) => {
      const reader = new FileReader();
      reader.onload = () => r(reader.result);
      reader.readAsDataURL(mp4);
    });
    return { url, ms, done, frames: player.info.frames };
  };
  /** How two frames differ: the most a channel does, and on average. */
  window.similar = async (a, b) => {
    const [x, y] = [await decoded(a), await decoded(b)];
    if (x.length !== y.length) return { most: Infinity, mean: Infinity };
    let [most, sum] = [0, 0];
    for (let i = 0; i < x.length; i += 4) {
      for (let k = 0; k < 3; k++) {
        const d = Math.abs(x[i + k] - y[i + k]);
        most = Math.max(most, d);
        sum += d;
      }
    }
    return { most, mean: sum / (x.length / 4) / 3 };
  };
  /** How the frame on screen differs from \`native\`'s where it is opaque: the canvas is. */
  window.shown = async (screenshot, native) => {
    const [x, y] = [await decoded(screenshot), await decoded(native)];
    if (x.length !== y.length) return Infinity;
    let most = 0;
    for (let i = 0; i < x.length; i += 4) {
      if (y[i + 3] !== 255) continue;
      for (let k = 0; k < 3; k++) most = Math.max(most, Math.abs(x[i + k] - y[i + k]));
    }
    return most;
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
    const kind = entry.endsWith(".json") ? "timeline" : "motion";
    const bound = props ? ["--props", JSON.stringify(props)] : [];
    for (const frame of frames) {
      const png = join(out, `${natives.size}.png`);
      const run = Bun.spawnSync(
        [valle, kind, "render", entry, "--frame", String(frame), ...bound, "-o", png],
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
      const file = /^\/((?:examples|tests\/cases|web\/packages\/engine\/dist)\/[\w./-]+)$/.exec(
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
  const profile = await mkdtemp(join(tmpdir(), "valle-chrome-"));
  const browser = Bun.spawn(
    [
      await install("chrome"),
      "--headless=new",
      "--remote-debugging-port=0",
      `--user-data-dir=${profile}`,
      "--no-first-run",
      // Else Chrome asks the keychain for its password store, and waits.
      "--use-mock-keychain",
      "--no-default-browser-check",
      "--hide-scrollbars",
      "--enable-unsafe-webgpu",
      "--force-color-profile=srgb",
      "--force-device-scale-factor=1",
      "--autoplay-policy=no-user-gesture-required",
      "about:blank",
    ],
    { stdout: "ignore", stderr: "pipe" },
  );
  const failed: string[] = [];
  try {
    const cdp = await Cdp.open(await listening(browser.stderr));
    const { targetId } = await cdp.send("Target.createTarget", { url: "about:blank" });
    const { sessionId } = await cdp.send("Target.attachToTarget", { targetId, flatten: true });
    const page = (method: string, params: object = {}) => cdp.send(method, params, sessionId);
    await page("Page.enable");
    // What the page and its workers say to the console goes to the test's.
    cdp.on("Target.attachedToTarget", ({ sessionId: worker }) => {
      void cdp.send("Runtime.enable", {}, worker);
      void cdp.send("Runtime.runIfWaitingForDebugger", {}, worker);
    });
    cdp.on("Runtime.consoleAPICalled", ({ type, args }) => {
      const text = args.map((a: any) => a.value ?? a.description).join(" ");
      console.log(`console.${type}: ${text}`);
    });
    cdp.on("Runtime.exceptionThrown", ({ exceptionDetails: e }) => {
      console.log(`exception: ${e.exception?.description ?? e.text}`);
    });
    await page("Runtime.enable");
    await page("Target.setAutoAttach", {
      autoAttach: true,
      waitForDebuggerOnStart: true,
      flatten: true,
    });
    // Room for the largest composition at 1 px a pixel.
    await page("Emulation.setDeviceMetricsOverride", {
      width: 1920,
      height: 1920,
      deviceScaleFactor: 1,
      mobile: false,
    });
    const loaded = cdp.event("Page.loadEventFired", sessionId);
    await page("Page.navigate", { url: `http://127.0.0.1:${server.port}/` });
    await loaded;
    const evaluate = async (expression: string) => {
      const { result, exceptionDetails } = await page("Runtime.evaluate", {
        expression,
        awaitPromise: true,
        returnByValue: true,
      });
      if (exceptionDetails) {
        throw new Error(exceptionDetails.exception?.description ?? exceptionDetails.text);
      }
      return result.value;
    };
    for (const { entry, frames, props } of CASES) {
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
            : `most ${d.most}, ${d.differ} of ${d.pixels} pixels by more than 1`;
        console.log(`${name}: ${line} (${d.ms} ms)`);
        if (d.size === true || d.most > STEP || d.differ > d.pixels * SHARE) {
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
        let most = Infinity;
        let screen = "";
        for (let tries = 0; tries < 20 && most > STEP; tries++) {
          await evaluate(`new Promise((r) => setTimeout(() => requestAnimationFrame(r), 100))`);
          const { data } = await page("Page.captureScreenshot", {
            format: "png",
            clip: { x: 0, y: 0, width, height, scale: 1 },
          });
          screen = data;
          most = await evaluate(
            `shown(${JSON.stringify(`data:image/png;base64,${data}`)}, ${JSON.stringify(native)})`,
          );
        }
        if (most > STEP) {
          const png = natives.get(name)!.slice(0, -4);
          await Bun.write(`${png}.screen.png`, Buffer.from(screen, "base64"));
          failed.push(`${name}: on screen, a channel differs by ${most}`);
        }
      }
    }
    // The last player plays on its own clock.
    const played = await evaluate("played()");
    console.log(`played: at ${played.at.toFixed(2)} s after 1 s, ${played.drawn} frames drawn`);
    if (played.at < 0.5 || played.drawn < 2) {
      failed.push(`playing for 1 s reached ${played.at} s and drew ${played.drawn} frames`);
    }
    // A timeline's sound: the native mix, and played on its clock.
    const heard = await evaluate(`heard("sound/sound.timeline.json", "/mix.wav")`);
    console.log(
      `sound: ${heard.samples} samples, most ${heard.most.toExponential(1)} apart; played: at ${heard.at.toFixed(2)} s after 1 s on the sound's clock`,
    );
    if (!heard.sound || heard.most > 1e-6 || heard.at < 0.5) {
      failed.push(`sound: ${JSON.stringify(heard)}`);
    }
    // Exports: the browser's MP4s, which the native engine decodes.
    for (const [entry, file] of [
      ["examples/edit.timeline.json", "edit.mp4"],
      ["sound/sound.timeline.json", "sound.mp4"],
    ]) {
      const e = await evaluate(`exported(${JSON.stringify(entry)})`);
      await Bun.write(join(out, file), Buffer.from(e.url.slice(e.url.indexOf(",") + 1), "base64"));
      console.log(`exported ${entry}: ${e.frames} frames in ${e.ms} ms`);
      if (e.done !== e.frames) {
        failed.push(`exporting ${entry} said ${e.done} of ${e.frames} frames were done`);
      }
    }
    // The frames: the export of the edit shown by a scene, and the edit drawn.
    await Bun.write(join(out, "shown.motion.tsx"), SHOWN);
    run([
      "motion",
      "render",
      join(out, "shown.motion.tsx"),
      "--frame",
      "45",
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
    await cdp.send("Browser.close").catch(() => {});
  } finally {
    browser.kill();
    await browser.exited;
    server.stop(true);
    await rm(profile, { recursive: true, force: true });
  }
  if (failed.length > 0) {
    throw new Error(
      `the browser's frames are not the native ones (<n>.png beside <n>.browser.png or <n>.screen.png in ${out}):\n${failed.join("\n")}`,
    );
  }
  await rm(out, { recursive: true, force: true });
}

await main();
