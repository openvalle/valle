// A worker running the engine, and what the page does for it: its requests answered in order,
// files fetched through the page's `fetch` where it gave one, and sound decoded, which a worker
// cannot. A player and each export have their own.

import type { Code, Inputs, Reply, Request, Source } from "./messages.ts";

/** What opens the same work again in another worker: an export's. */
export type Spawn = {
  /** The directory of the worker, the audio worklet and the wasm. */
  runtime: URL;
  base: string;
  fonts?: string;
  fetch?: (url: string) => Promise<Response>;
  entry: string;
  inputs: Inputs;
  /** Every file the page gave, by its path. */
  given: Record<string, Source>;
};

export class PlayerError extends Error {
  constructor(
    readonly code: Code,
    message: string,
  ) {
    super(message);
    this.name = "PlayerError";
  }
}

/** Samples a second of the engine's sound. */
export const RATE = 48_000;

/** What the worker asks the page outside its replies. */
type Asked =
  | { decode: number; file: Uint8Array }
  | { fetch: number; url: string }
  | { progress: number; done: number };

export class Connection {
  readonly worker: Worker;
  #requests = new Map<number, { resolve: (v: unknown) => void; reject: (e: unknown) => void }>();
  #next = 0;
  /** What hears how far each request that says so is, by its id. */
  #progress = new Map<number, (done: number) => void>();
  #fetch: ((url: string) => Promise<Response>) | undefined;
  /** Hears that the worker stopped. */
  stopped: (error: PlayerError) => void = () => {};

  /** A worker started on `canvas`, with the files of `spawn` given. */
  static async start(spawn: Spawn, canvas: OffscreenCanvas): Promise<Connection> {
    const worker = new Worker(new URL("worker.js", spawn.runtime), { type: "module" });
    const connection = new Connection(worker, spawn.fetch);
    try {
      await connection.send(
        {
          type: "start",
          canvas,
          base: spawn.base,
          fonts: spawn.fonts,
          proxied: spawn.fetch !== undefined,
        },
        [canvas],
      );
      if (Object.keys(spawn.given).length > 0) {
        await connection.send({ type: "give", files: spawn.given });
      }
    } catch (error) {
      worker.terminate();
      throw error;
    }
    return connection;
  }

  constructor(worker: Worker, fetch: ((url: string) => Promise<Response>) | undefined) {
    this.worker = worker;
    this.#fetch = fetch;
    worker.onmessage = (event: MessageEvent<Reply | Asked>) => {
      const data = event.data;
      if ("progress" in data) {
        this.#progress.get(data.progress)?.(data.done);
        return;
      }
      if ("decode" in data) {
        void this.#decode(data.decode, data.file);
        return;
      }
      if ("fetch" in data) {
        void this.#fetched(data.fetch, data.url);
        return;
      }
      const request = this.#requests.get(data.id);
      this.#requests.delete(data.id);
      if (data.ok) {
        request?.resolve(data.value);
      } else {
        request?.reject(new PlayerError(data.code, data.message));
      }
    };
    // The worker stopped: nothing it was asked answers.
    worker.onerror = (event: ErrorEvent) => {
      event.preventDefault();
      const error = new PlayerError("scene", `the engine stopped: ${event.message}`);
      this.fail(error);
      this.stopped(error);
    };
  }

  /** Answers the request with an id of `id`, and hears of how far it is with `progress`. */
  send(
    request: Request,
    transfer: Transferable[] = [],
    progress?: (done: number) => void,
  ): Promise<unknown> {
    const id = this.#next++;
    if (progress) {
      this.#progress.set(id, progress);
    }
    return new Promise((resolve, reject) => {
      this.#requests.set(id, { resolve, reject });
      this.worker.postMessage({ id, ...request }, transfer);
    }).finally(() => this.#progress.delete(id));
  }

  /** Fails every request not answered. */
  fail(error: PlayerError): void {
    for (const request of this.#requests.values()) {
      request.reject(error);
    }
    this.#requests.clear();
  }

  /** Lets the worker's GPU device go, and stops it. */
  async close(): Promise<void> {
    // A worker stuck on a request is stopped all the same.
    const destroyed = this.send({ type: "destroy" }).catch(() => {});
    await Promise.race([destroyed, new Promise((r) => setTimeout(r, 1000))]);
    this.fail(new PlayerError("scene", "the player was closed"));
    this.worker.terminate();
  }

  /** Fetches `url` with the page's `fetch` for the worker: its status, and its bytes when it is
   * found. */
  async #fetched(id: number, url: string): Promise<void> {
    try {
      const response = await this.#fetch!(url);
      const bytes = response.ok ? await response.arrayBuffer() : undefined;
      this.worker.postMessage(
        { fetched: id, status: response.status, statusText: response.statusText, bytes },
        bytes ? [bytes] : [],
      );
    } catch (error) {
      this.worker.postMessage({ fetched: id, error: String(error) });
    }
  }

  /** Decodes a file's sound for the worker: interleaved stereo at 48 000 samples a second, or
   * null where it has none. */
  async #decode(id: number, file: Uint8Array): Promise<void> {
    let pcm: Float32Array | null = null;
    try {
      // An offline context decodes without a user's gesture, at its rate.
      const decoder = new OfflineAudioContext(2, 1, RATE);
      const buffer = await decoder.decodeAudioData(file.buffer as ArrayBuffer);
      const left = buffer.getChannelData(0);
      const right = buffer.getChannelData(Math.min(1, buffer.numberOfChannels - 1));
      pcm = new Float32Array(2 * buffer.length);
      for (let i = 0; i < buffer.length; i++) {
        pcm[2 * i] = left[i];
        pcm[2 * i + 1] = right[i];
      }
    } catch {
      // No sound in it, or none the browser decodes.
    }
    this.worker.postMessage({ decoded: id, pcm }, pcm ? [pcm.buffer] : []);
  }
}

/** What each player has open, which an export opens again. */
export const spawns = new WeakMap<object, Spawn>();
