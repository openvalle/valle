/** Chrome capacity probe for the B01 geometry, independent of the Valle renderer.
 * Run: bun web/scripts/motion/motion_browser_capacity_chrome.ts
 * Optional: VALLE_BENCH_CAPACITY_RESULT=/absolute/new-result.json
 * DOM rAF intervals and direct WebGL2 gl.finish timings are diagnostic controls,
 * not pixel-equivalent or product playback acceptance results.
 */
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const client = new Bun.Transpiler({ loader: 'ts', target: 'browser' })
  .transformSync(await Bun.file(join(import.meta.dir, 'motion_browser_capacity_client.ts')).text());
let receive: ((value: any) => void) | null = null;
const server = Bun.serve({
  hostname: '127.0.0.1', port: 0,
  async fetch(request) {
    const url = new URL(request.url);
    if (url.pathname === '/client.js') return new Response(client, { headers: { 'Content-Type': 'text/javascript' } });
    if (url.pathname === '/result' && request.method === 'POST') {
      const result = await request.json();
      receive?.(result);
      return new Response('ok');
    }
    if (url.pathname !== '/') return new Response('not found', { status: 404 });
    return new Response('<!doctype html><meta charset="utf-8"><title>B01 browser baseline</title><script type="module" src="/client.js"></script>', {
      headers: { 'Content-Type': 'text/html; charset=utf-8' },
    });
  },
});

const chrome = Bun.env.VALLE_BENCH_CHROME ?? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const selections = Bun.argv.slice(2).length ? Bun.argv.slice(2) : ['webgl:10000', 'dom:10000', 'webgl:99856'];
const resultPath = Bun.env.VALLE_BENCH_CAPACITY_RESULT
  ?? join(tmpdir(), `b01-browser-capacity-${Date.now()}.json`);
if (await Bun.file(resultPath).exists()) throw new Error(`result already exists: ${resultPath}`);
const reports: any[] = [];
try {
  for (const selection of selections) {
    const [mode, count] = selection.split(':');
    for (let repeat = 1; repeat <= 3; repeat++) {
      const profile = await mkdtemp(join(tmpdir(), 'b01-baseline-chrome-'));
      const result = new Promise<any>((resolve) => { receive = resolve; });
      const process = Bun.spawn([
        chrome, '--headless=new', '--enable-gpu', '--use-angle=metal',
        '--disable-software-rasterizer', '--no-first-run', '--no-default-browser-check',
        `--user-data-dir=${profile}`, `${server.url}?mode=${mode}&count=${count}`,
      ], { stdout: 'ignore', stderr: 'ignore' });
      try {
        const value = await Promise.race([
          result,
          Bun.sleep(90000).then(() => ({ error: `timeout for ${selection} repeat ${repeat}` })),
        ]);
        reports.push({ repeat, ...value });
        console.log(JSON.stringify({ repeat, ...value }));
        if (value.error) throw new Error(value.error);
      } finally {
        receive = null;
        process.kill();
        await process.exited;
        await rm(profile, { recursive: true, force: true });
      }
    }
  }
} finally {
  await Bun.write(resultPath, JSON.stringify(reports, null, 2) + '\n');
  console.log(JSON.stringify({ resultPath, reports: reports.length }));
  server.stop();
}
