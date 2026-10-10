import type { Path } from "canvaskit-wasm";
import type { PathWire } from "./draw-program.ts";

/** Stable SkPath identities let Skia reuse vector preparation under changing device matrices. */
export class RasterPathCache {
  private entries = new Map<string, { path: Path; bytes: number }>();
  private identities = new WeakMap<PathWire, Map<string, string>>();
  private bytes = 0;
  hits = 0;
  misses = 0;
  constructor(private readonly maximumBytes = 32 * 1024 * 1024) {}

  acquire(wire: PathWire, fillRule: string, create: () => Path): { path: Path; owned: boolean } {
    let keys = this.identities.get(wire);
    let key = keys?.get(fillRule);
    if (key === undefined) {
      // A changing arena can still contain unchanged glyphs and clip contours. Compare their
      // complete data rather than trusting an index or a hash that could alias different paths.
      key = `${fillRule}:${JSON.stringify([wire.verbs, wire.points])}`;
      if (!keys) this.identities.set(wire, keys = new Map());
      keys.set(fillRule, key);
    }
    const cached = this.entries.get(key);
    if (cached) {
      this.hits++;this.entries.delete(key);this.entries.set(key, cached);
      return { path: cached.path, owned: false };
    }
    this.misses++;
    const path = create(), bytes = 128 + key.length * 2 + wire.points.length * 16 + wire.verbs.length * 8;
    if (bytes > this.maximumBytes) return { path, owned: true };
    while (this.entries.size >= 4096 || this.bytes + bytes > this.maximumBytes) {
      const oldest = this.entries.entries().next().value;
      if (!oldest) break;
      oldest[1].path.delete();this.bytes -= oldest[1].bytes;
      this.entries.delete(oldest[0]);
    }
    this.entries.set(key, { path, bytes });this.bytes += bytes;
    return { path, owned: false };
  }

  dispose(): void {
    for (const value of this.entries.values()) value.path.delete();
    this.entries.clear();this.bytes = 0;
    this.identities = new WeakMap();
  }
}
