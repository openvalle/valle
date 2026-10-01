import { expect, test } from "bun:test";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import type { CanvasKit } from "canvaskit-wasm";
import { initSync, ProductEngine } from "../../../generated/web/valle_engine.js";
import { CanvasKitExecutor, type CanvasKitExecutionReport } from "./executor.ts";

const root = resolve(import.meta.dir, "../../../../../..");
const cli = process.env.VALLE_TEST_CLI ?? join(root, "target/debug/valle");
const { default: CanvasKitInit } = await import("canvaskit-wasm/full") as unknown as {
  default: (options: { locateFile(file: string): string }) => Promise<CanvasKit>;
};

test("CanvasKit RasterProgram cache reuses exact pixels and invalidates on frame, output and surface changes", async () => {
  const CanvasKit = await CanvasKitInit({
    locateFile: () => Bun.resolveSync("canvaskit-wasm/bin/full/canvaskit.wasm", import.meta.dir),
  });
  initSync({ module: await readFile(join(root, "web/packages/engine/generated/web/valle_engine_bg.wasm")) });
  const directory = await mkdtemp(join(tmpdir(), "valle-raster-layer-cache-"));
  const surface = CanvasKit.MakeSurface(48, 48)!;
  let engine: ProductEngine | null = null;
  let executor: CanvasKitExecutor | null = null;
  await writeFile(join(directory, "scene.motion.tsx"), `
      export const composition = { width: 48, height: 48, fps: 30, duration: 3 };
      export default function CacheScene(ctx) {
        return <Scene style={{ width: 48, height: 48, backgroundColor: "#000" }}>
          <View style={{ position: "absolute", left: 4 + ctx.seconds * 8,
            top: 8, width: 16, height: 16,
            backgroundColor: interpolate(ctx.progress, [0, 1], ["#00ff00", "#ff0000"]) }} />
        </Scene>;
      }
    `);
  const server = Bun.spawn([cli, "--json", "motion", "studio", "scene.motion.tsx", "--port", "0",
    "--web-assets-dir", join(root, "web/dist")], {
    cwd: directory, env: { ...process.env, VALLE_HOME: join(directory, "home") },
    stdout: "pipe", stderr: "pipe",
  });
  try {
    const reader = server.stdout.getReader();
    let line = "";
    try {
      while (!line.includes("\n")) {
        const part = await reader.read();
        if (part.done) throw new Error(await new Response(server.stderr).text());
        line += new TextDecoder().decode(part.value);
      }
    } finally { reader.releaseLock(); }
    const ready = JSON.parse(line.split("\n")[0]!);
    const config = await (await fetch(new URL("/config.json", ready.url))).json() as Record<string, string>;
    if (!config.fixedPackageManifestJson) throw new Error(JSON.stringify(config.diagnostics ?? config));
    engine = new ProductEngine();
    const receipt = JSON.parse(engine.open_fixed_package(
      config.fixedPackageManifestJson!, config.timelineJson!, config.resourceManifestJson!,
      config.verifiedBindingBundleJson!,
    ));
    executor = new CanvasKitExecutor(CanvasKit, engine);
    const render = async (
      frame: number, current: CanvasKitExecutor,
      maxFrameBytes = 128n * 1024n * 1024n, transparent = false,
    ): Promise<{
      pixels: Uint8Array; report: CanvasKitExecutionReport;
    }> => {
      const ticket = engine!.evaluate_prepare_preview(receipt.renderId, BigInt(frame), 48, 48, transparent);
      try {
        engine!.lower_canvas_kit(ticket, 64n * 1024n * 1024n, 128n * 1024n * 1024n);
        engine!.bind(ticket, 1n);
        const report = await current.execute(
          engine!.plan_template_bytes(ticket), engine!.binding_bytes(ticket),
          engine!.bound_schedule_bytes(ticket), { generation: 1n, objects: new Map() },
          { surface, maxSurfaceBytes: 64n * 1024n * 1024n, maxFrameBytes },
        );
        const pixels = Uint8Array.from(surface.getCanvas().readPixels(0, 0, {
          width: 48, height: 48, colorType: CanvasKit.ColorType.RGBA_8888,
          alphaType: CanvasKit.AlphaType.Unpremul, colorSpace: CanvasKit.ColorSpace.SRGB,
        })!);
        return { pixels, report };
      } finally { engine!.release_ticket(ticket); }
    };
    const cold = async (frame: number, transparent = false) => {
      const fresh = new CanvasKitExecutor(CanvasKit, engine);
      try { return await render(frame, fresh, 128n * 1024n * 1024n, transparent); }
      finally { fresh.dispose(); }
    };
    const first = await render(0, executor);
    const repeat = await render(0, executor);
    expect(first.report.rasterLayerCacheMisses).toBeGreaterThan(0);
    expect(first.report.rasterLayerCacheEntries).toBeGreaterThan(0);
    expect(repeat.report.rasterLayerCacheHits).toBeGreaterThan(0);
    expect(repeat.pixels).toEqual(first.pixels);
    const moved = await render(30, executor);
    expect(moved.pixels).not.toEqual(first.pixels);
    expect(moved.pixels).toEqual((await cold(30)).pixels);
    expect((await render(0, executor)).pixels).toEqual((await cold(0)).pixels);
    const transparent = await render(0, executor, 128n * 1024n * 1024n, true);
    expect(transparent.report.rasterLayerCacheMisses).toBeGreaterThan(0);
    expect(transparent.pixels).toEqual((await cold(0, true)).pixels);
    const reset = await render(0, executor, 256n * 1024n * 1024n);
    expect(reset.report.rasterLayerCacheHits).toBe(0);
    expect(reset.report.rasterLayerCacheMisses).toBeGreaterThan(0);
    expect(reset.pixels).toEqual(first.pixels);
    expect(reset.report.rasterLayerCacheEntries).toBeLessThanOrEqual(64);
    expect(reset.report.rasterLayerCacheBytes).toBeLessThanOrEqual(64 * 1024 * 1024);
    let afterEviction: CanvasKitExecutionReport | null = null;
    for (let frame = 1; frame <= 70; frame += 1) afterEviction = (await render(frame, executor)).report;
    expect(afterEviction!.rasterLayerCacheEntries).toBe(64);
    const evicted = await render(0, executor);
    expect(evicted.report.rasterLayerCacheMisses).toBeGreaterThan(0);
    expect(evicted.pixels).toEqual((await cold(0)).pixels);
    expect(evicted.report.rasterLayerCacheEntries).toBeLessThanOrEqual(64);
  } finally {
    server.kill();
    await server.exited;
    executor?.dispose();
    engine?.free();
    surface.delete();
    await rm(directory, { recursive: true, force: true });
  }
}, 120_000);
