/** Serve a prepared B01 scene and the built Web runtime for a real browser GPU benchmark.
 * Run after `bun run build:runtime`: `bun web/scripts/motion/motion_browser_server.ts`.
 */
import { mkdtemp, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve, sep } from "node:path";

const root = resolve(import.meta.dir, "../../..");
const dist = join(root, "web/dist");
const temporary = await mkdtemp(join(tmpdir(), "valle-browser-b01-"));
const fixture = await readFile(join(root, "crates/valle-compiler/tests/fixtures/motion/composition/grid-instances.motion.tsx"), "utf8");
const rows = Number(Bun.env.VALLE_BENCH_ROWS ?? 10000);
const columns = Math.sqrt(rows);
if (!Number.isInteger(columns) || columns < 1 || columns > 316) throw new Error("VALLE_BENCH_ROWS must be a square from 1 to 99856");
const source = join(temporary, "grid-instances.motion.tsx");
await writeFile(source, fixture.replace("const COLS = 8;", `const COLS = ${columns};`));
const cli = Bun.env.VALLE_BENCH_CLI ?? join(root, "target/debug/valle");
const studio = Bun.spawn([cli, "--json", "motion", "studio", source, "--port", "0", "--web-assets-dir", dist], {
  cwd: temporary, stdout: "pipe", stderr: "inherit",
});
const reader = studio.stdout.getReader();
let readyLine = "";
while (!readyLine.includes("\n")) {
  const chunk = await reader.read();
  if (chunk.done) throw new Error("motion studio exited before ready");
  readyLine += new TextDecoder().decode(chunk.value);
}
reader.releaseLock();
const ready = JSON.parse(readyLine.split("\n")[0]) as { url: string };
const upstream = new URL(ready.url).origin;
const resultPath = Bun.env.VALLE_BENCH_RESULT ?? join(temporary, "result.json");
const html = `<!doctype html><html><head><meta charset="utf-8"><title>Valle B01 browser GPU benchmark</title>
<style>body{font:14px system-ui;background:#111;color:#eee;margin:20px}canvas{display:block;width:640px;height:360px}pre{white-space:pre-wrap}</style>
</head><body><h1>B01 browser GPU benchmark (${rows} instances)</h1><p id="status">Loading…</p>
<canvas id="preview" width="640" height="360"></canvas><pre id="report"></pre>
<script type="module" src="/bench/client.js"></script></body></html>`;
const clientPath = join(import.meta.dir, "motion_browser_client.ts");
const client = new Bun.Transpiler({ loader: "ts", target: "browser" })
  .transformSync(await Bun.file(clientPath).text());
const server = Bun.serve({
  hostname: "127.0.0.1", port: 0,
  async fetch(request) {
    const url = new URL(request.url);
    if (url.pathname === "/bench") return new Response(html, { headers: { "Content-Type": "text/html; charset=utf-8" } });
    if (url.pathname === "/bench/client.js") return new Response(client, { headers: { "Content-Type": "text/javascript" } });
    if (url.pathname === "/bench/result") {
      if (request.method === "POST") {
        await writeFile(resultPath, await request.text());
        return new Response("ok");
      }
      return new Response(Bun.file(resultPath));
    }
    if (url.pathname === "/config.json") {
      const response = await fetch(`${upstream}/config.json`);
      const config = await response.json();
      config.instanceRows = rows;
      return Response.json(config);
    }
    const path = resolve(dist, `.${url.pathname}`);
    if (!path.startsWith(`${dist}${sep}`)) return new Response("Not found", { status: 404 });
    const file = Bun.file(path);
    if (!await file.exists()) return new Response("Not found", { status: 404 });
    return new Response(file);
  },
});
console.log(JSON.stringify({ url: `${server.url}bench`, resultPath, upstream, rows }));
process.on("SIGINT", () => { studio.kill(); server.stop(); process.exit(130); });
process.on("SIGTERM", () => { studio.kill(); server.stop(); process.exit(143); });
