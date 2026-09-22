export { StudioTransport, formatTimecode, type StudioTransportIntent } from "../../shared/transport.ts";
import { LitElement, html, type TemplateResult } from "lit";
import { hydrateIcons } from "../../shared/icons.ts";

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
  fixedStart?: boolean;
  fixedEnd?: boolean;
  readOnly?: boolean;
  error?: boolean;
  media?: { kind: "video" | "audio"; assetId: string; sourceStartS: number; sourceDurationS: number };
}

export interface TimelineTrackView {
  id: string;
  kind: string;
  name?: string;
  indexLabel?: string;
  selected?: boolean;
  clips: ReadonlyArray<TimelineClipView>;
}

export interface TimelineViewModel {
  widthPx: number;
  laneWidthPx: number;
  labelWidthPx: number;
  playheadLeftPx: number;
  ticks: ReadonlyArray<TimelineTickView>;
  tracks: ReadonlyArray<TimelineTrackView>;
  emptyMessage?: string;
  showPlayhead?: boolean;
}

export interface InspectorValueRow {
  kind: "value";
  label: string;
  value: string;
}

export interface InspectorFieldRow {
  kind: "number" | "textarea" | "color" | "select" | "checkbox";
  key: string;
  label: string;
  value: string | number | boolean;
  min?: number;
  max?: number;
  step?: number;
  unit?: string;
  prefix?: string;
  values?: ReadonlyArray<string>;
  primaryText?: boolean;
  readOnly?: boolean;
}

export type InspectorRow = InspectorValueRow | InspectorFieldRow;

export interface InspectorSectionView {
  title?: string;
  rows: ReadonlyArray<InspectorRow>;
}

export interface InspectorViewModel {
  errorMessage?: string | null;
  clipId: string | null;
  kindLabel: string;
  kindClass?: string;
  title: string;
  subtitle?: string;
  summary?: ReadonlyArray<{ label: string; value: string }>;
  sections: ReadonlyArray<InspectorSectionView>;
  rawJson?: string;
  motionSource?: string;
  canDelete?: boolean;
  emptyHint?: string;
}

export type StudioInspectorIntent =
  | { type: "edit"; clipId: string; key: string; value: string | number | boolean }
  | { type: "edit-end"; clipId: string; key: string }
  | { type: "open-motion"; clipId: string }
  | { type: "delete"; clipId: string };

export interface StudioProjectControlsState {
  visible: boolean;
  dirty: boolean;
  saving?: boolean;
  conflictMessage: string | null;
}

export type StudioProjectIntent =
  | { type: "save" }
  | { type: "reload" };

export class StudioTimeline extends LitElement {
  static properties = {
    viewModel: { attribute: false },
  };

  declare viewModel: TimelineViewModel | null;

  #seekPointerId: number | null = null;

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
    hydrateIconsIn(this);
  }

  protected render(): TemplateResult {
    const model = this.viewModel;
    if (!model) return html``;
    const labelWidth = model.labelWidthPx;
    return html`
      <div class="timeline-content" style="width:${labelWidth + model.laneWidthPx}px">
        <div class="ruler">
          <div class="ruler-label">Track <span id="rulerUnit">sec</span></div>
          <div class="ruler-lane" id="rulerLane" style="width:${model.laneWidthPx}px"
            aria-label="Drag to seek">
            ${model.ticks.map((tick) => html`
              <span class="tick ${tick.label == null ? "minor" : ""}" style="left:${tick.leftPx}px">${tick.label ?? ""}</span>
            `)}
            <span class="playhead-grip" id="rulerGrip" aria-hidden="true"></span>
          </div>
        </div>
        ${model.emptyMessage
          ? html`<div class="empty-tracks"><strong>${model.emptyMessage}</strong></div>`
          : model.tracks.map((track) => this.#renderTrack(track, model))}
        ${model.showPlayhead === false
          ? html``
          : html`<div class="playhead" id="tlPlayhead" style="left:${model.playheadLeftPx}px"></div>`}
        <div class="drop-line" id="dropLine" hidden></div>
      </div>
    `;
  }

  #renderTrack(track: TimelineTrackView, model: TimelineViewModel): TemplateResult {
    const kindClass = kindClassOf(track.kind);
    return html`
      <div class="track-row ${kindClass}">
        <div class="track-head ${track.selected ? "selected" : ""}" title=${track.name ?? track.id}>
          <span class="icon" data-icon=${kindIconOf(track.kind)}></span>
          <span class="track-name">${track.name ?? track.id}</span>
          <span class="track-index">${track.indexLabel ?? track.kind}</span>
        </div>
        <div class="track-lane" style="width:${model.laneWidthPx}px">
          ${track.clips.map((clip) => this.#renderClip(clip))}
        </div>
      </div>
    `;
  }

  #renderClip(clip: TimelineClipView): TemplateResult {
    const kindClass = kindClassOf(clip.kind);
    const width = clip.widthPx;
    const tiny = width < 13;
    const narrow = width < 32;
    return html`
      <button
        type="button"
        class="clip ${kindClass} ${clip.selected ? "selected" : ""} ${clip.readOnly ? "readonly" : ""} ${clip.error ? "error" : ""} ${tiny ? "tiny" : narrow ? "narrow" : ""}"
        style="left:${clip.leftPx}px;width:${width}px"
        data-clip-id=${clip.id}
        title=${clip.title}
        aria-label=${clip.label}
        aria-pressed=${clip.selected}
      >
        <span class="clip-title">
          <span class="icon" data-icon=${kindIconOf(clip.kind)}></span>
          <span class="clip-text">${clip.label}</span>
          ${clip.readOnly ? html`<span class="icon" data-icon="lock" aria-label="Read only"></span>` : ""}
        </span>
        <span class="clip-body">
          ${clip.media
            ? html`<canvas class="tl-media" data-kind=${clip.media.kind} data-asset-id=${clip.media.assetId}></canvas>`
            : ""}
        </span>
        ${clip.timingEditable && !clip.readOnly && width >= 32
          ? html`
              ${clip.fixedStart ? "" : html`<span class="trim left" data-edge="left"></span>`}
              ${clip.fixedEnd ? "" : html`<span class="trim right" data-edge="right"></span>`}
            `
          : ""}
      </button>
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
    hydrateIconsIn(this);
  }

  protected render(): TemplateResult {
    const model = this.viewModel;
    if (!model || !model.clipId) {
      return html`
        <div class="empty-inspector">
          <h2>${model?.title ?? "Nothing selected"}</h2>
          <p>${model?.emptyHint ?? "Select a clip on the stage or timeline to inspect it."}</p>
        </div>
        ${model?.summary?.length
          ? html`<section class="ins-section">
              ${model.summary.map((row) => html`
                <div class="inspector-metric"><span>${row.label}</span><span>${row.value}</span></div>
              `)}
            </section>`
          : ""}
        ${model?.rawJson
          ? html`<details class="details"><summary>Technical details</summary><code>${model.rawJson}</code></details>`
          : ""}
      `;
    }

    return html`
      ${model.errorMessage ? html`<p class="field-error" role="alert">${model.errorMessage}</p>` : ""}
      <div class="inspector-summary ${model.kindClass ?? ""}">
        <span class="icon" data-icon=${kindIconOf(model.kindClass ?? model.kindLabel)}></span>
        <div>
          <strong>${model.title}</strong>
          ${model.subtitle ? html`<small>${model.subtitle}</small>` : ""}
        </div>
      </div>
      ${model.summary?.length
        ? html`<section class="ins-section">
            ${model.summary.map((row) => html`
              <div class="inspector-metric"><span>${row.label}</span><span>${row.value}</span></div>
            `)}
          </section>`
        : ""}
      ${model.sections.map((section) => html`
        <section class="ins-section">
          ${section.title ? html`<h2 class="section-label">${section.title}</h2>` : ""}
          ${section.rows.map((row) => this.#renderRow(row, model.clipId!))}
        </section>
      `)}
      ${model.motionSource
        ? html`
            <button class="button full-width" type="button" id="insOpenMotion" @click=${this.#openMotion}>
              <span class="icon" data-icon="motion"></span>
              Edit component · ${model.motionSource}
            </button>
          `
        : ""}
      ${model.canDelete
        ? html`<button class="ins-del" type="button" @click=${this.#delete}>Delete clip</button>`
        : ""}
      ${model.rawJson
        ? html`<details class="details"><summary>Technical details</summary><code>${model.rawJson}</code></details>`
        : ""}
    `;
  }

  #renderRow(row: InspectorRow, clipId: string): TemplateResult {
    if (row.kind === "value") {
      return html`
        <div class="inspector-metric">
          <span>${row.label}</span>
          <span>${row.value}</span>
        </div>
      `;
    }
    if (row.readOnly) {
      return html`
        <div class="inspector-metric">
          <span>${row.label}</span>
          <span>${row.value}</span>
        </div>
      `;
    }
    return html`
      <label class="field">
        <span class="field-label">${row.label}</span>
        ${this.#renderField(row, clipId)}
      </label>
    `;
  }

  #renderField(field: InspectorFieldRow, clipId: string): TemplateResult {
    if (field.kind === "textarea") {
      return html`
        <textarea data-edit-key=${field.key} data-clip-id=${clipId}
          .value=${String(field.value)} @change=${this.#edit} @blur=${this.#edit}
          @keydown=${this.#fieldKeydown}></textarea>
      `;
    }
    if (field.kind === "color") {
      return html`
        <div class="color-row">
          <input type="color" data-edit-key=${field.key} data-clip-id=${clipId}
            .value=${String(field.value).slice(0, 7)} @change=${this.#edit} @blur=${this.#edit} />
          <span class="color-value">${String(field.value).toUpperCase()}</span>
        </div>
      `;
    }
    if (field.kind === "select") {
      return html`
        <select data-edit-key=${field.key} data-clip-id=${clipId}
          .value=${String(field.value)} @change=${this.#edit} @blur=${this.#edit}>
          ${(field.values ?? []).map((value) => html`<option value=${value}>${value}</option>`)}
        </select>
      `;
    }
    if (field.kind === "checkbox") {
      return html`
        <input type="checkbox" data-edit-key=${field.key} data-clip-id=${clipId}
          .checked=${Boolean(field.value)} @change=${this.#edit} @blur=${this.#edit} />
      `;
    }
    return html`
      <span class="number-field">
        ${field.prefix ? html`<span class="field-prefix">${field.prefix}</span>` : ""}
        <input type="number" aria-label=${[field.label, field.unit].filter(Boolean).join(" ")} data-edit-key=${field.key} data-clip-id=${clipId}
          .value=${String(field.value)}
          min=${field.min ?? ""} max=${field.max ?? ""} step=${field.step ?? ""}
          @change=${this.#edit} @blur=${this.#edit}
          @keydown=${this.#fieldKeydown} />
        ${field.unit ? html`<span class="field-unit">${field.unit}</span>` : ""}
      </span>
    `;
  }

  #emit(intent: StudioInspectorIntent): void {
    this.dispatchEvent(new CustomEvent("studio-inspector-intent", {
      detail: intent,
      bubbles: true,
      composed: true,
    }));
  }

  #edit = (event: Event): void => {
    const input = event.currentTarget as HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement;
    const key = input.dataset.editKey;
    const clipId = input.dataset.clipId;
    if (!key || !clipId) return;
    const previous = this.viewModel?.sections.flatMap((section) => section.rows).find((row) => row.kind !== "value" && row.key === key);
    if (previous && String(previous.value) === input.value && input.type !== "checkbox") return;
    if (input instanceof HTMLInputElement && input.type === "number") {
      if (input.value === "" || !input.validity.valid) { input.setAttribute("aria-invalid", "true"); input.title = input.validationMessage || "Enter a valid number"; return; }
      const parsed = Number(input.value);
      if (!Number.isFinite(parsed)) return;
      input.removeAttribute("aria-invalid"); input.title = "";
      this.#emit({ type: "edit", clipId, key, value: parsed });
      return;
    }
    if (input instanceof HTMLInputElement && input.type === "checkbox") {
      this.#emit({ type: "edit", clipId, key, value: input.checked });
      return;
    }
    const value = input.type === "color" && previous && String(previous.value).length === 9
      ? input.value + String(previous.value).slice(7) : input.value;
    this.#emit({ type: "edit", clipId, key, value });
  };

  #editEnd = (event: Event): void => {
    const input = event.currentTarget as HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement;
    const key = input.dataset.editKey;
    const clipId = input.dataset.clipId;
    if (!key || !clipId) return;
    this.#emit({ type: "edit-end", clipId, key });
  };

  #fieldKeydown = (event: KeyboardEvent): void => {
    if (event.key === "Escape") {
      const input = event.currentTarget as HTMLInputElement | HTMLTextAreaElement;
      const field = this.viewModel?.sections.flatMap((section) => section.rows)
        .find((row) => row.kind !== "value" && row.key === input.dataset.editKey);
      if (field) input.value = String(field.value);
      input.removeAttribute("aria-invalid"); input.title = "";
      input.blur();
      event.stopPropagation();
      return;
    }
    if (event.key === "Enter" && !event.shiftKey) {
      (event.currentTarget as HTMLInputElement).blur();
    }
  };

  #delete = (): void => {
    if (this.viewModel?.clipId) {
      this.#emit({ type: "delete", clipId: this.viewModel.clipId });
    }
  };

  #openMotion = (): void => {
    if (this.viewModel?.clipId) {
      this.#emit({ type: "open-motion", clipId: this.viewModel.clipId });
    }
  };
}

export class StudioProjectControls extends LitElement {
  static properties = {
    controlsState: { attribute: false },
  };

  declare controlsState: StudioProjectControlsState;

  constructor() {
    super();
    this.controlsState = { visible: false, dirty: false, saving: false, conflictMessage: null };
  }

  protected createRenderRoot(): this {
    return this;
  }

  sync(state: StudioProjectControlsState): void {
    this.controlsState = state;
    this.requestUpdate();
    this.performUpdate();
    hydrateIconsIn(this);
  }

  protected render(): TemplateResult {
    const state = this.controlsState;
    if (!state.visible) return html``;
    return html`
      ${state.conflictMessage ? html`<button class="button" type="button" @click=${() => this.#emit("reload")}>Discard draft and reload</button>` : ""}
      <button class="button primary" id="saveBtn" type="button"
        ?disabled=${!state.dirty || state.saving === true}
        @click=${() => this.#emit("save")}>
        ${state.saving ? "Saving…" : "Save"}
      </button>
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

export function kindClassOf(kind: string): string {
  const value = kind.toLowerCase();
  if (value.includes("motion") || value.includes("lottie")) return "kind-motion";
  if (value.includes("audio")) return "kind-audio";
  if (value.includes("caption") || value.includes("text")) return "kind-caption";
  if (value.includes("effect") || value.includes("adjustment")) return "kind-effect";
  return "kind-visual";
}

export function kindIconOf(kind: string): string {
  const value = kind.toLowerCase();
  if (value.includes("motion") || value.includes("lottie")) return "motion";
  if (value.includes("audio")) return "music";
  if (value.includes("caption") || value.includes("text")) return "text";
  if (value.includes("effect") || value.includes("adjustment")) return "effect";
  return "film";
}

function hydrateIconsIn(root: ParentNode): void { hydrateIcons(root); }

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
    "studio-timeline": StudioTimeline;
    "studio-inspector": StudioInspector;
    "studio-project-controls": StudioProjectControls;
  }
}
