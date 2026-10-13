import { AudioSampleSink } from "mediabunny";
import { createMediaInput, type BrowserMediaInput } from "./input.ts";

/** A reader retains encoded input/demux metadata, never whole-source decoded PCM. Seeking and
 * decoding use the same Mediabunny/WebCodecs path as full-buffer audio. */
export class BrowserAudioWindowReader {
  private constructor(private readonly input: ReturnType<typeof createMediaInput>,
    private readonly sink: AudioSampleSink, readonly sampleRate: number, readonly channels: number,
    readonly duration: number) {}

  static async open(source: BrowserMediaInput): Promise<BrowserAudioWindowReader> {
    const input = createMediaInput(source);
    try {
      const track = await input.getPrimaryAudioTrack();
      if (!track) throw new Error("no audio track");
      return new BrowserAudioWindowReader(input, new AudioSampleSink(track), await track.getSampleRate(),
        await track.getNumberOfChannels(), await track.computeDuration());
    } catch (error) { input.dispose(); throw error; }
  }

  workingBytes(targetRate: number): number {
    // Raw decode with left/right resampler guards, resampled output, cached planes and RPC copy.
    return Math.ceil((this.sampleRate * 3 + targetRate * 6) * this.channels * 4);
  }

  async read(context: BaseAudioContext, start: number, end: number): Promise<AudioBuffer> {
    const rate = context.sampleRate;
    if (!Number.isSafeInteger(start) || !Number.isSafeInteger(end) || start < 0 || end <= start
      || end - start > rate || end > Math.round(this.duration * rate)) throw new RangeError("audio window outside source PCM");
    const resample = rate !== this.sampleRate;
    // Integer-second anchors preserve resampler phase; a full preceding second and a trailing
    // guard prevent an independent window's filter edge from entering requested samples.
    const first = resample ? Math.max(0, Math.floor(start / rate) - 1) * this.sampleRate : start;
    const last = Math.min(Math.round(this.duration * this.sampleRate),
      Math.ceil(end / rate * this.sampleRate) + (resample ? Math.ceil(this.sampleRate / 10) : 0));
    const decoded = context.createBuffer(this.channels, last - first, this.sampleRate);
    for await (const sample of this.sink.samples(first / this.sampleRate, last / this.sampleRate)) {
      try {
        if (sample.sampleRate !== this.sampleRate || sample.numberOfChannels !== this.channels) throw new Error("decoded audio format changed");
        const timestamp = Math.round(sample.timestamp * this.sampleRate);
        const sourceOffset = Math.max(0, first - timestamp);
        const destination = Math.max(0, timestamp - first);
        const count = Math.min(sample.numberOfFrames - sourceOffset, decoded.length - destination);
        if (count > 0) for (let channel = 0; channel < this.channels; channel++) {
          sample.copyTo(decoded.getChannelData(channel).subarray(destination, destination + count),
            { planeIndex: channel, format: "f32-planar", frameOffset: sourceOffset, frameCount: count });
        }
      } finally { sample.close(); }
    }
    if (!resample) return decoded;
    const offline = new OfflineAudioContext(this.channels, Math.max(1, Math.round(decoded.length * rate / this.sampleRate)), rate);
    const node = offline.createBufferSource(); node.buffer = decoded; node.connect(offline.destination); node.start();
    const converted = await offline.startRendering();
    const result = context.createBuffer(this.channels, end - start, rate);
    const offset = start - first / this.sampleRate * rate;
    for (let channel = 0; channel < this.channels; channel++) result.copyToChannel(converted.getChannelData(channel).subarray(offset, offset + result.length), channel);
    node.disconnect();
    return result;
  }

  close(): void { this.input.dispose(); }
}

/** Decode with the shared reader. WebAudio remains responsible only for PCM resampling/mixing. */
export async function decodeAudioBuffer(source: BrowserMediaInput, context: BaseAudioContext): Promise<AudioBuffer> {
  const input = createMediaInput(source);
  let buffer: AudioBuffer;
  try {
    const track = await input.getPrimaryAudioTrack();
    if (!track) throw new Error("no audio track");
    const sampleRate = await track.getSampleRate();
    const channels = await track.getNumberOfChannels();
    const length = Math.round(await track.computeDuration() * sampleRate);
    if (!Number.isSafeInteger(length) || length < 1) throw new Error("audio track has no playable samples");
    buffer = context.createBuffer(channels, length, sampleRate);
    for await (const sample of new AudioSampleSink(track).samples(0)) {
      try {
        if (sample.sampleRate !== sampleRate || sample.numberOfChannels !== channels) throw new Error("decoded audio format changed");
        const start = Math.round(sample.timestamp * sampleRate);
        const offset = Math.max(0, -start);
        const count = Math.min(sample.numberOfFrames - offset, length - Math.max(0, start));
        if (count > 0) for (let channel = 0; channel < channels; channel++) {
          sample.copyTo(buffer.getChannelData(channel).subarray(Math.max(0, start), Math.max(0, start) + count), {
            planeIndex: channel, format: "f32-planar", frameOffset: offset, frameCount: count,
          });
        }
      } finally { sample.close(); }
    }
  } finally { input.dispose(); }
  if (buffer.sampleRate === context.sampleRate) return buffer;
  const offline = new OfflineAudioContext(buffer.numberOfChannels,
    Math.max(1, Math.round(buffer.length * context.sampleRate / buffer.sampleRate)), context.sampleRate);
  const node = offline.createBufferSource();
  node.buffer = buffer; node.connect(offline.destination); node.start();
  return offline.startRendering();
}
