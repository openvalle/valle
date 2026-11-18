// The browser test layer: the engine's SDK, as `cargo xtask build-web` builds it, plays the
// examples in Playwright's Chromium with WebGPU. Its canvas shows the frames the native engine
// draws, within a step of rounding.
//
//   bun web/tests/player.ts <valle binary>
//
// The native frames are drawn first, which puts the built-in fonts in the cache; the page fetches
// them from there, so the test needs no external network after the browser is installed.

import { mkdtemp, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { cache, evaluate as evaluatePage, launch, root } from "./browser.ts";

/** The examples and frames compared, and the props bound once the player is open. */
const CASES: {
  entry: string;
  frames: number[];
  props?: Record<string, unknown>;
  /** Native files for resources whose defaults use the test server's virtual root. */
  nativeResources?: Record<string, string>;
  pixels?: { maxDifference: number; maxPixels: number };
}[] = [
  { entry: "tests/cases/css/generated/boxes-00/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/generated/boxes-01/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/generated/boxes-02/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/generated/boxes-03/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/generated/boxes-04/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/generated/boxes-05/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/generated/boxes-06/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/generated/boxes-07/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/generated/boxes-08/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/generated/boxes-09/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/generated/boxes-10/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/generated/boxes-11/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/generated/boxes-12/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/generated/boxes-13/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/generated/boxes-14/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/generated/boxes-15/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/generated/boxes-16/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/generated/boxes-17/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/generated/boxes-18/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/generated/boxes-19/scene.motion.tsx", frames: [0] },
  {
    entry: "tests/cases/svg/combinations/dynamic-file-effects/scene.motion.tsx",
    frames: [0, 7, 1, 6, 2, 5, 3, 4, 0],
  },
  {
    entry: "tests/cases/svg/combinations/dynamic-stroke-markers/scene.motion.tsx",
    frames: [0, 7, 1, 6, 2, 5, 3, 4, 0],
  },
  { entry: "tests/cases/svg/combinations/paint-clips-filters/file/scene.motion.tsx", frames: [0] },
  {
    entry: "tests/cases/svg/combinations/paint-clips-filters/inline/scene.motion.tsx",
    frames: [0],
  },
  {
    entry: "tests/cases/svg/combinations/path-text-metrics/file/scene.motion.tsx",
    nativeResources: { Test: "tests/resources/NotoSans.ttf" },
    frames: [0],
  },
  {
    entry: "tests/cases/svg/combinations/path-text-metrics/inline/scene.motion.tsx",
    nativeResources: { Test: "tests/resources/NotoSans.ttf" },
    frames: [0],
  },
  { entry: "tests/cases/svg/combinations/stroke-markers/file/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/svg/combinations/stroke-markers/inline/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/svg/combinations/viewport-use/file/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/svg/combinations/viewport-use/inline/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/transform/order-clipping/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/transform/perspective-groups/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/transforms/path-extents/scene.motion.tsx", frames: [0] },
  {
    entry: "tests/cases/css/transform/dynamic-order-clipping/scene.motion.tsx",
    frames: [0, 7, 1, 6, 2, 5, 3, 4, 0],
  },
  {
    entry: "tests/cases/css/transform/dynamic-perspective/scene.motion.tsx",
    frames: [0, 7, 1, 6, 2, 5, 3, 4, 0],
  },
  {
    entry: "tests/cases/css/transforms/dynamic-path-extents/scene.motion.tsx",
    frames: [0, 7, 1, 6, 2, 5, 3, 4, 0],
  },
  { entry: "tests/cases/css/compositing/blend-families/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/compositing/backdrop-roots/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/mask/three-layer-composites/scene.motion.tsx", frames: [0] },
  {
    entry: "tests/cases/css/compositing/dynamic-backdrop-roots/scene.motion.tsx",
    frames: [0, 7, 1, 6, 2, 5, 3, 4, 0],
  },
  {
    entry: "tests/cases/css/mask/dynamic-three-layers/scene.motion.tsx",
    frames: [0, 7, 1, 6, 2, 5, 3, 4, 0],
  },
  {
    entry: "tests/cases/css/text/font-features/scene.motion.tsx",
    nativeResources: {
      Test: "tests/resources/NotoSans.ttf",
      Han: "tests/resources/NotoSansSC.ttf",
      Hebrew: "tests/resources/NotoSansHebrew.ttf",
    },
    frames: [0],
  },
  {
    entry: "tests/cases/css/text/axes-synthesis/scene.motion.tsx",
    nativeResources: {
      Test: "tests/resources/NotoSans.ttf",
      Han: "tests/resources/NotoSansSC.ttf",
      Hebrew: "tests/resources/NotoSansHebrew.ttf",
    },
    frames: [0],
  },
  {
    entry: "tests/cases/css/text/relative-inheritance/scene.motion.tsx",
    nativeResources: {
      Test: "tests/resources/NotoSans.ttf",
      Han: "tests/resources/NotoSansSC.ttf",
      Hebrew: "tests/resources/NotoSansHebrew.ttf",
    },
    frames: [0],
  },
  {
    entry: "tests/cases/css/text/bidi-inline-atoms/scene.motion.tsx",
    nativeResources: {
      Test: "tests/resources/NotoSans.ttf",
      Han: "tests/resources/NotoSansSC.ttf",
      Hebrew: "tests/resources/NotoSansHebrew.ttf",
    },
    frames: [0],
  },
  {
    entry: "tests/cases/css/text/breaking-products/scene.motion.tsx",
    nativeResources: {
      Test: "tests/resources/NotoSans.ttf",
      Han: "tests/resources/NotoSansSC.ttf",
      Hebrew: "tests/resources/NotoSansHebrew.ttf",
    },
    frames: [0],
  },
  {
    entry: "tests/cases/css/text/nested-decorations/scene.motion.tsx",
    nativeResources: {
      Test: "tests/resources/NotoSans.ttf",
      Han: "tests/resources/NotoSansSC.ttf",
      Hebrew: "tests/resources/NotoSansHebrew.ttf",
    },
    frames: [0],
  },
  {
    entry: "tests/cases/css/text/dynamic-axes/scene.motion.tsx",
    nativeResources: {
      Test: "tests/resources/NotoSans.ttf",
      Han: "tests/resources/NotoSansSC.ttf",
      Hebrew: "tests/resources/NotoSansHebrew.ttf",
    },
    frames: [0, 7, 1, 6, 2, 5, 3, 4, 0],
  },
  {
    entry: "tests/cases/css/text/dynamic-bidi-atoms/scene.motion.tsx",
    nativeResources: {
      Test: "tests/resources/NotoSans.ttf",
      Han: "tests/resources/NotoSansSC.ttf",
      Hebrew: "tests/resources/NotoSansHebrew.ttf",
    },
    frames: [0, 7, 1, 6, 2, 5, 3, 4, 0],
  },
  {
    entry: "tests/cases/css/text/dynamic-breaking/scene.motion.tsx",
    nativeResources: {
      Test: "tests/resources/NotoSans.ttf",
      Han: "tests/resources/NotoSansSC.ttf",
      Hebrew: "tests/resources/NotoSansHebrew.ttf",
    },
    frames: [0, 7, 1, 6, 2, 5, 3, 4, 0],
  },
  {
    entry: "tests/cases/css/text/dynamic-nested-decorations/scene.motion.tsx",
    nativeResources: {
      Test: "tests/resources/NotoSans.ttf",
      Han: "tests/resources/NotoSansSC.ttf",
      Hebrew: "tests/resources/NotoSansHebrew.ttf",
    },
    frames: [0, 7, 1, 6, 2, 5, 3, 4, 0],
  },
  { entry: "tests/cases/css/background/layer-lists/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/background/fractional-repeat/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/background/rounded-position/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/background/position-order/scene.motion.tsx", frames: [0] },
  {
    entry: "tests/cases/css/background/text-effects/scene.motion.tsx",
    nativeResources: { Test: "tests/resources/NotoSans.ttf" },
    frames: [0],
  },
  { entry: "tests/cases/css/color/spaces-opacity/scene.motion.tsx", frames: [0] },
  {
    entry: "tests/cases/css/images/fit-extremes/scene.motion.tsx",
    nativeResources: { picture: "tests/resources/photo.png" },
    frames: [0],
  },
  {
    entry: "tests/cases/css/background/dynamic-rounded-position/scene.motion.tsx",
    frames: [0, 7, 1, 6, 2, 5, 3, 4, 0],
  },
  {
    entry: "tests/cases/css/background/dynamic-layer-lists/scene.motion.tsx",
    frames: [0, 7, 1, 6, 2, 5, 3, 4, 0],
  },
  {
    entry: "tests/cases/css/images/dynamic-fit-extremes/scene.motion.tsx",
    nativeResources: { picture: "tests/resources/photo.png" },
    frames: [0, 7, 1, 6, 2, 5, 3, 4, 0],
  },
  {
    entry: "tests/cases/css/color/dynamic-spaces-opacity/scene.motion.tsx",
    frames: [0, 7, 1, 6, 2, 5, 3, 4, 0],
  },
  { entry: "tests/cases/css/border/unequal-corners/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/border/mixed-joins/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/border/logical-corners/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/shadow/outline-layers/scene.motion.tsx", frames: [0] },
  {
    entry: "tests/cases/css/border/dynamic-corners/scene.motion.tsx",
    frames: [0, 7, 1, 6, 2, 5, 3, 4, 0],
  },
  {
    entry: "tests/cases/css/shadow/dynamic-outline/scene.motion.tsx",
    frames: [0, 7, 1, 6, 2, 5, 3, 4, 0],
  },
  { entry: "tests/cases/css/values/inherited-units/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/sizing/indefinite-percentages/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/cascade/logical-order/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/sizing/intrinsic-constraints/scene.motion.tsx", frames: [0] },
  {
    entry: "tests/cases/css/values/dynamic-inherited-units/scene.motion.tsx",
    frames: [0, 7, 1, 6, 2, 5, 3, 4, 0],
  },
  {
    entry: "tests/cases/css/sizing/dynamic-percentages/scene.motion.tsx",
    frames: [0, 7, 1, 6, 2, 5, 3, 4, 0],
  },
  { entry: "tests/cases/css/compositing/visibility-backdrop/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/text/wrap-matrix/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/text/alignment-matrix/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/text/decoration-matrix/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/text/truncation-matrix/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/text/mixed-size-matrix/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/compositing/filter-blend-matrix/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/compositing/clip-mask-matrix/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/compositing/nested-matrix/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/filter/transformed-bounds/scene.motion.tsx", frames: [0] },
  {
    entry: "tests/cases/css/flex/dynamic-constraints/scene.motion.tsx",
    frames: [0, 7, 1, 6, 2, 5, 3, 4, 0],
  },
  {
    entry: "tests/cases/css/text/dynamic-inheritance/scene.motion.tsx",
    frames: [0, 7, 1, 6, 2, 5, 3, 4, 0],
  },
  {
    entry: "tests/cases/css/compositing/dynamic-layers/scene.motion.tsx",
    frames: [0, 7, 1, 6, 2, 5, 3, 4, 0],
  },
  { entry: "tests/cases/css/flex/constraints-matrix/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/flex/wrap-thresholds/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/layout/nested-sizing/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/layout/logical-sides-matrix/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/position/logical-containing-block/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/values/absolute-units/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/values/font-ex/scene.motion.tsx", frames: [0] },
  {
    entry: "tests/cases/css/text/font-synthesis/scene.motion.tsx",
    frames: [0],
    nativeResources: { Hebrew: "tests/resources/NotoSansHebrew.ttf" },
  },
  { entry: "tests/cases/css/text/font-size-adjust/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/layout/vertical-block/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/layout/vertical-flex/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/layout/vertical-grid/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/layout/vertical-position/scene.motion.tsx", frames: [0] },
  {
    entry: "tests/cases/css/text/vertical-container/scene.motion.tsx",
    frames: [0],
    nativeResources: { CJK: "tests/resources/NotoSansSC.ttf" },
  },
  {
    entry: "tests/cases/css/text/caps/scene.motion.tsx",
    frames: [0],
    nativeResources: { Sans: "tests/resources/NotoSans.ttf" },
  },
  { entry: "tests/cases/svg/font-units/inline/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/svg/font-units/file/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/svg/absolute-units/inline/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/svg/absolute-units/file/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/svg/text-path-length/inline/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/svg/text-path-length/file/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/svg/inline-image/inline/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/svg/inline-image/file/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/svg/external-fragments/inline/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/svg/external-fragments/file/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/svg/color-interpolation/inline/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/svg/color-interpolation/file/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/svg/marker-overflow/inline/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/svg/marker-overflow/file/scene.motion.tsx", frames: [0] },
  {
    entry: "tests/cases/svg/text-direction/inline/scene.motion.tsx",
    frames: [0],
    nativeResources: {
      Latin: "tests/resources/NotoSans.ttf",
      Hebrew: "tests/resources/NotoSansHebrew.ttf",
    },
  },
  {
    entry: "tests/cases/svg/text-direction/file/scene.motion.tsx",
    frames: [0],
    nativeResources: {
      Latin: "tests/resources/NotoSans.ttf",
      Hebrew: "tests/resources/NotoSansHebrew.ttf",
    },
  },
  {
    entry: "tests/cases/svg/vertical-text/inline/scene.motion.tsx",
    frames: [0],
    nativeResources: {
      Latin: "tests/resources/NotoSans.ttf",
      CJK: "tests/resources/NotoSansSC.ttf",
    },
  },
  {
    entry: "tests/cases/svg/vertical-text/file/scene.motion.tsx",
    frames: [0],
    nativeResources: {
      Latin: "tests/resources/NotoSans.ttf",
      CJK: "tests/resources/NotoSansSC.ttf",
    },
  },
  {
    entry: "tests/cases/svg/text-path-stretch/inline/scene.motion.tsx",
    frames: [0],
    nativeResources: { Latin: "tests/resources/NotoSans.ttf" },
  },
  {
    entry: "tests/cases/svg/text-path-stretch/file/scene.motion.tsx",
    frames: [0],
    nativeResources: { Latin: "tests/resources/NotoSans.ttf" },
  },
  { entry: "tests/cases/svg/attribute-selectors/file/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/svg/structural-selectors/file/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/svg/escaped-selectors/file/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/svg/filter-paints/inline/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/svg/filter-paints/file/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/svg/filter-tile/inline/scene.motion.tsx", frames: [0, 5, 2, 7, 0] },
  { entry: "tests/cases/svg/filter-image/inline/scene.motion.tsx", frames: [0, 5, 2, 7, 0] },
  { entry: "tests/cases/svg/filter-specular/inline/scene.motion.tsx", frames: [0, 5, 2, 7, 0] },
  { entry: "tests/cases/svg/filter-diffuse/inline/scene.motion.tsx", frames: [0, 5, 2, 7, 0] },
  { entry: "tests/cases/svg/filter-convolve/inline/scene.motion.tsx", frames: [0, 5, 2, 7, 0] },
  { entry: "tests/cases/svg/filter-turbulence/inline/scene.motion.tsx", frames: [0, 5, 2, 7, 0] },
  { entry: "tests/cases/css/cascade/wide-keywords/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/text/inline-box/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/text/inline-directions/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/svg/stylesheets/file/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/svg/external-images/file/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/svg/filter-graph/inline/scene.motion.tsx", frames: [0, 5, 2, 7, 0] },
  { entry: "tests/cases/svg/filter-graph/file/scene.motion.tsx", frames: [0, 5, 2, 7, 0] },
  {
    entry: "tests/cases/css/color/current-mix/scene.motion.tsx",
    frames: [0, 5, 2, 7, 0],
    nativeResources: { Sans: "tests/resources/NotoSans.ttf" },
  },
  {
    entry: "tests/cases/css/text/underline-position/scene.motion.tsx",
    frames: [0, 5, 2, 7, 0],
    nativeResources: { CJK: "tests/resources/NotoSansSC.ttf" },
  },
  {
    entry: "tests/cases/css/text/bidi/scene.motion.tsx",
    frames: [0, 5, 2, 7, 0],
    nativeResources: {
      Latin: "tests/resources/NotoSans.ttf",
      Hebrew: "tests/resources/NotoSansHebrew.ttf",
    },
  },
  {
    entry: "tests/cases/css/text/combine/scene.motion.tsx",
    frames: [0, 5, 2, 7, 0],
    nativeResources: { CJK: "tests/resources/NotoSansSC.ttf" },
  },
  {
    entry: "tests/cases/css/text/emphasis/scene.motion.tsx",
    frames: [0, 5, 2, 7, 0],
    nativeResources: {
      Sans: "tests/resources/NotoSans.ttf",
      CJK: "tests/resources/NotoSansSC.ttf",
    },
  },
  { entry: "tests/cases/svg/text-baselines/inline/scene.motion.tsx", frames: [0, 5, 2, 7, 0] },
  { entry: "tests/cases/svg/text-baselines/file/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/svg/nested-viewports/inline/scene.motion.tsx", frames: [0, 5, 2, 7, 0] },
  { entry: "tests/cases/svg/nested-viewports/file/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/svg/clip-content/inline/scene.motion.tsx", frames: [0, 5, 2, 7, 0] },
  { entry: "tests/cases/svg/clip-content/file/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/transforms/shape-path/scene.motion.tsx", frames: [0, 5, 2, 7, 0] },
  { entry: "tests/cases/svg/non-scaling-stroke/inline/scene.motion.tsx", frames: [0, 5, 2, 7, 0] },
  { entry: "tests/cases/svg/non-scaling-stroke/file/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/svg/text/inline/scene.motion.tsx", frames: [0, 5, 2, 7, 0] },
  { entry: "tests/cases/svg/text/file/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/svg/pattern/inline/scene.motion.tsx", frames: [0, 5, 2, 7, 0] },
  { entry: "tests/cases/svg/pattern/file/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/svg/large-pattern/inline/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/svg/large-pattern/file/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/css/mask/shape/scene.motion.tsx", frames: [0, 5, 2, 7, 0] },
  {
    entry: "tests/cases/css/text/paragraph/scene.motion.tsx",
    frames: [0, 5, 2, 7, 0],
    nativeResources: { CJK: "tests/resources/NotoSansSC.ttf" },
  },
  {
    entry: "tests/cases/css/text/vertical/scene.motion.tsx",
    frames: [0, 5, 2, 7, 0],
    nativeResources: { CJK: "tests/resources/NotoSansSC.ttf" },
  },
  { entry: "tests/cases/css/text/font-size-keywords/scene.motion.tsx", frames: [0, 0] },
  { entry: "tests/cases/css/text/font-position/scene.motion.tsx", frames: [0, 0] },
  { entry: "tests/cases/css/text/vertical-align/scene.motion.tsx", frames: [0, 0] },
  {
    entry: "tests/cases/css/text/inline-boxes/scene.motion.tsx",
    frames: [0, 0],
    nativeResources: { picture: "tests/resources/svg/pixels.png" },
  },
  { entry: "tests/cases/css/text/inline-writing/scene.motion.tsx", frames: [0, 0] },
  { entry: "tests/cases/motion/inline-layout/scene.motion.tsx", frames: [0, 4, 2, 7, 0] },
  { entry: "examples/hello.motion.tsx", frames: [0, 60] },
  { entry: "tests/cases/svg/css-transforms/inline/scene.motion.tsx", frames: [0, 5, 2, 7, 0] },
  { entry: "tests/cases/svg/css-transforms/file/scene.motion.tsx", frames: [0] },
  { entry: "tests/cases/svg/css-effects/inline/scene.motion.tsx", frames: [0, 5, 2, 7, 0] },
  { entry: "tests/cases/svg/css-effects/file/scene.motion.tsx", frames: [0] },
  { entry: "examples/01-kinetic-title.motion.tsx", frames: [45] },
  { entry: "examples/02-logo-sting.motion.tsx", frames: [165, 270] },
  { entry: "examples/03-data-dashboard.motion.tsx", frames: [120] },
  { entry: "examples/04-ui-showcase.motion.tsx", frames: [150] },
  { entry: "examples/05-story-vertical.motion.tsx", frames: [150] },
  { entry: "examples/06-memphis-loop.motion.tsx", frames: [30] },
  { entry: "examples/07-route-map.motion.tsx", frames: [150] },
  // Reviewed native/WASM captures differ at 3 isolated pixels by at most 13 levels in the
  // rotating translucent ray path. Removing that path makes both frames pixel-identical.
  {
    entry: "examples/08-celebrate.motion.tsx",
    frames: [90, 198],
    pixels: { maxDifference: 16, maxPixels: 4 },
  },
  { entry: "examples/launch.motion.tsx", frames: [150] },
  { entry: "tests/cases/motion/spring/scene.motion.tsx", frames: [0, 6, 4, 10] },
  { entry: "tests/cases/motion/noise/scene.motion.tsx", frames: [0, 4, 2] },
  { entry: "tests/cases/motion/points/scene.motion.tsx", frames: [0, 7, 2, 4, 0] },
  { entry: "tests/cases/motion/svg-map/scene.motion.tsx", frames: [0, 7, 2, 5, 0] },
  { entry: "tests/cases/motion/svg-instances/scene.motion.tsx", frames: [0, 7, 2, 5, 0] },
  { entry: "tests/cases/motion/svg-shapes/scene.motion.tsx", frames: [0, 7, 2, 5, 0] },
  { entry: "tests/cases/motion/particles/scene.motion.tsx", frames: [0, 7, 8, 9] },
  { entry: "examples/09-cards-3d.motion.tsx", frames: [210] },
  { entry: "examples/10-lower-third.motion.tsx", frames: [40] },
  {
    entry: "examples/10-lower-third.motion.tsx",
    frames: [40],
    props: { name: "Ada Lovelace", accent: "#00C2A8" },
  },
  { entry: "examples/edit.timeline.json", frames: [20] },
  { entry: "tests/resources/shaders/scene.motion.tsx", frames: [0, 2, 3, 1] },
  { entry: "tests/resources/shaders/transitions.motion.tsx", frames: [0, 1, 2, 3, 4] },
  { entry: "tests/resources/shaders/grade.timeline.json", frames: [0] },
  { entry: "tests/resources/video/rotation.motion.tsx", frames: [0, 2, 1] },
  {
    entry: "tests/resources/timeline/captions/animations.timeline.json",
    frames: [6, 57, 111, 153, 207, 249, 282],
  },
  {
    entry: "tests/resources/timeline/captions/layout.timeline.json",
    frames: [0, 60, 120],
  },
  {
    entry: "tests/resources/timeline/captions/styled.timeline.json",
    frames: [15, 60, 75, 105, 120, 150, 179, 210],
  },
  {
    entry: "tests/resources/timeline/captions/boundaries.timeline.json",
    frames: [45, 75],
  },
];
/** The most a channel may differ, and the share of pixels that may differ by more than 1. */
const STEP = 2;
const SHARE = 0.001;

const valle = process.argv[2];
if (!valle) {
  throw new Error("usage: bun web/tests/player.ts <valle binary>");
}

const PAGE = `<!doctype html>
<style>html, body { margin: 0; } canvas { display: block; }</style>
<canvas></canvas>
<script type="module">
  import { createPlayer } from "/web/packages/engine/dist/index.js";
  import { exportBrowserVideo } from "/web/packages/engine/dist/export.js";
  import "/web/packages/engine/dist/element.js";
  /** The files players fetched through the page's \`fetch\`. */
  window.fetched = 0;
  const fetching = (url, init) => (window.fetched++, fetch(url, init));
  /** The pixels of a frame the native engine drew. */
  const decoded = async (url) => {
    const bitmap = await createImageBitmap(await (await fetch(url)).blob(), {
      colorSpaceConversion: "none",
      premultiplyAlpha: "none",
    });
    const c = new OffscreenCanvas(bitmap.width, bitmap.height).getContext("2d");
    c.drawImage(bitmap, 0, 0);
    return c.getImageData(0, 0, bitmap.width, bitmap.height).data;
  };
  /** Opens \`entry\`, draws frame \`index\`, and says how its pixels differ from \`native\`'s: the
   * most a channel does, and how many pixels do at all. */
  window.check = async (entry, index, native, props) => {
    await window.player?.close();
    const canvas = document.createElement("canvas");
    document.querySelector("canvas").replaceWith(canvas);
    const started = performance.now();
    window.player = await createPlayer({ canvas, entry, base: "/", fonts: "/fonts/", fetch: fetching });
    if (props) await window.player.setProps(props);
    // The canvas laid out at its size now, not once the worker's first frame reaches the page.
    const { width, height } = window.player.info;
    canvas.style.width = width + "px";
    canvas.style.height = height + "px";
    await window.player.draw(index);
    const ms = Math.round(performance.now() - started);
    const [x, y] = [await window.player.pixels(), await decoded(native)];
    if (x.length !== y.length) return { size: true, ms };
    let [most, differ] = [0, 0];
    /** Where pixels differ by more than 1: left, top, right and bottom. */
    const box = [Infinity, Infinity, -1, -1];
    for (let i = 0; i < x.length; i += 4) {
      // A 2D canvas keeps colors premultiplied: those of pixels not opaque come back rounded.
      const channels = x[i + 3] === 255 && y[i + 3] === 255 ? [0, 1, 2, 3] : [3];
      let d = 0;
      for (const k of channels) d = Math.max(d, Math.abs(x[i + k] - y[i + k]));
      most = Math.max(most, d);
      if (d > 1) {
        differ++;
        const [px, py] = [(i / 4) % width, Math.floor(i / 4 / width)];
        [box[0], box[1]] = [Math.min(box[0], px), Math.min(box[1], py)];
        [box[2], box[3]] = [Math.max(box[2], px), Math.max(box[3], py)];
      }
    }
    // A transferred canvas's DOM width/height may still be the initial 300x150 here.
    // Use the composition size for the screenshot, as for its CSS size above.
    return { most, differ, box, pixels: x.length / 4, ms, size: [width, height] };
  };
  /** The frame the player drew last, as a PNG's data address. */
  window.drawn = async () => {
    const { width, height } = window.player.info;
    const c = new OffscreenCanvas(width, height);
    const pixels = new Uint8ClampedArray(await window.player.pixels());
    c.getContext("2d").putImageData(new ImageData(pixels, width, height), 0, 0);
    const bytes = new Uint8Array(await (await c.convertToBlob()).arrayBuffer());
    return "data:image/png;base64," + btoa(Array.from(bytes, (b) => String.fromCharCode(b)).join(""));
  };
  /** How far a player's clock advances in a second of playback, and how many frames
   * it drew on the way: as many as the page's animation frames, which a page in the background
   * has fewer of. */
  window.played = async () => {
    const shown = new Set();
    const stats = [];
    const offs = [
      window.player.on("time", ({ frame }) => shown.add(frame)),
      window.player.on("stats", (s) => stats.push(s)),
    ];
    // Audio initialization is asynchronous. Measure playback once its clock starts;
    // the host's evaluation timeout still fails a player that never starts.
    const started = new Promise((resolve) => {
      offs.push(window.player.on("time", ({ frame }) => { if (frame > 0) resolve(); }));
    });
    window.player.seekFrame(0);
    window.player.play();
    await started;
    const from = window.player.currentTime;
    await new Promise((r) => setTimeout(r, 1000));
    window.player.pause();
    for (const off of offs) off();
    const counted = stats.reduce((n, s) => n + s.shown, 0);
    return { advanced: window.player.currentTime - from, drawn: shown.size, counted, memory: stats.at(-1)?.memory };
  };
  /** How the sound of \`entry\` differs from the native mix in the WAV at \`mix\`, and how far its
   * clock advances in a second of playback. */
  window.heard = async (entry, mix) => {
    await window.player?.close();
    const canvas = document.createElement("canvas");
    document.querySelector("canvas").replaceWith(canvas);
    const player = (window.player = await createPlayer({ canvas, entry, base: "/", fonts: "/fonts/" }));
    const native = new Float32Array(await (await (await fetch(mix)).arrayBuffer()).slice(44));
    const mine = await player.renderSound(0, native.length / 2 / 48000);
    let most = mine.length === native.length ? 0 : Infinity;
    for (let i = 0; i < native.length; i++) most = Math.max(most, Math.abs(mine[i] - native[i]));
    const { advanced } = await window.played();
    return { sound: player.hasSound, samples: native.length / 2, most, advanced };
  };
  /** Whether binding a prop to a value its control does not take fails, and leaves the frame
   * as it was. */
  window.invalid = async (index) => {
    const before = await window.player.pixels();
    try {
      await window.player.setProps({ accent: "not a color" });
      return { rejected: false };
    } catch (error) {
      await window.player.draw(index);
      const after = await window.player.pixels();
      const same = before.length === after.length && before.every((b, i) => b === after[i]);
      return { rejected: true, code: error.code, message: error.message, same };
    }
  };
  /** The MP4 \`entry\` exports as \`options\` say, as a data address, how long it took and how
   * many frames it said it had done of how many. */
  window.exported = async (entry, options = {}) => {
    await window.player?.close();
    const canvas = document.createElement("canvas");
    document.querySelector("canvas").replaceWith(canvas);
    const player = (window.player = await createPlayer({ canvas, entry, base: "/", fonts: "/fonts/" }));
    let [done, count] = [0, 0];
    const started = performance.now();
    const { blob: mp4 } = await exportBrowserVideo(player, {
      ...options,
      onProgress: ({ framesCompleted, frameCount }) => ([done, count] = [framesCompleted, frameCount]),
    });
    const ms = Math.round(performance.now() - started);
    const url = await new Promise((r) => {
      const reader = new FileReader();
      reader.onload = () => r(reader.result);
      reader.readAsDataURL(mp4);
    });
    return { url, ms, done, count, frames: player.info.frames };
  };
  /** How an export of what the player has open, cancelled after 3 frames, fails. */
  window.cancelled = async () => {
    const abort = new AbortController();
    try {
      await exportBrowserVideo(window.player, {
        signal: abort.signal,
        onProgress: ({ framesCompleted }) => framesCompleted === 3 && abort.abort(),
      });
      return "not cancelled";
    } catch (error) {
      return error.code;
    }
  };
  // Neither image is delivered until the worker has asked for both.
  window.openSharedImages = async () => {
    const entry = "batch.motion.tsx";
    const source = 'export const composition = {width:16,height:16,fps:1,duration:1};' +
      'export const controls = {resources:{a:resource({kind:"image",default:"a.png"}),' +
      'b:resource({kind:"image",default:"b.png"})}}; export default function Scene(){return <View/>}';
    const seen = [];
    let release;
    const both = new Promise(resolve => { release = resolve; });
    const player = await createPlayer({
      entry, canvas: document.createElement("canvas"), base: "/",
      files: { [entry]: new Blob([source]) },
      async fetch(url, init) {
        const name = new URL(url).pathname;
        if (name === "/a.png" || name === "/b.png") {
          seen.push(name);
          if (seen.length === 2) release();
          await both;
          return fetch("/tests/resources/svg/pixels.png");
        }
        return fetch(url, init);
      },
    });
    await player.close();
    return seen.sort().join(",");
  };
  window.edited = async (entry) => {
    await window.player?.close();
    const canvas = document.createElement("canvas");
    document.querySelector("canvas").replaceWith(canvas);
    const source = await (await fetch(entry)).json();
    let ranges = 0;
    let starts = 0;
    const player = (window.player = await createPlayer({
      canvas,
      entry,
      fetch(url, init) {
        if (new Headers(init?.headers).has("Range") && new URL(url).pathname.endsWith("/tone.wav")) {
          ranges++;
        }
        return fetch(url, init);
      },
    }));
    player.on("play", () => starts++);
    player.muted = true;
    await player.draw(24);
    const baseline = await player.renderSound(0.4, 0.2);
    const checks = ranges;
    const edit = structuredClone(source);
    edit.canvas.fps = 60;
    edit.tracks.visual[0].clips[0].color = "#ff0000ff";
    const changed = player.updateTimeline(edit);
    player.play();
    await changed;
    const at = player.currentTime;
    const resumed = player.playing && starts === 1;
    player.pause();
    const [r, g, b] = await player.pixels();
    const after = await player.renderSound(0.4, 0.2);
    const keptAudio = baseline.every((s, i) => s === after[i]) && baseline.some((s) => s !== 0);
    const reused = checks > 0 && ranges === checks;
    const rejected = await player.updateTimeline(edit, {
      files: { "/sound/tone.wav": new Blob(["broken audio"]) },
    }).then(() => false, () => true);
    const stillEdited = player.info.frames === 120 && (await player.pixels())[0] === r;
    const restoredAudio = (await player.renderSound(0.4, 0.2)).every((s, i) => s === baseline[i]);

    const snapshot = exportBrowserVideo(player, { target: { type: "opfs" } });
    edit.tracks.visual[0].clips[0].duration = 1;
    delete edit.tracks.audio;
    await player.updateTimeline(edit);
    const saved = await snapshot;
    const file = await saved.fileHandle.getFile();
    const video = document.createElement("video");
    video.preload = "metadata";
    video.src = URL.createObjectURL(file);
    try {
      await new Promise((resolve, reject) => {
        video.onloadedmetadata = resolve;
        video.onerror = () => reject(video.error);
      });
      const duration = video.duration;
      const root = await navigator.storage.getDirectory();
      const folder = await root.getDirectoryHandle("valle-exports");
      const abort = new AbortController();
      const cancelled = await exportBrowserVideo(player, {
        target: { type: "opfs" },
        signal: abort.signal,
        onProgress: ({ framesCompleted }) => { if (framesCompleted === 3) abort.abort(); },
      }).then(() => false, (error) => error.code === "cancelled");
      await saved.cleanup();
      const empty = (await folder.keys().next()).done;
      return { at, resumed, r, g, b, keptAudio, reused, rejected, stillEdited, restoredAudio, duration,
        bytes: file.size, cancelled, empty, editedDuration: player.info.frames / 60 };
    } finally {
      video.removeAttribute("src");
      await saved.cleanup();
    }
  };
  window.mediaRegressions = async () => {
    const entry = "/sound/surround.timeline.json";
    const old = await (await fetch(entry)).json();
    let downloads = 0;
    const player = await createPlayer({
      entry, canvas: document.createElement("canvas"),
      fetch(url, init) {
        if (new URL(url).pathname.endsWith("/surround.wav")) downloads++;
        return fetch(url, init);
      },
    });
    try {
      const directory = await (await navigator.storage.getDirectory()).getDirectoryHandle("valle-inputs");
      const countFiles = async () => { let count = 0; for await (const _ of directory.keys()) count++; return count; };
      const stored = await countFiles();
      const sample = async (time) => Array.from((await player.renderSound(time, 0.01)).slice(-2));
      const first = await sample(1);
      const late = await sample(32);
      const back = await sample(1);
      await player.draw(25 * 30);
      const before = Array.from((await player.pixels()).slice(0, 4));
      const failures = [];
      for (const width of [old.canvas.width, old.canvas.width + 16]) {
        const edit = structuredClone(old);
        edit.canvas.width = width;
        edit.resources.large = "large.motion.tsx";
        edit.tracks.visual[0].clips = [{ kind: "motion", component: "large", duration: 1, start: 0 }];
        delete edit.tracks.audio;
        const error = await player.updateTimeline(edit).then(() => "", e => e.message);
        const after = Array.from((await player.pixels()).slice(0, 4));
        failures.push({ error, restored: player.info.duration === 40 && player.currentTime === 25 && player.info.width === old.canvas.width
          && JSON.stringify(before) === JSON.stringify(after) && JSON.stringify(await sample(1)) === JSON.stringify(first) });
      }
      const previewDownloads = downloads;
      const { blob } = await exportBrowserVideo(player, { width: 32, height: 18, frameRate: 1 });
      const decoded = await new OfflineAudioContext(2, 1, 48000).decodeAudioData(await blob.arrayBuffer());
      const levels = Array.from({ length: decoded.numberOfChannels }, (_, ch) => {
        const samples = decoded.getChannelData(ch).subarray(48000, 96000);
        return Math.sqrt(samples.reduce((sum, v) => sum + v * v, 0) / samples.length);
      });
      const video = document.createElement("video");
      const url = URL.createObjectURL(blob);
      let exportedDuration;
      try {
        video.src = url;
        await new Promise((ok, fail) => { video.onloadedmetadata = ok; video.onerror = () => fail(video.error); });
        exportedDuration = video.duration;
      } finally {
        URL.revokeObjectURL(url);
      }
      await player.close();
      return { first, late, back, previewDownloads, exportDownloads: downloads - previewDownloads, stored, remaining: await countFiles(), failures, exportedDuration, levels };
    } catch (error) {
      await player.close();
      throw error;
    }
  };
  /** A <valle-player> of \`src\`, looping, playing on its own, muted: whether it plays, and how
   * its frame at 1 s differs from \`native\`'s; and whether taking it out of the page closes
   * its player. */
  window.element = async (src, native) => {
    await window.player?.close();
    window.player = undefined;
    const element = document.createElement("valle-player");
    element.setAttribute("src", src);
    element.setAttribute("runtime", "/web/packages/engine/dist/");
    for (const name of ["loop", "autoplay", "muted"]) element.setAttribute(name, "");
    const ready = new Promise((resolve, reject) => {
      element.addEventListener("ready", resolve, { once: true });
      element.addEventListener("error", (event) => reject(event.detail), { once: true });
    });
    document.body.append(element);
    await ready;
    const player = element.player;
    await new Promise((r) => setTimeout(r, 700));
    const [playing, at, muted, loop] = [player.playing, player.currentTime, player.muted, player.loop];
    const png = URL.createObjectURL(await player.frame(1));
    const [x, y] = [await decoded(png), await decoded(native)];
    let most = x.length === y.length ? 0 : Infinity;
    for (let i = 0; i < x.length && most !== Infinity; i++) most = Math.max(most, Math.abs(x[i] - y[i]));
    element.remove();
    return { playing, at, muted, loop, most, closed: element.player === undefined };
  };
  /** How two frames differ: the most a channel does, and on average. */
  window.similar = async (a, b) => {
    const [x, y] = [await decoded(a), await decoded(b)];
    if (x.length !== y.length) return { most: Infinity, mean: Infinity };
    let [most, sum, inkSum, inkChannels] = [0, 0, 0, 0];
    for (let i = 0; i < x.length; i += 4) {
      for (let k = 0; k < 3; k++) {
        const d = Math.abs(x[i + k] - y[i + k]);
        most = Math.max(most, d);
        sum += d;
        // Bright caption glyphs against a dark background: the full-frame mean alone
        // can hide missing words because text occupies little of the canvas.
        if (Math.max(y[i], y[i + 1], y[i + 2]) > 128) {
          inkSum += d;
          inkChannels++;
        }
      }
    }
    return { most, mean: sum / (x.length / 4) / 3, inkMean: inkChannels ? inkSum / inkChannels : 0, inkPixels: inkChannels / 3 };
  };
  /** How the frame on screen differs from \`native\`'s where it is opaque: the canvas is. */
  window.shown = async (screenshot, native) => {
    const [x, y] = [await decoded(screenshot), await decoded(native)];
    if (x.length !== y.length) return { most: Infinity, differ: Infinity };
    let most = 0, differ = 0;
    for (let i = 0; i < x.length; i += 4) {
      if (y[i + 3] !== 255) continue;
      let d = 0;
      for (let k = 0; k < 3; k++) d = Math.max(d, Math.abs(x[i + k] - y[i + k]));
      most = Math.max(most, d);
      if (d > 1) differ++;
    }
    return { most, differ };
  };
</script>
`;

/** The timeline with sound. */
const SOUND = {
  canvas: { width: 320, height: 180, fps: 30 },
  resources: { tone: "tone.wav" },
  tracks: {
    visual: [{ clips: [{ kind: "solid", color: "#204060ff", start: 0, duration: 2 }] }],
    audio: [{ clips: [{ src: "tone", start: 0.25, duration: 1.5, gain: 0.8 }] }],
  },
};

// A valid scene whose first GPU draw needs a texture larger than the device supports.
const OVERSIZED = `export const composition = { width: 320, height: 180, fps: 30, duration: 1 };
export default function Oversized() {
  const code = "@group(2) @binding(0) var backdrop: texture_2d<f32>; fn shade(i: ShaderInput) -> vec4f { return textureLoad(backdrop, vec2i(0), 0); }";
  return <ShaderLayer code={code} style={{ position: "absolute", width: 1000000, height: 16 }} />;
}`;

// Forty seconds of 5.1 PCM with signal in the centre speaker only.
function surround(): Uint8Array {
  const channels = 6;
  const frames = 40 * 48000;
  const size = frames * channels * 2;
  const bytes = new Uint8Array(44 + size);
  bytes.set(new TextEncoder().encode("RIFF"), 0);
  bytes.set(new TextEncoder().encode("WAVEfmt "), 8);
  bytes.set(new TextEncoder().encode("data"), 36);
  const v = new DataView(bytes.buffer);
  v.setUint32(4, size + 36, true);
  v.setUint32(16, 16, true);
  v.setUint16(20, 1, true);
  v.setUint16(22, channels, true);
  v.setUint32(24, 48000, true);
  v.setUint32(28, 48000 * channels * 2, true);
  v.setUint16(32, channels * 2, true);
  v.setUint16(34, 16, true);
  v.setUint32(40, size, true);
  for (let f = 0; f < frames; f++) {
    const amplitude = f < 20 * 48000 ? 8192 : -8192;
    v.setInt16(44 + (f * channels + 2) * 2, f % 480 < 240 ? -amplitude : amplitude, true);
  }
  return bytes;
}

/** A WAV of a sweep from `hertz` up an octave a second, for `seconds`: no two stretches of it
 * alike, so how late a copy of it is shows. Stereo 32-bit floats at 48 000 samples a second. */
function tone(seconds: number, hertz: number): Uint8Array {
  const samples = seconds * 48_000;
  const wav = new DataView(new ArrayBuffer(44 + samples * 8));
  const text = (at: number, s: string) =>
    [...s].forEach((c, i) => wav.setUint8(at + i, c.charCodeAt(0)));
  text(0, "RIFF");
  wav.setUint32(4, 36 + samples * 8, true);
  text(8, "WAVEfmt ");
  wav.setUint32(16, 16, true);
  wav.setUint16(20, 3, true);
  wav.setUint16(22, 2, true);
  wav.setUint32(24, 48_000, true);
  wav.setUint32(28, 48_000 * 8, true);
  wav.setUint16(32, 8, true);
  wav.setUint16(34, 32, true);
  text(36, "data");
  wav.setUint32(40, samples * 8, true);
  for (let i = 0; i < samples; i++) {
    const t = i / 48_000;
    const v = 0.5 * Math.sin((2 * Math.PI * hertz * (2 ** t - 1)) / Math.LN2);
    wav.setFloat32(44 + i * 8, v, true);
    wav.setFloat32(48 + i * 8, v, true);
  }
  return new Uint8Array(wav.buffer);
}

/** A scene that shows the edit's export, at the edit's size and rate. */
const SHOWN = `export const composition = { width: 640, height: 360, fps: 30, duration: 3 };
export const controls = {
  resources: { clip: resource({ kind: "video", default: "edit.mp4" }) },
};
export default function Shown() {
  return <Video src="resource://clip" style={{ width: 640, height: 360 }} />;
}
`;

/** A timeline that sounds the export of the timeline with sound. */
const LISTEN = {
  canvas: { width: 320, height: 180, fps: 30 },
  resources: { exported: "sound.mp4" },
  tracks: { audio: [{ clips: [{ src: "exported", start: 0, duration: 2 }] }] },
};

/** The samples of a WAV the native engine wrote: 32-bit floats after a 44-byte header. */
async function samples(path: string): Promise<Float32Array> {
  const bytes = await Bun.file(path).arrayBuffer();
  return new Float32Array(bytes.slice(44));
}

/** How many samples late `heard` is against `mixed`, at most 4096 either way, and how far apart
 * the two are then, as the RMS of their difference over the middle second. */
function aligned(heard: Float32Array, mixed: Float32Array): { lag: number; rms: number } {
  const apart = (lag: number) => {
    let sum = 0;
    for (let i = 24_000; i < 72_000; i++) {
      const d = (heard[2 * (i + lag)] ?? 0) - (mixed[2 * i] ?? 0);
      sum += d * d;
    }
    return Math.sqrt(sum / 48_000);
  };
  let best = { lag: 0, rms: apart(0) };
  for (let lag = -4096; lag <= 4096; lag++) {
    const rms = apart(lag);
    if (rms < best.rms) {
      best = { lag, rms };
    }
  }
  return best;
}

/** A scene for `<valle-player>`, without text: a square that moves. */
const ELEMENT = `export const composition = { width: 320, height: 180, fps: 30, duration: 2 };
export default function Element() {
  const time = useCurrentTime();
  const left = interpolate(time.progress, [0, 1], [0, 240]);
  return (
    <View className="relative h-full w-full" style={{ backgroundColor: "#203040" }}>
      <View style={{ position: "absolute", left, top: 50, width: 80, height: 80, backgroundColor: "#e0a030" }} />
    </View>
  );
}
`;

/** The width and height of the H.264 video of an MP4, from its `avc1` sample entry, and whether
 * it has a track of sound. */
function mp4(bytes: Uint8Array): { size: [number, number]; sound: boolean } {
  const text = Buffer.from(bytes).toString("latin1");
  // The sample entry in `stsd`: `avc1` is a brand in `ftyp` too.
  const at = text.indexOf("avc1", text.indexOf("stsd"));
  const view = new DataView(bytes.buffer, bytes.byteOffset);
  const size: [number, number] =
    at < 0 ? [0, 0] : [view.getUint16(at + 28), view.getUint16(at + 30)];
  return { size, sound: text.includes("mp4a") || text.includes("Opus") };
}

/** A frame's name: its entry, its index, and the props bound. */
function named(entry: string, frame: number, props?: Record<string, unknown>): string {
  return `${entry}#${frame}${props ? ` ${JSON.stringify(props)}` : ""}`;
}

/** The built-in fonts in the cache, by their file names. */
async function fonts(): Promise<Map<string, string>> {
  const dir = join(cache, "files");
  const found = new Map<string, string>();
  for (const sha of await readdir(dir).catch(() => [] as string[])) {
    for (const name of await readdir(join(dir, sha)).catch(() => [] as string[])) {
      found.set(name, join(dir, sha, name));
    }
  }
  return found;
}

function ranged(file: ReturnType<typeof Bun.file>, request: Request, headers?: HeadersInit) {
  const range = /^bytes=(\d+)-(\d*)$/.exec(request.headers.get("range") ?? "");
  if (!range) return new Response(file, { headers });
  const start = Number(range[1]);
  const end = Math.min(file.size - 1, range[2] ? Number(range[2]) : file.size - 1);
  if (start > end) return new Response(null, { status: 416 });
  const responseHeaders = new Headers(headers);
  responseHeaders.set("Content-Type", file.type);
  responseHeaders.set("Content-Range", `bytes ${start}-${end}/${file.size}`);
  return new Response(file.slice(start, end + 1), { status: 206, headers: responseHeaders });
}

async function main() {
  const out = await mkdtemp(join(tmpdir(), "valle-player-"));
  const natives = new Map<string, string>();
  for (const { entry, frames, props, nativeResources } of CASES) {
    const kind = entry.endsWith(".timeline.json") ? "timeline" : "motion";
    const bound = props ? ["--props", JSON.stringify(props)] : [];
    for (const [name, file] of Object.entries(nativeResources ?? {})) {
      bound.push("--resource", `${name}=${join(root, file)}`);
    }
    for (const frame of frames) {
      const name = named(entry, frame, props);
      if (natives.has(name)) continue;
      const png = join(out, `${natives.size}.png`);
      // Decode video with FFmpeg's own decoder: a virtual machine's hardware decoder hands
      // frames over unlike a real one's.
      const run = Bun.spawnSync(
        [
          valle,
          kind,
          "render",
          entry,
          "--frame",
          String(frame),
          ...bound,
          "--decoder",
          "ffmpeg",
          "-o",
          png,
        ],
        { cwd: root, stdout: "pipe", stderr: "pipe" },
      );
      if (run.exitCode !== 0) {
        throw new Error(`valle ${kind} render ${entry}: ${run.stderr.toString()}`);
      }
      natives.set(name, png);
    }
  }
  /** Runs the native engine. */
  const run = (args: string[]) => {
    const done = Bun.spawnSync([valle, ...args], { cwd: root, stdout: "pipe", stderr: "pipe" });
    if (done.exitCode !== 0) {
      throw new Error(`valle ${args.join(" ")}: ${done.stderr.toString()}`);
    }
  };
  // A timeline with sound: a tone, its own file, which both engines mix.
  const sound = join(out, "sound");
  await Bun.write(join(sound, "tone.wav"), tone(2, 440));
  await Bun.write(join(sound, "sound.timeline.json"), JSON.stringify(SOUND));
  await Bun.write(join(sound, "large.motion.tsx"), OVERSIZED);
  await Bun.write(join(sound, "surround.wav"), surround());
  await Bun.write(
    join(sound, "surround.timeline.json"),
    JSON.stringify({
      canvas: SOUND.canvas,
      resources: { sound: "surround.wav" },
      tracks: {
        visual: [
          {
            clips: [
              { kind: "solid", color: "#204060", start: 0, duration: 20 },
              { kind: "solid", color: "#ff0077", start: 20, duration: 20 },
            ],
          },
        ],
        audio: [{ clips: [{ src: "sound", start: 0, duration: 40 }] }],
      },
    }),
  );
  const mixed = Bun.spawnSync(
    [valle, "timeline", "render", join(sound, "sound.timeline.json"), "-o", join(out, "mix.wav")],
    { cwd: root, stdout: "pipe", stderr: "pipe" },
  );
  if (mixed.exitCode !== 0) {
    throw new Error(`valle timeline render: ${mixed.stderr.toString()}`);
  }
  const cached = await fonts();
  const server = Bun.serve({
    hostname: "127.0.0.1",
    port: 0,
    async fetch(request) {
      const path = decodeURIComponent(new URL(request.url).pathname);
      if (path === "/") {
        return new Response(PAGE, { headers: { "content-type": "text/html; charset=utf-8" } });
      }
      const font = /^\/fonts\/([\w.-]+)$/.exec(path);
      if (font) {
        const file = cached.get(font[1]);
        return file ? new Response(Bun.file(file)) : new Response("not found", { status: 404 });
      }
      if (path === "/sound/surround.wav")
        return new Response(
          Bun.file(join(sound, "surround.wav")).stream().pipeThrough(new TransformStream()),
          {
            headers: { "Content-Type": "audio/wav" },
          },
        );
      if (path === "/mix.wav" || path.startsWith("/sound/") || path.startsWith("/out/")) {
        return ranged(Bun.file(join(out, path.replace(/^\/(?:out\/)?/, ""))), request);
      }
      const frame = /^\/native\/(.+)$/.exec(path);
      if (frame && natives.has(frame[1])) {
        return new Response(Bun.file(natives.get(frame[1])!));
      }
      const file =
        /^\/((?:examples|tests\/(?:cases|resources)|web\/packages\/engine\/dist)\/[\w./-]+)$/.exec(
          path,
        );
      if (file && !file[1].includes("..")) {
        const f = Bun.file(join(root, file[1]));
        const type = file[1].endsWith(".js") ? { "content-type": "text/javascript" } : undefined;
        return f
          .exists()
          .then((there) =>
            there ? ranged(f, request, type) : new Response("not found", { status: 404 }),
          );
      }
      return new Response("not found", { status: 404 });
    },
  });
  const browser = await launch().catch((error) => {
    server.stop(true);
    throw error;
  });
  const failed: string[] = [];
  try {
    // Room for the largest composition at 1 px a pixel.
    const page = await browser.newPage({
      viewport: { width: 1920, height: 1920 },
      deviceScaleFactor: 1,
      colorScheme: "light",
    });
    console.log(`browser: Chromium ${browser.version()}`);
    page.on("console", (message) => console.log(`console.${message.type()}: ${message.text()}`));
    page.on("pageerror", (error) => console.log(`exception: ${error.message}`));
    await page.goto(`http://127.0.0.1:${server.port}/`);
    const evaluate = (expression: string) => evaluatePage(page, expression);
    for (const { entry, frames, props, pixels } of CASES) {
      for (const frame of frames) {
        const name = named(entry, frame, props);
        const native = `/native/${encodeURIComponent(name)}`;
        let d;
        try {
          d = await evaluate(
            `check(${JSON.stringify(entry)}, ${frame}, ${JSON.stringify(native)}, ${JSON.stringify(props)})`,
          );
        } catch (error) {
          failed.push(`${name}: ${String(error)}`);
          continue;
        }
        const line =
          d.size === true
            ? "a frame of another size"
            : `most ${d.most}, ${d.differ} of ${d.pixels} pixels by more than 1${d.differ ? `, within ${d.box.join(",")}` : ""}`;
        console.log(`${name}: ${line} (${d.ms} ms)`);
        const step = pixels?.maxDifference ?? STEP;
        const maxPixels = pixels?.maxPixels ?? d.pixels * SHARE;
        if (d.size === true || d.most > step || d.differ > maxPixels) {
          const url: string = await evaluate("drawn()");
          const png = Buffer.from(url.slice(url.indexOf(",") + 1), "base64");
          await Bun.write(
            join(out, `${natives.get(name)!.slice(out.length + 1, -4)}.browser.png`),
            png,
          );
          failed.push(`${name}: ${line}`);
          continue;
        }
        // The page shows it too.
        const [width, height] = d.size;
        // The canvas shows a frame the worker drew some animation frames later: how many is
        // the browser's, so the page is looked at again for up to 2 s.
        let shown = { most: Infinity, differ: Infinity };
        let screen = "";
        for (
          let tries = 0;
          tries < 20 && (shown.most > step || shown.differ > maxPixels);
          tries++
        ) {
          await evaluate(`new Promise((r) => setTimeout(() => requestAnimationFrame(r), 100))`);
          const png = await page.screenshot({
            clip: { x: 0, y: 0, width, height },
          });
          screen = png.toString("base64");
          shown = await evaluate(
            `shown(${JSON.stringify(`data:image/png;base64,${screen}`)}, ${JSON.stringify(native)})`,
          );
        }
        if (shown.most > step || shown.differ > maxPixels) {
          const png = natives.get(name)!.slice(0, -4);
          await Bun.write(`${png}.screen.png`, Buffer.from(screen, "base64"));
          failed.push(
            `${name}: on screen, most ${shown.most}, ${shown.differ} pixels by more than 1`,
          );
        }
        // A value its control does not take fails, and what was bound stays.
        if (props) {
          const invalid = await evaluate(`invalid(${frame})`);
          console.log(`invalid prop: ${JSON.stringify(invalid)}`);
          if (!invalid.rejected || !invalid.same) {
            failed.push(`binding an invalid prop: ${JSON.stringify(invalid)}`);
          }
        }
      }
    }
    // The last player plays on its own clock.
    const played = await evaluate("played()");
    console.log(
      `played: advanced ${played.advanced.toFixed(2)} s in 1 s, ${played.drawn} frames drawn, ${played.counted} counted, ${played.memory} bytes of memory`,
    );
    if (played.advanced < 0.5 || played.drawn < 2 || played.counted < 2 || !(played.memory > 0)) {
      failed.push(`playing for 1 s: ${JSON.stringify(played)}`);
    }
    const fetched = await evaluate("fetched");
    console.log(`fetched through the page's fetch: ${fetched} files`);
    if (fetched === 0) {
      failed.push("no file was fetched through the page's fetch");
    }
    // A timeline's sound: the native mix, and played on its clock.
    const heard = await evaluate(`heard("sound/sound.timeline.json", "/mix.wav")`);
    console.log(
      `sound: ${heard.samples} samples, most ${heard.most.toExponential(1)} apart; played: advanced ${heard.advanced.toFixed(2)} s in 1 s on the sound's clock`,
    );
    if (!heard.sound || heard.most > 1e-6 || heard.advanced < 0.5) {
      failed.push(`sound: ${JSON.stringify(heard)}`);
    }
    const batch = await evaluate("openSharedImages()");
    if (batch !== "/a.png,/b.png") failed.push(`declared resource batch: ${batch}`);
    const edited = await evaluate('edited("/sound/sound.timeline.json")');
    console.log(`timeline updates and OPFS: ${JSON.stringify(edited)}`);
    if (
      Math.abs(edited.at - 0.8) > 0.05 ||
      !edited.resumed ||
      edited.r < 250 ||
      edited.g > 1 ||
      edited.b > 1 ||
      !edited.keptAudio ||
      !edited.reused ||
      !edited.rejected ||
      !edited.stillEdited ||
      !edited.restoredAudio ||
      Math.abs(edited.duration - 2) > 0.05 ||
      !edited.bytes ||
      !edited.cancelled ||
      !edited.empty ||
      edited.editedDuration !== 1
    ) {
      failed.push(`timeline updates and OPFS: ${JSON.stringify(edited)}`);
    }
    const media = await evaluate("mediaRegressions()");
    console.log(`surround, remote seeking and failed presentation: ${JSON.stringify(media)}`);
    if (
      Math.abs(media.exportedDuration - 40) > 0.05 ||
      media.levels.length !== 2 ||
      media.levels.some((level: number) => Math.abs(level - Math.SQRT1_2 * 0.25) > 0.02) ||
      media.previewDownloads !== 1 ||
      media.exportDownloads !== 1 ||
      media.stored !== 1 ||
      media.remaining !== 0 ||
      media.first.some((v: number) => Math.abs(v - Math.SQRT1_2 * 0.25) > 1e-5) ||
      media.late.some((v: number) => Math.abs(v + Math.SQRT1_2 * 0.25) > 1e-5) ||
      JSON.stringify(media.back) !== JSON.stringify(media.first) ||
      media.failures.some(
        (f: { error: string; restored: boolean }) =>
          !f.error.includes("shader input exceeds this GPU's texture limit") || !f.restored,
      )
    )
      failed.push(`media regressions: ${JSON.stringify(media)}`);
    // Exports: the browser's MP4s, which the native engine decodes.
    for (const [entry, file] of [
      ["examples/edit.timeline.json", "edit.mp4"],
      ["sound/sound.timeline.json", "sound.mp4"],
      ["tests/resources/timeline/captions/animations.timeline.json", "captions.mp4"],
    ]) {
      const e = await evaluate(`exported(${JSON.stringify(entry)})`);
      await Bun.write(join(out, file), Buffer.from(e.url.slice(e.url.indexOf(",") + 1), "base64"));
      console.log(`exported ${entry}: ${e.frames} frames in ${e.ms} ms`);
      if (e.done !== e.frames || e.count !== e.frames) {
        failed.push(`exporting ${entry} said ${e.done} of ${e.count} frames were done`);
      }
    }
    // Reimport the subtitle export through the native decoder and check each animation,
    // including the final blank frame. This also validates the complete exported duration.
    const captionEntry = "tests/resources/timeline/captions/animations.timeline.json";
    const captionImport = join(out, "captions.timeline.json");
    await Bun.write(
      captionImport,
      JSON.stringify({
        canvas: { width: 640, height: 360, fps: 30 },
        resources: { video: "captions.mp4" },
        tracks: {
          visual: [{ items: [{ id: "video", kind: "video", src: "video", duration: 9.6 }] }],
        },
      }),
    );
    for (const frame of [6, 57, 111, 153, 207, 249, 282]) {
      const file = `caption-${frame}.png`;
      run([
        "timeline",
        "render",
        captionImport,
        "--decoder",
        "ffmpeg",
        "--frame",
        String(frame),
        "-o",
        join(out, file),
      ]);
      const native = `/native/${encodeURIComponent(named(captionEntry, frame))}`;
      const d = await evaluate(
        `similar(${JSON.stringify(`/out/${file}`)}, ${JSON.stringify(native)})`,
      );
      console.log(`caption export frame ${frame}: ${JSON.stringify(d)}`);
      if (d.mean > 1.5 || d.inkMean > 12 || (d.inkPixels === 0 && d.most > 8)) {
        failed.push(`subtitle export frame ${frame}: ${JSON.stringify(d)}`);
      }
    }
    // An export at another size and rate, without its sound, and one cancelled.
    const options = { width: 160, frameRate: 15, includeAudio: false };
    const small = await evaluate(
      `exported("sound/sound.timeline.json", ${JSON.stringify(options)})`,
    );
    const smallMp4 = mp4(Buffer.from(small.url.slice(small.url.indexOf(",") + 1), "base64"));
    console.log(
      `exported at ${JSON.stringify(options)}: ${small.count} frames of ${smallMp4.size.join("×")}, sound ${smallMp4.sound}`,
    );
    if (
      small.count !== 30 ||
      small.done !== 30 ||
      smallMp4.size.join() !== "160,90" ||
      smallMp4.sound
    ) {
      failed.push(
        `exporting at ${JSON.stringify(options)}: ${JSON.stringify({ ...small, url: undefined, ...smallMp4 })}`,
      );
    }
    const cancelled = await evaluate("cancelled()");
    if (cancelled !== "cancelled") {
      failed.push(`a cancelled export: ${cancelled}`);
    }
    // Export at 4× density. The unaligned delivery width also exercises padded NV12 rows:
    // white subpixels test resolution, colored quadrants test both chroma channels and rows.
    const detail = `export const composition = { width: 66, height: 32, fps: 2, duration: 1 };
export default function Scene() {
  return <View style={{ width: 66, height: 32, background: "black" }}>
    {Array.from({ length: 64 }, (_, i) => <View style={{ position: "absolute", left: i / 2, top: 0, width: 0.25, height: 32, background: "white" }} />)}
    {["red", "blue", "lime", "magenta"].map((background, i) => <View style={{ position: "absolute", left: 32 + i % 2 * 17, top: Math.floor(i / 2) * 16, width: 17, height: 16, background }} />)}
  </View>;
}`;
    await Bun.write(join(out, "detail.motion.tsx"), detail);
    const detailed = await evaluate(
      'exported("/out/detail.motion.tsx", { width: 264, height: 128, videoBitrate: 20000000 })',
    );
    await Bun.write(join(out, "detail.mp4"), await (await fetch(detailed.url)).arrayBuffer());
    await Bun.write(
      join(out, "detail-video.motion.tsx"),
      `export const composition = { width: 264, height: 128, fps: 2, duration: 1 };
export const controls = { resources: { video: resource({ kind: "video", default: "detail.mp4" }) } };
export default function Scene() { return <Video src="resource://video" style={{ width: 264, height: 128 }} />; }`,
    );
    run([
      "motion",
      "render",
      join(out, "detail.motion.tsx"),
      "--output-size",
      "264x128",
      "-o",
      join(out, "detail.png"),
    ]);
    run([
      "motion",
      "render",
      join(out, "detail-video.motion.tsx"),
      "--decoder",
      "ffmpeg",
      "-o",
      join(out, "detail-video.png"),
    ]);
    const detailDiff = await evaluate('similar("/out/detail.png", "/out/detail-video.png")');
    console.log(`resized export detail: mean ${detailDiff.mean.toFixed(2)} apart`);
    if (detailDiff.mean > 4) failed.push(`resized export pixels differ: ${detailDiff.mean}`);
    for (const size of [{ width: 7680, height: 4322 }, { width: 0 }, { width: 127, height: 72 }]) {
      const rejected = await evaluate(
        `exported("/out/detail.motion.tsx", ${JSON.stringify(size)}).then(() => false, () => true)`,
      );
      if (!rejected) failed.push(`export accepted invalid size ${JSON.stringify(size)}`);
    }
    // The frames: the export of the edit shown by a scene, and the edit drawn.
    await Bun.write(join(out, "shown.motion.tsx"), SHOWN);
    run([
      "motion",
      "render",
      join(out, "shown.motion.tsx"),
      "--frame",
      "45",
      "--decoder",
      "ffmpeg",
      "-o",
      join(out, "shown.png"),
    ]);
    run([
      "timeline",
      "render",
      "examples/edit.timeline.json",
      "--frame",
      "45",
      "-o",
      join(out, "edit.png"),
    ]);
    const frame = await evaluate(`similar("/out/shown.png", "/out/edit.png")`);
    console.log(`export frame 45: most ${frame.most}, mean ${frame.mean.toFixed(2)} apart`);
    if (frame.mean > 1.5) {
      failed.push(`the exported edit's frame 45 is ${frame.mean} apart on average`);
    }
    // <valle-player>.
    await Bun.write(join(out, "element.motion.tsx"), ELEMENT);
    run([
      "motion",
      "render",
      join(out, "element.motion.tsx"),
      "--frame",
      "30",
      "-o",
      join(out, "element.png"),
    ]);
    const element = await evaluate(`element("/out/element.motion.tsx", "/out/element.png")`);
    console.log(`<valle-player>: ${JSON.stringify(element)}`);
    if (
      !element.playing ||
      !(element.at > 0) ||
      !element.muted ||
      !element.loop ||
      element.most > STEP ||
      !element.closed
    ) {
      failed.push(`<valle-player>: ${JSON.stringify(element)}`);
    }
    // The sound: the export's, as a timeline's clip, and the mix.
    await Bun.write(join(out, "listen.timeline.json"), JSON.stringify(LISTEN));
    run(["timeline", "render", join(out, "listen.timeline.json"), "-o", join(out, "listen.wav")]);
    const heardBack = aligned(
      await samples(join(out, "listen.wav")),
      await samples(join(out, "mix.wav")),
    );
    console.log(
      `export sound: ${heardBack.lag} samples late, ${heardBack.rms.toFixed(4)} apart (RMS)`,
    );
    if (Math.abs(heardBack.lag) > 0 || heardBack.rms > 0.01) {
      failed.push(`the exported sound is ${heardBack.lag} samples late, ${heardBack.rms} apart`);
    }
  } finally {
    await Promise.all([browser.close(), server.stop(true)]);
  }
  if (failed.length > 0) {
    throw new Error(
      `the browser's frames are not the native ones (<n>.png beside <n>.browser.png or <n>.screen.png in ${out}):\n${failed.join("\n")}`,
    );
  }
  await rm(out, { recursive: true, force: true });
}

await main();
