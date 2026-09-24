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
