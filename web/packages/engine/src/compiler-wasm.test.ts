import { expect, test } from "bun:test";
import { readFile } from "node:fs/promises";
import { initSync, compile_motion_jsx, compile_motion_modules } from "../generated/web/valle_engine.js";
import {
  compileMotionJsxWithWasm, compileMotionModulesWithWasm, MotionCompileError,
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
