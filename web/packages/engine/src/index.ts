// Valle's engine in a page: a Motion scene or a Timeline played in a canvas, drawn with WebGPU in
// a worker. The files it reads are fetched from the page's address, or given.
//
//   const player = await createPlayer({ canvas, entry: "examples/01-kinetic-title.motion.tsx" });
//   player.play();

import type {
  Code,
  Controls,
  ExportOptions,
  Inputs,
  Opened,
  Reply,
  Request,
  Source,
} from "./messages.ts";

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

export type Export = Partial<ExportOptions> & {
  /** Hears of each frame encoded, and of how many there are. */
  onProgress?: (done: number, frames: number) => void;
  /** Cancels the export, which then fails with `cancelled`. */
  signal?: AbortSignal;
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

/** What plays a timeline's sound: an audio worklet the worker gives chunks to. */
type Audio = { context: AudioContext; node: AudioWorkletNode; gain: GainNode };

/** Samples a second of the engine's sound. */
const RATE = 48_000;

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
  /** While playing: the frame it started from, and when, in seconds of its clock: the audio
   * context's when there is sound, from when its first sample is heard (NaN until then), else the
   * page's. */
  #clock: { frame: number; at: number; gen?: number } | undefined;
  /** The sound of the timeline open, if it has any, and what plays it once it first plays. */
  #sound: { length: number } | null = null;
  #audio: Promise<Audio> | undefined;
  #gen = 0;
  #volume = 1;
  /** A frame being drawn, and the one asked for since. */
  #drawing = false;
  #wanted: number | undefined;
  #animation = 0;
  #given = 0;
  /** What hears how far each request that says so is, by its id. */
  #progress = new Map<number, (done: number) => void>();
  loop: boolean;

  constructor(worker: Worker, options: Options) {
    this.#worker = worker;
    this.#entry = options.entry;
    this.#inputs = { props: options.props ?? {}, data: options.data ?? {}, resources: {} };
    this.loop = options.loop ?? false;
    worker.onmessage = (
      event: MessageEvent<
        Reply | { decode: number; file: Uint8Array } | { progress: number; done: number }
      >,
    ) => {
      if ("progress" in event.data) {
        this.#progress.get(event.data.progress)?.(event.data.done);
        return;
      }
      if ("decode" in event.data) {
        void this.#decode(event.data.decode, event.data.file);
        return;
      }
      const reply = event.data;
      const request = this.#requests.get(reply.id);
      this.#requests.delete(reply.id);
      if (reply.ok) {
        request?.resolve(reply.value);
      } else {
        request?.reject(new PlayerError(reply.code, reply.message));
      }
    };
    // The worker stopped: nothing it was asked answers.
    worker.onerror = (event: ErrorEvent) => {
      event.preventDefault();
      const error = new PlayerError("scene", `the engine stopped: ${event.message}`);
      for (const request of this.#requests.values()) {
        request.reject(error);
      }
      this.#requests.clear();
      this.pause();
      this.#emit("error", error);
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

  /** The sound's volume, from 0 to 1. */
  get volume(): number {
    return this.#volume;
  }

  set volume(volume: number) {
    this.#volume = volume;
    void this.#audio?.then((a) => (a.gain.gain.value = volume));
  }

  /** Whether what is open has sound, which then sets the clock playing keeps to. */
  get hasSound(): boolean {
    return this.#sound !== null;
  }

  play(): void {
    if (this.#clock) {
      return;
    }
    const last = this.#info.frames - 1;
    this.#run(this.#frame >= last && !this.loop ? 0 : this.#frame);
    this.#emit("play", undefined);
    const tick = () => {
      const clock = this.#clock;
      if (!clock) {
        return;
      }
      const [n, d] = this.#rate;
      const elapsed = this.#now(clock) - clock.at;
      let index = clock.frame + (Number.isNaN(elapsed) ? 0 : Math.floor(elapsed * (n / d)));
      if (index > last) {
        if (!this.loop) {
          this.#show(last);
          this.pause();
          this.#emit("ended", undefined);
          return;
        }
        this.#run(0);
        index = 0;
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
    void this.#audio?.then((a) => a.node.port.postMessage({ stop: true }));
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
      this.#run(frame);
    }
    this.#show(frame);
  }

  /** The MP4 of what is open, as it is bound now: its frames from the first, H.264 by default,
   * and its sound, AAC where the browser encodes it, else Opus. Playing pauses. */
  async export(options: Export = {}): Promise<Blob> {
    this.pause();
    const { onProgress, signal, codec = "avc", bitrate } = options;
    signal?.throwIfAborted();
    const id = this.#next;
    const frames = this.#info.frames;
    this.#progress.set(id, (done) => onProgress?.(done, frames));
    const cancel = () => this.#worker.postMessage({ cancel: true });
    signal?.addEventListener("abort", cancel);
    try {
      const mp4 = (await this.#send({
        type: "export",
        options: { codec, bitrate },
      })) as ArrayBuffer;
      return new Blob([mp4], { type: "video/mp4" });
    } finally {
      this.#progress.delete(id);
      signal?.removeEventListener("abort", cancel);
    }
  }

  /** `seconds` of the sound from `start`, interleaved stereo at 48 000 samples a second, as an
   * export encodes it; silence past its end. Throws where what is open has no sound. */
  async renderSound(start: number, seconds: number): Promise<Float32Array> {
    const sample = Math.round(start * RATE);
    const samples = Math.round((start + seconds) * RATE) - sample;
    return (await this.#send({ type: "render-sound", sample, samples })) as Float32Array;
  }

  /** Starts the clock at `frame`: the sound's, from when it plays that frame's sample, else
   * the page's. */
  #run(frame: number): void {
    if (!this.#sound) {
      this.#clock = { frame, at: performance.now() / 1000 };
      return;
    }
    const gen = ++this.#gen;
    this.#clock = { frame, at: Number.NaN, gen };
    const sample = Math.round(this.#time(frame) * RATE);
    void this.#playing().then(async (a) => {
      await a.context.resume();
      a.node.port.postMessage({ start: gen, sample });
    });
  }

  /** The clock's time now, in seconds. */
  #now(clock: { gen?: number }): number {
    if (clock.gen === undefined) {
      return performance.now() / 1000;
    }
    const context = this.#context;
    return context ? context.currentTime - (context.outputLatency || context.baseLatency) : 0;
  }

  #context: AudioContext | undefined;

  /** What plays the sound, made the first time: in the call that first plays, as browsers let
   * pages start sound only on a user's gesture. */
  #playing(): Promise<Audio> {
    this.#audio ??= (async () => {
      const context = new AudioContext({ sampleRate: RATE, latencyHint: "interactive" });
      this.#context = context;
      await context.audioWorklet.addModule(new URL("./sound.js", import.meta.url));
      const node = new AudioWorkletNode(context, "valle-sound", {
        numberOfInputs: 0,
        outputChannelCount: [2],
      });
      const gain = new GainNode(context, { gain: this.#volume });
      node.connect(gain).connect(context.destination);
      node.port.onmessage = (event: MessageEvent<{ started: number; frame: number }>) => {
        const { started, frame } = event.data;
        if (this.#clock?.gen === started) {
          this.#clock.at = frame / context.sampleRate;
        }
      };
      const channel = new MessageChannel();
      node.port.postMessage({ port: channel.port1 }, [channel.port1]);
      await this.#send({ type: "worklet", port: channel.port2 }, [channel.port2]);
      return { context, node, gain };
    })();
    return this.#audio;
  }

  /** Decodes a file's sound for the worker, which cannot: interleaved stereo at 48 000 samples a
   * second, or null where it has none. */
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
    this.#worker.postMessage({ decoded: id, pcm }, pcm ? [pcm.buffer] : []);
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
    // A worker stuck on a request is stopped all the same.
    const destroyed = this.#send({ type: "destroy" }).catch(() => {});
    await Promise.race([destroyed, new Promise((r) => setTimeout(r, 1000))]);
    for (const request of this.#requests.values()) {
      request.reject(new PlayerError("scene", "the player was closed"));
    }
    this.#requests.clear();
    worker.terminate();
    await this.#audio?.then((a) => a.context.close()).catch(() => {});
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
    this.#sound = (await this.#send({ type: "sound", entry: this.#entry })) as {
      length: number;
    } | null;
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
