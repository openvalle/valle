import { expect, test } from "bun:test";
import { canvasKitGpuCacheBudget } from "./gpu-budget.ts";

const MIB = 1024n * 1024n;
test("GPU cache headroom grows with physical surfaces within the admitted budget", () => {
  expect(canvasKitGpuCacheBudget(1024n * MIB, 80n * MIB)).toBe(256 * Number(MIB));
  expect(canvasKitGpuCacheBudget(1024n * MIB, 256n * MIB)).toBe(512 * Number(MIB));
  expect(canvasKitGpuCacheBudget(1024n * MIB, 400n * MIB)).toBe(832 * Number(MIB));
  expect(canvasKitGpuCacheBudget(1024n * MIB, 700n * MIB)).toBe(1024 * Number(MIB));
  expect(canvasKitGpuCacheBudget(64n * MIB, 80n * MIB)).toBe(64 * Number(MIB));
  expect(canvasKitGpuCacheBudget(1n << 40n, 1n << 40n)).toBe(0xffffffff);
});
