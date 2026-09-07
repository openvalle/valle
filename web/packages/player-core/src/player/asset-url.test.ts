import { expect, test } from "bun:test";

import { resolveAssetUrl } from "./controller.ts";

test("root-relative assets stay rooted at the current origin", () => {
  expect(resolveAssetUrl("/motion-assets/product", "/assets/", null)).toBe("/motion-assets/product");
});

test("relative assets use the configured asset base", () => {
  expect(resolveAssetUrl("folder/cover image.png", "/assets/", null)).toBe(
    "/assets/folder/cover%20image.png",
  );
});

test("remote assets can use the same-origin proxy", () => {
  expect(resolveAssetUrl("https://cdn.example.test/cover.png", "/assets/", "/proxy")).toBe(
    "/proxy?url=https%3A%2F%2Fcdn.example.test%2Fcover.png",
  );
});
