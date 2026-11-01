// Draws Chrome baselines for the case runner: each job's HTML in the Chrome for Testing that
// tests/baselines/chrome/version pins, with the border box of every element that has a node
// index and the lines of every text node, written to `<out>.json`, and the frame without its text
// on a transparent canvas, `<out>.png`.
//
//   bun web/tests/chrome.ts <jobs.json>
//
// Chrome is downloaded once into ~/.valle/cache/chrome (or $VALLE_CACHE_DIR/chrome). The pages and
// the fonts come from a local server, so the HTML names no path on this machine.

import { mkdir, mkdtemp, rename, rm } from "node:fs/promises";
import { homedir, tmpdir } from "node:os";
import { dirname, join } from "node:path";

type Job = { html: string; size: [number, number]; out: string };
type Measured = { boxes: number[][]; lines: Record<string, string[]> };

const root = join(import.meta.dir, "../..");
const version = (await Bun.file(join(root, "tests/baselines/chrome/version")).text()).trim();
const jobs: Job[] = await Bun.file(process.argv[2]).json();

const platforms: Record<string, string> = {
  "darwin-arm64": "mac-arm64",
  "darwin-x64": "mac-x64",
  "linux-arm64": "linux-arm64",
  "linux-x64": "linux64",
  "win32-x64": "win64",
};
const platform = platforms[`${process.platform}-${process.arch}`];
if (!platform) {
  throw new Error(`Chrome for Testing has no build for ${process.platform} ${process.arch}`);
}

/** Draws every job in one Chrome. */
async function draw() {
  const chrome = await install();
  const server = Bun.serve({
    hostname: "127.0.0.1",
    port: 0,
    fetch(request) {
      const path = new URL(request.url).pathname;
      const job = /^\/job\/(\d+)$/.exec(path);
      if (job) {
        return new Response(jobs[Number(job[1])].html, {
          headers: { "content-type": "text/html; charset=utf-8" },
        });
      }
      if (/^\/fonts\/[\w.-]+$/.test(path)) {
        return new Response(Bun.file(join(root, path)));
      }
      return new Response("not found", { status: 404 });
    },
  });
  const profile = await mkdtemp(join(tmpdir(), "valle-chrome-"));
  const browser = Bun.spawn(
    [
      chrome,
      "--remote-debugging-port=0",
      `--user-data-dir=${profile}`,
      "--no-first-run",
      "--no-default-browser-check",
      "--hide-scrollbars",
      "--disable-gpu",
      "--force-color-profile=srgb",
      "--force-device-scale-factor=1",
      "--font-render-hinting=none",
      "about:blank",
    ],
    { stdout: "ignore", stderr: "pipe" },
  );

  try {
    const cdp = await Cdp.open(await listening(browser.stderr));
    const { targetId } = await cdp.send("Target.createTarget", { url: "about:blank" });
    const { sessionId } = await cdp.send("Target.attachToTarget", { targetId, flatten: true });
    const page = (method: string, params: object = {}) => cdp.send(method, params, sessionId);
    await page("Page.enable");
    await page("Emulation.setDefaultBackgroundColorOverride", {
      color: { r: 0, g: 0, b: 0, a: 0 },
    });
    for (const [i, job] of jobs.entries()) {
      const [width, height] = job.size;
      await page("Emulation.setDeviceMetricsOverride", {
        width,
        height,
        deviceScaleFactor: 1,
        mobile: false,
      });
      const loaded = cdp.event("Page.loadEventFired", sessionId);
      await page("Page.navigate", { url: `http://127.0.0.1:${server.port}/job/${i}` });
      await loaded;
      const { result, exceptionDetails } = await page("Runtime.evaluate", {
        expression: `(${measure})()`,
        awaitPromise: true,
        returnByValue: true,
      });
      if (exceptionDetails) {
        throw new Error(`${job.out}: ${exceptionDetails.exception?.description ?? "failed"}`);
      }
      const measured: Measured = result.value;
      await mkdir(dirname(job.out), { recursive: true });
      // A box a line, so a change shows as the lines it moves.
      const boxes = measured.boxes.map((b) => `  ${JSON.stringify(b)}`).join(",\n");
      const baseline = [
        "{",
        ` "chrome": ${JSON.stringify(version)},`,
        ` "html": ${JSON.stringify(job.html)},`,
        ` "boxes": [\n${boxes}\n ],`,
        ` "lines": ${JSON.stringify(measured.lines)}`,
        "}",
      ];
      await Bun.write(`${job.out}.json`, `${baseline.join("\n")}\n`);
      // Glyphs are not compared: Chrome's text rendering is not the engine's.
      await page("Runtime.evaluate", {
        expression: `document.head.insertAdjacentHTML("beforeend", "<style>* { -webkit-text-fill-color: transparent !important; }</style>")`,
      });
      const { data } = await page("Page.captureScreenshot", {
        format: "png",
        clip: { x: 0, y: 0, width, height, scale: 1 },
      });
      await Bun.write(`${job.out}.png`, Buffer.from(data, "base64"));
    }
    await cdp.send("Browser.close").catch(() => {});
  } finally {
    browser.kill();
    await browser.exited;
    server.stop(true);
    await rm(profile, { recursive: true, force: true });
  }
}

/** The pinned Chrome's executable, downloaded into the cache the first time. */
async function install(): Promise<string> {
  const cache = process.env.VALLE_CACHE_DIR ?? join(homedir(), ".valle/cache");
  const dir = join(cache, "chrome", version);
  const name = platform === "win64" ? "chrome-headless-shell.exe" : "chrome-headless-shell";
  const exe = join(dir, `chrome-headless-shell-${platform}`, name);
  if (await Bun.file(exe).exists()) {
    return exe;
  }
  const url = `https://storage.googleapis.com/chrome-for-testing-public/${version}/${platform}/chrome-headless-shell-${platform}.zip`;
  console.error(`downloading Chrome for Testing ${version} from ${url}`);
  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(`${url}: ${response.status} ${response.statusText}`);
  }
  // Unpacked beside the cache and moved in whole, so a broken download leaves nothing behind.
  const partial = `${dir}.partial`;
  await rm(partial, { recursive: true, force: true });
  await mkdir(partial, { recursive: true });
  const zip = join(partial, "chrome.zip");
  await Bun.write(zip, response);
  const unzip =
    platform === "win64" ? ["tar", "-xf", zip, "-C", partial] : ["unzip", "-q", zip, "-d", partial];
  if ((await Bun.spawn(unzip, { stdout: "inherit", stderr: "inherit" }).exited) !== 0) {
    throw new Error(`${unzip.join(" ")} failed`);
  }
  await rm(zip);
  await rename(partial, dir);
  return exe;
}

/** The DevTools address Chrome prints once it listens. Its later output is read and dropped. */
async function listening(stderr: ReadableStream<Uint8Array>): Promise<string> {
  const reader = stderr.getReader();
  const decoder = new TextDecoder();
  let text = "";
  for (;;) {
    const { value, done } = await reader.read();
    if (done) {
      throw new Error(`Chrome stopped before it listened:\n${text}`);
    }
    text += decoder.decode(value, { stream: true });
    const found = /DevTools listening on (ws:\/\/\S+)/.exec(text);
    if (found) {
      void (async () => {
        while (!(await reader.read()).done);
      })();
      return found[1];
    }
  }
}

/** Runs in the page: the boxes and lines once the fonts are in. */
function measure(): Promise<Measured> {
  const lines = (element: Element): string[] => {
    const out: string[] = [];
    let line = "";
    let top: number | undefined;
    const range = document.createRange();
    const walk = document.createTreeWalker(element, NodeFilter.SHOW_TEXT);
    for (let node = walk.nextNode() as Text | null; node; node = walk.nextNode() as Text | null) {
      for (let i = 0; i < node.data.length;) {
        const size = node.data.codePointAt(i)! > 0xffff ? 2 : 1;
        range.setStart(node, i);
        range.setEnd(node, i + size);
        const rects = range.getClientRects();
        // White space that collapsed away has no box.
        if (rects.length > 0) {
          if (top !== undefined && Math.abs(rects[0].top - top) > 1) {
            out.push(line);
            line = "";
          }
          top = rects[0].top;
          line += node.data.slice(i, i + size);
        }
        i += size;
      }
    }
    if (top !== undefined) {
      out.push(line);
    }
    return out.map((l) => l.replace(/[ \t\n\r\f]+/g, " ").trim());
  };
  return document.fonts.load('16px "Valle Sans"').then(() => {
    for (const face of document.fonts) {
      if (face.status !== "loaded") {
        throw new Error(`font ${face.family} is ${face.status}`);
      }
    }
    const boxes: number[][] = [];
    const text: Record<string, string[]> = {};
    for (const element of document.querySelectorAll<HTMLElement>("[data-n]")) {
      const n = Number(element.dataset.n);
      const r = element.getBoundingClientRect();
      boxes[n] = [r.x, r.y, r.width, r.height];
      if (element.hasAttribute("data-text")) {
        text[n] = lines(element);
      }
    }
    return { boxes, lines: text };
  });
}

/** A Chrome DevTools Protocol connection: commands with their replies, and events awaited. */
class Cdp {
  #ws: WebSocket;
  #next = 0;
  #replies = new Map<number, { resolve: (v: any) => void; reject: (e: Error) => void }>();
  #waiting: { method: string; sessionId?: string; resolve: (v: any) => void }[] = [];

  static async open(url: string): Promise<Cdp> {
    const ws = new WebSocket(url);
    await new Promise((resolve, reject) => {
      ws.onopen = resolve;
      ws.onerror = () => reject(new Error(`cannot connect to ${url}`));
    });
    return new Cdp(ws);
  }

  constructor(ws: WebSocket) {
    this.#ws = ws;
    ws.onmessage = (event) => {
      const message = JSON.parse(String(event.data));
      if (message.id !== undefined) {
        const reply = this.#replies.get(message.id);
        this.#replies.delete(message.id);
        if (message.error) {
          reply?.reject(new Error(`${message.error.message} (${message.error.code})`));
        } else {
          reply?.resolve(message.result);
        }
        return;
      }
      const i = this.#waiting.findIndex(
        (w) => w.method === message.method && w.sessionId === message.sessionId,
      );
      if (i >= 0) {
        this.#waiting.splice(i, 1)[0].resolve(message.params);
      }
    };
  }

  send(method: string, params: object = {}, sessionId?: string): Promise<any> {
    const id = ++this.#next;
    this.#ws.send(JSON.stringify({ id, method, params, sessionId }));
    return timed(
      method,
      new Promise((resolve, reject) => this.#replies.set(id, { resolve, reject })),
    );
  }

  /** The next `method` event of the session. */
  event(method: string, sessionId?: string): Promise<any> {
    return timed(
      method,
      new Promise((resolve) => this.#waiting.push({ method, sessionId, resolve })),
    );
  }
}

/** `promise`, or an error after 30 s: a page that never loads stops the run. */
function timed<T>(what: string, promise: Promise<T>): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const timeout = new Promise<never>((_, reject) => {
    timer = setTimeout(() => reject(new Error(`${what}: no answer from Chrome in 30 s`)), 30_000);
  });
  return Promise.race([promise, timeout]).finally(() => clearTimeout(timer));
}

await draw();
