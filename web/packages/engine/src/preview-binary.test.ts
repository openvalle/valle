import { expect, test } from "bun:test";
import { preparePreviewPackageWithWasm, resourceByteRange } from "./compiler.ts";

test("preview WASM boundary keeps font bytes out of JSON and reuses shared buffers", () => {
  const resourceBytes = new Uint8Array([1, 2, 3]);
  const font = new Uint8Array([0, 255, 127, 128]);
  Object.defineProperty(font, "toJSON", {
    value() {
      throw new Error("font bytes must not enter JSON");
    },
  });
  const output = new Uint8Array([9, 8]);
  const result = preparePreviewPackageWithWasm(
    {
      prepare_preview_package(json, bytes) {
        const input = JSON.parse(json);
        expect(json).not.toContain("Base64");
        expect(input.resourceBytes).toBeUndefined();
        expect(bytes).toEqual(new Uint8Array([1, 2, 3, 0, 255, 127, 128]));
        expect(resourceByteRange(input.motionInstances[0].fonts[0].bytes, bytes)).toEqual(font);
        expect(input.motionInstances[1].fonts[0].bytes).toEqual(
          input.motionInstances[0].fonts[0].bytes,
        );
        return [JSON.stringify({ timelineJson: "{}" }), output];
      },
    },
    {
      authorTimeline: { canvas: { width: 32, height: 32, fps: 4 }, tracks: {} },
      resourceBytes,
      motionInstances: [0, 1].map((index) => ({
        clipPath: `/tracks/visual/${index}/clips/0`,
        artifact: {},
        artifactDigest: "sha256:test",
        fonts: [{ bytes: font, role: "font" }],
      })),
    },
  );
  expect(result.resourceBytes).toBe(output);
  expect(result.timelineJson).toBe("{}");
});

test("resource byte ranges reject invalid, overflowing and out-of-bounds metadata", () => {
  const bytes = new Uint8Array([10, 20, 30]);
  expect([...resourceByteRange({ offset: 1, length: 2 }, bytes)]).toEqual([20, 30]);
  for (const range of [
    null,
    { offset: -1, length: 1 },
    { offset: 0, length: 4 },
    { offset: 2, length: 2 },
    { offset: 0.5, length: 1 },
    { offset: Number.MAX_SAFE_INTEGER, length: 1 },
  ])
    expect(() => resourceByteRange(range, bytes)).toThrow();
});
