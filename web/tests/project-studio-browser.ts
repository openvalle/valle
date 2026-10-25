// Real browser acceptance: home → imports → source reload/conflict → export → restore → move.
import { mkdtemp, readFile, writeFile, rename } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { browserLaunchCommand, resolveBrowser, terminateBrowser } from "./parity/lib/browser.ts";

const root = resolve(import.meta.dir, "../..");
const cli = process.env.VALLE_TEST_CLI ?? join(root, "target/debug/valle");
const runtime = process.env.VALLE_WEB_RUNTIME_DIR ?? join(root, "web/dist");
const browser = resolveBrowser();
if (!browser) {
  if (process.env.VALLE_WEB_PLAYER_REQUIRE_BROWSER || process.env.VALLE_WEB_PARITY_REQUIRE_BROWSER)
    throw new Error("browser unavailable");
  console.log(JSON.stringify({ status: "skipped", reason: "browser unavailable" }));
  process.exit(0);
}
const evidence = await mkdtemp(join(tmpdir(), "valle-project-studio-"));
const home = join(evidence, "home"),
  project = join(evidence, "promo");
const env = { ...process.env, VALLE_HOME: home };
const children: Bun.Subprocess[] = [];
let chrome: Bun.Subprocess | undefined, socket: WebSocket | undefined;
const events: unknown[] = [];
function assert(value: unknown, message: string): asserts value {
  if (!value) throw new Error(message);
}
async function deadline<T>(work: Promise<T>, label: string, milliseconds = 120_000): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([
      work,
      new Promise<never>((_, reject) => {
        timer = setTimeout(() => reject(new Error(`${label} timed out`)), milliseconds);
      }),
    ]);
  } finally {
    clearTimeout(timer);
  }
}
async function poll<T>(
  work: () => Promise<T>,
  ready: (value: T) => boolean,
  label: string,
): Promise<T> {
  const end = Date.now() + 120_000;
  let last: unknown;
  while (Date.now() < end) {
    try {
      const value = await work();
      last = value;
      if (ready(value)) return value;
    } catch (error) {
      last = String(error);
    }
    await Bun.sleep(200);
  }
  throw new Error(`${label}: ${JSON.stringify(last)}`);
}
async function run(args: string[]): Promise<any> {
  const child = Bun.spawn([cli, "--json", ...args], {
    env,
    cwd: evidence,
    stdout: "pipe",
    stderr: "pipe",
  });
  const [output, errors, code] = await Promise.all([
    new Response(child.stdout).text(),
    new Response(child.stderr).text(),
    child.exited,
  ]);
  assert(code === 0, `${args.join(" ")}: ${output}\n${errors}`);
  return JSON.parse(output);
}
async function studio(args: string[]): Promise<string> {
  const child = Bun.spawn([cli, "--events", ...args, "--port", "0", "--web-assets-dir", runtime], {
    env,
    cwd: evidence,
    stdout: "pipe",
    stderr: Bun.file(join(evidence, `studio-${children.length}.log`)),
  });
  children.push(child);
  const reader = child.stdout.getReader();
  let text = "";
  while (true) {
    const part = await deadline(reader.read(), "Studio startup");
    assert(!part.done, "Studio exited before ready");
    text += new TextDecoder().decode(part.value);
    const lines = text.split("\n");
    text = lines.pop()!;
    for (const line of lines) {
      const event = JSON.parse(line);
      if (event.type === "ready") {
        void (async () => {
          while (!(await reader.read()).done) {}
          reader.releaseLock();
        })();
        return event.data.url;
      }
    }
  }
}
function wav(): Uint8Array {
  const rate = 8000,
    bytes = new Uint8Array(44 + rate * 2),
    view = new DataView(bytes.buffer);
  for (const [at, text] of [
    [0, "RIFF"],
    [8, "WAVEfmt "],
    [36, "data"],
  ] as const)
    bytes.set(new TextEncoder().encode(text), at);
  view.setUint32(4, bytes.length - 8, true);
  view.setUint32(16, 16, true);
  view.setUint16(20, 1, true);
  view.setUint16(22, 1, true);
  view.setUint32(24, rate, true);
  view.setUint32(28, rate * 2, true);
  view.setUint16(32, 2, true);
  view.setUint16(34, 16, true);
  view.setUint32(40, rate * 2, true);
  return bytes;
}

try {
  await writeFile(join(evidence, "beat.wav"), wav());
  await run(["assets", "add", join(evidence, "beat.wav"), "--tag", "test", "--note", "Music bed"]);
  await run(["assets", "annotate", "music/beat.wav", "--at", "0.25", "--text", "Cue"]);
  await run(["assets", "add", join(root, "assets/fonts/noto/NotoSans-Regular.ttf")]);
  const homeUrl = await studio(["project", "studio"]);
  const profile = join(evidence, "chrome");
  chrome = Bun.spawn(
    [
      ...browserLaunchCommand(browser!),
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
    {
      stdout: "ignore",
      stderr: Bun.file(join(evidence, "chrome.log")),
    },
  );
  const port = await poll(
    async () =>
      Number((await readFile(join(profile, "DevToolsActivePort"), "utf8")).split("\n")[0]),
    (port) => port > 0,
    "Chrome startup",
  );
  const targets = (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()) as any[];
  socket = new WebSocket(targets.find((target) => target.type === "page").webSocketDebuggerUrl);
  await deadline(
    new Promise<void>((ok, bad) => {
      socket!.onopen = () => ok();
      socket!.onerror = () => bad(new Error("CDP connection"));
    }),
    "CDP connection",
  );
  let sequence = 0;
  const pending = new Map<
    number,
    { resolve: (value: any) => void; reject: (error: Error) => void }
  >();
  socket.onmessage = (event) => {
    const message = JSON.parse(String(event.data)),
      request = pending.get(message.id);
    if (!request) {
      if (message.method === "Runtime.exceptionThrown" && events.length < 100) events.push(message);
      return;
    }
    pending.delete(message.id);
    if (message.error) request.reject(new Error(JSON.stringify(message.error)));
    else request.resolve(message.result);
  };
  const cdp = async (method: string, params: object = {}): Promise<any> => {
    const id = ++sequence;
    try {
      return await deadline(
        new Promise((resolve, reject) => {
          pending.set(id, { resolve, reject });
          socket!.send(JSON.stringify({ id, method, params }));
        }),
        method,
      );
    } finally {
      pending.delete(id);
    }
  };
  const evaluate = async (expression: string): Promise<any> => {
    const result = await cdp("Runtime.evaluate", {
      expression,
      returnByValue: true,
      awaitPromise: true,
    });
    if (result.exceptionDetails) throw new Error(JSON.stringify(result.exceptionDetails));
    return result.result.value;
  };
  const state = () =>
    evaluate(
      `(()=>({url:location.href,boot:globalThis.valleStudioHost?{project:globalThis.valleStudioHost.boot.project,session:{kind:globalThis.valleStudioHost.boot.session.kind}}:null,state:document.querySelector('valle-studio-app')?{...document.querySelector('valle-studio-app').shellState,saving:document.querySelector('#projectControls')?.controlsState.saving,session:undefined}:null,text:document.body.innerText,source:document.querySelector('#sourceText')?.value,banner:document.querySelector('#stageBanner')?.textContent}))()`,
    );
  const ready = async (label: string) => {
    const value = await poll(
      state,
      (value) => value.state?.previewStatus === "ready" || value.state?.previewStatus === "error",
      label,
    );
    assert(
      value.state.previewStatus === "ready",
      `${label}: ${value.banner || value.state.previewMessage || value.text}`,
    );
    return value;
  };
  const click = (selector: string) =>
    evaluate(`document.querySelector(${JSON.stringify(selector)}).click()`);
  const field = (selector: string, value: string) =>
    evaluate(
      `(()=>{const input=document.querySelector(${JSON.stringify(selector)}); input.value=${JSON.stringify(value)}; input.dispatchEvent(new Event('input',{bubbles:true}));})()`,
    );
  const panelIdle = () =>
    poll(
      () =>
        evaluate(`!!document.querySelector('.studio-project-panel > nav button:not([disabled])')`),
      Boolean,
      "project panel idle",
    );
  const screenshot = async (name: string) => {
    const result = await cdp("Page.captureScreenshot", { format: "png" });
    await writeFile(join(evidence, `${name}.png`), Buffer.from(result.data, "base64"));
  };
  const drop = (files: Array<{ name: string; bytes: string }>) =>
    evaluate(
      `(()=>{const data=new DataTransfer();for(const file of ${JSON.stringify(files)})data.items.add(new File([Uint8Array.fromBase64(file.bytes)],file.name));document.querySelector('#timelinePane').dispatchEvent(new DragEvent('drop',{bubbles:true,dataTransfer:data}));})()`,
    );
  const disk = async () => JSON.parse(await readFile(join(project, "timeline.json"), "utf8"));
  await cdp("Page.enable");
  await cdp("Runtime.enable");
  await cdp("Page.navigate", { url: homeUrl });
  await poll(state, (value) => value.text.includes("Create a project"), "home");
  await screenshot("home");
  await field('[aria-label="New project directory"]', project);
  await field('[name="width"]', "32");
  await field('[name="height"]', "32");
  await field('[name="fps"]', "4");
  await evaluate(
    `document.querySelector('[aria-label="New project directory"]').closest('form').requestSubmit()`,
  );
  await poll(
    state,
    (value) => value.boot?.project?.entry && value.state?.previewStatus === "empty",
    "new empty project",
  );
  const emptyTimeline = await disk();
  const externalTimeline = structuredClone(emptyTimeline);
  externalTimeline.tracks.visual = [
    { clips: [{ kind: "solid", color: "#2468acff", start: 0, duration: 1 }] },
  ];
  await writeFile(join(project, "timeline.json"), JSON.stringify(externalTimeline));
  await ready("empty project accepts an external Timeline edit");
  await writeFile(join(project, "timeline.json"), JSON.stringify(emptyTimeline));
  await cdp("Page.reload");
  await poll(state, (value) => value.state?.previewStatus === "empty", "empty project restored");

  const studioUrl = (await state()).url;
  const attackScript =
    "parent.__valleSandboxEscape = true; document.documentElement.dataset.executed = 'yes'; fetch('/studio/boot').then(r => r.text()).then(text => parent.__valleStolenBoot = text);";
  await writeFile(
    join(project, "media/sandbox.html"),
    `<html><body><script>${attackScript}</script>Sandbox test</body></html>`,
  );
  await writeFile(
    join(project, "media/sandbox.svg"),
    `<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><script>${attackScript}</script><rect width="16" height="16" fill="red"/></svg>`,
  );
  for (const extension of ["html", "svg"]) {
    const url = new URL(`/project/file/media/sandbox.${extension}`, studioUrl).href;
    const embedded = await evaluate(
      `new Promise(resolve => { const frame=document.createElement('iframe'); frame.src=${JSON.stringify(url)}; frame.onload=()=>{ const opaque=frame.contentDocument===null; frame.remove(); resolve({opaque, escaped:!!globalThis.__valleSandboxEscape, stolen:!!globalThis.__valleStolenBoot}); }; document.body.append(frame); })`,
    );
    assert(
      embedded.opaque && !embedded.escaped && !embedded.stolen,
      `sandboxed ${extension} cannot access Studio`,
    );
    await cdp("Page.navigate", { url });
    await poll(
      () => evaluate("document.readyState"),
      (value) => value === "complete",
      `top-level ${extension}`,
    );
    assert(
      !(await evaluate("!!document.documentElement.dataset.executed")),
      `top-level ${extension} scripts blocked`,
    );
    await cdp("Page.navigate", { url: studioUrl });
    await poll(
      state,
      (value) => value.state?.previewStatus === "empty",
      "Studio after sandbox test",
    );
  }
  assert(
    await evaluate(
      `new Promise(resolve => { const image=new Image(); image.onload=()=>resolve(image.naturalWidth===16); image.onerror=()=>resolve(false); image.src='/project/file/media/sandbox.svg'; })`,
    ),
    "sandboxed SVG remains usable as an image",
  );
  console.error("project browser: empty external edit and HTML/SVG isolation passed");
  const image = (
    await readFile(join(root, "crates/valle-compiler/tests/fixtures/motion/modules/assets/dot.png"))
  ).toString("base64");
  await drop([{ name: "photo.png", bytes: image }]);
  await ready("first imported image");
  assert(
    (await disk()).tracks.visual[0].clips[0].kind === "image",
    "first drop creates and saves a clip",
  );
  console.error("project browser: home and first import passed");

  await click("#assetsPanelButton");
  await panelIdle();
  await poll(state, (value) => value.text.includes("Project files"), "asset panel");
  await evaluate(
    `Array.from(document.querySelectorAll('.studio-project-panel button')).find(button=>button.textContent?.trim()==='Library').click()`,
  );
  await poll(state, (value) => value.text.includes("music/beat.wav"), "library files");
  await click('[data-asset-path="music/beat.wav"]');
  assert(
    await evaluate(
      `!!document.querySelector('.asset-detail audio') && document.querySelectorAll('.asset-note-markers button').length===1`,
    ),
    "audio playback and timed notes",
  );
  await field('[aria-label="Asset tags"]', "test, browser");
  await evaluate(
    `document.querySelector('[aria-label="Asset tags"]').closest('form').requestSubmit()`,
  );
  await poll(
    async () => (await run(["assets", "show", "music/beat.wav"])).tags,
    (tags) => tags.includes("browser"),
    "tag edit",
  );
  await panelIdle();
  await field('[aria-label="Note text"]', "Browser note");
  await field('[aria-label="Note time"]', "0.5");
  await evaluate(
    `document.querySelector('[aria-label="Note text"]').closest('form').requestSubmit()`,
  );
  await poll(state, (value) => value.text.includes("Browser note"), "note edit");
  await panelIdle();
  await screenshot("assets");
  const font = await evaluate(
    `Array.from(document.querySelectorAll('[data-asset-path]')).find(button=>button.dataset.assetPath.startsWith('fonts/')).dataset.assetPath`,
  );
  await click(`[data-asset-path="${font}"]`);
  assert(await evaluate(`!!document.querySelector('.asset-font-sample')`), "font sample");
  await click('[data-asset-path="music/beat.wav"]');
  await evaluate(
    `(()=>{const data=new DataTransfer();const row=document.querySelector('[data-asset-path="music/beat.wav"]');row.dispatchEvent(new DragEvent('dragstart',{bubbles:true,dataTransfer:data}));document.querySelector('#timelinePane').dispatchEvent(new DragEvent('drop',{bubbles:true,dataTransfer:data}));})()`,
  );
  await poll(disk, (value) => value.tracks.audio?.length === 1, "library drag and save");
  await ready("library audio preview");
  await poll(
    state,
    (value) => !value.state.dirty && !value.state.saving && !value.state.conflict,
    "library import save acknowledged",
  );
  await click('[aria-label="Close project panel"]');
  // A file notification may arrive after the import edits the draft but before its save.
  await evaluate(
    `(()=>{const shell=document.querySelector('valle-studio-app');const notify=event=>{if(event.detail.intent.type==='dirty'&&event.detail.state.dirty){shell.removeEventListener('studio-statechange',notify);shell.dispatchEvent(new CustomEvent('studio-host-event',{detail:{type:'project',data:{}}}));}};shell.addEventListener('studio-statechange',notify);})()`,
  );
  const pausedSave = new Promise<string>((resolve) => {
    const listener = (event: MessageEvent) => {
      const message = JSON.parse(String(event.data));
      if (message.method === "Fetch.requestPaused") {
        socket!.removeEventListener("message", listener);
        resolve(message.params.requestId);
      }
    };
    socket!.addEventListener("message", listener);
  });
  await cdp("Fetch.enable", {
    patterns: [{ urlPattern: "*/timeline/edit", requestStage: "Response" }],
  });
  const source = `export const composition={width:32,height:32,fps:4,duration:1};export default function Card(){return <Scene><View style={{width:32,height:32,backgroundColor:'#67e8f9'}}/></Scene>;}`;
  await drop([{ name: "card.motion.tsx", bytes: Buffer.from(source).toString("base64") }]);
  await poll(disk, (value) => value.tracks.visual.length === 2, "component drop and save");
  const saveRequest = await deadline(pausedSave, "paused component save");
  await ready("component preview");
  assert(
    await evaluate(
      `document.querySelector('#projectControls').controlsState.saving && document.querySelector('valle-studio-app').shellState.conflict`,
    ),
    "import save is still pending with its file notification",
  );
  await click("#historyPanelButton");
  await panelIdle();
  await field('[aria-label="Snapshot message"]', "Before source edit");
  await evaluate(
    `document.querySelector('[aria-label="Snapshot message"]').closest('form').requestSubmit()`,
  );
  assert(
    !(await state()).text.includes("Save or resolve the current draft"),
    "snapshot waits for the pending import save before checking conflicts",
  );
  assert(
    await evaluate(`document.querySelector('.studio-project-panel > nav button').disabled`),
    "snapshot remains pending until the import save response arrives",
  );
  await cdp("Fetch.continueRequest", { requestId: saveRequest });
  await cdp("Fetch.disable");
  await poll(state, (value) => value.text.includes("Snapshot 1"), "save snapshot");
  await panelIdle();
  await screenshot("history");
  await click('[aria-label="Close project panel"]');
  const component = join(project, "components/card.motion.tsx");
  await writeFile(component, source.replace("#67e8f9", "#ef4444"));
  await poll(
    state,
    (value) => value.source?.includes("#ef4444") && value.state?.previewStatus === "ready",
    "agent source reload",
  );
  await screenshot("project");
  await click("#sourceToggle");
  await field("#sourceText", source.replace("#67e8f9", "#22c55e"));
  await writeFile(component, source.replace("#67e8f9", "#a855f7"));
  await poll(
    state,
    (value) => value.source?.includes("#22c55e") && Boolean(value.state?.conflictMessage),
    "dirty source conflict",
  );
  const dialog = new Promise<void>((resolve) => {
    const listener = (event: MessageEvent) => {
      const message = JSON.parse(String(event.data));
      if (message.method === "Page.javascriptDialogOpening") {
        socket!.removeEventListener("message", listener);
        void cdp("Page.handleJavaScriptDialog", { accept: true }).then(() => resolve());
      }
    };
    socket!.addEventListener("message", listener);
  });
  await click("#sourceReload");
  await deadline(dialog, "discard test draft");
  await poll(
    state,
    (value) => !value.state?.dirty && value.source?.includes("#a855f7"),
    "explicit source reload",
  );
  // The Timeline conflict is cleared only by an explicit document reload.
  await evaluate(
    `document.querySelector('#projectControls').dispatchEvent(new CustomEvent('studio-project-intent',{detail:{type:'reload'},bubbles:true}))`,
  );
  await poll(
    state,
    (value) => value.state?.previewStatus === "ready" && !value.state?.conflictMessage,
    "resolved conflict preview",
  );
  await click("#exportVideo");
  await panelIdle();
  await poll(state, (value) => value.text.includes("Export records"), "export panel");
  await evaluate(
    `Array.from(document.querySelectorAll('.studio-project-panel button')).find(button=>button.textContent?.trim()==='Export video').click()`,
  );
  await poll(
    async () => (await run(["project", "show", "-C", project])).lastExport,
    (value) => Boolean(value),
    "native export record",
  );
  const exported = (await run(["project", "show", "-C", project])).lastExport;
  assert(
    (await Bun.file(join(project, exported.outputs[0].path)).size) > 0,
    "export file was delivered",
  );
  await click("#historyPanelButton");
  await panelIdle();
  await poll(state, (value) => value.text.includes("Before source edit"), "history list");
  await evaluate(
    `Array.from(document.querySelectorAll('.project-history-list li')).find(row=>row.textContent.includes('Before source edit')).querySelector('button').click()`,
  );
  await poll(
    async () => readFile(component, "utf8"),
    (value) => value.includes("#67e8f9"),
    "restore source snapshot",
  );
  await ready("restored preview");
  await click('[aria-label="Close project panel"]');
  const unchanged = await readFile(join(project, "timeline.json"), "utf8");
  await drop([
    { name: "kept.bin", bytes: Buffer.from("retained on partial import").toString("base64") },
    {
      name: "bad.motion.tsx",
      bytes: Buffer.from(
        "import {x} from './missing';export default function Bad(){return <Scene/>;}",
      ).toString("base64"),
    },
  ]);
  await poll(
    async () => Bun.file(join(project, "media/kept.bin")).exists(),
    Boolean,
    "successful part of a partial import",
  );
  await Bun.sleep(500);
  assert(
    (await readFile(join(project, "timeline.json"), "utf8")) === unchanged,
    "partial import cannot edit the Timeline",
  );
  console.error("project browser: library, reload, export, restore, and partial import passed");

  const moved = join(evidence, "moved");
  await rename(project, moved);
  await cdp("Page.navigate", { url: homeUrl });
  await poll(state, (value) => value.text.includes("Recent projects"), "home after move");
  await field('[aria-label="Open project directory"]', moved);
  await evaluate(
    `document.querySelector('[aria-label="Open project directory"]').closest('form').requestSubmit()`,
  );
  await ready("reopened moved project");
  assert(
    (await state()).boot.project.root.replaceAll("\\", "/").endsWith("/moved"),
    "moved project context",
  );
  const secondary = join(moved, "other.json");
  await writeFile(
    secondary,
    JSON.stringify({
      canvas: { width: 32, height: 32, fps: 4 },
      tracks: { visual: [{ clips: [{ kind: "solid", color: "#123456", start: 0, duration: 1 }] }] },
    }),
  );
  const secondaryUrl = await studio(["timeline", "studio", secondary]);
  await cdp("Page.navigate", { url: secondaryUrl });
  await ready("secondary Timeline");
  assert(
    await evaluate(`document.querySelector('#historyPanelButton').hidden`),
    "secondary Timeline has no history controls",
  );
  const refused = await evaluate(
    `fetch('/project/execute',{method:'POST',headers:{'content-type':'application/json','x-valle-token':globalThis.valleStudioHost.boot.session.token},body:'{"op":"save"}'}).then(r=>r.json())`,
  );
  assert(
    refused.error?.message.includes("entry Timeline"),
    "backend also rejects secondary snapshot operations",
  );
  const standalone = join(evidence, "standalone.motion.tsx");
  await writeFile(standalone, source);
  const motionUrl = await studio(["motion", "studio", standalone]);
  await cdp("Page.navigate", { url: motionUrl });
  await ready("standalone Motion");
  await click("#saveAsProject");
  await poll(state, (value) => value.text.includes("Project directory"), "save as project dialog");
  const converted = join(evidence, "converted");
  await field("#saveAsProjectPath", converted);
  await evaluate(`document.querySelector('#saveAsProjectForm').requestSubmit()`);
  await ready("saved Motion project");
  await poll(
    state,
    (value) =>
      value.boot?.session.kind === "timeline-file" && value.boot.project.name === "converted",
    "converted project session",
  );
  assert(
    await Bun.file(join(converted, "components/standalone.motion.tsx")).exists(),
    "conversion copies the component",
  );
  await screenshot("converted");
  await writeFile(join(evidence, "browser-events.json"), JSON.stringify(events, null, 2));
  console.log(
    JSON.stringify({
      status: "ok",
      evidence,
      snapshot: exported.snapshot,
      export: exported.outputs[0].path,
      checks: [
        "home",
        "first clip",
        "library tags and notes",
        "audio and font previews",
        "library drag",
        "source reload and conflict",
        "native export",
        "restore",
        "partial import",
        "move",
        "secondary Timeline",
        "Motion save as project",
      ],
    }),
  );
} catch (error) {
  await writeFile(join(evidence, "browser-events.json"), JSON.stringify(events, null, 2));
  console.error(`Project Studio evidence: ${evidence}`);
  throw error;
} finally {
  socket?.close();
  if (chrome) await terminateBrowser(chrome);
  for (const child of children) {
    if (child.exitCode === null) child.kill();
    await child.exited;
  }
}
