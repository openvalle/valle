// Studio end to end: `valle timeline studio` on a timeline in a project, Studio driven in the
// pinned Chrome. It plays the timeline as the native engine draws it; a clip dragged on the
// timeline moves; edits undo and redo; saving writes the file; a change made outside while a
// draft is open is a conflict that keeps the draft; an export renders natively.
//
//   bun web/tests/studio.ts <valle binary>

import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { Cdp, flags, install, listening, root } from "./browser.ts";

const valle = process.argv[2];
if (!valle) {
  throw new Error("usage: bun web/tests/studio.ts <valle binary>");
}

/** A timeline of two clips in sequence, and one placed over them. */
const CUT = {
  canvas: { width: 320, height: 180, fps: 30, background: "#000000ff" },
  tracks: {
    visual: [
      {
        items: [
          { id: "a", kind: "solid", color: "#2563ebff", duration: 1 },
          { id: "b", kind: "solid", color: "#0d9488ff", duration: 1 },
        ],
      },
      {
        clips: [{ id: "c", kind: "solid", color: "#f59e0bff", start: 0.5, duration: 0.5 }],
      },
    ],
  },
};

/** Runs the native engine. */
function run(args: string[], cwd = root): string {
  const done = Bun.spawnSync([valle, ...args], { cwd, stdout: "pipe", stderr: "pipe" });
  if (done.exitCode !== 0) {
    throw new Error(`valle ${args.join(" ")}: ${done.stderr.toString()}${done.stdout.toString()}`);
  }
  return done.stdout.toString();
}

/** `check` until it holds, for up to 10 s. */
async function until<T>(what: string, check: () => Promise<T | undefined | false>): Promise<T> {
  for (let i = 0; i < 100; i++) {
    const value = await check();
    if (value) {
      return value;
    }
    await Bun.sleep(100);
  }
  throw new Error(`${what}: not in 10 s`);
}

async function main() {
  const dir = await mkdtemp(join(tmpdir(), "valle-studio-"));
  const project = join(dir, "film");
  run(["project", "create", project]);
  const cut = join(project, "cut.timeline.json");
  await Bun.write(cut, JSON.stringify(CUT, null, 2));
  run(["timeline", "render", cut, "--frame", "20", "-o", join(dir, "native.png")]);

  const service = Bun.spawn([valle, "timeline", "studio", cut, "--port", "0", "--events"], {
    stdout: "pipe",
    stderr: "inherit",
  });
  const profile = await mkdtemp(join(tmpdir(), "valle-chrome-"));
  const browser = Bun.spawn([await install("chrome"), ...flags(profile)], {
    stdout: "ignore",
    stderr: "pipe",
  });
  const failed: string[] = [];
  const expect = (holds: boolean, what: string) => {
    console.log(`${holds ? "ok" : "FAILED"}: ${what}`);
    if (!holds) failed.push(what);
  };
  try {
    const reader = service.stdout.getReader();
    const { value } = await reader.read();
    const ready = JSON.parse(new TextDecoder().decode(value).split("\n")[0]);
    const cdp = await Cdp.open(await listening(browser.stderr));
    const { targetId } = await cdp.send("Target.createTarget", { url: "about:blank" });
    const { sessionId } = await cdp.send("Target.attachToTarget", { targetId, flatten: true });
    const page = (method: string, params: object = {}) => cdp.send(method, params, sessionId);
    await page("Page.enable");
    await page("Runtime.enable");
    cdp.on("Runtime.exceptionThrown", ({ exceptionDetails: e }) => {
      console.log(`exception: ${e.exception?.description ?? e.text}`);
    });
    await page("Emulation.setDeviceMetricsOverride", {
      width: 1400,
      height: 900,
      deviceScaleFactor: 1,
      mobile: false,
    });
    const loaded = cdp.event("Page.loadEventFired", sessionId);
    await page("Page.navigate", { url: ready.url });
    await loaded;
    const evaluate = async (expression: string) => {
      const { result, exceptionDetails } = await page("Runtime.evaluate", {
        expression: `(async () => { const studio = document.querySelector("valle-studio"); ${expression} })()`,
        awaitPromise: true,
        returnByValue: true,
      });
      if (exceptionDetails) {
        throw new Error(exceptionDetails.exception?.description ?? exceptionDetails.text);
      }
      return result.value;
    };
    const isolated = await evaluate("return crossOriginIsolated;");
    expect(isolated === true, "the page is isolated across origins");
    await until("the player", () =>
      evaluate("return studio.player !== undefined && studio.layout !== undefined;"),
    );

    // It plays the timeline as the native engine draws it.
    const most = await evaluate(`
      await studio.player.draw(20);
      const x = await studio.player.pixels();
      const bitmap = await createImageBitmap(await (await fetch("data:image/png;base64,${Buffer.from(await Bun.file(join(dir, "native.png")).arrayBuffer()).toString("base64")}")).blob(), { colorSpaceConversion: "none", premultiplyAlpha: "none" });
      const c = new OffscreenCanvas(bitmap.width, bitmap.height).getContext("2d");
      c.drawImage(bitmap, 0, 0);
      const y = c.getImageData(0, 0, bitmap.width, bitmap.height).data;
      if (x.length !== y.length) return Infinity;
      let most = 0;
      for (let i = 0; i < x.length; i++) most = Math.max(most, Math.abs(x[i] - y[i]));
      return most;`);
    expect(most <= 2, `the preview's frame 20 is the native one (most ${most} apart)`);

    // A placed clip dragged 0.5 s later on the timeline.
    const box = await evaluate(`
      const view = studio.shadowRoot.querySelector("timeline-view");
      const r = view.shadowRoot.querySelector('[data-at="tracks.visual[1].clips[0]"]').getBoundingClientRect();
      return { x: r.x + r.width / 2, y: r.y + r.height / 2, perSecond: r.width / 0.5 };`);
    const mouse = (type: string, x: number) =>
      page("Input.dispatchMouseEvent", {
        type,
        x,
        y: box.y,
        button: "left",
        buttons: 1,
        clickCount: 1,
      });
    await mouse("mousePressed", box.x);
    for (let k = 1; k <= 5; k++) await mouse("mouseMoved", box.x + (box.perSecond * 0.5 * k) / 5);
    await mouse("mouseReleased", box.x + box.perSecond * 0.5);
    const moved = await until("the drag's edit", () =>
      evaluate(`const c = studio.layout.tracks[1].clips[0]; return c.start > 0.9 && c.start;`),
    );
    expect(Math.abs(moved - 1) < 1e-6, `the dragged clip starts at ${moved} s`);
    expect((await evaluate("return studio.drafts;")).length === 1, "the drag is a draft");

    // Undo, redo, and a sequence reordered.
    await evaluate("await studio.undo();");
    expect((await evaluate("return studio.drafts;")).length === 0, "undone, nothing is changed");
    await evaluate("await studio.redo();");
    await evaluate(`await studio.edit({ op: "move", clip: "b", index: 0 });`);
    const first = await evaluate("return studio.layout.tracks[0].clips[0].id;");
    expect(first === "b", `the sequence starts with ${first}`);

    // Saved: the file is the edit.
    await evaluate("await studio.save();");
    const saved = JSON.parse(await Bun.file(cut).text());
    expect(
      saved.tracks.visual[0].items[0].id === "b" && saved.tracks.visual[1].clips[0].start === 1,
      "saving wrote the edits",
    );
    expect((await evaluate("return studio.drafts;")).length === 0, "nothing is left to save");

    // A change made outside, over a draft: a conflict that keeps the draft.
    await evaluate(`await studio.edit({ op: "set", clip: "a", field: "name", value: "Mine" });`);
    const theirs = { ...saved, name: "Theirs" };
    await Bun.write(cut, JSON.stringify(theirs, null, 2));
    await until("the conflict", () => evaluate("return studio.conflicts.length === 1;"));
    await evaluate("await studio.save();");
    expect(
      JSON.parse(await Bun.file(cut).text()).name === "Theirs",
      "a save does not overwrite theirs",
    );
    expect((await evaluate("return studio.drafts;")).length === 1, "the draft is kept");
    await evaluate(`await studio.settle("cut.timeline.json", "theirs");`);
    const settled = await evaluate("return [studio.drafts.length, studio.conflicts.length];");
    expect(settled.join() === "0,0", "taking theirs drops the draft");

    // The export's resolution is independent of the preview and the saved design canvas.
    const beforeExport = await evaluate(
      "return [studio.player.info.width, studio.player.info.height];",
    );
    const id = await evaluate(
      'studio.renderRoot.querySelector("#output-size").value = "1280x720"; return await studio.export();',
    );
    const job = await until("the export", () =>
      evaluate(`const j = studio.job(${id}); return j?.result && j;`),
    );
    expect(job.result.status === "ok", `the export rendered: ${JSON.stringify(job.result)}`);
    expect(
      job.result.width === 1280 && job.result.height === 720,
      "the export uses the selected 720p size",
    );
    const afterExport = await evaluate(
      "return [studio.player.info.width, studio.player.info.height];",
    );
    expect(
      beforeExport.join() === afterExport.join(),
      "exporting does not resize the preview's canvas",
    );
    expect(await Bun.file(join(project, "renders/cut.mp4")).exists(), "renders/cut.mp4 is there");
    await cdp.send("Browser.close").catch(() => {});
  } finally {
    browser.kill();
    service.kill();
    await Promise.all([browser.exited, service.exited]);
    await rm(profile, { recursive: true, force: true });
  }
  if (failed.length > 0) {
    throw new Error(`Studio (in ${dir}):\n${failed.join("\n")}`);
  }
  await rm(dir, { recursive: true, force: true });
}

await main();
