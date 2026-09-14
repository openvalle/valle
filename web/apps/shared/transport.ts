import { LitElement, html, type TemplateResult } from "lit";
import { hydrateIcons } from "./icons.ts";

export type StudioTransportIntent =
  | { type: "toggle-play" }
  | { type: "pause" }
  | { type: "play" }
  | { type: "seek"; timeS: number }
  | { type: "seek-frame"; frame: number }
  | { type: "first-frame" }
  | { type: "last-frame" }
  | { type: "loop"; value: boolean }
  | { type: "mute"; value: boolean };

export class StudioTransport extends LitElement {
  static properties = {
    playing: { state: true },
    looping: { state: true },
    muted: { state: true },
    frame: { state: true },
    totalFrames: { state: true },
    fps: { state: true },
    timeS: { state: true },
    durationS: { state: true },
    disabled: { type: Boolean },
  };

  declare playing: boolean;
  declare looping: boolean;
  declare muted: boolean;
  declare frame: number;
  declare totalFrames: number;
  declare fps: number;
  declare timeS: number;
  declare durationS: number;
  declare disabled: boolean;

  #scrubbing = false;

  constructor() {
    super();
    this.playing = false;
    this.looping = true;
    this.muted = false;
    this.frame = 0;
    this.totalFrames = 0;
    this.fps = 30;
    this.timeS = 0;
    this.durationS = 0;
    this.disabled = false;
  }

  connectedCallback(): void {
    super.connectedCallback();
    window.addEventListener("keydown", this.#onKeydown);
  }

  disconnectedCallback(): void {
    window.removeEventListener("keydown", this.#onKeydown);
    super.disconnectedCallback();
  }

  protected createRenderRoot(): this {
    return this;
  }

  configure(durationS: number, fps: number): void {
    this.durationS = Math.max(0, durationS);
    this.fps = fps > 0 ? fps : 30;
    this.totalFrames = Math.max(0, Math.round(this.durationS * this.fps));
    this.requestUpdate();
  }

  sync(timeS: number, durationS: number, fps: number, playing: boolean): void {
    this.timeS = Math.max(0, timeS);
    this.durationS = Math.max(0, durationS);
    this.fps = fps > 0 ? fps : this.fps;
    this.totalFrames = Math.max(0, Math.round(this.durationS * this.fps));
    this.frame = Math.min(Math.max(0, this.totalFrames - 1), Math.max(0, Math.round(this.timeS * this.fps)));
    this.playing = playing;
    this.requestUpdate();
  }

  protected render(): TemplateResult {
    const timecode = formatTimecode(this.timeS);
    // Reserve the largest readout for this timeline, independent of the current frame.
    const frameDigits = Math.max(3, String(this.totalFrames).length);
    const timecodeWidth = formatTimecode(this.durationS).length;
    return html`
      <div class="transport-side">
        <button class="icon-button" type="button" title="First frame (Home)" aria-label="First frame"
          ?disabled=${this.disabled} @click=${this.#first} data-icon="skip-back"></button>
        <button class="icon-button" type="button" title="Previous frame (←)" aria-label="Previous frame"
          ?disabled=${this.disabled} @click=${this.#prev} data-icon="step-back"></button>
      </div>
      <div class="transport-middle" style=${`--transport-frame-digits: ${frameDigits}; --transport-time-width: ${timecodeWidth}ch`}>
        <button class="play-button" type="button" title="Play / pause (Space)" aria-label=${this.playing ? "Pause" : "Play"}
          ?disabled=${this.disabled} @click=${this.#toggle} data-icon=${this.playing ? "pause" : "play"}></button>
        <button class="icon-button" type="button" title="Next frame (→)" aria-label="Next frame"
          ?disabled=${this.disabled} @click=${this.#next} data-icon="step-forward"></button>
        <button class="icon-button" type="button" title="Last frame (End)" aria-label="Last frame"
          ?disabled=${this.disabled} @click=${() => this.#emit({ type: "last-frame" })} data-icon="skip-forward"></button>
        <button class="icon-button" type="button" title="Loop" aria-label="Loop" aria-pressed=${this.looping}
          ?disabled=${this.disabled} @click=${this.#loop} data-icon="repeat"></button>
        <span class="transport-separator"></span>
        <label class="time-input-wrap">
          <input id="frameInput" type="number" min="0" max=${Math.max(0, this.totalFrames - 1)} .value=${String(this.frame)}
            aria-label="Current frame" ?disabled=${this.disabled} @change=${this.#onFrameInput} @blur=${this.#onFrameInput} @keydown=${this.#frameKeydown} />
          <span>f</span>
        </label>
        <span class="time-total">/ ${this.totalFrames} f</span>
        <span class="time-code">${timecode}</span>
      </div>
      <div class="transport-side end">
        <button class="icon-button" type="button" title=${this.muted ? "Unmute" : "Mute"} aria-label=${this.muted ? "Unmute" : "Mute"}
          aria-pressed=${this.muted} ?disabled=${this.disabled} @click=${this.#mute}
          data-icon=${this.muted ? "volume-off" : "volume"}></button>
      </div>
    `;
  }

  protected updated(): void {
    hydrateIcons(this);
  }

  #emit(intent: StudioTransportIntent): void {
    this.dispatchEvent(new CustomEvent("studio-transport-intent", {
      detail: intent,
      bubbles: true,
      composed: true,
    }));
  }

  #toggle = (): void => this.#emit({ type: "toggle-play" });
  #first = (): void => this.#emit({ type: "first-frame" });
  #prev = (): void => this.#emit({ type: "seek-frame", frame: Math.max(0, this.frame - 1) });
  #next = (): void => this.#emit({ type: "seek-frame", frame: Math.min(Math.max(0, this.totalFrames - 1), this.frame + 1) });
  #loop = (): void => {
    this.looping = !this.looping;
    this.#emit({ type: "loop", value: this.looping });
  };
  #mute = (): void => {
    this.muted = !this.muted;
    this.#emit({ type: "mute", value: this.muted });
  };

  #frameKeydown = (event: KeyboardEvent): void => {
    if (event.key === "Escape") { (event.currentTarget as HTMLInputElement).value = String(this.frame); event.stopPropagation(); }
    if (event.key === "Enter" || event.key === "Escape") (event.currentTarget as HTMLInputElement).blur();
  };
  #onFrameInput = (event: Event): void => {
    const input = event.currentTarget as HTMLInputElement;
    const frame = Number(input.value);
    if (input.value.trim() === "" || !Number.isFinite(frame)) { input.value = String(this.frame); return; }
    if (frame === this.frame) return;
    this.frame = Math.min(Math.max(0, this.totalFrames - 1), Math.max(0, Math.round(frame)));
    input.value = String(this.frame);
    this.#emit({ type: "seek-frame", frame: this.frame });
  };

  #onKeydown = (event: KeyboardEvent): void => {
    if (event.metaKey || event.ctrlKey || event.altKey) return;
    if (this.disabled || event.isComposing || isEditableTarget(event)) return;
    const actions: Record<string, () => void> = {
      " ": this.#toggle, k: this.#toggle, ArrowLeft: this.#prev, ArrowRight: this.#next,
      Home: this.#first, End: () => this.#emit({ type: "last-frame" }),
    };
    const action = actions[event.key];
    if (action) { event.preventDefault(); action(); }
  };
}

export function formatTimecode(seconds: number): string {
  const centiseconds = Math.round(Math.max(0, Number(seconds) || 0) * 100);
  const hours = Math.floor(centiseconds / 360000);
  const minutes = Math.floor(centiseconds / 6000) % 60;
  const whole = Math.floor(centiseconds / 100) % 60;
  const tail = `${pad(minutes)}:${pad(whole)}.${pad(centiseconds % 100)}`;
  return hours > 0 ? `${pad(hours)}:${tail}` : tail;
}

function pad(value: number): string {
  return String(value).padStart(2, "0");
}

export function isEditableTarget(event: KeyboardEvent): boolean {
  return event.composedPath().some((node) => {
    if (!(node instanceof HTMLElement)) return false;
    return node.matches("input, textarea, select") || node.isContentEditable;
  });
}


if (!customElements.get("studio-transport")) customElements.define("studio-transport", StudioTransport);
declare global { interface HTMLElementTagNameMap { "studio-transport": StudioTransport } }
