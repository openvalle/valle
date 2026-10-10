import type { Paint } from "canvaskit-wasm";

interface Entry { paint: Paint; bytes: number; users: number }
export interface FillPaintLease { readonly paint: Paint; release(): void }

/** Exact fill paints, including gradient shaders. A live RasterTree lease pins its native paint. */
export class RasterPaintCache {
  private entries = new Map<string, Entry>();
  private bytes = 0;
  hits = 0;
  misses = 0;
  constructor(private readonly maximumBytes = 8 * 1024 * 1024) {}

  acquire(wire: unknown, create: () => Paint): FillPaintLease {
    const key = JSON.stringify(wire);
    let entry = this.entries.get(key);
    if (entry) {
      this.hits++;
      this.entries.delete(key); this.entries.set(key, entry);
    } else {
      this.misses++;
      const paint = create(), bytes = 512 + key.length * 8;
      const room = () => this.entries.size < 2048 && this.bytes + bytes <= this.maximumBytes;
      while (bytes <= this.maximumBytes && !room()) {
        const oldest = [...this.entries].find(([, item]) => item.users === 0);
        if (!oldest) break;
        this.entries.delete(oldest[0]); this.bytes -= oldest[1].bytes; oldest[1].paint.delete();
      }
      if (bytes > this.maximumBytes || !room()) {
        let active = true;
        return { paint, release() { if (active) { active = false; paint.delete(); } } };
      }
      entry = { paint, bytes, users: 0 };
      this.entries.set(key, entry); this.bytes += bytes;
    }
    entry.users++;
    let active = true;
    return { paint: entry.paint, release() { if (active) { active = false; entry!.users--; } } };
  }

  dispose(): void {
    for (const entry of this.entries.values()) entry.paint.delete();
    this.entries.clear(); this.bytes = 0;
  }
}
