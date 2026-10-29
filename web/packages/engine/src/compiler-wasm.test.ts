import { expect, test } from "bun:test";
import { readFile } from "node:fs/promises";
import {
  initSync,
  compile_motion_jsx,
  compile_motion_modules,
  prepare_preview_package,
  rewrite_motion_source,
  ProductEngine,
} from "../generated/web/valle_engine.js";
import {
  compileMotionJsxWithWasm,
  compileMotionModulesWithWasm,
  preparePreviewPackageWithWasm,
  rewriteMotionSourceWithWasm,
  MotionCompileError,
  type MotionRole,
} from "./compiler.ts";

initSync({
  module: await readFile(new URL("../generated/web/valle_engine_bg.wasm", import.meta.url)),
});

test("Wasm reuses closed expressions while random seeks keep current frame inputs", () => {
  const compiled = compileMotionJsxWithWasm(
    { compile_motion_jsx },
    `
    export const composition={width:64,height:32,fps:4,duration:1};
    const width=(t)=>t*20+10;
    export default function Main(){ const time = useCurrentTime();return <Scene>
      <View key="a" className="absolute" style={{left:0,top:0,width:width(time.seconds),height:10}}/>
      <View key="b" className="absolute" style={{left:32,top:0,width:width(time.seconds),height:10}}/>
    </Scene>;}`,
  );
  const nodes = compiled.artifact.nodes as Array<{
    key: string;
    styles: Array<{ property: string; value: { expr?: number } }>;
  }>;
  const ids = ["a", "b"].map(
    (key) =>
      nodes.find((node) => node.key === key)!.styles.find((style) => style.property === "width")!
        .value.expr,
  );
  expect(ids[0]).toBeDefined();
  expect(ids[1]).toBe(ids[0]);
  const preview = preparePreviewPackageWithWasm(
    { prepare_preview_package },
    {
      authorTimeline: {
        canvas: { width: 64, height: 32, fps: 4 },
        resources: { main: "main.motion.tsx" },
        tracks: {
          visual: [{ clips: [{ kind: "motion", component: "main", start: 0, duration: 1 }] }],
        },
      },
      motionInstances: [
        {
          clipPath: "/tracks/visual/0/clips/0",
          artifact: compiled.artifact,
          artifactDigest: compiled.artifactDigest,
          fonts: [],
        },
      ],
    },
  );
  const engine = new ProductEngine();
  try {
    const receipt = JSON.parse(
      engine.open_fixed_package(
        preview.fixedPackageManifestJson,
        preview.timelineJson,
        preview.resourceManifestJson,
        preview.verifiedBindingBundleJson,
        preview.resourceBytes,
      ),
    );
    for (const frame of [3, 0, 2, 2, 1, 3, 0]) {
      const ticket = engine.evaluate_prepare_preview(
        receipt.renderId,
        BigInt(frame),
        64,
        32,
        false,
      );
      try {
        const boxes = JSON.parse(engine.frame_inspection_json(ticket)).motion[0].boxes;
        // The last frame shows the end of the second.
        const seconds = frame === 3 ? 1 : frame / 4;
        expect(boxes.a).toEqual([0, 0, 10 + seconds * 20, 10]);
        expect(boxes.b).toEqual([32, 0, 10 + seconds * 20, 10]);
      } finally {
        engine.release_ticket(ticket);
      }
    }
  } finally {
    engine.free();
  }
});

test("prepared inspection keeps visible Scene3D placement through nested transforms and random seeks", () => {
  const compiled = compileMotionJsxWithWasm(
    { compile_motion_jsx },
    `
    export const composition={width:64,height:32,fps:4,duration:1};
    export default function Main(){ const time = useCurrentTime();return <Scene>
      <View key="moving" className="absolute" style={{left:time.seconds*8,top:0,width:64,height:32}}>
        <Scene3D key="stage" style={{position:"absolute",left:4,top:3,width:16,height:16}}
          camera={{position:[0,0,3],target:[0,0,0],fov:45}}>
          <Mesh key="visible-mesh" geometry={extrude(path("M -1 -1 L 1 -1 L 1 1 L -1 1 Z"),{depth:0.5})}
            material={{type:"unlit",color:"white"}}/>
        </Scene3D>
      </View>
      <View style={{opacity:0}}><Scene3D key="hidden" style={{width:16,height:16}}
        camera={{position:[0,0,3],target:[0,0,0],fov:45}}>
        <Mesh key="hidden-mesh" geometry={extrude(path("M -1 -1 L 1 -1 L 1 1 Z"),{depth:0.5})}/>
      </Scene3D></View>
    </Scene>;}`,
  );
  const preview = preparePreviewPackageWithWasm(
    { prepare_preview_package },
    {
      authorTimeline: {
        canvas: { width: 64, height: 32, fps: 4 },
        resources: { main: "main.motion.tsx" },
        tracks: {
          visual: [{ clips: [{ kind: "motion", component: "main", start: 0, duration: 1 }] }],
        },
      },
      motionInstances: [
        {
          clipPath: "/tracks/visual/0/clips/0",
          artifact: compiled.artifact,
          artifactDigest: compiled.artifactDigest,
          fonts: [],
        },
      ],
    },
  );
  const engine = new ProductEngine();
  try {
    const receipt = JSON.parse(
      engine.open_fixed_package(
        preview.fixedPackageManifestJson,
        preview.timelineJson,
        preview.resourceManifestJson,
        preview.verifiedBindingBundleJson,
        preview.resourceBytes,
      ),
    );
    const seen = new Map<number, string>();
    const transforms = new Map<number, number[]>();
    for (const frame of [3, 0, 2, 2, 1, 3, 0]) {
      const ticket = engine.evaluate_prepare_preview(
        receipt.renderId,
        BigInt(frame),
        64,
        32,
        false,
      );
      try {
        const json = engine.frame_inspection_json(ticket),
          inspection = JSON.parse(json);
        expect(inspection.renderId).toBe(receipt.renderId);
        expect(inspection.compositionFrame).toBe(frame);
        expect(inspection.scene3d).toHaveLength(1);
        const placement = inspection.scene3d[0];
        expect(placement.clipId).toBe(inspection.motion[0].clipId);
        expect(placement.contentHash).toMatch(/^sha256:[a-f0-9]{64}$/);
        expect(placement.deviceFromLocal.every(Number.isFinite)).toBe(true);
        transforms.set(frame, placement.deviceFromLocal);
        if (seen.has(frame)) expect(json).toBe(seen.get(frame)!);
        seen.set(frame, json);
      } finally {
        engine.release_ticket(ticket);
      }
    }
    // The last frame shows the end of the second: 8 px to the right of the first.
    expect(transforms.get(3)![2]! - transforms.get(0)![2]!).toBeCloseTo(8, 6);
  } finally {
    engine.free();
  }
});

test("Wasm shares bounded Circle preparation with Echo and resets it on package replacement", async () => {
  const source = await readFile(
    new URL("../../../tests/performance/fixtures/echo-circles.motion.tsx", import.meta.url),
    "utf8",
  );
  const compiled = compileMotionJsxWithWasm({ compile_motion_jsx }, source);
  const preview = preparePreviewPackageWithWasm(
    { prepare_preview_package },
    {
      authorTimeline: {
        canvas: { width: 640, height: 360, fps: 60 },
        resources: { main: "main.motion.tsx" },
        tracks: {
          visual: [{ clips: [{ kind: "motion", component: "main", start: 0, duration: 10 }] }],
        },
      },
      motionInstances: [
        {
          clipPath: "/tracks/visual/0/clips/0",
          artifact: compiled.artifact,
          artifactDigest: compiled.artifactDigest,
          fonts: [],
        },
      ],
    },
  );
  const engine = new ProductEngine();
  const open = () =>
    JSON.parse(
      engine.open_fixed_package(
        preview.fixedPackageManifestJson,
        preview.timelineJson,
        preview.resourceManifestJson,
        preview.verifiedBindingBundleJson,
        preview.resourceBytes,
      ),
    ).renderId as string;
  try {
    const id = open();
    const stats = () => JSON.parse(engine.motion_instance_cache_stats_json(id))[0][1];
    expect(stats().entries).toBe(0);
    for (const frame of [...Array.from({ length: 42 }, (_, i) => i), 41, 0, 30, 30, 59]) {
      const ticket = engine.evaluate_prepare_preview(id, BigInt(frame), 640, 360, false);
      engine.release_ticket(ticket);
      const current = stats();
      expect(current.entries).toBeLessThanOrEqual(current.max_entries);
      expect(current.resident_bytes).toBeLessThanOrEqual(current.max_bytes);
    }
    expect(stats().hits).toBeGreaterThan(0);
    expect(stats().evictions).toBeGreaterThan(0);
    const replacement = open();
    expect(JSON.parse(engine.motion_instance_cache_stats_json(replacement))[0][1].entries).toBe(0);
  } finally {
    engine.free();
  }
});

test("Wasm keeps host endpoints and geometry moving while the source holds", () => {
  const compiled = compileMotionJsxWithWasm(
    { compile_motion_jsx },
    `
    export const composition={width:64,height:32,fps:4,duration:1};
    export default function Host() { const time = useCurrentTime(); const comp = useComposition();return <Scene><View key="bar" className="absolute"
      style={{left:time.clipProgress*40,top:time.seconds*8,width:comp.clipDuration*6,
      height:12,backgroundColor:"white"}}/></Scene>;}`,
  );
  const preview = preparePreviewPackageWithWasm(
    { prepare_preview_package },
    {
      authorTimeline: {
        canvas: { width: 64, height: 32, fps: 4 },
        resources: { host: "host.motion.tsx" },
        tracks: {
          visual: [
            { clips: [{ kind: "motion", component: "host", start: 0, duration: 2, end: "hold" }] },
          ],
        },
      },
      motionInstances: [
        {
          clipPath: "/tracks/visual/0/clips/0",
          artifact: compiled.artifact,
          artifactDigest: compiled.artifactDigest,
          fonts: [],
        },
      ],
    },
  );
  const engine = new ProductEngine();
  try {
    const receipt = JSON.parse(
      engine.open_fixed_package(
        preview.fixedPackageManifestJson,
        preview.timelineJson,
        preview.resourceManifestJson,
        preview.verifiedBindingBundleJson,
        preview.resourceBytes,
      ),
    );
    const seen = new Map<number, string>();
    for (const frame of [7, 0, 3, 7, 1, 0]) {
      const ticket = engine.evaluate_prepare_preview(
        receipt.renderId,
        BigInt(frame),
        64,
        32,
        false,
      );
      try {
        const json = engine.frame_inspection_json(ticket);
        const inspection = JSON.parse(json).motion[0];
        expect(inspection.sourceFrame).toBe(Math.min(frame, 3));
        // From the source's last frame on, the scene shows its end.
        // Layout positions retain their f32 subpixels; only the host viewport is rounded.
        expect(inspection.boxes.bar).toEqual([
          Math.fround((frame * 40) / 7),
          frame >= 3 ? 8 : frame * 2,
          12,
          12,
        ]);
        if (seen.has(frame)) expect(json).toBe(seen.get(frame)!);
        seen.set(frame, json);
      } finally {
        engine.release_ticket(ticket);
      }
    }
  } finally {
    engine.free();
  }
});

test("Wasm maps overlay timing and rounds the host viewport through the shared engine", () => {
  for (const hold of ["once", "loop", "stretch"]) {
    const compiled = compileMotionJsxWithWasm(
      { compile_motion_jsx },
      `
      export const composition={width:64,height:32,fps:4,duration:2};
      export const role=overlay({intro:0.5,outro:0.5,hold:"${hold}"});
      export default function Main(){ const time = useCurrentTime(); const comp = useComposition();return <Scene><View key="bar" className="absolute"
        style={{left:time.seconds*12,top:time.clipProgress*20,width:comp.width/2,
        height:comp.height,backgroundColor:"white"}}/></Scene>;}`,
    );
    expect(compiled.artifact.role).toEqual({ type: "overlay", intro: "1/2", outro: "1/2", hold });
    for (const duration of [1, 1.5, 4]) {
      const author = {
        canvas: { width: 64, height: 32, fps: 4 },
        resources: { card: "card.motion.tsx" },
        tracks: {
          visual: [
            {
              clips: [
                {
                  kind: "motion" as const,
                  component: "card",
                  start: 0,
                  duration,
                  size: [31.5, 63.49] as [number, number],
                },
              ],
            },
          ],
        },
      };
      const prepare = (timeline: typeof author) =>
        preparePreviewPackageWithWasm(
          { prepare_preview_package },
          {
            authorTimeline: timeline,
            motionInstances: [
              {
                clipPath: "/tracks/visual/0/clips/0",
                artifact: compiled.artifact,
                artifactDigest: compiled.artifactDigest,
                fonts: [],
              },
            ],
          },
        );
      const preview = prepare(author);
      expect(preview.motionSourceMetadata.card).toEqual({
        duration: "2/1",
        role: compiled.artifact.role as MotionRole,
      });
      expect(() =>
        prepare({
          ...author,
          tracks: {
            visual: [{ clips: [{ ...author.tracks.visual[0]!.clips[0]!, fit: "contain" } as any] }],
          },
        }),
      ).toThrow("fit");
      expect(() =>
        prepare({
          ...author,
          tracks: {
            visual: [{ clips: [{ ...author.tracks.visual[0]!.clips[0]!, duration: 0.999 }] }],
          },
        }),
      ).toThrow("shorter");
      const engine = new ProductEngine();
      try {
        const receipt = JSON.parse(
          engine.open_fixed_package(
            preview.fixedPackageManifestJson,
            preview.timelineJson,
            preview.resourceManifestJson,
            preview.verifiedBindingBundleJson,
            preview.resourceBytes,
          ),
        );
        const frames = duration * 4,
          seen = new Map<number, string>();
        for (const frame of [frames - 1, 0, frames / 2, frames - 1, 1, 0]) {
          const ticket = engine.evaluate_prepare_preview(
            receipt.renderId,
            BigInt(frame),
            64,
            32,
            false,
          );
          try {
            const json = engine.frame_inspection_json(ticket),
              inspection = JSON.parse(json).motion[0];
            const t = frame / 4,
              exit = duration - 0.5;
            // The last frame of the outro shows the template's end.
            const source =
              frame === frames - 1
                ? 2
                : t < 0.5
                  ? t
                  : t >= exit
                    ? 1.5 + t - exit
                    : 0.5 +
                      (hold === "once"
                        ? Math.min(t - 0.5, 1)
                        : hold === "loop"
                          ? (t - 0.5) % 1
                          : (t - 0.5) / (duration - 1));
            expect(inspection.sourceFrame).toBe(Math.min(Math.floor(source * 4), 7));
            expect(inspection.boxes.bar).toEqual([
              Math.fround(source * 12),
              Math.fround((frame / (frames - 1)) * 20),
              16,
              63,
            ]);
            if (seen.has(frame)) expect(json).toBe(seen.get(frame)!);
            seen.set(frame, json);
          } finally {
            engine.release_ticket(ticket);
          }
        }
      } finally {
        engine.free();
      }
    }
  }
});

test("merged Engine Wasm compiles standalone JSX and linked modules", () => {
  const source = `export const composition = { width: 320, height: 180, duration: 1 };
    export default function Card() { const time = useCurrentTime(); return <Scene><View style={{ opacity: time.progress }} /></Scene>; }`;
  const standalone = compileMotionJsxWithWasm({ compile_motion_jsx }, source);
  expect(standalone.artifact.component).toBe("Card");
  expect(standalone.artifactDigest).toMatch(/^sha256:[a-f0-9]{64}$/);
  expect((standalone.artifact.composition as { width: number }).width).toBe(320);

  const linked = compileMotionModulesWithWasm({ compile_motion_modules }, "card.motion.tsx", {
    "card.motion.tsx": `export const composition = { width: 320, height: 180, duration: 1 };
      import { size } from './size';
      export default function Card() { return <Scene><View style={{ width: size }} /></Scene>; }`,
    "size.ts": "export const size = 64;",
  });
  expect(linked.artifact.component).toBe("Card");
  expect((linked.sourceMap.modules as unknown[]).length).toBe(2);
});

test("merged Engine Wasm freezes supplied PCM for audio-driven Motion", () => {
  const source = `export const composition={width:64,height:64,fps:30,duration:1};
    export const controls={assets:{beat:asset({kind:"audio",required:true})}};
    const BEAT=audioAnalysis("asset://beat",{bands:4,rate:30});
    export default function Bars(){ const time = useCurrentTime();return <Scene><View style={{width:10,
      height:BEAT.level(time.seconds)*50,backgroundColor:"#fff"}}/></Scene>}`;
  const samples = new Float32Array(8000);
  for (let index = 4000; index < samples.length; index++) {
    samples[index] = Math.sin((Math.PI * 2 * 440 * (index - 4000)) / 8000);
  }
  const hash = `sha256:${"a".repeat(64)}`;
  const options = {
    resources: [{ control: "beat", contentHash: hash }],
    audioSources: [{ control: "beat", contentHash: hash, sampleRate: 8000, samples }],
  };
  const compiled = compileMotionJsxWithWasm({ compile_motion_jsx }, source, options);
  const table = (
    compiled.artifact.exprs as Array<{ kind: string; signal?: { values?: number[] } }>
  ).find((expr) => expr.kind === "audioSample")?.signal?.values;
  expect(table).toBeTruthy();
  expect(table![8]).toBe(0);
  expect(table![23]).toBeGreaterThan(0.6);
  expect(() =>
    compileMotionJsxWithWasm({ compile_motion_jsx }, source, { resources: options.resources }),
  ).toThrow(MotionCompileError);
});

test("Wasm shares rounded radial bars and prepared audio-band indices through random seeks", async () => {
  const source = await readFile(
    new URL(
      "../../../../crates/valle-compiler/tests/fixtures/motion/composition/radial-bars.motion.tsx",
      import.meta.url,
    ),
    "utf8",
  );
  const compiled = compileMotionJsxWithWasm({ compile_motion_jsx }, source);
  expect(compiled.artifact.nodes).toHaveLength(2);
  expect(compiled.artifact.instanceGroups).toHaveLength(1);
  const preview = preparePreviewPackageWithWasm(
    { prepare_preview_package },
    {
      authorTimeline: {
        canvas: { width: 640, height: 360, fps: 30 },
        resources: { main: "radial.motion.tsx" },
        tracks: {
          visual: [{ clips: [{ kind: "motion", component: "main", start: 0, duration: 2 }] }],
        },
      },
      motionInstances: [
        {
          clipPath: "/tracks/visual/0/clips/0",
          artifact: compiled.artifact,
          artifactDigest: compiled.artifactDigest,
          fonts: [],
        },
      ],
    },
  );
  const engine = new ProductEngine();
  try {
    const receipt = JSON.parse(
      engine.open_fixed_package(
        preview.fixedPackageManifestJson,
        preview.timelineJson,
        preview.resourceManifestJson,
        preview.verifiedBindingBundleJson,
        preview.resourceBytes,
      ),
    );
    const seen = new Map<number, string>();
    for (const frame of [0, 7, 30, 59, 7, 0]) {
      const ticket = engine.evaluate_prepare_preview(
        receipt.renderId,
        BigInt(frame),
        640,
        360,
        false,
      );
      try {
        const boxes = JSON.parse(engine.frame_inspection_json(ticket)).motion[0].boxes;
        expect(Object.keys(boxes).filter((key) => key.startsWith("ray-"))).toHaveLength(72);
        const previous = seen.get(frame);
        if (previous !== undefined) expect(JSON.stringify(boxes)).toBe(previous);
        else seen.set(frame, JSON.stringify(boxes));
      } finally {
        engine.release_ticket(ticket);
      }
    }
  } finally {
    engine.free();
  }
  const hash = `sha256:${"a".repeat(64)}`;
  const audioSource = source
    .replace(
      "const rays",
      "export const controls={assets:{beat:asset({kind:'audio',required:true})}};\nconst BEAT=audioAnalysis('asset://beat',{bands:12,rate:30});\nconst rays",
    )
    .replace("(Math.sin(i * 0.73 + t * 3) + 1) * 28", "BEAT.band(i % 12, t) * 56");
  const audio = compileMotionJsxWithWasm({ compile_motion_jsx }, audioSource, {
    resources: [{ control: "beat", contentHash: hash }],
    audioSources: [
      { control: "beat", contentHash: hash, sampleRate: 8000, samples: new Float32Array(16000) },
    ],
  });
  expect(audio.artifact.instanceGroups).toHaveLength(1);
  expect(audio.warnings.some((warning) => warning.code === "instance-fallback")).toBe(false);
});

test("pulse audio analysis produces the Native artifact digest in WASM", async () => {
  const source = await readFile(
    new URL(
      "../../../../crates/valle-compiler/tests/fixtures/motion/composition/audio-reactive-pulses.motion.tsx",
      import.meta.url,
    ),
    "utf8",
  );
  const samples = new Float32Array(8000 * 2);
  for (const pulse of [2000, 6000, 10000, 14000]) samples.fill(0.8, pulse, pulse + 128);
  const hash = `sha256:${"a".repeat(64)}`;
  const compiled = compileMotionJsxWithWasm({ compile_motion_jsx }, source, {
    resources: [{ control: "beat", contentHash: hash }],
    audioSources: [{ control: "beat", contentHash: hash, sampleRate: 8000, samples }],
  });
  expect(compiled.artifactDigest).toBe(
    "sha256:7429daa056b0be9052e51b669f63b614d37e2a56c36e84b6fec4f7e937d9afcb",
  );
});

test("WASM admits shared staggered stops and a local frame-time path paint", () => {
  const source = `export const composition={width:64,height:64,fps:30,duration:1};
    export default function Demo() { const time = useCurrentTime();
      const paint=linearGradient(point(0,0),point(64,0),[
        gradientStop(0,interpolate(time.seconds,[0,1],["red","blue"])),
        gradientStop(1,"green")]);
      return <Scene><Path d="M 0 0 L 64 0 L 64 64 Z" fill={paint}/>
        <Text split="char" perUnit={(unit) => ({opacity:interpolate(time.seconds,
          [unit.index*0.1,1+unit.index*0.1],[0,1])})}>AB</Text></Scene>;
    }`;
  const compiled = compileMotionJsxWithWasm({ compile_motion_jsx }, source);
  expect(
    (compiled.artifact.exprs as Array<{ kind: string }>).some((expr) => expr.kind === "sub"),
  ).toBe(true);
  expect(JSON.stringify(compiled.artifact.nodes)).toContain('"kind":"linear"');
});

test("Motion tangent contacts remain warnings in standalone and linked Web compiles", async () => {
  const source = await readFile(
    new URL(
      "../../../../crates/valle-compiler/tests/fixtures/motion/composition/contact-warning.motion.tsx",
      import.meta.url,
    ),
    "utf8",
  );
  const standalone = compileMotionJsxWithWasm({ compile_motion_jsx }, source);
  expect(standalone.warnings).toHaveLength(1);
  expect(standalone.warnings[0]?.class).toBe("warning");
  expect(standalone.warnings[0]?.code).toBe("morph-contact");
  expect(standalone.warnings[0]?.message).toContain("0.500000000000");

  const entry = "contact.motion.tsx";
  const linked = compileMotionModulesWithWasm({ compile_motion_modules }, entry, {
    [entry]: source,
  });
  expect(linked.warnings).toHaveLength(1);
  expect(linked.warnings[0]?.sourcePath).toBe(entry);
  expect(() =>
    compileMotionJsxWithWasm(
      { compile_motion_jsx },
      source.replace('contactPolicy: "warn"', 'contactPolicy: "error"'),
    ),
  ).toThrow(MotionCompileError);
});

test("static map expansion reports its instance fallback reason in Web compiles", () => {
  const source = `export const composition={width:64,height:32,fps:30,duration:1};
    const items=[{id:"a",x:0},{id:"b",x:20}];
    export default function Grid(){return <Scene>{items.map(item=><View key={item.id}
      className="absolute " style={{left:item.x,top:0,width:10,height:10,backgroundColor:"#fff"}}/>)}</Scene>}`;
  expect(compileMotionJsxWithWasm({ compile_motion_jsx }, source).warnings).toHaveLength(0);
  const large = source.replace(
    'const items=[{id:"a",x:0},{id:"b",x:20}];',
    "const items=Array.from({length:64},(_,i)=>({id:`item${i}`,x:i*20}));",
  );
  const standalone = compileMotionJsxWithWasm({ compile_motion_jsx }, large);
  expect(standalone.warnings).toHaveLength(1);
  expect(standalone.warnings[0]?.code).toBe("instance-fallback");
  expect(standalone.warnings[0]?.message).toContain('exact className="absolute"');
  const entry = "grid.motion.tsx";
  const linked = compileMotionModulesWithWasm({ compile_motion_modules }, entry, {
    [entry]: large,
  });
  expect(linked.warnings[0]?.sourcePath).toBe(entry);
});

test("merged Engine Wasm uses host font bytes for measureText and reports source diagnostics", async () => {
  const font = new Uint8Array(
    await readFile(new URL("../../../../assets/fonts/noto/NotoSans-Regular.ttf", import.meta.url)),
  );
  const source = `export const composition = { width: 320, height: 180, duration: 1 };
    const measured = measureText('Hello', { fontSize: 24 });
    export default function Card() { return <Scene><View style={{ width: measured.width }} /></Scene>; }`;
  const compiled = compileMotionJsxWithWasm({ compile_motion_jsx }, source, { fonts: [font] });
  expect(compiled.artifact.component).toBe("Card");

  try {
    compileMotionModulesWithWasm({ compile_motion_modules }, "card.motion.tsx", {
      "card.motion.tsx": "export default function Card() { return <Scene><Missing /></Scene>; }",
    });
    throw new Error("expected compiler diagnostics");
  } catch (error) {
    expect(error).toBeInstanceOf(MotionCompileError);
    expect((error as MotionCompileError).diagnostics[0]?.sourcePath).toBe("card.motion.tsx");
  }
});

test("merged Engine Wasm admits a browser-compiled Motion as a fixed Timeline package", () => {
  const entry = "card.motion.tsx";
  const compiled = compileMotionModulesWithWasm({ compile_motion_modules }, entry, {
    [entry]: `export const composition = { width: 64, height: 64, fps: 30, duration: 1 };
      export default function Card() { return <Scene><View style={{ width: 64, height: 64, backgroundColor: '#2563eb' }} /></Scene>; }`,
  });
  const input = {
    authorTimeline: {
      canvas: { width: 64, height: 64, fps: 30 },
      resources: { card: entry },
      tracks: {
        visual: [
          { clips: [{ kind: "motion" as const, component: "card", start: 0, duration: 1 }] },
        ],
      },
    },
    motionInstances: [
      {
        clipPath: "/tracks/visual/0/clips/0",
        artifact: compiled.artifact,
        artifactDigest: compiled.artifactDigest,
        fonts: [],
      },
    ],
  };
  const preview = preparePreviewPackageWithWasm({ prepare_preview_package }, input);
  expect(JSON.parse(preview.fixedPackageManifestJson).format).toBe("valle.fixed-render-package@1");
  expect(JSON.parse(preview.timelineJson).document.canvas.width).toBe(64);
  expect(preview.externalResources).toEqual([]);
  const engine = new ProductEngine();
  const receipt = JSON.parse(
    engine.open_fixed_package(
      preview.fixedPackageManifestJson,
      preview.timelineJson,
      preview.resourceManifestJson,
      preview.verifiedBindingBundleJson,
      preview.resourceBytes,
    ),
  );
  const ticket = engine.evaluate_prepare_preview(receipt.renderId, 0n, 64, 64, false);
  expect(JSON.parse(engine.frame_inspection_json(ticket))).toBeTruthy();
  expect(() =>
    preparePreviewPackageWithWasm(
      { prepare_preview_package },
      {
        ...input,
        motionInstances: [],
      },
    ),
  ).toThrow(/missing/);
});

test("merged Engine Wasm rewrites a current declaration while preserving surrounding Unicode", () => {
  const source =
    "// 标题 😀\nexport const controls=defineControls({props:{title:string({default:'旧标题'})}});";
  const edited = rewriteMotionSourceWithWasm(
    { rewrite_motion_source },
    {
      source,
      target: { kind: "prop-default", name: "title" },
      value: "新标题",
    },
  );
  expect(edited.source).toBe(
    '// 标题 😀\nexport const controls=defineControls({props:{title:string({default:"新标题"})}});',
  );
  expect(edited.replacedSpan[0]).toBeGreaterThan(source.indexOf("旧标题"));
  expect(() =>
    rewriteMotionSourceWithWasm(
      { rewrite_motion_source },
      {
        source: "const title='x'; export const controls=title;",
        target: { kind: "prop-default", name: "title" },
        value: "new",
      },
    ),
  ).toThrow(/computed/);
});

test("Wasm compiles the same closed caption role and frozen input as Native", async () => {
  const fixture = new URL(
    "../../../../crates/valle-compiler/tests/fixtures/motion/caption/",
    import.meta.url,
  );
  const source = await readFile(new URL("timed-word-bars.motion.tsx", fixture), "utf8");
  const data = JSON.parse(await readFile(new URL("timed-word-bars.data.json", fixture), "utf8"));
  const compiled = compileMotionJsxWithWasm({ compile_motion_jsx }, source, {
    data: { source: "captions.json", value: data },
  });
  expect(compiled.artifact.role).toEqual({
    type: "captionPresenter",
    intro: "3/20",
    outro: "3/20",
  });
  expect(compiled.artifact.controls).toEqual({
    props: {},
    data: {},
    assets: { caption: { kind: "font", required: true } },
  });
  expect(compiled.artifactDigest).toBe(
    "sha256:0af16c3a9138ad1ce1dc80d20aa8d38f0bfe30e27486634d1f04e6cd993a14a0",
  );
  const movedOrigin = compileMotionJsxWithWasm({ compile_motion_jsx }, source, {
    data: { source: "elsewhere.json", value: data },
  });
  expect(movedOrigin.artifactDigest).toBe(compiled.artifactDigest);
  expect(movedOrigin.preparedDataDigest).toBe(compiled.preparedDataDigest);
  const changed = compileMotionJsxWithWasm({ compile_motion_jsx }, source, {
    data: {
      source: "captions.json",
      value: { ...data, runs: [data.runs[0], { ...data.runs[1], end: 1.4 }] },
    },
  });
  expect(changed.preparedDataDigest).not.toBe(compiled.preparedDataDigest);
  expect(() => compileMotionJsxWithWasm({ compile_motion_jsx }, source)).toThrow("caption data");
  expect(() =>
    compileMotionJsxWithWasm({ compile_motion_jsx }, source, {
      data: { source: "captions.json", value: { ...data, unknown: true } },
    }),
  ).toThrow("unknown");
  expect(() =>
    compileMotionJsxWithWasm({ compile_motion_jsx }, source, {
      data: {
        source: "captions.json",
        value: { ...data, runs: [data.runs[0], { ...data.runs[1], start: 0.4000004 }] },
      },
    }),
  ).toThrow("normalized");
  expect(() =>
    compileMotionJsxWithWasm({ compile_motion_jsx }, source, {
      data: {
        source: "captions.json",
        value: {
          ...data,
          text: "",
          runs: Array.from({ length: 9500 }, () => ({
            text: "",
            start: 0,
            end: 1,
            fontSize: 24,
            color: "#ffffff",
          })),
        },
      },
    }),
  ).toThrow("value budget");
  expect(() =>
    compileMotionJsxWithWasm(
      { compile_motion_jsx },
      source.replace("export const role", "export const controls={data:{}}; export const role"),
      { data: { source: "captions.json", value: data } },
    ),
  ).toThrow("controls.data");
});
