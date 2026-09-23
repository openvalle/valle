/** Coalesce fetches and bound retained source files. Failed loads remain retryable. */
export class MediaBlobCache {
  private entries = new Map<string, Promise<Blob>>();
  private queue: Promise<unknown> = Promise.resolve();
  private generation = 0;
  private controller = new AbortController();
  private sizes = new Map<string, number>();
  // This bounds encoded Blob storage, not JS ArrayBuffers. The browser may spool Blobs to disk.
  constructor(private readonly capacity = 32, private readonly maxBytes = 8 * 1024 ** 3) {}

  get(url: string, digest: string): Promise<Blob> {
    const key = `${digest}:${url}`;
    const cached = this.entries.get(key);
    if (cached) {
      this.entries.delete(key);
      this.entries.set(key, cached);
      return cached;
    }
    const generation = this.generation;
    const signal = this.controller.signal;
    const load = this.queue.then(async () => {
      if (generation !== this.generation) throw new Error("asset load cancelled");
      const response = await fetch(url, { signal });
      if (!response.ok) throw new Error(`fetch ${url}: HTTP ${response.status}`);
      const blob = await response.blob();
      signal.throwIfAborted();
      return blob;
    });
    this.queue = load.catch(() => {});
    this.entries.set(key, load);
    void load.then((blob) => {
      if (this.entries.get(key) !== load) return;
      this.sizes.set(key, blob.size);
      let bytes = [...this.sizes.values()].reduce((sum, size) => sum + size, 0);
      for (const candidate of this.entries.keys()) {
        if (this.sizes.size <= this.capacity && bytes <= this.maxBytes) break;
        const size = this.sizes.get(candidate);
        if (size === undefined) continue; // Never duplicate a pending fetch by evicting it.
        bytes -= size; this.entries.delete(candidate); this.sizes.delete(candidate);
      }
    }, () => {});
    void load.catch(() => { if (this.entries.get(key) === load) this.entries.delete(key); });
    return load;
  }

  clear(): void {
    this.generation += 1; this.entries.clear(); this.sizes.clear(); this.controller.abort();
    this.controller = new AbortController();
  }
}
