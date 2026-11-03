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
  /** How the frame on screen differs from \`native\`'s where it is opaque: the canvas is. */
  window.shown = async (screenshot, native) => {
    const [x, y] = [await decoded(screenshot), await decoded(native)];
    let most = 0;
    for (let i = 0; i < x.length; i += 4) {
      if (y[i + 3] !== 255) continue;
      for (let k = 0; k < 3; k++) most = Math.max(most, Math.abs(x[i + k] - y[i + k]));
    }
    return most;
  };
</script>
`;

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
      const frame = /^\/native\/(.+)$/.exec(path);
      if (frame && natives.has(frame[1])) {
        return new Response(Bun.file(natives.get(frame[1])!));
      }
      const file = /^\/((?:examples|web\/packages\/engine\/dist)\/[\w./-]+)$/.exec(path);
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
        await page("Emulation.setDeviceMetricsOverride", {
          width,
          height,
          deviceScaleFactor: 1,
          mobile: false,
        });
        await evaluate(`new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)))`);
        const { data } = await page("Page.captureScreenshot", {
          format: "png",
          clip: { x: 0, y: 0, width, height, scale: 1 },
        });
        const most = await evaluate(
          `shown(${JSON.stringify(`data:image/png;base64,${data}`)}, ${JSON.stringify(native)})`,
        );
        if (most > STEP) {
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
    await cdp.send("Browser.close").catch(() => {});
  } finally {
    browser.kill();
    await browser.exited;
    server.stop(true);
    await rm(profile, { recursive: true, force: true });
  }
  if (failed.length > 0) {
    throw new Error(
      `the browser's frames are not the native ones (<n>.png beside <n>.browser.png in ${out}):\n${failed.join("\n")}`,
    );
  }
  await rm(out, { recursive: true, force: true });
}

await main();
