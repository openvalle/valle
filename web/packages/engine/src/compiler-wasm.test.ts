import { expect, test } from "bun:test";
import { readFile } from "node:fs/promises";
import { initSync, compile_motion_jsx, compile_motion_modules, prepare_preview_package, rewrite_motion_source, ProductEngine } from "../generated/web/valle_engine.js";
import {
  compileMotionJsxWithWasm, compileMotionModulesWithWasm, preparePreviewPackageWithWasm,
  rewriteMotionSourceWithWasm, MotionCompileError,
} from "./compiler.ts";

initSync({ module: await readFile(new URL("../generated/web/valle_engine_bg.wasm", import.meta.url)) });

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
