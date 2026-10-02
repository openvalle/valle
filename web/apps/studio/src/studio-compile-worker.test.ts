import { expect, test } from "bun:test";
import { readFile } from "node:fs/promises";
import { compileStudioPreview } from "./studio-compile-worker.ts";

const runtimeBaseUrl = new URL("../../../", import.meta.url).href;
const runtimeAssets = {
  engine: {
    glue: "packages/engine/generated/web/valle_engine.js",
    wasm: "packages/engine/generated/web/valle_engine_bg.wasm",
  },
};

test("Studio Worker builds an exact one-clip Timeline without retaining unused font inputs", async () => {
  const font = new Uint8Array(await readFile(new URL("../../../../assets/fonts/noto/NotoSans-Regular.ttf", import.meta.url)));
  const result = await compileStudioPreview({
    id: 7, runtimeAssets, runtimeBaseUrl,
    standalone: { input: "/project/card.motion.tsx", fpsOverride: "24/1" },
    instances: [{
      clipPath: "/tracks/visual/0/clips/0", entry: "card.motion.tsx",
      modules: { "card.motion.tsx": `export const composition = { width: 64, height: 64, fps: 30, duration: 3/2 };
        export default function Card() { return <Scene><View style={{ width: 64, height: 64 }} /></Scene>; }` },
      fonts: [{ bytes: font, role: "font" }, { bytes: font, role: "formula-font" }],
    }],
  });
  expect(result.status).toBe("ok");
  if (result.status !== "ok") return;
  expect(result.authorTimeline.canvas.fps).toBe("24/1");
  expect(result.authorTimeline.tracks.visual?.[0]?.clips[0]?.duration).toBe(1.5);
  const manifest = JSON.parse(result.package.resourceManifestJson);
  expect(Object.keys(manifest.entries).every((id) => !id.startsWith("font:"))).toBe(true);
});

test("Studio Worker retains tangent contact warnings across cached previews", async () => {
  const source = await readFile(new URL("../../../../crates/valle-compiler/tests/fixtures/motion/composition/contact-warning.motion.tsx", import.meta.url), "utf8");
  const request = {
    runtimeAssets, runtimeBaseUrl,
    standalone: { input: "/project/contact-warning.motion.tsx" },
    instances: [{
      clipPath: "/tracks/visual/0/clips/0", entry: "contact-warning.motion.tsx",
      modules: { "contact-warning.motion.tsx": source },
    }],
  };
  const first = await compileStudioPreview({ id: 8, ...request });
  expect(first.status).toBe("ok");
  if (first.status !== "ok") return;
  expect(first.warnings).toHaveLength(1);
  expect(first.warnings[0]).toMatchObject({
    class: "warning", code: "morph-contact", sourcePath: "contact-warning.motion.tsx",
  });

  const cached = await compileStudioPreview({ id: 9, ...request });
  expect(cached.status).toBe("ok");
  if (cached.status !== "ok") return;
  expect(cached.timings.cacheHit).toBe(true);
  expect(cached.warnings).toEqual(first.warnings);
});

test("standalone caption Studio uses a caption track and preserves the saved preparation input", async () => {
  const font = new Uint8Array(await readFile(new URL("../../../../assets/fonts/noto/NotoSans-Regular.ttf", import.meta.url)));
  const digest = `sha256:${new Bun.CryptoHasher("sha256").update(font).digest("hex")}`;
  const descriptor = { faceIndex: 0, variationAxes: {} };
  const source = `export const composition={width:320,height:180,fps:4,duration:1};
    export const role=captionPresenter({intro:seconds(0.25),outro:seconds(0.25)});
    export default function Caption(ctx,props,data){return <Scene>{data.runs.map((run,index)=><Text key={index} style={{fontFamily:data.style.font,fontSize:data.style.fontSize,color:ctx.host.seconds>=run.start&&ctx.host.seconds<run.end?"#facc15":"#ffffff"}}>{run.text}</Text>)}</Scene>;}`;
  const data = { text: "Build story", runs: [{ text: "Build ", start: 0, end: 0.5 }, { text: "story", start: 0.5, end: 1 }],
    style: { font: "asset://caption", fontSize: 28, color: "#ffffff" }, region: [32, 54, 256, 90], align: "bottom-center" };
  const instance = { clipPath: "/tracks/visual/0/clips/0", entry: "caption.motion.tsx", modules: { "caption.motion.tsx": source },
    options: { data: { source: "caption.json", value: data },
      fontAliases: { "asset://caption": font }, resources: [{ control: "caption", contentHash: digest }] }, fonts: [] };
  const resourceInputs = [{ id: "resource:asset_0", entry: { kind: "font", digest, descriptor },
    facts: { kind: "font", descriptor, bytesBase64: Buffer.from(font).toString("base64") } }];
  const request = { id: 15, runtimeAssets, runtimeBaseUrl,
    standalone: { input: "/project/caption.motion.tsx", data,
      assets: [{ name: "caption", alias: "asset_0", path: "/project/font.ttf" }] },
    instances: [instance], resourceInputs };
  const result = await compileStudioPreview(request);
  if (result.status !== "ok") throw new Error(result.message);
  expect(result.authorTimeline.tracks.visual).toBeUndefined();
  expect(result.authorTimeline.tracks.caption?.[0]).toMatchObject({
    presenter: { component: "motion" }, style: { font: "asset_0" },
    layout: { region: [0.1, 0.3, 0.8, 0.5] }, clips: [{ runs: data.runs }],
  });
  expect(result.authorTimeline.tracks.caption![0]!.presenter).not.toHaveProperty("resources.caption");
  expect(result.instances[0]!.clipPath).toBe("/tracks/caption/0/clips/0");
  expect(result.timings.compilations).toBe(1);
  expect(result.warnings).toEqual([]);
  const reopened = await compileStudioPreview({ id: 16, runtimeAssets, runtimeBaseUrl, resourceInputs,
    authorTimeline: JSON.parse(JSON.stringify(result.authorTimeline)),
    instances: [{ ...instance, clipPath: result.instances[0]!.clipPath }] });
  if (reopened.status !== "ok") throw new Error(reopened.message);
  expect(reopened.instances[0]!.artifactDigest).toBe(result.instances[0]!.artifactDigest);
  expect(reopened.package.timelineJson).toBe(result.package.timelineJson);

  // Conversion uses Rust's q6 layout once. It never previews the old pixel input
  // while saving a differently rounded Timeline input.
  const fractional = { ...data, region: [13, 8, 256, 90] };
  const normalized = await compileStudioPreview({ ...request, id: 17,
    standalone: { ...request.standalone, data: fractional },
    instances: [{ ...instance, options: { ...instance.options, data: { source: "caption.json", value: fractional } } }] });
  if (normalized.status !== "ok") throw new Error(normalized.message);
  expect(normalized.warnings).toContainEqual(expect.objectContaining({ code: "caption-input-normalized" }));
  const normalizedReopen = await compileStudioPreview({ id: 18, runtimeAssets, runtimeBaseUrl, resourceInputs,
    authorTimeline: normalized.authorTimeline, instances: [{ ...instance, clipPath: normalized.instances[0]!.clipPath }] });
  if (normalizedReopen.status !== "ok") throw new Error(normalizedReopen.message);
  expect(normalizedReopen.instances[0]!.artifactDigest).toBe(normalized.instances[0]!.artifactDigest);
  expect(normalizedReopen.package.timelineJson).toBe(normalized.package.timelineJson);
  const repeated = await compileStudioPreview({ ...request, id: 19,
    standalone: { ...request.standalone, data: fractional, props: {} },
    instances: [{ ...instance, options: { ...instance.options, data: { source: "caption.json", value: fractional } } }] });
  if (repeated.status !== "ok") throw new Error(repeated.message);
  expect(repeated.timings.compilations).toBe(0);
  expect(repeated.instances[0]!.artifactDigest).toBe(normalized.instances[0]!.artifactDigest);
  expect(repeated.warnings).toEqual(normalized.warnings);
});

test("300 Timeline captions compile each immutable word input once and retain exact frame identity", async () => {
  const font = new Uint8Array(await readFile(new URL("../../../../assets/fonts/noto/NotoSans-Regular.ttf", import.meta.url)));
  const digest = `sha256:${new Bun.CryptoHasher("sha256").update(font).digest("hex")}`;
  const descriptor = { faceIndex: 0, variationAxes: {} };
  const source = `export const composition={width:160,height:48,fps:4,duration:1};
    export const role=captionPresenter({intro:seconds(0.25),outro:seconds(0.25)});
    export default function Caption(ctx,props,data){return <Scene><Text key="word" style={{fontFamily:data.style.font,fontSize:data.style.fontSize,color:ctx.host.seconds<1?"#facc15":"white"}}>{data.text}</Text></Scene>;}`;
  const clips = Array.from({ length: 300 }, (_, i) => ({ start: i * 2 + 0.125, duration: 2,
    runs: [{ text: `Word ${i % 100}`, start: 0, end: 1.75 }] }));
  const request = {
    id: 30, runtimeAssets, runtimeBaseUrl,
    authorTimeline: { canvas: { width: 160, height: 48, fps: 4 }, resources: { caption: "caption.motion.tsx", font: "font.ttf" },
      tracks: { caption: [{ presenter: { component: "caption" }, style: { font: "font", fontSize: 20 }, clips }] } },
    instances: clips.map((_, i) => ({ clipPath: `/tracks/caption/0/clips/${i}`, entry: "caption.motion.tsx", modules: { "caption.motion.tsx": source },
      options: { fontAliases: { "asset://caption": font }, resources: [{ control: "caption", contentHash: digest }] }, fonts: [] })),
    resourceInputs: [{ id: "resource:font", entry: { kind: "font", digest, descriptor }, facts: { kind: "font", descriptor, bytesBase64: Buffer.from(font).toString("base64") } }],
  };
  const before = process.memoryUsage().rss;
  const start = performance.now();
  const result = await compileStudioPreview(request);
  if (result.status !== "ok") throw new Error(result.message);
  expect(result.status).toBe("ok");
  expect(result.timings.compilations).toBe(100);
  expect(new Set(result.instances.map(instance => instance.artifactDigest)).size).toBe(100);
  expect(result.instances[0]!.artifactDigest).toBe(result.instances[100]!.artifactDigest);
  expect(JSON.parse(result.package.timelineJson).document.captions.tracks[0].items[1].type).toBe("motion");
  console.log(JSON.stringify({ captionBaseline: 300, immutableInputs: 100, compilations: result.timings.compilations,
    elapsedMs: Math.round(performance.now() - start), rssGrowthBytes: process.memoryUsage().rss - before }));
  const { ProductEngine } = await import("../../../packages/engine/generated/web/valle_engine.js");
  const engine = new ProductEngine();
  try {
    const p = result.package;
    const receipt = JSON.parse(engine.open_fixed_package(p.fixedPackageManifestJson, p.timelineJson, p.resourceManifestJson, p.verifiedBindingBundleJson));
    const seen = new Map<number, string>();
    for (const frame of [7, 1, 800 + 1, 7, 0, 800 + 1, 1]) {
      const ticket = engine.evaluate_prepare_preview(receipt.renderId, BigInt(frame), 160, 48, false);
      try {
        const json = engine.frame_inspection_json(ticket);
        if (seen.has(frame)) expect(json).toBe(seen.get(frame)!);
        seen.set(frame, json);
        if (frame === 1) expect(JSON.parse(json).motion[0].sourceFrame).toBe(0);
      } finally { engine.release_ticket(ticket); }
    }
  } finally { engine.free(); }
  // Saving/reopening preserves author words/timing; editing words changes preparation identity.
  const reopened = JSON.parse(JSON.stringify(result.authorTimeline));
  reopened.tracks.caption[0].clips[0].runs[0].text = "Changed";
  const edited = await compileStudioPreview({ ...request, id: 31, authorTimeline: reopened });
  expect(edited.status).toBe("ok");
  if (edited.status !== "ok") throw new Error(edited.message);
  expect(edited.instances[0]!.artifactDigest).not.toBe(result.instances[0]!.artifactDigest);
  expect(edited.instances[100]!.artifactDigest).toBe(result.instances[100]!.artifactDigest);
});
