import type { PathWire, RectWire } from "./draw-program.ts";

type Point = readonly [number, number];
const immutableRectangles = new WeakMap<PathWire, RectWire | null>();

/** The finite rectangle used by a glyph paint; preserve its exact clipping domain. */
export function paintRectangleBounds(path: PathWire): RectWire | null {
  if (immutableRectangles.has(path)) return immutableRectangles.get(path)!;
  const result = rectangleBounds(path);
  if (
    Object.isFrozen(path) &&
    Object.isFrozen(path.verbs) &&
    Object.isFrozen(path.points) &&
    path.points.every(Object.isFrozen)
  ) {
    if (result) Object.freeze(result);
    immutableRectangles.set(path, result);
  }
  return result;
}

function rectangleBounds(path: PathWire): RectWire | null {
  if (path.verbs.join(",") !== "moveTo,lineTo,lineTo,lineTo,close" || path.points.length !== 4)
    return null;
  const xs = new Set(path.points.map((p) => p[0])),
    ys = new Set(path.points.map((p) => p[1]));
  if (xs.size !== 2 || ys.size !== 2) return null;
  for (let at = 0; at < 4; at++) {
    const a = path.points[at]!,
      b = path.points[(at + 1) % 4]!;
    if ((a[0] === b[0]) === (a[1] === b[1])) return null;
  }
  const x = Math.min(...xs),
    y = Math.min(...ys);
  return { x, y, width: Math.max(...xs) - x, height: Math.max(...ys) - y };
}

function project(point: Point, matrix: readonly number[]): Point | null {
  const w = matrix[6]! * point[0] + matrix[7]! * point[1] + matrix[8]!;
  if (!Number.isFinite(w) || w <= 1e-9) return null;
  const x = (matrix[0]! * point[0] + matrix[1]! * point[1] + matrix[2]!) / w;
  const y = (matrix[3]! * point[0] + matrix[4]! * point[1] + matrix[5]!) / w;
  return Number.isFinite(x) && Number.isFinite(y) ? [x, y] : null;
}

/** A finite paint rectangle can fill an outline directly when its edges cannot affect coverage. */
export function clipPaintRectangleCoversOutline(
  rectangle: PathWire,
  outline: PathWire,
  matrix: readonly number[],
): boolean {
  if (
    rectangle.verbs.length !== 5 ||
    rectangle.points.length !== 4 ||
    rectangle.verbs.join(",") !== "moveTo,lineTo,lineTo,lineTo,close"
  )
    return false;
  if (!paintRectangleBounds(rectangle)) return false;
  const polygon = rectangle.points;
  const vertices = polygon.map((point) => project(point, matrix));
  if (vertices.some((point) => point === null)) return false;
  const points = vertices as Point[];
  const area = points.reduce((sum, point, at) => {
    const next = points[(at + 1) % points.length]!;
    return sum + point[0] * next[1] - point[1] * next[0];
  }, 0);
  if (!Number.isFinite(area) || Math.abs(area) < 1e-8) return false;
  const direction = Math.sign(area);
  const edges = points.map((a, at) => {
    const b = points[(at + 1) % points.length]!,
      dx = b[0] - a[0],
      dy = b[1] - a[1];
    return { a, dx, dy, margin: 2 * Math.hypot(dx, dy) };
  });
  for (const point of outline.points) {
    const device = project(point, matrix);
    if (!device) return false;
    for (const { a, dx, dy, margin } of edges) {
      if (direction * (dx * (device[1] - a[1]) - dy * (device[0] - a[0])) <= margin) return false;
    }
  }
  return true;
}
