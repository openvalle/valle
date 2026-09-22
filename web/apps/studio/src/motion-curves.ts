import { LitElement, html, svg, type PropertyValues } from "lit";
import type { MotionPropertyRequest, MotionPropertySamples } from "valle-engine/compiler";
import type { GoodMotionContext } from "./motion-preview.ts";

export interface MotionCurveInputs {
  context: GoodMotionContext;
  props: Record<string, { kind: string; value: unknown }>;
  selectedKey: string | null;
}

const record = (v: unknown): Record<string, unknown> => v && typeof v === "object" ? v as Record<string, unknown> : {};
const supported = new Set(["opacity", "translate", "scale", "rotate"]);

export function curveRequest(inputs: MotionCurveInputs, node: string, start: number, end: number): MotionPropertyRequest {
  const c = inputs.context;
  const controls = record(record(c.artifact.controls).props);
  const props = Object.fromEntries(Object.entries(inputs.props).map(([name, value]) => {
    const schema = record(controls[name]);
    const kind = value.kind === "value" ? record(schema.default).kind ?? record(schema.control).kind : value.kind;
    return [name, { kind: kind === "select" ? "enum" : kind, value: value.value }];
  }));
  return { node, startFrame: Math.max(0, Math.round(start)), endFrame: Math.min(c.durationFrames - 1, Math.round(end)),
    maxPoints: 240, durationFrames: c.durationFrames, fps: `${c.fps.num}/${c.fps.den}`,
    props,
    viewport: [c.viewport.width, c.viewport.height] };
}

export class StudioMotionCurves extends LitElement {
  static properties = { inputs: { attribute: false }, open: { state: true }, samples: { state: true }, error: { state: true },
    selected: { state: true }, start: { state: true }, end: { state: true }, axis: { state: true } };
  declare inputs: MotionCurveInputs | null;
  declare open: boolean;
  declare samples: MotionPropertySamples | null;
  declare error: string;
  declare selected: string;
  declare start: number;
  declare end: number;
  declare axis: "frames" | "seconds";
  constructor() {
    super(); this.inputs = null; this.open = false; this.samples = null; this.error = "";
    this.selected = ""; this.start = 0; this.end = 0; this.axis = "frames";
  }
  #worker: Worker | null = null;
  #fingerprint = "";
  #source = "";
  #current = 0;
  #rangeInitialized = false;
  #externalSelection: string | null = null;
  #visibility: ResizeObserver | null = null;
  #visible = true;
  connectedCallback(): void {
    super.connectedCallback();
    this.#visibility = new ResizeObserver(() => {
      const visible = Boolean(this.getClientRects().length && this.getBoundingClientRect().width > 0);
      if (visible === this.#visible) return;
      this.#visible = visible;
      if (!visible) {
        this.#cancel(); this.#fingerprint = "";
      } else requestAnimationFrame(() => { if (this.isConnected) this.requestUpdate(); });
    });
    this.#visibility.observe(this);
  }
  protected createRenderRoot(): this { return this; }
  disconnectedCallback(): void { this.#visibility?.disconnect(); this.#cancel(); super.disconnectedCallback(); }
  #cancel(): void { this.#worker?.terminate(); this.#worker = null; }
  protected updated(changed: PropertyValues): void {
    if (!this.inputs) { this.#cancel(); this.#fingerprint = ""; return; }
    const c = this.inputs.context;
    if (changed.has("inputs")) {
      const keys = this.#nodes().map(n => String(n.key));
      if (this.inputs.selectedKey !== this.#externalSelection && this.inputs.selectedKey && keys.includes(this.inputs.selectedKey)) this.selected = this.inputs.selectedKey;
      this.#externalSelection = this.inputs.selectedKey;
      if (!keys.includes(this.selected)) this.selected = keys[0] ?? "";
      if (!this.#rangeInitialized || this.#source !== c.input) {
        this.start = 0; this.end = c.durationFrames - 1; this.#rangeInitialized = true; this.#source = c.input;
      } else { this.end = Math.min(this.end, c.durationFrames - 1); this.start = Math.min(this.start, this.end); }
    }
    if (!this.open || !this.getClientRects().length || this.getBoundingClientRect().width === 0) { this.#cancel(); this.#fingerprint = ""; return; }
    const request = curveRequest(this.inputs, this.selected, this.start, this.end);
    const identity = JSON.stringify([c.generation, c.artifactDigest, request]);
    if (identity !== this.#fingerprint) {
      this.#fingerprint = identity; this.#cancel(); this.samples = null; this.error = "";
      if (!this.selected) { this.error = "No explicit opacity / translate / scale / rotate bindings."; return; }
      if (request.startFrame > request.endFrame) { this.error = "This range has no frames."; return; }
      const worker = new Worker(new URL("/runtime/workers/motion-curves.js", location.href), { type: "module" });
      this.#worker = worker;
      worker.onmessage = (event: MessageEvent<{ samples?: MotionPropertySamples; error?: string }>) => {
        if (this.#worker !== worker || this.#fingerprint !== identity || !this.open) return;
        this.samples = event.data.samples ?? null; this.error = event.data.error ?? ""; this.#cancel();
      };
      worker.onerror = (event) => { if (this.#worker === worker) { this.error = event.message; this.#cancel(); } };
      worker.postMessage({ artifact: c.artifact, request, runtimeAssets: c.runtimeAssets, runtimeBaseUrl: new URL(c.runtimeBaseUrl || "/", location.href).href });
    }
    this.syncFrame(this.#current);
  }
  #nodes(): Array<Record<string, unknown>> {
    const nodes = this.inputs?.context.artifact.nodes;
    return Array.isArray(nodes) ? nodes.map(record).filter(n => Array.isArray(n.styles) && n.styles.some(s => supported.has(String(record(s).property)))) : [];
  }
  syncFrame(frame: number): void {
    this.#current = Math.round(frame);
    const x = 28 + (frame - this.start) / Math.max(1, this.end - this.start) * 256;
    this.querySelectorAll<SVGLineElement>("[data-playhead]").forEach(line => {
      line.setAttribute("x1", String(x)); line.setAttribute("x2", String(x));
      line.style.visibility = frame < this.start || frame > this.end ? "hidden" : "visible";
    });
    this.querySelectorAll<HTMLOutputElement>("output[data-property]").forEach(output => {
      const channel = this.samples?.channels.find(c => c.property === output.dataset.property);
      const point = channel?.samples.find(p => p.frame === this.#current);
      output.textContent = point ? `${point.frame} f · ${point.value.map(n => n.toFixed(3)).join(", ")} ${channel?.unit}` : `${this.#current} f · outside sampled points`;
    });
  }
  #seek(frame: number): void { this.dispatchEvent(new CustomEvent("motion-curve-seek", { bubbles: true, composed: true, detail: { frame } })); }
  #coordinate(frame: number): string { const fps = this.inputs!.context.fps; return this.axis === "frames" ? `${frame} f` : `${(frame * fps.den / fps.num).toFixed(3)} s`; }
  protected render() {
    if (!this.inputs) return html``;
    const c = this.inputs.context;
    const source = c.sourceMap.nodes.find(n => n.key === this.selected);
    const span = record(source?.span);
    const location = source ? `${source.sourcePath ?? c.input}:${span.line ?? "?"}:${span.column ?? "?"}` : "No reliable source mapping";
    return html`<section class="ins-section motion-curves">
      <button class="button full-width" type="button" aria-expanded=${this.open} @click=${() => { this.open = !this.open; }}>Property curves ${this.open ? "−" : "+"}</button>
      ${!this.open ? "" : html`
        <div class="ins-note">${c.sourceMap.component} → child → property</div>
        <label class="field"><span class="field-label">Child / local property owner</span><select aria-label="Curve node" .value=${this.selected} @change=${(e: Event) => { this.selected = (e.target as HTMLSelectElement).value; }}>
          ${this.#nodes().map(n => { const mapping = c.sourceMap.nodes.find(m => m.key === n.key); const stack = mapping?.expansionStack;
            return html`<option value=${String(n.key)} .selected=${this.selected === n.key}>${Array.isArray(stack) && stack.length ? `${stack.join(" / ")} · ` : ""}${String(n.key)}</option>`; })}</select></label>
        <div class="curve-range">${(["start", "end"] as const).map(key => html`<label>${key} frame<input aria-label=${`Curve ${key} frame`} type="number" min="0" max=${c.durationFrames - 1} .value=${String(this[key])} @input=${(e: Event) => { this[key] = Math.max(0, Math.min(c.durationFrames - 1, Number((e.target as HTMLInputElement).value) || 0)); }} /></label>`)}
          <label>Axis<select .value=${this.axis} @change=${(e: Event) => { this.axis = (e.target as HTMLSelectElement).value as typeof this.axis; }}><option value="frames" .selected=${this.axis === "frames"}>Frames</option><option value="seconds" .selected=${this.axis === "seconds"}>Seconds</option></select></label></div>
        <p class="ins-note">${location}</p><button class="button" type="button" @click=${() => void navigator.clipboard.writeText(location)}>Copy source location</button>
        <p class="ins-note">Local authored values; parent transforms and inherited CSS are excluded. Dots are actual source frames. Lines only connect samples; they do not prove continuity.</p>
        ${this.error ? html`<p class="ins-note warning">${this.error}</p>` : !this.samples ? html`<p role="status">Sampling…</p>` : html`
          <p class="ins-note">${this.samples.frames.length} points · ${this.#coordinate(this.start)}–${this.#coordinate(this.end)} · speed ≈ adjacent-frame central difference; uncertain boundaries omitted.</p>
          ${this.samples.channels.map(channel => html`<div class="curve-channel"><strong>${channel.property}</strong>
            <p class="ins-note">${this.#propertySource(channel.property)}${channel.samples[0]?.value.length === 2 ? " · green X / blue Y" : ""}</p>
            ${channel.unavailable ? html`<p class="ins-note">${channel.unavailable}</p>` : html`
              <output data-property=${channel.property}></output>
              ${this.#chart(channel, false)}${this.#chart(channel, true)}
              ${this.#activity(channel)}
            `}</div>`)}
        `}
      `}
    </section>`;
  }
  #propertySource(property: string): string {
    const c = this.inputs!.context;
    const node = this.#nodes().find(n => n.key === this.selected);
    const style = (node?.styles as Array<Record<string, unknown>> | undefined)?.find(s => s.property === property);
    const value = record(style?.value);
    const mapping = c.sourceMap.exprs.find(m => m.id === value.expr);
    if (!mapping) return "Static / node source";
    const span = record(mapping.span), stack = mapping.expansionStack;
    return `${Array.isArray(stack) && stack.length ? `${stack.join(" → ")} · ` : ""}${mapping.sourcePath ?? c.input}:${span.line ?? "?"}:${span.column ?? "?"}`;
  }
  #activity(channel: MotionPropertySamples["channels"][number]) {
    const changes = channel.samples.filter((p, i, rows) => i > 0 && p.value.some((v, axis) => Math.abs(v - rows[i - 1]!.value[axis]!) > 1e-6));
    return html`<p class="ins-note">${changes.length ? `Observed change: ${this.#coordinate(changes[0]!.frame)}–${this.#coordinate(changes.at(-1)!.frame)} in this window; timing resolution follows sampled frames.` : "No change at sampled frames."}</p>`;
  }
  #chart(channel: MotionPropertySamples["channels"][number], speed: boolean) {
    const values = channel.samples.flatMap(p => (speed ? p.velocity : p.value) ?? []);
    const min = Math.min(0, ...values), max = Math.max(min + 1e-9, ...values), span = Math.max(1e-6, max - min);
    const x = (f: number) => 28 + (f - this.start) / Math.max(1, this.end - this.start) * 256;
    const y = (v: number) => 102 - (v - min) / span * 76;
    const dimensions = Math.max(1, ...channel.samples.map(p => p.value.length));
    const unit = `${channel.unit}${speed ? "/s ≈" : ""}`;
    return html`<svg class="curve-plot" viewBox="0 0 312 132" role="img" aria-label=${`${channel.property} ${speed ? "speed" : "value"}`} @click=${(event: MouseEvent) => {
      const rect = (event.currentTarget as SVGSVGElement).getBoundingClientRect();
      const target = this.start + (((event.clientX - rect.left) / rect.width * 312 - 28) / 256) * (this.end - this.start);
      const nearest = this.samples!.frames.reduce((a,b) => Math.abs(b-target) < Math.abs(a-target) ? b : a);
      this.#seek(nearest);
    }}>
      <text x="4" y="12">${speed ? "Speed" : "Value"} · ${unit}</text><text x="308" y="12" text-anchor="end">${min.toFixed(2)}–${max.toFixed(2)}</text>
      <line class="curve-grid" x1="28" x2="284" y1=${y(0)} y2=${y(0)} />
      ${Array.from({ length: dimensions }, (_, axis) => {
        let path = "", drawing = false;
        for (const point of channel.samples) {
          const v = (speed ? point.velocity : point.value)?.[axis];
          if (v == null || (speed && point.boundary)) { drawing = false; continue; }
          if (point.boundary) drawing = false;
          path += `${drawing ? "L" : "M"}${x(point.frame)},${y(v)} `; drawing = !point.boundary;
        }
        return svg`<path d=${path} class=${`curve-axis-${axis}`} fill="none" stroke-width="1.5" />
          ${channel.samples.map(point => {
            const v = (speed ? point.velocity : point.value)?.[axis];
            if (v == null) return svg``;
            return svg`<circle class=${point.boundary ? "curve-boundary" : `curve-axis-${axis}`} cx=${x(point.frame)} cy=${y(v)} r="2" tabindex="0" role="button" aria-label=${`Seek ${point.frame} frame`} @keydown=${(e: KeyboardEvent) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); e.stopPropagation(); this.#seek(point.frame); } }}><title>${point.frame} f · ${v.toFixed(4)} ${unit}${point.boundary ? ` · ${point.boundary}` : ""}</title></circle>`;
          })}`;
      })}
      <line data-playhead x1="28" x2="28" y1="20" y2="106" class="curve-playhead" />
      <text x="28" y="122">${this.#coordinate(this.start)}</text><text x="284" y="122" text-anchor="end">${this.#coordinate(this.end)}</text>
    </svg>`;
  }
}
customElements.define("studio-motion-curves", StudioMotionCurves);
