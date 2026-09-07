import { describe, expect, test } from "bun:test";
import path from "node:path";

import { APP_BUILDS } from "./runtime-entrypoints.ts";

const root = path.resolve(import.meta.dir, "..");

describe("app boundaries", () => {
  test("keep Timeline and Motion inside the single Studio app", async () => {
    const [timeline, timelineRuntime, motion] = await Promise.all([
      Bun.file(path.join(root, "apps/studio/src/timeline-workspace.ts")).text(),
      Bun.file(path.join(root, "apps/studio/src/timeline-workspace-runtime.ts")).text(),
      Bun.file(path.join(root, "apps/studio/src/motion-workspace.ts")).text(),
    ]);
    expect(timeline).toContain('from "@valle/player"');
    expect(timelineRuntime).toContain("runtimeAssets: config.runtimeAssets");
    expect(motion).toContain('from "@valle/player"');
    expect(motion).toContain("startMotionStudio");
  });

  test("routes demo and console through their app-local entries", async () => {
    const [demoHtml, demo, consoleHtml, consoleApp] = await Promise.all([
      Bun.file(path.join(root, "apps/preview/index.html")).text(),
      Bun.file(path.join(root, "apps/preview/src/host.ts")).text(),
      Bun.file(path.join(root, "apps/console/index.html")).text(),
      Bun.file(path.join(root, "apps/console/src/console-app.ts")).text(),
    ]);
    expect(demoHtml).toContain('src="./src/main.ts"');
    expect(demo).toContain('from "@valle/player"');
    expect(demo).toContain("runtimeAssets: config.runtimeAssets");
    expect(consoleHtml).toContain('src="./src/main.ts"');
    expect(consoleHtml).toContain("<valle-console-app>");
    expect(consoleApp).toContain('from "lit"');
    expect(`${demoHtml}\n${demo}\n${consoleHtml}\n${consoleApp}`).not.toMatch(/["'`]\/(?:wasm|vendor|src)\//);
  });

  test("publishes Motion only as a workspace in the single Studio app", () => {
    expect(APP_BUILDS.filter(([name]) => name.includes("studio"))).toEqual([
      ["studio", "apps/studio/index.html"],
    ]);
    expect(APP_BUILDS.some(([, entry]) => entry.includes("motion"))).toBeFalse();
  });
});
