import { describe, expect, test } from "bun:test";
import path from "node:path";

import { APP_BUILDS } from "./runtime-entrypoints.ts";

const root = path.resolve(import.meta.dir, "..");

describe("app boundaries", () => {
  test("keep Timeline and Motion inside the single Studio app", async () => {
    const [timeline, timelineRuntime, main] = await Promise.all([
      Bun.file(path.join(root, "apps/studio/src/timeline-workspace.ts")).text(),
      Bun.file(path.join(root, "apps/studio/src/timeline-workspace-runtime.ts")).text(),
      Bun.file(path.join(root, "apps/studio/src/main.ts")).text(),
    ]);
    expect(timeline).toContain('from "valle-engine/element"');
    expect(timelineRuntime).toContain("runtimeAssets: config.runtimeAssets");
    expect(main).toContain("startTimelineStudio");
    expect(main).toContain("StandaloneTimelineSession");
  });

  test("keeps Preview in the test fixture and removes Console", async () => {
    expect(await Bun.file(path.join(root,"tests/preview/index.html")).text()).toContain('src="./src/main.ts"');
    expect(await Bun.file(path.join(root,"apps/console/index.html")).exists()).toBeFalse();
    expect(APP_BUILDS).toEqual([["studio","apps/studio/index.html"]]);
  });

  test("publishes one Studio app for all entry types", () => {
    expect(APP_BUILDS.filter(([name]) => name.includes("studio"))).toEqual([
      ["studio", "apps/studio/index.html"],
    ]);
    expect(APP_BUILDS.some(([, entry]) => entry.includes("motion"))).toBeFalse();
  });
});
