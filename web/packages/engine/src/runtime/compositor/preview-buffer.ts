/** A bounded producer of owned, completed preview frames. Clock/presentation stay with the player. */
export class PreviewBuffer<T> {
  private entries: Array<{ frame: number; value: T }> = [];
  private stopped = false;
  private done = false;
  private failure: { cause: unknown } | null = null;
  private minimumFrame: number;
  readonly producer: Promise<void>;

  constructor(
    first: number,
    last: number,
    readonly capacity: number,
    private create: (frame: number) => Promise<T>,
    private dispose: (value: T) => void,
    private clockFrame?: () => number,
  ) {
    if (!Number.isSafeInteger(capacity) || capacity < 1)
      throw new RangeError("Preview capacity must be a positive integer");
    this.minimumFrame = first;
    this.producer = this.run(first, last);
  }
  private async run(first: number, last: number): Promise<void> {
    try {
      let cursor = first;
      while (cursor <= last && !this.stopped) {
        while (this.entries.length >= this.capacity && !this.stopped) await tick();
        if (this.stopped) break;
        const frame = Math.max(cursor, this.minimumFrame, this.clockFrame?.() ?? cursor);
        if (!Number.isSafeInteger(frame))
          throw new RangeError("Preview clock frame must be an integer");
        if (frame > last) break;
        const value = await this.create(frame);
        if (this.stopped) {
          this.dispose(value);
          break;
        }
        this.entries.push({ frame, value });
        cursor = frame + 1;
      }
    } catch (error) {
      this.failure = { cause: error };
    } finally {
      this.done = true;
    }
  }
  async prebuffer(count: number): Promise<void> {
    while (!this.stopped && !this.done && this.entries.length < Math.min(count, this.capacity))
      await tick();
    if (this.failure) throw this.failure.cause;
  }
  async take(frame: number): Promise<{ frame: number; value: T } | null> {
    this.minimumFrame = Math.max(this.minimumFrame, frame);
    for (;;) {
      while (this.entries.length && this.entries[0]!.frame < frame)
        this.dispose(this.entries.shift()!.value);
      if (this.entries.length) return this.entries.shift()!;
      if (this.failure) throw this.failure.cause;
      if (this.stopped || this.done) return null;
      await tick();
    }
  }
  close(): void {
    if (this.stopped) return;
    this.stopped = true;
    for (const entry of this.entries) this.dispose(entry.value);
    this.entries = [];
  }
}
const tick = () => new Promise<void>((resolve) => setTimeout(resolve, 2));
