// The timeline's tracks and clips, as the service lays them out. A clip is selected by a click,
// moved by dragging it, along its track or onto another of its kind, and trimmed by dragging an
// edge; the ruler is dragged to scrub. Each change is asked for as an `op` event, the operation
// the service does, and the view is drawn again from the layout that follows.

import { css, html, LitElement, nothing } from "lit";

import { icon, type IconName } from "./icons.ts";
import { frame, peaks, wave } from "./media.ts";
import { type Clip, type Layout, mediaUrl, type Track } from "./service.ts";

/** Pixels an edge is grabbed within. */
const EDGE = 6;
/** Pixels an edge snaps within. */
const SNAP = 8;
/** Pixels the tracks' labels are wide. */
const LABELS = 144;
/** Pixels a clip's frame is wide. */
const TILE = 46;
/** The most the timeline zooms in, over fitting its length. */
export const ZOOM = 50;

const LETTERS = { visual: "V", audio: "A", caption: "C", adjustment: "FX" } as const;

const ICONS: Record<string, IconName> = {
  video: "film",
  image: "image",
  motion: "spark",
  solid: "square",
  timeline: "layers",
  audio: "music",
  caption: "text",
  grade: "sliders",
};

type Drag = {
  clip: Clip;
  track: Track;
  /** The body moved, or an edge trimmed. */
  part: "body" | "head" | "tail";
  x: number;
  y: number;
  /** The tracks' rows on the page, to find the one under the pointer. */
  rows: { at: string; top: number; bottom: number }[];
  moved: boolean;
  /** Seconds it has moved by. */
  by: number;
  /** The track it is over, and how far down the page that is from its own. */
  to: Track;
  down: number;
  /** Why it cannot be dropped here. */
  bad: string | undefined;
  /** The time it snapped to. */
  guide: number | undefined;
};

/** What the view draws. */
export type Shown = {
  layout: Layout | undefined;
  selected: string | undefined;
  time: number;
  zoom: number;
  /** The timeline's file, which its resources' paths are relative to. */
  entry: string;
  /** Project and timeline identity, for local editing preferences. */
  scope: string;
  playing: boolean;
  snap: boolean;
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
      scrollbar-width: thin;
      scrollbar-color: #343843 transparent;
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
      z-index: 5;
    }
    .ruler {
      position: relative;
      height: 26px;
      background-color: #16181c;
      background-image: linear-gradient(to right, #3a3f4b 1px, transparent 1px);
      background-size: calc(var(--tick) / 5) 5px;
      background-position: 0 100%;
      background-repeat: repeat-x;
      border-bottom: 1px solid #22252c;
      cursor: ew-resize;
      overflow: hidden;
      touch-action: none;
    }
    .tick {
      position: absolute;
      top: 0;
      bottom: 0;
      color: #8d94a3;
      font-size: 9.5px;
      font-variant-numeric: tabular-nums;
      padding-left: 5px;
      border-left: 1px solid #4a505d;
      line-height: 17px;
      pointer-events: none;
    }
    .label {
      position: sticky;
      left: 0;
      z-index: 4;
      display: flex;
      align-items: center;
      gap: 6px;
      padding: 0 6px 0 10px;
      border-right: 1px solid #22252c;
      border-bottom: 1px solid #1c1e24;
      background: #16181c;
      overflow: hidden;
    }
    .label .name {
      flex: 1;
      min-width: 0;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
      color: #b9bfcb;
    }
    .label small {
      flex-shrink: 0;
      min-width: 22px;
      text-align: center;
      font-size: 9px;
      font-weight: 600;
      border-radius: 3px;
      padding: 0 3px;
      color: var(--ink);
      background: color-mix(in srgb, var(--ink) 16%, transparent);
    }
    .visual {
      --ink: #8fb4ee;
    }
    .audio {
      --ink: #7fcdb1;
    }
    .caption {
      --ink: #dcb173;
    }
    .adjustment {
      --ink: #e394b2;
    }
    .label button {
      display: grid;
      place-items: center;
      width: 20px;
      height: 20px;
      padding: 0;
      border: 0;
      border-radius: 4px;
      background: none;
      color: #6f7684;
      cursor: pointer;
      flex-shrink: 0;
    }
    .label button:hover {
      background: #292c34;
      color: #eef0f4;
    }
    .label button[aria-pressed="true"] {
      color: #f2bb68;
    }
    .label button:focus-visible {
      outline: 2px solid #b7d2ff;
    }
    .track {
      position: relative;
      height: 32px;
      border-bottom: 1px solid #1c1e24;
      background-color: #0e0f12;
      background-image: linear-gradient(to right, #ffffff06 1px, transparent 1px);
      background-size: var(--tick) 100%;
    }
    .row.off .clip,
    .row.off .transition {
      opacity: 0.35;
    }
    .row.locked .track {
      background-image: repeating-linear-gradient(-45deg, #ffffff07 0 6px, transparent 6px 12px);
      background-size: auto;
    }
    .row.locked .clip {
      pointer-events: none;
    }
    .row.target .track {
      background-color: #1c2230;
    }
    .clip {
      --fill: #2a4d85;
      --edge: #5f8fd6;
      position: absolute;
      top: 3px;
      bottom: 3px;
      border-radius: 5px;
      background: linear-gradient(#ffffff1c, #ffffff00 55%), var(--fill);
      border: 1px solid color-mix(in srgb, var(--edge) 70%, transparent);
      box-shadow: inset 0 1px 0 #ffffff1f;
      overflow: hidden;
      cursor: grab;
      touch-action: none;
      color: #f1f4fa;
    }
    .clip.image {
      --fill: #473d78;
      --edge: #8672c9;
    }
    .clip.motion {
      --fill: #652f6a;
      --edge: #b565ba;
    }
    .clip.solid {
      --fill: #3a3f4a;
      --edge: #6f7888;
    }
    .clip.timeline {
      --fill: #1f5663;
      --edge: #4a9bab;
    }
    .clip.audio {
      --fill: #1f5646;
      --edge: #4f9d83;
    }
    .clip.caption {
      --fill: #6b512c;
      --edge: #ad854a;
    }
    .clip.grade {
      --fill: #6a2e44;
      --edge: #bd6585;
    }
    .strip {
      position: absolute;
      inset: 0;
      width: 100%;
      height: 100%;
      pointer-events: none;
    }
    .strip img {
      position: absolute;
      top: 0;
      height: 100%;
      object-fit: cover;
      opacity: 0.5;
    }
    .wave {
      fill: color-mix(in srgb, var(--edge) 75%, transparent);
    }
    .title {
      position: relative;
      display: inline-flex;
      align-items: center;
      gap: 5px;
      max-width: 100%;
      height: 16px;
      margin: 4px 3px;
      padding: 0 5px;
      border-radius: 3px;
      background: color-mix(in srgb, var(--fill) 80%, transparent);
      font-size: 10px;
      line-height: 16px;
      white-space: nowrap;
      pointer-events: none;
    }
    .title span {
      overflow: hidden;
      text-overflow: ellipsis;
    }
    .title svg {
      flex-shrink: 0;
      opacity: 0.8;
    }
    .clip.off {
      opacity: 0.4;
    }
    .clip.selected {
      border-color: #fff;
      box-shadow:
        inset 0 0 0 1px #fff,
        0 0 0 3px #ffffff24;
      filter: brightness(1.15);
    }
    .clip.dragging {
      z-index: 3;
      cursor: grabbing;
      box-shadow: 0 4px 14px #000a;
    }
    .clip.bad {
      border-color: #f07a7a;
      box-shadow: inset 0 0 0 1px #f07a7a;
      background: #5b2a30;
    }
    .clip:focus-visible {
      outline: 2px solid #b7d2ff;
      outline-offset: 1px;
    }
    .transition {
      position: absolute;
      z-index: 1;
      top: 3px;
      bottom: 3px;
      min-width: 3px;
      border: 1px solid #ffffffb0;
      border-radius: 3px;
      background: linear-gradient(to top right, #ffffff59 50%, transparent 50%), #00000040;
      pointer-events: none;
    }
    .marker {
      position: absolute;
      top: 0;
      bottom: 0;
      border-left: 1px solid var(--color);
      pointer-events: none;
    }
    .ruler .marker .flag {
      position: absolute;
      left: -1px;
      top: 0;
      max-width: 90px;
      height: 11px;
      padding: 0 4px;
      border-radius: 0 3px 3px 0;
      background: var(--color);
      color: #14161a;
      font-size: 8.5px;
      font-weight: 600;
      line-height: 11px;
      white-space: nowrap;
      overflow: hidden;
      text-overflow: ellipsis;
    }
    .marker .span {
      position: absolute;
      inset: 0 auto 0 0;
      background: color-mix(in srgb, var(--color) 14%, transparent);
    }
    .track .marker {
      opacity: 0.35;
    }
    .track .marker .flag {
      display: none;
    }
    .guide {
      position: absolute;
      z-index: 2;
      top: 0;
      bottom: 0;
      width: 1px;
      background: #ffd166;
      pointer-events: none;
    }
    .playhead {
      position: absolute;
      z-index: 2;
      top: 0;
      bottom: 0;
      width: 1px;
      background: #ff5d5d;
      pointer-events: none;
    }
    .ruler .playhead::before {
      content: "";
      position: absolute;
      top: 0;
      left: -5px;
      width: 11px;
      height: 12px;
      background: #ff5d5d;
      clip-path: polygon(0 0, 100% 0, 100% 55%, 50% 100%, 0 55%);
    }
  `;

  #shown: Shown = {
    layout: undefined,
    selected: undefined,
    time: 0,
    zoom: 1,
    entry: "",
    scope: "",
    playing: false,
    snap: true,
  };
  /** Pixels a second. */
  #scale = 80;
  #drag: Drag | undefined;
  #scrubbing = false;
  /** The ids of the tracks locked, which are not edited here. */
  #locked = new Set<string>();
  /** A time kept at a place across the track area, in pixels, through a change of zoom. */
  #anchor: { time: number; x: number } | undefined;
  #waves = new WeakMap<Float32Array, Map<string, { d: string; columns: number }>>();
  #observer = new ResizeObserver(() => this.#measure());
  #redraw = () => this.requestUpdate();

  get layout(): Layout | undefined {
    return this.#shown.layout;
  }

  locked(id: string): boolean {
    return this.#locked.has(id);
  }

  connectedCallback(): void {
    super.connectedCallback();
    this.#observer.observe(this);
    this.addEventListener("wheel", this.#wheel, { passive: false });
  }

  disconnectedCallback(): void {
    this.#observer.disconnect();
    this.removeEventListener("wheel", this.#wheel);
    super.disconnectedCallback();
  }

  set(shown: Shown): void {
    if (shown.scope !== this.#shown.scope) {
      this.#locked = new Set(recall(shown.scope));
    }
    const scroller = this.renderRoot.querySelector(".scroller");
    if (shown.zoom !== this.#shown.zoom && !this.#anchor && scroller) {
      // Around the playhead where it is in view, else the middle.
      const width = scroller.clientWidth - LABELS;
      const x = this.#shown.time * this.#scale - scroller.scrollLeft;
      this.#anchor =
        x >= 0 && x <= width
          ? { time: this.#shown.time, x }
          : { time: (scroller.scrollLeft + width / 2) / this.#scale, x: width / 2 };
    }
    this.#shown = shown;
    this.#measure();
  }

  #measure(): void {
    this.#scale =
      (Math.max(this.clientWidth - LABELS - 24, 100) / Math.max(this.layout?.duration ?? 1, 1)) *
      this.#shown.zoom;
    this.requestUpdate();
  }

  updated(): void {
    const scroller = this.renderRoot.querySelector(".scroller");
    if (!scroller) return;
    const anchor = this.#anchor;
    this.#anchor = undefined;
    const x = this.#shown.time * this.#scale - scroller.scrollLeft;
    const width = scroller.clientWidth - LABELS;
    if (anchor) {
      scroller.scrollLeft = Math.max(0, anchor.time * this.#scale - anchor.x);
    } else if (this.#shown.playing && !this.#drag && (x < 0 || x > width)) {
      // Playback is followed a page at a time.
      scroller.scrollLeft = Math.max(0, this.#shown.time * this.#scale - width * 0.1);
    }
  }

  /** Each track's label by its place: V1, V2, A1, by its own kind. */
  #labels(): Map<string, string> {
    const counts: Record<string, number> = {};
    return new Map(
      (this.layout?.tracks ?? []).map((t) => {
        counts[t.kind] = (counts[t.kind] ?? 0) + 1;
        return [t.at, `${LETTERS[t.kind] ?? "T"}${counts[t.kind]}`];
      }),
    );
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
    const labels = this.#labels();
    const markers = (layout.markers ?? []).map((m) => {
      const color = m.color && CSS.supports("color", m.color) ? m.color : "#f2bb68";
      return html`<div
        class="marker"
        data-marker=${m.at}
        style="left:${m.at * s}px;--color:${color}"
      >
        ${m.duration ? html`<span class="span" style="width:${m.duration * s}px"></span>` : nothing}
        <span class="flag">${m.name ?? ""}</span>
      </div>`;
    });
    const guide = this.#drag?.guide;
    const lines = html`${markers}${
        guide === undefined ? nothing : html`<div class="guide" style="left:${guide * s}px"></div>`
      }
      <div class="playhead" style="left:${this.#shown.time * s}px"></div>`;
    const deselect = (e: PointerEvent) => {
      if (e.target === e.currentTarget) this.#emit("select", undefined);
    };
    return html`<div class="scroller" @pointerdown=${deselect}>
      <div
        class="sheet"
        style="width:${LABELS + layout.duration * s + 24}px;--tick:${step * s}px"
        @pointerdown=${deselect}
      >
        <div class="row ruler-row">
          <div class="label"><span class="name">${layout.tracks.length} tracks</span></div>
          <div
            class="ruler"
            @pointerdown=${(e: PointerEvent) => this.#scrub(e, "start")}
            @pointermove=${(e: PointerEvent) => this.#scrub(e, "move")}
            @pointerup=${(e: PointerEvent) => this.#scrub(e, "end")}
            @pointercancel=${(e: PointerEvent) => this.#scrub(e, "end")}
          >
            ${ticks}${lines}
          </div>
        </div>
        ${layout.tracks.map((track) => {
          const label = labels.get(track.at)!;
          const locked = this.#locked.has(track.id);
          const audio = track.kind === "audio";
          const off = audio ? "Mute" : "Hide";
          const target = this.#drag?.moved && this.#drag.to === track && this.#drag.down !== 0;
          return html`<div
            class="row ${track.enabled ? "" : "off"} ${locked ? "locked" : ""} ${target ? "target" : ""}"
            data-track=${track.at}
          >
            <div class="label" title=${track.id}>
              <small class=${track.kind}>${label}</small><span class="name">${track.id}</span>
              <button
                title=${`${off} track`}
                aria-label=${`${off} ${label}`}
                aria-pressed=${!track.enabled}
                @click=${() =>
                  this.#emit("op", {
                    op: "set",
                    track: track.at,
                    field: "enabled",
                    value: track.enabled ? false : null,
                  })}
              >
                ${icon(audio ? (track.enabled ? "volume" : "muted") : track.enabled ? "eye" : "eyeOff", 13)}
              </button>
              <button
                title="Lock track"
                aria-label=${`Lock ${label}`}
                aria-pressed=${locked}
                @click=${() => this.#lock(track.id)}
              >
                ${icon(locked ? "lock" : "unlock", 13)}
              </button>
            </div>
            <div class="track" @pointerdown=${deselect}>
              ${track.clips.map((clip) => this.#clip(track, clip))}
              ${track.transitions.map(
                (t) =>
                  html`<div
                    class="transition"
                    data-at=${t.at}
                    style="left:${t.start * s}px;width:${(t.end - t.start) * s}px"
                  ></div>`,
              )}
              ${lines}
            </div>
          </div>`;
        })}
      </div>
    </div>`;
  }

  #clip(track: Track, clip: Clip) {
    const drag = this.#drag?.clip === clip && this.#drag.moved ? this.#drag : undefined;
    let [start, duration] = [clip.start, clip.duration];
    if (drag?.part === "body") start += drag.by;
    if (drag?.part === "head") [start, duration] = [start + drag.by, duration - drag.by];
    if (drag?.part === "tail") duration += drag.by;
    const width = Math.max(duration * this.#scale, 2);
    const classes = [
      "clip",
      clip.kind,
      clip.enabled ? "" : "off",
      this.#shown.selected === clip.at ? "selected" : "",
      drag ? "dragging" : "",
      drag?.bad ? "bad" : "",
    ].join(" ");
    const shift = drag?.down ? `;transform:translateY(${drag.down}px)` : "";
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
      style="left:${start * this.#scale}px;width:${width}px${shift}"
      @pointerdown=${(e: PointerEvent) => this.#down(e, track, clip)}
      @pointermove=${(e: PointerEvent) => this.#move(e)}
      @pointerup=${() => this.#up()}
      @pointercancel=${() => this.#cancel()}
    >
      ${this.#strip(clip, width)}
      <span class="title"
        >${icon(ICONS[clip.kind] ?? "file", 11)}<span
          >${title(clip, clip.named ? clip.id : clip.kind)}</span
        ></span
      >
    </div>`;
  }

  /** What a clip plays, behind its name: a video's frames, an image, a sound's shape. */
  #strip(clip: Clip, width: number) {
    const media = clip.media;
    if (!media?.file || width < 24) return nothing;
    const url = mediaUrl(this.#shown.entry, media.file);
    if (clip.kind === "audio") {
      const sound = peaks(url, this.#redraw);
      if (!sound) return nothing;
      const end = media.trim + clip.duration * media.rate;
      const key = `${media.trim} ${end}`;
      let shapes = this.#waves.get(sound);
      if (!shapes) this.#waves.set(sound, (shapes = new Map()));
      let shape = shapes.get(key);
      if (!shape) shapes.set(key, (shape = wave(sound, media.trim, end)));
      return html`<svg
        class="strip wave"
        viewBox="0 0 ${shape.columns} 2"
        preserveAspectRatio="none"
      >
        <path d=${shape.d}></path>
      </svg>`;
    }
    const count = Math.min(24, Math.ceil(width / TILE));
    const each = width / count;
    const tiles = [];
    for (let i = 0; i < count; i++) {
      const src =
        clip.kind === "image"
          ? url
          : frame(url, media.trim + ((i * each) / this.#scale) * media.rate, this.#redraw);
      if (src) {
        tiles.push(
          html`<img
            src=${src}
            alt=""
            draggable="false"
            style="left:${i * each}px;width:${each}px"
          />`,
        );
      }
    }
    return html`<span class="strip">${tiles}</span>`;
  }

  #lock(id: string): void {
    if (!this.#locked.delete(id)) this.#locked.add(id);
    try {
      localStorage.setItem(
        `valle.studio.locked:${this.#shown.scope}`,
        JSON.stringify([...this.#locked]),
      );
    } catch {
      /* Preferences are optional. */
    }
    this.requestUpdate();
  }

  #wheel = (event: WheelEvent) => {
    const scroller = this.renderRoot.querySelector(".scroller");
    if (!scroller || !(event.ctrlKey || event.metaKey)) return;
    event.preventDefault();
    const x = Math.max(0, event.clientX - scroller.getBoundingClientRect().left - LABELS);
    const zoom = Math.min(ZOOM, Math.max(1, this.#shown.zoom * Math.exp(-event.deltaY * 0.01)));
    if (zoom === this.#shown.zoom) return;
    // The time under the pointer stays under it.
    this.#anchor = { time: (x + scroller.scrollLeft) / this.#scale, x };
    this.#emit("zoom", zoom);
  };

  /** The ruler pressed, dragged and let go: a time to show, and which of the three it is. */
  #scrub(event: PointerEvent, phase: "start" | "move" | "end") {
    const ruler = event.currentTarget as HTMLElement;
    if (phase === "start") {
      if (event.button !== 0) return;
      event.preventDefault();
      ruler.setPointerCapture(event.pointerId);
      this.#scrubbing = true;
    } else if (!this.#scrubbing) {
      return;
    }
    if (phase === "end") this.#scrubbing = false;
    const duration = this.layout?.duration ?? 0;
    let time = (event.clientX - ruler.getBoundingClientRect().left) / this.#scale;
    if (this.#shown.snap && !event.altKey) time += this.#snap([time], undefined, false)?.by ?? 0;
    this.#emit("seek", { time: Math.min(Math.max(0, time), duration), phase });
  }

  /** The nearest of the times edges snap to, within reach of one of `times`: the playhead, the
   * start, the markers, and the other clips' starts and ends. */
  #snap(times: number[], skip: Clip | undefined, playhead = true) {
    const layout = this.layout!;
    const to = [0, ...(layout.markers ?? []).map((m) => m.at)];
    if (playhead) to.push(this.#shown.time);
    for (const clip of layout.tracks.flatMap((t) => t.clips)) {
      if (clip !== skip) to.push(clip.start, clip.start + clip.duration);
    }
    let best: { by: number; at: number } | undefined;
    for (const time of times) {
      for (const at of to) {
        const by = at - time;
        if (Math.abs(by) * this.#scale < SNAP && (!best || Math.abs(by) < Math.abs(best.by))) {
          best = { by, at };
        }
      }
    }
    return best;
  }

  #down(event: PointerEvent, track: Track, clip: Clip) {
    if (event.button !== 0) return;
    event.preventDefault();
    const element = event.currentTarget as HTMLElement;
    element.setPointerCapture(event.pointerId);
    const box = element.getBoundingClientRect();
    const part =
      event.clientX - box.left < EDGE ? "head" : box.right - event.clientX < EDGE ? "tail" : "body";
    const rows = [...this.renderRoot.querySelectorAll<HTMLElement>(".row[data-track]")].map(
      (row) => {
        const { top, bottom } = row.getBoundingClientRect();
        return { at: row.dataset.track!, top, bottom };
      },
    );
    this.#drag = {
      clip,
      track,
      part,
      x: event.clientX,
      y: event.clientY,
      rows,
      moved: false,
      by: 0,
      to: track,
      down: 0,
      bad: undefined,
      guide: undefined,
    };
    addEventListener("keydown", this.#escape);
    this.#emit("select", clip.at);
  }

  #move(event: PointerEvent) {
    const drag = this.#drag;
    if (!drag) {
      const box = (event.currentTarget as HTMLElement).getBoundingClientRect();
      const edge = event.clientX - box.left < EDGE || box.right - event.clientX < EDGE;
      (event.currentTarget as HTMLElement).style.cursor = edge ? "ew-resize" : "";
      return;
    }
    if (!drag.moved && Math.hypot(event.clientX - drag.x, event.clientY - drag.y) < 3) return;
    drag.moved = true;
    const { clip, track } = drag;
    const fps = this.layout!.fps;
    const frames = (seconds: number) => Math.round(seconds * fps) / fps;
    const end = clip.start + clip.duration;
    let by = (event.clientX - drag.x) / this.#scale;
    drag.guide = drag.bad = undefined;
    drag.to = track;
    drag.down = 0;
    const snap = (times: number[]) => {
      const hit = this.#shown.snap && !event.altKey ? this.#snap(times, clip) : undefined;
      if (hit) {
        by += hit.by;
        drag.guide = hit.at;
      }
    };
    const clash = (on: Track, from: number, to: number) => {
      const other = on.clips.find(
        (c) => c !== clip && from < c.start + c.duration - 1e-9 && c.start < to - 1e-9,
      );
      if (other) {
        drag.bad = `“${title(clip)}” would overlap “${title(other)}” on ${this.#labels().get(on.at)}. Drop it on another track.`;
      }
    };
    if (drag.part === "body") {
      const own = drag.rows.find((r) => r.at === track.at);
      const row = drag.rows.find((r) => event.clientY >= r.top && event.clientY < r.bottom);
      const over = this.layout!.tracks.find((t) => t.at === row?.at);
      if (own && row && over && over.kind === track.kind && !this.#locked.has(over.id)) {
        drag.to = over;
        drag.down = row.top - own.top;
      }
      if (drag.to.sequence) {
        by = frames(by);
      } else {
        snap([clip.start + by, end + by]);
        by = Math.max(frames(by), -clip.start);
        clash(drag.to, clip.start + by, end + by);
      }
    } else if (drag.part === "head") {
      if (!track.sequence) snap([clip.start + by]);
      by = Math.min(frames(by), frames(clip.duration - 1 / fps));
      if (!track.sequence) {
        by = Math.max(by, -clip.start);
        clash(track, clip.start + by, end);
      }
    } else {
      if (!track.sequence) snap([end + by]);
      by = Math.max(frames(by), frames(1 / fps - clip.duration));
      if (!track.sequence) clash(track, clip.start, end + by);
    }
    drag.by = by;
    this.requestUpdate();
  }

  #escape = (event: KeyboardEvent) => {
    if (event.key === "Escape") this.#cancel();
  };

  #cancel() {
    removeEventListener("keydown", this.#escape);
    this.#drag = undefined;
    this.requestUpdate();
  }

  #up() {
    const drag = this.#drag;
    this.#cancel();
    if (!drag?.moved) return;
    const { clip, part, by, to } = drag;
    if (drag.bad) {
      this.#emit("refuse", drag.bad);
    } else if (part === "head") {
      if (by !== 0) this.#emit("op", { op: "trim", clip: clip.at, head: by });
    } else if (part === "tail") {
      if (by !== 0) this.#emit("op", { op: "trim", clip: clip.at, duration: clip.duration + by });
    } else if (to !== drag.track) {
      const start = to.sequence ? {} : { start: clip.start + by };
      this.#emit("op", { op: "move", clip: clip.at, track: to.at, ...start });
    } else if (by === 0) {
      return;
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

/** What a clip is called: its name, a caption's text, else its id. */
export function title(clip: Clip, fallback = clip.id): string {
  return clip.name ?? clip.text ?? fallback;
}

/** The tracks of `entry` locked when it was last open here. */
function recall(entry: string): string[] {
  try {
    const kept = JSON.parse(localStorage.getItem(`valle.studio.locked:${entry}`) ?? "[]");
    return Array.isArray(kept) ? kept.filter((id) => typeof id === "string") : [];
  } catch {
    return [];
  }
}

customElements.define("timeline-view", TimelineView);
