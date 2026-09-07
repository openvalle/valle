/**
 * Serializes complete-document saves. Edits made while a save is in flight
 * schedule exactly one more pass after the accepted revision becomes the new
 * base. A stale base is surfaced to the user and reloaded explicitly; Studio
 * never attempts a three-way document merge.
 */
export class TimelineSaveQueue {
  #pending = false;
  #running: Promise<boolean> | null = null;

  constructor(
    private readonly saveOne: () => Promise<boolean>,
    private readonly afterIdle: () => void = () => undefined,
  ) {}

  get active(): boolean {
    return this.#running !== null;
  }

  noteEdit(): void {
    if (this.#running) this.#pending = true;
  }

  request(): Promise<boolean> {
    this.#pending = true;
    if (this.#running) return this.#running;
    const running = (async () => {
      let result = true;
      while (this.#pending) {
        this.#pending = false;
        result = await this.saveOne();
        if (!result) break;
      }
      return result;
    })();
    this.#running = running.finally(() => {
      this.#running = null;
      this.afterIdle();
    });
    return this.#running;
  }
}
