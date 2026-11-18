// A worker running the engine, and what the page does for it: its requests answered in order,
// files fetched through the page's `fetch` where it gave one, including ranged media streams. A player and each export have their own.

import type { Code, Inputs, Reply, Request, Source } from "./messages.ts";

/** What opens the same work again in another worker: an export's. */
export type Spawn = {
  /** The directory of the worker, the audio worklet and the wasm. */
  runtime: URL;
  base: string;
  fonts?: string;
  fetch?: (url: string, init?: RequestInit) => Promise<Response>;
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
  | { media: number; url: string; headers: [string, string][] }
  | { cancelMedia: number }
  | { soundError: string }
  | { fetch: number; url: string }
  | { progress: number; done: number };

export class Connection {
  readonly worker: Worker;
  #requests = new Map<number, { resolve: (v: unknown) => void; reject: (e: unknown) => void }>();
  #next = 0;
  /** What hears how far each request that says so is, by its id. */
  #progress = new Map<number, (done: number) => void>();
  #media = new Map<number, AbortController>();
  #fetch: ((url: string, init?: RequestInit) => Promise<Response>) | undefined;
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

  constructor(
    worker: Worker,
    fetch: ((url: string, init?: RequestInit) => Promise<Response>) | undefined,
  ) {
    this.worker = worker;
    this.#fetch = fetch;
    worker.onmessage = (event: MessageEvent<Reply | Asked>) => {
      const data = event.data;
      if ("progress" in data) {
        this.#progress.get(data.progress)?.(data.done);
        return;
      }
      if ("media" in data) {
        void this.#stream(data.media, data.url, data.headers);
        return;
      }
      if ("cancelMedia" in data) {
        this.#media.get(data.cancelMedia)?.abort();
        this.#media.delete(data.cancelMedia);
        return;
      }
      if ("soundError" in data) {
        this.stopped(new PlayerError("file", data.soundError));
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
    for (const request of this.#media.values()) request.abort();
    this.#media.clear();
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

  /** Streams a media response to the worker without collecting it into an ArrayBuffer. */
  async #stream(id: number, url: string, headers: [string, string][]): Promise<void> {
    const abort = new AbortController();
    this.#media.set(id, abort);
    let reader: ReadableStreamDefaultReader<Uint8Array> | undefined;
    const finish = () => {
      this.#media.delete(id);
      abort.signal.removeEventListener("abort", cancel);
    };
    const cancel = () => {
      finish();
      void reader?.cancel(abort.signal.reason).catch(() => {});
    };
    try {
      const response = await this.#fetch!(url, { headers, signal: abort.signal });
      const body = response.body;
      if (abort.signal.aborted) {
        await body?.cancel();
        return;
      }
      if (!body || !response.ok) {
        finish();
        void body?.cancel().catch(() => {});
        this.worker.postMessage({
          media: id,
          status: response.status,
          statusText: response.statusText,
          headers: [...response.headers],
        });
        return;
      }
      const incoming = body.getReader();
      reader = incoming;
      abort.signal.addEventListener("abort", cancel, { once: true });
      const stream = new ReadableStream<Uint8Array>(
        {
          async pull(controller) {
            try {
              const chunk = await incoming.read();
              if (chunk.done) {
                finish();
                controller.close();
              } else {
                controller.enqueue(chunk.value);
              }
            } catch (error) {
              finish();
              controller.error(error);
            }
          },
          cancel(reason) {
            finish();
            return incoming.cancel(reason);
          },
        },
        { highWaterMark: 0 },
      );
      this.worker.postMessage(
        {
          media: id,
          status: response.status,
          statusText: response.statusText,
          headers: [...response.headers],
          body: stream,
        },
        [stream],
      );
    } catch (error) {
      cancel();
      if (!abort.signal.aborted) this.worker.postMessage({ media: id, error: String(error) });
    }
  }
}

/** What each player has open, which an export opens again. */
export const spawns = new WeakMap<object, Spawn>();
