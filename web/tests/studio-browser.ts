// Invoked by the Rust Studio acceptance tests. Run outside the sandbox on the host machine.
import { mkdtemp, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { createHash } from "node:crypto";
import { resolveBrowser } from "./parity/lib/browser.ts";

const root = resolve(import.meta.dir, "../..");
const mode = process.argv[2] ?? "audio";
const cli = process.env.VALLE_TEST_CLI ?? join(root, "target/debug/valle");
const runtime = process.env.VALLE_WEB_RUNTIME_DIR ?? join(root, "web/dist");
const browser = resolveBrowser();
if (!browser) {
  if (
    process.env.VALLE_WEB_PLAYER_REQUIRE_BROWSER ||
    process.env.VALLE_WEB_PARITY_REQUIRE_BROWSER
  )
    throw new Error("Chrome/Chromium is required");
  console.log(
    JSON.stringify({ status: "skipped", reason: "browser unavailable" }),
  );
  process.exit(0);
}
const dir = await mkdtemp(join(tmpdir(), "valle-studio-machine-"));
const source = `export const composition={width:240,height:140,fps:30,duration:3};
export const controls={props:{size:number({default:40,min:10,max:200})} AUDIO_CONTROL};
AUDIO_ANALYSIS
export default function Main(ctx,props){return <Scene style={{width:240,height:140,backgroundColor:"#101010"}}>
<View key="box" style={{position:"absolute",left:10,top:10,width:props.size,height:props.size AUDIO_HEIGHT,backgroundColor:"#67e8f9",opacity:0.5+ctx.progress*0.5}}/></Scene>;}`;
function assert(ok: unknown, message: string): asserts ok {
  if (!ok) throw new Error(message);
}
async function deadline<T>(
  work: Promise<T>,
  label: string,
  ms = 120_000,
): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([
      work,
      new Promise<never>((_, reject) => {
        timer = setTimeout(() => reject(new Error(`${label} timed out`)), ms);
      }),
    ]);
  } finally {
    clearTimeout(timer);
  }
}
async function stop(child: ReturnType<typeof Bun.spawn>): Promise<void> {
  child.kill("SIGTERM");
  try {
    await deadline(child.exited, "process shutdown", 5_000);
  } catch {
    child.kill("SIGKILL");
    await deadline(child.exited, "forced process shutdown", 5_000);
  }
}
async function poll<T>(
  work: () => Promise<T>,
  ready: (value: T) => boolean,
  label: string,
): Promise<T> {
  const end = Date.now() + 120_000;
  let last: T | undefined;
  while (Date.now() < end) {
    last = await work();
    if (ready(last)) return last;
    await Bun.sleep(200);
  }
  throw new Error(`${label} timed out: ${JSON.stringify(last)}`);
}
function wav(): Uint8Array {
  const rate = 48_000,
    n = rate * 3,
    bytes = new Uint8Array(44 + n * 2),
    view = new DataView(bytes.buffer);
  const text = (at: number, s: string) =>
    bytes.set(new TextEncoder().encode(s), at);
  text(0, "RIFF");
  view.setUint32(4, 36 + n * 2, true);
  text(8, "WAVEfmt ");
  view.setUint32(16, 16, true);
  view.setUint16(20, 1, true);
  view.setUint16(22, 1, true);
  view.setUint32(24, rate, true);
  view.setUint32(28, rate * 2, true);
  view.setUint16(32, 2, true);
  view.setUint16(34, 16, true);
  text(36, "data");
  view.setUint32(40, n * 2, true);
  for (let i = 0; i < n; i++)
    view.setInt16(
      44 + i * 2,
      i % rate < rate / 2 ? (i % 100) * 200 - 10000 : 0,
      true,
    );
  return bytes;
}
async function run(
  input: string,
  args: string[],
  edit?: { from: string; to: string },
  hot = false,
) {
  const evidence = await mkdtemp(join(dir, "run-"));
  const server = Bun.spawn(
    [
      cli,
      "--events",
      "motion",
      "studio",
      input,
      "--port",
      "0",
      "--web-assets-dir",
      runtime,
      ...args,
    ],
    { stdout: "pipe", stderr: Bun.file(join(evidence, "studio.log")) },
  );
  let chrome: ReturnType<typeof Bun.spawn> | undefined,
    socket: WebSocket | undefined;
  try {
    const reader = server.stdout.getReader();
    let lines = "",
      port = 0;
    while (!port) {
      const part = await deadline(reader.read(), "Studio startup");
      assert(!part.done, "Studio exited before ready");
      lines += new TextDecoder().decode(part.value);
      const parts = lines.split("\n");
      lines = parts.pop()!;
      for (const line of parts) {
        if (!line) continue;
        const event = JSON.parse(line);
        if (event.type === "ready") port = event.data.port;
      }
    }
    void (async () => {
      while (!(await reader.read()).done) {}
      reader.releaseLock();
    })();
    const profile = await mkdtemp(join(evidence, "chrome-"));
    chrome = Bun.spawn(
      [
        browser!,
        "--headless=new",
        "--no-sandbox",
        "--no-first-run",
        "--no-default-browser-check",
        "--disable-extensions",
        "--mute-audio",
        "--window-size=1280,900",
        "--remote-debugging-port=0",
        `--user-data-dir=${profile}`,
        "about:blank",
      ],
      { stdout: "ignore", stderr: Bun.file(join(evidence, "chrome.log")) },
    );
    const debugPort = await poll(
      async () => {
        try {
          return Number(
            (await readFile(join(profile, "DevToolsActivePort"), "utf8")).split(
              "\n",
            )[0],
          );
        } catch {
          return 0;
        }
      },
      (p) => p > 0,
      "Chrome startup",
    );
    const targets = (await (
      await fetch(`http://127.0.0.1:${debugPort}/json/list`)
    ).json()) as Array<{ type: string; webSocketDebuggerUrl: string }>;
    const target = targets.find((t) => t.type === "page");
    assert(target, "Chrome page target");
    socket = new WebSocket(target.webSocketDebuggerUrl);
    await deadline(
      new Promise<void>((ok, bad) => {
        socket!.onopen = () => ok();
        socket!.onerror = () => bad(new Error("CDP connection"));
      }),
      "CDP connection",
    );
    let next = 1;
    const pending = new Map<
      number,
      { ok: (v: any) => void; bad: (e: Error) => void }
    >();
    socket.onmessage = (event) => {
      const reply = JSON.parse(String(event.data));
      const waiter = pending.get(reply.id);
      if (!waiter) return;
      pending.delete(reply.id);
      if (reply.error) waiter.bad(new Error(JSON.stringify(reply.error)));
      else waiter.ok(reply.result);
    };
    const cdp = async (
      method: string,
      params: Record<string, unknown> = {},
    ) => {
      const id = next++;
      const response = new Promise<any>((ok, bad) => {
        pending.set(id, { ok, bad });
        socket!.send(JSON.stringify({ id, method, params }));
      });
      try {
        return await deadline(response, method);
      } finally {
        pending.delete(id);
      }
    };
    const evaluate = async (expression: string) => {
      const result = await cdp("Runtime.evaluate", {
        expression,
        returnByValue: true,
        awaitPromise: true,
      });
      if (result.exceptionDetails)
        throw new Error(JSON.stringify(result.exceptionDetails));
      return result.result.value;
    };
    await cdp("Page.enable");
    await cdp("Runtime.enable");
    await cdp("Page.navigate", { url: `http://127.0.0.1:${port}/studio` });
    const state = () =>
      evaluate(
        `(()=>{const app=document.querySelector('valle-studio-app');return {state:app?.shellState,source:document.querySelector('#sourceText')?.value,status:document.querySelector('#sourceStatus')?.textContent,compiles:performance.getEntriesByName('valle-studio-compile').length,text:document.body.innerText};})()`,
      );
    await poll(
      state,
      (s) => s.state?.previewStatus === "ready" && !!s.source,
      "initial preview",
    );
    const userAgent = await evaluate("navigator.userAgent");
    assert(/Chrome\//.test(userAgent), "real Chrome required");
    await evaluate("document.querySelector('#sourceToggle').click()");
    const screenshot = async (name: string) => {
      await evaluate(
        "new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)))",
      );
      const clip = await evaluate(
        `(()=>{const r=document.querySelector('#studioPlayer').canvas.getBoundingClientRect();return {x:r.x,y:r.y,width:r.width,height:r.height,scale:1};})()`,
      );
      const result = await cdp("Page.captureScreenshot", {
        format: "png",
        clip,
      });
      const bytes = Buffer.from(result.data, "base64");
      await writeFile(join(evidence, `${name}.png`), bytes);
      return createHash("sha256").update(bytes).digest("hex");
    };
    const before = await state();
    const original = String(before.source);
    const first = await screenshot("before");
    const editSource = async (text: string) =>
      evaluate(
        `(()=>{const field=document.querySelector('#sourceText');field.value=${JSON.stringify(text)};field.dispatchEvent(new Event('input',{bubbles:true}));})()`,
      );
    const changed = edit
      ? original.replace(edit.from, edit.to)
      : original + "\n// Browser compilation acceptance\n";
    assert(changed !== original, "source edit must change the draft");
    await editSource(changed);
    const edited = await poll(
      state,
      (s) => s.state?.previewStatus === "ready" && s.compiles > before.compiles,
      "browser-compiled draft preview",
    );
    assert(
      edited.status === "Unsaved source",
      `draft status: ${edited.status}`,
    );
    const second = await screenshot("edited");
    if (edit)
      assert(
        first !== second,
        "edited source must change visible preview pixels",
      );
    if (hot) {
      const count = edited.compiles;
      await editSource("export default function Broken( {");
      await poll(
        state,
        (s) => s.state?.previewStatus === "error",
        "invalid source diagnostics",
      );
      assert(
        second === (await screenshot("invalid-last-good")),
        "invalid source must retain the last good preview pixels",
      );
      await editSource(changed);
      await poll(
        state,
        (s) => s.state?.previewStatus === "ready" && s.compiles > count,
        "recover after invalid source",
      );
      assert(
        second === (await screenshot("recovered")),
        "recover must restore the same current-frame pixels",
      );
    }
    if (edit) {
      await evaluate("document.querySelector('#sourceSave').click()");
      await poll(
        state,
        (s) => s.status === "Saved" && s.state?.previewStatus === "ready",
        "saved current preview",
      );
      assert(
        (await readFile(input, "utf8")) === changed,
        "saved source must match the edited draft on disk",
      );
    }
    const final = await state();
    assert(final.text.includes("Preview up to date"), "visible preview status");
    const report = {
      mode,
      input,
      userAgent,
      compiles: final.compiles,
      status: final.status,
      preview: final.state.previewStatus,
      pixelsChanged: first !== second,
      evidence,
    };
    const page = await cdp("Page.captureScreenshot", { format: "png" });
    await writeFile(
      join(evidence, "studio.png"),
      Buffer.from(page.data, "base64"),
    );
    await writeFile(
      join(evidence, "result.json"),
      JSON.stringify(report, null, 2),
    );
    console.log(JSON.stringify(report));
  } finally {
    socket?.close();
    try {
      if (chrome) await stop(chrome);
    } finally {
      await stop(server);
    }
  }
}
try {
  if (mode === "modules") {
    const modules = join(
      root,
      "crates/valle-compiler/tests/fixtures/motion/modules",
    );
    for (const [name, args] of [
      [
        "components/dashboard.tsx",
        ["--data", join(modules, "data-dashboard/dashboard.data.json")],
      ],
      ["components/route-network.tsx", []],
      [
        "components/code-build.tsx",
        ["--asset", `dot=${join(modules, "assets/dot.png")}`],
      ],
      ["components/brand-poster.tsx", []],
    ] as const)
      await run(join(modules, name), [...args]);
  } else {
    const audio = mode === "audio";
    const input = join(dir, "scene.motion.tsx");
    await writeFile(
      input,
      source
        .replace(
          "AUDIO_CONTROL",
          audio ? ',assets:{beat:asset({kind:"audio",required:true})}' : "",
        )
        .replace(
          "AUDIO_ANALYSIS",
          audio
            ? 'const BEAT=audioAnalysis("asset://beat",{bands:8,fps:30});'
            : "",
        )
        .replace("AUDIO_HEIGHT", audio ? "+BEAT.level(ctx.seconds)*10" : "")
        .replace("backgroundColor:\"#101010\"", mode === "effects" ? 'backgroundColor:"#101010",filter:"film-grain(4294967295 0.5 2px)"' : 'backgroundColor:"#101010"')
        .replace('opacity:0.5+ctx.progress*0.5', mode === "effects" ? 'opacity:0.5+ctx.progress*0.5,filter:"radial-blur(120px 70px 128px)"' : 'opacity:0.5+ctx.progress*0.5'),
    );
    const args: string[] = [];
    if (audio) {
      const file = join(dir, "beat.wav");
      await writeFile(file, wav());
      args.push("--asset", `beat=${file}`);
    }
    await run(
      input,
      args,
      { from: "default:40", to: "default:80" },
      mode === "hot",
    );
  }
} catch (error) {
  console.error(`Studio evidence: ${dir}`);
  throw error;
}
