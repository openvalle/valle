// Studio end to end: `valle project studio` on a timeline in a project, driven through
// Playwright. It plays the timeline as the native engine draws it; a clip dragged on the
// timeline moves; edits undo and redo; saving writes the file; a change made outside while a
// draft is open is a conflict that keeps the draft; an export renders natively.
//
//   bun web/tests/studio.ts <valle binary>

import { mkdir, mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { evaluate as evaluatePage, launch, root } from "./browser.ts";

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
  await Bun.write(
    join(project, "project.json"),
    JSON.stringify({ name: "film", timeline: "cut.timeline.json" }),
  );
  await Bun.write(
    join(project, "other.motion.tsx"),
    `
export const composition = {width: 160, height: 90, fps: 30, duration: 2};
export default function Scene() {
  return <View style={{width: "100%", height: "100%", backgroundColor: "#e11d48"}} />;
}`,
  );
  await Bun.write(join(project, "notes.md"), "Notes for this project.");
  await Bun.write(
    join(project, "empty.timeline.json"),
    JSON.stringify({ canvas: CUT.canvas, tracks: {} }),
  );
  const linked = join(project, "linked.timeline.json");
  await Bun.write(
    linked,
    JSON.stringify({
      canvas: CUT.canvas,
      tracks: {
        visual: [
          {
            items: [
              CUT.tracks.visual[0].items[0],
              { transition: "fade", inOffset: 0.1, outOffset: 0.1 },
              CUT.tracks.visual[0].items[1],
            ],
          },
        ],
        caption: [
          { clips: [{ id: "title", text: "Title", start: { clip: "b" }, duration: 0.5 }] },
          {
            clips: [
              {
                id: "detail",
                text: "Detail",
                start: { clip: "title", offset: 0.5 },
                duration: 0.5,
              },
            ],
          },
        ],
      },
    }),
  );
  run(["timeline", "render", cut, "--frame", "20", "-o", join(dir, "native.png")]);

  const service = Bun.spawn([valle, "project", "studio", project, "--port", "0", "--events"], {
    stdout: "pipe",
    stderr: "inherit",
  });
  const browser = await launch().catch(async (error) => {
    service.kill();
    await service.exited;
    throw error;
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
    expect(new URL(ready.url).search === "", "Studio opens without a token");
    const page = await browser.newPage({
      viewport: { width: 1400, height: 900 },
      deviceScaleFactor: 1,
      colorScheme: "light",
    });
    console.log(`browser: Chromium ${browser.version()}`);
    page.on("pageerror", (error) => console.log(`exception: ${error.message}`));
    // Headless CI supplies only the system dialog's answer; the native render stays real.
    let outputFolder = project;
    let savePath: string | null | undefined;
    await page.route("**/api/save-file", async (route) => {
      const { name } = route.request().postDataJSON();
      expect(name.endsWith(".mp4"), "the system save dialog receives an MP4 name");
      await route.fulfill({
        json: {
          status: "ok",
          path: savePath === undefined ? join(outputFolder, "renders", name) : savePath,
        },
      });
    });
    await page.goto(ready.url);
    const evaluate = (expression: string) =>
      evaluatePage(
        page,
        `(async () => { const studio = document.querySelector("valle-studio"); ${expression} })()`,
      );
    const isolated = await evaluate("return crossOriginIsolated;");
    expect(isolated === true, "the page is isolated across origins");
    await until("the player", () =>
      evaluate("return studio.player !== undefined && studio.layout !== undefined;"),
    );

    expect(
      await evaluate(
        'const logo = studio.shadowRoot.querySelector("img.logo"); return logo.complete && logo.naturalWidth > 0;',
      ),
      "the Valle logo is served and displayed",
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

    await evaluate("studio.player.loop = true; studio.player.seekFrame(0);");
    await page.getByRole("button", { name: "Play", exact: true }).click();
    const rate = await until("live playback FPS", () =>
      evaluate(`
      const text = studio.shadowRoot.querySelector('[data-metric="fps"]').textContent;
      return text.includes("fps") && Number.parseFloat(text);`),
    );
    expect(rate > 0 && rate <= 31, `the 30 fps video's measured rate is ${rate} fps`);
    expect(
      await evaluate(`
        const head = studio.shadowRoot.querySelector('.preview-head').getBoundingClientRect();
        const fps = studio.shadowRoot.querySelector('[data-metric="fps"]').getBoundingClientRect();
        return Math.abs(fps.x + fps.width / 2 - head.x - head.width / 2) < 1
          && !studio.shadowRoot.querySelector('[data-metric="dropped"], [data-metric="frame-time"]');`),
      "only live FPS is shown, centered above the preview",
    );
    await page.getByRole("button", { name: "Pause", exact: true }).click();
    await evaluate("studio.player.loop = false;");
    expect(
      (await page.locator('[data-metric="fps"]').innerText()) === "— fps",
      "paused playback does not show stale live FPS",
    );

    // Panels change the workspace without replacing its active preview.
    await evaluate("window.preview = studio.player;");
    await page.getByRole("tab", { name: "Code", exact: true }).click();
    expect(
      await page.getByRole("textbox", { name: "Source code" }).isVisible(),
      "Code opens the source editor",
    );
    await page.getByRole("tab", { name: "Properties", exact: true }).click();
    await page.getByRole("button", { name: "Focus preview", exact: true }).click();
    expect(
      !(await page.getByRole("complementary", { name: "Inspector", exact: true }).isVisible()),
      "focus hides the side panels",
    );
    await page.getByRole("button", { name: "Focus preview", exact: true }).click();
    expect(
      await evaluate("return studio.player === window.preview;"),
      "switching panels keeps the player",
    );

    const filter = page.getByRole("textbox", { name: "Filter files" });
    await filter.fill("nothing-matches");
    expect(
      await evaluate(
        'return studio.shadowRoot.querySelectorAll(".files button.file").length === 0;',
      ),
      "the file filter narrows the list",
    );
    await filter.press("Space");
    expect(
      !(await evaluate("return studio.player.playing;")),
      "typing a space does not start playback",
    );
    await filter.fill("");

    const divider = page.getByRole("separator", { name: "Resize left panel" });
    const handle = (await divider.boundingBox())!;
    await page.mouse.move(handle.x + handle.width / 2, handle.y + handle.height / 2);
    await page.mouse.down();
    await page.mouse.move(handle.x + handle.width / 2 + 32, handle.y + handle.height / 2);
    await page.mouse.up();
    expect(
      await evaluate(
        'return Math.round(studio.shadowRoot.querySelector(".left").getBoundingClientRect().width) === 268;',
      ),
      "dragging resizes the file panel",
    );

    const clipWidth = () =>
      evaluate(
        'return studio.shadowRoot.querySelector("timeline-view").shadowRoot.querySelector(".clip").getBoundingClientRect().width;',
      );
    const fitWidth = await clipWidth();
    await page.getByRole("slider", { name: "Timeline zoom" }).fill("2");
    await until("the timeline zoom", async () => (await clipWidth()) > fitWidth * 1.9);
    expect(Math.abs((await clipWidth()) - fitWidth * 2) < 1, "timeline zoom expands the clips");
    await page.getByRole("button", { name: "Fit", exact: true }).click();
    await until("fit timeline", async () => Math.abs((await clipWidth()) - fitWidth) < 1);

    // A placed clip dragged 0.5 s later on the timeline.
    const box = await evaluate(`
      const view = studio.shadowRoot.querySelector("timeline-view");
      const r = view.shadowRoot.querySelector('[data-at="tracks.visual[1].clips[0]"]').getBoundingClientRect();
      return { x: r.x + r.width / 2, y: r.y + r.height / 2, perSecond: r.width / 0.5 };`);
    await page.mouse.move(box.x, box.y);
    await page.mouse.down();
    await page.mouse.move(box.x + box.perSecond * 0.5, box.y, { steps: 5 });
    await page.mouse.up();
    const moved = await until("the drag's edit", () =>
      evaluate(`const c = studio.layout.tracks[1].clips[0]; return c.start > 0.9 && c.start;`),
    );
    expect(Math.abs(moved - 1) < 1e-6, `the dragged clip starts at ${moved} s`);
    expect((await evaluate("return studio.drafts;")).length === 1, "the drag is a draft");

    // Selecting a scene switches the preview; unrelated text stays in the editor.
    const openFile = async (path: string, width: number) => {
      await page.locator(`button.file[title="${path}"]`).click();
      await until(`preview ${path}`, () =>
        evaluate(
          `return studio.player?.info.width === ${width} && !studio.shadowRoot.querySelector(".error") && !studio.shadowRoot.textContent.includes("Updating preview");`,
        ),
      );
    };
    await openFile("other.motion.tsx", 160);
    expect(
      await evaluate(
        "return !studio.timeline && studio.player.currentFrame === 0 && studio.layout === undefined;",
      ),
      "selecting Motion starts at frame zero and clears the timeline",
    );
    const color = await evaluate(
      "await studio.player.draw(0); return [...(await studio.player.pixels()).slice(0, 4)];",
    );
    expect(color.join() === "225,29,72,255", `the selected Motion is drawn (${color})`);
    expect(
      await evaluate('return studio.drafts.includes("cut.timeline.json");'),
      "switching files keeps the timeline draft",
    );
    await evaluate("window.preview = studio.player;");
    await page.locator('button.file[title="notes.md"]').click();
    expect(
      (await page.getByRole("textbox", { name: "Source code" }).inputValue()) ===
        "Notes for this project.",
      "a document opens as source",
    );
    expect(
      await evaluate("return studio.player === window.preview;"),
      "opening a document keeps the current preview",
    );

    await page.getByRole("button", { name: "Play", exact: true }).click();
    await openFile("cut.timeline.json", 320);
    expect(
      await evaluate("return studio.player.playing;"),
      "switching while playing continues playback",
    );
    await page.getByRole("button", { name: "Pause", exact: true }).click();
    expect(
      await evaluate("return studio.timeline && studio.layout.tracks[1].clips[0].start === 1;"),
      "returning to the timeline restores its draft",
    );

    // Several clicks while players are opening: the last selection wins.
    await evaluate(`
      for (const path of ["other.motion.tsx", "cut.timeline.json", "other.motion.tsx"]) {
        studio.shadowRoot.querySelector('button.file[title="' + path + '"]').click();
      }`);
    await until("the last selection", () =>
      evaluate(
        "return studio.player?.info.width === 160 && !studio.timeline && !studio.shadowRoot.textContent.includes('Updating preview');",
      ),
    );
    expect(
      await evaluate("return studio.player.currentFrame === 0;"),
      "rapid selection ends on the last file at frame zero",
    );
    await openFile("cut.timeline.json", 320);

    // Undo, redo, and a sequence reordered.
    await evaluate("await studio.undo();");
    expect((await evaluate("return studio.drafts;")).length === 0, "undone, nothing is changed");
    await evaluate("await studio.redo();");
    await page.locator('.clip[data-at="tracks.visual[0].items[1]"]').click();
    await evaluate(`await studio.edit({ op: "move", clip: "b", index: 0 });`);
    expect(
      (await page.getByRole("textbox", { name: "Clip name" }).getAttribute("placeholder")) === "b",
      "reordering keeps the moved clip selected",
    );
    await evaluate(
      `await studio.edit({ op: "set", clip: "b", field: "name", value: "Moved clip" });`,
    );
    await openFile("other.motion.tsx", 160);
    await page.getByRole("button", { name: "Undo", exact: true }).click();
    await until("undo navigates to the affected file", () =>
      evaluate(
        'return studio.timeline && studio.player?.info.width === 320 && studio.shadowRoot.textContent.includes("Undid edit in cut.timeline.json");',
      ),
    );
    expect(
      await evaluate(
        'return !JSON.parse(studio.text("cut.timeline.json")).tracks.visual[0].items[0].name;',
      ),
      "cross-file undo restores the clip and shows its timeline",
    );
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
    await openFile("other.motion.tsx", 160);
    await openFile("cut.timeline.json", 320);
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
    // Export from the actual dialog, including the unsaved edit shown in the preview.
    await evaluate(
      'await studio.edit({ op: "set", clip: "a", field: "name", value: "Exported draft" });',
    );
    await page.getByRole("button", { name: "Export", exact: true }).click();
    await page.keyboard.press("Backspace");
    expect(
      await evaluate("return studio.layout.tracks[1].clips.length === 1;"),
      "a dialog keeps workspace shortcuts inactive",
    );
    await page.getByRole("combobox", { name: "Export resolution" }).selectOption("1280x720");
    await page.getByRole("button", { name: "Export video", exact: true }).click();
    const id = await until("the export job", () => evaluate("return studio.job(1)?.id;"));
    expect(
      JSON.parse(await Bun.file(cut).text()).tracks.visual[0].items[1].name === "Exported draft",
      "export saves the preview's draft first",
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
    expect(
      await Bun.file(join(project, "renders/cut.mp4")).exists(),
      "the project's render is there",
    );
    const video = page.getByRole("link", { name: "Open video" });
    expect(
      (await page.request.get(new URL((await video.getAttribute("href"))!, ready.url).href)).ok(),
      "the exported video link is readable",
    );
    await page.getByRole("button", { name: "Close export" }).click();
    await openFile("other.motion.tsx", 160);
    await evaluate('studio.shadowRoot.querySelector("#output-size").value = "";');
    // A Motion selected in a project exports as Motion, not as the project's default timeline.
    const motionJob = await evaluate("return await studio.export();");
    const motionResult = await until("Motion export", () =>
      evaluate(`return studio.job(${motionJob})?.result;`),
    );
    expect(
      motionResult.status === "ok" &&
        motionResult.width === 160 &&
        motionResult.height === 90 &&
        motionResult.file === "renders/other.mp4",
      "export follows the selected Motion",
    );

    // The system dialog may return any destination, including one outside the workspace.
    const output = join(dir, "导出 videos", "成片.mp4");
    savePath = output;
    await page.getByRole("button", { name: "Export", exact: true }).click();
    expect(
      (await page.getByRole("button", { name: "Browse…", exact: true }).count()) === 0,
      "the export dialog has no custom folder browser",
    );
    await page.getByRole("button", { name: "Export video", exact: true }).click();
    const custom = await until("custom export", () => evaluate("return studio.job(3)?.result;"));
    expect(
      custom.status === "ok" && (await Bun.file(output).exists()),
      "the native render uses the system dialog's destination",
    );
    const exported = await page.getByRole("link", { name: "Open video" }).getAttribute("href");
    const download = await page.request.get(new URL(exported!, ready.url).href);
    expect(
      download.ok() &&
        Buffer.from(await download.body()).equals(
          Buffer.from(await Bun.file(output).arrayBuffer()),
        ),
      "the outside-workspace video link returns the exported file",
    );

    // Cancelling the system dialog leaves the existing file intact without saving drafts.
    await Bun.write(output, "keep the existing file");
    savePath = null;
    await page.getByRole("button", { name: "Export again", exact: true }).click();
    await until("save dialog cancelled", async () =>
      (await page.getByRole("status").allTextContents()).some((text) =>
        text.includes("Export cancelled"),
      ),
    );
    expect(
      (await Bun.file(output).text()) === "keep the existing file" &&
        !(await evaluate("return studio.job(4);")),
      "cancelling the save dialog keeps the existing file and starts no render",
    );
    savePath = output;
    await page.getByRole("button", { name: "Export video", exact: true }).click();
    const replacement = await until("replacement export", () =>
      evaluate("return studio.job(4)?.result;"),
    );
    expect(
      replacement.status === "ok" && (await Bun.file(output).bytes()).length > 100,
      "a replacement approved by the system dialog needs no second confirmation",
    );
    savePath = undefined;
    await page.getByRole("button", { name: "Close export" }).click();
    await Bun.write(join(project, "source.mp4"), Bun.file(join(project, "renders/cut.mp4")));
    await page.getByRole("tab", { name: "Files", exact: true }).click();
    await page.locator('button.file[title="empty.timeline.json"]').click();
    await until("empty timeline", () =>
      evaluate("return studio.timeline && studio.layout?.duration === 0 && !studio.player;"),
    );
    expect(
      await page.getByRole("button", { name: "Export", exact: true }).isDisabled(),
      "an empty timeline cannot export",
    );
    await page.getByRole("tab", { name: "Media", exact: true }).click();
    await page.locator('button.asset[title="source.mp4"]').click();
    await page.getByRole("button", { name: "Add to timeline", exact: true }).click();
    await until("first media preview", () =>
      evaluate(
        "return studio.player?.info.duration === 2 && studio.layout?.tracks[0].clips.length === 1 && !studio.shadowRoot.textContent.includes('Updating preview');",
      ),
    );
    expect(
      await evaluate(
        'return JSON.parse(studio.text("empty.timeline.json")).resources.source === "source.mp4";',
      ),
      "adding media creates the first track and resource binding",
    );
    await evaluate(
      'window.lastValid = studio.player; window.lastText = studio.text("empty.timeline.json"); await studio.edit({op:"trim",clip:"source",duration:20});',
    );
    expect(
      await evaluate(
        'return studio.player === window.lastValid && studio.text("empty.timeline.json") === window.lastText && studio.shadowRoot.textContent.includes("Edit not applied");',
      ),
      "an overlong trim keeps the valid document and preview",
    );
    await evaluate('await studio.edit({op:"delete",clip:"source"});');
    expect(
      await evaluate("return studio.layout.duration === 0 && !studio.player;"),
      "deleting the last clip leaves an editable empty timeline",
    );
    await evaluate("await studio.undo();");
    expect(
      await evaluate("return studio.player.info.duration === 2;"),
      "undo restores the last deleted clip",
    );
    await evaluate(
      'await Promise.all([studio.edit({op:"set",clip:"source",field:"name",value:"Queued edit"}), studio.edit({op:"trim",clip:"source",duration:1})]);',
    );
    expect(
      await evaluate(
        'const c=JSON.parse(studio.text("empty.timeline.json")).tracks.visual[0].items[0]; return c.name === "Queued edit" && c.duration === 1;',
      ),
      "concurrent edits are applied in order without losing either change",
    );
    // Navigation waits for an edit's media validation instead of cancelling it halfway.
    await page.getByRole("tab", { name: "Files", exact: true }).click();
    await evaluate(
      'window.pendingEdit = studio.edit({op:"set",clip:"source",field:"name",value:"Before navigation"});',
    );
    await openFile("other.motion.tsx", 160);
    await evaluate("await window.pendingEdit;");
    expect(
      await evaluate(
        'return !studio.timeline && JSON.parse(studio.text("empty.timeline.json")).tracks.visual[0].items[0].name === "Before navigation";',
      ),
      "switching files during an edit keeps the edit and selected file",
    );
    // Rename through the inspector, then delete through the timeline: both update relations
    // as one undoable edit, and the saved JSON is usable by the native CLI too.
    await openFile("linked.timeline.json", 320);
    await page.locator('.clip[data-at="tracks.visual[0].items[2]"]').click();
    await page.getByText("Advanced · clip JSON", { exact: true }).click();
    const clipJson = page.getByRole("textbox", { name: "Clip JSON", exact: true });
    const renamed = JSON.parse(await clipJson.inputValue());
    renamed.id = "hero";
    await clipJson.fill(JSON.stringify(renamed));
    await clipJson.press("Tab");
    await until("renamed clip and preview", () =>
      evaluate(
        'return studio.layout.tracks[0].clips[1].id === "hero" && !studio.shadowRoot.textContent.includes("Updating preview");',
      ),
    );
    expect(
      (await page.getByRole("textbox", { name: "Clip name" }).getAttribute("placeholder")) ===
        "hero",
      "changing an id through Advanced JSON keeps the clip selected",
    );
    expect(
      await evaluate(
        'const d=JSON.parse(studio.text("linked.timeline.json")); return d.tracks.caption[0].clips[0].start.clip === "hero" && d.tracks.caption[1].clips[0].start.clip === "title";',
      ),
      "renaming updates the direct anchor and keeps the dependent chain",
    );
    await evaluate("await studio.save();");
    expect(
      JSON.parse(await Bun.file(linked).text()).tracks.caption[0].clips[0].start.clip === "hero",
      "saving persists the renamed anchor",
    );
    await evaluate(
      'window.linkedText = studio.text("linked.timeline.json"); window.linkedPlayer = studio.player; await studio.edit({op:"set",clip:"hero",field:"id",value:"a"});',
    );
    expect(
      await evaluate(
        'return studio.text("linked.timeline.json") === window.linkedText && studio.player === window.linkedPlayer;',
      ),
      "a duplicate id keeps the document and player unchanged",
    );
    await page.locator('.clip[data-at="tracks.visual[0].items[2]"]').click();
    await page.keyboard.press("Delete");
    await until("linked deletion", () =>
      evaluate(
        'return studio.layout.tracks.flatMap(t=>t.clips).length === 1 && !studio.shadowRoot.textContent.includes("Updating preview");',
      ),
    );
    expect(
      await evaluate(
        'const d=JSON.parse(studio.text("linked.timeline.json")); return d.tracks.visual[0].items.length === 1 && d.tracks.caption.every(t=>t.clips.length === 0);',
      ),
      "deleting a clip removes its transition and recursively anchored captions",
    );
    expect(
      (await page.getByRole("textbox", { name: "Clip name" }).count()) === 0,
      "deleting clears the removed selection",
    );
    await evaluate("await studio.undo();");
    expect(
      await evaluate(
        'return studio.text("linked.timeline.json") === window.linkedText && studio.player.info.duration === 2;',
      ),
      "one undo restores the clip, transition and all anchored captions",
    );
    await evaluate("await studio.redo(); await studio.save();");
    expect(
      JSON.parse(await Bun.file(linked).text()).tracks.visual[0].items.length === 1,
      "redo and save persist the whole linked deletion",
    );
    expect(
      JSON.parse(run(["timeline", "check", linked, "--json"])).status === "ok",
      "the saved linked edit passes native timeline check",
    );
    // Standalone Studio does not have the renders/ directory created with a project.
    for (const kind of ["motion", "timeline"] as const) {
      const folder = join(dir, kind);
      outputFolder = folder;
      await mkdir(folder);
      const entry = kind === "motion" ? "standalone.motion.tsx" : "standalone.timeline.json";
      await Bun.write(
        join(folder, entry),
        kind === "motion"
          ? await Bun.file(join(project, "other.motion.tsx")).text()
          : JSON.stringify(CUT),
      );
      const standalone = Bun.spawn(
        [valle, kind, "studio", join(folder, entry), "--port", "0", "--events"],
        {
          stdout: "pipe",
          stderr: "inherit",
        },
      );
      try {
        const { value } = await standalone.stdout.getReader().read();
        const ready = JSON.parse(new TextDecoder().decode(value).split("\n")[0]);
        await page.goto(ready.url);
        await until(`standalone ${kind} preview`, () => evaluate("return !!studio.player;"));
        await page.getByRole("button", { name: "Export", exact: true }).click();
        await page.getByRole("button", { name: "Export video", exact: true }).click();
        const result = await until(`standalone ${kind} export`, () =>
          evaluate("return studio.job(1)?.result;"),
        );
        expect(
          result.status === "ok",
          `standalone ${kind} exports without an existing output directory: ${JSON.stringify(result)}`,
        );
        expect(
          await Bun.file(join(folder, "renders/standalone.mp4")).exists(),
          `standalone ${kind} creates its output video`,
        );
        const link = await page.getByRole("link", { name: "Open video" }).getAttribute("href");
        expect(
          (await page.request.get(new URL(link!, ready.url).href)).ok(),
          `standalone ${kind} can open the exported video`,
        );
      } finally {
        standalone.kill();
        await standalone.exited;
      }
    }
  } finally {
    service.kill();
    await Promise.all([browser.close(), service.exited]);
  }
  if (failed.length > 0) {
    throw new Error(`Studio (in ${dir}):\n${failed.join("\n")}`);
  }
  await rm(dir, { recursive: true, force: true });
}

await main();
