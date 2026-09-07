import {
  createBrowserValleWebPlayer,
  type BrowserValleWebPlayer,
  type CanonicalTimelineDocument,
} from "@valle/player-core";

export type VallePlayerState = "new" | "loading" | "ready" | "disposed";
export type VallePlayerOptions = Parameters<typeof createBrowserValleWebPlayer>[0];
export type ValleRenderPackageReplacement = Parameters<BrowserValleWebPlayer["replaceRenderPackage"]>[0];
export type ValleRenderResult = Awaited<ReturnType<BrowserValleWebPlayer["seek"]>>;

export interface VallePlayerStateDetail {
  previous: VallePlayerState;
  state: VallePlayerState;
}

export interface VallePlayerTimeDetail {
  timeS: number;
  durationS: number;
}

export interface VallePlayerPlayDetail {
  playing: boolean;
}

export interface VallePlayerRenderDetail {
  timeS: number;
  result: ValleRenderResult;
}

export interface VallePlayerDegradationDetail {
  degradations: ReadonlyArray<{ clipId?: string; effect: string; message?: string }>;
}

export interface VallePlayerErrorDetail {
  error: unknown;
  operation: string;
}

export interface VallePlayerEventMap {
  statechange: CustomEvent<VallePlayerStateDetail>;
  time: CustomEvent<VallePlayerTimeDetail>;
  play: CustomEvent<VallePlayerPlayDetail>;
  render: CustomEvent<VallePlayerRenderDetail>;
  degradation: CustomEvent<VallePlayerDegradationDetail>;
  error: CustomEvent<VallePlayerErrorDetail>;
}

type PlayerFactory = (options: VallePlayerOptions) => Promise<BrowserValleWebPlayer>;

export class VallePlayerController extends EventTarget {
  readonly options: VallePlayerOptions;

  #state: VallePlayerState = "new";
  #player: BrowserValleWebPlayer | null = null;
  #loadPromise: Promise<BrowserValleWebPlayer> | null = null;
  #disposePromise: Promise<void> | null = null;
  #lifecycleGeneration = 0;
  #contentGeneration = 0;
  #replaceTail: Promise<unknown> = Promise.resolve();
  #lastPlaying = false;
  #lastDegradationSignature = "[]";
  #factory: PlayerFactory;

  constructor(options: VallePlayerOptions, factory: PlayerFactory = createBrowserValleWebPlayer) {
    super();
    this.options = options;
    this.#factory = factory;
  }

  get state(): VallePlayerState {
    return this.#state;
  }

  get player(): BrowserValleWebPlayer | null {
    return this.#player;
  }

  get playing(): boolean {
    return this.#player?.playing ?? false;
  }

  get currentTimeS(): number {
    return this.#player?.currentTime() ?? 0;
  }

  get durationS(): number {
    return this.#player?.durationS() ?? 0;
  }

  load(): Promise<BrowserValleWebPlayer> {
    this.#assertNotDisposed("load");
    if (this.#player) return Promise.resolve(this.#player);
    if (this.#loadPromise) return this.#loadPromise;

    const generation = ++this.#lifecycleGeneration;
    this.#setState("loading");
    const loading = this.#factory(this.options)
      .then(async (player) => {
        if (this.#state === "disposed" || generation !== this.#lifecycleGeneration) {
          await player.close();
          throw new DOMException("player load was superseded", "AbortError");
        }
        this.#player = player;
        this.#lastPlaying = player.playing;
        player.onTimeUpdate = (timeS) => {
          if (this.#player !== player || this.#state !== "ready") return;
          this.#emit("time", { timeS, durationS: player.durationS() });
          queueMicrotask(() => this.#syncPlaying(player));
        };
        this.#setState("ready");
        this.#publishDegradations(player);
        return player;
      })
      .catch((error: unknown) => {
        if (this.#state !== "disposed" && generation === this.#lifecycleGeneration) {
          this.#setState("new");
          this.#emit("error", { error, operation: "load" });
        }
        throw error;
      })
      .finally(() => {
        if (this.#loadPromise === loading) this.#loadPromise = null;
      });
    this.#loadPromise = loading;
    return loading;
  }

  async play(): Promise<void> {
    const player = this.#requireReady("play");
    await this.#run("play", () => player.play());
    this.#syncPlaying(player);
  }

  pause(): void {
    const player = this.#requireReady("pause");
    try {
      player.pause();
      this.#syncPlaying(player);
      this.#emit("time", { timeS: player.currentTime(), durationS: player.durationS() });
    } catch (error) {
      this.#emit("error", { error, operation: "pause" });
      throw error;
    }
  }

  async seek(timeS: number): Promise<ValleRenderResult> {
    const player = this.#requireReady("seek");
    const result = await this.#run("seek", () => player.seek(timeS));
    this.#publishRender(player, result);
    return result;
  }

  async captureFrame(timeS = this.currentTimeS): Promise<ValleRenderResult> {
    const player = this.#requireReady("captureFrame");
    const result = await this.#run("captureFrame", () => player.scrub(timeS));
    this.#publishRender(player, result);
    return result;
  }

  replaceRenderPackage(next: ValleRenderPackageReplacement): Promise<ValleRenderResult | undefined> {
    const generation = ++this.#contentGeneration;
    const previous = this.#replaceTail.catch(() => undefined);
    const replacement = previous.then(async () => {
      const player = this.#requireReady("replaceRenderPackage");
      if (generation !== this.#contentGeneration) return undefined;
      const result = await this.#run("replaceRenderPackage", () => player.replaceRenderPackage(next));
      if (generation !== this.#contentGeneration || this.#state !== "ready") return undefined;
      this.#publishRender(player, result);
      return result;
    });
    this.#replaceTail = replacement;
    return replacement;
  }

  canonicalizeTimelineDocument(
    timeline: Parameters<BrowserValleWebPlayer["canonicalizeTimelineDocument"]>[0],
  ): CanonicalTimelineDocument {
    return this.#requireReady("canonicalizeTimelineDocument").canonicalizeTimelineDocument(timeline);
  }

  dispose(): Promise<void> {
    if (this.#disposePromise) return this.#disposePromise;
    const player = this.#player;
    this.#player = null;
    this.#lifecycleGeneration += 1;
    this.#contentGeneration += 1;
    this.#setState("disposed");
    this.#disposePromise = (async () => {
      if (!player) return;
      player.onTimeUpdate = undefined;
      await player.close();
    })().catch((error: unknown) => {
      this.#emit("error", { error, operation: "dispose" });
      throw error;
    });
    return this.#disposePromise;
  }

  addEventListener<K extends keyof VallePlayerEventMap>(
    type: K,
    listener: (this: VallePlayerController, event: VallePlayerEventMap[K]) => void,
    options?: boolean | AddEventListenerOptions,
  ): void;
  addEventListener(
    type: string,
    listener: EventListenerOrEventListenerObject | null,
    options?: boolean | AddEventListenerOptions,
  ): void;
  addEventListener(
    type: string,
    listener: EventListenerOrEventListenerObject | null,
    options?: boolean | AddEventListenerOptions,
  ): void {
    super.addEventListener(type, listener, options);
  }

  #requireReady(operation: string): BrowserValleWebPlayer {
    this.#assertNotDisposed(operation);
    if (!this.#player || this.#state !== "ready") {
      throw new Error(`cannot ${operation} while player state is ${this.#state}`);
    }
    return this.#player;
  }

  #assertNotDisposed(operation: string): void {
    if (this.#state === "disposed") throw new Error(`cannot ${operation} a disposed player`);
  }

  #setState(state: VallePlayerState): void {
    if (state === this.#state) return;
    const previous = this.#state;
    this.#state = state;
    this.#emit("statechange", { previous, state });
  }

  #syncPlaying(player: BrowserValleWebPlayer): void {
    if (this.#player !== player || this.#state !== "ready") return;
    if (this.#lastPlaying === player.playing) return;
    this.#lastPlaying = player.playing;
    this.#emit("play", { playing: player.playing });
  }

  #publishRender(player: BrowserValleWebPlayer, result: ValleRenderResult): void {
    this.#emit("time", { timeS: player.currentTime(), durationS: player.durationS() });
    this.#emit("render", { timeS: player.currentTime(), result });
    this.#publishDegradations(player);
  }

  #publishDegradations(player: BrowserValleWebPlayer): void {
    const degradations = player.stats.degradations ?? [];
    const signature = JSON.stringify(degradations);
    if (signature === this.#lastDegradationSignature) return;
    this.#lastDegradationSignature = signature;
    this.#emit("degradation", { degradations: [...degradations] });
  }

  async #run<T>(operation: string, action: () => Promise<T>): Promise<T> {
    try {
      return await action();
    } catch (error) {
      this.#emit("error", { error, operation });
      throw error;
    }
  }

  #emit<K extends keyof VallePlayerEventMap>(
    type: K,
    detail: VallePlayerEventMap[K]["detail"],
  ): void {
    this.dispatchEvent(new CustomEvent(type, { detail }));
  }
}
