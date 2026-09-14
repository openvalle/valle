/** Coalesce frozen drafts and discard both stale successes and stale failures. */
export class DraftPreview<T, R> {
  #version = 0;
  #pending: { version: number; draft: T } | null = null;
  #timer: ReturnType<typeof setTimeout> | null = null;
  #running: Promise<void> | null = null;

  constructor(private readonly options: {
    prepare(draft: T): Promise<R>;
    apply(result: R, draft: T, isCurrent: () => boolean): Promise<void>;
    updating(): void;
    failed(error: unknown): void;
    ready(): void;
  }) {}

  schedule(draft: T, delay = 150): void {
    this.#pending = { version: ++this.#version, draft: structuredClone(draft) };
    this.options.updating();
    if (this.#timer) clearTimeout(this.#timer);
    this.#timer = setTimeout(() => { this.#timer = null; void this.flush(); }, delay);
  }

  invalidate(): void {
    ++this.#version;
    this.#pending = null;
    if (this.#timer) clearTimeout(this.#timer);
    this.#timer = null;
  }

  async flush(): Promise<void> {
    if (this.#timer) clearTimeout(this.#timer);
    this.#timer = null;
    if (this.#running) return this.#running;
    this.#running = this.#drain().finally(() => { this.#running = null; });
    return this.#running;
  }

  async #drain(): Promise<void> {
    while (this.#pending) {
      const { version, draft } = this.#pending;
      this.#pending = null;
      const isCurrent = () => version === this.#version;
      try {
        const result = await this.options.prepare(draft);
        if (!isCurrent()) continue;
        await this.options.apply(result, draft, isCurrent);
        if (isCurrent()) this.options.ready();
      } catch (error) {
        if (isCurrent()) this.options.failed(error);
      }
    }
  }
}
