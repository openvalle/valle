import { describe, expect, test } from "bun:test";

import { BrowserValleWebPlayer } from "@valle/player-core/controller";
import { resolvePlayerRuntimeAssets } from "@valle/player-core/runtime-assets";

const runtimeAssets = {
  engine: { glue: "engine/engine.js", wasm: "engine/engine.wasm" },
  canvasKit: {
    base: { glue: "canvaskit/base.js", wasm: "canvaskit/base.wasm" },
    full: { glue: "canvaskit/full.js", wasm: "canvaskit/full.wasm" },
  },
  fonts: { defaultSans: "fonts/NotoSans-Regular.ttf" },
  workers: { productFrame: "workers/product-frame.js" },
};

describe("player runtime asset map", () => {
  test("resolves every executable runtime URL against one explicit base", () => {
    expect(resolvePlayerRuntimeAssets(runtimeAssets, "https://example.test/runtime/")).toEqual({
      engine: {
        glue: "https://example.test/runtime/engine/engine.js",
        wasm: "https://example.test/runtime/engine/engine.wasm",
      },
      canvasKit: {
        base: {
          glue: "https://example.test/runtime/canvaskit/base.js",
          wasm: "https://example.test/runtime/canvaskit/base.wasm",
        },
        full: {
          glue: "https://example.test/runtime/canvaskit/full.js",
          wasm: "https://example.test/runtime/canvaskit/full.wasm",
        },
      },

      fonts: {
        defaultSans: "https://example.test/runtime/fonts/NotoSans-Regular.ttf",
      },
      workers: {
        productFrame: "https://example.test/runtime/workers/product-frame.js",
      },
    });
  });

  test("anchors a same-origin path base before resolving runtime assets", () => {
    const expectedBase = new URL("/runtime/", import.meta.url);
    const resolved = resolvePlayerRuntimeAssets(runtimeAssets, "/runtime/");
    expect(resolved.engine.glue).toBe(new URL("engine/engine.js", expectedBase).href);
    expect(resolved.canvasKit.full.wasm).toBe(new URL("canvaskit/full.wasm", expectedBase).href);
    expect(resolved.fonts.defaultSans).toBe(new URL("fonts/NotoSans-Regular.ttf", expectedBase).href);
    expect(resolved.workers.productFrame).toBe(new URL("workers/product-frame.js", expectedBase).href);
  });

  test("controller rejects incomplete maps instead of falling back to hard-coded paths", () => {
    // Exercise runtime rejection with incomplete options; the type suppression must become an error if the required field is relaxed.
    // @ts-expect-error runtimeAssets is required; verify runtime rejection when omitted.
    expect(() => new BrowserValleWebPlayer({
      fixedPackageManifestJson: "{}",
      timelineJson: "{}",
      resourceManifestJson: "{}",
      verifiedBindingBundleJson: "{}",
    })).toThrow(/runtimeAssets is required/);
    // This function accepts a partial map, so runtime validation is required without a type suppression.
    expect(() => resolvePlayerRuntimeAssets({ engine: {} })).toThrow(/runtimeAssets\.engine\.glue/);
  });

});
