import type { SkPicture } from "canvaskit-wasm";
import type { DrawProgramWire, PathWire } from "./draw-program.ts";

export interface RasterTreeSignature {
  key: string;
  leaves: number;
  bytes: number;
}

/** Whole immutable vector scopes. World matrices and GPU pixels are never captured here. */
export class RasterTreeCache {
  private entries = new Map<string, { picture: SkPicture; bytes: number }>();
  private witnesses = new Map<string, number>();
  private witnessBytes = 0;
  private bytes = 0;
  private pathIds = new WeakMap<PathWire, number>();
  private nextPathId = 1;
  hits = 0;
  misses = 0;
  recordings = 0;
  constructor(private readonly maximumBytes = 64 * 1024 * 1024) {}

  signature(
    draw: DrawProgramWire,
    id: number,
    memo: Map<number, RasterTreeSignature | null>,
    depth = 0,
  ): RasterTreeSignature | null {
    if (memo.has(id)) return memo.get(id)!;
    if (depth > 128) return null;
    const node = draw.nodes[id],
      value = node?.value as Record<string, any> | undefined;
    if (!value || typeof value !== "object") return null;
    const path = (id: number): string | null => {
      const wire = draw.paths[id];
      if (
        !wire ||
        !Object.isFrozen(wire) ||
        !Object.isFrozen(wire.points) ||
        !Object.isFrozen(wire.verbs)
      )
        return null;
      let identity = this.pathIds.get(wire);
      if (identity === undefined) {
        if (!wire.points.every(Object.isFrozen)) return null;
        this.pathIds.set(wire, (identity = this.nextPathId++));
      }
      return String(identity);
    };
    const paint = (id: number | null) => (id == null ? null : draw.paints[id]);
    let result: RasterTreeSignature | null = null;
    if (node!.kind === "path" || (node!.kind === "glyphRun" && value.outline != null)) {
      const pathIndex = node!.kind === "path" ? value.path : value.outline,
        identity = path(pathIndex);
      if (identity !== null) {
        result = {
          key: JSON.stringify([
            "path",
            identity,
            value.fillRule ?? "nonZero",
            paint(node!.kind === "path" ? value.fill : value.paint),
            value.stroke,
            value.stroke == null ? null : paint(value.stroke.paint),
          ]),
          leaves: 1,
          bytes:
            256 +
            draw.paths[pathIndex]!.points.length * 16 +
            draw.paths[pathIndex]!.verbs.length * 8,
        };
      }
    } else if (
      node!.kind === "group" &&
      value.filters?.length === 0 &&
      value.mask == null &&
      value.shader == null &&
      value.transition == null &&
      value.backdrop == null &&
      value.internalBlend === "normal" &&
      Array.isArray(value.children)
    ) {
      let clip = JSON.stringify(value.clip),
        supported = true;
      if (value.clip?.kind === "path") {
        const identity = path(value.clip.value.path);
        if (identity === null) supported = false;
        else clip = JSON.stringify([value.clip, identity]);
      }
      if (supported) {
        const children = value.children.map((child: number) =>
          this.signature(draw, child, memo, depth + 1),
        );
        if (children.every((child: RasterTreeSignature | null) => child !== null)) {
          result = {
            key:
              JSON.stringify(["group", value.transform, value.opacity, clip]) +
              children
                .map((child: RasterTreeSignature) => `${child.key.length}:${child.key}`)
                .join(""),
            leaves: children.reduce(
              (sum: number, child: RasterTreeSignature) => sum + child.leaves,
              0,
            ),
            bytes:
              256 +
              children.reduce((sum: number, child: RasterTreeSignature) => sum + child.bytes, 0),
          };
        }
      }
    }
    memo.set(id, result);
    return result;
  }

  acquire(signature: RasterTreeSignature, record: () => SkPicture): SkPicture | null {
    const key = signature.key,
      cached = this.entries.get(key);
    if (cached) {
      this.hits++;
      this.entries.delete(key);
      this.entries.set(key, cached);
      return cached.picture;
    }
    this.misses++;
    if (signature.leaves < 8 || signature.bytes + key.length * 2 > this.maximumBytes) return null;
    if (!this.witnesses.has(key)) {
      const bytes = key.length * 2;
      while (this.witnesses.size >= 128 || this.witnessBytes + bytes > this.maximumBytes / 8) {
        const oldest = this.witnesses.entries().next().value;
        if (!oldest) break;
        this.witnesses.delete(oldest[0]);
        this.witnessBytes -= oldest[1];
      }
      if (bytes <= this.maximumBytes / 8) {
        this.witnesses.set(key, bytes);
        this.witnessBytes += bytes;
      }
      return null;
    }
    const picture = record();
    this.recordings++;
    const bytes = signature.bytes + key.length * 2 + picture.approximateBytesUsed();
    if (bytes > this.maximumBytes) {
      picture.delete();
      return null;
    }
    while (this.entries.size >= 64 || this.bytes + bytes > this.maximumBytes) {
      const oldest = this.entries.entries().next().value;
      if (!oldest) break;
      oldest[1].picture.delete();
      this.entries.delete(oldest[0]);
      this.bytes -= oldest[1].bytes;
    }
    this.entries.set(key, { picture, bytes });
    this.bytes += bytes;
    return picture;
  }

  dispose(): void {
    for (const entry of this.entries.values()) entry.picture.delete();
    this.entries.clear();
    this.witnesses.clear();
    this.bytes = 0;
    this.witnessBytes = 0;
    this.pathIds = new WeakMap();
    this.nextPathId = 1;
  }
}
