// Valle's engine in a page: a Motion scene or a Timeline played in a canvas, drawn with WebGPU in
// a worker. The files it reads are fetched from the page's address, or given.
//
//   const player = await createPlayer({ canvas, entry: "examples/01-kinetic-title.motion.tsx" });
//   player.play();

import { Connection, PlayerError, RATE, type Spawn, spawns } from "./connection.ts";
import type { Controls, Opened, Request, Source } from "./messages.ts";

export { PlayerError } from "./connection.ts";
export type { Code, Control, Controls, Opened, Source } from "./messages.ts";

export type Options = {
  canvas: HTMLCanvasElement;
  /** The scene, `.motion.tsx`, or the timeline, `.json`, as a path from `base`. */
  entry: string;
  /** Where the SDK's worker, audio worklet and wasm are: beside this module by default. */
  runtime?: string | URL;
  /** Where relative paths are fetched from: the page's address by default. */
  base?: string | URL;
  /** Files given in place of fetching their paths. */
  files?: Record<string, Source>;
  /** What fetches the files the engine reads, the built-in fonts among them: `fetch` by default. */
  fetch?: (url: string) => Promise<Response>;
  /** Where the built-in fonts are fetched from, by their file names, if not Google's addresses. */
  fonts?: string | URL;
  props?: Record<string, unknown>;
  data?: Record<string, unknown>;
  /** Each resource's files, by its name. */
  resources?: Record<string, Source[]>;
  loop?: boolean;
  muted?: boolean;
  /** From 0 to 1. */
  volume?: number;
};

/** Playback over `elapsedMs`: distinct completed video frames and their rate (not display
 * refreshes), skipped frames, average frame latency, and bytes of engine memory. */
export type Stats = {
  fps: number;
  elapsedMs: number;
  shown: number;
  dropped: number;
  frameMs: number;
  memory: number;
};

export type Events = {
  /** A frame was drawn: its time and index. */
  time: { time: number; frame: number };
  play: undefined;
  pause: undefined;
  /** Playing reached the last frame and does not loop. */
  ended: undefined;
  error: PlayerError;
  /** Each second of playing, and when it pauses. */
  stats: Stats;
};

/** What plays a timeline's sound: an audio worklet the worker gives chunks to. */
type Audio = { context: AudioContext; node: AudioWorkletNode; gain: GainNode };

export async function createPlayer(options: Options): Promise<Player> {
  const runtime =
    options.runtime === undefined
      ? new URL("./", import.meta.url)
      : new URL(String(options.runtime).replace(/\/?$/, "/"), location.href);
  const base = new URL(options.base ?? ".", location.href).href;
  const spawn: Spawn = {
    runtime,
    base,
    fonts: options.fonts === undefined ? undefined : new URL(options.fonts, base).href,
    fetch: options.fetch,
    entry: options.entry,
    inputs: { props: options.props ?? {}, data: options.data ?? {}, resources: {} },
    given: { ...options.files },
  };
  const offscreen = options.canvas.transferControlToOffscreen();
  const connection = await Connection.start(spawn, offscreen);
  const player = new Player(connection, spawn, options);
  try {
    await player.start(options.resources ?? {});
  } catch (error) {
    await connection.close();
    throw error;
  }
  return player;
}

export class Player {
  #connection: Connection;
  #spawn: Spawn;
  #info!: Opened;
  #rate!: [number, number];
  #listeners = new Map<keyof Events, Set<(value: never) => void>>();
  #frame = 0;
  /** While playing: the frame it started from, and when, in seconds of its clock: the audio
   * context's when there is sound to hear, from when its first sample is heard (NaN until then),
   * else the page's. */
  #clock: { frame: number; at: number; gen?: number } | undefined;
  /** The sound of the timeline open, if it has any, and what plays it once it first plays. */
  #sound: { length: number } | null = null;
  #audio: Promise<Audio> | undefined;
  #context: AudioContext | undefined;
  #gen = 0;
  #volume: number;
  #muted: boolean;
  /** A frame being drawn, and the one asked for since. */
  #drawing: number | undefined;
  #wanted: number | undefined;
  #animation = 0;
  #given = 0;
  /** What playing has done since it last said, the frame its clock last reached, and when it
   * said. */
  #stats: Omit<Stats, "fps" | "elapsedMs"> = { shown: 0, dropped: 0, frameMs: 0, memory: 0 };
  #lastShown: number | undefined;
  #statsEpoch = 0;
  #reached = 0;
  #said = 0;
  loop: boolean;

  constructor(connection: Connection, spawn: Spawn, options: Options) {
    this.#connection = connection;
    this.#spawn = spawn;
    this.loop = options.loop ?? false;
    this.#muted = options.muted ?? false;
    this.#volume = options.volume ?? 1;
    spawns.set(this, spawn);
    connection.stopped = (error) => {
      this.pause();
      this.#emit("error", error);
    };
  }

  /** @internal */
  async start(resources: Record<string, Source[]>): Promise<void> {
    for (const [name, files] of Object.entries(resources)) {
      this.#spawn.inputs.resources[name] = await this.#give(name, files);
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
  get currentFrame(): number {
    return this.#frame;
  }

  /** The time of the frame shown, in seconds. */
  get currentTime(): number {
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

  /** Whether the sound is silenced: playing then keeps to the page's clock, as it may without a
   * user's gesture. */
  get muted(): boolean {
    return this.#muted;
  }

  set muted(muted: boolean) {
    if (muted === this.#muted) {
      return;
    }
    this.#muted = muted;
    if (this.#clock) {
      this.#run(this.#frame);
    }
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
    this.#reached = this.#clock!.frame;
    this.#resetStats(this.#frame);
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
        index = this.#reached = 0;
      }
      // Frames the clock went past without asking for them.
      this.#stats.dropped += Math.max(0, index - this.#reached - 1);
      this.#reached = Math.max(this.#reached, index);
      // The canvas retains the frame. A faster display need not redraw the same video frame.
      if (index !== this.#frame) this.#show(index);
      if (performance.now() - this.#said >= 1000) {
        this.#say();
      }
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
    this.#silence();
    this.#say();
    this.#emit("pause", undefined);
  }

  /** Shows the frame at `seconds`; playing goes on from it. */
  seek(seconds: number): void {
    this.seekFrame(this.#index(seconds));
  }

  seekFrame(index: number): void {
    const frame = Math.min(Math.max(Math.trunc(index), 0), this.#info.frames - 1);
    if (this.#clock) {
      this.#run(frame);
      this.#reached = frame;
      this.#resetStats(frame);
    }
    this.#wanted = undefined;
    this.#show(frame);
  }

  /** The frame at `seconds` as a PNG, drawn off the canvas. */
  async frame(seconds: number): Promise<Blob> {
    const index = this.#index(seconds);
    const pixels = (await this.#send({ type: "still", index })) as Uint8Array;
    const { width, height } = this.#info;
    const canvas = new OffscreenCanvas(width, height);
    const image = new ImageData(new Uint8ClampedArray(pixels.buffer as ArrayBuffer), width, height);
    canvas.getContext("2d")!.putImageData(image, 0, 0);
    return canvas.convertToBlob({ type: "image/png" });
  }

  /** `seconds` of the sound from `start`, interleaved stereo at 48 000 samples a second, as an
   * export encodes it; silence past its end. Throws where what is open has no sound. */
  async renderSound(start: number, seconds: number): Promise<Float32Array> {
    const sample = Math.round(start * RATE);
    const samples = Math.round((start + seconds) * RATE) - sample;
    return (await this.#send({ type: "render-sound", sample, samples })) as Float32Array;
  }

  /** Draws frame `index` and resolves once it is drawn. */
  async draw(index: number): Promise<void> {
    this.#frame = index;
    const started = performance.now();
    const epoch = this.#statsEpoch;
    const { memory } = (await this.#send({ type: "frame", index })) as { memory: number };
    const stats = this.#stats;
    stats.memory = memory;
    if (this.#clock && epoch === this.#statsEpoch && index !== this.#lastShown) {
      stats.frameMs += performance.now() - started;
      stats.shown++;
      this.#lastShown = index;
    }
    this.#emit("time", { time: this.#time(index), frame: index });
  }

  /** The pixels of the frame drawn last: straight sRGB, 4 bytes a pixel, rows top down. */
  async pixels(): Promise<Uint8Array> {
    return (await this.#send({ type: "pixels" })) as Uint8Array;
  }

  /** Binds the scene's props to `props`, in place of those given before. A value that does not
   * fit its control fails, and what was bound stays. */
  async setProps(props: Record<string, unknown>): Promise<void> {
    await this.#rebind("props", props);
  }

  async setData(data: Record<string, unknown>): Promise<void> {
    await this.#rebind("data", data);
  }

  /** Binds resource `name` to `files`, or back to its default with none. */
  async setResource(name: string, files: Source[] | undefined): Promise<void> {
    const resources = { ...this.#spawn.inputs.resources };
    if (files === undefined || files.length === 0) {
      delete resources[name];
    } else {
      resources[name] = await this.#give(name, files);
    }
    await this.#rebind("resources", resources);
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
    spawns.delete(this);
    await this.#connection.close();
    await this.#audio?.then((a) => a.context.close()).catch(() => {});
  }

  #time(index: number): number {
    const [n, d] = this.#rate;
    return (index * d) / n;
  }

  #index(seconds: number): number {
    const [n, d] = this.#rate;
    const index = Math.floor(seconds * (n / d) + 1e-9);
    return Math.min(Math.max(index, 0), this.#info.frames - 1);
  }

  /** Starts the clock at `frame`: the sound's, from when it plays that frame's sample, else
   * the page's. */
  #run(frame: number): void {
    this.#silence();
    if (!this.#sound || this.#muted) {
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

  #silence(): void {
    void this.#audio?.then((a) => a.node.port.postMessage({ stop: true }));
  }

  /** The clock's time now, in seconds. */
  #now(clock: { gen?: number }): number {
    if (clock.gen === undefined) {
      return performance.now() / 1000;
    }
    const context = this.#context;
    return context ? context.currentTime - (context.outputLatency || context.baseLatency) : 0;
  }

  /** What plays the sound, made the first time: in the call that first plays, as browsers let
   * pages start sound only on a user's gesture. */
  #playing(): Promise<Audio> {
    this.#audio ??= (async () => {
      const context = new AudioContext({ sampleRate: RATE, latencyHint: "interactive" });
      this.#context = context;
      await context.audioWorklet.addModule(new URL("sound.js", this.#spawn.runtime));
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

  /** Says how playing went since it last said, and starts counting again. */
  #say(): void {
    const now = performance.now();
    const elapsedMs = now - this.#said;
    const { shown, dropped, frameMs, memory } = this.#stats;
    this.#emit("stats", {
      fps: elapsedMs > 0 ? (shown * 1000) / elapsedMs : 0,
      elapsedMs,
      shown,
      dropped,
      frameMs: shown ? frameMs / shown : 0,
      memory,
    });
    this.#stats = { shown: 0, dropped: 0, frameMs: 0, memory };
    this.#said = now;
  }

  /** Paused draws and seeks do not count as playback or dropped frames. */
  #resetStats(frame: number): void {
    this.#stats = { shown: 0, dropped: 0, frameMs: 0, memory: this.#stats.memory };
    this.#lastShown = frame;
    this.#statsEpoch++;
    this.#said = performance.now();
  }

  /** Asks for frame `index`, drawn once the one being drawn is; frames asked for in between are
   * dropped. */
  #show(index: number): void {
    this.#frame = index;
    if (this.#drawing !== undefined) {
      if (
        this.#clock &&
        this.#wanted !== undefined &&
        this.#wanted !== index &&
        this.#wanted !== this.#drawing
      ) {
        this.#stats.dropped++;
      }
      this.#wanted = index;
      return;
    }
    this.#drawing = index;
    this.draw(index)
      .catch((error: PlayerError) => {
        this.pause();
        this.#emit("error", error);
      })
      .finally(() => {
        this.#drawing = undefined;
        const wanted = this.#wanted;
        this.#wanted = undefined;
        if (wanted !== undefined && wanted !== index) {
          this.#show(wanted);
        }
      });
  }

  async #open(): Promise<void> {
    const { entry, inputs } = this.#spawn;
    this.#info = (await this.#send({ type: "open", entry, inputs })) as Opened;
    const [n, d = "1"] = this.#info.fps.split("/");
    this.#rate = [Number(n), Number(d)];
    this.#frame = Math.min(this.#frame, this.#info.frames - 1);
    this.#sound = (await this.#send({ type: "sound", entry })) as { length: number } | null;
    await this.draw(this.#frame);
  }

  /** Binds `key` of the inputs to `value` and opens what is open again; where that fails, opens
   * it again as it was. */
  async #rebind<K extends "props" | "data" | "resources">(
    key: K,
    value: Spawn["inputs"][K],
  ): Promise<void> {
    const playing = this.playing;
    this.pause();
    const before = this.#spawn.inputs[key];
    this.#spawn.inputs[key] = value;
    try {
      await this.#send({ type: "close" });
      await this.#open();
    } catch (error) {
      this.#spawn.inputs[key] = before;
      await this.#send({ type: "close" });
      await this.#open();
      throw error;
    } finally {
      if (playing) {
        this.play();
      }
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
    Object.assign(this.#spawn.given, given);
    return paths;
  }

  #send(request: Request, transfer: Transferable[] = []): Promise<unknown> {
    return this.#connection.send(request, transfer);
  }

  #emit<K extends keyof Events>(type: K, value: Events[K]): void {
    for (const listener of this.#listeners.get(type) ?? []) {
      (listener as (value: Events[K]) => void)(value);
    }
  }
}
