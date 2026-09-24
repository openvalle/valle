import { expect, test } from "bun:test";
import { standaloneAssetsFromMotionContext } from "./standalone-assets.ts";
import type { MotionContext } from "./host.ts";

test("frozen Motion image facts become a safe author alias and matching locator", () => {
  const entry = { kind: "image", digest: "sha256:test", descriptor: { width: 8, height: 8 } };
  const facts = { kind: "image", descriptor: entry.descriptor,
    temporalFootprint: { pastFrames: 0, futureFrames: 0 } };
  const context = {
    status: "ok",
    resourceManifest: { entries: { "asset:poster": entry } },
    verifiedBindingBundleJson: JSON.stringify({ bindings: {
      "asset:poster": { facts, dependencies: [] },
    } }),
    resourceLocators: [{ id: "asset:poster", url: "/preview-assets/test" }],
  } as unknown as MotionContext;
  const result = standaloneAssetsFromMotionContext(["poster=poster.png"], context);
  expect(result.bindings).toEqual([{ name: "poster", path: "poster.png", alias: "asset_0" }]);
  expect(result.resourceInputs).toEqual([{ id: "resource:asset_0", entry, facts }]);
  expect(result.locators).toEqual([{ id: "resource:asset_0", url: "/preview-assets/test" }]);
  expect(result.options.resources).toEqual([{ control: "poster", contentHash: "sha256:test" }]);
});
