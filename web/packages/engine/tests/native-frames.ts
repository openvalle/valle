import { readFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import type { CanvasKit } from "canvaskit-wasm";

const cli =
  process.env.VALLE_TEST_CLI ?? resolve(import.meta.dir, "../../../../target/debug/valle");

type FrameBatchReport = {
  compilations: number;
  delivery: { backend: string; frameKeys: number[] };
};

// Contact sheets retain request order and duplicates, and copy full-size RGBA cells verbatim.
// One process can therefore compile once and exercise cold, warm and random-access frames.
export async function renderNativeFrames(
  ck: CanvasKit,
  options: {
    cwd: string;
    args: string[];
    frames: readonly number[];
    output: string;
    width: number;
    height: number;
  },
): Promise<{ frames: Uint8Array<ArrayBuffer>[]; report: FrameBatchReport }> {
  const { cwd, args, frames, output, width, height } = options;
  if (frames.length === 0) throw new Error("Native frame batch must not be empty");
  const child = Bun.spawn(
    [cli, "--json", ...args, "--frames", frames.join(","), "--storyboard", output],
    {
      cwd,
      env: { ...process.env, VALLE_HOME: join(cwd, "home") },
      stdout: "pipe",
      stderr: "pipe",
    },
  );
  const [stdout, stderr, code] = await Promise.all([
    new Response(child.stdout).text(),
    new Response(child.stderr).text(),
    child.exited,
  ]);
  if (code !== 0) throw new Error(`Native frame batch exited ${code}: ${stdout}\n${stderr}`);
  const report = JSON.parse(stdout) as FrameBatchReport;
  if (JSON.stringify(report.delivery.frameKeys) !== JSON.stringify(frames)) {
    throw new Error(`Native frame batch changed request order: ${stdout}`);
  }
  const sheet = ck.MakeImageFromEncoded(await readFile(output));
  if (!sheet) throw new Error(`Cannot decode native frame batch: ${output}`);
  try {
    const columns = Math.min(frames.length, 4);
    if (
      sheet.width() !== width * columns ||
      sheet.height() !== height * Math.ceil(frames.length / columns)
    ) {
      throw new Error(`Native frame batch has unexpected dimensions: ${output}`);
    }
    const info = {
      width,
      height,
      colorType: ck.ColorType.RGBA_8888,
      alphaType: ck.AlphaType.Unpremul,
      colorSpace: ck.ColorSpace.SRGB,
    };
    return {
      report,
      frames: frames.map((_, index) => {
        const pixels = sheet.readPixels(
          (index % columns) * width,
          Math.floor(index / columns) * height,
          info,
        );
        if (!pixels) throw new Error(`Cannot read native frame batch cell ${index}: ${output}`);
        return Uint8Array.from(pixels);
      }),
    };
  } finally {
    sheet.delete();
  }
}
