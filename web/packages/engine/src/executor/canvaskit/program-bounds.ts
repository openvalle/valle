import type { DrawProgramWire, PathWire, RectWire } from "./draw-program.ts";

type Wire = Record<string, any>;
// null is empty; undefined means that a safe bound cannot be derived.
type Bounds = RectWire | null | undefined;

function finiteRect(value: RectWire): Bounds {
  return [value.x, value.y, value.width, value.height].every(Number.isFinite)
    ? value.width > 0 && value.height > 0 ? value : null
    : undefined;
}

function edges(left: number, top: number, right: number, bottom: number): Bounds {
  return finiteRect({ x: left, y: top, width: right - left, height: bottom - top });
}

function union(left: Bounds, right: Bounds): Bounds {
  if (left === undefined || right === undefined) return undefined;
  if (left === null) return right;
  if (right === null) return left;
  return edges(
    Math.min(left.x, right.x), Math.min(left.y, right.y),
    Math.max(left.x + left.width, right.x + right.width),
    Math.max(left.y + left.height, right.y + right.height),
  );
}

function intersect(left: Bounds, right: Bounds): Bounds {
  if (left === null || right === null) return null;
  if (left === undefined || right === undefined) return undefined;
  return edges(
    Math.max(left.x, right.x), Math.max(left.y, right.y),
    Math.min(left.x + left.width, right.x + right.width),
    Math.min(left.y + left.height, right.y + right.height),
  );
}

function outset(bounds: Bounds, amount: number): Bounds {
  if (bounds == null || !Number.isFinite(amount)) return bounds === null ? null : undefined;
  return edges(
    bounds.x - amount, bounds.y - amount,
    bounds.x + bounds.width + amount, bounds.y + bounds.height + amount,
  );
}

function pathBounds(path: PathWire | undefined): Bounds {
  if (!path) return undefined;
  if (path.points.length === 0) return null;
  let left = path.points[0]![0], top = path.points[0]![1];
  let right = left, bottom = top;
  for (let index = 1; index < path.points.length; index += 1) {
    const [x, y] = path.points[index]!;
    left = Math.min(left, x); top = Math.min(top, y);
    right = Math.max(right, x); bottom = Math.max(bottom, y);
  }
  // Retain zero-area path hulls until their stroke has been added.
  return { x: left, y: top, width: right - left, height: bottom - top };
}

function strokeOutset(stroke: Wire): number {
  const joinScale = stroke.join === "miter" ? stroke.miterLimit : 1;
  // DrawProgram derives ordinary Path bounds with a f32 inset.
  return Math.fround(stroke.width * 0.5 * joinScale);
}

function batchStrokeOutset(width: number, pathStyle: Wire | null): number {
  const joinScale = pathStyle == null || pathStyle.join === "miter"
    ? pathStyle?.miterLimit ?? 4 : 1;
  // The batch geometry and Native opacity layer multiply their f32 values in this order.
  return Math.fround(Math.fround(width * 0.5) * joinScale);
}

function mapAffine(bounds: Bounds, a: number, b: number, c: number, d: number, e: number, f: number): Bounds {
  if (bounds == null) return bounds;
  const ox = e + a * bounds.x + c * bounds.y;
  const oy = f + b * bounds.x + d * bounds.y;
  const ex = a * bounds.width, cx = c * bounds.height;
  const ey = b * bounds.width, cy = d * bounds.height;
  return edges(
    ox + Math.min(ex, 0) + Math.min(cx, 0), oy + Math.min(ey, 0) + Math.min(cy, 0),
    ox + Math.max(ex, 0) + Math.max(cx, 0), oy + Math.max(ey, 0) + Math.max(cy, 0),
  );
}

function mapMatrix(bounds: Bounds, matrix: number[]): Bounds {
  if (bounds == null) return bounds;
  const points = [
    [bounds.x, bounds.y], [bounds.x + bounds.width, bounds.y],
    [bounds.x + bounds.width, bounds.y + bounds.height], [bounds.x, bounds.y + bounds.height],
  ];
  const denominators = points.map(([x, y]) => matrix[6]! * x! + matrix[7]! * y! + matrix[8]!);
  if (denominators.some((w) => !Number.isFinite(w) || Math.abs(w) <= Number.EPSILON)
    || denominators.some((w) => Math.sign(w) !== Math.sign(denominators[0]!))) return undefined;
  const mapped = points.map(([x, y], index) => [
    (matrix[0]! * x! + matrix[1]! * y! + matrix[2]!) / denominators[index]!,
    (matrix[3]! * x! + matrix[4]! * y! + matrix[5]!) / denominators[index]!,
  ]);
  return edges(
    Math.min(...mapped.map((point) => point[0]!)), Math.min(...mapped.map((point) => point[1]!)),
    Math.max(...mapped.map((point) => point[0]!)), Math.max(...mapped.map((point) => point[1]!)),
  );
}

function instanceLocalBounds(program: DrawProgramWire, shape: Wire): Bounds {
  if (shape.kind === "circle") return { x: -1, y: -1, width: 2, height: 2 };
  if (shape.kind === "rect") return { x: 0, y: 0, width: 1, height: 1 };
  if (shape.kind === "image") return { x: 0, y: 0, width: 1, height: 1 };
  if (shape.kind === "roundRect") return finiteRect(shape.value.rect as RectWire);
  if (shape.kind === "path") return pathBounds(program.paths[shape.value as number]);
  return undefined;
}

/** Bounds passed before the per-row transform, matching Native's save_layer_alpha_f. */
export function instanceOpacityBounds(
  program: DrawProgramWire, shape: Wire, pathStyle: Wire | null,
  strokeWidth: number, affine: [number, number, number, number, number, number],
): RectWire | null {
  let local: Bounds = null;
  if (shape.kind === "path") {
    local = outset(instanceLocalBounds(program, shape), batchStrokeOutset(strokeWidth, pathStyle));
  } else if (strokeWidth === 0 && (shape.kind === "rect" || shape.kind === "roundRect" || shape.kind === "image")) {
    local = instanceLocalBounds(program, shape);
  }
  // Native leaves other shapes unbounded. A missing bound has the same fallback here.
  const result = mapMatrix(local, [affine[0], affine[2], affine[4], affine[1], affine[3], affine[5], 0, 0, 1]);
  return result ?? null;
}

function batchBounds(program: DrawProgramWire, node: Wire): Bounds {
  const local = instanceLocalBounds(program, node.shape as Wire);
  if (local === undefined) return undefined;
  const { start, count } = node.instances as { start: number; count: number };
  if (!Number.isSafeInteger(start) || !Number.isSafeInteger(count) || start + count > program.batchInstances.count) return undefined;
  const view = program.batchInstances.view;
  const tableCount = program.batchInstances.count;
  let result: Bounds = null;
  for (let row = start; row < start + count; row += 1) {
    const offset = row * 48;
    const width = view.getFloat32(tableCount * 68 + row * 4, true);
    const footprint = width > 0 ? outset(local, batchStrokeOutset(width, node.pathStyle)) : local;
    result = union(result, mapAffine(footprint,
      view.getFloat64(offset, true), view.getFloat64(offset + 8, true),
      view.getFloat64(offset + 16, true), view.getFloat64(offset + 24, true),
      view.getFloat64(offset + 32, true), view.getFloat64(offset + 40, true)));
  }
  return result;
}

function clipBounds(program: DrawProgramWire, clip: Wire): Bounds {
  if (clip.kind === "rect") return finiteRect(clip.value as RectWire);
  if (clip.kind === "roundRect") return finiteRect(clip.value.rect as RectWire);
  if (clip.kind === "path") return pathBounds(program.paths[clip.value.path as number]);
  return undefined;
}

/** DrawProgram's raster-tree output bound in the node's parent coordinate system. */
export function programNodeBounds(
  program: DrawProgramWire, id: number, cache: Map<number, Bounds>, depth = 0,
): Bounds {
  if (cache.has(id)) return cache.get(id);
  if (depth > 128) return undefined;
  const tagged = program.nodes[id];
  if (!tagged || typeof tagged.value !== "object" || tagged.value === null) return undefined;
  const node = tagged.value as Wire;
  let result: Bounds;
  switch (tagged.kind) {
    case "path": {
      result = node.fill == null && node.stroke == null ? null : pathBounds(program.paths[node.path as number]);
      if (node.stroke != null) result = outset(result, strokeOutset(node.stroke));
      break;
    }
    case "instanceBatch": result = batchBounds(program, node); break;
    case "image": result = node.opacity === 0 ? null : finiteRect(node.dst as RectWire); break;
    case "glyphRun": {
      result = node.outline == null ? finiteRect(node.bounds as RectWire) : pathBounds(program.paths[node.outline as number]);
      if (node.stroke != null) result = outset(result, strokeOutset(node.stroke));
      break;
    }
    case "shadow": {
      if (node.color.alpha === 0) { result = null; break; }
      const shape = node.shape.rect as RectWire;
      if (node.inset) { result = finiteRect(shape); break; }
      const spread = Math.max(node.spread, 0);
      result = edges(
        shape.x + node.offset[0] - spread - 3 * node.sigmaX,
        shape.y + node.offset[1] - spread - 3 * node.sigmaY,
        shape.x + shape.width + node.offset[0] + spread + 3 * node.sigmaX,
        shape.y + shape.height + node.offset[1] + spread + 3 * node.sigmaY,
      );
      break;
    }
    case "runtimeShader": case "scene3d": result = finiteRect(node.bounds as RectWire); break;
    case "group": {
      // Other group effects have their own scheduled pass and do not enter RasterTree.
      if (node.mask != null || node.backdrop != null || node.shader != null || node.transition != null
        || node.glass != null || node.glassForeground != null || node.filters.length !== 0) return undefined;
      result = null;
      for (const child of node.children as number[]) result = union(result, programNodeBounds(program, child, cache, depth + 1));
      if (node.clip != null) result = intersect(result, clipBounds(program, node.clip));
      if (node.opacity === 0) result = null;
      result = mapMatrix(result, node.transform as number[]);
      break;
    }
    default: return undefined;
  }
  cache.set(id, result);
  return result;
}
