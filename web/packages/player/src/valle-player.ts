import { LitElement, css, html, nothing } from "lit";
import type { PropertyValues } from "lit";

import {
  VallePlayerController,
  type VallePlayerEventMap,
  type VallePlayerOptions,
  type VallePlayerState,
  type ValleRenderResult,
  type ValleRenderPackageReplacement,
} from "./controller.ts";

export type VallePlayerElementOptions = Omit<VallePlayerOptions, "canvas">;
type VallePlayerRuntime = NonNullable<VallePlayerController["player"]>;
type OfflineAudioOptions = Parameters<VallePlayerRuntime["renderAudioOffline"]>[2];

const RELAYED_EVENTS = ["time", "play", "render", "degradation", "error"] as const;

export class VallePlayerElement extends LitElement {
  static properties = {
    autoplay: { type: Boolean, reflect: true },
    gpu: { type: Boolean, reflect: true },
    pxScale: { type: Number, attribute: "px-scale" },
    assetBaseUrl: { type: String, attribute: "asset-base-url" },
    proxyBase: { type: String, attribute: "proxy-base" },
    loadingLabel: { type: String, attribute: "loading-label" },
    state: { state: true },
    errorMessage: { state: true },
    fixedPackageManifestJson: { attribute: false },
    timelineJson: { attribute: false },
    resourceManifestJson: { attribute: false },
    verifiedBindingBundleJson: { attribute: false },
    assets: { attribute: false },
    runtimeAssets: { attribute: false },
    runtimeBaseUrl: { attribute: false },
    audioContext: { attribute: false },
  };

  static styles = css`
    :host {
      position: relative;
      display: block;
      overflow: hidden;
      min-width: 0;
      min-height: 12rem;
      color: #dbe7ff;
      background: #08111f;
      contain: layout paint style;
    }

    canvas {
      display: block;
      width: 100%;
      height: 100%;
      outline: none;
      background: transparent;
    }

    .status {
      position: absolute;
      inset: 0;
      z-index: 1;
      display: grid;
      place-items: center;
      padding: 1.5rem;
      background: color-mix(in srgb, #08111f 88%, transparent);
      font: 500 0.875rem/1.5 system-ui, sans-serif;
      text-align: center;
    }

    .status[hidden] {
      display: none;
    }

    .status.error {
      color: #fecaca;
      background: color-mix(in srgb, #2a0b11 92%, transparent);
    }

    .slot {
      position: absolute;
      inset: 0;
      z-index: 2;
      pointer-events: none;
    }

    ::slotted(*) {
      pointer-events: auto;
    }
  `;

  declare autoplay: boolean;
  declare gpu: boolean;
  declare pxScale: number;
  declare assetBaseUrl: string;
  declare proxyBase: string | null;
  declare loadingLabel: string;

  declare fixedPackageManifestJson: string;
  declare timelineJson: string;
  declare resourceManifestJson: string;
  declare verifiedBindingBundleJson: string;
  declare assets: VallePlayerOptions["assets"];
  declare runtimeAssets: VallePlayerOptions["runtimeAssets"] | null;
  declare runtimeBaseUrl: VallePlayerOptions["runtimeBaseUrl"];
  declare audioContext: VallePlayerOptions["audioContext"];

  declare state: VallePlayerState;
  declare errorMessage: string;

  #controller: VallePlayerController | null = null;
  #connectionGeneration = 0;

  constructor() {
    super();
    this.autoplay = false;
    this.gpu = true;
    this.pxScale = 1;
    this.assetBaseUrl = "/assets/";
    this.proxyBase = null;
    this.loadingLabel = "Loading Valle Player…";
    this.fixedPackageManifestJson = "";
    this.timelineJson = "";
    this.resourceManifestJson = "";
    this.verifiedBindingBundleJson = "";
    this.runtimeAssets = null;
    this.state = "new";
    this.errorMessage = "";
  }

  get controller(): VallePlayerController | null {
    return this.#controller;
  }

  get canvas(): HTMLCanvasElement | null {
    return this.renderRoot.querySelector("canvas");
  }

  get playing(): boolean {
    return this.#controller?.playing ?? false;
  }

  get stats() {
    return this.#runtime("read stats").stats;
  }

  get hitRects() {
    return this.#runtime("read hit map").hitRects;
  }

  get muted(): boolean {
    return this.#runtime("read muted").muted;
  }

  get masterVolume(): number {
    return this.#runtime("read volume").masterVolume;
  }

  connectedCallback(): void {
    super.connectedCallback();
    this.#connectionGeneration += 1;
    if (this.state === "disposed") this.state = "new";
    void this.updateComplete.then(() => this.#loadWhenConfigured());
  }

  disconnectedCallback(): void {
    this.#connectionGeneration += 1;
    void this.dispose();
    super.disconnectedCallback();
  }

  protected firstUpdated(): void {
    this.#loadWhenConfigured();
  }

  protected updated(changed: PropertyValues<this>): void {
    if (
      changed.has("fixedPackageManifestJson")
      || changed.has("timelineJson")
      || changed.has("resourceManifestJson")
      || changed.has("verifiedBindingBundleJson")
      || changed.has("runtimeAssets")
      || changed.has("runtimeBaseUrl")
    ) {
      this.#loadWhenConfigured();
    }
  }

  configure(options: VallePlayerElementOptions): void {
    this.fixedPackageManifestJson = options.fixedPackageManifestJson;
    this.timelineJson = options.timelineJson;
    this.resourceManifestJson = options.resourceManifestJson;
    this.verifiedBindingBundleJson = options.verifiedBindingBundleJson;
    this.assets = options.assets;
    this.runtimeAssets = options.runtimeAssets;
    this.runtimeBaseUrl = options.runtimeBaseUrl;
    this.assetBaseUrl = options.assetBaseUrl ?? this.assetBaseUrl;
    this.proxyBase = options.proxyBase ?? null;
    this.pxScale = options.pxScale ?? this.pxScale;
    this.gpu = options.gpu ?? this.gpu;
    this.audioContext = options.audioContext;
  }

  async load(options?: VallePlayerElementOptions): Promise<void> {
    if (options) this.configure(options);
    await this.updateComplete;
    if (!this.isConnected) throw new Error("cannot load a disconnected <valle-player>");
    if (!this.fixedPackageManifestJson) throw new Error("fixedPackageManifestJson is required");
    if (!this.timelineJson) throw new Error("timelineJson is required");
    if (!this.resourceManifestJson) throw new Error("resourceManifestJson is required");
    if (!this.verifiedBindingBundleJson) throw new Error("verifiedBindingBundleJson is required");
    if (this.runtimeAssets == null) throw new Error("runtimeAssets is required");
    if (this.#controller?.state === "ready") return;
    if (this.#controller?.state === "loading") {
      await this.#controller.load();
      return;
    }
    if (this.#controller?.state === "disposed") this.#controller = null;

    const canvas = this.canvas;
    if (!canvas) throw new Error("<valle-player> canvas is not rendered");
    const generation = this.#connectionGeneration;
    const controller = new VallePlayerController({
      fixedPackageManifestJson: this.fixedPackageManifestJson,
      timelineJson: this.timelineJson,
      resourceManifestJson: this.resourceManifestJson,
      verifiedBindingBundleJson: this.verifiedBindingBundleJson,
      assets: this.assets,
      canvas,
      assetBaseUrl: this.assetBaseUrl,
      proxyBase: this.proxyBase,
      runtimeAssets: this.runtimeAssets,
      runtimeBaseUrl: this.runtimeBaseUrl,
      pxScale: this.pxScale,
      gpu: this.gpu,
      audioContext: this.audioContext,
    });
    this.#controller = controller;
    this.#bindController(controller);
    try {
      await controller.load();
      if (
        generation !== this.#connectionGeneration
        || !this.isConnected
        || this.#controller !== controller
      ) {
        await controller.dispose();
        return;
      }
      if (this.autoplay) await controller.play();
    } catch (error) {
      if (this.#controller === controller && controller.state !== "disposed") {
        this.errorMessage = error instanceof Error ? error.message : String(error);
      }
      throw error;
    }
  }

  play(): Promise<void> {
    return this.#requiredController("play").play();
  }

  pause(): void {
    this.#requiredController("pause").pause();
  }

  seek(timeS: number): Promise<ValleRenderResult> {
    return this.#requiredController("seek").seek(timeS);
  }

  captureFrame(timeS = this.currentTime()): Promise<ValleRenderResult> {
    return this.#requiredController("captureFrame").captureFrame(timeS);
  }

  scrub(timeS: number): Promise<ValleRenderResult> {
    return this.captureFrame(timeS);
  }

  replaceRenderPackage(next: ValleRenderPackageReplacement): Promise<ValleRenderResult | undefined> {
    return this.#requiredController("replaceRenderPackage").replaceRenderPackage(next);
  }

  canonicalizeTimelineDocument(
    timeline: Parameters<VallePlayerRuntime["canonicalizeTimelineDocument"]>[0],
  ) {
    return this.#requiredController("canonicalizeTimelineDocument").canonicalizeTimelineDocument(timeline);
  }

  currentTime(): number {
    return this.#controller?.currentTimeS ?? 0;
  }

  durationS(): number {
    return this.#controller?.durationS ?? 0;
  }

  lastFrameTimeS(): number {
    return this.#runtime("read last frame time").lastFrameTimeS();
  }

  hasAudio(): boolean {
    return this.#runtime("inspect audio").hasAudio();
  }

  setVolume(value: number): number {
    return this.#runtime("set volume").setVolume(value);
  }

  setMuted(value: boolean): boolean {
    return this.#runtime("set muted").setMuted(value);
  }

  renderAudioOffline(
    startS: number,
    endS: number,
    options?: OfflineAudioOptions,
  ) {
    return this.#runtime("render audio offline").renderAudioOffline(startS, endS, options);
  }

  videoKeyframeThumbnails(
    assetId: string,
    options?: Parameters<VallePlayerRuntime["videoKeyframeThumbnails"]>[1],
  ) {
    return this.#runtime("extract video thumbnails").videoKeyframeThumbnails(assetId, options);
  }

  audioPeaks(
    assetId: string,
    options?: Parameters<VallePlayerRuntime["audioPeaks"]>[1],
  ) {
    return this.#runtime("extract audio peaks").audioPeaks(assetId, options);
  }

  motionInspection(
    ...args: Parameters<VallePlayerRuntime["motionInspection"]>
  ) {
    return this.#runtime("inspect Motion frame").motionInspection(...args);
  }

  async dispose(): Promise<void> {
    const controller = this.#controller;
    this.#controller = null;
    if (!controller) {
      this.state = "disposed";
      return;
    }
    await controller.dispose();
  }

  protected render() {
    const loading = this.state === "new" || this.state === "loading";
    const failed = this.errorMessage.length > 0;
    return html`
      <canvas part="canvas" aria-label="Valle player canvas"></canvas>
      <div class="status loading" part="loading" ?hidden=${!loading || failed}>
        ${this.loadingLabel}
      </div>
      <div class="status error" part="error" role="alert" ?hidden=${!failed}>
        ${failed ? this.errorMessage : nothing}
      </div>
      <div class="slot" part="overlay"><slot></slot></div>
    `;
  }

  #loadWhenConfigured(): void {
    if (this.#controller || !this.isConnected || !this.timelineJson || this.runtimeAssets == null) {
      return;
    }
    void this.load().catch((error: unknown) => {
      if (error instanceof DOMException && error.name === "AbortError") return;
      this.errorMessage = error instanceof Error ? error.message : String(error);
    });
  }

  #bindController(controller: VallePlayerController): void {
    controller.addEventListener("statechange", (event) => {
      if (this.#controller !== controller && event.detail.state !== "disposed") return;
      this.state = event.detail.state;
      if (event.detail.state === "ready") this.errorMessage = "";
      this.dispatchEvent(new CustomEvent("statechange", {
        detail: event.detail,
        bubbles: true,
        composed: true,
      }));
    });
    for (const type of RELAYED_EVENTS) {
      controller.addEventListener(type, (event: VallePlayerEventMap[typeof type]) => {
        if (this.#controller !== controller) return;
        if (type === "error") {
          const error = (event as VallePlayerEventMap["error"]).detail.error;
          this.errorMessage = error instanceof Error ? error.message : String(error);
        }
        this.dispatchEvent(new CustomEvent(type, {
          detail: event.detail,
          bubbles: true,
          composed: true,
        }));
      });
    }
  }

  #requiredController(operation: string): VallePlayerController {
    if (!this.#controller || this.#controller.state !== "ready") {
      throw new Error(`cannot ${operation} while <valle-player> state is ${this.state}`);
    }
    return this.#controller;
  }

  #runtime(operation: string) {
    const player = this.#requiredController(operation).player;
    if (!player) throw new Error(`cannot ${operation} without a player runtime`);
    return player;
  }
}

if (!customElements.get("valle-player")) {
  customElements.define("valle-player", VallePlayerElement);
}

declare global {
  interface HTMLElementTagNameMap {
    "valle-player": VallePlayerElement;
  }
}
