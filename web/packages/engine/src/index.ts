// Valle's engine in a page: a Motion scene or a Timeline played in a canvas, drawn with WebGPU in
// a worker. The files it reads are fetched from the page's address, or given.
//
//   const player = await createPlayer({ canvas, entry: "examples/01-kinetic-title.motion.tsx" });
//   player.play();

import type { Code, Controls, Inputs, Opened, Reply, Request, Source } from "./messages.ts";

export type { Code, Control, Controls, Opened, Source } from "./messages.ts";

export type Options = {
  canvas: HTMLCanvasElement;
  /** The scene, `.motion.tsx`, or the timeline, `.json`, as a path from `base`. */
  entry: string;
  /** Where relative paths are fetched from: the page's address by default. */
  base?: string | URL;
  /** Files given in place of fetching their paths. */
  files?: Record<string, Source>;
  /** Where the built-in fonts are fetched from, by their file names, if not Google's addresses. */
  fonts?: string | URL;
  props?: Record<string, unknown>;
  data?: Record<string, unknown>;
  /** Each resource's files, by its name. */
  resources?: Record<string, Source[]>;
  loop?: boolean;
};

export type Events = {
  /** A frame was drawn: its index and time. */
  frame: { frame: number; time: number };
  play: undefined;
  pause: undefined;
  /** Playing reached the last frame and does not loop. */
  ended: undefined;
  error: PlayerError;
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

export async function createPlayer(options: Options): Promise<Player> {
  const worker = new Worker(new URL("./worker.js", import.meta.url), { type: "module" });
  const player = new Player(worker, options);
  try {
    await player.start(options);
  } catch (error) {
    worker.terminate();
    throw error;
  }
  return player;
}

export class Player {
  #worker: Worker;
  #entry: string;
  #inputs: Inputs;
  #info!: Opened;
  #rate!: [number, number];
  #requests = new Map<number, { resolve: (v: unknown) => void; reject: (e: unknown) => void }>();
  #next = 0;
  #listeners = new Map<keyof Events, Set<(value: never) => void>>();
  #frame = 0;
  /** While playing: the frame and the time it started from. */
  #clock: { frame: number; at: number } | undefined;
  /** A frame being drawn, and the one asked for since. */
  #drawing = false;
  #wanted: number | undefined;
  #animation = 0;
  #given = 0;
  loop: boolean;

  constructor(worker: Worker, options: Options) {
    this.#worker = worker;
    this.#entry = options.entry;
    this.#inputs = { props: options.props ?? {}, data: options.data ?? {}, resources: {} };
    this.loop = options.loop ?? false;
    worker.onmessage = (event: MessageEvent<Reply>) => {
      const reply = event.data;
      const request = this.#requests.get(reply.id);
      this.#requests.delete(reply.id);
      if (reply.ok) {
        request?.resolve(reply.value);
      } else {
        request?.reject(new PlayerError(reply.code, reply.message));
      }
    };
  }

  /** @internal */
  async start(options: Options): Promise<void> {
    const offscreen = options.canvas.transferControlToOffscreen();
    const base = new URL(options.base ?? ".", location.href).href;
    const fonts = options.fonts === undefined ? undefined : new URL(options.fonts, base).href;
    await this.#send({ type: "start", canvas: offscreen, base, fonts }, [offscreen]);
    if (options.files) {
      await this.#send({ type: "give", files: options.files });
    }
    for (const [name, files] of Object.entries(options.resources ?? {})) {
      this.#inputs.resources[name] = await this.#give(name, files);
    }
    await this.#open();
  }

  /** The composition and the controls of what is open. */
  get info(): Opened {
    return this.#info;
  }

  get controls(): Controls {
    return this.#info.controls;
  }

  /** The frame shown, or being drawn. */
  get frame(): number {
    return this.#frame;
  }

  get time(): number {
    return this.#time(this.#frame);
  }

  get playing(): boolean {
    return this.#clock !== undefined;
  }

  play(): void {
    if (this.#clock) {
      return;
    }
    const last = this.#info.frames - 1;
    const from = this.#frame >= last && !this.loop ? 0 : this.#frame;
    this.#clock = { frame: from, at: performance.now() };
    this.#emit("play", undefined);
    const tick = () => {
      const clock = this.#clock;
      if (!clock) {
        return;
      }
      const [n, d] = this.#rate;
      let index = clock.frame + Math.floor(((performance.now() - clock.at) / 1000) * (n / d));
      if (index > last) {
        if (!this.loop) {
          this.#show(last);
          this.pause();
          this.#emit("ended", undefined);
          return;
        }
        index %= this.#info.frames;
      }
      this.#show(index);
      this.#animation = requestAnimationFrame(tick);
    };
    tick();
  }

  pause(): void {
    if (!this.#clock) {
      return;
    }
    this.#clock = undefined;
    cancelAnimationFrame(this.#animation);
    this.#emit("pause", undefined);
  }

  /** Shows the frame at `seconds`; playing goes on from it. */
  seek(seconds: number): void {
    const [n, d] = this.#rate;
    this.seekFrame(Math.floor(seconds * (n / d) + 1e-9));
  }

  seekFrame(index: number): void {
    const frame = Math.min(Math.max(Math.trunc(index), 0), this.#info.frames - 1);
    if (this.#clock) {
      this.#clock = { frame, at: performance.now() };
    }
    this.#show(frame);
  }

  /** Draws frame `index` and resolves once it is drawn. */
  async draw(index: number): Promise<void> {
    this.#frame = index;
    await this.#send({ type: "frame", index });
    this.#emit("frame", { frame: index, time: this.#time(index) });
  }

  /** The pixels of the frame drawn last: straight sRGB, 4 bytes a pixel, rows top down. */
  async pixels(): Promise<Uint8Array> {
    return (await this.#send({ type: "pixels" })) as Uint8Array;
  }

  /** Binds the scene's props to `props`, in place of those given before, and opens it again. */
  async setProps(props: Record<string, unknown>): Promise<void> {
    this.#inputs.props = props;
    await this.#reopen();
  }

  async setData(data: Record<string, unknown>): Promise<void> {
    this.#inputs.data = data;
    await this.#reopen();
  }

  /** Binds resource `name` to `files`, or back to its default with none. */
  async setResource(name: string, files: Source[] | undefined): Promise<void> {
    if (files === undefined || files.length === 0) {
      delete this.#inputs.resources[name];
    } else {
      this.#inputs.resources[name] = await this.#give(name, files);
    }
    await this.#reopen();
  }

  on<K extends keyof Events>(type: K, listener: (value: Events[K]) => void): () => void {
    const set = this.#listeners.get(type) ?? new Set();
    this.#listeners.set(type, set);
    set.add(listener as (value: never) => void);
    return () => set.delete(listener as (value: never) => void);
  }

  /** Stops the player and lets its GPU device go. */
  async close(): Promise<void> {
    this.pause();
    const worker = this.#worker;
    await this.#send({ type: "destroy" }).catch(() => {});
    for (const request of this.#requests.values()) {
      request.reject(new PlayerError("scene", "the player was closed"));
    }
    this.#requests.clear();
    worker.terminate();
  }

  #time(index: number): number {
    const [n, d] = this.#rate;
    return (index * d) / n;
  }

  /** Asks for frame `index`, drawn once the one being drawn is; frames asked for in between are
   * dropped. */
  #show(index: number): void {
    this.#frame = index;
    if (this.#drawing) {
      this.#wanted = index;
      return;
    }
    this.#drawing = true;
    this.draw(index)
      .catch((error: PlayerError) => {
        this.pause();
        this.#emit("error", error);
      })
      .finally(() => {
        this.#drawing = false;
        const wanted = this.#wanted;
        this.#wanted = undefined;
        if (wanted !== undefined && wanted !== index) {
          this.#show(wanted);
        }
      });
  }

  async #open(): Promise<void> {
    this.#info = (await this.#send({
      type: "open",
      entry: this.#entry,
      inputs: this.#inputs,
    })) as Opened;
    const [n, d = "1"] = this.#info.fps.split("/");
    this.#rate = [Number(n), Number(d)];
    this.#frame = Math.min(this.#frame, this.#info.frames - 1);
    await this.draw(this.#frame);
  }

  async #reopen(): Promise<void> {
    const playing = this.playing;
    this.pause();
    await this.#send({ type: "close" });
    await this.#open();
    if (playing) {
      this.play();
    }
  }

  /** Gives the worker `files`, each under a path of its own; an address is its own path. */
  async #give(name: string, files: Source[]): Promise<string[]> {
    const paths: string[] = [];
    const given: Record<string, Source> = {};
    for (const file of files) {
      if (typeof file === "string") {
        paths.push(file);
        continue;
      }
      const path = `valle:resource/${encodeURIComponent(name)}/${this.#given++}`;
      given[path] = file;
      paths.push(path);
    }
    await this.#send({ type: "give", files: given });
    return paths;
  }

  #send(request: Request, transfer: Transferable[] = []): Promise<unknown> {
    const id = this.#next++;
    return new Promise((resolve, reject) => {
      this.#requests.set(id, { resolve, reject });
      this.#worker.postMessage({ id, ...request }, transfer);
    });
  }

  #emit<K extends keyof Events>(type: K, value: Events[K]): void {
    for (const listener of this.#listeners.get(type) ?? []) {
      (listener as (value: Events[K]) => void)(value);
    }
  }
}
