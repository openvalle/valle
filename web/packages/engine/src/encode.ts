// Browser export draws NV12 on the GPU and gives its mapped planes to WebCodecs.

import {
  AudioSample,
  AudioSampleSource,
  BufferTarget,
  canEncodeAudio,
  Mp4OutputFormat,
  Output,
  StreamTarget,
  type StreamTargetChunk,
  VideoSample,
  VideoSampleSource,
} from "mediabunny";
import {
  type ExportOptions,
  exportSchedule,
  type FileDestination,
  type Opened,
} from "./messages.ts";
import type { Engine, VideoOutput } from "./wasm/valle_web.js";

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

/** Draws each scheduled frame into the encoder's planes and interleaves the sound. */
export async function encode(
  options: ExportOptions,
  composition: Opened,
  draw: (index: number, planes: VideoOutput) => Promise<void>,
  engine: Engine,
  sound: ((count: number) => Promise<Float32Array>) | undefined,
  destination: FileDestination | undefined,
  progress: (done: number) => void,
  signal: AbortSignal,
): Promise<ArrayBuffer | undefined> {
  const { width, height, bitrate } = options;
  const [n, d] = options.rate;
  const { count: frames, at } = exportSchedule(composition, options.rate);
  const file = destination && (await fileTarget(destination, signal));
  const target = file?.target ?? new BufferTarget();
  const output = new Output({
    format: new Mp4OutputFormat({ fastStart: file ? false : "in-memory" }),
    target,
  });
  let planes: VideoOutput | undefined;
  try {
    signal.throwIfAborted();
    planes = engine.video_output(width, height);
    const { buffer, stride } = planes;
    // WebKit reads NV12 as tightly packed even when VideoFrame specifies larger strides.
    // Y and interleaved UV each have width bytes per row for even H.264 dimensions.
    const packed = stride === width ? undefined : new Uint8Array(width * (height + height / 2));
    const video = new VideoSampleSource({ codec: "avc", bitrate });
    output.addVideoTrack(video, { frameRate: n / d });
    let audio: AudioSampleSource | undefined;
    if (sound && options.audio) {
      const { codec, sampleRate, channels, bitrate } = options.audio;
      const candidates = codec ? [codec] : (["aac", "opus"] as const);
      for (const candidate of candidates) {
        if (candidate === "opus" && sampleRate !== 48000) continue;
        if (await canEncodeAudio(candidate, { numberOfChannels: channels, sampleRate, bitrate })) {
          const transform =
            sampleRate === RATE && channels === 2
              ? undefined
              : { sampleRate, numberOfChannels: channels };
          audio = new AudioSampleSource({ codec: candidate, bitrate, transform });
          break;
        }
      }
      if (!audio)
        throw new Error(
          `Cannot encode ${codec ?? "AAC or Opus"} at ${sampleRate} Hz, ${channels} channel(s), ${bitrate} bits/s in this browser`,
        );
      output.addAudioTrack(audio);
    }
    await output.start();
    const seconds = (frames * d) / n;
    const samples = Math.round(seconds * RATE);
    let mixed = 0;
    for (let i = 0; i < frames; i++) {
      signal.throwIfAborted();
      await draw(at(i), planes);
      await buffer.mapAsync(GPUMapMode.READ);
      let frame: VideoSample;
      try {
        // VideoSample snapshots the bytes before unmap detaches the mapped ArrayBuffer.
        const mapped = new Uint8Array(buffer.getMappedRange());
        if (packed) {
          for (let src = 0, dst = 0; dst < packed.length; src += stride, dst += width) {
            packed.set(mapped.subarray(src, src + width), dst);
          }
        }
        frame = new VideoSample(packed ?? mapped, {
          format: "NV12",
          codedWidth: width,
          codedHeight: height,
          timestamp: (i * d) / n,
          duration: d / n,
          colorSpace: COLOR,
        });
      } finally {
        buffer.unmap();
      }
      try {
        await video.add(frame);
      } finally {
        frame.close();
      }
      // The sound a second ahead of the frames, so the two interleave.
      while (audio && sound && mixed < samples && mixed / RATE <= ((i + 1) * d) / n + 1) {
        const count = Math.min(RATE, samples - mixed);
        const chunk = new AudioSample({
          data: await sound(count),
          format: "f32",
          numberOfChannels: 2,
          sampleRate: RATE,
          timestamp: mixed / RATE,
        });
        try {
          await audio.add(chunk);
        } finally {
          chunk.close();
        }
        mixed += count;
      }
      progress(i + 1);
    }
    // Deliver any asynchronous GPU error from the final frame before committing the file.
    await engine.drawn();
    signal.throwIfAborted();
    await output.finalize();
    return target instanceof BufferTarget ? target.buffer! : undefined;
  } catch (error) {
    file?.fail();
    await output.cancel().catch(() => {});
    await file?.discard().catch(() => {});
    throw error;
  } finally {
    planes?.free();
  }
}

/** Positions are used by ordinary MP4 to fill in its media header at finalize. */
async function fileTarget(destination: FileDestination, signal: AbortSignal) {
  const file = await destination.handle.createWritable();
  let failed = false;
  let settled = false;
  const discard = async () => {
    if (settled) return;
    settled = true;
    await file.abort();
  };
  const writable = new WritableStream<StreamTargetChunk>({
    write(chunk) {
      signal.throwIfAborted();
      return file.write(chunk);
    },
    async close() {
      if (failed || signal.aborted) {
        await discard();
      } else {
        await file.close();
        settled = true;
      }
    },
    abort: discard,
  });
  return {
    target: new StreamTarget(writable, { chunked: true, chunkSize: 4 * 1024 * 1024 }),
    fail: () => {
      failed = true;
    },
    discard,
  };
}
