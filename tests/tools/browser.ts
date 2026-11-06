// Capture candidates only. Review their images and data before copying them to baselines/browser.
import { createHash } from "node:crypto";
import { mkdir, readdir, rm } from "node:fs/promises";
import { release } from "node:os";
import { join, relative, resolve, sep } from "node:path";
import { firefox, webkit } from "playwright";
import { launch, root } from "../../web/tests/browser";

const baseline = join(root, "tests/baselines/browser");
const output = join(root, "target/browser-baselines");
const filters = process.argv.slice(2).map((path) => path.replace(/\/+$/, ""));
const cases: string[] = [];
async function discover(dir: string) {
  if (await Bun.file(join(dir, "case.json")).exists()) {
    const config = await Bun.file(join(dir, "case.json")).json();
    const id = relative(join(root, "tests/cases"), dir).replaceAll("\\", "/");
    if (
      config.browser &&
      (!filters.length || filters.some((path) => id === path || id.startsWith(`${path}/`)))
    )
      cases.push(id);
    return;
  }
  for (const entry of await readdir(dir, { withFileTypes: true })) {
    if (entry.isDirectory()) await discover(join(dir, entry.name));
  }
}
await discover(join(root, "tests/cases"));
cases.sort();
if (!cases.length) throw new Error(`No browser cases match ${filters.join(", ") || "tests/cases"}`);
const fonts = Bun.spawn(["cargo", "xtask", "fonts"], {
  cwd: root,
  stdout: "inherit",
  stderr: "inherit",
});
if ((await fonts.exited) !== 0) throw new Error("Preparing the pinned test fonts failed");
await rm(output, { recursive: true, force: true });
await mkdir(output, { recursive: true });
const server = Bun.serve({
  hostname: "127.0.0.1",
  port: 0,
  async fetch(request) {
    const file = resolve(root, `.${decodeURIComponent(new URL(request.url).pathname)}`);
    if (!file.startsWith(join(root, "tests") + sep)) return new Response(null, { status: 403 });
    const data = Bun.file(file);
    return (await data.exists()) ? new Response(data) : new Response(null, { status: 404 });
  },
});
const versions: Record<string, string> = {};
try {
  for (const [name, open] of Object.entries({
    chrome: launch,
    firefox: () => firefox.launch(),
    webkit: () => webkit.launch(),
  })) {
    const browser = await open();
    try {
      versions[name] = browser.version();
      for (const id of cases) {
        const dir = join(output, id);
        await mkdir(dir, { recursive: true });
        const page = await browser.newPage({
          viewport: { width: 800, height: 600 },
          deviceScaleFactor: 1,
          colorScheme: "light",
          locale: "en-US",
        });
        try {
          const sources: Record<string, string> = {};
          const errors: string[] = [];
          page.on("pageerror", (e) => errors.push(e.message));
          page.on("response", (r) => {
            if (!r.ok()) errors.push(`${r.status()} ${r.url()}`);
          });
          await page.route("**/*", async (route) => {
            const url = new URL(route.request().url());
            if (url.origin !== server.url.origin) {
              errors.push(`External resource is not allowed: ${url}`);
              await route.abort();
              return;
            }
            const file = resolve(root, `.${decodeURIComponent(url.pathname)}`);
            if (!file.startsWith(join(root, "tests") + sep))
              throw new Error(`Invalid resource ${file}`);
            const bytes = await Bun.file(file).arrayBuffer();
            sources[relative(root, file)] = createHash("sha256")
              .update(new Uint8Array(bytes))
              .digest("hex");
            await route.continue();
          });
          await page.goto(`${server.url}tests/baselines/browser/${id}/scene.html`, {
            waitUntil: "networkidle",
          });
          const size = await page.locator("#canvas").evaluate((canvas) => {
            const r = canvas.getBoundingClientRect();
            if (r.x !== 0 || r.y !== 0 || !Number.isInteger(r.width) || !Number.isInteger(r.height))
              throw new Error("#canvas must start at (0,0) and have an integer size");
            return [r.width, r.height];
          });
          await page.setViewportSize({ width: size[0], height: size[1] });
          await page.evaluate(async () => {
            await document.fonts.ready;
            await Promise.all([...document.images].map((image) => image.decode()));
            await new Promise<void>((done) => requestAnimationFrame(() => done()));
          });
          const measurements = await page.evaluate(() => {
            const layout = [...document.querySelectorAll<HTMLElement>("[data-key]")].map(
              (element) => {
                const rect = element.getBoundingClientRect();
                const style = getComputedStyle(element);
                const border = { x: rect.x, y: rect.y, width: rect.width, height: rect.height };
                const inset = (
                  box: typeof border,
                  top: number,
                  right: number,
                  bottom: number,
                  left: number,
                ) => ({
                  x: box.x + left,
                  y: box.y + top,
                  width: box.width - left - right,
                  height: box.height - top - bottom,
                });
                const padding = inset(
                  border,
                  parseFloat(style.borderTopWidth),
                  parseFloat(style.borderRightWidth),
                  parseFloat(style.borderBottomWidth),
                  parseFloat(style.borderLeftWidth),
                );
                const content = inset(
                  padding,
                  parseFloat(style.paddingTop),
                  parseFloat(style.paddingRight),
                  parseFloat(style.paddingBottom),
                  parseFloat(style.paddingLeft),
                );
                return { key: element.dataset.key!, border, padding, content };
              },
            );
            const lines = [...document.querySelectorAll<HTMLElement>("[data-text]")].map(
              (element) => {
                // Uniform-font text, including wrapping. Read ranges without modifying the text.
                if (
                  element.childNodes.length !== 1 ||
                  element.firstChild?.nodeType !== Node.TEXT_NODE
                )
                  throw new Error("data-text requires one text node");
                const node = element.firstChild!;
                const string = node.textContent!;
                const text: string[] = [];
                const starts: number[] = [];
                const before: number[][] = [];
                const rect = (node: Node, i: number) => {
                  const range = document.createRange();
                  range.setStart(node, i);
                  range.setEnd(node, i + 1);
                  const r = range.getBoundingClientRect();
                  return [r.x, r.y, r.width, r.height];
                };
                let lastY: number | undefined;
                for (let i = 0; i < string.length; i++) {
                  const r = rect(node, i);
                  before.push(r);
                  if (string[i] === "\n") continue;
                  const y = r[1];
                  if (y !== lastY) {
                    text.push("");
                    starts.push(i);
                    lastY = y;
                  }
                  text[text.length - 1] += string[i];
                }
                const height = parseFloat(getComputedStyle(element).lineHeight);
                if (!Number.isFinite(height))
                  throw new Error("data-text requires explicit line-height");
                // Calibrate the range-top-to-baseline offset in a separate, unwrapped probe.
                // A marker in the measured paragraph can uncollapse a trailing space and
                // change the very line break being measured.
                const style = getComputedStyle(element);
                const probe = document.createElement("span");
                probe.style.cssText =
                  "position:absolute;left:-10000px;top:0;display:block;white-space:pre;margin:0;padding:0;border:0";
                probe.style.font = style.font;
                probe.style.fontFeatureSettings = style.fontFeatureSettings;
                probe.style.fontVariationSettings = style.fontVariationSettings;
                probe.style.lineHeight = style.lineHeight;
                const glyph = document.createTextNode("M");
                const marker = document.createElement("span");
                marker.style.cssText =
                  "display:inline-block;width:0;height:0;padding:0;margin:0;border:0;vertical-align:baseline";
                probe.append(glyph, marker);
                document.body.append(probe);
                const offset = marker.getBoundingClientRect().y - rect(glyph, 0)[1];
                probe.remove();
                const geometry = starts.map((i) => ({
                  x: before[i][0],
                  baseline: before[i][1] + offset,
                  height,
                }));
                const after = Array.from({ length: string.length }, (_, i) => rect(node, i));
                if (JSON.stringify(before) !== JSON.stringify(after))
                  throw new Error("baseline probes changed text layout");
                return { key: element.dataset.key!, text, geometry };
              },
            );
            return { layout, lines };
          });
          if (errors.length) throw new Error(errors.join("\n"));
          await page.screenshot({ path: join(dir, `${name}.png`), animations: "disabled" });
          await Bun.write(
            join(dir, `${name}.json`),
            JSON.stringify({ size, sources, measurements }, null, 2) + "\n",
          );
          console.log(`${name}: ${id}`);
        } finally {
          await page.close();
        }
      }
    } finally {
      await browser.close();
    }
  }
  await Bun.write(
    join(output, "environment.json"),
    JSON.stringify(
      {
        capturedAt: new Date().toISOString(),
        platform: process.platform,
        arch: process.arch,
        os: release(),
        playwright: (await Bun.file(join(root, "node_modules/playwright/package.json")).json())
          .version,
        deviceScaleFactor: 1,
        colorScheme: "light",
        browsers: versions,
      },
      null,
      2,
    ) + "\n",
  );
  console.log(`Candidates: ${output}\nReviewed baselines remain unchanged: ${baseline}`);
} finally {
  await server.stop(true);
}
