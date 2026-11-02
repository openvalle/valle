// Draws Chrome baselines for the case runner: each job's HTML in the Chrome for Testing that
// tests/baselines/chrome/version pins, with the border box of every element that has a node
// index and the lines of every text node, written to `<out>.json`, and the frame without its text
// on a transparent canvas, `<out>.png`. It also lists the CSS properties that Chrome knows,
// tests/baselines/chrome/properties.json, which the cases the engine must refuse are made from.
// A page with classes gets the stylesheet the Tailwind of package.json builds of them.
//
//   bun web/tests/chrome.ts <jobs.json>
//
// Chrome is downloaded once into ~/.valle/cache/chrome (or $VALLE_CACHE_DIR/chrome). The pages, the
// fonts and the files the cases read come from a local server, so the HTML names no path on this
// machine.

import { mkdir, mkdtemp, rename, rm } from "node:fs/promises";
import { homedir, tmpdir } from "node:os";
import { dirname, join } from "node:path";

type Job = { html: string; size: [number, number]; out: string };
type Measured = {
  boxes: number[][];
  lines: Record<string, string[]>;
  fonts: Record<string, Record<string, string>>;
  refused: string[];
};

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
  const pages = await Promise.all(jobs.map((job) => page(job.html)));
  const chrome = await install();
  const server = Bun.serve({
    hostname: "127.0.0.1",
    port: 0,
    fetch(request) {
      const path = new URL(request.url).pathname;
      const job = /^\/job\/(\d+)$/.exec(path);
      if (job) {
        return new Response(pages[Number(job[1])], {
          headers: { "content-type": "text/html; charset=utf-8" },
        });
      }
      if (/^\/fonts\/[\w.-]+$/.test(path)) {
        return new Response(Bun.file(join(root, path)));
      }
      const file = /^\/cases\/((?:[\w-][\w.-]*\/)*[\w-][\w.-]*)$/.exec(path);
      if (file) {
        return new Response(Bun.file(join(root, "tests/cases", file[1])));
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
    const refused: string[] = [];
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
      refused.push(...measured.refused.map((r) => `${job.out}: ${r}`));
      await mkdir(dirname(job.out), { recursive: true });
      // A box a line, so a change shows as the lines it moves.
      const boxes = measured.boxes.map((b) => `  ${JSON.stringify(b)}`).join(",\n");
      const fonts = Object.entries(measured.fonts)
        .map(([n, f]) => `  ${JSON.stringify(n)}: ${JSON.stringify(f)}`)
        .join(",\n");
      const baseline = [
        "{",
        ` "chrome": ${JSON.stringify(version)},`,
        ` "html": ${JSON.stringify(job.html)},`,
        ` "boxes": [\n${boxes}\n ],`,
        ` "lines": ${JSON.stringify(measured.lines)},`,
        ` "fonts": {\n${fonts}\n }`,
        "}",
      ];
      await Bun.write(`${job.out}.json`, `${baseline.join("\n")}\n`);
      // Glyphs, their strokes and their shadows are not compared: Chrome's text rendering is not
      // the engine's.
      await page("Runtime.evaluate", {
        expression: `document.head.insertAdjacentHTML("beforeend", "<style>* { -webkit-text-fill-color: transparent !important; -webkit-text-stroke-width: 0 !important; text-shadow: none !important; }</style>")`,
      });
      const { data } = await page("Page.captureScreenshot", {
        format: "png",
        clip: { x: 0, y: 0, width, height, scale: 1 },
      });
      await Bun.write(`${job.out}.png`, Buffer.from(data, "base64"));
    }
    if (refused.length > 0) {
      throw new Error(`Chrome refuses what the engine takes:\n${refused.join("\n")}`);
    }
    const { result } = await page("Runtime.evaluate", {
      expression: `(${known})()`,
      returnByValue: true,
    });
    const properties = (result.value as string[]).map((p) => `  ${JSON.stringify(p)}`).join(",\n");
    await Bun.write(
      join(root, "tests/baselines/chrome/properties.json"),
      `{\n "chrome": ${JSON.stringify(version)},\n "properties": [\n${properties}\n ]\n}\n`,
    );
    await cdp.send("Browser.close").catch(() => {});
  } finally {
    browser.kill();
    await browser.exited;
    server.stop(true);
    await rm(profile, { recursive: true, force: true });
  }
}

/** A job's HTML, with the stylesheet Tailwind builds of its classes where it asks for one. */
async function page(html: string): Promise<string> {
  const marker = /<style data-tailwind="([^"]*)"><\/style>/.exec(html);
  if (!marker) {
    return html;
  }
  const { stylesheet, version: tailwind } = await import("./tailwind.ts");
  if (marker[1] !== tailwind) {
    throw new Error(`a page asks for Tailwind ${marker[1]}, and ${tailwind} is installed`);
  }
  const classes = new Set<string>();
  for (const [, list] of html.matchAll(/ class="([^"]*)"/g)) {
    const text = list
      .replaceAll("&quot;", '"')
      .replaceAll("&lt;", "<")
      .replaceAll("&gt;", ">")
      .replaceAll("&amp;", "&");
    for (const c of text.split(/\s+/)) {
      if (c) classes.add(c);
    }
  }
  const css = await stylesheet([...classes]);
  return html.replace(marker[0], () => `<style data-tailwind="${tailwind}">\n${css}</style>`);
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

/** Runs in the page: the boxes and lines once the fonts and the images are in. */
function measure(): Promise<Measured> {
  // The declarations of a style attribute: semicolons outside quotes and brackets end them.
  const declarations = (style: string): string[] => {
    const out: string[] = [];
    let [start, depth, quote] = [0, 0, ""];
    for (let i = 0; i < style.length; i++) {
      const c = style[i];
      if (quote) {
        quote = c === quote ? "" : quote;
      } else if (c === '"' || c === "'") {
        quote = c;
      } else if (c === "(") {
        depth++;
      } else if (c === ")") {
        depth--;
      } else if (c === ";" && depth === 0) {
        out.push(style.slice(start, i).trim());
        start = i + 1;
      }
    }
    out.push(style.slice(start).trim());
    return out.filter((d) => d);
  };
  const lines = (element: Element): string[] => {
    const out: string[] = [];
    let line = "";
    let last: DOMRect | undefined;
    const range = document.createRange();
    const walk = document.createTreeWalker(element, NodeFilter.SHOW_TEXT);
    for (let node = walk.nextNode() as Text | null; node; node = walk.nextNode() as Text | null) {
      for (let i = 0; i < node.data.length;) {
        const size = node.data.codePointAt(i)! > 0xffff ? 2 : 1;
        range.setStart(node, i);
        range.setEnd(node, i + size);
        const rects = range.getClientRects();
        // White space that collapsed away has no box.
        // A line goes right, and its characters overlap, however big: the next one starts back
        // at the left, or below, which for characters of no size is lower.
        if (rects.length > 0) {
          const r = rects[0];
          if (
            last !== undefined &&
            (r.left < last.left - 1 || r.top >= Math.max(last.bottom - 1, last.top + 1))
          ) {
            out.push(line);
            line = "";
          }
          last = r;
          line += node.data.slice(i, i + size);
        }
        i += size;
      }
    }
    if (last !== undefined) {
      out.push(line);
    }
    return out.map((l) => l.replace(/[ \t\n\r\f]+/g, " ").trim());
  };
  // The files of img elements, and of images in styles, which Chrome paints once they are in;
  // url(#id) names an element of the page, not a file.
  const files = new Set<string>();
  for (const e of document.querySelectorAll<HTMLElement>("[data-n]")) {
    const s = getComputedStyle(e);
    for (const m of `${s.backgroundImage} ${s.maskImage}`.matchAll(/url\("([^"#]*)"\)/g)) {
      files.add(m[1]);
    }
  }
  const images = [
    ...[...document.images].map((i) => i.decode()),
    ...[...files].map((src) => {
      const i = new Image();
      i.src = src;
      return i.decode();
    }),
  ];
  return Promise.all([document.fonts.load('16px "Valle Sans"'), ...images]).then(() => {
    for (const face of document.fonts) {
      if (face.status !== "loaded") {
        throw new Error(`font ${face.family} is ${face.status}`);
      }
    }
    const boxes: number[][] = [];
    const text: Record<string, string[]> = {};
    const elements = [...document.querySelectorAll<HTMLElement>("[data-n]")];
    // A declaration Chrome drops would pass as the value the box has without it.
    const refused = elements.flatMap((e) =>
      declarations(e.getAttribute("style") ?? "").filter((d) => {
        const colon = d.indexOf(":");
        return !CSS.supports(d.slice(0, colon).trim(), d.slice(colon + 1).trim());
      }),
    );
    for (const element of elements) {
      const r = element.getBoundingClientRect();
      boxes[Number(element.dataset.n)] = [r.x, r.y, r.width, r.height];
    }
    // What the glyphs show, which are hidden in the pixels.
    const fonts: Measured["fonts"] = {};
    for (const element of elements.filter((e) => e.matches("[data-text], span"))) {
      const s = getComputedStyle(element);
      fonts[element.dataset.n!] = {
        color: s.color,
        fill: s.webkitTextFillColor,
        stroke: s.webkitTextStrokeWidth,
        stroke_color: s.webkitTextStrokeColor,
        shadow: s.textShadow,
        size: s.fontSize,
        weight: s.fontWeight,
        stretch: s.fontStretch,
        style: s.fontStyle,
        transform: s.textTransform,
      };
    }
    // Lines as laid out, untransformed: a transform or a motion path moves text but breaks no
    // line. `will-change` keeps what a transform contains and stacks, so the layout stays. Their
    // styles go back as written: Chrome's `cssText` can parse back to another style, as a
    // background shorthand with one box for both its origin and its clip.
    const moved = elements.filter((e) => {
      const s = getComputedStyle(e);
      return [s.transform, s.translate, s.rotate, s.scale, s.offsetPath].some((v) => v !== "none");
    });
    const written = moved.map((e) => e.getAttribute("style") ?? "");
    for (const e of moved) {
      e.style.transform = e.style.translate = e.style.rotate = e.style.scale = "none";
      e.style.offsetPath = "none";
      e.style.willChange = "transform";
    }
    for (const element of elements) {
      if (element.hasAttribute("data-text")) {
        text[Number(element.dataset.n)] = lines(element);
      }
    }
    moved.forEach((e, i) => e.setAttribute("style", written[i]));
    return { boxes, lines: text, fonts, refused };
  });
}

/** Runs in the page: the CSS properties Chrome knows, by their CSS names, sorted. */
function known(): string[] {
  const names = new Set<string>();
  for (const key in document.documentElement.style) {
    // `webkitTextStroke` is `-webkit-text-stroke`. Methods, and descriptors such as `src`, are not
    // properties.
    let name = key.replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`);
    if (name.startsWith("webkit-")) {
      name = `-${name}`;
    }
    if (CSS.supports(name, "initial")) {
      names.add(name);
    }
  }
  return [...names].sort();
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
