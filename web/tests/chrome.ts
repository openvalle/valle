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
// fonts and the files the cases read come from a local server. A baseline keeps the job's HTML,
// which names the built-in fonts by their files' names; Chrome loads its page, which the case
// runner gives the fonts' paths in the cache.

import { mkdir, mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";

import { Cdp, cache, install, listening, root, version } from "./browser.ts";

type Job = { html: string; page: string; size: [number, number]; out: string };
/** The easings `ease.rs` is tested against. */
const EASINGS = [
  "linear",
  "ease",
  "ease-in",
  "ease-out",
  "ease-in-out",
  "step-start",
  "step-end",
  "cubic-bezier(0.1, 0.7, 1, 0.1)",
  "cubic-bezier(0.68, -0.6, 0.32, 1.6)",
  "cubic-bezier(0, 1, 0, 1)",
  "cubic-bezier(1, 0, 1, 0)",
  "cubic-bezier(0, 0, 1, 1)",
  "steps(3)",
  "steps(4, start)",
  "steps(4, jump-start)",
  "steps(3, jump-none)",
  "steps(3, jump-both)",
  "steps(2, end)",
  "linear(0, 1)",
  "linear(0, 0.25 25% 75%, 1)",
  "linear(0, 0.7, 0.3, 1)",
  "linear(0, 1.2 60%, 0.9 80%, 1)",
  "linear(0 20%, 1 80%)",
  "linear(0.5, 0.5 20%, 0 40% 40%, 1)",
  "linear(0, 0.5 60%, 0.2 40%, 1)",
];

/** The paths `path/edit.rs` measures as Chrome does. */
const PATHS = [
  "M0 0 L100 0",
  "M10 10 L90 10 L90 60 Z",
  "M0 0 Q50 100 100 0",
  "M0 0 C0 100 100 -50 100 50",
  "M0 0 C30 0 70 0 100 0",
  "M10 50 A40 40 0 1 1 90 50",
  "M0 50 A50 25 30 0 1 100 50",
  "M0 0 L50 0 M100 0 L150 0",
  "M20 20 h60 v40 h-60 z m10 10 h20",
  "M0 0 S50 50 100 0 T200 0",
  "M5 5",
];

/** The formats and numbers `format.rs` is tested against, as text so -0 and NaN survive JSON. */
const FORMATS = [
  "number||true",
  "number||false",
  "number|0|true",
  "number|2|true",
  "number|5|false",
  "percent||true",
  "percent|1|true",
  "pad|2|false",
  "pad|4|false",
];
const VALUES = [
  "0",
  "-0",
  "0.5",
  "1.5",
  "2.5",
  "-2.5",
  "1.005",
  "0.125",
  "-0.0001",
  "0.0145",
  "7",
  "999.5",
  "999.9995",
  "1234567.891",
  "-1234.5678",
  "0.000001234",
  "5e-324",
  "1e21",
  "123456789012345680000",
  "NaN",
  "Infinity",
  "-Infinity",
];

type Measured = {
  boxes: number[][];
  lines: Record<string, string[]>;
  fonts: Record<string, Record<string, string>>;
  refused: string[];
};

const jobs: Job[] = await Bun.file(process.argv[2]).json();

/** Draws every job in one Chrome. */
async function draw() {
  const pages = await Promise.all(jobs.map((job) => page(job.page)));
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
      // A file in the cache, by its path.
      const cached = /^\/file(\/.+)$/.exec(path);
      if (cached && decodeURIComponent(cached[1]).startsWith(join(cache, "files") + "/")) {
        return new Response(Bun.file(decodeURIComponent(cached[1])));
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
    const { result: curves } = await page("Runtime.evaluate", {
      expression: `(${eased})(${JSON.stringify(EASINGS)})`,
      returnByValue: true,
    });
    const easings = Object.entries(curves.value as Record<string, number[]>)
      .map(([e, v]) => `  ${JSON.stringify(e)}: ${JSON.stringify(v)}`)
      .join(",\n");
    const { result: texts } = await page("Runtime.evaluate", {
      expression: `(${written})(${JSON.stringify(FORMATS)}, ${JSON.stringify(VALUES)})`,
      returnByValue: true,
    });
    const { result: lengths } = await page("Runtime.evaluate", {
      expression: `(${measured})(${JSON.stringify(PATHS)})`,
      returnByValue: true,
    });
    const paths = Object.entries(lengths.value as Record<string, unknown>)
      .map(([k, v]) => `  ${JSON.stringify(k)}: ${JSON.stringify(v)}`)
      .join(",\n");
    const formats = Object.entries(texts.value as Record<string, string>)
      .map(([k, v]) => `  ${JSON.stringify(k)}: ${JSON.stringify(v)}`)
      .join(",\n");
    await Bun.write(
      join(root, "tests/baselines/chrome/properties.json"),
      `{\n "chrome": ${JSON.stringify(version)},\n "properties": [\n${properties}\n ],\n "easings": {\n${easings}\n },\n "formats": {\n${formats}\n },\n "paths": {\n${paths}\n }\n}\n`,
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
  return Promise.all([...[...document.fonts].map((f) => f.load()), ...images]).then(() => {
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

/** Runs in the page: each easing's progress at twenty-firsts of a second of an animation. */
function eased(easings: string[]): Record<string, number[]> {
  const out: Record<string, number[]> = {};
  for (const easing of easings) {
    const options = { duration: 1000, easing, fill: "both" as const };
    const animation = document.documentElement.animate({ opacity: [0, 1] }, options);
    animation.pause();
    out[easing] = Array.from({ length: 21 }, (_, i) => {
      animation.currentTime = i * 50;
      return animation.effect!.getComputedTiming().progress!;
    });
    animation.cancel();
  }
  return out;
}

/** Runs in the page: each path's total length and its points at tenths of it. */
function measured(paths: string[]): Record<string, { length: number; points: number[][] }> {
  const out: Record<string, { length: number; points: number[][] }> = {};
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
  svg.append(path);
  document.body.append(svg);
  for (const d of paths) {
    path.setAttribute("d", d);
    const length = path.getTotalLength();
    const points = Array.from({ length: 11 }, (_, i) => {
      const p = path.getPointAtLength((length * i) / 10);
      return [p.x, p.y];
    });
    out[d] = { length, points };
  }
  svg.remove();
  return out;
}

/**
 * Runs in the page: each number as `Intl.NumberFormat("en-US")` writes it for each format, keyed
 * `format|value`, as `format.rs` reads the formats.
 */
function written(formats: string[], values: string[]): Record<string, string> {
  const out: Record<string, string> = {};
  for (const format of formats) {
    const [kind, n, grouping] = format.split("|");
    const options: Intl.NumberFormatOptions =
      kind === "pad"
        ? { minimumIntegerDigits: Number(n), useGrouping: false, maximumFractionDigits: 0 }
        : {
            style: kind === "percent" ? "percent" : "decimal",
            useGrouping: grouping === "true",
            ...(n === ""
              ? {}
              : { minimumFractionDigits: Number(n), maximumFractionDigits: Number(n) }),
          };
    const intl = new Intl.NumberFormat("en-US", options);
    for (const value of values) out[`${format}|${value}`] = intl.format(Number(value));
  }
  return out;
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

await draw();
