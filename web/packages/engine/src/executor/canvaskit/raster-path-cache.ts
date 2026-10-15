import type { Path } from "canvaskit-wasm";
import type { PathWire } from "./draw-program.ts";

interface Entry { wire: PathWire; fillRule: string; hash: number; path: Path; bytes: number }
const scratch = new Float64Array(1), words = new Uint32Array(scratch.buffer);

/** Stable SkPath identities let Skia reuse vector preparation under changing device matrices. */
export class RasterPathCache {
  private buckets = new Map<number, Entry[]>();
  /** Least recently used first. */
  private order = new Set<Entry>();
  /** The entry a wire last resolved to by content. Wires drawn once never pay for one. */
  private identities = new WeakMap<PathWire, Entry>();
  private bytes = 0;
  hits = 0;
  misses = 0;
  constructor(private readonly maximumBytes = 32 * 1024 * 1024) {}

  acquire(wire: PathWire, fillRule: string, create: () => Path): { path: Path; owned: boolean } {
    let entry = this.identities.get(wire);
    let hash = 0;
    if (!entry || entry.fillRule !== fillRule || !this.order.has(entry)) {
      // A changing arena can still contain unchanged glyphs and clip contours. Compare their
      // complete data rather than trusting an index or a hash that could alias different paths.
      hash = contentHash(wire, fillRule);
      entry = this.buckets.get(hash)?.find((candidate) => candidate.fillRule === fillRule && sameContent(candidate.wire, wire));
      if (entry) this.identities.set(wire, entry);
    }
    if (entry) {
      this.hits++;this.order.delete(entry);this.order.add(entry);
      return { path: entry.path, owned: false };
    }
    this.misses++;
    // The entry holds the native path and the wire it compares against.
    const path = create(), bytes = 128 + wire.points.length * 80 + wire.verbs.length * 40;
    if (bytes > this.maximumBytes) return { path, owned: true };
    while (this.order.size >= 4096 || this.bytes + bytes > this.maximumBytes) {
      const oldest = this.order.values().next().value;
      if (!oldest) break;
      this.evict(oldest);
    }
    entry = { wire, fillRule, hash, path, bytes };
    const bucket = this.buckets.get(hash);
    if (bucket) bucket.push(entry); else this.buckets.set(hash, [entry]);
    this.order.add(entry);this.bytes += bytes;
    return { path, owned: false };
  }

  dispose(): void {
    for (const entry of this.order) entry.path.delete();
    this.buckets.clear();this.order.clear();this.bytes = 0;
    this.identities = new WeakMap();
  }

  private evict(entry: Entry): void {
    entry.path.delete();this.bytes -= entry.bytes;this.order.delete(entry);
    const bucket = this.buckets.get(entry.hash)!;
    if (bucket.length === 1) this.buckets.delete(entry.hash);
    else bucket.splice(bucket.indexOf(entry), 1);
  }
}

function contentHash(wire: PathWire, fillRule: string): number {
  let value = 2166136261;
  for (let at = 0; at < fillRule.length; at++) value = Math.imul(value ^ fillRule.charCodeAt(at), 16777619);
  for (const verb of wire.verbs) value = Math.imul(value ^ (verb.length << 8 | verb.charCodeAt(0)), 16777619);
  for (const point of wire.points) {
    scratch[0] = point[0];
    value = Math.imul(value ^ words[0]!, 16777619);value = Math.imul(value ^ words[1]!, 16777619);
    scratch[0] = point[1];
    value = Math.imul(value ^ words[0]!, 16777619);value = Math.imul(value ^ words[1]!, 16777619);
  }
  return value >>> 0;
}

function sameContent(a: PathWire, b: PathWire): boolean {
  if (a.verbs.length !== b.verbs.length || a.points.length !== b.points.length) return false;
  for (let at = 0; at < a.verbs.length; at++) if (a.verbs[at] !== b.verbs[at]) return false;
  for (let at = 0; at < a.points.length; at++) {
    const p = a.points[at]!, q = b.points[at]!;
    if (p[0] !== q[0] || p[1] !== q[1]) return false;
  }
  return true;
}
