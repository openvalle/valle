// Browser versions and downloads belong to the pinned Playwright package.
import { homedir } from "node:os";
import { join } from "node:path";
import { chromium, type Page } from "playwright";

export const root = join(import.meta.dir, "../..");
export const cache = process.env.VALLE_CACHE_DIR ?? join(homedir(), ".valle/cache");

/** Use the full browser's headless mode so WebGPU canvases appear in screenshots. */
export function launch() {
  return chromium.launch({
    channel: "chromium",
    args: [
      "--enable-unsafe-webgpu",
      "--force-color-profile=srgb",
      "--force-device-scale-factor=1",
      "--autoplay-policy=no-user-gesture-required",
    ],
  });
}

/** Playwright's page.evaluate has no timeout; a stalled worker must still fail the test. */
export function evaluate(page: Page, expression: string): Promise<any> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const timeout = new Promise<never>((_, reject) => {
    timer = setTimeout(() => reject(new Error("no answer from the page in 30 s")), 30_000);
  });
  return Promise.race([page.evaluate(expression), timeout]).finally(() => clearTimeout(timer));
}
