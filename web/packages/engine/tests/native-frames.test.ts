import { expect, test } from "bun:test";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import type { CanvasKit } from "canvaskit-wasm";
import { renderNativeFrames } from "./native-frames.ts";

test("native frame batches preserve alpha, request order and repeats across sheet rows", async () => {
  const { default: CanvasKitInit } = (await import("canvaskit-wasm")) as unknown as {
    default: (options: { locateFile(file: string): string }) => Promise<CanvasKit>;
  };
  const ck = await CanvasKitInit({
    locateFile: () => Bun.resolveSync("canvaskit-wasm/bin/canvaskit.wasm", import.meta.dir),
  });
  const dir = await mkdtemp(join(tmpdir(), "valle-native-frame-batch-"));
  const cli =
    process.env.VALLE_TEST_CLI ?? resolve(import.meta.dir, "../../../../target/debug/valle");
  try {
    await writeFile(
      join(dir, "scene.motion.tsx"),
      `
      export const composition={width:24,height:16,fps:4,duration:1.25};
      export default function SceneTest(){const time=useCurrentTime();return <Scene style={{width:24,height:16}}>
        <View style={{position:"absolute",left:Math.floor(time.seconds*4),top:2,width:12,height:10,
          backgroundColor:"#ff804080",borderRadius:3}}/>
      </Scene>;}`,
    );
    const frames = [3, 0, 3, 4, 1];
    const batch = await renderNativeFrames(ck, {
      cwd: dir,
      args: ["motion", "render", "scene.motion.tsx", "--backend", "raster"],
      frames,
      output: join(dir, "batch.png"),
      width: 24,
      height: 16,
    });
    expect(batch.report.compilations).toBe(1);
    expect(batch.report.delivery.frameKeys).toEqual(frames);
    expect(batch.frames[0]).toEqual(batch.frames[2]);
    expect(
      batch.frames[0]!.some((value, index) => index % 4 === 3 && value > 0 && value < 255),
    ).toBe(true);
    const info = {
      width: 24,
      height: 16,
      colorType: ck.ColorType.RGBA_8888,
      alphaType: ck.AlphaType.Unpremul,
      colorSpace: ck.ColorSpace.SRGB,
    };
    for (const frame of new Set(frames)) {
      const output = join(dir, `single-${frame}.png`);
      const child = Bun.spawn(
        [
          cli,
          "--json",
          "motion",
          "render",
          "scene.motion.tsx",
          "--frame",
          String(frame),
          "--backend",
          "raster",
          "-o",
          output,
        ],
        {
          cwd: dir,
          env: { ...process.env, VALLE_HOME: join(dir, "home") },
          stdout: "pipe",
          stderr: "pipe",
        },
      );
      const [stdout, stderr, code] = await Promise.all([
        new Response(child.stdout).text(),
        new Response(child.stderr).text(),
        child.exited,
      ]);
      if (code !== 0) throw new Error(`${stdout}\n${stderr}`);
      const image = ck.MakeImageFromEncoded(await readFile(output))!;
      try {
        const pixels = Uint8Array.from(image.readPixels(0, 0, info)!);
        for (const [index, key] of frames.entries())
          if (key === frame) {
            expect(batch.frames[index], `frame ${frame} at request ${index}`).toEqual(pixels);
          }
      } finally {
        image.delete();
      }
    }
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
}, 30_000);
