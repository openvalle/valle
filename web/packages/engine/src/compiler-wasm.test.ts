import { expect, test } from "bun:test";
import { readFile } from "node:fs/promises";
import { initSync, compile_motion_jsx, compile_motion_modules, prepare_preview_package, rewrite_motion_source, ProductEngine } from "../generated/web/valle_engine.js";
import {
  compileMotionJsxWithWasm, compileMotionModulesWithWasm, preparePreviewPackageWithWasm,
  rewriteMotionSourceWithWasm, MotionCompileError,
} from "./compiler.ts";

initSync({ module: await readFile(new URL("../generated/web/valle_engine_bg.wasm", import.meta.url)) });

test("Wasm keeps host endpoints and geometry moving while the source holds", () => {
  const compiled = compileMotionJsxWithWasm({ compile_motion_jsx }, `
    export const composition={width:64,height:32,fps:4,duration:1};
    export default function Host(ctx) {return <Scene><View key="bar" className="absolute"
      style={{left:ctx.host.progress*40,top:ctx.seconds*8,width:ctx.host.duration*6,
      height:12,backgroundColor:"white"}}/></Scene>;}`);
  const preview = preparePreviewPackageWithWasm({ prepare_preview_package }, {
    authorTimeline: { canvas:{width:64,height:32,fps:4}, resources:{host:"host.motion.tsx"},
      tracks:{visual:[{clips:[{kind:"motion",component:"host",start:0,duration:2,end:"hold"}]}]} },
    motionInstances:[{clipPath:"/tracks/visual/0/clips/0",artifact:compiled.artifact,
      artifactDigest:compiled.artifactDigest,fonts:[]}],
  });
  const engine = new ProductEngine();
  try {
    const receipt = JSON.parse(engine.open_fixed_package(preview.fixedPackageManifestJson,
      preview.timelineJson,preview.resourceManifestJson,preview.verifiedBindingBundleJson));
    const seen = new Map<number, string>();
    for (const frame of [7,0,3,7,1,0]) {
      const ticket = engine.evaluate_prepare_preview(receipt.renderId,BigInt(frame),64,32,false);
      try {
        const json = engine.frame_inspection_json(ticket);
        const inspection = JSON.parse(json).motion[0];
        expect(inspection.sourceFrame).toBe(Math.min(frame,3));
        expect(inspection.boxes.bar).toEqual([Math.round(frame*40/7),Math.min(frame,3)*2,12,12]);
        if (seen.has(frame)) expect(json).toBe(seen.get(frame)!);
        seen.set(frame,json);
      } finally { engine.release_ticket(ticket); }
    }
  } finally { engine.free(); }
});

test("merged Engine Wasm compiles standalone JSX and linked modules", () => {
  const source = `export const composition = { width: 320, height: 180, duration: 1 };
    export default function Card(ctx) { return <Scene><View style={{ opacity: ctx.progress }} /></Scene>; }`;
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
    const BEAT=audioAnalysis("asset://beat",{bands:4,fps:30});
    export default function Bars(ctx){return <Scene><View style={{width:10,
      height:BEAT.level(ctx.seconds)*50,backgroundColor:"#fff"}}/></Scene>}`;
  const samples = new Float32Array(8000);
  for (let index = 4000; index < samples.length; index++) {
    samples[index] = Math.sin(Math.PI * 2 * 440 * (index - 4000) / 8000);
  }
  const hash = `sha256:${"a".repeat(64)}`;
  const options = { resources: [{ control: "beat", contentHash: hash }],
    audioSources: [{ control: "beat", contentHash: hash, sampleRate: 8000, samples }] };
  const compiled = compileMotionJsxWithWasm({ compile_motion_jsx }, source, options);
  const table = (compiled.artifact.exprs as Array<{ kind: string; samples?: number[] }>)
    .find((expr) => expr.kind === "audioSample")?.samples;
  expect(table).toBeTruthy();
  expect(table![8]).toBe(0);
  expect(table![23]).toBeGreaterThan(0.6);
  expect(() => compileMotionJsxWithWasm({ compile_motion_jsx }, source,
    { resources: options.resources })).toThrow(MotionCompileError);
});

test("pulse audio analysis produces the Native artifact digest in WASM", async () => {
  const source = await readFile(new URL("../../../../crates/valle-compiler/tests/fixtures/motion/composition/audio-reactive-pulses.motion.tsx", import.meta.url), "utf8");
  const samples = new Float32Array(8000 * 2);
  for (const pulse of [2000, 6000, 10000, 14000]) samples.fill(0.8, pulse, pulse + 128);
  const hash = `sha256:${"a".repeat(64)}`;
  const compiled = compileMotionJsxWithWasm({ compile_motion_jsx }, source, {
    resources: [{ control: "beat", contentHash: hash }],
    audioSources: [{ control: "beat", contentHash: hash, sampleRate: 8000, samples }],
  });
  expect(compiled.artifactDigest)
    .toBe("sha256:c64ba4b51605ffb3ddf7303056cbc8dd6b7847e3229924469f112652d9850a09");
});

test("WASM admits shared staggered stops and a local frame-time path paint", () => {
  const source = `export const composition={width:64,height:64,fps:30,duration:1};
    export default function Demo(ctx) {
      const paint=linearGradient(point(0,0),point(64,0),[
        gradientStop(0,interpolate(ctx.seconds,[0,1],["red","blue"])),
        gradientStop(1,"green")]);
      return <Scene><Path d="M 0 0 L 64 0 L 64 64 Z" fill={paint}/>
        <Text split="char" perUnit={{opacity:interpolate(ctx.seconds,
          [ctx.unit.index*0.1,1+ctx.unit.index*0.1],[0,1])}}>AB</Text></Scene>;
    }`;
  const compiled = compileMotionJsxWithWasm({ compile_motion_jsx }, source);
  expect((compiled.artifact.exprs as Array<{ kind: string }>).some((expr) => expr.kind === "sub")).toBe(true);
  expect(JSON.stringify(compiled.artifact.nodes)).toContain('"kind":"linear"');
});

test("Motion tangent contacts remain warnings in standalone and linked Web compiles", async () => {
  const source = await readFile(new URL("../../../../crates/valle-compiler/tests/fixtures/motion/composition/contact-warning.motion.tsx", import.meta.url), "utf8");
  const standalone = compileMotionJsxWithWasm({ compile_motion_jsx }, source);
  expect(standalone.warnings).toHaveLength(1);
  expect(standalone.warnings[0]?.class).toBe("warning");
  expect(standalone.warnings[0]?.code).toBe("morph-contact");
  expect(standalone.warnings[0]?.message).toContain("0.500000000000");

  const entry = "contact.motion.tsx";
  const linked = compileMotionModulesWithWasm({ compile_motion_modules }, entry, { [entry]: source });
  expect(linked.warnings).toHaveLength(1);
  expect(linked.warnings[0]?.sourcePath).toBe(entry);
  expect(() => compileMotionJsxWithWasm({ compile_motion_jsx },
    source.replace('contactPolicy: "warn"', 'contactPolicy: "error"'))).toThrow(MotionCompileError);
});

test("static map expansion reports its instance fallback reason in Web compiles", () => {
  const source = `export const composition={width:64,height:32,fps:30,duration:1};
    const items=[{id:"a",x:0},{id:"b",x:20}];
    export default function Grid(){return <Scene>{items.map(item=><View key={item.id}
      className="absolute " style={{left:item.x,top:0,width:10,height:10,backgroundColor:"#fff"}}/>)}</Scene>}`;
  expect(compileMotionJsxWithWasm({ compile_motion_jsx }, source).warnings).toHaveLength(0);
  const large = source.replace('const items=[{id:"a",x:0},{id:"b",x:20}];',
    'const items=Array.from({length:64},(_,i)=>({id:`item${i}`,x:i*20}));');
  const standalone = compileMotionJsxWithWasm({ compile_motion_jsx }, large);
  expect(standalone.warnings).toHaveLength(1);
  expect(standalone.warnings[0]?.code).toBe("instance-fallback");
  expect(standalone.warnings[0]?.message).toContain('exact className="absolute"');
  const entry = "grid.motion.tsx";
  const linked = compileMotionModulesWithWasm({ compile_motion_modules }, entry, { [entry]: large });
  expect(linked.warnings[0]?.sourcePath).toBe(entry);
});

test("merged Engine Wasm uses host font bytes for measureText and reports source diagnostics", async () => {
  const font = new Uint8Array(await readFile(new URL("../../../../assets/fonts/noto/NotoSans-Regular.ttf", import.meta.url)));
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
      canvas: { width: 64, height: 64, fps: 30 }, resources: { card: entry },
      tracks: { visual: [{ clips: [{ kind: "motion" as const, component: "card", start: 0, duration: 1 }] }] },
    },
    motionInstances: [{ clipPath: "/tracks/visual/0/clips/0", artifact: compiled.artifact,
      artifactDigest: compiled.artifactDigest, fonts: [] }],
  };
  const preview = preparePreviewPackageWithWasm({ prepare_preview_package }, input);
  expect(JSON.parse(preview.fixedPackageManifestJson).format).toBe("valle.fixed-render-package@1");
  expect(JSON.parse(preview.timelineJson).document.canvas.width).toBe(64);
  expect(preview.externalResources).toEqual([]);
  const engine = new ProductEngine();
  const receipt = JSON.parse(engine.open_fixed_package(
    preview.fixedPackageManifestJson, preview.timelineJson,
    preview.resourceManifestJson, preview.verifiedBindingBundleJson,
  ));
  const ticket = engine.evaluate_prepare_preview(receipt.renderId, 0n, 64, 64, false);
  expect(JSON.parse(engine.frame_inspection_json(ticket))).toBeTruthy();
  expect(() => preparePreviewPackageWithWasm({ prepare_preview_package }, {
    ...input, motionInstances: [],
  })).toThrow(/missing/);
});

test("merged Engine Wasm rewrites a current declaration while preserving surrounding Unicode", () => {
  const source = "// 标题 😀\nexport const controls=defineControls({props:{title:string({default:'旧标题'})}});";
  const edited = rewriteMotionSourceWithWasm({ rewrite_motion_source }, {
    source, target: { kind: "prop-default", name: "title" }, value: "新标题",
  });
  expect(edited.source).toBe("// 标题 😀\nexport const controls=defineControls({props:{title:string({default:\"新标题\"})}});");
  expect(edited.replacedSpan[0]).toBeGreaterThan(source.indexOf("旧标题"));
  expect(() => rewriteMotionSourceWithWasm({ rewrite_motion_source }, {
    source: "const title='x'; export const controls=title;",
    target: { kind: "prop-default", name: "title" }, value: "new",
  })).toThrow(/computed/);
});
