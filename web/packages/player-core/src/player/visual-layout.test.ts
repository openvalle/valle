import { describe, expect, test } from "bun:test";
import { assertCanvaskitImportLayout, canvaskitImportLayout } from "./visual-layout.ts";

describe("CanvasKit import layout", () => {
  test("reads packed ABI snake_case pixel_layout", () => {
    expect(canvaskitImportLayout({ pixel_layout: "rgba8" })).toBe("rgba8");
    expect(canvaskitImportLayout({ pixel_layout: "rgba16Float" })).toBe("rgba16Float");
  });

  test("does not treat a missing field as an empty supported layout", () => {
    expect(canvaskitImportLayout({})).toBe("");
    expect(() => assertCanvaskitImportLayout({})).toThrow("missing pixel_layout");
  });

  test("rejects host layouts CanvasKit cannot import", () => {
    expect(() => assertCanvaskitImportLayout({ pixel_layout: "nv12" })).toThrow("nv12");
  });
});
