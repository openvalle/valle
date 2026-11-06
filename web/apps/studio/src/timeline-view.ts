// The timeline's tracks and clips, as the service lays them out. A clip is selected by a click,
// moved by dragging it, trimmed by dragging an edge; each change is asked for as an `op` event,
// the operation the service does, and the view is drawn again from the layout that follows.

import { css, html, LitElement, nothing } from "lit";

import type { Clip, Layout } from "./service.ts";

/** Pixels an edge is grabbed within. */
const EDGE = 6;

type Drag = {
  clip: Clip;
  /** The body moved, or an edge trimmed. */
  part: "body" | "head" | "tail";
  x: number;
  /** Seconds it has moved by. */
  by: number;
};

export class TimelineView extends LitElement {
  static styles = css`
    :host {
      display: block;
      user-select: none;
      color: var(--text, #eef0f4);
      font:
        11px/1.5 system-ui,
        sans-serif;
      overflow: hidden;
    }
    * {
      box-sizing: border-box;
    }
    .scroller {
      height: 100%;
      overflow: auto;
    }
    .sheet {
      min-width: 100%;
    }
    .row {
      display: grid;
      grid-template-columns: 144px minmax(0, 1fr);
    }
    .ruler-row {
      position: sticky;
      top: 0;
      z-index: 3;
    }
    .ruler {
      position: relative;
      height: 28px;
      background: #1b1d22;
      border-bottom: 1px solid #2a2d35;
      cursor: pointer;
      overflow: hidden;
    }
    .tick {
      position: absolute;
      top: 6px;
      color: #959ba8;
      font-size: 10px;
      font-variant-numeric: tabular-nums;
      padding-left: 5px;
      border-left: 1px solid #444a56;
      line-height: 22px;
    }
    .label {
      position: sticky;
      left: 0;
      z-index: 2;
      display: flex;
      align-items: center;
      gap: 9px;
      padding: 0 13px;
      border-right: 1px solid #2a2d35;
      border-bottom: 1px solid #2a2d35;
      background: #1b1d22;
      overflow: hidden;
    }
    .label span:last-child {
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }
    .label small {
      color: #959ba8;
      font-size: 9px;
      border: 1px solid #444a56;
      border-radius: 3px;
      padding: 0 3px;
    }
    .track {
      position: relative;
      height: 48px;
      border-bottom: 1px solid #2a2d35;
      background-color: #141519;
      background-image: linear-gradient(to right, #ffffff05 1px, transparent 1px);
      background-size: var(--tick) 100%;
    }
    .clip {
      position: absolute;
      top: 6px;
      bottom: 6px;
      border-radius: 5px;
      background: #2e4973;
      border: 1px solid #5b7ba8;
      overflow: hidden;
      white-space: nowrap;
      text-overflow: ellipsis;
      padding: 0 9px;
      line-height: 34px;
      cursor: grab;
      touch-action: none;
      color: #e7effd;
    }
    .clip.audio {
      background: #25594d;
      border-color: #4f8d7a;
    }
    .clip.caption {
      background: #6b512c;
      border-color: #ad854a;
    }
    .clip.off {
      opacity: 0.4;
    }
    .clip.selected {
      box-shadow: inset 0 0 0 1px #b7d2ff;
      border-color: #b7d2ff;
      background: #365888;
    }
    .clip.selected::before,
    .clip.selected::after {
      content: "";
      position: absolute;
      top: 9px;
      bottom: 9px;
      width: 2px;
      background: #b7d2ff;
      border-radius: 2px;
    }
    .clip.selected::before {
      left: 2px;
    }
    .clip.selected::after {
      right: 2px;
    }
    .clip:focus-visible {
      outline: 2px solid #b7d2ff;
      outline-offset: 2px;
    }
    .playhead {
      position: absolute;
      z-index: 1;
      top: 0;
      bottom: 0;
      width: 1px;
      background: #a8caff;
      pointer-events: none;
    }
    .ruler .playhead::before {
      content: "";
      position: absolute;
      top: 0;
      left: -4px;
      width: 9px;
      height: 10px;
      background: #a8caff;
      clip-path: polygon(0 0, 100% 0, 100% 60%, 50% 100%, 0 60%);
    }
  `;

  layout: Layout | undefined;
  selected: string | undefined;
  time = 0;
  /** Pixels a second. */
  #scale = 80;
  #drag: Drag | undefined;

  #zoom = 1;
  #observer = new ResizeObserver(() => this.#measure());

  connectedCallback(): void {
    super.connectedCallback();
    this.#observer.observe(this);
  }

  disconnectedCallback(): void {
    this.#observer.disconnect();
    super.disconnectedCallback();
  }

  set(layout: Layout | undefined, selected: string | undefined, time: number, zoom = 1): void {
    this.layout = layout;
    this.selected = selected;
    this.time = time;
    if (zoom === 1 && this.#zoom !== 1)
      this.renderRoot.querySelector(".scroller")?.scrollTo({ left: 0 });
    this.#zoom = zoom;
    this.#measure();
  }

  #measure(): void {
    this.#scale =
      (Math.max(this.clientWidth - 168, 100) / Math.max(this.layout?.duration ?? 1, 1)) *
      this.#zoom;
    this.requestUpdate();
  }

  render() {
    const layout = this.layout;
    if (!layout) return nothing;
    const s = this.#scale;
    const step =
      [0.1, 0.25, 0.5, 1, 2, 5, 10, 30, 60, 120, 300, 600, 1800, 3600].find((v) => v * s >= 65) ??
      3600;
    const ticks = [];
    for (let i = 0; i <= Math.ceil(layout.duration / step); i++) {
      const t = i * step;
      const label =
        t < 60
          ? `${Number(t.toFixed(2))}s`
          : `${Math.floor(t / 60)}:${String(Math.round(t % 60)).padStart(2, "0")}`;
      ticks.push(html`<span class="tick" style="left:${t * s}px">${label}</span>`);
    }
    return html`<div class="scroller">
      <div class="sheet" style="width:${144 + layout.duration * s + 24}px;--tick:${step * s}px">
        <div class="row ruler-row">
          <div class="label"><span>Tracks</span></div>
          <div class="ruler" @pointerdown=${this.#seek}>
            ${ticks}
            <div class="playhead" style="left:${this.time * s}px"></div>
          </div>
        </div>
        ${layout.tracks.map(
          (track, i) =>
            html`<div class="row">
              <div class="label" title=${track.id}>
                <small
                  >${track.kind === "audio" ? "A" : track.kind === "caption" ? "C" : "V"}${i + 1}</small
                ><span>${track.id}</span>
              </div>
              <div class="track">
                ${track.clips.map((clip) => this.#clip(track.kind, clip))}
                <div class="playhead" style="left:${this.time * s}px"></div>
              </div>
            </div>`,
        )}
      </div>
    </div>`;
  }

  #clip(kind: string, clip: Clip) {
    const drag = this.#drag?.clip === clip ? this.#drag : undefined;
    let [start, duration] = [clip.start, clip.duration];
    if (drag?.part === "body") start += drag.by;
    if (drag?.part === "head") [start, duration] = [start + drag.by, duration - drag.by];
    if (drag?.part === "tail") duration += drag.by;
    const classes = [
      "clip",
      kind,
      clip.enabled ? "" : "off",
      this.selected === clip.at ? "selected" : "",
    ].join(" ");
    return html`<div
      class=${classes}
      data-at=${clip.at}
      tabindex="0"
      role="button"
      aria-label=${clip.name ?? clip.id}
      @keydown=${(e: KeyboardEvent) => {
        if (e.key === "Enter") this.#emit("select", clip.at);
      }}
      title=${`${clip.id} · ${clip.kind} · ${start.toFixed(2)}–${(start + duration).toFixed(2)} s`}
      style="left:${start * this.#scale}px;width:${Math.max(duration * this.#scale, 2)}px"
      @pointerdown=${(e: PointerEvent) => this.#down(e, clip)}
      @pointermove=${(e: PointerEvent) => this.#move(e)}
      @pointerup=${(e: PointerEvent) => this.#up(e)}
      @pointercancel=${() => {
        this.#drag = undefined;
        this.requestUpdate();
      }}
    >
      ${clip.name ?? (clip.named ? clip.id : clip.kind)}
    </div>`;
  }

  #seek(event: PointerEvent) {
    const box = (event.currentTarget as HTMLElement).getBoundingClientRect();
    this.#emit("seek", Math.max(0, (event.clientX - box.left) / this.#scale));
  }

  #down(event: PointerEvent, clip: Clip) {
    if (event.button !== 0) return;
    event.preventDefault();
    const element = event.currentTarget as HTMLElement;
    element.setPointerCapture(event.pointerId);
    const box = element.getBoundingClientRect();
    const part =
      event.clientX - box.left < EDGE ? "head" : box.right - event.clientX < EDGE ? "tail" : "body";
    this.#drag = { clip, part, x: event.clientX, by: 0 };
    this.#emit("select", clip.at);
  }

  #move(event: PointerEvent) {
    const drag = this.#drag;
    if (!drag) {
      return;
    }
    const fps = this.layout!.fps;
    drag.by = Math.round(((event.clientX - drag.x) / this.#scale) * fps) / fps;
    this.requestUpdate();
  }

  #up(_event: PointerEvent) {
    const drag = this.#drag;
    this.#drag = undefined;
    this.requestUpdate();
    if (!drag || drag.by === 0) {
      return;
    }
    const { clip, part, by } = drag;
    if (part === "head") {
      this.#emit("op", { op: "trim", clip: clip.at, head: by });
    } else if (part === "tail") {
      this.#emit("op", { op: "trim", clip: clip.at, duration: clip.duration + by });
    } else if (clip.at.includes(".items[")) {
      this.#emit("op", { op: "move", clip: clip.at, index: this.#place(clip, clip.start + by) });
    } else {
      this.#emit("op", { op: "move", clip: clip.at, start: clip.start + by });
    }
  }

  /** The place in its sequence a clip dropped at `start` goes to: before the first clip of the
   * sequence that starts after it, as its index among the sequence's items. */
  #place(clip: Clip, start: number): number {
    const list = clip.at.slice(0, clip.at.lastIndexOf("["));
    const track = this.layout!.tracks.find((t) => t.clips.includes(clip))!;
    const others = track.clips.filter((c) => c !== clip && c.at.startsWith(`${list}[`));
    const index = (c: Clip) => Number(c.at.slice(c.at.lastIndexOf("[") + 1, -1));
    const own = index(clip);
    const after = others.find((c) => c.start + c.duration / 2 > start);
    if (!after) {
      return Math.max(...others.map(index), own);
    }
    // Taken out first, the clip moves those after it back by one.
    return index(after) > own ? index(after) - 1 : index(after);
  }

  #emit(type: string, detail: unknown) {
    this.dispatchEvent(new CustomEvent(type, { detail }));
  }
}

customElements.define("timeline-view", TimelineView);
