// The frames of video files, which the worker decodes with WebCodecs as the engine shows them:
// mediabunny reads the containers. A reader goes on from the frame it gave last, so playing
// decodes each frame once; a jump starts it again from the key frame before.

import {
  ALL_FORMATS,
  BlobSource,
  Input,
  type InputVideoTrack,
  UrlSource,
  type VideoSample,
  VideoSampleSink,
} from "mediabunny";

/** How far ahead of the frame it gave a reader decodes on to the one asked for, in seconds:
 * further, it starts again from the key frame before. */
const AHEAD = 2;

export class Reader {
  #input: Input;
  #track: InputVideoTrack;
  #sink: VideoSampleSink;
  /** The color space the file tags, which frames' do not tell apart from a guess. */
  #tags: VideoColorSpaceInit;
  #samples: AsyncGenerator<VideoSample, void> | undefined;
  /** The frame it gave last, and the one after it. */
  #current: VideoSample | undefined;
  #next: VideoSample | undefined;

  private constructor(input: Input, track: InputVideoTrack, tags: VideoColorSpaceInit) {
    this.#input = input;
    this.#track = track;
    this.#tags = tags;
    this.#sink = new VideoSampleSink(track);
  }

  /** A reader of the file at `url`, or of its bytes; none where it has no video. */
  static async open(file: string | Blob): Promise<Reader | undefined> {
    const source = typeof file === "string" ? new UrlSource(file) : new BlobSource(file);
    const input = new Input({ source, formats: ALL_FORMATS });
    const track = await input.getPrimaryVideoTrack();
    if (!track) {
      input.dispose();
      return undefined;
    }
    return new Reader(input, track, await track.getColorSpace());
  }

  /** Its frames' size and its length in seconds, as the engine takes them. */
  async probe(): Promise<{ width: number; height: number; duration: number }> {
    const first = await this.#sink.getSample(await this.#track.getFirstTimestamp());
    if (!first) {
      throw new Error("no frame in it");
    }
    const { width: w, height: h } = first.visibleRect;
    const [width, height] = first.rotation % 180 ? [h, w] : [w, h];
    first.close();
    return { width, height, duration: await this.#track.computeDuration() };
  }

  /** The frame shown at `seconds`, the last that starts at or before then, or the first, as
   * the engine takes it. */
  async frame(seconds: number): Promise<Uint8Array> {
    const current = this.#current;
    // Past the last frame, it stays on it.
    const far =
      current !== undefined && this.#next !== undefined && seconds > current.timestamp + AHEAD;
    if (!current || seconds < current.timestamp || far) {
      await this.#from(seconds);
    }
    while (this.#next && this.#next.timestamp <= seconds) {
      this.#current?.close();
      this.#current = this.#next;
      this.#next = (await this.#samples!.next()).value ?? undefined;
    }
    return planes(this.#current!, this.#tags);
  }

  async #from(seconds: number) {
    this.#close();
    this.#samples = this.#sink.samples(seconds);
    this.#current = (await this.#samples.next()).value ?? undefined;
    if (!this.#current) {
      // Past its last frame: that one.
      this.#samples = this.#sink.samples((await this.#track.computeDuration()) - 1e-6);
      this.#current = (await this.#samples.next()).value ?? undefined;
      if (!this.#current) {
        throw new Error("no frame in it");
      }
    }
    this.#next = (await this.#samples.next()).value ?? undefined;
  }

  #close() {
    this.#current?.close();
    this.#next?.close();
    this.#current = this.#next = undefined;
    void this.#samples?.return();
  }

  dispose() {
    this.#close();
    this.#input.dispose();
  }
}

/** The formats the engine converts itself, by its numbers for them. */
const FORMATS: Record<string, number> = { I420: 1, NV12: 2, I420A: 3 };

/** A frame as the engine takes it: little-endian u32s of its format (0 for straight RGBA), BT.709
 * or not, full range or not, its coded width and height, its count of planes,
 * clockwise quarter turns, horizontal reflection after rotation, and each plane's offset and
 * bytes a row, then the planes. The engine converts 8-bit 4:2:0 itself, as the native engine
 * does: by the matrix and range the file tags, BT.709 or BT.601 by its height when it tags none,
 * its values taken as sRGB's. The browser converts the others. */
async function planes(sample: VideoSample, tags: VideoColorSpaceInit): Promise<Uint8Array> {
  const frame = sample.toVideoFrame();
  try {
    const { width, height } = frame.visibleRect!;
    const kind = (frame.format && FORMATS[frame.format]) ?? 0;
    const options = kind ? {} : ({ format: "RGBA", colorSpace: "srgb" } as const);
    const size = frame.allocationSize(options);
    const count = kind === 0 ? 0 : kind === 2 ? 2 : kind === 3 ? 4 : 3;
    const header = 4 * (8 + 2 * count);
    const out = new Uint8Array(header + size);
    const layout = await frame.copyTo(out.subarray(header), options);
    const { matrix, fullRange } = tags;
    const bt601 = matrix === "bt470bg" || matrix === "smpte170m" || matrix === "smpte240m";
    const bt709 = matrix === "bt709" || (!bt601 && height >= 720);
    const words = new DataView(out.buffer);
    const fields = [
      kind,
      bt709 ? 1 : 0,
      fullRange ? 1 : 0,
      width,
      height,
      count,
      sample.rotation / 90,
      sample.flip ? 1 : 0,
    ];
    for (const plane of kind ? layout : []) {
      fields.push(plane.offset, plane.stride);
    }
    fields.forEach((v, i) => words.setUint32(4 * i, v, true));
    return out;
  } finally {
    frame.close();
  }
}
