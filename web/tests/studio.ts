// Studio end to end: `valle project studio` on a timeline in a project, driven through
// Playwright. It plays the timeline as the native engine draws it; a clip dragged on the
// timeline moves; edits undo and redo; saving writes the file; a change made outside while a
// draft is open is a conflict that keeps the draft; an export renders natively.
//
//   bun web/tests/studio.ts <valle binary>

import { mkdir, mkdtemp, rm, symlink } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { cache, evaluate as evaluatePage, launch, root } from "./browser.ts";

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
  await Bun.write(
    join(project, "card.motion.tsx"),
    `export const composition = { width: 320, height: 180, fps: 30, duration: 2 };
export default function Scene() {
  return (
    <View style={{ position: "absolute", inset: 0 }}>
      <View key="card" style={{ position: "absolute", left: 40, top: 40, width: 120, height: 60, backgroundColor: "#fff" }}>
        <Text key="title" style={{ fontSize: 20 }}>Hi</Text>
      </View>
    </View>
  );
}
`,
  );
  await Bun.write(
    join(project, "picked.timeline.json"),
    JSON.stringify({
      canvas: CUT.canvas,
      resources: { card: "card.motion.tsx" },
      tracks: {
        visual: [
          { clips: [{ id: "bg", kind: "solid", color: "#000000ff", start: 0, duration: 2 }] },
          { clips: [{ id: "a", kind: "motion", component: "card", start: 0, duration: 2 }] },
        ],
      },
    }),
  );
  await Bun.write(join(project, "notes.md"), "Notes for this project.");
  await Bun.write(join(project, "Card.tsx"), "export default function Card() { return <View />; }");
  await Bun.write(join(project, "data.json"), '{"title":"Hello"}');
  await Bun.write(
    join(project, "empty.timeline.json"),
    JSON.stringify({ canvas: CUT.canvas, tracks: {} }),
  );
  await Bun.write(
    join(project, "unnamed.timeline.json"),
    JSON.stringify({
      canvas: CUT.canvas,
      tracks: {
        visual: [
          { clips: [{ kind: "solid", color: "red", start: 0, duration: 2 }] },
          { clips: [] },
        ],
      },
    }),
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

  // The projects opened here are remembered beside the cache: this run's own place, with the
  // real cache linked in, keeps them out of the user's recent projects.
  await mkdir(cache, { recursive: true });
  await mkdir(join(dir, "state"));
  await symlink(cache, join(dir, "state/cache"));
  const env = { ...process.env, VALLE_CACHE_DIR: join(dir, "state/cache") };
  const service = Bun.spawn([valle, "project", "studio", project, "--port", "0", "--events"], {
    stdout: "pipe",
    stderr: "inherit",
    env,
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
      expect(/\.(mp4|wav)$/.test(name), "the system save dialog receives a media file name");
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
        return !!studio.shadowRoot.querySelector('.transport [data-metric="fps"]')
          && !studio.shadowRoot.querySelector('.preview-head')
          && !studio.shadowRoot.querySelector('[data-metric="dropped"], [data-metric="frame-time"]');`),
      "only live FPS is shown, beside the playback controls",
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

    // Tracks are numbered by kind; the ruler scrubs by dragging; arrows step a frame; a click
    // on empty track lets the selection go.
    const view = 'studio.shadowRoot.querySelector("timeline-view").shadowRoot';
    expect(
      (await evaluate(
        `return [...${view}.querySelectorAll(".label small")].map((e) => e.textContent).join();`,
      )) === "V1,V2",
      "tracks are labelled by their kind and place",
    );
    const ruler = await evaluate(
      `const r = ${view}.querySelector(".ruler").getBoundingClientRect(); const c = ${view}.querySelector(".clip").getBoundingClientRect(); return { x: r.x, y: r.y + r.height / 2, perSecond: c.width };`,
    );
    await page.mouse.move(ruler.x + ruler.perSecond * 0.2, ruler.y);
    await page.mouse.down();
    await page.mouse.move(ruler.x + ruler.perSecond * 1.5, ruler.y, { steps: 5 });
    await page.mouse.up();
    await until("the ruler's scrub", () => evaluate("return studio.player.currentFrame === 45;"));
    await page.keyboard.press("ArrowRight");
    await page.keyboard.press("Shift+ArrowLeft");
    await until("frame steps", () => evaluate("return studio.player.currentFrame === 16;"));
    await page.keyboard.press("Home");
    await until("the first frame", () => evaluate("return studio.player.currentFrame === 0;"));
    await page.locator('.clip[data-at="tracks.visual[1].clips[0]"]').click();
    expect(
      (await page.getByRole("textbox", { name: "Clip name" }).count()) === 1,
      "a click selects a clip",
    );
    await evaluate('window.lockedText = studio.text("cut.timeline.json");');
    await page.getByRole("button", { name: "Lock V2", exact: true }).click();
    await page.keyboard.press("Delete");
    await until("locked delete refused", () =>
      evaluate('return studio.shadowRoot.textContent.includes("is locked");'),
    );
    expect(
      await evaluate('return studio.text("cut.timeline.json") === window.lockedText;'),
      "locking a selected track prevents keyboard deletion",
    );
    await page.getByRole("textbox", { name: "Clip name" }).fill("Must not change");
    await page.getByRole("textbox", { name: "Clip name" }).press("Tab");
    await evaluate("await studio.save();");
    expect(
      await evaluate('return studio.text("cut.timeline.json") === window.lockedText;'),
      "locking a track also prevents inspector edits",
    );
    await page.getByRole("button", { name: "Lock V2", exact: true }).click();
    await page.mouse.click(ruler.x + ruler.perSecond * 1.8, ruler.y + 60);
    expect(
      (await page.getByRole("textbox", { name: "Clip name" }).count()) === 0,
      "a click on an empty track clears the selection",
    );

    // A placed clip dragged 0.5 s later on the timeline.
    let edits = 0;
    page.on("request", (request) => {
      if (new URL(request.url()).pathname === "/api/edit") edits++;
    });
    const box = await evaluate(`
      window.preview = studio.player;
      window.canvas = studio.shadowRoot.querySelector(".canvas canvas");
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

    expect(edits === 1, "a drag uses the layout returned by the edit request");
    expect(
      await evaluate(
        'return studio.player === window.preview && studio.shadowRoot.querySelector(".canvas canvas") === window.canvas;',
      ),
      "dragging keeps the player and its canvas",
    );

    await page.getByRole("tab", { name: "Code", exact: true }).click();
    const editor = page.getByRole("textbox", { name: "Source code" });
    const valid = await editor.inputValue();
    await editor.fill("{");
    await page.locator(".stage .error").waitFor();
    await editor.fill(valid);
    await page.locator(".stage .error").waitFor({ state: "detached" });
    expect(
      await evaluate(
        "return studio.player === window.preview && studio.layout.tracks[1].clips[0].start === 1;",
      ),
      "correcting invalid timeline source restores the preview on the same player",
    );

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
    for (const file of ["Card.tsx", "data.json"]) {
      await page.locator(`button.file[title="${file}"]`).click();
      const source = await Bun.file(join(project, file)).text();
      await until(
        `source of ${file}`,
        async () =>
          (await page.getByRole("textbox", { name: "Source code" }).inputValue()) === source,
      );
      expect(
        await evaluate(
          "return studio.player === window.preview && !studio.shadowRoot.querySelector('.error');",
        ),
        `opening ${file} keeps the current preview`,
      );
    }

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
    await evaluate(
      "window.preview = studio.player; await studio.player.draw(20); await studio.undo();",
    );
    expect((await evaluate("return studio.drafts;")).length === 0, "undone, nothing is changed");
    await evaluate("await studio.redo();");
    expect(
      await evaluate(
        "return studio.player === window.preview && studio.player.currentFrame === 20;",
      ),
      "undo and redo preserve the player and playhead",
    );
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
    await page.locator("details.export-audio > summary").click();
    await page.getByRole("combobox", { name: "Export format" }).selectOption("wav");
    expect(
      (await page.getByRole("combobox", { name: "Audio bitrate" }).count()) === 0 &&
        (await page.getByRole("combobox", { name: "Export resolution" }).count()) === 0,
      "WAV offers PCM without compressed bitrate or video resolution",
    );
    await page.getByRole("combobox", { name: "Export format" }).selectOption("mp4");
    await page.getByRole("combobox", { name: "Sample rate" }).selectOption("44100");
    await page.getByRole("combobox", { name: "Channels" }).selectOption("1");
    await page.getByRole("combobox", { name: "Audio bitrate" }).selectOption("128000");
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
    expect(
      job.result.audio?.codec === "aac" &&
        job.result.audio.sampleRate === 44100 &&
        job.result.audio.channels === 1 &&
        job.result.audio.bitrate === 128000,
      "the native export used the Studio audio settings",
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
    await openFile("cut.timeline.json", 320);
    await page.getByRole("button", { name: "Export", exact: true }).click();
    await page.getByRole("combobox", { name: "Export format" }).selectOption("wav");
    await page.locator("details.export-audio:not([open]) > summary").click();
    await page.getByRole("combobox", { name: "Audio encoding" }).selectOption("pcm-s16le");
    await page.getByRole("button", { name: "Export audio", exact: true }).click();
    const wavLink = page.getByRole("link", { name: "Open audio" });
    await wavLink.waitFor();
    const wav = await page.request.get(
      new URL((await wavLink.getAttribute("href"))!, ready.url).href,
    );
    const wavData = await wav.body();
    expect(
      wav.ok() && wavData.toString("ascii", 8, 12) === "WAVE" && wavData.readUInt16LE(34) === 16,
      "Studio exports and serves a 16-bit WAV",
    );
    await page.getByRole("combobox", { name: "Export format" }).selectOption("mp4");
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
    await page.getByRole("button", { name: "Lock C1", exact: true }).click();
    await evaluate('await studio.edit({op:"delete",clip:"hero"});');
    expect(
      await evaluate('return studio.text("linked.timeline.json") === window.linkedText;'),
      "a locked caption track also blocks cascading deletion from an unlocked video track",
    );
    await page.getByRole("button", { name: "Lock C1", exact: true }).click();
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
      await evaluate(
        'const message = studio.shadowRoot.querySelector(".statusbar .message").textContent; return ["removed hero", "removed title", "removed detail", "removed transition"].every(s => message.includes(s));',
      ),
      "the edit result reports every linked removal in Studio",
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
    // A linked video stays outside the project, but previews through the local service.
    const outside = join(dir, "linked video.mp4");
    await Bun.write(outside, Bun.file(join(project, "source.mp4")));
    run(["project", "add", outside, "--link", "-C", project]);
    await Bun.write(
      join(project, "linked-media.timeline.json"),
      JSON.stringify({
        canvas: CUT.canvas,
        resources: { video: outside },
        tracks: {
          visual: [{ items: [{ id: "video", kind: "video", src: "video", duration: 2 }] }],
        },
      }),
    );
    await page.reload();
    await until("project reopened", () => evaluate("return !!studio.player;"));
    const partial = page.waitForResponse(
      (res) =>
        new URL(res.url()).pathname === "/api/linked" &&
        res.request().resourceType() === "fetch" &&
        /^bytes=\d+-\d+$/.test(res.request().headers().range ?? "") &&
        res.status() === 206,
    );
    await openFile("linked-media.timeline.json", 320);
    const range = await partial;
    expect(
      /^bytes \d+-\d+\/\d+$/.test(range.headers()["content-range"] ?? ""),
      "the linked video is read through the player by byte range",
    );
    expect(
      await evaluate(
        "await studio.player.draw(20); return (await studio.player.pixels()).length === 320 * 180 * 4;",
      ),
      "a video linked outside the project decodes and draws in Studio",
    );

    const keptFrame = await evaluate(`
      window.preview = studio.player;
      const before = (await studio.player.pixels()).slice(0, 3);
      await studio.edit({op: "set", clip: "video", field: "name", value: "First cut"});
      window.videoPixel = (await studio.player.pixels()).slice(0, 3);
      return studio.player === window.preview && studio.player.currentFrame === 20 &&
        before.every((channel, i) => channel === window.videoPixel[i]);`);
    expect(keptFrame, "editing a video keeps its player, playhead and displayed pixels");
    await Bun.write(outside, Bun.file(join(project, "renders/other.mp4")));
    await until("the replacement linked video in the preview", () =>
      evaluate(`
      if (studio.player === window.preview || studio.player?.currentFrame !== 20) return false;
      const pixel = (await studio.player.pixels()).slice(0, 3);
      return pixel.some((channel, i) => Math.abs(channel - window.videoPixel[i]) > 40);`),
    );

    await openFile("unnamed.timeline.json", 320);
    const cross = await evaluate(`
      const v = studio.shadowRoot.querySelector("timeline-view").shadowRoot;
      const a = v.querySelector(".clip").getBoundingClientRect();
      const b = v.querySelector('[data-track="tracks.visual[1]"]').getBoundingClientRect();
      return {x:a.x+a.width/2, y:a.y+a.height/2, to:b.y+b.height/2};`);
    await page.mouse.move(cross.x, cross.y);
    await page.mouse.down();
    await page.mouse.move(cross.x, cross.to, { steps: 5 });
    await page.mouse.up();
    await until("cross-track move", () =>
      evaluate("return studio.layout.tracks[1].clips.length === 1;"),
    );
    expect(
      (await page.getByRole("textbox", { name: "Clip name" }).getAttribute("placeholder")) ===
        "visual0.0",
      "an unnamed clip dragged between tracks retains its identity and selection",
    );

    // Replace with the same byte length: neither the service nor browser may use length as
    // the media's identity. A valid MP4 free atom pads the shorter export.
    const originals = await Promise.all(
      ["other", "cut"].map(async (name) =>
        Buffer.from(await Bun.file(join(project, `renders/${name}.mp4`)).arrayBuffer()),
      ),
    );
    const length = Math.max(...originals.map((bytes) => bytes.length)) + 8;
    const videos = originals.map((bytes) => {
      const free = Buffer.alloc(length - bytes.length);
      free.writeUInt32BE(free.length);
      free.write("free", 4);
      return Buffer.concat([bytes, free]);
    });
    await Bun.write(join(project, "sample.mp4"), videos[0]);
    const sound = (amplitude: number) => {
      const rate = 8000,
        samples = rate * 2;
      const wav = Buffer.alloc(44 + samples * 2);
      wav.write("RIFF", 0);
      wav.writeUInt32LE(wav.length - 8, 4);
      wav.write("WAVEfmt ", 8);
      wav.writeUInt32LE(16, 16);
      wav.writeUInt16LE(1, 20);
      wav.writeUInt16LE(1, 22);
      wav.writeUInt32LE(rate, 24);
      wav.writeUInt32LE(rate * 2, 28);
      wav.writeUInt16LE(2, 32);
      wav.writeUInt16LE(16, 34);
      wav.write("data", 36);
      wav.writeUInt32LE(samples * 2, 40);
      for (let i = 0; i < samples; i++)
        wav.writeInt16LE(
          Math.round(Math.sin((i * 2 * Math.PI * 440) / rate) * amplitude * 32767),
          44 + i * 2,
        );
      return wav;
    };
    await Bun.write(join(project, "sample.wav"), sound(0.2));
    await Bun.write(
      join(project, "sound.timeline.json"),
      JSON.stringify({
        canvas: CUT.canvas,
        resources: { sound: "sample.wav" },
        tracks: { audio: [{ clips: [{ id: "sound", src: "sound", start: 0, duration: 2 }] }] },
      }),
    );
    await page.reload();
    await until("project with media", () => evaluate("return !!studio.player;"));
    await openFile("sound.timeline.json", 320);
    await until("audio track", () =>
      evaluate('return studio.layout?.tracks[0]?.kind === "audio";'),
    );
    await page.getByRole("tab", { name: "Media", exact: true }).click();
    const thumbnail = page.locator('.asset[title="sample.mp4"] .thumbnail img');
    await thumbnail.waitFor({ timeout: 15000 });
    const oldThumbnail = await thumbnail.getAttribute("src");
    await Bun.write(join(project, "sample.mp4"), videos[1]);
    await until(
      "replaced video's thumbnail",
      async () =>
        (await thumbnail.count()) && (await thumbnail.getAttribute("src")) !== oldThumbnail,
    );
    expect(true, "replacing a video invalidates its cached thumbnail");
    const waveform = page.locator("timeline-view .strip.wave path");
    const mediaWave = page.locator('.asset[title="sample.wav"] .wave path');
    await waveform.waitFor({ timeout: 15000 });
    await mediaWave.waitFor({ timeout: 15000 });
    const oldWave = await waveform.getAttribute("d"),
      oldMediaWave = await mediaWave.getAttribute("d");
    await Bun.write(join(project, "sample.wav"), sound(0.8));
    await until(
      "replaced sound's waveforms",
      async () =>
        (await waveform.count()) &&
        (await mediaWave.count()) &&
        (await waveform.getAttribute("d")) !== oldWave &&
        (await mediaWave.getAttribute("d")) !== oldMediaWave,
    );
    expect(true, "replacing a sound refreshes both the timeline and media browser waveforms");

    // A note on the frame shown: a part of the picture drawn over, kept in reviews/ with the
    // picture as seen.
    await page.getByRole("tab", { name: "Files", exact: true }).click();
    await openFile("cut.timeline.json", 320);
    await evaluate("studio.player.seekFrame(15);");
    await page.getByRole("button", { name: "Add a note", exact: true }).click();
    const layer = page.getByLabel("Draw the part of the picture the note is about");
    const area = (await layer.boundingBox())!;
    await page.mouse.move(area.x + area.width * 0.5, area.y + area.height * 0.25);
    await page.mouse.down();
    await page.mouse.move(area.x + area.width * 0.75, area.y + area.height * 0.75, { steps: 4 });
    await page.mouse.up();
    await page.getByRole("textbox", { name: "Note", exact: true }).fill("make the orange warmer");
    await page.getByRole("button", { name: "Add note", exact: true }).click();
    const notes = await until("the note written", async () => {
      const file = Bun.file(join(project, "reviews/notes.json"));
      return (await file.exists()) && (await file.json()).notes;
    });
    const note = notes[0];
    const near = (a: number, b: number) => Math.abs(a - b) < 0.02;
    expect(
      notes.length === 1 &&
        note.timeline === "cut.timeline.json" &&
        note.at.start === 0.5 &&
        near(note.region.x, 0.5) &&
        near(note.region.y, 0.25) &&
        near(note.region.w, 0.25) &&
        near(note.region.h, 0.5) &&
        note.thread[0].text === "make the orange warmer",
      `a note keeps its frame, region and text: ${JSON.stringify(note)}`,
    );
    const seen = Bun.file(join(project, "reviews", note.image));
    const bytes = new Uint8Array(await seen.arrayBuffer());
    expect(
      note.image === "crops/n1.png" && bytes[0] === 0x89 && bytes[1] === 0x50,
      "a note keeps the picture seen",
    );
    expect(
      !(await layer.count()) && !(await evaluate("return studio.player.playing;")),
      "adding the note closes it on the paused frame",
    );
    const listed = JSON.parse(run(["project", "-C", project, "notes", "--json"]));
    expect(listed.notes[0]?.now?.start === 0.5, "the CLI lists the note Studio added where it is");

    // A note over a span: a Shift-drag over the ruler marks it, and it is listed with the first
    // note in the notes panel, where an answer and a resolve are written.
    const ruled = await evaluate(
      `const r = ${view}.querySelector(".ruler").getBoundingClientRect(); const c = ${view}.querySelector(".clip").getBoundingClientRect(); return { x: r.x, y: r.y + r.height / 2, perSecond: c.width / studio.layout.tracks[0].clips[0].duration };`,
    );
    await page.keyboard.down("Shift");
    await page.mouse.move(ruled.x + ruled.perSecond * 1, ruled.y);
    await page.mouse.down();
    await page.mouse.move(ruled.x + ruled.perSecond * 1.5, ruled.y, { steps: 4 });
    await page.mouse.up();
    await page.keyboard.up("Shift");
    await page.getByRole("textbox", { name: "Note", exact: true }).fill("too long here");
    await page.getByRole("button", { name: "Add note", exact: true }).click();
    const spanned = await until("the span's note", async () => {
      const file = await Bun.file(join(project, "reviews/notes.json")).json();
      return file.notes[1];
    });
    expect(
      spanned.id === "n2" &&
        Math.abs(spanned.at.start - 1) < 0.05 &&
        Math.abs(spanned.at.end - 1.5) < 0.05 &&
        !spanned.region,
      `a Shift-drag over the ruler notes a span: ${JSON.stringify(spanned.at)}`,
    );
    await until("the notes on the ruler", () =>
      evaluate(`return ${view}.querySelectorAll(".note-mark").length === 2;`),
    );
    expect(true, "the ruler marks both notes");
    await evaluate(`${view}.querySelector('.note-mark[data-note="n1"]').click();`);
    await until("the note shown", () =>
      evaluate('return studio.shadowRoot.querySelector(".note.open img.note-picture")?.complete;'),
    );
    expect(
      (await evaluate("return studio.player.currentFrame;")) === 15 &&
        (await page.locator(".note-seen .note-region").count()) === 1,
      "a note on the ruler goes to its frame and shows its part of the picture",
    );
    run(["project", "-C", project, "notes", "reply", "n1", "warmer how?", "--by", "ai"]);
    await until("the answer from outside", () =>
      page.getByText("warmer how?", { exact: true }).count(),
    );
    expect(true, "an answer written by the CLI is shown in the panel");
    await page.getByRole("textbox", { name: "Reply", exact: true }).fill("more red");
    await page.getByRole("button", { name: "Reply", exact: true }).click();
    await page.getByText("more red", { exact: true }).waitFor();
    await page.getByRole("button", { name: "Resolve", exact: true }).click();
    const resolved = await until("the note resolved", async () => {
      const n1 = (await Bun.file(join(project, "reviews/notes.json")).json()).notes[0];
      return n1.status === "resolved" && n1;
    });
    expect(
      resolved.thread.map((m: { by: string }) => m.by).join() === "user,ai,user" &&
        !(await page.locator('.note[data-note="n1"]').count()),
      "a resolved note leaves the open notes",
    );
    await page.getByLabel("Notes shown").selectOption("resolved");
    await page.getByRole("button", { name: "Reopen", exact: true }).click();
    await until("the note reopened", async () => {
      const n1 = (await Bun.file(join(project, "reviews/notes.json")).json()).notes[0];
      return n1.status === "open";
    });
    expect(true, "a resolved note can be reopened");

    // A click on the picture while noting picks the element drawn there, a click again the one
    // beneath, and its line opens in the source; the note is on it.
    await page.getByLabel("Notes shown").selectOption("open");
    await page.getByRole("tab", { name: "Files", exact: true }).click();
    await openFile("picked.timeline.json", 320);
    await evaluate("studio.player.seekFrame(15);");
    await page.getByRole("button", { name: "Add a note", exact: true }).click();
    const canvas = (await layer.boundingBox())!;
    const card = { x: canvas.x + canvas.width * 0.2, y: canvas.y + canvas.height * 0.3 };
    await page.mouse.click(card.x, card.y);
    const picked = page.locator(".note-picked");
    await picked.getByText("card/title in a", { exact: true }).waitFor();
    expect(
      (await page.locator(".note-hit").count()) === 1,
      "a click while noting picks the element drawn there",
    );
    await page.mouse.click(card.x, card.y);
    await picked.getByText("card in a", { exact: true }).waitFor();
    expect(true, "a click again picks the element beneath");
    await picked.getByRole("button", { name: "card.motion.tsx:5:7", exact: true }).click();
    const line = await until("the line picked", () =>
      evaluate(
        `const t = studio.shadowRoot.querySelector('textarea[aria-label="Source code"]'); return t && t.value.slice(t.selectionStart, t.selectionEnd);`,
      ),
    );
    expect(
      line.trim().startsWith('<View key="card"'),
      `the element's line opens in the source: ${line}`,
    );
    await page.getByRole("textbox", { name: "Note", exact: true }).fill("bigger card");
    await page.getByRole("button", { name: "Add note", exact: true }).click();
    const onCard = await until("the picked note", async () => {
      const file = await Bun.file(join(project, "reviews/notes.json")).json();
      return file.notes[2];
    });
    const [top] = onCard.targets ?? [];
    expect(
      top?.clip === "a" &&
        top.path === "card" &&
        top.site === "card.motion.tsx:5:7" &&
        top.offset === 0.5 &&
        onCard.targets[1]?.clip === "bg" &&
        near(onCard.region.x, 40 / 320) &&
        near(onCard.region.w, 120 / 320),
      `a note on a picked element is on it, where it is drawn: ${JSON.stringify(onCard)}`,
    );

    // A note's frame as it was seen is compared over the frame as it is now, wiped across.
    await evaluate(`${view}.querySelector('.note-mark[data-note="n3"]').click();`);
    await page.getByRole("button", { name: "Compare", exact: true }).click();
    const before = page.getByAltText("n3 as it was seen", { exact: true });
    await before.waitFor();
    await page.getByLabel("Before and now").fill("1");
    expect(
      (await evaluate("return studio.player.currentFrame;")) === 15 &&
        (await before.evaluate((e) => getComputedStyle(e).clipPath)) === "inset(0px 0% 0px 0px)",
      "a note's frame as seen is compared over the frame now",
    );
    await page.getByRole("button", { name: "Compare", exact: true }).click();
    expect(!(await before.count()), "comparing ends");

    // A component reviewed alone takes notes too, on its elements and no clip.
    await page.getByRole("tab", { name: "Files", exact: true }).click();
    await openFile("card.motion.tsx", 320);
    await evaluate("studio.player.seekFrame(15);");
    await page.getByRole("button", { name: "Add a note", exact: true }).click();
    const alone = (await layer.boundingBox())!;
    await page.mouse.click(alone.x + alone.width * 0.2, alone.y + alone.height * 0.3);
    await picked.getByText("card/title", { exact: true }).waitFor();
    await page.getByRole("textbox", { name: "Note", exact: true }).fill("bolder title");
    await page.getByRole("button", { name: "Add note", exact: true }).click();
    const onComponent = await until("the component's note", async () => {
      const file = await Bun.file(join(project, "reviews/notes.json")).json();
      return file.notes[3];
    });
    expect(
      onComponent.component === "card.motion.tsx" &&
        !onComponent.timeline &&
        JSON.stringify(onComponent.targets) ===
          JSON.stringify([{ path: "card/title", site: "card.motion.tsx:6:9" }]),
      `a component takes notes on its elements: ${JSON.stringify(onComponent)}`,
    );
    await page.getByRole("tab", { name: "Notes", exact: true }).click();
    await until("the component's notes listed", () =>
      evaluate('return studio.shadowRoot.querySelectorAll(".notes .note").length === 1;'),
    );
    expect(true, "the component's notes are listed apart from the timeline's");

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
        { stdout: "pipe", stderr: "inherit", env },
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
