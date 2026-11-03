// The audio worklet that plays a timeline's sound: the engine's worker renders it, a chunk at a
// time, as this asks over its port; it says when the first sample of each start plays, which the
// player's clock counts from.

/** What an audio worklet's scope has, which TypeScript's libraries do not declare. */
declare const currentFrame: number;
declare const sampleRate: number;
declare class AudioWorkletProcessor {
  readonly port: MessagePort;
}
declare function registerProcessor(name: string, processor: typeof AudioWorkletProcessor): void;

/** Samples kept ahead of what plays: a tenth of a second at least. */
const AHEAD = 0.25;

class Player extends AudioWorkletProcessor {
  /** The worker's port. */
  #source: MessagePort | undefined;
  /** The start playing, and its chunks to play: interleaved stereo. */
  #gen = -1;
  #chunks: Float32Array[] = [];
  #offset = 0;
  #queued = 0;
  #asked = false;
  #started = false;
  /** Samples played as silence since it started, for want of chunks: those that come later are
   * dropped, so the sound keeps to the clock. */
  #late = 0;

  constructor() {
    super();
    this.port.onmessage = (event: MessageEvent) => {
      const message = event.data;
      if (message.port) {
        this.#source = message.port as MessagePort;
        this.#source.onmessage = (e: MessageEvent<{ gen: number; data: Float32Array }>) =>
          this.#chunk(e.data.gen, e.data.data);
      } else if (message.start !== undefined) {
        this.#reset(message.start);
        this.#asked = true;
        this.#source?.postMessage({ gen: this.#gen, sample: message.sample });
      } else if (message.stop) {
        this.#reset(-1);
      }
    };
  }

  #reset(gen: number) {
    this.#gen = gen;
    this.#chunks = [];
    this.#offset = 0;
    this.#queued = 0;
    this.#asked = false;
    this.#started = false;
    this.#late = 0;
  }

  #chunk(gen: number, data: Float32Array) {
    if (gen !== this.#gen) {
      return;
    }
    this.#asked = false;
    const dropped = Math.min(this.#late, data.length / 2);
    this.#late -= dropped;
    if (dropped < data.length / 2) {
      this.#chunks.push(data.subarray(2 * dropped));
      this.#queued += data.length / 2 - dropped;
    }
  }

  process(_: Float32Array[][], outputs: Float32Array[][]): boolean {
    const [left, right] = outputs[0];
    let i = 0;
    while (i < left.length && this.#chunks.length > 0) {
      if (!this.#started) {
        this.#started = true;
        this.port.postMessage({ started: this.#gen, frame: currentFrame + i });
      }
      const chunk = this.#chunks[0];
      for (; i < left.length && this.#offset < chunk.length; i++, this.#offset += 2) {
        left[i] = chunk[this.#offset];
        right[i] = chunk[this.#offset + 1];
      }
      if (this.#offset >= chunk.length) {
        this.#chunks.shift();
        this.#offset = 0;
      }
    }
    this.#queued -= i;
    if (this.#started) {
      this.#late += left.length - i;
    }
    left.fill(0, i);
    right.fill(0, i);
    if (this.#gen >= 0 && !this.#asked && this.#queued < AHEAD * sampleRate) {
      this.#asked = true;
      this.#source?.postMessage({ gen: this.#gen });
    }
    return true;
  }
}

registerProcessor("valle-sound", Player as unknown as typeof AudioWorkletProcessor);
