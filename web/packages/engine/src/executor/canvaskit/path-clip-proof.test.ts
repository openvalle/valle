import { expect, test } from "bun:test";
import type { PathWire } from "./draw-program.ts";
import { clipPaintRectangleCoversOutline, paintRectangleBounds } from "./path-clip-proof.ts";

const box: PathWire = {
  verbs: ["moveTo", "lineTo", "lineTo", "lineTo", "close"],
  points: [
    [0, 0],
    [100, 0],
    [100, 100],
    [0, 100],
  ],
};
const identity = [1, 0, 0, 0, 1, 0, 0, 0, 1];
test("finite paint coverage preserves margins under affine and projective transforms", () => {
  const outline: PathWire = {
    verbs: ["moveTo", "quadTo", "lineTo", "close"],
    points: [
      [20, 20],
      [50, 5],
      [80, 30],
      [40, 70],
    ],
  };
  for (const matrix of [
    identity,
    [0.6, 0.2, 11, -0.1, 0.8, 5, 0, 0, 1],
    [1, 0, 0, 0, 1, 0, 0.001, 0.001, 1],
  ])
    expect(clipPaintRectangleCoversOutline(box, outline, matrix)).toBe(true);
  expect(clipPaintRectangleCoversOutline(box, outline, [0.02, 0, 0, 0, 0.02, 0, 0, 0, 1])).toBe(
    false,
  );
  expect(clipPaintRectangleCoversOutline(box, outline, [1, 0, 0, 0, 1, 0, 0, 0, -1])).toBe(false);
  expect(
    clipPaintRectangleCoversOutline(
      box,
      {
        ...outline,
        points: [
          [20, 20],
          [-1, 5],
          [80, 30],
          [40, 70],
        ],
      },
      identity,
    ),
  ).toBe(false);
});

test("rectangle recognition rejects crossed, repeated and curved contours", () => {
  expect(paintRectangleBounds(box)).toEqual({ x: 0, y: 0, width: 100, height: 100 });
  expect(paintRectangleBounds({ ...box, points: [...box.points].reverse() })).not.toBeNull();
  expect(
    paintRectangleBounds({
      ...box,
      points: [
        [0, 0],
        [100, 100],
        [0, 100],
        [100, 0],
      ],
    }),
  ).toBeNull();
  expect(
    paintRectangleBounds({
      ...box,
      points: [
        [0, 0],
        [100, 0],
        [100, 0],
        [0, 100],
      ],
    }),
  ).toBeNull();
  expect(
    paintRectangleBounds({ ...box, verbs: ["moveTo", "quadTo", "lineTo", "close"] }),
  ).toBeNull();
});

test("rectangle memoization requires immutable coordinates and owns an immutable bound", () => {
  const mutable: PathWire = {
    verbs: [...box.verbs],
    points: box.points.map((p) => [...p] as [number, number]),
  };
  expect(paintRectangleBounds(mutable)?.width).toBe(100);
  mutable.points[1]![0] = 120;
  mutable.points[2]![0] = 120;
  expect(paintRectangleBounds(mutable)?.width).toBe(120);
  for (const point of mutable.points) Object.freeze(point);
  Object.freeze(mutable.points);
  Object.freeze(mutable.verbs);
  Object.freeze(mutable);
  const bounds = paintRectangleBounds(mutable);
  expect(paintRectangleBounds(mutable)).toBe(bounds);
  expect(Object.isFrozen(bounds)).toBe(true);
});
