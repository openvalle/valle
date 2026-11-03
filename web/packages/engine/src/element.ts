// `<valle-player>`: a player in a page's markup, registered by importing this module. It shows
// the canvas and nothing else; controls are the page's own, on `element.player`.
//
//   <script type="module">import "valle-engine/element";</script>
//   <valle-player src="/works/card/card.motion.tsx" runtime="/valle/" loop autoplay muted></valle-player>
//
// It says `ready` once `player` has what `src` names open, and `error` with the error in its
// `detail` where that fails.

import { createPlayer, type Player } from "./index.ts";

export class VallePlayerElement extends HTMLElement {
  static observedAttributes = ["src", "runtime", "loop", "autoplay", "muted"];

  /** The player of what `src` names, once it is open. */
  player: Player | undefined;
  /** Each opening, so one started before the attributes changed again is let go. */
  #opening = 0;

  constructor() {
    super();
    const shadow = this.attachShadow({ mode: "open" });
    shadow.innerHTML = `<style>
  :host { display: inline-block; }
  canvas { display: block; max-width: 100%; height: auto; }
</style>`;
  }

  connectedCallback(): void {
    void this.#open();
  }

  disconnectedCallback(): void {
    this.#opening++;
    void this.#close();
  }

  attributeChangedCallback(name: string, before: string | null, after: string | null): void {
    if (before === after || !this.isConnected) {
      return;
    }
    const player = this.player;
    if (name === "loop" && player) {
      player.loop = after !== null;
    } else if (name === "muted" && player) {
      player.muted = after !== null;
    } else if (name === "src" || name === "runtime") {
      void this.#open();
    }
  }

  async #open(): Promise<void> {
    const opening = ++this.#opening;
    await this.#close();
    const src = this.getAttribute("src");
    if (!src || opening !== this.#opening) {
      return;
    }
    // A canvas gives its control to one worker only, so each player has its own.
    const canvas = document.createElement("canvas");
    this.shadowRoot!.querySelector("canvas")?.remove();
    this.shadowRoot!.append(canvas);
    let player: Player;
    try {
      player = await createPlayer({
        canvas,
        entry: src,
        runtime: this.getAttribute("runtime") ?? undefined,
        loop: this.hasAttribute("loop"),
        muted: this.hasAttribute("muted"),
      });
    } catch (error) {
      if (opening === this.#opening) {
        this.dispatchEvent(new CustomEvent("error", { detail: error }));
      }
      return;
    }
    if (opening !== this.#opening) {
      await player.close();
      return;
    }
    this.player = player;
    this.dispatchEvent(new CustomEvent("ready"));
    if (this.hasAttribute("autoplay")) {
      player.play();
    }
  }

  async #close(): Promise<void> {
    const player = this.player;
    this.player = undefined;
    await player?.close();
  }
}

if (!customElements.get("valle-player")) {
  customElements.define("valle-player", VallePlayerElement);
}
