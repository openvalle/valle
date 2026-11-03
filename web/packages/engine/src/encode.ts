// An export in the worker: every frame drawn off the canvas and read back as I420, as the native
// engine converts it, and the sound mixed from the start, encoded with WebCodecs into an MP4 that
// mediabunny writes in memory.

import {
  AudioSample,
  AudioSampleSource,
  BufferTarget,
  canEncodeAudio,
  Mp4OutputFormat,
  Output,
  VideoSample,
  VideoSampleSource,
} from "mediabunny";
import { type ExportOptions, exportFrames, type Opened } from "./messages.ts";
import { type Engine, i420, type Sound } from "./wasm/valle_web.js";

/** The tags of the frames it encodes: BT.709's primaries and matrix, limited range, and the
 * values as sRGB's, as the native engine tags its MP4s. */
const COLOR = {
  primaries: "bt709",
  transfer: "iec61966-2-1",
  matrix: "bt709",
  fullRange: false,
} as const;

/** Samples a second of the sound. */
const RATE = 48_000;

/** The MP4 of `composition` as `options` say: `draw` draws its frame of an index for `engine` to
 * read back, `sound` mixes from the start when it is to be heard, and `progress` hears of each
 * frame done. At another rate, each frame is the composition's frame at its time. */
export async function encode(
  options: ExportOptions,
  composition: Opened,
  draw: (index: number) => Promise<void>,
  engine: Engine,
  sound: Sound | undefined,
  progress: (done: number) => void,
): Promise<ArrayBuffer> {
  const { width, height, bitrate } = options;
  const [n, d] = options.rate;
  const indices = exportFrames(composition, options.rate);
  const frames = indices.length;
  const target = new BufferTarget();
  const output = new Output({ format: new Mp4OutputFormat({ fastStart: "in-memory" }), target });
  const video = new VideoSampleSource({ codec: "avc", bitrate });
  output.addVideoTrack(video, { frameRate: n / d });
  let audio: AudioSampleSource | undefined;
  if (sound) {
    const aac = await canEncodeAudio("aac", { numberOfChannels: 2, sampleRate: RATE });
    audio = new AudioSampleSource({ codec: aac ? "aac" : "opus", bitrate: 192_000 });
    output.addAudioTrack(audio);
  }
  await output.start();
  const fit = fitter(composition, width, height);
  const seconds = (frames * d) / n;
  const samples = Math.round(seconds * RATE);
  let mixed = 0;
  try {
    for (let i = 0; i < frames; i++) {
      await draw(indices[i]);
      const data = fit ? await fit(await engine.pixels()) : await engine.yuv();
      const frame = new VideoSample(data, {
        format: "I420",
        codedWidth: width,
        codedHeight: height,
        timestamp: (i * d) / n,
        duration: d / n,
        colorSpace: COLOR,
      });
      await video.add(frame);
      frame.close();
      // The sound a second ahead of the frames, so the two interleave.
      while (audio && sound && mixed < samples && mixed / RATE <= ((i + 1) * d) / n + 1) {
        const count = Math.min(RATE, samples - mixed);
        const chunk = new AudioSample({
          data: sound.render(count),
          format: "f32",
          numberOfChannels: 2,
          sampleRate: RATE,
          timestamp: mixed / RATE,
        });
        await audio.add(chunk);
        chunk.close();
        mixed += count;
      }
      progress(i + 1);
    }
    await output.finalize();
  } catch (error) {
    await output.cancel();
    throw error;
  }
  return target.buffer!;
}

/** What puts a frame of the composition's size into one of `width` × `height` as I420: scaled
 * whole, keeping its shape, centred on black; none where the sizes are the same. */
function fitter(
  composition: Opened,
  width: number,
  height: number,
): ((rgba: Uint8Array) => Promise<Uint8Array>) | undefined {
  const { width: w, height: h } = composition;
  if (w === width && h === height) {
    return undefined;
  }
  const scale = Math.min(width / w, height / h);
  const [fw, fh] = [Math.max(1, Math.round(w * scale)), Math.max(1, Math.round(h * scale))];
  const canvas = new OffscreenCanvas(width, height);
  const context = canvas.getContext("2d", { alpha: false })!;
  return async (rgba) => {
    const pixels = new ImageData(new Uint8ClampedArray(rgba.buffer as ArrayBuffer), w, h);
    const scaled = await createImageBitmap(pixels, {
      resizeWidth: fw,
      resizeHeight: fh,
      resizeQuality: "high",
    });
    context.fillStyle = "#000";
    context.fillRect(0, 0, width, height);
    context.drawImage(scaled, Math.floor((width - fw) / 2), Math.floor((height - fh) / 2));
    scaled.close();
    const fitted = context.getImageData(0, 0, width, height).data;
    return i420(new Uint8Array(fitted.buffer), width, height);
  };
}
