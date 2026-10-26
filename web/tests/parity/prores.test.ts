import { expect, test } from "bun:test";
import { decodeVideoFramesInBrowser, resolveBrowser } from "./lib/browser.ts";

const browser = resolveBrowser();
if (
  !browser &&
  (process.env.VALLE_WEB_PLAYER_REQUIRE_BROWSER || process.env.VALLE_WEB_PARITY_REQUIRE_BROWSER)
) {
  throw new Error("ProRes acceptance requires a browser");
}

// A moving 50% orange rounded box, an opaque green box and a transparent background.
// Native ProRes 4444, 128×72, 12 fps; no fonts or external assets.
const fixture = Bun.fileURLToPath(new URL("../fixtures/prores-alpha.mov", import.meta.url));
(browser ? test : test.skip)(
  "ProRes keeps alpha, source timestamps and repeat seeks through HTTP ranges",
  async () => {
    const times = [0, 0.5, 1 / 12, 11 / 12, 0.5];
    const result = await decodeVideoFramesInBrowser(browser!, fixture, times, {
      sourceType: "url",
    });
    for (const [index, frame] of result.frames.entries()) {
      expect([frame.width, frame.height]).toEqual([128, 72]);
      expect(Math.abs(frame.ptsS - times[index]!)).toBeLessThan(0.00001);
      const pixel = (x: number, y: number) =>
        frame.data.slice((y * frame.width + x) * 4, (y * frame.width + x) * 4 + 4);
      expect(pixel(0, 0)[3]).toBe(0);
      const translucent = pixel(Math.floor(8 + times[index]! * 36 + 16), 24);
      expect(Math.abs(translucent[3]! - 128)).toBeLessThanOrEqual(1);
      expect(translucent[0]!).toBeGreaterThan(translucent[1]!);
      const opaque = pixel(104, 36);
      expect(opaque[3]).toBe(255);
      expect(opaque[1]!).toBeGreaterThan(opaque[0]!);
    }
    expect(result.frames[1]!.data).toEqual(result.frames[4]!.data);
    expect(result.frames[0]!.data).not.toEqual(result.frames[3]!.data);
  },
  60_000,
);
