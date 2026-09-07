import { expect, test } from "bun:test";

import { buildMotionPreview, type GoodMotionContext } from "./motion-preview.ts";

const fixedTimeline = {
  document: {},
} as GoodMotionContext["timeline"];

const context = {
  status: "ok",
  protocolVersion: 1,
  generation: 2,
  input: "card.motion.tsx",
  artifactDigest: `sha256:${"a".repeat(64)}`,
  artifact: {
    formatVersion: 1,
    component: "Card",
    controls: {
      props: {},
      data: {},
      timing: { enterFrames: {}, holdCycleFrames: {}, exitFrames: {} },
      cues: {},
      assets: {},
      camera: { values: {} },
    },
  },
  preparedData: {},
  dataSource: null,
  timing: { enterFrames: 4, exitFrames: 5 },
  cueBindings: {},
  sourceMap: { version: 1, component: "Card", entry: "component.tsx", closureDigest: `sha256:${"c".repeat(64)}`, modules: [], nodes: [], exprs: [], objects: [] },
  assets: [{ name: "cover", kind: "image", url: "/motion-assets/cover" }],
  resourceLocators: [{ id: "asset:cover", url: "/motion-assets/cover" }],
  shaders: [],
  durationFrames: 60,
  fps: { num: 30, den: 1 },
  viewport: { width: 640, height: 360 },
  diagnostics: [],
  runtimeBaseUrl: "/",
  runtimeAssets: {},
  fixedPackageManifestJson: "producer-owned-fixed-package-manifest",
  timelineJson: "producer-owned-canonical-json",
  timeline: fixedTimeline,
  resourceManifestJson: "producer-owned-resource-manifest",
  resourceManifest: {} as GoodMotionContext["resourceManifest"],
  verifiedBindingBundleJson: "producer-owned-verified-bundle",
} satisfies GoodMotionContext;

test("Motion preview passes through one producer-owned fixed package", () => {
  const preview = buildMotionPreview(context);
  expect(preview.fixedPackageManifestJson).toBe("producer-owned-fixed-package-manifest");
  expect(preview.timelineJson).toBe("producer-owned-canonical-json");
  expect(preview.resourceManifestJson).toBe("producer-owned-resource-manifest");
  expect(preview.verifiedBindingBundleJson).toBe("producer-owned-verified-bundle");
  expect(preview.assets).toEqual([{ id: "asset:cover", url: "/motion-assets/cover" }]);
  expect(preview).not.toHaveProperty("motion");
});
