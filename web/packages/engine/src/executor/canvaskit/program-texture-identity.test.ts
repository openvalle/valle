import { describe, expect, test } from "bun:test";

import {
  admitProgramTextures,
  resolveProgramTexture,
  type CanvasKitExternalObject,
} from "./executor.ts";

function visualObject(key: string): CanvasKitExternalObject {
  return {
    key,
    kind: "visual",
    // Admission only needs a concrete, non-null image identity. CanvasKit ownership is outside
    // this pure binding-contract test.
    image: { key } as unknown as NonNullable<CanvasKitExternalObject["image"]>,
  };
}

describe("program texture execution identity", () => {
  test("keeps two producer-local samples of one logical asset distinct", () => {
    const logicalKey = "asset://shared-video";
    const earlyRequirement = {
      key: logicalKey,
      kind: "video",
      colorDomain: "linearRec2020",
      alpha: "premultiplied",
      sampleTimeMicros: 250_000,
    };
    const lateRequirement = {
      ...earlyRequirement,
      sampleTimeMicros: 750_000,
    };
    const earlyObject = visualObject("decoded-frame:250000");
    const lateObject = visualObject("decoded-frame:750000");

    const admitted = admitProgramTextures(
      [earlyRequirement, lateRequirement],
      [
        { key: logicalKey, slot: 1 },
        { key: logicalKey, slot: 2 },
      ],
      new Map([
        [1, earlyObject],
        [2, lateObject],
      ]),
    );

    expect(admitted.size).toBe(2);
    expect(resolveProgramTexture(admitted, lateRequirement)).toBe(lateObject);
    expect(resolveProgramTexture(admitted, earlyRequirement)).toBe(earlyObject);
    expect(() => resolveProgramTexture(admitted, {
      ...earlyRequirement,
      sampleTimeMicros: 500_000,
    })).toThrow("program texture");
    expect(() => admitProgramTextures(
      [{ ...earlyRequirement, sampleTimeMicros: null }],
      [{ key: logicalKey, slot: 1 }],
      new Map([[1, earlyObject]]),
    )).toThrow("video textures must carry sampleTimeMicros");
    expect(() => admitProgramTextures(
      [{ ...earlyRequirement, kind: "image", sampleTimeMicros: 250_000 }],
      [{ key: logicalKey, slot: 1 }],
      new Map([[1, earlyObject]]),
    )).toThrow("only video textures may carry sampleTimeMicros");
  });
});
