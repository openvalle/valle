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
      font:
        12px system-ui,
        sans-serif;
      color: #cbd5e1;
    }
    .ruler {
      position: relative;
      height: 18px;
      border-bottom: 1px solid #334155;
      cursor: pointer;
    }
    .tick {
      position: absolute;
      top: 2px;
      color: #64748b;
      font-size: 10px;
    }
    .track {
      position: relative;
      height: 34px;
      border-bottom: 1px solid #1e293b;
    }
    .label {
      position: absolute;
      left: 4px;
      top: 2px;
      font-size: 10px;
      color: #475569;
      pointer-events: none;
    }
    .clip {
      position: absolute;
      top: 4px;
      bottom: 4px;
      border-radius: 4px;
      background: #2563eb;
      border: 1px solid #1d4ed8;
      overflow: hidden;
      white-space: nowrap;
      text-overflow: ellipsis;
      padding: 0 6px;
      line-height: 24px;
      cursor: grab;
      box-sizing: border-box;
    }
    .clip.audio {
      background: #0d9488;
      border-color: #0f766e;
    }
    .clip.caption {
      background: #a16207;
      border-color: #854d0e;
    }
    .clip.off {
      opacity: 0.4;
    }
    .clip.selected {
      outline: 2px solid #f8fafc;
    }
    .playhead {
      position: absolute;
      top: 0;
      bottom: 0;
      width: 1px;
      background: #f43f5e;
      pointer-events: none;
    }
  `;

  layout: Layout | undefined;
  selected: string | undefined;
  time = 0;
  /** Pixels a second. */
  #scale = 80;
  #drag: Drag | undefined;

  set(layout: Layout | undefined, selected: string | undefined, time: number): void {
    this.layout = layout;
    this.selected = selected;
    this.time = time;
    if (layout) {
      const width = Math.max(this.clientWidth - 20, 200);
      this.#scale = width / Math.max(layout.duration, 1);
    }
    this.requestUpdate();
  }

  render() {
    const layout = this.layout;
    if (!layout) {
      return nothing;
    }
    const s = this.#scale;
    const step = s > 60 ? 1 : s > 15 ? 5 : 10;
    const ticks = [];
    for (let t = 0; t <= layout.duration; t += step) {
      ticks.push(html`<span class="tick" style="left:${t * s}px">${t}s</span>`);
    }
    return html`
      <div class="ruler" @pointerdown=${this.#seek}>
        ${ticks}
        <div class="playhead" style="left:${this.time * s}px"></div>
      </div>
      <div style="position:relative">
        ${layout.tracks.map(
          (track) => html`
            <div class="track">
              <span class="label">${track.id}</span>
              ${track.clips.map((clip) => this.#clip(track.kind, clip))}
            </div>
          `,
        )}
        <div class="playhead" style="left:${this.time * s}px"></div>
      </div>
    `;
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
      title=${`${clip.id} · ${clip.kind} · ${start.toFixed(2)}–${(start + duration).toFixed(2)} s`}
      style="left:${start * this.#scale}px;width:${Math.max(duration * this.#scale, 2)}px"
      @pointerdown=${(e: PointerEvent) => this.#down(e, clip)}
      @pointermove=${(e: PointerEvent) => this.#move(e)}
      @pointerup=${(e: PointerEvent) => this.#up(e)}
    >
      ${clip.name ?? (clip.named ? clip.id : clip.kind)}
    </div>`;
  }

  #seek(event: PointerEvent) {
    const box = (event.currentTarget as HTMLElement).getBoundingClientRect();
    this.#emit("seek", Math.max(0, (event.clientX - box.left) / this.#scale));
  }

  #down(event: PointerEvent, clip: Clip) {
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
