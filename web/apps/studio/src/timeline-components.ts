import { LitElement, html, type TemplateResult } from "lit";

export interface StudioMetaChip {
  text: string;
  tone?: "default" | "live" | "warning";
  strong?: boolean;
}

export type StudioTransportIntent =
  | { type: "toggle-play" }
  | { type: "pause" }
  | { type: "seek"; timeS: number };

export interface TimelineTickView {
  leftPx: number;
  label: string | null;
}

export interface TimelineClipView {
  id: string;
  kind: string;
  leftPx: number;
  widthPx: number;
  title: string;
  label: string;
  selected: boolean;
  timingEditable?: boolean;
  media?: { kind: "video" | "audio"; assetId: string };
}

export interface TimelineTrackView {
  id: string;
  kind: string;
  clips: ReadonlyArray<TimelineClipView>;
}

export interface TimelineViewModel {
  widthPx: number;
  laneWidthPx: number;
  playheadLeftPx: number;
  ticks: ReadonlyArray<TimelineTickView>;
  tracks: ReadonlyArray<TimelineTrackView>;
}

export interface InspectorValueRow {
  kind: "value";
  label: string;
  value: string;
}

export interface InspectorFieldRow {
  kind: "number" | "textarea" | "color";
  key: string;
  label: string;
  value: string | number;
  min?: number;
  max?: number;
  step?: number;
  primaryText?: boolean;
}

export type InspectorRow = InspectorValueRow | InspectorFieldRow;

export interface InspectorSectionView {
  title?: string;
  rows: ReadonlyArray<InspectorRow>;
}

export interface InspectorViewModel {
  clipId: string;
  sections: ReadonlyArray<InspectorSectionView>;
  rawJson: string;
  motionSource?: string;
}

export type StudioInspectorIntent =
  | { type: "edit"; clipId: string; key: string; value: string | number }
  | { type: "open-motion"; clipId: string }
  | { type: "delete"; clipId: string };

export interface StudioProjectControlsState {
  visible: boolean;
  dirty: boolean;
  conflictMessage: string | null;
}

export type StudioProjectIntent = { type: "save" | "reload" };

export class StudioHeaderMeta extends LitElement {
  static properties = {
    chips: { attribute: false },
  };

  declare chips: ReadonlyArray<StudioMetaChip>;

  constructor() {
    super();
    this.chips = [];
  }

  protected createRenderRoot(): this {
    return this;
  }

  protected render(): TemplateResult {
    return html`${this.chips.map((chip) => html`
      <span class="chip ${chip.tone === "warning" ? "warn" : chip.tone === "live" ? "live" : ""}">
        ${chip.strong ? html`<b>${chip.text}</b>` : chip.text}
      </span>
    `)}`;
  }
}

export class StudioTransport extends LitElement {
  protected createRenderRoot(): this {
    return this;
  }

  connectedCallback(): void {
    super.connectedCallback();
    window.addEventListener("keydown", this.#onKeydown);
  }

  disconnectedCallback(): void {
    window.removeEventListener("keydown", this.#onKeydown);
    super.disconnectedCallback();
  }

  protected render(): TemplateResult {
    return html`
      <button class="ctl" id="playToggle" type="button" title="Play / pause (Space)"
        aria-label="Play" @click=${this.#toggle}>
        <svg id="playIcon" viewBox="0 0 24 24" fill="currentColor"><path d="M8 5.5v13l11-6.5z"></path></svg>
      </button>
      <input id="scrub" type="range" min="0" max="1" value="0" step="0.001"
        aria-label="Seek" style="--pct:0%" @pointerdown=${this.#pause} @input=${this.#seek} />
      <span class="time"><span class="cur" id="tCur">0:00.000</span> / <span id="tDur">0:00.000</span>
        &nbsp;·&nbsp;<span id="tFrame">f0/0</span></span>
    `;
  }

  configure(durationS: number, _fps: number): void {
    const scrub = this.#scrub();
    scrub.max = String(Math.max(0.001, durationS));
    scrub.step = "0.001";
  }

  sync(timeS: number, durationS: number, _fps: number, playing: boolean): void {
    const scrub = this.#scrub();
    scrub.value = String(timeS);
    scrub.style.setProperty("--pct", `${durationS > 0 ? (timeS / durationS) * 100 : 0}%`);
    this.#required("tCur").textContent = formatTime(timeS);
    this.#required("tDur").textContent = formatTime(durationS);
    this.#required("tFrame").textContent = "compiled frame";
    this.#required("playIcon").setAttribute(
      "d",
      playing ? "M7 5h3.5v14H7zM13.5 5H17v14h-3.5z" : "M8 5.5v13l11-6.5z",
    );
    this.#required("playToggle").setAttribute("aria-label", playing ? "Pause" : "Play");
  }

  #required(id: string): HTMLElement {
    const element = this.querySelector<HTMLElement>(`#${id}`);
    if (!element) throw new Error(`missing Studio transport element #${id}`);
    return element;
  }

  #scrub(): HTMLInputElement {
    return this.#required("scrub") as HTMLInputElement;
  }

  #emit(intent: StudioTransportIntent): void {
    this.dispatchEvent(new CustomEvent("studio-transport-intent", {
      detail: intent,
      bubbles: true,
      composed: true,
    }));
  }

  #toggle = (): void => this.#emit({ type: "toggle-play" });
  #pause = (): void => this.#emit({ type: "pause" });
  #seek = (event: Event): void => {
    this.#emit({ type: "seek", timeS: Number((event.currentTarget as HTMLInputElement).value) });
  };
  #onKeydown = (event: KeyboardEvent): void => {
    if (event.metaKey || event.ctrlKey || event.altKey) return;
    if (event.key !== " " && event.key !== "k") return;
    event.preventDefault();
    this.#toggle();
  };
}

export class StudioTimeline extends LitElement {
  static properties = {
    viewModel: { attribute: false },
  };

  declare viewModel: TimelineViewModel | null;

  constructor() {
    super();
    this.viewModel = null;
  }

  protected createRenderRoot(): this {
    return this;
  }

  renderTimeline(viewModel: TimelineViewModel): void {
    this.viewModel = viewModel;
    this.requestUpdate();
    this.performUpdate();
  }

  protected render(): TemplateResult {
    const model = this.viewModel;
    if (!model) return html``;
    return html`
      <div class="tl" id="tl" style="width:${model.widthPx}px">
        <div class="tl-ruler" id="tlRuler">
          ${model.ticks.map((tick) => html`
            <div class=${tick.label == null ? "tl-tick-minor" : "tl-tick"}
              style="left:${tick.leftPx}px">${tick.label ?? ""}</div>
          `)}
        </div>
        ${model.tracks.map((track) => html`
          <div class="tl-row kind-${track.kind}">
            <div class="tl-label"><span class="tl-kind">${track.kind}</span><span class="tl-tid">${track.id}</span></div>
            <div class="tl-lane" style="width:${model.laneWidthPx}px">
              ${track.clips.map((clip) => html`
                <div class="tl-clip kind-${clip.kind} ${clip.selected ? "selected" : ""}"
                  style="left:${clip.leftPx}px;width:${clip.widthPx}px"
                  data-clip-id=${clip.id} title=${clip.title}>
                  ${clip.media ? html`<canvas class="tl-media" data-kind=${clip.media.kind}
                    data-asset-id=${clip.media.assetId}></canvas>` : ""}
                  <span>${clip.label}</span>
                  ${clip.timingEditable
                    ? html`<div class="tl-trim w"></div><div class="tl-trim e"></div>`
                    : ""}
                </div>
              `)}
            </div>
          </div>
        `)}
        <div class="tl-playhead" id="tlPlayhead" style="left:${model.playheadLeftPx}px"></div>
      </div>
    `;
  }
}

export class StudioInspector extends LitElement {
  static properties = {
    viewModel: { attribute: false },
  };

  declare viewModel: InspectorViewModel | null;

  constructor() {
    super();
    this.viewModel = null;
  }

  protected createRenderRoot(): this {
    return this;
  }

  renderInspector(viewModel: InspectorViewModel | null): void {
    this.viewModel = viewModel;
    this.requestUpdate();
    this.performUpdate();
  }

  protected render(): TemplateResult {
    const model = this.viewModel;
    if (!model) return html`<div class="empty">Nothing selected. Click the stage or a timeline clip.</div>`;
    return html`
      ${model.sections.map((section) => html`
        ${section.title ? html`<div class="ins-sec">${section.title}</div>` : ""}
        ${section.rows.map((row) => this.#renderRow(row))}
      `)}
      ${model.motionSource ? html`
        <button id="insOpenMotion" type="button" @click=${this.#openMotion}>
          Edit component · ${model.motionSource}
        </button>
      ` : ""}
      <button id="insDelete" class="ins-del" type="button" @click=${this.#delete}>Delete clip</button>
      <details><summary>raw clip JSON</summary><pre>${model.rawJson}</pre></details>
    `;
  }

  #renderRow(row: InspectorRow): TemplateResult {
    if (row.kind === "value") {
      return html`<div class="ins-row"><span class="ins-k">${row.label}</span><span class="ins-v">${row.value}</span></div>`;
    }
    return html`
      <div class="ins-row">
        <label class="ins-k">${row.label}</label>
        <span class="ins-v">${this.#renderField(row)}</span>
      </div>
    `;
  }

  #renderField(field: InspectorFieldRow): TemplateResult {
    if (field.kind === "textarea") {
      return html`<textarea id=${field.primaryText ? "insText" : ""} data-edit-key=${field.key}
        .value=${String(field.value)} @input=${this.#edit}></textarea>`;
    }
    if (field.kind === "color") {
      return html`<input type="color" data-edit-key=${field.key} .value=${String(field.value)} @input=${this.#edit} />`;
    }
    return html`<input type="number" data-edit-key=${field.key} .value=${String(field.value)}
      min=${field.min ?? ""} max=${field.max ?? ""} step=${field.step ?? ""} @change=${this.#edit} />`;
  }

  #emit(intent: StudioInspectorIntent): void {
    this.dispatchEvent(new CustomEvent("studio-inspector-intent", {
      detail: intent,
      bubbles: true,
      composed: true,
    }));
  }

  #edit = (event: Event): void => {
    const model = this.viewModel;
    const input = event.currentTarget as HTMLInputElement | HTMLTextAreaElement;
    const key = input.dataset.editKey;
    if (!model || !key) return;
    if (input instanceof HTMLInputElement && input.type === "number") {
      const parsed = Number(input.value);
      if (!Number.isFinite(parsed)) return;
      const min = input.min === "" ? Number.NEGATIVE_INFINITY : Number(input.min);
      const max = input.max === "" ? Number.POSITIVE_INFINITY : Number(input.max);
      const value = Math.min(max, Math.max(min, parsed));
      input.value = String(value);
      this.#emit({ type: "edit", clipId: model.clipId, key, value });
      return;
    }
    if (input instanceof HTMLTextAreaElement && input.value === "") return;
    this.#emit({ type: "edit", clipId: model.clipId, key, value: input.value });
  };

  #delete = (): void => {
    if (this.viewModel) this.#emit({ type: "delete", clipId: this.viewModel.clipId });
  };

  #openMotion = (): void => {
    if (this.viewModel) this.#emit({ type: "open-motion", clipId: this.viewModel.clipId });
  };
}

export class StudioProjectControls extends LitElement {
  static properties = {
    controlsState: { attribute: false },
  };

  declare controlsState: StudioProjectControlsState;

  constructor() {
    super();
    this.controlsState = { visible: false, dirty: false, conflictMessage: null };
  }

  protected createRenderRoot(): this {
    return this;
  }

  sync(state: StudioProjectControlsState): void {
    this.controlsState = state;
    this.requestUpdate();
    this.performUpdate();
  }

  protected render(): TemplateResult {
    const state = this.controlsState;
    return html`
      <button id="saveBtn" class="save-btn ${state.dirty ? "dirty" : ""}" type="button"
        ?hidden=${!state.visible} ?disabled=${!state.dirty} @click=${() => this.#emit("save")}>
        ${state.dirty ? "Save *" : "Saved"}
      </button>
      <span id="conflictBar" class="conflict-bar" ?hidden=${state.conflictMessage == null}>
        <span id="conflictText">${state.conflictMessage ?? ""}</span>
        <button id="conflictReload" type="button" @click=${() => this.#emit("reload")}>Discard draft and reload</button>
      </span>
    `;
  }

  #emit(type: StudioProjectIntent["type"]): void {
    this.dispatchEvent(new CustomEvent("studio-project-intent", {
      detail: { type } satisfies StudioProjectIntent,
      bubbles: true,
      composed: true,
    }));
  }
}

function formatTime(value: number): string {
  const seconds = Math.max(0, Number(value) || 0);
  const minutes = Math.floor(seconds / 60);
  const rest = seconds - minutes * 60;
  return `${minutes}:${rest.toFixed(3).padStart(6, "0")}`;
}

if (!customElements.get("studio-header-meta")) {
  customElements.define("studio-header-meta", StudioHeaderMeta);
}
if (!customElements.get("studio-transport")) {
  customElements.define("studio-transport", StudioTransport);
}
if (!customElements.get("studio-timeline")) {
  customElements.define("studio-timeline", StudioTimeline);
}
if (!customElements.get("studio-inspector")) {
  customElements.define("studio-inspector", StudioInspector);
}
if (!customElements.get("studio-project-controls")) {
  customElements.define("studio-project-controls", StudioProjectControls);
}

declare global {
  interface HTMLElementTagNameMap {
    "studio-header-meta": StudioHeaderMeta;
    "studio-transport": StudioTransport;
    "studio-timeline": StudioTimeline;
    "studio-inspector": StudioInspector;
    "studio-project-controls": StudioProjectControls;
  }
}
