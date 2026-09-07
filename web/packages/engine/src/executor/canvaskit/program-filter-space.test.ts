import { describe, expect, test } from "bun:test";

import { programFilterInDeviceSpace } from "./executor.ts";

describe("scheduled DrawProgram filter space", () => {
  test("maps caption similarity transforms into device pixels", () => {
    expect(programFilterInDeviceSpace(
      {
        kind: "dropShadow",
        offset: [2, 3],
        sigmaX: 5,
        sigmaY: 5,
        color: { red: 0, green: 0, blue: 0, alpha: 1 },
      },
      [0, -2, 8, 2, 0, 9, 0, 0, 1],
    )).toEqual({
      kind: "dropShadow",
      offset: [-6, 4],
      sigmaX: 10,
      sigmaY: 10,
      color: { red: 0, green: 0, blue: 0, alpha: 1 },
    });

    expect(programFilterInDeviceSpace(
      { kind: "blur", sigmaX: 3, sigmaY: 3 },
      [-2, 0, 0, 0, 2, 0, 0, 0, 1],
    )).toEqual({ kind: "blur", sigmaX: 6, sigmaY: 6 });
  });

  test("accepts an exactly representable axis-aligned nonuniform mapping", () => {
    expect(programFilterInDeviceSpace(
      { kind: "blur", sigmaX: 5, sigmaY: 5 },
      [2, 0, 0, 0, 3, 0, 0, 0, 1],
    )).toEqual({ kind: "blur", sigmaX: 10, sigmaY: 15 });
  });

  test("rejects transform classes the scheduled axis-aligned kernel cannot represent", () => {
    const blur = { kind: "blur", sigmaX: 5, sigmaY: 5 };
    for (const matrix of [
      [1, 1, 0, 0, 1, 0, 0, 0, 1],
      [1, 0, 0, 0, 1, 0, 0.01, 0, 1],
      [1, 0, 1e12, 0, 1, 1e12, 1e-6, 0, 1],
      [Number.MAX_VALUE, 0, 0, 0, Number.MAX_VALUE, 0, 0, 0, 1],
    ]) {
      expect(() => programFilterInDeviceSpace(blur, matrix)).toThrow();
    }
  });
});
