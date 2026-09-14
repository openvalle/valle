import { describe, expect, test } from "bun:test";
import { assetSuppliesAudio } from "./audio-prefetch.ts";

describe("audio prefetch admission", () => {
  test("video containers may supply an audio stream", () => {
    expect(assetSuppliesAudio("audio")).toBe(true);
    expect(assetSuppliesAudio("video")).toBe(true);
    expect(assetSuppliesAudio("image")).toBe(false);
    expect(assetSuppliesAudio("font")).toBe(false);
  });
});
