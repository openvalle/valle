/** Bounded, opt-in Chrome/Perfetto CPU timeline. GPU device time is not inferred from submission. */
export class RenderTrace {
  private readonly epoch = performance.timeOrigin + performance.now();
  private readonly events: Record<string, unknown>[] = [];
  private readonly workers = new Map<number, number>();
  private readonly workerSpans = new Set<string>();
  private dropped = 0;

  constructor(private readonly maximumEvents = 50_000) {
    if (!Number.isSafeInteger(maximumEvents) || maximumEvents < 1) throw new Error("invalid trace capacity");
  }

  span(name: string, args: Record<string, unknown> = {}): () => void {
    const start = performance.now();
    return () => this.record(name, performance.timeOrigin + start, performance.now() - start, args);
  }

  record(name: string, absoluteStartMs: number, durationMs: number, args: Record<string, unknown>, tid = 1): void {
    this.push({ name, cat: "valle.cpu", ph: "X", pid: 1, tid,
      ts: (absoluteStartMs - this.epoch) * 1000, dur: durationMs * 1000, args });
  }

  worker(name: string, origin: number | undefined, start: number | undefined, duration: number, frame: number): void {
    if (origin === undefined || start === undefined) return;
    const key = `${name}:${origin}:${start}`;
    if (this.workerSpans.has(key)) return;
    if (this.workerSpans.size >= this.maximumEvents) { this.dropped++; return; }
    this.workerSpans.add(key);
    let tid = this.workers.get(origin);
    if (tid === undefined) { tid = this.workers.size + 2; this.workers.set(origin, tid); }
    this.record(name, origin + start, duration, { frame }, tid);
  }

  evidence(name: string, args: Record<string, unknown>): void {
    this.push({ name, cat: "valle.evidence", ph: "i", s: "t", pid: 1, tid: 1,
      ts: (performance.timeOrigin + performance.now() - this.epoch) * 1000, args });
  }

  snapshot(metadata: Record<string, unknown> = {}) {
    return { traceEvents: [...this.events], displayTimeUnit: "ms", metadata: {
      ...metadata, maximumEvents: this.maximumEvents, droppedEvents: this.dropped,
      clock: "performance.timeOrigin + performance.now; worker origins aligned to main",
      gpuDeviceTime: "unmeasured", memoryScope: "reported cache/surface estimates; not browser process memory",
    } };
  }

  private push(event: Record<string, unknown>): void {
    if (this.events.length < this.maximumEvents) this.events.push(event);
    else this.dropped++;
  }
}
