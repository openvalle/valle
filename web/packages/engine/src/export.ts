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
import type { Engine, Sound } from "./wasm/valle_web.js";

/** What is exported: the composition open, and how its video is encoded. */
export type Job = {
  width: number;
  height: number;
  /** Frames a second as a fraction. */
  rate: [number, number];
  frames: number;
  codec: "avc" | "hevc";
  /** Bits a second; by default 0.15 a pixel a frame for H.264, 0.1 for HEVC, as natively. */
  bitrate?: number;
};

/** The tags of the frames it encodes: BT.709's primaries and matrix, limited range, and the
 * values as sRGB's, as the native engine tags its MP4s. */
const COLOR = {
  primaries: "bt709",
  transfer: "iec61966-2-1",
  matrix: "bt709",
  fullRange: false,
} as const;

/** Samples a second of the sound, and a chunk of it encoded at a time. */
const RATE = 48_000;

/** The MP4 of `job`: `draw` draws a frame for `engine` to read back, `sound` mixes from the start
 * when there is any, `progress` hears of each frame done, and `cancelled` stops it. */
export async function encode(
  job: Job,
  engine: Engine,
  draw: (index: number) => Promise<void>,
  sound: Sound | undefined,
  progress: (done: number) => void,
  cancelled: () => boolean,
): Promise<ArrayBuffer> {
  const [n, d] = job.rate;
  const target = new BufferTarget();
  const output = new Output({ format: new Mp4OutputFormat({ fastStart: "in-memory" }), target });
  const perPixel = job.codec === "hevc" ? 0.1 : 0.15;
  const video = new VideoSampleSource({
    codec: job.codec,
    bitrate: job.bitrate ?? Math.round(perPixel * job.width * job.height * (n / d)),
  });
  output.addVideoTrack(video, { frameRate: n / d });
  let audio: AudioSampleSource | undefined;
  if (sound) {
    const aac = await canEncodeAudio("aac", { numberOfChannels: 2, sampleRate: RATE });
    audio = new AudioSampleSource({ codec: aac ? "aac" : "opus", bitrate: 192_000 });
    output.addAudioTrack(audio);
  }
  await output.start();
  const seconds = (job.frames * d) / n;
  const samples = Math.round(seconds * RATE);
  let mixed = 0;
  try {
    for (let i = 0; i < job.frames; i++) {
      if (cancelled()) {
        throw new Error("cancelled");
      }
      await draw(i);
      const data = await engine.yuv();
      const frame = new VideoSample(data, {
        format: "I420",
        codedWidth: job.width,
        codedHeight: job.height,
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
