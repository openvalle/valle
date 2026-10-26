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

test("Studio caches actual import closures and only recompiles affected components", async () => {
  const modules = {
    "a.motion.tsx": `import { size } from './shared'; export const composition={width:64,height:32,fps:4,duration:1}; export default function A(){return <Scene><View style={{width:size,height:10}}/></Scene>;}`,
    "b.motion.tsx": `import { size } from './shared'; export const composition={width:64,height:32,fps:4,duration:1}; export default function B(){return <Scene><View style={{width:size,height:20}}/></Scene>;}`,
    "shared.ts": "export const size=10;",
    "unused.motion.tsx": "not valid TypeScript {{{",
  };
  const request = {
    runtimeAssets,
    runtimeBaseUrl,
    authorTimeline: {
      canvas: { width: 64, height: 32, fps: 4 },
      resources: { a: "a.motion.tsx", b: "b.motion.tsx" },
      tracks: {
        visual: [
          {
            clips: [
              { kind: "motion" as const, component: "a", start: 0, duration: 1 },
              { kind: "motion" as const, component: "b", start: 1, duration: 1 },
            ],
          },
        ],
      },
    },
    instances: ["a", "b"].map((name, index) => ({
      clipPath: `/tracks/visual/0/clips/${index}`,
      entry: `${name}.motion.tsx`,
      modules,
    })),
  };
  const first = await compileStudioPreview({ ...request, id: 100 });
  if (first.status !== "ok") throw new Error(first.message);
  expect(first.timings.compilations).toBe(2);
  modules["a.motion.tsx"] = modules["a.motion.tsx"].replace("height:10", "height:12");
  const edited = await compileStudioPreview({ ...request, id: 101 });
  if (edited.status !== "ok") throw new Error(edited.message);
  expect(edited.timings.compilations).toBe(1);
  expect(edited.instances[1]!.artifactDigest).toBe(first.instances[1]!.artifactDigest);
  modules["unused.motion.tsx"] = "another invalid unused file";
  const unrelated = await compileStudioPreview({ ...request, id: 102 });
  if (unrelated.status !== "ok") throw new Error(unrelated.message);
  expect(unrelated.timings.compilations).toBe(0);
  expect(unrelated.timings.cacheHit).toBe(true);
  modules["shared.ts"] = "export const size=12;";
  const shared = await compileStudioPreview({ ...request, id: 103 });
  if (shared.status !== "ok") throw new Error(shared.message);
  expect(shared.timings.compilations).toBe(2);
});

test("Studio Worker builds an exact one-clip Timeline without retaining unused font inputs", async () => {
  const font = new Uint8Array(
    await readFile(new URL("../../../../assets/fonts/noto/NotoSans-Regular.ttf", import.meta.url)),
  );
  const result = await compileStudioPreview({
    id: 7,
    runtimeAssets,
    runtimeBaseUrl,
    standalone: { input: "/project/card.motion.tsx", fpsOverride: "24/1" },
    instances: [
      {
        clipPath: "/tracks/visual/0/clips/0",
        entry: "card.motion.tsx",
        modules: {
          "card.motion.tsx": `export const composition = { width: 64, height: 64, fps: 30, duration: 3/2 };
        export default function Card() { return <Scene><View style={{ width: 64, height: 64 }} /></Scene>; }`,
        },
        fonts: [
          { bytes: font, role: "font" },
          { bytes: font, role: "formula-font" },
        ],
      },
    ],
  });
  expect(result.status).toBe("ok");
  if (result.status !== "ok") return;
  expect(result.authorTimeline.canvas.fps).toBe("24/1");
  expect(result.authorTimeline.tracks.visual?.[0]?.clips[0]?.duration).toBe(1.5);
  const manifest = JSON.parse(result.package.resourceManifestJson);
  expect(Object.keys(manifest.entries).every((id) => !id.startsWith("font:"))).toBe(true);
});

test("Studio Worker loads font URLs before compiling compile-time text measurement", async () => {
  const fontUrl = new URL("../../../../assets/fonts/noto/NotoSans-Variable.ttf", import.meta.url)
    .href;
  const result = await compileStudioPreview({
    id: 8,
    runtimeAssets,
    runtimeBaseUrl,
    standalone: { input: "/project/measured.motion.tsx" },
    instances: [
      {
        clipPath: "/tracks/visual/0/clips/0",
        entry: "measured.motion.tsx",
        modules: {
          "measured.motion.tsx": `export const composition = { width: 64, height: 64, fps: 30, duration: 1 };
        const WORD = measureText("Valle", { fontSize: 20 });
        export default function Measured() { return <Scene><View style={{ width: WORD.width, height: 8 }} /></Scene>; }`,
        },
        fontUrls: [{ url: fontUrl, role: "font" }],
      },
    ],
  });
  expect(result.status === "ok" ? "ok" : JSON.stringify(result)).toBe("ok");
});

test("covered project fonts avoid downloading defaults; missing glyphs keep fallback", async () => {
  const font = new Uint8Array(
    await readFile(new URL("../../../../assets/fonts/noto/NotoSans-Regular.ttf", import.meta.url)),
  );
  const digest = `sha256:${new Bun.CryptoHasher("sha256").update(font).digest("hex")}`;
  const source = (text: string) => `export const composition={width:64,height:32,fps:4,duration:1};
    export const controls={assets:{brand:asset({kind:"font",required:true})}};
    export default function T(){return <Scene><Text style={{fontFamily:"asset://brand"}}>${text}</Text></Scene>;}`;
  const request = {
    runtimeAssets,
    runtimeBaseUrl,
    standalone: {
      input: "/project/bound-font.motion.tsx",
      assets: [{ name: "brand", path: "brand.ttf", alias: "brand" }],
    },
    resourceInputs: [
      {
        id: "resource:brand",
        entry: { kind: "font" as const, digest, descriptor: { faceIndex: 0, variationAxes: {} } },
        facts: {
          kind: "font" as const,
          descriptor: { faceIndex: 0, variationAxes: {} },
          bytesBase64: font.toBase64(),
        },
      },
    ],
  };
  const instance = {
    clipPath: "/tracks/visual/0/clips/0",
    entry: "bound-font.motion.tsx",
    modules: { "bound-font.motion.tsx": source("Valle") },
    options: {
      resources: [{ control: "brand", contentHash: digest }],
      fontAliases: { "asset://brand": font },
    },
    fontUrls: [{ url: "https://invalid.example/not-needed.ttf", role: "font" as const }],
  };
  const covered = await compileStudioPreview({ ...request, id: 104, instances: [instance] });
  expect(covered.status === "ok" ? "ok" : covered.message).toBe("ok");
  if (covered.status !== "ok") return;
  expect(
    Object.keys(JSON.parse(covered.package.resourceManifestJson).entries).filter((id) =>
      id.startsWith("font:"),
    ),
  ).toEqual([]);
  const missing = await compileStudioPreview({
    ...request,
    id: 105,
    instances: [
      {
        ...instance,
        modules: { "bound-font.motion.tsx": source("中文") },
        fontUrls: [
          {
            url: new URL(
              "../../../../assets/fonts/noto/NotoSansCJKsc-Variable.otf",
              import.meta.url,
            ).href,
            role: "font",
          },
        ],
      },
    ],
  });
  expect(missing.status === "ok" ? "ok" : missing.message).toBe("ok");
  if (missing.status !== "ok") return;
  expect(
    Object.keys(JSON.parse(missing.package.resourceManifestJson).entries).some((id) =>
      id.startsWith("font:"),
    ),
  ).toBe(true);
});

test("Studio Worker retains tangent contact warnings across cached previews", async () => {
  const source = await readFile(
    new URL(
      "../../../../crates/valle-compiler/tests/fixtures/motion/composition/contact-warning.motion.tsx",
      import.meta.url,
    ),
    "utf8",
  );
  const request = {
    runtimeAssets,
    runtimeBaseUrl,
    standalone: { input: "/project/contact-warning.motion.tsx" },
    instances: [
      {
        clipPath: "/tracks/visual/0/clips/0",
        entry: "contact-warning.motion.tsx",
        modules: { "contact-warning.motion.tsx": source },
      },
    ],
  };
  const first = await compileStudioPreview({ id: 8, ...request });
  expect(first.status).toBe("ok");
  if (first.status !== "ok") return;
  expect(first.warnings).toHaveLength(1);
  expect(first.warnings[0]).toMatchObject({
    class: "warning",
    code: "morph-contact",
    sourcePath: "contact-warning.motion.tsx",
  });

  const cached = await compileStudioPreview({ id: 9, ...request });
  expect(cached.status).toBe("ok");
  if (cached.status !== "ok") return;
  expect(cached.timings.cacheHit).toBe(true);
  expect(cached.warnings).toEqual(first.warnings);
});

test("standalone caption Studio uses a caption track and preserves the saved preparation input", async () => {
  const font = new Uint8Array(
    await readFile(new URL("../../../../assets/fonts/noto/NotoSans-Regular.ttf", import.meta.url)),
  );
  const digest = `sha256:${new Bun.CryptoHasher("sha256").update(font).digest("hex")}`;
  const descriptor = { faceIndex: 0, variationAxes: {} };
  const source = `export const composition={width:320,height:180,fps:4,duration:1};
    export const role=captionPresenter({intro:0.25,outro:0.25});
    export default function Caption(props){ const time = useCurrentTime(); const caption = useCaption();return <Scene>{caption.runs.map((run,index)=><Text key={index} style={{fontFamily:caption.style.font,fontSize:caption.style.fontSize,color:time.clipSeconds>=run.start&&time.clipSeconds<run.end?"#facc15":"#ffffff"}}>{run.text}</Text>)}</Scene>;}`;
  const data = {
    text: "Build story",
    runs: [
      { text: "Build ", start: 0, end: 0.5 },
      { text: "story", start: 0.5, end: 1 },
    ],
    style: { font: "asset://caption", fontSize: 28, color: "#ffffff" },
    region: [32, 54, 256, 90],
    align: "bottom-center",
  };
  const instance = {
    clipPath: "/tracks/visual/0/clips/0",
    entry: "caption.motion.tsx",
    modules: { "caption.motion.tsx": source },
    options: {
      data: { source: "caption.json", value: data },
      fontAliases: { "asset://caption": font },
      resources: [{ control: "caption", contentHash: digest }],
    },
    fonts: [],
  };
  const resourceInputs = [
    {
      id: "resource:asset_0",
      entry: { kind: "font", digest, descriptor },
      facts: { kind: "font", descriptor, bytesBase64: font.toBase64() },
    },
  ];
  const request = {
    id: 15,
    runtimeAssets,
    runtimeBaseUrl,
    standalone: {
      input: "/project/caption.motion.tsx",
      data,
      assets: [{ name: "caption", alias: "asset_0", path: "/project/font.ttf" }],
    },
    instances: [instance],
    resourceInputs,
  };
  const result = await compileStudioPreview(request);
  if (result.status !== "ok") throw new Error(result.message);
  expect(result.authorTimeline.tracks.visual).toBeUndefined();
  expect(result.authorTimeline.tracks.caption?.[0]).toMatchObject({
    presenter: { component: "motion" },
    style: { font: "asset_0" },
    layout: { region: [0.1, 0.3, 0.8, 0.5] },
    clips: [{ runs: data.runs }],
  });
  expect(result.authorTimeline.tracks.caption![0]!.presenter).not.toHaveProperty(
    "resources.caption",
  );
  expect(result.instances[0]!.clipPath).toBe("/tracks/caption/0/clips/0");
  expect(result.timings.compilations).toBe(1);
  expect(result.warnings).toEqual([]);
  const reopened = await compileStudioPreview({
    id: 16,
    runtimeAssets,
    runtimeBaseUrl,
    resourceInputs,
    authorTimeline: JSON.parse(JSON.stringify(result.authorTimeline)),
    instances: [{ ...instance, clipPath: result.instances[0]!.clipPath }],
  });
  if (reopened.status !== "ok") throw new Error(reopened.message);
  expect(reopened.instances[0]!.artifactDigest).toBe(result.instances[0]!.artifactDigest);
  expect(reopened.package.timelineJson).toBe(result.package.timelineJson);

  // Conversion uses Rust's q6 layout once. It never previews the old pixel input
  // while saving a differently rounded Timeline input.
  const fractional = { ...data, region: [13, 8, 256, 90] };
  const normalized = await compileStudioPreview({
    ...request,
    id: 17,
    standalone: { ...request.standalone, data: fractional },
    instances: [
      {
        ...instance,
        options: { ...instance.options, data: { source: "caption.json", value: fractional } },
      },
    ],
  });
  if (normalized.status !== "ok") throw new Error(normalized.message);
  expect(normalized.warnings).toContainEqual(
    expect.objectContaining({ code: "caption-input-normalized" }),
  );
  const normalizedReopen = await compileStudioPreview({
    id: 18,
    runtimeAssets,
    runtimeBaseUrl,
    resourceInputs,
    authorTimeline: normalized.authorTimeline,
    instances: [{ ...instance, clipPath: normalized.instances[0]!.clipPath }],
  });
  if (normalizedReopen.status !== "ok") throw new Error(normalizedReopen.message);
  expect(normalizedReopen.instances[0]!.artifactDigest).toBe(
    normalized.instances[0]!.artifactDigest,
  );
  expect(normalizedReopen.package.timelineJson).toBe(normalized.package.timelineJson);
  const repeated = await compileStudioPreview({
    ...request,
    id: 19,
    standalone: { ...request.standalone, data: fractional, props: {} },
    instances: [
      {
        ...instance,
        options: { ...instance.options, data: { source: "caption.json", value: fractional } },
      },
    ],
  });
  if (repeated.status !== "ok") throw new Error(repeated.message);
  expect(repeated.timings.compilations).toBe(0);
  expect(repeated.instances[0]!.artifactDigest).toBe(normalized.instances[0]!.artifactDigest);
  expect(repeated.warnings).toEqual(normalized.warnings);
});

test("Timeline Motion transitions compile at their transition path as transition() templates", async () => {
  const dissolve = `export const composition={width:64,height:32,fps:4,duration:1};
    export const role=transition();
    export default function Dissolve(){ const time = useCurrentTime(); return <Scene className="relative h-full w-full">
      <Image src="input://from" className="absolute inset-0 h-full w-full"/>
      <Image src="input://to" className="absolute inset-0 h-full w-full" style={{opacity:time.clipProgress}}/>
    </Scene>;}`;
  const request = (source: string) =>
    ({
      id: 40,
      runtimeAssets,
      runtimeBaseUrl,
      authorTimeline: {
        canvas: { width: 64, height: 32, fps: 4 },
        resources: { dissolve: "dissolve.motion.tsx" },
        tracks: {
          visual: [
            {
              clips: [
                { kind: "solid", color: "#ff0000", start: 0, duration: 1.5 },
                { kind: "solid", color: "#0000ff", start: 0.5, duration: 1.5 },
              ],
              transitions: [{ from: 0, to: 1, kind: "motion", component: "dissolve" }],
            },
          ],
        },
      },
      instances: [
        {
          clipPath: "/tracks/visual/0/transitions/0",
          entry: "dissolve.motion.tsx",
          modules: { "dissolve.motion.tsx": source },
          fonts: [],
        },
      ],
    }) as Parameters<typeof compileStudioPreview>[0];
  const result = await compileStudioPreview(request(dissolve));
  if (result.status !== "ok") throw new Error(result.message);
  const items = JSON.parse(result.package.timelineJson).document.visual.tracks[0].items;
  expect(items[1].kernel.motion.role).toEqual({ type: "transition" });
  const clip = await compileStudioPreview(
    request(dissolve.replace("export const role=transition();", "")),
  );
  expect(clip.status).toBe("error");
  if (clip.status === "error") expect(clip.message).toContain("transition()");
});

test("Timeline effects compile at their effect and adjustment paths as effect() templates", async () => {
  const invert =
    "const invert=`uniform shader source;\nfloat4 valle_main(float2 uv){float4 c=sample_source(uv);return float4(1.0-c.rgb,c.a);}`;\n" +
    `export const composition={width:64,height:32,fps:4,duration:1};
    export const role=effect();
    export default function Invert(){ const comp = useComposition(); return <Scene>
      <ShaderLayer code={invert} inputs={{source:"input://source"}} style={{width:comp.width,height:comp.height}}/>
    </Scene>;}`;
  const request = (source: string) =>
    ({
      id: 41,
      runtimeAssets,
      runtimeBaseUrl,
      authorTimeline: {
        canvas: { width: 64, height: 32, fps: 4 },
        resources: { invert: "invert.motion.tsx" },
        tracks: {
          visual: [
            {
              clips: [
                {
                  kind: "solid",
                  color: "#ff0000",
                  start: 0,
                  duration: 2,
                  effects: [{ kind: "motion", component: "invert" }],
                },
              ],
            },
          ],
          adjustment: [
            { clips: [{ kind: "motion", component: "invert", start: 0.5, duration: 1 }] },
          ],
        },
      },
      instances: ["/tracks/visual/0/clips/0/effects/0", "/tracks/adjustment/0/clips/0"].map(
        (clipPath) => ({
          clipPath,
          entry: "invert.motion.tsx",
          modules: { "invert.motion.tsx": source },
          fonts: [],
        }),
      ),
    }) as Parameters<typeof compileStudioPreview>[0];
  const result = await compileStudioPreview(request(invert));
  if (result.status !== "ok") throw new Error(result.message);
  const { document } = JSON.parse(result.package.timelineJson);
  expect(document.visual.tracks[0].items[0].layer.effects[0].role).toEqual({ type: "effect" });
  expect(document.adjustments[0].effect.motion.role).toEqual({ type: "effect" });
  const clip = await compileStudioPreview(
    request(invert.replace("export const role=effect();", "")),
  );
  expect(clip.status).toBe("error");
  if (clip.status === "error") expect(clip.message).toContain("effect()");
});

test("300 Timeline captions compile their track's template once and retain exact frame identity", async () => {
  const font = new Uint8Array(
    await readFile(new URL("../../../../assets/fonts/noto/NotoSans-Regular.ttf", import.meta.url)),
  );
  const digest = `sha256:${new Bun.CryptoHasher("sha256").update(font).digest("hex")}`;
  const descriptor = { faceIndex: 0, variationAxes: {} };
  const source = `export const composition={width:160,height:48,fps:4,duration:1};
    export const role=captionPresenter({intro:0.25,outro:0.25});
    export default function Caption(props){ const time = useCurrentTime(); const caption = useCaption();return <Scene><Text key="word" style={{fontFamily:caption.style.font,fontSize:caption.style.fontSize,color:time.clipSeconds<1?"#facc15":"white"}}>{caption.text}</Text></Scene>;}`;
  const clips = Array.from({ length: 300 }, (_, i) => ({
    start: i * 2 + 0.125,
    duration: 2,
    runs: [{ text: `Word ${i % 100}`, start: 0, end: 1.75 }],
  }));
  const request = {
    id: 30,
    runtimeAssets,
    runtimeBaseUrl,
    authorTimeline: {
      canvas: { width: 160, height: 48, fps: 4 },
      resources: { caption: "caption.motion.tsx", font: "font.ttf" },
      tracks: {
        caption: [
          { presenter: { component: "caption" }, style: { font: "font", fontSize: 20 }, clips },
        ],
      },
    },
    instances: clips.map((_, i) => ({
      clipPath: `/tracks/caption/0/clips/${i}`,
      entry: "caption.motion.tsx",
      modules: { "caption.motion.tsx": source },
      options: {
        fontAliases: { "asset://caption": font },
        resources: [{ control: "caption", contentHash: digest }],
      },
      fonts: [],
    })),
    resourceInputs: [
      {
        id: "resource:font",
        entry: { kind: "font", digest, descriptor },
        facts: { kind: "font", descriptor, bytesBase64: font.toBase64() },
      },
    ],
  };
  const before = process.memoryUsage().rss;
  const start = performance.now();
  const result = await compileStudioPreview(request);
  if (result.status !== "ok") throw new Error(result.message);
  expect(result.status).toBe("ok");
  // The captions of a track share its layout; each caption's words are bound as it draws.
  expect(result.timings.compilations).toBe(1);
  expect(new Set(result.instances.map((instance) => instance.artifactDigest)).size).toBe(1);
  const items = JSON.parse(result.package.timelineJson).document.captions.tracks[0].items;
  expect(items[1].type).toBe("motion");
  expect(items[1].runs[0].text).toBe("Word 0");
  console.log(
    JSON.stringify({
      captionBaseline: 300,
      compilations: result.timings.compilations,
      elapsedMs: Math.round(performance.now() - start),
      rssGrowthBytes: process.memoryUsage().rss - before,
    }),
  );
  const { ProductEngine } = await import("../../../packages/engine/generated/web/valle_engine.js");
  const engine = new ProductEngine();
  try {
    const p = result.package;
    const receipt = JSON.parse(
      engine.open_fixed_package(
        p.fixedPackageManifestJson,
        p.timelineJson,
        p.resourceManifestJson,
        p.verifiedBindingBundleJson,
      ),
    );
    const seen = new Map<number, string>();
    for (const frame of [7, 1, 800 + 1, 7, 0, 800 + 1, 1]) {
      const ticket = engine.evaluate_prepare_preview(
        receipt.renderId,
        BigInt(frame),
        160,
        48,
        false,
      );
      try {
        const json = engine.frame_inspection_json(ticket);
        if (seen.has(frame)) expect(json).toBe(seen.get(frame)!);
        seen.set(frame, json);
        // At 4 fps the first admitted sample is 0.25 s; authored start 0.125 s
        // leaves source time 0.125 s, in source frame 0.
        if (frame === 1) expect(JSON.parse(json).motion[0].sourceFrame).toBe(0);
      } finally {
        engine.release_ticket(ticket);
      }
    }
  } finally {
    engine.free();
  }
  // Saving/reopening preserves author words/timing; editing words recompiles nothing.
  const reopened = JSON.parse(JSON.stringify(result.authorTimeline));
  reopened.tracks.caption[0].clips[0].runs[0].text = "Changed";
  const edited = await compileStudioPreview({ ...request, id: 31, authorTimeline: reopened });
  expect(edited.status).toBe("ok");
  if (edited.status !== "ok") throw new Error(edited.message);
  expect(edited.timings.compilations).toBe(0);
  expect(edited.instances[0]!.artifactDigest).toBe(result.instances[0]!.artifactDigest);
  expect(
    JSON.parse(edited.package.timelineJson).document.captions.tracks[0].items[1].runs[0].text,
  ).toBe("Changed");
});

test("Studio diagnoses misplaced roles before compiling host data with author names", async () => {
  for (const [component, track, declaration, expected] of [
    [
      "words",
      "visual",
      "captionPresenter({intro:0,outro:0})",
      "captionPresenter belongs on a caption track",
    ],
    [
      "title",
      "caption",
      "overlay({intro:0,outro:0,hold:'once'})",
      "a caption track presenter must declare captionPresenter",
    ],
  ] as const) {
    const source = `export const composition={width:64,height:32,duration:1}; export const role=${declaration};
      export default function Template(props){ const data = useData();return <Scene/>;}`;
    const clipPath = `/tracks/${track}/0/clips/0`;
    const tracks =
      track === "visual"
        ? { visual: [{ clips: [{ kind: "motion" as const, component, start: 0, duration: 1 }] }] }
        : {
            caption: [
              {
                presenter: { component },
                style: { font: "font", fontSize: 20 },
                clips: [{ start: 0, duration: 1, runs: [{ text: "word", start: 0, end: 1 }] }],
              },
            ],
          };
    const result = await compileStudioPreview({
      id: 50,
      runtimeAssets,
      runtimeBaseUrl,
      authorTimeline: {
        canvas: { width: 64, height: 32, fps: 4 },
        resources: { [component]: "template.motion.tsx", font: "font.ttf" },
        tracks,
      },
      instances: [
        {
          clipPath,
          entry: "template.motion.tsx",
          modules: { "template.motion.tsx": source },
          fonts: [],
        },
      ],
    });
    expect(result.status).toBe("error");
    if (result.status !== "error") throw new Error("misplaced role accepted");
    expect(result.message).toContain(`component \`${component}\``);
    expect(result.message).toContain(clipPath);
    expect(result.message).toContain(expected);
    expect(result.message).not.toContain("needs caption data");
    expect(result.message).not.toContain("binding is not declared");
  }
});
