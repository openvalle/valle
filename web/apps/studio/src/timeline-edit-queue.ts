export class TimelineEditQueue {
  #tail: Promise<void> = Promise.resolve();

  enqueue(task: () => Promise<void>, onRejected?: (error: unknown) => void | Promise<void>): Promise<void> {
    const queued = this.#tail.then(task);
    this.#tail = onRejected ? queued.catch(onRejected) : queued;
    return this.#tail;
  }

  idle(): Promise<void> {
    return this.#tail;
  }
}
