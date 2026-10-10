import { DRAW_PROGRAM_ABI } from "../../generated/packed-abi.ts";
import type { PathWire } from "./draw-program.ts";

interface Cached<T> { value: T; snapshot: Uint8Array; bytes: number }
interface CachedArena extends Cached<PathWire[]> { key: number }
const verbs = ["moveTo", "lineTo", "quadTo", "cubicTo", "close"];
const pointsPerVerb = [1, 1, 2, 3, 0];

/** Exact binary arenas and contours. Hash matches always require equality with owned bytes. */
export class PathArenaCache {
  private arenas = new Map<number, CachedArena>();
  private nextArenaId = 0;
  private contours = new Map<number, Cached<PathWire>>();
  private arenaBytes = 0;
  private contourBytes = 0;
  hits = 0;
  misses = 0;
  contourHits = 0;
  contourMisses = 0;
  constructor(private readonly maximumBytes = 64 * 1024 * 1024) {}

  decode(bytes: Uint8Array, count: number): PathWire[] {
    if (!Number.isSafeInteger(count) || count < 0 || count > DRAW_PROGRAM_ABI.maximumPaths || count > bytes.length / DRAW_PROGRAM_ABI.pathHeaderBytes)
      throw new Error("path count exceeds the arena");
    const key = arenaKey(bytes);
    // The arena cache has at most eight entries. A sampled index avoids scanning megabytes just
    // to locate a candidate; full owned-byte equality remains the only proof of a cache hit.
    // Keep colliding candidates independently so equal-length arenas cannot evict each other
    // solely because the index samples missed a changed contour.
    for (const [id, cached] of Array.from(this.arenas).reverse()) {
      if (cached.key === key && cached.value.length === count && equal(bytes, cached.snapshot)) {
        this.hits++; this.arenas.delete(id); this.arenas.set(id, cached); return cached.value;
      }
    }
    this.misses++;
    const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength), paths: PathWire[] = [];
    let at = 0, totalVerbs = 0, totalPoints = 0;
    for (let index = 0; index < count; index++) {
      const start = at;
      if (bytes.length - at < DRAW_PROGRAM_ABI.pathHeaderBytes) throw new Error("truncated path header");
      const verbCount = view.getUint32(at, true), pointCount = view.getUint32(at + 4, true);
      totalVerbs += verbCount; totalPoints += pointCount;
      if (totalVerbs > DRAW_PROGRAM_ABI.maximumPathVerbs || totalPoints > DRAW_PROGRAM_ABI.maximumPathPoints)
        throw new Error("path verb or point budget exceeded");
      at += DRAW_PROGRAM_ABI.pathHeaderBytes;
      const end = at + verbCount + pointCount * DRAW_PROGRAM_ABI.pathPointBytes;
      if (end > bytes.length) throw new Error("path row exceeds its section");
      const row = bytes.subarray(start, end), rowKey = hash(row), prior = this.contours.get(rowKey);
      let path: PathWire;
      if (prior && equal(row, prior.snapshot)) {
        this.contourHits++; this.contours.delete(rowKey); this.contours.set(rowKey, prior); path = prior.value;
      } else {
        this.contourMisses++;
        const codes = bytes.subarray(at, at + verbCount);
        const names: string[] = []; let expectedPoints = 0;
        for (const code of codes) {
          if (code >= verbs.length) throw new Error("unknown path verb");
          names.push(verbs[code]!); expectedPoints += pointsPerVerb[code]!;
        }
        if (expectedPoints !== pointCount) throw new Error("path point count does not match its verbs");
        const points: Array<[number, number]> = [];
        let cursor = at + verbCount;
        for (let point = 0; point < pointCount; point++, cursor += DRAW_PROGRAM_ABI.pathPointBytes) {
          const x = view.getFloat64(cursor, true), y = view.getFloat64(cursor + 8, true);
          if (!Number.isFinite(x) || !Number.isFinite(y)) throw new Error("path coordinates must be finite");
          points.push(Object.freeze([x, y]) as [number, number]);
        }
        path = { verbs: names, points }; Object.freeze(names); Object.freeze(points); Object.freeze(path);
        const cost = 128 + row.length + points.length * 64 + names.length * 32;
        if (cost <= this.maximumBytes / 2) {
          const replaced = this.contours.get(rowKey);
          if (replaced) { this.contourBytes -= replaced.bytes; this.contours.delete(rowKey); }
          while (this.contours.size >= 4096 || this.contourBytes + cost > this.maximumBytes / 2) {
            const oldest = this.contours.entries().next().value;
            if (!oldest) break;
            this.contourBytes -= oldest[1].bytes; this.contours.delete(oldest[0]);
          }
          this.contours.set(rowKey, { value: path, snapshot: row.slice(), bytes: cost }); this.contourBytes += cost;
        }
      }
      paths.push(path); at = end;
    }
    if (at !== bytes.length) throw new Error("trailing path arena bytes");
    Object.freeze(paths);
    let cost = 128 + bytes.length;
    for (const path of paths) cost += 128 + path.points.length * 64 + path.verbs.length * 32;
    if (cost <= this.maximumBytes / 2) {
      while (this.arenas.size >= 8 || this.arenaBytes + cost > this.maximumBytes / 2) {
        const oldest = this.arenas.entries().next().value;
        if (!oldest) break;
        this.arenaBytes -= oldest[1].bytes; this.arenas.delete(oldest[0]);
      }
      this.arenas.set(this.nextArenaId++, { key, value: paths, snapshot: bytes.slice(), bytes: cost }); this.arenaBytes += cost;
    }
    return paths;
  }

  clear(): void { this.arenas.clear(); this.contours.clear(); this.arenaBytes = 0; this.contourBytes = 0; this.nextArenaId = 0; }
}

function arenaKey(bytes: Uint8Array): number {
  if (bytes.length < 64 * 1024) return hash(bytes);
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  let value = Math.imul(2166136261 ^ bytes.length, 16777619);
  for (const start of [0, Math.floor((bytes.length - 64) / 2), bytes.length - 64]) {
    for (let offset = 0; offset < 64; offset += 4)
      value = Math.imul(value ^ view.getUint32(start + offset, true), 16777619);
  }
  return value >>> 0;
}

function hash(bytes: Uint8Array): number {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  let value = 2166136261, at = 0;
  for (; at + 16 <= bytes.length; at += 16) {
    value = Math.imul(value ^ view.getUint32(at, true), 16777619);
    value = Math.imul(value ^ view.getUint32(at + 4, true), 16777619);
    value = Math.imul(value ^ view.getUint32(at + 8, true), 16777619);
    value = Math.imul(value ^ view.getUint32(at + 12, true), 16777619);
  }
  for (; at + 4 <= bytes.length; at += 4) value = Math.imul(value ^ view.getUint32(at, true), 16777619);
  for (; at < bytes.length; at++) value = Math.imul(value ^ bytes[at]!, 16777619);
  return value >>> 0;
}

function equal(a: Uint8Array, b: Uint8Array): boolean {
  if (a.length !== b.length) return false;
  const av = new DataView(a.buffer, a.byteOffset, a.byteLength), bv = new DataView(b.buffer, b.byteOffset, b.byteLength);
  let at = 0;
  for (; at + 16 <= a.length; at += 16) {
    if (av.getUint32(at, true) !== bv.getUint32(at, true)
      || av.getUint32(at + 4, true) !== bv.getUint32(at + 4, true)
      || av.getUint32(at + 8, true) !== bv.getUint32(at + 8, true)
      || av.getUint32(at + 12, true) !== bv.getUint32(at + 12, true)) return false;
  }
  for (; at + 4 <= a.length; at += 4) if (av.getUint32(at, true) !== bv.getUint32(at, true)) return false;
  for (; at < a.length; at++) if (a[at] !== b[at]) return false;
  return true;
}
