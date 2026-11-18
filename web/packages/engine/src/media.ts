// An input and its track indexes are shared by the cursors over the same file.
import {
  ALL_FORMATS,
  type AudioSample,
  AudioSampleSink,
  BlobSource,
  Input,
  UrlSource,
} from "mediabunny";
import { Remote, READ_WINDOW } from "./remote.ts";
import { Reader } from "./video.ts";

export const PCM_PAGE = 48_000;
export class Media {
  readonly input: Input;
  #remote?: Remote;
  #video: ReturnType<Input["getPrimaryVideoTrack"]> | undefined;
  #audio: Promise<AudioTrack | null> | undefined;
  #info: Promise<Awaited<ReturnType<Reader["probe"]>> | undefined> | undefined;

  constructor(source: string | Blob, fetchFn?: typeof fetch) {
    if (typeof source === "string") this.#remote = new Remote(fetchFn ?? fetch.bind(globalThis));
    this.input = new Input({
      formats: ALL_FORMATS,
      source:
        typeof source === "string"
          ? new UrlSource(source, {
              fetchFn: this.#remote!.fetch,
              maxCacheSize: READ_WINDOW,
              parallelism: 1,
              getRetryDelay: (attempts) => (attempts >= 2 ? null : 0.15 * 2 ** attempts),
              // The pending read sees the error. A speculative request may finish after a seek.
              handleUnhandledError: () => {},
            })
          : new BlobSource(source),
    });
  }

  cancelDownload(): void {
    this.#remote?.abort();
  }

  async dispose(): Promise<void> {
    this.input.dispose();
    await this.#remote?.dispose();
  }

  async reader(): Promise<Reader | undefined> {
    const track = await (this.#video ??= this.input.getPrimaryVideoTrack());
    return track ? Reader.open(track) : undefined;
  }

  async probe(): Promise<Awaited<ReturnType<Reader["probe"]>> | undefined> {
    return (this.#info ??= this.#probe());
  }

  async #probe(): Promise<Awaited<ReturnType<Reader["probe"]>> | undefined> {
    const reader = await this.reader();
    try {
      return await reader?.probe();
    } finally {
      reader?.dispose();
    }
  }

  audio(): Promise<AudioTrack | null> {
    return (this.#audio ??= this.input.getPrimaryAudioTrack().then(async (track) => {
      if (!track || !(await track.canDecode())) return null;
      const [rate, duration, first] = await Promise.all([
        track.getSampleRate(),
        track.computeDuration(),
        track.getFirstTimestamp(),
      ]);
      return new AudioTrack(new AudioSampleSink(track), rate, Math.round(duration * rate), first);
    }));
  }
}

class AudioTrack {
  constructor(
    private sink: AudioSampleSink,
    readonly rate: number,
    readonly frames: number,
    private first: number,
  ) {}

  async page(index: number): Promise<Float32Array> {
    const start = index * PCM_PAGE;
    const length = Math.min(PCM_PAGE, Math.max(0, this.frames - start));
    const out = new Float32Array(2 * length);
    if (!length) return out;
    const end = (start + length) / this.rate;
    // Request the start packet as well: a decoded buffer can start before the page.
    const from = Math.max(this.first, start / this.rate);
    if (from >= end) return out;
    for await (const sample of this.sink.samples(from, end)) {
      try {
        const at = Math.round(sample.timestamp * this.rate);
        const offset = Math.max(0, start - at);
        const count = Math.min(sample.numberOfFrames - offset, start + length - at - offset);
        if (count <= 0) continue;
        copyStereo(sample, offset, count, out, 2 * (at + offset - start));
      } finally {
        sample.close();
      }
    }
    return out;
  }
}

// Default speaker order by channel count, as in native media/sound.rs.
// L/R are front, C is centre, B is back centre, l/r are side or surround, and - omits LFE.
const layouts: Record<number, string> = {
  3: "LR-",
  4: "LRCB",
  5: "LRClr",
  6: "LRC-lr",
  7: "LRC-Blr",
  8: "LRC-lrlr",
  10: "LRC-lrlr--",
  12: "LRC-lrlrlr--",
  14: "LRC-lrLRlrlr--",
  16: "LRC-lrLRlrlr----",
};

function copyStereo(
  sample: AudioSample,
  offset: number,
  count: number,
  out: Float32Array,
  start: number,
) {
  const channels = sample.numberOfChannels;
  const options = { frameOffset: offset, frameCount: count };
  if (channels === 2) {
    sample.copyTo(out.subarray(start, start + count * 2), {
      ...options,
      format: "f32",
      planeIndex: 0,
    });
    return;
  }
  const plane = new Float32Array(count);
  const planar = { ...options, format: "f32-planar" as const };
  if (channels === 1) {
    sample.copyTo(plane, { ...planar, planeIndex: 0 });
    for (let i = 0; i < count; i++) out[start + i * 2] = out[start + i * 2 + 1] = plane[i]!;
    return;
  }
  const layout = layouts[channels];
  if (!layout) throw new Error(`no default stereo mix for ${channels} audio channels`);
  out.fill(0, start, start + count * 2);
  for (let c = 0; c < channels; c++) {
    const speaker = layout[c];
    if (speaker === "-") continue;
    sample.copyTo(plane, { ...planar, planeIndex: c });
    const gain = speaker === "L" || speaker === "R" ? 1 : speaker === "B" ? 0.5 : Math.SQRT1_2;
    const left = speaker !== "R" && speaker !== "r";
    const right = speaker !== "L" && speaker !== "l";
    for (let i = 0; i < count; i++) {
      const value = plane[i]! * gain;
      const j = start + i * 2;
      if (left) out[j] = out[j]! + value;
      if (right) out[j + 1] = out[j + 1]! + value;
    }
  }
}
